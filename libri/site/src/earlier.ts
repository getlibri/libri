import { type RefObject, useCallback, useEffect, useRef, useState } from "react";

/**
 * Draws a talk from its newest messages, and its earlier ones as the person
 * scrolls up to them, as Slack, Telegram and WhatsApp Web do.
 *
 * Drawn whole, a long history was one render of every message's markdown
 * and one layout of all of it: a Chat tile opened beside another over 120
 * messages took 330 to 530 ms at full speed on a desk, past the 400 ms the
 * ui-latency gate allows (0.9.79's cut). Its newest messages fill the view
 * many times over, so the rest are drawn a page at a time, each before the
 * person reaches it: once the view stands within its own height of the
 * talk's top. The message at the top stays where it stood on the screen as
 * a page lands above it; held here, not left to the browser's scroll
 * anchoring, which useFootHold turns off while the view is at the foot.
 *
 * A history read afresh with the first message drawn gone from it (another
 * talk, a reload that starts elsewhere) is drawn from its newest again.
 */
const TAIL = 40;
const PAGE = 40;

// Marks each message's row in the talk (ChatView); the first is kept in place.
const MESSAGE_ROW = "data-message";

export function useEarlier<M extends { id: string }>(messages: M[], viewport: RefObject<HTMLElement | null>) {
  const [from, setFrom] = useState<string | null>(() => tailFrom(messages));
  let start = from == null ? 0 : messages.findIndex((m) => m.id === from);
  if (start < 0) {
    const tail = tailFrom(messages);
    // Its own state set during its render, as React's derived state is.
    if (tail !== from) setFrom(tail);
    start = tail == null ? 0 : messages.length - TAIL;
  }
  const now = useRef({ messages, start });
  now.current = { messages, start };
  // The first row drawn and where it stood, while a page is on its way.
  const kept = useRef<{ row: Element | null; top: number } | null>(null);
  // What waits on the whole talk being drawn: a quote's jump.
  const after = useRef<(() => void) | null>(null);

  const earlier = useCallback(() => {
    const { messages, start } = now.current;
    const v = viewport.current;
    if (!v || start <= 0 || kept.current) return;
    const row = v.querySelector(`[${MESSAGE_ROW}]`);
    kept.current = { row, top: row?.getBoundingClientRect().top ?? 0 };
    const to = Math.max(0, start - PAGE);
    setFrom(to === 0 ? null : messages[to].id);
  }, [viewport]);

  /** Draws the whole talk, then runs `then`. */
  const whole = useCallback((then: () => void) => {
    if (now.current.start <= 0) return then();
    after.current = then;
    setFrom(null);
  }, []);

  // Called once the runtime has drawn the messages (the chat's Drawn).
  const drawn = useCallback(() => {
    const k = kept.current;
    const v = viewport.current;
    if (k) {
      kept.current = null;
      if (v && k.row?.isConnected) {
        const moved = k.row.getBoundingClientRect().top - k.top;
        if (Math.abs(moved) >= 1) v.scrollTop += moved;
      }
    }
    if (after.current && now.current.start <= 0) {
      const then = after.current;
      after.current = null;
      then();
    }
  }, [viewport]);

  useEffect(() => {
    const v = viewport.current;
    if (!v || start <= 0) return;
    const near = () => {
      if (v.scrollTop < v.clientHeight) earlier();
    };
    v.addEventListener("scroll", near, { passive: true });
    // A talk too short to scroll gives no scroll event: a wheel still asks.
    v.addEventListener("wheel", near, { passive: true });
    return () => {
      v.removeEventListener("scroll", near);
      v.removeEventListener("wheel", near);
    };
  }, [viewport, start, earlier]);

  return { start, drawn, whole };
}

function tailFrom(messages: { id: string }[]) {
  return messages.length > TAIL ? messages[messages.length - TAIL].id : null;
}
