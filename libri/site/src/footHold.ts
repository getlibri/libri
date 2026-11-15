import { type RefObject, useCallback, useLayoutEffect, useRef } from "react";

/**
 * Keeps the talk from dropping when something at its foot goes away.
 *
 * The thread is read from its foot, so when a tool row ends, folds, or the
 * working book leaves, the talk got shorter, the browser pulled the scroll
 * back onto the new foot, and everything in view fell by that height, only to
 * rise again with the next row: the bounce. As ChatGPT and Claude.ai leave the
 * room a turn has taken below it, the talk here never gets shorter under its
 * reader: a spacer under it takes up whatever the talk gave back, and gives it
 * up again as the talk grows into it, so the next row lands where the blank
 * was and nothing moves.
 *
 * The room is only let go of by the person scrolling up past it: what scrolls
 * out of view below them is cut, so when they come back down the foot is the
 * talk's own again. The chat, Temp and each worker's page hold it alike, and
 * Android's Follow keeps the same room, the same way.
 *
 * The room is capped as ChatGPT and Claude.ai cap it: no more than lets the
 * person's last message stand at the top of the view, under the header its
 * top padding clears, its answer under it. Held whole, a turn whose tool
 * rows fold into one at its end left a screen's height of blank under the
 * answer, the question scrolled off above; and mid-turn a short question
 * stood up under the floating header. So a row folding mid-turn moves
 * nothing unless it would lift the question past that line. Android's
 * Follow caps it the same way.
 *
 * Only a turn's room is held: until a turn has run (`live`) since the hold
 * began, the talk is the old talk settling (markdown set, a reload, a
 * reconnect's refetch), and holding what that gave back left a blank
 * screen's height under the last line, the view pinned on it.
 *
 * A turn's room is what its own rows gave back: the talk below the person's
 * last message, measured from there. Measured over the whole talk, a page
 * opened while a turn ran held what the old talk above it gave back as it
 * settled (1561px on a desktop), and the turn running kept the cap off, so the view stood on a blank screen with the talk
 * scrolled off above it until the turn ended. A new message of the person's
 * starts the measure afresh.
 *
 * `synced` counts the reads of the history that lay the talk out afresh (a
 * load, a return to the page, a reconnect): the talk read back is a new
 * layout, not the turn folding under its reader. Held across one, what the
 * record laid out shorter than the stream had, on coming back to the page
 * mid-turn, stood as blank under the last line, thousands of px of it. The
 * chat's runtime draws the messages a render after they land, so the talk
 * is measured afresh when `drawn` says it drew them, not when they land.
 *
 * It is also the one follow: at the foot it keeps the newest in view, and
 * once the person goes up off the foot nothing moves what they read, not
 * streamed words, a call's row nor the dots, until they come back down.
 */
export function useFootHold(viewport: RefObject<HTMLElement | null>, main: RefObject<HTMLElement | null>, spacer: RefObject<HTMLElement | null>, atFoot: (at: boolean) => void, live: boolean, synced?: unknown) {
  const hold = useRef<Hold>(null);
  // Whether a hold was made here before: the first opens at the foot.
  const made = useRef(false);
  const running = useRef(live);
  running.current = live;
  // A turn ending may change nothing the observer sees (the fold landed while
  // it still ran), so the cap is put on here too.
  useLayoutEffect(() => {
    if (!live) hold.current?.settle();
  }, [live]);
  useLayoutEffect(() => {
    const v = viewport.current;
    const m = main.current;
    const s = spacer.current;
    if (!v || !m || !s) return;
    // A chat opens on its latest, so its first hold is at the foot. Read
    // from where the page stood, it began off the foot: the hold is made
    // after the history is drawn, with the view still at its top, and was
    // only pinned if the first scroll's event happened to land within 70px
    // of the foot; the talk growing in between (the header and the dock's
    // room settling, a photo) lost the race, and the chat opened short of
    // its foot and stayed there (2026-11-09, nix/measure_chat_foot.py).
    const h = (hold.current = holdFoot(v, m, s, atFoot, () => running.current, !made.current));
    made.current = true;
    return () => {
      h.stop();
      hold.current = null;
    };
  }, [viewport, main, spacer, atFoot]);
  const owed = useRef(false);
  useLayoutEffect(() => {
    owed.current = true;
  }, [synced]);
  // The runtime drew the messages: the first drawing after a read is its.
  const drawn = useCallback(() => {
    if (!owed.current) return;
    owed.current = false;
    hold.current?.rebase();
  }, []);
  // Back to the foot and following, as a send does wherever the reader was.
  const toFoot = useCallback(() => hold.current?.pin(), []);
  return { toFoot, drawn };
}

export type Hold = { stop: () => void; pin: () => void; settle: () => void; rebase: () => void };

/** Marks the person's own words in a talk: the chat's, Temp's and a worker's. */
export const ASKED = '[data-said="user"]';

const UP_KEYS = new Set(["ArrowUp", "PageUp", "Home"]);
const ENDS = ["touchend", "touchcancel", "pointerup", "pointercancel"] as const;
const INPUTS = ["pointerdown", "touchstart", "keydown"] as const;

/**
 * The hold itself, on the scrolling viewport `v`, the talk's element `m`, and
 * the spacer `s` at the foot inside it; says whether the reader stands at the
 * foot after each scroll, and `live` whether a turn runs. `start` opens it at
 * the foot; else it stays where the page is. Gives back the means to stop, and
 * to go back to the foot.
 */
export function holdFoot(v: HTMLElement, m: HTMLElement, s: HTMLElement, atFoot: (at: boolean) => void, live: () => boolean, start = false): Hold {
  const gap = () => v.scrollHeight - v.scrollTop - v.clientHeight;
  // The spacer's height, and the tallest the turn's part of the talk has
  // stood since `anchor`, the person's last message, it is measured from.
  let room = 0;
  s.style.height = "0px";
  let anchor: Element | null | undefined;
  let floor = 0;
  // The width the floor was measured at: a talk set at a new width (a wider
  // column, a tile resized) is a new layout, not a shrink under the reader.
  let width = m.offsetWidth;
  // Whether a turn has run since the hold began: before, nothing is held.
  let armed = false;
  let pinned = start || gap() < 8;
  let last = v.scrollTop;
  let lastAway = gap();
  // The person's last message, which a turn's part of the talk is measured from.
  const asked = () => {
    const all = m.querySelectorAll(ASKED);
    return all[all.length - 1] ?? null;
  };
  // How far below the talk's top `question` stands; with none, the talk's top.
  // Whole px, as offsetHeight is: a fraction left a spacer of under a px
  // that came and went on a phone.
  const topOf = (question: Element | null) => (question ? Math.round(question.getBoundingClientRect().top - m.getBoundingClientRect().top) : 0);
  // The most room a finished turn keeps: what puts the person's last message
  // at the top of the view, under the header the talk's top padding clears.
  // With none, the talk's own top, so the blank only fills the view.
  // The view is the scroller less the dock floating over its foot (--dock,
  // the room the talk keeps under it): counted whole, the cap let the blank
  // hold the message up past the first line by the dock's height.
  const cap = (part: number, question: Element | null) => {
    const pad = question ? parseFloat(getComputedStyle(m).paddingTop) || 0 : 0;
    const dock = parseFloat(getComputedStyle(v).getPropertyValue("--dock")) || 0;
    return Math.max(0, v.clientHeight - dock - pad - part);
  };
  const set = (px: number) => {
    room = px;
    s.style.height = `${px}px`;
  };
  // At the foot the follow alone places the talk, and the browser's scroll
  // anchoring is off: WebKit anchors too, on a line of the words streaming
  // in, and moved the talk 25px down the screen for a frame before the
  // follow put it back (nix/check_chat_steady.py). Off the foot, anchoring
  // holds what the person reads.
  const anchoring = () => {
    v.style.overflowAnchor = pinned ? "none" : "";
  };
  // Run before paint, after the layout that changed the talk: a shrink the
  // browser has already clamped the scroll for is put back in the same frame.
  const measure = () => {
    const question = asked();
    // The chat's spacer stands inside its talk, so the talk's height counts
    // it; a thread's (Temp, a memory chat, a worker) stands beside its talk,
    // under the talk's zoom. Taken off there too, the room fed itself: each
    // measure grew it by what the talk last gave back, and the observer on
    // the column measured again, so Temp scrolled away without end on a send.
    const part = m.offsetHeight - (m.contains(s) ? room : 0) - topOf(question);
    // Holding the tallest height across a width change left a spacer the size
    // of what the reflow gave back, and the view pinned on that blank foot.
    if (m.offsetWidth !== width || question !== anchor) {
      width = m.offsetWidth;
      anchor = question;
      floor = part;
    }
    if (live()) armed = true;
    floor = armed ? Math.max(floor, part) : part;
    // Capped while the turn runs too: uncapped, a short question whose
    // answer gave back room mid-turn was held up under the floating header
    // (Temp's Memory and Clear pills, on a phone), above the line the talk
    // starts on.
    if (floor > part) floor = part + Math.min(floor - part, cap(part, question));
    if (floor - part !== room) set(floor - part);
    // Away from the foot nothing is followed: the spacer keeps the scroll
    // from clamping, and the browser's own scroll anchoring holds the rest.
    if (pinned && !held) {
      v.scrollTop = v.scrollHeight;
      last = v.scrollTop;
      lastAway = gap();
    }
    anchoring();
  };
  const scrolled = () => {
    const d = gap();
    if (d > 0 && room > 0) {
      // Below the viewport, so cutting it moves nothing in view.
      const cut = Math.min(d, room);
      floor -= cut;
      set(room - cut);
    }
    // Said here, after the cut, which can bring the reader back to the foot.
    // Off the foot only by going up, away from it: the event of a follow's
    // own scroll lands a frame late, after the talk grew again, and reads
    // as short of the foot though nobody moved; scroll anchoring moves the
    // scroll up by what shrank above, and the foot is as far as it was. A
    // shrink's clamp lands on the foot.
    const top = v.scrollTop;
    const away = gap();
    // Going down, near is enough, as use-stick-to-bottom's 70px: a foot
    // still growing runs ahead of a wheel's last notch.
    if (away < 8 || (top > last && away < 70)) pinned = true;
    else if (byPerson() && top < last - 1 && away > lastAway + 1) pinned = false;
    // Pinned and moved off the foot by no one: the browser clamped the
    // scroll while the talk was shorter for a moment (WebKit, as a streamed
    // block is swapped for its next), and the talk grew back before this
    // event. Put back here, before the frame is drawn: left to the next
    // resize, the talk stood 25px low for a frame (nix/check_chat_steady.py).
    if (pinned && !held && away > 0) {
      v.scrollTop = v.scrollHeight;
      lastAway = gap();
      last = v.scrollTop;
      anchoring();
      atFoot(pinned);
      return;
    }
    last = top;
    lastAway = away;
    anchoring();
    atFoot(pinned);
  };
  // A hand on the talk owns its scroll: while a finger, a wheel or the
  // scrollbar's thumb holds it, nothing is followed, so the talk growing under the
  // hand can't pull it back down before its scroll reads as going up; a
  // wheel or key going up lets go at once, for the same reason. As
  // use-stick-to-bottom does; assistant-ui's own follow, which counted a
  // scroll as the person's only while the page's height stood still, lost
  // the hand mid-stream. A hand let go at the foot catches up.
  let held = false;
  const hold = () => {
    held = true;
  };
  // A scroll going up is the person's only with their hand on the talk or
  // just after they touched, clicked or pressed a key (a quote's jump, a
  // search hit, a key outside the composer). Any other is the page's or the
  // browser's: WebKit clamps the scroll as a streamed message is swapped for
  // its kept copy, and the event, landing after the talk grew back, read as
  // going up, so the chat let go of its foot mid-reply and stood 3000px
  // above it (nix/measure_chat_foot.py).
  let input = -Infinity;
  const inputs = (e: Event) => {
    if (e.type === "keydown" && (e.target as Element | null)?.closest?.("textarea, input, [contenteditable]")) return;
    input = performance.now();
  };
  const byPerson = () => held || performance.now() - input < 1000;
  const holdThumb = (e: PointerEvent) => {
    if (e.target === v) held = true;
  };
  const release = () => {
    if (!held) return;
    held = false;
    if (pinned) measure();
  };
  // A wheel is a hand on the talk until it rests: a browser's smooth wheel
  // scroll, followed past where it was heading, pulls back up to its end,
  // which reads as the person going up.
  let wheeling: number | undefined;
  const wheeled = (e: WheelEvent) => {
    if (e.deltaY < 0) {
      pinned = false;
      anchoring();
    }
    held = true;
    window.clearTimeout(wheeling);
    wheeling = window.setTimeout(release, 250);
  };
  const keyed = (e: KeyboardEvent) => {
    if (UP_KEYS.has(e.key) && !(e.target as Element | null)?.closest?.("textarea, input, [contenteditable]")) {
      pinned = false;
      anchoring();
    }
  };
  // A fold opened is read where it opened, so the words streaming under it
  // don't push it off the screen; scrolling back to the foot follows again,
  // as assistant-ui's pause on an opened disclosure did.
  const opened = (e: MouseEvent) => {
    const fold = (e.target as Element | null)?.closest?.('[aria-expanded="false"], summary');
    if (fold && m.contains(fold)) {
      pinned = false;
      anchoring();
    }
  };
  const watch = new ResizeObserver(measure);
  watch.observe(m);
  // The view's own height changes with the talk unchanged: an iPhone's
  // keyboard, Safari's toolbar settling on a reload or a return to the page,
  // the dock over the talk growing (the chat's room for it is a sibling of
  // the talk, inside the viewport). Watched through the talk alone, the foot
  // slid under the composer by as much and stayed there, the chat standing
  // "too high": 254px of the last message hidden for a keyboard's 300px
  // (nix/measure_chat_foot.py).
  watch.observe(v);
  for (const c of v.children) if (c !== m) watch.observe(c);
  v.addEventListener("click", opened, { capture: true });
  for (const kind of INPUTS) document.addEventListener(kind, inputs, { capture: true, passive: true });
  v.addEventListener("scroll", scrolled, { passive: true });
  v.addEventListener("touchstart", hold, { passive: true });
  v.addEventListener("pointerdown", holdThumb);
  for (const end of ENDS) window.addEventListener(end, release, { passive: true });
  v.addEventListener("wheel", wheeled, { passive: true });
  v.addEventListener("keydown", keyed);
  measure();
  const pin = () => {
    pinned = true;
    measure();
  };
  const stop = () => {
    window.clearTimeout(wheeling);
    watch.disconnect();
    v.removeEventListener("scroll", scrolled);
    v.removeEventListener("touchstart", hold);
    v.removeEventListener("pointerdown", holdThumb);
    for (const end of ENDS) window.removeEventListener(end, release);
    v.removeEventListener("wheel", wheeled);
    v.removeEventListener("keydown", keyed);
    v.removeEventListener("click", opened, { capture: true });
    v.style.overflowAnchor = "";
    for (const kind of INPUTS) document.removeEventListener(kind, inputs, { capture: true });
    s.style.height = "0px";
  };
  // The talk laid out afresh: measured from here on, as on a new message.
  const rebase = () => {
    anchor = undefined;
    measure();
  };
  return { stop, pin, settle: measure, rebase };
}
