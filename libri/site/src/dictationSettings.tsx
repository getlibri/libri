import { useCallback, useEffect, useState } from "react";
import { api } from "./api";
import { useListen } from "./dictate";

// Settings, Voice, Dictation: where speech is written down (stt.rs). The
// host's own model is the default, keeping the words on the host once the
// model is fetched; xAI and any OpenAI-compatible API are the other choices.

type Provider = "local" | "xai" | "openai";
type Model = { id: string; name: string; about: string; languages: string[]; bytes: number; installed: boolean; downloading?: { got: number; total: number } };
type Choice = { provider: Provider; model: string; openai_url: string; openai_model: string };
type Now = { choice: Choice; models: Model[]; local: boolean; xai: boolean; openai_key: boolean };

const megabytes = (b: number) => `${Math.round(b / 1e6)} MB`;

export function DictationSettings({ row, note, select, read, changed }: { row: string; note: string; select: string; read: boolean; changed: () => void }) {
  const [now, setNow] = useState<Now | null>(null);
  const [said, setSaid] = useState<string>();
  const [key, setKey] = useState("");
  const [heard, setHeard] = useState("");
  const load = useCallback(async () => {
    const r = await api("api/stt").catch(() => null);
    if (r?.ok) setNow((await r.json()) as Now);
  }, []);
  useEffect(() => void load(), [load]);
  const fetching = now?.models.some((m) => m.downloading);
  useEffect(() => {
    if (!fetching) return;
    const t = setInterval(() => void load(), 1500);
    return () => clearInterval(t);
  }, [fetching, load]);
  const test = useListen(setHeard, (why) => setHeard(why));
  if (!now) return null;

  const choose = async (change: Partial<Choice>, openai_key?: string) => {
    setSaid(undefined);
    const r = await api("api/stt", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ ...change, ...(openai_key !== undefined ? { openai_key } : {}) }),
    }).catch(() => null);
    if (!r?.ok) return setSaid(r ? (await r.text()) || `That failed (${r.status}).` : "The host does not answer.");
    setNow((await r.json()) as Now);
    changed();
  };

  const { choice } = now;
  const model = now.models.find((m) => m.id === choice.model);
  const ready = choice.provider === "local" ? !!model?.installed : choice.provider === "xai" ? now.xai : now.openai_key;
  const field = "w-44 rounded-md border border-frame bg-transparent px-2 py-1 text-label outline-none focus:border-ink";
  return (
    <>
      <div className="flex w-full items-center justify-between gap-3 px-5 py-2.5 text-body">
        <label htmlFor="dictation">Transcribe with</label>
        <select id="dictation" value={choice.provider} disabled={read} onChange={(e) => void choose({ provider: e.target.value as Provider })} className={select}>
          {now.local && <option value="local">This host</option>}
          <option value="xai">xAI (Grok)</option>
          <option value="openai">OpenAI-compatible API</option>
        </select>
      </div>
      {choice.provider === "local" &&
        now.models.map((m) => {
          const picked = m.id === choice.model;
          const status = m.downloading ? `Fetching ${m.downloading.total ? Math.floor((100 * m.downloading.got) / m.downloading.total) : 0}%` : m.installed ? "Here" : megabytes(m.bytes);
          return (
            <button key={m.id} type="button" disabled={read} onClick={() => void choose({ model: m.id })} className={row} aria-pressed={picked}>
              <span className="min-w-0 flex-1">
                <span className="block">
                  {picked ? "● " : "○ "}
                  {m.name}
                </span>
                <span className={`${note} block`}>{m.about}</span>
              </span>
              <span className={`${note} shrink-0`}>{status}</span>
            </button>
          );
        })}
      {choice.provider === "xai" && !now.xai && <p className={`px-5 py-2.5 ${note}`}>Connect xAI under Connections first.</p>}
      {choice.provider === "openai" && (
        <>
          <label className="flex w-full items-center justify-between gap-3 px-5 py-2 text-body">
            Address
            <input className={field} defaultValue={choice.openai_url} disabled={read} onBlur={(e) => e.target.value !== choice.openai_url && void choose({ openai_url: e.target.value.trim() })} />
          </label>
          <label className="flex w-full items-center justify-between gap-3 px-5 py-2 text-body">
            Model
            <input className={field} defaultValue={choice.openai_model} disabled={read} onBlur={(e) => e.target.value !== choice.openai_model && void choose({ openai_model: e.target.value.trim() })} />
          </label>
          <form
            className="flex w-full items-center justify-between gap-3 px-5 py-2 text-body"
            onSubmit={(e) => {
              e.preventDefault();
              void choose({}, key.trim()).then(() => setKey(""));
            }}
          >
            <label htmlFor="stt-key">Key</label>
            <input id="stt-key" type="password" className={field} value={key} disabled={read} placeholder={now.openai_key ? "Saved; type to replace" : "sk-…"} onChange={(e) => setKey(e.target.value)} />
          </form>
        </>
      )}
      <div className="flex w-full items-center justify-between gap-3 px-5 py-2.5 text-body">
        <span className="min-w-0 flex-1">
          <span className="block">Try it</span>
          <span className={`${note} block`}>{test.live || heard || (ready ? "Speak a sentence, then stop." : "Not ready yet.")}</span>
        </span>
        <button
          type="button"
          disabled={!ready || test.state === "writing"}
          onClick={() => {
            if (test.state === "hearing") return test.stop();
            setHeard("");
            test.start();
          }}
          className="shrink-0 rounded-full px-2.5 py-1 text-label text-mid hover:bg-hover hover:text-ink disabled:opacity-40"
        >
          {test.state === "hearing" ? "Stop" : test.state === "writing" ? "Writing…" : "Start"}
        </button>
      </div>
      {said && <p className="px-5 py-2.5 text-body text-accent">{said}</p>}
    </>
  );
}
