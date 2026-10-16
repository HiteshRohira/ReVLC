//! Playback quality, for analytics: does playback start, how long until
//! the first frame, how long seeks take, and how often playback stalls.
//!
//! Fed by the libVLC event pump. Each event carries the time libVLC sent
//! it, so the 20 ms batching in the pump doesn't skew the numbers.
//!
//! What libVLC 3 does, which the measurements rely on:
//! - Opening: Buffering climbs to 100, then the video output appears
//!   (Vout, roughly the first frame), then time updates start.
//! - Seeking: a time update with the target comes at once, then Buffering
//!   drops and climbs back to 100 (instantly for local files, seconds for
//!   streams), and playback resumes with the next time update.
//! - Stalls: mid-playback, Buffering drops below 100 and climbs back.
//! - Time updates come every 250–500 ms, so a seek's resume time is the
//!   first one's arrival less the media time already played past the target.
//!
//! Events: playback_started (time to first frame), playback_failed,
//! playback_abandoned (closed before the first frame), seek_completed,
//! seek_failed and playback_ended (watch time, stalls, seeks).

use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::telemetry;
use crate::vlc::{Event, StreamInfo, Vlc, event};

/// A seek that takes longer than this counts as failed.
const SEEK_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct Quality {
    playback: Option<Playback>,
}

struct Playback {
    /// Links this playback's events.
    id: String,
    /// The media generation it belongs to (see [`Vlc::generation`]).
    generation: u64,
    opened: Instant,
    /// Where it was asked to start (a resume point), in ms.
    start_ms: i64,
    /// What's played, sent with each event: source, scheme, format...
    props: Map<String, Value>,
    cache: f32,
    first_frame: Option<Instant>,
    seek: Option<Seek>,
    stall: Option<Instant>,
    stalls: u32,
    stall_ms: u64,
    seeks: u32,
    /// Seeks replaced by another before they finished.
    superseded: u32,
    last_time: Option<i64>,
    watched_ms: i64,
}

struct Seek {
    requested: Instant,
    from: i64,
    target: i64,
    rate: f32,
    kind: &'static str,
    /// Buffering has dropped since the request.
    flushed: bool,
}

/// How a playback was opened.
pub struct Open<'a> {
    pub uri: &'a str,
    /// Resumes from this point (ms) rather than the start.
    pub resume_ms: Option<i64>,
    pub hardware_decoding: bool,
    pub file_caching: i64,
    pub network_caching: i64,
}

impl Quality {
    /// A new item was opened (or failed to open) as `generation`.
    pub fn opened(&mut self, open: Open, generation: u64, error: Option<&str>) {
        self.close("replaced");
        let mut props = source_props(open.uri);
        let network = props.get("source").and_then(Value::as_str) == Some("network");
        props.insert("resumed".into(), open.resume_ms.is_some().into());
        props.insert("hw_decoding".into(), open.hardware_decoding.into());
        props.insert("caching_ms".into(), (if network { open.network_caching } else { open.file_caching }).into());
        let playback = Playback {
            id: uuid::Uuid::new_v4().to_string(),
            generation,
            opened: Instant::now(),
            start_ms: open.resume_ms.unwrap_or(0),
            props,
            cache: 0.0,
            first_frame: None,
            seek: None,
            stall: None,
            stalls: 0,
            stall_ms: 0,
            seeks: 0,
            superseded: 0,
            last_time: None,
            watched_ms: 0,
        };
        match error {
            Some(_) => playback.capture("playback_failed", json!({ "stage": "open", "elapsed_ms": 0 })),
            None => self.playback = Some(playback),
        }
    }

    /// The item became something else that keeps playing as `generation`:
    /// a web page or remote playlist became its stream.
    pub fn followed(&mut self, generation: u64) {
        if let Some(p) = &mut self.playback {
            p.generation = generation;
            p.cache = 0.0;
            p.props.insert("expanded".into(), true.into());
        }
    }

    /// The item turned into several others, each measured on its own.
    pub fn forget(&mut self) {
        self.playback = None;
    }

    /// The user asked to seek to `target` from `from` (ms).
    pub fn seek_requested(&mut self, vlc: &Vlc, from: i64, target: i64, kind: &'static str) {
        let Some(p) = &mut self.playback else { return };
        // Before the first frame, seeks only move the start; paused, playback
        // doesn't resume until the user says so.
        if p.first_frame.is_none() || !vlc.seekable() || vlc.state() != crate::vlc::State::Playing {
            return;
        }
        p.close_stall(Instant::now());
        if p.seek.take().is_some() {
            p.superseded += 1;
        }
        p.seeks += 1;
        let rate = vlc.rate();
        p.seek = Some(Seek { requested: Instant::now(), from, target, rate: if rate > 0.0 { rate } else { 1.0 }, kind, flushed: false });
    }

    /// Playback stopped: the user stopped it, picked something else, or quit.
    pub fn close(&mut self, reason: &str) {
        let Some(mut p) = self.playback.take() else { return };
        let now = Instant::now();
        if p.first_frame.is_none() {
            p.capture("playback_abandoned", json!({ "reason": reason, "waited_ms": ms(now - p.opened) }));
            return;
        }
        if let Some(seek) = p.seek.take() {
            p.capture("seek_failed", seek.props(json!({ "reason": "closed", "waited_ms": ms(now - seek.requested) })));
        }
        p.end(reason, now);
    }

    /// Handles a batch of libVLC events, in the order they came.
    pub fn handle(&mut self, events: &[Event], vlc: &Vlc) {
        for e in events {
            let Some(p) = &mut self.playback else { return };
            if e.generation != p.generation {
                continue;
            }
            match e.kind {
                event::BUFFERING => p.buffering(e),
                event::VOUT if p.first_frame.is_none() && e.cache >= 100.0 && vlc.has_video() => p.started(e.at, vlc),
                event::TIME_CHANGED => p.time_changed(e, vlc),
                event::PAUSED => {
                    // Paused before playback resumed: the wait is the user's.
                    p.seek = None;
                    p.close_stall(e.at);
                }
                event::ERROR | event::END_REACHED => {
                    let mut p = self.playback.take().unwrap();
                    let error = e.kind == event::ERROR;
                    if p.first_frame.is_none() {
                        let stage = if error { "error" } else { "ended_before_playing" };
                        p.capture("playback_failed", json!({ "stage": stage, "elapsed_ms": ms(e.at - p.opened) }));
                    } else {
                        p.end(if error { "error" } else { "finished" }, e.at);
                    }
                }
                _ => {}
            }
        }
    }
}

impl Playback {
    fn capture(&self, name: &str, extra: Value) {
        let mut props = self.props.clone();
        props.insert("playback_id".into(), self.id.clone().into());
        if let Value::Object(extra) = extra {
            props.extend(extra);
        }
        telemetry::capture(name, Value::Object(props));
    }

    fn started(&mut self, at: Instant, vlc: &Vlc) {
        self.first_frame = Some(at);
        let info = vlc.stream_info();
        let length = vlc.length();
        self.props.extend(stream_props(&info, length));
        self.capture("playback_started", json!({ "ttff_ms": ms(at.saturating_duration_since(self.opened)) }));
    }

    fn buffering(&mut self, e: &Event) {
        self.cache = e.cache;
        if self.first_frame.is_none() {
            return;
        }
        if let Some(seek) = &mut self.seek {
            if e.cache < 100.0 {
                seek.flushed = true;
            }
        } else if e.cache < 100.0 {
            self.stall.get_or_insert(e.at);
        } else {
            self.close_stall(e.at);
        }
    }

    fn time_changed(&mut self, e: &Event, vlc: &Vlc) {
        let previous = self.last_time.replace(e.time);
        if self.first_frame.is_none() {
            // Audio, or video whose output came before buffering finished.
            // The first update comes a little after playback starts: take
            // off what has played since.
            if e.cache >= 100.0 {
                let played = (e.time - self.start_ms).clamp(0, 2000);
                self.started(e.at - Duration::from_millis(played as u64), vlc);
            }
            return;
        }
        if let Some(seek) = &self.seek {
            let waited = e.at.saturating_duration_since(seek.requested);
            if waited > SEEK_TIMEOUT {
                let seek = self.seek.take().unwrap();
                self.capture("seek_failed", seek.props(json!({ "reason": "timeout", "waited_ms": ms(waited) })));
            } else if seek.flushed && e.cache >= 100.0 {
                let ahead = ((e.time - seek.target).max(0) as f32 / seek.rate) as u64;
                let latency = waited.saturating_sub(Duration::from_millis(ahead));
                let seek = self.seek.take().unwrap();
                self.capture("seek_completed", seek.props(json!({ "latency_ms": ms(latency) })));
            }
            return;
        }
        // Watch time: media time that played, skipping jumps.
        if let Some(previous) = previous {
            let delta = e.time - previous;
            if delta > 0 && delta < 5000 {
                self.watched_ms += delta;
            }
        }
    }

    fn close_stall(&mut self, at: Instant) {
        if let Some(start) = self.stall.take() {
            self.stalls += 1;
            self.stall_ms += ms(at.saturating_duration_since(start));
        }
    }

    fn end(&mut self, reason: &str, at: Instant) {
        self.close_stall(at);
        let props = json!({
            "reason": reason,
            "watched_ms": self.watched_ms,
            "stalls": self.stalls,
            "stall_ms": self.stall_ms,
            "seeks": self.seeks,
            "superseded_seeks": self.superseded,
        });
        self.capture("playback_ended", props);
    }
}

impl Seek {
    fn props(&self, extra: Value) -> Value {
        let mut props = json!({
            "kind": self.kind,
            "distance_ms": (self.target - self.from).abs(),
            "direction": if self.target >= self.from { "forward" } else { "back" },
            "rate": self.rate,
        });
        if let (Value::Object(props), Value::Object(extra)) = (&mut props, extra) {
            props.extend(extra);
        }
        props
    }
}

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

/// Containers and formats worth naming; anything else is "other", so
/// odd file names never leave the machine.
const FORMATS: &[&str] = &[
    "mp4", "m4v", "mkv", "webm", "mov", "avi", "ts", "m2ts", "mts", "mpg", "mpeg", "wmv", "flv", "ogv", "3gp", "vob",
    "mxf", "rmvb", "asf", "mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", "wma", "aiff", "mka", "m3u8", "mpd", "iso",
];

/// What kind of source a URI is, without the URI itself.
fn source_props(uri: &str) -> Map<String, Value> {
    let scheme = uri.split_once("://").map(|(s, _)| s.to_ascii_lowercase());
    let source = match scheme.as_deref() {
        None | Some("file") => "local",
        Some("dvd" | "dvdsimple" | "bluray" | "cdda" | "vcd") => "disc",
        Some("avcapture" | "avaudiocapture" | "screen" | "dshow" | "qtcapture") => "capture",
        Some(_) => "network",
    };
    // The last path segment's extension, ignoring any query or fragment.
    let path = uri.split(['?', '#']).next().unwrap_or(uri);
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    let format = match ext {
        Some(e) if FORMATS.contains(&e.as_str()) => e,
        _ if source == "local" || source == "network" => "other".into(),
        _ => "none".into(),
    };
    let mut props = Map::new();
    props.insert("source".into(), source.into());
    props.insert("scheme".into(), scheme.unwrap_or_else(|| "file".into()).into());
    props.insert("format".into(), format.into());
    props
}

/// What's playing: codecs, resolution, duration.
fn stream_props(info: &StreamInfo, length: i64) -> Map<String, Value> {
    let mut props = Map::new();
    props.insert("has_video".into(), info.video_codec.is_some().into());
    props.insert("video_codec".into(), info.video_codec.clone().into());
    props.insert("audio_codec".into(), info.audio_codec.clone().into());
    if info.height > 0 {
        props.insert("width".into(), info.width.into());
        props.insert("height".into(), info.height.into());
        props.insert("resolution".into(), resolution(info.width, info.height).into());
    }
    if info.fps > 0.0 {
        props.insert("fps".into(), ((info.fps * 100.0).round() / 100.0).into());
    }
    props.insert("live".into(), (length <= 0).into());
    if length > 0 {
        props.insert("duration_s".into(), (length / 1000).into());
    }
    props
}

/// The usual name for a frame size, as if it were 16:9, so letterboxed
/// and portrait video land in the right bucket.
fn resolution(width: u32, height: u32) -> &'static str {
    let (long, short) = (width.max(height), width.min(height));
    match long.max(short * 16 / 9) {
        w if w > 3840 => "8K",
        w if w > 2560 => "4K",
        w if w > 1920 => "1440p",
        w if w > 1280 => "1080p",
        w if w > 854 => "720p",
        w if w > 640 => "480p",
        _ => "SD",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(uri: &str) -> (String, String, String) {
        let p = source_props(uri);
        let get = |k: &str| p[k].as_str().unwrap().to_string();
        (get("source"), get("scheme"), get("format"))
    }

    #[test]
    fn sources_without_names() {
        let s = |a: &str, b: &str, c: &str| (a.to_string(), b.to_string(), c.to_string());
        assert_eq!(props("/Movies/Secret Film.MKV"), s("local", "file", "mkv"));
        assert_eq!(props(r"C:\Videos\x.weird"), s("local", "file", "other"));
        assert_eq!(props("https://cdn.example.com/live/index.m3u8?token=abc.mp4"), s("network", "https", "m3u8"));
        assert_eq!(props("https://www.youtube.com/watch?v=abc"), s("network", "https", "other"));
        assert_eq!(props("dvd:///Volumes/DISC"), s("disc", "dvd", "none"));
        assert_eq!(props("avcapture://"), s("capture", "avcapture", "none"));
    }

    #[test]
    fn resolutions() {
        assert_eq!(resolution(1920, 1080), "1080p");
        assert_eq!(resolution(1920, 800), "1080p");
        assert_eq!(resolution(1080, 1920), "1080p");
        assert_eq!(resolution(3840, 2160), "4K");
        assert_eq!(resolution(7680, 4320), "8K");
        assert_eq!(resolution(1280, 720), "720p");
        assert_eq!(resolution(320, 184), "SD");
    }
}

