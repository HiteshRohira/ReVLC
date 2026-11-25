//! Bounded in-memory LRU cache for decoded seek-bar preview JPEGs.
//!
//! Keeps preview frames purely in RAM (never writing hundreds of tiny JPEGs to
//! disk) and serves them directly to the webview via the custom `scrub://`
//! Tauri URI scheme protocol. Tracks the active run ID so a cancelled run can
//! never insert stale frames after a new run starts or cancels.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Default memory budget: 48 MiB (~3,500 frames at 320px JPEG quality 80).
pub const DEFAULT_MAX_BYTES: usize = 48 * 1024 * 1024;
/// Default entry cap (covers the capped 2,000-slot grid without eviction).
pub const DEFAULT_MAX_ENTRIES: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    run: u64,
    slot: i64,
}

struct Entry {
    bytes: Arc<[u8]>,
    seq: u64,
}

struct Inner {
    active_run: u64,
    map: HashMap<Key, Entry>,
    order: VecDeque<(Key, u64)>,
    total_bytes: usize,
    next_seq: u64,
    max_bytes: usize,
    max_entries: usize,
}

/// Thread-safe bounded LRU cache for seek-bar preview JPEGs.
pub struct ScrubCache {
    inner: Mutex<Inner>,
}

impl Default for ScrubCache {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_BYTES, DEFAULT_MAX_ENTRIES)
    }
}

impl ScrubCache {
    pub fn new(max_bytes: usize, max_entries: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                active_run: 0,
                map: HashMap::new(),
                order: VecDeque::new(),
                total_bytes: 0,
                next_seq: 1,
                max_bytes: max_bytes.max(1024),
                max_entries: max_entries.max(1),
            }),
        }
    }

    /// Sets the active preview run and clears frames from earlier runs.
    /// Any subsequent [`Self::insert_if_active`] call for an older run is ignored.
    pub fn reset_for_run(&self, run: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.active_run = run;
        inner.map.clear();
        inner.order.clear();
        inner.total_bytes = 0;
    }

    /// Inserts a JPEG frame only if `run` is still the active run.
    /// Returns `true` if inserted, `false` if `run` was already superseded.
    pub fn insert_if_active(&self, run: u64, slot: i64, jpeg: Vec<u8>) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.active_run != run {
            return false;
        }
        Self::insert_inner(&mut inner, run, slot, jpeg);
        true
    }

    /// Unconditional insert (for benchmarks and unit tests).
    #[cfg(test)]
    pub fn insert(&self, run: u64, slot: i64, jpeg: Vec<u8>) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::insert_inner(&mut inner, run, slot, jpeg);
    }

    fn insert_inner(inner: &mut Inner, run: u64, slot: i64, jpeg: Vec<u8>) {
        let len = jpeg.len();
        if len == 0 || len > inner.max_bytes {
            return;
        }
        let key = Key { run, slot };
        if let Some(old) = inner.map.remove(&key) {
            inner.total_bytes = inner.total_bytes.saturating_sub(old.bytes.len());
        }
        let seq = inner.next_seq;
        inner.next_seq = inner.next_seq.wrapping_add(1);
        inner.total_bytes += len;
        inner.map.insert(key, Entry { bytes: Arc::from(jpeg), seq });
        inner.order.push_back((key, seq));
        Self::evict(inner);
    }

    /// Retrieves JPEG bytes for `(run, slot)` and refreshes its LRU recency.
    pub fn get(&self, run: u64, slot: i64) -> Option<Arc<[u8]>> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let key = Key { run, slot };
        let seq = inner.next_seq;
        let bytes = {
            let entry = inner.map.get_mut(&key)?;
            entry.seq = seq;
            Arc::clone(&entry.bytes)
        };
        inner.next_seq = inner.next_seq.wrapping_add(1);
        inner.order.push_back((key, seq));
        if inner.order.len() > inner.map.len().saturating_mul(4).max(64) {
            let Inner { map, order, .. } = &mut *inner;
            order.retain(|(k, s)| map.get(k).is_some_and(|e| e.seq == *s));
        }
        Some(bytes)
    }

    #[cfg(test)]
    pub fn contains(&self, run: u64, slot: i64) -> bool {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.map.contains_key(&Key { run, slot })
    }

    fn evict(inner: &mut Inner) {
        while inner.total_bytes > inner.max_bytes || inner.map.len() > inner.max_entries {
            let Some((k, seq)) = inner.order.pop_front() else {
                inner.map.clear();
                inner.total_bytes = 0;
                break;
            };
            let is_live = inner.map.get(&k).is_some_and(|e| e.seq == seq);
            if is_live && let Some(removed) = inner.map.remove(&k) {
                inner.total_bytes = inner.total_bytes.saturating_sub(removed.bytes.len());
            }
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().map.len()
    }

    #[cfg(test)]
    pub fn total_bytes(&self) -> usize {
        self.inner.lock().unwrap().total_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recently_used_under_byte_and_entry_limits() {
        let cache = ScrubCache::new(1024, 3);
        cache.insert(1, 0, vec![1; 400]);
        cache.insert(1, 1, vec![2; 400]);
        assert_eq!(cache.len(), 2);
        // Touch slot 0 so slot 1 becomes LRU.
        assert_eq!(cache.get(1, 0).unwrap().len(), 400);
        // Inserting slot 2 (400B -> 1200B > 1024B) evicts slot 1, keeping 0 and 2.
        cache.insert(1, 2, vec![3; 400]);
        assert!(cache.contains(1, 0));
        assert!(!cache.contains(1, 1));
        assert!(cache.contains(1, 2));
        assert!(cache.total_bytes() <= 1024);
    }

    #[test]
    fn reset_for_run_rejects_stale_inserts() {
        let cache = ScrubCache::default();
        cache.reset_for_run(1);
        assert!(cache.insert_if_active(1, 0, vec![0xFF; 32]));
        cache.reset_for_run(2);
        assert!(!cache.contains(1, 0));
        assert!(!cache.insert_if_active(1, 1, vec![0xFF; 32]));
        assert!(cache.insert_if_active(2, 0, vec![0xEE; 32]));
        assert!(cache.contains(2, 0));
    }

    #[test]
    fn overwrite_updates_byte_accounting_and_is_thread_safe() {
        let cache = Arc::new(ScrubCache::new(4096, 64));
        cache.reset_for_run(1);
        assert!(cache.insert_if_active(1, 0, vec![1; 1000]));
        assert_eq!(cache.total_bytes(), 1000);
        // Overwriting slot 0 with a smaller JPEG updates total_bytes and payload accurately.
        assert!(cache.insert_if_active(1, 0, vec![2; 300]));
        assert_eq!(cache.total_bytes(), 300);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(1, 0).as_deref(), Some(vec![2u8; 300].as_slice()));

        // Entry cap evicts independently of byte cap.
        let capped = ScrubCache::new(1024 * 1024, 3);
        capped.reset_for_run(1);
        for k in 0..5 {
            assert!(capped.insert_if_active(1, k, vec![k as u8; 16]));
        }
        assert_eq!(capped.len(), 3);
        assert_eq!(capped.total_bytes(), 48);
        assert!(capped.get(1, 0).is_none() && capped.get(1, 1).is_none());
        assert!(capped.get(1, 2).is_some() && capped.get(1, 4).is_some());

        let handles: Vec<_> = (0..4)
            .map(|t| {
                let c = Arc::clone(&cache);
                std::thread::spawn(move || {
                    for i in 0..50 {
                        let slot = (t * 50 + i) as i64;
                        assert!(c.insert_if_active(1, slot, vec![t as u8; 128]));
                        let _ = c.get(1, slot);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert!(cache.len() > 0 && cache.len() <= 64);
        assert!(cache.total_bytes() <= 4096);
        let mut counted_entries = 0usize;
        let mut counted_bytes = 0usize;
        for slot in 0..200 {
            if let Some(bytes) = cache.get(1, slot) {
                counted_entries += 1;
                counted_bytes += bytes.len();
            }
        }
        assert_eq!(counted_entries, cache.len());
        assert_eq!(counted_bytes, cache.total_bytes());
    }
}
