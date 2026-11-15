// Search over everything said: main's chat whole, every message back to the
// first, and every worker's log, archived ones too, as ChatGPT's search
// reaches archived chats. Results come as you type, grouped by where they
// were said, as Slack groups them; picking one opens it in its place, the
// messages around it, as Slack and Open WebUI open a hit at its message.
// The host keeps the index (hippo's `find`), so a search is a few
// milliseconds there and the page only draws.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "./api";
import { md } from "./words";

/** A message found: `src` is "" for the chat, else the worker's folder;
 * `at` is the message's id there; the matched words sit between STX and ETX. */
type Hit = { src: string; at: number; kind: string; time: number; text: string };
type Found = { hits: Hit[]; loose: boolean };
type Source = { src: string; name: string; began: number; archived: boolean };
type Line = { at: number; kind: string; time: number; text: string; cut?: boolean };
type Order = "best" | "new";

const OPEN = "open-search";
/** Command, not Control, is the Mac's key for this. */
export const MAC = /Mac|iPhone|iPad/.test(navigator.platform);
/** Opens search from anywhere: the left column's row, the header's button. */
export const openSearch = () => window.dispatchEvent(new Event(OPEN));

/** Who said a line, as the chat names them. */
function who(kind: string, name: string): string {
  return { user: "You", talk: name, tool: "Tool", echo: "Output", work: "Report", note: "Note", from: "From a worker", to: "To a worker" }[kind] ?? kind;
}

const DAY = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });
const YEAR = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", year: "numeric" });
const CLOCK = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });

function when(s: number, clock = false): string {
  if (!s) return "";
  const d = new Date(s * 1000);
  const day = (d.getFullYear() === new Date().getFullYear() ? DAY : YEAR).format(d);
  return clock ? `${day}, ${CLOCK.format(d)}` : day;
}

/** Markdown's emphasis and code marks, which read as noise in plain text. */
const plain = (t: string) => t.replace(/\*\*|`/g, "");

/** The words the index matched in a snippet, as they stand in the text. */
const matched = (snippet: string) => [...new Set([...snippet.matchAll(/\u0002([^\u0003]*)\u0003/g)].map((m) => m[1].toLowerCase()))];

/** The snippet's marked words drawn marked. */
function Marked({ text }: { text: string }) {
  // A tool call's input is JSON, its line breaks escaped: spaces read better.
  const parts = plain(text).replace(/\\n/g, " ").split(/[\u0002\u0003]/);
  return (
    <>
      {parts.map((p, k) =>
        k % 2 ? (
          <mark key={k} className="rounded-sm bg-frame/60 px-px font-medium text-ink">
            {p}
          </mark>
        ) : (
          p
        ),
      )}
    </>
  );
}

/** Marks the words the index matched wherever they stand whole in `root`'s
 * text, by the CSS Custom Highlight API: the words are drawn by Streamdown,
 * so no mark can go into them. Without the API the hit keeps its frame. */
function useMarks(root: React.RefObject<HTMLElement | null>, words: string[], drawn: unknown) {
  useEffect(() => {
    const el = root.current;
    if (!el || !words.length || !("highlights" in CSS)) return;
    const re = new RegExp(`(?<![\\p{L}\\p{N}])(${words.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})(?![\\p{L}\\p{N}])`, "giu");
    const ranges: Range[] = [];
    const walk = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    for (let n = walk.nextNode(); n; n = walk.nextNode()) {
      for (const m of (n.textContent ?? "").matchAll(re)) {
        const r = new Range();
        r.setStart(n, m.index);
        r.setEnd(n, m.index + m[0].length);
        ranges.push(r);
      }
    }
    CSS.highlights.set("search-hit", new Highlight(...ranges));
    return () => void CSS.highlights.delete("search-hit");
  }, [root, words, drawn]);
}

async function find(q: string, order: Order, signal: AbortSignal): Promise<Found | string> {
  const r = await api(`api/find?q=${encodeURIComponent(q)}&order=${order}`, { signal });
  if (r.ok) return r.json();
  return r.status === 503 ? await r.text() : `Search failed (${r.status}).`;
}

async function around(src: string, at: number, before: number, after: number): Promise<Line[] | undefined> {
  const r = await api(`api/around?src=${encodeURIComponent(src)}&at=${at}&before=${before}&after=${after}`);
  return r.ok ? r.json() : undefined;
}

/** Hits grouped by where each was said, a group's place set by its best hit. */
function groups_of(hits: Hit[]): [string, Hit[]][] {
  const by = new Map<string, Hit[]>();
  for (const h of hits) by.set(h.src, [...(by.get(h.src) ?? []), h]);
  return [...by.entries()];
}

/** The first hit as the list shows it: the best group's best. */
const grouped = (hits: Hit[]) => groups_of(hits)[0]?.[1][0];

/** The search, mounted once: a centred window on a wide screen with the
 * results beside the one opened, a page of its own on a phone. Ctrl+K or
 * ⌘K opens it, as in Slack, Linear and ChatGPT; Escape steps back. */
export function Search({ wide, name }: { wide: boolean; name: string }) {
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const [order, setOrder] = useState<Order>("best");
  const [found, setFound] = useState<Found | string>();
  const [busy, setBusy] = useState(false);
  const [active, setActive] = useState(0);
  const [picked, setPicked] = useState<Hit>();
  const [sources, setSources] = useState<Map<string, Source>>(new Map());
  const field = useRef<HTMLInputElement>(null);
  // What Escape reads, from a listener made once.
  const now = useRef({ open, back: () => {} });

  useEffect(() => {
    const show = () => setOpen(true);
    const key = (e: KeyboardEvent) => {
      // A key the terminal took is the shell's (Ctrl K kills to the end of
      // the line), and on a Mac Ctrl K does the same in any text field.
      if (e.defaultPrevented) return;
      if ((MAC ? e.metaKey : e.ctrlKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((o) => !o);
      } else if (e.key === "Escape" && now.current.open) {
        // From anywhere in it, not only the field.
        now.current.back();
      }
    };
    window.addEventListener(OPEN, show);
    document.addEventListener("keydown", key);
    return () => {
      window.removeEventListener(OPEN, show);
      document.removeEventListener("keydown", key);
    };
  }, []);

  // The workers' names, asked again on each opening: workers come and go.
  useEffect(() => {
    if (!open) return;
    requestAnimationFrame(() => field.current?.select());
    api("api/sources")
      .then((r) => (r.ok ? r.json() : []))
      .then((all: Source[]) => setSources(new Map(all.map((s) => [s.src, s]))))
      .catch(() => {});
  }, [open]);

  // As typed: the newest asking wins, and one overtaken is called off.
  useEffect(() => {
    if (!open || !q.trim()) {
      setFound(undefined);
      setBusy(false);
      return;
    }
    const stop = new AbortController();
    setBusy(true);
    const t = window.setTimeout(() => {
      find(q, order, stop.signal)
        .then((f) => {
          setFound(f);
          setActive(0);
          setBusy(false);
          // Beside the list the first hit shows in its place, as Raycast
          // previews the one selected; a phone opens one only when tapped.
          if (wide) setPicked(typeof f === "string" ? undefined : order === "new" ? f.hits[0] : grouped(f.hits));
        })
        .catch(() => {});
    }, 40);
    return () => {
      window.clearTimeout(t);
      stop.abort();
    };
  }, [q, order, open, wide]);

  // Newest is one time order across every source, as Slack drops its
  // grouping when sorted by date; each row then names where it was said.
  const groups = useMemo(() => (!found || typeof found === "string" ? [] : order === "new" ? [["*", found.hits] as [string, Hit[]]] : groups_of(found.hits)), [found, order]);
  const flat = useMemo(() => groups.flatMap(([, hs]) => hs), [groups]);

  const close = useCallback(() => {
    setOpen(false);
    setPicked(undefined);
  }, []);
  // A phone steps back from a hit to the list first.
  const back = () => (picked && !wide ? setPicked(undefined) : close());
  now.current = { open, back };
  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = Math.max(0, Math.min(flat.length - 1, active + (e.key === "ArrowDown" ? 1 : -1)));
      setActive(next);
      if (wide && flat[next]) setPicked(flat[next]);
      document.getElementById(`hit-${next}`)?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter" && flat[active]) {
      e.preventDefault();
      setPicked(flat[active]);
    }
  };
  if (!open) return null;

  const label = (src: string) => (src ? (sources.get(src)?.name ?? src) : "Chat");
  const list = (
    <div className="min-h-0 flex-1 overflow-y-auto pb-[max(12px,env(safe-area-inset-bottom))]">
      {typeof found === "string" ? (
        <p className="px-5 py-4 text-control text-mid">{found}</p>
      ) : !q.trim() ? (
        <p className="px-5 py-4 text-control text-soft">Everything said here and by every worker. Words in quotes are found together.</p>
      ) : found && !found.hits.length ? (
        <p className="px-5 py-4 text-control text-mid">Nothing found.</p>
      ) : (
        <>
          {found?.loose && <p className="px-5 pt-3 text-label text-soft">No message has every word; these have some.</p>}
          {groups.map(([src, hs]) => {
            const s = sources.get(src);
            return (
              <section key={src} aria-label={src === "*" ? "Newest" : label(src)} className="pt-3">
                {src !== "*" && (
                  <h3 className="flex items-baseline gap-2 px-5 pb-1 text-caption text-soft">
                    <span className="font-semibold text-mid">{label(src)}</span>
                    {s && <span>{[when(s.began), s.archived && "archived"].filter(Boolean).join(" · ")}</span>}
                  </h3>
                )}
                {hs.map((h) => {
                  const k = flat.indexOf(h);
                  const on = picked ? picked === h : wide && k === active;
                  return (
                    <button
                      key={`${h.src}:${h.at}`}
                      id={`hit-${k}`}
                      type="button"
                      onClick={() => {
                        setActive(k);
                        setPicked(h);
                      }}
                      className={`mx-2 flex w-[calc(100%-16px)] flex-col gap-0.5 rounded-xl px-3 py-2 text-left ${on ? "bg-hover" : "hover:bg-hover/50"}`}
                    >
                      <span className="flex gap-2 text-caption text-soft">
                        {src === "*" && <span className="font-semibold text-mid">{label(h.src)}</span>}
                        <span>{who(h.kind, name)}</span>
                        <span>{when(h.time)}</span>
                      </span>
                      <span className="line-clamp-3 text-control leading-snug text-mid [overflow-wrap:anywhere]">
                        <Marked text={h.text} />
                      </span>
                    </button>
                  );
                })}
              </section>
            );
          })}
        </>
      )}
    </div>
  );
  const head = (
    <div className="flex shrink-0 items-center gap-2 border-hair px-3 lg:border-b pt-[max(8px,env(safe-area-inset-top))] pb-2 lg:pt-2">
      {!wide && (
        <button type="button" onClick={back} aria-label={picked ? "Back to results" : "Close search"} className="grid size-10 shrink-0 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink">
          <svg viewBox="0 0 24 24" className="size-[20px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
            <path d="M15 6l-6 6 6 6" />
          </svg>
        </button>
      )}
      <svg viewBox="0 0 24 24" className={`size-[18px] shrink-0 text-soft ${wide ? "ml-2" : ""}`} fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" aria-hidden>
        <circle cx="11" cy="11" r="7" />
        <path d="m20 20-3.5-3.5" />
      </svg>
      <input
        ref={field}
        type="search"
        value={q}
        onChange={(e) => {
          setQ(e.target.value);
          if (!wide) setPicked(undefined);
        }}
        onKeyDown={onKey}
        placeholder="Search everything said"
        aria-label="Search everything said"
        enterKeyHint="search"
        className="h-10 min-w-0 flex-1 bg-transparent text-prose outline-none placeholder:text-soft [&::-webkit-search-cancel-button]:hidden"
      />
      {busy && <span className="size-1.5 shrink-0 animate-pulse rounded-full bg-soft" aria-label="Searching" />}
      <div role="radiogroup" aria-label="Order" className="flex shrink-0 rounded-full border border-hair p-0.5 text-caption">
        {(["best", "new"] as const).map((o) => (
          <button key={o} type="button" role="radio" aria-checked={order === o} onClick={() => setOrder(o)} className={`h-7 rounded-full px-2.5 ${order === o ? "bg-hover text-ink" : "text-soft hover:text-mid"}`}>
            {o === "best" ? "Best" : "Newest"}
          </button>
        ))}
      </div>
      {wide && (
        <button type="button" onClick={close} aria-label="Close search" className="grid size-9 shrink-0 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink">
          <svg viewBox="0 0 24 24" className="size-[18px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" aria-hidden>
            <path d="M18 6 6 18M6 6l12 12" />
          </svg>
        </button>
      )}
    </div>
  );

  if (wide)
    return (
      <>
        <div onClick={close} aria-hidden className="fixed inset-0 z-40 bg-black/20" />
        <div role="dialog" aria-modal aria-label="Search" className="fixed top-[8vh] left-1/2 z-50 flex h-[min(720px,84vh)] w-[min(1040px,calc(100%-48px))] -translate-x-1/2 flex-col overflow-hidden rounded-2xl border-[1.25px] border-edge bg-paper shadow-lift">
          {head}
          <div className="flex min-h-0 flex-1">
            <div className="flex w-[400px] shrink-0 flex-col border-r border-hair">{list}</div>
            {picked ? <Place key={`${picked.src}:${picked.at}`} hit={picked} title={label(picked.src)} name={name} /> : <div className="flex-1" />}
          </div>
        </div>
      </>
    );
  return (
    <div role="dialog" aria-modal aria-label="Search" className="fixed inset-0 z-50 flex flex-col bg-paper pl-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)]">
      {head}
      {picked ? <Place key={`${picked.src}:${picked.at}`} hit={picked} title={label(picked.src)} name={name} /> : list}
    </div>
  );
}

/** Lines as the reader draws them: words one by one, and each run of tool
 * calls and their output folded into one row, as the chat folds them,
 * unless the hit is in it. */
type Row = { line: Line } | { run: Line[] };
function rows(lines: Line[], hit: number): Row[] {
  const out: Row[] = [];
  for (const l of lines) {
    const tool = l.kind === "tool" || l.kind === "echo";
    const last = out.at(-1);
    if (tool && last && "run" in last) last.run.push(l);
    else out.push(tool ? { run: [l] } : { line: l });
  }
  return out.flatMap((r) => ("run" in r && r.run.some((l) => l.at === hit) ? r.run.map((line) => ({ line })) : [r]));
}

/** A hit in its place: the messages around it, more on asking either way. */
function Place({ hit, title, name }: { hit: Hit; title: string; name: string }) {
  const [lines, setLines] = useState<Line[] | "failed">();
  const [span, setSpan] = useState({ before: 12, after: 12 });
  const mark = useRef<HTMLDivElement>(null);
  const first = useRef(true);
  const words = useMemo(() => matched(hit.text), [hit]);
  useMarks(mark, words, lines);
  useEffect(() => {
    let gone = false;
    around(hit.src, hit.at, span.before, span.after).then((l) => !gone && setLines(l ?? "failed"));
    return () => {
      gone = true;
    };
  }, [hit, span]);
  // Centred once, when first drawn; asking for more keeps the reader's place.
  useEffect(() => {
    if (first.current && Array.isArray(lines)) {
      first.current = false;
      // Centred, unless taller than the pane: then its top, so it reads
      // from its start.
      const el = mark.current;
      const pane = el?.closest(".overflow-y-auto");
      el?.scrollIntoView({ block: pane && el.offsetHeight > pane.clientHeight * 0.7 ? "start" : "center" });
    }
  }, [lines]);
  if (!lines) return <div className="flex-1" />;
  if (lines === "failed") return <p className="flex-1 px-5 py-4 text-control text-mid">Couldn't open it.</p>;
  const more = (side: "before" | "after") => setSpan((s) => ({ ...s, [side]: s[side] + 40 }));
  const top = lines[0]?.at > 0 && lines[0]?.at === hit.at - span.before;
  const bottom = lines.length > 0 && lines[lines.length - 1].at === hit.at + span.after;
  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="sticky top-0 z-10 border-hair bg-paper px-5 lg:border-b py-2 text-caption text-soft">
        <span className="font-semibold text-mid">{title}</span> · {when(hit.time, true)}
      </div>
      <div className="flex flex-col gap-1 px-3 py-3 pb-[max(16px,env(safe-area-inset-bottom))]">
        {top && (
          <button type="button" onClick={() => more("before")} className="self-center rounded-full px-3 py-1 text-label text-soft hover:bg-hover hover:text-mid">
            Earlier
          </button>
        )}
        {rows(lines, hit.at).map((r) => ("run" in r ? <Run key={r.run[0].at} run={r.run} /> : <One key={r.line.at} l={r.line} here={r.line.at === hit.at} at={r.line.at === hit.at ? mark : undefined} name={name} />))}
        {bottom && (
          <button type="button" onClick={() => more("after")} className="self-center rounded-full px-3 py-1 text-label text-soft hover:bg-hover hover:text-mid">
            Later
          </button>
        )}
      </div>
    </div>
  );
}

/** One line: words drawn as the chat draws them, a tool's in its own type. */
function One({ l, here, at, name }: { l: Line; here: boolean; at?: React.RefObject<HTMLDivElement | null>; name: string }) {
  const code = l.kind === "tool" || l.kind === "echo";
  return (
    <div ref={at} className={`scroll-mt-12 rounded-xl px-3 py-2 ${here ? "bg-hover ring-[1.25px] ring-frame" : ""}`}>
      <div className={`text-caption ${l.kind === "user" || l.kind === "talk" ? "font-semibold text-mid" : "text-soft"}`}>{who(l.kind, name)}</div>
      {code ? (
        <div className={`mt-0.5 font-mono text-caption whitespace-pre-wrap text-mid [overflow-wrap:anywhere] ${here ? "" : "line-clamp-4"}`}>
          {l.text}
          {l.cut && <span className="text-soft"> …</span>}
        </div>
      ) : (
        <div className="md mt-0.5 text-control text-ink">{md(l.cut ? `${l.text} …` : l.text)}</div>
      )}
    </div>
  );
}

/** A run of tool calls and their output, one row until opened. */
function Run({ run }: { run: Line[] }) {
  const [open, setOpen] = useState(false);
  const calls = run.filter((l) => l.kind === "tool").length || run.length;
  return (
    <div className="px-3">
      <button type="button" onClick={() => setOpen(!open)} aria-expanded={open} className="flex items-center gap-1.5 rounded-lg py-1 text-label text-soft hover:text-mid">
        <svg viewBox="0 0 24 24" className={`size-3.5 transition-transform motion-reduce:transition-none ${open ? "rotate-90" : ""}`} fill="none" stroke="currentColor" strokeWidth={2.4} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
          <path d="m9 6 6 6-6 6" />
        </svg>
        Ran {calls} {calls === 1 ? "tool" : "tools"}
      </button>
      {open && (
        <div className="flex flex-col gap-1 border-l border-hair pl-2">
          {run.map((l) => (
            <div key={l.at} className="font-mono text-caption whitespace-pre-wrap text-mid [overflow-wrap:anywhere] line-clamp-6">
              {l.text}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
