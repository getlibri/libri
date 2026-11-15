"""A stand-in for `libri tools`: serves the tool list a dump of the real
server gave, records each call, and answers from the case's canned results.

Env: STUB_NAME (server name, default libri), STUB_TOOLS (tools/list JSON), STUB_LOG (JSONL of calls),
STUB_RESULTS (JSON: tool name to result text; "*" for the rest)."""
import json, os, sys

tools = json.load(open(os.environ["STUB_TOOLS"]))
results = json.loads(os.environ.get("STUB_RESULTS", "{}"))
log = open(os.environ["STUB_LOG"], "a")
# A list gives successive calls of that tool successive results, the last repeating.
seen = {}

for line in sys.stdin:
    msg = json.loads(line)
    if "id" not in msg:
        continue
    method, mid = msg.get("method"), msg["id"]
    if method == "initialize":
        result = {
            "protocolVersion": msg["params"].get("protocolVersion", "2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": os.environ.get("STUB_NAME", "libri"), "version": "eval"},
        }
    elif method == "tools/list":
        result = {"tools": tools}
    elif method == "tools/call":
        name = msg["params"]["name"]
        args = msg["params"].get("arguments") or {}
        if name == "watch" and not any(v not in (None, False, [], {}) for v in args.values()):
            # watch with nothing lists them: a case gives the list it holds.
            name = "watch:list"
            text = results.get(name, "No watches.")
        else:
            text = results.get(name, results.get("*", "Done."))
        log.write(json.dumps({"name": msg["params"]["name"], "args": args}) + "\n")
        log.flush()
        if isinstance(text, list):
            text = text[min(seen.get(name, 0), len(text) - 1)]
        seen[name] = seen.get(name, 0) + 1
        result = {"content": [{"type": "text", "text": text}], "isError": False}
    else:
        result = {}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": mid, "result": result}) + "\n")
    sys.stdout.flush()
