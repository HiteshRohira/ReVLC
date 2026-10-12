mod playlist;
#[cfg(target_os = "macos")]
mod surface_macos;
mod vlc;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, RunEvent};

use playlist::{Playlist, Repeat};
use vlc::{State, Track, Vlc, event};

struct Core {
    vlc: Vlc,
    playlist: Mutex<Playlist>,
    app: AppHandle,
    /// Set once the video view exists; opens before that are queued so
    /// libVLC never falls back to its own window.
    ready: AtomicBool,
    pending: Mutex<Vec<String>>,
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
    audio_delay: i64,
    subtitle_delay: i64,
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
    engine: Result<String, String>,
}

impl Core {
    fn playlist(&self) -> MutexGuard<'_, Playlist> {
        self.playlist.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn state(&self) -> PlayerState {
        let v = &self.vlc;
        PlayerState {
            state: v.state(),
            time: v.time(),
            length: v.length(),
            seekable: v.seekable(),
            has_video: v.has_video(),
            rate: v.rate(),
            volume: v.volume(),
            muted: v.muted(),
            audio_delay: v.audio_delay(),
            subtitle_delay: v.subtitle_delay(),
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
        let uri = {
            let mut playlist = self.playlist();
            let uri = playlist.items.get(index).ok_or("no such playlist item")?.uri.clone();
            playlist.current = Some(index);
            uri
        };
        self.emit_playlist();
        self.vlc.open(&uri)
    }

    /// Adds to the playlist, optionally starting the first new item.
    fn add(&self, uris: Vec<String>, play: bool) -> Result<(), String> {
        if uris.is_empty() {
            return Ok(());
        }
        if !self.ready.load(Ordering::Acquire) {
            self.pending.lock().unwrap_or_else(|e| e.into_inner()).extend(uris);
            return Ok(());
        }
        let first = self.playlist().add(&uris);
        self.emit_playlist();
        if play { self.play_index(first) } else { Ok(()) }
    }

    fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
        let pending = std::mem::take(&mut *self.pending.lock().unwrap_or_else(|e| e.into_inner()));
        if let Err(e) = self.add(pending, true) {
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

/// Turns libVLC events into UI events. Time updates are throttled so the
/// web view isn't woken more than a few times a second.
fn pump_events(core: Arc<Core>, rx: Receiver<i32>) {
    const TIME_INTERVAL: Duration = Duration::from_millis(250);
    let mut last_time_emit = Instant::now() - TIME_INTERVAL;
    while let Ok(first) = rx.recv() {
        // Let bursts of events (e.g. opening a file) arrive, then handle them together.
        thread::sleep(Duration::from_millis(20));
        let events: Vec<i32> = std::iter::once(first).chain(rx.try_iter()).collect();
        let has = |ty: i32| events.contains(&ty);

        if has(event::END_REACHED) {
            if let Err(e) = core.advance(true) {
                core.notice("error", e);
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
            if let Some(title) = core.vlc.media_title() {
                let mut playlist = core.playlist();
                let changed = match playlist.current.and_then(|i| playlist.items.get_mut(i)) {
                    Some(item) if item.title != title => {
                        item.title = title;
                        true
                    }
                    _ => false,
                };
                drop(playlist);
                if changed {
                    core.emit_playlist();
                }
            }
        }
        if has(event::SNAPSHOT_TAKEN) {
            core.notice("info", "Snapshot saved to Pictures");
        }
        if [event::PLAYING, event::VOUT, event::ES_ADDED, event::ES_DELETED, event::ES_SELECTED]
            .iter()
            .any(|&e| has(e))
        {
            core.emit_tracks();
        }

        let only_time = events.iter().all(|&e| e == event::TIME_CHANGED);
        if !only_time || last_time_emit.elapsed() >= TIME_INTERVAL {
            core.emit_state();
            last_time_emit = Instant::now();
        }
    }
}

type Res = Result<(), String>;

/// `file://` URI for an absolute path, percent-encoding everything but
/// unreserved characters and `/`.
fn file_uri(path: &str) -> String {
    let mut uri = String::from("file://");
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => uri.push(b as char),
            _ => uri.push_str(&format!("%{b:02X}")),
        }
    }
    uri
}

#[tauri::command]
fn snapshot(player: tauri::State<Player>) -> Snapshot {
    match player.core() {
        Ok(core) => Snapshot {
            state: Some(core.state()),
            tracks: Some(core.tracks()),
            playlist: Some(core.playlist().clone()),
            engine: Ok(format!("libVLC {}", core.vlc.version)),
        },
        Err(e) => Snapshot { state: None, tracks: None, playlist: None, engine: Err(e) },
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
    let core = player.core()?;
    core.vlc.stop();
    core.emit_state();
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
    let dir = app.path().picture_dir().map_err(|e| e.to_string())?;
    player.core()?.vlc.snapshot(&dir)
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
    core.playlist().clear();
    core.vlc.stop();
    core.emit_playlist();
    core.emit_state();
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

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let resource_dir = app.path().resource_dir().ok();
    let vlc = match Vlc::load(resource_dir.as_deref()) {
        Ok(vlc) => vlc,
        Err(e) => {
            eprintln!("revlc: {e}");
            app.manage(Player(Err(e)));
            return Ok(());
        }
    };
    let core = Arc::new(Core {
        vlc,
        playlist: Mutex::default(),
        app: app.handle().clone(),
        ready: AtomicBool::new(false),
        pending: Mutex::default(),
    });

    let (tx, rx) = mpsc::channel();
    core.vlc.subscribe(tx);
    let pump = core.clone();
    thread::Builder::new().name("vlc-events".into()).spawn(move || pump_events(pump, rx))?;

    // Files passed on the command line.
    let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();
    let _ = core.add(args, true);

    let window = app.get_webview_window("main").ok_or("main window missing")?;
    let attach = core.clone();
    window.with_webview(move |webview| {
        #[cfg(target_os = "macos")]
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
        #[cfg(not(target_os = "macos"))]
        let _ = webview;
        attach.mark_ready();
    })?;

    app.manage(Player(Ok(core)));
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(setup)
        .invoke_handler(tauri::generate_handler![
            snapshot,
            open,
            play_index,
            toggle_pause,
            stop,
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
        ])
        .build(tauri::generate_context!())
        .expect("error while building revlc");

    app.run(|handle, event| {
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
