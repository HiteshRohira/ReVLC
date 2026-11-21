mod bookmarks;
mod capture;
mod cli;
mod disc;
mod effects;
mod history;
mod library;
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
mod machine;
mod menu;
mod meta;
mod playlist;
mod playlist_file;
mod quality;
mod roster;
mod settings;
mod store;
mod telemetry;
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
use effects::{Adjust, Effects, Equalizer, Filter, FilterSpec, Logo, Marquee, StereoMode, Transform};
use history::{History, UiEntry};
use library::{Library, LibraryItem};
use meta::MetaCache;
use playlist::{Item, Playlist, Repeat, SortKey};
use playlist_file::file_uri;
use settings::{Resume, Settings};
use roster::Roster;
use vlc::{Chapter, Engine, MediaInfo, Navigate, State, Stats, Title, Track, Vlc, event};

/// State shared by every player window: the settings, history, library,
/// bookmarks and thumbnails, plus the libVLC instance and the windows.
struct Shared {
    engine: Arc<Engine>,
    app: AppHandle,
    history: Mutex<History>,
    settings: Mutex<Settings>,
    data_dir: PathBuf,
    /// The user's preferred languages, for the default subtitle language.
    languages: Vec<String>,
    /// Installed font families (read once; AppKit needs the main thread).
    fonts: Vec<String>,
    library: Mutex<Library>,
    thumbs: Thumbnails,
    /// Where cached thumbnails and artwork go.
    thumbs_dir: PathBuf,
    meta: Mutex<MetaCache>,
    bookmarks: Mutex<Bookmarks>,
    /// The last library scan, reused for a little while.
    library_scan: Mutex<Option<LibraryScan>>,
    /// Wakes the thread that writes the history (see [`Shared::save_history`]).
    history_saves: mpsc::Sender<()>,
    /// Held while writing the history, so writes land in order.
    history_write: Mutex<()>,
    /// Held while applying a settings change, so changes apply in order.
    settings_change: Mutex<()>,
    /// The player windows and their sessions.
    registry: Mutex<Registry>,
}

/// The open player windows, and the session each one plays with.
#[derive(Default)]
struct Registry {
    windows: Roster,
    sessions: HashMap<String, Arc<Session>>,
}

/// One player window's state: its own libVLC player and queue. Everything
/// app-wide is reached through [`Shared`] (a `Session` derefs to it).
struct Session {
    shared: Arc<Shared>,
    /// The window's label, which events for it are addressed to.
    label: String,
    vlc: Vlc,
    playlist: Mutex<Playlist>,
    /// Set once the video view exists; opens before that are queued so
    /// libVLC never falls back to its own window.
    ready: AtomicBool,
    pending: Mutex<Pending>,
    /// What to do once the current item starts playing.
    resume: Mutex<Option<PendingSeek>>,
    /// Distinguishes the user's snapshots from thumbnail captures.
    user_snapshot: AtomicBool,
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
    /// Playback quality measurements, for analytics.
    quality: Mutex<quality::Quality>,
    /// Equaliser, stereo mode, video effects and zoom; they stay on from
    /// one item to the next.
    effects: Mutex<Effects>,
    /// The teletext page last asked for, and the media generation it was for.
    teletext_page: Mutex<Option<(u64, i32)>>,
    /// `--fullscreen`: video goes fullscreen when it starts (first window only).
    start_fullscreen: bool,
    /// `--play-and-exit`: quit when the queue has played to its end (first window only).
    play_and_exit: bool,
}

impl std::ops::Deref for Session {
    type Target = Shared;
    fn deref(&self) -> &Shared {
        &self.shared
    }
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

/// The app-wide state, in Tauri's managed state. libVLC may be missing, in
/// which case every command reports the error.
struct Players(Result<Arc<Shared>, String>);

/// What a command works with: the shared state, and the session of the
/// window that called it (none for Settings). Resolved from the calling
/// window's label, so a command acts on its own window's player.
struct Player {
    shared: Result<Arc<Shared>, String>,
    session: Option<Arc<Session>>,
}

impl Player {
    /// The calling window's session.
    fn core(&self) -> Result<&Arc<Session>, String> {
        match (&self.shared, &self.session) {
            (Err(e), _) => Err(e.clone()),
            (_, Some(session)) => Ok(session),
            (_, None) => Err("this window has no player".into()),
        }
    }

    /// The app-wide state, which every window (Settings too) can use.
    fn shared(&self) -> Result<&Arc<Shared>, String> {
        self.shared.as_ref().map_err(Clone::clone)
    }
}

impl<'de, R: tauri::Runtime> tauri::ipc::CommandArg<'de, R> for Player {
    fn from_command(command: tauri::ipc::CommandItem<'de, R>) -> Result<Self, tauri::ipc::InvokeError> {
        let webview = command.message.webview();
        let shared = webview.try_state::<Players>().map(|p| p.0.clone()).unwrap_or_else(|| Err("ReVLC is still starting".into()));
        let session = shared.as_ref().ok().and_then(|s| s.session(webview.label()));
        Ok(Player { shared, session })
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
    video: Vec<Track>,
    video_current: i32,
    subtitles: Vec<Track>,
    subtitle_current: i32,
    /// Whether what's playing has teletext, and whether a teletext track is selected.
    teletext: bool,
    teletext_on: bool,
}

#[derive(Clone, Serialize)]
struct EqualizerInfo {
    presets: Vec<String>,
    /// Band centre frequencies in Hz.
    bands: Vec<f32>,
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

impl Session {
    /// Sends an event to this window only.
    fn emit<S: Serialize + Clone>(&self, event: &str, payload: S) {
        let _ = self.app.emit_to(self.label.as_str(), event, payload);
    }

    /// Applies the settings that live in this window's player rather than
    /// in the shared libVLC options; `changed` names the options that did.
    fn apply_player_settings(&self, settings: &Settings, changed: &[&str]) {
        self.vlc.set_hardware_decoding(settings.hardware_decoding);
        for name in changed {
            // The player keeps its own copy of these, so set them directly too.
            if name.starts_with("video-title-") {
                self.vlc.set_title_display(settings.show_title, settings.title_position, settings.title_timeout);
            } else if name.starts_with("deinterlace") && settings.deinterlace >= 0 {
                let on = settings.deinterlace == 1;
                self.vlc.set_deinterlace(on.then_some(settings.deinterlace_mode.as_str()).map(|m| if m == "auto" { "blend" } else { m }));
            }
        }
    }

    fn playlist(&self) -> MutexGuard<'_, Playlist> {
        self.playlist.lock().unwrap_or_else(|e| e.into_inner())
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
            let subtitles_on = self.vlc.subtitle_track() >= 0;
            self.quality().close("settings_changed");
            if self.vlc.open(&item.uri, &item.options, item.trusted).is_ok() {
                for uri in self.subtitles().iter() {
                    let _ = self.vlc.add_subtitle(uri, subtitles_on);
                }
            }
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

    fn subtitles(&self) -> MutexGuard<'_, Vec<String>> {
        self.subtitles.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds a subtitle file or URL to what's playing and selects it.
    fn add_subtitle(&self, uri: String) -> Res {
        // So it can't land on an item that's being replaced.
        let _transition = self.transition();
        self.vlc.add_subtitle(&uri, true)?;
        self.subtitles().push(uri);
        Ok(())
    }

    fn quality(&self) -> MutexGuard<'_, quality::Quality> {
        self.quality.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Tells the quality measurements about a seek the user asked for.
    fn seeking(&self, target: i64, kind: &'static str) {
        self.quality().seek_requested(&self.vlc, self.vlc.time(), target, kind);
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

    /// The current item's title, for messages.
    fn current_title(&self) -> String {
        self.current_item().map_or_else(|| "this media".into(), |it| it.title)
    }

    /// Records the current position, and grabs a thumbnail if there's video.
    fn save_progress(&self, thumbnail: bool) {
        let Some(uri) = self.current_uri() else { return };
        let started = match self.vlc.state() {
            State::Playing | State::Paused => true,
            // Still loading, or buffering after a seek: only once it has a position.
            State::Opening | State::Buffering => self.vlc.time() > 0,
            _ => false,
        };
        if !started || !in_history(&uri) {
            return;
        }
        if thumbnail {
            self.queue_resume_thumbnail(&uri);
        }
        self.history().update(&uri, self.vlc.time(), self.vlc.length());
        self.save_history();
    }

    fn emit_chapters(&self) {
        self.emit("player://chapters", self.vlc.chapters());
        self.emit("player://titles", self.vlc.titles());
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
            video: v.video_tracks(),
            video_current: v.video_track(),
            subtitles: v.subtitle_tracks(),
            subtitle_current: v.subtitle_track(),
            teletext: !v.teletext_tracks().is_empty(),
            teletext_on: v.teletext_on(),
        }
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

    /// Changes the effects with `change`, applies them, and tells the UI.
    fn update_effects(&self, change: impl FnOnce(&mut Effects)) {
        let effects = {
            let mut effects = self.effects();
            change(&mut effects);
            effects.clone()
        };
        self.emit("player://effects", effects);
    }

    /// Applies the video filters (rotation, colour adjustments, effects).
    fn apply_video_filters(&self) {
        let chain = self.effects().video_chain();
        self.vlc.set_video_filters(&chain);
    }

    /// Applies the audio filters (equaliser and effects); `restart` rebuilds them.
    fn apply_audio_filters(&self, restart: bool) {
        let (chain, params) = {
            let effects = self.effects();
            (effects.audio_chain(), effects.audio_params())
        };
        self.vlc.set_audio_filters(&chain, &params, restart);
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

    /// [`Session::play_index`], with the transition lock already held.
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
        let result = self.vlc.open(uri, &item.options, item.trusted);
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
        self.quality().opened(open, self.vlc.generation(), result.as_ref().err().map(String::as_str));
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
        self.save_progress(true);
        self.quality().close("stopped");
        self.vlc.stop();
        self.emit_state();
    }

    /// Adds paths or URLs to the playlist, optionally starting the first
    /// new item. Folders add their media (and join the library); playlist
    /// files add their entries.
    fn add(&self, uris: Vec<String>, play: bool) -> Result<(), String> {
        self.add_with_options(uris, play, &[])
    }

    /// [`Session::add`], giving each new item the libVLC `options` (from the
    /// command line, like ":start-time=30").
    fn add_with_options(&self, uris: Vec<String>, play: bool, options: &[String]) -> Result<(), String> {
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
            self.add_subtitle(file_uri(&uri))?;
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

impl Shared {
    fn registry(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The session of the player window `label`.
    fn session(&self, label: &str) -> Option<Arc<Session>> {
        self.registry().sessions.get(label).cloned()
    }

    /// Every open player's session.
    fn sessions(&self) -> Vec<Arc<Session>> {
        self.registry().sessions.values().cloned().collect()
    }

    /// The session of the window that menu actions and opened files go to:
    /// the focused player window, else the newest.
    fn target_session(&self) -> Option<Arc<Session>> {
        let registry = self.registry();
        registry.windows.target().and_then(|label| registry.sessions.get(label)).cloned()
    }

    /// Whether any window is playing, so thumbnail decoding can wait.
    fn any_playing(&self) -> bool {
        self.sessions().iter().any(|s| s.vlc.state() == State::Playing)
    }

    /// Pushes VLC-backed settings into libVLC's configuration (only the ones
    /// that differ from `previous`), which every player shares. Returns the
    /// names of the options that changed.
    fn apply_vlc_settings(&self, settings: &Settings, previous: Option<&Settings>) -> Vec<&'static str> {
        let new = settings.vlc_config(&self.languages);
        let old = previous.map(|p| p.vlc_config(&self.languages));
        let mut changed = Vec::new();
        for (i, (name, value)) in new.iter().enumerate() {
            if old.as_ref().is_none_or(|o| o[i].1 != *value) {
                self.engine.set_config(name, value);
                changed.push(*name);
            }
        }
        changed
    }

    /// Saves and applies new settings to every window, reopening the
    /// current videos if a change only takes effect when a video starts.
    fn set_settings(&self, settings: Settings) -> Res {
        let _changing = self.settings_change.lock().unwrap_or_else(|e| e.into_inner());
        let previous = self.settings().clone();
        if previous == settings {
            return Ok(());
        }
        // Saved first: if that fails, nothing has changed.
        settings.save(&self.data_dir)?;
        let changed = self.apply_vlc_settings(&settings, Some(&previous));
        let sessions = self.sessions();
        for session in &sessions {
            session.apply_player_settings(&settings, &changed);
            if settings.audio_device != previous.audio_device {
                session.vlc.set_audio_device(&settings.audio_device);
            }
        }
        telemetry::set_enabled(settings.analytics);
        *self.settings() = settings.clone();
        let _ = self.app.emit("settings://changed", settings);
        if !changed.is_empty() {
            for session in sessions {
                thread::spawn(move || session.reload());
            }
        }
        Ok(())
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
        let mut retry = Vec::new();
        for item in &mut items {
            let thumb = self.library_thumb_path(&item.uri);
            let made = meta::modified(&thumb);
            if made > 0 {
                item.thumb = Some(thumb.to_string_lossy().into_owned());
            }
            if made == 0 || (scanned && (made < meta::modified(Path::new(&item.uri)) || changed.contains(&item.uri))) {
                if made > 0 {
                    retry.push(thumb.to_string_lossy().into_owned());
                }
                jobs.push(ThumbJob { uri: item.uri.clone(), out: thumb, kind: JobKind::Frame(0.3) });
            }
        }
        retry.extend(self.add_metadata(&mut items, &mut jobs));
        // Changed files are worth another try even if they were tried this session.
        self.thumbs.tried.lock().unwrap_or_else(|e| e.into_inner()).retain(|key| !retry.contains(key));
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
    /// jobs for whatever hasn't been read or cached yet. Returns the job
    /// keys of files read before that have changed since.
    fn add_metadata(&self, items: &mut [LibraryItem], jobs: &mut Vec<ThumbJob>) -> Vec<String> {
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
                    let job = ThumbJob { uri: item.uri.clone(), out: PathBuf::new(), kind: JobKind::Meta };
                    if cache.is_stale(&item.uri) {
                        changed.push(job.key());
                    }
                    jobs.push(job);
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

    fn bookmarks(&self) -> MutexGuard<'_, Bookmarks> {
        self.bookmarks.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Generates library thumbnails one at a time. Video frames wait while
/// video plays so decoding never competes with playback; metadata and
/// artwork go ahead of them meanwhile.
fn make_thumbnails(core: Arc<Shared>) {
    loop {
        let job = {
            let mut queue = core.thumbs.queue.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                let playing = core.any_playing();
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
            JobKind::Frame(at) => core.engine.thumbnail(&job.uri, &job.out, at, 480),
            JobKind::Meta => {
                // Remember failures too, so unreadable files aren't parsed on every scan.
                let tags = core.engine.parse(&job.uri, 5000);
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
fn pump_events(core: Arc<Session>, rx: Receiver<vlc::Event>) {
    const TIME_INTERVAL: Duration = Duration::from_millis(250);
    const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

    let mut last_time_emit = Instant::now() - TIME_INTERVAL;
    let mut last_progress = Instant::now();
    let mut loop_jump: Option<Instant> = None;
    // The last media that got past its start. Some broken files end
    // without an error, having played nothing.
    let mut played = None;
    while let Ok(first) = rx.recv() {
        // Let bursts of events (e.g. opening a file) arrive, then handle them together.
        thread::sleep(Duration::from_millis(20));
        let batch: Vec<vlc::Event> = std::iter::once(first).chain(rx.try_iter()).collect();
        let events: Vec<i32> = batch.iter().map(|e| e.kind).collect();
        let has = |ty: i32| events.contains(&ty);
        // Ending or failing only counts for what's open now, not for
        // media replaced since (the user picked another item meanwhile).
        let transition = core.transition();
        let generation = core.vlc.generation();
        let current = |ty: i32| batch.iter().any(|e| e.kind == ty && e.generation == generation);
        if current(event::TIME_CHANGED) && core.vlc.time() > 0 {
            played = Some(generation);
        }

        // Web pages and remote playlists end by expanding into what they hold:
        // one stream plays in place, several replace the entry in the queue.
        let subitems = if current(event::END_REACHED) { core.vlc.subitems() } else { Vec::new() };
        // Before the queue moves on, so a finished item counts as finished.
        if subitems.is_empty() {
            core.quality().handle(&batch, &core.vlc);
        } else {
            let events: Vec<vlc::Event> = batch.iter().filter(|e| e.kind != event::END_REACHED).copied().collect();
            core.quality().handle(&events, &core.vlc);
        }
        if subitems.len() == 1 && core.vlc.play_subitem() {
            core.quality().followed(core.vlc.generation());
            if let Some(uri) = core.current_uri() {
                core.set_resume(&uri);
            }
        } else if subitems.len() > 1 {
            core.quality().forget();
            if let Err(e) = core.expand_current(subitems) {
                core.notice("error", e);
            }
        } else if current(event::END_REACHED) {
            if played == Some(generation) {
                if let Some(uri) = core.current_uri() {
                    core.history().finish(&uri);
                    core.save_history();
                }
            } else {
                core.notice("error", format!("Could not play {}", core.current_title()));
            }
            let stop_after_each = core.settings().stop_after_each;
            let sleep_now = std::mem::take(&mut core.sleep().after_item);
            // Like VLC's --play-and-exit: quit once there's nothing left to play.
            if core.play_and_exit && core.playlist().next_index(true).is_none() {
                core.app.exit(0);
            }
            if sleep_now {
                core.notice("info", "Sleep timer: stopped after this item");
            } else if !stop_after_each
                && let Err(e) = core.advance_locked(true)
            {
                core.notice("error", e);
            }
        }
        if current(event::ERROR) {
            core.notice("error", format!("Could not play {}", core.current_title()));
        }
        // The resume point belongs to what's open now: only its own start
        // may use it, and before anything else can be opened.
        if current(event::PLAYING) {
            match core.resume.lock().unwrap_or_else(|e| e.into_inner()).take() {
                Some(PendingSeek::Seek { ms, announce }) => {
                    core.vlc.set_time(ms);
                    if announce {
                        core.notice("osd", format!("Resuming from {}", format_time(ms)));
                    }
                }
                Some(PendingSeek::Ask(ms)) => core.offer_resume(ms),
                None => {}
            }
        }
        drop(transition);
        if has(event::PLAYING) {
            let device = core.settings().audio_device.clone();
            if !device.is_empty() {
                core.vlc.set_audio_device(&device);
            }
            // The audio output picks its own stereo mode for each stream.
            let stereo = core.effects().stereo;
            if stereo != StereoMode::Stereo && core.vlc.stereo_mode() != stereo.code() {
                core.vlc.set_stereo_mode(stereo.code());
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
                    core.seeking(a, "loop");
                    core.vlc.set_time(a);
                    loop_jump = Some(Instant::now());
                }
            }
        }
        if has(event::TIME_CHANGED) && last_progress.elapsed() >= PROGRESS_INTERVAL {
            last_progress = Instant::now();
            if let Some(uri) = core.current_uri() {
                core.history().update(&uri, core.vlc.time(), core.vlc.length());
                core.save_history();
            }
        }
        if [event::PLAYING, event::LENGTH_CHANGED, event::MEDIA_CHANGED, event::TITLE_CHANGED].iter().any(|&e| has(e)) {
            core.emit_chapters();
        }
        if has(event::PLAYING)
            && let Some(title) = core.vlc.media_title()
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
async fn snapshot(player: Player) -> Result<Snapshot, String> {
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
        chapters: core.vlc.chapters(),
        titles: core.vlc.titles(),
        effects: core.effects().clone(),
        start_fullscreen: core.start_fullscreen,
        engine: Ok(format!("libVLC {}", core.vlc.version)),
        early: core.early().take().unwrap_or_default(),
    })
    .await
}

// Commands that change what plays run off the main thread: they wait for
// the transition lock, and the event thread can hold it while libVLC needs
// the main thread to close the previous video.

#[tauri::command]
async fn open(player: Player, uris: Vec<String>, play: bool) -> Res {
    off_main(player, move |core| core.add(uris, play)).await?
}

#[tauri::command]
async fn play_index(player: Player, index: usize) -> Res {
    off_main(player, move |core| core.play_index(index)).await?
}

#[tauri::command]
async fn toggle_pause(player: Player) -> Res {
    off_main(player, toggle_pause_now).await?
}

fn toggle_pause_now(core: &Session) -> Res {
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
async fn stop(player: Player) -> Res {
    off_main(player, Session::stop).await
}

/// Runs `work` on a background thread: Tauri runs synchronous commands on
/// the main thread, where disk access would freeze the window.
async fn off_main<T: Send + 'static>(player: Player, work: impl FnOnce(&Session) -> T + Send + 'static) -> Result<T, String> {
    let core = player.core()?.clone();
    tauri::async_runtime::spawn_blocking(move || work(&core)).await.map_err(|e| e.to_string())
}

/// [`off_main`] for work on the app-wide state, which Settings can do too.
async fn off_shared<T: Send + 'static>(player: Player, work: impl FnOnce(&Shared) -> T + Send + 'static) -> Result<T, String> {
    let shared = player.shared()?.clone();
    tauri::async_runtime::spawn_blocking(move || work(&shared)).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn history_list(player: Player) -> Result<Vec<UiEntry>, String> {
    off_shared(player, history_entries).await
}

fn history_entries(core: &Shared) -> Vec<UiEntry> {
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
async fn settings_get(player: Player) -> Result<SettingsInfo, String> {
    // Listing audio devices can be slow.
    off_shared(player, |core| SettingsInfo {
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
async fn settings_set(player: Player, settings: Settings) -> Res {
    off_shared(player, move |core| core.set_settings(settings)).await?
}

/// Seeks to a resume point the user accepted.
#[tauri::command]
fn resume_at(player: Player, ms: i64) -> Res {
    let core = player.core()?;
    core.seeking(ms, "resume");
    core.vlc.set_time(ms);
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
        // Only macOS has more than one player window.
        #[cfg(target_os = "macos")]
        "newWindow" => {
            // Off the main thread: on Windows, creating a window there deadlocks WebView2.
            let app = app.clone();
            thread::spawn(move || {
                if let Some(shared) = shared_of(&app)
                    && let Err(e) = open_player(&shared, Launch::default())
                {
                    eprintln!("revlc: could not open a new window: {e}");
                    shared.target_session().inspect(|s| s.notice("error", format!("Could not open a new window: {e}")));
                }
            });
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
async fn library_list(player: Player) -> Result<LibraryList, String> {
    off_shared(player, Shared::library_list).await
}

/// Lists the library from a fresh scan, picking up new files.
#[tauri::command]
async fn library_rescan(player: Player) -> Result<LibraryList, String> {
    off_shared(player, |core| {
        core.rescan_library();
        core.library_list()
    })
    .await
}

#[tauri::command]
async fn library_add(player: Player, path: String) -> Result<LibraryList, String> {
    off_shared(player, move |core| {
        core.library().add(&path)?;
        Ok(core.library_list())
    })
    .await?
}

#[tauri::command]
async fn library_remove(player: Player, path: String) -> Result<LibraryList, String> {
    off_shared(player, move |core| {
        core.library().remove(&path)?;
        Ok(core.library_list())
    })
    .await?
}

#[tauri::command]
async fn history_remove(player: Player, uri: String) -> Res {
    off_shared(player, move |core| {
        core.history().remove(&uri);
        core.write_history(true)
    })
    .await?
}

#[tauri::command]
fn seek(player: Player, ms: i64) -> Res {
    let core = player.core()?;
    core.seeking(ms, "jump");
    core.vlc.set_time(ms);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn seek_by(player: Player, delta_ms: i64) -> Res {
    let core = player.core()?;
    let target = core.vlc.time() + delta_ms;
    let length = core.vlc.length();
    let target = if length > 0 { target.min(length - 500) } else { target };
    core.seeking(target, "skip");
    core.vlc.set_time(target);
    core.emit_state();
    Ok(())
}

#[tauri::command]
async fn next(player: Player) -> Res {
    off_main(player, |core| core.advance(false)).await?
}

#[tauri::command]
async fn previous(player: Player) -> Res {
    off_main(player, previous_now).await?
}

fn previous_now(core: &Session) -> Res {
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
fn next_frame(player: Player) -> Res {
    let core = player.core()?;
    core.vlc.next_frame();
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_volume(player: Player, volume: i32) -> Res {
    let core = player.core()?;
    core.vlc.set_volume(volume);
    if volume > 0 && core.vlc.muted() {
        core.vlc.set_muted(false);
    }
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_muted(player: Player, muted: bool) -> Res {
    let core = player.core()?;
    core.vlc.set_muted(muted);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_rate(player: Player, rate: f32) -> Res {
    let core = player.core()?;
    core.vlc.set_rate(rate);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_audio_track(player: Player, id: i32) -> Res {
    let core = player.core()?;
    core.vlc.set_audio_track(id);
    core.emit_tracks();
    Ok(())
}

#[tauri::command]
fn set_subtitle_track(player: Player, id: i32) -> Res {
    let core = player.core()?;
    core.vlc.set_subtitle_track(id);
    core.emit_tracks();
    Ok(())
}

#[tauri::command]
fn add_subtitle(player: Player, uri: String) -> Res {
    let core = player.core()?;
    let uri = if uri.contains("://") { uri } else { file_uri(&uri) };
    core.add_subtitle(uri)
}

#[tauri::command]
fn set_audio_delay(player: Player, ms: i64) -> Res {
    let core = player.core()?;
    core.vlc.set_audio_delay(ms);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_subtitle_delay(player: Player, ms: i64) -> Res {
    let core = player.core()?;
    core.vlc.set_subtitle_delay(ms);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_aspect_ratio(player: Player, ratio: Option<String>) -> Res {
    player.core()?.vlc.set_aspect_ratio(ratio.as_deref());
    Ok(())
}

#[tauri::command]
fn set_crop(player: Player, geometry: Option<String>) -> Res {
    player.core()?.vlc.set_crop(geometry.as_deref());
    Ok(())
}

#[tauri::command]
fn set_deinterlace(player: Player, mode: Option<String>) -> Res {
    player.core()?.vlc.set_deinterlace(mode.as_deref());
    Ok(())
}

#[tauri::command]
fn set_video_track(player: Player, id: i32) -> Res {
    let core = player.core()?;
    core.vlc.set_video_track(id);
    core.emit_tracks();
    Ok(())
}

#[tauri::command]
fn set_stereo_mode(player: Player, mode: StereoMode) -> Res {
    let core = player.core()?;
    // Kept even with nothing playing; it's applied when playback starts.
    core.vlc.set_stereo_mode(mode.code());
    core.update_effects(|e| e.stereo = mode);
    Ok(())
}

#[tauri::command]
fn equalizer_info(player: Player) -> Result<EqualizerInfo, String> {
    let vlc = &player.core()?.vlc;
    Ok(EqualizerInfo { presets: vlc.equalizer_presets(), bands: vlc.equalizer_bands() })
}

fn apply_equalizer(core: &Session, eq: Equalizer) {
    core.vlc.set_equalizer(eq.enabled.then_some((eq.preamp, eq.bands.as_slice())));
    core.update_effects(|e| e.equalizer = eq);
    // libVLC replaces the audio filters with just the equaliser.
    core.apply_audio_filters(false);
}

/// Loads one of libVLC's presets and turns the equaliser on.
#[tauri::command]
fn set_equalizer_preset(player: Player, index: u32) -> Res {
    let core = player.core()?;
    let (preamp, bands) = core.vlc.equalizer_preset(index).ok_or("no such preset")?;
    let eq = Equalizer { enabled: true, preset: Some(index), preamp, bands }.normalized(core.vlc.equalizer_bands().len());
    apply_equalizer(core, eq);
    Ok(())
}

/// Sets the equaliser from the UI's values. Once they no longer match the
/// preset they came from, it counts as a custom setting.
#[tauri::command]
fn set_equalizer(player: Player, equalizer: Equalizer) -> Res {
    let core = player.core()?;
    let mut eq = equalizer.normalized(core.vlc.equalizer_bands().len());
    if let Some(index) = eq.preset
        && !core.vlc.equalizer_preset(index).is_some_and(|(preamp, bands)| eq.matches(preamp, &bands))
    {
        eq.preset = None;
    }
    apply_equalizer(core, eq);
    Ok(())
}

#[tauri::command]
fn set_adjust(player: Player, adjust: Adjust) -> Res {
    let core = player.core()?;
    let adjust = adjust.normalized();
    core.vlc.set_adjust(&adjust);
    core.update_effects(|e| e.adjust = adjust);
    // libVLC keeps adjust in the video filter chain, along with the others.
    core.apply_video_filters();
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
fn set_filter(player: Player, id: String, filter: Filter) -> Res {
    let core = player.core()?;
    let audio = effects::AUDIO_FILTERS.iter().any(|s| s.id == id);
    let (known, crossfeed_changed) = {
        let mut effects = core.effects();
        let crossfeed = |e: &Effects| e.audio_params().into_iter().find(|p| p.name == "stereowiden-crossfeed").map(|p| p.value);
        let before = crossfeed(&effects);
        let known = effects.set_filter(&id, filter);
        (known, crossfeed(&effects) != before)
    };
    if !known {
        return Err(format!("no such filter: {id}"));
    }
    if audio {
        // VLC 3's stereo widener applies a new crossfeed as feedback, so it
        // only takes crossfeed when it starts.
        core.apply_audio_filters(crossfeed_changed);
    } else {
        core.apply_video_filters();
    }
    core.update_effects(|_| {});
    Ok(())
}

#[tauri::command]
fn set_transform(player: Player, transform: Transform) -> Res {
    let core = player.core()?;
    core.update_effects(|e| e.transform = transform);
    core.apply_video_filters();
    Ok(())
}

#[tauri::command]
fn set_logo(player: Player, logo: Logo) -> Res {
    let core = player.core()?;
    let logo = logo.normalized();
    core.vlc.set_logo(&logo);
    core.update_effects(|e| e.logo = logo);
    Ok(())
}

#[tauri::command]
fn set_marquee(player: Player, marquee: Marquee) -> Res {
    let core = player.core()?;
    let marquee = marquee.normalized();
    core.vlc.set_marquee(&marquee);
    core.update_effects(|e| e.marquee = marquee);
    Ok(())
}

#[tauri::command]
fn set_zoom(player: Player, zoom: f32) -> Res {
    let core = player.core()?;
    let zoom = effects::zoom_preset(zoom);
    core.vlc.set_zoom(zoom);
    core.update_effects(|e| e.zoom = zoom);
    Ok(())
}

/// Shows a teletext page, sends a coloured key, or with 0 turns teletext off.
#[tauri::command]
fn set_teletext(player: Player, page: i32) -> Res {
    let core = player.core()?;
    core.vlc.set_teletext(page);
    if (1..1000).contains(&page) {
        *core.teletext_page.lock().unwrap_or_else(|e| e.into_inner()) = Some((core.vlc.generation(), page));
    }
    core.emit_tracks();
    Ok(())
}

/// Teletext's background. The decoder reads it when it starts, so a
/// teletext track on screen is selected again to apply it. That starts it on
/// the track's own page, so the page the user went to is asked for again
/// once the new decoder is there.
#[tauri::command]
async fn set_teletext_transparent(player: Player, transparent: bool) -> Res {
    off_main(player, move |core| {
        core.vlc.set_config("vbi-opaque", &vlc::ConfigValue::Bool(!transparent));
        if core.vlc.teletext_on() {
            let generation = core.vlc.generation();
            let track = core.vlc.subtitle_track();
            core.vlc.set_subtitle_track(-1);
            core.vlc.set_subtitle_track(track);
            let page = *core.teletext_page.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((g, page)) = page.filter(|(g, _)| *g == generation) {
                thread::sleep(Duration::from_millis(400));
                if core.vlc.generation() == g && core.vlc.subtitle_track() == track {
                    core.vlc.set_teletext(page);
                }
            }
        }
        core.update_effects(|e| e.teletext_transparent = transparent);
        core.emit_tracks();
    })
    .await
}

#[tauri::command]
fn media_info(player: Player) -> Result<Option<MediaInfo>, String> {
    Ok(player.core()?.vlc.media_info())
}

#[tauri::command]
fn media_stats(player: Player) -> Result<Option<Stats>, String> {
    Ok(player.core()?.vlc.stats())
}

#[tauri::command]
async fn take_snapshot(app: AppHandle, player: Player) -> Res {
    // VLC's default folder: the Desktop on macOS, Pictures on Windows.
    let default = if cfg!(target_os = "macos") { app.path().desktop_dir() } else { app.path().picture_dir() };
    let dir = player.core()?.settings().snapshot_dir(default.ok()).ok_or("no snapshot folder")?;
    // libVLC waits for the video to hand over a frame.
    off_main(player, move |core| {
        core.user_snapshot.store(true, Ordering::Release);
        core.vlc.snapshot(&dir, 0)
    })
    .await?
}

#[tauri::command]
fn hide_cursor(app: AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.run_on_main_thread(surface_macos::hide_cursor);
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

#[tauri::command]
async fn playlist_remove(player: Player, index: usize) -> Res {
    off_main(player, move |core| {
        let _transition = core.transition();
        let was_current = core.playlist().remove(index);
        if was_current {
            core.quality().close("stopped");
            core.vlc.stop();
        }
        core.emit_playlist();
    })
    .await
}

#[tauri::command]
fn playlist_move(player: Player, from: usize, to: usize) -> Res {
    let core = player.core()?;
    core.playlist().move_item(from, to);
    core.emit_playlist();
    Ok(())
}

#[tauri::command]
async fn playlist_clear(player: Player) -> Res {
    off_main(player, playlist_clear_now).await
}

fn playlist_clear_now(core: &Session) {
    let _transition = core.transition();
    core.stop_locked();
    core.playlist().clear();
    core.emit_playlist();
}

#[tauri::command]
fn set_repeat(player: Player, repeat: Repeat) -> Res {
    let core = player.core()?;
    core.playlist().repeat = repeat;
    core.emit_playlist();
    Ok(())
}

#[tauri::command]
fn set_shuffle(player: Player, shuffle: bool) -> Res {
    let core = player.core()?;
    core.playlist().shuffle = shuffle;
    core.emit_playlist();
    Ok(())
}

#[tauri::command]
fn playlist_sort(player: Player, by: SortKey) -> Res {
    let core = player.core()?;
    core.playlist().sort(by);
    core.emit_playlist();
    Ok(())
}

/// Saves the queue as M3U, PLS or XSPF (chosen by the file's extension).
#[tauri::command]
fn playlist_save(player: Player, path: String) -> Res {
    let core = player.core()?;
    let items = core.playlist().items.clone();
    playlist_file::write(Path::new(&path), &items)
}

/// Sets the A–B loop points (ms); `None` clears one.
#[tauri::command]
fn set_ab_loop(player: Player, a: Option<i64>, b: Option<i64>) -> Res {
    let core = player.core()?;
    *core.ab_loop() = match (a, b) {
        (Some(a), Some(b)) if b < a => (Some(b), Some(a)),
        ab => ab,
    };
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_sleep_timer(player: Player, minutes: Option<u32>, after_item: bool) -> Res {
    player.core()?.set_sleep_timer(minutes, after_item);
    Ok(())
}

#[tauri::command]
fn set_title(player: Player, index: i32) -> Res {
    let core = player.core()?;
    core.vlc.set_title(index);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn set_chapter(player: Player, index: i32) -> Res {
    let core = player.core()?;
    if let Some(chapter) = core.vlc.chapters().get(index.max(0) as usize) {
        core.seeking(chapter.start, "chapter");
    }
    core.vlc.set_chapter(index);
    core.emit_state();
    Ok(())
}

#[tauri::command]
fn navigate(player: Player, action: Navigate) -> Res {
    player.core()?.vlc.navigate(action);
    Ok(())
}

/// Bookmarks the current position.
#[tauri::command]
fn bookmark_add(player: Player, name: Option<String>) -> Res {
    let core = player.core()?;
    let uri = core.current_uri().ok_or("Nothing is playing")?;
    let time = core.vlc.time();
    core.bookmarks().add(&uri, time, name)?;
    core.emit_bookmarks();
    core.notice("osd", format!("Bookmark added at {}", format_time(time)));
    Ok(())
}

#[tauri::command]
fn bookmark_remove(player: Player, time: i64) -> Res {
    let core = player.core()?;
    let uri = core.current_uri().ok_or("Nothing is playing")?;
    core.bookmarks().remove(&uri, time)?;
    core.emit_bookmarks();
    Ok(())
}

#[tauri::command]
fn bookmark_rename(player: Player, time: i64, name: String) -> Res {
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
async fn open_disc(player: Player, kind: DiscKind, path: String, menus: bool) -> Res {
    off_main(player, move |core| core.add_items(disc::items(kind, &path, menus)?, true)).await?
}

/// Lists capture devices. Runs on the main thread (Tauri runs synchronous
/// commands there), which AppKit needs for the screen list.
#[tauri::command]
fn capture_devices() -> Devices {
    capture::devices()
}

#[tauri::command]
async fn open_capture(player: Player, capture: Capture) -> Res {
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

/// The app-wide state, once `setup` has made it.
fn shared_of(app: &AppHandle) -> Option<Arc<Shared>> {
    app.try_state::<Players>().and_then(|p| p.0.as_ref().ok().cloned())
}

/// The label of the player window that menu actions and notices go to.
fn target_label(app: &AppHandle) -> Option<String> {
    shared_of(app).and_then(|s| s.registry().windows.target().map(String::from))
}

/// Shows a notice in the player window that has the focus.
fn notify(app: &AppHandle, notice: Notice) {
    if let Some(label) = target_label(app) {
        let _ = app.emit_to(label.as_str(), "player://notice", notice);
    }
}

/// What the command line gave the first window.
#[derive(Default)]
struct Launch {
    files: Vec<String>,
    media_options: Vec<String>,
    fullscreen: bool,
    play_and_exit: bool,
}

/// Opens a player window with a player of its own. The first one is the
/// window in `tauri.conf.json`; later ones are made here, with the same
/// settings. Not for the main thread (window creation can block there).
fn open_player(shared: &Arc<Shared>, launch: Launch) -> Result<Arc<Session>, String> {
    let label = shared.registry().windows.open();
    let result = open_player_as(shared, &label, launch);
    if result.is_err() {
        close_player(shared, &label);
    }
    result
}

fn open_player_as(shared: &Arc<Shared>, label: &str, launch: Launch) -> Result<Arc<Session>, String> {
    let vlc = Vlc::new(shared.engine.clone()).map_err(|e| e.message)?;
    let settings = shared.settings().clone();
    vlc.set_hardware_decoding(settings.hardware_decoding);
    // libVLC's player has its own title-display default; apply ours.
    vlc.set_title_display(settings.show_title, settings.title_position, settings.title_timeout);
    if !settings.audio_device.is_empty() {
        vlc.set_audio_device(&settings.audio_device);
    }
    let session = Arc::new(Session {
        shared: shared.clone(),
        label: label.to_string(),
        vlc,
        playlist: Mutex::default(),
        ready: AtomicBool::new(false),
        pending: Mutex::default(),
        resume: Mutex::default(),
        user_snapshot: AtomicBool::new(false),
        ab_loop: Mutex::default(),
        sleep: Mutex::default(),
        transition: Mutex::default(),
        reloads: AtomicU64::new(0),
        subtitles: Mutex::new(Vec::new()),
        early: Mutex::new(Some(Early::default())),
        quality: Mutex::default(),
        effects: Mutex::default(),
        teletext_page: Mutex::default(),
        start_fullscreen: launch.fullscreen,
        play_and_exit: launch.play_and_exit,
    });
    let (tx, rx) = mpsc::channel();
    session.vlc.subscribe(tx);
    let pump = session.clone();
    thread::Builder::new().name(format!("vlc-events-{label}")).spawn(move || pump_events(pump, rx)).map_err(|e| e.to_string())?;
    // Registered before the window exists: its page asks for a snapshot as soon as it loads.
    shared.registry().sessions.insert(label.to_string(), session.clone());

    let app = &shared.app;
    let window = match app.get_webview_window(label) {
        Some(window) => window,
        None => {
            let mut config = app.config().app.windows.first().cloned().ok_or("no window configuration")?;
            config.label = label.to_string();
            WebviewWindowBuilder::from_config(app, &config).and_then(|b| b.build()).map_err(|e| e.to_string())?
        }
    };
    #[cfg(windows)]
    quiet_webview(&window);
    // Files passed on the command line; they wait until the video view exists.
    if let Err(e) = session.add_with_options(launch.files, true, &launch.media_options) {
        session.notice("error", e);
    }
    attach_surface(&session, &window).map_err(|e| e.to_string())?;
    Ok(session)
}

/// Puts the video view in `window`, for `session`'s player to draw into.
fn attach_surface(session: &Arc<Session>, window: &tauri::WebviewWindow) -> tauri::Result<()> {
    #[cfg(windows)]
    let hwnd = window.hwnd()?.0 as isize;
    let attach = session.clone();
    window.with_webview(move |webview| {
        #[cfg(target_os = "macos")]
        {
            let (app, label) = (attach.app.clone(), attach.label.clone());
            let last = std::cell::Cell::new(Instant::now() - Duration::from_secs(1));
            let on_pointer = move || {
                // Throttled: the UI only needs to know the pointer is active.
                if last.get().elapsed() >= Duration::from_millis(150) {
                    last.set(Instant::now());
                    let _ = app.emit_to(label.as_str(), "native://pointer", ());
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
    })
}

/// Forgets a player window that has closed, and ends its player. Returns
/// whether it was the last one, which quits the app.
fn close_player(shared: &Arc<Shared>, label: &str) -> bool {
    let (session, last) = {
        let mut registry = shared.registry();
        (registry.sessions.remove(label), registry.windows.close(label))
    };
    if let Some(session) = session {
        // Stopping waits for libVLC, and releasing the player for its video
        // output, which may need the main thread: not from here.
        thread::spawn(move || {
            session.set_sleep_timer(None, false);
            session.vlc.shutdown();
        });
    }
    last
}

/// Closes a player window: stops its player first (saving the position),
/// so libVLC isn't left drawing into a window that's gone.
fn close_window_gracefully(window: tauri::Window) {
    let shared = shared_of(window.app_handle());
    thread::spawn(move || {
        if let Some(session) = shared.and_then(|s| s.session(window.label())) {
            session.stop();
        }
        let _ = window.destroy();
    });
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    updater::start(app.handle());
    let resource_dir = app.path().resource_dir().ok();
    let data_dir = app.path().app_data_dir()?;
    let languages = preferred_languages();
    let settings = Settings::load(&data_dir);
    // Extra libVLC options for debugging, e.g. REVLC_VLC_ARGS="--verbose=2".
    let args: Vec<String> = std::env::var("REVLC_VLC_ARGS").unwrap_or_default().split_whitespace().map(String::from).collect();
    let mut cli = cli::parse(std::env::args().skip(1));
    if let Ok(cwd) = std::env::current_dir() {
        cli.resolve_paths(&cwd);
    }
    let engine = Engine::load(resource_dir.as_deref(), &args, &cli.vlc_args, &settings.vlc_config(&languages));
    telemetry::start(&data_dir, settings.analytics, engine.as_ref().map_or("unavailable", |v| v.version.as_str()));
    let engine = match engine {
        Ok(engine) => engine,
        Err(e) => {
            // Nothing can play. The message names paths, so only the stage is sent.
            telemetry::capture("engine_failed", serde_json::json!({ "stage": e.stage }));
            eprintln!("revlc: {}", e.message);
            app.manage(Players(Err(e.message)));
            return Ok(());
        }
    };
    let cache_dir = app.path().app_cache_dir()?;
    let (history_saves, history_rx) = mpsc::channel();
    let shared = Arc::new(Shared {
        engine,
        app: app.handle().clone(),
        history: Mutex::new(History::load(data_dir.clone(), cache_dir.clone())),
        settings: Mutex::new(settings),
        languages,
        fonts: font_families(),
        library: Mutex::new(Library::load(&data_dir)),
        thumbs: Thumbnails::default(),
        thumbs_dir: cache_dir.join("thumbs"),
        meta: Mutex::new(MetaCache::load(&cache_dir)),
        bookmarks: Mutex::new(Bookmarks::load(&data_dir)),
        library_scan: Mutex::default(),
        history_saves,
        history_write: Mutex::default(),
        settings_change: Mutex::default(),
        registry: Mutex::default(),
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
    let thumbs = shared.clone();
    thread::Builder::new().name("thumbnails".into()).spawn(move || make_thumbnails(thumbs))?;
    let saver = shared.clone();
    thread::Builder::new().name("history".into()).spawn(move || {
        while history_rx.recv().is_ok() {
            // One write covers every request that came meanwhile.
            history_rx.try_iter().for_each(drop);
            let _ = saver.write_history(true);
        }
    })?;
    app.manage(Players(Ok(shared.clone())));

    // The first window gets the command line: its files, then any Finder
    // opened before now.
    let launch = Launch {
        files: cli.files,
        media_options: cli.media_options,
        fullscreen: cli.fullscreen,
        play_and_exit: cli.play_and_exit,
    };
    let session = open_player(&shared, launch)?;
    if !cli.ignored.is_empty() {
        session.notice("error", format!("Ignored command-line options: {}", cli.ignored.join(" ")));
    }
    if !shared.engine.rejected_args.is_empty() {
        session.notice("error", format!("libVLC didn't accept {}, so ReVLC started without them", shared.engine.rejected_args.join(" ")));
    }
    let early = std::mem::take(&mut *EARLY_OPENS.lock().unwrap_or_else(|e| e.into_inner()));
    let _ = session.add(early, true);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // Windows starts a new process for each file opened from Explorer; it
    // hands its files to the running copy and exits. Windows has a single
    // player window; more than one is macOS-only for now.
    #[cfg(windows)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
        // libVLC has started, so only the files and their start times count.
        let mut cli = cli::parse(args.into_iter().skip(1));
        cli.resolve_paths(Path::new(&cwd));
        let Some(session) = shared_of(app).and_then(|s| s.target_session()) else { return };
        if let Some(window) = app.get_webview_window(&session.label) {
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        if !cli.files.is_empty() {
            session.add_from_main(cli.files, cli.media_options);
        }
    }));
    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(setup)
        .on_window_event(|window, event| {
            let Some(shared) = shared_of(window.app_handle()) else { return };
            // Settings isn't a player window.
            if !shared.registry().windows.contains(window.label()) {
                return;
            }
            match event {
                WindowEvent::Focused(true) => shared.registry().windows.focus(window.label()),
                // With others open, stop this window's player before it goes.
                WindowEvent::CloseRequested { api, .. } if shared.registry().windows.len() > 1 => {
                    api.prevent_close();
                    close_window_gracefully(window.clone());
                }
                // The last player window closing quits, even with Settings
                // open, rather than leaving the app running with nothing to show.
                WindowEvent::Destroyed if close_player(&shared, window.label()) => window.app_handle().exit(0),
                WindowEvent::Destroyed => {}
                _ => {}
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "settings" => {
                if let Err(e) = show_settings(app) {
                    eprintln!("revlc: could not open settings: {e}");
                }
            }
            // Everything else is a player action the UI knows how to run,
            // in the window that has the focus.
            id => {
                if !run_app_item(app, id)
                    && let Some(label) = target_label(app)
                {
                    let _ = app.emit_to(label.as_str(), "menu://action", id.to_string());
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
        ])
        .build(tauri::generate_context!())
        .expect("error while building revlc");

    app.run(|handle, event| {
        // The Quit menu item ends the app without asking: only Exit comes.
        if matches!(event, RunEvent::ExitRequested { .. } | RunEvent::Exit) {
            if let Some(shared) = shared_of(handle) {
                for session in shared.sessions() {
                    session.save_progress(false);
                    session.quality().close("quit");
                }
                // Now, rather than on the history thread, which quitting doesn't wait for.
                let _ = shared.write_history(true);
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
            // Into the window that was used last; before there is one, `setup` plays them.
            match shared_of(handle).and_then(|s| s.target_session()) {
                Some(session) => session.add_from_main(uris, Vec::new()),
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
