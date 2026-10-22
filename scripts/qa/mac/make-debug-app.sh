#!/usr/bin/env bash
# Wraps the debug binary in a throwaway .app so computer-use tools, which only
# select apps by name, path or bundle ID, can drive it. Re-run after every
# debug build: it copies the current binary.
#
#   QA_RUN=~/revlc-qa/mac-<date> scripts/qa/mac/make-debug-app.sh
#
# Launch the printed binary path directly, with the QA env set, e.g.
#   REVLC_VLC_ARGS=--no-audio "$QA_RUN/ReVLC-QA-Debug.app/Contents/MacOS/revlc" &
# The debug build loads its UI from Vite, so keep `npm run dev` running.
# The app still uses the real data dir (app.revlc): back it up first.
set -euo pipefail
: "${QA_RUN:?set QA_RUN to the run folder}"
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
APP="$QA_RUN/ReVLC-QA-Debug.app/Contents"
mkdir -p "$APP/MacOS" "$APP/Resources"
cp "${CARGO_TARGET_DIR:-$ROOT/src-tauri/target}/debug/revlc" "$APP/MacOS/revlc"
cat > "$APP/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>revlc</string>
  <key>CFBundleIdentifier</key><string>app.revlc.qa-debug</string>
  <key>CFBundleName</key><string>ReVLC QA Debug</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
echo "$APP/MacOS/revlc"
