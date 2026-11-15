// The agents running, at the composer: on its outline at the top right sit
// opaque overlapping circles, one per agent at work, each with its model's
// mark. They are the workers of the chat the composer belongs to and
// the harness's own subagents in its turn (Claude Code's Agent calls, omp's
// task). A tap sends them up out of the row into a column above the
// composer, each named beside its dot; a row opens that agent in a tile of
// its own (a tab of its own on a phone). Nothing at work, no circles.
import { createContext, useContext, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { firstLine, type Task } from "./chat";
import { Logo, logoOf } from "./marks";
import { useOffers } from "./modelNames";
import { ago, asks, type Thread } from "./work";
import { ACTS, actOf, isAgent, shortModel, type Act } from "./workTree";

/** What the circles are drawn from, which App holds: the workers, the main
 * chat's turn's calls, and the ways to open each. */
export type Running = {
  threads: Thread[];
  /** The main chat's calls and workers' streams, as useChat hears them. */
  tasks: Task[];
  /** The main chat's harness and model, which its own subagents run on. */
  main?: { harness: string; model: string };
  /** Opens a worker's thread in a tile, or a phone tab, of its own. */
  open: (t: Thread) => void;
  /** Opens one of the main chat's subagents, by its call. */
  openAgent?: (call: string) => void;
  /** Every worker, asleep and archived too, with the watches: what the
   * Workers place held before the composer took its work. */
  all?: () => ReactNode;
  /** Starts a worker on the person's own words (the Tasks sheet). */
  start?: () => void;
};

/** Opens the main chat's composer tree on every worker, wherever asked from
 * (the + menu, the launcher, Today, a #work link). */
export const SHOW_ALL = "libri:agents";
export const showAllAgents = () => dispatchEvent(new Event(SHOW_ALL));
export const RunningCtx = createContext<Running | null>(null);

/** Whose agents a composer shows: all of them in the main chat; in a memory's
 * chat, that memory's; in a worker's thread, its own children. */
export type AgentScope = { main: true } | { memory: string } | { thread: string };

type Agent = {
  key: string;
  name: string;
  kind: "worker" | "subagent";
  act: Act;
  logo: string | null;
  model?: string;
  status?: string;
  at?: number;
  kids: Agent[];
  open?: () => void;
};

/** A circle's width, and how far each next one steps left over it. */
const CIRCLE = 22;
const STEP = 15;

/** A worker counts as running while it works or asks the person for an
 * approval or an answer. One done (its report waiting to be read), napping,
 * asleep or archived leaves the circles and the column. */
const running = (t: Thread) => t.state === "working" || asks(t);

function useAgents(scope: AgentScope): Agent[] {
  const ctx = useContext(RunningCtx);
  const offers = useOffers();
  if (!ctx) return [];
  const { threads, tasks, main, open, openAgent } = ctx;
  const byId = new Map(threads.map((t) => [t.id, t]));
  // A running worker hangs from its nearest running head, so one asked for
  // by a worker now asleep still shows, at the top.
  const headOf = (t: Thread): string | undefined => {
    for (let h = t.head; h; h = byId.get(h)?.head) {
      const head = byId.get(h);
      if (!head) return undefined;
      if (running(head)) return h;
    }
    return undefined;
  };
  // A worker's own harness subagents, while their calls are open.
  const subagents = (task: Task | undefined, key: string, logo: string | null, openParent: () => void): Agent[] =>
    (task?.tools ?? [])
      .filter((c) => isAgent(c.name) && c.result === undefined)
      .map((c) => {
        const input = (c.input ?? {}) as { description?: string; subagent_type?: string };
        return { key: `${key}/${c.id}`, name: input.subagent_type ?? "Agent", kind: "subagent", act: "running", logo, status: input.description, kids: [], open: openParent };
      });
  const live = threads.filter(running);
  const worker = (t: Thread): Agent => {
    const task = tasks.find((k) => k.name === t.id && !k.done);
    const logo = t.pick ? logoOf(t.pick) : null;
    return {
      key: t.id,
      name: t.name,
      kind: "worker",
      act: actOf(t, task),
      logo,
      model: shortModel(t, offers),
      status: t.summary ?? t.title ?? firstLine(t.brief),
      at: t.at,
      kids: [...live.filter((s) => headOf(s) === t.id).map(worker), ...subagents(task, t.id, logo, () => open(t))],
      open: () => open(t),
    };
  };
  if ("thread" in scope) {
    const t = byId.get(scope.thread);
    if (!t) return [];
    const task = tasks.find((k) => k.name === t.id && !k.done);
    // Its sub-workers, wherever their heads stand, and its own subagents.
    const own = live.filter((s) => s.head === t.id || headOf(s) === t.id);
    return [...own.filter((s) => !own.some((o) => o.id === headOf(s))).map(worker), ...subagents(task, t.id, t.pick ? logoOf(t.pick) : null, () => open(t))];
  }
  const tops = live.filter((t) => !headOf(t)).filter((t) => ("memory" in scope ? t.memory === scope.memory : true));
  const workers = tops.map(worker);
  if ("memory" in scope) return workers;
  // The main chat's own subagents: its calls that are no worker.
  const ids = new Set(threads.map((t) => t.id));
  const logo = main ? logoOf(main) : null;
  const seated: Agent[] = tasks
    .filter((k) => !k.done && !ids.has(k.name))
    .map((k) => ({ key: `main/${k.name}`, name: "Agent", kind: "subagent", act: "running", logo, status: k.title ?? firstLine(k.text), kids: [], open: openAgent && (() => openAgent(k.name)) }));
  return [...workers, ...seated];
}

const flat = (list: Agent[]): Agent[] => list.flatMap((a) => [a, ...flat(a.kids)]);

/** The ring a circle wears for what its agent is doing: the person's amber
 * for a wait on them, red for a failure, none while it simply works. */
const ringOf = (act: Act) => (act === "person" || act === "failed" ? ACTS[act].colour : undefined);

function Circle({ a }: { a: Agent }) {
  const ring = ringOf(a.act);
  return (
    <span
      data-agent-circle={a.key}
      data-act={a.act}
      // A ringed one draws over its neighbour, so its ring shows whole.
      className={`grid size-[22px] shrink-0 place-items-center rounded-full border-2 border-lift bg-hover text-mid not-first:-ml-[7px] ${ring ? "relative z-10" : ""}`}
      style={ring ? { boxShadow: `0 0 0 1.5px ${ring}` } : undefined}
    >
      <Logo name={a.logo} size="size-3" />
    </span>
  );
}

/** The circles in the composer's outline, and the tree they open. The row
 * grows leftward, one circle per agent, as far as the composer allows: it
 * stops as far from the left end as it sits from the right, and only then
 * does its last circle become "+N". Renders nothing while no agent runs and
 * the tree is shut. */
export function ComposerAgents({ scope }: { scope: AgentScope }) {
  const agents = useAgents(scope);
  const ctx = useContext(RunningCtx);
  const all = flat(agents);
  // "running": the tree of what runs; "all": every worker and the watches.
  const [open, setShown] = useState<"running" | "all" | null>(null);
  // The column folding: its dots travel back to the row, then it goes
  // (`home`), unless tapped open again on the way.
  const [leaving, setLeaving] = useState(false);
  const folding = useRef(false);
  const setOpen = (next: "running" | "all" | null) => {
    folding.current = next === null && open === "running" && !still();
    setLeaving(folding.current);
    if (!folding.current) setShown(next);
  };
  const home = () => {
    if (!folding.current) return;
    folding.current = false;
    setLeaving(false);
    setShown(null);
  };
  const box = useRef<HTMLDivElement>(null);
  const main = "main" in scope;
  // Asked from elsewhere, the main chat's tree opens on every worker.
  useEffect(() => {
    if (!main) return;
    const show = () => setOpen("all");
    addEventListener(SHOW_ALL, show);
    return () => removeEventListener(SHOW_ALL, show);
  }, [main]);
  // A tap anywhere else, or Escape, folds the tree; not one on a menu or
  // sheet a row opened, which lies outside it.
  useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => {
      const at = e.target as Element;
      if (!box.current?.contains(at) && !at.closest?.("[role=menu], [role=dialog]:not([data-agents-tree])")) setOpen(null);
    };
    const key = (e: KeyboardEvent) => e.key === "Escape" && setOpen(null);
    addEventListener("pointerdown", away);
    addEventListener("keydown", key);
    return () => {
      removeEventListener("pointerdown", away);
      removeEventListener("keydown", key);
    };
  }, [open]);
  useEffect(() => {
    if (!all.length && open === "running") setOpen(null);
  }, [all.length, open]);
  // How many circles the composer has room for, kept as it resizes.
  const [room, setRoom] = useState(0);
  useLayoutEffect(() => {
    const root = box.current?.parentElement;
    const at = box.current;
    if (!root || !at) return;
    const measure = () => {
      const inset = root.getBoundingClientRect().right - at.getBoundingClientRect().right;
      setRoom(Math.max(1, Math.floor((root.getBoundingClientRect().width - 2 * inset - CIRCLE) / STEP) + 1));
    };
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(root);
    return () => watch.disconnect();
  }, [all.length > 0]);
  const fits = all.length <= room;
  const slots = fits ? all.length : Math.max(1, room - 1);
  if (!all.length && !open) return null;
  // Those waiting on the person come first, so a "+N" never hides one.
  const first = (a: Agent) => (a.act === "person" ? 0 : a.act === "failed" ? 1 : 2);
  // On the bar they stand in the column's order reversed, so the one
  // rising highest leads the train along the rail and none cross.
  const shown = [...all].sort((a, b) => first(a) - first(b)).slice(0, slots).sort((a, b) => all.indexOf(b) - all.indexOf(a));
  const more = all.length - shown.length;
  const waiting = all.filter((a) => a.act === "person").length;
  const label = `${all.length} agent${all.length === 1 ? "" : "s"} running${waiting ? `, ${waiting} waiting on you` : ""}`;
  const close = () => setOpen(null);
  // A row or "Start a worker" chosen: what it opens comes at once and the
  // column goes with it, its dots not travelling back first (as Android's
  // does). The fold timed with the chat opening ran past 400 ms.
  const pick = () => {
    folding.current = false;
    setLeaving(false);
    setShown(null);
  };
  return (
    // Centred on the outline, clear of its rounded corner.
    <div ref={box} className="absolute -top-[11px] right-8 max-sm:right-7" data-agents>
      {open === "running" && (
        <AgentTree agents={agents} close={close} pick={pick} leaving={leaving} home={home} all={main && ctx?.all ? () => setOpen("all") : undefined} start={main ? ctx?.start : undefined} />
      )}
      {open === "all" && ctx?.all && (
        <div
          role="dialog"
          aria-label="Every worker"
          className="floating absolute -right-8 bottom-full z-(--layer-pop) mb-2 flex max-h-[min(70vh,640px)] w-[min(480px,calc(100vw-28px))] flex-col bg-lift! max-sm:-right-7 rounded-2xl pt-2"
          data-agents-tree
          data-agents-all
        >
          <div className="min-h-0 flex-1 overflow-y-auto">{ctx.all()}</div>
          <Foot close={close} start={ctx.start} back={all.length ? () => setOpen("running") : undefined} />
        </div>
      )}
      {all.length > 0 && (
        <button
          type="button"
          aria-expanded={!!open}
          aria-label={label}
          title={label}
          onClick={() => setOpen(open && !leaving ? null : "running")}
          // Out in the column, these circles are the dots there: hidden
          // here until they are home.
          className={`relative z-(--layer-edge) flex h-[22px] items-center rounded-full ${open === "running" ? "[&>*]:opacity-0" : ""}`}
          data-agents-toggle
        >
          {shown.map((a) => (
            <Circle key={a.key} a={a} />
          ))}
          {more > 0 && (
            <span className="-ml-[7px] grid h-[22px] min-w-[22px] place-items-center rounded-full border-2 border-lift bg-hover px-1 text-[10px] font-semibold text-mid tabular-nums" data-agents-more>
              +{more}
            </span>
          )}
        </button>
      )}
    </div>
  );
}

/** The tree's foot: every worker (asleep and archived too, and the
 * watches), back to what runs, and starting a worker. At the column's
 * `end` its words end where the rows' do, clear of the dots' rail. */
function Foot({ close, all, back, start, end }: { close: () => void; all?: () => void; back?: () => void; start?: () => void; end?: boolean }) {
  const link = "h-8 rounded-full px-3 text-caption text-mid hover:bg-hover hover:text-ink";
  if (!all && !back && !start) return null;
  return (
    <div className={`flex items-center gap-1 px-2 py-1.5 ${end ? "justify-end pr-[26px]" : ""}`} data-agents-foot>
      {all && (
        <button type="button" className={link} onClick={all} data-agents-show-all>
          Show all
        </button>
      )}
      {back && (
        <button type="button" className={link} onClick={back} data-agents-running>
          Running
        </button>
      )}
      {!end && <span className="flex-1" />}
      {start && (
        <button
          type="button"
          className={link}
          onClick={() => {
            close();
            start();
          }}
          data-agents-start
        >
          Start a worker
        </button>
      )}
    </div>
  );
}

/** The longest dot's travel between the row and the column: quick enough
 * to feel immediate, and well inside the 400 ms every interaction is held
 * to. The others take time in step with their shorter way, so all keep one
 * speed; the least keeps the same share of it. */
const TRAVEL = 200;
const TRAVEL_LEAST = 86;
/** The rail's corner, from the bar up into the column: it fits between the
 * bar's last circle and the column's dots (21 px apart). */
const BEND = 20;
/** The share of a dot's time it spends gathering speed, and as much
 * slowing at the end; between, its speed holds. */
const RAMP = 0.22;
const still = () => matchMedia("(prefers-reduced-motion: reduce)").matches;

/** The agents risen out of the composer as a column on a floating panel:
 * each circle leaves the bar along one rail, right and round a corner up
 * to its slot, one under another above the composer's edge, its name, model and state to its left
 * and the talk behind dimmed; a sub-worker stands in under its head. Tapped
 * away, or the circles tapped again, they travel back (`leaving`) and it
 * goes once they are `home`. */
function AgentTree({ agents, close, pick, all, start, leaving, home }: { agents: Agent[]; close: () => void; pick: () => void; all?: () => void; start?: () => void; leaving: boolean; home: () => void }) {
  const rows: { a: Agent; d: number }[] = [];
  const walk = (list: Agent[], d: number) => list.forEach((a) => (rows.push({ a, d }), walk(a.kids, d + 1)));
  walk(agents, 0);
  const list = useRef<HTMLUListElement>(null);
  const [out, setOut] = useState(false);
  // The bar's own circles are the ones that rise: while the column is
  // out they hide on the bar, and each dot here starts exactly on its
  // circle (the "+N" for one under it). It runs one rail: right along the
  // bar, round a quarter circle and up into its row. Folding runs it back.
  const way = (dot: HTMLElement) => {
    const key = dot.dataset.dot!;
    const bar = [...document.querySelectorAll<HTMLElement>("[data-agents-toggle] > *")];
    const circle = bar.find((c) => c.dataset.agentCircle === key) ?? bar.find((c) => "agentsMore" in c.dataset);
    if (!circle) return null;
    const c = circle.getBoundingClientRect();
    const d = dot.getBoundingClientRect();
    const end = { x: d.left + d.width / 2, y: d.top + d.height / 2 };
    return { rail: rail({ x: c.left + c.width / 2, y: c.top + c.height / 2 }, end), end, scale: c.width / d.width };
  };
  // All set off together and keep one speed, so they leave the bar as a
  // train, each turning off where its row is. Folding is the same run
  // played backwards, the last to leave the first home.
  const flight = (back: boolean) => {
    const runs = dots().map((dot) => ({ dot, w: way(dot) }));
    const moves: Animation[] = [];
    const longest = Math.max(1, ...runs.map(({ w }) => w?.rail.length ?? 0));
    runs.forEach(({ dot, w }) => {
      const direction = back ? "reverse" : "normal";
      if (!w) return moves.push(dot.animate([{ transform: "scale(0.6)", opacity: 0 }, { transform: "none", opacity: 1 }], { duration: TRAVEL, fill: "both", direction }));
      const duration = Math.max(TRAVEL_LEAST, (TRAVEL * w.rail.length) / longest);
      // Sampled evenly in time, its speed set by `along`, so the browser
      // only joins near points with straight lines and the corner stays round.
      const steps = Math.max(24, Math.ceil(duration / 6));
      const frames = Array.from({ length: steps + 1 }, (_, k) => {
        const s = along(k / steps);
        const p = w.rail.at(s * w.rail.length);
        return { transform: `translate(${p.x - w.end.x}px, ${p.y - w.end.y}px) scale(${w.scale + (1 - w.scale) * s})` };
      });
      moves.push(dot.animate(frames, { duration, delay: back ? TRAVEL - duration : 0, fill: "both", direction }));
    });
    return moves;
  };
  const moving = useRef<Animation[]>([]);
  const dots = () => [...(list.current?.querySelectorAll<HTMLElement>("[data-dot]") ?? [])];
  // A scrolling list clips what passes outside it, and the dots run there
  // on their way from the bar: it scrolls only once its rows outgrow the
  // panel (then dots for rows out of sight are cut on the way, as the rows are).
  // Measured by its rows, as a dot out on the rail would count as overflow.
  useLayoutEffect(() => {
    const ul = list.current;
    const first = ul?.firstElementChild;
    const last = ul?.lastElementChild;
    if (!ul || !first || !last) return;
    ul.style.overflowY = "";
    const rows = last.getBoundingClientRect().bottom - first.getBoundingClientRect().top;
    const pad = parseFloat(getComputedStyle(ul).paddingTop) + parseFloat(getComputedStyle(ul).paddingBottom);
    if (rows + pad <= ul.clientHeight + 0.5) ul.style.overflowY = "visible";
  });
  useLayoutEffect(() => {
    if (!still()) moving.current = flight(false);
    setOut(true);
  }, []);
  // Folding ends when the last dot is home, not after a set time, so a
  // slowed clock (a recording, the browser's animation tools) slows it too.
  // Tapped open on the way, each turns back where it is.
  const folded = useRef(false);
  useLayoutEffect(() => {
    if (still()) return;
    if (leaving) {
      folded.current = true;
      setOut(false);
      moving.current = flight(true);
      Promise.all(moving.current.map((m) => m.finished)).then(home, () => {});
    } else if (folded.current) {
      folded.current = false;
      setOut(true);
      moving.current.forEach((m) => m.reverse());
    }
  }, [leaving]);
  const fade = `transition-[opacity,translate] duration-150 ease-out motion-reduce:transition-none ${out ? "opacity-100" : "translate-x-2 opacity-0"}`;
  return (
    <>
      {/* The talk behind, dimmed a little; a tap on it folds the column. */}
      <div aria-hidden onPointerDown={close} className={`fixed inset-0 bg-paper/70 transition-opacity duration-200 motion-reduce:transition-none ${out ? "opacity-100" : "opacity-0"}`} data-agents-scrim />
      <div
        // To the composer's right end, as "Every worker" is: its dots stand
        // right of the bar's last circle, room for the rail's corner.
        role="dialog"
        aria-label="Agents running"
        className="absolute -right-8 bottom-full z-(--layer-pop) mb-2 flex max-h-[min(60vh,520px)] max-sm:-right-7 w-[min(420px,calc(100vw-60px))] flex-col items-end px-2 pt-2"
        data-agents-tree
      >
        {/* The surface every floating panel has ("Every worker" too): with
            the talk showing through, the rows were hard to read. It fades
            in under the dots as they rise, so they still leave the row. */}
        <div aria-hidden className={`floating absolute inset-0 -z-10 bg-lift! rounded-2xl transition-opacity duration-150 ease-out motion-reduce:transition-none ${out ? "opacity-100" : "opacity-0"}`} data-agents-panel />
        {/* Over the foot, which the dots pass on their way up. */}
        <ul ref={list} role="tree" aria-label="Agents running" className="relative z-1 flex min-h-0 w-full flex-col items-end gap-1 overflow-y-auto overscroll-contain py-1 [scrollbar-width:none]">
          {rows.map(({ a, d }) => {
            const ring = ringOf(a.act);
            return (
              <li key={a.key} role="treeitem" aria-level={d + 1} data-agent-row={a.key} data-kind={a.kind} className="flex w-full justify-end">
                <button
                  type="button"
                  disabled={!a.open}
                  onClick={() => {
                    a.open?.();
                    pick();
                  }}
                  title={[`${a.name} · ${ACTS[a.act].label}`, a.model, a.status].filter(Boolean).join("\n")}
                  // A sub-worker stands in from its head by a dot's width.
                  style={{ marginRight: d * 18 }}
                  className="group flex min-w-0 max-w-full items-center gap-2.5 rounded-full py-0.5 pl-3 text-right disabled:cursor-default"
                >
                  <span className={`flex min-w-0 flex-col items-end leading-tight ${fade}`}>
                    <span className="flex min-w-0 items-baseline gap-2">
                      {a.kind === "subagent" ? <span className="shrink-0 text-[11px] text-soft">subagent</span> : a.model && <span className="min-w-0 shrink truncate font-mono text-[11px] text-soft">{a.model}</span>}
                      <span className={`shrink-0 truncate text-caption group-enabled:group-hover:underline ${a.kind === "subagent" ? "text-mid italic" : "font-semibold text-ink"}`}>{a.name}</span>
                    </span>
                    <span className="flex min-w-0 max-w-full items-center gap-1.5 text-caption text-soft">
                      {/* The time last, so a row's words read the same as it moves on. */}
                      {a.status && <span className="min-w-0 truncate">{a.status}</span>}
                      {a.at !== undefined && <span className="shrink-0 tabular-nums">{ago(a.at)}</span>}
                      <span className="inline-block size-1.5 shrink-0 rounded-full" style={{ background: ACTS[a.act].colour }} title={ACTS[a.act].label} />
                    </span>
                  </span>
                  <span
                    data-dot={a.key}
                    className="grid size-7 shrink-0 place-items-center rounded-full border-2 border-lift bg-hover text-mid shadow-lift will-change-transform group-enabled:group-hover:text-ink"
                    style={ring ? { boxShadow: `0 0 0 1.5px ${ring}` } : undefined}
                  >
                    <Logo name={a.logo} size="size-3.5" />
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
        <div className={`w-full ${fade}`}>
          <Foot close={pick} all={all} start={start} end />
        </div>
      </div>
    </>
  );
}

type Point = { x: number; y: number };

/** The rail from a bar circle to a dot's slot: along the bar, a quarter
 * circle of `BEND` (less where the way is shorter) and up. `at(s)` is the
 * point `s` px along it. */
function rail(from: Point, to: Point) {
  const h = Math.sign(to.x - from.x);
  const r = Math.min(BEND, Math.abs(to.x - from.x), Math.max(0, from.y - to.y));
  const run = Math.abs(to.x - from.x) - r;
  const turn = (Math.PI / 2) * r;
  const rise = Math.max(0, from.y - to.y) - r;
  const corner = { x: to.x - h * r, y: from.y - r };
  const at = (s: number): Point => {
    if (s <= run) return { x: from.x + h * s, y: from.y };
    if (s <= run + turn) {
      const a = r ? (s - run) / r : 0;
      return { x: corner.x + h * r * Math.sin(a), y: corner.y + r * Math.cos(a) };
    }
    return { x: to.x, y: Math.max(to.y, from.y - r - (s - run - turn)) };
  };
  return { length: run + turn + rise, at };
}

/** How far along its rail a dot is at time `t` (both 0 to 1): speed rises
 * evenly for `RAMP`, holds, and falls as evenly to rest. */
function along(t: number) {
  const top = 1 / (1 - RAMP);
  if (t < RAMP) return (top * t * t) / (2 * RAMP);
  if (t > 1 - RAMP) return 1 - (top * (1 - t) ** 2) / (2 * RAMP);
  return top * (t - RAMP / 2);
}
