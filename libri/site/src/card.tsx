// One MCP Apps card, hosted as the spec has a web host do it: a frame of
// this host's sandbox proxy (served with the card's CSP as its header, in
// an opaque origin), which frames the card's HTML; the reference SDK's
// AppBridge speaks the protocol. AppBridge has no MCP client of its own:
// the card's tool calls and reads go to the host, which runs them on its
// own session to the card's server, never through the harness.
//
// Loaded only when a card is drawn, so the SDK stays out of the page
// otherwise.

import { AppBridge, PostMessageTransport, buildAllowAttribute, type McpUiHostContext } from "@modelcontextprotocol/ext-apps/app-bridge";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { api, url } from "./api";
import type { CardTool } from "./apps";
import { followPage, pageOf } from "./pageLink";

declare global {
  interface Window {
    /** The Android app's channel for a card it draws (CallCardAlone). */
    LibriCard?: { postMessage: (text: string) => void };
  }
}

/** The tallest a card stands in the chat; past it, it scrolls inside. */
const TALLEST = 640;

/** How tall a card stands until it says: a frame with no area is one
 * Chromium stops drawing, and its ResizeObserver never reports. */
const FIRST = 120;

type View = { html: string; csp?: Record<string, string[]> | null; permissions?: Record<string, object> | null; prefersBorder?: boolean | null };

/** What a call gave back, as the card takes it: the whole result where the
 * harness passed it on, else its text, which the spec allows. */
export type Outcome = { content?: unknown[]; structuredContent?: unknown; _meta?: unknown; isError?: boolean };

const asked = new Map<string, Promise<View>>();
/** A card's view, asked once per server and URI; `version` asks again for
 * a Library page published anew under the same name. */
function view(card: CardTool, version = 0) {
  const key = `${card.server}\n${card.uri}\n${version}`;
  let v = asked.get(key);
  if (!v) {
    v = api(`api/apps/view?server=${encodeURIComponent(card.server)}&uri=${encodeURIComponent(card.uri)}`).then(async (r) => {
      if (!r.ok) throw new Error((await r.text()) || `The card would not load (${r.status}).`);
      return r.json();
    });
    v.catch(() => asked.delete(key));
    asked.set(key, v);
  }
  return v;
}

async function post<T>(path: string, body: unknown): Promise<T> {
  const r = await api(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
  if (!r.ok) throw new Error((await r.text()) || `${r.status}`);
  return r.json();
}

/** The page's colours and type under the spec's names, as they are now. */
/** The host's own @font-face rules (Inter), their files named in full: the
 * view runs at an opaque origin, where a path means nothing. The sandbox's
 * font-src allows this origin, which serves its build assets to any. */
let fontFaces: string | undefined;
function fonts(): string {
  if (fontFaces) return fontFaces;
  const rules: string[] = [];
  for (const sheet of Array.from(document.styleSheets)) {
    let list: CSSRuleList;
    try {
      list = sheet.cssRules;
    } catch {
      continue;
    }
    const base = sheet.href ?? location.href;
    for (const r of Array.from(list)) {
      if (r instanceof CSSFontFaceRule) rules.push(r.cssText.replace(/url\("?([^")]+)"?\)/g, (_, u: string) => `url("${new URL(u, base).href}")`));
    }
  }
  return (fontFaces = rules.join("\n"));
}

function styles(): McpUiHostContext["styles"] {
  const css = getComputedStyle(document.documentElement);
  const v = (name: string) => css.getPropertyValue(name).trim() || undefined;
  return {
    variables: {
      "--color-background-primary": v("--paper"),
      "--color-background-secondary": v("--lift"),
      "--color-background-tertiary": v("--block"),
      "--color-text-primary": v("--ink"),
      "--color-text-secondary": v("--mid"),
      "--color-text-tertiary": v("--soft"),
      "--color-border-primary": v("--frame"),
      "--color-border-secondary": v("--hair"),
      "--color-ring-primary": v("--accent"),
      "--font-sans": v("--font-sans"),
      "--font-mono": v("--font-mono"),
      "--border-radius-sm": v("--radius-sm"),
      "--border-radius-md": v("--radius-md"),
      "--border-radius-lg": v("--radius-lg"),
      "--border-radius-xl": v("--radius-xl"),
    } as NonNullable<McpUiHostContext["styles"]>["variables"],
    css: { fonts: fonts() },
  };
}

function context(card: CardTool, callId: string, width: number, full: boolean, height?: number): McpUiHostContext {
  const touch = matchMedia("(pointer: coarse)").matches;
  return {
    toolInfo: { id: callId, tool: card.definition as never },
    theme: document.documentElement.dataset.theme === "dark" ? "dark" : "light",
    styles: styles(),
    displayMode: full ? "fullscreen" : "inline",
    availableDisplayModes: ["inline", "fullscreen"],
    containerDimensions: full ? { width: innerWidth, height: innerHeight } : height != null ? { width, height } : { width, maxHeight: TALLEST },
    locale: navigator.language,
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    userAgent: "Libri",
    platform: touch ? "mobile" : "web",
    deviceCapabilities: { touch, hover: matchMedia("(hover: hover)").matches },
  };
}

type Props = { card: CardTool; callId: string; input: Record<string, unknown>; outcome: Outcome; say: (text: string) => void };

/** How a frame runs its view's tool calls; a Library page has its own way. */
type Calls = (tool: string, args: Record<string, unknown>) => Promise<unknown>;

/** The card for call `callId`, with its input and what it gave back.
 * `say` puts a card's message in the composer for the person to send.
 * Full screen is a second frame of the same card over the page: a frame
 * moved in the page reloads, and the chat's column holds a fixed frame to
 * itself. */
export default function Card(props: Props) {
  const [full, setFull] = useState(false);
  return (
    <>
      <Frame {...props} full={false} setFull={setFull} />
      {full && createPortal(<Frame {...props} full setFull={setFull} />, document.body)}
    </>
  );
}

/** A card alone on the page, full screen: `#card=<tool>` opens the card of
 * that tool with no call behind it, for a card that shows something live
 * (the browser's). The Android app opens it in a view of its own, which
 * closing the card closes; a browser goes back to the chat. */
export function CardPage({ card }: { card: CardTool }) {
  const leave = () => {
    window.close();
    // Still here: a tab that this page did not open, which close() leaves.
    setTimeout(() => location.replace(location.pathname + location.search), 100);
  };
  return <Frame card={card} callId="page" input={{}} outcome={{ content: [] }} say={() => {}} full setFull={(on) => !on && leave()} />;
}

/** One call's card alone, for the Android app, which draws it in its chat
 * in a web view as tall as the card: `#call-card=<call name>` with the
 * call's `input` and `outcome` as JSON. `fresh` asks a read-only tool
 * again for what it says now, as the web's pill does for a kept call.
 * `full` fills the window. The page tells the app, as JSON through
 * `LibriCard`, its height, a message for the composer, and the card's wish
 * to go full screen or come back. */
export function CallCardAlone({ card, input, told, fresh, full }: { card: CardTool; input: Record<string, unknown>; told: Outcome; fresh: boolean; full: boolean }) {
  const box = useRef<HTMLDivElement>(null);
  const [outcome, setOutcome] = useState<Outcome | null>(fresh ? null : told);
  const tell = (said: Record<string, unknown>) => window.LibriCard?.postMessage(JSON.stringify(said));
  useEffect(() => {
    if (!fresh) return;
    const reads = (card.definition.annotations as { readOnlyHint?: boolean } | undefined)?.readOnlyHint === true;
    if (!reads) return setOutcome(told);
    let on = true;
    post<Outcome>("api/apps/call", { server: card.server, tool: card.tool, arguments: input }).then(
      (o) => on && setOutcome(o),
      () => on && setOutcome(told),
    );
    return () => {
      on = false;
    };
    // The call is asked once, as the page opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  // The app sizes its view to the card, as the card resizes.
  useEffect(() => {
    const el = box.current;
    if (full || !el) return;
    const sized = new ResizeObserver(() => tell({ height: Math.ceil(el.getBoundingClientRect().height) }));
    sized.observe(el);
    return () => sized.disconnect();
  }, [full, outcome]);
  const setFull = (on: boolean) => tell(on ? { full: true } : { close: true });
  if (!outcome) return <div ref={box} className="h-10 animate-pulse rounded-xl border border-hair bg-lift" aria-label="Opening the card" />;
  return (
    <div ref={box} className="bg-paper">
      <Frame card={card} callId="call" input={input} outcome={outcome} say={(text) => tell({ say: text })} full={full} setFull={setFull} />
    </div>
  );
}

/** A Library page (library.rs): an MCP App of Libri's own, run in the
 * Library's pane, which it fills. Its calls go to its own page's data
 * alone (site.rs page_call); `version` is when it was last published, and
 * a new one draws it afresh. */
export function PageFrame({ name, version, say }: { name: string; version: number; say: (text: string) => void }) {
  const card = useMemo<CardTool>(() => ({ server: "libri", tool: name, uri: `ui://libri/pages/${name}`, definition: { name: "show_page", inputSchema: { type: "object" } } }), [name]);
  const calls = useCallback<Calls>((tool, args) => post(`api/library/${encodeURIComponent(name)}/call`, { tool, arguments: args }), [name]);
  return <Frame key={version} card={card} callId={`page-${name}`} input={{}} outcome={{ content: [] }} say={say} full={false} setFull={() => {}} calls={calls} version={version} />;
}

/** A built-in's view (builtins.rs view): one MCP App of Libri's own per
 * built-in, the same whatever provider fills it. Its calls go to the
 * built-in alone (site.rs builtin_call), which reads and never acts. */
export function BuiltinFrame({ id, say }: { id: string; say: (text: string) => void }) {
  const card = useMemo<CardTool>(() => ({ server: "libri", tool: id, uri: `ui://libri/builtins/${id}`, definition: { name: id, inputSchema: { type: "object" } } }), [id]);
  const calls = useCallback<Calls>((tool, args) => post(`api/builtins/${encodeURIComponent(id)}/call`, { tool, arguments: args }), [id]);
  return <Frame card={card} callId={`builtin-${id}`} input={{}} outcome={{ content: [] }} say={say} full={false} setFull={() => {}} calls={calls} />;
}

function Frame({ card, callId, input, outcome, say, full, setFull, calls, version }: Props & { full: boolean; setFull: (on: boolean) => void; calls?: Calls; version?: number }) {
  // A Library page fills its pane, which holds its title: no head of the
  // frame's own, no full screen, and it takes the pane's height.
  const pane = !!calls;
  const frame = useRef<HTMLIFrameElement>(null);
  const box = useRef<HTMLDivElement>(null);
  const [v, setView] = useState<View | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [height, setHeight] = useState(FIRST);
  const appBridge = useRef<AppBridge | null>(null);
  const ready = useRef(false);
  const latest = useRef(outcome);
  latest.current = outcome;

  useEffect(() => {
    let on = true;
    view(card, version).then(
      (got) => on && setView(got),
      (e: Error) => on && setFailed(e.message),
    );
    return () => {
      on = false;
    };
  }, [card, version]);

  // One AppBridge per frame, set up before the proxy loads so its ready
  // notice is heard; torn down, as the spec asks, before the frame goes.
  useEffect(() => {
    const f = frame.current;
    if (!v || !f?.contentWindow) return;
    const b = new AppBridge(
      null,
      { name: "Libri", version: "1" },
      { openLinks: {}, serverTools: {}, serverResources: {}, logging: {}, message: { text: {} } },
      { hostContext: context(card, callId, box.current?.clientWidth ?? 600, full, pane ? box.current?.clientHeight : undefined) },
    );
    b.onsandboxready = () => void b.sendSandboxResourceReady({ html: v.html, csp: v.csp ?? undefined, permissions: v.permissions ?? undefined });
    b.oninitialized = () => {
      ready.current = true;
      void b.sendToolInput({ arguments: input });
      void b.sendToolResult(latest.current as never);
    };
    b.onsizechange = ({ height: h }) => {
      if (h != null && h > 0) setHeight(Math.min(Math.ceil(h), TALLEST));
    };
    // The spec holds a card to its own server: every call goes there.
    b.oncalltool = (p) => (calls ? calls(p.name, p.arguments ?? {}) : post("api/apps/call", { server: card.server, tool: p.name, arguments: p.arguments ?? {} })) as never;
    // A page reads nothing but its own data, which its tools give.
    b.onreadresource = (p) => (calls ? Promise.reject(new Error("A Library page reads no resources.")) : post("api/apps/read", { server: card.server, uri: p.uri }));
    b.onopenlink = async ({ url: to }) => {
      if (!/^https?:\/\//i.test(to)) return { isError: true };
      // Another Library page opens in the Library, in place.
      const page = pageOf(to);
      if (page) return followPage(page), {};
      window.open(to, "_blank", "noopener,noreferrer");
      return {};
    };
    // A card's message goes to the composer, for the person to send or not.
    b.onmessage = async ({ content }) => {
      const text = (content as { type: string; text?: string }[])
        .flatMap((c) => (c.type === "text" && c.text ? [c.text] : []))
        .join("\n");
      if (text) say(text);
      return {};
    };
    b.onrequestdisplaymode = async ({ mode }) => {
      if (pane) return { mode: "inline" };
      const to = mode === "fullscreen";
      setFull(to);
      return { mode: to ? "fullscreen" : "inline" };
    };
    b.onloggingmessage = ({ level, data }) => console.debug(`[card ${card.server}/${card.tool}] ${level}`, data);
    appBridge.current = b;
    void b.connect(new PostMessageTransport(f.contentWindow, f.contentWindow));
    const theme = new MutationObserver(() => {
      if (ready.current) b.setHostContext(context(card, callId, box.current?.clientWidth ?? 600, full, pane ? box.current?.clientHeight : undefined));
    });
    theme.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    // The pane's size is the page's, told as the window or a column's drag changes it.
    const sized = new ResizeObserver(() => {
      const el = box.current;
      if (pane && el && ready.current) b.setHostContext(context(card, callId, el.clientWidth, full, el.clientHeight));
    });
    if (box.current) sized.observe(box.current);
    return () => {
      theme.disconnect();
      sized.disconnect();
      appBridge.current = null;
      const close = () => void b.close();
      if (ready.current) b.teardownResource({}, { timeout: 500 }).then(close, close);
      else close();
    };
    // The frame is drawn once per card; input and outcome are what it was
    // drawn with.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [v, card, callId]);

  useEffect(() => {
    if (!full) return;
    const off = (e: KeyboardEvent) => e.key === "Escape" && setFull(false);
    addEventListener("keydown", off);
    return () => removeEventListener("keydown", off);
  }, [full, setFull]);

  if (failed) return <p className={pane ? "chat-column pt-6 text-control text-mid" : "my-2 text-caption leading-4 text-mid"}>{pane ? `This page would not load: ${failed}` : `The ${card.tool} card would not load: ${failed}`}</p>;
  if (!v) return pane ? <div className="min-h-0 flex-1" aria-label="Loading the page" /> : <div className="my-2.5 h-10 animate-pulse rounded-xl border border-hair bg-lift" aria-label="Loading the card" />;
  const csp = encodeURIComponent(JSON.stringify(v.csp ?? {}));
  if (pane)
    return (
      <div ref={box} className="relative flex min-h-0 flex-1 flex-col">
        <iframe ref={frame} title={card.tool} src={url(`api/apps/sandbox?csp=${csp}&font=${encodeURIComponent(location.origin)}`)} sandbox="allow-scripts allow-same-origin allow-forms" className="block min-h-0 w-full flex-1 border-0 bg-paper" />
      </div>
    );
  const bordered = v.prefersBorder !== false;
  return (
    <div
      ref={box}
      role={full ? "dialog" : undefined}
      aria-modal={full || undefined}
      aria-label={full ? `${card.server} card` : undefined}
      className={
        full
          ? "fixed inset-0 z-50 flex flex-col bg-paper pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)]"
          : `not-prose relative my-2.5 overflow-hidden ${bordered ? "rounded-xl border border-hair bg-paper" : ""}`
      }
    >
      <div className={`flex items-center gap-2 text-caption leading-4 text-mid ${full ? "border-b border-hair px-4 py-2.5" : bordered ? "border-b border-hair px-3 py-1.5" : "py-1"}`}>
        <span className="min-w-0 flex-1 truncate">{card.server}</span>
        <button type="button" onClick={() => setFull(!full)} aria-label={full ? "Close full screen" : "Full screen"} className="grid size-7 place-items-center rounded-md hover:bg-hover">
          <svg viewBox="0 0 24 24" className="size-3.5" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
            {full ? <path d="M18 6 6 18M6 6l12 12" /> : <path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7" />}
          </svg>
        </button>
      </div>
      <iframe
        ref={frame}
        title={`${card.tool} card`}
        src={url(`api/apps/sandbox?csp=${csp}&font=${encodeURIComponent(location.origin)}`)}
        sandbox="allow-scripts allow-same-origin allow-forms"
        allow={buildAllowAttribute(v.permissions ?? undefined) || undefined}
        className={`block w-full border-0 bg-transparent ${full ? "flex-1" : ""}`}
        style={full ? undefined : { height }}
      />
    </div>
  );
}
