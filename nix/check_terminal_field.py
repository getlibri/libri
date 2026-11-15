#!/usr/bin/env python3
"""The terminal's own field (ghostty-web's textarea, which takes keys,
pastes and input methods) is never seen: no box and no blinking caret at
the tile's corner, before and after a right-click (ghostty-web restyles the
field inline for its paste), and it sits on the cursor's cell once text is
typed, where an input method's window should open. On a wide page and a
phone, against a throwaway Libri.

Needs Playwright and its Chromium, so it stays out of `nix flake check`.
Usage: check_terminal_field.py LIBRI_DIR (holding bin/libri and site/) [SHOTS_DIR]
"""

import json
import sys
import time
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from throwaway_libri import Throwaway  # noqa: E402
from playwright.sync_api import sync_playwright  # noqa: E402

SHOTS = Path(sys.argv[2]) if len(sys.argv) > 2 else None
failures = []


def check(what, ok, detail=""):
    print(f"ok   {what}" if ok else f"FAIL {what} {detail}", flush=True)
    if not ok:
        failures.append(what)


def post(base, path, body):
    req = urllib.request.Request(f"{base}/{path}", json.dumps(body).encode(), {"content-type": "application/json"})
    return urllib.request.urlopen(req, timeout=10).read()


def until(test, limit=30.0):
    end = time.time() + limit
    while time.time() < end:
        try:
            if test():
                return True
        except Exception:
            pass
        time.sleep(0.1)
    return False


FIELD = "[data-terminal-box] textarea"


def unseen(page):
    """The field's computed look: nothing of it shows."""
    s = page.evaluate(f"""() => {{ const t = document.querySelector('{FIELD}'); if (!t) return null;
        const c = getComputedStyle(t); return {{opacity: c.opacity, caret: c.caretColor, outline: c.outlineStyle}}; }}""")
    return s is not None and s["opacity"] == "0" and s["caret"] in ("rgba(0, 0, 0, 0)", "transparent"), s


def run(p, base, name, **context):
    page = p.chromium.launch().new_page(**context)
    page.goto(base)
    if not until(lambda: page.query_selector(f"{FIELD}") is not None and page.query_selector("[data-terminal-box] canvas") is not None):
        check(f"{name}: the terminal opens", False)
        return
    canvas = page.locator("[data-terminal-box] canvas").first.bounding_box()
    ok, look = unseen(page)
    check(f"{name}: the terminal's field shows nothing", ok, look)
    page.mouse.click(canvas["x"] + 40, canvas["y"] + 40)
    page.keyboard.type("echo field-check")
    time.sleep(1)
    at = page.evaluate(f"() => {{ const r = document.querySelector('{FIELD}').getBoundingClientRect(); return [r.left, r.top]; }}")
    check(f"{name}: the field sits on the cursor, past the line's first cells",
          at[0] > canvas["x"] + 40 and canvas["y"] <= at[1] <= canvas["y"] + canvas["height"], (at, canvas))
    page.mouse.click(canvas["x"] + 60, canvas["y"] + 60, button="right")
    time.sleep(0.3)
    page.mouse.click(canvas["x"] + 80, canvas["y"] + 80)
    time.sleep(0.3)
    ok, look = unseen(page)
    check(f"{name}: after a right-click the field still shows nothing", ok, look)
    if SHOTS:
        SHOTS.mkdir(parents=True, exist_ok=True)
        page.screenshot(path=SHOTS / f"terminal-{name}.png")


def main():
    t = Throwaway(sys.argv[1], prefix="libri-terminal-field-")
    try:
        post(t.base, "api/layout", {"spaces": [{"id": "w1", "name": "Shell", "focus": "l1", "full": False,
                                                 "root": {"t": "leaf", "id": "l1", "pane": {"kind": "terminal"}}}], "on": "w1"})
        post(t.base, "api/phone-layout", {"spaces": [{"id": "p1", "name": "Shell", "items": [{"kind": "terminal"}]}], "on": "p1"})
        with sync_playwright() as p:
            run(p, t.base, "desktop", viewport={"width": 1280, "height": 800})
            run(p, t.base, "phone", viewport={"width": 390, "height": 844}, is_mobile=True, has_touch=True)
    finally:
        t.close()
    if failures:
        print(f"{len(failures)} failed: {failures}")
        sys.exit(1)
    print("all passed")


main()
