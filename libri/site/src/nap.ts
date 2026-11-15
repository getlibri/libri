// A worker's nap, after T3 Code's snooze: the same five presets, the ones
// already past left out, and two that land in the same minute shown once.
// Libri keeps them fixed, as it has no setting for them.

/** One nap the menu offers: its words and when it wakes, in seconds. */
export type NapPreset = { label: string; until: number };

const at = (day: Date, hour: number) => {
  const d = new Date(day);
  d.setHours(hour, 0, 0, 0);
  return d;
};

const clock = (d: Date) =>
  d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });

/** The naps to offer at `now`, soonest first. */
export function napPresets(now: Date): NapPreset[] {
  const tomorrow = new Date(now);
  tomorrow.setDate(now.getDate() + 1);
  // Next Monday, a week on when today is one.
  const monday = new Date(now);
  monday.setDate(now.getDate() + (((8 - now.getDay()) % 7) || 7));
  const evening = at(now, 18);
  const all: [string, Date][] = [
    ["1 hour", new Date(now.getTime() + 3600_000)],
    ["3 hours", new Date(now.getTime() + 3 * 3600_000)],
    [`This evening, ${clock(evening)}`, evening],
    [`Tomorrow, ${clock(at(tomorrow, 9))}`, at(tomorrow, 9)],
    [`Monday, ${clock(at(monday, 9))}`, at(monday, 9)],
  ];
  const seen = new Set<number>();
  return all
    .filter(([, d]) => d.getTime() > now.getTime())
    .sort(([, a], [, b]) => a.getTime() - b.getTime())
    .filter(([, d]) => {
      const minute = Math.floor(d.getTime() / 60_000);
      if (seen.has(minute)) return false;
      seen.add(minute);
      return true;
    })
    .map(([label, d]) => ({ label, until: Math.floor(d.getTime() / 1000) }));
}

/** When a nap wakes, as a card's few words: the hour, and the day unless
 * it is today, as the host's `wake_words` says it. */
export function wakeWords(until: number, now = new Date()): string {
  const wake = new Date(until * 1000);
  const days = Math.round(
    (at(wake, 0).getTime() - at(now, 0).getTime()) / 86400_000,
  );
  if (days <= 0) return clock(wake);
  if (days === 1) return `tomorrow ${clock(wake)}`;
  if (days < 7)
    return `${wake.toLocaleDateString(undefined, { weekday: "short" })} ${clock(wake)}`;
  return `${wake.toLocaleDateString(undefined, { month: "short", day: "numeric" })} ${clock(wake)}`;
}

/** A `datetime-local` value for `d`, in local time, to the minute. */
export function localInput(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}
