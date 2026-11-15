import { Fragment, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useHostLayout } from "./hostLayout";
import { ownMenu, rowsOf, TileMenu, TileMenuCtx, type Codes, type TileRow } from "./tileMenu";
import { Strip, StripAdd, StripIcon, StripName, StripTab } from "./tabStrip";
import { another as anotherOf, nameOf, offered, tilesNow, VIEWS, type Kind } from "./tileViews";

// The page's layout: tiles under a bar, after Stencil's Tern and the tiling
// window managers (Hyprland, sway, tmux), in place of fixed columns (a design
// choice of 2026-11-07). A workspace is a saved arrangement of tiles and
// nothing more: every tile shows the same one chat, workers and Library.
// Each workspace is a binary tree of splits, as Hyprland's dwindle layout and
// tmux's panes are. What the tiles show is drawn flat, keyed by what it is
// rather than where, so splitting, resizing, moving it to another tile or
// switching workspace never remounts it: the shell, the chat and an open
// page keep their state.

export type { Kind } from "./tileViews";
/** What a tile shows: `arg` is a worker thread's id, a built-in's id, a
 * Library item (`page:<name>`, `list:<name>`; none for the gallery), or a
 * chat's named memory (none for the main chat). */
export type Pane = { kind: Kind; arg?: string; /** A chat tile's memories as tabs, `arg` the one shown (tiles.rs `tabs`). */ tabs?: string[] };
type Leaf = { t: "leaf"; id: string; pane: Pane | null };
type Split = { t: "split"; id: string; dir: "row" | "col"; at: number; a: Node; b: Node };
type Node = Leaf | Split;
type Workspace = { id: string; name: string; root: Node; focus: string; full: boolean };
/** A workspace's arrangement kept to start new ones from: its tiles and what
 * each shows, never what the chats in them said. */
type Template = { id: string; name: string; root: Node };
/** A closed workspace, kept under Recently closed (layout.rs `closed`). */
type Closed = Workspace & { closed_at: number };
type Saved = { spaces: Workspace[]; on: string; templates?: Template[]; closed?: Closed[] };

/** How long a closed workspace's Undo stays where its × was. */
export const UNDO_CLOSE_MS = 3000;
/** How long Recently closed keeps a workspace (layout.rs KEPT_CLOSED). */
export const KEPT_CLOSED_DAYS = 30;
const nowSecs = () => Math.floor(Date.now() / 1000);
/** Those closed within KEPT_CLOSED_DAYS, newest first. */
export const keptClosed = <W extends { closed_at: number }>(c: W[] | undefined): W[] =>
  (c ?? []).filter((w) => w && typeof w.closed_at === "number" && w.closed_at + KEPT_CLOSED_DAYS * 86400 > nowSecs()).sort((a, b) => b.closed_at - a.closed_at);
/** When it was closed, said briefly. */
export const closedAgo = (at: number) => {
  const m = Math.max(0, Math.round((nowSecs() - at) / 60));
  return m < 1 ? "just now" : m < 60 ? `${m} min ago` : m < 48 * 60 ? `${Math.round(m / 60)} h ago` : `${Math.round(m / 1440)} days ago`;
};

/** Each kind's name, as the host declares it (tiles.rs). */
export const KIND_NAMES: Record<Kind, string> = new Proxy({} as Record<Kind, string>, { get: (_, k) => nameOf(String(k)) });
/** One of each per workspace; the rest are one per `arg`. A chat is one per
 * memory: the main chat once, and each named memory's beside it. */
/** The same pane: its kind and what it shows. Any place opens as often as
 * the person likes, each its own tile (a design choice of 2026-11-13); this
 * only finds the one a link or a notification asks for. */
const same = (a: Pane, b: Pane) => a.kind === b.kind && (a.arg ?? "") === (b.arg ?? "");

let made = 0;
const fresh = (p: string) => `${p}${Date.now().toString(36)}${(made++).toString(36)}`;

/** `pane` as a new tile shows it beside `open` ones (tileViews.tsx). */
export const another = (pane: Pane, open: (Pane | null)[]): Pane => anotherOf(pane, open);
const leaf = (pane: Pane | null): Leaf => ({ t: "leaf", id: fresh("l"), pane });
const split = (dir: "row" | "col", a: Node, b: Node, at = 0.5): Split => ({ t: "split", id: fresh("s"), dir, at, a, b });

/** The arrangements a new, empty workspace offers to start from, beside the
 * person's own templates. There is no fixed set of workspaces: one is added
 * empty and is whatever is then put on it. */
const PRESETS: { name: string; note: string; make: () => Node }[] = [
  { name: "Chat", note: "The chat alone", make: () => leaf({ kind: "chat" }) },
  { name: "Chat and Temp", note: "The chat, a temp chat beside it", make: () => split("row", leaf({ kind: "chat" }), leaf({ kind: "temp" }), 0.6) },
  { name: "Chat and terminal", note: "The chat, the terminal beside it", make: () => split("row", leaf({ kind: "chat" }), leaf({ kind: "terminal" }), 0.5) },
];

function space(name: string, root: Node): Workspace {
  return { id: fresh("w"), name, root, focus: leaves(root)[0].id, full: false };
}
/** What a first run opens on, and what closing the last workspace leaves. */
const CHAT_ONLY = () => space("Chat", leaf({ kind: "chat" }));
const START = (): Saved => {
  const w = CHAT_ONLY();
  return { spaces: [w], on: w.id };
};
/** A copy of an arrangement under new ids, so a template and the workspaces
 * made from it never share a tile. */
function copied(n: Node): Node {
  return n.t === "leaf" ? leaf(n.pane && { ...n.pane }) : split(n.dir, copied(n.a), copied(n.b), n.at);
}
/** The first free name of the form `Workspace N`. */
function unnamed(names: string[]) {
  for (let i = names.length + 1; ; i++) if (!names.includes(`Workspace ${i}`)) return `Workspace ${i}`;
}

function sound(n: unknown): n is Node {
  const m = n as Node | null;
  if (!m || typeof m.id !== "string") return false;
  if (m.t === "leaf") return m.pane === null || (typeof m.pane === "object" && (VIEWS as readonly string[]).includes(m.pane.kind));
  return m.t === "split" && (m.dir === "row" || m.dir === "col") && typeof m.at === "number" && sound(m.a) && sound(m.b);
}
/** A tile that held the Workers place, gone since the composer's agents
 * took its work (agents.tsx), kept empty: its neighbours keep their room. */
function unworked(n: Node): Node {
  if (n.t === "leaf") return (n.pane as { kind: string } | null)?.kind === "workers" ? { ...n, pane: null } : n;
  return { ...n, a: unworked(n.a), b: unworked(n.b) };
}

/** A kept layout as this page reads it, or nothing for one of another shape. */
function readable(v: unknown): Saved | undefined {
  const kept = v as Saved | null;
  if (!kept || !Array.isArray(kept.spaces) || !kept.spaces.length) return;
  const sane = (r: unknown) => (r && typeof r === "object" && "t" in r ? unworked(r as Node) : r) as Node;
  const s: Saved = {
    ...kept,
    spaces: kept.spaces.map((w) => (w ? { ...w, root: sane(w.root) } : w)),
    templates: Array.isArray(kept.templates) ? kept.templates.map((t) => (t ? { ...t, root: sane(t.root) } : t)) : kept.templates,
  };
  if (!s.spaces.every((w) => w && typeof w.id === "string" && typeof w.name === "string" && sound(w.root))) return;
  const templates = Array.isArray(s.templates) ? s.templates.filter((t) => t && typeof t.id === "string" && typeof t.name === "string" && sound(t.root)) : [];
  const closed = keptClosed(Array.isArray(kept.closed) ? kept.closed : []).filter((w) => typeof w.id === "string" && typeof w.name === "string" && sound(sane(w.root))).map((w) => ({ ...w, root: sane(w.root), full: false }));
  return { spaces: s.spaces.map((w) => ({ ...w, full: !!w.full })), on: s.spaces.some((w) => w.id === s.on) ? s.on : s.spaces[0].id, templates, closed };
}

const useSaved = () => useHostLayout("api/layout", "libri-tiles", readable, START);

function leaves(n: Node): Leaf[] {
  return n.t === "leaf" ? [n] : [...leaves(n.a), ...leaves(n.b)];
}
function mapNode(n: Node, f: (n: Node) => Node): Node {
  const m = f(n);
  if (m !== n || m.t === "leaf") return m;
  const a = mapNode(m.a, f);
  const b = mapNode(m.b, f);
  return a === m.a && b === m.b ? m : { ...m, a, b };
}
/** The tree without a leaf: its sibling takes the parent's place. */
function remove(n: Node, id: string): Node | null {
  if (n.t === "leaf") return n.id === id ? null : n;
  const a = remove(n.a, id);
  const b = remove(n.b, id);
  if (!a) return b;
  if (!b) return a;
  return a === n.a && b === n.b ? n : { ...n, a, b };
}

type Side = "left" | "right" | "up" | "down";
/** Where a dragged tile lands on another: beside it on a side, or in its
 * place (the two swap), as Hyprland, tmux and VS Code's editor groups do. */
type Drop = { on: string; at: Side | "swap" };
/** The tree with tile `from` moved: swapped with `on`, or docked on a side
 * of it; `on` may be the root's id, which docks it on that side of the whole. */
function moved(root: Node, from: string, d: Drop): Node {
  const src = leaves(root).find((l) => l.id === from);
  if (!src || from === d.on) return root;
  if (d.at === "swap") {
    const dst = leaves(root).find((l) => l.id === d.on);
    if (!dst) return root;
    return mapNode(root, (n) => (n.id === from ? { ...n, pane: dst.pane } as Leaf : n.id === d.on ? { ...n, pane: src.pane } as Leaf : n));
  }
  const rest = remove(root, from);
  if (!rest) return root;
  const dir = d.at === "left" || d.at === "right" ? "row" : "col";
  const first = d.at === "left" || d.at === "up";
  const besides = (n: Node) => (first ? split(dir, src, n) : split(dir, n, src));
  if (d.on === root.id) return besides(rest);
  let hit = false;
  const out = mapNode(rest, (n) => (n.id === d.on && !hit ? ((hit = true), besides(n)) : n));
  return hit ? out : root;
}
/** Which part of a tile the pointer is over: its middle swaps, the band
 * along each edge docks on that side. */
function zone(r: Rect, x: number, y: number): Side | "swap" {
  const fx = (x - r.x) / r.w;
  const fy = (y - r.y) / r.h;
  if (fx > 0.3 && fx < 0.7 && fy > 0.3 && fy < 0.7) return "swap";
  const edges: [Side, number][] = [["left", fx], ["right", 1 - fx], ["up", fy], ["down", 1 - fy]];
  return edges.sort((a, b) => a[1] - b[1])[0][0];
}
/** The part of a tile a drop on `at` fills, for the preview. */
function half(r: Rect, at: Side | "swap"): Rect {
  return at === "left" ? { ...r, w: r.w / 2 } : at === "right" ? { ...r, x: r.x + r.w / 2, w: r.w / 2 } : at === "up" ? { ...r, h: r.h / 2 } : at === "down" ? { ...r, y: r.y + r.h / 2, h: r.h / 2 } : r;
}

/** What a pane is drawn under: the same pane in another tile or workspace
 * is the same element. A second of one in a workspace (two galleries) is
 * told apart by its tile. */
const paneKey = (p: Pane) => p.kind + (p.arg ? `:${p.arg}` : "");
function keyed(w: Workspace) {
  const keys = new Map<string, string>();
  const seen = new Set<string>();
  for (const l of leaves(w.root)) {
    let k = l.pane ? paneKey(l.pane) : `empty:${l.id}`;
    if (seen.has(k)) k = `${k}#${l.id}`;
    seen.add(k);
    keys.set(l.id, k);
  }
  return keys;
}

type Rect = { x: number; y: number; w: number; h: number };
type Sash = { id: string; dir: "row" | "col"; box: Rect; at: number };
/** Each leaf's place, as fractions of the whole, and each split's sash. */
function place(n: Node, r: Rect, out: { tiles: Map<string, Rect>; sashes: Sash[] }) {
  if (n.t === "leaf") return void out.tiles.set(n.id, r);
  const [ra, rb] =
    n.dir === "row"
      ? [{ ...r, w: r.w * n.at }, { ...r, x: r.x + r.w * n.at, w: r.w * (1 - n.at) }]
      : [{ ...r, h: r.h * n.at }, { ...r, y: r.y + r.h * n.at, h: r.h * (1 - n.at) }];
  place(n.a, ra, out);
  place(n.b, rb, out);
  out.sashes.push({ id: n.id, dir: n.dir, box: r, at: n.at });
}

/** The nearest tile in a direction, as a tiling manager moves focus: one
 * that lies that way and overlaps the focused one across, closest first. */
function neighbour(tiles: Map<string, Rect>, from: string, way: Side) {
  const f = tiles.get(from);
  if (!f) return;
  const e = 1e-6;
  let best: [number, string] | undefined;
  for (const [id, r] of tiles) {
    if (id === from) continue;
    const across = way === "left" || way === "right" ? Math.min(f.y + f.h, r.y + r.h) - Math.max(f.y, r.y) : Math.min(f.x + f.w, r.x + r.w) - Math.max(f.x, r.x);
    if (across <= e) continue;
    const gap =
      way === "left" ? f.x - (r.x + r.w) : way === "right" ? r.x - (f.x + f.w) : way === "up" ? f.y - (r.y + r.h) : r.y - (f.y + f.h);
    if (gap < -e) continue;
    if (!best || gap < best[0]) best = [gap, id];
  }
  return best?.[1];
}

const MAC = /Mac|iPhone|iPad/.test(navigator.platform);
/** The keys, all on Alt+Shift: Alt alone opens Firefox's menus and Alt+←
 * goes back, Ctrl+Shift selects text in the composer, Alt+Space opens the
 * window menu on GNOME, KDE and Windows, Ctrl+K is the memory's search,
 * and Super belongs to the window manager. */
export const KEYS: [string, string][] = [
  ["Alt Shift Space", "Open anything: a place, a page, a worker, a setting"],
  ["Alt Shift H J K L", "Move focus to the tile left, down, up or right"],
  ["Alt Shift ← ↑ → ↓", "Move the focused tile that way: it swaps with the tile there, or at the edge docks along it"],
  ["Alt Shift Enter", "Split the focused tile (it asks what to open)"],
  ["Alt Shift X", "Close the focused tile"],
  ["Alt Shift F", "Full screen the focused tile, and back"],
  ["Alt Shift 1–9", "Go to workspace 1–9"],
  ["Alt Shift N", "New workspace"],
  ["Alt Shift W", "Close the workspace (kept under Recently closed; every chat goes on)"],
  ["Alt Shift /", "Show these keys"],
  [MAC ? "⌘ K" : "Ctrl K", "Search the memory"],
];

export type TileApi = {
  /** Show something else in this tile. */
  replace: (p: Pane) => void;
  /** Open something beside this tile, or where it already is. */
  open: (p: Pane) => void;
  close: () => void;
  focused: boolean;
  phone: boolean;
};

/** One thing the launcher opens: a pane, into the tile it was asked from or
 * beside the focused one, or anything else by `run`. */
export type Launchable = { group: string; label: string; detail?: string; icon?: ReactNode; pane?: Pane; run?: () => void };

const ICON = "size-[18px] fill-none stroke-current stroke-[1.6] [stroke-linecap:round] [stroke-linejoin:round]";
function Icon({ children, className = ICON }: { children: ReactNode; className?: string }) {
  return (
    <svg viewBox="0 0 24 24" aria-hidden className={className}>
      {children}
    </svg>
  );
}
const CALENDAR = (
  <>
    <rect x="3.5" y="5" width="17" height="15" rx="2" />
    <path d="M3.5 10h17M8 3v4M16 3v4" />
  </>
);
export const KIND_ICONS: Record<Kind, ReactNode> = {
  chat: <path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12Z" />,
  temp: <path strokeDasharray="3 2.6" d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12Z" />,
  terminal: <path d="m5 8 4 4-4 4M12 16h7" />,
  board: (
    <>
      <rect x="3.5" y="4.5" width="17" height="13" rx="2" />
      <path d="m7 13.5 3-3 2.5 2.5L16 9.5M9 21l3-3.5 3 3.5" />
    </>
  ),
  browser: (
    <>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M3.5 12h17M12 3.5c2.4 2.3 3.5 5.2 3.5 8.5s-1.1 6.2-3.5 8.5M12 3.5C9.6 5.8 8.5 8.7 8.5 12s1.1 6.2 3.5 8.5" />
    </>
  ),
  system: <path d="M3.5 13h4l2.5-6 4 11 2.5-5h4" />,
  thread: (
    <>
      <circle cx="12" cy="8" r="3.5" />
      <path d="M5 20a7 7 0 0 1 14 0" />
    </>
  ),
  library: <path d="M4 5.5C6.5 4.5 9.5 4.5 12 6c2.5-1.5 5.5-1.5 8-.5V19c-2.5-1-5.5-1-8 .5-2.5-1.5-5.5-1.5-8-.5ZM12 6v13.5" />,
  builtin: CALENDAR,
  home: <path d="M4 11.5 12 5l8 6.5V20h-5v-5h-6v5H4Z" />,
  pinned: <path d="M9 4h6l-1 6 3 3H7l3-3ZM12 13v7" />,
};
const SPLIT_RIGHT = (
  <>
    <rect x="3.5" y="4.5" width="17" height="15" rx="2" />
    <path d="M12 4.5v15" />
  </>
);
const SPLIT_DOWN = (
  <>
    <rect x="3.5" y="4.5" width="17" height="15" rx="2" />
    <path d="M3.5 12h17" />
  </>
);
const FULL = <path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" />;
const UNFULL = <path d="M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5" />;
const CLOSE = <path d="M6 6l12 12M18 6 6 18" />;
const GRIP = (
  <>
    {[8, 12, 16].map((x) => [10, 14].map((y) => <circle key={`${x}${y}`} cx={x} cy={y} r="1.3" />))}
  </>
);
const PLUS = <path d="M12 5v14M5 12h14" />;
const FIND = (
  <>
    <circle cx="11" cy="11" r="7" />
    <path d="m20 20-3.5-3.5" />
  </>
);
export const GEAR = (
  <>
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z" />
  </>
);
export const KEYS_ICON = (
  <>
    <rect x="2.5" y="6" width="19" height="12" rx="2" />
    <path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M8 14h8" />
  </>
);

/** The dock's launchers, in its order; the rest are in the launcher. */
/** What the dock offers: the kinds the host says a wide screen offers. */
const dock = () => offered(false);

export function Tiles({
  onClosed,
  render,
  title,
  settings,
  update,
  marks,
  asked,
  launchable,
  again,
}: {
  /** Something to open from outside the tiles (a page the chat published). */
  asked?: { pane: Pane; n: number; own?: boolean; anew?: boolean };
  /** Told what a closed tile (bar item) showed, which may end it there. */
  onClosed?: (p: Pane) => void;
  /** What a tile shows. */
  render: (pane: Pane, tile: TileApi) => ReactNode;
  /** A pane's name (a worker's or a page's own). */
  title: (pane: Pane) => string;
  settings: () => void;
  /** Whether a newer build waits: a dot on Settings. */
  update: boolean;
  /** A dot on a launcher (a worker waiting, Temp answering). */
  marks: Partial<Record<Kind, ReactNode>>;
  /** All the launcher can open besides the dock's. */
  launchable: Launchable[];
  /** A dock button pressed for the tile already in focus. */
  again: (k: Kind) => void;
}) {
  const [saved, setSaved, undoAgent, keepAgent, hydrated, saveError] = useSaved();
  const ws = saved.spaces.find((w) => w.id === saved.on) ?? saved.spaces[0];
  const [keysShown, setKeysShown] = useState(false);
  const [asking, setAsking] = useState<string>();
  // The launcher, and the empty tile it fills if it was asked from one.
  const [launching, setLaunching] = useState<{ into?: string }>();
  const change = useCallback(
    (f: (w: Workspace) => Workspace) => setSaved((s) => ({ ...s, spaces: s.spaces.map((w) => (w.id === s.on ? f(w) : w)) })),
    [setSaved],
  );

  const all = leaves(ws.root);
  const { tiles, sashes } = useMemo(() => {
    const out = { tiles: new Map<string, Rect>(), sashes: [] as Sash[] };
    place(ws.root, { x: 0, y: 0, w: 1, h: 1 }, out);
    return out;
  }, [ws.root]);
  const focus = all.some((l) => l.id === ws.focus) ? ws.focus : all[0].id;
  const focusedPane = all.find((l) => l.id === focus)?.pane;

  const box = useRef<HTMLDivElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 1, h: 1 });
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const o = new ResizeObserver(() => setSize({ w: el.clientWidth || 1, h: el.clientHeight || 1 }));
    o.observe(el);
    return () => o.disconnect();
  }, []);

  const focusOn = useCallback((id: string) => change((w) => (w.focus === id ? w : { ...w, focus: id })), [change]);
  /** Splits a tile along its longer side, as dwindle does, unless told. */
  const splitTile = useCallback(
    (id: string, pane: Pane | null, dir?: "row" | "col") => {
      const r = tiles.get(id);
      const along = dir ?? (r && r.w * size.w >= r.h * size.h ? "row" : "col");
      const added = leaf(pane);
      change((w) => ({
        ...w,
        full: false,
        focus: added.id,
        root: mapNode(w.root, (n) => (n.t === "leaf" && n.id === id ? split(along, n, added) : n)),
      }));
      if (!pane) setAsking(added.id);
    },
    [change, tiles, size],
  );
  const setPane = useCallback(
    (id: string, pane: Pane) => change((w) => ({ ...w, focus: id, root: mapNode(w.root, (n) => (n.t === "leaf" && n.id === id ? { ...n, pane } : n)) })),
    [change],
  );
  const closeTile = useCallback(
    (id: string) => {
      // A Temp's tile closed ends that Temp (App.tsx `closed`).
      const gone = all.find((l) => l.id === id)?.pane;
      if (gone) onClosed?.(gone);
      change((w) => {
        // The last tile empties rather than goes, and offers the launchers.
        const root = remove(w.root, id) ?? leaf(null);
        const left = leaves(root);
        const near = neighbour(tiles, id, "left") ?? neighbour(tiles, id, "up") ?? neighbour(tiles, id, "right") ?? neighbour(tiles, id, "down");
        return { ...w, root, full: false, focus: left.some((l) => l.id === near) ? near! : left[0].id };
      });
    },
    [change, tiles, all, onClosed],
  );
  /** Brings what is asked for: where it already shows, focus moves to it;
   * otherwise it fills an empty tile, or splits the one asking. `anew`,
   * as the dock and the launcher ask: a tile of its own all the same. */
  const open = useCallback(
    (asked: Pane, from = focus, own = false, anew = false) => {
      const repeat = anew;
      const pane = repeat ? another(asked, all.map((l) => l.pane)) : asked;
      const there = repeat ? undefined : all.find((l) => l.pane && same(l.pane, pane));
      if (there) {
        if (there.id === focus) again(pane.kind);
        return change((w) => ({ ...w, focus: there.id, full: w.full && there.id === w.focus }));
      }
      // One worker's thread opened from the workers takes the place of the
      // thread already open, as a preview pane does, rather than splitting
      // again; one asked for a tile of its own gets one.
      const thread = pane.kind === "thread" && !own && !repeat ? all.find((l) => l.pane?.kind === "thread") : undefined;
      if (thread) return setPane(thread.id, pane);
      const empty = all.find((l) => !l.pane);
      if (empty) return setPane(empty.id, pane);
      splitTile(from, pane);
    },
    [all, focus, again, change, setPane, splitTile],
  );
  const opener = useRef(open);
  opener.current = open;
  useEffect(() => {
    if (asked && hydrated) opener.current(asked.pane, undefined, asked.own, asked.anew);
  }, [asked, hydrated]);
  const launch = (item: Launchable) => {
    const into = launching?.into;
    setLaunching(undefined);
    if (item.run) return item.run();
    if (!item.pane) return;
    if (into && all.some((l) => l.id === into && !l.pane)) setPane(into, another(item.pane, all.map((l) => l.pane)));
    else open(item.pane, focus, true, true);
  };

  const goSpace = useCallback((id: string) => setSaved((s) => ({ ...s, on: id })), [setSaved]);
  const [naming, setNaming] = useState<string>();
  /** Adds an empty workspace, goes to it and offers its name for editing. */
  const addSpace = () => {
    const w = space(unnamed(saved.spaces.map((x) => x.name)), leaf(null));
    setSaved((s) => ({ ...s, spaces: [...s.spaces, w], on: w.id }));
    setNaming(w.id);
  };
  /** Moves tile `id` with what it shows to workspace `to` (a new one when
   * none is named): out of this one as its ✕ takes it, into `to`'s empty
   * tile or beside its focused one, and goes there. */
  const moveToSpace = (id: string, to?: string) => {
    const pane = all.find((l) => l.id === id)?.pane ?? null;
    setSaved((s) => {
      const target = to ?? fresh("w");
      const spaces = s.spaces.map((w) => {
        if (w.id === s.on) {
          const root = remove(w.root, id) ?? leaf(null);
          const left = leaves(root);
          return { ...w, root, full: false, focus: left.some((l) => l.id === w.focus) ? w.focus : left[0].id };
        }
        if (w.id !== target) return w;
        const empty = leaves(w.root).find((l) => !l.pane);
        const added = leaf(pane);
        const root = empty ? mapNode(w.root, (n) => (n.t === "leaf" && n.id === empty.id ? { ...n, pane } : n)) : split("row", w.root, added, 0.5);
        return { ...w, root, focus: empty ? empty.id : added.id, full: false };
      });
      const made = to ? [] : [{ ...space(unnamed(s.spaces.map((x) => x.name)), leaf(pane)), id: target }];
      return { ...s, spaces: [...spaces, ...made], on: target };
    });
  };
  // The tile menu: where it opened, and what each place in a tile added.
  const [menuAt, setMenuAt] = useState<{ id: string; x: number; y: number }>();
  // The code each place in a tile gives its kind's rows, by tile.
  const added = useRef(new Map<string, Map<string, Codes>>());
  const registers = useRef(new Map<string, (kind: string, code: Codes | null) => void>());
  const registerFor = (id: string) => {
    let r = registers.current.get(id);
    if (!r) {
      r = (kind, code) => {
        const mine = added.current.get(id) ?? new Map<string, Codes>();
        if (code) mine.set(kind, code);
        else mine.delete(kind);
        added.current.set(id, mine);
      };
      registers.current.set(id, r);
    }
    return r;
  };
  /** A tile's menu: its place's rows, then what any tile does, as the host
   * names them (tiles.rs `TILE_ROWS`). */
  const menuOf = (id: string): TileRow[][] => {
    const kind = all.find((l) => l.id === id)?.pane?.kind;
    const own = kind ? rowsOf(tilesNow()?.tiles.find((t) => t.kind === kind)?.rows ?? [], added.current.get(id)?.get(kind) ?? {}) : [];
    const others = saved.spaces.filter((w) => w.id !== ws.id);
    const common: Codes = {
      "split-right": { run: () => splitTile(id, null, "row") },
      "split-down": { run: () => splitTile(id, null, "col") },
      "move": { run: () => {}, then: [...others.map((w) => ({ label: w.name, run: () => moveToSpace(id, w.id) })), { label: "New workspace", run: () => moveToSpace(id) }] },
      "close": { run: () => closeTile(id) },
    };
    const rows = rowsOf(tilesNow()?.tile_rows ?? [], common);
    // The splits and the move together, the close apart, as Tern groups them.
    return [own, rows.filter((r) => r.id !== "close"), rows.filter((r) => r.id === "close")];
  };
  /** Opens tile `id`'s menu at the pointer, unless the right click belongs
   * to something with a menu of its own. The notch always opens it. */
  const contextFor = (id: string) => (e: React.MouseEvent) => {
    const target = e.target as Element;
    const notch = !!target.closest("[data-notch]");
    if (!notch && ((e.defaultPrevented && !e.shiftKey) || ownMenu(target, e.shiftKey))) return;
    e.preventDefault();
    setMenuAt({ id, x: e.clientX, y: e.clientY });
  };

  /** Lays a template's arrangement on the workspace in use. A template the
   * person made also gives it its name, unless it was already renamed. */
  const startFrom = (root: Node, name?: string) => {
    const laid = copied(root);
    change((w) => ({ ...w, root: laid, focus: leaves(laid)[0].id, full: false, name: name && /^Workspace \d+$/.test(w.name) ? name : w.name }));
  };
  /** Closes a workspace: its arrangement only. Every chat and worker goes on;
   * the last one closed leaves a fresh one holding the chat. */
  // Closing a workspace keeps it under Recently closed; for a moment its ×
  // is an Undo, where the tab was.
  const [undoable, setUndoable] = useState<{ id: string; name: string; at: number }>();
  useEffect(() => {
    if (!undoable) return;
    const t = setTimeout(() => setUndoable(undefined), UNDO_CLOSE_MS);
    return () => clearTimeout(t);
  }, [undoable]);
  const closeSpace = (id: string) => {
    const at = saved.spaces.findIndex((w) => w.id === id);
    if (at < 0) return;
    setUndoable({ id, name: saved.spaces[at].name, at });
    setSaved((s) => {
      const at = s.spaces.findIndex((w) => w.id === id);
      if (at < 0) return s;
      const gone: Closed = { ...s.spaces[at], full: false, closed_at: nowSecs() };
      const closed = [gone, ...(s.closed ?? []).filter((w) => w.id !== id)];
      const spaces = s.spaces.filter((w) => w.id !== id);
      if (!spaces.length) {
        const w = CHAT_ONLY();
        return { ...s, spaces: [w], on: w.id, closed };
      }
      return { ...s, spaces, closed, on: s.on === id ? spaces[Math.max(0, at - 1)].id : s.on };
    });
  };
  /** Back from Recently closed, at `at` in the bar (else its end), shown. */
  const reopenSpace = (id: string, at?: number) =>
    setSaved((s) => {
      const w = s.closed?.find((c) => c.id === id);
      if (!w) return s;
      const { closed_at: _, ...back } = w;
      const spaces = [...s.spaces];
      spaces.splice(at ?? spaces.length, 0, back);
      return { ...s, spaces, closed: s.closed!.filter((c) => c.id !== id), on: id };
    });
  const forgetSpace = (id: string) => setSaved((s) => ({ ...s, closed: (s.closed ?? []).filter((c) => c.id !== id) }));
  const undoClose = () => {
    if (!undoable) return;
    reopenSpace(undoable.id, undoable.at);
    setUndoable(undefined);
  };
  const undoTab = undoable && (
    <StripTab key="undo" on={false} data-action="space-undo" aria-label={`Undo closing ${undoable.name}`} title={`Reopen ${undoable.name}`} onClick={undoClose}>
      Undo
    </StripTab>
  );
  const rename = (id: string, name: string) => {
    setSaved((s) => ({ ...s, spaces: s.spaces.map((w) => (w.id === id && name.trim() ? { ...w, name: name.trim() } : w)) }));
    setNaming(undefined);
  };
  const templates = saved.templates ?? [];
  const templateName = (name: string, except?: string) => {
    const names = new Set(templates.filter((t) => t.id !== except).map((t) => t.name.toLocaleLowerCase()));
    if (!names.has(name.toLocaleLowerCase())) return name;
    let number = 2;
    while (names.has(`${name} (${number})`.toLocaleLowerCase())) number++;
    return `${name} (${number})`;
  };
  const keepTemplate = (w: Workspace) => {
    const name = templateName(w.name);
    setSaved((s) => ({ ...s, templates: [...(s.templates ?? []), { id: fresh("t"), name, root: copied(w.root) }] }));
    setKept(name);
  };
  // The name of the template just saved, said for a moment under the bar.
  const [kept, setKept] = useState<string>();
  useEffect(() => {
    if (!kept) return;
    const t = setTimeout(() => setKept(undefined), 2600);
    return () => clearTimeout(t);
  }, [kept]);
  // The workspace whose menu (rename, save as template, close) is open.
  const [menu, setMenu] = useState<string>();
  const renameTemplate = (id: string, name: string) => {
    setSaved((s) => ({ ...s, templates: (s.templates ?? []).map((t) => (t.id === id && name.trim() ? { ...t, name: templateName(name.trim(), id) } : t)) }));
    setNamingTemplate(undefined);
  };
  const dropTemplate = (id: string) => setSaved((s) => ({ ...s, templates: (s.templates ?? []).filter((t) => t.id !== id) }));
  const [namingTemplate, setNamingTemplate] = useState<string>();
  const toggleFull = () => change((w) => ({ ...w, full: !w.full }));
  /** Moves a tile by the keys: it swaps with the tile that way, and with
   * none there it docks along that edge of the whole workspace. */
  const moveTile = (id: string, way: Side) => {
    if (all.length < 2) return;
    const to = neighbour(tiles, id, way);
    if (to) return change((w) => ({ ...w, full: false, focus: to, root: moved(w.root, id, { on: to, at: "swap" }) }));
    change((w) => ({ ...w, full: false, root: moved(w.root, id, { on: w.root.id, at: way }) }));
  };
  // A tile dragged by its grip, and where it would land.
  const [carrying, setCarrying] = useState<{ id: string; drop?: Drop }>();
  const dropAt = (e: React.PointerEvent) => {
    const b = box.current!.getBoundingClientRect();
    const x = (e.clientX - b.left) / b.width;
    const y = (e.clientY - b.top) / b.height;
    for (const [id, r] of tiles) if (x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h) return id === carrying?.id ? undefined : { on: id, at: zone(r, x, y) };
  };
  const grip = (id: string) => ({
    onPointerDown: (e: React.PointerEvent<HTMLElement>) => {
      if (e.button !== 0 || all.length < 2) return;
      e.preventDefault();
      e.currentTarget.setPointerCapture(e.pointerId);
      setCarrying({ id });
    },
    onPointerMove: (e: React.PointerEvent<HTMLElement>) => carrying?.id === id && setCarrying({ id, drop: dropAt(e) }),
    onPointerUp: () => {
      const d = carrying?.drop;
      setCarrying(undefined);
      // Focus follows the tile: after a swap it stands in the other's place.
      if (d) change((w) => ({ ...w, full: false, focus: d.at === "swap" ? d.on : id, root: moved(w.root, id, d) }));
    },
    onPointerCancel: () => setCarrying(undefined),
  });

  // The keys: in the capture phase, so the terminal and the composer don't
  // take them first.
  const keys = useRef<(e: KeyboardEvent) => void>(() => {});
  keys.current = (e) => {
    if (!e.altKey || !e.shiftKey || e.ctrlKey || e.metaKey) return;
    const ways: Record<string, Side> = { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down" };
    const looks: Record<string, Side> = { KeyH: "left", KeyJ: "down", KeyK: "up", KeyL: "right" };
    let done = true;
    if (looks[e.code]) {
      const to = neighbour(tiles, focus, looks[e.code]);
      if (to) change((w) => ({ ...w, focus: to, full: false }));
    } else if (ways[e.key]) moveTile(focus, ways[e.key]); else if (e.code === "Space") setLaunching((l) => (l ? undefined : {}));
    else if (e.key === "Enter") splitTile(focus, null);
    // The tile menu's Split right and Split down, as tmux's | and -.
    else if (e.code === "Backslash") splitTile(focus, null, "row");
    else if (e.code === "Minus") splitTile(focus, null, "col");
    else if (e.code === "KeyX") closeTile(focus);
    else if (e.code === "KeyF") toggleFull();
    else if (e.code === "KeyN") addSpace();
    else if (e.code === "KeyW") closeSpace(ws.id);
    else if (e.code === "Slash") setKeysShown((k) => !k);
    else if (/^Digit[1-9]$/.test(e.code)) {
      const w = saved.spaces[Number(e.code.slice(5)) - 1];
      if (w) goSpace(w.id);
    } else done = false;
    if (done) {
      e.preventDefault();
      e.stopPropagation();
    }
  };
  useEffect(() => {
    const k = (e: KeyboardEvent) => keys.current(e);
    addEventListener("keydown", k, true);
    return () => removeEventListener("keydown", k, true);
  }, []);

  const drag = (s: Sash, e: React.PointerEvent<HTMLDivElement>) => {
    const r = box.current!.getBoundingClientRect();
    const along = s.dir === "row" ? ((e.clientX - r.left) / r.width - s.box.x) / s.box.w : ((e.clientY - r.top) / r.height - s.box.y) / s.box.h;
    // No tile narrower than 160px or shorter than 120.
    const min = s.dir === "row" ? 160 / (s.box.w * size.w) : 120 / (s.box.h * size.h);
    const at = Math.min(1 - min, Math.max(min, along));
    change((w) => ({ ...w, root: mapNode(w.root, (n) => (n.id === s.id && n.t === "split" ? { ...n, at } : n)) }));
  };
  const [dragging, setDragging] = useState<string>();

  // A named memory's chat is no stand-in for the main chat.
  const here = new Set(all.map((l) => (l.pane?.kind === "chat" && l.pane.arg ? undefined : l.pane?.kind)));
  const shown = ws.full ? new Set([focus]) : null;
  const GAP = 6;

  // Everything any workspace shows stays mounted, the others' hidden, in
  // the order first seen: moving a frame in the page would reload it.
  const keysHere = keyed(ws);
  const panes = new Map<string, Pane | null>();
  for (const w of saved.spaces) {
    const k = w.id === ws.id ? keysHere : keyed(w);
    for (const l of leaves(w.root)) if (!panes.has(k.get(l.id)!)) panes.set(k.get(l.id)!, l.pane);
  }
  const order = useRef<string[]>([]);
  order.current = [...order.current.filter((k) => panes.has(k)), ...[...panes.keys()].filter((k) => !order.current.includes(k))];
  const leafOf = new Map([...keysHere].map(([id, k]) => [k, id]));
  const idle: TileApi = { replace: () => {}, open: (p) => open(p), close: () => {}, focused: false, phone: false };

  // The launcher's list: the dock's places, the rest the page offers, the
  // workspaces, and the keys.
  const entries: Launchable[] = [
    ...dock().map((k) => ({ group: "Places", label: KIND_NAMES[k], icon: KIND_ICONS[k], pane: { kind: k } })),
    ...launchable,
    ...saved.spaces.map((w, i) => ({ group: "Workspaces", label: w.name, detail: i < 9 ? `Alt Shift ${i + 1}` : undefined, run: () => goSpace(w.id) })),
    { group: "Workspaces", label: "New workspace", detail: "Alt Shift N", icon: PLUS, run: addSpace },
    ...templates.map((t) => ({ group: "Templates", label: t.name, detail: "New workspace", run: () => (addSpace(), startFrom(t.root, t.name)) })),
    { group: "Help", label: "Keys", detail: "Alt Shift /", icon: KEYS_ICON, run: () => setKeysShown(true) },
  ];

  /** Recently closed: each to reopen or delete for good. */
  const recentlyClosed = (item: string) =>
    !!saved.closed?.some((c) => c.id !== undoable?.id) && (
      <div role="group" aria-label="Recently closed" className="border-t border-hair pt-1" data-recently-closed>
        <p className="px-3 pt-1 pb-0.5 text-label text-soft">Recently closed</p>
        {saved.closed.filter((c) => c.id !== undoable?.id).map((c) => (
          <div key={c.id} className="flex items-center gap-1" data-closed-space={c.name}>
            <span className="min-w-0 flex-1 truncate pl-3 text-control text-ink">
              {c.name} <span className="text-label text-soft">{closedAgo(c.closed_at)}</span>
            </span>
            <button type="button" role="menuitem" data-action="space-reopen" aria-label={`Reopen ${c.name}`} onClick={() => (setMenu(undefined), reopenSpace(c.id))} className="h-9 shrink-0 rounded-full px-3 text-label text-ink hover:bg-hover">
              Reopen
            </button>
            <button type="button" role="menuitem" data-action="space-delete" aria-label={`Delete ${c.name}`} title="Delete it for good" onClick={() => forgetSpace(c.id)} className="grid size-9 shrink-0 place-items-center rounded-full text-soft hover:bg-hover hover:text-ink">
              <Icon className="size-3.5 fill-none stroke-current stroke-2 [stroke-linecap:round]">{CLOSE}</Icon>
            </button>
          </div>
        ))}
      </div>
    );
  /** A workspace's menu: rename, save as a template, close. */
  const spaceMenu = (w: Workspace | undefined) => {
    if (!w) return null;
    const item = "flex h-10 w-full items-center rounded-xl px-3 text-left text-control text-ink hover:bg-hover";
    const done = (f: () => void) => () => {
      setMenu(undefined);
      f();
    };
    return (
      <div role="menu" aria-label={`${w.name}'s menu`}>
        <button type="button" role="menuitem" data-action="space-rename" onClick={done(() => setNaming(w.id))} className={item}>
          Rename
        </button>
        <button type="button" role="menuitem" data-action="space-template" onClick={done(() => keepTemplate(w))} className={item}>
          Save as template
        </button>
        <button type="button" role="menuitem" data-action="space-close" onClick={done(() => closeSpace(w.id))} className={item}>
          Close workspace
        </button>
        <p className="px-3 pt-0.5 pb-1.5 text-label text-soft">Closing keeps every chat and worker, and the workspace itself under Recently closed for {KEPT_CLOSED_DAYS} days.</p>
        {recentlyClosed(item)}
      </div>
    );
  };

  return (
    <div ref={root} className="relative flex h-full flex-col bg-paper">
      {/* The bar: workspaces in a pill on the left, the dock on the right,
          Each tile carries its own controls (TileControls). */}
      <div className="relative z-30 flex shrink-0 items-center gap-2 px-3 pt-2.5 pb-2">
        {
          <Strip scroll role="tablist" aria-label="Workspaces" title="Workspaces are arrangements of tiles. Every one shows the same chat, workers and Library.">
            {saved.spaces.map((w, i) => [
              undoable?.at === i && undoTab,
              naming === w.id ? (
                <StripName key={w.id} name={w.name} label="Workspace name" done={(name) => rename(w.id, name)} cancel={() => setNaming(undefined)} />
              ) : (
                <StripTab
                  key={w.id}
                  on={w.id === ws.id}
                  data-space={w.name}
                  aria-current={w.id === ws.id ? "page" : undefined}
                  title={`${w.name}${i < 9 ? ` (Alt Shift ${i + 1})` : ""}. Double-click to rename; right-click for more.`}
                  onClick={() => goSpace(w.id)}
                  onDoubleClick={() => setNaming(w.id)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setMenu(w.id);
                  }}
                  close={{ label: `Close ${w.name}`, title: "Close the workspace: its tiles only, every chat goes on (Alt Shift W)", action: "space-close", run: () => closeSpace(w.id) }}
                >
                  {w.name}
                </StripTab>
              ),
            ])}
            {undoable && undoable.at >= saved.spaces.length && undoTab}
            <StripAdd label="New workspace (Alt Shift N)" onClick={addSpace} data-action="space-add" />
          </Strip>
        }
        <span className="flex-1" />
        <Strip aria-label="Dock">
          {dock().map((k) => (
            <span key={k} className="relative">
              <StripIcon label={`${KIND_NAMES[k]} (open beside the focused tile)`} onClick={() => open({ kind: k }, focus, true, true)} on={here.has(k) && focusedPane?.kind === k} data-open={k}>
                {KIND_ICONS[k]}
              </StripIcon>
              {marks[k]}
              {here.has(k) && <span aria-hidden className="pointer-events-none absolute bottom-0.5 left-1/2 size-1 -translate-x-1/2 rounded-full bg-frame" />}
            </span>
          ))}
          <span className="relative">
            <StripIcon label="Settings and usage" onClick={settings}>
              {GEAR}
            </StripIcon>
            {update && <span aria-hidden className="pointer-events-none absolute top-1.5 right-1.5 size-2 rounded-full bg-accent" />}
          </span>
          <span aria-hidden className="mx-0.5 h-5 w-px bg-hair" />
          <StripIcon label="Open anything (Alt Shift Space)" onClick={() => setLaunching((l) => (l ? undefined : {}))} on={!!launching}>
            {FIND}
          </StripIcon>
        </Strip>
        {menu && (
          <Sheet close={() => setMenu(undefined)} className="left-3 top-[58px] w-64">
            {spaceMenu(saved.spaces.find((w) => w.id === menu))}
          </Sheet>
        )}
        {undoAgent && (
          <p role="status" className="absolute top-[58px] left-1/2 z-50 flex -translate-x-1/2 items-center gap-3 rounded-full border border-hair bg-lift py-1.5 pr-1.5 pl-4 text-label text-ink shadow-dock">
            Libri rearranged your workspaces.
            <button type="button" onClick={undoAgent} className="rounded-full bg-hair px-3 py-0.5 font-medium text-ink hover:bg-soft/30">
              Undo
            </button>
            <button type="button" onClick={keepAgent} aria-label="Keep" className="rounded-full px-2 py-0.5 text-soft hover:text-ink">
              ✕
            </button>
          </p>
        )}
        {saveError && (
          <p role="status" className="absolute top-[58px] left-1/2 z-50 -translate-x-1/2 rounded-xl border border-hair bg-lift px-4 py-2 text-label text-ink shadow-dock">
            Couldn’t save workspaces. Retrying…
          </p>
        )}
        {kept && (
          <p role="status" className="absolute top-[58px] left-1/2 z-50 -translate-x-1/2 rounded-full border border-hair bg-lift px-4 py-1.5 text-label text-ink shadow-dock">
            Saved “{kept}” as a template: a new workspace can start from it.
          </p>
        )}
        {keysShown && (
          <Sheet close={() => setKeysShown(false)} className="right-3 top-[58px] w-[23rem]">
            <p className="px-3 pt-2 pb-1 text-control font-medium text-ink">Keys</p>
            <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 px-3 pb-2 text-label">
              {KEYS.map(([k, what]) => (
                <Fragment key={k}>
                  <dt className="font-mono whitespace-nowrap text-ink">{k}</dt>
                  <dd className="text-mid">{what}</dd>
                </Fragment>
              ))}
            </dl>
            <p className="border-t border-hair px-3 pt-2 pb-2 text-label text-mid">
              Everything has a button too: the notch at the middle of each tile's top edge moves, splits, full-screens and closes it; drag a gap between tiles to resize, double-click it to even them. A workspace is only an arrangement; every one shows the same chat. Its menu (the arrow by its name) renames it, saves it as a template, or closes it.
            </p>
          </Sheet>
        )}
        {launching && <Launcher items={entries} pick={launch} close={() => setLaunching(undefined)} />}
        {menuAt && (
          <TileMenu
            at={menuAt}
            label={`${(() => {
              const p = all.find((l) => l.id === menuAt.id)?.pane;
              return p ? title(p) : "Tile";
            })()} menu`}
            groups={menuOf(menuAt.id)}
            close={() => setMenuAt(undefined)}
          />
        )}
      </div>

      {/* isolate: the tiles' notches stay under the bar's menus. */}
      <div ref={box} className="relative isolate min-h-0 flex-1 mx-1.5 mb-1.5">
        {order.current.map((k) => {
          const id = leafOf.get(k);
          const pane = panes.get(k) ?? null;
          const r = !id ? null : shown ? (shown.has(id) ? { x: 0, y: 0, w: 1, h: 1 } : null) : tiles.get(id)!;
          const focused = id === focus;
          const tile: TileApi = id
            ? { replace: (p) => setPane(id, p), open: (p) => open(p, id), close: () => closeTile(id), focused, phone: false }
            : idle;
          return (
            <section
              key={k}
              aria-label={pane ? title(pane) : "New tile"}
              aria-hidden={!r}
              inert={!r}
              data-tile={!r ? undefined : focused ? "focused" : ""}
              data-here={id ? "" : undefined}
              data-surface={pane?.kind}
              data-thread={pane?.kind === "thread" ? pane.arg : undefined}
              onPointerDownCapture={() => id && !focused && focusOn(id)}
              onContextMenu={id ? contextFor(id) : undefined}
              onFocusCapture={() => id && !focused && focusOn(id)}
              style={
                r
                  ? {
                      left: `calc(${r.x * 100}% + ${GAP / 2}px)`,
                      top: `calc(${r.y * 100}% + ${GAP / 2}px)`,
                      width: `calc(${r.w * 100}% - ${GAP}px)`,
                      height: `calc(${r.h * 100}% - ${GAP}px)`,
                    } as React.CSSProperties
                  : undefined
              }
              className={`group/tile absolute flex flex-col overflow-hidden bg-paper ${r ? "" : "hidden"} rounded-xl border ${focused && all.length > 1 && !ws.full ? "border-frame shadow-[0_0_0_1px_var(--frame)]" : "border-hair"}`}
            >
              {id && (
                <TileControls
                  name={pane ? title(pane) : "the tile"}
                  shown={carrying?.id === id}
                  grip={all.length > 1 ? grip(id) : undefined}
                  split={(dir) => splitTile(id, null, dir)}
                  full={ws.full}
                  fullable={all.length > 1}
                  toggleFull={() => change((w) => ({ ...w, focus: id, full: !w.full }))}
                  close={() => closeTile(id)}
                  closable={!!pane || all.length > 1}
                />
              )}
              <div className="relative min-h-0 flex-1 [container-type:inline-size]">
                {pane ? (
                  <TileMenuCtx.Provider value={id ? registerFor(id) : null}>{render(pane, tile)}</TileMenuCtx.Provider>
                ) : (
                  <Launch
                    open={(p) => id && setPane(id, another(p, all.map((l) => l.pane)))}
                    more={() => setLaunching({ into: id })}
                    asked={!!id && asking === id}
                    start={
                      all.length === 1
                        ? { from: startFrom, templates, naming: namingTemplate, setNaming: setNamingTemplate, rename: renameTemplate, drop: dropTemplate }
                        : undefined
                    }
                  />
                )}
              </div>
            </section>
          );
        })}
        {!shown &&
          sashes.map((s) => {
            const row = s.dir === "row";
            const at = row ? s.box.x + s.box.w * s.at : s.box.y + s.box.h * s.at;
            return (
              <div
                key={s.id}
                role="separator"
                aria-orientation={row ? "vertical" : "horizontal"}
                aria-label="Resize the tiles"
                title="Drag to resize; double-click to even"
                tabIndex={0}
                onPointerDown={(e) => {
                  e.currentTarget.setPointerCapture(e.pointerId);
                  setDragging(s.id);
                }}
                onPointerMove={(e) => dragging === s.id && drag(s, e)}
                onPointerUp={() => setDragging(undefined)}
                onDoubleClick={() => change((w) => ({ ...w, root: mapNode(w.root, (n) => (n.id === s.id && n.t === "split" ? { ...n, at: 0.5 } : n)) }))}
                onKeyDown={(e) => {
                  const step = (row ? { ArrowLeft: -1, ArrowRight: 1 } : { ArrowUp: -1, ArrowDown: 1 })[e.key as "ArrowLeft"];
                  if (!step || e.altKey) return;
                  e.preventDefault();
                  change((w) => ({ ...w, root: mapNode(w.root, (n) => (n.id === s.id && n.t === "split" ? { ...n, at: Math.min(0.85, Math.max(0.15, n.at + step * 0.05)) } : n)) }));
                }}
                style={
                  row
                    ? { left: `calc(${at * 100}% - 5px)`, top: `${s.box.y * 100}%`, height: `${s.box.h * 100}%` }
                    : { top: `calc(${at * 100}% - 5px)`, left: `${s.box.x * 100}%`, width: `${s.box.w * 100}%` }
                }
                className={`group absolute z-20 grid place-items-center outline-none ${row ? "w-2.5 cursor-col-resize" : "h-2.5 cursor-row-resize"}`}
              >
                <span className={`rounded-full transition-colors group-hover:bg-frame group-focus-visible:bg-frame ${dragging === s.id ? "bg-frame" : "bg-transparent"} ${row ? "h-10 w-1" : "h-1 w-10"}`} />
              </div>
            );
          })}
        {carrying?.drop &&
          (() => {
            const t = tiles.get(carrying.drop.on);
            if (!t) return null;
            const p = half(t, carrying.drop.at);
            return (
              <div
                aria-hidden
                data-drop={carrying.drop.at}
                style={{ left: `calc(${p.x * 100}% + 6px)`, top: `calc(${p.y * 100}% + 6px)`, width: `calc(${p.w * 100}% - 12px)`, height: `calc(${p.h * 100}% - 12px)` }}
                className="pointer-events-none absolute z-40 grid place-items-center rounded-xl border-2 border-dashed border-edge bg-hover/60"
              >
                <span className="rounded-full bg-lift px-3 py-1 text-label text-ink shadow-dock">{carrying.drop.at === "swap" ? "Swap" : `Dock ${carrying.drop.at === "up" ? "above" : carrying.drop.at === "down" ? "below" : carrying.drop.at}`}</span>
              </div>
            );
          })()}
        {/* While dragging, a cover keeps the pointer from the tiles' frames. */}
        {dragging && <div className={`absolute inset-0 z-10 ${sashes.find((s) => s.id === dragging)?.dir === "row" ? "cursor-col-resize" : "cursor-row-resize"}`} />}
      </div>
    </div>
  );
}

/** A tile's own controls, a notch hung from the middle of its top edge,
 * flat against the border as a phone's notch is. At rest it is a small tab
 * that covers nothing of the tile's own header; pointed at, focused or
 * tapped it opens into the grip that drags the tile onto another, its
 * splits, full screen and close, and closes again once left. */
function TileControls({ name, shown, grip, split, full, fullable, toggleFull, close, closable }: {
  name: string;
  shown: boolean;
  grip?: Partial<React.HTMLAttributes<HTMLButtonElement>>;
  split: (dir: "row" | "col") => void;
  full?: boolean;
  fullable?: boolean;
  toggleFull?: () => void;
  close: () => void;
  closable: boolean;
}) {
  const small = "grid size-6 place-items-center rounded-full text-soft hover:bg-hover hover:text-ink disabled:opacity-35 disabled:hover:bg-transparent";
  const glyph = "size-[15px] fill-none stroke-current stroke-[1.6] [stroke-linecap:round] [stroke-linejoin:round]";
  // A touch screen has no hover: a tap opens the notch, and it closes itself.
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!open) return;
    const t = setTimeout(() => setOpen(false), 5000);
    return () => clearTimeout(t);
  }, [open]);
  const wide = shown || open;
  return (
    <div
      data-notch
      data-open={wide ? "" : undefined}
      className="group/notch absolute top-0 left-1/2 z-30 flex -translate-x-1/2 justify-center px-8 pb-3"
    >
      <button
        type="button"
        data-action="controls"
        aria-label={`Controls for ${name}`}
        aria-expanded={wide}
        title="Move, split, full screen or close the tile"
        onClick={() => setOpen(true)}
        className={`h-2.5 w-12 rounded-b-lg border border-t-0 border-hair bg-lift ${wide ? "hidden" : "group-hover/notch:hidden group-focus-within/notch:hidden"}`}
      />
    <div
      role="group"
      aria-label={`Controls for ${name}`}
      className={`items-center gap-0.5 rounded-b-xl border border-t-0 border-hair bg-lift px-1 pb-0.5 shadow-dock ${wide ? "flex" : "hidden group-hover/notch:flex group-focus-within/notch:flex"}`}
    >
      {grip && (
        <button type="button" data-action="move" aria-label={`Move ${name}: drag onto another tile, or Alt Shift arrow`} title="Drag onto another tile's side to dock it there, or its middle to swap (Alt Shift arrow)" {...grip} className={`${small} cursor-grab touch-none active:cursor-grabbing`}>
          <Icon className="size-4 fill-current stroke-none">{GRIP}</Icon>
        </button>
      )}
      <button type="button" data-action="split-h" aria-label="Split side by side" title="Split side by side" onClick={() => split("row")} className={small}>
            <Icon className={glyph}>{SPLIT_RIGHT}</Icon>
          </button>
          <button type="button" data-action="split-v" aria-label="Split stacked" title="Split stacked" onClick={() => split("col")} className={small}>
            <Icon className={glyph}>{SPLIT_DOWN}</Icon>
          </button>
          <button type="button" data-action="fullscreen" aria-label={full ? "Back to the tiles" : "Full screen"} title={full ? "Back to the tiles (Alt Shift F)" : "Full screen (Alt Shift F)"} onClick={toggleFull} disabled={!fullable} className={small}>
            <Icon className={glyph}>{full ? UNFULL : FULL}</Icon>
          </button>
      <button type="button" data-action="close" aria-label={`Close ${name}`} title="Close the tile (Alt Shift X)" onClick={close} disabled={!closable} className={small}>
        <Icon className={glyph}>{CLOSE}</Icon>
      </button>
    </div>
    </div>
  );
}

/** What an empty workspace offers to start from: the few arrangements
 * every Libri has, and the templates the person saved. */
type Start = {
  from: (root: Node, name?: string) => void;
  templates: Template[];
  naming?: string;
  setNaming: (id?: string) => void;
  rename: (id: string, name: string) => void;
  drop: (id: string) => void;
};

/** An empty tile offers what it can show, as a new tab page does; the only
 * tile of a workspace also offers the templates to lay out the whole of it. */
function Launch({ open, more, asked, start }: { open: (p: Pane) => void; more: () => void; asked: boolean; start?: Start }) {
  const first = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (asked) first.current?.focus();
  }, [asked]);
  const card = "flex items-center gap-2 rounded-xl border border-hair bg-lift px-3 py-2.5 text-control text-ink hover:border-frame focus-visible:outline-2 focus-visible:outline-frame disabled:opacity-45";
  return (
    <div className="flex h-full overflow-y-auto p-4">
      <div className="m-auto flex w-full max-w-sm flex-col gap-2">
        <p className="px-1 text-label text-soft">Open here</p>
        <div className="grid grid-cols-2 gap-2 @[28rem]:grid-cols-3">
          {dock().map((k, i) => {
            return (
              <button key={k} ref={i === 0 ? first : undefined} type="button" onClick={() => open({ kind: k })} data-open={k} className={card}>
                <Icon>{KIND_ICONS[k]}</Icon>
                {KIND_NAMES[k]}
              </button>
            );
          })}
          <button type="button" onClick={more} className={card}>
            <Icon>{FIND}</Icon>
            More…
          </button>
        </div>
        <p className="px-1 pt-1 text-label text-soft">More finds pages, workers, Calendar and Settings (Alt Shift Space).</p>
        {start && (
          <section aria-label="Templates" className="flex flex-col gap-1 pt-3">
            <p className="px-1 text-label text-soft">Or lay out the workspace from a template</p>
            {PRESETS.map((p) => (
              <button key={p.name} type="button" data-template={p.name} onClick={() => start.from(p.make())} className="flex flex-col rounded-xl px-3 py-2 text-left hover:bg-hover">
                <span className="text-control text-ink">{p.name}</span>
                <span className="text-label text-soft">{p.note}</span>
              </button>
            ))}
            {start.templates.length > 0 && <p className="px-1 pt-2 text-label text-soft">Yours</p>}
            {start.templates.map((t) => (
              <div key={t.id} data-template={t.name} className="flex items-center gap-1 rounded-xl pr-1 hover:bg-hover">
                {start.naming === t.id ? (
                  <input
                    autoFocus
                    defaultValue={t.name}
                    aria-label="Template name"
                    onFocus={(e) => e.currentTarget.select()}
                    onBlur={(e) => start.rename(t.id, e.currentTarget.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") start.rename(t.id, e.currentTarget.value);
                      if (e.key === "Escape") start.setNaming(undefined);
                    }}
                    className="m-1 h-9 min-w-0 flex-1 rounded-lg bg-lift px-3 text-control text-ink outline-2 outline-frame"
                  />
                ) : (
                  <button type="button" onClick={() => start.from(t.root, t.name)} className="flex min-w-0 flex-1 flex-col px-3 py-2 text-left">
                    <span className="truncate text-control text-ink">{t.name}</span>
                    <span className="truncate text-label text-soft">{leaves(t.root).map((l) => (l.pane ? KIND_NAMES[l.pane.kind] : "Empty")).join(", ")}</span>
                  </button>
                )}
                <button type="button" data-action="template-rename" onClick={() => start.setNaming(t.id)} className="h-8 rounded-full px-2.5 text-label text-soft hover:bg-paper hover:text-ink">
                  Rename
                </button>
                <button type="button" data-action="template-delete" aria-label={`Delete the template ${t.name}`} onClick={() => start.drop(t.id)} className="h-8 rounded-full px-2.5 text-label text-soft hover:bg-paper hover:text-ink">
                  Delete
                </button>
              </div>
            ))}
            <p className="px-1 pt-1 text-label text-soft">Save any workspace as a template from its menu (the arrow by its name). A template keeps the tiles and what each shows, never what was said in them.</p>
          </section>
        )}
      </div>
    </div>
  );
}

/** Ranks a launchable for what is typed: every word must start a word of
 * its name or group, or failing that appear in it; earlier and whole-name
 * matches rank first, as a command palette ranks. */
function rank(item: Launchable, words: string[]) {
  const name = item.label.toLowerCase();
  const all = `${name} ${item.group.toLowerCase()} ${item.detail?.toLowerCase() ?? ""}`;
  let score = 0;
  for (const w of words) {
    const at = all.search(new RegExp(`(^|[^a-z0-9])${w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`));
    if (at >= 0) score += at < name.length ? 3 : 2;
    else if (all.includes(w)) score += 1;
    else return 0;
  }
  return score + (name.startsWith(words.join(" ")) ? 2 : 0);
}

/** The launcher, after Raycast and VS Code's command palette: a box to type
 * in over a list grouped by kind, arrows to move, Enter to open. */
function Launcher({ items, pick, close }: { items: Launchable[]; pick: (i: Launchable) => void; close: () => void }) {
  const [typed, setTyped] = useState("");
  const [active, setActive] = useState(0);
  const words = typed.toLowerCase().split(/\s+/).filter(Boolean);
  const found = words.length
    ? items
        .map((item, i) => ({ item, i, s: rank(item, words) }))
        .filter((f) => f.s > 0)
        .sort((a, b) => b.s - a.s || a.i - b.i)
        .map((f) => f.item)
    : items;
  // Typed, the list is by rank alone; untyped, by group.
  const groups: [string, Launchable[]][] = [];
  if (words.length) groups.push(["", found]);
  else for (const item of found) (groups.find(([g]) => g === item.group) ?? groups[groups.push([item.group, []]) - 1])[1].push(item);
  const flat = groups.flatMap(([, g]) => g);
  const at = Math.min(active, Math.max(0, flat.length - 1));
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => {
    list.current?.querySelector(`[data-at="${at}"]`)?.scrollIntoView({ block: "nearest" });
  }, [at]);
  let n = 0;
  return (
    <>
      <div aria-hidden onClick={close} className="fixed inset-0 z-40 bg-ink/10" />
      <div role="dialog" aria-label="Open anything" className={`fixed z-50 flex max-h-[min(520px,70vh)] flex-col overflow-hidden rounded-2xl border border-hair bg-lift shadow-lift top-[12vh] left-1/2 w-[min(560px,calc(100%-48px))] -translate-x-1/2`}>
        <div className="flex shrink-0 items-center gap-2 border-b border-hair px-4">
          <Icon className="size-[18px] shrink-0 fill-none stroke-current stroke-[1.6] text-soft [stroke-linecap:round]">{FIND}</Icon>
          <input
            autoFocus
            value={typed}
            onChange={(e) => {
              setTyped(e.currentTarget.value);
              setActive(0);
            }}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault();
                setActive(Math.max(0, Math.min(flat.length - 1, at + (e.key === "ArrowDown" ? 1 : -1))));
              } else if (e.key === "Enter" && flat[at]) {
                e.preventDefault();
                pick(flat[at]);
              } else if (e.key === "Escape") {
                e.preventDefault();
                close();
              }
            }}
            placeholder="Open a place, page, worker or setting"
            aria-label="What to open"
            role="combobox"
            aria-expanded
            aria-controls="launcher-list"
            className="h-12 min-w-0 flex-1 bg-transparent text-body text-ink outline-none placeholder:text-soft"
          />
          <kbd className="shrink-0 font-mono text-label text-soft">Esc</kbd>
        </div>
        <div ref={list} id="launcher-list" role="listbox" className="min-h-0 flex-1 overflow-y-auto p-1.5">
          {flat.length === 0 && <p className="px-3 py-3 text-control text-soft">Nothing by that name.</p>}
          {groups.map(([g, members]) => (
            <Fragment key={g || "found"}>
              {g && <p className="px-3 pt-2 pb-1 text-label text-soft">{g}</p>}
              {members.map((item) => {
                const i = n++;
                return (
                  <button
                    key={`${item.group}:${item.label}:${i}`}
                    type="button"
                    role="option"
                    aria-selected={i === at}
                    data-at={i}
                    data-open={item.pane?.kind}
                    onMouseMove={() => i !== at && setActive(i)}
                    onClick={() => pick(item)}
                    className={`flex w-full items-center gap-3 rounded-xl px-3 py-2 text-left ${i === at ? "bg-hover" : ""}`}
                  >
                    <span className="grid size-5 shrink-0 place-items-center text-mid">{item.icon && <Icon>{item.icon}</Icon>}</span>
                    <span className="min-w-0 flex-1 truncate text-control text-ink">{item.label}</span>
                    {words.length > 0 && <span className="shrink-0 text-label text-soft">{item.group}</span>}
                    {item.detail && <span className="shrink-0 font-mono text-label text-soft">{item.detail}</span>}
                  </button>
                );
              })}
            </Fragment>
          ))}
        </div>
      </div>
    </>
  );
}

function Sheet({ close, className, children }: { close: () => void; className: string; children: ReactNode }) {
  useEffect(() => {
    const k = (e: KeyboardEvent) => e.key === "Escape" && close();
    addEventListener("keydown", k);
    return () => removeEventListener("keydown", k);
  }, [close]);
  return (
    <>
      <div aria-hidden onClick={close} className="fixed inset-0 z-40" />
      <div role="dialog" className={`absolute z-50 flex flex-col rounded-2xl border border-hair bg-lift p-1 shadow-lift ${className}`}>
        {children}
      </div>
    </>
  );
}
