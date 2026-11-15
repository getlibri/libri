#!/usr/bin/env python3
"""Holding a message works off the main chat too. On a touch screen, holding
the person's bubble or the reply's words in the Temp chat, or in a worker's
thread, opens the message menu with Copy and Select text, as it does in the
chat; on a mouse a right click leaves the browser's own menu alone. Runs the
built site against a stand-in host in Chromium at 390 (touch) and 1440
(mouse) in both themes. Needs Playwright and a browser, so it stays out of
`nix flake check`.
Usage: check_thread_hold.py SITE_DIR SHOTS_DIR
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

TEMP_ASKED = "What is the tallest tree?"
TEMP_SAID = "A coast redwood, at about 116 metres."
TEMP_QUEUED = "And the oldest one?"
BRIEF = "Count the trees in the garden plan."
WORKER_TOLD = "Count the hedges too."
WORKER_SAID = "Fourteen trees and three hedges."


def check(what, ok, detail=""):
    global total
    total += 1
    if not ok:
        failures.append(what)
        print(f"FAIL {what} {detail}", flush=True)


class Pages(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_):
        pass


def stand_in(context):
    temp = f"0|user: {TEMP_ASKED}\n1|talk: {TEMP_SAID}"
    worker = f"0|user: {BRIEF}\n1|talk: Reading the plan.\n2|user: (From the person, typed on your page) {WORKER_TOLD}\n3|talk: {WORKER_SAID}"
    thread = {"id": "w1", "name": "Arlo", "brief": BRIEF, "state": "waiting", "need": "review", "gist": WORKER_SAID, "at": int(time.time()) - 60, "read": True}
    replies = {
        "/api/me": {"name": "Libri", "personas": [], "voice": None, "models": False, "forgets": []},
        "/api/work": {"threads": [thread]},
        "/api/work/w1": {"text": worker},
        "/api/history": [{"i": 0, "kind": "talk", "text": "The main chat.", "date": "2026-11-05T18:00:00Z", "reaction": None}],
        "/api/usage": [],
        "/api/host": {"id": "stand-in", "name": "box", "assistant": "Libri", "version": "0", "uptime": 3600},
        "/api/pending": [],
        "/api/asks": [],
        "/api/scratchpad": {"id": "s1", "text": temp, "working": False, "words": "", "thinking": "", "stopping": False, "queued": [TEMP_QUEUED], "memory": True, "asks": []},
        "/api/task": {"agents": [], "screen": None},
        "/api/memories": {"memories": []},
        "/api/usage/report": {"accounts": []},
        "/api/builtins": [],
        "/api/library": {"pages": [], "lists": []},
        "/api/watches": {"watches": []},
        # No layout kept: each shell opens on its first one (phone.tsx, App.tsx).
        "/api/layout": {"version": 0, "layout": None, "by": None, "undo": False},
        "/api/phone-layout": {"version": 0, "layout": None, "by": None, "undo": False},
        "/api/memory": {"state": "working", "reason": None, "since": None, "waiting": 0, "newest_summary": None, "latest_message": None, "model": "haiku"},
    }

    def api(route):
        url = urlparse(route.request.url)
        path = url.path[url.path.index("/api/"):]
        if path == "/api/events":
            return route.fulfill(status=200, headers={"content-type": "text/event-stream"}, body=": open\n\n")
        return route.fulfill(json=replies.get(path, {}))

    context.route("**/api/**", api)


def load(browser, site, size, theme, mobile, hash=""):
    context = browser.new_context(viewport={"width": size[0], "height": size[1]}, service_workers="block", is_mobile=mobile, has_touch=mobile, reduced_motion="reduce")
    context.set_default_timeout(15000)
    if theme == "dark":
        context.add_init_script("localStorage.setItem('theme', 'dark')")
    stand_in(context)
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(site + "/" + hash)
    page.wait_for_selector("text=The main chat.") if not hash else page.wait_for_timeout(800)
    page.wait_for_timeout(400)
    return context, page, errors


def hold(page, locator):
    """A finger held on `locator` past the hold time, then lifted."""
    box = locator.bounding_box()
    x, y = box["x"] + min(box["width"] / 2, 60), box["y"] + box["height"] / 2
    cdp = page.context.new_cdp_session(page)
    cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [{"x": x, "y": y}]})
    page.wait_for_timeout(700)
    cdp.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
    page.wait_for_timeout(250)


def menu_items(page):
    return page.locator("[role=menu] [role=menuitem]").all_inner_texts()


def shut(page):
    """Escape shuts a menu still open; with none, it would shut the page."""
    if page.locator("[role=menu]").count():
        page.keyboard.press("Escape")
        page.wait_for_timeout(200)


def held(page, where, what, text, shot):
    hold(page, page.get_by_text(text, exact=True).last)
    items = menu_items(page)
    check(f"{where}: holding {what} opens Copy and Select text", items[:2] == ["Copy", "Select text"], items)
    page.screenshot(path=str(SHOTS / f"{where} {shot}.png"))
    if items[:2] == ["Copy", "Select text"]:
        page.locator("[role=menu]").get_by_text("Select text", exact=True).click()
        page.wait_for_timeout(250)
        dialog = page.locator("[role=dialog][aria-label='Select text']")
        check(f"{where}: Select text on {what} shows its words", dialog.count() == 1 and text in dialog.inner_text(), dialog.count())
        page.screenshot(path=str(SHOTS / f"{where} {shot} select.png"))
        if dialog.count():
            dialog.get_by_role("button", name="Close").click()
            page.wait_for_timeout(200)
    shut(page)
    check(f"{where}: the menu closes after {what}", page.locator("[role=menu]").count() == 0)


def open_temp(page, phone):
    """Temp opens from the dock on a wide screen, and on a phone is added to the bar from its plus."""
    if phone:
        page.locator("[data-phone] button[aria-label^='Add to']").first.tap()
        page.locator("[role=menuitem][data-add=temp]").tap()
    else:
        page.locator("button[aria-label^='Temp chat (']").first.click()


def phone(browser, site, theme):
    where = f"390 {theme} temp"
    context, page, errors = load(browser, site, (390, 844), theme, True)
    open_temp(page, True)
    page.wait_for_selector(f"text={TEMP_SAID}")
    page.wait_for_timeout(300)
    held(page, where, "the person's bubble", TEMP_ASKED, "asked")
    held(page, where, "the reply", TEMP_SAID, "said")
    held(page, where, "a queued bubble", TEMP_QUEUED, "queued")
    check(f"{where}: no page errors", not errors, errors)
    context.close()

    where = f"390 {theme} worker"
    context, page, errors = load(browser, site, (390, 844), theme, True, "#work")
    page.get_by_text(BRIEF).first.tap()
    page.wait_for_selector(f"text={WORKER_SAID}")
    page.wait_for_timeout(300)
    held(page, where, "the brief", BRIEF, "brief")
    held(page, where, "words the person told it", WORKER_TOLD, "told")
    held(page, where, "the reply", WORKER_SAID, "said")
    check(f"{where}: no page errors", not errors, errors)
    context.close()


def desktop(browser, site, theme):
    where = f"1440 {theme} temp"
    context, page, errors = load(browser, site, (1440, 900), theme, False)
    open_temp(page, False)
    page.wait_for_selector(f"text={TEMP_SAID}")
    page.wait_for_timeout(300)
    page.get_by_text(TEMP_SAID, exact=True).last.click(button="right")
    page.wait_for_timeout(250)
    check(f"{where}: a right click keeps the browser's menu", page.locator("[role=menu]").count() == 0)
    page.screenshot(path=str(SHOTS / f"{where}.png"))
    check(f"{where}: no page errors", not errors, errors)
    context.close()


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    pages = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Pages, directory=str(SITE)))
    threading.Thread(target=pages.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{pages.server_address[1]}"
    started = time.time()
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for theme in ("light", "dark"):
            phone(browser, site, theme)
            desktop(browser, site, theme)
        browser.close()
    print(f"{total} checks, {len(failures)} failed, {time.time() - started:.0f}s")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
