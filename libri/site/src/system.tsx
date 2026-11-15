import { useEffect, useState } from "react";
import { api } from "./api";

/** The host as /api/system reports it (libri/src/system.rs). */
type Sample = {
  t: number;
  cpu: number;
  load1: number;
  mem_total: number;
  mem_available: number;
  swap_total: number;
  swap_used: number;
  mem_some: number;
  mem_full: number;
  cpu_some: number;
  io_full: number;
  net_rx: number;
  net_tx: number;
  disk_read: number;
  disk_write: number;
};
type Report = {
  now: Sample;
  cores: number;
  disks: { mount: string; total: number; available: number }[];
  libri: Record<string, number>;
  workers: { worker: string; cpu: number; rss: number; procs: number }[];
  top: { pid: number; name: string; cpu: number; rss: number; worker: string | null }[];
  services: [string, string][];
  alerts: { key: string; text: string; since: number }[];
  fine: Sample[];
};

const EVERY = 3000;

const gb = (b: number) => `${(b / 2 ** 30).toFixed(1)} GB`;
const rate = (b: number) => (b >= 2 ** 20 ? `${(b / 2 ** 20).toFixed(1)} MB/s` : `${Math.round(b / 1024)} KB/s`);

/** A small line of the last hour, scaled to its own peak (or to `max`). */
function Spark({ values, max }: { values: number[]; max?: number }) {
  if (values.length < 2) return <svg className="h-6 w-full" />;
  const top = Math.max(max ?? 0, ...values, 1e-9);
  const points = values.map((v, i) => `${(i / (values.length - 1)) * 100},${24 - (v / top) * 22}`).join(" ");
  return (
    <svg viewBox="0 0 100 24" preserveAspectRatio="none" className="h-6 w-full text-mid" aria-hidden>
      <polyline points={points} fill="none" stroke="currentColor" strokeWidth="1.2" vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

function Stat({ label, value, note, values, max }: { label: string; value: string; note?: string; values: number[]; max?: number }) {
  return (
    <div className="flex flex-col gap-1 rounded-2xl bg-hover/60 px-4 py-3">
      <span className="text-caption text-mid">{label}</span>
      <span className="text-control text-ink tabular-nums">{value}</span>
      {note && <span className="text-caption text-mid tabular-nums">{note}</span>}
      <Spark values={values} max={max} />
    </div>
  );
}

/** The System tile: the host's CPU, memory, swap, disks and network over the
 * last hour, Libri's own share, each worker's, the busiest processes, the
 * services the house leans on, and any alert raised. */
export function SystemTile({ phone }: { phone: boolean }) {
  const [r, setR] = useState<Report>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    let live = true;
    let timer: number | undefined;
    const read = async () => {
      // A hidden page asks nothing; it reads again once shown.
      if (document.visibilityState === "visible") {
        try {
          const res = await api("api/system");
          if (!res.ok) throw new Error(`${res.status}`);
          const body = (await res.json()) as Report;
          if (live) {
            setR(body);
            setError(undefined);
          }
        } catch (e) {
          if (live) setError(`The host didn't answer (${e instanceof Error ? e.message : e}).`);
        }
      }
      if (live) timer = window.setTimeout(read, EVERY);
    };
    read();
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, []);

  const n = r?.now;
  const fine = r?.fine ?? [];
  return (
    <div className="flex h-full min-h-0 flex-col overflow-y-auto bg-paper" data-surface="system">
      <div className={`mx-auto flex w-full max-w-3xl flex-col gap-4 px-4 pb-10 ${phone ? "pt-[calc(var(--tile-top,0px)+8px)]" : "pt-6"}`}>
        {error && <p className="text-control text-mid">{error}</p>}
        {!r && !error && <p className="text-control text-mid">Reading the host…</p>}
        {r && n && (
          <>
            {r.alerts.length > 0 && (
              <ul className="flex flex-col gap-2" aria-label="Alerts">
                {r.alerts.map((a) => (
                  <li key={a.key} className="rounded-2xl bg-accent/10 px-4 py-3 text-control text-ink">
                    {a.text}
                  </li>
                ))}
              </ul>
            )}
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
              <Stat label="CPU" value={`${n.cpu.toFixed(0)}%`} note={`${r.cores} cores, load ${n.load1.toFixed(1)}`} values={fine.map((s) => s.cpu)} max={100} />
              <Stat
                label="Memory"
                value={`${gb(n.mem_total - n.mem_available)} of ${gb(n.mem_total)}`}
                note={`stalled ${n.mem_some.toFixed(0)}% / ${n.mem_full.toFixed(0)}%`}
                values={fine.map((s) => s.mem_total - s.mem_available)}
                max={n.mem_total}
              />
              <Stat label="Swap" value={`${gb(n.swap_used)} of ${gb(n.swap_total)}`} values={fine.map((s) => s.swap_used)} max={n.swap_total} />
              <Stat label="Network" value={`↓ ${rate(n.net_rx)}`} note={`↑ ${rate(n.net_tx)}`} values={fine.map((s) => s.net_rx + s.net_tx)} />
              <Stat label="Disk I/O" value={`R ${rate(n.disk_read)}`} note={`W ${rate(n.disk_write)}`} values={fine.map((s) => s.disk_read + s.disk_write)} />
              <Stat label="I/O stalled" value={`${n.io_full.toFixed(0)}%`} note={`CPU waits ${n.cpu_some.toFixed(0)}%`} values={fine.map((s) => s.io_full)} max={100} />
            </div>

            <section className="flex flex-col gap-1">
              <h3 className="text-caption text-mid">Disks</h3>
              {r.disks.map((d) => (
                <div key={d.mount} className="flex items-center gap-3 text-control">
                  <span className="w-28 shrink-0 truncate text-ink">{d.mount}</span>
                  <span className="h-1.5 flex-1 overflow-hidden rounded-full bg-hover">
                    <span className="block h-full rounded-full bg-mid" style={{ width: `${((d.total - d.available) / Math.max(d.total, 1)) * 100}%` }} />
                  </span>
                  <span className="w-28 shrink-0 text-right text-mid tabular-nums">{gb(d.available)} free</span>
                </div>
              ))}
            </section>

            <section className="flex flex-col gap-1">
              <h3 className="text-caption text-mid">Libri</h3>
              <p className="text-control text-ink tabular-nums">
                {["server", "harness", "browser"]
                  .filter((k) => r.libri[k] !== undefined)
                  .map((k) => `${{ server: "Libri itself", harness: "workers and harnesses", browser: "browser" }[k]} ${gb(r.libri[k])}`)
                  .join(" · ") || "Libri's cgroups aren't separated on this host."}
              </p>
            </section>

            {r.workers.length > 0 && (
              <section className="flex flex-col gap-1">
                <h3 className="text-caption text-mid">Workers</h3>
                {r.workers.map((w) => (
                  <div key={w.worker} className="flex items-center justify-between gap-3 text-control tabular-nums">
                    <span className="truncate text-ink">{w.worker.replace(/-\d{4}-\d{2}-\d{2}-\d{4}$/, "")}</span>
                    <span className="shrink-0 text-mid">
                      {w.cpu.toFixed(1)} cores · {gb(w.rss)} · {w.procs} {w.procs === 1 ? "process" : "processes"}
                    </span>
                  </div>
                ))}
              </section>
            )}

            <section className="flex flex-col gap-1">
              <h3 className="text-caption text-mid">Busiest processes</h3>
              {r.top.map((p) => (
                <div key={p.pid} className="flex items-center justify-between gap-3 text-control tabular-nums">
                  <span className="truncate text-ink">
                    {p.name}
                    <span className="text-mid"> {p.pid}{p.worker ? ` · ${p.worker.replace(/-\d{4}-\d{2}-\d{2}-\d{4}$/, "")}` : ""}</span>
                  </span>
                  <span className="shrink-0 text-mid">
                    {p.cpu.toFixed(1)} cores · {gb(p.rss)}
                  </span>
                </div>
              ))}
            </section>

            <section className="flex flex-wrap gap-2" aria-label="Services">
              {r.services.map(([unit, state]) => (
                <span key={unit} className={`rounded-full px-3 py-1 text-caption ${state === "active" ? "bg-hover text-ink" : "bg-accent/15 text-ink"}`}>
                  {unit} · {state}
                </span>
              ))}
            </section>
          </>
        )}
      </div>
    </div>
  );
}
