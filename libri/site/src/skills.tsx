import { useCallback, useEffect, useState } from "react";
import { api } from "./api";
import { Toggle } from "./toggle";

export type Skill = {
  name: string;
  description: string;
  body: string;
  /** Files beside SKILL.md, kept as they are by an edit here. */
  files: number;
  /** From the source repository: changed there, not here. */
  sourced: boolean;
};

/** The git repository Libri takes skills from (skills.rs `Source`). */
export type Source = { url: string | null; commit: string | null; pulled_ago: number | null; failed: string | null; pulling: boolean };

/** A skill of the person's own, outside Libri, and the folders it is in. */
export type Theirs = { name: string; description: string; places: string[] };

export type Skills = {
  /** Whether Libri installs its skills where the harnesses look. */
  on: boolean;
  /** Libri's own store, where a model asked to make a skill writes it. */
  dir: string;
  skills: Skill[];
  /** Folders in the store that aren't a skill, and why. */
  broken: [string, string][];
  /** Libri's skills left out because the person has one by the name. */
  clashes: { name: string; places: string[] }[];
  /** Libri's installed skills the person changed by hand, no longer updated. */
  edited: string[];
  theirs: Theirs[];
  source: Source;
};

const field = "w-full rounded-lg border border-frame bg-paper px-3 py-2 text-caption outline-none focus:border-soft";
const said = async (r: Response | null) => (r ? (await r.text()) || `That failed (${r.status}).` : "The host does not answer.");
const head = "px-5 pt-2 pb-1 text-caption font-medium tracking-[0.06em] text-mid uppercase";

/** Skills as skills.rs keeps them: the person's own, which Libri only
 * reads, and Libri's, installed where the harnesses look while management
 * is on. `now` stays null on a host without them (404). */
export function useSkills(open: boolean) {
  const [now, setNow] = useState<Skills | null>(null);
  const load = useCallback(async () => {
    const r = await api("api/skills", { cache: "no-store" }).catch(() => null);
    setNow(r?.ok ? await r.json() : null);
  }, []);
  useEffect(() => {
    if (open) void load();
  }, [open, load]);
  return [now, load, setNow] as const;
}

/** The words the chat is sent to make a skill: the request as the person
 * might put it, with where Libri's skills go, so any model writes the
 * folder with its own tools. */
const making = (what: string, dir: string) =>
  `Make a skill for me: ${what}\n\nWrite it as an Agent Skills folder in ${dir}: <name>/SKILL.md, starting with name and description frontmatter (the name lowercase with hyphens, the description saying when to use it), then the instructions; put any files it needs beside it. Tell me its name when it's done.`;

const where = (places: string[]) => places.join(", ");

const since = (s: number) => (s < 60 ? "just now" : s < 3600 ? `${Math.round(s / 60)} min ago` : `${Math.round(s / 3600)} h ago`);

/** The repository Libri takes skills from: named, pulled now, or forgotten. */
function SourceRow({ now, setNow, row, note, read }: { now: Skills; setNow: (s: Skills) => void; row: string; note: string; read: boolean }) {
  const [typed, setTyped] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [words, setWords] = useState<string>();
  const src = now.source;
  const post = async (path: string, body?: object) => {
    setBusy(true);
    setWords(undefined);
    const r = await api(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body ?? {}) }).catch(() => null);
    setBusy(false);
    if (!r?.ok) return setWords(await said(r));
    setTyped(null);
    setNow((await r.json()) as Skills);
  };
  const value = typed ?? src.url ?? "";
  const status = busy || src.pulling ? "Pulling…" : src.failed ? `Last pull failed: ${src.failed}` : src.commit ? `At ${src.commit}${src.pulled_ago != null ? `, pulled ${since(src.pulled_ago)}` : ""}` : src.url ? "Not pulled yet." : "";
  return (
    <>
      <h2 className={head}>Source</h2>
      <p className={`${note} px-5 pb-1`}>A git repository of skills, one per folder, pulled every hour.</p>
      <form
        className="flex gap-2 px-5 py-1.5"
        onSubmit={(e) => {
          e.preventDefault();
          void post("api/skills/source", { url: value.trim() || null });
        }}
      >
        <input value={value} onChange={(e) => setTyped(e.target.value)} disabled={read || busy} autoComplete="off" spellCheck={false} placeholder="git@example.com:me/skills.git" aria-label="Skill source repository" className={`${field} flex-1 font-mono`} />
        {!read && (typed !== null ? (
          <button disabled={busy} className="shrink-0 rounded-lg border-[1.25px] border-edge px-3 py-1.5 text-caption hover:bg-hover disabled:opacity-40">Save</button>
        ) : src.url ? (
          <button type="button" disabled={busy || src.pulling} onClick={() => void post("api/skills/pull")} className="shrink-0 rounded-lg border-[1.25px] border-edge px-3 py-1.5 text-caption hover:bg-hover disabled:opacity-40">Pull</button>
        ) : null)}
      </form>
      {status && <p role={src.failed ? "alert" : "status"} className={`px-5 pb-1 text-label ${src.failed ? "text-accent" : "text-mid"}`}>{status}</p>}
      {words && <p role="alert" className="px-5 pb-1 text-label text-accent">{words}</p>}
      {!read && src.url && typed === null && (
        <button type="button" disabled={busy} onClick={() => void post("api/skills/source", { url: null })} className={`${row} text-mid`}>
          Forget the source
        </button>
      )}
    </>
  );
}

export function SkillsPage({ skills: [now, load, setNow], row, note, read, say }: { skills: ReturnType<typeof useSkills>; row: string; note: string; read: boolean; say: (text: string) => void }) {
  // The skill being edited by name, "" for a new one.
  const [editing, setEditing] = useState<string | null>(null);
  const [asking, setAsking] = useState(false);
  const [what, setWhat] = useState("");
  const [busy, setBusy] = useState(false);
  const [words, setWords] = useState<string>();
  if (!now) return null;
  const done = (next?: Skills) => {
    setEditing(null);
    if (next) setNow(next);
    else void load();
  };
  const turn = async () => {
    setBusy(true);
    setWords(undefined);
    const r = await api("api/skills/managed", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ on: !now.on }) }).catch(() => null);
    setBusy(false);
    if (!r?.ok) return setWords(await said(r));
    setNow((await r.json()) as Skills);
  };
  const left = new Map(now.clashes.map((c) => [c.name, c.places]));
  return (
    <>
      <button type="button" role="switch" aria-checked={now.on} disabled={read || busy} onClick={() => void turn()} className={row}>
        Libri's skills
        <Toggle on={now.on} />
      </button>
      <p className={`${note} px-5 pb-2`}>
        {now.on
          ? "Installed where every model finds them. Yours are never touched."
          : "Models see only your own skills."}
      </p>
      {words && <p role="alert" className="px-5 pb-2 text-label text-accent">{words}</p>}
      <SourceRow now={now} setNow={setNow} row={row} note={note} read={read} />
      <h2 className={head}>Libri's</h2>
      {now.skills.map((s) =>
        editing === s.name ? (
          <SkillForm key={s.name} was={s} done={done} row={row} note={note} />
        ) : (
          <button key={s.name} type="button" onClick={() => !read && !s.sourced && setEditing(s.name)} disabled={read || s.sourced} className={`${row} !items-start disabled:!opacity-100`}>
            <span className="flex min-w-0 flex-1 flex-col text-left">
              <span className="flex items-baseline gap-2">
                <span className="font-mono text-caption">{s.name}</span>
                {s.sourced && <span className="text-label text-soft">source</span>}
              </span>
              <span className={`${note} line-clamp-2`}>{s.description}</span>
              {left.has(s.name) && <span className="text-label text-mid">Left out: you have your own {s.name} in {where(left.get(s.name)!)}.</span>}
              {now.edited.includes(s.name) && <span className="text-label text-mid">Edited by hand, so Libri no longer updates it.</span>}
            </span>
          </button>
        ),
      )}
      {!now.skills.length && editing !== "" && <p className={`${note} px-5 py-2`}>None yet. A skill is instructions a model loads when a task needs them.</p>}
      {now.broken.map(([name, why]) => (
        <p key={name} className="px-5 py-1 text-label text-mid">
          <span className="font-mono">{name}</span>: {why}
        </p>
      ))}
      {!read &&
        (editing === "" ? (
          <SkillForm done={done} row={row} note={note} />
        ) : asking ? (
          <form
            className="flex flex-col gap-1.5 px-5 py-2"
            onSubmit={(e) => {
              e.preventDefault();
              if (!what.trim()) return;
              say(making(what.trim(), now.dir));
              setWhat("");
              setAsking(false);
            }}
          >
            <label className={note}>
              What should the skill do?
              <textarea value={what} onChange={(e) => setWhat(e.target.value)} rows={3} autoFocus placeholder="Write my weekly report the way I like it: three sections, short, no jargon." className={`${field} mt-0.5`} />
            </label>
            <div className="flex gap-1.5">
              <button disabled={!what.trim()} className={`${row} flex-1 justify-center rounded-lg`}>Ask the chat</button>
              <button type="button" onClick={() => setAsking(false)} className={`${row} flex-1 justify-center rounded-lg`}>Cancel</button>
            </div>
          </form>
        ) : (
          <>
            <button type="button" onClick={() => setAsking(true)} className={row}>
              Make a skill with the chat
            </button>
            <button type="button" onClick={() => setEditing("")} className={row}>
              Write one yourself
            </button>
          </>
        ))}
      {read && <p className={`${note} px-5 pb-2`}>This device may only read.</p>}
      {!!now.theirs.length && (
        <>
          <h2 className={head}>Yours</h2>
          <p className={`${note} px-5 pb-1`}>Read for the $ menu, never changed.</p>
          {now.theirs.map((t) => (
            <div key={t.name} className="flex flex-col px-5 py-1.5">
              <span className="font-mono text-caption">{t.name}</span>
              <span className={`${note} line-clamp-2`}>{t.description}</span>
              <span className="truncate text-label text-soft" title={where(t.places)}>{where(t.places)}</span>
            </div>
          ))}
        </>
      )}
    </>
  );
}

/** One skill in the composer's menu (skills.rs `Offer`). */
export type SkillOffer = { name: string; description: string; source: "libri" | "yours" };
type Menus = Record<string, { mention: string; skills: SkillOffer[] }>;

// Asked each time a `$` opens the menu, so a skill just made shows without
// a reload.
const fetchMenus = () =>
  api("api/skills/menu", { cache: "no-store" })
    .then((r) => (r.ok ? (r.json() as Promise<Menus>) : null))
    .catch(() => null);

/** The `$` skill menu, as T3 Code has it: a `$` starting a word at the end of
 * the words lists the skills the harness at hand finds, Libri's and the
 * person's marked apart; picking one puts the harness's own way of naming it
 * in place of what was typed. */
export function useSkillMenu(text: string, setText: (t: string) => void, harness: string | undefined) {
  const typed = /(^|\s)\$([a-z0-9-]*)$/.exec(text);
  const [all, setAll] = useState<Menus | null>(null);
  const [at, setAt] = useState(0);
  const [shut, setShut] = useState<string | null>(null);
  const opening = !!typed;
  useEffect(() => {
    if (opening) void fetchMenus().then(setAll);
  }, [opening]);
  // A host with one harness may not say which the thread is on.
  const only = all && Object.keys(all).length === 1 ? Object.keys(all)[0] : undefined;
  const menu = all?.[harness ?? only ?? ""];
  const list = typed && menu && shut !== text ? menu.skills.filter((s) => s.name.startsWith(typed[2])) : [];
  const on = Math.min(at, list.length - 1);
  const take = (s: SkillOffer) => {
    setAt(0);
    setText(`${text.slice(0, text.length - typed![2].length - 1)}${menu!.mention}${s.name} `);
  };
  /** True when the key was the menu's. */
  const keys = (e: { key: string; shiftKey: boolean; preventDefault: () => void }) => {
    if (!list.length) return false;
    const step = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
    if (step) setAt((on + step + list.length) % list.length);
    else if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) take(list[on]);
    else if (e.key === "Escape") setShut(text);
    else return false;
    e.preventDefault();
    return true;
  };
  const shown = !!list.length && (
    <div role="listbox" aria-label="Skills" className="absolute inset-x-0 bottom-[calc(100%+6px)] z-30 max-h-[min(50vh,320px)] overflow-y-auto rounded-2xl border border-frame bg-lift py-1.5 shadow-lift">
      {list.map((s, i) => (
        <button
          key={s.name}
          type="button"
          role="option"
          aria-selected={i === on}
          // Before the textarea's blur, so the words keep their focus.
          onPointerDown={(e) => e.preventDefault()}
          onClick={() => take(s)}
          onPointerEnter={() => setAt(i)}
          className={`flex w-full items-baseline gap-3 px-4 py-1.5 text-left max-sm:min-h-11 max-sm:items-center ${i === on ? "bg-hover" : ""}`}
        >
          <span className="shrink-0 whitespace-nowrap font-mono text-control text-ink">
            {menu!.mention}
            {s.name}
          </span>
          <span className="min-w-0 flex-1 truncate text-control leading-5 text-mid">{s.description}</span>
          <span className="shrink-0 text-label text-soft">{s.source === "libri" ? "Libri" : "Yours"}</span>
        </button>
      ))}
    </div>
  );
  return { menu: shown, keys };
}

function SkillForm({ was, done, row, note }: { was?: Skill; done: (next?: Skills) => void; row: string; note: string }) {
  const [name, setName] = useState(was?.name ?? "");
  const [description, setDescription] = useState(was?.description ?? "");
  const [body, setBody] = useState(was?.body ?? "");
  const [busy, setBusy] = useState(false);
  const [words, setWords] = useState<string>();
  // A delete asks once more before it goes.
  const [sure, setSure] = useState(false);
  const send = async (path: string, init: RequestInit) => {
    setBusy(true);
    setWords(undefined);
    const r = await api(path, init).catch(() => null);
    setBusy(false);
    if (!r?.ok) return setWords(await said(r));
    done((await r.json()) as Skills);
  };
  const save = () =>
    send("api/skills", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ name: name.trim(), description, body }) });
  const drop = () => {
    if (!sure) return setSure(true);
    void send(`api/skills/${encodeURIComponent(was!.name)}/delete`, { method: "POST" });
  };
  return (
    <form
      className="flex flex-col gap-1.5 px-5 py-2"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <label className={note}>
        Name
        <input value={name} onChange={(e) => setName(e.target.value.toLowerCase())} readOnly={!!was} placeholder="weekly-report" autoComplete="off" spellCheck={false} className={`${field} mt-0.5 font-mono`} />
      </label>
      <label className={note}>
        When to use it
        <textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={2} placeholder="Writing the weekly report, or when asked for a summary of the week." className={`${field} mt-0.5`} />
      </label>
      <label className={note}>
        Instructions
        <textarea value={body} onChange={(e) => setBody(e.target.value)} rows={12} placeholder={"# Weekly report\n\n1. …"} spellCheck={false} className={`${field} mt-0.5 font-mono`} />
      </label>
      {!!was?.files && <span className={note}>{was.files === 1 ? "One file" : `${was.files} files`} beside it, kept as they are.</span>}
      <div className="flex gap-1.5">
        <button disabled={busy || !name.trim() || !description.trim()} className={`${row} flex-1 justify-center rounded-lg`}>Save</button>
        <button type="button" onClick={() => done()} className={`${row} flex-1 justify-center rounded-lg`}>Cancel</button>
      </div>
      {was && (
        <button type="button" onClick={drop} disabled={busy} className={`${row} justify-center rounded-lg ${sure ? "text-accent" : ""}`}>
          {sure ? `Delete ${was.name} and its files` : "Delete"}
        </button>
      )}
      {words && <p role="alert" className="text-label text-accent">{words}</p>}
    </form>
  );
}
