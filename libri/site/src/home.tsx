import { useEffect, useState } from "react";
import { api } from "./api";
import { useBuiltins, type Builtin } from "./connections";
import { ago, NEED, type Thread, type Work } from "./work";
import { BAR_FADE } from "./library";

/** One event of a built-in's agenda (builtins.rs events): `start` in RFC
 * 3339, or a date alone for a whole day. */
type AgendaEvent = { id?: string; title: string; start: string; end?: string; all_day: boolean; location?: string };

/** Pills past this many fold into one that opens Tasks. */
const SHOWN = 8;

/** The Home place, left of Chat on a phone (Android's Home.kt HomePlace):
 * the day at a glance, as Oura's Today reads (Ida's dashboard study,
 * 2026-11-06). A sentence says the day first; under it what needs the
 * person, the calendar's events, the workers at work and those done today
 * as pills, and the release. Colour is for what needs the person alone;
 * everything else is ink on paper. Every card opens what it is about. */
export function Today({ work, open, calendar, tasks }: { work: Work; open: (t: Thread) => void; calendar: (id: string) => void; tasks: () => void }) {
  const { all: builtins } = useBuiltins();
  const day = builtins?.find((b) => b.agenda && b.on && b.connection);
  const events = useToday(day);
  const version = useVersion();
  const now = new Date();
  const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime() / 1000;
  const waiting = work.threads.filter((t) => t.state === "waiting");
  const working = work.threads.filter((t) => t.state === "working");
  const done = work.threads
    .filter((t) => (t.state === "sleeping" || t.state === "archived") && t.at >= midnight)
    .sort((a, b) => b.at - a.at);
  return (
    <div data-place="home" className={`h-full overflow-y-auto px-[18px] pt-7 pb-[calc(var(--bar-room)+24px)] ${BAR_FADE}`}>
      <div className="mb-2 pl-0.5">
        <h1 className="font-serif text-heading leading-tight text-ink">{now.toLocaleDateString(undefined, { weekday: "long" })}</h1>
        <p className="text-control text-mid">{now.toLocaleDateString(undefined, { day: "numeric", month: "long" })}</p>
        <p className="mt-3.5 text-prose leading-normal text-ink">{sentence(waiting.length, working.length, day ? (events?.length ?? 0) : undefined)}</p>
      </div>
      {waiting.length > 0 && (
        <Section title="Needs you" first>
          {waiting.map((t) => (
            <button key={t.id} data-need type="button" onClick={() => open(t)} className="block w-full rounded-2xl border border-hair bg-lift px-4 py-3 text-left shadow-lift">
              <span className="flex items-center gap-2">
                <span aria-hidden className="size-2 shrink-0 rounded-full bg-accent" />
                <span className="truncate text-label font-semibold text-ink">{t.name}</span>
                <span className="min-w-0 flex-1 truncate text-caption font-medium text-accent">{(t.need && NEED[t.need]) ?? "Waiting"}</span>
                <span className="shrink-0 text-caption text-mid">{ago(t.at)}</span>
              </span>
              <span className="mt-1 block truncate text-control font-medium text-ink">{t.title ?? firstLine(t.brief)}</span>
              {t.gist.trim() && <span className="mt-0.5 line-clamp-2 block text-label text-mid">{t.gist}</span>}
            </button>
          ))}
        </Section>
      )}
      {day && (
        <Section title={day.title}>
          <button type="button" onClick={() => calendar(day.id)} className="flex w-full flex-col gap-2 rounded-2xl border border-hair bg-lift px-4 py-3 text-left shadow-lift">
            {events === undefined ? (
              <span className="text-control text-mid">Reading the calendar</span>
            ) : events.length === 0 ? (
              <span className="text-control text-mid">Nothing today.</span>
            ) : (
              events.map((e, i) => (
                <span key={e.id ?? i} className="flex items-start">
                  <span className="w-16 shrink-0 text-label leading-relaxed text-mid">{clock(e)}</span>
                  <span className="min-w-0 flex-1">
                    <span className="line-clamp-2 block text-control font-medium text-ink">{e.title.trim() || "Untitled"}</span>
                    {e.location?.trim() && <span className="block truncate text-caption text-mid">{e.location}</span>}
                  </span>
                </span>
              ))
            )}
          </button>
        </Section>
      )}
      {working.length > 0 && (
        <Section title="At work">
          <div className="flex flex-wrap gap-2">
            {working.map((t) => (
              <Pill key={t.id} name={t.name} words={t.title ?? firstLine(t.brief)} dot onClick={() => open(t)} />
            ))}
          </div>
        </Section>
      )}
      {done.length > 0 && (
        <Section title="Done today">
          <div className="flex flex-wrap gap-2">
            {done.slice(0, SHOWN).map((t) => (
              <Pill key={t.id} name={t.name} words={ago(t.at)} onClick={() => open(t)} />
            ))}
            {done.length > SHOWN && <Pill name={`${done.length - SHOWN} more`} onClick={tasks} />}
          </div>
        </Section>
      )}
      {version && (
        <p className="mt-[22px] pl-0.5 text-label text-mid">
          Libri {version.version} on {version.name}, up {uptime(version.uptime)}
        </p>
      )}
    </div>
  );
}

function Section({ title, first, children }: { title: string; first?: boolean; children: React.ReactNode }) {
  return (
    <section className={first ? "mt-4" : "mt-6"}>
      <h2 className="mb-2.5 pl-0.5 text-caption font-semibold tracking-wide text-mid uppercase">{title}</h2>
      <div className="flex flex-col gap-2.5">{children}</div>
    </section>
  );
}

/** A worker as a pill: its name and a few words. One at work wears an ink
 * dot, not green: Home keeps colour for what needs the person. */
function Pill({ name, words, dot, onClick }: { name: string; words?: string; dot?: boolean; onClick: () => void }) {
  return (
    <button type="button" onClick={onClick} className="flex max-w-full items-center rounded-full border border-hair bg-lift px-3 py-2 text-label">
      {dot && <span aria-label="Working" className="mr-[7px] size-1.5 shrink-0 rounded-full bg-ink" />}
      <span className="shrink-0 font-semibold text-ink">{name}</span>
      {words && <span className="ml-1.5 max-w-[180px] truncate text-mid">{words}</span>}
    </button>
  );
}

/** Today's events from the built-in, asked on arrival and every five
 * minutes: undefined until the first answer; a failure reads as none. */
function useToday(day: Builtin | undefined): AgendaEvent[] | undefined {
  const [events, setEvents] = useState<AgendaEvent[]>();
  const id = day?.id;
  useEffect(() => {
    if (!id) return;
    const ask = async () => {
      const from = new Date();
      from.setHours(0, 0, 0, 0);
      const to = new Date(from);
      to.setDate(to.getDate() + 1);
      const r = await api(`api/builtins/${encodeURIComponent(id)}/call`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ tool: "agenda", arguments: { from: rfc3339(from), to: rfc3339(to) } }),
      }).catch(() => null);
      const got = r?.ok ? ((await r.json()) as { isError?: boolean; structuredContent?: { events?: AgendaEvent[] } }) : null;
      setEvents(got && !got.isError ? (got.structuredContent?.events ?? []).sort((a, b) => a.start.localeCompare(b.start)) : []);
    };
    void ask();
    const t = window.setInterval(() => document.visibilityState === "visible" && void ask(), 300_000);
    return () => clearInterval(t);
  }, [id]);
  return id ? events : undefined;
}

function useVersion() {
  const [host, setHost] = useState<{ version: string; name: string; uptime: number }>();
  useEffect(() => {
    void api("api/host", { cache: "no-store" })
      .then((r) => (r.ok ? r.json() : undefined))
      .then(setHost)
      .catch(() => undefined);
  }, []);
  return host;
}

/** A local time with its offset, as the host's agenda takes it. */
function rfc3339(d: Date) {
  const pad = (n: number) => String(Math.abs(n)).padStart(2, "0");
  const off = -d.getTimezoneOffset();
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T00:00:00${off >= 0 ? "+" : "-"}${pad(Math.trunc(off / 60))}:${pad(off % 60)}`;
}

function clock(e: AgendaEvent) {
  if (e.all_day || e.start.length === 10) return "All day";
  const d = new Date(e.start);
  return isNaN(d.getTime()) ? e.start.slice(11, 16) : d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

function firstLine(text: string) {
  return text.split("\n").find((l) => l.trim())?.trim() ?? "";
}

function uptime(s: number) {
  return s < 3600 ? `${Math.floor(s / 60)}m` : s < 86400 ? `${Math.floor(s / 3600)}h` : `${Math.floor(s / 86400)}d`;
}

const COUNT = ["no", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];
const count = (n: number) => COUNT[n] ?? String(n);

/** The day in one sentence, as Oura's Today opens: what needs the person
 * first, then what runs, then the calendar when it is connected (Home.kt
 * `sentence`, word for word). */
export function sentence(waiting: number, working: number, events?: number) {
  const parts: string[] = [];
  if (waiting > 0) parts.push(waiting === 1 ? "one worker needs you" : `${count(waiting)} workers need you`);
  // After the workers that wait, "worker" goes without saying.
  const who = waiting > 0 ? "" : working === 1 ? " worker" : " workers";
  if (working > 0) parts.push(working === 1 ? `one${who} is at work` : `${count(working)}${who} are at work`);
  if (events !== undefined)
    parts.push(events === 0 ? "the calendar is clear" : events === 1 ? "one thing is on the calendar" : `${count(events)} things are on the calendar`);
  const joined = parts.length < 2 ? (parts[0] ?? "") : parts.length === 2 ? `${parts[0]}, and ${parts[1]}` : `${parts[0]}, ${parts[1]}, and ${parts[2]}`;
  const said = joined.charAt(0).toUpperCase() + joined.slice(1);
  if (!said) return "Nothing needs you.";
  return waiting === 0 ? `Nothing needs you. ${said}.` : `${said}.`;
}
