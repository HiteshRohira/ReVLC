//! Seek bar previews: small frames every few seconds through the whole
//! video, made in the background while it plays and cached as JPEGs.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

/// Width of a preview frame, in pixels (twice what's shown, for sharp screens).
pub const WIDTH: u32 = 320;

/// Changes when frames made by earlier versions shouldn't be reused.
const CACHE_VERSION: u8 = 4;

/// How long unused frames stay in the cache.
const KEEP: Duration = Duration::from_secs(7 * 24 * 3600);

/// How far apart the frames are: close enough for the zoomed-in strip, far
/// enough that a long video is done in a few minutes.
pub fn step(length_ms: i64) -> i64 {
    if length_ms <= 300_000 {
        2000
    } else if length_ms <= 1_800_000 {
        5000
    } else {
        10_000
    }
}

/// Frame indexes coarse to fine: every 64th, then the halves in between,
/// and so on, so the whole video has previews after only a few frames.
pub fn order(count: usize) -> Vec<usize> {
    let mut seen = vec![false; count];
    let mut out = Vec::with_capacity(count);
    let mut stride = count.next_power_of_two() / 2;
    loop {
        for i in (0..count).step_by(stride.max(1)) {
            if !std::mem::replace(&mut seen[i], true) {
                out.push(i);
            }
        }
        if stride <= 1 {
            return out;
        }
        stride /= 2;
    }
}

/// The start of the file names of the frames of video track `track` of
/// `uri`, one every `step` ms: frame `k` is `{prefix}{k}.jpg`. `signature`
/// (the file's size and modification time) keeps a replaced file from
/// reusing the previews of the one before.
pub fn prefix(dir: &Path, uri: &str, length_ms: i64, signature: (u64, u128), track: usize, step: i64) -> String {
    let mut h = DefaultHasher::new();
    (CACHE_VERSION, uri, length_ms, signature, track).hash(&mut h);
    // Joined, not formatted, so Windows paths keep one kind of separator.
    dir.join(format!("{:016x}-f{step}-", h.finish())).to_string_lossy().into_owned()
}

/// Where frame `k` is taken from, in ms.
pub fn time_at(length_ms: i64, step: i64, k: i64) -> i64 {
    (k * step).clamp(0, (length_ms - 1000).max(0))
}

/// How each frame is taken: decoding forward to the exact time, or taking
/// the keyframe before it. Exact frames are what the previews want, but in
/// heavy files (4K HEVC, keyframes far apart) each can take seconds; then the
/// keyframe is five times quicker and usually a different picture anyway.
pub struct Plan {
    keyframes: AtomicBool,
    /// How long the first exact frames took, until there are enough to decide.
    times: Mutex<Vec<Duration>>,
}

impl Plan {
    /// Exact frames slower than this (the middle of the first few) switch the
    /// file to keyframes: 1080p H.264 takes about 0.4 s, 4K HEVC 2 to 6.
    const SLOW: Duration = Duration::from_millis(1000);
    const SAMPLES: usize = 4;

    pub fn new() -> Self {
        Self { keyframes: AtomicBool::new(false), times: Mutex::new(Vec::new()) }
    }

    pub fn keyframes(&self) -> bool {
        self.keyframes.load(Ordering::Relaxed)
    }

    /// Notes how long an exact frame took, deciding once there are enough.
    pub fn exact_took(&self, took: Duration) {
        let mut times = self.times.lock().unwrap_or_else(|e| e.into_inner());
        if times.len() >= Self::SAMPLES {
            return;
        }
        times.push(took);
        if times.len() == Self::SAMPLES {
            times.sort_unstable();
            self.keyframes.store(times[Self::SAMPLES / 2] > Self::SLOW, Ordering::Relaxed);
        }
    }
}

/// Whether two frame files show the same picture: neighbouring frames taken
/// from the same keyframe.
pub fn same_picture(a: &Path, b: &Path) -> bool {
    let (Ok(a), Ok(b)) = (image::open(a), image::open(b)) else { return false };
    let (a, b) = (a.to_rgb8(), b.to_rgb8());
    if a.dimensions() != b.dimensions() {
        return false;
    }
    let sum: u64 = a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| x.abs_diff(*y) as u64).sum();
    sum < a.as_raw().len() as u64 * 2
}

/// Deletes frames not touched for a week.
pub fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|age| age > KEEP);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_covers_every_frame_once_coarse_first() {
        for count in [12, 13, 64, 100, 120] {
            let o = order(count);
            let mut sorted = o.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, (0..count).collect::<Vec<_>>());
            assert_eq!(o[0], 0);
        }
        // The first frames are spread out, not the first few in a row.
        assert!(order(100)[1] >= 50);
    }

    #[test]
    fn slow_exact_frames_switch_to_keyframes() {
        let plan = Plan::new();
        for ms in [3000, 400, 2500, 4000] {
            assert!(!plan.keyframes());
            plan.exact_took(Duration::from_millis(ms));
        }
        assert!(plan.keyframes());
        let plan = Plan::new();
        for ms in [3000, 400, 350, 500] {
            plan.exact_took(Duration::from_millis(ms));
        }
        assert!(!plan.keyframes());
    }

    #[test]
    fn same_picture_tells_frames_apart() {
        let dir = std::env::temp_dir().join(format!("revlc-scrub-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let save = |name: &str, shade: u8| {
            let path = dir.join(name);
            image::RgbImage::from_fn(32, 18, |x, _| image::Rgb([shade, (x * 8) as u8, 0])).save(&path).unwrap();
            path
        };
        let (a, b, c) = (save("a.jpg", 10), save("b.jpg", 10), save("c.jpg", 200));
        assert!(same_picture(&a, &b));
        assert!(!same_picture(&a, &c));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn step_grows_with_length() {
        assert_eq!(step(120_000), 2000);
        assert_eq!(step(1_400_000), 5000);
        assert_eq!(step(7_000_000), 10_000);
    }

    #[test]
    fn frames_sit_on_a_grid() {
        assert_eq!(time_at(600_000, 10_000, 3), 30_000);
        assert_eq!(time_at(600_000, 10_000, 60), 599_000);
        let p = |sig, track| prefix(Path::new("/c"), "a.mkv", 1000, sig, track, 5000);
        assert!(p((1, 2), 0).ends_with("-f5000-"));
        // Another file at the same path, or another video track, gets other names.
        assert_ne!(p((1, 2), 0), p((1, 3), 0));
        assert_ne!(p((1, 2), 0), p((1, 2), 1));
    }
}
