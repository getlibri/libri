// MCP Apps: which tool calls carry a card. The host knows, from its own
// sessions to the servers, and names each card's tool the way every
// harness names its calls; the page looks a call's name up. Asked once per
// page: a server added later shows its cards after a reload.

import { useEffect, useState } from "react";
import { api } from "./api";

/** A tool with a card: where it lives, and the tool as its server lists it. */
export type CardTool = {
  server: string;
  tool: string;
  uri: string;
  definition: Record<string, unknown>;
  /** The tool's card is the point of a call (apps.rs `Card::opens`): it
   * opens by itself under each call. Any other call's waits behind its row. */
  opens?: boolean;
};

let asked: Promise<Record<string, CardTool>> | null = null;

function cards() {
  // An older host has no /api/apps: no call has a card there.
  asked ??= api("api/apps")
    .then((r) => (r.ok ? r.json() : { cards: {} }))
    .then((j: { cards?: Record<string, CardTool> }) => j.cards ?? {})
    .catch(() => ({}));
  return asked;
}

/** The card a call of `name` draws, once the host has said; null till then
 * and for a call that has none. */
export function useCardTool(name: string): CardTool | null {
  const [found, setFound] = useState<CardTool | null>(null);
  useEffect(() => {
    let on = true;
    void cards().then((c) => on && setFound(c[name] ?? null));
    return () => {
      on = false;
    };
  }, [name]);
  return found;
}

/** The tool call a card is drawn for: the call itself; for `show_card`, the
 * tool it names, with `shown`; or, for OpenCode's
 * code mode (an `execute` call whose script calls MCP tools as
 * `tools["server"].tool({...})`), the one card tool its script calls. A
 * script calling several draws none: which result is whose can't be told.
 * Its input is the script's argument where that reads as JSON, else none. */
export function cardCall(name: string, args: unknown): { name: string; input: Record<string, unknown>; shown?: boolean } {
  const own = { name, input: (typeof args === "object" && args ? args : {}) as Record<string, unknown> };
  // The model asked to show a tool's card (tools.rs show_card): that tool's,
  // drawn from a fresh call with the arguments it named.
  if (/^(mcp__libri__|libri_)show_card$/.test(name)) {
    const a = own.input as { tool?: unknown; arguments?: unknown };
    if (typeof a.tool !== "string") return own;
    return { name: a.tool, input: (typeof a.arguments === "object" && a.arguments ? a.arguments : {}) as Record<string, unknown>, shown: true };
  }
  const code = name === "execute" ? (args as { code?: unknown } | undefined)?.code : undefined;
  if (typeof code !== "string") return own;
  const ref = String.raw`(?:\[\s*["']([^"']+)["']\s*\]|\.\s*([A-Za-z_$][\w$]*))`;
  const calls = [...code.matchAll(new RegExp(String.raw`\btools\s*${ref}\s*${ref}\s*\(`, "g"))];
  if (calls.length !== 1) return own;
  const [m] = calls;
  const server = m[1] ?? m[2];
  const tool = m[3] ?? m[4];
  return { name: `${server}_${tool}`, input: literal(code.slice(m.index! + m[0].length)) };
}

/** The object literal a script's call opens with, read as JSON: bare keys
 * and single quotes allowed, anything computed not. */
function literal(rest: string): Record<string, unknown> {
  const open = rest.trimStart();
  if (!open.startsWith("{")) return {};
  let depth = 0;
  let end = -1;
  for (let i = 0; i < open.length && end < 0; i++) {
    if (open[i] === "{") depth++;
    else if (open[i] === "}" && --depth === 0) end = i;
  }
  if (end < 0) return {};
  const json = open
    .slice(0, end + 1)
    .replace(/'([^'\\]*)'/g, (_, s: string) => JSON.stringify(s))
    .replace(/([{,]\s*)([A-Za-z_$][\w$]*)\s*:/g, '$1"$2":')
    .replace(/,\s*([}\]])/g, "$1");
  try {
    const v: unknown = JSON.parse(json);
    return typeof v === "object" && v && !Array.isArray(v) ? (v as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}
