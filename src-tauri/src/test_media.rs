//! Generated media and helpers shared by the engine tests (vlc_it.rs,
//! keyframes_it.rs), which are ignored by default: they need FFmpeg, and
//! the libVLC ones libVLC. See vlc_it.rs for how to run them.

use std::path::PathBuf;

/// Where the generated media and captured output go: REVLC_IT_DIR (CI keeps
/// it when a test fails), or a folder in the temp directory.
pub(crate) fn work() -> PathBuf {
    let dir = std::env::var_os("REVLC_IT_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("revlc-vlc-it"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Makes `name` with FFmpeg once per machine (the media is deterministic).
pub(crate) fn media(name: &str, args: &[&str]) -> PathBuf {
    let out = work().join(name);
    if !out.exists() {
        let ffmpeg = std::env::var("REVLC_TEST_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        // Tests run in parallel and may make the same file at once: each
        // writes its own copy, and the first rename wins.
        let tmp = work().join(format!("tmp-{}-{:?}-{name}", std::process::id(), std::thread::current().id()).replace(['(', ')'], ""));
        let status = std::process::Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(args)
            .arg(&tmp)
            .status()
            .expect("FFmpeg runs (set REVLC_TEST_FFMPEG)");
        assert!(status.success(), "FFmpeg couldn't make {name}");
        if std::fs::rename(&tmp, &out).is_err() {
            let _ = std::fs::remove_file(&tmp);
            assert!(out.exists(), "couldn't keep {name}");
        }
    }
    out
}

/// A check for an open bug: it must fail. If it starts passing, the test
/// fails so the mark gets removed (the Cua suite does the same).
pub(crate) fn known_bug(bug: &str, check: impl FnOnce() + std::panic::UnwindSafe) {
    match std::panic::catch_unwind(check) {
        Err(_) => eprintln!("known bug, still there: {bug}"),
        Ok(()) => panic!("known bug now passes; remove its mark: {bug}"),
    }
}
