#!/usr/bin/env python3
"""Long outages of the summarizer, and a store ten times the size."""
import json, os, re, subprocess, sys, threading, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from night import *
from pathlib import Path


def sampler(i):
    """While hippo's threads sit, says where: names what holds up a post."""
    def run():
        while i.proc.poll() is None:
            states = {}
            for t in Path(f"/proc/{i.proc.pid}/task").glob("*"):
                try:
                    st = (t / "stat").read_text().rsplit(")", 1)[1].split()[0]
                    wchan = (t / "wchan").read_text().strip()
                    states[(st, wchan)] = states.get((st, wchan), 0) + 1
                except OSError:
                    pass
            open(i.root / "threads.log", "a").write(f"{time.monotonic():.1f} {states}\n")
            time.sleep(1)
    threading.Thread(target=run, daemon=True).start()


def h(i):
    r = json.loads(i.ask("health", wait=120)[1])
    return {k: r.get(k) for k in ("state", "retries_last_hour")} | {"err": (r.get("last_error") or {}).get("kind")}


def outage_view(n=30000):
    """Hours of chat with no summarizer: does the view stay inside a model's window?"""
    i = Instance("l-view")
    i.set_mode("model_error")
    i.start(); i.stub(); time.sleep(2)
    body = "a line of chat in a long outage, " * 15
    t0 = time.monotonic()
    for k in range(n):
        i.post(f"{k} {body}", wait=30)
        if k in (100, 1000, 5000, 10000, 20000, n - 1):
            t = time.monotonic()
            ok, v = i.ask("view", "whole", wait=60)
            print(f"after {k} msgs ({k*len(body)>>10} KB): view {len(v)} bytes in {time.monotonic()-t:.2f}s, last: {v.splitlines()[-1][:90]!r}", flush=True)
    print(h(i), "calls", i.calls(), round(time.monotonic() - t0), flush=True)
    i.close()


def outage(mode, minutes, seconds_after=420):
    i = Instance("l-" + mode)
    i.start(); i.stub(); time.sleep(4)
    sampler(i)
    print("start", h(i), i.calls(), flush=True)
    i.set_mode(mode)
    t0 = time.monotonic()
    k = 0
    while time.monotonic() - t0 < minutes * 60:
        t = time.monotonic()
        try:
            i.post(f"outage word {k} " + "x" * 300, wait=120)
        except Exception as e:
            print("POST FAILED", repr(e), round(time.monotonic() - t, 1), i.logs(2), flush=True)
        if time.monotonic() - t > 3:
            print("slow post", round(time.monotonic() - t, 1), flush=True)
        k += 1
        time.sleep(4)
        if k % 30 == 0:
            print(f"+{round(time.monotonic()-t0)}s", h(i), "calls", i.calls(), flush=True)
    i.set_mode("ok")
    c = i.calls()
    t0 = time.monotonic()
    while time.monotonic() - t0 < seconds_after:
        ok, st = i.ask("status", wait=120)
        s = [l for l in st.splitlines() if l.startswith("tree")][0]
        if re.search(r" 0 unsummar", s):
            break
        time.sleep(2)
    print(f"recovered in {round(time.monotonic()-t0)}s; {s[:110]}; calls in outage {c}, after {i.calls()-c}", h(i), i.logs(1), flush=True)
    i.close()


for a in sys.argv[1:]:
    print("==", a, flush=True)
    f, *r = a.split(":")
    try:
        globals()[f](*[int(x) if x.isdigit() else x for x in r])
    except Exception as e:
        print("DRIVER", repr(e), flush=True)
