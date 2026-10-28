#!/usr/bin/env bash
# Runs the Cua suite in a fresh macOS VM: the "does it work on a Mac that has
# never seen ReVLC" check. Each run clones a prepared VM, installs the DMG the
# way a user does (drag to Applications), runs the suite inside, copies the
# report back and deletes the clone, so every run starts from the same Mac.
#
#   scripts/qa/device/run-mac-vm.sh --dmg ReVLC_0.2.11_aarch64.dmg [--only playback,menus]
#
# The prepared VM (BASE, default revlc-vm) is a Tart image of macOS with
# Python + Pillow and Cua Driver installed and granted Accessibility and Screen
# Recording once by hand (System Settings in the VM; SIP keeps scripts out).
# A VM has no OpenGL, so libVLC can't show video there: the suite runs with
# --no-video, and video stays a real-Mac check (run-mac.sh).
#
# Needs Tart (https://tart.run) and about 10 GB of free disk while it runs.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
BASE="${BASE:-revlc-vm}"
dmg="" only=""
while (($#)); do
  case "$1" in
    --dmg) dmg="$2"; shift 2 ;;
    --only) only="$2"; shift 2 ;;
    *) echo "Unknown option $1" >&2; exit 2 ;;
  esac
done
[[ -f "$dmg" ]] || { echo "Pass --dmg with a ReVLC DMG" >&2; exit 2; }
command -v tart >/dev/null || { echo "Tart isn't installed" >&2; exit 1; }

vm="revlc-vm-run-$$"
run="$HOME/revlc-qa/vm-mac-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$run"
echo "Run folder: $run"
askpass="$run/askpass.sh"
printf '#!/bin/sh\necho admin\n' > "$askpass" && chmod +x "$askpass"

cleanup() {
  launchctl remove "$vm" 2>/dev/null || true
  tart stop "$vm" >/dev/null 2>&1 || true
  tart delete "$vm" >/dev/null 2>&1 || true
}
trap cleanup EXIT

tart clone "$BASE" "$vm"
# A launchd job, so the VM doesn't depend on this shell staying open.
launchctl submit -l "$vm" -o "$run/vm.log" -e "$run/vm.log" -- "$(command -v tart)" run --no-graphics "$vm"
ip="$(tart ip "$vm" --wait 300)"
# The Cirrus Labs images log in as admin/admin.
vssh() {
  SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 \
    ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR "admin@$ip" "$@"
}
for _ in {1..60}; do vssh 'who | grep -q console' 2>/dev/null && break; sleep 5; done
vssh 'sw_vers -productVersion' | tee "$run/macos-version.txt"

# The build and the test scripts, then "drag to Applications".
vssh 'rm -rf ~/qa && mkdir -p ~/qa/scripts'
vssh 'cat > ~/qa/ReVLC.dmg' < "$dmg"
tar -C "$ROOT" -czf - scripts/qa | vssh 'tar -C ~/qa -xzf -'
vssh 'set -e; m=$(mktemp -d); hdiutil attach -nobrowse -readonly -mountpoint "$m" ~/qa/ReVLC.dmg >/dev/null;
      rm -rf /Applications/ReVLC.app; cp -R "$m/ReVLC.app" /Applications/; hdiutil detach "$m" -quiet;
      defaults read /Applications/ReVLC.app/Contents/Info.plist CFBundleShortVersionString' | tee "$run/version.txt"

# Cua's daemon, in the logged-in session, with the grants made by hand.
vssh 'open -n -g /Applications/CuaDriver.app --args serve --permission-mode unrestricted --dangerously-bypass-approvals --no-overlay'
for _ in {1..30}; do vssh '~/.local/bin/cua-driver status' >/dev/null 2>&1 && break; sleep 2; done

args="--no-video --out ~/qa/run --ffmpeg /Applications/ReVLC.app/Contents/MacOS/ffmpeg --bundle /Applications/ReVLC.app"
[[ -n "$only" ]] && args="$args --only $only"
status=0
vssh "cd ~/qa && PATH=\$HOME/.local/bin:\$PATH /usr/local/bin/python3 scripts/qa/ci/cua_suite.py $args -- /Applications/ReVLC.app/Contents/MacOS/revlc" \
  2>&1 | tee "$run/suite.log" || status=$?
vssh 'tar -C ~/qa/run -czf - report.json report.md screenshots logs 2>/dev/null' | tar -C "$run" -xzf - || true
echo
echo "Report: $run/report.md"
exit "$status"
