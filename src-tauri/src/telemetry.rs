//! Anonymous usage analytics, sent to PostHog.
//!
//! What it's for: which versions and machines ReVLC runs on, and whether
//! people get a good viewing experience (playback that starts, starts
//! quickly and seeks quickly). Each install gets a random id; nothing
//! identifies the user or what they watch: no file names, titles, paths,
//! URLs or hosts, only the kind of source ("local", "network"...), its
//! scheme and a known container extension. Turned off in Settings, nothing
//! is sent or queued.
//!
//! Events are queued and sent in batches from a thread of their own.
//! Development builds print them instead of sending them, unless
//! REVLC_ANALYTICS_SEND=1.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The PostHog project's API key (also in site/analytics.js). Project keys
/// only allow sending events, so they're meant to be public. Empty turns
/// analytics off; REVLC_POSTHOG_KEY at build time overrides it.
const PROJECT_KEY: &str = "";
const KEY: &str = match option_env!("REVLC_POSTHOG_KEY") {
    Some(key) => key,
    None => PROJECT_KEY,
};
const HOST: &str = match option_env!("REVLC_POSTHOG_HOST") {
    Some(host) => host,
    None => "https://us.i.posthog.com",
};

/// Sends at least this often while events are waiting...
const SEND_EVERY: Duration = Duration::from_secs(30);
/// ...or as soon as this many are.
const BATCH: usize = 20;
/// Events kept while offline; the oldest go first.
const MAX_QUEUED: usize = 500;
/// How long quitting waits for the last events to go.
const EXIT_WAIT: Duration = Duration::from_secs(1);

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

struct Telemetry {
    enabled: AtomicBool,
    /// Whether events leave the machine (release builds with a key).
    sends: bool,
    id: String,
    /// Sent with every event: app version, OS, machine.
    common: Map<String, Value>,
    state: Mutex<Queue>,
    wake: Condvar,
}

#[derive(Default)]
struct Queue {
    events: Vec<Value>,
    /// The last day (Unix days, UTC) an event was sent on, so one that
    /// stays open overnight still counts as active the next day.
    day: u64,
    exiting: bool,
    /// Set by the sender once it's done for good.
    done: bool,
}

/// What's kept between launches.
#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Saved {
    id: String,
    last_version: String,
}

static TELEMETRY: OnceLock<Telemetry> = OnceLock::new();

/// Starts analytics, sending the launch (and first-install or update) event.
pub fn start(data_dir: &Path, enabled: bool, engine: &str) {
    let file = data_dir.join("analytics.json");
    let mut saved: Saved = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let installed = saved.id.is_empty();
    if installed {
        saved.id = uuid::Uuid::new_v4().to_string();
    }
    let version = env!("CARGO_PKG_VERSION");
    let previous = std::mem::replace(&mut saved.last_version, version.to_string());
    if previous != version {
        let _ = crate::store::write_json(&file, &saved, true);
    }
    let sends = !KEY.is_empty() && (!cfg!(debug_assertions) || std::env::var_os("REVLC_ANALYTICS_SEND").is_some());
    let telemetry = Telemetry {
        enabled: AtomicBool::new(enabled),
        sends,
        id: saved.id,
        common: common_properties(engine),
        state: Mutex::default(),
        wake: Condvar::new(),
    };
    if TELEMETRY.set(telemetry).is_err() {
        return;
    }
    if installed {
        capture("app_installed", json!({}));
    } else if !previous.is_empty() && previous != version {
        capture("app_updated", json!({ "from_version": previous }));
    }
    capture("app_launched", json!({}));
    let _ = thread::Builder::new().name("analytics".into()).spawn(send_loop);
}

/// Turns analytics on or off (the Settings toggle). Off drops what's queued.
pub fn set_enabled(enabled: bool) {
    let Some(t) = TELEMETRY.get() else { return };
    if t.enabled.swap(enabled, Ordering::AcqRel) && !enabled {
        t.state.lock().unwrap_or_else(|e| e.into_inner()).events.clear();
    }
}

/// Queues an event. `properties` must be a JSON object.
pub fn capture(event: &str, properties: Value) {
    let Some(t) = TELEMETRY.get() else { return };
    if !t.enabled.load(Ordering::Acquire) {
        return;
    }
    let mut props = t.common.clone();
    if let Value::Object(extra) = properties {
        props.extend(extra);
    }
    props.insert("distinct_id".into(), t.id.clone().into());
    // Anonymous events: no person profiles.
    props.insert("$process_person_profile".into(), false.into());
    if !t.sends {
        eprintln!("revlc: analytics: {event} {}", Value::Object(props.clone()));
    }
    let now = crate::unix_ms();
    let mut state = t.state.lock().unwrap_or_else(|e| e.into_inner());
    state.day = now / DAY_MS;
    state.events.push(json!({ "event": event, "properties": props, "timestamp": iso8601(now) }));
    let excess = state.events.len().saturating_sub(MAX_QUEUED);
    state.events.drain(..excess);
    if state.events.len() >= BATCH {
        t.wake.notify_all();
    }
}

/// Sends what's queued before the app quits, waiting a moment at most.
pub fn flush_on_exit() {
    let Some(t) = TELEMETRY.get() else { return };
    let mut state = t.state.lock().unwrap_or_else(|e| e.into_inner());
    state.exiting = true;
    t.wake.notify_all();
    let deadline = Instant::now() + EXIT_WAIT;
    while !state.done {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        state = t.wake.wait_timeout(state, left).unwrap_or_else(|e| e.into_inner()).0;
    }
}

fn send_loop() {
    let Some(t) = TELEMETRY.get() else { return };
    let client = t.sends.then(|| {
        // The updater installs the same provider; whichever runs first wins.
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        reqwest::Client::builder().timeout(Duration::from_secs(15)).build().ok()
    });
    let client = client.flatten();
    loop {
        let (batch, exiting) = {
            let mut state = t.state.lock().unwrap_or_else(|e| e.into_inner());
            if !state.exiting && state.events.len() < BATCH {
                state = t.wake.wait_timeout(state, SEND_EVERY).unwrap_or_else(|e| e.into_inner()).0;
            }
            let today = crate::unix_ms() / DAY_MS;
            if state.day != today && t.enabled.load(Ordering::Acquire) && !state.exiting {
                drop(state);
                capture("app_active", json!({}));
                state = t.state.lock().unwrap_or_else(|e| e.into_inner());
            }
            (std::mem::take(&mut state.events), state.exiting)
        };
        if !batch.is_empty()
            && let Some(client) = &client
            && !send(client, &batch, exiting)
            && !exiting
        {
            // Offline: try again next time, ahead of newer events.
            let mut state = t.state.lock().unwrap_or_else(|e| e.into_inner());
            let newer = std::mem::replace(&mut state.events, batch);
            state.events.extend(newer);
            let excess = state.events.len().saturating_sub(MAX_QUEUED);
            state.events.drain(..excess);
        }
        if exiting {
            t.state.lock().unwrap_or_else(|e| e.into_inner()).done = true;
            t.wake.notify_all();
            return;
        }
    }
}

fn send(client: &reqwest::Client, batch: &[Value], exiting: bool) -> bool {
    let body = json!({ "api_key": KEY, "batch": batch });
    let request = client.post(format!("{HOST}/batch/")).json(&body);
    let request = if exiting { request.timeout(EXIT_WAIT) } else { request };
    match tauri::async_runtime::block_on(request.send()) {
        Ok(response) if response.status().is_success() => true,
        // The server turned the batch down: sending it again won't help.
        Ok(response) if response.status().is_client_error() => {
            eprintln!("revlc: analytics rejected: {}", response.status());
            true
        }
        Ok(response) => {
            eprintln!("revlc: analytics not sent: {}", response.status());
            false
        }
        Err(e) => {
            eprintln!("revlc: analytics not sent: {e}");
            false
        }
    }
}

fn common_properties(engine: &str) -> Map<String, Value> {
    let machine = crate::machine::info();
    let mut props = Map::new();
    props.insert("$lib".into(), "revlc".into());
    props.insert("$app_version".into(), env!("CARGO_PKG_VERSION").into());
    props.insert("$os".into(), machine.os.into());
    props.insert("$os_version".into(), machine.os_version.into());
    props.insert("os_build".into(), machine.os_build.into());
    props.insert("arch".into(), std::env::consts::ARCH.into());
    props.insert("cpu".into(), machine.cpu.into());
    props.insert("cpu_cores".into(), thread::available_parallelism().map_or(0, |n| n.get()).into());
    props.insert("ram_gb".into(), machine.ram_gb.into());
    if let Some(model) = machine.model {
        props.insert("machine_model".into(), model.into());
    }
    props.insert("libvlc_version".into(), engine.split_whitespace().next().unwrap_or(engine).into());
    props.insert("debug_build".into(), cfg!(debug_assertions).into());
    props
}

/// Unix ms as an ISO 8601 UTC timestamp.
fn iso8601(ms: u64) -> String {
    let secs = ms / 1000;
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        ms % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601(951_782_400_123), "2000-02-29T00:00:00.123Z");
        assert_eq!(iso8601(1_792_118_404_000), "2026-10-16T02:40:04.000Z");
    }
}
