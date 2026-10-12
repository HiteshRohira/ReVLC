# VLC feature parity checklist

This is ReVLC's public roadmap. The goal is every user-facing feature of VLC
3.0 desktop, with a modern UI. Codec support covers the popular formats
rather than everything VLC can decode.

Main features come first. Ship, get feedback, then keep ticking boxes.

- [x] done   - [ ] not started   *(engine)* = libVLC already does it, we only need UI

## Playback
- [x] Play / pause / stop, seek bar, jump with keyboard (±5 s, ±30 s, 0–90%)
- [x] Playback speed (0.25×–4×)
- [x] Frame step
- [x] Previous / next, restart on "previous" after 3 s
- [ ] Jump to time…
- [ ] A–B loop
- [ ] Chapters and titles menu *(engine)*
- [ ] Bookmarks
- [ ] Resume where you left off
- [ ] Recent media
- [ ] Sleep timer

## Opening media
- [x] Open files (dialog, multiple), drag and drop, command line, Finder "Open With"
- [x] Open network URL: HTTP(S), HLS, DASH, RTSP, RTMP, UDP *(engine)*
- [x] YouTube and similar via VLC's Lua scripts *(engine)*
- [ ] Open folder (recursive)
- [ ] Open disc: DVD with menus, Blu-ray, audio CD, VCD, ISO *(engine)*
- [ ] Capture devices: webcam, microphone, screen *(engine)*

## Playlist
- [x] Queue, double-click to play, remove, clear
- [x] Drag to reorder
- [x] Repeat (off / all / one), shuffle
- [ ] Save / load M3U, XSPF, PLS
- [ ] Search and sort
- [ ] Media library (folders, metadata, artwork)

## Subtitles
- [x] Embedded track selection (SRT, ASS/SSA, PGS, VobSub, WebVTT, DVB…) *(engine)*
- [x] Load external subtitle file (dialog or drag and drop)
- [x] Subtitle delay
- [ ] Text encoding override
- [ ] Font, size, colour, outline, position
- [ ] Secondary subtitle track
- [ ] Teletext
- [ ] Online subtitle search

## Audio
- [x] Audio track selection
- [x] Volume up to 150% from the slider (200% from the keyboard), mute
- [x] Audio delay
- [ ] Output device selection
- [ ] Equaliser with presets
- [ ] Compressor, spatialiser, stereo widener, normalisation, pitch
- [ ] Stereo mode (left / right / reverse / mono)
- [ ] Passthrough (S/PDIF, HDMI)
- [ ] Visualisations

## Video
- [x] Aspect ratio override
- [x] Crop
- [x] Deinterlace
- [x] Snapshot (saved to Pictures)
- [x] Fullscreen
- [ ] Zoom and pan
- [ ] Rotate / flip
- [ ] Colour adjustments (brightness, contrast, hue, saturation, gamma)
- [ ] Sharpen and other effects
- [ ] Logo and marquee overlays
- [ ] Video track selection
- [ ] Always on top, mini player, picture-in-picture
- [ ] HDR shown as HDR on XDR displays (see plan M2)

## Convert, stream and record
- [ ] Convert / save to another format
- [ ] Stream to the network (HTTP, RTSP, UDP)
- [ ] Record while playing

## Network and casting
- [ ] Chromecast
- [ ] UPnP / DLNA browsing
- [ ] SMB, SFTP, FTP, NFS browsing
- [ ] SAP announcements
- [ ] Internet radio and podcast directories

## Control
- [x] Keyboard shortcuts (VLC-style) and a shortcut overlay (`?`)
- [ ] Configurable shortcuts
- [ ] Mouse wheel and gesture actions
- [ ] Media keys and the macOS Now Playing widget
- [ ] Web interface and remote control
- [ ] Command-line options

## Settings and extras
- [ ] Preferences (simple and advanced)
- [ ] Per-file settings
- [ ] Media information and codec details
- [ ] Statistics and log viewer
- [ ] Lua extensions

## Platforms
- [x] macOS (Apple Silicon), libVLC bundled in the app
- [ ] Signed and notarised macOS builds
- [ ] Windows
- [ ] Linux
