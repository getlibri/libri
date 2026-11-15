#!/usr/bin/env python3
"""The Workers tree indents only what hangs from a worker: a memory's own
workers sit on its line, level with it, and each sub-worker steps in one
column a level, its node and its words alike. Folding a worker answers
within 400 ms. Runs the built site against a stand-in host in Chromium at
390 and 1440 in both themes. Needs Playwright and a browser, so it stays out
of `nix flake check`.
Usage: check_workers_indent.py SITE_DIR SHOTS_DIR
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
STEP = 22  # workTree.tsx DX


def check(what, ok, detail=""):
    print(f"{'PASS' if ok else 'FAIL'} {what} {'' if ok else str(detail)[:200]}", flush=True)
    if not ok:
        failures.append(what)


sys.path.insert(0, str(Path(__file__).resolve().parent))
_argv, sys.argv = sys.argv, [sys.argv[0], str(SITE), "serve"]
import measure_ui_performance as stand_in  # noqa: E402
sys.argv = _argv

# Live workers, a tree three deep under the first: two sub-workers, and one
# under the first of those.
THREADS = [t for t in stand_in.Host.replies["/api/work"]["threads"] if t["state"] != "archived"][:6]
HEAD, SUB, SUB2, DEEP = (t["id"] for t in THREADS[:4])
for t in THREADS:
    t["head"] = {SUB: HEAD, SUB2: HEAD, DEEP: SUB}.get(t["id"])
stand_in.Host.replies["/api/work"] = {**stand_in.Host.replies["/api/work"], "threads": THREADS}
DEPTH = {HEAD: 0, SUB: 1, SUB2: 1, DEEP: 2}


class Host(stand_in.Host):
    """Answers the layout stores as empty; a wait for their next version
    waits, as the real host's does."""

    def do_GET(self):
        path = urlparse(self.path).path
        if path.endswith("/api/layout") or path.endswith("/api/phone-layout"):
            if "since=" in self.path:
                time.sleep(5)
            try:
                return self.json({"version": 0, "layout": None, "by": None, "undo": False})
            except (BrokenPipeError, ConnectionResetError):
                return
        return super().do_GET()


# Each row's name's left edge and its node's centre, against the tree's own left.
PLACES = """() => {
  const tree = document.querySelector('[data-surface=worker-tree]');
  const left = tree.getBoundingClientRect().left;
  const svg = tree.querySelector('svg');
  const nodes = [...svg.children].filter((g) => g.tagName === 'g' && !g.dataset.lane)
    .map((g) => +g.querySelector('circle:last-of-type').getAttribute('cx'));
  return [...tree.querySelectorAll('[data-row]')].map((li, i) => ({
    key: li.dataset.row,
    words: Math.round(li.querySelector('button span span span').getBoundingClientRect().left - left),
    node: nodes[i],
  }));
}"""


def to_workers(page):
    page.goto(page.url.split("#")[0] + "#work")
    page.wait_for_selector("#workers-tab-tree, [data-item=workers]", state="attached")
    page.wait_for_timeout(800)
    if not page.locator("#workers-tab-tree").is_visible():
        page.locator("[data-item=workers]").first.click()
        page.wait_for_selector("#workers-tab-tree")


def run(browser, site, width, height, mobile, theme):
    tag = f"{width} {theme}"
    context = browser.new_context(viewport={"width": width, "height": height}, service_workers="block", is_mobile=mobile, has_touch=mobile)
    context.set_default_timeout(10000)
    context.add_init_script("localStorage.removeItem('libri-tree-folded')")
    if theme == "dark":
        context.add_init_script("localStorage.setItem('theme', 'dark')")
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(site + "/")
    to_workers(page)
    page.wait_for_selector("[data-row='memory:main']")
    page.wait_for_timeout(400)
    rows = {r["key"]: r for r in page.evaluate(PLACES)}
    main = rows["memory:main"]
    top = [t["id"] for t in THREADS if t["id"] not in DEPTH or DEPTH[t["id"]] == 0]
    check(f"{tag}: every row drawn", all(t["id"] in rows for t in THREADS), list(rows))
    check(f"{tag}: top-level workers' words sit level with Main's", all(rows[k]["words"] == main["words"] for k in top), {k: rows[k]["words"] for k in top} | {"main": main["words"]})
    check(f"{tag}: top-level workers' nodes sit on Main's line", all(rows[k]["node"] == main["node"] for k in top), {k: rows[k]["node"] for k in top} | {"main": main["node"]})
    for k, d in DEPTH.items():
        if d:
            check(f"{tag}: a depth-{d} sub-worker steps in {d} column(s)", rows[k]["words"] - main["words"] == d * STEP and rows[k]["node"] - main["node"] == d * STEP, rows[k])
    # The lane to a worker's sub-workers runs down their column; the
    # worker's own words start clear of it.
    for kid, head in ((SUB, HEAD), (DEEP, SUB)):
        check(f"{tag}: {head}'s words clear its sub-workers' lane", rows[head]["words"] >= rows[kid]["node"] + 10, (rows[head], rows[kid]))
    page.screenshot(path=str(SHOTS / f"indent-{width}-{theme}.png"), full_page=False)

    # Folding the worker with sub-workers answers within the 400 ms rule.
    button = page.locator(f"[data-row='{HEAD}'] button").first
    start = page.evaluate("performance.now()")
    button.click()
    page.wait_for_function(f"!document.querySelector(\"[data-row='{SUB}']\")")
    took = page.evaluate("performance.now()") - start
    check(f"{tag}: folding a worker answers in {took:.0f} ms (<400)", took < 400, took)
    button.click()
    page.wait_for_selector(f"[data-row='{SUB}']")
    check(f"{tag}: no page errors", not errors, errors)
    context.close()


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Host, directory=str(SITE)))
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for width, height, mobile in ((1440, 900, False), (390, 844, True)):
            for theme in ("light", "dark"):
                run(browser, site, width, height, mobile, theme)
        browser.close()
    print(f"{len(failures)} failed")
    sys.exit(1 if failures else 0)


main()
