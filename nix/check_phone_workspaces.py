#!/usr/bin/env python3
"""A phone's workspaces (phone.tsx): the bar at the foot is the workspace in
use, its items a row the person adds to with the plus and takes from by a
long press; the pill left of it switches workspace, and a swipe along the bar
goes to the next one. A first run shows one workspace, Main: Chat,
and the Library. A long press opens the item's menu, whose Edit the bar edits it in place: an item dragged
sideways takes a new place, one dropped on the plus (an ✕ meanwhile) leaves
the bar; any time an item flicked up leaves it, with an Undo; the chat never
leaves, nor does the workspace's disc. No page has a head: Temp's Memory
and Clear and the Library's buttons float at the top, and the terminal has
no ✕. The
chat's header holds only the side panel and search (Temp and the terminal
moved into the plus), a swipe right on the leftmost item opens the side
panel, and the panel lists no places. Workspaces are kept on the stand-in
host, so a reload keeps them. Runs the built site in Chromium at 390 and
leaves screenshots. Needs Playwright and a browser, so it stays out of `nix
flake check`.
Usage: check_phone_workspaces.py SITE_DIR SHOTS_DIR
       check_phone_workspaces.py SITE_DIR serve PORT  (the stand-in host
       alone, for the Android app on an emulator: adb reverse tcp:PORT tcp:PORT)
"""
import functools
import http.server
import json
import sys
import threading
import time
from pathlib import Path
from urllib.parse import parse_qs, urlparse

SITE = Path(sys.argv[1])
# The phone bar's strip, apart from any tile's own tabs.
BAR = "[role=tablist][aria-label$=\"'s items\"]"
failures = []


def check(what, ok, got=None):
    print(("ok   " if ok else "FAIL ") + what + ("" if ok or got is None else f": {got}"), flush=True)
    if not ok:
        failures.append(what)


# The stand-in host of real size answers the rest (measure_ui_performance.py).
sys.path.insert(0, str(Path(__file__).resolve().parent))
_argv, sys.argv = sys.argv, [sys.argv[0], str(SITE), "serve"]
import measure_ui_performance as stand_in  # noqa: E402
sys.argv = _argv

stand_in.Host.replies.update({
    "/api/builtins": [
        {"id": "calendar", "title": "Calendar", "about": "Your calendars and events.", "on": True, "provider": "fastmail", "provider_title": "Fastmail",
         "connection": None, "signed_in": False, "choices": [], "agenda": True},
    ],
})


class Layouts:
    """The host's two stores (layout.rs): each a version and what is kept,
    a wait for the next version, and one undo."""

    def __init__(self):
        self.lock = threading.Condition()
        self.kept = {"layout": {"version": 0, "layout": None, "by": None, "undo": False}, "phone-layout": {"version": 0, "layout": None, "by": None, "undo": False}}

    def read(self, store, since):
        with self.lock:
            if since is not None:
                self.lock.wait_for(lambda: self.kept[store]["version"] != since, timeout=5)
            return dict(self.kept[store])

    def change(self, store, layout):
        with self.lock:
            k = self.kept[store]
            if k["layout"] != layout:
                self.kept[store] = {"version": k["version"] + 1, "layout": layout, "by": "person", "undo": k["layout"] is not None}
                self.lock.notify_all()
            return dict(self.kept[store])


LAYOUTS = Layouts()


class Host(stand_in.Host):
    def store(self):
        path = urlparse(self.path).path
        for store in ("phone-layout", "layout"):
            if path.endswith(f"/api/{store}"):
                return store

    def do_GET(self):
        store = self.store()
        if store:
            since = parse_qs(urlparse(self.path).query).get("since")
            try:
                return self.json(LAYOUTS.read(store, int(since[0]) if since else None))
            except (BrokenPipeError, ConnectionResetError):
                return  # a wait the page gave up on
        return super().do_GET()

    def do_POST(self):
        store = self.store()
        if store:
            body = json.loads(self.rfile.read(int(self.headers.get("content-length") or 0)) or b"null")
            return self.json(LAYOUTS.change(store, body))
        return super().do_POST()


def serve(port=0):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), functools.partial(Host, directory=str(SITE)))
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{server.server_address[1]}"


def swipe(page, cdp, x0, y, x1):
    """A finger drawn sideways, as the page reads a swipe (touch events)."""
    steps = 8
    cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [{"x": x0, "y": y}]})
    for i in range(1, steps + 1):
        cdp.send("Input.dispatchTouchEvent", {"type": "touchMove", "touchPoints": [{"x": x0 + (x1 - x0) * i / steps, "y": y}]})
        time.sleep(0.012)
    cdp.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
    page.wait_for_timeout(400)


def hold(page, cdp, selector):
    """A finger held on an element, then lifted."""
    box = page.locator(selector).bounding_box()
    x, y = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
    cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [{"x": x, "y": y}]})
    page.wait_for_timeout(700)
    cdp.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
    page.wait_for_timeout(300)


def finger(page, cdp, points, wait=0.012, hold_ms=0):
    """A finger put down at the first point, held, drawn through the rest, lifted."""
    (x, y), *rest = points
    cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [{"x": x, "y": y}]})
    if hold_ms:
        page.wait_for_timeout(hold_ms)
    for x, y in rest:
        cdp.send("Input.dispatchTouchEvent", {"type": "touchMove", "touchPoints": [{"x": x, "y": y}]})
        time.sleep(wait)
    cdp.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
    page.wait_for_timeout(450)


def centre(page, selector):
    box = page.locator(selector).bounding_box()
    return box["x"] + box["width"] / 2, box["y"] + box["height"] / 2, box["width"]


def along(x0, y0, x1, y1, steps=10):
    return [(x0, y0)] + [(x0 + (x1 - x0) * i / steps, y0 + (y1 - y0) * i / steps) for i in range(1, steps + 1)]


def editing(page):
    return page.locator(f"{BAR}[data-editing]").count() == 1


def bar(page):
    return page.locator(f"{BAR} [role=tab]").evaluate_all("(els) => els.map((e) => e.textContent.trim())")


def shown_item(page):
    return page.locator(f"{BAR} [role=tab][aria-selected=true]").evaluate_all("(els) => els.map((e) => e.textContent.trim())")


def space(page):
    return page.locator("[data-action=spaces]").get_attribute("title")


def main():
    from playwright.sync_api import sync_playwright

    shots = Path(sys.argv[2])
    shots.mkdir(parents=True, exist_ok=True)
    site = serve()
    with sync_playwright() as pw:
        browser = pw.chromium.launch()
        context = browser.new_context(viewport={"width": 390, "height": 844}, service_workers="block", is_mobile=True, has_touch=True)
        # 60 s, as check_chat_steady allows: the release runs these while workers' builds
        # load the host, and a first page load there can pass 15 s.
        context.set_default_timeout(60000)
        page = context.new_page()
        cdp = context.new_cdp_session(page)
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.goto(site + "/")
        page.wait_for_selector(f"[data-phone] {BAR}")
        page.wait_for_timeout(800)

        check("a first run's bar: Chat, Library", bar(page) == ["Chat", "Library"], bar(page))
        check("the first workspace is Main", space(page) == "Main", space(page))
        # Two items: the bar holds them at their width, centred, inside its margin.
        tabs = page.locator(f"{BAR}").evaluate("(t) => [t.parentElement.getBoundingClientRect().right, ...[...t.children].map((e) => e.getBoundingClientRect().width)]")
        check("two items stand alike on the bar, inside its margin", tabs[0] <= 390 - 14 + 0.5 and min(tabs[1:]) > 70 and max(tabs[1:]) - min(tabs[1:]) < 1, tabs)
        header = page.locator("[data-surface=chat] header button, [data-phone] > button[aria-label='Side panel']").evaluate_all("(els) => els.filter((e) => e.offsetParent && !e.closest('[data-chat-tabs]')).map((e) => e.getAttribute('aria-label'))")
        check("the chat's header: the side panel and search, besides its memories' tabs", sorted(header) == ["Search", "Side panel"], header)
        check("the chat's memories' tabs show Main", page.locator("[data-surface=chat] header [data-chat-tabs] [data-memory-tab=main]").count() == 1)
        page.screenshot(path=shots / "phone-header-390.png", clip={"x": 0, "y": 0, "width": 390, "height": 140})
        page.screenshot(path=shots / "phone-workspace-chat-390.png")

        page.locator("[data-action=add]").click()
        page.wait_for_selector("[role=menu][aria-label^='Add to']")
        offered = page.locator("[role=menu] [data-add]").evaluate_all("(els) => els.map((e) => e.getAttribute('data-add'))")
        check("the plus offers Temp and the terminal", "temp" in offered and "terminal" in offered, offered)
        check("the plus offers the built-ins, never Today", "builtin:calendar" in offered and "home" not in offered, offered)
        check("the plus offers every worker, though no Workers item", "workers" in offered, offered)
        page.screenshot(path=shots / "phone-plus-menu-390.png")
        page.locator("[role=menu] [data-add=temp]").click()
        page.wait_for_timeout(300)
        page.locator("[data-action=add]").click()
        page.wait_for_selector("[role=menu][aria-label^='Add to']")
        page.locator("[role=menu] [data-add=terminal]").click()
        page.wait_for_timeout(300)
        check("an item added joins the bar and shows", bar(page)[-1] == "Terminal" and shown_item(page) == ["Terminal"], (bar(page), shown_item(page)))

        # The terminal's page: no head and no ✕; the bar leaves it.
        term = page.locator("[data-surface=terminal]")
        check("the terminal has no head and no ✕", term.locator("header").count() == 0 and page.locator("[aria-label='Close the terminal']").count() == 0)
        page.screenshot(path=shots / "phone-terminal-390.png")

        # A long press opens the item's menu (tileMenu.tsx): its place's rows,
        # Move to workspace, Edit the bar and Close; Edit the bar edits it,
        # the disc left alone and the plus an ✕.
        hold(page, cdp, "[role=tab][data-item=terminal]")
        page.wait_for_selector("[data-tile-menu]")
        rows = page.locator("[data-tile-menu] [data-row]").evaluate_all("(els) => els.map((e) => e.getAttribute('data-row'))")
        check("a long press opens the item's menu, its terminal rows first", rows[:3] == ["Paste", "Clear screen and scrollback", "Restart shell"] and "Edit the bar" in rows and "Close" in rows and "Move to workspace…" in rows, rows)
        page.screenshot(path=shots / "phone-item-menu-390.png")
        page.locator("[data-tile-menu] [data-row='Edit the bar']").click()
        page.wait_for_timeout(200)
        check("Edit the bar edits it", editing(page) and page.locator("[data-action=remove]").count() == 1)
        check("the disc is no item to edit", page.locator(f"{BAR} [data-action=spaces]").count() == 0)
        page.screenshot(path=shots / "phone-bar-editing-390.png")
        x, y, w = centre(page, "[role=tab][data-item=terminal]")
        finger(page, cdp, along(x, y, x - w, y))
        check("dragged left one place, it takes it", bar(page) == ["Chat", "Library", "Terminal", "Temp"], bar(page))
        x, y, _ = centre(page, "[role=tab][data-item=chat]")
        px, py, _ = centre(page, "[data-action=remove]")
        finger(page, cdp, along(x, y, px, py, 16))
        check("the chat dropped on the ✕ stays", bar(page)[0] == "Chat" and len(bar(page)) == 4, bar(page))
        x, y, _ = centre(page, "[role=tab][data-item=temp]")
        finger(page, cdp, along(x, y, px, py, 16))
        check("an item dropped on the ✕ leaves the bar", bar(page) == ["Chat", "Library", "Terminal"], bar(page))
        check("an Undo comes up", page.locator("[data-action=undo-close]").count() == 1)
        page.locator("[data-action=undo-close]").click()
        page.wait_for_timeout(300)
        check("Undo puts it back in its place", bar(page) == ["Chat", "Library", "Terminal", "Temp"], bar(page))
        page.locator("[data-edit-backdrop]").click(position={"x": 200, "y": 300})
        page.wait_for_timeout(200)
        check("a tap off the bar puts it down", not editing(page) and page.locator("[data-action=add]").count() == 1)

        # Any time, a quick flick up closes an item; the chat stays.
        x, y, _ = centre(page, "[role=tab][data-item=chat]")
        finger(page, cdp, along(x, y, x, y - 90, 5))
        check("the chat flicked up stays", bar(page)[0] == "Chat" and len(bar(page)) == 4, bar(page))
        x, y, _ = centre(page, "[role=tab][data-item=terminal]")
        finger(page, cdp, along(x, y, x, y - 90, 5))
        check("an item flicked up leaves the bar", bar(page) == ["Chat", "Library", "Temp"], bar(page))
        toast = page.locator("[role=status]:has([data-action=undo-close])")
        check("with an Undo naming it", toast.count() == 1 and "Terminal" in toast.inner_text(), toast.count())
        page.screenshot(path=shots / "phone-flick-closed-390.png")
        x, y, _ = centre(page, "[role=tab][data-item=temp]")
        finger(page, cdp, along(x, y, x + 6, y - 20, 8), wait=0.06)
        check("a slow small drag up closes nothing", bar(page) == ["Chat", "Library", "Temp"], bar(page))
        page.wait_for_timeout(1200)
        kept = LAYOUTS.read("phone-layout", None)["layout"] or {}
        check("the host keeps the bar as edited", [[p["kind"] for p in w["items"]] for w in kept.get("spaces", [])] == [["chat", "library", "temp"]], kept)

        # The Library's buttons float; its page runs up under the status bar.
        page.locator("[role=tab][data-item=library]").click()
        page.wait_for_timeout(400)
        lib = page.locator("[data-surface=library]")
        found = lib.locator("[aria-label='Find in the Library']").bounding_box()
        check("the Library's find floats at the top right", found is not None and found["y"] < 70 and found["x"] > 300, found)
        page.screenshot(path=shots / "phone-library-390.png")

        page.locator("[data-action=spaces]").click()
        page.locator("[data-action=space-add]").click()
        page.locator("input[aria-label='Workspace name']").fill("Shell")
        page.keyboard.press("Enter")
        page.locator("[data-action=add]").click()
        page.locator("[role=menu] [data-add=terminal]").click()
        page.locator("[data-action=add]").click()
        page.locator("[role=menu] [data-add=temp]").click()
        page.wait_for_timeout(300)
        check("a new workspace holds the chat, then what was added", space(page) == "Shell" and bar(page) == ["Chat", "Terminal", "Temp"], (space(page), bar(page)))
        page.locator("[role=tab][data-item=terminal]").click()
        page.wait_for_timeout(600)
        page.screenshot(path=shots / "phone-workspace-shell-390.png")
        # Temp: no head; Memory floats at the top right over the talk.
        page.locator("[role=tab][data-item=temp]").click()
        page.wait_for_timeout(400)
        temp = page.locator("[data-surface=temp]")
        switch = temp.locator("[role=switch]").bounding_box()
        check("Temp has no head", temp.locator("header").count() == 0 and temp.locator("[aria-label='Back to the chat']").count() == 0)
        check("Temp's Memory floats at the top right", switch is not None and switch["y"] < 70 and switch["x"] + switch["width"] > 390 - 20, switch)
        page.screenshot(path=shots / "phone-temp-390.png")
        page.locator("[data-action=spaces]").click()
        page.wait_for_selector("[role=dialog][aria-label=Workspaces]")
        page.screenshot(path=shots / "phone-switcher-390.png")
        page.locator("[role=dialog][aria-label=Workspaces] [data-space=Main] button").first.click()
        page.wait_for_timeout(300)
        check("the switcher goes to a workspace", space(page) == "Main" and bar(page) == ["Chat", "Library", "Temp"], (space(page), bar(page)))

        y = page.locator(f"{BAR}").bounding_box()["y"] + 26
        swipe(page, cdp, 300, y, 120)
        check("a swipe along the bar goes to the next workspace", space(page) == "Shell", space(page))
        swipe(page, cdp, 120, y, 300)
        check("and back", space(page) == "Main", space(page))

        page.wait_for_timeout(1200)
        kept = LAYOUTS.read("phone-layout", None)["layout"] or {}
        check("the host keeps both workspaces", [w["name"] for w in kept.get("spaces", [])] == ["Main", "Shell"], kept)
        check("the wide page's store is untouched", LAYOUTS.read("layout", None)["layout"] is None)
        page.reload()
        page.wait_for_selector(f"[data-phone] {BAR}")
        page.wait_for_timeout(800)
        page.locator("[data-action=spaces]").click()
        names = page.locator("[role=dialog][aria-label=Workspaces] [data-space]").evaluate_all("(els) => els.map((e) => e.getAttribute('data-space'))")
        check("a reload keeps them", names == ["Main", "Shell"], names)
        page.keyboard.press("Escape")
        page.locator("[aria-hidden=true].absolute.inset-0.z-40").click()

        page.locator("[role=tab][data-item=library]").click()
        swipe(page, cdp, 100, 400, 320)
        check("a swipe right steps to the item left of it", shown_item(page) == ["Chat"], shown_item(page))
        swipe(page, cdp, 100, 400, 320)
        page.wait_for_timeout(400)
        opened = page.locator("[role=region][aria-label='Side panel']").count() > 0 and page.locator("[role=region][aria-label='Side panel']").is_visible()
        check("past the leftmost item a swipe opens the side panel", opened)
        panel = page.locator("[role=region][aria-label='Side panel']").inner_text()
        check("the side panel lists no Calendar or Today", "Calendar" not in panel and "Today" not in panel, panel)
        rows = page.locator("[role=region][aria-label='Side panel'] [data-open]").evaluate_all("(els) => els.map((e) => e.getAttribute('data-open'))")
        check("the side panel's rows: Hosts, Settings", rows == ["hosts", "settings"], rows)
        check("the side panel heads with the host's state", "Connected" in panel and "working" in panel, panel)
        check("the side panel ends in the theme and the version", page.locator("[role=region][aria-label='Side panel'] [data-action=theme]").count() == 1 and "Libri" in panel, panel)
        page.screenshot(path=shots / "phone-side-panel-390.png")
        check("no page errors", not errors, errors)
        context.close()
        browser.close()
    print("FAILED: " + ", ".join(failures) if failures else "all passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    if sys.argv[2:3] == ["serve"]:
        print(serve(int(sys.argv[3])), flush=True)
        threading.Event().wait()
    else:
        main()
