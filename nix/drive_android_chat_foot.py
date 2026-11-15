#!/usr/bin/env python3
"""Pictures the Android app's chat foot, as check_edge_fade.py does the
page's: the talk scrolled a little up, so lines pass under the composer and
the bar, in the light and the dark theme. The talk should be wholly clear
above the dock and show dimmed beneath the composer and under the bar
(Kit.kt chatFade). Also opens Temp, whose dock floats the same way. Leaves
screenshots; checks only that the app shows the chat and does not crash.

Runs inside `nix develop .#android-emulator` against a throwaway Libri
holding a seeded chat; the APK must be a debug build:
  drive_android_chat_foot.py LIBRI_DIR APK SHOTS_DIR [SERIAL]
"""
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
LIBRI_DIR, APK, SHOTS = sys.argv[1], sys.argv[2], Path(sys.argv[3])
SERIAL = sys.argv[4] if len(sys.argv) > 4 else "emulator-5616"
sys.argv = [sys.argv[0], LIBRI_DIR, APK, SERIAL]
import smoke_android as s  # noqa: E402  its emulator, adb and screen
from drive_ui import seed_chat  # noqa: E402
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


def lift():
    """A slow drag down the middle of the screen: the talk moves back a
    little, so lines stand under the composer and the bar."""
    size = s.adb("shell", "wm", "size").split()[-1]
    w, h = map(int, size.split("x"))
    s.adb("shell", "input", "swipe", str(w // 2), str(h * 4 // 10), str(w // 2), str(h * 6 // 10), "600")
    time.sleep(1.5)


def main():
    t = Throwaway(LIBRI_DIR, prefix="libri-android-chat-foot-")
    booted = False
    try:
        seed_chat(t, 60, 1)
        last = s.words_of(s.history(t.base)[-1]["text"], 4)
        booted = s.boot()
        s.adb("uninstall", s.APP)
        s.adb("install", APK)
        s.adb("reverse", f"tcp:{t.port}", f"tcp:{t.port}")
        prefs = f'<?xml version="1.0" encoding="utf-8" standalone="yes" ?>\n<map>\n<string name="address">{t.base}</string>\n</map>\n'
        (t.root / "work/libri.xml").write_text(prefs)
        s.adb("push", str(t.root / "work/libri.xml"), "/data/local/tmp/libri.xml")
        s.adb("shell", "run-as", s.APP, "sh", "-c", "'mkdir -p shared_prefs && cp /data/local/tmp/libri.xml shared_prefs/libri.xml'")
        for theme in ("light", "dark"):
            s.adb("shell", "cmd", "uimode", "night", "yes" if theme == "dark" else "no")
            s.adb("shell", "am", "force-stop", s.APP)
            s.adb("shell", "am", "start", "-W", "-n", s.ACTIVITY)
            check(f"{theme}: the chat opens on its last message", s.waited(lambda x: last in x, 60) is not None)
            time.sleep(2)
            shot(f"android-chat-{theme}-foot")
            lift()
            shot(f"android-chat-{theme}-scrolled")
            temp = s.node(s.screen(), "Temp")
            if temp:
                s.adb("shell", "input", "tap", *map(str, temp))
                time.sleep(2)
                shot(f"android-temp-{theme}")
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
