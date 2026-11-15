#!/usr/bin/env python3
"""The person's words reach the turn under way, in order, whoever began it;
a routine stays in the worker named for it; and the scratchpad is a
full chat that leaves nothing in the memory.

Drives the real daemon against a real hippo in a scratch directory, with a
stand-in `claude` that stays open on background jobs as Claude Code does, and
reads the page's own event stream. Usage: check_delivery.py LIBRI_DIR (holding
bin/libri, bin/hippo).
"""

import json
import os
import re
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from datetime import date
from pathlib import Path

LIBRI = Path(sys.argv[1]) / "bin/libri"
HIPPO = Path(sys.argv[1]) / "bin/hippo"
TITLE = "Iris"
# A photo past the pipe's 64 KiB: the prompt's write, which opens the way into
# the turn, waits for the stand-in to read it.
PIPE_FULL = b"\x89PNG" + bytes(70_000)

# Claude Code as Libri runs it. A prompt with HOLD A JOB answers and keeps a
# background job running until the file `release` appears, then answers again.
# Words sent meanwhile are logged in `read.jsonl` and answered as they come.
# A scratchpad's turn, the only one given a config directory of its own, writes
# what it was given to `pad.jsonl` and reaches Libri's tool server as a harness
# does; its last words say what to do, see `pad`.
CLAUDE = r"""import json, os, re, select, subprocess, sys, time
from datetime import datetime, timedelta
from datetime import date
from pathlib import Path
args = sys.argv
if args[1:3] == ["auth", "status"]:
    sys.exit(0)
root = Path(os.environ["STAND_IN_ROOT"])
plain = "--safe-mode" in args
def say(ev):
    print(json.dumps(ev), flush=True)
def jobs(*names):
    say({"type": "system", "subtype": "background_tasks_changed",
         "tasks": [{"description": n} for n in names]})
def answer(text):
    say({"type": "stream_event", "event": {"type": "content_block_delta",
         "delta": {"type": "text_delta", "text": text}}})
    say({"type": "result", "subtype": "success", "is_error": False, "result": text,
         "session_id": "s", "usage": {}, "modelUsage": {}})
def words(line):
    content = json.loads(line)["message"]["content"]
    return content if isinstance(content, str) else "".join(c.get("text", "") for c in content)
if not plain and (root / "hold-start").exists():
    (root / "hold-start").unlink()
    (root / "started").touch()
    while not (root / "go").exists():
        time.sleep(0.05)
said = words(sys.stdin.readline())
# Libri's tool server, by the config a turn was started with.
class Server:
    def __init__(self):
        config = json.loads(args[args.index("--mcp-config") + 1])["mcpServers"]
        self.names = sorted(config)
        own = config["libri"]
        self.proc = subprocess.Popen([own["command"], *own["args"]], env=os.environ | own.get("env", {}),
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.id = 0
        self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                "clientInfo": {"name": "stand-in", "version": "0"}})
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.proc.stdin.flush()
    def rpc(self, method, params):
        self.id += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        return json.loads(self.proc.stdout.readline())
    def tools(self):
        return sorted(t["name"] for t in self.rpc("tools/list", {})["result"]["tools"])
    def call(self, name, arguments):
        got = self.rpc("tools/call", {"name": name, "arguments": arguments})
        return "".join(c.get("text", "") for c in got.get("result", {}).get("content", [])) or json.dumps(got)
# PAD TOOL thinks, says words, calls a tool whose result runs long, and
# answers. PAD HOLD stays open until `pad-go`, taking words sent meanwhile.
# PAD FAIL fails as a prompt past the model's limit does. PAD SHOW works a
# step and holds with thoughts and words in hand. PAD ASK asks and answers.
def pad():
    server = Server()
    tools = server.tools()
    system = next(args[i + 1] for i, a in enumerate(args) if a in ("--append-system-prompt", "--system-prompt"))
    work = said.index("<work>")
    told = re.findall(r"^\d+\|user: (.*)$", said[work:], re.M)[-1]
    seen = {"before": said[:work], "work": said[work:], "system": system, "args": args, "tools": tools,
            "servers": server.names, "config": os.environ.get("CLAUDE_CONFIG_DIR"), "cwd": os.getcwd(), "told": told}
    if "--session-id" in args:
        # What Claude Code spills to disk under the session, memory or no.
        spill = Path(os.environ["HOME"], ".claude/projects/-pad", args[args.index("--session-id") + 1])
        (spill / "tool-results").mkdir(parents=True)
        (spill / "tool-results/long.txt").write_text("PADSPILL")
        env = Path(os.environ["HOME"], ".claude/session-env", spill.name)
        env.mkdir(parents=True)
        seen["spill"], seen["session_env"] = str(spill), str(env)
    if "PAD TOOL" in told:
        if "lists" in tools:
            seen["search"] = server.call("lists", {})
        if "watch" in tools:
            at = (datetime.now() + timedelta(days=1)).strftime("%Y-%m-%d %H:%M")
            seen["reminded"] = server.call("watch", {"when": {"at": at}, "then": {"note": "PADREMINDER"}})
    with (root / "pad.jsonl").open("a") as f:
        f.write(json.dumps(seen) + "\n")
    def delta(kind, text):
        field = {"text_delta": "text", "thinking_delta": "thinking"}[kind]
        say({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": kind, field: text}}})
    if "PAD FAIL" in told:
        say({"type": "result", "subtype": "success", "is_error": True, "result": "Prompt is too long", "session_id": "s"})
        sys.exit(1)
    if "PAD TOOL" in told:
        delta("thinking_delta", "PADTHOUGHT")
        delta("text_delta", "Let me look.")
        say({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t1", "name": "mcp__libri__lists", "input": {}}]}})
        say({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t1", "content": seen.get("search", "") + "x" * 20000 + "PADRESULTEND"}]}})
        answer("Found it.")
    elif "PAD ASK" in told:
        say({"type": "control_request", "request_id": "pad", "request": {"subtype": "can_use_tool", "tool_name": "AskUserQuestion", "input": {"questions": [{"question": "Which draft?", "header": "Draft", "multiSelect": False, "options": [{"label": "Short", "description": "Two lines"}, {"label": "Long", "description": "All of it"}]}]}}})
        while "control_response" not in sys.stdin.readline():
            pass
        answer("Asked.")
    elif "PAD HOLD" in told or "PAD SHOW" in told:
        if "PAD SHOW" in told:
            delta("thinking_delta", "PADEARLY")
            delta("text_delta", "Looking first.")
            say({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "t2", "name": "mcp__libri__lists", "input": {}}]}})
            say({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "t2", "content": "PADSHOWN"}]}})
            delta("thinking_delta", "PADLATE")
        delta("text_delta", "Holding.")
        (root / "pad-held").touch()
        while not (root / "pad-go").exists():
            if select.select([sys.stdin], [], [], 0.05)[0]:
                line = sys.stdin.readline()
                if line.strip():
                    with (root / "pad-steered").open("a") as f:
                        f.write(words(line) + "\n")
        answer("Held.")
    else:
        answer("noted: " + told[:40])
    server.proc.terminate()
# A Temp turn keeps no session: a chat's own turns name theirs and keep it.
if "CLAUDE_CONFIG_DIR" in os.environ or ("--session-id" in args and "--no-session-persistence" in args and not plain):
    pad()
    sys.exit()
if plain:
    kind = re.search(r"<input>\n(\w+):", said)
    answer(f"{kind.group(1) if kind else 'talk'}: stand-in summary")
    sys.exit()
def note(text):
    with (root / "read.jsonl").open("a") as f:
        f.write(json.dumps({"at": time.time(), "said": text}) + "\n")
(root / "main-said").write_text(said)
# The prompt's last line; the view above it holds what was said before.
last = said.strip().removesuffix("</orders>").strip().split("\n")[-1]
note(last[-200:])
if "NOTHING TO SAY" in last:
    answer("(nothing new)")
    sys.exit()
if "HOLD A JOB" not in last:
    words = last.split(": ", 1)[-1]
    # The device's mark Libri puts before the words when it changes (origin.rs).
    if words.startswith("(from ") and ") " in words:
        words = words.split(") ", 1)[1]
    answer("answered: " + words[:60])
    sys.exit()
jobs("a long build")
name = "routine" if "Routine" in last else "person"
answer(f"{name} first answer")
while not (root / "release").exists():
    if select.select([sys.stdin], [], [], 0.1)[0]:
        line = sys.stdin.readline()
        if not line:
            break
        if line.strip():
            text = words(line)
            note(text)
            answer("answered: " + text.split(") ", 1)[-1][:60])
jobs()
answer(f"{name} last answer")
# Claude Code ends once its input closes.
sys.stdin.read()
"""


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def until(ready, what, within=20):
    end = time.monotonic() + within
    while time.monotonic() < end:
        if ready():
            return True
        time.sleep(0.1)
    raise TimeoutError(what)


class Rig:
    def __init__(self, root: Path):
        self.root = root
        for name in ("hippo", "home", "state"):
            (root / name).mkdir()
        claude = root / "claude"
        claude.write_text(f"#!{sys.executable}\n{CLAUDE}")
        claude.chmod(0o755)
        self.port = free_port()
        self.base_env = {
            "HOME": str(root / "home"),
            "PATH": os.environ["PATH"],
            # The host's pressure is not a throwaway's news (system.rs).
            "LIBRI_SYSTEM_ALERTS": "0",
            "HIPPO_DIR": str(root / "hippo"),
            "HIPPO_ARCHIVE": str(root / "archive"),
            "HIPPO_BACKEND": "none",
            "STAND_IN_ROOT": str(root),
        }
        self.hippo = subprocess.Popen(
            [HIPPO, "serve", "chat"],
            env=self.base_env,
            stdout=subprocess.DEVNULL,
            stderr=open(root / "hippo.log", "w"),
        )
        until(lambda: (root / "hippo/hippo.sock").exists(), "hippo up")
        self.start()
        self.tools = Tools(self)
        time.sleep(0.5)

    def start(self):
        root = self.root
        self.daemon = subprocess.Popen(
            [LIBRI],
            env=self.base_env
            | {
                "STATE_DIRECTORY": str(root / "state"),
                "LIBRI_NAME": "box-label",
                "LIBRI_PERSON": "Ada",
                "LIBRI_HIPPO_SHARED": TITLE,
                "LIBRI_BACKEND": "claude",
                "LIBRI_CLAUDE": str(root / "claude"),
                "LIBRI_MODEL": "stand-in",
                "LIBRI_SITE": f"127.0.0.1:{self.port}",
                "LIBRI_SITE_URL": f"http://127.0.0.1:{self.port}",
                "LIBRI_SITE_ROOT": str(LIBRI.parent.parent / "site"),
                "LIBRI_OPEN": "1",
                "LIBRI_WORK_TICK": "2",
            },
            stdout=subprocess.DEVNULL,
            stderr=open(root / "libri.log", "a"),
        )
        until(lambda: self.get("/api/host") is not None, "libri up")
        self.events = []
        threading.Thread(target=self.listen, args=(self.events,), daemon=True).start()
        until(lambda: self.get("/api/host") is not None, "stream up")

    def restart(self):
        self.daemon.terminate()
        self.daemon.wait()
        self.start()

    def request(self, path, body=None, timeout=20, method=None):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(
            f"http://127.0.0.1:{self.port}{path}",
            data=data,
            headers={"content-type": "application/json", "origin": f"http://127.0.0.1:{self.port}"},
            method=method,
        )
        return urllib.request.urlopen(req, timeout=timeout)

    def get(self, path):
        try:
            with self.request(path) as r:
                return r.read()
        except OSError:
            return None

    def listen(self, events):
        """What a page open now is told, as it happens."""
        try:
            with self.request("/api/events", timeout=None) as stream:
                for line in stream:
                    if line.startswith(b"data: "):
                        # The pulse carries the host's clock and newest
                        # event number, not an AG-UI event, so it has no type.
                        if "type" in (event := json.loads(line[6:])):
                            events.append(event)
        except OSError:
            pass  # A restart ends this stream; the next start opens another.

    def upload(self, data):
        """Sends a photo ahead of its message; answers its id."""
        body = (
            b'--b\r\ncontent-disposition: form-data; name="file"; filename="p.png"\r\n'
            b"content-type: image/png\r\n\r\n" + data + b"\r\n--b--\r\n"
        )
        req = urllib.request.Request(
            f"http://127.0.0.1:{self.port}/api/upload",
            data=body,
            headers={"content-type": "multipart/form-data; boundary=b", "origin": f"http://127.0.0.1:{self.port}"},
        )
        with urllib.request.urlopen(req, timeout=20) as r:
            return json.load(r)["id"]

    def send(self, text, photo=None):
        content = text
        if photo:
            content = [{"type": "text", "text": text}, {"type": "binary", "id": self.upload(photo)}]
        with self.request(
            "/api/run",
            {"messages": [{"id": f"m{time.monotonic_ns()}", "role": "user", "content": content}]},
        ):
            pass

    def shown(self):
        """The main chat's messages as a page reads them, oldest first. A
        worker's words carry its run's id and show under that worker."""
        out = {}
        for e in list(self.events):
            if "subagentRunId" in e:
                continue
            if e["type"] == "TEXT_MESSAGE_START":
                out[e["messageId"]] = ""
            elif e["type"] == "TEXT_MESSAGE_CONTENT":
                out[e["messageId"]] = out.get(e["messageId"], "") + e["delta"]
            elif e["type"] == "TEXT_MESSAGE_END" and e.get("metadata", {}).get("discarded"):
                out.pop(e["messageId"], None)
        return list(out.values())

    def on_page(self, text):
        return sum(text in m for m in self.shown())

    def read(self):
        """What the stand-in was told, oldest first."""
        f = self.root / "read.jsonl"
        return [json.loads(l)["said"] for l in f.read_text().splitlines()] if f.exists() else []

    def chat(self):
        out = subprocess.run(
            [HIPPO, "tail", "200", TITLE],
            env=self.base_env,
            capture_output=True,
            text=True,
            timeout=20,
        ).stdout
        return [json.loads(l) for l in out.splitlines() if l.startswith("{")]

    def count(self, kind, text):
        return sum(1 for m in self.chat() if m["kind"] == kind and text in m["text"])

    def stored(self):
        """Every message in the hippo store, however old."""
        out = subprocess.run(
            [HIPPO, "tail", "100000", TITLE],
            env=self.base_env,
            capture_output=True,
            text=True,
            timeout=20,
        ).stdout
        return out.splitlines()

    def in_store(self, word):
        """Whether `word` lies anywhere in hippo's files, whatever reads them."""
        needle = word.encode()
        return any(needle in f.read_bytes() for f in (self.root / "hippo").rglob("*") if f.is_file() and not f.is_socket())

    def pad(self):
        return json.loads(self.get("/api/scratchpad"))

    def tell_pad(self, text):
        with self.request("/api/scratchpad", {"text": text, "files": []}):
            pass

    def pad_turns(self):
        f = self.root / "pad.jsonl"
        # A long prompt is written in pieces, so the last line may be unfinished.
        return [json.loads(l) for l in f.read_text().split("\n")[:-1]] if f.exists() else []

    def pad_rests(self, within=20):
        return soon(lambda: not self.pad()["working"], within)

    def pad_answers(self, text):
        """Tells the scratchpad `text` and waits out the turn; answers what
        its harness was given."""
        had = len(self.pad_turns())
        self.tell_pad(text)
        until(lambda: len(self.pad_turns()) > had, f"a turn on {text[:40]}")
        self.pad_rests()
        return self.pad_turns()[-1]

    def routine(self, text):
        """A routine due at the daemon's next look, every 30 seconds."""
        at = time.strftime("%Y-%m-%d %H:%M", time.localtime())
        print(self.tools.call("watch", {"when": {"at": at}, "then": {"ask": text}}))
        until(lambda: any(text in s for s in self.read()), f"the routine {text}", within=45)

    def due_now(self, watch):
        """A daily routine due at the daemon's next look, not tomorrow."""
        at = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime())
        book = sqlite3.connect(self.root / "state/book.db", timeout=10)
        with book:
            book.execute("update watch set at = ? where id = ?", (at, watch))
        book.close()

    def thread(self, id):
        """A worker as the Work page lists it."""
        threads = json.loads(self.get("/api/work"))["threads"]
        return next(t for t in threads if t["id"] == id)

    def close(self):
        self.tools.close()
        for p in (self.daemon, self.hippo):
            p.terminate()
            p.wait()


class Tools:
    """Libri's tools at a person's scope, as a turn's harness reaches them."""

    def __init__(self, rig):
        self.proc = subprocess.Popen(
            [LIBRI, "tools", str(rig.root / "state")],
            env=rig.base_env
            | {
                "LIBRI_SCOPE": "person",
                "STATE_DIRECTORY": str(rig.root / "state"),
                "LIBRI_NAME": TITLE,
                "LIBRI_PERSON": "Ada",
            },
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        )
        self.id = 0
        self.rpc(
            "initialize",
            {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "check", "version": "0"}},
        )
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.proc.stdin.flush()

    def rpc(self, method, params):
        self.id += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        return json.loads(self.proc.stdout.readline())

    def call(self, name, arguments):
        return self.rpc("tools/call", {"name": name, "arguments": arguments})["result"]["content"][0]["text"]

    def close(self):
        self.proc.terminate()
        self.proc.wait()


failures = []


def check(what, ok, detail=""):
    print(f"ok   {what}" if ok else f"FAIL {what} {detail}", flush=True)
    if not ok:
        failures.append(what)


def soon(ready, within=8):
    try:
        return until(ready, "", within)
    except TimeoutError:
        return False


def routine_turn(rig):
    """The 10-30 stall: a routine with a background job held the turn lock
    for 13 minutes, and nothing the person sent could reach it."""
    rig.routine("HOLD A JOB in the routine")
    check("a routine's first answer shows while its job runs", soon(lambda: rig.on_page("routine first answer")))
    rig.send("first words")
    check("words reach a routine's turn at once", soon(lambda: any("first words" in s for s in rig.read())), rig.read())
    check("they are not left waiting", soon(lambda: rig.count("user", "first words") == 1))
    check("their answer shows while the job runs", soon(lambda: rig.on_page("answered: first words")), rig.shown())
    rig.send("second words")
    check("so do the next ones", soon(lambda: any("second words" in s for s in rig.read())), rig.read())
    (rig.root / "release").touch()
    check("the routine's last answer shows", soon(lambda: rig.on_page("routine last answer")), rig.shown())
    time.sleep(1)
    for text in ("routine first answer", "answered: first words", "routine last answer"):
        check(f"shown once: {text}", rig.on_page(text) == 1, rig.shown())
        check(f"kept once: {text}", rig.count("talk", text) == 1, rig.chat())
    (rig.root / "release").unlink()


def quiet_routine(rig):
    rig.routine("NOTHING TO SAY")
    time.sleep(2)
    check("a routine with nothing to say shows nothing", rig.on_page("(nothing new)") == 0, rig.shown())


def words_in_order(rig):
    """Words sent before a turn could take them go in ahead of later ones."""
    (rig.root / "hold-start").touch()
    rig.send("HOLD A JOB", PIPE_FULL)
    until(lambda: (rig.root / "started").exists(), "the turn's start")
    rig.send("sent early")
    time.sleep(1)
    (rig.root / "go").touch()
    until(lambda: rig.on_page("person first answer"), "the person's first answer")
    rig.send("sent later")
    ok = soon(lambda: any("sent later" in s for s in rig.read()))
    early = [i for i, s in enumerate(rig.read()) if "sent early" in s]
    later = [i for i, s in enumerate(rig.read()) if "sent later" in s]
    check("words sent at a turn's start reach it", ok and bool(early), rig.read())
    check("words reach the turn in the order sent", bool(early and later) and early[0] < later[0], rig.read())
    (rig.root / "release").touch()
    check("the turn ends", soon(lambda: rig.on_page("person last answer")))
    (rig.root / "release").unlink()


def stop_word(rig):
    """"stop" ends a turn the person did not begin, past all that waits."""
    rig.routine("HOLD A JOB to be stopped")
    since = len(rig.events)
    rig.send("stop")
    check(
        "stop ends a routine's turn",
        soon(lambda: any(e["type"] == "RUN_FINISHED" for e in rig.events[since:]), within=12),
    )
    rig.send("after the stop")
    check("words after a stop are answered", soon(lambda: rig.on_page("answered: after the stop"), within=12), rig.shown())


def scratchpad(rig):
    """With the memory on a turn has what the main chat has, and with it off
    the same but for what reads hippo; either way the store comes out as it
    went in."""
    rig.send("the word quetzal goes in the memory")
    until(lambda: rig.on_page("answered: the word quetzal"), "a word in the memory")
    until(lambda: rig.in_store("quetzal"), "the word in the store")
    time.sleep(2)
    before = rig.stored()
    pads = "scratchpad"

    on = rig.pad_answers("PAD TOOL PADFIRST")
    check("memory on: the chat's newest is in the turn", "quetzal" in on["before"], on["before"][:200])
    check("memory on: none of the old memory tools", "memory_search" not in on["tools"], on["tools"])
    check("memory on: it has Libri's prompt", "on the scratchpad" in on["system"], on["system"][:200])
    check("memory on: a reminder is set", on.get("reminded", "").startswith("Watch"), on.get("reminded"))
    check("memory on: it has the person's tools", {"watch", "lists"} <= set(on["tools"]), on["tools"])
    check("memory on: it starts no workers", not {"spawn", "tell_agent"} & set(on["tools"]), on["tools"])
    check("memory on: it runs on the person's own Claude Code", on["config"] is None, on["config"])
    check("what the CLI spills goes when the turn does", on["spill"] and not Path(on["spill"]).exists(), on["spill"])
    check("what the CLI keeps of its session goes too", not Path(on["session_env"]).exists(), on["session_env"])
    text = rig.pad()["text"]
    check("words before a call are in the log", "Let me look." in text, text[-300:])
    check("a call and its whole result are in the log", "lists" in text and "PADRESULTEND" in text, text[-300:])
    check("the answer is in the log", "Found it." in text, text[-300:])
    run = [e for e in rig.events if e.get("subagentRunId", "").startswith(pads)]
    kinds = {e["type"] for e in run}
    check("thinking streams to the page", any(e.get("delta") == "PADTHOUGHT" for e in run), kinds)
    check("words stream to the page", any("Let me look." in str(e.get("delta")) for e in run), kinds)
    check("a tool row streams to the page", "TOOL_CALL_START" in kinds, kinds)
    check("the main chat shows none of it", rig.on_page("Found it.") == 0)

    follow = rig.pad_answers("and a follow-up")
    check("a follow-up carries the tool's result", "PADRESULTEND" in follow["work"], follow["work"][-300:])
    check("a follow-up carries the words too", "PADFIRST" in follow["work"] and "Found it." in follow["work"])
    check("thinking is not carried", "PADTHOUGHT" not in follow["work"])

    rig.tell_pad("PAD HOLD one")
    until(lambda: (rig.root / "pad-held").exists(), "the held turn")
    rig.tell_pad("words while it runs")
    steered = rig.root / "pad-steered"
    check("words sent during a turn join it", soon(lambda: steered.exists() and "words while it runs" in steered.read_text()))
    (rig.root / "pad-go").touch()
    check("the held turn ends", rig.pad_rests())
    (rig.root / "pad-go").unlink()
    (rig.root / "pad-held").unlink()

    rig.tell_pad("PAD HOLD two")
    until(lambda: (rig.root / "pad-held").exists(), "the second held turn")
    with rig.request("/api/scratchpad/stop", {}):
        pass
    check("Stop ends the turn", rig.pad_rests())
    check("a stopped turn keeps its words", "Holding." in rig.pad()["text"], rig.pad()["text"][-200:])
    (rig.root / "pad-held").unlink()

    rig.get("/api/models")
    with rig.request("/api/model", {"harness": "claude", "model": "sonnet", "temp": True}):
        pass
    switched = rig.pad_answers("after a model change")
    model = switched["args"][switched["args"].index("--model") + 1]
    check("the model changes mid-thread", model == "sonnet", model)
    check("the new model has the whole thread", "PADRESULTEND" in switched["work"])

    held = rig.pad()["text"]
    failed = rig.pad_answers("PAD FAIL")
    text = rig.pad()["text"]
    check("a failed turn shows", "The run failed" in text and "too long" in text, text[-200:])
    check("a failed turn leaves the thread", text.startswith(held.rstrip()[:200]) and not rig.pad()["working"])
    big = "PAD BIG " + "w" * 3_000_000
    got = rig.pad_answers(big)
    check("words past 2 MB reach the harness on stdin", len(got["work"]) > 3_000_000, len(got["work"]))

    rig.tell_pad("PAD HOLD across a restart")
    until(lambda: (rig.root / "pad-held").exists(), "the turn before the restart")
    (rig.root / "pad-go").touch()
    rig.restart()
    check("a restart keeps the thread", "Found it." in rig.pad()["text"])
    check("a cut turn goes on", rig.pad_rests(30), rig.pad()["text"][-100:])
    check("a flip's last position stands by default", rig.pad()["memory"])
    (rig.root / "pad-held").unlink(missing_ok=True)

    with rig.request("/api/scratchpad/memory", {"on": False}):
        pass
    flipped = rig.pad_answers("a turn with the memory off")
    check("a flip takes effect from the next turn", "quetzal" not in flipped["before"] and "memory is off" in flipped["system"], flipped["before"][:100])
    cleared = rig.pad()["id"]
    with rig.request("/api/scratchpad", method="DELETE"):
        pass
    # The task store also keeps its own .talk.jsonl, the log of messages
    # between agents; Clear removes the pad's directory, not the store's log.
    check("Clear removes it", rig.pad()["id"] is None and not (rig.root / "state/scratchpad/task" / cleared).exists(), cleared)

    off = rig.pad_answers("PAD TOOL off first")
    check("memory off: no view", "quetzal" not in off["before"] and off["before"].split()[:1] == ["Now:"], off["before"][:200])
    check("memory off: no search", off.get("search", "").find("quetzal") < 0, off.get("search"))
    check("memory off: Libri's tools are the web alone", sorted(off["tools"]) == ["web_fetch", "web_search"], off["tools"])
    check("memory off: no other server", off["servers"] == ["libri"], off["servers"])
    check("memory off: no built-in tools", off["args"][off["args"].index("--tools") + 1] == "", off["args"])
    check("memory off: no server of the person's own", "--strict-mcp-config" in off["args"], off["args"])
    check("memory off: Libri's prompt, saying the memory is off", "on the scratchpad" in off["system"] and "memory is off" in off["system"], off["system"][:300])
    check("memory off: the person's own Claude Code", off["config"] is None and off["cwd"] == on["cwd"] and "--setting-sources" not in off["args"], (off["config"], off["cwd"]))
    check("memory off: what the CLI spills goes too", off.get("spill") and not Path(off["spill"]).exists(), off.get("spill"))
    check("the switch stands", json.loads(rig.get("/api/scratchpad"))["memory"] is False)
    rig.restart()
    check("the switch outlives a restart", rig.pad()["memory"] is False)

    after = rig.stored()
    check("hippo's count is what it was", len(after) == len(before), f"{len(before)} then {len(after)}")
    for word in ("PADRESULTEND", "PADFIRST", "a follow-up", "Holding.", "Found it.", "PADREMINDER", "words while it runs"):
        check(f"the store has no {word!r}", not rig.in_store(word))
    workers = json.loads(rig.get("/api/work"))["threads"]
    check("no scratchpad among the workers", not any(pads in t["id"] for t in workers), workers)
    with rig.request("/api/scratchpad", method="DELETE"):
        pass
    rig.send("back in the main chat")
    until(lambda: rig.on_page("answered: back in the main chat"), "a turn in the main chat")
    seen = (rig.root / "main-said").read_text()
    check("the main chat's view has none of it", "PAD TOOL" not in seen and "Found it." not in seen)


def two_clients(rig):
    """Two clients of one host, a page and a page or a page and the app: each
    hears the stream and reads /api/scratchpad, which holds all there is of
    the scratchpad, so what one does shows on the other."""
    rig.request("/api/scratchpad", method="DELETE").close()
    first, second = [], []
    for heard in (first, second):
        threading.Thread(target=rig.listen, args=(heard,), daemon=True).start()
    time.sleep(1)
    pings = lambda heard: sum(e.get("name") == "scratchpad" for e in heard)

    def both(what, ready):
        """Both clients are told, and read the same state, in which `ready` holds."""
        check(f"{what}: both read it", soon(lambda: ready(rig.pad())), rig.pad())
        a, b = rig.pad(), rig.pad()
        check(f"{what}: the same", a == b, (a, b))
        return a

    def acted(what, act, ready):
        had = pings(first), pings(second)
        act()
        check(f"{what}: both are told", soon(lambda: pings(first) > had[0] and pings(second) > had[1]), had)
        return both(what, ready)

    def fresh_hold():
        for name in ("pad-held", "pad-go"):
            (rig.root / name).unlink(missing_ok=True)

    fresh_hold()
    acted("a word sent on one", lambda: rig.tell_pad("PAD SHOW from the first"), lambda p: "PAD SHOW from the first" in p["text"] and p["working"])
    until(lambda: (rig.root / "pad-held").exists(), "the shown turn")
    on = both("a turn under way", lambda p: p["thinking"] == "PADLATE" and p["words"] == "Holding.")
    check("the tool row is in the log both read", "lists" in on["text"] and "PADSHOWN" in on["text"], on["text"])
    check("the words before it too", "Looking first." in on["text"], on["text"])
    check("thinking stays out of the log", "PADLATE" not in on["text"] and "PADEARLY" not in on["text"], on["text"])
    for heard in (first, second):
        kinds = {e.get("type") for e in heard if e.get("subagentRunId") == on["id"]}
        check("the stream brings words, thoughts and calls to each", {"TEXT_MESSAGE_CONTENT", "REASONING_MESSAGE_CONTENT", "TOOL_CALL_START"} <= kinds, kinds)

    both_read = lambda p: "words from the second" in p["text"] or "words from the second" in p["queued"]
    acted("a queued send", lambda: rig.tell_pad("words from the second"), both_read)
    steered = rig.root / "pad-steered"
    check("it reaches the run", soon(lambda: steered.exists() and "words from the second" in steered.read_text()), None)
    taken = both("taken", lambda p: "words from the second" in p["text"] and not p["queued"])
    check("after the words said before it", taken["text"].index("Holding.") < taken["text"].index("words from the second"), taken["text"])
    check("and the run's words are no longer in hand", taken["words"] == "", taken)

    stop = lambda: rig.request("/api/scratchpad/stop", {}).close()
    acted("Stop on the second", stop, lambda p: not p["working"])
    rest = both("stopped", lambda p: not p["working"])
    check("nothing in hand at rest", (rest["words"], rest["thinking"], rest["stopping"]) == ("", "", False), rest)
    check("the thread stays", "Looking first." in rest["text"], rest["text"])

    switch = lambda on: lambda: rig.request("/api/scratchpad/memory", {"on": on}).close()
    acted("the Memory switch on from one", switch(True), lambda p: p["memory"] is True)
    acted("off from the other", switch(False), lambda p: p["memory"] is False)
    acted("and on again", switch(True), lambda p: p["memory"] is True)

    def answers(client):
        rig.tell_pad(f"PAD ASK {client}")
        asked = both(f"an ask for {client}", lambda p: len(p["asks"]) == 1)
        ask = asked["asks"][0]
        reply = {"threadId": "t", "runId": "r", "messages": [], "resume": [{"interruptId": ask["id"], "status": "resolved", "payload": {"answers": {"Which draft?": "Short"}}}]}
        # Read once the answer is in the log: until then the two reads may
        # straddle its move there from the words in hand.
        settled = lambda p: not p["asks"] and p["text"].endswith("Asked.") and not p["words"]
        acted(f"the {client} answers", lambda: rig.request("/api/run", reply).close(), settled)
        check(f"the {client}'s answer ends the turn", rig.pad_rests(), rig.pad())
    answers("first")
    answers("second")

    fresh_hold()
    rig.tell_pad("PAD HOLD to be closed")
    until(lambda: (rig.root / "pad-held").exists(), "the turn to close")
    close = lambda: rig.request("/api/scratchpad", method="DELETE").close()
    gone = acted("Clear or Promote on one", close, lambda p: p["id"] is None)
    check("clears both", (gone["text"], gone["working"], gone["words"], gone["queued"], gone["asks"]) == ("", False, "", [], []), gone)
    check("none of it reached the memory", not rig.in_store("PADSHOWN") and not rig.in_store("from the first"))
    fresh_hold()


def routine_in_worker(rig):
    """A daily routine goes to the worker named for it and
    nothing lands in the main chat. Run first with none named, which shows the
    main chat would tell."""
    at = time.strftime("%Y-%m-%d %H:%M", time.localtime())
    set_daily = rig.tools.call("watch", {"when": {"at": at, "every": "daily"},
                                         "then": {"ask": "Your daily plan: what needs me today."}})
    print(set_daily)
    daily = int(re.search(r"Watch (\d+) set", set_daily).group(1))
    check("with no worker named the routine reaches the page", soon(lambda: rig.on_page("Your daily plan"), 40), rig.shown())
    check("and the main chat keeps it", soon(lambda: rig.count("talk", "Your daily plan") == 1), rig.chat())

    spawned = rig.tools.call("spawn", {"tasks": [{"task": "take the routine"}]})
    print(spawned)
    worker = json.loads(rig.get("/api/work"))["threads"][0]["id"]
    until(lambda: rig.thread(worker)["state"] == "waiting", "its first report", within=80)
    # Its report is a turn of the main chat, over before the counts below.
    time.sleep(3)
    with rig.request(f"/api/work/{worker}/read", {}):
        pass
    check("the worker has nothing to read", rig.thread(worker)["state"] == "sleeping", rig.thread(worker))
    print(rig.tools.call("watch", {"id": daily, "then": {"worker": worker}}))
    shown, kept = len(rig.shown()), len(rig.chat())

    rig.due_now(daily)
    log = rig.root / "state/task" / worker / "log.jsonl"
    check("the named worker takes the routine", soon(lambda: "Your daily plan" in log.read_text(), 40), log.read_text())
    check("it ends Waiting for the person", soon(lambda: rig.thread(worker)["state"] == "waiting", 40), rig.thread(worker))
    last = json.loads(log.read_text().splitlines()[-1])
    check("the worker answered in its thread", last["kind"] == "talk", last)
    time.sleep(2)
    check("nothing new on the page", len(rig.shown()) == shown, rig.shown()[shown:])
    check("nothing new in the main chat", len(rig.chat()) == kept, rig.chat()[kept:])
    check("the main chat never hears of it", rig.count("talk", "Your daily plan") == 1, rig.chat())


def main():
    with tempfile.TemporaryDirectory() as tmp:
        rig = Rig(Path(tmp))
        try:
            rig.send("hello")
            until(lambda: rig.on_page("answered: hello"), "the first answer")
            for scenario in (routine_turn, quiet_routine, words_in_order, stop_word, routine_in_worker, scratchpad, two_clients):
                try:
                    scenario(rig)
                except TimeoutError as e:
                    check(scenario.__name__, False, f"timed out on {e}")
                if (rig.root / "release").exists():
                    (rig.root / "release").unlink()
        finally:
            log = (Path(tmp) / "libri.log").read_text()
            rig.close()
        if failures:
            print(log[-2000:])
    sys.exit(1 if failures else 0)


main()
