// The scratchpads, shown as Temp: threads the host keeps and runs, a
// worker's in all but that nothing of them reaches the memory. Each Temp tile
// holds one by its id (the tile's arg), a store of its own here; a tile with
// none holds the host's current one, under the key "". Their state lives
// outside their tiles, so a turn goes on with the tile closed and its answer
// is there when it opens again; the host's log is what stands, and what the
// stream brings between two reads of it shows on top.
import { useSyncExternalStore } from "react";
import { api } from "./api";
import { fromAgentChat, taskEvent, toldWords, upload, askOf, resume, type Answers, type Ask, type Ev, type Interrupt, type Task } from "./chat";

/** What the host's ids for a scratchpad start with (scratchpad.rs). */
export const PREFIX = "scratchpad-";

export type Log = Pick<Task, "text" | "tools" | "words">;

type State = {
  /** The open scratchpad; null with none, as before the first words. */
  id: string | null;
  log: Log;
  /** What the stream brought since the log was read. */
  heard?: Task;
  working: boolean;
  stopping: boolean;
  /** The person's words waiting for the run, from any client. */
  queued: string[];
  memory: boolean;
  asks: Ask[];
  /** Whether the host has answered yet. */
  loaded: boolean;
  /** An answer that came while the column was closed. */
  unread: boolean;
  open: boolean;
  failed?: string;
};

const EMPTY: Log = { text: "", tools: [], words: "" };

const FIRST: State = { id: null, log: EMPTY, working: false, stopping: false, queued: [], memory: true, asks: [], loaded: false, unread: false, open: false };

/** One Temp's store. Reads overlap on a busy stream, and only the newest
 * stands. What the person does counts as newer than any read under way: a
 * read begun before New, Stop or the Memory switch must not bring back what
 * they changed. */
type Store = { state: State; listeners: Set<() => void>; reads: number };
const stores = new Map<string, Store>();
function storeOf(key: string): Store {
  let s = stores.get(key);
  if (!s) stores.set(key, (s = { state: { ...FIRST, id: key || null }, listeners: new Set(), reads: 0 }));
  return s;
}
function set(key: string, next: Partial<State>) {
  const s = storeOf(key);
  s.state = { ...s.state, ...next };
  s.listeners.forEach((l) => l());
}
/** Where a Temp's routes are: `?id=` for a Temp of its own. */
const at = (path: string, key: string) => (key ? `${path}?id=${encodeURIComponent(key)}` : path);

/** A Temp's store as it stands, outside React; "" the host's current one. */
export const scratchpadNow = (key = "") => storeOf(key).state;
export function useScratchpad(key = "") {
  const s = storeOf(key);
  return useSyncExternalStore(
    (l) => (s.listeners.add(l), () => void s.listeners.delete(l)),
    () => s.state,
  );
}

export const setScratchpadOpen = (open: boolean, key = "") => set(key, open ? { open, unread: false } : { open });

const longer = (a = "", b = "") => (b.length > a.length ? b : a);

/** A new Temp on the host, beside any open: its id, for a tile to hold. */
export async function newScratchpad(): Promise<string | undefined> {
  const r = await api("api/scratchpad/new", { method: "POST" }).catch(() => undefined);
  if (!r?.ok) return undefined;
  return ((await r.json()) as { id: string }).id;
}

/** Reads every Temp a tile holds, as a word from the host that one changed
 * asks. */
export function readScratchpads() {
  if (!stores.size) storeOf("");
  for (const key of stores.keys()) void readScratchpad(key);
}

export async function readScratchpad(key = "") {
  const s = storeOf(key);
  const n = ++s.reads;
  const r = await api(at("api/scratchpad", key)).catch(() => undefined);
  if (!r?.ok || n !== s.reads) return;
  const state = s.state;
  const set_ = (next: Partial<State>) => set(key, next);
  const j = (await r.json()) as { id: string | null; text: string; working: boolean; words: string; thinking: string; stopping: boolean; queued: string[]; memory: boolean; asks: Interrupt[] };
  if (n !== s.reads) return;
  const ended = state.working && !j.working;
  const was = j.id === state.id ? state.heard : undefined;
  set_({
    id: j.id,
    log: j.id ? fromAgentChat(j.text, true) : EMPTY,
    // At rest the log has all of it, and thinking was only for the moment.
    // Under a run the host's word holds what the stream brought before this
    // page was listening.
    heard: j.working && (was || j.words || j.thinking) ? { name: j.id ?? PREFIX, text: "", tools: [], ...was, words: longer(was?.words, j.words), thinking: longer(was?.thinking, j.thinking) || undefined } : undefined,
    working: j.working,
    stopping: j.stopping,
    queued: j.queued.map(toldWords),
    memory: j.memory,
    asks: j.asks.map(askOf),
    loaded: true,
    unread: state.unread || (ended && !state.open),
  });
}

/** A step of a run, off the stream: to the Temp whose run it is. */
export function scratchpadEvent(e: Ev) {
  const key = [...stores.keys()].find((k) => stores.get(k)!.state.id === e.subagentRunId);
  // One closed, here or on another client, or opened and not yet read.
  if (key === undefined) return readScratchpads();
  const state = storeOf(key).state;
  const before = state.heard ? [state.heard] : [];
  const unheard = state.heard?.unheard;
  const [heard] = taskEvent(before, e);
  set(key, { heard });
  if (heard?.unheard !== unheard) void readScratchpad(key);
}

/** The person's words, opening a scratchpad if none is. True once the host
 * has them; under a run they wait for its next step. */
export async function tellScratchpad(text: string, files: File[], key = "") {
  const set_ = (next: Partial<State>) => set(key, next);
  set_({ failed: undefined });
  const ids: string[] = [];
  for (const f of files) {
    const up = await upload(f);
    if (!up?.ok) {
      set_({ failed: "That file didn't reach the host." });
      return false;
    }
    ids.push(((await up.json()) as { id: string }).id);
  }
  const r = await api(at("api/scratchpad", key), { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ text, files: ids }) }).catch(() => undefined);
  if (!r?.ok) {
    set_({ failed: (r?.status === 409 && (await r.text())) || "That didn't reach the host." });
    return false;
  }
  set_({ working: true });
  void readScratchpad(key);
  return true;
}

export async function stopScratchpad(key = "") {
  storeOf(key).reads++;
  set(key, { stopping: true });
  const r = await api(at("api/scratchpad/stop", key), { method: "POST" }).catch(() => undefined);
  // A stop refused because the turn ended as it was pressed leaves no read
  // under way to say so; read the host, as the Memory switch does.
  if (!r?.ok) {
    set(key, { stopping: false });
    void readScratchpad(key);
  }
}

/** Answers one of its asks: the card is busy until the host has the
 * answer, and goes if the turn took the ask back. */
export async function replyScratchpad(id: string, yes: boolean, answers?: Answers, always?: boolean, key = "") {
  const asks = () => storeOf(key).state.asks;
  const busy = (on: boolean) => set(key, { asks: asks().map((a) => (a.id === id ? { ...a, busy: on } : a)) });
  busy(true);
  const r = await resume(id, yes, answers, always);
  if (r?.status === 409) return set(key, { asks: asks().filter((a) => a.id !== id) });
  if (!r?.ok) return busy(false);
  void readScratchpad(key);
}

/** Removes a Temp and all of it, once promoted or when its tile closes. */
export async function closeScratchpad(key = "") {
  storeOf(key).reads++;
  set(key, { id: key ? key : null, log: EMPTY, heard: undefined, working: false, stopping: false, queued: [], asks: [], failed: undefined });
  const r = await api(at("api/scratchpad", key), { method: "DELETE" }).catch(() => undefined);
  if (key) stores.delete(key);
  // A clear the host never took would come back with the next message:
  // show the chat it still holds, and say so.
  if (!r?.ok && !key) {
    await readScratchpad(key);
    set(key, { failed: "That didn't clear on the host." });
  }
}

/** Moves the scratchpad into the main chat as the talk it was, each
 * message as the main chat keeps its own; the host then has the pages read
 * the history again. True once it is there and the scratchpad gone. */
export async function promoteScratchpad(key = "") {
  set(key, { failed: undefined });
  const r = await api(at("api/scratchpad/promote", key), { method: "POST" }).catch(() => undefined);
  if (!r?.ok) {
    set(key, { failed: (r && r.status === 409 && (await r.text())) || "That didn't reach the main chat." });
    return false;
  }
  storeOf(key).reads++;
  set(key, { id: null, log: EMPTY, heard: undefined, working: false, stopping: false, queued: [], asks: [] });
  return true;
}

/** The memory switch, which every Temp shares (scratchpad.rs `MEMORY`). */
export async function setScratchpadMemory(on: boolean) {
  for (const key of stores.keys()) {
    storeOf(key).reads++;
    set(key, { memory: on });
  }
  const r = await api("api/scratchpad/memory", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ on }) }).catch(() => undefined);
  if (!r?.ok) readScratchpads();
}

/** The words of the talk and nothing else: what a brief to Work carries. */
export function wordsOf({ tools, words }: Log, name: string) {
  const said = (text?: string) => (text?.trim() ? [`${name}: ${text.trim()}`] : []);
  return [
    ...tools.flatMap((t) => [...said(t.before), ...(t.told !== undefined ? [`Me: ${t.told}`] : [])]),
    ...said(words),
  ].join("\n\n");
}
