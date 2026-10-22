//! Thin wrapper around libVLC 3, loaded at runtime.
//!
//! Everything libVLC-specific lives here so the rest of the app can move to
//! libVLC 4 later without changes.

use std::ffi::{CStr, CString, c_char, c_float, c_int, c_uint, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::time::Instant;

use libloading::Library;
use serde::Serialize;

type Inst = c_void;
type Player = c_void;
type Media = c_void;
type MediaList = c_void;
type EventManager = c_void;
type EventCallback = unsafe extern "C" fn(*const c_void, *mut c_void);
type LockCb = unsafe extern "C" fn(*mut c_void, *mut *mut c_void) -> *mut c_void;
type UnlockCb = unsafe extern "C" fn(*mut c_void, *mut c_void, *const *mut c_void);
type DisplayCb = unsafe extern "C" fn(*mut c_void, *mut c_void);
type FormatCb = unsafe extern "C" fn(*mut *mut c_void, *mut c_char, *mut c_uint, *mut c_uint, *mut c_uint, *mut c_uint) -> c_uint;
type CleanupCb = unsafe extern "C" fn(*mut c_void);

#[repr(C)]
struct ChapterDescription {
    time_offset: i64,
    duration: i64,
    name: *mut c_char,
}

#[repr(C)]
struct TitleDescription {
    duration: i64,
    name: *mut c_char,
    flags: c_uint,
}

#[repr(C)]
struct AudioDevice {
    next: *mut AudioDevice,
    device: *mut c_char,
    description: *mut c_char,
}

/// libvlc_media_track_t.
#[repr(C)]
struct MediaTrack {
    codec: u32,
    original_fourcc: u32,
    id: c_int,
    kind: c_int,
    profile: c_int,
    level: c_int,
    /// Points at a libvlc_video_track_t for video tracks.
    details: *mut c_void,
    bitrate: c_uint,
    language: *mut c_char,
    description: *mut c_char,
}

/// The start of libvlc_video_track_t.
#[repr(C)]
struct VideoTrack {
    height: c_uint,
    width: c_uint,
    sar_num: c_uint,
    sar_den: c_uint,
    frame_rate_num: c_uint,
    frame_rate_den: c_uint,
}

#[repr(C)]
struct TrackDescription {
    id: c_int,
    name: *mut c_char,
    next: *mut TrackDescription,
}

macro_rules! api {
    ($($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        #[allow(non_snake_case, dead_code)]
        struct Api { $($name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)* }

        impl Api {
            unsafe fn load(lib: &'static Library) -> Result<Self, libloading::Error> {
                unsafe {
                    Ok(Self { $($name: *lib.get(concat!(stringify!($name), "\0").as_bytes())?,)* })
                }
            }
        }
    };
}

api! {
    libvlc_new: fn(c_int, *const *const c_char) -> *mut Inst;
    libvlc_errmsg: fn() -> *const c_char;
    libvlc_get_version: fn() -> *const c_char;
    libvlc_set_user_agent: fn(*mut Inst, *const c_char, *const c_char);
    libvlc_set_app_id: fn(*mut Inst, *const c_char, *const c_char, *const c_char);
    libvlc_free: fn(*mut c_void);
    libvlc_media_new_path: fn(*mut Inst, *const c_char) -> *mut Media;
    libvlc_media_new_location: fn(*mut Inst, *const c_char) -> *mut Media;
    libvlc_media_release: fn(*mut Media);
    libvlc_media_add_option_flag: fn(*mut Media, *const c_char, c_uint);
    libvlc_media_subitems: fn(*mut Media) -> *mut MediaList;
    libvlc_media_get_mrl: fn(*mut Media) -> *mut c_char;
    libvlc_media_list_lock: fn(*mut MediaList);
    libvlc_media_list_unlock: fn(*mut MediaList);
    libvlc_media_list_count: fn(*mut MediaList) -> c_int;
    libvlc_media_list_item_at_index: fn(*mut MediaList, c_int) -> *mut Media;
    libvlc_media_list_release: fn(*mut MediaList);
    libvlc_media_player_release: fn(*mut Player);
    libvlc_video_set_callbacks: fn(*mut Player, LockCb, Option<UnlockCb>, Option<DisplayCb>, *mut c_void);
    libvlc_video_set_format_callbacks: fn(*mut Player, FormatCb, Option<CleanupCb>);
    libvlc_media_get_meta: fn(*mut Media, c_int) -> *mut c_char;
    libvlc_media_player_new: fn(*mut Inst) -> *mut Player;
    libvlc_media_player_set_media: fn(*mut Player, *mut Media);
    libvlc_media_player_get_media: fn(*mut Player) -> *mut Media;
    libvlc_media_player_event_manager: fn(*mut Player) -> *mut EventManager;
    libvlc_event_attach: fn(*mut EventManager, c_int, EventCallback, *mut c_void) -> c_int;
    libvlc_media_player_play: fn(*mut Player) -> c_int;
    libvlc_media_player_set_pause: fn(*mut Player, c_int);
    libvlc_media_player_stop: fn(*mut Player);
    libvlc_media_player_set_nsobject: fn(*mut Player, *mut c_void);
    libvlc_media_player_set_hwnd: fn(*mut Player, *mut c_void);
    libvlc_media_player_get_length: fn(*mut Player) -> i64;
    libvlc_media_player_get_time: fn(*mut Player) -> i64;
    libvlc_media_player_set_time: fn(*mut Player, i64);
    libvlc_media_player_get_state: fn(*mut Player) -> c_int;
    libvlc_media_player_is_seekable: fn(*mut Player) -> c_int;
    libvlc_media_player_has_vout: fn(*mut Player) -> c_uint;
    libvlc_media_player_set_rate: fn(*mut Player, c_float) -> c_int;
    libvlc_media_player_get_rate: fn(*mut Player) -> c_float;
    libvlc_media_player_next_frame: fn(*mut Player);
    libvlc_media_player_set_video_title_display: fn(*mut Player, c_int, c_uint);
    libvlc_media_player_add_slave: fn(*mut Player, c_int, *const c_char, bool) -> c_int;
    libvlc_audio_get_volume: fn(*mut Player) -> c_int;
    libvlc_audio_set_volume: fn(*mut Player, c_int) -> c_int;
    libvlc_audio_get_mute: fn(*mut Player) -> c_int;
    libvlc_audio_set_mute: fn(*mut Player, c_int);
    libvlc_audio_get_track_description: fn(*mut Player) -> *mut TrackDescription;
    libvlc_audio_output_device_list_get: fn(*mut Inst, *const c_char) -> *mut AudioDevice;
    libvlc_audio_output_device_list_release: fn(*mut AudioDevice);
    libvlc_audio_output_device_set: fn(*mut Player, *const c_char, *const c_char);
    libvlc_audio_get_track: fn(*mut Player) -> c_int;
    libvlc_audio_set_track: fn(*mut Player, c_int) -> c_int;
    libvlc_audio_get_delay: fn(*mut Player) -> i64;
    libvlc_audio_set_delay: fn(*mut Player, i64) -> c_int;
    libvlc_video_get_spu_description: fn(*mut Player) -> *mut TrackDescription;
    libvlc_video_get_spu: fn(*mut Player) -> c_int;
    libvlc_video_set_spu: fn(*mut Player, c_int) -> c_int;
    libvlc_video_get_spu_delay: fn(*mut Player) -> i64;
    libvlc_video_set_spu_delay: fn(*mut Player, i64) -> c_int;
    libvlc_track_description_list_release: fn(*mut TrackDescription);
    libvlc_video_set_aspect_ratio: fn(*mut Player, *const c_char);
    libvlc_video_set_crop_geometry: fn(*mut Player, *const c_char);
    libvlc_video_set_deinterlace: fn(*mut Player, *const c_char);
    libvlc_video_take_snapshot: fn(*mut Player, c_uint, *const c_char, c_uint, c_uint) -> c_int;
    libvlc_video_set_key_input: fn(*mut Player, c_uint);
    libvlc_media_player_get_full_chapter_descriptions: fn(*mut Player, c_int, *mut *mut *mut ChapterDescription) -> c_int;
    libvlc_chapter_descriptions_release: fn(*mut *mut ChapterDescription, c_uint);
    libvlc_video_set_mouse_input: fn(*mut Player, c_uint);
    libvlc_media_player_get_full_title_descriptions: fn(*mut Player, *mut *mut *mut TitleDescription) -> c_int;
    libvlc_title_descriptions_release: fn(*mut *mut TitleDescription, c_uint);
    libvlc_media_player_get_title: fn(*mut Player) -> c_int;
    libvlc_media_player_set_title: fn(*mut Player, c_int);
    libvlc_media_player_get_chapter: fn(*mut Player) -> c_int;
    libvlc_media_player_set_chapter: fn(*mut Player, c_int);
    libvlc_media_player_navigate: fn(*mut Player, c_uint);
    libvlc_media_parse_with_options: fn(*mut Media, c_int, c_int) -> c_int;
    libvlc_media_get_parsed_status: fn(*mut Media) -> c_int;
    libvlc_media_get_duration: fn(*mut Media) -> i64;
    libvlc_media_tracks_get: fn(*mut Media, *mut *mut *mut MediaTrack) -> c_uint;
    libvlc_media_tracks_release: fn(*mut *mut MediaTrack, c_uint);
}

/// libvlc_media_option_t: an option the application chose, which libVLC
/// applies even if it's unsafe (like stream output). Options from playlist
/// files leave it out, so libVLC only applies the safe ones.
const OPTION_TRUSTED: c_uint = 0x2;
const OPTION_UNIQUE: c_uint = 0x100;

/// libVLC media player events we listen to (values from libvlc_events.h).
pub mod event {
    pub const MEDIA_CHANGED: i32 = 0x100;
    pub const OPENING: i32 = 0x102;
    pub const BUFFERING: i32 = 0x103;
    pub const PLAYING: i32 = 0x104;
    pub const PAUSED: i32 = 0x105;
    pub const STOPPED: i32 = 0x106;
    pub const END_REACHED: i32 = 0x109;
    pub const ERROR: i32 = 0x10a;
    pub const TIME_CHANGED: i32 = 0x10b;
    pub const SEEKABLE_CHANGED: i32 = 0x10d;
    pub const TITLE_CHANGED: i32 = 0x10f;
    pub const SNAPSHOT_TAKEN: i32 = 0x110;
    pub const LENGTH_CHANGED: i32 = 0x111;
    pub const VOUT: i32 = 0x112;
    pub const ES_ADDED: i32 = 0x114;
    pub const ES_DELETED: i32 = 0x115;
    pub const ES_SELECTED: i32 = 0x116;
    pub const MUTED: i32 = 0x119;
    pub const UNMUTED: i32 = 0x11a;
    pub const AUDIO_VOLUME: i32 = 0x11b;
    pub const CHAPTER_CHANGED: i32 = 0x11d;

    pub const ALL: &[i32] = &[
        MEDIA_CHANGED, OPENING, BUFFERING, PLAYING, PAUSED, STOPPED, END_REACHED, ERROR,
        TIME_CHANGED, SEEKABLE_CHANGED, TITLE_CHANGED, SNAPSHOT_TAKEN, LENGTH_CHANGED, VOUT, ES_ADDED,
        ES_DELETED, ES_SELECTED, MUTED, UNMUTED, AUDIO_VOLUME, CHAPTER_CHANGED,
    ];
}

/// A player event, as libVLC sent it.
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub kind: i32,
    /// The media it belongs to (see [`Vlc::generation`]).
    pub generation: u64,
    /// When libVLC sent it; events are handled in batches, a little later.
    pub at: Instant,
    /// How full the input cache was, 0–100.
    pub cache: f32,
    /// The new time in ms, for TIME_CHANGED; -1 otherwise.
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

#[derive(Clone, Debug, Default)]
pub struct StreamInfo {
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
}

/// A codec's fourcc as text, like "h264" or "mp4a".
fn fourcc(code: u32) -> String {
    code.to_le_bytes().iter().map(|&b| if b.is_ascii_graphic() { b as char } else { ' ' }).collect::<String>().trim().to_string()
}

#[derive(Clone, Debug, Serialize)]
pub struct Chapter {
    pub start: i64,
    pub duration: i64,
    pub name: String,
}

/// A title of the current media: a DVD or Blu-ray title, or the single
/// title of a normal file.
#[derive(Clone, Debug, Serialize)]
pub struct Title {
    pub duration: i64,
    pub name: String,
    /// A disc menu (navigated with the arrow keys and Enter).
    pub menu: bool,
}

/// Disc menu navigation (libvlc_navigate_mode_t).
#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Navigate {
    Activate = 0,
    Up = 1,
    Down = 2,
    Left = 3,
    Right = 4,
    Popup = 5,
}

/// Metadata libVLC reads from a file (tags, embedded artwork, duration).
#[derive(Clone, Debug, Default, PartialEq, Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MediaMeta {
    pub title: Option<String>,
    pub show: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub date: Option<String>,
    /// A local file holding the artwork (libVLC extracts embedded covers to its cache).
    pub artwork: Option<String>,
    /// Milliseconds, 0 when unknown.
    pub duration: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Track {
    pub id: i32,
    pub name: String,
}

/// Functions from libvlccore for changing VLC's configuration at runtime.
/// New inputs and video outputs read their options from it, so changes
/// apply to the next video without restarting libVLC.
struct CoreApi {
    put_int: unsafe extern "C" fn(*mut c_void, *const c_char, i64),
    put_psz: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char),
}

/// A value for [`Vlc::set_config`].
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigValue {
    Int(i64),
    Bool(bool),
    Str(String),
}

/// Percentage of the input cache filled, from the last Buffering event.
/// Only the main player subscribes to events, so one value is enough.
static BUFFERING: AtomicU32 = AtomicU32::new(0);

/// Bumped whenever the main player gets new media, and sent with each event,
/// so a late event from the previous media can be told apart. Bumped once
/// the previous media has stopped: libVLC sends events from the input's
/// thread, which stopping joins, so nothing from it can carry the new value.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// libVLC's audio output module on this platform.
const AUDIO_MODULE: &str = if cfg!(target_os = "macos") { "auhal" } else if cfg!(windows) { "mmdevice" } else { "pulse" };

pub struct Vlc {
    api: Api,
    core: CoreApi,
    inst: *mut Inst,
    mp: *mut Player,
    /// Hardware decoding turned off; see [`Vlc::play_media`].
    software_decoding: AtomicBool,
    pub version: String,
}

// libVLC's media player API is thread-safe.
unsafe impl Send for Vlc {}
unsafe impl Sync for Vlc {}

/// Why libVLC couldn't start. `stage` is safe to send with analytics; the
/// message names paths, so it stays on the machine.
pub struct LoadError {
    /// `not_found`, `load` (a library didn't load), `symbol`, `version`
    /// (not libVLC 3), `init` (libvlc_new failed) or `player`.
    pub stage: &'static str,
    pub message: String,
}

impl LoadError {
    fn new(stage: &'static str, message: String) -> Self {
        Self { stage, message }
    }
}

/// Directories where we look for libVLC: `lib/`, `plugins/` and `share/`,
/// or on Windows VLC's own layout (the DLLs, `plugins/` and `lua/` together).
///
/// On Windows the resource directory can come as an extended-length
/// `\\?\C:\...` path. libVLC finds its plugins from the path libvlccore.dll
/// was loaded from, and Windows 10's msvcrt can't stat files under that
/// prefix, so libvlc_new finds no modules and fails. Paths are loaded in
/// their ordinary form.
fn candidate_dirs(resource_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("REVLC_VLC_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(res) = resource_dir {
        dirs.push(res.join("vlc"));
    }
    #[cfg(target_os = "macos")]
    dirs.push(PathBuf::from("/Applications/VLC.app/Contents/MacOS"));
    dirs.iter().map(|d| dunce::simplified(d).to_path_buf()).collect()
}

/// Where the libVLC libraries are inside a libVLC directory. On Windows,
/// libVLC finds its plugins and data next to libvlccore.dll: it reads
/// `VLC_PLUGIN_PATH` from the C runtime's copy of the environment, which
/// doesn't see variables we set after startup.
fn lib_dir(dir: &Path) -> PathBuf {
    if cfg!(windows) { dir.to_path_buf() } else { dir.join("lib") }
}

fn lib_names() -> (&'static str, &'static str) {
    if cfg!(target_os = "macos") {
        ("libvlccore.dylib", "libvlc.dylib")
    } else if cfg!(windows) {
        ("libvlccore.dll", "libvlc.dll")
    } else {
        ("libvlccore.so.9", "libvlc.so.5")
    }
}

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

/// A local path for libvlc_media_new_path. On Windows, libVLC only
/// recognises absolute paths written with backslashes.
fn path_cstr(path: &str) -> CString {
    if cfg!(windows) { cstr(&path.replace('/', "\\")) } else { cstr(path) }
}

fn opt_cstr(s: Option<&str>) -> Option<CString> {
    s.filter(|s| !s.is_empty()).map(cstr)
}

impl Vlc {
    /// `extra_args` are passed to libVLC as command-line options. `config`
    /// is applied before the media player is created, because the player
    /// copies some options (title display, deinterlacing) when it's made.
    pub fn load(resource_dir: Option<&Path>, extra_args: &[String], config: &[(&str, ConfigValue)]) -> Result<Self, LoadError> {
        let dirs = candidate_dirs(resource_dir);
        let dir = dirs
            .iter()
            .find(|d| lib_dir(d).join(lib_names().1).exists())
            .ok_or_else(|| {
                LoadError::new(
                    "not_found",
                    format!(
                        "libVLC 3 was not found. Install VLC 3 from videolan.org or set REVLC_VLC_DIR. Looked in: {}",
                        dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
                    ),
                )
            })?;

        // SAFETY: called once during startup, before libVLC starts its threads.
        unsafe {
            std::env::set_var("VLC_PLUGIN_PATH", dir.join("plugins"));
            std::env::set_var("VLC_DATA_PATH", dir.join("share"));
        }

        let (core_name, lib_name) = lib_names();
        let load = |name: &str| -> Result<&'static Library, LoadError> {
            let path = lib_dir(dir).join(name);
            // SAFETY: loading libVLC runs its initialisers, which are safe to run.
            let lib = unsafe { Library::new(&path) }
                .map_err(|e| LoadError::new("load", format!("Couldn't load {}: {e}", path.display())))?;
            Ok(Box::leak(Box::new(lib)))
        };
        // libvlc.dylib refers to libvlccore through @rpath, so load the core first.
        let core_lib = load(core_name)?;
        let lib = load(lib_name)?;
        let missing = |e: libloading::Error| LoadError::new("symbol", format!("libVLC is missing a symbol: {e}"));
        let api = unsafe { Api::load(lib) }.map_err(missing)?;
        let core = unsafe {
            CoreApi {
                put_int: *core_lib.get(b"config_PutInt\0").map_err(missing)?,
                put_psz: *core_lib.get(b"config_PutPsz\0").map_err(missing)?,
            }
        };

        // The "macosx" output draws subtitles as an overlay. The default
        // layer-based output tries to blend them into hardware-decoded
        // frames, which fails for 10-bit video, so subtitles never show.
        let vout: &[&str] = if cfg!(target_os = "macos") { &["--vout=macosx"] } else { &[] };
        let args: Vec<CString> = ["--no-snapshot-preview"]
            .iter()
            .chain(vout)
            .map(|a| a.to_string())
            .chain(extra_args.iter().cloned())
            .map(|a| cstr(&a))
            .collect();
        let argv: Vec<*const c_char> = args.iter().map(|a| a.as_ptr()).collect();
        let version = unsafe { CStr::from_ptr((api.libvlc_get_version)()) }.to_string_lossy().into_owned();
        // `put_config` relies on how libVLC 3 lays out its instance.
        if !version.starts_with("3.") {
            return Err(LoadError::new("version", format!("ReVLC needs libVLC 3, but {} has libVLC {version}", dir.display())));
        }
        unsafe {
            let inst = (api.libvlc_new)(argv.len() as c_int, argv.as_ptr());
            if inst.is_null() {
                return Err(LoadError::new(
                    "init",
                    format!("libVLC loaded from {} but couldn't start: {}", dir.display(), errmsg(&api)),
                ));
            }
            for (name, value) in config {
                put_config(&core, inst, name, value);
            }
            // Before the player makes its audio output, which names itself
            // after this: otherwise the volume mixer lists "VLC media player".
            // Streams keep VLC's own HTTP user agent, which some servers expect.
            let vlc_version = version.split_whitespace().next().unwrap_or("3");
            let (name, agent) = (cstr("ReVLC"), cstr(&format!("VLC/{vlc_version} LibVLC/{vlc_version}")));
            (api.libvlc_set_user_agent)(inst, name.as_ptr(), agent.as_ptr());
            let (id, ver, icon) = (cstr("app.revlc"), cstr(env!("CARGO_PKG_VERSION")), cstr("revlc"));
            (api.libvlc_set_app_id)(inst, id.as_ptr(), ver.as_ptr(), icon.as_ptr());
            let mp = (api.libvlc_media_player_new)(inst);
            if mp.is_null() {
                return Err(LoadError::new("player", format!("libVLC couldn't create a media player: {}", errmsg(&api))));
            }
            // The web UI handles all keyboard and mouse input.
            (api.libvlc_video_set_key_input)(mp, 0);
            (api.libvlc_video_set_mouse_input)(mp, 0);
            Ok(Self { api, core, inst, mp, software_decoding: AtomicBool::new(false), version })
        }
    }

    /// Audio outputs as (id, name). The id "" is the system default.
    pub fn audio_devices(&self) -> Vec<(String, String)> {
        let module = cstr(AUDIO_MODULE);
        let mut out = Vec::new();
        unsafe {
            let list = (self.api.libvlc_audio_output_device_list_get)(self.inst, module.as_ptr());
            let mut node = list;
            while !node.is_null() {
                let s = |p: *mut c_char| if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() };
                out.push((s((*node).device), s((*node).description)));
                node = (*node).next;
            }
            if !list.is_null() {
                (self.api.libvlc_audio_output_device_list_release)(list);
            }
        }
        out
    }

    /// Switches the audio output; "" means the system default.
    pub fn set_audio_device(&self, id: &str) {
        let c = cstr(id);
        unsafe { (self.api.libvlc_audio_output_device_set)(self.mp, std::ptr::null(), c.as_ptr()) }
    }

    /// Changes a VLC option (same names as the command line, without "--").
    /// Takes effect for the next video; reopen the current one to apply now.
    pub fn set_config(&self, name: &str, value: &ConfigValue) {
        put_config(&self.core, self.inst, name, value);
    }

    /// Turns hardware decoding on or off, from the next video.
    pub fn set_hardware_decoding(&self, on: bool) {
        self.software_decoding.store(!on, Ordering::Relaxed);
    }

    /// Shows the media title on the video when playback starts. `position`
    /// uses VLC's codes (0 center, 1 left, 2 right, 4 top, 8 bottom, sums for corners).
    pub fn set_title_display(&self, show: bool, position: i64, timeout_ms: i64) {
        let pos = match (show, position) {
            (false, _) => -1,
            (_, 1) => 1,
            (_, 2) => 2,
            (_, 4) => 3,
            (_, 5) => 4,
            (_, 6) => 5,
            (_, 8) => 6,
            (_, 9) => 7,
            (_, 10) => 8,
            _ => 0,
        };
        unsafe { (self.api.libvlc_media_player_set_video_title_display)(self.mp, pos, timeout_ms.max(0) as c_uint) }
    }

    /// Sends every player event to `tx`, with the media generation it
    /// belongs to (see [`Vlc::generation`]). The callback runs on a libVLC
    /// thread, so it must not call back into libVLC.
    pub fn subscribe(&self, tx: Sender<Event>) {
        unsafe extern "C" fn on_event(ev: *const c_void, data: *mut c_void) {
            unsafe {
                let ty = *(ev as *const c_int);
                // libvlc_event_t: int type, void *p_obj, then the union, whose
                // first member is `float new_cache` for Buffering and
                // `libvlc_time_t new_time` for TimeChanged.
                let union = (ev as *const u8).add(2 * size_of::<*const c_void>());
                if ty == event::BUFFERING {
                    BUFFERING.store((*(union as *const c_float)).to_bits(), Ordering::Relaxed);
                }
                let tx = &*(data as *const Sender<Event>);
                let _ = tx.send(Event {
                    kind: ty,
                    generation: GENERATION.load(Ordering::Acquire),
                    at: Instant::now(),
                    cache: f32::from_bits(BUFFERING.load(Ordering::Relaxed)),
                    time: if ty == event::TIME_CHANGED { *(union as *const i64) } else { -1 },
                });
            }
        }
        let data = Box::into_raw(Box::new(tx)) as *mut c_void;
        unsafe {
            let em = (self.api.libvlc_media_player_event_manager)(self.mp);
            for &ty in event::ALL {
                (self.api.libvlc_event_attach)(em, ty, on_event, data);
            }
        }
    }

    /// Sets the native view libVLC draws into (an `NSView*` on macOS).
    #[cfg(target_os = "macos")]
    pub fn set_nsview(&self, view: *mut c_void) {
        unsafe { (self.api.libvlc_media_player_set_nsobject)(self.mp, view) }
    }

    /// Sets the window libVLC draws into (an `HWND` on Windows).
    #[cfg(windows)]
    pub fn set_hwnd(&self, hwnd: *mut c_void) {
        unsafe { (self.api.libvlc_media_player_set_hwnd)(self.mp, hwnd) }
    }

    /// Identifies the media the player has: changes each time it opens one.
    pub fn generation(&self) -> u64 {
        GENERATION.load(Ordering::Acquire)
    }

    /// Opens a local path or a URL and starts playback. `options` are
    /// per-media libVLC options such as ":network-caching=1000"; unless
    /// `trusted`, libVLC ignores the unsafe ones (use for playlist files).
    pub fn open(&self, uri: &str, options: &[String], trusted: bool) -> Result<(), String> {
        BUFFERING.store(0, Ordering::Relaxed);
        let c = if uri.contains("://") { cstr(uri) } else { path_cstr(uri) };
        unsafe {
            let media = if uri.contains("://") {
                (self.api.libvlc_media_new_location)(self.inst, c.as_ptr())
            } else {
                (self.api.libvlc_media_new_path)(self.inst, c.as_ptr())
            };
            if media.is_null() {
                return Err(format!("could not open {uri}: {}", errmsg(&self.api)));
            }
            let flags = if trusted { OPTION_TRUSTED | OPTION_UNIQUE } else { OPTION_UNIQUE };
            for opt in options {
                let o = cstr(opt);
                (self.api.libvlc_media_add_option_flag)(media, o.as_ptr(), flags);
            }
            let result = self.play_media(media);
            (self.api.libvlc_media_release)(media);
            if result != 0 {
                return Err(format!("could not play {uri}: {}", errmsg(&self.api)));
            }
        }
        Ok(())
    }

    unsafe fn play_media(&self, media: *mut Media) -> c_int {
        unsafe {
            // Attaching the video view gives the player its own "avcodec-hw",
            // which hides the configured one from decoders. An option on the
            // media comes before the player's.
            if self.software_decoding.load(Ordering::Relaxed) {
                let o = cstr(":avcodec-hw=none");
                (self.api.libvlc_media_add_option_flag)(media, o.as_ptr(), OPTION_TRUSTED | OPTION_UNIQUE);
            }
            (self.api.libvlc_media_player_set_media)(self.mp, media);
            GENERATION.fetch_add(1, Ordering::AcqRel);
            (self.api.libvlc_media_player_play)(self.mp)
        }
    }

    /// Calls `f` with the items the current media expanded into (each
    /// retained, so `f` must release them). Web pages handled by VLC's Lua
    /// scripts, and remote playlists, end without playing anything and
    /// leave the real streams as subitems.
    fn with_subitems<T>(&self, f: impl FnOnce(Vec<*mut Media>) -> T) -> T {
        unsafe {
            let mut subs = Vec::new();
            let media = (self.api.libvlc_media_player_get_media)(self.mp);
            if !media.is_null() {
                let list = (self.api.libvlc_media_subitems)(media);
                (self.api.libvlc_media_release)(media);
                if !list.is_null() {
                    (self.api.libvlc_media_list_lock)(list);
                    for i in 0..(self.api.libvlc_media_list_count)(list) {
                        let sub = (self.api.libvlc_media_list_item_at_index)(list, i);
                        if !sub.is_null() {
                            subs.push(sub);
                        }
                    }
                    (self.api.libvlc_media_list_unlock)(list);
                    (self.api.libvlc_media_list_release)(list);
                }
            }
            f(subs)
        }
    }

    /// The locations and titles of what the current media expanded into.
    pub fn subitems(&self) -> Vec<(String, Option<String>)> {
        self.with_subitems(|subs| {
            subs.into_iter()
                .map(|sub| unsafe {
                    let raw = (self.api.libvlc_media_get_mrl)(sub);
                    let mrl = if raw.is_null() { String::new() } else { CStr::from_ptr(raw).to_string_lossy().into_owned() };
                    if !raw.is_null() {
                        (self.api.libvlc_free)(raw as *mut c_void);
                    }
                    let raw = (self.api.libvlc_media_get_meta)(sub, 0);
                    let title = (!raw.is_null()).then(|| CStr::from_ptr(raw).to_string_lossy().trim().to_string());
                    if !raw.is_null() {
                        (self.api.libvlc_free)(raw as *mut c_void);
                    }
                    (self.api.libvlc_media_release)(sub);
                    (mrl, title.filter(|t| !t.is_empty()))
                })
                .filter(|(mrl, _)| !mrl.is_empty())
                .collect()
        })
    }

    /// Plays the single item the current media expanded into, as is (with
    /// any options the script gave it). Returns whether there was one.
    pub fn play_subitem(&self) -> bool {
        self.with_subitems(|subs| unsafe {
            let result = match subs.as_slice() {
                [sub] => {
                    BUFFERING.store(0, Ordering::Relaxed);
                    self.play_media(*sub) == 0
                }
                _ => false,
            };
            for sub in subs {
                (self.api.libvlc_media_release)(sub);
            }
            result
        })
    }

    pub fn set_pause(&self, pause: bool) {
        unsafe { (self.api.libvlc_media_player_set_pause)(self.mp, pause as c_int) }
    }

    pub fn stop(&self) {
        unsafe { (self.api.libvlc_media_player_stop)(self.mp) }
        // Events from what was playing no longer count.
        GENERATION.fetch_add(1, Ordering::AcqRel);
    }

    /// How full the input cache is, 0–100. Drops after opening and seeking,
    /// and reaches 100 once libVLC has enough data to play.
    pub fn buffering(&self) -> f32 {
        f32::from_bits(BUFFERING.load(Ordering::Relaxed))
    }

    pub fn state(&self) -> State {
        match unsafe { (self.api.libvlc_media_player_get_state)(self.mp) } {
            1 => State::Opening,
            2 => State::Buffering,
            3 => State::Playing,
            4 => State::Paused,
            5 => State::Stopped,
            6 => State::Ended,
            7 => State::Error,
            _ => State::Idle,
        }
    }

    /// Current time in milliseconds.
    pub fn time(&self) -> i64 {
        unsafe { (self.api.libvlc_media_player_get_time)(self.mp) }.max(0)
    }

    pub fn set_time(&self, ms: i64) {
        unsafe { (self.api.libvlc_media_player_set_time)(self.mp, ms.max(0)) }
    }

    /// Duration in milliseconds (0 when unknown, e.g. live streams).
    pub fn length(&self) -> i64 {
        unsafe { (self.api.libvlc_media_player_get_length)(self.mp) }.max(0)
    }

    pub fn seekable(&self) -> bool {
        unsafe { (self.api.libvlc_media_player_is_seekable)(self.mp) != 0 }
    }

    pub fn has_video(&self) -> bool {
        unsafe { (self.api.libvlc_media_player_has_vout)(self.mp) > 0 }
    }

    /// The codecs, size and frame rate of what's playing.
    pub fn stream_info(&self) -> StreamInfo {
        let mut info = StreamInfo::default();
        unsafe {
            let media = (self.api.libvlc_media_player_get_media)(self.mp);
            if media.is_null() {
                return info;
            }
            let mut tracks: *mut *mut MediaTrack = std::ptr::null_mut();
            let count = (self.api.libvlc_media_tracks_get)(media, &mut tracks);
            for i in 0..count as usize {
                let track = &**tracks.add(i);
                match track.kind {
                    0 if info.audio_codec.is_none() => info.audio_codec = Some(fourcc(track.codec)),
                    1 if info.video_codec.is_none() => {
                        info.video_codec = Some(fourcc(track.codec));
                        if !track.details.is_null() {
                            let video = &*(track.details as *const VideoTrack);
                            info.width = video.width;
                            info.height = video.height;
                            if video.frame_rate_den > 0 {
                                info.fps = video.frame_rate_num as f32 / video.frame_rate_den as f32;
                            }
                        }
                    }
                    _ => {}
                }
            }
            if count > 0 {
                (self.api.libvlc_media_tracks_release)(tracks, count);
            }
            (self.api.libvlc_media_release)(media);
        }
        info
    }

    pub fn rate(&self) -> f32 {
        unsafe { (self.api.libvlc_media_player_get_rate)(self.mp) }
    }

    pub fn set_rate(&self, rate: f32) {
        unsafe { (self.api.libvlc_media_player_set_rate)(self.mp, rate.clamp(0.25, 4.0)) };
    }

    pub fn next_frame(&self) {
        unsafe { (self.api.libvlc_media_player_next_frame)(self.mp) }
    }

    /// Volume in percent (0–200).
    pub fn volume(&self) -> i32 {
        unsafe { (self.api.libvlc_audio_get_volume)(self.mp) }
    }

    pub fn set_volume(&self, volume: i32) {
        unsafe { (self.api.libvlc_audio_set_volume)(self.mp, volume.clamp(0, 200)) };
    }

    pub fn muted(&self) -> bool {
        unsafe { (self.api.libvlc_audio_get_mute)(self.mp) == 1 }
    }

    pub fn set_muted(&self, muted: bool) {
        unsafe { (self.api.libvlc_audio_set_mute)(self.mp, muted as c_int) }
    }

    fn tracks(&self, list: *mut TrackDescription) -> Vec<Track> {
        let mut tracks = Vec::new();
        let mut node = list;
        unsafe {
            while !node.is_null() {
                let name = if (*node).name.is_null() {
                    format!("Track {}", (*node).id)
                } else {
                    CStr::from_ptr((*node).name).to_string_lossy().into_owned()
                };
                tracks.push(Track { id: (*node).id, name });
                node = (*node).next;
            }
            if !list.is_null() {
                (self.api.libvlc_track_description_list_release)(list);
            }
        }
        tracks
    }

    pub fn audio_tracks(&self) -> Vec<Track> {
        self.tracks(unsafe { (self.api.libvlc_audio_get_track_description)(self.mp) })
    }

    pub fn audio_track(&self) -> i32 {
        unsafe { (self.api.libvlc_audio_get_track)(self.mp) }
    }

    pub fn set_audio_track(&self, id: i32) {
        unsafe { (self.api.libvlc_audio_set_track)(self.mp, id) };
    }

    pub fn subtitle_tracks(&self) -> Vec<Track> {
        self.tracks(unsafe { (self.api.libvlc_video_get_spu_description)(self.mp) })
    }

    pub fn subtitle_track(&self) -> i32 {
        unsafe { (self.api.libvlc_video_get_spu)(self.mp) }
    }

    pub fn set_subtitle_track(&self, id: i32) {
        unsafe { (self.api.libvlc_video_set_spu)(self.mp, id) };
    }

    /// Loads an external subtitle file or URL, selecting it if `select`.
    pub fn add_subtitle(&self, uri: &str, select: bool) -> Result<(), String> {
        let c = cstr(uri);
        match unsafe { (self.api.libvlc_media_player_add_slave)(self.mp, 0, c.as_ptr(), select) } {
            0 => Ok(()),
            _ => Err(format!("could not load subtitles from {uri}")),
        }
    }

    /// Audio delay in milliseconds.
    pub fn audio_delay(&self) -> i64 {
        (unsafe { (self.api.libvlc_audio_get_delay)(self.mp) }) / 1000
    }

    pub fn set_audio_delay(&self, ms: i64) {
        unsafe { (self.api.libvlc_audio_set_delay)(self.mp, ms * 1000) };
    }

    /// Subtitle delay in milliseconds.
    pub fn subtitle_delay(&self) -> i64 {
        (unsafe { (self.api.libvlc_video_get_spu_delay)(self.mp) }) / 1000
    }

    pub fn set_subtitle_delay(&self, ms: i64) {
        unsafe { (self.api.libvlc_video_set_spu_delay)(self.mp, ms * 1000) };
    }

    /// Forces an aspect ratio such as "16:9"; `None` restores the default.
    pub fn set_aspect_ratio(&self, ratio: Option<&str>) {
        let c = opt_cstr(ratio);
        let ptr = c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
        unsafe { (self.api.libvlc_video_set_aspect_ratio)(self.mp, ptr) }
    }

    /// Crops to a ratio ("16:9") or geometry ("WxH+X+Y"); `None` removes the crop.
    pub fn set_crop(&self, geometry: Option<&str>) {
        let c = opt_cstr(geometry);
        let ptr = c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
        unsafe { (self.api.libvlc_video_set_crop_geometry)(self.mp, ptr) }
    }

    /// Deinterlace mode such as "yadif"; `None` turns it off.
    pub fn set_deinterlace(&self, mode: Option<&str>) {
        let c = opt_cstr(mode);
        let ptr = c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
        unsafe { (self.api.libvlc_video_set_deinterlace)(self.mp, ptr) }
    }

    /// Saves a snapshot of the current frame. `path` is a directory (VLC
    /// picks the name) or a file; `width` 0 keeps the video's size.
    pub fn snapshot(&self, path: &Path, width: u32) -> Result<(), String> {
        let c = cstr(&path.to_string_lossy());
        match unsafe { (self.api.libvlc_video_take_snapshot)(self.mp, 0, c.as_ptr(), width, 0) } {
            0 => Ok(()),
            _ => Err("could not take a snapshot".into()),
        }
    }

    /// Chapters of the current title (empty when the media has none).
    pub fn chapters(&self) -> Vec<Chapter> {
        let mut list: *mut *mut ChapterDescription = std::ptr::null_mut();
        unsafe {
            let n = (self.api.libvlc_media_player_get_full_chapter_descriptions)(self.mp, -1, &mut list);
            if n <= 0 || list.is_null() {
                return Vec::new();
            }
            let chapters = (0..n as usize)
                .map(|i| {
                    let c = &**list.add(i);
                    let name = if c.name.is_null() {
                        format!("Chapter {}", i + 1)
                    } else {
                        CStr::from_ptr(c.name).to_string_lossy().into_owned()
                    };
                    Chapter { start: c.time_offset, duration: c.duration, name }
                })
                .collect();
            (self.api.libvlc_chapter_descriptions_release)(list, n as c_uint);
            chapters
        }
    }

    /// Titles of the current media (DVD and Blu-ray titles; one for files).
    pub fn titles(&self) -> Vec<Title> {
        let mut list: *mut *mut TitleDescription = std::ptr::null_mut();
        unsafe {
            let n = (self.api.libvlc_media_player_get_full_title_descriptions)(self.mp, &mut list);
            if n <= 0 || list.is_null() {
                return Vec::new();
            }
            let titles = (0..n as usize)
                .map(|i| {
                    let t = &**list.add(i);
                    // libvlc_title_menu; 0x02 is libvlc_title_interactive.
                    let menu = t.flags & 0x01 != 0;
                    let name = if t.name.is_null() {
                        if menu { "Menu".to_string() } else { format!("Title {}", i + 1) }
                    } else {
                        CStr::from_ptr(t.name).to_string_lossy().into_owned()
                    };
                    Title { duration: t.duration, name, menu }
                })
                .collect();
            (self.api.libvlc_title_descriptions_release)(list, n as c_uint);
            titles
        }
    }

    /// Index of the current title, or -1.
    pub fn title(&self) -> i32 {
        unsafe { (self.api.libvlc_media_player_get_title)(self.mp) }
    }

    pub fn set_title(&self, index: i32) {
        unsafe { (self.api.libvlc_media_player_set_title)(self.mp, index) }
    }

    /// Index of the current chapter, or -1.
    pub fn chapter(&self) -> i32 {
        unsafe { (self.api.libvlc_media_player_get_chapter)(self.mp) }
    }

    pub fn set_chapter(&self, index: i32) {
        unsafe { (self.api.libvlc_media_player_set_chapter)(self.mp, index) }
    }

    /// Moves around a DVD or Blu-ray menu.
    pub fn navigate(&self, action: Navigate) {
        unsafe { (self.api.libvlc_media_player_navigate)(self.mp, action as c_uint) }
    }

    /// Reads a local file's tags, embedded artwork and duration. Blocks for
    /// up to `timeout_ms`; doesn't touch playback.
    pub fn parse(&self, path: &str, timeout_ms: i32) -> Result<MediaMeta, String> {
        use std::time::{Duration, Instant};
        // libvlc_meta_t values.
        const TITLE: c_int = 0;
        const DATE: c_int = 8;
        const ARTWORK_URL: c_int = 15;
        const SEASON: c_int = 19;
        const EPISODE: c_int = 20;
        const SHOW_NAME: c_int = 21;
        // libvlc_media_fetch_local (parsing local files is implied).
        const FLAGS: c_int = 0x02;
        const PARSED_DONE: c_int = 4;

        let c = path_cstr(path);
        unsafe {
            let media = (self.api.libvlc_media_new_path)(self.inst, c.as_ptr());
            if media.is_null() {
                return Err(format!("could not open {path}"));
            }
            if (self.api.libvlc_media_parse_with_options)(media, FLAGS, timeout_ms) != 0 {
                (self.api.libvlc_media_release)(media);
                return Err(format!("could not parse {path}"));
            }
            let start = Instant::now();
            let limit = Duration::from_millis(timeout_ms as u64 + 500);
            let mut status = 0;
            while start.elapsed() < limit {
                status = (self.api.libvlc_media_get_parsed_status)(media);
                if status != 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let meta = |ty: c_int| {
                let raw = (self.api.libvlc_media_get_meta)(media, ty);
                if raw.is_null() {
                    return None;
                }
                let s = CStr::from_ptr(raw).to_string_lossy().trim().to_string();
                (self.api.libvlc_free)(raw as *mut c_void);
                Some(s).filter(|s| !s.is_empty())
            };
            let number = |ty: c_int| meta(ty).and_then(|s| s.parse::<u32>().ok());
            let result = if status == PARSED_DONE {
                let artwork = meta(ARTWORK_URL)
                    .and_then(|url| crate::playlist_file::file_uri_path(&url))
                    .filter(|p| Path::new(p).is_file());
                Ok(MediaMeta {
                    title: meta(TITLE),
                    show: meta(SHOW_NAME),
                    season: number(SEASON),
                    episode: number(EPISODE),
                    date: meta(DATE),
                    artwork,
                    duration: (self.api.libvlc_media_get_duration)(media).max(0),
                })
            } else {
                Err(format!("could not parse {path}"))
            };
            (self.api.libvlc_media_release)(media);
            result
        }
    }

    /// The title reported by the current media (e.g. a stream's title).
    pub fn media_title(&self) -> Option<String> {
        unsafe {
            let media = (self.api.libvlc_media_player_get_media)(self.mp);
            if media.is_null() {
                return None;
            }
            let raw = (self.api.libvlc_media_get_meta)(media, 0);
            (self.api.libvlc_media_release)(media);
            if raw.is_null() {
                return None;
            }
            let title = CStr::from_ptr(raw).to_string_lossy().into_owned();
            (self.api.libvlc_free)(raw as *mut c_void);
            Some(title).filter(|t| !t.trim().is_empty())
        }
    }
}

fn put_config(core: &CoreApi, inst: *mut Inst, name: &str, value: &ConfigValue) {
    let n = cstr(name);
    unsafe {
        // libvlc_instance_t starts with a pointer to libVLC's root object.
        // True of every libVLC 3, which `Vlc::load` insists on.
        let obj = *(inst as *mut *mut c_void);
        match value {
            ConfigValue::Int(v) => (core.put_int)(obj, n.as_ptr(), *v),
            ConfigValue::Bool(v) => (core.put_int)(obj, n.as_ptr(), *v as i64),
            ConfigValue::Str(v) => {
                let c = cstr(v);
                (core.put_psz)(obj, n.as_ptr(), c.as_ptr())
            }
        }
    }
}

/// Zeroed memory aligned the way libVLC wants picture planes.
struct FrameBuf {
    ptr: *mut u8,
    layout: std::alloc::Layout,
}

impl FrameBuf {
    const ALIGN: usize = 32;

    fn new(len: usize) -> Option<Self> {
        let layout = std::alloc::Layout::from_size_align(len, Self::ALIGN).ok().filter(|l| l.size() > 0)?;
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        // Lazily: a FrameBuf made from a null pointer would free it when dropped.
        (!ptr.is_null()).then(|| Self { ptr, layout })
    }

    fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.layout.size()) }
    }
}

impl Drop for FrameBuf {
    fn drop(&mut self) {
        unsafe { std::alloc::dealloc(self.ptr, self.layout) }
    }
}

/// A frame captured by [`Vlc::thumbnail`], in VLC's RV32 (BGRX) layout.
/// The video output thread fills in `frame` while the thumbnailing thread
/// only watches `frames`, so the frame sits in a cell of its own.
struct Grab {
    frame: std::cell::UnsafeCell<GrabFrame>,
    frames: std::sync::atomic::AtomicU32,
}

struct GrabFrame {
    buf: Option<FrameBuf>,
    width: u32,
    height: u32,
    target_width: u32,
}

unsafe extern "C" fn grab_format(
    opaque: *mut *mut c_void,
    chroma: *mut c_char,
    width: *mut c_uint,
    height: *mut c_uint,
    pitches: *mut c_uint,
    lines: *mut c_uint,
) -> c_uint {
    unsafe {
        // Only the video output thread touches the frame.
        let g = &mut *(*(*opaque as *const Grab)).frame.get();
        if *width == 0 || *height == 0 {
            return 0;
        }
        let w = g.target_width;
        // The frame's shape comes from the file; nothing real is taller than 4:1.
        let h = (*height as u64 * w as u64 / *width as u64).clamp(2, w as u64 * 4) as u32 & !1;
        let Some(buf) = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(4)).and_then(FrameBuf::new) else {
            return 0;
        };
        std::ptr::copy_nonoverlapping(b"RV32".as_ptr() as *const c_char, chroma, 4);
        (*width, *height, *pitches, *lines) = (w, h, w * 4, h);
        g.buf = Some(buf);
        (g.width, g.height) = (w, h);
        1
    }
}

unsafe extern "C" fn grab_lock(opaque: *mut c_void, planes: *mut *mut c_void) -> *mut c_void {
    unsafe {
        let g = &*(*(opaque as *const Grab)).frame.get();
        *planes = g.buf.as_ref().map_or(std::ptr::null_mut(), |b| b.ptr as *mut c_void);
    }
    std::ptr::null_mut()
}

unsafe extern "C" fn grab_display(opaque: *mut c_void, _picture: *mut c_void) {
    let g = unsafe { &*(opaque as *const Grab) };
    g.frames.fetch_add(1, std::sync::atomic::Ordering::Release);
}

impl Vlc {
    /// Decodes one frame about `fraction` of the way into `path` and saves it
    /// as a JPEG `width` pixels wide. Uses a separate, silent player so it
    /// never disturbs playback. Playback starts at the keyframe nearest the
    /// target rather than seeking there exactly, which would decode every
    /// frame since the last keyframe: seconds of 4K or 8K AV1. Decoders get
    /// few threads, since one frame doesn't need many.
    pub fn thumbnail(&self, path: &str, out: &Path, fraction: f64, width: u32) -> Result<(), String> {
        use std::sync::atomic::Ordering::Acquire;
        use std::time::{Duration, Instant};

        let length = self.parse(path, 3000).map(|m| m.duration).unwrap_or(0);
        let start = if length > 0 { ((length as f64 * fraction) as i64).clamp(0, (length - 1000).max(0)) } else { 0 };
        let grab = Box::into_raw(Box::new(Grab {
            frame: std::cell::UnsafeCell::new(GrabFrame { buf: None, width: 0, height: 0, target_width: width }),
            frames: Default::default(),
        }));
        let wait = |deadline: Duration, done: &dyn Fn() -> bool| {
            let begin = Instant::now();
            while !done() && begin.elapsed() < deadline {
                std::thread::sleep(Duration::from_millis(15));
            }
            done()
        };
        let api = &self.api;
        unsafe {
            let mp = (api.libvlc_media_player_new)(self.inst);
            let c = path_cstr(path);
            let media = (api.libvlc_media_new_path)(self.inst, c.as_ptr());
            if mp.is_null() || media.is_null() {
                if !mp.is_null() {
                    (api.libvlc_media_player_release)(mp);
                }
                if !media.is_null() {
                    (api.libvlc_media_release)(media);
                }
                drop(Box::from_raw(grab));
                return Err("could not create the thumbnail player".into());
            }
            for opt in [
                ":no-audio".to_string(),
                ":no-spu".to_string(),
                ":no-sub-autodetect-file".to_string(),
                ":avcodec-threads=2".to_string(),
                // dav1d's thread count; its tile setting doesn't add any.
                ":dav1d-thread-frames=2".to_string(),
                ":input-fast-seek".to_string(),
                format!(":start-time={:.3}", start as f64 / 1000.0),
            ] {
                let o = cstr(&opt);
                (api.libvlc_media_add_option_flag)(media, o.as_ptr(), OPTION_TRUSTED | OPTION_UNIQUE);
            }
            (api.libvlc_media_player_set_media)(mp, media);
            (api.libvlc_media_release)(media);
            (api.libvlc_video_set_callbacks)(mp, grab_lock, None, Some(grab_display), grab as *mut c_void);
            (api.libvlc_video_set_format_callbacks)(mp, grab_format, None);
            (api.libvlc_media_player_play)(mp);

            // A few frames in, past any grey first frame of an open GOP.
            let g = &*grab;
            let ok = wait(Duration::from_secs(10), &|| g.frames.load(Acquire) >= 3);
            (api.libvlc_media_player_stop)(mp);
            (api.libvlc_media_player_release)(mp);
            // Stopped and released: the video output is gone, so the frame is ours.
            let g = Box::from_raw(grab).frame.into_inner();
            let Some(buf) = g.buf.as_ref().filter(|_| ok && g.width > 0) else {
                return Err(format!("no frame decoded from {path}"));
            };
            let rgb: Vec<u8> = buf.as_slice().as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[1], p[0]]).collect();
            let img = image::RgbImage::from_raw(g.width, g.height, rgb).ok_or("bad frame size")?;
            img.save_with_format(out, image::ImageFormat::Jpeg).map_err(|e| e.to_string())
        }
    }
}

fn errmsg(api: &Api) -> String {
    unsafe {
        let msg = (api.libvlc_errmsg)();
        if msg.is_null() {
            "unknown error".into()
        } else {
            CStr::from_ptr(msg).to_string_lossy().into_owned()
        }
    }
}

