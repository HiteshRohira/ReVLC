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
mod store;
#[cfg(target_os = "macos")]
mod surface_macos;
#[cfg(windows)]
mod surface_windows;
#[cfg(windows)]
mod system_windows;
mod updater;
mod vlc;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};

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
    pending: Mutex<Pending>,
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
    /// Held while changing what plays, so a finished item can't advance
    /// the queue while the user is picking another.
    transition: Mutex<()>,
    /// Bumped by each settings change that reopens the video; only the last
    /// one of a burst does it.
    reloads: AtomicU64,
    /// The last library scan, reused for a little while.
    library_scan: Mutex<Option<LibraryScan>>,
}

struct LibraryScan {
    folders: Vec<String>,
    at: Instant,
    items: Vec<LibraryItem>,
}

/// How long a library scan is reused. Refreshes come every couple of
/// seconds while thumbnails are made; new files show up after this.
const LIBRARY_SCAN_TTL: Duration = Duration::from_secs(30);

/// Files macOS hands over before `setup` has made the player: on a cold
/// launch from Finder, the open event can come first. `setup` plays them.
/// Only touched on the main thread.
static EARLY_OPENS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// What was opened before the UI was ready, to play once it is.
#[derive(Default)]
struct Pending {
    items: Vec<Item>,
    subtitles: Vec<String>,
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

/// Inputs that are live devices rather than media.
fn is_capture(uri: &str) -> bool {
    ["avcapture://", "avaudiocapture://", "screen://", "dshow://"].iter().any(|s| uri.starts_with(s))
}

/// Whether to remember `uri` in the history: not devices, and not URLs
/// with a password in them (history is a plain file).
fn in_history(uri: &str) -> bool {
    let has_password = uri.split_once("://").is_some_and(|(_, rest)| {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        authority.rsplit_once('@').is_some_and(|(userinfo, _)| userinfo.contains(':'))
    });
    !is_capture(uri) && !has_password
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
        self.vlc.set_hardware_decoding(settings.hardware_decoding);
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

    fn transition(&self) -> MutexGuard<'_, ()> {
        self.transition.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Reopens the current item at the same position, so settings that
    /// only apply when a video starts take effect now. Changes often come
    /// in bursts (dragging a slider), so it waits and only the last runs.
    fn reload(&self) {
        let reload = self.reloads.fetch_add(1, Ordering::AcqRel) + 1;
        thread::sleep(Duration::from_millis(300));
        if self.reloads.load(Ordering::Acquire) != reload {
            return;
        }
        let (state, generation) = {
            let _transition = self.transition();
            let Some(item) = self.current_item() else { return };
            let state = self.vlc.state();
            if !matches!(state, State::Playing | State::Paused) {
                return;
            }
            *self.resume.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(PendingSeek::Seek { ms: self.vlc.time(), announce: false });
            let _ = self.vlc.open(&item.uri, &item.options, item.trusted);
            (state, self.vlc.generation())
        };
        if state == State::Paused {
            // Let it show a frame at the old position, then pause again,
            // unless something else has been opened since.
            thread::sleep(Duration::from_millis(400));
            let _transition = self.transition();
            if self.vlc.generation() == generation {
                self.vlc.set_pause(true);
            }
        }
    }

    fn library(&self) -> MutexGuard<'_, Library> {
        self.library.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Lists the library. Reads folders and files, so call it off the main thread.
    fn library_list(&self) -> LibraryList {
        let folders = self.library().folders.clone();
        let (mut items, scanned) = {
            let mut cache = self.library_scan.lock().unwrap_or_else(|e| e.into_inner());
            match cache.as_ref().filter(|c| c.folders == folders && c.at.elapsed() < LIBRARY_SCAN_TTL) {
                Some(c) => (c.items.clone(), false),
                None => {
                    let items = library::scan_folders(&folders);
                    *cache = Some(LibraryScan { folders: folders.clone(), at: Instant::now(), items: items.clone() });
                    (items, true)
                }
            }
        };
        // Queue thumbnails for anything that doesn't have one yet.
        let mut jobs = Vec::new();
        for item in &mut items {
            let thumb = self.library_thumb_path(&item.uri);
            if thumb.exists() {
                item.thumb = Some(thumb.to_string_lossy().into_owned());
            } else {
                jobs.push(ThumbJob { uri: item.uri.clone(), out: thumb, kind: JobKind::Frame(0.3) });
            }
        }
        self.add_metadata(&mut items, &mut jobs);
        // Only when every folder is there: files on an unplugged drive keep their thumbnails.
        if scanned && folders.iter().all(|f| Path::new(f).is_dir()) {
            self.prune_library_cache(&items, &jobs);
        }
        self.queue_thumbnails(jobs, true);
        LibraryList { folders, items }
    }

    fn library_thumb_path(&self, uri: &str) -> PathBuf {
        self.history().library_thumb_path(uri)
    }

    /// Deletes cached thumbnails, artwork and metadata of files no longer
    /// in the library (or artwork that has since changed).
    fn prune_library_cache(&self, items: &[LibraryItem], jobs: &[ThumbJob]) {
        let mut keep: HashSet<PathBuf> = jobs.iter().map(|j| j.out.clone()).collect();
        for item in items {
            keep.insert(self.library_thumb_path(&item.uri));
            keep.extend([&item.art, &item.group_art].into_iter().flatten().map(PathBuf::from));
        }
        // Job keys of what's deleted, so it's made again if the file comes back.
        let mut forget: HashSet<String> = HashSet::new();
        if let Ok(entries) = std::fs::read_dir(&self.thumbs_dir) {
            for path in entries.flatten().map(|e| e.path()) {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if (name.starts_with("lib-") || name.starts_with("art-")) && !keep.contains(&path) {
                    let _ = std::fs::remove_file(&path);
                    forget.insert(path.to_string_lossy().into_owned());
                }
            }
        }
        let uris: HashSet<&str> = items.iter().map(|i| i.uri.as_str()).collect();
        let mut meta = self.meta.lock().unwrap_or_else(|e| e.into_inner());
        forget.extend(meta.retain(&uris).into_iter().map(|uri| format!("meta:{uri}")));
        meta.save();
        drop(meta);
        self.thumbs.tried.lock().unwrap_or_else(|e| e.into_inner()).retain(|key| !forget.contains(key));
    }

    /// Forgets the last library scan, so the next list sees new files.
    fn rescan_library(&self) {
        *self.library_scan.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
        let mut queued: HashSet<String> = queue.iter().map(ThumbJob::key).collect();
        for job in jobs {
            let key = job.key();
            if once {
                if !tried.contains(&key) && queued.insert(key) {
                    queue.push_back(job);
                }
            } else {
                if !queued.insert(key.clone()) {
                    queue.retain(|j| j.key() != key);
                }
                // Resume-point thumbnails go first: they're what the home screen shows.
                queue.push_front(job);
            }
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
        if !matches!(self.vlc.state(), State::Playing | State::Paused) || !in_history(&uri) {
            return;
        }
        if thumbnail {
            self.queue_resume_thumbnail(&uri);
        }
        let mut history = self.history();
        history.update(&uri, self.vlc.time(), self.vlc.length());
        let _ = history.save(true);
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
        let _transition = self.transition();
        self.play_index_locked(index)
    }

    /// [`Core::play_index`], with the transition lock already held.
    fn play_index_locked(&self, index: usize) -> Result<(), String> {
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
        if in_history(uri) {
            self.set_resume(uri);
            let mut history = self.history();
            history.touch(uri, title);
            let _ = history.save(true);
        }
        self.emit_playlist();
        self.emit_bookmarks();
        self.vlc.open(uri, &item.options, item.trusted)
    }

    /// What to do about `uri`'s resume point once it starts playing.
    fn set_resume(&self, uri: &str) {
        let point = self.history().resume_point(uri);
        let pending = match (self.settings().resume, point) {
            (Resume::Always, Some(ms)) => Some(PendingSeek::Seek { ms, announce: true }),
            (Resume::Ask, Some(ms)) => Some(PendingSeek::Ask(ms)),
            _ => None,
        };
        *self.resume.lock().unwrap_or_else(|e| e.into_inner()) = pending;
    }

    /// Stops playback, remembering where we were.
    fn stop(&self) {
        let _transition = self.transition();
        self.stop_locked();
    }

    fn stop_locked(&self) {
        self.save_progress(true);
        self.vlc.stop();
        self.emit_state();
    }

    /// Adds paths or URLs to the playlist, optionally starting the first
    /// new item. Folders add their media (and join the library); playlist
    /// files add their entries.
    fn add(&self, uris: Vec<String>, play: bool) -> Result<(), String> {
        let mut items = Vec::new();
        let mut subtitles = Vec::new();
        for uri in uris.into_iter().map(playlist_file::native_path) {
            let path = Path::new(&uri);
            if uri.contains("://") {
                items.push(Item::new(uri));
            } else if path.is_dir() {
                // A disc's folder plays as a disc, not as a folder of files.
                match disc::kind_of(path) {
                    Some(kind) => items.extend(disc::items(kind, &uri, true)?),
                    None => {
                        if let Err(e) = self.library().add(&uri) {
                            self.notice("error", e);
                        }
                        items.extend(library::expand(vec![uri]).into_iter().map(Item::new));
                    }
                }
            } else if playlist_file::is_playlist(path) {
                items.extend(playlist_file::read(path)?);
            } else if library::is_subtitle(path) {
                subtitles.push(uri);
            } else {
                items.push(Item::new(uri));
            }
        }
        self.add_with_subtitles(items, subtitles, play)
    }

    /// Plays files the system handed us on the main thread, from another
    /// thread (see the comment above `open`).
    fn add_from_main(self: &Arc<Self>, uris: Vec<String>) {
        let core = self.clone();
        thread::spawn(move || {
            if let Err(e) = core.add(uris, true) {
                core.notice("error", e);
            }
        });
    }

    fn add_items(&self, items: Vec<Item>, play: bool) -> Result<(), String> {
        self.add_with_subtitles(items, Vec::new(), play)
    }

    /// Like VLC, subtitle files go into what's playing, or into the video
    /// opened along with them.
    fn add_with_subtitles(&self, items: Vec<Item>, subtitles: Vec<String>, play: bool) -> Result<(), String> {
        {
            // Checked under the lock `mark_ready` takes, so nothing can land
            // in `pending` after it has been emptied.
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            if !self.ready.load(Ordering::Acquire) {
                pending.items.extend(items);
                pending.subtitles.extend(subtitles);
                return Ok(());
            }
        }
        if !items.is_empty() {
            let first = self.playlist().add(items);
            self.emit_playlist();
            if play {
                self.play_index(first)?;
            }
        }
        for uri in subtitles {
            if self.current_uri().is_none() {
                self.notice("error", "Open a video first, then its subtitles");
                break;
            }
            self.vlc.add_subtitle(&file_uri(&uri))?;
            self.notice("osd", "Subtitles added");
        }
        Ok(())
    }

    fn mark_ready(&self) {
        let pending = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            self.ready.store(true, Ordering::Release);
            std::mem::take(&mut *pending)
        };
        if let Err(e) = self.add_with_subtitles(pending.items, pending.subtitles, true) {
            self.notice("error", e);
        }
    }

    /// Replaces the current item (a playlist URL) with what it lists, and
    /// plays the first. Transition lock held.
    fn expand_current(&self, subitems: Vec<(String, Option<String>)>) -> Res {
        let Some(index) = self.playlist().current else { return Ok(()) };
        let uri = self.current_uri().unwrap_or_default();
        let items = subitems
            .into_iter()
            .map(|(mrl, title)| {
                // Without a title of its own, libVLC reports the location.
                let title = title.filter(|t| *t != mrl && !t.contains("://"));
                let mut item = Item::new(playlist_file::file_uri_path(&mrl).unwrap_or(mrl));
                item.title = title.unwrap_or(item.title);
                item
            })
            .collect();
        self.playlist().replace(index, items);
        // Its entries get their own history; the list itself wasn't watched.
        let mut history = self.history();
        history.remove(&uri);
        let _ = history.save(true);
        drop(history);
        self.play_index_locked(index)
    }

    /// Moves to the next item; `auto` means the current one just finished.
    fn advance(&self, auto: bool) -> Result<(), String> {
        let _transition = self.transition();
        self.advance_locked(auto)
    }

    fn advance_locked(&self, auto: bool) -> Result<(), String> {
        let next = self.playlist().next_index(auto);
        match next {
            Some(i) => self.play_index_locked(i),
            None if auto => Ok(()),
            None => Err("end of playlist".into()),
        }
    }
}

/// Generates library thumbnails one at a time. Video frames wait while
/// video plays so decoding never competes with playback; metadata and
/// artwork go ahead of them meanwhile.
fn make_thumbnails(core: Arc<Core>) {
    loop {
        let job = {
            let mut queue = core.thumbs.queue.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                let playing = core.vlc.state() == State::Playing;
                let next = queue.iter().position(|j| !playing || !matches!(j.kind, JobKind::Frame(_)));
                if let Some(job) = next.and_then(|i| queue.remove(i)) {
                    break job;
                }
                // Idle: write out what was read.
                core.meta.lock().unwrap_or_else(|e| e.into_inner()).save();
                queue = if queue.is_empty() {
                    core.thumbs.wake.wait(queue).unwrap_or_else(|e| e.into_inner())
                } else {
                    // Only frames left, and video playing: check again shortly.
                    core.thumbs.wake.wait_timeout(queue, Duration::from_secs(1)).unwrap_or_else(|e| e.into_inner()).0
                };
            }
        };
        // Only real files can be read by the second player.
        if job.uri.contains("://") {
            continue;
        }
        core.thumbs.tried.lock().unwrap_or_else(|e| e.into_inner()).insert(job.key());
        let result = match job.kind {
            JobKind::Frame(at) => core.vlc.thumbnail(&job.uri, &job.out, at, 480),
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
fn pump_events(core: Arc<Core>, rx: Receiver<(i32, u64)>) {
    const TIME_INTERVAL: Duration = Duration::from_millis(250);
    const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

    let mut last_time_emit = Instant::now() - TIME_INTERVAL;
    let mut last_progress = Instant::now();
    let mut loop_jump: Option<Instant> = None;
    while let Ok(first) = rx.recv() {
        // Let bursts of events (e.g. opening a file) arrive, then handle them together.
        thread::sleep(Duration::from_millis(20));
        let batch: Vec<(i32, u64)> = std::iter::once(first).chain(rx.try_iter()).collect();
        let events: Vec<i32> = batch.iter().map(|&(ty, _)| ty).collect();
        let has = |ty: i32| events.contains(&ty);
        // Ending or failing only counts for what's open now, not for
        // media replaced since (the user picked another item meanwhile).
        let transition = core.transition();
        let generation = core.vlc.generation();
        let current = |ty: i32| batch.contains(&(ty, generation));

        // Web pages and remote playlists end by expanding into what they hold:
        // one stream plays in place, several replace the entry in the queue.
        let subitems = if current(event::END_REACHED) { core.vlc.subitems() } else { Vec::new() };
        if subitems.len() == 1 && core.vlc.play_subitem() {
            if let Some(uri) = core.current_uri() {
                core.set_resume(&uri);
            }
        } else if subitems.len() > 1 {
            if let Err(e) = core.expand_current(subitems) {
                core.notice("error", e);
            }
        } else if current(event::END_REACHED) {
            if let Some(uri) = core.current_uri() {
                let mut history = core.history();
                history.finish(&uri);
                let _ = history.save(true);
            }
            let stop_after_each = core.settings().stop_after_each;
            let sleep_now = std::mem::take(&mut core.sleep().after_item);
            if sleep_now {
                core.notice("info", "Sleep timer: stopped after this item");
            } else if !stop_after_each {
                if let Err(e) = core.advance_locked(true) {
                    core.notice("error", e);
                }
            }
        }
        if current(event::ERROR) {
            let title = {
                let playlist = core.playlist();
                playlist.current.and_then(|i| playlist.items.get(i)).map(|it| it.title.clone())
            };
            core.notice("error", format!("Could not play {}", title.unwrap_or_else(|| "this media".into())));
        }
        drop(transition);
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
                let _ = history.save(false);
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

// Commands that change what plays run off the main thread: they wait for
// the transition lock, and the event thread can hold it while libVLC needs
// the main thread to close the previous video.

#[tauri::command]
async fn open(player: tauri::State<'_, Player>, uris: Vec<String>, play: bool) -> Res {
    off_main(player, move |core| core.add(uris, play)).await?
}

#[tauri::command]
async fn play_index(player: tauri::State<'_, Player>, index: usize) -> Res {
    off_main(player, move |core| core.play_index(index)).await?
}

#[tauri::command]
async fn toggle_pause(player: tauri::State<'_, Player>) -> Res {
    off_main(player, toggle_pause_now).await?
}

fn toggle_pause_now(core: &Core) -> Res {
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
async fn stop(player: tauri::State<'_, Player>) -> Res {
    off_main(player, Core::stop).await
}

/// Runs `work` on a background thread: Tauri runs synchronous commands on
/// the main thread, where disk access would freeze the window.
async fn off_main<T: Send + 'static>(player: tauri::State<'_, Player>, work: impl FnOnce(&Core) -> T + Send + 'static) -> Result<T, String> {
    let core = player.core()?.clone();
    tauri::async_runtime::spawn_blocking(move || work(&core)).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn history_list(player: tauri::State<'_, Player>) -> Result<Vec<UiEntry>, String> {
    off_main(player, history_entries).await
}

fn history_entries(core: &Core) -> Vec<UiEntry> {
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
    list
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
    settings.save(&core.data_dir)?;
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

/// Runs a menu item that isn't a player action, for the macOS menu bar and
/// the menus the UI draws on Windows. False if `id` isn't one of them.
fn run_app_item(app: &AppHandle, id: &str) -> bool {
    match id {
        "checkUpdates" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { updater::check(&app, true).await });
        }
        "github" => {
            let opener = if cfg!(windows) { "explorer" } else { "open" };
            let _ = std::process::Command::new(opener).arg("https://github.com/HiteshRohira/ReVLC").spawn();
        }
        "quit" => app.exit(0),
        _ => return false,
    }
    true
}

#[tauri::command]
fn app_menu_item(app: AppHandle, id: String) {
    run_app_item(&app, &id);
}

fn show_settings(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("settings") {
        window.unminimize()?;
        return window.set_focus();
    }
    let builder = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html?view=settings".into()))
        .title("Settings")
        .inner_size(760.0, 640.0)
        .min_inner_size(640.0, 480.0)
        .center();
    // The UI draws the title bar: traffic lights over it on macOS, its own
    // window buttons elsewhere.
    #[cfg(target_os = "macos")]
    let builder = builder.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.decorations(false);
    let _window = builder.build()?;
    #[cfg(windows)]
    quiet_webview(&_window);
    Ok(())
}

/// Turns off what WebView2 does as a browser: its shortcuts (F5 and Ctrl+R
/// reload the UI mid-playback, Ctrl+P prints, Ctrl+F finds, F7 turns on caret
/// browsing), form autofill, pinch zoom and swipe navigation. Copy, paste and
/// the other editing keys aren't browser shortcuts and keep working.
#[cfg(windows)]
fn quiet_webview(window: &tauri::WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Settings3, ICoreWebView2Settings4, ICoreWebView2Settings5, ICoreWebView2Settings6,
    };
    use windows::core::Interface;
    let _ = window.with_webview(|webview| unsafe {
        let Ok(settings) = webview.controller().CoreWebView2().and_then(|core| core.Settings()) else { return };
        if let Ok(s) = settings.cast::<ICoreWebView2Settings3>() {
            let _ = s.SetAreBrowserAcceleratorKeysEnabled(false);
        }
        if let Ok(s) = settings.cast::<ICoreWebView2Settings4>() {
            let _ = s.SetIsGeneralAutofillEnabled(false);
            let _ = s.SetIsPasswordAutosaveEnabled(false);
        }
        if let Ok(s) = settings.cast::<ICoreWebView2Settings5>() {
            let _ = s.SetIsPinchZoomEnabled(false);
        }
        if let Ok(s) = settings.cast::<ICoreWebView2Settings6>() {
            let _ = s.SetIsSwipeNavigationEnabled(false);
        }
    });
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
async fn library_list(player: tauri::State<'_, Player>) -> Result<LibraryList, String> {
    off_main(player, Core::library_list).await
}

/// Lists the library from a fresh scan, picking up new files.
#[tauri::command]
async fn library_rescan(player: tauri::State<'_, Player>) -> Result<LibraryList, String> {
    off_main(player, |core| {
        core.rescan_library();
        core.library_list()
    })
    .await
}

#[tauri::command]
async fn library_add(player: tauri::State<'_, Player>, path: String) -> Result<LibraryList, String> {
    off_main(player, move |core| {
        core.library().add(&path)?;
        Ok(core.library_list())
    })
    .await?
}

#[tauri::command]
async fn library_remove(player: tauri::State<'_, Player>, path: String) -> Result<LibraryList, String> {
    off_main(player, move |core| {
        core.library().remove(&path)?;
        Ok(core.library_list())
    })
    .await?
}

#[tauri::command]
fn history_remove(player: tauri::State<Player>, uri: String) -> Res {
    let core = player.core()?;
    let mut history = core.history();
    history.remove(&uri);
    history.save(true)
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
async fn next(player: tauri::State<'_, Player>) -> Res {
    off_main(player, |core| core.advance(false)).await?
}

#[tauri::command]
async fn previous(player: tauri::State<'_, Player>) -> Res {
    off_main(player, previous_now).await?
}

fn previous_now(core: &Core) -> Res {
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
async fn playlist_remove(player: tauri::State<'_, Player>, index: usize) -> Res {
    off_main(player, move |core| {
        let _transition = core.transition();
        let was_current = core.playlist().remove(index);
        if was_current {
            core.vlc.stop();
        }
        core.emit_playlist();
    })
    .await
}

#[tauri::command]
fn playlist_move(player: tauri::State<Player>, from: usize, to: usize) -> Res {
    let core = player.core()?;
    core.playlist().move_item(from, to);
    core.emit_playlist();
    Ok(())
}

#[tauri::command]
async fn playlist_clear(player: tauri::State<'_, Player>) -> Res {
    off_main(player, playlist_clear_now).await
}

fn playlist_clear_now(core: &Core) {
    let _transition = core.transition();
    core.stop_locked();
    core.playlist().clear();
    core.emit_playlist();
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
    core.bookmarks().add(&uri, time, name)?;
    core.emit_bookmarks();
    core.notice("osd", format!("Bookmark added at {}", format_time(time)));
    Ok(())
}

#[tauri::command]
fn bookmark_remove(player: tauri::State<Player>, time: i64) -> Res {
    let core = player.core()?;
    let uri = core.current_uri().ok_or("Nothing is playing")?;
    core.bookmarks().remove(&uri, time)?;
    core.emit_bookmarks();
    Ok(())
}

#[tauri::command]
fn bookmark_rename(player: tauri::State<Player>, time: i64, name: String) -> Res {
    let core = player.core()?;
    let uri = core.current_uri().ok_or("Nothing is playing")?;
    core.bookmarks().rename(&uri, time, &name)?;
    core.emit_bookmarks();
    Ok(())
}

#[tauri::command]
fn disc_list() -> Vec<Disc> {
    disc::detect()
}

/// Plays a disc: a mounted volume, a disc folder or a disc image.
#[tauri::command]
async fn open_disc(player: tauri::State<'_, Player>, kind: DiscKind, path: String, menus: bool) -> Res {
    off_main(player, move |core| core.add_items(disc::items(kind, &path, menus)?, true)).await?
}

/// Lists capture devices. Runs on the main thread (Tauri runs synchronous
/// commands there), which AppKit needs for the screen list.
#[tauri::command]
fn capture_devices() -> Devices {
    capture::devices()
}

#[tauri::command]
async fn open_capture(player: tauri::State<'_, Player>, capture: Capture) -> Res {
    off_main(player, move |core| core.add_items(vec![capture::item(&capture)], true)).await?
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
        Ok(vlc) => {
            vlc.set_hardware_decoding(settings.hardware_decoding);
            vlc
        }
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
        transition: Mutex::default(),
        reloads: AtomicU64::new(0),
        library_scan: Mutex::default(),
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

    // Files passed on the command line, then any Finder opened before now.
    let mut args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();
    args.append(&mut EARLY_OPENS.lock().unwrap_or_else(|e| e.into_inner()));
    let _ = core.add(args, true);

    let window = app.get_webview_window("main").ok_or("main window missing")?;
    #[cfg(windows)]
    quiet_webview(&window);
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
        // Off the main thread, like the commands that open media.
        thread::spawn(move || attach.mark_ready());
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
            core.add_from_main(uris);
        }
    }));
    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(setup)
        // The player is the app: closing it quits, even with Settings open,
        // rather than leaving playback running with no window to show it.
        .on_window_event(|window, event| {
            if window.label() == "main" && matches!(event, WindowEvent::Destroyed) {
                window.app_handle().exit(0);
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "settings" => {
                if let Err(e) = show_settings(app) {
                    eprintln!("revlc: could not open settings: {e}");
                }
            }
            // Everything else is a player action the UI knows how to run.
            id => {
                if !run_app_item(app, id) {
                    let _ = app.emit_to("main", "menu://action", id.to_string());
                }
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
            app_menu_item,
            library_list,
            library_rescan,
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
            match handle.try_state::<Player>() {
                Some(player) => {
                    if let Ok(core) = player.core() {
                        core.add_from_main(uris);
                    }
                }
                None => EARLY_OPENS.lock().unwrap_or_else(|e| e.into_inner()).extend(uris),
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (handle, event);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_skips_devices_and_passwords() {
        assert!(in_history("/Movies/a.mkv"));
        assert!(in_history("https://example.com/live.m3u8?x=a@b"));
        assert!(in_history("rtsp://user@cam.local/stream"));
        assert!(!in_history("rtsp://user:secret@cam.local/stream"));
        assert!(!in_history("dshow://"));
        assert!(!in_history("avcapture://"));
    }
}
