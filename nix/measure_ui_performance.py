#!/usr/bin/env python3
"""How smooth and how quick the web page is, in numbers: every column's slide,
the side columns' sash, a fold, scrolling, typing, streaming, and the first
load. Runs the built site against a stand-in host of real size (a long chat
with code and lists, 36 workers, a long worker thread) in Chromium, at 1024,
1440 and a phone, with the processor at full speed and slowed 4 times as a
phone's is. Prints one JSON report. A measure, not a check: it fails nothing,
and the numbers are compared before and after a change. Needs Playwright and a
browser, so it stays out of `nix flake check`. Usage:
measure_ui_performance.py LIBRI/site [PART...] (a built Libri's site, its tiles from LIBRI/bin/libri), parts: load motion steady phone;
measure_ui_performance.py LIBRI/site serve PORT keeps the stand-in host up on
127.0.0.1:PORT, for screenshots and by hand.
"""
import functools
import http.server
import json
import queue
import sys
import threading
import time
from pathlib import Path
from urllib.parse import urlparse

from playwright.sync_api import sync_playwright

SITE = Path(sys.argv[1])
PARTS = sys.argv[2:] or ["load", "motion", "steady", "phone"]
NOW = int(time.time())
WORKER = "Reed"
FRAME = 1000 / 60


def talk_text(n):
    """A reply as the assistant writes them: paragraphs, a list, now and then code."""
    out = [f"Reply {n}. The harness came back with the run's state, and the summary reads as follows, in more words than a line so the text wraps across the column at every width we draw."]
    out.append("\n".join(f"- Item {k} of the list, with `inline code` and a [link](https://example.com/{k})." for k in range(4)))
    if n % 3 == 0:
        out.append("```rust\nfn main() {\n    let total: u64 = (0..1000).filter(|n| n % 3 == 0).sum();\n    println!(\"{total}\");\n}\n```")
    if n % 5 == 0:
        out.append("| Host | State | Since |\n|---|---|---|\n| box | live | 03:57 |\n| den | stable | 11-01 |")
    out.append("That is where it stands; the next step waits on the check.")
    return "\n\n".join(out)


def history():
    said = []
    for n in range(60):
        said.append({"i": 1000 + 2 * n, "kind": "user", "text": f"Question {n}: how does the run look now, and what is left before it can ship?", "date": "2026-11-01T12:40:00Z", "reaction": None})
        said.append({"i": 1001 + 2 * n, "kind": "talk", "text": talk_text(n), "date": "2026-11-01T12:41:00Z", "reaction": None})
    return said


NAMES = ["Reed", "Wren", "Sable", "Tarn", "Cross", "Moss", "Ibis", "Wigeon", "Heron", "Plover", "Teal", "Gull", "Ouzel", "Dunlin", "Finch", "Lark", "Rook", "Swift", "Kite", "Merlin", "Egret", "Crane", "Stint", "Knot", "Avocet", "Godwit", "Curlew", "Snipe", "Shrike", "Linnet", "Twite", "Pipit", "Bunting", "Thrush", "Robin", "Wagtail"]
STATES = ["working"] * 8 + ["waiting"] * 6 + ["sleeping"] * 14 + ["archived"] * 8


def tiles():
    """The kinds of tile as the built Libri beside the site declares them
    (`libri tiles`, tiles.rs), so the stand-in never drifts from the host."""
    import subprocess
    libri = SITE.parent / "bin" / "libri"
    if not libri.exists():
        # With no tiles the page draws no dock and no tile menus, and every
        # timing run passes on a page missing half its controls.
        sys.exit(f"{libri} is missing: give the site of a built Libri (<libri>/site), whose tiles the stand-in serves")
    return json.loads(subprocess.run([str(libri), "tiles"], capture_output=True, text=True, check=True).stdout)


def threads():
    return [
        {"id": f"w{k}", "name": name, "brief": f"Check the {name} work", "title": f"{name} batch check", "gist": "Comparing three quotes against the last run", "summary": "Comparing three quotes against the last run",
         "state": state, "at": NOW - 300 * (k + 1), "read": state in ("sleeping", "archived"), "waiting_on": [], "archived": state == "archived"}
        for k, (name, state) in enumerate(zip(NAMES, STATES))
    ]


def agent_chat():
    lines = []
    for n in range(80):
        lines.append(f"tool: Bash {{\"command\":\"cargo test -p libri -- step_{n}\",\"description\":\"Run step {n}\"}}")
        lines.append(f"echo: test result: ok. {n} passed; 0 failed")
        lines.append(f"talk: Step {n} passes; moving to the next file. " + "More words follow. " * 8)
    return "\n".join(lines)


class Host(http.server.SimpleHTTPRequestHandler):
    """The built site and a stand-in API on one port; /api/events streams
    what `stream` is given."""
    streams = []
    replies = {}

    def log_message(self, *_):
        pass

    def end_headers(self):
        # As the real host: the hashed files may be kept, the page not.
        if "/_site/" in self.path:
            self.send_header("cache-control", "public, max-age=31536000, immutable")
        super().end_headers()

    def do_GET(self):
        path = urlparse(self.path).path
        if "/api/" not in path:
            return super().do_GET()
        path = path[path.index("/api/"):]
        if path == "/api/events":
            self.send_response(200)
            self.send_header("content-type", "text/event-stream")
            self.send_header("cache-control", "no-store")
            self.end_headers()
            q = queue.Queue()
            Host.streams.append(q)
            try:
                self.wfile.write(b": open\n\n")
                self.wfile.flush()
                while True:
                    try:
                        item = q.get(timeout=5)
                    except queue.Empty:
                        item = ": ping\n\n"
                    self.wfile.write(item.encode())
                    self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                pass
            finally:
                Host.streams.remove(q)
            return
        if path.startswith("/api/agent/") and path.endswith("/chat"):
            return self.json({"text": agent_chat()})
        if path.startswith("/api/work/") and path.endswith("/choice"):
            return self.json(CHOICE)
        return self.json(Host.replies.get(path, {}))

    def do_POST(self):
        self.json({})

    def json(self, value):
        body = json.dumps(value).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("cache-control", "no-store")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def send(event):
    for q in list(Host.streams):
        q.put(f"data: {json.dumps(event)}\n\n")


def offer(harness, model, name, default=False, efforts=("low", "medium", "high", "xhigh", "max")):
    usage = {"claude": "claude", "codex": "codex"}[harness]
    provider = {"claude": "anthropic", "codex": "openai"}[harness]
    return {"default": default, "efforts": list(efforts), "harness": harness, "model": model, "name": name, "provider": provider, "usage": usage}


def mode(id, name, note, open=False, hidden=False):
    return {"hidden": hidden, "id": id, "name": name, "note": note, "open": open}


# As the host answers api/models: two harnesses' models and their own modes.
CHOICE = {"access": "auto", "effort": "", "pick": {"harness": "claude", "model": "claude-opus-5-5"}, "sandbox": "off"}
MODELS = {
    **CHOICE,
    "defaultHarness": "claude",
    "harnesses": [{"harness": "claude", "model": None, "on": True, "remedy": "run `claude` in a terminal on this host and sign in"},
                  {"harness": "codex", "model": None, "on": True, "remedy": "type /signin codex for a ChatGPT code"}],
    "offers": [offer("claude", "claude-opus-5-5", "Opus 5.5", True), offer("claude", "claude-sonnet-5-5", "Sonnet 5.5"), offer("claude", "claude-haiku-5-5", "Haiku 5.5"),
               offer("codex", "gpt-6.1-sol", "GPT-6.1-Sol", True, ("low", "medium", "high", "xhigh", "max", "ultra")), offer("codex", "gpt-6-luna", "GPT-6-Luna")],
    "modes": [
        {"harness": "claude", "defaultAccess": "auto", "defaultSandbox": "off",
         "access": [mode("manual", "Manual", "Asks before edits and commands"), mode("acceptEdits", "Accept edits", "Edits and file commands in the project run; asks before the rest"),
                    mode("plan", "Plan", "Reads and plans; changes nothing"), mode("auto", "Auto", "A classifier decides each action; it rarely asks"),
                    mode("bypassPermissions", "Full access", "Asks nothing", open=True)],
         "sandbox": [mode("off", "Sandbox off", "Commands run on the machine", open=True, hidden=True), mode("regular", "Sandbox, regular", "Bash runs inside the sandbox; the mode still asks", hidden=True)]},
        {"harness": "codex", "defaultAccess": "on-request", "defaultSandbox": "workspace-write",
         "access": [mode("untrusted", "Untrusted", "Asks before anything not known safe"), mode("on-request", "On request", "Asks to go past the sandbox"),
                    mode("never", "Never", "Never asks; what the sandbox stops goes back to the model", open=True)],
         "sandbox": [mode("read-only", "Read only", "Reads anything, writes nothing"), mode("workspace-write", "Workspace write", "Writes in the project"),
                     mode("danger-full-access", "Full access", "No sandbox", open=True)]},
    ],
    "others": None, "othersOffered": [], "signedOut": [],
    "temp": CHOICE,
}

Host.replies = {
    "/api/me": {"app": "Libri", "name": "Libri", "scope": None, "voice": None, "models": True, "forgets": []},
    "/api/models": MODELS,
    "/api/work-choice": CHOICE,
    "/api/worker-model": {"picked": None},
    "/api/builtins": [
        {"id": "calendar", "title": "Calendar", "about": "Your calendars and events.", "on": False, "connection": None, "provider": None, "signed_in": False,
         "providers": [{"id": "fastmail", "title": "Fastmail", "agenda": True, "found": None, "note": None}]},
        {"id": "mail", "title": "Mail", "about": "Your mailboxes: reading, drafting and filing mail.", "on": False, "connection": None, "provider": None, "signed_in": False,
         "providers": [{"id": "fastmail", "title": "Fastmail", "agenda": False, "found": None, "note": None}]},
    ],
    "/api/connections": {"servers": [], "xai": None, "xai_said": None, "xai_signing": None},
    "/api/library": {"lists": [], "stills": False,
                     "pages": [{"changed": NOW - 3600 * k, "name": f"plan-{k}", "title": f"Plan {k}" if k % 3 else None} for k in range(12)]},
    "/api/watches": {"watches": [
        {"id": 1, "active": True, "at": NOW + 3600, "every": "daily", "failing": None, "kind": "ask", "looks": None, "worker": None, "text": "Check the release notes of the tools we run for anything new since yesterday."},
        {"id": 2, "active": True, "at": NOW + 86400, "every": "weekly", "failing": None, "kind": "ask", "looks": None, "worker": None, "text": "Update the agent apps and say which versions changed."},
    ]},
    "/api/usage/report": {"shown": True, "notes": ["Shares count use logged on this host."], "accounts": [
        {"account": "claude", "name": "Claude", "notes": ["libri 99% · memory 1%"], "windows": [{"hours": 168.0, "name": "Week", "pct": 42, "resets": "Sun Nov 8 17:00", "window": "week"}]},
        {"account": "codex", "name": "ChatGPT", "notes": [], "windows": [{"hours": 5.0, "name": "5 hours", "pct": 7, "resets": "Fri Nov 6 17:00", "window": "5h"}, {"hours": 168.0, "name": "Week", "pct": 2, "resets": "Wed Nov 11 08:30", "window": "week"}]},
    ]},
    "/api/worker-names": {"builtin": NAMES, "added": [], "own": [], "use_own": False, "excluded": [], "allowed": len(NAMES), "left_out": []},
    "/api/guide": {"text": "## Engineering\n\n- Study how established products solve the problem first.\n- Choose the simplest design that fully meets today's needs.\n"},
    "/api/capabilities": {"fixed": [], "image": {"choices": [], "picked": None}},
    "/api/summarizer": {"elsewhere": True, "here": True, "last": None},
    "/api/synopsis": {"default": {"harness": "claude", "model": "haiku"}, "picked": None},
    "/api/devices": [],
    "/api/sources": [],
    "/api/voices": [],
    "/api/apps": {"cards": {}},
    "/api/work": {"threads": threads()},
    "/api/tiles": tiles(),
    "/api/history": history(),
    "/api/usage": [],
    "/api/host": {"id": "stand-in", "name": "box", "assistant": "Libri", "version": "0", "uptime": 3600},
    "/api/pending": [],
    "/api/asks": [],
    "/api/scratchpad": {"id": None, "text": "", "working": False, "words": "", "thinking": "", "stopping": False, "queued": [], "memory": True, "asks": []},
    "/api/task": {"agents": [], "screen": None},
    # As the host answers it; a reply without the list is read again on every render.
    "/api/memories": {"memories": []},
    "/api/memory": {"state": "working", "reason": None, "since": None, "waiting": 0, "newest_summary": None, "latest_message": None, "model": "haiku"},
}

# Frames and long animation frames from now until `stop`; CDP gives layouts and styles.
RECORD = """() => {
  const r = window.__rec = { frames: [], loaf: [], on: true };
  const tick = (t) => { r.frames.push(t); if (r.on) requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
  r.obs = new PerformanceObserver((l) => { for (const e of l.getEntries()) r.loaf.push({ d: e.duration, block: e.blockingDuration, layout: e.styleAndLayoutStart ? e.startTime + e.duration - e.styleAndLayoutStart : 0 }); });
  r.obs.observe({ type: 'long-animation-frame' });
}"""
STOP = """() => {
  const r = window.__rec; r.on = false; r.obs.disconnect();
  const gaps = r.frames.slice(1).map((t, i) => t - r.frames[i]);
  return { gaps, loaf: r.loaf };
}"""
METRICS = ("LayoutCount", "RecalcStyleCount", "LayoutDuration", "RecalcStyleDuration", "ScriptDuration", "TaskDuration")


def metrics(cdp):
    got = {m["name"]: m["value"] for m in cdp.send("Performance.getMetrics")["metrics"]}
    return {k: got[k] for k in METRICS}


def measured(page, cdp, act, settle=500):
    """Runs `act` and the settling after it under the frame recorder."""
    page.wait_for_timeout(200)
    before = metrics(cdp)
    page.evaluate(RECORD)
    page.wait_for_timeout(50)
    act()
    page.wait_for_timeout(settle)
    got = page.evaluate(STOP)
    after = metrics(cdp)
    gaps = got["gaps"]
    dropped = sum(max(0, round(g / FRAME) - 1) for g in gaps)
    return {
        "frames": len(gaps),
        "dropped": dropped,
        "worst_ms": round(max(gaps or [0]), 1),
        "p95_ms": round(sorted(gaps)[int(len(gaps) * 0.95)] if gaps else 0, 1),
        "long_frames": len(got["loaf"]),
        "long_frame_ms": round(sum(e["d"] for e in got["loaf"]), 1),
        "layouts": int(after["LayoutCount"] - before["LayoutCount"]),
        "styles": int(after["RecalcStyleCount"] - before["RecalcStyleCount"]),
        "layout_ms": round((after["LayoutDuration"] - before["LayoutDuration"]) * 1000, 1),
        "style_ms": round((after["RecalcStyleDuration"] - before["RecalcStyleDuration"]) * 1000, 1),
        "script_ms": round((after["ScriptDuration"] - before["ScriptDuration"]) * 1000, 1),
    }


def context_for(browser, size, mobile=False):
    context = browser.new_context(viewport={"width": size[0], "height": size[1]}, service_workers="block", is_mobile=mobile, has_touch=mobile, reduced_motion="no-preference")
    context.set_default_timeout(60000)
    for section in ("waiting", "working", "sleeping", "archived"):
        context.add_init_script(f"localStorage.setItem('libri-work-open-{section}', 'on')")
    context.route_web_socket("**/api/terminal*", lambda ws: ws.on_message(lambda _: ws.send(b"ada@box ~ $ ")))
    return context


def ready(page, wide=True):
    page.wait_for_selector("textarea[name=input]", timeout=60000)
    page.wait_for_selector("text=Reply 59.", timeout=60000)
    if wide:
        page.wait_for_selector(f"aside[aria-label='Work'] >> text={WORKER}", timeout=60000)
    page.wait_for_timeout(800)


def press(page, label):
    page.locator(f'[aria-label="{label}"]').first.click()


def card(page, name):
    page.locator("article[data-sheet]").filter(has=page.locator(f"span.font-semibold:text-is('{name}')")).first.locator("button").first.click()


def throttled(page, rate):
    cdp = page.context.new_cdp_session(page)
    cdp.send("Performance.enable")
    cdp.send("Emulation.setCPUThrottlingRate", {"rate": rate})
    return cdp


def motion(browser, site, out):
    for label, size in {"1024": (1024, 768), "1440": (1440, 900)}.items():
        for rate in (1, 4):
            context = context_for(browser, size)
            page = context.new_page()
            page.goto(site + "/")
            ready(page)
            cdp = throttled(page, rate)
            row = out.setdefault(f"{label} cpu/{rate}", {})
            # Each opening once before it is measured, so first-use loading (the
            # terminal's chunk) is counted apart from the slide.
            row["terminal open (first)"] = measured(page, cdp, lambda: press(page, "Terminal"))
            row["terminal close"] = measured(page, cdp, lambda: press(page, "Close the terminal"))
            row["terminal open"] = measured(page, cdp, lambda: press(page, "Terminal"))
            press(page, "Close the terminal")
            page.wait_for_timeout(600)
            row["temp open"] = measured(page, cdp, lambda: press(page, "Temp chat, not remembered"))
            row["temp close"] = measured(page, cdp, lambda: press(page, "Close the temp chat"))
            row["thread open"] = measured(page, cdp, lambda: card(page, WORKER), settle=900)
            row["thread close"] = measured(page, cdp, lambda: card(page, WORKER))
            row["fold workers"] = measured(page, cdp, lambda: press(page, "Fold the workers column"))
            row["unfold workers"] = measured(page, cdp, lambda: press(page, "Unfold the workers column"))
            if page.locator('[aria-label="Fold the left column"]').count():
                row["fold left"] = measured(page, cdp, lambda: press(page, "Fold the left column"))
                row["unfold left"] = measured(page, cdp, lambda: press(page, "Unfold the left column"))
            grip = page.get_by_role("separator", name="Resize the side columns").first
            if grip.count():
                b = grip.bounding_box()
                x, y = b["x"] + b["width"] / 2, b["y"] + b["height"] / 2

                def drag():
                    page.mouse.move(x, y)
                    page.mouse.down()
                    for k in range(1, 25):
                        page.mouse.move(x + 8 * k, y)
                        page.wait_for_timeout(12)
                    for k in range(24, -1, -1):
                        page.mouse.move(x + 8 * k, y)
                        page.wait_for_timeout(12)
                    page.mouse.up()
                row["sash drag 288-480-288"] = measured(page, cdp, drag, settle=200)
            context.close()


def steady(browser, site, out):
    for label, size in {"1440": (1440, 900)}.items():
        for rate in (1, 4):
            context = context_for(browser, size)
            page = context.new_page()
            page.goto(site + "/")
            ready(page)
            cdp = throttled(page, rate)
            row = out.setdefault(f"{label} cpu/{rate}", {})
            talk = page.locator("[data-talk]").first
            b = talk.bounding_box()

            def scroll(dy, x=b["x"] + b["width"] / 2, y=b["y"] + b["height"] / 2):
                def run():
                    page.mouse.move(x, y)
                    for _ in range(30):
                        page.mouse.wheel(0, dy)
                        page.wait_for_timeout(16)
                return run
            row["scroll chat up"] = measured(page, cdp, scroll(-120))
            row["scroll chat down"] = measured(page, cdp, scroll(120))
            w = page.locator("aside[aria-label='Work']").first.bounding_box()
            row["scroll workers"] = measured(page, cdp, scroll(80, w["x"] + w["width"] / 2, w["y"] + w["height"] / 2))
            box = page.locator("textarea[name=input]").first
            box.click()
            # Each key's own delay from press to paint, as the Event Timing API keeps it.
            page.evaluate("""() => { window.__keys = []; new PerformanceObserver((l) => { for (const e of l.getEntries()) if (e.name === 'keydown' || e.name === 'keypress' || e.name === 'input') window.__keys.push(e.duration); }).observe({ type: 'event', durationThreshold: 16, buffered: false }); }""")
            row["typing 80 keys"] = measured(page, cdp, lambda: page.keyboard.type("The quick brown fox jumps over the lazy dog, then the slow one, then all of them. ", delay=30))
            keys = page.evaluate("window.__keys")
            row["typing 80 keys"]["keys_over_16ms"] = len(keys)
            row["typing 80 keys"]["worst_key_ms"] = max(keys or [0])
            page.keyboard.press("Control+a")
            page.keyboard.press("Delete")

            def stream():
                mid = f"m{time.time()}"
                send({"type": "RUN_STARTED"})
                send({"type": "TEXT_MESSAGE_START", "messageId": mid, "role": "assistant"})
                words = talk_text(3).split(" ")
                for k in range(0, len(words), 2):
                    send({"type": "TEXT_MESSAGE_CONTENT", "messageId": mid, "delta": " ".join(words[k:k + 2]) + " "})
                    page.wait_for_timeout(25)
                send({"type": "TEXT_MESSAGE_END", "messageId": mid})
            row["streaming reply"] = measured(page, cdp, stream, settle=600)
            send({"type": "RUN_FINISHED"})
            context.close()


def phone(browser, site, out):
    for rate in (1, 4):
        context = context_for(browser, (390, 844), mobile=True)
        page = context.new_page()
        page.goto(site + "/")
        ready(page, wide=False)
        cdp = throttled(page, rate)
        row = out.setdefault(f"phone cpu/{rate}", {})
        row["to Work"] = measured(page, cdp, lambda: page.get_by_role("button", name="Work", exact=True).first.click())
        page.wait_for_selector(f"article >> text={WORKER}")
        row["thread open"] = measured(page, cdp, lambda: card(page, WORKER), settle=900)
        context.close()


def load(browser, site, out):
    """Cold is a fresh browser profile; warm a second visit with the files kept."""
    for label, size, mobile, net in (("1440", (1440, 900), False, None), ("phone 4G cpu/4", (390, 844), True, (40, 20e6 / 8))):
        context = context_for(browser, size, mobile)
        page = context.new_page()
        cdp = page.context.new_cdp_session(page)
        cdp.send("Network.enable")
        if net:
            cdp.send("Network.emulateNetworkConditions", {"offline": False, "latency": net[0], "downloadThroughput": net[1], "uploadThroughput": net[1]})
            cdp.send("Emulation.setCPUThrottlingRate", {"rate": 4})
        for visit in ("cold", "warm"):
            start = time.time()
            page.goto(site + "/")
            ready(page, wide=not mobile)
            usable = round((time.time() - start) * 1000 - 800)
            t = page.evaluate("""() => { const n = performance.getEntriesByType('navigation')[0]; const p = Object.fromEntries(performance.getEntriesByType('paint').map((e) => [e.name, Math.round(e.startTime)]));
              const res = performance.getEntriesByType('resource');
              return { fcp: p['first-contentful-paint'], dcl: Math.round(n.domContentLoadedEventEnd), js_kb: Math.round(res.filter((r) => r.name.endsWith('.js')).reduce((a, r) => a + r.transferSize, 0) / 1024),
                css_kb: Math.round(res.filter((r) => r.name.endsWith('.css')).reduce((a, r) => a + r.transferSize, 0) / 1024),
                files: res.filter((r) => /\\.(js|css|woff2?|wasm)$/.test(r.name)).map((r) => [r.name.split('/').pop(), Math.round(r.transferSize / 1024), Math.round(r.startTime), Math.round(r.responseEnd)]),
                api: res.filter((r) => r.name.includes('/api/') && !r.name.includes('/api/events')).map((r) => [new URL(r.name).pathname, Math.round(r.startTime), Math.round(r.responseEnd)]) }; }""")
            t["usable_ms"] = usable
            out[f"{label} {visit}"] = t
        context.close()


def main():
    if PARTS[0] == "serve":
        server = http.server.ThreadingHTTPServer(("127.0.0.1", int(PARTS[1])), functools.partial(Host, directory=str(SITE)))
        server.daemon_threads = True
        return server.serve_forever()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Host, directory=str(SITE)))
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    report = {}
    with sync_playwright() as p:
        browser = p.chromium.launch(args=["--enable-gpu-rasterization", "--ignore-gpu-blocklist"])
        for part, run in (("load", load), ("motion", motion), ("steady", steady), ("phone", phone)):
            if part in PARTS:
                report[part] = {}
                run(browser, site, report[part])
                print(f"-- {part} done", file=sys.stderr, flush=True)
        browser.close()
    print(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
