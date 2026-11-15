#!/usr/bin/env python3
"""health and post latency while the compactor hangs."""
import sys, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from night import *
i = Instance("h-hang")
i.start(); i.set_mode("hang"); i.stub(); time.sleep(6)
for k in range(12):
    i.post(f"word {k} " + "x" * 700)
t0 = time.monotonic()
print("view", round((lambda t: (i.ask("view", "whole", wait=60), time.monotonic() - t)[1])(time.monotonic()), 1), flush=True)
worst = []
for k in range(3):
    i.ask("view", "whole", wait=60); i.post("another " + "x" * 700)
i.set_mode("ok")
print("mode ok at", round(time.monotonic() - t0), flush=True)
while time.monotonic() - t0 < 100:
    t = time.monotonic()
    try:
        i.ask("health", wait=30); a = round(time.monotonic() - t, 2)
    except Exception as e:
        a = repr(e)[:40]
    t = time.monotonic()
    try:
        i.post("more words", wait=30); b = round(time.monotonic() - t, 2)
    except Exception as e:
        b = repr(e)[:40]
    worst.append((round(time.monotonic() - t0), a, b))
    time.sleep(1)
print(sorted(worst, key=lambda w: -(w[1] if isinstance(w[1], float) else 99))[:4], flush=True)
i.close()
