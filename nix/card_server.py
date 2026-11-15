#!/usr/bin/env python3
"""A small MCP server with one MCP Apps card, for proving Libri's cards end to
end: `show_greeting` (the model's) names the card `ui://card-test/greeting`,
and `count` (the card's alone) is what its button calls. The card speaks the
spec's postMessage protocol by hand, with no SDK, so it tests the host and
nothing else: it initializes, takes the tool's input and result, follows the
host's theme and colours, reports its size, calls a tool, sends a message to
the chat and asks to open a link. Stdio, newline-delimited JSON-RPC.
"""
import json
import sys

URI = "ui://card-test/greeting"
MIME = "text/html;profile=mcp-app"
TOOLS = [
    {
        "name": "show_greeting",
        "description": "Show a greeting card for a name. Call it when asked to greet someone.",
        "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]},
        "annotations": {"readOnlyHint": True},
        # Its card is the point of the call: shown with each (apps.rs OPENS).
        "_meta": {"ui": {"resourceUri": URI}, "is.libri/opens": True},
    },
    {
        "name": "look_up_greeting",
        "description": "Look up the greeting for a name, for the model's own use.",
        "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]},
        "annotations": {"readOnlyHint": True},
        # A lookup with a card, which shows only on asking.
        "_meta": {"ui": {"resourceUri": URI}},
    },
    {
        "name": "count",
        "description": "Add one to the card's counter.",
        "inputSchema": {"type": "object", "properties": {"n": {"type": "integer"}}},
        "_meta": {"ui": {"resourceUri": URI, "visibility": ["app"]}},
    },
]

CARD = """<!doctype html>
<html><head><meta charset="utf-8">
<style>
:root { color-scheme: light dark; --color-background-primary: light-dark(#fff, #1b1b1b);
  --color-text-primary: light-dark(#1b1b1b, #f2f2f2); --color-text-secondary: light-dark(#5f5f5f, #a8a8a8);
  --color-border-primary: light-dark(#d9d9d9, #3a3a3a); --font-sans: system-ui, sans-serif; --border-radius-md: 8px; }
html, body { margin: 0; background: var(--color-background-primary); color: var(--color-text-primary);
  font: 15px/1.45 var(--font-sans); }
main { padding: 14px 16px; }
h1 { font-size: 18px; margin: 0 0 4px; font-weight: 600; }
p { margin: 0 0 10px; color: var(--color-text-secondary); }
.row { display: flex; gap: 8px; flex-wrap: wrap; }
button { font: inherit; color: inherit; background: transparent; border: 1px solid var(--color-border-primary);
  border-radius: var(--border-radius-md); padding: 6px 12px; cursor: pointer; }
</style></head>
<body><main>
<h1 id="hello">Waiting for the tool…</h1>
<p id="detail">Card test: no result yet.</p>
<div class="row">
  <button id="count">Count: <span id="n">0</span></button>
  <button id="say">Send to chat</button>
  <button id="link">Open spec</button>
</div>
</main>
<script>
let id = 0;
const waiting = new Map();
function request(method, params) {
  const i = ++id;
  parent.postMessage({ jsonrpc: "2.0", id: i, method, params }, "*");
  return new Promise((ok, fail) => waiting.set(i, { ok, fail }));
}
function notify(method, params) { parent.postMessage({ jsonrpc: "2.0", method, params }, "*"); }
function style(ctx) {
  if (!ctx) return;
  if (ctx.theme) document.documentElement.style.colorScheme = ctx.theme;
  for (const [k, v] of Object.entries(ctx.styles?.variables || {})) if (v) document.documentElement.style.setProperty(k, v);
}
addEventListener("message", (e) => {
  if (e.source !== parent) return;
  const m = e.data || {};
  if (m.id && waiting.has(m.id)) {
    const w = waiting.get(m.id); waiting.delete(m.id);
    return m.error ? w.fail(m.error) : w.ok(m.result);
  }
  if (m.method === "ui/notifications/tool-input") {
    document.getElementById("hello").textContent = "Hello, " + (m.params.arguments.name || "?");
  } else if (m.method === "ui/notifications/tool-result") {
    const s = m.params.structuredContent;
    const text = (m.params.content || []).map((c) => c.text).join(" ");
    document.getElementById("detail").textContent = s ? "Structured result: " + JSON.stringify(s) : "Text result: " + text;
  } else if (m.method === "ui/notifications/host-context-changed") {
    style(m.params);
  } else if (m.method === "ui/resource-teardown") {
    parent.postMessage({ jsonrpc: "2.0", id: m.id, result: {} }, "*");
  }
});
document.getElementById("count").onclick = async () => {
  const n = Number(document.getElementById("n").textContent);
  const r = await request("tools/call", { name: "count", arguments: { n } });
  document.getElementById("n").textContent = r.structuredContent.n;
};
document.getElementById("say").onclick = () =>
  request("ui/message", { role: "user", content: [{ type: "text", text: "The card test says hello." }] });
document.getElementById("link").onclick = () =>
  request("ui/open-link", { url: "https://modelcontextprotocol.io/extensions/apps" });
(async () => {
  const r = await request("ui/initialize", {
    protocolVersion: "2026-01-26",
    appInfo: { name: "card-test", version: "1" },
    appCapabilities: { availableDisplayModes: ["inline"] },
  });
  style(r.hostContext);
  notify("ui/notifications/initialized", {});
  new ResizeObserver(() => notify("ui/notifications/size-changed", {
    width: document.documentElement.scrollWidth, height: document.documentElement.scrollHeight,
  })).observe(document.documentElement);
})();
</script></body></html>
"""


def reply(i, result):
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": i, "result": result}) + "\n")
    sys.stdout.flush()


def fail(i, message):
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": i, "error": {"code": -32602, "message": message}}) + "\n")
    sys.stdout.flush()


for line in sys.stdin:
    m = json.loads(line)
    method, i, p = m.get("method"), m.get("id"), m.get("params") or {}
    if method == "initialize":
        reply(i, {"protocolVersion": p.get("protocolVersion", "2025-06-18"),
                  "capabilities": {"tools": {}, "resources": {}},
                  "serverInfo": {"name": "card-test", "version": "1"}})
    elif method == "tools/list":
        reply(i, {"tools": TOOLS})
    elif method == "tools/call" and p.get("name") in ("show_greeting", "look_up_greeting"):
        name = p.get("arguments", {}).get("name", "?")
        reply(i, {"content": [{"type": "text", "text": f"Showed a greeting card for {name}."}],
                  "structuredContent": {"greeting": f"Hello, {name}", "name": name}})
    elif method == "tools/call" and p.get("name") == "count":
        n = int(p.get("arguments", {}).get("n", 0)) + 1
        reply(i, {"content": [{"type": "text", "text": str(n)}], "structuredContent": {"n": n}})
    elif method == "tools/call":
        fail(i, f"no tool {p.get('name')}")
    elif method == "resources/list":
        reply(i, {"resources": [{"uri": URI, "name": "greeting", "mimeType": MIME}]})
    elif method == "resources/read" and p.get("uri") == URI:
        reply(i, {"contents": [{"uri": URI, "mimeType": MIME, "text": CARD,
                                "_meta": {"ui": {"prefersBorder": True}}}]})
    elif method == "resources/read":
        fail(i, f"no resource {p.get('uri')}")
    elif i is not None:
        reply(i, {})
