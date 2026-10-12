#!/usr/bin/env bash
# Copies libVLC (lib/, plugins/, share/) into src-tauri/vlc so it gets bundled
# into ReVLC.app. Uses an installed VLC.app, or downloads the official one.
#
#   scripts/prepare-libvlc.sh [path/to/VLC.app]
set -euo pipefail

VLC_VERSION="${VLC_VERSION:-3.0.23}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/src-tauri/vlc"
APP="${1:-/Applications/VLC.app}"

if [[ ! -d "$APP/Contents/MacOS/lib" ]]; then
  arch="$(uname -m)"
  [[ "$arch" == "x86_64" ]] && flavour="intel64" || flavour="arm64"
  url="https://get.videolan.org/vlc/$VLC_VERSION/macosx/vlc-$VLC_VERSION-$flavour.dmg"
  tmp="$(mktemp -d)"
  echo "Downloading $url"
  curl -fL "$url" -o "$tmp/vlc.dmg"
  hdiutil attach -nobrowse -readonly -mountpoint "$tmp/mnt" "$tmp/vlc.dmg" >/dev/null
  trap 'hdiutil detach "$tmp/mnt" >/dev/null 2>&1 || true; rm -rf "$tmp"' EXIT
  APP="$tmp/mnt/VLC.app"
fi

SRC="$APP/Contents/MacOS"
echo "Copying libVLC from $SRC"
rm -rf "$DEST"
mkdir -p "$DEST/share"
# Resolve symlinks (libvlc.dylib -> libvlc.5.dylib) so bundling copies real files.
rsync -aL "$SRC/lib/" "$DEST/lib/"
# VLC's own Cocoa interface isn't needed; ReVLC is the interface.
rsync -a --exclude 'libmacosx_plugin.dylib' "$SRC/plugins/" "$DEST/plugins/"
# Lua scripts (YouTube and other sites, playlist parsers) and HRTF data.
rsync -a "$SRC/share/lua" "$SRC/share/hrtfs" "$DEST/share/"

du -sh "$DEST"
