#!/usr/bin/env python3
"""Proves the Android app's Browser place (Places.kt, the site's
`#browser-tile` in a web view) against a throwaway Libri with its own
browser on: an agent opens a page and the place shows it live; the person's
tap in the place presses the page's button, which the agent then reads; a
sideways drag over the page stays on the Browser rather than moving along
the bar. In the light and the dark theme. Leaves screenshots.

Runs inside `nix develop .#android-emulator`; the APK must be a debug build;
CHECK_CHROMIUM and CHECK_XVFB name the agents' browser and display:
  drive_android_browser.py LIBRI_DIR APK SHOTS_DIR [SERIAL]
"""
import http.server
import json
import os
import shutil
import tempfile
import subprocess
import sys
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
LIBRI_DIR, APK, SHOTS = sys.argv[1], sys.argv[2], Path(sys.argv[3])
SERIAL = sys.argv[4] if len(sys.argv) > 4 else "emulator-5620"
sys.argv = [sys.argv[0], LIBRI_DIR, APK, SERIAL]
import smoke_android as s  # noqa: E402  its emulator, adb and screen
from throwaway_libri import Throwaway  # noqa: E402

# One button over most of the page: a tap anywhere near the middle presses it.
PAGE = b"""<!doctype html><title>Android page</title>
<meta name="viewport" content="width=device-width">
<style>html,body{margin:0;height:100%}button{position:relative;width:100%;height:80%;font-size:40px;border:0;background:#c63}
#g{position:fixed;inset:0;background:#36c}</style><div id=g></div>
<button onclick="n++;this.textContent='Pressed '+n;this.style.background='#6e6'">Not pressed</button>
<p id=v></p>
<script>let n=0;const v=()=>document.getElementById('v').textContent='viewport '+innerWidth+'x'+innerHeight+' at '+devicePixelRatio;v();addEventListener('resize',v)</script>"""

failures = []


def check(what, ok, detail=""):
    print(f"{'PASS' if ok else 'FAIL'} {what} {'' if ok else str(detail)[:300]}", flush=True)
    if not ok:
        failures.append(what)


def shot(name):
    SHOTS.mkdir(parents=True, exist_ok=True)
    with open(SHOTS / f"{name}.png", "wb") as f:
        subprocess.run(["adb", "-s", SERIAL, "exec-out", "screencap", "-p"], stdout=f)


def pixel(x, y):
    """The screen's colour at (x, y), from a raw screen capture."""
    raw = subprocess.run(["adb", "-s", SERIAL, "exec-out", "screencap"], capture_output=True).stdout
    w, h = int.from_bytes(raw[0:4], "little"), int.from_bytes(raw[4:8], "little")
    at = len(raw) - w * h * 4 + (y * w + x) * 4
    return tuple(raw[at:at + 3])


def tap(label, xml=None):
    at = s.node(xml or s.screen(), label)
    if at:
        s.adb("shell", "input", "tap", *map(str, at))
    return at


class Tools:
    """Libri's tools at the person's scope, as a turn's harness reaches them."""

    def __init__(self, libri, env, state):
        self.proc = subprocess.Popen([libri, "tools", str(state)], env=env | {"LIBRI_SCOPE": "person"},
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.id = 0
        self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                "clientInfo": {"name": "check", "version": "0"}})
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.proc.stdin.flush()

    def rpc(self, method, params):
        self.id += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        return json.loads(self.proc.stdout.readline())

    def call(self, name, arguments):
        r = self.rpc("tools/call", {"name": name, "arguments": arguments})["result"]
        return r["content"][0]["text"], r.get("isError", False)


def serve_page():
    class Page(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.end_headers()
            self.wfile.write(PAGE)

        def log_message(self, *_):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Page)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{server.server_address[1]}/"


def main():
    t = None
    booted = False
    # Chromium's lock socket goes in the temporary folder, and a socket's
    # path has room for 107 bytes: the throwaway's own folder, deep in a
    # deep TMPDIR, can leave too few.
    short = tempfile.mkdtemp(prefix="c")
    try:
        t = Throwaway(LIBRI_DIR, prefix="libri-android-browser-", extra={
            "TMPDIR": short,
            "LIBRI_AGENT_BROWSER": "true",
            "LIBRI_CHROMIUM": os.environ["CHECK_CHROMIUM"],
            "LIBRI_XVFB": os.environ["CHECK_XVFB"],
        })
        tools = Tools(t.libri, t.env, t.root / "state")
        page = serve_page()
        booted = s.boot()
        s.adb("uninstall", s.APP)
        s.adb("install", APK)
        s.adb("reverse", f"tcp:{t.port}", f"tcp:{t.port}")
        prefs = f'<?xml version="1.0" encoding="utf-8" standalone="yes" ?>\n<map>\n<string name="address">{t.base}</string>\n</map>\n'
        (t.root / "work/libri.xml").write_text(prefs)
        s.adb("push", str(t.root / "work/libri.xml"), "/data/local/tmp/libri.xml")
        s.adb("shell", "run-as", s.APP, "sh", "-c", "'mkdir -p shared_prefs && cp /data/local/tmp/libri.xml shared_prefs/libri.xml'")
        import urllib.request
        req = urllib.request.Request(f"{t.base}/api/phone-layout", json.dumps({"spaces": [
            {"id": "s1", "name": "Main", "items": [{"kind": "chat"}, {"kind": "browser"}]},
        ], "on": "s1"}).encode(), {"content-type": "application/json"})
        urllib.request.urlopen(req, timeout=10)
        for theme in ("light", "dark"):
            s.adb("shell", "cmd", "uimode", "night", "yes" if theme == "dark" else "no")
            s.adb("shell", "am", "force-stop", s.APP)
            s.adb("shell", "am", "start", "-W", "-n", s.ACTIVITY)
            bar = s.waited(lambda x: 'text="Browser"' in x or 'content-desc="Browser"' in x, 60)
            check(f"{theme}: the bar offers the Browser", bar is not None)
            if not bar:
                continue
            tap("Browser", bar)
            said, bad = tools.call("browser_open", {"url": page})
            check(f"{theme}: the agent opens the page", not bad and "Not pressed" in said, said[:200])
            print("page:", [l for l in said.splitlines() if "viewport" in l], flush=True)
            shown = s.waited(lambda x: "Android page" in x, 30)
            check(f"{theme}: the place shows the agent's tab", shown is not None)
            time.sleep(2)
            shot(f"android-browser-{theme}-agent")
            # The page fills the place: its colour, not white or the paper,
            # at the place's right edge and just above the bar.
            size = s.adb("shell", "wm", "size").split()[-1].split("x")
            w, h = int(size[0]), int(size[1])
            bar = s.node(s.screen(), "Browser")
            low = (bar[1] - 200) if bar else int(h * 0.8)
            edge, foot = pixel(w - 12, h // 2), pixel(w - 12, low)
            page_colour = lambda c: not all(v > 200 for v in c)
            check(f"{theme}: the page reaches the place's right edge", page_colour(edge), edge)
            check(f"{theme}: the page reaches down to the bar", page_colour(foot) and foot[2] > foot[0], foot)
            # The person's tap, in the middle of the page, presses the button.
            size = s.adb("shell", "wm", "size").split()[-1].split("x")
            w, h = int(size[0]), int(size[1])
            s.adb("shell", "input", "tap", str(w // 2), str(h // 2))
            time.sleep(1.5)
            # A sideways drag over the page stays on the Browser.
            s.adb("shell", "input", "swipe", str(w // 5), str(h // 2), str(w * 4 // 5), str(h // 2 + 20), "250")
            time.sleep(1.5)
            check(f"{theme}: a drag over the page leaves the Browser shown",
                  s.waited(lambda x: "Android page" in x and "Message Libri" not in x, 8) is not None)
            shot(f"android-browser-{theme}-pressed")
            hand = s.waited(lambda x: "Hand back" in x, 5)
            if hand:
                tap("Hand back", hand)
                time.sleep(1)
            said, bad = tools.call("browser_look", {})
            check(f"{theme}: the person's tap reached the page", not bad and "Pressed" in said, said[:200])
        crash = s.adb("logcat", "-d", "-s", "AndroidRuntime:E")
        check("no crash", "FATAL EXCEPTION" not in crash, crash[-400:])
    finally:
        s.adb("shell", "cmd", "uimode", "night", "no")
        if t:
            # The daemon's log, beside the screenshots, for a failed run.
            SHOTS.mkdir(parents=True, exist_ok=True)
            (SHOTS / "libri.log").write_bytes((t.root / "libri.log").read_bytes())
            t.close()
        if booted:
            s.adb("emu", "kill")
        shutil.rmtree(short, ignore_errors=True)
    print(f"{len(failures)} failed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
