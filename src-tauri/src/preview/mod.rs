//! Warm-session hardware keyframe previewer and bounded in-memory LRU cache.
//!
//! Replaces the 63 MB bundled FFmpeg sidecar on macOS and Windows:
//! - Uses [`crate::container::index_video`] (pure-Rust MP4/MOV/MKV/WebM parser)
//!   to build per-track keyframe tables in milliseconds.
//! - Reuses ONE warm OS hardware decoder session per active `(file, track)`
//!   (VideoToolbox on macOS, Media Foundation + D3D11VA on Windows, system
//!   VA-API/NVDEC ffmpeg fallback on Linux) across all video tracks.
//! - Keeps decoded JPEG frames in [`cache::ScrubCache`] (bounded LRU in RAM),
//!   served directly to the webview over the custom `scrub://` Tauri protocol.
//! - Prioritizes hovered timestamps immediately via [`PriorityQueue`] ahead of
//!   background coarse-to-fine filling.

pub mod cache;
#[cfg(any(target_os = "macos", windows, test))]
pub mod scale;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
use macos::NativeDecoder;
#[cfg(windows)]
use windows::NativeDecoder;
#[cfg(target_os = "linux")]
use linux::NativeDecoder;
#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
struct NativeDecoder;
#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
impl NativeDecoder {
    fn open(_track: &crate::container::VideoTrackIndex) -> Result<Self, String> {
        Err("native preview decoder unavailable on this OS".into())
    }
    fn decode_keyframe(&mut self, _packet: &[u8], _target_width: u32) -> Result<Vec<u8>, String> {
        Err("native preview decoder unavailable".into())
    }
}

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::container::{self, PacketReader, VideoTrackIndex};
use crate::scrub;

/// A single warm hardware decode session for one `(file, video_track)`.
pub struct WarmSession {
    reader: PacketReader,
    decoder: NativeDecoder,
    track: VideoTrackIndex,
    /// Dedup map from keyframe index -> already-encoded JPEG bytes so adjacent
    /// scrub slots sharing the same keyframe don't re-decode it.
    decoded_kf: HashMap<usize, Vec<u8>>,
}

impl WarmSession {
    /// Opens a warm native preview session for 0-based video track `track_index`.
    pub fn open(path: &Path, track_index: usize) -> Result<Self, String> {
        let indexed = container::index_video(path)?;
        let track = indexed
            .video_tracks
            .into_iter()
            .nth(track_index)
            .ok_or_else(|| format!("video track {track_index} not found in container"))?;
        if !track.codec.is_hw_preview_supported() {
            return Err(format!("codec {:?} not supported by native previewer", track.codec));
        }
        if track.keyframes.is_empty() {
            return Err("container has no indexed keyframes".into());
        }
        let decoder = NativeDecoder::open(&track)?;
        let reader = PacketReader::open(path)?;
        Ok(Self {
            reader,
            decoder,
            track,
            decoded_kf: HashMap::new(),
        })
    }

    #[cfg(all(test, target_os = "macos"))]
    pub fn track(&self) -> &VideoTrackIndex {
        &self.track
    }

    /// False past the point where a truncated file's data runs out, where
    /// playback can't go either, so no preview is shown there.
    pub fn covers(&self, target_ms: i64) -> bool {
        self.track.data_end_ms.is_none_or(|end| target_ms < end)
    }

    /// Decodes the keyframe nearest `target_ms` into a JPEG `target_width` px wide.
    /// Reuses the warm decoder session and deduplicates repeated keyframe lookups.
    pub fn decode_nearest(&mut self, target_ms: i64, target_width: u32) -> Result<(i64, Vec<u8>), String> {
        let kf_idx = self.track.nearest_keyframe_index(target_ms).ok_or("no keyframes")?;
        let pts_ms = self.track.keyframes[kf_idx].pts_ms;
        if let Some(cached) = self.decoded_kf.get(&kf_idx) {
            return Ok((pts_ms, cached.clone()));
        }
        let pkt = self.reader.read_keyframe(&self.track, &self.track.keyframes[kf_idx])?;
        let jpeg = self.decoder.decode_keyframe(&pkt, target_width)?;
        if self.decoded_kf.len() < 128 {
            self.decoded_kf.insert(kf_idx, jpeg.clone());
        }
        Ok((pts_ms, jpeg))
    }
}

/// Priority work queue shared between `scrub_focus` (high priority) and the
/// background coarse-to-fine sweep for one active preview run.
pub struct PriorityQueue {
    inner: Mutex<QueueInner>,
    wake: Condvar,
    cancelled: AtomicBool,
}

struct QueueInner {
    focus: VecDeque<i64>,
    background: VecDeque<i64>,
    done: HashSet<i64>,
    count: i64,
}

impl PriorityQueue {
    pub fn new(count: i64) -> Arc<Self> {
        let capped = count.clamp(0, scrub::MAX_SLOTS);
        let background: VecDeque<i64> = scrub::order(capped as usize).into_iter().map(|k| k as i64).collect();
        Arc::new(Self {
            inner: Mutex::new(QueueInner {
                focus: VecDeque::new(),
                background,
                done: HashSet::new(),
                count: capped,
            }),
            wake: Condvar::new(),
            cancelled: AtomicBool::new(false),
        })
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.focus.clear();
        inner.background.clear();
        self.wake.notify_all();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Immediately prioritizes slots `from..=to` (middle first) ahead of all
    /// background work.
    pub fn prioritize(&self, from: i64, to: i64) {
        if self.is_cancelled() {
            return;
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.count <= 0 {
            return;
        }
        let lo = from.min(to).clamp(0, inner.count - 1);
        let hi = from.max(to).clamp(0, inner.count - 1);
        let mid = (lo + hi) / 2;
        let mut slots: Vec<i64> = (lo..=hi).filter(|k| !inner.done.contains(k)).collect();
        slots.sort_by_key(|&k| (k - mid).abs());
        if slots.is_empty() {
            return;
        }
        inner.focus.clear();
        inner.focus.extend(slots);
        self.wake.notify_all();
    }

    /// Pops the next slot to decode (always preferring hovered focus slots over
    /// background sweep slots). Returns `None` once cancelled or all slots are done.
    pub fn pop_next(&self) -> Option<i64> {
        if self.is_cancelled() {
            return None;
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        while let Some(k) = inner.focus.pop_front() {
            if inner.done.insert(k) {
                return Some(k);
            }
        }
        while let Some(k) = inner.background.pop_front() {
            if inner.done.insert(k) {
                return Some(k);
            }
        }
        None
    }
}

/// Per-process session nonce mixed into `scrub://` URLs so WebView2 / WebKit
/// HTTP caches can never serve frames from a previous app launch.
pub(crate) fn session_nonce() -> u64 {
    static NONCE: OnceLock<u64> = OnceLock::new();
    *NONCE.get_or_init(|| {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
        nanos ^ ((std::process::id() as u64) << 32)
    })
}

/// Platform-appropriate URL prefix for the custom `scrub` Tauri URI scheme.
/// On Windows WebView2 custom schemes map to `http://<scheme>.localhost/`;
/// on macOS and Linux they use `<scheme>://localhost/`.
pub fn scrub_url_prefix(run: u64) -> String {
    let nonce = session_nonce();
    if cfg!(windows) {
        format!("http://scrub.localhost/{nonce}-{run}/")
    } else {
        format!("scrub://localhost/{nonce}-{run}/")
    }
}

/// Parses a `scrub` protocol request path (`/{nonce}-{run}/{k}.jpg`) into
/// `(run, slot_k)`, rejecting requests whose session nonce doesn't match this
/// process.
pub fn parse_scrub_uri_path(path: &str) -> Option<(u64, i64)> {
    let trimmed = path.trim_start_matches('/');
    let (run_part, slot_s) = trimmed.split_once('/')?;
    let (nonce_s, run_s) = run_part.split_once('-')?;
    if nonce_s.parse::<u64>().ok()? != session_nonce() {
        return None;
    }
    let slot_s = slot_s.split('?').next().unwrap_or(slot_s).strip_suffix(".jpg").unwrap_or(slot_s);
    let run = run_s.parse::<u64>().ok()?;
    let slot = slot_s.parse::<i64>().ok()?;
    Some((run, slot))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_queue_puts_hovered_middle_first_and_deduplicates() {
        let q = PriorityQueue::new(20);
        q.prioritize(10, 14);
        let first5: Vec<i64> = (0..5).map(|_| q.pop_next().unwrap()).collect();
        assert_eq!(first5[0], 12);
        assert!(first5.iter().all(|k| (10..=14).contains(k)));
        let mut rest = Vec::new();
        while let Some(k) = q.pop_next() {
            rest.push(k);
        }
        assert_eq!(rest.len(), 15);
        assert!(rest.iter().all(|k| !(10..=14).contains(k)));
    }

    #[test]
    fn priority_queue_cancel_and_out_of_bounds_clamping() {
        let q = PriorityQueue::new(10);
        // Out-of-bounds focus range clamps safely to 0..=9, starting from middle slot 4.
        q.prioritize(-50, 999);
        assert_eq!(q.pop_next(), Some(4));
        let mut all = vec![4];
        while let Some(k) = q.pop_next() {
            all.push(k);
        }
        assert_eq!(all.len(), 10);
        all.sort();
        assert_eq!(all, (0..10).collect::<Vec<_>>());

        let q2 = PriorityQueue::new(10);
        q2.cancel();
        assert!(q2.is_cancelled());
        assert_eq!(q2.pop_next(), None);
        // Prioritizing after cancel stays empty.
        q2.prioritize(0, 5);
        assert_eq!(q2.pop_next(), None);
    }

    #[test]
    fn scrub_uri_path_validates_nonce_and_parses_slot() {
        let n = session_nonce();
        assert_eq!(parse_scrub_uri_path(&format!("/{n}-42/7.jpg")), Some((42, 7)));
        assert_eq!(parse_scrub_uri_path(&format!("/{n}-42/7.jpg?v=3")), Some((42, 7)));
        assert_eq!(parse_scrub_uri_path(&format!("/{n}-42/7")), Some((42, 7)));
        // Mismatched nonce (from an older process launch) is rejected.
        assert_eq!(parse_scrub_uri_path(&format!("/{}-42/7.jpg", n.wrapping_add(1))), None);
        assert_eq!(parse_scrub_uri_path("/bad/path"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn warm_session_decodes_h264_hevc_and_mkv_keyframes_on_macos() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("testmedia");
        for rel in ["test-1080p.mp4", "promo/Webb - Cassiopeia A.mp4", "Tears of Steel.mkv"] {
            let p = root.join(rel);
            if !p.exists() {
                continue;
            }
            let mut session = WarmSession::open(&p, 0).unwrap_or_else(|e| panic!("open {rel}: {e}"));
            let (_, jpg1) = session.decode_nearest(0, scrub::WIDTH).unwrap_or_else(|e| panic!("decode 0 {rel}: {e}"));
            let img1 = image::load_from_memory(&jpg1).expect("valid JPEG 1");
            assert_eq!(img1.width(), scrub::WIDTH);
            let (_, jpg2) = session.decode_nearest(15_000, scrub::WIDTH).unwrap_or_else(|e| panic!("decode 15s {rel}: {e}"));
            let img2 = image::load_from_memory(&jpg2).expect("valid JPEG 2");
            assert_eq!(img2.width(), scrub::WIDTH);
            assert!(session.track().width > 0);
        }
    }

    #[test]
    fn warm_session_rejects_corrupt_files_without_panicking() {
        let dir = std::env::temp_dir().join(format!("revlc-preview-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bad_mp4 = dir.join("corrupt.mp4");
        std::fs::write(&bad_mp4, b"\x00\x00\x00\x18ftypisom\x00\x00\x02\x00isommp41\x00\x00\x01\x00moov\xFF\xFF").unwrap();
        assert!(WarmSession::open(&bad_mp4, 0).is_err());
        let bad_mkv = dir.join("corrupt.mkv");
        std::fs::write(&bad_mkv, [0x1A, 0x45, 0xDF, 0xA3, 0x9F, 0x42, 0x86, 0x81, 0x01, 0x18, 0x53, 0x80, 0x67, 0xFF]).unwrap();
        assert!(WarmSession::open(&bad_mkv, 0).is_err());
        assert!(WarmSession::open(&dir.join("missing.mp4"), 0).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Engine-integration test (`engine-tests.yml` runs `-E 'test(/_it_/)' --run-ignored only`)
    /// exercising native warm-session decoding on macOS (VideoToolbox) and Windows
    /// (Media Foundation + D3D11VA) with real FFmpeg-generated H.264 MP4 and multi-track MKV.
    #[cfg(any(target_os = "macos", windows))]
    #[test]
    #[ignore = "needs FFmpeg; run via engine-tests.yml or REVLC_TEST_FFMPEG=... cargo test -- --ignored preview_it"]
    fn preview_it_warm_session_decodes_mp4_and_multitrack_mkv() {
        let ffmpeg = std::env::var_os("REVLC_TEST_FFMPEG")
            .map(std::path::PathBuf::from)
            .or_else(crate::keyframes::ffmpeg)
            .expect("FFmpeg required for preview_it");
        let dir = std::env::temp_dir().join(format!("revlc-preview-it-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 1. H.264 MP4 with distinct green (0-2s) and blue (2-4s) segments.
        let mp4 = dir.join("two-colors.mp4");
        let status = std::process::Command::new(&ffmpeg)
            .args([
                "-hide_banner", "-loglevel", "error", "-y",
                "-f", "lavfi", "-i", "color=c=green:s=320x180:d=2:r=10",
                "-f", "lavfi", "-i", "color=c=blue:s=320x180:d=2:r=10",
                "-filter_complex", "[0:v][1:v]concat=n=2:v=1:a=0[v]",
                "-map", "[v]", "-c:v", "libx264", "-g", "10", "-pix_fmt", "yuv420p",
                mp4.to_str().unwrap(),
            ])
            .status()
            .expect("spawn ffmpeg for mp4");
        assert!(status.success(), "ffmpeg failed to build H.264 MP4");

        let mut mp4_session = WarmSession::open(&mp4, 0).expect("open H.264 MP4 warm session");
        let (_, jpg_start) = mp4_session.decode_nearest(200, scrub::WIDTH).expect("decode green frame");
        let (_, jpg_late) = mp4_session.decode_nearest(3000, scrub::WIDTH).expect("decode blue frame");
        let im_start = image::load_from_memory(&jpg_start).unwrap().to_rgb8();
        let im_late = image::load_from_memory(&jpg_late).unwrap().to_rgb8();
        assert_eq!(im_start.width(), scrub::WIDTH);
        assert_eq!(im_late.width(), scrub::WIDTH);
        let p_start = im_start.get_pixel(im_start.width() / 2, im_start.height() / 2);
        let p_late = im_late.get_pixel(im_late.width() / 2, im_late.height() / 2);
        assert!(p_start[1] > p_start[2] + 30, "expected green at 0.2s, got {p_start:?}");
        assert!(p_late[2] > p_late[1] + 30, "expected blue at 3.0s, got {p_late:?}");

        // 2. Two-track H.264 MKV (track 0 red, track 1 blue).
        let mkv = dir.join("two-tracks.mkv");
        let status = std::process::Command::new(&ffmpeg)
            .args([
                "-hide_banner", "-loglevel", "error", "-y",
                "-f", "lavfi", "-i", "color=c=red:s=320x180:d=4:r=10",
                "-f", "lavfi", "-i", "color=c=blue:s=320x180:d=4:r=10",
                "-map", "0:v", "-map", "1:v",
                "-c:v", "libx264", "-g", "10", "-pix_fmt", "yuv420p",
                mkv.to_str().unwrap(),
            ])
            .status()
            .expect("spawn ffmpeg for mkv");
        assert!(status.success(), "ffmpeg failed to build 2-track MKV");

        let mut s0 = WarmSession::open(&mkv, 0).expect("open MKV track 0");
        let (_, j0) = s0.decode_nearest(1000, scrub::WIDTH).expect("decode MKV track 0");
        let im0 = image::load_from_memory(&j0).unwrap().to_rgb8();
        let p0 = im0.get_pixel(im0.width() / 2, im0.height() / 2);
        assert!(p0[0] > 150 && p0[2] < 80, "track 0 should be red, got {p0:?}");

        let mut s1 = WarmSession::open(&mkv, 1).expect("open MKV track 1");
        let (_, j1) = s1.decode_nearest(1000, scrub::WIDTH).expect("decode MKV track 1");
        let im1 = image::load_from_memory(&j1).unwrap().to_rgb8();
        let p1 = im1.get_pixel(im1.width() / 2, im1.height() / 2);
        assert!(p1[2] > 150 && p1[0] < 80, "track 1 should be blue, got {p1:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn warm_session_decodes_both_tracks_of_multitrack_mkv() {
        let ffmpeg = std::env::var_os("REVLC_TEST_FFMPEG")
            .map(std::path::PathBuf::from)
            .or_else(crate::keyframes::ffmpeg);
        let Some(ffmpeg) = ffmpeg else { return };
        let dir = std::env::temp_dir().join(format!("revlc-preview-mt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mkv = dir.join("two-tracks.mkv");
        // Track 0: red, Track 1: blue.
        let ok = std::process::Command::new(&ffmpeg)
            .args([
                "-hide_banner", "-loglevel", "error", "-y",
                "-f", "lavfi", "-i", "color=c=red:s=320x180:d=4:r=10",
                "-f", "lavfi", "-i", "color=c=blue:s=320x180:d=4:r=10",
                "-map", "0:v", "-map", "1:v",
                "-c:v", "libx264", "-g", "10", "-pix_fmt", "yuv420p",
                mkv.to_str().unwrap(),
            ])
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let mut s0 = WarmSession::open(&mkv, 0).expect("open track 0");
        let mut s1 = WarmSession::open(&mkv, 1).expect("open track 1");
        let (_, jpg0) = s0.decode_nearest(1000, 160).expect("decode track 0");
        let (_, jpg1) = s1.decode_nearest(1000, 160).expect("decode track 1");
        let img0 = image::load_from_memory(&jpg0).unwrap().to_rgb8();
        let img1 = image::load_from_memory(&jpg1).unwrap().to_rgb8();
        let p0 = img0.get_pixel(80, 45);
        let p1 = img1.get_pixel(80, 45);
        assert!(p0[0] > 150 && p0[2] < 80, "track 0 should be red, got {p0:?}");
        assert!(p1[2] > 150 && p1[0] < 80, "track 1 should be blue, got {p1:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
