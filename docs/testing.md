# Testing strategy

Test each thing at the cheapest layer that can prove it. Only go up a layer
when the layer below can't see the problem.

## Layers

| Layer | Proves | Where it runs |
|---|---|---|
| Checks: `npm run check`, `cargo clippy`, `cargo test` | Types, lints, Rust logic (parsing, settings, history, paths) | CI on macOS, Windows, Linux, every PR |
| Browser UI tests (`tests/ui`, Playwright) | The Svelte UI's behaviour with the backend mocked (`ui/tests/mock.ts`): screens, shortcuts, focus, menus, error states | CI on Linux, in WebKit (as macOS) and Chromium (as Windows) |
| libVLC tests (`src-tauri/src/vlc_it.rs`) | ReVLC's own engine code on a real libVLC 3, checked by the frames and audio it produces | CI on macOS, Windows, Linux |
| Cua suite (`scripts/qa/ci/cua_suite.py`) | The built app through its real window: launch, OS file opening, relaunch and resume, real playback, pixels, app data | CI on Linux; on demand on the Mac, the PC and clean installs |
| Hands-on audit (`docs/audit-playbook.md`) | Whatever the layers above can't reach | Before releases |

## What goes where

- **Browser UI test** if the backend can be mocked: almost anything inside the
  UI, including error states. It's fast and flake-free, so it's the default.
- **libVLC test** if it's about what ReVLC asks libVLC to do (filters, tracks,
  seeking, thumbnails): check the output frames, not the UI.
- **Cua** only for what needs the real app and OS: the native window and menus,
  launching and relaunching, files from Finder/Explorer or the command line,
  installers, real playback, and app data surviving restarts. Keep each Cua
  check end to end; don't repeat what a browser test already covers.
- **By hand** only for what no tool can judge well: installer and DMG
  appearance, real updates, hardware decoding, display scaling, audio devices,
  and Gatekeeper/SmartScreen prompts.

## Where Cua runs

- **CI (Linux, Xvfb):** every PR. Hosted Macs can't create libVLC's video view,
  and WebView2 hides the page from UI Automation without a screen reader, so
  macOS and Windows run on devices instead.
- **Devices (`scripts/qa/device/`):** Hitesh's Mac and PC, with real video and
  GPU, only when he asks. No self-hosted runners on his machines.
- **Clean installs:** a Tart VM on the Mac (`run-mac-vm.sh`). On the PC, a
  fresh Windows Sandbox per run (`scripts/qa/sandbox/run-sandbox.ps1`), with
  networking off and nothing left running afterwards. Linux clean installs are
  CI only.
- **No VirtualBox:** with Windows' Memory integrity on, it falls back to a slow
  mode where installs hung for hours and the suite ran 3× slower.

## Rules for every run

- Analytics off: `scripts/qa/appdata.py` backs up app data, seeds settings
  with analytics off before the first launch, then restores and verifies it.
  Never set `REVLC_ANALYTICS_SEND`.
- No sound (`--no-audio`).
- Known app bugs are expected failures. The suite flags them if they start
  passing.
