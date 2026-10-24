# Licences

ReVLC is Copyright (C) 2026 Hitesh Rohira, and free software under the GNU
General Public License, version 3 or (at your option) any later version. See
`LICENSE` (in the source code, the repository's top-level `LICENSE`).

ReVLC comes with ABSOLUTELY NO WARRANTY, to the extent permitted by law.

## libVLC and VLC plugins

The macOS and Windows packages include libVLC 3.0.24 and VLC plugins and data files (Lua scripts,
HRTF data) from VideoLAN's official VLC 3.0.24 builds, unmodified, with a
few plugins ReVLC doesn't need left out.

- libVLC (`libvlc`, `libvlccore`) is under the GNU Lesser General Public
  License, version 2.1 or later: `vlc/COPYING.LIB`.
- The plugins are under the GNU General Public License, version 2 or later,
  or the LGPL, version 2.1 or later: `vlc/COPYING`. Some contain third-party
  libraries under their own compatible licences.

Their complete source code, including the build recipes and patches for the
third-party libraries (`contrib/` in the tarball), is available from
VideoLAN: https://download.videolan.org/pub/videolan/vlc/3.0.24/

The experimental Linux package uses your distribution's libVLC 3 and VLC
plugin packages instead of bundling these libraries. Their version, source
packages, patches and notices are provided by the distribution's package
manager (`apt source vlc` on Mint/Ubuntu with source repositories enabled).

## FFmpeg

The macOS and Windows packages include an FFmpeg 9.0.2 program
(`ffmpeg/ffmpeg`, `ffmpeg/ffmpeg.exe`), unmodified, which makes the seek bar
previews. It is under the GNU General Public License, version 3 or later
(the same licence as ReVLC, in `LICENSE`), and contains third-party
libraries under their own compatible licences.

- macOS (Apple Silicon): Martin Riedl's static build,
  https://ffmpeg.martin-riedl.de/info/detail/macos/arm64/1789931890_9.0.2.
- Windows: Gyan Doshi's "essentials" build,
  https://github.com/GyanD/codexffmpeg/releases/tag/9.0.2 (build details:
  https://www.gyan.dev/ffmpeg/builds/).

FFmpeg's source code is available from https://ffmpeg.org/releases/
(`ffmpeg-9.0.2.tar.xz`); the sources of the libraries in each build are
listed on the pages above.

The Linux package uses your distribution's `ffmpeg` package instead.

"VLC" is a trademark of VideoLAN. ReVLC is an independent project and is not
affiliated with or endorsed by VideoLAN.

## Rust and JavaScript dependencies

The licences and copyright notices of the Rust crates and JavaScript
packages compiled into ReVLC are in `THIRD-PARTY-RUST.txt` and
`THIRD-PARTY-JS.txt`. Regenerate them with `scripts/third-party-licenses.sh`
after changing dependencies.

## Media

The subtitle preview in Settings is a still from Tears of Steel,
(CC) Blender Foundation, https://studio.blender.org/projects/tears-of-steel/,
under the Creative Commons Attribution 3.0 licence:
https://creativecommons.org/licenses/by/3.0/
