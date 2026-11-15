// The Whiteboard tile: Excalidraw's canvas over a board the host keeps
// (src/boards.rs), with the boards as tabs along its head. Which board a
// tile shows is its pane's `arg`, so each workspace (and each tile) keeps
// its own tab. The canvas, its fonts and its code load the first time a
// board opens, not with the page.
//
// Sync: the host keeps every element with Excalidraw's own version and
// nonce. This view sends the elements it changed (debounced), and follows
// the board with a long poll, merging what comes back element by element
// as Excalidraw's collaboration does: the higher version wins, on a tie the
// lower nonce. The agent's sticky notes ("jot this down") arrive the same
// way, within a beat.

import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Strip, StripAdd, StripName, StripTab } from "./tabStrip";
import { api } from "./api";
import { saveFile } from "./save";
import { useTileRows } from "./tileMenu";
import { confirmAction } from "./dialog";

type Element = { id: string; version: number; versionNonce: number; isDeleted?: boolean; [k: string]: unknown };
type Files = Record<string, { id: string; [k: string]: unknown }>;
type Scene = { version: number; elements: Element[]; files: Files };
/** One change, as the shared-object layer carries it (src/boards.rs Op). */
type Op = { op: "upsert"; element: Element } | { op: "delete"; id: string; version: number; versionNonce: number } | { op: "file"; file: Files[string] };
/** What a board's log has past a version: its ops, or the whole board. */
type Since = { version: number; ops: Op[] } | { version: number; snapshot: Scene };
export type BoardInfo = { id: string; name: string; version: number; changed: number };

type Lib = typeof import("@excalidraw/excalidraw");
type Api = import("@excalidraw/excalidraw/types").ExcalidrawImperativeAPI;

// Excalidraw fetches its fonts from here; the build copies them beside the
// page's own files (vite.config.ts), so a board never calls a CDN.
let loaded: Promise<Lib> | undefined;
const excalidraw = () => {
  (window as unknown as { EXCALIDRAW_ASSET_PATH: string }).EXCALIDRAW_ASSET_PATH = new URL("_site/excalidraw/", document.baseURI).href;
  return (loaded ??= Promise.all([import("@excalidraw/excalidraw"), import("@excalidraw/excalidraw/index.css")]).then(([m]) => {
    boardMark("editor-code");
    return m;
  }));
};

/** How long a board took to open, by step, as `performance` marks and one
 * console line ("libri-board-timing {...}", milliseconds since the page
 * began, plus its start as epoch milliseconds): what the Android app's
 * driver and the browser checks read to time the tile. */
const timed = new Set<string>();
export function boardMark(step: string) {
  if (timed.has(step)) return;
  timed.add(step);
  performance.mark(`libri-board-${step}`);
  if (step !== "ready") return;
  const at: Record<string, number> = { origin: Math.round(performance.timeOrigin) };
  for (const m of performance.getEntriesByType("mark")) if (m.name.startsWith("libri-board-")) at[m.name.slice(12)] = Math.round(m.startTime);
  console.debug(`libri-board-timing ${JSON.stringify(at)}`);
}

/** Whether `a` should replace `b`, as Excalidraw reconciles two copies. */
const wins = (a: Element, b: Element) => a.version > b.version || (a.version === b.version && a.versionNonce < b.versionNonce);

const SEND_AFTER = 300;

async function json<T>(r: Response): Promise<T> {
  if (!r.ok) throw new Error((await r.text()) || r.statusText);
  return r.json() as Promise<T>;
}

export async function listBoards(): Promise<BoardInfo[]> {
  boardMark("tile");
  const got = await json<BoardInfo[]>(await api("api/boards"));
  boardMark("boards");
  return got;
}
async function createBoard(name: string): Promise<BoardInfo> {
  return json(await api("api/boards", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ name }) }));
}

/** The boards the tabs show, refreshed when one is made or renamed here
 * and, as the agent can make the board Notes, every so often. */
function useBoards() {
  const [boards, setBoards] = useState<BoardInfo[]>();
  const reload = useCallback(() => listBoards().then(setBoards, () => setBoards((b) => b ?? [])), []);
  useEffect(() => {
    void reload();
    const t = window.setInterval(reload, 15000);
    return () => window.clearInterval(t);
  }, [reload]);
  return { boards, reload, setBoards };
}

// A board just made opens in its tile under a new key (a tile is drawn by
// what it shows), so the offer to name it is kept here across the remount.
let nameNext: string | undefined;

/** The first free name of the form `Board N`. */
function nextBoardName(boards: BoardInfo[]) {
  const used = new Set(boards.map((b) => b.name));
  let n = boards.length + 1;
  while (used.has(`Board ${n}`)) n++;
  return `Board ${n}`;
}

/** What a Whiteboard tile opened beside one shows: a board of its own,
 * made when it opens (tiles.tsx another). */
export const NEW_BOARD = "new";

export function BoardTile({ id, pick, phone, active = true }: { id?: string; pick: (id: string) => void; phone: boolean; active?: boolean }) {
  const { boards, reload, setBoards } = useBoards();
  // A tile with no board yet shows the first, or makes Notes on an empty
  // host. Once for each board it lacks: the pick reaches the layout, which
  // draws this tile again with a new `pick` before the board arrives as
  // `id`, and asking again each time loops until React gives up (seen on a
  // phone's Whiteboard item added with no board).
  // Only the tile on screen and focused keeps a pick: a phone's hidden
  // items share the shown item's `replace`, and a pick from one of them
  // would change what the shown item is.
  const asked = useRef<string | undefined>(undefined);
  const making = useRef(false);
  // A board of its own (New board, or a tile opened beside one), once.
  const make = useCallback(async () => {
    if (!boards || making.current) return;
    making.current = true;
    const b = await createBoard(nextBoardName(boards)).finally(() => (making.current = false));
    setBoards([...boards, b]);
    asked.current = b.id;
    pick(b.id);
  }, [boards, pick, setBoards]);
  useEffect(() => {
    if (!active) asked.current = undefined;
  }, [active]);
  useEffect(() => {
    if (!active || !boards) return;
    if (id === NEW_BOARD) {
      if (asked.current === NEW_BOARD) return;
      asked.current = NEW_BOARD;
      return void make();
    }
    if (id && boards.some((b) => b.id === id)) return;
    if (boards.length) {
      if (asked.current === boards[0].id) return;
      asked.current = boards[0].id;
      return pick(boards[0].id);
    }
    if (making.current) return;
    making.current = true;
    void createBoard("Notes").then((b) => {
      setBoards([b]);
      asked.current = b.id;
      pick(b.id);
    });
  }, [active, boards, id, pick, setBoards, make]);
  // What it shows: its own board, or until that is kept, the first.
  const on = boards?.find((b) => b.id === id) ?? (id === NEW_BOARD ? undefined : boards?.[0]);
  useTileRows("board", {
    "new-board": { run: () => void make() },
    "export": {
      disabled: !on,
      run: () => {},
      then: on
        ? [
            { label: "As a PNG picture", run: () => void exportBoard(on.id, on.name, "png") },
            { label: "As an SVG picture", run: () => void exportBoard(on.id, on.name, "svg") },
            { label: "As an Excalidraw file", run: () => void exportBoard(on.id, on.name, "excalidraw") },
          ]
        : undefined,
    },
  });
  return (
    // Every drag on a board is the canvas's or its tabs' own (draw, pan,
    // pinch, reorder), so the phone's page never swipes to the next item
    // from here (work.tsx keepsSwipe); the bar still does.
    // Its pinch and ctrl+wheel zoom the canvas, not the talk's text
    // (pinch.ts, data-own-zoom).
    <div className="flex h-full min-h-0 flex-col" data-surface="board" data-own-swipe data-own-zoom>
      <Tabs boards={boards ?? []} on={on?.id} pick={pick} reload={reload} setBoards={setBoards} phone={phone} />
      {/* isolate: Excalidraw's own layers stack inside the canvas, under
          the tabs' menus and the tile's notch. */}
      <div className="relative isolate min-h-0 flex-1">{on ? <Canvas key={on.id} board={on} /> : <p className="p-6 text-label text-mid">Opening the board…</p>}</div>
    </div>
  );
}

function Tabs({
  boards,
  on,
  pick,
  reload,
  setBoards,
  phone,
}: {
  boards: BoardInfo[];
  on?: string;
  pick: (id: string) => void;
  reload: () => Promise<void>;
  setBoards: (b: BoardInfo[]) => void;
  phone: boolean;
}) {
  const [naming, setNaming] = useState<string | undefined>(() => {
    const n = nameNext;
    nameNext = undefined;
    return n;
  });
  const [dragged, setDragged] = useState<string>();
  const add = async () => {
    const b = await createBoard(nextBoardName(boards));
    setBoards([...boards, b]);
    nameNext = b.id;
    setNaming(b.id);
    pick(b.id);
  };
  const rename = async (b: BoardInfo, name: string) => {
    setNaming(undefined);
    if (!name.trim() || name.trim() === b.name) return;
    await api(`api/boards/${b.id}/name`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ name }) });
    await reload();
  };
  const remove = async (b: BoardInfo) => {
    const yes = await confirmAction({ title: `Delete the board ${b.name}?`, body: "It goes from every workspace, and can't be brought back.", confirm: "Delete board", danger: true });
    if (!yes) return;
    await api(`api/boards/${b.id}/remove`, { method: "POST" });
    const left = boards.filter((x) => x.id !== b.id);
    setBoards(left);
    if (b.id === on && left.length) pick(left[0].id);
    else void reload();
  };
  const drop = async (target: string) => {
    if (!dragged || dragged === target) return;
    const ids = boards.map((b) => b.id).filter((i) => i !== dragged);
    ids.splice(ids.indexOf(target), 0, dragged);
    setBoards(ids.map((i) => boards.find((b) => b.id === i)!));
    setDragged(undefined);
    await api("api/boards/order", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ ids }) });
  };
  return (
    // The tabs scroll sideways; the Share and Export menus sit beside them,
    // out of the scroller (which would clip what drops below it) and above
    // the canvas.
    // Under the tile's notch (z-30, tiles.tsx Notch), which must stay
    // reachable over a board as over any tile.
    <div className={`relative z-20 flex h-11 shrink-0 items-center gap-1 px-2 ${phone ? "pt-[var(--tile-top,0px)] h-auto pb-1" : ""}`}>
    <div className="flex min-w-0 flex-1">
    <Strip scroll role="tablist" aria-label="Boards">
      {boards.map((b) =>
        naming === b.id ? (
          <StripName key={b.id} name={b.name} label="Board name" done={(name) => void rename(b, name)} cancel={() => setNaming(undefined)} />
        ) : (
          <StripTab
            key={b.id}
            on={b.id === on}
            draggable
            onDragStart={() => setDragged(b.id)}
            onDragOver={(e) => e.preventDefault()}
            onDrop={() => void drop(b.id)}
            onClick={() => pick(b.id)}
            onDoubleClick={() => setNaming(b.id)}
            title="Double-click to rename"
            close={{ label: `Delete ${b.name}`, run: () => void remove(b) }}
          >
            {b.name}
          </StripTab>
        ),
      )}
      <StripAdd label="New board" onClick={() => void add()} />
    </Strip>
    </div>
      {on && <ShareMenu id={on} />}
      {on && <Exports id={on} name={boards.find((b) => b.id === on)?.name ?? "board"} />}
    </div>
  );
}

type LinkInfo = { token: string; role: "view" | "edit"; made: number; expires: number | null; url: string | null };

/** Links that open the board to anyone holding one (src/share.rs): view or
 * edit, lapsing after a while or never, taken back here. */
function ShareMenu({ id }: { id: string }) {
  const [open, setOpen] = useState(false);
  const [links, setLinks] = useState<LinkInfo[]>([]);
  const [can, setCan] = useState(true);
  const [error, setError] = useState<string>();
  const [copied, setCopied] = useState<string>();
  const load = useCallback(async () => {
    try {
      const got = await json<{ links: LinkInfo[]; public: boolean }>(await api(`api/boards/${id}/links`));
      setLinks(got.links);
      setCan(got.public);
    } catch (e) {
      setError((e as Error).message);
    }
  }, [id]);
  useEffect(() => {
    if (open) void load();
  }, [open, load]);
  const copy = (l: LinkInfo) => {
    if (!l.url) return;
    void navigator.clipboard?.writeText(l.url);
    setCopied(l.token);
  };
  const make = async (role: "view" | "edit", days?: number) => {
    setError(undefined);
    try {
      const l = await json<LinkInfo>(await api(`api/boards/${id}/links`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ role, days }) }));
      setLinks((ls) => [l, ...ls]);
      copy(l);
    } catch (e) {
      setError((e as Error).message);
    }
  };
  const revoke = async (l: LinkInfo) => {
    await api(`api/boards/links/${encodeURIComponent(l.token)}/revoke`, { method: "POST" });
    setLinks((ls) => ls.filter((x) => x.token !== l.token));
  };
  const [days, setDays] = useState<number | undefined>(7);
  const until = (l: LinkInfo) => (l.expires ? `until ${new Date(l.expires * 1000).toLocaleDateString()}` : "no end");
  return (
    <span className="relative ml-auto shrink-0">
      <button type="button" aria-label="Share the board" aria-expanded={open} onClick={() => setOpen((o) => !o)} className="grid size-8 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink">
        <svg viewBox="0 0 24 24" aria-hidden className="size-[18px] fill-none stroke-current stroke-[1.6] [stroke-linecap:round] [stroke-linejoin:round]">
          <path d="M10 14a4 4 0 0 0 5.66 0l3-3a4 4 0 0 0-5.66-5.66l-1 1M14 10a4 4 0 0 0-5.66 0l-3 3a4 4 0 0 0 5.66 5.66l1-1" />
        </svg>
      </button>
      {open && (
        <span role="dialog" aria-label="Share the board" className="absolute top-9 right-0 z-50 flex w-72 flex-col gap-1 rounded-2xl border border-hair bg-lift p-2 shadow-lift">
          {!can ? (
            <span className="px-2 py-1 text-label text-mid">Links need a public address on this host (share.url).</span>
          ) : (
            <>
              <span className="flex items-center gap-1 px-1">
                <span className="text-small text-mid">Lapses</span>
                {([[1, "1 day"], [7, "7 days"], [30, "30 days"], [undefined, "never"]] as const).map(([d, label]) => (
                  <button key={label} type="button" aria-pressed={days === d} onClick={() => setDays(d)} className={`rounded-full px-2 py-0.5 text-small ${days === d ? "bg-hover text-ink" : "text-mid hover:text-ink"}`}>
                    {label}
                  </button>
                ))}
              </span>
              <MenuItem onClick={() => void make("view", days)}>New view link</MenuItem>
              <MenuItem onClick={() => void make("edit", days)}>New edit link</MenuItem>
            </>
          )}
          {error && <span className="px-2 text-small text-accent">{error}</span>}
          {links.map((l) => (
            <span key={l.token} data-link={l.role} className="flex items-center gap-1 rounded-xl px-2 py-1 text-small text-ink">
              <span className="min-w-0 flex-1 truncate">
                {l.role === "edit" ? "Can edit" : "Can view"} · {until(l)}
              </span>
              <button type="button" onClick={() => copy(l)} className="rounded-full px-2 text-mid hover:text-ink">
                {copied === l.token ? "Copied" : "Copy"}
              </button>
              <button type="button" onClick={() => void revoke(l)} className="rounded-full px-2 text-mid hover:text-accent">
                Take back
              </button>
            </span>
          ))}
        </span>
      )}
    </span>
  );
}

/** `#board=<token>`: the board a link opens, alone, for whoever holds it
 * (served by src/share.rs, which reads the token from a header, never the
 * address the browser sends). */
export function SharedBoard({ token }: { token: string }) {
  const [opened, setOpened] = useState<{ name: string; role: "view" | "edit"; scene: Scene }>();
  const [error, setError] = useState<string>();
  const headers = useMemo(() => ({ "x-share-token": token }), [token]);
  useEffect(() => {
    fetch("api/share/board", { headers })
      .then((r) => json<{ name: string; role: "view" | "edit"; scene: Scene }>(r))
      .then(setOpened, (e: Error) => setError(e.message));
  }, [headers]);
  const source = useMemo<Source | undefined>(
    () =>
      opened && {
        key: `share-${token.slice(0, 8)}`,
        load: async () => opened.scene,
        since: (v) => fetch(`api/share/board?since=${v}`, { headers }),
        send: (ops) => fetch("api/share/board", { method: "POST", headers: { ...headers, "content-type": "application/json" }, body: JSON.stringify({ ops }) }),
        readOnly: opened.role === "view",
      },
    [opened, token, headers],
  );
  useEffect(() => {
    if (opened) document.title = opened.name;
  }, [opened]);
  if (error) return <p className="p-6 text-label text-mid">This link doesn't open a board any more.</p>;
  if (!opened || !source) return <p className="p-6 text-label text-mid">Opening the board…</p>;
  return (
    <div className="fixed inset-0 flex flex-col bg-paper">
      <div className="flex h-11 shrink-0 items-center gap-2 px-4 text-label text-ink">
        <span className="truncate">{opened.name}</span>
        <span className="text-small text-mid">{opened.role === "edit" ? "You can edit" : "View only"}</span>
      </div>
      <div className="relative min-h-0 flex-1">
        <BoardCanvas source={source} />
      </div>
    </div>
  );
}

/** Saving a board out: a picture to paste anywhere, or the board itself
 * for excalidraw.com. */
/** Saves board `id` as a picture or as its own file, named `name`. */
async function exportBoard(id: string, name: string, kind: "png" | "svg" | "excalidraw") {
  const save = (blob: Blob, ext: string) => void saveFile(blob, `${name}.${ext}`);
  if (kind === "excalidraw") return save(await (await api(`api/boards/${id}/export`)).blob(), "excalidraw");
  const [lib, scene] = await Promise.all([excalidraw(), json<Scene>(await api(`api/boards/${id}`))]);
  const elements = scene.elements.filter((e) => !e.isDeleted) as never;
  const appState = { exportBackground: true, viewBackgroundColor: "#ffffff" };
  if (kind === "png") save(await lib.exportToBlob({ elements, files: scene.files as never, appState, mimeType: "image/png" }), "png");
  else save(new Blob([(await lib.exportToSvg({ elements, files: scene.files as never, appState })).outerHTML], { type: "image/svg+xml" }), "svg");
}

function Exports({ id, name }: { id: string; name: string }) {
  const [open, setOpen] = useState(false);
  const picture = async (kind: "png" | "svg") => {
    setOpen(false);
    await exportBoard(id, name, kind);
  };
  const file = async () => {
    setOpen(false);
    await exportBoard(id, name, "excalidraw");
  };
  return (
    <span className="relative shrink-0">
      <button type="button" aria-label="Export the board" aria-expanded={open} onClick={() => setOpen((o) => !o)} className="grid size-8 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink">
        <svg viewBox="0 0 24 24" aria-hidden className="size-[18px] fill-none stroke-current stroke-[1.6] [stroke-linecap:round] [stroke-linejoin:round]">
          <path d="M12 4v11M7 10l5 5 5-5M5 19h14" />
        </svg>
      </button>
      {open && (
        <span role="menu" className="absolute top-9 right-0 z-50 flex w-48 flex-col rounded-2xl border border-hair bg-lift p-1 shadow-lift">
          <MenuItem onClick={() => void picture("png")}>Picture (PNG)</MenuItem>
          <MenuItem onClick={() => void picture("svg")}>Picture (SVG)</MenuItem>
          <MenuItem onClick={() => void file()}>Board file (.excalidraw)</MenuItem>
        </span>
      )}
    </span>
  );
}
function MenuItem({ onClick, children }: { onClick: () => void; children: ReactNode }) {
  return (
    <button type="button" role="menuitem" onClick={onClick} className="rounded-xl px-3 py-2 text-left text-label text-ink hover:bg-hover">
      {children}
    </button>
  );
}

/** Where a canvas reads and writes its board: the host's own API for the
 * person, or a board link's (src/share.rs) for whoever holds the link. */
type Source = {
  key: string;
  load: () => Promise<Scene>;
  /** Waits for what changed past `v`: a 204 when nothing did. */
  since: (v: number) => Promise<Response>;
  send: (ops: Op[]) => Promise<Response>;
  /** Marks the board as where "jot this down" lands; the person's only. */
  focus?: () => void;
  readOnly?: boolean;
};

const ownSource = (id: string): Source => ({
  key: id,
  load: async () => json<Scene>(await api(`api/boards/${id}`)),
  since: (v) => api(`api/boards/${id}?since=${v}`),
  send: (ops) => api(`api/boards/${id}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ ops }) }),
  focus: () => void api(`api/boards/${id}/focus`, { method: "POST" }),
});

/** The canvas over one board, kept in step with the host. */
function Canvas({ board }: { board: BoardInfo }) {
  const source = useMemo(() => ownSource(board.id), [board.id]);
  return <BoardCanvas source={source} />;
}

/** The canvas over whatever board `source` reads, kept in step with it. */
function BoardCanvas({ source }: { source: Source }) {
  const board = { id: source.key };
  const [lib, setLib] = useState<Lib>();
  const [first, setFirst] = useState<Scene>();
  const [error, setError] = useState<string>();
  const ex = useRef<Api | null>(null);
  // What the host holds, by element: what this view last sent or heard.
  const known = useRef(new Map<string, Element>());
  const sentFiles = useRef(new Set<string>());
  const version = useRef(0);
  const sending = useRef<number | undefined>(undefined);
  const theme = useThemeName();
  // The canvas's box, which says how many elements are drawn, for checks
  // and tools to read without reaching into the canvas.
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let gone = false;
    Promise.all([excalidraw(), source.load()]).then(
      ([l, s]) => {
        if (gone) return;
        for (const e of s.elements) known.current.set(e.id, e);
        for (const f of Object.keys(s.files)) sentFiles.current.add(f);
        version.current = s.version;
        boardMark("scene");
        setLib(l);
        setFirst(s);
      },
      (e: Error) => !gone && setError(e.message),
    );
    return () => {
      gone = true;
    };
  }, [board.id]);

  // The board on screen is where "jot this down" lands. Every workspace's
  // tiles stay mounted, the others' hidden, so a tile claims its board only
  // when it shows (and again when the person touches it).
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const o = new IntersectionObserver((seen) => {
      if (seen.some((e) => e.isIntersecting)) source.focus?.();
    });
    o.observe(el);
    return () => o.disconnect();
  }, [board.id, lib, first]);

  // Follows the board: whatever another view or the agent changed.
  useEffect(() => {
    if (!lib || !first) return;
    let gone = false;
    const follow = async () => {
      while (!gone) {
        try {
          const r = await source.since(version.current);
          if (gone) return;
          if (r.status === 204) continue;
          const got = await json<Since>(r);
          version.current = got.version;
          if ("snapshot" in got) take(lib, got.snapshot.elements, got.snapshot.files);
          else {
            const els: Element[] = [];
            const files: Files = {};
            for (const op of got.ops) {
              if (op.op === "upsert") els.push(op.element);
              else if (op.op === "file") files[op.file.id] = op.file;
              else {
                const held = known.current.get(op.id) ?? ex.current?.getSceneElementsIncludingDeleted().find((e) => e.id === op.id);
                if (held) els.push({ ...(held as unknown as Element), isDeleted: true, version: op.version, versionNonce: op.versionNonce });
              }
            }
            take(lib, els, files);
          }
        } catch {
          await new Promise((w) => setTimeout(w, 3000));
        }
      }
    };
    void follow();
    return () => {
      gone = true;
    };
    // take reads refs only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lib, first, board.id]);

  /** Merges what the host has into the canvas, keeping local strokes that
   * are newer than the host's copy. */
  const take = (l: Lib, elements: Element[], files: Files) => {
    const api_ = ex.current;
    if (!api_) return;
    const local = new Map(api_.getSceneElementsIncludingDeleted().map((e) => [e.id, e as unknown as Element]));
    let changed = false;
    const incoming = l.restoreElements(elements as never, null, { refreshDimensions: false, repairBindings: true }) as unknown as Element[];
    for (const e of incoming) {
      const mine = local.get(e.id);
      known.current.set(e.id, e);
      if (!mine || wins(e, mine)) {
        local.set(e.id, e);
        changed = true;
      }
    }
    const fresh = Object.values(files).filter((f) => !sentFiles.current.has(f.id));
    if (fresh.length) {
      for (const f of fresh) sentFiles.current.add(f.id);
      api_.addFiles(fresh as never);
    }
    if (changed) api_.updateScene({ elements: [...local.values()] as never, captureUpdate: l.CaptureUpdateAction.NEVER });
  };

  /** Sends what this view changed: elements newer than the host's copy,
   * and images it hasn't had. */
  const send = useCallback(() => {
    const api_ = ex.current;
    if (!api_) return;
    const els = api_.getSceneElementsIncludingDeleted() as unknown as Element[];
    const changed = els.filter((e) => {
      const k = known.current.get(e.id);
      return !k || wins(e, k);
    });
    const all = api_.getFiles() as unknown as Files;
    const files: Files = {};
    for (const [fid, f] of Object.entries(all)) if (!sentFiles.current.has(fid)) files[fid] = f;
    if (!changed.length && !Object.keys(files).length) return;
    for (const e of changed) known.current.set(e.id, e);
    for (const fid of Object.keys(files)) sentFiles.current.add(fid);
    const ops: Op[] = [...changed.map((element) => ({ op: "upsert" as const, element })), ...Object.values(files).map((file) => ({ op: "file" as const, file }))];
    void source
      .send(ops)
      .then((r) => json<{ version: number }>(r))
      .then((v) => {
        // Only our own ops took the board here: no need to hear them back.
        if (v.version === version.current + ops.length) version.current = v.version;
      })
      .catch(() => {
        // Not kept: forget we sent it, so the next change sends it again.
        for (const e of changed) known.current.delete(e.id);
        for (const fid of Object.keys(files)) sentFiles.current.delete(fid);
      });
  }, [source]);

  // The canvas tidies a board as it opens (text measured in this browser's
  // fonts, bindings repaired), which bumps element versions. Those are this
  // view's own, not the person's changes: while it settles they become the
  // baseline rather than going to the host, or every opening would write.
  const settled = useRef(false);
  // A change the person makes at once is theirs, settled or not.
  const touched = useRef(false);
  useEffect(() => {
    if (!first) return;
    const t = window.setTimeout(() => (settled.current = true), 1500);
    return () => window.clearTimeout(t);
  }, [first]);
  const onChange = useCallback((els: readonly { isDeleted?: boolean }[]) => {
    box.current?.setAttribute("data-elements", String(els.filter((e) => !e.isDeleted).length));
    if (!settled.current && !touched.current) {
      for (const e of els as unknown as Element[]) known.current.set(e.id, e);
      return;
    }
    window.clearTimeout(sending.current);
    sending.current = window.setTimeout(send, SEND_AFTER);
  }, [send]);
  // A change still waiting goes before the tile does.
  useEffect(() => () => {
    window.clearTimeout(sending.current);
    send();
  }, [send]);

  const initialData = useMemo(
    () => first && lib && { elements: lib.restoreElements(first.elements as never, null), files: first.files as never, appState: { viewBackgroundColor: "transparent" }, scrollToContent: true },
    [first, lib],
  );
  if (error) return <p className="p-6 text-label text-accent">Couldn't open the board: {error}</p>;
  if (!lib || !initialData) return <p className="p-6 text-label text-mid">Opening the board…</p>;
  const { Excalidraw } = lib;
  return (
    <div ref={box} data-board={board.id} className="absolute inset-0"
      onPointerDown={() => {
        touched.current = true;
        source.focus?.();
      }}
      onKeyDown={() => (touched.current = true)}
    >
      <Excalidraw
        excalidrawAPI={(a) => {
          ex.current = a;
          // Drawable from the next frame the canvas paints.
          requestAnimationFrame(() => boardMark("ready"));
        }}
        initialData={initialData}
        onChange={onChange}
        theme={theme}
        viewModeEnabled={source.readOnly}
        UIOptions={{ canvasActions: { loadScene: false, saveToActiveFile: false, export: false, saveAsImage: false, toggleTheme: false } }}
      />
    </div>
  );
}

/** The page's theme, followed as it changes. */
function useThemeName(): "light" | "dark" {
  const read = (): "light" | "dark" => (document.documentElement.dataset.theme === "dark" ? "dark" : "light");
  const [t, setT] = useState(read);
  useEffect(() => {
    const o = new MutationObserver(() => setT(read()));
    o.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    return () => o.disconnect();
  }, []);
  return t;
}

