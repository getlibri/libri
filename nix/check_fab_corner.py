#!/usr/bin/env python3
"""The Tasks and Library + floats at the lower right of the column it belongs
to, on the phone and the wide screen alike: 56px, 16px in from the column's
right edge and foot, and the list under it scrolls its last row clear. Runs
the built site against the stand-in host of check_archived_last.py in Chromium
at 390 and 1440, with a Library long enough to scroll. Needs Playwright and a
browser, so it stays out of `nix flake check`.
Usage: check_fab_corner.py SITE_DIR SHOTS_DIR
"""
import functools
import http.server
import socket
import sys
import threading
import time
from pathlib import Path
from urllib.parse import urlparse

from playwright.sync_api import sync_playwright

SITE, SHOTS = Path(sys.argv[1]), Path(sys.argv[2])
NOW = int(time.time())
failures = []


def check(what, ok, detail=""):
    print(f"{'PASS' if ok else 'FAIL'} {what} {detail}", flush=True)
    if not ok:
        failures.append(what)


class Pages(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_):
        pass


def thread(tid, state, ago=300):
    return {"id": tid, "name": tid, "brief": f"Check the {tid} work", "gist": f"{tid} is at it.", "state": state, "at": NOW - ago,
            "read": state != "waiting", "waiting_on": [], "archived": False}


THREADS = [thread(f"W{i:02}", "sleeping", ago=1000 * i) for i in range(30)]
PAGES = [{"name": f"page-{i:02}", "title": f"Page {i:02}", "changed": NOW - 600 * i} for i in range(25)]


def api(route):
    path = urlparse(route.request.url).path
    path = path[path.index("/api/"):]
    if path == "/api/events":
        return route.fulfill(status=200, headers={"content-type": "text/event-stream"}, body=": open\n\n")
    replies = {
        "/api/me": {"name": "Libri", "personas": [], "voice": None, "models": False, "forgets": []},
        "/api/work": {"threads": THREADS},
        "/api/watches": {"watches": []},
        "/api/history": [{"i": 10, "kind": "talk", "text": "Hello.", "date": "2026-11-04T12:40:00Z", "reaction": None}],
        "/api/usage": [],
        "/api/host": {"id": "stand-in", "name": "box", "version": "0", "uptime": 3600},
        "/api/pending": [],
        "/api/asks": [],
        "/api/library": {"pages": PAGES, "lists": []},
        "/api/scratchpad": {"id": None, "text": "", "working": False, "words": "", "thinking": "", "stopping": False, "queued": [], "memory": True, "asks": []},
        "/api/task": {"agents": [], "screen": None},
        "/api/memory": {"state": "working", "reason": None, "since": None, "waiting": 0, "newest_summary": None, "latest_message": None, "model": "haiku"},
    }
    return route.fulfill(json=replies.get(path, {}))


# The +, the column it floats in, and the bottom of that column's last row
# once its scroller, if it has one, is at its foot.
PLACE = """(label) => {
  const b = document.querySelector(`button[aria-label="${label}"]`);
  if (!b) return null;
  const col = b.parentElement, r = b.getBoundingClientRect(), c = col.getBoundingClientRect();
  const scroller = [...col.querySelectorAll('*')].find((e) => e.scrollHeight > e.clientHeight + 4 && getComputedStyle(e).overflowY === 'auto');
  // A column short enough not to scroll is measured as it stands.
  if (scroller) scroller.scrollTop = scroller.scrollHeight;
  const rows = [...(scroller || col).querySelectorAll('li')].filter((e) => e.offsetParent);
  const last = rows.length ? Math.max(...rows.map((e) => e.getBoundingClientRect().bottom)) : null;
  return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width, height: r.height,
           colLeft: c.left, colRight: c.right, colBottom: c.bottom, colWidth: c.width, view: innerWidth,
           last, scrolls: !!scroller, text: col.textContent.slice(0, 80) };
}"""


def place(page, tag, label, column_word):
    page.wait_for_selector(f'button[aria-label="{label}"]')
    page.wait_for_timeout(400)
    p = page.evaluate(PLACE, label)
    check(f"{tag}: + is 56px", p and round(p["width"]) == 56 and round(p["height"]) == 56, p and (p["width"], p["height"]))
    check(f"{tag}: + 16px in from its column's right edge", p and abs(p["colRight"] - p["right"] - 16) <= 1, p and (p["colRight"], p["right"]))
    check(f"{tag}: + 16px above its column's foot", p and abs(p["colBottom"] - p["bottom"] - 16) <= 1, p and (p["colBottom"], p["bottom"]))
    check(f"{tag}: + in the right half of its column", p and p["left"] > p["colLeft"] + p["colWidth"] / 2, p and (p["colLeft"], p["left"]))
    check(f"{tag}: + floats in the {column_word} column", p and column_word in p["text"], p and p["text"])
    check(f"{tag}: last row scrolls clear of the +", p and p["last"] is not None and p["last"] <= p["top"] + 1, p and (p["last"], p["top"]))
    return p


def run(browser, site, width, height, mobile):
    context = browser.new_context(viewport={"width": width, "height": height}, service_workers="block", is_mobile=mobile, has_touch=mobile)
    context.set_default_timeout(10000)
    context.add_init_script("localStorage.setItem('libri-work-open-sleeping', 'on')")
    context.route("**/api/**", api)
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(site + "/")
    if mobile:
        page.locator("nav[aria-label=Places] button:has-text('Tasks')").first.click()
    place(page, f"{width} Tasks", "Start a worker", "Tasks")
    page.screenshot(path=str(SHOTS / f"fab-{width}-tasks.png"))
    page.click('button[aria-label="Start a worker"]')
    check(f"{width} Tasks: + opens the box and turns to Close",
          page.locator("input[placeholder='What should a new worker do?']").count() == 1 and page.locator('button[aria-label="Close"][aria-expanded=true]').count() >= 1)
    page.screenshot(path=str(SHOTS / f"fab-{width}-tasks-open.png"))
    page.click('button[aria-label="Close"][aria-expanded=true]')
    if mobile:
        page.locator("nav[aria-label=Places] button:has-text('Library')").first.click()
    place(page, f"{width} Library", "What to make", "Library")
    page.screenshot(path=str(SHOTS / f"fab-{width}-library.png"))
    page.click('button[aria-label="What to make"]')
    check(f"{width} Library: + opens the box", page.locator("input[placeholder='A document, a dashboard, a page…']").count() == 1)
    page.screenshot(path=str(SHOTS / f"fab-{width}-library-open.png"))
    check(f"{width}: no page errors", not errors, errors)
    context.close()


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), functools.partial(Pages, directory=str(SITE)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{port}"
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for width, height, mobile in ((390, 844, True), (1440, 900, False)):
            run(browser, site, width, height, mobile)
        browser.close()
    print(f"{len(failures)} failed")
    sys.exit(1 if failures else 0)


main()
