#!/usr/bin/env python3
"""Where the chat stands against its foot, in numbers: after the page opens,
a reload, the page hidden and shown, a send, a reply streamed and ended, and
photos loading late. For each: `below`, how far the last message's foot
stands above the composer's top (the view scrolled too high when it is large,
the last line hidden when it is negative); `gap`, the scroll's distance from
its own foot; and `spacer`, the foot room useFootHold holds. Runs the built
site against measure_ui_performance.py's stand-in host in Chromium and WebKit
at 390x844. A measure, not a check: it fails nothing, and the numbers are
compared before and after a change. Needs Playwright and both browsers, so it
stays out of `nix flake check`.
Usage: measure_chat_foot.py SITE_DIR [SHOTS_DIR] [HISTORY_JSON]; a history
read from a host's /api/history stands in for the made-up one.
"""
import functools
import http.server
import json
import struct
import sys
import threading
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import measure_ui_performance as stand  # noqa: E402  (its host and chat)

from playwright.sync_api import sync_playwright  # noqa: E402

SITE = Path(sys.argv[1])
SHOTS = Path(sys.argv[2]) if len(sys.argv) > 2 else None
if len(sys.argv) > 3:
    KEPT = json.loads(Path(sys.argv[3]).read_text())
    stand.history = lambda: list(KEPT)
STATE = """() => {
  const v = document.querySelector('.overflow-y-auto:has(> main [data-message-id])');
  const s = v.querySelector(':scope > main').lastElementChild;
  const last = [...v.querySelectorAll('[data-message-id]')].at(-1);
  const input = document.querySelector('textarea[name=input]').closest('.floating');
  return {
    below: Math.round(input.getBoundingClientRect().top - last.getBoundingClientRect().bottom),
    gap: Math.round(v.scrollHeight - v.scrollTop - v.clientHeight),
    spacer: s.offsetHeight,
  };
}"""
SHOW = """(state) => {
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
  document.dispatchEvent(new Event('visibilitychange'));
}"""


def png(w, h):
    """A grey PNG of w x h, tall as a phone's photo is."""
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = b"".join(b"\0" + b"\x80" * w for _ in range(h))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 0, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


PNG = png(600, 900)
PHOTO = {"i": 1200, "kind": "user", "text": "(sent a photo: tide.png) What is this?", "date": "2026-11-01T13:00:00Z", "reaction": None}
DRAFT = "\n".join(f"Line {k} of a draft left from before, long enough to wrap." for k in range(6))
ANSWER = {"i": 1201, "kind": "talk", "text": "A tide chart, drawn by hand.", "date": "2026-11-01T13:01:00Z", "reaction": None}


def stream_reply(page, out, wait=0.12):
    stand.send({"type": "RUN_STARTED"})
    for k in range(6):
        stand.send({"type": "TEXT_MESSAGE_START", "messageId": f"r{k}", "role": "assistant"})
        stand.send({"type": "TEXT_MESSAGE_CONTENT", "messageId": f"r{k}", "delta": f"Step {k}: " + stand.talk_text(k)})
        stand.send({"type": "TEXT_MESSAGE_END", "messageId": f"r{k}"})
        stand.send({"type": "TOOL_CALL_START", "toolCallId": f"c{k}", "toolCallName": "Bash"})
        stand.send({"type": "TOOL_CALL_ARGS", "toolCallId": f"c{k}", "delta": '{"command":"cargo test","description":"Run the tests"}'})
        stand.send({"type": "TOOL_CALL_END", "toolCallId": f"c{k}"})
        stand.send({"type": "TOOL_CALL_RESULT", "toolCallId": f"c{k}", "content": f"test result: ok. {k} passed"})
        threading.Event().wait(wait)
    page.wait_for_timeout(600)
    out["while streaming"] = page.evaluate(STATE)
    stand.send({"type": "RUN_FINISHED"})


def open_page(browser, site, photos=False, draft=None):
    mobile = browser.browser_type.name == "chromium"
    context = browser.new_context(viewport={"width": 390, "height": 844}, service_workers="block", is_mobile=mobile, has_touch=True)
    context.set_default_timeout(30000)
    if photos:
        # Late, as a phone on mobile data has them.
        def late(route):
            threading.Event().wait(1.2)
            route.fulfill(status=200, content_type="image/png", body=PNG)
        context.route("**/api/file/**", late)
    if draft:
        context.add_init_script(f"localStorage.setItem('draft', {json.dumps(draft)})")
    page = context.new_page()
    page.goto(site + "/")
    page.wait_for_selector("textarea[name=input]")
    page.wait_for_selector("text=A tide chart" if photos else "[data-message-id]")
    return context, page


def shot(page, name):
    if SHOTS:
        SHOTS.mkdir(parents=True, exist_ok=True)
        page.screenshot(path=str(SHOTS / f"{name}.png"))


def run(browser, site):
    out = {}
    engine = browser.browser_type.name
    stand.Host.replies["/api/history"] = stand.history()
    stand.Host.replies["/api/task"] = {"agents": [], "typing": False}
    context, page = open_page(browser, site)
    page.wait_for_timeout(300)
    out["open 0.3s"] = page.evaluate(STATE)
    page.wait_for_timeout(2200)
    out["open 2.5s"] = page.evaluate(STATE)
    shot(page, f"{engine}-open")
    page.reload()
    page.wait_for_selector("[data-message-id]")
    page.wait_for_timeout(2500)
    out["reload"] = page.evaluate(STATE)
    page.evaluate(SHOW, "hidden")
    page.wait_for_timeout(400)
    page.evaluate(SHOW, "visible")
    page.wait_for_timeout(2500)
    out["hidden and shown"] = page.evaluate(STATE)
    # The screen's height changing under the page, as an iPhone's keyboard
    # or Safari's toolbar does, the talk itself unchanged.
    page.set_viewport_size({"width": 390, "height": 544})
    page.wait_for_timeout(800)
    out["screen 300px shorter"] = page.evaluate(STATE)
    page.set_viewport_size({"width": 390, "height": 844})
    page.wait_for_timeout(800)
    out["screen back"] = page.evaluate(STATE)
    page.fill("textarea[name=input]", "How does the run look?")
    page.keyboard.press("Enter")
    page.wait_for_timeout(1200)
    out["after send"] = page.evaluate(STATE)
    stream_reply(page, out)
    page.wait_for_timeout(1500)
    out["reply ended"] = page.evaluate(STATE)
    shot(page, f"{engine}-reply-ended")
    context.close()
    # A draft kept from before, which the composer grows to hold as the page
    # opens, under a talk already set at its foot.
    context, page = open_page(browser, site, draft=DRAFT)
    page.wait_for_timeout(2500)
    out["opened with a draft"] = page.evaluate(STATE)
    shot(page, f"{engine}-draft")
    page.fill("textarea[name=input]", "")
    page.wait_for_timeout(500)
    page.type("textarea[name=input]", "One line,")
    for _ in range(5):
        page.keyboard.press("Shift+Enter")
        page.keyboard.type("and one more,")
    page.wait_for_timeout(800)
    out["typed six lines"] = page.evaluate(STATE)
    context.close()
    stand.Host.replies["/api/history"] = stand.history() + [PHOTO, ANSWER]
    context, page = open_page(browser, site, photos=True)
    page.wait_for_timeout(300)
    out["photos loading"] = page.evaluate(STATE)
    page.wait_for_timeout(2500)
    out["photos loaded"] = page.evaluate(STATE)
    context.close()
    return out


def main():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(stand.Host, directory=str(SITE)))
    server.daemon_threads = True
    server.handle_error = lambda *_: None
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    report = {}
    with sync_playwright() as p:
        for engine in (p.chromium, p.webkit):
            browser = engine.launch()
            report[engine.name] = run(browser, site)
            browser.close()
    print(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
