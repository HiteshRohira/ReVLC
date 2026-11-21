#!/usr/bin/env bash
# Optional developer/test helper: downloads a pinned FFmpeg binary into
# src-tauri/binaries for generating synthetic test media (`vlc_it` /
# `keyframes_it`). ReVLC no longer bundles FFmpeg as an app sidecar — seek
# bar previews use native OS hardware decoders (VideoToolbox on macOS,
# Media Foundation + D3D11VA on Windows) with an optional fallback to any
# system `ffmpeg` on PATH or libVLC.
#
# Does nothing if src-tauri/binaries already holds $FFMPEG_VERSION.
set -euo pipefail

FFMPEG_VERSION="9.0.2"
MAC_ARM64_URL="https://ffmpeg.martin-riedl.de/download/macos/arm64/1789931890_9.0.2/ffmpeg.zip"
MAC_ARM64_SHA256="c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924"
MAC_X64_URL="https://ffmpeg.martin-riedl.de/download/macos/amd64/1789931006_9.0.2/ffmpeg.zip"
MAC_X64_SHA256="7c6b4125b191cbf773832dc51f424cf2b6bb7da43007d1e066f95909e47cacd4"
WIN_URL="https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/ffmpeg-9.0.2-essentials_build.zip"
WIN_SHA256="60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/src-tauri/binaries"

if [[ "$(uname -s)" == Linux ]]; then
  echo "Linux uses the distribution's FFmpeg. On Mint/Ubuntu, install:"
  echo "  sudo apt install ffmpeg"
  exit 0
fi

# Tauri names a sidecar after the Rust target it runs on.
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) OUT="$DEST/ffmpeg-$TRIPLE.exe" ;;
  *) OUT="$DEST/ffmpeg-$TRIPLE" ;;
esac
if [[ -s "$OUT" && "$(cat "$DEST/.ffmpeg-version" 2>/dev/null)" == "$FFMPEG_VERSION" ]]; then
  echo "FFmpeg $FFMPEG_VERSION is already in $DEST"
  exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$DEST"
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    echo "Downloading $WIN_URL"
    curl -fL "$WIN_URL" -o "$tmp/ffmpeg.zip"
    echo "$WIN_SHA256 *$tmp/ffmpeg.zip" | sha256sum -c -
    unzip -q -j "$tmp/ffmpeg.zip" "*/bin/ffmpeg.exe" -d "$tmp"
    cp "$tmp/ffmpeg.exe" "$OUT"
    ;;
  *)
    if [[ "$(uname -m)" == arm64 ]]; then
      url="$MAC_ARM64_URL" sum="$MAC_ARM64_SHA256"
    else
      url="$MAC_X64_URL" sum="$MAC_X64_SHA256"
    fi
    echo "Downloading $url"
    curl -fL "$url" -o "$tmp/ffmpeg.zip"
    echo "$sum *$tmp/ffmpeg.zip" | shasum -a 256 -c -
    unzip -q "$tmp/ffmpeg.zip" ffmpeg -d "$tmp"
    cp "$tmp/ffmpeg" "$OUT"
    chmod +x "$OUT"
    ;;
esac

echo "$FFMPEG_VERSION" >"$DEST/.ffmpeg-version"
du -sh "$OUT"
