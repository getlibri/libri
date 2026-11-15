#!/usr/bin/env python3
"""The Library as a gallery (2026-11-05): pages as cards, each with a still
of how it looks, grouped by when they changed, then the lists. The stills
are plain images the host renders once per version (snapshot.rs); no page
runs in the gallery, and a card holds its shape and its title until its
still arrives. Runs the built site against a stand-in host with 44 pages
and two lists, whose stills Chromium renders here with snapshot.rs's own
settings, in Chromium, Firefox and WebKit, at 390, 1024 and 1440 in both
themes, and scrolls the phone's gallery top to bottom, timing its frames.
Needs Playwright, browsers and `chromium` (or LIBRI_CHROMIUM), so it stays
out of `nix flake check`.
Usage: check_library_gallery.py SITE_DIR SHOTS_DIR [browser ...]
"""
import functools
import http.server
import json
import os
import subprocess
import tempfile
import sys
import threading
import time
from pathlib import Path
from urllib.parse import urlparse

from playwright.sync_api import sync_playwright

SITE, SHOTS = Path(sys.argv[1]), Path(sys.argv[2])
BROWSERS = sys.argv[3:] or ["chromium", "firefox", "webkit"]
SAID = "Your pages are on the Library's shelves."
NOW = int(time.time())
TOPICS = ["Freedom index", "Box memory", "Tide tables", "Working, as a book", "Daily todo", "Hardware deals", "Seafood, ranked",
          "Reading list", "Trip to Portsmouth", "Budget, November", "Garden plan", "Router setup", "Weekly brief", "Recipe: soda bread"]
HUES = [18, 140, 210, 35, 280, 95, 0, 190]
PAGES = []
for i in range(44):
    # Four today, eight this week, the rest earlier.
    ago = 600 + i * 2400 if i < 4 else (86400 * (1 + (i - 4) % 6) if i < 12 else 86400 * (8 + i * 3))
    PAGES.append({"name": f"page-{i:02d}", "title": f"{TOPICS[i % len(TOPICS)]}{'' if i < len(TOPICS) else f' ({i // len(TOPICS) + 1})'}", "changed": NOW - ago})
LISTS = [{"name": "Groceries", "items": ["eggs", "bread", "butter", "oats", "lemons", "coffee"]},
         {"name": "Books to read", "items": ["The Rings of Saturn", "Middlemarch"]}]
failures = []
total = 0


def check(what, ok, detail=""):
    global total
    total += 1
    if not ok:
        failures.append(what)
        print(f"FAIL {what} {detail}", flush=True)


def page_html(i, title):
    """A stand-in page as show_page makes them: a heading, some prose, a table
    or bars, in a colour of its own."""
    hue = HUES[i % len(HUES)]
    bars = "".join(f"<div style='height:18px;margin:6px 0;width:{30 + (i * 17 + k * 23) % 65}%;background:hsl({hue} 55% {45 + k * 6}%);border-radius:4px'></div>" for k in range(5))
    rows = "".join(f"<tr><td>Row {k}</td><td>{(i * 7 + k * 13) % 97}</td><td>{(i + k) % 5} m</td></tr>" for k in range(6))
    body = bars if i % 2 else f"<table style='border-collapse:collapse;width:100%'>{rows}</table>"
    return (f"<!doctype html><body style='margin:0;font:16px system-ui;color:#222;background:hsl({hue} 30% 97%)'>"
            f"<header style='padding:28px 32px;background:hsl({hue} 45% 32%);color:white'><h1 style='margin:0;font-size:34px'>{title}</h1>"
            f"<p style='margin:6px 0 0;opacity:.8'>Made for Ada</p></header><main style='padding:24px 32px'>"
            f"<p>A page the assistant made and keeps up to date, with the figures below.</p>{body}</main></body>")


def stills():
    """Each stand-in page's still, rendered as snapshot.rs renders, and one
    page left without, as a host shows before its render."""
    browser = os.environ.get("LIBRI_CHROMIUM") or "chromium"
    work = Path(tempfile.mkdtemp(prefix="libri-stills-"))
    made = {}
    for i, p in enumerate(PAGES[:-1]):
        (work / p["name"]).mkdir()
        (work / p["name"] / "index.html").write_text(page_html(i, p["title"]))
        out = work / f"{p['name']}.webp"
        subprocess.run([browser, "--headless", "--disable-gpu", "--hide-scrollbars", "--mute-audio", "--no-first-run",
                        f"--user-data-dir={work / '.browser'}", "--window-size=1024,768", "--force-device-scale-factor=0.5",
                        "--virtual-time-budget=3000", f"--screenshot={out}", f"file://{work / p['name'] / 'index.html'}"],
                       check=True, capture_output=True, timeout=60)
        made[p["name"]] = out.read_bytes()
    return made


STILLS = {}


def stand_in(context):
    replies = {
        "/api/me": {"name": "Libri", "personas": [], "voice": None, "models": False, "forgets": []},
        "/api/work": {"threads": []},
        "/api/history": [
            {"i": 10, "kind": "user", "text": "Where are my pages?", "date": "2026-11-05T08:43:00Z", "reaction": None},
            {"i": 11, "kind": "talk", "text": SAID, "date": "2026-11-05T08:44:00Z", "reaction": None},
        ],
        "/api/usage": [],
        "/api/host": {"id": "stand-in", "name": "box", "assistant": "Libri", "version": "0", "uptime": 3600},
        "/api/pending": [],
        "/api/asks": [],
        "/api/library": {"pages": PAGES, "lists": LISTS, "stills": True},
        "/api/scratchpad": {"id": None, "text": "", "working": False, "words": "", "thinking": "", "stopping": False, "queued": [], "memory": True, "asks": []},
        "/api/task": {"agents": [], "screen": None},
        "/api/memory": {"state": "working", "reason": None, "since": None, "waiting": 0, "newest_summary": None, "latest_message": None, "model": "haiku"},
    }

    def api(route):
        url = urlparse(route.request.url)
        path = url.path[url.path.index("/api/"):]
        if path == "/api/events":
            return route.fulfill(status=200, headers={"content-type": "text/event-stream"}, body=": open\n\n")
        if path.endswith("/snapshot"):
            still = STILLS.get(path.split("/")[3])
            return route.fulfill(status=200, content_type="image/webp", body=still) if still else route.fulfill(status=404)
        return route.fulfill(json=replies.get(path, {}))

    context.route("**/api/**", api)
    for i, p in enumerate(PAGES):
        context.route(f"**/{p['name']}/", lambda r, *_, i=i, t=p["title"]: r.fulfill(status=200, content_type="text/html", body=page_html(i, t)))


class Pages(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_):
        pass


def load(browser, site, size, theme, hash_="", mobile=False):
    opts = {"viewport": {"width": size[0], "height": size[1]}, "service_workers": "block", "reduced_motion": "reduce"}
    if mobile and browser.browser_type.name != "firefox":
        opts.update(is_mobile=True, has_touch=True)
    context = browser.new_context(**opts)
    context.set_default_timeout(30000)
    if theme == "dark":
        context.add_init_script("try { localStorage.setItem('theme', 'dark') } catch {}")
    stand_in(context)
    page = context.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(site + "/" + hash_)
    page.wait_for_selector(f"text={SAID}" if not hash_ else "section[aria-label=Today]")
    return context, page, errors


def settle(page):
    """Waits for every still on screen to show, as the person would see it."""
    page.wait_for_function("""() => [...document.querySelectorAll('section img')].filter(i => {
        const r = i.getBoundingClientRect(); return r.bottom > 0 && r.top < innerHeight;
      }).every(i => i.complete)""", timeout=15000)
    page.wait_for_timeout(500)


# Frames per step of a scroll, timed in the page; also counts any frame
# mounted, which the gallery should never hold.
MEASURE = """async (step) => {
  const list = document.querySelector('section[aria-label=Today]').closest('.overflow-y-auto');
  const times = []; let most = 0; let mostLoading = 0;
  await new Promise((done) => {
    let last = performance.now();
    const tick = (t) => {
      times.push(t - last); last = t;
      most = Math.max(most, document.querySelectorAll('section iframe').length);
      mostLoading = Math.max(mostLoading, [...document.querySelectorAll('section img')].filter(i => !i.complete).length);
      if (list.scrollTop + list.clientHeight >= list.scrollHeight - 1) return done();
      list.scrollTop += step;
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
  times.shift();
  const sorted = [...times].sort((a, b) => a - b);
  const q = (p) => sorted[Math.min(sorted.length - 1, Math.floor(p * sorted.length))];
  return { frames: times.length, median: q(0.5), p95: q(0.95), worst: sorted[sorted.length - 1],
           over33: times.filter(t => t > 33.4).length, over50: times.filter(t => t > 50).length, most, mostLoading, height: list.scrollHeight };
}"""


def phone(browser, site, theme, results):
    name = browser.browser_type.name
    where = f"{name} 390 {theme}"
    context, page, errors = load(browser, site, (390, 844), theme, "#library", mobile=True)
    check(f"{where}: Today groups the newest pages", page.locator("section[aria-label=Today] li").count() == 4)
    check(f"{where}: This week and Earlier follow", page.locator("section[aria-label='This week'] li").count() == 8 and page.locator("section[aria-label=Earlier] li").count() == 32)
    check(f"{where}: lists come last as cards", page.locator("section[aria-label=Lists] li").count() == 2)
    check(f"{where}: a long Library has a find box", page.get_by_role("searchbox", name="Find in the Library").is_visible())
    cols = page.evaluate("() => getComputedStyle(document.querySelector('section[aria-label=Today] ul')).gridTemplateColumns.split(' ').length")
    check(f"{where}: two columns on a phone", cols == 2, str(cols))
    check(f"{where}: no page runs in the gallery", page.locator("section iframe").count() == 0)
    check(f"{where}: every still loads lazily", page.evaluate("() => [...document.querySelectorAll('section img')].every(i => i.loading === 'lazy')"))
    shapes = page.evaluate("() => [...document.querySelectorAll('section[aria-label=Today] li button')].map(b => b.getBoundingClientRect().height)")
    settle(page)
    after = page.evaluate("() => [...document.querySelectorAll('section[aria-label=Today] li button')].map(b => b.getBoundingClientRect().height)")
    check(f"{where}: cards keep their shape as stills arrive", shapes == after, f"{shapes} {after}")
    painted = page.evaluate("() => [...document.querySelectorAll('section[aria-label=Today] img')].filter(i => i.naturalWidth > 0).length")
    check(f"{where}: the stills on screen paint", painted == 4, str(painted))
    page.screenshot(path=SHOTS / f"{name}-390-{theme}.png")
    if theme == "light":
        # 25px a frame is 1500px a second at 60 frames: a brisk flick.
        r = page.evaluate(MEASURE, 25)
        results[name] = r
        check(f"{where}: scrolling never mounts a page", r["most"] == 0, json.dumps(r))
        page.wait_for_timeout(1500)
        check(f"{where}: a page with no still yet keeps its tinted card", page.locator("section[aria-label=Earlier] li").last.locator("img").count() == 0)
        page.screenshot(path=SHOTS / f"{name}-390-{theme}-bottom.png")
        page.get_by_role("searchbox", name="Find in the Library").fill("tide")
        check(f"{where}: find narrows the gallery", page.locator("section li").count() == 3, str(page.locator("section li").count()))
        page.locator("section li button").first.click()
        page.wait_for_timeout(600)
        check(f"{where}: a card opens its page", page.locator("[data-sheet] iframe").count() == 1)
    check(f"{where}: no page errors", not errors, "; ".join(errors))
    context.close()


def wide(browser, site, size, theme):
    name = browser.browser_type.name
    where = f"{name} {size[0]} {theme}"
    context, page, errors = load(browser, site, size, theme)
    rail = page.locator('section[aria-label="Library"]').first
    unfold = page.get_by_role("button", name="Unfold the left column")
    if not rail.is_visible() and unfold.count() == 0:
        # WebKit's 1024 leaves less than 1024 beside its scrollbar: the
        # phone's layout, whose bar holds the Library.
        page.get_by_role("button", name="Library", exact=True).last.click()
    else:
        if not rail.is_visible():
            # A narrower wide screen folds the left column: its button lays it over the chat.
            unfold.click()
            rail = page.locator('section[aria-label="Library"]').last
        rail.locator("li").nth(4).wait_for()
        check(f"{where}: the rail shows the latest five", rail.locator("li").count() == 5)
        check(f"{where}: All counts the whole Library", rail.get_by_role("button", name="All 46").is_visible())
        page.wait_for_timeout(1500)
        page.screenshot(path=SHOTS / f"{name}-{size[0]}-{theme}-rail.png")
        rail.get_by_role("button", name="All 46").click()
    page.wait_for_selector("section[aria-label=Today]")
    settle(page)
    cols = page.evaluate("() => getComputedStyle(document.querySelector('section[aria-label=Today] ul')).gridTemplateColumns.split(' ').length")
    check(f"{where}: the gallery takes three columns or more", cols >= 3, str(cols))
    page.screenshot(path=SHOTS / f"{name}-{size[0]}-{theme}-all.png")
    check(f"{where}: no page errors", not errors, "; ".join(errors))
    context.close()


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Pages, directory=str(SITE)))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    site = f"http://127.0.0.1:{server.server_address[1]}"
    results = {}
    STILLS.update(stills())
    with sync_playwright() as p:
        for name in BROWSERS:
            browser = getattr(p, name).launch()
            for theme in ["light", "dark"]:
                phone(browser, site, theme, results)
                for size in [(1024, 768), (1440, 900)]:
                    wide(browser, site, size, theme)
            browser.close()
    for name, r in results.items():
        print(f"scroll 390 {name}: {r['frames']} frames over {r['height']}px, median {r['median']:.1f}ms, p95 {r['p95']:.1f}ms, worst {r['worst']:.1f}ms, >33ms {r['over33']}, >50ms {r['over50']}, frames mounted {r['most']}, most images loading {r['mostLoading']}")
    (SHOTS / "scroll.json").write_text(json.dumps(results, indent=1))
    print(f"{total - len(failures)}/{total} passed")
    sys.exit(1 if failures else 0)


main()
