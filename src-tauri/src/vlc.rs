//! Thin wrapper around libVLC 3, loaded at runtime.
//!
//! Everything libVLC-specific lives here so the rest of the app can move to
//! libVLC 4 later without changes.

use std::ffi::{CStr, CString, c_char, c_float, c_int, c_uint, c_void};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use libloading::Library;
use serde::Serialize;

type Inst = c_void;
type Player = c_void;
type Media = c_void;
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
struct TrackDescription {
    id: c_int,
    name: *mut c_char,
    next: *mut TrackDescription,
}

macro_rules! api {
    ($($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        #[allow(non_snake_case)]
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
    libvlc_free: fn(*mut c_void);
    libvlc_media_new_path: fn(*mut Inst, *const c_char) -> *mut Media;
    libvlc_media_new_location: fn(*mut Inst, *const c_char) -> *mut Media;
    libvlc_media_release: fn(*mut Media);
    libvlc_media_add_option: fn(*mut Media, *const c_char);
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
    libvlc_media_player_get_length: fn(*mut Player) -> i64;
    libvlc_media_player_get_time: fn(*mut Player) -> i64;
    libvlc_media_player_set_time: fn(*mut Player, i64);
    libvlc_media_player_get_state: fn(*mut Player) -> c_int;
    libvlc_media_player_is_seekable: fn(*mut Player) -> c_int;
    libvlc_media_player_has_vout: fn(*mut Player) -> c_uint;
    libvlc_media_player_set_rate: fn(*mut Player, c_float) -> c_int;
    libvlc_media_player_get_rate: fn(*mut Player) -> c_float;
    libvlc_media_player_next_frame: fn(*mut Player);
    libvlc_media_player_add_slave: fn(*mut Player, c_int, *const c_char, bool) -> c_int;
    libvlc_audio_get_volume: fn(*mut Player) -> c_int;
    libvlc_audio_set_volume: fn(*mut Player, c_int) -> c_int;
    libvlc_audio_get_mute: fn(*mut Player) -> c_int;
    libvlc_audio_set_mute: fn(*mut Player, c_int);
    libvlc_audio_get_track_description: fn(*mut Player) -> *mut TrackDescription;
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
}

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
        TIME_CHANGED, SEEKABLE_CHANGED, SNAPSHOT_TAKEN, LENGTH_CHANGED, VOUT, ES_ADDED,
        ES_DELETED, ES_SELECTED, MUTED, UNMUTED, AUDIO_VOLUME, CHAPTER_CHANGED,
    ];
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

#[derive(Clone, Debug, Serialize)]
pub struct Chapter {
    pub start: i64,
    pub duration: i64,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Track {
    pub id: i32,
    pub name: String,
}

pub struct Vlc {
    api: Api,
    inst: *mut Inst,
    mp: *mut Player,
    pub version: String,
}

// libVLC's media player API is thread-safe.
unsafe impl Send for Vlc {}
unsafe impl Sync for Vlc {}

/// Directories where we look for libVLC (`lib/`, `plugins/`, `share/`).
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
    dirs
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

fn opt_cstr(s: Option<&str>) -> Option<CString> {
    s.filter(|s| !s.is_empty()).map(cstr)
}

impl Vlc {
    /// `extra_args` are passed to libVLC as command-line options.
    pub fn load(resource_dir: Option<&Path>, extra_args: &[String]) -> Result<Self, String> {
        let dirs = candidate_dirs(resource_dir);
        let dir = dirs
            .iter()
            .find(|d| d.join("lib").join(lib_names().1).exists())
            .ok_or_else(|| {
                format!(
                    "libVLC 3 was not found. Install VLC 3 from videolan.org or set REVLC_VLC_DIR. Looked in: {}",
                    dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
                )
            })?;

        // SAFETY: called once during startup, before libVLC starts its threads.
        unsafe {
            std::env::set_var("VLC_PLUGIN_PATH", dir.join("plugins"));
            std::env::set_var("VLC_DATA_PATH", dir.join("share"));
        }

        let (core_name, lib_name) = lib_names();
        let load = |name: &str| -> Result<&'static Library, String> {
            let path = dir.join("lib").join(name);
            // SAFETY: loading libVLC runs its initialisers, which are safe to run.
            let lib = unsafe { Library::new(&path) }.map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Box::leak(Box::new(lib)))
        };
        // libvlc.dylib refers to libvlccore through @rpath, so load the core first.
        load(core_name)?;
        let lib = load(lib_name)?;
        let api = unsafe { Api::load(lib) }.map_err(|e| format!("libVLC is missing a symbol: {e}"))?;

        // The "macosx" output draws subtitles as an overlay. The default
        // layer-based output tries to blend them into hardware-decoded
        // frames, which fails for 10-bit video, so subtitles never show.
        let args: Vec<CString> = ["--no-video-title-show", "--no-osd", "--no-snapshot-preview", "--snapshot-format=jpg", "--vout=macosx"]
            .iter()
            .map(|a| a.to_string())
            .chain(extra_args.iter().cloned())
            .map(|a| cstr(&a))
            .collect();
        let argv: Vec<*const c_char> = args.iter().map(|a| a.as_ptr()).collect();
        unsafe {
            let inst = (api.libvlc_new)(argv.len() as c_int, argv.as_ptr());
            if inst.is_null() {
                return Err(format!("libvlc_new failed: {}", errmsg(&api)));
            }
            let mp = (api.libvlc_media_player_new)(inst);
            if mp.is_null() {
                return Err(format!("could not create a media player: {}", errmsg(&api)));
            }
            // The web UI handles all keyboard and mouse input.
            (api.libvlc_video_set_key_input)(mp, 0);
            (api.libvlc_video_set_mouse_input)(mp, 0);
            let version = CStr::from_ptr((api.libvlc_get_version)()).to_string_lossy().into_owned();
            Ok(Self { api, inst, mp, version })
        }
    }

    /// Sends every player event type to `tx`. The callback runs on a libVLC
    /// thread, so it must not call back into libVLC.
    pub fn subscribe(&self, tx: Sender<i32>) {
        unsafe extern "C" fn on_event(ev: *const c_void, data: *mut c_void) {
            unsafe {
                let ty = *(ev as *const c_int);
                let tx = &*(data as *const Sender<i32>);
                let _ = tx.send(ty);
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

    /// Opens a local path or a URL and starts playback.
    pub fn open(&self, uri: &str) -> Result<(), String> {
        let c = cstr(uri);
        unsafe {
            let media = if uri.contains("://") {
                (self.api.libvlc_media_new_location)(self.inst, c.as_ptr())
            } else {
                (self.api.libvlc_media_new_path)(self.inst, c.as_ptr())
            };
            if media.is_null() {
                return Err(format!("could not open {uri}: {}", errmsg(&self.api)));
            }
            (self.api.libvlc_media_player_set_media)(self.mp, media);
            (self.api.libvlc_media_release)(media);
            if (self.api.libvlc_media_player_play)(self.mp) != 0 {
                return Err(format!("could not play {uri}: {}", errmsg(&self.api)));
            }
        }
        Ok(())
    }

    pub fn set_pause(&self, pause: bool) {
        unsafe { (self.api.libvlc_media_player_set_pause)(self.mp, pause as c_int) }
    }

    pub fn stop(&self) {
        unsafe { (self.api.libvlc_media_player_stop)(self.mp) }
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

    /// Loads an external subtitle file or URL and selects it.
    pub fn add_subtitle(&self, uri: &str) -> Result<(), String> {
        let c = cstr(uri);
        match unsafe { (self.api.libvlc_media_player_add_slave)(self.mp, 0, c.as_ptr(), true) } {
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

/// A frame captured by [`Vlc::thumbnail`], in VLC's RV32 (BGRX) layout.
struct Grab {
    buf: Vec<u8>,
    width: u32,
    height: u32,
    target_width: u32,
    frames: std::sync::atomic::AtomicU32,
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
        let g = &mut *(*opaque as *mut Grab);
        if *width == 0 || *height == 0 {
            return 0;
        }
        let w = g.target_width;
        let h = ((*height as u64 * w as u64 / *width as u64) as u32).max(2) & !1;
        std::ptr::copy_nonoverlapping(b"RV32".as_ptr() as *const c_char, chroma, 4);
        (*width, *height, *pitches, *lines) = (w, h, w * 4, h);
        g.buf = vec![0; (w * h * 4) as usize];
        (g.width, g.height) = (w, h);
        1
    }
}

unsafe extern "C" fn grab_lock(opaque: *mut c_void, planes: *mut *mut c_void) -> *mut c_void {
    unsafe {
        let g = &mut *(opaque as *mut Grab);
        *planes = g.buf.as_mut_ptr() as *mut c_void;
    }
    std::ptr::null_mut()
}

unsafe extern "C" fn grab_display(opaque: *mut c_void, _picture: *mut c_void) {
    let g = unsafe { &*(opaque as *const Grab) };
    g.frames.fetch_add(1, std::sync::atomic::Ordering::Release);
}

impl Vlc {
    /// Decodes one frame about `fraction` of the way into `path` and saves it
    /// as a JPEG `width` pixels wide. Uses a separate, silent, software-decoded
    /// player so it never disturbs playback.
    pub fn thumbnail(&self, path: &str, out: &Path, fraction: f64, width: u32) -> Result<(), String> {
        use std::sync::atomic::Ordering::Acquire;
        use std::time::{Duration, Instant};

        let grab = Box::into_raw(Box::new(Grab {
            buf: Vec::new(),
            width: 0,
            height: 0,
            target_width: width,
            frames: Default::default(),
        }));
        let wait = |deadline: Duration, done: &dyn Fn() -> bool| {
            let start = Instant::now();
            while !done() && start.elapsed() < deadline {
                std::thread::sleep(Duration::from_millis(15));
            }
            done()
        };
        let api = &self.api;
        unsafe {
            let mp = (api.libvlc_media_player_new)(self.inst);
            let c = cstr(path);
            let media = (api.libvlc_media_new_path)(self.inst, c.as_ptr());
            if mp.is_null() || media.is_null() {
                drop(Box::from_raw(grab));
                return Err("could not create the thumbnail player".into());
            }
            for opt in [":no-audio", ":no-spu", ":codec=avcodec", ":avcodec-hw=none", ":no-sub-autodetect-file"] {
                let o = cstr(opt);
                (api.libvlc_media_add_option)(media, o.as_ptr());
            }
            (api.libvlc_media_player_set_media)(mp, media);
            (api.libvlc_media_release)(media);
            (api.libvlc_video_set_callbacks)(mp, grab_lock, None, Some(grab_display), grab as *mut c_void);
            (api.libvlc_video_set_format_callbacks)(mp, grab_format, None);
            (api.libvlc_media_player_play)(mp);

            let g = &*grab;
            let started = wait(Duration::from_secs(5), &|| g.frames.load(Acquire) > 0);
            let mut ok = started;
            if started {
                let length = (api.libvlc_media_player_get_length)(mp);
                if length > 0 {
                    let target = ((length as f64 * fraction) as i64).clamp(0, length - 1000);
                    (api.libvlc_media_player_set_time)(mp, target);
                    let before = g.frames.load(Acquire);
                    // A few frames after the seek, so we skip any stale one.
                    ok = wait(Duration::from_secs(6), &|| {
                        g.frames.load(Acquire) >= before + 3 && (api.libvlc_media_player_get_time)(mp) >= target - 2000
                    });
                }
            }
            (api.libvlc_media_player_stop)(mp);
            (api.libvlc_media_player_release)(mp);
            let g = Box::from_raw(grab);
            if !ok || g.width == 0 {
                return Err(format!("no frame decoded from {path}"));
            }
            let rgb: Vec<u8> = g.buf.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]).collect();
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
