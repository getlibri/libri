#!/usr/bin/env python3
"""What one health call costs, with the store's real usage log, and what a post
meets while one runs."""
import sys, threading, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from night import *
i = Instance("h-health")
i.start(); i.stub(); time.sleep(3)
ts = []
for _ in range(6):
    t = time.monotonic(); i.ask("health", wait=60); ts.append(round(time.monotonic() - t, 2))
print("health", ts, flush=True)
t = threading.Thread(target=lambda: [i.ask("health", wait=60) for _ in range(5)])
t.start()
p = []
while t.is_alive():
    s = time.monotonic(); i.post("x", wait=60); p.append(round(time.monotonic() - s, 2))
print("posts during health", len(p), "max", max(p), flush=True)
i.close()
