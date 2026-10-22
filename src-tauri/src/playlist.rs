use serde::{Deserialize, Serialize};

use crate::library::natural_cmp;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Item {
    pub uri: String,
    pub title: String,
    /// Per-item libVLC options, such as ":cdda-track=3".
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// Whether ReVLC made `options` itself (a disc or capture device), so
    /// libVLC should apply unsafe ones too. Not for options read from files.
    #[serde(skip)]
    pub trusted: bool,
}

impl Item {
    pub fn new(uri: impl Into<String>) -> Self {
        let uri = uri.into();
        let title = if uri.contains("://") { title_for(&uri) } else { crate::meta::display_title(std::path::Path::new(&uri)) };
        Self { title, uri, options: Vec::new(), trusted: false }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortKey {
    Title,
    Path,
    Reverse,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Playlist {
    pub items: Vec<Item>,
    pub current: Option<usize>,
    pub repeat: Repeat,
    pub shuffle: bool,
}

/// A readable title for a path or URL until the media reports its own.
pub fn title_for(uri: &str) -> String {
    let trimmed = uri.trim_end_matches('/');
    let last = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    let last = last.split(['?', '#']).next().unwrap_or(last);
    if last.is_empty() { uri.to_string() } else { percent_decode(last) }
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(b) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Playlist {
    /// Appends items and returns the index of the first one added.
    pub fn add(&mut self, items: Vec<Item>) -> usize {
        let first = self.items.len();
        self.items.extend(items);
        first
    }

    /// Sorts the items, keeping the current one current.
    pub fn sort(&mut self, key: SortKey) {
        let mut items: Vec<(usize, Item)> = std::mem::take(&mut self.items).into_iter().enumerate().collect();
        match key {
            SortKey::Title => items.sort_by(|a, b| natural_cmp(&a.1.title, &b.1.title)),
            SortKey::Path => items.sort_by(|a, b| natural_cmp(&a.1.uri, &b.1.uri)),
            SortKey::Reverse => items.reverse(),
        }
        self.current = self.current.and_then(|c| items.iter().position(|(i, _)| *i == c));
        self.items = items.into_iter().map(|(_, item)| item).collect();
    }

    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.items.len() {
            return false;
        }
        self.items.remove(index);
        let removed_current = self.current == Some(index);
        self.current = match self.current {
            Some(c) if c == index => None,
            Some(c) if c > index => Some(c - 1),
            c => c,
        };
        removed_current
    }

    /// Puts `items` in place of the item at `index` (a playlist URL, by
    /// what it lists).
    pub fn replace(&mut self, index: usize, items: Vec<Item>) {
        if index >= self.items.len() || items.is_empty() {
            return;
        }
        let added = items.len();
        self.items.splice(index..=index, items);
        if let Some(c) = self.current.filter(|&c| c > index) {
            self.current = Some(c + added - 1);
        }
    }

    pub fn move_item(&mut self, from: usize, to: usize) {
        if from >= self.items.len() || to >= self.items.len() || from == to {
            return;
        }
        let item = self.items.remove(from);
        self.items.insert(to, item);
        if let Some(c) = self.current {
            self.current = Some(if c == from {
                to
            } else if from < c && to >= c {
                c - 1
            } else if from > c && to <= c {
                c + 1
            } else {
                c
            });
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.current = None;
    }

    /// The item to play after the current one. `auto` is true when the
    /// current item ended by itself (so "repeat one" applies).
    pub fn next_index(&self, auto: bool) -> Option<usize> {
        let len = self.items.len();
        if len == 0 {
            return None;
        }
        let Some(cur) = self.current else { return Some(0) };
        if auto && self.repeat == Repeat::One {
            return Some(cur);
        }
        if self.shuffle && len > 1 {
            let mut pick = random_index(len);
            if pick == cur {
                pick = (pick + 1) % len;
            }
            return Some(pick);
        }
        if cur + 1 < len {
            Some(cur + 1)
        } else if self.repeat != Repeat::Off {
            Some(0)
        } else {
            None
        }
    }

    pub fn prev_index(&self) -> Option<usize> {
        let len = self.items.len();
        match self.current {
            _ if len == 0 => None,
            Some(0) if self.repeat != Repeat::Off => Some(len - 1),
            Some(0) | None => Some(0),
            Some(c) => Some(c - 1),
        }
    }
}

fn random_index(len: usize) -> usize {
    use std::hash::{BuildHasher, RandomState};
    (RandomState::new().hash_one(std::time::SystemTime::now()) % len as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_keeps_current() {
        let mut p = Playlist::default();
        p.add(["/a.mkv", "http://x/list.m3u", "/b.mkv"].map(Item::new).to_vec());
        p.current = Some(2);
        p.replace(1, ["/1.mkv", "/2.mkv"].map(Item::new).to_vec());
        let uris: Vec<_> = p.items.iter().map(|i| i.uri.as_str()).collect();
        assert_eq!(uris, ["/a.mkv", "/1.mkv", "/2.mkv", "/b.mkv"]);
        assert_eq!(p.current, Some(3));
    }

    fn list(n: usize) -> Playlist {
        let mut p = Playlist::default();
        p.add((0..n).map(|i| Item::new(format!("/m/{i}.mkv"))).collect());
        p
    }

    #[test]
    fn sort_keeps_current() {
        let mut p = Playlist::default();
        p.add(["/b/Episode 10.mkv", "/a/Episode 2.mkv", "/c/Episode 1.mkv"].map(Item::new).to_vec());
        p.current = Some(0);
        p.sort(SortKey::Title);
        let titles: Vec<_> = p.items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Episode 1.mkv", "Episode 2.mkv", "Episode 10.mkv"]);
        assert_eq!(p.current, Some(2));
        p.sort(SortKey::Path);
        assert_eq!(p.items[0].uri, "/a/Episode 2.mkv");
        assert_eq!(p.current, Some(1));
        p.sort(SortKey::Reverse);
        assert_eq!(p.current, Some(1));
        assert_eq!(p.items[0].uri, "/c/Episode 1.mkv");
    }

    #[test]
    fn titles() {
        assert_eq!(title_for("/Users/a/Movies/My%20Film.mkv"), "My Film.mkv");
        assert_eq!(title_for("https://example.com/live/stream.m3u8?token=1"), "stream.m3u8");
        assert_eq!(title_for("https://example.com/"), "example.com");
    }

    #[test]
    fn next_and_repeat() {
        let mut p = list(3);
        p.current = Some(2);
        assert_eq!(p.next_index(true), None);
        p.repeat = Repeat::All;
        assert_eq!(p.next_index(true), Some(0));
        p.repeat = Repeat::One;
        assert_eq!(p.next_index(true), Some(2));
        assert_eq!(p.next_index(false), Some(0));
    }

    #[test]
    fn move_keeps_current() {
        let mut p = list(4);
        p.current = Some(1);
        p.move_item(0, 3);
        assert_eq!(p.current, Some(0));
        p.move_item(0, 2);
        assert_eq!(p.current, Some(2));
        p.move_item(3, 0);
        assert_eq!(p.current, Some(3));
    }

    #[test]
    fn remove_adjusts_current() {
        let mut p = list(3);
        p.current = Some(2);
        assert!(!p.remove(0));
        assert_eq!(p.current, Some(1));
        assert!(p.remove(1));
        assert_eq!(p.current, None);
    }
}
