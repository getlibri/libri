// The Browser tile: Libri's own browser on the host, the one agents browse
// with, shown live and driven by the person (browser_live.rs). The host lays
// the page out at this tile's size and sends a JPEG frame whenever it
// changes; the tile draws the newest, and sends the person's mouse, wheel,
// touch, keys and text back as DevTools input. Tabs, address and history are
// the host's, so every screen that opens the tile shows the same browser.

import { useCallback, useEffect, useRef, useState } from "react";
import { api, url } from "./api";
import { Strip, StripAdd, StripTab } from "./tabStrip";

type Tab = { id: string; url: string; title: string; logins: boolean };
type Status = {
  running: boolean;
  holder: "agent" | "person";
  waiting: string | null;
  granted: string | null;
  url: string;
  title: string;
  yours: boolean;
  tabs: Tab[];
  shown: string;
};
type Note = { clip?: string; choose?: { node: number; multiple: boolean }; download?: string; error?: string };

function address() {
  const u = new URL(url("api/browser/live", true), location.href);
  u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
  return u.href;
}

/** DevTools' modifier bits: Alt 1, Ctrl 2, Meta 4, Shift 8. */
const mods = (e: { altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }) =>
  (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
const BUTTONS = ["left", "middle", "right"] as const;
/** How often a moving mouse is told, at most. */
const MOVE_MS = 30;

export function BrowserTile({ phone }: { phone: boolean }) {
  const [status, setStatus] = useState<Status>();
  const [state, setState] = useState<"connecting" | "open" | "lost">("connecting");
  const [toast, setToast] = useState<string>();
  const [typed, setTyped] = useState<string>();
  // A page's file input that asked for files, while the person hasn't
  // picked: the person's own tap on Choose opens their device's picker (a
  // picker opens only from a tap, and the ask may be an agent's).
  const [asking, setAsking] = useState<{ node: number; multiple: boolean }>();
  const ws = useRef<WebSocket | undefined>(undefined);
  const canvas = useRef<HTMLCanvasElement>(null);
  const box = useRef<HTMLDivElement>(null);
  const keys = useRef<HTMLInputElement>(null);
  const page = useRef({ width: 1, height: 1 });
  const send = useCallback((m: object) => {
    if (ws.current?.readyState === WebSocket.OPEN) ws.current.send(JSON.stringify(m));
  }, []);
  const say = useCallback((t: string) => {
    setToast(t);
    setTimeout(() => setToast((x) => (x === t ? undefined : x)), 4000);
  }, []);

  // The newest frame wins: one decodes at a time, and a frame that came
  // while one decoded replaces the waiting one.
  const decoding = useRef(false);
  const next = useRef<ArrayBuffer | undefined>(undefined);
  const draw = useCallback(async (buf: ArrayBuffer) => {
    if (decoding.current) {
      next.current = buf;
      return;
    }
    decoding.current = true;
    try {
      // The page's width and height, and the picture's width, in CSS
      // pixels (browser_live.rs), then the picture.
      const head = new DataView(buf, 0, 12);
      const width = head.getUint32(0, true) || 1;
      const height = head.getUint32(4, true) || 1;
      page.current = { width, height };
      const img = await createImageBitmap(new Blob([buf.slice(12)], { type: "image/jpeg" }));
      // A picture wider than the page (Chromium's window is 500 points
      // wide at least) holds the page at its top left: only that is drawn.
      const k = img.width / (head.getUint32(8, true) || width);
      const w = Math.min(img.width, Math.round(width * k));
      const h = Math.min(img.height, Math.round(height * k));
      const c = canvas.current;
      if (c) {
        if (c.width !== w || c.height !== h) {
          c.width = w;
          c.height = h;
        }
        c.getContext("2d")?.drawImage(img, 0, 0, w, h, 0, 0, w, h);
      }
      img.close();
    } catch {
      // A frame cut short by a closing socket: the next one draws.
    }
    decoding.current = false;
    const n = next.current;
    next.current = undefined;
    if (n) void draw(n);
  }, []);

  const choose = useCallback(
    (node: number, multiple: boolean) => {
      const input = document.createElement("input");
      input.type = "file";
      input.multiple = multiple;
      input.onchange = async () => {
        const form = new FormData();
        form.append("node", String(node));
        for (const f of Array.from(input.files ?? [])) form.append("file", f, f.name);
        const r = await api("api/browser/choose", { method: "POST", body: form }).catch(() => undefined);
        if (!r?.ok) say("The file didn't reach the page.");
      };
      input.click();
    },
    [say],
  );

  useEffect(() => {
    let gone = false;
    let retry: ReturnType<typeof setTimeout>;
    const connect = () => {
      setState("connecting");
      const s = new WebSocket(address());
      s.binaryType = "arraybuffer";
      ws.current = s;
      s.onopen = () => {
        setState("open");
        fit();
      };
      s.onmessage = (e) => {
        if (typeof e.data !== "string") return void draw(e.data as ArrayBuffer);
        const m = JSON.parse(e.data) as { status?: Status; note?: Note };
        if (m.status) setStatus(m.status);
        const n = m.note;
        if (n?.clip) void navigator.clipboard?.writeText(n.clip).catch(() => say("Copying needs the page in focus."));
        if (n?.choose) setAsking(n.choose);
        if (n?.download) say(`Saved to your files: ${n.download}`);
        if (n?.error) say(n.error);
      };
      s.onclose = () => {
        if (gone) return;
        setState("lost");
        retry = setTimeout(connect, 1500);
      };
    };
    // The page is laid out at this tile's size, in CSS pixels, while the
    // tile is in sight: a hidden one leaves the size to the one shown.
    const fit = () => {
      if (document.visibilityState !== "visible") return;
      const b = box.current?.getBoundingClientRect();
      if (!b || b.width < 50 || b.height < 50) return;
      // Laid out as a phone lays out a page on a touch screen alone.
      send({ kind: "view", width: Math.round(b.width), height: Math.round(b.height), scale: devicePixelRatio || 1, mobile: phone && matchMedia("(pointer: coarse)").matches });
    };
    const watch = new ResizeObserver(() => fit());
    if (box.current) watch.observe(box.current);
    const seen = () => (document.visibilityState === "visible" ? fit() : send({ kind: "away" }));
    document.addEventListener("visibilitychange", seen);
    connect();
    return () => {
      gone = true;
      clearTimeout(retry);
      watch.disconnect();
      document.removeEventListener("visibilitychange", seen);
      ws.current?.close();
    };
  }, [draw, send, choose, say, phone]);

  // Where a point on the tile is on the page, in the page's CSS pixels.
  // The picture is drawn whole at its own shape from the top left
  // (object-contain): a picture of another size, until the page is laid out
  // again for this one, leaves paper beside it rather than stretching.
  const at = (e: { clientX: number; clientY: number }) => {
    const c = canvas.current!;
    const r = c.getBoundingClientRect();
    const k = Math.min(r.width / (c.width || 1), r.height / (c.height || 1));
    return {
      x: ((e.clientX - r.left) / (c.width * k || 1)) * page.current.width,
      y: ((e.clientY - r.top) / (c.height * k || 1)) * page.current.height,
    };
  };
  const moved = useRef(0);
  const touch = useRef<{ x: number; y: number; moved: boolean } | undefined>(undefined);
  const onPointer = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const p = at(e);
    if (e.pointerType === "touch") {
      // A finger scrolls by dragging and clicks by tapping.
      if (e.type === "pointerdown") touch.current = { x: e.clientX, y: e.clientY, moved: false };
      else if (e.type === "pointermove" && touch.current) {
        const dy = touch.current.y - e.clientY;
        const dx = touch.current.x - e.clientX;
        if (Math.abs(dy) + Math.abs(dx) > 4) {
          touch.current = { x: e.clientX, y: e.clientY, moved: true };
          send({ kind: "wheel", ...p, dx: dx * 1.5, dy: dy * 1.5 });
        }
      } else if (e.type === "pointerup" && touch.current) {
        if (!touch.current.moved) {
          send({ kind: "mouse", type: "mousePressed", ...p, button: "left", buttons: 1, clicks: 1 });
          send({ kind: "mouse", type: "mouseReleased", ...p, button: "left", buttons: 0, clicks: 1 });
          keys.current?.focus({ preventScroll: true });
        }
        touch.current = undefined;
      }
      return;
    }
    const button = BUTTONS[e.button] ?? "none";
    if (e.type === "pointermove") {
      const now = performance.now();
      if (now - moved.current < MOVE_MS) return;
      moved.current = now;
      send({ kind: "mouse", type: "mouseMoved", ...p, button: "none", buttons: e.buttons, modifiers: mods(e) });
    } else if (e.type === "pointerdown") {
      canvas.current?.focus({ preventScroll: true });
      canvas.current?.setPointerCapture(e.pointerId);
      send({ kind: "mouse", type: "mousePressed", ...p, button, buttons: e.buttons, clicks: Math.max(1, e.detail), modifiers: mods(e) });
    } else if (e.type === "pointerup") {
      send({ kind: "mouse", type: "mouseReleased", ...p, button, buttons: e.buttons, clicks: Math.max(1, e.detail), modifiers: mods(e) });
    }
  };
  // The wheel scrolls the page, never the tile, and a pinch's ctrl+wheel
  // zooms nothing here.
  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const k = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? 800 : 1;
      send({ kind: "wheel", ...at(e), dx: e.deltaX * k, dy: e.deltaY * k, modifiers: mods(e) });
    };
    c.addEventListener("wheel", wheel, { passive: false });
    return () => c.removeEventListener("wheel", wheel);
  }, [send]);

  // Keys go to the page as keys; a printable one carries its text. Copy and
  // paste cross between the person's clipboard and the page.
  const onKey = (e: React.KeyboardEvent) => {
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === "v") return; // the paste event carries the text
    e.preventDefault();
    const down = e.type === "keydown";
    const printable = e.key.length === 1 && !e.ctrlKey && !e.metaKey;
    send({
      kind: "key",
      type: down ? (printable ? "keyDown" : "rawKeyDown") : "keyUp",
      key: e.key,
      code: e.code,
      text: down && printable ? e.key : e.key === "Enter" && down ? "\r" : undefined,
      key_code: e.keyCode,
      modifiers: mods(e),
    });
    if (down && mod && e.key.toLowerCase() === "c") setTimeout(() => send({ kind: "copy" }), 60);
  };
  const onPaste = (e: React.ClipboardEvent) => {
    const text = e.clipboardData.getData("text/plain");
    if (text) {
      e.preventDefault();
      send({ kind: "text", text });
    }
  };
  // A phone's keyboard types through a hidden field: its words go as text,
  // its backspace and enter as keys.
  const onBeforeInput = (e: React.FormEvent<HTMLInputElement>) => {
    const ev = e.nativeEvent as InputEvent;
    e.preventDefault();
    if (ev.inputType === "insertText" && ev.data) send({ kind: "text", text: ev.data });
    else if (ev.inputType === "insertLineBreak" || ev.inputType === "insertParagraph") {
      send({ kind: "key", type: "keyDown", key: "Enter", code: "Enter", text: "\r", key_code: 13 });
      send({ kind: "key", type: "keyUp", key: "Enter", code: "Enter", key_code: 13 });
    } else if (ev.inputType === "deleteContentBackward") {
      send({ kind: "key", type: "rawKeyDown", key: "Backspace", code: "Backspace", key_code: 8 });
      send({ kind: "key", type: "keyUp", key: "Backspace", code: "Backspace", key_code: 8 });
    }
  };

  const shown = status?.tabs.find((t) => t.id === status.shown);
  const agentTab = shown && !shown.logins;
  const go = (words: string) => {
    send({ kind: "go", url: words });
    setTyped(undefined);
    canvas.current?.focus({ preventScroll: true });
  };
  const mine = status?.holder === "person";

  return (
    <div className="flex h-full min-h-0 flex-col bg-paper" data-surface="browser" data-own-zoom data-own-swipe>
      <div className={`flex shrink-0 px-2 ${phone ? "pt-[var(--tile-top,0px)] pb-1" : "pt-1.5 pb-1"}`}>
        <Strip scroll role="tablist" aria-label="Tabs">
          {status?.tabs.map((t) => {
            const name = t.title || t.url.replace(/^https?:\/\//, "") || "New tab";
            return (
              <StripTab key={t.id} on={t.id === status.shown} title={t.url} onClick={() => send({ kind: "tab_show", id: t.id })} close={{ label: `Close tab ${name}`, run: () => send({ kind: "tab_close", id: t.id }) }}>
                {name}
              </StripTab>
            );
          })}
          <StripAdd label="New tab" onClick={() => send({ kind: "tab_new" })} />
        </Strip>
      </div>
      <form
        className="flex shrink-0 items-center gap-1 px-2 pb-1.5"
        onSubmit={(e) => {
          e.preventDefault();
          go(typed ?? status?.url ?? "");
        }}
      >
        <button type="button" aria-label="Back" className="flex size-8 items-center justify-center rounded-full text-mid hover:bg-hover hover:text-ink" onClick={() => send({ kind: "back" })}>
          ‹
        </button>
        <button type="button" aria-label="Forward" className="flex size-8 items-center justify-center rounded-full text-mid hover:bg-hover hover:text-ink" onClick={() => send({ kind: "forward" })}>
          ›
        </button>
        <button type="button" aria-label="Reload" className="flex size-8 items-center justify-center rounded-full text-mid hover:bg-hover hover:text-ink" onClick={() => send({ kind: "reload" })}>
          ↻
        </button>
        <input
          aria-label="Address"
          value={typed ?? (status?.url === "about:blank" ? "" : status?.url ?? "")}
          placeholder="Search or type an address"
          onChange={(e) => setTyped(e.target.value)}
          onFocus={(e) => e.currentTarget.select()}
          onBlur={() => setTyped(undefined)}
          className="h-8 min-w-0 flex-1 rounded-full bg-hover px-3 text-label text-ink outline-none"
        />
        {status?.running && (
          <span className={`shrink-0 px-2 text-meta ${mine ? "text-ink" : "text-mid"}`} title={agentTab ? "An agent's tab: not signed in, cleared when the browser stops." : undefined}>
            {mine ? "You" : "Agent"}
            {agentTab ? " · private" : ""}
          </span>
        )}
        {mine && (
          <button type="button" className="h-8 shrink-0 rounded-full px-3 text-label text-mid hover:bg-hover hover:text-ink" onClick={() => send({ kind: "give" })}>
            Hand back
          </button>
        )}
      </form>
      {asking && (
        <p className="flex shrink-0 items-center gap-2 px-3 pb-1.5 text-label text-ink">
          The page asks for {asking.multiple ? "files" : "a file"}.
          <button
            className="rounded-full bg-hover px-3 py-1"
            onClick={() => {
              choose(asking.node, asking.multiple);
              setAsking(undefined);
            }}
          >
            Choose…
          </button>
          <button className="text-mid" onClick={() => setAsking(undefined)}>
            Not now
          </button>
        </p>
      )}
      {status?.waiting && <p className="shrink-0 px-3 pb-1.5 text-label text-accent">An agent waits on you: {status.waiting}</p>}
      <div ref={box} className="relative min-h-0 flex-1 overflow-hidden">
        <canvas
          ref={canvas}
          tabIndex={0}
          aria-label="The page"
          className="absolute inset-0 size-full touch-none object-contain object-left-top outline-none"
          onPointerDown={onPointer}
          onPointerMove={onPointer}
          onPointerUp={onPointer}
          onContextMenu={(e) => e.preventDefault()}
          onKeyDown={onKey}
          onKeyUp={onKey}
          onPaste={onPaste}
        />
        {phone && (
          <input
            ref={keys}
            aria-label="Type into the page"
            autoCapitalize="off"
            autoComplete="off"
            autoCorrect="off"
            spellCheck={false}
            className="absolute bottom-0 left-0 size-px opacity-0"
            onBeforeInput={onBeforeInput}
            onInput={(e) => {
              // A keyboard that sent no beforeinput leaves its words here.
              const v = e.currentTarget.value;
              if (v) send({ kind: "text", text: v });
              e.currentTarget.value = "";
            }}
          />
        )}
        {(!status?.running || state !== "open") && (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 text-label text-mid">
            {state === "open" ? (
              <>
                <p>Libri's browser, shared with your agents.</p>
                <button className="rounded-full bg-hover px-4 py-1.5 text-ink" onClick={() => send({ kind: "tab_new" })}>
                  Start browsing
                </button>
              </>
            ) : (
              <p>{state === "lost" ? "Reconnecting…" : "Connecting…"}</p>
            )}
          </div>
        )}
        {toast && <p className="absolute bottom-3 left-1/2 -translate-x-1/2 rounded-full bg-ink px-3 py-1.5 text-label text-paper">{toast}</p>}
      </div>
    </div>
  );
}
