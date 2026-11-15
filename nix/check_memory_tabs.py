#!/usr/bin/env python3
"""A chat tile's memories as tabs (memoryTabs.tsx; tiles.rs chat `tabs`):
against a throwaway Libri with a memory, Tutor, on a wide page and on a
phone, the chat shows its tabs at its top and no memory pill in the
composer; the + lists the memories not open and New memory…; picking Tutor
opens its tab and shows its chat, kept in the layout as the pane's `arg` and
`tabs`; its composer says "Message Tutor" and the main one "Message" and the
assistant's name; Main's tab goes back; closing Tutor's tab keeps the
memory; New memory… makes one with its instructions and opens it. Each tap
within 400 ms. Leaves screenshots. Needs Playwright and a browser, so it
stays out of `nix flake check`.
Usage: check_memory_tabs.py LIBRI_DIR SHOTS_DIR
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


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    t = Throwaway(LIBRI_DIR, prefix="libri-memtabs-")

    def get(path):
        return json.loads(urllib.request.urlopen(t.base + path, timeout=20).read() or b"null")

    def post(path, body):
        req = urllib.request.Request(t.base + path, method="POST", data=json.dumps(body).encode(), headers={"content-type": "application/json"})
        return json.loads(urllib.request.urlopen(req, timeout=20).read() or b"null")

    def timed(what, act, until):
        start = time.monotonic()
        act()
        until()
        took = (time.monotonic() - start) * 1000
        check(f"{what} within 400 ms", took < 400, f"{took:.0f} ms")

    try:
        tutor = post("/api/memories", {"name": "Tutor"})["id"]
        post("/api/layout", {"spaces": [{"id": "w", "name": "Main", "focus": "c", "full": False,
                                         "root": {"t": "leaf", "id": "c", "pane": {"kind": "chat"}}}], "on": "w"})
        post("/api/phone-layout", {"spaces": [{"id": "p", "name": "Main", "items": [{"kind": "chat"}, {"kind": "library"}]}], "on": "p"})

        def pane():
            return get("/api/layout")["layout"]["spaces"][0]["root"]["pane"]

        with sync_playwright() as p:
            browser = p.chromium.launch()
            for width in (1440, 390):
                phone = width < 640
                tag = "phone" if phone else "wide"
                context = browser.new_context(viewport={"width": width, "height": 844 if phone else 900}, is_mobile=phone, has_touch=phone, service_workers="block")
                context.set_default_timeout(30000)
                page = context.new_page()
                errors = []
                page.on("pageerror", lambda e: errors.append(str(e)))
                page.goto(t.base + "/")
                tabs = page.locator("[data-chat-tabs]").filter(visible=True).first
                tabs.wait_for()
                page.wait_for_timeout(800)
                assistant = page.evaluate("document.querySelector('meta[name=assistant]')?.content ?? ''")
                box = page.locator("[data-surface=chat] textarea").first
                want = f"Message {assistant}" if assistant else "Send a message"
                check(f"{tag}: the main composer says {want!r}", box.get_attribute("placeholder") == want, box.get_attribute("placeholder"))
                check(f"{tag}: the chat's tabs show Main", tabs.locator("[data-memory-tab=main][aria-selected=true]").count() == 1)
                check(f"{tag}: no memory pill in the composer", page.locator("[data-surface=chat] form button[title=Memory]").count() == 0)
                timed(f"{tag}: the + opens its list", lambda: tabs.locator("[data-action=memory-add]").click(), lambda: page.locator("[data-memory-menu]").wait_for())
                listed = page.locator("[data-memory-menu]").inner_text()
                check(f"{tag}: it lists Tutor and New memory…", "Tutor" in listed and "New memory" in listed, listed)
                page.screenshot(path=SHOTS / f"{tag}-plus.png")
                timed(f"{tag}: Tutor's tab", lambda: page.locator(f"[data-memory-menu] [data-open-memory='{tutor}']").click(),
                      lambda: page.locator(f"[data-memory='{tutor}'] textarea").filter(visible=True).first.wait_for())
                check(f"{tag}: Tutor's composer says 'Message Tutor'", page.locator(f"[data-memory='{tutor}'] textarea").filter(visible=True).first.get_attribute("placeholder") == "Message Tutor")
                if not phone:
                    check("wide: the tile keeps both tabs, Tutor shown", soon(lambda: pane() == {"kind": "chat", "arg": tutor, "tabs": ["", tutor]}), str(pane()))
                else:
                    item = lambda: get("/api/phone-layout")["layout"]["spaces"][0]["items"][0]
                    check("phone: the item keeps both tabs, Tutor shown", soon(lambda: item() == {"kind": "chat", "arg": tutor, "tabs": ["", tutor]}), str(item()))
                page.screenshot(path=SHOTS / f"{tag}-tutor.png")
                tabs = page.locator("[data-chat-tabs]").filter(visible=True).first
                timed(f"{tag}: back to Main's tab", lambda: tabs.locator("[data-memory-tab=main]").click(),
                      lambda: page.locator("[data-surface=chat] textarea").filter(visible=True).first.wait_for())
                tabs = page.locator("[data-chat-tabs]").filter(visible=True).first
                tabs.locator(f"[data-memory-tab='{tutor}']").hover()
                tabs.locator("[aria-label='Close Tutor']").click()
                check(f"{tag}: closing Tutor's tab keeps the memory", soon(lambda: tabs.locator(f"[data-memory-tab='{tutor}']").count() == 0) and any(m["id"] == tutor for m in get("/api/memories")["memories"]))
                if not phone:
                    check("wide: and the tile is back to Main alone", soon(lambda: pane() == {"kind": "chat"}), str(pane()))
                    tabs.locator("[data-action=memory-add]").click()
                    page.locator("[data-action=memory-new]").click()
                    page.get_by_label("New memory's name").fill("Jobs")
                    page.get_by_label("New memory's instructions").fill("Keep the job search going.")
                    page.locator("[data-memory-menu] button[type=submit]").click()
                    jobs = soon(lambda: next((m for m in get("/api/memories")["memories"] if m["name"] == "Jobs" and m.get("instructions")), None))
                    check("wide: New memory… makes Jobs with its instructions", bool(jobs) and jobs.get("instructions") == "Keep the job search going.", str(jobs))
                    check("wide: and opens its tab", bool(jobs) and soon(lambda: pane().get("arg") == jobs["id"]), str(pane()))
                    page.screenshot(path=SHOTS / "wide-jobs.png")
                check(f"{tag}: no page errors", not errors, "; ".join(errors))
                context.close()
            browser.close()
    finally:
        t.close()
    print(f"{total - len(failures)}/{total} passed")
    sys.exit(1 if failures else 0)


main()
