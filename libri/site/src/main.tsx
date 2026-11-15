import { lazy, StrictMode, Suspense } from "react";
import { createRoot } from "react-dom/client";
import "@fontsource-variable/inter";
import "./index.css";
import { takeHosts } from "./hosts";
import { comeHome, findHere } from "./api";
import { App } from "./App";
import { useCardTool } from "./apps";
import { useTheme } from "./theme";
import { tell } from "./dialog";

const CardPage = lazy(() => import("./card").then((m) => ({ default: m.CardPage })));
const CallCardAlone = lazy(() => import("./card").then((m) => ({ default: m.CallCardAlone })));
const PageAlone = lazy(() => import("./library").then((m) => ({ default: m.PageAlone })));
const BrowserTile = lazy(() => import("./browser").then((m) => ({ default: m.BrowserTile })));
const SystemTile = lazy(() => import("./system").then((m) => ({ default: m.SystemTile })));
const BuiltinAlone = lazy(() => import("./library").then((m) => ({ default: m.BuiltinAlone })));

/** `#card=<tool>`: that tool's card alone, full screen (card.tsx CardPage). */
function OneCard({ name }: { name: string }) {
  useTheme();
  const card = useCardTool(name);
  if (!card) return <p className="p-6 text-small text-mid">Loading the card…</p>;
  return (
    <Suspense>
      <CardPage card={card} />
    </Suspense>
  );
}
const oneCard = new URLSearchParams(location.hash.slice(1)).get("card");

/** JSON from the address, or `or` where it doesn't read. */
function parsed<T>(text: string | null, or: T): T {
  try {
    return text ? (JSON.parse(text) as T) : or;
  } catch {
    return or;
  }
}

/** `#call-card=<call name>&input=…&outcome=…`: one call's card alone, for
 * the Android app's chat (card.tsx CallCardAlone). */
function OneCallCard({ name, asked }: { name: string; asked: URLSearchParams }) {
  const card = useCardTool(name);
  if (!card) return null;
  const outcome = parsed<Record<string, unknown>>(asked.get("outcome"), { content: [] });
  return (
    <Suspense>
      <CallCardAlone
        card={card}
        input={parsed<Record<string, unknown>>(asked.get("input"), {})}
        told={{ ...outcome, ...(asked.get("error") === "1" ? { isError: true } : {}) }}
        fresh={asked.get("fresh") === "1"}
        full={asked.get("full") === "1"}
      />
    </Suspense>
  );
}
const oneCallCard = new URLSearchParams(location.hash.slice(1)).get("call-card");
/** `#browser-tile`: the Browser tile alone, for the Android app's place of
 * it (browser.tsx). */
const browserTile = new URLSearchParams(location.hash.slice(1)).has("browser-tile");
function BrowserTileAlone() {
  useTheme();
  return (
    <div className="fixed inset-0 bg-paper">
      <Suspense>
        <BrowserTile phone />
      </Suspense>
    </div>
  );
}
/** `#system-tile`: the System tile alone, for the Android app's place of it
 * (system.tsx). */
const systemTile = new URLSearchParams(location.hash.slice(1)).has("system-tile");
function SystemTileAlone() {
  useTheme();
  return (
    <div className="fixed inset-0 bg-paper">
      <Suspense>
        <SystemTile phone />
      </Suspense>
    </div>
  );
}
/** `#page-alone=<name>`: that Library page alone (library.tsx PageAlone). */
const onePage = new URLSearchParams(location.hash.slice(1)).get("page-alone");
// The app that shows a page alone names its theme (`&theme=dark`): its web
// view keeps no storage, so the stored choice index.html reads is absent.
// Applied, not stored: the app's theme is the app's.
/** `#builtin-alone=<id>`: a built-in's view alone (library.tsx BuiltinAlone). */
const oneBuiltin = new URLSearchParams(location.hash.slice(1)).get("builtin-alone");
if (onePage || oneBuiltin || oneCallCard || browserTile) {
  const asked = new URLSearchParams(location.hash.slice(1));
  document.documentElement.dataset.theme = asked.get("theme") === "dark" ? "dark" : "light";
  // And its paper (`&paper=E6D8C1`): the app paints flat where the web
  // paper is lighter, so a page or view on the web's would sit as a pale box.
  const paper = asked.get("paper");
  if (paper && /^[0-9a-f]{6}$/i.test(paper)) document.documentElement.style.setProperty("--paper", `#${paper}`);
}

// Mark whatever is scrolling, so index.css shows its bar until it rests.
const resting = new WeakMap<Element, number>();
addEventListener(
  "scroll",
  (e) => {
    const el = e.target instanceof Element ? e.target : document.documentElement;
    el.setAttribute("data-scrolling", "");
    clearTimeout(resting.get(el));
    resting.set(el, window.setTimeout(() => el.removeAttribute("data-scrolling"), 800));
  },
  { capture: true, passive: true },
);

// A pairing link (`libri pair`) carries its secret in the fragment, which
// never reaches a server or its logs: trade it for this device's token,
// kept as the session cookie, then drop it from the address and history.
async function pairFromLink() {
  const fragment = new URLSearchParams(location.hash.slice(1));
  // Another host's page sends its list of hosts along (hosts.tsx).
  takeHosts(fragment);
  const secret = fragment.get("pair");
  if (fragment.has("hosts") || secret) history.replaceState(null, "", location.pathname + location.search);
  if (!secret) return;
  const r = await fetch("api/pair", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ secret, name: navigator.userAgent }),
  });
  if (!r.ok) await tell({ title: "Pairing failed", body: await r.text() });
}

comeHome();
Promise.all([pairFromLink(), findHere()]).finally(() =>
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      {oneCard ? (
        <OneCard name={oneCard} />
      ) : oneCallCard ? (
        <OneCallCard name={oneCallCard} asked={new URLSearchParams(location.hash.slice(1))} />
      ) : onePage ? (
        <Suspense>
          <PageAlone name={onePage} />
        </Suspense>
      ) : browserTile ? (
        <BrowserTileAlone />
      ) : systemTile ? (
        <SystemTileAlone />
      ) : oneBuiltin ? (
        <Suspense>
          <BuiltinAlone id={oneBuiltin} />
        </Suspense>
      ) : (
        <App />
      )}
    </StrictMode>,
  ),
);
