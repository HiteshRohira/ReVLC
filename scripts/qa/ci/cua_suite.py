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
        els = self.find(label, window_id=window_id, button=not any_role, **kw)
        ensure(els, f'no "{label}" {"control" if any_role else "button"}')
        cua("click", {"element_token": els[0]["element_token"]})

    def key(self, key, mods=()):
        args = {"pid": self.pid, "window_id": self.window_id}
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
        cua("click", {"element_token": els[0]["element_token"]})

    def settings_window(self):
        """Opens Settings (its own window) and returns that window's id:
        from Home's button when Home shows, else the menu or shortcut."""
        if self.find("Settings", button=True) and self.find("Home"):
            self.press("Settings")
        elif MAC:
            self.menu("App", "Settings…")
        else:
            self.shortcut(",")
        w = wait_for(lambda: [w for w in self.windows() if w["window_id"] != self.window_id], "the Settings window", 10)
        return w[0]["window_id"]

    def double_click_centre(self):
        b = self.bounds() or self.frame
        # Pixel clicks are relative to a screenshot Cua has taken of the window.
        cua("get_window_state", {"pid": self.pid, "window_id": self.window_id, "include_accessibility_tree": False})
        args = {"pid": self.pid, "window_id": self.window_id, "x": b["width"] // 2, "y": b["height"] // 3}
        try:
            cua("double_click", args)
        except RuntimeError as e:
            # WebKitGTK ignores synthetic background input; a CI desktop can take focus.
            if "background_unavailable" not in str(e):
                raise
            cua("double_click", {**args, "delivery_mode": "foreground"})

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
        if self.find("Get started", button=True):
            self.press("Get started")
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
            if self.proc:
                self.proc.kill()
                self.proc.wait()
            else:
                os.kill(self.pid, 9)
        self.log.close()


class Context:
    def __init__(self, a):
        self.out = a.out
        self.app = a.app
        self.bundle = a.bundle
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
        r = subprocess.run([sys.executable, "-I", str(ROOT / "scripts/qa/appdata.py"), cmd, str(self.out)], capture_output=True, text=True)
        if r.returncode:
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

    ctx.check("fits a 720×450 window", fits, known_bug="W7: onboarding text runs below a minimum-size window", app=app)
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
        app.key("up")
        after = wait_for(lambda: (v := app.slider("Volume")) > before and v, "the volume to rise", 5)
        return f"{before:g} → {after:g}"

    ctx.check("↑ raises the volume", volume, app=app)

    def mute():
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
        b = wait_for(lambda: (b := app.bounds()) and b["width"] >= screen["width"] - 2 and b["height"] >= screen["height"] - 2 and b,
                     f"the window to cover the {screen['width']}×{screen['height']} screen", 10)
        app.shot("fullscreen")
        return f"{before['width']:g}×{before['height']:g} → {b['width']:g}×{b['height']:g}, the whole screen"

    if ctx.check("F enters fullscreen", enter_f, app=app):
        ctx.check("keys still work in fullscreen", lambda: (app.key("space"), app.wait("Play", button=True, timeout=5),
                                                            app.key("space"), app.wait("Pause", button=True, timeout=5)) and "Space paused and resumed", app=app)
        ctx.check("Escape leaves fullscreen", lambda: (app.key("escape"), app.wait("Fullscreen", button=True, timeout=10)) and "back to a window", app=app)
        time.sleep(1.5)

    def double_click():
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
        app.wait("Media information", timeout=8)
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
            if app.find("Media information"):
                app.key("escape")
                app.wait_gone("Media information", timeout=5)
            # Space on a background control would have paused playback.
            ensure(app.playing(), "focus left the dialog: Space reached the player")
            app.shortcut("i") if not MAC else app.menu("Window", "Media Information…")
            app.wait("Media information", timeout=8)
            return "12 Tabs and Space stayed inside the dialog"

        ctx.check("focus stays in the dialog", trap, app=app)
        ctx.check("Escape closes it", lambda: (app.key("escape"), app.wait_gone("Media information", timeout=5)) and "closed", app=app)

    def help_text():
        if app.find("Media information"):
            app.key("escape")
            app.wait_gone("Media information", timeout=5)
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

    if not MAC:
        def title_menus():
            ensure(app.find("Video", button=True) or app.find("Playback", button=True), "no Media/Playback/Video menus while playing")
            return "menus reachable"

        ctx.check("title-bar menus while playing", title_menus, known_bug="W6: the Windows and Linux menus only show on Home", app=app)


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
        title = wait_for(lambda: "second" in (t := app.title()) and t, "the window to show second.mp4", 15)
        ensure(len(app.windows()) == 1, "a second window opened")
        return f"same window now plays {title!r}"

    ctx.check("opening another file uses the running app" if MAC else "a second launch hands its file over", hand_off, app=app)


@scenario
def bad_files(ctx):
    app = ctx.launch(ctx.media / "zero.mp4")
    app.finish_onboarding()
    ctx.check("empty file: readable error", lambda: app.wait_text("Could not play", timeout=15) and "notice shown", app=app)
    ctx.check("corrupt file: readable error", lambda: ctx.hand_off(ctx.media / "corrupt.mp4") or app.wait_text("Could not play", timeout=15) and "notice shown", app=app)
    ctx.check("still responsive", lambda: ensure(app.alive(), "app exited") and app.wait_text("Home", timeout=10) and "Home shown", app=app)


@scenario
def engine_failure(ctx):
    app = ctx.launch(env={"REVLC_VLC_DIR": str(ctx.broken_vlc)})
    app.finish_onboarding()
    ctx.check("explains the engine failure", lambda: app.wait_text("playback engine couldn't start", timeout=15) and "message shown", app=app)

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
    ctx.check("-f starts in fullscreen", lambda: app.wait("Exit fullscreen", button=True, timeout=15) and "fullscreen", app=app)
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
    if not ctx.check("starts on the first file", lambda: wait_for(lambda: "playback" in app.title(), "playback.mp4", 15) and app.title(), app=app):
        return

    def up_next():
        app.press("Up next")
        app.wait_text("second", timeout=5)
        app.press("Up next")
        return "both files listed"

    ctx.check("Up next lists both", up_next, app=app)
    ctx.check("N plays the next one", lambda: (app.key("n"), wait_for(lambda: "second" in (t := app.title()) and t, "second.mp4", 10))[-1], app=app)


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
        for label in ("Pause", "Seek", "Volume", "Fullscreen", "Settings", "Up next", "Subtitles"):
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
        bold = wait_for(lambda: [e for e in app.read(w) if e.get("label") == "Bold" and e.get("actions")
                                 and "text" not in (e.get("role") or "").lower()], "the Bold switch", 10)
        cua("click", {"element_token": bold[0]["element_token"]})
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
