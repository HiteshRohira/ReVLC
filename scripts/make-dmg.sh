#!/usr/bin/env bash
# Packs the built ReVLC.app into a drag-to-Applications disk image.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE="$ROOT/src-tauri/target/release/bundle"
VERSION="$(node -p "require('$ROOT/package.json').version")"
OUT="$BUNDLE/dmg/ReVLC_${VERSION}_$(uname -m).dmg"

stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT
cp -R "$BUNDLE/macos/ReVLC.app" "$stage/"
ln -s /Applications "$stage/Applications"
mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"
hdiutil create -quiet -volname ReVLC -srcfolder "$stage" -format UDZO -ov "$OUT"
echo "$OUT ($(du -h "$OUT" | cut -f1))"
