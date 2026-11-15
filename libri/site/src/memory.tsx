import { useEffect, useRef, useState } from "react";
import { api } from "./api";

/** How the memory is working, as /api/memory (site.rs `memory_health`) tells
 * it. The host decides the state and the words; `unknown` is this page's own,
 * for a host that can't be reached. */
export type MemoryHealth = {
  state: "working" | "catching_up" | "paused" | "recording" | "failing" | "unknown";
  reason?: string | null;
  since?: string | null;
  waiting?: number | null;
  newest_summary?: string | null;
  model?: string | null;
  lines?: number | null;
  bytes?: number | null;
};

const SILENT = "The memory status isn't answering.";
const blank = { reason: null, since: null, waiting: null, newest_summary: null, lines: null, bytes: null };

export const MEMORY_WORD: Record<MemoryHealth["state"], string> = {
  working: "Working",
  catching_up: "Catching up",
  paused: "Paused",
  recording: "Recording",
  failing: "Failing",
  unknown: "Unknown",
};

// Made once: toLocaleString builds a formatter on every call, which on a
// long chat's first drawing cost a phone more than any other line of ours.
const TIME = new Intl.DateTimeFormat(undefined, { timeStyle: "short" });
const DATE_TIME = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

/** A moment as the app shows one: the time today, the date as well before. */
export function stamp(at: Date) {
  const today = at.toDateString() === new Date().toDateString();
  return (today ? TIME : DATE_TIME).format(at);
}

/** A size as a storage page shows one, counted in thousands as Places.kt
 * `sized` does. */
export function sized(bytes: number) {
  const unit = Math.min(Math.floor(Math.log10(Math.max(bytes, 1)) / 3), 4);
  const n = bytes / 1000 ** unit;
  const shown = n.toLocaleString(undefined, { maximumFractionDigits: unit && n < 100 ? 1 : 0 });
  return `${shown} ${["bytes", "KB", "MB", "GB", "TB"][unit]}`;
}

const asked = (path: string) => api(path, { cache: "no-store", signal: AbortSignal.timeout(5000) }).catch(() => null);

/** The one reading of the memory, held at the app's root so the line, the
 * panel's row and the Memory page can't disagree. It asks every 30 seconds
 * while the app is on screen and at once on coming back, and the host's own
 * clock for a silent memory counts from these reads. Null is nothing to show:
 * not asked yet, or an older host without the route. */
export function useMemory() {
  const [memory, setMemory] = useState<MemoryHealth | null>(null);
  // Host up but no status: when it began, and whether it has held a second
  // check, so one slow answer on a loaded host never raises the line.
  const silent = useRef<string>(undefined);
  useEffect(() => {
    const ask = async () => {
      if (document.visibilityState !== "visible") return;
      const r = await asked("api/memory");
      if (r?.status === 404) return setMemory(null);
      const read: MemoryHealth | null = r?.ok ? await r.json().catch(() => null) : null;
      if (read) {
        silent.current = undefined;
        return setMemory(read);
      }
      const host = r?.status === 401 ? null : await asked("api/host");
      if (!host?.ok) {
        silent.current = undefined;
        return setMemory({ state: "unknown", ...blank });
      }
      if (!silent.current) silent.current = new Date().toISOString();
      else setMemory({ ...blank, state: "failing", reason: SILENT, since: silent.current });
    };
    void ask();
    const every = setInterval(ask, 30_000);
    document.addEventListener("visibilitychange", ask);
    return () => {
      clearInterval(every);
      document.removeEventListener("visibilitychange", ask);
    };
  }, []);
  return memory;
}

/** The solid line above everything while the memory fails, the whole of it a
 * way to the detail. It has no close: it goes at the first status that isn't
 * failing. The live region stays mounted, or a screen reader would not hear
 * it fill, and keeps its first words so a later check says nothing new. */
export function MemoryLine({ memory, detail }: { memory: MemoryHealth | null; detail: () => void }) {
  const failing = memory?.state === "failing";
  const said = useRef<string>("");
  const words = failing ? `Memory failing${memory.since ? ` since ${stamp(new Date(memory.since))}` : ""}: ${memory.reason}` : "";
  said.current = failing ? said.current || words : "";
  return (
    <>
      <div role="status" className="sr-only">
        {said.current}
      </div>
      {failing && (
        <button type="button" onClick={detail} className="w-full shrink-0 truncate bg-accent px-4 pt-[env(safe-area-inset-top)] text-left text-label leading-9 font-medium text-on-accent hover:brightness-95 sm:text-center">
          {words}
        </button>
      )}
    </>
  );
}
