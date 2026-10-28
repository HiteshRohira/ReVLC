#!/usr/bin/env bash
# Runs inside the Ubuntu guest (from run-vm.ps1, through guestcontrol as the
# VM's user): installs ReVLC the way a user would (apt install ./x.deb), then
# runs the Cua suite against the installed app in the logged-in desktop
# session. Writes OUT_DIR/result.txt (install details, the run folder).
#
#   SUDO_PASS=... guest-run-linux.sh DEB OUT_DIR [--only NAME,...]
set -uo pipefail
deb="$(realpath "$1")" out="$2"; shift 2
mkdir -p "$out"
result="$out/result.txt"
: > "$result"
repo="$(cd "$(dirname "$0")/../../.." && pwd)"
sudo_() { printf '%s\n' "$SUDO_PASS" | sudo -S -p '' "$@"; }

# guestcontrol runs outside the desktop session: borrow its environment.
shell_pid="$(pgrep -u "$(id -u)" -x gnome-shell | head -1)"
[[ -n "$shell_pid" ]] || { echo "No desktop session (gnome-shell) for $(id -un)" >> "$result"; exit 1; }
while IFS= read -r -d '' kv; do
  case "$kv" in DISPLAY=*|XAUTHORITY=*|DBUS_SESSION_BUS_ADDRESS=*|XDG_RUNTIME_DIR=*|XDG_SESSION_TYPE=*|WAYLAND_DISPLAY=*) export "$kv" ;; esac
done < "/proc/$shell_pid/environ"
export PATH="$HOME/.local/bin:$PATH"
echo "Session: ${XDG_SESSION_TYPE:-?} DISPLAY=${DISPLAY:-}" >> "$result"

start=$SECONDS
DEBIAN_FRONTEND=noninteractive sudo_ apt-get -o DPkg::Lock::Timeout=600 install -y "$deb" > "$out/apt-install.log" 2>&1
echo "apt install exit code: $? after $((SECONDS - start)) s" >> "$result"
pkg="$(dpkg-deb -f "$deb" Package)"
echo "Installed: $pkg $(dpkg-query -W -f '${Version}' "$pkg" 2>/dev/null)" >> "$result"
app="$(dpkg -L "$pkg" 2>/dev/null | grep -m1 '^/usr/bin/')"
echo "Installed exe: $app" >> "$result"
[[ -n "$app" ]] || exit 1

# WebKitGTK only builds its accessibility tree when GNOME's toolkit
# accessibility is on (the screen-reader flag of Windows). Restored after.
was_a11y="$(gsettings get org.gnome.desktop.interface toolkit-accessibility)"
gsettings set org.gnome.desktop.interface toolkit-accessibility true

run="$HOME/revlc-qa/vm-linux-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$run/logs"
if ! cua-driver status >/dev/null 2>&1; then
  cua-driver serve >"$run/logs/cua-serve.log" 2>&1 &
  for _ in {1..30}; do cua-driver status >/dev/null 2>&1 && break; sleep 1; done
fi
export CUA_DAEMON_RESTART="cua-driver serve >/dev/null 2>&1 &"
# The recording is evidence only; the run does not depend on it.
size="$(xdpyinfo 2>/dev/null | awk '/dimensions:/ {print $2}')"
ffmpeg -hide_banner -loglevel error -y -f x11grab -framerate 15 ${size:+-video_size "$size"} -i "$DISPLAY" \
  -vf "scale=1280:-2" -c:v libx264 -pix_fmt yuv420p "$run/recording.mp4" >"$run/recording.log" 2>&1 &
rec=$!
status=0
python3 -I "$repo/scripts/qa/ci/cua_suite.py" --out "$run" --ffmpeg ffmpeg "$@" -- "$app" > "$run/suite.log" 2>&1 || status=$?
kill -INT "$rec" 2>/dev/null; wait "$rec" 2>/dev/null
gsettings set org.gnome.desktop.interface toolkit-accessibility "$was_a11y"

echo "Suite exit code: $status" >> "$result"
echo "Run folder: $run" >> "$result"
