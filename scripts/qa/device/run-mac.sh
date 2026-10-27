#!/usr/bin/env bash
# Runs the Cua suite (scripts/qa/ci/cua_suite.py) on this Mac against the
# build you're about to ship, with real video. Run it when you want to, for
# example before a release:
#
#   scripts/qa/device/run-mac.sh --dmg ReVLC_0.2.11_aarch64.dmg
#   scripts/qa/device/run-mac.sh --app path/to/ReVLC.app [--only playback,effects]
#
# The app is copied into the run folder, so an installed ReVLC isn't touched.
# Your ReVLC data is backed up before the first launch and restored at the
# end (scripts/qa/appdata.py), and every launch has analytics off and no
# sound. It takes over the screen at times (fullscreen, windows coming to
# the front), so leave the Mac unlocked and don't use it while it runs.
#
# Needs Cua Driver with Accessibility and Screen Recording, python3 with
# Pillow, and Quit ReVLC first. Results: RUN_DIR/report.md and report.json,
# screenshots, the accessibility tree of every failed check, and app logs.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
app="" dmg="" only=""
while (($#)); do
  case "$1" in
    --app) app="$2"; shift 2 ;;
    --dmg) dmg="$2"; shift 2 ;;
    --only) only="$2"; shift 2 ;;
    *) echo "Unknown option $1" >&2; exit 2 ;;
  esac
done
[[ -n "$app$dmg" ]] || { echo "Pass --dmg or --app" >&2; exit 2; }

run="$HOME/revlc-qa/device-mac-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$run"
echo "Run folder: $run"

# Preconditions: each one is a reason a run would fail for the wrong reason.
if pgrep -x revlc >/dev/null; then
  echo "ReVLC is running. Quit it first: the test build would hand its files to it." >&2
  exit 1
fi
if [[ "$(python3 -c 'import Quartz; print(int(Quartz.CGSessionCopyCurrentDictionary().get("CGSSessionScreenIsLocked", 0)))' 2>/dev/null || echo 0)" == 1 ]]; then
  echo "The screen is locked; a locked screen blanks the web view." >&2
  exit 1
fi
command -v cua-driver >/dev/null || { echo "Cua Driver isn't installed: https://cua.ai/driver" >&2; exit 1; }
cua-driver status >/dev/null 2>&1 || { echo "Cua Driver's daemon isn't running; run: cua-driver permissions grant" >&2; exit 1; }
python3 -c 'import PIL' 2>/dev/null || { echo "python3 needs Pillow: python3 -m pip install --user pillow" >&2; exit 1; }

# The build under test, copied in.
if [[ -n "$dmg" ]]; then
  mount="$(mktemp -d)"
  hdiutil attach -nobrowse -readonly -mountpoint "$mount" "$dmg" >/dev/null
  trap 'hdiutil detach "$mount" -quiet || true' EXIT
  cp -R "$mount/ReVLC.app" "$run/ReVLC.app"
  hdiutil detach "$mount" -quiet
  trap - EXIT
else
  cp -R "$app" "$run/ReVLC.app"
fi
# Launched directly by the suite, so Gatekeeper's first-launch check doesn't
# apply; that one stays a manual release check.
xattr -cr "$run/ReVLC.app"
defaults read "$run/ReVLC.app/Contents/Info.plist" CFBundleShortVersionString > "$run/version.txt"
echo "Testing ReVLC $(cat "$run/version.txt")"

# If the run is cut short, put your ReVLC data back.
cleanup() {
  if [[ -f "$run/backup/manifest.json" && ! -f "$run/logs/data-restore-verification.json" ]]; then
    pkill -9 -x revlc 2>/dev/null || true
    sleep 1
    python3 -I "$ROOT/scripts/qa/appdata.py" restore "$run"
  fi
}
trap cleanup EXIT

args=(--out "$run" --ffmpeg "$run/ReVLC.app/Contents/MacOS/ffmpeg" --bundle "$run/ReVLC.app")
[[ -n "$only" ]] && args+=(--only "$only")
status=0
python3 "$ROOT/scripts/qa/ci/cua_suite.py" "${args[@]}" -- "$run/ReVLC.app/Contents/MacOS/revlc" | tee "$run/suite.log" || status=$?
echo
echo "Report: $run/report.md"
exit "$status"
