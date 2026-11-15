#!/usr/bin/env python3
"""How the Android app scrolls a page held in a WebView: a Library page and
the Calendar built-in, each an MCP App in the host's sandbox, as a phone has
them. A finger is never straight, so each vertical swipe here drifts sideways
by 0, 30, 60 and 120 px (the touch slop is 8 dp, 22 px at 440 dpi). The
views carry a probe that tells the stand-in host how far each swipe scrolled
the page and whether the WebView was sent a touchcancel, which is what a
gesture of the app's taking the swipe away does. Prints one JSON report, with
the app's own frame counts (dumpsys gfxinfo). A measure, not a check: it
fails nothing, and the numbers are compared before and after a change.

Runs inside `nix develop .#android-emulator` (adb on PATH, an emulator
running or bootable) with Playwright's Python for the stand-in host:
  measure_android_scroll.py SITE_DIR APK [SERIAL]
"""
import functools
import http.server
import json
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

# An emulator starts only when the host has room for it, and at most three
# run at once (nix/with_memory.sh): each boots a 3 GB guest beside the
# workers' builds. A drive run bare re-runs itself through the wrapper.
if os.environ.get("LIBRI_EMULATOR_ADMITTED") != "1":
    os.environ["LIBRI_EMULATOR_ADMITTED"] = "1"
    _wrap = Path(__file__).resolve().parent / "with_memory.sh"
    os.execv(str(_wrap), [str(_wrap), "5", "--emulator", sys.executable, *sys.argv])
from urllib.parse import parse_qs, urlparse

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
SITE_DIR, APK = sys.argv[1], sys.argv[2]
SERIAL = sys.argv[3] if len(sys.argv) > 3 else "emulator-5614"
sys.argv = [sys.argv[0], SITE_DIR, "serve"]
import measure_ui_performance as stand_in  # noqa: E402  the stand-in host of real size

APP = "is.libri"
ROOT = HERE.parent
SANDBOX = re.search(r'pub const SANDBOX: &str = r#"(.*?)"#;', (ROOT / "libri/src/apps.rs").read_text(), re.S).group(1)
CALENDAR = (ROOT / "libri/src/views/calendar.html").read_text()
DRIFTS = [0, 30, 60, 120]

# Told to the host on every scrollend and touchcancel; the host keeps the last.
PROBE = """<script>
(() => {
  const at = { y: 0, cancels: 0, ends: 0, moves: 0 };
  const tell = () => { at.y = Math.round(scrollY); fetch('/api/probe?' + new URLSearchParams(at), { method: 'POST' }); };
  addEventListener('touchmove', () => { at.moves++; }, { passive: true });
  addEventListener('touchcancel', () => { at.cancels++; tell(); }, { passive: true });
  addEventListener('touchend', () => { at.ends++; setTimeout(tell, 900); }, { passive: true });
  addEventListener('scrollend', tell);
})();
</script>"""

PAGE = "<!doctype html><html><head><meta charset='utf-8'><meta name='viewport' content='width=device-width, initial-scale=1'>" \
    "<style>body{margin:0;font:16px/1.5 system-ui;padding:12px 20px}section{padding:14px 0;border-bottom:1px solid #8883}</style></head><body>" \
    + "".join(f"<section><h3>Step {k}</h3><p>{'A plan step with enough words to wrap over two or three lines on a phone. ' * 3}</p></section>" for k in range(120)) \
    + "</body></html>"


def agenda():
    day = time.strftime("%Y-%m-%d")
    base = time.mktime(time.strptime(day, "%Y-%m-%d"))
    events = []
    for d in range(14):
        for h in range(8, 20, 2):
            t = time.strftime("%Y-%m-%dT%H:00:00", time.localtime(base + d * 86400 + h * 3600))
            events.append({"start": t, "time_zone": "UTC", "duration": "PT1H", "title": f"Meeting {d}-{h}", "location": "Room 4", "all_day": False})
    return {"content": [{"type": "text", "text": "{}"}], "structuredContent": {"events": events}}


class Host(stand_in.Host):
    probe = {}

    def do_GET(self):
        path = urlparse(self.path).path
        if path.endswith("/api/apps/sandbox"):
            body = SANDBOX.encode()
            self.send_response(200)
            self.send_header("content-type", "text/html")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            return self.wfile.write(body)
        if path.endswith("/api/apps/view"):
            uri = parse_qs(urlparse(self.path).query)["uri"][0]
            html = CALENDAR if uri.endswith("/builtins/calendar") else PAGE
            return self.json({"html": html.replace("</body>", PROBE + "</body>"), "csp": None, "permissions": None})
        return super().do_GET()

    def do_POST(self):
        u = urlparse(self.path)
        if u.path.endswith("/api/probe"):
            Host.probe = {k: int(v[0]) for k, v in parse_qs(u.query).items()}
            return self.json({})
        if "/api/builtins/calendar/call" in u.path:
            self.rfile.read(int(self.headers.get("content-length") or 0))
            return self.json(agenda())
        return super().do_POST()


# The stand-in answers from its own class's table, so the changes go there.
stand_in.Host.replies["/api/library"] = {"lists": [], "stills": False, "pages": [{"changed": stand_in.NOW, "name": "scroll-test", "title": "Scroll test"}]}
# A tool with a card, and a call of it last in the chat, whose Open shows
# the card full screen (Card.kt, the site's #card=).
CARD = "mcp__board__plan_board"
stand_in.Host.replies["/api/apps"] = {"cards": {CARD: {"server": "board", "tool": "plan_board", "uri": "ui://board/plan", "definition": {"name": "plan_board", "inputSchema": {"type": "object"}}}}}
stand_in.Host.replies["/api/history"].append({"i": 2000, "kind": "tool", "text": CARD + " {}", "date": "2026-11-01T12:42:00Z", "reaction": None})
stand_in.Host.replies["/api/builtins"] = [
    {"id": "calendar", "title": "Calendar", "about": "Your calendars and events.", "on": True, "connection": "fastmail", "provider": "fastmail", "signed_in": True,
     "providers": [{"id": "fastmail", "title": "Fastmail", "agenda": True, "found": None, "note": None}]},
]


def adb(*args, check=False):
    return subprocess.run(["adb", "-s", SERIAL, *args], capture_output=True, text=True, check=check).stdout


def boot():
    if adb("get-state").strip() == "device":
        return
    home = os.environ["ANDROID_HOME"]
    image = sorted(Path(home, "system-images").glob("*/*/x86_64"))[0]
    package = ";".join(image.relative_to(home).parts)
    subprocess.run(f"echo no | avdmanager create avd --force -n scroll -k '{package}' -d pixel_6", shell=True, capture_output=True)
    port = SERIAL.split("-")[1]
    subprocess.Popen(["emulator", "-avd", "scroll", "-port", port, "-cores", "4", "-memory", "3072", "-partition-size", "4096", "-no-window", "-no-audio",
                      "-no-boot-anim", "-no-snapshot", "-accel", "on", "-gpu", "swiftshader_indirect"], stdout=open("/tmp/measure_android_scroll_emulator.log", "w"), stderr=subprocess.STDOUT)
    adb("wait-for-device")
    for _ in range(120):
        if adb("shell", "getprop", "sys.boot_completed").strip() == "1":
            return
        time.sleep(3)
    raise SystemExit("the emulator did not boot")


def tap(label):
    for _ in range(10):
        adb("shell", "uiautomator", "dump", "/data/local/tmp/ui.xml")
        xml = adb("shell", "cat", "/data/local/tmp/ui.xml")
        m = re.findall(r'(?:text|content-desc)="' + re.escape(label) + r'"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', xml)
        if m:
            x1, y1, x2, y2 = map(int, m[-1])
            adb("shell", "input", "tap", str((x1 + x2) // 2), str((y1 + y2) // 2))
            return
        time.sleep(1)
    raise SystemExit(f"no node: {label}")


def swipes(name):
    """Each drift, three times: up the page by 900 px in 250 ms, then back."""
    out = {}
    for drift in DRIFTS:
        runs = []
        for _ in range(3):
            Host.probe = {}
            adb("shell", "input", "swipe", "540", "1700", str(540 + drift), "800", "250")
            time.sleep(2.5)
            got = dict(Host.probe)
            adb("shell", "input", "swipe", "540", "600", "540", "2000", "120")  # back to the top, no drift
            time.sleep(1.5)
            adb("shell", "input", "swipe", "540", "600", "540", "2000", "120")
            time.sleep(1.5)
            runs.append({"scrolled": got.get("y", 0), "cancelled": got.get("cancels", 0) > 0})
        out[f"drift_{drift}px"] = {"scrolled_px": [r["scrolled"] for r in runs], "cancelled": sum(r["cancelled"] for r in runs), "of": len(runs)}
        print(f"-- {name} drift {drift}: {out[f'drift_{drift}px']}", file=sys.stderr, flush=True)
    return out


def frames():
    g = adb("shell", "dumpsys", "gfxinfo", APP)
    pick = lambda k: (re.search(k + r": ([\d.]+)", g) or [None, None])[1]
    return {"frames": pick("Total frames rendered"), "janky": pick("Janky frames"), "p90_ms": pick("90th percentile"), "p99_ms": pick("99th percentile")}


def main():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Host, directory=SITE_DIR))
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    port = server.server_address[1]
    boot()
    adb("shell", "wm", "density", "440")
    adb("uninstall", APP)
    print(adb("install", APK).strip().splitlines()[-1], file=sys.stderr)
    adb("reverse", f"tcp:{port}", f"tcp:{port}")
    prefs = f'<?xml version="1.0" encoding="utf-8" standalone="yes" ?>\n<map>\n<string name="address">http://127.0.0.1:{port}</string>\n</map>\n'
    Path("/tmp/measure_android_scroll.xml").write_text(prefs)
    adb("push", "/tmp/measure_android_scroll.xml", "/data/local/tmp/libri.xml")
    adb("shell", "run-as", APP, "sh", "-c", "'mkdir -p shared_prefs && cp /data/local/tmp/libri.xml shared_prefs/libri.xml'")
    adb("shell", "am", "start", "-W", "-n", f"{APP}/su.libri.MainActivity")
    time.sleep(8)
    report = {}
    # A swipe that goes across still moves along the bar: Chat to Tasks.
    adb("shell", "input", "swipe", "900", "1200", "200", "1260", "250")
    time.sleep(2)
    adb("shell", "uiautomator", "dump", "/data/local/tmp/ui.xml")
    report["swipe_across_reaches_tasks"] = "Start a worker" in adb("shell", "cat", "/data/local/tmp/ui.xml")
    tap("Chat")
    time.sleep(1)
    tap("Library")
    time.sleep(2)
    tap("Scroll test")
    time.sleep(5)
    adb("shell", "dumpsys", "gfxinfo", APP, "reset")
    report["library_page"] = swipes("library page")
    report["library_page"]["gfx"] = frames()
    adb("shell", "input", "keyevent", "4")
    time.sleep(1)
    tap("Chat")
    time.sleep(1)
    tap("Side panel")
    time.sleep(2)
    tap("Calendar")
    time.sleep(5)
    adb("shell", "dumpsys", "gfxinfo", APP, "reset")
    report["calendar"] = swipes("calendar")
    report["calendar"]["gfx"] = frames()
    # Back from the Calendar comes to the chat, and only from the chat
    # does Back leave the app.
    adb("shell", "input", "keyevent", "4")
    time.sleep(2)
    report["back_from_calendar_reaches_chat"] = in_front() and "Temp chat, not remembered" in ui()
    if not report["back_from_calendar_reaches_chat"]:
        adb("shell", "am", "start", "-W", "-n", f"{APP}/su.libri.MainActivity")
        time.sleep(5)
    tap("Open")
    time.sleep(5)
    adb("shell", "dumpsys", "gfxinfo", APP, "reset")
    report["full_card"] = swipes("full card")
    report["full_card"]["gfx"] = frames()
    adb("shell", "input", "keyevent", "4")
    time.sleep(2)
    report["back_from_card_reaches_chat"] = in_front() and "Temp chat, not remembered" in ui()
    if not report["back_from_card_reaches_chat"]:
        adb("shell", "am", "start", "-W", "-n", f"{APP}/su.libri.MainActivity")
        time.sleep(5)
    adb("shell", "input", "keyevent", "4")
    time.sleep(2)
    report["back_from_chat_leaves"] = not in_front()
    print(json.dumps(report))


def ui():
    adb("shell", "uiautomator", "dump", "/data/local/tmp/ui.xml")
    got = adb("shell", "cat", "/data/local/tmp/ui.xml")
    print("-- ui:", sorted(set(re.findall(r'(?:text|content-desc)="([^"]+)"', got))), file=sys.stderr, flush=True)
    return got


def in_front():
    print("-- focus:", (re.search(r"mCurrentFocus=.*", adb("shell", "dumpsys", "window")) or [""])[0], file=sys.stderr, flush=True)
    return APP + "/" in (re.search(r"mCurrentFocus=.*", adb("shell", "dumpsys", "window")) or [""])[0]


if __name__ == "__main__":
    main()
