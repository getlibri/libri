// The host the app talks to: the one picked in Settings, reached at its
// address as Paseo's app talks to a daemon it was pointed at, or else the
// one that served the page, if one did (a static host may serve the app alone).
// A picked host that needs a sign-in gets the token this browser holds for
// it as a bearer header, or as `?token=` where no header can go (the event
// stream, images, audio); an open one gets none. Picking a host reloads
// the page, so nothing from one host's chat lingers in another's.

/** The address of the host picked, kept per browser. */
const CURRENT = "libri-current";
const HOSTS = "libri-hosts";

type Remote = { url: string; token?: string };

/** This page's own origin and path, no trailing slash. */
export const self = () => (location.origin + location.pathname).replace(/\/+$/, "");

/** The host picked, with its token if it has one; null for the page's own. */
export function remote(): Remote | null {
  const url = localStorage.getItem(CURRENT);
  if (!url || url === self()) return null;
  try {
    const hosts: { url?: string; token?: string }[] = JSON.parse(localStorage.getItem(HOSTS) ?? "[]");
    const h = hosts.find((h) => h.url === url);
    return h ? { url, token: h.token } : null;
  } catch {
    return null;
  }
}

/** Whether the page came from a host, found once at start (`findHere`):
 * the app served on its own has no API beside it. */
let alone = false;
export const standalone = () => alone;
export const hostless = () => alone && !remote();
export async function findHere() {
  const r = await fetch("api/host", { cache: "no-store" }).catch(() => null);
  alone = !r?.ok || !r.headers.get("content-type")?.includes("json");
}

/** Talks to the host at `url` from now on (null: the page's own), afresh. */
export function use(url: string | null) {
  // The host page closes with the move, not stays open on the next host.
  if (location.hash === "#add-host") history.replaceState(null, "", location.pathname + location.search);
  if (url && url !== self()) localStorage.setItem(CURRENT, url);
  else localStorage.removeItem(CURRENT);
  location.reload();
}

/** `#here` in the address always comes back to the page's own host: the
 * way home when a picked host is gone and its page cannot be used. Typed
 * on the open page it only changes the fragment, so that counts too. */
export function comeHome() {
  const home = () => {
    if (location.hash !== "#here") return false;
    localStorage.removeItem(CURRENT);
    history.replaceState(null, "", location.pathname + location.search);
    return true;
  };
  home();
  window.addEventListener("hashchange", () => home() && location.reload());
}

/** Where an API path (`api/...`) is: on the picked host, with the token
 * in the query when `media` (for what cannot send a header). */
export function url(path: string, media = false) {
  const r = remote();
  if (!r) return path;
  const at = `${r.url}/${path}`;
  return media && r.token ? `${at}${at.includes("?") ? "&" : "?"}token=${encodeURIComponent(r.token)}` : at;
}

/** fetch for an API path, on whichever host is picked. */
export function api(path: string, init: RequestInit = {}) {
  const r = remote();
  if (!r) return fetch(path, init);
  const headers = new Headers(init.headers);
  if (r.token) headers.set("authorization", `Bearer ${r.token}`);
  return fetch(url(path), { ...init, headers, credentials: "omit" });
}
