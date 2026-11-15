import { useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";

// Neither pane narrower than this share of the window.
const MIN = 0.2;
const clamp = (at: number) => Math.min(1 - MIN, Math.max(MIN, at));

/** Two panes and the line between them, which a drag, the arrow keys or a
 * double-click (back to even) moves. Without a second, the first fills the
 * row in the same place, so opening one beside it keeps the first's state. */
export function Panes({ start, keep, children: [a, b] }: { start: number; keep: (at: number) => void; children: [ReactNode, ReactNode] }) {
  const [at, setAt] = useState(start);
  const [dragging, setDragging] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const settle = (to: number) => {
    setAt(to);
    keep(to);
  };
  const drag = (e: PointerEvent<HTMLDivElement>) => {
    const r = box.current!.getBoundingClientRect();
    setAt(clamp((e.clientX - r.left) / r.width));
  };
  const key = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.key === "ArrowLeft" ? -0.05 : e.key === "ArrowRight" ? 0.05 : 0;
    if (!step) return;
    e.preventDefault();
    settle(clamp(at + step));
  };
  return (
    <div ref={box} className="flex h-full">
      <div style={b ? { width: `${at * 100}%` } : undefined} className={`flex min-w-0 flex-col ${b ? "shrink-0" : "flex-1"}`}>
        {a}
      </div>
      {b && (
        <>
          <div
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize the panes"
            aria-valuenow={Math.round(at * 100)}
            tabIndex={0}
            title="Drag to resize; double-click to even"
            onPointerDown={(e) => {
              e.currentTarget.setPointerCapture(e.pointerId);
              setDragging(true);
            }}
            onPointerMove={(e) => dragging && drag(e)}
            onPointerUp={() => {
              setDragging(false);
              settle(at);
            }}
            onDoubleClick={() => settle(0.5)}
            onKeyDown={key}
            className="group relative z-10 w-px shrink-0 cursor-col-resize bg-hair outline-none"
          >
            {/* A grip wider than the line it draws, as editors' sashes are. */}
            <div className={`absolute inset-y-0 -left-[3px] w-[7px] transition-colors group-hover:bg-frame group-focus-visible:bg-frame ${dragging ? "bg-frame" : ""}`} />
          </div>
          <div className="flex min-w-0 flex-1 flex-col">{b}</div>
        </>
      )}
    </div>
  );
}

/** How wide the window's own content is beside a pane it opens (a
 * subagent's work). */
export const BESIDE = {
  start: () => Number(localStorage.getItem("beside")) || 0.5,
  keep: (at: number) => localStorage.setItem("beside", String(at)),
};
