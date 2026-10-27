#!/usr/bin/env bash
# Installs Cua Driver on a GitHub-hosted macOS runner, grants it Accessibility
# and Screen Recording, and starts its daemon. Only for disposable CI runners:
# it writes the system TCC database, which hosted runners allow because SIP is
# off there. Mirrors Cua's own hosted-runner setup (trycua/cua,
# scripts/ci/macos/run-hosted-rust-e2e.sh).
#
#   cua-macos-setup.sh LOG_DIR [RECORDER...]
#
# Each RECORDER (an executable path, such as FFmpeg) also gets Screen
# Recording, so it can record the screen with other apps' windows in it.
set -euo pipefail
LOGS="${1:?Pass a log directory}"
shift
mkdir -p "$LOGS"
APP=/Applications/CuaDriver.app
CLIENT=com.trycua.driver
TCC_DB="/Library/Application Support/com.apple.TCC/TCC.db"

csrutil status | tee "$LOGS/sip.txt"
sw_vers | tee "$LOGS/sw_vers.txt"

/bin/bash -c "$(curl -fsSL https://cua.ai/driver/install.sh)" -- --no-modify-path
echo "$HOME/.local/bin" >> "$GITHUB_PATH"
export PATH="$HOME/.local/bin:$PATH"
cua-driver --version | tee "$LOGS/cua-version.txt"

# Grant by bundle id, pinned to the app's signature like System Settings does.
requirement="$(codesign -d -r- "$APP" 2>&1 | awk -F 'designated => ' '/designated =>/ { print $2; exit }')"
[[ -n "$requirement" ]] || { echo "Could not read $APP's code requirement" >&2; exit 1; }
tmp="$(mktemp -d)"
printf '%s\n' "$requirement" > "$tmp/req.txt"
csreq -r "$tmp/req.txt" -b "$tmp/req.bin"
hex="$(od -An -tx1 -v "$tmp/req.bin" | tr -d ' \n')"
sudo sqlite3 "$TCC_DB" "INSERT OR REPLACE INTO access(
  service,client,client_type,auth_value,auth_reason,auth_version,csreq,flags,
  indirect_object_identifier_type,indirect_object_identifier,indirect_object_code_identity,last_modified)
VALUES
  ('kTCCServiceAccessibility','$CLIENT',0,2,2,1,X'$hex',0,0,'UNUSED',NULL,strftime('%s','now')),
  ('kTCCServiceScreenCapture','$CLIENT',0,2,2,1,X'$hex',0,0,'UNUSED',NULL,strftime('%s','now'));"
for recorder in "$@"; do
  # client_type 1 is a path; unsigned binaries have no code requirement.
  sudo sqlite3 "$TCC_DB" "INSERT OR REPLACE INTO access(
    service,client,client_type,auth_value,auth_reason,auth_version,csreq,flags,
    indirect_object_identifier_type,indirect_object_identifier,indirect_object_code_identity,last_modified)
  VALUES ('kTCCServiceScreenCapture','$(cd "$(dirname "$recorder")" && pwd)/$(basename "$recorder")',1,2,4,1,NULL,0,0,'UNUSED',NULL,strftime('%s','now'));"
done
sudo sqlite3 "$TCC_DB" "SELECT service,client,auth_value FROM access WHERE service IN ('kTCCServiceAccessibility','kTCCServiceScreenCapture');" | tee "$LOGS/tcc.txt"

# macOS 15+ also asks each screen-capturing app to reconfirm; pre-answer it.
approvals="$HOME/Library/Group Containers/group.com.apple.replayd/ScreenCaptureApprovals.plist"
mkdir -p "$(dirname "$approvals")"
defaults write "$approvals" "$CLIENT" -dict \
  kScreenCaptureApprovalLastAlerted -date "3024-01-01 00:00:00 +0000" \
  kScreenCaptureApprovalLastUsed -date "3024-01-01 00:00:00 +0000"
killall -HUP replayd 2>/dev/null || true

# Launch through LaunchServices so macOS attributes the grants to the app.
open -n -g "$APP" --args serve --permission-mode unrestricted --dangerously-bypass-approvals --no-overlay
for _ in {1..60}; do
  if cua-driver permissions status --json > "$LOGS/permissions.json" 2>/dev/null \
      && python3 -c 'import json,sys; p=json.load(open(sys.argv[1])); sys.exit(not (p.get("accessibility") and p.get("screen_recording")))' "$LOGS/permissions.json"; then
    cat "$LOGS/permissions.json"
    exit 0
  fi
  sleep 1
done
cat "$LOGS/permissions.json" || true
echo "Cua Driver did not report Accessibility and Screen Recording" >&2
exit 1
