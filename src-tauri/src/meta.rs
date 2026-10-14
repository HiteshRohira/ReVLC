//! Library metadata: titles, years and season/episode numbers worked out
//! from file and folder names, local poster files, and a cache of what
//! libVLC reads from the files themselves (tags, embedded artwork, duration).

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::vlc::MediaMeta;

/// What a file's name and folders say about it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NameInfo {
    /// The movie title, or the episode title when there is one.
    pub title: String,
    pub year: Option<u32>,
    pub show: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

/// Release-name words that end a title ("Movie.2019.1080p.BluRay.x264").
const JUNK: &[&str] = &[
    "480p", "576p", "720p", "1080p", "1080i", "2160p", "4k", "uhd", "bluray", "blu-ray", "brrip", "bdrip", "webrip",
    "web-dl", "webdl", "hdtv", "hdrip", "dvdrip", "remux", "x264", "x265", "h264", "h265", "hevc", "10bit", "hdr10",
    "proper", "repack", "aac", "ac3", "dts", "ddp5", "atmos", "xvid", "amzn", "subbed", "dubbed",
];

/// Splits a name into words: dots and underscores count as spaces when
/// the name has no real spaces ("The.Matrix.1999" → "The Matrix 1999").
fn words(name: &str) -> String {
    let name: String = if name.contains(' ') { name.replace('_', " ") } else { name.replace(['.', '_'], " ") };
    // Drop [bracketed] tags such as release groups or hashes.
    let mut out = String::new();
    let mut depth = 0;
    for c in name.chars() {
        match c {
            '[' | '{' => depth += 1,
            ']' | '}' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Trims separators and stray brackets from the ends of a title.
fn tidy(s: &str) -> String {
    s.trim_matches(|c: char| c.is_whitespace() || "-–—_.,:(".contains(c)).to_string()
}

/// Cuts `s` at the first release-name word.
fn cut_junk(s: &str) -> &str {
    let lower = s.to_ascii_lowercase();
    let mut pos = 0;
    for word in lower.split(' ') {
        let w = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-');
        if JUNK.contains(&w) {
            return &s[..pos];
        }
        pos += word.len() + 1;
    }
    s
}

fn number(s: &str) -> Option<u32> {
    s.parse().ok()
}

/// Finds "S01E02", "s1e2", "S01 E02" or "1x02". Returns (start, end, season, episode).
fn find_episode(s: &str) -> Option<(usize, usize, u32, u32)> {
    let b = s.as_bytes();
    let digits = |from: usize, max: usize| {
        let n = b[from..].iter().take(max).take_while(|c| c.is_ascii_digit()).count();
        (n > 0).then(|| (number(&s[from..from + n]).unwrap_or(0), from + n))
    };
    for i in 0..b.len() {
        let boundary = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        if !boundary {
            continue;
        }
        if b[i].eq_ignore_ascii_case(&b's') {
            if let Some((season, mut j)) = digits(i + 1, 2) {
                if j < b.len() && b[j] == b' ' {
                    j += 1;
                }
                if j < b.len() && b[j].eq_ignore_ascii_case(&b'e') {
                    if let Some((episode, end)) = digits(j + 1, 3) {
                        return Some((i, end, season, episode));
                    }
                }
            }
        } else if let Some((season, j)) = digits(i, 2) {
            if j < b.len() && (b[j] == b'x' || b[j] == b'X') {
                if let Some((episode, end)) = digits(j + 1, 3) {
                    if end == b.len() || !b[end].is_ascii_alphanumeric() {
                        return Some((i, end, season, episode));
                    }
                }
            }
        }
    }
    None
}

/// Finds a year (1900–2099) after the start of the title, optionally in
/// parentheses. Returns (start, year).
fn find_year(s: &str) -> Option<(usize, u32)> {
    let b = s.as_bytes();
    let mut found = None;
    for i in 1..b.len().saturating_sub(3) {
        let boundary_before = !b[i - 1].is_ascii_alphanumeric();
        let end = i + 4;
        let boundary_after = end == b.len() || !b[end].is_ascii_alphanumeric();
        if boundary_before && boundary_after && b[i..end].iter().all(u8::is_ascii_digit) {
            let year = number(&s[i..end])?;
            if (1900..2100).contains(&year) {
                // The last year wins: "2001 A Space Odyssey 1968".
                found = Some((i, year));
            }
        }
    }
    found
}

/// "Season 2", "Series 02", "S02" → 2.
fn season_folder(name: &str) -> Option<u32> {
    let lower = name.trim().to_ascii_lowercase();
    let rest = lower
        .strip_prefix("season")
        .or_else(|| lower.strip_prefix("series"))
        .or_else(|| lower.strip_prefix('s'))?;
    number(rest.trim_start_matches([' ', '.', '_', '-']))
}

/// "E05", "Episode 5", "Ep 5", or a leading "05 - " / "05. " → 5, plus what follows it.
fn find_episode_only(s: &str) -> Option<(u32, String)> {
    let lower = s.to_ascii_lowercase();
    for prefix in ["episode ", "ep ", "ep", "e"] {
        for (i, _) in lower.match_indices(prefix) {
            if i > 0 && lower.as_bytes()[i - 1].is_ascii_alphanumeric() {
                continue;
            }
            let rest = &s[i + prefix.len()..];
            let n = rest.bytes().take(3).take_while(u8::is_ascii_digit).count();
            if n > 0 && rest.as_bytes().get(n).is_none_or(|c| !c.is_ascii_alphanumeric()) {
                return Some((number(&rest[..n])?, tidy(&rest[n..])));
            }
        }
    }
    let n = s.bytes().take(3).take_while(u8::is_ascii_digit).count();
    let rest = &s[n..];
    if n > 0 && (rest.starts_with(" - ") || rest.starts_with(". ") || rest.starts_with(' ') || rest.is_empty()) {
        return Some((number(&s[..n])?, tidy(rest)));
    }
    None
}

/// A show or movie folder name without its year: "Dark (2017)" → "Dark".
fn folder_title(name: &str) -> String {
    let w = words(name);
    let w = cut_junk(&w);
    match find_year(w) {
        Some((i, _)) => tidy(&w[..i]),
        None => tidy(w),
    }
}

/// Works out a title, year and episode numbers from `path`'s name and folders.
pub fn parse_path(path: &Path) -> NameInfo {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = words(&stem);
    let parent = path.parent();
    let parent_name = parent.and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
    let folder_season = parent_name.as_deref().and_then(season_folder);
    // "Show/Season 1/…" names the show in the grandparent folder.
    let folder_show = match folder_season {
        Some(_) => parent.and_then(Path::parent).and_then(|p| p.file_name()),
        None => None,
    }
    .map(|n| folder_title(&n.to_string_lossy()))
    .filter(|s| !s.is_empty());

    if let Some((start, end, season, episode)) = find_episode(&name) {
        let show = tidy(cut_junk(&name[..start]));
        let show = if show.is_empty() { folder_show } else { Some(show) };
        let title = tidy(cut_junk(&name[end..]));
        return NameInfo { title, year: None, show, season: Some(season), episode: Some(episode) };
    }
    if let Some(season) = folder_season {
        let (episode, title) = match find_episode_only(&name) {
            Some((e, t)) => (Some(e), tidy(cut_junk(&t))),
            None => (None, tidy(cut_junk(&name))),
        };
        return NameInfo { title, year: None, show: folder_show, season: Some(season), episode };
    }
    let cut = cut_junk(&name);
    let (title, year) = match find_year(cut) {
        Some((i, year)) => (tidy(&cut[..i]), Some(year)),
        None => (tidy(cut), None),
    };
    // "Tears of Steel (2012)/Tears of Steel.mkv": the folder has the year.
    let year = year.or_else(|| {
        let folder = words(parent_name.as_deref()?);
        let (_, y) = find_year(&folder)?;
        (folder_title(&folder).eq_ignore_ascii_case(&title)).then_some(y)
    });
    NameInfo { title: if title.is_empty() { stem } else { title }, year, ..Default::default() }
}

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];

/// Image files per folder, read once per library scan.
#[derive(Default)]
pub struct Images(HashMap<PathBuf, Vec<PathBuf>>);

impl Images {
    /// The first image named `<name>.<jpg|jpeg|png>` in `dir`, any case.
    fn find(&mut self, dir: &Path, names: &[String]) -> Option<PathBuf> {
        let images = self.0.entry(dir.to_path_buf()).or_insert_with(|| {
            std::fs::read_dir(dir)
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| {
                            p.extension()
                                .and_then(|e| e.to_str())
                                .is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
                        })
                        .collect()
                })
                .unwrap_or_default()
        });
        names.iter().find_map(|name| {
            images
                .iter()
                .find(|p| p.file_stem().is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(name)))
                .cloned()
        })
    }

    /// A poster for one video: "<name>-poster.jpg" or "<name>.jpg" next to it.
    pub fn file_poster(&mut self, path: &Path) -> Option<PathBuf> {
        let stem = path.file_stem()?.to_string_lossy();
        self.find(path.parent()?, &[format!("{stem}-poster"), stem.to_string()])
    }

    /// A poster for a folder (a movie's folder, or a show), Kodi/Plex style.
    pub fn folder_poster(&mut self, dir: &Path) -> Option<PathBuf> {
        self.find(dir, &["poster", "folder", "cover", "show", "movie"].map(String::from))
    }

    /// The poster for the show or folder `path` belongs to. Episodes in a
    /// season folder use the season's poster, then the show's.
    pub fn group_poster(&mut self, path: &Path) -> Option<PathBuf> {
        let dir = path.parent()?;
        match dir.file_name().and_then(|n| season_folder(&n.to_string_lossy())) {
            Some(season) => {
                let show = dir.parent()?;
                self.find(show, &[format!("season{season:02}-poster"), format!("season{season}-poster")])
                    .or_else(|| self.folder_poster(dir))
                    .or_else(|| self.folder_poster(show))
            }
            None => self.folder_poster(dir),
        }
    }
}

pub fn modified(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// Where the resized copy of an artwork file is cached (named after the
/// source and its modification time, so edits make a new copy).
pub fn art_cache_path(thumbs: &Path, src: &Path) -> PathBuf {
    let mut h = DefaultHasher::new();
    src.hash(&mut h);
    modified(src).hash(&mut h);
    thumbs.join(format!("art-{:016x}.jpg", h.finish()))
}

/// Saves a copy of `src` at most `width` pixels wide as a JPEG.
pub fn resize_art(src: &Path, out: &Path, width: u32) -> Result<(), String> {
    let img = image::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let img = if img.width() > width { img.thumbnail(width, u32::MAX) } else { img };
    img.to_rgb8().save_with_format(out, image::ImageFormat::Jpeg).map_err(|e| e.to_string())
}

#[derive(Clone, Serialize, Deserialize)]
struct Cached {
    modified: u64,
    meta: MediaMeta,
}

/// What libVLC read from each library file, keyed by path and invalidated
/// when the file changes.
pub struct MetaCache {
    file: PathBuf,
    map: HashMap<String, Cached>,
    dirty: bool,
}

impl MetaCache {
    pub fn load(cache_dir: &Path) -> Self {
        let file = cache_dir.join("meta.json");
        let map = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { file, map, dirty: false }
    }

    /// Cached metadata, or `None` if the file is new or has changed.
    pub fn get(&self, path: &str) -> Option<&MediaMeta> {
        self.map.get(path).filter(|c| c.modified == modified(Path::new(path))).map(|c| &c.meta)
    }

    pub fn insert(&mut self, path: &str, meta: MediaMeta) {
        self.map.insert(path.to_string(), Cached { modified: modified(Path::new(path)), meta });
        self.dirty = true;
    }

    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        if let Ok(json) = serde_json::to_vec(&self.map) {
            let _ = std::fs::write(&self.file, json);
        }
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(path: &str) -> NameInfo {
        parse_path(Path::new(path))
    }

    #[test]
    fn movies() {
        let m = parse("/m/The.Matrix.1999.1080p.BluRay.x264-GROUP.mkv");
        assert_eq!((m.title.as_str(), m.year), ("The Matrix", Some(1999)));
        let m = parse("/m/Tears of Steel (2012)/Tears of Steel.mkv");
        assert_eq!((m.title.as_str(), m.year), ("Tears of Steel", Some(2012)));
        let m = parse("/m/2001 A Space Odyssey (1968).mkv");
        assert_eq!((m.title.as_str(), m.year), ("2001 A Space Odyssey", Some(1968)));
        let m = parse("/m/Open movies/Big Buck Bunny.mp4");
        assert_eq!((m.title.as_str(), m.year, m.show), ("Big Buck Bunny", None, None));
        let m = parse("/m/[Group] Some_Film_720p.mp4");
        assert_eq!(m.title, "Some Film");
    }

    #[test]
    fn episodes_from_file_names() {
        let e = parse("/tv/Dark.S01E02.Lies.1080p.WEB.mkv");
        assert_eq!(e, NameInfo { title: "Lies".into(), year: None, show: Some("Dark".into()), season: Some(1), episode: Some(2) });
        let e = parse("/tv/The Office - 2x05 - Halloween.avi");
        assert_eq!((e.show.as_deref(), e.season, e.episode, e.title.as_str()), (Some("The Office"), Some(2), Some(5), "Halloween"));
        let e = parse("/tv/Dark (2017)/Season 1/S01E03.mkv");
        assert_eq!((e.show.as_deref(), e.season, e.episode, e.title.as_str()), (Some("Dark"), Some(1), Some(3), ""));
    }

    #[test]
    fn episodes_from_folders() {
        let e = parse("/tv/Planet Earth/Season 02/03 - Jungles.mkv");
        assert_eq!((e.show.as_deref(), e.season, e.episode, e.title.as_str()), (Some("Planet Earth"), Some(2), Some(3), "Jungles"));
        let e = parse("/tv/Show/Series 1/Episode 4.mkv");
        assert_eq!((e.season, e.episode), (Some(1), Some(4)));
    }

    #[test]
    fn season_folders() {
        assert_eq!(season_folder("Season 01"), Some(1));
        assert_eq!(season_folder("S2"), Some(2));
        assert_eq!(season_folder("Specials"), None);
        assert_eq!(season_folder("Sintel"), None);
    }
}
