mod bookmarks;
mod capture;
mod cli;
mod container;
mod disc;
mod effects;
mod engine;
mod history;
mod keyframes;
mod library;
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
mod machine;
mod menu;
mod meta;
mod network;
mod playlist;
mod playlist_file;
mod playlists;
mod preview;
mod quality;
mod radio;
mod scrub;
mod settings;
mod store;
mod telemetry;
#[cfg(test)]
mod bench;
#[cfg(test)]
mod data_compat_tests;
#[cfg(test)]
mod prop_tests;
#[cfg(test)]
mod test_media;
#[cfg(target_os = "linux")]
mod surface_linux;
#[cfg(target_os = "macos")]
mod surface_macos;
#[cfg(windows)]
mod surface_windows;
#[cfg(windows)]
mod system_windows;
mod updater;
mod vlc;

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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
use effects::{Adjust, Effects, Equalizer, Filter, FilterSpec, Logo, Marquee, StereoMode, Transform};
use engine::{
    Chapter, Engine, EngineEvent, EqualizerInfo, EventKind, Found, MediaInfo, Navigate, State, Stats,
    SubitemExpansion, Title, Tracks,
};
use history::{History, UiEntry};
use library::{Library, LibraryItem, Song};
use meta::MetaCache;
use network::{Listing, Place, Server, Servers};
use playlist::{Item, Playlist, Repeat, SortKey};
use playlist_file::file_uri;
use playlists::{Playlists, Saved};
use radio::{Favorites, Station};
use settings::{Resume, Settings};
use vlc::VlcEngine;

/// UI event sink abstracted from Tauri's `AppHandle` so `Core` can be tested
/// directly with a mock `Engine`.
trait UiEvents: Send + Sync {
    fn emit_json(&self, event: &str, payload: serde_json::Value);
    fn exit(&self, code: i32);
}

impl UiEvents for AppHandle {
    fn emit_json(&self, event: &str, payload: serde_json::Value) {
        let _ = self.emit(event, payload);
    }

    fn exit(&self, code: i32) {
        AppHandle::exit(self, code);
    }
}

/// Active focus-priority state for a running FFmpeg scrub pass.
struct ScrubActive {
    grid: ScrubGrid,
    ffmpeg: PathBuf,
    accel: keyframes::Accel,
    made: Arc<Mutex<HashSet<i64>>>,
    focus_cancel: Mutex<Arc<keyframes::CancelToken>>,
    focus_in_flight: AtomicUsize,
}

struct Core {
    engine: Arc<dyn Engine>,
    playlist: Mutex<Playlist>,
    ui: Arc<dyn UiEvents>,
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
    /// Parallel pure-Rust metadata scanner (decoupled from thumbnail frame grabs).
    meta_scanner: MetaScanner,
    /// Where cached thumbnails and artwork go.
    thumbs_dir: PathBuf,
    /// Where raw cover images extracted from tags/attachments are cached.
    embedded_art_dir: PathBuf,
    /// Temporary scratch directory for libVLC thumbnail fallback (cleaned on startup).
    scrub_dir: PathBuf,
    /// Serializes `scrub_prepare` and `scrub_cancel` so two concurrent calls can
    /// never share a run ID or cancel token.
    scrub_lock: Mutex<()>,
    /// Bounded in-memory LRU cache of decoded preview JPEGs served via `scrub://`.
    scrub_cache: Arc<preview::cache::ScrubCache>,
    /// Priority queue for the active warm-session preview run (hover before sweep).
    scrub_priority: Mutex<Option<Arc<preview::PriorityQueue>>>,
    /// Bumped whenever previews are asked for again, which ends the previous run.
    scrub_run: AtomicU64,
    /// Cancellation token for the active scrub run (immediately kills child processes).
    scrub_cancel_token: Mutex<Arc<keyframes::CancelToken>>,
    /// Active FFmpeg scrub run metadata so `scrub_focus` can prioritize hovered slots.
    scrub_active: Mutex<Option<Arc<ScrubActive>>>,
    /// Bounds concurrent scrub pass workers across rapid track/video changes.
    scrub_passes_active: Arc<(Mutex<usize>, Condvar)>,
    /// The current run's preview frames still to make, which the pointer
    /// reorders. Each run has its own queue, so a worker from an ended run
    /// can never take (and make, from the wrong video) a later run's work.
    scrub_jobs: Mutex<Arc<Mutex<VecDeque<ScrubJob>>>>,
    meta: Mutex<MetaCache>,
    bookmarks: Mutex<Bookmarks>,
    /// The playlists the user saved.
    playlists: Mutex<Playlists>,
    /// Saved network servers.
    servers: Mutex<Servers>,
    /// Favourite radio stations.
    radio: Mutex<Favorites>,
    /// A–B loop points in ms.
    ab_loop: Mutex<(Option<i64>, Option<i64>)>,
    sleep: Mutex<SleepTimer>,
    /// Held while changing what plays, so a finished item can't advance
    /// the queue while the user is picking another.
    transition: Mutex<()>,
    /// Bumped by each settings change that reopens the video; only the last
    /// one of a burst does it.
    reloads: AtomicU64,
    /// Subtitle files added to what's playing, to add again when a settings
    /// change reopens it.
    subtitles: Mutex<Vec<String>>,
    /// What the UI is told before it's listening, handed over with its
    /// first snapshot. `None` once it has been.
    early: Mutex<Option<Early>>,
    /// The last library scan, reused for a little while.
    library_scan: Mutex<Option<LibraryScan>>,
    /// Playback quality measurements, for analytics.
    quality: Mutex<quality::Quality>,
    /// Wakes the thread that writes the history (see [`Core::save_history`]).
    history_saves: mpsc::Sender<()>,
    /// Held while writing the history, so writes land in order.
    history_write: Mutex<()>,
    /// Held while applying a settings change, so changes apply in order.
    settings_change: Mutex<()>,
    /// Equaliser, stereo mode, video effects and zoom; they stay on from
    /// one item to the next.
    effects: Mutex<Effects>,
    /// `--fullscreen`: video goes fullscreen when it starts, this session.
    start_fullscreen: bool,
    /// `--play-and-exit`: quit when the queue has played to its end.
    play_and_exit: bool,
}

struct LibraryScan {
    folders: Vec<String>,
    at: Instant,
    found: library::Scan,
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

/// Metadata jobs waiting to be scanned in parallel by pure-Rust parsers
/// (with `Engine::parse` fallback for exotic formats).
#[derive(Default)]
struct MetaScanner {
    queue: Mutex<VecDeque<String>>,
    wake: Condvar,
    /// Paths already attempted this session so unreadable files aren't retried forever.
    tried: Mutex<HashSet<String>>,
    /// Throttles `library://thumb` notifications so scanning hundreds of files in
    /// milliseconds never floods the webview with one event per file.
    last_emit: Mutex<Option<Instant>>,
    pending_emit: AtomicBool,
}

/// Thumbnails and artwork waiting to be made, handled by one background thread.
#[derive(Default)]
struct Thumbnails {
    queue: Mutex<VecDeque<ThumbJob>>,
    wake: Condvar,
    /// Jobs already attempted this session, so failures aren't retried forever.
    tried: Mutex<HashSet<String>>,
}

struct ThumbJob {
    uri: String,
    /// The file to write.
    out: PathBuf,
    kind: JobKind,
}

enum JobKind {
    /// A video frame, this far into the media (0–1).
    Frame(f64),
    /// A resized copy of a poster or cover image.
    Art(PathBuf),
}

impl ThumbJob {
    fn key(&self) -> String {
        self.out.to_string_lossy().into_owned()
    }
}

#[derive(Clone, Serialize)]
struct LibraryList {
    folders: Vec<String>,
    /// The videos.
    items: Vec<LibraryItem>,
    music: Vec<Song>,
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
struct Notice {
    kind: &'static str,
    text: String,
}

/// An offer to continue `uri` from `ms` (the "Ask" setting).
#[derive(Clone, Serialize)]
struct ResumeOffer {
    uri: String,
    ms: i64,
}

/// Notices and a resume offer from before the UI was listening (a
/// damaged history, or a file opened on a cold launch).
#[derive(Clone, Default, Serialize)]
struct Early {
    notices: Vec<Notice>,
    resume: Option<ResumeOffer>,
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
    effects: Effects,
    /// `--fullscreen` was given.
    start_fullscreen: bool,
    engine: Result<String, String>,
    early: Early,
}

impl Core {
    fn emit<T: Serialize>(&self, event: &str, payload: T) {
        if let Ok(value) = serde_json::to_value(payload) {
            self.ui.emit_json(event, value);
        }
    }

    fn playlist(&self) -> MutexGuard<'_, Playlist> {
        self.playlist.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn history(&self) -> MutexGuard<'_, History> {
        self.history.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Writes the history if it changed (and, unless `force`, the last
    /// write was a while ago). The history isn't locked during the write.
    fn write_history(&self, force: bool) -> Res {
        let _writing = self.history_write.lock().unwrap_or_else(|e| e.into_inner());
        let Some((file, entries)) = self.history().unsaved(force) else { return Ok(()) };
        store::write_json(&file, &entries, false).inspect_err(|e| {
            eprintln!("revlc: {e}");
            self.history().save_failed();
        })
    }

    /// Writes the history soon, from its own thread: for callers holding
    /// the transition lock, which shouldn't wait for the disk.
    fn save_history(&self) {
        let _ = self.history_saves.send(());
    }

    fn settings(&self) -> MutexGuard<'_, Settings> {
        self.settings.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Saves and applies new settings, reopening the current video if a
    /// change only takes effect when a video starts.
    fn set_settings(self: &Arc<Self>, settings: Settings) -> Res {
        let _changing = self.settings_change.lock().unwrap_or_else(|e| e.into_inner());
        let previous = self.settings().clone();
        if previous == settings {
            return Ok(());
        }
        // Saved first: if that fails, nothing has changed.
        settings.save(&self.data_dir)?;
        if previous.seekbar_preview && !settings.seekbar_preview {
            self.scrub_cancel();
        }
        let engine_changed = self.engine.apply_settings(&settings, Some(&previous), &self.languages);
        telemetry::set_enabled(settings.analytics);
        *self.settings() = settings.clone();
        self.emit("settings://changed", settings);
        if engine_changed {
            let reload = self.clone();
            thread::spawn(move || reload.reload());
        }
        Ok(())
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
            let state = self.engine.state();
            if !matches!(state, State::Playing | State::Paused) {
                return;
            }
            *self.resume.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(PendingSeek::Seek { ms: self.engine.time(), announce: false });
            let subtitles_on = self.engine.subtitle_track() >= 0;
            self.quality().close("settings_changed");
            if self.engine.open(&item.uri, &item.options, item.trusted).is_ok() {
                for uri in self.subtitles().iter() {
                    let _ = self.engine.add_subtitle(uri, subtitles_on);
                }
            }
            (state, self.engine.generation())
        };
        if state == State::Paused {
            // Let it show a frame at the old position, then pause again,
            // unless something else has been opened since.
            thread::sleep(Duration::from_millis(400));
            let _transition = self.transition();
            if self.engine.generation() == generation {
                self.engine.set_pause(true);
            }
        }
    }

    fn subtitles(&self) -> MutexGuard<'_, Vec<String>> {
        self.subtitles.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds a subtitle file or URL to what's playing and selects it.
    fn add_subtitle(&self, uri: String) -> Res {
        // So it can't land on an item that's being replaced.
        let _transition = self.transition();
        self.engine.add_subtitle(&uri, true)?;
        self.subtitles().push(uri);
        Ok(())
    }

    fn quality(&self) -> MutexGuard<'_, quality::Quality> {
        self.quality.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Tells the quality measurements about a seek the user asked for.
    fn seeking(&self, target: i64, kind: &'static str) {
        self.quality().seek_requested(&*self.engine, self.engine.time(), target, kind);
    }

    fn library(&self) -> MutexGuard<'_, Library> {
        self.library.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Lists the library. Reads folders and files, so call it off the main thread.
    fn library_list(&self) -> LibraryList {
        let folders = self.library().folders.clone();
        let (library::Scan { videos: mut items, mut music }, scanned) = {
            let mut cache = self.library_scan.lock().unwrap_or_else(|e| e.into_inner());
            match cache.as_ref().filter(|c| c.folders == folders && c.at.elapsed() < LIBRARY_SCAN_TTL) {
                Some(c) => (c.found.clone(), false),
                None => {
                    let found = library::scan_folders(&folders);
                    *cache = Some(LibraryScan { folders: folders.clone(), at: Instant::now(), found: found.clone() });
                    (found, true)
                }
            }
        };
        // Queue thumbnails for anything that doesn't have one yet, or (on a
        // fresh scan) whose file has changed since (the old one shows meanwhile).
        // A replaced file can be older than its thumbnail, so a change the
        // metadata cache noticed counts too.
        let changed: HashSet<String> = if scanned {
            let cache = self.meta.lock().unwrap_or_else(|e| e.into_inner());
            items.iter().filter(|item| cache.is_stale(&item.uri)).map(|item| item.uri.clone()).collect()
        } else {
            HashSet::new()
        };
        let mut jobs = Vec::new();
        let mut meta_jobs = Vec::new();
        let mut retry_thumbs = Vec::new();
        let mut retry_meta = Vec::new();
        for item in &mut items {
            let thumb = self.library_thumb_path(&item.uri);
            let made = meta::modified(&thumb);
            if made > 0 {
                item.thumb = Some(thumb.to_string_lossy().into_owned());
            }
            if made == 0 || (scanned && (made < meta::modified(Path::new(&item.uri)) || changed.contains(&item.uri))) {
                if made > 0 {
                    retry_thumbs.push(thumb.to_string_lossy().into_owned());
                }
                jobs.push(ThumbJob { uri: item.uri.clone(), out: thumb, kind: JobKind::Frame(0.3) });
            }
        }
        retry_meta.extend(self.add_metadata(&mut items, &mut jobs, &mut meta_jobs));
        retry_meta.extend(self.add_music_metadata(&mut music, &mut jobs, &mut meta_jobs));
        // Changed files are worth another try even if they were tried this session.
        self.thumbs.tried.lock().unwrap_or_else(|e| e.into_inner()).retain(|key| !retry_thumbs.contains(key));
        self.meta_scanner.tried.lock().unwrap_or_else(|e| e.into_inner()).retain(|key| !retry_meta.contains(key));
        // Only when every folder is there: files on an unplugged drive keep their thumbnails.
        if scanned && folders.iter().all(|f| Path::new(f).is_dir()) {
            self.prune_library_cache(&items, &music, &jobs);
        }
        self.queue_meta_jobs(meta_jobs);
        self.queue_thumbnails(jobs, true);
        LibraryList { folders, items, music }
    }

    /// Queues metadata scan jobs for the parallel pure-Rust metadata scanner pool.
    fn queue_meta_jobs(&self, uris: Vec<String>) {
        if uris.is_empty() {
            return;
        }
        let tried = self.meta_scanner.tried.lock().unwrap_or_else(|e| e.into_inner());
        let mut queue = self.meta_scanner.queue.lock().unwrap_or_else(|e| e.into_inner());
        let mut queued: HashSet<String> = queue.iter().cloned().collect();
        let mut added = false;
        for uri in uris {
            if !tried.contains(&uri) && queued.insert(uri.clone()) {
                queue.push_back(uri);
                added = true;
            }
        }
        if added {
            self.meta_scanner.wake.notify_all();
        }
    }

    /// Fills in song titles, artists, albums and covers, queueing jobs for
    /// whatever hasn't been read or cached yet. Returns the URIs of files
    /// read before that have changed since.
    fn add_music_metadata(&self, tracks: &mut [Song], jobs: &mut Vec<ThumbJob>, meta_jobs: &mut Vec<String>) -> Vec<String> {
        let mut changed = Vec::new();
        let cache = self.meta.lock().unwrap_or_else(|e| e.into_inner());
        let mut images = meta::Images::default();
        for track in tracks {
            let path = Path::new(&track.uri);
            let (number, title) = meta::parse_track_name(&track.name);
            let tags = match cache.get(&track.uri) {
                Some(m) => m.clone(),
                None => {
                    if cache.is_stale(&track.uri) {
                        changed.push(track.uri.clone());
                    }
                    meta_jobs.push(track.uri.clone());
                    Default::default()
                }
            };
            // libVLC reports the file name as the title when there's no tag
            // (and the scanner drops it, but older caches can have it). A tag
            // that matches the name without its extension is a real title.
            let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            track.name = tags.title.filter(|t| *t != file).unwrap_or(title);
            track.track = tags.track.or(number);
            track.disc = tags.disc;
            track.artist = tags.artist;
            track.album = tags.album;
            track.album_artist = tags.album_artist;
            track.genre = tags.genre;
            track.year = tags.date.as_deref().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok());
            track.duration = tags.duration;
            // The cover inside the file, or a cover file next to the songs.
            track.art = tags.artwork.map(PathBuf::from).or_else(|| images.album_cover(path)).and_then(|src| {
                let out = meta::art_cache_path(&self.thumbs_dir, &src);
                if out.exists() {
                    return Some(out.to_string_lossy().into_owned());
                }
                jobs.push(ThumbJob { uri: src.to_string_lossy().into_owned(), out, kind: JobKind::Art(src) });
                None
            });
        }
        changed
    }

    fn library_thumb_path(&self, uri: &str) -> PathBuf {
        self.history().library_thumb_path(uri)
    }

    /// Deletes cached thumbnails, artwork and metadata of files no longer
    /// in the library (or artwork that has since changed).
    fn prune_library_cache(&self, items: &[LibraryItem], music: &[Song], jobs: &[ThumbJob]) {
        let mut keep: HashSet<PathBuf> = jobs.iter().map(|j| j.out.clone()).collect();
        for item in items {
            keep.insert(self.library_thumb_path(&item.uri));
            keep.extend([&item.art, &item.group_art].into_iter().flatten().map(PathBuf::from));
        }
        keep.extend(music.iter().filter_map(|t| t.art.as_ref()).map(PathBuf::from));
        // Job keys of what's deleted, so it's made again if the file comes back.
        let mut forget_thumbs: HashSet<String> = HashSet::new();
        if let Ok(entries) = std::fs::read_dir(&self.thumbs_dir) {
            for path in entries.flatten().map(|e| e.path()) {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if (name.starts_with("lib-") || name.starts_with("art-")) && !keep.contains(&path) {
                    let _ = std::fs::remove_file(&path);
                    forget_thumbs.insert(path.to_string_lossy().into_owned());
                }
            }
        }
        let uris: HashSet<&str> = items.iter().map(|i| i.uri.as_str()).chain(music.iter().map(|t| t.uri.as_str())).collect();
        let mut meta = self.meta.lock().unwrap_or_else(|e| e.into_inner());
        let forget_meta: HashSet<String> = meta.retain(&uris).into_iter().collect();
        let keep_embedded = meta.referenced_artwork();
        meta.save();
        drop(meta);
        if let Ok(entries) = std::fs::read_dir(&self.embedded_art_dir) {
            for path in entries.flatten().map(|e| e.path()) {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if name.starts_with("emb-") && !keep_embedded.contains(&path) {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        self.thumbs.tried.lock().unwrap_or_else(|e| e.into_inner()).retain(|key| !forget_thumbs.contains(key));
        self.meta_scanner.tried.lock().unwrap_or_else(|e| e.into_inner()).retain(|uri| !forget_meta.contains(uri));
    }

    /// Forgets the last library scan, so the next list sees new files.
    fn rescan_library(&self) {
        *self.library_scan.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Fills in titles, episode numbers, durations and artwork, queueing
    /// jobs for whatever hasn't been read or cached yet. Returns the URIs of
    /// files read before that have changed since.
    fn add_metadata(&self, items: &mut [LibraryItem], jobs: &mut Vec<ThumbJob>, meta_jobs: &mut Vec<String>) -> Vec<String> {
        let mut changed = Vec::new();
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
                    if cache.is_stale(&item.uri) {
                        changed.push(item.uri.clone());
                    }
                    meta_jobs.push(item.uri.clone());
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
        changed
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
        let (time, length) = (self.engine.time(), self.engine.length());
        if length > 0 && self.engine.has_video() {
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

    /// The current item's title, for messages.
    fn current_title(&self) -> String {
        self.current_item().map_or_else(|| "this media".into(), |it| it.title)
    }

    /// Records the current position, and grabs a thumbnail if there's video.
    fn save_progress(&self, thumbnail: bool) {
        let Some(uri) = self.current_uri() else { return };
        let started = match self.engine.state() {
            State::Playing | State::Paused => true,
            // Still loading, or buffering after a seek: only once it has a position.
            State::Opening | State::Buffering => self.engine.time() > 0,
            _ => false,
        };
        if !started || !in_history(&uri) {
            return;
        }
        if thumbnail {
            self.queue_resume_thumbnail(&uri);
        }
        self.history().update(&uri, self.engine.time(), self.engine.length());
        self.save_history();
    }

    fn emit_chapters(&self) {
        self.emit("player://chapters", self.engine.chapters());
        self.emit("player://titles", self.engine.titles());
    }

    fn bookmarks(&self) -> MutexGuard<'_, Bookmarks> {
        self.bookmarks.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn playlists(&self) -> MutexGuard<'_, Playlists> {
        self.playlists.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn servers(&self) -> MutexGuard<'_, Servers> {
        self.servers.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn radio(&self) -> MutexGuard<'_, Favorites> {
        self.radio.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn current_bookmarks(&self) -> Vec<Bookmark> {
        self.current_uri().map(|uri| self.bookmarks().list(&uri)).unwrap_or_default()
    }

    fn emit_bookmarks(&self) {
        self.emit("player://bookmarks", self.current_bookmarks());
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
                if matches!(core.engine.state(), State::Playing | State::Buffering | State::Opening) {
                    core.engine.set_pause(true);
                    core.notice("info", "Sleep timer: playback paused");
                }
                core.emit_state();
            });
        }
        self.emit_state();
    }

    fn state(&self) -> PlayerState {
        let e = &*self.engine;
        let (loop_a, loop_b) = *self.ab_loop();
        let (sleep_at, sleep_after_item) = {
            let sleep = self.sleep();
            (sleep.at, sleep.after_item)
        };
        PlayerState {
            state: e.state(),
            time: e.time(),
            length: e.length(),
            seekable: e.seekable(),
            has_video: e.has_video(),
            rate: e.rate(),
            volume: e.volume(),
            muted: e.muted(),
            buffering: e.buffering(),
            audio_delay: e.audio_delay(),
            subtitle_delay: e.subtitle_delay(),
            title: e.title(),
            chapter: e.chapter(),
            loop_a,
            loop_b,
            sleep_at,
            sleep_after_item,
        }
    }

    fn tracks(&self) -> Tracks {
        self.engine.tracks()
    }

    fn emit_state(&self) {
        self.emit("player://state", self.state());
    }

    fn emit_tracks(&self) {
        self.emit("player://tracks", self.tracks());
    }

    fn effects(&self) -> MutexGuard<'_, Effects> {
        self.effects.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Changes the effects with `change`, and tells the UI.
    fn update_effects(&self, change: impl FnOnce(&mut Effects)) -> Effects {
        let effects = {
            let mut effects = self.effects();
            change(&mut effects);
            effects.clone()
        };
        self.emit("player://effects", effects.clone());
        effects
    }

    fn emit_playlist(&self) {
        let playlist = self.playlist().clone();
        self.emit("player://playlist", playlist);
    }

    fn notice(&self, kind: &'static str, text: impl Into<String>) {
        let notice = Notice { kind, text: text.into() };
        match self.early().as_mut() {
            Some(early) => early.notices.push(notice),
            None => self.emit("player://notice", notice),
        }
    }

    fn offer_resume(&self, ms: i64) {
        let Some(uri) = self.current_uri() else { return };
        let offer = ResumeOffer { uri, ms };
        match self.early().as_mut() {
            Some(early) => early.resume = Some(offer),
            None => self.emit("player://resume-prompt", offer),
        }
    }

    fn early(&self) -> MutexGuard<'_, Option<Early>> {
        self.early.lock().unwrap_or_else(|e| e.into_inner())
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
            // A start time from the command line says where to start.
            if !item.options.iter().any(|o| o.starts_with(":start-time=")) {
                self.set_resume(uri);
            }
            self.history().touch(uri, title);
            self.save_history();
        }
        self.emit_playlist();
        self.emit_bookmarks();
        self.subtitles().clear();
        let result = self.engine.open(uri, &item.options, item.trusted);
        let resume_ms = match *self.resume.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(PendingSeek::Seek { ms, .. }) => Some(ms),
            _ => None,
        };
        let open = {
            let settings = self.settings();
            quality::Open {
                uri,
                resume_ms,
                hardware_decoding: settings.hardware_decoding,
                file_caching: settings.file_caching,
                network_caching: settings.network_caching,
            }
        };
        self.quality().opened(open, self.engine.generation(), result.as_ref().err().map(String::as_str));
        result
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
        self.scrub_cancel();
        self.save_progress(true);
        self.quality().close("stopped");
        self.engine.stop();
        self.emit_state();
    }

    /// Adds paths or URLs to the playlist, optionally starting the first
    /// new item. Folders add their media (and join the library); playlist
    /// files add their entries.
    fn add(&self, uris: Vec<String>, play: bool) -> Result<(), String> {
        self.add_with_options(uris, play, &[])
    }

    /// [`Core::add`], giving each new item the libVLC `options` (from the
    /// command line, like ":start-time=30").
    fn add_with_options(&self, uris: Vec<String>, play: bool, options: &[String]) -> Result<(), String> {
        let Opened { mut items, subtitles, folders, failed } = opened(uris);
        // A folder joins the library even if nothing in it, or in the rest, can play.
        for folder in folders {
            if let Err(e) = self.library().add(&folder) {
                self.notice("error", e);
            }
        }
        // Nothing at all could be opened: that's the caller's error to show.
        if items.is_empty() && subtitles.is_empty() && !failed.is_empty() {
            return Err(failed.join("; "));
        }
        for e in failed {
            self.notice("error", e);
        }
        for item in &mut items {
            item.options.extend(options.iter().cloned());
        }
        self.add_with_subtitles(items, subtitles, play)
    }

    /// Plays files the system handed us on the main thread, from another
    /// thread (see the comment above `open`).
    fn add_from_main(self: &Arc<Self>, uris: Vec<String>, options: Vec<String>) {
        let core = self.clone();
        thread::spawn(move || {
            if let Err(e) = core.add_with_options(uris, true, &options) {
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
        // libVLC looks for subtitles when it opens a video, so only one
        // opened with it here can already have them.
        let opened_now = !items.is_empty() && play;
        if !items.is_empty() {
            let first = self.playlist().add(items);
            self.emit_playlist();
            if play {
                self.play_index(first)?;
            }
        }
        let (autodetect, fuzzy) = {
            let settings = self.settings();
            (settings.sub_autodetect, settings.sub_autodetect_fuzzy)
        };
        for uri in subtitles {
            let Some(video) = self.current_uri() else {
                self.notice("error", "Open a video first, then its subtitles");
                break;
            };
            // libVLC loads it with the video already; adding it lists it twice.
            if !(opened_now && autodetect && vlc_autodetects(Path::new(&video), Path::new(&uri), fuzzy)) {
                self.add_subtitle(file_uri(&uri))?;
            }
            self.notice("osd", "Subtitles added");
        }
        Ok(())
    }

    fn mark_ready(&self) {
        let damaged = self.history().damaged.take();
        if let Some(aside) = damaged {
            self.notice("error", format!("Your history couldn't be read, so it starts empty. The old file is kept at {}", aside.display()));
        }
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
        self.history().remove(&uri);
        self.save_history();
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

/// What a list of paths and URLs holds.
#[derive(Default)]
struct Opened {
    items: Vec<Item>,
    subtitles: Vec<String>,
    /// Plain folders, which also join the library.
    folders: Vec<String>,
    /// Why some of them couldn't be opened; the rest still are.
    failed: Vec<String>,
}

/// Sorts paths and URLs into what to play: folders give their media,
/// playlist files their entries, and a disc's folder plays as a disc.
fn opened(uris: Vec<String>) -> Opened {
    let mut found = Opened::default();
    for uri in uris.into_iter().map(playlist_file::native_path) {
        let path = Path::new(&uri);
        if uri.contains("://") {
            found.items.push(Item::new(uri));
        } else if path.is_dir() {
            // A disc's folder plays as a disc, not as a folder of files.
            match disc::kind_of(path) {
                Some(kind) => match disc::items(kind, &uri, true) {
                    Ok(items) => found.items.extend(items),
                    Err(e) => found.failed.push(e),
                },
                None => {
                    found.items.extend(library::expand(vec![uri.clone()]).into_iter().map(Item::new));
                    found.folders.push(uri);
                }
            }
        } else if playlist_file::is_playlist(path) {
            match playlist_file::read(path) {
                Ok(items) => found.items.extend(items),
                Err(e) => found.failed.push(e),
            }
        } else if library::is_subtitle(path) {
            found.subtitles.push(uri);
        } else {
            found.items.push(Item::new(uri));
        }
    }
    found
}

/// Parallel library metadata worker: parses MP4, Matroska/WebM, and audio
/// files in pure Rust (milliseconds, zero decoder spawn) and falls back to
/// `Engine::parse` only for exotic container formats.
fn scan_metadata_worker(core: Arc<Core>) {
    const EMIT_INTERVAL: Duration = Duration::from_millis(250);
    let file_sig = |p: &Path| -> Option<(u64, u128)> {
        let m = std::fs::metadata(p).ok()?;
        let t = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
        Some((m.len(), t))
    };
    loop {
        let uri = {
            let mut queue = core.meta_scanner.queue.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(uri) = queue.pop_front() {
                    break uri;
                }
                core.meta.lock().unwrap_or_else(|e| e.into_inner()).save();
                // Flush any coalesced UI update once the batch drains.
                if core.meta_scanner.pending_emit.swap(false, Ordering::AcqRel) {
                    *core.meta_scanner.last_emit.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
                    core.emit("library://thumb", String::new());
                }
                queue = core.meta_scanner.wake.wait(queue).unwrap_or_else(|e| e.into_inner());
            }
        };
        if uri.contains("://") {
            continue;
        }
        core.meta_scanner.tried.lock().unwrap_or_else(|e| e.into_inner()).insert(uri.clone());
        let path = Path::new(&uri);
        let sig_before = file_sig(path);
        let tags = metadata_with_fallback(container::parse_metadata(path, &core.embedded_art_dir), || {
            core.engine.parse(&uri, 5000).map(|meta| without_file_name_title(meta, path))
        });
        // Skip committing if the file was modified/replaced mid-parse so stale tags
        // aren't stamped with the replacement file's mtime.
        if sig_before.is_some() && file_sig(path) != sig_before {
            continue;
        }
        // Remember failures too, so unreadable files aren't parsed on every scan.
        core.meta.lock().unwrap_or_else(|e| e.into_inner()).insert(&uri, tags.clone().unwrap_or_default());
        match tags {
            Ok(_) => {
                let should_emit = {
                    let mut last = core.meta_scanner.last_emit.lock().unwrap_or_else(|e| e.into_inner());
                    if last.is_none_or(|t| t.elapsed() >= EMIT_INTERVAL) {
                        *last = Some(Instant::now());
                        core.meta_scanner.pending_emit.store(false, Ordering::Release);
                        true
                    } else {
                        core.meta_scanner.pending_emit.store(true, Ordering::Release);
                        false
                    }
                };
                if should_emit {
                    core.emit("library://thumb", uri);
                }
            }
            Err(e) => eprintln!("revlc: metadata scan failed: {e}"),
        }
    }
}

/// Drops the title libVLC reports when a file has no title tag: the file's
/// name, with or without its extension.
fn without_file_name_title(mut meta: engine::MediaMeta, path: &Path) -> engine::MediaMeta {
    let names = [path.file_name(), path.file_stem()].map(|n| n.map(|n| n.to_string_lossy().into_owned()));
    if meta.title.as_ref().is_some_and(|t| names.iter().flatten().any(|n| n == t)) {
        meta.title = None;
    }
    meta
}

/// Prefers the pure-Rust metadata and falls back to libVLC when the format
/// isn't handled. Some containers don't record a duration (fragmented MP4
/// with a zero movie-header duration, MKV without Info/Duration), so libVLC
/// fills it in while the Rust tags and artwork are kept.
fn metadata_with_fallback(rust: Option<engine::MediaMeta>, vlc: impl FnOnce() -> Result<engine::MediaMeta, String>) -> Result<engine::MediaMeta, String> {
    match rust {
        None => vlc(),
        Some(mut meta) => {
            if meta.duration <= 0
                && let Ok(v) = vlc()
            {
                meta.duration = v.duration;
            }
            Ok(meta)
        }
    }
}

/// Generates library thumbnails one at a time. Video frames wait while
/// video plays so decoding never competes with playback; artwork goes
/// ahead of them meanwhile.
fn make_thumbnails(core: Arc<Core>) {
    loop {
        let job = {
            let mut queue = core.thumbs.queue.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                let playing = core.engine.state() == State::Playing;
                let next = queue.iter().position(|j| !playing || !matches!(j.kind, JobKind::Frame(_)));
                if let Some(job) = next.and_then(|i| queue.remove(i)) {
                    break job;
                }
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
            JobKind::Frame(at) => core.engine.thumbnail(&job.uri, &job.out, at, 480),
            JobKind::Art(src) => meta::resize_art(&src, &job.out, 360),
        };
        match result {
            Ok(()) => {
                core.emit("library://thumb", job.uri);
            }
            Err(e) => eprintln!("revlc: library job failed: {e}"),
        }
    }
}

/// Turns engine events into UI events. Time updates are throttled so the
/// web view isn't woken more than a few times a second.
fn pump_events(core: Arc<Core>, rx: Receiver<EngineEvent>) {
    const TIME_INTERVAL: Duration = Duration::from_millis(250);
    const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

    let mut last_time_emit = Instant::now() - TIME_INTERVAL;
    let mut last_progress = Instant::now();
    let mut loop_jump: Option<Instant> = None;
    // The last media that got past its start. Some broken files end
    // without an error, having played nothing.
    let mut played = None;
    // How far the current media got (its generation and time), from the
    // times its updates carry: a seek into data a truncated file lacks ends
    // it early, which isn't watching it to the end.
    let mut reached: Option<(u64, i64)> = None;
    // The media whose end or error has been dealt with, so one that sends
    // both, in the same batch of events or apart, is dealt with once.
    let mut over = None;
    // The queue items that were over in a row without playing anything.
    let mut failed: Vec<usize> = Vec::new();
    // The media the queue moved on to by itself. Failures only add up while
    // it does: whatever the user starts next begins a fresh count.
    let mut chained = None;
    while let Ok(first) = rx.recv() {
        // Let bursts of events (e.g. opening a file) arrive, then handle them together.
        thread::sleep(Duration::from_millis(20));
        let batch: Vec<EngineEvent> = std::iter::once(first).chain(rx.try_iter()).collect();
        let events: Vec<EventKind> = batch.iter().map(|e| e.kind).collect();
        let has = |ty: EventKind| events.contains(&ty);
        // Ending or failing only counts for what's open now, not for
        // media replaced since (the user picked another item meanwhile).
        let transition = core.transition();
        let generation = core.engine.generation();
        let current = |ty: EventKind| batch.iter().any(|e| e.kind == ty && e.generation == generation);
        if current(EventKind::TimeChanged) && core.engine.time() > 0 {
            played = Some(generation);
        }
        if let Some(time) = batch
            .iter()
            .filter(|e| e.kind == EventKind::TimeChanged && e.generation == generation && e.time >= 0)
            .map(|e| e.time)
            .next_back()
        {
            reached = Some((generation, time));
        }

        // Web pages and remote playlists end by expanding into what they hold:
        // one stream plays in place, several replace the entry in the queue.
        // Not once its error has been dealt with: what happens to it is decided once.
        let expansion = if current(EventKind::EndReached) && over != Some(generation) {
            core.engine.expand_subitems()
        } else {
            SubitemExpansion::None
        };
        // Before the queue moves on, so a finished item counts as finished.
        if matches!(expansion, SubitemExpansion::None) {
            core.quality().handle(&batch, &*core.engine);
        } else {
            let events: Vec<EngineEvent> = batch.iter().filter(|e| e.kind != EventKind::EndReached).copied().collect();
            core.quality().handle(&events, &*core.engine);
        }
        match expansion {
            SubitemExpansion::Followed => {
                if chained == Some(generation) {
                    chained = Some(core.engine.generation());
                }
                core.quality().followed(core.engine.generation());
                if let Some(uri) = core.current_uri() {
                    core.set_resume(&uri);
                }
            }
            SubitemExpansion::Replace(subitems) => {
                core.quality().forget();
                match core.expand_current(subitems) {
                    Ok(()) if chained == Some(generation) => chained = Some(core.engine.generation()),
                    Ok(()) => {}
                    Err(e) => core.notice("error", e),
                }
            }
            SubitemExpansion::None if (current(EventKind::EndReached) || current(EventKind::Error)) && over != Some(generation) => {
                over = Some(generation);
                let did_play = played == Some(generation);
                if did_play && !current(EventKind::Error) {
                    let reached = reached.filter(|r| r.0 == generation).map_or(0, |r| r.1);
                    if let Some(uri) = core.current_uri()
                        && history::reached_the_end(reached, core.engine.length())
                    {
                        core.history().finish(&uri);
                        core.save_history();
                    }
                } else {
                    // An error part-way keeps its place in the history, to resume from.
                    core.notice("error", format!("Could not play {}", core.current_title()));
                }
                if did_play || chained != Some(generation) {
                    failed.clear();
                }
                if !did_play && let Some(index) = core.playlist().current {
                    failed.push(index);
                }
                let stop_after_each = core.settings().stop_after_each;
                let sleep_now = std::mem::take(&mut core.sleep().after_item);
                // Not what just failed again, and not round and round a queue where nothing plays.
                let next = core.playlist().next_after(&failed);
                // Like VLC's --play-and-exit: quit once there's nothing left to play.
                if core.play_and_exit && next.is_none() {
                    core.ui.exit(0);
                }
                if sleep_now {
                    core.notice("info", "Sleep timer: stopped after this item");
                } else if !stop_after_each && let Some(index) = next {
                    match core.play_index_locked(index) {
                        Ok(()) => chained = Some(core.engine.generation()),
                        Err(e) => core.notice("error", e),
                    }
                }
            }
            SubitemExpansion::None => {}
        }
        // Something else was opened above: the rest of this batch belongs to
        // what was open before, so it mustn't touch the new media's resume point.
        let moved_on = core.engine.generation() != generation;
        // The resume point belongs to what's open now: only its own start
        // may use it, and before anything else can be opened.
        if current(EventKind::Playing) && !moved_on {
            match core.resume.lock().unwrap_or_else(|e| e.into_inner()).take() {
                Some(PendingSeek::Seek { ms, announce }) => {
                    core.engine.set_time(ms);
                    if announce {
                        core.notice("osd", format!("Resuming from {}", format_time(ms)));
                    }
                }
                Some(PendingSeek::Ask(ms)) => core.offer_resume(ms),
                None => {}
            }
        }
        drop(transition);
        if has(EventKind::Playing) {
            let settings = core.settings().clone();
            let effects = core.effects().clone();
            core.engine.on_playing(&settings, &effects);
        }
        if has(EventKind::Paused) {
            core.save_progress(true);
        }
        if has(EventKind::TimeChanged) {
            // A–B loop: back to A whenever playback passes B. Time events
            // can lag a seek, so jump once until we're back before B.
            if let (Some(a), Some(b)) = *core.ab_loop() {
                let time = core.engine.time();
                if time < b {
                    loop_jump = None;
                } else if loop_jump.is_none_or(|at| at.elapsed() > Duration::from_secs(1)) {
                    core.seeking(a, "loop");
                    core.engine.set_time(a);
                    loop_jump = Some(Instant::now());
                }
            }
        }
        if has(EventKind::TimeChanged) && !moved_on && last_progress.elapsed() >= PROGRESS_INTERVAL {
            last_progress = Instant::now();
            if let Some(uri) = core.current_uri() {
                core.history().update(&uri, core.engine.time(), core.engine.length());
                core.save_history();
            }
        }
        if [EventKind::Playing, EventKind::LengthChanged, EventKind::MediaChanged, EventKind::TitleChanged].iter().any(|&e| has(e)) {
            core.emit_chapters();
        }
        if has(EventKind::Playing)
            && let Some(title) = core.engine.media_title()
        {
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
        if has(EventKind::SnapshotTaken) && core.user_snapshot.swap(false, Ordering::AcqRel) {
            core.notice("info", "Snapshot saved");
        }
        if [EventKind::Playing, EventKind::Vout, EventKind::EsAdded, EventKind::EsDeleted, EventKind::EsSelected]
            .iter()
            .any(|&e| has(e))
        {
            core.emit_tracks();
        }

        // Time and buffering progress arrive many times a second; throttle
        // them, but always send the moment buffering finishes.
        let minor = events.iter().all(|&e| e == EventKind::TimeChanged || e == EventKind::Buffering)
            && !(has(EventKind::Buffering) && core.engine.buffering() >= 100.0);
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
async fn snapshot(player: tauri::State<'_, Player>) -> Result<Snapshot, String> {
    if let Err(e) = player.core() {
        return Ok(Snapshot {
            state: None,
            tracks: None,
            playlist: None,
            chapters: Vec::new(),
            titles: Vec::new(),
            bookmarks: Vec::new(),
            effects: Effects::default(),
            start_fullscreen: false,
            engine: Err(e),
            early: Early::default(),
        });
    }
    // Discs can take a while to list their titles and chapters.
    off_main(player, |core| Snapshot {
        // Before the playlist lock below, which lasts for this whole expression.
        bookmarks: core.current_bookmarks(),
        state: Some(core.state()),
        tracks: Some(core.tracks()),
        playlist: Some(core.playlist().clone()),
        chapters: core.engine.chapters(),
        titles: core.engine.titles(),
        effects: core.effects().clone(),
        start_fullscreen: core.start_fullscreen,
        engine: Ok(core.engine.name_and_version()),
        early: core.early().take().unwrap_or_default(),
    })
    .await
}

/// Seek bar preview frames of what's playing: one every `step` ms through
/// the whole video, frame `k` being the file `{prefix}{k}.jpg`.
#[derive(Clone, Serialize)]
struct ScrubInfo {
    /// Tells this run's events from an earlier one's.
    run: u64,
    prefix: String,
    step: i64,
    /// The frames that exist already.
    ready: Vec<i64>,
}

/// The preview frames of one run: frame `k` of `count`, `step` ms apart, is
/// stored in [`Core::scrub_cache`] and served over `scrub://` at `{prefix}{k}.jpg`.
#[derive(Clone)]
struct ScrubGrid {
    run: u64,
    uri: Arc<String>,
    length: i64,
    /// The selected video track, by position among the video tracks.
    track: usize,
    /// Whether libVLC may make frames: only when the file has one video
    /// stream. Its thumbnail player decodes the file's default stream, which
    /// needn't be the first or the selected one, and can't be told otherwise.
    libvlc: bool,
    step: i64,
    count: i64,
    /// The frames still to make with libVLC fallback.
    queue: Arc<Mutex<VecDeque<ScrubJob>>>,
}

/// What previews are made from: the current video's URI and length, the
/// selected video track by position among its video streams (the way FFmpeg's
/// `0:V:n` counts them; libVLC's track ids aren't stream numbers), and how many
/// video streams it has.
struct ScrubSource {
    uri: String,
    length: i64,
    track: usize,
    streams: usize,
}

/// A frame to make via libVLC fallback: how far in, its slot number, and
/// whether to force an exact decode.
struct ScrubJob {
    at: i64,
    k: i64,
    /// Exact whatever the plan: a redo of a frame that repeated its neighbour.
    exact: bool,
}

impl Core {
    /// What previews are made from, if they can be. See [`ScrubSource`].
    fn scrub_source(&self) -> Option<ScrubSource> {
        let uri = self.current_uri()?;
        let length = self.engine.length();
        // Only real local files can be indexed/decoded for seek previews.
        let ok = self.settings().seekbar_preview && !uri.contains("://") && length > 0 && self.engine.has_video() && self.engine.seekable();
        if !ok {
            return None;
        }
        if !Path::new(&uri).is_file() {
            return None;
        }
        let current = self.engine.video_track();
        let tracks: Vec<i32> = self.engine.video_tracks().iter().map(|t| t.id).filter(|&id| id >= 0).collect();
        let track = tracks.iter().position(|&id| id == current).unwrap_or(0);
        Some(ScrubSource { uri, length, track, streams: tracks.len() })
    }

    /// Ends the current preview run immediately: signals cancellation, wakes
    /// any waiting warm-session coordinator, kills any running FFmpeg fallback
    /// children, clears pending queues, and drops cached frames from memory.
    fn scrub_cancel(&self) {
        let _guard = self.scrub_lock.lock().unwrap_or_else(|e| e.into_inner());
        self.scrub_cancel_locked();
    }

    fn scrub_cancel_locked(&self) -> (u64, Arc<keyframes::CancelToken>) {
        let next_run = self.scrub_run.fetch_add(1, Ordering::SeqCst) + 1;
        let next_token = keyframes::CancelToken::new();
        let prev = std::mem::replace(
            &mut *self.scrub_cancel_token.lock().unwrap_or_else(|e| e.into_inner()),
            Arc::clone(&next_token),
        );
        prev.cancel();
        if let Some(priority) = self.scrub_priority.lock().unwrap_or_else(|e| e.into_inner()).take() {
            priority.cancel();
        }
        if let Some(active) = self.scrub_active.lock().unwrap_or_else(|e| e.into_inner()).take() {
            active.focus_cancel.lock().unwrap_or_else(|e| e.into_inner()).cancel();
        }
        self.scrub_jobs.lock().unwrap_or_else(|e| e.into_inner()).lock().unwrap_or_else(|e| e.into_inner()).clear();
        self.scrub_cache.reset_for_run(next_run);
        // Acquire the condvar mutex before notifying so a waiter checking the
        // predicate cannot miss the wakeup.
        let _active_lock = self.scrub_passes_active.0.lock().unwrap_or_else(|e| e.into_inner());
        self.scrub_passes_active.1.notify_all();
        (next_run, next_token)
    }

    /// Starts making seek-bar preview frames for the current video into the
    /// bounded in-memory [`preview::cache::ScrubCache`], served via `scrub://`.
    /// Prefers a warm native hardware decode session (VideoToolbox on macOS,
    /// Media Foundation + D3D11VA on Windows) with instant hover prioritization,
    /// falling back to optional system `ffmpeg` on PATH or libVLC for exotic formats.
    fn scrub_prepare(self: &Arc<Self>) -> Option<ScrubInfo> {
        let _guard = self.scrub_lock.lock().unwrap_or_else(|e| e.into_inner());
        let (run, cancel) = self.scrub_cancel_locked();
        let ScrubSource { uri, length, track, streams } = self.scrub_source()?;
        let step = scrub::step(length);
        let count = ((length - 1) / step + 1).clamp(1, scrub::MAX_SLOTS);
        let prefix = preview::scrub_url_prefix(run);
        let queue = Arc::default();
        *self.scrub_jobs.lock().unwrap_or_else(|e| e.into_inner()) = Arc::clone(&queue);
        let priority = preview::PriorityQueue::new(count);
        *self.scrub_priority.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&priority));
        let libvlc = streams <= 1;
        let grid = ScrubGrid { run, uri: Arc::new(uri), length, track, libvlc, step, count, queue };
        let core = self.clone();
        let _ = thread::Builder::new().name("scrub-preview".into()).spawn(move || {
            core.scrub_coordinate(grid, priority, cancel);
        });
        Some(ScrubInfo { run, prefix, step, ready: Vec::new() })
    }

    /// Tells the UI preview frame `k` is made; false if the run has ended.
    fn scrub_made(&self, grid: &ScrubGrid, k: i64) -> bool {
        if self.scrub_run.load(Ordering::SeqCst) != grid.run {
            return false;
        }
        self.emit("player://scrub", serde_json::json!({ "run": grid.run, "k": k }));
        true
    }

    /// Stores a decoded JPEG into the bounded in-memory LRU cache if not yet
    /// present for this run, and notifies the UI.
    fn scrub_store_frame(&self, grid: &ScrubGrid, made: &Mutex<HashSet<i64>>, k: i64, jpeg: Vec<u8>) -> bool {
        if self.scrub_run.load(Ordering::SeqCst) != grid.run {
            return false;
        }
        if !made.lock().unwrap_or_else(|e| e.into_inner()).insert(k) {
            return self.scrub_run.load(Ordering::SeqCst) == grid.run;
        }
        if !self.scrub_cache.insert_if_active(grid.run, k, jpeg) {
            return false;
        }
        self.scrub_made(grid, k)
    }

    /// Single bounded preview coordinator per active run:
    /// 1. Tries a warm native OS hardware decoder (`WarmSession`) using our
    ///    pure-Rust MP4/MKV/WebM keyframe index (works across all video tracks).
    /// 2. Falls back to optional system `ffmpeg` on PATH if installed (for
    ///    exotic containers/codecs, Linux, or mid-video decoder failures).
    /// 3. Falls back to `Engine::thumbnail_at` (single-video-track files only).
    fn scrub_coordinate(
        self: Arc<Self>,
        grid: ScrubGrid,
        priority: Arc<preview::PriorityQueue>,
        cancel: Arc<keyframes::CancelToken>,
    ) {
        // Bound active preview runs to 1 across rapid track/video switches.
        {
            let (lock, cvar) = &*self.scrub_passes_active;
            let mut active = lock.lock().unwrap_or_else(|e| e.into_inner());
            while *active >= 1 {
                if cancel.is_cancelled() || priority.is_cancelled() || self.scrub_run.load(Ordering::SeqCst) != grid.run {
                    return;
                }
                active = cvar.wait(active).unwrap_or_else(|e| e.into_inner());
            }
            if cancel.is_cancelled() || priority.is_cancelled() || self.scrub_run.load(Ordering::SeqCst) != grid.run {
                return;
            }
            *active += 1;
        }
        struct PassGuard(Arc<(Mutex<usize>, Condvar)>);
        impl Drop for PassGuard {
            fn drop(&mut self) {
                let (lock, cvar) = &*self.0;
                let mut active = lock.lock().unwrap_or_else(|e| e.into_inner());
                *active = active.saturating_sub(1);
                cvar.notify_all();
            }
        }
        let _guard = PassGuard(Arc::clone(&self.scrub_passes_active));
        background_priority();

        // Primary path: warm OS hardware decoder session (zero subprocesses, zero disk I/O).
        let mut succeeded: HashSet<i64> = HashSet::new();
        let mut need_fallback = true;
        if let Ok(mut session) = preview::WarmSession::open(Path::new(&*grid.uri), grid.track) {
            let mut consecutive_errors = 0usize;
            let mut aborted_by_errors = false;
            while let Some(k) = priority.pop_next() {
                if cancel.is_cancelled() || self.scrub_run.load(Ordering::SeqCst) != grid.run {
                    return;
                }
                let target_ms = scrub::time_at(grid.length, grid.step, k);
                if !session.covers(target_ms) {
                    continue;
                }
                match session.decode_nearest(target_ms, scrub::WIDTH) {
                    Ok((_, jpeg)) => {
                        consecutive_errors = 0;
                        if !self.scrub_cache.insert_if_active(grid.run, k, jpeg) {
                            return;
                        }
                        succeeded.insert(k);
                        if !self.scrub_made(&grid, k) {
                            return;
                        }
                    }
                    Err(_) => {
                        consecutive_errors += 1;
                        if consecutive_errors >= 5 {
                            aborted_by_errors = true;
                            break;
                        }
                    }
                }
            }
            // Complete unless we aborted on sustained errors or produced nothing at all.
            if !aborted_by_errors && !succeeded.is_empty() {
                need_fallback = false;
            }
        }

        if !need_fallback || cancel.is_cancelled() || self.scrub_run.load(Ordering::SeqCst) != grid.run {
            return;
        }

        // Past where a truncated file's data runs out there's nothing to show
        // (or play): count those slots as done so no fallback fills them.
        let data_end = container::index_video(Path::new(&*grid.uri))
            .ok()
            .and_then(|m| m.video_tracks.get(grid.track).and_then(|t| t.data_end_ms));
        if let Some(end) = data_end {
            succeeded.extend((0..grid.count).filter(|&k| scrub::time_at(grid.length, grid.step, k) >= end));
        }

        // Fallback 1: system `ffmpeg` on PATH (exotic containers/codecs, Linux, or mid-stream error).
        if let Some(ffmpeg) = keyframes::ffmpeg() {
            self.scrub_pass_ffmpeg(grid, &ffmpeg, succeeded, cancel);
        } else if grid.libvlc {
            // Fallback 2: libVLC thumbnail player (only safe when there is 1 video stream).
            self.scrub_fill(grid, &succeeded, cancel);
        }
    }

    /// Fallback keyframe pass via system FFmpeg, writing decoded JPEGs straight
    /// into the bounded in-memory `ScrubCache`.
    fn scrub_pass_ffmpeg(
        self: Arc<Self>,
        grid: ScrubGrid,
        ffmpeg: &Path,
        have: HashSet<i64>,
        cancel: Arc<keyframes::CancelToken>,
    ) {
        let accel = keyframes::accel_cancelable(ffmpeg, &grid.uri, grid.track, scrub::WIDTH, &cancel);
        if cancel.is_cancelled() || self.scrub_run.load(Ordering::SeqCst) != grid.run {
            return;
        }
        let parts = if accel.gpu() { 4 } else { thread::available_parallelism().map_or(2, |n| n.get() / 2).max(1) as i64 };
        let span = grid.length / parts + 1;
        let made = Arc::new(Mutex::new(have));
        let active_state = Arc::new(ScrubActive {
            grid: grid.clone(),
            ffmpeg: ffmpeg.to_path_buf(),
            accel: accel.clone(),
            made: Arc::clone(&made),
            focus_cancel: Mutex::new(keyframes::CancelToken::new()),
            focus_in_flight: AtomicUsize::new(0),
        });
        *self.scrub_active.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&active_state));

        let passes: Vec<_> = (0..parts)
            .map(|part| {
                let (core, grid, made, accel, ffmpeg, cancel) = (
                    self.clone(),
                    grid.clone(),
                    made.clone(),
                    accel.clone(),
                    ffmpeg.to_path_buf(),
                    Arc::clone(&cancel),
                );
                thread::spawn(move || {
                    keyframes::pass_cancelable(&ffmpeg, &grid.uri, grid.track, &accel, part * span, span, scrub::WIDTH, &cancel, |at, jpeg| {
                        let k = ((at + grid.step / 2) / grid.step).clamp(0, grid.count - 1);
                        core.scrub_store_frame(&grid, &made, k, jpeg)
                    })
                })
            })
            .collect();
        for pass in passes {
            if let Ok(Err(e)) = pass.join() {
                eprintln!("revlc: keyframe pass failed: {e}");
            }
        }
        // Cancel any in-flight focus child and clear active focus state cleanly.
        active_state.focus_cancel.lock().unwrap_or_else(|e| e.into_inner()).cancel();
        {
            let mut slot = self.scrub_active.lock().unwrap_or_else(|e| e.into_inner());
            if slot.as_ref().is_some_and(|a| Arc::ptr_eq(a, &active_state)) {
                *slot = None;
            }
        }
        if !cancel.is_cancelled() && self.scrub_run.load(Ordering::SeqCst) == grid.run && grid.libvlc {
            let made = made.lock().unwrap_or_else(|e| e.into_inner()).clone();
            self.scrub_fill(grid, &made, cancel);
        }
    }

    /// Fallback frame generator using `Engine::thumbnail_at` when neither the
    /// native hardware previewer nor system FFmpeg could handle the file.
    /// Joins all worker threads before returning so the coordinator's active
    /// permit stays held until every worker has exited.
    fn scrub_fill(self: &Arc<Self>, grid: ScrubGrid, have: &HashSet<i64>, cancel: Arc<keyframes::CancelToken>) {
        const WORKERS: usize = 4;
        *grid.queue.lock().unwrap_or_else(|e| e.into_inner()) = scrub::order(grid.count as usize)
            .into_iter()
            .map(|k| k as i64)
            .filter(|k| !have.contains(k))
            .map(|k| ScrubJob { at: scrub::time_at(grid.length, grid.step, k), k, exact: false })
            .collect();
        let plan = Arc::new(scrub::Plan::new());
        let mut workers = Vec::with_capacity(WORKERS);
        for worker_id in 0..WORKERS {
            let (core, grid, plan, cancel) = (self.clone(), grid.clone(), plan.clone(), Arc::clone(&cancel));
            if let Ok(handle) = thread::Builder::new().name("scrub".into()).spawn(move || {
                background_priority();
                loop {
                    if cancel.is_cancelled() || core.scrub_run.load(Ordering::SeqCst) != grid.run {
                        return;
                    }
                    let Some(job) = grid.queue.lock().unwrap_or_else(|e| e.into_inner()).pop_front() else { return };
                    let tmp = std::env::temp_dir().join(format!(
                        "revlc-scrub-{}-{}-{}-{worker_id}.jpg",
                        std::process::id(),
                        grid.run,
                        job.k
                    ));
                    let exact = job.exact || !plan.keyframes();
                    let began = Instant::now();
                    let decoded = core.engine.thumbnail_at(&grid.uri, &tmp, job.at, scrub::WIDTH, exact);
                    let bytes = if decoded.is_ok() { std::fs::read(&tmp).ok() } else { None };
                    let _ = std::fs::remove_file(&tmp);
                    if let Some(jpeg) = bytes
                        && !cancel.is_cancelled()
                        && core.scrub_run.load(Ordering::SeqCst) == grid.run
                    {
                        if exact && !job.exact {
                            plan.exact_took(began.elapsed());
                        }
                        if !exact {
                            let redo = [(job.k - 1, job.k), (job.k + 1, job.k + 1)]
                                .into_iter()
                                .find(|&(other, _)| {
                                    other >= 0
                                        && core
                                            .scrub_cache
                                            .get(grid.run, other)
                                            .is_some_and(|prev| scrub::same_picture_bytes(&jpeg, &prev))
                                })
                                .map(|(_, later)| later);
                            if let Some(k) = redo {
                                let job = ScrubJob { at: scrub::time_at(grid.length, grid.step, k), k, exact: true };
                                grid.queue.lock().unwrap_or_else(|e| e.into_inner()).push_back(job);
                            }
                        }
                        if !core.scrub_cache.insert_if_active(grid.run, job.k, jpeg) {
                            return;
                        }
                        if !core.scrub_made(&grid, job.k) {
                            return;
                        }
                    }
                }
            }) {
                workers.push(handle);
            }
        }
        for w in workers {
            let _ = w.join();
        }
    }

    /// Moves the frames `from..=to` still to make to the front of the queue,
    /// the middle first: they're what the user is looking at. For the warm
    /// native decoder this reorders its priority queue immediately with zero
    /// subprocesses; for an active FFmpeg fallback pass it also triggers a
    /// bounded focused decode.
    fn scrub_focus(self: &Arc<Self>, from: i64, to: i64) {
        if let Some(priority) = self.scrub_priority.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            priority.prioritize(from, to);
        }

        let (from, to) = (from.min(to), from.max(to));
        let mid = (from + to) / 2;
        let queue = Arc::clone(&self.scrub_jobs.lock().unwrap_or_else(|e| e.into_inner()));
        {
            let mut queue = queue.lock().unwrap_or_else(|e| e.into_inner());
            let (mut wanted, rest): (Vec<ScrubJob>, Vec<ScrubJob>) = queue.drain(..).partition(|j| (from..=to).contains(&j.k));
            wanted.sort_by_key(|j| (j.k - mid).abs());
            queue.extend(wanted);
            queue.extend(rest);
        }

        // Immediate hover priority during an active FFmpeg fallback pass.
        let active = self.scrub_active.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(active) = active else { return };
        if self.scrub_run.load(Ordering::SeqCst) != active.grid.run {
            return;
        }
        let lo = from.clamp(0, active.grid.count - 1);
        let hi = to.clamp(0, active.grid.count - 1);
        {
            let made = active.made.lock().unwrap_or_else(|e| e.into_inner());
            if (lo..=hi).all(|k| made.contains(&k)) {
                return;
            }
        }
        let focus_token = keyframes::CancelToken::new();
        {
            let mut slot = active.focus_cancel.lock().unwrap_or_else(|e| e.into_inner());
            slot.cancel();
            *slot = Arc::clone(&focus_token);
        }
        if active.focus_in_flight.fetch_add(1, Ordering::AcqRel) >= 2 {
            active.focus_in_flight.fetch_sub(1, Ordering::AcqRel);
            return;
        }
        let start_ms = (lo * active.grid.step).saturating_sub(active.grid.step).max(0);
        let len_ms = ((hi - lo + 3) * active.grid.step).min(active.grid.length - start_ms).max(active.grid.step);
        let core = self.clone();
        let _ = thread::Builder::new().name("scrub-focus".into()).spawn(move || {
            let _ = keyframes::pass_cancelable(
                &active.ffmpeg,
                &active.grid.uri,
                active.grid.track,
                &active.accel,
                start_ms,
                len_ms,
                scrub::WIDTH,
                &focus_token,
                |at, jpeg| {
                    let k = ((at + active.grid.step / 2) / active.grid.step).clamp(0, active.grid.count - 1);
                    core.scrub_store_frame(&active.grid, &active.made, k, jpeg)
                },
            );
            active.focus_in_flight.fetch_sub(1, Ordering::AcqRel);
        });
    }
}

#[tauri::command]
async fn scrub_prepare(player: tauri::State<'_, Player>) -> Result<Option<ScrubInfo>, String> {
    let core = player.core()?.clone();
    tauri::async_runtime::spawn_blocking(move || core.scrub_prepare()).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn scrub_cancel(player: tauri::State<'_, Player>) -> Res {
    player.core()?.scrub_cancel();
    Ok(())
}

#[tauri::command]
fn scrub_focus(player: tauri::State<'_, Player>, from: i64, to: i64) -> Res {
    player.core()?.scrub_focus(from, to);
    Ok(())
}

/// Lowers the calling thread's priority, so preview frames give way to
/// playback. libVLC's threads for the frame's player inherit it.
fn background_priority() {
    #[cfg(target_os = "linux")]
    // SAFETY: gettid has no preconditions.
    keyframes::lower_priority(unsafe { libc::syscall(libc::SYS_gettid) } as u32);
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
    match core.engine.state() {
        State::Playing | State::Buffering | State::Opening => core.engine.set_pause(true),
        State::Paused => core.engine.set_pause(false),
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
async fn settings_get(player: tauri::State<'_, Player>) -> Result<SettingsInfo, String> {
    // Listing audio devices can be slow.
    off_main(player, |core| SettingsInfo {
        settings: core.settings().clone(),
        defaults: Settings::default(),
        system_languages: core.languages.clone(),
        audio_devices: core.engine.audio_devices(),
        fonts: core.fonts.clone(),
    })
    .await
}

/// Saves new settings, applies them, and reopens the current video if a
/// change only takes effect when a video starts.
#[tauri::command]
async fn settings_set(player: tauri::State<'_, Player>, settings: Settings) -> Res {
    let core = player.core()?.clone();
    tauri::async_runtime::spawn_blocking(move || core.set_settings(settings)).await.map_err(|e| e.to_string())?
}

/// Seeks to a resume point the user accepted.
#[tauri::command]
fn resume_at(player: tauri::State<Player>, ms: i64) -> Res {
    let core = player.core()?;
    core.seeking(ms, "resume");
    core.engine.set_time(ms);
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
        "github" => open_externally("https://github.com/HiteshRohira/ReVLC"),
        // macOS has its own About panel in the app menu; these are for the
        // menus the UI draws elsewhere.
        "about" => {
            use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
            let text = format!(
                "ReVLC {}\n\nA media player built on libVLC.\nThe licences of ReVLC and the software it includes are under Help → Third-Party Licences.",
                app.package_info().version
            );
            app.dialog().message(text).title("About ReVLC").kind(MessageDialogKind::Info).show(|_| {});
        }
        "licenses" => {
            if let Ok(dir) = app.path().resource_dir() {
                // The Linux package keeps "../licenses/" under "_up_".
                let folder = ["licenses", "_up_/licenses"].map(|p| dir.join(p)).into_iter().find(|p| p.is_dir());
                // Explorer can't open a \\?\ path.
                open_externally(dunce::simplified(&folder.unwrap_or(dir)));
            }
        }
        "quit" => app.exit(0),
        _ => return false,
    }
    true
}

/// Opens a web page or folder in the system's app for it.
fn open_externally(target: impl AsRef<std::ffi::OsStr>) {
    let opener = if cfg!(windows) { "explorer" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = std::process::Command::new(opener).arg(target).spawn();
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
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::*;
        let context = gtk::Label::new(None).pango_context();
        let mut fonts: Vec<String> = context.list_families().iter().map(|f| f.name().to_string()).collect();
        fonts.sort_by_key(|f| f.to_lowercase());
        fonts
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
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
async fn history_remove(player: tauri::State<'_, Player>, uri: String) -> Res {
    off_main(player, move |core| {
        core.history().remove(&uri);
        core.write_history(true)
    })
    .await?
}

#[tauri::command]
fn seek(player: tauri::State<Player>, ms: i64) -> Res {
    let core = player.core()?;
    core.seeking(ms, "jump");
    core.engine.set_time(ms);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn seek_by(player: tauri::State<Player>, delta_ms: i64) -> Res {
    let core = player.core()?;
    let target = core.engine.time() + delta_ms;
    let length = core.engine.length();
    let target = if length > 0 { target.min(length - 500) } else { target };
    core.seeking(target, "skip");
    core.engine.set_time(target);
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
    if core.engine.time() > 3000 {
        core.engine.set_time(0);
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
    core.engine.next_frame();
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_volume(player: tauri::State<Player>, volume: i32) -> Res {
    let core = player.core()?;
    core.engine.set_volume(volume);
    if volume > 0 && core.engine.muted() {
        core.engine.set_muted(false);
    }
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_muted(player: tauri::State<Player>, muted: bool) -> Res {
    let core = player.core()?;
    core.engine.set_muted(muted);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_rate(player: tauri::State<Player>, rate: f32) -> Res {
    let core = player.core()?;
    core.engine.set_rate(rate);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_audio_track(player: tauri::State<Player>, id: i32) -> Res {
    let core = player.core()?;
    core.engine.set_audio_track(id);
    core.emit_tracks();
    Ok(())
}

#[tauri::command]
fn set_subtitle_track(player: tauri::State<Player>, id: i32) -> Res {
    let core = player.core()?;
    core.engine.set_subtitle_track(id);
    core.emit_tracks();
    Ok(())
}

#[tauri::command]
fn add_subtitle(player: tauri::State<Player>, uri: String) -> Res {
    let core = player.core()?;
    let uri = if uri.contains("://") { uri } else { file_uri(&uri) };
    core.add_subtitle(uri)
}

#[tauri::command]
fn set_audio_delay(player: tauri::State<Player>, ms: i64) -> Res {
    let core = player.core()?;
    core.engine.set_audio_delay(ms);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_subtitle_delay(player: tauri::State<Player>, ms: i64) -> Res {
    let core = player.core()?;
    core.engine.set_subtitle_delay(ms);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_aspect_ratio(player: tauri::State<Player>, ratio: Option<String>) -> Res {
    player.core()?.engine.set_aspect_ratio(ratio.as_deref());
    Ok(())
}

#[tauri::command]
fn set_crop(player: tauri::State<Player>, geometry: Option<String>) -> Res {
    player.core()?.engine.set_crop(geometry.as_deref());
    Ok(())
}

#[tauri::command]
fn set_deinterlace(player: tauri::State<Player>, mode: Option<String>) -> Res {
    player.core()?.engine.set_deinterlace(mode.as_deref());
    Ok(())
}

#[tauri::command]
fn set_video_track(player: tauri::State<Player>, id: i32) -> Res {
    let core = player.core()?;
    core.engine.set_video_track(id);
    core.emit_tracks();
    Ok(())
}

#[tauri::command]
fn set_stereo_mode(player: tauri::State<Player>, mode: StereoMode) -> Res {
    let core = player.core()?;
    // Kept even with nothing playing; it's applied when playback starts.
    core.engine.set_stereo_mode(mode);
    core.update_effects(|e| e.stereo = mode);
    Ok(())
}

#[tauri::command]
fn equalizer_info(player: tauri::State<Player>) -> Result<EqualizerInfo, String> {
    Ok(player.core()?.engine.equalizer_info())
}

fn apply_equalizer(core: &Core, eq: Equalizer) {
    let effects = core.update_effects(|e| e.equalizer = eq);
    core.engine.apply_equalizer(&effects);
}

/// Loads one of the engine's presets and turns the equaliser on.
#[tauri::command]
fn set_equalizer_preset(player: tauri::State<Player>, index: u32) -> Res {
    let core = player.core()?;
    let (preamp, bands) = core.engine.equalizer_preset(index).ok_or("no such preset")?;
    let band_count = core.engine.equalizer_info().bands.len();
    let eq = Equalizer { enabled: true, preset: Some(index), preamp, bands }.normalized(band_count);
    apply_equalizer(core, eq);
    Ok(())
}

/// Sets the equaliser from the UI's values. Once they no longer match the
/// preset they came from, it counts as a custom setting.
#[tauri::command]
fn set_equalizer(player: tauri::State<Player>, equalizer: Equalizer) -> Res {
    let core = player.core()?;
    let band_count = core.engine.equalizer_info().bands.len();
    let mut eq = equalizer.normalized(band_count);
    if let Some(index) = eq.preset
        && !core.engine.equalizer_preset(index).is_some_and(|(preamp, bands)| eq.matches(preamp, &bands))
    {
        eq.preset = None;
    }
    apply_equalizer(core, eq);
    Ok(())
}

#[tauri::command]
fn set_adjust(player: tauri::State<Player>, adjust: Adjust) -> Res {
    let core = player.core()?;
    let adjust = adjust.normalized();
    let effects = core.update_effects(|e| e.adjust = adjust);
    core.engine.apply_adjust(&effects);
    Ok(())
}

#[derive(Serialize)]
struct FilterSpecs {
    video: &'static [FilterSpec],
    audio: &'static [FilterSpec],
}

/// The video and audio filters the effects panel offers.
#[tauri::command]
fn filter_specs() -> FilterSpecs {
    FilterSpecs { video: effects::VIDEO_FILTERS, audio: effects::AUDIO_FILTERS }
}

/// Turns a video or audio filter on or off, or changes its settings.
#[tauri::command]
fn set_filter(player: tauri::State<Player>, id: String, filter: Filter) -> Res {
    let core = player.core()?;
    let audio = effects::AUDIO_FILTERS.iter().any(|s| s.id == id);
    let (prev, next) = {
        let mut effects = core.effects();
        let prev = effects.clone();
        if !effects.set_filter(&id, filter) {
            return Err(format!("no such filter: {id}"));
        }
        (prev, effects.clone())
    };
    if audio {
        core.engine.apply_audio_filters(&next, &prev);
    } else {
        core.engine.apply_video_filters(&next);
    }
    core.update_effects(|_| {});
    Ok(())
}

#[tauri::command]
fn set_transform(player: tauri::State<Player>, transform: Transform) -> Res {
    let core = player.core()?;
    let effects = core.update_effects(|e| e.transform = transform);
    core.engine.apply_video_filters(&effects);
    Ok(())
}

#[tauri::command]
fn set_logo(player: tauri::State<Player>, logo: Logo) -> Res {
    let core = player.core()?;
    let logo = logo.normalized();
    core.engine.apply_logo(&logo);
    core.update_effects(|e| e.logo = logo);
    Ok(())
}

#[tauri::command]
fn set_marquee(player: tauri::State<Player>, marquee: Marquee) -> Res {
    let core = player.core()?;
    let marquee = marquee.normalized();
    core.engine.apply_marquee(&marquee);
    core.update_effects(|e| e.marquee = marquee);
    Ok(())
}

#[tauri::command]
fn set_zoom(player: tauri::State<Player>, zoom: f32) -> Res {
    let core = player.core()?;
    let zoom = effects::zoom_preset(zoom);
    core.engine.set_zoom(zoom);
    core.update_effects(|e| e.zoom = zoom);
    Ok(())
}

/// Shows a teletext page, sends a coloured key, or with 0 turns teletext off.
#[tauri::command]
fn set_teletext(player: tauri::State<Player>, page: i32) -> Res {
    let core = player.core()?;
    core.engine.set_teletext(page);
    core.emit_tracks();
    Ok(())
}

/// Teletext's background.
#[tauri::command]
async fn set_teletext_transparent(player: tauri::State<'_, Player>, transparent: bool) -> Res {
    off_main(player, move |core| {
        core.engine.set_teletext_transparent(transparent);
        core.update_effects(|e| e.teletext_transparent = transparent);
        core.emit_tracks();
    })
    .await
}

#[tauri::command]
fn media_info(player: tauri::State<Player>) -> Result<Option<MediaInfo>, String> {
    Ok(player.core()?.engine.media_info())
}

#[tauri::command]
fn media_stats(player: tauri::State<Player>) -> Result<Option<Stats>, String> {
    Ok(player.core()?.engine.stats())
}

#[tauri::command]
async fn take_snapshot(app: AppHandle, player: tauri::State<'_, Player>) -> Res {
    // VLC's default folder: the Desktop on macOS, Pictures on Windows.
    let default = if cfg!(target_os = "macos") { app.path().desktop_dir() } else { app.path().picture_dir() };
    let dir = player.core()?.settings().snapshot_dir(default.ok()).ok_or("no snapshot folder")?;
    // The engine waits for the video to hand over a frame.
    off_main(player, move |core| {
        // One at a time, so each finds only the file it made.
        static TAKING: Mutex<()> = Mutex::new(());
        let _taking = TAKING.lock().unwrap_or_else(|e| e.into_inner());
        let format = core.settings().snapshot_format.clone();
        let before = snapshot_pngs(&dir);
        core.user_snapshot.store(true, Ordering::Release);
        core.engine.snapshot(&dir, 0)?;
        // libVLC's snapshot call always writes PNG, whatever the format setting.
        // Its name comes from the prefix setting, cleaned of characters a file
        // name can't have, so the new file is found by what wasn't there before.
        if format != "png" {
            for png in snapshot_pngs(&dir).difference(&before) {
                convert_snapshot(png, &format)?;
            }
        }
        Ok(())
    })
    .await?
}

/// The PNG files in `dir`.
fn snapshot_pngs(dir: &Path) -> HashSet<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return HashSet::new() };
    entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"))).collect()
}

/// Re-encodes the PNG snapshot at `png` as `format` ("jpg" or "tiff") beside
/// it, under the same name when that's free, and removes the PNG.
fn convert_snapshot(png: &Path, format: &str) -> Res {
    let ext = if format == "tiff" { "tiff" } else { "jpg" };
    let image = image::open(png).map_err(|e| format!("could not read the snapshot: {e}"))?.to_rgb8();
    let free = |p: &Path| !p.exists() && !p.with_extension("png").exists();
    let mut out = png.with_extension(ext);
    if out.exists() {
        // A name from an earlier run: sequential names take the next free
        // number, others a suffix.
        let stem = png.file_stem().and_then(|s| s.to_str()).unwrap_or("snapshot");
        let digits = stem.len() - stem.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        let (base, first) = match stem.split_at(stem.len() - digits) {
            (base, num) if digits > 0 => (base.to_string(), num.parse::<u64>().unwrap_or(0) + 1),
            _ => (format!("{stem}-"), 1),
        };
        out = (first..)
            .map(|n| png.with_file_name(format!("{base}{n:0digits$}.{ext}")))
            .find(|p| free(p))
            .ok_or("no free snapshot name")?;
    }
    let bytes = if ext == "tiff" {
        tiff_rgb(image.width(), image.height(), image.as_raw())
    } else {
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 92)
            .encode_image(&image)
            .map_err(|e| format!("could not save the snapshot: {e}"))?;
        bytes
    };
    std::fs::write(&out, bytes).map_err(|e| format!("could not save the snapshot: {e}"))?;
    let _ = std::fs::remove_file(png);
    Ok(())
}

/// An uncompressed baseline TIFF of 8-bit RGB pixels.
fn tiff_rgb(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    const ENTRIES: u16 = 10;
    let ifd = 8u32;
    let bits = ifd + 2 + ENTRIES as u32 * 12 + 4;
    let pixels = bits + 6;
    let mut out = Vec::with_capacity(pixels as usize + rgb.len());
    out.extend_from_slice(b"II*\0");
    out.extend_from_slice(&ifd.to_le_bytes());
    out.extend_from_slice(&ENTRIES.to_le_bytes());
    let mut entry = |tag: u16, kind: u16, count: u32, value: u32| {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    };
    const SHORT: u16 = 3;
    const LONG: u16 = 4;
    entry(256, LONG, 1, width); // ImageWidth
    entry(257, LONG, 1, height); // ImageLength
    entry(258, SHORT, 3, bits); // BitsPerSample, at `bits`
    entry(259, SHORT, 1, 1); // Compression: none
    entry(262, SHORT, 1, 2); // Photometric: RGB
    entry(273, LONG, 1, pixels); // StripOffsets
    entry(277, SHORT, 1, 3); // SamplesPerPixel
    entry(278, LONG, 1, height); // RowsPerStrip
    entry(279, LONG, 1, rgb.len() as u32); // StripByteCounts
    entry(284, SHORT, 1, 1); // PlanarConfiguration: chunky
    out.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..3 {
        out.extend_from_slice(&8u16.to_le_bytes());
    }
    out.extend_from_slice(rgb);
    out
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
            core.quality().close("stopped");
            core.engine.stop();
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
    core.engine.set_title(index);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_chapter(player: tauri::State<Player>, index: i32) -> Res {
    let core = player.core()?;
    if let Some(chapter) = core.engine.chapters().get(index.max(0) as usize) {
        core.seeking(chapter.start, "chapter");
    }
    core.engine.set_chapter(index);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn navigate(player: tauri::State<Player>, action: Navigate) -> Res {
    player.core()?.engine.navigate(action);
    Ok(())
}

/// Bookmarks the current position.
#[tauri::command]
fn bookmark_add(player: tauri::State<Player>, name: Option<String>) -> Res {
    let core = player.core()?;
    let uri = core.current_uri().ok_or("Nothing is playing")?;
    let time = core.engine.time();
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

/// Whether libVLC's subtitle autodetection loads `subtitle` with `video` by
/// itself, by its rules for files in the video's folder: `fuzzy` 1 takes any
/// subtitle file, 2 one whose name contains the video's, 3 one that starts
/// with it, 4 only the same name. Names are compared ignoring case and
/// punctuation, without extensions.
fn vlc_autodetects(video: &Path, subtitle: &Path, fuzzy: i64) -> bool {
    // libVLC 3's SLAVE_SPU_EXTENSIONS: the files it looks for (not .mks).
    const AUTODETECTED: &[&str] = &[
        "aqt", "ass", "cdg", "dks", "idx", "jss", "mpl2", "mpsub", "pjs", "psb", "rt", "sami", "sbv", "scc", "smi", "srt",
        "ssa", "stl", "sub", "tt", "ttml", "usf", "vtt", "webvtt",
    ];
    let ext = subtitle.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if fuzzy <= 0 || video.parent() != subtitle.parent() || !AUTODETECTED.contains(&ext.as_str()) {
        return false;
    }
    let name = |p: &Path| {
        let stem = p.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        stem.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
    };
    let (video, subtitle) = (name(video), name(subtitle));
    if video.is_empty() {
        return false;
    }
    let level = if subtitle == video {
        4
    } else if subtitle.starts_with(&video) {
        3
    } else if subtitle.contains(&video) {
        2
    } else {
        1
    };
    level >= fuzzy
}

/// Paths and URLs as queue items: folders give their media, playlist files
/// their entries. Subtitle files are left out.
fn media_items(uris: Vec<String>) -> Result<Vec<Item>, String> {
    let mut items = Vec::new();
    for uri in uris.into_iter().map(playlist_file::native_path) {
        let path = Path::new(&uri);
        if uri.contains("://") {
            items.push(Item::new(uri));
        } else if path.is_dir() {
            items.extend(library::expand(vec![uri]).into_iter().map(Item::new));
        } else if playlist_file::is_playlist(path) {
            items.extend(playlist_file::read(path)?);
        } else if !library::is_subtitle(path) {
            items.push(Item::new(uri));
        }
    }
    Ok(items)
}

#[tauri::command]
async fn playlists_list(player: tauri::State<'_, Player>) -> Result<Vec<Saved>, String> {
    off_main(player, |core| core.playlists().lists.clone()).await
}

/// Makes a playlist, empty or holding what's in the queue.
#[tauri::command]
async fn playlists_create(player: tauri::State<'_, Player>, name: String, from_queue: bool) -> Result<Vec<Saved>, String> {
    off_main(player, move |core| {
        // Cameras and screens aren't something to come back to.
        let items = if from_queue { core.playlist().items.iter().filter(|i| !is_capture(&i.uri)).cloned().collect() } else { Vec::new() };
        let mut lists = core.playlists();
        lists.create(&name, items)?;
        Ok(lists.lists.clone())
    })
    .await?
}

/// Makes a playlist from an M3U, PLS or XSPF file, named after the file.
#[tauri::command]
async fn playlists_import(player: tauri::State<'_, Player>, path: String) -> Result<Vec<Saved>, String> {
    off_main(player, move |core| {
        let path = Path::new(&path);
        let items = playlist_file::read(path)?;
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut lists = core.playlists();
        lists.create_numbered(&name, items)?;
        Ok(lists.lists.clone())
    })
    .await?
}

#[tauri::command]
async fn playlists_rename(player: tauri::State<'_, Player>, id: String, name: String) -> Result<Vec<Saved>, String> {
    off_main(player, move |core| {
        let mut lists = core.playlists();
        lists.rename(&id, &name)?;
        Ok(lists.lists.clone())
    })
    .await?
}

#[tauri::command]
async fn playlists_delete(player: tauri::State<'_, Player>, id: String) -> Result<Vec<Saved>, String> {
    off_main(player, move |core| {
        let mut lists = core.playlists();
        lists.delete(&id)?;
        Ok(lists.lists.clone())
    })
    .await?
}

/// Adds files, folders or URLs to a saved playlist.
#[tauri::command]
async fn playlists_add(player: tauri::State<'_, Player>, id: String, uris: Vec<String>) -> Result<Vec<Saved>, String> {
    off_main(player, move |core| {
        let items = media_items(uris)?;
        let mut lists = core.playlists();
        lists.add(&id, items)?;
        Ok(lists.lists.clone())
    })
    .await?
}

#[tauri::command]
async fn playlists_remove_item(player: tauri::State<'_, Player>, id: String, index: usize, uri: String) -> Result<Vec<Saved>, String> {
    off_main(player, move |core| {
        let mut lists = core.playlists();
        lists.remove_item(&id, index, &uri)?;
        Ok(lists.lists.clone())
    })
    .await?
}

/// Plays a saved playlist from its item `index`. It takes the queue's place.
#[tauri::command]
async fn playlists_play(player: tauri::State<'_, Player>, id: String, index: usize) -> Res {
    off_main(player, move |core| {
        let items = core.playlists().items(&id)?;
        if items.is_empty() {
            return Err("That playlist is empty".to_string());
        }
        let index = index.min(items.len() - 1);
        let _transition = core.transition();
        core.stop_locked();
        let first = {
            let mut queue = core.playlist();
            queue.clear();
            queue.add(items)
        };
        core.emit_playlist();
        core.play_index_locked(first + index)
    })
    .await?
}

/// Where to find media on the network.
#[derive(Clone, Serialize)]
struct Places {
    /// Network volumes the system has mounted.
    mounted: Vec<Place>,
    /// Servers the user saved.
    servers: Vec<Server>,
    /// Servers found on the local network so far.
    found: Vec<Found>,
}

/// Lists network places, and starts looking for servers on the local
/// network if that isn't running. The UI asks again every few seconds
/// while it shows them.
#[tauri::command]
async fn network_places(player: tauri::State<'_, Player>) -> Result<Places, String> {
    off_main(player, |core| {
        core.engine.discovery_start();
        Places { mounted: network::mounted(), servers: core.servers().list.clone(), found: core.engine.discovered() }
    })
    .await
}

/// Stops looking for servers: the UI has left the network view.
#[tauri::command]
async fn network_leave(player: tauri::State<'_, Player>) -> Res {
    off_main(player, |core| core.engine.discovery_stop()).await
}

/// Lists a folder on a mounted volume or a server.
#[tauri::command]
async fn network_browse(player: tauri::State<'_, Player>, location: String, user: Option<String>, password: Option<String>) -> Result<Listing, String> {
    off_main(player, move |core| network::browse(&*core.engine, &location, user.as_deref().unwrap_or_default(), password.as_deref().unwrap_or_default())).await?
}

#[tauri::command]
async fn network_save(player: tauri::State<'_, Player>, url: String, name: String, user: Option<String>) -> Result<Vec<Server>, String> {
    off_main(player, move |core| {
        let mut servers = core.servers();
        servers.save(&url, &name, user)?;
        Ok(servers.list.clone())
    })
    .await?
}

#[tauri::command]
async fn network_forget(player: tauri::State<'_, Player>, url: String) -> Result<Vec<Server>, String> {
    off_main(player, move |core| {
        let mut servers = core.servers();
        servers.remove(&url)?;
        Ok(servers.list.clone())
    })
    .await?
}

/// Stations from the radio directory: by name, by genre, or the most played.
#[tauri::command]
async fn radio_stations(query: String, tag: String) -> Result<Vec<Station>, String> {
    radio::stations(&query, &tag).await
}

#[tauri::command]
async fn radio_favorites(player: tauri::State<'_, Player>) -> Result<Vec<Station>, String> {
    off_main(player, |core| core.radio().list.clone()).await
}

#[tauri::command]
async fn radio_favorite(player: tauri::State<'_, Player>, station: Station, on: bool) -> Result<Vec<Station>, String> {
    off_main(player, move |core| {
        let mut favorites = core.radio();
        favorites.set(station, on)?;
        Ok(favorites.list.clone())
    })
    .await?
}

/// Plays a station, under its name rather than its address.
#[tauri::command]
async fn radio_play(player: tauri::State<'_, Player>, station: Station) -> Res {
    if !radio::playable(&station.url) {
        return Err("That station has no address to play".into());
    }
    let item = Item { uri: station.url, title: station.name, options: Vec::new(), trusted: false };
    off_main(player, move |core| core.add_items(vec![item], true)).await?
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
    #[cfg(target_os = "linux")]
    {
        let mut langs = Vec::new();
        for lang in gtk::glib::language_names() {
            let code = lang.split(['-', '_', '.', '@']).next().unwrap_or_default().to_lowercase();
            if !code.is_empty() && code != "c" && !langs.contains(&code) {
                langs.push(code);
            }
        }
        langs
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    Vec::new()
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    main_window(app)?;
    updater::start(app.handle());
    let resource_dir = app.path().resource_dir().ok();
    let data_dir = app.path().app_data_dir()?;
    let languages = preferred_languages();
    let (settings, settings_damaged) = Settings::load(&data_dir);
    // Extra libVLC options for debugging, e.g. REVLC_VLC_ARGS="--verbose=2".
    let args: Vec<String> = std::env::var("REVLC_VLC_ARGS").unwrap_or_default().split_whitespace().map(String::from).collect();
    let mut cli = cli::parse(std::env::args().skip(1));
    if let Ok(cwd) = std::env::current_dir() {
        cli.resolve_paths(&cwd);
    }
    // Before the engine: the menu bar (and its Settings…) is there even if
    // the engine can't start. The UI sends the real bindings once it loads;
    // start with the defaults.
    let keys: HashMap<String, String> = [("open", "Cmd+O"), ("openFolder", "Shift+Cmd+O"), ("openUrl", "Cmd+N"), ("queue", "Cmd+L")]
        .into_iter()
        .map(|(a, k)| (a.to_string(), k.to_string()))
        .collect();
    if cfg!(target_os = "macos") {
        app.set_menu(menu::build(app.handle(), &keys)?)?;
    }
    let engine = VlcEngine::load_with_settings(resource_dir.as_deref(), &args, &cli.vlc_args, &settings, &languages);
    telemetry::start(&data_dir, settings.analytics, engine.as_ref().map_or("unavailable", |v| v.version()));
    let engine: Arc<dyn Engine> = match engine {
        Ok(vlc) => Arc::new(vlc),
        Err(e) => {
            // Nothing can play. The message names paths, so only the stage is sent.
            telemetry::capture("engine_failed", serde_json::json!({ "stage": e.stage }));
            eprintln!("revlc: {}", e.message);
            app.manage(Player(Err(e.message)));
            return Ok(());
        }
    };
    let cache_dir = app.path().app_cache_dir()?;
    let (history_saves, history_rx) = mpsc::channel();
    let app_handle = app.handle().clone();
    let core = Arc::new(Core {
        engine,
        playlist: Mutex::default(),
        ui: Arc::new(app_handle.clone()),
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
        meta_scanner: MetaScanner::default(),
        thumbs_dir: cache_dir.join("thumbs"),
        embedded_art_dir: cache_dir.join("embedded"),
        scrub_dir: cache_dir.join("scrub"),
        scrub_lock: Mutex::default(),
        scrub_cache: Arc::new(preview::cache::ScrubCache::default()),
        scrub_priority: Mutex::new(None),
        scrub_run: AtomicU64::new(0),
        scrub_cancel_token: Mutex::new(keyframes::CancelToken::new()),
        scrub_active: Mutex::new(None),
        scrub_passes_active: Arc::new((Mutex::new(0), Condvar::new())),
        scrub_jobs: Mutex::default(),
        meta: Mutex::new(MetaCache::load(&cache_dir)),
        bookmarks: Mutex::new(Bookmarks::load(&data_dir)),
        playlists: Mutex::new(Playlists::load(&data_dir)),
        servers: Mutex::new(Servers::load(&data_dir)),
        radio: Mutex::new(Favorites::load(&data_dir)),
        ab_loop: Mutex::default(),
        sleep: Mutex::default(),
        transition: Mutex::default(),
        reloads: AtomicU64::new(0),
        subtitles: Mutex::new(Vec::new()),
        early: Mutex::new(Some(Early::default())),
        library_scan: Mutex::default(),
        quality: Mutex::default(),
        history_saves,
        history_write: Mutex::default(),
        settings_change: Mutex::default(),
        effects: Mutex::default(),
        start_fullscreen: cli.fullscreen,
        play_and_exit: cli.play_and_exit,
        data_dir,
    });
    if !cli.ignored.is_empty() {
        core.notice("error", format!("Ignored command-line options: {}", cli.ignored.join(" ")));
    }
    if !core.engine.rejected_args().is_empty() {
        core.notice("error", format!("libVLC didn't accept {}, so ReVLC started without them", core.engine.rejected_args().join(" ")));
    }
    // Files that couldn't be read were moved aside; say so, like the history's.
    let kept = |d: &store::Damaged| d.aside.as_ref().map(|p| format!(" The old file is kept at {}", p.display())).unwrap_or_default();
    if let Some(d) = &settings_damaged {
        core.notice("error", format!("Your settings couldn't be read, so they're back to the defaults, with usage statistics off.{}", kept(d)));
    }
    if let Some(d) = core.bookmarks().damaged.take() {
        core.notice("error", format!("Your bookmarks couldn't be read, so they start empty.{}", kept(&d)));
    }
    if let Some(d) = core.library().damaged.take() {
        core.notice("error", format!("Your library folders couldn't be read, so the library starts empty.{}", kept(&d)));
    }

    let scrub_dir = core.scrub_dir.clone();
    thread::spawn(move || scrub::prune(&scrub_dir));
    let thumbs = core.clone();
    thread::Builder::new().name("thumbnails".into()).spawn(move || make_thumbnails(thumbs))?;
    let meta_workers = thread::available_parallelism().map_or(4, |n| n.get().clamp(2, 6));
    for i in 0..meta_workers {
        let scanner = core.clone();
        thread::Builder::new().name(format!("meta-scan-{i}")).spawn(move || scan_metadata_worker(scanner))?;
    }
    let saver = core.clone();
    thread::Builder::new().name("history".into()).spawn(move || {
        while history_rx.recv().is_ok() {
            // One write covers every request that came meanwhile.
            history_rx.try_iter().for_each(drop);
            let _ = saver.write_history(true);
        }
    })?;

    let (tx, rx) = mpsc::channel();
    core.engine.subscribe(tx);
    let pump = core.clone();
    thread::Builder::new().name("vlc-events".into()).spawn(move || pump_events(pump, rx))?;

    // Files passed on the command line, then any Finder opened before now.
    if let Err(e) = core.add_with_options(cli.files, true, &cli.media_options) {
        core.notice("error", e);
    }
    let early = std::mem::take(&mut *EARLY_OPENS.lock().unwrap_or_else(|e| e.into_inner()));
    if let Err(e) = core.add(early, true) {
        core.notice("error", e);
    }

    let window = app.get_webview_window("main").ok_or("main window missing")?;
    #[cfg(windows)]
    quiet_webview(&window);
    #[cfg(windows)]
    let hwnd = window.hwnd()?.0 as isize;
    let attach = core.clone();
    let pointer_handle = app_handle;
    window.with_webview(move |webview| {
        #[cfg(target_os = "macos")]
        {
            let app = pointer_handle.clone();
            let last = std::cell::Cell::new(Instant::now() - Duration::from_secs(1));
            let on_pointer = move || {
                // Throttled: the UI only needs to know the pointer is active.
                if last.get().elapsed() >= Duration::from_millis(150) {
                    last.set(Instant::now());
                    let _ = app.emit("native://pointer", ());
                }
            };
            match surface_macos::attach(webview.inner(), webview.ns_window(), on_pointer) {
                Ok(view) => attach.engine.set_nsview(view),
                Err(e) => eprintln!("revlc: could not create the video view: {e}"),
            }
        }
        // The web view covers the whole window and gets every mouse event,
        // so Windows needs no pointer monitor.
        #[cfg(windows)]
        {
            let _ = (webview, &pointer_handle);
            match surface_windows::attach(hwnd as *mut std::ffi::c_void) {
                Ok(video) => attach.engine.set_hwnd(video),
                Err(e) => eprintln!("revlc: could not create the video window: {e}"),
            }
        }
        #[cfg(target_os = "linux")]
        {
            let _ = &pointer_handle;
            let frames = Arc::new(Mutex::new(None));
            match surface_linux::attach(&webview.inner(), frames.clone()) {
                Ok(()) => attach.engine.set_gtk_surface(frames),
                Err(e) => {
                    eprintln!("revlc: could not create the video surface: {e}");
                    return;
                }
            }
        }
        #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
        let _ = (webview, &pointer_handle);
        // Off the main thread, like the commands that open media.
        thread::spawn(move || attach.mark_ready());
    })?;

    app.manage(Player(Ok(core)));
    Ok(())
}

/// Builds the player window from its tauri.conf entry (marked not to be made
/// there, in each platform's file too: those replace the list). Started again right after a copy was closed or killed, Windows can
/// still have that copy's WebView2 helpers holding the profile folder for a
/// few seconds, and making the web view fails with "the requested resource is
/// in use"; that failure is waited out.
fn main_window(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let config = app.config().app.windows.iter().find(|w| w.label == "main").ok_or("main window missing from tauri.conf")?.clone();
    retry_while_busy(Duration::from_secs(10), Duration::from_millis(250), || WebviewWindowBuilder::from_config(app.handle(), &config)?.build())?;
    Ok(())
}

/// Runs `attempt` until it succeeds, fails some other way, or `limit` passes
/// with it still failing because the web view's profile is busy.
fn retry_while_busy<T, E: std::fmt::Display>(limit: Duration, pause: Duration, mut attempt: impl FnMut() -> Result<T, E>) -> Result<T, E> {
    let start = Instant::now();
    loop {
        match attempt() {
            Err(e) if webview_busy(&e.to_string()) && start.elapsed() < limit => {
                eprintln!("revlc: web view busy, trying again: {e}");
                thread::sleep(pause);
            }
            done => return done,
        }
    }
}

/// Whether a web view error is WebView2's ERROR_BUSY (0x800700AA): another
/// process is still using the profile folder.
fn webview_busy(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("0x800700aa") || error.contains("resource is in use")
}

/// Tells the user ReVLC can't start, in a dialog on Windows (where there's no
/// terminal to read), and exits.
fn fail_to_start(error: &dyn std::fmt::Display) -> ! {
    let error = error.to_string();
    let mut message = format!("ReVLC couldn't start.\n\n{error}");
    if webview_busy(&error) {
        message.push_str("\n\nA copy of ReVLC that was just closed may still be shutting down. Wait a moment, then open ReVLC again.");
    }
    eprintln!("revlc: {message}");
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (text, caption) = (wide(&message), wide("ReVLC"));
        // SAFETY: both strings are NUL-terminated and outlive the call.
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_OK | MB_ICONERROR) };
    }
    std::process::exit(1)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    surface_linux::init();
    let builder = tauri::Builder::default();
    // Windows starts a new process for each file opened from Explorer; it
    // hands its files to the running copy and exits.
    #[cfg(any(windows, target_os = "linux"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        // libVLC has started, so only the files and their start times count.
        let mut cli = cli::parse(args.into_iter().skip(1));
        cli.resolve_paths(Path::new(&cwd));
        if cli.files.is_empty() {
            return;
        }
        if let Some(Ok(core)) = app.try_state::<Player>().as_deref().map(Player::core) {
            core.add_from_main(cli.files, cli.media_options);
        }
    }));
    let app = match builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Serves seek-bar preview JPEGs directly from the bounded in-memory LRU
        // cache without touching disk (`scrub://localhost/{nonce}-{run}/{k}.jpg`).
        .register_uri_scheme_protocol("scrub", |ctx, req| {
            let cache = ctx
                .app_handle()
                .try_state::<Player>()
                .and_then(|p| p.core().ok().map(|c| Arc::clone(&c.scrub_cache)));
            scrub_protocol_response(cache.as_deref(), req.uri().path())
        })
        // Tauri panics on a setup error, which with panic = "abort" closes
        // the app without a word.
        .setup(|app| {
            if let Err(e) = setup(app) {
                fail_to_start(&e);
            }
            Ok(())
        })
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
            scrub_prepare,
            scrub_focus,
            scrub_cancel,
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
            set_video_track,
            set_stereo_mode,
            equalizer_info,
            set_equalizer_preset,
            set_equalizer,
            set_adjust,
            filter_specs,
            set_filter,
            set_transform,
            set_logo,
            set_marquee,
            set_zoom,
            set_teletext,
            set_teletext_transparent,
            media_info,
            media_stats,
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
            playlists_list,
            playlists_create,
            playlists_import,
            playlists_rename,
            playlists_delete,
            playlists_add,
            playlists_remove_item,
            playlists_play,
            network_places,
            network_leave,
            network_browse,
            network_save,
            network_forget,
            radio_stations,
            radio_favorites,
            radio_favorite,
            radio_play,
        ])
        .build(tauri::generate_context!())
    {
        Ok(app) => app,
        Err(e) => fail_to_start(&e),
    };

    app.run(|handle, event| {
        // The Quit menu item ends the app without asking: only Exit comes.
        if matches!(event, RunEvent::ExitRequested { .. } | RunEvent::Exit) {
            if let Some(Ok(core)) = handle.try_state::<Player>().as_deref().map(Player::core) {
                core.save_progress(false);
                // Now, rather than on the history thread, which quitting doesn't wait for.
                let _ = core.write_history(true);
                core.quality().close("quit");
            }
            telemetry::flush_on_exit();
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
                        core.add_from_main(uris, Vec::new());
                    }
                }
                None => EARLY_OPENS.lock().unwrap_or_else(|e| e.into_inner()).extend(uris),
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (handle, event);
    });
}

/// Builds the HTTP response for `scrub://` custom-protocol requests directly
/// from the bounded in-memory [`preview::cache::ScrubCache`].
fn scrub_protocol_response(cache: Option<&preview::cache::ScrubCache>, path: &str) -> tauri::http::Response<Vec<u8>> {
    let body = cache.and_then(|c| {
        let (run, k) = preview::parse_scrub_uri_path(path)?;
        c.get(run, k)
    });
    match body {
        Some(bytes) => tauri::http::Response::builder()
            .status(200)
            .header("Content-Type", "image/jpeg")
            .header("Cache-Control", "no-store")
            .body(bytes.to_vec())
            .unwrap_or_else(|_| tauri::http::Response::new(Vec::new())),
        None => tauri::http::Response::builder()
            .status(404)
            .header("Content-Type", "text/plain")
            .body(b"not found".to_vec())
            .unwrap_or_else(|_| tauri::http::Response::new(Vec::new())),
    }
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

    // As WebView2 reports a profile folder another process still holds.
    const BUSY: &str = "failed to create webview: WebView2 error: WindowsError(Error { code: HRESULT(0x800700AA), message: \"The requested resource is in use.\" })";

    #[test]
    fn busy_web_view_is_waited_out() {
        let mut tries = 0;
        let got = retry_while_busy(Duration::from_secs(5), Duration::ZERO, || {
            tries += 1;
            if tries < 3 { Err(BUSY) } else { Ok("window") }
        });
        assert_eq!((got, tries), (Ok("window"), 3));
    }

    #[test]
    fn other_failures_and_lasting_busy_give_up() {
        let mut tries = 0;
        let got: Result<(), _> = retry_while_busy(Duration::from_secs(5), Duration::ZERO, || {
            tries += 1;
            Err("failed to create webview: WebView2 runtime not found")
        });
        assert!(got.is_err());
        assert_eq!(tries, 1);
        let got: Result<(), _> = retry_while_busy(Duration::from_millis(30), Duration::from_millis(5), || Err(BUSY));
        assert_eq!(got, Err(BUSY));
    }

    #[test]
    fn every_config_leaves_the_main_window_to_setup() {
        // A platform file's window list replaces the main one, so each needs
        // "create": false, or Tauri makes the window and setup's fails.
        for (file, json) in [
            ("tauri.conf.json", include_str!("../tauri.conf.json")),
            ("tauri.linux.conf.json", include_str!("../tauri.linux.conf.json")),
            ("tauri.windows.conf.json", include_str!("../tauri.windows.conf.json")),
            ("tauri.macos.conf.json", include_str!("../tauri.macos.conf.json")),
        ] {
            let config: serde_json::Value = serde_json::from_str(json).unwrap();
            for window in config["app"]["windows"].as_array().into_iter().flatten() {
                assert_eq!(window["create"], false, "{file}: window {}", window["label"]);
            }
        }
    }

    #[test]
    fn one_unreadable_playlist_leaves_the_rest_to_open() {
        let dir = std::env::temp_dir().join(format!("revlc-opened-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
        std::fs::write(dir.join("good.m3u"), "one.mkv\ntwo.mkv\n").unwrap();
        // gone.m3u isn't there, so it can't be read.
        let found = opened(vec![path("good.m3u"), path("gone.m3u"), path("film.mkv"), path("film.srt"), "https://example.com/live.m3u8".into()]);
        let uris: Vec<&str> = found.items.iter().map(|i| i.uri.as_str()).collect();
        assert_eq!(uris, [path("one.mkv"), path("two.mkv"), path("film.mkv"), "https://example.com/live.m3u8".to_string()]);
        assert_eq!(found.subtitles, [path("film.srt")]);
        assert!(found.folders.is_empty());
        assert_eq!(found.failed.len(), 1);
        assert!(found.failed[0].contains("gone.m3u"), "{}", found.failed[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn busy_is_told_apart() {
        assert!(webview_busy(BUSY));
        assert!(webview_busy("error encountered during setup hook: HRESULT(0x800700aa)"));
        assert!(!webview_busy("failed to create webview: HRESULT(0x80070002)"));
    }

    struct TestUi {
        events: Mutex<Vec<(String, serde_json::Value)>>,
        exited: AtomicBool,
    }

    impl TestUi {
        fn new() -> Arc<Self> {
            Arc::new(Self { events: Mutex::default(), exited: AtomicBool::new(false) })
        }

        fn events(&self) -> Vec<(String, serde_json::Value)> {
            self.events.lock().unwrap().clone()
        }
    }

    impl UiEvents for TestUi {
        fn emit_json(&self, event: &str, payload: serde_json::Value) {
            self.events.lock().unwrap().push((event.to_string(), payload));
        }

        fn exit(&self, _code: i32) {
            self.exited.store(true, Ordering::Release);
        }
    }

    fn test_core(name: &str) -> (Arc<Core>, Arc<engine::mock::MockEngine>, Arc<TestUi>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("revlc-core-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mock = Arc::new(engine::mock::MockEngine::default());
        let ui = TestUi::new();
        let (history_saves, _rx) = mpsc::channel();
        let core = Arc::new(Core {
            engine: mock.clone(),
            playlist: Mutex::default(),
            ui: ui.clone(),
            ready: AtomicBool::new(true),
            pending: Mutex::default(),
            history: Mutex::new(History::load(dir.clone(), dir.clone())),
            resume: Mutex::default(),
            settings: Mutex::new(Settings { analytics: false, ..Settings::default() }),
            data_dir: dir.clone(),
            languages: vec!["en".into()],
            fonts: Vec::new(),
            user_snapshot: AtomicBool::new(false),
            library: Mutex::new(Library::load(&dir)),
            thumbs: Thumbnails::default(),
            meta_scanner: MetaScanner::default(),
            thumbs_dir: dir.join("thumbs"),
            embedded_art_dir: dir.join("embedded"),
            scrub_dir: dir.join("scrub"),
            scrub_lock: Mutex::default(),
            scrub_cache: Arc::new(preview::cache::ScrubCache::default()),
            scrub_priority: Mutex::new(None),
            scrub_run: AtomicU64::new(0),
            scrub_cancel_token: Mutex::new(keyframes::CancelToken::new()),
            scrub_active: Mutex::new(None),
            scrub_passes_active: Arc::new((Mutex::new(0), Condvar::new())),
            scrub_jobs: Mutex::default(),
            meta: Mutex::new(MetaCache::load(&dir)),
            bookmarks: Mutex::new(Bookmarks::load(&dir)),
            playlists: Mutex::new(Playlists::load(&dir)),
            servers: Mutex::new(Servers::load(&dir)),
            radio: Mutex::new(Favorites::load(&dir)),
            ab_loop: Mutex::default(),
            sleep: Mutex::default(),
            transition: Mutex::default(),
            reloads: AtomicU64::new(0),
            subtitles: Mutex::new(Vec::new()),
            early: Mutex::new(None),
            library_scan: Mutex::default(),
            quality: Mutex::default(),
            history_saves,
            history_write: Mutex::default(),
            settings_change: Mutex::default(),
            effects: Mutex::default(),
            start_fullscreen: false,
            play_and_exit: false,
        });
        (core, mock, ui, dir)
    }

    fn wait_until(timeout: Duration, mut pred: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if pred() {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        pred()
    }

    #[test]
    fn core_state_machine_opens_resumes_loops_and_advances_queue() {
        let (core, mock, ui, dir) = test_core("sm");
        let (tx, rx) = mpsc::channel();
        mock.subscribe(tx);
        let pump_core = core.clone();
        let pump = thread::spawn(move || pump_events(pump_core, rx));

        // Seed resume position for track 1 and set Resume::Always. Use platform-native
        // paths under `dir` so Windows path normalization (`\` vs `/`) matches identically.
        let path_a = dir.join("a.mp4").to_string_lossy().into_owned();
        let path_b = dir.join("b.mp4").to_string_lossy().into_owned();
        core.settings().resume = Resume::Always;
        core.history().touch(&path_a, "a.mp4");
        core.history().update(&path_a, 42_000, 120_000);

        core.add(vec![path_a.clone(), path_b.clone()], true).unwrap();
        assert_eq!(mock.opened_uris.lock().unwrap().as_slice(), [path_a.as_str()]);

        // Playing event triggers pending resume seek.
        mock.send_event(EventKind::Playing, 100.0, -1);
        assert!(wait_until(Duration::from_secs(2), || mock.seeks.lock().unwrap().as_slice() == [42_000]));

        // A-B loop jumps back to A when playback passes B.
        *core.ab_loop() = (Some(10_000), Some(20_000));
        mock.time.store(21_000, Ordering::Release);
        mock.send_event(EventKind::TimeChanged, 100.0, 21_000);
        assert!(wait_until(Duration::from_secs(2), || mock.seeks.lock().unwrap().last().copied() == Some(10_000)));
        *core.ab_loop() = (None, None);

        // EndReached after playback advances to track 2.
        mock.send_event(EventKind::EndReached, 100.0, -1);
        assert!(wait_until(Duration::from_secs(2), || {
            mock.opened_uris.lock().unwrap().as_slice() == [path_a.as_str(), path_b.as_str()]
        }));
        assert_eq!(core.playlist().current, Some(1));

        // Subitem Replace expands track 2 into two new entries and plays the first.
        *mock.subitems.lock().unwrap() = SubitemExpansion::Replace(vec![
            ("https://example.com/1.m3u8".into(), Some("Stream 1".into())),
            ("https://example.com/2.m3u8".into(), Some("Stream 2".into())),
        ]);
        mock.send_event(EventKind::EndReached, 100.0, -1);
        assert!(wait_until(Duration::from_secs(2), || {
            core.current_uri().as_deref() == Some("https://example.com/1.m3u8")
        }));
        assert_eq!(core.playlist().items.len(), 3);

        // Stop & close channel cleanly.
        core.stop();
        assert!(mock.stops.load(Ordering::Acquire) >= 1);
        assert!(ui.events().iter().any(|(e, _)| e == "player://playlist"));
        *mock.tx.lock().unwrap() = None;
        let _ = pump.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn core_skips_unplayable_items_without_looping_forever() {
        let (core, mock, ui, dir) = test_core("unplayable");
        let (tx, rx) = mpsc::channel();
        mock.subscribe(tx);
        let pump_core = core.clone();
        let pump = thread::spawn(move || pump_events(pump_core, rx));

        let bad_1 = dir.join("1.mp4").to_string_lossy().into_owned();
        let bad_2 = dir.join("2.mp4").to_string_lossy().into_owned();
        core.playlist().repeat = Repeat::All;
        core.add(vec![bad_1.clone(), bad_2.clone()], true).unwrap();
        // Item 0 errors without ever advancing time > 0 -> advances to item 1.
        mock.send_event(EventKind::Error, 0.0, -1);
        assert!(wait_until(Duration::from_secs(2), || {
            mock.opened_uris.lock().unwrap().as_slice() == [bad_1.as_str(), bad_2.as_str()]
        }));

        // Item 1 also errors -> wait until its error notice is emitted, then verify
        // every item in the Repeat::All queue failed and it did NOT loop back to item 0.
        mock.send_event(EventKind::Error, 0.0, -1);
        assert!(wait_until(Duration::from_secs(2), || {
            ui.events().iter().any(|(e, p)| e == "player://notice" && p["text"].as_str().is_some_and(|t| t.contains("2.mp4")))
        }));
        assert_eq!(mock.opened_uris.lock().unwrap().len(), 2);

        *mock.tx.lock().unwrap() = None;
        let _ = pump.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scrub_focus_prioritizes_hovered_slots_middle_first() {
        let (core, _mock, _ui, dir) = test_core("scrub-focus");
        let queue = Arc::new(Mutex::new(
            (0..20)
                .map(|k| ScrubJob { at: k * 2000, k, exact: false })
                .collect::<VecDeque<_>>(),
        ));
        *core.scrub_jobs.lock().unwrap() = Arc::clone(&queue);
        let priority = preview::PriorityQueue::new(20);
        *core.scrub_priority.lock().unwrap() = Some(Arc::clone(&priority));
        core.scrub_focus(10, 14);
        let order: Vec<i64> = queue.lock().unwrap().iter().map(|j| j.k).collect();
        // Middle of 10..=14 is 12, followed by 11/13, then 10/14, then the rest.
        assert_eq!(order[0], 12);
        assert!(order[..5].iter().all(|k| (10..=14).contains(k)));
        // Warm-session PriorityQueue is also reordered middle-first.
        assert_eq!(priority.pop_next(), Some(12));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scrub_protocol_and_cancel_isolate_runs_without_spawning_children() {
        let (core, _mock, ui, dir) = test_core("scrub-proto-iso");
        let (run1, _tok1) = {
            let _guard = core.scrub_lock.lock().unwrap();
            core.scrub_cancel_locked()
        };
        let (run2, _tok2) = {
            let _guard = core.scrub_lock.lock().unwrap();
            core.scrub_cancel_locked()
        };
        assert!(run2 > run1);

        let grid1 = ScrubGrid {
            run: run1,
            uri: Arc::new("/media/a.mp4".into()),
            length: 10_000,
            track: 0,
            libvlc: false,
            step: 2_000,
            count: 5,
            queue: Arc::default(),
        };
        let grid2 = ScrubGrid { run: run2, ..grid1.clone() };
        let made = Mutex::new(HashSet::new());
        // Stale run 1 store must be rejected and emit zero UI events.
        assert!(!core.scrub_store_frame(&grid1, &made, 0, vec![0xFF, 0xD8, 0xFF, 0xD9]));
        assert!(ui.events().is_empty());

        // Active run 2 store populates cache, emits player://scrub, and serves 200 image/jpeg no-store.
        let payload = vec![0xFF, 0xD8, 0xFF, 0xE0, 0xAA, 0xBB];
        assert!(core.scrub_store_frame(&grid2, &made, 3, payload.clone()));
        let nonce = preview::session_nonce();
        let ok_resp = scrub_protocol_response(Some(&core.scrub_cache), &format!("/{nonce}-{run2}/3.jpg?v=1"));
        assert_eq!(ok_resp.status(), 200);
        assert_eq!(ok_resp.headers().get("Content-Type").and_then(|v| v.to_str().ok()), Some("image/jpeg"));
        assert_eq!(ok_resp.headers().get("Cache-Control").and_then(|v| v.to_str().ok()), Some("no-store"));
        assert_eq!(ok_resp.body(), &payload);

        // Wrong session nonce and unknown slot return 404.
        let bad_nonce = scrub_protocol_response(Some(&core.scrub_cache), &format!("/{}-{run2}/3.jpg", nonce.wrapping_add(1)));
        assert_eq!(bad_nonce.status(), 404);
        assert_eq!(scrub_protocol_response(Some(&core.scrub_cache), &format!("/{nonce}-{run2}/99.jpg")).status(), 404);

        // After cancel, previously cached frame immediately returns 404.
        core.scrub_cancel();
        assert_eq!(scrub_protocol_response(Some(&core.scrub_cache), &format!("/{nonce}-{run2}/3.jpg")).status(), 404);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_library_cache_keeps_shared_embedded_art_and_drops_unreferenced() {
        let (core, _mock, _ui, dir) = test_core("prune-emb");
        std::fs::create_dir_all(&core.embedded_art_dir).unwrap();
        let shared_art = core.embedded_art_dir.join("emb-1111.jpg");
        let drop_art = core.embedded_art_dir.join("emb-2222.jpg");
        std::fs::write(&shared_art, b"shared").unwrap();
        std::fs::write(&drop_art, b"drop").unwrap();
        let song1_path = dir.join("song1.mp3");
        let song2_path = dir.join("song2.mp3");
        std::fs::write(&song1_path, b"mp3-1").unwrap();
        std::fs::write(&song2_path, b"mp3-2").unwrap();
        let song1_uri = song1_path.to_string_lossy().into_owned();
        let song2_uri = song2_path.to_string_lossy().into_owned();
        // Both songs reference the same embedded cover; song2 is removed from the library,
        // while song1 remains -> shared cover MUST stay, unrelated drop_art MUST be removed.
        for uri in [&song1_uri, &song2_uri] {
            core.meta.lock().unwrap().insert(
                uri,
                engine::MediaMeta { artwork: Some(shared_art.to_string_lossy().into_owned()), ..Default::default() },
            );
        }
        let make_song = |uri: String| Song {
            uri,
            name: "song".into(),
            root: dir.to_string_lossy().into_owned(),
            folder: String::new(),
            modified: 0,
            artist: None,
            album: None,
            album_artist: None,
            genre: None,
            track: None,
            disc: None,
            year: None,
            duration: 0,
            art: None,
        };
        core.prune_library_cache(&[], &[make_song(song1_uri)], &[]);
        assert!(shared_art.exists(), "shared embedded art still referenced by song1 must stay");
        assert!(!drop_art.exists(), "unreferenced embedded art must be pruned");
        assert!(core.meta.lock().unwrap().get(&song2_uri).is_none(), "removed song metadata must be pruned");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scrub_fill_stores_frames_in_memory_cache_and_cleans_up_after_errors() {
        let (core, mock, ui, dir) = test_core("scrub-fill-mem");
        let mut jpeg = Vec::new();
        let img = image::RgbImage::from_fn(8, 8, |x, y| image::Rgb([(x * 30) as u8, (y * 30) as u8, 128]));
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80)
            .encode_image(&img)
            .unwrap();
        *mock.thumbnail_bytes.lock().unwrap() = Some(jpeg.clone());
        // Inject an error AFTER partial write on slot 2 (timestamp 4000 ms).
        *mock.thumbnail_fail_at.lock().unwrap() = Some(4_000);

        let (run, cancel) = {
            let _guard = core.scrub_lock.lock().unwrap();
            core.scrub_cancel_locked()
        };
        let uri = dir.join("clip.avi").to_string_lossy().into_owned();
        let grid = ScrubGrid {
            run,
            uri: Arc::new(uri.clone()),
            length: 10_000,
            track: 0,
            libvlc: true,
            step: 2_000,
            count: 5,
            queue: Arc::default(),
        };
        core.scrub_fill(grid, &HashSet::new(), cancel);

        // Slots 0, 1, 3, 4 succeeded into memory cache; slot 2 failed and was NOT cached.
        for k in [0i64, 1, 3, 4] {
            assert_eq!(core.scrub_cache.get(run, k).as_deref(), Some(jpeg.as_slice()), "slot {k} missing");
        }
        assert!(core.scrub_cache.get(run, 2).is_none(), "failed slot 2 must not be cached");
        // Verify recorded engine calls passed exact URI and scrub::WIDTH.
        let calls = mock.thumbnail_calls.lock().unwrap();
        assert!(calls.iter().all(|(u, _, w, _)| u == &uri && *w == scrub::WIDTH));
        assert!(calls.iter().any(|(_, at, _, _)| *at == 4_000));
        drop(calls);

        let scrub_events: Vec<i64> = ui
            .events()
            .into_iter()
            .filter(|(name, _)| name == "player://scrub")
            .filter_map(|(_, payload)| (payload["run"].as_u64() == Some(run)).then(|| payload["k"].as_i64()).flatten())
            .collect();
        for k in [0i64, 1, 3, 4] {
            assert!(scrub_events.contains(&k), "missing player://scrub event for slot {k}");
        }
        assert!(!scrub_events.contains(&2), "failed slot 2 must not emit player://scrub");

        // Zero files created in `scrub_dir` and zero leftover temp files in temp_dir for this run.
        assert!(!core.scrub_dir.exists(), "scrub_fill must never touch cache/scrub on disk");
        let prefix = format!("revlc-scrub-{}-{run}-", std::process::id());
        let leftovers: Vec<_> = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
            .collect();
        assert!(leftovers.is_empty(), "leftover scrub temp files: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn metadata_fallback_fills_a_missing_duration_and_keeps_rust_tags() {
        let vlc = || Ok(engine::MediaMeta { duration: 180_021, title: Some("vlc".into()), ..Default::default() });
        // Fragmented MP4 / MKV without Info/Duration: Rust found tags but no length.
        let rust = engine::MediaMeta { title: Some("rust".into()), artwork: Some("art.jpg".into()), ..Default::default() };
        let meta = metadata_with_fallback(Some(rust), vlc).unwrap();
        assert_eq!(meta.duration, 180_021);
        assert_eq!(meta.title.as_deref(), Some("rust"));
        assert_eq!(meta.artwork.as_deref(), Some("art.jpg"));
        // A known duration never asks libVLC.
        let rust = engine::MediaMeta { duration: 5_000, ..Default::default() };
        let meta = metadata_with_fallback(Some(rust), || panic!("libVLC parse not needed")).unwrap();
        assert_eq!(meta.duration, 5_000);
        // Unsupported formats go to libVLC; a libVLC failure keeps the Rust tags.
        assert_eq!(metadata_with_fallback(None, vlc).unwrap().duration, 180_021);
        let rust = engine::MediaMeta { title: Some("rust".into()), ..Default::default() };
        let meta = metadata_with_fallback(Some(rust), || Err("timeout".into())).unwrap();
        assert_eq!((meta.duration, meta.title.as_deref()), (0, Some("rust")));
    }

    #[test]
    fn snapshots_are_saved_in_the_chosen_format() {
        let dir = std::env::temp_dir().join(format!("revlc-snapshot-format-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let png = |name: &str| {
            let path = dir.join(name);
            image::RgbImage::from_pixel(4, 2, image::Rgb([200, 10, 20])).save(&path).unwrap();
            path
        };
        png("earlier.png");
        let before = snapshot_pngs(&dir);
        let first = png("snap-00001.png");
        let new: Vec<_> = snapshot_pngs(&dir).difference(&before).cloned().collect();
        assert_eq!(new, vec![first.clone()], "only the new snapshot is converted");

        convert_snapshot(&first, "jpg").unwrap();
        let jpg = std::fs::read(dir.join("snap-00001.jpg")).unwrap();
        assert_eq!(&jpg[..3], &[0xFF, 0xD8, 0xFF], "JPEG signature");
        assert!(!first.exists(), "the PNG is replaced");

        // A later run where libVLC reuses number 1: the next free number is taken.
        convert_snapshot(&png("snap-00001.png"), "jpg").unwrap();
        assert!(dir.join("snap-00002.jpg").exists());

        convert_snapshot(&png("snap-2026-11-23-10h00m00s123.png"), "tiff").unwrap();
        let tiff = std::fs::read(dir.join("snap-2026-11-23-10h00m00s123.tiff")).unwrap();
        assert_eq!(&tiff[..4], b"II*\0", "TIFF signature");
        assert_eq!(&tiff[tiff.len() - 3..], &[200, 10, 20], "pixels stored uncompressed");
        assert_eq!(tiff.len(), 8 + 2 + 10 * 12 + 4 + 6 + 4 * 2 * 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_title_tag_matching_the_file_name_keeps_its_number() {
        let (core, _mock, _ui, dir) = test_core("music-title-stem");
        let song = |name: &str| Song {
            uri: dir.join(format!("{name}.mp3")).to_string_lossy().into_owned(),
            name: name.into(),
            root: dir.to_string_lossy().into_owned(),
            folder: String::new(),
            modified: 0,
            artist: None,
            album: None,
            album_artist: None,
            genre: None,
            track: None,
            disc: None,
            year: None,
            duration: 0,
            art: None,
        };
        let mut songs = [song("31-id3v23"), song("07 Untagged"), song("02 Old cache")];
        let title = |t: &str| engine::MediaMeta { title: Some(t.into()), ..Default::default() };
        core.meta.lock().unwrap().insert(&songs[0].uri, title("31-id3v23"));
        core.meta.lock().unwrap().insert(&songs[1].uri, engine::MediaMeta::default());
        // libVLC's file-name title, cached by an older version.
        core.meta.lock().unwrap().insert(&songs[2].uri, title("02 Old cache.mp3"));
        core.add_music_metadata(&mut songs, &mut Vec::new(), &mut Vec::new());
        let names: Vec<_> = songs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["31-id3v23", "Untagged", "Old cache"]);

        // libVLC's stand-in title (the file name, with or without extension) isn't a tag.
        let path = Path::new("/music/07 Song.flac");
        assert_eq!(without_file_name_title(title("07 Song.flac"), path).title, None);
        assert_eq!(without_file_name_title(title("07 Song"), path).title, None);
        assert_eq!(without_file_name_title(title("Song"), path).title.as_deref(), Some("Song"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn subtitles_libvlc_autodetects_are_not_added_again() {
        let dir = Path::new("/films");
        let video = dir.join("The Film (2020).mkv");
        let at = |name: &str, fuzzy| vlc_autodetects(&video, &dir.join(name), fuzzy);
        assert!(at("The Film (2020).srt", 3));
        assert!(at("the.film.2020.en.srt", 3), "case and punctuation don't matter");
        assert!(!at("Extras - The Film 2020.srt", 3));
        assert!(at("Extras - The Film 2020.srt", 2), "contains the name");
        assert!(at("Other.srt", 1) && !at("Other.srt", 2));
        assert!(!at("The Film (2020).en.srt", 4) && at("The Film (2020).srt", 4));
        assert!(!at("The Film (2020).srt", 0), "autodetection off");
        assert!(!vlc_autodetects(&video, Path::new("/elsewhere/The Film (2020).srt"), 3), "another folder");
        assert!(!at("The Film (2020).mks", 3), "libVLC doesn't look for .mks files");
        assert!(!vlc_autodetects(&dir.join("---.mkv"), &dir.join("Other.srt"), 3), "a name of only punctuation matches nothing");
    }
}
