#!/usr/bin/env python3
"""Pictures of the Android app's Settings > Connections > Other apps on an
emulator against a throwaway Libri (drive_other_apps.py is the web's): one
connector an app signed in to by OAuth, one made by key, their modules, and
the key shown once after Add by key.

Runs inside `nix develop .#android-emulator` (adb and the emulator on PATH);
boots an emulator on SERIAL if none is up. The APK must be a debug build:
  drive_android_other_apps.py LIBRI_DIR APK OUT [SERIAL]
"""

import json
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import smoke_android  # noqa: E402
from check_outside import CALLBACK, http, pkce, sign_in  # noqa: E402
from smoke_android import boot, node, screen, waited  # noqa: E402
from throwaway_libri import Throwaway  # noqa: E402

LIBRI_DIR, APK, OUT = sys.argv[1], sys.argv[2], Path(sys.argv[3])
SERIAL = sys.argv[4] if len(sys.argv) > 4 else "emulator-5618"
smoke_android.SERIAL = SERIAL
APP = "is.libri"
ACTIVITY = f"{APP}/su.libri.MainActivity"
OUT.mkdir(parents=True, exist_ok=True)
adb = smoke_android.adb


def shot(name):
    import subprocess
    png = subprocess.run(["adb", "-s", SERIAL, "exec-out", "screencap", "-p"], capture_output=True).stdout
    (OUT / name).write_bytes(png)


def tap(label, limit=20):
    xml = waited(lambda x: node(x, label), limit)
    at = node(xml, label) if xml else None
    if not at:
        raise SystemExit(f"no {label!r} on screen")
    adb("shell", "input", "tap", str(at[0]), str(at[1]))
    time.sleep(1)


def scroll_to(label, tries=12):
    for _ in range(tries):
        if node(screen(), label):
            return True
        adb("shell", "input", "swipe", "540", "1800", "540", "700", "400")
        time.sleep(0.6)
    return False


def main():
    rig = Throwaway(LIBRI_DIR, prefix="libri-other-apps-android-")
    base, booted = rig.base, False
    try:
        meta = json.loads(http("GET", f"{base}/.well-known/oauth-authorization-server")[2])
        client = json.loads(http("POST", meta["registration_endpoint"],
                                 body={"client_name": "Claude", "redirect_uris": [CALLBACK]})[2])["client_id"]
        verifier, challenge = pkce()
        sign_in(base, client, ["memory", "workers"], verifier, challenge, name="Claude on the phone")
        http("POST", f"{base}/api/connectors", body={"name": "Laptop", "modules": ["memory", "extras"]})

        booted = boot()
        adb("uninstall", APP)
        print(adb("install", APK).strip().splitlines()[-1])
        adb("reverse", f"tcp:{rig.port}", f"tcp:{rig.port}")
        prefs = f'<?xml version="1.0" encoding="utf-8" standalone="yes" ?>\n<map>\n<string name="address">{base}</string>\n</map>\n'
        (rig.root / "work/libri.xml").write_text(prefs)
        adb("push", str(rig.root / "work/libri.xml"), "/data/local/tmp/libri.xml")
        adb("shell", "run-as", APP, "sh", "-c", "'mkdir -p shared_prefs && cp /data/local/tmp/libri.xml shared_prefs/libri.xml'")
        for scheme in ("no", "yes"):
            adb("shell", "cmd", "uimode", "night", scheme)
            adb("shell", "am", "force-stop", APP)
            adb("shell", "am", "start", "-W", "-n", ACTIVITY)
            time.sleep(4)
            tap("Side panel")
            tap("Settings")
            tap("Connections")
            if not scroll_to("Connect an app"):
                raise SystemExit("Other apps not found")
            # Back up so the head of the section shows.
            adb("shell", "input", "swipe", "540", "900", "540", "1500", "400")
            time.sleep(0.8)
            scheme_name = "dark" if scheme == "yes" else "light"
            shot(f"android-other-apps-{scheme_name}.png")
            if scheme == "no":
                scroll_to("Add by key")
                tap("Add by key")
                tap("Name, as Claude Code on the laptop")
                adb("shell", "input", "text", "Claude%sCode")
                time.sleep(0.5)
                # The keyboard covers the form's buttons.
                adb("shell", "input", "keyevent", "111")
                time.sleep(0.8)
                scroll_to("Cancel", tries=4)
                time.sleep(1)
                xml = screen()
                import re
                # The form's Add, not Add by key: the button whose label is the word alone.
                m = re.search(r'text="Add"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', xml)
                x1, y1, x2, y2 = map(int, m.groups())
                adb("shell", "input", "tap", str((x1 + x2) // 2), str((y1 + y2) // 2))
                time.sleep(3)
                scroll_to("Key for Claude Code", tries=4)
                shot("android-other-apps-key.png")
        crashed = "FATAL EXCEPTION" in adb("logcat", "-d", "-b", "crash")
        print("crashed", crashed, "strays", rig.strays())
    finally:
        adb("shell", "cmd", "uimode", "night", "no")
        if booted:
            adb("emu", "kill")
        rig.close()


if __name__ == "__main__":
    main()
