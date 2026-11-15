// A link to a Library page, `#page=<name>` (library.rs link), opens in the
// Library of the Libri the reader has open: on this page's own origin, or
// at app.libri.is, the address the links are written with, whichever host
// is open now.

const HOSTED = "app.libri.is";

/** The page a link names, if it is a Libri page link. */
export function pageOf(href: string): string | undefined {
  let url: URL;
  try {
    url = new URL(href, location.href);
  } catch {
    return undefined;
  }
  if (url.origin !== location.origin && url.hostname !== HOSTED) return undefined;
  return new URLSearchParams(url.hash.slice(1)).get("page") || undefined;
}

/** Opens a page in the Library, by way of App's hashchange follower. */
export function followPage(page: string) {
  location.hash = `page=${encodeURIComponent(page)}`;
}

/** The page a show_page result published, from the link it gives. */
export function publishedPage(result: string): string | undefined {
  const m = /#page=([^\s)\]"'<>]+)/.exec(result);
  return m ? decodeURIComponent(m[1]) : undefined;
}
