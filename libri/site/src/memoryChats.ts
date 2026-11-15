// The named memories (memories.rs) and their chats. A memory's chat is the
// host's to keep and run, as Temp's is (scratchpad.ts), one per memory, so
// several stand side by side in a workspace's tiles; its words go to that
// memory alone. The store holds every memory's chat that a tile has read,
// by memory id, and what the stream brought between two reads of it.
import { useSyncExternalStore } from "react";
import { api } from "./api";
import { askOf, fromAgentChat, resume, taskEvent, toldWords, upload, type Answers, type Ask, type Ev, type Interrupt, type Task } from "./chat";

/** What the host's ids for a memory's chat start with (memories.rs). */
export const PREFIX = "memory-";

export type Memory = { id: string; name: string; instructions: string };

export type Log = Pick<Task, "text" | "tools" | "words">;

export type MemoryChat = {
  log: Log;
  heard?: Task;
  working: boolean;
  stopping: boolean;
  queued: string[];
  /** Its run's permission prompts and questions, waiting on the person. */
  asks: Ask[];
  loaded: boolean;
  /** Gone from the host: deleted here or on another client. */
  gone: boolean;
  failed?: string;
};

const EMPTY: Log = { text: "", tools: [], words: "" };
const FRESH: MemoryChat = { log: EMPTY, working: false, stopping: false, queued: [], asks: [], loaded: false, gone: false };

type State = { memories: Memory[] | undefined; chats: Record<string, MemoryChat> };
let state: State = { memories: undefined, chats: {} };
const listeners = new Set<() => void>();
function set(next: Partial<State>) {
  state = { ...state, ...next };
  listeners.forEach((l) => l());
}
function setChat(id: string, next: Partial<MemoryChat>) {
  set({ chats: { ...state.chats, [id]: { ...(state.chats[id] ?? FRESH), ...next } } });
}
const listen = (l: () => void) => {
  listeners.add(l);
  return () => void listeners.delete(l);
};
const now = () => state;

/** Every named memory, read once and again on each change. */
export function useMemories() {
  const s = useSyncExternalStore(listen, now);
  if (s.memories === undefined && !reading) void readMemories();
  return s.memories;
}

/** Memory `id`'s chat, read on first use. */
export function useMemoryChat(id: string) {
  const s = useSyncExternalStore(listen, now);
  const chat = s.chats[id];
  if (!chat && !asked.has(id)) {
    asked.add(id);
    setTimeout(() => void readMemoryChat(id));
  }
  return chat ?? FRESH;
}

/** The chats of memories `ids`, each read on first use: whether each works
 * or waits on the person, for the Workers tree. */
export function useMemoryChats(ids: string[]) {
  const s = useSyncExternalStore(listen, now);
  for (const id of ids)
    if (!s.chats[id] && !asked.has(id)) {
      asked.add(id);
      setTimeout(() => void readMemoryChat(id));
    }
  return s.chats;
}

/** The chats a tile asked for before the host answered. */
const asked = new Set<string>();
let reading = false;
export async function readMemories() {
  reading = true;
  const r = await api("api/memories").catch(() => undefined);
  reading = false;
  if (!r?.ok) return;
  const j = (await r.json()) as { memories: Memory[] };
  set({ memories: j.memories });
}

// Per memory: only the newest read stands, and what the person does counts
// as newer than any read under way.
const reads: Record<string, number> = {};
const longer = (a = "", b = "") => (b.length > a.length ? b : a);

export async function readMemoryChat(id: string) {
  const n = (reads[id] = (reads[id] ?? 0) + 1);
  const r = await api(`api/memories/${encodeURIComponent(id)}/chat`).catch(() => undefined);
  if (n !== reads[id]) return;
  if (r?.status === 404) return setChat(id, { gone: true, loaded: true, working: false });
  if (!r?.ok) return;
  const j = (await r.json()) as { id: string; text: string; working: boolean; words: string; thinking: string; stopping: boolean; queued: string[]; asks: Interrupt[] };
  if (n !== reads[id]) return;
  const was = state.chats[id]?.heard;
  setChat(id, {
    log: fromAgentChat(j.text, true),
    heard: j.working && (was || j.words || j.thinking) ? { name: j.id, text: "", tools: [], ...was, words: longer(was?.words, j.words), thinking: longer(was?.thinking, j.thinking) || undefined } : undefined,
    working: j.working,
    stopping: j.stopping,
    queued: j.queued.map(toldWords),
    asks: j.asks.map(askOf),
    loaded: true,
    gone: false,
  });
}

/** A step of a memory chat's run, off the stream. */
export function memoryChatEvent(e: Ev) {
  const id = e.subagentRunId!.slice(PREFIX.length);
  const chat = state.chats[id];
  if (!chat) return void (asked.has(id) && readMemoryChat(id));
  const before = chat.heard ? [chat.heard] : [];
  const unheard = chat.heard?.unheard;
  const [heard] = taskEvent(before, e);
  setChat(id, { heard });
  if (heard?.unheard !== unheard) void readMemoryChat(id);
}

/** The host says a memory's chat changed, or a memory came or went. */
export function memoryChatChanged(thread: string) {
  const id = thread.slice(PREFIX.length);
  if (state.chats[id]) void readMemoryChat(id);
  void readMemories();
}

export async function tellMemoryChat(id: string, text: string, files: File[]) {
  setChat(id, { failed: undefined });
  const ids: string[] = [];
  for (const f of files) {
    const up = await upload(f);
    if (!up?.ok) {
      setChat(id, { failed: "That file didn't reach the host." });
      return false;
    }
    ids.push(((await up.json()) as { id: string }).id);
  }
  const r = await api(`api/memories/${encodeURIComponent(id)}/chat`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ text, files: ids }) }).catch(() => undefined);
  if (!r?.ok) {
    setChat(id, { failed: "That didn't reach the host." });
    return false;
  }
  setChat(id, { working: true });
  void readMemoryChat(id);
  return true;
}

/** Answers one of its asks, as Temp's (`replyScratchpad`). */
export async function replyMemoryChat(id: string, ask: string, yes: boolean, answers?: Answers, always?: boolean) {
  const asks = () => state.chats[id]?.asks ?? [];
  const busy = (on: boolean) => setChat(id, { asks: asks().map((a) => (a.id === ask ? { ...a, busy: on } : a)) });
  busy(true);
  const r = await resume(ask, yes, answers, always);
  if (r?.status === 409) return setChat(id, { asks: asks().filter((a) => a.id !== ask) });
  if (!r?.ok) return busy(false);
  void readMemoryChat(id);
}

export async function stopMemoryChat(id: string) {
  reads[id] = (reads[id] ?? 0) + 1;
  setChat(id, { stopping: true });
  const r = await api(`api/memories/${encodeURIComponent(id)}/stop`, { method: "POST" }).catch(() => undefined);
  if (!r?.ok) {
    setChat(id, { stopping: false });
    void readMemoryChat(id);
  }
}

const json = (method: string, body?: object) => ({ method, headers: { "content-type": "application/json" }, body: body && JSON.stringify(body) });

/** Adds a memory; its id, or why it couldn't. */
export async function addMemory(name: string): Promise<{ id?: string; error?: string }> {
  const r = await api("api/memories", json("POST", { name })).catch(() => undefined);
  if (!r?.ok) return { error: (r && (await r.text())) || "That didn't reach the host." };
  const m = (await r.json()) as { id: string };
  await readMemories();
  return { id: m.id };
}

/** Renames a memory or sets its instructions; why not, if it didn't. */
export async function changeMemory(id: string, change: { name?: string; instructions?: string }) {
  const r = await api(`api/memories/${encodeURIComponent(id)}`, json("POST", change)).catch(() => undefined);
  await readMemories();
  return r?.ok ? undefined : (r && (await r.text())) || "That didn't reach the host.";
}

export async function deleteMemory(id: string) {
  const r = await api(`api/memories/${encodeURIComponent(id)}`, { method: "DELETE" }).catch(() => undefined);
  await readMemories();
  return r?.ok ? undefined : (r && (await r.text())) || "That didn't reach the host.";
}
