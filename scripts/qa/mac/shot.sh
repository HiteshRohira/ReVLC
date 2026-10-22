#!/usr/bin/env bash
# Takes a QA screenshot, refusing if the screen is locked (a locked screen
# blanks the web view, which looks like a UI bug but isn't).
#
#   QA_RUN=~/revlc-qa/mac-<date> scripts/qa/mac/shot.sh 12-what-it-shows [--full]
#
# Captures ReVLC's main window when one is on screen, else the whole screen
# (--full always captures the whole screen). Saves $QA_RUN/screenshots/<name>.png.
set -euo pipefail
: "${QA_RUN:?set QA_RUN to the run folder}"
name="$1"
desktop="$QA_RUN/bin/desktop"
mkdir -p "$QA_RUN/screenshots" "$QA_RUN/logs" "$QA_RUN/bin"
[[ -x "$desktop" ]] || swiftc -O "$(dirname "$0")/desktop.swift" -o "$desktop"

if /usr/sbin/ioreg -n Root -d1 | grep -q '"CGSSessionScreenIsLocked"=Yes'; then
  echo "$(date -u +%FT%TZ) SCREEN LOCKED, refused $name" | tee -a "$QA_RUN/logs/screen-lock.log" >&2
  exit 1
fi

window=""
[[ "${2:-}" == --full ]] || window="$("$desktop" windowid)"
if [[ -n "$window" ]]; then
  screencapture -x -l "$window" "$QA_RUN/screenshots/$name.png"
else
  screencapture -x "$QA_RUN/screenshots/$name.png"
fi
echo "$QA_RUN/screenshots/$name.png"
