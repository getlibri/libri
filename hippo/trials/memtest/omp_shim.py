"""OpenAI-style /chat/completions on 127.0.0.1 that runs each request on a
kept `omp --mode rpc` process, so hippo's stock `http` backend can summarize
through the omp harness (asked for 11-07: "use omp for the harness for
deepseek"). Hippo has no omp backend; this adapter exists only for the
memory test and goes when hippo gains one. The request's `model`,
`reasoning_effort` and system prompt pass through as omp's --model,
--thinking and --system-prompt, so idle processes wait per such triple, at
most IDLE of each; `new_session` empties one before its next request (it keeps the
system prompt), sparing omp's start on every call. Every call is appended
to LEDGER with omp's own token and cost figures."""
import json, os, subprocess, sys, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PORT = int(sys.argv[1])
LEDGER = sys.argv[2]
EMPTY = sys.argv[3]
IDLE = 8
TIMEOUT = 600
lock = threading.Lock()
idle = {}


def flatten(messages):
    """System prompt apart; the rest as one prompt, earlier turns labelled,
    the way a stateless retry resends what came before."""
    system = "\n\n".join(m["content"] if isinstance(m["content"], str) else
                           "".join(c.get("text", "") for c in m["content"])
                           for m in messages if m["role"] == "system")
    parts = []
    rest = [m for m in messages if m["role"] != "system"]
    for k, m in enumerate(rest):
        text = m["content"] if isinstance(m["content"], str) else "\n\n".join(c.get("text", "") for c in m["content"])
        if len(rest) > 1:
            text = f"[{'Your earlier reply' if m['role'] == 'assistant' else 'Request'}]\n{text}"
        parts.append(text)
    return system, "\n\n".join(parts)


class Omp:
    """One `omp --mode rpc`: newline JSON frames, a request answered by a
    `response` with its id, a prompt ended by `prompt_result`."""

    def __init__(self, key):
        model, effort, system = key
        args = ["nice", "-n", "10", "omp", "--mode", "rpc", "--no-session", "--no-tools", "--no-lsp",
                "--model", model, "--system-prompt", system]
        if effort:
            args += ["--thinking", effort]
        self.p = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True, cwd=EMPTY)
        self.n = 0

    def send(self, frame):
        self.n += 1
        frame["id"] = f"r{self.n}"
        self.p.stdin.write(json.dumps(frame) + "\n")
        self.p.stdin.flush()
        return frame["id"]

    def until(self, done):
        """Frames until `done` returns one; None if omp ends or takes over
        TIMEOUT (then it is killed)."""
        timer = threading.Timer(TIMEOUT, self.p.kill)
        timer.start()
        try:
            for line in self.p.stdout:
                try:
                    f = json.loads(line)
                except ValueError:
                    continue
                if done(f):
                    return f
            return None
        finally:
            timer.cancel()

    def ask(self, prompt):
        """The last agent_end of a prompt, or None."""
        i = self.send({"type": "prompt", "message": prompt})
        end = []

        def done(f):
            if f.get("type") == "agent_end":
                end[:] = [f]
            if f.get("type") == "response" and f.get("id") == i and not f.get("success"):
                return True
            return f.get("type") == "prompt_result" and f.get("id") == i

        return end[0] if self.until(done) and end else None

    def reset(self):
        i = self.send({"type": "new_session"})
        f = self.until(lambda f: f.get("type") == "response" and f.get("id") == i)
        return bool(f and f.get("success") and not f.get("data", {}).get("cancelled"))

    def close(self):
        self.p.kill()
        self.p.wait()


def take(key):
    """An idle process for `key`, its session emptied, else a new one. The
    reset comes here rather than on return so the next request finds the
    process idle instead of starting another beside it."""
    while True:
        with lock:
            o = idle.get(key, []) and idle[key].pop()
        if not o:
            return Omp(key)
        if o.p.poll() is None and o.reset():
            return o
        o.close()


def give(key, o):
    with lock:
        pool = idle.setdefault(key, [])
        if len(pool) < IDLE:
            return pool.append(o)
    o.close()


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["content-length"])))
        system, prompt = flatten(body["messages"])
        key = (body["model"], body.get("reasoning_effort"), system)
        o = take(key)
        try:
            end = o.ask(prompt)
        except OSError:
            end = None
        if not end:
            o.close()
            return self.reply(502, {"error": "omp gave no result"})
        try:
            self.answer(body, end)
        finally:
            give(key, o)

    def answer(self, body, end):
        m = end["messages"][-1]
        text = "".join(c.get("text", "") for c in m.get("content", []) if c.get("type") == "text")
        u = m.get("usage", {})
        rec = {"model": f"{m.get('provider')}/{m.get('model')}", "effort": body.get("reasoning_effort"),
               "in": u.get("input", 0), "cached": u.get("cacheRead", 0), "out": u.get("output", 0),
               "cost": u.get("cost", {}).get("total", 0), "stop": m.get("stopReason")}
        with lock, open(LEDGER, "a") as f:
            f.write(json.dumps(rec) + "\n")
        err = str(m.get("errorMessage", ""))
        if m.get("stopReason") == "error" or not text.strip():
            code = 429 if any(k in err.lower() for k in ("limit", "quota", "429", "insufficient")) else 502
            return self.reply(code, {"error": err[:400] or "empty reply"})
        self.reply(200, {"choices": [{"message": {"role": "assistant", "content": text},
                                      "finish_reason": "length" if m.get("stopReason") == "length" else "stop"}],
                         "usage": {"prompt_tokens": rec["in"] + rec["cached"], "completion_tokens": rec["out"],
                                   "prompt_tokens_details": {"cached_tokens": rec["cached"]}}})

    def reply(self, code, obj):
        raw = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)


ThreadingHTTPServer(("127.0.0.1", PORT), H).serve_forever()
