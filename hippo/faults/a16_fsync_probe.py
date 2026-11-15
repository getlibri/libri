#!/usr/bin/env python3
"""Times a plain write and fdatasync beside a post to a real hippo, once a second,
to tell a stall of the disk from a stall inside hippo."""
import os, sys, threading, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from night import *
secs = int(sys.argv[1])
i = Instance("h-fsync"); i.start(); i.stub(); time.sleep(6)
f = os.open(NIGHT / "run/h-fsync/probe", os.O_WRONLY | os.O_CREAT | os.O_APPEND)
end = time.monotonic() + secs
n = [0, 0.0, 0.0]
def syncer():
    while time.monotonic() < end:
        t = time.monotonic(); os.write(f, b"x" * 300); os.fdatasync(f); d = time.monotonic() - t
        n[1] = max(n[1], d)
        if d > 1: print(time.strftime("%H:%M:%S"), "plain sync", round(d, 2), flush=True)
        time.sleep(1)
threading.Thread(target=syncer, daemon=True).start()
k = 0
while time.monotonic() < end:
    t = time.monotonic()
    try: ok, o = i.post(f"word {k}", wait=300)
    except Exception as e: print(time.strftime("%H:%M:%S"), "post failed", repr(e)[:50], flush=True)
    d = time.monotonic() - t; n[2] = max(n[2], d); n[0] += 1; k += 1
    if d > 1: print(time.strftime("%H:%M:%S"), "post", k, round(d, 2), open("/proc/loadavg").read()[:5], flush=True)
    time.sleep(1)
print("done", n, flush=True)
i.close()
