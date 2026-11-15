// The conversation as Libri keeps it: history from hippo, then what
// the server pushes over SSE, with history again whenever the stream may
// have missed something. The server answers one message at a
// time and says so with `typing`.
import { useCallback, useEffect, useRef, useState } from "react";
import { api, url as apiUrl } from "./api";
import { forHost, keepFiles, keptFiles } from "./keep";
import { PREFIX, readScratchpads, scratchpadEvent } from "./scratchpad";
import { PREFIX as MEMORY_PREFIX, memoryChatChanged, memoryChatEvent } from "./memoryChats";
import { publishedPage } from "./pageLink";
import type { AttachmentAdapter } from "@assistant-ui/react";

// crypto.randomUUID is for secure pages only, and the app is also served
// over plain http (for Safari, which refuses an http host from https), so
// the same version-4 id is made from getRandomValues, which every page has.
const newId = (): string =>
  crypto.randomUUID?.() ??
  "10000000-1000-4000-8000-100000000000".replace(/[018]/g, (c) => (Number(c) ^ (crypto.getRandomValues(new Uint8Array(1))[0] & (15 >> (Number(c) / 4)))).toString(16));

/** Actual harness measurements. Missing values are not estimated. */
export type Metrics = { context_tokens?: number | null; context_window?: number | null; tokens_per_second?: number | null; input?: number; output?: number; cache_read?: number; cache_write?: number; model?: string | null };

export type Msg = {
  id: string;
  metrics?: Metrics;
  /** Narration before tool work, hidden in Simple. */
  progress?: boolean;
  role: "user" | "assistant" | "system";
  text: string;
  /** Shown word for word: a draft waiting for approval. */
  plain?: boolean;
  /** A picture Libri sent, as a URL. */
  picture?: string;
  /** A kept file sent back, saved from /api/file/{name}. */
  file?: string;
  reaction?: string;
  /** Waiting for the computer to come back; drawn with a dotted edge. */
  queued?: boolean;
  /** Why the site refused it; kept with a retry until the person decides. */
  failed?: string;
  /** News from outside the talk (a routine, watch, alert or passed-on
   * message) that has landed in the chat; drawn with a rule beside it. */
  outside?: boolean;
  date?: Date;
  /** Files sent with the message; `image` is a local URL to show one by. */
  files?: { name: string; image?: string }[];
  /** The tools the model used on the way to this answer, in order. */
  tools?: Tool[];
  /** Still working: tool calls go here until the answer arrives. */
  live?: boolean;
  /** How many calls the turn had made when a worker's report woke it
   * again: a quiet reply to that report takes back only what came after. */
  woke?: number;
  /** The model's thinking since its last call, summarized, as it streams. */
  thinking?: string;
  /** The AG-UI text message its words came in last. */
  seg?: string;
  /** The earlier message this one replies to. */
  quote?: Quote;
  /** The kind of device the person sent it from (libri/src/origin.rs). */
  from?: string;
  /** The hippo row it begins with, which a reply's quote names: from the
   * history, or told live once hippo takes it (the `row` event). */
  row?: number;
  /** The rows its words came in under live, to know it by when the `row`
   * event names one (its calls go by `Tool.row`). */
  keys?: string[];
};

/** The earlier message a message replies to, by its hippo row (`Said.i`),
 * with its first words (`Quote` in main.rs). */
export type Quote = { id: number; text: string };

/** A tool call: `before` holds the words the model said on its way to it,
 * which the page shows above it, as a terminal does. */
export type Tool = { id: string; name: string; input: unknown; result?: string; error?: boolean; before?: string; thought?: string; agent?: string; report?: string; told?: string; args?: string; /** A kept file shown in a scratchpad's thread. */ file?: string; /** The call's display result, restored from its hippo row. */ structured?: unknown; /** This page heard the call begin, so its card opens by itself. */ heard?: boolean; /** The hippo row the call is kept as. */ row?: string; /** When it began, by the host's clock (AG-UI's timestamp), and how long it ran, in milliseconds. */ began?: number; took?: number; /** What it has given so far, while it runs: a shell's output as it prints. Live only. */ partial?: string };

/** A subagent on its own: its name, its task, and its work so far, words
 * and calls in order. One the seat's Agent call started goes by the call's
 * id, with `title` its description; nothing outside the CLI can stop it.
 * `stopping` once a stop is asked for; `done` once it has left the list. */
export type Task = { metrics?: Metrics; name: string; text: string; tools: Tool[]; words: string; title?: string; stopping?: boolean; done?: boolean; /** Counts results of calls this page never heard begin. */ unheard?: number; /** A scratchpad's thinking since its last call, as it streams. */ thinking?: string };

type Agent = { name: string; task: string; title?: string };

/** An act waiting for the person's word: today only a mail draft. */
export type Act = { kind: "mail"; account: string | null; to: string; subject: string; body: string };
export type Pending = { id: number; act: Act; event: string | null; /** A scratchpad drafted it: it shows on that column, and what comes of it stays there. */ scratchpad?: boolean };
/** A card in the tray: `busy` while a verdict is carried out, `error` when it failed. */
export type Card = Pending & { busy?: boolean; error?: string };

/** One of AskUserQuestion's questions. */
export type Question = { question: string; header?: string; options: { label: string; description?: string }[]; multiSelect?: boolean };
/** What the seat asks mid-turn, as Claude Code would in a terminal: may `tool` run, or AskUserQuestion's questions. */
/** `always`: a connection's tool that asks first, whose card offers "Always allow" too. */
export type Ask = { id: string; tool: string; input: Record<string, unknown>; about: string | null; always?: boolean; busy?: boolean };
/** Each question's answer, by its text; several picks joined by ", ". */
export type Answers = Record<string, string>;

/** Replies the person may tap under the newest message: the model's
 * offer_replies call, which a tap answers as a reply to `quote`, or a
 * question of AskUserQuestion's with a few answers, which a tap answers. */
export type ReplyOffer = { replies: string[]; quote?: Quote; ask?: { id: string; question: string } };

/** Most replies a message offers: REPLIES_MAX in tools.rs. */
const REPLIES_MAX = 3;
/** What offer_replies answers once it took them (tools.rs): hippo keeps a
 * call's result but not whether it failed, so a refusal is told apart by it. */
const SHOWN = "Shown under your message.";

/** Whether a tool call is an offer_replies the chat shows as its replies,
 * so its own row would say the same twice (2026-11-06); a refused or
 * failed one keeps its row, so the fault shows. */
export const offerShown = (name: string, result: unknown, error?: boolean) =>
  name.endsWith("offer_replies") && !error && (result === undefined || (typeof result === "string" && result.trim() === SHOWN));

/** The replies the newest message offers, if it is the model's, finished
 * and kept: they go once the person answers or anything later is said. */
export function offered(ms: Msg[]): ReplyOffer | undefined {
  const m = ms.at(-1);
  if (m?.role !== "assistant" || m.live) return undefined;
  const call = [...(m.tools ?? [])].reverse().find((t) => t.name.endsWith("offer_replies") && t.result?.trim() === SHOWN && !t.error);
  const options = (call?.input as { options?: unknown } | undefined)?.options;
  const row = m.row;
  if (!Array.isArray(options) || row === undefined) return undefined;
  const replies = options.filter((o): o is string => typeof o === "string" && !!o.trim()).slice(0, REPLIES_MAX);
  return replies.length ? { replies, quote: { id: row, text: firstLine(m.text) } } : undefined;
}

/** An AskUserQuestion of one question with a few single answers, offered as
 * replies, as Claude asks it in a terminal; anything more stays its card. */
export function askOffer(a: Ask): ReplyOffer | undefined {
  if (a.tool !== "AskUserQuestion" || a.busy) return undefined;
  const qs = a.input.questions as Question[] | undefined;
  const q = qs?.length === 1 ? qs[0] : undefined;
  if (!q || q.multiSelect || !q.options.length || q.options.length > REPLIES_MAX) return undefined;
  return { replies: q.options.map((o) => o.label), ask: { id: a.id, question: q.question } };
}

export type Said = { i: number; kind: string; text: string; date: string; display?: { version: number; result?: unknown; metrics?: Metrics }; reaction: string | null; src?: { e: string }; quote?: Quote; from?: string };

/** What the turn holds that hippo doesn't yet: `start` is when it began, in
 * Unix milliseconds, `seq` the number of the last event it took. */
type TurnRecord = { start: number; seq: number; shown: Ev[] };

/** An AG-UI event (docs.ag-ui.com) as the server sends it: the fields
 * this page reads. `subagentRunId` marks a subagent's work. */
export type Ev = {
  type: string;
  messageId?: string;
  delta?: string;
  role?: string;
  toolCallId?: string;
  toolCallName?: string;
  content?: unknown;
  activityType?: string;
  patch?: { op: string; path: string; value: unknown }[];
  subagentRunId?: string;
  name?: string;
  description?: string;
  result?: unknown;
  value?: unknown;
  outcome?: { type: "success" } | { type: "interrupt"; interrupts: Interrupt[] };
  metadata?: Record<string, unknown>;
  /** Its number in the turn's record, if the record took it. */
  seq?: number;
  /** The row this event will be, known before the row exists. */
  key?: string;
  /** The row hippo had last settled when this event came. */
  after?: string;
  /** When the host sent it, in Unix milliseconds. */
  timestamp?: number;
};

/** A run's interrupt: one of the seat's asks. */
export type Interrupt = { id: string; reason: string; message?: string; metadata: { tool: string; input: Record<string, unknown>; always?: boolean } };

export const askOf = (i: Interrupt): Ask => ({ id: i.id, tool: i.metadata.tool, input: i.metadata.input, about: i.message ?? null, always: !!i.metadata.always });

/** The one conversation, AG-UI's thread. */
const THREAD = "main";

/** The slash commands the page answers itself; `/` opens them as a menu. */
export const COMMANDS: { name: string; arg?: string; does: string }[] = [
  { name: "model", arg: "[name]", does: "show or switch the model for the next turns" },
  { name: "effort", arg: "[level]", does: "show or switch the thinking effort for the next turns" },
  { name: "harness", arg: "[name]", does: "show or switch the agent harness (Claude Code, Codex, OpenCode…) for the next turns" },
  { name: "permission-mode", arg: "[mode]", does: "show or switch the permission mode for the next turns" },
  { name: "sandbox", arg: "[setting]", does: "show or switch the harness's sandbox for the next turns" },
  { name: "signin", arg: "[harness]", does: "show which harnesses are signed in, or sign Codex or omp in to ChatGPT by a code (OpenCode, Pi and Muse use their own config)" },
  { name: "work", arg: "[directions]", does: "create a task from this part of the chat, a thread of its own on the Tasks page" },
  { name: "stop", does: "stop the turn running now" },
  { name: "memory", arg: "[name]", does: "show which memory this chat is on, or open another memory's chat here" },
  { name: "help", does: "list these commands" },
  { name: "clear", does: "start a new session: the next turn opens afresh, given the chat's newest words" },
  { name: "new", does: "the same as /clear" },
];

/** The harness the main chat runs on and the slash commands it has of its
 * own (site.rs `commands_now`), as it last reported them: a message that is
 * one of them alone goes to it as typed. `known` is false until a harness
 * that reports them has; `all` holds every harness's. */
export type HarnessCommands = { harness: string; commands: string[]; known: boolean; all: Record<string, string[]> };

export async function harnessCommands(): Promise<HarnessCommands | undefined> {
  const r = await api("api/commands").catch(() => undefined);
  return r?.ok ? ((await r.json()) as HarnessCommands) : undefined;
}

/** Commands a chat tile answers itself, as it alone knows how: `/memory`
 * moves the tile, which only the tile can. Each is told the argument and
 * how to put a note in the chat. */
export const tileCommands = new Map<string, (arg: string, note: (said: string) => void) => void | Promise<void>>();

/** A slash command that isn't Libri's: one the main chat's harness has goes
 * to it as typed (false: send it); one it hasn't, but another harness has,
 * is answered here with a note rather than sent to the model as words, as
 * is /compact on a harness that compacts only on its own. Anything else is
 * words ("/stop the car"). */
async function unrunnable(text: string, note: (said: string) => void): Promise<boolean> {
  const name = /^\/([\w:.-]+)(?:\s[^\n]*)?$/.exec(text.trim())?.[1];
  if (!name) return false;
  const own = await harnessCommands();
  // A harness that hasn't reported its commands yet is given the words.
  if (!own || !own.known || own.commands.includes(name)) return false;
  const others = Object.entries(own.all).filter(([h, ns]) => h !== own.harness && ns.includes(name)).map(([h]) => h);
  if (name === "compact") note(`${own.harness} compacts its session on its own as it fills; it can't be asked to here. /clear starts a new one.`);
  else if (others.length) note(`/${name} is ${others.join(" and ")}'s; ${own.harness} can't run it here.`);
  else return false;
  return true;
}

/** Told when a command changes what the composer's pickers show. */
export const picksChanged = new Set<() => void>();

/** The longest message text, in bytes, that /api/run takes (SEND_MAX in site.rs). */

/** A message the site has not taken yet. Every send starts here and leaves
 * once the site answers, so no sync, reload or refusal can lose it: one the
 * computer couldn't be reached for waits, one it refused stays with the reason.
 * All of it outlives a reload: words in localStorage, files in IndexedDB. */
type Unsent = {
  id: string;
  text: string;
  date: number;
  files?: File[];
  shown?: Msg["files"];
  /** How many files IndexedDB holds for it, which an earlier page wrote. */
  filed?: number;
  redo?: "again";
  quote?: Quote;
  /** The person's last message when a redo was asked, which it replaces. */
  over?: string;
  queued?: boolean;
  failed?: string;
  /** From an earlier page, which may have got as far as sending it. */
  carried?: boolean;
};
const UNSENT = () => forHost("unsent");
const loadUnsent = (): Unsent[] => {
  try {
    const us: Unsent[] = JSON.parse(localStorage.getItem(UNSENT()) ?? "[]");
    return us.map((u) => ({ ...u, carried: true }));
  } catch {
    return [];
  }
};
const saveUnsent = (us: Unsent[]) => {
  const kept = us.map(({ id, text, date, redo, quote, over, queued, failed, files, filed }) => ({ id, text, date, redo, quote, over, queued, failed, filed: files?.length || filed }));
  if (kept.length) localStorage.setItem(UNSENT(), JSON.stringify(kept));
  else localStorage.removeItem(UNSENT());
};
/** No answer, or the proxy's while Libri restarts: the computer is down,
 * not the message wrong. */
const down = (r: Response | undefined) => !r || r.status === 502 || r.status === 503 || r.status === 504;
export const SEND_MAX = 32 * 1024;
/** The largest file /api/upload takes (UPLOAD_MAX in site.rs). */
const UPLOAD_MAX = 100_000_000;
const megabytes = (n: number) => `${Math.ceil(n / 1_000_000)} MB`;
// The queue goes one at a time, so a request that never answers would hold
// back all behind it. The site drops a second copy, so giving up is safe.
const RUN_WAIT = 20_000;
const UPLOAD_WAIT = 30 * 60_000;
/** This long without a ping and the stream is taken for dead. The host pings
 * each second when quiet (PULSE in site.rs), so a stale-link notice could
 * come after three. */
const STREAM_SILENCE = 45_000;
// How long a first load waits on its stream to open before reading the
// history without it.
const FIRST_OPEN = 3000;

/** Why the site turned a message or its file away, in the person's words. */
async function refusal(r: Response, file?: File): Promise<string> {
  if (r.status === 403) return (await r.text().catch(() => "")) || "This device may not send messages.";
  if (r.status === 413) return file ? `${file.name} is too large; the limit is ${megabytes(UPLOAD_MAX)}.` : "This message is too large to send.";
  if (r.status === 429) return "Too many messages at once. Try again in a moment.";
  if (r.status === 400) return "The computer could not read this message.";
  return "The computer hit an error and did not take this message.";
}

const ROLE: Record<string, Msg["role"]> = { user: "user", talk: "assistant", note: "system" };

// A draft as a door shows it (pending.rs `Act::show`), kept in hippo as talk;
// on reload it is framed as it was.
const DRAFT = /^Draft to [^\n]+\nSubject: [\s\S]*\n\n👍 sends it, 👎 drops it\.$/;
/** How hippo logs a picture or file Libri sent (`send_outbox`). */
const SENT = /^\(sent a (picture|file): (.+)\)$/;
// Whole, since assistant-ui shows a picture only from an https: (or data:
// or blob:) URL.
const fileUrl = (name: string) => new URL(apiUrl(`api/file/${encodeURIComponent(name)}`, true), location.href).href;

export type Chat =
  | { state: "loading" }
  | { state: "signed-out" }
  | { state: "error"; error: string }
  | {
      state: "ready";
      messages: Msg[];
      typing: boolean;
      /** Words on their way, which the computer hasn't taken yet. */
      sending: boolean;
      /** The turn's background jobs still running, by description. */
      jobs: string[];
      /** `redo` puts the message in place of the person's last one. */
      send: (text: string, files: File[], redo?: "again", quote?: Quote) => Promise<void>;
      /** Sends a refused message again. */
      retry: (id: string) => void;
      /** Takes a refused message off the page; its words and files come back, to edit. */
      discard: (id: string) => { text: string; files: File[] } | undefined;
      stop: () => Promise<void>;
      cards: Card[];
      decide: (id: number, yes: boolean) => Promise<void>;
      asks: Ask[];
      /** Lets the tool run (with answers, for questions) or refuses it. */
      reply: (id: string, yes: boolean, answers?: Answers, always?: boolean) => Promise<void>;
      tasks: Task[];
      stopTask: (name: string) => Promise<void>;
      /** Counts each word sent to a subagent, which changes where it stands. */
      touched: number;
      /** Tells a subagent the person's words, as tell_agent would; false if it couldn't. */
      tellTask: (name: string, text: string, files?: File[]) => Promise<boolean>;
      /** Reads the conversation afresh from hippo, as after a call. */
      refresh: () => void;
      /** The page show_page last published while this page listened, to open by itself. */
      published?: Published;
      /** Counts each read of the history that laid the talk out afresh. */
      synced: number;
    };

/** A page published live; `n` tells one publishing from the next of the same page. */
export type Published = { page: string; n: number };

export function useChat(): Chat {
  const [status, setStatus] = useState<"loading" | "signed-out" | "ready" | string>("loading");
  const [messages, setMessages] = useState<Msg[]>([]);
  const [typing, setTyping] = useState(false);
  const [sending, setSending] = useState(false);
  const [jobs, setJobs] = useState<string[]>([]);
  const [cards, setCards] = useState<Card[]>([]);
  const [asks, setAsks] = useState<Ask[]>([]);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [touched, setTouched] = useState(0);
  const [published, setPublished] = useState<Published>();
  const [synced, setSynced] = useState(0);
  const resync = useRef<() => void>(() => {});
  const unsent = useRef<Unsent[]>(loadUnsent());

  const lastSaid = useRef<string | undefined>(undefined);
  const flushing = useRef(false);
  // An earlier page's files, which nothing may send before they are back.
  const restored = useRef<Promise<void>>(Promise.resolve());

  const forget = useCallback((id: string) => {
    const u = unsent.current.find((u) => u.id === id);
    unsent.current = unsent.current.filter((x) => x.id !== id);
    saveUnsent(unsent.current);
    if (u?.files?.length || u?.filed) void keepFiles(id, []);
  }, []);

  const mark = useCallback((id: string, change: Partial<Unsent>) => {
    unsent.current = unsent.current.map((u) => (u.id === id ? { ...u, ...change } : u));
    saveUnsent(unsent.current);
    setMessages((ms) => ms.map((m) => (m.id === id ? { ...m, ...("queued" in change ? { queued: change.queued } : {}), ...("failed" in change ? { failed: change.failed } : {}) } : m)));
  }, []);

  // A message and its files to Libri.
  const deliver = useCallback(async (u: Unsent): Promise<"sent" | "down" | "signed-out" | { failed: string }> => {
    // Files go ahead, each answering the id the message names it by.
    const parts: unknown[] = [];
    for (const f of u.files ?? []) {
      if (f.size > UPLOAD_MAX) return { failed: `${f.name} is ${megabytes(f.size)}; the limit is ${megabytes(UPLOAD_MAX)}.` };
      const r = await upload(f);
      if (r?.status === 401) return "signed-out";
      if (down(r)) return "down";
      if (!r!.ok) return { failed: await refusal(r!, f) };
      const { id } = (await r!.json()) as { id: string };
      parts.push({ type: "binary", id, mimeType: f.type || "application/octet-stream", filename: f.name });
    }
    const content = parts.length ? [{ type: "text", text: u.text }, ...parts] : u.text;
    const r = await run({ messages: [{ id: u.id, role: "user", content }], forwardedProps: { ...(u.redo ? { redo: u.redo } : {}), ...(u.quote ? { quote: u.quote } : {}) } });
    if (r?.status === 401) return "signed-out";
    if (down(r)) return "down";
    return r!.ok ? "sent" : { failed: await refusal(r!) };
  }, []);

  // The queue in order, each message once the one before has an answer, until
  // the computer is down again; a refused one stays out of the way. The site
  // drops a second copy, so a message whose answer was lost may go twice.
  const flush = useCallback(async () => {
    if (flushing.current) return;
    flushing.current = true;
    setSending(true);
    try {
      await restored.current;
      for (;;) {
        const u = unsent.current.find((u) => !u.failed);
        if (!u) break;
        const result = await deliver(u);
        if (result === "signed-out") {
          setStatus("signed-out");
          break;
        }
        // Heard over the stream already, so it is in the talk whatever came back.
        if (!unsent.current.some((x) => x.id === u.id)) continue;
        if (result === "sent") {
          forget(u.id);
          setMessages((ms) => ms.map((m) => (m.id === u.id ? { ...m, queued: false } : m)));
          // A message from an earlier page may be in the history already.
          if (u.carried) resync.current();
          continue;
        }
        // A redo doesn't wait, since what it replaces may have moved on by then.
        if (result === "down" && !u.redo) {
          for (const w of unsent.current) if (!w.failed) mark(w.id, { queued: true });
          break;
        }
        mark(u.id, result === "down" ? { failed: "The computer can't be reached." } : result);
        // A redo refused leaves the history as it was; the page shows that again.
        if (u.redo) resync.current();
      }
    } finally {
      flushing.current = false;
      setSending(false);
    }
  }, []);

  useEffect(() => {
    const waiting = unsent.current.filter((u) => u.filed);
    restored.current = Promise.all(
      waiting.map(async (u) => {
        const files = await keptFiles(u.id);
        await Promise.all(files.map((file) => attachments.add({ file })));
        const shown = files.map((f) => ({ name: f.name, image: f.type.startsWith("image/") ? URL.createObjectURL(f) : undefined }));
        unsent.current = unsent.current.map((x) => (x.id === u.id ? { ...x, files, shown } : x));
        setMessages((ms) => ms.map((m) => (m.id === u.id ? { ...m, files: shown } : m)));
        // The words without the file would be a different message.
        if (files.length < (u.filed ?? 0)) mark(u.id, { failed: "This browser did not keep the file. Add it again.", queued: false });
      }),
    ).then(() => undefined);
  }, []);

  useEffect(() => {
    lastSaid.current = messages.filter((m) => m.role === "user" && !unsent.current.some((u) => u.id === m.id)).at(-1)?.text;
  }, [messages]);

  useEffect(() => {
    let events: EventSource | undefined;
    let retry: number | undefined;
    let healing: number | undefined;
    let opening: number | undefined;
    let wait = 1000;
    let heard = 0;
    // The id of the newest event taken, which a reopened stream names so the
    // host sends only what came after it; empty before the first.
    let last = "";
    let ready = false;
    let gone = false;
    // show_page calls heard begin on the stream: only their results open the
    // page, so history and a reconnect's read never do.
    const publishing = new Set<string>();
    // The newest sync, and the turn starts and ends heard: an answer overtaken
    // by either is out of date.
    let asked = 0;
    let turns = 0;
    // What the stream brings while a sync reads, which its record may lack.
    let reading = false;
    let during: Ev[] = [];
    // History from hippo, on the first load and whenever the host can't
    // replay what the page missed (it fell too far behind, or Libri
    // restarted): what the stream carried while nothing listened is in
    // hippo by then.
    const sync = async (): Promise<boolean> => {
      const mine = ++asked;
      const heard = turns;
      reading = true;
      during = [];
      try {
        return await read(mine, heard);
      } finally {
        if (mine === asked) reading = false;
      }
    };
    const read = async (mine: number, heard: number): Promise<boolean> => {
      // The record first: what leaves it while the history is read is then
      // in the history, so no read falls between the two.
      const t = await api("api/task");
      const [r, w] = await Promise.all([api("api/history"), api("api/pending")]);
      if (r.status === 401 || t.status === 401) {
        setStatus("signed-out");
        return false;
      }
      if (!r.ok) throw new Error(`History failed: ${r.status}`);
      if (!t.ok) throw new Error(`Turn failed: ${t.status}`);
      const said: Said[] = await r.json();
      // The tray is extra: a chat without it still works.
      const waiting: Pending[] = w.ok ? await w.json() : [];
      const now: { agents: Agent[]; typing?: boolean; turn?: TurnRecord } = await t.json();
      if (gone) return false;
      if (mine !== asked) return true;
      if (heard !== turns) return sync();
      const turning = !!now.typing;
      // The end of a turn may have gone out while nothing listened.
      setTyping(turning);
      setJobs(turning ? jobsOf(now.turn?.shown ?? []) : []);
      setAsks(turning ? asksOf(now.turn?.shown ?? []) : []);
      // The work so far went out over the stream; a page that missed it
      // starts from the task.
      setTasks((ks) => atWork(ks, now.agents));
      const turn = now.turn && { ...now.turn, shown: [...now.turn.shown, ...during.filter((e) => (e.seq ?? 0) > now.turn!.seq)] };
      setMessages((was) => fromHistory(said, turn, turning, was));
      setSynced((n) => n + 1);
      // The history knows nothing of the messages still waiting.
      setMessages((ms) => [
        ...ms,
        ...unsent.current
          .filter((u) => !ms.some((m) => m.id === u.id))
          .map((u): Msg => ({ id: u.id, role: "user", text: u.text, files: u.shown, quote: u.quote, date: new Date(u.date), queued: u.queued, failed: u.failed })),
      ]);
      setCards(waiting);
      setStatus("ready");
      ready = true;
      void flush();
      return true;
    };
    // What the page missed is only in the history, so a read that fails is
    // read again, each wait longer, until one lands; otherwise it stays a
    // hole in the talk until the next reload.
    const settle = () => {
      window.clearTimeout(healing);
      readScratchpads();
      sync().then(
        () => void (wait = 1000),
        (e) => {
          // A first load says why it has no chat yet, as start() does.
          if (!ready) setStatus(`${e instanceof Error ? e.message : e}. Trying again…`);
          healing = window.setTimeout(settle, wait);
          wait = Math.min(wait * 2, 30000);
        },
      );
    };
    // The browser retries a dropped stream by itself but gives up on an
    // answer that isn't 200, such as nginx's 502 while Libri restarts;
    // then the page tries again until Libri is back, and the stream's
    // opening says whether it must read the chat. The light read first
    // tells a host that is down from one that signed the page out.
    const again = () => {
      window.clearTimeout(retry);
      retry = window.setTimeout(() => {
        api("api/task").then(
          (r) => (r.status === 401 ? setStatus("signed-out") : r.ok ? listen() : again()),
          () => again(),
        );
      }, 3000);
    };
    const listen = () => {
      events?.close();
      // The stream opens on where the page stands: the host answers with the
      // events since (`replay`), or with `sync` when the page must read the
      // chat whole (it fell too far behind, or Libri restarted). A page that
      // stands nowhere yet reads the chat as the stream opens.
      // The browser's own reconnects send its newest id, the same as `last`.
      events = new EventSource(apiUrl(`api/events?after=${encodeURIComponent(last)}`, true));
      heard = Date.now();
      events.onopen = () => {
        heard = Date.now();
        window.clearTimeout(opening);
        if (!last) settle();
      };
      // Each ping says where the page stands, for one no event has reached.
      events.addEventListener("ping", (m) => {
        heard = Date.now();
        if (m.lastEventId) last = m.lastEventId;
      });
      events.addEventListener("sync", () => {
        heard = Date.now();
        settle();
      });
      // What the page missed, at once, so it draws once.
      events.addEventListener("replay", (m) => {
        heard = Date.now();
        for (const e of JSON.parse(m.data) as Ev[]) take(e);
        if (m.lastEventId) last = m.lastEventId;
      });
      events.onerror = () => {
        if (events?.readyState === EventSource.CLOSED && !gone) (ready ? again() : fallBack());
      };
      events.onmessage = (m) => {
        heard = Date.now();
        take(JSON.parse(m.data));
        if (m.lastEventId) last = m.lastEventId;
      };
    };
    const take = (e: Ev) => {
      if (e.type === "TEXT_MESSAGE_CHUNK" && e.role === "user" && unsent.current.some((u) => u.id === e.messageId)) forget(e.messageId!);
      if (reading) during.push(e);
      if (e.subagentRunId?.startsWith(PREFIX)) return scratchpadEvent(e);
      if (e.subagentRunId?.startsWith(MEMORY_PREFIX)) return memoryChatEvent(e);
      if (e.subagentRunId) return setTasks((ks) => taskEvent(ks, e));
      // What the record holds no number for is in hippo by now: a sync
      // under way may have read too soon, and is read again.
      const unnumbered = e.seq === undefined && (e.type === "TEXT_MESSAGE_START" || e.type === "CUSTOM");
      if (e.type === "RUN_STARTED" || e.type === "RUN_FINISHED" || unnumbered) turns++;
      if (e.type === "TOOL_CALL_START" && SHOW_PAGE.test(e.toolCallName ?? "")) publishing.add(e.toolCallId!);
      if (e.type === "TOOL_CALL_RESULT" && publishing.delete(e.toolCallId!) && !e.metadata?.error) {
        const page = publishedPage(String(e.content ?? ""));
        if (page) setPublished((p) => ({ page, n: (p?.n ?? 0) + 1 }));
      }
      switch (e.type) {
        case "RUN_STARTED":
          setTyping(true);
          setAsks([]);
          break;
        case "RUN_FINISHED":
          if (e.outcome?.type === "interrupt") {
            setAsks(e.outcome.interrupts.map(askOf));
            break;
          }
          setMessages((ms) => apply(ms, e));
          setTyping(false);
          setJobs([]);
          setAsks([]);
          break;
        case "ACTIVITY_SNAPSHOT":
          if (e.activityType === "jobs") setJobs((e.content as { names: string[] }).names);
          else if (e.activityType === "tool_output") setMessages((ms) => apply(ms, e));
          else if (e.activityType === "approval") {
            const p = e.content as Pending;
            setCards((cs) => (cs.some((c) => c.id === p.id) ? cs : [...cs, p]));
          }
          break;
        case "ACTIVITY_DELTA": {
          const outcome = e.patch?.find((p) => p.path === "/outcome")?.value as { text: string; done: boolean; scratchpad?: boolean } | undefined;
          const id = Number(e.messageId?.replace(/^act-/, ""));
          if (e.activityType !== "approval" || !outcome) break;
          setCards((cs) => (outcome.done ? cs.filter((c) => c.id !== id) : cs.map((c) => (c.id === id ? { ...c, busy: false, error: outcome.text } : c))));
          // What came of it stays in the talk, as it does in hippo; a
          // scratchpad's joins its own log.
          if (outcome.scratchpad) readScratchpads();
          else if (outcome.done) setMessages((ms) => [...ms, { id: newId(), role: "assistant", text: outcome.text, date: new Date() }]);
          break;
        }
        case "SUBAGENT_STARTED":
        case "SUBAGENT_FINISHED":
          setTasks((ks) => taskEvent(ks, e));
          setMessages((ms) => apply(ms, e));
          break;
        case "CUSTOM": {
          if (e.name === "scratchpad") readScratchpads();
          else if (e.name === "memory-chat") memoryChatChanged((e.value as { id: string }).id);
          // A temp chat promoted: its talk is in the history now.
          else if (e.name === "history") settle();
          else if (e.name === "forgot") {
            // The oldest went and the rest were numbered again: all of it
            // is read afresh.
            settle();
          } else setMessages((ms) => apply(ms, e));
          break;
        }
        default:
          setMessages((ms) => apply(ms, e));
      }
    };
    // A phone suspends a page it hides, and its stream may come back looking
    // open with nothing on the far end, so each return opens another.
    const shown = () => {
      if (ready && document.visibilityState === "visible") listen();
    };
    document.addEventListener("visibilitychange", shown);
    window.addEventListener("online", shown);
    resync.current = settle;
    const tick = window.setInterval(() => {
      // A send can fail while the stream stays up, so the queue is tried
      // again on its own as well as on each reconnect.
      if (unsent.current.length) void flush();
      // A stream cut somewhere on the way looks open for ever, while the page
      // stays up and hears nothing; only the missing pings give it away.
      if (ready && document.visibilityState === "visible" && events?.readyState !== EventSource.CLOSED && Date.now() - heard > STREAM_SILENCE) listen();
    }, 10000);
    // A first load whose stream won't open reads the history to say why
    // (signed out, or the host's error), and keeps trying.
    const start = () =>
      sync().then(
        (ok) => ok && listen(),
        (e) => {
          setStatus(`${e instanceof Error ? e.message : e}. Trying again…`);
          again();
        },
      );
    // A refused or dropped stream only makes the browser retry it, saying
    // nothing, so a first load whose stream hasn't opened soon reads the
    // history anyway, once, and takes over from the stream.
    const fallBack = () => {
      window.clearTimeout(opening);
      window.clearTimeout(healing);
      if (fell || ready) return;
      fell = true;
      events?.close();
      void start();
    };
    let fell = false;
    // The stream first, whose opening reads the history: read before it as
    // well, the history came twice on every load, the second time to catch
    // what went out between the two.
    listen();
    opening = window.setTimeout(fallBack, FIRST_OPEN);
    return () => {
      gone = true;
      window.clearTimeout(retry);
      window.clearTimeout(healing);
      window.clearTimeout(opening);
      window.clearInterval(tick);
      events?.close();
      document.removeEventListener("visibilitychange", shown);
      window.removeEventListener("online", shown);
    };
  }, []);

  // A slash command the page knows is done here, answered with a note, as
  // Claude Code and Codex answer theirs in place; any other goes on as
  // words, so the agent's own commands and skills still reach it. So does a
  // message that only opens with a command's name: "/stop the car" is a
  // sentence, and treating it as a command would lose it.
  const command = useCallback(async (text: string) => {
    const [, name, arg = ""] = /^\/([\w-]+)(?:\s+(\S+))?$/.exec(text.trim()) ?? [];
    const c = COMMANDS.find((c) => c.name === name?.toLowerCase());
    const note = (said: string) => setMessages((ms) => [...ms, { id: newId(), role: "system", text: said, date: new Date() }]);
    if (!c) return unrunnable(text, note);
    if (arg && !c.arg) return false;
    if (c.name === "stop") {
      const r = await api("api/stop", { method: "POST" }).catch(() => undefined);
      if (r?.status === 401) setStatus("signed-out");
    } else if (c.name === "model" || c.name === "effort" || c.name === "harness") {
      // As the composer's pickers read and set them: the pick and the
      // offers from api/models, a change through api/model or api/effort,
      // and only what the host answers is said.
      type Offer = { harness: string; model: string; name: string; efforts: string[]; default: boolean };
      type Now = { pick: { harness: string; model: string }; effort: string; offers: Offer[] };
      const r0 = await api("api/models").catch(() => undefined);
      if (r0?.status === 401) return setStatus("signed-out"), true;
      const m: Now | undefined = r0?.ok ? await r0.json() : undefined;
      if (!m) return note("Couldn't reach the computer."), true;
      const want = arg.trim().toLowerCase();
      const on = m.offers.find((o) => o.harness === m.pick.harness && o.model === m.pick.model);
      const set = async (path: string, body: object, said: string) => {
        const r = await api(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) }).catch(() => undefined);
        if (r?.status === 401) return void setStatus("signed-out");
        if (!r) return note("Couldn't reach the computer.");
        if (!r.ok) return note((await r.text().catch(() => "")) || `That didn't change (${r.status}).`);
        note(said);
        for (const tell of picksChanged) tell();
      };
      if (c.name === "effort") {
        const efforts = on?.efforts ?? [];
        const now = m.effort || "the default effort";
        if (!want) note(`Thinking at ${now}.${efforts.length ? ` The choices are ${efforts.join(", ")}, or default.` : ""}`);
        else if (want === (m.effort || "default")) note(`Already at ${want}.`);
        else if (want !== "default" && !efforts.includes(want)) note(`No effort ${want}.${efforts.length ? ` The choices are ${efforts.join(", ")}, or default.` : ""}`);
        else await set("api/effort", { effort: want === "default" ? "" : want }, `Switched to ${want === "default" ? "the default effort" : want}, from the next turn on.`);
      } else if (c.name === "model") {
        const names = [...new Set(m.offers.map((o) => o.model))];
        // The current harness's first, as a name may be on several.
        const found = [...m.offers].sort((a, b) => Number(b.harness === m.pick.harness) - Number(a.harness === m.pick.harness)).find((o) => o.model.toLowerCase() === want || o.name.toLowerCase() === want);
        if (!want) note(`Running on ${on?.name ?? m.pick.model ?? "the default model"} (${m.pick.harness}). The choices are ${names.join(", ")}.`);
        else if (!found) note(`No model ${arg.trim()}. The choices are ${names.join(", ")}.`);
        else if (found.harness === m.pick.harness && found.model === m.pick.model) note(`Already on ${found.name}.`);
        else await set("api/model", { harness: found.harness, model: found.model }, `Switched to ${found.name}${found.harness !== m.pick.harness ? ` on ${found.harness}` : ""}, from the next turn on.`);
      } else {
        const harnesses = [...new Set(m.offers.map((o) => o.harness))];
        const offered = m.offers.filter((o) => o.harness === want);
        const to = offered.find((o) => o.default) ?? offered[0];
        if (!want) note(`Answering through ${m.pick.harness}. The choices are ${harnesses.join(", ")}.`);
        else if (want === m.pick.harness) note(`Already on ${want}.`);
        else if (!to) note(`No harness ${want}. The choices are ${harnesses.join(", ")}.`);
        else await set("api/model", { harness: to.harness, model: to.model }, `Switched to ${to.harness}, on ${to.name}, from the next turn on.`);
      }
    } else if (c.name === "permission-mode" || c.name === "sandbox") {
      const [what, word] = c.name === "sandbox" ? (["sandbox", "sandbox"] as const) : (["access", "mode"] as const);
      type Choice = { id: string; name: string; note: string };
      type Held = { pick: { harness: string }; access: string; sandbox: string | null; modes?: ({ harness: string } & Record<"access" | "sandbox", Choice[]>)[] };
      const m = await api("api/models").then((r) => r.json() as Promise<Held>).catch(() => undefined);
      if (!m) return note("Couldn't reach the computer."), true;
      const all = m.modes?.find((h) => h.harness === m.pick.harness)?.[what] ?? [];
      const want = all.find((o) => o.id.toLowerCase() === arg.toLowerCase());
      if (!all.length) note(`${m.pick.harness} has no ${word} here.`);
      else if (!arg) note(`On ${all.find((o) => o.id === m[what])?.name ?? m[what]}.\n${all.map((o) => `${o.id}: ${o.note}`).join("\n")}`);
      else if (!want) note(`No ${word} ${arg}. The choices are ${all.map((o) => o.id).join(", ")}.`);
      else if (want.id === m[what]) note(`Already on ${want.name}.`);
      else {
        const r = await api("api/access", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ [what]: want.id }) }).catch(() => undefined);
        if (r?.status === 401) return setStatus("signed-out"), true;
        if (!r?.ok) note(`Couldn't switch to ${want.name}.`);
        else {
          note(`Switched to ${want.name}, from the next turn on.`);
          for (const tell of picksChanged) tell();
        }
      }
    } else if (c.name === "signin") {
      const [harness = "", provider] = arg.trim().split(/\s+/);
      if (!harness) {
        const r = await api("api/signin").catch(() => undefined);
        if (r?.status === 401) return setStatus("signed-out"), true;
        const all: Record<string, boolean> = r?.ok ? await r.json() : {};
        const list = Object.entries(all).map(([h, yes]) => `${h}: ${yes ? "signed in" : "not signed in"}`);
        note(list.length ? `${list.join("\n")}\n/signin codex or /signin omp starts a ChatGPT sign-in. OpenCode, Pi and Muse sign in through their own config.` : "Only one harness here; it is signed in by the service.");
      } else {
        const body = { harness: harness.toLowerCase(), provider };
        const r = await api("api/signin", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) }).catch(() => undefined);
        if (r?.status === 401) return setStatus("signed-out"), true;
        if (r?.ok) {
          const { url, code } = (await r.json()) as { url: string; code: string };
          note(`Open ${url} and enter the code ${code} within 15 minutes. This chat says when it's done.`);
          void followSignin(body.harness, code, note);
        } else note(`That sign-in failed${r ? ` (${r.status}${r.status === 400 ? ": only /signin codex and /signin omp sign in here" : ""})` : ""}.`);
      }
    } else if (c.name === "help") {
      const own = await harnessCommands();
      const theirs = own?.commands.filter((n) => !COMMANDS.some((c) => c.name === n)) ?? [];
      note(
        COMMANDS.map((c) => `/${c.name}${c.arg ? ` ${c.arg}` : ""}: ${c.does}`).join("\n") +
          (theirs.length ? `\n\n${own!.harness}'s own, sent to it as typed: ${theirs.map((n) => `/${n}`).join(", ")}` : ""),
      );
    } else if (c.name === "memory") {
      const run = tileCommands.get("memory");
      if (run) await run(arg.trim(), note);
      else note("This chat is on Main.");
    } else {
      // clear, new: the next turn opens a new session on its harness.
      const r = await api("api/session/clear", { method: "POST" }).catch(() => undefined);
      if (r?.status === 401) return setStatus("signed-out"), true;
      if (r?.status === 409) note("A turn is running. /stop it or wait for it, then /clear.");
      else if (!r?.ok) note("Couldn't reach the computer to start a new session.");
      else note("Cleared: the next turn starts a new session, given the chat's newest words. Everything said stays in memory.");
    }
    return true;
  }, []);

  const send = useCallback(async (text: string, files: File[], redo?: "again", quote?: Quote) => {
    // /work is words to Libri, which writes the brief and spawns it.
    const work = /^\/work(?:\s+([\s\S]*))?$/i.exec(text.trim());
    if (!redo && !quote && work) text = `Create task: ${work[1]?.trim() || "carry out what we just settled"}`;
    else if (!redo && !quote && !files.length && text.startsWith("/") && (await command(text))) return;
    // Words too long for a message go as a text file, read whole, as
    // ChatGPT and Claude take a long paste.
    if (new TextEncoder().encode(text).length > SEND_MAX) {
      files = [...files, new File([text], "pasted.txt", { type: "text/plain" })];
      text = "";
    }
    const local = newId();
    const shown = files.map((f) => ({ name: f.name, image: f.type.startsWith("image/") ? URL.createObjectURL(f) : undefined }));
    setMessages((ms) => {
      // A redo takes the place of the last exchange, as the history will.
      const at = redo ? ms.map((m) => m.role).lastIndexOf("user") : -1;
      return [...(at < 0 ? ms : ms.slice(0, at)), { id: local, role: "user", text, files: shown, quote, date: new Date() }];
    });
    unsent.current = [...unsent.current, { id: local, text, date: Date.now(), files, shown, redo, quote, over: redo ? lastSaid.current : undefined }];
    saveUnsent(unsent.current);
    if (files.length) void keepFiles(local, files);
    await flush();
  }, []);

  const retry = useCallback((id: string) => {
    const u = unsent.current.find((u) => u.id === id);
    if (!u) return;
    // Others have spoken since the redo was refused: replacing the last
    // message now would replace theirs, so it goes as a message of its own.
    mark(id, { failed: undefined, queued: false, ...(u.redo && u.over !== lastSaid.current ? { redo: undefined } : {}) });
    void flush();
  }, []);

  const discard = useCallback((id: string) => {
    const u = unsent.current.find((u) => u.id === id);
    if (!u) return undefined;
    forget(id);
    setMessages((ms) => ms.filter((m) => m.id !== id));
    return { text: u.text, files: u.files ?? [] };
  }, []);

  // The turn answers "(Stopped.)" over the stream.
  const stop = useCallback(async () => {
    const r = await api("api/stop", { method: "POST" }).catch(() => undefined);
    if (r?.status === 401) setStatus("signed-out");
  }, []);

  // The agent ends on Libri's next tick, within half a minute.
  const stopTask = useCallback(async (name: string) => {
    const stopping = (on: boolean) => setTasks((ks) => touch(ks, name, (k) => ({ ...k, stopping: on })));
    stopping(true);
    const r = await api(`api/agent/${encodeURIComponent(name)}/stop`, { method: "POST" }).catch(() => undefined);
    if (r?.status === 401) setStatus("signed-out");
    // Not asked after all: the button comes back to try again.
    else if (!r?.ok) stopping(false);
    else setTouched((n) => n + 1);
  }, []);

  // The words join its work where they fall, as the person said them; a
  // done agent goes on from its log and comes back to the list at work.
  const tellTask = useCallback(async (name: string, text: string, files: File[] = []) => {
    const ids: string[] = [];
    for (const f of files) {
      const up = await upload(f);
      if (up?.status === 401) setStatus("signed-out");
      if (!up?.ok) return false;
      ids.push(((await up.json()) as { id: string }).id);
    }
    const r = await api(`api/agent/${encodeURIComponent(name)}/tell`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ text, files: ids }) }).catch(() => undefined);
    if (r?.status === 401) setStatus("signed-out");
    if (!r?.ok) return false;
    setTasks((ks) => touch(ks, name, (k) => ({ ...k, stopping: false, words: "", tools: [...k.tools, { id: `told-${Date.now()}`, name: "", input: null, told: text || files.map((f) => f.name).join(", "), before: k.words.trim() || undefined }] })));
    setTouched((n) => n + 1);
    return true;
  }, []);

  const decide = useCallback(async (id: number, yes: boolean) => {
    setCards((cs) => cs.map((c) => (c.id === id ? { ...c, busy: true, error: undefined } : c)));
    const r = await api("api/decide", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ id, yes }),
    }).catch(() => undefined);
    if (r?.status === 401) return setStatus("signed-out");
    // Gone means decided already.
    if (r?.status === 404) return setCards((cs) => cs.filter((c) => c.id !== id));
    // Accepted: `settled` comes over the stream once it is carried out.
    if (!r?.ok) setCards((cs) => cs.map((c) => (c.id === id ? { ...c, busy: false, error: r ? `That failed (${r.status}).` : "Libri couldn't be reached." } : c)));
  }, []);

  const reply = useCallback(async (id: string, yes: boolean, answers?: Answers, always?: boolean) => {
    setAsks((as) => as.map((a) => (a.id === id ? { ...a, busy: true } : a)));
    const r = await resume(id, yes, answers, always);
    if (r?.status === 401) return setStatus("signed-out");
    // Answered: the next run starts over the stream. Gone: the turn took it back.
    if (r?.status === 409) return setAsks((as) => as.filter((a) => a.id !== id));
    if (!r?.ok) setAsks((as) => as.map((a) => (a.id === id ? { ...a, busy: false } : a)));
  }, []);

  const refresh = useCallback(() => resync.current(), []);

  if (status === "loading" || status === "signed-out") return { state: status };
  if (status !== "ready") return { state: "error", error: status };
  return { state: "ready", messages, typing, sending, jobs, send, retry, discard, stop, cards, decide, asks, reply, tasks, touched, stopTask, tellTask, refresh, published, synced };
}

// The agents at work now, each keeping the work the page has seen of it;
// those gone from the list are done, and their pages stay to be read. A
// Stop asked of one ends when it is done: done, or back at work after.
export function atWork(ks: Task[], agents: Agent[]): Task[] {
  const now = agents.map((a) => {
    const k = ks.find((k) => k.name === a.name);
    return { ...(k ?? { tools: [], words: "" }), name: a.name, text: a.task, title: a.title, done: false, stopping: k?.done ? false : k?.stopping };
  });
  const gone = ks.filter((k) => !agents.some((a) => a.name === k.name)).map((k) => ({ ...k, done: true, stopping: false }));
  return [...gone, ...now];
}

/** Starts an AG-UI run: the person's new message, answers to the open
 * interrupts, or both. Its events come over /api/events. */
const run = (input: { messages: unknown[]; resume?: unknown[]; forwardedProps?: unknown }) =>
  api("api/run", {
    method: "POST",
    signal: AbortSignal.timeout(RUN_WAIT),
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ threadId: THREAD, runId: newId(), state: {}, tools: [], context: [], forwardedProps: {}, ...input }),
  }).catch(() => undefined);

/** Answers an open interrupt, the main chat's or Temp's alike: 409 when the
 * turn took it back. */
export const resume = (id: string, yes: boolean, answers?: Answers, always?: boolean) =>
  run({ messages: [], resume: [{ interruptId: id, status: yes ? "resolved" : "cancelled", payload: answers || always ? { ...(answers && { answers }), ...(always && { always: true }) } : undefined }] });

// A change to a subagent's entry; one this page never heard work, such as a
// thread opened from the Work list, gets an entry by it.
function touch(ks: Task[], name: string, f: (k: Task) => Task): Task[] {
  return ks.some((k) => k.name === name) ? ks.map((k) => (k.name === name ? f(k) : k)) : [...ks, f({ name, text: "", tools: [], words: "", done: true })];
}

// Follows a device sign-in until the harness says it's done or failed, or
// the code runs out, and says which in the chat.
async function followSignin(harness: string, code: string, note: (text: string) => void) {
  const end = Date.now() + 16 * 60_000;
  while (Date.now() < end) {
    await new Promise((r) => setTimeout(r, 3000));
    const r = await api("api/signin/progress").catch(() => undefined);
    if (!r?.ok) continue;
    const s = ((await r.json()) as Record<string, { state?: string; code?: string; error?: string | null }>)[harness];
    // Another sign-in started since: that one says how it went.
    if (!s || s.code !== code) return;
    if (s.state === "done") {
      const next = harness === "omp" ? "/model picks one of its ChatGPT models (openai-codex/…)." : "/harness codex uses it.";
      return note(`${harness === "omp" ? "omp" : "Codex"} is signed in. ${next}`);
    }
    if (s.state === "failed") return note(`The ${harness} sign-in failed: ${s.error ?? "no reason given"}.`);
  }
}

// A subagent's work as it comes, and its start and end; work before the
// page heard it started still belongs to it.
export function taskEvent(ks: Task[], e: Ev): Task[] {
  const name = e.subagentRunId!;
  const k = ks.find((k) => k.name === name);
  const t: Task = k ?? { name, text: "", tools: [], words: "" };
  let next = t;
  switch (e.type) {
    case "CUSTOM":
      if (e.name === "libri.metrics" || e.name === "libri.usage") next = { ...t, metrics: { ...t.metrics, ...(e.value as Metrics) } };
      else if (e.name === "libri.tool_result") {
        const r = e.value as { toolCallId: string; result: unknown };
        next = { ...t, tools: t.tools.map((tool) => tool.id === r.toolCallId ? { ...tool, structured: r.result } : tool) };
      }
      break;
    case "SUBAGENT_STARTED":
      next = { ...t, text: e.description ?? t.text, title: e.name !== name ? e.name : t.title, done: false, stopping: t.done ? false : t.stopping };
      break;
    case "SUBAGENT_FINISHED":
      if (!k) return ks;
      next = { ...t, done: true, stopping: false };
      break;
    case "TEXT_MESSAGE_CONTENT":
      next = { ...t, words: t.words + (e.delta ?? "") };
      break;
    case "REASONING_MESSAGE_CONTENT":
      next = { ...t, thinking: (t.thinking ?? "") + (e.delta ?? "") };
      break;
    case "TOOL_CALL_START":
      next = { ...t, words: "", thinking: undefined, tools: [...t.tools, { id: e.toolCallId!, name: e.toolCallName!, input: undefined, args: "", began: e.timestamp, before: t.words.trim() || undefined, thought: t.thinking?.trim() ? t.thinking : undefined }] };
      break;
    case "TOOL_CALL_ARGS":
      next = { ...t, tools: t.tools.map((x) => (x.id === e.toolCallId ? { ...x, args: (x.args ?? "") + (e.delta ?? "") } : x)) };
      break;
    case "TOOL_CALL_END": {
      const call = t.tools.find((x) => x.id === e.toolCallId);
      if (!call) break;
      const input = parse(call.args ?? "");
      // A subagent hands its report back in a call; its page shows it as its last words.
      next =
        call.name === "SubagentHandback"
          ? { ...t, words: handback(call.before ?? "", input), tools: t.tools.filter((x) => x !== call) }
          : { ...t, tools: t.tools.map((x) => (x === call ? { ...x, input, args: undefined } : x)) };
      break;
    }
    case "TOOL_CALL_RESULT":
      // A call begun before this page was listening is only in the log.
      next = t.tools.some((x) => x.id === e.toolCallId) ? { ...t, tools: t.tools.map((x) => (x.id === e.toolCallId ? { ...x, result: String(e.content ?? ""), partial: undefined, error: !!e.metadata?.error, took: took(x, e) } : x)) } : { ...t, unheard: (t.unheard ?? 0) + 1 };
      break;
    case "ACTIVITY_SNAPSHOT": {
      if (e.activityType !== "tool_output") return ks;
      const o = e.content as { toolCallId: string; text: string };
      next = { ...t, tools: t.tools.map((x) => (x.id === o.toolCallId && x.result === undefined ? { ...x, partial: o.text } : x)) };
      break;
    }
  }
  return k ? ks.map((x) => (x === k ? next : x)) : [...ks, next];
}

// A subagent's id, which its Agent call's result names and hippo keeps its
// chat by; and Claude Code's news of a task done, with its report.
const agentOf = (result: string) => /^agentId: (\w+)/m.exec(result)?.[1];
// What Claude Code says in place of a report it handed back on its own.
export const POINTER = /^This agent's report was delivered to you/;

/** Markdown as a line of plain text: its first line that says anything, the
 * marks taken off. A code fence's own line says nothing. */
export const firstLine = (s: string) =>
  (s.split("\n").find((l) => l.trim() && !/^\s*(```|~~~)\S*\s*$/.test(l)) ?? "")
    .replace(/^\s*((#+|[-*+]|\d+[.)])\s+|>\s*)*/, "")
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/(?<![\w*~])(\*\*?|__?|~~)(?=\S)(.+?)(?<=\S)\1(?![\w*~])/g, "$2")
    .replaceAll("`", "");
const NEWS = /^<task-notification>[\s\S]*?<task-id>(\w+)<\/task-id>[\s\S]*?(?:<result>([\s\S]*?)<\/result>|$)/;

// A subagent hands its report back in a call; its page shows it as its
// last words.
const handback = (words: string, input: unknown) => {
  const report = (input as { message?: unknown } | undefined)?.message;
  return typeof report === "string" ? (words.trim() ? `${words}\n\n${report}` : report) : words;
};

// What a tell's bubble says, from the line the host logged: the words, or
// the files' names with none, as the page shows it when sending. The files
// named ahead and any text inlined after are the model's.
/** The device mark the host puts before the person's words when they
 * change device (libri/src/origin.rs), taken off for the bubble. */
export const unmarked = (line: string) => line.replace(/^\(from (?:android|desktop|web-phone|web-desktop|other)\) /, "");

export function toldWords(line: string) {
  const [, sent, rest = line] = /^\(sent ((?:a photo|a file): [\s\S]*?)\)(?:$| |(?=\n))([\s\S]*)$/.exec(line) ?? [];
  const cut = rest.indexOf('\n<file name="');
  const words = (cut < 0 ? rest : rest.slice(0, cut)).trim();
  return words || sent?.split("; ").map((f) => f.replace(/^a (?:photo|file): /, "").replace(/, kept unseen$/, "")).join(", ") || "";
}

/** A subagent's whole chat as hippo zooms it, `{i}|{kind}: {text}` per
 * message, read as its page shows one: the task, then words and calls.
 * In a scratchpad's, `plain`, every line of the person's is their words. */
export function fromAgentChat(text: string, plain = false): Pick<Task, "text" | "tools" | "words"> {
  const t: Pick<Task, "text" | "tools" | "words"> = { text: "", tools: [], words: "" };
  const calls = new Map<string, Task["tools"][number]>();
  for (const m of text.split(/\n(?=\d+\|)/)) {
    // `tool [id]` and `echo [id]` name the call a result answers; a log
    // from before ids pairs by order.
    const [, i, kind, key, kept] = /^(\d+)\|(\w+)(?: \[([^\]]*)\])?: ([\s\S]*)$/.exec(m.trim()) ?? [];
    if (!kind) continue;
    // The host indents an entry's further lines: only it starts a line.
    const body = kept.replace(/\n {2}/g, "\n");
    if (kind === "user" && !t.text && !plain) t.text = body;
    else if (kind === "file") {
      t.tools.push({ id: `a-${i}`, name: "", input: null, file: body, before: t.words.trim() || undefined });
      t.words = "";
    } else if (kind === "user") {
      const told = plain ? body : /^\(From the person, typed on your page\) ([\s\S]*)$/.exec(body)?.[1];
      if (told === undefined) continue;
      t.tools.push({ id: `a-${i}`, name: "", input: null, told: toldWords(unmarked(told)), before: t.words.trim() || undefined });
      t.words = "";
    } else if (kind === "tool") {
      const [, name, rest] = /^(\S+)\s?([\s\S]*)$/.exec(body) ?? [, body, ""];
      if (name === "SubagentHandback") {
        t.words = handback(t.words, parse(rest));
        continue;
      }
      const tool = { id: `a-${i}`, name, input: parse(rest), before: t.words.trim() || undefined };
      t.tools.push(tool);
      // An id a harness reuses across turns names the newest call.
      if (key) calls.set(key, tool);
      t.words = "";
    } else if (kind === "echo") {
      const keyed = key ? calls.get(key) : undefined;
      const open = keyed?.result === undefined && keyed ? keyed : t.tools.find((x) => x.name && x.result === undefined);
      if (open) open.result = body;
    } else if (kind === "from") {
      // A row, not a bubble: a bubble is the person's own words.
      const [, who, reply, said] = /^(.+?)( \(reply\))?: ([\s\S]*)$/.exec(body) ?? [, "a worker", , body];
      t.tools.push({ id: `a-${i}`, name: `${reply ? "Reply" : "Message"} from ${who}`, input: null, result: said, before: t.words.trim() || undefined });
      t.words = "";
    } else if (kind === "talk") t.words += (t.words ? "\n\n" : "") + body;
  }
  return t;
}


/** What one shown thing does to the talk: the person's words, a call or
 * its result, words on the way, the answer, a picture or a file. */
// The background jobs a turn's record last counted.
/** The index of the last of `xs` that `f` holds for, or -1. */
function lastWhere<T>(xs: T[], f: (x: T) => boolean): number {
  for (let i = xs.length - 1; i >= 0; i--) if (f(xs[i])) return i;
  return -1;
}

function jobsOf(shown: Ev[]): string[] {
  const last = shown[lastWhere(shown, (e) => e.type === "ACTIVITY_SNAPSHOT" && e.activityType === "jobs")];
  return (last?.content as { names: string[] } | undefined)?.names ?? [];
}

/** The interrupts the record's last run ended with: the asks still open. */
function asksOf(shown: Ev[]): Ask[] {
  const last = shown[lastWhere(shown, (e) => e.type === "RUN_STARTED" || e.type === "RUN_FINISHED")];
  return last?.outcome?.type === "interrupt" ? last.outcome.interrupts.map(askOf) : [];
}

/** How long a call ran, where its start and result both carry the host's time. */
const took = (t: Tool, e: Ev) => (t.began !== undefined && e.timestamp !== undefined ? Math.max(0, e.timestamp - t.began) : undefined);

const at = (ms: Msg[], i: number, m: Msg) => [...ms.slice(0, i), m, ...ms.slice(i + 1)];

/** What one AG-UI event does to the talk: the person's words, words and
 * thinking as they stream, calls and their results, a picture or a file. */
export function apply(ms: Msg[], e: Ev): Msg[] {
  if (e.subagentRunId) return ms;
  const last = ms.at(-1);
  const seg = (id?: string) => lastWhere(ms, (m) => m.seg === id);
  switch (e.type) {
    case "TEXT_MESSAGE_CHUNK":
      // The person's words, from this page or another door.
      if (e.role !== "user") return ms;
      // The server's echo of this page's own message proves it arrived,
      // whatever the send's own reply said: a phone that drops the
      // connection mid-send loses the reply, not the message.
      if (ms.some((m) => m.id === e.messageId))
        return ms.map((m) => (m.id === e.messageId ? { ...m, queued: false, failed: undefined } : m));
      {
        // A routine's, watch's or alert's framing shows as its note, as
        // fromHistory shows it, so live news is marked as from outside too.
        const note = framed(e.delta ?? "");
        if (note) return [...ms, { id: e.messageId!, role: "system", text: note, date: new Date() }];
      }
      return [...ms, { id: e.messageId!, role: "user", text: e.delta ?? "", date: new Date() }];
    case "TEXT_MESSAGE_START": {
      const fresh: Msg = { id: e.messageId!, role: "assistant", text: "", seg: e.messageId, live: true, date: new Date() };
      // A draft word for word, or a message from no turn, stands apart.
      if (e.metadata?.plain || e.metadata?.aside) return [...ms, { ...fresh, plain: !!e.metadata.plain, live: false }];
      if (!last?.live) return [...ms, fresh];
      // Words after words already ended: those were an answer of their own.
      if (last.text.trim()) return [...ms.slice(0, -1), { ...last, live: false }, fresh];
      // The words join the turn under its id: a new one would draw it, and
      // the cards in it, again.
      return [...ms.slice(0, -1), { ...last, seg: e.messageId }];
    }
    case "TEXT_MESSAGE_CONTENT": {
      const i = seg(e.messageId);
      if (i < 0) return ms;
      const m = ms[i];
      const keys = e.key && !m.keys?.includes(e.key) ? [...(m.keys ?? []), e.key] : m.keys;
      return at(ms, i, { ...m, text: m.text + (e.delta ?? ""), keys });
    }
    case "TEXT_MESSAGE_END": {
      const i = seg(e.messageId);
      // Words the turn never said as its answer go; its calls stay.
      if (i < 0 || !e.metadata?.discarded) return ms;
      const m = ms[i];
      return m.tools || m.thinking ? at(ms, i, { ...m, text: "" }) : [...ms.slice(0, i), ...ms.slice(i + 1)];
    }
    case "REASONING_MESSAGE_CONTENT":
      if (last?.live) return [...ms.slice(0, -1), { ...last, thinking: (last.thinking ?? "") + (e.delta ?? "") }];
      return [...ms, { id: e.messageId!, role: "assistant", text: "", thinking: e.delta ?? "", live: true, date: new Date() }];
    case "TOOL_CALL_START": {
      const tool: Tool = { id: e.toolCallId!, name: e.toolCallName!, input: undefined, args: "", heard: true, row: e.key, began: e.timestamp };
      if (last?.live) {
        // Words said before a call are a message of their own and the call
        // opens the next, so the turn reads in the order it went, live as
        // after a reload (F13).
        if (leadIn(last.text)) {
          return [...ms.slice(0, -1), { ...last, live: false, progress: true }, { id: `w-${tool.id}`, role: "assistant", text: "", tools: [tool], live: true, date: new Date() }];
        }
        if (last.thinking?.trim()) tool.thought = last.thinking;
        return [...ms.slice(0, -1), { ...last, text: "", thinking: undefined, tools: [...(last.tools ?? []), tool] }];
      }
      return [...ms, { id: `w-${tool.id}`, role: "assistant", text: "", tools: [tool], live: true, date: new Date() }];
    }
    case "TOOL_CALL_ARGS":
      return onTool(ms, e.toolCallId!, (t) => ({ ...t, args: (t.args ?? "") + (e.delta ?? "") }));
    case "TOOL_CALL_END":
      return onTool(ms, e.toolCallId!, (t) => ({ ...t, input: parse(t.args ?? ""), args: undefined }));
    case "TOOL_CALL_RESULT": {
      const text = String(e.content ?? "");
      return onTool(ms, e.toolCallId!, (t) => ({ ...t, result: text, partial: undefined, error: !!e.metadata?.error, agent: agentOf(text) ?? t.agent, took: took(t, e) }));
    }
    case "ACTIVITY_SNAPSHOT": {
      // A running call's output so far, whole each time, until its result.
      if (e.activityType !== "tool_output") return ms;
      const o = e.content as { toolCallId: string; text: string };
      return onTool(ms, o.toolCallId, (t) => (t.result === undefined ? { ...t, partial: o.text } : t));
    }
    case "SUBAGENT_FINISHED": {
      // The report goes to the Agent call that started it.
      const report = typeof e.result === "string" ? e.result : "";
      if (!report || POINTER.test(report)) return ms;
      return onTool(ms, e.subagentRunId ?? "", (t) => ({ ...t, report }));
    }
    case "RUN_FINISHED":
      // The turn is over: what it said stands. An interrupt only pauses it.
      if (e.outcome?.type === "interrupt") return ms;
      return ms.some((m) => m.live) ? ms.map((m) => (m.live ? { ...m, live: false } : m)) : ms;
    case "CUSTOM": {
      if (e.name === "libri.metrics" || e.name === "libri.usage") {
        const index = ms.map((m) => m.role === "assistant" && !!m.live).lastIndexOf(true);
        return index < 0 ? ms : at(ms, index, { ...ms[index], metrics: { ...ms[index].metrics, ...(e.value as Metrics) } });
      }
      const v = e.value as { name?: string; to?: string; emoji?: string };
      if (e.name === "picture") return [...ms, { id: newId(), role: "assistant", text: "", picture: fileUrl(v.name!), date: new Date() }];
      if (e.name === "file") return [...ms, { id: newId(), role: "assistant", text: "", file: v.name, date: new Date() }];
      if (e.name === "react") return ms.map((m) => (m.id === v.to || m.seg === v.to ? { ...m, reaction: v.emoji } : m));
      if (e.name === "libri.tool_result") {
        const r = e.value as { toolCallId: string; result: unknown };
        return onTool(ms, r.toolCallId, (t) => ({ ...t, structured: r.result }));
      }
      if (e.name === "queued") {
        const q = e.value as { id: string; on: boolean };
        return ms.map((m) => (m.id === q.id ? { ...m, queued: q.on } : m));
      }
      if (e.name === "row") {
        // hippo took a row: the message drawn live that holds it is named by
        // it from now, as a reload would name it. Rows come in order, so the
        // first a message hears is the one it begins with.
        const r = e.value as { key: string; row: number; heard?: string[] };
        const holds = (m: Msg) => r.heard?.includes(m.id) || m.keys?.includes(r.key) || m.tools?.some((t) => t.row === r.key);
        return ms.map((m) => (m.row === undefined && holds(m) ? { ...m, row: r.row } : m));
      }
      return ms;
    }
    default:
      return ms;
  }
}

const onTool = (ms: Msg[], id: string, f: (t: Tool) => Tool): Msg[] =>
  ms.map((m) => (m.tools?.some((t) => t.id === id) ? { ...m, tools: m.tools.map((t) => (t.id === id ? f(t) : t)) } : m));

// The conversation as hippo keeps it. A seat's history holds its work too,
// a `tool` line per call (`Name {input}`) and an `echo` per result, in the
// order they came: they gather into the answer they led to, as in a live
// turn, with the words between them kept above the call they led into.
// Libri marks words sent mid-turn for the model, and hippo logs them so;
// the page shows them as the person wrote them, and so knows them again.
const STEERED = /^\(sent while you were working\) /;

// A seat answers a background job's news that tells the person nothing new
// with NO_REPLY alone (OpenClaw's convention), and a watch with nothing to
// tell with "(nothing new)" (main.rs, QUIET); Libri never sends either,
// and the page hides them, and while one streams in, its start.
const QUIETS = ["NO_REPLY", "(nothing new)"];
export const quiet = (text: string, live = false) =>
  QUIETS.some((q) => (live ? text.trim() && q.startsWith(text.trim()) : text.trim() === q));
/** Words that stand as a message before a call, unless they are a quiet
 * answer: one said mid-turn (a worker's report answered, then the person's
 * message worked on) must not show above the calls that follow it. */
const leadIn = (text: string) => (text.trim() && !quiet(text) ? text : undefined);
/** A message of the model's own turn: not a draft, a sent picture or file. */
const own = (m: Msg) => m.role === "assistant" && !m.plain && !m.picture && !m.file;

// What a seat is told by its routines, watches and messages comes
// as words from the person, framed by Libri (main.rs, `who`); the page
// shows each as the note Libri logged for it.
const FRAMED: [RegExp, string][] = [
  [/^Routine \S+ set, due now: ([\s\S]*)$/, "routine"],
  [/^Watch \S+ set, fired now [\s\S]*?answer only [^:]*: ([\s\S]*)$/, "watch"],
  [/^From other agents, quoted in <message>[\s\S]*?a routine's answer: ([\s\S]*)$/, "message"],
];
const framed = (text: string) => {
  for (const [re, kind] of FRAMED) {
    const m = re.exec(text);
    if (m) return `(${kind}) ${m[1]}`;
  }
};

/** What a result's row key adds to its call's, as hippo.rs keeps it. */
const RESULT = ".result";
/** show_page by any harness's name for it: bare, or an MCP server's `mcp__libri__show_page`. */
const SHOW_PAGE = /(^|__)show_page$/;

/** The chat a sync leaves: hippo's rows, then what `record` holds that they
 * lack, each thing after the row it came after. An event names the row it
 * will be, so hippo and the record never need comparing by their words; this
 * is UniiChat's `Chat.live`. `was` lends only the thumbnails of the person's
 * files, which no row holds yet. */
export function fromHistory(said: Said[], record: TurnRecord | undefined, turning: boolean, was: Msg[] = []): Msg[] {
  const rows = new Set(said.flatMap((m) => (m.src ? [m.src.e] : [])));
  const held = record?.shown ?? [];
  // A call stays in the record until its result does, so a row can take the
  // id the stream's result will name it by.
  const ids = new Map(held.filter((e) => e.type === "TOOL_CALL_START" && e.key && rows.has(e.key)).map((e) => [e.key!, e.toolCallId!]));
  const lacking = held.filter((e) => !e.key || !rows.has(e.key));
  // An event past the history's window goes first, unless a later one has
  // a row to stand on, when it is the newest and goes last.
  const anchored = lastWhere(lacking, (e) => rows.has(e.after ?? ""));
  const after = new Map<string, Ev[]>();
  const oldest: Ev[] = [];
  const newest: Ev[] = [];
  lacking.forEach((e, n) => {
    if (rows.has(e.after ?? "")) after.set(e.after!, [...(after.get(e.after!) ?? []), e]);
    else (n < anchored ? oldest : newest).push(e);
  });
  let out: Msg[] = [];
  // Each call by its row, for its result's row to name: hippo.rs result_key.
  const calls = new Map<string, Tool>();
  const take = (events: Ev[] = []) => {
    for (const e of events) {
      // A turn's calls are in hippo as it goes; the rest join them.
      const last = out.at(-1);
      if (last?.role === "assistant" && last.tools) last.live = true;
      out = apply(out, e);
    }
  };
  const row = (m: Said) => {
    const last = out.at(-1);
    const turn = last && own(last) ? last : undefined;
    if (m.kind === "tool") {
      const [, name, rest] = /^(\S+)\s?([\s\S]*)$/.exec(m.text) ?? [, m.text, ""];
      const tool: Tool = { id: ids.get(m.src?.e ?? "") ?? `h-${m.i}`, name, input: parse(rest), row: m.src?.e, began: Date.parse(m.date) };
      if (m.src) calls.set(m.src.e, tool);
      // A call joins the calls before it; after words, it opens a message
      // of its own, so the turn reads in the order it went (F13).
      if (!turn || leadIn(turn.text)) {
        if (turn) turn.progress = true;
        out.push({ id: `h-${m.i}`, row: m.i, role: "assistant", text: "", tools: [tool], date: new Date(m.date) });
        return;
      }
      if (turn.thinking?.trim()) tool.thought = turn.thinking;
      turn.text = "";
      turn.thinking = undefined;
      turn.tools = [...(turn.tools ?? []), tool];
      turn.date = new Date(m.date);
      return;
    }
    if (m.kind === "work") {
      // A subagent's report as hippo keeps it, `[id]\n{report}\n\nFull chat: …`.
      // A Libri agent's (`[Ada] report\n…`) has no call to join: the
      // model passes it on in its own words.
      const [, id, report] = /^\[(\w+)\]\n([\s\S]*?)(?:\n\nFull chat: hippo zoom \w+)?$/.exec(m.text) ?? [];
      const call = id && out.flatMap((o) => o.tools ?? []).reverse().find((t) => t.agent === id);
      if (call) call.report = report.trim();
      if (turn) turn.woke = turn.tools?.length ?? 0;
      return;
    }
    if (m.kind === "echo") {
      // A subagent's report finds the Agent call that started it, by the
      // agent's id; other news of Claude Code's own isn't the model's.
      const news = NEWS.exec(m.text);
      if (news) {
        const call = out.flatMap((o) => o.tools ?? []).reverse().find((t) => t.agent === news[1]);
        if (call && news[2] !== undefined && !POINTER.test(news[2])) call.report = news[2].trim();
        return;
      }
      // A result's row names its call's; rows kept before that come in
      // the order of their calls.
      const key = m.src?.e;
      const named = key?.endsWith(RESULT) ? calls.get(key.slice(0, -RESULT.length)) : undefined;
      const open = named ?? (key?.endsWith(RESULT) ? undefined : turn?.tools?.find((t) => t.result === undefined));
      // Hippo dates its rows to the second, so a kept call's time is that coarse.
      const ended = Date.parse(m.date);
      if (open) Object.assign(open, { structured: m.display?.version === 1 ? m.display.result : undefined, result: m.text, agent: agentOf(m.text), took: open.began !== undefined && ended >= open.began ? ended - open.began : undefined });
      return;
    }
    if (m.kind === "talk" && quiet(m.text)) {
      // A routine that found nothing leaves no trace: not its work, nor the
      // note that started it. A turn a worker's report woke again keeps what
      // it had when woken: the calls after it go. The turn is the model's
      // messages since anyone else's last.
      let start = out.length;
      while (start > 0 && own(out[start - 1])) start--;
      let w = out.length - 1;
      while (w >= start && out[w].woke === undefined) w--;
      if (w < start) w = -1;
      if (turn && w >= 0) {
        const woken = out[w];
        if ((woken.tools?.length ?? 0) > woken.woke! || w + 1 < out.length) {
          out.splice(w + 1);
          Object.assign(woken, { thinking: undefined, tools: woken.tools?.slice(0, woken.woke) });
        }
        return;
      }
      if (turn?.tools && !turn.text.trim()) out.splice(start);
      if (out.at(-1)?.role === "system") out.pop();
      return;
    }
    // The answer after the calls, once.
    if (m.kind === "talk" && (turn?.tools || turn?.live) && !turn.text && !SENT.test(m.text) && !DRAFT.test(m.text)) {
      Object.assign(turn, { metrics: m.display?.version === 1 ? m.display.metrics : undefined, text: m.text, date: new Date(m.date) });
      return;
    }
    // What Libri sent is kept, and hippo logs it by name.
    const note = m.kind === "user" ? framed(m.text) : undefined;
    if (note) {
      out.push({ id: `h-${m.i}`, row: m.i, role: "system", text: note, date: new Date(m.date) });
      return;
    }
    const sent = m.kind === "talk" ? SENT.exec(m.text) : null;
    out.push({
      id: `h-${m.i}`,
      row: m.i,
      role: ROLE[m.kind],
      metrics: m.display?.version === 1 ? m.display.metrics : undefined,
      ...(m.kind === "user" ? theirs(m.text) : { text: sent ? "" : m.text }),
      plain: m.kind === "talk" && DRAFT.test(m.text),
      picture: sent?.[1] === "picture" ? fileUrl(sent[2]) : undefined,
      file: sent?.[1] === "file" ? sent[2] : undefined,
      reaction: m.reaction ?? undefined,
      quote: m.kind === "user" ? (m.quote ?? undefined) : undefined,
      from: m.kind === "user" ? m.from : undefined,
      date: new Date(m.date),
    });
  };
  take(oldest);
  for (const m of said) {
    row(m);
    take(after.get(m.src?.e ?? ""));
  }
  take(newest);
  keepIds(out, was);
  out.forEach((m, n) => {
    // Only the newest of a running turn goes on growing.
    m.live = turning && n === out.length - 1 && m.role === "assistant" && !m.plain && !!(m.live || m.tools);
    if (m.role === "user" && !m.files?.length) m.files = was.find((w) => w.id === m.id)?.files ?? m.files;
  });
  return out;
}

/** A sync leaves the calls the page already shows under the ids it shows
 * them by, and their turn too, matched by the call's id or its hippo row:
 * a new id would draw a card again, and a card drawn again loses what the
 * person did in it. What only this page heard (that it heard the call, its
 * whole result) stays with it. */
function keepIds(out: Msg[], was: Msg[]) {
  const byId = new Map<string, [Msg, Tool]>();
  const byRow = new Map<string, [Msg, Tool]>();
  for (const m of was)
    for (const t of m.tools ?? []) {
      byId.set(t.id, [m, t]);
      if (t.row) byRow.set(t.row, [m, t]);
    }
  const taken = new Set(out.map((m) => m.id));
  for (const m of out) {
    let from: Msg | undefined;
    for (const t of m.tools ?? []) {
      const [owner, prev] = byId.get(t.id) ?? (t.row ? byRow.get(t.row) : undefined) ?? [];
      if (!prev || !owner) continue;
      Object.assign(t, { id: prev.id, heard: t.heard ?? prev.heard, structured: t.structured ?? prev.structured });
      from ??= owner;
    }
    if (from && from.id !== m.id && !taken.has(from.id)) {
      taken.delete(m.id);
      m.id = from.id;
      taken.add(from.id);
    }
  }
}

// The person's words as hippo has them, "[image]\n(sent while you were
// working) (sent a photo: a.png; a file: b.txt) words", as the page shows
// them: the words, under the files kept in the library.
function theirs(text: string): Pick<Msg, "text" | "files"> {
  text = text.replace(/^(\[(image|document)\]\n)+/, "").replace(STEERED, "");
  const sent = /^\(sent ((?:a (?:photo|file): [^;]+?(?:, kept unseen)?(?:; )?)+)\) ?([\s\S]*)$/.exec(text);
  if (!sent) return { text };
  const files = sent[1].split("; ").map((f) => {
    const [, kind, name] = /^a (photo|file): (.+?)(?:, kept unseen)?$/.exec(f) ?? [, "file", f];
    return { name, image: kind === "photo" ? fileUrl(name) : undefined };
  });
  return { text: sent[2], files };
}

// A call's input as the model wrote it; one cut short in hippo stays words.
function parse(input: string): unknown {
  if (!input) return {};
  try {
    return JSON.parse(input);
  } catch {
    return input;
  }
}

// The model reads photos up to 3.75 MB (IMAGE_MAX in main.rs) in JPEG, PNG,
// GIF or WebP. A phone photo is often bigger, or HEIC, so the page redraws
// it as a JPEG no longer than 2048 px on its long side, which the model
// downscales to anyway. That copy goes along as the photo's view; the
// original is what Libri keeps.
const views = new WeakMap<File, File>();
const PHOTO_MAX = 3_500_000;
const PHOTO_TYPES = ["image/jpeg", "image/png", "image/gif", "image/webp"];

async function fit(file: File): Promise<File | undefined> {
  if (!file.type.startsWith("image/") || (file.size <= PHOTO_MAX && PHOTO_TYPES.includes(file.type))) return;
  // One the browser can't draw (a TIFF, a RAW) is kept unseen.
  const bitmap = await createImageBitmap(file).catch(() => undefined);
  if (!bitmap) return;
  const scale = Math.min(1, 2048 / Math.max(bitmap.width, bitmap.height));
  const canvas = new OffscreenCanvas(Math.round(bitmap.width * scale), Math.round(bitmap.height * scale));
  canvas.getContext("2d")!.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close();
  const blob = await canvas.convertToBlob({ type: "image/jpeg", quality: 0.85 });
  return new File([blob], file.name.replace(/\.[^.]*$/, "") + ".jpg", { type: "image/jpeg" });
}

/** Holds a chosen file in the composer until it goes out with the message. */
/** A file sent ahead of the words that come with it, answering its id. */
export function upload(f: File) {
  const body = new FormData();
  body.append("file", f);
  const view = views.get(f);
  if (view) body.append("view", view);
  return api("api/upload", { method: "POST", body, signal: AbortSignal.timeout(UPLOAD_WAIT) }).catch(() => undefined);
}

export const attachments: AttachmentAdapter = {
  accept: "*",
  async add({ file }) {
    const view = await fit(file);
    if (view) views.set(file, view);
    return {
      id: newId(),
      type: file.type.startsWith("image/") ? "image" : "document",
      name: file.name,
      contentType: file.type,
      file,
      status: { type: "requires-action", reason: "composer-send" },
    };
  },
  async remove() {},
  async send(a) {
    return { ...a, status: { type: "complete" }, content: [] };
  },
};

