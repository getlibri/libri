#!/usr/bin/env python3
"""Proves the Android app's Whiteboard place (Places.kt BoardPlace) against a
throwaway Libri: the host's own page of the tile (`#board-tile=`) in a web
view, its tabs, and the board picked there kept as that place's own in the
phone's workspaces, so it opens on it again after a restart.

In the light and the dark theme: the Whiteboard item on the bar opens the
board the workspace keeps (Notes, with the agent's note on it); picking the
Plans tab is kept by the host as that item's board; after a restart the
place opens on Plans; exports land in Downloads; the image tool opens the
system's file picker. Leaves screenshots.

Runs inside `nix develop .#android-emulator`; the APK must be a debug build:
  drive_android_board.py LIBRI_DIR APK SHOTS_DIR [SERIAL]
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
SERIAL = sys.argv[4] if len(sys.argv) > 4 else "emulator-5618"
sys.argv = [sys.argv[0], LIBRI_DIR, APK, SERIAL]
import smoke_android as s  # noqa: E402  its emulator, adb and screen
from throwaway_libri import Throwaway  # noqa: E402

failures = []


def check(what, ok, detail=""):
    print(f"{'PASS' if ok else 'FAIL'} {what} {'' if ok else str(detail)[:300]}", flush=True)
    if not ok:
        failures.append(what)


def shot(name):
    SHOTS.mkdir(parents=True, exist_ok=True)
    with open(SHOTS / f"{name}.png", "wb") as f:
        subprocess.run(["adb", "-s", SERIAL, "exec-out", "screencap", "-p"], stdout=f)


def tap(label, xml=None):
    at = s.node(xml or s.screen(), label)
    if at:
        s.adb("shell", "input", "tap", *map(str, at))
    return at


def get(base, path):
    return json.loads(urllib.request.urlopen(f"{base}/{path}", timeout=10).read())


def post(base, path, body):
    req = urllib.request.Request(f"{base}/{path}", json.dumps(body).encode(), {"content-type": "application/json"})
    return json.loads(urllib.request.urlopen(req, timeout=10).read() or b"null")


def note(text, x):
    """A sticky note as the canvas keeps one: a filled rectangle."""
    return {"id": f"n{x}", "type": "rectangle", "x": x, "y": 100, "width": 200, "height": 160,
            "backgroundColor": "#ffec99", "fillStyle": "solid", "strokeColor": "transparent",
            "version": 1, "versionNonce": x + 7, "isDeleted": False, "seed": x + 1}


def board_kept(base):
    """The board the phone's first workspace keeps for its Whiteboard item."""
    layout = get(base, "api/phone-layout")["layout"]
    space = next(w for w in layout["spaces"] if w["id"] == "s1")
    return next((i.get("arg") for i in space["items"] if i["kind"] == "board"), None)


def main():
    t = None
    booted = False
    try:
        t = Throwaway(LIBRI_DIR, prefix="libri-android-board-")
        notes = post(t.base, "api/boards", {"name": "Notes"})["id"]
        plans = post(t.base, "api/boards", {"name": "Plans"})["id"]
        post(t.base, f"api/boards/{notes}", {"elements": [note("x", 0), note("y", 240)]})
        post(t.base, "api/phone-layout", {"spaces": [
            {"id": "s1", "name": "Main", "items": [{"kind": "chat"}, {"kind": "board", "arg": notes}]},
        ], "on": "s1"})
        booted = s.boot()
        s.adb("uninstall", s.APP)
        s.adb("install", APK)
        s.adb("reverse", f"tcp:{t.port}", f"tcp:{t.port}")
        prefs = f'<?xml version="1.0" encoding="utf-8" standalone="yes" ?>\n<map>\n<string name="address">{t.base}</string>\n</map>\n'
        (t.root / "work/libri.xml").write_text(prefs)
        s.adb("push", str(t.root / "work/libri.xml"), "/data/local/tmp/libri.xml")
        s.adb("shell", "run-as", s.APP, "sh", "-c", "'mkdir -p shared_prefs && cp /data/local/tmp/libri.xml shared_prefs/libri.xml'")
        for theme in ("light", "dark"):
            # Each theme starts on Notes again.
            post(t.base, "api/phone-layout", {"spaces": [
                {"id": "s1", "name": "Main", "items": [{"kind": "chat"}, {"kind": "board", "arg": notes}]},
            ], "on": "s1"})
            s.adb("shell", "cmd", "uimode", "night", "yes" if theme == "dark" else "no")
            s.adb("shell", "am", "force-stop", s.APP)
            s.adb("shell", "am", "start", "-W", "-n", s.ACTIVITY)
            bar = s.waited(lambda x: 'text="Whiteboard"' in x or 'content-desc="Whiteboard"' in x, 60)
            check(f"{theme}: the bar offers the workspace's Whiteboard", bar is not None)
            if not bar:
                continue
            tap("Whiteboard", bar)
            opened = s.waited(lambda x: "Notes" in x and "Plans" in x, 60)
            check(f"{theme}: the Whiteboard place opens its board, with the tabs", opened is not None)
            time.sleep(3)
            shot(f"android-board-{theme}-notes")
            # A sideways drag on the board is the canvas's: it draws or pans,
            # and the page stays on the Whiteboard rather than moving to Chat.
            s.adb("shell", "input", "swipe", "300", "1200", "900", "1250", "250")
            s.adb("shell", "input", "swipe", "900", "1400", "200", "1350", "250")
            time.sleep(1.5)
            check(f"{theme}: a drag on the board leaves the Whiteboard shown",
                  s.waited(lambda x: "Notes" in x and "Message Libri" not in x, 8) is not None)
            if opened and tap("Plans", opened):
                kept = False
                end = time.time() + 15
                while time.time() < end and not kept:
                    kept = board_kept(t.base) == plans
                    time.sleep(0.5)
                check(f"{theme}: the tab picked is kept as the item's board", kept, board_kept(t.base))
                check(f"{theme}: picking a tab leaves the Whiteboard shown",
                      s.waited(lambda x: "Notes" in x and "Plans" in x and "Message Libri" not in x, 10) is not None)
                time.sleep(2)
                shot(f"android-board-{theme}-plans")
            # After a restart the place opens on the board it keeps.
            s.adb("shell", "am", "force-stop", s.APP)
            s.adb("shell", "am", "start", "-W", "-n", s.ACTIVITY)
            bar = s.waited(lambda x: 'text="Whiteboard"' in x or 'content-desc="Whiteboard"' in x, 60)
            if bar:
                tap("Whiteboard", bar)
                time.sleep(4)
                url = s.adb("shell", "dumpsys", "activity", "top")
                check(f"{theme}: after a restart the place opens on the kept board",
                      board_kept(t.base) == plans and s.waited(lambda x: "Plans" in x, 30) is not None, url[-200:])
                shot(f"android-board-{theme}-restarted")
                # Exports reach Downloads: the page hands the file to the app.
                s.adb("shell", "rm", "-f", "/sdcard/Download/Plans.excalidraw", "/sdcard/Download/Plans.png")
                # The menu's items by where they sit from the button: a web
                # view's accessibility tree can lag the page by seconds.
                for item, file, dx, dy in (("Board file", "Plans.excalidraw", -232, 302), ("Picture (PNG)", "Plans.png", -308, 112)):
                    ex = tap("Export the board")
                    if ex:
                        time.sleep(1.5)
                        s.adb("shell", "input", "tap", str(ex[0] + dx), str(ex[1] + dy))
                    got = False
                    end = time.time() + 15
                    while time.time() < end and not got:
                        got = file in s.adb("shell", "ls", "/sdcard/Download/")
                        time.sleep(0.5)
                    check(f"{theme}: export {item} saves {file} to Downloads", got)
                # The image tool's file input opens the system's picker.
                if tap("Insert image"):
                    picker = s.waited(lambda x: "documentsui" in x or "Recent" in x or "Browse" in x, 15)
                    check(f"{theme}: inserting an image opens the file picker", picker is not None)
                    shot(f"android-board-{theme}-picker")
                    s.adb("shell", "input", "keyevent", "KEYCODE_BACK")
                    time.sleep(1.5)
                else:
                    check(f"{theme}: the board offers its image tool", False)
        crash = s.adb("logcat", "-d", "-s", "AndroidRuntime:E")
        check("no crash", "FATAL EXCEPTION" not in crash, crash[-400:])
    finally:
        s.adb("shell", "cmd", "uimode", "night", "no")
        if t:
            t.close()
        if booted:
            s.adb("emu", "kill")
    print(f"{len(failures)} failed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
