#!/usr/bin/env python3
"""A call's MCP Apps card shows by itself only when showing it is the point
(2026-11-13): a tool whose card is what it is for (`is.libri/opens`) shows
it under each call of the person's turn; a lookup the model makes for
itself shows its row alone, no card and no pill, and its row's details open
the card on asking; `show_card` naming a lookup shows its card, drawn from
a fresh call. Read back after a reload, the display-first call waits behind
its pill and the lookup still shows nothing.

Drives the real daemon with the stand-in Claude Code (stand_in_claude.py:
GREET, LOOK, SHOWCARD) and card_server.py as a connection, in Chromium at
1440px and 390px. Needs Playwright and its Chromium, so it stays out of
`nix flake check`.
Usage: check_card_opens.py LIBRI_DIR (holding bin/libri and site/) [SHOTS_DIR]
"""

import json
import sys
import tempfile
import time
from pathlib import Path

from playwright.sync_api import sync_playwright

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from throwaway_libri import Throwaway  # noqa: E402

LIBRI_DIR = Path(sys.argv[1])
SHOTS = Path(sys.argv[2]) if len(sys.argv) > 2 else None
failures = []


def check(what, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'} {what}{'' if ok else ' ' + str(detail)[:300]}", flush=True)
    if not ok:
        failures.append(what)


def until(test, limit=20.0):
    end = time.monotonic() + limit
    while time.monotonic() < end:
        if test():
            return True
        time.sleep(0.25)
    return False


def send(page, text):
    box = page.locator("textarea").first
    box.fill(text)
    box.press("Enter")


def cards(page):
    """The cards drawn, by their frames' titles."""
    return page.locator("iframe[title$=' card']").count()


def card_says(page, words):
    """Whether a card's own frame (inside the sandbox's) shows `words`."""
    for f in page.frames:
        try:
            if words in (f.locator("body").inner_text(timeout=200) or ""):
                return True
        except Exception:
            continue
    return False


def shot(page, name):
    if SHOTS:
        SHOTS.mkdir(parents=True, exist_ok=True)
        page.screenshot(path=str(SHOTS / f"{name}.png"))


def main():
    added = Path(tempfile.mkdtemp(prefix="card-opens-")) / "added.json"
    added.write_text(json.dumps({"mcpServers": {"card-test": {"type": "stdio", "command": sys.executable, "args": [str(HERE / "card_server.py")]}}}))
    t = Throwaway(LIBRI_DIR, prefix="libri-card-opens-", extra={"LIBRI_MCP": str(added)})
    try:
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            for width, height in ((1440, 900), (390, 844)):
                w = f"{width}"
                context = browser.new_context(viewport={"width": width, "height": height})
                page = context.new_page()
                page.goto(t.base)
                page.locator("textarea").first.wait_for(timeout=20000)
                before = cards(page)
                pills = page.get_by_text("Show the card-test card").count()
                # A display-first tool's card shows under the call.
                send(page, f"GREET Ada{w}")
                check(f"{w}: a display-first call shows its card", until(lambda: cards(page) > before and card_says(page, f"Hello, Ada{w}")))
                shown = cards(page)
                # A lookup's: its row alone, no card and no pill.
                send(page, f"LOOK Bo{w}")
                check(f"{w}: the lookup's turn ends", until(lambda: page.get_by_text("Looked.").count() > 0))
                time.sleep(1.5)
                check(f"{w}: a lookup shows no card", cards(page) == shown, cards(page))
                check(f"{w}: a lookup shows no pill", page.get_by_text("Show the card-test card").count() == pills)
                shot(page, f"lookup-{w}")
                # Opening the row brings what it opens into view, under the
                # head and above the dock; closing it keeps the row in place.
                row = page.get_by_text("look up greeting").last
                row.scroll_into_view_if_needed()
                page.evaluate("""() => { const s = document.querySelector('[data-chat-scroll]');
                  const r = [...document.querySelectorAll('summary')].pop().getBoundingClientRect();
                  s.scrollBy(0, r.bottom - (s.getBoundingClientRect().bottom - parseFloat(getComputedStyle(s).getPropertyValue('--dock') || 0)) + 4); }""")
                time.sleep(0.5)
                row.click()
                time.sleep(1.2)
                fits = page.evaluate("""() => { const s = document.querySelector('[data-chat-scroll]');
                  const d = [...document.querySelectorAll('details')].pop().getBoundingClientRect();
                  const v = s.getBoundingClientRect(); const css = getComputedStyle(s);
                  const top = v.top + (parseFloat(css.getPropertyValue('--talk-top')) || 0);
                  const bottom = v.bottom - (parseFloat(css.getPropertyValue('--dock')) || 0);
                  return d.height > bottom - top ? Math.abs(d.top - (top + 8)) < 6 : d.top >= top - 1 && d.bottom <= bottom + 1; }""")
                check(f"{w}: an opened row comes into view", fits)
                shot(page, f"opened-row-{w}")
                at = page.evaluate("() => [...document.querySelectorAll('summary')].pop().getBoundingClientRect().top")
                row.click()
                time.sleep(0.8)
                after = page.evaluate("() => [...document.querySelectorAll('summary')].pop().getBoundingClientRect().top")
                check(f"{w}: closing the row keeps it where it was", abs(after - at) < 3, (at, after))
                # Its row's details open it on asking.
                row.click()
                opener = page.get_by_role("button", name="Open its card")
                check(f"{w}: the lookup's row offers its card", until(lambda: opener.count() > 0))
                if opener.count():
                    opener.last.click()
                    check(f"{w}: the row opens the lookup's card", until(lambda: cards(page) > shown and card_says(page, f"Hello, Bo{w}")))
                    shown = cards(page)
                shot(page, f"lookup-opened-{w}")
                # The model asks to show the lookup's card: a fresh call draws it.
                send(page, f"SHOWCARD Cy{w}")
                # Its fresh call asks first, as a card's call does here.
                allow = page.get_by_role("button", name="Allow")
                if until(lambda: allow.count() > 0, 15):
                    allow.last.click()
                check(f"{w}: show_card shows the card it names, called afresh",
                      until(lambda: cards(page) > shown and card_says(page, f"Hello, Cy{w}") and card_says(page, "Structured result"), 30))
                shot(page, f"show-card-{w}")
                # Read back: the display-first call waits behind its pill; the
                # lookup's row still draws nothing.
                page.reload()
                page.locator("textarea").first.wait_for(timeout=20000)
                check(f"{w}: after a reload the display-first call waits behind its pill",
                      until(lambda: page.get_by_text("Show the card-test card").count() >= 1))
                time.sleep(1)
                pills = page.get_by_text("Show the card-test card").count()
                check(f"{w}: after a reload only display-first calls have a pill (one GREET a width so far)", pills == (1 if w == "1440" else 2), pills)
                shot(page, f"reloaded-{w}")
                errors = []
                page.on("pageerror", lambda e: errors.append(str(e)))
                check(f"{w}: no page errors", not errors, errors)
                context.close()
            browser.close()
    finally:
        t.close()
    print(f"{len(failures)} failed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
