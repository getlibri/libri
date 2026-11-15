import { createContext, useEffect, useRef, useState, type ReactNode } from "react";
import { addMemory, changeMemory, useMemories } from "./memoryChats";
import { Strip, StripAdd, StripTab } from "./tabStrip";

/* A chat tile's memories as tabs along its top (tiles.rs: the chat kind
   keeps `tabs`): Main and each named memory opened in it, the one shown its
   `arg`. The + offers the memories not open here and a new one. Closing a
   tab only takes it from the tile; the memory and its talk stay. */

/** The tabs a chat tile draws in its header, set where the tile is drawn. */
export const ChatTabsCtx = createContext<ReactNode>(null);

/** A chat pane's tabs: its own, else the one memory it shows. */
export const tabsOf = (p: { arg?: string; tabs?: string[] }) => (p.tabs?.length ? p.tabs : [p.arg ?? ""]);

export function ChatTabs({ tabs, on, show, close }: { tabs: string[]; on: string; show: (id: string) => void; close: (id: string) => void }) {
  const memories = useMemories();
  const [adding, setAdding] = useState(false);
  const nameOf = (id: string) => (id ? (memories?.find((m) => m.id === id)?.name ?? id) : "Main");
  // A tab whose memory was deleted elsewhere goes, unless it is shown,
  // where the memory's chat says so itself.
  const known = (id: string) => !id || !memories || memories.some((m) => m.id === id);
  const shown = tabs.filter((t) => t === on || known(t));
  const closed = (memories ?? []).filter((m) => !tabs.includes(m.id));
  return (
    <span className="relative flex min-w-0" data-chat-tabs>
      <Strip scroll role="tablist" aria-label="Memories in this chat">
        {shown.map((id) => (
          <StripTab
            key={id || "main"}
            on={id === on}
            data-memory-tab={id || "main"}
            title={id ? `${nameOf(id)}: its own memory` : "Main: the main chat and its memory"}
            onClick={() => show(id)}
            close={shown.length > 1 ? { label: `Close ${nameOf(id)}`, title: "Close the tab; the memory and its talk stay", run: () => close(id) } : undefined}
          >
            {nameOf(id)}
          </StripTab>
        ))}
        <StripAdd label="Open a memory here" aria-expanded={adding} onClick={() => setAdding((a) => !a)} data-action="memory-add" />
      </Strip>
      {adding && (
        <AddMemory
          offered={[...(tabs.includes("") ? [] : [{ id: "", name: "Main" }]), ...closed]}
          pick={(id) => (setAdding(false), show(id))}
          close={() => setAdding(false)}
        />
      )}
    </span>
  );
}

/** The +'s list: memories not open in the tile, then a new one, made as
 * Settings › Agents makes one: a name and, if wanted, its instructions. */
function AddMemory({ offered, pick, close }: { offered: { id: string; name: string }[]; pick: (id: string) => void; close: () => void }) {
  const box = useRef<HTMLDivElement>(null);
  const [making, setMaking] = useState(false);
  const [name, setName] = useState("");
  const [instructions, setInstructions] = useState("");
  const [said, setSaid] = useState<string>();
  useEffect(() => {
    const away = (e: PointerEvent) => box.current && !box.current.contains(e.target as Node) && close();
    const key = (e: KeyboardEvent) => e.key === "Escape" && close();
    addEventListener("pointerdown", away);
    addEventListener("keydown", key);
    return () => {
      removeEventListener("pointerdown", away);
      removeEventListener("keydown", key);
    };
  }, [close]);
  const make = async () => {
    const { id, error } = await addMemory(name);
    if (error || !id) return setSaid(error ?? "That didn't reach the host.");
    if (instructions.trim()) {
      const failed = await changeMemory(id, { instructions: instructions.trim() });
      if (failed) return setSaid(failed);
    }
    pick(id);
  };
  const row = "flex h-10 w-full items-center rounded-xl px-3 text-left text-control text-ink hover:bg-hover";
  const field = "w-full rounded-lg border border-frame bg-paper px-3 py-2 text-body text-ink outline-none focus:border-soft";
  return (
    <div ref={box} role="menu" aria-label="Open a memory" className="floating absolute top-full left-0 z-(--layer-pop) mt-2 w-72 max-w-[calc(100vw-28px)] rounded-2xl bg-lift! p-1" data-memory-menu>
      {offered.map((m) => (
        <button key={m.id || "main"} type="button" role="menuitem" data-open-memory={m.id || "main"} onClick={() => pick(m.id)} className={row}>
          {m.name}
        </button>
      ))}
      {!making ? (
        <button type="button" role="menuitem" data-action="memory-new" onClick={() => setMaking(true)} className={`${row} text-mid`}>
          New memory…
        </button>
      ) : (
        <form
          className="flex flex-col gap-2 p-2"
          onSubmit={(e) => {
            e.preventDefault();
            void make();
          }}
        >
          <input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="Name, as Tutor" aria-label="New memory's name" className={field} />
          <textarea value={instructions} onChange={(e) => setInstructions(e.target.value)} rows={3} placeholder="Instructions, if any: what its chat is for" aria-label="New memory's instructions" className={`${field} resize-none`} />
          {said && (
            <p role="alert" className="text-label text-accent">
              {said}
            </p>
          )}
          <button type="submit" disabled={!name.trim()} className="self-end rounded-full bg-hover px-4 py-1.5 text-control text-ink disabled:opacity-40">
            Make it
          </button>
        </form>
      )}
    </div>
  );
}
