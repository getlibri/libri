// Hosts and devices: each machine that runs Libri is a host the app knows
// by its stable id (/api/host), whatever address reaches it, as T3 Code
// knows its environments; a device pairs with each host for a token of its
// own (`libri pair`, or a link made here), revocable from Settings.
//
// The app talks to each host from this page (api.ts), as Paseo's app does
// to a daemon it is pointed at: an open host by its address alone. A pairing link waiting is opened at its host's own page, where
// the pairing leaves its cookie; the list travels there in the link's
// fragment, which never reaches a server, and tokens never travel at all.
import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { api, remote, self as here_, use } from "./api";
import { confirmAction } from "./dialog";

/** A host as /api/host describes it; addresses and url only when paired. */
export type HostInfo = { id: string; name: string; assistant?: string; version: string; commit?: string | null; uptime: number; addresses?: string[]; url?: string | null; admits?: boolean };
/** A host in this browser's list: where its page is, and a pairing link not yet opened. */
export type Host = { id: string; name: string; url: string; pair?: string; token?: string };
type Device = { id: string; name: string; scope: "full" | "read"; paired: number; this: boolean };

const HOSTS = "libri-hosts";
const CHANGED = "libri-hosts-changed";

const listen = (then: () => void) => {
  window.addEventListener(CHANGED, then);
  window.addEventListener("storage", then);
  return () => {
    window.removeEventListener(CHANGED, then);
    window.removeEventListener("storage", then);
  };
};
// A host's daemon speaks plain http on its own port, as Paseo's does, so an
// address typed bare ("100.64.0.7", "box") means http on Libri's port.
const PORT = "6780";
const typed = (address: string) => {
  const t = address.trim();
  const u = new URL(t.includes("://") ? t : `http://${t}`);
  if (!t.includes("://") && !u.port) u.port = PORT;
  return u;
};
const parse = (raw: string): Host[] => {
  try {
    const hosts = JSON.parse(raw);
    return Array.isArray(hosts) ? hosts.filter((h) => typeof h?.url === "string") : [];
  } catch {
    return [];
  }
};
const save = (hosts: Host[]) => {
  localStorage.setItem(HOSTS, JSON.stringify(hosts));
  window.dispatchEvent(new Event(CHANGED));
};
const stored = () => parse(localStorage.getItem(HOSTS) ?? "[]");

/** Where a host's page is: its origin and path, no fragment, no trailing slash. */
export const origin = (url: string) => {
  const u = new URL(url, location.href);
  return (u.origin + u.pathname).replace(/\/+$/, "");
};

/** Adds hosts to the list, or updates the ones already in it, by id or address. */
export function remember(...hosts: Host[]) {
  const all = stored();
  for (const h of hosts) {
    const at = all.findIndex((x) => (h.id && x.id === h.id) || x.url === h.url);
    if (at < 0) all.push(h);
    else all[at] = { ...all[at], ...h, pair: h.pair ?? all[at].pair, token: h.token ?? all[at].token };
  }
  save(all);
}

/** Reads the hosts another host's page sent along in the fragment (`hosts=`). */
export function takeHosts(fragment: URLSearchParams) {
  const raw = fragment.get("hosts");
  if (!raw) return;
  try {
    remember(...parse(atob(raw.replace(/-/g, "+").replace(/_/g, "/"))));
  } catch {
    // A mangled list is no list; this host is added on its own anyway.
  }
}

/** The link that opens a host's page with this browser's list, and its pairing if one waits. */
function linkTo(host: Host, all: Host[]) {
  // Pairing secrets stay with the host they belong to.
  const list = all.map(({ id, name, url }) => ({ id, name, url }));
  const hosts = btoa(JSON.stringify(list)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  const fragment = new URLSearchParams(host.pair ? { pair: host.pair, hosts } : { hosts });
  return `${host.url}/#${fragment}`;
}

/** This app at the other scheme, with this browser's list along. Chromium
 * lets only a secure page reach the local network (after its prompt), and
 * then lets it call a plain-http address there; Safari refuses plain http
 * from a secure page outright. So the app is served both ways, and a host
 * one copy cannot reach is offered to the other. */
function otherCopy(all: Host[]) {
  const list = all.map(({ id, name, url }) => ({ id, name, url }));
  const hosts = btoa(JSON.stringify(list)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  const scheme = location.protocol === "https:" ? "http:" : "https:";
  return `${scheme}//${location.host}${location.pathname}#hosts=${hosts}`;
}

/** Pairs this browser with the host at `url` by a pairing secret, for a
 * token of its own. */
async function signIn(url: string, secret: string) {
  const r = await fetch(`${url}/api/pair`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ secret, name: navigator.userAgent }),
    credentials: "omit",
  }).catch(() => null);
  if (!r) return { error: "It does not answer." };
  if (!r.ok) return { error: (await r.text()) || `That failed (${r.status}).` };
  const got: { token: string; host: { id: string; name: string } } = await r.json();
  return { host: { id: got.host.id, name: got.host.name, url, token: got.token } as Host };
}

/** Asks a host who it is, timed. */
async function probe(url: string): Promise<{ info: HostInfo; ms: number } | null> {
  const start = performance.now();
  try {
    const r = await fetch(`${url}/api/host`, { cache: "no-store", credentials: "omit", signal: AbortSignal.timeout(5000) });
    if (!r.ok) return null;
    const info: HostInfo = await r.json();
    return { info, ms: Math.round(performance.now() - start) };
  } catch {
    return null;
  }
}

/** Why a host added from this page does not answer, as far as a page can
 * tell, and the other copy of the app when that one could reach it: the
 * browser's Local Network Access permission refused (Chromium asks before
 * a public page reaches a private or tailnet address, and a refusal is
 * silent), a plain-http page Chromium keeps off the network, or a
 * plain-http host a secure page may not call (Safari). */
async function unanswered(url: string): Promise<{ said: string; other?: boolean }> {
  const loopback = ["localhost", "127.0.0.1", "[::1]"].includes(new URL(url).hostname);
  // Chrome 145 split the permission in two; 142-144 know the older name.
  for (const name of [loopback ? "loopback-network" : "local-network", "local-network-access"]) {
    const state = await navigator.permissions?.query({ name: name as PermissionName }).then((s) => s.state, () => null);
    if (state === "denied") return { said: "Added, but this browser keeps the page off your network: allow Local network access in this site's settings, then pick it." };
  }
  if (!isSecureContext && !loopback) return { said: "Added, but this browser reaches your network only from the secure copy of this page.", other: true };
  if (isSecureContext && url.startsWith("http://") && !loopback) return { said: "Added, but this browser refuses a plain-http host from a secure page.", other: true };
  return { said: "Added, but it does not answer yet." };
}

/** The host in use as it describes itself to this device (paired: with
 * its other addresses), and the address it answered at. */
export function useThisHost(want: boolean) {
  const [info, setInfo] = useState<(HostInfo & { at: string }) | null>(null);
  useEffect(() => {
    if (!want) return;
    const at = remote()?.url ?? here_();
    api("api/host", { cache: "no-store" })
      .then((r) => (r.ok ? r.json() : null))
      .then((i: HostInfo | null) => setInfo(i && { ...i, at }))
      .catch(() => {});
  }, [want]);
  return info;
}

type Seen = { ms: number; info: HostInfo } | "down" | undefined;

/** Every host this browser knows, asked every 15 seconds while `want` holds:
 * the list, each one's latency or down by address, and the one in use. */
export function useKnownHosts(want: boolean) {
  const raw = useSyncExternalStore(listen, () => localStorage.getItem(HOSTS) ?? "[]");
  const here = useThisHost(want);
  const [seen, setSeen] = useState<Record<string, Seen>>({});
  // The host the app talks to: the page's own unless another was picked.
  const talking = remote()?.url ?? origin(location.href);
  // It is always listed, under the id it gives at the address it was
  // asked at: a host picked since is not it.
  useEffect(() => {
    if (here) remember({ id: here.id, name: here.name, url: here.at });
  }, [here]);
  const hosts = parse(raw);

  const look = useCallback(async (list: Host[]) => {
    const got = await Promise.all(list.map(async (h) => [h.url, await probe(h.url)] as const));
    setSeen(Object.fromEntries(got.map(([url, p]) => [url, p ?? "down"])));
    // A host that answers names itself: keep its id and name current.
    const named = got.flatMap(([url, p]) => (p ? [{ url, id: p.info.id, name: p.info.name }] : []));
    if (named.some((n) => list.some((h) => h.url === n.url && (h.id !== n.id || h.name !== n.name)))) remember(...named);
  }, []);
  useEffect(() => {
    if (!want) return;
    void look(stored());
    const every = setInterval(() => void look(stored()), 15000);
    return () => clearInterval(every);
  }, [want, look, raw.length]);
  return { hosts, seen, talking };
}

/** A press that moves the app to a host, or opens the pairing waiting at its own page. */
export function HostLink({ host, hosts, label, title, className, children }: { host: Host; hosts: Host[]; label?: string; title?: string; className: string; children: React.ReactNode }) {
  // Talked to from this page, unless a pairing waits at its own.
  const inPlace = !host.pair;
  return (
    <a
      href={inPlace ? undefined : linkTo(host, hosts)}
      role="button"
      aria-label={label}
      title={title}
      className={`${className} cursor-pointer`}
      onClick={() => (inPlace ? use(host.url === here_() ? null : host.url) : host.pair && remember({ ...host, pair: undefined }))}
    >
      {children}
    </a>
  );
}

// The host page (`#add-host`) opens over the host in use as a history entry of
// its own, so Back, Cancel and the browser's back all return to that host
// as it was; only a host that answers replaces it.
const PICKING = "#add-host";
const PICKED = "libri-picking";
const onPicking = (then: () => void) => {
  window.addEventListener("popstate", then);
  window.addEventListener("hashchange", then);
  window.addEventListener(PICKED, then);
  return () => {
    window.removeEventListener("popstate", then);
    window.removeEventListener("hashchange", then);
    window.removeEventListener(PICKED, then);
  };
};
/** Whether the host page is open over the host in use. */
export const usePicking = () => useSyncExternalStore(onPicking, () => location.hash === PICKING);
/** Opens the host page over the host in use. */
export function openPicking() {
  history.pushState({ picking: true }, "", PICKING);
  window.dispatchEvent(new Event(PICKED));
}
/** Closes the host page, back to the host in use: the entry it pushed is
 * popped; one the page was loaded at is replaced, so back never leaves. */
export function closePicking() {
  if (location.hash !== PICKING) return;
  if (history.state?.picking) return history.back();
  history.replaceState(null, "", location.pathname + location.search);
  window.dispatchEvent(new Event(PICKED));
}

/** A known host's row: its state, name and address; a press switches to it
 * and the cross forgets it, except the one in use. */
function HostRow({ host: h, hosts, seen: s, current, note }: { host: Host; hosts: Host[]; seen: Seen; current: boolean; note: string }) {
  return (
    <div className="group flex w-full items-center gap-2 px-5 py-2 text-body">
      <span className={`size-2 shrink-0 rounded-full ${s === "down" ? "bg-accent" : s ? "bg-active" : "bg-soft"}`} aria-label={s === "down" ? "Down" : s ? "Up" : "Checking"} />
      {current ? (
        <span className="min-w-0 flex-1">
          <span className="block truncate">{h.name}</span>
          <span className={`${note} block truncate`}>In use · {new URL(h.url).host}</span>
        </span>
      ) : (
        <HostLink host={h} hosts={hosts} label={`Switch to ${h.name}`} className="min-w-0 flex-1">
          <span className="block truncate">{h.name}</span>
          <span className={`${note} block truncate`}>
            {h.pair ? "Opens to pair · " : ""}
            {h.id ? `${h.id.slice(0, 8)} · ` : ""}
            {new URL(h.url).host}
          </span>
        </HostLink>
      )}
      <span className={`${note} shrink-0 tabular-nums`}>{s === "down" ? "Down" : s ? `${s.ms} ms` : "…"}</span>
      {!current && (
        <button type="button" onClick={() => save(hosts.filter((x) => x.url !== h.url))} aria-label={`Forget ${h.name}`} title="Forget this host" className="grid size-7 shrink-0 place-items-center rounded-full text-mid hover:bg-hover hover:text-ink">
          ×
        </button>
      )}
    </div>
  );
}

/** Settings' Hosts page: every host this browser knows, each with
 * its latency or down, and switching. `start` opens the address form, and
 * its Cancel then calls `cancel`. */
export function Hosts({ open, row, note, start = false, cancel }: { open: boolean; row: string; note: string; start?: boolean; cancel?: () => void }) {
  const { hosts, seen, talking } = useKnownHosts(open);
  const [adding, setAdding] = useState(start);
  const [address, setAddress] = useState("");
  const [said, setSaid] = useState<string>();
  const [other, setOther] = useState<string>();

  const add = async () => {
    setSaid(undefined);
    setOther(undefined);
    let url: string;
    let pair: string | undefined;
    try {
      const u = typed(address);
      pair = new URLSearchParams(u.hash.slice(1)).get("pair") ?? undefined;
      u.hash = "";
      url = origin(u.href);
    } catch {
      return setSaid("That is not an address.");
    }
    if (pair) {
      const got = await signIn(url, pair);
      if ("error" in got) return setSaid(got.error);
      remember(got.host);
      return use(url);
    }
    const p = await probe(url);
    remember({ id: p?.info.id ?? "", name: p?.info.name ?? new URL(url).host, url });
    setAddress("");
    setAdding(false);
    if (!p) {
      const why = await unanswered(url);
      if (why.other) setOther(otherCopy(stored()));
      return setSaid(why.said);
    }
    // Hosts before 0.9.30 do not say; they are tried.
    if (p.info.admits === false) return setSaid(`Added, but it does not let this page in: add ${location.origin} to its allowed origins (daemon.origins).`);
    use(url);
  };

  const input = "w-full rounded-lg border border-frame bg-paper px-2.5 py-1.5 text-control outline-none focus:border-soft";
  return (
    <>
      {hosts.map((h) => (
        <HostRow key={h.url} host={h} hosts={hosts} seen={seen[h.url]} current={h.url === talking} note={note} />
      ))}
      {adding ? (
        <form
          className="flex flex-col gap-2 px-5 py-2"
          onSubmit={(e) => {
            e.preventDefault();
            void add();
          }}
        >
          <input autoFocus value={address} onChange={(e) => setAddress(e.target.value)} placeholder="Tailnet address, or a pairing link" aria-label="Address or pairing link" className={input} />
          <span className="flex justify-end gap-2">
            <button type="button" onClick={() => (cancel ? cancel() : setAdding(false))} className="rounded-full px-3 py-1 text-label text-mid hover:bg-hover">
              Cancel
            </button>
            <button type="submit" disabled={!address.trim()} className="rounded-full bg-ink px-3 py-1 text-label text-paper disabled:opacity-40">
              Add
            </button>
          </span>
        </form>
      ) : (
        <button type="button" onClick={() => setAdding(true)} className={row}>
          Add a host
          <span className={note}>By address or link</span>
        </button>
      )}
      {said && <p className={`${note} px-5 pb-1`}>{said}</p>}
      {other && (
        <p className={`${note} px-5 pb-1`}>
          <a href={other} className="underline">Open it there</a>
        </p>
      )}
    </>
  );
}

/** Settings: the devices paired with this host, to revoke, and a link to pair another. */
export function Devices({ open, row, note, read }: { open: boolean; row: string; note: string; read: boolean }) {
  const [devices, setDevices] = useState<Device[] | null>(null);
  const [links, setLinks] = useState<string[]>();
  const [said, setSaid] = useState<string>();
  const load = useCallback(async () => {
    const r = await api("api/devices", { cache: "no-store" }).catch(() => null);
    setDevices(r?.ok ? await r.json() : null);
  }, []);
  useEffect(() => {
    if (open) void load();
  }, [open, load]);
  const revoke = async (d: Device) => {
    if (!(await confirmAction({ title: d.this ? "Unpair this device?" : `Unpair ${d.name}?`, body: "It signs out at once.", confirm: "Unpair", danger: true }))) return;
    const r = await api(`api/devices/${encodeURIComponent(d.id)}/revoke`, { method: "POST" });
    if (!r.ok) return setSaid(await r.text());
    if (d.this) return location.reload();
    void load();
  };
  const make = async (readOnly: boolean) => {
    setSaid(undefined);
    const r = await api("api/pair/new", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ read: readOnly }) });
    if (!r.ok) return setSaid(r.status === 403 ? "This device may only read." : `That failed (${r.status}).`);
    setLinks((await r.json()).links);
  };
  if (devices === null) return null;
  return (
    <>
      {read && <p className={`${note} px-5 pb-1`}>Read only: sees the chat, changes nothing.</p>}
      {devices.length === 0 && <p className={`${note} px-5 py-1`}>None yet.</p>}
      {devices.map((d) => (
        <div key={d.id} className="flex w-full items-center gap-2 px-5 py-2 text-body">
          <span className="min-w-0 flex-1">
            <span className="block truncate" title={d.name}>
              {shortName(d.name)}
            </span>
            <span className={`${note} block`}>
              {d.this ? "This device · " : ""}
              {d.scope === "read" ? "Read only" : "Full"} · paired {new Date(d.paired * 1000).toLocaleDateString()}
            </span>
          </span>
          {!read && (
            <button type="button" onClick={() => void revoke(d)} className="shrink-0 rounded-full px-2.5 py-1 text-label text-mid hover:bg-hover hover:text-ink">
              Revoke
            </button>
          )}
        </div>
      ))}
      {!read &&
        (links ? (
          <div className="flex flex-col gap-1.5 px-5 py-2">
            <span className={note}>Open on the other device within ten minutes:</span>
            {links.map((l) => (
              <button key={l} type="button" onClick={() => void navigator.clipboard?.writeText(l).then(() => setSaid("Copied."))} title="Copy" className="truncate rounded-lg bg-block px-2.5 py-1.5 text-left font-mono text-caption">
                {l}
              </button>
            ))}
            <button type="button" onClick={() => setLinks(undefined)} className={`${note} self-end hover:text-ink`}>
              Done
            </button>
          </div>
        ) : (
          <>
            <button type="button" onClick={() => void make(false)} className={row}>
              Pair a device
              <span className={note}>Full</span>
            </button>
            <button type="button" onClick={() => void make(true)} className={row}>
              Pair a device to read
              <span className={note}>Read only</span>
            </button>
          </>
        ))}
      {said && <p className={`${note} px-5 pb-1`}>{said}</p>}
    </>
  );
}

/** A browser's long user agent, cut to what tells devices apart. */
function shortName(name: string) {
  // In order: an Android phone says Linux too, and Chrome says Safari.
  const os = ["iPhone", "iPad", "Android", "CrOS", "Mac OS X", "Windows", "Linux"].find((o) => name.includes(o));
  const browser = ["Firefox", "Edg", "Chrome", "Safari"].find((b) => name.includes(b));
  if (!name.includes("Mozilla/") || !os) return name;
  const label = { "Mac OS X": "Mac", Edg: "Edge", CrOS: "ChromeOS" } as Record<string, string>;
  return [browser && (label[browser] ?? browser), label[os] ?? os].filter(Boolean).join(" on ");
}

type Peer = { name: string; url: string; state: "asking" | "asked" | "paired"; last?: number };

/** Settings: other people's assistants this one is paired with, to message
 * each other, and the requests either way: pair by address, approve, revoke. */
export function Peers({ open, row, note, read }: { open: boolean; row: string; note: string; read: boolean }) {
  const [peers, setPeers] = useState<Peer[] | null>(null);
  const [adding, setAdding] = useState(false);
  const [address, setAddress] = useState("");
  const [said, setSaid] = useState<string>();
  const load = useCallback(async () => {
    const r = await api("api/peers", { cache: "no-store" }).catch(() => null);
    setPeers(r?.ok ? await r.json() : null);
  }, []);
  useEffect(() => {
    if (open) void load();
  }, [open, load]);
  const post = async (path: string, url: string) => {
    setSaid(undefined);
    const r = await api(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ url }) }).catch(() => null);
    if (!r) return setSaid("Couldn't reach this host."), false;
    if (!r.ok) return setSaid(await r.text()), false;
    void load();
    return true;
  };
  const ask = async () => {
    if (await post("api/peers", address.trim())) {
      setAdding(false);
      setAddress("");
      setSaid("Asked. It pairs once they approve.");
    }
  };
  const revoke = async (p: Peer) => {
    const asked =
      p.state === "asked"
        ? { title: `Decline ${p.name}'s request?`, confirm: "Decline" }
        : p.state === "asking"
          ? { title: "Withdraw this request?", confirm: "Withdraw" }
          : { title: `Unpair ${p.name}?`, body: "You can't message each other until you pair again.", confirm: "Unpair" };
    if (await confirmAction({ ...asked, danger: true })) void post("api/peers/revoke", p.url);
  };
  if (peers === null) return null;
  const status = (p: Peer) =>
    p.state === "asking" ? "Waiting for them to approve" : p.state === "asked" ? "Asks to pair" : p.last ? `Last message ${new Date(p.last * 1000).toLocaleString()}` : "Paired";
  return (
    <>
      {peers.length === 0 && <p className={`${note} px-5 py-1`}>None yet. Pair with another assistant to message each other.</p>}
      {peers.map((p) => (
        <div key={p.url} className="flex w-full items-center gap-2 px-5 py-2 text-body">
          <span className="min-w-0 flex-1">
            <span className="block truncate" title={p.url}>
              {p.name || p.url.replace(/^https?:\/\//, "")}
            </span>
            <span className={`${note} block`}>{status(p)}</span>
          </span>
          {!read && p.state === "asked" && (
            <button type="button" onClick={() => void post("api/peers/approve", p.url)} className="shrink-0 rounded-full bg-block px-2.5 py-1 text-label text-ink hover:bg-hover">
              Approve
            </button>
          )}
          {!read && (
            <button type="button" onClick={() => void revoke(p)} className="shrink-0 rounded-full px-2.5 py-1 text-label text-mid hover:bg-hover hover:text-ink">
              {p.state === "asked" ? "Decline" : p.state === "asking" ? "Withdraw" : "Revoke"}
            </button>
          )}
        </div>
      ))}
      {!read &&
        (adding ? (
          <form
            className="flex items-center gap-2 px-5 py-2"
            onSubmit={(e) => {
              e.preventDefault();
              void ask();
            }}
          >
            <input autoFocus value={address} onChange={(e) => setAddress(e.target.value)} placeholder="http://127.0.0.1:8952" aria-label="The other assistant's address" className="min-w-0 flex-1 rounded-lg bg-block px-2.5 py-1.5 text-body outline-none" />
            <button type="submit" disabled={!address.trim()} className="shrink-0 rounded-full bg-block px-2.5 py-1 text-label text-ink hover:bg-hover disabled:opacity-50">
              Ask
            </button>
            <button type="button" onClick={() => setAdding(false)} className={`${note} shrink-0 hover:text-ink`}>
              Cancel
            </button>
          </form>
        ) : (
          <button type="button" onClick={() => setAdding(true)} className={row}>
            Pair with an assistant
            <span className={note}>By address</span>
          </button>
        ))}
      {said && <p className={`${note} px-5 pb-1`}>{said}</p>}
    </>
  );
}
