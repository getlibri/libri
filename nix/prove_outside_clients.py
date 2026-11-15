#!/usr/bin/env python3
"""Stock MCP apps against a throwaway Libri: each searches and opens the
memory, sends into the main chat and waits for the answer, through a connector made
by key, as Settings > Other apps hands it out.

Not a flake check: it needs the apps installed and signed in. It never
touches a live Libri: the daemon and hippo are a throwaway (throwaway_libri),
the apps keep no session on disk (Claude Code --no-session-persistence, Codex
--ephemeral) and work in the scratch root, and the run fails if its mark is
found anywhere else in the home or in the caller's own memory afterwards.

Usage: prove_outside_clients.py LIBRI_DIR OUT_DIR
LIBRI_DIR holds bin/libri and bin/hippo; OUT_DIR gets one transcript per app,
the mark masked.
"""

import json
import os
import shutil
import subprocess
import sys
import urllib.request
from pathlib import Path

from check_outside import Http, connect, tool
from throwaway_libri import Throwaway

LIBRI_DIR, OUT = Path(sys.argv[1]).resolve(), Path(sys.argv[2])
TOOLS = ["memory_recall", "memory_open", "message_send", "events_wait"]


def ask(app):
    return (
        "Using only the libri MCP tools: 1) memory_recall for 'tidal' and memory_open the first hit's id. "
        f"2) message_send the text '{app} says the tidal clock book is renewed {box.mark}'. "
        "3) events_wait with the returned next as cursor (timeout_ms 20000), repeating with each new next until a "
        "message not from 'user' arrives. Then report what each tool returned, briefly."
    )


def claude(app, servers, model="sonnet"):
    config = box.root / f"{app}.json"
    config.write_text(json.dumps({"mcpServers": servers}))
    allowed = " ".join(f"mcp__libri__{t}" for t in TOOLS)
    got = subprocess.run(
        ["claude", "-p", "--model", model, "--no-session-persistence", "--strict-mcp-config",
         "--mcp-config", str(config), "--allowedTools", allowed, "--output-format", "stream-json", "--verbose",
         ask(app)],
        cwd=box.root / "work", env=apps_env, capture_output=True, text=True, timeout=400, stdin=subprocess.DEVNULL)
    calls = []
    for line in got.stdout.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if event.get("type") == "assistant":
            calls += [c["name"].removeprefix("mcp__libri__") for c in event["message"]["content"]
                      if c["type"] == "tool_use"]
    return got.returncode, got.stdout, calls


def codex(app):
    got = subprocess.run(
        ["codex", "exec", "--ephemeral", "--json", "--skip-git-repo-check", "-s", "read-only",
         "-c", 'approval_policy="never"', "-c", f'mcp_servers.libri.url="{box.base}/mcp"',
         "-c", 'mcp_servers.libri.bearer_token_env_var="LIBRI_CONNECTOR_KEY"',
         # Codex asks before every MCP call; with no one to ask it refuses,
         # so the proof lets this server's tools run.
         "-c", 'mcp_servers.libri.default_tools_approval_mode="approve"', ask(app)],
        cwd=box.root / "work", env=apps_env, capture_output=True, text=True, timeout=400, stdin=subprocess.DEVNULL)
    calls = []
    for line in got.stdout.splitlines():
        try:
            item = json.loads(line).get("item") or {}
        except ValueError:
            continue
        if item.get("type") == "mcp_tool_call" and item.get("status") == "completed":
            calls.append(item.get("tool"))
    return got.returncode, got.stdout, calls


def main():
    global box, apps_env
    box = Throwaway(LIBRI_DIR, prefix="libri-proof-")
    # The apps run as the person does: their own sign-ins and config, the
    # Desktop entry's `libri` on the PATH, and the relay finding this daemon
    # as it finds the host's own through its config.
    apps_env = os.environ | {"PATH": f"{LIBRI_DIR}/bin:{os.environ['PATH']}", "LIBRI_SITE": f"127.0.0.1:{box.port}",
                             "TMPDIR": str(box.root / "work")}
    for k in ("LIBRI_CONNECTOR_KEY", "HIPPO_DIR"):
        apps_env.pop(k, None)
    failed, transcripts = [], {}
    try:
        def post(path, body):
            req = urllib.request.Request(f"{box.base}{path}", data=json.dumps(body).encode(),
                                         headers={"content-type": "application/json", "origin": box.base})
            return urllib.request.urlopen(req, timeout=30).read()

        # One connector by key, as Settings makes it for apps that take a header.
        key = json.loads(post("/api/connectors", {"name": "Proof", "modules": ["memory", "chats"]}))["key"]
        apps_env["LIBRI_CONNECTOR_KEY"] = key
        post("/api/run", {"messages": [{"id": "seed1", "role": "user",
                                        "content": "The library book on tidal clocks is due back on the 14th"}]})
        runs = {
            "Claude Code over HTTP": lambda a: claude(a, {"libri": {
                "type": "http", "url": f"{box.base}/mcp", "headers": {"Authorization": f"Bearer {key}"}}}),
            # Settings' entry for Claude Desktop, as written there.
            "Claude Code with the Claude Desktop entry": lambda a: claude(
                a, {"libri": {"command": "libri", "args": ["mcp"]}}, model="haiku"),
            "Codex over HTTP": codex,
        }
        reader = Http(f"{box.base}/mcp", key)
        connect(reader)
        for app, run in runs.items():
            rc, out, calls = run(app)
            transcripts[app] = out
            _, chat = tool(reader, "messages_read", chat="main")
            arrived = f"{app} says the tidal clock book is renewed {box.mark}" in chat
            ok = rc == 0 and all(t in calls for t in TOOLS) and arrived
            print(f"{'ok  ' if ok else 'FAIL'} {app}: rc={rc} calls={calls} arrived={arrived}", flush=True)
            if not ok:
                failed.append(app)
        strays = box.strays()
        print(f"{'ok  ' if not strays else 'FAIL'} nothing written outside the scratch root {strays or ''}")
        if strays:
            failed.append("leak")
    finally:
        log = box.shown((box.root / "libri.log").read_text()[-3000:])
        box.close()
    OUT.mkdir(parents=True, exist_ok=True)
    for app, out in transcripts.items():
        (OUT / (app.lower().replace(" ", "-") + ".jsonl")).write_text(box.shown(out))
    if failed:
        print(log)
        sys.exit(f"{len(failed)} failed: {', '.join(failed)}")
    print("all passed")


if __name__ == "__main__":
    main()
