# Plan 005: ReVLC has a tested host for VLC-style Lua extensions, not yet wired into the app

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat c190987..HEAD -- src-tauri/Cargo.toml src-tauri/src/lib.rs licenses/README.md`
> Changes inside `lib.rs` functions are expected (plans 001 to 003 edit
> them). What matters here is the `mod` list at the top of `lib.rs` and the
> `[dependencies]` table in `Cargo.toml`: compare them with the excerpts in
> "Current state". On a mismatch, treat it as a STOP condition.
> Also run `ls src-tauri/src/extensions.rs`: it must not exist yet.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW (new code that nothing in the app calls yet)
- **Depends on**: none (independent of plans 001 to 004)
- **Category**: direction
- **Planned at**: commit `c190987`, 2026-11-11

## Why this matters

VLC users extend the player with Lua scripts ("extensions": subtitle
finders, lyrics, resume helpers). `docs/parity.md` lists "Lua extensions" as
not done, and nothing in ReVLC loads them: libVLC, the library ReVLC plays
with, has no way to start one. The decision recorded in
`plans/extensions-design.md` is to run these scripts in ReVLC's own Rust
code, with a Lua 5.1 interpreter (the Lua version VLC 3 ships) built into
the app, and to give the scripts VLC's `vlc.*` functions backed by ReVLC's
player.

This plan is the first of four phases. It adds the interpreter and one new
module that finds extensions, runs their hooks and limits them, with unit
tests. Nothing in the app calls the module yet, so users see no change. The
next phase adds the Extensions menu.

The module was written and tested in a scratch crate outside this
repository (macOS, Apple Silicon, Rust 1.93.1, `mlua` 0.12.2): 16 tests
pass and `cargo clippy --all-targets -- -D warnings` is clean. Your job is
to bring it into the app's crate and prove it builds, lints and passes
there. You are not expected to design anything.

## Current state

Files and their roles:

- `src-tauri/Cargo.toml` — the app crate's manifest. You add one dependency.
- `src-tauri/src/lib.rs` — the crate root. You add one `mod` line.
- `src-tauri/src/extensions.rs` — does not exist. You create it by copying
  `plans/attachments/005-extensions.rs` unchanged.
- `licenses/README.md` — third-party notices shipped with the app. You add
  a short section for Lua.
- `plans/attachments/005-extensions.rs` — the module, 976 lines: about 585
  of code, then `#[cfg(test)] mod tests` with 16 tests. SHA-256:
  `a09f51491370ea201510b0e5c0909737c243be1b0cd203784fab03944d4e69d4`.

`src-tauri/Cargo.toml`, the end of `[dependencies]` (lines 27 to 32 at the
planned commit):

```toml
# Analytics; the same HTTP and TLS crates the updater uses.
reqwest = { version = "0.13", default-features = false, features = ["json", "rustls-no-provider", "system-proxy"] }
rustls = { version = "0.23", default-features = false, features = ["ring"] }
uuid = { version = "1", features = ["v4"] }

[target.'cfg(target_os = "macos")'.dependencies]
```

`src-tauri/src/lib.rs`, lines 1 to 12:

```rust
mod bookmarks;
mod capture;
mod cli;
mod disc;
mod effects;
mod history;
mod keyframes;
mod library;
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
mod machine;
mod menu;
mod meta;
```

`licenses/README.md` has sections in this order: `## libVLC and VLC plugins`,
`## FFmpeg`, `## Rust and JavaScript dependencies`, `## Media`. The third
one reads:

```markdown
## Rust and JavaScript dependencies

The licences and copyright notices of the Rust crates and JavaScript
packages compiled into ReVLC are in `THIRD-PARTY-RUST.txt` and
`THIRD-PARTY-JS.txt`. Regenerate them with `scripts/third-party-licenses.sh`
after changing dependencies.
```

What the module provides (so you can read test failures; do not change it):

| Item | Role |
|---|---|
| `scan(dir) -> Scan` | Reads the descriptor of every `.lua` file in a folder; lists the ones that failed and why |
| `describe(path) -> Result<Descriptor, String>` | Runs one script with no libraries and calls its `descriptor()` |
| `trait Host` | What a script can ask of the player. The tests implement it with a recorder; the app will implement it later |
| `Extension::activate(&descriptor, host)` | Starts a thread and a Lua state, runs `activate()`. Methods `menu`, `trigger_menu`, `input_changed`, `playing_changed`, `meta_changed`, `deactivate`, `is_alive` |
| `trigger(&descriptor, host)` | Runs a trigger-only extension's `trigger()` once |
| `Limits`, `Watch` | 10 seconds per hook, 2 seconds for a descriptor, 64 MiB of Lua memory; a script past its time is stopped |
| `PRELUDE` | Lua run before each script: removes `os.exit` and `string.dump`, makes `loadstring`, `load`, `loadfile` and `dofile` refuse compiled Lua, and makes `require` report that modules are not available |

**Conventions to match** (the attachment already follows them):

- Rust in this repository is formatted by hand with long lines. There is no
  `rustfmt.toml`, and `cargo fmt --check` reports about 500 differences
  across the crate. **Never run `cargo fmt`**, and do not let an editor
  format on save.
- Errors are `Result<_, String>`. Mutexes are locked with
  `.lock().unwrap_or_else(|e| e.into_inner())`. No `unwrap()` outside
  tests.
- Tests live in `#[cfg(test)] mod tests` at the end of the file and use
  temporary folders named with `std::process::id()`.
- Comments are plain sentences that say what the code is for.

## Commands you will need

Run from the repository root unless a row says otherwise.

| Purpose | Command | Expected on success |
|---|---|---|
| Install UI packages | `npm ci` | exit 0 |
| Build the UI (the Rust build embeds it) | `npm run build` | exit 0 |
| Folders the Rust build expects | `mkdir -p src-tauri/vlc src-tauri/binaries` | exit 0 |
| FFmpeg stand-in (macOS and Windows) | the `touch` command below this table | exit 0 |
| Update `Cargo.lock` for the new dependency | `cd src-tauri && cargo check` | exit 0 |
| Lint | `cd src-tauri && cargo clippy --locked --all-targets -- -D warnings` | exit 0, no warnings |
| The new tests | `cd src-tauri && cargo nextest run --locked extensions::tests` | `16 tests run: 16 passed` |
| All Rust tests | `cd src-tauri && cargo nextest run --locked` | all pass |

The FFmpeg stand-in, to copy exactly (on Windows in Git Bash, add `.exe` to
the file name):

```sh
touch "src-tauri/binaries/ffmpeg-$(rustc -vV | sed -n 's/^host: //p')"
```

If `cargo nextest` is not installed, `cargo test --locked extensions::tests`
runs the same tests; expect `16 passed`.

`mlua` 0.12 needs Rust 1.88 or newer (`rustc -V`) and a C compiler, because
it builds Lua from source: Xcode's command-line tools on macOS,
`build-essential` on Linux, the MSVC build tools on Windows.

## Scope

**In scope** (the only files you should modify or create):

- `src-tauri/Cargo.toml`
- `src-tauri/Cargo.lock` (changed by Cargo, not by hand)
- `src-tauri/src/extensions.rs` (create, as a copy of the attachment)
- `src-tauri/src/lib.rs` (three added lines in the `mod` list, nothing else)
- `licenses/README.md`
- `licenses/THIRD-PARTY-RUST.txt` (only by running the script in step 6, and
  only if its tool is installed)
- `plans/README.md` (status row only)

**Out of scope** (do NOT touch):

- Any other part of `src-tauri/src/lib.rs`: no `Host` implementation for
  `Core`, no Tauri commands, no calls into the new module. That is the next
  phase, and it has decisions the maintainer has not made yet.
- Anything under `ui/`, `tests/`, `scripts/`, `.github/`.
- `docs/parity.md`: "Lua extensions" stays unchecked, because users cannot
  use them yet.
- The contents of the attachment. If something in it must change for the
  build or the tests to pass, see "STOP conditions".
- VLC's own files under `src-tauri/vlc`. ReVLC's extensions do not use
  VLC's Lua plugin.

## Git workflow

- Other agent threads edit the main checkout. Work in your own worktree:
  `git worktree add ../revlc-plan-005 -b feat/extensions-lua-host main`
  and never run `git commit -a` in the main checkout.
- `plans/` may not be committed yet, in which case your worktree has no
  `plans/` folder. Read this plan and copy the attachment from the checkout
  where you found them; step 2 shows how.
- Commit message style is a plain sentence with an area prefix, as in
  `git log`: `Settings: one unreadable value takes its default on its own`,
  `Memory video output: rotated video and video track changes`. For this
  plan: `Extensions: a Lua host for VLC-style extensions, not yet in the app`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Add the dependency

In `src-tauri/Cargo.toml`, add two lines after the `uuid` line, so the end
of `[dependencies]` reads:

```toml
uuid = { version = "1", features = ["v4"] }
# Runs VLC-style Lua extensions (extensions.rs): Lua 5.1, the version VLC 3 uses, built from source.
mlua = { version = "0.12", features = ["lua51", "vendored"] }
```

Do not add any other `mlua` feature. In particular not `send` (each Lua
state stays on the thread that made it), not `luajit`, not `lua54`.

Prepare the build inputs (`npm ci`, `npm run build`, the `mkdir` and `touch`
commands from the table), then run `cd src-tauri && cargo check`. This
downloads `mlua` and writes it into `Cargo.lock`.

**Verify**:
- `cd src-tauri && cargo check --locked` → exit 0.
- `grep -c '^name = "mlua"$' src-tauri/Cargo.lock` → `1`.
- `grep -c '^name = "lua-src"$' src-tauri/Cargo.lock` → `1`.
- `git diff --stat` → only `src-tauri/Cargo.toml` and `src-tauri/Cargo.lock`.

### Step 2: Add the module

Copy the attachment to `src-tauri/src/extensions.rs`, byte for byte. From
your worktree, where `<plans>` is the `plans` folder you are reading this
plan in:

```sh
cp <plans>/attachments/005-extensions.rs src-tauri/src/extensions.rs
```

**Verify**:
- `shasum -a 256 src-tauri/src/extensions.rs` → starts with
  `a09f51491370ea201510b0e5c0909737c243be1b0cd203784fab03944d4e69d4`
  (on Windows: `sha256sum`).
- `grep -c '#\[test\]' src-tauri/src/extensions.rs` → `16`.

### Step 3: Declare the module

In `src-tauri/src/lib.rs`, add three lines between `mod effects;` and
`mod history;`:

```rust
mod effects;
// Nothing in the app uses these yet; the Extensions menu will.
#[allow(dead_code)]
mod extensions;
mod history;
```

The `allow` is needed: nothing calls the module yet, and CI treats the
resulting "never used" warnings as errors. The phase that wires the module
in removes it.

**Verify**:
- `git diff -- src-tauri/src/lib.rs | grep -c '^+[^+]'` → `3`.
- `git diff -- src-tauri/src/lib.rs | grep -c '^-[^-]'` → `0`.
- `cd src-tauri && cargo check --locked` → exit 0.

### Step 4: Run the new tests

**Verify**: `cd src-tauri && cargo nextest run --locked extensions::tests`
→ a `Summary` line with `16 tests run: 16 passed`. The run takes a second
or two: some tests wait on purpose for a script to be stopped.

The 16 tests, so you can tell that none is missing:

```
a_descriptor_is_read
a_missing_title_is_the_file_name
a_scan_lists_lua_files_in_name_order_and_says_which_failed
a_descriptor_that_never_returns_is_stopped
the_hooks_run_in_order_and_only_the_declared_listeners
scripts_reach_the_player_through_the_host
nothing_playing_is_nil
an_extension_that_cannot_activate_is_not_started
a_failing_hook_is_reported_and_the_extension_stays
a_stuck_script_is_stopped_without_its_deactivate
keep_alive_gives_a_long_hook_more_time
vlc_deactivate_ends_the_extension_after_the_running_hook
dropping_an_extension_ends_its_thread
a_trigger_only_extension_runs_once
scripts_cannot_load_bytecode_modules_or_end_the_app
a_script_that_uses_too_much_memory_fails_its_hook
```

Then run them five more times to confirm they are not timing-dependent on
your machine:

```sh
cd src-tauri && for i in 1 2 3 4 5; do cargo nextest run --locked extensions::tests 2>&1 | grep Summary; done
```

**Verify**: five `Summary` lines, each with `16 tests run: 16 passed`.

### Step 5: Lint and run everything

**Verify**:
- `cd src-tauri && cargo clippy --locked --all-targets -- -D warnings` →
  exit 0.
- `cd src-tauri && cargo nextest run --locked` → all pass. The count is the
  count before your change plus 16; nothing that passed before may fail.

### Step 6: Licence notices

Lua is compiled into the app from now on, so its notice must ship.

In `licenses/README.md`, add this section between `## FFmpeg` (after its
last paragraph, the one about the Linux package and the VLC trademark) and
`## Rust and JavaScript dependencies`:

```markdown
## Lua

ReVLC includes the Lua 5.1.5 interpreter, unmodified, to run extensions. It
is Copyright (C) 1994-2012 Lua.org, PUC-Rio, under the MIT licence:
https://www.lua.org/license.html

```

Check where exactly the FFmpeg section ends before inserting: the paragraph
that begins `"VLC" is a trademark of VideoLAN.` belongs to the sections
above and must stay above the new `## Lua` heading.

Then regenerate the crate notices if the tool is available:

```sh
cargo about --version && bash scripts/third-party-licenses.sh
```

If `cargo about` is not installed, do not install it. Leave
`licenses/THIRD-PARTY-RUST.txt` as it is and say so in your report: the
release steps in `README.md` regenerate it before each release.

**Verify**:
- `grep -c '^## Lua$' licenses/README.md` → `1`.
- `grep -n '^## ' licenses/README.md` → the headings in this order: libVLC
  and VLC plugins, FFmpeg, Lua, Rust and JavaScript dependencies, Media.
- If the script ran: `grep -c 'mlua' licenses/THIRD-PARTY-RUST.txt` → at
  least `1`.

### Step 7: Check that Lua's symbols stay private (Linux and macOS)

VLC's own Lua plugin also contains or links a Lua. ReVLC's copy must not be
visible to it. Build the program and count the exported Lua symbols.

On Linux:

```sh
cd src-tauri && cargo build --locked && nm -D --defined-only target/debug/revlc | grep -c ' lua_'
```

**Verify** on Linux: prints `0`.

On macOS the plugin carries its own private copy and exported symbols in
the program are not searched by it, so this check does not apply. On
Windows it does not apply either. If you are not on Linux, write in your
report that this step was left to CI's Linux job and the reviewer.

## Test plan

- The 16 tests arrive with the module; no new tests to write. They cover:
  reading a descriptor and its capabilities; scanning a folder with valid,
  broken, compiled and unrelated files; the order of hooks; listeners that
  were not declared are not called; every `vlc.*` function reaching `Host`;
  nothing playing; activation failures; a failing hook; a script stopped
  for running too long, with and without `vlc.keep_alive()`;
  `vlc.deactivate()`; dropping an extension; trigger-only extensions; the
  refusal of bytecode, `os.exit`, `debug`, `package` and `require`; the
  memory limit.
- No libVLC, window or network is involved, so these are ordinary unit
  tests (the first layer in `docs/testing.md`) and run on all three
  platforms in the `Checks` workflow.
- Verification: `cd src-tauri && cargo nextest run --locked` → all pass,
  including the 16 new tests.

## Done criteria

ALL must hold:

- [ ] `src-tauri/src/extensions.rs` has the SHA-256 given in step 2
- [ ] `cd src-tauri && cargo clippy --locked --all-targets -- -D warnings` exits 0
- [ ] `cd src-tauri && cargo nextest run --locked` exits 0, with 16 tests whose names start with `extensions::tests::`
- [ ] `git diff c190987 -- src-tauri/src/lib.rs` shows this plan's three added lines in the `mod` list and no other change made by you
- [ ] `grep -c 'features = \["lua51", "vendored"\]' src-tauri/Cargo.toml` → `1`
- [ ] `grep -c '^## Lua$' licenses/README.md` → `1`
- [ ] `git status --short` lists only the files in "In scope"
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- `cargo check` fails while building `mlua-sys` or `lua-src` (no C
  compiler, or a platform problem). Report the full error. Do not switch to
  another Lua version, to `luajit`, or to a system Lua.
- Cargo selects an `mlua` version other than 0.12.x, or the attachment does
  not compile against the version it selected. Report the compiler errors;
  do not rewrite the module to fit.
- Any of the 16 tests fails, on the first run or in the five repeats. Report
  the test name and output. Do not edit a test, raise a time limit, or mark
  a test ignored to get a pass.
- Clippy reports a warning inside `extensions.rs` (a newer Clippy than the
  one it was checked with). Report the warning. A one-line fix that does
  not change behaviour is acceptable only if the operator agrees, because
  it changes the checksum in step 2.
- The `mod` list in `lib.rs` already contains `extensions`, or
  `src-tauri/src/extensions.rs` already exists.
- The Linux symbol check in step 7 prints anything but `0`.
- You find yourself wanting to call the module from `lib.rs` or to add a
  command or a menu. That is the next phase.

## Maintenance notes

- **Decisions this plan carries, for the maintainer to confirm before it
  runs**: Lua 5.1 through `mlua` as the extension language, and extensions
  that run with the user's full rights once the user has turned them on.
  Both are argued in `plans/extensions-design.md`. Nothing here loads an
  extension in the running app, so merging this does not commit to the
  trust model; it does add the dependency.
- The next phase (`Host` for `Core`, commands, the Extensions menu and the
  approval prompt) is outlined in `plans/extensions-design.md` and is to be
  written as its own plan once this one has merged. It removes the
  `#[allow(dead_code)]` from `lib.rs`.
- Rules the next phases must keep: never call `Extension::menu` or
  `Extension::deactivate` while holding a lock that a `Host` method takes
  (both wait for the extension's thread); do not join an extension's thread
  from inside `Host::stopped` (it is called on that thread); give every
  blocking `Host` call its own timeout, because the time limit can only
  stop a script between Lua instructions.
- `Extension::deactivate` can wait up to the hook limit (10 seconds) for a
  running hook. Call it from a background thread, never from the UI thread.
- A script that catches the "took too long" error in a `pcall` loop is not
  stopped by the limit. This is accepted: the limit is for accidents, and
  an extension is code the user chose to run.
- Reviewer: read `PRELUDE` and the test
  `scripts_cannot_load_bytecode_modules_or_end_the_app` together; confirm
  the Windows and Linux jobs of `Checks` are green, since the module was
  only ever built on macOS; and confirm step 7's result on Linux.
- Binary size: the scratch build grew by about 0.35 MB with this module and
  Lua (release profile, Apple Silicon).
