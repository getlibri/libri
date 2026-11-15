#!/usr/bin/env python3
"""Every interaction with the page answers within 400 ms: each button, menu,
tab, pane, dialog and its close, the keyboard's shortcuts, the phone bar's
gestures, typing, sending (until the person's bubble shows; the model's turn
is not the page's), scrolling and the sashes' drags. A target of 100 ms is
flagged, not failed.

The interactions are found, not listed by hand: from the first screen the
check clicks every control it can see, and every control that click brings
up, three levels deep, at 1440 and at 390 (a phone, touch), restoring the
screen between clicks. The gestures and keys no click reaches are driven by
hand below. `--inventory` lists each handler in the page's source with
whether the run reached it, so a control the crawl missed shows up.

Each interaction is timed from the input event (its timeStamp) to the frame
after the page last changed for it: the last DOM change, or the end of a
finite animation, before 200 ms with neither. Calls to the host that it
started are timed apart ("host"); the stand-in host answers at once, so the
time is the page's own. The browser's own interaction delay (Event Timing,
the INP measure) is kept beside it. A long press's hold is the person's
wait, not the page's, so it is taken off.

Runs the built site against measure_ui_performance.py's stand-in host (a
120-message chat, 36 workers) with check_phone_workspaces.py's layout store,
in Chromium. Fails on any interaction whose median, unthrottled, is over
400 ms, once it is over again timed alone. Needs Playwright and a browser, so
it stays out of `nix flake check`; the release runs it after the checks.
Usage: check_ui_latency.py LIBRI/site [--slow] [--runs N] [--jobs N]
  [--table FILE] [--inventory SRC_DIR] [--only desk|phone] [--depth N] [--limit S]
--slow also runs with the processor slowed 4 times, as a phone's is; --jobs
browsers time the found interactions side by side, per processor speed
(default 4); --table writes the full table, and its numbers beside it as JSON.
"""
import argparse
import atexit
import json
import os
import re
import signal
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ARGS = argparse.ArgumentParser()
ARGS.add_argument("site")
ARGS.add_argument("--slow", action="store_true")
ARGS.add_argument("--runs", type=int, default=3)
ARGS.add_argument("--table")
ARGS.add_argument("--inventory")
ARGS.add_argument("--only", choices=("desk", "phone"))
ARGS.add_argument("--depth", type=int, default=3)
ARGS.add_argument("--jobs", type=int, default=4)
# The whole run's limit in seconds, crawl and timing; past it the run stops
# its shares and fails. A run alone takes about 30 minutes.
ARGS.add_argument("--limit", type=int, default=int(os.environ.get("UI_LATENCY_LIMIT", 5400)))
# Internal: one worker process's share, written by the main run.
ARGS.add_argument("--part")
OPT = ARGS.parse_args()

LIMIT = 400
# Each driven interaction named as it starts, for finding one that stalls.
TRACE = bool(os.environ.get("UI_LATENCY_TRACE"))
TARGET = 100
# Runs of a miss timed again before it fails.
CONFIRM = 5
QUIET = 200
CAP = 5000
# The fewest interactions a whole run finds of a page that loaded (585: 482
# at 1440 and 103 at 390 on 2026-11-13). Far fewer means the stand-in host or
# the page broke, not that the page lost its controls; lower it knowingly
# when the page does.
FLOOR = 540
# A share that has finished no interaction for this long has stalled: it is
# stopped, and the run fails. Progress is an interaction finished, timed or
# not driven, never one begun: on 2026-11-13 a share whose browser driver had
# died began a new one every two minutes and held the release for 80 minutes
# under a watch that counted beginnings (a share now exits when its browser
# goes, and its own limit bounds the rest). Counting only timings stopped, on
# 2026-11-14, three desk shares whose first items each failed quickly to be
# driven, and lost what they had found.
STALL = int(os.environ.get("UI_LATENCY_STALL", 300))
# A browser and its share's stand-in host at their usual peak, in bytes: a
# share starts only when the run's cgroup has this much room below its cap
# (memory.high, else memory.max) and is not already stalling on memory. On
# 2026-11-13 a release's shares started in a worker cgroup pressed against
# its memory.high (memory pressure ~60%); throttled, three of four timed
# nothing in five minutes while the host had 80 GB free.
SHARE_MEMORY = int(os.environ.get("UI_LATENCY_SHARE_MEMORY", 2 * 1024**3))
# Memory pressure (PSI "full" avg10, percent) above which a share waits.
PRESSURE = 20
# How long a share waits for room before the run fails.
ROOM_WAIT = int(os.environ.get("UI_LATENCY_ROOM_WAIT", 900))
# A share's own limit beside the run's: its interactions' fair time, twice.
SHARE_EACH = 20
SHARE_BASE = 600
# For proving the limits: a share that hangs (UI_LATENCY_TEST_HANG=hang) or
# fails each interaction slowly (=slow), as a dead browser does.
TEST_HANG = os.environ.get("UI_LATENCY_TEST_HANG", "")
# The phone bar's hold (phone.tsx) and a message's hold (hold.tsx): the wait
# before the menu, which the person makes, not the page.
BAR_HOLD = 450
MESSAGE_HOLD = 400

# The stand-in host with the layout stores (check_phone_workspaces.py), which
# itself carries measure_ui_performance.py's chat and workers.
sys.path.insert(0, str(Path(__file__).resolve().parent))
_argv, sys.argv = sys.argv, [sys.argv[0], OPT.site, "serve"]
import check_phone_workspaces as workspaces  # noqa: E402
sys.argv = _argv
stand_in = workspaces.stand_in

from playwright.sync_api import Error as PlaywrightError  # noqa: E402
from playwright.sync_api import sync_playwright  # noqa: E402

# In the page from its first script: the input's time, every DOM change and
# finite animation after it, the host calls it started, and the browser's own
# interaction entries. `settle` resolves once the page has been still for
# QUIET ms, or at the cap.
PROBE = """
(() => {
  const L = window.__lat = { armed: false, frames: [] };
  const tick = (now) => { if (!L.armed) return; L.frames.push(now); requestAnimationFrame(tick); };
  L.arm = () => {
    Object.assign(L, { armed: true, pointer: false, changes: [], hold: 0, t0: null, end: null, last: null, first: null, calls: [], inp: 0, kind: null, frames: [], armedAt: performance.now() });
    requestAnimationFrame(tick);
  };
  // A pointer coming to rest is the input only for a control its hover opens.
  const input = (e) => { if (L.armed && L.t0 === null && (e.type !== 'pointerover' || L.pointer)) { L.t0 = e.timeStamp; L.kind = e.type; } };
  for (const t of ['pointerover', 'pointerdown', 'mousedown', 'touchstart', 'keydown', 'wheel', 'contextmenu', 'dblclick']) addEventListener(t, input, { capture: true, passive: true });
  const release = (e) => { if (L.armed && L.t0 !== null) L.end = e.timeStamp; };
  for (const t of ['pointerdown', 'pointermove', 'pointerup', 'touchmove', 'touchend', 'mouseup', 'keyup', 'wheel']) addEventListener(t, release, { capture: true, passive: true });
  const changed = (why) => { if (L.armed && L.t0 !== null) { const n = performance.now(); if (L.first === null) L.first = n; L.last = n; L.why = why; if (L.changes.length < 2000) L.changes.push(n); } };
  const named = (n) => { const e = n.nodeType === 1 ? n : n.parentElement; return e ? `${e.tagName.toLowerCase()}${e.getAttribute('aria-label') ? '[' + e.getAttribute('aria-label') + ']' : ''}.${String(e.className).slice(0, 40)}` : '?'; };
  new MutationObserver((ms) => changed(`${ms.at(-1).type} ${ms.at(-1).attributeName || ''} ${named(ms.at(-1).target)}`)).observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
  const own = window.fetch;
  window.fetch = function (resource, init) {
    const url = new URL(typeof resource === 'string' ? resource : resource.url, location.href).href;
    const ours = L.armed && L.t0 !== null && url.includes('/api/') && !url.includes('/api/events') && !url.includes('since=');
    const call = ours ? { url: new URL(url, location.href).pathname, start: performance.now(), end: null } : null;
    if (call) L.calls.push(call);
    const p = own.apply(this, arguments);
    if (call) p.then((r) => r.clone().text()).catch(() => {}).finally(() => { call.end = performance.now(); });
    return p;
  };
  try {
    new PerformanceObserver((l) => { for (const e of l.getEntries()) if (L.armed && L.t0 !== null && e.interactionId && e.startTime >= L.t0 - 1) L.inp = Math.max(L.inp, e.duration); })
      .observe({ type: 'event', durationThreshold: 16, buffered: false });
  } catch {}
  const moving = () => document.getAnimations().find((a) => a.playState === 'running' && Number.isFinite(a.effect?.getComputedTiming().endTime));
  // The frame that painted a change at `t`: the first frame begun after it.
  const paint = (t) => L.frames.find((f) => f > t) ?? performance.now();
  L.settle = (quiet, cap) => new Promise((done) => {
    const check = () => {
      const now = performance.now();
      if (L.t0 === null) return now - L.armedAt > cap ? (L.armed = false, done(null)) : requestAnimationFrame(check);
      const m = moving();
      if (m) changed(`animation ${m.animationName || m.transitionProperty || m.id} ${m.effect?.target ? named(m.effect.target) : ''}`);
      const open = L.calls.some((c) => c.end === null);
      const still = L.last === null ? L.t0 : L.last;
      if ((!open && now - still >= quiet) || now - L.t0 > cap) {
        const frames = L.frames.filter((f) => f >= L.t0);
        const gaps = frames.slice(1).map((f, i) => f - frames[i]);
        const settled = paint(still);
        L.armed = false;
        const held = L.t0 + L.hold;
        const shown = L.changes.find((c) => c >= held);
        return done({ held: shown === undefined ? null : paint(shown) - held, done: settled - L.t0, first: L.first === null ? null : paint(L.first) - L.t0,
          after_release: settled - Math.max(L.t0, L.end ?? L.t0), jank: Math.max(0, ...gaps), inp: L.inp, kind: L.kind,
          calls: L.calls.map((c) => ({ url: c.url, ms: (c.end ?? now) - c.start })), capped: now - L.t0 > cap, why: L.why || null, open: L.calls.filter((c) => c.end === null).map((c) => c.url) });
      }
      requestAnimationFrame(check);
    };
    requestAnimationFrame(check);
  });
})();
"""

# Every control a person can reach on the screen as it is: visible, not inert,
# within the page's width (a panel slid off the side is not reachable) and,
# when in view, the topmost thing at its centre. Each gets a signature; a run
# of the same control (a button on every message) is reduced to the last one
# in view.
CONTROLS = """() => {
  const sel = 'button,[role=button],[role=tab],[role=menuitem],[role=menuitemradio],[role=menuitemcheckbox],[role=option],[role=switch],[role=checkbox],[role=radio],[role=link],a[href],select,summary,input[type=checkbox],input[type=radio],input[type=range]';
  const W = innerWidth, H = innerHeight;
  const out = [];
  document.querySelectorAll('[data-lat]').forEach((e) => e.removeAttribute('data-lat'));
  for (const e of document.querySelectorAll(sel)) {
    if (e.closest('[inert],[aria-hidden=true]') || e.disabled || e.getAttribute('aria-disabled') === 'true') continue;
    if (!e.checkVisibility({ checkVisibilityCSS: true })) continue;
    const r = e.getBoundingClientRect();
    if (r.width < 2 || r.height < 2 || r.right <= 0 || r.left >= W) continue;
    const inView = r.bottom > 0 && r.top < H;
    if (inView) {
      const t = document.elementFromPoint(Math.min(W - 1, Math.max(0, r.left + r.width / 2)), Math.min(H - 1, Math.max(0, r.top + r.height / 2)));
      if (!t || !(t === e || e.contains(t) || t.contains(e))) continue;
    } else {
      // Out of view: reached by scrolling what holds it, unless something
      // lies over that (an open menu's backdrop) and takes the hand first.
      let s = e.parentElement;
      while (s && !(s.scrollHeight > s.clientHeight && /auto|scroll/.test(getComputedStyle(s).overflowY))) s = s.parentElement;
      if (s) {
        const b = s.getBoundingClientRect();
        const t = document.elementFromPoint(Math.min(W - 1, Math.max(0, (Math.max(b.left, 0) + Math.min(b.right, W)) / 2)), Math.min(H - 1, Math.max(0, (Math.max(b.top, 0) + Math.min(b.bottom, H)) / 2)));
        if (!t || !s.contains(t)) continue;
      }
    }
    const href = e.getAttribute('href');
    const outside = href && !href.startsWith('#') && new URL(href, location.href).origin !== location.origin;
    // A relative time ("2 min. ago") moves on between finding a control and timing it.
    const text = (e.innerText || e.value || '').trim().replace(/\\s+/g, ' ').replace(/\\b(\\d+ ?\\w{1,4}\\.? ago|just now)\\b/gi, '').trim().slice(0, 40);
    const name = e.getAttribute('aria-label') || e.getAttribute('title') || text || e.getAttribute('data-action') || e.getAttribute('data-item') || e.tagName.toLowerCase();
    const sig = [e.tagName, e.getAttribute('role') || '', e.getAttribute('aria-label') || '', e.getAttribute('title') || '', text, e.getAttribute('data-action') || '', e.getAttribute('data-item') || ''].join('|');
    // A list's rows (cards, pages, results) differ only in their words.
    const kind = [e.tagName, e.getAttribute('role') || '', e.getAttribute('aria-label') || '', e.closest('[role=list],ul,ol,[role=listbox],[role=menu]') ? 'list' : ''].join('|');
    out.push({ sig, kind, name, inView, top: r.top, outside: !!outside, el: e });
  }
  const bySig = new Map();
  for (const c of out) {
    const had = bySig.get(c.sig);
    if (!had || (c.inView && (!had.inView || c.top > had.top))) bySig.set(c.sig, c);
  }
  const perKind = new Map();
  const kept = [];
  for (const c of bySig.values()) {
    const n = perKind.get(c.kind) || 0;
    perKind.set(c.kind, n + 1);
    if (c.kind.endsWith('|list') && n >= 2) continue;
    c.el.setAttribute('data-lat', String(kept.length));
    kept.push({ sig: c.sig, name: c.name, outside: c.outside, lat: String(kept.length) });
  }
  return kept;
}"""

# Labels the same on many screens whose result differs by screen: kept per
# screen rather than once.
GENERIC = re.compile(r"^(close|back|cancel|done|ok|save|remove|delete|more|menu|open|clear|undo|copy|edit|add)\b", re.I)
# Leaves the page, or asks the operating system: not the page's to answer.
SKIP = {
    "Attach files or photos": "opens the system's file picker",
    "Attach": "opens the system's file picker",
    "Check for updates": "reloads the page to the newest build",
}


class Run:
    def __init__(self, browser, site, width, rate):
        self.browser, self.site, self.width, self.rate = browser, site, width, rate
        self.phone = width == "phone"
        self.context = None
        self.reloads = 0
        # Screens the crawl could not open, named in its report: a short
        # crawl says where it fell short.
        self.lost = []
        # The screen the last click left, so the next on it needs no reload.
        self._on, self._screen = None, None

    def fresh(self):
        """A first visit: no layout kept on the host, nothing in the page's storage."""
        workspaces.LAYOUTS.__init__()
        if self.context:
            self.context.close()
        size = (390, 844) if self.phone else (1440, 900)
        self.context = stand_in.context_for(self.browser, size, mobile=self.phone)
        self.context.add_init_script(PROBE)
        self.context.set_default_timeout(4000)
        self.page = self.context.new_page()
        self.page.on("dialog", lambda d: d.dismiss())
        self.page.on("filechooser", lambda _: None)
        self.cdp = self.page.context.new_cdp_session(self.page)
        self.cdp.send("Emulation.setCPUThrottlingRate", {"rate": self.rate})
        for attempt in (1, 2):
            try:
                self.page.goto(self.site + "/", timeout=60000)
                self.page.wait_for_selector("textarea[name=input]", timeout=60000)
                self.page.wait_for_selector("text=Reply 59.", timeout=60000)
                break
            except PlaywrightError:
                # A first load once hung on a busy machine; a second is the person's reload.
                if attempt == 2:
                    raise
        self.page.wait_for_timeout(600 * self.rate)
        self.reloads += 1

    def controls(self):
        return self.page.evaluate(CONTROLS)

    def at(self, path):
        """The screen reached by clicking `path` (signatures) from the first one."""
        self.fresh()
        for sig in path:
            found = [c for c in self.controls() if c["sig"] == sig]
            if not found:
                raise LookupError(f"no {sig.split('|')}")
            self.press(found[0])
            self.page.wait_for_timeout(250 * self.rate)

    def reach(self, control):
        """Brings an out-of-view control into view as a person does: their
        hand scrolls first (a wheel notch, a finger's drag), which lets go of
        the chat's hold on its foot; a script's scroll alone the hold puts
        back, and the control slid away under the tap."""
        target = self.page.locator(f'[data-lat="{control["lat"]}"]').first
        at = target.evaluate("""(e) => {
          const r = e.getBoundingClientRect();
          if (r.bottom > 0 && r.top < innerHeight) return null;
          let s = e.parentElement;
          while (s && !(s.scrollHeight > s.clientHeight && /auto|scroll/.test(getComputedStyle(s).overflowY))) s = s.parentElement;
          if (!s) return null;
          const b = s.getBoundingClientRect();
          return { x: b.left + b.width / 2, y: (Math.max(b.top, 0) + Math.min(b.bottom, innerHeight)) / 2, up: r.top < 0 };
        }""")
        if not at:
            return
        x, y, way = at["x"], at["y"], -1 if at["up"] else 1
        if self.phone:
            touch(self, along(x, y, x, y - 150 * way, 6))
        else:
            self.page.mouse.move(x, y)
            self.page.mouse.wheel(0, 120 * way)
        self.page.wait_for_timeout(150 * self.rate)
        target.scroll_into_view_if_needed(timeout=1500)
        self.page.wait_for_timeout(300 * self.rate)

    def press(self, control):
        self.reach(control)
        target = self.page.locator(f'[data-lat="{control["lat"]}"]').first
        try:
            if self.phone:
                target.tap(timeout=1500)
            else:
                # Gone under the pointer: its hover was the press (a tile's
                # notch opens its controls to a resting pointer).
                if self.rest_on(target):
                    return
                target.click(timeout=1500)
        except PlaywrightError:
            # Shown on hover (a tile's controls in its notch): the pointer goes
            # to the nearest shown ancestor first, as a person's would.
            target.evaluate("""(e) => { let a = e.parentElement; while (a && a.getBoundingClientRect().width < 2) a = a.parentElement; a?.setAttribute('data-lat-hover', ''); }""")
            self.page.locator("[data-lat-hover]").first.hover(timeout=1500)
            self.page.evaluate("document.querySelectorAll('[data-lat-hover]').forEach((e) => e.removeAttribute('data-lat-hover'))")
            target.tap(timeout=2500) if self.phone else target.click(timeout=2500)

    def rest_on(self, target):
        """Brings the pointer to rest on `target` and says whether it went
        away under it. A plain move: Playwright's hover waits for the target
        to take the pointer, which one that gives way never does."""
        b = target.bounding_box()
        if not b:
            return False
        self.page.mouse.move(b["x"] + b["width"] / 2, b["y"] + b["height"] / 2)
        self.page.wait_for_timeout(50)
        return not target.is_visible()

    def opens_on_hover(self, control):
        """Whether a pointer resting on the control is its press: it gives
        way under the pointer to what it stands for. The pointer goes off
        again after."""
        if self.phone:
            return False
        target = self.page.locator(f'[data-lat="{control["lat"]}"]').first
        gone = self.rest_on(target)
        self.page.mouse.move(1, self.page.viewport_size["height"] / 2)
        self.page.wait_for_timeout(250 * self.rate)
        return gone

    def measure(self, act, hold=0, pointer=False):
        """Times `act` from its input event to the frame after the page last
        changed for it: a pointer coming to rest on it when `pointer`."""
        self.page.evaluate("([h, p]) => { window.__lat.arm(); window.__lat.hold = h; window.__lat.pointer = p; }", [hold, pointer])
        act()
        # Settling counts frames, which a page that stops painting never
        # draws; a timer bounds the wait, as one stalled click held a share
        # for hours.
        got = self.page.evaluate(
            "([q, c]) => Promise.race([window.__lat.settle(q, c), new Promise((r) => setTimeout(() => r('stalled'), c + 5000))])",
            [QUIET, CAP],
        )
        if got == "stalled":
            raise LookupError("the page stopped painting")
        if got is None:
            raise LookupError("no input reached the page")
        if hold:
            # A hold is read from its threshold to the frame showing what it opens.
            if got["held"] is None:
                raise LookupError("the hold opened nothing")
            got["done"], got["first"] = got["held"], got["held"]
        return got

    def state(self):
        return frozenset(c["sig"] for c in self.controls())


def crawl(run, depth):
    """The interactions a click reaches, three screens deep: name, the path
    of clicks to its screen, its control's signature."""
    found, seen_screens, measured = [], set(), set()
    queue = [((), ())]
    while queue:
        if time.time() >= DEADLINE:
            fail(f"the crawl ({run.width}) was still going at the run's {OPT.limit} s limit")
        path, names = queue.pop(0)
        try:
            run.at(path)
        except (LookupError, PlaywrightError) as e:
            if not path:
                # The first screen never loaded: nothing found is no pass.
                fail(f"the page never loaded from the stand-in host ({run.width}): {str(e).splitlines()[0][:160]}")
            run.lost.append(" › ".join(names))
            print(f"   cannot reach {' › '.join(names)}: {str(e)[:80]}", flush=True)
            continue
        screen = run.state()
        if screen in seen_screens:
            continue
        seen_screens.add(screen)
        for control in run.controls():
            label = control["name"]
            key = (names, control["sig"]) if GENERIC.match(label) else control["sig"]
            if key in measured:
                continue
            measured.add(key)
            name = " › ".join(names + (label,))
            if control["outside"]:
                found.append({"name": name, "skip": "a link to another site"})
                continue
            if label in SKIP:
                found.append({"name": name, "skip": SKIP[label]})
                continue
            found.append({"name": name, "path": path, "sig": control["sig"], "label": label})
            if len(path) + 1 >= depth:
                continue
            # What the click brings up is a screen of its own, crawled in turn.
            try:
                if run.state() != screen:
                    run.at(path)
                c = next(c for c in run.controls() if c["sig"] == control["sig"])
                run.press(c)
                run.page.wait_for_timeout(300 * run.rate)
                after = run.state()
                if run.page.url.split("#")[0] != (run.site + "/") or not after - screen:
                    continue
                queue.append((path + (control["sig"],), names + (label,)))
            except (StopIteration, PlaywrightError) as e:
                why = "its control was gone when its screen was opened again" if isinstance(e, StopIteration) else str(e).splitlines()[0][:80]
                run.lost.append(name)
                print(f"   cannot open {name}: {why}", flush=True)
                continue
            finally:
                run.page.keyboard.press("Escape")
                run.page.wait_for_timeout(150 * run.rate)
    return found


def drive_click(run, item, runs):
    """`runs` timings of one found click, each from its own screen."""
    times = []
    for _ in range(runs):
        try:
            if run._on != item["path"] or run.state() != run._screen:
                run.at(item["path"])
                run._on, run._screen = item["path"], run.state()
            c = next((c for c in run.controls() if c["sig"] == item["sig"]), None)
            if c is None:
                run.at(item["path"])
                run._screen = run.state()
                c = next((c for c in run.controls() if c["sig"] == item["sig"]), None)
                if c is None:
                    return times, "its control was not on its screen again"
            run.reach(c)
            if "hover" not in item:
                item["hover"] = run.opens_on_hover(c)
            times.append(run.measure(lambda: run.press(c), pointer=item["hover"]))
            if run.page.url.split("#")[0] != run.site + "/":
                run._on = None
                continue
            run.page.keyboard.press("Escape")
            run.page.wait_for_timeout(120 * run.rate)
        except (LookupError, PlaywrightError) as e:
            run._on = None
            return times, str(e).split("\n")[0][:100]
    return times, None


# The interactions no single click reaches: keys, gestures, drags, typing,
# sending, scrolling. Each is a name, a set-up on a fresh page that returns
# the act, and how the act is read: "tap" from the input, ("hold", ms) with
# the person's hold taken off, "drag" as the worse of the longest frame
# during it and the settling after the finger lifts, "keys" as the slowest
# key's Event Timing.
def touch(run, points, hold_ms=0, wait=0.012):
    (x, y), *rest = points
    run.cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [{"x": x, "y": y}]})
    if hold_ms:
        run.page.wait_for_timeout(hold_ms)
    for x, y in rest:
        run.cdp.send("Input.dispatchTouchEvent", {"type": "touchMove", "touchPoints": [{"x": x, "y": y}]})
        time.sleep(wait)
    run.cdp.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})


def centre(run, selector):
    b = run.page.locator(selector).first.bounding_box()
    if not b:
        raise LookupError(f"no {selector} on screen")
    return b["x"] + b["width"] / 2, b["y"] + b["height"] / 2, b["width"]


def along(x0, y0, x1, y1, steps=10):
    return [(x0, y0)] + [(x0 + (x1 - x0) * i / steps, y0 + (y1 - y0) * i / steps) for i in range(1, steps + 1)]


def key(name):
    return lambda run: lambda: run.page.keyboard.press(name)


def typing(run):
    run.page.locator("textarea[name=input]").first.click()
    run.page.evaluate("""() => { window.__keys = []; new PerformanceObserver((l) => { for (const e of l.getEntries()) if (e.interactionId) window.__keys.push(e.duration); }).observe({ type: 'event', durationThreshold: 16 }); }""")
    return lambda: run.page.keyboard.type("The quick brown fox jumps over the lazy dog. ", delay=30)


def sending(run):
    run.page.locator("textarea[name=input]").first.click()
    run.page.keyboard.type("Latency probe message")
    run.page.wait_for_timeout(200 * run.rate)
    return lambda: run.page.keyboard.press("Enter")


def scrolling(run):
    b = run.page.locator("[data-said=talk]").last.bounding_box()
    x, y = b["x"] + b["width"] / 2, min(600, b["y"] + 40)

    def act():
        run.page.mouse.move(x, y)
        for _ in range(20):
            run.page.mouse.wheel(0, -120)
            run.page.wait_for_timeout(16)
    return act


def message_hold(run):
    b = run.page.locator("[data-said=talk]").last.bounding_box()
    x, y = b["x"] + 60, max(b["y"] + 20, 160)
    if run.phone:
        return lambda: touch(run, [(x, y)], hold_ms=MESSAGE_HOLD + 300)
    return lambda: run.page.mouse.click(x, y, button="right")


def bar_terminal(run):
    """The phone bar with a Terminal item to edit, close and bring back."""
    run.page.locator("[data-action=add]").click()
    run.page.locator("[role=menu] [data-add=terminal]").click()
    run.page.wait_for_timeout(400 * run.rate)
    return centre(run, "[role=tab][data-item=terminal]")


def bar_editing(run):
    """The bar in edit mode, as the person reaches it: a hold opens the
    item's menu (tileMenu.tsx), whose Edit the bar edits the bar."""
    x, y, w = bar_terminal(run)
    touch(run, [(x, y)], hold_ms=BAR_HOLD + 250)
    run.page.locator("[data-tile-menu] [data-row='Edit the bar']").click(timeout=3000)
    run.page.wait_for_selector(f"{workspaces.BAR}[data-editing]", timeout=3000)
    run.page.wait_for_timeout(300 * run.rate)
    return x, y, w


def bar_hold(run):
    x, y, _ = bar_terminal(run)
    return lambda: touch(run, [(x, y)], hold_ms=BAR_HOLD + 250)


def bar_reorder(run):
    x, y, w = bar_editing(run)
    return lambda: touch(run, along(x, y, x - w, y))


def bar_drop_close(run):
    x, y, _ = bar_editing(run)
    px, py, _ = centre(run, "[data-action=remove]")
    return lambda: touch(run, along(x, y, px, py, 16))


def bar_save(run):
    bar_editing(run)
    return lambda: run.page.locator("[data-edit-backdrop]").click(position={"x": 200, "y": 300})


def bar_flick(run):
    x, y, _ = bar_terminal(run)
    return lambda: touch(run, along(x, y, x, y - 90, 5))


def bar_undo(run):
    x, y, _ = bar_terminal(run)
    touch(run, along(x, y, x, y - 90, 5))
    run.page.wait_for_selector("[data-action=undo-close]", timeout=3000)
    run.page.wait_for_timeout(300 * run.rate)
    return lambda: run.page.locator("[data-action=undo-close]").first.tap()


def bar_swipe(run):
    run.page.locator("[data-action=spaces]").click()
    run.page.locator("[data-action=space-add]").click()
    run.page.wait_for_timeout(400 * run.rate)
    # The new one opens named in place: the name is kept, then Main is taken.
    run.page.keyboard.press("Enter")
    run.page.wait_for_timeout(300 * run.rate)
    if not run.page.locator("[role=dialog][aria-label=Workspaces]").count():
        run.page.locator("[data-action=spaces]").click()
    run.page.locator("[role=dialog][aria-label=Workspaces] [data-space=Main] button").first.click()
    run.page.wait_for_timeout(400 * run.rate)
    y = run.page.locator(workspaces.BAR).bounding_box()["y"] + 26
    return lambda: touch(run, along(300, y, 120, y, 8))


def split_drag(run):
    run.page.keyboard.press("Alt+Shift+Enter")
    run.page.wait_for_timeout(600 * run.rate)
    x, y, _ = centre(run, "[role=separator]")

    def act():
        run.page.mouse.move(x, y)
        run.page.mouse.down()
        for k in range(1, 25):
            run.page.mouse.move(x + 160 * k / 24, y)
            run.page.wait_for_timeout(12)
        run.page.mouse.up()
    return act


def tab(run):
    return run.page.locator("button[title^='Chat (Alt Shift 1)']").first


def second_space(run):
    run.page.keyboard.press("Alt+Shift+KeyN")
    run.page.wait_for_timeout(500 * run.rate)
    return lambda: run.page.keyboard.press("Alt+Shift+Digit1")


def gestures(phone):
    out = [
        ("type 45 keys in the composer (slowest key)", typing, "keys"),
        ("send a message (until the bubble shows)", sending, "tap"),
        ("scroll the chat up (20 wheel steps)", scrolling, "drag"),
        ("Escape with nothing open", key("Escape"), "tap"),
        ("Ctrl K (search)", key("Control+k"), "tap"),
        ("hold a message (its menu)" if phone else "right-click a message (its menu)", message_hold, ("hold", MESSAGE_HOLD) if phone else "tap"),
    ]
    if phone:
        out += [
            ("bar: hold an item (its menu)", bar_hold, ("hold", BAR_HOLD)),
            ("bar: drag an item sideways (reorder)", bar_reorder, "drag"),
            ("bar: drop an item on × (close)", bar_drop_close, "drag"),
            ("bar: tap outside (save the edit)", bar_save, "tap"),
            ("bar: flick an item up (close)", bar_flick, "drag"),
            ("bar: Undo after a close", bar_undo, "tap"),
            ("bar: swipe to the next workspace", bar_swipe, "drag"),
        ]
    else:
        out += [
            ("Alt Shift N (new workspace)", key("Alt+Shift+KeyN"), "tap"),
            ("Alt Shift Space (open anything)", key("Alt+Shift+Space"), "tap"),
            ("Alt Shift Enter (split the tile)", key("Alt+Shift+Enter"), "tap"),
            ("Alt Shift F (tile full screen)", key("Alt+Shift+KeyF"), "tap"),
            ("Alt Shift / (show the keys)", key("Alt+Shift+Slash"), "tap"),
            ("Alt Shift 1 (back to the first workspace)", second_space, "tap"),
            ("double-click a workspace tab (rename)", lambda r: lambda: tab(r).dblclick(position={"x": 12, "y": 12}), "tap"),
            ("right-click a workspace tab (its menu)", lambda r: lambda: tab(r).click(button="right", position={"x": 12, "y": 12}), "tap"),
            ("drag the split between two tiles", split_drag, "drag"),
        ]
    return out


def drive_gesture(run, g, runs):
    name, setup, how = g
    times = []
    for _ in range(runs):
        try:
            run.fresh()
            act = setup(run)
            run.page.wait_for_timeout(100 * run.rate)
            got = run.measure(act, hold=how[1] if isinstance(how, tuple) else 0)
            if how == "drag":
                got["done"] = max(got["after_release"], got["jank"])
            elif how == "keys":
                keys = run.page.evaluate("window.__keys") or [0]
                got["done"], got["first"] = max(keys), statistics.median(keys)
            times.append(got)
        except (LookupError, PlaywrightError) as e:
            run._on = None
            return times, str(e).split("\n")[0][:100]
    run._on = None
    return times, None


def summary(times):
    done = [t["done"] for t in times]
    calls = [c["ms"] for t in times for c in t["calls"]]
    urls = sorted({c["url"] for t in times for c in t["calls"]})
    inp = [t["inp"] for t in times if t["inp"]]
    # Nothing on screen changed: the interaction may not have taken.
    still = all(t["first"] is None for t in times)
    firsts = [t["first"] for t in times if t["first"] is not None]
    return {"still": still, "first": round(statistics.median(firsts)) if firsts else None, "median": round(statistics.median(done)), "worst": round(max(done)), "inp": round(max(inp)) if inp else 0,
            "host": round(max(calls)) if calls else None, "calls": urls, "capped": any(t.get("capped") for t in times)}


def inventory(src, names):
    """Each handler in the page's source and whether a measured interaction
    carries a label written beside it."""
    rows = []
    said = " ".join(names).lower()
    for f in sorted(Path(src).glob("*.tsx")):
        lines = f.read_text().splitlines()
        for i, line in enumerate(lines):
            for h in re.findall(r"\b(on(?:Click|PointerDown|KeyDown|ContextMenu|DoubleClick|TouchStart|Change|Submit))=", line):
                near = " ".join(lines[max(0, i - 6): i + 6])
                labels = re.findall(r'(?:aria-label|title)="([^"{]+)"', near) + re.findall(r">\s*([A-Z][A-Za-z ]{2,30})\s*<", near)
                hit = next((l for l in labels if l.strip().lower() in said), None)
                rows.append((f"{f.name}:{i + 1}", h, labels[0] if labels else "", bool(hit)))
    return rows


def put(path, text):
    """Writes `text` whole: a reader sees the old file or the new, never half."""
    part = path.with_name(path.name + ".part")
    part.write_text(text)
    part.replace(path)


def measure_part(spec):
    """One worker's share: the clicks and gestures named in `spec`, at its
    width and processor speed, timed `runs` times each."""
    width, rate = spec["width"], spec["rate"]
    col = f"{width} cpu/{rate}"
    out = {"results": {}, "not_driven": {}}
    put(Path(spec["out"]), json.dumps(out))
    stage = Path(spec["at"] + ".stage") if spec.get("at") else None

    def said(what):
        """Where the share is: kept for the run's watch to name when it
        stops the share, and printed under UI_LATENCY_TRACE."""
        if stage:
            put(stage, f"{time.strftime('%T')} {what}")
        if TRACE:
            print(f"   {col} {Path(spec.get('at') or '-').stem} {time.strftime('%T')} {what}", file=sys.stderr, flush=True)

    said("starting playwright")
    with sync_playwright() as p:
        said("launching chromium")
        browser = p.chromium.launch(args=["--enable-gpu-rasterization", "--ignore-gpu-blocklist"])
        said("serving the stand-in host")
        run = Run(browser, workspaces.serve(), width, rate)
        said("ready")
        at = Path(spec["at"]) if spec.get("at") else None
        if TEST_HANG == "hang":
            time.sleep(10**9)
        jobs = [(item["name"], lambda item=item: drive_click(run, item, spec["runs"])) for item in spec["items"]]
        jobs += [(g[0], lambda g=g: drive_gesture(run, g, spec["runs"])) for g in gestures(width == "phone") if g[0] in spec["gestures"]]
        for item in spec["items"]:
            item["path"] = tuple(item["path"])
        for name, drive in jobs:
            began = time.time()
            said(f"> {name}")
            if TEST_HANG == "slow":
                time.sleep(30)
                times, why = [], "a test's slow failure"
            else:
                times, why = drive()
            said(f"< {round(time.time() - began, 1)} s {len(times)} timed {why or ''}")
            if times:
                out["results"][name] = {col: summary(times)}
            if why:
                out["not_driven"][f"{width}: {name}"] = why
            # Kept as it goes, so a share stopped later keeps what it found.
            put(Path(spec["out"]), json.dumps(out))
            # Progress for the run's watch: an interaction finished.
            if at:
                at.write_text(f"{name}, not driven: {why}" if why and not times else name)
            if not browser.is_connected():
                # Nothing after this could be timed; say so rather than fail
                # each interaction at its timeout.
                print(f"   the browser went away at {name}", file=sys.stderr, flush=True)
                sys.exit(3)
        browser.close()
    return out


def cgroup_dir():
    """This process's cgroup (v2), where its browsers are charged."""
    try:
        rel = Path("/proc/self/cgroup").read_text().split("::", 1)[1].strip()
        return Path("/sys/fs/cgroup") / rel.lstrip("/")
    except (OSError, IndexError):
        return None


def memory_room():
    """The bytes left below the tightest cap on this cgroup and its parents,
    the worst memory pressure among them, and the cgroup that sets each; the
    host's MemAvailable where no cap is set."""
    room, room_at, pressure, pressure_at = None, "the host", 0.0, None
    d = cgroup_dir()
    while d and d != Path("/sys/fs/cgroup") and d.exists():
        try:
            current = int((d / "memory.current").read_text())
            for name in ("memory.high", "memory.max"):
                v = (d / name).read_text().strip()
                if v != "max" and (room is None or int(v) - current < room):
                    room, room_at = int(v) - current, f"{d.relative_to('/sys/fs/cgroup')} ({name})"
            full = re.search(r"full avg10=([\d.]+)", (d / "memory.pressure").read_text())
            if full and float(full[1]) > pressure:
                pressure, pressure_at = float(full[1]), str(d.relative_to("/sys/fs/cgroup"))
        except (OSError, ValueError):
            pass
        d = d.parent
    if room is None:
        info = Path("/proc/meminfo").read_text()
        room = int(re.search(r"MemAvailable:\s+(\d+)", info)[1]) * 1024
    return room, room_at, pressure, pressure_at


def memory_said():
    room, room_at, pressure, pressure_at = memory_room()
    return f"{room / 1024**3:.1f} GiB left below {room_at}, memory pressure {pressure:.0f}%" + (f" in {pressure_at}" if pressure_at else "")


# The share processes running, each in a process group of its own with its
# browser, so a stop takes the browser too and none outlives the run.
PARTS = []


def kill(proc):
    """Stops a share and its browser, waiting a bounded time."""
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    try:
        proc.wait(timeout=30)
    except subprocess.TimeoutExpired:
        pass


def stop_parts():
    for proc in PARTS:
        if proc.poll() is None:
            kill(proc)
    PARTS.clear()


# When the run must have ended: set as it starts, from --limit.
DEADLINE = float("inf")


def measure_parts(work, specs, at_once, first=0):
    """Runs each spec in a process of its own, `at_once` side by side: the
    results by name and column, what could not be driven, and what stalled
    (a share that finished nothing for STALL seconds, or ran past its own or the
    run's limit, is stopped)."""
    results, not_driven, stalled = {}, {}, {}
    running, waiting = [], list(enumerate(specs, first))
    tight_since = None
    while waiting or running:
        while waiting and len(running) < at_once:
            # A browser started into a cgroup at its cap is throttled until it
            # stalls: wait for room, and fail rather than time the throttling.
            room, _, pressure, _ = memory_room()
            if room < SHARE_MEMORY or pressure > PRESSURE:
                if tight_since is None:
                    tight_since = time.time()
                    print(f"-- waiting for memory to start a share: {memory_said()}", file=sys.stderr, flush=True)
                if not running and (time.time() - tight_since > ROOM_WAIT or time.time() >= DEADLINE):
                    fail(f"no room to start a browser for {round(time.time() - tight_since)} s: {memory_said()}")
                break
            tight_since = None
            k, spec = waiting.pop(0)
            f = work / f"part-{k}.json"
            at = work / f"part-{k}.at"
            at.write_text("starting")
            f.write_text(json.dumps({**spec, "at": str(at), "out": str(f) + ".out"}))
            proc = subprocess.Popen([sys.executable, __file__, OPT.site, "--part", str(f)], start_new_session=True)
            PARTS.append(proc)
            share_limit = SHARE_BASE + SHARE_EACH * spec["runs"] * (len(spec["items"]) + len(spec["gestures"]))
            running.append((f, spec, proc, at, time.time() + share_limit))
        time.sleep(2)
        for f, spec, proc, at, until in list(running):
            col = f"{spec['width']} cpu/{spec['rate']}"
            if proc.poll() is None:
                now = time.time()
                idle = now - at.stat().st_mtime
                if idle >= STALL:
                    why = f"finished nothing for {round(idle)} s (last finished: {at.read_text()}); stopped"
                elif now >= until:
                    why = f"past its own limit (last finished: {at.read_text()}); stopped"
                elif now >= DEADLINE:
                    why = f"still running at the run's {OPT.limit} s limit (last finished: {at.read_text()}); stopped"
                else:
                    continue
                kill(proc)
                stage = Path(str(at) + ".stage")
                doing = f"; at {stage.read_text()}" if stage.exists() else ""
                stalled[f"{col} share {f.stem}"] = f"{why}{doing} ({memory_said()})"
            running.remove((f, spec, proc, at, until))
            PARTS.remove(proc)
            out = Path(str(f) + ".out")
            if proc.returncode != 0 and f"{col} share {f.stem}" not in stalled:
                stalled[f"{col} share {f.stem}"] = f"exited {proc.returncode} (last finished: {at.read_text()})"
            if not out.exists():
                continue
            # A share stopped or gone keeps what it found before.
            got = json.loads(out.read_text())
            for name, cols in got["results"].items():
                results.setdefault(name, {}).update(cols)
            not_driven.update(got["not_driven"])
    return results, not_driven, stalled


def fail(why):
    print(f"FAIL {why}", flush=True)
    sys.exit(1)


def main():
    if OPT.part:
        spec = json.loads(Path(OPT.part).read_text())
        spec.setdefault("out", OPT.part + ".out")
        measure_part(spec)
        return
    # A run stopped from outside (a release's timeout) takes its shares and
    # their browsers with it.
    atexit.register(stop_parts)
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    signal.signal(signal.SIGHUP, lambda *_: sys.exit(129))
    widths = [OPT.only] if OPT.only else ["desk", "phone"]
    rates = [1, 4] if OPT.slow else [1]
    results, skipped, failed_to_drive = {}, {}, {}
    started = time.time()
    global DEADLINE
    DEADLINE = started + OPT.limit
    work = Path(tempfile.mkdtemp(prefix="ui-latency-"))
    specs, clicks_of, lost = [], {}, 0
    with sync_playwright() as p:
        browser = p.chromium.launch(args=["--enable-gpu-rasterization", "--ignore-gpu-blocklist"])
        site = workspaces.serve()
        for width in widths:
            finder = Run(browser, site, width, 1)
            found = crawl(finder, OPT.depth)
            finder.context.close()
            lost += len(finder.lost)
            print(f"-- {width}: {len(found)} interactions found by clicking, {finder.reloads} loads, {len(finder.lost)} screens not opened, {round(time.time() - started)} s", file=sys.stderr, flush=True)
            clicks = []
            for item in found:
                if "skip" in item:
                    skipped[f"{width}: {item['name']}"] = item["skip"]
                else:
                    clicks.append(item)
            names = [g[0] for g in gestures(width == "phone")]
            # Each processor speed's share split across the jobs; a share keeps
            # the clicks of one screen together, so it reloads less.
            clicks.sort(key=lambda i: i["path"])
            clicks_of[width] = clicks
            for rate in rates:
                for k in range(OPT.jobs):
                    specs.append({"width": width, "rate": rate, "runs": OPT.runs, "items": clicks[len(clicks) * k // OPT.jobs: len(clicks) * (k + 1) // OPT.jobs],
                                  "gestures": names[k::OPT.jobs]})
        browser.close()
    found_all = sum(len(c) for c in clicks_of.values()) + len(skipped)
    if not OPT.only and OPT.depth >= 3 and found_all < FLOOR:
        why = f"{lost} screens it could not open, named above" if lost else "it opened every screen it tried, so the stand-in host or the page drew fewer controls"
        fail(f"the crawl found {found_all} interactions, fewer than the {FLOOR} a page that loaded has; {why}")
    measured, not_driven, stalled = measure_parts(work, specs, OPT.jobs * len(rates))
    for name, cols in measured.items():
        results.setdefault(name, {}).update(cols)
    failed_to_drive.update(not_driven)
    print(f"-- measured, {round(time.time() - started)} s", file=sys.stderr, flush=True)

    # A miss is timed again before it fails: CONFIRM runs, one browser alone,
    # so a moment the machine was busy elsewhere (a release runs beside other
    # work) is not read as the page's. A miss that is the page's misses again.
    missed = [(n, c) for n, cols in results.items() for c, s in cols.items() if c.endswith("cpu/1") and s["median"] > LIMIT]
    if missed:
        again = []
        for n, c in missed:
            width = c.split()[0]
            item = next((i for i in clicks_of[width] if i["name"] == n), None)
            again.append({"width": width, "rate": 1, "runs": CONFIRM, "items": [item] if item else [], "gestures": [] if item else [n]})
        confirmed, _, stalled_again = measure_parts(work, again, 1, first=len(specs))
        stalled.update(stalled_again)
        for name, cols in confirmed.items():
            for c, s in cols.items():
                s["confirmed"] = True
                results[name][c] = s
        print(f"-- {len(missed)} over {LIMIT} ms timed again, {round(time.time() - started)} s", file=sys.stderr, flush=True)

    over = sorted(((n, c, s) for n, cols in results.items() for c, s in cols.items() if c.endswith("cpu/1") and s["median"] > LIMIT), key=lambda x: -x[2]["median"])
    report = {"results": results, "skipped": skipped, "not_driven": failed_to_drive}
    if OPT.inventory:
        report["inventory"] = inventory(OPT.inventory, list(results))
    if OPT.table:
        write_table(Path(OPT.table), report, widths, rates)
        Path(OPT.table).with_suffix(".json").write_text(json.dumps(report, indent=1))
    for n, c, s in over:
        print(f"FAIL {n} ({c}): median {s['median']} ms, worst {s['worst']} ms, over {LIMIT} ms")
    for n, why in sorted(failed_to_drive.items()):
        print(f"not driven: {n}: {why}")
    # A stalled share's interactions were never timed: no pass without them.
    for n, why in sorted(stalled.items()):
        print(f"FAIL stalled: {n}: {why}")
    if not results:
        print("FAIL nothing was measured")
        sys.exit(1)
    if over or stalled:
        sys.exit(1)
    print(f"ok   {len(results)} interactions, none over {LIMIT} ms unthrottled")


def write_table(path, report, widths, rates):
    cols = [f"{w} cpu/{r}" for w in widths for r in rates]
    out = [f"# UI latency: input to the frame after the page settles ({time.strftime('%Y-%m-%d')})", "",
           "Written by `nix/check_ui_latency.py --slow --table`. Each cell: median / worst ms over the runs, from the",
           "input event to the frame after the page last changed for it (the end of any animation it started included).",
           f"`!!` over the {LIMIT} ms limit (an unthrottled median over it fails the check), `!` over the {TARGET} ms target.",
           "`first` is the median to the first frame that changed, unthrottled, the slower width: the gap between it and the",
           "cell is animation or later work. `inp` is the browser's own interaction delay (Event Timing, at least 16 ms when",
           "reported). `host` is the longest call to the stand-in host the interaction started; the stand-in answers at once.",
           "`-` not reached at that width; `(no change)` nothing on screen changed. A name is the path of clicks to it.", "",
           "| interaction | " + " | ".join(cols) + " | first | inp | host ms | host calls |",
           "|---|" + "---|" * (len(cols) + 4)]

    def cell(s):
        if not s:
            return "-"
        mark = "!!" if s["median"] > LIMIT else "!" if s["median"] > TARGET else ""
        return f"{mark}{s['median']} / {s['worst']}" + (" cap" if s["capped"] else "") + (" (no change)" if s.get("still") else "")

    def worst(got):
        return max((s["median"] for s in got.values()), default=0)

    for name, got in sorted(report["results"].items(), key=lambda kv: (-worst(kv[1]), kv[0])):
        ones = [got.get(f"{w} cpu/1") for w in widths]
        first = max((s.get("first") or 0 for s in ones if s), default=0)
        inp = max((s["inp"] for s in ones if s), default=0)
        host = max((s["host"] or 0 for s in got.values()), default=0)
        calls = sorted({u for s in got.values() for u in s["calls"]})
        label = " ".join(name.split()).replace("|", "/")
        out.append(f"| {label} | " + " | ".join(cell(got.get(c)) for c in cols) + f" | {first} | {inp} | {host or ''} | {' '.join(calls)} |")
    out += ["", "## Not driven", ""] + [f"- {' '.join(k.split())}: {v}" for k, v in sorted({**report["skipped"], **report["not_driven"]}.items())]
    if "inventory" in report:
        missed = [r for r in report["inventory"] if not r[3]]
        out += ["", f"## Source handlers whose written label no measured interaction carries ({len(missed)} of {len(report['inventory'])})", "",
                "Matched by the aria-label, title or button text written within six lines of the handler. Many carry a label",
                "made at run time, or answer a state the stand-in host never shows, so are reached under another name or not",
                "at all; read this list as where the crawl may have gaps, not as a list of gaps.", ""]
        out += [f"- {r[0]} {r[1]} {r[2]}" for r in missed]
    path.write_text("\n".join(out) + "\n")


if __name__ == "__main__":
    main()
