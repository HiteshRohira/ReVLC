//! Recently played media with resume positions and thumbnails, stored as
//! JSON in the app's data directory.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 60;
const SAVE_INTERVAL: Duration = Duration::from_secs(10);
/// Don't offer to resume within this many ms of the start or the end.
const RESUME_MARGIN: i64 = 15_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub uri: String,
    pub title: String,
    /// Last position in ms.
    pub position: i64,
    /// Duration in ms (0 when unknown).
    pub length: i64,
    /// Unix time of the last update, in seconds.
    pub updated: u64,
    pub finished: bool,
}

/// An entry as sent to the UI, with the path of its thumbnail if there is one.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiEntry {
    #[serde(flatten)]
    pub entry: Entry,
    pub thumb: Option<String>,
}

pub struct History {
    file: PathBuf,
    thumbs: PathBuf,
    /// Most recent first.
    entries: Vec<Entry>,
    dirty: bool,
    last_save: Instant,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl History {
    pub fn load(data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        let file = data_dir.join("history.json");
        let entries = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let thumbs = cache_dir.join("thumbs");
        let _ = std::fs::create_dir_all(&thumbs);
        let _ = std::fs::create_dir_all(&data_dir);
        Self { file, thumbs, entries, dirty: false, last_save: Instant::now() }
    }

    pub fn thumb_path(&self, uri: &str) -> PathBuf {
        self.thumbs.join(thumb_name(uri))
    }

    /// Thumbnail for the library grid (a fixed frame, not the resume point).
    pub fn library_thumb_path(&self, uri: &str) -> PathBuf {
        self.thumbs.join(format!("lib-{}", thumb_name(uri)))
    }

    fn find(&mut self, uri: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.uri == uri)
    }

    /// Marks `uri` as just opened, moving it to the front.
    pub fn touch(&mut self, uri: &str, title: &str) {
        let entry = match self.entries.iter().position(|e| e.uri == uri) {
            Some(i) => self.entries.remove(i),
            None => Entry {
                uri: uri.to_string(),
                title: title.to_string(),
                position: 0,
                length: 0,
                updated: 0,
                finished: false,
            },
        };
        self.entries.insert(0, Entry { updated: now(), ..entry });
        for old in self.entries.drain(MAX_ENTRIES.min(self.entries.len())..) {
            let _ = std::fs::remove_file(self.thumbs.join(thumb_name(&old.uri)));
        }
        self.dirty = true;
    }

    pub fn set_title(&mut self, uri: &str, title: &str) {
        if let Some(e) = self.find(uri) {
            e.title = title.to_string();
            self.dirty = true;
        }
    }

    pub fn update(&mut self, uri: &str, position: i64, length: i64) {
        if let Some(e) = self.find(uri) {
            e.position = position;
            if length > 0 {
                e.length = length;
            }
            e.finished = false;
            e.updated = now();
            self.dirty = true;
        }
    }

    pub fn finish(&mut self, uri: &str) {
        if let Some(e) = self.find(uri) {
            e.position = 0;
            e.finished = true;
            e.updated = now();
            self.dirty = true;
        }
    }

    pub fn remove(&mut self, uri: &str) {
        self.entries.retain(|e| e.uri != uri);
        let _ = std::fs::remove_file(self.thumb_path(uri));
        self.dirty = true;
    }

    /// Where to resume `uri`, if it was left part-way through.
    pub fn resume_point(&self, uri: &str) -> Option<i64> {
        let e = self.entries.iter().find(|e| e.uri == uri)?;
        let before_end = e.length == 0 || e.position < e.length - RESUME_MARGIN;
        (!e.finished && e.position > RESUME_MARGIN && before_end).then_some(e.position)
    }

    /// Writes the history if it changed (and, unless `force`, the last
    /// write was a while ago). Stays dirty on failure, so it's retried.
    pub fn save(&mut self, force: bool) -> Result<(), String> {
        if !self.dirty || (!force && self.last_save.elapsed() < SAVE_INTERVAL) {
            return Ok(());
        }
        self.last_save = Instant::now();
        crate::store::write_json(&self.file, &self.entries, false).inspect_err(|e| eprintln!("revlc: {e}"))?;
        self.dirty = false;
        Ok(())
    }

    pub fn list(&self) -> Vec<UiEntry> {
        self.entries
            .iter()
            .map(|e| UiEntry {
                entry: e.clone(),
                thumb: Some(self.thumb_path(&e.uri))
                    .filter(|p| p.exists())
                    .map(|p| p.to_string_lossy().into_owned()),
            })
            .collect()
    }
}

fn thumb_name(uri: &str) -> String {
    let mut h = DefaultHasher::new();
    uri.hash(&mut h);
    format!("{:016x}.jpg", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> History {
        let dir = std::env::temp_dir().join(format!("revlc-history-test-{}", std::process::id()));
        History::load(dir.join("data"), dir.join("cache"))
    }

    #[test]
    fn resume_only_mid_way() {
        let mut h = history();
        h.touch("a", "A");
        h.update("a", 5_000, 600_000);
        assert_eq!(h.resume_point("a"), None);
        h.update("a", 120_000, 600_000);
        assert_eq!(h.resume_point("a"), Some(120_000));
        h.update("a", 595_000, 600_000);
        assert_eq!(h.resume_point("a"), None);
        h.update("a", 120_000, 600_000);
        h.finish("a");
        assert_eq!(h.resume_point("a"), None);
    }

    #[test]
    fn touch_moves_to_front() {
        let mut h = history();
        h.touch("a", "A");
        h.touch("b", "B");
        h.touch("a", "A");
        assert_eq!(h.entries.iter().map(|e| e.uri.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    }
}
