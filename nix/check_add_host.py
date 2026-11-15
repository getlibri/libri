#!/usr/bin/env python3
"""Adding a host (2026-11-05) opens a host page over the host in use, as a
history entry of its own: Back, Cancel and the browser's back return to that
host untouched, and only a host that answers replaces it. The side panel's
Host lists every other host the browser knows, to switch to or forget. Runs
the built site against two stand-in hosts, box (the page's own) and kit, in
Chromium at 1440 and 390. Needs Playwright and a browser, so it stays out of
`nix flake check`.
Usage: check_add_host.py SITE_DIR SHOTS_DIR
"""
import functools
import http.server
import sys
import threading
from pathlib import Path
from urllib.parse import urlparse

from playwright.sync_api import sync_playwright

SITE, SHOTS = Path(sys.argv[1]), Path(sys.argv[2])
KIT = "http://kit.example:7000"
SAID = {"box": "Box is the page's own host.", "kit": "Kit answers from another address."}
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
    def replies(name):
        return {
            "/api/me": {"name": "Libri", "personas": [], "voice": None, "models": False, "forgets": []},
            "/api/work": {"threads": []},
            "/api/history": [{"i": 1, "kind": "talk", "text": SAID[name], "date": "2026-11-05T18:00:00Z", "reaction": None}],
            "/api/usage": [],
            "/api/host": {"id": f"{name}-id", "name": name, "assistant": "Libri", "version": "0", "uptime": 3600},
            "/api/pending": [],
            "/api/asks": [],
            "/api/devices": [],
            "/api/connections": {"servers": [], "xai": None},
            "/api/watches": [],
            "/api/guide": {"text": ""},
            "/api/worker-names": {"builtin": [], "added": [], "own": [], "use_own": False, "excluded": [], "allowed": 0, "left_out": []},
            "/api/library": {"pages": [], "lists": []},
            "/api/scratchpad": {"id": None, "text": "", "working": False, "words": "", "thinking": "", "stopping": False, "queued": [], "memory": True, "asks": []},
            "/api/task": {"agents": [], "screen": None},
            "/api/memory": {"state": "working", "reason": None, "since": None, "waiting": 0, "newest_summary": None, "latest_message": None, "model": "haiku"},
        }

    def api(route):
        url = urlparse(route.request.url)
        name = "kit" if url.netloc == urlparse(KIT).netloc else "box"
        cors = {"access-control-allow-origin": "*", "access-control-allow-headers": "*"}
        if route.request.method == "OPTIONS":
            return route.fulfill(status=204, headers=cors)
        path = url.path[url.path.index("/api/"):]
        if path == "/api/events":
            return route.fulfill(status=200, headers={"content-type": "text/event-stream", **cors}, body=": open\n\n")
        return route.fulfill(json=replies(name).get(path, {}), headers=cors)

    context.route("**/api/**", api)


def load(browser, site, size, mobile=False):
    context = browser.new_context(viewport={"width": size[0], "height": size[1]}, service_workers="block", is_mobile=mobile, has_touch=mobile, reduced_motion="reduce")
    context.set_default_timeout(30000)
    stand_in(context)
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(site + "/")
    page.wait_for_selector(f"text={SAID['box']}")
    page.wait_for_timeout(400)
    return context, page, errors


def current(page):
    return page.evaluate("localStorage.getItem('libri-current')")


def flow(page, where, open_host):
    """The host page's ways back, adding kit, switching back and forgetting it."""
    shot = lambda name: page.screenshot(path=SHOTS / f"{where}-{name}.png")
    host_page = page.get_by_role("heading", name="Add a host")

    open_host()
    add = page.get_by_role("button", name="Add a host")
    check(f"{where}: the Host section offers Add a host", add.is_visible())
    shot("host")
    add.click()
    check(f"{where}: Add a host opens the host page", host_page.is_visible() and page.evaluate("location.hash") == "#add-host")
    check(f"{where}: the host page shows the host in use", page.get_by_text("In use").is_visible())
    check(f"{where}: the address field is open, with Cancel", page.get_by_role("textbox", name="Address or pairing link").is_visible() and page.get_by_role("button", name="Cancel").is_visible())
    shot("add-host")
    page.go_back()
    page.wait_for_timeout(300)
    check(f"{where}: browser back closes it", host_page.count() == 0 and page.evaluate("location.hash") == "")
    check(f"{where}: the host in use is untouched", current(page) is None and page.get_by_text(SAID["box"]).is_visible())

    for way in ["Cancel", "Back"]:
        open_host()
        page.get_by_role("button", name="Add a host").click()
        page.get_by_role("button", name=way, exact=True).click()
        page.wait_for_timeout(300)
        check(f"{where}: {way} closes it", host_page.count() == 0 and page.evaluate("location.hash") == "")
        page.go_back()
        page.wait_for_timeout(300)
        check(f"{where}: after {way}, back does not reopen it", host_page.count() == 0)
        page.go_forward()

    open_host()
    page.get_by_role("button", name="Add a host").click()
    page.get_by_role("textbox", name="Address or pairing link").fill("kit.example:7000")
    page.get_by_role("button", name="Add", exact=True).click()
    page.wait_for_selector(f"text={SAID['kit']}")
    page.wait_for_timeout(400)
    check(f"{where}: an answering host replaces the one in use", current(page) == KIT and page.evaluate("location.hash") == "")
    open_host()
    page.wait_for_timeout(600)
    check(f"{where}: the Host section lists box to switch to", page.get_by_role("button", name="Switch to box").first.is_visible())
    check(f"{where}: kit, in use, has no Forget", page.get_by_role("button", name="Forget kit").count() == 0)
    shot("hosts-list")
    page.get_by_role("button", name="Switch to box").last.click()
    page.wait_for_selector(f"text={SAID['box']}")
    check(f"{where}: switching back returns to box", current(page) is None)
    open_host()
    page.wait_for_timeout(300)
    page.get_by_role("button", name="Forget kit").last.click()
    page.wait_for_timeout(300)
    check(f"{where}: Forget takes kit off the list", page.get_by_role("button", name="Switch to kit").count() == 0 and "kit" not in page.evaluate("localStorage.getItem('libri-hosts')"))


def wide(browser, site):
    context, page, errors = load(browser, site, (1440, 900))

    def open_host():
        if page.get_by_role("button", name="Add a host").count() == 0:
            page.locator('aside[aria-label="Hosts and settings"]').get_by_role("button", name="Host: ", exact=False).first.click()
            page.wait_for_timeout(400)

    flow(page, "1440", open_host)
    check("1440: no page errors", not errors, "; ".join(errors))
    context.close()


def phone(browser, site):
    context, page, errors = load(browser, site, (390, 844), True)

    def open_host():
        if page.get_by_role("button", name="Add a host").count() == 0:
            page.get_by_role("button", name="Side panel").click()
            page.wait_for_timeout(400)

    flow(page, "390", open_host)
    check("390: no page errors", not errors, "; ".join(errors))
    context.close()


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Pages, directory=str(SITE)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    with sync_playwright() as p:
        browser = p.chromium.launch()
        wide(browser, site)
        phone(browser, site)
        browser.close()
    print(f"{total - len(failures)}/{total} passed")
    sys.exit(1 if failures else 0)


main()
