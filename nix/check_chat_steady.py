#!/usr/bin/env python3
"""The chat holds still. Whatever is on the screen moves only when the person
scrolls or a streamed reply pushes it up: at rest nothing moves, while reading
higher up nothing moves as a reply streams below, and at the foot the text
only ever climbs. Each frame it records where a message on screen sits; a
step back down, or any step while reading, is the stutter seen on 11-05
("the chat is like microstuttering up and down", worse on an iPhone).
Runs the built site against measure_ui_performance.py's stand-in host, its
long chat with turns that open on a call (a fold whose margins once changed a
contained message's height, and the page kept remeasuring it), in Chromium
and WebKit at 1440 and 390. Needs Playwright and both
browsers, so it stays out of `nix flake check`.
Usage: check_chat_steady.py SITE_DIR [--report]
"""
import functools
import http.server
import json
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import measure_ui_performance as stand  # noqa: E402  (its host and chat)

from playwright.sync_api import sync_playwright  # noqa: E402

SITE = Path(sys.argv[1])
REPORT = "--report" in sys.argv[2:]
# Sub-pixel wobble from rounding is not a stutter a person sees.
STILL = 1.0

def history():
    """The stand-in's chat, every third turn worked through a call first,
    half of those ending on the call with no words after."""
    said = []
    for row in stand.history():
        n = (row["i"] - 1000) // 2
        if row["kind"] == "talk" and n % 3 == 1:
            said.append({**row, "i": row["i"] * 10, "kind": "tool", "text": 'Bash {"command":"cargo test","description":"Run the tests"}'})
            said.append({**row, "i": row["i"] * 10 + 1, "kind": "echo", "text": "test result: ok. 12 passed"})
        if not (row["kind"] == "talk" and n % 3 == 1 and n % 2):
            said.append({**row, "i": row["i"] * 10 + 2})
    return said


stand.Host.replies["/api/history"] = history()

# Each frame: the top of `pick` (a message chosen by `choose`) relative to the
# chat's viewport, and the viewport's scroll. `choose` runs once.
RECORD = """(how) => {
  const v = document.querySelector('[data-message-id]').closest('.overflow-y-auto');
  const ms = [...v.querySelectorAll('[data-message-id]')];
  const top = v.getBoundingClientRect().top;
  const pick = how === 'last'
    ? ms.at(-1)
    : ms.find((m) => m.getBoundingClientRect().top - top > 40) || ms[0];
  const r = window.__steady = { ys: [], tops: [], shifts: 0, on: true };
  const tick = () => {
    r.ys.push(pick.getBoundingClientRect().top - top);
    r.tops.push(v.scrollTop);
    if (r.on) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
  if (PerformanceObserver.supportedEntryTypes.includes('layout-shift')) {
    r.obs = new PerformanceObserver((l) => { for (const e of l.getEntries()) if (!e.hadRecentInput) r.shifts += e.value; });
    r.obs.observe({ type: 'layout-shift' });
  }
}"""
STOP = """() => { const r = window.__steady; r.on = false; r.obs?.disconnect(); return r; }"""


def summary(r):
    ys = r["ys"]
    steps = [b - a for a, b in zip(ys, ys[1:]) if abs(b - a) >= STILL]
    downs = [s for s in steps if s > 0]
    turns = sum(1 for a, b in zip(steps, steps[1:]) if (a > 0) != (b > 0))
    return {
        "frames": len(ys),
        "moves": len(steps),
        "moved_px": round(sum(abs(s) for s in steps), 1),
        "down_steps": len(downs),
        "down_px": round(sum(downs), 1),
        "reversals": turns,
        "layout_shift": round(r["shifts"], 4),
        # Scroll anchoring can hide a height change above the view by moving
        # the scroll to match, which still costs a redraw a frame.
        "scroll_moves": sum(1 for a, b in zip(r["tops"], r["tops"][1:]) if abs(b - a) >= STILL),
    }


def recorded(page, how, act, settle=1200):
    page.evaluate(RECORD, how)
    page.wait_for_timeout(50)
    act()
    page.wait_for_timeout(settle)
    return summary(page.evaluate(STOP))


def stream(page, chunks=200, gap=30):
    def run():
        mid = f"m{time.time()}"
        stand.send({"type": "RUN_STARTED"})
        stand.send({"type": "TEXT_MESSAGE_START", "messageId": mid, "role": "assistant"})
        words = " ".join(stand.talk_text(n) for n in (3, 5, 7)).split(" ")
        for k in range(0, min(len(words), chunks * 3), 3):
            stand.send({"type": "TEXT_MESSAGE_CONTENT", "messageId": mid, "delta": " ".join(words[k:k + 3]) + " "})
            page.wait_for_timeout(gap)
        stand.send({"type": "TEXT_MESSAGE_END", "messageId": mid})
        stand.send({"type": "RUN_FINISHED"})
    return run


def viewport_of(page):
    return page.locator(".overflow-y-auto:has(> main [data-message-id])").first


def runs(browser, site, size, mobile):
    out = {}
    context = browser.new_context(viewport={"width": size[0], "height": size[1]}, service_workers="block", is_mobile=mobile and browser.browser_type.name != "firefox", has_touch=mobile)
    context.set_default_timeout(60000)
    page = context.new_page()
    page.goto(site + "/")
    page.wait_for_selector("textarea[name=input]")
    page.wait_for_selector("text=Reply 59.")
    page.wait_for_timeout(1500)
    out["idle at foot"] = recorded(page, "first", lambda: page.wait_for_timeout(3000), settle=200)
    out["streaming at foot"] = recorded(page, "last", stream(page))
    page.wait_for_timeout(800)
    # Reading higher up: a long way up, as in a real chat. The person's own
    # scroll, as a drag of the scrollbar's thumb makes it (a press on the
    # viewport itself): the follow lets go of the foot only for a scroll a
    # person made, not one the page or the browser made.
    viewport_of(page).evaluate("""(v) => {
      v.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
      v.scrollTo({ top: v.scrollHeight * 0.45, behavior: 'instant' });
      setTimeout(() => window.dispatchEvent(new PointerEvent('pointerup')), 100);
    }""")
    page.wait_for_timeout(900)
    out["idle reading up"] = recorded(page, "first", lambda: page.wait_for_timeout(3000), settle=200)
    out["streaming reading up"] = recorded(page, "first", stream(page))
    context.close()
    return out


def bad(name, s):
    """What counts as a stutter in each run."""
    if name.startswith("idle") or name == "streaming reading up":
        return s["moved_px"] > 0 or s["scroll_moves"] > 0
    # At the foot, the text climbs as words arrive; it never steps back down.
    return s["down_steps"] > 0


def main():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(stand.Host, directory=str(SITE)))
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    report, failures = {}, []
    with sync_playwright() as p:
        for engine in (p.chromium, p.webkit):
            browser = engine.launch()
            for label, size, mobile in (("1440", (1440, 900), False), ("390", (390, 844), True)):
                got = runs(browser, site, size, mobile)
                report[f"{engine.name} {label}"] = got
                failures += [f"{engine.name} {label} {name}: {json.dumps(s)}" for name, s in got.items() if bad(name, s)]
            browser.close()
    if REPORT:
        print(json.dumps(report, indent=1))
    for f in failures:
        print("FAIL", f)
    print(f"check_chat_steady: {16 - len(failures)}/16 runs held still")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
