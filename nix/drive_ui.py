#!/usr/bin/env python3
"""Libri's UI driver: scenario scripts that drive the page as a person does
and measure it, for the speed-and-stability hill climb. Each run starts a
throwaway Libri (throwaway_libri.py: the real daemon and hippo in a scratch
root, the stand-in Claude answering), seeds a long chat into its memory, and
in Chromium at a desktop and a phone size:

  open      first paint, time until usable, page weight, DOM and heap
  type      key-to-paint delay while typing in the composer
  stream    a streamed reply: time to its first words and to its end, frames
  scroll    the long chat scrolled up and back, then the jump to the foot
  surfaces  Temp, Terminal, Workers and the Library opened and closed
  page      a Library page published by a turn, then opened by its link
  drop      the network lost for a while, and the time to hear the host again
  restart   the daemon restarted under the page, the same
  session   many turns and openings: DOM and JS heap growth per round

Golden screenshots are taken at rest (open, each surface, the page) and
compared with those kept in nix/ui_goldens/LAYOUT/ within a tolerance; a
difference past it is written beside the run as a diff image. Console errors
are counted per scenario. The run is written as JSON and a scorecard printed;
the same seed seeds the same chat and the same replies, so runs compare.

Works with either layout: the columns (the default) and the tiled one
(--layout tiles, with --site serving a build of that branch's page). The
page carries data-open / data-surface hooks for what the driver opens, with
accessible names as the fallback.

Never touches a running Libri or its memory: the throwaway's strays() check
fails a run that leaves its mark outside its scratch root.

Usage:
  drive_ui.py LIBRI_DIR [--site DIR] [--layout columns|tiles] [--only a,b]
              [--sizes desktop,phone] [--messages N] [--seed N] [--repeat N]
              [--rounds N] [--out DIR] [--update-goldens]
  drive_ui.py --compare BEFORE.json AFTER.json
LIBRI_DIR holds bin/libri, bin/hippo and site/ (the flake's `libri`).
"""

import argparse
import json
import os
import random
import socket
import statistics
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from throwaway_libri import Throwaway, until  # noqa: E402

SCENARIOS = ["open", "type", "stream", "scroll", "surfaces", "page", "workspaces", "layout_sync", "settings", "reload", "terminal_keys", "code_contrast", "worker_action", "drop", "restart", "session"]
SIZES = {"desktop": {"viewport": {"width": 1440, "height": 900}, "cpu": 1},
         "phone": {"viewport": {"width": 390, "height": 844}, "cpu": 4, "is_mobile": True, "has_touch": True}}
SURFACES = ["temp", "terminal", "workers", "library"]
# Accessible names that open a surface, for a page without the data-open hook.
OPENERS = {"temp": ["Temp chat, not remembered", "Temp"], "terminal": ["Terminal"],
           "workers": ["Unfold the workers column", "Workers", "Tasks", "Work"], "library": ["Library"]}
# And that show it: the terminal's region, the workers' column or tile.
SHOWN = {"temp": ["[data-surface=temp]"], "terminal": ["[data-surface=terminal]", "section[aria-label=Terminal]"],
         "workers": ["[data-surface=workers]", "[aria-label=Workers][data-tile]"],
         "library": ["[data-surface=library]", "[aria-label=Library][data-tile]"]}
# What differs run to run in a picture that is otherwise the same: clocks,
# the terminal's shell, the memory's state line.
VOLATILE = ["time", "[data-volatile]", ".xterm-screen", "[aria-label=Working]"]
FRAME = 1000 / 60
TOLERANCE = 0.004  # share of pixels that may differ past CHANNEL
CHANNEL = 24

RECORD = """() => {
  const r = window.__rec = { frames: [], loaf: [], on: true };
  const tick = (t) => { r.frames.push(t); if (r.on) requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
  r.obs = new PerformanceObserver((l) => { for (const e of l.getEntries()) r.loaf.push(e.duration); });
  r.obs.observe({ type: 'long-animation-frame' });
}"""
STOP = """() => {
  const r = window.__rec; r.on = false; r.obs.disconnect();
  return { gaps: r.frames.slice(1).map((t, i) => t - r.frames[i]), loaf: r.loaf };
}"""
FIRST_WORDS = """(n) => { const t = document.querySelectorAll('[data-said=talk]');
  return t.length > n && t[t.length - 1].innerText.trim().length > 0; }"""
# Each opening of the page's event stream, by the page's clock; and the means
# to cut them, as a network that drops leaves a stream the browser gave up on.
STREAMS = """(() => { const ES = window.EventSource; window.__opens = []; window.__streams = [];
  window.EventSource = class extends ES { constructor(...a) { super(...a); window.__streams.push(this);
    this.addEventListener('open', () => window.__opens.push(performance.now())); } };
  window.__cut = () => { for (const s of window.__streams) if (s.readyState !== 2) { s.close(); s.dispatchEvent(new Event('error')); } }; })();"""
KEYS = """() => { window.__keys = []; new PerformanceObserver((l) => { for (const e of l.getEntries())
  if (e.name === 'keydown') window.__keys.push(e.duration); })
  .observe({ type: 'event', durationThreshold: 16, buffered: false }); }"""


# The long chat: questions and replies of the length and shape the real ones
# have, from the seed alone.
WORDS = ("the run host build page chat worker summary memory reply list file test step check branch deploy "
         "phone layout column scroll frame stream tile dock workspace commit merge review").split()


def said(rng, n, kind):
    if kind == "user":
        return " ".join(rng.choice(WORDS) for _ in range(rng.randint(6, 30))).capitalize() + "?"
    parts = [" ".join(rng.choice(WORDS) for _ in range(rng.randint(30, 90))).capitalize() + "."]
    if n % 4 == 0:
        parts.append("\n".join(f"- {rng.choice(WORDS)} `{rng.choice(WORDS)}` {rng.choice(WORDS)}" for _ in range(4)))
    if n % 7 == 0:
        parts.append("```rust\nfn main() {\n    let n: u64 = (0..1000).sum();\n    println!(\"{n}\");\n}\n```")
    if n % 11 == 0:
        parts.append("| Host | State |\n|---|---|\n| box | live |\n| den | stable |")
    return "\n\n".join(parts)


def seed_chat(t, count, seed):
    """`count` messages into the throwaway's memory, as Libri posts them."""
    rng = random.Random(seed)
    sock = str(t.root / "hippo/hippo.sock")
    for n in range(count):
        kind = "user" if n % 2 == 0 else "talk"
        with socket.socket(socket.AF_UNIX) as s:
            s.connect(sock)
            s.sendall((json.dumps({"cmd": "post", "args": ["Iris", kind, said(rng, n, kind)]}) + "\n").encode())
            reply = json.loads(s.makefile().readline())
            if not reply.get("ok"):
                raise RuntimeError(f"seeding: {reply}")
    return said(random.Random(seed), 0, "user")


def soon(test, what, wait):
    """As until(), finely enough to time what it waits for."""
    end = time.monotonic() + wait
    while time.monotonic() < end:
        if test():
            return
        time.sleep(0.02)
    raise TimeoutError(what)


def restart(t):
    t.daemon.terminate()
    t.daemon.wait()
    t.daemon = subprocess.Popen([t.libri], env=t.env, stdout=subprocess.DEVNULL, stderr=open(t.root / "libri.log", "a"))
    until(t.up, "libri back", 60)


class Run:
    """One page at one size, and what it measured."""

    def __init__(self, browser, t, size, args, out, shots):
        self.args, self.out, self.shots, self.size, self.t = args, out, shots, size, t
        base = t.base
        spec = dict(SIZES[size])
        self.cpu = spec.pop("cpu")
        self.context = browser.new_context(**spec, service_workers="block", reduced_motion="no-preference")
        self.context.set_default_timeout(30000)
        self.context.add_init_script(STREAMS)
        self.page = self.context.new_page()
        self.errors = []
        self.page.on("console", lambda m: m.type == "error" and self.errors.append(m.text[:300]))
        self.page.on("pageerror", lambda e: self.errors.append(f"pageerror: {str(e)[:300]}"))
        self.url = base + "/" + ("?layout=tiles" if args.layout == "tiles" else "")
        self.cdp = self.context.new_cdp_session(self.page) if browser.browser_type.name == "chromium" else None
        for domain in ("Performance", "Network", "HeapProfiler") if self.cdp else ():
            self.cdp.send(f"{domain}.enable")
        self.turns = 0

    # Measuring.
    def frames(self, act, settle=500):
        """`act` and its settling under the frame recorder and Chromium's counters."""
        self.page.wait_for_timeout(150)
        before = self.counters()
        self.page.evaluate(RECORD)
        self.page.wait_for_timeout(50)
        start = time.monotonic()
        extra = act()
        took = (time.monotonic() - start) * 1000
        self.page.wait_for_timeout(settle)
        got = self.page.evaluate(STOP)
        after = self.counters()
        gaps = sorted(got["gaps"])
        row = {
            "ms": round(took),
            "frames": len(gaps),
            "dropped": sum(max(0, round(g / FRAME) - 1) for g in gaps),
            "worst_frame_ms": round(gaps[-1] if gaps else 0, 1),
            "p95_frame_ms": round(gaps[int(len(gaps) * 0.95)] if gaps else 0, 1),
            "long_frames": len(got["loaf"]),
            "long_frame_ms": round(sum(got["loaf"]), 1),
            "layout_ms": round((after["LayoutDuration"] - before["LayoutDuration"]) * 1000, 1),
            "script_ms": round((after["ScriptDuration"] - before["ScriptDuration"]) * 1000, 1),
        }
        if isinstance(extra, dict):
            row |= extra
        return row

    def counters(self):
        if not self.cdp:
            return {"LayoutDuration": 0, "ScriptDuration": 0}
        return {m["name"]: m["value"] for m in self.cdp.send("Performance.getMetrics")["metrics"]}

    def weight(self):
        """DOM nodes and JS heap after a collection."""
        if not self.cdp:
            return {"dom_nodes": self.page.locator("*").count(), "heap_mb": None}
        self.cdp.send("HeapProfiler.collectGarbage")
        heap = self.cdp.send("Runtime.getHeapUsage")["usedSize"]
        nodes = self.cdp.send("Memory.getDOMCounters")["nodes"]
        return {"dom_nodes": nodes, "heap_mb": round(heap / 2**20, 2)}

    def throttle(self, on):
        if self.cdp:
            self.cdp.send("Emulation.setCPUThrottlingRate", {"rate": self.cpu if on else 1})

    def picture(self, name):
        if not self.shots:
            return
        self.page.wait_for_timeout(400)
        path = self.shots / f"{self.size}-{name}.png"
        mask = [self.page.locator(s) for s in VOLATILE]
        self.page.screenshot(path=path, caret="hide", mask=mask, mask_color="#808080")

    # Driving.
    def composer(self):
        return self.page.locator("textarea[name=input]:visible").first

    def talks(self):
        return self.page.locator("[data-said=talk]")

    def send(self, text):
        box = self.composer()
        box.fill(text)
        box.press("Enter")

    def turn(self, words):
        """A streamed reply of `words` words: when its first words show, and its end."""
        self.turns += 1
        before = self.talks().count()
        self.send(f"STREAM {words} {self.args.seed + self.turns}")
        start = time.monotonic()
        self.page.wait_for_function(FIRST_WORDS, arg=before, polling="raf", timeout=30000)
        first = (time.monotonic() - start) * 1000
        last, steady = -1, 0
        while steady < 4:
            n = len(self.talks().last.inner_text())
            steady = steady + 1 if n == last else 0
            last = n
            self.page.wait_for_timeout(150)
        return {"first_words_ms": round(first), "reply_ms": round((time.monotonic() - start) * 1000 - 600), "reply_chars": last}

    def find(self, selectors):
        for s in selectors:
            loc = self.page.locator(s).filter(visible=True)
            if loc.count():
                return loc.first
        return None

    def opener(self, kind):
        found = self.find([f"[data-open={kind}]"])
        if found:
            return found
        for name in OPENERS[kind]:
            loc = self.page.get_by_role("button", name=name, exact=True).filter(visible=True)
            if loc.count():
                return loc.first
        return None

    def surface(self, kind):
        return self.find(SHOWN[kind])

    def close(self, kind):
        shown = self.surface(kind)
        if not shown:
            return
        shut = shown.locator('[aria-label^="Close"], [aria-label^="Back to the chat"]').filter(visible=True)
        if shut.count():
            shut.first.click()
        else:
            self.page.keyboard.press("Escape")
        self.page.wait_for_timeout(400)

    def forget_layout(self):
        """The tiled layout keeps its arrangement on the host; each step starts
        from the one a new host opens on."""
        (self.t.root / "state/layout.json").unlink(missing_ok=True)

    def home(self):
        """Back to the chat alone, as it opens."""
        try:
            self.page.evaluate("localStorage.removeItem('libri-tiles')")
        except Exception:
            pass
        self.page.goto("about:blank")
        self.forget_layout()
        self.page.goto(self.url)
        self.composer().wait_for()
        self.page.wait_for_timeout(500)

    # Scenarios.
    def open(self):
        self.forget_layout()
        start = time.monotonic()
        self.page.goto(self.url)
        self.composer().wait_for(timeout=60000)
        self.page.locator("[data-said]").last.wait_for(timeout=60000)
        usable = (time.monotonic() - start) * 1000
        t = self.page.evaluate("""() => { const n = performance.getEntriesByType('navigation')[0];
          const p = Object.fromEntries(performance.getEntriesByType('paint').map((e) => [e.name, e.startTime]));
          const res = performance.getEntriesByType('resource');
          const kb = (re) => Math.round(res.filter((r) => re.test(r.name)).reduce((a, r) => a + r.transferSize, 0) / 1024);
          return { first_paint_ms: Math.round(p['first-contentful-paint'] ?? -1), dom_loaded_ms: Math.round(n.domContentLoadedEventEnd),
                   js_kb: kb(/\\.js($|\\?)/), css_kb: kb(/\\.css($|\\?)/), requests: res.length }; }""")
        t["usable_ms"] = round(usable)
        t["messages_shown"] = self.page.locator("[data-said]").count()
        self.page.wait_for_timeout(800)
        t |= self.weight()
        self.picture("open")
        return t

    def type(self):
        box = self.composer()
        box.click()
        self.page.evaluate(KEYS)
        self.throttle(True)
        row = self.frames(lambda: self.page.keyboard.type("The quick brown fox jumps over the lazy dog, then again and again. ", delay=30))
        self.throttle(False)
        keys = self.page.evaluate("window.__keys")
        box.fill("")
        return row | {"keys": 67, "keys_over_16ms": len(keys), "worst_key_ms": round(max(keys or [0]), 1)}

    def stream(self):
        self.throttle(True)
        row = self.frames(lambda: self.turn(600), settle=300)
        self.throttle(False)
        return row

    def scroll(self):
        chat = self.find(["[data-surface=chat]"]) or self.page.locator("[data-talk]").first
        b = chat.bounding_box()
        x, y = b["x"] + b["width"] / 2, b["y"] + b["height"] / 2

        def wheel(dy, steps):
            def go():
                self.page.mouse.move(x, y)
                for _ in range(steps):
                    self.page.mouse.wheel(0, dy)
                    self.page.wait_for_timeout(16)
            return go
        self.throttle(True)
        out = {"up": self.frames(wheel(-400, 60)), "down": self.frames(wheel(400, 30))}
        foot = self.page.get_by_role("button", name="Scroll to bottom")
        if foot.count() and foot.first.is_enabled():
            out["jump to foot"] = self.frames(lambda: foot.first.click(), settle=1200)
        out["at"] = round(self.page.evaluate("(e) => e.scrollTop", chat.element_handle()))
        self.throttle(False)
        return out

    def surfaces(self):
        out = {}
        for kind in SURFACES:
            if self.size == "desktop" and self.surface(kind):
                out[kind] = {"already_open": True}
                self.picture(kind)
                continue
            button = self.opener(kind)
            if not button:
                out[kind] = {"missing": "no control opens it"}
                continue

            def go(kind=kind, button=button):
                button.click()
                start = time.monotonic()
                self.page.locator(", ".join(SHOWN[kind])).filter(visible=True).first.wait_for(timeout=15000)
                return {"shown_ms": round((time.monotonic() - start) * 1000)}
            self.throttle(True)
            try:
                out[kind] = {"open": self.frames(go, settle=700)}
            except Exception as e:
                out[kind] = {"failed": str(e).splitlines()[0][:200]}
                self.throttle(False)
                self.home()
                continue
            self.throttle(False)
            self.picture(kind)
            self.throttle(True)
            out[kind]["close"] = self.frames(lambda kind=kind: self.close(kind))
            self.throttle(False)
            self.home()
        return out

    def page_link(self):
        name = f"drive-{self.size}"
        start = time.monotonic()
        self.send(f"PUBLISH {name}")

        def showing():
            return any(f.query_selector(f"#mark-{name}") for f in self.page.frames if not f.is_detached())
        out = {}
        try:
            soon(showing, "page published", 30)
            out["published_shown_ms"] = round((time.monotonic() - start) * 1000)
        except TimeoutError:
            out["published_shown_ms"] = None
        self.home()
        link = self.page.get_by_role("link", name=f"page {name}").last
        link.wait_for(timeout=20000)
        tabs = len(self.context.pages)
        start = time.monotonic()
        link.click()
        try:
            soon(showing, "page opened", 20)
            out["link_shown_ms"] = round((time.monotonic() - start) * 1000)
        except TimeoutError:
            out["link_shown_ms"] = None
        out["new_tabs"] = len(self.context.pages) - tabs
        self.picture("page")
        self.home()
        return out

    # The tiled layout's workspaces and templates.
    def spaces(self):
        """The workspaces' names, as the pill (or a phone's sheet) lists them."""
        if self.size == "phone":
            self.page.locator("[data-space]").first.click()
            names = self.page.locator("[role=dialog] [data-action=space-menu]").evaluate_all(
                "(bs) => bs.map((b) => b.getAttribute('aria-label').replace(/^More for /, ''))")
            self.page.keyboard.press("Escape")
            self.page.wait_for_timeout(200)
            return names
        return self.page.locator("[data-space]").evaluate_all("(bs) => bs.map((b) => b.dataset.space)")

    def space_menu(self, action):
        """Picks `action` from the menu of the workspace in use."""
        if self.size == "phone":
            self.page.locator("[data-space]").first.click()
            self.page.locator("[role=dialog] [data-action=space-menu][data-current]").click()
        else:
            self.page.locator("[aria-current=page][data-space]").click(button="right")
        self.page.locator(f"[role=menu] [data-action={action}]").filter(visible=True).first.click()
        self.page.wait_for_timeout(300)

    def add_space(self):
        if self.size == "phone":
            self.page.locator("[data-space]").first.click()
            self.page.locator("[role=dialog] [data-action=space-add]").click()
        else:
            self.page.locator("[data-action=space-add]").click()
        box = self.page.get_by_label("Workspace name").filter(visible=True)
        box.wait_for(timeout=5000)
        return box

    def tiles_shown(self):
        return self.page.locator("section[data-here]").evaluate_all("(ts) => ts.map((t) => t.dataset.surface || 'empty')")

    def covered(self, kind):
        """What of a tile's own header its notch lies over: at rest, any
        control or words; opened (hovered), any control. Both should be none."""
        tile = self.page.locator(f"section[data-surface={kind}]").filter(visible=True).first
        over = """(t, [part, words]) => {
            const n = t.querySelector('[data-notch]');
            const p = n && n.querySelector(part);
            if (!p || !p.offsetParent) return ['no notch'];
            const a = p.getBoundingClientRect();
            const hit = (r) => r.width && r.left < a.right && r.right > a.left && r.top < a.bottom && r.bottom > a.top;
            const out = [...t.querySelectorAll('button, input, a, [role=button]')]
              .filter((b) => !n.contains(b) && b.offsetParent && hit(b.getBoundingClientRect()))
              .map((b) => b.getAttribute('aria-label') || b.textContent.trim().slice(0, 40));
            if (words) {
              const w = document.createTreeWalker(t, NodeFilter.SHOW_TEXT);
              for (let x; (x = w.nextNode());) {
                if (!x.textContent.trim() || n.contains(x)) continue;
                const r = document.createRange(); r.selectNodeContents(x);
                if ([...r.getClientRects()].some(hit)) out.push('words: ' + x.textContent.trim().slice(0, 40));
              }
            }
            return out;
        }"""
        self.page.mouse.move(0, 0)
        self.page.wait_for_timeout(300)
        rest = tile.evaluate(over, ["[data-action=controls]", True])
        if self.size == "phone":
            tile.locator("[data-notch] [data-action=controls]").click()
        else:
            tile.locator("[data-notch]").hover()
        self.page.wait_for_timeout(300)
        opened = tile.evaluate(over, ["[role=group]", False])
        return {"at_rest": rest, "opened": opened}

    def layout_sync(self):
        """Initial read failure, refused save/retry and navigation during hydration."""
        self.home()
        self.page.wait_for_timeout(1000)
        host = self.page.request.get(self.t.base + "/api/layout").json()["layout"]
        host["spaces"][0]["name"] = "Host arrangement"
        self.page.request.post(self.t.base + "/api/layout", data=host)
        self.page.goto("about:blank")
        failed = {"read": True, "write": False}
        posts = []

        def route(request):
            method = request.request.method
            if method == "POST":
                posts.append(request.request.post_data_json)
            if (method == "GET" and failed["read"]) or (method == "POST" and failed["write"]):
                request.fulfill(status=503, body="unavailable")
            else:
                request.continue_()

        self.page.route("**/api/layout*", route)
        self.page.goto(self.url)
        self.composer().wait_for()
        self.page.wait_for_timeout(1500)
        assert posts == [], "a failed first read posted cached workspaces"
        failed["read"] = False
        soon(lambda: "Host arrangement" in self.spaces(), "host layout after retry", 10)
        failed["write"] = True
        box = self.add_space()
        box.fill("Pending workspace")
        box.press("Enter")
        soon(lambda: self.page.get_by_role("status").filter(has_text="Couldn’t save workspaces").count() > 0, "save failure shown", 8)
        failed["write"] = False
        soon(lambda: any(w["name"] == "Pending workspace" for w in self.page.request.get(self.t.base + "/api/layout").json()["layout"]["spaces"]), "failed save retried", 10)
        self.page.unroute("**/api/layout*", route)
        self.page.goto("about:blank")
        self.page.goto(self.url + "#library")
        self.page.locator("section[data-surface=library]").filter(visible=True).wait_for()
        self.page.wait_for_timeout(1500)
        assert self.page.locator("section[data-surface=library]").filter(visible=True).count(), "hydration dropped the requested Library"
        return {"no_stale_post": True, "save_retried": True, "navigation_kept": True}

    def reload(self):
        """Loading a finished history must not retain a blank turn's floor."""
        self.home()
        heights = []
        for _ in range(3):
            self.page.reload()
            self.composer().wait_for()
            self.page.locator("[data-said]").last.wait_for()
            self.page.wait_for_timeout(1200)
            height = self.page.locator('section[data-surface=chat] main > div[aria-hidden]').evaluate("e => e.offsetHeight")
            heights.append(height)
            assert height <= 1, f"reload retained a {height}px spacer"
        return {"spacer_px": heights}

    def code_contrast(self):
        self.home()
        token = self.page.locator('[data-streamdown="code-block"] code span[style]').filter(has_text="fn").first
        token.wait_for()
        ratios = {}
        for theme in ("light", "dark"):
            self.page.evaluate("t => document.documentElement.dataset.theme = t", theme)
            ratio = token.evaluate("""e => {
                const color = getComputedStyle(e).color.match(/[\d.]+/g).slice(0,3).map(Number);
                const hex = getComputedStyle(document.documentElement).getPropertyValue('--block').trim();
                const bg = [1,3,5].map(i => parseInt(hex.slice(i,i+2),16));
                const lum = rgb => rgb.map(v=>v/255).map(v=>v<=0.04045?v/12.92:((v+0.055)/1.055)**2.4).reduce((a,v,i)=>a+v*[0.2126,0.7152,0.0722][i],0);
                const a=lum(color), b=lum(bg); return (Math.max(a,b)+0.05)/(Math.min(a,b)+0.05);
            }""")
            ratios[theme] = round(ratio, 2)
            assert ratio >= 4.5, f"{theme} code keyword contrast {ratio:.2f}:1"
        self.page.evaluate("document.documentElement.dataset.theme='light'")
        return {"keyword_contrast": ratios}

    def worker_action(self):
        self.home()
        self.opener("workers").click()
        tile = self.page.locator("section[data-surface=workers]").filter(visible=True).first
        action = tile.get_by_role("button", name="Start a worker", exact=True)
        action.wait_for()
        viewport = tile.locator(".overflow-y-auto").first
        positions = []
        for fraction in (0, 0.5, 1):
            viewport.evaluate("(v,f) => v.scrollTop=v.scrollHeight*f", fraction)
            self.page.wait_for_timeout(200)
            v, b = viewport.bounding_box(), action.bounding_box()
            assert v["y"] + v["height"] <= b["y"], "worker action overlays the scrolling list"
            positions.append(round(b["y"] - v["y"] - v["height"]))
        self.picture("worker-action")
        return {"clearance_px": positions}

    def terminal_keys(self):
        self.home()
        self.opener("terminal").click()
        terminal = self.page.locator("section[data-surface=terminal]").filter(visible=True).first
        terminal.wait_for()
        keys = terminal.get_by_role("button", name="Esc", exact=True)
        if self.size == "phone":
            keys.wait_for()
            terminal.get_by_role("button", name="Ctrl", exact=True).wait_for()
            terminal.get_by_role("button", name="↑", exact=True).wait_for()
        else:
            assert keys.count() == 0, "desktop showed phone keys"
        return {"phone_keys": self.size == "phone"}

    def settings(self):
        self.home()
        self.page.get_by_role("button", name="Settings and usage", exact=True).filter(visible=True).first.click()
        settings = self.page.locator('[aria-label="Settings"]').filter(visible=True).first
        if self.size == "phone":
            settings.get_by_role("button", name="Settings", exact=True).click()
        version = settings.locator('[aria-label="Libri version and updates"]')
        version.wait_for()
        self.page.wait_for_timeout(300)
        assert "v" in version.inner_text(), "Settings did not show a version"
        version.get_by_role("button", name="Check for updates").click()
        version.get_by_role("status").wait_for()
        self.picture("settings-version")
        bounds = version.bounding_box()
        assert bounds["x"] >= -1, f"Settings content scrolled sideways: {bounds}"
        return {"version": version.inner_text(), "update_control": True, "bounds": bounds}

    def kept_templates(self):
        try:
            kept = json.loads((self.t.root / "state/layout.json").read_text())
            return [t["name"] for t in kept.get("layout", kept).get("templates", [])]
        except (OSError, ValueError):
            return None

    def workspaces(self):
        """Adds, renames, lays out and closes workspaces; saves a template,
        starts one from it, renames and deletes it; and finds what the notch
        on a Temp tile covers of its header."""
        if self.args.layout != "tiles":
            return {"skipped": "the columns layout has no workspaces"}
        self.home()
        out = {"first_run": self.spaces()}
        # Add: an empty workspace, its name open for typing.
        box = None

        def add():
            nonlocal box
            box = self.add_space()
        out["add"] = self.frames(add)
        box.fill("Scratch")
        box.press("Enter")
        self.page.wait_for_timeout(300)
        out["after_add"] = self.spaces()
        out["new_tiles"] = self.tiles_shown()
        self.picture("workspace-new")
        # A template lays it out.
        self.page.locator('[data-template="Chat and Temp"]').filter(visible=True).first.click()
        self.page.wait_for_timeout(500)
        out["from_preset"] = self.tiles_shown()
        if self.size == "phone":
            self.page.get_by_role("button", name="Temp chat", exact=True).filter(visible=True).first.click()
            self.page.wait_for_timeout(300)
        out["notch_covers_temp"] = self.covered("temp")
        self.picture("workspace-notch")
        # Save as template, then start a new workspace from it.
        self.space_menu("space-template")
        soon(lambda: "Scratch" in (self.kept_templates() or []), "template kept on the host", 10)
        out["kept_on_host"] = self.kept_templates()
        self.space_menu("space-template")
        soon(lambda: "Scratch (2)" in (self.kept_templates() or []), "distinct second template name", 10)
        out["distinct_template_names"] = self.kept_templates()
        # Remove the second copy so the rename/delete scenario still ends empty.
        state = self.page.request.get(self.t.base + "/api/layout").json()["layout"]
        state["templates"] = [t for t in state["templates"] if t["name"] != "Scratch (2)"]
        self.page.request.post(self.t.base + "/api/layout", data=state)
        self.page.wait_for_timeout(600)
        box = self.add_space()
        box.press("Escape")
        self.page.locator('[data-template="Scratch"] button').first.click()
        self.page.wait_for_timeout(500)
        out["from_template"] = {"tiles": self.tiles_shown(), "spaces": self.spaces()}
        # Rename and delete the template, from a new empty workspace.
        box = self.add_space()
        box.press("Escape")
        self.page.locator('[data-template="Scratch"] [data-action=template-rename]').click()
        name = self.page.get_by_label("Template name")
        name.fill("Pair")
        name.press("Enter")
        self.page.wait_for_timeout(300)
        out["renamed_template"] = self.page.locator("[data-template=Pair]").count() == 1
        self.picture("workspace-templates")
        self.page.locator("[data-template=Pair] [data-action=template-delete]").click()
        soon(lambda: self.kept_templates() == [], "template deleted on the host", 10)
        out["deleted_template"] = self.page.locator("[data-template=Pair]").count() == 0
        # Rename in place: double-click on a desktop, long press on a phone.
        pill = self.page.locator("[aria-current=page][data-space]" if self.size == "desktop" else "[data-space]").first
        if self.size == "desktop":
            pill.dblclick()
        else:
            b = pill.bounding_box()
            self.page.mouse.move(b["x"] + 20, b["y"] + 10)
            self.page.mouse.down()
            self.page.wait_for_timeout(700)
            self.page.mouse.up()
        name = self.page.get_by_label("Workspace name").filter(visible=True)
        name.fill("Renamed")
        name.press("Enter")
        self.page.wait_for_timeout(300)
        out["renamed_space"] = "Renamed" in self.spaces()
        # Close every workspace: the last leaves one holding the chat.
        closed = 0
        while len(self.spaces()) > 1 and closed < 8:
            self.space_menu("space-close")
            closed += 1
        self.space_menu("space-close")
        out["closed"] = closed + 1
        out["after_close"] = {"spaces": self.spaces(), "tiles": self.tiles_shown()}
        self.home()
        return out

    def heard(self, act):
        """Runs `act`, then times, by the page's clock, until its event stream
        opens again; then whether a turn still comes through."""
        act()
        start = self.page.evaluate("performance.now()")
        try:
            self.page.wait_for_function("(t) => window.__opens.some((o) => o > t)", arg=start, timeout=60000)
            back = round(self.page.evaluate("(t) => Math.min(...window.__opens.filter((o) => o > t))", start) - start)
        except Exception:
            back = None
        # Openings past the first in the next 6 s: each one more sync of the history.
        self.page.wait_for_timeout(6000)
        again = self.page.evaluate("(t) => window.__opens.filter((o) => o > t).length", start) - 1 if back is not None else None
        ok = True
        try:
            self.turn(30)
        except Exception:
            ok = False
        return {"reconnect_ms": back, "extra_reopens": again, "turn_after": ok}

    def drop(self):
        """Offline for 4 s with the stream cut, then online again."""
        def act():
            self.context.set_offline(True)
            self.page.evaluate("window.__cut()")
            self.page.wait_for_timeout(4000)
            self.context.set_offline(False)
        return self.heard(act)

    def restart(self, t):
        """The daemon restarted under the page; timed from when it answers again."""
        return self.heard(lambda: restart(t))

    def session(self, rounds):
        start = self.weight()
        for _ in range(rounds):
            self.turn(150)
            for kind in ("temp", "terminal"):
                button = self.opener(kind)
                if button:
                    button.click()
                    self.page.wait_for_timeout(400)
                    self.close(kind)
        end = self.weight()
        return {"rounds": rounds, "start": start, "end": end,
                "dom_nodes_per_round": round((end["dom_nodes"] - start["dom_nodes"]) / rounds, 1),
                "heap_mb_per_round": round((end["heap_mb"] - start["heap_mb"]) / rounds, 3)}


def scenario(run, name, *given):
    errors = len(run.errors)
    try:
        got = getattr(Run, {"page": "page_link"}.get(name, name))(run, *given)
    except Exception as e:  # a scenario that fails is a result, and the rest still run
        got = {"failed": f"{type(e).__name__}: {str(e)[:1500]}"}
        try:
            (run.out / "failures").mkdir(exist_ok=True)
            got["picture"] = str(run.out / "failures" / f"{run.size}-{name}.png")
            run.page.screenshot(path=got["picture"])
        except Exception:
            pass
        try:
            run.throttle(False)
            run.home()
        except Exception:
            pass
    got["console_errors"] = run.errors[errors:]
    if name in ("drop", "restart"):
        # A lost connection's own failed loads are what these scenarios cause.
        got["network_errors"] = [e for e in got["console_errors"] if e.startswith("Failed to load resource: net::")]
        got["console_errors"] = [e for e in got["console_errors"] if e not in got["network_errors"]]
    print(f"-- {run.size} {name}", file=sys.stderr, flush=True)
    return got


def compare_shots(shots, goldens, update):
    """Each picture against its golden: the share of pixels past CHANNEL."""
    from PIL import Image, ImageChops
    out = {}
    goldens.mkdir(parents=True, exist_ok=True)
    for shot in sorted(shots.glob("*.png")):
        golden = goldens / shot.name
        if update or not golden.exists():
            golden.write_bytes(shot.read_bytes())
            out[shot.stem] = "kept as golden"
            continue
        a, b = Image.open(golden).convert("RGB"), Image.open(shot).convert("RGB")
        if a.size != b.size:
            out[shot.stem] = {"differs": 1.0, "why": f"size {b.size} against {a.size}"}
            continue
        diff = ImageChops.difference(a, b).convert("L").point(lambda v: 255 if v > CHANNEL else 0)
        share = sum(diff.histogram()[255:]) / (a.size[0] * a.size[1])
        row = {"differs": round(share, 5), "ok": share <= TOLERANCE}
        if share > TOLERANCE:
            path = shots / f"{shot.stem}.diff.png"
            Image.composite(Image.new("RGB", a.size, (230, 40, 40)), b, diff).save(path)
            row["diff"] = str(path)
        out[shot.stem] = row
    return out


def median_of(runs):
    """Numbers become their median across repeats; the rest is the first run's."""
    first = runs[0]
    if isinstance(first, dict):
        return {k: median_of([r.get(k) for r in runs if isinstance(r, dict)]) for k in first}
    if isinstance(first, (int, float)) and not isinstance(first, bool) and all(isinstance(r, (int, float)) for r in runs):
        return round(statistics.median(runs), 2)
    return first


# What the scorecard shows: (label, path into a size's results, lower is better).
CARD = [
    ("first paint ms", "open.first_paint_ms"), ("usable ms", "open.usable_ms"), ("JS kB", "open.js_kb"),
    ("DOM nodes at open", "open.dom_nodes"), ("heap MB at open", "open.heap_mb"),
    ("keys over 16ms", "type.keys_over_16ms"), ("worst key ms", "type.worst_key_ms"),
    ("stream first words ms", "stream.first_words_ms"), ("stream dropped frames", "stream.dropped"),
    ("stream long frames ms", "stream.long_frame_ms"), ("scroll up dropped", "scroll.up.dropped"),
    ("scroll up p95 frame ms", "scroll.up.p95_frame_ms"), ("scroll down dropped", "scroll.down.dropped"),
    ("jump to foot dropped", "scroll.jump to foot.dropped"),
    ("temp open ms", "surfaces.temp.open.ms"), ("terminal open ms", "surfaces.terminal.open.ms"),
    ("workers open ms", "surfaces.workers.open.ms"), ("library open ms", "surfaces.library.open.ms"),
    ("page by link ms", "page.link_shown_ms"), ("reconnect after drop ms", "drop.reconnect_ms"),
    ("reconnect after restart ms", "restart.reconnect_ms"),
    ("extra reopens after drop", "drop.extra_reopens"), ("extra reopens after restart", "restart.extra_reopens"), ("DOM nodes / round", "session.dom_nodes_per_round"),
    ("heap MB / round", "session.heap_mb_per_round"),
]


def dig(tree, path):
    for k in path.split("."):
        if not isinstance(tree, dict) or k not in tree:
            return None
        tree = tree[k]
    return tree


def errors_of(results):
    return sum(len(v.get("console_errors", [])) for v in results.values() if isinstance(v, dict))


def failed_of(results):
    return [k for k, v in results.items() if isinstance(v, dict) and "failed" in v]


def scorecard(report, other=None):
    sizes = list(report["sizes"])
    head = f"{'':30}" + "".join(f"{s:>16}" for s in sizes)
    lines = [f"Libri UI  layout={report['layout']}  seed={report['seed']}  messages={report['messages']}  repeat={report['repeat']}"
             f"  load {report.get('load_before', ['?'])[0]}->{report.get('load_after', ['?'])[0]}", head]
    rows = CARD + [("console errors", None)]
    for label, path in rows:
        cells = []
        for s in sizes:
            res = report["sizes"][s]
            v = errors_of(res) if path is None else dig(res, path)
            cell = "-" if v is None else str(v)
            if other and s in other["sizes"]:
                o = errors_of(other["sizes"][s]) if path is None else dig(other["sizes"][s], path)
                if isinstance(v, (int, float)) and isinstance(o, (int, float)) and o != v:
                    cell = f"{o}->{v}"
            cells.append(f"{cell:>16}")
        lines.append(f"{label:30}" + "".join(cells))
    for s in sizes:
        bad = failed_of(report["sizes"][s])
        if bad:
            lines.append(f"FAILED at {s}: " + ", ".join(f"{k} ({report['sizes'][s][k]['failed']})" for k in bad))
    shots = report.get("goldens") or {}
    off = [k for k, v in shots.items() if isinstance(v, dict) and not v.get("ok")]
    lines.append(f"goldens: {len(shots)} pictures, {len(off)} past tolerance" + (": " + ", ".join(off) if off else ""))
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("libri_dir", nargs="?")
    ap.add_argument("--compare", nargs=2, metavar=("BEFORE", "AFTER"))
    ap.add_argument("--site", help="the page to serve, a build of another branch")
    ap.add_argument("--browser", choices=["chromium", "webkit"], default="chromium")
    ap.add_argument("--layout", choices=["columns", "tiles"], default="columns")
    ap.add_argument("--only", default=",".join(SCENARIOS))
    ap.add_argument("--sizes", default="desktop,phone")
    ap.add_argument("--messages", type=int, default=2000)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--repeat", type=int, default=1)
    ap.add_argument("--rounds", type=int, default=10, help="the long session's rounds")
    ap.add_argument("--out", default="ui-driver-run")
    ap.add_argument("--goldens", default=str(HERE / "ui_goldens"))
    ap.add_argument("--update-goldens", action="store_true")
    args = ap.parse_args()
    if args.compare:
        before, after = (json.loads(Path(p).read_text()) for p in args.compare)
        print(scorecard(after, before))
        return
    if not args.libri_dir:
        ap.error("LIBRI_DIR is needed")
    only = [s for s in args.only.split(",") if s]
    unknown = set(only) - set(SCENARIOS)
    if unknown:
        ap.error(f"no scenario {', '.join(sorted(unknown))}")
    out = Path(args.out).resolve()
    shots = out / "shots"
    shots.mkdir(parents=True, exist_ok=True)

    from playwright.sync_api import sync_playwright
    t = Throwaway(args.libri_dir, prefix="libri-ui-driver-", site=args.site)
    report = {"browser": args.browser, "cpu_throttling": args.browser == "chromium", "layout": args.layout, "seed": args.seed, "messages": args.messages, "repeat": args.repeat,
              "site": args.site or str(Path(args.libri_dir) / "site"), "at": time.strftime("%Y-%m-%dT%H:%M:%S"),
              # Timings mean little on a busy machine: what it carried, before and after.
              "load_before": [round(x, 1) for x in os.getloadavg()], "sizes": {}}
    try:
        start = time.monotonic()
        seed_chat(t, args.messages, args.seed)
        report["seeding_s"] = round(time.monotonic() - start, 1)
        with sync_playwright() as p:
            browser = getattr(p, args.browser).launch(**({"args": ["--enable-gpu-rasterization", "--ignore-gpu-blocklist"]} if args.browser == "chromium" else {}))
            for size in [s for s in args.sizes.split(",") if s]:
                repeats = []
                for k in range(args.repeat):
                    run = Run(browser, t, size, args, out, shots if k == 0 else None)
                    res = {}
                    for name in SCENARIOS:
                        if name not in only:
                            continue
                        if name != "open" and "open" not in res:
                            run.home()
                            res["open"] = {"skipped": True}
                        given = {"restart": (t,), "session": (args.rounds,)}.get(name, ())
                        res[name] = scenario(run, name, *given)
                    run.context.close()
                    repeats.append(res)
                report["sizes"][size] = median_of(repeats)
            browser.close()
        report["goldens"] = compare_shots(shots, Path(args.goldens) / args.layout, args.update_goldens)
        report["load_after"] = [round(x, 1) for x in os.getloadavg()]
        report["strays"] = t.strays()
    finally:
        t.close(keep=False)
    (out / "run.json").write_text(json.dumps(report, indent=1))
    print(scorecard(report))
    print(f"report: {out / 'run.json'}")
    if report["strays"]:
        print(f"LEAKED outside the scratch root: {report['strays']}")
        sys.exit(1)


if __name__ == "__main__":
    main()
