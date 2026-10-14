//! Bookmarks: named positions in a file, stored per URI as JSON in the
//! app's data directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    /// Position in ms.
    pub time: i64,
    pub name: String,
}

pub struct Bookmarks {
    file: PathBuf,
    map: BTreeMap<String, Vec<Bookmark>>,
}

impl Bookmarks {
    pub fn load(data_dir: &Path) -> Self {
        let file = data_dir.join("bookmarks.json");
        let map = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { file, map }
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_vec(&self.map) {
            let _ = std::fs::write(&self.file, json);
        }
    }

    pub fn list(&self, uri: &str) -> Vec<Bookmark> {
        self.map.get(uri).cloned().unwrap_or_default()
    }

    /// Adds a bookmark at `time`, named "Bookmark N" unless a name is given.
    /// A bookmark within a second of an existing one replaces it.
    pub fn add(&mut self, uri: &str, time: i64, name: Option<String>) {
        let list = self.map.entry(uri.to_string()).or_default();
        list.retain(|b| (b.time - time).abs() >= 1000);
        let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("Bookmark {}", list.len() + 1));
        list.push(Bookmark { time, name });
        list.sort_by_key(|b| b.time);
        self.save();
    }

    pub fn remove(&mut self, uri: &str, time: i64) {
        if let Some(list) = self.map.get_mut(uri) {
            list.retain(|b| b.time != time);
            if list.is_empty() {
                self.map.remove(uri);
            }
            self.save();
        }
    }

    pub fn rename(&mut self, uri: &str, time: i64, name: &str) {
        if let Some(b) = self.map.get_mut(uri).and_then(|l| l.iter_mut().find(|b| b.time == time)) {
            b.name = name.trim().to_string();
            self.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_sorts_and_replaces_nearby() {
        let dir = std::env::temp_dir().join(format!("revlc-bookmarks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut b = Bookmarks::load(&dir);
        b.add("a", 60_000, None);
        b.add("a", 10_000, Some("Intro".into()));
        b.add("a", 60_400, Some("Fight".into()));
        let list = b.list("a");
        assert_eq!(list.iter().map(|b| (b.time, b.name.as_str())).collect::<Vec<_>>(), [(10_000, "Intro"), (60_400, "Fight")]);
        b.rename("a", 10_000, "Opening");
        b.remove("a", 60_400);
        assert_eq!(Bookmarks::load(&dir).list("a"), [Bookmark { time: 10_000, name: "Opening".into() }]);
    }
}
