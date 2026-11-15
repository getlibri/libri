#!/usr/bin/env python3
"""Every page of a phone's side panel fits its width: Hosts, and each Settings
category, at 390x844 in Chromium. Nothing on the page shown runs
past the panel's right edge, and no box in it scrolls sideways.

The stand-in host (measure_ui_performance.py) answers with what makes a page
wide: a host listening on several addresses, one an IPv6 one, and a pairing
request whose Approve and Decline share a row with its name. Motion is reduced,
so each page is read where it rests, not partway through its slide: a picture
taken during the 200 ms slide shows the page cut off, which it isn't (Settings
› Hosts, 2026-11-13). Needs Playwright with both browsers, so it stays out of
`nix flake check`.
Usage: check_settings_fit.py SITE_DIR SHOTS_DIR
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


sys.path.insert(0, str(Path(__file__).resolve().parent))
_argv, sys.argv = sys.argv, [sys.argv[0], str(SITE), "serve"]
import measure_ui_performance as stand_in  # noqa: E402

sys.argv = _argv

LAYOUT = {"spaces": [{"id": "main", "name": "Main", "items": [{"kind": "chat"}]}], "on": "main"}
HOST = {
    "id": "0f1e2d3c4b5a69788796a5b4c3d2e1f0",
    "name": "a-host-with-a-long-name",
    "assistant": "Libri",
    "version": "0.9.75",
    "commit": "7570d79",
    "uptime": 3600,
    "admits": True,
    "tools": True,
    "url": "https://assistant.example.org",
    "addresses": ["192.168.100.120:8951", "10.100.0.1:8951", "100.110.120.74:8951", "[fd7a:115c:a1e0:ab12:4843:cd96:6258:b240]:8951"],
}
PEERS = [
    {"url": "http://127.0.0.1:8952", "name": "Ann", "state": "asked", "last": None},
    {"url": "https://a-rather-long-address-for-a-peer.example.org:8951", "name": "", "state": "paired", "last": 1763070000},
]


class Host(stand_in.Host):
    """The layout stores answer once; the host and its peers make Hosts wide."""

    def do_GET(self):
        path = urlparse(self.path).path
        if path.endswith("/api/layout") or path.endswith("/api/phone-layout"):
            if "since=" in self.path:
                time.sleep(5)
            try:
                return self.json({"version": 1, "layout": LAYOUT, "by": None, "undo": False})
            except (BrokenPipeError, ConnectionResetError):
                return
        if path.endswith("/api/host"):
            return self.json(HOST)
        if path.endswith("/api/peers"):
            return self.json(PEERS)
        return super().do_GET()


# What runs past the right edge of the panel page shown, and what scrolls
# sideways in it. The page shown is the panel's region or nav that is not
# inert and not hidden.
OVER = """() => {
  const levels = [...document.querySelectorAll('aside [role=region], aside nav, [role=region][aria-label], nav[aria-label="Settings categories"]')]
    .filter((e) => !e.closest('[inert]') && e.getAttribute('aria-hidden') !== 'true' && e.offsetParent);
  const shown = levels.find((e) => e.getAttribute('aria-label') !== 'Side panel' || levels.length === 1) ?? levels[0];
  if (!shown) return { page: null };
  const edge = Math.min(innerWidth, shown.getBoundingClientRect().right);
  const name = (e) => e.tagName + (e.getAttribute('aria-label') ? `[${e.getAttribute('aria-label')}]` : '') + ' ' + (e.innerText || '').trim().split('\\n')[0].slice(0, 30);
  const past = [...shown.querySelectorAll('*')]
    .filter((e) => e.getClientRects().length && getComputedStyle(e).visibility !== 'hidden')
    .map((e) => [e, e.getBoundingClientRect()])
    .filter(([, r]) => r.width > 0 && r.right > edge + 0.5)
    .slice(0, 4)
    .map(([e, r]) => `${name(e)} to ${Math.round(r.right)}`);
  const sideways = [shown, ...shown.querySelectorAll('*')]
    .filter((e) => e.scrollWidth > e.clientWidth + 1 && /(auto|scroll)/.test(getComputedStyle(e).overflowX))
    .slice(0, 3)
    .map((e) => `${name(e)} ${e.scrollWidth}>${e.clientWidth}`);
  return { page: shown.getAttribute('aria-label'), edge: Math.round(edge), past, sideways };
}"""


DOCUMENT = "() => ({ wide: document.scrollingElement.scrollWidth, view: innerWidth })"


def fits(page, engine, what):
    page.wait_for_timeout(300)
    m = page.evaluate(OVER)
    if m.get("page") is None:
        # The entry opened a view of its own over the app, not a panel page:
        # the document as a whole must not scroll sideways.
        d = page.evaluate(DOCUMENT)
        check(f"{engine} {what}: the view it opens does not scroll sideways", d["wide"] <= d["view"] + 1, d)
    else:
        check(f"{engine} {what}: nothing runs past the panel's edge", not m.get("past"), m)
        check(f"{engine} {what}: nothing scrolls sideways", not m.get("sideways"), m)
    page.screenshot(path=str(SHOTS / f"{engine}-{what.lower().replace(' › ', '-').replace(' ', '-')}.png"))


def opened(context, site, errors):
    """A fresh page with the side panel open. Taps are forced: headless
    WebKit draws no frames, so the panel's slide never ends and nothing in
    it ever counts as still; with motion reduced, nothing moves anyway."""
    page = context.new_page()
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(site + "/")
    page.wait_for_selector("[data-surface=chat] textarea")
    page.wait_for_timeout(600)
    page.get_by_role("button", name="Side panel").first.tap(force=True)
    page.wait_for_timeout(300)
    return page


LISTED = """() => [...document.querySelectorAll('nav[aria-label="Settings categories"]')]
  .filter((n) => !n.closest('[inert]') && n.offsetParent)
  .flatMap((n) => [...n.querySelectorAll('button')].map((b) => b.innerText.trim().split('\\n')[0]))"""


def run(pw, site, engine):
    browser = getattr(pw, engine).launch()
    context = browser.new_context(viewport={"width": 390, "height": 844}, has_touch=True, is_mobile=engine == "chromium", service_workers="block", reduced_motion="reduce")
    context.set_default_timeout(15000)
    errors = []
    page = opened(context, site, errors)
    page.locator("button[data-open=hosts]").tap(force=True)
    fits(page, engine, "Hosts")
    page.close()
    page = opened(context, site, errors)
    page.locator("button[data-open=settings]").tap(force=True)
    page.wait_for_timeout(300)
    names = [n for n in page.evaluate(LISTED) if n]
    page.close()
    check(f"{engine}: Settings lists its pages", len(names) >= 5, names)
    # Each from a fresh page: an entry may open a view of its own, with no
    # way back to the list.
    for name in names:
        page = opened(context, site, errors)
        page.locator("button[data-open=settings]").tap(force=True)
        page.wait_for_timeout(300)
        page.locator('nav[aria-label="Settings categories"]:not([inert])').get_by_role("button", name=name, exact=False).first.tap(force=True)
        fits(page, engine, f"Settings › {name}")
        page.close()
    # Page errors are not asserted: the stand-in host answers some Settings
    # pages' calls with shapes they don't expect, which is no question of fit.
    context.close()
    browser.close()


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Host, directory=str(SITE)))
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    with sync_playwright() as pw:
        # Chromium only: the headless WebKit here draws no frames, so its
        # transitions never end and a screenshot waits for a frame forever;
        # it can't say where a page rests. check_side_panel covers WebKit's
        # touch handling, which needs no frame.
        for engine in ("chromium",):
            run(pw, site, engine)
    print(f"{total - len(failures)}/{total} passed")
    sys.exit(1 if failures else 0)


main()
