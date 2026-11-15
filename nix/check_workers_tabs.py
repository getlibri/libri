#!/usr/bin/env python3
"""The Workers tile has three tabs, Workers, Watches and Archived, each
showing only its own: Workers and Watches carry their counts on the label,
Archived carries none there and says its count once opened. Runs the built
site against a stand-in host in Chromium at 390 and 1440 in both themes.
Needs Playwright and a browser, so it stays out of `nix flake check`.
Usage: check_workers_tabs.py SITE_DIR SHOTS_DIR
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


def check(what, ok, detail=""):
    print(f"{'PASS' if ok else 'FAIL'} {what} {'' if ok else str(detail)[:200]}", flush=True)
    if not ok:
        failures.append(what)


# The stand-in host of real size answers the API (measure_ui_performance.py).
sys.path.insert(0, str(Path(__file__).resolve().parent))
_argv, sys.argv = sys.argv, [sys.argv[0], str(SITE), "serve"]
import measure_ui_performance as stand_in  # noqa: E402
sys.argv = _argv

THREADS = stand_in.Host.replies["/api/work"]["threads"]
WATCHES = stand_in.Host.replies["/api/watches"]["watches"]


class Host(stand_in.Host):
    """Answers the layout stores as empty; a wait for their next version
    waits, as the real host's does, so the page does not ask in a loop."""

    def do_GET(self):
        path = urlparse(self.path).path
        if path.endswith("/api/layout") or path.endswith("/api/phone-layout"):
            if "since=" in self.path:
                time.sleep(5)
            try:
                return self.json({"version": 0, "layout": None, "by": None, "undo": False})
            except (BrokenPipeError, ConnectionResetError):
                return
        return super().do_GET()


TABS = """() => [...document.querySelectorAll('[role=tab][id^=workers-tab-]')]
  .map((b) => ({ id: b.id.slice(12), text: b.textContent.trim(), on: b.getAttribute('aria-selected') === 'true' }))"""


def panel(page):
    return page.locator("[role=tabpanel][id^=workers-panel-]").first


def to_workers(page):
    """The page's #work address opens the Workers tile; the phone's bar
    holds it as an item too."""
    page.goto(page.url.split("#")[0] + "#work")
    page.wait_for_selector("#workers-tab-tree, [data-item=workers]", state="attached")
    page.wait_for_timeout(800)
    if not page.locator("#workers-tab-tree").is_visible():
        page.locator("[data-item=workers]").first.click()
        page.wait_for_selector("#workers-tab-tree")


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
    to_workers(page)
    page.wait_for_selector("#workers-tab-tree")
    page.wait_for_timeout(400)
    tabs = page.evaluate(TABS)
    live = sum(t["state"] != "archived" for t in THREADS)
    check(f"{tag}: three tabs in order", [t["id"] for t in tabs] == ["tree", "watches", "archived"], tabs)
    check(f"{tag}: Workers opens first", [t["id"] for t in tabs if t["on"]] == ["tree"], tabs)
    check(f"{tag}: Workers counts the live and sleeping", tabs[0]["text"] == f"Workers{live}", tabs[0]["text"])
    check(f"{tag}: Watches counts the watches", tabs[1]["text"] == f"Watches{len(WATCHES)}", tabs[1]["text"])
    check(f"{tag}: Archived's label has no count", tabs[2]["text"] == "Archived", tabs[2]["text"])
    page.screenshot(path=str(SHOTS / f"workers-{width}-{theme}-tree.png"))

    page.locator("#workers-tab-watches").click()
    page.wait_for_timeout(300)
    words = panel(page).inner_text()
    check(f"{tag}: Watches shows every watch", all(w["text"] in words for w in WATCHES), words[:200])
    check(f"{tag}: Watches shows no archived worker", not any(t["name"] in words for t in THREADS if t["state"] == "archived"), words[:200])
    page.screenshot(path=str(SHOTS / f"workers-{width}-{theme}-watches.png"))

    page.locator("#workers-tab-archived").click()
    page.wait_for_timeout(300)
    words = panel(page).inner_text()
    put = [t["name"] for t in THREADS if t["state"] == "archived"]
    check(f"{tag}: Archived says its count inside", page.locator("[data-archived-count]").inner_text().strip().lower() == f"{len(put)} archived", words[:80])
    check(f"{tag}: Archived shows every archived worker", all(n in words for n in put), words[:200])
    check(f"{tag}: Archived shows no watch", not any(w["text"] in words for w in WATCHES), words[:200])
    page.screenshot(path=str(SHOTS / f"workers-{width}-{theme}-archived.png"))

    page.goto(site + "/")
    to_workers(page)
    page.wait_for_selector("#workers-tab-archived")
    check(f"{tag}: the tab picked stays on this device", page.locator("#workers-tab-archived").get_attribute("aria-selected") == "true")
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
