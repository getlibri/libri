#!/usr/bin/env python3
"""Proves a call with the chat on the Android app (Call.kt ChatCall, with
CallService) on an emulator, against a throwaway Libri whose hearing and
voice are check_call_chat.py's stand-in speech service.

The Call button shows with no xAI voice, since the host says it can call;
the call opens and listens; CallService runs in the foreground as a
microphone service; with the screen off the microphone is not silenced,
and a reply the chat gives (a message sent to it meanwhile) is spoken and
reaches the phone; on hang-up the service stops. Leaves screenshots.

The emulator runs with audio (smoke_android.py's has none), so it is its
own, booted here on SERIAL and shut down after.

Runs inside `nix develop .#android-emulator`; the APK must be a debug build:
  drive_android_call.py LIBRI_DIR APK SILERO_VAD SHOTS_DIR [SERIAL]
"""
import json
import os
import re
import subprocess
import sys
import threading
import time
import urllib.request
from http.server import ThreadingHTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from check_call_chat import Speech, events  # noqa: E402
from throwaway_libri import Throwaway, free_port  # noqa: E402

LIBRI_DIR, APK, VAD, SHOTS = sys.argv[1], sys.argv[2], Path(sys.argv[3]), Path(sys.argv[4])
SERIAL = sys.argv[5] if len(sys.argv) > 5 else "emulator-5620"
APP = "is.libri"
ACTIVITY = f"{APP}/su.libri.MainActivity"
failures = []


def check(what, ok, detail=""):
    print(f"{'PASS' if ok else 'FAIL'} {what} {'' if ok else str(detail)[:600]}", flush=True)
    if not ok:
        failures.append(what)


def adb(*args):
    return subprocess.run(["adb", "-s", SERIAL, *args], capture_output=True, text=True).stdout


def shot(name):
    SHOTS.mkdir(parents=True, exist_ok=True)
    with open(SHOTS / f"{name}.png", "wb") as f:
        subprocess.run(["adb", "-s", SERIAL, "exec-out", "screencap", "-p"], stdout=f)


def boot():
    home = os.environ["ANDROID_HOME"]
    image = sorted(Path(home, "system-images").glob("*/*/x86_64"))[0]
    package = ";".join(image.relative_to(home).parts)
    subprocess.run(f"echo no | avdmanager create avd --force -n libri-call -k '{package}' -d pixel_6", shell=True, capture_output=True)
    log = open(SHOTS / "emulator.log", "w")
    subprocess.Popen(["emulator", "-avd", "libri-call", "-port", SERIAL.split("-")[1], "-cores", "4", "-memory", "3072", "-no-window",
                      "-no-boot-anim", "-no-snapshot", "-accel", "on", "-gpu", "swiftshader_indirect"], stdout=log, stderr=subprocess.STDOUT)
    adb("wait-for-device")
    for _ in range(120):
        if adb("shell", "getprop", "sys.boot_completed").strip() == "1":
            return
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
        got = test()
        if got:
            return got
        time.sleep(0.5)
    return None


def call_service():
    """CallService's line in the activity manager, or none when it is not running."""
    out = adb("shell", "dumpsys", "activity", "services", f"{APP}/su.libri.CallService")
    return out if "ServiceRecord" in out else None


def recording():
    """The app's microphone use as the audio service sees it."""
    out = adb("shell", "dumpsys", "audio")
    lines = [l for l in out.splitlines() if APP in l and ("rec" in l.lower() or "silenced" in l.lower())]
    return "\n".join(lines)


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    port = free_port()
    server = ThreadingHTTPServer(("127.0.0.1", port), Speech)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    box = Throwaway(LIBRI_DIR, prefix="libri-android-call-", extra={"LIBRI_SPEECH_API": f"http://127.0.0.1:{port}/v1"})
    state = box.root / "state"
    (state / "stt" / "models").mkdir(parents=True, exist_ok=True)
    (state / "stt" / "models" / "silero_vad.onnx").symlink_to(VAD.resolve())
    (state / "stt.json").write_text(json.dumps({
        "provider": "openai", "model": "nemotron-en",
        "openai_url": f"http://127.0.0.1:{port}/v1", "openai_model": "whisper-1",
    }))
    (state / "stt-openai-key").write_text("stand-in")
    try:
        boot()
        adb("uninstall", APP)
        check("the APK installs", "Success" in adb("install", APK))
        adb("reverse", f"tcp:{box.port}", f"tcp:{box.port}")
        prefs = f'<?xml version="1.0" encoding="utf-8" standalone="yes" ?>\n<map>\n<string name="address">{box.base}</string>\n</map>\n'
        (box.root / "work" / "libri.xml").write_text(prefs)
        adb("push", str(box.root / "work" / "libri.xml"), "/data/local/tmp/libri.xml")
        adb("shell", "run-as", APP, "sh", "-c", "'mkdir -p shared_prefs && cp /data/local/tmp/libri.xml shared_prefs/libri.xml'")
        for p in ("RECORD_AUDIO", "POST_NOTIFICATIONS"):
            adb("shell", "pm", "grant", APP, f"android.permission.{p}")
        adb("shell", "am", "start", "-W", "-n", ACTIVITY)

        call = waited(lambda: node(screen(), "Call "), 60)
        check("the Call button shows with no xAI voice", call is not None)
        shot("chat")
        if not call:
            return
        adb("shell", "input", "tap", *map(str, call))
        listening = waited(lambda: "Listening" in screen(), 30)
        shot("call")
        check("the call opens and listens", listening)
        service = waited(call_service, 10)
        check("CallService runs in the foreground", service and "isForeground=true" in service, service)
        kind = re.search(r"\btypes=0x([0-9a-f]+)", service or "")
        # ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        check("as a microphone service", kind and int(kind[1], 16) & 0x80,
              [l.strip() for l in (service or "").splitlines() if "oreground" in l])

        adb("shell", "input", "keyevent", "KEYCODE_SLEEP")
        time.sleep(4)
        check("the screen is off", "mWakefulness=Asleep" in adb("shell", "dumpsys", "power") or "Display Power: state=OFF" in adb("shell", "dumpsys", "power"))
        heard = recording()
        print("audio service:", heard, flush=True)
        check("the microphone is heard with the screen off", heard and "silenced:true" not in heard.lower().replace(" ", ""), heard)
        # The chat answers while the screen is off: the voice says it.
        body = json.dumps({"messages": [{"id": f"m{time.monotonic_ns()}", "role": "user", "content": "say hello on the call"}]}).encode()
        req = urllib.request.Request(f"{box.base}/api/run", data=body, headers={"Content-Type": "application/json"})
        threading.Thread(target=lambda: urllib.request.urlopen(req, timeout=60).read(), daemon=True).start()
        spoken = waited(lambda: any("say hello on the call" in s for s in events.get("spoken", [])), 30)
        check("the host speaks the chat's reply with the screen off", spoken, events)
        check("the call still holds the microphone", call_service() is not None)

        adb("shell", "input", "keyevent", "KEYCODE_WAKEUP")
        adb("shell", "wm", "dismiss-keyguard")
        shown = waited(lambda: "claude answers" in screen(), 20)
        shot("spoken")
        check("the phone played the reply (its words are the caption)", shown)
        hang = node(screen(), "Hang up")
        if hang:
            adb("shell", "input", "tap", *map(str, hang))
        stopped = waited(lambda: call_service() is None, 15)
        check("hanging up stops CallService", stopped)
        check("no crash", "FATAL EXCEPTION" not in adb("logcat", "-d", "-b", "crash"))
    finally:
        adb("emu", "kill")
        server.shutdown()
        box.close(keep=bool(failures))
    print("all passed" if not failures else f"{len(failures)} failed: {failures}")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
