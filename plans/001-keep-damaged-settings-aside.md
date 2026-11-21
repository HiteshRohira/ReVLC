# Plan 001: A damaged settings, bookmarks or library file is kept aside and reported, never silently reset

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat c190987..HEAD -- src-tauri/src/store.rs src-tauri/src/settings.rs src-tauri/src/bookmarks.rs src-tauri/src/library.rs src-tauri/src/lib.rs src-tauri/src/data_compat_tests.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `c190987`, 2026-11-11

## Why this matters

ReVLC keeps user data as JSON files in its data folder. When `history.json`
can't be parsed, the app moves it aside and tells the user. The other three
files don't get that care: an unparseable `settings.json`, `bookmarks.json` or
`library.json` is silently treated as "no file", and the next save overwrites
it. For settings this has a privacy cost: the defaults have analytics **on**,
so a user who opted out is silently opted back in, and a launch event is
queued before they can notice. A hand edit with a stray comma is enough to
trigger it; so is a power cut, because the files are renamed into place
without being flushed first.

After this plan: a damaged file is moved aside next to the original, the user
gets a notice naming where it went, damaged settings start with analytics
**off**, and bookmarks and library folders are flushed to disk before the
rename.

**Product decision baked into this plan** (the maintainer accepted
"analytics on by default" for fresh installs; this is a narrower case): when
the settings file is unreadable we cannot know whether the user opted out, so
analytics starts off. If the maintainer disagrees, they will edit this plan
before you run it. Do not change this yourself.

## Current state

All paths are relative to the repository root. Rust sources are in
`src-tauri/src/`.

- `store.rs` — the one helper for saving JSON (42 lines). There is no
  matching "read" helper.
- `settings.rs` — `Settings` (a serde struct) and `Settings::load`.
- `bookmarks.rs` — `Bookmarks::load`.
- `library.rs` — `Library::load`.
- `history.rs` — already does the right thing; it is the pattern to copy.
- `lib.rs` — `setup()` loads all of them and can queue notices for the UI.
- `data_compat_tests.rs` — tests that load data files written by old releases.

`store.rs:7-22` today:

```rust
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
```

`settings.rs:163-169` today (an unparseable file falls through to
`unwrap_or_default()`):

```rust
    pub fn load(data_dir: &Path) -> Self {
        std::fs::read(file(data_dir))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .map(Self::from_json)
            .unwrap_or_default()
    }
```

`Settings::from_json` (just below it) already handles a JSON object with one
bad field; it returns `Self::default()` when the JSON isn't an object. Keep
that function as it is.

`bookmarks.rs:22-29` today:

```rust
    pub fn load(data_dir: &Path) -> Self {
        let file = data_dir.join("bookmarks.json");
        let map = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { file, map }
    }
```

`library.rs:172-184` today:

```rust
    pub fn load(data_dir: &Path) -> Self {
        let file = data_dir.join("library.json");
        let saved: Vec<String> = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut folders: Vec<String> = Vec::new();
        for folder in saved.iter().map(|f| normalize(f)) {
            if !folders.contains(&folder) {
                folders.push(folder);
            }
        }
        Self { file, folders }
    }
```

The pattern to copy, `history.rs:54-67`:

```rust
    pub fn load(data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        let file = data_dir.join("history.json");
        let mut damaged = None;
        let entries = match std::fs::read(&file).map(|bytes| serde_json::from_slice(&bytes)) {
            Ok(Ok(entries)) => entries,
            // Unreadable: keep it aside rather than overwrite it with an empty history.
            Ok(Err(e)) => {
                eprintln!("revlc: history.json is damaged: {e}");
                let aside = data_dir.join(format!("history-damaged-{}.json", now()));
                damaged = std::fs::rename(&file, &aside).is_ok().then_some(aside);
                Vec::new()
            }
            Err(_) => Vec::new(),
        };
```

How `lib.rs` uses them, in `fn setup` (line numbers at the planned commit):

```rust
// lib.rs:2377
    let settings = Settings::load(&data_dir);
// lib.rs:2385 — analytics starts from the loaded value and queues "app_launched"
    telemetry::start(&data_dir, settings.analytics, vlc.as_ref().map_or("unavailable", |v| v.version.as_str()));
// lib.rs:2413 and 2420, inside the `Core { ... }` literal
        library: Mutex::new(Library::load(&data_dir)),
        bookmarks: Mutex::new(Bookmarks::load(&data_dir)),
// lib.rs:2438-2443 — notices queued before the UI is listening
    if !cli.ignored.is_empty() {
        core.notice("error", format!("Ignored command-line options: {}", cli.ignored.join(" ")));
    }
    if !core.vlc.rejected_args.is_empty() {
        core.notice("error", format!("libVLC didn't accept {}, so ReVLC started without them", core.vlc.rejected_args.join(" ")));
    }
```

The existing history notice, `lib.rs:999-1002`, shows the wording style:

```rust
        let damaged = self.history().damaged.take();
        if let Some(aside) = damaged {
            self.notice("error", format!("Your history couldn't be read, so it starts empty. The old file is kept at {}", aside.display()));
        }
```

`Core` has accessors `core.library()` and `core.bookmarks()` that return the
locked value (`MutexGuard`).

**Conventions to match** (this repo is hand-formatted):

- **Do NOT run `cargo fmt`.** There is no `rustfmt.toml`; the code uses long
  lines (up to about 150 columns) and `cargo fmt` would rewrite every file.
  Match the surrounding style by hand.
- Every public item has a `///` doc comment in plain sentences. No section
  banners.
- Errors are `Result<_, String>` with user-readable messages.
- Mutexes are locked with `.lock().unwrap_or_else(|e| e.into_inner())`, never
  `.unwrap()`. The release build uses `panic = "abort"`, so do not add
  `unwrap()`/`expect()` outside tests.
- Tests live in a `#[cfg(test)] mod tests` at the bottom of each file and use
  a temp folder named after the process id, e.g.
  `std::env::temp_dir().join(format!("revlc-bookmarks-{}", std::process::id()))`.

## Commands you will need

Taken from `.github/workflows/checks.yml`. `npm run check` was run during
planning and passed; the cargo commands were not run during planning.

| Purpose | Command | Expected on success |
|---|---|---|
| Install | `npm ci` (repo root) | exit 0 |
| Build the UI (the Rust build embeds it) | `npm run build && mkdir -p src-tauri/vlc src-tauri/binaries` | exit 0 |
| FFmpeg stand-in (the Rust build expects the file) | the `touch` command below this table | exit 0 |
| Lint | `cd src-tauri && cargo clippy --locked --all-targets -- -D warnings` | exit 0, no warnings |
| Tests | `cd src-tauri && cargo nextest run --locked` | all pass (tests marked `#[ignore]` are skipped; that is expected) |
| One test | `cd src-tauri && cargo nextest run --locked <name substring>` | the named tests pass |

If `cargo nextest` is not installed, use `cargo test --locked` instead.

The FFmpeg stand-in, from the repo root (macOS and Linux; on Windows add `.exe` to the name):

```sh
touch "src-tauri/binaries/ffmpeg-$(rustc -vV | sed -n 's/^host: //p')"
```

**Never launch the app** (`npm run tauri dev`, the built binary, etc.). This
plan needs no launch, and the project forbids agent runs that could send
analytics.

## Scope

**In scope** (the only files you should modify):
- `src-tauri/src/store.rs`
- `src-tauri/src/settings.rs`
- `src-tauri/src/bookmarks.rs`
- `src-tauri/src/library.rs`
- `src-tauri/src/lib.rs` (only inside `fn setup`)
- `src-tauri/src/data_compat_tests.rs`
- `plans/README.md` (status row only)

**Out of scope** (do NOT touch, even though they look related):
- `src-tauri/src/history.rs` — already correct; do not refactor it onto the
  new helper.
- `src-tauri/src/meta.rs` and `src-tauri/src/telemetry.rs` — a metadata cache
  and an install id; losing either is harmless.
- The existing `write_json` behaviour for history and the metadata cache.
  History is written every few seconds during playback; flushing it each time
  would wake the disk constantly.
- `Settings::save` stays on plain `write_json`: settings are saved on every
  tick of a slider drag, and a flush per tick (a full device flush on macOS)
  would make sliders lag.
- Anything in `ui/`. The UI already shows notices.

## Git workflow

- Other agent threads edit the main checkout. Work in your own worktree:
  `git worktree add ../revlc-plan-001 -b fix/damaged-data-files main`
  and never run `git commit -a` in the main checkout.
- Commit per logical unit. Message style is a plain sentence, optionally with
  an area prefix, as in `git log`: `Settings: one unreadable value takes its
  default on its own`, `Memory video output: use the visible size, not the
  decoder's padded one`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Add `Damaged`, `read_json` and `write_json_durable` to `store.rs`

In `src-tauri/src/store.rs`:

1. Change the imports to `use std::path::{Path, PathBuf};` and
   `use serde::{Serialize, de::DeserializeOwned};`.
2. Add:

```rust
/// A saved file that couldn't be read as what it should hold.
#[derive(Clone, Debug, PartialEq)]
pub struct Damaged {
    /// Where the file was moved, so the next save doesn't overwrite it.
    /// `None` if it couldn't be moved.
    pub aside: Option<PathBuf>,
}

/// Moves an unreadable file out of the way, to `<name>-damaged-<unix seconds>.json`
/// next to it.
pub fn set_aside(path: &Path) -> Damaged {
    let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let aside = path.with_file_name(format!("{stem}-damaged-{now}.json"));
    Damaged { aside: std::fs::rename(path, &aside).is_ok().then_some(aside) }
}

/// Reads a JSON file. `Ok(None)` when there's no file (or it can't be
/// opened). A file that is there but doesn't hold a `T` is moved aside
/// rather than left to be overwritten, and reported as [`Damaged`].
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, Damaged> {
    let Ok(bytes) = std::fs::read(path) else { return Ok(None) };
    match serde_json::from_slice(&bytes) {
        Ok(value) => Ok(Some(value)),
        Err(e) => {
            eprintln!("revlc: {} is damaged: {e}", path.display());
            Err(set_aside(path))
        }
    }
}
```

3. Make the temp-file write flushable. Turn the body of `write_json` into a
   private `fn write(path: &Path, value: &impl Serialize, pretty: bool, flush: bool) -> Result<(), String>`.
   When `flush` is true, write the temp file with
   `std::fs::File::create(&tmp)`, `write_all(&json)` (needs
   `use std::io::Write;`) and `sync_all()`, mapping each error with the
   existing `fail` closure, before the rename. When false, keep the existing
   `std::fs::write(&tmp, json)`. Then:

```rust
pub fn write_json(path: &Path, value: &impl Serialize, pretty: bool) -> Result<(), String> {
    write(path, value, pretty, false)
}

/// [`write_json`], flushed to the disk before it replaces the old file, so
/// a power cut can't leave an empty file. For files saved rarely.
pub fn write_json_durable(path: &Path, value: &impl Serialize, pretty: bool) -> Result<(), String> {
    write(path, value, pretty, true)
}
```

   Keep the existing doc comment on `write_json`.

4. Add tests to the existing `mod tests` in `store.rs`:
   - `reads_what_was_written`: `write_json_durable` then `read_json::<Vec<i32>>`
     gives `Ok(Some(vec))`; `read_json::<Vec<i32>>` of a path that doesn't
     exist gives `Ok(None)`.
   - `a_damaged_file_is_moved_aside`: write the bytes `[1, 2` to `x.json`;
     `read_json::<Vec<i32>>` returns `Err(Damaged { aside: Some(p) })`; the
     file at `p` holds the same bytes; `x.json` no longer exists; `p`'s file
     name starts with `x-damaged-`.

**Verify**: `cd src-tauri && cargo nextest run --locked store` → all `store::tests` pass (3 tests).

### Step 2: `Settings::load` reports damage and starts with analytics off

In `src-tauri/src/settings.rs`, replace `load` with:

```rust
    /// The saved settings, and what happened to the file if it couldn't be
    /// read. Then the settings are the defaults, except that analytics is
    /// off: whether the user had turned it off can't be known. They're saved
    /// at once, so the next launch doesn't turn it back on.
    pub fn load(data_dir: &Path) -> (Self, Option<crate::store::Damaged>) {
        let path = file(data_dir);
        let damaged = match crate::store::read_json::<serde_json::Value>(&path) {
            Ok(None) => return (Self::default(), None),
            Ok(Some(json @ serde_json::Value::Object(_))) => return (Self::from_json(json), None),
            // JSON, but not a settings object.
            Ok(Some(_)) => crate::store::set_aside(&path),
            Err(damaged) => damaged,
        };
        let settings = Self { analytics: false, ..Self::default() };
        if let Err(e) = settings.save(data_dir) {
            eprintln!("revlc: {e}");
        }
        (settings, Some(damaged))
    }
```

Do not change `from_json`, `save`, or anything else in the file. The two
existing tests in `settings.rs` call `serde_json::from_str`, not `load`, and
need no change.

**Verify**: `cd src-tauri && cargo check --locked --all-targets 2>&1 | grep -c "^error"` → a non-zero count is expected here (callers of `load` in `lib.rs` and `data_compat_tests.rs` are fixed in steps 4 and 5). Confirm every error is about `Settings::load`'s return type and nothing else.

### Step 3: `Bookmarks::load` and `Library::load` report damage and save durably

In `src-tauri/src/bookmarks.rs`:

- Add a field to `Bookmarks`:
  ```rust
      /// What happened to an unreadable bookmarks file, to tell the user.
      pub damaged: Option<crate::store::Damaged>,
  ```
- Replace the body of `load` so it uses the helper:
  ```rust
          let file = data_dir.join("bookmarks.json");
          let (map, damaged) = match crate::store::read_json(&file) {
              Ok(map) => (map.unwrap_or_default(), None),
              Err(damaged) => (BTreeMap::new(), Some(damaged)),
          };
          Self { file, map, damaged }
  ```
- In `save`, call `crate::store::write_json_durable` instead of
  `crate::store::write_json`.

In `src-tauri/src/library.rs`, do the same for `Library`: add the
`pub damaged: Option<crate::store::Damaged>` field with a doc comment, read
`saved: Vec<String>` through `crate::store::read_json(&file)` (empty `Vec`
when missing or damaged), keep the existing normalise-and-dedupe loop, build
`Self { file, folders, damaged }`, and switch `save` to
`write_json_durable`.

Search each file for other places that construct the struct literally
(`Bookmarks {` / `Library {`); at the planned commit only `load` does. If you
find another, give it `damaged: None`.

Add one test to each file's `mod tests`:

- `bookmarks.rs` — `a_damaged_file_is_kept_aside`: write `{"a": [` to
  `bookmarks.json` in a fresh temp dir; `Bookmarks::load` gives an empty
  `list("a")` and `damaged` with `aside: Some(p)`; `p` holds the original
  bytes; then `add("a", 1000, None)` succeeds and a fresh `Bookmarks::load`
  has one bookmark and `damaged == None`; the file at `p` still exists.
- `library.rs` — the same shape with `["/Movies",` in `library.json`, then
  `add("/Movies")`.

**Verify**: `cd src-tauri && cargo nextest run --locked bookmarks library 2>&1 | tail -5` → compiles once steps 4 and 5 are done; if it fails to compile only because of `Settings::load` callers, continue to step 4 and re-run this at the end of step 5.

### Step 4: Use the results in `setup` (`lib.rs`)

In `src-tauri/src/lib.rs`, inside `fn setup` only:

1. Line 2377 becomes:
   ```rust
       let (settings, settings_damaged) = Settings::load(&data_dir);
   ```
2. Directly after the `rejected_args` notice block (after line 2443 at the
   planned commit), add:
   ```rust
       // Files that couldn't be read were moved aside; say so, like the history's.
       let kept = |d: &store::Damaged| d.aside.as_ref().map(|p| format!(" The old file is kept at {}", p.display())).unwrap_or_default();
       if let Some(d) = &settings_damaged {
           core.notice("error", format!("Your settings couldn't be read, so they're back to the defaults, with usage statistics off.{}", kept(d)));
       }
       if let Some(d) = core.bookmarks().damaged.take() {
           core.notice("error", format!("Your bookmarks couldn't be read, so they start empty.{}", kept(&d)));
       }
       if let Some(d) = core.library().damaged.take() {
           core.notice("error", format!("Your library folders couldn't be read, so the library starts empty.{}", kept(&d)));
       }
   ```
   `store` is already a module of this crate (`mod store;` at the top of
   `lib.rs`), so `store::Damaged` resolves.

Do not move the `telemetry::start(...)` call: it already receives
`settings.analytics`, which is now `false` for a damaged file.

**Verify**: `cd src-tauri && cargo check --locked 2>&1 | tail -3` → `Finished` with no errors (non-test build).

### Step 5: Update and extend `data_compat_tests.rs`

In `src-tauri/src/data_compat_tests.rs`:

1. Line 39: `let s = Settings::load(&copy(v));` becomes
   ```rust
           let (s, damaged) = Settings::load(&copy(v));
           assert!(damaged.is_none(), "{v}: settings.json set aside as damaged");
   ```
2. Line 94 (in `one_unreadable_setting_keeps_the_others`):
   `let s = Settings::load(&dir);` becomes
   ```rust
       let (s, damaged) = Settings::load(&dir);
       assert!(damaged.is_none(), "one bad value isn't a damaged file");
   ```
3. In `every_released_history_library_and_bookmarks_file_loads`, also assert
   `Library::load(&dir).damaged.is_none()` and (inside the existing
   `if data(v).join("bookmarks.json").exists()` block)
   `Bookmarks::load(&dir).damaged.is_none()`.
4. Add a new test:

```rust
#[test]
fn an_unreadable_settings_file_is_kept_aside_with_analytics_off() {
    // Cut short, an empty file, and JSON that isn't a settings object.
    for bad in ["{\"analytics\": false,", "", "[]"] {
        let dir = copy("0.2.10");
        let file = dir.join("settings.json");
        std::fs::write(&file, bad).unwrap();
        let (s, damaged) = Settings::load(&dir);
        let aside = damaged.expect("reported as damaged").aside.expect("moved aside");
        assert_eq!(std::fs::read_to_string(&aside).unwrap(), bad, "the old file is kept as it was");
        assert!(!s.analytics, "{bad:?}: analytics came back on");
        assert_eq!(Settings { analytics: true, ..s }, Settings::default(), "{bad:?}: everything else is the default");
        // Saved at once: the next launch reads it, and analytics stays off.
        let (again, damaged) = Settings::load(&dir);
        assert!(damaged.is_none(), "{bad:?}: damaged again on the next launch");
        assert!(!again.analytics, "{bad:?}: analytics came back on at the next launch");
    }
}
```

**Verify**: `cd src-tauri && cargo nextest run --locked data_compat store bookmarks library settings` → all pass, including `an_unreadable_settings_file_is_kept_aside_with_analytics_off`, `a_damaged_file_is_moved_aside`, and the two `a_damaged_file_is_kept_aside` tests.

### Step 6: Full checks

**Verify**:
- `cd src-tauri && cargo clippy --locked --all-targets -- -D warnings` → exit 0.
- `cd src-tauri && cargo nextest run --locked` → all pass.

## Test plan

New tests (all listed in the steps above):

- `store.rs`: `reads_what_was_written`, `a_damaged_file_is_moved_aside`.
- `bookmarks.rs`: `a_damaged_file_is_kept_aside`.
- `library.rs`: `a_damaged_file_is_kept_aside`.
- `data_compat_tests.rs`: `an_unreadable_settings_file_is_kept_aside_with_analytics_off`,
  plus the "not damaged" assertions on every released version's files.

Structural patterns: `history.rs` test `damaged_file_is_kept_aside` for the
aside checks; `bookmarks.rs` test `add_sorts_and_replaces_nearby` for temp
folders.

## Done criteria

ALL must hold:

- [ ] `cd src-tauri && cargo clippy --locked --all-targets -- -D warnings` exits 0
- [ ] `cd src-tauri && cargo nextest run --locked` exits 0, and the five new tests named above exist and pass
- [ ] `grep -n "unwrap_or_default()" src-tauri/src/settings.rs | grep -c "from_value(merged)"` prints `1`, and `grep -c "from_slice" src-tauri/src/settings.rs src-tauri/src/bookmarks.rs src-tauri/src/library.rs` prints `0` for all three files
- [ ] `grep -c "write_json_durable" src-tauri/src/bookmarks.rs src-tauri/src/library.rs` prints `1` for each; `grep -c "write_json_durable" src-tauri/src/settings.rs` prints `0`
- [ ] `git status --short` lists only the in-scope files
- [ ] `git diff --stat` shows no whitespace-only reformatting (you did not run `cargo fmt`)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The code at the locations in "Current state" doesn't match the excerpts.
- `Settings`, `Bookmarks` or `Library` is constructed or loaded somewhere this
  plan doesn't mention (other than tests), so the signature change ripples
  further than `lib.rs` and `data_compat_tests.rs`.
- A released version's data file in `src-tauri/tests/data/` is reported as
  damaged by the new code. That would mean real users' files get moved aside:
  do not "fix" the test.
- A step's verification fails twice after a reasonable fix attempt.
- You find yourself wanting to change `history.rs`, `ui/`, or `Settings::save`.

## Maintenance notes

- If the engine (libVLC) fails to load, `setup` returns before the notices
  are queued, so the user sees the engine error and not the "couldn't be
  read" notices. The files are still moved aside and analytics is still off.
  Accepted for now.
- `history.rs` keeps its own copy of the aside logic. Moving it onto
  `store::read_json` is a safe follow-up but was left out to keep this change
  small.
- Any new JSON data file should be read with `store::read_json`, and saved
  with `write_json_durable` unless it is written many times a minute.
- Reviewer: check the notice wording, and that `settings.json` is rewritten
  only in the damaged case (a missing file must not be created on launch,
  or first-run detection elsewhere could change).
- A Cua scenario next to `corrupt_history` in `scripts/qa/ci/cua_suite.py`
  (corrupt `settings.json`, launch, expect the notice) would cover the notice
  end to end. Deferred: it needs a built app.
