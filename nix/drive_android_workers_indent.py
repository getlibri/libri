#!/usr/bin/env python3
"""Drives the Android app's Workers tree on an emulator against a throwaway
Libri (check_workers_indent.py is the web's): a memory's own workers sit
level with it, and only a sub-worker steps in, one column. Runs in the light
and the dark theme and saves screenshots.

Runs inside `nix develop .#android-emulator` (adb and the emulator on PATH);
boots an emulator on SERIAL if none is up. The APK must be a debug build:
  drive_android_workers_indent.py LIBRI_DIR APK OUT [SERIAL]
"""

import json
import re
import subprocess
import sys
import time
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
STEP_DP = 20  # WorkTree.kt DX
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

    def spawn(self, task):
        return self.tool("spawn", tasks=[{"task": task}]).split()[1].strip(".,:")


def left(nodes, label):
    return next((b[0] for t, b in nodes if t == label), None)


def main():
    rig = Throwaway(LIBRI_DIR, prefix="libri-indent-android-")
    agents = Agents(rig)
    booted = False
    try:
        top = agents.spawn("Check part one")
        head = agents.spawn("ASK CHAT HEAD")
        said = [""]

        def approved():
            said[0] = agents.tool("answer_subworker_request", name=head, approve=True)
            return said[0].startswith("Started ")
        until(approved, f"{head} gets its sub-worker", wait=90)
        sub = said[0].split()[1].strip(".,:")
        print("top", top, "head", head, "sub", sub, flush=True)

        booted = boot()
        adb("uninstall", APP)
        adb("install", APK)
        adb("reverse", f"tcp:{rig.port}", f"tcp:{rig.port}")
        prefs = f'<?xml version="1.0" encoding="utf-8" standalone="yes" ?>\n<map>\n<string name="address">{rig.base}</string>\n</map>\n'
        (rig.root / "libri.xml").write_text(prefs)
        adb("push", str(rig.root / "libri.xml"), "/data/local/tmp/libri.xml")
        adb("shell", "run-as", APP, "sh", "-c", "'mkdir -p shared_prefs && cp /data/local/tmp/libri.xml shared_prefs/libri.xml'")
        dpi = int(adb("shell", "wm", "density").split()[-1])
        step = round(STEP_DP * dpi / 160)

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
            x = {n: left(nodes, n) for n in ("Main", top, head, sub)}
            check(f"{theme}: every row drawn", None not in x.values(), x)
            if None not in x.values():
                check(f"{theme}: top-level workers sit level with Main", x[top] == x["Main"] == x[head], x)
                check(f"{theme}: the sub-worker steps in one column", abs(x[sub] - x[head] - step) <= 2, (x, step))
            shot(f"android-indent-{theme}.png")
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
