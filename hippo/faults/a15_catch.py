#!/usr/bin/env python3
"""Waits for a stuck post on a loaded host and saves every thread's stack."""
import os, subprocess, sys, threading, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from night import *
secs, mode = int(sys.argv[1]), sys.argv[2]
i = Instance("h-catch-" + mode)
i.start(); i.set_mode(mode); i.stub(); time.sleep(6)
GDB = "/nix/store/bcadwcq0i3hw6ilvshn5cyr97kfsdayz-gdb-17.2/bin/gdb"
inflight = [None]
slow = []
end = time.monotonic() + secs
def poster():
    k = 0
    while time.monotonic() < end:
        inflight[0] = time.monotonic()
        try:
            i.post(f"word {k} " + "x" * 300, wait=120)
        except Exception as e:
            print("post", k, repr(e)[:60], flush=True)
        d = time.monotonic() - inflight[0]
        inflight[0] = None
        if d > 2:
            slow.append(round(d, 1)); print("slow", round(d, 1), flush=True)
        k += 1
        time.sleep(1)
threading.Thread(target=poster, daemon=True).start()
n = 0
while time.monotonic() < end and n < 3:
    t = inflight[0]
    if t and time.monotonic() - t > 5:
        n += 1
        r = subprocess.run([GDB, "-p", str(i.proc.pid), "-batch", "-ex", "thread apply all bt 16"],
                           capture_output=True, text=True, timeout=200)
        open(NIGHT / f"logs/a15-stack-{mode}-{n}.log", "w").write(r.stdout + r.stderr)
        print("stack", n, flush=True)
        time.sleep(30)
    time.sleep(0.5)
print("done", slow, flush=True)
i.close()
