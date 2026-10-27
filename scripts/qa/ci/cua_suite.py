#!/usr/bin/env python3
"""Real-app checks for ReVLC, driven through a running Cua Driver daemon.

    python3 scripts/qa/ci/cua_suite.py --out RUN_DIR --ffmpeg FFMPEG [--no-video]
        [--only NAME,...] [--bundle ReVLC.app] -- APP [ARGS...]

Each scenario launches the built app on generated media, drives its real
window through Cua (the accessibility tree, clicks and key presses) and checks
what it shows. Every scenario starts from freshly seeded app data with
analytics off (scripts/qa/appdata.py), so the data dirs are backed up first
and restored at the end. Every check runs even after another fails.

--no-video skips the picture checks, for runners that can't show video
(hosted macOS has no GPU, so libVLC can't create its OpenGL view).

Writes RUN_DIR/report.json, RUN_DIR/report.md (also appended to the GitHub
step summary), screenshots and app logs. Exits 1 if any check failed.
Checks marked as known bugs don't fail the run; if one starts passing, the
report says so, so the mark can be removed.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
import traceback
from pathlib import Path

from PIL import Image, ImageChops

ROOT = Path(__file__).resolve().parents[3]
# A fresh Cua session per run: a long-running daemon can end an old one.
SESSION = f"revlc-suite-{os.getpid()}"
MAC = sys.platform == "darwin"
WIN = os.name == "nt"
LINUX = sys.platform.startswith("linux")
PLATFORM = "macos" if MAC else "windows" if WIN else "linux"
TIME = re.compile(r"^\s*(?:(\d+):)?(\d+):(\d\d)\s*/")


class Fail(Exception):
    pass


class Skip(Exception):
    pass


DAEMON_GONE = ("daemon is not running", "closed connection without response")


def cua(tool, args, shot=None, timeout=60, retry=True):
    cmd = ["cua-driver", "call", tool, json.dumps({**args, "session": SESSION})]
    if shot:
        cmd += ["--screenshot-out-file", str(shot)]
    # UTF-8 explicitly: Windows would decode with its ANSI code page.
    r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
    if r.returncode:
        message = (r.stderr or r.stdout or "").strip()
        if retry and "session has ended" in message:
            subprocess.run(["cua-driver", "call", "start_session", json.dumps({"session": SESSION})], capture_output=True)
            return cua(tool, args, shot, timeout, retry=False)
        restart = os.environ.get("CUA_DAEMON_RESTART")
        if retry and restart and any(m in message for m in DAEMON_GONE):
            # The daemon can crash on a hosted runner; Cua's own CI restarts
            # it with a watchdog. Restart it once and retry the call.
            print(f"  (Cua daemon gone during {tool}; restarting it)", flush=True)
            subprocess.run(restart, shell=True)
            wait_for(lambda: subprocess.run(["cua-driver", "status"], capture_output=True).returncode == 0, "the Cua daemon", 30)
            return cua(tool, args, shot, timeout, retry=False)
        raise RuntimeError(f"{tool}: {message[:600]}")
    return json.loads(r.stdout) if r.stdout.strip() else {}


def ensure(ok, message):
    if not ok:
        raise Fail(message)
    return ok


def wait_for(fn, what, timeout=10.0, interval=0.5):
    """Polls fn until it returns something truthy; Fail if it never does."""
    end = time.monotonic() + timeout
    while True:
        try:
            value = fn()
        except (Fail, RuntimeError):
            value = None
        if value:
            return value
        if time.monotonic() > end:
            raise Fail(f"timed out after {timeout:g} s waiting for {what}")
        time.sleep(interval)


def centre(path):
    im = Image.open(path).convert("RGB")
    w, h = im.size
    return im.crop((w // 4, h // 3, 3 * w // 4, 2 * h // 3))


def colourful(path):
    """Share of centre pixels that are saturated: testsrc2 is, a black or
    covered video surface is not."""
    px = list(centre(path).getdata())
    return sum(max(p) - min(p) > 100 and max(p) > 150 for p in px) / len(px)


def is_button(e):
    role = (e.get("role") or "").lower()
    return "button" in role or (not role and any("press" in a.lower() for a in e.get("actions", [])))


def parse_time(label):
    m = TIME.match(label or "")
    if not m:
        return None
    h, mnt, s = m.groups()
    return ((int(h or 0) * 60 + int(mnt)) * 60 + int(s)) * 1000


class App:
    """One launched ReVLC process and its main window."""

    def __init__(self, ctx, name, args=(), env=None, pid=None):
        self.ctx = ctx
        self.name = name
        self.log = open(ctx.out / "logs" / f"{name}.log", "w")
        if pid:  # started by someone else, such as Launch Services
            self.proc, self.pid = None, pid
        else:
            e = dict(os.environ, REVLC_VLC_ARGS="--no-audio", **(env or {}))
            self.proc = subprocess.Popen([*ctx.app, *map(str, args)], env=e, stdout=self.log, stderr=subprocess.STDOUT)
            self.pid = self.proc.pid
        self.window_id = None
        self.frame = None
        self.snapshot = None
        self.elements = []

    # Windows and state

    def windows(self):
        ws = cua("list_windows", {"pid": self.pid}).get("windows", [])
        # Layer 0 only: macOS also lists the app's menu-bar strip as a window.
        return [w for w in ws if w.get("bounds", {}).get("width", 0) > 300 and w.get("bounds", {}).get("height", 0) > 200
                and w.get("layer", 0) == 0]

    def wait_window(self, timeout=40, title=None):
        def find():
            if not self.alive():
                raise Fail("app exited before its window appeared")
            ws = self.windows()
            if title:
                ws = [w for w in ws if title in (w.get("title") or "")]
            return ws
        ws = wait_for(find, "the ReVLC window", timeout)
        w = max(ws, key=lambda w: w["bounds"]["width"] * w["bounds"]["height"])
        self.window_id = w["window_id"]
        self.frame = w["bounds"]
        try:
            # On Windows the runner's console starts on top; recordings and
            # foreground key delivery want ReVLC in front.
            cua("bring_to_front", {"pid": self.pid, "window_id": self.window_id})
        except RuntimeError:
            pass
        return w

    def title(self):
        ws = [w for w in self.windows() if w["window_id"] == self.window_id]
        return ws[0].get("title", "") if ws else ""

    def bounds(self):
        ws = [w for w in self.windows() if w["window_id"] == self.window_id]
        return ws[0]["bounds"] if ws else None

    def read(self, window_id=None):
        s = cua("get_window_state", {
            "pid": self.pid, "window_id": window_id or self.window_id, "tree_format": "elements",
            "include_screenshot": False, "max_elements": 1500,
        })
        self.snapshot = s.get("snapshot_id")
        self.elements = s.get("elements", [])
        return self.elements

    def text(self, window_id=None):
        """Every label and value in the window, one per line."""
        out = []
        for e in self.read(window_id):
            for k in ("label", "value", "title", "description"):
                v = e.get(k)
                if isinstance(v, str) and v.strip():
                    out.append(v.strip())
        return "\n".join(out)

    def find(self, label=None, contains=None, role=None, button=False, window_id=None, fresh=True):
        els = self.read(window_id) if fresh else self.elements
        found = []
        for e in els:
            lab = e.get("label") or ""
            if label is not None and lab != label:
                continue
            if contains is not None and contains.lower() not in lab.lower():
                continue
            if role is not None and role.lower() not in (e.get("role") or "").lower():
                continue
            if button and not is_button(e):
                continue
            found.append(e)
        return found

    def has(self, label, **kw):
        return bool(self.find(label, **kw))

    def wait(self, label, timeout=10, **kw):
        return wait_for(lambda: self.find(label, **kw), f'"{label}"', timeout)

    def wait_gone(self, label, timeout=10, **kw):
        return wait_for(lambda: not self.find(label, **kw), f'"{label}" to go away', timeout)

    def wait_text(self, needle, timeout=10, window_id=None):
        return wait_for(lambda: needle.lower() in self.text(window_id).lower(), f'text "{needle}"', timeout)

    # Actions

    def press(self, label, window_id=None, any_role=False, **kw):
        for attempt in (1, 2):
            els = self.find(label, window_id=window_id, button=not any_role, **kw)
            ensure(els, f'no "{label}" {"control" if any_role else "button"}')
            try:
                return self.click_el(els[0], window_id)
            except RuntimeError as e:
                # A restarted Cua daemon forgets earlier reads; read again.
                if attempt == 2 or not any(c in str(e) for c in ("stale_element_token", "window_never_read")):
                    raise

    def click_el(self, e, window_id=None):
        """Clicks an element through accessibility, or at its centre when
        it has no action (WebKitGTK menu items have none)."""
        if e.get("actions"):
            return cua("click", {"element_token": e["element_token"]})
        f = e.get("frame")
        ensure(f, f'"{e.get("label")}" has no action and no position')
        return self.pixel_click(f["x"] + f["w"] / 2, f["y"] + f["h"] / 2, window_id)

    def pixel_click(self, sx, sy, window_id=None, count=1):
        """Clicks at screen point (sx, sy). Cua takes window-local pixels of
        its latest screenshot of the window, so read one first."""
        wid = window_id or self.window_id
        st = cua("get_window_state", {"pid": self.pid, "window_id": wid, "include_accessibility_tree": False})
        wb = st.get("window_bounds") or self.bounds()
        scale = (st.get("screenshot_width") or wb["width"]) / wb["width"]
        args = {"pid": self.pid, "window_id": wid, "x": round((sx - wb["x"]) * scale), "y": round((sy - wb["y"]) * scale)}
        tool = "double_click" if count == 2 else "click"
        try:
            return cua(tool, args)
        except RuntimeError as err:
            if "background_unavailable" not in str(err) and "window_never_read" not in str(err):
                raise
            return cua(tool, {**args, "delivery_mode": "foreground"})

    def switches(self, label, window_id=None):
        """Elements that can be the switch named `label`: switch-like roles
        first, and never a tab of the same name."""
        els = [e for e in self.read(window_id) if e.get("label") == label and e.get("actions")
               and not any(r in (e.get("role") or "").lower() for r in ("text", "tab", "radio"))]
        return sorted(els, key=lambda e: not any(r in (e.get("role") or "").lower() for r in ("check", "switch", "toggle")))

    def switch_on(self, label, window_id=None):
        """Whether the switch named `label` is on, from its checked state."""
        els = self.switches(label, window_id)
        ensure(els, f'no "{label}" switch')
        e = els[0]
        return bool(e.get("selected")) or str(e.get("value")).lower() in ("1", "true", "on", "checked")

    def gear_item(self, prefix):
        """Opens the gear menu and returns the label of its item that starts
        with `prefix`, such as "Playback speed 1.5×"; leaves the menu open."""
        if not self.find(contains="Playback speed"):
            self.press("Settings")
        items = wait_for(lambda: [e for e in self.read() if (e.get("label") or "").startswith(prefix)], f'"{prefix}" in the gear menu', 5)
        return items[0]["label"]

    def close_menus(self):
        if self.find(contains="Playback speed"):
            self.key("escape")
            wait_for(lambda: not self.find(contains="Playback speed"), "the gear menu to close", 5)

    def type_into(self, element, text):
        """Types into a text field: focus it, then type."""
        cua("click", {"element_token": element["element_token"]})
        try:
            cua("type_text", {"pid": self.pid, "element_token": element["element_token"], "text": text})
        except RuntimeError:
            cua("type_text", {"pid": self.pid, "window_id": self.window_id, "text": text, "delivery_mode": "foreground"})

    def button_after(self, label, window_id=None):
        """The first button after the element named `label`: the control in
        a row whose name isn't linked to it (the shortcut list)."""
        els = self.read(window_id)
        idx = [i for i, e in enumerate(els) if e.get("label") == label and not is_button(e)]
        ensure(idx, f'no "{label}"')
        for e in els[idx[0] + 1:]:
            if is_button(e):
                return e
        raise Fail(f'no button after "{label}"')

    def click_switch(self, label, window_id):
        """Flips the switch named `label`. On Linux a switch reports no
        position, so click where it sits: the right end of its row."""
        els = self.switches(label, window_id)
        ensure(els, f'no "{label}" switch')
        f = els[0].get("frame")
        if f and f.get("w") and f.get("h"):
            try:
                return cua("click", {"element_token": els[0]["element_token"]})
            except RuntimeError as e:
                if "element_bounds_unavailable" not in str(e):
                    raise
        by_index = {e["element_index"]: e for e in self.elements}
        parent = by_index.get(els[0].get("parent_index"))
        while parent and not (parent.get("frame") or {}).get("w"):
            parent = by_index.get(parent.get("parent_index"))
        ensure(parent, f'"{label}" and its parents have no position')
        f = parent["frame"]
        return self.pixel_click(f["x"] + f["w"] - 19, f["y"] + f["h"] / 2, window_id)

    def key(self, key, mods=(), window_id=None):
        args = {"pid": self.pid, "window_id": window_id or self.window_id}
        tool, args = ("hotkey", {**args, "keys": [*mods, key]}) if mods else ("press_key", {**args, "key": key})
        try:
            return cua(tool, args)
        except RuntimeError as e:
            # Linux has no background key route; a CI desktop can take focus.
            if "background_unavailable" not in str(e) and "foreground" not in str(e):
                raise
            return cua(tool, {**args, "delivery_mode": "foreground"})

    def shortcut(self, key):
        """The app's primary-modifier shortcut: ⌘ on macOS (via the menu
        bar, which owns ⌘ shortcuts there), Ctrl elsewhere."""
        if MAC:
            return cua("hotkey", {"pid": self.pid, "window_id": self.window_id, "keys": ["cmd", key], "delivery_mode": "foreground"})
        # Background Ctrl chords don't reach WebView2; send them as real input.
        return cua("hotkey", {"pid": self.pid, "window_id": self.window_id, "keys": ["ctrl", key], "delivery_mode": "foreground"})

    def menu(self, top, item):
        """Runs a menu item the way a user does: the menu bar on macOS, the
        title-bar menus elsewhere (which only show on Home)."""
        if MAC:
            app_menu = {"App": "ReVLC"}.get(top, top)
            cua("invoke_menu", {"pid": self.pid, "path": [app_menu, item]})
            return
        self.press({"App": "Tools"}.get(top, top))
        # Items carry their shortcut in their name, e.g. "Settings…Ctrl+,".
        els = wait_for(lambda: [e for e in self.read() if (e.get("label") or "").startswith(item)], f'menu item "{item}"', 5)
        self.click_el(els[0])

    def command(self, mac_menu, key):
        """Runs a command: from the macOS menu bar, or its Ctrl shortcut."""
        if MAC:
            cua("invoke_menu", {"pid": self.pid, "path": list(mac_menu)})
        else:
            self.shortcut(key)

    def settings_window(self):
        """Opens Settings (its own window) and returns that window's id:
        Home's Settings button, or the player's gear menu (More settings…)."""
        if self.find("Settings", button=True) and self.find("Home"):
            self.press("Settings")
        else:
            self.press("Settings")
            items = wait_for(lambda: self.find(contains="More settings"), "More settings… in the gear menu", 5)
            self.click_el(items[0])
        w = wait_for(lambda: [w for w in self.windows() if w["window_id"] != self.window_id], "the Settings window", 10)
        return w[0]["window_id"]

    def double_click_centre(self):
        b = self.bounds() or self.frame
        x, y = b["x"] + b["width"] / 2, b["y"] + b["height"] / 3
        # None of the web views take synthetic background double-clicks (on
        # macOS the video is a native view under the page); send real ones.
        st = cua("get_window_state", {"pid": self.pid, "window_id": self.window_id, "include_accessibility_tree": False})
        wb = st.get("window_bounds") or b
        scale = (st.get("screenshot_width") or wb["width"]) / wb["width"]
        cua("double_click", {"pid": self.pid, "window_id": self.window_id, "x": round((x - wb["x"]) * scale),
                             "y": round((y - wb["y"]) * scale), "delivery_mode": "foreground"})

    def shot(self, name):
        path = self.ctx.out / "screenshots" / f"{name}.png"
        cua("get_window_state", {"pid": self.pid, "window_id": self.window_id, "include_accessibility_tree": False}, shot=path)
        return path

    # Player state

    def time_ms(self):
        for e in self.read():
            t = parse_time(e.get("label"))
            if t is not None:
                return t
        raise Fail("no time display")

    def playing(self):
        return self.has("Pause", button=True)

    def wait_playing(self, timeout=20):
        wait_for(self.playing, "playback to start (a Pause button)", timeout)

    def slider(self, label):
        els = self.find(label)
        ensure(els, f'no "{label}" slider')
        v = els[0].get("value")
        try:
            return float(v)
        except (TypeError, ValueError):
            raise Fail(f'"{label}" has no numeric value ({v!r})')

    def finish_onboarding(self):
        """A fresh profile shows the welcome screen until media plays or
        Get started is pressed. Wait for the page to load, then pass it."""
        # Labels only the page has: Windows' own title bar has buttons too.
        page = ("Get started", "Back to home", "Home", "Open…", "Videos")
        wait_for(lambda: [e for e in self.read() if e.get("label") in page], "the page to load", 20)
        if self.find("Get started", button=True, fresh=False):
            try:
                self.press("Get started")
            except Fail:
                pass  # media started playing, which finishes onboarding too
            self.wait_gone("Get started")

    # Lifecycle

    def alive(self):
        if self.proc:
            return self.proc.poll() is None
        try:
            os.kill(self.pid, 0)
            return True
        except OSError:
            return False

    def wait_exit(self, timeout):
        end = time.monotonic() + timeout
        while self.alive():
            if time.monotonic() > end:
                return False
            time.sleep(0.3)
        return True

    def close(self, timeout=20):
        """Quits the way a user does, so ReVLC saves its state on the way out:
        ReVLC › Quit ReVLC on macOS, the window's close button elsewhere."""
        if MAC:
            cua("invoke_menu", {"pid": self.pid, "path": ["ReVLC", "Quit ReVLC"]})
        else:
            self.press("Close")
        ensure(self.wait_exit(timeout), f"app didn't quit within {timeout} s")

    def kill(self):
        if self.alive():
            if WIN:
                # The whole tree: WebView2's helpers otherwise keep the
                # profile's files locked.
                subprocess.run(["taskkill", "/F", "/T", "/PID", str(self.pid)], capture_output=True)
            elif self.proc:
                self.proc.kill()
            else:
                os.kill(self.pid, 9)
        if self.proc:
            self.proc.wait()
        self.log.close()


class Context:
    def __init__(self, a):
        self.out = a.out
        self.app = a.app
        self.bundle = a.bundle.resolve() if a.bundle else None
        self.ffmpeg = a.ffmpeg
        self.ci = bool(os.environ.get("CI"))
        self.no_video = a.no_video
        self.media = self.out / "media"
        self.results = []
        self.scenario = None
        self.apps = []
        for d in ("screenshots", "logs", "media"):
            (self.out / d).mkdir(parents=True, exist_ok=True)

    def appdata(self, cmd):
        for attempt in range(10):
            r = subprocess.run([sys.executable, "-I", str(ROOT / "scripts/qa/appdata.py"), cmd, str(self.out)], capture_output=True, text=True)
            if not r.returncode:
                return
            # On Windows, WebView2 helpers can hold the profile for a moment.
            if not WIN or "PermissionError" not in r.stderr:
                break
            subprocess.run(["taskkill", "/F", "/IM", "msedgewebview2.exe"], capture_output=True)
            time.sleep(1)
        raise RuntimeError(f"appdata.py {cmd}: {r.stdout}{r.stderr}")

    def data_dir(self):
        """Where ReVLC keeps history.json: Tauri's app data dir."""
        if MAC:
            return Path.home() / "Library/Application Support/app.revlc"
        if WIN:
            return Path(os.environ["APPDATA"]) / "app.revlc"
        base = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")
        return base / "app.revlc"

    def hand_off(self, path):
        """Opens `path` while ReVLC runs, as the OS does: Launch Services
        sends macOS's running app an open event; elsewhere a second launch
        passes its file to the first and exits."""
        if MAC:
            ensure(self.bundle, "--bundle is needed on macOS")
            subprocess.run(["open", "-a", str(self.bundle), str(path)], check=True)
            return
        other = self.launch(path, name=f"{self.scenario}-hand-off-{len(self.apps)}", wait=False)
        ensure(other.wait_exit(20), "the second launch kept running instead of handing over")

    def launch(self, *args, name=None, env=None, wait=True):
        app = App(self, name or f"{self.scenario}-{len(self.apps) + 1}", args, env)
        self.apps.append(app)
        if wait:
            app.wait_window()
        return app

    def check(self, name, fn, known_bug=None, app=None):
        """Runs one check and records it. `known_bug` marks an open bug: a
        failure is expected, and a pass is reported so the mark can go."""
        start = time.monotonic()
        try:
            detail = fn()
            status = "xpass" if known_bug else "pass"
        except Skip as e:
            status, detail = "skip", str(e)
        except Fail as e:
            status, detail = ("xfail" if known_bug else "fail"), str(e)
        except Exception as e:  # a broken check is a failure, with its trace
            status, detail = ("xfail" if known_bug else "fail"), f"{type(e).__name__}: {e}\n{traceback.format_exc(limit=3)}"
        if status in ("fail", "xfail") and app and app.window_id:
            try:
                base = f"{self.scenario}--{re.sub(r'[^A-Za-z0-9]+', '-', name)}--failed"
                app.shot(base)
                (self.out / "logs" / f"{base}.json").write_text(json.dumps(app.read(), indent=1, ensure_ascii=False), encoding="utf-8")
            except Exception:
                pass
        r = {"scenario": self.scenario, "check": name, "status": status, "detail": detail if isinstance(detail, str) else json.dumps(detail),
             "known_bug": known_bug, "seconds": round(time.monotonic() - start, 1)}
        self.results.append(r)
        print(f"  [{status.upper():5}] {name}: {r['detail'].splitlines()[0] if r['detail'] else ''}", flush=True)
        return status in ("pass", "xpass")


# Fixtures

def make_media(ctx):
    m = ctx.media

    def ff(out, *args):
        if not out.exists():
            out.parent.mkdir(parents=True, exist_ok=True)
            subprocess.run([ctx.ffmpeg, "-hide_banner", "-loglevel", "error", "-y", *args, "-c:v", "mpeg4", "-q:v", "3", str(out)], check=True)

    ff(m / "playback.mp4", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24", "-t", "60")
    ff(m / "second.mp4", "-f", "lavfi", "-i", "smptebars=size=640x360:rate=24", "-t", "40")
    ff(m / "subtitled.mp4", "-f", "lavfi", "-i", "color=c=0x204060:size=640x360:rate=24", "-t", "60")
    (m / "subtitled.srt").write_text("1\n00:00:00,000 --> 00:01:00,000\nSUBTITLE CHECK\n", encoding="utf-8")
    (m / "zero.mp4").write_bytes(b"")
    (m / "corrupt.mp4").write_bytes(b"\x00\x00\x00\x18ftypmp42" + os.urandom(200_000))
    for name in ("a.mp4", "b.mp4"):
        (m / "folder").mkdir(exist_ok=True)
        shutil.copyfile(m / "second.mp4", m / "folder" / name)
    # Over 300 characters with non-ASCII names: Windows' old 260-character
    # limit and Unicode handling both matter here.
    deep = m.joinpath(*["Ünïcødé 視頻 テスト フォルダ — long path segment"] * 6)
    deep.mkdir(parents=True, exist_ok=True)
    long_file = deep / "長いファイル名 — playback.mp4"
    if not long_file.exists():
        shutil.copyfile(m / "playback.mp4", long_file)
    ctx.long_file = long_file
    def ff_raw(out, *args):
        if not out.exists():
            out.parent.mkdir(parents=True, exist_ok=True)
            subprocess.run([ctx.ffmpeg, "-hide_banner", "-loglevel", "error", "-y", *map(str, args), str(out)], check=True)

    # Three named chapters.
    meta = m / "chapters.txt"
    meta.write_text(";FFMETADATA1\n" + "".join(
        f"[CHAPTER]\nTIMEBASE=1/1000\nSTART={a * 1000}\nEND={(a + 20) * 1000}\ntitle={n}\n"
        for a, n in ((0, "Opening"), (20, "Middle part"), (40, "Closing"))), encoding="utf-8")
    ff_raw(m / "chapters.mp4", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24", "-i", meta, "-map", "0", "-map_metadata", "1",
           "-t", "60", "-c:v", "mpeg4", "-q:v", "3")
    # Two audio tracks and two subtitle tracks, with languages.
    for lang, text in (("eng", "ENGLISH LINE"), ("fre", "LIGNE FRANCAISE")):
        (m / f"tracks-{lang}.srt").write_text(f"1\n00:00:00,000 --> 00:01:00,000\n{text}\n", encoding="utf-8")
    ff_raw(m / "tracks.mkv", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24", "-f", "lavfi", "-i", "sine=frequency=440",
           "-f", "lavfi", "-i", "sine=frequency=880", "-i", m / "tracks-eng.srt", "-i", m / "tracks-fre.srt",
           "-map", "0", "-map", "1", "-map", "2", "-map", "3", "-map", "4", "-t", "60", "-c:v", "mpeg4", "-q:v", "3", "-c:a", "aac",
           "-c:s", "srt", "-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "language=fre",
           "-metadata:s:s:0", "language=eng", "-metadata:s:s:1", "language=fre")
    # An HLS stream, served from a local web server while the suite runs.
    ff_raw(m / "hls" / "stream.m3u8", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24", "-t", "30", "-c:v", "mpeg4", "-q:v", "3",
           "-f", "hls", "-hls_time", "2", "-hls_list_size", "0", "-hls_segment_type", "mpegts")
    # A playlist file of two items.
    (m / "two.m3u").write_text("#EXTM3U\n" + "".join(f"{m / n}\n" for n in ("playback.mp4", "second.mp4")), encoding="utf-8")
    # A libVLC that can't load: the files exist, so ReVLC tries them, and fails.
    broken = m / "broken-vlc"
    lib = broken if WIN else broken / "lib"
    lib.mkdir(parents=True, exist_ok=True)
    names = ("libvlccore.dll", "libvlc.dll") if WIN else ("libvlccore.dylib", "libvlc.dylib") if MAC else ("libvlccore.so.9", "libvlc.so.5")
    for n in names:
        (lib / n).write_bytes(b"not a library")
    ctx.broken_vlc = broken


# Scenarios. Each launches its own app(s) on freshly seeded data.

SCENARIOS = []


def scenario(fn):
    SCENARIOS.append(fn)
    return fn


@scenario
def onboarding(ctx):
    app = ctx.launch()

    def welcome():
        if not ctx.ci and wait_for(lambda: app.find("Get started", button=True) or app.find("Home"), "the first screen", 15) and not app.find("Get started"):
            # The web view keeps "onboarded" outside the data dirs on macOS,
            # so a machine that has run ReVLC before skips it.
            raise Skip("this machine has already been onboarded")
        app.wait("Get started", button=True, timeout=15)
        return "shown"

    if not ctx.check("welcome shown", welcome, app=app):
        return
    app.shot("onboarding")

    def fits():
        b = app.bounds()
        cua("set_window_frame", {"pid": app.pid, "window_id": app.window_id, "x": b["x"], "y": b["y"], "width": 720, "height": 450})
        time.sleep(1)
        b = app.bounds()
        ensure(abs(b["width"] - 720) < 40 and abs(b["height"] - 450) < 40, f"couldn't resize the window: it's {b['width']:g}×{b['height']:g}")
        app.shot("onboarding-720x450")
        bottom = b["y"] + b["height"]
        outside = [e.get("label") for e in app.read() if e.get("frame") and e.get("label")
                   and e["frame"]["y"] + e["frame"]["h"] > bottom + 1 and e["frame"]["h"] > 0]
        ensure(not outside, f"cut off at the bottom of a 720×450 window: {outside}")
        return "everything fits"

    ctx.check("fits a 720×450 window", fits, app=app,
              known_bug=None if LINUX else "W7: onboarding text runs below a minimum-size window")
    ctx.check("Get started", lambda: (app.press("Get started"), app.wait_gone("Get started"), app.wait_text("Home"))[-1] and "Home shown", app=app)


@scenario
def playback(ctx):
    app = ctx.launch(ctx.media / "playback.mp4")
    app.finish_onboarding()
    if not ctx.check("starts playing", lambda: app.wait_playing() or "Pause button shown", app=app):
        return
    time.sleep(3)
    ctx.check("controls readable", lambda: ensure(all(app.find(l) for l in ("Pause", "Seek", "Volume", "Fullscreen", "Settings", "Up next")),
                                                  "missing one of Pause, Seek, Volume, Fullscreen, Settings, Up next") and "all six found", app=app)
    title = app.title()
    ctx.check("window title", lambda: ensure("playback" in title, f"title is {title!r}") and title, app=app)

    def video():
        if ctx.no_video:
            raise Skip("this runner can't show video")
        f = colourful(app.shot("playing"))
        return ensure(f > 0.2, f"only {f:.0%} of the centre shows the test pattern") and f"{f:.0%} of the centre is test pattern"

    ctx.check("video on screen", video, app=app)

    def pause_button():
        app.press("Pause")
        app.wait("Play", button=True, timeout=5)
        if ctx.no_video:
            return "button flipped to Play"
        a = app.shot("paused")
        time.sleep(1.5)
        b = app.shot("paused-later")
        ensure(ImageChops.difference(centre(a), centre(b)).getbbox() is None, "the picture kept moving")
        return "button flipped to Play; picture stopped"

    ctx.check("Pause button pauses", pause_button, app=app)
    ctx.check("Space resumes", lambda: (app.key("space"), app.wait("Pause", button=True, timeout=5)) and "playing again", app=app)

    def seek():
        before = app.time_ms()
        app.key("right")
        after = wait_for(lambda: (t := app.time_ms()) >= before + 4000 and t, "the clock to jump 5 s", 5)
        return f"{before // 1000} s → {after // 1000} s"

    ctx.check("→ jumps forward", seek, app=app)
    def jump():
        app.key("5")
        t = wait_for(lambda: 29_000 <= (t := app.time_ms()) <= 36_000 and t, "the clock near 0:30", 5)
        return f"at {t // 1000} s"

    ctx.check("5 jumps to 50%", jump, app=app)

    def volume():
        before = app.slider("Volume")
        if WIN and before == 0:
            raise Skip("no audio output on this runner: libVLC reports volume 0")
        app.key("up")
        after = wait_for(lambda: (v := app.slider("Volume")) > before and v, "the volume to rise", 5)
        return f"{before:g} → {after:g}"

    ctx.check("↑ raises the volume", volume, app=app)

    def mute():
        if WIN and app.slider("Volume") == 0:
            raise Skip("no audio output on this runner: libVLC reports volume 0")
        app.key("m")
        app.wait("Unmute", button=True, timeout=5)
        app.key("m")
        app.wait("Mute", button=True, timeout=5)
        return "M muted and unmuted"

    ctx.check("M mutes", mute, app=app)


@scenario
def fullscreen(ctx):
    app = ctx.launch(ctx.media / "playback.mp4")
    app.finish_onboarding()
    if not ctx.check("starts playing", lambda: app.wait_playing() or "playing", app=app):
        return
    before = app.bounds()

    def enter_f():
        app.key("f")
        app.wait("Exit fullscreen", button=True, timeout=10)
        screen = cua("get_screen_size", {})
        # Notched MacBooks keep fullscreen windows below the notch strip.
        b = wait_for(lambda: (b := app.bounds()) and b["width"] >= screen["width"] - 2 and b["height"] >= screen["height"] - 60 and b,
                     f"the window to cover the {screen['width']}×{screen['height']} screen", 10)
        app.shot("fullscreen")
        return f"{before['width']:g}×{before['height']:g} → {b['width']:g}×{b['height']:g}, the whole screen"

    if ctx.check("F enters fullscreen", enter_f, app=app):
        ctx.check("keys still work in fullscreen", lambda: (app.key("space"), app.wait("Play", button=True, timeout=5),
                                                            app.key("space"), app.wait("Pause", button=True, timeout=5)) and "Space paused and resumed", app=app)
        ctx.check("Escape leaves fullscreen", lambda: (app.key("escape"), app.wait("Fullscreen", button=True, timeout=10)) and "back to a window", app=app)
        time.sleep(1.5)

    def double_click():
        if ctx.no_video:
            # Without a picture the player shows its audio-only placeholder
            # there, which isn't the video area that double-click acts on.
            raise Skip("this runner can't show video")
        app.double_click_centre()
        app.wait("Exit fullscreen", button=True, timeout=10)
        time.sleep(1)
        app.double_click_centre()
        app.wait("Fullscreen", button=True, timeout=10)
        return "double-click entered and left fullscreen"

    ctx.check("double-click toggles fullscreen", double_click, app=app)


@scenario
def menus(ctx):
    app = ctx.launch(ctx.media / "playback.mp4")
    app.finish_onboarding()
    if not ctx.check("starts playing", lambda: app.wait_playing() or "playing", app=app):
        return

    def gear():
        start = app.time_ms()
        for _ in range(5):
            app.press("Settings")
            # Its items show their value too: "Playback speed Normal".
            wait_for(lambda: app.find(contains="Playback speed"), "the gear menu", 5)
            app.key("escape")
            wait_for(lambda: not app.find(contains="Playback speed"), "the gear menu to close", 5)
        later = wait_for(lambda: (t := app.time_ms()) > start + 1000 and t, "the clock to keep moving", 5)
        return f"opened and closed 5 times; clock {start // 1000} s → {later // 1000} s"

    ctx.check("gear menu opens and closes", gear, app=app)

    def info():
        if MAC:
            app.menu("Window", "Media Information…")
        else:
            app.shortcut("i")
        # Windows doesn't expose the dialog's heading; its tabs show everywhere.
        app.wait("Statistics", timeout=8)
        tabs = [l for l in ("General", "Codec", "Statistics") if app.find(l, fresh=False)]
        ensure(len(tabs) == 3, f"tabs found: {tabs}")
        app.press("Codec", any_role=True)
        return "dialog with General, Codec and Statistics"

    if ctx.check("media information opens", info, app=app):
        def trap():
            for _ in range(12):
                app.key("tab")
            app.key("space")
            time.sleep(1)
            # The dialog hides the player from the accessibility tree
            # (aria-modal), so close it before reading the play state.
            if app.find("Statistics"):
                app.key("escape")
                app.wait_gone("Statistics", timeout=5)
            # Space on a background control would have paused playback.
            ensure(app.playing(), "focus left the dialog: Space reached the player")
            app.shortcut("i") if not MAC else app.menu("Window", "Media Information…")
            app.wait("Statistics", timeout=8)
            return "12 Tabs and Space stayed inside the dialog"

        ctx.check("focus stays in the dialog", trap, app=app)
        ctx.check("Escape closes it", lambda: (app.key("escape"), app.wait_gone("Statistics", timeout=5)) and "closed", app=app)

    if not MAC:
        def title_menus():
            ensure(app.playing(), "not playing")
            names = ("Media", "Playback", "Audio", "Video", "Tools", "Help")
            found = [n for n in names if app.find(n, button=True, fresh=False)]
            ensure(len(found) == len(names), f"menus while playing: {found or 'none'}")
            return "all six menus reachable while playing"

        ctx.check("title-bar menus while playing", title_menus, known_bug="W6: the Windows and Linux menus only show on Home", app=app)

    def help_text():
        if app.find("Statistics"):
            app.key("escape")
            app.wait_gone("Statistics", timeout=5)
        if not MAC:
            app.press("Back to home")
            app.wait_text("Home", timeout=10)
        app.menu("Help", "Keyboard Shortcuts")
        app.wait("Keyboard shortcuts", timeout=8)
        text = app.text()
        if MAC:
            ensure("⌘" in text and "Return" in text, "expected ⌘ and Return")
        else:
            ensure("Ctrl+" in text and "Enter" in text and "Return" not in text and "⌘" not in text, "expected Ctrl+ and Enter, not ⌘ or Return")
        app.key("escape")
        return "platform names are right"

    ctx.check("shortcut list uses this platform's keys", help_text, app=app)



@scenario
def resume(ctx):
    app = ctx.launch(ctx.media / "playback.mp4")
    app.finish_onboarding()
    if not ctx.check("starts playing", lambda: app.wait_playing() or "playing", app=app):
        return
    app.key("5")
    time.sleep(3)
    if not ctx.check("quits normally", lambda: app.close() or "exited", app=app):
        return
    again = ctx.launch(ctx.media / "playback.mp4")

    def offer():
        # The offer's own aria-label isn't exposed on every platform; its
        # text and buttons are.
        again.wait("Continue", button=True, timeout=15)
        ensure("continue from" in again.text().lower(), "no Continue from… text")
        again.press("Continue")
        t = wait_for(lambda: (t := again.time_ms()) >= 28_000 and t, "the clock past 0:28", 8)
        return f"offered and resumed at {t // 1000} s"

    ctx.check("offers to continue after relaunch", offer, app=again)
    if MAC and ctx.bundle:
        def finder_open():
            again.close()
            # Launch Services starts the app and sends it an open-file event,
            # the way a double-click in Finder does.
            subprocess.run(["open", "-a", str(ctx.bundle), str(ctx.media / "playback.mp4")], check=True)
            pid = wait_for(lambda: [w["pid"] for w in cua("list_windows", {"app": "ReVLC"}).get("windows", [])
                                    if w["pid"] != again.pid and w.get("layer", 0) == 0], "ReVLC from open -a", 30)[0]
            opened = App(ctx, "resume-open-a", pid=pid)
            ctx.apps.append(opened)
            opened.wait_window()
            try:
                opened.wait("Continue", button=True, timeout=15)
            except Fail:
                opened.shot("resume--finder-open--failed")
                (ctx.out / "logs" / "resume--finder-open--failed.json").write_text(
                    json.dumps(opened.read(), indent=1, ensure_ascii=False), encoding="utf-8")
                raise
            return "a cold open from Finder offered to continue"

        ctx.check("cold open from Finder offers to continue", finder_open, app=again)


@scenario
def second_instance(ctx):
    app = ctx.launch(ctx.media / "playback.mp4")
    app.finish_onboarding()
    if not ctx.check("starts playing", lambda: app.wait_playing() or "playing", app=app):
        return

    def hand_off():
        ctx.hand_off(ctx.media / "second.mp4")
        wait_for(lambda: app.find("second"), "the player to show second.mp4", 15)
        ensure(len(app.windows()) == 1, "a second window opened")
        return "the same window now plays second.mp4"

    if ctx.check("opening another file uses the running app" if MAC else "a second launch hands its file over", hand_off, app=app):
        ctx.check("the window title follows", lambda: wait_for(lambda: "second" in (t := app.title()) and t, "the title to show second.mp4", 10), app=app)


@scenario
def bad_files(ctx):
    app = ctx.launch(ctx.media / "zero.mp4")
    app.finish_onboarding()
    ctx.check("empty file: readable error", lambda: app.wait_text("Could not play", timeout=15) and "notice shown", app=app)
    ctx.check("corrupt file: readable error", lambda: ctx.hand_off(ctx.media / "corrupt.mp4") or app.wait_text("Could not play", timeout=15) and "notice shown", app=app)
    # A fresh profile that hasn't played anything yet goes back to the welcome screen.
    ctx.check("still responsive", lambda: ensure(app.alive(), "app exited") and (app.finish_onboarding() or app.wait_text("Home", timeout=10)) and "Home shown", app=app)


@scenario
def engine_failure(ctx):
    app = ctx.launch(env={"REVLC_VLC_DIR": str(ctx.broken_vlc)})
    app.finish_onboarding()
    def explains():
        # The heading isn't exposed on Windows; the error under it is, on
        # every platform.
        wait_for(lambda: (t := app.text()) and ("couldn't start" in t or "Couldn't load" in t), "the engine error on Home", 15)
        lines = [l for l in app.text().splitlines() if "couldn't start" in l or "Couldn't load" in l]
        return f"shown: {lines[-1][:120]}"

    ctx.check("explains the engine failure", explains, app=app)

    def settings():
        w = app.settings_window()
        wait_for(lambda: "General" in app.text(w), "the Settings tabs", 10)
        time.sleep(3)
        text = app.text(w)
        ensure("Loading…" not in text, 'Settings still says "Loading…"')
        return "Settings opened and loaded"

    ctx.check("Settings still opens", settings, app=app)

    if MAC:
        def menu_bar():
            items = [e.get("label") for e in cua("get_window_state", {"pid": app.pid, "window_id": app.window_id, "tree_format": "elements",
                                                                      "include_screenshot": False, "query": "Settings"}).get("elements", [])]
            ensure("Settings…" in items, "the menu bar has no Settings… (only the default app menu)")
            return "Settings… is in the menu bar"

        ctx.check("menu bar still has Settings…", menu_bar, app=app,
                  known_bug="found by this suite: the custom menu bar is only built when the engine starts")


@scenario
def corrupt_history(ctx):
    data = ctx.data_dir()
    data.mkdir(parents=True, exist_ok=True)
    (data / "history.json").write_text('[{"uri": tru', encoding="utf-8")
    app = ctx.launch(ctx.media / "playback.mp4")
    app.finish_onboarding()
    ctx.check("tells the user", lambda: app.wait_text("history couldn't be read", timeout=15) and "notice shown", app=app)
    ctx.check("plays anyway", lambda: app.wait_playing() or "playing", app=app)


@scenario
def command_line(ctx):
    app = ctx.launch("--start-time=30", ctx.media / "playback.mp4", name="command_line-start")
    app.finish_onboarding()
    ctx.check("--start-time", lambda: f"started at {wait_for(lambda: (t := app.time_ms()) >= 29_000 and t, 'the clock past 0:29', 15) // 1000} s", app=app)
    app.kill()

    app = ctx.launch("--intf=dummy", "--no-such-option", ctx.media / "playback.mp4", name="command_line-ignored")
    app.finish_onboarding()
    ctx.check("unsafe options are reported", lambda: app.wait_text("Ignored command-line options: --intf=dummy", timeout=15) and "notice shown", app=app)
    ctx.check("and the file still plays", lambda: app.wait_playing() or "playing", app=app)
    app.kill()

    app = ctx.launch("-f", ctx.media / "playback.mp4", name="command_line-fullscreen")
    app.finish_onboarding()
    def starts_fullscreen():
        if ctx.no_video:
            # ReVLC enters fullscreen once the video appears, and there's none here.
            raise Skip("this runner can't show video")
        return app.wait("Exit fullscreen", button=True, timeout=15) and "fullscreen"

    ctx.check("-f starts in fullscreen", starts_fullscreen, app=app)
    app.kill()

    app = ctx.launch("--play-and-exit", "--run-time=3", ctx.media / "playback.mp4", name="command_line-exit")

    def exits():
        start = time.monotonic()
        ensure(app.wait_exit(30), "still running 30 s later")
        return f"exited after {time.monotonic() - start:.1f} s"

    ctx.check("--play-and-exit --run-time=3 quits", exits, app=app)


@scenario
def paths_and_folders(ctx):
    app = ctx.launch(ctx.long_file)
    app.finish_onboarding()
    ctx.check(f"plays from a {len(str(ctx.long_file))}-character Unicode path", lambda: app.wait_playing() or "playing", app=app)
    app.kill()

    app = ctx.launch(ctx.media / "folder", name="paths_and_folders-folder")
    app.finish_onboarding()
    ctx.check("a folder plays its first file", lambda: app.wait_playing() or "playing", app=app)

    def library():
        app.press("Back to home")
        app.wait_text("Videos", timeout=10)
        ensure(any(e.get("label", "").startswith("Videos") and "2" in e.get("label", "") for e in app.read()) or "2" in app.text(),
               "Videos doesn't show the folder's 2 files")
        return "the folder is in the library"

    ctx.check("the folder joins the library", library, app=app)


@scenario
def queue(ctx):
    app = ctx.launch(ctx.media / "playback.mp4", ctx.media / "second.mp4")
    app.finish_onboarding()
    # The player shows what's playing as text next to Back to home.
    if not ctx.check("starts on the first file", lambda: app.wait("playback", timeout=15) and "playing playback.mp4", app=app):
        return
    ctx.check("the window title follows", lambda: wait_for(lambda: "playback" in (t := app.title()) and t, "the title to show playback.mp4", 10), app=app)

    def up_next():
        app.press("Up next")
        app.wait_text("second", timeout=5)
        app.press("Up next")
        return "both files listed"

    ctx.check("Up next lists both", up_next, app=app)
    ctx.check("N plays the next one", lambda: (app.key("n"), app.wait("second", timeout=10)) and "playing second.mp4", app=app)


@scenario
def small_window(ctx):
    app = ctx.launch(ctx.media / "playback.mp4")
    app.finish_onboarding()
    if not ctx.check("starts playing", lambda: app.wait_playing() or "playing", app=app):
        return

    def fits():
        b = app.bounds()
        cua("set_window_frame", {"pid": app.pid, "window_id": app.window_id, "x": b["x"], "y": b["y"], "width": 720, "height": 450})
        time.sleep(1.5)
        b = app.bounds()
        ensure(abs(b["width"] - 720) < 40 and abs(b["height"] - 450) < 40, f"couldn't resize the window: it's {b['width']:g}×{b['height']:g}")
        app.shot("player-720x450")
        right, bottom = b["x"] + b["width"], b["y"] + b["height"]
        missing, outside = [], []
        # The volume slider stays collapsed until the speaker is hovered; Mute is its button.
        for label in ("Pause", "Seek", "Mute", "Fullscreen", "Settings", "Up next", "Subtitles"):
            els = app.find(label)
            if not els:
                missing.append(label)
                continue
            f = els[0].get("frame")
            if not f:
                missing.append(f"{label} (no position)")
                continue
            if f["x"] + f["w"] > right + 1 or f["y"] + f["h"] > bottom + 1:
                outside.append(label)
        ensure(not missing and not outside, f"missing {missing}, outside the window {outside}")
        return f"all controls inside {b['width']:g}×{b['height']:g}"

    ctx.check("controls fit a 720×450 window", fits, app=app)


@scenario
def subtitles(ctx):
    app = ctx.launch(ctx.media / "subtitled.mp4", ctx.media / "subtitled.srt")
    app.finish_onboarding()
    if not ctx.check("starts playing", lambda: app.wait_playing() or "playing", app=app):
        return

    def tracks():
        """Opens the Subtitles menu and returns its track items."""
        app.press("Subtitles")
        wait_for(lambda: app.find("Off"), "the Subtitles menu", 5)
        return [e for e in app.elements if (e.get("label") or "").startswith("Track")]

    def listed():
        items = tracks()
        app.key("escape")
        ensure(items, "no subtitle track besides Off")
        return f"tracks {[e.get('label') for e in items]}"

    ctx.check("the SRT is listed", listed, app=app)

    def white_text(name):
        im = Image.open(app.shot(name)).convert("RGB")
        w, h = im.size
        band = im.crop((w // 6, int(h * 0.6), 5 * w // 6, int(h * 0.95)))
        return sum(min(p) > 200 for p in band.getdata())

    def drawn():
        if ctx.no_video:
            raise Skip("this runner can't show video")
        white = white_text("subtitle")
        return ensure(white > 200, f"only {white} white pixels where the subtitle goes") and f"{white} white pixels of subtitle text"

    ctx.check("the subtitle is drawn", drawn, app=app)

    def survives_style_change():
        w = app.settings_window()
        tab = wait_for(lambda: [e for e in app.find("Subtitles", window_id=w) if is_button(e)], "the Subtitles tab", 10)
        cua("click", {"element_token": tab[0]["element_token"]})
        # The switch is named after its row's label; its role differs by platform.
        wait_for(lambda: [e for e in app.read(w) if e.get("label") == "Bold"], "the Bold switch", 10)
        app.click_switch("Bold", w)
        time.sleep(3)
        if not ctx.no_video:
            # What a user sees: the subtitle is still on the picture.
            white = white_text("subtitle-after-bold")
            ensure(white > 200, f"the subtitle is gone from the picture ({white} white pixels)")
            return f"still drawn ({white} white pixels)"
        items = tracks()
        app.key("escape")
        checked = [e.get("label") for e in items if e.get("selected") or str(e.get("value")).lower() in ("1", "true")]
        ensure(checked, f"no subtitle track selected after the change ({[e.get('label') for e in items]})")
        return f"still selected: {checked}"

    ctx.check("turning on Bold keeps the subtitle", survives_style_change, app=app)


def frame_pair(app, name):
    """Pauses, takes a frame, and returns its path (the picture must hold still)."""
    if app.playing():
        app.press("Pause")
        app.wait("Play", button=True, timeout=5)
    time.sleep(0.8)
    return app.shot(name)


def mean_rgb_diff(a, b, invert=False):
    pa, pb = list(centre(a).getdata()), list(centre(b).getdata())
    n = min(len(pa), len(pb))
    f = (lambda x: 255 - x) if invert else (lambda x: x)
    return sum(abs(f(x[i]) - y[i]) for x, y in zip(pa[:n], pb[:n]) for i in range(3)) / (3 * n)


def greyish(path):
    px = list(centre(path).getdata())
    return sum(max(p) - min(p) < 24 for p in px) / len(px)


@scenario
def effects(ctx):
    app = ctx.launch(ctx.media / "playback.mp4")
    app.finish_onboarding()
    if not ctx.check("starts playing", lambda: app.wait_playing() or "playing", app=app):
        return

    def opens():
        app.gear_item("Effects and filters")
        app.click_el(app.find(contains="Effects and filters")[-1])
        wait_for(lambda: all(app.find(t, fresh=False) or app.find(t) for t in ("Equaliser", "Colour", "Overlays")), "the effects tabs", 8)
        return "Equaliser, Audio, Colour, Video and Overlays tabs"

    if not ctx.check("panel opens from the gear menu", opens, app=app):
        return

    def invert():
        if ctx.no_video:
            raise Skip("this runner can't show video")
        before = frame_pair(app, "effects-before")
        app.press("Video", any_role=True)
        app.click_switch("Invert colours", app.window_id)
        app.key("e")  # next frame, drawn with the filter
        time.sleep(1.5)
        after = app.shot("effects-inverted")
        inv, same = mean_rgb_diff(before, after, invert=True), mean_rgb_diff(before, after)
        ensure(inv < same / 2, f"the picture isn't inverted (distance to inverse {inv:.0f}, to original {same:.0f})")
        app.click_switch("Invert colours", app.window_id)
        return f"inverted: {inv:.0f} from the inverse, {same:.0f} from the original"

    ctx.check("Invert colours inverts the picture", invert, app=app)

    def greyscale():
        if ctx.no_video:
            raise Skip("this runner can't show video")
        app.press("Colour", any_role=True)
        app.click_switch("Adjust colours", app.window_id)
        sat = wait_for(lambda: [e for e in app.read() if e.get("label") == "Saturation" and "slider" in (e.get("role") or "").lower()
                                or e.get("label") == "Saturation" and e.get("frame") and e["frame"].get("w", 0) > 40],
                       "the Saturation slider", 5)[0]
        # Focus the slider and press Home, as a keyboard user does: WebKit
        # ignores synthetic background clicks on a range input.
        def saturation():
            return [e for e in app.read() if e.get("label") == "Saturation" and "slider" in (e.get("role") or "").lower()][0]

        for _ in range(3):
            cua("click", {"element_token": saturation()["element_token"]})
            app.key("home")
            time.sleep(0.8)
            if str(saturation().get("value")) in ("0", "0.0"):
                break
        else:
            raise Fail(f"couldn't set Saturation to 0 (it's {saturation().get('value')}; enabled={saturation().get('enabled')})")
        app.key("e")
        time.sleep(1.5)
        g = greyish(app.shot("effects-greyscale"))
        ensure(g > 0.9, f"only {g:.0%} of the picture is grey with saturation at 0")
        return f"{g:.0%} of the picture is grey"

    ctx.check("Saturation at 0 makes it grey", greyscale, app=app)

    def equaliser():
        app.press("Equaliser", any_role=True)
        before = app.switch_on("Equaliser")
        app.click_switch("Equaliser", app.window_id)
        after = wait_for(lambda: app.switch_on("Equaliser") != before, "the Equaliser switch to change", 5)
        return f"switched {'on' if not before else 'off'}"

    ctx.check("Equaliser switch", equaliser, app=app)

    def stays_on():
        app.press("Video", any_role=True)
        if not app.switch_on("Invert colours"):
            app.click_switch("Invert colours", app.window_id)
        ctx.hand_off(ctx.media / "second.mp4")
        app.wait("second", timeout=15)
        if not app.find("Invert colours"):
            app.press("Video", any_role=True)
        ensure(app.switch_on("Invert colours"), "Invert colours turned off for the next file")
        return "still on for the next file"

    ctx.check("filters stay on for the next file", stays_on, app=app)


@scenario
def playback_controls(ctx):
    app = ctx.launch(ctx.media / "playback.mp4")
    app.finish_onboarding()
    if not ctx.check("starts playing", lambda: app.wait_playing() or "playing", app=app):
        return

    def speed():
        app.key("]")
        app.key("]")
        label = app.gear_item("Playback speed")
        app.close_menus()
        ensure("1.5" in label, f"gear menu says {label!r}")
        t0, w0 = app.time_ms(), time.monotonic()
        time.sleep(4)
        t1, w1 = app.time_ms(), time.monotonic()
        rate = (t1 - t0) / 1000 / (w1 - w0)
        ensure(1.2 < rate < 1.8, f"the clock ran at {rate:.2f}× real time")
        app.key("=")
        ensure("Normal" in app.gear_item("Playback speed"), "= didn't return to normal speed")
        app.close_menus()
        return f"{label}; clock ran at {rate:.2f}×; = back to Normal"

    ctx.check("] speeds up, = resets", speed, app=app)

    def jump():
        app.command(("Playback", "Jump to Time…"), "t")
        field = wait_for(lambda: [e for e in app.read() if "text" in (e.get("role") or "").lower() and "field" in (e.get("role") or "").lower()
                                  or (e.get("role") or "").lower() in ("edit", "entry", "axtextfield")], "the time field", 8)[0]
        app.type_into(field, "0:45")
        app.key("return")
        t = wait_for(lambda: 44_000 <= (t := app.time_ms()) <= 50_000 and t, "the clock near 0:45", 8)
        return f"jumped to {t // 1000} s"

    ctx.check("Jump to time", jump, app=app)

    def ab_loop():
        app.key("2")  # 20 %
        time.sleep(1)
        app.key("l", ("shift",))
        time.sleep(3)
        app.key("l", ("shift",))
        label = app.gear_item("A–B loop")
        app.close_menus()
        ensure("Off" not in label, f"gear menu says {label!r}")
        times = []
        for _ in range(8):
            times.append(app.time_ms())
            time.sleep(1)
        ensure(max(times) - min(times) <= 6000, f"the clock left the loop: {[t // 1000 for t in times]}")
        app.key("l", ("shift",))  # third press clears it
        return f"{label}; clock stayed in {min(times) // 1000}–{max(times) // 1000} s"

    ctx.check("A–B loop holds playback in the loop", ab_loop, app=app)

    def bookmark():
        app.command(("Playback", "Add Bookmark"), "b")
        label = wait_for(lambda: (l := app.gear_item("Bookmarks")) and "1" in l and l, "Bookmarks 1", 5)
        app.close_menus()
        return label

    ctx.check("Add bookmark", bookmark, app=app)

    def snapshot():
        shots = ctx.out / "screenshots"
        before = set(shots.glob("*"))
        app.key("s", ("shift",))
        new = wait_for(lambda: [p for p in shots.glob("*") if p not in before and p.suffix.lower() in (".png", ".jpg", ".jpeg")],
                       "a snapshot file", 10)
        return f"saved {new[0].name}"

    ctx.check("Shift+S saves a snapshot", snapshot, app=app)

    def sleep_timer():
        app.gear_item("Sleep timer")
        app.click_el(app.find(contains="Sleep timer")[0])
        choice = wait_for(lambda: [e for e in app.read() if (e.get("label") or "").endswith("minutes")], "sleep timer choices", 5)[0]
        app.click_el(choice)
        app.close_menus()
        badge = wait_for(lambda: app.find(contains="Turn off", button=True), "the sleep timer badge", 5)[0]
        app.click_el(badge)
        wait_for(lambda: not app.find(contains="Turn off", button=True), "the badge to clear", 5)
        return f"{choice['label']}, shown and turned off"

    ctx.check("Sleep timer", sleep_timer, app=app)


@scenario
def chapters(ctx):
    app = ctx.launch(ctx.media / "chapters.mp4")
    app.finish_onboarding()
    if not ctx.check("starts playing", lambda: app.wait_playing() or "playing", app=app):
        return
    ctx.check("gear menu lists chapters", lambda: app.gear_item("Chapters") and (app.close_menus() or "Chapters shown"), app=app)

    def next_chapter():
        app.key("n", ("shift",))
        t = wait_for(lambda: 19_000 <= (t := app.time_ms()) <= 26_000 and t, "the clock at the second chapter", 8)
        ensure(app.find(contains="Middle part"), "the second chapter's name isn't shown")
        return f"at {t // 1000} s, showing Middle part"

    ctx.check("Shift+N goes to the next chapter", next_chapter, app=app)


@scenario
def tracks(ctx):
    app = ctx.launch(ctx.media / "tracks.mkv")
    app.finish_onboarding()
    if not ctx.check("starts playing", lambda: app.wait_playing() or "playing", app=app):
        return

    def audio():
        first = app.gear_item("Audio")
        app.close_menus()
        app.key("b")
        second = wait_for(lambda: (l := app.gear_item("Audio")) != first and l, "the audio track to change", 5)
        app.close_menus()
        return f"{first} → {second}"

    ctx.check("B switches audio track", audio, app=app)

    def subs():
        app.press("Subtitles")
        wait_for(lambda: app.find("Off"), "the Subtitles menu", 5)
        names = [e.get("label") for e in app.elements if "eng" in (e.get("label") or "").lower() or "fre" in (e.get("label") or "").lower()
                 or (e.get("label") or "").startswith("Track")]
        app.key("escape")
        ensure(len(names) >= 2, f"subtitle tracks listed: {names}")
        return f"listed {names}"

    ctx.check("both subtitle tracks listed", subs, app=app)


@scenario
def streams(ctx):
    url = f"{ctx.http}/hls/stream.m3u8"
    app = ctx.launch(url)
    app.finish_onboarding()
    ctx.check("plays an HLS stream", lambda: app.wait_playing(30) or f"playing {url}", app=app)
    ctx.check("its clock moves", lambda: (t := app.time_ms()) is not None and wait_for(lambda: app.time_ms() > t + 1500, "the clock to move", 8)
              and "advancing", app=app)
    app.kill()

    app = ctx.launch(name="streams-dialog")
    app.finish_onboarding()

    def dialog():
        app.command(("File", "Open Network Stream…"), "n")
        field = wait_for(lambda: [e for e in app.read() if (e.get("role") or "").lower() in ("axtextfield", "edit", "entry", "text")
                                  and not e.get("label") or "textfield" in (e.get("role") or "").lower()], "the URL field", 8)[0]
        app.type_into(field, url)
        app.key("return")
        app.wait_playing(30)
        return "opened from the dialog and playing"

    ctx.check("Open network stream dialog", dialog, app=app)


@scenario
def playlist_files(ctx):
    app = ctx.launch(ctx.media / "two.m3u")
    app.finish_onboarding()
    ctx.check("an M3U starts its first item", lambda: app.wait("playback", timeout=15) and "playing playback.mp4", app=app)

    def listed():
        app.press("Up next")
        app.wait_text("second", timeout=5)
        return "both items in Up next"

    ctx.check("Up next lists the playlist", listed, app=app)

    def remove():
        b = wait_for(lambda: [e for e in app.read() if (e.get("label") or "").startswith("Remove second")], "Remove second…", 5)[0]
        app.click_el(b)
        wait_for(lambda: not [e for e in app.read() if (e.get("label") or "").startswith("Remove second")], "second to leave the queue", 5)
        return "removed"

    ctx.check("an item can be removed", remove, app=app)

    def shuffle():
        # A toggle button: a checkbox to accessibility on macOS.
        before = wait_for(lambda: app.find("Shuffle"), "Shuffle", 5)[0]
        app.click_el(before)
        after = wait_for(lambda: (a := app.find("Shuffle")) and (a[0].get("value"), a[0].get("selected")) != (before.get("value"), before.get("selected")) and a,
                         "Shuffle to change state", 5)[0]
        app.click_el(after)
        return "Shuffle toggles"

    ctx.check("Shuffle toggles", shuffle, app=app)


@scenario
def history(ctx):
    app = ctx.launch(ctx.media / "playback.mp4")
    app.finish_onboarding()
    if not ctx.check("starts playing", lambda: app.wait_playing() or "playing", app=app):
        return
    # Halfway in: ReVLC lists a file under Recent until it's far enough in
    # to be worth continuing.
    app.key("5")
    time.sleep(3)

    def continue_watching():
        app.press("Back to home")
        app.wait_text("Continue watching", timeout=10)
        ensure(app.find("Remove playback from history", button=True), "playback isn't in Continue watching")
        return "playback listed under Continue watching"

    if ctx.check("Continue watching lists it", continue_watching, app=app):
        ctx.check("it can be removed from history", lambda: (app.press("Remove playback from history"),
                                                             app.wait_gone("Remove playback from history", timeout=5)) and "removed", app=app)


@scenario
def settings_persist(ctx):
    app = ctx.launch()
    app.finish_onboarding()

    def find_switch(w, label):
        """Opens each Settings tab until `label` shows."""
        for tab in ("General", "Video", "Audio", "Subtitles", "Input & Codecs"):
            if [e for e in app.read(w) if e.get("label") == label]:
                return True
            tabs = [e for e in app.elements if e.get("label") == tab]
            if tabs:
                app.click_el(tabs[0], w)
                time.sleep(0.8)
        return bool([e for e in app.read(w) if e.get("label") == label])

    def toggle_and_relaunch():
        w = app.settings_window()
        ensure(find_switch(w, "Pause when minimised"), "no Pause when minimised switch")
        before = app.switch_on("Pause when minimised", w)
        app.click_switch("Pause when minimised", w)
        wait_for(lambda: app.switch_on("Pause when minimised", w) != before, "the switch to change", 5)
        time.sleep(1.5)
        saved = json.loads((ctx.data_dir() / "settings.json").read_text(encoding="utf-8")).get("pauseWhenMinimized")
        ensure(saved == (not before), f"settings.json has pauseWhenMinimized={saved}")
        app.close()
        again = ctx.launch(name="settings_persist-2")
        again.finish_onboarding()
        w2 = again.settings_window()
        ensure(find_switch_for(again, w2), "no Pause when minimised switch after relaunch")
        ensure(again.switch_on("Pause when minimised", w2) == (not before), "the setting didn't survive a relaunch")
        return f"{'on' if not before else 'off'} in settings.json and after relaunch"

    def find_switch_for(a, w):
        nonlocal app
        app_, app = app, a
        try:
            return find_switch(w, "Pause when minimised")
        finally:
            app = app_

    ctx.check("a setting survives a relaunch", toggle_and_relaunch, app=app)


@scenario
def shortcuts(ctx):
    app = ctx.launch(ctx.media / "playback.mp4")
    app.finish_onboarding()
    if not ctx.check("starts playing", lambda: app.wait_playing() or "playing", app=app):
        return

    def rebind():
        w = app.settings_window()
        tab = wait_for(lambda: [e for e in app.read(w) if e.get("label") == "Shortcuts"], "the Shortcuts tab", 8)[0]
        app.click_el(tab, w)
        key_button = wait_for(lambda: app.button_after("Mute", w), "Mute's shortcut", 8)
        app.click_el(key_button, w)
        time.sleep(0.5)
        app.key("y", window_id=w)
        time.sleep(1)
        # Back in the player, Y now mutes.
        cua("bring_to_front", {"pid": app.pid, "window_id": app.window_id})
        app.key("y")
        app.wait("Unmute", button=True, timeout=5)
        app.key("y")
        return "Mute moved to Y, and Y mutes"

    ctx.check("a shortcut can be changed", rebind, app=app)


def serve_media(ctx):
    """Serves the media folder on a local port, for the stream scenarios."""
    import functools
    import http.server
    import threading

    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *args):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Quiet, directory=str(ctx.media)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    ctx.http = f"http://127.0.0.1:{server.server_address[1]}"


def main():
    # Windows consoles default to a code page that can't print "→" or "—".
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--ffmpeg", required=True)
    ap.add_argument("--bundle", type=Path, help="macOS: the .app, for opening files through Launch Services")
    ap.add_argument("--no-video", action="store_true", help="skip the picture checks")
    ap.add_argument("--only", help="comma-separated scenario names")
    ap.add_argument("app", nargs="+")
    a = ap.parse_args()
    a.out = a.out.resolve()
    ctx = Context(a)
    make_media(ctx)
    serve_media(ctx)
    if not (ctx.out / "backup/manifest.json").exists():
        ctx.appdata("backup")
    only = set(a.only.split(",")) if a.only else None
    started = time.monotonic()
    try:
        for fn in SCENARIOS:
            if only and fn.__name__ not in only:
                continue
            ctx.scenario = fn.__name__
            print(f"== {ctx.scenario}", flush=True)
            ctx.appdata("seed")
            try:
                fn(ctx)
            except Exception as e:
                ctx.results.append({"scenario": ctx.scenario, "check": "(scenario)", "status": "fail",
                                    "detail": f"{type(e).__name__}: {e}\n{traceback.format_exc(limit=4)}", "known_bug": None, "seconds": 0})
                print(f"  [FAIL ] scenario crashed: {e}", flush=True)
            finally:
                for app in ctx.apps:
                    app.kill()
                ctx.apps = []
                time.sleep(1)
    finally:
        try:
            ctx.appdata("restore")
        except Exception as e:
            print(f"restore: {e}", flush=True)
    write_report(ctx, time.monotonic() - started)
    sys.exit(1 if any(r["status"] == "fail" for r in ctx.results) else 0)


def write_report(ctx, seconds):
    rs = ctx.results
    counts = {s: sum(r["status"] == s for r in rs) for s in ("pass", "fail", "skip", "xfail", "xpass")}
    (ctx.out / "report.json").write_text(json.dumps({"platform": PLATFORM, "seconds": round(seconds), "counts": counts, "results": rs}, indent=2))
    icon = {"pass": "✅", "fail": "❌", "skip": "⏭️", "xfail": "🐞", "xpass": "⚠️"}
    lines = [f"### {PLATFORM}: {counts['pass']} passed, {counts['fail']} failed, {counts['skip']} skipped, "
             f"{counts['xfail']} known bugs, {counts['xpass']} known bugs now passing ({seconds / 60:.1f} min)", "",
             "| | Scenario | Check | Detail |", "|---|---|---|---|"]
    for r in rs:
        detail = (r["detail"] or "").splitlines()[0][:160].replace("|", "\\|")
        if r["known_bug"]:
            detail = f"{r['known_bug']}. {detail}"
        lines.append(f"| {icon[r['status']]} | {r['scenario']} | {r['check']} | {detail} |")
    md = "\n".join(lines) + "\n"
    (ctx.out / "report.md").write_text(md, encoding="utf-8")
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write(md)
    print(md)


if __name__ == "__main__":
    main()
