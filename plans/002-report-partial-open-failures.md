# Plan 002: One unreadable playlist no longer cancels an open, and open errors on a cold launch are shown

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat c190987..HEAD -- src-tauri/src/lib.rs`
> `lib.rs` may already contain plan 001's changes (inside `fn setup`, around
> the `Settings::load` call and the notices after it). Those are expected.
> For everything else, compare the "Current state" excerpts against the live
> code before proceeding; on a mismatch, treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none (run after plan 001 if both are being done: they edit
  different parts of the same function and file)
- **Category**: bug
- **Planned at**: commit `c190987`, 2026-11-11

## Why this matters

When ReVLC is asked to open several things at once (a drop, the file picker,
the command line, "Open with" on Windows and Linux), one unreadable playlist
file or disc folder aborts the whole request: nothing is added to the queue,
even the files that were fine. On a cold launch from the command line the
error is also thrown away, so the app opens to the Home screen with no
message at all. After this plan, everything that can be opened is opened,
each failure is reported as a notice, and the cold-launch path reports
errors too.

## Current state

All in `src-tauri/src/lib.rs`.

`Core::add_with_options`, `lib.rs:918-950`. The two `?` operators return
early and drop every item collected so far:

```rust
    /// [`Core::add`], giving each new item the libVLC `options` (from the
    /// command line, like ":start-time=30").
    fn add_with_options(&self, uris: Vec<String>, play: bool, options: &[String]) -> Result<(), String> {
        let mut items = Vec::new();
        let mut subtitles = Vec::new();
        for uri in uris.into_iter().map(playlist_file::native_path) {
            let path = Path::new(&uri);
            if uri.contains("://") {
                items.push(Item::new(uri));
            } else if path.is_dir() {
                // A disc's folder plays as a disc, not as a folder of files.
                match disc::kind_of(path) {
                    Some(kind) => items.extend(disc::items(kind, &uri, true)?),
                    None => {
                        if let Err(e) = self.library().add(&uri) {
                            self.notice("error", e);
                        }
                        items.extend(library::expand(vec![uri]).into_iter().map(Item::new));
                    }
                }
            } else if playlist_file::is_playlist(path) {
                items.extend(playlist_file::read(path)?);
            } else if library::is_subtitle(path) {
                subtitles.push(uri);
            } else {
                items.push(Item::new(uri));
            }
        }
        for item in &mut items {
            item.options.extend(options.iter().cloned());
        }
        self.add_with_subtitles(items, subtitles, play)
    }
```

The cold-launch calls in `fn setup`, `lib.rs:2473-2476`, discard the result:

```rust
    // Files passed on the command line, then any Finder opened before now.
    let _ = core.add_with_options(cli.files, true, &cli.media_options);
    let early = std::mem::take(&mut *EARLY_OPENS.lock().unwrap_or_else(|e| e.into_inner()));
    let _ = core.add(early, true);
```

Facts you need:

- `playlist_file::read(path: &Path) -> Result<Vec<Item>, String>` fails only
  when the file can't be read; its message is `Could not read <path>: <os error>`.
- `disc::items(kind, path: &str, menus: bool) -> Result<Vec<Item>, String>`
  fails with messages such as `<path> doesn't exist` or `No tracks found on <name>`.
- `library::expand(uris: Vec<String>) -> Vec<String>` lists the media files
  under folders. `library::is_subtitle(path: &Path) -> bool` and
  `playlist_file::is_playlist(path: &Path) -> bool` decide by file extension
  only (`.srt`, `.m3u`, ...): they do not touch the disk.
- `self.notice("error", text)` shows a message in the UI. Before the UI is
  listening (during `setup`) notices are queued and delivered with the UI's
  first snapshot, so calling it during `setup` is correct.
- `Item::new(uri)` builds a queue item; `Item` has public fields `uri`,
  `title`, `options`, `trusted`.
- The other callers of `add_with_options` already surface an `Err`: the
  `open` command returns it to the UI, which shows it as an error notice;
  `Core::add_from_main` turns it into a notice.

**Conventions to match** (this repo is hand-formatted):

- **Do NOT run `cargo fmt`.** There is no `rustfmt.toml`; lines run up to
  about 150 columns and `cargo fmt` would rewrite the whole file. Match the
  surrounding style by hand.
- `///` doc comments in plain sentences on every new item.
- Errors are `Result<_, String>`. No `unwrap()`/`expect()` outside tests (the
  release build aborts on panic).
- Tests go in the existing `#[cfg(test)] mod tests` at the bottom of `lib.rs`
  and use a temp folder named after the process id, e.g.
  `std::env::temp_dir().join(format!("revlc-opened-{}", std::process::id()))`.

## Commands you will need

Taken from `.github/workflows/checks.yml`; the cargo commands were not run
during planning.

| Purpose | Command | Expected on success |
|---|---|---|
| Install | `npm ci` (repo root) | exit 0 |
| Build the UI (the Rust build embeds it) | `npm run build && mkdir -p src-tauri/vlc src-tauri/binaries` | exit 0 |
| FFmpeg stand-in (the Rust build expects the file) | the `touch` command below this table | exit 0 |
| Lint | `cd src-tauri && cargo clippy --locked --all-targets -- -D warnings` | exit 0 |
| Tests | `cd src-tauri && cargo nextest run --locked` | all pass (`#[ignore]`d tests are skipped; expected) |

If `cargo nextest` is not installed, use `cargo test --locked`.

The FFmpeg stand-in, from the repo root (macOS and Linux; on Windows add `.exe` to the name):

```sh
touch "src-tauri/binaries/ffmpeg-$(rustc -vV | sed -n 's/^host: //p')"
```

**Never launch the app.** This plan needs no launch, and the project forbids
agent runs that could send analytics.

## Scope

**In scope** (the only files you should modify):
- `src-tauri/src/lib.rs` — `Core::add_with_options`, the two cold-launch
  lines in `fn setup`, one new free function with its struct, one new test.
- `plans/README.md` (status row only)

**Out of scope** (do NOT touch):
- `playlist_file.rs`, `disc.rs`, `library.rs` — their error behaviour is fine.
- `Core::add_with_subtitles`, `Core::mark_ready`, `Core::add_from_main`.
- The `open` Tauri command and anything in `ui/`.
- Nested playlists, playlist size limits, or any other open behaviour.

## Git workflow

- Other agent threads edit the main checkout. Work in your own worktree:
  `git worktree add ../revlc-plan-002 -b fix/partial-open-failures main`
  and never run `git commit -a` in the main checkout.
- Commit message style is a plain sentence, as in `git log`: `Leave the main
  window to setup on Linux and Windows too`, `Settings: one unreadable value
  takes its default on its own`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Pull the sorting of inputs into a function that never fails

In `src-tauri/src/lib.rs`, directly above `impl Core`'s `add_with_options`
is not possible (it is inside the `impl`), so add this as a free function
and struct just above the `/// Generates library thumbnails one at a time.`
comment that precedes `fn make_thumbnails`:

```rust
/// What a list of paths and URLs holds.
#[derive(Default)]
struct Opened {
    items: Vec<Item>,
    subtitles: Vec<String>,
    /// Plain folders, which also join the library.
    folders: Vec<String>,
    /// Why some of them couldn't be opened; the rest still are.
    failed: Vec<String>,
}

/// Sorts paths and URLs into what to play: folders give their media,
/// playlist files their entries, and a disc's folder plays as a disc.
fn opened(uris: Vec<String>) -> Opened {
    let mut found = Opened::default();
    for uri in uris.into_iter().map(playlist_file::native_path) {
        let path = Path::new(&uri);
        if uri.contains("://") {
            found.items.push(Item::new(uri));
        } else if path.is_dir() {
            // A disc's folder plays as a disc, not as a folder of files.
            match disc::kind_of(path) {
                Some(kind) => match disc::items(kind, &uri, true) {
                    Ok(items) => found.items.extend(items),
                    Err(e) => found.failed.push(e),
                },
                None => {
                    found.items.extend(library::expand(vec![uri.clone()]).into_iter().map(Item::new));
                    found.folders.push(uri);
                }
            }
        } else if playlist_file::is_playlist(path) {
            match playlist_file::read(path) {
                Ok(items) => found.items.extend(items),
                Err(e) => found.failed.push(e),
            }
        } else if library::is_subtitle(path) {
            found.subtitles.push(uri);
        } else {
            found.items.push(Item::new(uri));
        }
    }
    found
}
```

**Verify**: `cd src-tauri && cargo check --locked 2>&1 | tail -3` → `Finished`; a "never used" warning for `opened`/`Opened` is expected until step 2.

### Step 2: Use it in `add_with_options`

Replace the body of `Core::add_with_options` (keep its doc comment and
signature) with:

```rust
        let Opened { mut items, subtitles, folders, failed } = opened(uris);
        // Nothing at all could be opened: that's the caller's error to show.
        if items.is_empty() && subtitles.is_empty() && !failed.is_empty() {
            return Err(failed.join("; "));
        }
        for e in failed {
            self.notice("error", e);
        }
        for folder in folders {
            if let Err(e) = self.library().add(&folder) {
                self.notice("error", e);
            }
        }
        for item in &mut items {
            item.options.extend(options.iter().cloned());
        }
        self.add_with_subtitles(items, subtitles, play)
```

**Verify**: `cd src-tauri && cargo check --locked 2>&1 | tail -3` → `Finished`, no warnings about `opened` or `Opened`.

### Step 3: Report cold-launch errors

In `fn setup`, replace the two `let _ =` lines shown in "Current state" with:

```rust
    // Files passed on the command line, then any Finder opened before now.
    if let Err(e) = core.add_with_options(cli.files, true, &cli.media_options) {
        core.notice("error", e);
    }
    let early = std::mem::take(&mut *EARLY_OPENS.lock().unwrap_or_else(|e| e.into_inner()));
    if let Err(e) = core.add(early, true) {
        core.notice("error", e);
    }
```

**Verify**: `grep -n "let _ = core.add" src-tauri/src/lib.rs` → no output.

### Step 4: Test `opened`

Add to the `mod tests` at the bottom of `lib.rs`:

```rust
    #[test]
    fn one_unreadable_playlist_leaves_the_rest_to_open() {
        let dir = std::env::temp_dir().join(format!("revlc-opened-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
        std::fs::write(dir.join("good.m3u"), "one.mkv\ntwo.mkv\n").unwrap();
        // gone.m3u isn't there, so it can't be read.
        let found = opened(vec![path("good.m3u"), path("gone.m3u"), path("film.mkv"), path("film.srt"), "https://example.com/live.m3u8".into()]);
        let uris: Vec<&str> = found.items.iter().map(|i| i.uri.as_str()).collect();
        assert_eq!(uris, [path("one.mkv"), path("two.mkv"), path("film.mkv"), "https://example.com/live.m3u8".to_string()]);
        assert_eq!(found.subtitles, [path("film.srt")]);
        assert!(found.folders.is_empty());
        assert_eq!(found.failed.len(), 1);
        assert!(found.failed[0].contains("gone.m3u"), "{}", found.failed[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }
```

If the `uris` comparison fails only because of path separators or symlinked
temp folders (for example `/private/var` versus `/var` on macOS), compare
with `std::path::Path::new(..)` values instead of strings; do not weaken the
`failed` assertions.

**Verify**: `cd src-tauri && cargo nextest run --locked one_unreadable_playlist` → 1 test passes.

### Step 5: Full checks

**Verify**:
- `cd src-tauri && cargo clippy --locked --all-targets -- -D warnings` → exit 0.
- `cd src-tauri && cargo nextest run --locked` → all pass.

## Test plan

- New: `one_unreadable_playlist_leaves_the_rest_to_open` in `lib.rs`'s
  `mod tests` (happy entries, the unreadable playlist, a subtitle, a URL).
- Pattern: the temp-folder tests in `src-tauri/src/playlist_file.rs`
  (`round_trip`) and `src-tauri/src/library.rs` (`failed_saves_change_nothing`).
- `Core::add_with_options` itself needs a running player and window and has
  no unit test; its new body is a straight use of `opened`.

## Done criteria

ALL must hold:

- [ ] `cd src-tauri && cargo clippy --locked --all-targets -- -D warnings` exits 0
- [ ] `cd src-tauri && cargo nextest run --locked` exits 0; `one_unreadable_playlist_leaves_the_rest_to_open` exists and passes
- [ ] `grep -n "let _ = core.add" src-tauri/src/lib.rs` returns no matches
- [ ] `grep -c "playlist_file::read(path)?" src-tauri/src/lib.rs` prints `0`
- [ ] `git status --short` lists only `src-tauri/src/lib.rs` and `plans/README.md`
- [ ] `git diff --stat` shows a small diff (you did not run `cargo fmt`)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- `add_with_options` or the cold-launch lines don't match the excerpts
  (beyond plan 001's expected edits elsewhere in `setup`).
- `opened` needs access to `self` for anything other than what step 2 does
  with its result.
- A step's verification fails twice after a reasonable fix attempt.
- You find yourself wanting to change `playlist_file.rs`, `disc.rs`,
  `library.rs` or `ui/`.

## Maintenance notes

- Behaviour change to review: when some inputs fail and others don't, the
  caller now gets `Ok` and the failures arrive as separate error notices.
  Only "everything failed" is an `Err`.
- Folders are now added to the library after their files are listed, not
  before; the result is the same.
- Any new kind of input (for example a new playlist format) belongs in
  `opened`, pushing to `failed` instead of returning early.
- Deferred: a Cua scenario for "launch with an unreadable playlist on the
  command line, expect a notice" in `scripts/qa/ci/cua_suite.py`. It needs a
  built app.
