#!/usr/bin/env python3
"""Backs up, seeds and restores ReVLC's data dirs around a QA run.

Debug and release builds share the real data dirs, so a QA run must start
from a backup and end with an exact restore. See docs/audit-playbook.md.

    python3 -I scripts/qa/appdata.py backup  RUN_DIR
    python3 -I scripts/qa/appdata.py seed    RUN_DIR   # clean dirs, analytics off
    python3 -I scripts/qa/appdata.py restore RUN_DIR   # also verifies
    python3 -I scripts/qa/appdata.py verify  RUN_DIR

macOS: ~/Library/Application Support/app.revlc, ~/Library/Caches/app.revlc
and ~/Library/WebKit/app.revlc (the web view's storage: onboarding and other
UI state).
Windows: %APPDATA%\\app.revlc and %LOCALAPPDATA%\\app.revlc.
Linux: $XDG_DATA_HOME/app.revlc, $XDG_CONFIG_HOME/app.revlc and
$XDG_CACHE_HOME/app.revlc (using the XDG defaults when unset).
The app must not be running. Backups go to RUN_DIR/backup/, never /tmp
(a reboot clears it). Restore moves the QA data to RUN_DIR/logs/ as evidence,
copies the originals back and checks every file's hash, mode and mtime, and
where every symlink points.
"""
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path


def data_dirs():
    if sys.platform == "darwin":
        lib = Path.home() / "Library"
        return {"support": lib / "Application Support/app.revlc", "caches": lib / "Caches/app.revlc", "webkit": lib / "WebKit/app.revlc"}
    if os.name == "nt":
        return {"roaming": Path(os.environ["APPDATA"]) / "app.revlc", "local": Path(os.environ["LOCALAPPDATA"]) / "app.revlc"}
    if sys.platform.startswith("linux"):
        def xdg(name, fallback):
            value = Path(os.environ.get(name, ""))
            return value if value.is_absolute() else Path.home() / fallback
        config = xdg("XDG_CONFIG_HOME", ".config")
        cache = xdg("XDG_CACHE_HOME", ".cache")
        data = xdg("XDG_DATA_HOME", ".local/share")
        # seed() writes settings to the first directory: Tauri's app_data_dir.
        return {"data": data / "app.revlc", "config": config / "app.revlc", "caches": cache / "app.revlc"}
    sys.exit("Unsupported platform")


def running():
    if os.name == "nt":
        out = subprocess.run(["tasklist", "/FI", "IMAGENAME eq revlc.exe"], capture_output=True, text=True).stdout
        return "revlc.exe" in out.lower()
    return subprocess.run(["pgrep", "-x", "revlc"], capture_output=True).returncode == 0


def manifest(root):
    files = {}
    if root.exists():
        for f in sorted(root.rglob("*")):
            if f.is_symlink():
                files[f.relative_to(root).as_posix()] = {"symlink": os.readlink(f)}
            elif f.is_file():
                s = f.stat()
                files[f.relative_to(root).as_posix()] = {
                    "sha256": hashlib.sha256(f.read_bytes()).hexdigest(),
                    "mode": s.st_mode & 0o7777,
                    "mtimeNs": s.st_mtime_ns,
                }
    return files


def verify(run):
    expected = json.loads((run / "backup/manifest.json").read_text())
    ok = True
    for label, path in data_dirs().items():
        if label not in expected:
            print(f"{label}: not in this (older) backup; left as it is")
            continue
        want, have = expected[label]["files"], manifest(path)
        diffs = [rel for rel in sorted(set(want) | set(have)) if want.get(rel) != have.get(rel)]
        for rel in diffs:
            print(f"{label}/{rel}: expected {want.get(rel)}, found {have.get(rel)}")
        if expected[label]["existed"] != path.exists():
            print(f"{label}: existed {expected[label]['existed']}, now {path.exists()}")
            diffs.append(".")
        print(f"{label}: {len(have)} files, {len(diffs)} differences")
        ok = ok and not diffs
    (run / "logs").mkdir(parents=True, exist_ok=True)
    (run / "logs/data-restore-verification.json").write_text(json.dumps({"match": ok, "at": time.time()}, indent=2))
    print("restore verified" if ok else "RESTORE MISMATCH")
    return ok


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    cmd, run = sys.argv[1], Path(sys.argv[2]).expanduser().resolve()
    backup = run / "backup"
    if cmd != "verify" and running():
        sys.exit("ReVLC is running; kill it first")

    if cmd == "backup":
        if (backup / "manifest.json").exists():
            sys.exit(f"{backup} already holds a backup; not overwriting it")
        backup.mkdir(parents=True, exist_ok=True)
        info = {}
        for label, path in data_dirs().items():
            info[label] = {"path": str(path), "existed": path.exists(), "files": manifest(path)}
            if path.exists():
                shutil.copytree(path, backup / label, symlinks=True)
            print(f"{label}: {len(info[label]['files'])} files from {path}")
        (backup / "manifest.json").write_text(json.dumps(info, indent=2))

    elif cmd == "seed":
        if not (backup / "manifest.json").exists():
            sys.exit("back up first")
        # Seeding empties every data dir, so each must be in the backup.
        if missing := set(data_dirs()) - set(json.loads((backup / "manifest.json").read_text())):
            sys.exit(f"the backup doesn't cover {', '.join(sorted(missing))}; restore it and start a new run")
        for path in data_dirs().values():
            if path.exists():
                shutil.rmtree(path)
        support = next(iter(data_dirs().values()))
        support.mkdir(parents=True)
        settings = {
            "analytics": False,  # never let QA runs reach PostHog
            "autoUpdate": False,
            "audio": False,
            "snapshotPath": str(run / "screenshots"),
            "resume": "ask",
        }
        (support / "settings.json").write_text(json.dumps(settings, indent=2))
        print(f"seeded {support / 'settings.json'}")

    elif cmd == "restore":
        if not (backup / "manifest.json").exists():
            sys.exit("no backup to restore")
        info = json.loads((backup / "manifest.json").read_text())
        stamp = time.strftime("%Y%m%d-%H%M%S")
        for label, path in data_dirs().items():
            if label not in info:
                continue
            if path.exists():
                kept = run / "logs" / f"qa-appdata-{label}-{stamp}"
                kept.parent.mkdir(parents=True, exist_ok=True)
                shutil.move(str(path), str(kept))
            if info[label]["existed"]:
                shutil.copytree(backup / label, path, symlinks=True)
        sys.exit(0 if verify(run) else 1)

    elif cmd == "verify":
        sys.exit(0 if verify(run) else 1)
    else:
        sys.exit(__doc__)


main()
