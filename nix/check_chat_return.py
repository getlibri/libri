#!/usr/bin/env python3
"""The chat opens on its last line, and comes back to it. Opening the page
while a turn runs, or coming back to it mid-turn, leaves no blank under the
talk: on 11-07 a desktop reloaded mid-turn kept a 1561px blank under the last
line, and a page come back to after a streamed turn kept thousands of px, the
talk scrolled off the top of the screen ("chat scrolls way up off the page").
Mid-turn: a reload with the turn running, and a turn streamed and then the page
hidden and shown, its history read back laid out shorter than the stream had
it (the record lacks the turn's rows). Idle, no turn running: a reload, the
page hidden and shown, and a turn that ended and then the page hidden and
shown, its history read back shorter than the stream (on main that kept
377-549px of blank under the last line, the talk pushed up off the screen).
And the screen made shorter under an idle chat, as a keyboard or Safari's
toolbar does (0.9.53 hid 254px of the last message under the composer).
After each, the spacer under the talk
is gone and the last message stands in view at the foot.
Runs the built site against measure_ui_performance.py's stand-in host in
Chromium and WebKit at 1440 and 390. Needs Playwright and both browsers, so it
stays out of `nix flake check`.
Usage: check_chat_return.py SITE_DIR
"""
import functools
import http.server
import sys
import threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import measure_ui_performance as stand  # noqa: E402  (its host and chat)

from playwright.sync_api import sync_playwright  # noqa: E402

SITE = Path(sys.argv[1])
VIEWPORT = ".overflow-y-auto:has(> main [data-message-id])"
# The spacer, and how far the last message's foot stands above the
# composer's top: the talk runs on under the composer, so the view's own foot
# is under it too.
STATE = """() => {
  const v = document.querySelector('%s');
  const s = v.querySelector(':scope > main').lastElementChild;
  const last = [...v.querySelectorAll('[data-message-id]')].at(-1);
  const input = document.querySelector('textarea[name=input]').closest('.floating');
  return { spacer: s.offsetHeight, below: Math.round(input.getBoundingClientRect().top - last.getBoundingClientRect().bottom) };
}""" % VIEWPORT
SHOW = """(state) => {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
  document.dispatchEvent(new Event('visibilitychange'));
}"""
ASKED = {"i": 1200, "kind": "user", "text": "Question 60: run the checks and tell me.", "date": "2026-11-01T13:00:00Z", "reaction": None}
TURN = {"agents": [], "typing": True, "turn": {"start": 1200, "seq": 0, "shown": []}}
# Above the composer: the talk's foot padding is 40px.
NEAR = 120


def turn():
    """A turn's words and calls, ten of each, as the stream sends them."""
    stand.send({"type": "RUN_STARTED"})
    for k in range(10):
        stand.send({"type": "TEXT_MESSAGE_START", "messageId": f"t{k}", "role": "assistant"})
        stand.send({"type": "TEXT_MESSAGE_CONTENT", "messageId": f"t{k}", "delta": f"Step {k}: " + stand.talk_text(k)})
        stand.send({"type": "TEXT_MESSAGE_END", "messageId": f"t{k}"})
        stand.send({"type": "TOOL_CALL_START", "toolCallId": f"c{k}", "toolCallName": "Bash"})
        stand.send({"type": "TOOL_CALL_ARGS", "toolCallId": f"c{k}", "delta": '{"command":"cargo test","description":"Run the tests"}'})
        stand.send({"type": "TOOL_CALL_END", "toolCallId": f"c{k}"})
        stand.send({"type": "TOOL_CALL_RESULT", "toolCallId": f"c{k}", "content": f"test result: ok. {k} passed"})
        threading.Event().wait(0.25)


def open_page(browser, site, size, mobile):
    context = browser.new_context(viewport={"width": size[0], "height": size[1]}, service_workers="block", is_mobile=mobile and browser.browser_type.name == "chromium", has_touch=mobile)
    context.set_default_timeout(30000)
    page = context.new_page()
    page.goto(site + "/")
    page.wait_for_selector("text=Question 60")
    page.wait_for_timeout(1500)
    return context, page


def reloaded(browser, site, size, mobile):
    stand.Host.replies["/api/history"] = stand.history() + [ASKED]
    stand.Host.replies["/api/task"] = TURN
    context, page = open_page(browser, site, size, mobile)
    page.reload()
    page.wait_for_selector("text=Question 60")
    page.wait_for_timeout(2000)
    got = page.evaluate(STATE)
    context.close()
    return got


def returned(browser, site, size, mobile):
    stand.Host.replies["/api/history"] = stand.history() + [ASKED]
    stand.Host.replies["/api/task"] = {"agents": [], "typing": False}
    context, page = open_page(browser, site, size, mobile)
    turn()
    page.wait_for_timeout(800)
    stand.Host.replies["/api/task"] = TURN
    page.evaluate(SHOW, "hidden")
    page.wait_for_timeout(500)
    page.evaluate(SHOW, "visible")
    page.wait_for_timeout(2000)
    got = page.evaluate(STATE)
    context.close()
    return got


def idle(browser, site, size, mobile, back):
    stand.Host.replies["/api/history"] = stand.history() + [ASKED]
    stand.Host.replies["/api/task"] = {"agents": [], "typing": False}
    context, page = open_page(browser, site, size, mobile)
    if back == "reload":
        page.reload()
        page.wait_for_selector("text=Question 60")
    else:
        if back == "after a turn":
            turn()
            stand.send({"type": "RUN_FINISHED"})
            page.wait_for_timeout(1500)
        page.evaluate(SHOW, "hidden")
        page.wait_for_timeout(400)
        page.evaluate(SHOW, "visible")
    page.wait_for_timeout(2500)
    got = page.evaluate(STATE)
    context.close()
    return got


def shorter(browser, site, size, mobile):
    """The screen 300px shorter with the talk unchanged, as an iPhone's
    keyboard or Safari's toolbar makes it: on main the foot slid under the
    composer and stayed there, 254px of the last message hidden."""
    stand.Host.replies["/api/history"] = stand.history() + [ASKED]
    stand.Host.replies["/api/task"] = {"agents": [], "typing": False}
    context, page = open_page(browser, site, size, mobile)
    page.set_viewport_size({"width": size[0], "height": size[1] - 300})
    page.wait_for_timeout(800)
    got = page.evaluate(STATE)
    context.close()
    return got


RUNS = (
    ("reload mid-turn", reloaded),
    ("back to the page mid-turn", returned),
    ("idle reload", lambda *a: idle(*a, "reload")),
    ("idle back to the page", lambda *a: idle(*a, "hidden")),
    ("back to the page after a turn", lambda *a: idle(*a, "after a turn")),
    ("screen made shorter", shorter),
)


def main():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(stand.Host, directory=str(SITE)))
    server.daemon_threads = True
    # A page closed mid-request is no failure of the page.
    server.handle_error = lambda *_: None
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    failures, total = [], 0
    with sync_playwright() as p:
        for engine in (p.chromium, p.webkit):
            browser = engine.launch()
            for label, size, mobile in (("1440", (1440, 900), False), ("390", (390, 844), True)):
                for name, run in RUNS:
                    total += 1
                    got = run(browser, site, size, mobile)
                    if got["spacer"] > 1 or not 0 <= got["below"] <= NEAR:
                        failures.append(f"{engine.name} {label} {name}: {got}")
            browser.close()
    for f in failures:
        print("FAIL", f)
    print(f"check_chat_return: {total - len(failures)}/{total} opened on the last line")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
