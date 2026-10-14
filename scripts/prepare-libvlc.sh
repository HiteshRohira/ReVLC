#!/usr/bin/env bash
# Copies libVLC into src-tauri/vlc so it gets bundled
# into the app. On macOS, uses an installed VLC.app or downloads the official
# one; on Windows (Git Bash), downloads the official win64 zip.
#
#   scripts/prepare-libvlc.sh [path/to/VLC.app]
set -euo pipefail

VLC_VERSION="${VLC_VERSION:-3.0.23}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/src-tauri/vlc"

prepare_windows() {
  local url="https://get.videolan.org/vlc/$VLC_VERSION/win64/vlc-$VLC_VERSION-win64.zip"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  echo "Downloading $url"
  curl -fL "$url" -o "$tmp/vlc.zip"
  local sum
  sum="$(curl -fsSL "$url.sha256" | cut -d' ' -f1)"
  echo "$sum *$tmp/vlc.zip" | sha256sum -c -
  unzip -q "$tmp/vlc.zip" -d "$tmp"
  local src="$tmp/vlc-$VLC_VERSION"

  # VLC's own layout: libVLC finds plugins/ and lua/ next to libvlccore.dll.
  rm -rf "$DEST"
  mkdir -p "$DEST"
  cp "$src/libvlc.dll" "$src/libvlccore.dll" "$DEST/"
  cp -r "$src/plugins" "$DEST/plugins"
  # VLC's own interfaces aren't needed; ReVLC is the interface.
  rm -rf "$DEST/plugins/gui"
  # Lua scripts (YouTube and other sites, playlist parsers) and HRTF data.
  cp -r "$src/lua" "$src/hrtfs" "$DEST/"
  # A plugin cache makes libVLC start faster.
  "$src/vlc-cache-gen.exe" "$DEST/plugins"
}

prepare_macos() {
  local app="${1:-/Applications/VLC.app}"
  if [[ ! -d "$app/Contents/MacOS/lib" ]]; then
    local arch flavour url
    arch="$(uname -m)"
    [[ "$arch" == "x86_64" ]] && flavour="intel64" || flavour="arm64"
    url="https://get.videolan.org/vlc/$VLC_VERSION/macosx/vlc-$VLC_VERSION-$flavour.dmg"
    tmp="$(mktemp -d)"
    echo "Downloading $url"
    curl -fL "$url" -o "$tmp/vlc.dmg"
    hdiutil attach -nobrowse -readonly -mountpoint "$tmp/mnt" "$tmp/vlc.dmg" >/dev/null
    trap 'hdiutil detach "$tmp/mnt" >/dev/null 2>&1 || true; rm -rf "$tmp"' EXIT
    app="$tmp/mnt/VLC.app"
  fi

  local src="$app/Contents/MacOS"
  echo "Copying libVLC from $src"
  rm -rf "$DEST"
  mkdir -p "$DEST/share"
  # Resolve symlinks (libvlc.dylib -> libvlc.5.dylib) so bundling copies real files.
  rsync -aL "$src/lib/" "$DEST/lib/"
  # VLC's own Cocoa interface isn't needed; ReVLC is the interface.
  rsync -a --exclude 'libmacosx_plugin.dylib' "$src/plugins/" "$DEST/plugins/"
  # Lua scripts (YouTube and other sites, playlist parsers) and HRTF data.
  rsync -a "$src/share/lua" "$src/share/hrtfs" "$DEST/share/"
}

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) prepare_windows ;;
  *) prepare_macos "$@" ;;
esac

du -sh "$DEST"
