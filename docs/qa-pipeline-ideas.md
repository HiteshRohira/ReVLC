# QA pipeline ideas

Ideas from the 2026-11-23 audit of PR #21 (the Rust media pipeline), run on
the Mac and the PC. None of these are done yet. They're here to explore later.
Each one names the layer from `docs/testing.md` it belongs to.

IDs refer to the audit's findings:

- **M1:** slow cold TS and M2TS previews without FFmpeg.
- **M2:** HDR10 preview colours differ from playback.
- **M3, W4:** snapshot format setting ignored.
- **M4:** deinterlace toggle out of sync with Settings.
- **M5:** subtitles under the seek bar.
- **M6:** a matching SRT listed twice.
- **M7:** library duration missing for fragmented MP4 and MKV without a duration.
- **M8:** AAC in an M2TS private stream missing.
- **W1:** truncated MP4 previews past the data.
- **W2:** padded preview size on Windows.
- **W3:** paused video-track switch shows the audio screen.
- **W5:** failed seek marks a video Watched.
- **W6:** a title tag equal to the file name loses its number.
- **W7:** `.mks` listed in Videos.
- **W8:** no About on the Windows Help menu.

## Rust unit tests

- Fragmented MP4 whose movie header has a zero duration, and MKV with no
  Info/Duration. Check the duration the library scanner stores, not only what
  the parser returns (M7).
- A metadata parse that succeeds but is missing fields, to exercise the
  scanner's libVLC fallback (M7).
- Truncated MP4 sample tables, with preview requests past the last sample
  that's still in the file (W1).
- An ID3 title that happens to equal the file name stem keeps its leading
  number (W6).

## Engine tests

- Previews built from frames with the time burned in. Assert the preview's
  time and its visible aspect ratio, including odd sizes such as 96×54
  (W1, W2).
- Run the preview tests twice: once with no `ffmpeg` on PATH and once with it.
  Both audit machines had Homebrew or system FFmpeg installed, which hid M1
  when the app was started from a terminal.
- Check the first bytes of a snapshot file for each format setting
  (PNG, JPEG, TIFF) (M3, W4).
- Video track changes while paused and while playing (W3). Assert how long
  until the new picture shows: while playing, Windows showed black for about
  3 s after a switch.
- Audio pitch preservation and replay gain, measured from the output.

## Browser UI tests

- The player's deinterlace toggle matches the saved setting (M4).
- A subtitle-only `.mks` isn't listed in Videos (W7).
- History after a failed seek past the data doesn't say Watched (W5).
- Windows Help menu entries, including About (W8).
- An unavailable preview still leaves the time tooltip usable.
- A matching sidecar SRT is listed once (M6), and captions are placed clear of
  the controls (M5).

## Cua

- Cold TS and M2TS hover timing with Finder's PATH
  (`/usr/bin:/bin:/usr/sbin:/sbin`) and with a full PATH, over a few trials.
  Keep the first spinner screenshot (M1).
- Detect a Windows Security prompt (firewall) and skip the checks that need
  focus while it's open, instead of failing them. This prompt took focus on
  the PC and caused 10 of the Windows errors.
- Running `cargo test` on Windows triggers the same firewall prompt for the
  `revlc_lib` test binary, and the open prompt blocked seek-bar hovers in a
  later run. Allow it once on the PC, or run the Rust tests after the GUI
  checks.
- The Mac double-click fullscreen check failed on 2026-11-24 for both the 0.2.12
  and 0.2.11 releases. 0.2.11 passed it on 2026-11-22, so something in the test
  setup changed. Find out why Cua's background double-click no longer reaches
  the web view.
- A display selector, so the suite uses the Agent Test Display on the Mac.
- Refresh native window IDs after fullscreen.
- Log preview latency, source (native, ffmpeg, libVLC) and image size, so a
  hover that never arrived can be told apart from a decoder failure.
- Keep the real 125% and 150% DPI checks, Explorer file associations and
  uninstall restoration on Windows.
- Use a controlled audio output for any check that asserts volume, and a
  caption pixel check that tolerates the controls' gradient.

## New fixtures for generate-media

Faststart and fragmented MP4, MKV without cues or duration, AV1 in MP4 and
MKV, HEVC Main10 HDR10 (BT.2020/PQ bars), a rotated clip, a long 96×54 clip,
a truncated clip with time-stamped frames, two video tracks with distinct
colours and short GOPs, a subtitle-only MKS, M2TS with AAC in a private stream
and with AC-3, and tagged audio in every format (including AIFF and Opus
covers).

## Script fixes

- `scripts/qa/windows/generate-media.ps1`: flatten the directory array so it runs on Windows
  PowerShell 5.1.
- `scripts/qa/windows/native-tests.mjs`: import `execFileSync`.
- `scripts/qa/device/run-windows.ps1`: honour `QA_RUN`.
- Recording helper on Windows: report when it's done and support capturing
  only the app window, so other windows don't end up in recordings.
