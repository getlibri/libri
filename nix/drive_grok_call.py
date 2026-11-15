#!/usr/bin/env python3
"""A live call with Grok Voice in front, driven in Chromium against a
throwaway Libri whose chat is the stand-in Claude: the page's microphone
plays call_speech.wav ("What is on my calendar tomorrow?"), Grok hears it,
hands it to the chat with ask_chat (the host's /api/call/ask), the chat's
answer goes back to Grok, and Grok says it. On hang-up the call is saved
without the turns that went to the chat, which already holds them.

It talks to xAI for real, so it runs by hand, not with the checks, and
costs a call of a few seconds. XAI_TOKEN is a bearer token for xAI: an API
key, or a signed-in host's current access token (given to the throwaway
as a key, so it never refreshes the sign-in it came from).

Usage: XAI_TOKEN=... drive_grok_call.py LIBRI_DIR
"""

import json
import os
import re
import struct
import sys
import time
import urllib.request
import wave
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from check_call_chat import SPEECH, check, failed, tone, wav_bytes  # noqa: E402
from throwaway_libri import Throwaway, until  # noqa: E402

# Records the realtime socket's events and the page's first sent audio.
RECORD = """
  window.__events = [];
  const Native = window.WebSocket;
  window.WebSocket = function (url, protocols) {
    const ws = protocols === undefined ? new Native(url) : new Native(url, protocols);
    if (String(url).includes("x.ai")) {
      ws.addEventListener("message", (e) => {
        if (typeof e.data !== "string") {
          window.__firstAudio ??= performance.now();
          window.__audioAfterAnswer ??= window.__answered === undefined ? undefined : performance.now();
          return;
        }
        const ev = JSON.parse(e.data);
        window.__events.push({ t: performance.now(), type: ev.type, name: ev.name, arguments: ev.arguments, transcript: ev.transcript, error: ev.error });
      });
      const send = ws.send.bind(ws);
      ws.send = (data) => {
        if (typeof data !== "string") window.__micStart ??= performance.now();
        else if (data.includes("function_call_output")) window.__answered = performance.now();
        return send(data);
      };
    }
    return ws;
  };
  window.WebSocket.prototype = Native.prototype;
  Object.assign(window.WebSocket, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 });
  localStorage.setItem("callMode", "grok");
"""


def main():
    from playwright.sync_api import sync_playwright

    token = os.environ["XAI_TOKEN"]
    box = Throwaway(sys.argv[1], prefix="libri-grok-call-")
    conn = box.root / "state" / "connections"
    conn.mkdir(mode=0o700, exist_ok=True)
    (conn / "xai.json").write_text(json.dumps({"how": "key", "key": token}))
    (conn / "xai.json").chmod(0o600)
    with wave.open(str(SPEECH)) as w:
        rate = w.getframerate()
        said = list(struct.unpack(f"<{w.getnframes()}h", w.readframes(w.getnframes())))
    mic = box.root / "mic.wav"
    mic.write_bytes(wav_bytes(tone(1.0, rate) + said + tone(40.0, rate), rate))
    speech_ends = (1.0 + len(said) / rate) * 1000
    try:
        with sync_playwright() as p:
            browser = p.chromium.launch(
                headless=True,
                args=[
                    "--use-fake-ui-for-media-stream",
                    "--use-fake-device-for-media-stream",
                    f"--use-file-for-fake-audio-capture={mic}",
                    "--autoplay-policy=no-user-gesture-required",
                ],
            )
            page = browser.new_page(viewport={"width": 1280, "height": 900})
            errors = []
            page.on("pageerror", lambda e: errors.append(str(e)))
            page.on("console", lambda m: m.type in ("warning", "error") and errors.append(m.text))
            page.add_init_script(RECORD)
            page.goto(box.base)
            page.get_by_role("button", name=re.compile(r"^Call ")).click()

            def events():
                return page.evaluate("window.__events")

            def asked():
                return next((e for e in events() if e["type"] == "response.function_call_arguments.done"), None)

            try:
                until(asked, "Grok hands the request to the chat", wait=40)
            except TimeoutError:
                print("events:", [(e["type"], e.get("error")) for e in events()])
                print("page:", errors)
                raise
            ask = asked()
            check("Grok calls ask_chat", ask["name"] == "ask_chat", ask)
            request = json.loads(ask["arguments"] or "{}").get("request", "")
            print("request:", request)
            check("with the person's request", "calendar" in request.lower(), request)
            until(lambda: page.evaluate("window.__answered") is not None, "the chat answers", wait=60)
            until(lambda: page.evaluate("window.__audioAfterAnswer") is not None, "Grok speaks the answer", wait=30)

            def spoken():
                return [e["transcript"] for e in events() if e["type"] == "response.output_audio_transcript.done" and e["transcript"]]

            until(lambda: len(spoken()) >= 1 and page.evaluate("window.__events.at(-1).type") != "", "Grok's words", wait=30)
            page.wait_for_timeout(4000)
            words = spoken()
            print("Grok said:", words)
            # The stand-in's answer only echoes the request, so Grok's words for it
            # vary; that it speaks again after the answer is what shows it.
            check("Grok speaks again once the chat answers", len(words) >= 2, words)
            history = json.dumps(json.load(urllib.request.urlopen(f"{box.base}/api/history")))
            check("the chat holds the request marked (call)", json.dumps(f"(call) {request}")[1:-1] in history)
            at = page.evaluate("[window.__micStart, window.__firstAudio, window.__answered, window.__audioAfterAnswer]")
            mic0, first, answered, after = at
            heard_at = next(e["t"] for e in events() if e["type"] == "response.function_call_arguments.done")
            print(f"time: speech end -> ask_chat {(heard_at - mic0 - speech_ends) / 1000:.2f}s")
            print(f"time: speech end -> Grok's first audio {(first - mic0 - speech_ends) / 1000:.2f}s")
            print(f"time: ask_chat -> the chat's answer {(answered - heard_at) / 1000:.2f}s")
            print(f"time: the chat's answer -> Grok says it {(after - answered) / 1000:.2f}s")
            page.get_by_role("button", name="Hang up").click()
            page.wait_for_timeout(1500)
            history = json.dumps(json.load(urllib.request.urlopen(f"{box.base}/api/history")))
            # Every turn went through ask_chat, which the chat already holds,
            # so the saved call adds none of them: not Grok's words around
            # the handing over, nor the call's note.
            check("the saved call leaves out the handed-over turns",
                  not any(json.dumps(f"(call) {w}")[1:-1] in history for w in words) and "a voice call" not in history, history[-1500:])
            check("no page errors", not [e for e in errors if "call:" not in e], errors)
            if errors:
                print("page:", errors)
            browser.close()
    finally:
        box.close(keep=bool(failed))
    print("all passed" if not failed else f"{len(failed)} failed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
