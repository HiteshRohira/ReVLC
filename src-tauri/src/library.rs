//! Library folders: a persisted list of directories whose media we show,
//! plus helpers to find media files inside folders.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

// Media VLC registers for on macOS, for folder scans. Leaves out
// extensions other files share (".ps" PostScript, ".rec", ".mod", ".it"):
// those still open from Finder, but a scan would pick up non-media.
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "m4v", "mkv", "webm", "mov", "qt", "avi", "ts", "m2ts", "mts", "m2t", "mt2s", "tp", "mpg", "mpeg", "m1v",
    "m2v", "m2p", "wmv", "flv", "f4v", "ogv", "ogm", "ogx", "3gp", "3g2", "vob", "vro", "evo", "divx", "rm",
    "rmvb", "asf", "dv", "mxf", "gxf", "wtv", "nsv", "nuv", "tod", "bik", "crf", "axv", "anx", "mks", "dav", "xesc",
];
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "m4a", "m4b", "aac", "ac3", "eac3", "flac", "wav", "ogg", "oga", "opus", "spx", "wma", "alac", "aif",
    "aiff", "aifc", "ape", "mka", "wv", "tta", "mpc", "dts", "mlp", "aob", "m2a", "mp1", "mp2", "mpa", "amr", "oma",
    "voc", "vqf", "axa", "mid", "midi", "rmi", "xm", "s3m",
];
pub const SUBTITLE_EXTENSIONS: &[&str] = &[
    "srt", "ass", "ssa", "vtt", "sub", "mpsub", "idx", "sup", "smi", "ttml", "usf", "utf", "rt", "aqt", "jss", "pjs",
    "psb",
];

/// Stop scanning after this many files so a huge folder can't stall the app.
const MAX_FILES: usize = 10_000;
/// ...or after looking at this many entries, media or not.
const MAX_ENTRIES: usize = 200_000;
const MAX_DEPTH: usize = 8;

fn has_ext(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

pub fn is_video(path: &Path) -> bool {
    has_ext(path, VIDEO_EXTENSIONS)
}

pub fn is_subtitle(path: &Path) -> bool {
    has_ext(path, SUBTITLE_EXTENSIONS)
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

/// All files under `dir` accepted by `keep`, in natural order. Folders
/// reached more than once through links are only listed the first time.
pub fn scan(dir: &Path, keep: fn(&Path) -> bool) -> Vec<PathBuf> {
    struct Walk {
        keep: fn(&Path) -> bool,
        out: Vec<PathBuf>,
        seen: HashSet<PathBuf>,
        entries: usize,
    }
    fn walk(dir: &Path, depth: usize, w: &mut Walk) {
        if !dir.canonicalize().is_ok_and(|real| w.seen.insert(real)) {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries.flatten().take(MAX_ENTRIES.saturating_sub(w.entries)).collect();
        w.entries += entries.len();
        entries.retain(|e| !e.file_name().to_string_lossy().starts_with('.'));
        entries.sort_by(|a, b| natural_cmp(&a.file_name().to_string_lossy(), &b.file_name().to_string_lossy()));
        for entry in entries {
            if w.out.len() >= MAX_FILES {
                return;
            }
            let path = entry.path();
            if path.is_dir() {
                if depth < MAX_DEPTH {
                    walk(&path, depth + 1, w);
                }
            // Empty files (a failed download or copy) can't play.
            } else if (w.keep)(&path) && entry.metadata().is_ok_and(|m| m.len() > 0) {
                w.out.push(path);
            }
        }
    }
    let mut w = Walk { keep, out: Vec::new(), seen: HashSet::new(), entries: 0 };
    walk(dir, 0, &mut w);
    w.out
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
        let saved: Vec<String> = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut folders: Vec<String> = Vec::new();
        for folder in saved.iter().map(|f| normalize(f)) {
            if !folders.contains(&folder) {
                folders.push(folder);
            }
        }
        Self { file, folders }
    }

    fn save(&self) -> Result<(), String> {
        crate::store::write_json(&self.file, &self.folders, false)
    }

    pub fn add(&mut self, folder: &str) -> Result<(), String> {
        let folder = normalize(folder);
        if self.folders.contains(&folder) {
            return Ok(());
        }
        let mut folders = self.folders.clone();
        folders.push(folder);
        folders.sort_by(|a, b| natural_cmp(a, b));
        self.replace(folders)
    }

    pub fn remove(&mut self, folder: &str) -> Result<(), String> {
        let folder = normalize(folder);
        let folders = self.folders.iter().filter(|f| **f != folder).cloned().collect();
        self.replace(folders)
    }

    /// Saves `folders` as the library, keeping the old list if that fails
    /// (so trying again saves again).
    fn replace(&mut self, folders: Vec<String>) -> Result<(), String> {
        let old = std::mem::replace(&mut self.folders, folders);
        self.save().inspect_err(|_| self.folders = old)
    }
}

/// A folder path written one way: the platform's separators, and no
/// trailing one (except for a drive's root).
fn normalize(folder: &str) -> String {
    let folder = crate::playlist_file::native_path(folder.trim().to_string());
    let trimmed = folder.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() || trimmed.ends_with(':') { folder } else { trimmed.to_string() }
}

/// Videos in the library `folders`. A file inside two of them (one inside
/// the other) belongs to the innermost one.
pub fn scan_folders(folders: &[String]) -> Vec<LibraryItem> {
    folders
        .iter()
        .flat_map(|root| {
            let root_path = Path::new(root);
            let inner: Vec<&Path> = folders.iter().map(Path::new).filter(|f| *f != root_path && f.starts_with(root_path)).collect();
            scan(root_path, is_video)
                .into_iter()
                .filter(move |path| !inner.iter().any(|f| path.starts_with(f)))
                .map(move |path| {
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
        .collect()
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
    fn nested_folders_list_files_once() {
        let dir = std::env::temp_dir().join(format!("revlc-library-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Show")).unwrap();
        std::fs::write(dir.join("Movie.mkv"), b"x").unwrap();
        std::fs::write(dir.join("Show").join("Episode 1.mkv"), b"x").unwrap();
        // Empty files are left out.
        std::fs::write(dir.join("Broken.mkv"), b"").unwrap();
        let root = dir.to_string_lossy().into_owned();
        let show = dir.join("Show").to_string_lossy().into_owned();
        let items = scan_folders(&[root.clone(), show.clone()]);
        let found: Vec<_> = items.iter().map(|i| (i.title.as_str(), i.root.as_str())).collect();
        assert_eq!(found, [("Movie", root.as_str()), ("Episode 1", show.as_str())]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn linked_folders_are_listed_once() {
        let dir = std::env::temp_dir().join(format!("revlc-library-links-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Show")).unwrap();
        std::fs::write(dir.join("Show").join("a.mkv"), b"x").unwrap();
        // A loop back to the top, and a second way into Show.
        std::os::unix::fs::symlink(&dir, dir.join("Show").join("loop")).unwrap();
        std::os::unix::fs::symlink(dir.join("Show"), dir.join("Again")).unwrap();
        let found = scan(&dir, is_video);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn failed_saves_change_nothing() {
        let dir = std::env::temp_dir().join(format!("revlc-library-save-{}", std::process::id()));
        // A folder where library.json should go makes every save fail.
        std::fs::create_dir_all(dir.join("library.json").join("x")).unwrap();
        let mut library = Library::load(&dir);
        assert!(library.add("/Movies").is_err());
        assert!(library.folders.is_empty());
        // Once saving works again, the same add saves.
        std::fs::remove_dir_all(dir.join("library.json")).unwrap();
        library.add("/Movies").unwrap();
        assert_eq!(Library::load(&dir).folders, [normalize("/Movies")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_paths_are_normalized() {
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(normalize("/Movies/"), format!("{sep}Movies"));
        assert_eq!(normalize("/"), sep.to_string());
        assert_eq!(normalize("C:\\"), "C:\\");
    }

    #[test]
    fn media_extensions() {
        assert!(is_video(Path::new("/x/Movie.MKV")));
        assert!(is_media(Path::new("song.flac")));
        assert!(!is_media(Path::new("notes.txt")));
        assert!(is_subtitle(Path::new("Movie.en.SRT")));
        assert!(!is_media(Path::new("Movie.srt")));
        assert!(!is_media(Path::new("paper.ps")));
    }
}
