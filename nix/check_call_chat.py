"""A call with the chat, driven end to end in Chromium with a recorded
voice: the page's microphone plays call_speech.wav (Kokoro saying "What is
on my calendar tomorrow?"), the host's voice detector (Silero) ends the
utterance, it is written down, the words reach the main chat as the
person's message marked "(call)", the stand-in Claude answers, and the
answer comes back as speech the page plays.

By default the words are written and spoken by a stand-in speech service
with OpenAI's API. With --local MODELS_DIR (a folder holding the
Nemotron model and kokoro-multi-lang-v1_0, as ~/hold/stt-models and
~/hold/tts-models hold them) the host's own models do both, and the
host's timing of each stage is printed: the check to run by hand when
tuning a call's speed.

Usage: check_call_chat.py LIBRI_DIR SILERO_VAD [--local MODELS_DIR]
"""

import io
import re
import json
import math
import struct
import sys
import threading
import time
import urllib.request
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from throwaway_libri import Throwaway, free_port, until  # noqa: E402

HEARD = "what is on my calendar tomorrow"
SPEECH = Path(__file__).parent / "call_speech.wav"
NEMOTRON = "sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25"
KOKORO = "kokoro-multi-lang-v1_0"


def plain(text):
    return re.sub(r"[^a-z ]", "", text.lower()).strip()
failed = []
events = {}


def check(what, ok, seen=None):
    print(("ok   " if ok else "FAIL ") + what + ("" if ok or seen is None else f": {seen}"))
    if not ok:
        failed.append(what)


def tone(seconds, rate, freq=440.0, level=0.0):
    n = int(seconds * rate)
    return [int(level * 32767 * math.sin(2 * math.pi * freq * i / rate)) for i in range(n)]


def wav_bytes(samples, rate):
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(struct.pack(f"<{len(samples)}h", *samples))
    return buf.getvalue()


class Speech(BaseHTTPRequestHandler):
    """A stand-in for a speech service with OpenAI's API."""

    def log_message(self, *a):
        pass

    def do_POST(self):
        size = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(size)
        if self.path.endswith("/audio/transcriptions"):
            events.setdefault("transcribe", time.monotonic())
            events["audio_bytes"] = len(body)
            out = json.dumps({"text": HEARD}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
        elif self.path.endswith("/audio/speech"):
            events.setdefault("speak", time.monotonic())
            events.setdefault("spoken", []).append(json.loads(body)["input"])
            out = wav_bytes(tone(0.3, 24000, level=0.2), 24000)
            self.send_response(200)
            self.send_header("Content-Type", "audio/wav")
        else:
            self.send_response(404)
            out = b""
        self.send_header("Content-Length", str(len(out)))
        self.end_headers()
        self.wfile.write(out)


def main():
    from playwright.sync_api import sync_playwright

    libri, vad = sys.argv[1], Path(sys.argv[2])
    models = Path(sys.argv[sys.argv.index("--local") + 1]) if "--local" in sys.argv else None
    port = free_port()
    server = ThreadingHTTPServer(("127.0.0.1", port), Speech)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    extra = {} if models else {"LIBRI_SPEECH_API": f"http://127.0.0.1:{port}/v1"}
    box = Throwaway(libri, prefix="libri-call-", extra=extra)
    state = box.root / "state"
    (state / "stt" / "models").mkdir(parents=True, exist_ok=True)
    (state / "stt" / "models" / "silero_vad.onnx").symlink_to(vad.resolve())
    if models:
        (state / "stt" / "models" / NEMOTRON).symlink_to((models / NEMOTRON).resolve())
        (state / "tts").mkdir(exist_ok=True)
        (state / "tts" / KOKORO).symlink_to((models / KOKORO).resolve())
        (state / "stt.json").write_text(json.dumps({"provider": "local", "model": "nemotron-en"}))
    else:
        # Hearing follows the dictation choice (stt.rs): the stand-in, as an
        # OpenAI-compatible service, so no local model has to be fetched.
        (state / "stt.json").write_text(json.dumps({
            "provider": "openai", "model": "nemotron-en",
            "openai_url": f"http://127.0.0.1:{port}/v1", "openai_model": "whisper-1",
        }))
        (state / "stt-openai-key").write_text("stand-in")
    # The microphone: a second of quiet, the question, then long quiet, so
    # the ear hears one utterance before Chromium loops the file.
    with wave.open(str(SPEECH)) as w:
        rate = w.getframerate()
        said = list(struct.unpack(f"<{w.getnframes()}h", w.readframes(w.getnframes())))
    mic = box.root / "mic.wav"
    mic.write_bytes(wav_bytes(tone(1.0, rate) + said + tone(20.0, rate), rate))
    speech_ends = 1.0 + len(said) / rate
    try:
        me = json.load(urllib.request.urlopen(f"{box.base}/api/me"))
        check("the host offers a call with a speech service and no xAI", me.get("call") is True, me)
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
            # The call socket's messages, recorded in the page: Playwright's
            # websocket event misses this socket in headless Chromium.
            page.add_init_script("""
              window.__frames = [];
              const Native = window.WebSocket;
              window.WebSocket = function (url, protocols) {
                const ws = protocols === undefined ? new Native(url) : new Native(url, protocols);
                if (String(url).includes("call/socket")) {
                  ws.addEventListener("message", (e) => {
                    const text = typeof e.data === "string";
                    window.__frames.push({ t: performance.now(), text: text ? e.data : null, bytes: text ? 0 : (e.data.size ?? e.data.byteLength ?? 0) });
                  });
                  // The microphone's first sample out: the recording's start.
                  const send = ws.send.bind(ws);
                  ws.send = (data) => {
                    if (typeof data !== "string") window.__micStart ??= performance.now();
                    return send(data);
                  };
                }
                return ws;
              };
              window.WebSocket.prototype = Native.prototype;
              Object.assign(window.WebSocket, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 });
            """)

            def frames():
                return page.evaluate("window.__frames")

            page.goto(box.base)
            page.get_by_role("button", name=re.compile(r"^Call ")).click()
            started = time.monotonic()
            host_log = box.root / "libri.log"

            def heard_frame():
                return any(f["text"] and '"heard"' in f["text"] for f in frames())

            try:
                until(heard_frame, "the ear writes the utterance down", wait=30)
            except TimeoutError:
                print("frames:", frames()[:20])
                print("page errors:", errors)
                print("host log:", host_log.read_text()[-3000:])
                raise
            if not models:
                ear_after = events["transcribe"] - started - speech_ends
                check("the ear sends one utterance after the speech ends", 0 < ear_after < 2.0, f"{ear_after:.2f}s")
                check("the utterance is about the speech's length", 40_000 < events["audio_bytes"] < 140_000, events["audio_bytes"])
            try:
                until(lambda: any(f["bytes"] for f in frames()), "audio comes back", wait=30)
            except TimeoutError:
                print("frames:", frames()[:20])
                print("events:", events)
                print("host log:", host_log.read_text()[-3000:])
                raise
            got = frames()
            first_audio = next(f["t"] for f in got if f["bytes"]) - page.evaluate("window.__micStart")
            texts = [json.loads(f["text"]) for f in got if f["text"] is not None]
            types = [t["type"] for t in texts]
            check("the page hears ready, speech and heard", all(k in types for k in ("ready", "speech", "heard")), types)
            heard = next((t["text"] for t in texts if t["type"] == "heard"), "")
            check("heard carries the words written down", plain(heard) == HEARD, heard)
            said = [t["text"] for t in texts if t["type"] == "say"]
            check("the chat's answer is spoken", any("claude answers" in s for s in said), said)
            check("the answer is spoken once", sum("claude answers" in s for s in said) == 1, [t for t in texts if t["type"] != "ready"])
            if not models:
                check("the voice says the chat's answer, as written", any(HEARD in s for s in events.get("spoken", [])), events.get("spoken"))
            history = json.load(urllib.request.urlopen(f"{box.base}/api/history"))
            words = json.dumps(history)
            check("the chat holds the person's words marked (call)", json.dumps(f"(call) {heard}")[1:-1] in words)
            page.wait_for_timeout(800)
            status = page.locator("p.text-control").first.inner_text()
            check("the call screen speaks or listens", status in ("Speaking", "Listening"), status)
            # The page's clock: from the end of the recorded speech, counted
            # from the microphone's first sample, to the first audio reaching
            # the page. The host's own stages follow.
            print(f"time: speech end -> first audio at the page {first_audio / 1000 - speech_ends:.2f}s")
            for line in host_log.read_text().splitlines():
                if "libri: call:" in line:
                    print("host:", line.split("libri: call: ", 1)[1])
            check("no page errors", not errors, errors)
            page.get_by_role("button", name="Hang up").click()
            browser.close()
    finally:
        server.shutdown()
        box.close(keep=bool(failed))
    print("all passed" if not failed else f"{len(failed)} failed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
