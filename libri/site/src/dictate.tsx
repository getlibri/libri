import { useEffect, useRef, useState } from "react";
import { url } from "./api";

// Dictation, as ChatGPT's composer has it: tap, speak, tap again, and the
// words join the composer to be read over before they go. The microphone
// streams to the host (stt.rs), which writes it down with the engine chosen
// in Settings, Voice: its own model, showing the words while they are
// spoken, or a service, a phrase at a time. Nothing is kept.

type State = "idle" | "hearing" | "writing";

// How long a failure stays over the composer.
const SHOWN_MS = 4000;
// The rate the host's speech engine takes.
const RATE = 16000;

/** `words` after what is already in the composer. */
export const joined = (text: string, words: string) => (!text || /\s$/.test(text) ? text + words : `${text} ${words}`);

// Hands the microphone to the page as 100 ms of 16 kHz samples, averaged
// down from whatever rate the device records at.
const CAPTURE = `
class Capture extends AudioWorkletProcessor {
  constructor() { super(); this.step = sampleRate / ${RATE}; this.at = 0; this.sum = 0; this.n = 0; this.buf = new Float32Array(${RATE / 10}); this.k = 0; }
  process([input]) {
    const ch = input[0];
    if (ch) for (let i = 0; i < ch.length; i++) {
      this.sum += ch[i]; this.n++; this.at++;
      if (this.at >= this.step) {
        this.at -= this.step;
        this.buf[this.k++] = this.sum / this.n; this.sum = 0; this.n = 0;
        if (this.k === this.buf.length) { this.port.postMessage(this.buf.slice()); this.k = 0; }
      }
    }
    return true;
  }
}
registerProcessor("capture", Capture);
`;

function pcm16(f: Float32Array): ArrayBuffer {
  const out = new Int16Array(f.length);
  for (let i = 0; i < f.length; i++) out[i] = Math.max(-1, Math.min(1, f[i])) * 0x7fff;
  return out.buffer;
}

function address(path: string) {
  const u = new URL(url(path, true), location.href);
  u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
  return u.href;
}

type Heard =
  | { type: "ready"; provider: string; model: string }
  | { type: "speech_started"; utterance: number }
  | { type: "partial"; utterance: number; text: string }
  | { type: "final"; utterance: number; text: string }
  | { type: "done" }
  | { type: "error"; message: string };

/** Speech written down while it is spoken: `live` is everything so far,
 * `start` opens the microphone, `stop` ends it and gives the words to `done`
 * (or the reason to `failed`). */
export function useListen(done: (words: string) => void, failed: (why: string) => void) {
  const [state, setState] = useState<State>("idle");
  const [live, setLive] = useState("");
  const end = useRef<(() => void) | null>(null);
  const cancel = useRef<(() => void) | null>(null);
  const said = useRef({ done, failed });
  said.current = { done, failed };

  useEffect(() => () => cancel.current?.(), []);

  const start = async () => {
    // A browser only offers the microphone over https.
    if (!navigator.mediaDevices) return said.current.failed("The microphone needs https");
    setState("hearing");
    setLive("");
    let over = false;
    let stream: MediaStream | undefined;
    let ctx: AudioContext | undefined;
    let ws: WebSocket | undefined;
    const early: ArrayBuffer[] = [];
    const finals: string[] = [];
    let partial = "";
    const words = () => [...finals, partial].filter(Boolean).join(" ");
    const close = () => {
      over = true;
      end.current = null;
      cancel.current = null;
      stream?.getTracks().forEach((t) => t.stop());
      void ctx?.close();
      ws?.close();
    };
    const fail = (why: string) => {
      if (over) return;
      close();
      setState("idle");
      setLive("");
      said.current.failed(why);
    };
    cancel.current = close;
    end.current = () => {
      stream?.getTracks().forEach((t) => t.stop());
      void ctx?.close();
      setState("writing");
      if (ws?.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ type: "end" }));
      else fail("The host does not answer");
    };
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
    } catch (e) {
      console.warn("dictate:", e);
      return fail("The microphone is blocked");
    }
    if (over) return close();
    ws = new WebSocket(address("api/stt/stream?silence_ms=1200"));
    ws.binaryType = "arraybuffer";
    ws.onopen = () => early.splice(0).forEach((f) => ws!.send(f));
    ws.onmessage = (e) => {
      if (typeof e.data !== "string") return;
      const m = JSON.parse(e.data) as Heard;
      if (m.type === "partial") partial = m.text;
      else if (m.type === "final") {
        finals.push(m.text);
        partial = "";
      } else if (m.type === "error") return fail(m.message);
      else if (m.type === "done") {
        const all = words().trim();
        close();
        setState("idle");
        setLive("");
        return all ? said.current.done(all) : said.current.failed("Nothing heard");
      }
      if (!over) setLive(words());
    };
    ws.onclose = () => fail("The host stopped listening");
    ctx = new AudioContext();
    await ctx.audioWorklet.addModule(URL.createObjectURL(new Blob([CAPTURE], { type: "text/javascript" })));
    if (over) return;
    const capture = new AudioWorkletNode(ctx, "capture");
    ctx.createMediaStreamSource(stream).connect(capture);
    capture.port.onmessage = (e: MessageEvent<Float32Array>) => {
      const frame = pcm16(e.data);
      if (ws?.readyState === WebSocket.OPEN) ws.send(frame);
      else if (early.length < 600) early.push(frame);
    };
  };

  return {
    state,
    live,
    start: () => void start(),
    stop: () => end.current?.(),
  };
}

/** The microphone button. `add` takes the words; `hush` quiets anything
 * playing, which the microphone would otherwise hear. */
export function Dictate({ add, hush }: { add: (words: string) => void; hush?: () => void }) {
  const [failed, setFailed] = useState("");
  const addNow = useRef(add);
  addNow.current = add;
  const { state, live, start, stop } = useListen((words) => addNow.current(words), setFailed);

  useEffect(() => {
    if (!failed) return;
    const t = window.setTimeout(() => setFailed(""), SHOWN_MS);
    return () => window.clearTimeout(t);
  }, [failed]);

  const hearing = state === "hearing";
  const label = hearing ? "Stop dictating" : state === "writing" ? "Writing it down" : "Dictate";
  const shown = failed || live;
  return (
    <>
      {shown && (
        <p role="status" className={`absolute right-2 bottom-[calc(100%+6px)] z-20 max-w-[min(32rem,calc(100%-1rem))] rounded-2xl border border-frame bg-lift px-3 py-1.5 text-control leading-5 shadow-lift ${failed ? "text-mid" : "text-ink"}`}>
          {shown}
        </p>
      )}
      <button
        type="button"
        // The text box keeps its focus, and a phone its keyboard.
        onPointerDown={(e) => e.preventDefault()}
        onClick={() => {
          if (hearing) stop();
          else if (state === "idle") {
            hush?.();
            setFailed("");
            start();
          }
        }}
        disabled={state === "writing"}
        aria-label={label}
        aria-pressed={hearing}
        title={label}
        className={`grid size-[38px] shrink-0 place-items-center rounded-full disabled:opacity-40 ${hearing ? "bg-ink text-paper" : "text-mid hover:bg-hover hover:text-ink"}`}
      >
        <svg viewBox="0 0 24 24" className="size-[18px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
          <rect x="9" y="3" width="6" height="11" rx="3" />
          <path d="M5 11a7 7 0 0 0 14 0M12 18v3" />
        </svg>
      </button>
    </>
  );
}
