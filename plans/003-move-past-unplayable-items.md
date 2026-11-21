# Plan 003: The queue moves past an item that can't play, and never reopens one in a loop

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat c190987..HEAD -- src-tauri/src/lib.rs src-tauri/src/playlist.rs`
> `lib.rs` may already contain the changes of plans 001 and 002 (inside
> `fn setup` and `Core::add_with_options`, plus a new `fn opened`). Those are
> expected and don't overlap this plan. For `fn pump_events` and
> `playlist.rs`, compare the "Current state" excerpts against the live code
> before proceeding; on a mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED (it changes the event loop that decides what plays next)
- **Depends on**: none (run after 001 and 002 if all are being done, since
  they edit the same file)
- **Category**: bug
- **Planned at**: commit `c190987`, 2026-11-11

## Why this matters

Two things go wrong when a queue item can't play:

1. **A tight reopen loop.** Some broken files end without an error, having
   played nothing. With Repeat One on (or Repeat All when nothing in the
   queue can play), the app reopens the same broken item again and again with
   no pause. Every round shows a "Could not play" notice, writes the history
   file, and queues a `playback_failed` analytics event.
2. **A stalled queue.** When libVLC reports an error (a missing file, an
   unreachable stream), the app shows a notice and stops there. The next item
   never starts, and `--play-and-exit` (documented as "Quit once the queue
   has played to its end") never quits.

After this plan: an item that was over without playing is never repeated, the
queue moves on to the next item after an error just as it does after a normal
end, the queue stops once every item in a row has failed, and
`--play-and-exit` exits when nothing is left to try.

## Current state

- `src-tauri/src/playlist.rs` — the queue (`Playlist`): items, the current
  index, repeat and shuffle, and the pure function that picks the next item.
- `src-tauri/src/lib.rs` — `fn pump_events` (starts at line 1100) turns
  libVLC events into decisions and UI events. It runs on its own thread.

`playlist.rs:149-174`, the function that picks the next item:

```rust
    /// The item to play after the current one. `auto` is true when the
    /// current item ended by itself (so "repeat one" applies).
    pub fn next_index(&self, auto: bool) -> Option<usize> {
        let len = self.items.len();
        if len == 0 {
            return None;
        }
        let Some(cur) = self.current else { return Some(0) };
        if auto && self.repeat == Repeat::One {
            return Some(cur);
        }
        if self.shuffle && len > 1 {
            let mut pick = random_index(len);
            if pick == cur {
                pick = (pick + 1) % len;
            }
            return Some(pick);
        }
        if cur + 1 < len {
            Some(cur + 1)
        } else if self.repeat != Repeat::Off {
            Some(0)
        } else {
            None
        }
    }
```

`lib.rs:1104-1123`, the top of the loop in `pump_events`:

```rust
    let mut last_time_emit = Instant::now() - TIME_INTERVAL;
    let mut last_progress = Instant::now();
    let mut loop_jump: Option<Instant> = None;
    // The last media that got past its start. Some broken files end
    // without an error, having played nothing.
    let mut played = None;
    while let Ok(first) = rx.recv() {
        // Let bursts of events (e.g. opening a file) arrive, then handle them together.
        thread::sleep(Duration::from_millis(20));
        let batch: Vec<vlc::Event> = std::iter::once(first).chain(rx.try_iter()).collect();
        let events: Vec<i32> = batch.iter().map(|e| e.kind).collect();
        let has = |ty: i32| events.contains(&ty);
        // Ending or failing only counts for what's open now, not for
        // media replaced since (the user picked another item meanwhile).
        let transition = core.transition();
        let generation = core.vlc.generation();
        let current = |ty: i32| batch.iter().any(|e| e.kind == ty && e.generation == generation);
        if current(event::TIME_CHANGED) && core.vlc.time() > 0 {
            played = Some(generation);
        }
```

`lib.rs:1135-1170`, the part this plan changes (the last `else if` branch and
the separate `ERROR` block after it):

```rust
        if subitems.len() == 1 && core.vlc.play_subitem() {
            core.quality().followed(core.vlc.generation());
            if let Some(uri) = core.current_uri() {
                core.set_resume(&uri);
            }
        } else if subitems.len() > 1 {
            core.quality().forget();
            if let Err(e) = core.expand_current(subitems) {
                core.notice("error", e);
            }
        } else if current(event::END_REACHED) {
            if played == Some(generation) {
                if let Some(uri) = core.current_uri() {
                    core.history().finish(&uri);
                    core.save_history();
                }
            } else {
                core.notice("error", format!("Could not play {}", core.current_title()));
            }
            let stop_after_each = core.settings().stop_after_each;
            let sleep_now = std::mem::take(&mut core.sleep().after_item);
            // Like VLC's --play-and-exit: quit once there's nothing left to play.
            if core.play_and_exit && core.playlist().next_index(true).is_none() {
                core.app.exit(0);
            }
            if sleep_now {
                core.notice("info", "Sleep timer: stopped after this item");
            } else if !stop_after_each
                && let Err(e) = core.advance_locked(true)
            {
                core.notice("error", e);
            }
        }
        if current(event::ERROR) {
            core.notice("error", format!("Could not play {}", core.current_title()));
        }
```

Facts you need:

- `generation` is a counter that changes every time the player opens or stops
  media. An event "counts" only if it carries the current generation
  (`current(...)`).
- `played == Some(generation)` means the media that is open now got past its
  start (a time update above zero).
- `event::END_REACHED` and `event::ERROR` are libVLC's two "this item is
  over" events. A failed open normally sends only `ERROR`; a broken file can
  send only `END_REACHED`. Do not rely on which one arrives, or on them
  arriving in the same batch: the code below handles one, the other, or both.
- `core.play_index_locked(index: usize) -> Result<(), String>` opens the item
  at `index`; the transition lock is already held at this point in the loop.
- `core.advance_locked(auto)` is `next_index(auto)` followed by
  `play_index_locked`. It stays in use for the Next button
  (`Core::advance(false)`); do not remove it or change its signature.
- `Repeat` has variants `Off`, `All`, `One`.

**Conventions to match** (this repo is hand-formatted):

- **Do NOT run `cargo fmt`.** There is no `rustfmt.toml`; lines run up to
  about 150 columns and `cargo fmt` would rewrite both files. Match the
  surrounding style by hand.
- `///` doc comments in plain sentences; comments explain why, briefly.
- No `unwrap()`/`expect()` outside tests (the release build aborts on panic).
- Tests go in the existing `#[cfg(test)] mod tests` at the bottom of
  `playlist.rs`, using its helper `fn list(n: usize) -> Playlist` (a queue of
  `n` items, nothing current). The existing test `next_and_repeat` is the
  pattern.

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

**Never launch the app.** The project forbids agent runs that could send
analytics, and test runs must be silent. The checks above are the whole
verification for this plan; the real-window check is listed under
"Maintenance notes" for the maintainer.

## Scope

**In scope** (the only files you should modify):
- `src-tauri/src/playlist.rs` — one new method and its tests.
- `src-tauri/src/lib.rs` — only inside `fn pump_events`.
- `plans/README.md` (status row only)

**Out of scope** (do NOT touch):
- `Playlist::next_index` and `Playlist::prev_index` — the Next and Previous
  buttons depend on them as they are.
- `Core::advance`, `Core::advance_locked`, `Core::play_index_locked`.
- `src-tauri/src/quality.rs` — it already records errors and ends for
  analytics; leave what it is told unchanged.
- The sub-item branches (`subitems.len() == 1` and `> 1`) of the same `if`.
- Shuffle behaviour in general (shuffle with Repeat Off currently plays on
  without end; that is a separate product question).
- `scripts/qa/ci/cua_suite.py` and anything in `ui/`.

## Git workflow

- Other agent threads edit the main checkout. Work in your own worktree:
  `git worktree add ../revlc-plan-003 -b fix/unplayable-items main`
  and never run `git commit -a` in the main checkout.
- Commit message style is a plain sentence, as in `git log`: `Memory video
  output: rotated video and video track changes`, `Leave the main window to
  setup on Linux and Windows too`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Add `Playlist::next_after` with tests

In `src-tauri/src/playlist.rs`, directly after `next_index` (before
`prev_index`), add:

```rust
    /// The item to play after the current one was over by itself. `failed`
    /// is how many items in a row, this one included, were over without
    /// playing anything (0 when it played). An item that failed isn't
    /// repeated, and once as many have failed in a row as the queue holds,
    /// the queue stops rather than going round for ever.
    pub fn next_after(&self, failed: usize) -> Option<usize> {
        if failed > 0 && (failed >= self.items.len() || self.repeat == Repeat::One) {
            return None;
        }
        self.next_index(true)
    }
```

Add to `mod tests` in the same file:

```rust
    #[test]
    fn what_played_goes_on_as_before() {
        let mut p = list(3);
        p.current = Some(2);
        assert_eq!(p.next_after(0), None);
        p.repeat = Repeat::All;
        assert_eq!(p.next_after(0), Some(0));
        p.repeat = Repeat::One;
        assert_eq!(p.next_after(0), Some(2));
    }

    #[test]
    fn what_failed_is_not_repeated() {
        let mut p = list(3);
        p.current = Some(0);
        // The next item gets its turn.
        assert_eq!(p.next_after(1), Some(1));
        p.repeat = Repeat::One;
        assert_eq!(p.next_after(1), None);
        // Round and round a queue where nothing plays: stop after one lap.
        p.repeat = Repeat::All;
        p.current = Some(2);
        assert_eq!(p.next_after(2), Some(0));
        assert_eq!(p.next_after(3), None);
        // Shuffle picks at random, so it's the count that ends it.
        p.shuffle = true;
        assert!(p.next_after(1).is_some_and(|i| i != 2));
        assert_eq!(p.next_after(3), None);
        // One item on repeat, or none at all.
        let mut one = list(1);
        one.current = Some(0);
        one.repeat = Repeat::All;
        assert_eq!(one.next_after(1), None);
        assert_eq!(Playlist::default().next_after(1), None);
    }
```

**Verify**: `cd src-tauri && cargo nextest run --locked what_played_goes_on what_failed_is_not` → 2 tests pass. (A "never used" warning for `next_after` in the non-test build is expected until step 2.)

### Step 2: Track failures in `pump_events`

In `src-tauri/src/lib.rs`, in `fn pump_events`, directly after the line
`let mut played = None;` add:

```rust
    // The media whose end or error has been dealt with, so one that sends
    // both, in the same batch of events or apart, is dealt with once.
    let mut over = None;
    // How many items in a row were over without playing anything.
    let mut failed = 0usize;
```

**Verify**: `cd src-tauri && cargo check --locked 2>&1 | tail -3` → `Finished` (warnings about `over`/`failed` being unused are expected until step 3).

### Step 3: Treat an error like an end, and pick the next item with `next_after`

In the same function, replace the whole `else if current(event::END_REACHED) { ... }`
branch **and** the separate `if current(event::ERROR) { ... }` block that
follows it (both shown in "Current state") with this single branch. The two
branches before it (`subitems.len() == 1 ...` and `subitems.len() > 1`) stay
exactly as they are:

```rust
        } else if (current(event::END_REACHED) || current(event::ERROR)) && over != Some(generation) {
            over = Some(generation);
            let did_play = played == Some(generation);
            if did_play && !current(event::ERROR) {
                if let Some(uri) = core.current_uri() {
                    core.history().finish(&uri);
                    core.save_history();
                }
            } else {
                // An error part-way keeps its place in the history, to resume from.
                core.notice("error", format!("Could not play {}", core.current_title()));
            }
            failed = if did_play { 0 } else { failed + 1 };
            let stop_after_each = core.settings().stop_after_each;
            let sleep_now = std::mem::take(&mut core.sleep().after_item);
            // Not what just failed again, and not round and round a queue where nothing plays.
            let next = core.playlist().next_after(failed);
            // Like VLC's --play-and-exit: quit once there's nothing left to play.
            if core.play_and_exit && next.is_none() {
                core.app.exit(0);
            }
            if sleep_now {
                core.notice("info", "Sleep timer: stopped after this item");
            } else if !stop_after_each
                && let Some(index) = next
                && let Err(e) = core.play_index_locked(index)
            {
                core.notice("error", e);
            }
        }
```

Check the braces: the `if subitems.len() == 1 && ...` chain must still close
once, and the code that follows (the comment `// The resume point belongs to
what's open now` and `if current(event::PLAYING) {`) must be unchanged.

**Verify**:
- `cd src-tauri && cargo check --locked 2>&1 | tail -3` → `Finished`, no warnings.
- `grep -c "Could not play" src-tauri/src/lib.rs` → `1` (it was `2`).
- `grep -c "next_after" src-tauri/src/lib.rs` → `1`.
- `grep -n "advance_locked(true)" src-tauri/src/lib.rs` → no output.

### Step 4: Full checks

**Verify**:
- `cd src-tauri && cargo clippy --locked --all-targets -- -D warnings` → exit 0.
  If clippy reports that the `auto` parameter of `advance_locked` is now
  always `false`, do NOT change `advance_locked`: stop and report (see STOP
  conditions).
- `cd src-tauri && cargo nextest run --locked` → all pass.

## Test plan

- New unit tests in `playlist.rs`: `what_played_goes_on_as_before` (the
  behaviour for items that played is unchanged for Repeat Off, All and One)
  and `what_failed_is_not_repeated` (next item after a failure, Repeat One
  stops, a full lap of failures stops, shuffle stops by count, single-item
  and empty queues).
- Pattern: the existing `next_and_repeat` test in the same file.
- `pump_events` needs a real player and window and has no unit test in this
  repo; the decision it makes now lives in `next_after`, which is tested. The
  end-to-end check is listed under "Maintenance notes".

## Done criteria

ALL must hold:

- [ ] `cd src-tauri && cargo clippy --locked --all-targets -- -D warnings` exits 0
- [ ] `cd src-tauri && cargo nextest run --locked` exits 0; the two new tests exist and pass
- [ ] `grep -c "Could not play" src-tauri/src/lib.rs` prints `1`
- [ ] `grep -n "advance_locked(true)" src-tauri/src/lib.rs` returns no matches
- [ ] `git diff c190987 -- src-tauri/src/playlist.rs` shows only additions (no changed or removed lines in `next_index`/`prev_index`)
- [ ] `git status --short` lists only `src-tauri/src/lib.rs`, `src-tauri/src/playlist.rs` and `plans/README.md`
- [ ] `git diff --stat` shows a small diff (you did not run `cargo fmt`)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- `pump_events` doesn't match the excerpts: in particular if the
  `END_REACHED` branch or the `ERROR` block has already been changed.
- Clippy or the compiler objects to anything in `advance_locked`,
  `next_index` or `quality.rs` after your change.
- You believe the fix needs a sleep, a timer, or a retry counter stored
  anywhere other than the two local variables from step 2.
- A step's verification fails twice after a reasonable fix attempt.

## Maintenance notes

- **Behaviour changes to review:** (1) after a libVLC error the queue now
  moves to the next item, including for a stream that fails part-way; (2) an
  item that fails under Repeat One stops playback instead of retrying;
  (3) `--play-and-exit` exits after an error on the last item; (4) one batch
  with both an error and an end now shows one notice, not two.
- **Not covered here:** `play_index_locked` returning `Err` straight away
  (libVLC refusing to create or start the media) still stops the queue with a
  notice. It is rare; handle it with the same `failed` counter if it shows up.
- **For the maintainer to check in the real app** (needs a build, silent,
  analytics off): a queue of a good file, a zero-byte `.mp4` and another good
  file plays the third after a notice for the second; a single zero-byte file
  with Repeat One shows one notice and stops; `revlc --play-and-exit
  /no/such/file.mp4` exits. A matching scenario fits next to
  `command_line-exit` in `scripts/qa/ci/cua_suite.py`.
- **Assumption worth an engine test later:** that a failed open sends `ERROR`
  without `END_REACHED`. The code here is correct either way, but the stalled
  queue only happens if that holds. `vlc_it_broken_files_end_cleanly_and_the_next_file_plays`
  in `src-tauri/src/vlc_it.rs` is where a check on the events would go.
- If shuffle is ever changed to play each item once, `next_after`'s count is
  the place that ends a lap.
