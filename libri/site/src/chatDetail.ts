import { useSyncExternalStore } from "react";

export type ChatDetail = "simple" | "standard" | "full";
const KEY = "chat-detail";
const CHANGED = "libri:chat-detail";
export function chatDetail(): ChatDetail {
  try {
    const value = localStorage.getItem(KEY);
    return value === "simple" || value === "full" ? value : "standard";
  } catch { return "standard"; }
}
export function setChatDetail(value: ChatDetail) {
  try { localStorage.setItem(KEY, value); } catch { /* Private storage may be unavailable. */ }
  dispatchEvent(new Event(CHANGED));
}
function subscribe(changed: () => void) {
  addEventListener(CHANGED, changed);
  addEventListener("storage", changed);
  return () => { removeEventListener(CHANGED, changed); removeEventListener("storage", changed); };
}
/** A display preference shared by every harness and thread in this browser. */
export function useChatDetail() {
  return useSyncExternalStore(subscribe, chatDetail, () => "standard" as ChatDetail);
}
