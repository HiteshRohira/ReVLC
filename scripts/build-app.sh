#!/usr/bin/env bash
# Builds the app: on macOS, ReVLC.app, its signed updater archive and the
# .dmg; on Windows (Git Bash), the NSIS installer.
#
# The macOS updater archive is signed with the key in
# ~/.tauri/revlc-updater.key (or $TAURI_SIGNING_PRIVATE_KEY). Installed
# copies only accept updates signed with it, so keep it backed up and never
# commit it.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KEY="${REVLC_UPDATER_KEY:-$HOME/.tauri/revlc-updater.key}"

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) windows=1 ;;
  *) windows=0 ;;
esac

# Windows builds don't make updater artifacts yet (tauri.windows.conf.json).
if [[ $windows == 0 && -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  [[ -f "$KEY" ]] || { echo "Updater signing key not found at $KEY" >&2; exit 1; }
  export TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY")"
  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
fi

cd "$ROOT"
test -d src-tauri/vlc || bash scripts/prepare-libvlc.sh
npx tauri build
[[ $windows == 1 ]] || scripts/make-dmg.sh
