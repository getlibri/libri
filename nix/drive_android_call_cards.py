#!/usr/bin/env python3
"""Proves the Android app's MCP Apps cards in the chat (Card.kt CallCards),
a sent image drawn in the chat, and a named memory's chat opened by
`/memory <name>`, against a throwaway Libri whose stand-in harness calls
card_server.py's `show_greeting` (stand_in_claude.py "GREET name").

In the light and the dark theme: a call heard as it ran opens its card under
it, with the call's input and result; the card's own button calls its tool
on its server; the card's full screen opens over everything and Back closes
it. After a restart the kept call waits behind its pill, and the pill opens
it asked afresh (a read-only tool). An image the turn sent shows as a
picture. A lookup (LOOK) shows its row alone, whose Open shows its card
full screen; `show_card` (SHOWCARD) shows a lookup's card from a fresh call.
`/memory jobs` opens the Jobs memory's chat. Leaves screenshots.

Runs inside `nix develop .#android-emulator`; the APK must be a debug build:
  drive_android_call_cards.py LIBRI_DIR APK SHOTS_DIR [SERIAL]
"""
import json
import re
import struct
import subprocess
import sys
import time
import urllib.request
import zlib
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


def png(w, h, rgb):
    """A plain image of one colour, as a PNG's bytes."""
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def bounds(xml, label):
    """The last node starting with `label`, as (top, bottom) in pixels."""
    m = re.findall(r'(?:text|content-desc)="' + re.escape(label) + r'[^"]*"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', xml)
    if not m:
        return None
    _, y1, _, y2 = map(int, m[-1])
    return y1, y2


def tap(label, xml=None):
    at = s.node(xml or s.screen(), label)
    if at:
        s.adb("shell", "input", "tap", *map(str, at))
    return at


def say(words):
    """Types words in the open composer and sends them."""
    tap("Message")
    time.sleep(0.5)
    s.adb("shell", "input", "text", words.replace(" ", "%s"))
    time.sleep(0.5)
    tap("Send")


def post(base, path, body):
    req = urllib.request.Request(f"{base}/{path}", json.dumps(body).encode(), {"content-type": "application/json"})
    return urllib.request.urlopen(req, timeout=10).read()


def main():
    root = Path(__file__).resolve().parent
    t = None
    booted = False
    try:
        added = Path("/tmp") / f"call-cards-{int(time.time())}.json"
        added.write_text(json.dumps({"mcpServers": {"card-test": {"type": "stdio", "command": sys.executable, "args": [str(root / "card_server.py")]}}}))
        t = Throwaway(LIBRI_DIR, prefix="libri-android-call-cards-", extra={"LIBRI_MCP": str(added)})
        post(t.base, "api/memories", {"name": "Jobs"})
        booted = s.boot()
        s.adb("uninstall", s.APP)
        s.adb("install", APK)
        s.adb("reverse", f"tcp:{t.port}", f"tcp:{t.port}")
        prefs = f'<?xml version="1.0" encoding="utf-8" standalone="yes" ?>\n<map>\n<string name="address">{t.base}</string>\n</map>\n'
        (t.root / "work/libri.xml").write_text(prefs)
        s.adb("push", str(t.root / "work/libri.xml"), "/data/local/tmp/libri.xml")
        s.adb("shell", "run-as", s.APP, "sh", "-c", "'mkdir -p shared_prefs && cp /data/local/tmp/libri.xml shared_prefs/libri.xml'")
        for n, theme in enumerate(("light", "dark")):
            s.adb("shell", "cmd", "uimode", "night", "yes" if theme == "dark" else "no")
            s.adb("shell", "am", "force-stop", s.APP)
            s.adb("shell", "am", "start", "-W", "-n", s.ACTIVITY)
            check(f"{theme}: the chat opens", s.waited(lambda x: 'text="Message' in x or "Message Libri" in x, 60) is not None)
            time.sleep(2)
            # The turn leaves a picture in its outbox, which goes with its answer.
            outbox = t.root / "state/outbox"
            outbox.mkdir(exist_ok=True)
            (outbox / f"2026111{n}-swatch-{theme}.png").write_bytes(png(240, 160, (200, 90, 40) if theme == "light" else (40, 120, 200)))
            name = "Ada" if theme == "light" else "Bo"
            say(f"GREET {name}")
            card = s.waited(lambda x: f"Hello, {name}" in x and "Text result" in x, 60)
            check(f"{theme}: the heard call's card shows its input and result", card is not None)
            time.sleep(1)
            shot(f"android-card-{theme}-inline")
            counted = False
            if card and tap("Count", card):
                # A card's call of its server's tool asks the person first, as
                # any connection's tool on Ask does.
                asked = s.waited(lambda x: "The card-test card asks to run count" in x, 20)
                check(f"{theme}: the card's call asks first", asked is not None)
                if asked:
                    tap("Allow", asked)
                    counted = s.waited(lambda x: "Count: 1" in x, 20) is not None
            check(f"{theme}: the card's button calls its tool on its server", counted)
            check(f"{theme}: the sent image shows as a picture, not a file to save",
                  s.waited(lambda x: f"swatch-{theme}.png" in x and "Tap to save" not in x, 20) is not None)
            shot(f"android-card-{theme}-counted")
            xml = s.screen()
            full = tap("Full screen", xml)
            check(f"{theme}: the card has its full-screen button", full is not None)
            if full:
                big = s.waited(lambda x: "Close full screen" in x and f"Hello, {name}" in x, 30)
                check(f"{theme}: full screen opens the card over everything", big is not None)
                shot(f"android-card-{theme}-full")
                s.adb("shell", "input", "keyevent", "KEYCODE_BACK")
                time.sleep(1.5)
                check(f"{theme}: Back closes full screen", "Close full screen" not in s.screen())
        # A restart reads the chat back from the record: the kept call waits
        # behind its pill, which asks the read-only tool afresh.
        s.adb("shell", "am", "force-stop", s.APP)
        s.adb("shell", "am", "start", "-W", "-n", s.ACTIVITY)
        pill = s.waited(lambda x: "Show the card-test card" in x, 60)
        check("a kept call's card waits behind its pill", pill is not None)
        shot("android-card-dark-kept")
        if pill:
            tap("Show the card-test card", pill)
            asked = s.waited(lambda x: "Structured result" in x or "asks to run show_greeting" in x, 40)
            if asked and "asks to run" in asked:
                tap("Allow", asked)
                asked = s.waited(lambda x: "Structured result" in x, 40)
            check("the pill opens the card, asked afresh", asked is not None)
            shot("android-card-dark-fresh")
        # A lookup the model makes for itself shows its row alone: no card,
        # no pill. Its row's Open shows the card full screen on asking.
        frames = lambda x: x.count("Text result") + x.count("Structured result")
        was = s.screen()
        before, pills = frames(was), was.count("Show the card-test card")
        say("LOOK Di")
        looked = s.waited(lambda x: "Looked." in x and "look up greeting" in x, 60)
        check("a lookup's call shows its row", looked is not None)
        time.sleep(2)
        xml = s.screen()
        check("a lookup shows no card", "Hello, Di" not in xml, frames(xml) - before)
        check("a lookup shows no pill", xml.count("Show the card-test card") <= pills, xml.count("Show the card-test card"))
        shot("android-lookup-dark")
        # Opening the row brings its details into view above the composer;
        # closing it keeps the row where it was.
        row_at = bounds(xml, "card-test · look up greeting")
        tap("card-test · look up greeting", xml)
        time.sleep(1.5)
        xml = s.screen()
        detail, box = bounds(xml, "Looked up Di"), bounds(xml, "Message Libri")
        check("an opened row's details come into view", detail is not None and box is not None and detail[1] <= box[0], (detail, box))
        shot("android-lookup-dark-row-open")
        row_open = bounds(xml, "card-test · look up greeting")
        tap("card-test · look up greeting", xml)
        time.sleep(1.5)
        xml = s.screen()
        row_shut = bounds(xml, "card-test · look up greeting")
        # It stays put; at the talk's very foot, where nothing lies below to
        # scroll into, it goes back to where it stood when tapped open.
        check("closing the row keeps it where it was", row_open and row_shut and (abs(row_open[0] - row_shut[0]) < 12 or abs(row_at[0] - row_shut[0]) < 12),
              (row_at, row_open, row_shut))
        # The row's own Open, on its line: a card's "Open spec" is another.
        row = bounds(xml, "card-test · look up greeting")
        opens = [tuple(map(int, b)) for b in re.findall(r'text="Open"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', xml)]
        mine = [o for o in opens if row and o[1] < row[1] and o[3] > row[0]]
        opened = None
        if looked and mine:
            x1, y1, x2, y2 = mine[-1]
            s.adb("shell", "input", "tap", str((x1 + x2) // 2), str((y1 + y2) // 2))
            opened = True
        big = s.waited(lambda x: "Close full screen" in x and "Hello, Di" in x, 30) if opened else None
        check("the lookup's row opens its card full screen", big is not None)
        shot("android-lookup-dark-opened")
        s.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        time.sleep(1.5)
        # The model asks to show it (show_card): a fresh call draws it.
        say("SHOWCARD Eve")
        asked = s.waited(lambda x: "Hello, Eve" in x or "asks to run look_up_greeting" in x, 60)
        if asked and "asks to run" in asked:
            tap("Allow", asked)
            asked = s.waited(lambda x: "Hello, Eve" in x and "Structured result" in x, 40)
        check("show_card shows the card it names, called afresh", asked is not None)
        shot("android-show-card-dark")
        # `/memory jobs` opens that memory's chat.
        say("/memory jobs")
        check("/memory jobs opens the Jobs chat", s.waited(lambda x: "Message Jobs" in x, 20) is not None)
        shot("android-memory-jobs-dark")
        s.adb("shell", "input", "keyevent", "KEYCODE_BACK")
        time.sleep(1)
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
