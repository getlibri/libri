// Holding something on a touch screen opens its menu, as messaging apps do.
// A mouse keeps the browser's own right-click menu, unless the thing has no
// other way to its actions; a touch laptop has both, each for its own input.
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

// Android's own long press timeout, which the Android app's hold uses too.
const HOLD_MS = 400;

export type MenuItem = { label: string; icon: ReactNode; run: () => void };

/** What was held, and whether the finger that held it is still down. */
export type Grip = { source: HTMLElement; pressed: boolean };

/** Opens `menu` on a hold; with no `menu` a hold does nothing. A hold starts
 * only off the controls inside, unless `whole`: then the thing is itself one
 * control, the press that opened the menu must not also press it, and a right
 * click or the keyboard's menu key opens the menu as well. */
export function Held({ children, menu, whole = false, className = "" }: { children: ReactNode; menu?: (grip: Grip, close: () => void) => ReactNode; whole?: boolean; className?: string }) {
  const [grip, setGrip] = useState<Grip | null>(null);
  const body = useRef<HTMLDivElement>(null);
  const hold = useRef<{ timer: number; x: number; y: number } | undefined>(undefined);
  const mouse = useRef(true);
  const opened = useRef(false);
  const release = () => {
    if (hold.current) clearTimeout(hold.current.timer);
    hold.current = undefined;
  };
  const open = () => {
    const pressed = !!hold.current;
    release();
    navigator.vibrate?.(10);
    opened.current = true;
    if (body.current) setGrip({ source: body.current, pressed });
  };
  return (
    <div className={`relative ${className}`}>
      {/* Touch screens don't select by holding here (holding opens the
          menu); a menu's Select text does it instead. */}
      <div
        ref={body}
        className="[@media(pointer:coarse)]:select-none [@media(pointer:coarse)]:[-webkit-touch-callout:none]"
        onPointerDown={(e) => {
          mouse.current = e.pointerType === "mouse";
          opened.current = false;
          if (!menu || e.pointerType !== "touch") return;
          if (!whole && (e.target as HTMLElement).closest("button, a, input, textarea, summary")) return;
          const { clientX: x, clientY: y } = e;
          hold.current = { timer: window.setTimeout(open, HOLD_MS), x, y };
        }}
        onPointerMove={(e) => {
          if (hold.current && Math.hypot(e.clientX - hold.current.x, e.clientY - hold.current.y) > 8) release();
        }}
        onPointerUp={release}
        onPointerCancel={release}
        onClickCapture={(e) => {
          if (whole && opened.current) e.stopPropagation();
        }}
        // Android's own long press, which beats the timer on some phones.
        onContextMenu={(e) => {
          if (!menu || (mouse.current && !whole) || (e.target as HTMLElement).closest("a")) return;
          e.preventDefault();
          if (!grip) open();
        }}
      >
        {children}
      </div>
      {grip &&
        menu?.(grip, () => {
          opened.current = false;
          setGrip(null);
        })}
    </div>
  );
}

/** A held thing's menu: the page dims and blurs, a copy of the thing stands
 * where it was, and a 270px menu hangs 10px under it, left if the thing is
 * `framed` on a card here and right if it brings its own edge. Something too
 * tall to fit with the menu shows its top and scrolls. A tap outside closes it.
 * `over`, when set, stands in place of both. */
/** `foot` stands under the items, for a choice no button makes: a time. */
export function HoldMenu({ grip, framed, label, items, close, over, foot }: { grip: Grip; framed: boolean; label: string; items: MenuItem[]; close: () => void; over?: ReactNode; foot?: ReactNode }) {
  const { source, pressed } = grip;
  const box = useRef<HTMLDivElement>(null);
  // The finger that held the thing is still down. Android's own long press
  // lands on the menu moments after it opens, lifting the finger clicks
  // whatever is now under it, and moving it would scroll the page behind; none
  // of these may count until it is up.
  const held = useRef(pressed);
  useEffect(() => {
    if (!pressed) return;
    const lift = () => setTimeout(() => (held.current = false), 50);
    const still = (e: TouchEvent) => {
      if (held.current && e.cancelable) e.preventDefault();
    };
    addEventListener("pointerup", lift);
    addEventListener("pointercancel", lift);
    addEventListener("touchmove", still, { passive: false });
    return () => {
      removeEventListener("pointerup", lift);
      removeEventListener("pointercancel", lift);
      removeEventListener("touchmove", still);
    };
  }, [pressed]);
  const [place, setPlace] = useState<{ left: number; width: number; top: number; height: number; menu: number } | null>(null);
  const copy = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const esc = (e: globalThis.KeyboardEvent) => e.key === "Escape" && close();
    addEventListener("keydown", esc);
    return () => removeEventListener("keydown", esc);
  }, [close]);
  // The copy is the thing's own drawing, cloned once: a second live render of
  // a message would register with the thread a second time.
  const replaced = over !== undefined;
  useLayoutEffect(() => {
    const el = box.current;
    const into = copy.current;
    if (!el || !into) return;
    const drawn = source.cloneNode(true) as HTMLElement;
    into.replaceChildren(drawn);
    const r = source.getBoundingClientRect();
    const m = el.getBoundingClientRect();
    const pad = framed ? 14 : 0;
    const left = Math.max(12, r.left - pad);
    const width = Math.min(innerWidth - 12, r.right + pad) - left;
    const height = Math.min(r.height + 2 * pad, innerHeight - m.height - 46);
    const top = Math.max(16, Math.min(r.top - pad, innerHeight - height - m.height - 26));
    const menu = framed ? left : left + width - m.width;
    setPlace({ left, width, top, height, menu: Math.max(12, Math.min(menu, innerWidth - m.width - 12)) });
  }, [source, framed, replaced, items.length, !foot]);
  // With no finger on it the menu may have come from the keyboard, which goes
  // into it and back to where it was. It can take focus once it is placed.
  const placed = !!place;
  useEffect(() => {
    if (!placed || pressed) return;
    const back = document.activeElement as HTMLElement | null;
    box.current?.querySelector("button")?.focus({ preventScroll: true });
    return () => back?.focus({ preventScroll: true });
  }, [placed, pressed]);
  return createPortal(
    <div
      className="fixed inset-0 z-40 bg-black/30 backdrop-blur-md"
      onClickCapture={(e) => {
        if (held.current) e.stopPropagation();
      }}
      onClick={close}
      onContextMenu={(e) => {
        e.preventDefault();
        if (!held.current) close();
      }}
    >
      {over ?? (
        <>
          <div
            ref={copy}
            aria-hidden
            style={place ? { left: place.left, width: place.width, top: place.top, maxHeight: place.height } : { visibility: "hidden" }}
            className={`fixed overflow-y-auto overscroll-contain [&_*]:pointer-events-none ${framed ? "rounded-2xl bg-lift p-[14px] shadow-dock" : ""}`}
          />
          <div
            ref={box}
            role="menu"
            aria-label={label}
            onClick={(e) => e.stopPropagation()}
            style={place ? { left: place.menu, top: place.top + place.height + 10 } : { visibility: "hidden" }}
            className="rise fixed w-[min(270px,calc(100vw-24px))] overflow-hidden rounded-2xl border border-frame bg-lift py-1.5 shadow-dock"
          >
            {items.map((it) => (
              <button key={it.label} type="button" role="menuitem" onClick={it.run} className="flex w-full items-center gap-3 px-5 py-3 text-left text-prose outline-none hover:bg-hover focus-visible:bg-hover active:bg-hover">
                <span className="flex-1">{it.label}</span>
                <span className="text-mid">{it.icon}</span>
              </button>
            ))}
            {foot}
          </div>
        </>
      )}
    </div>,
    document.body,
  );
}
