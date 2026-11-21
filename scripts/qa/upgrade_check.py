#!/usr/bin/env python3
"""Checks that an update keeps the user's data: run `before` with the
previous release installed, install the new one over it, then run `after`.

    python3 -I scripts/qa/upgrade_check.py before --exe OLD_EXE (--ffmpeg FFMPEG | --clip VIDEO) --work DIR
    (install the new release over the old one)
    python3 -I scripts/qa/upgrade_check.py after --exe NEW_EXE --work DIR

`before` writes settings a user might have changed (analytics off among
them, before the first launch), plays a short clip with the old app and
records what it saved. `after` plays the clip with the new app and checks
the settings and history survived, and that the history wasn't set aside
as damaged. Releases before 0.2.10 ship no FFmpeg, so for those pass a
ready-made clip with --clip. For a throwaway machine (Windows Sandbox, a VM): it uses the
real data dir, with no backup.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

SETTINGS = {
    "analytics": False,  # never let QA runs reach PostHog
    "autoUpdate": False,
    "resume": "never",
    "font": "Georgia",
    "fontSize": 12,
    "networkCaching": 5000,
    "shortcuts": {"play-pause": "Ctrl+Space"},
}


def data_dir():
    if sys.platform == "darwin":
        return Path.home() / "Library/Application Support/app.revlc"
    if os.name == "nt":
        return Path(os.environ["APPDATA"]) / "app.revlc"
    return Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share") / "app.revlc"


def play(exe, clip, work, name):
    """Plays 4 s of the clip and quits, as a user's short session."""
    env = dict(os.environ, REVLC_VLC_ARGS="--no-audio")
    with open(work / f"{name}.log", "w") as log:
        r = subprocess.run([exe, "--play-and-exit", "--run-time=4", str(clip)], env=env, stdout=log, stderr=subprocess.STDOUT, timeout=120)
    return r.returncode


def load(name):
    path = data_dir() / name
    return json.loads(path.read_text(encoding="utf-8")) if path.exists() else None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("step", choices=("before", "after"))
    ap.add_argument("--exe", required=True)
    ap.add_argument("--ffmpeg")
    ap.add_argument("--clip", type=Path, help="a short video to play, instead of making one with --ffmpeg")
    ap.add_argument("--work", required=True, type=Path)
    a = ap.parse_args()
    work = a.work.resolve()
    work.mkdir(parents=True, exist_ok=True)
    clip = work / "upgrade-clip.mp4"
    problems = []

    if a.step == "before":
        if data_dir().exists():
            sys.exit(f"{data_dir()} already exists; this is for a fresh machine")
        if a.clip:
            shutil.copyfile(a.clip, clip)
        elif a.ffmpeg:
            subprocess.run([a.ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24",
                            "-t", "30", "-c:v", "mpeg4", "-q:v", "3", str(clip)], check=True)
        else:
            sys.exit("before needs --clip or --ffmpeg")
        data_dir().mkdir(parents=True)
        (data_dir() / "settings.json").write_text(json.dumps(SETTINGS, indent=2), encoding="utf-8")
        code = play(a.exe, clip, work, "before")
        history = load("history.json") or []
        if code:
            problems.append(f"the old version exited with code {code}")
        if not any(clip.name in (e.get("uri") or "") for e in history):
            problems.append("the old version didn't save the clip in history")
        (work / "before.json").write_text(json.dumps({"settings": load("settings.json"), "history": history}, indent=2), encoding="utf-8")
    else:
        before = json.loads((work / "before.json").read_text(encoding="utf-8"))
        code = play(a.exe, clip, work, "after")
        if code:
            problems.append(f"the new version exited with code {code}")
        settings = load("settings.json") or {}
        for key, value in SETTINGS.items():
            if settings.get(key) != value:
                problems.append(f"setting {key} is {settings.get(key)!r}, was {value!r}")
        history = load("history.json") or []
        if not any(clip.name in (e.get("uri") or "") for e in history):
            problems.append(f"the clip is gone from history ({len(history)} entries; {len(before['history'])} before)")
        damaged = sorted(p.name for p in data_dir().glob("history-damaged*"))
        if damaged:
            problems.append(f"the history was set aside as damaged: {damaged}")

    for p in problems:
        print(f"FAIL {a.step}: {p}")
    if not problems:
        print(f"ok {a.step}: " + ("settings and history saved by the old version" if a.step == "before"
                                  else "settings and history survived the update"))
    sys.exit(1 if problems else 0)


main()
