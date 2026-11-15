#!/usr/bin/env python3
"""Sub-workers in the Work list: a sub is an ordinary card in its own state's
section, sorted like any other, so the section counts, the tab dot and "Mark
all read" reach it. Only a muted "Head ›" before its title says where it came
from, giving way first to the sub's own title, and its page's header reads
"Head › Sub". Runs the built site against check_worker_cards.py's stand-in
host in Chromium at 390 and 1440 in both themes, and keeps shots of the list,
of a sub's card, of a Waiting sub and of a sub's page.
Needs Playwright and a browser, so it stays out of `nix flake check`.
Usage: check_sub_workers.py SITE_DIR SHOTS_DIR
"""

import functools
import http.server
import sys
import threading
import types
from pathlib import Path

from playwright.sync_api import sync_playwright

# The card check's stand-in host and helpers, without running its sweep.
src = (Path(__file__).parent / "check_worker_cards.py").read_text()
cards = types.ModuleType("cards")
cards.__dict__["__file__"] = __file__
exec(src[: src.index("\ndef main():")], cards.__dict__)
thread, check, SITE, SHOTS = cards.thread, cards.check, cards.SITE, cards.SHOTS
base = cards.threads


def threads():
    return base() + [
        thread("s1", "Kite", "working", title="Supplier search", summary="Two found", head="w4"),
        thread("s4", "Snipe", "sleeping", title="Chart scan", report="Scanned.", head="w10"),
        thread("s2", "Finch", "waiting", title="Supplier list", report="Three suppliers, one question.", head="w9"),
        # The longest name Settings takes as a head, and a long title of its own.
        thread("s3", "Rook", "working", title=cards.LONG_TITLE, summary="Still going", head="w16"),
    ]


cards.threads = threads

NESTED = "() => document.querySelectorAll('li ul li button').length"
HEADS = """() => [...document.querySelectorAll('button[aria-expanded]')].map((b) => b.textContent.trim())"""
CRUMB = """(a) => { const p = a.querySelector('p'); const crumb = p.querySelector('.text-soft'); const own = p.firstElementChild; const box = (e) => e.getBoundingClientRect(); return { crumb: crumb ? crumb.textContent : null, seen: !!crumb && box(crumb).top === box(own).top, own: box(own).width, room: box(p).width, high: box(p).height }; }"""


def run(browser, site, label, theme, mobile):
    tag = f"{label}-{theme}"
    size = cards.WIDTHS[label]
    context, page, errors, host = cards.load(browser, site, size, theme, mobile)
    cards.to_work(page, not mobile)
    check(f"{tag}: nothing is nested", page.evaluate(NESTED) == 0)
    for name, head in (("Kite", "Tarn"), ("Snipe", "Plover"), ("Finch", "Heron"), ("Rook", "Wolfeschlegelste")):
        c = cards.card(page, name)
        check(f"{tag}: {name} is a card of its own", c.count() == 1)
        crumb = c.evaluate(CRUMB)
        check(f"{tag}: {name}'s card names {head} first", crumb["crumb"] == f"{head} ›", crumb)
        check(f"{tag}: {name}'s title row is one line", crumb["high"] < 22, crumb)
    kite = cards.card(page, "Kite").evaluate(CRUMB)
    check(f"{tag}: Kite's head shows before its title", kite["seen"], kite)
    rook = cards.card(page, "Rook").evaluate(CRUMB)
    check(f"{tag}: Rook's own title takes the whole row", not rook["seen"] and rook["own"] >= rook["room"] * 0.9, rook)
    check(f"{tag}: a head's own card names no head", cards.card(page, "Tarn").evaluate(CRUMB)["crumb"] is None)
    heads = page.evaluate(HEADS)
    check(f"{tag}: Waiting counts Finch", any(h.startswith("Waiting") and "4" in h for h in heads), heads)
    page.screenshot(path=SHOTS / f"{tag}-list.png", full_page=True)
    cards.card(page, "Kite").screenshot(path=SHOTS / f"{tag}-sub-card.png")
    cards.card(page, "Rook").screenshot(path=SHOTS / f"{tag}-sub-card-long.png")
    cards.card(page, "Finch").screenshot(path=SHOTS / f"{tag}-waiting-sub.png")
    cards.card(page, "Kite").locator("button").first.click()
    header = page.locator("[role=complementary] h2").first
    header.wait_for()
    check(f"{tag}: Kite's page reads Tarn › Kite", header.text_content().startswith("Tarn › Kite"), header.text_content())
    page.wait_for_timeout(800)
    page.screenshot(path=SHOTS / f"{tag}-sub-page.png")
    if mobile:
        page.get_by_role("button", name="Back to Tasks").first.click()
        page.wait_for_timeout(500)
    page.get_by_role("button", name="Mark all read").first.click()
    page.wait_for_timeout(300)
    check(f"{tag}: Mark all read reaches Finch", ("s2", "read") in host.calls, host.calls)
    check(f"{tag}: no page errors", not errors, errors)
    context.close()
    cards.said(tag)


def main():
    handler = functools.partial(cards.Pages, directory=str(SITE))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    SHOTS.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for label in ("1440", "390"):
            for theme in ("light", "dark"):
                run(browser, site, label, theme, label == "390")
        browser.close()
    print(f"{cards.total} checks, {len(cards.failures)} failed", flush=True)
    sys.exit(1 if cards.failures else 0)


main()
