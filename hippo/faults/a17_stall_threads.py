#!/usr/bin/env python3
"""Waits for a slow post and records where each hippo thread sleeps, by wchan,
which needs no ptrace. A sync on a copy of the day file runs beside it, to tell
that file's fsync from a plain new file's."""
import collections, os, shutil, subprocess, sys, threading, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from night import *
from pathlib import Path
secs = int(sys.argv[1])
i = Instance("h-threads"); i.start(); i.stub(); time.sleep(6)
copy = i.root / "day-copy"
subprocess.run(["cp", "--reflink=always", str(i.last_day()), str(copy)], check=True)
f = os.open(copy, os.O_WRONLY | os.O_APPEND)
end = time.monotonic() + secs
worst = {"sync": 0.0, "post": 0.0}
flight = [None]
def syncer():
    while time.monotonic() < end:
        t = time.monotonic(); os.write(f, b"x" * 300 + b"\n"); os.fdatasync(f); d = time.monotonic() - t
        worst["sync"] = max(worst["sync"], d)
        if d > 1: print(time.strftime("%H:%M:%S"), "copy sync", round(d, 2), flush=True)
        time.sleep(1)
def sampler():
    pid = i.proc.pid
    while time.monotonic() < end:
        t = flight[0]
        if t and time.monotonic() - t > 1.5:
            seen = collections.Counter()
            for p in Path(f"/proc/{pid}/task").iterdir():
                try:
                    seen[(p / "stat").read_text().rsplit(")", 1)[1].split()[0] + " " + (p / "wchan").read_text()] += 1
                except OSError:
                    pass
            print(time.strftime("%H:%M:%S"), "threads", round(time.monotonic() - t, 1), dict(seen), flush=True)
            time.sleep(1)
        time.sleep(0.2)
threading.Thread(target=syncer, daemon=True).start()
threading.Thread(target=sampler, daemon=True).start()
k = 0
while time.monotonic() < end:
    flight[0] = t = time.monotonic()
    try: i.post(f"word {k}", wait=300)
    except Exception as e: print("post failed", repr(e)[:50], flush=True)
    flight[0] = None
    d = time.monotonic() - t; worst["post"] = max(worst["post"], d); k += 1
    if d > 1: print(time.strftime("%H:%M:%S"), "post", k, round(d, 2), open("/proc/loadavg").read()[:5], flush=True)
    time.sleep(0.5)
print("done", k, worst, flush=True)
i.close()
