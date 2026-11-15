// The header pill's button, carried to the ✕ of the column it opens. The pill
// closes up at once; what left it crosses the page alongside the column's
// slide, drawn as that part of the bubble and landing as the ✕, so the eye
// reads one control. Plain DOM on the body: the column clips its own box and
// the page's frame has layout containment, so nothing inside them could make
// the trip.

/** The columns' slide, which the carry keeps time with, and a move with it
 * (--slide-time and --slide-curve in index.css). It eases to a stop and never
 * overshoots: bounce and springing width make people motion sick. */
export const SLIDE = { duration: 280, easing: "cubic-bezier(0.32, 0.72, 0, 1)" };

let lifted: { box: HTMLElement; left: number; top: number } | undefined;

/** Takes a button as the pill gives it up, before the column opens. Under
 * reduced motion nothing travels. */
export function lift(button: HTMLElement) {
  lifted = undefined;
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  const { left, top } = button.getBoundingClientRect();
  const box = document.createElement("div");
  box.ariaHidden = "true";
  box.className = "pointer-events-none fixed z-50 grid size-9 place-items-center text-mid";
  box.style.left = `${left}px`;
  box.style.top = `${top}px`;
  // The bubble's height round the button: its padding and border.
  const bubble = document.createElement("div");
  bubble.className = "absolute -inset-[5px] rounded-full border border-hair bg-lift shadow-dock";
  box.append(bubble, button.firstElementChild!.cloneNode(true));
  lifted = { box, left, top };
}

/** Sends what was lifted to `close`. Called with the column laid out whole,
 * before its slide starts, so `close` stands where the trip ends. */
export function land(close: HTMLElement) {
  if (!lifted) return;
  const { box, left, top } = lifted;
  lifted = undefined;
  const end = close.getBoundingClientRect();
  const [bubble, from] = box.children as HTMLCollectionOf<HTMLElement>;
  const to = close.firstElementChild!.cloneNode(true) as HTMLElement;
  box.append(to);
  for (const glyph of [from, to]) glyph.style.gridArea = "1 / 1";
  document.body.append(box);
  box.animate({ transform: ["none", `translate(${end.left - left}px, ${end.top - top}px)`] }, SLIDE).onfinish = () => box.remove();
  bubble.animate({ opacity: [1, 0] }, SLIDE);
  from.animate({ opacity: [1, 0] }, SLIDE);
  to.animate({ opacity: [0, 1] }, SLIDE);
  close.animate({ opacity: [0, 0] }, SLIDE);
}
