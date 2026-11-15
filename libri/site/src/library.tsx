// The Library: what the assistant made and keeps for the person, the pages
// show_page published and the lists in its book, as a phone's files or a
// chat app's gallery of made things lists them. A page opens in place as
// an MCP App (card.tsx PageFrame), live: what it keeps, the person and the
// agent both change; + picks a kind and asks the chat for a new one in a
// composer at the panel's foot.
// The watches the Tasks page lists are here too: they are the book's as
// well, and a snooze or a delete edits them with no model call.
import { Fragment, lazy, Suspense, useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { useTileRows } from "./tileMenu";
import { api, url } from "./api";
import { joined } from "./dictate";
import { draft, forHost, keepDraft } from "./keep";
import { useTheme } from "./theme";
import { Held, HoldMenu, type Grip, type MenuItem } from "./hold";
import { napPresets, wakeWords } from "./nap";

// The MCP Apps host is loaded when a page is opened, as the chat loads it
// for a card.
const PageFrame = lazy(() => import("./card").then((m) => ({ default: m.PageFrame })));
const BuiltinFrame = lazy(() => import("./card").then((m) => ({ default: m.BuiltinFrame })));

export type LibraryPage = { name: string; title?: string | null; changed: number };
export type LibraryList = { name: string; items: string[] };
/** What the Library opens: a page, a list, or the whole of it. */
export type Shelved = { page: string } | { list: string } | { all: true };

export const sameShelved = (a?: Shelved, b?: Shelved) => JSON.stringify(a) === JSON.stringify(b);

/** What a host from before the Library sends: nothing, which reads as empty.
 * `stills` says the host renders a still of each page (snapshot.rs); with
 * none, a card shows its title alone and asks for no image. */
type Read = { pages: LibraryPage[]; lists: LibraryList[]; stills?: boolean };

/** The Library as last read, asked again while in view and when the page
 * comes back to the front: show_page writes it from any turn. */
export function useLibrary(looking: boolean) {
  const [read, setRead] = useState<Read>();
  const last = useRef("");
  const ask = useCallback(async () => {
    const r = await api("api/library").catch(() => undefined);
    if (!r?.ok || !r.headers.get("content-type")?.includes("json")) return setRead((v) => v ?? { pages: [], lists: [] });
    const text = await r.text();
    if (text === last.current) return;
    last.current = text;
    setRead(JSON.parse(text) as Read);
  }, []);
  useEffect(() => {
    void ask();
    if (!looking) return;
    const again = () => document.visibilityState === "visible" && void ask();
    const id = window.setInterval(again, 15000);
    document.addEventListener("visibilitychange", again);
    return () => {
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", again);
    };
  }, [ask, looking]);
  return { pages: read?.pages ?? [], lists: read?.lists ?? [], stills: !!read?.stills, loaded: !!read, ask };
}
export type Library = ReturnType<typeof useLibrary>;

const pageName = (p: LibraryPage) => p.title || p.name;

/** When something changed, as a list's few words. */
function when(secs: number) {
  const d = new Date(secs * 1000);
  const today = new Date().toDateString() === d.toDateString();
  return today
    ? d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
    : d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

const Icon = ({ children, className = "size-[18px]" }: { children: ReactNode; className?: string }) => (
  <svg viewBox="0 0 24 24" className={`${className} shrink-0`} fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    {children}
  </svg>
);
export const PageMark = () => (
  <>
    <path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8Z" />
    <path d="M14 3v5h5M9 13h6M9 17h4" />
  </>
);
const ListMark = () => <path d="M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01" />;
/** The Library's own mark: books on a shelf. */
export const LibraryMark = () => (
  <>
    <path d="M5 4h3v16H5zM10 4h3v16h-3z" />
    <path d="m15.5 5.2 2.9-.8 3.6 15-2.9.8z" />
  </>
);

/** The words the Library's composer sends the chat: a request, as the
 * person might word it, so the model makes it with whatever tool suits. The
 * kind only words the request; the host knows no kinds. */
const asking = (what: string, kind?: Kind) =>
  `Make this for my Library, as a page on my site${kind ? `, ${KINDS[kind].ask}` : ""}: ${what}`;

/** What + offers to make, each a word and a mark, as a maker's menu lists
 * its kinds. */
const KINDS = {
  page: { name: "Web page", ask: "a web page", mark: <><circle cx="12" cy="12" r="8.5" /><path d="M3.5 12h17M12 3.5c2.5 2.6 3.5 5.4 3.5 8.5s-1 5.9-3.5 8.5c-2.5-2.6-3.5-5.4-3.5-8.5s1-5.9 3.5-8.5Z" /></> },
  document: { name: "Document", ask: "a document", mark: <PageMark /> },
  slides: { name: "Presentation", ask: "a presentation", mark: <><rect x="3.5" y="4.5" width="17" height="12" rx="1.5" /><path d="M12 16.5V20M8.5 20h7" /></> },
  design: { name: "Design", ask: "a design", mark: <><path d="m14.5 5.5 4 4L9 19H5v-4Z" /><path d="m12.5 7.5 4 4" /></> },
  dashboard: { name: "Dashboard", ask: "a dashboard", mark: <><rect x="4" y="4" width="7" height="9" rx="1.5" /><rect x="13" y="4" width="7" height="5" rx="1.5" /><rect x="13" y="11" width="7" height="9" rx="1.5" /><rect x="4" y="15" width="7" height="5" rx="1.5" /></> },
} as const;
type Kind = keyof typeof KINDS;

/** + at the Library's lower right, small, and the menu of kinds it opens
 * just above it. A kind picked opens a composer at the panel's foot holding
 * it as a chip, in place of a box at the top far from the thumb. */
function Maker({ say }: { say: (text: string) => void }) {
  const [menu, setMenu] = useState(false);
  const [composing, setComposing] = useState(false);
  const [kind, setKind] = useState<Kind>();
  const [text, setText] = useState("");
  const done = () => {
    setComposing(false);
    setKind(undefined);
    setText("");
  };
  useEffect(() => {
    if (!menu) return;
    const key = (e: KeyboardEvent) => e.key === "Escape" && setMenu(false);
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  }, [menu]);
  if (composing)
    return (
      <form
        className="shrink-0 border-hair bg-paper p-3 lg:border-t"
        onSubmit={(e) => {
          e.preventDefault();
          if (!text.trim()) return;
          say(asking(text.trim(), kind));
          done();
        }}
      >
        <div className="flex flex-col gap-2 rounded-xl border border-frame bg-lift px-3 py-2.5 focus-within:border-soft">
          {kind && (
            <span className="flex w-fit items-center gap-1.5 rounded-full bg-hover py-0.5 pr-1 pl-2.5 text-label text-ink">
              <Icon className="size-3.5">{KINDS[kind].mark}</Icon>
              {KINDS[kind].name}
              <button type="button" onClick={() => setKind(undefined)} aria-label={`Remove ${KINDS[kind].name}`} className="grid size-5 place-items-center rounded-full text-mid hover:bg-paper hover:text-ink">
                <Icon className="size-3"><path d="M18 6 6 18M6 6l12 12" /></Icon>
              </button>
            </span>
          )}
          <div className="flex items-end gap-2">
            <textarea
              autoFocus
              rows={1}
              value={text}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") done();
                else if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  e.currentTarget.form?.requestSubmit();
                }
              }}
              aria-label="Describe what to build"
              placeholder="Describe what to build"
              className="field-sizing-content max-h-40 min-w-0 flex-1 resize-none bg-transparent py-1 text-body text-ink outline-none placeholder:text-soft"
            />
            {text.trim() ? (
              <button type="submit" aria-label="Ask" className="grid size-8 shrink-0 place-items-center rounded-full bg-accent text-on-accent hover:brightness-110">
                <Icon className="size-4"><path d="M12 19V5M5 12l7-7 7 7" /></Icon>
              </button>
            ) : (
              <button type="button" onClick={done} aria-label="Close" className="grid size-8 shrink-0 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink">
                <Icon className="size-4"><path d="M18 6 6 18M6 6l12 12" /></Icon>
              </button>
            )}
          </div>
        </div>
      </form>
    );
  return (
    <>
      {menu && <div onClick={() => setMenu(false)} aria-hidden className="absolute inset-0 z-20" />}
      {menu && (
        <ul role="menu" aria-label="Make" className="absolute right-4 bottom-[calc(68px+var(--bar-room,0px))] z-20 flex w-48 flex-col rounded-xl border border-hair bg-lift p-1 shadow-lift">
          {(Object.keys(KINDS) as Kind[]).map((k) => (
            <li key={k}>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenu(false);
                  setKind(k);
                  setComposing(true);
                }}
                className="flex w-full items-center gap-3 rounded-lg px-2.5 py-2 text-left text-control text-ink hover:bg-hover"
              >
                <span className="text-mid"><Icon className="size-[18px]">{KINDS[k].mark}</Icon></span>
                {KINDS[k].name}
              </button>
            </li>
          ))}
        </ul>
      )}
      <PlusButton label={menu ? "Close" : "Make something"} on={menu} onClick={() => setMenu((m) => !m)} expanded={menu} />
    </>
  );
}

/** The + at a column's lower right, 16px in: on a wide screen small, the
 * scale of a chat app's own (40px); below 1024px Android's 56px Fab (Kit.kt),
 * Material's size, where the thumb reaches. */
export function PlusButton({ label, on, onClick, expanded }: { label: string; on: boolean; onClick: () => void; expanded: boolean }) {
  return (
    <button
      type="button"
      aria-label={label}
      aria-expanded={expanded}
      onClick={onClick}
      className="absolute right-4 bottom-[calc(16px+var(--bar-room,0px))] z-20 grid size-14 place-items-center rounded-full bg-accent text-on-accent shadow-lift hover:brightness-110 lg:size-10"
    >
      <Icon className="size-6 lg:size-5">{on ? <path d="M6 6l12 12M18 6L6 18" /> : <path d="M12 5v14M5 12h14" />}</Icon>
    </button>
  );
}

/** The foot a list under the floating + pads: the button, its 16px gap,
 * and 16px again, so the last row scrolls clear (Android's FAB_CLEARANCE);
 * on a phone the floating bar's room too (--bar-room, App.tsx Places). */
export const FAB_CLEARANCE = "pb-[calc(88px+var(--bar-room,0px))] lg:pb-[72px]";

/** A list running on under the floating bar fades out as it passes under
 * it (index.css edge-fade), and 24px above it, inside the list's padding. */
export const BAR_FADE = "edge-fade [--fade-bottom:calc(var(--bar-room,0px)+24px)]";

/** How long ago something changed, as a card's few words: "9 min ago",
 * "3 days ago", then the date. */
function ago(secs: number) {
  const gone = Math.max(0, Date.now() / 1000 - secs);
  const words = new Intl.RelativeTimeFormat(undefined, { numeric: "auto", style: "short" });
  if (gone < 60) return "just now";
  if (gone < 3600) return words.format(-Math.floor(gone / 60), "minute");
  if (gone < 86400) return words.format(-Math.floor(gone / 3600), "hour");
  if (gone < 7 * 86400) return words.format(-Math.floor(gone / 86400), "day");
  return when(secs);
}

/** The groups a gallery's pages fall in, newest first, as a phone's photos
 * and Duet's Library group made things by when they changed. */
function grouped(pages: LibraryPage[]) {
  const midnight = new Date().setHours(0, 0, 0, 0) / 1000;
  const groups: { name: string; pages: LibraryPage[] }[] = [
    { name: "Today", pages: [] },
    { name: "This week", pages: [] },
    { name: "Earlier", pages: [] },
  ];
  for (const p of [...pages].sort((a, b) => b.changed - a.changed))
    groups[p.changed >= midnight ? 0 : p.changed >= midnight - 6 * 86400 ? 1 : 2].pages.push(p);
  return groups.filter((g) => g.pages.length > 0);
}

/** How long a card waits to ask again for a still the host has not yet
 * rendered, and how many times: a page published a moment ago gets one
 * within a render or two. */
const STILL_AGAIN = 15000;
const STILL_TRIES = 4;

/** A page's own faint hue, from its name, so cards waiting for their
 * stills read apart, mixed into the paper of either theme. */
function tint(name: string) {
  // FNV-1a, so names a letter apart land far apart on the wheel.
  let h = 0x811c9dc5;
  for (const c of name) h = Math.imul(h ^ c.charCodeAt(0), 0x01000193) >>> 0;
  return `color-mix(in oklch, var(--paper) 84%, oklch(0.62 0.1 ${h % 360}))`;
}

/** A page as it looks, small: a still the host renders once per version
 * (snapshot.rs), shown as a plain image in a box of fixed shape, so
 * nothing shifts as it arrives. Until it does, and on a host with no
 * browser to render, the box is tinted, a hue of the page's own. It
 * paints over the tint as it decodes, with no fade, and is never hidden
 * meanwhile: a fade on every card cost WebKit half its frames as a long
 * gallery scrolled, and WebKit never loads a lazy image that is hidden
 * (check_library_gallery.py). The live
 * page runs only when it is opened. */
function Thumb({ name, changed, small, stills }: { name: string; changed: number; small?: boolean; stills: boolean }) {
  const [shown, setShown] = useState(false);
  const [tries, setTries] = useState(0);
  const [missing, setMissing] = useState(false);
  useEffect(() => {
    setShown(false);
    setTries(0);
    setMissing(false);
  }, [changed]);
  useEffect(() => {
    if (!stills || !missing || tries >= STILL_TRIES) return;
    const id = window.setTimeout(() => {
      setMissing(false);
      setTries((t) => t + 1);
    }, STILL_AGAIN);
    return () => window.clearTimeout(id);
  }, [stills, missing, tries]);
  return (
    <div className="relative aspect-[4/3] overflow-hidden" style={{ background: tint(name) }} aria-hidden>
      {!small && !shown && (
        <span className="absolute inset-0 grid place-items-center text-soft/60">
          <Icon className="size-7"><PageMark /></Icon>
        </span>
      )}
      {stills && !missing && (
        <img
          key={tries}
          src={url(`api/library/${encodeURIComponent(name)}/snapshot?v=${changed}`, true)}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          onLoad={() => setShown(true)}
          onError={() => setMissing(true)}
          className="absolute inset-0 size-full object-cover object-top"
        />
      )}
    </div>
  );
}

const card = "group flex w-full flex-col overflow-hidden rounded-xl border border-hair bg-lift text-left shadow-lift transition-colors hover:border-frame focus-visible:outline-2 focus-visible:outline-accent";
const here = "border-frame ring-2 ring-accent/40";

/** A page's card: how it looks, its title, and when it changed. */
function PageCard({ p, opened, open, stills }: { p: LibraryPage; opened: boolean; open: () => void; stills: boolean }) {
  return (
    <li>
      <button type="button" onClick={open} aria-current={opened ? "page" : undefined} title={pageName(p)} className={`${card} ${opened ? here : ""}`}>
        <Thumb name={p.name} changed={p.changed} stills={stills} />
        <span className="flex flex-col gap-1 border-t border-hair px-3 pt-2 pb-2.5">
          <span className="line-clamp-2 text-label leading-snug font-medium text-ink">{pageName(p)}</span>
          <span className="flex items-center gap-1.5 text-caption text-soft">
            <Icon className="size-3.5"><PageMark /></Icon>
            <span className="tabular-nums">{ago(p.changed)}</span>
          </span>
        </span>
      </button>
    </li>
  );
}

/** A list's card: its first items in place of a picture. */
function ListCard({ l, opened, open }: { l: LibraryList; opened: boolean; open: () => void }) {
  return (
    <li>
      <button type="button" onClick={open} aria-current={opened ? "page" : undefined} title={l.name} className={`${card} ${opened ? here : ""}`}>
        <span className="flex aspect-[4/3] flex-col gap-1 overflow-hidden bg-paper px-3 pt-3 text-caption leading-snug text-mid" aria-hidden>
          {l.items.slice(0, 5).map((item, i) => (
            <span key={i} className="flex items-baseline gap-2">
              <span className="size-1 shrink-0 translate-y-[-2px] rounded-full bg-soft" />
              <span className="truncate">{item}</span>
            </span>
          ))}
          {l.items.length === 0 && <span className="text-soft">Empty</span>}
        </span>
        <span className="flex flex-col gap-1 border-t border-hair px-3 pt-2 pb-2.5">
          <span className="line-clamp-2 text-label leading-snug font-medium text-ink">{l.name}</span>
          <span className="flex items-center gap-1.5 text-caption text-soft">
            <Icon className="size-3.5"><ListMark /></Icon>
            <span className="tabular-nums">
              {l.items.length} {l.items.length === 1 ? "item" : "items"}
            </span>
          </span>
        </span>
      </button>
    </li>
  );
}

const head = "flex items-baseline gap-2 px-1 pb-2 text-label font-semibold text-ink";
const count = "text-caption font-normal text-soft tabular-nums";
/** As many columns as fit at 150px or more: two on a phone, more beside a
 * wide screen's chat. */
const grid = "grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-3";
/** Kinds the Library will keep beside pages and lists, shown as stubs so
 * the shelf says where they will go. Not built yet. */
const LATER = [
  { name: "Notes", mark: <><path d="M5 4h14v16H5z" /><path d="M8.5 8.5h7M8.5 12h7M8.5 15.5h4" /></> },
  { name: "Reminders", mark: <><circle cx="12" cy="13" r="7" /><path d="M12 9.5V13l2.5 1.5M5 4.5 3 6.5M19 4.5l2 2" /></> },
];

/** The whole Library, as its gallery shows it: pages as cards grouped by when they changed, then the lists. */
export function LibraryShelf({ library, open, opened, find = "" }: { library: Library; open: (s: Shelved) => void; opened?: Shelved; find?: string }) {
  const { loaded } = library;
  const words = find.trim().toLowerCase();
  const pages = words ? library.pages.filter((p) => `${pageName(p)} ${p.name}`.toLowerCase().includes(words)) : library.pages;
  const lists = words ? library.lists.filter((l) => `${l.name} ${l.items.join(" ")}`.toLowerCase().includes(words)) : library.lists;
  return (
    <div className="mx-auto flex w-full max-w-[1120px] flex-col gap-6 px-2 pt-4 pb-10">
      {loaded && library.pages.length === 0 && library.lists.length === 0 && (
        <div className="flex flex-col items-center gap-3 rounded-2xl border border-dashed border-frame px-6 py-10 text-center">
          <span className="text-soft"><Icon className="size-8"><LibraryMark /></Icon></span>
          <p className="max-w-[34ch] text-control text-mid">Pages the assistant makes for you and the lists it keeps are shelved here. + asks for one.</p>
        </div>
      )}
      {words && pages.length === 0 && lists.length === 0 && <p className="px-1 text-control text-mid">Nothing in the Library matches.</p>}
      {grouped(pages).map((g) => (
        <section key={g.name} aria-label={g.name}>
          <h2 className={head}>
            {g.name} <span className={count}>{g.pages.length}</span>
          </h2>
          <ul className={grid}>
            {g.pages.map((p) => (
              <PageCard key={p.name} p={p} opened={sameShelved(opened, { page: p.name })} open={() => open({ page: p.name })} stills={library.stills} />
            ))}
          </ul>
        </section>
      ))}
      {lists.length > 0 && (
        <section aria-label="Lists">
          <h2 className={head}>
            Lists <span className={count}>{lists.length}</span>
          </h2>
          <ul className={grid}>
            {lists.map((l) => (
              <ListCard key={l.name} l={l} opened={sameShelved(opened, { list: l.name })} open={() => open({ list: l.name })} />
            ))}
          </ul>
        </section>
      )}
      {!words && (
        <section aria-label="Coming later">
          <h2 className={head}>Coming later</h2>
          <ul className={grid}>
            {LATER.map((k) => (
              <li key={k.name} aria-disabled className="flex flex-col overflow-hidden rounded-xl border border-dashed border-frame">
                <span className="grid aspect-[4/3] place-items-center text-soft" aria-hidden>
                  <Icon className="size-7">{k.mark}</Icon>
                </span>
                <span className="flex flex-col gap-1 border-t border-dashed border-frame px-3 pt-2 pb-2.5">
                  <span className="text-label leading-snug font-medium text-mid">{k.name}</span>
                  <span className="text-caption text-soft">Not built yet</span>
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

/** What the Library opened, whole: a page in its frame, a list's items, or
 * the gallery, with its head: title, find, close, as a chat app's side
 * panel has. `close` puts it away; an item's `back` steps to the gallery. */
/** `say` sends the person's words to the chat; `compose` puts a page's
 * message in the chat's composer, for the person to send or not. */
/** A link in a window of its own, as Open in window gives a page. */
const open_ = (href: string) => window.open(href, "_blank", "noopener,width=1100,height=800");

export function LibraryItem({ shelved, library, open, close, say, compose, back }: { shelved: Shelved; library: Library; open: (s: Shelved) => void; close?: () => void; say: (text: string) => void; compose: (text: string) => void; back?: () => void }) {
  // The gallery's find box, opened from its head as a chat app's search is.
  const [finding, setFinding] = useState(false);
  const [find, setFind] = useState("");
  const page = "page" in shelved ? library.pages.find((p) => p.name === shelved.page) : undefined;
  const list = "list" in shelved ? library.lists.find((l) => l.name === shelved.list) : undefined;
  const title = "all" in shelved ? "Library" : page ? pageName(page) : list ? list.name : "page" in shelved ? shelved.page : shelved.list;
  // The tile menu's Reload draws the page afresh; Open in window gives it a
  // browser window of its own, at its own link.
  const [drawn, setDrawn] = useState(0);
  useTileRows(
    "library",
    page
      ? {
          "reload": { run: () => setDrawn((n) => n + 1) },
          "window": { run: () => void open_(`${location.origin}${location.pathname}#page=${encodeURIComponent(page.name)}`) },
        }
      : {},
  );
  return (
    <div data-surface="library" className="relative flex h-full min-w-0 flex-col bg-paper">
      {/* On a phone no head: its buttons float at the
          top over the shelf, as the chat's do, and the bar names it. */}
      <header className="flex h-[51px] shrink-0 items-center gap-1 border-hair px-2 lg:h-[70px] lg:border-b lg:pr-3 phone:pointer-events-none phone:absolute phone:inset-x-0 phone:top-[calc(var(--tile-top,0px)+10px)] phone:z-20 phone:h-11 phone:gap-2 phone:border-0 phone:px-3.5">
        {back && (
          <button type="button" onClick={back} aria-label="Back to the Library" className="grid size-9 shrink-0 phone:pointer-events-auto phone:floating phone:size-11 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink">
            <Icon className="size-5"><path d="m15 18-6-6 6-6" /></Icon>
          </button>
        )}
        {/* No title on a phone, as Chat has none: the bar names it. */}
        <h1 className={`min-w-0 flex-1 truncate text-prose font-semibold phone:invisible ${back ? "px-1" : "px-2"} ${"all" in shelved ? "max-lg:invisible" : ""}`}>{title}</h1>
        {"all" in shelved && (
          <button type="button" onClick={() => (finding ? (setFinding(false), setFind("")) : setFinding(true))} aria-label="Find in the Library" aria-pressed={finding} className={`grid size-9 shrink-0 phone:pointer-events-auto phone:floating phone:size-11 place-items-center rounded-full hover:bg-hover hover:text-ink ${finding ? "text-ink" : "text-mid"}`}>
            <Icon className="size-[18px]"><circle cx="11" cy="11" r="6.5" /><path d="m20 20-4.2-4.2" /></Icon>
          </button>
        )}
        {close && (
          <button type="button" onClick={close} aria-label="Close" className="grid size-9 shrink-0 phone:pointer-events-auto phone:floating phone:size-11 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink">
            <Icon className="size-5"><path d="M18 6 6 18M6 6l12 12" /></Icon>
          </button>
        )}
      </header>
      {"page" in shelved ? (
        page ? (
          <div className="flex min-h-0 flex-1 flex-col phone:pt-[var(--tile-top)]">
            <Suspense fallback={<div className="min-h-0 flex-1" />}>
              <PageFrame key={drawn} name={page.name} version={page.changed} say={compose} />
            </Suspense>
          </div>
        ) : (
          <p className="chat-column pt-6 text-control text-mid">{library.loaded ? "This is gone from the Library." : ""}</p>
        )
      ) : "all" in shelved ? (
        <>
          {finding && (
            <div className="shrink-0 border-hair px-3 py-2 lg:border-b phone:pt-[var(--chrome-top)]">
              <input
                type="search"
                autoFocus
                value={find}
                onChange={(e) => setFind(e.target.value)}
                onKeyDown={(e) => e.key === "Escape" && (setFind(""), setFinding(false))}
                placeholder="Find in the Library"
                aria-label="Find in the Library"
                className="w-full rounded-lg bg-lift px-3 py-1.5 text-body text-ink outline-none placeholder:text-soft"
              />
            </div>
          )}
          <div className={`min-h-0 flex-1 overflow-y-auto px-2 phone:[--fade-top:var(--top-fade)] ${finding ? "" : "phone:pt-[var(--chrome-top)]"} ${FAB_CLEARANCE} ${BAR_FADE}`}>
            <LibraryShelf library={library} open={open} find={find} />
          </div>
          <Maker say={say} />
        </>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto phone:pt-[var(--chrome-top)]">
          {list ? (
            <ul className="chat-column flex flex-col gap-0.5 pt-4 pb-10">
              {list.items.map((item, i) => (
                <li key={i} className="flex items-start gap-3 rounded-lg px-3 py-2 text-body text-ink">
                  <span className="mt-[0.55em] size-1.5 shrink-0 rounded-full bg-frame" />
                  <span className="min-w-0 break-words">{item}</span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="chat-column pt-6 text-control text-mid">{library.loaded ? "This is gone from the Library." : ""}</p>
          )}
        </div>
      )}
    </div>
  );
}

/** A watch as the host lists it (library.rs Shown). */
export type Watch = {
  id: number;
  at?: number | null;
  every?: string | null;
  kind: "note" | "ask";
  text: string;
  worker?: string | null;
  active: boolean;
  looks?: string | null;
  failing?: string | null;
  /** The look's arguments, as JSON. A host from before these lists none. */
  arguments?: string | null;
  tests?: string[];
  fields?: string[];
  shared?: string[];
  pushed?: boolean;
};

/** The watches, asked again while the Tasks page is in view; a host from
 * before them answers nothing, which reads as none. */
export function useWatches(looking: boolean) {
  const [watches, setWatches] = useState<Watch[]>([]);
  const last = useRef("");
  const ask = useCallback(async () => {
    const r = await api("api/watches").catch(() => undefined);
    if (!r?.ok || !r.headers.get("content-type")?.includes("json")) return;
    const text = await r.text();
    if (text === last.current) return;
    last.current = text;
    setWatches((JSON.parse(text) as { watches: Watch[] }).watches);
  }, []);
  useEffect(() => {
    void ask();
    if (!looking) return;
    const id = window.setInterval(() => document.visibilityState === "visible" && void ask(), 30000);
    return () => window.clearInterval(id);
  }, [ask, looking]);
  const edit = useCallback(
    async (id: number, how: "delete" | "snooze", until?: number) => {
      // Shown at once, as the host will have it.
      last.current = "";
      setWatches((all) => (how === "delete" ? all.filter((w) => w.id !== id) : all.map((w) => (w.id === id ? { ...w, at: until } : w))));
      await api(`api/watches/${id}/${how}`, {
        method: "POST",
        ...(until !== undefined && { headers: { "Content-Type": "application/json" }, body: JSON.stringify({ until }) }),
      }).catch(() => undefined);
      await ask();
    },
    [ask],
  );
  return {
    watches,
    snooze: (id: number, until: number) => void edit(id, "snooze", until),
    remove: (id: number) => void edit(id, "delete"),
  };
}
export type Watches = ReturnType<typeof useWatches>;

/** What a watch is, in a word: a reminder is a note at a time. */
const kindOf = (w: Watch) => (w.looks ? "Watch" : w.at == null ? "On push" : w.kind === "note" ? "Reminder" : "Routine");

/** How often a watch repeats, as a person says it: Daily, Every 30m. */
const repeats = (every?: string | null) => (!every ? "Once" : /^\d/.test(every) ? `Every ${every}` : every[0].toUpperCase() + every.slice(1));

/** Everything a watch holds, line by line, for its card opened. */
function watchDetails(w: Watch): [string, string][] {
  const when = w.at != null ? new Date(w.at * 1000).toLocaleString(undefined, { weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" }) : undefined;
  return [
    [w.kind === "ask" ? "Ask" : "Note", w.text],
    ["Next", w.pushed ? "When a push arrives" : (when ?? "Not timed")],
    ["Repeats", repeats(w.every)],
    ...(w.looks ? [["Looks at", w.arguments ? `${w.looks} ${w.arguments}` : w.looks] as [string, string]] : []),
    ...(w.tests?.length ? [["Tests", w.tests.join("\n")] as [string, string]] : []),
    ...(w.fields?.length ? [["Sees", w.fields.join(", ")] as [string, string]] : []),
    ["Goes to", w.worker ? `${w.worker}'s thread` : "The main chat"],
    ...(w.shared?.length ? [["Shared with", w.shared.join(", ")] as [string, string]] : []),
    ["State", w.active ? "On" : "Off"],
    ...(w.failing ? [["Failing", w.failing] as [string, string]] : []),
  ];
}

/** One watch's card: a tap opens it in place to show all it holds, another
 * closes it; Snooze and Delete on a hold or a right click. */
export function WatchRow({ w, watches }: { w: Watch; watches: Watches }) {
  const [open, setOpen] = useState(false);
  const note = [
    w.at != null && wakeWords(w.at),
    w.every && `every ${w.every}`,
    w.worker && `in ${w.worker}'s thread`,
    !w.active && "off",
  ]
    .filter(Boolean)
    .join(" · ");
  const flip = () => setOpen((o) => !o);
  return (
    <li>
      <Held whole menu={(grip, close) => <WatchMenu grip={grip} close={close} w={w} watches={watches} />}>
        <article
          role="button"
          tabIndex={0}
          aria-expanded={open}
          data-watch={w.id}
          onClick={flip}
          onKeyDown={(e) => {
            if (e.key !== "Enter" && e.key !== " ") return;
            e.preventDefault();
            flip();
          }}
          className="cursor-pointer rounded-xl border border-hair bg-lift px-3 py-2 shadow-lift outline-none focus-visible:ring-2 focus-visible:ring-active"
        >
          <div className="flex items-center gap-1.5 text-caption leading-[15.8125px]">
            <span className={`size-2 shrink-0 rounded-full ${w.failing ? "bg-accent" : w.active ? "border-[1.5px] border-active" : "bg-frame"}`} />
            <span className="shrink-0 font-semibold text-ink">{kindOf(w)}</span>
            <span className="min-w-0 flex-1 truncate text-soft">
              {w.failing && <span className="rounded-sm bg-accent/10 px-1.5 font-medium text-accent">Failing</span>}
              {note}
            </span>
          </div>
          {open ? (
            <dl data-watch-details className="mt-1.5 grid grid-cols-[80px_minmax(0,1fr)] gap-x-3 gap-y-1 text-label leading-snug">
              {watchDetails(w).map(([label, value]) => (
                <Fragment key={label}>
                  <dt className="text-caption leading-snug text-soft">{label}</dt>
                  <dd className="min-w-0 break-words whitespace-pre-wrap text-ink">{value}</dd>
                </Fragment>
              ))}
            </dl>
          ) : (
            <p className="mt-0.5 truncate text-label leading-snug font-medium">{w.text}</p>
          )}
        </article>
      </Held>
    </li>
  );
}

function WatchMenu({ grip, close, w, watches }: { grip: Grip; close: () => void; w: Watch; watches: Watches }) {
  const [step, setStep] = useState<"menu" | "snooze">("menu");
  const icon = (d: ReactNode) => <Icon className="size-[17px]">{d}</Icon>;
  const clock = icon(<><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></>);
  const done = (run: () => void) => () => {
    run();
    close();
  };
  // A snooze moves the next time on from when it was due, or from now.
  const from = new Date(Math.max(Date.now(), (w.at ?? 0) * 1000));
  const items: MenuItem[] =
    step === "menu"
      ? [
          ...(w.at != null ? [{ label: "Snooze", icon: clock, run: () => setStep("snooze") }] : []),
          { label: "Delete", icon: icon(<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3" />), run: done(() => watches.remove(w.id)) },
        ]
      : napPresets(from).map((p) => ({ label: p.label, icon: clock, run: done(() => watches.snooze(w.id, p.until)) }));
  return <HoldMenu grip={grip} framed={false} label={step === "menu" ? kindOf(w) : "Snooze until"} items={items} close={close} />;
}

/** The app's own window to the page that holds it (the Android app's
 * WebView, Library.kt ShelvedPage), given to the host's origin alone. */
declare global {
  interface Window {
    LibriApp?: { postMessage: (text: string) => void };
  }
}

/** A Library page alone, filling the window: `#page-alone=<name>`, for an
 * app that shows the page in a view of its own (Android) and draws its own
 * head around it. A page's message goes to that app's composer; in a
 * browser tab, to this host's chat composer, which the tab then opens. */
export function PageAlone({ name }: { name: string }) {
  useTheme();
  const library = useLibrary(true);
  const page = library.pages.find((p) => p.name === name);
  const toComposer = (text: string) => {
    if (window.LibriApp) return window.LibriApp.postMessage(text);
    keepDraft(forHost("draft"), joined(draft(forHost("draft")), text));
    location.replace(location.pathname + location.search);
  };
  // Fixed to the window, not h-dvh: Android's WebView (Chrome 133) sizes dvh
  // while the view is still 0 high and keeps it at 0, drawing nothing.
  return (
    <div className="fixed inset-0 flex flex-col bg-paper">
      {page ? (
        <Suspense fallback={<div className="min-h-0 flex-1" />}>
          <PageFrame name={page.name} version={page.changed} say={toComposer} />
        </Suspense>
      ) : (
        <p className="chat-column pt-6 text-control text-mid">{library.loaded ? "This is gone from the Library." : ""}</p>
      )}
    </div>
  );
}

/** A built-in's view alone, filling the window: `#builtin-alone=<id>`, for
 * the Android app, which shows it in the chat's place under its own head. A
 * message from it goes to that app's composer. */
export function BuiltinAlone({ id }: { id: string }) {
  useTheme();
  const toComposer = (text: string) => window.LibriApp?.postMessage(text);
  return (
    <div className="fixed inset-0 flex flex-col bg-paper">
      <Suspense fallback={<div className="min-h-0 flex-1" />}>
        <BuiltinFrame id={id} say={toComposer} />
      </Suspense>
    </div>
  );
}
