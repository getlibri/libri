#!/usr/bin/env python3
"""Closing a workspace keeps it (tiles.tsx, phone.tsx closeSpace; layout.rs
`closed`): against a throwaway Libri, on a wide page and on a phone, a
workspace closed goes under Recently closed on the host; for 3 seconds an
Undo stands where it was, and puts it back in its place; then the Undo goes.
Recently closed lists it with Reopen, which brings it back, and Delete, which
drops it for good. A page that leaves `closed` out of what it writes keeps
the host's. Each step within 400 ms. Leaves screenshots. Needs Playwright
and a browser, so it stays out of `nix flake check`.
Usage: check_workspace_close.py LIBRI_DIR SHOTS_DIR
"""
import json
import sys
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from throwaway_libri import Throwaway  # noqa: E402

LIBRI_DIR, SHOTS = sys.argv[1], Path(sys.argv[2])
failures = []
total = 0


def check(what, ok, detail=""):
    global total
    total += 1
    print(("ok   " if ok else "FAIL ") + what + ("" if ok or not detail else f": {detail}"), flush=True)
    if not ok:
        failures.append(what)


def soon(test, wait=10):
    end = time.monotonic() + wait
    while time.monotonic() < end:
        got = test()
        if got:
            return got
        time.sleep(0.1)
    return test()


def leaf(i, kind="chat"):
    return {"t": "leaf", "id": f"l{i}", "pane": {"kind": kind}}


WIDE = {"spaces": [{"id": f"w{i}", "name": name, "focus": f"l{i}", "full": False, "root": leaf(i, kind)}
                   for i, (name, kind) in enumerate([("Main", "chat"), ("Build", "library"), ("Notes", "chat")])], "on": "w0"}
PHONE = {"spaces": [{"id": f"p{i}", "name": name, "items": [{"kind": "chat"}]} for i, name in enumerate(["Main", "Build", "Notes"])], "on": "p0"}


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    t = Throwaway(LIBRI_DIR, prefix="libri-close-")

    def get(path):
        return json.loads(urllib.request.urlopen(t.base + path, timeout=20).read() or b"null")

    def post(path, body):
        req = urllib.request.Request(t.base + path, method="POST", data=json.dumps(body).encode(), headers={"content-type": "application/json"})
        return json.loads(urllib.request.urlopen(req, timeout=20).read() or b"null")

    def names(path):
        return [w["name"] for w in get(path)["layout"]["spaces"]]

    def closed(path):
        return [w["name"] for w in get(path)["layout"].get("closed", [])]

    def timed(what, act, until):
        start = time.monotonic()
        act()
        until()
        took = (time.monotonic() - start) * 1000
        check(f"{what} within 400 ms", took < 400, f"{took:.0f} ms")

    try:
        post("/api/layout", WIDE)
        post("/api/phone-layout", PHONE)
        with sync_playwright() as p:
            browser = p.chromium.launch()
            # The wide page.
            context = browser.new_context(viewport={"width": 1440, "height": 900}, service_workers="block")
            context.set_default_timeout(30000)
            page = context.new_page()
            errors = []
            page.on("pageerror", lambda e: errors.append(str(e)))
            page.goto(t.base + "/")
            tab = lambda name: page.locator(f"[aria-label=Workspaces] [data-space='{name}']")
            tab("Build").wait_for()
            page.wait_for_timeout(800)
            x = tab("Build").bounding_box()["x"]
            tab("Build").hover()
            timed("wide: closing a workspace", lambda: page.locator("[aria-label=Workspaces] [aria-label='Close Build']").click(),
                  lambda: page.locator("[data-action=space-undo]").wait_for())
            undo = page.locator("[data-action=space-undo]")
            check("wide: its Undo stands where it was", abs(undo.bounding_box()["x"] - x) < 2, f"{undo.bounding_box()['x']} vs {x}")
            check("wide: the host keeps it under closed", soon(lambda: closed("/api/layout") == ["Build"] and names("/api/layout") == ["Main", "Notes"]), str(get("/api/layout")["layout"]))
            page.screenshot(path=SHOTS / "wide-undo.png", clip={"x": 0, "y": 0, "width": 1440, "height": 64})
            timed("wide: Undo", lambda: undo.click(), lambda: tab("Build").wait_for())
            check("wide: Undo puts it back in its place", soon(lambda: names("/api/layout") == ["Main", "Build", "Notes"] and closed("/api/layout") == []), str(names("/api/layout")))
            # Left alone, the Undo goes after 3 seconds.
            tab("Notes").hover()
            page.locator("[aria-label=Workspaces] [aria-label='Close Notes']").click()
            page.wait_for_timeout(2500)
            check("wide: the Undo is still there at 2.5 s", undo.count() == 1)
            page.wait_for_timeout(900)
            check("wide: and gone after 3 s", undo.count() == 0)
            # Recently closed, from any workspace's right-click menu.
            timed("wide: the menu on a right-click", lambda: tab("Main").click(button="right"), lambda: page.locator("[data-recently-closed]").wait_for())
            check("wide: Recently closed lists it", page.locator("[data-recently-closed] [data-closed-space=Notes]").count() == 1)
            page.screenshot(path=SHOTS / "wide-recently-closed.png")
            timed("wide: Reopen", lambda: page.locator("[aria-label='Reopen Notes']").click(), lambda: tab("Notes").wait_for())
            check("wide: Reopen brings it back, shown", soon(lambda: names("/api/layout") == ["Main", "Build", "Notes"] and get("/api/layout")["layout"]["on"] == "w2"), str(get("/api/layout")["layout"]["on"]))
            tab("Notes").hover()
            page.locator("[aria-label=Workspaces] [aria-label='Close Notes']").click()
            tab("Main").click(button="right")
            page.locator("[aria-label='Delete Notes']").click()
            check("wide: Delete drops it for good", soon(lambda: closed("/api/layout") == [] and names("/api/layout") == ["Main", "Build"]), str(get("/api/layout")["layout"]))
            check("wide: no page errors", not errors, "; ".join(errors))
            context.close()

            # A phone.
            context = browser.new_context(viewport={"width": 390, "height": 844}, is_mobile=True, has_touch=True, service_workers="block")
            context.set_default_timeout(30000)
            page = context.new_page()
            errors = []
            page.on("pageerror", lambda e: errors.append(str(e)))
            page.goto(t.base + "/")
            page.locator("[data-action=spaces]").wait_for()
            page.wait_for_timeout(800)
            page.locator("[data-action=spaces]").click()
            sheet = page.locator("[role=dialog][aria-label=Workspaces]")
            row = lambda name: sheet.locator(f"[data-space='{name}']")
            y = row("Build").bounding_box()["y"]
            row("Build").locator("[data-action=space-menu]").click()
            timed("phone: closing a workspace", lambda: row("Build").locator("[data-action=space-close]").click(),
                  lambda: sheet.locator("[data-action=space-undo]").wait_for())
            undo = sheet.locator("[data-action=space-undo]")
            check("phone: its Undo stands where it was", abs(undo.bounding_box()["y"] - y) < 2, f"{undo.bounding_box()['y']} vs {y}")
            check("phone: the host keeps it under closed", soon(lambda: closed("/api/phone-layout") == ["Build"] and names("/api/phone-layout") == ["Main", "Notes"]), str(get("/api/phone-layout")["layout"]))
            page.screenshot(path=SHOTS / "phone-undo.png")
            timed("phone: Undo", lambda: undo.click(), lambda: row("Build").wait_for())
            check("phone: Undo puts it back in its place", soon(lambda: names("/api/phone-layout") == ["Main", "Build", "Notes"] and closed("/api/phone-layout") == []))
            row("Notes").locator("[data-action=space-menu]").click()
            row("Notes").locator("[data-action=space-close]").click()
            page.wait_for_timeout(3400)
            check("phone: the Undo goes after 3 s", undo.count() == 0)
            check("phone: Recently closed lists it", sheet.locator("[data-recently-closed] [data-closed-space=Notes]").count() == 1)
            page.screenshot(path=SHOTS / "phone-recently-closed.png")
            # An app that writes no `closed` leaves the host's as it was.
            soon(lambda: closed("/api/phone-layout") == ["Notes"])
            kept = get("/api/phone-layout")["layout"]
            post("/api/phone-layout", {"spaces": kept["spaces"], "on": kept["on"]})
            check("a writer without `closed` keeps the host's", closed("/api/phone-layout") == ["Notes"])
            timed("phone: Reopen", lambda: sheet.locator("[aria-label='Reopen Notes']").click(), lambda: page.locator("[data-action=spaces][title=Notes]").wait_for())
            check("phone: Reopen brings it back", soon(lambda: names("/api/phone-layout") == ["Main", "Build", "Notes"] and closed("/api/phone-layout") == []))
            check("phone: no page errors", not errors, "; ".join(errors))
            context.close()
            browser.close()
    finally:
        t.close()
    print(f"{total - len(failures)}/{total} passed")
    sys.exit(1 if failures else 0)


main()
