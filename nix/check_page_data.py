#!/usr/bin/env python3
"""A Library page both sides use (2026-11-06): the agent publishes a todo
page with show_page and fills it with page_data_change; the person opens it
in the Library, where it runs as an MCP App, and checks one item; the agent
reads that with page_data; the agent checks another, and the open page shows
it within a few seconds, with no reload.

Drives the real daemon and its real tool server (as a turn's harness reaches
it) in a scratch directory, and the built site in Chromium. Needs Playwright
and its Chromium, so it stays out of `nix flake check`.
Usage: check_page_data.py LIBRI_DIR (holding bin/libri and site/) [SHOTS_DIR];
CHECK_BROWSER=firefox or webkit runs it in that browser instead.
"""

import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import Error as PlaywrightError, sync_playwright

LIBRI = Path(sys.argv[1]) / "bin/libri"
SITE = Path(sys.argv[1]) / "site"
SHOTS = Path(sys.argv[2]) if len(sys.argv) > 2 else None
TODO = (Path(__file__).parent / "todo_page.html").read_text()
failures = []


def check(what, ok, detail=""):
    print(f"ok   {what}" if ok else f"FAIL {what} {detail}", flush=True)
    if not ok:
        failures.append(what)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def until(test, what, limit=30.0):
    end = time.time() + limit
    while time.time() < end:
        if test():
            return
        time.sleep(0.1)
    raise SystemExit(f"timed out: {what}")


class Tools:
    """Libri's tools at the person's scope, as a turn's harness reaches them."""

    def __init__(self, env, state):
        self.proc = subprocess.Popen([LIBRI, "tools", str(state)], env=env | {"LIBRI_SCOPE": "person"},
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.id = 0
        self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                "clientInfo": {"name": "check", "version": "0"}})
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.proc.stdin.flush()

    def rpc(self, method, params):
        self.id += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        return json.loads(self.proc.stdout.readline())

    def call(self, name, arguments):
        r = self.rpc("tools/call", {"name": name, "arguments": arguments})["result"]
        return r["content"][0]["text"], r.get("isError", False)


def page_frame(page):
    """The page's own frame: inside the sandbox proxy, inside the Library."""
    for f in page.frames:
        try:
            if f.query_selector("#items") is not None:
                return f
        except Exception:
            pass
    return None


def ask_from(frame, text):
    """The page asks the host to put words in the chat (ui/message)."""
    frame.evaluate("""(text) => parent.postMessage({ jsonrpc: "2.0", id: 9998, method: "ui/message",
        params: { role: "user", content: [{ type: "text", text }] } }, "*")""", text)


def shown(page, text):
    """Whether the words stand anywhere on the page outside a composer."""
    return page.get_by_text(text).evaluate_all("(es) => es.filter((e) => e.tagName !== 'TEXTAREA').length") > 0


def composer(page):
    # The page's message opens the chat, which loads the page anew: a poll
    # landing mid-load finds no page to read and reads nothing.
    try:
        boxes = page.query_selector_all("textarea")
        return " ".join(b.input_value() for b in boxes)
    except PlaywrightError as e:
        if "Execution context was destroyed" not in str(e):
            raise
        return ""


def main():
    root = Path(tempfile.mkdtemp(prefix="libri-page-data-"))
    for d in ("home", "state", "hippo"):
        (root / d).mkdir()
    port = free_port()
    url = f"http://127.0.0.1:{port}"
    env = {
        "HOME": str(root / "home"),
        "PATH": os.environ["PATH"],
        "HIPPO_DIR": str(root / "hippo"),
        "STATE_DIRECTORY": str(root / "state"),
        "LIBRI_NAME": "box-label",
        "LIBRI_PERSON": "Ada",
        "LIBRI_PAGES": str(root / "state/pages"),
        "LIBRI_PAGES_URL": url,
    }
    daemon = subprocess.Popen([LIBRI], env=env | {
        "LIBRI_BACKEND": "claude",
        "LIBRI_CLAUDE": "/bin/false",
        "LIBRI_MODEL": "stand-in",
        "LIBRI_SITE": f"127.0.0.1:{port}",
        "LIBRI_SITE_URL": url,
        "LIBRI_SITE_ROOT": str(SITE),
        "LIBRI_OPEN": "1",
    }, stdout=subprocess.DEVNULL, stderr=open(root / "libri.log", "w"))

    def up():
        try:
            return urllib.request.urlopen(f"{url}/api/host", timeout=2).status == 200
        except OSError:
            return False

    try:
        until(up, "libri up")
        tools = Tools(env, root / "state")
        said, bad = tools.call("show_page", {"name": "todo", "html": TODO})
        check("the agent publishes the page, linked in the Library", not bad and f"{url}/#page=todo" in said, said)
        items = {"milk": {"text": "Milk", "done": False, "at": 1}, "eggs": {"text": "Eggs", "done": False, "at": 2},
                 "bread": {"text": "Bread", "done": False, "at": 3}}
        said, bad = tools.call("page_data_change", {"page": "todo", "patch": {"items": items}})
        check("the agent fills it", not bad and json.loads(said)["version"] == 1, said)
        said, bad = tools.call("page_data", {"page": "nothing"})
        check("data of a page that isn't there is refused", bad, said)

        with sync_playwright() as p:
            browser = getattr(p, os.environ.get("CHECK_BROWSER", "chromium")).launch()
            page = browser.new_page(viewport={"width": 1440, "height": 900})
            page.goto(f"{url}/#page=todo")
            until(lambda: page_frame(page) is not None and page_frame(page).query_selector("li[data-key=milk]") is not None,
                  "the page shows its items", 30)
            f = page_frame(page)
            check("the page runs in the Library with the agent's three items", len(f.query_selector_all("li")) == 3)
            if SHOTS:
                SHOTS.mkdir(parents=True, exist_ok=True)
                page.screenshot(path=SHOTS / "1-opened.png")

            # The person checks Milk.
            f.click("li[data-key=milk] input")
            def person_seen():
                d = json.loads(tools.call("page_data", {"page": "todo"})[0])
                return d["data"]["items"]["milk"]["done"] and d["by"] == "person"
            until(person_seen, "the agent sees the person's check", 10)
            d = json.loads(tools.call("page_data", {"page": "todo"})[0])
            check("the agent sees the person checked Milk, and only Milk",
                  d["by"] == "person" and [k for k, v in d["data"]["items"].items() if v["done"]] == ["milk"], d)

            # The agent checks Bread; the open page shows it without a reload.
            t0 = time.time()
            tools.call("page_data_change", {"page": "todo", "patch": {"items": {"bread": {"done": True}}}})
            until(lambda: page_frame(page).query_selector("li[data-key=bread].done") is not None, "the page shows the agent's check", 30)
            took = time.time() - t0
            check(f"the page shows the agent's check live ({took:.2f} s)", took < 3, took)
            f = page_frame(page)
            check("both checks stand", f.query_selector("li[data-key=milk] input").is_checked()
                  and f.query_selector("li[data-key=bread] input").is_checked()
                  and not f.query_selector("li[data-key=eggs] input").is_checked())
            check("the page says the agent changed it last", "the agent" in f.inner_text("#said"), f.inner_text("#said"))
            if SHOTS:
                page.screenshot(path=SHOTS / "2-both-checked.png")

            # A page reaches only its own data: no other tool of Libri's.
            r = f.evaluate("""() => new Promise((ok) => {
                const i = 9999;
                addEventListener("message", (e) => { if (e.data && e.data.id === i) ok(e.data.result || e.data.error); });
                parent.postMessage({ jsonrpc: "2.0", id: i, method: "tools/call", params: { name: "show_page", arguments: { name: "x", html: "x" } } }, "*");
            })""")
            check("the page cannot call another of Libri's tools", bool(r.get("isError")) or "code" in r, r)
            check("nor did it publish anything", not (root / "state/pages/x").exists())

            # The page has the chat's own type: the host hands its fonts over.
            until(lambda: page_frame(page).evaluate("""() => [...document.fonts].some((x) => x.family.includes('Inter') && x.status === 'loaded')"""),
                  "Inter loads in the page", 15)
            check("the page draws in the chat's Inter", True)

            # A page's message waits in the composer; it is not sent.
            said = "Add oat milk to the list, please."
            ask_from(f, said)
            until(lambda: said in composer(page), "the page's message in the composer", 10)
            time.sleep(1.5)
            check("a page's message waits in the composer", said in composer(page), composer(page))
            check("and is not sent", not shown(page, said))

            # Phone width: the page opens over the chat.
            phone = browser.new_page(viewport={"width": 390, "height": 844})
            phone.goto(f"{url}/#page=todo")
            until(lambda: page_frame(phone) is not None and page_frame(phone).query_selector("li[data-key=bread].done") is not None,
                  "the page on a phone", 30)
            check("the page opens on a phone with the same data", True)
            if SHOTS:
                phone.screenshot(path=SHOTS / "3-phone.png")

            # The page alone (`#page-alone=`), as the Android app shows it:
            # its data live, its message to the app's own composer.
            alone = browser.new_page(viewport={"width": 390, "height": 844})
            alone.add_init_script("window.LibriApp = { postMessage: (t) => { (window.told = window.told || []).push(t); } };")
            alone.goto(f"{url}/#page-alone=todo")
            until(lambda: page_frame(alone) is not None and page_frame(alone).query_selector("li[data-key=bread].done") is not None,
                  "the page alone", 30)
            check("the page alone has no head and no chat", alone.query_selector("header") is None and alone.query_selector("textarea") is None)
            fa = page_frame(alone)
            fa.click("li[data-key=eggs] input")
            until(lambda: json.loads(tools.call("page_data", {"page": "todo"})[0])["data"]["items"]["eggs"]["done"], "the agent sees eggs", 10)
            check("the person's check on the page alone reaches the agent", True)
            t0 = time.time()
            tools.call("page_data_change", {"page": "todo", "patch": {"items": {"eggs": {"done": False}}}})
            until(lambda: page_frame(alone).query_selector("li[data-key=eggs]:not(.done)") is not None, "the page alone shows the agent's change", 30)
            check(f"the page alone shows the agent's change live ({time.time() - t0:.2f} s)", time.time() - t0 < 3)
            ask_from(fa, said)
            until(lambda: alone.evaluate("window.told || []") == [said], "the app hears the page's message", 10)
            check("the page alone hands its message to the app", True)
            if SHOTS:
                alone.screenshot(path=SHOTS / "4-alone.png")

            # The app's night mode comes in the address: the app's web view
            # keeps no storage. Applied, not stored.
            night = browser.new_page(viewport={"width": 390, "height": 844})
            night.goto(f"{url}/#page-alone=todo&theme=dark")
            until(lambda: page_frame(night) is not None and page_frame(night).query_selector("li[data-key=milk]") is not None, "the page alone by night", 30)
            dark = page_frame(night).evaluate("() => getComputedStyle(document.body).backgroundColor")
            check("by night the page alone draws dark", night.evaluate("document.documentElement.dataset.theme") == "dark"
                  and sum(int(c) for c in dark[dark.index("(") + 1:dark.index(")")].split(",")[:3]) < 200, dark)
            check("and keeps no theme of its own", night.evaluate("localStorage.getItem('theme')") is None)
            if SHOTS:
                night.screenshot(path=SHOTS / "5-alone-night.png")

            # In a browser tab, with no app around it, the message opens the chat with it in the composer.
            tab = browser.new_page(viewport={"width": 390, "height": 844})
            tab.goto(f"{url}/#page-alone=todo")
            until(lambda: page_frame(tab) is not None and page_frame(tab).query_selector("li[data-key=milk]") is not None, "the page alone in a tab", 30)
            ask_from(page_frame(tab), said)
            until(lambda: said in composer(tab), "the tab's chat composer has the message", 15)
            check("in a tab, the page's message opens the chat's composer with it, unsent", not shown(tab, said))
            browser.close()
    finally:
        daemon.terminate()
        daemon.wait()

    print(f"{'FAILED' if failures else 'passed'}: {len(failures)} failures", flush=True)
    sys.exit(1 if failures else 0)


main()
