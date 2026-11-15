#!/usr/bin/env python3
"""Libri's browser end to end: an agent opens a page in Libri's own browser;
the person sees it live in the browser card, takes control, presses the
page's button with a tap on the picture, and hands it back, and the agent
reads what the person did. Then an agent hands the browser over and its
call waits until the person hands it back. At desktop (1440) and phone
(390, touch) sizes.

Drives the real daemon, its real tool server (as a turn's harness reaches
it) and its real Chromium on Xvfb, in a scratch directory, and the built
site in Playwright's Chromium. Needs Playwright, Chromium and Xvfb, so it
stays out of `nix flake check`.
Usage: check_browser.py LIBRI_DIR (holding bin/libri and site/) [SHOTS_DIR];
CHECK_CHROMIUM and CHECK_XVFB name the agents' browser and display, else
`chromium` and `Xvfb` on PATH.
"""

import http.server
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

LIBRI = Path(sys.argv[1]) / "bin/libri"
SITE = Path(sys.argv[1]) / "site"
SHOTS = Path(sys.argv[2]) if len(sys.argv) > 2 else None

# One button over the whole page, so a tap anywhere on the picture lands on
# it at either size; its label says how many times it was pressed.
TEST_PAGE = b"""<!doctype html><title>Press page</title>
<meta name="viewport" content="width=device-width">
<style>html,body{margin:0;height:100%}button{width:100%;height:100%;font-size:40px}</style>
<button id="b" onclick="this.textContent='Pressed '+(++n)">Not pressed</button>
<script>let n=0</script>"""

failures = []


def check(what, ok, detail=""):
    print(f"ok   {what}" if ok else f"FAIL {what} {detail}", flush=True)
    if not ok:
        failures.append(what)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def until(test, what, limit=30.0):
    end = time.time() + limit
    while time.time() < end:
        if test():
            return True
        time.sleep(0.1)
    check(what, False, "timed out")
    return False


class Tools:
    """Libri's tools at the person's scope, as a turn's harness reaches them."""

    def __init__(self, env, state):
        self.proc = subprocess.Popen([LIBRI, "tools", str(state)], env=env | {"LIBRI_SCOPE": "person"},
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.id = 0
        self.lock = threading.Lock()
        self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                "clientInfo": {"name": "check", "version": "0"}})
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.proc.stdin.flush()

    def rpc(self, method, params):
        with self.lock:
            self.id += 1
            self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n")
            self.proc.stdin.flush()
            return json.loads(self.proc.stdout.readline())

    def call(self, name, arguments):
        r = self.rpc("tools/call", {"name": name, "arguments": arguments})["result"]
        return r["content"][0]["text"], r.get("isError", False)


def card(page):
    """The browser card's own frame, inside the sandbox proxy."""
    for f in page.frames:
        try:
            if f.query_selector("#pic") is not None:
                return f
        except Exception:
            pass
    return None


def text(page, sel):
    f = card(page)
    return f.inner_text(sel) if f else ""


def serve_test_page():
    class Page(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.end_headers()
            self.wfile.write(TEST_PAGE)

        def log_message(self, *_):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Page)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{server.server_address[1]}/"


def run_size(p, url, tools, test_page, name, viewport, phone):
    said, bad = tools.call("browser_open", {"url": test_page})
    check(f"{name}: the agent opens the page in Libri's browser", not bad and "Not pressed" in said, said)
    b = p.chromium.launch()
    ctx = b.new_context(viewport=viewport, is_mobile=phone, has_touch=phone,
                        device_scale_factor=2 if phone else 1)
    page = ctx.new_page()
    page.goto(f"{url}/#card=mcp__libri__browser_open")
    if not until(lambda: card(page) is not None and card(page).evaluate(
            "() => !document.getElementById('pic').hidden && document.getElementById('pic').src.startsWith('data:image/jpeg')"),
            f"{name}: the card shows the browser's live picture", 30):
        print("  frames:", [fr.url[:80] for fr in page.frames], "page:", page.inner_text("body")[:300])
        if SHOTS:
            page.screenshot(path=SHOTS / f"{name}-failed.png")
        return
    check(f"{name}: the card shows the browser's live picture", True)
    f = card(page)
    check(f"{name}: the card says the agent is browsing", "The agent is browsing" in f.inner_text("#state"), f.inner_text("#state"))
    pic = f.query_selector("#pic").bounding_box()
    check(f"{name}: the picture fits the screen", pic and pic["x"] >= 0 and pic["x"] + pic["width"] <= viewport["width"] + 1, pic)
    if SHOTS:
        page.screenshot(path=SHOTS / f"{name}-1-watching.png")

    start = time.time()
    f.click("#take")
    if until(lambda: "You have control" in text(page, "#state"), f"{name}: Take control gives the person the browser", 10):
        took = time.time() - start
        check(f"{name}: Take control answers within 400 ms", took < 0.4, f"{took:.2f}s")
    said, bad = tools.call("browser_look", {})
    check(f"{name}: the agent is held off while the person has it", bad and "person has the browser" in said, said)
    f = card(page)
    box = f.query_selector("#pic").bounding_box()
    x, y = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
    if phone:
        page.touchscreen.tap(x, y)
    else:
        page.mouse.click(x, y)
    # The picture follows the page: the button's new label is painted.
    seen = {"at": 0}

    def repainted():
        src = card(page).evaluate("() => document.getElementById('pic').src.length")
        changed = seen["at"] and src != seen["at"]
        seen["at"] = src
        return changed
    until(repainted, f"{name}: the picture repaints after the person's tap", 10)
    if SHOTS:
        page.screenshot(path=SHOTS / f"{name}-2-control.png")
    card(page).click("#take")
    until(lambda: "The agent is browsing" in text(page, "#state"), f"{name}: Hand back returns the browser to the agent", 10)
    said, bad = tools.call("browser_look", {})
    check(f"{name}: the agent reads what the person did", not bad and "Pressed 1" in said, said)
    ctx.close()
    b.close()


def hand_over(p, url, tools):
    out = {}

    def ask():
        out["said"], out["bad"] = tools.call("browser_hand_over", {"reason": "Press the button for me"})
    t = threading.Thread(target=ask)
    t.start()
    b = p.chromium.launch()
    page = b.new_page(viewport={"width": 1440, "height": 900})
    page.goto(f"{url}/#card=mcp__libri__browser_open")
    until(lambda: "Waiting on you: Press the button for me" in text(page, "#state"), "hand-over: the card says the agent waits on the person", 20)
    card(page).click("#take")
    until(lambda: "You have control" in text(page, "#state"), "hand-over: the person takes it", 10)
    card(page).click("#take")
    t.join(20)
    check("hand-over: the agent's call returns once the person hands it back",
          not t.is_alive() and not out.get("bad") and "handed the browser back" in out.get("said", ""), out)
    b.close()


def main():
    chromium = os.environ.get("CHECK_CHROMIUM") or shutil.which("chromium")
    xvfb = os.environ.get("CHECK_XVFB") or shutil.which("Xvfb")
    if not chromium or not xvfb:
        raise SystemExit("needs chromium and Xvfb (CHECK_CHROMIUM, CHECK_XVFB)")
    if SHOTS:
        SHOTS.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="libri-browser-"))
    for d in ("home", "state", "hippo"):
        (root / d).mkdir()
    port = free_port()
    url = f"http://127.0.0.1:{port}"
    env = {
        "HOME": str(root / "home"),
        "PATH": os.environ["PATH"],
        "HIPPO_DIR": str(root / "hippo"),
        "STATE_DIRECTORY": str(root / "state"),
        "LIBRI_NAME": "box-label",
        "LIBRI_PERSON": "Ada",
    }
    daemon = subprocess.Popen([LIBRI], env=env | {
        "LIBRI_BACKEND": "claude",
        "LIBRI_CLAUDE": "/bin/false",
        "LIBRI_MODEL": "stand-in",
        "LIBRI_SITE": f"127.0.0.1:{port}",
        "LIBRI_SITE_URL": url,
        "LIBRI_SITE_ROOT": str(SITE),
        "LIBRI_OPEN": "1",
        "LIBRI_AGENT_BROWSER": "true",
        "LIBRI_CHROMIUM": chromium,
        "LIBRI_XVFB": xvfb,
    }, stdout=subprocess.DEVNULL, stderr=open(root / "libri.log", "w"))

    def up():
        try:
            return urllib.request.urlopen(f"{url}/api/host", timeout=2).status == 200
        except OSError:
            return False

    try:
        until(up, "libri up")
        until(lambda: (root / "state/browser.sock").exists(), "the browser's socket")
        tools = Tools(env, root / "state")
        test_page = serve_test_page()
        with sync_playwright() as p:
            run_size(p, url, tools, test_page, "1440", {"width": 1440, "height": 900}, False)
            run_size(p, url, tools, test_page, "390", {"width": 390, "height": 844}, True)
            hand_over(p, url, tools)
    finally:
        daemon.terminate()
        daemon.wait(10)
        print(f"log: {root / 'libri.log'}")
    if failures:
        raise SystemExit(f"{len(failures)} failed: {failures}")
    print("all passed")


main()
