//! Engine-agnostic playback abstraction and domain types.
//!
//! Everything libVLC-specific (raw C event codes, config option names,
//! filter-chain formatting, stereo-widener/equaliser/teletext quirks,
//! subitem expansion) lives in [`crate::vlc::VlcEngine`]. Core, quality
//! telemetry, network browsing and unit tests talk only to [`Engine`].

#[cfg(any(target_os = "macos", windows))]
use std::ffi::c_void;
use std::path::Path;
use std::sync::mpsc::Sender;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::effects::{Effects, Logo, Marquee, StereoMode};
use crate::settings::Settings;

/// Engine-agnostic playback event kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    MediaChanged,
    Opening,
    Buffering,
    Playing,
    Paused,
    Stopped,
    EndReached,
    Error,
    TimeChanged,
    SeekableChanged,
    TitleChanged,
    SnapshotTaken,
    LengthChanged,
    Vout,
    EsAdded,
    EsDeleted,
    EsSelected,
    Muted,
    Unmuted,
    AudioVolume,
    /// The audio output started on a device, which it does each time it
    /// (re)connects: PulseAudio's stream, after a stereo mode change too.
    AudioDevice,
    ChapterChanged,
}

/// A player event emitted by an [`Engine`].
#[derive(Clone, Copy, Debug)]
pub struct EngineEvent {
    pub kind: EventKind,
    /// The media generation it belongs to (see [`Engine::generation`]).
    pub generation: u64,
    /// When the engine emitted it; events are handled in batches, a little later.
    pub at: Instant,
    /// How full the input cache was, 0–100.
    pub cache: f32,
    /// The new time in ms, for [`EventKind::TimeChanged`]; -1 otherwise.
    pub time: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Idle,
    Opening,
    Buffering,
    Playing,
    Paused,
    Stopped,
    Ended,
    Error,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StreamInfo {
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Chapter {
    pub start: i64,
    pub duration: i64,
    pub name: String,
}

/// A title of the current media: a DVD or Blu-ray title, or the single
/// title of a normal file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Title {
    pub duration: i64,
    pub name: String,
    /// A disc menu (navigated with the arrow keys and Enter).
    pub menu: bool,
}

/// Disc menu navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Navigate {
    Activate = 0,
    Up = 1,
    Down = 2,
    Left = 3,
    Right = 4,
    Popup = 5,
}

/// Metadata read from a media file (tags, embedded artwork, duration).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MediaMeta {
    pub title: Option<String>,
    pub show: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub date: Option<String>,
    /// A local file holding the extracted artwork.
    pub artwork: Option<String>,
    /// Milliseconds, 0 when unknown.
    pub duration: i64,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    /// Track and disc numbers ("3", or "3/12" in the tag).
    pub track: Option<u32>,
    pub disc: Option<u32>,
}

/// Something found while browsing a server or folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrowseEntry {
    pub uri: String,
    pub name: String,
    pub dir: bool,
}

/// A server or share discovered on the local network.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Found {
    pub uri: String,
    pub name: String,
    /// A server to browse, rather than a stream to play.
    pub dir: bool,
    /// The kind of discovery that found it, such as "Bonjour Network Discovery".
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Track {
    pub id: i32,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tracks {
    pub audio: Vec<Track>,
    pub audio_current: i32,
    pub video: Vec<Track>,
    pub video_current: i32,
    pub subtitles: Vec<Track>,
    pub subtitle_current: i32,
    /// Whether what's playing has teletext, and whether a teletext track is selected.
    pub teletext: bool,
    pub teletext_on: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct EqualizerInfo {
    pub presets: Vec<String>,
    /// Band centre frequencies in Hz.
    pub bands: Vec<f32>,
}

/// What the media information dialog shows about one track.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    /// "audio", "video", "text" or "unknown".
    pub kind: &'static str,
    pub id: i32,
    /// The codec's fourcc, like "h264".
    pub codec: String,
    /// Engine's name for the codec, like "H264 - MPEG-4 AVC (part 10)".
    pub codec_name: Option<String>,
    pub language: Option<String>,
    pub description: Option<String>,
    /// Bits per second, 0 when unknown.
    pub bitrate: u32,
    pub channels: u32,
    /// Sample rate in Hz.
    pub rate: u32,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    /// Sample aspect ratio like "1:1", when known.
    pub sar: Option<String>,
    /// A text track's character encoding.
    pub encoding: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaInfo {
    /// The media's MRL.
    pub location: String,
    /// (field, value) for each tag the media has.
    pub meta: Vec<(&'static str, String)>,
    pub tracks: Vec<TrackInfo>,
}

/// Playback statistics of the current media (VLC's Statistics tab).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub read_bytes: i64,
    /// kb/s.
    pub input_bitrate: f32,
    pub demux_read_bytes: i64,
    /// kb/s.
    pub demux_bitrate: f32,
    pub demux_corrupted: i64,
    pub demux_discontinuity: i64,
    pub decoded_video: i64,
    pub decoded_audio: i64,
    pub displayed_pictures: i64,
    pub lost_pictures: i64,
    pub played_abuffers: i64,
    pub lost_abuffers: i64,
}

/// Why an engine couldn't start. `stage` is safe to send with analytics; the
/// message names paths, so it stays on the machine.
#[derive(Clone, Debug)]
pub struct LoadError {
    /// `not_found`, `load`, `symbol`, `version`, `init` or `player`.
    pub stage: &'static str,
    pub message: String,
}

impl LoadError {
    pub fn new(stage: &'static str, message: String) -> Self {
        Self { stage, message }
    }
}

/// What happened when the current media expanded into subitems (e.g., a Lua
/// playlist script or remote playlist).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubitemExpansion {
    /// No subitems; the media genuinely ended.
    None,
    /// A single subitem was started in place (and `generation()` advanced).
    Followed,
    /// Several subitems should replace the current playlist entry: `(mrl, title)`.
    Replace(Vec<(String, Option<String>)>),
}

/// An owned CPU frame handed from the engine to the GTK video surface on Linux.
#[cfg(any(target_os = "linux", test))]
pub struct PresentedFrame {
    pub pixels: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub stride: i32,
}

/// Playback engine interface implemented by [`crate::vlc::VlcEngine`] and
/// mocked in unit tests.
pub trait Engine: Send + Sync {
    /// Human-readable engine name & version for the UI (e.g. "libVLC 3.0.24").
    fn name_and_version(&self) -> String;
    /// Raw engine version string (e.g. "3.0.24 Vetinari").
    fn version(&self) -> &str;
    /// User command-line arguments the engine rejected during startup.
    fn rejected_args(&self) -> &[String];

    /// Identifies the media currently open; increments on every open/stop/follow.
    fn generation(&self) -> u64;
    /// Subscribes `tx` to engine events.
    fn subscribe(&self, tx: Sender<EngineEvent>);
    /// Stops playback and ends the event stream, so its reader finishes.
    /// The player is released once the last reference to it goes.
    fn shutdown(&self);
    /// Another player on the same engine (each player window has its own),
    /// with `settings` applied.
    fn new_player(&self, settings: &crate::settings::Settings) -> Result<std::sync::Arc<dyn Engine>, LoadError>;

    #[cfg(target_os = "macos")]
    fn set_nsview(&self, view: *mut c_void);
    #[cfg(windows)]
    fn set_hwnd(&self, hwnd: *mut c_void);
    #[cfg(target_os = "linux")]
    fn set_gtk_surface(&self, frames: std::sync::Arc<std::sync::Mutex<Option<PresentedFrame>>>);

    /// Opens a local path or URL and starts playback.
    fn open(&self, uri: &str, options: &[String], trusted: bool) -> Result<(), String>;
    fn stop(&self);
    fn set_pause(&self, pause: bool);
    fn state(&self) -> State;
    fn time(&self) -> i64;
    fn set_time(&self, ms: i64);
    fn length(&self) -> i64;
    fn seekable(&self) -> bool;
    fn has_video(&self) -> bool;
    fn buffering(&self) -> f32;
    fn rate(&self) -> f32;
    fn set_rate(&self, rate: f32);
    fn next_frame(&self);
    fn volume(&self) -> i32;
    fn set_volume(&self, volume: i32);
    /// The volume each new media is set to before it plays (negative: none).
    fn set_start_volume(&self, volume: i32);
    fn muted(&self) -> bool;
    fn set_muted(&self, muted: bool);

    /// Inspects subitems produced by the current media on end-of-stream.
    /// If there is exactly one, plays it in place and returns [`SubitemExpansion::Followed`].
    fn expand_subitems(&self) -> SubitemExpansion;

    // Tracks, subtitles, chapters, titles, navigation, teletext.
    fn tracks(&self) -> Tracks;
    fn set_audio_track(&self, id: i32);
    fn video_tracks(&self) -> Vec<Track>;
    fn video_track(&self) -> i32;
    fn set_video_track(&self, id: i32);
    fn subtitle_track(&self) -> i32;
    fn set_subtitle_track(&self, id: i32);
    fn add_subtitle(&self, uri: &str, select: bool) -> Result<(), String>;
    fn audio_delay(&self) -> i64;
    fn set_audio_delay(&self, ms: i64);
    fn subtitle_delay(&self) -> i64;
    fn set_subtitle_delay(&self, ms: i64);
    fn chapters(&self) -> Vec<Chapter>;
    fn titles(&self) -> Vec<Title>;
    fn title(&self) -> i32;
    fn set_title(&self, index: i32);
    fn chapter(&self) -> i32;
    fn set_chapter(&self, index: i32);
    fn navigate(&self, action: Navigate);
    fn set_teletext(&self, page: i32);
    fn set_teletext_transparent(&self, transparent: bool);

    // Settings, output devices, geometry & snapshots.
    /// Applies engine-backed settings (diffing against `previous` when present).
    /// Returns `true` if a change requires reopening the active media.
    fn apply_settings(&self, settings: &Settings, previous: Option<&Settings>, system_languages: &[String]) -> bool;
    fn audio_devices(&self) -> Vec<(String, String)>;
    fn set_aspect_ratio(&self, ratio: Option<&str>);
    fn set_crop(&self, geometry: Option<&str>);
    fn set_deinterlace(&self, mode: Option<&str>);
    fn snapshot(&self, path: &Path, width: u32) -> Result<(), String>;

    // Effects (engine handles backend-specific filter-chain & quirk details).
    fn set_stereo_mode(&self, mode: StereoMode) -> bool;
    /// Reapplies settings/effects that the backend resets when a new stream starts playing.
    fn on_playing(&self, settings: &Settings, effects: &Effects);
    fn equalizer_info(&self) -> EqualizerInfo;
    fn equalizer_preset(&self, index: u32) -> Option<(f32, Vec<f32>)>;
    fn apply_equalizer(&self, effects: &Effects);
    fn apply_adjust(&self, effects: &Effects);
    fn apply_video_filters(&self, effects: &Effects);
    fn apply_audio_filters(&self, effects: &Effects, previous: &Effects);
    fn apply_logo(&self, logo: &Logo);
    fn apply_marquee(&self, marquee: &Marquee);
    fn set_zoom(&self, factor: f32);

    // Info, stats, metadata parsing, thumbnails & network.
    fn stream_info(&self) -> StreamInfo;
    fn media_title(&self) -> Option<String>;
    fn media_info(&self) -> Option<MediaInfo>;
    fn stats(&self) -> Option<Stats>;
    fn parse(&self, path: &str, timeout_ms: i32) -> Result<MediaMeta, String>;
    fn thumbnail(&self, path: &str, out: &Path, fraction: f64, width: u32) -> Result<(), String>;
    fn thumbnail_at(&self, path: &str, out: &Path, start: i64, width: u32, precise: bool) -> Result<(), String>;
    fn browse(&self, url: &str, timeout_ms: i32) -> Result<Vec<BrowseEntry>, String>;
    fn discovery_start(&self);
    fn discovery_stop(&self);
    fn discovered(&self) -> Vec<Found>;
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU64, Ordering};
    use std::sync::Mutex;

    /// Configurable in-memory [`Engine`] for unit-testing `Core` and `Quality`.
    pub struct MockEngine {
        pub version: String,
        pub generation: AtomicU64,
        pub state: Mutex<State>,
        pub time: AtomicI64,
        pub length: AtomicI64,
        pub seekable: AtomicBool,
        pub has_video: AtomicBool,
        pub buffering: Mutex<f32>,
        pub rate: Mutex<f32>,
        pub volume: AtomicI32,
        pub start_volume: AtomicI32,
        pub muted: AtomicBool,
        pub stream_info: Mutex<StreamInfo>,
        pub media_title: Mutex<Option<String>>,
        pub tracks: Mutex<Tracks>,
        pub chapters: Mutex<Vec<Chapter>>,
        pub titles: Mutex<Vec<Title>>,
        pub subitems: Mutex<SubitemExpansion>,
        pub open_error: Mutex<Option<String>>,
        pub settings_reload: AtomicBool,
        pub opened_uris: Mutex<Vec<String>>,
        pub seeks: Mutex<Vec<i64>>,
        pub stops: AtomicU64,
        pub pauses: Mutex<Vec<bool>>,
        pub added_subtitles: Mutex<Vec<(String, bool)>>,
        pub teletext_transparent_calls: Mutex<Vec<bool>>,
        pub thumbnail_bytes: Mutex<Option<Vec<u8>>>,
        pub thumbnail_calls: Mutex<Vec<(String, i64, u32, bool)>>,
        pub thumbnail_fail_at: Mutex<Option<i64>>,
        pub parse_calls: Mutex<Vec<String>>,
        pub tx: Mutex<Option<Sender<EngineEvent>>>,
        /// The players made with [`Engine::new_player`].
        pub players: Mutex<Vec<std::sync::Arc<MockEngine>>>,
        pub discovery_stops: AtomicU64,
    }

    impl Default for MockEngine {
        fn default() -> Self {
            Self {
                version: "3.0.24-mock".into(),
                generation: AtomicU64::new(0),
                state: Mutex::new(State::Idle),
                time: AtomicI64::new(0),
                length: AtomicI64::new(120_000),
                seekable: AtomicBool::new(true),
                has_video: AtomicBool::new(true),
                buffering: Mutex::new(100.0),
                rate: Mutex::new(1.0),
                volume: AtomicI32::new(100),
                start_volume: AtomicI32::new(-1),
                muted: AtomicBool::new(false),
                stream_info: Mutex::new(StreamInfo {
                    video_codec: Some("h264".into()),
                    audio_codec: Some("mp4a".into()),
                    width: 1920,
                    height: 1080,
                    fps: 24.0,
                }),
                media_title: Mutex::new(None),
                tracks: Mutex::new(Tracks::default()),
                chapters: Mutex::new(Vec::new()),
                titles: Mutex::new(Vec::new()),
                subitems: Mutex::new(SubitemExpansion::None),
                open_error: Mutex::new(None),
                settings_reload: AtomicBool::new(false),
                opened_uris: Mutex::new(Vec::new()),
                seeks: Mutex::new(Vec::new()),
                stops: AtomicU64::new(0),
                pauses: Mutex::new(Vec::new()),
                added_subtitles: Mutex::new(Vec::new()),
                teletext_transparent_calls: Mutex::new(Vec::new()),
                thumbnail_bytes: Mutex::new(None),
                thumbnail_calls: Mutex::new(Vec::new()),
                thumbnail_fail_at: Mutex::new(None),
                parse_calls: Mutex::new(Vec::new()),
                tx: Mutex::new(None),
                players: Mutex::new(Vec::new()),
                discovery_stops: AtomicU64::new(0),
            }
        }
    }

    impl MockEngine {
        pub fn send_event(&self, kind: EventKind, cache: f32, time: i64) {
            if let Some(tx) = self.tx.lock().unwrap().as_ref() {
                let _ = tx.send(EngineEvent {
                    kind,
                    generation: self.generation(),
                    at: Instant::now(),
                    cache,
                    time,
                });
            }
        }
    }

    impl Engine for MockEngine {
        fn name_and_version(&self) -> String {
            format!("MockEngine {}", self.version)
        }
        fn version(&self) -> &str {
            &self.version
        }
        fn rejected_args(&self) -> &[String] {
            &[]
        }
        fn generation(&self) -> u64 {
            self.generation.load(Ordering::Acquire)
        }
        fn subscribe(&self, tx: Sender<EngineEvent>) {
            *self.tx.lock().unwrap() = Some(tx);
        }
        fn shutdown(&self) {
            self.stop();
            *self.tx.lock().unwrap() = None;
        }
        fn new_player(&self, _settings: &crate::settings::Settings) -> Result<std::sync::Arc<dyn Engine>, LoadError> {
            let player = std::sync::Arc::new(MockEngine::default());
            self.players.lock().unwrap().push(player.clone());
            Ok(player)
        }
        #[cfg(target_os = "macos")]
        fn set_nsview(&self, _view: *mut c_void) {}
        #[cfg(windows)]
        fn set_hwnd(&self, _hwnd: *mut c_void) {}
        #[cfg(target_os = "linux")]
        fn set_gtk_surface(&self, _frames: std::sync::Arc<std::sync::Mutex<Option<PresentedFrame>>>) {}

        fn open(&self, uri: &str, _options: &[String], _trusted: bool) -> Result<(), String> {
            self.opened_uris.lock().unwrap().push(uri.to_string());
            if let Some(err) = self.open_error.lock().unwrap().clone() {
                return Err(err);
            }
            self.generation.fetch_add(1, Ordering::AcqRel);
            *self.state.lock().unwrap() = State::Playing;
            Ok(())
        }
        fn stop(&self) {
            self.stops.fetch_add(1, Ordering::AcqRel);
            self.generation.fetch_add(1, Ordering::AcqRel);
            *self.state.lock().unwrap() = State::Stopped;
        }
        fn set_pause(&self, pause: bool) {
            self.pauses.lock().unwrap().push(pause);
            *self.state.lock().unwrap() = if pause { State::Paused } else { State::Playing };
        }
        fn state(&self) -> State {
            *self.state.lock().unwrap()
        }
        fn time(&self) -> i64 {
            self.time.load(Ordering::Acquire)
        }
        fn set_time(&self, ms: i64) {
            self.time.store(ms.max(0), Ordering::Release);
            self.seeks.lock().unwrap().push(ms.max(0));
        }
        fn length(&self) -> i64 {
            self.length.load(Ordering::Acquire)
        }
        fn seekable(&self) -> bool {
            self.seekable.load(Ordering::Acquire)
        }
        fn has_video(&self) -> bool {
            self.has_video.load(Ordering::Acquire)
        }
        fn buffering(&self) -> f32 {
            *self.buffering.lock().unwrap()
        }
        fn rate(&self) -> f32 {
            *self.rate.lock().unwrap()
        }
        fn set_rate(&self, rate: f32) {
            *self.rate.lock().unwrap() = rate.clamp(0.25, 4.0);
        }
        fn next_frame(&self) {}
        fn volume(&self) -> i32 {
            self.volume.load(Ordering::Acquire)
        }
        fn set_volume(&self, volume: i32) {
            self.volume.store(volume.clamp(0, 200), Ordering::Release);
        }
        fn set_start_volume(&self, volume: i32) {
            self.start_volume.store(volume, Ordering::Release);
        }
        fn muted(&self) -> bool {
            self.muted.load(Ordering::Acquire)
        }
        fn set_muted(&self, muted: bool) {
            self.muted.store(muted, Ordering::Release);
        }
        fn expand_subitems(&self) -> SubitemExpansion {
            let exp = std::mem::replace(&mut *self.subitems.lock().unwrap(), SubitemExpansion::None);
            if matches!(exp, SubitemExpansion::Followed) {
                self.generation.fetch_add(1, Ordering::AcqRel);
            }
            exp
        }
        fn tracks(&self) -> Tracks {
            self.tracks.lock().unwrap().clone()
        }
        fn set_audio_track(&self, id: i32) {
            self.tracks.lock().unwrap().audio_current = id;
        }
        fn video_tracks(&self) -> Vec<Track> {
            self.tracks.lock().unwrap().video.clone()
        }
        fn video_track(&self) -> i32 {
            self.tracks.lock().unwrap().video_current
        }
        fn set_video_track(&self, id: i32) {
            self.tracks.lock().unwrap().video_current = id;
        }
        fn subtitle_track(&self) -> i32 {
            self.tracks.lock().unwrap().subtitle_current
        }
        fn set_subtitle_track(&self, id: i32) {
            self.tracks.lock().unwrap().subtitle_current = id;
        }
        fn add_subtitle(&self, uri: &str, select: bool) -> Result<(), String> {
            self.added_subtitles.lock().unwrap().push((uri.to_string(), select));
            Ok(())
        }
        fn audio_delay(&self) -> i64 {
            0
        }
        fn set_audio_delay(&self, _ms: i64) {}
        fn subtitle_delay(&self) -> i64 {
            0
        }
        fn set_subtitle_delay(&self, _ms: i64) {}
        fn chapters(&self) -> Vec<Chapter> {
            self.chapters.lock().unwrap().clone()
        }
        fn titles(&self) -> Vec<Title> {
            self.titles.lock().unwrap().clone()
        }
        fn title(&self) -> i32 {
            -1
        }
        fn set_title(&self, _index: i32) {}
        fn chapter(&self) -> i32 {
            -1
        }
        fn set_chapter(&self, _index: i32) {}
        fn navigate(&self, _action: Navigate) {}
        fn set_teletext(&self, _page: i32) {}
        fn set_teletext_transparent(&self, transparent: bool) {
            self.teletext_transparent_calls.lock().unwrap().push(transparent);
        }
        fn apply_settings(&self, _settings: &Settings, _previous: Option<&Settings>, _system_languages: &[String]) -> bool {
            self.settings_reload.load(Ordering::Acquire)
        }
        fn audio_devices(&self) -> Vec<(String, String)> {
            vec![("".into(), "System default".into())]
        }
        fn set_aspect_ratio(&self, _ratio: Option<&str>) {}
        fn set_crop(&self, _geometry: Option<&str>) {}
        fn set_deinterlace(&self, _mode: Option<&str>) {}
        fn snapshot(&self, _path: &Path, _width: u32) -> Result<(), String> {
            Ok(())
        }
        fn set_stereo_mode(&self, _mode: StereoMode) -> bool {
            true
        }
        fn on_playing(&self, _settings: &Settings, _effects: &Effects) {}
        fn equalizer_info(&self) -> EqualizerInfo {
            EqualizerInfo { presets: vec!["Flat".into()], bands: vec![60.0; 10] }
        }
        fn equalizer_preset(&self, index: u32) -> Option<(f32, Vec<f32>)> {
            (index == 0).then(|| (12.0, vec![0.0; 10]))
        }
        fn apply_equalizer(&self, _effects: &Effects) {}
        fn apply_adjust(&self, _effects: &Effects) {}
        fn apply_video_filters(&self, _effects: &Effects) {}
        fn apply_audio_filters(&self, _effects: &Effects, _previous: &Effects) {}
        fn apply_logo(&self, _logo: &Logo) {}
        fn apply_marquee(&self, _marquee: &Marquee) {}
        fn set_zoom(&self, _factor: f32) {}
        fn stream_info(&self) -> StreamInfo {
            self.stream_info.lock().unwrap().clone()
        }
        fn media_title(&self) -> Option<String> {
            self.media_title.lock().unwrap().clone()
        }
        fn media_info(&self) -> Option<MediaInfo> {
            None
        }
        fn stats(&self) -> Option<Stats> {
            None
        }
        fn parse(&self, path: &str, _timeout_ms: i32) -> Result<MediaMeta, String> {
            self.parse_calls.lock().unwrap().push(path.to_string());
            Ok(MediaMeta::default())
        }
        fn thumbnail(&self, _path: &str, _out: &Path, _fraction: f64, _width: u32) -> Result<(), String> {
            Ok(())
        }
        fn thumbnail_at(&self, path: &str, out: &Path, start: i64, width: u32, precise: bool) -> Result<(), String> {
            self.thumbnail_calls.lock().unwrap().push((path.to_string(), start, width, precise));
            if let Some(bytes) = self.thumbnail_bytes.lock().unwrap().as_ref() {
                std::fs::write(out, bytes).map_err(|e| e.to_string())?;
            }
            if *self.thumbnail_fail_at.lock().unwrap() == Some(start) {
                return Err("injected thumbnail failure after partial write".into());
            }
            Ok(())
        }
        fn browse(&self, _url: &str, _timeout_ms: i32) -> Result<Vec<BrowseEntry>, String> {
            Ok(Vec::new())
        }
        fn discovery_start(&self) {}
        fn discovery_stop(&self) {
            self.discovery_stops.fetch_add(1, Ordering::AcqRel);
        }
        fn discovered(&self) -> Vec<Found> {
            Vec::new()
        }
    }
}
