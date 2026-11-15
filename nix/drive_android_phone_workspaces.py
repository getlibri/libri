#!/usr/bin/env python3
"""Drives the Android app's workspaces on an emulator against a throwaway
Libri (check_phone_workspaces.py is the web's): a first run's bar (Main:
Chat, Workers, Library), the chat's header with only the side panel and
search, the plus's menu with Temp and the terminal, the bar edited in place
(a long press lifts it, an item dragged reorders, one dropped on the ✕
leaves, the disc and the chat stay), a quick flick up closing an item with
an Undo, Temp and the terminal without heads (Temp's Memory and Clear
floating, the terminal's keys above the bar), a second workspace made and
named, the switcher,
a swipe along the bar to change workspace, and a swipe right on the first
item opening the side panel (the host's state, Hosts, Settings, the theme
and version). Checks the host keeps the workspaces and saves screenshots.

Runs inside `nix develop .#android-emulator` (adb and the emulator on PATH);
boots an emulator on SERIAL if none is up. The APK must be a debug build:
  drive_android_phone_workspaces.py LIBRI_DIR APK OUT [SERIAL]
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
from throwaway_libri import Throwaway  # noqa: E402

LIBRI_DIR, APK, OUT = sys.argv[1], sys.argv[2], Path(sys.argv[3])
SERIAL = sys.argv[4] if len(sys.argv) > 4 else "emulator-5616"
smoke_android.SERIAL = SERIAL
APP = "is.libri"
ACTIVITY = f"{APP}/su.libri.MainActivity"
OUT.mkdir(parents=True, exist_ok=True)
failures = []


def check(what, ok, got=None):
    print(("ok   " if ok else "FAIL ") + what + ("" if ok or got is None else f": {got}"), flush=True)
    if not ok:
        failures.append(what)


def adb(*args):
    return subprocess.run(["adb", "-s", SERIAL, *args], capture_output=True, text=True).stdout


def shot(name):
    png = subprocess.run(["adb", "-s", SERIAL, "exec-out", "screencap", "-p"], capture_output=True).stdout
    (OUT / name).write_bytes(png)


def dump():
    """The screen's nodes as (text or description, bounds)."""
    for _ in range(5):
        adb("shell", "uiautomator", "dump", "--compressed", "/data/local/tmp/ui.xml")
        xml = adb("shell", "cat", "/data/local/tmp/ui.xml")
        if "<hierarchy" in xml:
            # One node per <node ...> tag, its text and its description each
            # findable, so a pill described "Memory on" but showing "Memory"
            # answers to both.
            out = []
            for tag in re.findall(r'<node [^>]*>', xml):
                t = re.search(r' text="([^"]*)"', tag)
                d = re.search(r' content-desc="([^"]*)"', tag)
                b = re.search(r' bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', tag)
                if not b:
                    continue
                box = tuple(map(int, b.groups()))
                for v in {(t and t[1]) or "", (d and d[1]) or ""} - {""} or {""}:
                    out.append((v, box))
            return out
        time.sleep(1)
    return []


def find(label, nodes=None, starts=False):
    for t, b in nodes if nodes is not None else dump():
        if t == label or (starts and t.startswith(label)):
            return b


def middle(b):
    return (b[0] + b[2]) // 2, (b[1] + b[3]) // 2


def tap(label, starts=False, wait=1.0):
    b = find(label, starts=starts)
    if not b:
        shot("android-debug-missing.png")
        raise SystemExit(f"no {label!r} on the screen: {[t for t, _ in dump() if t]}")
    adb("shell", "input", "tap", *map(str, middle(b)))
    time.sleep(wait)


def hold(label):
    """A long press on a bar item opens its menu (TileViews.kt ItemMenu);
    its Edit the bar edits the bar, as the press itself once did."""
    # A slow drag just before may have held already and opened the menu.
    edit = find("Edit the bar")
    if edit:
        adb("shell", "input", "tap", *map(str, middle(edit)))
        time.sleep(1)
        return
    x, y = middle(find(label))
    adb("shell", "input", "swipe", str(x), str(y), str(x), str(y), "900")
    for _ in range(4):
        time.sleep(1)
        edit = find("Edit the bar")
        if edit:
            adb("shell", "input", "tap", *map(str, middle(edit)))
            time.sleep(1)
            return
    print(f"DEBUG no item menu after holding {label}: {[t for t, _ in dump() if t]}", flush=True)


def plus_of(nodes=None):
    """The plus at the bar's end, an ✕ while the bar is edited."""
    nodes = nodes if nodes is not None else dump()
    return find("Add to", nodes, starts=True) or find("Done editing the bar", nodes)


def bar(nodes):
    """The bar's items, left to right: the labels on its row at the foot."""
    plus = plus_of(nodes)
    if not plus:
        return []
    top, foot = plus[1], plus[3]
    names = {"Chat", "Temp", "Terminal", "Workers", "Library", "Pinned", "Calendar"}
    row = sorted((b[0], t) for t, b in nodes if t in names and b[1] >= top and b[3] <= foot)
    return [t for _, t in row]


def swipe(a, b, ms):
    adb("shell", "input", "swipe", *map(str, (*a, *b)), str(ms))
    time.sleep(1.2)


def flick(label):
    """A quick upward flick on a bar item."""
    x, y = middle(find(label))
    swipe((x, y), (x, y - 260), 90)


def main():
    rig = Throwaway(LIBRI_DIR, prefix="libri-spaces-android-")

    def http(method, path, body=None):
        req = urllib.request.Request(rig.base + path, method=method, data=json.dumps(body).encode() if body is not None else None,
                                     headers={"content-type": "application/json"})
        return json.loads(urllib.request.urlopen(req, timeout=40).read() or b"null")

    booted = False
    try:
        booted = boot()
        # A Pixel 6 is 1080px wide: at 443dpi it is 390dp, the web checks' width.
        adb("shell", "wm", "density", "443")
        adb("uninstall", APP)
        installed = subprocess.run(["adb", "-s", SERIAL, "install", APK], capture_output=True, text=True)
        said = (installed.stdout + installed.stderr).strip()
        print(said.splitlines()[-1] if said else "adb install said nothing", flush=True)
        if "Success" not in installed.stdout:
            raise SystemExit(f"the APK did not install: {said}")
        adb("reverse", f"tcp:{rig.port}", f"tcp:{rig.port}")
        prefs = f'<?xml version="1.0" encoding="utf-8" standalone="yes" ?>\n<map>\n<string name="address">{rig.base}</string>\n</map>\n'
        (rig.root / "work/libri.xml").write_text(prefs)
        adb("push", str(rig.root / "work/libri.xml"), "/data/local/tmp/libri.xml")
        adb("shell", "run-as", APP, "sh", "-c", "'mkdir -p shared_prefs && cp /data/local/tmp/libri.xml shared_prefs/libri.xml'")
        adb("shell", "am", "start", "-W", "-n", ACTIVITY)
        time.sleep(6)

        nodes = dump()
        check("a first run's bar: Chat, Library", bar(nodes) == ["Chat", "Library"], bar(nodes))
        check("the first workspace is Main", bool(find("Workspace: Main. Switch workspace", nodes)))
        check("the chat's header: the side panel and search, no Temp or terminal",
              bool(find("Side panel", nodes)) and bool(find("Search", nodes)) and not find("Terminal", nodes) and not find("Temp chat, not remembered", nodes))
        shot("android-workspace-main.png")

        # Temp first, so the bar holds four items as before the Workers item went.
        tap("Add to", starts=True)
        tap("Temp chat", wait=2)
        check("an item added joins the bar", bar(dump())[-1:] == ["Temp"], bar(dump()))
        tap("Add to", starts=True)
        nodes = dump()
        check("the plus offers every worker, though no Workers item", bool(find("Workers", nodes)))
        check("the plus offers Temp and the terminal", bool(find("Temp chat", nodes)) and bool(find("Terminal", nodes)))
        shot("android-plus-menu.png")
        tap("Terminal", wait=2)
        # The terminal raises the keyboard, which hides the bar; its own key puts it down.
        tap("Hide the keyboard", wait=1.5)
        nodes = dump()
        check("an item added joins the bar", bar(nodes)[-1:] == ["Terminal"], bar(nodes))
        # The terminal: no head and no ✕, its keys clear above the bar.
        check("the terminal has no head and no close", not find("Close the terminal", nodes), [t for t, _ in nodes if t])
        keys, plus = find("Paste", nodes), plus_of(nodes)
        check("the terminal's keys sit above the bar", bool(keys) and bool(plus) and keys[3] <= plus[1], (keys, plus))
        shot("android-terminal.png")

        # Editing: a long press lifts the bar, the plus becomes the ✕, the disc stays put.
        disc = find("Workspace: Main. Switch workspace", nodes)
        x, y = middle(find("Library"))
        adb("shell", "input", "swipe", str(x), str(y), str(x), str(y), "900")
        time.sleep(1)
        nodes = dump()
        check("a long press opens the item's menu", bool(find("Library menu", nodes)) and bool(find("Edit the bar", nodes)) and bool(find("Move to workspace…", nodes)), [t for t, _ in nodes if t])
        shot("android-item-menu.png")
        tap("Edit the bar")
        nodes = dump()
        check("a long press edits the bar: the plus becomes the ✕", bool(find("Done editing the bar", nodes)) and not find("Add to", nodes, starts=True))
        check("the disc stays where it was while editing", find("Workspace: Main. Switch workspace", nodes) == disc, (disc, find("Workspace: Main. Switch workspace", nodes)))
        shot("android-bar-editing.png")
        d = middle(disc)
        swipe(d, (d[0] + 400, d[1]), 600)
        check("the disc cannot be dragged", bar(dump()) == ["Chat", "Library", "Temp", "Terminal"] and find("Workspace: Main. Switch workspace") == disc, bar(dump()))
        # Terminal dragged one place left.
        t, l = middle(find("Terminal")), middle(find("Temp"))
        swipe(t, (l[0], t[1]), 700)
        nodes = dump()
        if not plus_of(nodes):
            shot("android-debug-after-drag.png")
            print("DEBUG", [(t, b) for t, b in nodes if t], flush=True)
            adb("shell", "dumpsys", "input_method") and print("IME shown:", "mInputShown=true" in adb("shell", "dumpsys", "input_method"))
        check("an item dragged left takes the place before", bar(nodes) == ["Chat", "Library", "Terminal", "Temp"], bar(nodes))
        # The chat dropped on the ✕ stays; Temp dropped on it leaves.
        x = middle(plus_of())
        swipe(middle(find("Chat")), x, 900)
        check("the chat dropped on the ✕ stays", bar(dump()) == ["Chat", "Library", "Terminal", "Temp"], bar(dump()))
        swipe(middle(find("Temp")), middle(plus_of()), 700)
        time.sleep(0.6)
        # The Undo lasts 5 s and the dump is slow, so it is tapped straight
        # from the dump; the screenshot comes from the last drop below.
        nodes = dump()
        check("an item dropped on the ✕ leaves the bar", bar(nodes) == ["Chat", "Library", "Terminal"], bar(nodes))
        check("and an Undo is offered", bool(find("Closed Temp.", nodes)) and bool(find("Undo", nodes)), [t for t, _ in nodes if t])
        undo = find("Undo", nodes)
        if undo:
            adb("shell", "input", "tap", *map(str, middle(undo)))
        time.sleep(1)
        check("Undo puts it back in its place", bar(dump()) == ["Chat", "Library", "Terminal", "Temp"], bar(dump()))
        # A tap off the bar puts it down.
        adb("shell", "input", "tap", "540", "900")
        time.sleep(1)
        check("a tap off the bar ends editing", bool(find("Add to", starts=True)) and not find("Done editing the bar"))

        # A flick up: the chat stays; the terminal closes, with an Undo.
        flick("Chat")
        check("a flick up on the chat closes nothing", bar(dump()) == ["Chat", "Library", "Terminal", "Temp"], bar(dump()))
        flick("Terminal")
        time.sleep(0.5)
        shot("android-flick-closed.png")
        nodes = dump()
        check("a flick up closes an item", bar(nodes) == ["Chat", "Library", "Temp"], bar(nodes))
        check("with an Undo naming it", bool(find("Closed Terminal.", nodes)) and bool(find("Undo", nodes)), [t for t, _ in nodes if t])
        undo = find("Undo", nodes)
        if undo:
            adb("shell", "input", "tap", *map(str, middle(undo)))
        time.sleep(1)
        check("Undo brings the flicked item back", bar(dump()) == ["Chat", "Library", "Terminal", "Temp"], bar(dump()))
        # A slow, short drag up is not a flick.
        x, y = middle(find("Terminal"))
        swipe((x, y), (x, y - 40), 900)
        check("a slow small drag up closes nothing", "Terminal" in bar(dump()), bar(dump()))
        hold("Terminal")
        swipe(middle(find("Terminal")), middle(plus_of()), 700)
        time.sleep(0.4)
        shot("android-dropped-on-x.png")
        adb("shell", "input", "tap", "540", "900")
        time.sleep(6)
        check("the host keeps the edited bar", bar(dump()) == ["Chat", "Library", "Temp"], bar(dump()))

        # Temp: no head; Memory floats at the top right, and Clear once it has
        # talk. It is on the bar already: the plus would open a second, new one.
        tap("Temp", wait=2)
        nodes = dump()
        check("Temp has no head and no way back drawn", not find("Back to the chat", nodes) and not find("not remembered", nodes, starts=True) and not find(" · not remembered", nodes),
              [t for t, _ in nodes if t])
        mem = find("Memory off", nodes) or find("Memory on", nodes)
        width = int(adb("shell", "wm", "size").split()[-1].split("x")[0])
        check("Temp's Memory floats at the top right", bool(mem) and mem[2] > width * 0.8 and mem[1] < 400, mem)
        tap("Message", starts=True)
        adb("shell", "input", "text", "hello")
        time.sleep(0.5)
        tap("Send", wait=6)
        nodes = dump()
        clear = find("Clear", nodes)
        mem = find("Memory off", nodes) or find("Memory on", nodes)
        check("Clear floats beside Memory once Temp has talk", bool(clear) and bool(mem) and abs(middle(clear)[1] - middle(mem)[1]) < 20 and clear[0] > mem[2] and clear[2] > width * 0.8, (clear, mem))
        shot("android-temp.png")
        # Send leaves the keyboard up, and the bar hides under it; Back puts it down.
        if "mInputShown=true" in adb("shell", "dumpsys", "input_method"):
            adb("shell", "input", "keyevent", "KEYCODE_BACK")
            time.sleep(1)
        tap("Chat", wait=1.5)
        hold("Temp")
        swipe(middle(find("Temp")), middle(plus_of()), 700)
        adb("shell", "input", "tap", "540", "900")
        time.sleep(1)
        check("Temp dropped on the ✕ leaves the bar", "Temp" not in bar(dump()), bar(dump()))

        tap("Workspace: Main. Switch workspace")
        tap("New workspace")
        adb("shell", "input", "keyevent", "KEYCODE_MOVE_END")
        for _ in range(12):
            adb("shell", "input", "keyevent", "KEYCODE_DEL")
        adb("shell", "input", "text", "Shell")
        adb("shell", "input", "keyevent", "66")
        time.sleep(1.5)
        tap("Add to", starts=True)
        tap("Terminal", wait=2)
        # The terminal raises the keyboard, which hides the bar.
        tap("Hide the keyboard")
        tap("Add to", starts=True)
        tap("Temp chat", wait=2)
        nodes = dump()
        check("a new workspace holds the chat, then what was added",
              bool(find("Workspace: Shell. Switch workspace", nodes)) and bar(nodes) == ["Chat", "Terminal", "Temp"], bar(nodes))
        tap("Terminal", wait=3)
        tap("Hide the keyboard")
        shot("android-workspace-shell.png")
        tap("Workspace: Shell. Switch workspace")
        shot("android-switcher.png")
        tap("Main")
        check("the switcher goes to a workspace", bar(dump()) == ["Chat", "Library"], bar(dump()))

        plus = find("Add to", starts=True)
        y = (plus[1] + plus[3]) // 2
        adb("shell", "input", "swipe", "800", str(y), "250", str(y), "250")
        time.sleep(1.5)
        check("a swipe along the bar goes to the next workspace", bool(find("Workspace: Shell. Switch workspace")))
        adb("shell", "input", "swipe", "250", str(y), "800", str(y), "250")
        time.sleep(1.5)
        check("and back", bool(find("Workspace: Main. Switch workspace")))

        time.sleep(2)
        kept = http("GET", "/api/phone-layout")["layout"] or {}
        check("the host keeps both workspaces", [w["name"] for w in kept.get("spaces", [])] == ["Main", "Shell"], kept)

        tap("Library", wait=1.5)
        adb("shell", "input", "swipe", "200", "1200", "900", "1200", "250")
        time.sleep(1.5)
        adb("shell", "input", "swipe", "200", "1200", "900", "1200", "250")
        time.sleep(1.5)
        nodes = dump()
        check("past the leftmost item a swipe opens the side panel", bool(find("Settings", nodes)) and bool(find("Hosts", nodes)))
        check("the side panel: the host's state, no Calendar", bool(find("Connected", nodes) or find("Connecting", nodes)) and not find("Calendar", nodes),
              [t for t, _ in nodes if t])
        shot("android-side-panel.png")
    finally:
        if booted:
            adb("emu", "kill")
        rig.close()
    print("FAILED: " + ", ".join(failures) if failures else "all passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
