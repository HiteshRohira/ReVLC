//! Saving the app's JSON files.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Serialize, de::DeserializeOwned};

/// A saved file that couldn't be read as what it should hold.
#[derive(Clone, Debug, PartialEq)]
pub struct Damaged {
    /// Where the file was moved, so the next save doesn't overwrite it.
    /// `None` if it couldn't be moved.
    pub aside: Option<PathBuf>,
}

/// Moves an unreadable file out of the way, to `<name>-damaged-<unix seconds>.json`
/// next to it (with a number after it if one from the same second is there).
pub fn set_aside(path: &Path) -> Damaged {
    let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut aside = path.with_file_name(format!("{stem}-damaged-{now}.json"));
    let mut n = 1;
    while aside.exists() {
        n += 1;
        aside = path.with_file_name(format!("{stem}-damaged-{now}-{n}.json"));
    }
    Damaged { aside: std::fs::rename(path, &aside).is_ok().then_some(aside) }
}

/// Reads a JSON file. `Ok(None)` when there's no file. A file that is
/// there but can't be read, or doesn't hold a `T`, is moved aside rather
/// than left to be overwritten, and reported as [`Damaged`].
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, Damaged> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            eprintln!("revlc: {} can't be read: {e}", path.display());
            return Err(set_aside(path));
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(value) => Ok(Some(value)),
        Err(e) => {
            eprintln!("revlc: {} is damaged: {e}", path.display());
            Err(set_aside(path))
        }
    }
}

/// Writes `value` as JSON to `path`. Goes through a temporary file, so a
/// failed or interrupted write leaves the previous contents intact.
pub fn write_json(path: &Path, value: &impl Serialize, pretty: bool) -> Result<(), String> {
    write(path, value, pretty, false)
}

/// [`write_json`], flushed to the disk before it replaces the old file, so
/// a power cut can't leave an empty file. For files saved rarely.
pub fn write_json_durable(path: &Path, value: &impl Serialize, pretty: bool) -> Result<(), String> {
    write(path, value, pretty, true)
}

/// Writes through a temporary file; `flush` makes it reach the disk before the rename.
fn write(path: &Path, value: &impl Serialize, pretty: bool, flush: bool) -> Result<(), String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let fail = |e: &dyn std::fmt::Display| format!("Could not save {name}: {e}");
    let json = if pretty { serde_json::to_vec_pretty(value) } else { serde_json::to_vec(value) }.map_err(|e| fail(&e))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
    }
    let tmp = path.with_file_name(format!("{name}.tmp"));
    if flush {
        let mut file = std::fs::File::create(&tmp).map_err(|e| fail(&e))?;
        file.write_all(&json).map_err(|e| fail(&e))?;
        file.sync_all().map_err(|e| fail(&e))?;
    } else {
        std::fs::write(&tmp, json).map_err(|e| fail(&e))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        fail(&e)
    })
}

/// Reads a saved list. A missing file is an empty list. A file that can't
/// be understood is kept aside under another name rather than left to be
/// overwritten by the next save, and the list starts empty.
pub fn read_list<T: serde::de::DeserializeOwned>(path: &Path) -> Vec<T> {
    let Ok(bytes) = std::fs::read(path) else { return Vec::new() };
    match serde_json::from_slice(&bytes) {
        Ok(list) => list,
        Err(e) => {
            let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            let aside = path.with_file_name(format!("{stem}-damaged-{secs}.json"));
            eprintln!("revlc: {} is damaged ({e}); kept as {}", path.display(), aside.display());
            let _ = std::fs::rename(path, &aside);
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_damaged_list_is_kept_aside() {
        let dir = std::env::temp_dir().join(format!("revlc-store-damaged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("things.json");
        assert_eq!(read_list::<i32>(&file), Vec::<i32>::new());
        std::fs::write(&file, b"[1, 2").unwrap();
        assert_eq!(read_list::<i32>(&file), Vec::<i32>::new());
        // The damaged file is still there under another name, not at the old one.
        assert!(!file.exists());
        let kept: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(kept.len(), 1);
        assert!(kept[0].starts_with("things-damaged-"));
        assert_eq!(std::fs::read(dir.join(&kept[0])).unwrap(), b"[1, 2");
        write_json(&file, &vec![3], false).unwrap();
        assert_eq!(read_list::<i32>(&file), [3]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_what_was_written() {
        let dir = std::env::temp_dir().join(format!("revlc-store-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("x.json");
        write_json_durable(&file, &vec![1, 2, 3], false).unwrap();
        assert_eq!(read_json::<Vec<i32>>(&file), Ok(Some(vec![1, 2, 3])));
        assert!(!dir.join("x.json.tmp").exists());
        assert_eq!(read_json::<Vec<i32>>(&dir.join("missing.json")), Ok(None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_file_is_moved_aside() {
        let dir = std::env::temp_dir().join(format!("revlc-store-aside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("x.json");
        std::fs::write(&file, b"[1, 2").unwrap();
        let Err(Damaged { aside: Some(aside) }) = read_json::<Vec<i32>>(&file) else { panic!("not reported as damaged and moved aside") };
        assert_eq!(std::fs::read(&aside).unwrap(), b"[1, 2");
        assert!(!file.exists());
        assert!(aside.file_name().unwrap().to_string_lossy().starts_with("x-damaged-"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_cant_be_read_is_moved_aside() {
        let dir = std::env::temp_dir().join(format!("revlc-store-unreadable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // A folder where the file should be: there, but it can't be read.
        std::fs::create_dir_all(dir.join("x.json")).unwrap();
        let Err(Damaged { aside: Some(aside) }) = read_json::<Vec<i32>>(&dir.join("x.json")) else { panic!("not reported as damaged and moved aside") };
        assert!(aside.is_dir());
        assert!(!dir.join("x.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn damage_twice_in_a_second_keeps_both() {
        let dir = std::env::temp_dir().join(format!("revlc-store-twice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("x.json");
        std::fs::write(&file, b"first").unwrap();
        let Err(Damaged { aside: Some(first) }) = read_json::<Vec<i32>>(&file) else { panic!("first not moved aside") };
        std::fs::write(&file, b"second").unwrap();
        let Err(Damaged { aside: Some(second) }) = read_json::<Vec<i32>>(&file) else { panic!("second not moved aside") };
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), b"first");
        assert_eq!(std::fs::read(&second).unwrap(), b"second");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_and_replaces() {
        let dir = std::env::temp_dir().join(format!("revlc-store-{}", std::process::id()));
        let file = dir.join("sub").join("x.json");
        write_json(&file, &vec![1, 2], false).unwrap();
        write_json(&file, &vec![3], true).unwrap();
        let back: Vec<i32> = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(back, [3]);
        assert!(!dir.join("sub").join("x.json.tmp").exists());
        // A folder where the file should go can't be replaced.
        std::fs::create_dir_all(dir.join("blocked.json").join("inner")).unwrap();
        assert!(write_json(&dir.join("blocked.json"), &1, false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
