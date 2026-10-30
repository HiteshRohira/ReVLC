//! Property tests: the parsers and the history checked against many
//! generated inputs (proptest), not just the hand-picked ones in each
//! module's tests. A failure prints the smallest input that breaks it.

use crate::history::History;
use crate::playlist::Item;
use crate::playlist_file::{file_uri, file_uri_path, native_path, read, write};
use proptest::prelude::*;
use std::path::PathBuf;

/// A path segment with spaces, punctuation and non-Latin letters, but none
/// of the characters a file name can't hold.
fn segment() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 _.,'()\\[\\]#%&+=!@~ÄéÜñ視頻テスト😀-]{1,16}".prop_filter("a real name", |s| {
        let t = s.trim();
        !t.is_empty() && t != "." && t != ".." && !s.ends_with(['.', ' ']) && !s.starts_with(' ')
    })
}

fn absolute_path() -> impl Strategy<Value = String> {
    prop::collection::vec(segment(), 1..5).prop_map(|parts| {
        let joined = parts.join("/");
        if cfg!(windows) { format!("C:/{joined}.mkv") } else { format!("/{joined}.mkv") }
    })
}

/// A title as a player shows it: one line, no surrounding spaces.
fn title() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 ,.:;'\"&<>!?#=/ÄéÜñ視頻テスト😀-]{1,30}".prop_map(|t| t.trim().to_string()).prop_filter("non-empty", |t| !t.is_empty())
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("revlc-prop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    #[test]
    fn a_file_uri_names_the_same_path(path in absolute_path()) {
        let uri = file_uri(&path);
        prop_assert!(uri.is_ascii(), "{uri}");
        prop_assert_eq!(file_uri_path(&uri), Some(native_path(path)));
    }

    #[test]
    fn saved_playlists_read_back_the_same(
        entries in prop::collection::vec((absolute_path(), title()), 1..6),
        format in prop::sample::select(vec!["m3u", "m3u8", "pls", "xspf"]),
    ) {
        let items: Vec<Item> = entries.iter().map(|(p, t)| Item { title: t.clone(), ..Item::new(native_path(p.clone())) }).collect();
        let file = temp(&format!("list-{}.{format}", rand_suffix(&entries)));
        write(&file, &items).unwrap();
        let back = read(&file).unwrap();
        let _ = std::fs::remove_file(&file);
        prop_assert_eq!(back.len(), items.len());
        for (a, b) in items.iter().zip(&back) {
            prop_assert_eq!(&a.uri, &b.uri);
            prop_assert_eq!(&a.title, &b.title);
        }
    }

    #[test]
    fn the_history_stays_consistent(ops in prop::collection::vec((0u8..5, 0usize..8, 0i64..10_000_000, 0i64..10_000_000), 1..120)) {
        let dir = temp(&format!("history-{}", rand_suffix(&ops)));
        let mut h = History::load(dir.clone(), dir.join("cache"));
        let uri = |n: usize| format!("file:///videos/{n}.mkv");
        for &(op, n, a, b) in &ops {
            match op {
                0 => h.touch(&uri(n), "t"),
                1 => h.update(&uri(n), a, b),
                2 => h.finish(&uri(n)),
                3 => h.remove(&uri(n)),
                _ => h.set_title(&uri(n), "renamed"),
            }
        }
        let list = h.list();
        let mut uris: Vec<&str> = list.iter().map(|e| e.entry.uri.as_str()).collect();
        prop_assert!(list.len() <= 60);
        uris.sort();
        uris.dedup();
        prop_assert_eq!(uris.len(), list.len(), "each file once");
        for e in &list {
            let resume = h.resume_point(&e.entry.uri);
            if e.entry.finished {
                prop_assert_eq!(resume, None, "a finished file offers no resume");
            }
            if let Some(at) = resume {
                prop_assert!(at > 0 && (e.entry.length == 0 || at < e.entry.length));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_names_never_break_the_title_parser(name in "\\PC{0,80}") {
        let info = crate::meta::parse_path(std::path::Path::new(&format!("/videos/{name}.mkv")));
        let _ = crate::meta::display_title(std::path::Path::new(&name));
        prop_assert!(info.season.is_none() || info.episode.is_some());
    }

    #[test]
    fn episodes_are_found_in_release_names(show in "[A-Z][a-z]{2,8}( [A-Z][a-z]{2,8}){0,2}", s in 1u32..40, e in 1u32..99, sep in prop::sample::select(vec![".", " ", "_"])) {
        let name = format!("{}{sep}S{s:02}E{e:02}{sep}1080p{sep}WEB-DL.mkv", show.replace(' ', sep));
        let info = crate::meta::parse_path(std::path::Path::new(&format!("/tv/{name}")));
        prop_assert_eq!((info.season, info.episode), (Some(s), Some(e)), "{}", name);
        prop_assert_eq!(info.show.as_deref(), Some(show.as_str()), "{}", name);
    }

    #[test]
    fn any_command_line_parses(args in prop::collection::vec("\\PC{0,24}", 0..8)) {
        let cli = crate::cli::parse(args);
        prop_assert!(cli.files.len() <= 8);
    }
}

/// A file name suffix that differs between cases running at once.
fn rand_suffix<T: std::fmt::Debug>(v: &T) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{v:?}").hash(&mut h);
    std::thread::current().id().hash(&mut h);
    format!("{:x}", h.finish())
}
