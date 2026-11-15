#!/usr/bin/env python3
"""The talk reads at the composer's width. In the main chat and in Temp the
messages' column, from a user bubble's left edge to the assistant text's
right edge, has the composer's own left and right edges, at 1440 and 1920,
with the side columns open and folded, in both themes. Runs the built site
against a stand-in host in Chromium and prints each measure. Needs
Playwright and a browser, so it stays out of `nix flake check`.
Usage: check_chat_width.py SITE_DIR SHOTS_DIR
"""
import functools
import http.server
import itertools
import sys
import threading
from pathlib import Path
from urllib.parse import urlparse

from playwright.sync_api import sync_playwright

SITE, SHOTS = Path(sys.argv[1]), Path(sys.argv[2])
LONG = " ".join(["Every line of the talk should end where the composer's edge does, so the two read as one column."] * 4)
ASKED = "Make the chat text as wide as the box I type in, in the main chat and in Temp, so the two edges line up exactly at every width."
failures = []
total = 0


def check(what, ok, detail=""):
    global total
    total += 1
    if not ok:
        failures.append(what)
    print(f"{'ok  ' if ok else 'FAIL'} {what} {detail}", flush=True)


class Pages(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_):
        pass


def stand_in(context):
    replies = {
        "/api/me": {"name": "Libri", "personas": [], "voice": None, "models": False, "forgets": []},
        "/api/work": {"threads": []},
        "/api/history": [
            {"i": 10, "kind": "user", "text": ASKED, "date": "2026-11-04T18:52:00Z", "reaction": None},
            {"i": 11, "kind": "talk", "text": LONG, "date": "2026-11-04T18:53:00Z", "reaction": None},
        ],
        "/api/usage": [],
        "/api/host": {"id": "stand-in", "name": "box", "assistant": "Libri", "version": "0", "uptime": 3600},
        "/api/pending": [],
        "/api/asks": [],
        "/api/library": {"pages": [], "lists": []},
        "/api/memories": {"memories": []},
        "/api/usage/report": {"accounts": []},
        "/api/builtins": [],
        "/api/watches": {"watches": []},
        # No layout kept: each shell opens on its first one (phone.tsx, App.tsx).
        "/api/layout": {"version": 0, "layout": None, "by": None, "undo": False},
        "/api/phone-layout": {"version": 0, "layout": None, "by": None, "undo": False},
        "/api/scratchpad": {"id": "scratchpad-t1", "text": "", "working": True, "words": LONG, "thinking": "", "stopping": False, "queued": [ASKED], "memory": True, "asks": []},
        "/api/task": {"agents": [], "screen": None},
        "/api/memory": {"state": "working", "reason": None, "since": None, "waiting": 0, "newest_summary": None, "latest_message": None, "model": "haiku"},
    }

    def api(route):
        url = urlparse(route.request.url)
        path = url.path[url.path.index("/api/"):]
        if path == "/api/events":
            return route.fulfill(status=200, headers={"content-type": "text/event-stream"}, body=": open\n\n")
        return route.fulfill(json=replies.get(path, {}))

    context.route("**/api/**", api)


# The column's edges: the widest of the user bubbles' left edges and the
# assistant reply's fill (or its text, outside one), against the composer's frame.
MEASURE = """(where) => {
  const root = document.querySelector(where);
  const r = (e) => e.getBoundingClientRect();
  const bubbles = [...root.querySelectorAll('p, div')].filter((e) => e.textContent.trim() === %r && e.children.length === 0 || (e.textContent.trim() === %r && getComputedStyle(e).borderTopStyle !== 'none' && getComputedStyle(e).borderTopWidth !== '0px'));
  const bubble = bubbles.map(r).sort((a, b) => b.width - a.width)[0];
  const texts = [...root.querySelectorAll('p')].filter((e) => e.textContent.startsWith('Every line of the talk'));
  const text = texts.map((e) => r(e.closest('.said') || e)).sort((a, b) => b.width - a.width)[0];
  const form = root.querySelector('textarea').closest('form, [data-composer], .rounded-3xl');
  const c = r(form);
  return { bubbleLeft: bubble && bubble.left, bubbleRight: bubble && bubble.right, textLeft: text && text.left, textRight: text && text.right, composerLeft: c.left, composerRight: c.right };
}""" % (ASKED, ASKED)


def measure(page, where, label):
    m = page.evaluate(MEASURE, where)
    left = m["bubbleLeft"] if m["bubbleLeft"] is not None else None
    detail = " ".join(f"{k}={v:.1f}" for k, v in m.items() if v is not None)
    check(f"{label}: the assistant reply starts at the composer's left", m["textLeft"] is not None and abs(m["textLeft"] - m["composerLeft"]) < 0.51, detail)
    check(f"{label}: the assistant reply ends at the composer's right", m["textRight"] is not None and abs(m["textRight"] - m["composerRight"]) < 0.51)
    check(f"{label}: a user bubble ends at the composer's right", m["bubbleRight"] is not None and abs(m["bubbleRight"] - m["composerRight"]) < 0.51)
    return m


def run(browser, site, size, theme, text):
    context = browser.new_context(viewport={"width": size[0], "height": size[1]}, service_workers="block", reduced_motion="reduce")
    if theme == "dark":
        context.add_init_script("try { localStorage.setItem('theme', 'dark') } catch {}")
    # The talk's text size, as the settings stepper or a pinch leaves it.
    context.add_init_script(f"try {{ localStorage.setItem('talk-size', '{text / 16}') }} catch {{}}")
    stand_in(context)
    page = context.new_page()
    page.goto(site + "/")
    page.wait_for_selector("text=Every line of the talk")
    page.wait_for_timeout(400)
    where = f"{size[0]} {theme} {text}px"
    measure(page, "[data-surface=chat]", f"{where} main")
    page.screenshot(path=SHOTS / f"width-{size[0]}-{theme}-{text}px-main.png")
    # Temp opens from the dock, beside the main chat (App.tsx tiles).
    page.locator("button[aria-label^='Temp chat (']").first.click()
    page.wait_for_timeout(600)
    page.wait_for_selector("section[aria-label='Temp chat'] >> text=Every line of the talk")
    tm = measure(page, "[data-surface=temp]", f"{where} Temp")
    mm = measure(page, "[data-surface=chat]", f"{where} main beside Temp")
    check(f"{where}: Temp's column is as wide as the main chat's beside it", abs((tm["composerRight"] - tm["composerLeft"]) - (mm["composerRight"] - mm["composerLeft"])) < 0.51 and abs((tm["textRight"] - tm["textLeft"]) - (mm["textRight"] - mm["textLeft"])) < 0.51)
    page.screenshot(path=SHOTS / f"width-{size[0]}-{theme}-{text}px-temp.png")
    context.close()


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Pages, directory=str(SITE)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for size, theme in itertools.product([(1440, 900), (1920, 1080)], ["light", "dark"]):
            run(browser, site, size, theme, 16)
        for size in [(1440, 900), (1920, 1080)]:
            run(browser, site, size, "light", 18)
        browser.close()
    print(f"{total - len(failures)}/{total} passed")
    sys.exit(1 if failures else 0)


main()
