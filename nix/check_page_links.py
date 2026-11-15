#!/usr/bin/env python3
"""A Library page's link opens in place, and a page published live opens by
itself (2026-11-06): a link to a page in the chat, at this host or at
app.libri.is, opens the page in the Library with no new tab, while any other
link still gets one; a page show_page publishes in the turn under way comes
up in the Library unasked; one published before the page loaded does not.

Drives the real daemon, with a stand-in Claude Code that publishes through
Libri's own tool server, and the built site in Chromium at 1440px and 390px.
Needs Playwright and its Chromium, so it stays out of `nix flake check`.
Usage: check_page_links.py LIBRI_DIR (holding bin/libri and site/) [SHOTS_DIR]
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

from playwright.sync_api import sync_playwright

LIBRI = Path(sys.argv[1]) / "bin/libri"
SITE = Path(sys.argv[1]) / "site"
SHOTS = Path(sys.argv[2]) if len(sys.argv) > 2 else None
failures = []

# Claude Code as Libri runs it, cut to what this check asks of it: "PUBLISH
# name" publishes a page through the tool server it was handed and says its
# link; "LINK name" only says the link, beside a link elsewhere.
STAND_IN = r'''
import json, subprocess, sys
args = sys.argv
if args[1:3] == ["auth", "status"]:
    sys.exit(0)
def say(ev):
    print(json.dumps(ev), flush=True)
def words(t):
    say({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": t}}})
def end(t):
    say({"type": "result", "subtype": "success", "is_error": False, "result": t, "session_id": "s", "usage": {}, "modelUsage": {}})
content = json.loads(sys.stdin.readline())["message"]["content"]
said = content if isinstance(content, str) else "".join(c.get("text", "") for c in content)
if "--safe-mode" in args:
    end("talk: stand-in summary")
    sys.exit()
last = said.strip().removesuffix("</orders>").strip().split("\n")[-1].split(": ", 1)[-1]
# The device's mark Libri puts before the words when it changes (origin.rs).
if last.startswith("(from ") and ") " in last:
    last = last.split(") ", 1)[1]
verb, _, name = last.partition(" ")
if verb == "PUBLISH":
    given = json.loads(args[args.index("--mcp-config") + 1])["mcpServers"]["libri"]
    proc = subprocess.Popen([given["command"], *given["args"]], env={**__import__("os").environ, **given["env"]}, text=True,
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    def call(n, method, params):
        proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": n, "method": method, "params": params}) + "\n")
        proc.stdin.flush()
        return json.loads(proc.stdout.readline())
    call(1, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "stand-in", "version": "0"}})
    proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
    proc.stdin.flush()
    page = {"name": name, "html": f"<h1 id='mark-{name}'>Page {name}</h1>"}
    say({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "p1", "name": "mcp__libri__show_page", "input": page}]}})
    got = call(2, "tools/call", {"name": "show_page", "arguments": page})["result"]["content"][0]["text"]
    say({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "p1", "content": got}]}})
    reply = "Made it."
elif verb == "LINK":
    reply = f"See [page {name}](https://app.libri.is/#page={name}) or [elsewhere](https://example.com/)."
else:
    reply = "stand-in: " + last
words(reply)
end(reply)
'''


def check(what, ok, detail=""):
    print(f"ok   {what}" if ok else f"FAIL {what} {detail}", flush=True)
    if not ok:
        failures.append(what)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def until(test, limit=20.0):
    end = time.time() + limit
    while time.time() < end:
        if test():
            return True
        time.sleep(0.2)
    return False


def showing(page, name):
    """Whether the page's own frame, inside the Library, is up."""
    for f in page.frames:
        try:
            if f.query_selector(f"#mark-{name}") is not None:
                return True
        except Exception:
            pass
    return False


def send(page, text):
    box = page.locator("textarea").first
    box.fill(text)
    box.press("Enter")


def main():
    root = Path(tempfile.mkdtemp(prefix="libri-page-links-"))
    for d in ("home", "state", "hippo"):
        (root / d).mkdir()
    claude = root / "claude"
    claude.write_text(f"#!{sys.executable}\n{STAND_IN}")
    claude.chmod(0o755)
    port = free_port()
    url = f"http://127.0.0.1:{port}"
    env = {k: v for k, v in os.environ.items() if not k.startswith(("HIPPO_", "LIBRI_", "STATE_", "XDG_"))}
    env |= {
        "HOME": str(root / "home"),
        "HIPPO_DIR": str(root / "hippo"),
        "HIPPO_BACKEND": "none",
        "STATE_DIRECTORY": str(root / "state"),
        "LIBRI_NAME": "box-label",
        "LIBRI_PERSON": "Ada",
        "LIBRI_PAGES": str(root / "state/pages"),
        "LIBRI_PAGES_URL": "https://app.libri.is",
        "LIBRI_BACKEND": "claude",
        "LIBRI_CLAUDE": str(claude),
        "LIBRI_MODEL": "stand-in",
        "LIBRI_SITE": f"127.0.0.1:{port}",
        "LIBRI_SITE_URL": url,
        "LIBRI_SITE_ROOT": str(SITE),
        "LIBRI_OPEN": "1",
    }
    daemon = subprocess.Popen([LIBRI], env=env, stdout=subprocess.DEVNULL, stderr=open(root / "libri.log", "w"))

    def up():
        try:
            return urllib.request.urlopen(f"{url}/api/host", timeout=2).status == 200
        except OSError:
            return False

    try:
        if not until(up, 30):
            raise SystemExit("libri never came up")
        with sync_playwright() as pw:
            browser = pw.chromium.launch()
            for width, height, label in ((1440, 900, "wide"), (390, 844, "phone")):
                context = browser.new_context(viewport={"width": width, "height": height})
                page = context.new_page()
                page.goto(url)
                page.locator("textarea").first.wait_for(timeout=20000)
                first = f"{label}a"
                send(page, f"PUBLISH {first}")
                check(f"{label}: a page published live opens by itself", until(lambda: showing(page, first)))
                if SHOTS:
                    page.screenshot(path=str(SHOTS / f"{label}-published.png"))
                # Back to the chat, then the reload: the history holds the
                # publishing, which must not open it again.
                page.goto(url)
                page.locator("textarea").first.wait_for(timeout=20000)
                time.sleep(3)
                check(f"{label}: the history opens no page", not showing(page, first))
                send(page, f"LINK {first}")
                link = page.get_by_role("link", name=f"page {first}")
                link.wait_for(timeout=20000)
                tabs = len(context.pages)
                link.click()
                check(f"{label}: a page link opens in the Library", until(lambda: showing(page, first)))
                time.sleep(1)
                check(f"{label}: and no new tab", len(context.pages) == tabs, f"{len(context.pages)} tabs")
                if SHOTS:
                    page.screenshot(path=str(SHOTS / f"{label}-link.png"))
                if label == "phone":
                    # The page lies over the chat; the chat again.
                    page.goto(url)
                    page.locator("textarea").first.wait_for(timeout=20000)
                other = page.get_by_role("link", name="elsewhere").last
                other.wait_for(timeout=20000)
                with context.expect_page(timeout=10000) as opened:
                    other.click()
                check(f"{label}: any other link gets a new tab", True)
                opened.value.close()
                context.close()
            browser.close()
    finally:
        daemon.terminate()
        daemon.wait()
    if failures:
        print(f"{len(failures)} failed; log in {root}/libri.log")
        sys.exit(1)
    print("all ok")


main()
