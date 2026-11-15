#!/usr/bin/env python3
"""A chat's room stays bounded through a turn. After a send, a turn runs six
calls whose printing rows then fold into one, so the talk gives back a few
hundred px mid-turn, and words stream in a line at a time; the spacer under the talk (footHold.ts) takes that up,
and must hold no more than a view's height and stop there. On 11-10 Temp's
spacer grew on itself every frame after such a shrink, and the chat
scrolled away without end on each send: in a thread the spacer stands
beside the talk, and the measure took it off the talk's height anyway.

Mid-turn too, the spacer holds no more than lets the person's message
stand at the talk's first line: on 11-13 a few lines' message in Temp on a
phone was held up under the Memory and Clear pills ("390 temp long").

Covers Temp, a worker's thread and a memory's chat, the threads that share
useThreadScroll, at 1440 and 390, against a stand-in host whose event
stream the check drives. Each is opened as the only pane of the host's
layout and found by its data-surface, its talk as the zoomed element in its
scroller and the spacer as what follows the talk, so the check runs against
any branch's built site whose threads keep those marks; it says which it
could not find rather than passing. Build the site to test with
`nix build .#site --out-link /tmp/site` on that branch. Needs Playwright and
Chromium, so it stays out of `nix flake check`.
Usage: check_thread_room.py SITE_DIR [SHOTS_DIR]
"""
import functools
import http.server
import json
import sys
import threading
import time
from pathlib import Path
from urllib.parse import urlparse

from playwright.sync_api import sync_playwright

SITE = Path(sys.argv[1])
SHOTS = Path(sys.argv[2]) if len(sys.argv) > 2 else None
failures = []
total = 0

ASKED = "List the files, please."
PRINTED = "\n".join(f"-rw-r--r-- 1 ada ada {n * 37:>6} file-{n:02}.txt" for n in range(40))
# A first message of a few lines, so that the turn's printing rows push it
# up past the talk's first line before they fold (Temp on a phone, 11-13).
ASKED_LONG = " ".join(["Look through each part of the archive and list every file in it, with its size."] * 4)
SAID = " ".join(["There are forty files in each part, all of them text, the largest under two kilobytes."] * 6)
# A drop this size or more between two frames proves the turn gave room back
# (six running rows folding into one), more than a streamed line takes up
# again; without one the check proved nothing about the spacer.
SHRINK = 100


def check(what, ok, detail=""):
    global total
    total += 1
    print(f"{'ok  ' if ok else 'FAIL'} {what}" + (f": {detail}" if detail != "" else ""), flush=True)
    if not ok:
        failures.append(what)


class Pages(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_):
        pass


# Stands in for EventSource, so the check sends the turn's events itself.
FAKE_EVENTS = """
(() => {
  const all = [];
  class Fake {
    constructor(url) { this.url = url; this.readyState = 1; all.push(this); setTimeout(() => this.onopen && this.onopen({}), 0); }
    addEventListener() {}
    removeEventListener() {}
    close() { this.readyState = 2; }
  }
  Fake.CONNECTING = 0; Fake.OPEN = 1; Fake.CLOSED = 2;
  window.EventSource = Fake;
  window.__emit = (e) => { for (const s of all) if (s.readyState === 1 && s.onmessage) s.onmessage({ data: JSON.stringify(e) }); };
})();
"""

# Each drawn frame, the talk's, spacer's and scroller's heights.
SAMPLER = """
([surface, asked]) => {
  const root = document.querySelector(`[data-surface="${surface}"]`);
  const talk = root && [...root.querySelectorAll("div")].find((d) => d.style.zoom);
  const room = talk && talk.nextElementSibling;
  let v = talk && talk.parentElement;
  while (v && !/(auto|scroll)/.test(getComputedStyle(v).overflowY)) v = v.parentElement;
  if (!talk || !room || !v) return `talk ${!!talk}, spacer ${!!room}, scroller ${!!v}`;
  // The talk's first line: the scroller's top and the top padding of the
  // talk's box, which clears what floats at the top (Temp's pills).
  let padded = talk;
  while (padded && padded !== v && !parseFloat(getComputedStyle(padded).paddingTop)) padded = padded.parentElement;
  const line = () => v.getBoundingClientRect().top + (padded && padded !== v ? parseFloat(getComputedStyle(padded).paddingTop) : 0);
  const question = () => [...root.querySelectorAll("p, div")].filter((e) => e.textContent.trim().startsWith(asked.slice(0, 40))).at(-1);
  window.__frames = [];
  // Read after the frame is drawn: a frame callback runs before the layout
  // and the hold's ResizeObserver, and would see a shrink the hold puts back
  // before anything is painted.
  const after = new MessageChannel();
  after.port1.onmessage = () => window.__frames.push({ t: performance.now(), talk: talk.offsetHeight, room: room.offsetHeight, scroll: v.scrollHeight, view: v.clientHeight, top: v.scrollTop, asked: (question()?.getBoundingClientRect().top ?? NaN) - line() });
  const tick = () => {
    after.port2.postMessage(0);
    window.__sampling = requestAnimationFrame(tick);
  };
  tick();
  return "";
}
"""


class Host:
    """The stand-in host: one thread of each kind, idle until the person sends."""

    def __init__(self, kind, asked=ASKED):
        self.kind = kind
        self.asked = asked
        self.sent = False

    def log(self):
        said = f"0|user: {self.asked}" if self.kind != "thread" else f"0|user: Tidy the folder.\n1|talk: On it.\n2|user: (From the person, typed on your page) {self.asked}"
        return said if self.sent else ("0|user: Tidy the folder.\n1|talk: On it." if self.kind == "thread" else "")

    def reply(self, path, method):
        if method == "POST":
            self.sent = True
            return {"id": "x"}
        live = self.sent
        return {
            "/api/me": {"name": "Libri", "personas": [], "voice": None, "models": False, "forgets": []},
            "/api/history": [{"i": 0, "kind": "talk", "text": "The main chat.", "date": "2026-11-10T18:00:00Z", "reaction": None}],
            "/api/work": {"threads": [{"id": "w1", "name": "Arlo", "brief": "Tidy the folder.", "state": "working", "gist": "Tidying", "at": int(time.time()) - 60, "read": True}]},
            "/api/work/w1": {"text": self.log() if self.kind == "thread" else ""},
            "/api/scratchpad": {"id": "scratchpad-s1" if self.kind == "temp" and live else None, "text": self.log() if self.kind == "temp" else "", "working": self.kind == "temp" and live, "words": "", "thinking": "", "stopping": False, "queued": [], "memory": True, "asks": []},
            "/api/memories": {"memories": [{"id": "m1", "name": "Tutor", "instructions": ""}]},
            "/api/memories/m1/chat": {"id": "memory-m1", "text": self.log() if self.kind == "memory-chat" else "", "working": self.kind == "memory-chat" and live, "words": "", "thinking": "", "stopping": False, "queued": [], "asks": []},
            "/api/host": {"id": "stand-in", "name": "box", "assistant": "Libri", "version": "0", "uptime": 3600},
            "/api/pending": [],
            "/api/asks": [],
            "/api/usage": [],
            "/api/usage/report": {"accounts": []},
            "/api/task": {"agents": [], "screen": None},
            "/api/memory": {"state": "working", "reason": None, "since": None, "waiting": 0, "newest_summary": None, "latest_message": None, "model": "haiku"},
        }.get(path)


PANES = {"temp": {"kind": "temp"}, "memory-chat": {"kind": "chat", "arg": "m1"}, "thread": {"kind": "thread", "arg": "w1"}}
RUNS = {"temp": "scratchpad-s1", "memory-chat": "memory-m1", "thread": "w1"}


def route(context, host):
    pane = PANES[host.kind]
    desk = {"spaces": [{"id": "s", "name": "Check", "root": {"t": "leaf", "id": "l", "pane": pane}, "focus": "l", "full": False}], "on": "s"}
    phone = {"spaces": [{"id": "s", "name": "Check", "items": [pane]}], "on": "s"}

    def api(r):
        url = urlparse(r.request.url)
        path = url.path[url.path.index("/api/"):]
        if path == "/api/events":
            return r.fulfill(status=200, headers={"content-type": "text/event-stream"}, body=": open\n\n")
        if path in ("/api/layout", "/api/phone-layout") and r.request.method == "GET":
            # The first read answers; a wait for the next version is refused,
            # and the page tries again five seconds on.
            if url.query:
                return r.fulfill(status=503)
            return r.fulfill(json={"version": 1, "layout": desk if path == "/api/layout" else phone, "undo": False})
        # What this stand-in doesn't serve is missing, as on an older host.
        answer = host.reply(path, r.request.method)
        return r.fulfill(status=404, body="") if answer is None else r.fulfill(json=answer)

    context.route("**/api/**", api)


def turn(page, run, said=SAID):
    """A turn: six calls run at once, each printing, then end together and
    fold into one row, giving back a few hundred px; words then stream in a
    line at a time, each line less than what was given back."""
    emit = lambda e: page.evaluate("(e) => window.__emit(e)", {"subagentRunId": run, "timestamp": int(time.time() * 1000), **e})
    calls = [f"c{n}" for n in range(6)]
    for n, c in enumerate(calls):
        emit({"type": "TOOL_CALL_START", "toolCallId": c, "toolCallName": "Bash"})
        emit({"type": "TOOL_CALL_ARGS", "toolCallId": c, "delta": json.dumps({"command": f"ls -l part-{n}"})})
        emit({"type": "TOOL_CALL_END", "toolCallId": c})
        emit({"type": "ACTIVITY_SNAPSHOT", "activityType": "tool_output", "messageId": f"o{n}", "content": {"toolCallId": c, "text": PRINTED}})
    page.wait_for_timeout(500)
    for n, c in enumerate(calls):
        emit({"type": "TOOL_CALL_RESULT", "toolCallId": c, "messageId": f"r{n}", "content": PRINTED})
    page.wait_for_timeout(300)
    for word in said.split():
        emit({"type": "TEXT_MESSAGE_CONTENT", "messageId": "m1", "delta": word + " "})
        page.wait_for_timeout(40)


def one(browser, site, kind, size, asked=ASKED):
    where = f"{size[0]} {kind}" + (" long" if asked != ASKED else "")
    host = Host(kind, asked)
    context = browser.new_context(viewport={"width": size[0], "height": size[1]}, service_workers="block", reduced_motion="reduce")
    context.set_default_timeout(15000)
    context.add_init_script(FAKE_EVENTS)
    route(context, host)
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(site + "/")
    surface = page.locator(f'[data-surface="{kind}"]').last
    try:
        surface.wait_for()
    except Exception:
        check(f"{where}: the thread opens", False, "no such data-surface")
        context.close()
        return
    page.wait_for_timeout(600)
    box = surface.locator("textarea").last
    box.fill(asked)
    surface.get_by_role("button", name="Send").last.click()
    # A long message may show folded; its opening words show either way.
    page.get_by_text(asked[:40]).last.wait_for()
    page.wait_for_timeout(400)
    missing = page.evaluate(SAMPLER, [kind, asked])
    if missing:
        check(f"{where}: talk, spacer and scroller found", False, missing)
        context.close()
        return
    # The long message's turn folds and its answer has not begun, as on
    # the phone (11-13): what is left is shorter than the view.
    turn(page, RUNS[kind], SAID if asked == ASKED else "")
    page.wait_for_timeout(2000)
    frames = page.evaluate("cancelAnimationFrame(window.__sampling), window.__frames")
    if SHOTS:
        page.screenshot(path=str(SHOTS / f"{where}.png"))
    i = max(range(len(frames) - 1), key=lambda n: frames[n]["talk"] - frames[n + 1]["talk"])
    a, b = frames[i], frames[min(i + 3, len(frames) - 1)]
    gave = a["talk"] - frames[i + 1]["talk"]
    check(f"{where}: the turn's talk gave room back mid-turn", gave >= SHRINK, f"{gave}px")
    # The hold itself: what the talk gave back the spacer took. Read three
    # frames on: a frame's read can land between two of the hold's measures.
    # As far as the cap lets it: no further than brings the person's message
    # down to the talk's first line.
    took = b["talk"] + b["room"] >= a["talk"] + a["room"] - 2 or 0 <= b["asked"] + 1 <= 24
    check(f"{where}: the spacer takes up what the talk gave back, up to the cap", took, f"talk {a['talk']}->{b['talk']}, spacer {a['room']}->{b['room']}, message {b['asked']:.0f}px from the first line")
    room = max(f["room"] for f in frames)
    view = frames[-1]["view"]
    check(f"{where}: the spacer stays within a view", room <= view, f"most {room}px, view {view}px")
    end = frames[-1]["t"]
    settled = [f["scroll"] for f in frames if f["t"] >= end - 1000]
    check(f"{where}: scrollHeight stops growing", max(settled) - min(settled) <= 2, f"{min(settled)}->{max(settled)}px over the last second")
    # While the spacer holds room, the person's message stands no higher
    # than the talk's first line: held above it, it stood under the pills.
    held = [f for f in frames[i + 1:] if f["room"] > 0]
    worst = min((f["asked"] for f in held), default=0)
    check(f"{where}: the held room keeps the person's message below the talk's first line", worst >= -1, f"{worst:.0f}px past it")
    check(f"{where}: no page errors", not errors, errors)
    context.close()


def main():
    if SHOTS:
        SHOTS.mkdir(parents=True, exist_ok=True)
    pages = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Pages, directory=str(SITE)))
    threading.Thread(target=pages.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{pages.server_address[1]}"
    started = time.time()
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for size in ((1440, 900), (390, 844)):
            for kind in ("temp", "thread", "memory-chat"):
                one(browser, site, kind, size)
        one(browser, site, "temp", (390, 844), ASKED_LONG)
        browser.close()
    print(f"{total} checks, {len(failures)} failed, {time.time() - started:.0f}s")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
