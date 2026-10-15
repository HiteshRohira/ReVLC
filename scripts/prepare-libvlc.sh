#!/usr/bin/env bash
# Copies libVLC into src-tauri/vlc so it gets bundled
# into the app. On macOS, uses an installed VLC.app or downloads the official
# one; on Windows (Git Bash), downloads the official win64 zip.
#
#   scripts/prepare-libvlc.sh [path/to/VLC.app]
#
# Does nothing if src-tauri/vlc already holds $VLC_VERSION. Keep this at the
# latest VLC 3 release: older ones have known security holes.
set -euo pipefail

VLC_VERSION="${VLC_VERSION:-3.0.24}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/src-tauri/vlc"

if [[ $# -eq 0 && "$(cat "$DEST/VERSION" 2>/dev/null)" == "$VLC_VERSION" ]]; then
  echo "libVLC $VLC_VERSION is already in $DEST"
  exit 0
fi

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
  # Lua scripts (sites like Vimeo, playlist parsers) and HRTF data.
  cp -r "$src/lua" "$src/hrtfs" "$DEST/"
  # A plugin cache makes libVLC start faster.
  "$src/vlc-cache-gen.exe" "$DEST/plugins"
}

prepare_macos() {
  local app="${1:-/Applications/VLC.app}"
  local installed
  installed="$(defaults read "$app/Contents/Info.plist" CFBundleShortVersionString 2>/dev/null || true)"
  if [[ -z "${1:-}" && "$installed" != "$VLC_VERSION" ]]; then
    # An installed VLC of another version isn't what we ship.
    app=""
  elif [[ -n "${1:-}" && "$installed" != "$VLC_VERSION" ]]; then
    echo "Warning: $app is VLC $installed, not $VLC_VERSION" >&2
  fi
  if [[ ! -d "$app/Contents/MacOS/lib" ]]; then
    local arch flavour url
    arch="$(uname -m)"
    [[ "$arch" == "x86_64" ]] && flavour="intel64" || flavour="arm64"
    url="https://get.videolan.org/vlc/$VLC_VERSION/macosx/vlc-$VLC_VERSION-$flavour.dmg"
    tmp="$(mktemp -d)"
    echo "Downloading $url"
    curl -fL "$url" -o "$tmp/vlc.dmg"
    local sum
    sum="$(curl -fsSL "$url.sha256" | cut -d' ' -f1)"
    echo "$sum *$tmp/vlc.dmg" | shasum -a 256 -c -
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
  # Lua scripts (sites like Vimeo, playlist parsers) and HRTF data.
  rsync -a "$src/share/lua" "$src/share/hrtfs" "$DEST/share/"
}

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) prepare_windows ;;
  *) prepare_macos "$@" ;;
esac

echo "$VLC_VERSION" >"$DEST/VERSION"

du -sh "$DEST"
