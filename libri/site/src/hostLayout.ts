import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "./api";

// An arrangement kept on the host: the wide page's tiles (tiles.tsx,
// api/layout) and a phone's workspaces (phone.tsx, api/phone-layout), two
// stores apart.
/** The layout is the host's (api/layout), so every browser signed in to it
 * opens the same arrangement, and the agent can change it too. This
 * browser's copy draws the page at once while the host is asked; nothing is
 * written back until the host has answered, so a stale copy never overwrites
 * the host's. Each kept change has a version, and the page waits on the next
 * one, so a change from the agent or another screen shows here at once. */
type Kept = { version: number; layout: unknown; by?: string; undo: boolean };
export function useHostLayout<Saved>(path: string, cache: string, readable: (v: unknown) => Saved | undefined, start: () => Saved) {
  const [saved, setSaved] = useState<Saved>(() => {
    try {
      return readable(JSON.parse(localStorage.getItem(cache) ?? "")) ?? start();
    } catch {
      return start();
    }
  });
  const [heard, setHeard] = useState(false);
  // The agent's latest change, while it may be undone in one tap.
  const [byAgent, setByAgent] = useState(false);
  const version = useRef(-1);
  const sent = useRef("");
  const pending = useRef(false);
  pending.current = heard && JSON.stringify(saved) !== sent.current;
  const [saveError, setSaveError] = useState(false);
  const read = useRef(readable);
  read.current = readable;
  const adopt = useCallback((k: Kept) => {
    version.current = k.version;
    const s = read.current(k.layout);
    if (!s) return;
    // What the host holds needn't go back to it.
    sent.current = JSON.stringify(s);
    setSaved(s);
    setByAgent(k.by === "agent" && k.undo);
  }, []);
  useEffect(() => {
    let live = true;
    let ask = new AbortController();
    const follow = async () => {
      while (live) {
        try {
          const since = version.current < 0 ? "" : `?since=${version.current}`;
          const r = await api(`${path}${since}`, { cache: "no-store", signal: ask.signal });
          if (!live) return;
          if (r.status === 200) {
            const k = (await r.json()) as Kept;
            if (k.version !== version.current) {
              if (pending.current) version.current = k.version;
              else adopt(k);
            }
            setHeard(true);
          }
          if (!r.ok) await new Promise((w) => setTimeout(w, 5000));
        } catch {
          if (!live) return;
          await new Promise((w) => setTimeout(w, 5000));
        }
      }
    };
    void follow();
    // A phone suspends a hidden page, and the wait may not come back.
    const shown = () => {
      if (document.visibilityState !== "visible") return;
      ask.abort();
      ask = new AbortController();
    };
    document.addEventListener("visibilitychange", shown);
    return () => {
      live = false;
      ask.abort();
      document.removeEventListener("visibilitychange", shown);
    };
  }, [adopt, path]);
  useEffect(() => {
    if (!heard) return;
    const text = JSON.stringify(saved);
    localStorage.setItem(cache, text);
    if (text === sent.current) {
      setSaveError(false);
      return;
    }
    // A drag writes many times a second: the host gets where it settled.
    // Keep an unacknowledged change through a failed request and retry it.
    let live = true;
    let id: number;
    const save = async () => {
      try {
        const r = await api(path, { method: "POST", headers: { "content-type": "application/json" }, body: text });
        if (!r.ok) throw new Error("Layout save refused");
        const k = (await r.json()) as Kept;
        if (!live) return;
        version.current = k.version;
        sent.current = text;
        pending.current = false;
        setSaveError(false);
        setByAgent(false);
      } catch {
        if (!live) return;
        setSaveError(true);
        id = window.setTimeout(() => void save(), 5000);
      }
    };
    id = window.setTimeout(() => void save(), 600);
    return () => {
      live = false;
      window.clearTimeout(id);
    };
  }, [saved, heard, path, cache]);
  const undo = useCallback(() => {
    setByAgent(false);
    void api(`${path}/undo`, { method: "POST" })
      .then(async (r) => r.ok && adopt((await r.json()) as Kept))
      .catch(() => {});
  }, [adopt, path]);
  return [saved, setSaved, byAgent && undo, () => setByAgent(false), heard, saveError] as const;
}

