#!/usr/bin/env python3
"""Drives the Android app's Workers tile tabs on an emulator against a
throwaway Libri (check_workers_tabs.py is the web's): three tabs, Workers,
Watches and Archived; Workers and Watches carry their counts on the label,
Archived none, and says its count once opened; each tab shows only its own.
Runs in the light and the dark theme and saves screenshots.

Runs inside `nix develop .#android-emulator` (adb and the emulator on PATH);
boots an emulator on SERIAL if none is up. The APK must be a debug build:
  drive_android_workers_tabs.py LIBRI_DIR APK OUT [SERIAL]
"""

import json
import re
import subprocess
import sys
import time
import urllib.request
from datetime import datetime, timedelta
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from smoke_android import boot  # noqa: E402
import smoke_android  # noqa: E402
from throwaway_libri import Throwaway, until  # noqa: E402

LIBRI_DIR, APK, OUT = sys.argv[1], sys.argv[2], Path(sys.argv[3])
SERIAL = sys.argv[4] if len(sys.argv) > 4 else "emulator-5616"
smoke_android.SERIAL = SERIAL
APP = "is.libri"
ACTIVITY = f"{APP}/su.libri.MainActivity"
OUT.mkdir(parents=True, exist_ok=True)
NOTE = "TABNOTE feed the cat"
failures = []


def check(what, ok, detail=""):
    print(f"{'PASS' if ok else 'FAIL'} {what} {'' if ok else str(detail)[:300]}", flush=True)
    if not ok:
        failures.append(what)


def adb(*args):
    return subprocess.run(["adb", "-s", SERIAL, *args], capture_output=True, text=True).stdout


def shot(name):
    png = subprocess.run(["adb", "-s", SERIAL, "exec-out", "screencap", "-p"], capture_output=True).stdout
    (OUT / name).write_bytes(png)


def dump():
    """The screen's texts in layout order, each with its bounds."""
    for _ in range(5):
        adb("shell", "uiautomator", "dump", "--compressed", "/data/local/tmp/ui.xml")
        xml = adb("shell", "cat", "/data/local/tmp/ui.xml")
        if "<hierarchy" in xml:
            return [(m[0] or m[1], tuple(map(int, m[2:]))) for m in re.findall(
                r'text="([^"]*)"[^>]*?content-desc="([^"]*)"[^>]*?bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', xml) if m[0] or m[1]]
        time.sleep(1)
    return []


def tap(nodes, label, last=False):
    hits = [b for t, b in nodes if t == label]
    if not hits:
        return False
    b = hits[-1] if last else hits[0]
    adb("shell", "input", "tap", str((b[0] + b[2]) // 2), str((b[1] + b[3]) // 2))
    time.sleep(2)
    return True


def tab_row(nodes):
    """The texts on the tab strip's line, left to right."""
    top = next((b for t, b in nodes if t == "Archived"), None)
    if not top:
        return []
    return [t for t, b in sorted(nodes, key=lambda n: n[1][0]) if abs(b[1] - top[1]) < 12 and b[0] < top[2] + 4]


class Agents:
    """Libri's tools at the main chat's scope, as its harness reaches them."""

    def __init__(self, rig):
        env = rig.env | {"LIBRI_SCOPE": "person", "STATE_DIRECTORY": str(rig.root / "state")}
        self.proc = subprocess.Popen([str(rig.libri), "tools", str(rig.root / "state")], env=env, text=True,
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.id = 0
        self.call("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "check", "version": "0"}})
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.proc.stdin.flush()

    def call(self, method, params):
        self.id += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        return json.loads(self.proc.stdout.readline())

    def tool(self, called, **a):
        return self.call("tools/call", {"name": called, "arguments": a})["result"]["content"][0]["text"]


def main():
    rig = Throwaway(LIBRI_DIR, prefix="libri-tabs-android-")
    agents = Agents(rig)

    def http(method, path, body=None):
        req = urllib.request.Request(rig.base + path, method=method, data=json.dumps(body).encode() if body is not None else None,
                                     headers={"content-type": "application/json"})
        return json.loads(urllib.request.urlopen(req, timeout=20).read() or b"null")

    booted = False
    try:
        agents.tool("spawn", tasks=[{"task": f"Check part {k}"} for k in range(3)])
        until(lambda: len(http("GET", "/api/work")["threads"]) >= 3, "three workers start", wait=60)
        until(lambda: all(t["state"] == "sleeping" for t in http("GET", "/api/work")["threads"]), "the workers finish", wait=120)
        threads = http("GET", "/api/work")["threads"]
        put = threads[:2]
        for t in put:
            http("POST", f"/api/work/{t['id']}/archive")
        until(lambda: sum(t["state"] == "archived" for t in http("GET", "/api/work")["threads"]) == 2, "two are archived", wait=30)
        at = (datetime.now() + timedelta(days=1)).strftime("%Y-%m-%d %H:%M")
        agents.tool("watch", when={"at": at}, then={"note": NOTE})
        until(lambda: len(http("GET", "/api/watches")["watches"]) == 1, "the watch is set", wait=30)
        live = [t["name"] for t in threads[2:]]
        archived = [t["name"] for t in put]
        print("live", live, "archived", archived, flush=True)

        booted = boot()
        adb("uninstall", APP)
        adb("install", APK)
        adb("reverse", f"tcp:{rig.port}", f"tcp:{rig.port}")
        prefs = f'<?xml version="1.0" encoding="utf-8" standalone="yes" ?>\n<map>\n<string name="address">{rig.base}</string>\n</map>\n'
        (rig.root / "libri.xml").write_text(prefs)
        adb("push", str(rig.root / "libri.xml"), "/data/local/tmp/libri.xml")
        adb("shell", "run-as", APP, "sh", "-c", "'mkdir -p shared_prefs && cp /data/local/tmp/libri.xml shared_prefs/libri.xml'")

        for theme in ("light", "dark"):
            adb("shell", "cmd", "uimode", "night", "yes" if theme == "dark" else "no")
            adb("shell", "am", "force-stop", APP)
            adb("shell", "am", "start", "-W", "-n", ACTIVITY)
            time.sleep(5)
            nodes = dump()
            if not any(t == "Watches" for t, _ in nodes):
                # Every worker, from the bar's plus: the Work page has no item.
                tap(nodes, "Add to Main")
                tap(dump(), "Workers", last=True)
                nodes = dump()
            if theme == "light":
                # A fresh install opens on Workers; the dark run finds the tab
                # the light run left, Archived, kept on the device.
                row = tab_row(nodes)
                check(f"{theme}: three tabs, counts on Workers and Watches only", row == ["Workers", str(len(live)), "Watches", "1", "Archived"], row)
                check(f"{theme}: Workers opens first", all(n in [t for t, _ in nodes] for n in live), [t for t, _ in nodes])
            else:
                check(f"{theme}: the tab picked stays on this device", f"{len(archived)} ARCHIVED" in [t for t, _ in nodes], [t for t, _ in nodes])
                tap(nodes, "Workers")
                nodes = dump()
            shot(f"android-tabs-{theme}-workers.png")

            tap(nodes, "Watches")
            nodes = dump()
            texts = [t for t, _ in nodes]
            check(f"{theme}: Watches shows the watch", any(NOTE in t for t in texts), texts)
            check(f"{theme}: Watches shows no archived worker", not any(n in texts for n in archived), texts)
            shot(f"android-tabs-{theme}-watches.png")

            tap(nodes, "Archived")
            nodes = dump()
            texts = [t for t, _ in nodes]
            check(f"{theme}: Archived says its count inside", f"{len(archived)} ARCHIVED" in texts, texts)
            check(f"{theme}: Archived shows every archived worker", all(n in texts for n in archived), texts)
            check(f"{theme}: Archived shows no watch and no live worker", not any(NOTE in t for t in texts) and not any(n in texts for n in live), texts)
            check(f"{theme}: Archived's label still has no count", tab_row(nodes)[-1:] == ["Archived"] and str(len(archived)) not in tab_row(nodes), tab_row(nodes))
            shot(f"android-tabs-{theme}-archived.png")
        crash = adb("logcat", "-d", "-s", "AndroidRuntime:E")
        check("no crash", "FATAL EXCEPTION" not in crash, crash[-400:])
    finally:
        adb("shell", "cmd", "uimode", "night", "no")
        agents.proc.terminate()
        rig.close()
        if booted:
            adb("emu", "kill")
    print(f"{len(failures)} failed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
