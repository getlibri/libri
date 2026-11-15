#!/usr/bin/env python3
"""The attacks left over: stalled clients with the log kept, a large message with
no summarizer, two daemons on one store, a clock far ahead, wrong permissions,
id reuse after truncation, a duplicated id."""
import json, os, re, signal, socket, subprocess, sys, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from night import *
from pathlib import Path


def rm(i):
    subprocess.run(["chmod", "-R", "u+rwx", str(i.root)])
    subprocess.run(["rm", "-rf", str(i.root)])


def idle():
    """Under the 6 GB address limit: attributes the earlier refusal."""
    i = Instance("m-idle")
    i.start()
    i.stub()
    held = []
    for n in range(4000):
        s = socket.socket(socket.AF_UNIX)
        try:
            s.settimeout(5)
            s.connect(str(i.sock))
            s.sendall(b'{"cmd":"view"')
            held.append(s)
        except Exception as e:
            print("connect failed at", n, repr(e), flush=True)
            break
    time.sleep(1)
    print("idle clients", len(held), "alive", i.proc.poll(), flush=True)
    print("log", i.logs(4), flush=True)
    print("probe", probe(i), flush=True)
    time.sleep(12)
    print("probe past the read limit", i.proc.poll(), probe(i), flush=True)
    i.close()


def idle_free(count=12000):
    """Without the address limit, as the live service runs."""
    i = Instance("m-idle2", limit="unlimited")
    i.start()
    i.stub()
    held = []
    t0 = time.monotonic()
    for n in range(count):
        s = socket.socket(socket.AF_UNIX)
        try:
            s.settimeout(5)
            s.connect(str(i.sock))
            s.sendall(b'{"cmd":"view"')
            held.append(s)
        except Exception as e:
            print("connect failed at", n, repr(e), flush=True)
            break
    time.sleep(1)
    st = Path(f"/proc/{i.proc.pid}/status").read_text() if i.proc.poll() is None else ""
    print("idle clients", len(held), round(time.monotonic() - t0, 1), "exit", i.proc.poll(),
          re.findall(r"(?:VmRSS|Threads):\s+\d+", st), flush=True)
    print("probe", probe(i), i.logs(2), flush=True)
    [s.close() for s in held]
    time.sleep(2)
    print("after they go", probe(i) if i.proc.poll() is None else "dead", flush=True)
    i.close()
    rm(i)


def big_nosum(mb=1):
    """A large message while the summarizer refuses: the view takes it whole?"""
    i = Instance(f"m-nosum{mb}")
    i.set_mode("refuse")
    i.start()
    i.stub()
    time.sleep(3)
    print("post", i.post("z" * (mb << 20), wait=120)[1][:60], flush=True)
    time.sleep(1)
    ok, v = i.ask("view", "whole", wait=60)
    print(f"view with {mb} MB unsummarized: {len(v)} bytes", flush=True)
    print(json.loads(i.ask("health")[1]).get("state"), flush=True)
    i.close()
    rm(i)


def two_lock_deleted():
    """The lock file removed under a running hippo; a second starts on the store."""
    i = Instance("m-two")
    i.start()
    i.stub()
    os.link(i.sock, i.store / "first.sock")
    (i.store / "lock").unlink()
    second = i.spawn()
    time.sleep(4)
    print("second alive", second.poll() is None, "first alive", i.proc.poll() is None, i.logs(2), flush=True)
    first = Instance.__new__(Instance)
    first.__dict__ |= i.__dict__
    def ask_at(sock, *a):
        s = socket.socket(socket.AF_UNIX); s.settimeout(20); s.connect(str(sock))
        s.sendall((json.dumps({"cmd": a[0], "args": list(a[1:])}) + "\n").encode())
        b = b""
        while not b.endswith(b"\n"):
            g = s.recv(65536)
            if not g:
                break
            b += g
        return json.loads(b)
    for k in range(6):
        where = i.store / "first.sock" if k % 2 == 0 else i.sock
        print("  post", k, ask_at(where, "post", "Iris", "user", f"T{k}x words"), flush=True)
    day = i.last_day().read_bytes().splitlines()
    ids = [json.loads(l)["i"] for l in day[-6:]]
    print("last ids on disk", ids, flush=True)
    i.kill(second)
    i.kill(i.proc)
    time.sleep(0.3)
    print("restart", i.start(within=30), i.logs(1), flush=True)
    i.close()


def two_held():
    """A second start while the first is stopped, and while it runs."""
    i = Instance("m-held")
    i.start()
    i.signal(signal.SIGSTOP)
    t = time.monotonic()
    second = i.spawn()
    try:
        second.wait(15)
    except subprocess.TimeoutExpired:
        pass
    print("second with first stopped: exit", second.poll(), round(time.monotonic() - t, 1), i.logs(1), flush=True)
    i.kill(second)
    i.signal(signal.SIGCONT)
    print("first", probe(i)["post"], flush=True)
    i.close()
    rm(i)


def clock_ahead():
    """A last day file dated far ahead, as a clock gone to 2031 leaves it."""
    i = Instance("m-clock")
    p = i.last_day()
    day = p.stem
    q = p.with_name("2031-06-01.jsonl")
    ls = [json.loads(l) for l in p.read_text().splitlines() if l.strip()]
    for m in ls[-20:]:
        m["date"] = "2031-06-01" + m["date"][10:]
    p.write_text("".join(json.dumps(m, ensure_ascii=False, separators=(",", ":")) + "\n" for m in ls[:-20]))
    q.write_text("".join(json.dumps(m, ensure_ascii=False, separators=(",", ":")) + "\n" for m in ls[-20:]))
    print("start", i.start(within=30), i.logs(1), flush=True)
    i.stub()
    print("probe", probe(i), flush=True)
    print("newest files", sorted(x.name for x in (i.store / "main").iterdir())[-3:], flush=True)
    print("tail", [json.loads(l)["date"][:19] for l in i.tail(2).splitlines()], flush=True)
    time.sleep(3)
    ok, v = i.ask("view", "whole")
    print("view last", v.splitlines()[-1][:80], flush=True)
    print("restart ok", (i.kill(), time.sleep(0.3), i.start(within=30))[2], flush=True)
    i.close()
    rm(i)


def perms():
    i = Instance("m-perm")
    i.start()
    i.stub()
    time.sleep(1)
    for path, mode, name in ((i.store, 0o000, "store dir 000"), (i.store / "main", 0o500, "main read-only"),
                             (i.store / "tree", 0o500, "tree read-only"), (i.sock, 0o000, "socket 000"),
                             (i.store, 0o500, "store dir read-only")):
        path.chmod(mode)
        print(name, probe(i), flush=True)
        try:
            print("   health", {k: v for k, v in json.loads(i.ask("health")[1]).items() if k in ("state", "last_error")}, flush=True)
        except Exception as e:
            print("   health", repr(e)[:70], flush=True)
        path.chmod(0o700)
    print("after", probe(i), flush=True)
    i.kill(); time.sleep(0.3)
    (i.store / "main").chmod(0o000)
    print("start with main 000:", i.start(within=10), i.logs(1), flush=True)
    (i.store / "main").chmod(0o700)
    i.close()
    rm(i)


def id_reuse():
    """The last two-thirds of a day cut off: do new messages pair with old tree nodes?"""
    i = Instance("m-reuse")
    p = i.last_day()
    ls = p.read_bytes().split(b"\n")
    keep = len(ls) // 3
    before = json.loads(ls[keep + 5])
    p.write_bytes(b"\n".join(ls[:keep]) + b"\n")
    print("cut", len(ls), "to", keep, "next id", json.loads(ls[keep - 1])["i"] + 1, flush=True)
    print("start", i.start(within=30), i.logs(1), flush=True)
    i.stub()
    new = before["i"] - 5
    for k in range(10):
        i.post(f"AFTERCUT{k} fresh words")
    time.sleep(2)
    ok, st = i.ask("status")
    print([l[:100] for l in st.splitlines() if l.startswith("tree")], flush=True)
    ok, z = i.ask("zoom", str(new), "1")
    print("message at reused id", repr(z[:90]), flush=True)
    ok, z = i.ask("zoom", str(new & ~1), "2")
    print("zoom of level-1 span on it", repr(z[:300]), flush=True)
    ok, v = i.ask("view", "whole")
    print("view mentions the new words", "AFTERCUT" in v, "old words of the cut remain in summaries:", len(v), flush=True)
    i.close()
    rm(i)


def dup_id():
    i = Instance("m-dup")
    p = i.last_day()
    ls = p.read_bytes().split(b"\n")
    m = json.loads(ls[-3])
    m["i"] -= 1
    ls[-3] = json.dumps(m).encode()
    p.write_bytes(b"\n".join(ls))
    print("dup id: start", i.start(within=20), i.logs(1), flush=True)
    i.close()
    rm(i)


def order():
    """Two writers, each in strict order: is each one's order kept on disk?"""
    import threading
    i = Instance("m-order")
    i.start()
    i.stub()
    def w(n):
        for k in range(200):
            i.post(f"O{n}x{k}e")
    ts = [threading.Thread(target=w, args=(n,)) for n in range(8)]
    [t.start() for t in ts]
    [t.join() for t in ts]
    seen = {}
    bad = 0
    for l in i.last_day().read_text().splitlines():
        r = re.search(r"O(\d)x(\d+)e", json.loads(l)["text"])
        if r and json.loads(l)["text"].startswith("O"):
            n, k = int(r[1]), int(r[2])
            if seen.get(n, -1) >= k:
                bad += 1
            seen[n] = k
    print("each writer in order; out of order:", bad, "last seen", seen, flush=True)
    i.close()
    rm(i)


for a in sys.argv[1:]:
    print("==", a, flush=True)
    try:
        f, *r = a.split(":")
        globals()[f](*map(int, r))
    except Exception as e:
        print("DRIVER", a, repr(e), flush=True)
