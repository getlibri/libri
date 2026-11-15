// What the person opens in a chat stays in view: something opened off screen
// scrolls down until its top fits, as the person asked. Opening a row, a fold or a card scrolls its chat so the
// opened block shows whole, or, when it is taller than the view, so its top
// sits just under the chat's head; closing one keeps the row tapped where it
// was. Only the person's own tap moves the view: nothing here runs while a
// turn streams on its own.

/** The chat that scrolls `el`, or the page. */
function scrollerOf(el: Element): HTMLElement {
  return el.closest<HTMLElement>("[data-chat-scroll]") ?? (document.scrollingElement as HTMLElement);
}

/** A length the chat's own styles give, in pixels; 0 where none. */
function px(el: Element, name: string): number {
  return parseFloat(getComputedStyle(el).getPropertyValue(name)) || 0;
}

/** The band of `scroller` clear of what floats over it: its head above
 * (--talk-top, the room the first line keeps) and its dock below (--dock). */
function band(scroller: HTMLElement) {
  const r = scroller === document.scrollingElement ? new DOMRect(0, 0, innerWidth, innerHeight) : scroller.getBoundingClientRect();
  return { top: r.top + px(scroller, "--talk-top"), bottom: r.bottom - px(scroller, "--dock") };
}

/** Room left between the block and the band's edges. */
const MARGIN = 8;

/** How far `scroller` must move to show `block` as the person wants it. */
function needed(block: Element, scroller: HTMLElement): number {
  const { top, bottom } = band(scroller);
  const r = block.getBoundingClientRect();
  if (r.height > bottom - top - 2 * MARGIN) return r.top - (top + MARGIN);
  if (r.bottom > bottom - MARGIN) return r.bottom - (bottom - MARGIN);
  if (r.top < top + MARGIN) return r.top - (top + MARGIN);
  return 0;
}

/** After the person opens `block`: scroll its chat, smoothly and briefly,
 * until it is in view. A block that grows as it loads (a card) is followed
 * for a moment, so it lands in view at its full height. */
export function reveal(block: Element | null) {
  if (!block) return;
  const scroller = scrollerOf(block);
  const move = () => {
    const by = needed(block, scroller);
    if (Math.abs(by) > 1) scroller.scrollBy({ top: by, behavior: "smooth" });
  };
  requestAnimationFrame(move);
  const grows = new ResizeObserver(move);
  grows.observe(block);
  setTimeout(() => grows.disconnect(), 900);
}

/** Before the person closes something under `row`: keeps `row` where it
 * stands on the screen once what it held is gone. */
export function hold(row: Element | null) {
  if (!row) return;
  const scroller = scrollerOf(row);
  const was = row.getBoundingClientRect().top;
  // Kept for a moment, frame by frame: the chat's own foot keeping moves
  // the talk again as the room under it settles.
  const until = performance.now() + 400;
  const keep = () => {
    const by = row.getBoundingClientRect().top - was;
    if (Math.abs(by) > 1) scroller.scrollBy({ top: by });
    if (performance.now() < until) requestAnimationFrame(keep);
  };
  requestAnimationFrame(keep);
}
