// A worker's model by the name the composer's model picker shows ("Opus
// 5.5"), whatever form its pick was stored in: the full id, the id without
// its maker's prefix or date, or a harness's alias ("opus", which Claude Code
// takes for its newest Opus). The offers come from api/models, which App
// asks for; until they arrive, or for an alias nothing on offer matches, the
// model shows as stored.
import { useSyncExternalStore } from "react";

export type NamedOffer = { harness: string; model: string; name: string; default?: boolean };

let OFFERS: NamedOffer[] = [];
const watchers = new Set<() => void>();

/** Called with each api/models answer. */
export function publishOffers(offers: NamedOffer[]) {
  OFFERS = offers;
  for (const w of watchers) w();
}

/** The offers on hand, kept current. */
export function useOffers(): NamedOffer[] {
  return useSyncExternalStore(
    (w) => (watchers.add(w), () => watchers.delete(w)),
    () => OFFERS,
  );
}

/** An id as harnesses vary it: no provider path, maker prefix or date. */
const bare = (id: string) =>
  id
    .split("/")
    .pop()!
    .toLowerCase()
    .replace(/^claude-/, "")
    .replace(/[-@]\d{8}$/, "");

/** Its version numbers, for picking the newest of a family. */
const version = (id: string) => (bare(id).match(/\d+/g) ?? []).map(Number);
const newer = (a: number[], b: number[]) => {
  for (let i = 0; i < Math.max(a.length, b.length); i++) if ((a[i] ?? 0) !== (b[i] ?? 0)) return (b[i] ?? 0) - (a[i] ?? 0);
  return 0;
};

/** The display name of `model` on `harness`: an exact offer, then one equal
 * once both are bare, then for an alias (a word with no version, as "opus")
 * the newest offer of that family on the harness; an empty model is the
 * harness's default. Else the model as stored, bare. */
export function modelName(harness: string, model: string, offers: NamedOffer[]): string {
  if (!model) return offers.find((o) => o.harness === harness && o.default)?.name ?? "";
  return offerOf(harness, model, offers)?.name ?? bare(model);
}

/** The offer a stored model stands for, as modelName finds it; none for an
 * empty model or an alias nothing on offer matches. */
export function offerOf<O extends NamedOffer>(harness: string, model: string, offers: O[]): O | undefined {
  if (!model) return undefined;
  const mine = offers.filter((o) => o.harness === harness);
  const exact = mine.find((o) => o.model === model) ?? mine.find((o) => bare(o.model) === bare(model));
  if (exact) return exact;
  const alias = bare(model);
  if (!/^[a-z]+$/.test(alias)) return undefined;
  return mine.filter((o) => bare(o.model).startsWith(`${alias}-`)).sort((a, b) => newer(version(a.model), version(b.model)))[0];
}
