// Pinned: messages kept to find again without scrolling the one long chat,
// as WhatsApp stars them. A pin keeps the message's words, so it outlives
// the page; from the Pinned page it comes back into the chat, or opens with
// the talk around it. Pins and the switch that hides the place are kept per
// device for now; moving them to the host makes them follow the person.
import { useState, useSyncExternalStore } from "react";
import { Streamdown } from "streamdown";
import type { Msg } from "./chat";

export type Pin = { id: string; role: Msg["role"]; text: string; date?: number; at: number };

const PINS = "libri-pins";
const PINS_ON = "libri-pins-on";
const CHANGED = "libri-pins-changed";

export const listen = (then: () => void) => {
  window.addEventListener(CHANGED, then);
  window.addEventListener("storage", then);
  return () => {
    window.removeEventListener(CHANGED, then);
    window.removeEventListener("storage", then);
  };
};
export const write = (key: string, value: string) => {
  localStorage.setItem(key, value);
  window.dispatchEvent(new Event(CHANGED));
};

/** Whether the Pinned place is on; on unless turned off in settings. */
export function usePinsOn(): [boolean, (on: boolean) => void] {
  const on = useSyncExternalStore(listen, () => localStorage.getItem(PINS_ON) !== "off");
  return [on, (next) => write(PINS_ON, next ? "on" : "off")];
}

// The stored string is the snapshot, so React sees a change only when it changes.
const parse = (raw: string): Pin[] => {
  try {
    return JSON.parse(raw);
  } catch {
    return [];
  }
};
export function usePins() {
  const raw = useSyncExternalStore(listen, () => localStorage.getItem(PINS) ?? "[]");
  const pins = parse(raw);
  const save = (next: Pin[]) => write(PINS, JSON.stringify(next));
  return {
    pins,
    pinned: (id: string) => pins.some((p) => p.id === id),
    toggle: (m: Omit<Pin, "at">) =>
      save(pins.some((p) => p.id === m.id) ? pins.filter((p) => p.id !== m.id) : [{ ...m, at: Date.now() }, ...pins]),
    drop: (id: string) => save(pins.filter((p) => p.id !== id)),
  };
}

export function PinIcon({ filled }: { filled?: boolean }) {
  return (
    <svg viewBox="0 0 24 24" className="size-[15px]" fill={filled ? "currentColor" : "none"} stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 17v5" />
      <path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z" />
    </svg>
  );
}

const when = (ms?: number) =>
  ms === undefined
    ? ""
    : new Date(ms).toLocaleString([], { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });

const WHO: Partial<Record<Msg["role"], string>> = { user: "You" };

/** The Pinned page: newest pin first; each comes back into the chat, opens
 * with the talk around it as the page holds it, or is unpinned. */
export function PinnedList({ messages, name, resume }: { messages: Msg[]; name: string; resume: (pin: Pin) => void }) {
  const { pins, drop } = usePins();
  const [open, setOpen] = useState<string>();
  const small = "h-8 rounded-full px-3 text-label text-mid hover:bg-hover hover:text-ink";
  if (!pins.length)
    return (
      <p className="mx-auto mt-10 max-w-[18rem] text-center text-control text-soft">
        Nothing pinned. Tap the pin under a message to keep it here.
      </p>
    );
  return (
    <div className="mx-auto flex w-[min(var(--container-chat),100%-28px)] flex-col gap-2 py-3">
      {pins.map((p) => {
        // Matched by id, or by its words once a reload has renumbered it.
        const at = messages.findIndex((m) => m.id === p.id || (m.role === p.role && m.text === p.text));
        const around = at < 0 ? [] : messages.slice(Math.max(0, at - 3), at + 4).filter((m) => m.role !== "system" && m.text.trim());
        return (
          <article key={p.id} className="rounded-2xl border border-hair bg-lift/60 px-4 py-3">
            <div className="flex gap-2 text-caption leading-4 text-soft">
              <span className="font-semibold text-mid">{WHO[p.role] ?? name}</span>
              <span>{when(p.date)}</span>
            </div>
            <div className="mt-1 text-control [overflow-wrap:anywhere] [&_p]:my-1">
              <Streamdown>{p.text}</Streamdown>
            </div>
            <div className="-mx-2 mt-1 flex flex-wrap gap-1">
              <button type="button" onClick={() => resume(p)} className={small}>
                Resume in chat
              </button>
              <button type="button" onClick={() => setOpen(open === p.id ? undefined : p.id)} className={small}>
                {open === p.id ? "Hide context" : "Context"}
              </button>
              <button type="button" onClick={() => drop(p.id)} className={small}>
                Unpin
              </button>
            </div>
            {open === p.id && (
              <div className="mt-2 flex flex-col gap-2 border-t border-hair pt-2">
                {!around.length && <p className="text-label text-soft">Not among the messages this page has loaded.</p>}
                {around.map((m) => (
                  <div key={m.id} className={m.id === messages[at]?.id ? "" : "opacity-70"}>
                    <div className="text-caption leading-4 font-semibold text-mid">{WHO[m.role] ?? name}</div>
                    <div className="line-clamp-6 text-label whitespace-pre-wrap [overflow-wrap:anywhere]">{m.text.replace(/\*\*|`/g, "")}</div>
                  </div>
                ))}
              </div>
            )}
          </article>
        );
      })}
    </div>
  );
}

/** What resuming a pin sends: the pinned words, for the chat to pick up. */
export const resumed = (p: Pin, name: string) =>
  `Picking up from a pinned message (${WHO[p.role] ?? name}, ${when(p.date)}):\n\n${p.text
    .split("\n")
    .map((l) => `> ${l}`)
    .join("\n")}`;
