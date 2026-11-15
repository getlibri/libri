#!/usr/bin/env python3
"""The longer chat cases: the 'isn't answering' notice, catching_up, Libri's own
hippo dying, Libri restarted under a live hippo."""
import json, signal, sys, time, subprocess, os
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from rig import *

LONG = "a long message about nothing at all, " * 40


def mem(r):
    b = r.get("/api/memory", wait=8)
    try:
        m = json.loads(b)
        return {k: m.get(k) for k in ("state", "reason", "waiting", "since")}
    except Exception:
        return repr(b)[:100]


def children(r):
    out = []
    for e in Path("/proc").glob("[0-9]*/environ"):
        try:
            if (e.parent / "comm").read_text().strip() == "hippo" and f"HIPPO_DIR={r.dir}".encode() in e.read_bytes().split(b"\0"):
                out.append(int(e.parent.name))
        except OSError:
            pass
    return out


def isnt_answering():
    r = Rig("c2-answering", 9186)
    r.hippo(); r.up()
    print("before", r.send("hello")[1], mem(r), flush=True)
    time.sleep(1)
    print("heard", mem(r), flush=True)
    r.kill_hippo()
    t0 = time.monotonic()
    last = None
    while time.monotonic() - t0 < 215:
        m = mem(r)
        if m != last:
            print(f"  +{round(time.monotonic()-t0)}s {m}", flush=True)
            last = m
        time.sleep(5)
    print("turn now:", r.send("words after three minutes", wait=10)[2][-260:], flush=True)
    r.close()


def catching_up():
    r = Rig("c2-catching", 9187)
    r.hippo(); r.up()
    (r.root / "summarizer-down").write_text("")
    print("start", mem(r), flush=True)
    for k in range(14):
        x = r.send(f"{k} {LONG}", wait=10)
    time.sleep(1)
    print("after 14 long turns", mem(r), "last turn", x[0], flush=True)
    t0 = time.monotonic()
    last = None
    while time.monotonic() - t0 < 215:
        m = mem(r)
        if m != last:
            print(f"  +{round(time.monotonic()-t0)}s {m}", flush=True)
            last = m
        time.sleep(5)
    print("told in chat?", [l[:200] for l in r.ev if "emory" in l or "ummar" in l][-3:], flush=True)
    t, st, o = r.send("a turn during the outage", wait=45)
    print("turn during outage", t, flush=True)
    (r.root / "summarizer-down").unlink()
    t0 = time.monotonic()
    while time.monotonic() - t0 < 120:
        m = mem(r)
        if m != last:
            print(f"  recovering +{round(time.monotonic()-t0)}s {m}", flush=True)
            last = m
        if isinstance(m, dict) and m["state"] == "working" and not m["waiting"]:
            break
        time.sleep(2)
    print("calls", len(r.turns()), flush=True)
    print(r.hs("status").splitlines()[1:3], flush=True)
    r.close()


def lone_child():
    r = Rig("c2-lone", 9188)
    r.up(lone=True, extra={"HIPPO_AGENT": "Kit", "LIBRI_HIPPO_SERVE": "1"})
    time.sleep(1)
    print("hippo children", children(r), flush=True)
    print("turn", r.send("hello from kit side", wait=15)[0], len(r.turns()), flush=True)
    kids = children(r)
    raw = Path(f"/proc/{kids[0]}/environ").read_bytes() if kids else b""
    print("child HIPPO_DIR", [e for e in raw.split(b"\0") if e.startswith(b"HIPPO_DIR")], flush=True)
    assert kids and f"HIPPO_DIR={r.dir}".encode() in raw.split(b"\0"), "child is mine"
    os.kill(kids[0], signal.SIGKILL)
    time.sleep(2)
    print("after kill: children", children(r), flush=True)
    x = r.send("words after the child died", wait=12)
    print("turn", x[0], x[2][-260:], "model calls", len(r.turns()), flush=True)
    time.sleep(15)
    print("after 15s: children", children(r), "history", r.status("/api/history", wait=5), flush=True)
    print([l[:170] for l in (r.root / "libri.log").read_text(errors="replace").splitlines()[-4:]], flush=True)
    r.close()


def libri_restart():
    r = Rig("c2-restart", 9189)
    r.hippo(); r.up()
    for k in range(3):
        r.send(f"{k} {LONG}", wait=10)
    r.sig(signal.SIGKILL, r.d)
    r.d.wait()
    print("libri down; hippo health", r.hs("health")[:220], flush=True)
    time.sleep(10)
    r.hs("status")
    print(r.hs("status").splitlines()[1:], flush=True)
    r.up()
    x = r.send("words after libri came back", wait=25)
    print("turn", x[0], x[1], flush=True)
    time.sleep(8)
    print(r.hs("status").splitlines()[1:4], flush=True)
    print("compactor.sock", (r.dir / "compactor.sock").exists(), flush=True)
    r.close()


for n in sys.argv[1:]:
    try:
        print("==", n, flush=True)
        globals()[n]()
    except Exception as e:
        print("DRIVER", repr(e), flush=True)
