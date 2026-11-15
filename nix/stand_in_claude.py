# Claude Code as Libri runs it, for the checks that have no real one
# (check_memory.py, check_service.nix): stream-json out, the turn's work on the
# stream. Records how it was started. Its sessions are kept as the real one's
# are: --session-id makes one, --resume takes one up, and one it doesn't have
# fails as the CLI does. Left to keep a session, it writes a transcript.
import json, os, re, subprocess, sys, time, uuid
from pathlib import Path
args = sys.argv
if args[1:3] == ["auth", "status"]:
    sys.exit(0)
log = Path(os.environ["STAND_IN_LOG"])
plain = "--safe-mode" in args
def say(ev):
    print(json.dumps(ev), flush=True)
def end(text):
    say({"type": "result", "subtype": "success", "is_error": False, "result": text,
         "session_id": "s", "usage": {}, "modelUsage": {}})
line = sys.stdin.readline()
asked = json.loads(line)
# Libri's model list: Claude Code's answer to the SDK's initialize.
if asked.get("type") == "control_request" and asked["request"]["subtype"] == "initialize":
    say({"type": "control_response", "response": {"subtype": "success", "request_id": asked["request_id"],
         "response": {"models": [{"value": "default", "displayName": "Default"},
                                 *({"value": m, "displayName": m.title(), "supportedEffortLevels": ["low", "high"]}
                                   for m in ("opus", "sonnet", "haiku")),
                                 {"value": "claude-sonnet-4-6", "displayName": "Sonnet 4.6"}]}}})
    sys.exit()
# The sessions it has, in a file beside its log, which a check may empty
# to stand for sessions the CLI lost.
sessions = log.parent / "stand-in-sessions"
known = set(sessions.read_text().split()) if sessions.exists() else set()
opt = lambda flag: args[args.index(flag) + 1] if flag in args else None
resumed = opt("--resume")
if resumed and resumed not in known:
    print(f"No conversation found with session ID: {resumed}", file=sys.stderr)
    sys.exit(1)
own = opt("--session-id") or resumed
if own and own not in known:
    with sessions.open("a") as f:
        f.write(own + "\n")
content = asked["message"]["content"]
said = content if isinstance(content, str) else "".join(c.get("text", "") for c in content)
# What its session holds, as the real one's does: every message sent to it,
# and a fork's start a copy of what it forked.
threads = log.parent / "stand-in-threads"
threads.mkdir(exist_ok=True)
held = (threads / resumed).read_text() if resumed and (threads / resumed).exists() else ""
thread = held + said + "\n"
if own:
    (threads / own).write_text(thread)
with log.open("a") as f:
    f.write(json.dumps({"args": args[1:], "said": said, "plain": plain}) + "\n")
# Its own slash commands, as the real one's init line lists them.
say({"type": "system", "subtype": "init", "slash_commands": ["compact", "context"]})
if plain:
    end("stand-in summary")
    sys.exit()
# Its own task alone, as the roster shows every worker the others'.
if re.search(r"^user: WORK SLOWLY", said, re.M):
    # "WORK SLOWLY AGENT": starts a harness subagent first, as Claude Code's
    # Agent call does, which runs on while it works.
    if re.search(r"^user: WORK SLOWLY AGENT", said, re.M):
        say({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "a1", "name": "Agent",
             "input": {"description": "Search the job boards", "subagent_type": "Explore", "prompt": "look"}}]}})
    time.sleep(60)
# A worker on orders in its task reaches the tool server it was handed, by
# the command and environment in its --mcp-config, as a harness does.
def server():
    given = json.loads(args[args.index("--mcp-config") + 1])["mcpServers"]["libri"]
    proc = subprocess.Popen([given["command"], *given["args"]], env=os.environ | given["env"], text=True,
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    def call(method, params):
        proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}) + "\n")
        proc.stdin.flush()
        return json.loads(proc.stdout.readline())
    call("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                        "clientInfo": {"name": "stand-in", "version": "0"}})
    proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
    proc.stdin.flush()
    def tool(called, **arguments):
        got = call("tools/call", {"name": called, "arguments": arguments})["result"]
        return got.get("isError", False), got["content"][0]["text"]
    return given, call, tool
# A worker's task leads this run's <orders>, whether or not <work> came
# before it; what it was told before is in its thread.
work = said[said.find("<orders>"):]
orders = re.search(r"^user: (\S+) ?(.*)$", work, re.M)
entries = re.findall(r"^(?:user|from|to): .*$", thread[thread.rfind("<orders>"):], re.M)
def target(name):
    """HEAD names the worker that started this one, as its prompt says."""
    if name != "HEAD":
        return name
    return re.search(r"(\S+) gave you a task", args[args.index("--append-system-prompt") + 1])[1]
if orders and orders[1] == "LISTEN":
    end("heard it")
    sys.exit()
if orders and orders[1] == "CHAT":
    _, _, tool = server()
    if "ENOUGH" in thread and "(reply)" in entries[-1]:
        end("finished")
        sys.exit()
    refused, text = tool("tell_agent", name=target(orders[2]), message="the next question")
    end(f"refused: {text}" if refused else "asked")
    sys.exit()
if orders and orders[1] == "SEND":
    _, _, tool = server()
    if "(reply)" in entries[-1]:
        end("answered")
        sys.exit()
    to, words = orders[2].split(" ", 1)
    refused, text = tool("tell_agent", name=target(to), message=words.replace("|NL|", "\n"))
    end(f"refused: {text}" if refused else "sent")
    sys.exit()
if orders and orders[1] == "ASK":
    # Asks the main chat for a sub-worker on the rest of its orders, and
    # finishes once one has reported to it.
    _, _, tool = server()
    if "(reply)" in entries[-1]:
        end("sub answered")
        sys.exit()
    # A head its sub-worker writes to answers it, slowly if asked to.
    if entries[-1].startswith("from: "):
        if "SLOWLY" in entries[-1]:
            time.sleep(60)
        end("heard it")
        sys.exit()
    refused, text = tool("request_subworker", task=orders[2], why="it is another kind of work")
    with log.open("a") as f:
        f.write(json.dumps({"args": [], "said": "", "plain": False, "asked_sub": {
            "brief": orders[2], "refused": refused, "text": text, "args": args[1:],
            "system": args[args.index("--append-system-prompt") + 1], "prompt": said}}) + "\n")
    end(f"refused: {text}" if refused else "asked")
    sys.exit()
# "TOOL name seconds": holds a call to the named tool open that long, as a
# slow read, edit or build does (drive_cards.py colours a worker by it).
if orders and orders[1] == "TOOL":
    name, seconds = orders[2].split()
    say({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "held", "name": name, "input": {}}]}})
    time.sleep(float(seconds))
    say({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "held", "content": "done"}]}})
    end("used it")
    sys.exit()
# "CRASH": the run fails, as a harness that dies does.
if orders and orders[1] == "CRASH":
    say({"type": "result", "subtype": "error", "is_error": True, "result": "stand-in crashed"})
    sys.exit(1)
# "NEED ask": asks the person from its card and stops.
if orders and orders[1] == "NEED":
    _, _, tool = server()
    refused, text = tool("ask_person", need="answer", ask=orders[2])
    end(f"refused: {text}" if refused else "asked the person")
    sys.exit()
if orders and orders[1] == "SYSTEM":
    with log.open("a") as f:
        f.write(json.dumps({"args": [], "said": "", "plain": False, "shown": {
            "words": orders[2], "system": args[args.index("--append-system-prompt") + 1], "prompt": said}}) + "\n")
    end("shown")
    sys.exit()
if orders and orders[1] == "LOOK":
    given, call, tool = server()
    names = [t["name"] for t in call("tools/list", {})["result"]["tools"]]
    servers = json.loads(args[args.index("--mcp-config") + 1])["mcpServers"]
    with log.open("a") as f:
        f.write(json.dumps({"args": [], "said": "", "plain": False, "looked": {
            "system": args[args.index("--append-system-prompt") + 1], "worker": given["env"].get("LIBRI_WORKER"),
            "tools": names, "roster": tool("workers")[1], "servers": sorted(servers),
            "chat": tool("agent_chat", name=orders[2]), "args": args[1:],
            "self": tool("tell_agent", name=given["env"]["LIBRI_WORKER"], message="x"),
            "asked": tool("tell_agent", name="main", message="please start a worker, as this needs two"),
            "prompt": said}}) + "\n")
    end("looked")
    sys.exit()
last = said.strip().removesuffix("</orders>").strip().split("\n")[-1].split(": ", 1)[-1]
# The device's mark Libri puts before the words when it changes (origin.rs).
if last.startswith("(from ") and ") " in last:
    last = last.split(") ", 1)[1]
delta = lambda t: say({"type": "stream_event", "event": {"type": "content_block_delta",
                       "delta": {"type": "text_delta", "text": t}}})
# "STREAM n [seed]": a reply of n words streamed a few at a time, as a model's
# comes, with paragraphs, a list and code (drive_ui.py measures the page under it).
streamed = re.match(r"STREAM (\d+)(?: (\d+))?$", last)
if streamed:
    import random
    pick = random.Random(int(streamed[2] or 0))
    words = "the run state check host build page chat worker summary memory reply list file test step".split()
    out = []
    for k in range(int(streamed[1])):
        out.append(pick.choice(words) + ("." if k % 17 == 16 else ""))
        if k % 60 == 59:
            out.append("\n\n- an item with `code`\n- another item\n\n```rust\nfn main() {}\n```\n\n")
    text = ""
    for k in range(0, len(out), 3):
        piece = " ".join(out[k:k + 3]) + " "
        text += piece
        delta(piece)
        time.sleep(0.02)
    end(text)
    sys.exit()
# "PUBLISH name": a Library page through Libri's own tool server, then its link.
if last.startswith("PUBLISH "):
    name = last.split(" ", 1)[1]
    _, _, tool = server()
    page = {"name": name, "html": f"<h1 id='mark-{name}'>Page {name}</h1>"}
    say({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "p1", "name": "mcp__libri__show_page", "input": page}]}})
    _, got = tool("show_page", **page)
    say({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "p1", "content": got}]}})
    reply = f"Made [page {name}]({os.environ.get('LIBRI_SITE_URL', '')}/#page={name})."
    delta(reply)
    end(reply)
    sys.exit()
# "RECALL words": which memory the memory's server it was handed reads:
# the one HIPPO2_MEMORY names, else main; none when it has no such server.
if last.startswith("RECALL "):
    servers = json.loads(args[args.index("--mcp-config") + 1])["mcpServers"]
    memory = servers["memory-stand-in"]["env"].get("HIPPO2_MEMORY", "main") if "memory-stand-in" in servers else "none"
    end(f"recalled from memory {memory}")
    sys.exit()
# "SPAWN task": starts a worker on the task through the tool server it was
# handed, and says what the tool answered and the store that server reads.
if last.startswith("SPAWN "):
    given, _, tool = server()
    refused, text = tool("spawn", tasks=[{"task": last.split(" ", 1)[1]}])
    env = given["env"]
    end(f"spawned from {env.get('HIPPO_DIR')} memory={env.get('LIBRI_MEMORY')} refused={refused}: {text}")
    sys.exit()
# "PERMIT": asks leave to run a shell command, as Claude Code's permission
# prompt does, and says what the person answered.
if last == "PERMIT":
    say({"type": "control_request", "request_id": "permit", "request": {"subtype": "can_use_tool", "tool_name": "Bash", "input": {"command": "date"}}})
    while True:
        got = sys.stdin.readline()
        if not got:
            end("no answer")
            sys.exit()
        if "control_response" in got:
            break
    allowed = json.loads(got)["response"].get("response", {}).get("behavior") == "allow"
    end("permitted" if allowed else "refused")
    sys.exit()
# "LOOK name": the card test's lookup, whose card shows only on asking;
# "SHOWCARD name": Libri's show_card naming that lookup, which shows it.
if last.startswith(("LOOK ", "SHOWCARD ")):
    kind, name = last.split(" ", 1)
    call = ({"name": "mcp__card-test__look_up_greeting", "input": {"name": name}} if kind == "LOOK" else
            {"name": "mcp__libri__show_card", "input": {"tool": "mcp__card-test__look_up_greeting", "arguments": {"name": name}}})
    say({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": f"l-{name}", **call}]}})
    say({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": f"l-{name}", "content": f"Looked up {name}."}]}})
    delta("Looked.")
    end("Looked.")
    sys.exit()
# "GREET name": a call of the card test's tool (card_server.py), which
# draws an MCP Apps card under it.
if last.startswith("GREET "):
    name = last[len("GREET "):].strip()
    say({"type": "assistant", "message": {"content": [
        {"type": "tool_use", "id": "g1", "name": "mcp__card-test__show_greeting", "input": {"name": name}}]}})
    say({"type": "user", "message": {"content": [
        {"type": "tool_result", "tool_use_id": "g1", "content": f"Hello, {name}!"}]}})
    delta("Greeted.")
    end("Greeted.")
    sys.exit()
if "USE A TOOL" in last:
    delta("Looking first.")
    say({"type": "assistant", "message": {"content": [
        {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "date"}}]}})
    say({"type": "user", "message": {"content": [
        {"type": "tool_result", "tool_use_id": "t1", "content": "the tool's output"}]}})
    say({"type": "assistant", "message": {"content": [
        {"type": "tool_use", "id": "t2", "name": "mcp__memory-stand-in__recall", "input": {"words": "x"}}]}})
    say({"type": "user", "message": {"content": [
        {"type": "tool_result", "tool_use_id": "t2", "content": "recalled, not kept"}]}})
    delta("Done with it.")
    end("Done with it.")
    sys.exit()
# A bare /compact answers nothing, as the real one's does.
if said.strip().endswith("\n/compact") or said.strip() == "/compact":
    end("")
    sys.exit()
reply = "claude answers: " + last
if "--no-session-persistence" not in args:
    title = args[args.index("--name") + 1] if "--name" in args else "untitled"
    session = str(uuid.uuid4())
    project = Path(os.environ["HOME"], ".claude/projects/-scratch")
    project.mkdir(parents=True, exist_ok=True)
    stamp = "2026-10-30T12:00:00.000Z"
    entries = [
        {"type": "custom-title", "customTitle": title, "sessionId": session},
        {"type": "user", "uuid": str(uuid.uuid4()), "timestamp": stamp, "cwd": "/scratch",
         "sessionId": session, "message": {"role": "user", "content": said}},
        {"type": "assistant", "uuid": str(uuid.uuid4()), "timestamp": stamp, "sessionId": session,
         "message": {"role": "assistant", "content": [{"type": "text", "text": reply}]}},
    ]
    with open(project / f"{session}.jsonl", "w") as f:
        for e in entries:
            f.write(json.dumps(e) + "\n")
say({"type": "stream_event", "event": {"type": "content_block_delta",
     "delta": {"type": "text_delta", "text": reply}}})
end(reply)
