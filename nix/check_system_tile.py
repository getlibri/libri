#!/usr/bin/env python3
"""The System tile and the `system` tool end to end: the real daemon samples
this machine, the built site's `#system-tile` page shows its CPU, memory,
disks, processes and services, and an agent's `system` tool answers the same
numbers in words. At desktop (1440) and phone (390, touch) sizes.

Drives the real daemon and its tool server in a scratch directory, and the
page in Playwright, so it stays out of `nix flake check`.
Usage: check_system_tile.py LIBRI_DIR (holding bin/libri and site/) [SHOTS_DIR].
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


def check(what, ok, detail=""):
    print(f"ok   {what}" if ok else f"FAIL {what} {detail}", flush=True)
    if not ok:
        failures.append(what)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def until(test, limit=30.0):
    end = time.time() + limit
    while time.time() < end:
        try:
            if test():
                return True
        except Exception:
            pass
        time.sleep(0.1)
    return False


def tool(env, state, name):
    """One call of an agent's tool, as a turn's harness makes it."""
    calls = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "check", "version": "0"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": name, "arguments": {}}},
    ]
    out = subprocess.run([LIBRI, "tools", str(state)], env=env | {"LIBRI_SCOPE": "person"},
                         input="".join(json.dumps(c) + "\n" for c in calls), capture_output=True, text=True, timeout=60).stdout
    for line in out.splitlines():
        msg = json.loads(line)
        if msg.get("id") == 2:
            r = msg["result"]
            return r["content"][0]["text"], r.get("isError", False)
    return out, True


def size(p, url, name, viewport, phone):
    b = p.chromium.launch()
    ctx = b.new_context(viewport=viewport, is_mobile=phone, has_touch=phone, device_scale_factor=2 if phone else 1)
    page = ctx.new_page()
    errors = []
    page.on("pageerror", lambda e: errors.append(str(e)))
    page.goto(f"{url}/#system-tile")
    tile = page.locator("[data-surface=system]")
    check(f"{name}: the tile shows the host's numbers",
          until(lambda: "CPU" in tile.inner_text() and "Memory" in tile.inner_text(), 20), tile.inner_text()[:200])
    text = tile.inner_text()
    for part in ("Swap", "Disks", "Busiest processes", "Libri"):
        check(f"{name}: the tile shows {part}", part in text)
    check(f"{name}: the disks show what is free", " free" in text, text[:300])
    # A second read lands: the sparklines draw once two samples are in.
    check(f"{name}: the sparklines draw", until(lambda: page.locator("[data-surface=system] polyline").count() >= 3, 20))
    wide = page.evaluate("() => document.documentElement.scrollWidth <= window.innerWidth + 1")
    check(f"{name}: nothing runs off the side", wide)
    check(f"{name}: no page errors", not errors, errors)
    if SHOTS:
        page.screenshot(path=SHOTS / f"{name}.png", full_page=True)
    ctx.close()
    b.close()


def main():
    if SHOTS:
        SHOTS.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="libri-system-tile-"))
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
        check("libri up", until(up))
        report = json.loads(urllib.request.urlopen(f"{url}/api/system", timeout=5).read())
        check("/api/system reports memory and cores", report["now"]["mem_total"] > 0 and report["cores"] > 0, report.get("now"))
        said, bad = tool(env, root / "state", "system")
        check("the system tool answers the host's memory", not bad and "memory" in said and "CPU" in said, said[:300])
        with sync_playwright() as p:
            size(p, url, "1440", {"width": 1440, "height": 900}, False)
            size(p, url, "390", {"width": 390, "height": 844}, True)
    finally:
        daemon.terminate()
        daemon.wait(10)
        print(f"log: {root / 'libri.log'}")
    if failures:
        raise SystemExit(f"{len(failures)} failed: {failures}")
    print("all passed")


main()
