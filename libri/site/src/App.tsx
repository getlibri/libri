import { createContext, lazy, memo, Suspense, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type KeyboardEvent as KeyEvent, type ReactNode } from "react";
import { useChatDetail, setChatDetail, type ChatDetail } from "./chatDetail";
import { useSetting, modelSetting } from "./settings";
import { WhatsNew } from "./news";
import { BOOK_DURATION_MS, BOOK_PAGES, BOOK_STROKE, BOOK_VIEWBOX } from "./bookLoader";
import {
  ActionBarPrimitive,
  AssistantRuntimeProvider,
  AttachmentPrimitive,
  ComposerPrimitive,
  MessagePrimitive,
  ThreadPrimitive,
  useAui,
  useAuiState,
  useExternalStoreRuntime,
  type AppendMessage,
  type Attachment,
  type ThreadMessageLike,
  type ToolCallMessagePartProps,
} from "@assistant-ui/react";
import { useFootHold } from "./footHold";
import { useEarlier } from "./earlier";
import { Held, HoldMenu, type Grip, type MenuItem } from "./hold";
import { MEMORY_WORD, MemoryLine, sized, stamp, useMemory, type MemoryHealth } from "./memory";
import { StreamdownTextPrimitive } from "@assistant-ui/react-streamdown";
import { pageOf } from "./pageLink";
import { usePinchText, useTextSize } from "./pinch";
import { api, hostless, remote, standalone, url as apiUrl, use } from "./api";
import { PinIcon, PinnedList, resumed, usePins, usePinsOn } from "./pins";
import { compose, draft, forHost, keepDraft, keepFiles, keptFiles, onCompose } from "./keep";
import { askOffer, attachments, offerShown, COMMANDS, harnessCommands, offered, tileCommands, type HarnessCommands, picksChanged, SEND_MAX, firstLine, fromAgentChat, POINTER, quiet, useChat, type Answers, type Ask, type Card, type Chat, type Msg, type Metrics, type ReplyOffer, type Question, type Quote, type Task } from "./chat";
import { useTheme } from "./theme";
import { usePush } from "./push";
import { closePicking, Devices, Hosts, openPicking, Peers, useKnownHosts, usePicking, type HostInfo } from "./hosts";
import { Builtins, Connections, OtherApps, Xai, useBuiltins, useConnections } from "./connections";
import { BESIDE, Panes } from "./split";
import { Bars, Call, type CallMode, callMode, setCallMode, SPEED, useVoiceChoice, useVoiceSpeed, voiceChoice, voiceSpeed } from "./call";
import { Dictate, joined } from "./dictate";
import { DictationSettings } from "./dictationSettings";
import { Search, openSearch } from "./search";
import { md, usePlugins } from "./words";
import { closeScratchpad, newScratchpad, promoteScratchpad, readScratchpad, replyScratchpad, setScratchpadMemory, setScratchpadOpen, stopScratchpad, tellScratchpad, useScratchpad, wordsOf } from "./scratchpad";
import { WorkersTile } from "./workTree";
import { Logo, logoOf } from "./marks";
import { offerOf, publishOffers } from "./modelNames";
import { useTileRows } from "./tileMenu";
import { askTiles, offered as offeredKinds, useTiles } from "./tileViews";
import { ChatTabs, ChatTabsCtx, tabsOf } from "./memoryTabs";
import { ComposerAgents, RunningCtx, showAllAgents, type AgentScope, type Running } from "./agents";
import { addMemory, changeMemory, deleteMemory, replyMemoryChat, stopMemoryChat, tellMemoryChat, useMemories, useMemoryChat, type Memory } from "./memoryChats";
import { BoardTile } from "./board";
import { BrowserTile } from "./browser";
import { SystemTile } from "./system";
import { Toggle, toggleButton } from "./toggle";
import { Terminal } from "./terminal";
import { SkillsPage, useSkillMenu, useSkills } from "./skills";
import { GEAR, KEPT_CLOSED_DAYS, KIND_ICONS, KIND_NAMES, Tiles, UNDO_CLOSE_MS, type Launchable, type Pane, type TileApi } from "./tiles";
import { Phone, type Addable } from "./phone";
import { cardCall, useCardTool, type CardTool } from "./apps";
import type { Outcome } from "./card";
import { hold, reveal } from "./reveal";
import { LibraryItem, useLibrary, useWatches, type Shelved } from "./library";
import { Today } from "./home";
import { ago, asks, Glyph as WorkGlyph, parentOf, TempMark, Promote, promotion, ShellCtx, stateOf, useShell, useWork, type Shell, type Thread as WorkThread, type Work } from "./work";

type Args = ToolCallMessagePartProps["args"];

// `redraw` resends the question that led to an answer; `typing`: a turn
// is running, so the last exchange can't be redone yet.
// `retry` and `discard` act on a message the site refused.
// `quoting` is the earlier message the composer's next send replies to,
// shown as a strip over the words; `quote` sets or clears it.
type Redo = { redraw: (msgId: string) => void; typing: boolean; writable: boolean; retry: Ready["retry"]; discard: Ready["discard"]; quoting: Quote | null; quote: (q: Quote | null) => void };
// Draws a chat's whole talk, then runs what waits on it (useEarlier).
const WholeCtx = createContext<(then: () => void) => void>((then) => then());
const RedoCtx = createContext<Redo>({ redraw() {}, typing: false, writable: false, retry() {}, discard: () => undefined, quoting: null, quote() {} });
const useRedo = () => useContext(RedoCtx);

// Subagents, as Victor shows them: each at work is a dot above the
// composer, and its Agent call in the talk is a frame with its report. Both
// open its own page. `open` takes the call's id, and hippo's id for the
// agent, which finds its chat once the page no longer holds it.
type Agents = { tasks: Task[]; open: (call: string, agent?: string, title?: string) => void };
const AgentCtx = createContext<Agents>({ tasks: [], open() {} });
const isAgent = (name: string) => name === "Agent" || name === "Task";

const convert = (m: Msg): ThreadMessageLike => ({
  id: m.id,
  role: m.role,
  content: [
    // Each call in the order they came; words before them are the message
    // above (F13).
    ...(m.tools ?? []).flatMap((t) => [
      ...(t.thought ? [{ type: "reasoning" as const, text: t.thought }] : []),
      {
        type: "tool-call" as const,
        toolCallId: t.id,
        toolName: t.name,
        args: (t.input ?? {}) as Args,
        result: t.result,
        isError: t.error,
        artifact: t.agent || t.report || t.heard || t.took !== undefined || t.partial ? { agent: t.agent, report: t.report, structured: t.structured, heard: t.heard, took: t.took, partial: t.partial } : undefined,
      },
    ]),
    ...(m.thinking?.trim() ? [{ type: "reasoning" as const, text: m.thinking }] : []),
    ...(m.text && !quiet(m.text, m.live) ? [{ type: "text" as const, text: m.text }] : []),
    ...(m.picture ? [{ type: "image" as const, image: m.picture }] : []),
    ...(m.file ? [{ type: "file" as const, data: apiUrl(`api/file/${encodeURIComponent(m.file)}`, true), mimeType: "", filename: m.file }] : []),
  ],
  attachments: m.files?.map((f, n) => ({
    id: `${m.id}-file-${n}`,
    type: f.image ? "image" : "document",
    name: f.name,
    status: { type: "complete" },
    content: f.image ? [{ type: "image", image: f.image }] : [],
  })),
  createdAt: m.date,
  metadata: { custom: { dated: !!m.date, progress: m.progress, metrics: m.metrics, plain: m.plain ?? false, reaction: m.reaction ?? null, queued: m.queued ?? false, failed: m.failed, outside: m.outside ?? false, woke: m.woke, live: m.live ?? false, musing: m.live && !m.text ? m.thinking : undefined, quote: m.quote, row: m.row, from: m.from } },
});

// Several assistants may share one host, so the server names the site
// in the title and who answers in a meta tag.
const APP = document.title || "Chat";
/** The assistant's own name (the host's LIBRI_NAME), if it has one. */
const ASSISTANT = document.querySelector<HTMLMetaElement>('meta[name="assistant"]')?.content ?? "";
const NAME = ASSISTANT || APP;
/** A composer's invitation: to the one it talks to by name, else plainly. */
const messageTo = (name: string | undefined) => (name ? `Message ${name}` : "Send a message");

export function App() {
  const picking = usePicking();
  if (hostless()) return <Welcome />;
  // The host page opens over the host in use, which runs on beneath it
  // untouched until another host answers.
  return (
    <>
      <Talk />
      <WhatsNew />
      {picking && (
        <div className="fixed inset-0 z-[60] overflow-y-auto bg-paper">
          <Welcome back />
        </div>
      )}
    </>
  );
}

function Talk() {
  const chat = useChat();
  if (chat.state === "loading") return <Solo><Dots /><Home /></Solo>;
  if (chat.state === "signed-out") return <SignIn />;
  if (chat.state === "error") return <Solo><p className="text-mid">{chat.error}</p><Home /></Solo>;
  return <Places chat={chat} />;
}

type Ready = Extract<Chat, { state: "ready" }>;

// The places are tiles under a bar (tiles.tsx): the chat, Temp, the
// terminal, the workers, a worker's thread, the Library, Calendar, each a
// tile of its own, on a phone one at a time. The chat stays mounted however
// the tiles change, so its stream goes on.
function Places({ chat }: { chat: Ready }) {
  const wide = useWide();
  const [update, checkUpdate] = useUpdate();
  const { all: builtins } = useBuiltins();
  const mains = MAINS.filter((m) => m !== "chat" && builtins?.some((b) => b.id === m && b.on));
  // Settings: a centred window on a wide screen, a panel slid out from the left on a phone.
  const [over, setOver] = useState(false);
  const [pinsOn] = usePinsOn();
  const work = useWork(chat.tasks, true, chat.touched);
  // The kinds of tile as the host declares them (tileViews.tsx), asked once.
  useTiles();
  useEffect(() => void askTiles(), []);
  const library = useLibrary(true);
  const memories = useMemories();
  const watches = useWatches(true);
  const { send } = chat;
  // What the page asks the tiles to open: a link's page, the address it was opened at.
  const [asked, setAsked] = useState<{ pane: Pane; n: number; own?: boolean; anew?: boolean } | undefined>(() => {
    const page = new URLSearchParams(location.hash.slice(1)).get("page");
    const pane: Pane | undefined = page
      ? { kind: "library", arg: `page:${page}` }
      : location.hash === "#library"
        ? { kind: "library" }
        : location.hash === "#calendar"
            ? { kind: "builtin", arg: "calendar" }
            : undefined;
    // A #work link opens the composer's tree on every worker, once it is drawn.
    if (location.hash === "#work") setTimeout(showAllAgents, 800);
    if (location.hash) history.replaceState(null, "", location.pathname + location.search);
    return pane && { pane, n: 0 };
  });
  // `own`: in a tile, or a phone tab, of its own, not in place of a thread
  // already open (the composer's agents open each where it can stay).
  // `anew`: as the dock asks, a tile (or tab) of its own even where it shows.
  const request = useCallback((pane: Pane, own?: boolean, anew?: boolean) => setAsked((a) => ({ pane, n: (a?.n ?? 0) + 1, own, anew })), []);
  const openPage = useCallback((page: string) => request({ kind: "library", arg: `page:${page}` }), [request]);
  // A page's link followed in the chat itself changes only the fragment.
  useEffect(() => {
    const follow = () => {
      const page = new URLSearchParams(location.hash.slice(1)).get("page");
      if (!page) return;
      history.replaceState(null, "", location.pathname + location.search);
      openPage(page);
    };
    addEventListener("hashchange", follow);
    return () => removeEventListener("hashchange", follow);
  }, [openPage]);
  // A page's link anywhere in the app opens the page here, as the fragment
  // would, though it names app.libri.is or opens in a new tab as other
  // links do; a click held for a new tab or window still gets one.
  useEffect(() => {
    const click = (e: MouseEvent) => {
      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
      const a = (e.target as Element | null)?.closest?.("a[href]");
      const page = a && pageOf((a as HTMLAnchorElement).href);
      if (!page) return;
      e.preventDefault();
      openPage(page);
    };
    document.addEventListener("click", click, true);
    return () => document.removeEventListener("click", click, true);
  }, [openPage]);
  // A page published in the turn under way opens by itself, as an artifact
  // or canvas opens beside the chat when it is made. Only what the stream
  // tells live: the history never reopens one.
  const { published } = chat;
  useEffect(() => {
    if (published) openPage(published.page);
  }, [published, openPage]);
  const shell = useMemo<Shell>(
    () => ({
      settings: () => setOver((o) => !o),
      update,
      promote: (directions, from) => void send(promotion(directions, from), []),
    }),
    [send, update],
  );
  const toChat = () => request({ kind: "chat" });
  const say = (text: string) => {
    toChat();
    void send(text, []);
  };
  // A page's message waits in the composer, never goes to the agent unsent.
  const toComposer = (text: string) => {
    toChat();
    compose(text);
  };
  // Brought to the chat, a thread is handled there.
  const pull = (t: WorkThread) => {
    void work.mark(t);
    say(`Bring back from Work: ${t.name}'s thread. Tell me what came of it and what's next; its report is in your memory.`);
  };
  const openThread = (t: WorkThread) => {
    void work.mark(t);
    request({ kind: "thread", arg: t.id });
  };
  // Chat pressed on the dock while the chat is in focus goes back to the talk.
  const [left, setLeft] = useState(0);
  const closeOver = useCallback(() => setOver(false), []);
  const reached = useCallback(() => setToPage(undefined), []);
  const memory = useMemory();
  // The panel opens on a page when asked for one: the line's Memory, a
  // setting picked in the launcher.
  const [toPage, setToPage] = useState<Page>();
  const settingsAt = (to?: Page) => {
    setToPage(to);
    setOver(true);
  };
  // A built-in's empty view asks for Settings > Connections.
  useEffect(() => {
    const to = () => {
      setToPage("connections");
      setOver(true);
    };
    addEventListener("libri:connections", to);
    return () => removeEventListener("libri:connections", to);
  }, []);
  // + on Workers starts a worker on the person's own words, not through the
  // main chat; a line says what came of it.
  const [started, setStarted] = useState<{ name?: string; error?: string }>();
  useEffect(() => {
    if (!started) return;
    const id = window.setTimeout(() => setStarted(undefined), 8000);
    return () => window.clearTimeout(id);
  }, [started]);
  // + opens the Tasks sheet (TaskSheet), a whole screen on a phone and a
  // window on a wide one, as the Android app's TaskSheet.kt.
  const [newTask, setNewTask] = useState(false);
  const taskSheet = newTask && (
    <TaskSheet
      wide={wide}
      close={() => setNewTask(false)}
      start={async (brief, files, choice) => {
        const said = await work.start(brief, files, choice);
        setStarted(said);
        return !said.error;
      }}
    />
  );
  // The counts drop whole: past the empty first item they wrap out of the one
  // line this box shows, and the section heads carry the same numbers.
  // What the composer's circles of agents at work are drawn from.
  const { m: picks } = useModels(false);
  const running = useMemo<Running>(
    () => ({
      threads: work.threads,
      tasks: chat.tasks,
      main: picks?.pick,
      open: (t) => {
        void work.mark(t);
        request({ kind: "thread", arg: t.id }, true);
      },
      // What the Workers place held: the tree with the asleep, the watches
      // and the archived, each on its tab.
      all: () => (
        <>
          {started && (
            <p role="status" className={`px-5 pb-2 text-label ${started.error ? "text-accent" : "text-mid"}`}>
              {started.error ?? `Started ${started.name}. Its report comes to the chat.`}
            </p>
          )}
          <WorkersTile work={work} tasks={chat.tasks} main={{ live: chat.typing, wait: chat.asks.length > 0 }} open={openThread} pull={pull} watches={watches} openChat={(id) => request(id ? { kind: "chat", arg: id } : { kind: "chat" })} />
        </>
      ),
      start: () => setNewTask(true),
    }),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the rest are drawn fresh each time `all` is called
    [work, chat.tasks, picks?.pick, request, started, watches, chat.typing, chat.asks.length],
  );
  const builtinName = (id?: string) => builtins?.find((b) => b.id === id)?.title ?? MAIN_NAMES[id as Main] ?? "Built-in";
  // What the launcher opens besides the dock's places.
  const launchable: Launchable[] = [
    ...mains.map((m) => ({ group: "Places", label: builtinName(m), icon: KIND_ICONS.builtin, pane: { kind: "builtin", arg: m } as Pane })),
    { group: "Places", label: "Today", icon: KIND_ICONS.home, pane: { kind: "home" } },
    ...(pinsOn ? [{ group: "Places", label: "Pinned", icon: KIND_ICONS.pinned, pane: { kind: "pinned" } as Pane }] : []),
    { group: "Places", label: "Search the memory", detail: MAC_KEY, icon: KIND_ICONS.chat, run: openSearch },
    ...work.threads.map((t) => ({ group: "Workers", label: t.name, detail: t.state, icon: KIND_ICONS.thread, pane: { kind: "thread", arg: t.id } as Pane })),
    { group: "Workers", label: "All workers", detail: "asleep and archived too, and the watches", icon: WORKERS_ICON, run: () => (request({ kind: "chat" }), setTimeout(showAllAgents, 300)) },
    { group: "Workers", label: "Start a worker", icon: WORKERS_ICON, run: () => setNewTask(true) },
    ...library.pages.map((p) => ({ group: "Library", label: p.title || p.name, icon: KIND_ICONS.library, pane: { kind: "library", arg: `page:${p.name}` } as Pane })),
    ...library.lists.map((l) => ({ group: "Library", label: l.name, detail: "list", icon: KIND_ICONS.library, pane: { kind: "library", arg: `list:${l.name}` } as Pane })),
    ...LAUNCHED_PAGES.map((p) => ({ group: "Settings", label: PAGE_TITLES[p], icon: GEAR, run: () => settingsAt(p) })),
  ];
  const titleOf = (p: Pane) =>
                p.kind === "thread"
                  ? (work.threads.find((t) => t.id === p.arg)?.name ?? "Worker")
                  : p.kind === "chat" && p.arg
                    ? (memories?.find((m) => m.id === p.arg)?.name ?? "Memory")
                    : p.kind === "builtin"
                    ? builtinName(p.arg)
                    : p.kind === "library" && p.arg
                      ? (library.pages.find((g) => `page:${g.name}` === p.arg)?.title ?? p.arg.slice(p.arg.indexOf(":") + 1))
                      : KIND_NAMES[p.kind];
  const renderPane = (p: Pane, tile: TileApi) => {
                switch (p.kind) {
                  case "chat": {
                    // Its memories as tabs: a pick shows that one, opening
                    // its tab if it has none here (memoryTabs.tsx).
                    const tabs = tabsOf(p);
                    const shown = p.arg ?? "";
                    const show = (id: string, keep = tabs.includes(id) ? tabs : [...tabs, id]) =>
                      tile.replace({ kind: "chat", ...(id ? { arg: id } : {}), ...(keep.length > 1 || keep[0] !== id ? { tabs: keep } : {}) });
                    const close = (id: string) => {
                      const keep = tabs.filter((t) => t !== id);
                      if (!keep.length) return;
                      show(id === shown ? keep[Math.max(0, tabs.indexOf(id) - 1)] : shown, keep);
                    };
                    return (
                      <MemoryPickCtx.Provider value={{ id: p.arg ?? null, pick: (id) => show(id ?? "") }}>
                        <ChatTabsCtx.Provider value={<ChatTabs tabs={tabs} on={shown} show={(id) => show(id)} close={close} />}>
                          {p.arg ? <MemoryChatTile key={p.arg} id={p.arg} name={NAME} phone={tile.phone} /> : <Thread {...chat} threads={work.threads} left={left} newTemp={() => request({ kind: "temp" }, true, true)} />}
                        </ChatTabsCtx.Provider>
                      </MemoryPickCtx.Provider>
                    );
                  }
                  case "builtin":
                    return <MainView id={p.arg as Exclude<Main, "chat">} say={toComposer} />;
                  case "temp":
                    return <Temp key={p.arg ?? ""} id={p.arg} hold={(id) => tile.replace({ kind: "temp", arg: id })} name={NAME} close={tile.close} send={say} place={tile.phone ? "page" : "tile"} cards={chat.cards.filter((c) => c.scratchpad)} decide={chat.decide} />;
                  case "terminal":
                    return <Terminal key={p.arg ?? "main"} id={p.arg} phone={tile.phone} close={tile.close} />;
                  case "browser":
                    return <BrowserTile phone={tile.phone} />;
                  case "system":
                    return <SystemTile phone={tile.phone} />;
                  case "board":
                    return <BoardTile id={p.arg} pick={(id) => tile.replace({ kind: "board", arg: id })} phone={tile.phone} active={tile.focused} />;
                  case "thread":
                    return <ThreadTile key={p.arg} id={p.arg!} work={work} tasks={chat.tasks} stop={chat.stopTask} tell={chat.tellTask} close={tile.close} />;
                  case "library": {
                    const shelved: Shelved = !p.arg ? { all: true } : p.arg.startsWith("list:") ? { list: p.arg.slice(5) } : { page: p.arg.slice(5) };
                    return (
                      <LibraryItem
                        shelved={shelved}
                        library={library}
                        open={(s) => tile.replace(paneOf(s))}
                        say={say}
                        compose={toComposer}
                        back={p.arg ? () => tile.replace({ kind: "library" }) : undefined}
                      />
                    );
                  }
                  case "home":
                    return <Today work={work} open={openThread} calendar={(id) => request({ kind: "builtin", arg: id })} tasks={() => (request({ kind: "chat" }), setTimeout(showAllAgents, 300))} />;
                  case "pinned":
                    return (
                      <div className="h-full overflow-y-auto pt-2 pb-6">
                        <PinnedList messages={chat.messages} name={NAME} resume={(pin) => say(resumed(pin, NAME))} />
                      </div>
                    );
                }
  };
  // A Temp tile (bar item) closed ends its Temp; one still being made has none.
  const closeTemp = useCallback((p: Pane) => {
    if (p.kind === "temp" && p.arg && p.arg !== "new") void closeScratchpad(p.arg);
  }, []);
  // What a phone's plus adds to its workspace: every place, Temp and the
  // terminal among them (a design choice of 2026-11-10: they left the chat's
  // header for it).
  const addable: Addable[] = [
    ...offeredKinds(true).map((k) => ({ label: KIND_NAMES[k], icon: KIND_ICONS[k], pane: { kind: k } as Pane })),
    // Every worker, in the main chat's composer tree (agents.tsx).
    { label: "Workers", icon: WORKERS_ICON, run: () => (request({ kind: "chat" }), setTimeout(showAllAgents, 300)) },
    ...(pinsOn ? [{ label: KIND_NAMES.pinned, icon: KIND_ICONS.pinned, pane: { kind: "pinned" } as Pane }] : []),
    ...mains.map((m) => ({ label: builtinName(m), icon: KIND_ICONS.builtin, pane: { kind: "builtin", arg: m } as Pane })),
  ];
  return (
    <ShellCtx.Provider value={shell}>
     <RunningCtx.Provider value={running}>
      <div className="flex h-full flex-col">
        <MemoryLine memory={memory} detail={() => settingsAt("memory")} />
        {/* Layout containment makes this the frame of the fixed panel, so it
            sits under the line rather than over it. --panel-width is the
            phone panel's, and how far it moves the tiles: they keep 56px in
            view, as on Android. */}
        <div style={{ "--panel-width": "min(340px, calc(100% - 56px))" } as React.CSSProperties} className="relative min-h-0 flex-1 [contain:layout]">
          <div
            inert={over}
            className={`relative z-10 h-full bg-paper ${wide ? "" : `transition-[translate,border-radius] duration-[260ms] ease-out motion-reduce:transition-none ${over ? "translate-x-[var(--panel-width)] overflow-hidden rounded-l-[28px] ring-[1.25px] ring-frame" : ""}`}`}
          >
            {wide ? (
            <Tiles
              asked={asked}
              onClosed={closeTemp}
              settings={() => settingsAt()}
              update={update}
              launchable={launchable}
              again={(k) => k === "chat" && setLeft((n) => n + 1)}
              title={titleOf}
              marks={{ temp: <TempMark /> }}
              render={renderPane}
            />
            ) : (
              <Phone
                asked={asked}
              onClosed={closeTemp}
                again={(k) => k === "chat" && setLeft((n) => n + 1)}
                panel={() => setOver(true)}
                still={over || newTask}
                title={titleOf}
                addable={addable}
                marks={{ temp: <TempMark at="top-1.5 left-[38px]" /> }}
                render={renderPane}
              />
            )}
          </div>
          {/* The page left in view closes the panel, by a tap or a swipe
              back towards it. Outside the page's own box: that box is
              inert while the panel is open, and an inert box takes no
              tap, in WebKit or Chromium, so a scrim inside it left no way
              out on a phone (an iPhone, 0.9.73). */}
          {over && !wide && <Shut close={closeOver} />}
          <Panel windowed={wide} open={over} close={closeOver} update={update} check={checkUpdate} memory={memory} to={toPage} reached={reached} say={say} counts={{ working: work.working, waiting: work.waiting }} />
          {taskSheet}
        </div>
      </div>
      <Viewer />
      <Search wide={wide} name={NAME} />
     </RunningCtx.Provider>
    </ShellCtx.Provider>
  );
}

/** A worker's thread in a tile: read once it is done while in view, unless
 * it waits on a question or an approval, which reading is not. */
function ThreadTile({ id, work, tasks, stop, tell, close }: { id: string; work: Work; tasks: Task[]; stop: (name: string) => Promise<void>; tell: (name: string, text: string, files?: File[]) => Promise<boolean>; close: () => void }) {
  const t = work.threads.find((w) => w.id === id);
  useTileRows("thread", {
    "stop": { disabled: t?.state !== "working", run: () => void stop(id) },
    "archive": { disabled: !t, run: () => void work.act(id, t?.state === "archived" ? "restore" : "archive") },
  });
  useEffect(() => {
    if (t?.state === "waiting" && !asks(t)) void work.mark(t);
  }, [t, work]);
  return <AgentPage call={id} shown={t?.name} side tasks={tasks} stop={stop} tell={tell} close={close} thread={t} parent={parentOf(t, work.threads)} />;
}

/** The settings the launcher lists: those every host shows. */
const LAUNCHED_PAGES: Page[] = ["appearance", "chat", "usage", "connections", "notifications", "memory", "hosts"];
const MAC_KEY = /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘ K" : "Ctrl K";

/** A Library item as a tile's pane (tiles.tsx). */
function paneOf(s: Shelved): Pane {
  return "all" in s ? { kind: "library" } : "page" in s ? { kind: "library", arg: `page:${s.page}` } : { kind: "library", arg: `list:${s.list}` };
}



/** A picture opened large, from wherever it was tapped. */
const VIEW = "view-picture";
const view = (src: string) => window.dispatchEvent(new CustomEvent(VIEW, { detail: src }));

/** Over the page left in view beside the phone's open panel: a tap, or a
 * swipe left, closes the panel. Pointer events, so a finger in WebKit and
 * Chromium and a mouse alike. */
function Shut({ close }: { close: () => void }) {
  const from = useRef<number | null>(null);
  return (
    <div
      data-panel-shut
      aria-hidden
      onPointerDown={(e) => void (from.current = e.clientX)}
      onPointerUp={(e) => {
        const x = from.current;
        from.current = null;
        // A tap, or a swipe back towards the panel; a swipe the other way
        // is no request to close.
        if (x === null || e.clientX - x < 24) close();
      }}
      onPointerCancel={() => void (from.current = null)}
      className="absolute inset-y-0 right-0 left-[var(--panel-width)] z-20 touch-none"
    />
  );
}

/** The picture opened large over a dimmed page, fit to the screen, as a
 * phone's gallery shows one: a tap anywhere, Escape, or a swipe down on a
 * phone puts it away. */
function Viewer() {
  const [src, setSrc] = useState<string>();
  const [drag, setDrag] = useState(0);
  const from = useRef<number>(undefined);
  useEffect(() => {
    const open = (e: globalThis.Event) => setSrc((e as CustomEvent<string>).detail);
    window.addEventListener(VIEW, open);
    return () => window.removeEventListener(VIEW, open);
  }, []);
  useEffect(() => {
    if (!src) return;
    const key = (e: KeyboardEvent) => e.key === "Escape" && setSrc(undefined);
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [src]);
  if (!src) return null;
  const close = () => {
    setSrc(undefined);
    setDrag(0);
  };
  return (
    <div
      role="dialog"
      aria-label="Picture"
      onClick={close}
      onTouchStart={(e) => (from.current = e.touches.length === 1 ? e.touches[0].clientY : undefined)}
      onTouchMove={(e) => from.current !== undefined && setDrag(Math.max(0, e.touches[0].clientY - from.current))}
      onTouchEnd={() => {
        from.current = undefined;
        if (drag > 90) close();
        else setDrag(0);
      }}
      style={{ background: `rgb(0 0 0 / ${0.82 * (1 - Math.min(drag, 300) / 400)})` }}
      className="fixed inset-0 z-50 grid place-items-center p-[max(12px,env(safe-area-inset-top))_12px_max(12px,env(safe-area-inset-bottom))]"
    >
      <img src={src} alt="" style={{ transform: `translateY(${drag}px)` }} className={`max-h-full max-w-full rounded-lg object-contain shadow-lift ${drag ? "" : "transition-transform"}`} />
      <button type="button" onClick={close} aria-label="Close" className="absolute top-[max(10px,env(safe-area-inset-top))] right-3 grid size-10 place-items-center rounded-full bg-black/40 text-title leading-none text-white">
        ×
      </button>
    </div>
  );
}

/** A page with one thing on it, centred high, as UniiChat's sign-in. */
function Solo({ children }: { children: React.ReactNode }) {
  return (
    <>
      <ThemeButton className="fixed top-2.5 right-3" />
      <main className="mx-auto mt-[18vh] w-[min(420px,100%-48px)] text-center">{children}</main>
    </>
  );
}

/** Switches light and dark, showing where a press takes you. */
function ThemeButton({ className = "" }: { className?: string }) {
  const [theme, toggle] = useTheme();
  const dark = theme === "dark";
  return (
    <button
      onClick={toggle}
      aria-label={dark ? "Light mode" : "Dark mode"}
      title={dark ? "Light mode" : "Dark mode"}
      className={`grid size-9 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink ${className}`}
    >
      <svg viewBox="0 0 24 24" className="size-[18px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
        {dark ? (
          <>
            <circle cx="12" cy="12" r="4" />
            <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41" />
          </>
        ) : (
          <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
        )}
      </svg>
    </button>
  );
}

/** `scope` is this device's, when it is a paired one (host.rs). */
type Me = { name: string; voice: string | null; dictate?: boolean; call?: boolean; models: boolean; forgets: number[]; scope?: "full" | "read" | null };

// Who answers here, asked once for the speak buttons:
// on first use, not at load, so a page that pairs first asks as the
// paired device and learns its scope.
let ME: Promise<Me> | undefined;
const askMe = () =>
  (ME ??= api("api/me")
    .then((r) => (r.ok ? r.json() : Promise.reject()))
    .catch(() => ({ name: "", voice: null, models: false, forgets: [] })));

/** Asks again, for a change that alters who answers here. */
const meChanged = () => {
  ME = undefined;
  window.dispatchEvent(new Event("me"));
};

function useMe() {
  const [me, setMe] = useState<Me | null>(null);
  useEffect(() => {
    const again = () => void askMe().then(setMe);
    window.addEventListener("me", again);
    return () => window.removeEventListener("me", again);
  }, []);
  useEffect(() => {
    askMe().then(setMe);
  }, []);
  return me;
}

/** Keeps an open menu whole inside its row of pills, the container its width
 * caps at (`max-w-[100cqw]`): a pill far along a narrow composer opens its menu
 * leftward by what would run past the end, so it never goes under the next
 * column or off the window. */
function useMenuInRow(anchor: React.RefObject<HTMLDivElement | null>, open: boolean) {
  const menu = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const row = anchor.current?.closest(".\\@container");
    if (!open || !row) return;
    const place = () => {
      if (!menu.current || !anchor.current) return;
      const from = anchor.current.getBoundingClientRect().left;
      const over = from + menu.current.offsetWidth - row.getBoundingClientRect().right;
      menu.current.style.translate = over > 0 ? `${-over}px` : "";
    };
    place();
    const watch = new ResizeObserver(place);
    watch.observe(row);
    return () => watch.disconnect();
  }, [open]);
  return menu;
}

/** A menu that shuts when a press or focus lands anywhere outside it, or
 * on Escape, as Radix's do; the press still does its own work, so a tap
 * into the composer both shuts the menu and starts typing. */
function useMenu() {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const outside = (e: globalThis.Event) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    // Caught first and stopped: Escape shuts the menu, not the Temp chat behind it.
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setOpen(false);
    };
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("focusin", outside, true);
    document.addEventListener("keydown", key, true);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("focusin", outside, true);
      document.removeEventListener("keydown", key, true);
    };
  }, [open]);
  return [open, setOpen, ref] as const;
}

// What a person says sits in the composer's own lifted paper and frame
// line, as it was typed: an ink edge (9:1) was the loudest line on the
// page, louder than the answer it asked for.
const EDGE = "border-frame bg-lift";
const BUBBLE = "relative w-fit max-w-[min(82%,560px)] rounded-3xl rounded-br-md border-[1.25px] px-[17px] py-2.5 whitespace-pre-wrap [overflow-wrap:anywhere] [text-wrap:wrap]";

/** The talk's text size (pinch.ts), on what sits inside a chat-column and
 * never on the column itself: zoom scales a box's own width, so a zoomed
 * column would stop meeting the composer's edges, which the same
 * chat-column places (a design choice of 2026-11-04). */
const TALK_SIZE = { zoom: "var(--talk, 1)" } as const;

/** The floating pill the header's controls sit in, over the talk. */
const FLOAT = "rounded-full border border-hair bg-lift/85 p-1 shadow-dock backdrop-blur-md";
/** One header button as its own disc, as Android's FloatButton: each child of
 * the row gets the pill's paper, edge and shadow. */
const DISC = "[&>*]:grid [&>*]:size-11 [&>*]:place-items-center [&>*]:floating";

/** A round icon button for the header; `children` are the icon's paths.
 * Icon only, the label in its tooltip: the words beside the icons, tried on
 * wide screens, were dropped (a design choice of 2026-11-05). */
function HeaderButton({ label, onClick, pressed, opens, children }: { label: string; onClick: (e: React.MouseEvent<HTMLButtonElement>) => void; pressed?: boolean; opens?: string; children: React.ReactNode }) {
  return (
    <button type="button" onClick={onClick} aria-label={label} data-open={opens} title={label} aria-pressed={pressed} className={`flex h-9 min-w-9 items-center justify-center rounded-full hover:bg-hover hover:text-ink ${pressed ? "bg-hover text-ink" : "text-mid"}`}>
      <svg viewBox="0 0 24 24" className="size-[18px] shrink-0" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
        {children}
      </svg>
    </button>
  );
}

/** The replies on offer and how a tap sends one, for the newest answer to
 * draw inside itself: under its words, above its buttons, so the gap to
 * them is a message's own at every width (Piers, 2026-11-06). */
type Offering = { offer: ReplyOffer; send: Ready["send"]; reply: Reply };
const OfferCtx = createContext<Offering | null>(null);

// The build the page runs, named by its hashed script; a deploy changes it.
const BUILD = document.querySelector<HTMLScriptElement>('script[type="module"][src]')?.getAttribute("src");

/** True once the server has a newer build than this page: checked when the
 * app comes back to the front, at most once a minute. An installed app
 * lives on in the background, so it would otherwise run an old build until
 * its process died. */
function useUpdate(): [boolean, () => Promise<boolean | null>] {
  const [ready, setReady] = useState(false);
  // Asked now, whatever the minute: whether a newer build waits, or null offline.
  const ask = useCallback(async () => {
    try {
      const html = await (await fetch("./", { cache: "no-store" })).text();
      const now = /<script type="module"[^>]*src="([^"]+)"/.exec(html)?.[1];
      const newer = !!(BUILD && now && now !== BUILD);
      if (newer) setReady(true);
      return newer;
    } catch {
      return null;
    }
  }, []);
  useEffect(() => {
    let last = 0;
    const check = () => {
      if (document.visibilityState !== "visible" || Date.now() - last < 60_000) return;
      last = Date.now();
      void ask();
    };
    check();
    document.addEventListener("visibilitychange", check);
    return () => document.removeEventListener("visibilitychange", check);
  }, [ask]);
  return [ready, ask];
}

/** Reloads the app on the newest build: the worker first, so notifications
 * run the new sw.js too. */
async function reload() {
  try {
    await (await navigator.serviceWorker?.getRegistration())?.update();
  } catch {
    // An old worker only delays its own update; the page still reloads.
  }
  location.reload();
}


/** The settings' pages, in the order the list shows them. */
type Page = "appearance" | "chat" | "models" | "usage" | "workers" | "guide" | "connections" | "notifications" | "voice" | "memory" | "hosts";
const PAGE_TITLES: Record<Page, string> = {
  appearance: "Appearance",
  chat: "Chat",
  models: "Models",
  usage: "Usage",
  workers: "Workers",
  guide: "Agents",
  connections: "Connections",
  notifications: "Notifications",
  voice: "Voice",
  memory: "Memory",
  hosts: "Hosts",
};




/** A cog, for Settings. */
function GearIcon({ className }: { className: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  );
}


/** The host's build, asked once a page: a newer build reloads the page,
 * so the answer holds as long as the page does. A failed ask is forgotten
 * and asked again. */
let hostBuild: Promise<string | undefined> | undefined;
function hostVersion() {
  hostBuild ??= api("api/host", { cache: "no-store" })
    .then((r) => (r.ok ? (r.json() as Promise<HostInfo>) : null))
    .then((h) => h?.version)
    .catch(() => undefined)
    .then((v) => {
      if (!v) hostBuild = undefined;
      return v;
    });
  return hostBuild;
}

/** The update button of the side panel's foot and the left column's: a
 * tap checks, and says for a few seconds what it found; with a newer build
 * waiting it reloads into it. */
function UpdateButton({ update, check, known }: { update: boolean; check: () => Promise<boolean | null>; known?: string }) {
  const [found, setFound] = useState<string>();
  // The host's build names the title and "Up to date": the one the caller
  // already holds, else the page's single ask, shared by every button.
  const [asked, setAsked] = useState<string>();
  useEffect(() => {
    if (known) return;
    let live = true;
    void hostVersion().then((v) => live && setAsked(v));
    return () => {
      live = false;
    };
  }, [known]);
  const version = known ?? asked;
  const updateTap = async () => {
    if (update) return reload();
    setFound("Checking…");
    const newer = await check();
    setFound(newer === null ? "Couldn't check; offline?" : newer ? undefined : `Up to date${version ? ` · v${version}` : ""}`);
  };
  useEffect(() => {
    if (!found || found === "Checking…") return;
    const t = setTimeout(() => setFound(undefined), 4000);
    return () => clearTimeout(t);
  }, [found]);
  return (

    <span className="relative inline-flex shrink-0">
      {found && <span className="absolute right-0 bottom-full z-50 mb-1 w-max max-w-48 rounded-xl border border-hair bg-lift px-3 py-2 text-label text-mid shadow-lift" role="status">{found}</span>}
      <button type="button" onClick={() => void updateTap()} aria-label={update ? "Update ready: reload" : "Check for updates"} title={update ? "Update ready: reload" : version ? `v${version} · check for updates` : "Check for updates"} className={`relative grid size-9 shrink-0 place-items-center rounded-full hover:bg-hover ${update ? "text-accent" : "text-mid hover:text-ink"}`}>
        <svg viewBox="0 0 24 24" className="size-[20px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
          <circle cx="12" cy="12" r="10" />
          <path d="m16 12-4-4-4 4M12 16V8" />
        </svg>
        {update && <span className="absolute top-1 right-1 size-2 rounded-full bg-accent" />}
      </button>
    </span>
  );
}


/** Settings, as a centred window on a wide screen, or on a phone a panel
 * that slides out from the left over the chat. The panel
 * opens on a list of categories, each with its current value, and a tap
 * slides that category's page in, as the phone apps' settings do. Escape
 * goes back to the list, then puts the panel away; so do the shade and the
 * close button. The window shows the same pages beside their list, and
 * Escape puts it away. */
/** What the chat's place can show: the chat, and each built-in with a view. */
type Main = "chat" | "calendar";
const MAINS: readonly Main[] = ["chat", "calendar"];
const MAIN_NAMES: Record<Main, string> = { chat: "Chat", calendar: "Calendar" };

/** A built-in's view, filling its tile. */
function MainView({ id, say }: { id: Exclude<Main, "chat">; say: (text: string) => void }) {
  const builtin = useBuiltins().all?.find((b) => b.id === id);
  return (
    <div className="flex h-full flex-col bg-paper">
      <div className="flex min-h-0 flex-1 flex-col">
        {/* No connection fills it yet: the one way forward, not a frame
            that says so. */}
        {builtin && !builtin.connection ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center">
            <p className="text-body text-ink">No connection yet</p>
            <p className="max-w-80 text-control text-mid">Add a connection, such as Fastmail, and {builtin.title} fills from it on its own.</p>
            <button type="button" onClick={() => dispatchEvent(new Event("libri:connections"))} className="min-h-11 rounded-full bg-ink px-5 text-control font-medium text-paper hover:opacity-85">
              Add a connection
            </button>
          </div>
        ) : (
          <Suspense fallback={<div className="min-h-0 flex-1" />}>
            <BuiltinFrame id={id} say={say} />
          </Suspense>
        )}
      </div>
    </div>
  );
}

function Panel({ open, close, update, check, memory, to, reached, say, counts, windowed = false }: { open: boolean; close: () => void; say: (text: string) => void; update: boolean; check: () => Promise<boolean | null>; memory: MemoryHealth | null; to: Page | undefined; reached: () => void; counts: { working: number; waiting: number }; windowed?: boolean }) {
  const [pinsOn, setPinsOn] = usePinsOn();
  const [theme, toggleTheme] = useTheme();
  const [px, setPx, bounds] = useTextSize();
  // Mounted with the header, not the panel, so the worker registers and
  // the bell's state is known before the panel first opens.
  const [bell, toggleBell] = usePush();
  const me = useMe();
  const voice = me?.voice;
  const [choice, setChoice] = useVoiceChoice();
  const [speed, setSpeed] = useVoiceSpeed();
  const [mode, setMode] = useState<CallMode>(callMode);
  const pickMode = (m: CallMode) => {
    setCallMode(m);
    setMode(m);
  };
  const detail = useChatDetail();
  const voices = useVoices(open && !!voice);
  const [forget, pickForget] = useForget(open && !!me?.forgets?.length);
  const { host, seen } = useHost(open);
  const known = useKnownHosts(open && !windowed);
  const makers = useMakers(open && !!me?.models);
  const { m: models } = useModels(open && !!me?.models);
  const usage = useUsage(false);
  const names = useSetting<WorkerNamesNow>("api/worker-names", open);
  const guide = useSetting<{ text: string }>("api/guide", open);
  const hostFacts = useSetting<{ text: string }>("api/guide/host", open);
  const skills = useSkills(open);
  const [connected, loadConnected] = useConnections(open);
  const [hearing, setHearing] = useState(false);
  // Three levels, each sliding over the last: the host, the settings'
  // list, one category's page.
  const [view, setView] = useState<"home" | "list" | Page>("home");
  const page = view === "home" || view === "list" ? null : view;
  // The page last opened stays drawn while the list slides back over it.
  const [shown, setShown] = useState<Page>("appearance");
  const shut = useRef<HTMLButtonElement>(null);
  const back = useRef<HTMLButtonElement>(null);
  const go = (v: "list" | Page) => {
    if (v !== "list") setShown(v);
    setView(v);
  };
  const up = () => setView((v) => (v === "home" || v === "list" ? "home" : "list"));
  useEffect(() => {
    if (!to) return;
    go(to);
    reached();
  }, [to, reached]);
  useEffect(() => {
    if (!open) {
      // Back to the host once the panel has slid away, not while it does.
      const t = setTimeout(() => setView("home"), 200);
      return () => clearTimeout(t);
    }
    (view === "home" || windowed ? shut : back).current?.focus();
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (view !== "home" && !windowed) up();
      else close();
    };
    document.addEventListener("keydown", key);
    return () => document.removeEventListener("keydown", key);
  }, [open, close, view, windowed]);
  const picked = choice ?? voice;
  const hear = () => {
    if (hearing) return stop();
    if (!picked) return;
    setHearing(true);
    play(`Hi, I'm ${me?.name}. This is how I sound.`, picked, () => setHearing(false));
  };
  const row = "flex min-h-11 w-full items-center justify-between gap-6 px-5 py-2.5 text-left text-body hover:bg-hover disabled:opacity-40 disabled:hover:bg-transparent";
  const note = "text-label text-mid";
  const select = "max-w-44 bg-transparent text-right text-label text-mid outline-none";
  const voiceName = (id: string) => voices.find((v) => v.id === id)?.name ?? id;
  const bellWord = { on: "On", off: "Off", denied: "Blocked", unsupported: "Not on this browser" }[bell];
  // A category shows only when it has something to set or see.
  const pages: [Page, string][] = [
    ["appearance", `${theme === "dark" ? "Dark" : "Light"} · ${px}px`],
    ["chat", `Pinned ${pinsOn ? "on" : "off"}`],
    ...(me?.models ? [["models", models?.offers.find((o) => o.harness === models.pick.harness && o.model === models.pick.model)?.name ?? ""] as [Page, string]] : []),
    ["usage", usage?.accounts.map((a) => a.name).join(", ") || "Spend"],
    ...(names.now ? [["workers", names.now.allowed ? `${names.now.allowed} names` : "Numbered"] as [Page, string]] : []),
    ...(guide.now ? [["guide", guide.now.text ? "Yours" : "None"] as [Page, string]] : []),
    ...(connected ? [["connections", [...connected.servers.filter((c) => c.on).map((c) => c.name), connected.xai && "xAI"].filter(Boolean).join(", ") || "None"] as [Page, string]] : []),
    ["notifications", bellWord],
    ["voice", voice && picked ? `${voiceName(picked)} · ${speed.toFixed(1)}×` : "Dictation"],
    ["memory", memory ? MEMORY_WORD[memory.state] : forget ? forgetLabel(forget) : ""],
    ["hosts", host?.name ?? ""],
  ];
  const body: Record<Page, () => React.ReactNode> = {
    appearance: () => (
      <>
      <Section title="Display">
        <button type="button" onClick={toggleTheme} className={row}>
          Theme
          <span className={note}>{theme === "dark" ? "Dark" : "Light"}</span>
        </button>
        {/* The pinch's size, by the pixel, for a desktop and for anyone
            who'd rather not pinch. */}
        <div className="flex w-full items-center justify-between gap-3 px-5 py-1.5 text-body">
          Text size
          <span className="flex items-center gap-1">
            <button type="button" onClick={() => setPx(px - 1)} disabled={px <= bounds.min} aria-label="Smaller text" className="grid size-7 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink disabled:opacity-40 disabled:hover:bg-transparent">
              −
            </button>
            <span className={`${note} w-10 text-center tabular-nums`}>{px}px</span>
            <button type="button" onClick={() => setPx(px + 1)} disabled={px >= bounds.max} aria-label="Larger text" className="grid size-7 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink disabled:opacity-40 disabled:hover:bg-transparent">
              +
            </button>
          </span>
        </div>
      </Section>
      <Section title="Workspaces">
        <div className={row} data-setting="undo-close">
          Undo after closing one
          <span className={note}>{UNDO_CLOSE_MS / 1000} seconds</span>
        </div>
        <div className={row} data-setting="kept-closed">
          Recently closed keeps it
          <span className={note}>{KEPT_CLOSED_DAYS} days</span>
        </div>
      </Section>
      </>
    ),
    chat: () => (
      <>
        <Section title="Messages">
          <div className={row}>
            <label htmlFor="chat-detail">Chat detail</label>
            <select id="chat-detail" className={select} value={detail} onChange={(e) => setChatDetail(e.target.value as ChatDetail)}>
              <option value="simple">Simple</option><option value="standard">Standard</option><option value="full">Full</option>
            </select>
          </div>
          <p className={`${note} px-5 pb-2`}>Simple shows replies only; Full adds tools and thinking.</p>
          <button type="button" role="switch" aria-checked={pinsOn} onClick={() => setPinsOn(!pinsOn)} className={row}>
            Pinned
            <Toggle on={pinsOn} />
          </button>
          <p className={`${note} px-5 pb-2`}>Pin messages to find them later.</p>
        </Section>
      </>
    ),
    models: () => <Models want={open} makers={makers} />,
    usage: () => <UsagePage usage={usage} row={row} note={note} />,
    workers: () => <WorkerNames names={names} row={row} note={note} />,
    guide: () => (
      <>
        <GuideEditor guide={guide} note={note} title="Rules" label="Your rules" about="Every model reads these each turn. Calls read only the # Identity section." />
        <GuideEditor guide={hostFacts} note={note} title="Host facts" label="This host's facts" about="Facts about this host, for the main chat, Temp and workers." />
        <MemoriesEditor note={note} />
        {skills[0] && (
          <Section title="Skills">
            <SkillsPage skills={skills} row={row} note={note} read={me?.scope === "read"} say={(text) => { close(); say(text); }} />
          </Section>
        )}
      </>
    ),
    connections: () => (
      <>
      <Section title="Connections">
        <Connections now={connected} load={loadConnected} row={row} note={note} read={me?.scope === "read"} />
      </Section>
      <Section title="Built-ins">
        <Builtins load={loadConnected} note={note} read={me?.scope === "read"} />
      </Section>
      <Section title="xAI">
        <Xai now={connected} load={loadConnected} changed={meChanged} row={row} note={note} read={me?.scope === "read"} />
      </Section>
      <Section title="Other apps">
        <OtherApps row={row} note={note} read={me?.scope === "read"} />
      </Section>
      </>
    ),
    notifications: () => (
      <Section title="This device">
        <button type="button" role="switch" aria-checked={bell === "on"} onClick={toggleBell} disabled={bell === "denied" || bell === "unsupported"} title={bell === "denied" ? "Blocked in the browser's settings" : undefined} className={row}>
          Push notifications
          {bell === "on" || bell === "off" ? <Toggle on={bell === "on"} /> : <span className={note}>{bellWord}</span>}
        </button>
        <p className={`${note} px-5 pb-2`}>
          {bell === "denied"
            ? "Blocked by the browser. Allow notifications for this site."
            : bell === "unsupported"
              ? "Not available here. On iPhone, add Libri to the Home Screen first."
              : "Watches, alerts and messages, sent to this device."}
        </p>
      </Section>
    ),
    voice: () => (
      <>
      {voice && (
      <Section title="Voice">
        <div className="flex w-full items-center justify-between gap-3 px-5 py-2.5 text-body">
          <label htmlFor="voice">Voice</label>
          <span className="flex items-center gap-1">
            <select
              id="voice"
              value={picked ?? ""}
              onChange={(e) => {
                stop();
                setChoice(e.target.value);
              }}
              className={`${select} capitalize`}
            >
              {/* The shown voice stays listed while xAI's list loads, or if it fails. */}
              {[...new Set([...voices.map((v) => v.id), ...(picked ? [picked] : [])])].map((id) => (
                <option key={id} value={id}>
                  {voiceName(id)}
                </option>
              ))}
            </select>
            <button
              type="button"
              onClick={hear}
              aria-label={hearing ? "Stop the sample" : "Hear this voice"}
              title={hearing ? "Stop the sample" : "Hear this voice"}
              className="grid size-7 place-items-center rounded-md text-mid hover:bg-hover hover:text-ink"
            >
              <svg viewBox="0 0 24 24" className={`size-[15px] ${hearing ? "animate-pulse" : ""}`} fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
                {hearing ? <rect x="6" y="6" width="12" height="12" rx="1.5" /> : <path d="m7 4 12 8-12 8z" />}
              </svg>
            </button>
          </span>
        </div>
        <label className="block px-5 py-2.5 text-body">
          <span className="flex justify-between gap-6">
            Voice speed
            <span className={note}>{speed.toFixed(1)}×</span>
          </span>
          <input type="range" min={SPEED.min} max={SPEED.max} step={SPEED.step} value={speed} onChange={(e) => setSpeed(Number(e.target.value))} className="mt-2 w-full accent-ink" />
        </label>
        <label className={row}>
          Calls
          <select value={mode} onChange={(e) => pickMode(e.target.value as CallMode)} className={select}>
            <option value="chat">Talk with the chat</option>
            <option value="grok">Grok Voice in front</option>
          </select>
        </label>
      </Section>
      )}
      <Section title="Dictation">
        <DictationSettings row={row} note={note} select={select} read={me?.scope === "read"} changed={meChanged} />
      </Section>
      </>
    ),
    memory: () => (
      <>
      {memory && (
        <Section title="Status">
          <Fact label="State" value={MEMORY_WORD[memory.state]} />
          {memory.state === "failing" && (
            <>
              <p className="px-5 py-2.5 text-body text-accent">{memory.reason}</p>
              {memory.since && <Fact label="Since" value={stamp(new Date(memory.since))} />}
            </>
          )}
          {memory.newest_summary && <Fact label="Newest summary" value={stamp(new Date(memory.newest_summary))} />}
          {!!memory.waiting && <Fact label="Waiting" value={memory.waiting.toLocaleString()} />}
          {memory.lines != null && <Fact label="Total lines" value={memory.lines.toLocaleString()} />}
          {memory.bytes != null && <Fact label="Total size" value={sized(memory.bytes)} />}
        </Section>
      )}
      <Section title="Memory">
        {!!me?.forgets?.length && (
          <div className="flex w-full items-center justify-between gap-3 px-5 py-2.5 text-body">
            <label htmlFor="forget">Forget after</label>
            <select id="forget" value={forget ?? ""} onChange={(e) => pickForget(Number(e.target.value))} className={select}>
              {me.forgets.map((m) => (
                <option key={m} value={m}>
                  {forgetLabel(m)}
                </option>
              ))}
            </select>
          </div>
        )}
      </Section>
      </>
    ),
    hosts: () => (
      <>
        <Section title="This machine">
          {/* The one in use and how it answers: the side panel's head row
              until 11-06, when they were moved the host into Settings. */}
          <div className="flex w-full items-center gap-2.5 px-5 py-2.5 text-body">
            <span className={`size-2 shrink-0 rounded-full ${seen === "down" ? "bg-accent" : seen !== undefined ? "bg-active" : "bg-soft"}`} aria-hidden />
            <span className="min-w-0 flex-1 truncate">{host?.name ?? (seen === "down" ? "Not answering" : "Asking…")}</span>
            <span className={`${note} shrink-0 tabular-nums`}>{seen === "down" ? "Down" : seen !== undefined ? `Up · ${seen} ms` : "…"}</span>
          </div>
          {!host ? (
            <div className={`${note} px-5 py-2.5`}>Asking…</div>
          ) : (
            <>
              <Fact label="Assistant" value={host.assistant ?? host.name} />
              <Fact label="Version" value={host.commit ? `${host.version} · ${host.commit}` : host.version} />
              <Fact label="Running for" value={lasted(host.uptime)} />
              <Fact label="Host ID" value={host.id.slice(0, 8)} title={host.id} />
              {host.url && <Fact label="Address" value={host.url.replace(/^https?:\/\//, "")} />}
              {host.addresses?.map((a) => (
                <Fact key={a} label="Listens on" value={a} />
              ))}
            </>
          )}
        </Section>
        <Section title="Devices">
          <Devices open={open && shown === "hosts"} row={row} note={note} read={me?.scope === "read"} />
        </Section>
        <Section title="Peers">
          <Peers open={open && shown === "hosts"} row={row} note={note} read={me?.scope === "read"} />
        </Section>
        <Section title="Other machines">
          <Hosts open={open && shown === "hosts"} row={row} note={note} />
        </Section>
      </>
    ),
  };
  const shade = <div onClick={close} aria-hidden className={`fixed inset-0 z-40 bg-black/25 transition-opacity duration-200 motion-reduce:transition-none ${open ? "opacity-100" : "pointer-events-none opacity-0"}`} />;
  const shutButton = (
    <button ref={shut} type="button" onClick={close} aria-label="Close the side panel" title="Close" className="-mr-2 grid size-9 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink">
      <svg viewBox="0 0 24 24" className="size-[18px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
        <path d="M18 6 6 18M6 6l12 12" />
      </svg>
    </button>
  );
  // Updates: lit when a newer build waits, a tap loads it; otherwise a tap
  // checks now and says the version.
  const updateButton = <UpdateButton update={update} check={check} known={host?.version} />;
  if (windowed)
    return (
      <>
        {shade}
        <aside
          role="dialog"
          aria-modal
          aria-label="Settings"
          aria-hidden={!open}
          inert={!open}
          className={`fixed top-1/2 left-1/2 z-50 flex h-[min(640px,calc(100%-48px))] w-[min(820px,calc(100%-48px))] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-2xl border-[1.25px] border-edge bg-paper shadow-lift transition duration-200 ease-out motion-reduce:transition-none ${open ? "opacity-100" : "pointer-events-none scale-[0.97] opacity-0"}`}
        >
          <div className="flex h-[57px] shrink-0 items-center justify-between border-b border-hair px-5">
            <span className="font-serif text-title">Settings</span>
            {shutButton}
          </div>
          <div className="flex min-h-0 flex-1">
            <nav aria-label="Settings categories" className="flex w-[220px] shrink-0 flex-col border-r border-hair">
              <div className="min-h-0 flex-1 overflow-y-auto p-2">
                {pages.map(([p]) => (
                  <button key={p} type="button" onClick={() => go(p)} aria-current={(page ?? "appearance") === p ? "page" : undefined} className={`block w-full rounded-xl px-3 py-2 text-left text-body ${(page ?? "appearance") === p ? "bg-hover text-ink" : "text-mid hover:bg-hover hover:text-ink"}`}>
                    {PAGE_TITLES[p]}
                  </button>
                ))}
              </div>
              <div className="flex flex-wrap items-center gap-1 px-3 py-2" aria-label="Libri version and updates">
                <span className="mr-auto text-label text-mid">Libri{host?.version ? ` v${host.version}` : ""}</span>
                {updateButton}
              </div>
            </nav>
            {/* The window opens on its first page: there is no host page above
                the categories since the host moved into them (11-06). */}
            <div role="region" aria-label={PAGE_TITLES[page ?? "appearance"]} className="min-w-0 flex-1 overflow-y-auto py-2">
              {body[page ?? "appearance"]()}
            </div>
          </div>
        </aside>
      </>
    );
  // On a phone the panel lies under the places at the left, and opening it
  // moves them over (Places, as Android's), so no shade: the places in view
  // at the right edge are the way back. It comes in a quarter behind them,
  // as on Android, and is hidden once shut.
  return (
    <aside
      aria-label="Settings"
      aria-hidden={!open}
      inert={!open}
      className={`fixed inset-y-0 left-0 z-0 flex w-[var(--panel-width)] flex-col bg-paper pb-[env(safe-area-inset-bottom)] pl-[env(safe-area-inset-left)] transition-[translate,visibility] duration-[260ms] ease-out motion-reduce:transition-none ${open ? "visible translate-x-0" : "invisible -translate-x-1/4"}`}
    >
        {/* The mark and the name, as Claude Duet's panel heads itself (a design choice of
            2026-11-06); spacing, not a rule, parts it from what follows. */}
        <div className={`flex shrink-0 items-center px-5 pt-[env(safe-area-inset-top)] ${view === "home" ? "h-3" : "h-[72px]"}`}>
          {view !== "home" && (
            <span className="-ml-2 flex min-w-0 items-center gap-1">
              <button ref={back} type="button" onClick={up} aria-label={page ? "Back to settings" : "Back"} title="Back" className="grid size-9 shrink-0 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink">
                <Chevron left />
              </button>
              <span className="truncate font-serif text-title">{page ? PAGE_TITLES[page] : "Settings"}</span>
            </span>
          )}
        </div>
        <div className="relative min-h-0 flex-1 overflow-clip">
          {/* As Tern's drawer: the host's
              state at the top, then Hosts and Settings, and at the foot the
              theme and the version. The places are the bar's, and a phone's
              workspaces its pill's. */}
          <div
            role="region"
            aria-label="Side panel"
            inert={view !== "home"}
            aria-hidden={view !== "home"}
            className={`absolute inset-0 flex flex-col transition-transform duration-200 ease-out motion-reduce:transition-none ${view === "home" ? "visible translate-x-0" : "invisible pointer-events-none -translate-x-full"}`}
          >
            <section data-host-status aria-label="Host" className="shrink-0 px-6 pt-6 pb-5">
              <span aria-hidden className="grid size-12 place-items-center rounded-full bg-ink font-serif text-title text-paper">{(host?.name ?? "L").slice(0, 1).toUpperCase()}</span>
              <p className="mt-3 truncate text-title font-medium text-ink">{host?.name ?? "Libri"}</p>
              <p className="mt-0.5 flex min-w-0 items-center gap-2 text-body text-mid">
                <span className="truncate">{(remote()?.url ?? location.origin).replace(/^https?:\/\//, "")}</span>
                <span aria-hidden className={`size-2 shrink-0 rounded-full ${seen === "down" ? "bg-accent" : seen === undefined ? "bg-soft" : "bg-active"}`} />
                <span className="shrink-0">{seen === "down" ? "Unreachable" : seen === undefined ? "Connecting" : "Connected"}</span>
              </p>
              <p className="mt-3 flex gap-5 text-body text-mid">
                <span><span className="font-medium text-ink">{counts.working}</span> working</span>
                <span><span className="font-medium text-ink">{counts.waiting}</span> waiting</span>
              </p>
            </section>
            <div className="mx-6 h-px shrink-0 bg-hair" />
            <div className="min-h-0 flex-1 overflow-y-auto px-3 pt-2">
              <button type="button" data-open="hosts" onClick={() => go("hosts")} className="flex w-full items-center gap-3.5 rounded-full px-4 py-3 text-left text-body text-ink hover:bg-hover">
                <svg viewBox="0 0 24 24" aria-hidden className="size-5 text-mid" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
                  <rect x="3" y="4" width="18" height="7" rx="2" />
                  <rect x="3" y="13" width="18" height="7" rx="2" />
                  <path d="M7 7.5h.01M7 16.5h.01" />
                </svg>
                <span className="flex-1">Hosts</span>
                <span className="text-label text-soft">{known.hosts.length || ""}</span>
              </button>
              <button type="button" data-open="settings" onClick={() => go("list")} className="flex w-full items-center gap-3.5 rounded-full px-4 py-3 text-left text-body text-ink hover:bg-hover">
                <GearIcon className="size-5 text-mid" />
                Settings
              </button>
            </div>
            <div className="flex shrink-0 items-center gap-1 border-t border-hair px-3 py-2">
              <button type="button" data-action="theme" onClick={toggleTheme} aria-label={`${theme === "dark" ? "Dark" : "Light"}: switch to ${theme === "dark" ? "light" : "dark"}`} className="flex items-center gap-2.5 rounded-full px-3 py-2 text-body text-ink hover:bg-hover">
                <svg viewBox="0 0 24 24" aria-hidden className="size-5 text-mid" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
                  {theme === "dark" ? <path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5Z" /> : <><circle cx="12" cy="12" r="4" /><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" /></>}
                </svg>
                {theme === "dark" ? "Dark" : "Light"}
              </button>
              <span className="flex-1" />
              {update ? updateButton : <span className="px-2 font-mono text-label text-soft">Libri {host?.version ?? ""}</span>}
            </div>
          </div>
          <nav
            aria-label="Settings categories"
            inert={view !== "list"}
            aria-hidden={view !== "list"}
            className={`absolute inset-0 flex flex-col transition-transform duration-200 ease-out motion-reduce:transition-none ${view === "list" ? "visible translate-x-0" : view === "home" ? "invisible pointer-events-none translate-x-full" : "invisible pointer-events-none -translate-x-full"}`}
          >
            <div className="min-h-0 flex-1 overflow-y-auto py-2">
              {pages.map(([p, value]) => (
                <button key={p} type="button" onClick={() => go(p)} className={row}>
                  {PAGE_TITLES[p]}
                  <span className={`${note} flex min-w-0 items-center gap-1`}>
                    <span className="truncate">{value}</span>
                    <span className="-mr-1.5 shrink-0 text-soft">
                      <Chevron />
                    </span>
                  </span>
                </button>
              ))}
            </div>
            <div className="flex flex-wrap items-center gap-1 px-5 py-2" aria-label="Libri version and updates">
              <span className="mr-auto text-label text-mid">Libri{host?.version ? ` v${host.version}` : ""}</span>
              {updateButton}
            </div>
          </nav>
          <div
            role="region"
            aria-label={PAGE_TITLES[shown]}
            inert={!page}
            aria-hidden={!page}
            className={`absolute inset-0 overflow-y-auto py-2 transition-transform duration-200 ease-out motion-reduce:transition-none ${page ? "visible translate-x-0" : "invisible pointer-events-none translate-x-full"}`}
          >
            {body[shown]()}
          </div>
        </div>
    </aside>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="py-2">
      <h2 className="px-5 pb-1 text-caption font-medium tracking-[0.06em] text-mid uppercase">{title}</h2>
      {children}
    </section>
  );
}

/** A setting that is read, not set: its name and its value. */
function Fact({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className="flex min-h-11 w-full items-center justify-between gap-6 px-5 py-2.5 text-body">
      {/* The name keeps one line and the value gives way: at a phone's
          width a long address wrapped "Listens on" onto two. */}
      <span className="shrink-0 whitespace-nowrap">{label}</span>
      <span title={title} className="min-w-0 truncate text-label text-mid tabular-nums">
        {value}
      </span>
    </div>
  );
}

/** This machine as /api/host (site.rs `host`) tells it; the addresses and
 * URL come only to a signed-in page. */

/** The machine this page talks to, asked every 15 seconds while `want`
 * holds, so its running time stays fresh and its silence shows: `seen` is
 * the answer's time in ms, or down. What it last said is kept while down. */
function useHost(want: boolean) {
  const [host, setHost] = useState<HostInfo | null>(null);
  const [seen, setSeen] = useState<number | "down">();
  useEffect(() => {
    if (!want) return;
    const ask = async () => {
      const start = performance.now();
      const r = await api("api/host", { cache: "no-store", signal: AbortSignal.timeout(5000) }).catch(() => null);
      if (!r?.ok) return setSeen("down");
      const info: HostInfo = await r.json();
      setHost(info);
      setSeen(Math.round(performance.now() - start));
    };
    void ask();
    const every = setInterval(ask, 15000);
    return () => clearInterval(every);
  }, [want]);
  return { host, seen };
}


/** Seconds as the largest whole unit: "40 seconds", "3 hours", "2 days". */
function lasted(s: number) {
  const [n, unit] = s >= 86400 ? [Math.floor(s / 86400), "day"] : s >= 3600 ? [Math.floor(s / 3600), "hour"] : s >= 60 ? [Math.floor(s / 60), "minute"] : [s, "second"];
  return `${n} ${unit}${n === 1 ? "" : "s"}`;
}

/** A model on offer and the harness that runs it (catalog.rs `Offer`). */
type Offer = { harness: string; model: string; name: string; provider: string; efforts: string[]; default: boolean; usage?: string | null };
type Choice = { harness: string; model: string };
/** A permission mode or sandbox setting, in the harness's own terms (access.rs). */
type Mode = { id: string; name: string; note: string; open: boolean; hidden?: boolean };
type HarnessModes = { harness: string; access: Mode[]; sandbox: Mode[]; defaultAccess: string; defaultSandbox: string | null };
/** `modes` is missing on a host from before the harnesses' own modes. */
type Models = { pick: Choice; effort: string; temp: { pick: Choice; effort: string; access: string; sandbox: string | null }; access: string; sandbox: string | null; modes?: HarnessModes[]; others: string | null; othersOffered: string[]; signedOut?: string[]; harnesses?: HarnessSetting[]; defaultHarness?: string; offers: Offer[] };
// One harness in Settings: on or off, its default model (null for its own), how to sign it in.
type HarnessSetting = { harness: string; on: boolean; model: string | null; remedy: string };

/** How the harnesses' own makers name them. */
const HARNESS_NAMES: Record<string, string> = { claude: "Claude Code", codex: "Codex", muse: "Muse Code", opencode: "OpenCode", omp: "omp", pi: "Pi", grok: "Grok Build" };

/** How the providers name themselves, by the harnesses' ids for them. */
const PROVIDER_NAMES: Record<string, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  "opencode-go": "OpenCode Go",
  "opencode-zen": "OpenCode Zen",
  opencode: "OpenCode Zen",
  deepseek: "DeepSeek",
  abliteration: "Abliteration",
  meta: "Meta",
  local: "Local",
};

const key = (p: Choice) => `${p.harness} ${p.model}`;

// The models on offer and the pick, asked once and shared by the pickers
// under the composer and the panel, so a change in one shows in the other.
let MODELS: Models | null = null;
const modelWatchers = new Set<(m: Models) => void>();

function askModels() {
  return api("api/models")
    .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
    .then(
      (m: Models) => {
        MODELS = m;
        publishOffers(m.offers);
        for (const w of modelWatchers) w(m);
      },
      () => {},
    );
}

/** The models, asked again whenever `want` turns true; `post` makes a
 * change and asks again, `busy` while it runs. */
function useModels(want: boolean) {
  const [m, setM] = useState(MODELS);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    modelWatchers.add(setM);
    picksChanged.add(askModels);
    return () => {
      modelWatchers.delete(setM);
      picksChanged.delete(askModels);
    };
  }, []);
  useEffect(() => {
    if (want) askModels();
  }, [want]);
  const post = (path: string, body: object) => {
    setBusy(true);
    return api(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) })
      .then((r) => r.ok || Promise.reject(r.status))
      .catch(() => {})
      .then(askModels)
      .finally(() => setBusy(false));
  };
  return { m, busy, post };
}

// assistant-ui's composer takes the focus on a mouse down anywhere in it but
// a control of its own list, and focus leaving a menu shuts it: without this
// a click on a row of a role="listbox" menu never lands.
const keepMenuOpen = (e: React.MouseEvent) => e.preventDefault();

/** The model and its thinking as one line of quiet words, "Opus 5.5 High",
 * as T3 Code sets them: each word opens its own menu, and only the hover
 * fill shows where one ends. */
const WORD = "rounded-md px-1";

/** A quiet pill that opens a short menu above it, drawn as the model menu
 * is; arrows move, Enter picks, Escape shuts. */
type Option = { value: string; label: string; note?: string };
function Pill({ label, title, icon, words = "", rows = "", word = false, value, options, disabled, pick }: { label: string; title: string; icon?: React.ReactNode; words?: string; rows?: string; word?: boolean; value: string; options: Option[]; disabled: boolean; pick: (v: string) => void }) {
  const [open, setOpen, ref] = useMenu();
  const menu = useMenuInRow(ref, open);
  const [active, setActive] = useState(0);
  useEffect(() => {
    if (open) setActive(Math.max(0, options.findIndex((o) => o.value === value)));
  }, [open]);
  const take = (o?: Option) => {
    if (!o) return;
    setOpen(false);
    if (o.value !== value) pick(o.value);
  };
  const keys = (e: React.KeyboardEvent) => {
    if (!open) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const step = e.key === "ArrowDown" ? 1 : -1;
      setActive((a) => (a + step + options.length) % options.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      take(options[active]);
    }
  };
  return (
    <div ref={ref} className={`relative min-w-0 ${word ? "shrink-0" : "@max-xl:shrink-0"}`} onKeyDown={keys}>
      <button
        type="button"
        title={title}
        aria-label={title}
        aria-haspopup="listbox"
        aria-expanded={open}
        disabled={disabled}
        onClick={() => setOpen(!open)}
        className={`flex h-8 max-w-full min-w-0 items-center text-label hover:bg-hover hover:text-ink disabled:opacity-50 ${word ? WORD : "gap-1 rounded-full px-2.5 @max-xl:gap-0.5 @max-xl:px-1.5"} ${open ? "bg-hover text-ink" : "text-mid"}`}
      >
        {icon}
        <span className={`truncate ${words}`}>{label}</span>
        {/* An icon pill under a mouse gives its chevron's room to the model's name; a finger keeps the sign it opens. */}
        {!word && (
          <svg viewBox="0 0 16 16" className="size-3 shrink-0 @max-xl:pointer-fine:hidden" fill="none" stroke="currentColor" strokeWidth="1.6" aria-hidden>
            <path d="M4 6l4 4 4-4" />
          </svg>
        )}
      </button>
      {open && (
        <div ref={menu} role="listbox" aria-label={title} onMouseDown={keepMenuOpen} className="absolute bottom-full left-0 z-(--layer-pop) mb-2 w-56 max-w-[100cqw] overflow-hidden rounded-xl border-[1.25px] border-edge bg-lift py-1 shadow-lift">
          {options.map((o, i) => {
            const picked = o.value === value;
            return (
              <div
                key={o.value}
                role="option"
                aria-selected={picked}
                onPointerMove={() => setActive(i)}
                onClick={() => take(o)}
                className={`flex cursor-pointer items-center gap-2 px-4 py-1.5 text-control text-ink ${i === active ? "bg-hover" : ""}`}
              >
                <span className="min-w-0 flex-1">
                  <span className={`block ${rows}`}>{o.label}</span>
                  {o.note && <span className="block text-caption leading-snug text-mid">{o.note}</span>}
                </span>
                {picked && (
                  <svg viewBox="0 0 16 16" className="size-3.5 shrink-0 text-mid" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden>
                    <path d="M3.5 8.5l3 3 6-7" />
                  </svg>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

/** A worker's own model and thinking, which start as the main chat's, and
 * its access, which starts on its harness's default; each stays its own
 * once picked. */
function useWorkerChoice(name?: string, at?: string) {
  const [choice, setChoice] = useState<{ pick: Choice; effort: string; access: string; sandbox: string | null }>();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string>();
  const path = at ?? `api/work/${encodeURIComponent(name ?? "")}/choice`;
  useEffect(() => {
    if (!name) return;
    setFailed(undefined);
    let live = true;
    api(path)
      .then((r) => (r.ok ? r.json() : undefined))
      .then((c) => live && c && setChoice(c))
      .catch(() => {});
    return () => void (live = false);
  }, [name, path]);
  const post = (change: object) => {
    setBusy(true);
    setFailed(undefined);
    api(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(change) })
      .then(async (r) => (r.ok ? setChoice(await r.json()) : setFailed((await r.text()) || "That didn't change.")))
      .catch(() => setFailed("That didn't reach the host."))
      .finally(() => setBusy(false));
  };
  return { choice: name ? choice : undefined, busy, failed, post };
}

type WorkerChoice = ReturnType<typeof useWorkerChoice>;

/** What a worker started from the Tasks sheet will run on: the Workers
 * default (api/work-choice) until the sheet's pickers change it, held here
 * and sent with the brief. Another harness has other modes, so a move to one
 * leaves its own defaults to stand. `changed` says whether to send it at all. */
function useNewWorkerChoice() {
  type Held = NonNullable<WorkerChoice["choice"]>;
  const [choice, setChoice] = useState<Held>();
  const [changed, setChanged] = useState(false);
  useEffect(() => {
    let live = true;
    api("api/work-choice")
      .then((r) => (r.ok ? r.json() : undefined))
      .then((c) => live && c && setChoice(c))
      .catch(() => {});
    return () => void (live = false);
  }, []);
  const post = (change: object) => {
    const c = change as Partial<Held>;
    setChanged(true);
    setChoice((was) => {
      if (!was) return was;
      const moved = !!c.pick && c.pick.harness !== was.pick.harness;
      return { ...was, ...(moved ? { access: "", sandbox: null } : {}), ...c };
    });
  };
  const own: WorkerChoice = { choice, busy: false, failed: undefined, post };
  return { own, sent: changed ? choice : undefined };
}

/** Tasks' +: a new worker's brief, written in the chat's own composer (the
 * worker one, AgentComposer) with its files, model, thinking, access and mic.
 * On a phone a whole screen, a close at the top left and the words sitting
 * on the keyboard under a heading and a line of help, as Android's
 * TaskSheet.kt and Claude's Tasks sheet; on a wide screen the same in a
 * window over the page. Escape closes it; the words stay as a draft. */
function TaskSheet({ wide, close, start }: { wide: boolean; close: () => void; start: (brief: string, files: File[], choice: NonNullable<WorkerChoice["choice"]> | undefined) => Promise<boolean> }) {
  const { own, sent } = useNewWorkerChoice();
  // On a phone the sheet stops at the keyboard: Chrome shrinks the page for
  // it, Safari and Firefox only the visual viewport (terminal.tsx).
  const [room, setRoom] = useState<{ top: number; height: number }>();
  useEffect(() => {
    const view = window.visualViewport;
    if (wide || !view) return;
    const follow = () => setRoom({ top: view.offsetTop, height: view.height });
    follow();
    view.addEventListener("resize", follow);
    view.addEventListener("scroll", follow);
    return () => {
      view.removeEventListener("resize", follow);
      view.removeEventListener("scroll", follow);
    };
  }, [wide]);
  useEffect(() => {
    const key = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [close]);
  const shut = (
    <button type="button" onClick={close} aria-label="Close" title="Close" className="grid size-10 shrink-0 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink">
      <svg viewBox="0 0 24 24" className="size-5" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" aria-hidden>
        <path d="M18 6 6 18M6 6l12 12" />
      </svg>
    </button>
  );
  const words = (
    <div className="px-[22px] pb-3">
      <h2 className="font-serif text-title text-ink">New task</h2>
      <p className="mt-1 text-label text-mid">A worker takes it from here and reports to its card in Tasks.</p>
    </div>
  );
  const composer = (
    <LoneComposer
      setup={{ draft: forHost("draft:task-sheet"), placeholder: "What should a new worker do?", label: "Message a new worker", own, autoFocus: true }}
      send={async (text, files) => {
        const ok = await start(text, files, sent);
        if (ok) close();
        return ok;
      }}
    />
  );
  if (wide)
    return (
      <div className="fixed inset-0 z-50 grid place-items-center bg-black/25 p-6" onClick={close}>
        <div role="dialog" aria-modal aria-label="New task" onClick={(e) => e.stopPropagation()} className="flex w-full max-w-[640px] flex-col rounded-2xl border-[1.25px] border-edge bg-paper pt-2 pb-2 shadow-lift">
          <div className="px-2">{shut}</div>
          <div className="pt-6">{words}</div>
          <div className="px-3">{composer}</div>
        </div>
      </div>
    );
  return (
    <div
      role="dialog"
      aria-modal
      aria-label="New task"
      style={room ? { top: room.top, height: room.height } : undefined}
      className="fixed inset-x-0 top-0 z-50 flex h-dvh flex-col bg-paper pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)]"
    >
      <div className="flex h-14 shrink-0 items-center px-2">{shut}</div>
      {/* The words sit on the keyboard, the heading just above them. */}
      <div className="min-h-0 flex-1" />
      {words}
      <div className="px-3.5">{composer}</div>
    </div>
  );
}

/** The model the next turn runs on, picked from every harness's own list,
 * and its thinking, under the composer as T3 Code and Paseo put them; the
 * harness follows from the model. `temp` picks for Temp, apart from the
 * main chat. */
function ModelPickers({ temp = false, own }: { temp?: boolean; own?: WorkerChoice }) {
  const me = useMe();
  const { m, busy, post } = useModels(!!me?.models);
  if (!me?.models || !m || !m.offers.length || (own && !own.choice)) return null;
  const chosen = own?.choice ?? (temp ? m.temp : m);
  // A pick stored as an alias ("opus") shows as the model it stands for.
  const now = offerOf(chosen.pick.harness, chosen.pick.model, m.offers);
  const efforts = now?.efforts ?? [];
  const effort = efforts.includes(chosen.effort) ? chosen.effort : "";
  // Each harness's own modes, so a thread shows those of the one it is on.
  const modes = m.modes?.find((h) => h.harness === chosen.pick.harness);
  const access = own?.choice?.access ?? (temp ? m.temp.access : m.access);
  const sandbox = own?.choice?.sandbox ?? (temp ? m.temp.sandbox : m.sandbox);
  const mode = modes?.access.find((o) => o.id === access);
  const box = modes?.sandbox.find((o) => o.id === sandbox);
  // Claude Code keeps its sandbox behind /sandbox: its pill shows only while
  // one holds, since auto-allow changes what runs unasked.
  const boxed = modes?.sandbox.some((o) => !o.hidden) || (!!box && !box.open);
  return (
    <div className="flex min-w-0 items-center">
      <div className="flex min-w-0 items-center pl-1.5">
      <ModelMenu m={m} chosen={chosen.pick} now={now} busy={busy || !!own?.busy} pick={(o) => (own ? own.post({ pick: { harness: o.harness, model: o.model } }) : post("api/model", { harness: o.harness, model: o.model, temp }))} />
      {efforts.length > 0 && (
        <Pill
          label={effort || "Default"}
          title="Thinking"
          word
          words="capitalize"
          rows="capitalize"
          value={effort}
          disabled={busy || !!own?.busy}
          // Empty is the harness's own default, always there to go back to.
          options={[{ value: "", label: "Default" }, ...efforts.map((e) => ({ value: e, label: e }))]}
          pick={(effort) => (own ? own.post({ effort }) : post("api/effort", { effort, temp }))}
        />
      )}
      </div>
      {modes && (
        <Pill
          label={mode?.name ?? access}
          title="Access"
          icon={<Shield full={!!mode?.open} />}
          words="@max-xl:hidden"
          value={access}
          disabled={busy || !!own?.busy}
          // A hidden mode stays listed while it is the one in force.
          options={modes.access.filter((o) => !o.hidden || o.id === access).map((o) => ({ value: o.id, label: o.name, note: o.note }))}
          pick={(access) => (own ? own.post({ access }) : post("api/access", { access, temp }))}
        />
      )}
      {modes && boxed && (
        <Pill
          label={box?.name ?? sandbox ?? ""}
          title="Sandbox"
          icon={<Sandbox open={!!box?.open} />}
          words="@max-xl:hidden"
          value={sandbox ?? ""}
          disabled={busy || !!own?.busy}
          options={modes.sandbox.map((o) => ({ value: o.id, label: o.name, note: o.note }))}
          pick={(sandbox) => (own ? own.post({ sandbox }) : post("api/access", { sandbox, temp }))}
        />
      )}
    </div>
  );
}

/** Which memory a chat tile is on, and how to put it on another: the
 * tile's own, so a workspace keeps each tile on its memory. None outside a
 * chat tile, where no memory is picked. */
const MemoryPickCtx = createContext<{ id: string | null; pick: (id: string | null) => void } | null>(null);

/** `/memory [name]` in the main chat, as the tile's tabs do: names this
 * chat's memory, or opens the named one's tab in this tile (focuses it, if
 * open). The latest main-chat tile shown takes it. */
function useMemoryCommand() {
  const at = useContext(MemoryPickCtx);
  const memories = useMemories();
  const latest = useRef({ at, memories });
  latest.current = { at, memories };
  useEffect(() => {
    const run = (arg: string, note: (said: string) => void) => {
      const { at, memories } = latest.current;
      const named = memories ?? [];
      const choices = named.length ? ` /memory ${named.map((m) => m.name.toLowerCase()).join(", /memory ")} opens that memory's tab here.` : " No other memory is set up.";
      const want = arg.toLowerCase();
      if (!want) return note(`${at?.id ? "This chat is on its own memory." : "This chat is on Main."}${choices}`);
      if (want === "main") return at ? at.pick(null) : note(`This chat is on Main.${choices}`);
      const m = named.find((m) => m.id === want || m.name.toLowerCase() === want);
      if (!m) return note(`No memory ${arg}.${choices}`);
      if (!at) return note(`Open ${m.name} from the + beside a chat's tabs.`);
      at.pick(m.id);
    };
    tileCommands.set("memory", run);
    return () => {
      if (tileCommands.get("memory") === run) tileCommands.delete("memory");
    };
  }, []);
}

/** A box, as the shield: filled when nothing holds the harness in. */
function Sandbox({ open }: { open: boolean }) {
  return (
    <svg viewBox="0 0 16 16" className="size-4 shrink-0" fill={open ? "currentColor" : "none"} stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" aria-hidden>
      <path d="M2.5 5 8 2l5.5 3v6L8 14l-5.5-3zM2.5 5 8 8l5.5-3M8 8v6" />
    </svg>
  );
}

/** A shield, empty while something asks and filled when nothing does. */
function Shield({ full }: { full: boolean }) {
  return (
    <svg viewBox="0 0 16 16" className="size-4 shrink-0" fill={full ? "currentColor" : "none"} stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" aria-hidden>
      <path d="M8 1.8l5 1.9v4c0 3.1-2.1 5.5-5 6.5-2.9-1-5-3.4-5-6.5v-4z" />
    </svg>
  );
}

/** Offers grouped by who serves them, in the order the harnesses listed them. */
function byProvider(offers: Offer[]) {
  const groups = new Map<string, Offer[]>();
  for (const o of offers) groups.set(o.provider, [...(groups.get(o.provider) ?? []), o]);
  return groups;
}

/** The model pill: a menu with a search box over the list, as T3 Code's
 * picker, since a hundred models in a native select is a long scroll. Every
 * word typed must appear in the model's name, id or provider; arrows move,
 * Enter picks. */
function ModelMenu({ m, chosen, now, busy, pick }: { m: Models; chosen: Choice; now?: Offer; busy: boolean; pick: (o: Offer) => void }) {
  const [open, setOpen, ref] = useMenu();
  const menu = useMenuInRow(ref, open);
  const [query, setQuery] = useState("");
  // No logo known, no room kept for one.
  const mark = now && logoOf(now);
  const [active, setActive] = useState(0);
  const list = useRef<HTMLDivElement>(null);
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const shown = m.offers.filter((o) => {
    const text = `${o.name} ${o.model} ${o.provider} ${PROVIDER_NAMES[o.provider] ?? ""}`.toLowerCase();
    return words.every((w) => text.includes(w));
  });
  const groups = byProvider(shown);
  const flat = [...groups.values()].flat();
  useEffect(() => {
    if (!open) return;
    setQuery("");
    const i = [...byProvider(m.offers).values()].flat().findIndex((o) => key(o) === key(chosen));
    setActive(Math.max(0, i));
  }, [open]);
  useEffect(() => {
    list.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active, open]);
  const take = (o?: Offer) => {
    if (!o) return;
    setOpen(false);
    if (key(o) !== key(chosen)) pick(o);
  };
  const keys = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const step = e.key === "ArrowDown" ? 1 : -1;
      setActive((a) => (flat.length ? (a + step + flat.length) % flat.length : 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      take(flat[active]);
    }
  };
  let index = 0;
  return (
    <div ref={ref} className="relative min-w-0">
      <button
        type="button"
        title={`Model, run in ${HARNESS_NAMES[chosen.harness] ?? chosen.harness}`}
        aria-haspopup="listbox"
        aria-expanded={open}
        disabled={busy}
        onClick={() => setOpen(!open)}
        className={`flex h-8 max-w-full min-w-0 items-center gap-1.5 text-label hover:bg-hover hover:text-ink disabled:opacity-50 ${WORD} ${open ? "bg-hover text-ink" : "text-mid"}`}
      >
        {mark && <Logo name={mark} />}
        <span className="truncate">{now?.name ?? (chosen.model || "Default")}</span>
      </button>
      {open && (
        <div ref={menu} className="absolute bottom-full left-0 z-(--layer-pop) mb-2 flex max-h-[min(28rem,60dvh)] w-80 max-w-[100cqw] flex-col overflow-hidden rounded-xl border-[1.25px] border-edge bg-lift shadow-lift">
          <input
            autoFocus
            type="search"
            role="combobox"
            aria-label="Search models"
            aria-expanded
            aria-controls="model-list"
            aria-activedescendant={flat[active] ? `model-${active}` : undefined}
            placeholder="Search models"
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
            onKeyDown={keys}
            className="border-b-[1.25px] border-edge bg-transparent px-4 py-2.5 text-body text-ink outline-none placeholder:text-mid"
          />
          <div ref={list} id="model-list" role="listbox" aria-label="Models" onMouseDown={keepMenuOpen} className="overflow-y-auto overscroll-contain py-1">
            {!flat.length && <div className="px-4 py-2 text-control text-mid">No model matches.</div>}
            {[...groups].map(([provider, offers]) => (
              <div key={provider} role="group" aria-label={PROVIDER_NAMES[provider] ?? provider}>
                <div className="px-4 pt-2 pb-1 text-caption font-medium tracking-wide text-mid uppercase">{PROVIDER_NAMES[provider] ?? provider}</div>
                {offers.map((o) => {
                  const i = index++;
                  const picked = key(o) === key(chosen);
                  return (
                    <div
                      key={key(o)}
                      id={`model-${i}`}
                      data-index={i}
                      role="option"
                      aria-selected={picked}
                      onPointerMove={() => setActive(i)}
                      onClick={() => take(o)}
                      className={`flex cursor-pointer items-center gap-2 px-4 py-1.5 text-control text-ink ${i === active ? "bg-hover" : ""}`}
                    >
                      <Logo name={logoOf(o)} />
                      <span className="min-w-0 flex-1 truncate">{o.name}</span>
                      {picked && (
                        <svg viewBox="0 0 16 16" className="size-3.5 shrink-0 text-mid" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden>
                          <path d="M3.5 8.5l3 3 6-7" />
                        </svg>
                      )}
                    </div>
                  );
                })}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

/** What makes what besides answers, as /api/capabilities (site.rs)
 * tells it: pictures by a model picked among those this host reaches, the
 * rest by the one service that does them. */
type Maker = { provider: string; model: string; name: string };
type Makers = { image: { picked: Maker | null; choices: Maker[] }; fixed: { capability: string; provider: string; name: string }[] };

/** The makers, asked each time `want` turns true; `pick` sets the picture
 * model and asks again. */
function useMakers(want: boolean) {
  const [makers, setMakers] = useState<Makers | null>(null);
  const ask = () =>
    api("api/capabilities")
      .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
      .then(setMakers, () => {});
  useEffect(() => {
    if (want) ask();
  }, [want]);
  const pick = (c: Maker) =>
    api("api/capabilities", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ capability: "image", ...c }) })
      .catch(() => {})
      .then(ask);
  return { makers, pick };
}

/** The model that writes workers' titles and summaries: the cheapest the
 * host can run, or any on offer, asked each time `want` turns true. The host
 * holds the choice, so every device shows the same one. `now` stays null on
 * a host without the choice (404). A pick the host refuses leaves `now` as it
 * was and `said` gives the reason. */
type SynopsisNow = { picked: Choice | null; default: Choice | null };

/** The names new workers are given, as names.rs keeps them. The host holds
 * the list, so every device shows the same one. `now` stays null on a host
 * without it (404). A change the host refuses leaves `now` as it was and
 * `said` gives the reason. */
type WorkerNamesNow = { builtin: string[]; added: string[]; own: string[]; use_own: boolean; excluded: string[]; allowed: number; left_out: string[] };

function GuideEditor({ guide: { now, said, busy, change }, note, title, label, about }: { guide: ReturnType<typeof useSetting<{ text: string }>>; note: string; title: string; label: string; about: string }) {
  // The text as being edited; null until a keystroke, so the box shows what
  // the host holds.
  const [typed, setTyped] = useState<string | null>(null);
  if (!now) return null;
  const write = async () => {
    if (await change({ text: typed ?? "" })) setTyped(null);
  };
  return (
    <Section title={title}>
      <p className={`${note} px-5 pb-2`}>{about}</p>
      {said && (
        <p role="alert" className="px-5 pb-2 text-label text-accent">
          {said}
        </p>
      )}
      <div className="flex flex-col gap-2 px-5 py-1.5">
        <textarea value={typed ?? now.text} onChange={(e) => setTyped(e.target.value)} rows={16} aria-label={label} className="min-w-0 rounded-lg border border-frame bg-paper px-3 py-2 text-body outline-none focus:border-soft" />
        <button type="button" onClick={() => void write()} disabled={busy || typed === null} className="self-start rounded-lg border-[1.25px] border-edge px-3 py-2 text-body hover:bg-hover disabled:opacity-40 disabled:hover:bg-transparent">
          Save
        </button>
      </div>
    </Section>
  );
}

/** A name that a tap excludes or brings back. */
function NameChip({ name, off, busy, tap }: { name: string; off: boolean; busy: boolean; tap: () => void }) {
  return (
    <button type="button" aria-pressed={!off} title={off ? `Bring ${name} back` : `Exclude ${name}`} disabled={busy} onClick={tap} className={`min-h-8 rounded-full border-[1.25px] border-frame px-2.5 py-1 text-label hover:bg-hover ${off ? "text-mid line-through" : ""}`}>
      {name}
    </button>
  );
}

function WorkerNames({ names: { now, said, busy, change }, row, note }: { names: ReturnType<typeof useSetting<WorkerNamesNow>>; row: string; note: string }) {
  const [typed, setTyped] = useState("");
  // Their own list as being edited; null until a keystroke, so the box
  // shows what the host holds.
  const [pasted, setPasted] = useState<string | null>(null);
  if (!now) return null;
  const off = new Set(now.excluded.map((n) => n.toLowerCase()));
  const excluded = (n: string) => off.has(n.toLowerCase());
  const sway = (n: string) => void change({ change: excluded(n) ? "restore" : "exclude", name: n });
  const add = async () => {
    if (await change({ change: "add", name: typed })) setTyped("");
  };
  const save = async () => {
    if (await change({ change: "set_own", text: pasted ?? "" })) setPasted(null);
  };
  const find = typed.trim().toLowerCase();
  const found = (n: string) => n.toLowerCase().includes(find);
  const chips = "flex flex-wrap gap-1.5 px-5 pt-1 pb-2";
  const field = "min-w-0 rounded-lg border border-frame bg-paper px-3 py-2 text-body outline-none focus:border-soft";
  const button = "shrink-0 rounded-lg border-[1.25px] border-edge px-3 py-2 text-body hover:bg-hover disabled:opacity-40 disabled:hover:bg-transparent";
  const heading = "px-5 pt-3 text-label text-mid";
  const added = now.added.filter(found);
  const builtin = now.builtin.filter(found);
  return (
    <Section title="Names">
      <button type="button" role="switch" onClick={() => void change({ change: "use_own", on: !now.use_own })} disabled={busy} aria-checked={now.use_own} className={row}>
        Use my own list
        <Toggle on={now.use_own} />
      </button>
      <p className={`${note} px-5 pb-2`}>{now.use_own ? "Your list replaces the built-in names." : "Tap a name to leave it out or bring it back."}</p>
      {now.allowed === 0 && <p className={`${note} px-5 pb-2`}>No names left; new workers are numbered.</p>}
      {said && (
        <p role="alert" className="px-5 pb-2 text-label text-accent">
          {said}
        </p>
      )}
      {now.use_own ? (
        <>
          <div className="flex flex-col items-start gap-2 px-5 pb-1">
            <textarea value={pasted ?? now.own.join("\n")} onChange={(e) => setPasted(e.target.value)} rows={5} placeholder="Names, with commas or one per line" aria-label="Your list of names" className={`${field} w-full resize-y`} />
            <button type="button" onClick={() => void save()} disabled={busy || pasted === null} className={button}>
              Save list
            </button>
          </div>
          {now.left_out.length > 0 && <p className={`${note} px-5 pt-1`}>Left out (one word, letters only): {now.left_out.join(", ")}.</p>}
          {now.own.length > 0 && (
            <>
              <h3 className={heading}>Your list · {now.own.length}</h3>
              <div className={chips}>
                {now.own.map((n) => (
                  <NameChip key={n} name={n} off={excluded(n)} busy={busy} tap={() => sway(n)} />
                ))}
              </div>
            </>
          )}
        </>
      ) : (
        <>
          <form
            className="flex gap-2 px-5 pb-1"
            onSubmit={(e) => {
              e.preventDefault();
              void add();
            }}
          >
            <input value={typed} onChange={(e) => setTyped(e.target.value)} autoComplete="off" autoCapitalize="words" spellCheck={false} placeholder="Find or add a name" aria-label="Find or add a name" className={`${field} flex-1`} />
            <button disabled={busy || !find} className={button}>
              Add
            </button>
          </form>
          {added.length > 0 && (
            <>
              <h3 className={heading}>Added · {added.length}</h3>
              <div className={chips}>
                {added.map((n) => (
                  <span key={n} className={`inline-flex min-h-8 items-center gap-0.5 rounded-full border-[1.25px] border-frame py-0.5 pr-1 pl-2.5 text-label ${excluded(n) ? "text-mid line-through" : ""}`}>
                    {n}
                    <button type="button" disabled={busy} aria-label={`Remove ${n}`} title={`Remove ${n}`} onClick={() => void change({ change: "remove", name: n })} className="grid size-6 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink">
                      <svg viewBox="0 0 24 24" className="size-3.5" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" aria-hidden>
                        <path d="M18 6 6 18M6 6l12 12" />
                      </svg>
                    </button>
                  </span>
                ))}
              </div>
            </>
          )}
          <h3 className={heading}>Built-in · {builtin.length}</h3>
          <div className={chips}>
            {builtin.map((n) => (
              <NameChip key={n} name={n} off={excluded(n)} busy={busy} tap={() => sway(n)} />
            ))}
          </div>
        </>
      )}
    </Section>
  );
}

function SynopsisModel({ m, synopsis: { now, said, busy, change }, className }: { m: { offers: Offer[] }; synopsis: ReturnType<typeof useSetting<SynopsisNow>>; className: string }) {
  const key = (o: Choice) => `${o.harness}/${o.model}`;
  const named = (p: Choice) => m.offers.find((o) => key(o) === key(p))?.name ?? p.model;
  const picked = now?.picked;
  return (
    <>
      {now && (
        <div className="flex w-full items-center justify-between gap-3 px-5 py-2.5 text-body">
          <label htmlFor="synopsis-model">Worker titles</label>
          <select id="synopsis-model" disabled={busy} value={picked ? key(picked) : ""} onChange={(e) => void change(modelSetting(m.offers.find((o) => key(o) === e.target.value)))} className={className}>
            <option value="">Default{now.default ? ` (${named(now.default)})` : ""}</option>
            {/* Still shown once it is off the list: it stays the pick. */}
            {picked && !m.offers.some((o) => key(o) === key(picked)) && <option value={key(picked)}>{picked.model}</option>}
            {[...new Set(m.offers.map((o) => o.provider))].map((p) => (
              <optgroup key={p} label={PROVIDER_NAMES[p] ?? p}>
                {m.offers
                  .filter((o) => o.provider === p)
                  .map((o) => (
                    <option key={key(o)} value={key(o)}>
                      {o.name}
                    </option>
                  ))}
              </optgroup>
            ))}
          </select>
        </div>
      )}
      {said && (
        <p role="alert" className="px-5 pb-1 text-label text-accent">
          {said}
        </p>
      )}
    </>
  );
}

/** The model new workers start on, asked each time `want` turns true: the
 * host holds it, so every device shows the same one. `now` stays null on a
 * host without the choice (404). A pick the host refuses leaves `now` as it
 * was and `said` gives the reason. */
type WorkerModel = { picked: { harness: string; model: string } | null };

/** The default model for each kind of making: answers (the same pick as
 * under the composer), new workers, pictures, and the fixed ones with who
 * makes them. */
/** omp owns both the login and its credential store. */
function OmpSignin() {
  const [provider, setProvider] = useState("openai-codex-device");
  const [status, setStatus] = useState<{ state: string; url?: string; code?: string; error?: string } | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (status?.state !== "waiting") return;
    const timer = setInterval(() => {
      void api("api/signin/progress").then(async (r) => {
        if (!r.ok) return;
        const next = (await r.json()).omp;
        if (next && next.code === status.code) { setStatus(next); if (next.state === "done") void askModels(); }
      }).catch(() => {});
    }, 3000);
    return () => clearInterval(timer);
  }, [status?.state, status?.code]);
  const start = async () => {
    setBusy(true);
    try {
      const r = await api("api/signin", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ harness: "omp", provider }) });
      if (!r.ok) throw new Error(await r.text());
      setStatus({ state: "waiting", ...await r.json() });
    } catch (e) { setStatus({ state: "failed", error: String(e) }); }
    finally { setBusy(false); }
  };
  return <Section title="omp accounts">
    <div className="flex flex-wrap items-center justify-between gap-3 px-5 py-2.5">
      <label htmlFor="omp-login-provider">Sign in with</label>
      <select id="omp-login-provider" disabled={busy || status?.state === "waiting"} value={provider} onChange={(e) => setProvider(e.target.value)} className="min-w-0 bg-transparent text-label">
        <option value="openai-codex-device">ChatGPT / Codex</option>
        <option value="kimi-code">Kimi Code</option>
        <option value="kilo">Kilo</option>
      </select>
      <button type="button" disabled={busy || status?.state === "waiting"} onClick={() => void start()} className="rounded-lg border border-hair px-3 py-1.5 text-label disabled:opacity-40">{busy ? "Starting…" : "Sign in"}</button>
    </div>
    {status?.state === "waiting" && <div role="status" className="space-y-2 px-5 py-2 text-label">
      <p>Open the provider’s page and enter <strong className="font-mono select-all break-all">{status.code}</strong>.</p>
      <a className="text-active underline" href={status.url} target="_blank" rel="noreferrer">Continue sign-in</a>
      <p>Waiting for omp to finish signing in…</p>
    </div>}
    {status?.state === "done" && <p role="status" className="px-5 py-2 text-label">Signed in. Models have been refreshed.</p>}
    {status?.state === "failed" && <p role="alert" className="px-5 py-2 text-label text-accent">{status.error}</p>}
    <p className="px-5 pb-3 text-label text-mid">For other providers, run <code>omp login</code> on this host.</p>
  </Section>;
}

function Models({ want, makers: { makers, pick } }: { want: boolean; makers: ReturnType<typeof useMakers> }) {
  const { m, busy, post } = useModels(want);
  const workers = useSetting<WorkerModel>("api/worker-model", want);
  const synopsis = useSetting<SynopsisNow>("api/synopsis", want);
  const select = "max-w-48 truncate bg-transparent text-right text-label text-mid outline-none";
  const key = (o: { harness?: string; provider?: string; model: string }) => `${o.harness ?? o.provider}/${o.model}`;
  const image = makers?.image;
  const held = workers.now?.picked;
  return (
    <>
      <OmpSignin />
      <Section title="Defaults">
        {m && m.offers.length > 0 && (
          <div className="flex w-full items-center justify-between gap-3 px-5 py-2.5 text-body">
            <label htmlFor="chat-model">Chat</label>
            <select id="chat-model" disabled={busy} value={key(m.pick)} onChange={(e) => { const o = m.offers.find((o) => key(o) === e.target.value); if (o) post("api/model", { harness: o.harness, model: o.model }); }} className={select}>
              {[...new Set(m.offers.map((o) => o.provider))].map((p) => (
                <optgroup key={p} label={PROVIDER_NAMES[p] ?? p}>
                  {m.offers
                    .filter((o) => o.provider === p)
                    .map((o) => (
                      <option key={key(o)} value={key(o)}>
                        {o.name}
                      </option>
                    ))}
                </optgroup>
              ))}
            </select>
          </div>
        )}
        {m && m.offers.length > 0 && workers.now && (
          <div className="flex w-full items-center justify-between gap-3 px-5 py-2.5 text-body">
            <label htmlFor="worker-model">Workers</label>
            <select id="worker-model" disabled={workers.busy} value={held ? key(held) : ""} onChange={(e) => void workers.change(modelSetting(m.offers.find((o) => key(o) === e.target.value)))} className={select}>
              <option value="">Same as Chat</option>
              {/* Still shown once it is off the list: it stays the pick, and new workers start on Chat's model until it returns. */}
              {held && !m.offers.some((o) => key(o) === key(held)) && <option value={key(held)}>Not offered: {held.model}</option>}
              {[...new Set(m.offers.map((o) => o.provider))].map((p) => (
                <optgroup key={p} label={PROVIDER_NAMES[p] ?? p}>
                  {m.offers
                    .filter((o) => o.provider === p)
                    .map((o) => (
                      <option key={key(o)} value={key(o)}>
                        {o.name}
                      </option>
                    ))}
                </optgroup>
              ))}
            </select>
          </div>
        )}
        {workers.said && (
          <p role="alert" className="px-5 pb-1 text-label text-accent">
            {workers.said}
          </p>
        )}
        {m && m.offers.length > 0 && <SynopsisModel m={m} synopsis={synopsis} className={select} />}
        {image && image.choices.length > 0 && (
          <div className="flex w-full items-center justify-between gap-3 px-5 py-2.5 text-body">
            <label htmlFor="image-model" className="shrink-0">Image generation</label>
            <select id="image-model" value={image.picked ? key(image.picked) : ""} onChange={(e) => { const c = image.choices.find((c) => key(c) === e.target.value); if (c) pick(c); }} className={`min-w-0 ${select}`}>
              {image.choices.map((c) => (
                <option key={key(c)} value={key(c)}>
                  {c.name}
                </option>
              ))}
            </select>
          </div>
        )}
        {makers?.fixed.map((f) => (
          <Fact key={f.capability} label={f.capability} value={f.name} />
        ))}
        <p className="px-5 pb-2 text-label text-mid">The picker under the message box changes Chat too.{workers.now && " Workers keep the model they start on."}</p>
      </Section>
      {!!m?.modes?.length && (
        <Section title="Permissions">
          {m.modes.map((h) => (
            <div key={h.harness}>
              <div className="flex w-full items-center justify-between gap-3 px-5 py-2.5 text-body">
                <label htmlFor={`access-${h.harness}`}>{HARNESS_NAMES[h.harness] ?? h.harness}</label>
                <select id={`access-${h.harness}`} disabled={busy} value={h.defaultAccess} title={h.access.find((o) => o.id === h.defaultAccess)?.note} onChange={(e) => post("api/access/default", { harness: h.harness, access: e.target.value })} className={select}>
                  {h.access.map((o) => (
                    <option key={o.id} value={o.id}>
                      {o.name}
                    </option>
                  ))}
                </select>
              </div>
              {h.sandbox.length > 0 && (
                <div className="flex w-full items-center justify-between gap-3 px-5 py-2.5 text-body">
                  <label htmlFor={`sandbox-${h.harness}`}>{HARNESS_NAMES[h.harness] ?? h.harness} sandbox</label>
                  <select id={`sandbox-${h.harness}`} disabled={busy} value={h.defaultSandbox ?? ""} title={h.sandbox.find((o) => o.id === h.defaultSandbox)?.note} onChange={(e) => post("api/access/default", { harness: h.harness, sandbox: e.target.value })} className={select}>
                    {h.sandbox.map((o) => (
                      <option key={o.id} value={o.id}>
                        {o.name}
                      </option>
                    ))}
                  </select>
                </div>
              )}
            </div>
          ))}
          <p className="px-5 pb-2 text-label text-mid">Where each chat starts; the picker under the message box changes it per chat.</p>
        </Section>
      )}
      {m && ((m.harnesses?.length ?? 0) > 0 || m.othersOffered.length > 1) && (
        <Section title="Harnesses">
          {(m.harnesses ?? []).map((h) => {
            const name = HARNESS_NAMES[h.harness] ?? h.harness;
            const out = h.on && (m.signedOut?.includes(h.harness) ?? false);
            const own = m.offers.filter((o) => o.harness === h.harness);
            const isDefault = m.defaultHarness === h.harness;
            // The host refuses the last one off; the default moves only by picking another.
            const held = h.on && (isDefault || m.harnesses!.filter((x) => x.on).length === 1);
            return (
              <div key={h.harness} className="py-1">
                <div className={`flex w-full items-center justify-between gap-3 px-5 py-2 text-body ${out ? "opacity-50" : ""}`}>
                  <span className={h.on && !out ? "" : "text-mid"}>{name}</span>
                  <span className="flex items-center gap-2">
                    {h.on && (
                      <button type="button" aria-pressed={isDefault} disabled={busy || out || isDefault} onClick={() => post("api/harness", { harness: h.harness, default: true })} className={`rounded-full px-3 py-1 text-label ${isDefault ? "bg-ink text-paper" : "text-mid hover:text-ink"}`}>
                        Default
                      </button>
                    )}
                    <button type="button" role="switch" aria-checked={h.on} aria-label={`${name} on`} disabled={busy || held} title={held ? (isDefault ? "Make another harness the default first" : "One harness has to stay on") : undefined} onClick={() => post("api/harness", { harness: h.harness, on: !h.on })} className={toggleButton}>
                      <Toggle on={h.on} />
                    </button>
                  </span>
                </div>
                {h.on && !out && (
                  <div className="flex w-full items-center justify-between gap-3 px-5 py-1.5 text-label text-mid">
                    <label htmlFor={`default-model-${h.harness}`}>Default model</label>
                    <select id={`default-model-${h.harness}`} disabled={busy} value={h.model ?? ""} onChange={(e) => post("api/harness", { harness: h.harness, model: e.target.value })} className={select}>
                      <option value="">Its own</option>
                      {h.model && !own.some((o) => o.model === h.model) && <option value={h.model}>{h.model}</option>}
                      {own.map((o) => (
                        <option key={o.model} value={o.model}>
                          {o.name}
                        </option>
                      ))}
                    </select>
                  </div>
                )}
                {out && <p className="px-5 pb-1 text-label text-accent">Not signed in: {h.remedy.replace(/\.$/, "")}.</p>}
              </div>
            );
          })}
          {m.othersOffered.length > 1 && (
            <div className="flex w-full items-center justify-between gap-3 px-5 py-2.5 text-body">
              <span>Other models via</span>
              <span role="radiogroup" aria-label="Other models via" className="flex rounded-full border-[1.25px] border-edge p-0.5">
                {m.othersOffered.map((h) => {
                  const out = m.signedOut?.includes(h) ?? false;
                  return (
                    <button
                      key={h}
                      type="button"
                      role="radio"
                      aria-checked={m.others === h}
                      disabled={busy || out}
                      title={out ? "Not signed in" : undefined}
                      onClick={() => m.others !== h && post("api/others", { others: h })}
                      className={`rounded-full px-3 py-1 text-label ${m.others === h ? "bg-ink text-paper" : out ? "text-mid opacity-50" : "text-mid hover:text-ink"}`}
                    >
                      {HARNESS_NAMES[h] ?? h}
                    </button>
                  );
                })}
              </span>
            </div>
          )}
          <p className="px-5 pb-2 text-label text-mid">Only harnesses that are on show in the pickers.</p>
        </Section>
      )}
    </>
  );
}

// The model's thinking, summarized, folded to one line as Paseo shows it:
// "Thinking" pulses while it streams; a tap opens the words.
function Thought({ text }: { text: string }) {
  const { musing } = useCustom();
  return <Musing text={text} now={musing === text} />;
}

function Musing({ text, now }: { text: string; now: boolean }) {
  const detail = useChatDetail();
  const [open, setOpen] = useState(detail === "full");
  useEffect(() => setOpen(detail === "full"), [detail]);
  if (detail === "simple") return null;
  return (
    <div className="my-1.5">
      <button
        type="button"
        onClick={(e) => {
          const at = e.currentTarget;
          if (open) hold(at);
          else reveal(at.parentElement);
          setOpen((o) => !o);
        }}
        aria-expanded={open}
        className="flex items-center gap-1.5 text-label text-soft hover:text-mid"
      >
        <svg viewBox="0 0 24 24" className="size-3.5 shrink-0" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
          {GLYPHS.brain}
        </svg>
        <span className={now ? "thinking" : undefined}>{now ? "Thinking" : "Thought"}</span>
        <svg viewBox="0 0 16 16" className={`size-3 transition-transform ${open ? "rotate-90" : ""}`} fill="none" stroke="currentColor" strokeWidth="1.6" aria-hidden>
          <path d="M6 4l4 4-4 4" />
        </svg>
      </button>
      {open && <div className="mt-1 border-l-2 border-hair pl-3 text-control leading-relaxed whitespace-pre-wrap text-soft [overflow-wrap:anywhere]">{text.trim()}</div>}
    </div>
  );
}

// The turn's background jobs, named as Claude Code names them, while they
// run on after the answer or beside the work.
function Jobs({ names }: { names: string[] }) {
  return (
    <div className="my-1.5 flex items-center gap-2 text-label text-soft" aria-label="Running in the background">
      <span className="size-3 shrink-0 animate-spin rounded-full border-[1.5px] border-hair border-t-active" />
      <span className="min-w-0 truncate">
        <span className="thinking">Working</span> · {names.length === 1 ? names[0] : `${names.length} jobs: ${names.join(", ")}`}
      </span>
    </div>
  );
}

// Three dots while the turn runs with nothing streaming: no word, since
// the pause may be a call or the network as often as the model thinking.
/** While a turn runs with nothing to show yet: the book loader, its pages
 * flipping over and back (bookLoader.ts). Under reduced motion it stays the
 * shut book, a flat bar. */
function Dots() {
  const still = useMemo(() => matchMedia("(prefers-reduced-motion: reduce)").matches, []);
  return (
    <div className="flex h-6 items-center text-soft" aria-label="Working" role="img">
      <svg viewBox={BOOK_VIEWBOX} className="h-[18px] w-auto" aria-hidden="true">
        <g fill="none" stroke="currentColor" strokeWidth={BOOK_STROKE} strokeLinecap="round">
          {BOOK_PAGES.map((frames, i) => (
            <path key={i} d={frames[0]}>
              {!still && <animate attributeName="d" dur={`${BOOK_DURATION_MS}ms`} repeatCount="indefinite" values={frames.join(";")} />}
            </path>
          ))}
        </g>
      </svg>
    </div>
  );
}


/** Back to the host that served this page, while another is picked: the
 * way out when that one is down or turns this device away. */
function Home() {
  const far = remote();
  if (!far) return null;
  return (
    <p className="mt-8 text-label text-mid">
      Talking to {new URL(far.url).host}.{" "}
      <button type="button" className="underline underline-offset-2 hover:text-ink" onClick={() => (standalone() ? openPicking() : use(null))}>
        {standalone() ? "Pick another" : `Back to ${location.host}`}
      </button>
    </p>
  );
}

function SignIn() {
  return (
    <Solo>
      <h1 className="font-serif text-display leading-[1.05] tracking-[-0.015em]">{APP}</h1>
      <p className="mt-[18px] text-control text-mid">Open a link from `libri pair` on the host.</p>
      <Home />
    </Solo>
  );
}

/** The app served on its own, before a host is picked: empty, with the
 * hosts this browser knows and a way to add one, as Paseo's app opens.
 * Opened over a host in use (`back`) it adds a host: Back and Cancel return
 * to that host as it was. */
function Welcome({ back = false }: { back?: boolean }) {
  return (
    <Solo>
      {back && (
        <button type="button" onClick={closePicking} aria-label="Back" title="Back" className="fixed top-2.5 left-3 grid size-9 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink">
          <svg viewBox="0 0 24 24" className="size-[18px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
            <path d="M15 6l-6 6 6 6" />
          </svg>
        </button>
      )}
      <h1 className="font-serif text-display leading-[1.05] tracking-[-0.015em]">{back ? "Add a host" : APP}</h1>
      <p className="mt-[18px] text-control text-mid">{back ? "By its address or a pairing link, or pick one this browser knows." : "Add a machine by its address to see its chats."}</p>
      <div className="mt-8 w-[min(360px,100%)] text-left">
        <Hosts open start={back} cancel={back ? closePicking : undefined} row="flex w-full items-center justify-between gap-6 px-5 py-2.5 text-left text-body hover:bg-hover" note="text-label text-mid" />
      </div>
    </Solo>
  );
}

const Arrow = ({ down = false }: { down?: boolean }) => (
  <svg viewBox="0 0 24 24" className={`size-5 ${down ? "rotate-180" : ""}`} fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
    <path d="M12 19V5M5 12l7-7 7 7" />
  </svg>
);

const Plus = () => (
  <svg viewBox="0 0 24 24" className="size-5" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round">
    <path d="M12 5v14M5 12h14" />
  </svg>
);

// What the + offers: a phone gets its camera too, as ChatGPT's and
// Claude's apps do; a desktop has none worth asking for.
const TOUCH = matchMedia("(pointer: coarse)").matches;
const SOURCES = [
  ...(TOUCH ? [{ label: "Camera", accept: "image/*", capture: "environment", path: "M14.5 4h-5L7 7H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3l-2.5-3zM12 17a4 4 0 1 0 0-8 4 4 0 0 0 0 8z" }] : []),
  { label: TOUCH ? "Photos" : "Add image", accept: "image/*", path: "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM8.5 10a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3zM21 15l-5-5L5 21" },
  { label: TOUCH ? "Files" : "Upload file", accept: "*", path: "m21.44 11.05-9.19 9.19a6 6 0 0 1-8.49-8.49l8.57-8.57A4 4 0 1 1 18 8.84l-8.59 8.57a2 2 0 0 1-2.83-2.83l8.49-8.48" },
];

/** The composer's +: a small menu above it of where a file comes from. */
function ChatAddMenu() {
  const aui = useAui();
  return <AddMenu add={(f) => void aui.composer.addAttachment(f)} />;
}

function AddMenu({ add }: { add: (file: File) => void }) {
  const [open, setOpen, ref] = useMenu();
  const pick = (accept: string, capture?: string) => {
    setOpen(false);
    const input = document.createElement("input");
    input.type = "file";
    input.accept = accept;
    input.multiple = !capture;
    if (capture) input.setAttribute("capture", capture);
    input.onchange = () => {
      for (const f of input.files ?? []) add(f);
    };
    input.click();
  };
  return (
    <div ref={ref} className="relative shrink-0">
      <button
        type="button"
        aria-label="Attach"
        title="Attach files or photos"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className={`grid size-[38px] place-items-center rounded-full hover:text-ink ${open ? "bg-hover text-ink" : "text-mid"}`}
      >
        <Plus />
      </button>
      {open && (
        <div role="menu" className="absolute bottom-full left-0 z-(--layer-pop) mb-2 min-w-44 overflow-hidden rounded-xl border-[1.25px] border-edge bg-lift py-1 shadow-lift">
          {SOURCES.map((o) => (
            <button key={o.label} type="button" role="menuitem" onClick={() => pick(o.accept, o.capture)} className="flex w-full items-center gap-3 px-4 py-2 text-left text-body text-ink hover:bg-hover">
              <svg viewBox="0 0 24 24" className="size-[18px] text-mid" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round">
                <path d={o.path} />
              </svg>
              {o.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** A local URL for a photo's thumbnail, let go when the chip goes. */
function usePicture(a: Attachment): string | undefined {
  const [url, setUrl] = useState<string>();
  useEffect(() => {
    if (!a.file || !a.file.type.startsWith("image/")) return;
    const u = URL.createObjectURL(a.file);
    setUrl(u);
    return () => URL.revokeObjectURL(u);
  }, [a.file]);
  const shown = a.content?.find((p) => p.type === "image");
  return shown?.type === "image" ? shown.image : url;
}

// A file in the composer or on a sent message: the photo itself, or a
// card with the file's name.
function Chip({ attachment: a, removable = false }: { attachment: Attachment; removable?: boolean }) {
  const picture = usePicture(a);
  return (
    <AttachmentPrimitive.Root className="relative w-fit">
      {picture ? (
        <button type="button" onClick={() => view(picture)} aria-label={`Open ${a.name}`} className="block cursor-zoom-in">
          <img src={picture} alt={a.name} className={`rounded-xl border border-hair object-cover ${removable ? "size-16" : "max-h-72 max-w-full"}`} />
        </button>
      ) : (
        <div className="flex max-w-60 items-center gap-2 rounded-xl border border-hair bg-paper px-3 py-2 text-control leading-5">
          <span className="rounded-md bg-hair px-1.5 py-0.5 text-caption leading-4 font-semibold text-mid uppercase">{a.name.split(".").pop()?.slice(0, 4)}</span>
          <span className="truncate">{a.name}</span>
        </div>
      )}
      {removable && (
        <AttachmentPrimitive.Remove
          aria-label="Remove"
          className="absolute -top-1.5 -right-1.5 grid size-5 place-items-center rounded-full bg-ink text-caption leading-none text-paper"
        >
          ×
        </AttachmentPrimitive.Remove>
      )}
    </AttachmentPrimitive.Root>
  );
}

/** A picture Libri sent; a tap opens it large. */
function Picture() {
  const image = useAuiState((s) => (s.part.type === "image" ? s.part.image : null));
  if (!image) return null;
  return (
    <button type="button" onClick={() => view(image)} aria-label="Open the picture" className="block cursor-zoom-in">
      <img src={image} alt="" className="max-h-80 max-w-full rounded-lg border border-hair" />
    </button>
  );
}

/** A file Libri sent back: its card saves it. */
function Download() {
  const part = useAuiState((s) => (s.part.type === "file" ? s.part : null));
  if (!part) return null;
  const name = part.filename ?? "file";
  return (
    <a
      href={part.data}
      download={name}
      className="my-1 flex w-fit max-w-full items-center gap-2.5 rounded-xl border border-hair bg-paper px-3 py-2 text-control leading-5 no-underline hover:bg-hover"
    >
      <span className="rounded-md bg-hair px-1.5 py-0.5 text-caption leading-4 font-semibold text-mid uppercase">{name.includes(".") ? name.split(".").pop()?.slice(0, 4) : "file"}</span>
      <span className="truncate text-ink">{name}</span>
      <svg viewBox="0 0 24 24" className="size-4 shrink-0 text-mid" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
        <path d="M12 4v12m0 0-5-5m5 5 5-5M5 20h14" />
      </svg>
    </a>
  );
}

type Send = Extract<Chat, { state: "ready" }>["send"];
type Decide = (id: number, yes: boolean) => Promise<void>;
type Reply = (id: string, yes: boolean, answers?: Answers, always?: boolean) => Promise<void>;

/** Says when the runtime has drawn its messages, which it does a render after
 * the chat's land: in the same commit as theirs, so the DOM holds them. */
function Drawn({ then }: { then: () => void }) {
  const drawing = useAuiState((s) => s.thread.messages);
  useLayoutEffect(then, [drawing, then]);
  return null;
}

type ThreadProps = {
  messages: Msg[];
  typing: boolean;
  sending: boolean;
  jobs: string[];
  send: Send;
  retry: Ready["retry"];
  discard: Ready["discard"];
  stop: () => Promise<void>;
  cards: Card[];
  decide: Decide;
  asks: Ask[];
  reply: Reply;
  tasks: Task[];
  stopTask: (name: string) => Promise<void>;
  tellTask: (name: string, text: string, files?: File[]) => Promise<boolean>;
  refresh: () => void;
  threads: WorkThread[];
  /** Counts each press of Chat on the talk, which leaves an agent's page. */
  left: number;
  /** Counts each read of the history that laid the talk out afresh. */
  synced: number;
};

/** How a chat's composer stands: where its draft is kept, its words'
 * placeholder, whose model it picks (the main chat's, Temp's, or a thread's
 * own), and what only the main chat has (its slash commands, a call). */
/** The mark for workers, in the launcher and the phone's plus. */
const WORKERS_ICON = (
  <>
    <rect x="3.5" y="4" width="17" height="6" rx="2" />
    <rect x="3.5" y="14" width="17" height="6" rx="2" />
  </>
);

/** The main chat's composer shows every agent at work. */
const MAIN_AGENTS: AgentScope = { main: true };

type ComposerSetup = {
  /** The draft's key (keep.ts), apart for each chat. */
  draft: string;
  placeholder: string;
  label?: string;
  /** Temp: its own model, and the dashed edge that says its words are not kept. */
  temp?: boolean;
  own?: WorkerChoice;
  commands?: boolean;
  call?: () => void;
  autoFocus?: boolean;
  /** Whose agents at work show in the composer's outline (agents.tsx). */
  agents?: AgentScope;
};

type ChatViewProps = {
  /** data-surface, which the checks find each chat by. */
  surface: string;
  messages: Msg[];
  /** A turn runs. */
  typing: boolean;
  sending?: boolean;
  send: (text: string, files: File[], again?: "again", quote?: Quote) => Promise<unknown>;
  stop: () => Promise<void>;
  /** Stop shows while this holds and the composer is empty; a turn by default. */
  stoppable?: boolean;
  stopping?: boolean;
  retry?: Ready["retry"];
  discard?: Ready["discard"];
  asks?: Ask[];
  reply?: Reply;
  cards?: Card[];
  decide?: Decide;
  jobs?: string[];
  synced?: unknown;
  /** None where the chat is only read: a seat's own agent, a device paired to read. */
  composer: ComposerSetup | null;
  /** What stands where nothing has been said yet. */
  empty?: ReactNode;
  /** What stands under the talk: a fault, what a worker is doing. */
  foot?: ReactNode;
  /** Over the composer, beside the tray: Temp's promote. */
  over?: ReactNode;
  head: () => ReactNode;
  className?: string;
  /** Over the talk: a call, a subagent's page on a phone. */
  children?: ReactNode;
};

/** Every chat: the main chat, Temp, a named memory's chat, a worker's
 * thread. One renderer (assistant-ui's), one scroll hold, one dock and one
 * composer; a chat differs only by what it is given. Two renderers, a
 * hand-drawn one for the threads, let the threads' scroll drift from the
 * chat's (2026-11-10: Temp ran off up the page on every send). */
function ChatView({ surface, messages, typing, sending = false, send, stop, stoppable = typing, stopping, retry = noRetry, discard = noDiscard, asks = [], reply = noReply, cards = [], decide = noDecide, jobs = [], synced, composer, empty, foot, over, head, className = "", children }: ChatViewProps) {
  const detail = useChatDetail();
  const viewport = useRef<HTMLDivElement>(null);
  // Marked over the whole history, since a notice marks the answers after
  // it, then drawn from where useEarlier has reached.
  const earlier = useEarlier(messages, viewport);
  const marked = useMemo(() => markOutside(messages), [messages]);
  const talk = useMemo(() => marked.slice(earlier.start), [marked, earlier.start]);
  // Dots show whenever the turn runs with nothing streaming, as
  // Claude Code's spinner does, so a pause mid-turn doesn't look finished;
  // streamed thinking shows as its own row, a running call spins in its
  // stack, and a question waiting on the person isn't work.
  const live = messages.at(-1)?.live ? messages.at(-1) : undefined;
  // A call still runs even after the person's mid-turn words land below it.
  const answer = [...messages].reverse().find((m) => m.role !== "user");
  const running = !!answer?.live && !!answer.tools?.some((t) => t.result === undefined && !t.error);
  const working = typing && !asks.length && !running;
  const writable = !!composer;
  // Drawn inside the newest answer (Offered), so with none showing (the
  // person's words below it) a question stays its card.
  const answerLast = messages.at(-1)?.role === "assistant";
  const offer = useMemo(() => (!writable || !answerLast ? undefined : (offered(messages) ?? asks.map(askOffer).find(Boolean))), [messages, asks, writable, answerLast]);
  const offering = useMemo(() => (offer ? { offer, send: async (text: string, files: File[], again?: "again", quote?: Quote) => void (await send(text, files, again, quote)), reply } : null), [offer, send, reply]);

  const { dock, column } = useDock();
  const [bottom, setBottom] = useState(true);
  const talkMain = useRef<HTMLElement>(null);
  const footRoom = useRef<HTMLDivElement>(null);
  const { toFoot, drawn: held } = useFootHold(viewport, talkMain, footRoom, setBottom, typing, synced);
  const { drawn: placed, whole } = earlier;
  const drawn = useCallback(() => {
    placed();
    held();
  }, [placed, held]);

  // Read when a redraw is pressed, so the messages needn't all redraw on
  // every word that streams in.
  const now = useRef(messages);
  now.current = messages;
  const [quoting, quote] = useState<Quote | null>(null);
  // onNew reads it when the send lands, not as it stood at the last render.
  const quoted = useRef(quoting);
  quoted.current = quoting;
  const redo = useMemo<Redo>(
    () => ({
      redraw(msgId) {
        // Resend the question that led here — the person's last words before
        // this answer — so it is answered afresh from current data. The
        // last answer is replaced, as ChatGPT regenerates; an older one is
        // asked again below, since replacing it would drop all that followed.
        const messages = now.current;
        const i = messages.findIndex((m) => m.id === msgId);
        const last = !typing && !messages.slice(i + 1).some((m) => m.role === "user");
        for (let j = i - 1; j >= 0; j--) if (messages[j].role === "user") return void send(messages[j].text, [], last ? "again" : undefined);
      },
      typing,
      writable,
      retry,
      discard,
      quoting,
      quote,
    }),
    [send, typing, writable, retry, discard, quoting],
  );

  const runtime = useExternalStoreRuntime({
    messages: talk,
    convertMessage: convert,
    isRunning: false,
    adapters: { attachments },
    onNew: async (m: AppendMessage) => {
      const text = m.content.flatMap((p) => (p.type === "text" ? [p.text] : [])).join("\n");
      const files = (m.attachments ?? []).flatMap((a) => (a.file ? [a.file] : []));
      if (text.trim() || files.length) {
        const q = quoted.current ?? undefined;
        quote(null);
        // A send goes to the latest, as Claude's and the messaging apps' do,
        // wherever a Reply or Edit left the page.
        requestAnimationFrame(toFoot);
        await send(text, files, undefined, q);
      }
    },
  });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
     <RedoCtx.Provider value={redo}>
     <WholeCtx.Provider value={whole}>
     <OfferCtx.Provider value={offering}>
      <ThreadPrimitive.Root data-surface={surface} className={`relative flex h-full min-h-0 flex-col bg-paper ${className}`}>
        {head()}
        <div className="flex min-h-0 flex-1">
        {/* The composer sits under the talk, not sticky inside it: Android
            Chrome kept placing a sticky footer by the talk's height from
            before the tab bar came back, sinking it behind the bar. */}
        <div ref={column} className="relative flex min-w-0 flex-1 flex-col [--dock-foot:14px] max-sm:[--dock-foot:8px] [--fade-under:var(--dock,0px)] [--fade-floor:var(--chat-foot,var(--dock-foot))] [--fade-top:var(--top-fade,12px)] phone:[--fade-hide:var(--tile-top,0px)] phone:[--fade-top:calc(var(--tile-top,0px)+var(--chat-top-fade))]">
        {/* Its follow is off: useFootHold follows the foot, and lets go of it
            when the person goes up. The talk fills the whole column, under
            the dock (and on a phone the bar), which float over it; it stays
            clear to the dock's top edge and dims beneath it, and fades at the
            top under whatever floats there, as the Claude app's: on a phone
            hidden under the status bar, then fading in over a short band
            just below it (--chat-top-fade), so the talk reads clear behind
            the header's buttons, as Claude Duet's (photo, 2026-11-13). The edges are
            the two chat-edge layers below, not a mask on this scroller. */}
        <ThreadPrimitive.Viewport ref={viewport} autoScroll={false} data-chat-scroll className="flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain">
          <main ref={talkMain} className="chat-column pt-[var(--talk-top,16px)] pb-10">
            <div style={TALK_SIZE} className="flex flex-col gap-3.5">
                {!messages.length && empty}
                <ThreadPrimitive.Messages>
                  {({ message }) =>
                    (
                      // The address a quote's tap goes to (jumpTo).
                      <div data-message data-row={(message.metadata.custom as { row?: number }).row} className="rounded-xl">
                        {message.role === "user" ? <UserMessage /> : message.role === "assistant" ? <AssistantMessage /> : <Note />}
                      </div>
                    )
                  }
                </ThreadPrimitive.Messages>
                <Drawn then={drawn} />
                {/* A finished answer with the turn still open is the model
                    waiting on a background job, not thinking. */}
                {detail === "simple" ? typing && !asks.length && <Dots /> : working && (live ? !live.thinking?.trim() && !live.text.trim() : messages.at(-1)?.role !== "assistant") && <Dots />}
                {detail !== "simple" && typing && jobs.length > 0 && <Jobs names={jobs} />}
                {foot}
            </div>
            {/* The room a turn has taken stays under it (useFootHold). */}
            <div ref={footRoom} aria-hidden />
          </main>
          {/* The dock's room: the last message scrolls whole above the
              composer, and the talk shows on under and around it. */}
          <div aria-hidden className="h-[var(--dock,0px)] shrink-0" />
        </ThreadPrimitive.Viewport>
          <div aria-hidden data-chat-edge="top" className="chat-edge-top" />
          <div aria-hidden data-chat-edge="foot" className="chat-edge-foot" />
          {/* Short of room the talk goes first, then the tray; the composer
              stays whole. It floats over the talk with nothing behind it. */}
          <div ref={dock} className="pointer-events-none absolute inset-x-0 bottom-0 z-20 flex max-h-full min-h-0 flex-col pt-2 pb-[var(--chat-foot,var(--dock-foot))] max-sm:pt-1.5">
            <div className="chat-column pointer-events-auto relative flex min-h-0 flex-col">
              {/* What waits or works above the composer scrolls as one, so the talk stays in view. */}
              <div className="max-h-[min(calc(var(--room,100vh)*0.66),720px)] min-h-0 shrink-[9999] overflow-y-auto pt-2.5 empty:hidden">
                {asks
                  .filter((a) => a.id !== offer?.ask?.id)
                  .map((a) => (
                    <AskCard key={a.id} ask={a} reply={reply} />
                  ))}
                <Tray cards={cards} decide={decide} />
              </div>
              {over}
              <button type="button" aria-label="Scroll to bottom" disabled={bottom} onClick={() => viewport.current?.scrollTo({ top: viewport.current.scrollHeight })} className="absolute -top-12 left-1/2 z-10 grid size-9 -translate-x-1/2 place-items-center rounded-full border border-frame bg-lift text-mid shadow-lift hover:text-ink disabled:invisible">
                <Arrow down />
              </button>
              <ReadingStop />
              {composer && <Composer setup={composer} typing={typing} sending={sending} stoppable={stoppable} stopping={stopping} stop={stop} />}
            </div>
          </div>
        </div>
        </div>
        {children}
      </ThreadPrimitive.Root>
     </OfferCtx.Provider>
     </WholeCtx.Provider>
     </RedoCtx.Provider>
    </AssistantRuntimeProvider>
  );
}
const noRetry: Ready["retry"] = () => {};
const noDiscard: Ready["discard"] = () => undefined;
const noReply: Reply = async () => {};
const noDecide: Decide = async () => {};

/** The composer, the same in every chat: the words, files, model, thinking
 * and access under them, as T3 Code and Paseo lay it out, then the one
 * button (Trailing). */
function Composer({ setup, typing, sending, stoppable, stopping, stop }: { setup: ComposerSetup; typing: boolean; sending: boolean; stoppable: boolean; stopping?: boolean; stop: () => Promise<void> }) {
  const me = useMe();
  const account = usePickedAccount(useUsage(typing), setup.temp ? "temp" : setup.own ? setup.own.choice : "chat");
  return (
    <>
      {setup.own?.failed && <p role="alert" className="mb-1.5 px-3 text-label text-accent">{setup.own.failed}</p>}
      {/* Opaque and unblurred, unlike other floating chrome: Chromium clips a
          backdrop-filter to a large radius without antialiasing on GPU paths
          (crbug 40890814's fast rounded corners), which stair-stepped this
          outline at 1x; at 90% fill the blur showed nothing anyway. */}
      <ComposerPrimitive.Root className={`floating relative shrink-0 rounded-[26px] bg-lift! [backdrop-filter:none]! [-webkit-backdrop-filter:none]! py-[7px] pr-[7px] pl-2 focus-within:border-soft/50 max-sm:py-[5px] max-sm:pr-[5px] max-sm:pl-1.5 ${setup.temp ? "border-dashed! border-frame!" : ""}`}>
        {setup.agents && <ComposerAgents scope={setup.agents} />}
        <ComposerPrimitive.AttachmentDropzone className="flex flex-col">
        <KeepDraft at={setup.draft} />
        <QuoteStrip />
        <div className="flex flex-wrap gap-2 px-3 empty:hidden [&:not(:empty)]:pt-2 [&:not(:empty)]:pb-1">
          <ComposerPrimitive.Attachments>{({ attachment }) => <Chip attachment={attachment} removable />}</ComposerPrimitive.Attachments>
        </div>
        <div className="flex px-2.5">
        <SlashInput setup={setup} />
        </div>
        {/* z-20: a container is a stacking context, and its menus open over the talk. */}
        <div className="@container relative z-20 flex items-center gap-1">
        <ChatAddMenu />
        <ModelPickers temp={setup.temp} own={setup.own} />
        <span className="flex-1" />
        <UsageRings account={account} />
        {me?.dictate && <ChatDictate />}
        <Trailing stoppable={stoppable} stopping={stopping} busy={typing || sending} stop={stop} call={setup.call} />
        </div>
        </ComposerPrimitive.AttachmentDropzone>
      </ComposerPrimitive.Root>
    </>
  );
}

/** A composer with no talk above it: the new-task sheet's. */
function LoneComposer({ setup, send }: { setup: ComposerSetup; send: (text: string, files: File[]) => Promise<boolean> }) {
  const runtime = useExternalStoreRuntime({
    messages: [] as Msg[],
    convertMessage: convert,
    isRunning: false,
    adapters: { attachments },
    onNew: async (m: AppendMessage) => {
      const text = m.content.flatMap((p) => (p.type === "text" ? [p.text] : [])).join("\n");
      const files = (m.attachments ?? []).flatMap((a) => (a.file ? [a.file] : []));
      if (text.trim() || files.length) await send(text.trim(), files);
    },
  });
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <Composer setup={setup} typing={false} sending={false} stoppable={false} stop={async () => {}} />
    </AssistantRuntimeProvider>
  );
}

function Thread({ messages, typing, sending, jobs, send, retry, discard, stop, cards, decide, asks, reply, tasks, stopTask, tellTask, refresh, threads, left, synced, newTemp }: ThreadProps & { newTemp?: () => void }) {
  usePinchText();
  // The tile menu's rows for the chat.
  const lastAnswer = [...messages].reverse().find((m) => m.role === "assistant" && !m.progress && m.text.trim())?.text;
  useTileRows("chat", {
    "search": { run: openSearch },
    "copy-answer": { disabled: !lastAnswer, run: () => void navigator.clipboard?.writeText(lastAnswer ?? "") },
    "new-temp": newTemp && { run: newTemp },
  });
  const [calling, setCalling] = useState(false);
  const me = useMe();
  // A call needs a speech service or xAI's voice; an older host says only the voice.
  const canCall = me?.call ?? !!me?.voice;
  useMemoryCommand();
  const [agent, setAgent] = useState<{ call: string; agent?: string; title?: string }>();
  const agents = useMemo<Agents>(() => ({ tasks, open: (call, agent, title) => setAgent({ call, agent, title }) }), [tasks]);
  // The main chat's own subagents, tapped in the composer's tree, open as
  // their cards' Agent pages do: beside the talk, or over it on a phone.
  const outer = useContext(RunningCtx);
  const ownRunning = useMemo(() => outer && { ...outer, openAgent: (call: string) => setAgent({ call }) }, [outer]);
  useEffect(() => setAgent(undefined), [left]);
  const wide = useWide();
  const readOnly = useMe()?.scope === "read";
  const page = (side: boolean) => agent && <AgentPage key={agent.call} {...agent} thread={threads.find((t) => t.id === agent.call)} parent={parentOf(threads.find((t) => t.id === agent.call), threads)} side={side} tasks={tasks} stop={stopTask} tell={tellTask} close={() => setAgent(undefined)} />;
  const tabs = useContext(ChatTabsCtx);
  const head = () => {
    const finding = (
      <>
        {/* Always first: the one way in on a tablet, which has no key
            (a design choice of 2026-11-03). On a wide screen, the top left pill. */}
        <HeaderButton label="Search" onClick={openSearch}>
          <circle cx="11" cy="11" r="7" />
          <path d="m20 20-3.5-3.5" />
        </HeaderButton>
      </>
    );
    return (
      // A bar docked over the talk, in the page's paper, the side
      // columns' heads' height, with the chat's name at its left: floating
      // over the talk, the controls had words running under them and no
      // name for where the person was (a design choice of 2026-11-05).
      // Its foot fades into the talk rather than ending in a hairline, and
      // the fade runs only from the pill row's foot (58px) to where the
      // talk starts (--talk-top, 72px): a line clear of the pills reads in
      // full rather than half faded.
      // On a phone, Android's header (Places.kt): no name, each button a
      // disc of its own 10px apart, floating over the talk with nothing
      // behind it, as the Claude app's: the talk runs on up under it and
      // the tiles' bar to the top of the screen, and fades out there
      // (the viewport's mask), rather than ending at a band of paper.
      <header className={`pointer-events-none absolute inset-x-0 z-20 flex justify-between ${wide ? "top-0 h-[72px] items-center bg-[linear-gradient(var(--paper)_58px,transparent)] px-[max(12px,env(safe-area-inset-left))] pb-0.5" : "top-[var(--tile-top,0px)] h-16 items-start px-[max(14px,env(safe-area-inset-left))] pt-2.5"}`}>
        <div className="pointer-events-auto flex min-w-0 items-center gap-2">
          {/* On a wide screen what finds in the chat stands apart
              from what opens beside it, a pill of its own at the top left,
              as the right one mirrored (a design choice of 2026-11-04). */}
          {wide && <div className={`flex ${FLOAT}`}>{finding}</div>}
          {/* A chat tile's memories as tabs, or the assistant's name where
              there is no tile (memoryTabs.tsx). */}
          {tabs ? <div className={`min-w-0 ${wide ? "" : "ml-[52px]"}`}>{tabs}</div> : wide && <h1 className="min-w-0 truncate px-1.5 text-control font-medium text-ink">{NAME}</h1>}
        </div>
        {/* On a phone what finds stands at the right; the dock
            over the tiles opens the rest. */}
        {!wide && <div className={`pointer-events-auto flex gap-2.5 [&>*]:rounded-full ${DISC}`}>{finding}</div>}
      </header>
    );
  };
  return (
    <AgentCtx.Provider value={agents}>
     <RunningCtx.Provider value={ownRunning}>
      {/* On a wide screen a subagent's work opens beside the talk, as the
          split view's second pane; on a phone, over it. */}
      <Panes start={BESIDE.start()} keep={BESIDE.keep}>
        <ChatView
          surface="chat"
          className={`[--talk-top:calc(var(--tile-top,0px)+72px)] ${wide ? "[--top-fade:0px]" : ""}`}
          messages={messages}
          typing={typing}
          sending={sending}
          send={send}
          stop={stop}
          retry={retry}
          discard={discard}
          asks={asks}
          reply={reply}
          cards={cards.filter((c) => !c.scratchpad)}
          decide={decide}
          jobs={jobs}
          synced={synced}
          // A device paired to read sees the chat and sends nothing.
          composer={readOnly ? null : { draft: forHost("draft"), placeholder: messageTo(ASSISTANT), commands: true, autoFocus: wide, call: canCall ? () => setCalling(true) : undefined, agents: MAIN_AGENTS }}
          head={head}
        >
          <TakeCompose />
          {calling && (
            <Call
              name={NAME}
              ended={() => {
                setCalling(false);
                refresh();
              }}
            />
          )}
          {!wide && page(false)}
        </ChatView>
        {wide && page(true)}
      </Panes>
     </RunningCtx.Provider>
    </AgentCtx.Provider>
  );
}

// The composer as it stands outlives a reload: back at the start, then saved
// as it changes, and empty again once it sends.
function KeepDraft({ at }: { at: string }) {
  const aui = useAui();
  const text = useAuiState((s) => s.composer.text);
  const added = useAuiState((s) => s.composer.attachments);
  const back = useRef(false);
  useEffect(() => {
    const composer = aui.composer();
    const words = draft(at);
    if (words && !composer.getState().text) composer.setText(words);
    void keptFiles(at)
      // The page back from another tab still has them in the composer.
      .then((fs) => Promise.all((composer.getState().attachments.length ? [] : fs).map((f) => composer.addAttachment(f))))
      .finally(() => void (back.current = true));
  }, [aui, at]);
  useEffect(() => {
    if (back.current) keepDraft(at, text);
  }, [at, text]);
  useEffect(() => {
    if (back.current) void keepFiles(at, added.flatMap((a) => (a.file ? [a.file] : [])));
  }, [at, added]);
  return null;
}

// What another part of the page puts into the main chat's composer (compose).
function TakeCompose() {
  const aui = useAui();
  useEffect(() => {
    const composer = aui.composer();
    return onCompose((t) => composer.setText(joined(composer.getState().text, t)));
  }, [aui]);
  return null;
}

// The composer's autosizer measures on a render and on a window resize, so a
// render while a column slides beside it leaves the height a sliver of chat
// needs. A new width renders again, wherever it came from.
function useFitToWidth() {
  const input = useRef<HTMLTextAreaElement>(null);
  const [, setWidth] = useState(0);
  useEffect(() => {
    const el = input.current;
    if (!el) return;
    const o = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width));
    o.observe(el);
    return () => o.disconnect();
  }, []);
  return input;
}

// The composer's words, with the slash commands as a menu over it while
// they are a bare "/name", as Claude Code and Codex list theirs: arrows
// move, Tab or Enter takes one, Escape shuts it. One that takes an argument
// is filled in to finish; the others run at once. Memo'd: the autosizer
// measures in a layout effect on every render, forcing the whole page to lay
// out, so it renders on its own words, width and state, not the page's.
/** A line of the `/` menu: one of Libri's commands, or one of the harness's
 * own, tagged with its harness. */
type MenuCommand = { name: string; arg?: string; does: string; harness?: string };

const HARNESS_LABELS: Record<string, string> = { claude: "Claude Code", codex: "Codex", opencode: "OpenCode", omp: "omp", pi: "Pi", muse: "Muse" };
const harnessLabel = (h: string) => HARNESS_LABELS[h] ?? h;

const SlashInput = memo(function SlashInput({ setup }: { setup: ComposerSetup }) {
  const aui = useAui();
  const input = useFitToWidth();
  const text = useAuiState((s) => s.composer.text);
  const [at, setAt] = useState(0);
  const [shut, setShut] = useState<string | null>(null);
  const me = useMe();
  const { m } = useModels(!!me?.models);
  const skills = useSkillMenu(text, (t) => aui.composer.setText(t), setup.temp ? m?.temp.pick.harness : (setup.own?.choice?.pick.harness ?? m?.pick.harness));
  const typed = setup.commands ? /^\/([\w:.-]*)$/.exec(text) : null;
  // The harness's own, below Libri's, once the menu opens: what it last
  // reported (site.rs `commands_now`), each sent to it as typed.
  const [own, setOwn] = useState<HarnessCommands>();
  const opening = !!typed;
  useEffect(() => {
    if (opening) void harnessCommands().then(setOwn);
  }, [opening]);
  const theirs: MenuCommand[] = (own?.commands ?? []).filter((n) => !COMMANDS.some((c) => c.name === n)).map((n) => ({ name: n, does: "", harness: own!.harness }));
  const list: MenuCommand[] = typed && shut !== text ? [...COMMANDS, ...theirs].filter((c) => c.name.startsWith(typed[1].toLowerCase())) : [];
  const on = Math.min(at, list.length - 1);
  const take = (c: MenuCommand) => {
    setAt(0);
    // A harness's command may take words after it, which it alone knows.
    aui.composer.setText(c.arg || c.harness ? `/${c.name} ` : `/${c.name}`);
    if (!c.arg && !c.harness) aui.composer.send();
  };
  const keys = (e: KeyEvent<HTMLTextAreaElement>) => {
    if (skills.keys(e)) return;
    if (!list.length) return;
    const step = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
    if (step) setAt((on + step + list.length) % list.length);
    else if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) take(list[on]);
    else if (e.key === "Escape") setShut(text);
    else return;
    e.preventDefault();
  };
  return (
    <>
      {skills.menu}
      {!!list.length && (
        <div
          role="listbox"
          aria-label="Commands"
          className="absolute inset-x-0 bottom-[calc(100%+6px)] z-30 max-h-[min(50vh,320px)] overflow-y-auto rounded-2xl border border-frame bg-lift py-1.5 shadow-lift"
        >
          {list.map((c, i) => (
            <button
              key={c.name}
              type="button"
              role="option"
              aria-selected={i === on}
              // The arrows' pick kept in sight in a list longer than the menu.
              ref={i === on ? (el) => el?.scrollIntoView({ block: "nearest" }) : undefined}
              // Before the textarea's blur, so the words keep their focus.
              onPointerDown={(e) => e.preventDefault()}
              onClick={() => take(c)}
              onPointerEnter={() => setAt(i)}
              className={`flex w-full items-baseline gap-3 px-4 py-1.5 text-left ${i === on ? "bg-hover" : ""}`}
            >
              <span className="shrink-0 whitespace-nowrap font-mono text-control text-ink">
                /{c.name}
                {c.arg && <span className="text-soft"> {c.arg}</span>}
              </span>
              <span className="min-w-0 flex-1 truncate text-control leading-5 text-mid">{c.does}</span>
              {c.harness && <span className="shrink-0 text-label text-soft">{harnessLabel(c.harness)}</span>}
            </button>
          ))}
        </div>
      )}
      <ComposerPrimitive.Input
        ref={input}
        // Not on a phone: it would raise its keyboard and hide the bar at
        // the foot before the person has chosen to type.
        autoFocus={!!setup.autoFocus}
        placeholder={setup.placeholder}
        aria-label={setup.label}
        onKeyDown={keys}
        // A paste too long for a message becomes a text file at once, shown
        // as a chip before it goes, as Claude and ChatGPT take one.
        onPaste={(e) => {
          const pasted = e.clipboardData.getData("text/plain");
          if (new TextEncoder().encode(pasted).length <= SEND_MAX) return;
          e.preventDefault();
          void aui.composer.addAttachment(new File([pasted], "pasted.txt", { type: "text/plain" }));
        }}
        className="max-h-[min(40vh,max(48px,calc(var(--room,100vh)-112px)))] flex-1 resize-none bg-transparent py-[7px] text-prose leading-normal outline-none placeholder:text-soft"
      />
    </>
  );
}, (a, b) => (Object.keys({ ...a.setup, ...b.setup }) as (keyof ComposerSetup)[]).every((k) => k === "call" || a.setup[k] === b.setup[k]));

// Dictation into the chat's composer. The words join whatever is there by
// the time they come back, which may be more than when the person spoke.
function ChatDictate() {
  const aui = useAui();
  const text = useAuiState((s) => s.composer.text);
  return <Dictate add={(words) => aui.composer.setText(joined(text, words))} hush={hush} />;
}

const CALL_ARMS = 1000;

// The one button after the composer: Stop while Libri is answering;
// else, with nothing to send and a voice to call, a call; else Send, which the
// primitive greys when there is nothing to send. One button, never two
// half-lit ones. Call waits a moment after the slot has stopped changing: on a
// slow line it would otherwise come up under a thumb still on Send or Stop.
function Trailing({ stoppable, stopping, busy, stop, call }: { stoppable: boolean; stopping?: boolean; busy: boolean; stop: () => Promise<void>; call?: () => void }) {
  const text = useAuiState((s) => s.composer.text);
  const files = useAuiState((s) => s.composer.attachments.length);
  // A picture or file alone is something to send.
  const empty = !text.trim() && !files;
  const round = "grid size-[38px] shrink-0 place-items-center rounded-full";
  const wide = useWide();
  const still = empty && !busy;
  const [armed, setArmed] = useState(still);
  useEffect(() => {
    if (!still) return setArmed(false);
    const wait = window.setTimeout(() => setArmed(true), CALL_ARMS);
    return () => clearTimeout(wait);
  }, [still]);
  // Words typed while a turn runs join it, so Send stands in for Stop.
  if (stoppable && empty)
    return (
      <button type="button" onClick={() => void stop()} disabled={stopping} aria-label={stopping ? "Stopping" : "Stop"} title={stopping ? "Stopping" : "Stop"} className={`${round} border border-frame text-ink hover:bg-hover disabled:opacity-50`}>
        <span className="size-3 rounded-[2px] bg-current" />
      </button>
    );
  if (call && still && armed)
    return (
      <button type="button" onClick={call} aria-label={`Call ${NAME}`} title={`Call ${NAME}`} className={`${round} bg-ink text-paper hover:opacity-85`}>
        <Bars />
      </button>
    );
  return (
    // Pressing Send keeps the text box focused until the tap lands: a blur
    // on press would drop the phone's keyboard, the page would grow under
    // the finger, and the tap would miss the moved button. Once sent, a
    // phone puts the keyboard away.
    // Empty, Send is itself dimmed, not a tan disc of its own: a different
    // button read as broken rather than as waiting for words.
    <ComposerPrimitive.Send
      aria-label="Send"
      onPointerDown={(e) => e.preventDefault()}
      onClick={() => !wide && window.setTimeout(() => (document.activeElement as HTMLElement | null)?.blur())}
      className={`${round} bg-ink text-paper disabled:opacity-25`}>
      <Arrow />
    </ComposerPrimitive.Send>
  );
}

function useCustom() {
  return useAuiState((s) => s.message.metadata.custom) as { progress?: boolean; metrics?: Metrics; plain: boolean; reaction: string | null; queued: boolean; failed?: string; outside: boolean; woke?: number; live: boolean; musing?: string; quote?: Quote; row?: number; from?: string };
}

/** The `rise` of what arrives, played once: hiding a pane and showing it
 * again (another workspace, the phone's other place) restarts every CSS
 * animation in it, so the class goes once it has played. */
function useRise(arrives = true) {
  const [on, setOn] = useState(arrives);
  return { rise: on ? "rise" : "", onAnimationEnd: (e: React.AnimationEvent) => void (e.target === e.currentTarget && setOn(false)) };
}

/** A message rises only when it comes in while the page is open, and once:
 * one already said when the page opened, or drawn again (a second tile),
 * is simply there, as a messaging app shows its history.
 * 120 messages rising together held every redraw of the chat for 320 ms. */
const pageOpened = Date.now();
const risen = new Set<string>();
function useArrival() {
  const id = useAuiState((s) => s.message.id);
  const at = useAuiState((s) => s.message.createdAt);
  // assistant-ui stamps an undated message with the time it is converted, so
  // only a message dated by us (the chat's, and what the person sends) may
  // rise: a worker's or Temp's log has no dates and is history when opened.
  const dated = useAuiState((s) => (s.message.metadata.custom as { dated?: boolean }).dated === true);
  const r = useRise(dated && !risen.has(id) && !!at && at.getTime() >= pageOpened);
  useEffect(() => void risen.add(id), [id]);
  return r;
}

/** The message a hippo row is, brought into view and lit for a moment, as
 * a messaging app's quote jumps to what it answers. A row older than the
 * page has loaded is not there to jump to. The jump is done in a frame: no
 * smooth scroll (over a long chat it ran 1.2 s), lit at once, and the light
 * only lets go afterwards. */
function jumpTo(row: number) {
  const at = document.querySelector<HTMLElement>(`[data-row="${row}"]`);
  if (!at) return;
  at.scrollIntoView({ block: "center" });
  at.style.backgroundColor = "var(--hover)";
  window.setTimeout(() => {
    at.style.backgroundColor = "";
    at.animate([{ backgroundColor: "var(--hover)" }, { backgroundColor: "transparent" }], { duration: 600, easing: "ease-out" });
  }, 600);
}

/** One line of the message a reply answers, over the bubble; a tap goes
 * to it. */
function QuoteLine({ quote, className = "" }: { quote: Quote; className?: string }) {
  const whole = useContext(WholeCtx);
  // A row the talk hasn't drawn yet (useEarlier) is drawn, then jumped to.
  const jump = () => (document.querySelector(`[data-row="${quote.id}"]`) ? jumpTo(quote.id) : whole(() => jumpTo(quote.id)));
  return (
    <button type="button" onClick={jump} className={`flex max-w-[min(82%,560px)] min-w-0 items-center gap-1.5 rounded-md px-1.5 py-0.5 text-left text-label text-mid hover:bg-hover ${className}`}>
      {/* A straight bar of its own: a left border on the rounded button bent into a "(". */}
      <span aria-hidden className="h-3.5 w-0.5 shrink-0 rounded-full bg-edge" />
      <span className="shrink-0 font-medium">Reply to</span>
      <span className="min-w-0 truncate">{quote.text}</span>
    </button>
  );
}

/** Over the composer while its next send replies to an earlier message:
 * what it names, and an × to send it plain. */
function QuoteStrip() {
  const { quoting, quote } = useRedo();
  if (!quoting) return null;
  return (
    <div className="flex items-center gap-2 px-3 pt-1.5">
      <QuoteLine quote={quoting} className="max-w-none flex-1" />
      <button type="button" aria-label="Cancel reply" title="Cancel reply" onClick={() => quote(null)} className="grid size-7 shrink-0 place-items-center rounded-md text-soft hover:bg-hover hover:text-ink">
        <svg viewBox="0 0 24 24" className="size-[15px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" aria-hidden>
          <path d="M18 6 6 18M6 6l12 12" />
        </svg>
      </button>
    </div>
  );
}

/** Puts the composer's focus back, so what was put there is seen. */
const focusComposer = () => window.setTimeout(() => document.querySelector<HTMLTextAreaElement>("[data-composer] textarea, textarea[name=input]")?.focus());

/** Puts an earlier message in the composer's strip, as the one the next
 * send replies to. */
function useReply() {
  const { quote } = useRedo();
  return (row: number, text: string) => {
    quote({ id: row, text: firstLine(text) });
    focusComposer();
  };
}

/** Edit, the same in every chat: the message's words go
 * into the composer, to change and send as a new message. The one edited
 * stays as it was; nothing is rewound. */
function useEditInComposer() {
  const { quote } = useRedo();
  const aui = useAui();
  return (text: string) => {
    quote(null);
    aui.thread().composer().setText(text);
    focusComposer();
  };
}

/** Reply beside a message, shown on hover, as the hold menu's Reply on a
 * touch screen: only what hippo keeps has a row to name. */
function ReplyThis() {
  const reply = useReply();
  const text = useAuiState((s) => s.message.content.flatMap((p) => (p.type === "text" ? [p.text] : [])).join("\n").trim());
  const { live, row } = useCustom();
  if (row === undefined || live || !text) return null;
  return (
    <button
      type="button"
      onClick={() => reply(row, text)}
      aria-label="Reply"
      title="Reply"
      className="grid size-7 shrink-0 place-items-center rounded-md text-soft opacity-0 transition-opacity group-hover:opacity-100 hover:bg-hover hover:text-ink max-sm:opacity-100"
    >
      <svg viewBox="0 0 24 24" className="size-[15px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
        <path d="m9 14-5-5 5-5" />
        <path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11" />
      </svg>
    </button>
  );
}

function Reaction() {
  const { reaction } = useCustom();
  return reaction ? (
    <span className="absolute -bottom-3 right-3 rounded-full border border-hair bg-lift px-1.5 py-0.5 text-control leading-none">{reaction}</span>
  ) : null;
}

/** The time, shown when the pointer rests on a message. */
/** The words for the kind of device a message came from (libri/src/origin.rs). */
const DEVICE: Record<string, string> = {
  android: "Android app",
  desktop: "desktop app",
  "web-phone": "phone browser",
  "web-desktop": "browser",
  other: "another app",
};

function Time({ className = "" }: { className?: string }) {
  const at = useAuiState((s) => s.message.createdAt);
  const { from } = useCustom();
  if (!at) return null;
  const device = from && DEVICE[from];
  return <div className={`text-caption leading-4 text-soft opacity-0 transition-opacity group-hover:opacity-100 ${className}`}>{device ? `${stamp(at)} · ${device}` : stamp(at)}</div>;
}

// A person's words: framed at the right, as wide as the text. A pasted log
// breaks anywhere rather than running off the page.
function UserMessage() {
  const { queued, failed, quote } = useCustom();
  const { rise, onAnimationEnd } = useArrival();
  return (
    <HeldMessage>
    <MessagePrimitive.Root data-said="user" onAnimationEnd={onAnimationEnd} className={`${rise} group flex flex-col items-end gap-1.5`}>
      <div className="flex max-w-[min(82%,560px)] flex-wrap justify-end gap-1.5 empty:hidden">
        <MessagePrimitive.Attachments>{({ attachment }) => <Chip attachment={attachment} />}</MessagePrimitive.Attachments>
      </div>
      {quote && <QuoteLine quote={quote} />}
      {/* The time and pencil sit beside the words, as iMessage and Slack
          keep theirs, so back-to-back messages aren't spaced apart by a row
          that is empty until hovered. */}
      <div className="flex w-full items-end justify-end gap-2">
        <Time className="mb-1.5 shrink-0" />
        <span className={`flex items-end gap-2 ${TOUCH_HIDDEN}`}>
          <PinThis />
          {!queued && !failed && <ReplyThis />}
          {!queued && !failed && <Edit />}
        </span>
        <MessagePrimitive.If hasContent>
        <div className={`${BUBBLE} ${queued ? "border-dotted" : ""} ${failed ? "border-accent" : EDGE}`}>
          {/* A dotted edge: waiting for the computer to come back, or for the turn under way. */}
          <MessagePrimitive.Parts />
          <Reaction />
        </div>
        </MessagePrimitive.If>
      </div>
      {failed && <Refused reason={failed} />}
    </MessagePrimitive.Root>
    </HeldMessage>
  );
}

/** Under a message the site refused: why, and what can be done with it. */
function Refused({ reason }: { reason: string }) {
  const id = useAuiState((s) => s.message.id);
  const { retry, discard } = useRedo();
  const aui = useAui();
  const edit = () => {
    const back = discard(id);
    if (!back) return;
    const composer = aui.thread().composer();
    composer.setText(back.text);
    for (const f of back.files) void composer.addAttachment(f);
  };
  const action = "rounded-md px-1.5 py-0.5 font-medium text-ink hover:bg-hover";
  return (
    <p role="alert" className="flex flex-wrap items-center justify-end gap-x-1 text-right text-label text-accent">
      <span className="mr-1">{reason}</span>
      <button type="button" className={action} onClick={() => retry(id)}>Retry</button>
      <button type="button" className={action} onClick={edit}>Edit</button>
      <button type="button" className={action} onClick={() => void discard(id)}>Remove</button>
    </p>
  );
}

/** Pencil beside each of the person's messages, shown on hover: its words
 * go into the composer, to change and send anew (useEditInComposer). */
function Edit() {
  const edit = useEditInComposer();
  const { writable } = useRedo();
  const text = useAuiState((s) => s.message.content.flatMap((p) => (p.type === "text" ? [p.text] : [])).join("\n").trim());
  if (!text || !writable) return null;
  return (
    <button
      type="button"
      onClick={() => edit(text)}
      aria-label="Edit"
      title="Edit"
      className="grid size-7 place-items-center rounded-md text-soft opacity-0 transition-opacity group-hover:opacity-100 hover:bg-hover hover:text-ink max-sm:opacity-100"
    >
      <svg viewBox="0 0 24 24" className="size-[15px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
        <path d="M12 20h9" />
        <path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z" />
      </svg>
    </button>
  );
}

// A live answer is parsed as it streams, its unfinished markdown closed
// for it, and shows a caret until the whole answer replaces it. One mode
// throughout, the primitive's streaming, which stops closing markdown once
// the message is complete: a switch to static at the end redrew each code
// block, a frame 6px short in WebKit, a step in the text at every reply.
function Text() {
  const { live } = useCustom();
  const plugins = usePlugins(useAuiState((s) => s.message.content.map((p) => (p.type === "text" ? p.text : "")).join("")));
  // Only the words still being written carry the caret, not those a call
  // has since followed.
  const end = useAuiState((s) => s.message.parts.at(-1)?.type === "text" && s.message.parts.at(-1) === s.part);
  return (
    <StreamdownTextPrimitive
      plugins={plugins}
      shikiTheme={plugins?.code.getThemes()}
      containerClassName={live && end ? "writing" : undefined}
      lineNumbers={false}
      controls={{ code: true, table: false }}
      linkSafety={{ enabled: false }}
    />
  );
}

/** The harness's actual context and generation rate, separate from billing totals. */
function RunMeasures({ metrics }: { metrics?: Metrics }) {
  const detail = useChatDetail();
  if (detail !== "full" || !metrics) return null;
  const { context_tokens: used, context_window: limit, tokens_per_second: rate } = metrics;
  const context = used != null && limit != null && limit > 0;
  const number = (n: number) => Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 }).format(n);
  return <div data-run-measures className="not-prose my-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-caption text-soft tabular-nums">
    {context && <span className="flex min-w-0 items-center gap-2" title="Context reported by the harness; not cumulative token usage">
      <span role="progressbar" aria-label="Model context" aria-valuemin={0} aria-valuemax={limit} aria-valuenow={used} className="h-1 w-16 overflow-hidden rounded-full bg-hair"><span className="block h-full rounded-full bg-active" style={{ width: `${Math.min(100, used / limit * 100)}%` }} /></span>
      <span>Context {number(used)} / {number(limit)}</span>
    </span>}
    {rate != null && rate > 0 && <span>{rate.toFixed(1)} tok/s</span>}
    {metrics.input != null && <span title="Cumulative tokens across the turn’s model calls">{number(metrics.input)} in · {number(metrics.output ?? 0)} out</span>}
    {metrics.model && <span className="min-w-0 break-all font-mono">{metrics.model}</span>}
  </div>;
}

function AssistantMessage() {
  const detail = useChatDetail();
  const { plain, outside, metrics, progress, live } = useCustom();
  const hasText = useAuiState((s) => s.message.content.some((p) => p.type === "text" && p.text.trim()));
  const { rise, onAnimationEnd } = useArrival();
  if (detail === "simple" && (progress || live || !hasText)) return null;
  return (
    <HeldMessage>
    <MessagePrimitive.Root data-said="talk" onAnimationEnd={onAnimationEnd} className={`${rise} group relative ${outside ? OUTSIDE : ""}`}>
      {plain ? (
        // A draft waiting for approval: framed and named on its frame, as
        // UniiChat frames a report.
        <div className="relative mt-2 rounded-xl border-[1.25px] border-edge px-5 py-3.5 whitespace-pre-wrap [overflow-wrap:anywhere]">
          <span data-overhang className="absolute -top-[0.75em] left-3.5 bg-paper px-1.5 text-caption leading-4 font-semibold tracking-wide text-mid uppercase">Draft</span>
          <MessagePrimitive.Parts />
        </div>
      ) : (
        <div className="md">
          <MessagePrimitive.GroupedParts groupBy={byRun} indicator="never">
            {({ part }) => {
              if (detail === "simple" && part.type !== "text") return null;
              switch (part.type) {
                case "group-tools":
                  return (
                    <>
                      <ToolStack indices={part.indices} />
                      <Cards indices={part.indices} />
                    </>
                  );
                case "text":
                  return (
                    <div className={SAID}>
                      <Text />
                    </div>
                  );
                case "reasoning":
                  return <Thought text={(part as { text: string }).text} />;
                case "tool-call":
                  return (
                    <>
                      <Tool {...(part as unknown as ToolCallMessagePartProps)} />
                      <CallCard call={part as unknown as Call} />
                    </>
                  );
                case "image":
                  return <Picture />;
                case "file":
                  return <Download />;
                default:
                  return null;
              }
            }}
          </MessagePrimitive.GroupedParts>
        </div>
      )}
      <RunMeasures metrics={metrics} />
      <Offered />
      <Reaction />
      <Actions />
    </MessagePrimitive.Root>
    </HeldMessage>
  );
}

// The row of message buttons is for a mouse; on a touch screen holding a
// message opens them as a menu instead, as messaging apps do.
const TOUCH_HIDDEN = "[@media(pointer:coarse)]:hidden";
// Held on a touch screen, a message opens what can be done with it.
function HeldMessage({ children }: { children: ReactNode }) {
  return <Held menu={(grip, close) => <MessageMenu grip={grip} close={close} />}>{children}</Held>;
}

/** What can be done with one message: Copy, Select text, Pin, Read aloud,
 * Redraw, Create task, Reply, Edit; each only where it applies. As Claude's and
 * the messaging apps' hold menus: the page blurs, a copy of the message
 * stands out of it where it was, and the menu hangs under it. */
function MessageMenu({ grip, close }: { grip: Grip; close: () => void }) {
  const message = useAuiState((s) => s.message);
  const { live, queued, failed, row } = useCustom();
  const redo = useRedo();
  const reply = useReply();
  const edit = useEditInComposer();
  // Only what hippo keeps can be named; a message still on its way has no row.
  const shell = useShell();
  const [pinsOn] = usePinsOn();
  const { pinned, toggle } = usePins();
  const speak = useSpeak();
  const [selecting, setSelecting] = useState(false);
  const text = message.content.flatMap((p) => (p.type === "text" && p.text.trim() ? [p.text.trim()] : [])).join("\n\n");
  const icon = (d: ReactNode) => (
    <svg viewBox="0 0 24 24" className="size-[17px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      {d}
    </svg>
  );
  const done = (run: () => void) => () => {
    run();
    close();
  };
  const items: MenuItem[] = [];
  if (text && !live) {
    items.push({ label: "Copy", icon: icon(<><rect x="9" y="9" width="12" height="12" rx="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" /></>), run: done(() => void navigator.clipboard.writeText(text)) });
    items.push({ label: "Select text", icon: icon(<><path d="M4 7V4h3M20 7V4h-3M4 17v3h3M20 17v3h-3" /><path d="M9 9h6M9 12h6M9 15h4" /></>), run: () => setSelecting(true) });
  }
  // A message still waiting to go has its own way back (Refused, put back).
  if (redo.writable && message.role === "user" && text && !queued && !failed) items.push({ label: "Edit", icon: icon(<><path d="M12 20h9" /><path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z" /></>), run: done(() => edit(text)) });
  if (pinsOn && text && !live && !queued) {
    const is = pinned(message.id);
    items.push({ label: is ? "Unpin" : "Pin", icon: <PinIcon filled={is} />, run: done(() => toggle({ id: message.id, role: message.role, text, date: message.createdAt?.getTime() })) });
  }
  if (message.role === "assistant" && speak.able) items.push({ label: speak.label, icon: icon(speak.state === "idle" ? <><path d="M11 5 6 9H2v6h4l5 4V5z" /><path d="M15.54 8.46a5 5 0 0 1 0 7.07" /></> : <rect x="6" y="6" width="12" height="12" rx="1.5" />), run: done(() => void speak.press()) });
  if (redo.writable && message.role === "assistant" && !live) items.push({ label: "Redraw", icon: icon(<><path d="M21 2v6h-6M3 22v-6h6" /><path d="M3.5 9a9 9 0 0 1 14.85-3.36L21 8M21 15a9 9 0 0 1-14.85 3.36L3 16" /></>), run: done(() => redo.redraw(message.id)) });
  if (shell && text && !live) items.push({ label: "Create task", icon: <WorkGlyph className="size-[17px]" />, run: done(() => shell.promote("", { id: message.id, text })) });
  if (row !== undefined && text && !live) items.push({ label: "Reply", icon: icon(<><path d="m9 14-5-5 5-5" /><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11" /></>), run: done(() => reply(row, text)) });
  const at_ = message.createdAt;
  const when = at_ && stamp(at_);
  return (
    <HoldMenu
      grip={grip}
      framed={message.role !== "user"}
      label={when ? `Message, ${when}` : "Message"}
      items={items}
      close={close}
      over={
        selecting ? <SelectText text={text} close={close} /> : undefined
      }
    />
  );
}

/** A message's text whole, to select from on a touch screen, where holding
 * opens the menu instead of selecting. */
function SelectText({ text, close }: { text: string; close: () => void }) {
  return (
    <div role="dialog" aria-label="Select text" onClick={(e) => e.stopPropagation()} className="rise fixed inset-x-3 top-[10dvh] mx-auto flex max-h-[80dvh] max-w-[640px] flex-col rounded-2xl border border-frame bg-lift shadow-dock">
      <div className="flex items-center gap-2 border-b border-hair px-4 py-2.5">
        <h2 className="flex-1 text-body font-semibold">Select text</h2>
        <button type="button" onClick={() => (void navigator.clipboard.writeText(text), close())} className="rounded-full px-3 py-1.5 text-control leading-5 text-mid hover:bg-hover">
          Copy all
        </button>
        <HeaderButton label="Close" onClick={close}>
          <path d="M18 6 6 18M6 6l12 12" />
        </HeaderButton>
      </div>
      <div className="flex-1 overflow-y-auto px-4 py-3 text-body leading-relaxed whitespace-pre-wrap select-text [overflow-wrap:anywhere] [-webkit-touch-callout:default]">{text}</div>
    </div>
  );
}

// Copy, read aloud and redraw under words said, and under the turn's last
// piece; a piece that is only tool calls, cut by words sent mid-turn, has
// nothing to copy and no row.
function Actions() {
  const shown = useAuiState((s) => s.message.isLast || s.message.content.some((p) => p.type === "text" && !!p.text.trim()));
  if (!shown) return null;
  return (
    <div className={`relative mt-1 flex items-center gap-2 ${TOUCH_HIDDEN}`}>
      <Copy />
      <ReplyThis />
      <PinThis />
      <Speak />
      <Redraw />
      <PromoteThis />
      <Time />
    </div>
  );
}

// Promote under a finished reply: the reply's words anchor the brief.
function PromoteThis() {
  const id = useAuiState((s) => s.message.id);
  const text = useAuiState((s) => s.message.content.flatMap((p) => (p.type === "text" ? [p.text] : [])).join("\n"));
  const live = useAuiState((s) => s.message.status?.type === "running");
  if (live || !text.trim()) return null;
  return <Promote id={id} text={text} />;
}

// One player for the page, so starting a reply stops the last; `stopping`
// resets the button that was playing. The site streams speech as xAI makes
// it, so the player starts on the first piece; its address is kept by text.
const player = new Audio();
const spoken = new Map<string, Promise<string>>();
let stopping: (() => void) | null = null;

// The reply being read, for every button that can stop it: the one under the
// message, its menu, and the bar over the composer.
type Reading = { text: string; state: "loading" | "playing" };
let reading: Reading | null = null;
const readingListeners = new Set<() => void>();
const reads = (next: Reading | null) => {
  reading = next;
  readingListeners.forEach((f) => f());
};
const readingListen = (f: () => void) => {
  readingListeners.add(f);
  return () => void readingListeners.delete(f);
};
const useReading = () => useSyncExternalStore(readingListen, () => reading);

function stop() {
  player.pause();
  stopping?.();
}
const hush = stop;

const speechId = (text: string, voice: string | null) => `${voice}\n${text}`;

function speech(text: string, voice: string | null) {
  const id = speechId(text, voice);
  let url = spoken.get(id);
  if (!url) {
    url = api("api/speak", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(voice ? { text, voice } : { text }),
    })
      .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
      .then(({ key }: { key: string }) => apiUrl(`api/speak/${key}`, true));
    spoken.set(id, url);
    url.catch(() => spoken.delete(id));
  }
  return url;
}

/** Plays `text` in `voice` alone, calling `done` when it ends, fails or is
 * stopped: the sample in Settings. */
async function play(text: string, voice: string, done: () => void) {
  stop();
  const end = () => {
    if (stopping === end) stopping = null;
    done();
  };
  stopping = end;
  try {
    const src = await speech(text, voice);
    if (stopping !== end) return;
    player.src = src;
    player.playbackRate = voiceSpeed();
    player.onended = end;
    player.onerror = () => {
      spoken.delete(speechId(text, voice));
      end();
    };
    await player.play();
  } catch {
    if (stopping === end) end();
  }
}

/** One limit window of a subscription (usage.rs `Window`). */
type UsageWindow = { window: string; name: string; hours: number | null; pct: number; resets: string; resets_at?: number; model?: string };
/** A subscription, its windows shortest first (usage.rs `Account`). */
type UsageAccount = { account: string; name: string; plan?: string; windows: UsageWindow[]; notes: string[]; read_at: number; stale: boolean };
type Usage = { shown: boolean; accounts: UsageAccount[]; notes: string[] };

// One read for every composer and Settings, as the models are.
let USAGE: Usage | undefined;
const usageWatchers = new Set<(u: Usage) => void>();
const setUsageNow = (u: Usage) => {
  USAGE = u;
  for (const w of usageWatchers) w(u);
};
const askUsage = () =>
  api("api/usage/report")
    .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
    .then(setUsageNow, () => {});

/** Every subscription's windows, asked on opening, every few minutes, and
 * when a turn ends, since that is when they move. The server keeps each
 * provider's reading five minutes. */
function useUsage(typing: boolean): Usage | undefined {
  const [usage, setUsage] = useState(USAGE);
  useEffect(() => {
    usageWatchers.add(setUsage);
    return () => void usageWatchers.delete(setUsage);
  }, []);
  // On load and every five minutes, turn or no turn, so a page opened
  // mid-turn draws them too.
  useEffect(() => {
    void askUsage();
    const every = setInterval(askUsage, 5 * 60_000);
    return () => clearInterval(every);
  }, []);
  // A turn's end is what moves them; the load is asked for above.
  const was = useRef(typing);
  useEffect(() => {
    if (was.current && !typing) void askUsage();
    was.current = typing;
  }, [typing]);
  return usage;
}

/** Shows or hides the rings in every composer, here at once and on the
 * host for every device. */
function showUsage(shown: boolean) {
  if (USAGE) setUsageNow({ ...USAGE, shown });
  return api("api/usage", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ shown }) })
    .then((r) => r.ok || Promise.reject(r.status))
    .catch(askUsage);
}

/** The subscription behind the model a composer has picked, as
 * ModelPickers finds the pick: the chat's, Temp's, or a worker's own,
 * none until that is known. */
function usePickedAccount(usage: Usage | undefined, which: "chat" | "temp" | { pick: Choice } | undefined): UsageAccount | undefined {
  const me = useMe();
  const { m } = useModels(!!me?.models);
  if (!usage?.shown || !m || !which) return undefined;
  const chosen = which === "chat" ? m : which === "temp" ? m.temp : which;
  const id = m.offers.find((o) => o.harness === chosen.pick.harness && o.model === chosen.pick.model)?.usage;
  return usage.accounts.find((a) => a.account === id);
}

/** How full a ring is drawn: muted, ink from 70%, the red pencil past 90%,
 * as Paseo colours its context meter. */
const fill = (pct: number) => (pct > 90 ? "var(--accent)" : pct >= 70 ? "var(--ink)" : "var(--mid)");

/** Rings by the send button, after Paseo's context meter, for the picked
 * model's subscription: its shortest window outside, the next inside, and
 * a subscription with one window has one ring. A hover on a desktop or a
 * tap on a phone shows every window with its reset. */
function UsageRings({ account }: { account?: UsageAccount }) {
  const [open, setOpen, ref] = useMenu();
  const mouse = useRef(false);
  const shown = account?.windows.slice(0, 2) ?? [];
  if (!account || !shown.length) return null;
  const ring = (l: UsageWindow, r: number) => {
    const round = 2 * Math.PI * r;
    const pct = Math.min(100, Math.max(0, l.pct));
    return (
      <g key={r}>
        <circle cx={11} cy={11} r={r} fill="none" stroke="var(--hair)" strokeWidth={2} />
        {pct > 0 && (
          <circle cx={11} cy={11} r={r} fill="none" stroke={fill(pct)} strokeWidth={2} strokeLinecap="round" strokeDasharray={round} strokeDashoffset={round - (pct / 100) * round} transform="rotate(-90 11 11)" />
        )}
      </g>
    );
  };
  const label = account.windows.map((l) => `${l.name} ${l.pct}%`).join(", ");
  return (
    <div ref={ref} className="relative shrink-0">
      <button
        type="button"
        aria-label={`${account.name} use: ${label}`}
        aria-expanded={open}
        onPointerEnter={(e) => {
          mouse.current = e.pointerType === "mouse";
          if (mouse.current) setOpen(true);
        }}
        onPointerLeave={(e) => e.pointerType === "mouse" && setOpen(false)}
        onPointerDown={(e) => (mouse.current = e.pointerType === "mouse")}
        // A mouse opened it on the way in; only a touch toggles it.
        onClick={() => mouse.current || setOpen(!open)}
        className="grid h-[38px] w-7 place-items-center rounded-full hover:bg-hover"
      >
        <svg viewBox="0 0 22 22" className="size-[22px]" aria-hidden>
          {shown.map((l, i) => ring(l, i ? 5.5 : 9.5))}
        </svg>
      </button>
      {open && (
        <div role="tooltip" className="absolute right-0 bottom-full z-(--layer-pop) mb-2 w-max rounded-xl border-[1.25px] border-edge bg-lift px-4 py-2.5 text-label shadow-lift">
          <p className="mb-1 text-body text-ink">{account.name} use</p>
          {account.windows.map((l) => (
            <p key={l.window} className="flex justify-between gap-5 text-mid">
              <span>
                {l.name} <span className="text-ink tabular-nums">{l.pct}%</span>
              </span>
              <span className="tabular-nums">resets {l.resets}</span>
            </p>
          ))}
        </div>
      )}
    </div>
  );
}

/** Settings' Usage: each subscription's windows as bars, with a mark where
 * even spending would be by now (T3 Code's pace line), then what the model
 * calls cost at API prices over a span, cut the way the person picks. */
function UsagePage({ usage, row, note }: { usage?: Usage; row: string; note: string }) {
  if (!usage) return <p className={`${note} px-5 py-2.5`}>Asking…</p>;
  return (
    <>
      <Section title="Limits">
        {usage.accounts.length === 0 && <p className={`${note} px-5 py-2`}>No subscription signed in here reports limits.</p>}
        {usage.accounts.map((a) => (
          <div key={a.account} className="pb-2">
            <div className="flex items-baseline justify-between gap-4 px-5 pt-2.5 text-body text-ink">
              <span>{a.name}</span>
              <span className="text-label text-mid">{[a.plan, a.stale && `as of ${clock(a.read_at)}`].filter(Boolean).join(" · ")}</span>
            </div>
            {a.windows.map((w) => (
              <LimitBar key={w.window} w={w} />
            ))}
            {a.notes.map((n, i) => (
              <p key={i} className={`${note} px-5 py-0.5`}>
                {n}
              </p>
            ))}
          </div>
        ))}
      </Section>
      <Spend note={note} />
      <Section title="Composer">
        <button type="button" role="switch" aria-checked={usage.shown} onClick={() => void showUsage(!usage.shown)} className={row}>
          Show usage in the input
          <Toggle on={usage.shown} />
        </button>
        <p className={`${note} px-5 pb-2`}>Rings by Send show the picked model's limits.</p>
      </Section>
      <CompactSection row={row} note={note} />
    </>
  );
}

/** The points a session may be compacted at, in tokens, as the picker
 * offers them; the host takes any whole number from its least up. */
const COMPACT_POINTS = [100_000, 200_000, 300_000, 500_000, 1_000_000];

const tokensLabel = (n: number) => (n >= 1_000_000 ? `${n / 1_000_000}M` : `${Math.round(n / 1000)}k`);

/** Settings' Sessions: how large a chat's session grows before its harness
 * compacts it, which sets what every turn costs. */
function CompactSection({ row, note }: { row: string; note: string }) {
  const [now, setNow] = useState<{ tokens: number; picked: boolean } | undefined>();
  const [failed, setFailed] = useState("");
  useEffect(() => {
    api("api/compact")
      .then((r) => (r.ok ? r.json() : undefined))
      .then((v) => v && setNow(v))
      .catch(() => {});
  }, []);
  const pick = async (tokens: number | null) => {
    setFailed("");
    const r = await api("api/compact", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ tokens }) }).catch(() => undefined);
    if (!r?.ok) return setFailed(r ? await r.text() : "Couldn't reach the computer.");
    const v = await api("api/compact").then((r) => r.json()).catch(() => undefined);
    if (v) setNow(v);
  };
  const points = now && !COMPACT_POINTS.includes(now.tokens) ? [...COMPACT_POINTS, now.tokens].sort((a, b) => a - b) : COMPACT_POINTS;
  return (
    <Section title="Sessions">
      <label className={row}>
        Compact sessions at
        <select
          aria-label="Compact sessions at"
          className={`${note} bg-transparent text-right`}
          value={now?.tokens ?? ""}
          disabled={!now}
          onChange={(e) => void pick(Number(e.target.value))}
        >
          {points.map((n) => (
            <option key={n} value={n}>
              {tokensLabel(n)} tokens
            </option>
          ))}
        </select>
      </label>
      {now?.picked && (
        <button type="button" className={row} onClick={() => void pick(null)}>
          Back to the default
        </button>
      )}
      <p className={`${note} px-5 pb-2`}>
        Smaller costs less each turn; the memory keeps everything.
      </p>
      {failed && <p className={`${note} px-5 pb-2 text-red-700 dark:text-red-400`}>{failed}</p>}
    </Section>
  );
}

const clock = (at: number) => new Date(at * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });

/** One window: used share as a bar, and a thin mark at how far through the
 * window it is, so a fill past the mark is spending ahead of pace. */
function LimitBar({ w }: { w: UsageWindow }) {
  const pct = Math.min(100, Math.max(0, w.pct));
  const through = w.resets_at && w.hours ? Math.min(1, Math.max(0, 1 - (w.resets_at - Date.now() / 1000) / (w.hours * 3600))) : null;
  // Five points either side of the mark still reads as on pace.
  const pace = through === null ? "" : pct > through * 100 + 5 ? " · ahead of pace" : pct < through * 100 - 5 ? " · under pace" : " · on pace";
  return (
    <div className="px-5 py-2">
      <div className="flex items-baseline justify-between gap-4 text-label">
        <span className="whitespace-nowrap text-ink">{w.name}</span>
        <span className="text-ink tabular-nums">{w.pct}%</span>
      </div>
      <div className="relative mt-1.5 h-1.5 rounded-full bg-hair">
        <div className="h-full rounded-full" style={{ width: `${pct}%`, background: fill(pct) }} />
        {through !== null && <div aria-hidden className="absolute -top-0.5 h-2.5 w-0.5 rounded-full bg-mid" style={{ left: `calc(${through * 100}% - 1px)` }} />}
      </div>
      <p className="mt-1 text-caption text-mid tabular-nums">
        resets {w.resets}
        {pace}
      </p>
    </div>
  );
}

/** One group's sums (ledger.rs `Sum`). */
type SpendSum = { key: string; name: string; calls: number; input: number; cache_read: number; cache_write: number; output: number; cost: number; unpriced: number };
type SpendPrice = { match: string; company: string; input: number; cache_read: number; cache_write: number; output: number; source: string };
/** ledger.rs `Spend`. */
type SpendReport = {
  since: number;
  total: SpendSum;
  by_caller: SpendSum[];
  by_model: SpendSum[];
  by_company: SpendSum[];
  by_harness: SpendSum[];
  by_paid: SpendSum[];
  days: { day: string; cost: number }[];
  prices_checked: string;
  prices: SpendPrice[];
};
type SpanName = "day" | "week" | "month";
type Cut = "by_caller" | "by_model" | "by_company" | "by_harness" | "by_paid";
const SPANS: [SpanName, string][] = [
  ["day", "Today"],
  ["week", "7 days"],
  ["month", "30 days"],
];
const CUTS: [Cut, string][] = [
  ["by_caller", "Who"],
  ["by_model", "Model"],
  ["by_company", "Maker"],
  ["by_harness", "Harness"],
  ["by_paid", "Paid by"],
];

const dollars = (d: number) => (d > 0 && d < 0.01 ? "<$0.01" : `$${d.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`);
const tokenCount = (n: number) => (n >= 1e9 ? `${(n / 1e9).toFixed(1)}B` : n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${Math.round(n / 1e3)}k` : `${n}`);
const sent = (s: SpendSum) => s.input + s.cache_read + s.cache_write;

/** A row of choices, one picked. */
function Choices<T extends string>({ label, all, now, pick }: { label: string; all: [T, string][]; now: T; pick: (t: T) => void }) {
  return (
    <span role="radiogroup" aria-label={label} className="flex flex-wrap gap-1">
      {all.map(([k, name]) => (
        <button key={k} type="button" role="radio" aria-checked={now === k} onClick={() => pick(k)} className={`min-h-8 rounded-full px-3 py-1 text-label ${now === k ? "bg-ink text-paper" : "text-mid hover:bg-hover hover:text-ink"}`}>
          {name}
        </button>
      ))}
    </span>
  );
}

/** What the model calls cost at API prices over a span, subscription runs
 * included, and who made them: from Libri's own ledger of every call. */
function Spend({ note }: { note: string }) {
  const [span, setSpan] = useState<SpanName>("week");
  const [cut, setCut] = useState<Cut>("by_caller");
  const [all, setAll] = useState(false);
  const [r, setR] = useState<SpendReport | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let live = true;
    setFailed(false);
    api(`api/usage/spend?span=${span}`)
      .then((x) => (x.ok ? x.json() : Promise.reject(x.status)))
      .then(
        (v: SpendReport) => live && setR(v),
        () => live && setFailed(true),
      );
    return () => void (live = false);
  }, [span]);
  const rows = r?.[cut] ?? [];
  const shown = all ? rows : rows.slice(0, 10);
  const top = Math.max(...rows.map((s) => s.cost), 0);
  const peak = Math.max(...(r?.days.map((d) => d.cost) ?? []), 0);
  return (
    <Section title="Spend">
      <div className="flex flex-wrap items-center justify-between gap-3 px-5 py-2">
        <Choices label="Span" all={SPANS} now={span} pick={setSpan} />
      </div>
      {failed && <p className={`${note} px-5 py-2`}>Couldn't read the ledger.</p>}
      {r && (
        <>
          <div className="px-5 pt-2 pb-1">
            <p className="text-heading leading-tight text-ink tabular-nums">{dollars(r.total.cost)}</p>
            <p className={note}>
              at API prices · {r.total.calls.toLocaleString()} calls · {tokenCount(sent(r.total))} in, {tokenCount(r.total.output)} out
              {r.total.unpriced > 0 && ` · ${r.total.unpriced} on unpriced models`}
            </p>
          </div>
          {span !== "day" && r.days.length > 1 && (
            <div role="img" className="flex h-10 items-end gap-0.5 px-5 py-1.5" aria-label={`Cost per day, highest ${dollars(peak)}`}>
              {r.days.map((d) => (
                <div key={d.day} title={`${d.day}: ${dollars(d.cost)}`} className="min-w-0 flex-1 rounded-[2px] bg-frame" style={{ height: `${peak ? Math.max(6, (d.cost / peak) * 100) : 6}%` }} />
              ))}
            </div>
          )}
          <div className="px-5 py-2">
            <Choices label="Cut by" all={CUTS} now={cut} pick={(c) => (setCut(c), setAll(false))} />
          </div>
          {rows.length === 0 && <p className={`${note} px-5 py-2`}>No model calls in this span.</p>}
          {shown.map((s) => (
            <div key={s.key} className="px-5 py-2">
              <div className="flex items-baseline justify-between gap-4 text-body">
                <span className="min-w-0 truncate text-ink">{s.name}</span>
                <span className="text-ink tabular-nums">{s.unpriced === s.calls ? "no price" : dollars(s.cost)}</span>
              </div>
              <div className="mt-1 h-1 rounded-full bg-hair">
                <div className="h-full rounded-full bg-frame" style={{ width: `${top ? (s.cost / top) * 100 : 0}%` }} />
              </div>
              <p className={`${note} mt-1 tabular-nums`}>
                {r.total.cost > 0 ? `${Math.round((s.cost / r.total.cost) * 100)}% · ` : ""}
                {s.calls.toLocaleString()} calls · {tokenCount(sent(s))} in ({tokenCount(s.cache_read)} cached) · {tokenCount(s.output)} out
              </p>
            </div>
          ))}
          {rows.length > shown.length && (
            <button type="button" onClick={() => setAll(true)} className="px-5 py-2 text-label text-mid hover:text-ink">
              Show all {rows.length}
            </button>
          )}
          <details className="px-5 py-2">
            <summary className={`${note} cursor-pointer`}>Prices, checked {r.prices_checked}</summary>
            <p className={`${note} pt-2`}>USD per million tokens: input · cache read · cache write · output.</p>
            {r.prices.map((p) => (
              <p key={p.match} className={`${note} pt-1.5`}>
                <span className="text-ink">{p.match}</span> · {p.input} · {p.cache_read} · {p.cache_write} · {p.output}
                <br />
                {p.source}
              </p>
            ))}
          </details>
        </>
      )}
    </Section>
  );
}

/** How long the chat remembers, in minutes, asked once `want` turns true,
 * and a way to change it: older messages and files go, oldest first. */
function useForget(want: boolean): [number | null, (m: number) => void] {
  const [minutes, setMinutes] = useState<number | null>(null);
  useEffect(() => {
    if (!want) return;
    api("api/forget")
      .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
      .then((v: { minutes: number }) => setMinutes(v.minutes), () => {});
  }, [want]);
  const pick = (m: number) => {
    const was = minutes;
    setMinutes(m);
    api("api/forget", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ minutes: m }) })
      .then((r) => r.ok || Promise.reject(r.status))
      .catch(() => setMinutes(was));
  };
  return [minutes, pick];
}

/** Minutes as the menu says them: "10 minutes", "3 hours", "1 week". */
function forgetLabel(m: number) {
  const [n, unit] = m % 10080 === 0 ? [m / 10080, "week"] : m % 1440 === 0 ? [m / 1440, "day"] : m % 60 === 0 ? [m / 60, "hour"] : [m, "minute"];
  return `${n} ${unit}${n === 1 ? "" : "s"}`;
}

type Voice = { id: string; name: string };
let voiceList: Promise<Voice[]> | null = null;

/** xAI's voices, asked once `want` turns true; empty until then or if it fails. */
function useVoices(want: boolean) {
  const [voices, setVoices] = useState<Voice[]>([]);
  useEffect(() => {
    if (!want) return;
    voiceList ??= api("api/voices").then((r) => (r.ok ? r.json() : Promise.reject(r.status)));
    voiceList.then(setVoices, () => {
      voiceList = null;
    });
  }, [want]);
  return voices;
}

/** Reads the answer aloud in the assistant's voice; pressed again, stops.
 * Absent where the assistant has no voice. */
function useSpeak() {
  const { live } = useCustom();
  const voice = useMe()?.voice;
  // The answer: the words after the last call, not those on the way.
  const text = useAuiState((s) => {
    const parts = s.message.content as readonly { type: string; text?: string }[];
    const from = parts.map((p) => p.type).lastIndexOf("tool-call") + 1;
    return parts
      .slice(from)
      .map((p) => (p.type === "text" ? (p.text ?? "") : ""))
      .join("");
  });
  const now = useReading();
  const state = now?.text === text ? now.state : "idle";
  const able = !live && !!voice && !!text.trim();
  const press = async () => {
    if (state !== "idle") return stop();
    stop();
    const done = () => {
      if (stopping !== done) return;
      stopping = null;
      reads(null);
    };
    stopping = done;
    reads({ text, state: "loading" });
    const picked = voiceChoice();
    try {
      const src = await speech(text, picked);
      if (stopping !== done) return;
      player.src = src;
      player.playbackRate = voiceSpeed();
      player.onended = done;
      // A reading cut off mid-play is not kept; the next press makes it afresh.
      player.onerror = () => {
        spoken.delete(speechId(text, picked));
        done();
      };
      await player.play();
      // Stopped while it started: the stop has set the button already.
      if (stopping === done) reads({ text, state: "playing" });
    } catch {
      // A play cut short by Stop, or by another reading, is no failure.
      if (stopping !== done) return;
      spoken.delete(speechId(text, picked));
      done();
    }
  };
  return { able, state, press, label: state === "idle" ? "Read aloud" : "Stop reading" };
}

/** Stops the reading from wherever the person has gone since starting it:
 * a place of its own over the composer, since a reply may be scrolled away. */
function ReadingStop() {
  const now = useReading();
  if (!now) return null;
  return (
    <button type="button" onClick={stop} className="absolute -top-12 right-0 z-10 flex h-9 items-center gap-2 rounded-full border border-frame bg-lift pr-3.5 pl-3 text-label text-ink shadow-lift">
      <span className={`size-2.5 rounded-[2px] bg-ink ${now.state === "loading" ? "animate-pulse" : ""}`} />
      Stop reading
    </button>
  );
}

function Speak() {
  const { able, state, press, label } = useSpeak();
  if (!able) return null;
  return (
    <button
      type="button"
      onClick={press}
      aria-label={label}
      title={label}
      className={`grid size-7 place-items-center rounded-md transition-opacity hover:bg-hover hover:text-ink max-sm:opacity-100 ${state === "idle" ? "text-soft opacity-0 group-hover:opacity-100" : "text-ink opacity-100"}`}
    >
      <svg viewBox="0 0 24 24" className={`size-[15px] ${state === "loading" ? "animate-pulse" : ""}`} fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
        {state === "playing" ? (
          <rect x="6" y="6" width="12" height="12" rx="1.5" />
        ) : (
          <>
            <path d="M11 5 6 9H2v6h4l5 4V5z" />
            <path d="M15.54 8.46a5 5 0 0 1 0 7.07M19.07 4.93a10 10 0 0 1 0 14.14" />
          </>
        )}
      </svg>
    </button>
  );
}

// Circle-arrows beside Copy: resend the question that led to this answer.
function Redraw() {
  const { live } = useCustom();
  const id = useAuiState((s) => s.message.id);
  const redo = useRedo();
  if (live || !redo.writable) return null;
  return (
    <button
      type="button"
      onClick={() => redo.redraw(id)}
      aria-label="Redraw"
      title="Redraw"
      className="grid size-7 place-items-center rounded-md text-soft opacity-0 transition-opacity group-hover:opacity-100 hover:bg-hover hover:text-ink max-sm:opacity-100"
    >
      <svg viewBox="0 0 24 24" className="size-[15px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
        <path d="M21 2v6h-6M3 22v-6h6" />
        <path d="M3.5 9a9 9 0 0 1 14.85-3.36L21 8M21 15a9 9 0 0 1-14.85 3.36L3 16" />
      </svg>
    </button>
  );
}

/** Pins the message to the Pinned place; shown as Copy is, and kept lit
 * while pinned. */
function PinThis() {
  const message = useAuiState((s) => s.message);
  const [on] = usePinsOn();
  const { pinned, toggle } = usePins();
  const { live, queued } = useCustom();
  // A turn's pieces of talk, cut by its tool calls, read as paragraphs.
  const text = message.content.flatMap((p) => (p.type === "text" && p.text.trim() ? [p.text.trim()] : [])).join("\n\n");
  if (!on || live || queued || !text) return null;
  const is = pinned(message.id);
  return (
    <button
      type="button"
      onClick={() => toggle({ id: message.id, role: message.role, text, date: message.createdAt?.getTime() })}
      aria-label={is ? "Unpin" : "Pin"}
      aria-pressed={is}
      title={is ? "Unpin" : "Pin"}
      className={`grid size-7 shrink-0 place-items-center rounded-md hover:bg-hover hover:text-ink ${is ? "text-ink" : "text-soft opacity-0 transition-opacity group-hover:opacity-100 max-sm:opacity-100"}`}
    >
      <PinIcon filled={is} />
    </button>
  );
}

/** Copies the answer's words, shown with the time on hover; on a phone, always. */
function Copy() {
  const { live } = useCustom();
  if (live) return null;
  return (
    <ActionBarPrimitive.Root>
      <ActionBarPrimitive.Copy
        aria-label="Copy"
        title="Copy"
        className="group/copy grid size-7 place-items-center rounded-md text-soft opacity-0 transition-opacity group-hover:opacity-100 hover:bg-hover hover:text-ink max-sm:opacity-100"
      >
        <svg viewBox="0 0 24 24" className="size-[15px] group-data-[copied]/copy:hidden" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
          <rect x="9" y="9" width="12" height="12" rx="2" />
          <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
        </svg>
        <svg viewBox="0 0 24 24" className="hidden size-[15px] group-data-[copied]/copy:block" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
          <path d="M20 6 9 17l-5-5" />
        </svg>
      </ActionBarPrimitive.Copy>
    </ActionBarPrimitive.Root>
  );
}

// `mcp__calendar__events` reads as "calendar · events"; Claude's own tools
// (Read, WebSearch) keep their names.
function toolName(name: string) {
  const mcp = /^mcp__(.+?)__(.+)$/.exec(name);
  return mcp ? `${mcp[1]} · ${mcp[2].replaceAll("_", " ")}` : name;
}

// Each harness has its own name for tell_agent: `mcp__libri__tell_agent` on
// Claude and Codex, its own prefix on the rest.
const isTell = (name: string) => /(^|_)tell_agent$/.test(name);

// Words between workers, either end: who, and the words. `Message from Abel`
// is the log's own row; its words are its result, and it has no other.
function message(name: string, args: Args | string | undefined, result: unknown) {
  if (/^(Message|Reply) from /.test(name)) return { label: name, words: String(result ?? ""), arrived: true };
  if (!isTell(name)) return null;
  const a = (typeof args === "object" && args ? args : {}) as Record<string, unknown>;
  return { label: a.name ? `Message to ${a.name}` : "Message", words: String(a.message ?? ""), arrived: false };
}

// The first short string among the arguments says what a call was about:
// a query, a path, a URL.
function gist(args: Args | string | undefined) {
  if (typeof args === "string") return args.split("\n")[0];
  const first = Object.values(args ?? {}).find((v) => typeof v === "string" && v.trim());
  return typeof first === "string" ? first.split("\n")[0] : "";
}

// Claude Code's tools as a person would name them, each with its glyph.
const TOOLS: Record<string, [string, Glyph]> = {
  Bash: ["Shell", "shell"],
  BashOutput: ["Shell output", "shell"],
  KillShell: ["Stop shell", "shell"],
  Read: ["Read", "file"],
  Edit: ["Edit", "pen"],
  MultiEdit: ["Edit", "pen"],
  NotebookEdit: ["Edit", "pen"],
  Write: ["Write", "pen"],
  Grep: ["Search", "find"],
  Glob: ["Find files", "find"],
  WebFetch: ["Fetch", "web"],
  WebSearch: ["Web search", "web"],
  Agent: ["Agent", "agent"],
  Task: ["Agent", "agent"],
  TodoWrite: ["Todos", "list"],
  ToolSearch: ["Tool search", "find"],
  Skill: ["Skill", "agent"],
  AskUserQuestion: ["Question", "ask"],
  // omp's own: a failed model call it tries again by itself.
  Retry: ["Retry", "brain"],
};

type Glyph = "brain" | "shell" | "file" | "pen" | "find" | "web" | "agent" | "list" | "ask" | "plug";

const GLYPHS: Record<Glyph, ReactNode> = {
  brain: (
    <>
      <path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z" />
      <path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z" />
      <path d="M15 13a4.5 4.5 0 0 1-3-4 4.5 4.5 0 0 1-3 4M17.6 6.5a3 3 0 0 0 .4-1.5M6 5a3 3 0 0 0 .4 1.5" />
    </>
  ),
  shell: <path d="m4 17 6-6-6-6M12 19h8" />,
  file: (
    <>
      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
      <path d="M14 2v6h6" />
    </>
  ),
  pen: <path d="M12 20h9M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4z" />,
  find: (
    <>
      <circle cx="11" cy="11" r="7" />
      <path d="m21 21-4.3-4.3" />
    </>
  ),
  web: (
    <>
      <circle cx="12" cy="12" r="10" />
      <path d="M2 12h20M12 2a15.3 15.3 0 0 1 0 20 15.3 15.3 0 0 1 0-20" />
    </>
  ),
  agent: <path d="m12 3 1.9 5.8L20 10.5l-6.1 1.7L12 18l-1.9-5.8L4 10.5l6.1-1.7z" />,
  list: <path d="M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01" />,
  ask: (
    <>
      <circle cx="12" cy="12" r="10" />
      <path d="M9.1 9a3 3 0 0 1 5.8 1c0 2-3 3-3 3M12 17h.01" />
    </>
  ),
  plug: <path d="M12 22v-5M9 8V2M15 8V2M18 8v5a6 6 0 0 1-12 0V8z" />,
};

const glyphOf = (name: string): Glyph => (isTell(name) || /^(Message|Reply) from /.test(name) ? "agent" : (TOOLS[name]?.[1] ?? "plug"));

const home = (path: string) => path.replace(/^\/home\/[^/]+/, "~");

// What a call was about, in a few words: the command, the file, the query.
function about(name: string, args: Args | string | undefined): string {
  if (typeof args !== "object" || args === null) return gist(args);
  const a = args as Record<string, unknown>;
  const str = (k: string) => (typeof a[k] === "string" ? (a[k] as string) : "");
  switch (name) {
    case "Bash":
      // A leading `cd somewhere &&` says nothing about what ran.
      return str("command").split("\n")[0].replace(/^cd\s+\S+\s*&&\s*/, "");
    case "Read":
    case "Edit":
    case "MultiEdit":
    case "Write":
      return home(str("file_path"));
    case "NotebookEdit":
      return home(str("notebook_path"));
    case "Grep":
      return str("pattern") + (str("path") ? `  in ${home(str("path"))}` : "");
    case "WebFetch":
      return str("url").replace(/^https?:\/\//, "");
    case "Agent":
    case "Task":
      return str("description");
    case "TodoWrite": {
      if (!Array.isArray(a.todos)) return "";
      const todos = a.todos as { status?: string }[];
      return `${todos.filter((t) => t.status === "completed").length} of ${todos.length} done`;
    }
    case "Retry":
      return str("description");
  }
  return gist(args);
}

// One quiet line per tool the model used, with the words it said on the
// way above it; a spinner until the result is back, and a click opens what
// went in and came out, drawn for the tool it was.
function ToolLine(props: ToolCallMessagePartProps) {
  const { toolName: name, args, result, isError } = props;
  const { live } = useCustom();
  return <ToolRow name={name} args={args} result={result} isError={isError} live={live} took={tookOf(props)} structured={(props.artifact as { structured?: unknown } | undefined)?.structured} />;
}

type Row = { name: string; args: Args | string | undefined; result: unknown; isError?: boolean; live?: boolean; took?: number; partial?: string; structured?: unknown };

/** Lines summed over a run's edits; null where none of its calls edited. */
const total = (each: ([number, number] | null)[]) =>
  each.reduce<[number, number] | null>((n, l) => (l ? [(n?.[0] ?? 0) + l[0], (n?.[1] ?? 0) + l[1]] : n), null);
/** Time summed over a run's calls; unknown unless every call's is known. */
const sum = (each: (number | undefined)[]) => (each.length && each.every((t) => t !== undefined) ? each.reduce<number>((n, t) => n + t!, 0) : undefined);

// Everything the model did between its words, its thinking and its calls,
// gathers into one stack. An Agent call stands on its own, as the frame of
// its report.
const byRun = (part: { type: string; toolName?: string; args?: unknown }) =>
  part.type === "reasoning" || (part.type === "tool-call" && !isAgent(part.toolName ?? ""))
    ? (["group-tools"] as const)
    : null;

type Call = { type: string; toolCallId: string; toolName: string; args: Args | string | undefined; result?: unknown; isError?: boolean; artifact?: { partial?: string; structured?: unknown } };
type Worked = Call | { type: "reasoning"; text: string };

// What a stack did, in words: "Ran 4 commands, read 2 files".
const DID: Record<string, [string, string, string]> = {
  Bash: ["ran", "command", "commands"],
  BashOutput: ["ran", "command", "commands"],
  KillShell: ["ran", "command", "commands"],
  Read: ["read", "file", "files"],
  Edit: ["edited", "file", "files"],
  MultiEdit: ["edited", "file", "files"],
  NotebookEdit: ["edited", "file", "files"],
  Write: ["wrote", "file", "files"],
  Grep: ["ran", "search", "searches"],
  Glob: ["ran", "search", "searches"],
  ToolSearch: ["ran", "search", "searches"],
  WebFetch: ["fetched", "page", "pages"],
  WebSearch: ["searched the web", "time", "times"],
  Agent: ["sent", "agent", "agents"],
  Task: ["sent", "agent", "agents"],
  AskUserQuestion: ["asked", "question", "questions"],
  TodoWrite: ["updated", "to-do list", "to-do lists"],
  Retry: ["retried", "model call", "model calls"],
};

// What a folded stack touched, short enough to stand beside its tally:
// a command by what it was for, a file by its name. The tally alone ("Ran 1
// command, read 1 file") said what kind of work, never which.
function touched(calls: Call[]): string {
  const said = new Set<string>();
  for (const c of calls) {
    const a = typeof c.args === "object" && c.args !== null ? (c.args as Record<string, unknown>) : {};
    const path = typeof a.file_path === "string" ? a.file_path : typeof a.notebook_path === "string" ? a.notebook_path : "";
    const what = path ? path.split("/").pop()! : c.toolName === "Bash" && typeof a.description === "string" && a.description ? a.description : about(c.toolName, c.args);
    if (what) said.add(what);
  }
  const shown = [...said].slice(0, 2);
  return shown.join(" · ") + (said.size > 2 ? ` +${said.size - 2}` : "");
}

// What an edit changed, in lines: those the two sides share at either end
// are context, not change, as a diff counts them.
function changed(old: string, next: string): [number, number] {
  const a = old ? old.split("\n") : [];
  const b = next ? next.split("\n") : [];
  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head++;
  let tail = 0;
  while (tail < a.length - head && tail < b.length - head && a[a.length - 1 - tail] === b[b.length - 1 - tail]) tail++;
  return [b.length - head - tail, a.length - head - tail];
}

/** Lines a call added and took out, for an edit or a write; null for any other call. */
function linesOf(name: string, args: Args | string | undefined): [number, number] | null {
  if (typeof args !== "object" || args === null) return null;
  const a = args as Record<string, unknown>;
  const str = (k: string) => (typeof a[k] === "string" ? (a[k] as string) : "");
  if (name === "Edit") return "old_string" in a || "new_string" in a ? changed(str("old_string"), str("new_string")) : null;
  if (name === "MultiEdit" && Array.isArray(a.edits))
    return (a.edits as { old_string?: string; new_string?: string }[]).reduce<[number, number]>((n, e) => {
      const [add, cut] = changed(e.old_string ?? "", e.new_string ?? "");
      return [n[0] + add, n[1] + cut];
    }, [0, 0]);
  if (name === "Write") return [str("content") ? str("content").split("\n").length : 0, 0];
  return null;
}

/** A call's running time, short: "<1s", "1.4s", "12s", "3m 5s". A time in whole
 * seconds (from hippo, which dates its rows to the second) shows no tenths. */
function spent(ms: number) {
  if (ms < 1000) return "<1s";
  if (ms % 1000 === 0 && ms < 60_000) return `${ms / 1000}s`;
  if (ms < 10_000) return `${(ms / 1000).toFixed(1)}s`;
  const s = Math.round(ms / 1000);
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
}

const tookOf = (part: unknown) => ((part as { artifact?: { took?: number } }).artifact?.took);
// A running call's output so far, which its row shows until the result.
const partialOf = (part: unknown) => ((part as { artifact?: { partial?: string; structured?: unknown } }).artifact?.partial);

/** The plain measures at a row's end: lines in and out, then time taken.
 * Quiet, figures only; the colour of a diff's sides stays in the diff. */
function Measures({ lines, took }: { lines: [number, number] | null; took?: number }) {
  if (!lines && took === undefined) return null;
  return (
    <span className="shrink-0 whitespace-nowrap font-mono text-caption tabular-nums text-soft [font-variant-ligatures:none]">
      {lines && (
        <>
          <span className="text-emerald-700 dark:text-emerald-400">+{lines[0]}</span> <span className="text-red-700 dark:text-red-400">−{lines[1]}</span>
        </>
      )}
      {lines && took !== undefined && " · "}
      {took !== undefined && spent(took)}
    </span>
  );
}

function did(calls: Call[], thought: boolean): string {
  const tally = new Map<string, [string, string, string, number]>();
  for (const c of calls) {
    const [verb, one, many] = DID[c.toolName] ?? (isTell(c.toolName) ? ["sent", "message", "messages"] : ["used", "tool", "tools"]);
    const key = `${verb} ${many}`;
    const t = tally.get(key) ?? [verb, one, many, 0];
    t[3]++;
    tally.set(key, t);
  }
  const said = [...(thought ? ["thought"] : []), ...[...tally.values()].map(([verb, one, many, n]) => `${verb} ${n} ${n === 1 ? one : many}`)].join(", ");
  return said.charAt(0).toUpperCase() + said.slice(1);
}

// A run of thinking and calls as one line that opens into their rows, each
// of which opens in turn. While the run goes on, the thinking or call under
// way shows below the line, and each one done folds up into the line.
// MCP Apps cards, under the calls that drew them and outside their fold,
// so a folded stack never hides one. A call this page heard opens its card
// at once; one from the history (a card is never kept) waits behind a pill,
// and opens with the call's kept text, or calls a read-only tool again for
// what it says now.
const CardFrame = lazy(() => import("./card"));
const BuiltinFrame = lazy(() => import("./card").then((m) => ({ default: m.BuiltinFrame })));

function Cards({ indices }: { indices: readonly number[] }) {
  const parts = useAuiState((s) => s.message.parts) as readonly Worked[];
  const calls = indices.map((i) => parts[i]).filter((w): w is Call => w?.type === "tool-call");
  return calls.map((c) => <CallCard key={c.toolCallId} call={c} />);
}

// A card shows under its call only when showing it is the point: the model
// asked (`show_card`), or the tool's card is what it is for (`opens`) and
// the person's own turn made the call. A lookup the model made for itself,
// and any call of a turn a routine, a watch, a message or a worker's report
// started, is its row alone; the row's details open its card on asking.
function CallCard({ call }: { call: Call }) {
  const drawn = cardCall(call.toolName, call.args);
  const card = useCardTool(drawn.name);
  const { outside, woke } = useCustom();
  const heard = !!((call as { artifact?: { heard?: boolean } }).artifact?.heard);
  if (!card || (heard && call.result === undefined)) return null;
  if (drawn.shown) return <CallCardOf card={card} call={call} input={drawn.input} fresh />;
  if (!card.opens || outside || woke !== undefined) return null;
  return <CallCardOf card={card} call={call} input={drawn.input} />;
}

function CallCardOf({ card, call, input, fresh = false }: { card: CardTool; call: Call; input: Record<string, unknown>; fresh?: boolean }) {
  const aui = useAui();
  const kept = ((call as { artifact?: unknown }).artifact ?? {}) as { structured?: Outcome; heard?: boolean };
  const told: Outcome = { ...(kept.structured ?? { content: [{ type: "text", text: String(call.result ?? "") }] }), ...(call.isError ? { isError: true } : {}) };
  // A heard call's whole result may land a moment after its text: the
  // card takes whichever it has when it starts.
  const [opened, setOutcome] = useState<Outcome | null>(null);
  const outcome = kept.heard && !fresh ? told : opened;
  const [busy, setBusy] = useState(false);
  // What `show_card` names is drawn from a fresh call of it, as it said;
  // only a tool that reads is called again.
  useEffect(() => {
    if (!fresh || opened) return;
    if ((card.definition.annotations as { readOnlyHint?: boolean } | undefined)?.readOnlyHint !== true) {
      setOutcome({ content: [{ type: "text", text: `${card.tool} changes things, so its card isn't called up again here.` }], isError: true });
      return;
    }
    let on = true;
    api("api/apps/call", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ server: card.server, tool: card.tool, arguments: input }) })
      .then(async (r) => on && setOutcome(r.ok ? await r.json() : { content: [{ type: "text", text: (await r.text()) || `${r.status}` }], isError: true }))
      .catch(() => on && setOutcome({ content: [{ type: "text", text: "The host couldn't be reached." }], isError: true }));
    return () => {
      on = false;
    };
    // Called once, as the card first draws.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fresh]);
  if (fresh && !outcome) return <div className="my-2.5 h-10 animate-pulse rounded-xl border border-hair bg-lift" aria-label="Opening the card" />;
  if (outcome) {
    return (
      <Suspense fallback={<div className="my-2.5 h-10 animate-pulse rounded-xl border border-hair bg-lift" aria-label="Loading the card" />}>
        <CardFrame card={card} callId={call.toolCallId} input={input} outcome={outcome} say={(text) => aui.thread().composer().setText(text)} />
      </Suspense>
    );
  }
  const show = async () => {
    const reads = (card.definition.annotations as { readOnlyHint?: boolean } | undefined)?.readOnlyHint === true;
    if (!reads) return setOutcome(told);
    setBusy(true);
    const r = await api("api/apps/call", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ server: card.server, tool: card.tool, arguments: input }) }).catch(() => null);
    setBusy(false);
    setOutcome(r?.ok ? await r.json() : told);
  };
  return (
    <button type="button" onClick={() => void show()} disabled={busy} className="not-prose my-1.5 inline-flex items-center gap-2 rounded-full border border-hair px-3 py-1 text-caption leading-4 text-mid hover:bg-hover disabled:opacity-60">
      <svg viewBox="0 0 24 24" className="size-3.5" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
        <rect x="3" y="4" width="18" height="16" rx="2" />
        <path d="M3 9h18" />
      </svg>
      {busy ? "Opening…" : `Show the ${card.server} card`}
    </button>
  );
}

function ToolStack({ indices }: { indices: readonly number[] }) {
  const { live, musing } = useCustom();
  const parts = useAuiState((s) => s.message.parts) as readonly Worked[];
  return <Stack items={indices.map((i) => parts[i])} live={live} musing={musing} />;
}

// The stack itself, for the chat's messages and for a thread's log alike
// (Temp, a worker's page), so every thread folds its work the same way.
function Stack({ items: all, live, musing }: { items: readonly Worked[]; live: boolean; musing?: string }) {
  const detail = useChatDetail();
  const [open, setOpen] = useState(false);
  const items = all.filter((w) => w?.type === "reasoning" || (w?.type === "tool-call" && !offerShown((w as Call).toolName, (w as Call).result, (w as Call).isError)));
  if (!items.length) return null;
  const row = (w: Worked, running: boolean, n: number) =>
    w.type === "reasoning" ? (
      <Musing key={`thought-${n}`} text={(w as { text: string }).text} now={running && musing === (w as { text: string }).text} />
    ) : (
      <ToolRow key={(w as Call).toolCallId} name={(w as Call).toolName} args={(w as Call).args} result={(w as Call).result} isError={(w as Call).isError} live={running} took={tookOf(w)} partial={partialOf(w)} structured={(w as { artifact?: { structured?: unknown } }).artifact?.structured} />
    );
  if (detail === "full") return <div data-chat-detail="full" className="space-y-2">{items.map((w, n) => row(w, live, n))}</div>;
  if (detail === "simple") return null;
  if (items.length === 1) return row(items[0], live, 0);
  const going = (w: Worked) => live && (w.type === "reasoning" ? musing === (w as { text: string }).text : (w as Call).result === undefined);
  const busy = items.filter(going);
  const done = items.filter((w) => !busy.includes(w));
  const calls = done.filter((w): w is Call => w.type === "tool-call");
  const failed = calls.filter((c) => c.isError).length;
  const glyphs: Glyph[] = [...(calls.length < done.length ? ["brain" as const] : []), ...calls.map((c) => glyphOf(c.toolName))];
  return (
    <div className="not-prose my-2.5 text-label">
      {done.length > 0 && (
        <button
          type="button"
          onClick={(e) => {
            const at = e.currentTarget;
            if (open) hold(at);
            else reveal(at.parentElement);
            setOpen(!open);
          }}
          aria-expanded={open}
          className="group/stack -mx-1.5 flex w-[calc(100%+0.75rem)] items-center gap-2 rounded-md px-1.5 py-[3px] text-left hover:bg-hover/50"
        >
          {/* One glyph, the run's first kind of work, in the column a lone
              call's row keeps: overlapping chips read as a cluster to decode. */}
          <svg viewBox="0 0 24 24" className="size-3.5 shrink-0 text-soft" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
            {GLYPHS[glyphs[0]]}
          </svg>
          <span key={done.length} className={`${live ? "absorb " : ""}shrink-0 font-medium text-ink/85`}>
            {did(calls, calls.length < done.length)}
          </span>
          {failed > 0 && <span className="shrink-0 text-caption leading-4 text-accent">{failed} failed</span>}
          {/* The chevron follows the words, where the eye already is; at
              the column's far edge it sat half a page from what it opens. */}
          <svg viewBox="0 0 24 24" className="-ml-1 size-3.5 shrink-0 text-soft transition group-aria-expanded/stack:rotate-90" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
            <path d="m9 18 6-6-6-6" />
          </svg>
          <span className="min-w-0 truncate text-soft">{touched(calls)}</span>
          <span className="ml-auto shrink-0 pl-2">
            <Measures lines={total(calls.map((c) => linesOf(c.toolName, c.args)))} took={sum(calls.map(tookOf))} />
          </span>
        </button>
      )}
      {open && <div className="rise mt-1 ml-[10px] border-l border-hair pl-3.5">{done.map((w, n) => row(w, false, n))}</div>}
      {busy.length > 0 && (
        <div className={done.length ? "mt-1 ml-[10px] border-l border-hair pl-3.5" : ""}>
          {busy.map((w, n) => (
            <div key={w.type === "reasoning" ? `thinking-${n}` : (w as Call).toolCallId} className="rise">
              {row(w, true, n)}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function ToolRow({ name, args, result, isError, live, took, partial, structured }: Row) {
  const detail = useChatDetail();
  const [open, setOpen] = useState(detail === "full");
  useEffect(() => setOpen(detail === "full"), [detail]);
  const done = result !== undefined;
  // While it runs, the last line it printed shows under the row, as a
  // terminal shows a command's; open, the row shows all of it so far.
  const printing = !done && live && partial ? partial : undefined;
  const lastLine = printing?.trimEnd().split("\n").pop() ?? "";
  const said = message(name, args, result);
  const label = said?.label ?? TOOLS[name]?.[0] ?? toolName(name);
  return (
    <details open={open} onToggle={(e) => setOpen(e.currentTarget.open)} className="group/tool not-prose text-label [:not(details)+&]:mt-2 [&:has(+:not(details))]:mb-3 [&:last-child]:mb-3">
      <summary
        onClick={(e) => {
          // The person's tap: what it opens comes into view, and what it
          // closes leaves the row where it stood.
          const row = e.currentTarget;
          if (row.parentElement instanceof HTMLDetailsElement && row.parentElement.open) hold(row);
          else reveal(row.parentElement);
        }}
        className={`-mx-1.5 flex cursor-pointer list-none items-center gap-x-2 rounded-md px-1.5 py-[3px] marker:hidden hover:bg-hover/50 [&::-webkit-details-marker]:hidden ${lastLine ? "flex-wrap" : ""}`}>
        {!done && live ? (
          <span className="size-3.5 shrink-0 animate-spin rounded-full border-[1.5px] border-hair border-t-active" />
        ) : (
          <svg viewBox="0 0 24 24" className={`size-3.5 shrink-0 ${isError ? "text-accent" : "text-soft"}`} fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
            {GLYPHS[glyphOf(name)]}
          </svg>
        )}
        {/* One coloured mark a row: a failed call's glyph carries it. */}
        <span className="shrink-0 font-medium text-ink/80">{label}</span>
        <span className={`min-w-0 truncate font-mono text-caption text-soft [font-variant-ligatures:none] ${lastLine ? "grow basis-0" : ""}`}>{said ? firstLine(said.words) : about(name, args)}</span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5 pl-2">
          <Measures lines={done || !live ? nativeChanges(structured) ?? linesOf(name, args) : null} took={took} />
          <svg viewBox="0 0 24 24" className="size-3.5 shrink-0 text-soft opacity-0 transition group-open/tool:rotate-90 group-hover/tool:opacity-100 group-open/tool:opacity-100" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
            <path d="m9 18 6-6-6-6" />
          </svg>
        </span>
        {/* Inside the summary: a closed fold draws nothing else. */}
        {lastLine && <span className="basis-full truncate pl-[22px] font-mono text-caption leading-4 text-soft [font-variant-ligatures:none] group-open/tool:hidden">{lastLine}</span>}
      </summary>
      <div className="mt-1 mb-2.5 ml-[6px] flex flex-col gap-2 border-l border-hair pl-4">
        {said ? <div className="md message-words rounded-lg border border-hair bg-lift px-3 py-2 text-control">{md(said.words)}</div> : <ToolBody name={name} args={args} />}
        <ToolDetails result={structured} />
        {said?.arrived ? null : done ? <Output text={String(result)} error={isError} /> : printing ? <Output text={printing} /> : <span className="text-caption leading-4 text-soft">{live ? "Running…" : "No result."}</span>}
        {done && <RowCard name={name} args={args} result={result} isError={isError} structured={structured} />}
      </div>
    </details>
  );
}

/** A call's card on asking, in its row's details, for a call whose card
 * doesn't open by itself (CallCard): drawn from what the call gave back. */
function RowCard({ name, args, result, isError, structured }: { name: string; args: Args | string | undefined; result: unknown; isError?: boolean; structured?: unknown }) {
  const aui = useAui();
  const drawn = cardCall(name, args);
  const card = useCardTool(drawn.name);
  const [open, setOpen] = useState(false);
  if (!card || drawn.shown) return null;
  if (!open)
    return (
      <button
        type="button"
        onClick={(e) => {
          reveal(e.currentTarget.parentElement);
          setOpen(true);
        }}
        className="inline-flex w-fit items-center gap-2 rounded-full border border-hair px-3 py-1 text-caption leading-4 text-mid hover:bg-hover"
      >
        <svg viewBox="0 0 24 24" className="size-3.5" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
          <rect x="3" y="4" width="18" height="16" rx="2" />
          <path d="M3 9h18" />
        </svg>
        Open its card
      </button>
    );
  const told: Outcome = { ...((structured as Outcome | undefined) ?? { content: [{ type: "text", text: String(result ?? "") }] }), ...(isError ? { isError: true } : {}) };
  return (
    <Suspense fallback={<div className="h-10 animate-pulse rounded-xl border border-hair bg-lift" aria-label="Loading the card" />}>
      <CardFrame card={card} callId={`row-${drawn.name}`} input={drawn.input} outcome={told} say={(text) => aui.thread().composer().setText(text)} />
    </Suspense>
  );
}

// What a tool took or gave, which can run to any length. No ligatures: code
// shows `--` and `===` as typed.
const BLOCK = "max-h-72 overflow-auto rounded-lg border border-hair bg-lift px-3 py-2 font-mono text-caption leading-relaxed [font-variant-ligatures:none] whitespace-pre-wrap [overflow-wrap:anywhere]";


// What went in: a command as typed, an edit as a diff, a file as written.
function ToolBody({ name, args }: { name: string; args: Args | string | undefined }) {
  if (typeof args !== "object" || args === null) return args ? <pre className={BLOCK}>{args}</pre> : null;
  const a = args as Record<string, unknown>;
  const str = (k: string) => (typeof a[k] === "string" ? (a[k] as string) : "");
  const path = <div className="font-mono text-caption text-mid [overflow-wrap:anywhere]">{home(str("file_path"))}</div>;
  switch (name) {
    case "Bash":
      return (
        <>
          {str("description") && <div className="text-caption leading-4 text-soft">{str("description")}</div>}
          <pre className={BLOCK}>
            <span className="text-soft select-none">$ </span>
            {str("command")}
          </pre>
        </>
      );
    case "Edit":
      if (!("old_string" in a) && !("new_string" in a)) return <pre className={BLOCK}>{JSON.stringify(a, null, 2)}</pre>;
      return (
        <>
          {path}
          <Diff old={str("old_string")} next={str("new_string")} />
        </>
      );
    case "MultiEdit":
      return (
        <>
          {path}
          {(Array.isArray(a.edits) ? (a.edits as { old_string?: string; new_string?: string }[]) : []).map((e, k) => (
            <Diff key={k} old={e.old_string ?? ""} next={e.new_string ?? ""} />
          ))}
        </>
      );
    case "Write":
      return (
        <>
          {path}
          <pre className={BLOCK}>{str("content")}</pre>
        </>
      );
    case "TodoWrite":
      return Array.isArray(a.todos) ? <Todos todos={a.todos as Todo[]} /> : null;
    case "Read":
      return a.offset || a.limit ? (
        <div className="font-mono text-caption text-mid">
          {home(str("file_path"))}
          {a.offset ? ` from line ${String(a.offset)}` : ""}
          {a.limit ? `, ${String(a.limit)} lines` : ""}
        </div>
      ) : null;
  }
  if (Object.keys(a).length === 0) return null;
  return <pre className={BLOCK}>{JSON.stringify(a, null, 2)}</pre>;
}

type Todo = { content?: string; status?: string; phase?: string };

// A to-do list as a checklist: each item's state by its mark, done ones
// struck through, under its phase's name when the list has phases.
function Todos({ todos }: { todos: Todo[] }) {
  const mark: Record<string, string> = { completed: "✓", in_progress: "◐", abandoned: "–", blocked: "!" };
  let phase: string | undefined;
  return (
    <ul className="flex flex-col gap-1 text-control">
      {todos.flatMap((t, k) => {
        const head = t.phase && t.phase !== phase ? <li key={`p${k}`} className="pt-1 text-caption font-medium text-soft first:pt-0">{t.phase}</li> : null;
        phase = t.phase;
        const done = t.status === "completed" || t.status === "abandoned";
        const item = (
          <li key={k} className="flex items-baseline gap-2">
            <span aria-label={t.status ?? "pending"} className={`w-3 shrink-0 text-center font-mono text-caption ${t.status === "in_progress" ? "text-active" : "text-soft"}`}>{mark[t.status ?? ""] ?? "○"}</span>
            <span className={done ? "text-soft line-through" : t.status === "in_progress" ? "text-ink" : "text-mid"}>{t.content}</span>
          </li>
        );
        return head ? [head, item] : [item];
      })}
    </ul>
  );
}

/** Count known before/after snapshots rather than opaque edit instructions. */
function nativeChanges(result: unknown): [number, number] | null {
  const details = (result as { details?: { oldText?: string; newText?: string; perFileResults?: { oldText?: string; newText?: string }[] } } | undefined)?.details;
  if (!details) return null;
  const files = Array.isArray(details.perFileResults) ? details.perFileResults : [details];
  return total(files.map((file) => typeof file.oldText === "string" && typeof file.newText === "string" ? changed(file.oldText, file.newText) : null));
}

/** Native omp results carry diffs and images outside their text output. */
function ToolDetails({ result }: { result?: unknown }) {
  if (!result || typeof result !== "object") return null;
  type FileDiff = { path?: string; diff?: string; oldText?: string; newText?: string; diagnostics?: unknown };
  const r = result as { details?: FileDiff & { perFileResults?: FileDiff[] }; content?: { type?: string; mimeType?: string; data?: string; file?: string }[] };
  const files = Array.isArray(r.details?.perFileResults) ? r.details.perFileResults : r.details ? [r.details] : [];
  const images = (Array.isArray(r.content) ? r.content : []).filter((c) => c?.type === "image" && /^image\/(png|jpeg|gif|webp)$/.test(c.mimeType ?? "") && (typeof c.data === "string" || typeof c.file === "string"));
  return <>
    {files.map((file, i) => <div key={i} data-tool-diff className="min-w-0 space-y-1">
      {file.path && <div className="break-all font-mono text-caption text-mid">{file.path}</div>}
      {file.diff ? <pre className={BLOCK}>{file.diff.split("\n").map((line, j) => <span key={j} className={`block ${/^\s*\+/.test(line) ? "bg-emerald-500/12" : /^\s*-/.test(line) ? "bg-red-500/10" : ""}`}>{line || " "}</span>)}</pre> : typeof file.oldText === "string" && typeof file.newText === "string" ? <Diff old={file.oldText} next={file.newText} /> : null}
      {file.diagnostics != null && <Output text={typeof file.diagnostics === "string" ? file.diagnostics : JSON.stringify(file.diagnostics, null, 2)} />}
    </div>)}
    {images.map((image, i) => <img key={i} src={image.file ? apiUrl(`api/file/${encodeURIComponent(image.file)}`, true) : `data:${image.mimeType};base64,${image.data}`} alt="Tool image" loading="lazy" className="max-h-80 max-w-full rounded-lg object-contain" />)}
  </>;
}

// An edit's lines out and in, as a diff reads.
function Diff({ old, next }: { old: string; next: string }) {
  const line = (sign: string, tint: string) => (l: string, k: number) => (
    <div key={sign + k} className={`px-3 ${tint}`}>
      <span className="text-soft select-none">{sign} </span>
      {l}
    </div>
  );
  return (
    <pre className="max-h-72 overflow-auto rounded-lg border border-hair bg-lift py-2 font-mono text-caption leading-relaxed whitespace-pre-wrap [overflow-wrap:anywhere]">
      {old && old.split("\n").map(line("−", "bg-red-500/10"))}
      {next && next.split("\n").map(line("+", "bg-emerald-500/12"))}
    </pre>
  );
}

// What came out, cut by the server to what a page can hold.
function Output({ text, error }: { text: string; error?: boolean }) {
  if (!text.trim()) return <span className="text-caption leading-4 text-soft">No output.</span>;
  return <pre className={`${BLOCK} ${error ? "border-accent/40 text-accent" : "text-mid"}`}>{text}</pre>;
}

// A subagent's call draws as its frame; the rest as a ToolLine.
function Tool(props: ToolCallMessagePartProps) {
  const detail = useChatDetail();
  if (detail === "simple") return null;
  if (offerShown(props.toolName, props.result, props.isError)) return null;
  if (isAgent(props.toolName)) return <AgentFrame {...props} />;
  return <ToolLine {...props} />;
}

const Chevron = ({ left = false }: { left?: boolean }) => (
  <svg viewBox="0 0 24 24" className="size-[18px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
    <path d={left ? "m15 18-6-6 6-6" : "m9 18 6-6-6-6"} />
  </svg>
);

// What Libri notes beside the talk: quiet, in the serif. Alerts land
// here with whole log lines, so it breaks anywhere too.
function Note() {
  const text = useAuiState((s) => s.message.content.map((p) => (p.type === "text" ? p.text : "")).join(""));
  // A phone folds any "(Word) " note to one line beside a hairline rule, as
  // Android's Note does; a wide screen folds the house's own kinds.
  const wide = useWide();
  const fed = (wide ? /^\((routine|task|alert|message|seat)\) / : /^\((\w+)\) /).exec(text);
  const style = "font-serif text-control leading-relaxed whitespace-pre-wrap text-mid italic [overflow-wrap:anywhere]";
  const said = notice(text);
  const { rise, onAnimationEnd } = useArrival();
  if (said != null) {
    return (
      <MessagePrimitive.Root onAnimationEnd={onAnimationEnd} className={`${rise} rounded-lg bg-accent/8 px-3 py-2 text-control leading-relaxed whitespace-pre-wrap text-mid [overflow-wrap:anywhere]`}>
        {said}
      </MessagePrimitive.Root>
    );
  }
  if (!fed) {
    return (
      <MessagePrimitive.Root onAnimationEnd={onAnimationEnd} className={`${rise} ${style}`}>
        <MessagePrimitive.Parts />
      </MessagePrimitive.Root>
    );
  }
  // What the house fed Libri (routine prompts, alert logs, passed-on
  // messages) is long and not meant for reading; its answer follows as
  // talk, so this folds to one line that opens on a click.
  const body = text.slice(fed[0].length);
  const label = { routine: "Routine", task: "Task", alert: "Alert", message: "Message" }[fed[1]] ?? fed[1].charAt(0).toUpperCase() + fed[1].slice(1);
  const { outside } = useCustom();
  return (
    <MessagePrimitive.Root onAnimationEnd={onAnimationEnd} className={`${rise} ${style} ${outside ? OUTSIDE : wide ? "" : "border-l-2 border-hair pl-3"}`}>
      <details className="group">
        <summary className="cursor-pointer list-none truncate marker:hidden [&::-webkit-details-marker]:hidden">
          <span className="not-italic">{label}</span>
          <span className="group-open:hidden"> · {body.split("\n")[0]}</span>
        </summary>
        <div className="mt-1">{body}</div>
      </details>
    </MessagePrimitive.Root>
  );
}

// What the house feeds the seat (routines, watches, alerts, messages
// passed on) lands in the chat as it comes, its note and the
// answer after it keeping a faint rule beside them: they came from outside
// the talk.
const NEWS = /^\((routine|watch|alert|message)\) /;
const OUTSIDE = "border-l-2 border-hair pl-3";
// A reply's words on a faint fill the column's width, so replies sent one
// after another read as separate messages; calls and cards between them
// keep their own frames on the paper. Unframed, unlike the person's.
// The same at every width, and on Android (Talk.kt Said).
const SAID = "said rounded-2xl bg-said px-4 py-3 not-last:mb-2.5 not-first:mt-2.5";

// Libri's own word about itself (a restart, its memory going and coming
// back), told apart from the model's replies. Notices kept before they
// carried "(libri) " were saved as replies in parentheses; they read the same.
const NOTICE = /^\(libri\) ([\s\S]*)$|^\(((?:Libri restarted|My memory is) [\s\S]*)\)$/;
const notice = (text: string) => {
  const n = NOTICE.exec(text.trim());
  return n ? (n[1] ?? n[2]) : null;
};

function markOutside(messages: Msg[]) {
  const out: Msg[] = [];
  for (let i = 0; i < messages.length; i++) {
    const m = messages[i];
    if (m.role !== "user" && notice(m.text) != null) {
      out.push({ ...m, role: "system" });
      continue;
    }
    if (m.role !== "system" || !NEWS.test(m.text)) {
      out.push(m);
      continue;
    }
    out.push({ ...m, outside: true });
    while (messages[i + 1]?.role === "assistant") out.push({ ...messages[++i], outside: true });
  }
  return out;
}


function useMedia(query: string) {
  const [on, setOn] = useState(() => matchMedia(query).matches);
  useEffect(() => {
    const m = matchMedia(query);
    const change = () => setOn(m.matches);
    m.addEventListener("change", change);
    return () => m.removeEventListener("change", change);
  }, [query]);
  return on;
}

// Whether the window is wide enough for a pane beside the talk.
export const useWide = () => useMedia("(min-width: 1024px)");


// A subagent's name: the Agent call's description, or a Libri agent's own.
const nameOf = (t: Task) => t.title || t.name;

// The Agent call in the talk: a frame named on its edge, holding the
// report; its work stays on the agent's own page.
function AgentFrame({ toolCallId, args, result, isError, artifact }: ToolCallMessagePartProps) {
  const { tasks, open } = useContext(AgentCtx);
  const { live } = useCustom();
  const { description } = (args ?? {}) as { description?: string };
  const { agent, report } = (artifact ?? {}) as { agent?: string; report?: string };
  const task = tasks.find((t) => t.name === toolCallId);
  const out = typeof result === "string" ? result : result === undefined ? undefined : JSON.stringify(result);
  const launched = !!out?.startsWith("Async agent launched");
  // Run in the foreground, the call's result is the report itself.
  const said = report ?? (out !== undefined && !launched && !isError && !POINTER.test(out) ? out : undefined);
  const working = task ? !task.done : live && said === undefined && !isError;
  const name = description || "Agent";
  // The model passes the report on in its own words right after, so a
  // long one shows its start here and opens whole on a press.
  const [whole, setWhole] = useState(false);
  const [long, setLong] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const b = box.current;
    if (b && !whole) setLong(b.scrollHeight > b.clientHeight + 4);
  }, [said, whole]);
  return (
    <div data-agent-progress={working ? "running" : "done"} className="relative my-5 rounded-xl border-[1.25px] border-edge first:mt-2">
      <span className="absolute -top-[0.75em] left-3.5 flex items-center gap-1.5 bg-paper px-1.5 text-caption leading-4 font-semibold tracking-wide text-mid uppercase">
        {working && <span className="size-1.5 animate-pulse rounded-full bg-active" />}
        {name}
      </span>
      <div className="px-5 pt-3.5 pb-1 text-control">
        {said ? (
          <div
            ref={box}
            onClick={() => long && !whole && setWhole(true)}
            className={whole ? "" : `max-h-[8.5em] overflow-hidden ${long ? "cursor-pointer [mask-image:linear-gradient(to_bottom,black_55%,transparent)]" : ""}`}
          >
            {md(said)}
          </div>
        ) : isError ? (
          <p className="text-accent [overflow-wrap:anywhere]">{out}</p>
        ) : (
          <p className="flex items-center gap-2 py-1 text-mid">
            {working && <span className="size-3 shrink-0 animate-spin rounded-full border-[1.5px] border-hair border-t-active" />}
            {working ? "Working" : "No report came back."}
          </p>
        )}
      </div>
      <div className="flex justify-end gap-1 px-3 pb-2">
        {long && (
          <button type="button" onClick={() => setWhole(!whole)} className="rounded-full px-2.5 py-1 text-caption text-soft hover:bg-hover hover:text-ink">
            {whole ? "Less" : "Whole report"}
          </button>
        )}
        <button type="button" onClick={() => open(toolCallId, agent, name)} className="rounded-full px-2.5 py-1 text-caption text-soft hover:bg-hover hover:text-ink">
          Its work ›
        </button>
      </div>
    </div>
  );
}

// The log's entries, each with the result the page heard since, and what
// the page heard that the log hasn't got. Alike entries pair off in order,
// so a call made twice stays twice.
function withHeard(log: Task["tools"], heard: Task["tools"]) {
  const key = (t: Task["tools"][number]) => JSON.stringify([t.name, t.told ?? t.input]);
  const rest = [...heard];
  const tools = log.map((t) => {
    const i = rest.findIndex((h) => key(h) === key(t));
    if (i < 0) return t;
    const [h] = rest.splice(i, 1);
    return t.result === undefined ? { ...t, result: h.result, error: h.error } : t;
  });
  return { tools, rest };
}

// What the page heard goes on from the log it opened with, less what the
// log already has.
function merged(kept: Pick<Task, "text" | "tools" | "words"> | null | undefined, task: Task | undefined) {
  const { tools, rest: heard } = withHeard(kept?.tools ?? [], task?.tools ?? []);
  // The stream keeps a run's words into the next, so what's heard may lead
  // with the log's latest or end on it; read twice, they'd stand until the
  // log came again, so only what follows them is new.
  const unlogged = (words = "") => {
    const last = [...(kept?.tools ?? []).map((t) => t.before), kept?.words].filter(Boolean).at(-1)?.trim();
    const w = words.trim();
    if (!last || !w.includes(last)) return w;
    return w.endsWith(last) ? "" : w.startsWith(last) ? w.slice(last.length).trim() : w;
  };
  return !kept
    ? task
    : {
        ...task,
        ...kept,
        tools: heard.length ? [...tools, { ...heard[0], before: [kept.words, unlogged(heard[0].before)].filter(Boolean).join("\n\n") || undefined }, ...heard.slice(1)] : tools,
        words: unlogged(task?.words) || (heard.length ? "" : kept.words),
      };
}

// A thread's words and calls in order, a worker's or a scratchpad's.
// A thread opens at the latest, as a chat does, and stays there while the
// work grows unless scrolled up to read. An opened fold comes whole into view
// at its nearest edge, and one already in view moves nothing.
// The dock above a composer. The tray and the text box are capped by the
// height the talk and the dock share, not the window's, which also holds the
// header and the bar.
function useDock() {
  const dock = useRef<HTMLDivElement>(null);
  const column = useRef<HTMLDivElement>(null);
  // The dock's height, as --dock on the column: the chat's talk runs on
  // under its floating dock and leaves that much room at its foot. A dock
  // growing at the foot keeps the foot in view by useFootHold, which
  // watches the viewport and that room as well as the talk.
  useEffect(() => {
    const el = dock.current;
    const c = column.current;
    if (!el || !c) return;
    const o = new ResizeObserver(() => c.style.setProperty("--dock", `${el.offsetHeight}px`));
    o.observe(el);
    return () => o.disconnect();
  }, []);
  useEffect(() => {
    const el = column.current;
    if (!el) return;
    const o = new ResizeObserver(() => el.style.setProperty("--room", `${el.clientHeight}px`));
    o.observe(el);
    return () => o.disconnect();
  }, []);
  return { dock, column };
}

/** A thread's log (a worker's, Temp's, a named memory's chat) as the chat's
 * messages, so every chat draws through ChatView: the person's words, and a
 * worker's report, each a message of their own; the words a call came after
 * a message before it, as the chat's are (convert); and the calls between
 * them, with the words that end the run, the answer they made. */
function logMessages(work: Pick<Task, "tools" | "words" | "thinking" | "metrics"> | undefined, { brief, queued = [], live }: { brief?: string; queued?: string[]; live: boolean }): Msg[] {
  const out: Msg[] = brief ? [{ id: "brief", role: "user", text: brief }] : [];
  let answer: Msg | undefined;
  for (const t of work?.tools ?? []) {
    if (t.before) {
      // Words just before the person's are the reply that ended a round, which
      // Simple keeps; words before a call are the run's progress.
      out.push({ id: `before-${t.id}`, role: "assistant", text: t.before, progress: t.told === undefined });
      answer = undefined;
    }
    const report = t.told === undefined ? null : reportOf(t.told);
    if (t.told !== undefined || t.file) {
      answer = undefined;
      const file = t.file ? apiUrl(`api/file/${encodeURIComponent(t.file)}`, true) : undefined;
      out.push(
        report
          ? { id: t.id, role: "assistant", text: `**${report.heading}**\n\n${report.body}`, outside: true }
          : t.told !== undefined
            ? { id: t.id, role: "user", text: t.told }
            : /\.(jpe?g|png|gif|webp)$/i.test(t.file!)
              ? { id: t.id, role: "assistant", text: "", picture: file }
              : { id: t.id, role: "assistant", text: "", file: t.file },
      );
      continue;
    }
    if (!answer) out.push((answer = { id: `run-${t.id}`, role: "assistant", text: "", tools: [] }));
    answer.tools!.push(t);
  }
  const words = work?.words?.trim() ?? "";
  const thinking = work?.thinking?.trim() ? work.thinking : undefined;
  // The run's words and thinking go on the answer its calls make, or one of
  // their own; a turn just begun is an empty answer, live, for the dots.
  if (!answer && (words || thinking || live)) out.push((answer = { id: `words-${out.at(-1)?.id ?? "start"}`, role: "assistant", text: "" }));
  if (answer) Object.assign(answer, { text: words, thinking, live, metrics: work?.metrics });
  return [...out, ...queued.map((q, i): Msg => ({ id: `queued-${i}`, role: "user", text: q, queued: true }))];
}

/** A thread's head: its name, and on its right what acts on it. */
/** A chat column's head on a wide screen, as the main chat's (ChatPane's
 * head): a bar docked over the talk in the page's paper, fading into it
 * rather than ending in a rule, the column's name at its left and its
 * controls in one floating pill at its right. Its ChatView takes DOCKED, so
 * the talk starts below it and scrolls up under the fade. */
function ThreadHead({ children, right }: { children: ReactNode; right?: ReactNode }) {
  return (
    <header className="pointer-events-none absolute inset-x-0 top-0 z-20 flex h-[72px] items-center gap-2 bg-[linear-gradient(var(--paper)_58px,transparent)] px-[max(12px,env(safe-area-inset-left))] pb-0.5">
      <h2 className="pointer-events-auto flex min-w-0 flex-1 items-center truncate px-1.5 text-control font-medium text-ink">{children}</h2>
      {right && <div className={`pointer-events-auto flex shrink-0 items-center ${FLOAT}`}>{right}</div>}
    </header>
  );
}

/** ChatView's className under a ThreadHead, as the main chat's: the talk's
 * top clear of the bar, and no second fade under the bar's own. */
const DOCKED = "[--talk-top:calc(var(--tile-top,0px)+72px)] [--top-fade:0px]";

const EMPTY_NOTE = "mx-auto mt-10 max-w-[18rem] text-center text-control text-soft";

/** The scratchpad: a full chat the host keeps and runs, whose words, calls
 * and results go to the model whole each turn and nowhere else. The Memory
 * switch says what the next turn has: with it on, what the main chat has but
 * leaving nothing behind, and with it off the same but the memory. On a wide screen it
 * is a column of its own, or covers the chat's column where the window lacks
 * the room; on a phone, a place like Chat and Work. */
function Temp({ id, hold, close, send, name, place, cards, decide }: { id?: string; hold: (id: string) => void; close: () => void; send: (text: string) => void; name: string; place: "tile" | "page"; cards: Card[]; decide: Decide }) {
  // A tile opened beside a Temp (arg `new`, tiles.rs) makes a Temp of its own
  // and holds it by its id from then on; one with no arg holds the host's
  // current Temp, as before there were several.
  const key = id && id !== "new" ? id : "";
  const making = useRef(false);
  useEffect(() => {
    if (id !== "new" || making.current) return;
    making.current = true;
    void newScratchpad().then((made) => made && hold(made));
  }, [id, hold]);
  const pad = useScratchpad(key);
  useEffect(() => {
    if (key) void readScratchpad(key);
  }, [key]);
  // Its Temp closed elsewhere (another screen, or the item's Undo after it
  // went): a fresh one takes its place.
  useEffect(() => {
    if (key && pad.loaded && pad.id === null) hold("new");
  }, [key, pad.loaded, pad.id, hold]);
  const work = merged(pad.log, pad.heard);
  const page = place === "page";
  const started = !!work?.tools.length || !!work?.words;
  const messages = useMemo(() => logMessages(work, { queued: pad.queued, live: pad.working }), [work, pad.queued, pad.working]);
  // Open, it is read: an answer that comes now needs no mark.
  useEffect(() => {
    setScratchpadOpen(true, key);
    return () => setScratchpadOpen(false, key);
  }, [key]);
  // To chat, the talk moves over as it was and starts no turn; to Work,
  // it goes as a brief, which is a request.
  const promote = async (toWork: boolean) => {
    if (!toWork) return void ((await promoteScratchpad(key)) && close());
    const talk = [wordsOf(work ?? pad.log, name), ...pad.queued.map((q) => `Me: ${q}`)].filter(Boolean).join("\n\n");
    send(`${promotion("work on what this temp chat was about")}\n\n(The temp chat)\n${talk}`);
    void closeScratchpad(key);
    close();
  };
  // Clear: this tile's Temp goes and a fresh one takes its place in it.
  const clearIt = async () => {
    await closeScratchpad(key);
    if (key) hold("new");
  };
  useTileRows("temp", { "promote": { disabled: !started, run: () => void promote(true) } });
  const memory = (
    <button type="button" role="switch" aria-checked={pad.memory} onClick={() => void setScratchpadMemory(!pad.memory)} title={pad.memory ? "Memory on: the next turn reads the memory and has your tools" : "Memory off: the next turn has only the web"} className={page ? "floating pointer-events-auto flex h-11 items-center gap-2.5 rounded-full pr-2.5 pl-4 text-label text-mid" : "flex h-9 shrink-0 items-center gap-2.5 rounded-full pr-1.5 pl-3 text-control text-mid hover:bg-hover hover:text-ink"}>
      Memory
      {/* The switch Settings and Android draw, not one of its own. */}
      <Toggle on={pad.memory} />
    </button>
  );
  const clear = started && (
    // Clear is its own button beside the switch in the head's pill, as the main chat's head
    // holds its controls, so it never reads as the switch's second label.
    // Where the head is too narrow for its word, as at 1024 beside the workers' fold button, it keeps
    // only its glyph, so the ✕ is never pushed under that button.
    <button type="button" onClick={() => void clearIt()} title="Clear the temp chat" className={page ? "floating pointer-events-auto flex h-11 items-center gap-1.5 rounded-full px-4 text-label text-mid" : "flex h-9 shrink-0 items-center gap-1.5 rounded-full px-3 text-control text-mid hover:bg-hover hover:text-ink @max-[220px]:px-2"}>
      <svg viewBox="0 0 24 24" className={page ? "size-4" : "size-[18px]"} fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
        <path d="M7 21l-4.3-4.3c-1-1-1-2.5 0-3.4l9.6-9.6c1-1 2.5-1 3.4 0l5.6 5.6c1 1 1 2.5 0 3.4L13 21M22 21H7M5 11l9 9" />
      </svg>
      <span className={page ? "" : "@max-[220px]:sr-only"}>Clear</span>
    </button>
  );
  return (
    <section role="region" aria-label="Temp chat" className={page ? "relative flex h-full min-h-0 flex-1 flex-col" : "@container flex h-full w-[100cqw] shrink-0 flex-col"}>
      <ChatView
        surface="temp"
        className={page ? "[--talk-top:var(--chrome-top)]" : DOCKED}
        messages={messages}
        typing={pad.working}
        send={(text, files) => tellScratchpad(text, files, key)}
        stop={() => stopScratchpad(key)}
        stopping={pad.stopping}
        asks={pad.asks}
        reply={(ask, yes, answers, always) => replyScratchpad(ask, yes, answers, always, key)}
        cards={cards}
        decide={decide}
        composer={{ draft: forHost(key ? `draft:temp:${key}` : "draft:temp"), placeholder: "Message the temp chat", label: "Temp chat message", temp: true, autoFocus: !page }}
        empty={<p className={EMPTY_NOTE}>{pad.memory ? `${name} reads the memory and has your tools here, and none of this goes into memory unless you promote it.` : `${name} has only the web here and no memory. None of this is kept unless you promote it.`}</p>}
        foot={pad.failed && <p role="alert" className="text-label text-accent">{pad.failed}</p>}
        over={
          started && (
            <div className="flex shrink-0 justify-end gap-2 pt-1.5 pb-2">
              <button type="button" onClick={() => void promote(false)} disabled={pad.working} title={pad.working ? "Once Temp has answered" : "Move this talk into the chat as it was"} className="floating h-9 rounded-full px-3.5 text-caption text-mid hover:text-ink disabled:opacity-50">
                Promote to chat
              </button>
              <button type="button" onClick={() => void promote(true)} className="floating flex h-9 items-center gap-1 rounded-full px-3.5 text-caption text-mid hover:text-ink">
                <WorkGlyph className="size-3.5" /> Create task
              </button>
            </div>
          )
        }
        head={() =>
          // On a phone no head: the bar names the page and
          // leads back to the chat, and Memory and Clear float at the top right
          // over the talk, as the chat's buttons do (Places.kt TempPlace).
          page ? (
            <div className="pointer-events-none absolute inset-x-0 top-[calc(var(--tile-top,0px)+10px)] z-20 flex justify-end gap-2 px-3.5">
              {memory}
              {clear}
            </div>
          ) : (
            <ThreadHead
              right={
                <>
                  {memory}
                  {clear}
                </>
              }
            >
              {/* One line high: each word wraps out of sight unless it fits whole, the
                  first too, since an empty item holds the line and takes no room. */}
              <span className="flex h-[1lh] min-w-0 flex-1 flex-wrap gap-x-1 overflow-hidden whitespace-nowrap before:-mr-1 before:h-[1lh] before:content-['']">
                <span>Temp</span>
                <span>chat</span>
                <span className="font-normal text-soft">· not remembered</span>
              </span>
            </ThreadHead>
          )
        }
      />
    </section>
  );
}

/** A worker's report as a named memory's chat is told it (main.rs
 * `to_memory`): its heading line and what it said, without the note to the
 * model after it. */
function reportOf(told: string): { heading: string; body: string } | null {
  const m = /^<agent-report>\n([^\n]*)\n?([\s\S]*?)\n<\/agent-report>/.exec(told);
  return m ? { heading: m[1], body: m[2] } : null;
}

/** A named memory's chat: Temp's surface, kept for good and in its own
 * memory. Its turns read only that memory, with the rules every memory
 * shares and its own instructions; the main chat never sees it. */
function MemoryChatTile({ id, name, phone }: { id: string; name: string; phone: boolean }) {
  const memories = useMemories();
  const memory = memories?.find((m) => m.id === id);
  const at = useContext(MemoryPickCtx);
  const tabs = useContext(ChatTabsCtx);
  const chat = useMemoryChat(id);
  const own = useWorkerChoice(`memory-${id}`, `api/memories/${encodeURIComponent(id)}/choice`);
  const work = merged(chat.log, chat.heard);
  const messages = useMemo(() => logMessages(work, { queued: chat.queued, live: chat.working }), [work, chat.queued, chat.working]);
  const gone = chat.gone || (memories !== undefined && !memory);
  return (
    <section data-memory={id} role="region" aria-label={`${memory?.name ?? "Memory"} chat`} className={phone ? "flex h-full min-h-0 flex-1 flex-col" : "flex h-full w-[100cqw] shrink-0 flex-col"}>
      <ChatView
        surface="memory-chat"
        className={phone ? "" : DOCKED}
        messages={messages}
        typing={chat.working}
        send={(text, files) => tellMemoryChat(id, text, files)}
        stop={() => stopMemoryChat(id)}
        stopping={chat.stopping}
        asks={chat.asks}
        reply={(ask, yes, answers, always) => replyMemoryChat(id, ask, yes, answers, always)}
        composer={gone ? null : { draft: forHost(`draft:memory-${id}`), placeholder: messageTo(memory?.name), label: messageTo(memory?.name), own, autoFocus: !phone, agents: { memory: id } }}
        empty={!gone && <p className={EMPTY_NOTE}>{`${name} remembers this chat in the ${memory?.name ?? ""} memory alone, apart from the main chat.`}</p>}
        foot={
          gone ? (
            <p className={EMPTY_NOTE}>
              This memory was deleted.{" "}
              <button type="button" className="underline" onClick={() => at?.pick(null)}>
                Back to Main
              </button>
            </p>
          ) : (
            chat.failed && <p role="alert" className="text-label text-accent">{chat.failed}</p>
          )
        }
        head={() =>
          tabs ? (
            <header className={`pointer-events-none absolute inset-x-0 z-20 flex ${phone ? "top-[var(--tile-top,0px)] h-16 items-start px-[max(14px,env(safe-area-inset-left))] pt-2.5" : "top-0 h-[72px] items-center bg-[linear-gradient(var(--paper)_58px,transparent)] px-[max(12px,env(safe-area-inset-left))] pb-0.5"}`}>
              <div className={`pointer-events-auto min-w-0 ${phone ? "ml-[52px]" : ""}`}>{tabs}</div>
            </header>
          ) : (
            <ThreadHead>
              {memory?.name ?? "Memory"}&nbsp;<span className="font-normal text-soft">· its own memory</span>
            </ThreadHead>
          )
        }
      />
    </section>
  );
}

/** Settings' memories: each named memory's name and instructions, layered
 * on the rules above, and the way to add one. Deleting one removes its
 * talk and its memory for good. */
function MemoriesEditor({ note }: { note: string }) {
  const memories = useMemories();
  const [adding, setAdding] = useState("");
  const [said, setSaid] = useState<string>();
  if (!memories) return null;
  const add = async () => {
    const { error } = await addMemory(adding);
    setSaid(error);
    if (!error) setAdding("");
  };
  const field = "min-w-0 rounded-lg border border-frame bg-paper px-3 py-2 text-body outline-none focus:border-soft";
  const button = "shrink-0 rounded-lg border-[1.25px] border-edge px-3 py-2 text-body hover:bg-hover disabled:opacity-40 disabled:hover:bg-transparent";
  return (
    <Section title="Memories">
      <p className={`${note} px-5 pb-2`}>Each memory keeps its own chat apart from Main.</p>
      {memories.map((m) => (
        <MemoryRow key={m.id} memory={m} note={note} field={field} button={button} />
      ))}
      {said && (
        <p role="alert" className="px-5 pb-2 text-label text-accent">
          {said}
        </p>
      )}
      <form
        className="flex gap-2 px-5 py-1.5"
        onSubmit={(e) => {
          e.preventDefault();
          void add();
        }}
      >
        <input value={adding} onChange={(e) => setAdding(e.target.value)} placeholder="New memory, as Tutor" aria-label="New memory's name" className={`${field} flex-1`} />
        <button type="submit" disabled={!adding.trim()} className={button}>
          Add
        </button>
      </form>
    </Section>
  );
}

function MemoryRow({ memory, note, field, button }: { memory: Memory; note: string; field: string; button: string }) {
  const [name, setName] = useState<string | null>(null);
  const [text, setText] = useState<string | null>(null);
  const [said, setSaid] = useState<string>();
  const [sure, setSure] = useState(false);
  const save = async () => {
    const error = await changeMemory(memory.id, { ...(name !== null ? { name } : {}), ...(text !== null ? { instructions: text } : {}) });
    setSaid(error);
    if (!error) {
      setName(null);
      setText(null);
    }
  };
  const remove = async () => {
    if (!sure) return setSure(true);
    setSaid(await deleteMemory(memory.id));
  };
  return (
    <div className="flex flex-col gap-2 border-t border-hair px-5 py-3" data-memory={memory.id}>
      <input value={name ?? memory.name} onChange={(e) => setName(e.target.value)} aria-label={`${memory.name}'s name`} className={`${field} font-semibold`} />
      <textarea value={text ?? memory.instructions} onChange={(e) => setText(e.target.value)} rows={6} placeholder="agent.md: what a chat on this memory is for and how to go about it" aria-label={`${memory.name}'s instructions`} className={`${field} resize-y`} />
      {said && (
        <p role="alert" className="text-label text-accent">
          {said}
        </p>
      )}
      <div className="flex items-center gap-2">
        <button type="button" onClick={() => void save()} disabled={name === null && text === null} className={button}>
          Save
        </button>
        <button type="button" onClick={() => void remove()} onBlur={() => setSure(false)} className={`${button} ${sure ? "text-accent" : ""}`}>
          {sure ? "Delete its memory for good" : "Delete"}
        </button>
        {sure && <span className={note}>Its talk and memory go too.</span>}
      </div>
    </div>
  );
}

/** Asks before a worker merges into main, as Android's dialog does: in the
 * page, not the browser's own confirm. */
function MergeAsk({ name, archived, merge, cancel }: { name: string; archived: boolean; merge: () => void; cancel: () => void }) {
  useEffect(() => {
    // Caught first, so Escape puts down the question, not the page under it.
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      cancel();
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
  }, [cancel]);
  return (
    <div className="fixed inset-0 z-50 grid place-items-center bg-black/30 p-4" onClick={cancel}>
      <div role="alertdialog" aria-modal aria-labelledby="merge-title" onClick={(e) => e.stopPropagation()} className="rise w-[min(400px,100%)] rounded-2xl border border-frame bg-lift p-5 shadow-dock">
        <h3 id="merge-title" className="text-body font-semibold">Merge {name} into main?</h3>
        <p className="mt-2 text-body text-mid">
          All {name} did since it started, its tool calls and their output too, joins the main chat's memory as though done there.{archived ? "" : ` ${name} is then archived.`} This can't be undone.
        </p>
        <div className="mt-4 flex justify-end gap-1">
          <button type="button" onClick={cancel} className="rounded-full px-4 py-2 text-control text-mid hover:bg-hover hover:text-ink">Cancel</button>
          <button type="button" onClick={merge} autoFocus className="rounded-full px-4 py-2 text-control font-semibold text-accent hover:bg-hover">Merge</button>
        </div>
      </div>
    </div>
  );
}

// A subagent's own page, over the talk: its task, then its words and calls
// in order, live while it works. One the page no longer holds, after a
// reload, comes from hippo, which keeps every subagent's chat. Only the
// libri's own agents can be stopped.
export function AgentPage({ call, shown, agent, title, side, fold, thread, parent, tasks, stop, tell, close }: { call: string; shown?: string; agent?: string; title?: string; side?: boolean; fold?: boolean; thread?: WorkThread; parent?: string; tasks: Task[]; stop: (name: string) => Promise<void>; tell: (name: string, text: string, files?: File[]) => Promise<boolean>; close: () => void }) {
  const detail = useChatDetail();
  const task = tasks.find((t) => t.name === call);
  const state = thread?.state;
  const wide = useWide();
  const [kept, setKept] = useState<Pick<Task, "text" | "tools" | "words"> | null>();
  // Libri's own agents keep their logs in its state, the seat's in
  // hippo; one of Libri's is known by its name, with no title.
  const own = !agent && !title;
  const fetching = !!agent || own;
  // The log on every open, whatever this page heard, and again once the work
  // stops, for the report the stream doesn't carry.
  useEffect(() => {
    if (!fetching) return;
    let on = true;
    api(agent ? `api/agent/${encodeURIComponent(agent)}/chat` : `api/work/${encodeURIComponent(call)}`)
      .then((r) => (r.ok ? r.json() : null))
      .then((j: { text: string } | null) => on && setKept(j ? fromAgentChat(j.text) : null))
      .catch(() => on && setKept(null));
    return () => {
      on = false;
    };
  }, [fetching, agent, call, state, thread?.waiting_on?.length, task?.unheard]);
  // Escape leaves the page over the talk; beside it, the talk may want the key.
  useEffect(() => {
    if (side) return;
    const key = (e: KeyboardEvent) => e.key === "Escape" && close();
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  }, [close, side]);
  const work = merged(kept, task);
  // The host says where a thread stands; only the seat's own agents, which
  // have no thread, go by what the page heard.
  const live = state ? state === "working" && !thread?.waiting_on?.length : !!task && !task.done;
  // Its brief stands in for a log not yet written, so a worker just
  // started shows what it was asked, not a bare loader.
  const brief = work?.text || thread?.brief || undefined;
  const messages = useMemo(() => logMessages(work, { brief, live }), [work, brief, live]);
  // A worker forked the main chat's memory; merging brings what it did since
  // into main as though said there, once, and puts the thread away; an
  // archived one can be merged too. Not while its job goes on, paused on a
  // reply included, nor while it asks the person something: the host
  // refuses those.
  const name = shown ?? call;
  const [landed, setLanded] = useState<string>();
  const [asking, setAsking] = useState(false);
  const [done, setDone] = useState(false);
  // The host says when there is nothing new to merge, so the button stays
  // gone after a reload, until the worker does more.
  const mergeable = own && !!thread && thread.state !== "working" && !asks(thread) && !thread.waiting_on?.length && !thread.merged && !done;
  const merge = async () => {
    setAsking(false);
    const r = await api(`api/work/${encodeURIComponent(call)}/merge`, { method: "POST" }).catch(() => undefined);
    const said = (await r?.text().catch(() => "")) || "";
    setDone(!!r?.ok);
    setLanded(r?.ok ? said || "Merged." : said || `${name} wasn't merged.`);
  };
  // Only Libri's own workers take words; the seat's Agent calls have no way
  // in from outside the CLI.
  const writable = own && (!task || !task.title);
  // The thread's pickers read and set its own model, thinking and access
  // (site.rs `work_choice`), never the main chat's.
  const choice = useWorkerChoice(writable ? call : undefined);
  return (
    <div className={`flex flex-col bg-paper ${side ? "h-full" : "rise absolute inset-0 z-30"}`} role="complementary" data-thread={call} aria-label={title ?? shown ?? (task ? nameOf(task) : "Agent")}>
      <ChatView
        surface="thread"
        messages={messages}
        typing={live}
        send={(text, files) => tell(call, text.trim(), files)}
        stop={() => stop(call)}
        stoppable={live || !!thread?.waiting_on?.length}
        stopping={task?.stopping}
        composer={writable ? { draft: forHost(`draft:${call}`), placeholder: live ? `Tell ${name} while it works` : `Tell ${name} more`, label: `Message ${name}`, agents: { thread: call }, own: choice } : null}
        // Beside the loader, what the host says it is doing now.
        foot={
          <>
            {live && thread?.summary && detail !== "simple" && (
              <span className="min-w-0 truncate text-label font-medium text-mid">
                Working · {thread.summary}
                <span className="font-normal text-soft tabular-nums"> · {ago(thread.at)}</span>
              </span>
            )}
            {!messages.length && (fetching && kept === undefined ? <Dots /> : !live && <p className="text-mid">Its work wasn't kept.</p>)}
          </>
        }
        head={() => (
          <>
            <header className="flex h-[57px] shrink-0 items-center gap-1.5 px-[max(12px,env(safe-area-inset-left))] max-sm:h-[51px] lg:pr-1.5">
              {!side && <button type="button" onClick={close} aria-label="Back to the talk" title="Back" className="grid size-9 shrink-0 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink">
                <svg viewBox="0 0 24 24" className="size-[18px]" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round">
                  <path d="m15 18-6-6 6-6" />
                </svg>
              </button>}
              <h2 className={`min-w-0 flex-1 truncate text-body font-semibold ${side ? "pl-2" : ""}`}>
                {parent && <span className="font-normal text-mid">{parent} › </span>}
                {title ?? shown ?? (task ? nameOf(task) : own ? call : "Agent")}
                <span className="font-normal text-mid"> · {live ? "working" : thread ? stateOf(thread).replace(/^./, (c) => c.toLowerCase()) : "done"}</span>
              </h2>
              {mergeable && (
                <HeaderButton label="Merge into main" onClick={() => setAsking(true)}>
                  <circle cx="18" cy="18" r="3" />
                  <circle cx="6" cy="6" r="3" />
                  <path d="M6 21V9a9 9 0 0 0 9 9" />
                </HeaderButton>
              )}
              {side &&
                (fold && !wide ? (
                  // On a phone the thread is a page over Work, put down.
                  <HeaderButton label="Back to Tasks" onClick={close}>
                    <path d="M12 5v14M6 13l6 6 6-6" />
                  </HeaderButton>
                ) : (
                  <HeaderButton label="Close its work" onClick={close}>
                    <path d="M18 6 6 18M6 6l12 12" />
                  </HeaderButton>
                ))}
            </header>
            {landed && (
              <p role="status" className="shrink-0 px-4 py-2 text-caption text-mid">
                {landed}
              </p>
            )}
            {asking && <MergeAsk name={name} archived={thread?.state === "archived"} merge={() => void merge()} cancel={() => setAsking(false)} />}
          </>
        )}
      />
    </div>
  );
}

// Acts waiting for the person's word sit above the composer, where the
// next thing is done, until a verdict settles them. A mail draft reads as
// a letter on a sheet of its own.
// What the seat asks mid-turn, where Claude Code would ask in the terminal:
// AskUserQuestion's questions to pick from, or whether a tool may run.
/** The replies the newest message offers, as a row of buttons under it, as
 * a messaging app's suggested replies (a design choice of 2026-11-06: "quick-reply chips
 * under the relevant assistant message, not above composer"). A tap sends
 * the reply tied to the message, or answers the question asked. */
function Offered() {
  const offering = useContext(OfferCtx);
  const last = useAuiState((s) => s.message.isLast);
  if (!offering || !last) return null;
  const { offer } = offering;
  return <Replies key={offer.ask?.id ?? offer.quote?.id} {...offering} />;
}

function Replies({ offer, send, reply }: Offering) {
  const [sent, setSent] = useState<string>();
  const { rise, onAnimationEnd } = useRise();
  const tap = (r: string) => {
    setSent(r);
    if (offer.ask) reply(offer.ask.id, true, { [offer.ask.question]: r });
    else void send(r, [], undefined, offer.quote);
  };
  return (
    <div role="group" aria-label={offer.ask?.question ?? "Suggested replies"} onAnimationEnd={onAnimationEnd} className={`${rise} mt-2 flex flex-col gap-2`}>
      {/* A question's chips carry their question, or they answer nothing seen. */}
      {offer.ask && <p className="text-body font-medium">{offer.ask.question}</p>}
      <div className="flex flex-wrap gap-2">
        {offer.replies.map((r) => (
          <button key={r} type="button" disabled={sent !== undefined} onClick={() => tap(r)} className="min-h-9 rounded-full border border-frame bg-lift px-3.5 py-1.5 text-left text-control text-ink hover:bg-hover disabled:opacity-50 max-lg:min-h-11">
            {r}
          </button>
        ))}
      </div>
    </div>
  );
}

function AskCard({ ask: a, reply }: { ask: Ask; reply: Reply }) {
  const questions = a.tool === "AskUserQuestion" ? ((a.input.questions as Question[] | undefined) ?? []) : undefined;
  const [picked, setPicked] = useState<Record<string, string[]>>({});
  const [other, setOther] = useState<Record<string, string>>({});
  const answer = (q: Question) => (other[q.question]?.trim() ? [...(picked[q.question] ?? []), other[q.question].trim()] : (picked[q.question] ?? [])).join(", ");
  const pick = (q: Question, label: string) =>
    setPicked((p) => {
      const was = p[q.question] ?? [];
      const now = q.multiSelect ? (was.includes(label) ? was.filter((l) => l !== label) : [...was, label]) : [label];
      return { ...p, [q.question]: now };
    });
  const ready = !questions || questions.every((q) => answer(q));
  const go = () => reply(a.id, true, questions && Object.fromEntries(questions.map((q) => [q.question, answer(q)])));
  const { rise, onAnimationEnd } = useRise();
  return (
    <article onAnimationEnd={onAnimationEnd} className={`${rise} relative mb-3.5 shrink-0 rounded-xl border-[1.25px] border-edge bg-lift shadow-lift`} aria-label={questions ? "Questions" : `May ${a.tool} run?`}>
      <span className="absolute -top-[0.75em] left-3.5 rounded-sm bg-lift px-1.5 text-caption leading-4 font-semibold tracking-wide text-accent uppercase">Waiting on you</span>
      {questions ? (
        <div className="flex flex-col gap-4 px-5 pt-4 pb-2">
          {questions.map((q) => (
            <fieldset key={q.question} className="flex flex-col gap-2">
              {q.header && <legend className="text-caption font-semibold tracking-wider text-soft uppercase">{q.header}</legend>}
              <p className="text-body font-medium">{q.question}</p>
              <div className="flex flex-wrap gap-2">
                {q.options.map((o) => {
                  const on = picked[q.question]?.includes(o.label);
                  return (
                    <button
                      key={o.label}
                      title={o.description}
                      onClick={() => pick(q, o.label)}
                      className={`rounded-xl border px-3 py-1.5 text-left text-control ${on ? "border-ink bg-ink text-paper" : "border-frame hover:bg-hover"}`}
                    >
                      {o.label}
                      {o.description && o.description !== o.label && <span className={`block text-caption ${on ? "opacity-75" : "text-mid"}`}>{o.description}</span>}
                    </button>
                  );
                })}
              </div>
              <input
                value={other[q.question] ?? ""}
                onChange={(e) => setOther((o) => ({ ...o, [q.question]: e.target.value }))}
                placeholder="Or say something else"
                className="rounded-xl border border-frame bg-transparent px-3 py-1.5 text-control outline-none focus:border-soft"
              />
            </fieldset>
          ))}
        </div>
      ) : (
        <div className="flex flex-col gap-1.5 px-5 pt-4 pb-2">
          <p className="text-body font-medium">
            May I use <code className="text-control">{a.tool}</code>?
          </p>
          {a.about && <p className="text-control text-mid">{a.about}</p>}
          <pre className="max-h-48 overflow-auto rounded-lg bg-hover px-3 py-2 text-caption whitespace-pre-wrap [overflow-wrap:anywhere]">{shown(a.input)}</pre>
        </div>
      )}
      <footer className="flex flex-wrap items-center justify-end gap-2 px-3.5 pb-3">
        {a.busy ? (
          <span className="flex h-9 items-center gap-2 px-3 text-label text-mid">
            <span className="size-3 animate-spin rounded-full border-[1.5px] border-hair border-t-active" /> Working
          </span>
        ) : (
          <>
            <button onClick={() => reply(a.id, false)} className="h-9 rounded-full px-4 text-control text-mid hover:bg-hover hover:text-ink">
              {questions ? "Skip" : "Deny"}
            </button>
            {/* A connection's tool: "Always allow" sets it to Allow in Settings > Connections. */}
            {a.always && (
              <button onClick={() => reply(a.id, true, undefined, true)} className="h-9 rounded-full border border-frame px-4 text-control text-ink hover:bg-hover">
                Always allow
              </button>
            )}
            <button disabled={!ready} onClick={go} className="h-9 rounded-full bg-ink px-5 text-control font-medium text-paper hover:opacity-85 disabled:opacity-40">
              {questions ? "Answer" : a.always ? "Allow once" : "Allow"}
            </button>
          </>
        )}
      </footer>
    </article>
  );
}

// A tool's input as the person reads it: the command or the file when it
// has one, else the whole input.
function shown(input: Record<string, unknown>): string {
  for (const k of ["command", "file_path", "url", "pattern"]) if (typeof input[k] === "string") return input[k] as string;
  return JSON.stringify(input, null, 2);
}

function Tray({ cards, decide }: { cards: Card[]; decide: Decide }) {
  if (cards.length === 0) return null;
  return (
    <div className="mb-3 flex flex-col gap-3.5">
      {cards.map((c) => (
        <Letter key={c.id} card={c} decide={decide} />
      ))}
    </div>
  );
}

function Letter({ card: c, decide }: { card: Card; decide: Decide }) {
  const { to, account, subject, body } = c.act;
  const [open, setOpen] = useState(false);
  const head = "w-16 shrink-0 text-caption font-semibold tracking-wider text-soft uppercase";
  const { rise, onAnimationEnd } = useRise();
  return (
    <article onAnimationEnd={onAnimationEnd} className={`${rise} relative shrink-0 rounded-xl border-[1.25px] border-edge bg-lift shadow-lift`} aria-label={`Draft to ${to}`}>
      <span className="absolute -top-[0.75em] left-3.5 rounded-sm bg-lift px-1.5 text-caption leading-4 font-semibold tracking-wide text-accent uppercase">
        Waiting on you
      </span>
      <dl className="flex flex-col gap-0.5 border-b border-dashed border-frame px-5 pt-4 pb-2.5 text-control">
        <div className="flex items-baseline">
          <dt className={head}>To</dt>
          <dd className="truncate">{to}</dd>
        </div>
        {account && (
          <div className="flex items-baseline">
            <dt className={head}>From</dt>
            <dd className="truncate text-mid">{account}</dd>
          </div>
        )}
        <div className="flex items-baseline">
          <dt className={head}>Subject</dt>
          <dd className="font-medium [overflow-wrap:anywhere]">{subject}</dd>
        </div>
      </dl>
      {/* Folded to a few lines so the talk stays in view; a click reads it whole. */}
      <div onClick={() => setOpen(!open)} title={open ? undefined : "Read it all"} className="cursor-pointer px-5 py-3">
        <p className={`font-serif text-body leading-relaxed whitespace-pre-wrap [overflow-wrap:anywhere] ${open ? "max-h-72 overflow-y-auto" : "line-clamp-3"}`}>
          {body}
        </p>
      </div>
      {c.error && <p className="px-5 pb-1 text-label text-accent">{c.error}</p>}
      <footer className="flex items-center justify-end gap-2 px-3.5 pb-3">
        {c.busy ? (
          <span className="flex h-9 items-center gap-2 px-3 text-label text-mid">
            <span className="size-3 animate-spin rounded-full border-[1.5px] border-hair border-t-active" /> Working
          </span>
        ) : (
          <>
            <button onClick={() => decide(c.id, false)} className="h-9 rounded-full px-4 text-control text-mid hover:bg-hover hover:text-ink">
              Drop
            </button>
            <button onClick={() => decide(c.id, true)} className="h-9 rounded-full bg-ink px-5 text-control font-medium text-paper hover:opacity-85">
              Send
            </button>
          </>
        )}
      </footer>
    </article>
  );
}
