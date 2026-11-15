#!/usr/bin/env python3
"""kill -9 at exact system calls, by strace's fault injection: a SIGKILL as the
Nth call of each kind is made, under a steady load of posts with compaction on.
After each, a plain restart must hold every acknowledged word exactly once."""
import json, os, random, re, signal, subprocess, sys, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from night import *
from pathlib import Path

TARGETS = {
    "write": [3, 6, 9, 14, 21, 30, 44],
    "fdatasync": [1, 2, 3, 4, 6, 9],
    "fsync": [1, 2, 3],
    "rename": [1, 2, 3, 4, 5, 6, 7, 8],
    "ftruncate": [1],
    "openat": [40, 80, 160],
}
BODY = "x" * 700

def spawn_traced(i, target, n):
    cmd = f"ulimit -v {i.limit}; exec strace -f -qq -o /dev/null -e trace={target} -e inject={target}:signal=SIGKILL:when={n} {HIPPO} serve code"
    return subprocess.Popen(["bash", "-c", cmd], env=i.env(), stdout=subprocess.DEVNULL,
                            stderr=open(i.log, "a"), start_new_session=True)

def cleanup(p, i):
    # Killing strace leaves its tracee running, so go by each pid's own HIPPO_DIR.
    want = f"HIPPO_DIR={i.store}".encode()
    for e in Path("/proc").glob("[0-9]*/environ"):
        try:
            if want in e.read_bytes().split(b"\0"):
                os.kill(int(e.parent.name), signal.SIGKILL)
        except OSError:
            pass
    p.wait()

only = sys.argv[1:]
for target, ns in TARGETS.items():
    if only and target not in only:
        continue
    for n in ns:
        name = f"k-{target}-{n}"
        i = Instance(name)
        i.stub()
        p = spawn_traced(i, target, n)
        i.proc = p
        acked, tried = [], 0
        t0 = time.monotonic()
        while time.monotonic() - t0 < 30:
            if p.poll() is not None:
                break
            if i.sock.exists():
                break
            time.sleep(0.02)
        reached = 0
        stamp = f"N{os.getpid()}x{n}"
        if p.poll() is None:
            for k in range(30):
                w = f"{stamp}w{k}e"
                try:
                    ok, _ = i.post(f"{w} {BODY}", wait=8)
                    if ok:
                        acked.append(w)
                except Exception:
                    break
                time.sleep(0.05)
                if p.poll() is not None:
                    break
        died_at = len(acked)
        fired = len(acked) < 30
        cleanup(p, i)
        (i.store / "lock")  # left as a dead socket on purpose: a kill leaves it
        i.proc = None
        up = i.start(within=60)
        if up is None:
            print(f"{name:18} acked={died_at} NOSTART {i.logs(1)}", flush=True)
            i.close(); subprocess.run(["rm", "-rf", str(i.root)]); continue
        ok, text = i.ask("search", stamp + "w\\d+e", wait=30)
        found = re.findall(stamp + r"w\d+e", text)
        lost = [w for w in acked if w not in found]
        dupes = len(found) - len(set(found))
        pr = probe(i)
        ok, tailed = i.ask("tail", "40")
        ids = [json.loads(l)["i"] for l in tailed.splitlines()]
        gaps = [b for a, b in zip(ids, ids[1:]) if b != a + 1]
        time.sleep(2.5)
        ok, st = i.ask("status")
        tree = [l for l in st.splitlines() if l.startswith("tree")]
        i.kill()
        up2 = i.start(within=60)
        print(f"{name:18} fired={fired} acked={died_at} found={len(found)} lost={len(lost)} dup_lines={dupes} gaps={gaps} view={pr['view'][0]} post={pr['post'][0]} restart2={'ok' if up2 else 'NOSTART'} {tree[0][:80] if tree else ''}", flush=True)
        i.close()
        subprocess.run(["rm", "-rf", str(i.root)])
