# Testing strategy

Test each thing at the cheapest layer that can prove it. Only go up a layer
when the layer below can't see the problem.

## Layers

| Layer | Proves | Where it runs |
|---|---|---|
| Checks: `npm run check`, `cargo clippy`, `cargo nextest` | Types, lints, Rust logic (parsing, settings, history, paths), with property tests (`prop_tests.rs`) and data saved by every release (`data_compat_tests.rs`) | CI on macOS, Windows, Linux, every PR |
| Browser UI tests (`tests/ui`, Playwright) | The Svelte UI with the backend mocked (`ui/tests/mock.ts`): screens, shortcuts, focus, menus, error states, axe accessibility, screenshots | CI on Linux (WebKit as macOS, Chromium as Windows), a real Mac and Windows; screenshots in a pinned Linux container |
| Engine tests (`vlc_it.rs`, `keyframes_it.rs`) | ReVLC's engine code on the libVLC and FFmpeg it ships, checked by the frames and audio they produce, across common codecs and sizes | CI on macOS, Windows, Linux |
| Cua suite (`scripts/qa/ci/cua_suite.py`) | The built app through its real window: launch, OS file opening, relaunch, real playback, pixels, app data, stalled streams | CI on Linux; on demand on the Mac, the PC and clean installs |
| Hands-on audit (`docs/audit-playbook.md`) | Whatever the layers above can't reach | Before releases |

Releases wait for the checks, the browser tests, the engine tests and the
Linux Cua run (`release.yml`). Coverage (every PR) and mutation testing
(weekly) are reports only.

## What goes where

- **Browser UI test** if the backend can be mocked: almost anything inside the
  UI, including error states. It's fast and flake-free, so it's the default.
  The mock knows the app's exact command list; a test checks it against
  `generate_handler!`.
- **Engine test** if it's about what ReVLC asks libVLC or FFmpeg to do
  (filters, tracks, seeking, previews, decoder fallback): check the output
  frames, not the UI.
- **Cua** only for what needs the real app and OS: the native window and menus,
  launching and relaunching, files from Finder/Explorer or the command line,
  installers, real playback, and app data surviving restarts. Keep each Cua
  check end to end; don't repeat what a browser test already covers.
- **By hand** only for what no tool can judge well: installer and DMG
  appearance, real updates, hardware decoding, display scaling, audio devices,
  and Gatekeeper/SmartScreen prompts.

Every bug an audit finds gets an automated check at the lowest layer that
can see it, before or with its fix.

## Where Cua runs

- **CI (Linux, Xvfb):** every PR. Hosted Macs can't create libVLC's video view,
  and WebView2 hides the page from UI Automation without a screen reader, so
  macOS and Windows run on devices instead. No Cua on hosted Mac or Windows
  machines.
- **Devices (`scripts/qa/device/`):** Hitesh's Mac and PC, with real video and
  GPU, only when he asks (`--quick` for a short set). No self-hosted runners
  on his machines.
- **Clean installs:** a Tart VM on the Mac (`run-mac-vm.sh`; `make-mac-vm.sh`
  rebuilds its image). On the PC, a fresh Windows Sandbox per run
  (`scripts/qa/sandbox/run-sandbox.ps1`), with networking off and nothing left
  running afterwards: `-NoGpu` for a PC without GPU decoding, `-Previous` to
  update from the last release and check settings and history survive. Linux
  clean installs are CI only.
- **No VirtualBox:** with Windows' Memory integrity on, it falls back to a slow
  mode where installs hung for hours and the suite ran 3× slower.

## Rules for every run

- Analytics off: `scripts/qa/appdata.py` backs up app data, seeds settings
  with analytics off before the first launch, then restores and verifies it.
  CI builds have no analytics key. Never set `REVLC_ANALYTICS_SEND`.
- No sound (`--no-audio`).
- One Cua Driver version everywhere (`scripts/qa/cua-version.txt`); each
  report records the commit, installer checksum and app, libVLC, FFmpeg, Cua,
  WebView2 and OS versions.
- Wait for a condition, never a fixed time, unless the wait is the measurement.
- After every scenario, the app logs are checked for panics, crashes,
  WebView2 errors and failed preview passes.
- Known app bugs are expected failures (`known_bug` in Rust and Cua,
  `test.fail` in Playwright). A known bug that starts passing fails the run,
  so its mark gets removed. A Cua or script error is reported as an error,
  never as the app's failure or as the known bug.
