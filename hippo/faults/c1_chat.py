#!/usr/bin/env python3
"""The chat's side of each hippo fault, with the real daemon, a stand-in model
and a real hippo: does a turn run, are the words saved, what does the person
see, and what comes back."""
import json, signal, sys, time, subprocess
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from rig import *

which = sys.argv[1:] or ["healthy", "down", "stopped", "sock_gone", "restart_mid", "read_only"]


def report(name, r, words):
    print(f"-- {name}", flush=True)
    t, st, out = r
    print(f"   turn: {t}s status={st} said={out[-220:]!r}", flush=True)
    print(f"   model calls={len(r_.turns())} history={r_.status('/api/history', wait=8)} memory={(r_.get('/api/memory', wait=8) or b'')[:200]!r}", flush=True)


port = 9180
for name in which:
    r_ = Rig("c-" + name, port)
    port += 1
    try:
        assert r_.hippo(), "hippo up"
        assert r_.up(), "libri up"
        base = r_.send("first healthy words", wait=20)
        print(f"== {name}: baseline {base[0]}s {base[1]} {base[2][-120:]!r} calls={len(r_.turns())}", flush=True)
        if name == "healthy":
            report(name, r_.send("second words"), "second words")
        elif name == "down":
            r_.kill_hippo()
            report(name, r_.send("words while hippo is down", wait=30), "")
            r_.hippo()
            time.sleep(1)
            print("   after restart: saved?", r_.has("words while hippo is down"), flush=True)
            report(name + " recovered", r_.send("after hippo came back", wait=30), "")
            print("   saved?", r_.has("after hippo came back"), flush=True)
        elif name == "stopped":
            r_.sig(signal.SIGSTOP)
            report(name, r_.send("words while hippo is stopped", wait=25), "")
            t, st, o = r_.send("second message while stopped", wait=10)
            print(f"   second turn: {t}s {st} {o[-120:]!r}", flush=True)
            print(f"   page: history={r_.status('/api/history', wait=8)}", flush=True)
            r_.sig(signal.SIGCONT)
            time.sleep(3)
            print("   after CONT: saved?", r_.has("words while hippo is stopped"), r_.has("second message while stopped"), "calls", len(r_.turns()), flush=True)
        elif name == "sock_gone":
            (r_.dir / "hippo.sock").unlink()
            report(name, r_.send("words with the socket gone", wait=25), "")
            time.sleep(8)
            print("   socket back by itself?", (r_.dir / "hippo.sock").exists(), flush=True)
        elif name == "restart_mid":
            r_.kill_hippo()
            r_.hippo()
            report(name, r_.send("words after a restart", wait=25), "")
            print("   saved?", r_.has("words after a restart"), flush=True)
        elif name == "read_only":
            # A store that can't be written: the save fails, the view reads.
            day = sorted((r_.dir / "main").glob("*.jsonl"))[-1]
            day.chmod(0o444)
            r_.kill_hippo()
            r_.hippo()
            report(name, r_.send("words on a read-only store", wait=25), "")
            day.chmod(0o644)
        print("   libri log:", [l[:150] for l in (r_.root / "libri.log").read_text(errors="replace").splitlines()[-3:]], flush=True)
    except Exception as e:
        print(f"== {name}: DRIVER {e!r}", flush=True)
    finally:
        r_.close()
