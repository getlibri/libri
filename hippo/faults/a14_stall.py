#!/usr/bin/env python3
"""Catches a stuck post with the compactor hung: dumps each thread's stack by gdb."""
import subprocess, sys, threading, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from night import *
i = Instance("h-stall")
i.start(); i.set_mode("hang"); i.stub(); time.sleep(6)
done = threading.Event()
def poster():
    for k in range(12):
        t = time.monotonic()
        try:
            i.post(f"word {k} " + "x" * 700, wait=25)
        except Exception as e:
            print("post", k, repr(e)[:50], round(time.monotonic() - t, 1), flush=True)
        else:
            print("post", k, round(time.monotonic() - t, 2), flush=True)
    done.set()
threading.Thread(target=poster, daemon=True).start()
done.wait(14)
if not done.is_set():
    r = subprocess.run(["nix", "shell", "nixpkgs#gdb", "-c", "gdb", "-p", str(i.proc.pid), "-batch", "-ex", "thread apply all bt 14"],
                       capture_output=True, text=True, timeout=120)
    open(NIGHT / "logs/a14-stack.log", "w").write(r.stdout + r.stderr)
    print("stack saved", flush=True)
done.wait(40)
i.close()
