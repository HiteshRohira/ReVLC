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

#[cfg(test)]
mod tests {
    use super::*;

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
