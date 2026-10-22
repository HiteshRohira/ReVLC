# Audit / QA playbook

How to run a hands-on QA pass of a ReVLC branch on macOS and Windows at the
same time. It is based on the `audit-fixes-oct` audit (PR #4, 2026-10-22).
That audit found 10 macOS and 11 Windows issues, with 83 + 184 screenshots and
37 recordings. Both machines were left exactly as they were found.

The setup: you (the orchestrator, on the Mac) start two QA threads, one on this
Mac and one on the Windows PC. You watch them, then merge their reports into
one answer for the user. The QA threads only test. They never change code.

## 0. Before you start (orchestrator, about 3 min)

Each item here prevented a delay from last time (see section 6).

- [ ] **Mac checkout.** `git -C ~/projects/revlc status --short` is empty, and
      `git log -1` matches the PR head. If the root is on another branch, use a
      worktree, and in the prompt tell the thread to share the warm build dir:
      `CARGO_TARGET_DIR=~/projects/revlc/src-tauri/target`. A cold build costs
      about 7 min.
- [ ] **t3-windows is connected.** `mcp__t3-windows__t3_project_list` answers.
- [ ] **Mac screen is unlocked.** `ioreg -n Root -d1 | grep CGSSessionScreenIsLocked`
      shows nothing or `No`. Ask the user to keep both machines awake and
      unlocked for about 90 min.
- [ ] **Mac tools are present.**
  - `command -v ffmpeg ffprobe`
  - `~/.tauri/revlc-updater.key` exists, so `npm run build:app` can sign the
    updater archive.
- [ ] **Fixtures and helpers are present.** `~/revlc-qa/media/` exists. Copy
      the newest run's helpers into the new run folder (or `~/revlc-qa/tools/`)
      and name them in the prompt, so the thread reuses them instead of
      rewriting them:
  - Mac: `logs/shot.sh`, `logs/desktop.swift` (and the compiled `logs/desktop`), `logs/generate-media.py`, `ReVLC-QA-Debug.app`
  - Windows (in `C:\Users\hites\revlc-qa\win-<date>\`): `qa-desktop.ps1`, `qa-cdp.mjs`, `qa-native-tests.mjs`, `qa-capture-webview.mjs`, `patch-vlc-fixture.cjs`, `generate-entry-cap.cjs`, the DPI script, and `tools\ffmpeg-*-essentials_build\bin\`
- [ ] **Facts in the prompt are current.**
  - PR head SHA
  - the change list, pasted from the PR body
  - the accepted trade-offs (memory `accepted-tradeoffs`)
  - the installed ReVLC version on the PC. The Windows thread checks this in
    its preflight.
- [ ] **Launch Windows first.** It's the critical path.

## 1. Launch the two threads

Find the model IDs first. Call `orchestrator_capabilities` on both servers
(`mcp__t3-code__…` for the Mac, `mcp__t3-windows__…` for the PC). Then call
`t3_project_list` on both to get the ReVLC project IDs:

| | Mac | Windows PC ("GalaxyDestroyer") |
|---|---|---|
| MCP server | `t3-code` | `t3-windows` (user-scope MCP, relay URL; `claude mcp list` should show it Connected) |
| Project | `/Users/hitesh/projects/revlc` (id `f247a241-9294-4f6f-912f-453c2d8db597`) | `C:\Users\hites\code\ReVLC` (id `92eba796-4cfa-4c97-86e8-3149df4028df`) |
| Model | `codex` / `gpt-6.1-sol`, `reasoningEffort: high` | same |
| Runtime mode | `full-access` (it needs to build, launch apps, take screenshots) | same |
| Workspace | `{"type":"root"}` if the root checkout is already on the branch and clean. This keeps `src-tauri/target` warm. Otherwise use a worktree. | `{"type":"worktree","baseRef":"<branch>","branch":"qa-<branch>-win","startFromOrigin":true}`. The branch usually only exists on origin there. |

The user asked for GPT-6.1-Sol. If they name another model, use theirs.
Use `high` effort. The default `low` is too shallow for this job.

Mac launch, shortened:

```json
{
  "title": "ReVLC <branch>: macOS QA",
  "projectId": "f247a241-9294-4f6f-912f-453c2d8db597",
  "workspaceStrategy": {"type": "root"},
  "modelSelection": {"instanceId": "codex", "model": "gpt-6.1-sol",
                     "options": [{"id": "reasoningEffort", "value": "high"}]},
  "runtimeMode": "full-access",
  "message": "<Mac prompt from section 3>"
}
```

The Windows launch is the same call on `mcp__t3-windows__t3_thread_launch`, with
the Windows project ID, the worktree strategy above and the Windows prompt.
Right after launching, read each thread once (`t3_thread_read`, `view:
"activity"`, `limit: 5`). This confirms that workspace preparation finished and
the run started.

### Monitoring

- Mac: `mcp__t3-code__t3_thread_wait` with `timeoutMs: 600000` works. It returns
  `timedOut: true` while the thread is still running.
- Windows: `mcp__t3-windows__t3_thread_wait` fails with "The operation timed
  out" after about 60 s, whatever `timeoutMs` you pass. Don't use it. Poll with
  `mcp__t3-windows__t3_thread_read` (`view: "messages"`, `afterPosition` = the
  last position you saw, small `limit`, `maxCharsPerItem` ≈ 600).
- Big activity reads overflow the tool result. Page with `limit` 30–60 and
  `afterPosition`, and cap `maxCharsPerItem`. If a result gets saved to a file,
  pull out what you need with `jq`.
- Give the user a one-line update when a thread reports something new: a
  confirmed bug, a build result, the cleanup stage. Don't post updates when
  nothing has changed.
- Expect the Windows thread to take longer. Last time Mac took about 54 min and
  Windows about 88 min.

## 2. Ground rules (put these in every prompt)

- **Test only.** No source edits, commits, pushes, merges or PR comments. Report
  each bug with repro steps and a likely cause (`file:line`). Don't fix
  anything. Leave `git status` clean. Fixtures, harnesses and evidence go
  outside the repo.
- **Silent runs.** The user wears AirPods. Always launch with
  `REVLC_VLC_ARGS=--no-audio` and never play audio out loud. On macOS, never
  start the app by name or bundle ID (`cua.getApp("revlc")` launches the
  installed `/Applications/ReVLC.app` without the flag). Always launch the
  exact binary path with the env var set.
- **Kill and confirm.**
  - macOS: `pkill -9 -f target/debug/revlc`, `pkill -9 -f target/release/bundle/macos/ReVLC.app/Contents/MacOS/revlc`, and the wrapper path (see 4.1). Confirm with `pgrep -fl revlc`.
  - Windows: `taskkill /F /IM revlc.exe`, then confirm `Get-Process revlc` returns nothing.
- **Back up app data first and restore it exactly at the end.** Debug and
  release builds share the real data dirs.
  - macOS: `~/Library/Application Support/app.revlc` (history.json,
    settings.json, analytics.json) and `~/Library/Caches/app.revlc` (thumbs/,
    meta.json, WebKit/).
  - Windows: `%APPDATA%\app.revlc` and `%LOCALAPPDATA%\app.revlc`.
  - Back up into the run's evidence folder (`…/backup/`), not `/tmp`.
  - Verify the restore with a hash manifest (see 4.4).
- **Installed app on Windows.** The PC has ReVLC 0.2.6 installed in
  `%LOCALAPPDATA%\ReVLC` (1,179 files). Before installing a test build, back up:
  - the install folder's file hashes
  - the Start Menu shortcut `%APPDATA%\Microsoft\Windows\Start Menu\Programs\ReVLC.lnk`
  - registry exports of `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\ReVLC`, `HKCU\Software\Classes\ReVLC.*`, the `.ext\OpenWithProgids` entries, and `RegisteredApplications`

  Install the test build into an empty folder under the evidence dir (e.g.
  `…\Installed ReVLC Test`), not over 0.2.6. Afterwards: uninstall it,
  re-import the registry, and hash-check the original files.
- **Locked screen.** A locked screen blanks the web view and drops synthetic
  input. Check before trusting any screenshot. Never report a blank UI as a
  bug without that check.
  - macOS: `ioreg -n Root -d1 | grep CGSSessionScreenIsLocked`.
  - Windows: `OpenInputDesktop` returns non-zero, plus a desktop screenshot.
- **Accepted trade-offs. Don't flag these:**
  - no notarization or code signing
  - no Intel or extra builds
  - analytics on by default
  - password URLs in bookmarks
  - auto-update on by default
  - query-token stream URLs kept in history
  - Windows SMB/UNC paths from playlists behaving like VLC

  Also leave out anything the change list marks as deliberately not changed.
- **No localhost links in the report.**

## 3. Prompt template

Fill in the `{…}` placeholders. The two prompts share everything except the
blocks marked **[Mac]** and **[Windows]**. Paste the change list from the PR
description, grouped as the PR groups it. Phrase each item as something to
test, and include how to test it when that isn't obvious (e.g. "corrupt
history.json deliberately, launch, screenshot the notice").

```markdown
You are a QA tester for ReVLC (Rust + Tauri 2 + Svelte + libVLC 3 media player;
macOS-first, Windows supported). Test the `{BRANCH}` branch (PR #{PR},
{PR_URL}, head {SHORT_SHA}) on this {Mac|Windows PC} thoroughly, then report
every bug, issue, UI bug and UI inconsistency you find. The user wants to see
the evidence themselves, so take a LOT of screenshots and several screen
recordings.

## Ground rules
- This is test-only. Do NOT edit source, commit, push, merge, or comment on the
  PR. If you find a bug, report it with repro steps and a guess at the cause
  (file:line); don't fix it. Leave `git status` clean.
- [Mac] The checkout is already on `{BRANCH}`.
- [Windows] You're in a throwaway worktree branch `qa-{BRANCH}-win`; first
  confirm HEAD is {SHORT_SHA} (`git log -1`); if not, `git fetch origin` and
  check out `origin/{BRANCH}`.
- Silent runs: always launch with `REVLC_VLC_ARGS=--no-audio`. Never play audio
  out loud. [Mac] Never launch ReVLC by name/bundle id (that starts the
  installed /Applications copy); always launch the exact binary path.
- Kill: [Mac] `pkill -9 -f target/debug/revlc` (and the release/wrapper path),
  confirm with `pgrep`. [Windows] `taskkill /F /IM revlc.exe`, confirm with
  Get-Process.
- Debug and release builds share the real data dirs. Back them up FIRST into
  your evidence folder's `backup/` and restore them exactly when you finish,
  verified by a hash manifest.
  [Mac] `~/Library/Application Support/app.revlc` and `~/Library/Caches/app.revlc`.
  [Windows] `%APPDATA%\app.revlc` and `%LOCALAPPDATA%\app.revlc`.
- [Windows] ReVLC 0.2.6 is installed in `%LOCALAPPDATA%\ReVLC`. Before
  installing a test build, back up its file hashes, Start Menu shortcut and
  registry (Uninstall\ReVLC, Classes\ReVLC.*, OpenWithProgids,
  RegisteredApplications). Install the test build into an empty folder under
  your evidence dir. At the end uninstall it, restore the registry and verify
  the original files by hash.
- A locked screen or disconnected session blanks the web view and drops
  synthetic input. Check before trusting a screenshot; never report a blank UI
  without that check.
- Don't flag these accepted trade-offs: {ACCEPTED_TRADEOFFS}.
- No localhost links in your report.

## Working efficiently
- Preflight (≤3 min), before anything else: screen unlocked; ffmpeg/ffprobe
  found (reuse {HELPER_DIR} and {FFMPEG_DIR}; [Windows] don't use winget for
  ffmpeg, it timed out last time); data dirs backed up; [Windows] installed
  ReVLC version and install path recorded.
- Reuse the helpers in {HELPER_DIR} (screenshots, recording, key/mouse input,
  [Windows] CDP driver and WebView capture) instead of writing new ones.
- Start the release build first, in the background, as soon as dependencies
  are installed. It's the longest step and the installed-app tests wait on it.
  [Windows] Set `CARGO_TARGET_DIR=%USERPROFILE%\revlc-qa\cargo-target` so later
  audits reuse the build. The installer is then under
  `%CARGO_TARGET_DIR%\release\bundle\nsis\`.
- Open media by passing paths on the command line ([Mac]
  `…/revlc <video> <subs.srt>`) or ([Windows]) with CDP
  `invoke("open",{uris,play:true})`. Only drive the native file picker when the
  picker itself is under test.
- Run ffmpeg with its working directory set to the fixture folder. fMP4 HLS
  writes `init.mp4` relative to the cwd, and last time it landed in the repo.
- 200k cap: test through the app with a `000-before-cap.mp4` / 200,005 empty
  files / `zzz-after-cap.mp4` folder, not a compiled harness.
- Stalled-stream tests: have the local server hold the segment until a flag
  file is deleted, not for a fixed sleep. A 45 s hold was too short last time.
- Append each finding to `logs/findings.md` as soon as it's confirmed (ID,
  severity, repro, expected/actual, evidence, cause). Write the PASS/FAIL
  table from your test log as you go. The final report is then assembly, not
  writing.
- Time box: spend at most 10 min on any one investigation or automation
  problem. If it's still stuck, record it as NOT TESTED with the reason and
  move on.
- Before cleanup, review every screenshot you'll cite (open a contact sheet or
  sample them): no covering windows, no blank frames, the right window.
  Retake now. Restore data once, at the very end, then verify. Don't start new
  tests after the restore.
- Final report in Markdown only (`report.md` + the thread message). No HTML
  version unless asked.

## Evidence
- Save everything under [Mac] `~/revlc-qa/mac-{DATE}/` [Windows]
  `%USERPROFILE%\revlc-qa\win-{DATE}\` (outside the repo): `screenshots/`,
  `recordings/`, `logs/`, `backup/`.
- Screenshots named `NN-what-it-shows.png`, 40+ covering every screen and state
  you touch. Name bug evidence `NN-bug-…`.
  [Mac] `screencapture -x` via a wrapper that refuses when the screen is locked.
  [Windows] PowerShell System.Drawing CopyFromScreen after SetProcessDPIAware;
  for UI-only states also capture the WebView directly over CDP (see playbook).
- Recordings: at least 6 short silent clips (playback/seek, resume after
  relaunch, video+subtitle open, drag-and-drop, key-hold, library/home,
  settings, installer/updater if testable). [Mac] `screencapture -x -v -V <s>
  -k`. [Windows] ffmpeg `-f gdigrab -i desktop`. Check each with ffprobe (right
  length, no audio stream).
- Write `evidence-index.md` listing every file. Embed the key screenshots in
  your final message with absolute-path markdown so the user sees them in this
  thread, and list the recordings with paths.

## What changed on this branch (test each one)
{CHANGE_LIST}

## Also do a general QA pass
Every screen and control: home, library, player controls, seek bar, volume,
queue, subtitles & audio tracks, speed, fullscreen, settings (all tabs),
bookmarks, history/continue watching, about/update UI, menus and keyboard
shortcuts, window resize (minimum and huge), dark/light, empty and error states
(missing file, bad URL, unsupported, corrupt, zero-byte), long filenames and
unicode, many files. Look for misaligned/clipped text, inconsistent
spacing/fonts/colours/icons, hover/focus states, overflow, flicker, wrong copy.
[Windows] Also: DPI 100%/150% (restore the original), titlebar/window chrome,
scrollbars, focus rings, ⌘ shown instead of Ctrl, long paths, paths with
spaces, file associations / "Open with".

## Test media
Use the fixture set in [Mac] `~/revlc-qa/media/` [Windows]
`%USERPROFILE%\revlc-qa\media\`; generate anything missing with ffmpeg (see
playbook "Fixture set"). [Windows] If ffmpeg isn't on PATH, look under
`%USERPROFILE%\revlc-qa\` for the essentials build from the last audit.

## Build
Read README, package.json, src-tauri and .github/workflows/{checks,release}.yml.
Run `npm run check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
and report results. Test the debug app AND a release build:
[Mac] `npm run build:app` (it loads the updater key from
`~/.tauri/revlc-updater.key`); test `target/release/bundle/macos/ReVLC.app`.
[Windows] use `npm.cmd`, not `npm` (PowerShell blocks npm.ps1). Build the
installer the way release.yml does (`bash scripts/build-app.sh` via Git Bash)
and test the installed app. Say exactly which builds you tested.

## Final report (in this thread)
1. Summary: environment ([Windows] Windows version, GPU, DPI), build/test
   results, what was tested, what couldn't be.
2. Bugs table: ID, severity (critical/high/medium/low), title, repro steps,
   expected vs actual, evidence paths, likely cause (file:line).
3. UI bugs and inconsistencies (separate list, screenshots inline).
4. Each branch change above: PASS / FAIL / NOT TESTED with one line of evidence.
5. Confirm data restored (hash check), app processes killed, [Windows] test
   build uninstalled and registry restored, repo clean.
Also save the report as `report.md` in the evidence folder.
```

Things the orchestrator should check before sending:

- The change list says what to do for anything that's hard to trigger. For
  example, "Restart now" needs a real update: tell the thread to test what it
  can and say what it couldn't.
- The facts are current: head SHA, installed version on the PC, data paths.
  Check them with `git log -1`. For the PC, ask the Windows thread to confirm.

## 4. Techniques that worked

### 4.1 macOS

- **App-wrapper for the debug binary.** The computer-use tool (`cua.getApp`)
  only accepts an app name, bundle path or bundle ID. It can't attach to the
  bare `target/debug/revlc`.
  - Copy the binary into a throwaway bundle outside the repo:
    `~/revlc-qa/<run>/ReVLC-QA-Debug.app/Contents/MacOS/revlc`.
  - Give it an `Info.plist` with `CFBundleExecutable=revlc`, a unique
    `CFBundleIdentifier` (e.g. `app.revlc.qa-debug`) and
    `CFBundlePackageType=APPL`.
  - Launch that binary directly with `REVLC_VLC_ARGS=--no-audio`, then
    `cua.getApp("<path to .app>")`.
  - The debug build loads `http://localhost:1420`, so keep `npm run dev` (Vite)
    running in the background.
  - The release `.app` can be selected directly by its bundle path.
- **Screenshots with a lock check.** `shot.sh NN-name` checks
  `CGSSessionScreenIsLocked` in `ioreg`, refuses to capture if the screen is
  locked, and logs `IOConsoleUsers`. Then it runs `screencapture -x`.
- **Recordings.** `screencapture -x -v -V <seconds> -k <file>.mov &`. Start it
  in the background, then drive the app. Afterwards, encode highlights to
  smaller MP4s in parallel (`ffmpeg -c:v libx264 -crf 25 -an -movflags
  +faststart`). Check them with `ffprobe` (duration, no audio stream).
- **CGEvent helper (`desktop.swift`).** Small subcommands:
  - `windows`/`windowid` (find the ReVLC window)
  - `key <keycode> <count>`: posts real autorepeat events (`keyboardEventAutorepeat=1`). This is what proved that held Space/F toggle once while seek/volume repeat.
  - `click`, `move`, `drag`
- **Clean first launch with seeded settings.** After the backup, empty the data
  dir and write a `settings.json`:
  `{"analytics":false,"autoUpdate":false,"audio":false,"snapshotPath":"<evidence>/screenshots","resume":"ask"}`.
  The first launch then shows onboarding, can't send analytics or update, and
  saves snapshots into the evidence folder.
- **Open files by argv:** `…/revlc <video> <subtitle.srt>`. It's faster and
  more reliable than driving the native file picker. Use the picker only when
  the picker itself is under test.
- **Native Finder drop.** Select both files in a Finder window, then
  `finder.drag(...)` onto the player. Check the result in the subtitle menu.
- **Corrupt history.** Quit, write `[{"uri": tru` to `history.json`, launch.
  Capture at 1, 2 and 4 s plus a 15 s recording, because the notice is
  transient or missing. Check that `history-damaged-<time>.json` exists.
- **Quit while buffering.** Run a local `http.server` that serves the TS HLS
  fixture and holds one segment open. Quit once the UI shows Buffering, then
  diff `history.json`.
- **Unicode subtitle font check.** Turn the title overlay off. Compare the
  default font (Helvetica Neue) with Hiragino Sans on an SRT containing
  `日本語`. This isolated a missing-glyph bug.

### 4.2 Windows

- **WebView2 over CDP.** Launch with
  `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9223 --remote-allow-origins=*"`
  (plus `REVLC_VLC_ARGS=--no-audio`). A small Node script (`qa-cdp.mjs`) then
  does `Runtime.evaluate` against `http://127.0.0.1:9223/json`. From there you
  can:
  - read `document.body.innerText`
  - click buttons by text or aria-label
  - call backend commands directly: `window.__TAURI_INTERNALS__.invoke("open",{uris:[…],play:true})`, `"snapshot"`, `"seek"`, `"toggle_pause"`, `"settings_get"`
  - send keys with `Input.dispatchKeyEvent` (`autoRepeat: true` for held keys)
  - capture just the WebView with `Page.captureScreenshot` (`fromSurface: true`)
- **Notes on CDP:**
  - Pick the target by URL or title: the main window vs `view=settings`. Never
    fall back to `targets[0]`, which may be the Settings window.
  - Return plain values from `evaluate`. Use `.then()` on promises. An object
    holding an un-awaited promise hangs until the 30 s timeout.
  - WebView captures are immune to other windows covering the app. They show
    the web UI only, so use desktop captures when the video frame or
    subtitles matter.
- **A test driver (`qa-native-tests.mjs`)** runs suites (`codecs`, `hls`,
  `library`, …) over one CDP socket and logs each check to
  `logs/native-tests.jsonl` as `{name, pass, elapsedMs, …}`. That log becomes
  the PASS/FAIL table.
- **PowerShell helpers (`qa-desktop.ps1`).** Dot-source it in each command.
  - `Shot <name>` (CopyFromScreen of the virtual screen after
    `SetProcessDPIAware`), `Record <name> <seconds>` (ffmpeg gdigrab),
    `Focus-ReVLC`, `Keys`, `Click`, `Eval-Js`.
  - UI Automation helpers for the NSIS installer/uninstaller (`Nsis-Inspect`,
    `Nsis-Click <automationId>`).
  - `PrintWindow` for capturing one specific window.
- **DPI changes.** Drive Windows Settings → Display → Scale through UI
  Automation (find the `Scale` ComboBox, expand it, select). Capture at 100%
  and 150%, then restore the original (125% on this PC).
- **Version-spoofed libVLC.** Copy `libvlc.dll` and `libvlccore.dll` into a
  temp folder and patch the copied `libvlc.dll` so its version string reads
  `4.0.24` instead of `3.0.24`. Point `REVLC_VLC_DIR` at the folder and check
  that it's refused with a readable message. A folder holding only `libvlc.dll`
  tests the missing-core error.
- **Junction loop:** `mklink /J` back to a parent inside a library folder.
  **Long Unicode path:** nested folders reaching about 346 characters, after
  confirming `LongPathsEnabled`.
- **Which DLLs actually loaded:**
  `(Get-Process revlc).Modules | ? ModuleName -in 'libvlc.dll','libvlccore.dll'`.
  This proved the installed build used its bundled 3.0.24, not the PC's VLC
  3.0.16.
- **Native Explorer drop.** Use SetCursorPos + mouse_event with DPI-aware
  coordinates, and bring the Explorer window to the foreground first.

### 4.3 Both platforms

- **200k-entry cap.** Put `000-before-cap.mp4` in a folder, then 200,005 empty
  `entry-NNNNNN.txt` files, then `zzz-after-cap.mp4`. Add the folder as a
  library. Only the "before" file should appear. On Windows the scan took about
  13 s and the UI stayed responsive. This tests the real app, with no harness.
- **Replaced files.** Replace a file in place with one of a different colour,
  duration and title: blue 48 s ↔ red 72 s, via write-to-temp then
  `os.replace`. Also try a copy whose mtime is older than the cached thumbnail.
  That second case is what exposed the stale thumbnail on Windows.
- **Settings burst.** Toggle settings fast through the UI, relaunch, then read
  `settings.json` and screenshot Settings to show the values stuck.
- **Resume isolation.** Two queued items: A with a saved position, B fresh.
  Switch A→B→A quickly by double-clicking in the queue and with `n`/`p`. Check
  that each one resumes only its own position.
- **Before and after files.** Copy `history.json`/`settings.json` into `logs/`
  before and after each data-changing test. The report can then cite exact
  values (e.g. "saved 11,250 ms").

### 4.4 Restore verification

1. Back up with metadata.
2. At the end, kill the app.
3. Move the QA data aside into `logs/` so it's kept as evidence.
4. Move the originals back.
5. Compare a manifest of relative path → SHA-256, mode and mtime for the backup
   and the restored dir.

On macOS don't compare `gid`: a copy into `/tmp` takes the `wheel` group while
the originals are `staff`. On Windows also check that the install folder has
the same file count and hashes (1,179 files last time) and no extra files.

### 4.5 Fixture set

The fixtures last time:

- `long-h264.mp4`: 180 s, testsrc2, AAC sine track, title metadata
- MKV with 2 audio tracks + embedded SRT
- VP9 WebM with a Unicode name (`日本語 🦀 café — VP9.webm`)
- MPEG-4 AVI; HEVC (Windows)
- `paired-open/` video + unrelated-name `.srt` containing English, `日本語` and `café`
- `hls-fmp4/` and `hls-mpegts/`, each with `stream.m3u8` + `master.m3u8` (`#EXT-X-STREAM-INF`)
- `corrupt.mp4`, `zero-byte.mp4`, `unsupported.xyz`
- a 240-character Unicode filename
- `library-many/` with 80 hardlinked episodes + a `loop` symlink to itself
- red/blue replacement pair
- `.m3u` playlist

The Mac generator is `~/revlc-qa/mac-2026-10-22/logs/generate-media.py`. The
media lives in `~/revlc-qa/media/` and `%USERPROFILE%\revlc-qa\media\`. Reuse
it and only add what a new change needs.

## 5. Merging the two reports

When both threads finish, read each one's final message. Pull the full
findings list from the Windows `REPORT.md`; it shows up in the thread's
activity near the end. Then write one answer:

1. **Headline.** Did each platform build and pass checks, and how many
   findings? Mention crashes, or say there were none. Say both machines were
   restored, with the hash-check result.
2. **Builds and checks.** One line per platform. Include which builds were
   tested and any build-step failures.
3. **Evidence locations.** Mac paths are clickable here. Windows screenshots
   only show up in the PC's T3 thread; say so.
4. **Bugs found on both platforms first.** Number them, and give a likely cause
   (`file:line`) for each. Confirmation on two platforms is the strongest
   signal.
5. **Mac only, then Windows only.** Sort by severity.
6. Embed 2–3 key Mac screenshots inline.
7. **How the PR's changes held up.** Give PASS/FAIL/NOT TESTED per change, with
   both platforms' evidence merged. Call out where a PR change itself fails or
   is incomplete. These are fixes to make before merging.
8. **Not covered.** E.g. a real update install, a fresh Windows 10 machine,
   discs, leak profiling.
9. **What needs the user's call.** Separate:
   - bugs in the PR's own changes (fix before merge)
   - pre-existing bugs (fix now or later?)
   - anything that touches an accepted trade-off or is a product decision

   Don't change code until the user says which.

Settle the QA threads once their results are merged and the user no longer
needs them.

## 6. Time sinks from the last run (2026-10-22)

Wall-clock times:

- Mac: 08:14–09:08 (54 min).
- Windows: 08:14–09:42 (88 min). It was the critical path.
- The orchestrator delivered the merged report at 09:43, 35 min after the Mac
  thread finished.

All times UTC. Each row's fix is now in section 0 or the prompt template.

| # | Where | What happened | Lost | Prevention |
|---|---|---|---|---|
| 1 | Win 08:16–08:22 | `winget install Gyan.FFmpeg` hung, then failed (`-2147012894`, timeout). A direct `curl` from gyan.dev took 47 s. | ~3 min before fixtures could start | Reuse the ffmpeg already downloaded under `%USERPROFILE%\revlc-qa\`. Never winget. |
| 2 | Win 08:15–08:16 | `npm` failed: PowerShell execution policy blocks `npm.ps1` | ~1 min | Prompt says `npm.cmd` |
| 3 | Win 08:16–08:27 | Fresh worktree, so cold Cargo builds: clippy 1m17s, then test/debug builds; `build-app.sh` 6m41s. The debug app first ran at 08:21 (Mac: 08:16). | ~5 min to first UI test; installer at 08:27 | Shared `CARGO_TARGET_DIR`; start the release build first |
| 4 | Win 08:29–08:31 | fMP4 fixture: ffmpeg wrote `init.mp4` into the repo worktree (cwd), so the HLS test failed falsely and the worktree was dirty | ~2 min | Run ffmpeg with cwd = fixture dir; reuse the existing fixtures |
| 5 | Win 08:23, 08:24, 08:46 | CDP problems: fallback to `targets[0]` hit the Settings window; `evaluate` returning an object with an un-awaited promise hung until the 30 s timeout | ~2 min | The notes in 4.2; reuse the fixed `qa-cdp.mjs` |
| 6 | Win 08:26–08:28 | Chased a "snapshot not created" issue that was a focus problem | ~2 min | Time box; reuse helpers that focus the window first |
| 7 | Win 08:34–08:37 | DPI change by driving the Settings UI through UI Automation, written from scratch | ~3 min | Reuse the saved scale script |
| 8 | Win 08:44–08:47 | Explorer drag-and-drop: several attempts (stale DPI coordinates, wrong foreground window) | ~2.5 min | Reuse the working drag sequence in `qa-desktop.ps1` |
| 9 | Win 09:12–09:21 | Uninstall through the NSIS UI + registry restore. One `reg import` step timed out and was rerun. | ~8 min (needed, but slow) | Keep the restore script (`prepare-registry-restore.cjs` + `.reg` files) and rerun it; consider `uninstall.exe /S` |
| 10 | Win 09:25–09:31 | After the full restore, found desktop captures with Explorer covering ReVLC. Moved the originals aside again, put the QA data back, retook with WebView capture, restored and hash-checked again. | ~6 min | Review captures before cleanup; use WebView capture for UI-only states |
| 11 | Win 09:30–09:42 | Final report: a findings list built in Node, REPORT.md + REPORT.html, `html_preview`/`html_render`, a context compaction at 09:42 | ~12 min | `logs/findings.md` kept as you go; Markdown only |
| 12 | Win 09:14 | Provider stream disconnected ("websocket closed by server") | 23 s, recovered | None needed |
| 13 | Mac 08:16 | `cua.getApp("revlc")` launched the installed `/Applications/ReVLC.app` without `--no-audio` (and against the real data dir). It was killed straight away. | <1 min, **safety** | Never get the app by name; launch the binary path |
| 14 | Mac 08:16–08:17 | The computer-use tool couldn't select the bare `target/debug/revlc` and needed the `.app` wrapper (plus a separate `npm run dev`) | ~1 min | Reuse `ReVLC-QA-Debug.app`; refresh its binary |
| 15 | Mac 08:17–08:19 | `npm run tauri build` ran 2 min, then exited non-zero: `TAURI_SIGNING_PRIVATE_KEY` isn't set. The key exists at `~/.tauri/revlc-updater.key`, but only `scripts/build-app.sh` loads it. | Noise in the report, a false "build failed" | Prompt says `npm run build:app` |
| 16 | Mac 08:15 | The media generator failed once on a Python syntax error | <1 min | Reuse `generate-media.py` |
| 17 | Mac 08:20–08:27, 08:39 | Native file picker automation: clipboard paste timeout, multi-select ignored Cmd+A (needed Shift+Down), "Go to field unavailable" | ~3 min | Open by argv |
| 18 | Mac 08:22–08:41 | Small computer-use errors: `keyNotFound("=")` (use `"equal"`), stale element IDs after relaunch and fullscreen | ~1 min | After a relaunch or fullscreen change, call `getApp`/`getAXState` again before clicking IDs |
| 19 | Mac 08:41–08:43 | The first Finder drag didn't land; recording 16 was wasted | ~1.5 min | Use `finder.drag` after a fresh `getAXState`; check before recording |
| 20 | Mac 08:50–08:52 | The scan-cap harness (compiling `library.rs` outside the crate) failed twice: missing `crate::meta`, then `serde_json` | ~2 min | Test the cap through the app (see 4.3) |
| 21 | Mac 08:50–08:54 | The slow-HLS server held the segment for 45 s, the stream recovered before quit, and the test was rerun with 120 s | ~3.5 min | Hold until a flag file is removed |
| 22 | Mac 08:56–08:58 | The restore check failed on `gid` (backup in `/tmp` had group `wheel`) and needed a manual comparison | ~2 min | Back up under the run folder; compare hash, mode, mtime only |
| 23 | Mac 08:59–09:01 | A new test (Japanese glyphs) after the restore needed a second wipe and restore | ~2 min | No new tests after the restore |
| 24 | Mac 08:56–09:08 | Evidence index, MP4 highlights, report | ~12 min | `findings.md` as you go; encode highlights while testing |
| 25 | Mac | Not a delay: only `Application Support/app.revlc` was backed up. QA thumbnails (14:02 and 14:19 local) are still in `~/Library/Caches/app.revlc/thumbs`. | n/a | Back up and restore the Caches dir too (now in the rules) |
| 26 | Orchestrator | `t3-windows` `t3_thread_wait` errored after about 60 s for any `timeoutMs` (3 tries, 08:15–08:36) | none (it switched to reads) | Section 1: poll Windows with `t3_thread_read` |
| 27 | Orchestrator | Windows screenshots can't be shown in the Mac thread, so the merged report only linked them | n/a | Say so in the merge. A transfer step is an open question. |

Together, the preventable items come to about 25 min on Windows and about
15 min on the Mac. A rerun with these fixes should take around 60 min.
