#!/usr/bin/env python3
"""Launches a built ReVLC on a test video and drives it through a running
Cua Driver daemon: finds the window, reads its accessibility tree, checks the
video is on screen, pauses with the Pause button and resumes with Space.

    python3 scripts/qa/ci/cua_smoke.py --out RUN_DIR --media VIDEO [--no-video] -- APP [ARGS...]

--no-video skips the picture checks, for runners that can't show video
(hosted macOS has no GPU, so libVLC can't create its OpenGL view). Every check
runs even after one fails, so a run shows everything that works. Writes RUN_DIR/report.json and screenshots; exits 1 if
any check failed. Seed the app data (appdata.py) before running this.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

from PIL import Image, ImageChops

# Rows look like `[8] AXButton (Pause) [actions=[press]]` on macOS,
# `[2] Button "Minimize"` on Windows and `[7]  "Pause" [actions=[press]]` on
# Linux, where AT-SPI roles come through empty.
ROW = re.compile(r'^\s*- \[(\d+)\] ?(\S*)\s+(?:"([^"]*)"|\(([^)]*)\))(.*)$', re.M)


def cua(tool, args, shot=None):
    # One session for the whole run: element tokens don't survive into another.
    cmd = ["cua-driver", "call", tool, json.dumps({**args, "session": "revlc-smoke"})]
    if shot:
        cmd += ["--screenshot-out-file", str(shot)]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
    if r.returncode:
        raise RuntimeError(f"{tool} failed: {r.stderr.strip() or r.stdout.strip()}")
    return json.loads(r.stdout)


def buttons(tree, label):
    """Element rows whose role is a button and whose name is `label`."""
    return [int(i) for i, role, quoted, paren, rest in ROW.findall(tree)
            if (quoted or paren) == label and ("button" in role.lower() or "press" in rest)]


def centre(path):
    im = Image.open(path).convert("RGB")
    w, h = im.size
    return im.crop((w // 4, h // 3, 3 * w // 4, 2 * h // 3))


def colourful(path):
    """Share of centre pixels that are saturated: testsrc2 is, a black or
    covered video surface is not."""
    px = list(centre(path).getdata())
    return sum(max(p) - min(p) > 100 and max(p) > 150 for p in px) / len(px)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--media", required=True)
    ap.add_argument("--no-video", action="store_true", help="skip the picture checks")
    ap.add_argument("app", nargs="+")
    a = ap.parse_args()
    out = a.out
    (out / "screenshots").mkdir(parents=True, exist_ok=True)
    (out / "logs").mkdir(parents=True, exist_ok=True)
    checks = {}

    def check(name, fn):
        try:
            ok, detail = fn()
        except Exception as e:  # recorded, never fatal: report every result
            ok, detail = False, f"{type(e).__name__}: {e}"
        checks[name] = {"ok": bool(ok), "detail": detail}
        print(f"[{'PASS' if ok else 'FAIL'}] {name}: {detail}", flush=True)
        return ok

    env = dict(os.environ, REVLC_VLC_ARGS="--no-audio")
    log = open(out / "logs/revlc.log", "w")
    app = subprocess.Popen([*a.app, a.media], env=env, stdout=log, stderr=subprocess.STDOUT)
    win = {}

    def find_window():
        for _ in range(60):
            if app.poll() is not None:
                return False, f"app exited with {app.returncode}"
            ws = cua("list_windows", {"pid": app.pid}).get("windows", [])
            ws = [w for w in ws if w.get("bounds", {}).get("width", 0) > 300]
            if ws:
                win.update(ws[0])
                # Only for the screen recording: on Windows the runner's console
                # starts on top. Cua's own reads don't need the window in front.
                try:
                    cua("bring_to_front", {"pid": app.pid, "window_id": ws[0]["window_id"]})
                except RuntimeError as e:
                    print(f"bring_to_front: {e}", flush=True)
                return True, {k: ws[0].get(k) for k in ("window_id", "title", "bounds", "is_on_screen")}
            time.sleep(1)
        listing = cua("list_windows", {})
        (out / "logs/all-windows.json").write_text(json.dumps(listing, indent=2))
        return False, "no ReVLC window within 60 s (all windows in logs/all-windows.json)"

    state = {}

    def read(name):
        s = cua("get_window_state", {"pid": app.pid, "window_id": win["window_id"], "max_elements": 600},
                shot=out / f"screenshots/{name}.png")
        (out / f"logs/{name}-tree.md").write_text(s.get("tree_markdown", ""))
        state.update(s)
        return s

    if check("window", find_window):
        time.sleep(6)  # let playback start and the first frames render
        # The web page's tree can arrive a moment after the window's.
        for _ in range(5):
            if buttons(read("playing")["tree_markdown"], "Pause"):
                break
            time.sleep(2)
        check("ax_tree", lambda: (lambda s: (
            buttons(s["tree_markdown"], "Pause") and "Seek" in s["tree_markdown"],
            f"{s.get('total_element_count')} elements; Pause button rows {buttons(s['tree_markdown'], 'Pause')}; "
            f"degraded={s.get('degraded_reason')}"))(state))
        if not a.no_video:
            check("video_on_screen", lambda: (lambda f: (f > 0.2, f"{f:.0%} of the centre is saturated test pattern"))(
                colourful(out / "screenshots/playing.png")))

        def pause_by_button():
            rows = buttons(state["tree_markdown"], "Pause")
            if not rows:
                return False, "no Pause button to click"
            r = cua("click", {"element_token": f"{state['snapshot_id']}:{rows[0]}"})
            time.sleep(1.5)
            s = read("paused")
            time.sleep(1.5)
            cua("get_window_state", {"pid": app.pid, "window_id": win["window_id"], "include_accessibility_tree": False},
                shot=out / "screenshots/paused-still.png")
            frozen = ImageChops.difference(centre(out / "screenshots/paused.png"),
                                           centre(out / "screenshots/paused-still.png")).getbbox() is None
            flipped = bool(buttons(s["tree_markdown"], "Play"))
            if a.no_video:  # no picture to freeze
                return flipped, f"click effect={r.get('effect')}; button now Play={flipped}"
            return frozen and flipped, f"click effect={r.get('effect')}; button now Play={flipped}; frame frozen={frozen}"

        check("pause_with_button", pause_by_button)

        def resume_with_space():
            key = {"pid": app.pid, "window_id": win["window_id"], "key": "space"}
            try:
                r = cua("press_key", key)
            except RuntimeError as e:
                # Linux has no background key route; a CI desktop can take focus.
                if "background_unavailable" not in str(e):
                    raise
                r = cua("press_key", {**key, "delivery_mode": "foreground"})
            time.sleep(1.5)
            s = read("resumed")
            return bool(buttons(s["tree_markdown"], "Pause")), f"press_key effect={r.get('effect')}"

        check("resume_with_space", resume_with_space)
        check("still_running", lambda: (app.poll() is None, f"exit code {app.poll()}"))

    app.kill()
    app.wait()
    log.close()
    (out / "report.json").write_text(json.dumps({"platform": sys.platform, "checks": checks}, indent=2))
    failed = [k for k, v in checks.items() if not v["ok"]]
    print("All checks passed" if not failed else f"Failed: {', '.join(failed)}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
