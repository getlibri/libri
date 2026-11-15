#!/usr/bin/env python3
"""What scrolls under floating chrome fades out instead of being cut off. A
chat (the main one, Temp) is wholly clear down to its dock's top edge and
shows on dimmed beneath the composer and the phone's bar (as Claude
Duet's), by two paper layers over the scroller (chat-edge-top and
chat-edge-foot) that stay put while the talk scrolls.
Each such scroll box carries its mask; scrolled to its foot, its last line
stays clear of the fade; and the composer still takes a tap. Runs the built site against a stand-in host in Chromium at 390 and
1440, in both themes, and leaves screenshots: each place at its foot and
scrolled a little way up, so lines show passing into the fade. Needs
Playwright and a browser, so it stays out of `nix flake check`.
Usage: check_edge_fade.py SITE_DIR SHOTS_DIR
"""
import functools
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


# The stand-in host of real size answers the API (measure_ui_performance.py).
sys.path.insert(0, str(Path(__file__).resolve().parent))
_argv, sys.argv = sys.argv, [sys.argv[0], str(SITE), "serve"]
import measure_ui_performance as stand_in  # noqa: E402
sys.argv = _argv

LINE = "The tide came in over the flats while Bob and Ada walked the harbour wall, counting boats that rode at anchor in the evening light."
# The phone's bar holds Temp too, so the check can open it there; a wide
# screen shows the chat and Temp side by side.
PHONE_LAYOUT = {"spaces": [{"id": "main", "name": "Main", "items": [{"kind": "chat"}, {"kind": "temp"}]}], "on": "main"}
WIDE_LAYOUT = {"spaces": [{"id": "main", "name": "Main", "focus": "c", "full": False, "root": {
    "t": "split", "id": "s", "dir": "row", "at": 0.6,
    "a": {"t": "leaf", "id": "c", "pane": {"kind": "chat"}}, "b": {"t": "leaf", "id": "t", "pane": {"kind": "temp"}}}}], "on": "main"}


class Host(stand_in.Host):
    """Temp holds a long talk, so it scrolls under its dock; the layout
    stores answer once, and a wait for their next version waits, as the real
    host's does, so the page does not ask in a loop."""

    def do_GET(self):
        path = urlparse(self.path).path
        if path.endswith("/api/layout") or path.endswith("/api/phone-layout"):
            if "since=" in self.path:
                time.sleep(5)
            layout = PHONE_LAYOUT if path.endswith("/api/phone-layout") else WIDE_LAYOUT
            try:
                return self.json({"version": 1, "layout": layout, "by": None, "undo": False})
            except (BrokenPipeError, ConnectionResetError):
                return
        if path.endswith("/api/scratchpad"):
            return self.json({"id": "scratchpad-t1", "text": "", "working": False, "words": "", "thinking": "", "stopping": False,
                              "queued": [f"{n}. {LINE}" for n in range(12)], "memory": True, "asks": []})
        return super().do_GET()


# The visible scroll box under `where` that carries the fade, its mask, and
# how far its last child's foot sits above the box's foot less the fade.
FADED = """(where) => {
  const boxes = [...document.querySelectorAll(where + ' .edge-fade, ' + where + ' [data-chat-scroll]')].filter((e) => e.offsetParent && e.scrollHeight > e.clientHeight);
  const box = boxes[0];
  if (!box) return null;
  box.scrollTop = box.scrollHeight;
  const cs = getComputedStyle(box);
  const chat = box.matches('[data-chat-scroll]');
  const column = box.parentElement;
  const edge = (k) => column.querySelector(':scope > [data-chat-edge=' + k + ']');
  const foot = chat ? edge('foot') : null;
  const top = chat ? edge('top') : null;
  const fcs = foot && getComputedStyle(foot);
  // A chat's clear band ends at its foot layer's top edge; a list's at its fade.
  const fade = chat ? (foot ? foot.getBoundingClientRect().height : 0) : parseFloat(cs.getPropertyValue('--fade-bottom')) || 0;
  const leaves = [...box.querySelectorAll('p, li, button, h2, h3')].filter((e) => e.offsetParent && e.getBoundingClientRect().height > 0);
  const lastFoot = Math.max(...leaves.map((e) => e.getBoundingClientRect().bottom));
  const boxFoot = box.getBoundingClientRect().bottom;
  return {
    mask: chat ? (top && getComputedStyle(top).backgroundImage) : cs.maskImage || cs.webkitMaskImage,
    scrollerMask: cs.maskImage || cs.webkitMaskImage,
    footFill: fcs && fcs.backgroundImage,
    footUnder: foot && Math.abs(foot.getBoundingClientRect().bottom - boxFoot) < 1 && fcs.pointerEvents === 'none',
    fade, scrim: chat, clear: boxFoot - fade - lastFoot,
  };
}"""


def faded(page, where, label):
    m = page.evaluate(FADED, where)
    check(f"{label}: a scroll box carries the fade", m is not None and "gradient" in (m["mask"] or ""), str(m))
    if m:
        check(f"{label}: scrolled to its foot, the last line is clear of the fade", m["clear"] >= -0.5, f"clear={m['clear']:.1f} fade={m['fade']}")
    return m


def scrimmed(page, where, label):
    """A chat: clear to the dock, the talk at 15% beneath the composer."""
    m = faded(page, where, label)
    if m:
        check(f"{label}: the chat dims beneath its dock (85% paper, at the box's foot, untouchable)", m["scrim"] and "0.85)" in (m["footFill"] or "") and m["footUnder"], str(m))
        check(f"{label}: no mask on the scroller, which a compositor scroll would carry", m["scrollerMask"] in (None, "", "none"), str(m))
    return m


def lift(page, where, by):
    """Scrolls the faded box `by` px up from its foot by the wheel, as a
    person does (the chat's foot hold lets go only then), so lines pass
    under the dock."""
    box = page.evaluate(
        "(where) => { const b = [...document.querySelectorAll(where + ' .edge-fade, ' + where + ' [data-chat-scroll]')].find((e) => e.offsetParent && e.scrollHeight > e.clientHeight); if (!b) return null; const r = b.getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 3]; }",
        where,
    )
    if box:
        page.mouse.move(*box)
        page.mouse.wheel(0, -by)
    page.wait_for_timeout(400)


def composer_takes_taps(page, label):
    box = page.locator("textarea").first.bounding_box()
    hit = page.evaluate("([x, y]) => document.elementFromPoint(x, y)?.tagName", [box["x"] + box["width"] / 2, box["y"] + box["height"] / 2])
    check(f"{label}: the composer still takes a tap", hit == "TEXTAREA", str(hit))


def run(browser, site, width, theme):
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
    page.wait_for_selector("[data-surface=chat] [data-chat-scroll]")
    page.wait_for_timeout(1500)
    tag = f"{width}-{theme}"
    chat, temp = "[data-surface=chat]", "[data-surface=temp]"
    scrimmed(page, chat, f"{tag} chat")
    composer_takes_taps(page, f"{tag} chat")
    page.screenshot(path=SHOTS / f"chat-{tag}-foot.png")
    lift(page, chat, 220)
    page.screenshot(path=SHOTS / f"chat-{tag}-scrolled.png")
    if phone:
        page.locator("[role=tablist] button[data-open=temp]").first.click()
    page.wait_for_selector(f"{temp} >> text=11. {LINE}")
    page.wait_for_timeout(500)
    scrimmed(page, temp, f"{tag} Temp")
    lift(page, temp, 220)
    page.screenshot(path=SHOTS / f"temp-{tag}-scrolled.png")
    check(f"{tag}: no page errors", not errors, "; ".join(errors))
    context.close()


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
                run(browser, site, width, theme)
        browser.close()
    print(f"{total - len(failures)}/{total} passed")
    sys.exit(1 if failures else 0)


main()
