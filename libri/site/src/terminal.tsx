// The terminal: the host's shell, drawn with ghostty-web (Ghostty's own
// terminal, built for the browser). The shell lives on the host and goes on
// while this is closed; opening it again is sent the screen as it is
// (src/terminal.rs). On a wide screen it is a column of its own beside the
// chat, or over the chat when the window has no room for both; on a phone it
// is an item on the bar (phone.tsx), with a row of the keys a phone keyboard
// lacks at its foot, as Paseo, Termux and Blink do, and while the keyboard is
// up it fills what the keyboard leaves.

import type { FontMetrics } from "ghostty-web";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { url } from "./api";
import { land } from "./carry";
import { useTileRows } from "./tileMenu";

type Ghostty = typeof import("ghostty-web");

// The library and its WASM, loaded the first time the terminal opens, not
// with the page.
let loaded: Promise<Ghostty> | undefined;
const ghostty = () =>
  (loaded ??= import("ghostty-web").then(async (g) => {
    await g.init();
    return g;
  }));

/** The page's colours, so the terminal sits in it rather than on it. */
function theme() {
  const css = getComputedStyle(document.documentElement);
  const v = (name: string) => css.getPropertyValue(name).trim();
  return {
    background: v("--paper"),
    foreground: v("--ink"),
    cursor: v("--accent"),
    selectionBackground: v("--hover"),
    selectionForeground: v("--ink"),
  };
}

/** Where the terminal's socket is: the picked host's, or this page's; `id`
 * names a shell of its own (a terminal tile opened beside another), none
 * the shared one the agents use too. */
function address(id?: string) {
  const u = new URL(url("api/terminal", true), location.href);
  if (id) u.searchParams.set("id", id);
  u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
  return u.href;
}

// The keys rows, laid out as Paseo's: Ctrl, Alt and Shift hold for the
// next key, typed or tapped; the arrows take them as xterm encodes them.
type Mods = { ctrl: boolean; alt: boolean; shift: boolean };
const NONE: Mods = { ctrl: false, alt: false, shift: false };
const ARROWS: Record<string, string> = { "↑": "A", "↓": "B", "→": "C", "←": "D" };
const ROWS = [
  ["Esc", "Tab", "ctrl", "↑", "shift", "hide"],
  ["alt", "Paste", "←", "↓", "→", "Enter"],
] as const;

/** What a key sends with the modifiers held. */
function encode(key: string, m: Mods) {
  const code = 1 + (m.shift ? 1 : 0) + (m.alt ? 2 : 0) + (m.ctrl ? 4 : 0);
  if (ARROWS[key]) return code > 1 ? `\x1b[1;${code}${ARROWS[key]}` : `\x1b[${ARROWS[key]}`;
  if (key === "Tab") return m.shift ? "\x1b[Z" : "\t";
  if (key === "Esc") return "\x1b";
  return "\r";
}

/** A typed character with the held modifiers: Shift uppercases, Ctrl makes
 * its control code, Alt puts Escape before it. */
function modify(data: string, m: Mods) {
  if (m.shift) data = data.toUpperCase();
  if (m.ctrl) {
    const c = data.toUpperCase().charCodeAt(0);
    if (c >= 64 && c <= 95) data = String.fromCharCode(c - 64);
    else if (data === " ") data = "\0";
  }
  return m.alt ? `\x1b${data}` : data;
}

/** What turning `was` into `now` sends the shell: a Backspace for each
 * character gone past their common start, then the new ones. */
function edit(was: string, now: string) {
  const a = [...was];
  const b = [...now];
  let same = 0;
  while (same < a.length && same < b.length && a[same] === b[same]) same++;
  return "\x7f".repeat(a.length - same) + b.slice(same).join("").replaceAll("\n", "\r");
}

/** Types into the shell what a phone keyboard does to the field. ghostty-web
 * cancels the field's beforeinput on its container and takes composition only
 * as it ends, so a keyboard that composes (Gboard in any browser; a page cannot
 * declare the field as the app does, Terminal.kt) gets no letters through, or
 * whole words at a space. Those events stop at the field here, the words stay
 * in it, and each change is sent as it happens, so the keyboard can go back
 * into a word or correct it and the shell's line follows. */
function readSoftKeyboard(field: HTMLTextAreaElement, typed: (data: string) => void) {
  let was = "";
  const own = (e: Event) => e.stopPropagation();
  for (const name of ["compositionstart", "compositionupdate", "compositionend"]) field.addEventListener(name, own);
  field.addEventListener("beforeinput", (e) => {
    e.stopPropagation();
    was = field.value;
    // Nothing in the field to change, and the shell's line may still have some.
    if (!was && e.inputType === "deleteContentBackward") typed("\x7f");
  });
  field.addEventListener("input", (e) => {
    const data = edit(was, field.value);
    if (data) typed(data);
    if (!(e as InputEvent).isComposing && field.value.includes("\n")) field.value = "";
  });
}

type State = "connecting" | "open" | "ended" | "lost";

export function Terminal({ id, close, phone, covering }: { id?: string; close: () => void; phone: boolean; covering?: boolean }) {
  const box = useRef<HTMLDivElement>(null);
  const send = useRef<(data: string) => void>(() => {});
  // Pasted through the terminal, so a program that asks for bracketed
  // paste gets it.
  const paste = useRef<(text: string) => void>(() => {});
  const [state, setState] = useState<State>("connecting");
  const [mods, setMods] = useState<Mods>(NONE);
  const held = useRef<Mods>(NONE);
  held.current = mods;
  // Bumped to start afresh: a new shell after the last one exited.
  const [run, setRun] = useState(0);
  // The terminal's field, once it is read for a phone keyboard.
  const field = useRef<HTMLTextAreaElement | null>(null);
  // A child's layout effect runs before its column's slide starts, so the ✕
  // is where the pill's button is bound.
  const shut = useRef<HTMLButtonElement>(null);
  useLayoutEffect(() => {
    if (!phone) land(shut.current!);
  }, [phone]);
  // On a phone the screen stops at the keyboard. Chrome shrinks the page for
  // it (interactive-widget in index.html); Safari and Firefox only shrink the
  // visual viewport, which this follows while the keyboard is up, so the
  // last rows stay above it.
  const [room, setRoom] = useState<{ top: number; height: number }>();
  const refit = useRef(() => {});
  useEffect(() => {
    const view = window.visualViewport;
    if (!phone || !view) return;
    let tallest = view.height;
    const follow = () => {
      tallest = Math.max(tallest, view.height);
      setRoom(view.height < tallest - 120 ? { top: view.offsetTop, height: view.height } : undefined);
      refit.current();
    };
    follow();
    view.addEventListener("resize", follow);
    view.addEventListener("scroll", follow);
    return () => {
      view.removeEventListener("resize", follow);
      view.removeEventListener("scroll", follow);
    };
  }, [phone]);

  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      // Escape belongs to the shell (vim needs it); this closes from the
      // button, or Escape with a modifier held.
      if (e.key === "Escape" && (e.metaKey || e.altKey)) close();
    };
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  }, [close]);

  // The tile menu's rows reach the shell through these, set while it runs.
  const actions = useRef<{ paste: () => void; clear: () => void; restart: () => void } | null>(null);
  // Restart shell: the host ends the shell; its clean close then opens anew.
  const restarting = useRef(false);
  useTileRows("terminal", {
    "paste": { disabled: state !== "open", run: () => actions.current?.paste() },
    "clear": { disabled: state !== "open", run: () => actions.current?.clear() },
    "restart": { run: () => (state === "open" ? actions.current?.restart() : setRun((r) => r + 1)) },
  });

  useEffect(() => {
    let done = false;
    let ws: WebSocket | undefined;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let term: InstanceType<Ghostty["Terminal"]> | undefined;
    let fit: InstanceType<Ghostty["FitAddon"]> | undefined;
    const bytes = new TextEncoder();
    actions.current = {
      paste: () => void navigator.clipboard?.readText().then((text) => ws?.readyState === WebSocket.OPEN && ws.send(bytes.encode(text))),
      // The page's screen and scrollback go, then Ctrl-L has the shell
      // redraw its prompt, as a terminal app's Clear does.
      clear: () => {
        term?.clear();
        if (ws?.readyState === WebSocket.OPEN) ws.send(bytes.encode("\x0c"));
      },
      restart: () => {
        restarting.current = true;
        if (ws?.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ op: "restart" }));
      },
    };
    void ghostty().then((g) => {
      if (done || !box.current) return;
      term = new g.Terminal({
        fontSize: phone ? 13 : 14,
        fontFamily: "ui-monospace, 'JetBrains Mono', Menlo, Consolas, monospace",
        cursorBlink: true,
        scrollback: 5000,
        theme: theme(),
      });
      fit = new g.FitAddon();
      term.loadAddon(fit);
      term.open(box.current);
      const t = term;
      const renderer = t.renderer;
      if (renderer) {
        // ghostty-web 0.4.0 sizes a row from M alone, so the next row's
        // background cuts off every tail. Leave room under the baseline for
        // g's descent, whatever the font and through a remeasure. A
        // line-height option, or descender measurement, in ghostty-web
        // removes this.
        const cell = renderer as unknown as { measureFont(): FontMetrics };
        const measure = cell.measureFont.bind(renderer);
        cell.measureFont = () => {
          const metrics = measure();
          const c = document.createElement("canvas").getContext("2d")!;
          c.font = `${t.options.fontSize}px ${t.options.fontFamily}`;
          const descent = Math.ceil(c.measureText("g").actualBoundingBoxDescent);
          return { ...metrics, height: metrics.baseline + descent };
        };
        renderer.remeasureFont();
        // ghostty-web 0.4.0 clears a strip across the last column to draw its
        // scrollbar. The fade's last frame repaints only changed rows, so the
        // strip stays over the text: repaint all of it there. A resize names
        // no opacity and draws at full with no bar to show: give it none. A
        // release that does both itself removes this.
        const render = renderer.render.bind(renderer);
        let bar = false;
        renderer.render = (buffer, forceAll, viewportY, provider, opacity = 0) => {
          const faded = bar && opacity === 0;
          bar = opacity > 0;
          render(buffer, forceAll || faded, viewportY, provider, opacity);
        };
      }
      fit.fit();
      fit.observeResize();
      const fitted = fit;
      // A keyboard taking room keeps the prompt in sight.
      refit.current = () => requestAnimationFrame(() => {
        fitted.fit();
        t.scrollToBottom();
      });
      paste.current = (text) => t.paste(text);
      send.current = (data) => {
        if (ws?.readyState === WebSocket.OPEN) ws.send(bytes.encode(data));
      };
      const typed = (data: string) => {
        // Modifiers from the keys row apply to the next character, as the
        // keyboard's own would.
        const m = held.current;
        if ((m.ctrl || m.alt || m.shift) && data.length === 1) {
          data = modify(data, m);
          setMods(NONE);
          // A control code is no letter for the keyboard to go back to.
          if (field.current) field.current.value = "";
        }
        send.current(data);
      };
      t.onData(typed);
      // A desktop's keys come through ghostty-web as keydown, and its input
      // methods compose with it; only a touch screen's keyboard is read here.
      field.current = matchMedia("(pointer: coarse)").matches && t.textarea ? t.textarea : null;
      if (field.current) readSoftKeyboard(field.current, typed);
      // The field that takes keys, pastes and input methods sits on the
      // cursor's cell, so an input method's window opens by the text being
      // typed; it is never seen (index.css [data-terminal-box] textarea).
      const atCursor = () => {
        const ta = t.textarea;
        const canvas = box.current?.querySelector("canvas");
        if (!ta || !canvas || !t.cols || !t.rows) return;
        const r = canvas.getBoundingClientRect();
        ta.style.position = "fixed";
        ta.style.left = `${r.left + (t.buffer.active.cursorX * r.width) / t.cols}px`;
        ta.style.top = `${r.top + (t.buffer.active.cursorY * r.height) / t.rows}px`;
      };
      t.onCursorMove(atCursor);
      t.onRender(atCursor);
      t.onResize(({ cols, rows }) => {
        if (ws?.readyState === WebSocket.OPEN) ws.send(JSON.stringify({ cols, rows }));
      });
      const connect = () => {
        setState("connecting");
        const s = new WebSocket(address(id));
        ws = s;
        s.binaryType = "arraybuffer";
        let first = true;
        s.onopen = () => s.send(JSON.stringify({ cols: t.cols, rows: t.rows }));
        s.onmessage = (e) => {
          // The first frame is the whole screen: draw it on a cleared
          // terminal, so a reconnect does not stack screens. Cleared as the
          // host clears it, not with t.reset(): that rebuilds the WASM
          // terminal on freed memory, which the next widening shows as junk
          // cells (ghostty-web 0.4.0).
          if (first) {
            t.write("\x1bc");
            first = false;
            setState("open");
          }
          t.write(new Uint8Array(e.data as ArrayBuffer));
        };
        s.onclose = (e) => {
          if (done || ws !== s) return;
          // The host closes cleanly when the shell exits; anything else
          // is the connection, tried again.
          if (e.code === 1000) {
            if (!restarting.current) return setState("ended");
            restarting.current = false;
            setRun((r) => r + 1);
          } else {
            setState("lost");
            retry = setTimeout(connect, 1500);
          }
        };
      };
      connect();
      // A phone's keyboard waits for a tap on the terminal: raised on
      // opening, it would cover half the screen of a terminal opened only to
      // read it. A desktop takes the keys at once.
      if (!phone) t.focus();
    });
    return () => {
      done = true;
      actions.current = null;
      refit.current = () => {};
      clearTimeout(retry);
      ws?.close();
      fit?.dispose();
      term?.dispose();
    };
  }, [phone, run]);

  return (
    <section
      data-surface="terminal"
      role="region"
      aria-label="Terminal"
      style={phone ? room : undefined}
      className={`flex flex-col overflow-hidden bg-paper ${
        phone
          ? room
            ? "fixed inset-x-0 z-50"
            : "relative h-full"
          : `h-full shrink-0 ${covering ? "w-[100cqw]" : "w-(--tools-width)"}`
      }`}
    >
      {/* On a phone no head and no ✕: the bar names
          the terminal and leaves it; the shell's state floats at the top
          right only while it is not plainly running. */}
      {phone ? (
        state !== "open" && (
          <div className="pointer-events-none absolute inset-x-0 top-[calc(var(--tile-top,0px)+10px)] z-20 flex justify-end px-3.5">
            {state === "ended" ? (
              <button type="button" onClick={() => setRun((r) => r + 1)} className="floating pointer-events-auto h-11 rounded-full px-4 text-label text-ink">
                Shell exited · New shell
              </button>
            ) : (
              <span role="status" className="floating flex h-11 items-center rounded-full px-4 text-label text-mid">
                {state === "connecting" ? "Connecting…" : "Reconnecting…"}
              </span>
            )}
          </div>
        )
      ) : (
      <header className="flex h-[57px] shrink-0 items-center gap-2 pr-1.5 pl-4">
        <h2 className="text-body font-semibold">Terminal</h2>
        <span className="flex-1 text-label text-soft">
          {state === "connecting" ? "connecting…" : state === "lost" ? "reconnecting…" : state === "ended" ? "shell exited" : ""}
        </span>
        {state === "ended" && (
          <button type="button" onClick={() => setRun((r) => r + 1)} className="h-8 rounded-full px-3 text-label text-mid hover:bg-hover hover:text-ink">
            New shell
          </button>
        )}
        <button
          ref={shut}
          type="button"
          onClick={close}
          aria-label="Close the terminal"
          title="Close (the shell keeps running)"
          className="grid size-9 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink"
        >
          <svg viewBox="0 0 24 24" className="size-[18px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round">
            <path d="M18 6 6 18M6 6l12 12" />
          </svg>
        </button>
      </header>
      )}
      <div
        ref={box}
        data-terminal-box
        // A tap on the strip past the last row misses the canvas, and focus
        // falling to the page there drops the keyboard.
        onMouseDown={(e) => {
          if (!field.current || (e.target as Element).closest("canvas, textarea")) return;
          e.preventDefault();
          field.current.focus();
        }}
        className={`min-h-0 flex-1 overflow-hidden px-2 py-1.5 ${phone ? "pt-[calc(var(--tile-top,0px)+6px)]" : ""}`}
      />
      {phone && (
        <div className="grid shrink-0 grid-cols-6 gap-1.5 px-2 py-2">
          {ROWS.flat().map((k) =>
            k === "ctrl" || k === "alt" || k === "shift" ? (
              <Key key={k} on={mods[k]} onClick={() => setMods((m) => ({ ...m, [k]: !m[k] }))}>
                {k[0].toUpperCase() + k.slice(1)}
              </Key>
            ) : k === "hide" ? (
              // Puts the phone's keyboard away; a tap on the screen brings it back.
              <Key key={k} label="Hide the keyboard" onClick={() => (document.activeElement as HTMLElement | null)?.blur()}>
                <svg viewBox="0 0 24 24" className="mx-auto size-[18px]" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
                  <rect x="3" y="6" width="18" height="12" rx="1.5" />
                  <path d="M7 10h.01M11 10h.01M15 10h.01M8 14h8M3 3l18 18" />
                </svg>
              </Key>
            ) : k === "Paste" ? (
              <Key key={k} onClick={() => void navigator.clipboard?.readText().then((t) => t && paste.current(t), () => {})}>
                Paste
              </Key>
            ) : (
              <Key
                key={k}
                onClick={() => {
                  send.current(encode(k, held.current));
                  setMods(NONE);
                  // The line has moved on from what the field holds.
                  if (field.current) field.current.value = "";
                }}
              >
                {k}
              </Key>
            ),
          )}
        </div>
      )}
    </section>
  );
}

function Key({ on, onClick, label, children }: { on?: boolean; onClick: () => void; label?: string; children: React.ReactNode }) {
  return (
    <button
      type="button"
      aria-label={label}
      aria-pressed={on}
      // Kept from taking focus, so the keyboard stays up.
      onPointerDown={(e) => e.preventDefault()}
      onClick={onClick}
      className={`h-10 rounded-lg border border-hair text-body ${on ? "bg-ink text-paper" : "bg-block text-mid active:bg-hover"}`}
    >
      {children}
    </button>
  );
}
