# revlc — direction and first milestone

Status: proposed · Date: 2026-10-12

## Goal

Full **VLC feature parity** (every user-facing feature VLC has; see
`docs/parity.md`), with a modern UI, low resource use and easy 4K HDR.
Codec coverage only needs the popular formats, not everything VLC can decode.

## The single stack

**Rust core + Tauri 2 (system webview UI) + libVLC as the engine.**

| Layer | Choice |
|-------|--------|
| Engine | libVLC 3.0.x (stable), behind an `Engine` trait so we can move to 4.0 later |
| Core | Rust: player state, playlists, library (SQLite), settings, shortcuts, IPC |
| UI | Web UI (TypeScript + a light framework such as Svelte or Solid) in the OS webview |
| Video | libVLC draws into a native view placed **under** a transparent webview |

### Why libVLC (not libmpv)

Full parity rules out libmpv. VLC's features that mpv doesn't have are all
part of libVLC: stream output/conversion, Chromecast rendering, UPnP/SMB/SAP
network discovery, DVD/Blu-ray menus, capture devices, recording, and
audio/video filters (logo, marquee, mosaic, visualisations). Using libVLC
means we spend our time on the UI instead of re-implementing these.

### Why libVLC 3 now

libVLC 4.0 is still unreleased. Embedding into a native view
(`libvlc_media_player_set_nsobject` / `set_hwnd` / `set_xwindow`) works the
same in 3 and 4, so moving later is mostly API renames inside the `Engine`
trait. Risk to check in M1: how VLC 3 shows HDR on macOS (EDR, or tone-mapped
to SDR). If it only tone-maps, we test a libVLC 4 nightly in M2.

### Why Tauri (not Electron, Qt or a Rust UI toolkit)

- **Lightweight:** uses the OS webview (WKWebView, WebView2, WebKitGTK)
  instead of bundling Chromium. Apps are a few MB and use much less memory
  than Electron.
- **Modern UI quickly:** HTML/CSS/TS, the same UI on every platform.
- **We own the native window,** so we can put the video view under the
  webview: an `NSView` under a transparent `WKWebView` on macOS, DirectComposition
  with WebView2 on Windows, subsurfaces/GTK overlay on Linux. Electron makes this hard.
- **Rust** for everything that isn't UI, as originally intended.
- Rust UI toolkits (GPUI, Slint, Iced) aren't mature enough for a polished
  three-platform app with video and accessibility. Qt was ruled out.

**Trade-offs:** webviews behave slightly differently per OS (WebKitGTK on
Linux is the weakest), and the video-under-webview layer needs a small
amount of native code per OS. That code lives in one module per platform:

```
src-tauri/
  engine/        Engine trait + libVLC implementation (FFI)
  core/          player state, playlist, library, settings, shortcuts
  surface/
    macos.rs     NSView under WKWebView         ← M1
    windows.rs   HWND/DComp under WebView2      ← M4
    linux.rs     X11/Wayland                    ← M4
ui/              web UI
```

### Licensing

libVLC is LGPL 2.1+, Tauri is MIT/Apache. We can choose any licence; GPLv3
open source is still the simplest if we want to reuse VLC code directly.

## Milestones (macOS first)

**M1 — Player skeleton (about 1–2 weeks):** Tauri window, libVLC in an NSView
under a transparent webview, open a file or URL, play/pause/seek/volume,
audio and subtitle tracks, fullscreen, drag and drop, shortcuts. Check
HDR on the XDR display and the resource numbers against VLC.

**M2 — HDR and resources (time-boxed to 1 week):** correct HDR on XDR (libVLC
4 nightly if needed), no dropped frames over 5 minutes of 4K24 HDR / 4K60
SDR, controls over video without flicker. Compare memory, CPU, power
(`powermetrics`), cold start and app size against VLC.

**M3 — Parity, in the order of `docs/parity.md`.**

**M4 — Windows, then Linux:** shared code builds as-is; write `surface/` for each OS.

## Decision gate

We only change the stack if M1 shows that the video can't sit cleanly under
the webview on macOS (focus, fullscreen, resizing). Fallback in that case:
keep Rust and libVLC, render the controls natively in the same window.
