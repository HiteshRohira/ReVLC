# ReVLC

A modern, lightweight media player built on libVLC.

**[revlc.vercel.app](https://revlc.vercel.app)** · [Download the alpha](https://github.com/HiteshRohira/ReVLC/releases/tag/v0.1.0-alpha)

ReVLC aims for full VLC feature parity with a cleaner, faster UI. It is early:
see the [feature parity checklist](docs/parity.md) for what works today and
what's next.

![ReVLC playing a video with subtitles](docs/screenshot.jpg)

## Stack

- **Engine:** libVLC 3 (the same engine as VLC), loaded at runtime
- **Core:** Rust (player state, playlist, events)
- **UI:** Tauri 2 + Svelte 5, rendered by the system web view
- **Video:** libVLC draws into a native view placed under a transparent web
  view, so the HTML controls sit on top of hardware-decoded video

See [docs/plan.md](docs/plan.md) for the reasoning and milestones.

## Install (macOS, Apple Silicon)

Download the `.dmg` from [Releases](../../releases) and drag ReVLC to
Applications. Builds aren't notarised yet, so the first time, right-click
ReVLC and choose **Open**, or run `xattr -cr /Applications/ReVLC.app`.

## Building

Requirements: Rust, Node 20+.

```sh
npm install
npm run prepare:vlc   # copies libVLC from /Applications/VLC.app, or downloads VLC 3
npm run tauri dev     # run in development
npm run build:app     # ReVLC.app and a .dmg in src-tauri/target/release/bundle
```

`prepare:vlc` puts libVLC (libraries, plugins and Lua scripts, about 80 MB)
in `src-tauri/vlc`, which is bundled into the app. Without it, development
builds fall back to an installed VLC.app. Set `REVLC_VLC_DIR` to use libVLC
from somewhere else (a directory with `lib/`, `plugins/` and `share/`).

## Known issues

- Subtitles can overlap the control bar while it's visible (Settings → Subtitles → "Raise subtitles by" moves them up).
- HDR is tone-mapped to SDR by libVLC 3 (true HDR on XDR displays is milestone M2).
- Only macOS so far; Windows and Linux are next.

## Contributing

Pick an unchecked item in [docs/parity.md](docs/parity.md). Most "engine"
items only need UI and a thin command in `src-tauri/src/vlc.rs`.

## License

GPL-3.0-or-later. The app bundles libVLC (LGPL-2.1-or-later) and VLC
plugins (GPL/LGPL) from [VideoLAN](https://www.videolan.org/); their
source code is available at https://code.videolan.org/videolan/vlc.
