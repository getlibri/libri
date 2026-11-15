// The kinds of tile as the host declares them (tiles.rs, /api/tiles): their
// names, what their arg is, which open again and how, and the rows their
// menus add. This page keeps only what is its own: a view for each kind it
// draws (VIEWS, App.tsx renderPane draws them) and a glyph for each row
// (ROW_ICONS); nix/check_tile_coverage.py holds both to the host's list.
import { useSyncExternalStore, type ReactNode } from "react";
import { api } from "./api";

export type RowDef = { id: string; label: string; keys?: string };
export type Fresh = { is: "name"; prefix: string } | { is: "word"; word: string };
export type TileDef = {
  kind: string;
  name: string;
  icon: string;
  about: string;
  arg: { takes: "none" | "may" | "must"; about?: string };
  runs: { wide: boolean; phone: boolean };
  many: boolean;
  offered: boolean;
  fresh?: Fresh;
  rows: RowDef[];
};
export type Registry = { tiles: TileDef[]; tile_rows: RowDef[]; bar_rows: RowDef[] };

/** The kinds this page has a view for. */
export const VIEWS = ["chat", "temp", "terminal", "thread", "library", "board", "browser", "system", "builtin", "pinned", "home"] as const;
export type Kind = (typeof VIEWS)[number];

/** A glyph per row the host names, drawn as the bar's. */
const g = (d: ReactNode) => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    {d}
  </svg>
);
export const ROW_ICONS: Record<string, ReactNode> = {
  "search": g(<><circle cx="11" cy="11" r="6.5" /><path d="m20 20-4.2-4.2" /></>),
  "copy-answer": g(<><rect x="8" y="8" width="12" height="12" rx="2" /><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" /></>),
  "new-temp": g(<path strokeDasharray="3 2.6" d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12Z" />),
  "promote": g(<path d="M12 19V5m-6 6 6-6 6 6" />),
  "paste": g(<><rect x="6" y="4.5" width="12" height="16" rx="2" /><path d="M9.5 4.5h5v3h-5z" /></>),
  "clear": g(<path d="M5 7h14M8 7V5h8v2M7 7l1 12h8l1-12" />),
  "restart": g(<path d="M19 12a7 7 0 1 1-2-4.9M19 4v4h-4" />),
  "stop": g(<rect x="6.5" y="6.5" width="11" height="11" rx="2" />),
  "archive": g(<path d="M4 7h16v3H4zM6 10v9h12v-9M10 14h4" />),
  "reload": g(<path d="M19 12a7 7 0 1 1-2-4.9M19 4v4h-4" />),
  "window": g(<path d="M14 4h6v6M20 4l-8 8M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5" />),
  "new-board": g(<path d="M12 5v14M5 12h14" />),
  "export": g(<path d="M12 4v11m-4-4 4 4 4-4M5 19h14" />),
  "split-right": g(<><rect x="3.5" y="4.5" width="17" height="15" rx="2" /><path d="M12 4.5v15" /></>),
  "split-down": g(<><rect x="3.5" y="4.5" width="17" height="15" rx="2" /><path d="M3.5 12h17" /></>),
  "move": g(<path d="M4 12h12m-4-4 4 4-4 4M20 5v14" />),
  "close": g(<path d="m6 6 12 12M18 6 6 18" />),
  "edit-bar": g(<path d="M4 20h4L19 9l-4-4L4 16zM14 6l4 4" />),
};

let REGISTRY: Registry | null = null;
const watchers = new Set<() => void>();

/** Asks the host for its list, once a page is signed in to it. */
export async function askTiles() {
  const r = await api("api/tiles").catch(() => undefined);
  if (!r?.ok) return;
  REGISTRY = (await r.json()) as Registry;
  watchers.forEach((w) => w());
}

/** The host's list as it stands, outside React; null before it answers. */
export const tilesNow = () => REGISTRY;
export function useTiles(): Registry | null {
  return useSyncExternalStore(
    (w) => (watchers.add(w), () => void watchers.delete(w)),
    () => REGISTRY,
  );
}

/** A kind's declaration, if the host has one. */
export const tileOf = (kind: string) => REGISTRY?.tiles.find((t) => t.kind === kind);

/** A kind's name as the host gives it. */
export const nameOf = (kind: string) => tileOf(kind)?.name ?? kind.charAt(0).toUpperCase() + kind.slice(1);

/** The kinds the dock (wide) or the plus (a phone) offer, in the host's
 * order, that this page draws. */
export const offered = (phone: boolean) =>
  (REGISTRY?.tiles ?? []).filter((t) => t.offered && (phone ? t.runs.phone : t.runs.wide) && (VIEWS as readonly string[]).includes(t.kind)).map((t) => t.kind as Kind);

let made = 0;

/** `pane` as a new tile shows it beside `open` ones: a kind the host says
 * gives its second one an arg of its own gets it (a terminal a shell of its
 * own, a Temp or a whiteboard a new one); any other shows the same again. */
export function another<P extends { kind: string; arg?: string }>(pane: P, open: (P | null)[]): P {
  const fresh = tileOf(pane.kind)?.fresh;
  const twin = open.some((p) => p && p.kind === pane.kind && (p.arg ?? "") === (pane.arg ?? ""));
  if (!fresh || !twin || pane.arg) return pane;
  const arg = fresh.is === "name" ? `${fresh.prefix}${Date.now().toString(36)}${(made++).toString(36)}` : fresh.word;
  return { ...pane, arg };
}

/** A row the page can run: what to do, and whether it can now. */
export type RowCode = { run: () => void; disabled?: boolean; then?: { label: string; run: () => void }[] };
