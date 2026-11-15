// A tile's own menu, as Tern's pane menu: a right click anywhere in a tile
// that has no menu of its own there, or on its notch, opens it at the
// pointer; on a phone a long press on the item's tab in the bar opens it,
// with Edit the bar among its rows.
// Rows in groups: the tile's own (each place adds its rows with
// useTileRows), then what any tile does (split, move, close). Drawn as the
// hold menu's (hold.tsx): lift, a frame, rounded, the dock's shadow.
import { createContext, useContext, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { ROW_ICONS, type RowCode, type RowDef } from "./tileViews";

export type TileRow = {
  /** The host's id for it (tiles.rs), which the checks find it by. */
  id?: string;
  label: string;
  icon?: ReactNode;
  /** The keys that do the same, as the person types them. */
  keys?: string;
  run: () => void;
  disabled?: boolean;
  /** Rows a second step offers in place of this menu (Move to workspace). */
  then?: TileRow[];
};

/** The code a place inside a tile runs for its kind's rows, by row id. */
export type Codes = Record<string, RowCode | undefined>;
type Register = (kind: string, code: Codes | null) => void;
export const TileMenuCtx = createContext<Register | null>(null);

/** Gives the menu of the tile this is drawn in the code behind `kind`'s
 * rows (tiles.rs names them, with their labels and keys), while drawn. */
export function useTileRows(kind: string, code: Codes) {
  const register = useContext(TileMenuCtx);
  useEffect(() => {
    if (!register) return;
    register(kind, code);
    return () => register(kind, null);
  });
}

/** The rows the host names, each with the code given for it: a row with no
 * code here is left out. */
export function rowsOf(defs: RowDef[], code: Codes): TileRow[] {
  return defs.flatMap((d) => {
    const c = code[d.id];
    if (!c) return [];
    return [{ id: d.id, label: d.label, keys: d.keys, icon: ROW_ICONS[d.id], run: c.run, disabled: c.disabled, then: c.then?.map((t) => ({ label: t.label, run: t.run })) }];
  });
}

/** Whether a right click at `target` belongs to something with a menu of
 * its own: a field, a link, selected words, the terminal's screen (Shift
 * gives the tile's menu there) or the whiteboard's canvas. */
export function ownMenu(target: Element, shift: boolean): boolean {
  if (target.closest("input, textarea, [contenteditable=true], a[href], [data-own-menu]")) return true;
  if (!shift && target.closest("[data-surface=terminal] canvas, [data-surface=terminal] textarea")) return true;
  if (target.closest(".excalidraw")) return true;
  return !!getSelection()?.toString();
}

/** The menu, at `at` (a point, clamped to the window). */
export function TileMenu({ at, above = false, label, groups, close }: { at: { x: number; y: number }; above?: boolean; label: string; groups: TileRow[][]; close: () => void }) {
  const box = useRef<HTMLDivElement>(null);
  const [step, setStep] = useState<{ label: string; rows: TileRow[] } | null>(null);
  const [place, setPlace] = useState<{ left: number; top: number } | null>(null);
  const shown = (step ? [step.rows] : groups).filter((g) => g.length);
  useLayoutEffect(() => {
    const m = box.current?.getBoundingClientRect();
    if (!m) return;
    // `above`: over the point, as from a tab in the phone's bar.
    const y = above ? at.y - m.height - 8 : at.y;
    setPlace({ left: Math.max(8, Math.min(at.x, innerWidth - m.width - 8)), top: Math.max(8, Math.min(y, innerHeight - m.height - 8)) });
  }, [at.x, at.y, step, above]);
  useEffect(() => {
    box.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus({ preventScroll: true });
  }, [place !== null, step]);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        if (step) setStep(null);
        else close();
        return;
      }
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      e.preventDefault();
      const rows = [...(box.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
      const at = rows.indexOf(document.activeElement as HTMLButtonElement);
      rows[(at + (e.key === "ArrowDown" ? 1 : rows.length - 1)) % rows.length]?.focus();
    };
    addEventListener("keydown", key, true);
    return () => removeEventListener("keydown", key, true);
  }, [close, step]);
  return createPortal(
    <div
      className="fixed inset-0 z-50"
      onPointerDown={(e) => e.target === e.currentTarget && close()}
      onContextMenu={(e) => {
        e.preventDefault();
        close();
      }}
    >
      <div
        ref={box}
        role="menu"
        aria-label={step ? step.label : label}
        style={place ?? { visibility: "hidden" }}
        className="rise fixed w-64 overflow-hidden rounded-2xl border border-frame bg-lift py-1.5 shadow-dock"
        data-tile-menu
      >
        {step && <p className="px-4 pt-1 pb-1.5 text-label text-soft">{step.label}</p>}
        {shown.map((g, i) => (
          <div key={i} role="group" className={i ? "mt-1.5 border-t border-hair pt-1.5" : ""}>
            {g.map((r) => (
              <button
                key={r.label}
                type="button"
                role="menuitem"
                disabled={r.disabled}
                onClick={() => {
                  if (r.then) return setStep({ label: r.label.replace(/…$/, ""), rows: r.then });
                  close();
                  r.run();
                }}
                className="flex w-full items-center gap-3 px-4 py-1.5 text-left text-control text-ink outline-none hover:bg-hover focus-visible:bg-hover disabled:text-soft disabled:hover:bg-transparent"
                data-row={r.label}
                data-row-id={r.id}
              >
                <span className="grid size-4 shrink-0 place-items-center text-mid [&>svg]:size-4">{r.icon}</span>
                <span className="min-w-0 flex-1 truncate">{r.label}</span>
                {r.keys && <kbd className="shrink-0 font-sans text-label text-soft">{r.keys}</kbd>}
              </button>
            ))}
          </div>
        ))}
      </div>
    </div>,
    document.body,
  );
}

/** A glyph for a row, drawn as the bar's: a 24-unit path, stroked. */
export function RowIcon({ children }: { children: ReactNode }) {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      {children}
    </svg>
  );
}
