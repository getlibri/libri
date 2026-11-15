#!/usr/bin/env python3
"""A watch's card in the Workers tile's Watches tab opens in place on a tap
to show everything it holds (the whole text, when and how often, its look
and tests, who it goes to and is shared with, on or off) and closes on a
second tap, each within the 400 ms every interaction must keep to. A hold
or right click still opens the card's menu and leaves it as it was. Runs the
built site against a stand-in host in Chromium at 390 and 1440 in both
themes. Needs Playwright and a browser, so it stays out of `nix flake check`.
Usage: check_watch_details.py SITE_DIR SHOTS_DIR
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
LIMIT_MS = 400
failures = []


def check(what, ok, detail=""):
    print(f"{'PASS' if ok else 'FAIL'} {what} {'' if ok else str(detail)[:300]}", flush=True)
    if not ok:
        failures.append(what)


sys.path.insert(0, str(Path(__file__).resolve().parent))
_argv, sys.argv = sys.argv, [sys.argv[0], str(SITE), "serve"]
import measure_ui_performance as stand_in  # noqa: E402
sys.argv = _argv

LONG = (
    "Look through the fares to Lisbon each half hour and tell me of any under two hundred, "
    "with the airline, the times and the link, so I can book before it goes."
)
WATCHES = [
    {
        "id": 7, "active": True, "at": int(time.time()) + 5400, "every": "30m", "kind": "ask", "text": LONG,
        "worker": "Wren", "looks": "travel fares", "arguments": '{"to":"LIS"}', "failing": None,
        "tests": ["price under 200"], "fields": ["price", "airline"], "shared": ["Ada"], "pushed": False,
    },
    {
        "id": 8, "active": False, "at": None, "every": None, "kind": "note", "text": "A build finished.",
        "worker": None, "looks": None, "failing": None, "tests": [], "fields": [], "shared": [], "pushed": True,
    },
]
WANT = {
    7: ["Ask", LONG, "Next", "Repeats", "Every 30m", "Looks at", 'travel fares {"to":"LIS"}', "Tests", "price under 200",
        "Sees", "price, airline", "Goes to", "Wren's thread", "Shared with", "Ada", "State", "On"],
    8: ["Note", "When a push arrives", "Once", "The main chat", "Off"],
}


class Host(stand_in.Host):
    """The stand-in host with watches that hold everything; the layout
    stores answer empty, and a wait for their next version waits."""

    def do_GET(self):
        path = urlparse(self.path).path
        try:
            if path.endswith("/api/watches"):
                return self.json({"watches": WATCHES})
            if path.endswith("/api/layout") or path.endswith("/api/phone-layout"):
                if "since=" in self.path:
                    time.sleep(5)
                return self.json({"version": 0, "layout": None, "by": None, "undo": False})
        except (BrokenPipeError, ConnectionResetError):
            return
        return super().do_GET()


def to_watches(page):
    page.goto(page.url.split("#")[0] + "#work")
    page.wait_for_selector("#workers-tab-tree, [data-item=workers]", state="attached")
    page.wait_for_timeout(800)
    if not page.locator("#workers-tab-tree").is_visible():
        page.locator("[data-item=workers]").first.click()
        page.wait_for_selector("#workers-tab-tree")
    page.locator("#workers-tab-watches").click()
    page.wait_for_selector("[data-watch='7']")


def timed(page, card, want_open):
    """Taps the card and times, in the page, from the tap to the frame
    that shows it opened or closed."""
    page.evaluate("""([id, open]) => { window.__flip = new Promise((done) => {
        const card = document.querySelector(`[data-watch='${id}']`);
        const t0 = performance.now();
        card.addEventListener('click', () => requestAnimationFrame(() => requestAnimationFrame(() => {
          const shown = !!card.querySelector('[data-watch-details]');
          done({ ms: performance.now() - t0, ok: shown === open });
        })), { once: true });
      }); }""", [card, want_open])
    page.locator(f"[data-watch='{card}']").click()
    return page.evaluate("window.__flip")


def run(browser, site, width, height, mobile, theme):
    tag = f"{width} {theme}"
    context = browser.new_context(viewport={"width": width, "height": height}, service_workers="block", is_mobile=mobile, has_touch=mobile)
    context.set_default_timeout(10000)
    if theme == "dark":
        context.add_init_script("localStorage.setItem('theme', 'dark')")
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(site + "/")
    to_watches(page)
    page.wait_for_timeout(300)
    check(f"{tag}: cards start closed", page.locator("[data-watch-details]").count() == 0)
    check(f"{tag}: a closed card says it is closed", page.locator("[data-watch='7']").get_attribute("aria-expanded") == "false")
    page.screenshot(path=str(SHOTS / f"watch-{width}-{theme}-closed.png"))

    for card in (7, 8):
        flip = timed(page, card, True)
        check(f"{tag}: watch {card} opens in place", flip["ok"], flip)
        check(f"{tag}: watch {card} opens in {flip['ms']:.0f} ms (< {LIMIT_MS})", flip["ms"] < LIMIT_MS, flip)
        words = page.locator(f"[data-watch='{card}'] [data-watch-details]").inner_text()
        missing = [w for w in WANT[card] if w not in words]
        check(f"{tag}: watch {card} shows all it holds", not missing, missing)
    check(f"{tag}: an opened card says it is open", page.locator("[data-watch='7']").get_attribute("aria-expanded") == "true")
    box = page.locator("[data-watch='7']").bounding_box()
    check(f"{tag}: the opened card stays inside the window", box["x"] >= 0 and box["x"] + box["width"] <= width, box)
    page.screenshot(path=str(SHOTS / f"watch-{width}-{theme}-open.png"), full_page=False)

    for card in (7, 8):
        flip = timed(page, card, False)
        check(f"{tag}: watch {card} closes on a second tap", flip["ok"], flip)
        check(f"{tag}: watch {card} closes in {flip['ms']:.0f} ms (< {LIMIT_MS})", flip["ms"] < LIMIT_MS, flip)

    if not mobile:
        page.locator("[data-watch='7']").click(button="right")
        page.wait_for_timeout(300)
        check(f"{tag}: a right click opens the menu", page.get_by_role("menuitem", name="Delete").count() > 0)
        check(f"{tag}: a right click leaves the card closed", page.locator("[data-watch-details]").count() == 0)
        page.keyboard.press("Escape")
        page.wait_for_timeout(200)
        page.locator("[data-watch='7']").focus()
        page.keyboard.press("Enter")
        page.wait_for_timeout(100)
        check(f"{tag}: Enter opens a focused card", page.locator("[data-watch='7'] [data-watch-details]").count() == 1)
        page.keyboard.press("Enter")
    check(f"{tag}: no page errors", not errors, errors)
    context.close()


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Host, directory=str(SITE)))
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for width, height, mobile in ((1440, 900, False), (390, 844, True)):
            for theme in ("light", "dark"):
                run(browser, site, width, height, mobile, theme)
        browser.close()
    print(f"{len(failures)} failed")
    sys.exit(1 if failures else 0)


main()
