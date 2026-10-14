#!/usr/bin/env bash
# Collects a built release for GitHub and adds it to the update manifest.
#
#   scripts/release-files.sh [notes]
#
# Puts this platform's files in release/ under their published names (the
# .dmg and updater archive on macOS, the installer on Windows) and adds
# this platform to site/latest.json, which installed copies poll.
#
# Every platform in the manifest needs an entry, or its copies fail to
# check. So the manifest only moves to the new version once every platform
# in it has this release; until then the new entry waits under the old
# version and nobody is offered an update. Build each platform, run this on
# each, and commit site/latest.json in between.
#
# Upload the files to the GitHub release *before* deploying the site, or
# apps will be offered a download that doesn't exist yet.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE="$ROOT/src-tauri/target/release/bundle"
VERSION="$(cd "$ROOT" && node -p "require('./package.json').version")"
TAG="v$VERSION-alpha"
NOTES="${1:-See https://github.com/HiteshRohira/ReVLC/releases/tag/$TAG}"
DOWNLOAD="https://github.com/HiteshRohira/ReVLC/releases/download/$TAG"

OUT="$ROOT/release"
rm -rf "$OUT" && mkdir -p "$OUT"

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    platform="windows-x86_64"
    FILE="ReVLC-$VERSION-alpha-x64-setup.exe"
    cp "$BUNDLE/nsis/ReVLC_${VERSION}_x64-setup.exe" "$OUT/$FILE"
    SIG="$(cat "$BUNDLE/nsis/ReVLC_${VERSION}_x64-setup.exe.sig")"
    (cd "$OUT" && sha256sum "$FILE")
    ;;
  *)
    ARCH="$(uname -m)"
    [[ "$ARCH" == "arm64" ]] && platform="darwin-aarch64" || platform="darwin-x86_64"
    DMG="ReVLC-$VERSION-alpha-$ARCH.dmg"
    FILE="ReVLC-$VERSION-alpha-$ARCH.app.tar.gz"
    cp "$BUNDLE/dmg/ReVLC_${VERSION}_$ARCH.dmg" "$OUT/$DMG"
    cp "$BUNDLE/macos/ReVLC.app.tar.gz" "$OUT/$FILE"
    SIG="$(cat "$BUNDLE/macos/ReVLC.app.tar.gz.sig")"
    (cd "$OUT" && shasum -a 256 "$DMG")
    ;;
esac

cd "$ROOT"
VERSION="$VERSION" NOTES="$NOTES" PLATFORM="$platform" SIG="$SIG" TAG="$TAG" \
URL="$DOWNLOAD/$FILE" \
node -e '
  const fs = require("fs");
  const e = process.env;
  const path = "site/latest.json";
  const manifest = fs.existsSync(path) ? JSON.parse(fs.readFileSync(path, "utf8")) : { platforms: {} };
  manifest.platforms[e.PLATFORM] = { signature: e.SIG, url: e.URL };
  const waiting = Object.keys(manifest.platforms).filter((p) => !manifest.platforms[p].url.includes(`/${e.TAG}/`));
  if (waiting.length === 0) {
    manifest.version = e.VERSION;
    manifest.notes = e.NOTES;
    manifest.pub_date = new Date().toISOString().replace(/\.\d+Z$/, "Z");
    console.log(`site/latest.json now offers ${e.VERSION} on every platform`);
  } else {
    console.log(`site/latest.json stays at ${manifest.version}: ${waiting.join(", ")} still needs ${e.TAG}`);
  }
  fs.writeFileSync(path, JSON.stringify({ version: manifest.version, notes: manifest.notes, pub_date: manifest.pub_date, platforms: manifest.platforms }, null, 2) + "\n");
'
echo "Wrote $OUT for $TAG"
