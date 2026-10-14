mod bookmarks;
mod capture;
mod disc;
mod history;
mod library;
mod menu;
mod meta;
mod playlist;
mod playlist_file;
mod settings;
#[cfg(target_os = "macos")]
mod surface_macos;
#[cfg(windows)]
mod surface_windows;
#[cfg(windows)]
mod system_windows;
mod updater;
mod vlc;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

use bookmarks::{Bookmark, Bookmarks};
use capture::{Capture, Devices};
use disc::{Disc, DiscKind};
use history::{History, UiEntry};
use library::{Library, LibraryItem};
use meta::MetaCache;
use playlist::{Item, Playlist, Repeat, SortKey};
use playlist_file::file_uri;
use settings::{Resume, Settings};
use vlc::{Chapter, Navigate, State, Title, Track, Vlc, event};

struct Core {
    vlc: Vlc,
    playlist: Mutex<Playlist>,
    app: AppHandle,
    /// Set once the video view exists; opens before that are queued so
    /// libVLC never falls back to its own window.
    ready: AtomicBool,
    pending: Mutex<Vec<Item>>,
    history: Mutex<History>,
    /// What to do once the current item starts playing.
    resume: Mutex<Option<PendingSeek>>,
    settings: Mutex<Settings>,
    data_dir: PathBuf,
    /// The user's preferred languages, for the default subtitle language.
    languages: Vec<String>,
    /// Installed font families (read once; AppKit needs the main thread).
    fonts: Vec<String>,
    /// Distinguishes the user's snapshots from thumbnail captures.
    user_snapshot: AtomicBool,
    library: Mutex<Library>,
    thumbs: Thumbnails,
    /// Where cached thumbnails and artwork go.
    thumbs_dir: PathBuf,
    meta: Mutex<MetaCache>,
    bookmarks: Mutex<Bookmarks>,
    /// A–B loop points in ms.
    ab_loop: Mutex<(Option<i64>, Option<i64>)>,
    sleep: Mutex<SleepTimer>,
}

#[derive(Default)]
struct SleepTimer {
    /// When to pause, in Unix ms.
    at: Option<u64>,
    /// Stop once the current item ends instead of playing the next.
    after_item: bool,
    /// Bumped on every change so an old timer thread knows to give up.
    generation: u64,
}

fn unix_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Inputs that are live devices rather than media, kept out of the history.
fn is_capture(uri: &str) -> bool {
    ["avcapture://", "avaudiocapture://", "screen://"].iter().any(|s| uri.starts_with(s))
}

#[derive(Clone, Copy)]
enum PendingSeek {
    /// Jump there, optionally with a "Resuming from…" message.
    Seek { ms: i64, announce: bool },
    /// Offer to resume (the "Ask" setting).
    Ask(i64),
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsInfo {
    settings: Settings,
    defaults: Settings,
    system_languages: Vec<String>,
    audio_devices: Vec<(String, String)>,
    fonts: Vec<String>,
}

/// Thumbnails, artwork and metadata waiting to be made, handled by one
/// background thread.
#[derive(Default)]
struct Thumbnails {
    queue: Mutex<VecDeque<ThumbJob>>,
    wake: Condvar,
    /// Jobs already attempted this session, so failures aren't retried forever.
    tried: Mutex<HashSet<String>>,
}

struct ThumbJob {
    uri: String,
    /// The file to write (unused for metadata).
    out: PathBuf,
    kind: JobKind,
}

enum JobKind {
    /// A video frame, this far into the media (0–1).
    Frame(f64),
    /// Read the file's tags, duration and embedded artwork.
    Meta,
    /// A resized copy of a poster or cover image.
    Art(PathBuf),
}

impl ThumbJob {
    fn key(&self) -> String {
        match self.kind {
            JobKind::Meta => format!("meta:{}", self.uri),
            _ => self.out.to_string_lossy().into_owned(),
        }
    }
}

#[derive(Clone, Serialize)]
struct LibraryList {
    folders: Vec<String>,
    items: Vec<LibraryItem>,
}

/// libVLC may be missing, in which case every command reports the error.
struct Player(Result<Arc<Core>, String>);

impl Player {
    fn core(&self) -> Result<&Arc<Core>, String> {
        self.0.as_ref().map_err(Clone::clone)
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlayerState {
    state: State,
    time: i64,
    length: i64,
    seekable: bool,
    has_video: bool,
    rate: f32,
    volume: i32,
    muted: bool,
    /// Input cache fill, 0–100; below 100 while loading or after a seek.
    buffering: f32,
    audio_delay: i64,
    subtitle_delay: i64,
    /// Index of the current title and chapter, or -1.
    title: i32,
    chapter: i32,
    loop_a: Option<i64>,
    loop_b: Option<i64>,
    /// When the sleep timer pauses playback, in Unix ms.
    sleep_at: Option<u64>,
    sleep_after_item: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Tracks {
    audio: Vec<Track>,
    audio_current: i32,
    subtitles: Vec<Track>,
    subtitle_current: i32,
}

#[derive(Clone, Serialize)]
struct Notice {
    kind: &'static str,
    text: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    state: Option<PlayerState>,
    tracks: Option<Tracks>,
    playlist: Option<Playlist>,
    chapters: Vec<Chapter>,
    titles: Vec<Title>,
    bookmarks: Vec<Bookmark>,
    engine: Result<String, String>,
}

impl Core {
    fn playlist(&self) -> MutexGuard<'_, Playlist> {
        self.playlist.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn history(&self) -> MutexGuard<'_, History> {
        self.history.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn settings(&self) -> MutexGuard<'_, Settings> {
        self.settings.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Pushes VLC-backed settings into libVLC's configuration (only the ones
    /// that differ from `previous`). Returns whether anything changed.
    fn apply_vlc_settings(&self, settings: &Settings, previous: Option<&Settings>) -> bool {
        let new = settings.vlc_config(&self.languages);
        let old = previous.map(|p| p.vlc_config(&self.languages));
        let mut changed = false;
        for (i, (name, value)) in new.iter().enumerate() {
            if old.as_ref().is_none_or(|o| o[i].1 != *value) {
                self.vlc.set_config(name, value);
                changed = true;
                // The player keeps its own copy of these, so set them directly too.
                if name.starts_with("video-title-") {
                    self.vlc.set_title_display(settings.show_title, settings.title_position, settings.title_timeout);
                } else if name.starts_with("deinterlace") && settings.deinterlace >= 0 {
                    let on = settings.deinterlace == 1;
                    self.vlc.set_deinterlace(on.then_some(settings.deinterlace_mode.as_str()).map(|m| if m == "auto" { "blend" } else { m }));
                }
            }
        }
        changed
    }

    /// Reopens the current item at the same position, so settings that
    /// only apply when a video starts take effect now.
    fn reload(&self) {
        let Some(item) = self.current_item() else { return };
        let state = self.vlc.state();
        if !matches!(state, State::Playing | State::Paused) {
            return;
        }
        *self.resume.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(PendingSeek::Seek { ms: self.vlc.time(), announce: false });
        let _ = self.vlc.open(&item.uri, &item.options);
        if state == State::Paused {
            // Let it show a frame at the old position, then pause again.
            thread::sleep(Duration::from_millis(400));
            self.vlc.set_pause(true);
        }
    }

    fn library(&self) -> MutexGuard<'_, Library> {
        self.library.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn library_list(&self) -> LibraryList {
        let library = self.library();
        let history = self.history();
        let mut items = library.items(|uri| {
            let p = history.library_thumb_path(uri);
            p.exists().then(|| p.to_string_lossy().into_owned())
        });
        // Queue thumbnails for anything that doesn't have one yet.
        let mut jobs: Vec<_> = items
            .iter()
            .filter(|i| i.thumb.is_none())
            .map(|i| ThumbJob { uri: i.uri.clone(), out: history.library_thumb_path(&i.uri), kind: JobKind::Frame(0.3) })
            .collect();
        drop(history);
        self.add_metadata(&mut items, &mut jobs);
        self.queue_thumbnails(jobs, true);
        LibraryList { folders: library.folders.clone(), items }
    }

    /// Fills in titles, episode numbers, durations and artwork, queueing
    /// jobs for whatever hasn't been read or cached yet.
    fn add_metadata(&self, items: &mut [LibraryItem], jobs: &mut Vec<ThumbJob>) {
        let cache = self.meta.lock().unwrap_or_else(|e| e.into_inner());
        let mut images = meta::Images::default();
        let art = |src: Option<PathBuf>, jobs: &mut Vec<ThumbJob>| {
            let src = src?;
            let out = meta::art_cache_path(&self.thumbs_dir, &src);
            if out.exists() {
                return Some(out.to_string_lossy().into_owned());
            }
            jobs.push(ThumbJob { uri: src.to_string_lossy().into_owned(), out, kind: JobKind::Art(src) });
            None
        };
        for item in items {
            let path = Path::new(&item.uri);
            let name = meta::parse_path(path);
            let tags = match cache.get(&item.uri) {
                Some(m) => m.clone(),
                None => {
                    jobs.push(ThumbJob { uri: item.uri.clone(), out: PathBuf::new(), kind: JobKind::Meta });
                    Default::default()
                }
            };
            // libVLC reports the file name as the title when there's no tag.
            let tag_title = tags.title.filter(|t| {
                let file = path.file_name().map(|f| f.to_string_lossy()).unwrap_or_default();
                *t != file && *t != item.title && !meta::is_release_name(t)
            });
            item.show = tags.show.or(name.show);
            item.season = tags.season.or(name.season);
            item.episode = tags.episode.or(name.episode);
            item.name = tag_title.unwrap_or(name.title);
            item.year = name.year.or_else(|| tags.date.as_deref().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok()));
            item.duration = tags.duration;
            item.art = art(images.file_poster(path).or(tags.artwork.map(PathBuf::from)), jobs);
            item.group_art = art(images.group_poster(path), jobs);
        }
    }

    /// Queues thumbnail jobs. With `once`, jobs already attempted are skipped;
    /// otherwise a job replaces any queued one for the same output.
    fn queue_thumbnails(&self, jobs: Vec<ThumbJob>, once: bool) {
        let tried = self.thumbs.tried.lock().unwrap_or_else(|e| e.into_inner());
        let mut queue = self.thumbs.queue.lock().unwrap_or_else(|e| e.into_inner());
        for job in jobs {
            let key = job.key();
            if once && tried.contains(&key) {
                continue;
            }
            queue.retain(|j| j.key() != key);
            // Resume-point thumbnails go first: they're what the home screen shows.
            if once { queue.push_back(job) } else { queue.push_front(job) }
        }
        self.thumbs.wake.notify_one();
    }

    /// Queues a "continue watching" thumbnail at the current position.
    fn queue_resume_thumbnail(&self, uri: &str) {
        let (time, length) = (self.vlc.time(), self.vlc.length());
        if length > 0 && self.vlc.has_video() {
            let out = self.history().thumb_path(uri);
            let kind = JobKind::Frame(time as f64 / length as f64);
            self.queue_thumbnails(vec![ThumbJob { uri: uri.to_string(), out, kind }], false);
        }
    }

    fn current_item(&self) -> Option<Item> {
        let playlist = self.playlist();
        playlist.current.and_then(|i| playlist.items.get(i)).cloned()
    }

    fn current_uri(&self) -> Option<String> {
        self.current_item().map(|it| it.uri)
    }

    /// Records the current position, and grabs a thumbnail if there's video.
    fn save_progress(&self, thumbnail: bool) {
        let Some(uri) = self.current_uri() else { return };
        if !matches!(self.vlc.state(), State::Playing | State::Paused) || is_capture(&uri) {
            return;
        }
        if thumbnail {
            self.queue_resume_thumbnail(&uri);
        }
        let mut history = self.history();
        history.update(&uri, self.vlc.time(), self.vlc.length());
        history.save(true);
    }

    fn emit_chapters(&self) {
        let _ = self.app.emit("player://chapters", self.vlc.chapters());
        let _ = self.app.emit("player://titles", self.vlc.titles());
    }

    fn bookmarks(&self) -> MutexGuard<'_, Bookmarks> {
        self.bookmarks.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn current_bookmarks(&self) -> Vec<Bookmark> {
        self.current_uri().map(|uri| self.bookmarks().list(&uri)).unwrap_or_default()
    }

    fn emit_bookmarks(&self) {
        let _ = self.app.emit("player://bookmarks", self.current_bookmarks());
    }

    fn ab_loop(&self) -> MutexGuard<'_, (Option<i64>, Option<i64>)> {
        self.ab_loop.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sleep(&self) -> MutexGuard<'_, SleepTimer> {
        self.sleep.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Pauses after `minutes`, or stops after the current item, or (with
    /// neither) cancels the timer.
    fn set_sleep_timer(self: &Arc<Self>, minutes: Option<u32>, after_item: bool) {
        let generation = {
            let mut sleep = self.sleep();
            sleep.generation += 1;
            sleep.at = minutes.map(|m| unix_ms() + m as u64 * 60_000);
            sleep.after_item = after_item && minutes.is_none();
            sleep.generation
        };
        if let Some(m) = minutes {
            let core = self.clone();
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(m as u64 * 60);
                while Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(500));
                    if core.sleep().generation != generation {
                        return;
                    }
                }
                *core.sleep() = SleepTimer { generation: generation + 1, ..Default::default() };
                if matches!(core.vlc.state(), State::Playing | State::Buffering | State::Opening) {
                    core.vlc.set_pause(true);
                    core.notice("info", "Sleep timer: playback paused");
                }
                core.emit_state();
            });
        }
        self.emit_state();
    }

    fn state(&self) -> PlayerState {
        let v = &self.vlc;
        let (loop_a, loop_b) = *self.ab_loop();
        let (sleep_at, sleep_after_item) = {
            let sleep = self.sleep();
            (sleep.at, sleep.after_item)
        };
        PlayerState {
            state: v.state(),
            time: v.time(),
            length: v.length(),
            seekable: v.seekable(),
            has_video: v.has_video(),
            rate: v.rate(),
            volume: v.volume(),
            muted: v.muted(),
            buffering: v.buffering(),
            audio_delay: v.audio_delay(),
            subtitle_delay: v.subtitle_delay(),
            title: v.title(),
            chapter: v.chapter(),
            loop_a,
            loop_b,
            sleep_at,
            sleep_after_item,
        }
    }

    fn tracks(&self) -> Tracks {
        let v = &self.vlc;
        Tracks {
            audio: v.audio_tracks(),
            audio_current: v.audio_track(),
            subtitles: v.subtitle_tracks(),
            subtitle_current: v.subtitle_track(),
        }
    }

    fn emit_state(&self) {
        let _ = self.app.emit("player://state", self.state());
    }

    fn emit_tracks(&self) {
        let _ = self.app.emit("player://tracks", self.tracks());
    }

    fn emit_playlist(&self) {
        let playlist = self.playlist().clone();
        let _ = self.app.emit("player://playlist", playlist);
    }

    fn notice(&self, kind: &'static str, text: impl Into<String>) {
        let _ = self.app.emit("player://notice", Notice { kind, text: text.into() });
    }

    fn play_index(&self, index: usize) -> Result<(), String> {
        self.save_progress(true);
        let item = {
            let mut playlist = self.playlist();
            let item = playlist.items.get(index).ok_or("no such playlist item")?.clone();
            playlist.current = Some(index);
            item
        };
        let (uri, title) = (&item.uri, &item.title);
        *self.ab_loop() = (None, None);
        *self.resume.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if !is_capture(uri) {
            let mut history = self.history();
            let point = history.resume_point(uri);
            let pending = match (self.settings().resume, point) {
                (Resume::Always, Some(ms)) => Some(PendingSeek::Seek { ms, announce: true }),
                (Resume::Ask, Some(ms)) => Some(PendingSeek::Ask(ms)),
                _ => None,
            };
            *self.resume.lock().unwrap_or_else(|e| e.into_inner()) = pending;
            history.touch(uri, title);
            history.save(true);
        }
        self.emit_playlist();
        self.emit_bookmarks();
        self.vlc.open(uri, &item.options)
    }

    /// Stops playback, remembering where we were.
    fn stop(&self) {
        self.save_progress(true);
        self.vlc.stop();
        self.emit_state();
    }

    /// Adds paths or URLs to the playlist, optionally starting the first
    /// new item. Folders add their media (and join the library); playlist
    /// files add their entries.
    fn add(&self, uris: Vec<String>, play: bool) -> Result<(), String> {
        let mut items = Vec::new();
        for uri in uris.into_iter().map(playlist_file::native_path) {
            let path = Path::new(&uri);
            if uri.contains("://") {
                items.push(Item::new(uri));
            } else if path.is_dir() {
                // A disc's folder plays as a disc, not as a folder of files.
                match disc::kind_of(path) {
                    Some(kind) => items.extend(disc::items(kind, &uri, true)?),
                    None => {
                        self.library().add(&uri);
                        items.extend(library::expand(vec![uri]).into_iter().map(Item::new));
                    }
                }
            } else if playlist_file::is_playlist(path) {
                items.extend(playlist_file::read(path)?);
            } else {
                items.push(Item::new(uri));
            }
        }
        self.add_items(items, play)
    }

    fn add_items(&self, items: Vec<Item>, play: bool) -> Result<(), String> {
        if items.is_empty() {
            return Ok(());
        }
        if !self.ready.load(Ordering::Acquire) {
            self.pending.lock().unwrap_or_else(|e| e.into_inner()).extend(items);
            return Ok(());
        }
        let first = self.playlist().add(items);
        self.emit_playlist();
        if play { self.play_index(first) } else { Ok(()) }
    }

    fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
        let pending = std::mem::take(&mut *self.pending.lock().unwrap_or_else(|e| e.into_inner()));
        if let Err(e) = self.add_items(pending, true) {
            self.notice("error", e);
        }
    }

    /// Moves to the next item; `auto` means the current one just finished.
    fn advance(&self, auto: bool) -> Result<(), String> {
        let next = self.playlist().next_index(auto);
        match next {
            Some(i) => self.play_index(i),
            None if auto => Ok(()),
            None => Err("end of playlist".into()),
        }
    }
}

/// Generates library thumbnails one at a time, pausing while video plays so
/// decoding never competes with playback.
fn make_thumbnails(core: Arc<Core>) {
    loop {
        let job = {
            let mut queue = core.thumbs.queue.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(job) = queue.pop_front() {
                    break job;
                }
                // Idle: write out what was read.
                core.meta.lock().unwrap_or_else(|e| e.into_inner()).save();
                queue = core.thumbs.wake.wait(queue).unwrap_or_else(|e| e.into_inner());
            }
        };
        // Only real files can be read by the second player.
        if job.uri.contains("://") {
            continue;
        }
        core.thumbs.tried.lock().unwrap_or_else(|e| e.into_inner()).insert(job.key());
        let result = match job.kind {
            JobKind::Frame(at) => {
                while core.vlc.state() == State::Playing {
                    thread::sleep(Duration::from_secs(1));
                }
                core.vlc.thumbnail(&job.uri, &job.out, at, 480)
            }
            JobKind::Meta => {
                // Remember failures too, so unreadable files aren't parsed on every scan.
                let tags = core.vlc.parse(&job.uri, 5000);
                core.meta.lock().unwrap_or_else(|e| e.into_inner()).insert(&job.uri, tags.clone().unwrap_or_default());
                tags.map(|_| ())
            }
            JobKind::Art(src) => meta::resize_art(&src, &job.out, 360),
        };
        match result {
            Ok(()) => {
                let _ = core.app.emit("library://thumb", job.uri);
            }
            Err(e) => eprintln!("revlc: library job failed: {e}"),
        }
    }
}

/// Turns libVLC events into UI events. Time updates are throttled so the
/// web view isn't woken more than a few times a second.
fn pump_events(core: Arc<Core>, rx: Receiver<i32>) {
    const TIME_INTERVAL: Duration = Duration::from_millis(250);
    const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

    let mut last_time_emit = Instant::now() - TIME_INTERVAL;
    let mut last_progress = Instant::now();
    let mut loop_jump: Option<Instant> = None;
    while let Ok(first) = rx.recv() {
        // Let bursts of events (e.g. opening a file) arrive, then handle them together.
        thread::sleep(Duration::from_millis(20));
        let events: Vec<i32> = std::iter::once(first).chain(rx.try_iter()).collect();
        let has = |ty: i32| events.contains(&ty);

        if has(event::END_REACHED) {
            if let Some(uri) = core.current_uri() {
                let mut history = core.history();
                history.finish(&uri);
                history.save(true);
            }
            let stop_after_each = core.settings().stop_after_each;
            let sleep_now = std::mem::take(&mut core.sleep().after_item);
            if sleep_now {
                core.notice("info", "Sleep timer: stopped after this item");
            } else if !stop_after_each {
                if let Err(e) = core.advance(true) {
                    core.notice("error", e);
                }
            }
        }
        if has(event::ERROR) {
            let title = {
                let playlist = core.playlist();
                playlist.current.and_then(|i| playlist.items.get(i)).map(|it| it.title.clone())
            };
            core.notice("error", format!("Could not play {}", title.unwrap_or_else(|| "this media".into())));
        }
        if has(event::PLAYING) {
            match core.resume.lock().unwrap_or_else(|e| e.into_inner()).take() {
                Some(PendingSeek::Seek { ms, announce }) => {
                    core.vlc.set_time(ms);
                    if announce {
                        core.notice("osd", format!("Resuming from {}", format_time(ms)));
                    }
                }
                Some(PendingSeek::Ask(ms)) => {
                    let _ = core.app.emit("player://resume-prompt", ms);
                }
                None => {}
            }
            let device = core.settings().audio_device.clone();
            if !device.is_empty() {
                core.vlc.set_audio_device(&device);
            }
        }
        if has(event::PAUSED) {
            core.save_progress(true);
        }
        if has(event::TIME_CHANGED) {
            // A–B loop: back to A whenever playback passes B. Time events
            // can lag a seek, so jump once until we're back before B.
            if let (Some(a), Some(b)) = *core.ab_loop() {
                let time = core.vlc.time();
                if time < b {
                    loop_jump = None;
                } else if loop_jump.is_none_or(|at| at.elapsed() > Duration::from_secs(1)) {
                    core.vlc.set_time(a);
                    loop_jump = Some(Instant::now());
                }
            }
        }
        if has(event::TIME_CHANGED) && last_progress.elapsed() >= PROGRESS_INTERVAL {
            last_progress = Instant::now();
            if let Some(uri) = core.current_uri() {
                let mut history = core.history();
                history.update(&uri, core.vlc.time(), core.vlc.length());
                history.save(false);
            }
        }
        if [event::PLAYING, event::LENGTH_CHANGED, event::MEDIA_CHANGED, event::TITLE_CHANGED].iter().any(|&e| has(e)) {
            core.emit_chapters();
        }
        if has(event::PLAYING) {
            if let Some(title) = core.vlc.media_title() {
                let mut playlist = core.playlist();
                let changed = match playlist.current.and_then(|i| playlist.items.get_mut(i)) {
                    Some(item) if item.title != title => {
                        item.title = title.clone();
                        true
                    }
                    _ => false,
                };
                drop(playlist);
                if changed {
                    if let Some(uri) = core.current_uri() {
                        core.history().set_title(&uri, &title);
                    }
                    core.emit_playlist();
                }
            }
        }
        if has(event::SNAPSHOT_TAKEN) && core.user_snapshot.swap(false, Ordering::AcqRel) {
            core.notice("info", "Snapshot saved");
        }
        if [event::PLAYING, event::VOUT, event::ES_ADDED, event::ES_DELETED, event::ES_SELECTED]
            .iter()
            .any(|&e| has(e))
        {
            core.emit_tracks();
        }

        // Time and buffering progress arrive many times a second; throttle
        // them, but always send the moment buffering finishes.
        let minor = events.iter().all(|&e| e == event::TIME_CHANGED || e == event::BUFFERING)
            && !(has(event::BUFFERING) && core.vlc.buffering() >= 100.0);
        if !minor || last_time_emit.elapsed() >= TIME_INTERVAL {
            core.emit_state();
            last_time_emit = Instant::now();
        }
    }
}

type Res = Result<(), String>;

fn format_time(ms: i64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[tauri::command]
fn snapshot(player: tauri::State<Player>) -> Snapshot {
    match player.core() {
        Ok(core) => Snapshot {
            // Before the playlist lock below, which lasts for this whole expression.
            bookmarks: core.current_bookmarks(),
            state: Some(core.state()),
            tracks: Some(core.tracks()),
            playlist: Some(core.playlist().clone()),
            chapters: core.vlc.chapters(),
            titles: core.vlc.titles(),
            engine: Ok(format!("libVLC {}", core.vlc.version)),
        },
        Err(e) => Snapshot {
            state: None,
            tracks: None,
            playlist: None,
            chapters: Vec::new(),
            titles: Vec::new(),
            bookmarks: Vec::new(),
            engine: Err(e),
        },
    }
}

#[tauri::command]
fn open(player: tauri::State<Player>, uris: Vec<String>, play: bool) -> Res {
    player.core()?.add(uris, play)
}

#[tauri::command]
fn play_index(player: tauri::State<Player>, index: usize) -> Res {
    player.core()?.play_index(index)
}

#[tauri::command]
fn toggle_pause(player: tauri::State<Player>) -> Res {
    let core = player.core()?;
    match core.vlc.state() {
        State::Playing | State::Buffering | State::Opening => core.vlc.set_pause(true),
        State::Paused => core.vlc.set_pause(false),
        _ => {
            let index = {
                let playlist = core.playlist();
                playlist.current.or(if playlist.items.is_empty() { None } else { Some(0) })
            };
            if let Some(i) = index {
                return core.play_index(i);
            }
        }
    }
    Ok(())
}

#[tauri::command]
fn stop(player: tauri::State<Player>) -> Res {
    player.core()?.stop();
    Ok(())
}

#[tauri::command]
fn history_list(player: tauri::State<Player>) -> Result<Vec<UiEntry>, String> {
    let core = player.core()?;
    let list = core.history().list();
    // Fill in thumbnails that are missing, e.g. because the app quit first.
    let missing: Vec<_> = list
        .iter()
        .filter(|e| e.thumb.is_none() && e.entry.length > 0 && !e.entry.uri.contains("://"))
        .map(|e| ThumbJob {
            uri: e.entry.uri.clone(),
            out: core.history().thumb_path(&e.entry.uri),
            kind: JobKind::Frame(if e.entry.position > 0 { e.entry.position as f64 / e.entry.length as f64 } else { 0.3 }),
        })
        .collect();
    core.queue_thumbnails(missing, true);
    Ok(list)
}

#[tauri::command]
fn settings_get(player: tauri::State<Player>) -> Result<SettingsInfo, String> {
    let core = player.core()?;
    Ok(SettingsInfo {
        settings: core.settings().clone(),
        defaults: Settings::default(),
        system_languages: core.languages.clone(),
        audio_devices: core.vlc.audio_devices(),
        fonts: core.fonts.clone(),
    })
}

/// Saves new settings, applies them, and reopens the current video if a
/// change only takes effect when a video starts.
#[tauri::command]
fn settings_set(player: tauri::State<Player>, settings: Settings) -> Res {
    let core = player.core()?;
    let previous = core.settings().clone();
    if previous == settings {
        return Ok(());
    }
    let vlc_changed = core.apply_vlc_settings(&settings, Some(&previous));
    if settings.audio_device != previous.audio_device {
        core.vlc.set_audio_device(&settings.audio_device);
    }
    settings.save(&core.data_dir);
    *core.settings() = settings.clone();
    let _ = core.app.emit("settings://changed", settings);
    if vlc_changed {
        let reload = core.clone();
        thread::spawn(move || reload.reload());
    }
    Ok(())
}

/// Seeks to a resume point the user accepted.
#[tauri::command]
fn resume_at(player: tauri::State<Player>, ms: i64) -> Res {
    player.core()?.vlc.set_time(ms);
    Ok(())
}

/// Async: on Windows, creating a window from a synchronous command (which
/// runs on the main thread) deadlocks WebView2.
#[tauri::command]
async fn open_settings(app: AppHandle) -> Res {
    show_settings(&app).map_err(|e| e.to_string())
}

/// Rebuilds the menu bar with the current shortcut bindings.
#[tauri::command]
fn set_menu_shortcuts(app: AppHandle, keys: HashMap<String, String>) -> Res {
    // Only macOS has a menu bar; elsewhere the UI handles every shortcut.
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let menu = menu::build(&app, &keys).map_err(|e| e.to_string())?;
    app.set_menu(menu).map(|_| ()).map_err(|e| e.to_string())
}

fn show_settings(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("settings") {
        window.unminimize()?;
        return window.set_focus();
    }
    let builder = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html?view=settings".into()))
        .title("Settings")
        .inner_size(760.0, 640.0)
        .min_inner_size(640.0, 480.0);
    // The UI draws the title bar: traffic lights over it on macOS, its own
    // window buttons elsewhere.
    #[cfg(target_os = "macos")]
    let builder = builder.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.decorations(false);
    builder.build()?;
    Ok(())
}

/// Installed font families, for the subtitle font picker. Main thread only.
fn font_families() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        let Some(mtm) = objc2::MainThreadMarker::new() else { return Vec::new() };
        let manager = objc2_app_kit::NSFontManager::sharedFontManager(mtm);
        let mut fonts: Vec<String> = manager.availableFontFamilies().iter().map(|f| f.to_string()).collect();
        fonts.retain(|f| !f.starts_with('.'));
        fonts.sort_by_key(|f| f.to_lowercase());
        fonts
    }
    #[cfg(windows)]
    {
        system_windows::font_families()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    Vec::new()
}

#[tauri::command]
fn library_list(player: tauri::State<Player>) -> Result<LibraryList, String> {
    Ok(player.core()?.library_list())
}

#[tauri::command]
fn library_add(player: tauri::State<Player>, path: String) -> Result<LibraryList, String> {
    let core = player.core()?;
    core.library().add(&path);
    Ok(core.library_list())
}

#[tauri::command]
fn library_remove(player: tauri::State<Player>, path: String) -> Result<LibraryList, String> {
    let core = player.core()?;
    core.library().remove(&path);
    Ok(core.library_list())
}

#[tauri::command]
fn history_remove(player: tauri::State<Player>, uri: String) -> Res {
    let core = player.core()?;
    let mut history = core.history();
    history.remove(&uri);
    history.save(true);
    Ok(())
}

#[tauri::command]
fn seek(player: tauri::State<Player>, ms: i64) -> Res {
    let core = player.core()?;
    core.vlc.set_time(ms);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn seek_by(player: tauri::State<Player>, delta_ms: i64) -> Res {
    let core = player.core()?;
    let target = core.vlc.time() + delta_ms;
    let length = core.vlc.length();
    core.vlc.set_time(if length > 0 { target.min(length - 500) } else { target });
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn next(player: tauri::State<Player>) -> Res {
    player.core()?.advance(false)
}

#[tauri::command]
fn previous(player: tauri::State<Player>) -> Res {
    let core = player.core()?;
    // Like most players: restart the current item unless we're near its start.
    if core.vlc.time() > 3000 {
        core.vlc.set_time(0);
        return Ok(());
    }
    let prev = core.playlist().prev_index();
    match prev {
        Some(i) => core.play_index(i),
        None => Ok(()),
    }
}

#[tauri::command]
fn next_frame(player: tauri::State<Player>) -> Res {
    let core = player.core()?;
    core.vlc.next_frame();
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_volume(player: tauri::State<Player>, volume: i32) -> Res {
    let core = player.core()?;
    core.vlc.set_volume(volume);
    if volume > 0 && core.vlc.muted() {
        core.vlc.set_muted(false);
    }
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_muted(player: tauri::State<Player>, muted: bool) -> Res {
    let core = player.core()?;
    core.vlc.set_muted(muted);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_rate(player: tauri::State<Player>, rate: f32) -> Res {
    let core = player.core()?;
    core.vlc.set_rate(rate);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_audio_track(player: tauri::State<Player>, id: i32) -> Res {
    let core = player.core()?;
    core.vlc.set_audio_track(id);
    core.emit_tracks();
    Ok(())
}

#[tauri::command]
fn set_subtitle_track(player: tauri::State<Player>, id: i32) -> Res {
    let core = player.core()?;
    core.vlc.set_subtitle_track(id);
    core.emit_tracks();
    Ok(())
}

#[tauri::command]
fn add_subtitle(player: tauri::State<Player>, uri: String) -> Res {
    let core = player.core()?;
    let uri = if uri.contains("://") { uri } else { file_uri(&uri) };
    core.vlc.add_subtitle(&uri)
}

#[tauri::command]
fn set_audio_delay(player: tauri::State<Player>, ms: i64) -> Res {
    let core = player.core()?;
    core.vlc.set_audio_delay(ms);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_subtitle_delay(player: tauri::State<Player>, ms: i64) -> Res {
    let core = player.core()?;
    core.vlc.set_subtitle_delay(ms);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_aspect_ratio(player: tauri::State<Player>, ratio: Option<String>) -> Res {
    player.core()?.vlc.set_aspect_ratio(ratio.as_deref());
    Ok(())
}

#[tauri::command]
fn set_crop(player: tauri::State<Player>, geometry: Option<String>) -> Res {
    player.core()?.vlc.set_crop(geometry.as_deref());
    Ok(())
}

#[tauri::command]
fn set_deinterlace(player: tauri::State<Player>, mode: Option<String>) -> Res {
    player.core()?.vlc.set_deinterlace(mode.as_deref());
    Ok(())
}

#[tauri::command]
fn take_snapshot(app: AppHandle, player: tauri::State<Player>) -> Res {
    let core = player.core()?;
    // VLC's default folder: the Desktop on macOS, Pictures on Windows.
    let default = if cfg!(target_os = "macos") { app.path().desktop_dir() } else { app.path().picture_dir() };
    let dir = core.settings().snapshot_dir(default.ok()).ok_or("no snapshot folder")?;
    core.user_snapshot.store(true, Ordering::Release);
    core.vlc.snapshot(&dir, 0)
}

#[tauri::command]
fn hide_cursor(app: AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.run_on_main_thread(surface_macos::hide_cursor);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

#[tauri::command]
fn playlist_remove(player: tauri::State<Player>, index: usize) -> Res {
    let core = player.core()?;
    let was_current = core.playlist().remove(index);
    if was_current {
        core.vlc.stop();
    }
    core.emit_playlist();
    Ok(())
}

#[tauri::command]
fn playlist_move(player: tauri::State<Player>, from: usize, to: usize) -> Res {
    let core = player.core()?;
    core.playlist().move_item(from, to);
    core.emit_playlist();
    Ok(())
}

#[tauri::command]
fn playlist_clear(player: tauri::State<Player>) -> Res {
    let core = player.core()?;
    core.stop();
    core.playlist().clear();
    core.emit_playlist();
    Ok(())
}

#[tauri::command]
fn set_repeat(player: tauri::State<Player>, repeat: Repeat) -> Res {
    let core = player.core()?;
    core.playlist().repeat = repeat;
    core.emit_playlist();
    Ok(())
}

#[tauri::command]
fn set_shuffle(player: tauri::State<Player>, shuffle: bool) -> Res {
    let core = player.core()?;
    core.playlist().shuffle = shuffle;
    core.emit_playlist();
    Ok(())
}

#[tauri::command]
fn playlist_sort(player: tauri::State<Player>, by: SortKey) -> Res {
    let core = player.core()?;
    core.playlist().sort(by);
    core.emit_playlist();
    Ok(())
}

/// Saves the queue as M3U, PLS or XSPF (chosen by the file's extension).
#[tauri::command]
fn playlist_save(player: tauri::State<Player>, path: String) -> Res {
    let core = player.core()?;
    let items = core.playlist().items.clone();
    playlist_file::write(Path::new(&path), &items)
}

/// Sets the A–B loop points (ms); `None` clears one.
#[tauri::command]
fn set_ab_loop(player: tauri::State<Player>, a: Option<i64>, b: Option<i64>) -> Res {
    let core = player.core()?;
    *core.ab_loop() = match (a, b) {
        (Some(a), Some(b)) if b < a => (Some(b), Some(a)),
        ab => ab,
    };
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_sleep_timer(player: tauri::State<Player>, minutes: Option<u32>, after_item: bool) -> Res {
    player.core()?.set_sleep_timer(minutes, after_item);
    Ok(())
}

#[tauri::command]
fn set_title(player: tauri::State<Player>, index: i32) -> Res {
    let core = player.core()?;
    core.vlc.set_title(index);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_chapter(player: tauri::State<Player>, index: i32) -> Res {
    let core = player.core()?;
    core.vlc.set_chapter(index);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn navigate(player: tauri::State<Player>, action: Navigate) -> Res {
    player.core()?.vlc.navigate(action);
    Ok(())
}

/// Bookmarks the current position.
#[tauri::command]
fn bookmark_add(player: tauri::State<Player>, name: Option<String>) -> Res {
    let core = player.core()?;
    let uri = core.current_uri().ok_or("Nothing is playing")?;
    let time = core.vlc.time();
    core.bookmarks().add(&uri, time, name);
    core.emit_bookmarks();
    core.notice("osd", format!("Bookmark added at {}", format_time(time)));
    Ok(())
}

#[tauri::command]
fn bookmark_remove(player: tauri::State<Player>, time: i64) -> Res {
    let core = player.core()?;
    let uri = core.current_uri().ok_or("Nothing is playing")?;
    core.bookmarks().remove(&uri, time);
    core.emit_bookmarks();
    Ok(())
}

#[tauri::command]
fn bookmark_rename(player: tauri::State<Player>, time: i64, name: String) -> Res {
    let core = player.core()?;
    let uri = core.current_uri().ok_or("Nothing is playing")?;
    core.bookmarks().rename(&uri, time, &name);
    core.emit_bookmarks();
    Ok(())
}

#[tauri::command]
fn disc_list() -> Vec<Disc> {
    disc::detect()
}

/// Plays a disc: a mounted volume, a disc folder or a disc image.
#[tauri::command]
fn open_disc(player: tauri::State<Player>, kind: DiscKind, path: String, menus: bool) -> Res {
    let core = player.core()?;
    core.add_items(disc::items(kind, &path, menus)?, true)
}

/// Lists capture devices. Runs on the main thread (Tauri runs synchronous
/// commands there), which AppKit needs for the screen list.
#[tauri::command]
fn capture_devices() -> Devices {
    capture::devices()
}

#[tauri::command]
fn open_capture(player: tauri::State<Player>, capture: Capture) -> Res {
    player.core()?.add_items(vec![capture::item(&capture)], true)
}

/// The user's preferred languages as two-letter codes, most preferred first.
fn preferred_languages() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        let mut langs: Vec<String> = Vec::new();
        for lang in objc2_foundation::NSLocale::preferredLanguages().iter() {
            let code = lang.to_string().split(['-', '_']).next().unwrap_or_default().to_lowercase();
            if !code.is_empty() && !langs.contains(&code) {
                langs.push(code);
            }
        }
        langs
    }
    #[cfg(windows)]
    {
        system_windows::preferred_languages()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    Vec::new()
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    updater::start(app.handle());
    let resource_dir = app.path().resource_dir().ok();
    let data_dir = app.path().app_data_dir()?;
    let languages = preferred_languages();
    let settings = Settings::load(&data_dir);
    // Extra libVLC options for debugging, e.g. REVLC_VLC_ARGS="--verbose=2".
    let args: Vec<String> = std::env::var("REVLC_VLC_ARGS").unwrap_or_default().split_whitespace().map(String::from).collect();
    let vlc = match Vlc::load(resource_dir.as_deref(), &args, &settings.vlc_config(&languages)) {
        Ok(vlc) => vlc,
        Err(e) => {
            eprintln!("revlc: {e}");
            app.manage(Player(Err(e)));
            return Ok(());
        }
    };
    let cache_dir = app.path().app_cache_dir()?;
    let core = Arc::new(Core {
        vlc,
        playlist: Mutex::default(),
        app: app.handle().clone(),
        ready: AtomicBool::new(false),
        pending: Mutex::default(),
        history: Mutex::new(History::load(data_dir.clone(), cache_dir.clone())),
        resume: Mutex::default(),
        settings: Mutex::new(settings.clone()),
        languages,
        fonts: font_families(),
        user_snapshot: AtomicBool::new(false),
        library: Mutex::new(Library::load(&data_dir)),
        thumbs: Thumbnails::default(),
        thumbs_dir: cache_dir.join("thumbs"),
        meta: Mutex::new(MetaCache::load(&cache_dir)),
        bookmarks: Mutex::new(Bookmarks::load(&data_dir)),
        ab_loop: Mutex::default(),
        sleep: Mutex::default(),
        data_dir,
    });

    // The UI sends the real bindings once it loads; start with the defaults.
    let keys: HashMap<String, String> = [("open", "Cmd+O"), ("openFolder", "Shift+Cmd+O"), ("openUrl", "Cmd+N"), ("queue", "Cmd+L")]
        .into_iter()
        .map(|(a, k)| (a.to_string(), k.to_string()))
        .collect();
    if cfg!(target_os = "macos") {
        app.set_menu(menu::build(app.handle(), &keys)?)?;
    }
    // libVLC's player has its own title-display default; apply ours.
    core.vlc.set_title_display(settings.show_title, settings.title_position, settings.title_timeout);
    let thumbs = core.clone();
    thread::Builder::new().name("thumbnails".into()).spawn(move || make_thumbnails(thumbs))?;

    let (tx, rx) = mpsc::channel();
    core.vlc.subscribe(tx);
    let pump = core.clone();
    thread::Builder::new().name("vlc-events".into()).spawn(move || pump_events(pump, rx))?;

    // Files passed on the command line.
    let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();
    let _ = core.add(args, true);

    let window = app.get_webview_window("main").ok_or("main window missing")?;
    #[cfg(windows)]
    let hwnd = window.hwnd()?.0 as isize;
    let attach = core.clone();
    window.with_webview(move |webview| {
        #[cfg(target_os = "macos")]
        {
            let app = attach.app.clone();
            let last = std::cell::Cell::new(Instant::now() - Duration::from_secs(1));
            let on_pointer = move || {
                // Throttled: the UI only needs to know the pointer is active.
                if last.get().elapsed() >= Duration::from_millis(150) {
                    last.set(Instant::now());
                    let _ = app.emit("native://pointer", ());
                }
            };
            match surface_macos::attach(webview.inner(), webview.ns_window(), on_pointer) {
                Ok(view) => attach.vlc.set_nsview(view),
                Err(e) => eprintln!("revlc: could not create the video view: {e}"),
            }
        }
        // The web view covers the whole window and gets every mouse event,
        // so Windows needs no pointer monitor.
        #[cfg(windows)]
        {
            let _ = webview;
            match surface_windows::attach(hwnd as *mut std::ffi::c_void) {
                Ok(video) => attach.vlc.set_hwnd(video),
                Err(e) => eprintln!("revlc: could not create the video window: {e}"),
            }
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        let _ = webview;
        attach.mark_ready();
    })?;

    app.manage(Player(Ok(core)));
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // Windows starts a new process for each file opened from Explorer; it
    // hands its files to the running copy and exits.
    #[cfg(windows)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        let uris: Vec<String> = args.into_iter().skip(1).filter(|a| !a.starts_with('-')).collect();
        if uris.is_empty() {
            return;
        }
        if let Some(Ok(core)) = app.try_state::<Player>().as_deref().map(Player::core) {
            if let Err(e) = core.add(uris, true) {
                core.notice("error", e);
            }
        }
    }));
    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(setup)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "settings" => {
                if let Err(e) = show_settings(app) {
                    eprintln!("revlc: could not open settings: {e}");
                }
            }
            "checkUpdates" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { updater::check(&app, true).await });
            }
            "github" => {
                let _ = std::process::Command::new("open").arg("https://github.com/HiteshRohira/ReVLC").spawn();
            }
            // Everything else is a player action the UI knows how to run.
            id => {
                let _ = app.emit_to("main", "menu://action", id.to_string());
            }
        })
        .invoke_handler(tauri::generate_handler![
            updater::update_state,
            updater::update_check,
            updater::update_install,
            updater::update_restart,
            snapshot,
            open,
            play_index,
            toggle_pause,
            stop,
            history_list,
            history_remove,
            settings_get,
            settings_set,
            resume_at,
            open_settings,
            set_menu_shortcuts,
            library_list,
            library_add,
            library_remove,
            seek,
            seek_by,
            next,
            previous,
            next_frame,
            set_volume,
            set_muted,
            set_rate,
            set_audio_track,
            set_subtitle_track,
            add_subtitle,
            set_audio_delay,
            set_subtitle_delay,
            set_aspect_ratio,
            set_crop,
            set_deinterlace,
            take_snapshot,
            hide_cursor,
            playlist_remove,
            playlist_move,
            playlist_clear,
            set_repeat,
            set_shuffle,
            playlist_sort,
            playlist_save,
            set_ab_loop,
            set_sleep_timer,
            set_title,
            set_chapter,
            navigate,
            bookmark_add,
            bookmark_remove,
            bookmark_rename,
            disc_list,
            open_disc,
            capture_devices,
            open_capture,
        ])
        .build(tauri::generate_context!())
        .expect("error while building revlc");

    app.run(|handle, event| {
        if let RunEvent::ExitRequested { .. } = event {
            if let Some(Ok(core)) = handle.try_state::<Player>().as_deref().map(Player::core) {
                core.save_progress(false);
            }
        }
        // Files opened from Finder ("Open With", double-click, dock drop).
        #[cfg(target_os = "macos")]
        if let RunEvent::Opened { urls } = event {
            let uris: Vec<String> = urls
                .into_iter()
                .map(|u| u.to_file_path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| u.to_string()))
                .collect();
            if let Some(Ok(core)) = handle.try_state::<Player>().as_deref().map(Player::core) {
                if let Err(e) = core.add(uris, true) {
                    core.notice("error", e);
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (handle, event);
    });
}
