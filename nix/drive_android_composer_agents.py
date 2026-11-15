#!/usr/bin/env python3
"""Pictures the Android app's agents at the composer (Agents.kt), as
check_composer_agents.py checks the page's: with 1, 3 and 7 workers at work,
the circles across the composer's top edge, then the tree risen out of it,
then a row opened as a tab of its own, in the light and the dark theme. One
worker waits on the person. Checks the circles' label, the tree's rows and
the new tab from the screen's nodes, and that the app does not crash.

Runs inside `nix develop .#android-emulator` against a throwaway Libri; the
APK must be a debug build:
  drive_android_composer_agents.py LIBRI_DIR APK SHOTS_DIR [SERIAL]
"""
import json
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
LIBRI_DIR, APK, SHOTS = sys.argv[1], sys.argv[2], Path(sys.argv[3])
SERIAL = sys.argv[4] if len(sys.argv) > 4 else "emulator-5616"
sys.argv = [sys.argv[0], LIBRI_DIR, APK, SERIAL]
import smoke_android as s  # noqa: E402  its emulator, adb and screen
from throwaway_libri import Throwaway, until  # noqa: E402

failures = []


class Agents:
    """Libri's tools at the main chat's scope, as its harness reaches them
    (drive_android_worker_cards.py's, which reads its own arguments on import)."""

    def __init__(self, rig):
        env = rig.env | {"LIBRI_SCOPE": "person", "STATE_DIRECTORY": str(rig.root / "state"),
                         "LIBRI_NAME": "box-label", "LIBRI_PERSON": "Ada"}
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

    def spawn(self, task):
        got = self.call("tools/call", {"name": "spawn", "arguments": {"tasks": [{"task": task}]}})
        return got["result"]["content"][0]["text"].split()[1].strip(".,:")


def running(x):
    """Whether the circles count a worker: it works or asks the person for
    an approval or an answer (Agents.kt running)."""
    return x["state"] == "working" or (x["state"] == "waiting" and x.get("need") in ("approval", "answer"))


def check(what, ok, detail=""):
    print(f"{'PASS' if ok else 'FAIL'} {what} {'' if ok else str(detail)[:300]}", flush=True)
    if not ok:
        failures.append(what)


def shot(name):
    SHOTS.mkdir(parents=True, exist_ok=True)
    with open(SHOTS / f"{name}.png", "wb") as f:
        subprocess.run(["adb", "-s", SERIAL, "exec-out", "screencap", "-p"], stdout=f)


def labelled(prefix):
    """The bounds of the node whose text or description starts with `prefix`."""
    for _ in range(5):
        s.adb("shell", "uiautomator", "dump", "--compressed", "/data/local/tmp/ui.xml")
        xml = s.adb("shell", "cat", "/data/local/tmp/ui.xml")
        if "<hierarchy" in xml:
            import re
            for m in re.finditer(r'text="([^"]*)"[^>]*?content-desc="([^"]*)"[^>]*?bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', xml):
                if (m[1] or m[2]).startswith(prefix):
                    x0, y0, x1, y1 = map(int, m.groups()[2:])
                    return (x0 + x1) // 2, (y0 + y1) // 2
            return None
        time.sleep(1)
    return None


def main():
    t = Throwaway(LIBRI_DIR, prefix="libri-android-agents-", extra={"LIBRI_WORK_TICK": "1"})
    agents = Agents(t)
    booted = False

    def work():
        return json.loads(urllib.request.urlopen(t.base + "/api/work", timeout=20).read())["threads"]

    try:
        booted = s.boot()
        s.adb("uninstall", s.APP)
        s.adb("install", APK)
        s.adb("reverse", f"tcp:{t.port}", f"tcp:{t.port}")
        prefs = f'<?xml version="1.0" encoding="utf-8" standalone="yes" ?>\n<map>\n<string name="address">{t.base}</string>\n</map>\n'
        (t.root / "work/libri.xml").write_text(prefs)
        s.adb("push", str(t.root / "work/libri.xml"), "/data/local/tmp/libri.xml")
        s.adb("shell", "run-as", s.APP, "sh", "-c", "'mkdir -p shared_prefs && cp /data/local/tmp/libri.xml shared_prefs/libri.xml'")
        # One that has finished, its report unread (Waiting, "For review"),
        # leaves the circles: every count below would be one more with it.
        done = agents.spawn("Report that the quotes match.")
        until(lambda: any(x["name"] == done and x["state"] == "waiting" and x.get("need") == "review" for x in work()), f"{done} done", wait=90)
        spawned = 0
        # The second waits on the person; the rest hold a call open.
        tasks = ["TOOL Read 1800", "NEED Which board first?"] + ["TOOL Bash 1800"] * 5
        for count in (1, 3, 7):
            while spawned < count:
                agents.spawn(tasks[spawned])
                spawned += 1
            until(lambda: sum(running(x) for x in work()) >= count, f"{count} at work", wait=90)
            for theme in ("light", "dark"):
                s.adb("shell", "cmd", "uimode", "night", "yes" if theme == "dark" else "no")
                s.adb("shell", "am", "force-stop", s.APP)
                s.adb("shell", "am", "start", "-W", "-n", s.ACTIVITY)
                time.sleep(6)
                tag = f"{theme}-{count}"
                circles = labelled(f"{count} agent")
                check(f"{tag}: the circles say {count} running", circles is not None)
                shot(f"android-agents-{tag}-folded")
                if not circles:
                    continue
                s.adb("shell", "input", "tap", *map(str, circles))
                time.sleep(1.5)
                shot(f"android-agents-{tag}-risen")
                first = next(x for x in work() if running(x))
                row = labelled(first["name"])
                check(f"{tag}: the tree lists {first['name']}", row is not None)
                check(f"{tag}: the tree leaves out {done}, done", labelled(done) is None)
                if row and theme == "light" and count == 3:
                    s.adb("shell", "input", "tap", *map(str, row))
                    time.sleep(3)
                    shot(f"android-agents-{tag}-opened")
                    check(f"{tag}: {first['name']} opens as a tab of its own", labelled(first["name"]) is not None)
        crash = s.adb("logcat", "-d", "-s", "AndroidRuntime:E")
        check("no crash", "FATAL EXCEPTION" not in crash, crash[-400:])
    finally:
        s.adb("shell", "cmd", "uimode", "night", "no")
        t.close()
        if booted:
            s.adb("emu", "kill")
    print(f"{len(failures)} failed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
