#!/usr/bin/env python3
"""The app as app.libri.is serves it, apart from any host, talking to a
host's plain-http daemon at a private address, as Paseo's app talks to its
daemon. The app is served both ways, since each engine reaches such a host
from one copy only: Chromium from the https copy, once Local Network Access
is granted (it refuses from the http copy, an insecure context), WebKit from
the http copy (it refuses plain http from a secure page). In each engine, at
a desktop and a phone width: the copy that works opens on the welcome, a host
added by its bare address is talked to, a chat turn goes there and its
answer comes back, and a host that does not list the app's origin
(LIBRI_ORIGINS) refuses it; the copy that cannot reach the host says why and
links to the other with the host list along; Chromium without the
permission says so. Headless Chromium cannot show its prompt, so the check
sets the permission as a person's Allow or Block does. The hosts are
throwaway daemons on ADDRESS, a non-loopback address of this machine (a
tailnet or LAN one), a stand-in Claude answering. Needs Playwright and its
browsers, so it stays out of `nix flake check`.

Usage: check_hosted_app.py LIBRI_DIR ADDRESS [SHOTS_DIR]
"""

import mimetypes
import os
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

LIBRI_DIR = Path(sys.argv[1])
SITE = LIBRI_DIR / "site"
ADDRESS = sys.argv[2]
SHOTS = Path(sys.argv[3]) if len(sys.argv) > 3 else None
SECURE, PLAIN = "https://app.libri.is", "http://app.libri.is"
STAND_IN = Path(__file__).with_name("stand_in_claude.py").read_text()
failures = []


def check(what, ok, detail=""):
    print(f"ok   {what}" if ok else f"FAIL {what} {detail}", flush=True)
    if not ok:
        failures.append(what)


def free_port():
    with socket.socket() as s:
        s.bind((ADDRESS, 0))
        return s.getsockname()[1]


def host(root: Path, name: str, origins: str):
    """A throwaway daemon in `root`, none of this shell's Libri or hippo
    settings inherited."""
    root.mkdir()
    claude = root / "claude"
    claude.write_text(f"#!{sys.executable}\n{STAND_IN}")
    claude.chmod(0o755)
    port = free_port()
    env = {k: v for k, v in os.environ.items() if not k.startswith(("HIPPO_", "LIBRI_", "STATE_", "XDG_"))}
    env |= {
        "HOME": str(root),
        "STATE_DIRECTORY": str(root / "state"),
        "STAND_IN_LOG": str(root / "stand-in.jsonl"),
        "LIBRI_NAME": name,
        "LIBRI_PERSON": "Ada",
        "LIBRI_BACKEND": "claude",
        "LIBRI_CLAUDE": str(claude),
        "LIBRI_MODEL": "stand-in",
        "HIPPO_BACKEND": "none",
        "LIBRI_SITE": f"{ADDRESS}:{port}",
        "LIBRI_SITE_URL": f"http://{ADDRESS}:{port}",
        "LIBRI_SITE_ROOT": str(SITE),
        "LIBRI_OPEN": "1",
        "LIBRI_ORIGINS": origins,
    }
    daemon = subprocess.Popen(
        [LIBRI_DIR / "bin/libri"], env=env, stdout=subprocess.DEVNULL, stderr=open(root / "libri.log", "w")
    )
    for _ in range(150):
        try:
            urllib.request.urlopen(f"http://{ADDRESS}:{port}/api/host", timeout=2)
            return daemon, f"http://{ADDRESS}:{port}"
        except OSError:
            time.sleep(0.2)
    raise TimeoutError(f"{name} up")


def serve_app(route):
    """The hosted app, at either scheme: the build's files, and nothing else
    (no API)."""
    path = route.request.url.split("://", 1)[1].split("/", 1)[1] if route.request.url.count("/") > 2 else ""
    path = path.split("?")[0].split("#")[0] or "index.html"
    file = (SITE / path).resolve()
    if SITE.resolve() not in file.parents or not file.is_file():
        return route.fulfill(status=404, body="Not found", content_type="text/plain")
    kind = mimetypes.guess_type(file.name)[0] or "application/octet-stream"
    route.fulfill(status=200, body=file.read_bytes(), content_type=kind)


def context_for(browser, **options):
    context = browser.new_context(**options)
    for app in (SECURE, PLAIN):
        context.route(f"{app}/**", serve_app)
    return context


def add(page, address):
    page.get_by_role("button", name="Add a host").click()
    page.get_by_label("Address or pairing link").fill(address)
    page.get_by_role("button", name="Add", exact=True).click()


def bare(url):
    return url.split("://", 1)[1]


def set_network(context, page, app, setting):
    """Local Network Access as a person's Allow or Block at the prompt sets it."""
    cdp = context.new_cdp_session(page)
    within = cdp.send("Target.getTargetInfo")["targetInfo"]["browserContextId"]
    for name in ("local-network-access", "local-network", "loopback-network"):
        cdp.send(
            "Browser.setPermission",
            {"permission": {"name": name}, "setting": setting, "origin": app, "browserContextId": within},
        )


def seen(page, text, what, timeout=20000):
    try:
        page.get_by_text(text).first.wait_for(timeout=timeout)
        check(what, True)
        return True
    except Exception as e:
        check(what, False, e)
        return False


def refused(browser, where, good):
    """Chromium at the https copy after a person blocks Local Network Access."""
    context = context_for(browser)
    page = context.new_page()
    set_network(context, page, SECURE, "denied")
    page.goto(SECURE + "/")
    add(page, bare(good))
    seen(page, "Local network access", f"{where}: blocked Local Network Access, the page says so")
    context.close()


def wrong_copy(browser, where, app, other, good, text):
    """The copy this engine cannot reach the host from says why, and its
    link opens the other copy with the host in its list."""
    context = context_for(browser)
    page = context.new_page()
    page.goto(app + "/")
    add(page, bare(good))
    if seen(page, text, f"{where}: {app} cannot reach the host and says why"):
        page.get_by_role("link", name="Open it there").click()
        page.wait_for_url(f"{other}/**", timeout=20000)
        page.get_by_text("Add a machine by its address").first.wait_for(timeout=20000)
        hosts = page.evaluate("localStorage.getItem('libri-hosts')") or ""
        check(f"{where}: its link opens {other} with the host listed", good in hosts, hosts)
    context.close()


def run(p, engine: str, width: int, height: int, good: str, refusing: str):
    where = f"{engine} {width}"
    browser = getattr(p, engine).launch()
    if engine == "chromium":
        app = SECURE
        refused(browser, where, good)
        wrong_copy(browser, where, PLAIN, SECURE, good, "only from the secure copy")
    else:
        app = PLAIN
        wrong_copy(browser, where, SECURE, PLAIN, good, "refuses a plain-http host")
    context = context_for(browser, viewport={"width": width, "height": height})
    page = context.new_page()
    if engine == "chromium":
        set_network(context, page, SECURE, "granted")
    origins, errors = [], []
    page.on("request", lambda r: r.url.startswith(good) and origins.append(r.headers.get("origin")))
    page.on("console", lambda m: m.type == "error" and errors.append(m.text))

    page.goto(app + "/")
    page.get_by_text("Add a machine by its address").wait_for(timeout=20000)
    check(f"{where}: {app} opens on the welcome", True)
    if SHOTS:
        page.screenshot(path=SHOTS / f"{engine}-{width}-welcome.png")

    # A host that does not list the app's origin cannot be read from it.
    add(page, bare(refusing))
    page.get_by_text("it does not let this page in").wait_for(timeout=20000)
    check(f"{where}: a host not allowing the origin refuses it, and the page says so", page.locator("textarea").count() == 0)
    page.get_by_title("Forget this host").first.click()

    # Typed bare, as a person types a tailnet address: http is meant.
    add(page, bare(good))
    page.wait_for_selector("textarea", timeout=30000)
    check(f"{where}: an allowed host is picked and its chat opens", page.evaluate("localStorage.getItem('libri-current')") == good)
    words = f"hello from {engine} at {width}"
    page.locator("textarea").first.fill(words)
    page.keyboard.press("Enter")
    seen(page, f"claude answers: {words}", f"{where}: a turn goes to the host and its answer comes back", 60000)
    sent = [o for o in origins if o]
    check(f"{where}: every call carried the app's origin", sent and set(sent) == {app}, set(sent))
    if SHOTS:
        page.screenshot(path=SHOTS / f"{engine}-{width}-chat.png")
    check(f"{where}: no blocked or failed loads", not [e for e in errors if "Mixed Content" in e or "CORS" in e or "blocked" in e], errors[:5])
    browser.close()


def main():
    if SHOTS:
        SHOTS.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        good, good_at = host(Path(tmp) / "rook", "Rook", f"{SECURE},{PLAIN}")
        refusing, refusing_at = host(Path(tmp) / "kit", "Kit", "")
        try:
            with sync_playwright() as p:
                for engine in ("chromium", "webkit"):
                    for width, height in ((1440, 900), (390, 844)):
                        try:
                            run(p, engine, width, height, good_at, refusing_at)
                        except Exception as e:
                            check(f"{engine} {width}: ran to the end", False, e)
        finally:
            for d in (good, refusing):
                d.terminate()
                d.wait()
    if failures:
        sys.exit(f"{len(failures)} failed: " + "; ".join(failures))
    print("all good")


main()
