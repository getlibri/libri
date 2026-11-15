// What's new: after the host updates, one note with every change since the
// version this device last saw on it (GET /api/news). A device that has
// seen none is new to the host and starts at the version it finds. Kept
// per host, by its stable id, so switching hosts tells each one's news.
import { useEffect, useState } from "react";
import { api } from "./api";

type Release = { version: string; date: string; changes: string[] };

const seenKey = (host: string) => `libri-seen-version:${host}`;

export function WhatsNew() {
  const [news, setNews] = useState<{ host: string; version: string; releases: Release[] } | null>(null);

  useEffect(() => {
    (async () => {
      const host = await api("api/host", { cache: "no-store" })
        .then((r) => (r.ok ? r.json() : null))
        .catch(() => null);
      if (!host?.id || !host.version) return;
      const seen = localStorage.getItem(seenKey(host.id));
      if (seen === host.version) return;
      if (!seen) return localStorage.setItem(seenKey(host.id), host.version);
      const r = await api(`api/news?since=${encodeURIComponent(seen)}`).catch(() => null);
      const body = r?.ok ? await r.json() : null;
      if (!body?.releases?.length) return localStorage.setItem(seenKey(host.id), host.version);
      setNews({ host: host.id, version: body.version, releases: body.releases });
    })();
  }, []);

  if (!news) return null;
  const close = () => {
    localStorage.setItem(seenKey(news.host), news.version);
    setNews(null);
  };
  const from = news.releases[news.releases.length - 1].version;
  return (
    <aside
      role="status"
      aria-label="What's new"
      className="fixed right-3 bottom-24 left-3 z-50 mx-auto max-h-[60vh] max-w-md overflow-y-auto overscroll-contain rounded-2xl bg-lift p-4 text-control leading-relaxed text-mid shadow-dock"
    >
      <div className="mb-2 flex items-baseline justify-between gap-3">
        <h2 className="font-medium text-ink">
          What's new in {news.version === from ? news.version : `${from} to ${news.version}`}
        </h2>
        <button type="button" onClick={close} className="rounded-full px-2 py-0.5 text-soft hover:bg-hover hover:text-ink">
          Close
        </button>
      </div>
      <ul className="list-disc space-y-1 pl-5">
        {news.releases.flatMap((r) => r.changes.map((c, i) => <li key={`${r.version}-${i}`}>{c}</li>))}
      </ul>
    </aside>
  );
}
