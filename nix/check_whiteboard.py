#!/usr/bin/env python3
"""The Whiteboard tile (2026-11-13): boards the person draws on and the
agent writes to. Checks, against the real daemon and its real tool server in
a scratch directory, with the built site in Chromium:

- "jot this down" (board_note) on an empty host makes the board Notes with
  the note on it, and the open tile shows a second note live, no reload;
- a stroke drawn in the tile reaches the host, and the agent reads it;
- two workspaces keep their own tab: switching shows each its own board;
- tabs: a new board, a rename, and the tab order;
- a second browser on the same board sees the first one's stroke live;
- export gives an .excalidraw file;
- the tile on a phone-width page;
- board links, on their own listener: a view link opens the board read-only
  and follows the agent's notes live, an edit link's stroke reaches the
  person's board, the listener serves nothing else, and a link taken back
  opens nothing;
- WebKit at iPhone size: a Whiteboard item with no board opens without a
  page error and keeps the board it opened on.

Needs Playwright and its Chromium, so it stays out of `nix flake check`.
Usage: check_whiteboard.py LIBRI_DIR (holding bin/libri and site/) [SHOTS_DIR]
"""

import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

LIBRI = Path(sys.argv[1]) / "bin/libri"
SITE = Path(sys.argv[1]) / "site"
SHOTS = Path(sys.argv[2]) if len(sys.argv) > 2 else None
failures = []
# Any of the browser's own dialogs (confirm, alert, prompt) is a failure:
# the page asks in Libri's own (site/src/dialog.tsx).
native_dialogs = []


def guarded(page):
    def seen(d):
        native_dialogs.append(f"{d.type}: {d.message}")
        d.dismiss()
    page.on("dialog", seen)
    return page


def check(what, ok, detail=""):
    print(f"ok   {what}" if ok else f"FAIL {what} {detail}", flush=True)
    if not ok:
        failures.append(what)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def until(test, what, limit=30.0):
    end = time.time() + limit
    while time.time() < end:
        try:
            if test():
                return True
        except Exception:
            pass
        time.sleep(0.1)
    check(what, False, "timed out")
    return False


class Tools:
    """Libri's tools at the person's scope, as a turn's harness reaches them."""

    def __init__(self, env, state):
        self.proc = subprocess.Popen([LIBRI, "tools", str(state)], env=env | {"LIBRI_SCOPE": "person"},
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.id = 0
        self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                "clientInfo": {"name": "check", "version": "0"}})
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.proc.stdin.flush()

    def rpc(self, method, params):
        self.id += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        return json.loads(self.proc.stdout.readline())

    def call(self, name, arguments):
        r = self.rpc("tools/call", {"name": name, "arguments": arguments})["result"]
        return r["content"][0]["text"], r.get("isError", False)


def get(url):
    with urllib.request.urlopen(url, timeout=10) as r:
        return json.loads(r.read())


def post(url, body):
    req = urllib.request.Request(url, data=json.dumps(body).encode(), headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read() or b"null")


def leaf(i, pane):
    return {"t": "leaf", "id": i, "pane": pane}


# Every workspace's tiles stay mounted (switching never remounts them), so
# only the one on screen counts.
SHOWN = "[data-board]:visible"


def drawn(page):
    v = page.locator(SHOWN).first.get_attribute("data-elements", timeout=2000)
    return int(v) if v else 0


def board_on(page):
    return page.locator(SHOWN).first.get_attribute("data-board", timeout=2000)


def shot(page, name):
    if SHOTS:
        SHOTS.mkdir(parents=True, exist_ok=True)
        page.screenshot(path=SHOTS / f"{name}.png")


def main():
    root = Path(tempfile.mkdtemp(prefix="libri-whiteboard-"))
    for d in ("home", "state", "hippo"):
        (root / d).mkdir()
    port = free_port()
    url = f"http://127.0.0.1:{port}"
    share_port = free_port()
    share = f"http://127.0.0.1:{share_port}"
    env = {
        "HOME": str(root / "home"),
        "PATH": os.environ["PATH"],
        "HIPPO_DIR": str(root / "hippo"),
        "STATE_DIRECTORY": str(root / "state"),
        "LIBRI_NAME": "box-label",
        "LIBRI_PERSON": "Ada",
        "LIBRI_PAGES": str(root / "state/pages"),
        "LIBRI_PAGES_URL": url,
    }
    daemon = subprocess.Popen([LIBRI], env=env | {
        "LIBRI_BACKEND": "claude",
        "LIBRI_CLAUDE": "/bin/false",
        "LIBRI_MODEL": "stand-in",
        "LIBRI_SITE": f"127.0.0.1:{port}",
        "LIBRI_SITE_URL": url,
        "LIBRI_SITE_ROOT": str(SITE),
        "LIBRI_OPEN": "1",
        "LIBRI_SHARE_LISTEN": f"127.0.0.1:{share_port}",
        "LIBRI_SHARE_URL": share,
    }, stdout=subprocess.DEVNULL, stderr=open(root / "libri.log", "w"))

    def up():
        try:
            return urllib.request.urlopen(f"{url}/api/host", timeout=2).status == 200
        except OSError:
            return False

    try:
        if not until(up, "libri up"):
            return
        tools = Tools(env, root / "state")

        # "Jot this down" on a host with no boards makes Notes.
        said, bad = tools.call("board_note", {"text": "Buy milk"})
        check("jot this down makes the board Notes", not bad and "Notes" in said, said)
        boards = get(f"{url}/api/boards")
        notes = boards[0]["id"]
        plans = post(f"{url}/api/boards", {"name": "Plans"})["id"]
        said, _ = tools.call("board_read", {"board": "Notes"})
        check("the agent reads the note back", "Buy milk" in said and "note" in said, said)

        # Two workspaces, each with a whiteboard tile on its own board.
        layout = {"spaces": [
            {"id": "w1", "name": "One", "root": leaf("l1", {"kind": "board", "arg": notes}), "focus": "l1", "full": False},
            {"id": "w2", "name": "Two", "root": leaf("l2", {"kind": "board", "arg": plans}), "focus": "l2", "full": False},
        ], "on": "w1"}
        post(f"{url}/api/layout", layout)

        with sync_playwright() as p:
            browser = p.chromium.launch()
            page = guarded(browser.new_page(viewport={"width": 1440, "height": 900}))
            t0 = time.time()
            page.goto(url)
            until(lambda: page.query_selector("[data-board]:visible canvas") is not None, "the tile draws its canvas", 30)
            check(f"the board opens ({time.time() - t0:.1f} s, canvas loaded on first open)", True)
            until(lambda: drawn(page) >= 2, "the note shows on the board", 15)
            check("the tile shows Notes with the agent's note", board_on(page) == notes)
            sel = page.query_selector("[aria-label=Boards] [role=tab][aria-selected=true]:visible")
            check("the Notes tab is the selected one", sel is not None and "Notes" in sel.inner_text(), sel and sel.inner_text())
            shot(page, "1-notes")

            # The agent jots again while the board is open: it shows live.
            t0 = time.time()
            tools.call("board_note", {"text": "Call the plumber", "colour": "pink"})
            ok = until(lambda: drawn(page) >= 4, "the second note shows live", 10)
            if ok:
                check(f"the agent's second note shows live ({time.time() - t0:.2f} s)", time.time() - t0 < 3)
            shot(page, "2-jotted-live")

            # The person draws a rectangle.
            box = page.locator(SHOWN).first.bounding_box()
            x, y = box["x"] + box["width"] * 0.3, box["y"] + box["height"] * 0.7
            page.mouse.click(x - 40, y - 40)
            page.keyboard.press("r")
            page.mouse.move(x, y)
            page.mouse.down()
            page.mouse.move(x + 160, y + 90, steps=8)
            page.mouse.up()
            until(lambda: sum(1 for e in get(f"{url}/api/boards/{notes}")["elements"]
                              if e["type"] == "rectangle" and not e.get("customData", {}).get("note")) == 1,
                  "the stroke reaches the host", 10)
            said, _ = tools.call("board_read", {"board": notes})
            check("the agent reads the person's rectangle", "rectangle" in said, said)
            shot(page, "3-drawn")

            # A second browser on the same board sees a new stroke live.
            other = guarded(browser.new_page(viewport={"width": 1200, "height": 800}))
            other.goto(url)
            until(lambda: other.query_selector("[data-board]:visible canvas") is not None and drawn(other) >= 5, "the second view opens", 30)
            before = drawn(other)
            page.keyboard.press("o")
            page.mouse.move(x + 260, y)
            page.mouse.down()
            page.mouse.move(x + 360, y + 80, steps=6)
            page.mouse.up()
            t0 = time.time()
            ok = until(lambda: drawn(other) > before, "the other view sees the ellipse", 10)
            if ok:
                check(f"another view sees the stroke live ({time.time() - t0:.2f} s)", time.time() - t0 < 3)
            other.close()

            # Workspace Two shows Plans; back on One, Notes again.
            page.keyboard.press("Escape")
            page.locator("[data-space=Two]").click()
            until(lambda: board_on(page) == plans, "workspace Two shows Plans", 10)
            check("workspace Two keeps its own tab (Plans)", board_on(page) == plans)
            page.locator("[data-space=One]").first.click()
            until(lambda: board_on(page) == notes, "workspace One shows Notes again", 10)
            check("workspace One keeps Notes", board_on(page) == notes)
            saved = get(f"{url}/api/layout")
            panes = {w["id"]: w["root"]["pane"] for w in saved["layout"]["spaces"]} if "layout" in saved else {w["id"]: w["root"]["pane"] for w in saved["spaces"]}
            check("the layout keeps each workspace's board", panes["w1"]["arg"] == notes and panes["w2"]["arg"] == plans, panes)

            # Zoom on the board is the canvas's: ctrl+wheel (a trackpad's
            # pinch) and Ctrl +/- zoom the board, never the chat's text
            # (pinch.ts, data-own-zoom); ctrl+wheel elsewhere still sizes it.
            # On a workspace with the chat beside the board, as the person has it.
            kept = get(f"{url}/api/layout")
            lay = kept.get("layout") or kept
            lay["spaces"].append({"id": "w3", "name": "Three", "focus": "l3b", "full": False,
                                  "root": {"t": "split", "id": "s3", "dir": "row", "at": 0.5,
                                           "a": leaf("l3a", {"kind": "chat"}), "b": leaf("l3b", {"kind": "board", "arg": notes})}})
            lay["on"] = "w3"
            post(f"{url}/api/layout", lay)
            # Wide enough that the board, half the page, keeps its zoom control.
            was = page.viewport_size
            page.set_viewport_size({"width": 2000, "height": 1000})
            page.reload()
            until(lambda: page.locator("[data-board]:visible canvas").count() > 0, "the chat and the board side by side", 30)
            talk = lambda: page.evaluate("getComputedStyle(document.documentElement).getPropertyValue('--talk').trim()")
            zoom = lambda: page.locator("[data-board]:visible").first.get_by_title("Reset zoom").inner_text()
            talk0, zoom0 = talk(), zoom()
            box = page.locator(SHOWN).first.bounding_box()
            page.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
            page.keyboard.down("Control")
            page.mouse.wheel(0, -300)
            page.keyboard.up("Control")
            time.sleep(0.5)
            check("ctrl+wheel on the board zooms the board", zoom() != zoom0, (zoom0, zoom()))
            check("ctrl+wheel on the board leaves the chat's text size", talk() == talk0, (talk0, talk()))
            zoom1 = zoom()
            page.mouse.click(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
            page.keyboard.press("Control+Equal")
            time.sleep(0.5)
            check("Ctrl + with focus on the board zooms the board", zoom() != zoom1, (zoom1, zoom()))
            check("Ctrl + on the board leaves the chat's text size", talk() == talk0, (talk0, talk()))
            chat = page.locator("[data-chat-scroll]:visible").first.bounding_box()
            page.mouse.move(chat["x"] + chat["width"] / 2, chat["y"] + chat["height"] / 2)
            page.keyboard.down("Control")
            page.mouse.wheel(0, -120)
            page.keyboard.up("Control")
            time.sleep(0.3)
            check("ctrl+wheel on the chat still sizes the chat's text", talk() != talk0, (talk0, talk()))
            page.evaluate("localStorage.removeItem('talk-size')")
            lay["spaces"] = [w for w in lay["spaces"] if w["id"] != "w3"]
            lay["on"] = "w1"
            post(f"{url}/api/layout", lay)
            page.set_viewport_size(was)
            page.reload()
            until(lambda: board_on(page) == notes, "back on workspace One", 30)

            # The tile's notch (move, split, full screen, close) works over a
            # board as over any tile: nothing of the board covers it.
            notch = page.locator("[data-notch]:visible").first
            nb = notch.bounding_box()
            on_top = page.evaluate("([x, y]) => !!document.elementFromPoint(x, y)?.closest('[data-notch]')",
                                   [nb["x"] + nb["width"] / 2, nb["y"] + 4])
            check("the tile's notch is on top of the board", on_top)
            page.mouse.move(nb["x"] + nb["width"] / 2, nb["y"] + 4)
            split = page.locator("[data-notch]:visible [data-action='split-h']").first
            check("the notch opens over the board", until(lambda: split.is_visible(), "the notch's controls show", 5))
            before = json.dumps(get(f"{url}/api/layout"))
            split.click()
            check("the notch's split works on a board tile",
                  until(lambda: '"split"' in json.dumps(get(f"{url}/api/layout")) and json.dumps(get(f"{url}/api/layout")) != before,
                        "the board tile split", 5))
            post(f"{url}/api/layout/undo", {})
            until(lambda: board_on(page) == notes, "back to the board alone", 10)

            # A new board from the tabs, renamed, opens in this tile.
            page.click("button[aria-label='New board']")
            until(lambda: page.query_selector("input[aria-label='Board name']") is not None, "the new board's name is offered", 5)
            page.fill("input[aria-label='Board name']", "House")
            page.keyboard.press("Enter")
            until(lambda: any(b["name"] == "House" for b in get(f"{url}/api/boards")), "the board is named House", 5)
            house = next(b["id"] for b in get(f"{url}/api/boards") if b["name"] == "House")
            until(lambda: board_on(page) == house, "the tile shows House", 10)
            check("a new board opens in the tile under its name", True)
            check("the tabs list the boards in order",
                  [t.inner_text().replace("×", "").strip() for t in page.query_selector_all("[aria-label=Boards] [role=tab]:visible")] == ["Notes", "Plans", "House"],
                  [t.inner_text() for t in page.query_selector_all("[aria-label=Boards] [role=tab]:visible")])
            shot(page, "4-tabs")

            # Deleting a board asks in Libri's own confirm: Esc keeps it,
            # "Delete board" deletes it.
            page.click("button[aria-label='Delete House']")
            asked = page.locator("[role=alertdialog]")
            check("deleting a board asks in Libri's own dialog",
                  until(lambda: asked.is_visible(), "the confirm shows", 5) and asked.get_by_role("button", name="Delete board").is_visible())
            shot(page, "4b-confirm")
            page.keyboard.press("Escape")
            check("Esc keeps the board", until(lambda: not asked.is_visible(), "the confirm closes", 5)
                  and any(b["name"] == "House" for b in get(f"{url}/api/boards")))
            page.click("button[aria-label='Delete House']")
            until(lambda: asked.is_visible(), "the confirm shows again", 5)
            asked.get_by_role("button", name="Delete board").click()
            check("Delete board deletes it", until(lambda: not any(b["name"] == "House" for b in get(f"{url}/api/boards")), "House gone", 5))

            # Export: the board as an .excalidraw file.
            f = get(f"{url}/api/boards/{notes}/export")
            check("export gives an .excalidraw file with the notes", f["type"] == "excalidraw"
                  and any(e.get("text") == "Buy milk" for e in f["elements"]), f.get("type"))

            # Dark.
            page.evaluate("() => document.documentElement.dataset.theme = 'dark'")
            time.sleep(0.8)
            shot(page, "5-dark")

            # A phone.
            phone = guarded(browser.new_page(viewport={"width": 390, "height": 844}, is_mobile=True, has_touch=True))
            phone.goto(url)
            post(f"{url}/api/phone-layout", {"spaces": [{"id": "p1", "name": "Board", "items": [{"kind": "board", "arg": notes}], "on": 0}], "on": "p1"})
            phone.reload()
            until(lambda: phone.query_selector("[data-board]:visible canvas") is not None, "the phone shows the board", 30)
            check("the tile works on a phone", drawn(phone) >= 5, drawn(phone))
            shot(phone, "6-phone")
            # A sideways drag on the board is the canvas's, not the page's
            # swipe to the next item (work.tsx keepsSwipe, data-own-swipe).
            post(f"{url}/api/phone-layout", {"spaces": [{"id": "p1", "name": "Board", "items": [{"kind": "chat"}, {"kind": "board", "arg": notes}]}], "on": "p1"})
            phone.reload()
            phone.get_by_text("Whiteboard", exact=True).first.tap()
            until(lambda: phone.query_selector("[data-board]:visible canvas") is not None, "the phone shows the board again", 30)
            box = phone.locator("[data-board]:visible").first.bounding_box()
            cdp = phone.context.new_cdp_session(phone)
            def drag(x0, x1, y):
                cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [{"x": x0, "y": y}]})
                for i in range(1, 9):
                    cdp.send("Input.dispatchTouchEvent", {"type": "touchMove", "touchPoints": [{"x": x0 + (x1 - x0) * i / 8, "y": y}]})
                cdp.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
            drag(box["x"] + 40, box["x"] + box["width"] - 40, box["y"] + box["height"] / 2)
            drag(box["x"] + box["width"] - 40, box["x"] + 40, box["y"] + box["height"] / 2 + 40)
            time.sleep(0.8)
            check("a drag on the phone's board stays on the board", phone.query_selector("[data-board]:visible canvas") is not None)
            post(f"{url}/api/phone-layout", {"spaces": [{"id": "p1", "name": "Board", "items": [{"kind": "board", "arg": notes}], "on": 0}], "on": "p1"})
            phone.reload()
            until(lambda: phone.query_selector("[data-board]:visible canvas") is not None, "the phone shows the board alone", 30)
            # A board picked on the phone is the workspace's own, as on
            # Android and the wide page, not a view lying over the item.
            phone.locator("[aria-label=Boards] [role=tab]:visible", has_text="Plans").first.tap()
            def phone_board():
                kept = get(f"{url}/api/phone-layout")["layout"]
                space = next(w for w in kept["spaces"] if w["id"] == "p1")
                return space["items"][0].get("arg")
            check("a board picked on the phone is the workspace's own",
                  until(lambda: phone_board() == plans, "the phone keeps Plans", 10), phone_board())
            phone.reload()
            check("the phone opens on the board it keeps",
                  until(lambda: board_on(phone) == plans, "the phone reopens on Plans", 20), board_on(phone))

            # Board links.
            view = post(f"{url}/api/boards/{notes}/links", {"role": "view", "days": 7})
            edit = post(f"{url}/api/boards/{notes}/links", {"role": "edit"})
            check("a link opens at the share address", (view.get("url") or "").startswith(f"{share}/#board="), view)
            listed = get(f"{url}/api/boards/{notes}/links")
            check("the board lists its two links", listed["public"] and len(listed["links"]) == 2, listed)

            def share_status(path, token=None, method="GET", body=None):
                h = {"x-share-token": token} if token else {}
                if body is not None:
                    h["content-type"] = "application/json"
                req = urllib.request.Request(f"{share}{path}", method=method, headers=h,
                                             data=json.dumps(body).encode() if body is not None else None)
                try:
                    with urllib.request.urlopen(req, timeout=10) as r:
                        return r.status
                except urllib.error.HTTPError as e:
                    return e.code

            check("the share listener serves no other route",
                  share_status("/api/boards") == 404 and share_status(f"/api/boards/{notes}") == 404
                  and share_status("/api/host") == 404, "")
            check("a made-up token opens nothing", share_status("/api/share/board", "made-up") == 404)
            check("a view link may not change the board",
                  share_status("/api/share/board", view["token"], "POST", {"ops": []}) == 403)

            viewer = guarded(browser.new_page(viewport={"width": 1200, "height": 800}))
            viewer.goto(view["url"])
            until(lambda: viewer.query_selector("[data-board] canvas") is not None, "the view link opens the board", 30)
            before = drawn(viewer)
            check("the view link shows the board read-only",
                  before >= 5 and viewer.get_by_text("View only").count() == 1, before)
            tools.call("board_note", {"text": "Seen by the link", "board": "Notes"})
            check("the view link follows a new note live", until(lambda: drawn(viewer) > before, "live note on the view link", 10))
            shot(viewer, "7-view-link")

            editor = guarded(browser.new_page(viewport={"width": 1200, "height": 800}))
            editor.goto(edit["url"])
            until(lambda: editor.query_selector("[data-board] canvas") is not None, "the edit link opens the board", 30)
            had = len([e for e in get(f"{url}/api/boards/{notes}")["elements"] if not e.get("isDeleted")])
            editor.mouse.click(660, 660)
            editor.keyboard.press("r")
            editor.mouse.move(700, 500)
            editor.mouse.down()
            editor.mouse.move(860, 590, steps=8)
            editor.mouse.up()
            check("an edit link's stroke reaches the person's board",
                  until(lambda: len([e for e in get(f"{url}/api/boards/{notes}")["elements"] if not e.get("isDeleted")]) > had,
                        "the edit link's rectangle on the host", 10))
            shot(editor, "8-edit-link")

            post(f"{url}/api/boards/links/{view['token']}/revoke", {})
            check("a link taken back opens nothing", share_status("/api/share/board", view["token"]) == 404)
            check("the other link still opens", share_status("/api/share/board", edit["token"]) == 200)
            browser.close()

            # An iPhone (WebKit, 390x844 at 3x): a Whiteboard item added with
            # no board yet opens on the first board, keeps it, and never
            # loops on its pick (React's "maximum update depth", which took
            # the whole page down on Safari).
            ios = p.webkit.launch()
            ctx = ios.new_context(viewport={"width": 390, "height": 844}, device_scale_factor=3, is_mobile=True, has_touch=True)
            phone = guarded(ctx.new_page())
            errors = []
            phone.on("pageerror", lambda e: errors.append(str(e)))
            post(f"{url}/api/phone-layout", {"spaces": [{"id": "p1", "name": "Main", "items": [{"kind": "chat"}, {"kind": "board"}]}], "on": "p1"})
            phone.goto(url)
            phone.get_by_text("Whiteboard", exact=True).first.tap()
            check("WebKit on an iPhone opens a Whiteboard with no board yet",
                  until(lambda: phone.query_selector("[data-board]:visible canvas") is not None, "the iPhone shows the board", 30))
            time.sleep(2)
            check("WebKit on an iPhone raises no page error", not errors, errors[:2])
            items = lambda: get(f"{url}/api/phone-layout")["layout"]["spaces"][0]["items"]
            # The pick is saved after the board opens, later on a loaded host.
            until(lambda: any(i["kind"] == "board" and i.get("arg") for i in items()), "the iPhone's pick is kept", 15)
            check("the iPhone's Whiteboard item keeps the board it opened on", any(i["kind"] == "board" and i.get("arg") for i in items()), items())
            shot(phone, "9-iphone")
            ios.close()
    finally:
        daemon.terminate()
    check("no browser dialog appeared", not native_dialogs, native_dialogs)
    if failures:
        print(f"{len(failures)} failed: {failures}")
        sys.exit(1)
    print("all passed")


main()
