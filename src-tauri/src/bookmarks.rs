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
    /// What happened to an unreadable bookmarks file, to tell the user.
    pub damaged: Option<crate::store::Damaged>,
}

impl Bookmarks {
    pub fn load(data_dir: &Path) -> Self {
        let file = data_dir.join("bookmarks.json");
        let (map, damaged) = match crate::store::read_json(&file) {
            Ok(map) => (map.unwrap_or_default(), None),
            Err(damaged) => (BTreeMap::new(), Some(damaged)),
        };
        Self { file, map, damaged }
    }

    fn save(&self) -> Result<(), String> {
        crate::store::write_json_durable(&self.file, &self.map, false)
    }

    pub fn list(&self, uri: &str) -> Vec<Bookmark> {
        self.map.get(uri).cloned().unwrap_or_default()
    }

    /// Adds a bookmark at `time`, named "Bookmark N" unless a name is given.
    /// A bookmark within a second of an existing one replaces it.
    pub fn add(&mut self, uri: &str, time: i64, name: Option<String>) -> Result<(), String> {
        let list = self.map.entry(uri.to_string()).or_default();
        list.retain(|b| (b.time - time).abs() >= 1000);
        let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("Bookmark {}", list.len() + 1));
        list.push(Bookmark { time, name });
        list.sort_by_key(|b| b.time);
        self.save()
    }

    pub fn remove(&mut self, uri: &str, time: i64) -> Result<(), String> {
        let Some(list) = self.map.get_mut(uri) else { return Ok(()) };
        list.retain(|b| b.time != time);
        if list.is_empty() {
            self.map.remove(uri);
        }
        self.save()
    }

    pub fn rename(&mut self, uri: &str, time: i64, name: &str) -> Result<(), String> {
        let Some(b) = self.map.get_mut(uri).and_then(|l| l.iter_mut().find(|b| b.time == time)) else { return Ok(()) };
        b.name = name.trim().to_string();
        self.save()
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
        b.add("a", 60_000, None).unwrap();
        b.add("a", 10_000, Some("Intro".into())).unwrap();
        b.add("a", 60_400, Some("Fight".into())).unwrap();
        let list = b.list("a");
        assert_eq!(list.iter().map(|b| (b.time, b.name.as_str())).collect::<Vec<_>>(), [(10_000, "Intro"), (60_400, "Fight")]);
        b.rename("a", 10_000, "Opening").unwrap();
        b.remove("a", 60_400).unwrap();
        assert_eq!(Bookmarks::load(&dir).list("a"), [Bookmark { time: 10_000, name: "Opening".into() }]);
    }

    #[test]
    fn a_damaged_file_is_kept_aside() {
        let dir = std::env::temp_dir().join(format!("revlc-bookmarks-damaged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bookmarks.json"), b"{\"a\": [").unwrap();
        let mut b = Bookmarks::load(&dir);
        assert!(b.list("a").is_empty());
        let aside = b.damaged.clone().and_then(|d| d.aside).expect("moved aside");
        assert_eq!(std::fs::read(&aside).unwrap(), b"{\"a\": [");
        b.add("a", 1000, None).unwrap();
        let again = Bookmarks::load(&dir);
        assert_eq!(again.list("a").len(), 1);
        assert_eq!(again.damaged, None);
        assert!(aside.exists(), "the save overwrote the damaged file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
