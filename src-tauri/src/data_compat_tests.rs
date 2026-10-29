//! Data saved by every released version still loads: settings, history,
//! library folders, bookmarks and the analytics id. The files in
//! tests/data/<version> are what that version wrote for a user who changed
//! things (one folder per release that changed a format), so falling back
//! to defaults shows up as a lost value.
//!
//! When a release changes a saved format, add its folder here.

use crate::bookmarks::Bookmarks;
use crate::history::History;
use crate::library::Library;
use crate::settings::{Resume, Settings, Wheel};
use std::path::{Path, PathBuf};

const VERSIONS: &[&str] = &["0.1.0", "0.1.2", "0.2.5", "0.2.9", "0.2.10"];
const FILM: &str = "file:///Users/someone/Movies/film.mkv";

fn data(version: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(version)
}

/// A copy to load from, since loading may write (history keeps a damaged
/// file aside, for one).
fn copy(version: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("revlc-data-compat-{}-{n}-{version}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for f in std::fs::read_dir(data(version)).unwrap().flatten() {
        std::fs::copy(f.path(), dir.join(f.file_name())).unwrap();
    }
    dir
}

#[test]
fn every_released_settings_file_keeps_its_values() {
    for v in VERSIONS {
        let s = Settings::load(&copy(v));
        assert_eq!(s.resume, Resume::Never, "{v}: resume");
        assert_eq!(s.font, "Georgia", "{v}: font");
        assert_eq!(s.font_size, 12, "{v}: font size");
        assert_eq!(s.subtitle_language.as_deref(), Some("es"), "{v}: subtitle language");
        assert_eq!(s.network_caching, 5000, "{v}: network caching");
        assert_eq!(s.shortcuts.get("play-pause").map(String::as_str), Some("Ctrl+Space"), "{v}: shortcuts");
        let has = |field: &str| std::fs::read_to_string(data(v).join("settings.json")).unwrap().contains(&format!("\"{field}\""));
        if has("autoUpdate") {
            assert!(!s.auto_update, "{v}: auto update");
        }
        if has("analytics") {
            assert!(!s.analytics, "{v}: the analytics opt-out was lost");
        }
        if has("wheel") {
            assert_eq!(s.wheel, Wheel::Seek, "{v}: wheel");
        }
        if has("seekbarPreview") {
            assert!(!s.seekbar_preview, "{v}: seek bar preview");
        }
    }
}

#[test]
fn every_released_history_library_and_bookmarks_file_loads() {
    for v in VERSIONS {
        let dir = copy(v);
        let history = History::load(dir.clone(), dir.join("cache"));
        assert!(history.damaged.is_none(), "{v}: history.json set aside as damaged");
        assert_eq!(history.list().len(), 2, "{v}: history entries");
        assert_eq!(history.resume_point(FILM), Some(1_234_000), "{v}: resume point");
        assert_eq!(Library::load(&dir).folders, ["/Users/someone/Movies", "/Volumes/Media/TV"], "{v}: library folders");
        if data(v).join("bookmarks.json").exists() {
            let names: Vec<String> = Bookmarks::load(&dir).list(FILM).into_iter().map(|b| b.name).collect();
            assert_eq!(names, ["Opening", "Bookmark 2"], "{v}: bookmarks");
        }
    }
}

#[test]
fn the_analytics_id_survives_an_update() {
    for v in VERSIONS.iter().filter(|v| data(v).join("analytics.json").exists()) {
        let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(data(v).join("analytics.json")).unwrap()).unwrap();
        assert_eq!(crate::telemetry::saved_id(&copy(v)), saved["id"].as_str().unwrap(), "{v}: analytics id");
    }
}

#[test]
fn one_unreadable_setting_keeps_the_others() {
    let dir = copy("0.2.10");
    let file = dir.join("settings.json");
    let text = std::fs::read_to_string(&file).unwrap().replace("\"fontSize\": 12", "\"fontSize\": \"large\"");
    std::fs::write(&file, text).unwrap();
    crate::test_media::known_bug("one setting of the wrong type resets every setting, the analytics opt-out included", move || {
        let s = Settings::load(&dir);
        assert!(!s.analytics, "analytics turned back on");
        assert_eq!(s.resume, Resume::Never, "resume reset");
    });
}
