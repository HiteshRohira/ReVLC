//! Reading and writing playlist files: M3U/M3U8, PLS and XSPF.

use std::collections::BTreeMap;
use std::path::Path;

use crate::playlist::{Item, percent_decode, title_for};

pub const EXTENSIONS: &[&str] = &["m3u", "m3u8", "pls", "xspf"];

pub fn is_playlist(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

fn ext(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase()
}

/// `file://` URI for an absolute path, percent-encoding everything but
/// unreserved characters and `/`. Windows paths (`C:\Videos\a.mkv`) become
/// `file:///C:/Videos/a.mkv`.
pub fn file_uri(path: &str) -> String {
    let mut uri = String::from("file://");
    let path = path.replace('\\', "/");
    let drive = has_drive(&path);
    if drive {
        uri.push('/');
    }
    for (i, b) in path.bytes().enumerate() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => uri.push(b as char),
            b':' if drive && i == 1 => uri.push(':'),
            _ => uri.push_str(&format!("%{b:02X}")),
        }
    }
    uri
}

/// A local path written the platform's way, so the same file always has
/// the same history and bookmark key (`C:/a.mkv` is `C:a.mkv` on Windows).
pub fn native_path(uri: String) -> String {
    if cfg!(windows) && !uri.contains("://") { uri.replace('/', "\\") } else { uri }
}

/// Whether a path starts with a Windows drive letter, like `C:/`.
fn has_drive(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// Turns an entry into something we can open: a path for local files
/// (relative ones are resolved against the playlist's folder), or a URL.
fn resolve(entry: &str, dir: &Path) -> String {
    let entry = entry.trim();
    if let Some(rest) = entry.strip_prefix("file://") {
        let path = percent_decode(rest.strip_prefix("localhost").unwrap_or(rest));
        // file:///C:/a.mkv is the path C:/a.mkv.
        return native_path(match path.strip_prefix('/') {
            Some(p) if has_drive(p) => p.to_string(),
            _ => path,
        });
    }
    if entry.contains("://") {
        return entry.to_string();
    }
    let path = Path::new(entry);
    native_path(if path.is_absolute() { entry.to_string() } else { dir.join(entry).to_string_lossy().into_owned() })
}

fn item(entry: &str, title: Option<String>, options: Vec<String>, dir: &Path) -> Item {
    let uri = resolve(entry, dir);
    let title = title.filter(|t| !t.trim().is_empty()).unwrap_or_else(|| title_for(&uri));
    Item { uri, title, options }
}

/// Reads a playlist file.
pub fn read(path: &Path) -> Result<Vec<Item>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_start_matches('\u{feff}');
    let dir = path.parent().unwrap_or(Path::new("/"));
    let items = match ext(path).as_str() {
        "pls" => read_pls(text, dir),
        "xspf" => read_xspf(text, dir),
        _ => read_m3u(text, dir),
    };
    Ok(items)
}

fn read_m3u(text: &str, dir: &Path) -> Vec<Item> {
    let mut items = Vec::new();
    let mut title = None;
    let mut options = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            title = info.split_once(',').map(|(_, t)| t.trim().to_string());
        } else if let Some(opt) = line.strip_prefix("#EXTVLCOPT:") {
            options.push(format!(":{}", opt.trim().trim_start_matches(':')));
        } else if !line.starts_with('#') {
            items.push(item(line, title.take(), std::mem::take(&mut options), dir));
        }
    }
    items
}

fn read_pls(text: &str, dir: &Path) -> Vec<Item> {
    let mut entries: BTreeMap<u32, (Option<String>, Option<String>)> = BTreeMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        if let Some(n) = key.strip_prefix("file").and_then(|n| n.parse().ok()) {
            entries.entry(n).or_default().0 = Some(value);
        } else if let Some(n) = key.strip_prefix("title").and_then(|n| n.parse().ok()) {
            entries.entry(n).or_default().1 = Some(value);
        }
    }
    entries
        .into_values()
        .filter_map(|(file, title)| Some(item(&file?, title, Vec::new(), dir)))
        .collect()
}

/// The text inside the first `<tag>…</tag>` in `xml`.
fn element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{tag}>"))? + start;
    Some(&xml[start..end])
}

fn read_xspf(text: &str, dir: &Path) -> Vec<Item> {
    let Some(list) = element(text, "trackList") else { return Vec::new() };
    list.split("<track>")
        .skip(1)
        .filter_map(|track| {
            let track = track.split("</track>").next()?;
            let location = unescape(element(track, "location")?);
            let title = element(track, "title").map(unescape);
            let options = track
                .split("<vlc:option>")
                .skip(1)
                .filter_map(|o| o.split("</vlc:option>").next())
                .map(|o| format!(":{}", unescape(o).trim_start_matches(':')))
                .collect();
            Some(item(&location, title, options, dir))
        })
        .collect()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn unescape(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix("<![CDATA[").and_then(|s| s.strip_suffix("]]>")).unwrap_or(s);
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp..];
        let Some(semi) = after.find(';').filter(|&i| i <= 10) else {
            out.push('&');
            rest = &after[1..];
            continue;
        };
        let entity = &after[1..semi];
        let ch = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .map(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').map(|d| d.parse().ok()))
                .flatten()
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Writes `items` to `path`, in the format its extension names (M3U by default).
pub fn write(path: &Path, items: &[Item]) -> Result<(), String> {
    let text = match ext(path).as_str() {
        "pls" => write_pls(items),
        "xspf" => write_xspf(items),
        _ => write_m3u(items),
    };
    std::fs::write(path, text).map_err(|e| format!("Could not save {}: {e}", path.display()))
}

fn write_m3u(items: &[Item]) -> String {
    let mut out = String::from("#EXTM3U\n");
    for it in items {
        out.push_str(&format!("#EXTINF:-1,{}\n", it.title.replace('\n', " ")));
        for opt in &it.options {
            out.push_str(&format!("#EXTVLCOPT:{}\n", opt.trim_start_matches(':')));
        }
        out.push_str(&it.uri);
        out.push('\n');
    }
    out
}

fn write_pls(items: &[Item]) -> String {
    let mut out = String::from("[playlist]\n");
    for (i, it) in items.iter().enumerate() {
        out.push_str(&format!("File{n}={}\nTitle{n}={}\n", it.uri, it.title.replace('\n', " "), n = i + 1));
    }
    out.push_str(&format!("NumberOfEntries={}\nVersion=2\n", items.len()));
    out
}

fn write_xspf(items: &[Item]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <playlist xmlns=\"http://xspf.org/ns/0/\" xmlns:vlc=\"http://www.videolan.org/vlc/playlist/ns/0/\" version=\"1\">\n\
         \t<title>Playlist</title>\n\t<trackList>\n",
    );
    for it in items {
        let location = if it.uri.contains("://") { it.uri.clone() } else { file_uri(&it.uri) };
        out.push_str(&format!(
            "\t\t<track>\n\t\t\t<location>{}</location>\n\t\t\t<title>{}</title>\n",
            escape(&location),
            escape(&it.title)
        ));
        if !it.options.is_empty() {
            out.push_str("\t\t\t<extension application=\"http://www.videolan.org/vlc/playlist/0\">\n");
            for opt in &it.options {
                out.push_str(&format!("\t\t\t\t<vlc:option>{}</vlc:option>\n", escape(opt.trim_start_matches(':'))));
            }
            out.push_str("\t\t\t</extension>\n");
        }
        out.push_str("\t\t</track>\n");
    }
    out.push_str("\t</trackList>\n</playlist>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An absolute path on this platform.
    const MOVIE: &str = if cfg!(windows) { r"C:\Movies\A & B.mkv" } else { "/Movies/A & B.mkv" };

    /// `name` in `dir`, written the way this platform does.
    fn joined(dir: &str, name: &str) -> String {
        native_path(Path::new(dir).join(name).to_string_lossy().into_owned())
    }

    fn items() -> Vec<Item> {
        vec![
            Item { uri: MOVIE.into(), title: "A & B".into(), options: vec![] },
            Item { uri: "https://example.com/live.m3u8".into(), title: "Live".into(), options: vec![] },
            Item { uri: "cdda:///dev/rdisk4".into(), title: "Track 3".into(), options: vec![":cdda-track=3".into()] },
        ]
    }

    fn round_trip(ext: &str) {
        let dir = std::env::temp_dir().join(format!("revlc-pl-{}-{ext}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("list.{ext}"));
        let mut expected = items();
        if ext == "pls" {
            // PLS has no per-item options.
            expected[2].options.clear();
        }
        write(&path, &expected).unwrap();
        assert_eq!(read(&path).unwrap(), expected, "{ext}");
    }

    #[test]
    fn formats_round_trip() {
        for ext in ["m3u", "m3u8", "pls", "xspf"] {
            round_trip(ext);
        }
    }

    #[test]
    fn relative_entries_and_file_uris() {
        let dir = Path::new("/Music/Album");
        let list = read_m3u("#EXTM3U\n01 Intro.mp3\n#EXTINF:12,Second\nfile:///Music/Other%20Song.flac\n", dir);
        assert_eq!(list[0].uri, joined("/Music/Album", "01 Intro.mp3"));
        assert_eq!(list[0].title, "01 Intro.mp3");
        assert_eq!(list[1].uri, native_path("/Music/Other Song.flac".into()));
        assert_eq!(list[1].title, "Second");
    }

    #[test]
    fn windows_paths() {
        assert_eq!(file_uri(r"C:\My Videos\a.mkv"), "file:///C:/My%20Videos/a.mkv");
        let expected = native_path("C:/My Videos/a.mkv".into());
        assert_eq!(resolve("file:///C:/My%20Videos/a.mkv", Path::new("/x")), expected);
        if cfg!(windows) {
            assert_eq!(expected, r"C:\My Videos\a.mkv");
        }
    }

    #[test]
    fn pls_and_xspf_from_other_players() {
        let pls = "[playlist]\nNumberOfEntries=2\nFile2=b.mp3\nFile1=http://radio.example/stream\nTitle1=Radio\n";
        let list = read_pls(pls, Path::new("/x"));
        assert_eq!(list.iter().map(|i| i.uri.as_str()).collect::<Vec<_>>(), ["http://radio.example/stream".to_string(), joined("/x", "b.mp3")]);
        let xspf = r#"<?xml version="1.0"?><playlist><trackList>
            <track><title>It&apos;s &#x41;</title><location>file:///a/It%27s.mp4</location></track>
            <track><location>b.mp4</location></track></trackList></playlist>"#;
        let list = read_xspf(xspf, Path::new("/x"));
        assert_eq!(list[0].title, "It's A");
        assert_eq!(list[0].uri, native_path("/a/It's.mp4".into()));
        assert_eq!(list[1].uri, joined("/x", "b.mp4"));
    }
}
