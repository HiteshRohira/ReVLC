#!/usr/bin/env bash
# Makes the prepared macOS VM that run-mac-vm.sh clones for each run:
# Cirrus Labs' plain macOS image plus Python with Pillow and the pinned Cua
# Driver (scripts/qa/cua-version.txt). Run it once, or again to rebuild the
# VM from scratch (say, for a new macOS):
#
#   scripts/qa/device/make-mac-vm.sh [--replace]
#
# One step is by hand: macOS doesn't let scripts grant Accessibility and
# Screen Recording. The script opens the VM's window and the right page of
# System Settings, waits for you, then checks the grants and shuts it down.
#
# Needs Tart (https://tart.run), about 30 GB of free disk for the image and
# a network connection for the downloads.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
BASE="${BASE:-revlc-vm}"
IMAGE="${IMAGE:-ghcr.io/cirruslabs/macos-tahoe-vanilla:latest}"
PYTHON_VERSION="3.13.9"
CUA_VERSION="$(cat "$ROOT/scripts/qa/cua-version.txt")"
replace=""
while (($#)); do
  case "$1" in
    --replace) replace=1; shift ;;
    *) echo "Unknown option $1" >&2; exit 2 ;;
  esac
done
command -v tart >/dev/null || { echo "Tart isn't installed" >&2; exit 1; }
if tart list --quiet 2>/dev/null | grep -qx "$BASE"; then
  [[ -n "$replace" ]] || { echo "$BASE already exists; pass --replace to rebuild it" >&2; exit 1; }
  tart delete "$BASE"
fi

work="$(mktemp -d)"
askpass="$work/askpass.sh"
printf '#!/bin/sh\necho admin\n' > "$askpass" && chmod +x "$askpass"
job="$BASE-setup-$$"
cleanup() {
  launchctl remove "$job" 2>/dev/null || true
  tart stop "$BASE" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

tart clone "$IMAGE" "$BASE"
# Headless first, for the scripted part.
launchctl submit -l "$job" -o "$work/vm.log" -e "$work/vm.log" -- "$(command -v tart)" run --no-graphics "$BASE"
ip="$(tart ip "$BASE" --wait 300)"
# The Cirrus Labs images log in as admin/admin.
vssh() {
  SSH_ASKPASS="$askpass" SSH_ASKPASS_REQUIRE=force DISPLAY=:0 \
    ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR "admin@$ip" "$@"
}
logged_in=""
for _ in {1..60}; do vssh 'who | grep -q console' 2>/dev/null && { logged_in=1; break; }; sleep 5; done
[[ -n "$logged_in" ]] || { echo "The VM never logged in to its desktop (5 min)." >&2; exit 1; }

# Python from python.org (the suite runs /usr/local/bin/python3), and Pillow.
pkg="$HOME/revlc-qa/cache/python-$PYTHON_VERSION-macos11.pkg"
if [[ ! -f "$pkg" ]]; then
  mkdir -p "$(dirname "$pkg")"
  curl -fL --retry 3 -o "$pkg.part" "https://www.python.org/ftp/python/$PYTHON_VERSION/python-$PYTHON_VERSION-macos11.pkg"
  mv "$pkg.part" "$pkg"
fi
vssh 'cat > /tmp/py.pkg' < "$pkg"
vssh 'echo admin | sudo -S installer -pkg /tmp/py.pkg -target / >/dev/null && rm /tmp/py.pkg'
vssh '/usr/local/bin/python3 -m pip install --quiet pillow && /usr/local/bin/python3 -c "import PIL; print(\"Pillow\", PIL.__version__)"'

# Cua Driver, the version the suite is pinned to.
vssh "CUA_DRIVER_RS_VERSION=$CUA_VERSION CUA_TELEMETRY=0 /bin/bash -c \"\$(curl -fsSL https://cua.ai/driver/install.sh)\" -- --no-modify-path >/dev/null"
have="$(vssh '~/.local/bin/cua-driver --version')"
[[ "$have" == "cua-driver $CUA_VERSION" ]] || { echo "Installed $have, wanted $CUA_VERSION" >&2; exit 1; }

# The grants, by hand, in the VM's window.
launchctl remove "$job"
tart stop "$BASE" >/dev/null
launchctl submit -l "$job" -o "$work/vm.log" -e "$work/vm.log" -- "$(command -v tart)" run "$BASE"
ip="$(tart ip "$BASE" --wait 300)"
for _ in {1..60}; do vssh 'who | grep -q console' 2>/dev/null && break; sleep 5; done
vssh 'open -n -g /Applications/CuaDriver.app --args serve --no-overlay; sleep 3;
      open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"'
cat <<'EOF'

In the VM's window, System Settings › Privacy & Security:
  1. Accessibility: turn on CuaDriver (add it with + from /Applications if it isn't listed).
  2. Screen & System Audio Recording: turn on CuaDriver the same way.
Then come back here and press Return.
EOF
read -r
granted="$(vssh '~/.local/bin/cua-driver call check_permissions "{}"' 2>&1 || true)"
echo "$granted"
if ! python3 -c 'import json, sys; d = json.load(sys.stdin); sys.exit(0 if d.get("accessibility") and d.get("screen_recording") else 1)' <<<"$granted" 2>/dev/null; then
  echo "Cua Driver doesn't have both grants yet; run this again with --replace." >&2
  exit 1
fi
vssh 'echo admin | sudo -S shutdown -h now' >/dev/null 2>&1 || true
for _ in {1..60}; do tart list | grep -E "^local +$BASE " | grep -q stopped && break; sleep 2; done
echo "$BASE is ready. run-mac-vm.sh clones it for each run."
