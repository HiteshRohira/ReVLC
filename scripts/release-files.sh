#!/usr/bin/env bash
# Collects a built release for GitHub and writes the update manifest.
#
#   scripts/release-files.sh [notes]
#
# Puts the .dmg and updater archive in release/ under their published
# names, and writes site/latest.json, which installed copies poll. Upload
# the files to the GitHub release *before* deploying the site, or apps
# will be offered a download that doesn't exist yet.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE="$ROOT/src-tauri/target/release/bundle"
VERSION="$(cd "$ROOT" && node -p "require('./package.json').version")"
TAG="v$VERSION-alpha"
ARCH="$(uname -m)"
NOTES="${1:-See https://github.com/HiteshRohira/ReVLC/releases/tag/$TAG}"

OUT="$ROOT/release"
rm -rf "$OUT" && mkdir -p "$OUT"

# Windows: just the installer. It has no updater archive yet, so
# site/latest.json (the macOS manifest) is left alone.
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    SETUP="ReVLC-$VERSION-alpha-x64-setup.exe"
    cp "$BUNDLE/nsis/ReVLC_${VERSION}_x64-setup.exe" "$OUT/$SETUP"
    (cd "$OUT" && sha256sum "$SETUP")
    echo "Wrote $OUT for $TAG"
    exit 0
    ;;
esac
DMG="ReVLC-$VERSION-alpha-$ARCH.dmg"
ARCHIVE="ReVLC-$VERSION-alpha-$ARCH.app.tar.gz"
cp "$BUNDLE/dmg/ReVLC_${VERSION}_$ARCH.dmg" "$OUT/$DMG"
cp "$BUNDLE/macos/ReVLC.app.tar.gz" "$OUT/$ARCHIVE"

[[ "$ARCH" == "arm64" ]] && platform="darwin-aarch64" || platform="darwin-x86_64"
VERSION="$VERSION" NOTES="$NOTES" PLATFORM="$platform" \
URL="https://github.com/HiteshRohira/ReVLC/releases/download/$TAG/$ARCHIVE" \
SIG="$(cat "$BUNDLE/macos/ReVLC.app.tar.gz.sig")" \
node -e '
  const e = process.env;
  const manifest = {
    version: e.VERSION,
    notes: e.NOTES,
    pub_date: new Date().toISOString().replace(/\.\d+Z$/, "Z"),
    platforms: { [e.PLATFORM]: { signature: e.SIG, url: e.URL } },
  };
  process.stdout.write(JSON.stringify(manifest, null, 2) + "\n");
' > "$ROOT/site/latest.json"

(cd "$OUT" && shasum -a 256 "$DMG")
echo "Wrote $OUT and site/latest.json for $TAG"
