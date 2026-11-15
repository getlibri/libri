#!/usr/bin/env python3
"""Stand-in for Libri's compactor.sock. The behaviour is read from a mode file
on every call so a test can change it while the instance runs.

Modes: ok, slow, hang, refuse, limit, model_error, garbage, huge, empty,
invented, nonjson, close.
"""
import json, os, socket, sys, threading, time

store, mode_file = sys.argv[1], sys.argv[2]
path = os.path.join(store, "compactor.sock")
assert path.startswith("/tmp/libri-night/hippo/"), path
if os.path.exists(path):
    os.unlink(path)
srv = socket.socket(socket.AF_UNIX)
srv.bind(path)
srv.listen(64)
count_lock = threading.Lock()
count = 0


def reply(conn):
    global count
    buf = b""
    while not buf.endswith(b"\n"):
        got = conn.recv(1 << 20)
        if not got:
            return
        buf += got
    with count_lock:
        count += 1
        with open(mode_file + ".calls", "w") as f:
            f.write(str(count))
    req = json.loads(buf)
    mode = open(mode_file).read().strip()
    # A leaf must open with its message's kind or compact.rs sends it back.
    prompt = req.get("prompt") or ""
    kind = "talk"
    for k in ("user", "talk", "tool", "echo", "work", "note"):
        if ("\n" + k + ": ") in prompt or prompt.startswith(k + ": "):
            kind = k
            break
    usage = {"input": 10, "output": 5}
    out = {"text": f"{kind}: stub summary {count}", "model": "stub", "usage": usage}
    if mode == "slow":
        time.sleep(5)
    elif mode == "hang":
        time.sleep(100000)
    elif mode == "refuse":
        out = {"error": "I'm sorry, but I can't help with that."}
    elif mode == "limit":
        out = {"error": "usage limit reached"}
    elif mode == "model_error":
        out = {"error": "the login is gone"}
    elif mode == "garbage":
        out["text"] = "\x00\xff<<<>>>" * 3
    elif mode == "huge":
        out["text"] = "talk: " + "x" * 2_000_000
    elif mode == "empty":
        out["text"] = ""
    elif mode == "invented":
        out["text"] = f"{kind}: Deployed and pushed, all tests passed and it is live."
    elif mode == "nonjson":
        conn.sendall(b"this is not json\n")
        return
    elif mode == "close":
        return
    conn.sendall((json.dumps(out) + "\n").encode())


def run(conn):
    try:
        reply(conn)
    except Exception as e:
        print("stub:", e, file=sys.stderr)
    finally:
        conn.close()


while True:
    c, _ = srv.accept()
    threading.Thread(target=run, args=(c,), daemon=True).start()
