//! Seek bar previews: small frames every few seconds through the whole
//! video, made in the background while it plays and cached as JPEGs.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
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

/// The start of the file names of `uri`'s frames, one every `step` ms:
/// frame `k` is `{prefix}{k}.jpg`.
pub fn prefix(dir: &Path, uri: &str, length_ms: i64, step: i64) -> String {
    let mut h = DefaultHasher::new();
    (CACHE_VERSION, uri, length_ms).hash(&mut h);
    format!("{}/{:016x}-f{step}-", dir.display(), h.finish())
}

/// Where frame `k` is taken from, in ms.
pub fn time_at(length_ms: i64, step: i64, k: i64) -> i64 {
    (k * step).clamp(0, (length_ms - 1000).max(0))
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
    fn step_grows_with_length() {
        assert_eq!(step(120_000), 2000);
        assert_eq!(step(1_400_000), 5000);
        assert_eq!(step(7_000_000), 10_000);
    }

    #[test]
    fn frames_sit_on_a_grid() {
        assert_eq!(time_at(600_000, 10_000, 3), 30_000);
        assert_eq!(time_at(600_000, 10_000, 60), 599_000);
        assert!(prefix(Path::new("/c"), "a.mkv", 1000, 5000).ends_with("-f5000-"));
    }
}
