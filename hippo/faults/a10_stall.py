#!/usr/bin/env python3
"""Attributes slow posts on a loaded host: posts to an instance whose store sits
beside a plain file probe doing the same fdatasync, both timed for `secs`."""
import os, sys, threading, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from night import *

secs = int(sys.argv[1]) if len(sys.argv) > 1 else 300
i = Instance("s-stall")
i.start(); i.stub(); time.sleep(3)
posts, syncs = [], []
end = time.monotonic() + secs
def probe_fs():
    f = open(i.root / "probe", "ab")
    while time.monotonic() < end:
        t = time.monotonic()
        f.write(b"x" * 200); f.flush(); os.fdatasync(f.fileno())
        syncs.append((t, time.monotonic() - t))
        time.sleep(0.1)
threading.Thread(target=probe_fs, daemon=True).start()
while time.monotonic() < end:
    t = time.monotonic()
    i.post("a short line", wait=300)
    posts.append((t, time.monotonic() - t))
    time.sleep(0.1)
def top(x): return [round(d, 2) for _, d in sorted(x, key=lambda y: -y[1])[:5]]
print("posts", len(posts), "top", top(posts), "syncs", len(syncs), "top", top(syncs), flush=True)
for t, d in sorted(posts, key=lambda y: -y[1])[:3]:
    near = [round(s, 2) for w, s in syncs if t - 2 <= w <= t + d]
    print("  slow post", round(d, 2), "plain fdatasync max in the same window", max(near or [0]), flush=True)
i.close()
