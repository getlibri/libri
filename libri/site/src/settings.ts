import { useEffect, useState } from "react";
import { api } from "./api";

/** Optional host setting: missing endpoints stay hidden, failed writes keep
 * the last value, and successful writes return the host's canonical value. */
export function useSetting<T>(path: string, want: boolean) {
  const [now, setNow] = useState<T | null>(null);
  const [said, say] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!want) return;
    api(path)
      .then((r) => {
        if (r.status === 404) return;
        if (!r.ok) throw new Error();
        return r.json().then((value: T) => {
          setNow(value);
          say(null);
        });
      })
      .catch(() => say("The host didn't answer."));
  }, [path, want]);
  const change = async (body: object) => {
    say(null);
    setBusy(true);
    try {
      const r = await api(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) });
      if (!r.ok) {
        say(await r.text());
        return false;
      }
      setNow((await r.json()) as T);
      return true;
    } catch {
      say("The host didn't answer.");
      return false;
    } finally {
      setBusy(false);
    }
  };
  return { now, said, busy, change };
}

/** Only the saved choice goes to the host, never catalog display fields. */
export const modelSetting = (o?: { harness: string; model: string }) => o ? { harness: o.harness, model: o.model } : {};
