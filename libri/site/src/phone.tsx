import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { useHostLayout } from "./hostLayout";
import { rowsOf, TileMenu, TileMenuCtx, type Codes, type TileRow } from "./tileMenu";
import { tilesNow, VIEWS } from "./tileViews";
import { another, closedAgo, KEPT_CLOSED_DAYS, keptClosed, KIND_ICONS, KIND_NAMES, UNDO_CLOSE_MS, type Kind, type Pane, type TileApi } from "./tiles";
import { useSideSwipe } from "./work";

// A phone's layout (a design choice of 2026-11-10): the bar at the foot of
// the screen is a workspace, a row of items the person adds to and takes
// from, one shown at a time. A pill left of the bar switches workspace and a
// plus right of it adds an item, as Arc's Spaces and Safari's tab groups
// keep sets of tabs apart. A phone's workspaces are its own, kept apart from
// the wide page's tiles (layout.rs Store::Phone); the Android app draws the
// same (Places.kt).

type Space = { id: string; name: string; items: Pane[]; /** The item an agent's tile_open asked shown (layout.rs). */ shown?: string };
type Saved = { spaces: Space[]; on: string; /** Recently closed (layout.rs `closed`). */ closed?: (Space & { closed_at: number })[] };

/** One of each per workspace; the rest are one per `arg`. */
const same = (a: Pane, b: Pane) => a.kind === b.kind && (a.arg ?? "") === (b.arg ?? "");
const paneKey = (p: Pane) => p.kind + (p.arg ? `:${p.arg}` : "");
/** What a phone's bar can hold (layout.rs PHONE_KINDS): no Today. */
const KINDS: Kind[] = VIEWS.filter((k) => k !== "home");

let made = 0;
const fresh = () => `p${Date.now().toString(36)}${(made++).toString(36)}`;
/** What a first run opens on: the bar as it stood before workspaces. */
const FIRST = (): Saved => {
  const w: Space = { id: fresh(), name: "Main", items: [{ kind: "chat" }, { kind: "library" }] };
  return { spaces: [w], on: w.id };
};
/** The first free name of the form `Workspace N`. */
function unnamed(names: string[]) {
  for (let i = names.length + 1; ; i++) if (!names.includes(`Workspace ${i}`)) return `Workspace ${i}`;
}

/** A kept layout as this page reads it, or nothing for one of another shape. */
function readable(v: unknown): Saved | undefined {
  const kept = v as Saved | null;
  if (!kept || !Array.isArray(kept.spaces) || !kept.spaces.length) return;
  // The Workers item is gone from the bar since the composer's agents took
  // its work (agents.tsx); a bar left with nothing holds the chat.
  const s: Saved = {
    ...kept,
    spaces: kept.spaces.map((w) => {
      if (!w || !Array.isArray(w.items)) return w;
      const items = w.items.filter((p) => (p as { kind: string } | null)?.kind !== "workers");
      return { ...w, items: items.length ? items : [{ kind: "chat" }] };
    }),
  };
  const sound = (w: Space) =>
    w && typeof w.id === "string" && typeof w.name === "string" && Array.isArray(w.items) && w.items.length > 0 && w.items.every((p) => p && KINDS.includes(p.kind));
  if (!s.spaces.every(sound)) return;
  const closed = keptClosed(Array.isArray(kept.closed) ? kept.closed : []).filter(sound);
  return { spaces: s.spaces, on: s.spaces.some((w) => w.id === s.on) ? s.on : s.spaces[0].id, closed };
}

/** Something the plus can add to the workspace, or do (`run`) instead. */
export type Addable = { label: string; icon: ReactNode } & ({ pane: Pane; run?: undefined } | { run: () => void; pane?: undefined });

const ICON = "size-[22px] fill-none stroke-current stroke-[1.6] [stroke-linecap:round] [stroke-linejoin:round]";
function Icon({ children, className = ICON }: { children: ReactNode; className?: string }) {
  return (
    <svg viewBox="0 0 24 24" aria-hidden className={className}>
      {children}
    </svg>
  );
}
const PLUS = <path d="M12 5v14M5 12h14" />;
const CROSS = <path d="M18 6 6 18M6 6l12 12" />;
const PANEL = (
  <>
    <rect x="3.5" y="4.5" width="17" height="15" rx="2.5" />
    <path d="M9.5 4.5v15" />
  </>
);
const DOTS = (
  <>
    <circle cx="5" cy="12" r="1.6" />
    <circle cx="12" cy="12" r="1.6" />
    <circle cx="19" cy="12" r="1.6" />
  </>
);
/** The bar's disc and pill: the one floating make (index.css floating). */
const LIFT = "floating";
/** The composer's height at rest: one line and its controls row. */
const COMPOSER_REST = 88;
/** The bar's height, two thirds of the composer's at rest: the two floating
 * pills at a phone's foot in proportion, as Claude Duet's (photo,
 * 2026-11-13; at the composer's full height it stood too heavy, 11-13).
 * Places.kt BAR_HEIGHT. */
const BAR = Math.round((COMPOSER_REST * 2) / 3);
/** The gap at a phone's foot outside its pills: from the screen's sides,
 * and from the bar to the gesture area. Places.kt FOOT_GAP. */
const GAP = 14;
/** The gap between the pills at a phone's foot: composer to bar, disc to
 * tabs. Between the send button's inset (6, 2026-11-13), which read as
 * squeezed, and the outer gap it once shared (14): a little more room, the
 * pills still one group (2026-11-14). Places.kt FOOT_INNER. */
const INNER = 10;
const ROW = "flex h-12 w-full items-center rounded-xl px-3 text-left text-control text-ink hover:bg-hover disabled:text-soft disabled:hover:bg-transparent";

export function Phone({
  onClosed,
  render,
  title,
  marks,
  asked,
  addable,
  panel,
  still,
  again,
}: {
  render: (pane: Pane, tile: TileApi) => ReactNode;
  /** A pane's name (a worker's or a page's own). */
  title: (pane: Pane) => string;
  /** A dot on an item (a worker waiting, Temp answering). */
  marks: Partial<Record<Kind, ReactNode>>;
  /** Something to open from outside (a page the chat published). */
  asked?: { pane: Pane; n: number; own?: boolean; anew?: boolean };
  /** Told what a closed tile (bar item) showed, which may end it there. */
  onClosed?: (p: Pane) => void;
  /** What the plus offers to add. */
  addable: Addable[];
  /** Opens the side panel. */
  panel: () => void;
  /** No swipes while something lies over the page. */
  still: boolean;
  /** An item pressed while it is the one shown. */
  again: (k: Kind) => void;
}) {
  const [saved, setSaved, undoAgent, keepAgent, hydrated, saveError] = useHostLayout("api/phone-layout", "libri-phone", readable, FIRST);
  const ws = saved.spaces.find((w) => w.id === saved.on) ?? saved.spaces[0];
  const items = ws.items;
  // Which item shows is this screen's own, never kept: each phone picks its
  // own place in a workspace, as each browser tab keeps its own scroll.
  const [shownKey, setShownKey] = useState<string>();
  const at = Math.max(0, shownKey ? items.findIndex((p) => paneKey(p) === shownKey) : 0);
  // Something opened that is on no item (a worker's thread, a page, a
  // built-in) lies over the item it came from until put down, as a pushed
  // page does in a phone's tab, and is never kept.
  const [over, setOver] = useState<Pane>();
  // The bar being edited (below): no swipes on the page meanwhile.
  const [editing, setEditing] = useState(false);
  const view = over ?? items[at];
  const lit = over ? items.findIndex((p) => p.kind === (over.kind === "library" ? "library" : "")) : at;

  const show = useCallback((i: number) => {
    setOver(undefined);
    setShownKey((k) => {
      const p = items[i];
      return p ? paneKey(p) : k;
    });
  }, [items]);
  const change = useCallback((f: (w: Space) => Space) => setSaved((s) => ({ ...s, spaces: s.spaces.map((w) => (w.id === s.on ? f(w) : w)) })), [setSaved]);

  /** Brings what is asked for: the item that holds it, or over the item shown. */
  const open = useCallback(
    (pane: Pane) => {
      const i = items.findIndex((p) => same(p, pane));
      if (i < 0) return setOver(pane);
      if (i === at && !over) again(pane.kind);
      show(i);
    },
    [items, at, over, again, show],
  );
  const opener = useRef(open);
  opener.current = open;
  // An agent's tile_open names the item to show (layout.rs `open_phone`).
  const agentShown = useRef(ws.shown);
  useEffect(() => {
    if (!ws.shown || ws.shown === agentShown.current) return;
    agentShown.current = ws.shown;
    setOver(undefined);
    setShownKey(ws.shown);
  }, [ws.shown]);
  // A pane asked for a tab of its own is added to the bar (add, below).
  const adder = useRef<(pane: Pane) => void>(() => {});
  useEffect(() => {
    if (asked && hydrated) (asked.own || asked.anew ? adder.current : opener.current)(asked.pane);
  }, [asked, hydrated]);

  const tile: TileApi = {
    phone: true,
    focused: true,
    open,
    // A tile's own change of what it shows: back to the item when it names
    // it; a chat's memory or a whiteboard's board picked anew changes the
    // item, so the workspace keeps it (as Android and the wide page do);
    // else it lies over.
    // A chat's tabs changing keep the item; a tab whose memory another
    // item shows goes to that item, as the bar holds each once.
    replace: (p) => {
      const here = items[at];
      const retabbed = String(p.tabs ?? "") !== String(here.tabs ?? "");
      if (same(p, here) && !retabbed) return setOver(undefined);
      const own = (here.kind === "chat" || here.kind === "board") && p.kind === here.kind;
      const other = items.findIndex((q, i) => i !== at && same(q, p));
      if (!over && own && other < 0) {
        change((w) => ({ ...w, items: w.items.map((q) => (same(q, here) ? p : q)) }));
        return setShownKey(paneKey(p));
      }
      if (!over && own && p.kind === "chat") return show(other);
      setOver(p);
    },
    // Put down, what lies over goes; an item's own close (Temp's back,
    // the terminal's ✕) goes back to the chat, as they did as overlays.
    close: () => {
      if (over) return setOver(undefined);
      const chat = items.findIndex((q) => q.kind === "chat" && !q.arg);
      show(chat >= 0 ? chat : Math.max(0, at - 1));
    },
  };

  // A swipe on the page steps along the items; past the first it opens the
  // side panel, as the leftmost tab of a phone app does.
  useSideSwipe(!still && !editing, (by) => {
    const to = (over ? Math.max(lit, at) : at) + by;
    if (to < 0) return panel();
    if (to < items.length) show(to);
  });

  const goSpace = (id: string) => {
    setSaved((s) => ({ ...s, on: id }));
    setShownKey(undefined);
    setOver(undefined);
  };
  // A swipe along the bar itself goes to the workspace beside, as Arc's
  // does; the page's own swipe keeps away from it (data-own-swipe).
  const swipeFrom = useRef<{ x: number; y: number }>(undefined);
  const barSwipe = {
    onTouchStart: (e: React.TouchEvent) => {
      const t = e.touches[0];
      swipeFrom.current = e.touches.length === 1 ? { x: t.clientX, y: t.clientY } : undefined;
    },
    onTouchEnd: (e: React.TouchEvent) => {
      const f = swipeFrom.current;
      swipeFrom.current = undefined;
      if (!f || menu || editing) return;
      const t = e.changedTouches[0];
      const dx = t.clientX - f.x;
      if (Math.abs(dx) < 48 || Math.abs(dx) < Math.abs(t.clientY - f.y) * 2) return;
      const i = saved.spaces.findIndex((w) => w.id === ws.id) + (dx < 0 ? 1 : -1);
      const to = saved.spaces[i];
      if (to) goSpace(to.id);
    },
  };

  // What lies open above the bar: the workspaces or the plus's menu.
  const [menu, setMenu] = useState<"spaces" | "add">();
  const [naming, setNaming] = useState<string>();
  const [spaceMenu, setSpaceMenu] = useState<string>();
  const addSpace = () => {
    const w: Space = { id: fresh(), name: unnamed(saved.spaces.map((x) => x.name)), items: [{ kind: "chat" }] };
    setSaved((s) => ({ ...s, spaces: [...s.spaces, w], on: w.id }));
    setShownKey(undefined);
    setOver(undefined);
    setNaming(w.id);
    made.current = w.id;
  };
  // A workspace just made is named in the list, which then goes, leaving its bar.
  const made = useRef<string>(undefined);
  const rename = (id: string, name: string) => {
    setSaved((s) => ({ ...s, spaces: s.spaces.map((w) => (w.id === id && name.trim() ? { ...w, name: name.trim() } : w)) }));
    setNaming(undefined);
    if (made.current === id) setMenu(undefined);
    made.current = undefined;
  };
  /** Closes a workspace into Recently closed: every chat and worker goes
   * on, and for a moment an Undo stands where it was listed; the last one
   * closed leaves a fresh one. */
  const [undoable, setUndoable] = useState<{ id: string; name: string; at: number }>();
  useEffect(() => {
    if (!undoable) return;
    const t = setTimeout(() => setUndoable(undefined), UNDO_CLOSE_MS);
    return () => clearTimeout(t);
  }, [undoable]);
  const closeSpace = (id: string) => {
    const at = saved.spaces.findIndex((w) => w.id === id);
    if (at >= 0) setUndoable({ id, name: saved.spaces[at].name, at });
    setSaved((s) => {
      const i = s.spaces.findIndex((w) => w.id === id);
      if (i < 0) return s;
      const { shown: _, ...gone } = s.spaces[i];
      const closed = [{ ...gone, closed_at: Math.floor(Date.now() / 1000) }, ...(s.closed ?? []).filter((w) => w.id !== id)];
      const spaces = s.spaces.filter((w) => w.id !== id);
      if (!spaces.length) return { ...FIRST(), closed };
      return { ...s, spaces, closed, on: s.on === id ? spaces[Math.max(0, i - 1)].id : s.on };
    });
    setShownKey(undefined);
    setOver(undefined);
    setSpaceMenu(undefined);
  };
  /** Back from Recently closed, at `at` in the list (else its end), shown. */
  const reopenSpace = (id: string, at?: number) => {
    setSaved((s) => {
      const w = s.closed?.find((c) => c.id === id);
      if (!w) return s;
      const { closed_at: _, ...back } = w;
      const spaces = [...s.spaces];
      spaces.splice(at ?? spaces.length, 0, back);
      return { ...s, spaces, closed: s.closed!.filter((c) => c.id !== id), on: id };
    });
    setShownKey(undefined);
    setOver(undefined);
  };
  const forgetSpace = (id: string) => setSaved((s) => ({ ...s, closed: (s.closed ?? []).filter((c) => c.id !== id) }));
  const undoSpaceClose = () => {
    if (!undoable) return;
    reopenSpace(undoable.id, undoable.at);
    setUndoable(undefined);
  };
  const undoRow = undoable && (
    <button key="undo" type="button" data-action="space-undo" aria-label={`Undo closing ${undoable.name}`} onClick={undoSpaceClose} className={`${ROW} justify-between gap-2`}>
      <span className="truncate text-soft">Closed {undoable.name}</span>
      <span className="font-medium text-ink">Undo</span>
    </button>
  );
  // A terminal added beside one on the bar is a shell of its own (tiles.tsx
  // another); any other place the bar holds once.
  const add = (asked: Pane) => {
    setMenu(undefined);
    const pane = another(asked, items);
    if (!items.some((p) => same(p, pane))) change((w) => ({ ...w, items: [...w.items, pane] }));
    setOver(undefined);
    setShownKey(paneKey(pane));
  };
  adder.current = add;
  // The bar edits in place, as a phone's
  // home screen does: a long press lifts every item but the disc, which is
  // pinned; dragged sideways an item takes a new place, dropped on the plus
  // (an ✕ while editing) it leaves the bar, and a tap off the bar puts it
  // down. Any time, an item flicked up leaves the bar, as a card flicked
  // away in a phone's app switcher, with an Undo. The chat stays: the bar
  // is the way back to it. Leaving the bar keeps what an item shows.
  const [drag, setDrag] = useState<{ key: string; from: number; to: number; dx: number; width: number; over: boolean }>();
  const [leaving, setLeaving] = useState<string>();
  const [closed, setClosed] = useState<{ space: string; pane: Pane; at: number; name: string }>();
  const plus = useRef<HTMLButtonElement>(null);
  const closable = (p: Pane) => !(p.kind === "chat" && !p.arg) && items.length > 1;
  // An item's menu, from a hold on it in the bar: what its place adds, then
  // moving it to another workspace, editing the bar, and closing it.
  const [itemMenu, setItemMenu] = useState<{ i: number; x: number; y: number }>();
  // The code each item's place gives its kind's rows, by item.
  const added = useRef(new Map<string, Map<string, Codes>>());
  const registers = useRef(new Map<string, (kind: string, code: Codes | null) => void>());
  const registerFor = (k: string) => {
    let r = registers.current.get(k);
    if (!r) {
      r = (kind, code) => {
        const mine = added.current.get(k) ?? new Map<string, Codes>();
        if (code) mine.set(kind, code);
        else mine.delete(kind);
        added.current.set(k, mine);
      };
      registers.current.set(k, r);
    }
    return r;
  };
  /** Puts item `i` on workspace `to`'s bar (a new one when none is named),
   * off this one, and goes there. */
  const moveToSpace = (i: number, to?: string) => {
    const p = items[i];
    if (!p) return;
    setSaved((s) => {
      const target = to ?? fresh();
      const spaces = s.spaces.map((w) => {
        if (w.id === s.on) return { ...w, items: w.items.filter((_, j) => j !== i) };
        return w.id === target && !w.items.some((q) => same(q, p)) ? { ...w, items: [...w.items, p] } : w;
      });
      const made = to ? [] : [{ id: target, name: unnamed(s.spaces.map((w) => w.name)), items: [{ kind: "chat" } as Pane, ...(p.kind === "chat" && !p.arg ? [] : [p])] }];
      return { ...s, spaces: [...spaces, ...made], on: target };
    });
    setShownKey(paneKey(p));
  };
  /** An item's menu: its place's rows, then what any bar item does, as
   * the host names them (tiles.rs `BAR_ROWS`). */
  const menuOf = (i: number): TileRow[][] => {
    const p = items[i];
    if (!p) return [];
    const others = saved.spaces.filter((w) => w.id !== ws.id);
    const own = rowsOf(tilesNow()?.tiles.find((t) => t.kind === p.kind)?.rows ?? [], added.current.get(paneKey(p))?.get(p.kind) ?? {});
    const bar: Codes = {
      "move": closable(p) ? { run: () => {}, then: [...others.map((w) => ({ label: w.name, run: () => moveToSpace(i, w.id) })), { label: "New workspace", run: () => moveToSpace(i) }] } : undefined,
      "edit-bar": { run: () => setEditing(true) },
      "close": closable(p) ? { run: () => closeItem(i) } : undefined,
    };
    const rows = rowsOf(tilesNow()?.bar_rows ?? [], bar);
    return [own, rows.filter((r) => r.id !== "close"), rows.filter((r) => r.id === "close")];
  };

  const moveItem = (from: number, to: number) => {
    if (from === to || to < 0 || to >= items.length) return;
    change((w) => {
      const next = [...w.items];
      next.splice(to, 0, ...next.splice(from, 1));
      return { ...w, items: next };
    });
  };
  const closeItem = (i: number) => {
    const gone = items[i];
    if (!gone || !closable(gone)) return;
    // A Temp's item closed ends that Temp; an Undo brings a fresh one.
    onClosed?.(gone);
    const k = paneKey(gone);
    // It rises and fades while its room closes (the tab's transition), then goes.
    setLeaving(k);
    setTimeout(() => {
      setLeaving(undefined);
      change((w) => ({ ...w, items: w.items.filter((p) => !same(p, gone)) }));
      if (paneKey(view) === k || i === at) setShownKey(paneKey(items[i - 1] ?? items[i + 1]));
      setClosed({ space: ws.id, pane: gone, at: i, name: label(gone) });
    }, 180);
  };
  const undoClose = () => {
    if (!closed) return;
    setSaved((s) => ({
      ...s,
      spaces: s.spaces.map((w) => (w.id !== closed.space || w.items.some((p) => same(p, closed.pane)) ? w : { ...w, items: [...w.items.slice(0, closed.at), closed.pane, ...w.items.slice(closed.at)] })),
    }));
    setClosed(undefined);
  };
  useEffect(() => {
    if (!closed) return;
    const t = setTimeout(() => setClosed(undefined), 5000);
    return () => clearTimeout(t);
  }, [closed]);
  useEffect(() => {
    if (!editing) return;
    const done = (e: KeyboardEvent) => e.key === "Escape" && setEditing(false);
    addEventListener("keydown", done);
    return () => removeEventListener("keydown", done);
  }, [editing]);

  // One finger on an item: a tap shows it, a quick flick up closes it, a
  // hold starts editing, and while editing a drag sideways carries it.
  const gesture = useRef<{ key: string; i: number; x: number; y: number; t: number; moved: boolean; held: boolean; carrying: boolean; timer?: ReturnType<typeof setTimeout> }>(undefined);
  const editingNow = useRef(editing);
  editingNow.current = editing;
  const overPlus = (x: number, y: number) => {
    const r = plus.current?.getBoundingClientRect();
    return !!r && x > r.left - 8 && x < r.right + 8 && y > r.top - 16 && y < r.bottom + 8;
  };
  const grip = (i: number, p: Pane) => ({
    onPointerDown: (e: React.PointerEvent<HTMLButtonElement>) => {
      if (e.button !== 0) return;
      const g = { key: paneKey(p), i, x: e.clientX, y: e.clientY, t: e.timeStamp, moved: false, held: false, carrying: false } as NonNullable<typeof gesture.current>;
      gesture.current = g;
      // A hold opens the item's menu (tileMenu.tsx), whose Edit the bar
      // starts editing; the finger lifted after it presses nothing.
      if (!editing)
        g.timer = setTimeout(() => {
          gesture.current = undefined;
          navigator.vibrate?.(10);
          const r = (e.target as HTMLElement).closest("button")?.getBoundingClientRect();
          setItemMenu({ i, x: r ? r.left : e.clientX, y: r ? r.top : e.clientY });
        }, 450);
    },
    onPointerMove: (e: React.PointerEvent<HTMLButtonElement>) => {
      const g = gesture.current;
      if (!g || g.key !== paneKey(p)) return;
      const dx = e.clientX - g.x;
      const dy = e.clientY - g.y;
      if (!g.moved && Math.hypot(dx, dy) > 8) {
        g.moved = true;
        clearTimeout(g.timer);
      }
      if (!editing && !g.held) return;
      if (!g.carrying && Math.abs(dx) > 6) {
        g.carrying = true;
        e.currentTarget.setPointerCapture(e.pointerId);
      }
      if (!g.carrying) return;
      const width = e.currentTarget.getBoundingClientRect().width;
      const to = Math.max(0, Math.min(items.length - 1, g.i + Math.round(dx / width)));
      setDrag({ key: g.key, from: g.i, to, dx, width, over: overPlus(e.clientX, e.clientY) });
    },
    onPointerUp: (e: React.PointerEvent<HTMLButtonElement>) => {
      const g = gesture.current;
      gesture.current = undefined;
      if (!g || g.key !== paneKey(p)) return;
      clearTimeout(g.timer);
      setDrag(undefined);
      if (g.carrying) {
        if (overPlus(e.clientX, e.clientY)) closeItem(g.i);
        else moveItem(g.i, Math.max(0, Math.min(items.length - 1, g.i + Math.round((e.clientX - g.x) / e.currentTarget.getBoundingClientRect().width))));
        return;
      }
      if (g.held || editing) return;
      const dx = e.clientX - g.x;
      const dy = e.clientY - g.y;
      // Quick and mostly upward: a flick, not a slow drag nor a sideways swipe.
      if (dy < -36 && Math.abs(dy) > Math.abs(dx) * 1.5 && e.timeStamp - g.t < 400) return closeItem(g.i);
      if (g.moved) return;
      if (i === at && !over) again(p.kind);
      show(i);
    },
    onPointerCancel: () => {
      clearTimeout(gesture.current?.timer);
      gesture.current = undefined;
      setDrag(undefined);
    },
    onKeyDown: (e: React.KeyboardEvent) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        return editing ? setEditing(false) : show(i);
      }
      if (!editing) return;
      if (e.key === "ArrowLeft") moveItem(i, i - 1);
      if (e.key === "ArrowRight") moveItem(i, i + 1);
      if (e.key === "Delete" || e.key === "Backspace") closeItem(i);
    },
    onContextMenu: (e: React.MouseEvent) => e.preventDefault(),
  });
  // While a finger holds or carries an item the bar's row must not scroll
  // under it: a pan the browser takes would cancel the finger's pointer.
  // React's own touchmove listener is passive, so this one is set by hand.
  const strip = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = strip.current;
    if (!el) return;
    const keep = (e: TouchEvent) => {
      const g = gesture.current;
      if (g && (g.held || editingNow.current)) e.preventDefault();
    };
    el.addEventListener("touchmove", keep, { passive: false });
    return () => el.removeEventListener("touchmove", keep);
  });

  // The bar steps aside while the keyboard is up, as Android's does. The
  // page is sized to what the keyboard leaves (interactive-widget=
  // resizes-content), so a page well short of the tallest it has been at
  // this width has the keyboard over the rest; a field merely in focus, as
  // the terminal's is when shown, raises nothing.
  const [typing, setTyping] = useState(false);
  useEffect(() => {
    let width = innerWidth;
    let tallest = innerHeight;
    const measure = () => {
      if (innerWidth !== width) [width, tallest] = [innerWidth, innerHeight];
      tallest = Math.max(tallest, innerHeight);
      setTyping(innerHeight < tallest - 120);
    };
    addEventListener("resize", measure);
    return () => removeEventListener("resize", measure);
  }, []);

  // Everything any workspace holds stays mounted, the rest hidden, so a
  // shell, a chat or an open page keeps its state across items and spaces.
  const panes = new Map<string, Pane>();
  for (const w of saved.spaces) for (const p of w.items) panes.set(paneKey(p), p);
  if (over) panes.set(paneKey(over), over);
  const order = useRef<string[]>([]);
  order.current = [...order.current.filter((k) => panes.has(k)), ...[...panes.keys()].filter((k) => !order.current.includes(k))];
  const viewKey = paneKey(view);

  const label = (p: Pane) => (p.kind === "temp" ? "Temp" : p.kind === "thread" || p.kind === "builtin" || (p.kind === "library" && p.arg) || (p.kind === "chat" && p.arg) ? title(p) : KIND_NAMES[p.kind]);
  const sheet = "absolute z-50 flex max-h-[min(70vh,520px)] flex-col overflow-y-auto rounded-2xl border border-hair bg-lift p-1 shadow-lift";
  const sheetAt = { bottom: `calc(${GAP + BAR + INNER}px + env(safe-area-inset-bottom, 0px))` };

  return (
    <div
      data-phone
      style={{
        "--tile-top": "env(safe-area-inset-top, 0px)",
        // Where a page's first line starts: below the floating controls
        // at the top (10px off the status bar, 44px tall, 10px under).
        "--chrome-top": "calc(env(safe-area-inset-top, 0px) + 64px)",
        // Only a thin fade, under the status bar's icons: the page is not
        // cut off below them.
        "--top-fade": "calc(env(safe-area-inset-top, 0px) + 8px)",
        // A chat's fade below the status bar: a short band, the talk clear
        // behind the header's buttons, as Claude Duet's (Kit.kt CHAT_TOP_FADE).
        "--chat-top-fade": "28px",
        // The room the bar takes at the foot, the inner gap above it
        // included, so what stands on it (the composer) stands that off it.
        "--bar-room": typing ? "0px" : `calc(${GAP + BAR + INNER}px + env(safe-area-inset-bottom, 0px))` } as React.CSSProperties}
      className="relative h-full overflow-hidden bg-paper"
    >
      {order.current.map((k) => {
        const pane = panes.get(k)!;
        const shown = k === viewKey;
        // A list runs on under the bar; a page that ends in a composer stops above it.
        // A chat runs on under the bar too, its dock lifted above it, so the
        // talk shows dimmed beneath the composer and the bar (--chat-foot).
        const chat = pane.kind === "chat" || pane.kind === "temp" || pane.kind === "thread";
        const under = chat || pane.kind === "library" || pane.kind === "pinned" || pane.kind === "builtin";
        return (
          <section
            key={k}
            aria-label={title(pane)}
            aria-hidden={!shown}
            inert={!shown}
            data-surface={pane.kind}
            data-thread={pane.kind === "thread" ? pane.arg : undefined}
            data-tile={shown ? "focused" : undefined}
            // No band at the top: each page runs up under the status bar and
            // keeps its own first line clear of it (--tile-top, --chrome-top).
            style={{ paddingBottom: under ? 0 : "var(--bar-room)", ...(chat ? { "--chat-foot": typing ? `${GAP}px` : "var(--bar-room)" } : {}) } as React.CSSProperties}
            className={`absolute inset-0 flex flex-col overflow-hidden bg-paper ${shown ? "" : "hidden"}`}
          >
            <div className="relative min-h-0 flex-1 [container-type:inline-size]">
              <TileMenuCtx.Provider value={registerFor(k)}>{render(pane, shown ? tile : { ...tile, focused: false })}</TileMenuCtx.Provider>
            </div>
          </section>
        );
      })}

      {/* On the chat the side panel's button floats at the top left, as
          Android's does; search stands at the right in the chat's own header. */}
      {view.kind === "chat" && !over && (
        <button
          type="button"
          aria-label="Side panel"
          title="Side panel"
          onClick={panel}
          style={{ top: "calc(var(--tile-top) + 10px)" }}
          className={`absolute left-[max(14px,env(safe-area-inset-left))] z-30 grid size-11 place-items-center rounded-full text-mid ${LIFT}`}
        >
          <Icon className="size-[18px] fill-none stroke-current stroke-[1.6] [stroke-linejoin:round]">{PANEL}</Icon>
        </button>
      )}

      {!typing && (
        <div
          data-own-swipe
          {...barSwipe}
          // No edge on the disc and the pill: told apart from the page by
          // their fill and shadow alone, as Claude Duet's bar (2026-11-13).
          className="absolute inset-x-0 bottom-0 z-40 flex items-center justify-center [--float-edge:transparent]"
          style={{ gap: INNER, paddingInline: GAP, paddingBottom: `calc(env(safe-area-inset-bottom, 0px) + ${GAP}px)` }}
        >
          <button
            type="button"
            data-action="spaces"
            aria-label={`Workspace: ${ws.name}. Switch workspace`}
            aria-expanded={menu === "spaces"}
            title={ws.name}
            onClick={() => setMenu((m) => (m === "spaces" ? undefined : "spaces"))}
            style={{ width: BAR, height: BAR }}
            className={`grid shrink-0 place-items-center rounded-full text-control font-medium text-ink ${LIFT}`}
          >
            {/* A disc, so the items keep the room: the name's first letter. */}
            <span aria-hidden>{[...ws.name.trim()][0]?.toUpperCase() ?? "·"}</span>
          </button>
          {/* Three items fill the room beside the disc; fewer take their
              thirds of it, more share it down to 56px each, then scroll.
              The plus keeps the pill's end. As Places.kt Bar: the room
              less the plus (3rem) and the pill's padding (0.5rem). */}
          <div className="flex min-w-0 flex-1 justify-center">
          <div style={{ height: BAR, width: `calc(${Math.min(items.length, 3)} / 3 * (100% - 3.5rem) + 3.5rem)` }} className={`flex min-w-0 items-center rounded-full p-1 ${LIFT}`}>
          <div
            ref={strip}
            role="tablist"
            aria-label={`${ws.name}'s items`}
            data-editing={editing || undefined}
            className={`flex h-full min-w-0 flex-1 items-center rounded-full [scrollbar-width:none] ${editing && items.length <= 4 ? "gap-1 overflow-visible" : editing ? "gap-1 overflow-x-auto" : "overflow-x-auto"}`}
          >
            {items.map((p, i) => {
              const on = i === lit;
              const k = paneKey(p);
              const carried = drag?.key === k;
              // While one is carried the others step aside to its landing place.
              const shift = !drag || carried ? 0 : i > drag.from && i <= drag.to ? -drag.width : i < drag.from && i >= drag.to ? drag.width : 0;
              const gone = leaving === k;
              return (
                <button
                  key={k}
                  type="button"
                  role="tab"
                  aria-selected={on}
                  aria-description={editing ? (closable(p) ? "Arrows move it, Delete takes it off the bar" : "Arrows move it") : "Flick up to close; hold for its menu"}
                  data-open={p.kind}
                  data-item={k}
                  data-leaving={gone || undefined}
                  {...grip(i, p)}
                  style={carried ? { transform: `translate(${drag.dx}px, -4px) scale(1.06)` } : shift ? { transform: `translateX(${shift}px)` } : undefined}
                  className={`relative flex h-full min-w-[56px] flex-1 basis-0 flex-col items-center justify-center rounded-full select-none [-webkit-touch-callout:none] ${
                    editing ? "[touch-action:none]" : "[touch-action:pan-x]"
                  } ${carried ? "z-10 transition-none" : "transition-[transform,opacity,flex-grow,min-width] duration-200 ease-out motion-reduce:transition-none"} ${
                    gone ? "min-w-0! grow-0! -translate-y-6 opacity-0" : ""
                  } ${editing ? `floating ${gone ? "" : "-translate-y-0.5"} ${on ? "text-ink" : "text-mid"} ${carried && drag.over ? "opacity-60" : ""}` : on ? "bg-hover text-ink" : "text-soft"}`}
                >
                  <Icon>{KIND_ICONS[p.kind]}</Icon>
                  <span className={`max-w-full truncate text-caption leading-[15px] tracking-[-0.01em] ${on ? "font-medium" : ""}`}>{label(p)}</span>
                  {marks[p.kind]}
                </button>
              );
            })}
          </div>
            {/* While editing, the plus is the ✕ an item is dropped on to
                leave the bar; a tap on it puts the bar down. */}
            <button
              ref={plus}
              type="button"
              data-action={editing ? "remove" : "add"}
              aria-label={editing ? "Done editing the bar (drop an item here to take it off)" : `Add to ${ws.name}`}
              aria-expanded={editing ? undefined : menu === "add"}
              onClick={() => (editing ? setEditing(false) : setMenu((m) => (m === "add" ? undefined : "add")))}
              className={`grid h-full w-12 shrink-0 place-items-center rounded-full transition-colors ${drag?.over ? "bg-accent/15 text-accent" : editing ? "text-ink" : "text-mid"}`}
            >
              <Icon>{editing ? CROSS : PLUS}</Icon>
            </button>
          </div>
          </div>
        </div>
      )}

      {menu && <div aria-hidden onClick={() => (setMenu(undefined), setSpaceMenu(undefined))} className="absolute inset-0 z-40" />}
      {/* A tap anywhere off the bar puts it down; the page under takes nothing meanwhile. */}
      {editing && <div aria-hidden data-edit-backdrop onClick={() => setEditing(false)} className="absolute inset-0 z-30" />}
      {closed && (
        <p role="status" style={sheetAt} className="floating absolute left-1/2 z-50 flex -translate-x-1/2 items-center gap-3 rounded-full py-1.5 pr-1.5 pl-4 text-label whitespace-nowrap text-ink">
          Closed {closed.name}.
          <button type="button" data-action="undo-close" onClick={undoClose} className="rounded-full bg-hair px-3 py-0.5 font-medium text-ink hover:bg-soft/30">
            Undo
          </button>
        </p>
      )}
      {menu === "spaces" && (
        <div role="dialog" aria-label="Workspaces" style={sheetAt} className={`${sheet} inset-x-2`}>
          <p className="px-3 pt-2 pb-1 text-label text-soft">Workspaces</p>
          {saved.spaces.map((w, i) => [
            undoable?.at === i && undoRow,
            <div key={w.id} data-space={w.name}>
              <div className={`flex items-center gap-1 rounded-xl pr-1 ${w.id === ws.id ? "bg-paper" : ""}`}>
                {naming === w.id ? (
                  <input
                    autoFocus
                    defaultValue={w.name}
                    aria-label="Workspace name"
                    onFocus={(e) => e.currentTarget.select()}
                    onBlur={(e) => rename(w.id, e.currentTarget.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") rename(w.id, e.currentTarget.value);
                      if (e.key === "Escape") setNaming(undefined);
                    }}
                    className="m-1 h-10 min-w-0 flex-1 rounded-lg bg-lift px-3 text-control text-ink outline-2 outline-frame"
                  />
                ) : (
                  <button
                    type="button"
                    onClick={() => {
                      goSpace(w.id);
                      setMenu(undefined);
                    }}
                    className={`flex h-12 min-w-0 flex-1 flex-col justify-center px-3 text-left ${w.id === ws.id ? "text-ink" : "text-mid"}`}
                  >
                    <span className={`truncate text-control ${w.id === ws.id ? "font-medium" : ""}`}>{w.name}</span>
                    <span className="truncate text-label text-soft">{w.items.map(label).join(", ")}</span>
                  </button>
                )}
                <button
                  type="button"
                  data-action="space-menu"
                  aria-label={`More for ${w.name}`}
                  aria-expanded={spaceMenu === w.id}
                  onClick={() => setSpaceMenu((m) => (m === w.id ? undefined : w.id))}
                  className="grid size-10 shrink-0 place-items-center rounded-full text-soft hover:bg-hover"
                >
                  <Icon className="size-[18px] fill-current stroke-none">{DOTS}</Icon>
                </button>
              </div>
              {spaceMenu === w.id && (
                <div role="menu" aria-label={`${w.name}'s menu`} className="pl-3">
                  <button type="button" role="menuitem" data-action="space-rename" onClick={() => (setSpaceMenu(undefined), setNaming(w.id))} className={ROW}>
                    Rename
                  </button>
                  <button type="button" role="menuitem" data-action="space-close" onClick={() => closeSpace(w.id)} className={ROW}>
                    Close workspace
                  </button>
                </div>
              )}
            </div>,
          ])}
          {undoable && undoable.at >= saved.spaces.length && undoRow}
          <button type="button" data-action="space-add" onClick={addSpace} className={`${ROW} gap-2`}>
            <Icon className="size-[18px] fill-none stroke-current stroke-[1.6] [stroke-linecap:round]">{PLUS}</Icon>
            New workspace
          </button>
          {!!saved.closed?.some((c) => c.id !== undoable?.id) && (
            <div role="group" aria-label="Recently closed" className="border-t border-hair pt-1" data-recently-closed>
              <p className="px-3 pt-1 pb-0.5 text-label text-soft">Recently closed</p>
              {saved.closed.filter((c) => c.id !== undoable?.id).map((c) => (
                <div key={c.id} className="flex items-center gap-1 pr-1" data-closed-space={c.name}>
                  <span className="min-w-0 flex-1 truncate px-3 text-control text-ink">
                    {c.name} <span className="text-label text-soft">{closedAgo(c.closed_at)}</span>
                  </span>
                  <button type="button" data-action="space-reopen" aria-label={`Reopen ${c.name}`} onClick={() => (reopenSpace(c.id), setMenu(undefined))} className="h-10 shrink-0 rounded-full px-3 text-label text-ink hover:bg-hover">
                    Reopen
                  </button>
                  <button type="button" data-action="space-delete" aria-label={`Delete ${c.name}`} onClick={() => forgetSpace(c.id)} className="grid size-10 shrink-0 place-items-center rounded-full text-soft hover:bg-hover">
                    <Icon className="size-[16px] fill-none stroke-current stroke-[1.8] [stroke-linecap:round]">{CROSS}</Icon>
                  </button>
                </div>
              ))}
            </div>
          )}
          <p className="px-3 pt-0.5 pb-2 text-label text-soft">Closing keeps every chat and worker, and the workspace under Recently closed for {KEPT_CLOSED_DAYS} days. Swipe along the bar to switch.</p>
        </div>
      )}
      {menu === "add" && (
        <div role="menu" aria-label={`Add to ${ws.name}`} style={sheetAt} className={`${sheet} right-2 w-64`}>
          <p className="px-3 pt-2 pb-1 text-label text-soft">Add to {ws.name}</p>
          {addable.map((a) => {
            return (
              <button
                key={a.pane ? paneKey(a.pane) : a.label}
                type="button"
                role="menuitem"
                data-add={a.pane ? paneKey(a.pane) : a.label.toLowerCase()}
                onClick={() => {
                  if (a.pane) return add(a.pane);
                  setMenu(undefined);
                  a.run();
                }}
                className={`${ROW} gap-3`}
              >
                <span className="grid size-5 place-items-center text-mid">
                  <Icon className="size-[18px] fill-none stroke-current stroke-[1.6] [stroke-linecap:round] [stroke-linejoin:round]">{a.icon}</Icon>
                </span>
                <span className="min-w-0 flex-1 truncate">{a.label}</span>
              </button>
            );
          })}
        </div>
      )}
      {itemMenu && items[itemMenu.i] && (
        <TileMenu at={itemMenu} above label={`${title(items[itemMenu.i])} menu`} groups={menuOf(itemMenu.i)} close={() => setItemMenu(undefined)} />
      )}
      {undoAgent && (
        <p role="status" className="absolute top-[calc(var(--tile-top)+64px)] left-1/2 z-50 flex -translate-x-1/2 items-center gap-3 rounded-full border border-hair bg-lift py-1.5 pr-1.5 pl-4 text-label whitespace-nowrap text-ink shadow-dock">
          Your workspaces were rearranged.
          <button type="button" onClick={undoAgent} className="rounded-full bg-hair px-3 py-0.5 font-medium text-ink hover:bg-soft/30">
            Undo
          </button>
          <button type="button" onClick={keepAgent} aria-label="Keep" className="rounded-full px-2 py-0.5 text-soft hover:text-ink">
            ✕
          </button>
        </p>
      )}
      {saveError && (
        <p role="status" className="absolute top-[calc(var(--tile-top)+64px)] left-1/2 z-50 -translate-x-1/2 rounded-xl border border-hair bg-lift px-4 py-2 text-label whitespace-nowrap text-ink shadow-dock">
          Couldn’t save workspaces. Retrying…
        </p>
      )}
    </div>
  );
}
