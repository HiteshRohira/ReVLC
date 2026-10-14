//! Optical discs and disc images: finding inserted discs and turning a
//! disc into playlist items.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::library::natural_cmp;
use crate::playlist::{Item, title_for};
use crate::playlist_file::file_uri;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscKind {
    Dvd,
    Bluray,
    Cd,
    Vcd,
}

#[derive(Clone, Debug, Serialize)]
pub struct Disc {
    pub kind: DiscKind,
    pub name: String,
    pub path: String,
}

fn has_dir(path: &Path, name: &str) -> bool {
    path.join(name).is_dir() || path.join(name.to_ascii_lowercase()).is_dir()
}

/// What kind of disc a mounted volume or folder holds, if any.
pub fn kind_of(path: &Path) -> Option<DiscKind> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_uppercase()).unwrap_or_default();
    if has_dir(path, "VIDEO_TS") || name == "VIDEO_TS" {
        Some(DiscKind::Dvd)
    } else if has_dir(path, "BDMV") || name == "BDMV" {
        Some(DiscKind::Bluray)
    } else if has_dir(path, "MPEGAV") || has_dir(path, "MPEG2") {
        Some(DiscKind::Vcd)
    } else if path.join(".TOC.plist").is_file() || has_cda(path) {
        // macOS mounts audio CDs as a volume of AIFF tracks with a table of
        // contents; Windows shows them as .cda track files.
        Some(DiscKind::Cd)
    } else {
        None
    }
}

fn has_cda(path: &Path) -> bool {
    cfg!(windows) && !files(path, &["cda"]).is_empty()
}

/// Discs currently inserted: optical drives with a disc in them.
#[cfg(windows)]
pub fn detect() -> Vec<Disc> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW};
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_CDROM;

    let mut discs = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        let wide: Vec<u16> = root.encode_utf16().chain([0]).collect();
        if unsafe { GetDriveTypeW(wide.as_ptr()) } != DRIVE_CDROM {
            continue;
        }
        let mut label = [0u16; 261];
        // Fails when the drive is empty.
        let ok = unsafe {
            GetVolumeInformationW(wide.as_ptr(), label.as_mut_ptr(), label.len() as u32, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0)
        };
        if ok == 0 {
            continue;
        }
        let path = Path::new(&root);
        let Some(kind) = kind_of(path) else { continue };
        let len = label.iter().position(|&c| c == 0).unwrap_or(0);
        let label = String::from_utf16_lossy(&label[..len]);
        discs.push(Disc { kind, name: if label.is_empty() { root.clone() } else { label }, path: root });
    }
    discs
}

/// Discs currently inserted (mounted volumes that look like one).
#[cfg(not(windows))]
pub fn detect() -> Vec<Disc> {
    let Ok(entries) = std::fs::read_dir("/Volumes") else { return Vec::new() };
    let mut discs: Vec<Disc> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            Some(Disc {
                kind: kind_of(&path)?,
                name: e.file_name().to_string_lossy().into_owned(),
                path: path.to_string_lossy().into_owned(),
            })
        })
        .collect();
    discs.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    discs
}

/// Files in `dir` with one of `exts`, in natural order.
fn files(dir: &Path, exts: &[&str]) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<String> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
        })
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    out.sort_by(|a, b| natural_cmp(a, b));
    out
}

/// A display name for a disc at `path` (its volume or image name).
fn disc_name(path: &Path) -> String {
    let p = if path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("VIDEO_TS") || n.eq_ignore_ascii_case("BDMV")) {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| title_for(&p.to_string_lossy()))
}

/// Playlist items for the disc at `path` (a mounted volume, a folder or a
/// disc image). `menus` uses DVD and Blu-ray menus instead of playing the
/// main title directly.
pub fn items(kind: DiscKind, path: &str, menus: bool) -> Result<Vec<Item>, String> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(format!("{path} doesn't exist"));
    }
    // libVLC decodes the location like a file URI, so encode the path the same way.
    let location = file_uri(path).trim_start_matches("file://").to_string();
    let name = disc_name(p);
    let items = match kind {
        DiscKind::Dvd => {
            let scheme = if menus { "dvd" } else { "dvdsimple" };
            vec![Item { uri: format!("{scheme}://{location}"), title: name.clone(), options: Vec::new() }]
        }
        DiscKind::Bluray => {
            let options = if menus { Vec::new() } else { vec![":no-bluray-menu".to_string()] };
            vec![Item { uri: format!("bluray://{location}"), title: name.clone(), options }]
        }
        // libVLC reads an audio CD drive directly and lists its tracks.
        DiscKind::Cd if has_cda(p) => vec![Item { uri: format!("cdda://{location}"), title: name.clone(), options: Vec::new() }],
        DiscKind::Cd => files(p, &["aiff", "aif", "wav", "flac"]).into_iter().map(Item::new).collect(),
        DiscKind::Vcd => ["MPEGAV", "mpegav", "MPEG2", "mpeg2"]
            .iter()
            .flat_map(|d| files(&p.join(d), &["dat", "mpg", "mpeg"]))
            .map(Item::new)
            .collect(),
    };
    if items.is_empty() {
        return Err(format!("No tracks found on {name}"));
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_disc_layouts() {
        let root = std::env::temp_dir().join(format!("revlc-disc-{}", std::process::id()));
        let dvd = root.join("MY MOVIE");
        let cd = root.join("Audio CD");
        std::fs::create_dir_all(dvd.join("VIDEO_TS")).unwrap();
        std::fs::create_dir_all(&cd).unwrap();
        std::fs::write(cd.join(".TOC.plist"), "").unwrap();
        std::fs::write(cd.join("10 Audio Track.aiff"), "").unwrap();
        std::fs::write(cd.join("2 Audio Track.aiff"), "").unwrap();
        assert_eq!(kind_of(&dvd), Some(DiscKind::Dvd));
        assert_eq!(kind_of(&dvd.join("VIDEO_TS")), Some(DiscKind::Dvd));
        assert_eq!(kind_of(&cd), Some(DiscKind::Cd));
        assert_eq!(kind_of(&root), None);

        let items = items(DiscKind::Dvd, &dvd.to_string_lossy(), true).unwrap();
        assert!(items[0].uri.starts_with("dvd:///") && items[0].uri.ends_with("/MY%20MOVIE"));
        assert_eq!(items[0].title, "MY MOVIE");
        let tracks = super::items(DiscKind::Cd, &cd.to_string_lossy(), true).unwrap();
        assert_eq!(tracks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["2 Audio Track.aiff", "10 Audio Track.aiff"]);
    }
}
