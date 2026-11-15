#!/usr/bin/env python3
"""Several Temps at once (scratchpad.rs, App.tsx Temp): against a throwaway
Libri, a wide page with a Temp tile opens a second Temp from the dock, which
becomes a Temp of its own on the host, held by its id in the layout; words to
each reach only its own Temp; the first keeps its talk across a reload; Clear
in the second gives that tile a fresh Temp; closing the second's tile ends its
Temp on the host and leaves the first. Leaves screenshots. Needs Playwright
and a browser, so it stays out of `nix flake check`.
Usage: check_temps.py LIBRI_DIR SHOTS_DIR
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


def soon(test, wait=30):
    end = time.monotonic() + wait
    while time.monotonic() < end:
        got = test()
        if got:
            return got
        time.sleep(0.3)
    return test()


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    t = Throwaway(LIBRI_DIR, prefix="libri-temps-")

    def get(path):
        return json.loads(urllib.request.urlopen(t.base + path, timeout=20).read() or b"null")

    def post(path, body):
        req = urllib.request.Request(t.base + path, method="POST", data=json.dumps(body).encode(), headers={"content-type": "application/json"})
        return json.loads(urllib.request.urlopen(req, timeout=20).read() or b"null")

    def temp_tiles():
        l = get("/api/layout")["layout"] or {}
        out = []

        def walk(n):
            if n.get("t") == "leaf":
                p = n.get("pane") or {}
                if p.get("kind") == "temp":
                    out.append(p.get("arg"))
            else:
                walk(n["a"])
                walk(n["b"])
        for w in l.get("spaces", []):
            walk(w["root"])
        return out

    try:
        # The chat beside one Temp tile, as the person left it.
        post("/api/layout", {"spaces": [{"id": "w", "name": "Main", "focus": "c", "full": False, "root": {
            "t": "split", "id": "s", "dir": "row", "at": 0.5,
            "a": {"t": "leaf", "id": "c", "pane": {"kind": "chat"}},
            "b": {"t": "leaf", "id": "m", "pane": {"kind": "temp"}}}}], "on": "w"})
        with sync_playwright() as p:
            browser = p.chromium.launch()
            context = browser.new_context(viewport={"width": 1440, "height": 900}, service_workers="block")
            context.set_default_timeout(60000)
            page = context.new_page()
            errors = []
            page.on("pageerror", lambda e: errors.append(str(e)))
            page.goto(t.base + "/")
            page.wait_for_selector("section[data-surface=temp] textarea")
            page.wait_for_timeout(1500)

            # The first Temp hears words.
            first = page.locator("section[data-surface=temp]").nth(0)
            first.locator("textarea").fill("first temp words")
            first.locator("textarea").press("Enter")
            a = soon(lambda: get("/api/scratchpad").get("id"))
            check("the first Temp is the host's current one", bool(a), str(a))

            # The dock opens a second Temp, a Temp of its own, held by its id.
            page.locator("[aria-label=Dock] button[data-open=temp]").click()
            held = soon(lambda: [x for x in temp_tiles() if x and x.startswith("scratchpad-")])
            check("the dock opens a second Temp tile holding a new Temp", len(page.locator("section[data-surface=temp]").all()) == 2 and len(held) == 1 and held[0] != a, str(temp_tiles()))
            b = held[0] if held else None
            second = page.locator("section[data-surface=temp]").nth(1)
            second.locator("textarea").fill("second temp words")
            second.locator("textarea").press("Enter")
            check("words to the second reach only its Temp",
                  soon(lambda: "second temp words" in get(f"/api/scratchpad?id={b}")["text"]) and "second temp words" not in get("/api/scratchpad")["text"],
                  get(f"/api/scratchpad?id={b}")["text"][:200])
            check("and the first keeps its own", "first temp words" in get("/api/scratchpad")["text"] and "first temp words" not in get(f"/api/scratchpad?id={b}")["text"])
            page.screenshot(path=SHOTS / "two-temps.png")

            # A reload keeps each tile's Temp.
            page.reload()
            page.wait_for_selector("section[data-surface=temp] textarea")
            soon(lambda: page.locator("section[data-surface=temp] >> text=second temp words").count() > 0, 20)
            check("a reload keeps each tile's Temp",
                  page.locator("section[data-surface=temp]").nth(0).locator("text=first temp words").count() > 0
                  and page.locator("section[data-surface=temp]").nth(1).locator("text=second temp words").count() > 0)

            # Clear in the second gives that tile a fresh Temp.
            second = page.locator("section[data-surface=temp]").nth(1)
            second.locator("button[title='Clear the temp chat']").click()
            fresh = soon(lambda: [x for x in temp_tiles() if x and x.startswith("scratchpad-") and x != b])
            check("Clear gives that tile a fresh Temp, the old one gone", bool(fresh) and get(f"/api/scratchpad?id={b}")["id"] is None, str(temp_tiles()))
            c = fresh[0] if fresh else None

            # Closing the second's tile ends its Temp, and leaves the first.
            tile = page.locator("section[data-surface=temp]").nth(1)
            box = tile.locator("[data-notch]").first.bounding_box()
            page.mouse.move(box["x"] + box["width"] / 2, box["y"] + 4)
            page.wait_for_timeout(150)
            page.mouse.click(box["x"] + box["width"] / 2, box["y"] + 4, button="right")
            page.locator("[data-tile-menu] [data-row-id=close]").click()
            check("closing its tile ends that Temp on the host", soon(lambda: get(f"/api/scratchpad?id={c}")["id"] is None) is True or get(f"/api/scratchpad?id={c}")["id"] is None)
            check("and leaves the first", get("/api/scratchpad")["id"] == a)
            page.screenshot(path=SHOTS / "one-left.png")
            check("no page errors", not errors, "; ".join(errors))
            browser.close()
    finally:
        t.close()
    print(f"{total - len(failures)}/{total} passed")
    sys.exit(1 if failures else 0)


main()
