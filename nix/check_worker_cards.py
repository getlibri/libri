#!/usr/bin/env python3
"""Worker cards: every card three rows in every section, state and width, no
row ever wraps, and nothing moves a card by a pixel. A card holds no buttons: a
right click or the keyboard's menu key brings up its actions, a finger's hold
the same, and a tap still opens the worker; a Working card offers nothing. A
mouse resting on a card gets its words whole from the browser's tooltip. Cards
hold at each end of the side columns' drag, and an open thread's card is
marked once the thread is out. No card says the state its section
and dot already say, and the worker's own page still does. Runs the built site
against a stand-in host in Chromium at 390, 1024 and 1440 in both themes. Given
a Libri binary it also seeds workers on a real scratch host, first with
nothing cheap to call on and then with a call that fails, and proves no card's
line is ever empty. Needs Playwright and a browser, so it stays out of
`nix flake check`.
Usage: check_worker_cards.py SITE_DIR SHOTS_DIR [DAEMON]
"""

import functools
import http.server
import itertools
import json
import re
import os
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path
from urllib.parse import urlparse

from playwright.sync_api import sync_playwright

SITE, SHOTS = Path(sys.argv[1]), Path(sys.argv[2])
DAEMON = sys.argv[3] if len(sys.argv) > 3 else None
WIDTHS = {"390": (390, 844), "1024": (1024, 768), "1440": (1440, 900)}
LONG_TITLE = "Rebuild the harbour tide tables for every port on the coast"
LONG_REPORT = "The charts are filed and the tide tables rebuilt for every port, with three corrections to the Portsmouth figures that came out of the second survey, which the harbour master has signed, and a note on the spring tides that needs your word before anything else goes out."
failures = []
total = 0


def check(what, ok, detail=""):
    global total
    total += 1
    if not ok:
        failures.append(what)
        print(f"FAIL {what} {detail}", flush=True)


def said(what):
    print(f"--   {what} ({total} checks, {len(failures)} failed)", flush=True)


class Pages(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_):
        pass


def thread(tid, name, state, ago=300, **more):
    gist = more.get("summary") or more.get("report") or f"Check the {name} work"
    return {"id": tid, "name": name, "brief": f"Check the {name} work", "gist": gist, "state": state, "at": int(time.time()) - ago, "read": state in ("sleeping", "archived"), "waiting_on": [], "archived": state == "archived"} | more


def threads():
    return [
        thread("w1", "Wren", "waiting", title="Harbour tide tables", report="One question for you before I go on.", need="answer", gist="Which port first, Portsmouth or Poole?"),
        thread("w2", "Vaughn", "waiting", title=LONG_TITLE, report=LONG_REPORT, need="approval", gist=LONG_REPORT),
        thread("w3", "Sable", "waiting", need="review"),
        thread("w4", "Tarn", "working", title="Mooring line quotes", summary="Comparing three quotes"),
        thread("w5", "Cross", "working", title=LONG_TITLE, summary=LONG_REPORT),
        thread("w6", "Moss", "working", title="Port notice", report="An earlier report, until the first summary."),
        thread("w7", "Reed", "working", title="Nothing said yet"),
        thread("w8", "Ibis", "working", title="On hold", waiting_on=["Wren", "Finch", "Sable", "Tarn", "Cross"]),
        thread("w15", "Wigeon", "working", title="On a short hold", waiting_on=["Wren"]),
        thread("w9", "Heron", "sleeping", title="Radio survey", report="Finished."),
        thread("w10", "Plover", "sleeping", title=LONG_TITLE, report=LONG_REPORT),
        thread("w11", "Teal", "sleeping"),
        thread("w12", "Gull", "archived", title="Old radio survey", report="Finished long ago."),
        thread("w13", "Ouzel", "archived", title=LONG_TITLE, report=LONG_REPORT),
        thread("w14", "Dunlin", "archived"),
        # The longest name Settings takes.
        thread("w16", "Wolfeschlegelste", "sleeping", title="Longest name", report="Finished."),
        thread("w17", "Eider", "napping", title="Chart corrections", report="Ready when you are.", nap=int(time.time()) + 3 * 3600),
        thread("w18", "Smew", "napping", title=LONG_TITLE, report=LONG_REPORT, nap=int(time.time()) + 4 * 86400),
        # At work, with a nap kept for when its run ends.
        thread("w19", "Scaup", "working", title="Tide gauge", summary="Reading the gauge", nap=int(time.time()) + 3600),
    ]


class Host:
    """The stand-in host: its threads, and what the page asked of it."""

    def __init__(self):
        self.threads = threads()
        self.calls = []
        self.events = ": open\n\n"
        self.agents = []

    def find(self, tid):
        return next(t for t in self.threads if t["id"] == tid)

    def api(self, route):
        path = urlparse(route.request.url).path
        path = path[path.index("/api/"):]
        if path == "/api/events":
            # Told once: a reconnection would count the calls again.
            told, self.events = self.events, ": open\n\n"
            return route.fulfill(status=200, headers={"content-type": "text/event-stream"}, body=told)
        if path.startswith("/api/work/") and route.request.method == "POST":
            _, _, _, tid, what = path.split("/")
            self.calls.append((tid, what))
            t = self.find(tid)
            if what == "read" and t.get("need") in ("approval", "answer"):
                t |= {"read": True}
            elif what in ("read", "clear"):
                t |= {"read": True, "need": None, "state": "sleeping" if t["state"] == "waiting" else t["state"]}
            elif what == "archive":
                t |= {"state": "archived"}
            elif what == "restore":
                t |= {"state": "sleeping"}
            elif what == "nap":
                until = route.request.post_data_json["until"]
                self.calls[-1] = (tid, what, until)
                t |= {"nap": until} | ({} if t["state"] == "working" else {"state": "napping"})
            elif what == "wake":
                t.pop("nap", None)
                if t["state"] != "working":
                    t |= {"state": "waiting", "read": False}
            return route.fulfill(json={})
        replies = {
            "/api/me": {"name": "Libri", "personas": [], "voice": None, "models": False, "forgets": []},
            "/api/work": {"threads": self.threads},
            "/api/history": [{"i": 10, "kind": "talk", "text": "The cards are on the Work page.", "date": "2026-11-01T12:40:00Z", "reaction": None}],
            "/api/usage": [],
            "/api/host": {"id": "stand-in", "name": "Box", "version": "0", "uptime": 3600},
            "/api/pending": [],
            "/api/asks": [],
            "/api/scratchpad": {"id": None, "text": "", "working": False, "words": "", "thinking": "", "stopping": False, "queued": [], "memory": True, "asks": []},
            "/api/task": {"agents": self.agents, "screen": None},
            "/api/memory": {"state": "working", "reason": None, "since": None, "waiting": 0, "newest_summary": None, "latest_message": None, "model": "haiku"},
        }
        if path in replies:
            return route.fulfill(json=replies[path])
        return route.fulfill(json={})


def load(browser, site, size, theme, mobile, side=None):
    context = browser.new_context(viewport={"width": size[0], "height": size[1]}, service_workers="block", is_mobile=mobile, has_touch=mobile)
    context.set_default_timeout(10000)
    if theme == "dark":
        context.add_init_script("localStorage.setItem('theme', 'dark')")
    for section in ("waiting", "working", "napping", "sleeping", "archived"):
        context.add_init_script(f"localStorage.setItem('libri-work-open-{section}', 'on')")
    if side:
        context.add_init_script(f"localStorage.setItem('libri-side-width', '{side}')")
    host = Host()
    context.route("**/api/**", host.api)
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(site + "/")
    return context, page, errors, host


def to_work(page, wide):
    if not wide:
        page.locator("nav[aria-label=Places] button:has-text('Tasks')").first.click()
    page.wait_for_selector("article >> text=Wren")
    for name in ("Waiting", "Working", "Sleeping", "Archived"):
        heads = page.locator(f"button:has-text('{name} ·')")
        if heads.count() and heads.first.get_attribute("aria-expanded") == "false":
            heads.first.click()
    page.wait_for_selector("article >> text=Dunlin")
    page.wait_for_timeout(300)


READ = """() => [...document.querySelectorAll('article[data-sheet]')].filter((a) => a.querySelector('button')).map((a) => {
  const b = a.getBoundingClientRect();
  const name = a.querySelector('span.font-semibold').textContent;
  const lines = [...a.querySelectorAll('p')];
  const go = a.querySelector('button');
  const row = (e) => { const c = getComputedStyle(e); return { high: e.getBoundingClientRect().height, line: parseFloat(c.lineHeight), wrap: c.whiteSpace, end: c.textOverflow }; };
  return { name, top: b.top, height: b.height, width: b.width, buttons: a.querySelectorAll('button').length,
    tip: go.getAttribute('title'), note: go.firstElementChild.children[2].textContent, head: row(go.firstElementChild), rows: lines.map(row), title: lines[0].textContent, said: lines[1].textContent,
    wraps: go.scrollWidth > go.clientWidth };
})"""


def card(page, name):
    return page.locator("article[data-sheet]").filter(has=page.locator(f"span.font-semibold:text-is('{name}')")).first


def heights(page, tag):
    cards = page.evaluate(READ)
    check(f"{tag}: every card shown", len(cards) == len(threads()), len(cards))
    tall = {c["height"] for c in cards}
    check(f"{tag}: one height", len(tall) == 1, sorted(tall))
    check(f"{tag}: nothing wider than its card", not any(c["wraps"] for c in cards))
    check(f"{tag}: one button on every card, the card's own", all(c["buttons"] == 1 for c in cards), [c["name"] for c in cards if c["buttons"] != 1])
    rows = [r for c in cards for r in c["rows"]]
    check(f"{tag}: title and summary are one row each", all(abs(r["high"] - r["line"]) < 0.6 and r["wrap"] == "nowrap" and r["end"] == "ellipsis" for r in rows), rows[:2])
    check(f"{tag}: the head is one row", all(abs(c["head"]["high"] - c["head"]["line"]) < 3 for c in cards), cards[0]["head"])
    return cards


def press_and_hold(page, target, ms=520, lift=True):
    """A real finger, held still, through the browser's touch input."""
    box = target.bounding_box()
    x, y = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
    cdp = page.context.new_cdp_session(page)
    cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [{"x": x, "y": y}]})
    page.wait_for_timeout(ms)
    if lift:
        cdp.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
    return cdp


WAITING_MENU = ["Mark read", "Nap", "Bring to chat", "Archive"]


def naps(page, tag, host):
    """Nap turns the menu to T3's presets and a time of one's own; a nap
    lands in Napping with its wake time, and Wake now brings it back."""
    napping_section(page, tag)
    c = card(page, "Plover")
    c.scroll_into_view_if_needed()
    c.click(button="right")
    page.wait_for_timeout(250)
    page.get_by_role("menuitem", name="Nap").click()
    page.wait_for_timeout(250)
    words = page.get_by_role("menu").get_by_role("menuitem").all_text_contents()
    check(f"{tag}: Nap offers the presets, soonest first, then a time", words[:2] == ["1 hour", "3 hours"] and words[-1] == "Pick a time" and any(w.startswith("Tomorrow") for w in words), words)
    page.screenshot(path=SHOTS / f"{tag}-nap-presets.png")
    before = time.time()
    page.get_by_role("menuitem", name="1 hour").click()
    page.wait_for_timeout(400)
    sent = [c for c in host.calls if c[:2] == ("w10", "nap")]
    check(f"{tag}: a preset puts the worker down for its time", len(sent) == 1 and abs(sent[0][2] - before - 3600) < 30, sent)
    check(f"{tag}: the menu shuts after a nap", page.get_by_role("menu").count() == 0)
    check(f"{tag}: the napped card moves to Napping", "wakes" in card(page, "Plover").inner_text(), card(page, "Plover").inner_text())
    c = card(page, "Teal")
    c.scroll_into_view_if_needed()
    c.click(button="right")
    page.wait_for_timeout(250)
    page.get_by_role("menuitem", name="Nap").click()
    page.get_by_role("menuitem", name="Pick a time").click()
    page.wait_for_timeout(250)
    field = page.get_by_label("Wake time")
    check(f"{tag}: Pick a time asks for one", field.count() == 1)
    wake = time.localtime(time.time() + 2 * 86400)
    field.fill(time.strftime("%Y-%m-%dT10:30", wake))
    page.screenshot(path=SHOTS / f"{tag}-nap-time.png")
    page.get_by_role("menu").get_by_role("button", name="Nap").click()
    page.wait_for_timeout(400)
    sent = [c for c in host.calls if c[:2] == ("w11", "nap")]
    want = int(time.mktime(time.strptime(time.strftime("%Y-%m-%d 10:30", wake), "%Y-%m-%d %H:%M")))
    check(f"{tag}: a time of one's own is the one sent", len(sent) == 1 and sent[0][2] == want, (sent, want))
    c = card(page, "Eider")
    c.scroll_into_view_if_needed()
    c.click(button="right")
    page.wait_for_timeout(250)
    page.get_by_role("menuitem", name="Wake now").click()
    page.wait_for_timeout(400)
    check(f"{tag}: Wake now tells the host", ("w17", "wake") in host.calls, host.calls)
    check(f"{tag}: the woken card says no wake time", "wakes" not in card(page, "Eider").inner_text(), card(page, "Eider").inner_text())


def napping_section(page, tag):
    """The Napping section open: a ringed dot and the wake time on each card,
    and a Working card with a nap to come says when it begins."""
    check(f"{tag}: a napping card says when it wakes", "wakes" in card(page, "Eider").inner_text(), card(page, "Eider").inner_text())
    heads = page.get_by_text("Napping", exact=False).all_text_contents()
    check(f"{tag}: the napping cards have a section of their own", any(h.strip().startswith("Napping") for h in heads), heads)
    dot = card(page, "Eider").locator("[role=img]")
    check(f"{tag}: a napping card's dot is a ring", dot.get_attribute("aria-label") == "Napping" and "border" in dot.locator("span").first.get_attribute("class"), dot.get_attribute("aria-label"))
    check(f"{tag}: a Working card with a nap to come says so", "naps till" in card(page, "Scaup").inner_text(), card(page, "Scaup").inner_text())
    # Centred, so the stacked section heads don't cover the napping cards.
    card(page, "Eider").evaluate("e => e.scrollIntoView({block: 'center'})")
    page.wait_for_timeout(300)
    page.screenshot(path=SHOTS / f"{tag}-napping-section.png")


def menu_words(page, name):
    return page.get_by_role("menu", name=name).get_by_role("menuitem").all_text_contents()


def desktop(browser, site, label, size, theme, tag):
    context, page, errors, host = load(browser, site, size, theme, False)
    to_work(page, size[0] >= 1024)
    before = heights(page, tag)
    height = before[0]["height"]
    page.screenshot(path=SHOTS / f"{tag}-rest.png", full_page=True)

    # Hover: the words whole in a tooltip, and no card moves.
    tips = {c["name"]: c for c in before}
    vaughn = tips["Vaughn"]
    check(f"{tag}: the tooltip holds the name, title and summary whole", vaughn["tip"] == f"Vaughn · For approval\n{LONG_TITLE}\n{LONG_REPORT}", vaughn["tip"])
    ibis = "Ibis · Waiting on Wren, Finch, Sable, Tarn, Cross"
    check(f"{tag}: the tooltip opens with who it waits on, whole", tips["Ibis"]["tip"].split("\n")[0] == ibis, tips["Ibis"]["tip"])
    check(f"{tag}: every card has its tooltip", all(c["tip"] == "\n".join([" · ".join(filter(None, [c["name"], c["note"]])), c["title"], c["said"]]) for c in before))
    c = card(page, "Vaughn")
    c.scroll_into_view_if_needed()
    at = [(x["name"], x["top"]) for x in page.evaluate(READ)]
    c.hover()
    page.wait_for_timeout(200)
    check(f"{tag}: nothing moves on hover", [(x["name"], x["top"]) for x in page.evaluate(READ)] == at)
    check(f"{tag}: no card grows on hover", all(x["height"] == height for x in page.evaluate(READ)))
    check(f"{tag}: hover shows no buttons", c.locator("button").count() == 1)

    # A right click: the menu, and the worker stays shut.
    for name, words in (("Vaughn", WAITING_MENU), ("Plover", ["Nap", "Bring to chat", "Archive"]), ("Smew", ["Wake now", "Bring to chat", "Archive"]), ("Scaup", ["Cancel nap"]), ("Ouzel", ["Restore"])):
        c = card(page, name)
        c.scroll_into_view_if_needed()
        c.click(button="right")
        page.wait_for_timeout(250)
        check(f"{tag}: {name}'s right click offers", menu_words(page, name) == words, menu_words(page, name))
        check(f"{tag}: {name}'s right click opens no worker", page.locator(f"textarea[placeholder*='{name}']").count() == 0)
        if name == "Vaughn":
            page.screenshot(path=SHOTS / f"{tag}-right-click.png")
        page.keyboard.press("Escape")
        page.wait_for_timeout(200)
        check(f"{tag}: Esc shuts {name}'s menu", page.get_by_role("menu").count() == 0)
    c = card(page, "Tarn")
    c.scroll_into_view_if_needed()
    c.click(button="right")
    page.wait_for_timeout(250)
    check(f"{tag}: a Working card offers no menu", page.get_by_role("menu").count() == 0)
    page.keyboard.press("Escape")

    # The keyboard: the menu key on a card's focus goes into its menu and back.
    c = card(page, "Wren")
    c.locator("button").first.focus()
    page.keyboard.press("ContextMenu")
    page.wait_for_timeout(250)
    check(f"{tag}: the menu key opens the card's menu", menu_words(page, "Wren") == WAITING_MENU, menu_words(page, "Wren"))
    focused = page.evaluate("document.activeElement.textContent")
    check(f"{tag}: focus lands on Mark read", focused == "Mark read", focused)
    ring = page.evaluate("""() => { const c = getComputedStyle(document.activeElement); return { outline: c.outlineStyle, fill: c.backgroundColor }; }""")
    check(f"{tag}: the focused row takes the hover fill, not a ring", ring["outline"] == "none" and ring["fill"] != "rgba(0, 0, 0, 0)", ring)
    page.screenshot(path=SHOTS / f"{tag}-keyboard.png")
    for _ in range(3):
        page.keyboard.press("Tab")
    check(f"{tag}: Tab reaches Archive", page.evaluate("document.activeElement.textContent") == "Archive")
    page.keyboard.press("Escape")
    page.wait_for_timeout(200)
    back = page.evaluate("document.activeElement.closest('article')?.textContent ?? ''")
    check(f"{tag}: Esc gives focus back to the card", "Wren" in back, back)
    page.keyboard.press("ContextMenu")
    page.wait_for_timeout(250)
    page.keyboard.press("Enter")
    page.wait_for_timeout(200)
    check(f"{tag}: Enter on Mark read tells the host", ("w1", "clear") in host.calls, host.calls)

    # Each action from the menu, and a plain press on a card opens it.
    page.wait_for_timeout(300)
    card(page, "Vaughn").click(button="right")
    page.get_by_role("menuitem", name="Archive").click()
    page.wait_for_timeout(300)
    check(f"{tag}: Archive tells the host", ("w2", "archive") in host.calls, host.calls)
    card(page, "Ouzel").click(button="right")
    page.get_by_role("menuitem", name="Restore").click()
    page.wait_for_timeout(300)
    check(f"{tag}: Restore tells the host", ("w13", "restore") in host.calls, host.calls)
    naps(page, tag, host)
    card(page, "Heron").locator("button").first.click()
    page.wait_for_selector("text=· sleeping")
    check(f"{tag}: a press opens the worker", True)
    check(f"{tag}: no page errors", not errors, errors)
    context.close()


def side_widths(browser, site, theme):
    """Cards hold at each end of the columns' drag, and at the default."""
    for side in (240, 288, 480):
        tag = f"side-{side}-{theme}"
        context, page, errors, host = load(browser, site, (1920, 1080), theme, False, side=side)
        to_work(page, True)
        cards = heights(page, tag)
        check(f"{tag}: the column takes the width", all(c["width"] == side - 33 for c in cards), sorted({c["width"] for c in cards}))
        far = card(page, "Wolfeschlegelste").locator("button div").first
        time_box = far.locator(":scope > span").last.bounding_box()
        box = card(page, "Wolfeschlegelste").bounding_box()
        check(f"{tag}: the longest name leaves the time inside the card", time_box["x"] + time_box["width"] <= box["x"] + box["width"], (time_box, box))
        page.mouse.move(0, 0)
        page.screenshot(path=SHOTS / f"{tag}-rest.png")
        card(page, "Vaughn").click(button="right")
        page.wait_for_timeout(250)
        words = menu_words(page, "Vaughn")
        check(f"{tag}: the right click menu comes up", words == WAITING_MENU, words)
        page.screenshot(path=SHOTS / f"{tag}-right-click.png")
        check(f"{tag}: no page errors", not errors, errors)
        context.close()


# An open thread's card joins the thread only while the thread slides out
# (check_header_pill follows the slide); once it is out the card stands back in
# the list, marked: the thread's paper and a bar at its left, its corners round,
# no shadow (a design choice of 2026-11-05: a card joined at rest read as cut
# off).
JOIN = """(name) => {
  const a = [...document.querySelectorAll('article[data-sheet]')].find((x) => x.querySelector('span.font-semibold')?.textContent === name);
  const pane = document.querySelector('[data-thread]');
  const column = document.querySelector('aside[aria-label="Work"] .border-l.bg-paper');
  const c = getComputedStyle(a), p = getComputedStyle(pane), w = getComputedStyle(column);
  const b = a.getBoundingClientRect(), k = column.getBoundingClientRect();
  return { left: b.left, top: b.top, bottom: b.bottom, rule: k.left, fill: c.backgroundColor, paper: p.backgroundColor,
    edge: c.borderTopColor, under: c.borderBottomColor, right: c.borderRightColor, side: c.borderLeftColor, outline: w.borderLeftColor,
    shadow: c.boxShadow, radius: [c.borderTopLeftRadius, c.borderBottomLeftRadius] };
}"""


def shown_shadows(shadow):
    """A computed box-shadow's layers that draw, colour left off: Tailwind
    stacks its rings and shadows as transparent layers too."""
    layers = re.findall(r"(rgba?\([^)]*\)) ([^,]+)", shadow)
    return [rest.strip() for colour, rest in layers if not colour.startswith("rgba(0, 0, 0, 0)")]


def joined(browser, site, theme):
    for label, size in (("2560", (2560, 1440)), ("1440", (1440, 900))):
        tag = f"join-{label}-{theme}"
        context, page, errors, host = load(browser, site, size, theme, False)
        to_work(page, True)
        for where, name in (("top", "Wren"), ("middle", "Moss"), ("scrolled", "Dunlin")):
            c = card(page, name)
            c.scroll_into_view_if_needed()
            c.locator("button").first.click()
            page.wait_for_selector("[data-thread]")
            page.wait_for_timeout(700)
            # The opened thread can narrow the column and push a low card
            # out of view; the close shot needs it in.
            c.scroll_into_view_if_needed()
            j = page.evaluate(JOIN, name)
            check(f"{tag} {where}: the card stands back in the list", j["left"] > j["rule"], j)
            check(f"{tag} {where}: the card takes the thread's paper", j["fill"] == j["paper"], j)
            check(f"{tag} {where}: the card's outline is the column's", j["edge"] == j["under"] == j["right"] == j["side"] == j["outline"], j)
            check(f"{tag} {where}: its corners are round", "0px" not in j["radius"], j)
            check(f"{tag} {where}: a bar at its left, and no shadow", shown_shadows(j["shadow"]) == ["2px 0px 0px 0px inset"], j)
            page.mouse.move(0, 0)
            page.screenshot(path=SHOTS / f"{tag}-{where}.png")
            x, y = j["rule"], j["top"]
            page.screenshot(path=SHOTS / f"{tag}-{where}-close.png", clip={"x": x - 60, "y": y - 30, "width": 160, "height": j["bottom"] - y + 60})
        # Another card, then the thread shut: the old card is a plain card again.
        card(page, "Heron").locator("button").first.click()
        page.wait_for_timeout(700)
        j = page.evaluate(JOIN, "Dunlin")
        check(f"{tag}: the card left behind is plain again", j["left"] > j["rule"] and j["fill"] != j["paper"] and "inset" not in j["shadow"], j)
        card(page, "Heron").locator("button").first.click()
        page.wait_for_timeout(700)
        j = page.evaluate(JOIN.replace("const pane = document.querySelector('[data-thread]');", "const pane = document.body;"), "Heron")
        check(f"{tag}: a folded thread leaves its card plain", j["left"] > j["rule"] and "inset" not in j["shadow"], j)
        page.screenshot(path=SHOTS / f"{tag}-shut.png")
        check(f"{tag}: no page errors", not errors, errors)
        context.close()


# The workers' segment of the pill carries the Work tab's mark: red while
# any worker waits on the person, green while any works, pulsing while one
# runs (not only waiting on another worker), nothing otherwise.
MARKS = {
    "waiting": (lambda: threads(), "bg-accent", False),
    "running": (lambda: [t for t in threads() if t["state"] != "waiting"], "bg-active", True),
    "held": (lambda: [t for t in threads() if t["state"] != "waiting" and (t["state"] != "working" or t["waiting_on"])], "bg-active", False),
    "idle": (lambda: [t for t in threads() if t["state"] in ("sleeping", "archived")], None, False),
}


def pill_mark(browser, site, theme):
    for label, size in (("2560", (2560, 1440)), ("1440", (1440, 900))):
        tag = f"pill-{label}-{theme}"
        context, page, errors, host = load(browser, site, size, theme, False)
        page.locator('[aria-label="Fold the workers column"]').first.click()
        for state, (make, colour, pulse) in MARKS.items():
            host.threads = make()
            page.reload()
            button = page.locator('[aria-label="Unfold the workers column"]')
            button.wait_for()
            page.wait_for_timeout(500)
            mark = button.locator("xpath=..").locator("span.rounded-full")
            if colour is None:
                check(f"{tag} {state}: no mark", mark.count() == 0, mark.count())
            else:
                cls = mark.get_attribute("class") or "" if mark.count() == 1 else ""
                check(f"{tag} {state}: {colour} mark", colour in cls.split(), cls)
                check(f"{tag} {state}: pulses {pulse}", ("animate-pulse" in cls.split()) == pulse, cls)
            page.mouse.move(0, 0)
            page.screenshot(path=SHOTS / f"{tag}-{state}.png", clip={"x": size[0] - 260, "y": 0, "width": 260, "height": 70})
        # Open, the column is its own mark: the pill's segment and its dot go.
        button.click()
        page.wait_for_timeout(700)
        check(f"{tag}: open column, no pill segment", page.locator('[aria-label="Unfold the workers column"]').count() == 0)
        page.screenshot(path=SHOTS / f"{tag}-open.png")
        check(f"{tag}: no page errors", not errors, errors)
        context.close()


def stepped(tid, count):
    """A subagent's start and its calls, as the event stream carries them."""
    told = [{"type": "SUBAGENT_STARTED", "subagentRunId": tid, "name": tid, "description": "At work"}]
    told += [{"type": "TOOL_CALL_START", "subagentRunId": tid, "toolCallId": f"{tid}-{i}", "toolCallName": "Bash"} for i in range(count)]
    return "".join(f"data: {json.dumps(e)}\n\n" for e in told)


# What stands between a card's name and its time, by state; nothing says the
# state itself, which the section and the dot already do.
NOTES = {
    "Wren": ("For an answer", "Waiting"), "Sable": ("For review", "Waiting"), "Tarn": ("3 steps", "Working"), "Moss": ("1 step", "Working"), "Reed": ("", "Working"),
    "Wigeon": ("Waiting on Wren · 2 steps", "Working"), "Heron": ("", "Sleeping"), "Teal": ("", "Sleeping"), "Gull": ("", "Archived"), "Dunlin": ("", "Archived"),
}


def notes(browser, site, label, size, theme, tag, mobile):
    """No state word on a card, and what stays there: dot, time, steps, who it waits on."""
    context, page, errors, host = load(browser, site, size, theme, mobile)
    host.agents = [{"name": tid, "task": "At work"} for tid in ("w4", "w6", "w15")]
    host.events = stepped("w4", 3) + stepped("w6", 1) + stepped("w15", 2)
    page.reload()
    to_work(page, size[0] >= 1024)
    height = heights(page, tag)[0]["height"]
    page.wait_for_selector("article >> text=3 steps")
    for name, (note, state) in NOTES.items():
        row = card(page, name).locator("button div").first
        spans = row.locator(":scope > span")
        check(f"{tag}: {name} reads {note!r} beside its name", spans.nth(2).text_content() == note, spans.nth(2).text_content())
        check(f"{tag}: {name}'s dot is labelled {state}", row.get_by_role("img", name=state, exact=True).count() == 1)
        check(f"{tag}: {name} keeps the one height", card(page, name).bounding_box()["height"] == height)
    ibis = card(page, "Ibis").locator("button div").first.locator(":scope > span").nth(2).text_content()
    check(f"{tag}: Ibis names who it waits on", ibis == "Waiting on Wren, Finch, Sable, Tarn, Cross", ibis)
    right = {round(b["x"] + b["width"], 3) for b in (card(page, n).locator("button div").first.locator(":scope > span").last.bounding_box() for n in NOTES)}
    check(f"{tag}: the time keeps its place on every card", len(right) == 1, right)
    tip = card(page, "Wigeon").locator("button").first.get_attribute("title").split("\n")[0]
    check(f"{tag}: the tooltip's head counts the steps too", tip == "Wigeon · Waiting on Wren · 2 steps", tip)
    page.screenshot(path=SHOTS / f"{tag}-notes.png", full_page=True)

    # With no section around it, the worker's page still says the word.
    card(page, "Heron").locator("button").first.click()
    page.wait_for_selector("text=· sleeping")
    # After the sheet's opening, which carries the card over the column.
    page.wait_for_timeout(800)
    page.screenshot(path=SHOTS / f"{tag}-page-head.png")
    check(f"{tag}: the worker's page says its state", page.locator("text=· sleeping").count() >= 1)
    check(f"{tag}: no page errors", not errors, errors)
    context.close()


def crossing(browser, site, label, size, theme, tag, mobile):
    """A worker crossing from Working to Sleeping keeps its height to the pixel."""
    context, page, errors, host = load(browser, site, size, theme, mobile)
    host.threads = [thread("a", "Reed", "working", title="Nothing said yet"), thread("b", "Tarn", "working", title="Mooring line quotes", summary="Comparing three quotes")]
    to_cross = host.threads[0]
    page.reload()
    if size[0] < 1024:
        page.locator("nav[aria-label=Places] button:has-text('Tasks')").first.click()
    page.wait_for_selector("article >> text=Reed")
    page.wait_for_timeout(200)
    frames = []
    page.evaluate("""() => { window.frames_ = []; const look = () => { for (const a of document.querySelectorAll('article[data-sheet]')) window.frames_.push(a.getBoundingClientRect().height); requestAnimationFrame(look); }; look(); }""")
    was = card(page, "Reed").bounding_box()
    page.screenshot(path=SHOTS / f"{tag}-crossing-1-working.png")
    to_cross |= {"state": "sleeping", "read": True, "report": "All done, and the report is in.", "gist": "All done, and the report is in."}
    host.threads[1] |= {"summary": LONG_REPORT, "gist": LONG_REPORT}
    page.wait_for_function("document.body.textContent.includes('Sleeping')", timeout=20000)
    for name in ("Sleeping",):
        heads = page.locator(f"button:has-text('{name} ·')")
        if heads.count() and heads.first.get_attribute("aria-expanded") == "false":
            heads.first.click()
    page.wait_for_selector("article >> text=All done", timeout=20000)
    page.wait_for_timeout(200)
    frames = set(page.evaluate("window.frames_"))
    check(f"{tag}: one height on every frame of the crossing", len(frames) == 1 and was["height"] in frames, sorted(frames))
    check(f"{tag}: the crossed card is that height", card(page, "Reed").bounding_box()["height"] == was["height"])
    page.screenshot(path=SHOTS / f"{tag}-crossing-2-sleeping.png")
    check(f"{tag}: no page errors", not errors, errors)
    context.close()


def phone(browser, site, theme, tag):
    context, page, errors, host = load(browser, site, WIDTHS["390"], theme, True)
    to_work(page, False)
    heights(page, tag)
    height = card(page, "Wren").bounding_box()["height"]
    check(f"{tag}: no hover on this screen", not page.evaluate("matchMedia('(hover: hover)').matches"))

    # A tap opens the worker and shows no actions.
    c = card(page, "Wren")
    c.scroll_into_view_if_needed()
    c.tap()
    page.wait_for_timeout(600)
    check(f"{tag}: a tap opens the worker", page.locator("textarea[placeholder*='Wren']").count() == 1, page.locator("textarea").count())
    page.get_by_role("button", name="Back to Tasks").first.click() if page.get_by_role("button", name="Back to Tasks").count() else page.go_back()
    page.wait_for_timeout(600)

    # A hold opens the menu; lifting the finger neither opens the worker nor picks.
    c = card(page, "Vaughn")
    c.scroll_into_view_if_needed()
    press_and_hold(page, c)
    page.wait_for_timeout(300)
    menu = page.get_by_role("menu", name="Vaughn")
    check(f"{tag}: a hold opens the card's menu", menu.count() == 1)
    words = menu.get_by_role("menuitem").all_text_contents()
    check(f"{tag}: the menu offers the four", words == WAITING_MENU, words)
    check(f"{tag}: the lift did not open the worker", page.locator("textarea[placeholder*='Vaughn']").count() == 0)
    check(f"{tag}: the lift pressed no item", not [c for c in host.calls if c[0] == "w2"], host.calls)
    page.screenshot(path=SHOTS / f"{tag}-hold-waiting.png")
    page.get_by_role("menuitem", name="Mark read").tap()
    page.wait_for_timeout(300)
    check(f"{tag}: Mark read from the menu tells the host", ("w2", "clear") in host.calls, host.calls)
    check(f"{tag}: the menu shuts", menu.count() == 0)

    # A finger held a long time lifts cleanly too.
    page.wait_for_timeout(400)
    c = card(page, "Plover")
    c.scroll_into_view_if_needed()
    press_and_hold(page, c, ms=1300)
    page.wait_for_timeout(300)
    menu = page.get_by_role("menu", name="Plover")
    check(f"{tag}: a long hold leaves the menu up", menu.count() == 1)
    check(f"{tag}: and no worker open", page.locator("textarea[placeholder*='Plover']").count() == 0)
    page.mouse.click(5, 5)
    page.wait_for_timeout(200)
    check(f"{tag}: a tap outside shuts it", menu.count() == 0)
    page.wait_for_timeout(200)
    c = card(page, "Ouzel")
    c.scroll_into_view_if_needed()
    press_and_hold(page, c)
    page.wait_for_timeout(300)
    menu = page.get_by_role("menu", name="Ouzel")
    check(f"{tag}: an archived card offers Restore", menu.get_by_role("menuitem").all_text_contents() == ["Restore"])
    page.screenshot(path=SHOTS / f"{tag}-hold-archived.png")
    menu.get_by_role("menuitem", name="Restore").tap()
    page.wait_for_timeout(300)
    check(f"{tag}: Restore from the menu tells the host", ("w13", "restore") in host.calls, host.calls)

    # The Napping section, and Nap's presets and Pick a time at a phone's width.
    page.wait_for_timeout(300)
    napping_section(page, tag)
    c = card(page, "Teal")
    c.scroll_into_view_if_needed()
    press_and_hold(page, c)
    page.wait_for_timeout(300)
    page.get_by_role("menuitem", name="Nap").tap()
    page.wait_for_timeout(250)
    words = page.get_by_role("menu").get_by_role("menuitem").all_text_contents()
    check(f"{tag}: Nap offers the presets, then a time", words[:2] == ["1 hour", "3 hours"] and words[-1] == "Pick a time", words)
    page.screenshot(path=SHOTS / f"{tag}-nap-presets.png")
    page.get_by_role("menuitem", name="Pick a time").tap()
    page.wait_for_timeout(250)
    field = page.get_by_label("Wake time")
    check(f"{tag}: Pick a time asks for one", field.count() == 1)
    field.fill(time.strftime("%Y-%m-%dT10:30", time.localtime(time.time() + 2 * 86400)))
    page.screenshot(path=SHOTS / f"{tag}-nap-time.png")
    page.get_by_role("menu").get_by_role("button", name="Nap").tap()
    page.wait_for_timeout(400)
    check(f"{tag}: a phone's nap reaches the host", any(c[:2] == ("w11", "nap") for c in host.calls), host.calls)

    # A Working card offers no menu.
    page.wait_for_timeout(300)
    c = card(page, "Tarn")
    c.scroll_into_view_if_needed()
    press_and_hold(page, c)
    page.wait_for_timeout(300)
    check(f"{tag}: a Working card opens no menu", page.get_by_role("menu").count() == 0)

    # The message's hold shares this code and must still come up.
    context.close()
    context, page, errors, host = load(browser, site, WIDTHS["390"], theme, True)
    line = page.get_by_text("The cards are on the Work page.")
    line.wait_for()
    press_and_hold(page, line)
    page.wait_for_timeout(300)
    menu = page.get_by_role("menu")
    check(f"{tag}: a hold on a message opens its menu", menu.count() == 1 and "Copy" in menu.get_by_role("menuitem").all_text_contents())
    page.screenshot(path=SHOTS / f"{tag}-hold-message.png")
    check(f"{tag}: no page errors", not errors, errors)
    context.close()


BASH = json.dumps({"command": "curl -H 'Authorization: Bearer hunter2' https://x.y", "description": "Fetch the chandler's prices"})
SPARE = json.dumps({"command": "ls /etc/passwd"})


def seed(task, name, brief, *entries, summary=None, running=False, archived=False, read=True):
    """A worker's folder as the host leaves it; `running` only once it is up."""
    d = task / name
    (d / "told").mkdir(parents=True)
    lines = [("note", "subagent"), ("user", brief), *entries]
    (d / "log.jsonl").write_text("".join(json.dumps({"kind": k, "text": t}) + "\n" for k, t in lines))
    (d / "choice.json").write_text(json.dumps({"pick": {"harness": "claude", "model": "haiku"}, "access": "auto"}))
    (d / "title").write_text("")
    if summary:
        (d / "summary").write_text(summary)
    if read:
        (d / "read").write_text(str(int((d / "log.jsonl").stat().st_mtime) + 1))
    if archived:
        (d / "archived").write_text("")


# Each worker, the state it must read and the words its card must say.
SEEDED = {
    "Alma": ("working", "Fresh brief for Alma"),
    "Basil": ("working", "Fetch the chandler's prices"),
    "Cleo": ("working", "Running a command"),
    "Dara": ("working", "Reading a file"),
    "Ezra": ("working", "Comparing three quotes"),
    "Fern": ("working", "All filed, and the charts are in"),
    "Gus": ("sleeping", "Last step: Fetch the chandler's prices"),
    "Hana": ("waiting", "Last step: Reading a file"),
    "Ivo": ("archived", "Brief for Ivo, stopped before a step"),
    "Juno": ("sleeping", "The report is in."),
    "Kit": ("sleeping", "Started"),
}


def seed_all(task):
    output = "x" * 200000
    seed(task, "Alma", "Fresh brief for Alma\nwith more under it", read=False)
    seed(task, "Basil", "Price the lines", ("tool", f"Bash {BASH}"), ("echo", output), read=False)
    seed(task, "Cleo", "Check the port", ("tool", f"Bash {SPARE}"), ("echo", output), read=False)
    # Its summary is due and the model fails: the step stays.
    seed(task, "Dara", "Read the notice", ("tool", 'Read {"file_path":"/etc/shadow"}'), ("echo", output), read=False)
    seed(task, "Ezra", "Compare the quotes", ("tool", "Read {}"), summary="Comparing three quotes", read=False)
    seed(task, "Fern", "File the charts", ("talk", "All filed, and the charts are in"), ("user", "One more thing"), read=False)
    seed(task, "Gus", "Price the lines", ("tool", f"Bash {BASH}"), ("echo", output))
    seed(task, "Hana", "Read the notice", ("tool", "Read {}"), read=False)
    seed(task, "Ivo", "Brief for Ivo, stopped before a step", archived=True)
    seed(task, "Juno", "Write it up", ("tool", "Read {}"), ("talk", "**The report** is in.\n\nMore below."))
    seed(task, "Kit", "```\nls\n```")


def scratch_host(libri, root, port, log):
    home = root / "home"
    for d in (home / ".config", home / ".local/share", home / ".cache", home / ".local/state", root / "hippo", root / "hippo-dir", root / "pages"):
        d.mkdir(parents=True, exist_ok=True)
    (root / "pages/index.html").write_text("<!doctype html>")
    env = {k: os.environ[k] for k in ("PATH", "LANG", "TZ") if k in os.environ} | {
        "HOME": str(home), "XDG_DATA_HOME": str(home / ".local/share"), "XDG_CONFIG_HOME": str(home / ".config"),
        "XDG_CACHE_HOME": str(home / ".cache"), "XDG_STATE_HOME": str(home / ".local/state"),
        "STATE_DIRECTORY": str(root / "state"), "LIBRI_WORKDIR": str(root / "hippo"), "HIPPO_DIR": str(root / "hippo-dir"),
        "LIBRI_NAME": "Cleo", "LIBRI_PERSON": "Ada", "LIBRI_SEAT_MODE": "1", "LIBRI_BACKEND": "claude",
        "LIBRI_CLAUDE": "/bin/false", "LIBRI_MODEL": "haiku", "LIBRI_HARNESSES": "claude",
        "LIBRI_SITE": f"127.0.0.1:{port}", "LIBRI_SITE_URL": f"http://127.0.0.1:{port}", "HIPPO_BACKEND": "none",
        "LIBRI_SITE_ROOT": str(root / "pages"), "LIBRI_OPEN": "1", "LIBRI_WORK_TICK": "2",
    }
    host = subprocess.Popen([libri], env=env, cwd=root, stderr=log, stdout=subprocess.DEVNULL, start_new_session=True)
    for _ in range(120):
        try:
            socket.create_connection(("127.0.0.1", port), 1).close()
            return host
        except OSError:
            time.sleep(0.5)
    raise RuntimeError("the scratch host never listened")


def gists(browser, site, libri):
    root = Path(tempfile.mkdtemp(prefix="worker-gist-"))
    task = root / "state/task"
    task.mkdir(parents=True)
    seed_all(task)
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    log = open(SHOTS / "gist-host.log", "w")
    host = scratch_host(libri, root, port, log)
    try:
        for name in ("Alma", "Basil", "Cleo", "Dara", "Ezra", "Fern"):
            (task / name / "state.json").write_text(json.dumps({"phase": {"state": "running"}}))

        def api(route):
            path = urlparse(route.request.url).path
            path = path[path.index("/api/"):]
            if path == "/api/events":
                return route.fulfill(status=200, headers={"content-type": "text/event-stream"}, body=": open\n\n")
            route.fulfill(response=route.fetch(url=f"http://127.0.0.1:{port}{path}"))

        for case in ("no-model", "failed-call"):
            expected = dict(SEEDED)
            if case == "failed-call":
                # hippo's list is out of reach here, so the first sweep had no
                # model at all. A pick it cannot run makes a real call fail: a
                # title's, as only a run started by the host is summarised.
                (root / "state/synopsis-pick").write_text('{"harness":"claude","model":"haiku"}')
                seed(task, "Lena", "Check the mooring lines")
                (task / "Lena/title").unlink()
                (task / "Lena/state.json").write_text(json.dumps({"phase": {"state": "queued"}}))
                for _ in range(60):
                    if json.loads((task / "Lena/state.json").read_text()).get("reported"):
                        break
                    time.sleep(0.5)
                check("a model call failed on the scratch host", "synopsis" in (SHOTS / "gist-host.log").read_text())
                expected["Lena"] = ("sleeping", "Check the mooring lines")
            for label, mobile in (("390", True), ("1440", False)):
                for theme in ("light", "dark"):
                    tag = f"gist-{case}-{label}-{theme}"
                    context = browser.new_context(viewport=dict(zip(("width", "height"), WIDTHS[label])), service_workers="block", is_mobile=mobile, has_touch=mobile)
                    context.set_default_timeout(20000)
                    if theme == "dark":
                        context.add_init_script("localStorage.setItem('theme', 'dark')")
                    for section in ("waiting", "working", "sleeping", "archived"):
                        context.add_init_script(f"localStorage.setItem('libri-work-open-{section}', 'on')")
                    context.route("**/api/**", api)
                    page = context.new_page()
                    page.goto(site + "/")
                    if not mobile:
                        page.wait_for_selector("article >> text=Alma")
                    else:
                        page.locator("nav[aria-label=Places] button:has-text('Tasks')").first.click()
                    page.wait_for_selector("article >> text=Kit")
                    page.wait_for_timeout(300)
                    cards = {c["name"]: c for c in page.evaluate("""() => [...document.querySelectorAll('article[data-sheet]')].filter((a) => a.querySelector('button')).map((a) => {
                        const lines = a.querySelectorAll('p');
                        return { name: a.querySelector('span.font-semibold').textContent, state: a.querySelector('[role=img]').getAttribute('aria-label'), height: a.getBoundingClientRect().height,
                          said: lines[1].textContent, shown: lines[1].getBoundingClientRect().height, text: a.textContent };
                    })""")}
                    check(f"{tag}: all shown", set(cards) == set(expected), sorted(cards))
                    check(f"{tag}: one height", len({c["height"] for c in cards.values()}) == 1, sorted({c["height"] for c in cards.values()}))
                    for name, (state, words) in expected.items():
                        c = cards.get(name, {"state": "", "said": "", "text": ""})
                        check(f"{tag}: {name}'s dot says {state}", c["state"].lower() == state, c["state"])
                        check(f"{tag}: {name}'s lines are not empty", c["said"].strip() != "", c)
                        check(f"{tag}: {name} says {words!r}", c["said"] == words, c["said"])
                        check(f"{tag}: {name} shows no arguments or output", not any(w in c["text"] for w in ("hunter2", "/etc", "xxxxxxx")), c["text"])
                    page.screenshot(path=SHOTS / f"{tag}.png", full_page=True)
                    context.close()
        said("gists")
    finally:
        host.terminate()
        host.wait(20)
        log.close()


def record(task, name, **fields):
    """A worker's state.json, written whole; with no marker files beside it
    the host takes it as it stands."""
    (task / name / "state.json").write_text(json.dumps({"phase": {"state": "idle"}} | fields))


def logged(task, name):
    return [json.loads(line) for line in (task / name / "log.jsonl").read_text().splitlines()]


# The Waiting cards a real host gives, each with its tag and the line under it.
TAGS = {
    "Wren": ("For approval", "Merge chat-width-4 to main?"),
    "Sable": ("For an answer", "Blue or green for the light theme?"),
    "Heron": ("For review", "The tide tables are rebuilt."),
}


def waits(browser, site, libri):
    """On a real host: each Waiting card says what it needs, a worker paused
    on another stays in Working untagged, opening a card that asks keeps it,
    and Mark read clears it."""
    root = Path(tempfile.mkdtemp(prefix="worker-waits-"))
    task = root / "state/task"
    task.mkdir(parents=True)
    now = int(time.time())
    seed(task, "Wren", "Chat width", ("talk", "chat-width-4 is ready."), read=False)
    seed(task, "Sable", "Light theme", ("talk", "Which do you want?"), read=False)
    seed(task, "Heron", "Tide tables", ("talk", "The tide tables are rebuilt."), read=False)
    seed(task, "Ibis", "Long job", read=False)
    seed(task, "Wigeon", "Waits on a worker at work", ("to", "Ibis: anything?"), read=False)
    size = lambda n: (task / n / "log.jsonl").stat().st_size
    record(task, "Wren", need={"kind": "approval", "ask": "Merge chat-width-4 to main?"})
    record(task, "Sable", read=size("Sable"), need={"kind": "answer", "ask": "Blue or green for the light theme?"})
    record(task, "Heron")
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    log = open(SHOTS / "waits-host.log", "w")
    host = scratch_host(libri, root, port, log)
    try:
        # Running only once the host is up, which would resume it.
        record(task, "Ibis", phase={"state": "running"})
        record(task, "Wigeon", phase={"state": "paused"}, awaited={"Ibis": now - 60})
        state = lambda n: json.loads((task / n / "state.json").read_text())
        check("waits: a wait on a worker at work holds", "Ibis" in state("Wigeon").get("awaited", {}), state("Wigeon"))
        calls = []

        def api(route):
            path = urlparse(route.request.url).path
            path = path[path.index("/api/"):]
            if path == "/api/events":
                return route.fulfill(status=200, headers={"content-type": "text/event-stream"}, body=": open\n\n")
            if route.request.method == "POST":
                calls.append(path)
            route.fulfill(response=route.fetch(url=f"http://127.0.0.1:{port}{path}"))

        for label, mobile in (("390", True), ("1440", False)):
            for theme in ("light", "dark"):
                tag = f"waits-{label}-{theme}"
                context = browser.new_context(viewport=dict(zip(("width", "height"), WIDTHS[label])), service_workers="block", is_mobile=mobile, has_touch=mobile)
                context.set_default_timeout(20000)
                if theme == "dark":
                    context.add_init_script("localStorage.setItem('theme', 'dark')")
                for section in ("waiting", "working", "sleeping", "archived"):
                    context.add_init_script(f"localStorage.setItem('libri-work-open-{section}', 'on')")
                context.route("**/api/**", api)
                page = context.new_page()
                page.goto(site + "/")
                if mobile:
                    page.locator("nav[aria-label=Places] button:has-text('Tasks')").first.click()
                page.wait_for_selector("article >> text=Heron")
                page.wait_for_timeout(300)
                cards = {c["name"]: c for c in page.evaluate(READ)}
                check(f"{tag}: one height", len({c["height"] for c in cards.values()}) == 1, sorted({c["height"] for c in cards.values()}))
                for name, (said_tag, ask) in TAGS.items():
                    c = cards.get(name, {"note": "", "said": ""})
                    check(f"{tag}: {name} is tagged {said_tag!r}", c["note"] == said_tag, c["note"])
                    check(f"{tag}: {name} says {ask!r}", c["said"] == ask, c["said"])
                check(f"{tag}: Wigeon, waiting on a worker, stays in Working untagged", cards.get("Wigeon", {}).get("note") == "Waiting on Ibis", cards.get("Wigeon"))
                page.screenshot(path=SHOTS / f"{tag}.png", full_page=True)
                context.close()
        # Opening a card that asks reads it and keeps it; Mark read clears it.
        context = browser.new_context(viewport={"width": 1440, "height": 900}, service_workers="block")
        context.set_default_timeout(20000)
        for section in ("waiting", "working", "sleeping", "archived"):
            context.add_init_script(f"localStorage.setItem('libri-work-open-{section}', 'on')")
        context.route("**/api/**", api)
        page = context.new_page()
        page.goto(site + "/")
        page.wait_for_selector("article >> text=Heron")
        card(page, "Sable").locator("button").first.click()
        page.wait_for_selector("text=· waiting")
        page.wait_for_timeout(1500)
        check("waits: opening Sable read it", "/api/work/Sable/read" in calls, calls)
        check("waits: Sable still asks after it is read", state("Sable").get("need") is not None, state("Sable"))
        page.screenshot(path=SHOTS / "waits-1440-light-open-asking.png")
        card(page, "Wren").click(button="right")
        page.get_by_role("menu").get_by_text("Mark read").click()
        page.wait_for_timeout(5000)
        check("waits: Mark read clears Wren", "/api/work/Wren/clear" in calls and state("Wren").get("need") is None, state("Wren"))
        check("waits: Wren sleeps once cleared", card(page, "Wren").get_by_role("img", name="Sleeping").count() == 1)
        check("waits: Sable still waits after the list is asked again", card(page, "Sable").get_by_role("img", name="Waiting").count() == 1)
        context.close()
        said("waits")
    finally:
        host.terminate()
        host.wait(20)
        log.close()


def main():
    handler = functools.partial(Pages, directory=str(SITE))
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    SHOTS.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for (label, size), theme in itertools.product(WIDTHS.items(), ("light", "dark")):
            tag = f"{label}-{theme}"
            if label == "390":
                phone(browser, site, theme, tag)
            else:
                desktop(browser, site, label, size, theme, tag)
            notes(browser, site, label, size, theme, tag, label == "390")
            crossing(browser, site, label, size, theme, tag, label == "390")
            said(tag)
        for theme in ("light", "dark"):
            side_widths(browser, site, theme)
            said(f"side widths {theme}")
            joined(browser, site, theme)
            said(f"joined {theme}")
            pill_mark(browser, site, theme)
            said(f"pill mark {theme}")
        if DAEMON:
            gists(browser, site, DAEMON)
            waits(browser, site, DAEMON)
        browser.close()
    print(f"{total} checks, {len(failures)} failed", flush=True)
    sys.exit(1 if failures else 0)


main()
