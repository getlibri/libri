#!/usr/bin/env python3
"""The phone's side panel closes every way it offers, in WebKit and Chromium.

On a phone the panel slides the page right and leaves a strip of it in view.
A tap on that strip, or a swipe left across it, closes the panel; a swipe the
other way leaves it open; Escape closes it too. On 0.9.73 none of the strip's
ways worked in either engine: the scrim stood inside the page's own box, which
is inert while the panel is open, and an inert box takes no tap. An iPhone
(Safari) had no way back out.

Runs the built site against the stand-in host (measure_ui_performance.py) at
390x844 with touch, in WebKit and Chromium, each way on a fresh page, and
leaves a picture of the open panel per engine. Needs Playwright with both
browsers, so it stays out of `nix flake check`.
Usage: check_side_panel.py SITE_DIR SHOTS_DIR
"""
import functools
import http.server
import sys
import threading
import time
from pathlib import Path
from urllib.parse import urlparse

from playwright.sync_api import sync_playwright

SITE, SHOTS = Path(sys.argv[1]), Path(sys.argv[2])
failures = []
total = 0


def check(what, ok, detail=""):
    global total
    total += 1
    print(("ok   " if ok else "FAIL ") + what + ("" if ok or not detail else f": {detail}"), flush=True)
    if not ok:
        failures.append(what)


sys.path.insert(0, str(Path(__file__).resolve().parent))
_argv, sys.argv = sys.argv, [sys.argv[0], str(SITE), "serve"]
import measure_ui_performance as stand_in  # noqa: E402

sys.argv = _argv

LAYOUT = {"spaces": [{"id": "main", "name": "Main", "items": [{"kind": "chat"}, {"kind": "workers"}, {"kind": "temp"}]}], "on": "main"}


class Host(stand_in.Host):
    """The layout stores answer once; a wait for their next version waits."""

    def do_GET(self):
        path = urlparse(self.path).path
        if path.endswith("/api/layout") or path.endswith("/api/phone-layout"):
            if "since=" in self.path:
                time.sleep(5)
            try:
                return self.json({"version": 1, "layout": LAYOUT, "by": None, "undo": False})
            except (BrokenPipeError, ConnectionResetError):
                return
        return super().do_GET()


# Whether the page stands slid aside for the panel.
OPEN = "() => [...document.querySelectorAll('[inert]')].some((e) => String(e.className).includes('translate-x-[var(--panel-width)]'))"
# A finger's drag across the strip, as pointer events: WebKit's driver has no
# touch drag, and the strip reads pointers.
DRAG = """([x, y, dx]) => {
  const t = document.elementFromPoint(x, y);
  const ev = (type, at) => t.dispatchEvent(new PointerEvent(type, { bubbles: true, pointerType: "touch", pointerId: 7, isPrimary: true, clientX: at, clientY: y }));
  ev("pointerdown", x);
  ev("pointermove", x + dx / 2);
  ev("pointerup", x + dx);
}"""
# Where the strip of page stands beside the open panel.
STRIP = (370, 420)


def way(page, how):
    if how == "a tap on the page beside it":
        page.touchscreen.tap(*STRIP)
    elif how == "a swipe left across the page":
        page.evaluate(DRAG, [*STRIP, -200])
    elif how == "a swipe right across the page":
        page.evaluate(DRAG, [*STRIP, 120])
    else:
        page.keyboard.press("Escape")


WAYS = {
    "a tap on the page beside it": True,
    "a swipe left across the page": True,
    "a swipe right across the page": False,
    "Escape": True,
}


def run(pw, site, engine):
    browser = getattr(pw, engine).launch()
    # WebKit's driver takes no is_mobile; touch is what matters here.
    context = browser.new_context(viewport={"width": 390, "height": 844}, has_touch=True, is_mobile=engine == "chromium", service_workers="block")
    context.set_default_timeout(15000)
    for how, closes in WAYS.items():
        page = context.new_page()
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.goto(site + "/")
        page.wait_for_selector("[data-surface=chat] textarea")
        page.wait_for_timeout(800)
        page.get_by_role("button", name="Side panel").first.tap()
        page.wait_for_timeout(500)
        opened = page.evaluate(OPEN)
        check(f"{engine}: the side panel opens", opened)
        if how == "Escape" and SHOTS:
            page.screenshot(path=str(SHOTS / f"open-{engine}.png"))
        way(page, how)
        page.wait_for_timeout(500)
        shut = not page.evaluate(OPEN)
        check(f"{engine}: {how} {'closes' if closes else 'leaves open'} the panel", shut == closes, f"closed={shut}")
        check(f"{engine} {how}: no page errors", not errors, errors)
        page.close()
    context.close()
    browser.close()


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Host, directory=str(SITE)))
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    with sync_playwright() as pw:
        for engine in ("webkit", "chromium"):
            run(pw, site, engine)
    print(f"{total - len(failures)}/{total} passed")
    sys.exit(1 if failures else 0)


main()
