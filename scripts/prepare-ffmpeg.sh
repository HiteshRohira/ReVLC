#!/usr/bin/env bash
# Puts FFmpeg in src-tauri/ffmpeg so it gets bundled into the app: it makes
# the seek bar previews, decoding a video's keyframes on the GPU
# (VideoToolbox on macOS, D3D11VA on Windows). On macOS (Apple Silicon),
# Martin Riedl's static build; on Windows (Git Bash), Gyan Doshi's
# "essentials" build. Both are pinned to a version and checksum.
#
# Does nothing if src-tauri/ffmpeg already holds $FFMPEG_VERSION. When the
# version changes, update the URLs and checksums below and the version and
# source links in licenses/README.md.
set -euo pipefail

FFMPEG_VERSION="9.0.2"
MAC_URL="https://ffmpeg.martin-riedl.de/download/macos/arm64/1789931890_9.0.2/ffmpeg.zip"
MAC_SHA256="c8ed4c4e6978a03c485edbfe4e0a5dc2380f8a30bba5150531b31b094492d924"
WIN_URL="https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/ffmpeg-9.0.2-essentials_build.zip"
WIN_SHA256="60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$ROOT/src-tauri/ffmpeg"

if [[ "$(uname -s)" == Linux ]]; then
  echo "Linux uses the distribution's FFmpeg. On Mint/Ubuntu, install:"
  echo "  sudo apt install ffmpeg"
  exit 0
fi

if [[ "$(cat "$DEST/VERSION" 2>/dev/null)" == "$FFMPEG_VERSION" ]]; then
  echo "FFmpeg $FFMPEG_VERSION is already in $DEST"
  exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    echo "Downloading $WIN_URL"
    curl -fL "$WIN_URL" -o "$tmp/ffmpeg.zip"
    echo "$WIN_SHA256 *$tmp/ffmpeg.zip" | sha256sum -c -
    unzip -q -j "$tmp/ffmpeg.zip" "*/bin/ffmpeg.exe" -d "$tmp"
    rm -rf "$DEST"
    mkdir -p "$DEST"
    cp "$tmp/ffmpeg.exe" "$DEST/"
    ;;
  *)
    if [[ "$(uname -m)" != arm64 ]]; then
      # Not an error: without FFmpeg, ReVLC makes previews with libVLC. The
      # folder must still exist, as the build bundles it.
      echo "Skipping FFmpeg: only Apple Silicon builds are bundled." >&2
      mkdir -p "$DEST"
      exit 0
    fi
    echo "Downloading $MAC_URL"
    curl -fL "$MAC_URL" -o "$tmp/ffmpeg.zip"
    echo "$MAC_SHA256 *$tmp/ffmpeg.zip" | shasum -a 256 -c -
    unzip -q "$tmp/ffmpeg.zip" ffmpeg -d "$tmp"
    rm -rf "$DEST"
    mkdir -p "$DEST"
    cp "$tmp/ffmpeg" "$DEST/"
    chmod +x "$DEST/ffmpeg"
    ;;
esac

echo "$FFMPEG_VERSION" >"$DEST/VERSION"
du -sh "$DEST"
