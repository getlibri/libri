import { useCallback, useEffect, useRef, useState } from "react";
import { api, remote } from "./api";
import { confirmAction } from "./dialog";
import { Toggle, toggleButton } from "./toggle";

export type Server = {
  name: string;
  address: string;
  remote: boolean;
  /** The names of its headers or variables; their values never leave the host. */
  secrets: string[];
  withheld: string[];
  /** Each tool its server listed last, with the person's rule and the one
   * its annotations give. */
  tools: { name: string; description?: string | null; rule: Rule; default: Rule }[];
  on: boolean;
  signed_in: boolean;
  account: string | null;
};

export type Rule = "allow" | "ask" | "off";
const RULES: [Rule, string][] = [["allow", "Allow"], ["ask", "Ask"], ["off", "Off"]];

type Connections = {
  servers: Server[];
  xai: string | null;
  xai_signing: { url: string; code: string } | null;
  xai_said: string | null;
};

type Tested = { sign_in: boolean; tools: { name: string; description?: string; withheld: boolean }[] };

/** The person's connections: MCP servers added by name and address, as
 * Claude Desktop's custom connectors and `claude mcp add` take them, which
 * every harness here then shares. A server that signs in by OAuth returns
 * the browser to this host's loopback address: on the host that finishes
 * the sign-in; on any other device the page fails to load, and its address
 * is pasted here. */
export function useConnections(open: boolean) {
  const [now, setNow] = useState<Connections | null>(null);
  const load = useCallback(async () => {
    const r = await api("api/connections", { cache: "no-store" }).catch(() => null);
    setNow(r?.ok ? await r.json() : null);
  }, []);
  useEffect(() => {
    if (open) void load();
  }, [open, load]);
  return [now, load] as const;
}

const said = async (r: Response | null) => (r ? (await r.text()) || `That failed (${r.status}).` : "The host does not answer.");
const field = "w-full rounded-lg border border-frame bg-paper px-3 py-2 text-caption outline-none focus:border-soft";
const small = "shrink-0 rounded-full px-2.5 py-1 text-label text-mid hover:bg-hover hover:text-ink";
/** The one on/off switch of Settings, as the harnesses' (App.tsx): a ringed pill. */
export function Connections({ now, load, row, note, read }: { now: Connections | null; load: () => Promise<void>; row: string; note: string; read: boolean }) {
  // The one being added ("" for a new one) or edited, by name.
  const [editing, setEditing] = useState<string | null>(null);
  if (!now) return null;
  return (
    <>
      {!read && <Providers load={load} row={row} note={note} />}
      {now.servers.map((s) =>
        editing === s.name ? (
          <ServerForm key={s.name} was={s} done={() => { setEditing(null); void load(); }} row={row} note={note} />
        ) : (
          <ServerRow key={s.name} s={s} load={load} edit={() => setEditing(s.name)} note={note} row={row} read={read} />
        ),
      )}
      <HostServers note={note} />
      {!read &&
        (editing === "" ? (
          <ServerForm done={() => { setEditing(null); void load(); }} row={row} note={note} />
        ) : (
          <button type="button" onClick={() => setEditing("")} className={row}>
            Add another MCP server
            <span className={note}>A URL, or a command run on this host</span>
          </button>
        ))}
      {read && <p className={`${note} px-5 pb-2`}>This device may only read.</p>}
    </>
  );
}

/** Signing a connection in: its button, and below it, once started, where
 * the address the server ends on is pasted when the sign-in cannot come back
 * by itself. ServerRow and a built-in's provider row both use it. */
function useSignIn(name: string, signedIn: boolean, load: () => Promise<void>, say: (words: string | undefined) => void, row: string, note: string) {
  return useSignInAs(name, signedIn, load, say, row, note);
}

/** As useSignIn, its sign-in also started by `start(tab, name)`: for a
 * connection added by the same tap, the tab opened before anything waited. */
function useSignInAs(name: string, signedIn: boolean, load: () => Promise<void>, say: (words: string | undefined) => void, row: string, note: string) {
  const [signing, setSigning] = useState(false);
  const [busy, setBusy] = useState(false);
  const [address, setAddress] = useState("");
  // Signed in on this machine, the sign-in finishes in the other tab:
  // coming back here shows it.
  useEffect(() => {
    if (!signing) return;
    const back = () => void load();
    window.addEventListener("focus", back);
    return () => window.removeEventListener("focus", back);
  }, [signing, load]);
  useEffect(() => {
    if (signedIn) setSigning(false);
  }, [signedIn]);
  const signIn = async (given?: Window | null, as?: string) => {
    say(undefined);
    // Opened before the request answers, so the browser counts it as the
    // tap's and does not block it.
    const tab = given === undefined ? window.open("", "_blank") : given;
    // The address this page reaches the host at, which the sign-in comes
    // back to: the picked host's, or the page's own.
    const origin = new URL(remote()?.url ?? location.href).origin;
    const r = await api(`api/connections/servers/${encodeURIComponent(as ?? name)}/sign-in`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ origin }),
    }).catch(() => null);
    if (!r?.ok) {
      tab?.close();
      return say(await said(r));
    }
    const { url } = await r.json();
    if (tab) tab.location.href = url;
    else location.href = url;
    setSigning(true);
  };
  const finish = async () => {
    setBusy(true);
    say(undefined);
    const r = await api("api/connections/finish", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ address: address.trim() }),
    }).catch(() => null);
    setBusy(false);
    if (!r?.ok) return say(await said(r));
    setSigning(false);
    setAddress("");
    await load();
  };
  const button = <button type="button" onClick={() => void signIn()} className={small}>{signedIn ? "Sign in again" : "Sign in"}</button>;
  const paste = signing && !signedIn && (
    <form
      className="flex flex-col gap-1.5 px-5 py-2"
      onSubmit={(e) => {
        e.preventDefault();
        void finish();
      }}
    >
      <span className={note}>Not back in Libri? Paste the address of the page you ended on.</span>
      <input value={address} onChange={(e) => setAddress(e.target.value)} placeholder="https://…/connect/back?code=…" aria-label="Address of the page the server sent you to" autoComplete="off" className={`${field} font-mono`} />
      <button disabled={busy || !address.trim()} className={`${row} justify-center rounded-lg`}>Finish signing in</button>
    </form>
  );
  return { button, paste, start: signIn };
}

type Offered = { id: string; title: string; note: string | null; covers: string[]; connection: string | null; signed_in: boolean };

/** The providers Libri knows, as a phone's Add account lists them: one
 * tap adds the connection and opens its sign-in, and every built-in it
 * covers takes it. One connected and signed in is not offered again. */
function Providers({ load, row, note }: { load: () => Promise<void>; row: string; note: string }) {
  const [all, setAll] = useState<Offered[]>();
  const get = useCallback(async () => {
    const r = await api("api/connections/providers", { cache: "no-store" }).catch(() => null);
    setAll(r?.ok ? await r.json() : undefined);
  }, []);
  useEffect(() => {
    void get();
  }, [get]);
  const open = all?.filter((p) => !p.signed_in) ?? [];
  if (!open.length) return null;
  return (
    <div className="pb-1" aria-label="Add a connection" role="group">
      <p className={`${note} px-5 pt-1`}>Add a connection</p>
      {open.map((p) => (
        <ProviderRow key={p.id} p={p} row={row} note={note} load={async () => { await Promise.all([get(), load()]); dispatchEvent(new Event("libri:builtins")); }} />
      ))}
    </div>
  );
}

function ProviderRow({ p, row, note, load }: { p: Offered; row: string; note: string; load: () => Promise<void> }) {
  const [words, setWords] = useState<string>();
  const [name, setName] = useState(p.connection ?? "");
  const sign = useSignInAs(name, p.signed_in, load, setWords, row, note);
  const connect = async () => {
    setWords(undefined);
    const tab = window.open("", "_blank");
    const r = await api(`api/connections/providers/${encodeURIComponent(p.id)}`, { method: "POST" }).catch(() => null);
    if (!r?.ok) {
      tab?.close();
      return setWords(await said(r));
    }
    const { name: made } = (await r.json()) as { name: string };
    setName(made);
    await load();
    await sign.start(tab, made);
  };
  return (
    <>
      <button type="button" onClick={() => void connect()} className={row}>
        <span className="min-w-0">
          <span className="block">{p.connection ? `Sign in to ${p.title}` : p.title}</span>
          <span className={`${note} block`}>{p.covers.join(", ")}</span>
        </span>
        <span className={note}>{p.connection ? "Sign in" : "Connect"}</span>
      </button>
      {sign.paste}
      {words && <p className={`${note} px-5 py-1`}>{words}</p>}
    </>
  );
}

/** One tool's rule, as a three-way switch. */
function RuleSwitch({ server, tool, rule, read, changed }: { server: string; tool: string; rule: Rule; read: boolean; changed: () => Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const put = async (to: Rule) => {
    if (to === rule) return;
    setBusy(true);
    await api(`api/connections/servers/${encodeURIComponent(server)}/tools`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ tool, rule: to }),
    }).catch(() => null);
    await changed();
    setBusy(false);
  };
  return (
    <span role="radiogroup" aria-label={`${tool}`} className="flex shrink-0 rounded-full border border-frame p-0.5">
      {RULES.map(([r, label]) => (
        <button
          key={r}
          type="button"
          role="radio"
          aria-checked={rule === r}
          disabled={read || busy}
          onClick={() => void put(r)}
          className={`min-h-8 rounded-full px-2.5 text-label max-lg:min-h-10 ${rule === r ? "bg-ink text-paper" : "text-mid hover:text-ink"}`}
        >
          {label}
        </button>
      ))}
    </span>
  );
}

/** A connection's tools, each with Allow, Ask or Off: folded to a count
 * until opened, as Claude's connector settings list them. */
function Tools({ s, read, note, load }: { s: Server; read: boolean; note: string; load: () => Promise<void> }) {
  const [open, setOpen] = useState(false);
  if (!s.tools.length) return null;
  const asks = s.tools.filter((t) => t.rule === "ask").length;
  const off = s.tools.filter((t) => t.rule === "off").length;
  return (
    <div className="px-5 py-1">
      <button type="button" aria-expanded={open} onClick={() => setOpen(!open)} className={`${note} min-h-9 hover:text-ink`}>
        {s.tools.length} tools{asks ? ` · ${asks} ask first` : ""}{off ? ` · ${off} off` : ""} {open ? "▴" : "▾"}
      </button>
      {open && (
        <ul aria-label={`${s.name} tools`} className="flex flex-col">
          {s.tools.map((t) => (
            <li key={t.name} className="flex min-h-11 items-center gap-3">
              <span className="min-w-0 flex-1">
                <span className="block truncate font-mono text-caption text-ink">{t.name}</span>
                {t.description && <span className={`${note} block truncate`} title={t.description}>{t.description}</span>}
              </span>
              <RuleSwitch server={s.name} tool={t.name} rule={t.rule} read={read} changed={load} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

type Kind = { kind: string; title: string; failed: string | null; items: { id: string; name: string; color: string | null; shown: boolean }[] };

/** What a connection brings in (Fastmail's calendars), each shown or
 * hidden: the Calendar view and the agent's own searches leave out a
 * hidden one. */
function Brings({ s, read, note }: { s: Server; read: boolean; note: string }) {
  const [kinds, setKinds] = useState<Kind[]>();
  const path = `api/connections/servers/${encodeURIComponent(s.name)}/items`;
  const get = useCallback(async () => {
    const r = await api(path, { cache: "no-store" }).catch(() => null);
    setKinds(r?.ok ? await r.json() : undefined);
  }, [path]);
  // A server run here needs no sign-in.
  const ready = (s.signed_in || !s.remote) && s.on;
  useEffect(() => {
    if (ready) void get();
  }, [ready, get]);
  const show = async (kind: string, id: string, shown: boolean) => {
    await api(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ kind, id, shown }) }).catch(() => null);
    await get();
    dispatchEvent(new Event("libri:builtins"));
  };
  if (!kinds?.length) return null;
  return (
    <>
      {kinds.map((k) => (
        <div key={k.kind} className="px-5 py-1" role="group" aria-label={`${s.name} ${k.title}`}>
          <p className={note}>{k.title}</p>
          {k.failed && <p className={note}>{k.failed}</p>}
          {k.items.map((i) => (
            <div key={i.id} className="flex min-h-11 items-center gap-3 text-body">
              <span className="size-2.5 shrink-0 rounded-full border border-hair" style={i.color ? { background: i.color } : undefined} />
              <span className="min-w-0 flex-1 truncate">{i.name}</span>
              <button type="button" role="switch" aria-checked={i.shown} aria-label={`Show ${i.name}`} disabled={read} onClick={() => void show(k.kind, i.id, !i.shown)} className={toggleButton}>
                <Toggle on={i.shown} />
              </button>
            </div>
          ))}
        </div>
      ))}
    </>
  );
}

function ServerRow({ s, load, edit, note, row, read }: { s: Server; load: () => Promise<void>; edit: () => void; note: string; row: string; read: boolean }) {
  const [tested, setTested] = useState<Tested>();
  const [busy, setBusy] = useState(false);
  const [words, setWords] = useState<string>();
  const path = `api/connections/servers/${encodeURIComponent(s.name)}`;
  const sign = useSignIn(s.name, s.signed_in, load, setWords, row, note);
  const test = async () => {
    setBusy(true);
    setWords(undefined);
    setTested(undefined);
    const r = await api(`${path}/test`, { method: "POST" }).catch(() => null);
    setBusy(false);
    if (!r?.ok) return setWords(await said(r));
    setTested(await r.json());
    // The test lists the tools through the relay, which keeps them for here.
    await load();
  };
  const toggle = async () => {
    const r = await api("api/connections/servers", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ name: s.name, address: s.address, withheld: s.withheld, on: !s.on }),
    }).catch(() => null);
    if (!r?.ok) return setWords(await said(r));
    await load();
  };
  const remove = async () => {
    if (!(await confirmAction({ title: `Remove ${s.name}?`, body: "Its tools and sign-in are removed.", confirm: "Remove", danger: true }))) return;
    const r = await api(`${path}/remove`, { method: "POST" }).catch(() => null);
    if (!r?.ok) return setWords(await said(r));
    await load();
  };
  const state = s.signed_in ? (s.account ? `Signed in as ${s.account}` : "Signed in") : s.secrets.length ? s.secrets.join(", ") : null;
  return (
    <div className="py-1">
      <div className="flex w-full items-center gap-2 px-5 py-1 text-body">
        <span className="min-w-0 flex-1">
          <span className="block">{s.name}</span>
          <span className={`${note} block truncate font-mono`} title={s.address}>{s.address}</span>
          {state && <span className={`${note} block truncate`}>{state}</span>}
        </span>
        {!read && (
          <button type="button" role="switch" aria-checked={s.on} aria-label={`${s.name} on`} onClick={() => void toggle()} className={toggleButton}>
            <Toggle on={s.on} />
          </button>
        )}
      </div>
      {!read && (
        <div className="flex flex-wrap gap-1 px-4">
          <button type="button" disabled={busy} onClick={() => void test()} className={small}>{busy ? "Testing…" : "Test"}</button>
          {s.remote && sign.button}
          <button type="button" onClick={edit} className={small}>Edit</button>
          <button type="button" onClick={() => void remove()} className={small}>Remove</button>
        </div>
      )}
      {sign.paste}
      {tested?.sign_in && <p className={`${note} px-5 py-1`}>{s.name} wants you to sign in first.</p>}
      {tested && !tested.sign_in && !s.tools.length && <p className={`${note} px-5 py-1`}>{tested.tools.length} tools</p>}
      <Brings s={s} read={read} note={note} />
      <Tools s={s} read={read} note={note} load={load} />
      {words && <p className={`${note} px-5 py-1`}>{words}</p>}
    </div>
  );
}

/** A server the host's deployment adds to every turn (servers.rs
 * `deployment`): shown, never changed here. `tools` is null when it did
 * not answer. */
type HostServer = { name: string; tools: number | null };

/** The servers this host adds, one row each with its tool count. They
 * live in the host's own setup, so they have no switch, edit or remove. */
function HostServers({ note }: { note: string }) {
  const [all, setAll] = useState<HostServer[]>([]);
  useEffect(() => {
    let live = true;
    void api("api/connections/host", { cache: "no-store" })
      .then((r) => (r.ok ? (r.json() as Promise<HostServer[]>) : []))
      .catch(() => [])
      .then((v) => live && setAll(v));
    return () => {
      live = false;
    };
  }, []);
  return all.map((s) => (
    <div key={s.name} className="flex min-h-11 w-full items-center gap-2 px-5 py-2 text-body">
      <span className="min-w-0 flex-1">
        <span className="block">{s.name}</span>
        <span className={`${note} block`}>Added by this host</span>
      </span>
      {s.tools !== null && <span className={`${note} shrink-0 tabular-nums`}>{s.tools === 1 ? "1 tool" : `${s.tools} tools`}</span>}
    </div>
  ));
}

/** Adds a connection, or edits one: its secrets are never sent back, so an
 * edit leaves them as they are unless new ones are typed. */
function ServerForm({ was, done, row, note }: { was?: Server; done: () => void; row: string; note: string }) {
  const [name, setName] = useState(was?.name ?? "");
  const [address, setAddress] = useState(was?.address ?? "");
  const [secrets, setSecrets] = useState("");
  const [withheld, setWithheld] = useState(was?.withheld.join(", ") ?? "");
  const [busy, setBusy] = useState(false);
  const [words, setWords] = useState<string>();
  const remote = /^https?:\/\//.test(address.trim());
  const save = async () => {
    setBusy(true);
    setWords(undefined);
    const r = await api("api/connections/servers", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        name: name.trim(),
        address: address.trim(),
        // Left out on an edit with nothing typed, the kept ones stay.
        ...(secrets.trim() || !was ? { secrets } : {}),
        withheld: withheld.split(",").map((t) => t.trim()).filter(Boolean),
      }),
    }).catch(() => null);
    setBusy(false);
    if (!r?.ok) return setWords(await said(r));
    done();
  };
  return (
    <form
      className="flex flex-col gap-1.5 px-5 py-2"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <label className={note}>
        Name
        <input value={name} onChange={(e) => setName(e.target.value)} readOnly={!!was} placeholder="mail" autoComplete="off" className={`${field} mt-0.5`} />
      </label>
      <label className={note}>
        Address
        <input value={address} onChange={(e) => setAddress(e.target.value)} placeholder="https://example.com/mcp  or  npx some-mcp-server" autoComplete="off" spellCheck={false} className={`${field} mt-0.5 font-mono`} />
      </label>
      <label className={note}>
        {remote ? "Headers, one per line" : "Environment, one per line"} (optional)
        <textarea
          value={secrets}
          onChange={(e) => setSecrets(e.target.value)}
          rows={2}
          placeholder={was?.secrets.length ? `Kept: ${was.secrets.join(", ")}. Type to replace.` : remote ? "Authorization: Bearer …" : "API_KEY=…"}
          autoComplete="off"
          spellCheck={false}
          className={`${field} mt-0.5 font-mono`}
        />
      </label>
      <label className={note}>
        Withheld tools (optional)
        <input value={withheld} onChange={(e) => setWithheld(e.target.value)} placeholder="send_email, delete_file" autoComplete="off" spellCheck={false} className={`${field} mt-0.5 font-mono`} />
      </label>
      <div className="flex gap-1.5">
        <button disabled={busy || !name.trim() || !address.trim()} className={`${row} flex-1 justify-center rounded-lg`}>{was ? "Save" : "Add"}</button>
        <button type="button" onClick={done} className={`${row} flex-1 justify-center rounded-lg`}>Cancel</button>
      </div>
      {words && <p className={note}>{words}</p>}
    </form>
  );
}

/** xAI, for read aloud, calls, X search and pictures: a device-code sign-in
 * approved on any device, so nothing comes back to paste, or an API key for
 * anyone without a plan that covers the sign-in. */
export function Xai({ now, load, changed, row, note, read }: { now: Connections | null; load: () => Promise<void>; changed: () => void; row: string; note: string; read: boolean }) {
  const [keying, setKeying] = useState(false);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [said, setSaid] = useState<string>();
  const signing = now?.xai_signing;
  const connected = !!now?.xai;
  // The host waits on the approval, so the page only watches for its end.
  useEffect(() => {
    if (!signing) return;
    const t = setInterval(() => void load(), 3000);
    return () => clearInterval(t);
  }, [signing, load]);
  const was = useRef(connected);
  useEffect(() => {
    if (was.current !== connected) changed();
    was.current = connected;
  }, [connected, changed]);
  if (!now) return null;
  const start = async () => {
    setSaid(undefined);
    setBusy(true);
    const r = await api("api/connections/xai", { method: "POST" }).catch(() => null);
    setBusy(false);
    if (!r?.ok) return setSaid(r ? (await r.text()) || `That failed (${r.status}).` : "The host does not answer.");
    await load();
  };
  const useKey = async () => {
    setSaid(undefined);
    setBusy(true);
    const r = await api("api/connections/xai/key", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ key: key.trim() }),
    }).catch(() => null);
    setBusy(false);
    if (!r?.ok) return setSaid(r ? (await r.text()) || `That failed (${r.status}).` : "The host does not answer.");
    setKey("");
    setKeying(false);
    await load();
  };
  const drop = async () => {
    if (!(await confirmAction({ title: "Disconnect xAI?", body: "Read aloud, calls, X search and pictures stop.", confirm: "Disconnect", danger: true }))) return;
    const r = await api("api/connections/xai/drop", { method: "POST" });
    if (!r.ok) return setSaid(await r.text());
    await load();
  };
  const shown = said ?? now.xai_said;
  return (
    <>
      <div className="flex w-full items-center gap-2 px-5 py-2 text-body">
        <span className="min-w-0 flex-1">
          <span className="block">xAI</span>
          <span className={`${note} block truncate`}>{now.xai ?? "Read aloud, calls, X search and pictures"}</span>
        </span>
        {!read &&
          (now.xai ? (
            <button type="button" onClick={() => void drop()} className="shrink-0 rounded-full px-2.5 py-1 text-label text-mid hover:bg-hover hover:text-ink">
              Disconnect
            </button>
          ) : (
            !signing && (
              <button type="button" disabled={busy} onClick={() => void start()} className="shrink-0 rounded-full px-2.5 py-1 text-label text-mid hover:bg-hover hover:text-ink">
                Sign in with Grok
              </button>
            )
          ))}
      </div>
      {signing && !now.xai && (
        <div className="flex flex-col gap-1.5 px-5 py-2">
          <span className={note}>Approve on xAI's page if the code matches.</span>
          <span className="text-center font-mono text-title tracking-widest">{signing.code}</span>
          <a href={signing.url} target="_blank" rel="noreferrer" className={`${row} justify-center rounded-lg`}>
            Open xAI's page
          </a>
        </div>
      )}
      {!read && !now.xai && !signing && (
        <div className="flex flex-col gap-1.5 px-5 pb-2">
          <span className={note}>Needs SuperGrok or X Premium+, or an API key from console.x.ai.</span>
          {keying ? (
            <form
              className="flex flex-col gap-1.5"
              onSubmit={(e) => {
                e.preventDefault();
                void useKey();
              }}
            >
              <input
                value={key}
                onChange={(e) => setKey(e.target.value)}
                type="password"
                placeholder="xai-…"
                aria-label="xAI API key"
                autoComplete="off"
                className="w-full rounded-lg border border-frame bg-paper px-3 py-2 font-mono text-caption outline-none focus:border-soft"
              />
              <button disabled={busy || !key.trim()} className={`${row} justify-center rounded-lg`}>
                Use this key
              </button>
            </form>
          ) : (
            <button type="button" onClick={() => setKeying(true)} className={`${note} self-start underline underline-offset-2 hover:text-ink`}>
              Use an API key
            </button>
          )}
        </div>
      )}
      {shown && <p className={`${note} px-5 pb-2`}>{shown}</p>}
    </>
  );
}

type Choice = { id: string; title: string; connection: string };
export type Builtin = {
  id: string;
  title: string;
  about: string;
  on: boolean;
  /** What fills it: the connected account that covers it, none before one is. */
  provider: string | null;
  provider_title: string | null;
  connection: string | null;
  signed_in: boolean;
  /** The connected accounts that cover it: a pick shows only with two. */
  choices: Choice[];
  agenda: boolean;
};

/** The built-ins the host has, asked again on `useBuiltins().get()`: the
 * side panel shows Mail and Calendar by them. */
export function useBuiltins() {
  const [all, setAll] = useState<Builtin[] | null>(null);
  const get = useCallback(async () => {
    const r = await api("api/builtins", { cache: "no-store" }).catch(() => null);
    setAll(r?.ok ? await r.json() : null);
  }, []);
  useEffect(() => {
    void get();
    // Settings changes them in another part of the page.
    addEventListener("libri:builtins", get);
    return () => removeEventListener("libri:builtins", get);
  }, [get]);
  return { all, get };
}

/** The built-ins, each a switch and the connection that fills it: a
 * connection a provider covers it with fills it on its own, so there is
 * nothing to pick unless two do. */
export function Builtins({ load, note, read }: { load: () => Promise<void>; note: string; read: boolean }) {
  const { all, get } = useBuiltins();
  const [words, setWords] = useState<string>();
  const put = async (id: string, change: Partial<{ on: boolean; provider: string }>) => {
    setWords(undefined);
    const r = await api(`api/builtins/${encodeURIComponent(id)}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(change),
    }).catch(() => null);
    if (!r?.ok) setWords(await said(r));
    await Promise.all([get(), load()]);
    dispatchEvent(new Event("libri:builtins"));
  };
  if (!all) return null;
  return (
    <>
      {all.map((b) => (
        <div key={b.id} className="px-5 py-2">
          <div className="flex items-center justify-between gap-4 text-body">
            <span className="min-w-0">
              <span className="text-ink">{b.title}</span>
              <span className={`${note} block`}>
                {b.connection ? `From ${b.provider_title}${b.signed_in ? "" : " · not signed in"}` : "No connection yet: add one above"}
              </span>
            </span>
            <button type="button" role="switch" aria-checked={b.on} aria-label={`${b.title} on`} disabled={read} onClick={() => void put(b.id, { on: !b.on })} className={toggleButton}>
              <Toggle on={b.on} />
            </button>
          </div>
          {b.on && b.choices.length > 1 && (
            <div role="radiogroup" aria-label={`${b.title} from`} className="mt-1.5 flex flex-wrap gap-1">
              {b.choices.map((c) => (
                <button key={c.id} type="button" role="radio" aria-checked={c.id === b.provider} disabled={read} onClick={() => void put(b.id, { provider: c.id })} className={`${small} ${c.id === b.provider ? "bg-hover text-ink" : ""}`}>
                  {c.title}
                </button>
              ))}
            </div>
          )}
        </div>
      ))}
      {words && <p className={`${note} px-5 pb-2 text-ink`}>{words}</p>}
    </>
  );
}

type Module = { id: string; title: string; about: string; start: boolean };
export type Connector = { id: string; name: string; modules: string[]; app: string | null; made: number; used: number };
type Door = { url: string; modules: Module[]; connectors: Connector[] };

/** What to paste where: an app that signs in by OAuth takes the address
 * alone; one that takes a header gets the key as well, shown once. */
export function pasteText(url: string, key?: string) {
  if (!key)
    return [
      { app: "Claude, ChatGPT, Cursor and other connector apps", text: url, step: "Add it as a custom connector, then sign in to Libri." },
      { app: "Claude Code", text: `claude mcp add --transport http libri ${url}` },
      { app: "Codex", text: `codex mcp add libri --url ${url}` },
    ];
  const header = `Authorization: Bearer ${key}`;
  return [
    { app: "Claude Code", text: `claude mcp add --transport http libri ${url} --header "${header}"` },
    { app: "Apps that take a header", text: JSON.stringify({ mcpServers: { libri: { url, headers: { Authorization: `Bearer ${key}` } } } }) },
    { app: "Apps that start a command, on this host", text: JSON.stringify({ mcpServers: { libri: { command: "libri", args: ["mcp"], env: { LIBRI_CONNECTOR_KEY: key } } } }) },
  ];
}

/** A module's pill: ringed in ink when on, in the frame's lighter line when
 * off, so a row of them reads at a glance. */
const modulePill = (on: boolean) =>
  `shrink-0 rounded-full border-[1.25px] px-3 py-1 text-label disabled:opacity-50 ${on ? "border-ink text-ink" : "border-frame text-mid hover:text-ink"}`;

const since = (unix: number) => (unix ? new Date(unix * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }) : "never");

/** The apps let in at /mcp, each a connector with a login of its own and
 * the modules of tools it may use, revoked here; and how to add one. */
export function OtherApps({ row, note, read }: { row: string; note: string; read: boolean }) {
  const [door, setDoor] = useState<Door>();
  const [words, setWords] = useState<string>();
  const [adding, setAdding] = useState(false);
  const [made, setMade] = useState<{ name: string; key: string }>();
  const load = useCallback(async () => {
    const r = await api("api/connectors", { cache: "no-store" }).catch(() => null);
    if (r?.ok) setDoor(await r.json());
  }, []);
  useEffect(() => void load(), [load]);
  if (!door) return null;
  // The address another device reaches, where the page came from elsewhere.
  const url = remote() ? `${remote()!.url.replace(/\/+$/, "")}/mcp` : door.url;
  const copy = (t: string) => void navigator.clipboard?.writeText(t).then(() => setWords("Copied."));
  const post = async (path: string, body?: unknown) => {
    setWords(undefined);
    const r = await api(path, { method: "POST", headers: { "content-type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) }).catch(() => null);
    if (!r?.ok) {
      setWords(await said(r));
      return null;
    }
    return r.json();
  };
  const toggle = async (c: Connector, m: string) => {
    const modules = c.modules.includes(m) ? c.modules.filter((x) => x !== m) : [...c.modules, m];
    if (await post(`api/connectors/${c.id}`, { modules })) await load();
  };
  const revoke = async (c: Connector) => {
    if (!(await confirmAction({ title: `Revoke ${c.name}?`, body: `${c.app ?? "The app"} is signed out at once, and its key stops working.`, confirm: "Revoke", danger: true }))) return;
    if (await post(`api/connectors/${c.id}/revoke`)) await load();
  };
  return (
    <>
      <p className={`${note} px-5 pb-1`}>Apps that use Libri over MCP, each limited to the parts you allow.</p>
      {door.connectors.map((c) => (
        <div key={c.id} className="py-1.5">
          <div className="flex w-full items-center gap-2 px-5 py-1 text-body">
            <span className="min-w-0 flex-1">
              <span className="block">{c.name}</span>
              <span className={`${note} block truncate`}>{c.app ? `Signed in by ${c.app}` : "By key"} · used {since(c.used)}</span>
            </span>
            {!read && (
              <button type="button" onClick={() => void revoke(c)} className={small}>
                Revoke
              </button>
            )}
          </div>
          <div className="flex flex-wrap gap-1.5 px-5 py-1">
            {door.modules.map((m) => {
              const on = c.modules.includes(m.id);
              return (
                <button key={m.id} type="button" role="switch" aria-checked={on} aria-label={`${c.name}: ${m.title}`} title={m.about} disabled={read} onClick={() => void toggle(c, m.id)} className={modulePill(on)}>
                  {m.title}
                </button>
              );
            })}
          </div>
        </div>
      ))}
      {made && (
        <div className="flex flex-col gap-1.5 px-5 py-2">
          <span className="text-body">Key for {made.name}</span>
          <span className={note}>Shown once. Paste it where the app takes a header.</span>
          <PasteList items={pasteText(url, made.key)} note={note} copy={copy} />
          <button type="button" onClick={() => setMade(undefined)} className={`${row} justify-center rounded-lg`}>Done</button>
        </div>
      )}
      {adding ? (
        <AddConnector modules={door.modules} row={row} note={note} cancel={() => setAdding(false)} add={async (name, modules) => {
          const got = await post("api/connectors", { name, modules });
          if (!got) return;
          setAdding(false);
          setMade({ name: got.connector.name, key: got.key });
          await load();
        }} />
      ) : (
        !read && !made && (
          <button type="button" onClick={() => setAdding(true)} className={row}>
            Add by key
            <span className={note}>For apps that take a header</span>
          </button>
        )
      )}
      <div className="flex flex-col gap-1.5 px-5 py-2">
        <span className="text-ink">Connect an app</span>
        <PasteList items={pasteText(url)} note={note} copy={copy} />
      </div>
      {words && <p className={`${note} px-5 pb-2 text-ink`}>{words}</p>}
    </>
  );
}

function PasteList({ items, note, copy }: { items: { app: string; text: string; step?: string }[]; note: string; copy: (t: string) => void }) {
  return (
    <>
      {items.map((s) => (
        <div key={s.app} className="flex flex-col gap-1">
          <span className={note}>{s.app}</span>
          <button type="button" onClick={() => copy(s.text)} title="Copy" className="rounded-lg bg-block px-2.5 py-1.5 text-left font-mono text-caption break-all select-text">
            {s.text}
          </button>
          {s.step && <span className={note}>{s.step}</span>}
        </div>
      ))}
    </>
  );
}

function AddConnector({ modules, row, note, add, cancel }: { modules: Module[]; row: string; note: string; add: (name: string, modules: string[]) => Promise<void>; cancel: () => void }) {
  const [name, setName] = useState("");
  const [picked, setPicked] = useState(() => modules.filter((m) => m.start).map((m) => m.id));
  const [busy, setBusy] = useState(false);
  return (
    <form
      className="flex flex-col gap-1.5 px-5 py-2"
      onSubmit={(e) => {
        e.preventDefault();
        setBusy(true);
        void add(name.trim(), picked).finally(() => setBusy(false));
      }}
    >
      <label className={note}>
        Name
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Claude Code on the laptop" autoComplete="off" className={`${field} mt-0.5`} />
      </label>
      {modules.map((m) => (
        <label key={m.id} className="flex items-start gap-2.5 py-1 text-body">
          <input type="checkbox" checked={picked.includes(m.id)} onChange={() => setPicked((p) => (p.includes(m.id) ? p.filter((x) => x !== m.id) : [...p, m.id]))} className="mt-1 accent-ink" />
          <span className="min-w-0">
            {m.title}
            <span className={`${note} block`}>{m.about}</span>
          </span>
        </label>
      ))}
      <div className="flex gap-1.5">
        <button disabled={busy || !name.trim()} className={`${row} flex-1 justify-center rounded-lg`}>Add</button>
        <button type="button" onClick={cancel} className={`${row} flex-1 justify-center rounded-lg`}>Cancel</button>
      </div>
    </form>
  );
}
