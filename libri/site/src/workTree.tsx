// The Workers tile as a tree, as a git commit graph draws its branches: the
// memories are the roots, and each worker hangs from the memory whose chat
// started it (and forked it) or from the worker that asked for it. Each
// worker is a card: its name and model, its title, and a status line the
// summarizer refreshes while it works (synopsis.rs). Colour says what a node
// is doing, the same across the whole tree (ACTS). A lane glows in its
// worker's colour while that worker works and is dim otherwise; it runs
// dashes only while a message travels along it between two agents, the way
// the message goes (task::Talk). Click folds a subtree, double-click opens the thread. A
// harness's own subagents (Claude Code's Agent calls) show as hollow italic
// leaves, to look at only.
import { useEffect, useState } from "react";
import { firstLine, type Task } from "./chat";
import type { Watches } from "./library";
import { useMemories, useMemoryChats } from "./memoryChats";
import { modelName, useOffers, type NamedOffer } from "./modelNames";
import { Held } from "./hold";
import { ago, NEED, stateOf, WatchList, ArchivedList, workerMenu, type Talk, type Thread, type Work } from "./work";

/** What a node is doing, as far as the page can tell. Worker thinking never
 * reaches the page (agui.rs keeps only a log's words), so thinking and
 * writing are one; a tool is told apart by its name alone. */
export type Act = "thinking" | "reading" | "editing" | "running" | "person" | "agent" | "failed" | "idle";

/** One palette for the whole tree, each far from the others in light and
 * dark; amber stays the person's, red the failure's. */
export const ACTS: Record<Act, { colour: string; label: string }> = {
  thinking: { colour: "#6c8cff", label: "Thinking or writing" },
  reading: { colour: "#2bb3a3", label: "Reading or searching" },
  editing: { colour: "#b07cff", label: "Editing files" },
  running: { colour: "#3ccf91", label: "Running a command or tool" },
  person: { colour: "#f2a93b", label: "Waiting on you" },
  agent: { colour: "#8a94a8", label: "Waiting on an agent, or napping" },
  failed: { colour: "#e5484d", label: "Last run failed" },
  idle: { colour: "var(--color-soft)", label: "Idle or done" },
};

/** Tool names, as Claude Code, Codex, omp and OpenCode name them, by what
 * they do. Any other call is `running`. */
const READS = /(^|[_.])(read|grep|glob|find|ls|list|search|fetch|view|look|zoom|cat|web_?search|web_?fetch|notebookread)/i;
const EDITS = /(^|[_.])(edit|multiedit|write|apply_?patch|patch|notebookedit|create_file|str_replace)/i;
const actOfCall = (tool: string): Act => {
  const name = tool.replace(/^mcp__\w+?__/, "");
  return EDITS.test(name) ? "editing" : READS.test(name) ? "reading" : "running";
};

/** The acts that are work under way: a lane to one of them glows. */
const WORKING: Act[] = ["thinking", "reading", "editing", "running"];

/** How long a lane runs its dashes for one message, in milliseconds. */
const TALK_SHOWN = 4000;

const CARD = 64;
const ROOT = 36;
const LEAF = 34;
const X0 = 18;
const DX = 22;
const FOLDED = "libri-tree-folded";
const TAB = "libri-workers-tab";

export type Node = {
  key: string;
  name: string;
  kind: "memory" | "worker" | "harness";
  act: Act;
  /** The harness, model and thinking level, short. */
  model?: string;
  /** How its harness's report differs from what it was started on. */
  mismatch?: string;
  title?: string;
  status?: string;
  at?: number;
  kids: Node[];
  open?: () => void;
  /** The worker it draws, for its hold menu. */
  thread?: Thread;
};

export const isAgent = (name: string) => name === "Agent" || name === "Task";

/** A worker's harness, its model by the name the model picker shows it
 * (modelNames.ts), then its thinking level. */
export function shortModel(t: Pick<Thread, "pick" | "effort">, offers: NamedOffer[]): string | undefined {
  const pick = t.pick;
  if (!pick) return undefined;
  return [pick.harness, modelName(pick.harness, pick.model, offers), t.effort].filter(Boolean).join(" · ");
}

/** What worker `t` is doing: what the host says of it first, then, while it
 * works, the call its stream has open, else the one its log has open (a
 * call begun before this page was listening). */
export function actOf(t: Thread, task: Task | undefined): Act {
  if (t.state === "waiting") return t.failed ? "failed" : "person";
  if (t.state === "working") {
    if (t.waiting_on?.length) return "agent";
    const call = [...(task?.tools ?? [])].reverse().find((c) => c.result === undefined && !isAgent(c.name));
    const tool = call?.name ?? t.calling;
    return tool ? actOfCall(tool) : "thinking";
  }
  if (t.state === "napping") return "agent";
  return t.failed ? "failed" : "idle";
}

/** A worker's own harness subagents still running, as its calls report them. */
export function harnessOf(task: Task | undefined, key: string): Node[] {
  return (task?.tools ?? [])
    .filter((c) => isAgent(c.name) && c.result === undefined)
    .map((c) => {
      const input = (c.input ?? {}) as {
        description?: string;
        subagent_type?: string;
      };
      return {
        key: `${key}/${c.id}`,
        name: input.subagent_type ?? "Agent",
        status: input.description,
        kind: "harness",
        act: "running",
        kids: [],
      };
    });
}

/** Each message in `talk` with when this page first saw it, kept while it
 * still shows, and a clock that ticks when the next one stops. */
function useTalkSeen(talk: Talk[]): Map<string, { t: Talk; seen: number }> {
  const [seen, setSeen] = useState(() => new Map<string, { t: Talk; seen: number }>());
  const [, tick] = useState(0);
  const now = Date.now();
  const fresh = talk.filter((t) => !seen.has(`${t.at}|${t.from}|${t.to}`));
  useEffect(() => {
    if (!fresh.length) return;
    setSeen((was) => {
      const next = new Map([...was].filter(([, s]) => Date.now() - s.seen < TALK_SHOWN));
      for (const t of fresh) next.set(`${t.at}|${t.from}|${t.to}`, { t, seen: Date.now() });
      return next;
    });
  });
  const live = [...seen.values()].filter((s) => now - s.seen < TALK_SHOWN);
  useEffect(() => {
    if (!live.length) return;
    const due = Math.min(...live.map((s) => s.seen + TALK_SHOWN)) - Date.now();
    const id = window.setTimeout(() => tick((n) => n + 1), Math.max(0, due) + 50);
    return () => window.clearTimeout(id);
  });
  return new Map([...seen].filter(([, s]) => now - s.seen < TALK_SHOWN));
}

/** The colours' key, named as one: eight bare dots read as decoration. */
function Legend() {
  const [open, setOpen] = useState(false);
  const shown: Act[] = ["thinking", "reading", "editing", "running", "person", "agent", "failed", "idle"];
  return (
    <div className="relative ml-auto" onMouseLeave={() => setOpen(false)}>
      <button
        type="button"
        aria-expanded={open}
        aria-label="What the colours mean"
        title="What the colours mean"
        onClick={() => setOpen(!open)}
        onMouseEnter={() => setOpen(true)}
        className="flex h-7 items-center gap-1.5 rounded-full px-2.5 text-caption text-soft hover:bg-hover hover:text-mid"
        data-legend
      >
        <span className="grid grid-cols-4 gap-0.5" aria-hidden>
          {shown.map((a) => (
            <span key={a} className="size-1.5 rounded-full" style={{ background: ACTS[a].colour }} />
          ))}
        </span>
        Key
      </button>
      {open && (
        <ul role="note" className="absolute top-full right-0 z-10 mt-1 rounded-lg border border-frame bg-lift p-2 text-caption text-ink shadow-lift" data-legend-list>
          {shown.map((a) => (
            <li key={a} className="flex items-center gap-2 py-0.5 whitespace-nowrap">
              <span className="size-2.5 shrink-0 rounded-full" style={{ background: ACTS[a].colour }} />
              {ACTS[a].label}
            </li>
          ))}
          <li className="flex items-center gap-2 py-0.5 whitespace-nowrap">
            <svg width="10" height="10" aria-hidden>
              <line x1="5" y1="0" x2="5" y2="10" stroke="var(--color-ink)" strokeWidth="2" strokeDasharray="2 3" />
            </svg>
            A message between two agents
          </li>
        </ul>
      )}
    </div>
  );
}

type Tree = Parameters<typeof WorkTree>[0];

const TABS = ["tree", "watches", "archived"] as const;
type Tab = (typeof TABS)[number];

/** The Workers tile: the tree of what lives, the watches, and the workers
 * put away, each on a tab of its own so none crowds the others. Archived
 * shows its count only once opened, so a pile of old workers never reads
 * as something to do. */
export function WorkersTile({ watches, ...tree }: Tree & { watches: Watches }) {
  const [tab, setTab] = useState<Tab>(() => TABS.find((t) => t === localStorage.getItem(TAB)) ?? "tree");
  const pick = (t: Tab) => {
    if (t === "tree") localStorage.removeItem(TAB);
    else localStorage.setItem(TAB, t);
    setTab(t);
  };
  const archived = tree.work.threads.filter((t) => t.state === "archived").length;
  const tabs: { id: Tab; label: string; count?: number }[] = [
    { id: "tree", label: "Workers", count: tree.work.threads.length - archived },
    { id: "watches", label: "Watches", count: watches.watches.length },
    { id: "archived", label: "Archived" },
  ];
  return (
    <>
      <div className="flex items-center gap-1 px-3 pb-2">
        <div role="tablist" aria-label="Workers" className="flex items-center gap-1">
          {tabs.map((t) => (
            <button
              key={t.id}
              type="button"
              role="tab"
              id={`workers-tab-${t.id}`}
              aria-selected={tab === t.id}
              aria-controls={`workers-panel-${t.id}`}
              onClick={() => pick(t.id)}
              className={`h-7 rounded-full border px-3 text-caption whitespace-nowrap ${tab === t.id ? "border-hair bg-hover font-medium text-ink" : "border-transparent text-soft hover:text-mid"}`}
            >
              {t.label}
              {!!t.count && <span className="ml-1.5 text-soft tabular-nums">{t.count}</span>}
            </button>
          ))}
        </div>
        {tab === "tree" && <Legend />}
      </div>
      <div role="tabpanel" id={`workers-panel-${tab}`} aria-labelledby={`workers-tab-${tab}`}>
        {tab === "tree" ? (
          <WorkTree {...tree} />
        ) : tab === "watches" ? (
          watches.watches.length ? <WatchList watches={watches} /> : <p className="px-5 pt-4 text-control text-soft">No watches.</p>
        ) : archived ? (
          <>
            <p className="chat-column pt-2 text-caption font-semibold tracking-wider text-soft uppercase" data-archived-count>
              {archived} archived
            </p>
            <ArchivedList work={tree.work} open={tree.open} pull={tree.pull} />
          </>
        ) : (
          <p className="px-5 pt-4 text-control text-soft">No archived workers.</p>
        )}
      </div>
    </>
  );
}

function WorkTree({
  work,
  tasks,
  main,
  open,
  pull,
  openChat,
}: {
  work: Work;
  tasks: Task[];
  /** The main chat at work, and waiting on the person. */
  main: { live: boolean; wait: boolean };
  open: (t: Thread) => void;
  /** Hands a worker's thread back to the main chat. */
  pull: (t: Thread) => void;
  /** Opens the chat of memory `id`, or the main chat for null. */
  openChat: (id: string | null) => void;
}) {
  const memories = useMemories() ?? [];
  const chats = useMemoryChats(memories.map((m) => m.id));
  const offers = useOffers();
  const talking = useTalkSeen(work.talk);
  const [folded, setFolded] = useState<Set<string>>(() => new Set(JSON.parse(localStorage.getItem(FOLDED) ?? "[]") as string[]));
  const fold = (key: string) => {
    const next = new Set(folded);
    if (!next.delete(key)) next.add(key);
    localStorage.setItem(FOLDED, JSON.stringify([...next]));
    setFolded(next);
  };

  const threads = work.threads.filter((t) => t.state !== "archived");
  const ids = new Set(threads.map((t) => t.id));
  const worker = (t: Thread): Node => {
    const task = tasks.find((k) => k.name === t.id && !k.done);
    const act = actOf(t, task);
    const tag = t.state === "waiting" && t.need ? NEED[t.need] : t.failed && act === "failed" ? "Failed" : stateOf(t);
    return {
      key: t.id,
      name: t.name,
      kind: "worker",
      act,
      model: shortModel(t, offers),
      mismatch: t.mismatch,
      title: t.title || firstLine(t.brief),
      // While it works, the summarizer's line; after, its report or its ask.
      status: [tag, t.summary ?? t.gist].filter(Boolean).join(" · "),
      at: t.at,
      kids: [...threads.filter((s) => s.head === t.id).map(worker), ...harnessOf(task, t.id)],
      open: () => open(t),
      thread: t,
    };
  };
  // A worker hangs from the memory it belongs to; one whose memory was
  // deleted, from Main, so it stays in reach.
  const named = new Set(memories.map((m) => m.id));
  const rootOf = (t: Thread) => (t.memory && named.has(t.memory) ? t.memory : null);
  const top = (memory: string | null) => threads.filter((t) => (!t.head || !ids.has(t.head)) && rootOf(t) === memory).map(worker);
  const seated: Node[] = tasks
    .filter((k) => !k.done && !ids.has(k.name))
    .map((k) => ({
      key: `main/${k.name}`,
      name: "Agent",
      status: k.title ?? firstLine(k.text),
      kind: "harness",
      act: "running",
      kids: [],
    }));
  const chatAct = (live: boolean, wait: boolean): Act => (wait ? "person" : live ? "thinking" : "idle");
  const roots: Node[] = [
    {
      key: "memory:main",
      name: "Main",
      kind: "memory",
      act: chatAct(main.live, main.wait),
      kids: [...top(null), ...seated],
      open: () => openChat(null),
    },
    ...memories.map((m): Node => {
      const c = chats[m.id];
      return {
        key: `memory:${m.id}`,
        name: m.name,
        kind: "memory",
        act: chatAct(!!c?.working, !!c?.asks.length),
        kids: top(m.id),
        open: () => openChat(m.id),
      };
    }),
  ];

  // Every node's parent, folded or not, to find the way a message goes.
  const parentOf = new Map<string, string>();
  const link = (list: Node[]) => list.forEach((n) => n.kids.forEach((k) => (parentOf.set(k.key, n.key), link([k]))));
  link(roots);

  // The rows in order, each with its depth, its parent's row and its top.
  type Row = {
    n: Node;
    d: number;
    i: number;
    y: number;
    h: number;
    parent?: Row;
  };
  const rows: Row[] = [];
  let y = 0;
  const walk = (list: Node[], d: number, parent?: Row) => {
    for (const n of list) {
      const h = n.kind === "worker" ? CARD : n.kind === "memory" ? ROOT : LEAF;
      const r: Row = { n, d, i: rows.length, y, h, parent };
      y += h;
      rows.push(r);
      if (!folded.has(n.key)) walk(n.kids, d + 1, r);
    }
  };
  walk(roots, 0);
  const shownRow = new Map(rows.map((r) => [r.n.key, r]));

  // The lanes a message runs along: up from its sender to where the two
  // branches meet, then down to whom it is for, each lane keyed by the row
  // it ends at and run the way the message goes. A lane folded away is
  // drawn by the fold, so it shows nothing.
  const flows = new Map<string, "up" | "down">();
  for (const { t } of talking.values()) {
    const chain = (k: string) => {
      const c = [k];
      for (let p = parentOf.get(k); p; p = parentOf.get(p)) c.push(p);
      return c;
    };
    const up = chain(t.from);
    const down = chain(t.to);
    const meet = up.find((k) => down.includes(k));
    if (!meet) continue;
    for (const k of up.slice(0, up.indexOf(meet))) if (shownRow.get(k)?.parent) flows.set(k, "up");
    for (const k of down.slice(0, down.indexOf(meet))) if (shownRow.get(k)?.parent) flows.set(k, "down");
  }

  // A memory's own workers sit on its line, as commits sit on their branch;
  // only a worker's sub-workers step in, one column a level. A row's words
  // start a column past its node, where the lane to its own sub-workers
  // runs down, so that lane never crosses them.
  const column = (r: Row) => Math.max(0, r.d - 1);
  const words = (r: Row) => X0 + (column(r) + 1) * DX + 12;
  const width = X0 + Math.max(0, ...rows.map(column)) * DX + 16;
  // A node sits level with its name, the first line of its card.
  const at = (r: Row) => ({
    x: X0 + column(r) * DX,
    y: r.y + (r.n.kind === "worker" ? 16 : r.h / 2),
  });
  const count = (n: Node): number => n.kids.reduce((c, k) => c + 1 + count(k), 0);

  return (
    <div data-surface="worker-tree">
      <div className="relative">
        <svg width={width} height={y} className="pointer-events-none absolute top-0 left-0" aria-hidden>
          <defs>
            <filter id="lane-blur" x="-50%" y="-50%" width="200%" height="200%">
              <feGaussianBlur stdDeviation="2.5" />
            </filter>
          </defs>
          {rows.map((r) => {
            if (!r.parent) return null;
            const { x, y } = at(r);
            const p = at(r.parent);
            // Out of the parent along a smooth fork to this one's column, then
            // down it; on the parent's own column, straight down.
            const d = `M${p.x},${p.y} C${x},${p.y} ${x},${p.y} ${x},${p.y + 14} L${x},${y}`;
            const flow = flows.get(r.n.key);
            const working = WORKING.includes(r.n.act);
            const colour = ACTS[r.n.act].colour;
            return (
              <g key={r.n.key} data-lane={r.n.key} data-flow={flow} data-glow={working || undefined}>
                <path d={d} stroke={working ? colour : "var(--color-edge)"} strokeWidth={2} fill="none" strokeLinecap="round" opacity={working ? 1 : 0.6} />
                {working && <path d={d} stroke={colour} strokeWidth={6} fill="none" filter="url(#lane-blur)" className="lane-glow" />}
                {flow && <path d={d} stroke="var(--color-ink)" strokeWidth={2.5} fill="none" strokeLinecap="round" className={flow === "down" ? "lane-flow" : "lane-flow lane-flow-up"} />}
              </g>
            );
          })}
          {rows.map((r) => {
            const { x, y } = at(r);
            const n = r.n;
            const colour = ACTS[n.act].colour;
            if (n.kind === "harness") return <circle key={n.key} cx={x} cy={y} r={4} fill="var(--color-paper)" stroke={colour} strokeWidth={1.75} />;
            const size = n.kind === "memory" ? 8 : 6;
            const lit = n.act !== "idle";
            // Working shows by a breathing glow; a wait or a failure holds still.
            const breathes = WORKING.includes(n.act);
            return (
              <g key={n.key}>
                {breathes && <circle cx={x} cy={y} r={size + 3} fill={colour} filter="url(#lane-blur)" className="lane-glow" />}
                <circle cx={x} cy={y} r={size} fill={lit ? colour : "var(--color-paper)"} stroke={colour} strokeWidth={n.kind === "memory" ? 2.5 : 2} opacity={lit ? 1 : 0.7} />
              </g>
            );
          })}
        </svg>
        <ul role="tree" aria-label="Workers by memory">
          {rows.map((r) => {
            const n = r.n;
            const parent = n.kids.length > 0;
            const shut = folded.has(n.key);
            const colour = ACTS[n.act].colour;
            const row = (
              <button
                type="button"
                // A leaf has nothing to fold, so a click opens it.
                onClick={() => (parent ? fold(n.key) : n.open?.())}
                onDoubleClick={() => n.open?.()}
                disabled={n.kind === "harness"}
                title={[`${n.name} · ${ACTS[n.act].label}`, n.model, n.title, n.status].filter(Boolean).join("\n")}
                style={{ height: r.h, paddingLeft: words(r) }}
                className={`flex w-full pr-3 text-left disabled:cursor-default ${n.kind === "worker" ? "items-start pt-1.5" : "items-center"} ${n.kind === "harness" ? "" : "hover:bg-hover"}`}
              >
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="flex min-w-0 items-baseline gap-2">
                    <span className={`shrink-0 truncate text-caption ${n.kind === "harness" ? "text-soft italic" : "font-semibold text-ink"}`}>{n.name}</span>
                    {/* What the row is, and what folds under it, as a
                            tree view's twistie and badge say. */}
                    {(n.kind === "memory" || parent) && (
                      <span className="flex min-w-0 shrink items-center gap-1 truncate text-caption text-soft" data-under>
                        {[n.kind === "memory" && "chat", parent && `${count(n)} ${n.kind === "memory" ? "worker" : "sub-worker"}${count(n) === 1 ? "" : "s"}`].filter(Boolean).join(" · ")}
                        {parent && (
                          <svg
                            viewBox="0 0 24 24"
                            className={`size-3 shrink-0 transition-transform ${shut ? "-rotate-90" : ""}`}
                            fill="none"
                            stroke="currentColor"
                            strokeWidth={2.4}
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            aria-hidden
                          >
                            <path d="m6 9 6 6 6-6" />
                          </svg>
                        )}
                      </span>
                    )}
                    {n.model && (
                      <span className="min-w-0 shrink truncate font-mono text-[11px] text-soft" data-model>
                        {n.model}
                      </span>
                    )}
                    {n.mismatch && (
                      <span
                        className="shrink-0 font-mono text-[11px]"
                        style={{ color: ACTS.failed.colour }}
                        title={`Model mismatch: ${n.mismatch}`}
                        data-mismatch
                      >
                        mismatch
                      </span>
                    )}
                  </span>
                  {n.title && n.kind === "worker" && (
                    <span className="truncate text-caption text-ink" data-title>
                      {n.title}
                    </span>
                  )}
                  {n.status && (
                    <span className="truncate text-caption text-soft" data-status>
                      {n.kind === "worker" && <span className="mr-1.5 inline-block size-1.5 rounded-full align-middle" style={{ background: colour }} />}
                      {n.status}
                    </span>
                  )}
                </span>
                {n.at !== undefined && <span className="shrink-0 pl-3 text-caption text-soft tabular-nums">{ago(n.at)}</span>}
              </button>
            );
            return (
              <li key={n.key} role="treeitem" aria-level={r.d + 1} aria-expanded={parent ? !shut : undefined} data-row={n.key} data-act={n.act}>
                {/* A worker's hold or right click opens its menu, as its card's did. */}
                {n.thread ? (
                  <Held whole menu={workerMenu(n.thread, work, pull)}>
                    {row}
                  </Held>
                ) : (
                  row
                )}
              </li>
            );
          })}
        </ul>
      </div>
    </div>
  );
}
