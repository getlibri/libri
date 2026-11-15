#!/usr/bin/env python3
"""The Browser tile end to end: the person browses Libri's own browser in the
tile (address bar, clicks, keys, tabs, a download, a file chooser), an agent
browses the same browser through its tools and the tile shows it live, the
person's own tabs stay closed to an agent without their logins, and handing
back gives the agent the page the person left. Measures input to repaint.
At desktop (1440) and phone sizes (390 at 2x, 411 at 2.625x, touch).

Drives the real daemon, its tool server and its Chromium on Xvfb in a
scratch directory, and the built site's `#browser-tile` page in Playwright.
Needs Playwright, Chromium and Xvfb, so it stays out of `nix flake check`.
Usage: check_browser_tile.py LIBRI_DIR (holding bin/libri and site/) [SHOTS_DIR];
CHECK_CHROMIUM and CHECK_XVFB name the agents' browser and display, else
`chromium` and `Xvfb` on PATH.

Input to repaint here is over loopback. Over the tailnet it adds about one
round trip (`tailscale ping` from the host to the device) and the frame's
transfer. To measure it whole, the viewing browser must run on the other
host: there, `playwright run-server --host <its tailnet address>`; here,
the daemon listening on this host's tailnet address (LIBRI_SITE) and the
check's Playwright connecting with `chromium.connect("ws://<host>:<port>/")`
instead of launching its own.
"""

import http.server
import json
import os
import shutil
import socket
import statistics
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

# A page whose every part says what happened to it, so the agent's look
# reads what the person did in the tile: a big button that counts presses
# and turns the page a new colour each time, a field, a file input that
# names what it got, a download and a link to a new tab.
MAIN = b"""<!doctype html><title>Tile page</title>
<meta name="viewport" content="width=device-width">
<style>html,body{margin:0;height:100%;font:20px sans-serif}
#b{width:100%;height:55%;font-size:36px;border:0}
#f{font-size:20px;margin:8px}</style>
<button id="b" onclick="n++;this.textContent='Pressed '+n;this.style.background=['#e66','#6e6','#66e','#ee6'][n%4]">Not pressed</button>
<input id="f" placeholder="Type here" aria-label="Field">
<input id="u" type="file" aria-label="Upload" onchange="document.getElementById('got').textContent='Got '+[...this.files].map(f=>f.name).join(',')">
<span id="got">Nothing chosen</span>
<a id="d" href="/file.txt">Download the file</a>
<a id="t" href="/two" target="_blank">Another tab</a>
<script>let n=0</script>"""
# Green to its edges, so a picture that leaves part of the tile white shows;
# a count that keeps it painting, so the cast, not one screenshot, draws it.
TWO = b"<!doctype html><title>Second page</title><meta name=\"viewport\" content=\"width=device-width\"><style>#g{position:fixed;inset:0;background:#3c6}</style><div id=g><p>The second page</p><p id=t></p></div><script>let i=0;setInterval(()=>t.textContent=++i,250)</script>"
FILE = b"downloaded words\n"

failures = []
timings = []


def check(what, ok, detail=""):
    print(f"ok   {what}" if ok else f"FAIL {what} {detail}", flush=True)
    if not ok:
        failures.append(what)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def until(test, what, limit=30.0, quiet=False):
    end = time.time() + limit
    while time.time() < end:
        try:
            if test():
                return True
        except Exception:
            pass
        time.sleep(0.05)
    if not quiet:
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


def serve_pages():
    class Page(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            body, kind, extra = MAIN, "text/html", {}
            if self.path == "/two":
                body = TWO
            elif self.path == "/file.txt":
                body, kind, extra = FILE, "text/plain", {"Content-Disposition": 'attachment; filename="tile-file.txt"'}
            self.send_response(200)
            self.send_header("Content-Type", kind)
            for k, v in extra.items():
                self.send_header(k, v)
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Page)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{server.server_address[1]}/"


def canvas_mark(page):
    """A cheap fingerprint of what the tile's canvas shows now."""
    return page.evaluate("""() => {const c=document.querySelector('canvas[aria-label="The page"]');
      if(!c||!c.width)return '';const x=c.getContext('2d');const d=x.getImageData(0,0,c.width,Math.min(c.height,c.height)).data;
      let h=0;for(let i=0;i<d.length;i+=997)h=(h*31+d[i])|0;return c.width+'x'+c.height+':'+h}""")


def corner(page):
    """The colour of the canvas's bottom right corner, a few pixels in."""
    return page.evaluate("""() => {const c=document.querySelector('canvas[aria-label="The page"]');
      const d=c.getContext('2d').getImageData(c.width-6,c.height-6,1,1).data;return [d[0],d[1],d[2]]}""")


def tabs(page):
    return page.locator("[role=tab]").count()


def status_text(page):
    return page.inner_text("form")


def press_and_time(page, phone, name):
    """Presses the page's button through the tile; answers the time from the
    tap to the canvas showing the change."""
    box = page.locator('canvas[aria-label="The page"]').bounding_box()
    x, y = box["x"] + box["width"] / 2, box["y"] + box["height"] * 0.25
    before = canvas_mark(page)
    start = time.time()
    if phone:
        page.touchscreen.tap(x, y)
    else:
        page.mouse.click(x, y)
    if until(lambda: canvas_mark(page) != before, f"{name}: the tile repaints after the person's press", 10):
        took = time.time() - start
        timings.append((name, took))
        return took
    return None


def run_size(p, url, tools, pages, name, viewport, phone, ratio=1):
    b = p.chromium.launch()
    ctx = b.new_context(viewport=viewport, is_mobile=phone, has_touch=phone,
                        device_scale_factor=ratio, accept_downloads=True)
    page = ctx.new_page()
    page.goto(f"{url}/#browser-tile")
    until(lambda: page.get_by_text("Start browsing").is_visible() or tabs(page) > 0, f"{name}: the tile connects", 20)
    if page.get_by_text("Start browsing").is_visible():
        page.get_by_text("Start browsing").click()
        until(lambda: tabs(page) == 1, f"{name}: Start browsing opens one tab", 30)
    else:
        n = tabs(page)
        page.get_by_role("button", name="New tab").click()
        until(lambda: tabs(page) == n + 1, f"{name}: New tab opens a tab", 15)
    first = tabs(page)

    # The address bar opens the page; the tile paints it at its own size.
    page.get_by_label("Address").fill(pages)
    page.get_by_label("Address").press("Enter")
    check(f"{name}: the address bar opens the page",
          until(lambda: "Tile page" in page.inner_text("[role=tablist]"), f"{name}: tab title", 20, quiet=True),
          page.inner_text("[role=tablist]"))
    until(lambda: canvas_mark(page) != "", f"{name}: the tile draws the page", 20)
    time.sleep(0.5)

    # The person's press reaches the page, and the tile repaints.
    took = press_and_time(page, phone, name)
    if took is not None:
        check(f"{name}: press to repaint {took * 1000:.0f} ms (under 400)", took < 0.4)
    c = page.locator('canvas[aria-label="The page"]').bounding_box()
    w = page.evaluate("() => document.querySelector('canvas[aria-label=\"The page\"]').width")
    # A picture pixel per screen pixel, up to the 2x the browser draws at.
    want = c["width"] * min(ratio, 2)
    check(f"{name}: the page is cast at the screen's pixels ({w} for {want:.0f})", w >= want - 8, (w, c))
    if SHOTS:
        page.screenshot(path=SHOTS / f"{name}-1-page.png")
    check(f"{name}: the person holds the browser while browsing", "You" in status_text(page), status_text(page))
    said, bad = tools.call("browser_look", {})
    check(f"{name}: the agent is held off while the person browses", bad and "person has the browser" in said, said)

    # Keys reach the page: typed into the field.
    box = page.locator('canvas[aria-label="The page"]').bounding_box()
    fx, fy = box["x"] + 100, box["y"] + box["height"] * 0.58
    if phone:
        page.touchscreen.tap(fx, fy)
        page.keyboard.insert_text("hello tile")
    else:
        page.mouse.click(fx, fy)
        page.keyboard.type("hello tile")
    time.sleep(0.5)
    if SHOTS:
        page.screenshot(path=SHOTS / f"{name}-2-typed.png")

    # Hand back: the agent reads the person's own tab, as they left it.
    page.get_by_role("button", name="Hand back").click()
    until(lambda: "Agent" in status_text(page), f"{name}: Hand back gives the browser to the agents", 10)
    said, bad = tools.call("browser_look", {})
    check(f"{name}: the agent reads the person's press", not bad and "Pressed 1" in said, said[:300])
    check(f"{name}: the agent reads what the person typed", not bad and "hello tile" in said, said[:400])
    said, bad = tools.call("browser_scroll", {"down": True})
    check(f"{name}: an agent can't act in the person's own tab", bad and "person's own" in said, said[:200])

    # An agent's open takes a tab of its own, which shows live in the tile.
    before = canvas_mark(page)
    said, bad = tools.call("browser_open", {"url": pages + "two"})
    check(f"{name}: the agent opens a page in a tab of its own", not bad and "second page" in said and tabs(page) == first + 1,
          (said[:120], tabs(page)))
    check(f"{name}: the agent's page shows live in the tile", until(lambda: canvas_mark(page) != before, "live", 10, quiet=True))
    check(f"{name}: the tile follows the agent's address", until(lambda: "two" in page.get_by_label("Address").input_value(), "addr", 5, quiet=True),
          page.get_by_label("Address").input_value())
    # The page fills the tile: its green reaches the far corner.
    green = lambda: (lambda c: c[1] > 150 and c[0] < 120 and c[2] < 160)(corner(page))
    check(f"{name}: the page fills the tile to its corner", until(green, "corner", 5, quiet=True), corner(page))
    if SHOTS:
        page.screenshot(path=SHOTS / f"{name}-3-agent.png")
    said, bad = tools.call("browser_tab", {"tab": 1})
    check(f"{name}: an agent switches tabs and reads the other", not bad and "Tile page" in said, said[:200])

    # An agent's tab made while no view is open, as a phone's place opens
    # after the agent began, fills the view that opens later.
    page.goto("about:blank")
    said, bad = tools.call("browser_tab", {"tab": 0, "new": True})
    said, bad = tools.call("browser_open", {"url": pages + "two"})
    page.goto(f"{url}/#browser-tile")
    until(lambda: canvas_mark(page) != "", f"{name}: the tile draws again", 20)
    check(f"{name}: a tab made with no view open fills the view", until(green, "corner", 8, quiet=True), corner(page))
    if SHOTS:
        page.screenshot(path=SHOTS / f"{name}-3b-later.png")

    # The tile's own tabs: a new one, shown; closed again.
    n = tabs(page)
    page.get_by_role("button", name="New tab").click()
    check(f"{name}: New tab makes a tab", until(lambda: tabs(page) == n + 1, "new tab", 15, quiet=True), tabs(page))
    while tabs(page) > 1:
        n = tabs(page)
        page.get_by_role("button", name="Close tab").last.click()
        if not until(lambda: tabs(page) == n - 1, f"{name}: Close tab closes it", 10):
            break
    if SHOTS:
        page.screenshot(path=SHOTS / f"{name}-4-closed.png")
    if page.get_by_role("button", name="Hand back").count():
        page.get_by_role("button", name="Hand back").click()
    ctx.close()
    b.close()


def files(p, url, pages, state):
    """An agent's tab downloads into the person's files; its file input asks
    the person's screen, and the page gets the file they chose."""
    b = p.chromium.launch()
    page = b.new_page(viewport={"width": 1280, "height": 900})
    page.goto(f"{url}/#browser-tile")
    until(lambda: tabs(page) >= 1, "files: the tile shows a tab", 20)
    said, bad = TOOLS.call("browser_tab", {"new": True})
    said, bad = TOOLS.call("browser_open", {"url": pages})
    check("files: the agent opens the page", not bad and "Tile page" in said, said[:200])
    ref = next((l.split("]")[0][1:] for l in said.splitlines() if "Download the file" in l), None)
    if ref:
        TOOLS.call("browser_click", {"ref": int(ref)})
    check("files: a download lands in the person's files",
          until(lambda: (state / "files" / "tile-file.txt").exists(), "download", 15, quiet=True),
          list((state / "files").glob("*")) if (state / "files").exists() else "no files folder")
    check("files: the tile says where the download went",
          until(lambda: "Saved to your files" in page.inner_text("body"), "toast", 5, quiet=True))
    said = TOOLS.call("browser_look", {})[0]
    ref = next((l.split("]")[0][1:] for l in said.splitlines() if "Upload" in l), None)
    upload = Path(tempfile.mkdtemp()) / "chosen.txt"
    upload.write_text("chosen words")
    if ref:
        TOOLS.call("browser_click", {"ref": int(ref)})
    if until(lambda: page.get_by_role("button", name="Choose…").is_visible(), "files: the tile offers the page's file chooser", 10):
        with page.expect_file_chooser(timeout=10000) as fc:
            page.get_by_role("button", name="Choose…").click()
        fc.value.set_files(str(upload))
        check("files: the page gets the file the person chose",
              until(lambda: "Got chosen.txt" in TOOLS.call("browser_look", {})[0], "chosen", 10, quiet=True))
    if SHOTS:
        page.screenshot(path=SHOTS / "files.png")
    b.close()


def main():
    global TOOLS
    chromium = os.environ.get("CHECK_CHROMIUM") or shutil.which("chromium")
    xvfb = os.environ.get("CHECK_XVFB") or shutil.which("Xvfb")
    if not chromium or not xvfb:
        raise SystemExit("needs chromium and Xvfb (CHECK_CHROMIUM, CHECK_XVFB)")
    if SHOTS:
        SHOTS.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="libri-browser-tile-"))
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
        TOOLS = Tools(env, root / "state")
        pages = serve_pages()
        with sync_playwright() as p:
            only = os.environ.get("CHECK_ONLY", "1440,390,411,files").split(",")
            if "1440" in only:
                run_size(p, url, TOOLS, pages, "1440", {"width": 1440, "height": 900}, False)
            if "390" in only:
                run_size(p, url, TOOLS, pages, "390", {"width": 390, "height": 844}, True, 2)
            # A common Android screen, finer than the 2x the browser draws at.
            if "411" in only:
                run_size(p, url, TOOLS, pages, "411", {"width": 411, "height": 914}, True, 2.625)
            if "files" in only:
                files(p, url, pages, root / "state")
    finally:
        daemon.terminate()
        daemon.wait(10)
        print(f"log: {root / 'libri.log'}")
    if timings:
        print("press to repaint: " + ", ".join(f"{n} {t * 1000:.0f} ms" for n, t in timings)
              + f"; median {statistics.median(t for _, t in timings) * 1000:.0f} ms")
    if failures:
        raise SystemExit(f"{len(failures)} failed: {failures}")
    print("all passed")


TOOLS = None
main()
