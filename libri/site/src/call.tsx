import { useEffect, useRef, useState } from "react";
import { api, url } from "./api";

// A voice call, in one of two ways (Settings > Voice). With the chat
// (ChatCall), the host hears the person, the main chat answers and the
// host speaks its reply. With Grok Voice in front (GrokCall), the page
// talks with xAI directly on a short-lived pass the site mints, and Grok
// hands anything that needs knowing or doing to the chat with ask_chat;
// when that call ends, what was said goes into hippo.

const REALTIME_URL = "wss://api.x.ai/v1/realtime?model=grok-voice-latest";
const RATE = 24000;

// How fast the assistant speaks, on calls and read aloud; kept on this
// device. Read aloud plays at any of these; xAI takes 0.7 to 1.5 on a call.
export const SPEED = { min: 0.5, max: 3, step: 0.1 };
const clamp = (s: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, s));

export function voiceSpeed(): number {
  const s = Number(localStorage.getItem("voiceSpeed"));
  return s ? clamp(s, SPEED.min, SPEED.max) : 1;
}

// Picked here since this page loaded; the host keeps the voice itself.
let voicePicked: string | null = null;

/** The voice picked on this page, or null for the host's saved one. */
export function voiceChoice(): string | null {
  return voicePicked;
}

/** The voice picked on this page, and its setter, which saves it on the host. */
export function useVoiceChoice(): [string | null, (v: string) => void] {
  const [voice, setVoice] = useState(voiceChoice);
  const set = (v: string) => {
    voicePicked = v;
    setVoice(v);
    void api("api/voice", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ voice: v }),
    }).catch((e) => console.warn("voice:", e));
  };
  return [voice, set];
}

/** The voice speed and its setter. */
export function useVoiceSpeed(): [number, (s: number) => void] {
  const [speed, setSpeed] = useState(voiceSpeed);
  const set = (s: number) => {
    if (s === 1) localStorage.removeItem("voiceSpeed");
    else localStorage.setItem("voiceSpeed", String(s));
    setSpeed(s);
  };
  return [speed, set];
}

type State = "connecting" | "listening" | "thinking" | "speaking" | "ended" | { error: string };
type Line = { kind: "user" | "talk"; text: string };

// Hands the microphone's samples to the page in 20 ms frames.
const CAPTURE = `
class Capture extends AudioWorkletProcessor {
  constructor() { super(); this.buf = new Float32Array(${RATE / 50}); this.n = 0; }
  process([input]) {
    const ch = input[0];
    if (ch) for (let i = 0; i < ch.length; i++) {
      this.buf[this.n++] = ch[i];
      if (this.n === this.buf.length) { this.port.postMessage(this.buf.slice()); this.n = 0; }
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

function level(f: Float32Array): number {
  let sum = 0;
  for (const x of f) sum += x * x;
  return Math.min(1, Math.sqrt(sum / f.length) * 6);
}

/** How a call runs, kept on this device: "chat" (the default) carries the
 * person's words into the main chat and speaks its replies; "grok" puts
 * Grok Voice in front, quick to talk, handing real requests to the chat. */
export type CallMode = "chat" | "grok";
export function callMode(): CallMode {
  return localStorage.getItem("callMode") === "grok" ? "grok" : "chat";
}
export function setCallMode(m: CallMode) {
  if (m === "chat") localStorage.removeItem("callMode");
  else localStorage.setItem("callMode", m);
}

/** The whole-screen call with `name`; `ended` runs once it is over and saved. */
export function Call({ name, ended }: { name: string; ended: () => void }) {
  return callMode() === "grok" ? <GrokCall name={name} ended={ended} /> : <ChatCall name={name} ended={ended} />;
}

// The microphone for a call with the chat: 16 kHz mono, sent as 20 ms
// frames of 16-bit samples, which the host's ear (speech.rs) takes.
const EAR_RATE = 16000;
const EAR_CAPTURE = `
class Capture extends AudioWorkletProcessor {
  constructor() { super(); this.buf = new Float32Array(${EAR_RATE / 50}); this.n = 0; }
  process([input]) {
    const ch = input[0];
    if (ch) for (let i = 0; i < ch.length; i++) {
      this.buf[this.n++] = ch[i];
      if (this.n === this.buf.length) { this.port.postMessage(this.buf.slice()); this.n = 0; }
    }
    return true;
  }
}
registerProcessor("capture", Capture);
`;

/** Where the host's call socket is: the picked host's, or this page's. */
function callAddress() {
  const u = new URL(url("api/call/socket", true), location.href);
  u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
  return u.href;
}

/** A call with the chat itself: what the person says goes into the main
 * chat as their message, and the chat's reply is spoken back as it comes,
 * so the model, memory and tools on the call are the chat's own. The words
 * stay in the chat as it goes, so ending the call saves nothing more. */
function ChatCall({ name, ended }: { name: string; ended: () => void }) {
  const [state, setState] = useState<State>("connecting");
  const [muted, setMuted] = useState(false);
  const [caption, setCaption] = useState("");
  const [loud, setLoud] = useState(0);
  const hangUp = useRef<() => void>(() => {});
  const mute = useRef(false);

  useEffect(() => {
    let ws: WebSocket | undefined;
    let ear: AudioContext | undefined;
    let voice: AudioContext | undefined;
    let mic: MediaStream | undefined;
    let over = false;
    // The voice plays through an audio element, as on a Grok call, so a
    // phone's volume keys move it while the microphone is open.
    let out: MediaStreamAudioDestinationNode | undefined;
    const speaker = new Audio();
    // Sentences arrive as text, then their audio; each plays after the last.
    let next = 0;
    let pending: string | null = null;
    const playing = new Set<AudioBufferSourceNode>();
    const silence = () => {
      for (const s of playing) s.stop();
      playing.clear();
      next = 0;
    };
    const play = async (data: ArrayBuffer, text: string) => {
      if (!voice || !out) return;
      let buf: AudioBuffer;
      try {
        buf = await voice.decodeAudioData(data);
      } catch (e) {
        return console.warn("call voice:", e);
      }
      if (over) return;
      const src = voice.createBufferSource();
      src.buffer = buf;
      src.playbackRate.value = voiceSpeed();
      src.connect(out);
      next = Math.max(next, voice.currentTime + 0.03);
      const at = next;
      src.start(at);
      next += buf.duration / voiceSpeed();
      playing.add(src);
      window.setTimeout(() => !over && playing.has(src) && setCaption(text), Math.max(0, (at - voice.currentTime) * 1000));
      setState("speaking");
      src.onended = () => {
        playing.delete(src);
        if (!playing.size && !over) setState("listening");
      };
    };

    const finish = (why?: string, quiet = false) => {
      if (over) return;
      over = true;
      silence();
      ws?.close();
      mic?.getTracks().forEach((t) => t.stop());
      ear?.close();
      voice?.close();
      speaker.pause();
      speaker.srcObject = null;
      setState(why ? { error: why } : "ended");
      if (!why && !quiet) ended();
    };
    hangUp.current = () => finish();
    const closing = () => finish(undefined, true);
    window.addEventListener("pagehide", closing);

    (async () => {
      voice = new AudioContext();
      out = voice.createMediaStreamDestination();
      speaker.srcObject = out.stream;
      // Within the tap that opened the call, so autoplay allows it.
      speaker.play().catch((e) => console.warn("call speaker:", e));
      ear = new AudioContext({ sampleRate: EAR_RATE });
      mic = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
      if (over) return mic.getTracks().forEach((t) => t.stop());
      await ear.audioWorklet.addModule(URL.createObjectURL(new Blob([EAR_CAPTURE], { type: "text/javascript" })));
      const capture = new AudioWorkletNode(ear, "capture");
      ear.createMediaStreamSource(mic).connect(capture);
      ws = new WebSocket(callAddress());
      ws.binaryType = "arraybuffer";
      capture.port.onmessage = (e: MessageEvent<Float32Array>) => {
        if (mute.current) return setLoud(0);
        setLoud(level(e.data));
        if (ws?.readyState === WebSocket.OPEN) ws.send(pcm16(e.data));
      };
      ws.onmessage = (e) => {
        if (e.data instanceof ArrayBuffer) {
          const text = pending ?? "";
          pending = null;
          void play(e.data, text);
          return;
        }
        const ev = JSON.parse(e.data);
        switch (ev.type) {
          case "ready":
            setState("listening");
            break;
          case "speech":
            // The person talks over the voice: it stops, and their words
            // steer the chat as any message does.
            if (playing.size) {
              silence();
              ws?.send(JSON.stringify({ type: "barge" }));
            }
            setState("listening");
            break;
          case "partial":
            setCaption(ev.text);
            break;
          case "heard":
            setCaption(ev.text);
            setState("thinking");
            break;
          case "say":
            pending = ev.text;
            break;
          case "error":
            setCaption(ev.text);
            break;
        }
      };
      ws.onclose = (e) => {
        if (!over) finish(e.code === 1000 || e.code === 1005 ? undefined : `The call dropped (${e.code}${e.reason ? `: ${e.reason}` : ""}).`);
      };
    })().catch((e) => {
      const mics: Record<string, string> = {
        NotAllowedError: "The microphone is blocked for this site.",
        NotFoundError: "No microphone found.",
        NotReadableError: "Another app is using the microphone.",
      };
      finish((e instanceof DOMException && mics[e.name]) || (e instanceof Error ? e.message : String(e)));
    });

    return () => {
      window.removeEventListener("pagehide", closing);
      finish(undefined, true);
    };
    // One call per sheet.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const toggleMute = () => {
    mute.current = !mute.current;
    setMuted(mute.current);
  };
  return <CallScreen name={name} state={state} muted={muted} loud={loud} caption={caption} toggleMute={toggleMute} hangUp={() => hangUp.current()} ended={ended} />;
}

/** A call with Grok Voice in front, on a pass the host mints: it talks,
 * and hands anything that needs knowing or doing to the chat (`ask_chat`),
 * whose answer it says. */
function GrokCall({ name, ended }: { name: string; ended: () => void }) {
  const [state, setState] = useState<State>("connecting");
  const [muted, setMuted] = useState(false);
  const [caption, setCaption] = useState("");
  const [loud, setLoud] = useState(0);
  const hangUp = useRef<() => void>(() => {});
  const mute = useRef(false);

  useEffect(() => {
    let ws: WebSocket | undefined;
    let ctx: AudioContext | undefined;
    let mic: MediaStream | undefined;
    // Grok's voice plays through an audio element, not straight to the
    // speakers: with the microphone open Android is in a call, and its
    // volume keys move the call's volume, which only a media stream's
    // playback follows.
    let out: MediaStreamAudioDestinationNode | undefined;
    const speaker = new Audio();
    let over = false;
    const began = Date.now();
    // What was said, in the order it was said: a line is placed when its
    // turn starts, and its words fill in once heard.
    const lines = new Map<string, Line>();
    // The turns that went to the chat with ask_chat, left out of the saved
    // call since the chat already holds the request and its answer: the
    // person's words that asked, and Grok's responses around the handing
    // over (its "let me check" and its saying of the answer), by item and
    // by response. What Grok handled alone is kept.
    const asked = new Set<string>();
    const handed = new Set<string>();
    const responseOf = new Map<string, string>();
    let lastHeard: string | undefined;
    let answers = 0;
    const early: ArrayBuffer[] = [];
    // Grok's audio, queued back to back; cut off when the person speaks.
    let next = 0;
    const playing = new Set<AudioBufferSourceNode>();
    const silence = () => {
      for (const s of playing) s.stop();
      playing.clear();
      next = 0;
    };
    const play = (data: ArrayBuffer) => {
      if (!ctx || !out) return;
      const pcm = new Int16Array(data);
      const buf = ctx.createBuffer(1, pcm.length, RATE);
      const ch = buf.getChannelData(0);
      for (let i = 0; i < pcm.length; i++) ch[i] = pcm[i] / 0x8000;
      const src = ctx.createBufferSource();
      src.buffer = buf;
      src.connect(out);
      next = Math.max(next, ctx.currentTime + 0.05);
      src.start(next);
      next += buf.duration;
      playing.add(src);
      src.onended = () => {
        playing.delete(src);
        if (!playing.size && !over) setState("listening");
      };
      setState("speaking");
    };

    // Hanging up saves the call and hands back to the chat; leaving the
    // page, or the sheet going away, only saves it.
    const finish = (why?: string, quiet = false) => {
      if (over) return;
      over = true;
      silence();
      ws?.close();
      mic?.getTracks().forEach((t) => t.stop());
      ctx?.close();
      speaker.pause();
      speaker.srcObject = null;
      const said = [...lines]
        .filter(([id, l]) => {
          const response = responseOf.get(id);
          return l.text.trim() && !asked.has(id) && !(response && handed.has(response));
        })
        .map(([, l]) => l);
      setState(why ? { error: why } : "ended");
      const done = () => (why || quiet ? undefined : ended());
      if (!said.length) return done();
      api("api/call/end", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ lines: said, minutes: Math.max(1, Math.round((Date.now() - began) / 60000)) }),
        // A page closed mid-call still saves it.
        keepalive: true,
      }).finally(done);
    };
    hangUp.current = () => finish();
    const closing = () => finish(undefined, true);
    window.addEventListener("pagehide", closing);

    (async () => {
      // The microphone and the pass together, as xAI advises; the
      // microphone's first words wait for the line to open.
      ctx = new AudioContext({ sampleRate: RATE });
      out = ctx.createMediaStreamDestination();
      speaker.srcObject = out.stream;
      // Within the tap that opened the call, so autoplay allows it.
      speaker.play().catch((e) => console.warn("call speaker:", e));
      const [stream, pass] = await Promise.all([
        navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true } }),
        api("api/call", { method: "POST" }).then(async (r) => {
          if (!r.ok) throw new Error(r.status === 404 ? `${name} has no voice.` : `The call failed to start (${r.status}).`);
          return r.json() as Promise<{ secret: string; voice: string; instructions: string }>;
        }),
      ]);
      mic = stream;
      if (over) return stream.getTracks().forEach((t) => t.stop());
      await ctx.audioWorklet.addModule(URL.createObjectURL(new Blob([CAPTURE], { type: "text/javascript" })));
      const capture = new AudioWorkletNode(ctx, "capture");
      ctx.createMediaStreamSource(stream).connect(capture);
      capture.port.onmessage = (e: MessageEvent<Float32Array>) => {
        if (mute.current) return setLoud(0);
        setLoud(level(e.data));
        const frame = pcm16(e.data);
        if (ws?.readyState === WebSocket.OPEN) ws.send(frame);
        else if (early.length < 100) early.push(frame);
      };

      ws = new WebSocket(REALTIME_URL, [`xai-client-secret.${pass.secret}`]);
      ws.binaryType = "arraybuffer";
      ws.onopen = () => {
        const audio = { format: { type: "audio/pcm", rate: RATE }, transport: "binary" };
        ws!.send(
          JSON.stringify({
            type: "session.update",
            session: {
              voice: voiceChoice() ?? pass.voice,
              instructions: pass.instructions,
              turn_detection: { type: "server_vad" },
              // The chat does the real work; Grok talks and hands it over.
              tools: [
                {
                  type: "function",
                  name: "ask_chat",
                  description: "Hand a request to the chat, which has the memory and tools, and get its answer to say.",
                  parameters: { type: "object", properties: { request: { type: "string", description: "What the person wants, in their words." } }, required: ["request"] },
                },
              ],
              audio: { input: { ...audio, transcription: { model: "grok-transcribe" } }, output: { ...audio, speed: clamp(voiceSpeed(), 0.7, 1.5) } },
            },
          }),
        );
        for (const f of early.splice(0)) ws!.send(f);
        setState("listening");
      };
      ws.onmessage = (e) => {
        if (e.data instanceof ArrayBuffer) return play(e.data);
        const ev = JSON.parse(e.data);
        const line = (id: string, kind: Line["kind"]) => {
          if (!lines.has(id)) lines.set(id, { kind, text: "" });
          return lines.get(id)!;
        };
        switch (ev.type) {
          case "input_audio_buffer.speech_started":
            silence();
            setState("listening");
            break;
          case "input_audio_buffer.committed":
            lastHeard = ev.item_id;
            line(ev.item_id, "user");
            break;
          case "conversation.item.input_audio_transcription.completed":
            line(ev.item_id, "user").text = ev.transcript ?? "";
            break;
          case "response.created":
            // The response that says a handed-over answer.
            if (answers > 0) {
              answers--;
              if (ev.response?.id) handed.add(ev.response.id);
            }
            break;
          case "response.output_audio_transcript.delta": {
            responseOf.set(ev.item_id, ev.response_id);
            const l = line(ev.item_id, "talk");
            l.text += ev.delta ?? "";
            setCaption(l.text);
            break;
          }
          case "response.output_audio_transcript.done":
            if (ev.transcript) line(ev.item_id, "talk").text = ev.transcript;
            break;
          case "response.function_call_arguments.done": {
            if (ev.name !== "ask_chat") break;
            if (lastHeard) asked.add(lastHeard);
            if (ev.response_id) handed.add(ev.response_id);
            let request = "";
            try {
              request = JSON.parse(ev.arguments ?? "{}").request ?? "";
            } catch {
              request = "";
            }
            setCaption(request);
            void api("api/call/ask", {
              method: "POST",
              headers: { "content-type": "application/json" },
              body: JSON.stringify({ text: request }),
            })
              .then(async (r) => (r.ok ? ((await r.json()) as { text: string; still?: boolean }) : { text: "", still: false }))
              .catch(() => ({ text: "", still: false }))
              .then((a) => {
                if (over || ws?.readyState !== WebSocket.OPEN) return;
                const output = a.text || (a.still ? "The chat is still working on it; the answer will be in the chat." : "The chat couldn't answer that.");
                ws.send(JSON.stringify({ type: "conversation.item.create", item: { type: "function_call_output", call_id: ev.call_id, output } }));
                answers++;
                ws.send(JSON.stringify({ type: "response.create" }));
              });
            break;
          }
          case "error":
            console.warn("call:", ev.error ?? ev);
            break;
        }
      };
      ws.onclose = (e) => {
        if (!over) finish(e.code === 1000 ? undefined : `The call dropped (${e.code}${e.reason ? `: ${e.reason}` : ""}).`);
      };
    })().catch((e) => {
      const mics: Record<string, string> = {
        NotAllowedError: "The microphone is blocked for this site.",
        NotFoundError: "No microphone found.",
        NotReadableError: "Another app is using the microphone.",
      };
      finish((e instanceof DOMException && mics[e.name]) || (e instanceof Error ? e.message : String(e)));
    });

    return () => {
      window.removeEventListener("pagehide", closing);
      finish(undefined, true);
    };
    // One call per sheet.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const toggleMute = () => {
    mute.current = !mute.current;
    setMuted(mute.current);
  };
  return <CallScreen name={name} state={state} muted={muted} loud={loud} caption={caption} toggleMute={toggleMute} hangUp={() => hangUp.current()} ended={ended} />;
}

/** The call's whole screen: who, how it stands, the level, what is said. */
function CallScreen({
  name,
  state,
  muted,
  loud,
  caption,
  toggleMute,
  hangUp,
  ended,
}: {
  name: string;
  state: State;
  muted: boolean;
  loud: number;
  caption: string;
  toggleMute: () => void;
  hangUp: () => void;
  ended: () => void;
}) {
  const failed = typeof state === "object";
  const status = failed
    ? state.error
    : { connecting: "Connecting…", listening: muted ? "Muted" : "Listening", thinking: "Thinking…", speaking: "Speaking", ended: "Saving the call…" }[state];
  return (
    <div className="fixed inset-0 z-50 flex flex-col items-center bg-paper px-6 pt-[max(18vh,env(safe-area-inset-top))] pb-[max(40px,env(safe-area-inset-bottom))] text-ink">
      <h2 className="font-serif text-heading">{name}</h2>
      <p className={`mt-1.5 text-control leading-5 ${failed ? "text-accent" : "text-mid"}`}>{status}</p>
      <div className="mt-[9vh] flex h-24 items-center gap-2" aria-hidden>
        {[0.55, 0.85, 1, 0.85, 0.55].map((w, i) => (
          <span
            key={i}
            className={`w-2.5 rounded-full bg-ink transition-[height] duration-100 ${state === "speaking" ? "animate-[talk_0.9s_ease-in-out_infinite]" : ""}`}
            style={{ height: state === "speaking" ? undefined : `${12 + w * loud * 84}px`, animationDelay: `${i * 0.12}s` }}
          />
        ))}
      </div>
      <p className="mt-[6vh] line-clamp-4 max-w-[34ch] text-center text-prose leading-relaxed text-mid">{caption}</p>
      <div className="mt-auto flex items-center gap-10">
        {failed ? (
          <button type="button" onClick={ended} className="rounded-full border border-frame px-6 py-3 text-ink hover:bg-hover">
            Close
          </button>
        ) : (
          <>
            <button
              type="button"
              onClick={toggleMute}
              aria-label={muted ? "Unmute" : "Mute"}
              title={muted ? "Unmute" : "Mute"}
              className={`grid size-16 place-items-center rounded-full border border-frame ${muted ? "bg-ink text-paper" : "text-ink hover:bg-hover"}`}
            >
              <svg viewBox="0 0 24 24" className="size-6" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round">
                <rect x="9" y="3" width="6" height="11" rx="3" />
                <path d="M5 11a7 7 0 0 0 14 0M12 18v3" />
                {muted && <path d="M4 4l16 16" />}
              </svg>
            </button>
            <button
              type="button"
              onClick={hangUp}
              disabled={state === "ended"}
              aria-label="Hang up"
              title="Hang up"
              className="grid size-16 place-items-center rounded-full bg-[#c4453a] text-white hover:opacity-90 disabled:opacity-50"
            >
              <svg viewBox="0 0 24 24" className="size-7" fill="currentColor">
                <path d="M12 9c-2.4 0-4.6.5-6.6 1.4-.6.3-.9.9-.9 1.5v2.4c0 .7.6 1.2 1.3 1.1l2.6-.5c.6-.1 1-.6 1-1.2v-1.6c1.7-.5 3.5-.5 5.2 0v1.6c0 .6.4 1.1 1 1.2l2.6.5c.7.1 1.3-.4 1.3-1.1v-2.4c0-.6-.3-1.2-.9-1.5C16.6 9.5 14.4 9 12 9z" />
              </svg>
            </button>
          </>
        )}
      </div>
    </div>
  );
}

/** The voice button's mark: bars like a voice's level. */
export const Bars = () => (
  <svg viewBox="0 0 24 24" className="size-5" fill="none" stroke="currentColor" strokeWidth={2.2} strokeLinecap="round">
    <path d="M4.5 10v4M9.5 5v14M14.5 7.5v9M19.5 10v4" />
  </svg>
);
