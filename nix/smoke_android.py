#!/usr/bin/env python3
"""The Android app's smoke scenario for the UI driver (drive_ui.py is the
web's): on an emulator, the debug app opens on a throwaway Libri holding a
long chat, shows it, sends a message whose streamed reply comes back, and
scrolls the chat. Measures the cold start (am start -W), the time from the
send to the reply on screen, and the frames the scroll drew (dumpsys
gfxinfo). Prints one JSON report.

Runs inside `nix develop .#android-emulator` (adb and the emulator on PATH);
boots an emulator on SERIAL if none is up. The APK must be a debug build,
since its settings are written with run-as:
  smoke_android.py LIBRI_DIR APK [SERIAL]
"""

import json
import os
import re
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

# An emulator starts only when the host has room for it, and at most three
# run at once (nix/with_memory.sh): each boots a 3 GB guest beside the
# workers' builds. A drive run bare re-runs itself through the wrapper,
# with the command line it was given (sys.orig_argv): a drive importing
# this module has already cut sys.argv down to this module's arguments,
# and re-run with those it read its serial as its shots folder.
if os.environ.get("LIBRI_EMULATOR_ADMITTED") != "1":
    os.environ["LIBRI_EMULATOR_ADMITTED"] = "1"
    _wrap = Path(__file__).resolve().parent / "with_memory.sh"
    os.execv(str(_wrap), [str(_wrap), "5", "--emulator", sys.executable, *sys.orig_argv[1:]])

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from drive_ui import seed_chat  # noqa: E402
from throwaway_libri import Throwaway  # noqa: E402

LIBRI_DIR, APK = sys.argv[1], sys.argv[2]
SERIAL = sys.argv[3] if len(sys.argv) > 3 else "emulator-5616"
APP = "is.libri"
ACTIVITY = f"{APP}/su.libri.MainActivity"
MESSAGES = 400
SEED = 1


def adb(*args):
    return subprocess.run(["adb", "-s", SERIAL, *args], capture_output=True, text=True).stdout


def boot():
    """Says whether it booted the emulator, which is then shut down after."""
    if adb("get-state").strip() == "device":
        return False
    home = os.environ["ANDROID_HOME"]
    image = sorted(Path(home, "system-images").glob("*/*/x86_64"))[0]
    package = ";".join(image.relative_to(home).parts)
    subprocess.run(f"echo no | avdmanager create avd --force -n libri-smoke -k '{package}' -d pixel_6", shell=True, capture_output=True)
    subprocess.Popen(["emulator", "-avd", "libri-smoke", "-port", SERIAL.split("-")[1], "-cores", "4", "-memory", "3072", "-no-window", "-no-audio",
                      "-no-boot-anim", "-no-snapshot", "-accel", "on", "-gpu", "swiftshader_indirect"],
                     stdout=open("/tmp/smoke_android_emulator.log", "w"), stderr=subprocess.STDOUT)
    adb("wait-for-device")
    for _ in range(120):
        if adb("shell", "getprop", "sys.boot_completed").strip() == "1":
            return True
        time.sleep(3)
    raise SystemExit("the emulator did not boot")


def screen():
    adb("shell", "uiautomator", "dump", "/data/local/tmp/ui.xml")
    return adb("shell", "cat", "/data/local/tmp/ui.xml")


def node(xml, label):
    m = re.findall(r'(?:text|content-desc)="' + re.escape(label) + r'[^"]*"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', xml)
    if not m:
        return None
    x1, y1, x2, y2 = map(int, m[-1])
    return (x1 + x2) // 2, (y1 + y2) // 2


def waited(test, limit):
    end = time.monotonic() + limit
    while time.monotonic() < end:
        xml = screen()
        if test(xml):
            return xml
        time.sleep(0.5)
    return None


def history(base):
    return json.load(urllib.request.urlopen(f"{base}/api/history", timeout=10))


def words_of(text, n=3):
    return " ".join(re.findall(r"[A-Za-z]+", text)[:n])


def main():
    t = Throwaway(LIBRI_DIR, prefix="libri-smoke-android-")
    report = {"apk": APK, "messages": MESSAGES, "seed": SEED}
    try:
        seed_chat(t, MESSAGES, SEED)
        last = words_of(history(t.base)[-1]["text"])
        booted = boot()
        adb("uninstall", APP)
        report["install"] = adb("install", APK).strip().splitlines()[-1]
        adb("reverse", f"tcp:{t.port}", f"tcp:{t.port}")
        prefs = f'<?xml version="1.0" encoding="utf-8" standalone="yes" ?>\n<map>\n<string name="address">{t.base}</string>\n</map>\n'
        (t.root / "work/libri.xml").write_text(prefs)
        adb("push", str(t.root / "work/libri.xml"), "/data/local/tmp/libri.xml")
        adb("shell", "run-as", APP, "sh", "-c", "'mkdir -p shared_prefs && cp /data/local/tmp/libri.xml shared_prefs/libri.xml'")
        started = adb("shell", "am", "start", "-W", "-n", ACTIVITY)
        report["cold_start_ms"] = int((re.search(r"TotalTime: (\d+)", started) or [0, -1])[1])
        start = time.monotonic()
        shown = waited(lambda x: last in x, 60)
        report["chat_shown"] = shown is not None
        report["chat_shown_ms"] = round((time.monotonic() - start) * 1000) if shown else None

        # A send, and its streamed reply on screen.
        box = node(shown or screen(), "Message")
        report["sent"] = False
        if box:
            adb("shell", "input", "tap", *map(str, box))
            adb("shell", "input", "text", "STREAM%s40%s7")
            send = node(screen(), "Send")
            if send:
                adb("shell", "input", "tap", *map(str, send))
                start = time.monotonic()
                # The reply's words, once the host has kept them.
                reply = None
                for _ in range(120):
                    h = history(t.base)
                    if len(h) > 1 and h[-2]["text"] == "STREAM 40 7" and h[-1].get("kind") == "talk":
                        reply = words_of(h[-1]["text"])
                        break
                    time.sleep(0.25)
                report["sent"] = True
                got = reply and waited(lambda x: reply in x, 30)
                report["reply_shown_ms"] = round((time.monotonic() - start) * 1000) if got else None

        # The chat scrolled up and back, the app's own frame counts.
        adb("shell", "dumpsys", "gfxinfo", APP, "reset")
        for _ in range(6):
            adb("shell", "input", "swipe", "540", "700", "540", "1900", "200")
            time.sleep(0.6)
        for _ in range(6):
            adb("shell", "input", "swipe", "540", "1900", "540", "700", "200")
            time.sleep(0.6)
        g = adb("shell", "dumpsys", "gfxinfo", APP)
        pick = lambda k: (re.search(k + r": ([\d.]+)", g) or [None, None])[1]  # noqa: E731
        report["scroll"] = {"frames": pick("Total frames rendered"), "janky": pick("Janky frames"),
                            "p90_ms": pick("90th percentile"), "p99_ms": pick("99th percentile")}
        report["crashed"] = "FATAL EXCEPTION" in adb("logcat", "-d", "-b", "crash")
        report["strays"] = t.strays()
        if booted:
            adb("emu", "kill")
    finally:
        t.close()
    print(json.dumps(report, indent=1))
    ok = report.get("chat_shown") and report.get("reply_shown_ms") is not None and not report.get("crashed") and not report.get("strays")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
