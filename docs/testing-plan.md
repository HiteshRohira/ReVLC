# Automated testing plan

Status: planned. A spike on `ci/cua-ui-spike` (October 26) proved that Cua can
drive the real app on GitHub's macOS, Windows and Linux runners; nothing else
is built yet.

Today every branch gets a hands-on audit (see [audit-playbook.md](audit-playbook.md)):
one QA agent on the Mac and one on the Windows PC drive the real app with
computer use for 1–1.5 hours each, taking 80–180 screenshots. It finds real
bugs, but it costs a lot of tokens, ties up both machines, and is fragile:

- A locked screen blanks the web view.
- WebKit ignores synthetic clicks with a click count of 0.
- Clicks can land in other apps when ReVLC isn't frontmost.
- The native file picker is hard to drive.
- A Vite server from another checkout can serve the wrong UI.
- The Windows relay times out.

The goal is to make automated tests the default merge gate, and keep the real
Mac and PC for what only real hardware can prove. Not everything has to run in
CI: checks that need a real machine are fine, as long as they run before a
public release. The target is a 10–18 minute warm CI run; that figure is an
estimate, not a measurement.

This plan was written from the October 2026 audits (`audit-fixes-oct` on both
platforms, its macOS re-test, `parity-quick-wins` on both platforms, the PR #7
audit and the 0.2.9 and 0.2.10 release checks), the Windows engine failure a
user hit on 0.2.5, the QA helpers in `scripts/qa/`, the CI workflows and the
Cua spike.

Every check gets two answers: what kind of test it is (section 1) and where it
has to run (section 4). The same Cua script can run on a GitHub runner, in a
clean VM or on the real Mac and PC; what changes is what each place can prove.
Of the 33 bugs the audits found, about 30 would have been caught on GitHub's
runners alone. Read that with care: every audit ran on our own machines, so
bugs that only show up on someone else's computer are under-represented, and
the Windows engine failure is exactly that kind.

## 1. Where each kind of check belongs

CI already runs `svelte-check`, Clippy and `cargo test` on macOS and Windows,
but with an empty libVLC directory, so nothing decodes or plays. The biggest
gains are real media tests and browser interaction tests on top of that.

| What the audits check | Best automated home | What it can't prove |
|---|---|---|
| Runtime loading: bundled libVLC chosen, missing core or plugins, readable errors, refusing non-3.x | Rust integration tests in separate processes, with the prepared runtime and deliberately broken copies | The installed resource layout (package tests cover that) |
| Decoding and media inspection: H.264, HEVC, VP9, audio formats, TS and fMP4 HLS, tags, tracks, chapters, broken files | The real `Vlc` wrapper with memory and file outputs | Hardware decoding, the native video view |
| Video effects: colour, invert, sepia, rotation, crop, filters, staying on across items | Live memory-output tests: check pixels and frame size, and again after the next item opens | The native output can differ |
| Audio effects: pitch, mono and channel routing, equaliser, gain, combinations | The real `Vlc` writing a WAV: check frequency, level, channels | Output devices, Bluetooth, passthrough |
| Playback flow: resume, stale events, end and error handling, A–B loop, sleep timer, quitting while buffering | Rust tests around the real event handling, plus some real-engine tests | A few start-up and quit cases need the real app |
| Saved data: settings order, failed saves, history recovery, bookmarks, progress saved on quit | Rust tests with temporary folders and storage that can be made to fail; UI tests for save order | A real updater restart |
| Library and cache: symlink loops, scan cap, scan reuse, replaced files, stale thumbnails, artwork | File-system integration tests plus real thumbnail generation | — |
| UI rendering: home, queue, player, onboarding, settings, dialogs, effects, media info, empty, error and loading states, overflow | Playwright against the Svelte app with Tauri mocked, plus a few screenshot comparisons | Rust and native video |
| UI interaction: seeking, volume, menus, held keys, focus trapping, accessible names, wheel modes, shortcut conflicts | Playwright; Vitest for pure helpers | OS key delivery and focus |
| IPC and start-up: command arguments and results, events, early notices and resume offers, the two windows staying in sync | Contract tests plus a small real-app suite | — |
| Native window behaviour: fullscreen focus, green button, resize, minimise, always on top, cursor wake, controls over video | Cua against the built app on GitHub's runners | Video under the controls on macOS (no GPU on hosted Macs) |
| OS integration: menu bar, file picker, Finder/Explorer drops, file associations, cold and warm opens, second instance, command line | Unit tests for argument, path and menu logic; real-app process tests; a few native checks | DOM clicks don't prove OS hand-offs |
| Packaging and updates: bundle contents, NSIS install and uninstall, registry clean-up, update state, signatures, restart | Artifact checks, silent installer tests, a local update feed; release smoke test | Installer appearance, replacing a real installed version |
| Platform presentation: Windows DPI, mixed monitors, macOS fonts, subtitle glyphs, title overlap | Browser layout tests for CSS; render tests and native screenshots for libVLC text | Browser device scale isn't Windows display scaling |
| Hardware and outside sources: discs, cameras, microphones, screen capture, audio devices, network shares | Unit tests for URI building; recorded fixtures where possible; targeted manual checks | Hosted runners have none of the hardware |

For time-based behaviour (library scan reuse, the sleep timer), inject a clock
instead of waiting. For the scan cap, test the boundary with controlled
enumeration, and keep one real 200,000-entry run nightly or when the scanner
changes.

## 2. Tools

### Rust tests for backend behaviour

Keep `cargo test` and Clippy on both platforms. Add tests next to the existing
modules rather than moving the backend into a new framework. A few seams make
the important cases deterministic: a clock, an event sink, and storage and
network operations that tests can control. With those:

- an old item's end-of-media event can't finish the new item;
- a file that ends before it ever played stays unwatched;
- a failed settings write leaves the applied settings unchanged;
- a slow history write doesn't hold up event handling;
- early notices and resume offers survive until the UI picks them up.

Tauri's mock runtime helps test command serialisation and dispatch, but it
doesn't create a real web view or video surface.

### Real libVLC tests (already proven)

On `parity-quick-wins`, libVLC was tested without a screen or sound from Rust
tests driving the real `Vlc` wrapper:

- **Video:** frames went into memory through libVLC's video callbacks, filters
  were switched on the live output, and pixels were checked (red turned cyan
  with invert; sepia tinted it).
- **Audio:** written to a WAV with `--aout=afile --audiofile-file=…`, then
  checked: pitch +12 doubled 440 Hz to 880 Hz; mono gave equal channels.

To make that a suite:

- Use the runtime that ships: `scripts/prepare-libvlc.sh` already downloads
  VLC 3.0.24, checks it and prepares the plugins. Cache it per OS, version and
  script hash.
- Set `REVLC_VLC_DIR` explicitly and log the runtime path and version; never
  fall back to an installed VLC silently.
- Force software decoding. Use video callbacks for frames and `afile` for
  audio; use dummy outputs only where the test is about state.
- Assert numerically and with tolerance: average colour of a region, frame
  size after rotation, dominant frequency, channel equality, level change.
  Never compare exact hashes of decoded frames or audio.
- Put the callbacks on the player under test. `thumbnail()` uses a separate
  player with subtitles off, so it only proves thumbnail decoding.
- Run scenarios one at a time or one per process, with an outer timeout. The
  wrapper has process-wide state and sets environment variables when it loads.

Memory callbacks take a different presentation path from the native view
([libVLC docs](https://videolan.videolan.me/vlc-3.0/group__libvlc__media__player.html)),
so these tests can't certify the macOS native video path.

### Playwright for most UI checks

Run the real Svelte app in headless Chromium and WebKit, with Tauri mocked
before the app starts. Mock commands, events, window operations, dialog
results, asset URLs and window identity
([Tauri mocking](https://v2.tauri.app/develop/tests/mocking/)). Keep the mock
small: fixed scenarios, recorded command arguments, explicit events, and a
failure for any unknown command.

Start with 20–30 flows:

- open every menu, settings tab, effects tab and media information tab;
- seek with mouse and keyboard; change volume and speed;
- check which shortcuts repeat when held and which run once;
- Tab and Shift+Tab through dialogs; Escape; focus restored afterwards;
- pending, successful and failed settings saves and update checks;
- engine failure, corrupt history, no tracks, long names, empty library;
- wheel modes, and scrolling inside panels;
- fail on uncaught exceptions, unhandled rejections and Svelte
  "update depth exceeded" errors.

Use Vitest for pure functions (time parsing, shortcut handling, formatting).
For visual checks, start with 12–20 states at normal and minimum window size.
Pin browser versions and fonts, freeze clocks and statistics, mask only what
really changes, and review baseline changes by hand
([Playwright snapshots](https://playwright.dev/docs/test-snapshots)).
Playwright's WebKit is the same engine as WKWebView, not the app's window.

### Windows: reuse CDP

Wrap the useful scenarios from `scripts/qa/windows/cdp-lib.mjs` and
`native-tests.mjs` in Playwright Test, attached with
`chromium.connectOverCDP()`
([WebView2 testing](https://playwright.dev/docs/webview2)). Pick the main or
settings page explicitly and give each launch its own WebView2 profile.
`tauri-driver` with Edge WebDriver also works on Windows; adopt it only if CDP
falls short. Neither drives the native picker or the NSIS installer.

The existing helpers need hardening before they can gate merges:

- `wait()` can return even though its condition never became true;
- several suites log observations without asserting them;
- logging `pass: false` doesn't fail the process;
- `network-buffer.mjs` catches errors without failing;
- some steps import Vite source modules, which don't exist in a packaged build.

### Cua for the real app on all three platforms

Proven on GitHub's runners by the `ci/cua-ui-spike` branch
(`scripts/qa/ci/cua_smoke.py` and its workflow): the built app opens a generated test
video, and Cua finds the window, reads its accessibility tree, clicks Pause,
checks the button flips to Play and the picture stops, and resumes with Space.
Each job took 4–11 minutes with warm caches.

| | macOS 14 / 26 | Windows | Linux |
|---|---|---|---|
| Cua drives the app window | yes | yes | yes |
| Reads every control in the web view | yes | yes, with the flag below | yes |
| Video renders | **no**: no GPU, libVLC can't create its OpenGL view (15 too) | yes, in software | yes, in software |
| Pause by button, resume by Space | yes | yes | yes |

What each runner needed:

- **macOS:** `scripts/qa/ci/cua-macos-setup.sh` grants Cua Accessibility and
  Screen Recording by writing the system permissions database, which works
  because SIP is off on hosted runners. It copies Cua's own hosted-runner CI.
- **Windows:** WebView2 only exposes the page to UI Automation when it starts
  with `--force-renderer-accessibility`. The environment variable is ignored
  because Tauri passes its own browser arguments, so the CI build adds the flag
  through Tauri's `additionalBrowserArgs`.
- **Linux:** the Xvfb desktop from the Linux smoke test plus the accessibility
  bus (`at-spi2-core`). Cua has no background key route there, so key presses
  bring the window forward, which is fine on a CI desktop.

Cua replaces the macOS embedded-WebDriver trial: it already drives the real
WKWebView on a hosted Mac. Playwright stays the tool for UI logic; Cua is for
the built app, native windows, menus and OS input.

### Hosted runners

They suit everything except hardware. Section 4 lists what they hide.

## 3. CI design

### On every application PR

| Job | Contents | Warm runtime (estimate) |
|---|---|---:|
| `ui` (Linux) | Svelte check and build, Vitest, Playwright Chromium and WebKit, selected visual checks | 3–6 min |
| `native-checks-macos` | Existing Rust checks plus backend and real libVLC tests | 7–12 min |
| `native-checks-windows` | Same | 9–18 min |
| `ui-smoke` (all three) | The Cua smoke test on the built app | 4–11 min (measured) |
| `qa-gate` | Requires the jobs above | — |

Cold caches could push the slowest job to 20–30 minutes; measure before
growing the suite.

- Make `qa-gate` required in branch protection, and have it handle jobs that
  were deliberately skipped, so a skipped workflow can't leave it pending.
- Cancel superseded runs when new commits arrive; if a merge queue is used,
  run on `merge_group` too.
- Real-app tests build the app with the frontend embedded, launch the exact
  binary and check its build identifier. Browser tests start their own server
  and refuse to reuse one already running.
- On hosted macOS the smoke test skips the on-screen video check: there is no
  GPU. Video correctness on macOS comes from the libVLC memory-output tests and
  the real-Mac checks in section 4.

### Nightly

- The full Cua suite on all three platforms: menus, settings, effects,
  focus, fullscreen and keyboard, file opens from Finder and Explorer, the
  command line, second instance, resume across relaunch.
- More codecs, containers and multi-track fixtures.
- Repeated cold starts and resume and reload cycles.
- HLS stalls, disconnects and recovery against a local server.
- The real scan cap and long paths.
- Endurance: open, stop, filters and thumbnails in a loop, watching resources.
- Unsigned bundle layout and a silent Windows installer smoke test.

Budget 20–40 minutes per platform, non-blocking at first while flakiness is
understood. Skip commits that already passed an equivalent run.

### Before a release

Extend `release.yml` to check the candidate before its files count as
publishable:

- architecture, version, libVLC runtime and plugins, licences, update manifest;
- launch from the bundled or installed location with `REVLC_VLC_DIR` unset;
- install and uninstall in a throwaway Windows environment, checking
  registrations and clean-up;
- update success, failure, bad signature, interrupted download, restart with
  history kept.

Then, before publishing, the clean-VM and real-machine checks in section 4.

A local update feed with a throwaway key tests the updater's mechanics without
production secrets. It doesn't prove that the production-signed build replaces
the previous release, so keep that as a release check. PR jobs never get the
production signing key.

### Caching, fixtures and artifacts

- Cache Cargo (by OS, architecture, toolchain, lockfile), the prepared libVLC
  (by OS, version, script hash; keep plugin timestamps) and pinned tool
  downloads. Build each binary once per job.
- Give every test its own app data, cache, media output and web view profile.
- Generate most media with ffmpeg: short colour patterns, asymmetric patterns
  for rotation, distinct per-channel tones, subtitles, chapters, multiple
  tracks, both HLS variants. Check fixtures with ffprobe (and that HLS
  segments exist) before testing ReVLC, so a broken fixture fails as a
  fixture. Commit small text fixtures; keep binary samples only when they
  can't be generated (teletext, for example), with their source and licence.
- Upload test results, event logs, stdout and stderr, console errors, build
  and runtime identities, and screenshots, diffs, PNGs or WAVs only for
  failures. Keep passing artifacts briefly. Never commit proof screenshots or
  recordings to the repository.
- Against flakes: wait for readiness instead of sleeping, clean up explicitly,
  bound every process, fix clocks and seeds, use local servers, and report
  infrastructure failures separately from product failures. Allow at most one
  diagnostic retry for app automation, showing the first failure.

The same suites should run locally in the background (UI tests while editing
the UI, Rust tests after backend changes, libVLC tests when playback code
changes) without opening a window or contacting PostHog.

Cost: public repositories use standard hosted runners at no charge. At paid
rates a PR run (about 6 Linux, 14 Windows and 12 macOS minutes) is roughly
$0.90, and a 30-minute nightly per native platform roughly $2.

## 4. Where each check must run

### GitHub's runners: every PR and nightly

Sections 1–3. They differ from a user's machine in ways that hide real bugs:

- **No GPU.** macOS shows no native video at all; Windows and Linux render in
  software.
- **No audio devices**, Bluetooth, discs or cameras.
- **One virtual display at 100% scale**, US English, the latest OS only.
- **Developer tools and Visual C++ runtimes are already installed**, so a
  missing-DLL bug passes. That is one of the likeliest causes of the 0.2.5
  engine failure on a user's Windows 10 PC.
- **macOS has SIP off and Cua's permissions pre-seeded**, so first-launch
  permission prompts never appear.
- **The app is never downloaded through a browser**, so no Gatekeeper
  quarantine and no SmartScreen.

### Clean VMs on our own hardware: before each release

Install the published files on a fresh snapshot with nothing else installed,
and run the same Cua smoke script:

- Windows 10 and Windows 11 on the PC (VirtualBox snapshots, or Windows
  Sandbox for Windows 11: Windows 11 Pro includes it and every launch starts
  clean);
- a macOS VM on the Mac (UTM or Tart), ideally an older macOS too.

The real PC can't stand in for the clean Windows 11 install: it has
developer tools, runtimes, VLC and earlier ReVLC versions on it, which is the
opposite of a new user's machine. A VM also resets in seconds and runs without
taking over the desktop. This layer catches "doesn't start on a stranger's
PC". It is fully scriptable; it just doesn't run on GitHub.

### The real Mac and PC: before a public release

Scripted with Cua where possible, run when a PR changes the area and always
before a public release:

| Check | Why CI or a VM can't prove it |
|---|---|
| macOS native video: picture, subtitles over video, resize, fullscreen; title position | Every hosted macOS image fails to create the OpenGL context |
| Hardware decoding, real GPU drivers (AMD, Intel, NVIDIA), 10-bit and HDR | No real GPU |
| Windows at 125% and 150%, mixed-DPI monitors; Retina plus an external display | One 100% display |
| Audio devices, AirPods and Bluetooth switching, passthrough | No audio hardware |
| The downloaded installer: Gatekeeper quarantine on the DMG, SmartScreen, antivirus touching libVLC plugins | Nothing is downloaded through a browser |
| A real update from the previous release, history kept | Needs the previous version installed |
| Sleep and wake, a locked screen, trackpad gestures | Not available in VMs |
| Discs, cameras, microphones, network shares (only when affected) | No hardware |
| Installer appearance and overall look | Human judgement: Cua takes the screenshots, a person looks |

- Keep the unlocked-screen requirement. Before each native action, check the
  intended process, window and frontmost app, and stop if focus changed.
- Native checks run as scripts, not as an agent deciding every click.

### After a release

The existing sanity and smoke runs on the published files.

## 5. Test hooks the app needs

A few seams, not a general remote control:

| Hook | Purpose | Risk and containment |
|---|---|---|
| Separate data, cache and web view folders | No more backup and restore; runs can't interfere | Set before anything initialises; cover every store and web view profile |
| Output mode in the test harness | Attach memory callbacks or file audio | Keep the production native output intact and tested separately |
| Diagnostic state that doesn't consume anything | Read generation, current item, pending work, errors, readiness, runtime path | `snapshot` hands over one-time early events; test polling must not take them from the UI |
| Clock, storage, event and HTTP seams | Force cache expiry, failed writes, late listeners, buffering, analytics races | Same production logic, different dependencies |
| Stable accessible names and test IDs | Reliable controls and window selection | Prefer roles and names; IDs only where ambiguous |
| Optional automation plugin | Drive WKWebView and WebView2 without computer use | Behind its own Cargo feature; no plugin permissions or servers in shipped builds |

- Build headless test harnesses directly in Rust; there's no need for a public
  `--headless` mode.
- Any control endpoint is absent from normal builds. If a test server is
  needed, bind it locally, authenticate per run, limit file access to the
  fixture folder and close it on teardown. A release check confirms test
  features are off.
- Analytics: `scripts/qa/windows/analytics-race.mjs` sets
  `REVLC_ANALYTICS_SEND`. Don't reuse it as is; test queueing, disabling and
  retries through an injected transport that can't reach PostHog. Every
  launch still seeds `"analytics": false` first and never sets
  `REVLC_ANALYTICS_SEND`.

## 6. Phases

| Phase | Work | Effort (estimate) |
|---|---|---:|
| A | Harden assertions and exit codes, validate fixtures, build identity, required gate | ½–1 day |
| B | Tauri mock bootstrap, core Playwright flows, console-error failures, accessibility and selected visual checks | 1½–2½ days |
| C | Turn the memory-output and WAV experiments into per-process libVLC tests; codecs, subtitles, replaced files | 2–3 days |
| D | Backend seams and deterministic start-up, event and storage tests | 2–3 days |
| E | Turn the Cua spike into the PR smoke job, then grow it into the nightly suite on all three platforms | ½ day, then 1–2 days |
| F | Broader nightly, release and package tests; clean-VM and real-machine scripts | 2–3 days, gradually |

About two weeks for the core, in small steps. The UI tests and the
already-proven libVLC approach should pay off within the first few days.

### Past audit bugs and what would have caught them

These are bugs the audits found (most are fixed); the point is which layer
would have caught each one, and whether GitHub's runners are enough. "Real
Mac" or "clean VM" means section 4.

| Bug | Test that would catch it | Phase |
|---|---|---|
| "Ask" resume offer lost on a cold launch from Finder | Late-listener test in Rust, UI hydration test, cold-start app smoke test | B, D, E |
| Changing subtitle style dropped external subtitles (both platforms) | Real-engine reload test: track stays selected, Off stays Off, old SRT doesn't follow another file | C, D |
| Recovered history showed no notice (both platforms) | Corrupt history in a temp folder: file kept aside, notice pending, notice shown | B, D |
| Fullscreen loses keyboard focus (macOS) | Cua: F, then real key input, on a hosted Mac; confirm on the real Mac before release | E, real Mac |
| Double-click on video didn't enter fullscreen | Browser handler test plus a Cua double-click | B, E |
| Unplayable file marked Watched (both platforms) | Feed end-before-play events; replay malformed media through the real engine | C, D |
| Update result shown in the other window | Delayed update scenario; check progress and result in Settings | B, E |
| Shuffle button had no accessible name | Accessible role, name and state checks | B |
| Periodic history writes on the event thread | Block the writer; event handling must continue | D |
| Start-up title covered subtitles | Default-policy test and a known-frame render; native confirmation | C, D, real Mac |
| Default font showed Japanese subtitles as boxes (macOS) | Memory-output render on a hosted Mac, which has the real macOS fonts: libVLC draws subtitles into the frame | C |
| Valid file failed at a 346-character Unicode path (Windows) | Real wrapper opens the same file at short and long paths | C |
| Replacing a file with an older timestamp kept the old thumbnail (Windows) | Replace, move the timestamp back, rescan, check new metadata and pixels | C, D |
| Focus escaped a modal dialog (Windows) | Tab and Shift+Tab containment, background inert | B |
| Engine failure left Settings loading forever (Windows) | Reject `settings_get`; a visible error state | B |
| "Return" shown instead of "Enter" on Windows | Platform-specific shortcut text | B |
| NSIS footer clipped; generic uninstaller artwork | Cua screenshots of the installer on hosted Windows; a person looks | E, F |
| Gear menu froze with Svelte's update-depth error (Windows) | Open the menu with real tracks and effects; fail on console errors | B |
| Marquee `$t`/`$T` codes shown literally (both platforms) | Render the advertised codes through real libVLC | C |
| Relative-path Rust test assumed Unix paths | Windows `cargo test` already caught it | A |
| Windows menus disappeared during playback | Playwright: title bar and Alt handling mounted on the player; Cua on hosted Windows | B, E |
| Onboarding disclosure clipped at minimum height | Playwright at 720×450; 150% scaling on the real PC | B, real PC |
| Media information focus escaped (Windows) | Tab and Shift+Tab containment | B |
| Scan-cap boundary depended on file-system order | Controlled enumeration in a Rust test | D |
| Previews reused after replacing a file with one of the same length (PR #7) | Replace the file, relaunch, check new preview pixels | C, D |
| Preview generation continued after disabling and stopping (PR #7) | Count generated frames after disable and stop | C, D |
| Previews ignored the selected video track (PR #7) | Two-track fixture: preview pixels match the selected track | C |
| Engine failed on a user's Windows 10 PC on first launch (0.2.5); cause unknown | Published installer on a clean Windows 10 VM; better startup diagnostics | F, clean VM |
| "libVLC not found" shown for every engine failure | Each failure class produces its own message | B, D |

## Top five recommendations

1. Gate merges on real libVLC output tests on both platforms.
2. Move routine UI interaction and visual checks into Playwright with Tauri mocked.
3. Test event races and saved data deterministically in Rust.
4. Drive the built app with Cua on GitHub's macOS, Windows and Linux runners: a smoke test on every PR, the full suite nightly.
5. Before each release, run the same scripts on clean VMs and the real Mac and PC, limited to what hosted runners can't prove.
