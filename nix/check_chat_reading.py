#!/usr/bin/env python3
"""Reading higher up while a reply streams moves nothing. The person scrolls
up off the foot by hand (a wheel or a finger, not a script) while the turn
streams words, opens and ends a call, and shows the working dots; from then
on whatever is on their screen stays where it is until they come back down.
One run has a question come up above the composer meanwhile: the dock under
the talk grew, and scrolled the talk up by as much (190px) wherever the
person was. Back at the foot, the chat follows the reply again.
Runs the built site against measure_ui_performance.py's stand-in host in
Chromium and WebKit at 390 (touch) and 1440. Needs Playwright and both
browsers, so it stays out of `nix flake check`.
Usage: check_chat_reading.py SITE_DIR [--report]
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
STILL = 1.0
VIEWPORT = ".overflow-y-auto:has(> main [data-message-id])"

# The top of the message the reader looks at, each frame, from `start`.
RECORD = """() => {
  const v = document.querySelector('%s');
  const top = v.getBoundingClientRect().top;
  const pick = [...v.querySelectorAll('[data-message-id]')].find((m) => m.getBoundingClientRect().top - top > 40);
  const r = window.__read = { ys: [], on: true };
  const tick = () => { r.ys.push(pick.getBoundingClientRect().top - top); if (r.on) requestAnimationFrame(tick); };
  requestAnimationFrame(tick);
}""" % VIEWPORT
STOP = "() => { const r = window.__read; r.on = false; return r.ys; }"
ASK = {"id": "ask-1", "message": "Run the tests?", "metadata": {"tool": "Bash", "input": {"command": "cargo test", "description": "Run the tests"}}}
GAP = "(v) => v.scrollHeight - v.scrollTop - v.clientHeight"
# The least gap over 20 frames: a word lands between frames, and the follow
# takes it up in the next, so one reading can catch it in between.
FOLLOWED = """(v) => new Promise((done) => {
  let least = Infinity, n = 0;
  const tick = () => { least = Math.min(least, v.scrollHeight - v.scrollTop - v.clientHeight); if (++n < 20) requestAnimationFrame(tick); else done(least); };
  requestAnimationFrame(tick);
})"""


def turn(page, done, rounds=16, pace=0.1):
    """A turn as a harness streams one: words, a call that runs and ends,
    a pause on the dots, more words; until `done` is set."""
    stand.send({"type": "RUN_STARTED"})
    words = " ".join(stand.talk_text(n) for n in (3, 5)).split(" ")
    k = 0
    while not done.is_set() and k < rounds:
        mid = f"m{time.time()}"
        stand.send({"type": "TEXT_MESSAGE_START", "messageId": mid, "role": "assistant"})
        # A line at a time, so the talk grows every few frames, as a fast
        # model's list does (~250px a second at pace 0.1; at 0.02 it grows
        # nearly every frame, which a follow must still tell from the hand).
        for j in range(0, len(words), 6):
            stand.send({"type": "TEXT_MESSAGE_CONTENT", "messageId": mid, "delta": " ".join(words[j:j + 6]) + "\n\n"})
            time.sleep(pace)
        stand.send({"type": "TEXT_MESSAGE_END", "messageId": mid})
        call = f"c{time.time()}"
        stand.send({"type": "TOOL_CALL_START", "toolCallId": call, "toolCallName": "Bash"})
        stand.send({"type": "TOOL_CALL_ARGS", "toolCallId": call, "delta": '{"command":"cargo test","description":"Run the tests"}'})
        stand.send({"type": "TOOL_CALL_END", "toolCallId": call})
        time.sleep(0.5)
        stand.send({"type": "TOOL_CALL_RESULT", "toolCallId": call, "content": "test result: ok. 12 passed"})
        time.sleep(0.4)
        k += 1
    stand.send({"type": "RUN_FINISHED"})


def by_hand(page, mobile, dy, stroke=300, steps=15):
    """Scroll the chat by about `dy` as a person does: a finger's drags of
    `stroke` px over `steps` frames on a touch screen (raw touches; Chromium's
    synthesized scroll gesture left this scroller where it was), else wheel
    notches. Playwright has no touch for WebKit, whose 390 page is drawn
    without it."""
    box = page.locator(VIEWPORT).first.bounding_box()
    x, y = box["x"] + box["width"] / 2, box["y"] + box["height"] / 2
    # Inside the talk end to end, which a question's card above the
    # composer makes shorter.
    stroke = min(stroke, box["height"] * 0.6)
    strokes = max(1, round(abs(dy) / stroke))
    if mobile and page.context.browser.browser_type.name == "chromium":
        cdp = page.context.new_cdp_session(page)
        sign = 1 if dy < 0 else -1
        for _ in range(strokes):
            y0 = y - sign * stroke / 2
            cdp.send("Input.dispatchTouchEvent", {"type": "touchStart", "touchPoints": [{"x": x, "y": y0}]})
            for k in range(1, steps + 1):
                cdp.send("Input.dispatchTouchEvent", {"type": "touchMove", "touchPoints": [{"x": x, "y": y0 + sign * stroke * k / steps}]})
                page.wait_for_timeout(16)
            cdp.send("Input.dispatchTouchEvent", {"type": "touchEnd", "touchPoints": []})
            page.wait_for_timeout(60)
        cdp.detach()
    else:
        page.mouse.move(x, y)
        notch = 100 if steps > 10 else 1000
        for _ in range(max(1, round(abs(dy) / notch))):
            page.mouse.wheel(0, notch if dy > 0 else -notch)
            page.wait_for_timeout(steps * 4)


def run(browser, site, size, mobile, dy, pace, ask=False):
    out = {}
    context = browser.new_context(viewport={"width": size[0], "height": size[1]}, service_workers="block", is_mobile=mobile and browser.browser_type.name == "chromium", has_touch=mobile)
    context.set_default_timeout(60000)
    page = context.new_page()
    page.goto(site + "/")
    page.wait_for_selector("textarea[name=input]")
    page.wait_for_selector("text=Reply 59.")
    page.wait_for_timeout(1500)
    v = page.locator(VIEWPORT).first
    done = threading.Event()
    streaming = threading.Thread(target=turn, args=(page, done), kwargs={"pace": pace})
    streaming.start()
    page.wait_for_timeout(900)
    # Up off the foot while the talk grows under the hand: a short slow drag,
    # as one goes up to reread a line, or a long one.
    by_hand(page, mobile, -dy, stroke=min(dy, 300), steps=30 if dy < 300 else 15)
    page.wait_for_timeout(400)
    page.evaluate(RECORD)
    if ask:
        # A question comes up above the composer while the person reads: the
        # dock grows under the talk, and once scrolled the talk by as much.
        page.wait_for_timeout(1000)
        stand.send({"type": "RUN_FINISHED", "outcome": {"type": "interrupt", "interrupts": [ASK]}})
        page.wait_for_timeout(2500)
    else:
        page.wait_for_timeout(3500)
    ys = page.evaluate(STOP)
    steps = [b - a for a, b in zip(ys, ys[1:]) if abs(b - a) >= STILL]
    out["reading up"] = {"frames": len(ys), "moves": len(steps), "moved_px": round(sum(abs(s) for s in steps), 1), "gap": round(v.evaluate(GAP))}
    # Back down by hand: the chat follows the turn again.
    for _ in range(40):
        # Flicks: quick strokes that fling on.
        by_hand(page, mobile, min(v.evaluate(GAP) + 400, 4000), stroke=400, steps=4)
        page.wait_for_timeout(150)
        if v.evaluate(FOLLOWED) < 8:
            break
    reached = round(v.evaluate(FOLLOWED))
    # Still streaming: the reply's growth is followed.
    page.wait_for_timeout(1500)
    out["back at foot"] = {"reached": reached, "gap": round(v.evaluate(FOLLOWED)), "streaming": streaming.is_alive()}
    done.set()
    streaming.join()
    context.close()
    return out


def bad(name, s):
    if name == "reading up":
        return s["moved_px"] > 0 or s["gap"] < 60
    return s["gap"] > 8 or not s["streaming"]


def main():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(stand.Host, directory=str(SITE)))
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    report, failures = {}, []
    with sync_playwright() as p:
        for engine in (p.chromium, p.webkit):
            browser = engine.launch()
            for label, size, mobile in (("390", (390, 844), True), ("1440", (1440, 900), False)):
                for dy, pace, ask in ((200, 0.1, False), (900, 0.1, False), (200, 0.02, False), (300, 0.1, True)):
                    got = run(browser, site, size, mobile, dy, pace, ask)
                    if pace < 0.1:
                        # Too fast for a hand to catch on the way back down.
                        got.pop("back at foot")
                    case = f"{engine.name} {label} up {dy} pace {pace}" + (" ask" if ask else "")
                    report[case] = got
                    failures += [f"{case} {name}: {json.dumps(s)}" for name, s in got.items() if bad(name, s)]
            browser.close()
    if REPORT:
        print(json.dumps(report, indent=1))
    for f in failures:
        print("FAIL", f)
    print(f"check_chat_reading: {28 - len(failures)}/28 held")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
