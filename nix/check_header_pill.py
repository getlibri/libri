#!/usr/bin/env python3
"""The wide layout at 1024 and over. The header pill lists what can still be
opened, closes up from the left, gets each item back in its fixed place, goes
when nothing is left, and carries the button to the column's ✕ with no travel
under reduced motion. The columns stand left column | chat | Temp | terminal |
thread | workers, and what needs room takes it from the left column, the
workers, then the chat; a thread keeps the workers. A collapse pressed by hand
holds through a reload, one forced by the room undoes itself. Each side has
one fold button that no fold moves, and the composer comes back to its height
through every order. A thread slides out of the workers column with its card
going to the column's edge in step, then back into the list, marked. Runs the built
site against a stand-in host, in Chromium at 1024, 1440, 1920 and 3100 in both
themes, through every order of opening and closing. Needs Playwright and a
browser, so it stays out of `nix flake check`. Usage: check_header_pill.py
SITE_DIR SHOTS_DIR
"""
import base64
import functools
import http.server
import itertools
import sys
import threading
import time
from pathlib import Path
from urllib.parse import urlparse

from playwright.sync_api import sync_playwright

SITE, SHOTS = Path(sys.argv[1]), Path(sys.argv[2])

WIDTHS = {"1024": (1024, 768), "1440": (1440, 900), "1920": (1920, 1080), "3100": (3100, 1300)}
WORKER = "Reed"
TERMINAL = ("Terminal", "Close the terminal", "Terminal")
TEMP = ("Temp chat, not remembered", "Close the temp chat", "Temp chat")
TEMP_ITEM = TEMP[0]
FOLD_LEFT, UNFOLD_LEFT = "Fold the left column", "Unfold the left column"
FOLD_WORKERS, UNFOLD_WORKERS = "Fold the workers column", "Unfold the workers column"
FIRE = "http://den.test"
# The pill's inset from the chat's edge; a head's clearance of a float at its
# corner (the head's 68px padding), as the even margins of 2026-11-03 set them.
INSET, CLEAR = 12, 68
# The centre of a fold button from its edge and from the top, as the pill's row.
BUTTON_CENTRE, ROW_CENTRE = 35, 35
SETTLE = 420
# While a thread slides out its card goes with it across the workers column's
# border, onto its rule; once the thread is out, and at rest, the card stands
# this far inside it, marked (joined only while sliding, 2026-11-05).
TRAVEL = 17
failures = []
total = 0
# Presses a covering column kept from their control, which every_order counts:
# at 1024 Temp and a thread lie over the pill, so those orders never reach it.
kept_off = 0

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


def host_reply(personas):
    now = int(time.time())
    return {
        "/api/me": {"name": "Libri", "personas": personas, "voice": None, "models": False, "forgets": []},
        "/api/work": {"threads": [{"id": "w1", "name": WORKER, "brief": "Check the header pill", "state": "working", "summary": "Reading the pill", "at": now - 300, "read": True}]},
        "/api/history": [
            {"i": 10, "kind": "user", "text": "Open the terminal and tell me what the pill does.", "date": "2026-11-01T12:40:00Z", "reaction": None},
            {"i": 11, "kind": "talk", "text": "It lists what can still be opened. Open one and it leaves the pill, and the column's own ✕ closes it.", "date": "2026-11-01T12:41:00Z", "reaction": None},
        ],
        "/api/usage": [],
        "/api/host": {"id": "stand-in", "name": "box", "assistant": "Libri", "version": "0", "uptime": 3600},
        "/api/pending": [],
        "/api/asks": [],
        "/api/scratchpad": {"id": None, "text": "", "working": False, "words": "", "thinking": "", "stopping": False, "queued": [], "memory": True, "asks": []},
        "/api/task": {"agents": [], "screen": None},
        "/api/connections": {"servers": [], "xai": None, "xai_signing": None, "xai_said": None},
        "/api/memory": {"state": "working", "reason": None, "since": None, "waiting": 0, "newest_summary": None, "latest_message": None, "model": "haiku"},
    }

def stand_in(context, personas, site):
    replies = context.replies = host_reply(personas)

    def api(route):
        path = urlparse(route.request.url).path
        path = path[path.index("/api/"):]
        if path == "/api/events":
            return route.fulfill(status=200, headers={"content-type": "text/event-stream"}, body=": open\n\n")
        if path in replies:
            return route.fulfill(json=replies[path])
        return route.fulfill(json={})

    def shell(ws):
        context.shells = getattr(context, "shells", 0) + 1
        ws.on_message(lambda _: ws.send(b"ada@box ~ $ "))

    context.route("**/api/**", api)
    context.route_web_socket("**/api/terminal*", shell)
    context.route(f"{FIRE}/api/host", lambda r: r.fulfill(json={"id": "den", "name": "den", "assistant": "Libri", "version": "0", "uptime": 60}, headers={"access-control-allow-origin": "*"}))

READ = """() => {
  const box = (e) => { const b = e.getBoundingClientRect(); return { left: b.left, top: b.top, right: b.right, bottom: b.bottom, width: b.width, height: b.height }; };
  const talk = document.querySelector('[data-talk]');
  const pill = talk.querySelector('header > div:last-child');
  const shut = document.querySelector('[aria-label="Close the terminal"]');
  const column = document.querySelector('section[aria-label="Terminal"]');
  const tempShut = document.querySelector('[aria-label="Close the temp chat"]');
  const tempColumn = document.querySelector('section[aria-label="Temp chat"]');
  const tempTitle = tempColumn?.querySelector('h2');
  return {
    items: [...pill.querySelectorAll('button')].map((b) => b.getAttribute('aria-label')),
    drawn: getComputedStyle(pill).display !== 'none',
    pill: box(pill),
    talk: box(talk),
    shut: shut && box(shut),
    column: column && box(column),
    tempShut: tempShut && box(tempShut),
    tempTitle: tempTitle && box(tempTitle),
    // Temp's section carries its own hair, the terminal's stands in a wrapper: the edge is inside the hair for both.
    tempColumn: tempColumn && { ...box(tempColumn), right: box(tempColumn).right - parseFloat(getComputedStyle(tempColumn).borderRightWidth) },
    carried: [...document.body.children].filter((e) => e.matches('div.fixed[aria-hidden]')).length,
  };
}"""


def room(w, shown, left=True, workers=True):
    """The rule, restated: the terminal takes a third of the chat window from
    2560 wide, half from 1536 down; a thread is the chat measure (740)
    with gutters and border, 781; Temp and the chat share their room equally up
    to 781 each; either narrows only where the window can't hold that; the chat keeps 480 by closing the left column, then the
    workers, then narrowing Temp, then itself; a thread splits its room with
    the chat equally, keeping both sides, then the workers alone, then
    neither, and covers the chat only where the halves fall under 480 even
    so; its place goes by the window alone. What has no room in the row
    stands in the chat's cell whole, one at a time (2026-11-05)."""
    side, pane = 288, 781
    half = lambda sides: min(pane, (w - sides * side) / 2)
    sides = next((n for n in (2, 1, 0) if half(n) >= 480), 0)
    thread = half(sides)
    share = 1 / 3 + (1 / 6) * min(1, max(0, (2560 - w) / (2560 - 1536)))
    beside = thread >= 480
    used = thread if "thread" in shown and beside else 0
    in_cell = "thread" in shown and not beside
    term, tmp = "terminal" in shown, "temp" in shown
    def at(l, k, narrow):
        space = w - used - (side if l else 0) - (side if k else 0)
        tools = share * space if term else 0
        temp = (min(pane, space - tools - 480) if narrow else min(pane, (space - tools) / 2)) if tmp else 0
        fits = space - temp - tools >= 480 and (not tmp or temp >= 280) and (not term or tools >= 360) and not in_cell
        return l, k, max(temp, 280) if tmp else 0, tools, fits
    workers = workers and not ("thread" in shown and thread >= 480 and sides == 0)
    tries = [at(l, k, n) for n in (False, True) for l, k in ((left, workers), (False, workers), (False, workers and "thread" in shown))]
    l, k, temp, tools, chat = next((t for t in tries if t[4]), tries[5])
    rest = w - used - (side if k else 0)
    return {
        "side": side, "thread": thread, "tools": tools, "temp": temp, "beside": beside,
        "left": l and chat, "workers": k, "chat": chat, "cell": rest,
    }


PLACED = """() => {
  const box = (s) => { const e = document.querySelector(s); if (!e) return null; const b = e.getBoundingClientRect(); return { left: b.left, right: b.right, width: b.width }; };
  return {
    w: document.querySelector('[data-talk]').closest('.grid').getBoundingClientRect().width,
    left: box('aside[aria-label="Hosts and settings"]'),
    talk: box('[data-talk]'),
    temp: box('[data-temp]'),
    tools: box('[data-tools]'),
    thread: box('[data-thread]'),
    workers: box('aside[aria-label="Work"]'),
  };
}"""


def near(a, b):
    return abs(a - b) < 1.01


def placed(page, where, shown, left=True, workers=True):
    """Every column stands where the rule says, in the fixed order."""
    r = page.evaluate(PLACED)
    w = r["w"]
    m = room(w, shown, left, workers)
    def at(name, box, right, width):
        check(f"{where}: {name} stands in its place", near(box["right"], right) and near(box["width"], width), (name, box, right, width, m))
    at("the workers", r["workers"], w, m["side"] if m["workers"] else 0)
    edge = r["workers"]["left"]
    if "thread" in shown and m["beside"]:
        at("the thread", r["thread"], edge, m["thread"])
        edge = r["thread"]["left"]
    if m["chat"]:
        if "terminal" in shown:
            at("the terminal", r["tools"], edge, m["tools"])
            edge = r["tools"]["left"]
        if "temp" in shown:
            at("Temp", r["temp"], edge, m["temp"])
            edge = r["temp"]["left"]
        check(f"{where}: the chat keeps its least", r["talk"]["width"] >= 479, r["talk"])
    start = m["side"] if m["left"] else 0
    check(f"{where}: the left column stands as the rule says", near(r["left"]["width"], start), (r["left"], m))
    at("the chat's cell", r["talk"], edge, edge - start)
    cell = r["talk"]
    if not m["chat"]:
        if "thread" in shown and not m["beside"]:
            at("the thread over the chat", r["thread"], cell["right"], cell["width"])
        if "terminal" in shown:
            at("the terminal over the chat", r["tools"], cell["right"], cell["width"])
        if "temp" in shown:
            at("Temp over the chat", r["temp"], cell["right"], cell["width"])
    return m


def reachable(page, target):
    """Whether a press on this control lands on it, not on what covers it."""
    return target.evaluate("""(e) => { const b = e.getBoundingClientRect();
        const at = document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2);
        return !!at && (e === at || e.contains(at)); }""")


def pill_button(page, column):
    return page.locator("[data-talk] header").get_by_role("button", name=column[0], exact=True)


def terminal_button(page):
    return pill_button(page, TERMINAL)


def card(page):
    return page.get_by_text(WORKER, exact=True).first


def open_thread(page):
    """From the workers column, or from it laid over the chat where the room took it."""
    if page.locator('aside[aria-label="Work"]').get_by_text(WORKER, exact=True).count() == 0:
        button(page, UNFOLD_WORKERS).click()
    card(page).click()
    page.wait_for_selector("[data-thread]")
    return True


def close_thread(page):
    if press(page, page.get_by_role("button", name="Close its work").first):
        page.wait_for_selector("[data-thread]", state="detached")
        return True
    return False


def button(page, name):
    return page.get_by_role("button", name=name, exact=True)


def press(page, target):
    global kept_off
    if target.count() == 0 or not reachable(page, target):
        kept_off += 1
        return False
    target.click()
    return True


def steps(page):
    return {
        "terminal": (lambda: press(page, pill_button(page, TERMINAL)), lambda: press(page, page.get_by_role("button", name=TERMINAL[1]))),
        "temp": (lambda: press(page, pill_button(page, TEMP)), lambda: press(page, page.get_by_role("button", name=TEMP[1]))),
        "thread": (lambda: open_thread(page), lambda: close_thread(page)),
    }


def rules_hold(page, where, shown, last, left=True, workers=True):
    m = placed(page, where, shown, left, workers)
    r = page.evaluate(READ)
    # The Library lives in the left column alone on a wide screen (2026-11-04),
    # not in the pill. The workers' button, away, is the pill's last segment
    # (2026-11-03), and floats at the row's right corner while Temp or the
    # terminal covers the pill.
    covered = not m["chat"] and bool({"terminal", "temp"} & set(shown))
    want = ([TEMP_ITEM] if "temp" not in shown else []) + (["Terminal"] if "terminal" not in shown else []) + ([UNFOLD_WORKERS] if not m["workers"] and not covered else []) + ([last] if last else [])
    check(f"{where}: the pill lists what can still be opened, in its fixed order", r["items"] == want, r["items"])
    check(f"{where}: an empty pill is gone and reserves nothing", r["drawn"] == bool(want) and (bool(want) or r["pill"]["width"] == 0), r)
    # The workers' float stands over the row's last column; a thread beside keeps it off Temp and the terminal.
    away = not m["workers"] and covered and not ("thread" in shown and m["beside"])
    if want:
        check(f"{where}: the pill's right end stays at the chat's edge", abs(r["talk"]["right"] - r["pill"]["right"] - INSET) < 0.51, (INSET, r))
        check(f"{where}: the pill centres on the head's row", abs(r["pill"]["top"] + r["pill"]["height"] / 2 - ROW_CENTRE) < 0.51, r["pill"])
    check(f"{where}: no ✕ in the pill", not any("Close the" in i for i in r["items"]), r["items"])
    if "temp" in shown:
        check(f"{where}: Temp's head stays on one line", r["tempTitle"]["height"] < 30, r["tempTitle"])
    for name, shut, column, last_in_row in (("terminal", "shut", "column", away), ("temp", "tempShut", "tempColumn", away and "terminal" not in shown or away and not m["chat"])):
        if name in shown:
            centre = r[shut]["left"] + r[shut]["width"] / 2
            far = r[column]["right"] - centre
            check(f"{where}: the {name} column closes with the ✕ clear of its edge", abs(far - (86 if last_in_row else 24)) < 0.51, (far, last_in_row))
    return r


def every_order(page, where, last, left=True, workers=True):
    actions = steps(page)
    runs = 0
    for opening in itertools.permutations(actions):
        for closing in itertools.permutations(actions):
            shown = set()
            for name in opening:
                if actions[name][0]():
                    shown.add(name)
                rules_hold(page, f"{where} open {'>'.join(opening)}", shown, last, left, workers)
            # What lies under another waits its turn: the one on top always closes.
            for _ in range(len(actions)):
                for name in closing:
                    if name in shown and actions[name][1]():
                        shown.discard(name)
                    rules_hold(page, f"{where} close {'>'.join(closing)}", shown, last, left, workers)
            check(f"{where}: all closed by their own ✕", not shown, shown)
            runs += 1
    return runs


SAMPLE = """([opener, closer]) => new Promise((done) => {
  const button = document.querySelector(`[data-talk] header button[aria-label="${opener}"]`);
  const start = button.getBoundingClientRect();
  const pill = button.closest('header > div:last-child');
  const before = pill.getBoundingClientRect();
  const frames = [];
  const began = performance.now();
  button.click();
  const frame = () => {
    const box = [...document.body.children].find((e) => e.matches('div.fixed[aria-hidden]'));
    const shut = document.querySelector(`[aria-label="${closer}"]`);
    const edge = pill.getBoundingClientRect();
    frames.push({
      at: performance.now() - began,
      box: box ? (({ left, top }) => ({ left, top }))(box.getBoundingClientRect()) : null,
      pill: pill.isConnected && getComputedStyle(pill).display !== 'none' ? edge.width : 0,
      shut: shut ? Number(getComputedStyle(shut).opacity) : null,
    });
    if (frames.length > 1 && !box) {
      const end = shut.getBoundingClientRect();
      done({ start: { left: start.left, top: start.top, width: start.width }, before: before.width, frames, end: { left: end.left, top: end.top } });
    } else requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
})"""


def flight(page, where, column=TERMINAL, moved=False):
    s = page.evaluate(SAMPLE, list(column[:2]))
    moving = [f for f in s["frames"] if f["box"]]
    check(f"{where}: the button travels", len(moving) >= 5, len(moving))
    if not moving:
        return s
    first, end = moving[0], s["end"]
    away = [abs(f["box"]["left"] - end["left"]) + abs(f["box"]["top"] - end["top"]) for f in moving]
    check(f"{where}: it never overshoots the ✕", all(b <= a + 0.51 for a, b in zip(away, away[1:])), away)
    check(f"{where}: it lands on the ✕", away[-1] < 3, away[-1])
    trip = abs(s["start"]["left"] - end["left"]) + abs(s["start"]["top"] - end["top"])
    check(f"{where}: it starts from the pill", abs(first["box"]["left"] - s["start"]["left"]) <= abs(end["left"] - s["start"]["left"]) * 0.6 + 1, (first, s["start"], trip))
    widths = {round(f["pill"], 1) for f in s["frames"]}
    # A move runs in a view transition, whose change lands a frame after the press.
    check(f"{where}: the pill closes up once, no springing width", len(widths) == 1 or (moved and len(widths) == 2), widths)
    check(f"{where}: the ✕ waits for the button", all(f["shut"] == 0 for f in moving[:-1]), [f["shut"] for f in moving])
    check(f"{where}: the ✕ stands when it lands", s["frames"][-1]["shut"] == 1, s["frames"][-1])
    check(f"{where}: the trip keeps the slide's time", 240 <= moving[-1]["at"] <= 400, moving[-1]["at"])
    check(f"{where}: nothing is left behind", page.evaluate(READ)["carried"] == 0)
    return s


FREEZE = """(opener) => {
  document.querySelector(`[data-talk] header button[aria-label="${opener}"]`).click();
  return new Promise((done) => queueMicrotask(() => {
    window.held = document.getAnimations().filter((a) => a.effect.getTiming().duration === 280);
    for (const a of window.held) a.pause();
    done(window.held.length);
  }));
}"""


def stills(page, name, column=TERMINAL):
    """The trip stopped at set moments, for the eye."""
    held = page.evaluate(FREEZE, column[0])
    check(f"{name}: slide and carry keep one clock", held >= 5, held)
    for ms in (0, 30, 80, 160):
        page.evaluate("(t) => { for (const a of window.held) a.currentTime = t; }", ms)
        page.wait_for_timeout(40)
        page.screenshot(path=SHOTS / f"{name}-trip-{ms:03}ms.png")
    page.evaluate("() => { for (const a of window.held) a.finish(); }")
    page.wait_for_timeout(120)



CLOSE_FREEZE = """(closer) => {
  document.querySelector(`[aria-label="${closer}"]`).click();
  return new Promise((done) => requestAnimationFrame(() => {
    window.held = document.getAnimations().filter((a) => a.effect.getTiming().duration === 280);
    for (const a of window.held) a.pause();
    done(window.held.length);
  }));
}"""


def close_stills(page, name, column=TERMINAL):
    """The close stopped at set moments, for the eye."""
    held = page.evaluate(CLOSE_FREEZE, column[1])
    check(f"{name}: the close slides", held >= 1, held)
    for ms in (0, 80, 160, 240):
        page.evaluate("(t) => { for (const a of window.held) a.currentTime = t; }", ms)
        page.wait_for_timeout(40)
        page.screenshot(path=SHOTS / f"{name}-close-{ms:03}ms.png")
    page.evaluate("() => { for (const a of window.held) a.finish(); }")
    page.wait_for_timeout(120)

FRAMES = """([control, column, mode]) => new Promise((done) => {
  if (mode === 'unfold') document.documentElement.dataset.unfold = 'beside';
  const frames = [];
  const began = performance.now();
  document.querySelector(control).click();
  const frame = () => {
    const box = document.querySelector(`[data-column]:has(section[aria-label="${column}"])`);
    if (box) { const { left, right } = box.getBoundingClientRect(); frames.push({ left, right }); }
    if (performance.now() - began > 420) {
      delete document.documentElement.dataset.unfold;
      done(frames);
    } else requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
})"""


def towards(page, where, control, column, mode, holds):
    """One open or close: the column's right edge holds still and, with motion,
    the left does all the moving, one way. Where the workers give way or come
    back that edge rides on them, and only the width goes one way."""
    frames = page.evaluate(FRAMES, [control, column, mode])
    if mode == "motion":
        check(f"{where}: seen moving", len(frames) >= 5, len(frames))
    widths = [f["right"] - f["left"] for f in frames]
    check(f"{where}: its width goes one way", widths == sorted(widths) or widths == sorted(widths, reverse=True), widths)
    if holds:
        held = {round(f["right"]) for f in frames}
        check(f"{where}: it keeps to its right edge", len(held) <= 1, held)
    return frames


def home_edges(browser, site):
    """Temp and the terminal, in a row and over the chat, with motion, reduced,
    and in a thread's unfold: every close goes where the open came from."""
    pill = '[data-talk] header button[aria-label="%s"]'
    for (label, width, thread), mode in itertools.product((("row, workers give way", 1024, False), ("row", 1440, False), ("over the chat", 1440, True), ("row", 1920, True), ("row", 3100, True)), ("motion", "reduced", "unfold")):
        context, page, errors = load(browser, site, (width, 800), "light", (), reduced=mode == "reduced")
        ready(page)
        shown = {"thread"} if thread else set()
        if thread:
            open_thread(page)
            page.wait_for_timeout(600)
        where = f"{width} {label} {mode}"
        holds = lambda name: room(width, shown)["workers"] == room(width, shown | {name})["workers"]
        moving = "motion" if mode == "motion" else "still"
        towards(page, f"{where}: terminal opening", pill % TERMINAL[0], TERMINAL[2], moving, holds("terminal"))
        page.wait_for_timeout(120)
        covering = page.locator("[data-covering] section[aria-label='Terminal']").count() == 1
        check(f"{where}: {label} as meant", covering == (label == "over the chat"))
        towards(page, f"{where}: terminal closing", f'[aria-label="{TERMINAL[1]}"]', TERMINAL[2], "motion" if mode == "motion" else mode, holds("terminal"))
        page.wait_for_timeout(120)
        towards(page, f"{where}: Temp opening", pill % TEMP[0], TEMP[2], moving, holds("temp"))
        page.wait_for_timeout(120)
        towards(page, f"{where}: Temp closing", f'[aria-label="{TEMP[1]}"]', TEMP[2], "motion" if mode == "motion" else mode, holds("temp"))
        check(f"{where}: no page errors", not errors, errors)
        context.close()
    said("every close goes home")


HOLD = """(control) => new Promise((done) => {
  const left = (s) => { const e = document.querySelector(s); return e ? e.getBoundingClientRect().left : null; };
  const frames = [];
  const began = performance.now();
  document.querySelector(control).click();
  const frame = () => {
    frames.push({ thread: left('[data-thread]'), workers: left('aside[aria-label="Work"]') });
    if (performance.now() - began > 460) done(frames);
    else requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
})"""


def still(frames, key):
    return {round(f[key] * 2) / 2 for f in frames if f[key] is not None}


def across(page, where, control, shown, after, left=True, left_after=True):
    """One press with the thread's and the workers' x read every frame."""
    w = page.viewport_size["width"]
    before, later = room(w, shown, left), room(w, after, left_after)
    frames = page.evaluate(HOLD, control)
    if "thread" in shown and "thread" in after:
        check(f"{where}: the thread's x holds", len(still(frames, "thread")) == 1, still(frames, "thread"))
    if before["workers"] and later["workers"]:
        check(f"{where}: the workers' x holds", len(still(frames, "workers")) == 1, still(frames, "workers"))
    page.wait_for_timeout(120)
    placed(page, where, after, left_after)


SETS = (("temp", "terminal", "thread"), ("terminal", "temp", "thread"), ("thread", "temp", "terminal"), ("thread", "terminal", "temp"), ("thread", "terminal"), ("thread", "temp"), ("terminal", "temp"), ("temp", "terminal"), ("thread",), ("terminal",), ("temp",))


def gather(page, names):
    """Opens these in this order; what a cover keeps from its button stays shut."""
    actions = steps(page)
    shown = set()
    for name in names:
        if actions[name][0]():
            shown.add(name)
        page.wait_for_timeout(420)
    return shown


def nothing_jumps(browser, site):
    """Each column closed and opened again, from every set of open ones: at
    every frame the thread and the workers stand still."""
    pill = '[data-talk] header button[aria-label="%s"]'
    shut = '[aria-label="%s"]'
    for label, size in WIDTHS.items():
        context, page, errors = load(browser, site, size, "light", ())
        for names in SETS:
            tag = ">".join(names)
            for column in ("terminal", "temp", "thread", "left"):
                if column != "left" and column not in names:
                    continue
                page.reload()
                ready(page)
                shown = gather(page, names)
                m = room(size[0], shown)
                placed(page, f"{label} {tag} all open", shown)
                where = f"{label} {tag} close {column}"
                if column == "left":
                    if m["left"]:
                        across(page, where, shut % FOLD_LEFT, shown, shown, True, False)
                        across(page, f"{label} {tag} reopen left", shut % UNFOLD_LEFT, shown, shown, False, True)
                    continue
                if column not in shown:
                    continue
                control = {"terminal": TERMINAL[1], "temp": TEMP[1], "thread": "Close its work"}[column]
                if not reachable(page, page.locator(shut % control).first):
                    continue
                across(page, where, shut % control, shown, shown - {column})
                page.wait_for_timeout(300)
                if column == "thread":
                    continue
                item = {"terminal": TERMINAL[0], "temp": TEMP[0]}[column]
                if page.locator(pill % item).count() and reachable(page, page.locator(pill % item)):
                    across(page, f"{label} {tag} reopen {column}", pill % item, shown - {column}, shown)
        check(f"{label} nothing jumps: no page errors", not errors, errors)
        context.close()
        said(f"{label}: nothing jumps")


def order_shots(browser, site):
    """All open, then each closed in turn, for the eye, in both themes."""
    for (label, size), theme in itertools.product(WIDTHS.items(), ("light", "dark")):
        context, page, errors = load(browser, site, size, theme, ())
        ready(page)
        names = ("temp", "terminal", "thread")
        shown = gather(page, names)
        page.mouse.move(size[0] // 2, size[1] // 2)
        page.screenshot(path=SHOTS / f"order-{label}-{theme}-all-open.png")
        for column in ("terminal", "temp", "thread", "left", "workers"):
            page.reload()
            ready(page)
            shown = gather(page, names)
            actions = steps(page)
            if column in actions:
                if column in shown:
                    actions[column][1]()
            elif page.get_by_role("button", name=f"Fold the {column if column == 'left' else 'workers'} column", exact=True).count():
                page.get_by_role("button", name=f"Fold the {column} column", exact=True).click()
            page.wait_for_timeout(520)
            page.mouse.move(size[0] // 2, size[1] // 2)
            page.screenshot(path=SHOTS / f"order-{label}-{theme}-no-{column}.png")
            page.evaluate("localStorage.removeItem('libri-left-folded'); localStorage.removeItem('libri-workers-folded')")
        if size[0] <= 1920:
            page.reload()
            ready(page)
            page.get_by_role("button", name=TERMINAL[0], exact=True).click()
            page.wait_for_timeout(450)
            close_stills(page, f"terminal-{label}-{theme}")
            if open_thread(page):
                page.wait_for_timeout(600)
        check(f"{label} {theme} shots: no page errors", not errors, errors)
        context.close()
    said("shots of the order")



def load(browser, site, size, theme, personas=(), reduced=False, seed=None, path="/", mobile=False):
    context = browser.new_context(viewport={"width": size[0], "height": size[1]}, reduced_motion="reduce" if reduced else "no-preference", service_workers="block", is_mobile=mobile, has_touch=mobile)
    # A full `nix flake check` on the same host starves the renderer past the default 30s.
    context.set_default_timeout(180000)
    seed = dict(seed or {})
    if theme == "dark":
        seed["theme"] = "dark"
    for key, value in seed.items():
        context.add_init_script(f"localStorage.getItem({key!r}) ?? localStorage.setItem({key!r}, {value!r})")
    stand_in(context, list(personas), site)
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(site + path)
    return context, page, errors

def ready(page, workers=True):
    page.wait_for_selector("[data-talk] header button[aria-label='Terminal']", timeout=30000)
    page.wait_for_timeout(600)
    if workers:
        page.wait_for_selector(f"aside[aria-label='Work'] >> text={WORKER}", timeout=30000)





LEFT = """() => {
  const box = (e) => { const b = e.getBoundingClientRect(); return { left: b.left, top: b.top, right: b.right, bottom: b.bottom, height: b.height, width: b.width }; };
  const fold = document.querySelector('[aria-label="Fold the left column"]');
  const hosts = document.querySelector('nav[aria-label="Hosts"]');
  const whole = hosts?.parentElement.parentElement;
  const head = whole?.querySelector('header');
  return {
    aside: box(document.querySelector('aside[aria-label="Hosts and settings"]')),
    whole: whole && box(whole),
    head: head && box(head),
    name: head?.textContent,
    nameBox: head && box(head.querySelector('span')),
    fold: fold && box(fold),
    hosts: hosts && box(hosts),
    rows: hosts ? [...hosts.querySelectorAll('button, a')].map((r) => r.getAttribute('aria-label')) : [],
    text: hosts ? [...hosts.querySelectorAll('button, a')].map((r) => r.textContent) : [],
    foot: whole ? [...whole.lastElementChild.querySelectorAll('button')].map((b) => b.getAttribute('aria-label')) : [],
    footBox: whole && box(whole.lastElementChild),
    title: document.title,
    saved: JSON.parse(localStorage.getItem('libri-hosts') ?? '[]').map((h) => h.name),
    folded: localStorage.getItem('libri-left-folded'),
    workersFolded: localStorage.getItem('libri-workers-folded'),
    workers: box(document.querySelector('aside[aria-label="Work"]')),
    overlay: [...document.querySelectorAll('.shadow-lift')].filter((e) => e.matches('.absolute.inset-y-0')).map(box),
  };
}"""


def left_column(browser, site):
    """The head, the fold, the hosts and the foot, in both themes, with a saved
    list from before names followed the hostname."""
    saved = f'[{{"id":"stand-in","name":"Ada","url":"{site}"}},{{"id":"den","name":"Old name","url":"{FIRE}"}}]'
    for (label, size), theme in itertools.product((("1440", WIDTHS["1440"]), ("1920", WIDTHS["1920"])), ("light", "dark")):
        name = f"{label}-{theme}"
        context, page, errors = load(browser, site, size, theme, (), seed={"libri-hosts": saved})
        ready(page)
        page.wait_for_timeout(400)
        r = page.evaluate(LEFT)
        check(f"{name}: the column is as wide as the workers", abs(r["aside"]["width"] - 288) < 1, r)
        check(f"{name}: the head is 70 and names the app", r["head"]["height"] == 70 and r["name"] == r["title"], r)
        # The Library heads the column and the machinery stands at its foot,
        # over Settings, as status does in a window's bottom bar (2026-11-04).
        check(f"{name}: the hosts stand at the foot, over Settings", r["hosts"]["bottom"] <= r["footBox"]["top"] + 0.5 and r["hosts"]["top"] > (r["whole"]["top"] + r["whole"]["bottom"]) / 2, r)
        check(f"{name}: this host, then the other", r["rows"] == ["Host: box", "Switch to den"], r["rows"])
        check(f"{name}: each with its name and latency", r["text"][0].startswith("box") and r["text"][0].endswith(" ms") and r["text"][1].startswith("den") and r["text"][1].endswith(" ms"), r["text"])
        check(f"{name}: saved names took the hostname with no migration", r["saved"] == ["box", "den"], r["saved"])
        check(f"{name}: no Chat, Temp or worker row", all(page.locator('aside[aria-label="Hosts and settings"]').get_by_text(t, exact=True).count() == 0 for t in ("Chat", "Temp", WORKER)))
        check(f"{name}: the foot is Settings alone", r["foot"] == ["Settings"], r["foot"])
        check(f"{name}: the fold button stands at the float's place", abs(r["fold"]["left"] + r["fold"]["width"] / 2 - BUTTON_CENTRE) < 0.51 and abs(r["fold"]["top"] + r["fold"]["height"] / 2 - ROW_CENTRE) < 0.51, r["fold"])
        check(f"{name}: the name starts clear of it", r["nameBox"]["left"] == CLEAR, r["nameBox"])
        page.mouse.move(size[0] // 2, size[1] // 2)
        page.screenshot(path=SHOTS / f"column-{name}-whole.png")
        button(page, FOLD_LEFT).click()
        page.wait_for_timeout(420)
        r = page.evaluate(LEFT)
        check(f"{name}: folded, it is gone", r["aside"]["width"] == 0 and r["folded"] == "1", r)
        page.mouse.move(size[0] // 2, size[1] // 2)
        page.screenshot(path=SHOTS / f"column-{name}-folded.png")
        page.reload()
        ready(page)
        check(f"{name}: folded through a reload", page.evaluate(LEFT)["aside"]["width"] == 0)
        spot = button(page, UNFOLD_LEFT).bounding_box()
        check(f"{name}: its reopening button floats at the top left", spot["x"] < 30 and spot["y"] < 30, spot)
        button(page, UNFOLD_LEFT).click()
        page.wait_for_timeout(420)
        r = page.evaluate(LEFT)
        check(f"{name}: unfolding brings it back", r["folded"] == "0" and abs(r["aside"]["width"] - 288) < 1, r)
        check(f"{name}: no page errors", not errors, errors)
        context.close()

    # The rows press as they should.
    context, page, errors = load(browser, site, WIDTHS["1440"], "light", (), seed={"libri-hosts": saved})
    ready(page)
    button(page, "Host: box").click()
    page.wait_for_timeout(300)
    check("this host's row opens Settings on Host", page.get_by_text("Running for").count() >= 1)
    page.screenshot(path=SHOTS / "column-host-pressed.png")
    page.keyboard.press("Escape")
    page.wait_for_timeout(200)
    button(page, "Switch to den").click()
    page.wait_for_load_state()
    check("another host's row switches to it", page.evaluate("localStorage.getItem('libri-current')") == FIRE, page.evaluate("JSON.stringify(localStorage)"))
    context.close()
    said("left column")


SIDES = """() => ["Hosts and settings", "Work"].map((name) => document.querySelector(`aside[aria-label="${name}"] > div`)?.getBoundingClientRect().width ?? 0)"""


def drag_grip(page, grip, by):
    box = grip.bounding_box()
    x, y = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
    page.mouse.move(x, y)
    page.mouse.down()
    page.mouse.move(x + by / 2, y)
    page.mouse.move(x + by, y)
    page.mouse.up()
    page.wait_for_timeout(100)


def side_drag(browser, site):
    """Either side column's sash moves both, kept through a reload, and a drag
    stops short of moving another column."""
    context, page, errors = load(browser, site, WIDTHS["1920"], "dark")
    ready(page)
    page.wait_for_timeout(400)
    grips = page.get_by_role("separator", name="Resize the side columns")
    check("sides: one sash on each column", grips.count() == 2, grips.count())
    check("sides: both start at 288", page.evaluate(SIDES) == [288, 288], page.evaluate(SIDES))
    drag_grip(page, grips.first, 32)
    check("sides: the left sash moves both", page.evaluate(SIDES) == [320, 320], page.evaluate(SIDES))
    page.reload()
    ready(page)
    page.wait_for_timeout(400)
    check("sides: a reload keeps the width", page.evaluate(SIDES) == [320, 320], page.evaluate(SIDES))
    drag_grip(page, grips.last, 40)
    check("sides: the workers' sash moves both, the other way", page.evaluate(SIDES) == [280, 280], page.evaluate(SIDES))
    drag_grip(page, grips.first, -200)
    check("sides: a drag stops at the least", page.evaluate(SIDES) == [240, 240], page.evaluate(SIDES))
    grips.first.dblclick()
    page.wait_for_timeout(100)
    check("sides: a double-click gives the default", page.evaluate(SIDES) == [288, 288], page.evaluate(SIDES))
    grips.first.focus()
    page.keyboard.press("ArrowRight")
    page.wait_for_timeout(100)
    check("sides: the arrow keys move both", page.evaluate(SIDES) == [304, 304], page.evaluate(SIDES))
    box = grips.last.bounding_box()
    dead = page.evaluate("""([x, top, bottom]) => {
        const out = [];
        for (let y = top + 1; y < bottom; y += 4) if (document.elementFromPoint(x, y)?.getAttribute("role") !== "separator") out.push(y);
        return out;
    }""", [box["x"] + 3, box["y"], box["y"] + box["height"]])
    check("sides: the workers' sash answers its whole height, over the section heads", not dead, dead)
    grips.first.dblclick()
    drag_grip(page, grips.first, -200)
    counts = page.evaluate("""() => {
        const c = document.querySelector("[data-counts]"), box = c.parentElement.getBoundingClientRect(), r = c.getBoundingClientRect();
        return { whole: r.right <= box.right + 0.5, shown: r.top < box.bottom - 1 };
    }""")
    check("sides: the head's counts are whole or gone at the least", counts["whole"] or not counts["shown"], counts)
    check("sides: no page errors", not errors, errors)
    context.close()
    # 1100 fits both columns at 288, and a jump past what fits lands at the last that does.
    context, page, errors = load(browser, site, (1100, 800), "dark")
    ready(page)
    page.wait_for_timeout(400)
    grips = page.get_by_role("separator", name="Resize the side columns")
    drag_grip(page, grips.first, 300)
    check("sides: a drag folds nothing", page.evaluate(SIDES) == [310, 310], page.evaluate(SIDES))
    check("sides: the chat keeps its least", page.locator("[data-talk]").bounding_box()["width"] >= 479, page.evaluate(SIDES))
    check("sides: no page errors at 1100", not errors, errors)
    context.close()


def collapse(browser, site):
    """A collapse pressed by hand holds, one forced by the room undoes itself,
    and a column reopened with no room lies over the chat from its own side."""
    for (label, size), theme in itertools.product(WIDTHS.items(), ("light", "dark")):
        w = size[0]
        name = f"{label}-{theme}"

        # Pressed, then forced: closing what forced it leaves the pressed one away.
        context, page, errors = load(browser, site, size, theme, ())
        ready(page)
        button(page, FOLD_WORKERS).click()
        page.wait_for_timeout(420)
        r = page.evaluate(LEFT)
        check(f"{name}: pressed, the workers are gone and it is kept", r["workers"]["width"] == 0 and r["workersFolded"] == "1", r)
        spot = button(page, UNFOLD_WORKERS).bounding_box()
        check(f"{name}: their button floats at the top right", w - spot["x"] - spot["width"] < 30 and spot["y"] < 30, spot)
        rules_hold(page, f"{name} workers pressed", set(), None, workers=False)
        page.mouse.move(w // 2, size[1] // 2)
        page.screenshot(path=SHOTS / f"collapse-{name}-workers-pressed.png")
        page.reload()
        ready(page, workers=False)
        rules_hold(page, f"{name} workers pressed, reloaded", set(), None, workers=False)
        press(page, pill_button(page, TERMINAL))
        page.wait_for_timeout(420)
        rules_hold(page, f"{name} workers pressed, terminal open", {"terminal"}, None, workers=False)
        button(page, TERMINAL[1]).click()
        page.wait_for_timeout(420)
        r = page.evaluate(LEFT)
        check(f"{name}: the terminal shut, a pressed collapse holds", r["workers"]["width"] == 0 and r["workersFolded"] == "1", r)
        button(page, UNFOLD_WORKERS).click()
        page.wait_for_timeout(420)
        r = page.evaluate(LEFT)
        check(f"{name}: reopened by hand it stands in the row", abs(r["workers"]["width"] - 288) < 1 and r["workersFolded"] == "0", r)
        check(f"{name} pressed: no page errors", not errors, errors)
        context.close()

        # Left pressed, then the room forcing the workers.
        context, page, errors = load(browser, site, size, theme, (), seed={"libri-left-folded": "1"})
        ready(page)
        shown = gather(page, ("terminal", "temp"))
        rules_hold(page, f"{name} left pressed, both open", shown, None, left=False)
        page.mouse.move(w // 2, size[1] // 2)
        page.screenshot(path=SHOTS / f"collapse-{name}-pressed-then-forced.png")
        # The last opened first: in the chat's cell it lies on top.
        for column in [c for c in ("temp", "terminal") if c in shown]:
            press(page, button(page, (TERMINAL if column == "terminal" else TEMP)[1]))
            page.wait_for_timeout(450)
        r = page.evaluate(LEFT)
        check(f"{name}: with room back the forced workers return, the pressed left does not", abs(r["workers"]["width"] - 288) < 1 and r["aside"]["width"] == 0 and r["folded"] == "1", r)
        page.reload()
        ready(page)
        check(f"{name}: and through a reload", page.evaluate(LEFT)["aside"]["width"] == 0)
        context.close()

        # Forced, then reopened by hand: over the chat, from its own side.
        for column, opener in (("terminal", TERMINAL), ("temp", TEMP)):
            context, page, errors = load(browser, site, size, theme, ())
            ready(page)
            pill_button(page, opener).click()
            page.wait_for_timeout(450)
            m = room(w, {column})
            if not m["left"]:
                button(page, UNFOLD_LEFT).click()
                page.wait_for_timeout(120)
                r = page.evaluate(LEFT)
                check(f"{name} {column}: the left column lies over the chat", r["aside"]["width"] == 0 and len(r["overlay"]) == 1 and r["overlay"][0]["left"] == 0, r)
                page.screenshot(path=SHOTS / f"collapse-{name}-{column}-left-over.png")
                button(page, FOLD_LEFT).click()
                page.wait_for_timeout(120)
                r = page.evaluate(LEFT)
                check(f"{name} {column}: put away, it is not pressed", r["folded"] == "0" and not r["overlay"], r)
            if not m["workers"]:
                button(page, UNFOLD_WORKERS).click()
                page.wait_for_timeout(120)
                r = page.evaluate(LEFT)
                check(f"{name} {column}: the workers lie over the chat", r["workers"]["width"] == 0 and len(r["overlay"]) == 1 and abs(r["overlay"][0]["right"] - w) < 1, r)
                page.screenshot(path=SHOTS / f"collapse-{name}-{column}-workers-over.png")
                button(page, FOLD_WORKERS).click()
                page.wait_for_timeout(120)
                r = page.evaluate(LEFT)
                check(f"{name} {column}: put away, it is not pressed", r["workersFolded"] != "1" and not r["overlay"], r)
            button(page, opener[1]).click()
            page.wait_for_timeout(450)
            rules_hold(page, f"{name} {column} shut", set(), None)
            check(f"{name} {column}: no page errors", not errors, errors)
            context.close()
        said(f"{name}: pressed and forced")


STAY = """([control, frames_for]) => new Promise((done) => {
  const named = (side) => [...document.querySelectorAll(`[aria-label="Fold the ${side} column"], [aria-label="Unfold the ${side} column"]`)];
  const frames = [];
  const began = performance.now();
  const sample = () => frames.push(Object.fromEntries(['left', 'workers'].map((side) => {
    const found = named(side);
    const { left, top, width, height } = found[0].querySelector('svg').getBoundingClientRect();
    return [side, { count: found.length, x: left + width / 2, y: top + height / 2, pill: !!found[0].closest('[data-talk]') }];
  })));
  sample();
  document.querySelector(control).click();
  const frame = () => {
    sample();
    if (performance.now() - began > frames_for) done(frames);
    else requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
})"""


def stays(page, where, control):
    """One press with both sides' buttons read every frame: one each, and no
    icon away from the place it holds at rest. The workers' button away joins
    the chat's pill (2026-11-03), so it holds its corner only out of the pill."""
    w = page.viewport_size["width"]
    frames = page.evaluate(STAY, [control, 460])
    check(f"{where}: seen moving", len(frames) >= 5, len(frames))
    for side, x in (("left", BUTTON_CENTRE), ("workers", w - BUTTON_CENTRE)):
        check(f"{where}: one {side} button in every frame", all(f[side]["count"] == 1 for f in frames), [f[side]["count"] for f in frames])
        off = max((abs(f[side]["x"] - x) + abs(f[side]["y"] - ROW_CENTRE) for f in frames if not f[side]["pill"]), default=0)
        check(f"{where}: the {side} icon never moves", off < 0.01, off)
    page.wait_for_timeout(120)


CORNERS = {"left": lambda w: {"x": 0, "y": 0, "width": 400, "height": 72}, "right": lambda w: {"x": w - 420, "y": 0, "width": 420, "height": 72}}


def corners(page, name, w):
    for side, clip in CORNERS.items():
        page.screenshot(path=SHOTS / f"corner-{name}-{side}.png", clip=clip(w))


def midway(page, name, control, w):
    """The press stopped early in its slide, whose easing is fast at the start, for the eye."""
    page.evaluate("""(control) => {
      document.querySelector(control).click();
      return new Promise((done) => requestAnimationFrame(() => {
        window.held = document.getAnimations().filter((a) => a.effect.getTiming().duration === 280);
        for (const a of window.held) { a.pause(); a.currentTime = 30; }
        done();
      }));
    }""", control)
    page.wait_for_timeout(60)
    corners(page, name, w)
    page.evaluate("() => { for (const a of window.held) a.finish(); }")
    page.wait_for_timeout(450)


def fold_buttons(browser, site):
    """Pressed and forced, in and out: each side keeps one button and no icon
    moves. Over the chat too, where the button stays above the column."""
    shut = '[aria-label="%s"]'
    pill = '[data-talk] header button[aria-label="%s"]'
    for (label, size), theme in itertools.product((("1024", WIDTHS["1024"]), ("1440", WIDTHS["1440"]), ("1920", WIDTHS["1920"])), ("light", "dark")):
        w = size[0]
        name = f"{label}-{theme}"
        context, page, errors = load(browser, site, size, theme, ())
        ready(page)
        page.mouse.move(w // 2, size[1] // 2)
        corners(page, f"{name}-rest", w)
        m = room(w, set())
        if m["left"]:
            stays(page, f"{name} left folded by hand", shut % FOLD_LEFT)
            corners(page, f"{name}-left-folded", w)
            page.reload()
            ready(page)
            stays(page, f"{name} left unfolded by hand", shut % UNFOLD_LEFT)
            midway(page, f"{name}-left-folding", shut % FOLD_LEFT, w)
            midway(page, f"{name}-left-unfolding", shut % UNFOLD_LEFT, w)
        else:
            stays(page, f"{name} left over the chat", shut % UNFOLD_LEFT)
            corners(page, f"{name}-left-over", w)
            stays(page, f"{name} left put away", shut % FOLD_LEFT)
        stays(page, f"{name} workers folded by hand", shut % FOLD_WORKERS)
        corners(page, f"{name}-workers-folded", w)
        stays(page, f"{name} workers unfolded by hand", shut % UNFOLD_WORKERS)
        midway(page, f"{name}-workers-folding", shut % FOLD_WORKERS, w)
        midway(page, f"{name}-workers-unfolding", shut % UNFOLD_WORKERS, w)
        for names in (("terminal",), ("temp",), ("terminal", "temp")):
            tag = "+".join(names)
            for column in names:
                opener = TERMINAL if column == "terminal" else TEMP
                if page.locator(pill % opener[0]).count() and reachable(page, page.locator(pill % opener[0])):
                    stays(page, f"{name} {tag}: forced by {column}", pill % opener[0])
            corners(page, f"{name}-{tag}-open", w)
            # The last opened lies on top where the two share the chat's cell.
            for column in reversed(names):
                opener = TERMINAL if column == "terminal" else TEMP
                if reachable(page, page.locator(shut % opener[1])):
                    stays(page, f"{name} {tag}: returned by {column}", shut % opener[1])
        if room(w, {"terminal"})["left"] != m["left"]:
            midway(page, f"{name}-forced", pill % TERMINAL[0], w)
        check(f"{name} fold buttons: no page errors", not errors, errors)
        context.close()
        said(f"{name}: fold buttons")


def temp_head(browser, site):
    """Temp's head beside the float, on one line at any width that holds it."""
    for w, theme in itertools.product((1024, 1100, 1200, 1272, 1274, 1280, 1440), ("light", "dark")):
        name = f"{w}-{theme}"
        context, page, errors = load(browser, site, (w, 800), theme, ())
        ready(page)
        press(page, pill_button(page, TEMP))
        page.wait_for_timeout(SETTLE)
        r = page.evaluate(READ)
        title = page.locator('section[aria-label="Temp chat"] h2')
        check(f"{name}: Temp's head stays on one line", r["tempTitle"]["height"] < 30, r["tempTitle"])
        check(f"{name}: the title ends short of the ✕", r["tempTitle"]["right"] <= r["tempShut"]["left"], r)
        said(f"{name}: title {r['tempTitle']['width']:.0f} wide, cut: {title.evaluate('(e) => e.scrollWidth > e.clientWidth')}")
        page.mouse.move(w // 2, 400)
        if w in (1024, 1280):
            page.screenshot(path=SHOTS / f"temp-head-{name}.png")
        context.close()


COMPOSER = """() => {
  const input = document.querySelector('[data-talk] textarea');
  const dock = input.closest('form');
  // What the box needs at its width now, by a textarea nobody has sized.
  const fresh = document.createElement('textarea');
  const style = getComputedStyle(input);
  for (const k of ['boxSizing', 'font', 'lineHeight', 'letterSpacing', 'paddingTop', 'paddingBottom', 'paddingLeft', 'paddingRight', 'borderWidth', 'whiteSpace', 'overflowWrap'])
    fresh.style[k] = style[k];
  fresh.style.width = `${input.getBoundingClientRect().width}px`;
  fresh.style.position = 'fixed';
  fresh.style.visibility = 'hidden';
  fresh.style.height = '0';
  fresh.rows = 1;
  fresh.value = input.value || input.placeholder;
  document.body.append(fresh);
  const needs = Math.min(fresh.scrollHeight, parseFloat(style.maxHeight));
  fresh.remove();
  return { input: input.getBoundingClientRect().height, needs, dock: dock.getBoundingClientRect().height, width: input.getBoundingClientRect().width };
}"""
LONG_DRAFT = "A draft long enough to wrap in a narrow chat and to hold its own lines in a wide one, " * 2


def settled(page):
    """A column's move runs as a view transition, during which a press lands on
    no control; waits it out, then the slide."""
    page.wait_for_function("() => !document.documentElement.dataset.unfold")
    page.wait_for_timeout(SETTLE)


def composer(browser, site):
    """Every order of opening and closing, with the slide on: a render while a
    column slides must not leave the composer the height of a sliver of chat."""
    for w, h, draft in ((1024, 768, ""), (1280, 800, ""), (1440, 900, ""), (1920, 1080, ""), (1440, 900, LONG_DRAFT)):
        where = f"{w}{' draft' if draft else ''}"
        context, page, errors = load(browser, site, (w, h), "light", ())
        ready(page)
        if draft:
            page.locator("[data-talk] textarea").fill(draft)
            page.wait_for_timeout(120)
        rest = page.evaluate(COMPOSER)
        actions = steps(page)

        def fits(step):
            r = page.evaluate(COMPOSER)
            check(f"{where} {step}: the composer is the height its words need", abs(r["input"] - r["needs"]) < 1, r)
            if not draft:
                check(f"{where} {step}: the empty composer is back at its height", abs(r["dock"] - rest["dock"]) < 1, (r, rest))

        runs = 0
        for opening in itertools.permutations(actions):
            for closing in itertools.permutations(actions):
                shown = set()
                for name in opening:
                    if actions[name][0]():
                        shown.add(name)
                    settled(page)
                    fits(f"open {'>'.join(opening)} at {name}")
                for _ in range(len(actions)):
                    for name in closing:
                        if name in shown and actions[name][1]():
                            shown.discard(name)
                            settled(page)
                            fits(f"close {'>'.join(closing)} at {name}")
                check(f"{where}: all closed by their own ✕", not shown, shown)
                runs += 1
        check(f"{where} composer: no page errors", not errors, errors)
        context.close()
        said(f"{where}: composer through {runs} orders")


SLIDING = """(control) => new Promise((done) => {
  const box = (e) => { if (!e) return null; const { left, right, width } = e.getBoundingClientRect(); return { left, right, width }; };
  const frames = [];
  const began = performance.now();
  const sample = () => {
    const thread = document.querySelector('[data-thread]');
    const cards = [...document.querySelectorAll('article[data-sheet]')];
    frames.push({
      thread: box(thread?.closest('[data-column]')),
      page: thread?.getBoundingClientRect().width,
      left: document.querySelector('aside[aria-label="Hosts and settings"] [data-column] > div')?.getBoundingClientRect().width ?? null,
      name: thread?.querySelector('[role="complementary"]')?.textContent.slice(0, 40),
      workers: box(document.querySelector('aside[aria-label="Work"]')),
      cards: Object.fromEntries(cards.map((c) => [c.querySelector('span.font-semibold').textContent, { left: c.getBoundingClientRect().left, height: c.getBoundingClientRect().height, radius: getComputedStyle(c).borderTopLeftRadius }])),
      unfold: document.documentElement.dataset.unfold ?? null,
      pseudo: document.getAnimations().filter((a) => a.effect.pseudoElement?.startsWith('::view-transition')).length,
      slides: document.getAnimations().filter((a) => a.effect.target?.dataset?.column && a.effect.getTiming().duration === 280).length,
    });
  };
  sample();
  document.querySelector(control).click();
  const frame = () => {
    sample();
    if (performance.now() - began > 900) done(frames);
    else requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
})"""
SECOND = "Tarn"


CARD_BUTTON = """(name) => { const c = [...document.querySelectorAll('article[data-sheet]')].find((a) => a.querySelector('span.font-semibold').textContent === name); c.querySelector('button').dataset.press = name; return '[data-press="' + name + '"]'; }"""


def sliding(page, name):
    return page.evaluate(SLIDING, page.evaluate(CARD_BUTTON, name))


def one_way(values):
    return values == sorted(values) or values == sorted(values, reverse=True)


def out_at(frames, full):
    """The first frame with the thread all the way out."""
    return next((n for n, f in enumerate(frames) if f["thread"] and abs(f["thread"]["width"] - full) < 1.01), len(frames) - 1)


def in_step(where, frames, name, full):
    """Every frame of the slide the card has gone as far along its travel as the thread along its width."""
    edge = frames[-1]["workers"]["left"]
    moving = [f for f in frames[: out_at(frames, full) + 1] if f["thread"] and f["cards"].get(name)]
    apart = [abs(f["thread"]["width"] / full - (edge + TRAVEL - f["cards"][name]["left"]) / TRAVEL) for f in moving]
    check(f"{where}: the card keeps step with the thread", apart and max(apart) < 0.03, max(apart or [9]))


def same_height(where, frames):
    heights = {c["height"] for f in frames for c in f["cards"].values()}
    check(f"{where}: every card one height on every frame", len(heights) == 1, heights)


def slide_stills(page, name, control):
    """The press stopped at set moments, for the eye."""
    page.evaluate("""(control) => {
      document.querySelector(control).click();
      return new Promise((done) => requestAnimationFrame(() => {
        window.held = document.getAnimations().filter((a) => a.effect.getTiming().duration === 280);
        for (const a of window.held) a.pause();
        done();
      }));
    }""", control)
    for ms in (20, 50, 100, 200):
        page.evaluate("(t) => { for (const a of window.held) a.currentTime = t; }", ms)
        page.wait_for_timeout(40)
        page.screenshot(path=SHOTS / f"{name}-{ms:03}ms.png")
    page.evaluate("() => { for (const a of window.held) a.finish(); }")
    page.wait_for_timeout(450)


PIXELS_APART = """async (shots) => {
  const read = async (shot) => {
    const image = await createImageBitmap(await (await fetch(`data:image/png;base64,${shot}`)).blob());
    const canvas = new OffscreenCanvas(image.width, image.height).getContext('2d');
    canvas.drawImage(image, 0, 0);
    return canvas.getImageData(0, 0, image.width, image.height).data;
  };
  const [a, b] = await Promise.all(shots.map(read));
  return a.reduce((most, value, n) => Math.max(most, Math.abs(value - b[n])), 0);
}"""


FOLD_BOX = """(label) => {
  const box = document.querySelector(`[aria-label="${label}"]`).parentElement;
  const head = [...document.querySelectorAll('header')].find((h) => !h.closest('[data-talk]') && (() => {
    const a = h.getBoundingClientRect(), b = box.getBoundingClientRect();
    return a.left <= b.left && a.right >= b.right && a.top <= b.top;
  })());
  const seen = getComputedStyle(box);
  return { paper: seen.backgroundColor, radius: seen.borderTopLeftRadius, bottom: box.getBoundingClientRect().bottom, head: getComputedStyle(head.parentElement).backgroundColor, hair: head.getBoundingClientRect().bottom };
}"""


def fold_box(page, where, label):
    """A fold button's paper covers a head going past and shows at no rest."""
    r = page.evaluate(FOLD_BOX, label)
    check(f"{where}: {label} has the head's paper on a round box", r["paper"] == r["head"] and r["radius"] != "0px", r)
    check(f"{where}: {label}'s box stops short of the hair", r["bottom"] < r["hair"] - 1, r)
    box = page.get_by_role("button", name=label, exact=True).locator("xpath=..").bounding_box()
    shot = lambda: page.screenshot(clip=box)
    filled = shot()
    page.evaluate("(label) => { document.querySelector(`[aria-label=\"${label}\"]`).parentElement.style.background = 'none'; }", label)
    bare = shot()
    page.evaluate("(label) => { document.querySelector(`[aria-label=\"${label}\"]`).parentElement.style.background = ''; }", label)
    # The grain's blend rounds the two apart by 1 in 255 on a few pixels.
    blank = page.context.new_page()
    apart = blank.evaluate(PIXELS_APART, [base64.b64encode(filled).decode(), base64.b64encode(bare).decode()])
    blank.close()
    check(f"{where}: {label}'s box cannot be seen at rest", apart <= 1, apart)


MOVE_CLOCK = """() => {
  const root = document.documentElement;
  const groups = document.getAnimations().filter((a) => a.effect.pseudoElement?.startsWith('::view-transition-group'));
  const edge = (name) => {
    const seen = getComputedStyle(root, `::view-transition-group(${name})`);
    const left = new DOMMatrix(seen.transform).e;
    return { left, right: left + parseFloat(seen.width) };
  };
  return {
    clocks: groups.map((a) => [a.effect.getTiming().duration, getComputedStyle(root, a.effect.pseudoElement).animationTimingFunction]),
    tools: edge('tools-column'),
    thread: edge('thread-column'),
  };
}"""


def two_workers(context, page):
    now = int(time.time())
    context.replies["/api/work"]["threads"].append({"id": "w2", "name": SECOND, "brief": "Check the card", "state": "working", "summary": "Reading the card", "at": now - 200, "read": True})
    page.reload()
    ready(page)
    page.wait_for_selector(f"aside[aria-label='Work'] >> text={SECOND}")


def thread_slide(browser, site):
    """The thread opens and closes by the slide Temp and the terminal use, out of
    the workers column's left edge, and the card goes to that edge with it.
    Only where a thread keeps the workers: at 1024 it folds them (2026-11-04)."""
    for (label, size), theme in itertools.product(WIDTHS.items(), ("light", "dark")):
        w = size[0]
        if not room(w, {"thread"})["workers"]:
            continue
        name = f"{label}-{theme}"
        context, page, errors = load(browser, site, size, theme, ())
        ready(page)
        two_workers(context, page)
        page.mouse.move(w // 2, size[1] // 2)
        fold_box(page, name, FOLD_WORKERS)
        if room(w, set())["left"]:
            fold_box(page, name, FOLD_LEFT)
        full = room(w, {"thread"})["thread"] if room(w, {"thread"})["beside"] else None
        rest = page.evaluate("() => [...document.querySelectorAll('article[data-sheet]')].map((c) => c.getBoundingClientRect().left)")
        opened = sliding(page, WORKER)
        settled_full = opened[-1]["thread"]["width"]
        full = full or settled_full
        drawn = [f for f in opened if f["thread"]]
        widths = [f["thread"]["width"] for f in drawn]
        check(f"{name} open: no morph, no view transition", all(f["pseudo"] == 0 and f["unfold"] is None for f in opened), opened[-1])
        check(f"{name} open: the column's slide runs", any(f["slides"] for f in opened) and len({round(x) for x in widths}) >= 5, widths)
        check(f"{name} open: its width runs from nothing, one way", widths[0] < 1 and one_way(widths) and abs(widths[-1] - full) < 1.01, widths)
        check(f"{name} open: it comes out of the workers' left edge", all(abs(f["thread"]["right"] - f["workers"]["left"]) < 1.01 for f in drawn), drawn[-1])
        check(f"{name} open: the page holds its width, clipped, never squeezed", all(abs(f["page"] - full) < 1.01 for f in drawn), {f["page"] for f in drawn})
        check(f"{name} open: a left column giving way holds its width too", len({round(f["left"]) for f in opened if f["left"] is not None}) <= 1, {f["left"] for f in opened})
        edge = opened[-1]["workers"]["left"]
        out = opened[out_at(opened, full)]
        check(f"{name} open: the card reaches the column's edge with the thread", abs(out["cards"][WORKER]["left"] - edge) < 0.51, out["cards"])
        # The corners ease with the slide, so the frame it lands on may hold a hair of them.
        check(f"{name} open: its left corners go square on the way", float(out["cards"][WORKER]["radius"][:-2]) < 0.5 and out["cards"][SECOND]["radius"] != "0px", out["cards"])
        check(f"{name} open: then it goes back into the list", abs(opened[-1]["cards"][WORKER]["left"] - edge - TRAVEL) < 0.51 and opened[-1]["cards"][WORKER]["radius"] != "0px", opened[-1]["cards"])
        check(f"{name} open: the other card stays", all(abs(f["cards"][SECOND]["left"] - rest[1 if rest[0] else 0]) < 0.51 for f in opened), opened[-1]["cards"])
        in_step(f"{name} open", opened, WORKER, full)
        same_height(f"{name} open", opened)
        page.mouse.move(w // 2, size[1] // 2)
        page.screenshot(path=SHOTS / f"slide-{name}-open.png")

        if label == "1440":
            press = page.evaluate(CARD_BUTTON, WORKER)
            slide_stills(page, f"slide-{name}-closing", press)
            slide_stills(page, f"slide-{name}-opening", press)

        # From one worker to another the thread stays and the cards swap.
        swap = sliding(page, SECOND)
        boxes = [f["thread"] for f in swap]
        check(f"{name} swap: the thread does not slide shut and open", all(b and abs(b["width"] - full) < 1.01 for b in boxes) and not any(f["slides"] for f in swap), boxes)
        lefts = {round(c["left"] - edge, 1) for f in swap for c in f["cards"].values()}
        check(f"{name} swap: nothing slides, so no card moves", lefts == {TRAVEL}, lefts)
        same_height(f"{name} swap", swap)
        page.screenshot(path=SHOTS / f"slide-{name}-swapped.png")

        # Tapping the open card folds it, back the way it came.
        closed = sliding(page, SECOND)
        drawn = [f for f in closed if f["thread"]]
        widths = [f["thread"]["width"] for f in drawn]
        check(f"{name} close: the slide runs, one way", any(f["slides"] for f in closed) and len({round(x) for x in widths}) >= 5 and one_way(widths), widths)
        check(f"{name} close: it goes into the workers' left edge", all(abs(f["thread"]["right"] - f["workers"]["left"]) < 1.01 for f in drawn), drawn[-1])
        check(f"{name} close: it is gone", closed[-1]["thread"] is None, closed[-1])
        check(f"{name} close: the card stays in the list", all(abs(f["cards"][SECOND]["left"] - edge - TRAVEL) < 0.51 for f in closed), closed[-1]["cards"])
        same_height(f"{name} close", closed)

        # By its own ✕ too.
        sliding(page, WORKER)
        shut = sliding_control(page, "Close its work")
        check(f"{name} ✕: it slides shut", any(f["slides"] for f in shut) and shut[-1]["thread"] is None, shut[-1])
        check(f"{name} ✕: the card goes back", abs(shut[-1]["cards"][WORKER]["left"] - edge - TRAVEL) < 0.51, shut[-1]["cards"])

        # Pressed again mid-slide it turns back.
        press = page.evaluate(CARD_BUTTON, WORKER)
        page.locator(press).click()
        page.wait_for_timeout(90)
        page.locator(press).click()
        page.wait_for_timeout(500)
        r = page.evaluate("() => ({ thread: !!document.querySelector('[data-thread]'), left: document.querySelector('[data-press]').closest('article').getBoundingClientRect().left })")
        check(f"{name} twice: shut, back in the list", not r["thread"] and abs(r["left"] - edge - TRAVEL) < 0.51, r)

        # A card that moves section while open stays marked in the list, with no slide.
        sliding(page, WORKER)
        context.replies["/api/work"]["threads"][0] |= {"state": "sleeping", "summary": None}
        page.wait_for_function("document.body.textContent.includes('Sleeping')", timeout=20000)
        heads = page.locator("button:has-text('Sleeping ·')")
        if heads.first.get_attribute("aria-expanded") == "false":
            heads.first.click()
        page.wait_for_selector(f"aside[aria-label='Work'] article >> text={WORKER}")
        page.wait_for_timeout(350)
        moved = page.evaluate(SLIDING, "html")
        check(f"{name} section: the open card keeps its place in the list", abs(moved[-1]["cards"][WORKER]["left"] - edge - TRAVEL) < 0.51, moved[-1]["cards"])
        check(f"{name} section: the thread stands", moved[-1]["thread"] and abs(moved[-1]["thread"]["width"] - full) < 1.01, moved[-1]["thread"])
        same_height(f"{name} section", moved)
        page.screenshot(path=SHOTS / f"slide-{name}-section.png")
        check(f"{name} slide: no page errors", not errors, errors)
        context.close()

        # Reduced motion: no slide, the thread snaps open and the card stays in the list.
        context, page, errors = load(browser, site, size, theme, (), reduced=True)
        ready(page)
        frames = sliding(page, WORKER)
        settled = [f for f in frames[1:] if f["thread"]]
        check(f"{name} reduced: nothing slides", not any(f["slides"] or f["pseudo"] for f in frames), frames[-1])
        check(f"{name} reduced: it snaps open", settled and all(abs(f["thread"]["width"] - full) < 1.01 for f in settled), settled[:2])
        check(f"{name} reduced: the card does not move", all(abs(f["cards"][WORKER]["left"] - edge - TRAVEL) < 0.51 for f in frames), settled[0]["cards"])
        frames = sliding(page, WORKER)
        check(f"{name} reduced: tapped again it snaps shut", all(f["thread"] is None for f in frames[1:]) and abs(frames[-1]["cards"][WORKER]["left"] - edge - TRAVEL) < 0.51, frames[-1])
        context.close()
        said(f"{name}: the thread's slide")

    # At both ends of the side columns' drag the card still reaches the edge, and comes back.
    for side in (240, 480):
        context, page, errors = load(browser, site, WIDTHS["1920"], "light", (), seed={"libri-side-width": str(side)})
        ready(page)
        frames = sliding(page, WORKER)
        last = frames[-1]
        check(f"{side}: the workers hold that width", abs(last["workers"]["width"] - side) < 1, last["workers"])
        out = frames[out_at(frames, last["thread"]["width"])]
        check(f"{side}: the card reaches the edge", abs(out["cards"][WORKER]["left"] - last["workers"]["left"]) < 0.51, out)
        check(f"{side}: and comes back into the list", abs(last["cards"][WORKER]["left"] - last["workers"]["left"] - TRAVEL) < 0.51, last)
        page.screenshot(path=SHOTS / f"slide-side-{side}.png")
        context.close()

    # A move: the terminal leaves the row for the thread, and the thread still slides.
    for theme in ("light", "dark"):
        context, page, errors = load(browser, site, WIDTHS["1440"], theme, ())
        ready(page)
        terminal_button(page).click()
        page.wait_for_timeout(450)
        placed(page, f"1440 {theme} terminal in the row", {"terminal"})
        press = page.evaluate(CARD_BUTTON, WORKER)
        for way, then, stage in (("open", {"thread", "terminal"}, "opening"), ("close", {"terminal"}, "closing")):
            page.evaluate("""(control) => {
              document.querySelector(control).click();
              const held = () => document.getAnimations().filter((a) => a.effect.pseudoElement?.includes('thread-column'));
              return new Promise((done) => { const look = () => (held().length ? done() : requestAnimationFrame(look)); look(); });
            }""", press)
            r = page.evaluate("""() => {
              const all = document.getAnimations().filter((a) => a.effect.pseudoElement?.startsWith('::view-transition') || a.effect.getTiming().duration === 280);
              for (const a of all) { a.pause(); a.currentTime = 40; }
              window.held = all;
              return all.filter((a) => a.effect.pseudoElement === '::view-transition-group(thread-column)').map((a) => ({ time: a.effect.getTiming().duration, ease: a.effect.getComputedTiming().easing }));
            }""")
            check(f"1440 {theme} {stage} with a move: the thread's group keeps the slide's time", len(r) == 1 and r[0]["time"] == 280, r)
            for ms in (40, 100, 200):
                page.evaluate("(t) => { for (const a of window.held) a.currentTime = t; }", ms)
                page.wait_for_timeout(60)
                clock = page.evaluate(MOVE_CLOCK)
                check(f"1440 {theme} {stage} with a move: every group on one clock at {ms}", len({tuple(c) for c in clock["clocks"]}) == 1 and clock["clocks"][0] == [280, "cubic-bezier(0.32, 0.72, 0, 1)"], clock["clocks"])
                check(f"1440 {theme} {stage} with a move: the terminal and the thread stay together at {ms}", abs(clock["tools"]["right"] - clock["thread"]["left"]) < 1.01, clock)
                page.screenshot(path=SHOTS / f"slide-move-{theme}-{stage}-{ms:03}ms.png")
            page.evaluate("(t) => { for (const a of window.held) a.currentTime = t; }", 40)
            page.wait_for_timeout(60)
            page.screenshot(path=SHOTS / f"slide-move-{theme}-{stage}.png")
            page.evaluate("() => { for (const a of window.held) a.finish(); }")
            page.wait_for_function("() => !document.documentElement.dataset.unfold")
            page.wait_for_timeout(450)
            placed(page, f"1440 {theme} {stage} with a move", then)
        check(f"1440 {theme} move: no page errors", not errors, errors)
        context.close()

    # A move at 1024, where the thread takes half the row from the chat and the
    # terminal goes into the chat's cell: the thread's column still runs on the
    # slide's time.
    context, page, errors = load(browser, site, WIDTHS["1024"], "light", ())
    ready(page)
    terminal_button(page).click()
    page.wait_for_timeout(450)
    button(page, UNFOLD_WORKERS).click()
    page.wait_for_timeout(200)
    press = page.evaluate(CARD_BUTTON, WORKER)
    page.evaluate("""(control) => {
      document.querySelector(control).click();
      const held = () => document.getAnimations().filter((a) => a.effect.pseudoElement?.includes('thread-column'));
      return new Promise((done) => { const look = () => (held().length ? done() : requestAnimationFrame(look)); look(); });
    }""", press)
    r = page.evaluate("""() => {
      window.held = document.getAnimations().filter((a) => a.effect.pseudoElement?.startsWith('::view-transition') || a.effect.getTiming().duration === 280);
      for (const a of window.held) { a.pause(); a.currentTime = 40; }
      return window.held.filter((a) => a.effect.pseudoElement?.includes('thread-column')).map((a) => [a.effect.pseudoElement, a.effect.getTiming().duration]);
    }""")
    check("1024 cover with a move: the thread's page runs on the slide's time", any(p.endswith("(thread-column)") and t == 280 for p, t in r), r)
    page.wait_for_timeout(60)
    page.screenshot(path=SHOTS / "slide-move-1024-opening.png")
    page.evaluate("() => { for (const a of window.held) a.finish(); }")
    page.wait_for_function("() => !document.documentElement.dataset.unfold")
    page.wait_for_timeout(450)
    placed(page, "1024 cover with a move", {"thread", "terminal"})
    check("1024 cover with a move: no page errors", not errors, errors)
    context.close()

    # Below 1024 the card still unfolds into the page.
    for size, mobile, label in (((1023, 768), False, "1023"), ((390, 844), True, "390")):
        context, page, errors = load(browser, site, size, "light", (), mobile=mobile)
        page.wait_for_selector("header button[aria-label='Terminal']", timeout=30000)
        page.wait_for_timeout(600)
        page.locator("nav[aria-label='Places']").get_by_role("button", name="Tasks").first.click()
        page.wait_for_selector(f"article >> text={WORKER}")
        before = page.locator("article[data-sheet]").first.bounding_box()
        check(f"{label}: the card keeps the sheet's name", page.evaluate("() => document.querySelector('article[data-sheet]').style.viewTransitionName") == "sheet-w1")
        page.locator("article[data-sheet] button").first.click()
        page.wait_for_selector("div.absolute[data-sheet]")
        page.wait_for_timeout(600)
        check(f"{label}: no column, the page over the list", page.locator("[data-thread]").count() == 0 and page.evaluate("() => document.querySelector('div.absolute[data-sheet]').style.viewTransitionName") == "sheet-w1")
        page.screenshot(path=SHOTS / f"slide-phone-{label}.png")
        check(f"{label}: no page errors", not errors, errors)
        context.close()
    said("the thread's slide, the rest")


def sliding_control(page, label):
    return page.evaluate(SLIDING, f'[aria-label="{label}"]')


def rooms(browser, site):
    """What stands in the chat's cell, one at a time, the cell's switch, and
    that the shell and Temp's talk live through every change of room."""
    # Windows on either side of each width the rule turns on.
    context, page, errors = load(browser, site, (1440, 900), "light", (), reduced=True)
    ready(page)
    for w in (1024, 1100, 1343, 1344, 1433, 1434, 1600, 1919, 2240, 2999, 3000):
        page.set_viewport_size({"width": w, "height": 900})
        page.wait_for_timeout(120)
        for names in SETS:
            page.reload()
            ready(page)
            shown = gather(page, names)
            placed(page, f"{w} {'>'.join(names)}", shown)
        said(f"{w}: every set stands by the rule")
    context.close()

    # 1024: the terminal, then Temp, each in the chat's whole cell, one at a
    # time; the cell's switch names the chat and both, and brings each back.
    context, page, errors = load(browser, site, WIDTHS["1024"], "light")
    ready(page)
    flight(page, "1024 terminal, workers giving way")
    page.wait_for_timeout(300)
    rules_hold(page, "1024 terminal", {"terminal"}, None)
    page.evaluate("document.querySelector('section[aria-label=\"Terminal\"]').dataset.lives = '1'")
    flight(page, "1024 Temp, the terminal going over the chat with it", TEMP, moved=True)
    page.wait_for_timeout(300)
    rules_hold(page, "1024 both", {"terminal", "temp"}, None)
    page.locator("section[aria-label='Temp chat'] textarea").fill("kept through a move")
    switch = page.get_by_role("tablist", name="In the chat's place")
    tabs = lambda: switch.get_by_role("tab").all_inner_texts()
    picked = lambda: switch.locator("[aria-selected=true]").first.text_content() if switch.locator("[aria-selected=true]").count() else None
    check("1024 both: the switch names the chat and both", tabs() == ["Chat", "Temp chat", "Terminal"], tabs())
    check("1024 both: Temp, opened last, is on top", picked() == "Temp chat" and reachable(page, button(page, TEMP[1])), picked())
    page.screenshot(path=SHOTS / "1024-temp-over-terminal.png")
    for name, control in (("Terminal", button(page, TERMINAL[1])), ("Chat", page.locator("[data-talk] textarea")), ("Temp chat", button(page, TEMP[1]))):
        switch.get_by_role("tab", name=name, exact=True).click()
        page.wait_for_timeout(150)
        check(f"1024 switch: {name} comes on top, in reach", picked() == name and reachable(page, control), picked())
        under = [s for s, n in (("[data-talk]", "Chat"), ("[data-temp]", "Temp chat"), ("[data-tools]", "Terminal")) if n != name]
        check(f"1024 switch: under {name} nothing takes a press", all(page.evaluate("(s) => document.querySelector(s).inert", s) for s in under), under)
        if name == "Chat":
            page.screenshot(path=SHOTS / "1024-chat-over-temp-and-terminal.png")
            # On top, the chat's pill is in reach again, and the workers'
            # button with it: nothing covers the pill for it to float clear of.
            items = page.evaluate(READ)["items"]
            check("1024 chat on top: the pill holds the workers' button alone", items == [UNFOLD_WORKERS], items)
    check("1024 switch: the shell lived", page.locator("section[aria-label='Terminal'][data-lives]").count() == 1)
    # Temp gone, the terminal has room in the row again, beside the chat.
    page.get_by_role("button", name=TEMP[1]).click()
    page.wait_for_timeout(600)
    check("1024: with Temp shut the terminal is back in the row, no switch", switch.count() == 0, picked())
    rules_hold(page, "1024 Temp shut", {"terminal"}, None)
    page.get_by_role("button", name=TERMINAL[1]).click()
    page.wait_for_timeout(600)
    check("1024 one at a time: no page errors", not errors, errors)
    context.close()
    context, page, errors = load(browser, site, WIDTHS["1024"], "light")
    ready(page)
    gather(page, ("terminal", "temp"))
    page.locator("section[aria-label='Temp chat'] textarea").fill("kept through a move")
    page.get_by_role("tablist", name="In the chat's place").get_by_role("tab", name="Terminal", exact=True).click()
    page.wait_for_timeout(150)
    page.get_by_role("button", name=TERMINAL[1]).click()
    page.wait_for_timeout(600)
    rules_hold(page, "1024 Temp left", {"temp"}, None)
    check("1024: Temp's talk lived through the move", page.locator("section[aria-label='Temp chat'] textarea").input_value() == "kept through a move")
    context.close()

    # At 1024 a thread stands beside the chat (2026-11-04) and Temp goes into
    # the chat's cell: both stay in reach, and closing the thread keeps Temp.
    context, page, errors = load(browser, site, WIDTHS["1024"], "light", ())
    ready(page)
    pill_button(page, TEMP).click()
    page.wait_for_timeout(450)
    open_thread(page)
    page.wait_for_timeout(700)
    placed(page, "1024 Temp, then a thread", {"temp", "thread"})
    check("1024: the thread opened last is in reach", reachable(page, page.get_by_role("button", name="Close its work").first))
    check("1024: Temp stays in reach beside it", reachable(page, button(page, TEMP[1])))
    page.screenshot(path=SHOTS / "1024-thread-over-temp.png")
    close_thread(page)
    page.wait_for_timeout(700)
    check("1024: closing it shows Temp", reachable(page, button(page, TEMP[1])))
    context.close()

    # A thread beside, then a narrower window: both want the cell.
    context, page, errors = load(browser, site, WIDTHS["1440"], "light", ())
    ready(page)
    shells = lambda: getattr(context, "shells", 0)
    open_thread(page)
    page.wait_for_timeout(700)
    pill_button(page, TERMINAL).click()
    page.wait_for_timeout(500)
    placed(page, "1440 thread, terminal over the chat", {"thread", "terminal"})
    page.screenshot(path=SHOTS / "1440-thread-terminal-over-chat.png")
    alive = shells()
    page.evaluate("document.querySelector('section[aria-label=\"Terminal\"]').dataset.lives = '1'")
    for w in (1024, 1920, 3100, 1440):
        page.set_viewport_size({"width": w, "height": 900})
        page.wait_for_timeout(500)
        placed(page, f"{w} after resizing", {"thread", "terminal"})
        check(f"{w}: the shell lived through the change of room", page.locator("section[aria-label='Terminal'][data-lives]").count() == 1 and shells() == alive, shells())
    page.set_viewport_size({"width": 1024, "height": 900})
    page.wait_for_timeout(500)
    check("1024 resized: the terminal is in reach", reachable(page, button(page, TERMINAL[1])))
    check("1024 resized: the thread stays in reach beside it", reachable(page, page.get_by_role("button", name="Close its work").first))
    button(page, TERMINAL[1]).click()
    page.wait_for_timeout(500)
    check("1024 resized: closing it shows the thread", reachable(page, page.get_by_role("button", name="Close its work").first))
    check("resizing: no page errors", not errors, errors)
    context.close()

    # 1920 with a thread: Temp, then the terminal, the terminal on top.
    context, page, errors = load(browser, site, WIDTHS["1920"], "light", ())
    ready(page)
    open_thread(page)
    page.wait_for_timeout(700)
    shown = gather(page, ("temp", "terminal"))
    check("1920: Temp and the terminal both open beside a thread", shown == {"temp", "terminal"}, shown)
    placed(page, "1920 thread, both", shown | {"thread"})
    page.screenshot(path=SHOTS / "1920-thread-temp-terminal.png")
    # Narrower: still one on top, the last.
    page.set_viewport_size({"width": 1440, "height": 900})
    page.wait_for_timeout(500)
    placed(page, "1440 thread, both", shown | {"thread"})
    check("1440: the terminal opened last lies on top", reachable(page, button(page, TERMINAL[1])))
    button(page, TERMINAL[1]).click()
    page.wait_for_timeout(500)
    check("1440: closing it shows Temp", reachable(page, button(page, TEMP[1])))
    check("cells: no page errors", not errors, errors)
    context.close()

    # A worker chosen from the workers laid over the chat brings them back.
    context, page, errors = load(browser, site, WIDTHS["1024"], "light", ())
    ready(page)
    pill_button(page, TERMINAL).click()
    page.wait_for_timeout(450)
    open_thread(page)
    page.wait_for_timeout(800)
    placed(page, "1024 thread from the laid-over workers", {"terminal", "thread"})
    check("1024: the overlay went with it", not page.evaluate(LEFT)["overlay"])
    close_thread(page)
    page.wait_for_timeout(800)
    placed(page, "1024 thread shut, the room takes the workers again", {"terminal"})
    context.close()

    # Pinned swaps in the workers' head.
    pin = '[{"id":"m1","role":"assistant","text":"Pinned words","at":1}]'
    context, page, errors = load(browser, site, WIDTHS["1440"], "light", (), seed={"libri-pins": pin})
    ready(page)
    aside = page.locator('aside[aria-label="Work"]')
    aside.get_by_role("button", name="Pinned").click()
    page.wait_for_timeout(200)
    check("Pinned swaps in the workers' place", aside.get_by_text("Pinned words").count() >= 1 and aside.get_by_text(WORKER, exact=True).count() == 0)
    page.screenshot(path=SHOTS / "1440-pinned.png")
    context.close()
    said("what stands in the chat's cell")


def trips(browser, site):
    # The trip, with its stills.
    for (label, size), theme in itertools.product(WIDTHS.items(), ("light", "dark")):
        name = f"{label}-{theme}"
        context, page, errors = load(browser, site, size, theme)
        ready(page)
        page.mouse.move(size[0] // 2, size[1] // 2)
        page.screenshot(path=SHOTS / f"{name}-1-closed.png")
        flight(page, name)
        page.wait_for_timeout(500)
        page.screenshot(path=SHOTS / f"{name}-2-open.png")
        before = page.evaluate(READ)
        page.get_by_role("button", name="Close the terminal").click()
        page.wait_for_timeout(30)
        check(f"{name}: closing needs no travel", page.evaluate(READ)["carried"] == 0)
        check(f"{name}: closing puts it back at once", "Terminal" in page.evaluate(READ)["items"], before)
        page.wait_for_timeout(400)
        page.screenshot(path=SHOTS / f"{name}-3-closed-again.png")
        stills(page, name)
        page.screenshot(path=SHOTS / f"{name}-4-open-after-trip.png")
        page.get_by_role("button", name=TERMINAL[1]).click()
        page.wait_for_timeout(450)
        flight(page, f"{name} Temp", TEMP)
        page.wait_for_timeout(500)
        page.screenshot(path=SHOTS / f"{name}-5-temp-open.png")
        page.get_by_role("button", name=TEMP[1]).click()
        page.wait_for_timeout(30)
        check(f"{name}: closing Temp needs no travel", page.evaluate(READ)["carried"] == 0)
        check(f"{name}: closing Temp puts it back at once, first", page.evaluate(READ)["items"][0] == TEMP_ITEM)
        page.wait_for_timeout(450)
        stills(page, f"{name}-temp", TEMP)
        press(page, terminal_button(page))
        page.wait_for_timeout(600)
        page.screenshot(path=SHOTS / f"{name}-6-temp-and-terminal.png")
        check(f"{name}: no page errors", not errors, errors)
        said(name)
        context.close()

    # Reduced motion: nothing travels.
    for (label, size), theme in itertools.product(WIDTHS.items(), ("light", "dark")):
        name = f"{label}-{theme}-reduced"
        context, page, errors = load(browser, site, size, theme, reduced=True)
        ready(page)
        # Where the terminal takes the workers' room, their button joins the pill's end.
        away = [UNFOLD_WORKERS] if not room(size[0], {"terminal"})["workers"] else []
        page.evaluate("""() => { window.seen = 0; new MutationObserver(() => { window.seen += document.querySelectorAll('body > div.fixed[aria-hidden]').length; }).observe(document.body, { childList: true }); }""")
        terminal_button(page).click()
        page.wait_for_timeout(50)
        check(f"{name}: nothing travels", page.evaluate("window.seen") == 0)
        check(f"{name}: nothing is sliding", page.evaluate("document.getAnimations().filter((a) => a.effect.getTiming().duration === 280).length") == 0)
        check(f"{name}: the pill has closed up", page.evaluate(READ)["items"] == [TEMP_ITEM, *away], page.evaluate(READ)["items"])
        page.screenshot(path=SHOTS / f"{name}-open.png")
        page.get_by_role("button", name="Close the terminal").click()
        page.wait_for_timeout(50)
        check(f"{name}: closed, back in its place", page.evaluate(READ)["items"] == [TEMP_ITEM, "Terminal"], page.evaluate(READ)["items"])
        pill_button(page, TEMP).click()
        page.wait_for_timeout(50)
        check(f"{name}: Temp's button does not travel", page.evaluate("window.seen") == 0)
        check(f"{name}: Temp has left the pill", page.evaluate(READ)["items"] == ["Terminal", *([UNFOLD_WORKERS] if not room(size[0], {"temp"})["workers"] else [])], page.evaluate(READ)["items"])
        page.get_by_role("button", name=TEMP[1]).click()
        page.wait_for_timeout(50)
        check(f"{name}: Temp is back first", page.evaluate(READ)["items"] == [TEMP_ITEM, "Terminal"], page.evaluate(READ)["items"])
        context.close()


def narrow(browser, site):
    """Below 1024: the terminal fills the screen, Temp opens from the top bar."""
    for size, mobile, name in (((1023, 768), False, "1023"), ((390, 844), True, "390")):
        context, page, errors = load(browser, site, size, "light", (), mobile=mobile)
        page.wait_for_selector("header button[aria-label='Terminal']", timeout=30000)
        page.wait_for_timeout(600)
        # Temp opens from the chat's top bar, not a place on the foot bar (2026-11-04).
        check(f"{name}: Temp stands in the top bar", page.locator(f"header button[aria-label='{TEMP_ITEM}']").count() == 1)
        check(f"{name}: Temp is no bar tab", page.locator("nav[aria-label='Places']").get_by_text("Temp").count() == 0)
        page.locator("header button[aria-label='Terminal']").first.click()
        page.wait_for_timeout(60)
        check(f"{name}: the terminal fills the screen", page.get_by_role("dialog", name="Terminal").count() == 1)
        check(f"{name}: its button stays", page.locator("header button[aria-label='Terminal']").count() == 1)
        check(f"{name}: nothing travels", page.locator("body > div.fixed[aria-hidden]").count() == 0)
        page.get_by_role("button", name="Close the terminal").click()
        page.wait_for_timeout(60)
        check(f"{name}: no page errors", not errors, errors)
        context.close()


def main():
    only = sys.argv[3:]

    SHOTS.mkdir(parents=True, exist_ok=True)
    pages = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Pages, directory=str(SITE)))
    threading.Thread(target=pages.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{pages.server_address[1]}"
    with sync_playwright() as p:
        browser = p.chromium.launch()
        def wanted(part):
            return not only or part in only
        # Every order, with the slide off so the states read at once.
        if wanted("orders"):
            for (label, size), theme in itertools.product(WIDTHS.items(), ("light", "dark")):
                where = f"{label} {theme}"
                context, page, errors = load(browser, site, size, theme, reduced=True)
                ready(page)
                runs = every_order(page, where, None)
                check(f"{where}: no page errors", not errors, errors)
                said(f"{where}: {runs} orders, {kept_off} presses covered so far")
                context.close()
            # The same with both columns pressed away, where each opening has more room.
            for label, size in WIDTHS.items():
                where = f"{label} pressed"
                context, page, errors = load(browser, site, size, "light", (), reduced=True, seed={"libri-left-folded": "1"})
                ready(page)
                runs = every_order(page, where, None, left=False)
                check(f"{where}: no page errors", not errors, errors)
                said(f"{where}: {runs} orders")
                context.close()
        if wanted("trips"):
            trips(browser, site)
        if wanted("jumps"):
            nothing_jumps(browser, site)
        if wanted("rooms"):
            rooms(browser, site)
        if wanted("collapse"):
            collapse(browser, site)
        if wanted("edges"):
            home_edges(browser, site)
        if wanted("left"):
            left_column(browser, site)
        if wanted("sides"):
            side_drag(browser, site)
        if wanted("buttons"):
            fold_buttons(browser, site)
        if wanted("composer"):
            composer(browser, site)
        if wanted("temp"):
            temp_head(browser, site)
        if wanted("slide"):
            thread_slide(browser, site)
        if wanted("shots"):
            order_shots(browser, site)
        if wanted("narrow"):
            narrow(browser, site)
        browser.close()
    print(f"{total} checks, {len(failures)} failed", flush=True)
    sys.exit(1 if failures else 0)


main()
