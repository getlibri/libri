#!/usr/bin/env python3
"""Where the Android app's chat stands against its foot, in numbers, as
measure_chat_foot.py has the page's: after the app opens, after it goes to
the background and comes back, after a cold start, after a send, while its
reply streams and once it ended. For each, `below` is how far the last
message's foot stands above the composer's top, in px (the chat scrolled too
high when it is large, the last line hidden under the composer when
negative), read from the screen's accessibility tree as the composer's field
top less the lowest line of words above it. Leaves a screenshot of
each. A measure, not a check: it fails nothing.

Runs inside `nix develop .#android-emulator` against a throwaway Libri
holding a seeded chat; the APK must be a debug build:
  measure_android_foot.py LIBRI_DIR APK SHOTS_DIR [SERIAL]
"""
import json
import re
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

BOUNDS = re.compile(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"')


def nodes(xml):
    """Every node's text (or description) and its box."""
    out = []
    for m in re.finditer(r"<node [^>]*>", xml):
        tag = m.group(0)
        text = (re.search(r' text="([^"]*)"', tag) or [None, ""])[1] or (re.search(r'content-desc="([^"]*)"', tag) or [None, ""])[1]
        b = BOUNDS.search(tag)
        if b:
            out.append((text, tuple(map(int, b.groups())), tag))
    return out


def state(words):
    """How far the talk's lowest line stands above the composer's top, and
    whether the message holding `words` (the newest) is on screen."""
    xml = s.screen()
    all_ = nodes(xml)
    field = [b for _, b, tag in all_ if "EditText" in tag]
    if not field:
        return {"below": None, "seen": False}
    top = field[0][1]
    # Words only (a button's description is no line of the talk), above the
    # composer's field: the bar's names and the field's own words are below.
    lines = [b for _, b, tag in all_ if re.search(r' text="[^"]+"', tag) and "EditText" not in tag and b[1] < top]
    return {"below": top - max(b[3] for b in lines) if lines else None, "seen": any(words in t for t, _, _ in all_)}


def shot(name):
    SHOTS.mkdir(parents=True, exist_ok=True)
    import subprocess
    with open(SHOTS / f"{name}.png", "wb") as f:
        subprocess.run(["adb", "-s", SERIAL, "exec-out", "screencap", "-p"], stdout=f)


def main():
    t = Throwaway(LIBRI_DIR, prefix="libri-android-foot-")
    report = {}
    try:
        seed_chat(t, 120, 1)
        last = s.words_of(s.history(t.base)[-1]["text"], 4)
        # Left up for the next run, as a before and after want one emulator.
        s.boot()
        s.adb("uninstall", s.APP)
        s.adb("install", APK)
        s.adb("reverse", f"tcp:{t.port}", f"tcp:{t.port}")
        prefs = f'<?xml version="1.0" encoding="utf-8" standalone="yes" ?>\n<map>\n<string name="address">{t.base}</string>\n</map>\n'
        (t.root / "work/libri.xml").write_text(prefs)
        s.adb("push", str(t.root / "work/libri.xml"), "/data/local/tmp/libri.xml")
        s.adb("shell", "run-as", s.APP, "sh", "-c", "'mkdir -p shared_prefs && cp /data/local/tmp/libri.xml shared_prefs/libri.xml'")
        s.adb("shell", "am", "start", "-W", "-n", s.ACTIVITY)
        s.waited(lambda x: last in x, 60)
        time.sleep(1)
        report["open 1s"] = state(last)
        time.sleep(5)
        report["open 6s"] = state(last)
        shot("open")
        s.adb("shell", "input", "keyevent", "KEYCODE_HOME")
        time.sleep(3)
        s.adb("shell", "am", "start", "-W", "-n", s.ACTIVITY)
        time.sleep(4)
        report["back from home"] = state(last)
        s.adb("shell", "am", "force-stop", s.APP)
        s.adb("shell", "am", "start", "-W", "-n", s.ACTIVITY)
        s.waited(lambda x: last in x, 60)
        time.sleep(4)
        report["cold start"] = state(last)
        shot("cold-start")
        box = s.node(s.screen(), "Message")
        s.adb("shell", "input", "tap", *map(str, box))
        s.adb("shell", "input", "text", "STREAM%s40%s7")
        s.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        time.sleep(1)
        send = s.node(s.screen(), "Send")
        s.adb("shell", "input", "tap", *map(str, send))
        time.sleep(1.5)
        report["after send"] = state("STREAM 40 7")
        time.sleep(2.5)
        shot("streaming")
        reply = None
        for _ in range(120):
            h = s.history(t.base)
            if len(h) > 1 and h[-2]["text"] == "STREAM 40 7" and h[-1].get("kind") == "talk":
                reply = s.words_of(h[-1]["text"], 4)
                break
            time.sleep(0.25)
        time.sleep(4)
        report["reply ended"] = state(reply or last)
        shot("reply-ended")
        s.adb("shell", "input", "keyevent", "KEYCODE_HOME")
        time.sleep(3)
        s.adb("shell", "am", "start", "-W", "-n", s.ACTIVITY)
        time.sleep(4)
        report["back after the reply"] = state(reply or last)
        shot("back-after-reply")
        # A little way up from the foot, so lines show passing under the
        # composer and the bar, and the same in the dark.
        s.adb("shell", "input", "swipe", "540", "1100", "540", "1500", "400")
        time.sleep(1.5)
        shot("up")
        s.adb("shell", "cmd", "uimode", "night", "yes")
        time.sleep(3)
        shot("up-dark")
        s.adb("shell", "cmd", "uimode", "night", "no")
        report["crashed"] = "FATAL EXCEPTION" in s.adb("logcat", "-d", "-b", "crash")
        report["strays"] = t.strays()
    finally:
        t.close()
    print(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
