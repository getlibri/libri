// The page's three places, as Duet lays them out on a bar at the bottom:
// Chat, the one long conversation; Work, the threads promoted out of it,
// each a subagent with a context of its own; and Temp, a scratchpad nothing
// remembers. Work is Libri's subagents (task.rs): a thread
// is promoted by asking the chat, which writes its brief and spawns it.
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { firstLine, upload, type Task } from "./chat";
import { api } from "./api";
import { Held, HoldMenu, type Grip, type MenuItem } from "./hold";
import { localInput, napPresets } from "./nap";
import { useScratchpad } from "./scratchpad";
import { WatchRow, type Watches } from "./library";


/** The pin, for the places that show Pinned. */
export const PinMark = () => (
  <>
    <path d="M12 17v5" />
    <path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z" />
  </>
);

/** One thread on the Work page, as /api/work lists it (task::Thread). */
export type Thread = {
  /** What every call about it takes; the name comes back round, this doesn't. */
  id: string;
  name: string;
  brief: string;
  /** A cheap model's few words on the work; the brief stands in until then. */
  title?: string;
  state: "working" | "waiting" | "napping" | "sleeping" | "archived";
  /** What it is doing now, sent only while it is working. */
  summary?: string;
  gist: string;
  at: number;
  report?: string;
  /** Whether the person has read the log as it stands, on any device. */
  read: boolean;
  /** Who it is paused on: its job isn't over, but no run is going. */
  waiting_on?: string[];
  /** What a Waiting card waits for; for approval and answer `gist` is the
   * worker's ask. Absent from a host from before it. */
  need?: "review" | "approval" | "answer";
  /** For a sub-worker, the ID of the worker that asked for it, named before
   * its title on the card and before its name on its own page. */
  head?: string;
  /** The named memory it belongs to, whose chat started it and has its
   * report; absent, the main one. */
  memory?: string;
  /** When its nap ends, in seconds since the epoch, while it has one. One at
   * work keeps it for when its run ends. */
  nap?: number;
  /** The branches an archived worker left with work not on main, kept for
   * the person to land or drop. */
  unlanded?: string[];
  /** It has been merged into the main chat and done nothing since. */
  merged?: boolean;
  /** The harness and model its next run goes on; an empty model is the
   * harness's own default. Absent from a host from before it. */
  pick?: { harness: string; model: string };
  /** The thinking level its next run is held to; absent, the harness's
   * default. */
  effort?: string;
  /** What its harness last reported a run is on (task::Ran). */
  ran?: { model: string; effort?: string };
  /** How that report differs from what it was started on, if it does. */
  mismatch?: string;
  /** Its last run failed, and none has run since. */
  failed?: boolean;
  /** The tool it has a call open to, while it works. */
  calling?: string;
};

/** A message lately gone between agents (task::Talk): each end a worker's
 * ID, or `memory:<id>` for a memory's chat, as the Workers tree keys its rows. */
export type Talk = { at: number; from: string; to: string };

export type Shell = {
  /** Opens or closes settings. */
  settings: () => void;
  /** Whether a newer build of the app is waiting. */
  update: boolean;
  /** Asks the chat to move this part of it to Work, citing `from`. */
  promote: (directions: string, from?: { id: string; text: string }) => void;
};
export const ShellCtx = createContext<Shell | null>(null);
export const useShell = () => useContext(ShellCtx);

/** The words that ask the chat to promote; Libri's prompt (AGENTS in
 * main.rs) knows them. */
export function promotion(
  directions: string,
  from?: { id: string; text: string },
) {
  const said = directions.trim() || "carry out what we just settled";
  if (!from) return `Create task: ${said}`;
  const hippo = /^h-(\d+)$/.exec(from.id)?.[1];
  const quote = from.text
    .trim()
    .split("\n")
    .slice(0, 4)
    .join("\n")
    .slice(0, 400);
  return `Create task: ${said}\n\n${hippo ? `From message ${hippo}:` : "From this:"}\n> ${quote.replaceAll("\n", "\n> ")}`;
}

/** The Work list, asked again while anything works or the list is in view,
 * and whenever the chat hears an agent start or end or sends one word. A Waiting thread
 * needs the person until they open it or mark it read. */
export function useWork(tasks: Task[], looking: boolean, touched: number) {
  const [threads, setThreads] = useState<Thread[]>([]);
  const [talk, setTalk] = useState<Talk[]>([]);
  // The list as last read: most asks bring it back unchanged, and a new array
  // would draw every card again for nothing.
  const last = useRef("");
  const ask = useCallback(async () => {
    const r = await api("api/work").catch(() => undefined);
    if (!r?.ok) return;
    const got = (await r.json()) as { threads: Thread[]; talk?: Talk[] };
    // The talk comes and goes by itself; the threads are drawn again only
    // when they change.
    setTalk((was) => (got.talk?.length || was.length ? (got.talk ?? []) : was));
    const text = JSON.stringify(got.threads);
    if (text === last.current) return;
    last.current = text;
    setThreads(got.threads);
  }, []);
  const at = tasks.map((t) => `${t.name}:${t.done ? 1 : 0}`).join();
  useEffect(() => void ask(), [ask, at, touched]);
  const busy = threads.some((t) => t.state === "working");
  useEffect(() => {
    if (!busy && !looking) return;
    const id = window.setInterval(
      () => document.visibilityState === "visible" && void ask(),
      busy ? 4000 : 15000,
    );
    return () => window.clearInterval(id);
  }, [ask, busy, looking]);
  // Reading a thread settles a card for review; one that asked for approval
  // or an answer waits until the person gives it, or clears it from the menu.
  const settle = useCallback(
    async (how: "read" | "clear", ts: Thread[]) => {
      const ids = new Set(ts.map((t) => t.id));
      const settled = (t: Thread) => how === "clear" || !asks(t);
      // The list now differs from the host's, so its next answer is taken whole.
      last.current = "";
      setThreads((all) =>
        all.map((t) =>
          ids.has(t.id) && settled(t)
            ? {
                ...t,
                read: true,
                need: undefined,
                state: t.state === "waiting" ? "sleeping" : t.state,
              }
            : t,
        ),
      );
      await Promise.all(
        ts.map((t) =>
          api(`api/work/${encodeURIComponent(t.id)}/${how}`, {
            method: "POST",
          }).catch(() => undefined),
        ),
      );
    },
    [],
  );
  const mark = useCallback((...ts: Thread[]) => settle("read", ts), [settle]);
  const clear = useCallback((...ts: Thread[]) => settle("clear", ts), [settle]);
  const act = useCallback(
    async (id: string, what: "archive" | "restore" | "wake") => {
      await api(`api/work/${encodeURIComponent(id)}/${what}`, {
        method: "POST",
      }).catch(() => undefined);
      await ask();
    },
    [ask],
  );
  /** Starts a worker on the person's own words and files, not through the
   * main chat, on `choice` as the Tasks sheet's pickers left it (else the
   * Workers default); the host answers its name, or why not. */
  const start = useCallback(
    async (brief: string, files: File[] = [], choice?: { pick: { harness: string; model: string }; effort: string; access: string; sandbox: string | null }) => {
      const ids: string[] = [];
      for (const f of files) {
        const up = await upload(f);
        if (!up?.ok) return { error: "A file didn't reach the host." };
        ids.push(((await up.json()) as { id: string }).id);
      }
      // Empty is the harness's own default, so it is left out.
      const picked = choice && {
        pick: choice.pick,
        effort: choice.effort,
        ...(choice.access ? { access: choice.access } : {}),
        ...(choice.sandbox ? { sandbox: choice.sandbox } : {}),
      };
      const r = await api("api/work", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ brief, files: ids, ...picked }),
      }).catch(() => undefined);
      await ask();
      if (!r?.ok) return { error: (await r?.text().catch(() => "")) || "The worker did not start." };
      return { name: ((await r.json()) as { name: string }).name };
    },
    [ask],
  );
  /** Puts a thread down for a nap until `until`, in seconds since the epoch. */
  const nap = useCallback(
    async (id: string, until: number) => {
      await api(`api/work/${encodeURIComponent(id)}/nap`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ until }),
      }).catch(() => undefined);
      await ask();
    },
    [ask],
  );
  // A nap ends on the host's tick; the list asks again as the first one due
  // passes, as T3's sidebar keeps a clock that moves when a snooze wakes.
  const due = threads.reduce(
    (soonest, t) => (t.state === "napping" && t.nap && t.nap < soonest ? t.nap : soonest),
    Infinity,
  );
  useEffect(() => {
    if (due === Infinity) return;
    const id = window.setTimeout(
      () => void ask(),
      Math.min(2 ** 31 - 1, Math.max(0, due * 1000 - Date.now()) + 1500),
    );
    return () => window.clearTimeout(id);
  }, [ask, due]);
  return {
    threads,
    talk,
    mark,
    clear,
    act,
    nap,
    start,
    working: threads.filter((t) => t.state === "working").length,
    running: threads.filter(
      (t) => t.state === "working" && !t.waiting_on?.length,
    ).length,
    waiting: threads.filter((t) => t.state === "waiting").length,
  };
}

export type Work = ReturnType<typeof useWork>;

export const STATE: Record<Thread["state"], string> = {
  working: "Working",
  waiting: "Waiting",
  napping: "Napping",
  sleeping: "Sleeping",
  archived: "Archived",
};

/** A Waiting card's tag, what it waits for (as
 * T3 Code's settle inbox and Linear's Inbox label each row by its reason). */
/** An archived card's tag while its branches hold work not on main. */
export const UNLANDED = "Unlanded work";

export const NEED: Record<NonNullable<Thread["need"]>, string> = {
  review: "For review",
  approval: "For approval",
  answer: "For an answer",
};

/** Whether a Waiting card asks the person to do more than read it. */
export const asks = (t: Thread) =>
  t.state === "waiting" && (t.need === "approval" || t.need === "answer");

/** Who a paused thread waits on: the one part of its state no section says. */
const waitingOn = (t: Thread) =>
  t.waiting_on?.length ? `Waiting on ${t.waiting_on.join(", ")}` : undefined;

/** The name of the worker that asked for sub-worker `t`, if it is listed. */
export const parentOf = (t: Thread | undefined, threads: Thread[]) =>
  t?.head ? threads.find((h) => h.id === t.head)?.name : undefined;

/** A thread's state in words, naming who a paused one waits on. */
export const stateOf = (t: Thread) => waitingOn(t) ?? STATE[t.state];

export function ago(secs: number) {
  const s = Math.max(0, Date.now() / 1000 - secs);
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  if (s < 86400 * 7) return `${Math.floor(s / 86400)}d`;
  return new Date(secs * 1000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

/** The Workers tile's Watches tab: the watches and reminders. */
export function WatchList({ watches }: { watches: Watches }) {
  return (
    <ul data-surface="workers" className="chat-column flex flex-col gap-2 pt-2 pb-10">
      {watches.watches.map((w) => (
        <WatchRow key={w.id} w={w} watches={watches} />
      ))}
    </ul>
  );
}

/** The Workers tile's Archived tab: the workers put away, newest first as
 * the server lists them. */
export function ArchivedList({ work, open, pull }: { work: Work; open: (t: Thread) => void; pull: (t: Thread) => void }) {
  const put = work.threads.filter((t) => t.state === "archived");
  const row = (t: Thread) => {
    const parent = parentOf(t, work.threads);
    const heading = t.title ?? firstLine(t.brief);
    const tag = t.unlanded?.length ? UNLANDED : undefined;
    return (
      <li key={t.id}>
        <Held
          whole
          menu={workerMenu(t, work, pull)}
        >
          {/* Three rows, none of which ever wraps; a mouse resting on the
              card gets their words whole from the browser. */}
          <article className="rounded-xl border border-hair bg-lift px-3 py-2 shadow-lift transition-colors hover:border-frame">
            <button
              type="button"
              onClick={() => open(t)}
              title={[[t.name, tag].filter(Boolean).join(" · "), parent ? `${parent} › ${heading}` : heading, t.gist].join("\n")}
              className="block w-full text-left"
            >
              <div className="flex items-center gap-1.5 text-caption leading-[15.8125px]">
                <span role="img" aria-label={STATE[t.state]} className="grid size-2.5 shrink-0 place-items-center">
                  <span className="size-2 rounded-full bg-frame" />
                </span>
                <span className="min-w-0 truncate font-semibold text-ink">{t.name}</span>
                <span className="min-w-0 flex-1 truncate text-soft">
                  {tag && <span className="rounded-sm bg-accent/10 px-1.5 font-medium text-accent">{tag}</span>}
                </span>
                <span className="shrink-0 text-soft tabular-nums">{ago(t.at)}</span>
              </div>
              {/* Reversed and wrapping, the head's name stands before the
                  title only while both fit: short of room it goes to a
                  second, hidden line whole, and the sub's own title wins. */}
              <p className="mt-0.5 flex h-[1lh] flex-row-reverse flex-wrap justify-end truncate text-label leading-snug font-medium">
                <span className="max-w-full min-w-0 truncate">{heading}</span>
                {parent && <span className="shrink-0 pr-1 font-normal whitespace-nowrap text-soft">{parent} ›</span>}
              </p>
              {/* No gist, no line: an empty one read as a card half drawn. */}
              {t.gist && <p className="mt-0.5 h-[1lh] truncate text-caption leading-snug text-mid">{t.gist}</p>}
            </button>
          </article>
        </Held>
      </li>
    );
  };
  return <ul data-surface="workers" className="chat-column flex flex-col gap-2 pt-2 pb-10">{put.map(row)}</ul>;
}

/** What a worker's hold or right click offers: a Working one only the
 * cancel of a nap it keeps for later, an archived one its restore.
 * `toNap` turns the menu to the nap's times. */
function offers(t: Thread, work: Work, pull: (t: Thread) => void, toNap: () => void): MenuItem[] {
  const item = (label: string, d: ReactNode, run: () => void) => ({
    label,
    icon: (
      <svg viewBox="0 0 24 24" className="size-[17px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
        {d}
      </svg>
    ),
    run,
  });
  const { act, clear } = work;
  if (t.state === "working") return t.nap ? [item("Cancel nap", <path d="M18 6 6 18M6 6l12 12" />, () => void act(t.id, "wake"))] : [];
  if (t.state === "archived") return [item("Restore", <path d="M3 12a9 9 0 1 0 3-6.7L3 8M3 3v5h5" />, () => void act(t.id, "restore"))];
  return [
    ...(t.state === "waiting" ? [item("Mark read", <path d="M20 6 9 17l-5-5" />, () => void clear(t))] : []),
    t.state === "napping"
      ? item(
          "Wake now",
          <>
            <circle cx="12" cy="12" r="4" />
            <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
          </>,
          () => void act(t.id, "wake"),
        )
      : item("Nap", <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />, toNap),
    item("Bring to chat", <path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12Z" />, () => pull(t)),
    item(
      "Archive",
      <>
        <rect x="3" y="4" width="18" height="4" rx="1" />
        <path d="M5 8v11a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V8M10 12h4" />
      </>,
      () => void act(t.id, "archive"),
    ),
  ];
}

/** A worker's hold menu, for `Held`'s `menu`; none when it offers nothing. */
export const workerMenu = (t: Thread, work: Work, pull: (t: Thread) => void) =>
  offers(t, work, pull, () => {}).length > 0
    ? (grip: Grip, close: () => void) => <CardMenu grip={grip} close={close} t={t} offers={(toNap) => offers(t, work, pull, toNap)} nap={(until) => void work.nap(t.id, until)} />
    : undefined;

/** A card's menu, which turns to the nap's times on Nap, as T3's snooze
 * submenu does, and to a time of the person's own on "Pick a time". */
function CardMenu({
  grip,
  close,
  t,
  offers,
  nap,
}: {
  grip: Grip;
  close: () => void;
  t: Thread;
  offers: (toNap: () => void) => MenuItem[];
  nap: (until: number) => void;
}) {
  const [step, setStep] = useState<"menu" | "nap" | "time">("menu");
  const [time, setTime] = useState(() => localInput(new Date(Date.now() + 3600_000)));
  const done = (run: () => void) => () => {
    run();
    close();
  };
  const moon = <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />;
  const icon = (d: ReactNode) => (
    <svg viewBox="0 0 24 24" className="size-[17px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      {d}
    </svg>
  );
  const items: MenuItem[] =
    step === "menu"
      ? offers(() => setStep("nap")).map((o) => (o.label === "Nap" ? o : { ...o, run: done(o.run) }))
      : step === "nap"
        ? [
            ...napPresets(new Date()).map((p) => ({ label: p.label, icon: icon(moon), run: done(() => nap(p.until)) })),
            { label: "Pick a time", icon: icon(<><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></>), run: () => setStep("time") },
          ]
        : [];
  const until = Math.floor(new Date(time).getTime() / 1000);
  const ahead = until > Date.now() / 1000;
  return (
    <HoldMenu
      grip={grip}
      framed={false}
      label={step === "menu" ? t.name : `Nap ${t.name} until`}
      items={items}
      close={close}
      foot={
        step === "time" && (
          <form
            className="flex flex-col gap-2 px-4 py-2"
            onSubmit={(e) => {
              e.preventDefault();
              if (ahead) done(() => nap(until))();
            }}
          >
            <input
              type="datetime-local"
              aria-label="Wake time"
              value={time}
              min={localInput(new Date())}
              onChange={(e) => setTime(e.target.value)}
              className="w-full rounded-lg border border-frame bg-paper px-2 py-1.5 text-[15px] text-ink"
            />
            <button type="submit" disabled={!ahead} className="self-end rounded-lg px-3 py-1.5 text-[15px] font-medium text-ink hover:bg-hover disabled:text-soft">
              Nap
            </button>
          </form>
        )
      }
    />
  );
}

/** Promote's mark: an arrow leaving a box. */
export function Glyph({
  className = "inline size-[15px] align-[-2px]",
}: {
  className?: string;
}) {
  return (
    <svg
      viewBox="0 0 24 24"
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.9}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M14 4h6v6M20 4l-9 9" />
      <path d="M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4" />
    </svg>
  );
}


/** On the Temp button: a pulse while a closed temp chat is answering, a dot
 * once its answer waits. */
export function TempMark({ at = "top-2 left-[26px]" }: { at?: string }) {
  const { working: busy, unread, open } = useScratchpad();
  if (open || (!busy && !unread)) return null;
  return <span aria-label={busy ? "Answering" : "Answer waiting"} className={`absolute ${at} size-2 rounded-full bg-accent ${busy ? "animate-pulse" : ""}`} />;
}
/** Under a reply: promote what it is about to a Work thread, with the
 * person's directions if they give any. The panel hangs from the reply's own
 * row, not the button, so it stays inside the column at any width. */
export function Promote({ id, text }: { id: string; text: string }) {
  const shell = useShell();
  const [asking, setAsking] = useState(false);
  const [directions, setDirections] = useState("");
  if (!shell) return null;
  const go = () => {
    shell.promote(directions, { id, text });
    setAsking(false);
    setDirections("");
  };
  return (
    <span>
      <button
        type="button"
        onClick={() => setAsking(!asking)}
        aria-label="Create task"
        title="Create task"
        className={`grid size-7 place-items-center rounded-md hover:bg-hover hover:text-ink ${asking ? "text-ink" : "text-soft opacity-0 transition-opacity group-hover:opacity-100 max-sm:opacity-100"}`}
      >
        <Glyph className="size-[15px]" />
      </button>
      {asking && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            go();
          }}
          data-overhang
          className="rise absolute bottom-9 left-0 z-30 flex w-[min(340px,100%)] flex-col gap-2 rounded-2xl border border-frame bg-lift p-3 shadow-dock"
        >
          <label className="text-caption font-semibold text-mid">
            Create task
          </label>
          <textarea
            autoFocus
            value={directions}
            onChange={(e) => setDirections(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") setAsking(false);
              if (
                e.key === "Enter" &&
                !e.shiftKey &&
                !e.nativeEvent.isComposing &&
                matchMedia("(pointer: fine)").matches
              ) {
                e.preventDefault();
                go();
              }
            }}
            rows={2}
            placeholder="Directions, if any (e.g. build it, research the options)"
            className="resize-none rounded-xl border border-hair bg-paper px-3 py-2 text-control outline-none placeholder:text-soft focus:border-soft"
          />
          <div className="flex justify-end gap-1">
            <button
              type="button"
              onClick={() => setAsking(false)}
              className="rounded-full px-3 py-1 text-label text-mid hover:bg-hover"
            >
              Cancel
            </button>
            <button
              type="submit"
              className="rounded-full bg-ink px-3.5 py-1 text-label font-medium text-paper"
            >
              Promote
            </button>
          </div>
        </form>
      )}
    </span>
  );
}

/** How far a finger must travel sideways, in pixels, to change place. */
const SWIPE_PLACE = 64;

/** Whether a sideways swipe from here belongs to what is under the finger:
 * text being edited, or something that scrolls sideways itself. */
function keepsSwipe(target: EventTarget | null) {
  for (let el = target as HTMLElement | null; el; el = el.parentElement) {
    // A phone's bar reads its own swipe, to change workspace (phone.tsx).
    if (el.matches("input, textarea, select, [data-own-swipe]")) return true;
    if (el.scrollWidth > el.clientWidth && /auto|scroll/.test(getComputedStyle(el).overflowX)) return true;
  }
  return false;
}

/** A swipe sideways on the phone's page, read on lift: `move(1)` for a swipe
 * left, `move(-1)` for right. A mostly vertical pull is a scroll, two fingers
 * are a pinch, and an open menu or dialog keeps its own gestures. */
export function useSideSwipe(on: boolean, move: (by: 1 | -1) => void) {
  const latest = useRef(move);
  latest.current = move;
  useEffect(() => {
    if (!on) return;
    let from: { x: number; y: number } | undefined;
    const start = (e: TouchEvent) => {
      const t = e.touches[0];
      from = e.touches.length === 1 && !keepsSwipe(e.target) ? { x: t.clientX, y: t.clientY } : undefined;
    };
    const end = (e: TouchEvent) => {
      const f = from;
      from = undefined;
      if (!f || document.querySelector('[role=menu], [role=listbox], [role=dialog]')) return;
      const t = e.changedTouches[0];
      const dx = t.clientX - f.x;
      const dy = t.clientY - f.y;
      if (Math.abs(dx) >= SWIPE_PLACE && Math.abs(dx) > Math.abs(dy) * 2) latest.current(dx < 0 ? 1 : -1);
    };
    document.addEventListener("touchstart", start, { passive: true });
    document.addEventListener("touchend", end, { passive: true });
    return () => {
      document.removeEventListener("touchstart", start);
      document.removeEventListener("touchend", end);
    };
  }, [on]);
}
