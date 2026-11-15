#!/usr/bin/env python3
"""A tile's menu (tileMenu.tsx), as Tern's pane menu: a right click in a tile
opens it at the pointer, its place's rows first (the chat's Search in chat,
Copy last answer and New Temp; a worker's Stop and Archive; a Library page's
Reload and Open in window), then Split right, Split down, Move to
workspace… and Close tile, with their keys. A right click in the composer
keeps the browser's own menu; one on the notch always opens the tile's. The
rows do what they say, and Alt Shift \\ and Alt Shift - split as the menu's
keys say. Any place opens again from the dock as a tile of its own, a second
terminal naming a shell of its own. Runs the built site against a stand-in
host in Chromium at 1440, light and dark, and leaves screenshots. Needs
Playwright and a browser, so it stays out of `nix flake check`.
Usage: check_tile_menu.py SITE_DIR SHOTS_DIR
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


# The stand-in host of real size answers the API (measure_ui_performance.py).
sys.path.insert(0, str(Path(__file__).resolve().parent))
_argv, sys.argv = sys.argv, [sys.argv[0], str(SITE), "serve"]
import measure_ui_performance as stand_in  # noqa: E402
sys.argv = _argv

# The chat, a worker's thread and a Library page side by side; a second
# workspace to move a tile to.
WIDE_LAYOUT = {"spaces": [
    {"id": "main", "name": "Main", "focus": "c", "full": False, "root": {
        "t": "split", "id": "s", "dir": "row", "at": 0.5,
        "a": {"t": "leaf", "id": "c", "pane": {"kind": "chat"}},
        "b": {"t": "split", "id": "s2", "dir": "col", "at": 0.5,
              "a": {"t": "leaf", "id": "t", "pane": {"kind": "thread", "arg": "w0"}},
              "b": {"t": "leaf", "id": "l", "pane": {"kind": "library", "arg": "page:plan-1"}}}}},
    {"id": "other", "name": "Shell", "focus": "o", "full": False, "root": {"t": "leaf", "id": "o", "pane": None}},
], "on": "main"}


class Host(stand_in.Host):
    def do_GET(self):
        path = urlparse(self.path).path
        if path.endswith("/api/layout"):
            if "since=" in self.path:
                time.sleep(5)
            try:
                return self.json({"version": 1, "layout": WIDE_LAYOUT, "by": None, "undo": False})
            except (BrokenPipeError, ConnectionResetError):
                return
        return super().do_GET()


def tiles(page, surface):
    return page.locator(f"section[data-tile][data-surface={surface}]").count()


def menu_at(page, where, shift=False):
    """Right clicks the middle of `where`; the menu's row names, or None."""
    box = page.locator(where).first.bounding_box()
    if shift:
        page.keyboard.down("Shift")
    page.mouse.click(box["x"] + box["width"] / 2, box["y"] + min(box["height"] / 2, 160), button="right")
    if shift:
        page.keyboard.up("Shift")
    page.wait_for_timeout(250)
    m = page.locator("[data-tile-menu]")
    if not m.count():
        return None
    return m.locator("[data-row]").evaluate_all("(els) => els.map((e) => e.getAttribute('data-row'))")


def shut(page):
    page.keyboard.press("Escape")
    page.wait_for_timeout(150)


def run(browser, site, theme):
    context = browser.new_context(viewport={"width": 1440, "height": 900}, service_workers="block", reduced_motion="reduce")
    # 60 s, as check_chat_steady allows: the release runs these while workers' builds
    # load the host, and a first page load there can pass 15 s.
    context.set_default_timeout(60000)
    context.grant_permissions(["clipboard-read", "clipboard-write"])
    if theme == "dark":
        context.add_init_script("try { localStorage.setItem('theme', 'dark') } catch {}")
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(site + "/")
    page.wait_for_selector("section[data-surface=chat] textarea")
    page.wait_for_selector("section[data-surface=library] iframe, section[data-surface=library] [data-page]", state="attached")
    page.wait_for_timeout(1200)
    tag = theme

    # The chat: its own rows, then what any tile does.
    started = time.monotonic()
    rows = menu_at(page, "section[data-surface=chat] [data-chat-scroll]")
    took = (time.monotonic() - started) * 1000 - 250
    check(f"{tag}: a right click in the chat opens its tile menu", rows is not None, str(rows))
    rows = rows or []
    check(f"{tag}: within 400 ms", took < 400, f"{took:.0f} ms")
    check(f"{tag}: the chat's rows first", rows[:3] == ["Search in chat", "Copy last answer", "New Temp"], rows)
    check(f"{tag}: then split, move and close", rows[3:] == ["Split right", "Split down", "Move to workspace…", "Close tile"], rows)
    keys = page.locator("[data-tile-menu] kbd").all_inner_texts()
    check(f"{tag}: the splits and close show their keys", "Alt Shift \\" in keys and "Alt Shift -" in keys and "Alt Shift X" in keys, keys)
    page.screenshot(path=SHOTS / f"chat-menu-{tag}.png")
    page.locator("[data-tile-menu] [data-row='Copy last answer']").click()
    copied = page.evaluate("() => navigator.clipboard.readText()")
    check(f"{tag}: Copy last answer copies the last answer", copied.startswith("Reply 59."), copied[:60])

    # The composer keeps the browser's own menu.
    rows = menu_at(page, "section[data-surface=chat] textarea")
    check(f"{tag}: a right click in the composer keeps the browser's menu", rows is None, str(rows))
    if rows is not None:
        shut(page)

    # A worker's thread and a Library page add their own rows.
    rows = menu_at(page, "section[data-surface=thread] [data-chat-scroll]") or []
    check(f"{tag}: a worker's thread offers Stop and Archive", rows[:2] == ["Stop", "Archive"], rows)
    shut(page)
    rows = menu_at(page, "section[data-surface=library] header") or []
    check(f"{tag}: a Library page offers Reload and Open in window", rows[:2] == ["Reload", "Open in window"], rows)
    shut(page)

    # The notch always opens it.
    notch = page.locator("section[data-surface=library] [data-notch]").first.bounding_box()
    page.mouse.move(notch["x"] + notch["width"] / 2, notch["y"] + 4)
    page.wait_for_timeout(150)
    page.mouse.click(notch["x"] + notch["width"] / 2, notch["y"] + 4, button="right")
    page.wait_for_timeout(250)
    check(f"{tag}: a right click on the notch opens the tile menu", page.locator("[data-tile-menu]").count() == 1)
    shut(page)

    # Split right from the menu adds a tile; Move to workspace takes a second step.
    before = page.locator("section[data-tile]").count()
    menu_at(page, "section[data-surface=chat] [data-chat-scroll]")
    page.locator("[data-tile-menu] [data-row='Split right']").click()
    page.wait_for_timeout(300)
    check(f"{tag}: Split right adds a tile", page.locator("section[data-tile]").count() == before + 1, str(page.locator("section[data-tile]").count()))
    menu_at(page, "section[data-surface=library] header")
    page.locator("[data-tile-menu] [data-row='Move to workspace…']").click()
    page.wait_for_timeout(200)
    step = page.locator("[data-tile-menu] [data-row]").evaluate_all("(els) => els.map((e) => e.getAttribute('data-row'))")
    check(f"{tag}: Move to workspace… lists the others and a new one", step == ["Shell", "New workspace"], step)
    page.screenshot(path=SHOTS / f"move-step-{tag}.png")
    page.locator("[data-tile-menu] [data-row=Shell]").click()
    page.wait_for_timeout(400)
    check(f"{tag}: the tile goes to that workspace, and the page with it", tiles(page, "library") == 1 and page.locator("section[data-tile][data-surface=chat]").count() == 0, str(tiles(page, "library")))
    page.keyboard.press("Alt+Shift+Digit1")
    page.wait_for_timeout(400)

    # The keys the menu names.
    page.locator("section[data-surface=chat] textarea").click()
    before = page.locator("section[data-tile]").count()
    page.keyboard.press("Alt+Shift+Backslash")
    page.wait_for_timeout(250)
    page.keyboard.press("Alt+Shift+Minus")
    page.wait_for_timeout(250)
    check(f"{tag}: Alt Shift \\ and Alt Shift - split", page.locator("section[data-tile]").count() == before + 2, str(page.locator("section[data-tile]").count()))

    # Any place opens again from the dock, a tile of its own.
    chats = tiles(page, "chat")
    page.locator("[aria-label=Dock] button[data-open=chat]").click()
    page.wait_for_timeout(300)
    check(f"{tag}: the dock opens the chat again beside the open one", tiles(page, "chat") == chats + 1, str(tiles(page, "chat")))
    for _ in range(2):
        page.locator("[aria-label=Dock] button[data-open=terminal]").click()
        page.wait_for_timeout(300)
    check(f"{tag}: and the terminal twice, two tiles", tiles(page, "terminal") == 2, str(tiles(page, "terminal")))
    page.screenshot(path=SHOTS / f"opened-again-{tag}.png")
    check(f"{tag}: no page errors", not errors, "; ".join(errors))
    context.close()


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Host, directory=str(SITE)))
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for theme in ("light", "dark"):
            run(browser, site, theme)
        browser.close()
    print(f"{total - len(failures)}/{total} passed")
    sys.exit(1 if failures else 0)


main()
