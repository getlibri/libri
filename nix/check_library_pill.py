#!/usr/bin/env python3
"""The Library opens from the chat's pill (2026-11-05), as a side panel
beside the chat on a wide screen and over it on a phone: a head with its
title, find and close, the gallery, and a small + at the lower right whose
menu of kinds sits just above it. A kind picked opens a composer at the
panel's foot holding it as a removable chip. The left column keeps the
hosts, the memory and Settings, and the phone's bar no Library. Runs the
built site against a stand-in host, in Chromium at 1440 in both themes and
390, the panel as wide as the side columns (390px by default) on a wide screen. Needs Playwright and a browser, so it stays out of `nix flake check`.
Usage: check_library_pill.py SITE_DIR SHOTS_DIR
"""
import functools
import http.server
import sys
import threading
from pathlib import Path
from urllib.parse import urlparse

from playwright.sync_api import sync_playwright

SITE, SHOTS = Path(sys.argv[1]), Path(sys.argv[2])
SAID = "The Library opens from the pill now."
PAGES = [
    {"name": "freedom-index", "title": "Freedom index", "changed": 1762290000},
    {"name": "libri-working-book", "title": "Working, as a book", "changed": 1762280000},
    {"name": "box-ram", "title": "Box memory", "changed": 1762200000},
]
LISTS = [{"name": "groceries", "items": ["eggs", "bread"]}]
failures = []
total = 0


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
    replies = {
        "/api/me": {"name": "Libri", "personas": [], "voice": None, "models": False, "forgets": []},
        "/api/work": {"threads": []},
        "/api/history": [
            {"i": 10, "kind": "user", "text": "Put the Library in the pill.", "date": "2026-11-04T18:43:00Z", "reaction": None},
            {"i": 11, "kind": "talk", "text": SAID, "date": "2026-11-04T18:44:00Z", "reaction": None},
        ],
        "/api/usage": [],
        "/api/host": {"id": "stand-in", "name": "box", "assistant": "Libri", "version": "0", "uptime": 3600},
        "/api/pending": [],
        "/api/asks": [],
        "/api/library": {"pages": PAGES, "lists": LISTS},
        "/api/scratchpad": {"id": None, "text": "", "working": False, "words": "", "thinking": "", "stopping": False, "queued": [], "memory": True, "asks": []},
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
    # A Library page opens at its name under the site.
    for p in PAGES:
        context.route(f"**/{p['name']}/", lambda r, t=p["title"]: r.fulfill(status=200, content_type="text/html", body=f"<body style='font:16px sans-serif;padding:24px'><h1>{t}</h1><p>A page from the Library.</p></body>"))


def load(browser, site, size, theme, mobile=False):
    context = browser.new_context(viewport={"width": size[0], "height": size[1]}, service_workers="block", is_mobile=mobile, has_touch=mobile, reduced_motion="reduce")
    context.set_default_timeout(30000)
    if theme == "dark":
        # Also runs in a Library page's sandboxed frame, which has no storage.
        context.add_init_script("try { localStorage.setItem('theme', 'dark') } catch {}")
    stand_in(context)
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(site + "/")
    page.wait_for_selector(f"text={SAID}")
    page.wait_for_timeout(400)
    return context, page, errors


def box(locator):
    return locator.bounding_box()


def panel_flow(page, where, panel, shots):
    """The panel's own flow, the same on either screen."""
    check(f"{where}: the panel's head has find and close", panel.get_by_role("button", name="Find in the Library").is_visible() and panel.get_by_role("button", name="Close").first.is_visible())
    check(f"{where}: the gallery shows pages and lists", panel.get_by_text("Freedom index").is_visible() and panel.get_by_text("groceries").is_visible())
    later = panel.locator('section[aria-label="Coming later"]')
    check(f"{where}: Notes and Reminders stand as not built yet", later.get_by_text("Notes").is_visible() and later.get_by_text("Reminders").is_visible() and later.get_by_text("Not built yet").count() == 2)
    plus = panel.get_by_role("button", name="Make something")
    pb = plus.bounding_box()
    check(f"{where}: + is small", pb["width"] <= 40.5 and pb["height"] <= 40.5, pb)
    fb = panel.bounding_box()
    check(f"{where}: + sits at the panel's lower right", fb["x"] + fb["width"] - (pb["x"] + pb["width"]) < 24 and fb["y"] + fb["height"] - (pb["y"] + pb["height"]) < 24, (fb, pb))
    plus.click()
    page.wait_for_timeout(250)
    menu = panel.get_by_role("menu", name="Make")
    items = menu.get_by_role("menuitem").all_inner_texts()
    check(f"{where}: + opens its kinds", [t.strip() for t in items] == ["Web page", "Document", "Presentation", "Design", "Dashboard"], items)
    mb = menu.bounding_box()
    check(f"{where}: the menu sits just above +", mb["y"] + mb["height"] <= pb["y"] and pb["y"] - (mb["y"] + mb["height"]) < 24, (mb, pb))
    page.screenshot(path=shots / f"{where.replace(' ', '-')}-menu.png")
    menu.get_by_role("menuitem", name="Presentation").click()
    page.wait_for_timeout(250)
    field = panel.get_by_role("textbox", name="Describe what to build")
    check(f"{where}: a kind opens the composer at the foot", field.is_visible() and field.bounding_box()["y"] > fb["y"] + fb["height"] / 2)
    check(f"{where}: the composer holds the kind as a chip", panel.get_by_role("button", name="Remove Presentation").is_visible())
    check(f"{where}: no box opens at the top", panel.get_by_placeholder("A document, a dashboard, a page…").count() == 0)
    field.fill("Ten slides on the tides")
    page.screenshot(path=shots / f"{where.replace(' ', '-')}-composer.png")
    panel.get_by_role("button", name="Remove Presentation").click()
    check(f"{where}: the chip comes off", panel.get_by_role("button", name="Remove Presentation").count() == 0 and field.is_visible())
    field.fill("")
    panel.get_by_role("button", name="Close").last.click()
    check(f"{where}: an empty composer closes back to +", panel.get_by_role("button", name="Make something").is_visible())


def wide(browser, site, size, theme):
    where = f"{size[0]} {theme}"
    context, page, errors = load(browser, site, size, theme)
    header = page.locator("[data-talk] header").first
    right = header.locator(":scope > div").nth(1)
    left_col = page.locator('aside[aria-label="Hosts and settings"]')
    check(f"{where}: the left column has no Library", left_col.get_by_text("Library").count() == 0)
    check(f"{where}: the left column keeps its host row and Settings", left_col.get_by_role("button", name="Host: box").count() == 1 and left_col.get_by_role("button", name="Settings").count() == 1)
    button = right.get_by_role("button", name="Library", exact=True)
    check(f"{where}: the pill has a Library button", button.is_visible())
    check(f"{where}: the pill's buttons have no words", right.inner_text().strip() == "", right.inner_text())
    page.screenshot(path=SHOTS / f"pill-{size[0]}-{theme}-closed.png")
    button.click()
    page.wait_for_timeout(500)
    panel = page.locator("[data-temp]")
    tb, cb = panel.bounding_box(), page.locator("[data-talk]").bounding_box()
    check(f"{where}: the Library opens beside the chat, full height", tb["x"] >= cb["x"] + cb["width"] - 1 and tb["height"] > size[1] - 80, (tb, cb))
    check(f"{where}: the panel is as wide as the side columns (390)", abs(tb["width"] - 390) < 1.5, tb)
    check(f"{where}: the pill drops Library while it is open", right.get_by_role("button", name="Library", exact=True).count() == 0)
    page.screenshot(path=SHOTS / f"pill-{size[0]}-{theme}-open.png")
    panel_flow(page, f"pill-{size[0]}-{theme}", panel, SHOTS)
    panel.get_by_text("Freedom index").click()
    page.wait_for_timeout(500)
    check(f"{where}: an item opens in the panel", panel.locator("iframe").count() == 1)
    page.screenshot(path=SHOTS / f"pill-{size[0]}-{theme}-item.png")
    panel.get_by_role("button", name="Back to the Library").click()
    page.wait_for_timeout(400)
    check(f"{where}: back returns to the gallery", panel.get_by_text("Box memory").is_visible())
    panel.get_by_role("button", name="Close").first.click()
    page.wait_for_timeout(500)
    check(f"{where}: close puts the panel away", panel.get_by_text("Box memory").count() == 0 and right.get_by_role("button", name="Library", exact=True).is_visible())
    check(f"{where}: no page errors", not errors, "; ".join(errors))
    context.close()


def phone(browser, site):
    context, page, errors = load(browser, site, (390, 844), "light", True)
    header = page.locator("header").first
    check("390: the pill has a Library button", header.get_by_role("button", name="Library", exact=True).count() == 1)
    check("390: the bar has no Library", page.locator("nav button:has-text('Library')").count() == 0)
    page.screenshot(path=SHOTS / "pill-390-closed.png")
    header.get_by_role("button", name="Library", exact=True).click()
    page.wait_for_timeout(500)
    panel = page.locator("[data-sheet]").last
    check("390: the Library opens over the chat", panel.bounding_box()["width"] == 390)
    check("390: it keeps #library", page.evaluate("location.hash") == "#library")
    page.screenshot(path=SHOTS / "pill-390-open.png")
    panel_flow(page, "pill-390", panel, SHOTS)
    panel.get_by_role("button", name="Close").first.click()
    page.wait_for_timeout(400)
    check("390: close drops #library", page.evaluate("location.hash") == "")
    page.goto(site + "/#library")
    page.reload()
    page.wait_for_timeout(800)
    check("390: #library opens it again", page.locator("[data-sheet]").get_by_text("Freedom index").is_visible())
    check("390: no page errors", not errors, "; ".join(errors))
    context.close()


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Pages, directory=str(SITE)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for size in [(1440, 900)]:
            for theme in ["light", "dark"]:
                wide(browser, site, size, theme)
        phone(browser, site)
        browser.close()
    print(f"{total - len(failures)}/{total} passed")
    sys.exit(1 if failures else 0)


main()
