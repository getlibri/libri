// Libri's own confirm and notice, in place of the browser's confirm() and
// alert(): the page's paper, type and buttons rather than a system box.
// Each is a call that waits for the answer, as the browser's are, and
// mounts itself, so any page (the app, a whiteboard alone) can ask.
//
//   if (!(await confirmAction({ title: "Delete the board Plans?", body: "…", confirm: "Delete board" }))) return;
//   await tell({ title: "Pairing failed", body: reason });
//
// Esc, a click outside and Cancel answer no; Enter on a focused button
// presses it. A destructive confirm starts with Cancel focused, so a stray
// Enter keeps what is there.

import { useEffect, useRef, type ReactNode } from "react";
import { createRoot } from "react-dom/client";

type Asked = {
  title: string;
  body?: ReactNode;
  /** The button that does it, named for what it does ("Delete board"). */
  confirm?: string;
  cancel?: string;
  /** It can't be undone: the button in the accent colour, Cancel first. */
  danger?: boolean;
};

function Dialog({ asked, answer, notice }: { asked: Asked; answer: (yes: boolean) => void; notice: boolean }) {
  const cancel = useRef<HTMLButtonElement>(null);
  const go = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    (asked.danger && !notice ? cancel : go).current?.focus();
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      answer(false);
    };
    // Capture: the page's own keys (a tile's, the composer's) never see it.
    document.addEventListener("keydown", key, true);
    return () => document.removeEventListener("keydown", key, true);
  }, [asked, answer, notice]);
  return (
    <div className="fixed inset-0 z-[100] grid place-items-center bg-black/25 p-6" onClick={() => answer(false)} data-libri-dialog>
      <div
        role={notice ? "dialog" : "alertdialog"}
        aria-modal
        aria-labelledby="libri-dialog-title"
        onClick={(e) => e.stopPropagation()}
        className="flex w-full max-w-[400px] flex-col gap-2 rounded-2xl border-[1.25px] border-edge bg-paper p-5 shadow-lift"
      >
        <h2 id="libri-dialog-title" className="text-body font-medium text-ink">
          {asked.title}
        </h2>
        {asked.body && <div className="text-label text-mid">{asked.body}</div>}
        <div className="mt-3 flex justify-end gap-2">
          {!notice && (
            <button ref={cancel} type="button" onClick={() => answer(false)} className="rounded-full px-4 py-1.5 text-label text-mid hover:bg-hover hover:text-ink">
              {asked.cancel ?? "Cancel"}
            </button>
          )}
          <button
            ref={go}
            type="button"
            onClick={() => answer(true)}
            className={`rounded-full px-4 py-1.5 text-label ${asked.danger ? "bg-accent text-paper" : "bg-ink text-paper"} hover:opacity-90`}
          >
            {asked.confirm ?? (notice ? "OK" : "Continue")}
          </button>
        </div>
      </div>
    </div>
  );
}

function show(asked: Asked, notice: boolean): Promise<boolean> {
  return new Promise((done) => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    const before = document.activeElement as HTMLElement | null;
    const answer = (yes: boolean) => {
      root.unmount();
      host.remove();
      before?.focus?.();
      done(yes);
    };
    root.render(<Dialog asked={asked} answer={answer} notice={notice} />);
  });
}

/** Asks before doing something; true for yes. */
export const confirmAction = (asked: Asked) => show(asked, false);

/** Says something that needs the person's eye; resolves once read. */
export const tell = (asked: Asked) => show(asked, true).then(() => undefined);
