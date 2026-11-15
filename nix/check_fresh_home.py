#!/usr/bin/env python3
"""A fresh install, proved on real harnesses: Libri in a HOME that holds that
harness's login and nothing else, under a sandbox that hides the host's own
harness configuration.

For each harness: a real turn, a memory that reaches the model, a second
message that sees the first, its shell tool and Libri's own, the turn in hippo
word for word, a repository's instruction file honoured, no compaction limit
set by Libri, each layer of the guide once, and a worker that reads the memory.
The last counts what goes over the wire rather than what a model says: every
harness is pointed at a forwarding meter through its own base URL setting,
which writes each request down and passes it on. Claude Code alone goes through
it as a proxy that terminates TLS: a base URL makes it send requests of another
shape, one more cache mark among them. Then a server of the person's
own, named `browser`, is added to the harness's own config, and a turn uses it
beside Libri's; on omp the server has another name, as omp drops any server
called `browser` in favour of its own browser tool. Last an over-long turn compacts natively, at a point lowered in
the harness's own setting so that it costs tens of thousands of tokens rather
than a whole window, and the next message starts clean.

Costs real model calls, so it stays out of `nix flake check`. Scratch state
only, on free ports; no session of a harness is kept. Logins come from the
account running this and never leave the scratch HOME, which is deleted:

    claude    CLAUDE_CODE_OAUTH_TOKEN, else the access token of
              ~/.claude/.credentials.json, which cannot refresh from here
    codex     a copy of ~/.codex/auth.json
    opencode  DEEPSEEK_API_KEY, else the file DEEPSEEK_KEY_FILE names
    omp       the same

Needs openssl on the PATH, for the certificate of Claude Code's meter.

Usage: check_fresh_home.py LIBRI_DIR [--only a,b] [--guide] [--base-url] [--keep]

--guide stops after the guide's own turns, which spend least: an empty guide,
then each layer of it, then an edit in Settings with no restart.

--base-url sends Claude Code to its meter by ANTHROPIC_BASE_URL rather than as
an HTTPS proxy. It spends one more cache mark that way, which Libri allows for.
"""

import contextlib
import http.client
import http.server
import json
import os
import random
import shutil
import signal
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

LIBRI_DIR = Path(sys.argv[1]).resolve()
LIBRI, HIPPO = LIBRI_DIR / "bin/libri", LIBRI_DIR / "bin/hippo"
ONLY = sys.argv[sys.argv.index("--only") + 1].split(",") if "--only" in sys.argv else ["claude", "codex", "opencode", "omp"]
KEEP = "--keep" in sys.argv
GUIDE_ONLY = "--guide" in sys.argv
BASE_URL = "--base-url" in sys.argv
# A sentence of the product's own rules (libri/src/guide.rs), on every install.
PRODUCT = "Everything said here is recorded word for word on its own."
MODELS = {
    "claude": "haiku",
    "codex": "gpt-6-luna",
    "opencode": "deepseek",
    "omp": "deepseek",
}
UPSTREAM = {
    "claude": "api.anthropic.com",
    "codex": "chatgpt.com",
    "opencode": "api.deepseek.com",
    "omp": "api.deepseek.com",
}
OWN_SERVER = {"claude": "browser", "codex": "browser", "opencode": "browser", "omp": "person"}
TURN = {"claude": 150, "codex": 150, "opencode": 200, "omp": 300}
# What the host holds of each harness, or of Libri, that a fresh machine does not.
# CHECK_HIDE adds a host's own secret and state paths, colon-separated.
HIDDEN = ["/etc/codex", "/etc/claude-code", "/var/lib/libri", "/home"] + [
    p for p in os.environ.get("CHECK_HIDE", "").split(":") if p
]
HOP = {"connection", "keep-alive", "transfer-encoding", "te", "upgrade", "proxy-authorization", "host", "content-length"}
SERVER = """
import json, sys
for line in sys.stdin:
    m = json.loads(line)
    r = None
    if m.get("method") == "initialize":
        r = {"protocolVersion": m["params"].get("protocolVersion", "2025-03-26"),
             "capabilities": {"tools": {}}, "serverInfo": {"name": "person", "version": "1"}}
    elif m.get("method") == "tools/list":
        r = {"tools": [{"name": "person_word", "description": "The person's word.",
                        "inputSchema": {"type": "object", "properties": {}}}]}
    elif m.get("method") == "tools/call":
        r = {"content": [{"type": "text", "text": "PERSONWORD"}]}
    if "id" in m:
        print(json.dumps({"jsonrpc": "2.0", "id": m["id"], "result": r}), flush=True)
"""
table = {}


def note(h, what, ok, detail=""):
    table.setdefault(h, {})[what] = bool(ok)
    print(f"{'ok  ' if ok else 'FAIL'} {h}: {what} {str(detail)[:400] if not ok else ''}", flush=True)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def until(test, what, within=60):
    end = time.monotonic() + within
    while time.monotonic() < end:
        got = test()
        if got:
            return got
        time.sleep(0.5)
    raise TimeoutError(what)


def certificate(root, host):
    """A throwaway certificate for `host` that is its own authority."""
    pem = root / "meter.pem"
    subprocess.run(
        ["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-subj", f"/CN={host}",
         "-addext", f"subjectAltName=DNS:{host}", "-addext", "basicConstraints=critical,CA:TRUE",
         "-keyout", pem, "-out", pem],
        check=True,
        capture_output=True,
    )
    return pem


class Meters(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def handle_error(self, *_):
        # A harness hanging up mid-stream is no finding.
        pass


def meter(upstream, log=None, pem=None):
    """A forwarding proxy to `upstream` that keeps each request's body, also in
    the file `log` if named. With `pem` it also answers as an HTTPS proxy for
    `upstream`, and refuses every other host."""
    bodies = []

    class Meter(http.server.BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_):
            pass

        def do_CONNECT(self):
            if self.path.split(":")[0] != upstream:
                self.send_error(502)
                return
            self.send_response(200)
            self.end_headers()
            self.wfile.flush()
            tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            tls.load_cert_chain(pem)
            self.connection = tls.wrap_socket(self.connection, server_side=True)
            self.rfile = self.connection.makefile("rb")
            self.wfile = self.connection.makefile("wb")

        def do_GET(self):
            # A WebSocket upgrade: refused, so the harness falls back on HTTP.
            if self.headers.get("upgrade"):
                self.send_response(404)
                self.send_header("content-length", "0")
                self.end_headers()
                return
            self.do_POST()

        def do_POST(self):
            body = self.rfile.read(int(self.headers.get("content-length", 0)))
            bodies.append(body.decode(errors="replace"))
            if log:
                with open(log, "a") as f:
                    f.write(json.dumps({"path": self.path, "body": bodies[-1]}) + "\n")
            connection = http.client.HTTPSConnection(upstream, context=ssl.create_default_context(), timeout=300)
            headers = {k: v for k, v in self.headers.items() if k.lower() not in HOP}
            connection.request(self.command, self.path, body, headers)
            reply = connection.getresponse()
            self.send_response(reply.status)
            for k, v in reply.getheaders():
                if k.lower() not in HOP:
                    self.send_header(k, v)
            self.send_header("transfer-encoding", "chunked")
            self.end_headers()
            while chunk := reply.read1(65536):
                self.wfile.write(f"{len(chunk):x}\r\n".encode() + chunk + b"\r\n")
                self.wfile.flush()
            self.wfile.write(b"0\r\n\r\n")
            connection.close()

    server = Meters(("127.0.0.1", 0), Meter)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, bodies


def deepseek_key():
    key = os.environ.get("DEEPSEEK_API_KEY")
    return key or Path(os.environ["DEEPSEEK_KEY_FILE"]).read_text().strip()


def login(h, home, port, pem):
    """The harness's login in `home`, and its meter: what stays of it is one
    address. Returns the variables the login and the meter need."""
    base = f"http://127.0.0.1:{port}"
    if h == "claude":
        token = os.environ.get("CLAUDE_CODE_OAUTH_TOKEN")
        if not token:
            token = json.loads(Path("~/.claude/.credentials.json").expanduser().read_text())["claudeAiOauth"]["accessToken"]
        route = {"ANTHROPIC_BASE_URL": base} if BASE_URL else {"HTTPS_PROXY": base, "NO_PROXY": "127.0.0.1,localhost", "NODE_EXTRA_CA_CERTS": str(pem)}
        return {"CLAUDE_CODE_OAUTH_TOKEN": token, **route}
    if h == "codex":
        (home / ".codex").mkdir()
        shutil.copy(Path("~/.codex/auth.json").expanduser(), home / ".codex/auth.json")
        (home / ".codex/auth.json").chmod(0o600)
        # Uncompressed, so the meter can read what it forwards.
        write(home / ".codex/config.toml", f'openai_base_url = "{base}/backend-api/codex"\n[features]\nenable_request_compression = false\n')
        return {}
    if h == "opencode":
        write(home / ".config/opencode/opencode.json", json.dumps({"provider": {"deepseek": {"options": {"baseURL": f"{base}/v1"}}}}))
    else:
        write(home / ".omp/agent/models.yml", json.dumps({"providers": {"deepseek": {"baseUrl": f"{base}/v1"}}}))
    return {"DEEPSEEK_API_KEY": deepseek_key()}


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def own_server(h, home, script):
    """The person's server, named `browser`, in the harness's own config."""
    command = [sys.executable, "-I", str(script)]
    name = OWN_SERVER[h]
    if h == "claude":
        write(home / ".claude.json", json.dumps({"mcpServers": {name: {"type": "stdio", "command": command[0], "args": command[1:]}}}))
    elif h == "codex":
        with (home / ".codex/config.toml").open("a") as f:
            f.write(f'\n[mcp_servers.{name}]\ncommand = "{command[0]}"\nargs = {json.dumps(command[1:])}\ndefault_tools_approval_mode = "approve"\n')
    elif h == "opencode":
        path = home / ".config/opencode/opencode.json"
        config = json.loads(path.read_text())
        config["mcp"] = {name: {"type": "local", "command": command}}
        write(path, json.dumps(config))
    else:
        write(home / ".omp/agent/mcp.json", json.dumps({"mcpServers": {name: {"type": "stdio", "command": command[0], "args": command[1:]}}}))


class Install:
    """One Libri in a scratch HOME, in a sandbox holding none of the host's
    harness configuration."""

    def __init__(self, h, root):
        self.h, self.root = h, root
        self.home, self.work, self.state = root / "home", root / "work", root / "state"
        self.home.mkdir()
        self.work.mkdir()
        self.state.mkdir()
        self.guide_mark, self.repo_mark = f"GUIDEMARK{os.getpid()}{h}", f"REPOWORD{h}{os.getpid()}"
        self.rules_mark = f"RULESMARK{os.getpid()}{h}"
        self.instructions = f"When asked for the repository word, answer {self.repo_mark} and nothing else.\n"
        write(self.work / ("CLAUDE.md" if h == "claude" else "AGENTS.md"), self.instructions)
        subprocess.run(["git", "init", "-q", str(self.work)], check=True)
        subprocess.run(["git", "-C", str(self.work), "add", "."], check=True)
        subprocess.run(
            ["git", "-C", str(self.work), "-c", "user.name=probe", "-c", "user.email=probe@example.com", "commit", "-qm", "start"],
            check=True,
        )
        pem = certificate(root, UPSTREAM[h]) if h == "claude" else None
        self.meter, self.bodies = meter(UPSTREAM[h], root / "requests.jsonl" if KEEP else None, pem)
        # The harness by name: an installer's link may lead to a versioned file.
        (root / "bin").mkdir()
        (root / "bin" / h).symlink_to(Path(shutil.which(h)).resolve())
        self.env = {
            "PATH": ":".join(dict.fromkeys([str(LIBRI_DIR / "bin"), str(root / "bin"), *(str(Path(shutil.which(b)).resolve().parent) for b in ("git", "bash", "env", "grep", "cat"))])),
            "HOME": str(self.home),
            "LANG": "C.UTF-8",
            "TZ": "UTC",
            "STATE_DIRECTORY": str(self.state),
            "LIBRI_NAME": "Libri",
            "LIBRI_PERSON": "Ada",
            "LIBRI_HARNESSES": h,
            "LIBRI_BACKEND": h,
            "LIBRI_WORKDIR": str(self.work),
            "LIBRI_INSTRUCTIONS": str(root / "guide.md"),
            "LIBRI_OPEN": "1",
        } | login(h, self.home, self.meter.server_address[1], pem)
        self.daemon = None
        self.start()

    def start(self):
        self.stop()
        self.port = free_port()
        env = self.env | {
            "LIBRI_SITE": f"127.0.0.1:{self.port}",
            "LIBRI_SITE_URL": f"http://127.0.0.1:{self.port}",
            "LIBRI_SITE_ROOT": str(LIBRI_DIR / "site"),
        }
        hide = [a for path in HIDDEN if Path(path).exists() for a in ("--tmpfs", path)]
        # A machine with no bwrap is a new one, with nothing of the kind to hide.
        bwrap = shutil.which("bwrap")
        sandbox = [bwrap, "--dev-bind", "/", "/", *hide, "--chdir", str(self.root)] if bwrap else []
        self.daemon = subprocess.Popen(
            [*sandbox, str(LIBRI)],
            env=env,
            cwd=self.root,
            stdout=subprocess.DEVNULL,
            stderr=open(self.root / "libri.log", "a"),
            start_new_session=True,
        )
        until(lambda: self.get("/api/host"), "libri up", 60)
        if self.h in ("opencode", "omp"):
            self.post("/api/others", {"others": self.h})
        self.post("/api/model", self.offer())
        # Auto would stop each shell call on a card with no one to answer it.
        self.post("/api/access", {"access": "full"})

    def offer(self):
        """The cheap model, by the name this harness's own list gives it."""
        mine = [o for o in json.loads(self.get("/api/models"))["offers"] if o["harness"] == self.h]
        chosen = next((o for o in mine if MODELS[self.h] in o["model"]), None)
        if not chosen:
            raise RuntimeError(f"no {MODELS[self.h]} on offer: {mine}")
        return {"harness": self.h, "model": chosen["model"]}

    def stop(self):
        if self.daemon:
            os.killpg(self.daemon.pid, signal.SIGTERM)
            self.daemon.wait()
            self.daemon = None

    def close(self):
        self.stop()
        self.meter.shutdown()

    def request(self, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.port}{path}",
            data=data,
            headers={"content-type": "application/json", "origin": f"http://127.0.0.1:{self.port}"},
        )
        return urllib.request.urlopen(request, timeout=90)

    def get(self, path):
        try:
            with self.request(path) as r:
                return r.read()
        except (OSError, urllib.error.URLError):
            return None

    def post(self, path, body):
        with self.request(path, body) as r:
            return r.read()

    def chat(self):
        env = {"HIPPO_DIR": str(self.state / "hippo"), "PATH": os.environ["PATH"]}
        out = subprocess.run([HIPPO, "tail", "1000"], env=env, capture_output=True, text=True, timeout=30).stdout
        return [json.loads(l) for l in out.splitlines() if l.startswith("{")]

    def idle(self):
        task = self.get("/api/task")
        return task is not None and not json.loads(task)["typing"]

    def say(self, text, marker=None, within=None):
        """One turn: the words after the person's message, once it ended."""
        marker = marker or text
        self.post("/api/run", {"messages": [{"id": f"m{time.monotonic_ns()}", "role": "user", "content": text}]})
        until(lambda: any(m["kind"] == "user" and marker in m["text"] for m in self.chat()), f"{self.h} took {marker}", 45)
        end = time.monotonic() + (within or TURN[self.h])
        while time.monotonic() < end:
            time.sleep(2)
            if self.idle():
                time.sleep(3)
                if self.idle():
                    break
        chat = self.chat()
        at = next(i for i, m in enumerate(chat) if m["kind"] == "user" and marker in m["text"])
        return chat[at + 1 :]


def words(after):
    return "\n".join(m["text"] for m in after if m["kind"] == "talk")


def called(after, name):
    return [m for m in after if m["kind"] == "tool" and name in m["text"]]


def lower_compaction(h, run):
    """The harness's own compaction point, set low in its own setting."""
    if h == "claude":
        run.env["CLAUDE_CODE_AUTO_COMPACT_WINDOW"] = "45000"
    elif h == "codex":
        with (run.home / ".codex/config.toml").open("r+") as f:
            settings = f.read()
            f.seek(0)
            f.write(f"model_auto_compact_token_limit = 21000\n{settings}")
    elif h == "opencode":
        path = run.home / ".config/opencode/opencode.json"
        config = json.loads(path.read_text())
        model = run.offer()["model"].split("/", 1)[1]
        config["provider"]["deepseek"]["models"] = {model: {"limit": {"context": 45000, "output": 4000}}}
        write(path, json.dumps(config))
    else:
        write(run.home / ".omp/agent/config.yml", json.dumps({"compaction": {"thresholdTokens": 15000}}))


def long_turn(h, run, planted):
    lower_compaction(h, run)
    run.start()
    names = [f"bulk_{i}.txt" for i in range(20)]
    ends = []
    for name in names:
        bulk = [f"{random.getrandbits(32):08x}" for _ in range(2600)]
        ends += [bulk[0], bulk[-1]]
        write(run.work / name, " ".join(bulk))
    seen = len(run.bodies)
    marker = f"long{time.monotonic_ns()}"
    after = run.say(
        # A model left to itself may discard the output and fill nothing.
        f"({marker}) Run plain cat on each of these files, with no redirect or pipe so the whole of each comes back to you, one at a time: one call in a step, then wait for its result before the next, never two at once, in this order, whatever else happens: "
        f"{' '.join(names)}. Then reply with the word FINISHED{marker} alone.",
        marker,
        900,
    )
    # The model may come out of a compaction with the word and not its suffix.
    note(h, "an over-long turn ends and lands in hippo", "FINISHED" in words(after), words(after)[-300:])
    turn = [b for b in run.bodies[seen:] if run.guide_mark in b]
    sizes = [len(b) for b in turn]
    print(f"     {h} request sizes in the long turn: {sizes}", flush=True)
    # Libri's memory calls quote the turn too, and a small one may land last.
    reach = [len(b) for b in turn if "You write Libri's memory" not in b]
    # Harnesses clip a tool's output, each its own way, so size alone says
    # little: what the model once saw is gone from what it sees at the end,
    # or the conversation it is sent has collapsed.
    gone = [w for w in ends if any(w in b for b in turn) and w not in turn[-1]]
    requests = [json.loads(b) for b in run.bodies[seen:] if b.startswith("{")]
    lengths = [len(r.get("messages") or r.get("input") or []) for r in requests if r.get("tools")]
    fell = any(b < a / 2 for a, b in zip(lengths, lengths[1:]))
    # On Anthropic's own address Claude Code keeps the conversation in a thread
    # on the server and sends only what is new, until a compaction starts another.
    threads = sum(r.get("thread", {}).get("type") == "create" for r in requests if r.get("tools"))
    note(h, "the turn compacted natively", gone or fell or threads > 1, f"{len(gone)} words gone, lengths {lengths}, sizes {sizes}, threads {threads}")
    marker = f"after{time.monotonic_ns()}"
    after = run.say(f"({marker}) What does the pier gauge read? Use no tools. Answer with the whole reading, every digit of it, alone.", marker)
    note(h, "the next message starts clean", planted in words(after), words(after))
    fresh = [len(b) for b in run.bodies if marker in b and run.guide_mark in b]
    note(h, "and carries no bulk of the long turn", fresh and all(n < reach[-1] * 2 for n in fresh) and not any(marker in b and "bulk_3" in b.split(marker)[-1] for b in run.bodies), fresh)


def worker(h, run, planted):
    marker = f"worker{time.monotonic_ns()}"
    after = run.say(
        # Codex lists MCP tools only when asked, so the model must be told to look.
        f"({marker}) Libri's server has a tool named spawn, though your tool list may leave it out: look it up if you have a way to, and use it, not any agent tool of your own, to start one worker with this task: call memory_search with the regex pier gauge, then report the reading.",
        marker,
    )
    note(h, "a worker starts", called(after, "spawn"), after)
    try:
        until(lambda: any(m["kind"] == "work" and planted in m["text"] for m in run.chat()), "the worker's report", TURN[h])
        note(h, "the worker reads the memory and reports", True)
    except TimeoutError:
        note(h, "the worker reads the memory and reports", False, [m for m in run.chat() if m["kind"] == "work"][-2:])
    until(run.idle, "the turn after the report", TURN[h])


def delivered(body):
    """A request less what the model wrote itself. DeepSeek's reasoning may
    quote its rules, and OpenCode sends that back in the next request, which
    counts as a second copy though nothing delivered it twice."""
    try:
        request = json.loads(body)
    except ValueError:
        return body
    for key in ("messages", "input"):
        if isinstance(request.get(key), list):
            request[key] = [m for m in request[key] if m.get("role") != "assistant" and m.get("type") != "reasoning"]
    return json.dumps(request, ensure_ascii=False)


def second_copy(bodies, mark):
    """What leads up to a second copy of `mark`, for a count that is not one."""
    for b in bodies:
        if b.count(mark) > 1:
            at = b.index(mark, b.index(mark) + 1)
            return b[max(0, at - 300) : at + len(mark)]
    return ""


def fill_guide(run):
    """The host's layer as a deployment writes it, the person's through Settings."""
    write(run.root / "guide.md", f"Standing rule {run.guide_mark}.\n")
    run.post("/api/guide", {"text": f"Rule of the person {run.rules_mark}.\n"})


def guide_layers(h, run):
    """What the harness is sent of the guide, turn by turn, on one daemon."""
    sent = lambda marker: [delivered(b) for b in run.bodies if marker in b]
    count = lambda bodies, mark: sorted({b.count(mark) for b in bodies})

    marker = f"empty{time.monotonic_ns()}"
    after = run.say(f"({marker}) Reply with the word READY alone.", marker)
    note(h, "an empty guide in an empty HOME runs a turn", "READY" in words(after), after or (run.root / "libri.log").read_text()[-400:])
    note(h, "with no guide the product's rules arrive once", count(sent(marker), PRODUCT) == [1], count(sent(marker), PRODUCT))

    fill_guide(run)
    marker = f"layers{time.monotonic_ns()}"
    after = run.say(f"({marker}) Reply with the word READY alone.", marker)
    bodies = sent(marker)
    shown = {m: count(bodies, m) for m in (PRODUCT, run.rules_mark, run.guide_mark)}
    note(h, "each layer arrives once in every request", bodies and all(c == [1] for c in shown.values()), shown)

    edited = f"EDITED{run.rules_mark}"
    run.post("/api/guide", {"text": f"Rule of the person {edited}.\n"})
    marker = f"edit{time.monotonic_ns()}"
    after = run.say(f"({marker}) Reply with the word READY alone.", marker)
    bodies = sent(marker)
    note(h, "an edit in Settings reaches the next turn, no restart", bodies and count(bodies, edited) == [1], count(bodies, edited))
    # The text is the one mark inside the other, so the old one is its own count.
    old = max((b.count(run.rules_mark) - b.count(edited) for b in bodies), default=0)
    note(h, "and the text it replaced is gone", old == 0, old)
    print(f"     {h} guide: {len(run.bodies)} requests so far; layers {shown}", flush=True)


@contextlib.contextmanager
def fresh(h):
    """An install in an empty HOME, closed and removed when the block ends."""
    root = Path(tempfile.mkdtemp(prefix=f"libri-fresh-{h}-", dir="/var/tmp"))
    run = Install(h, root)
    try:
        seed = sorted(p.relative_to(run.home).as_posix() for p in run.home.rglob("*") if p.is_file())
        print(f"     {h} HOME holds: {seed}", flush=True)
        yield run
    finally:
        run.close()
        log = (root / "libri.log").read_text(errors="replace")
        print(f"     {h} log tail: {log[-300:]!r}", flush=True)
        if KEEP:
            print(f"     {h} kept {root}", flush=True)
        else:
            shutil.rmtree(root, ignore_errors=True)


def proof(h):
    # On an install of their own, so the turns below start with an empty memory.
    with fresh(h) as run:
        guide_layers(h, run)
    if GUIDE_ONLY:
        return

    with fresh(h) as run:
        root = run.root
        fill_guide(run)
        word = f"heron{h}{os.getpid()}"
        sentence = f"The tide table is kept at {word}."
        first = f"Reply with exactly this sentence and nothing else: {sentence}"
        after = run.say(first)
        note(h, "a real turn answers", word in words(after), after or (root / "libri.log").read_text()[-400:])
        chat = run.chat()
        note(h, "the person's words land word for word", sum(m["kind"] == "user" and m["text"] == first for m in chat) == 1, chat[-4:])
        note(h, "the answer lands word for word", any(m["kind"] == "talk" and m["text"].strip() == sentence for m in chat), chat[-4:])

        marker = f"second{time.monotonic_ns()}"
        after = run.say(f"({marker}) Where is the tide table kept? Use no tools. Answer with that whole word, every digit of it, alone.", marker)
        note(h, "a second message sees the first", word in words(after), words(after))

        # In the memory only: no harness session of Libri's ever held it.
        planted = f"egret{h}{os.getpid()}"
        run.post("/api/call/end", {"lines": [{"kind": "user", "text": f"The pier gauge reads {planted}."}], "minutes": 2})
        marker = f"memory{time.monotonic_ns()}"
        after = run.say(f"({marker}) What does the pier gauge read? Use no tools. Answer with the whole reading, every digit of it, alone.", marker)
        note(h, "the memory reaches the model", planted in words(after), words(after))

        marker = f"shell{time.monotonic_ns()}"
        # What Libri used to set on a harness shows in the shell it runs: the
        # 200k compaction limit, the instruction-file switch, the hook guard.
        command = f"echo {marker}x $(printenv | grep -c -E 'AUTO_COMPACT|DISABLE_CLAUDE_MDS|LIBRI_RUNNING')"
        after = run.say(f"Run this shell command with your tool, then reply with its output alone: {command}", marker)
        note(h, "a shell tool works", any(m["kind"] == "tool" for m in after) and f"{marker}x" in words(after), after)
        note(h, "Libri sets no compaction limit or switch", f"{marker}x 0" in words(after), words(after))
        note(h, "a tool call lands in hippo", any(m["kind"] == "tool" and marker in m["text"] for m in run.chat()), after)

        marker = f"search{time.monotonic_ns()}"
        after = run.say(f"({marker}) Call the memory_search tool with the regex pier gauge, and answer with the reading it finds.", marker)
        note(h, "Libri's own tool works", called(after, "memory_search") and planted in words(after), after)

        marker = f"repo{time.monotonic_ns()}"
        after = run.say(f"({marker}) What is the repository word? Use no tools.", marker)
        note(h, "the repository's instruction file is honoured", run.repo_mark in words(after), words(after))

        turns = [delivered(b) for b in run.bodies if run.guide_mark in b]
        note(h, "the meter saw the turns", len(turns) >= 6, f"{len(run.bodies)} requests, {len(turns)} with the guide")
        counts = sorted({b.count(run.guide_mark) for b in turns})
        note(h, "the guide arrives once in every request", counts == [1], (counts, second_copy(turns, run.guide_mark)))
        layers = {m: sorted({b.count(m) for b in turns}) for m in (PRODUCT, run.rules_mark)}
        note(h, "so does each other layer", all(c == [1] for c in layers.values()), (layers, second_copy(turns, run.rules_mark)))
        # In a turn's first request: a model may read the file itself later on.
        # OpenCode's title request quotes the prompt and may go first; it is
        # no turn, and takes no instruction file.
        repo = next(b for b in run.bodies if marker in b and run.guide_mark in b and "You are a title generator" not in b).count(run.instructions.strip())
        note(h, "the instruction file arrives once", repo == 1, repo)
        print(f"     {h} requests: {len(run.bodies)}; with the guide: {len(turns)}; guide counts {counts}; instruction count {repo}", flush=True)
        extra = subprocess.run(["git", "-C", str(run.work), "status", "--porcelain"], capture_output=True, text=True).stdout.split("\n")
        print(f"     {h} left in the repository: {[l for l in extra if l]}", flush=True)

        worker(h, run, planted)

        script = root / "person_server.py"
        script.write_text(SERVER)
        own_server(h, run.home, script)
        run.start()
        marker = f"own{time.monotonic_ns()}"
        after = run.say(f"({marker}) Call the person_word tool of the {OWN_SERVER[h]} server, then reply with what it returned.", marker)
        note(h, "the person's own server runs beside Libri's", "PERSONWORD" in words(after), after or (root / "libri.log").read_text()[-400:])
        marker = f"beside{time.monotonic_ns()}"
        after = run.say(f"({marker}) Call the memory_search tool with the regex pier gauge, and answer with the reading it finds.", marker)
        note(h, "and Libri's tools still work", called(after, "memory_search") and planted in words(after), after)

        long_turn(h, run, planted)


def main():
    for h in ONLY:
        try:
            proof(h)
        except Exception as e:
            note(h, "the proof ran to its end", False, repr(e))
    width = max((len(w) for checks in table.values() for w in checks), default=0)
    print()
    for h, checks in table.items():
        print(h)
        for what, ok in checks.items():
            print(f"  {what:<{width}} {'ok' if ok else 'FAIL'}")
    sys.exit(0 if all(ok for checks in table.values() for ok in checks.values()) else 1)


main()
