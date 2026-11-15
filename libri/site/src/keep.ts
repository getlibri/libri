// What the browser holds for the person between visits, for the host it was
// written for: words in localStorage, files in IndexedDB, since a photo won't
// fit in the one and the other takes a File whole.
import { remote } from "./api";

/** `key` for the host picked now, so a message waiting for one never goes to another. */
export const forHost = (key: string) => {
  const r = remote();
  return r ? `${key}:${r.url}` : key;
};

const files = () =>
  new Promise<IDBDatabase>((done, failed) => {
    const open = indexedDB.open("libri-files", 1);
    open.onupgradeneeded = () => open.result.createObjectStore("files");
    open.onsuccess = () => done(open.result);
    open.onerror = () => failed(open.error);
  });

function store<T>(mode: IDBTransactionMode, act: (s: IDBObjectStore) => IDBRequest<T>) {
  return files().then(
    (db) =>
      new Promise<T>((done, failed) => {
        const r = act(db.transaction("files", mode).objectStore("files"));
        r.onsuccess = () => done(r.result);
        r.onerror = () => failed(r.error);
      }).finally(() => db.close()),
  );
}

/** Private windows and full disks refuse; the words are kept either way. */
export const keepFiles = (key: string, fs: File[]) =>
  (fs.length ? store("readwrite", (s) => s.put(fs, key)) : store("readwrite", (s) => s.delete(key))).then(
    () => undefined,
    () => undefined,
  );

export const keptFiles = (key: string): Promise<File[]> =>
  store<File[] | undefined>("readonly", (s) => s.get(key)).then(
    (fs) => fs ?? [],
    () => [],
  );

/** Words for the chat's composer from elsewhere on the page (a Library
 * page's message): the chat puts them in its box, for the person to send
 * or not. */
const COMPOSE = "libri:compose";
export const compose = (text: string) => dispatchEvent(new CustomEvent(COMPOSE, { detail: text }));
export function onCompose(take: (text: string) => void) {
  const heard = (e: Event) => take((e as CustomEvent<string>).detail);
  addEventListener(COMPOSE, heard);
  return () => removeEventListener(COMPOSE, heard);
}

/** A composer's words as last typed. */
export const draft = (key: string) => localStorage.getItem(key) ?? "";
export const keepDraft = (key: string, text: string) => {
  // Full storage only costs the draft.
  try {
    if (text) localStorage.setItem(key, text);
    else localStorage.removeItem(key);
  } catch {}
};
