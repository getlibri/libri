#!/usr/bin/env python3
"""A compactor that misbehaves: what a turn meets, what is saved, what returns."""
import json, subprocess, sys, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from night import *

BODY = "some words of a long message about nothing. " * 30


def health(i):
    return json.loads(i.ask("health", wait=10)[1])


def tree_tail(i, n=3):
    p = i.last_day("tree")
    out = []
    for l in p.read_text(errors="replace").splitlines()[-n:]:
        try:
            m = json.loads(l)
            out.append((m["l"], m["i"], m.get("size"), m.get("model"), m["text"][:60]))
        except Exception:
            out.append(("torn", l[:40]))
    return out


for mode in sys.argv[1:] or "ok slow hang refuse limit model_error garbage huge empty invented nonjson close".split():
    i = Instance("m-" + mode)
    i.start()
    i.set_mode(mode)
    i.stub()
    time.sleep(6)  # the startup hold on an absent socket lapses
    t = time.monotonic()
    rs = []
    for k in range(12):
        rs.append(i.post(f"user-word {mode}{k} {BODY}", wait=40)[0])
    post_s = round(time.monotonic() - t, 2)
    views = []
    for k in range(3):
        t = time.monotonic()
        try:
            ok, v = i.ask("view", "whole", wait=45)
            views.append((ok, round(time.monotonic() - t, 2), len(v)))
        except Exception as e:
            views.append((repr(e)[:50], round(time.monotonic() - t, 2), 0))
        i.post(f"another {k} {BODY}", wait=40)
    h = health(i)
    print(f"{mode:12} posts_all_ok={all(rs)} 12posts={post_s}s views={views} calls={i.calls()}", flush=True)
    print(f"    health state={h.get('state')} waiting={h.get('waiting')} last_error={(h.get('last_error') or {}).get('kind')} ", flush=True)
    print(f"    tree {tree_tail(i)}", flush=True)
    # Recovery: the compactor comes back.
    i.set_mode("ok")
    t = time.monotonic()
    end = t + 90
    state = None
    while time.monotonic() < end:
        h = health(i)
        state = h.get("state")
        ok, st = i.ask("status")
        line = [l for l in st.splitlines() if l.startswith("tree")][0]
        if " 0 unsummarized" in line:
            break
        i.post(f"nudge {BODY}") if time.monotonic() - t > 70 else None
        time.sleep(1)
    print(f"    recovery {round(time.monotonic()-t,1)}s state={state} :: {line[:110]} calls={i.calls()}", flush=True)
    v = i.ask("view", "whole", wait=45)[1]
    print(f"    view bytes={len(v)} log: {[l for l in i.logs(200) if 'retry' in l or 'failed' in l or 'refused' in l][-2:]}", flush=True)
    i.close()
    subprocess.run(["rm", "-rf", str(i.root)])
