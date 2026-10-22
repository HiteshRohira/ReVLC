#!/usr/bin/env bash
# Runs the installed .deb in an isolated CI desktop. Never enables analytics.
# Requires an X server (xvfb-run in CI) and the tools in linux-build.yml.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RUN="${1:?Pass an absolute evidence directory}"
mkdir -p "$RUN/screenshots" "$RUN/logs"
python3 -I "$ROOT/scripts/qa/appdata.py" backup "$RUN"
python3 -I "$ROOT/scripts/qa/appdata.py" seed "$RUN"
app_pid=""
cleanup() {
  if [[ -n "$app_pid" ]]; then
    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
  fi
  python3 -I "$ROOT/scripts/qa/appdata.py" restore "$RUN"
}
trap cleanup EXIT
openbox >"$RUN/logs/openbox.log" 2>&1 &
xcompmgr >"$RUN/logs/compositor.log" 2>&1 &
ffmpeg -hide_banner -loglevel error -y -f lavfi -i 'testsrc2=size=640x360:rate=24' -t 60 -c:v libx264 -pix_fmt yuv420p "$RUN/Linux playback.mp4"
# Xvfb has no DRI3/VA-API device. Exercise the same embedded X11 surface
# with WebKit's non-DMA-BUF path and VLC's software X11 video output.
WEBKIT_DISABLE_DMABUF_RENDERER=1 REVLC_VLC_ARGS='--vout=xcb_x11' revlc "$RUN/Linux playback.mp4" >"$RUN/logs/revlc.log" 2>&1 &
app_pid=$!
for _ in {1..30}; do
  kill -0 "$app_pid"
  window="$(xdotool search --onlyvisible --pid "$app_pid" --class revlc 2>/dev/null | head -1 || true)"
  [[ -n "$window" ]] && break
  sleep 1
done
[[ -n "${window:-}" ]] || { echo 'ReVLC window did not appear' >&2; exit 1; }
xdotool windowactivate --sync "$window"
sleep 8
xwininfo -root -tree >"$RUN/logs/windows.txt"
import -window "$window" "$RUN/screenshots/window.png"
import -window root "$RUN/screenshots/playing.png"
xdotool key --clearmodifiers space
sleep 1
import -window root "$RUN/screenshots/paused.png"
sleep 1
import -window root "$RUN/screenshots/paused-still.png"
xdotool key --clearmodifiers f
sleep 1
xprop -id "$window" _NET_WM_STATE >"$RUN/logs/fullscreen-state.txt"
rg -q '_NET_WM_STATE_FULLSCREEN' "$RUN/logs/fullscreen-state.txt"
import -window root "$RUN/screenshots/fullscreen.png"
xdotool key --clearmodifiers Escape
sleep 1
kill -0 "$app_pid"
# The center of testsrc2 contains saturated colour. A black/opaque webview,
# missing libVLC, or a detached video window must fail this smoke check.
python3 -I - "$RUN/screenshots" <<'PY'
import sys
from pathlib import Path
from PIL import Image, ImageChops
root = Path(sys.argv[1])
im = Image.open(root / 'playing.png').convert('RGB')
w, h = im.size
box = (w//4, h//3, 3*w//4, 2*h//3)
pixels = list(im.crop(box).getdata())
colour = sum(max(p) - min(p) > 100 and max(p) > 150 for p in pixels)
assert colour > len(pixels) // 5, 'Embedded video is missing or obscured'
paused = Image.open(root / 'paused.png').convert('RGB').crop(box)
still = Image.open(root / 'paused-still.png').convert('RGB').crop(box)
assert ImageChops.difference(paused, still).getbbox() is None, 'Space did not pause the embedded video'
PY
echo 'Linux package rendered embedded video, paused with Space, and entered fullscreen.'
