#!/usr/bin/env python3
"""Drives the Android app's Workers tree on an emulator against a throwaway
Libri (drive_worker_cards.py is the web's): Main and a Jobs memory as roots,
workers under them coloured by what each does, a sub-worker under its head,
the lanes running dashes while the head and its sub-worker message each
other, a tap folding a subtree, and the colours' key. Checks each node's
colour from the screen's pixels and saves screenshots.

Runs inside `nix develop .#android-emulator` (adb and the emulator on PATH);
boots an emulator on SERIAL if none is up. The APK must be a debug build:
  drive_android_worker_cards.py LIBRI_DIR APK OUT [SERIAL]
"""

import json
import re
import subprocess
import sys
import time
import urllib.request
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

# WorkTree.kt's Act colours.
ACTS = {"thinking": (0x6C, 0x8C, 0xFF), "reading": (0x2B, 0xB3, 0xA3), "editing": (0xB0, 0x7C, 0xFF),
        "running": (0x3C, 0xCF, 0x91), "person": (0xF2, 0xA9, 0x3B), "agent": (0x8A, 0x94, 0xA8),
        "failed": (0xE5, 0x48, 0x4D)}


def adb(*args):
    return subprocess.run(["adb", "-s", SERIAL, *args], capture_output=True, text=True).stdout


def shot(name):
    png = subprocess.run(["adb", "-s", SERIAL, "exec-out", "screencap", "-p"], capture_output=True).stdout
    (OUT / name).write_bytes(png)
    return OUT / name


def pixels(path):
    from PIL import Image
    return Image.open(path).convert("RGB")


def dump():
    """The screen's nodes as (text or description, bounds). uiautomator waits
    for the app to idle, which the tree's glow never does, so it may fail."""
    for _ in range(5):
        adb("shell", "uiautomator", "dump", "--compressed", "/data/local/tmp/ui.xml")
        xml = adb("shell", "cat", "/data/local/tmp/ui.xml")
        if "<hierarchy" in xml:
            return [(m[0] or m[1], tuple(map(int, m[2:]))) for m in re.findall(
                r'text="([^"]*)"[^>]*?content-desc="([^"]*)"[^>]*?bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', xml)]
        time.sleep(1)
    return []


def find(nodes, label):
    return next((b for t, b in nodes if t == label), None)


class Agents:
    """Libri's tools at the main chat's scope, as its harness reaches them."""

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

    def tool(self, called, **a):
        return self.call("tools/call", {"name": called, "arguments": a})["result"]["content"][0]["text"]

    def spawn(self, task):
        return self.tool("spawn", tasks=[{"task": task}]).split()[1].strip(".,:")


def main():
    rig = Throwaway(LIBRI_DIR, prefix="libri-cards-android-", extra={"LIBRI_WORK_TICK": "1"})
    report = {}
    agents = Agents(rig)

    def http(method, path, body=None):
        req = urllib.request.Request(rig.base + path, method=method, data=json.dumps(body).encode() if body is not None else None,
                                     headers={"content-type": "application/json"})
        return json.loads(urllib.request.urlopen(req, timeout=20).read() or b"null")

    def thread(name):
        return next((t for t in http("GET", "/api/work")["threads"] if t["name"] == name), None)

    try:
        jobs = http("POST", "/api/memories", {"name": "Jobs"})
        names = {act: agents.spawn(task) for act, task in [
            ("reading", "TOOL Read 900"), ("editing", "TOOL Edit 900"), ("running", "TOOL Bash 900"),
            ("failed", "CRASH"), ("person", "NEED Which board first?")]}
        head = agents.spawn("ASK CHAT HEAD")
        said = [""]

        def approved():
            said[0] = agents.tool("answer_subworker_request", name=head, approve=True)
            return said[0].startswith("Started ")
        until(approved, f"{head} gets its sub-worker", wait=90)
        sub = said[0].split()[1].strip(".,:")
        # The Jobs chat starts a worker of its own, which hangs from Jobs.
        http("POST", f"/api/memories/{jobs['id']}/chat", {"text": "SPAWN WORK SLOWLY on the job boards"})
        until(lambda: any(t.get("memory") == jobs["id"] for t in http("GET", "/api/work")["threads"]), "Jobs starts a worker", wait=60)
        jobs_worker = next(t["name"] for t in http("GET", "/api/work")["threads"] if t.get("memory") == jobs["id"])
        until(lambda: thread(names["failed"]).get("failed"), "a run fails", wait=60)
        until(lambda: thread(names["person"])["state"] == "waiting", "a worker asks the person", wait=60)
        until(lambda: all(thread(names[a]).get("calling") for a in ("reading", "editing", "running")), "the calls are open", wait=60)
        report["workers"] = names | {"head": head, "sub": sub, "jobs": jobs_worker}

        booted = boot()
        adb("uninstall", APP)
        report["install"] = adb("install", APK).strip().splitlines()[-1]
        adb("reverse", f"tcp:{rig.port}", f"tcp:{rig.port}")
        prefs = f'<?xml version="1.0" encoding="utf-8" standalone="yes" ?>\n<map>\n<string name="address">{rig.base}</string>\n</map>\n'
        (rig.root / "work/libri.xml").write_text(prefs)
        adb("push", str(rig.root / "work/libri.xml"), "/data/local/tmp/libri.xml")
        adb("shell", "run-as", APP, "sh", "-c", "'mkdir -p shared_prefs && cp /data/local/tmp/libri.xml shared_prefs/libri.xml'")
        adb("logcat", "-c")
        adb("shell", "am", "start", "-W", "-n", ACTIVITY)
        time.sleep(4)
        nodes = dump()
        # Every worker, from the bar's plus: the Work page has no item.
        plus = find(nodes, "Add to Main")
        if plus:
            adb("shell", "input", "tap", str((plus[0] + plus[2]) // 2), str((plus[1] + plus[3]) // 2))
            time.sleep(2)
            nodes = dump()
        work = next((b for t, b in reversed(nodes) if t == "Workers"), None)
        report["work_tab"] = bool(work)
        if work:
            adb("shell", "input", "tap", str((work[0] + work[2]) // 2), str((work[1] + work[3]) // 2))
        else:
            # The places page sideways; Work is the next one over.
            adb("shell", "input", "swipe", "900", "1200", "150", "1200", "200")
        time.sleep(5)
        shot("android-cards-0-raw.png")
        nodes = dump()
        report["dumped"] = bool(nodes)
        density = int(re.search(r"(\d+)", adb("shell", "wm", "density")).group(1)) / 160
        dp = lambda v: round(v * density)  # noqa: E731
        everyone = [names[a] for a in names] + [head, sub, jobs_worker]
        report["rows_found"] = {n: bool(find(nodes, n)) for n in ["Main", "Jobs", *everyone]}

        # Each worker's node dot sits left of its name: at X0 + DX*column,
        # its words at X0 + DX*(column + 1) + 10, a memory's own workers in
        # column 0 and each sub-worker a column further in.
        def dot(name, depth):
            b = find(nodes, name)
            if not b:
                return None
            x = b[0] - dp(14 + 20 * depth + 10) + dp(14 + 20 * (depth - 1))
            y = b[1] - dp(7) + dp(17)
            return x, y

        img = pixels(shot("android-cards-1-tree.png"))

        def colour(at):
            # The dot's middle, the least of the glow around it.
            px = [img.getpixel((at[0] + dx, at[1] + dy)) for dx in (-1, 0, 1) for dy in (-1, 0, 1)]
            return tuple(sorted(c[i] for c in px)[4] for i in range(3))

        def near(a, b):
            return all(abs(x - y) < 40 for x, y in zip(a, b))

        want = {names[a]: a for a in names} | {jobs_worker: "thinking"}
        got = {}
        for name, act in want.items():
            at = dot(name, 1)
            got[name] = {"want": act, "pixel": colour(at) if at else None}
            got[name]["ok"] = bool(at) and near(got[name]["pixel"], ACTS[act])
        report["colours"] = got

        # The head and its sub-worker message each other: the lane between
        # them and the head's lane up to Main run dashes for a while.
        agents.tool("tell_agent", name=sub, message="CHAT HEAD again please")
        agents.tool("tell_agent", name=head, message="how goes it?")
        time.sleep(2)
        talking = [shot(f"android-cards-2-talk-{i}.png") for i in range(3)]
        time.sleep(12)
        quiet = shot("android-cards-3-quiet.png")

        # Along the head's lane down to its sub-worker, which runs down the
        # sub-worker's column: dashes put the ink's dark pixels where the
        # lane is otherwise one colour.
        h, s = dot(head, 1), dot(sub, 2)
        def dark(path):
            im = pixels(path)
            if not (h and s):
                return -1
            return sum(1 for y in range(h[1] + dp(16), s[1] - dp(10)) if sum(im.getpixel((s[0], y))) < 200)
        report["dash_pixels_talking"] = [dark(p) for p in talking]
        report["dash_pixels_quiet"] = dark(quiet)

        # A tap on the head folds its sub-worker away; another opens it.
        nodes = dump() or nodes
        hb = find(nodes, head)
        if hb:
            adb("shell", "input", "tap", str(hb[0] + 10), str((hb[1] + hb[3]) // 2))
            time.sleep(2)
            folded = dump()
            report["fold_hides_sub"] = bool(folded) and find(folded, sub) is None
            shot("android-cards-4-folded.png")
            adb("shell", "input", "tap", str(hb[0] + 10), str((hb[1] + hb[3]) // 2))
            time.sleep(2)
        legend = find(nodes, "What the colours mean")
        if legend:
            adb("shell", "input", "tap", str((legend[0] + legend[2]) // 2), str((legend[1] + legend[3]) // 2))
            time.sleep(1.5)
            shot("android-cards-5-legend.png")
            report["legend_open"] = bool(find(dump(), "Last run failed"))
        report["crashed"] = "FATAL EXCEPTION" in adb("logcat", "-d", "-b", "crash")
        report["strays"] = rig.strays()
        if booted:
            adb("emu", "kill")
    finally:
        agents.proc.terminate()
        rig.close()
    print(json.dumps(report, indent=1))
    ok = (all(v["ok"] for v in report.get("colours", {}).values()) and all(report.get("rows_found", {}).values())
          and max(report.get("dash_pixels_talking", [0])) > report.get("dash_pixels_quiet", 0) and report.get("fold_hides_sub")
          and not report.get("crashed") and not report.get("strays"))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
