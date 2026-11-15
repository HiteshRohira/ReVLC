//! Saved playlists: named lists of media the user keeps, stored as JSON in
//! the app's data directory. (The queue of what's playing is `playlist.rs`;
//! playlist files are `playlist_file.rs`.)

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::playlist::Item;

const MAX_NAME: usize = 120;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub uri: String,
    pub title: String,
    /// Per-item libVLC options, as in a playlist file: only the safe ones apply.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

impl From<Item> for Entry {
    fn from(item: Item) -> Self {
        Self { uri: item.uri, title: item.title, options: item.options }
    }
}

impl From<Entry> for Item {
    fn from(entry: Entry) -> Self {
        Self { uri: entry.uri, title: entry.title, options: entry.options, trusted: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub items: Vec<Entry>,
    /// When it last changed, Unix seconds.
    #[serde(default)]
    pub updated: u64,
}

pub struct Playlists {
    file: PathBuf,
    pub lists: Vec<Saved>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Playlists {
    pub fn load(data_dir: &Path) -> Self {
        let file = data_dir.join("playlists.json");
        let lists = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { file, lists }
    }

    /// Saves after `change`, putting the lists back as they were if the
    /// change or the save fails.
    fn update<T>(&mut self, change: impl FnOnce(&mut Vec<Saved>) -> Result<T, String>) -> Result<T, String> {
        let before = self.lists.clone();
        let result = change(&mut self.lists).and_then(|value| {
            crate::store::write_json(&self.file, &self.lists, false)?;
            Ok(value)
        });
        if result.is_err() {
            self.lists = before;
        }
        result
    }

    /// A name fit to save: trimmed, not empty, not too long, not taken by
    /// another playlist (`except` is the one being renamed).
    fn clean_name(lists: &[Saved], name: &str, except: Option<&str>) -> Result<String, String> {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() {
            return Err("Give the playlist a name".into());
        }
        if name.chars().count() > MAX_NAME {
            return Err("That name is too long".into());
        }
        if lists.iter().any(|l| Some(l.id.as_str()) != except && l.name.to_lowercase() == name.to_lowercase()) {
            return Err(format!("There's already a playlist called “{name}”"));
        }
        Ok(name)
    }

    /// Makes a playlist and returns its id. Newest first.
    pub fn create(&mut self, name: &str, items: Vec<Item>) -> Result<String, String> {
        self.update(|lists| {
            let name = Self::clean_name(lists, name, None)?;
            let id = uuid::Uuid::new_v4().to_string();
            lists.insert(0, Saved { id: id.clone(), name, items: items.into_iter().map(Entry::from).collect(), updated: now() });
            Ok(id)
        })
    }

    /// Like [`Playlists::create`], but a taken name gets a number: "Mix 2".
    pub fn create_numbered(&mut self, name: &str, items: Vec<Item>) -> Result<String, String> {
        let base = name.split_whitespace().collect::<Vec<_>>().join(" ");
        let base = if base.is_empty() { "Playlist".to_string() } else { base.chars().take(MAX_NAME - 8).collect() };
        let taken = |n: &str| self.lists.iter().any(|l| l.name.to_lowercase() == n.to_lowercase());
        let name = std::iter::once(base.clone()).chain((2..).map(|n| format!("{base} {n}"))).find(|n| !taken(n)).unwrap_or(base);
        self.create(&name, items)
    }

    fn find<'a>(lists: &'a mut [Saved], id: &str) -> Result<&'a mut Saved, String> {
        lists.iter_mut().find(|l| l.id == id).ok_or_else(|| "That playlist no longer exists".to_string())
    }

    pub fn rename(&mut self, id: &str, name: &str) -> Result<(), String> {
        self.update(|lists| {
            let name = Self::clean_name(lists, name, Some(id))?;
            let list = Self::find(lists, id)?;
            list.name = name;
            list.updated = now();
            Ok(())
        })
    }

    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        self.update(|lists| {
            lists.retain(|l| l.id != id);
            Ok(())
        })
    }

    pub fn add(&mut self, id: &str, items: Vec<Item>) -> Result<(), String> {
        self.update(|lists| {
            let list = Self::find(lists, id)?;
            list.items.extend(items.into_iter().map(Entry::from));
            list.updated = now();
            Ok(())
        })
    }

    pub fn remove_item(&mut self, id: &str, index: usize) -> Result<(), String> {
        self.update(|lists| {
            let list = Self::find(lists, id)?;
            if index < list.items.len() {
                list.items.remove(index);
                list.updated = now();
            }
            Ok(())
        })
    }

    /// A playlist's items, ready to queue.
    pub fn items(&self, id: &str) -> Result<Vec<Item>, String> {
        let list = self.lists.iter().find(|l| l.id == id).ok_or("That playlist no longer exists")?;
        Ok(list.items.iter().cloned().map(Item::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("revlc-playlists-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn item(uri: &str) -> Item {
        Item { uri: uri.into(), title: uri.into(), options: Vec::new(), trusted: false }
    }

    #[test]
    fn playlists_are_kept_between_launches() {
        let dir = dir("keep");
        let mut lists = Playlists::load(&dir);
        let road = lists.create("  Road   trip ", vec![item("/m/a.mp3"), item("/m/b.mp3")]).unwrap();
        let focus = lists.create("Focus", Vec::new()).unwrap();
        lists.add(&focus, vec![item("https://radio.example/stream")]).unwrap();
        lists.remove_item(&road, 0).unwrap();
        lists.rename(&road, "Road trip 2026").unwrap();

        let back = Playlists::load(&dir);
        let names: Vec<_> = back.lists.iter().map(|l| (l.name.as_str(), l.items.len())).collect();
        assert_eq!(names, [("Focus", 1), ("Road trip 2026", 1)]);
        assert_eq!(back.items(&road).unwrap()[0].uri, "/m/b.mp3");
        // Options saved with an item come back untrusted, as from a file.
        assert!(!back.items(&focus).unwrap()[0].trusted);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_must_be_usable_and_unique() {
        let dir = dir("names");
        let mut lists = Playlists::load(&dir);
        let id = lists.create("Mix", Vec::new()).unwrap();
        assert!(lists.create("   ", Vec::new()).is_err());
        assert!(lists.create("mix", Vec::new()).is_err());
        assert!(lists.create(&"x".repeat(MAX_NAME + 1), Vec::new()).is_err());
        // Renaming to its own name (or its case) is fine; to another's isn't.
        lists.rename(&id, "MIX").unwrap();
        let other = lists.create("Other", Vec::new()).unwrap();
        assert!(lists.rename(&other, "mix").is_err());
        assert!(lists.rename("gone", "New").is_err());
        // An imported file with a taken name gets a number.
        lists.create_numbered("Mix", Vec::new()).unwrap();
        lists.create_numbered("Mix", Vec::new()).unwrap();
        let names: Vec<_> = lists.lists.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Mix 3", "Mix 2", "Other", "MIX"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_saves_change_nothing() {
        let dir = dir("fail");
        // A folder where playlists.json should go makes every save fail.
        std::fs::create_dir_all(dir.join("playlists.json").join("x")).unwrap();
        let mut lists = Playlists::load(&dir);
        assert!(lists.create("Mix", Vec::new()).is_err());
        assert!(lists.lists.is_empty());
        std::fs::remove_dir_all(dir.join("playlists.json")).unwrap();
        let id = lists.create("Mix", vec![item("/a.mp3")]).unwrap();
        lists.delete(&id).unwrap();
        assert!(Playlists::load(&dir).lists.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
