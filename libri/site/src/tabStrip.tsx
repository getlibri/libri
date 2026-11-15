import type { ComponentPropsWithoutRef, ReactNode } from "react";

/* Every row of tabs or launchers in Libri is one strip: a round pill of
   items, the one in use marked by a soft fill and nothing else (no outline,
   no inner border), the same hover on each. The workspace bar, the dock,
   the whiteboard's boards and a chat tile's memories all draw with it, so
   they read as one family. */

const STRIP = "flex items-center gap-0.5 rounded-full border border-hair bg-lift/85 p-1 shadow-dock backdrop-blur-md";
/** An item's shape and states, shared by labels, icons and the +. */
const ITEM = "shrink-0 rounded-full focus-visible:outline-2 focus-visible:outline-frame";
const MARK = (on: boolean | undefined) => (on ? "bg-hover text-ink" : "text-mid hover:bg-hover hover:text-ink");
const GLYPH = "fill-none stroke-current [stroke-linecap:round] [stroke-linejoin:round]";

/** The pill. `scroll` lets a long row scroll sideways without a scrollbar. */
export function Strip({ scroll, className = "", children, ...rest }: { scroll?: boolean; className?: string; children: ReactNode } & ComponentPropsWithoutRef<"div">) {
  return (
    <div {...rest} className={`${STRIP} ${scroll ? "min-w-0 overflow-x-auto [scrollbar-width:none]" : "shrink-0"} ${className}`}>
      {children}
    </div>
  );
}

/** A named tab: picks on click; `close` puts a × at its end, shown while it
 * is in use or hovered and kept in place otherwise, so names never shift.
 * Anything else (a context menu, a double-click, dragging, data-*) passes
 * through to the tab itself. */
export function StripTab({
  on,
  label,
  close,
  className = "",
  children,
  ...rest
}: {
  on?: boolean;
  /** Says what the × closes, as "Close Notes". */
  close?: { label: string; run: () => void; title?: string; action?: string };
  label?: string;
  className?: string;
  children: ReactNode;
} & Omit<ComponentPropsWithoutRef<"button">, "children">) {
  return (
    <span className={`group relative flex shrink-0 items-center ${className}`}>
      <button
        type="button"
        role="tab"
        aria-selected={!!on}
        aria-label={label}
        {...rest}
        className={`${ITEM} flex h-9 max-w-48 items-center truncate pl-3.5 text-control whitespace-nowrap ${close ? "pr-8" : "pr-3.5"} ${MARK(on)} ${on ? "font-medium" : ""}`}
      >
        {children}
      </button>
      {close && (
        <button
          type="button"
          data-action={close.action}
          aria-label={close.label}
          title={close.title ?? close.label}
          onClick={close.run}
          className={`absolute right-1.5 grid size-6 place-items-center rounded-full text-soft hover:bg-lift hover:text-ink focus-visible:opacity-100 ${on ? "" : "opacity-0 group-hover:opacity-100"}`}
        >
          <svg viewBox="0 0 24 24" aria-hidden className={`size-3 stroke-2 ${GLYPH}`}>
            <path d="M6 6l12 12M18 6 6 18" />
          </svg>
        </button>
      )}
    </span>
  );
}

/** An icon item, as the dock's launchers: `on` marks the one in use. */
export function StripIcon({ on, label, children, ...rest }: { on?: boolean; label: string; children: ReactNode } & Omit<ComponentPropsWithoutRef<"button">, "children">) {
  return (
    <button type="button" title={label} aria-label={label} aria-pressed={on} {...rest} className={`${ITEM} grid size-8 place-items-center min-[400px]:size-9 disabled:opacity-35 disabled:hover:bg-transparent ${MARK(on)}`}>
      <svg viewBox="0 0 24 24" aria-hidden className={`size-[18px] stroke-[1.6] ${GLYPH}`}>
        {children}
      </svg>
    </button>
  );
}

/** The + that adds a tab, the strip's last item. */
export function StripAdd({ label, ...rest }: { label: string } & Omit<ComponentPropsWithoutRef<"button">, "children">) {
  return (
    <StripIcon label={label} {...rest}>
      <path d="M12 5v14M5 12h14" />
    </StripIcon>
  );
}

/** A tab being renamed: a field in the tab's own shape. */
export function StripName({ name, label, done, cancel }: { name: string; label: string; done: (name: string) => void; cancel: () => void }) {
  return (
    <input
      autoFocus
      defaultValue={name}
      aria-label={label}
      onFocus={(e) => e.currentTarget.select()}
      onBlur={(e) => done(e.currentTarget.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur();
        if (e.key === "Escape") cancel();
      }}
      className={`${ITEM} h-9 w-32 bg-hover px-3.5 text-control text-ink outline-none`}
    />
  );
}
