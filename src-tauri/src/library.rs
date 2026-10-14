//! Library folders: a persisted list of directories whose media we show,
//! plus helpers to find media files inside folders.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use serde::Serialize;

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mkv", "webm", "mov", "avi", "ts", "m2ts", "mts", "mpg", "mpeg", "wmv", "flv", "ogv", "3gp", "vob",
    "divx", "rmvb", "f4v", "asf",
];
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", "wma", "alac", "aiff", "ape", "mka",
];

/// Stop scanning after this many files so a huge folder can't stall the app.
const MAX_FILES: usize = 10_000;
const MAX_DEPTH: usize = 8;

fn has_ext(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

pub fn is_video(path: &Path) -> bool {
    has_ext(path, VIDEO_EXTENSIONS)
}

pub fn is_media(path: &Path) -> bool {
    is_video(path) || has_ext(path, AUDIO_EXTENSIONS)
}

/// Compares names so "Episode 2" sorts before "Episode 10".
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut n = String::new();
                    while let Some(c) = it.peek().filter(|c| c.is_ascii_digit()) {
                        n.push(*c);
                        it.next();
                    }
                    n
                };
                let (na, nb) = (take(&mut a), take(&mut b));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// All files under `dir` accepted by `keep`, in natural order.
pub fn scan(dir: &Path, keep: fn(&Path) -> bool) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, keep: fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        entries.sort_by(|a, b| natural_cmp(&a.file_name().to_string_lossy(), &b.file_name().to_string_lossy()));
        for entry in entries {
            if out.len() >= MAX_FILES {
                return;
            }
            let path = entry.path();
            if path.is_dir() {
                if depth < MAX_DEPTH {
                    walk(&path, depth + 1, keep, out);
                }
            } else if keep(&path) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, 0, keep, &mut out);
    out
}

/// Replaces folders in `uris` with the media files inside them.
pub fn expand(uris: Vec<String>) -> Vec<String> {
    uris.into_iter()
        .flat_map(|uri| {
            let path = Path::new(&uri);
            if !uri.contains("://") && path.is_dir() {
                scan(path, is_media).into_iter().map(|p| p.to_string_lossy().into_owned()).collect()
            } else {
                vec![uri]
            }
        })
        .collect()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    pub uri: String,
    pub title: String,
    /// The library folder this file was found in.
    pub root: String,
    /// The file's folder, relative to `root` ("" when directly inside it).
    pub folder: String,
    /// Modification time, Unix seconds.
    pub modified: u64,
    pub thumb: Option<String>,
    /// A clean title: the movie's name, or the episode's title.
    pub name: String,
    pub year: Option<u32>,
    pub show: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    /// Milliseconds, 0 until the file has been read.
    pub duration: i64,
    /// This video's poster or embedded cover (a cached copy).
    pub art: Option<String>,
    /// The poster of its show, season or folder (a cached copy).
    pub group_art: Option<String>,
}

pub struct Library {
    file: PathBuf,
    pub folders: Vec<String>,
}

impl Library {
    pub fn load(data_dir: &Path) -> Self {
        let file = data_dir.join("library.json");
        let folders = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { file, folders }
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_vec(&self.folders) {
            let _ = std::fs::write(&self.file, json);
        }
    }

    pub fn add(&mut self, folder: &str) {
        if !self.folders.iter().any(|f| f == folder) {
            self.folders.push(folder.to_string());
            self.folders.sort_by(|a, b| natural_cmp(a, b));
            self.save();
        }
    }

    pub fn remove(&mut self, folder: &str) {
        self.folders.retain(|f| f != folder);
        self.save();
    }

    /// Videos in every library folder. `thumb` maps a file to its cached
    /// thumbnail path, if one exists.
    pub fn items(&self, thumb: impl Fn(&str) -> Option<String>) -> Vec<LibraryItem> {
        self.folders
            .iter()
            .flat_map(|root| {
                let root_path = Path::new(root);
                scan(root_path, is_video).into_iter().map(move |path| {
                    let uri = path.to_string_lossy().into_owned();
                    let folder = path
                        .parent()
                        .and_then(|p| p.strip_prefix(root_path).ok())
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    LibraryItem {
                        name: title.clone(),
                        title,
                        root: root.clone(),
                        folder,
                        modified: crate::meta::modified(&path),
                        thumb: None,
                        uri,
                        year: None,
                        show: None,
                        season: None,
                        episode: None,
                        duration: 0,
                        art: None,
                        group_art: None,
                    }
                })
            })
            .map(|mut item| {
                item.thumb = thumb(&item.uri);
                item
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["Episode 10.mkv", "episode 2.mkv", "Episode 1.mkv", "Extras"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["Episode 1.mkv", "episode 2.mkv", "Episode 10.mkv", "Extras"]);
        assert_eq!(natural_cmp("S01E02", "S01E02"), Ordering::Equal);
        assert_eq!(natural_cmp("a007", "a7"), Ordering::Equal);
    }

    #[test]
    fn media_extensions() {
        assert!(is_video(Path::new("/x/Movie.MKV")));
        assert!(is_media(Path::new("song.flac")));
        assert!(!is_media(Path::new("notes.txt")));
    }
}
