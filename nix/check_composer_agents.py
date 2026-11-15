#!/usr/bin/env python3
"""The agents running, at the composer (agents.tsx): with workers at work the
composer's top right edge carries a row of opaque, overlapping circles
centred on its whole outline, below any popover over them, one per running
worker, each with its model's mark, a "+N" past four; one
waiting on the person wears the amber ring. A tap rises the tree of them out
of the composer, sub-workers nested; a tap elsewhere folds it; a row opens
that worker in a tile of its own on a wide screen (beside a thread already
open, not in its place) and in a tab of its own on a phone. With nothing at
work there are no circles. Runs the built site against a stand-in host in
Chromium at 390 and 1440, in both themes, with 0, 1, 3 and 7 workers, and
leaves screenshots folded and risen. Needs Playwright and a browser, so it
stays out of `nix flake check`.
Usage: check_composer_agents.py SITE_DIR SHOTS_DIR
"""
import functools
import math
import http.server
import sys
import threading
import time
from pathlib import Path
from urllib.parse import urlparse

from playwright.sync_api import sync_playwright

SITE, SHOTS = Path(sys.argv[1]), Path(sys.argv[2])
failures = []
total = 0


def check(what, ok, detail=""):
    global total
    total += 1
    print(("ok   " if ok else "FAIL ") + what + ("" if ok or not detail else f": {detail}"), flush=True)
    if not ok:
        failures.append(what)


# The stand-in host of real size answers the rest of the API (measure_ui_performance.py).
sys.path.insert(0, str(Path(__file__).resolve().parent))
_argv, sys.argv = sys.argv, [sys.argv[0], str(SITE), "serve"]
import measure_ui_performance as stand_in  # noqa: E402
sys.argv = _argv

NOW = int(time.time())
# Seven workers on as many models; the third waits on the person, the fourth
# is the second's sub-worker. A run shows the first `Host.count` of them,
# and two asleep that never show.
PICKS = [
    # The alias and the bare id both stand for the offer named Opus 5.5.
    ("claude", "opus"), ("codex", "gpt-6.1-sol"), ("omp", "deepseek-v4.1-flash"), ("claude", "opus-5-5"),
    ("opencode", "kimi-k3"), ("grok", "grok-5"), ("muse", "muse-wonder-full-1.0"),
]
NAMES = ["Reed", "Wren", "Sable", "Tarn", "Ibis", "Heron", "Plover"]


def threads(count):
    out = []
    # Past the seven, more of the same models under numbered names.
    names = NAMES + [f"Agent{k}" for k in range(len(NAMES), count)]
    picks = [PICKS[k % len(PICKS)] for k in range(len(names))]
    for k, (name, (harness, model)) in enumerate(zip(names, picks)):
        if k >= count:
            break
        t = {"id": f"w{k}", "name": name, "brief": f"Check the {name} work", "title": f"{name} batch check", "gist": "Comparing quotes",
             "summary": "Comparing three quotes against the last run", "state": "waiting" if k == 2 else "working", "need": "answer" if k == 2 else None,
             "at": NOW - 60 * (k + 1), "read": False, "waiting_on": [], "pick": {"harness": harness, "model": model}}
        if k == 3 and count > 3:
            t["head"] = "w1"
        out.append(t)
    out += [{"id": f"s{k}", "name": f"Sleeper{k}", "brief": "Old work", "gist": "Done", "state": "sleeping", "at": NOW - 9000, "read": True, "waiting_on": []} for k in range(2)]
    # Neither at work nor asking: one done with its report unread, one
    # napping, one archived. None has a circle or a row.
    out += [{"id": "d0", "name": "Finisher", "brief": "Old work", "gist": "Done", "state": "waiting", "need": "review", "at": NOW - 120, "read": False, "waiting_on": []},
            {"id": "n0", "name": "Napper", "brief": "Old work", "gist": "Resting", "state": "napping", "at": NOW - 120, "read": True, "waiting_on": []},
            {"id": "a0", "name": "Keeper", "brief": "Old work", "gist": "Done", "state": "archived", "at": NOW - 9000, "read": True, "waiting_on": []}]
    return out


# Workers that are not running, by id: done, napping, asleep, archived.
IDLE = ("d0", "n0", "s0", "a0")


# A wide screen: the chat beside a thread already open, so a worker opened
# from the circles must take a tile of its own rather than that one's place.
WIDE_LAYOUT = {"spaces": [{"id": "main", "name": "Main", "focus": "c", "full": False, "root": {
    "t": "split", "id": "s", "dir": "row", "at": 0.6,
    "a": {"t": "leaf", "id": "c", "pane": {"kind": "chat"}}, "b": {"t": "leaf", "id": "t", "pane": {"kind": "thread", "arg": "s0"}}}}], "on": "main"}
# Kept from before the Workers place went: its item is dropped as the bar is read.
PHONE_LAYOUT = {"spaces": [{"id": "main", "name": "Main", "items": [{"kind": "chat"}, {"kind": "workers"}]}], "on": "main"}


class Host(stand_in.Host):
    count = 0

    def do_GET(self):
        path = urlparse(self.path).path
        if path.endswith("/api/work"):
            return self.json({"threads": threads(Host.count)})
        if path.endswith("/api/layout") or path.endswith("/api/phone-layout"):
            if "since=" in self.path:
                time.sleep(5)
            layout = PHONE_LAYOUT if path.endswith("/api/phone-layout") else WIDE_LAYOUT
            try:
                return self.json({"version": 1, "layout": layout, "by": None, "undo": False})
            except (BrokenPipeError, ConnectionResetError):
                return
        return super().do_GET()


CHAT = "[data-surface=chat]"


def run(browser, site, width, theme, count):
    Host.count = count
    phone = width < 640
    size = {"width": width, "height": 844 if phone else 900}
    context = browser.new_context(viewport=size, service_workers="block", reduced_motion="reduce", is_mobile=phone, has_touch=phone)
    # 60 s, as check_chat_steady allows: the release runs these while workers' builds
    # load the host, and a first page load there can pass 15 s.
    context.set_default_timeout(60000)
    if theme == "dark":
        context.add_init_script("try { localStorage.setItem('theme', 'dark') } catch {}")
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(site + "/")
    page.wait_for_selector(f"{CHAT} textarea")
    page.wait_for_timeout(1200)
    tag = f"{width}-{theme}-{count}"
    agents = page.locator(f"{CHAT} [data-agents]")
    if phone:
        check(f"{tag}: a kept bar's Workers item is gone", page.locator("[role=tab][data-item=workers]").count() == 0)
    if count == 0:
        check(f"{tag}: nothing at work, no circles", agents.count() == 0)
        # Every worker stays in reach with none at work: the phone's plus, or
        # anything asking the main chat's tree to show them all.
        if phone:
            page.locator("[data-action=add]").click()
            page.locator("[role=menu] [data-add=workers]").click()
        else:
            page.evaluate("() => dispatchEvent(new Event('libri:agents'))")
        page.wait_for_selector(f"{CHAT} [data-agents-all]")
        everyone = page.locator(f"{CHAT} [data-agents-all]").inner_text()
        check(f"{tag}: with none at work, every worker is still in reach, asleep too", "Sleeper0" in everyone and "Watches" in everyone and "Archived" in everyone, everyone[:200])
        check(f"{tag}: and a worker can be started from there", page.locator(f"{CHAT} [data-agents-start]").count() == 1)
        page.screenshot(path=SHOTS / f"all-{tag}.png")
        check(f"{tag}: no page errors", not errors, "; ".join(errors))
        context.close()
        return
    check(f"{tag}: the circles show", agents.count() == 1)
    circles = page.locator(f"{CHAT} [data-agent-circle]")
    idle = [i for i in IDLE if page.locator(f"{CHAT} [data-agent-circle={i}]").count()]
    check(f"{tag}: no circle for one done, napping, asleep or archived", not idle, str(idle))
    # The row has room for as many 22px circles, each 15px on from the last,
    # as fit between the same inset from either end of the composer.
    room = page.evaluate("""() => {
      const box = document.querySelector('[data-surface=chat] [data-agents]');
      const root = box.parentElement.getBoundingClientRect();
      const inset = root.right - box.getBoundingClientRect().right;
      return Math.floor((root.width - 2 * inset - 22) / 15) + 1;
    }""")
    want = count if count <= room else room - 1
    check(f"{tag}: {want} circle(s) of room for {room}", circles.count() == want, str(circles.count()))
    more = page.locator(f"{CHAT} [data-agents-more]")
    if count > room:
        check(f"{tag}: only past the room does the last become +{count - want}", more.count() == 1 and more.inner_text().strip() == f"+{count - want}", more.inner_text() if more.count() else "none")
        ends = page.evaluate("""() => {
          const box = document.querySelector('[data-surface=chat] [data-agents]');
          const root = box.parentElement.getBoundingClientRect();
          const row = document.querySelector('[data-surface=chat] [data-agents-toggle]').getBoundingClientRect();
          return { left: row.left - root.left, right: root.right - row.right };
        }""")
        check(f"{tag}: the full row stops as far from the left end as from the right", abs(ends["left"] - ends["right"]) <= 15, str(ends))
    else:
        check(f"{tag}: no +N while all fit", more.count() == 0)
    if count >= 3:
        check(f"{tag}: the one waiting on the person wears the amber ring", page.locator(f"{CHAT} [data-agent-circle=w2][data-act=person]").count() == 1)
    # The row sits on the composer's top edge, at its right, half above it.
    geo = page.evaluate("""() => {
      const root = document.querySelector('[data-surface=chat] [data-agents]').parentElement.getBoundingClientRect();
      const row = document.querySelector('[data-surface=chat] [data-agents-toggle]').getBoundingClientRect();
      return { above: root.top - row.top, below: row.bottom - root.top, right: root.right - row.right };
    }""")
    check(f"{tag}: the circles sit centred on the composer's top edge at its right", 9 <= geo["above"] <= 13 and 9 <= geo["below"] <= 13 and 20 < geo["right"] < 48, str(geo))
    # The outline stays whole, and the circles are opaque, so they sit on
    # the line and hide it where they cover it.
    look = page.evaluate("""() => {
      const root = document.querySelector('[data-surface=chat] [data-agents]').parentElement;
      const alpha = (c) => { const m = c.match(/rgba?\\(([^)]+)\\)/); const p = m ? m[1].split(/[ ,/]+/).filter(Boolean) : []; return p.length > 3 ? +p[3] : 1; };
      const fills = [...document.querySelectorAll('[data-surface=chat] [data-agent-circle], [data-surface=chat] [data-agents-more]')].map((c) => alpha(getComputedStyle(c).backgroundColor));
      return { line: alpha(getComputedStyle(root).borderTopColor), width: getComputedStyle(root).borderTopWidth, fills };
    }""")
    check(f"{tag}: the composer's outline is whole", look["line"] > 0 and look["width"] == "1px", str(look))
    check(f"{tag}: every circle is opaque", all(f == 1 for f in look["fills"]), str(look["fills"]))
    # A popover over the circles covers them (the layer order in index.css).
    if not phone:
        page.locator(f"{CHAT} button[aria-label*=' use: ']").first.hover()
        tip = page.locator(f"{CHAT} [role=tooltip]")
        tip.wait_for()
        over = page.evaluate("""() => {
          const tip = document.querySelector('[data-surface=chat] [role=tooltip]').getBoundingClientRect();
          const out = [];
          for (const c of document.querySelectorAll('[data-surface=chat] [data-agent-circle], [data-surface=chat] [data-agents-more]')) {
            const r = c.getBoundingClientRect();
            const x = Math.max(r.left, tip.left) + 2, y = Math.max(r.top, tip.top) + 2;
            if (x < Math.min(r.right, tip.right) && y < Math.min(r.bottom, tip.bottom)) out.push(!!document.elementFromPoint(x, y)?.closest('[role=tooltip]'));
          }
          return out;
        }""")
        check(f"{tag}: the usage card covers the circles it overlaps", all(over), f"{len(over)} overlapping, covered {over}")
        page.screenshot(path=SHOTS / f"card-{tag}.png")
        page.mouse.move(5, 5)
        tip.wait_for(state="detached")
    page.screenshot(path=SHOTS / f"folded-{tag}.png")
    started = time.monotonic()
    page.locator(f"{CHAT} [data-agents-toggle]").click()
    page.wait_for_selector(f"{CHAT} [data-agents-tree]")
    took = (time.monotonic() - started) * 1000
    check(f"{tag}: the tree rises within 400 ms", took < 400, f"{took:.0f} ms")
    check(f"{tag}: its foot offers every worker and starting one", page.locator(f"{CHAT} [data-agents-show-all]").count() == 1 and page.locator(f"{CHAT} [data-agents-start]").count() == 1)
    rows = page.locator(f"{CHAT} [data-agent-row]")
    check(f"{tag}: a row per running worker", rows.count() == count, str(rows.count()))
    idle = [i for i in IDLE if page.locator(f"{CHAT} [data-agent-row={i}]").count()]
    check(f"{tag}: no row for one done, napping, asleep or archived", not idle, str(idle))
    named = page.locator(f"{CHAT} [data-agent-row=w0]").inner_text()
    check(f"{tag}: a model stored as its alias shows by its picker name", "Opus 5.5" in named and "· opus" not in named, named)
    if count > 3:
        check(f"{tag}: the sub-worker nests under its head", page.locator(f"{CHAT} [data-agent-row=w3][aria-level='2']").count() == 1)
        named = page.locator(f"{CHAT} [data-agent-row=w3]").inner_text()
        check(f"{tag}: a model stored as its bare id shows by its picker name", "Opus 5.5" in named, named)
    tree = page.locator(f"{CHAT} [data-agents-tree]").bounding_box()
    composer = page.locator(f"{CHAT} [data-agents]").bounding_box()
    check(f"{tag}: the tree rises above the composer", tree["y"] + tree["height"] <= composer["y"] + 1, f"tree foot {tree['y'] + tree['height']:.0f}, circles top {composer['y']:.0f}")
    check(f"{tag}: the tree stays on the screen", tree["x"] >= 8 and tree["x"] + tree["width"] <= size["width"] - 8, str(tree))
    page.screenshot(path=SHOTS / f"risen-{tag}.png")
    # Show all: every worker, the asleep too, with the watches and the
    # archived, where the Workers place held them; Running goes back.
    page.locator(f"{CHAT} [data-agents-show-all]").click()
    page.wait_for_selector(f"{CHAT} [data-agents-all]")
    everyone = page.locator(f"{CHAT} [data-agents-all]").inner_text()
    check(f"{tag}: Show all lists every worker, the asleep too", "Sleeper0" in everyone and "Watches" in everyone, everyone[:200])
    page.locator(f"{CHAT} [data-agents-running]").click()
    page.wait_for_selector(f"{CHAT} [data-agent-row]")
    # A tap elsewhere folds it.
    page.mouse.click(10, size["height"] // 3)
    page.wait_for_timeout(150)
    check(f"{tag}: a tap elsewhere folds the tree", page.locator(f"{CHAT} [data-agents-tree]").count() == 0)
    # A row opens that worker where it can stay.
    page.locator(f"{CHAT} [data-agents-toggle]").click()
    page.locator(f"{CHAT} [data-agent-row=w0] button").click()
    if phone:
        page.wait_for_selector("[role=tablist] button[data-open=thread]")
        check(f"{tag}: the worker opens in a tab of its own", page.locator("[role=tablist] button[data-open=thread]").count() == 1)
        page.wait_for_selector("[data-thread=w0]")
    else:
        page.wait_for_selector("[data-thread=w0]")
        check(f"{tag}: the worker opens in a tile of its own, the thread already open kept", page.locator("[data-thread=s0]").count() >= 1 and page.locator("[data-thread=w0]").count() >= 1)
    page.wait_for_timeout(300)
    page.screenshot(path=SHOTS / f"opened-{tag}.png")
    check(f"{tag}: no page errors", not errors, "; ".join(errors))
    context.close()


def motion(browser, site):
    """Unreduced motion: the bar's own circles rise into a column, hidden on
    the bar while out; each runs one rail (right along the bar, round a
    corner, up), settled in its slot within 400 ms, one above another at
    the same x, its words to its left, on a panel, the talk dimmed;
    folded, they travel back and the column goes."""
    Host.count = 3
    context = browser.new_context(viewport={"width": 1440, "height": 900}, service_workers="block")
    context.set_default_timeout(60000)
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(site + "/")
    page.wait_for_selector(f"{CHAT} [data-agents-toggle]")
    page.wait_for_timeout(1200)
    # Where each dot is every frame, from the tap until it settles.
    page.evaluate("""() => {
      window.__dots = [];
      const t0 = performance.now();
      const watch = () => {
        const dots = [...document.querySelectorAll('[data-surface=chat] [data-dot]')].map((d) => { const r = d.getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; });
        window.__dots.push([performance.now() - t0, dots]);
        if (performance.now() - t0 < 700) requestAnimationFrame(watch);
      };
      document.querySelector('[data-surface=chat] [data-agents-toggle]').addEventListener('click', () => requestAnimationFrame(watch), { once: true });
    }""")
    page.locator(f"{CHAT} [data-agents-toggle]").click()
    page.wait_for_timeout(800)
    frames = page.evaluate("window.__dots")
    final = frames[-1][1]
    settled = next((t for t, d in frames if d and all(abs(a[0] - b[0]) < 0.5 and abs(a[1] - b[1]) < 0.5 for a, b in zip(d, final))), None)
    first = next((d for _, d in frames if d), [])
    check("motion: the dots set off from the row", bool(first) and max(abs(a[1] - b[1]) for a, b in zip(first, final)) > 20, str(first[:2]))
    check("motion: each dot settles in its slot within 400 ms", settled is not None and settled < 400, f"{settled} ms")
    check("motion: the bar's circles are the ones that rise (hidden there while out)", page.evaluate("[...document.querySelectorAll('[data-surface=chat] [data-agents-toggle] > *')].every((c) => getComputedStyle(c).opacity === '0')"))
    check("motion: a column, one above another", len({round(x) for x, _ in final if True}) <= 2 and all(final[k][1] < final[k + 1][1] for k in range(len(final) - 1)), str(final))
    words = page.evaluate("""() => [...document.querySelectorAll('[data-surface=chat] [data-agent-row] button')].map((b) => {
      const dot = b.querySelector('[data-dot]').getBoundingClientRect();
      const text = b.firstElementChild.getBoundingClientRect();
      return text.right <= dot.left + 1;
    })""")
    check("motion: each agent's words stand to the left of its dot", words and all(words), str(words))
    check("motion: on a solid panel, so the talk behind can't show through", page.evaluate("getComputedStyle(document.querySelector('[data-surface=chat] [data-agents-panel]')).backgroundColor") not in ("rgba(0, 0, 0, 0)", "transparent"))
    check("motion: the talk behind is dimmed", page.locator(f"{CHAT} [data-agents-scrim]").count() == 1)
    page.screenshot(path=SHOTS / "motion-risen.png")
    page.mouse.click(400, 300)
    page.wait_for_timeout(100)
    check("motion: folding, they are still travelling back at 100 ms", page.locator(f"{CHAT} [data-agents-tree]").count() == 1)
    page.wait_for_timeout(300)
    check("motion: and the column is gone by 400 ms", page.locator(f"{CHAT} [data-agents-tree]").count() == 0)
    check("motion: home, the bar's circles show again", page.evaluate("[...document.querySelectorAll('[data-surface=chat] [data-agents-toggle] > *')].every((c) => getComputedStyle(c).opacity === '1')"))
    rail(page)
    # A row chosen opens its worker at once: the column goes with the click,
    # its dots not travelling back first, which would hold the open past 400 ms.
    page.locator(f"{CHAT} [data-agents-toggle]").click()
    page.wait_for_timeout(500)
    gone = page.evaluate("""async (sel) => {
        document.querySelector(sel + ' [data-agent-row] button:not([disabled])').click();
        await new Promise((r) => requestAnimationFrame(r));
        return { tree: !!document.querySelector(sel + ' [data-agents-tree]'),
                 moving: document.getAnimations().filter((a) => a.playState === 'running' && a.effect?.target?.closest?.('[data-agents]')).length };
    }""", CHAT)
    check("motion: a row chosen closes the column on the next frame, nothing travelling back", not gone["tree"] and gone["moving"] == 0, str(gone))
    check("motion: no page errors", not errors, "; ".join(errors))
    context.close()


RAIL_BEND = 20


def on_rail(start, end, p):
    """How far `p` is from the rail between a bar circle and its slot: along
    the bar, a quarter circle of RAIL_BEND, then up (agents.tsx `rail`)."""
    (sx, sy), (ex, ey) = start, end
    h = (ex > sx) - (ex < sx)
    r = min(RAIL_BEND, abs(ex - sx), max(0, sy - ey))
    cx, cy = ex - h * r, sy - r
    x, y = p
    run = math.hypot(x - min(max(x, min(sx, cx)), max(sx, cx)), y - sy)
    rise = math.hypot(x - ex, y - min(max(y, ey), cy))
    turn = abs(math.hypot(x - cx, y - cy) - r) if r and (x - cx) * h >= 0 and y <= sy + 0.5 and y >= cy - 0.5 else float("inf")
    return min(run, rise, turn)


def rail(page):
    """Slowed ten times, frame by frame: each dot leaves exactly on its
    circle and stays on one rail, through a round corner, never jumping;
    folding runs the same rail back to the circle."""
    def record(go):
        # Set going and slowed in one task, so the first sample is the
        # animation's first frame.
        page.evaluate("""async (go) => {
          window.__dots = [];
          window.__seen = [];
          const t0 = performance.now();
          const watch = () => {
            document.getAnimations().forEach((a) => (a.playbackRate = 0.1));
            const dots = [...document.querySelectorAll('[data-surface=chat] [data-dot]')].map((d) => { const r = d.getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; });
            window.__dots.push([performance.now() - t0, dots]);
            // What shows at each dot's centre: a dot, unless something clips it.
            window.__seen.push(dots.every(([x, y]) => document.elementFromPoint(x, y)?.closest('[data-dot]')));
            if (performance.now() - t0 < 4500) requestAnimationFrame(watch);
          };
          document.querySelector(go).dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
          document.querySelector(go).click();
          await Promise.resolve();  // React commits a tap in a microtask
          watch();
        }""", go)
        page.wait_for_timeout(4700)
        record.seen = page.evaluate("window.__seen")
        return [d for _, d in page.evaluate("window.__dots") if d]
    circles = page.evaluate("""() => [...document.querySelectorAll('[data-surface=chat] [data-agent-circle]')].map((c) => { const r = c.getBoundingClientRect(); return [c.dataset.agentCircle, r.left + r.width / 2, r.top + r.height / 2]; })""")
    out = record("[data-surface=chat] [data-agents-toggle]")
    order = page.evaluate("[...document.querySelectorAll('[data-surface=chat] [data-dot]')].map((d) => d.dataset.dot)")
    home = {k: (x, y) for k, x, y in circles}
    starts = [home.get(k) for k in order]
    ends = out[-1]
    check("rail: each dot leaves exactly on its circle", out and all(s and math.dist(s, p) < 0.75 for s, p in zip(starts, out[0])), f"{starts} {out[0] if out else None}")
    check("rail: every dot shows the whole way, nothing clipping it", record.seen and all(record.seen), f"{record.seen.count(False)} frames hidden")
    off = max((on_rail(starts[k], ends[k], d[k]) for d in out for k in range(len(d))), default=99)
    check("rail: every frame of every dot lies on its rail", off < 1.0, f"{off:.2f} px off")
    # Round: each dot is seen inside its corner, off both straight legs.
    cornered = [any(min(abs(d[k][0] - ends[k][0]), abs(d[k][1] - starts[k][1])) > 4 and abs(d[k][0] - ends[k][0]) < RAIL_BEND - 4 and abs(d[k][1] - starts[k][1]) < RAIL_BEND - 4 for d in out) for k in range(len(ends))]
    check("rail: each dot turns a round corner", all(cornered), str(cornered))
    steps = [math.dist(a[k], b[k]) for a, b in zip(out, out[1:]) for k in range(len(a))]
    check("rail: no jump between frames (slowed ten times)", steps and max(steps) < 4, f"{max(steps):.2f} px" if steps else "")
    # One speed between the ends, so no pause at the corner: the middle
    # half of each dot's frames move it alike.
    even = []
    for k in range(len(ends)):
        moving = [math.dist(a[k], b[k]) for a, b in zip(out, out[1:]) if math.dist(a[k], b[k]) > 0.01]
        core = moving[len(moving) // 4: 3 * len(moving) // 4]
        even.append(round(max(core) / max(min(core), 1e-3), 2) if core else 99)
    check("rail: each dot keeps one speed through the corner", all(e < 1.5 for e in even), str(even))
    near = min((math.dist(d[j], d[k]) for d in out for j in range(len(d)) for k in range(j + 1, len(d))), default=0)
    check("rail: they travel as a train, none running into another", near > 10, f"{near:.1f} px")
    back = record("[data-surface=chat] [data-agents-scrim]")
    back = [d for d in back if len(d) == len(ends)]
    off = max((on_rail(starts[k], ends[k], d[k]) for d in back for k in range(len(d))), default=99)
    check("rail: folding runs the same rail back", back and off < 1.0, f"{off:.2f} px off")
    check("rail: and every dot reaches its circle", back and all(math.dist(s, p) < 0.75 for s, p in zip(starts, back[-1])), f"{back[-1] if back else None}")
    steps = [math.dist(a[k], b[k]) for a, b in zip(back, back[1:]) for k in range(len(a))]
    check("rail: no jump folding", steps and max(steps) < 4, f"{max(steps):.2f} px" if steps else "")


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Host, directory=str(SITE)))
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for width in (390, 1440):
            for theme in ("light", "dark"):
                for count in (0, 1, 3, 7, 60):
                    run(browser, site, width, theme, count)
        motion(browser, site)
        browser.close()
    print(f"{total - len(failures)}/{total} passed")
    sys.exit(1 if failures else 0)


main()
