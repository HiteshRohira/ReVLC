//! Self-updates. Checks revlc.vercel.app/latest.json (set in tauri.conf.json)
//! shortly after launch and then every few hours. With automatic updates on,
//! a new version is downloaded and installed in the background; either way
//! the UI asks the user, who decides when to restart.
//!
//! Runs here rather than in the web view, whose timers macOS throttles
//! while the window is hidden.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::Player;

const FIRST_CHECK: Duration = Duration::from_secs(10);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Idle,
    Checking,
    Available,
    Downloading,
    Ready,
    Error,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    status: Status,
    version: String,
    /// Download progress, 0–1, when the size is known.
    progress: Option<f64>,
    /// Whether the user started the download (background ones stay quiet).
    by_user: bool,
}

pub struct Updater {
    state: Mutex<UpdateState>,
    update: Mutex<Option<Update>>,
}

impl Default for Updater {
    fn default() -> Self {
        Self {
            state: Mutex::new(UpdateState { status: Status::Idle, version: String::new(), progress: None, by_user: false }),
            update: Mutex::default(),
        }
    }
}

impl Updater {
    fn set(&self, app: &AppHandle, change: impl FnOnce(&mut UpdateState)) {
        let state = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            change(&mut state);
            state.clone()
        };
        let _ = app.emit("update://state", state);
    }

    fn status(&self) -> Status {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).status
    }
}

fn auto_update(app: &AppHandle) -> bool {
    app.try_state::<Player>().and_then(|p| p.core().ok().map(|core| core.settings().auto_update)).unwrap_or(true)
}

/// Starts the periodic background check.
pub fn start(app: &AppHandle) {
    app.manage(Updater::default());
    // Development builds aren't installed apps.
    if cfg!(debug_assertions) {
        return;
    }
    let app = app.clone();
    let _ = std::thread::Builder::new().name("updater".into()).spawn(move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            tauri::async_runtime::block_on(check(&app, false));
            std::thread::sleep(CHECK_EVERY);
        }
    });
}

/// `manual` is a check the user asked for: it reports "up to date" and errors.
pub async fn check(app: &AppHandle, manual: bool) {
    let updater = app.state::<Updater>();
    if manual {
        let _ = app.emit("update://manual-check", ());
    }
    // Already checking, or installed and waiting for a restart.
    if matches!(updater.status(), Status::Checking | Status::Downloading | Status::Ready) {
        return;
    }
    updater.set(app, |s| s.status = Status::Checking);
    let result = match app.updater() {
        Ok(u) => u.check().await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    match result {
        Ok(Some(update)) => {
            let version = update.version.clone();
            *updater.update.lock().unwrap_or_else(|e| e.into_inner()) = Some(update);
            updater.set(app, |s| {
                s.status = Status::Available;
                s.version = version;
            });
            if auto_update(app) {
                install(app, false).await;
            }
        }
        Ok(None) => {
            updater.set(app, |s| s.status = Status::Idle);
            if manual {
                notice(app, "info", "ReVLC is up to date".into());
            }
        }
        Err(e) => {
            eprintln!("revlc: update check failed: {e}");
            updater.set(app, |s| s.status = Status::Error);
            if manual {
                notice(app, "error", format!("Could not check for updates: {e}"));
            }
        }
    }
}

/// Downloads and installs the pending update; it takes effect on restart.
pub async fn install(app: &AppHandle, by_user: bool) {
    let updater = app.state::<Updater>();
    let Some(update) = updater.update.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return;
    };
    if updater.status() == Status::Downloading {
        return;
    }
    updater.set(app, |s| {
        s.status = Status::Downloading;
        s.progress = None;
        s.by_user = by_user;
    });
    let mut received = 0u64;
    let mut last_emit = Instant::now();
    let result = update
        .download_and_install(
            |chunk, total| {
                received += chunk as u64;
                if last_emit.elapsed() >= Duration::from_millis(200) {
                    last_emit = Instant::now();
                    updater.set(app, |s| s.progress = total.filter(|&t| t > 0).map(|t| received as f64 / t as f64));
                }
            },
            || {},
        )
        .await;
    match result {
        Ok(()) => updater.set(app, |s| s.status = Status::Ready),
        Err(e) => {
            eprintln!("revlc: update install failed: {e}");
            updater.set(app, |s| s.status = Status::Available);
            if by_user {
                notice(app, "error", format!("Could not install the update: {e}"));
            }
        }
    }
}

fn notice(app: &AppHandle, kind: &'static str, text: String) {
    let _ = app.emit_to("main", "player://notice", crate::Notice { kind, text });
}

#[tauri::command]
pub fn update_state(updater: tauri::State<Updater>) -> UpdateState {
    updater.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[tauri::command]
pub async fn update_check(app: AppHandle) {
    check(&app, true).await;
}

#[tauri::command]
pub async fn update_install(app: AppHandle) {
    install(&app, true).await;
}

/// Restarts into the installed update, saving the resume point first.
#[tauri::command]
pub fn update_restart(app: AppHandle) {
    if let Some(player) = app.try_state::<Player>()
        && let Ok(core) = player.core()
    {
        core.stop();
    }
    app.restart();
}
