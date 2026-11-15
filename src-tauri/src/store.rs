//! Saving the app's JSON files.

use std::path::Path;

use serde::Serialize;

/// Writes `value` as JSON to `path`. Goes through a temporary file, so a
/// failed or interrupted write leaves the previous contents intact.
pub fn write_json(path: &Path, value: &impl Serialize, pretty: bool) -> Result<(), String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let fail = |e: &dyn std::fmt::Display| format!("Could not save {name}: {e}");
    let json = if pretty { serde_json::to_vec_pretty(value) } else { serde_json::to_vec(value) }.map_err(|e| fail(&e))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| fail(&e))?;
    }
    let tmp = path.with_file_name(format!("{name}.tmp"));
    std::fs::write(&tmp, json).map_err(|e| fail(&e))?;
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
