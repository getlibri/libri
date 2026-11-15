#!/usr/bin/env python3
"""Run under `unshare -Urm`: a store on a small tmpfs of its own. Fills the disk
and the inode table with hippo running; frees them; checks every acknowledged
word and a restart. Also EIO at fdatasync by strace injection (outside tmpfs)."""
import json, os, re, subprocess, sys, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from night import *
from pathlib import Path

BODY = "words of a message of some length. " * 25


def run(name, opts):
    i = Instance("d-" + name, seed=None)
    subprocess.run(["mount", "-t", "tmpfs", "-o", opts, "tmpfs", str(i.store)], check=True)
    print(f"== {name} {opts}", flush=True)
    i.start()
    i.stub()
    time.sleep(6)
    return i


def fill(i, acked, tag, n=100000):
    """Posts until one is refused. Returns the error and how many went in."""
    for k in range(n):
        w = f"{tag}w{k}e"
        try:
            ok, out = i.post(f"{w} {BODY}", wait=45)
        except Exception as e:
            return f"TRANSPORT {e!r}", k
        if not ok:
            return out[:160], k
        acked.append(w)
    return "no refusal", n


def check(i, acked, tag):
    # The search answer stops at 100 lines, so read the day files themselves.
    text = "".join(p.read_text(errors="replace") for p in (i.store / "main").glob("*.jsonl"))
    found = set(re.findall(tag + r"w\d+e", text))
    print(f"   acked={len(acked)} found={len(found)} lost={len([w for w in acked if w not in found])} extra={len(found - set(acked))}", flush=True)


def health(i):
    h = json.loads(i.ask("health")[1])
    return {k: h.get(k) for k in ("state", "reason")}, (h.get("last_error") or {}).get("error", "")[:90]


def disk():
    i = run("full", "size=512k,nr_inodes=4000")
    (i.store / "ballast").write_bytes(b"\0" * 200000)
    acked = []
    err, n = fill(i, acked, "F")
    print("   first refusal after", n, ":", err, flush=True)
    print("   probe", probe(i), flush=True)
    print("   health", health(i), flush=True)
    print("   df", subprocess.run(["df", "--output=avail,iavail", str(i.store)], capture_output=True, text=True).stdout.split()[-2:], flush=True)
    time.sleep(3)
    check(i, acked, "F")
    # Restart on the full disk: does it come back?
    i.kill(); time.sleep(0.3)
    up = i.start(within=20)
    print("   restart on the full disk:", up, i.logs(1), flush=True)
    if up:
        print("   probe", probe(i), flush=True)
        check(i, acked, "F")
    # Free room. The usage log and old views are what can go.
    (i.store / "ballast").unlink()
    time.sleep(2)
    if i.proc.poll() is not None or not up:
        i.proc = None
        print("   restart after room:", i.start(within=20), flush=True)
    print("   after room:", probe(i), health(i), flush=True)
    check(i, acked, "F")
    time.sleep(7)
    ok, st = i.ask("status")
    print("   ", [l[:100] for l in st.splitlines() if l.startswith("tree")], flush=True)
    i.close()
    subprocess.run(["umount", str(i.store)])


def inodes():
    i = run("inodes", "size=8m,nr_inodes=24")
    acked = []
    for k in range(10):
        i.post(f"I0w{k}e {BODY}"); acked.append(f"I0w{k}e")
    n = 0
    fillers = i.store / "filler"
    fillers.mkdir()
    try:
        while True:
            (fillers / str(n)).write_text("")
            n += 1
    except OSError as e:
        print("   inodes gone after", n, "fillers:", e, flush=True)
    for k in range(10, 20):
        i.post(f"I0w{k}e {BODY}"); acked.append(f"I0w{k}e")
    print("   probe", probe(i), flush=True)
    print("   health", health(i), flush=True)
    print("   log", i.logs(3), flush=True)
    check(i, acked, "I0")
    i.kill(); time.sleep(0.3)
    print("   restart with no inodes:", i.start(within=20), i.logs(1), flush=True)
    i.close()
    subprocess.run(["umount", str(i.store)])


def eio():
    i = Instance("d-eio")
    cmd = f"exec strace -f -qq -o /dev/null -e trace=fdatasync -e inject=fdatasync:error=EIO:when=1+ {HIPPO} serve code"
    i.stub()
    p = subprocess.Popen(["bash", "-c", cmd], env=i.env(), stdout=subprocess.DEVNULL, stderr=open(i.log, "a"), start_new_session=True)
    i.proc = p
    while not i.sock.exists():
        time.sleep(0.05)
    time.sleep(0.5)
    print("== fdatasync EIO", flush=True)
    rs = [i.post(f"E{k}x") for k in range(6)]
    print("  ", rs[:6], flush=True)
    print("   probe", probe(i), health(i), flush=True)
    # Where did the unacknowledged words go: the page cache may still hold them.
    print("   tail", [json.loads(l)["text"][:20] for l in i.tail(3).splitlines()], flush=True)
    i.close()
    for e in Path("/proc").glob("[0-9]*/environ"):
        try:
            if f"HIPPO_DIR={i.store}".encode() in e.read_bytes().split(b"\0"):
                os.kill(int(e.parent.name), 9)
        except OSError:
            pass


for n in sys.argv[1:] or ["disk", "inodes", "eio"]:
    try:
        globals()[n]()
    except Exception as e:
        print("DRIVER", n, repr(e), flush=True)
