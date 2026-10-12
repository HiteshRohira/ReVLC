# revlc

A modern, lightweight media player built on libVLC.

revlc aims for full VLC feature parity with a cleaner, faster UI. It is early:
see the [feature parity checklist](docs/parity.md) for what works today and
what's next.

## Stack

- **Engine:** libVLC 3 (the same engine as VLC), loaded at runtime
- **Core:** Rust (player state, playlist, events)
- **UI:** Tauri 2 + Svelte 5, rendered by the system web view
- **Video:** libVLC draws into a native view placed under a transparent web
  view, so the HTML controls sit on top of hardware-decoded video

See [docs/plan.md](docs/plan.md) for the reasoning and milestones.

## Running (macOS)

Requirements: Rust, Node 20+, and [VLC 3](https://www.videolan.org/vlc/)
installed in `/Applications` (revlc uses its libVLC until we bundle our own).

```sh
npm install
npm run tauri dev
# or open a file directly:
npx tauri dev -- -- /path/to/video.mkv
```

To use libVLC from somewhere else, set `REVLC_VLC_DIR` to a directory
containing `lib/`, `plugins/` and `share/`.

## Contributing

Pick an unchecked item in [docs/parity.md](docs/parity.md). Most "engine"
items only need UI and a thin command in `src-tauri/src/vlc.rs`.

## License

GPL-3.0-or-later. libVLC is LGPL-2.1-or-later.
