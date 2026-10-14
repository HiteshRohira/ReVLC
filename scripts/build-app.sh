#!/usr/bin/env bash
# Builds the app and its signed updater files: on macOS, ReVLC.app, the
# updater archive and the .dmg; on Windows (Git Bash), the NSIS installer,
# which is also the update package.
#
# Updates are signed with the key in ~/.tauri/revlc-updater.key (or
# $TAURI_SIGNING_PRIVATE_KEY). Installed copies only accept updates signed
# with it, so keep it backed up and never commit it.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
KEY="${REVLC_UPDATER_KEY:-$HOME/.tauri/revlc-updater.key}"

if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  [[ -f "$KEY" ]] || { echo "Updater signing key not found at $KEY" >&2; exit 1; }
  # $(cat) drops a trailing newline, which the key must not have.
  export TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY")"
  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
fi

cd "$ROOT"
test -d src-tauri/vlc || bash scripts/prepare-libvlc.sh
npx tauri build
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) ;;
  *) scripts/make-dmg.sh ;;
esac
