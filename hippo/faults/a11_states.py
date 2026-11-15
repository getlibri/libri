#!/usr/bin/env python3
"""States a crash or power cut leaves: forget stopped between its renames, and
everything not fsynced gone at once. Also EIO while the store is loaded."""
import json, os, shutil, subprocess, sys, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from night import *

def report(i, name):
    up = i.start(within=30)
    print(f"{name}: up={up} {i.logs(2)}", flush=True)
    if up:
        i.stub()
        ok, st = i.ask("status")
        print("   ", [l[:120] for l in st.splitlines()[:4]], flush=True)
        print("    probe", probe(i), flush=True)
        ok, v = i.ask("view", "whole")
        print("    view bytes", len(v), flush=True)
        print("    id after a post:", json.loads(i.tail(1).splitlines()[-1])["i"], flush=True)
    i.close()
    subprocess.run(["rm", "-rf", str(i.root)])

def forget(stop):
    i = Instance(f"f-{stop}")
    s = i.store
    subprocess.run(["cp", "-a", "--reflink=auto", str(s / "main"), str(s / "main.new")], check=True)
    subprocess.run(["cp", "-a", "--reflink=auto", str(s / "tree"), str(s / "tree.new")], check=True)
    (s / "tree").rename(s / "tree.gone")
    if stop >= 2:
        (s / "main").rename(s / "main.gone")
    if stop >= 3:
        (s / "main.new").rename(s / "main")
    report(i, f"forget stopped after rename {stop}")

def power():
    i = Instance("f-power")
    s = i.store
    p = i.last_day()
    b = p.read_bytes()
    p.write_bytes(b[: len(b) - 40])
    t = i.last_day("tree")
    ls = t.read_bytes().split(b"\n")
    t.write_bytes(b"\n".join(ls[:-60]) + b"\n")
    for n in ("view.json", "context.json", "state/cursors.json"):
        if (s / n).exists():
            (s / n).write_bytes(b"")
    report(i, "power cut: last line torn, unsynced tree gone, renamed files empty")

def eio(n):
    i = Instance(f"f-eio{n}")
    i.stub()
    cmd = f"exec strace -f -qq -o /dev/null -e trace=read -e inject=read:error=EIO:when={n} {HIPPO} serve code"
    p = subprocess.Popen(["bash", "-c", cmd], env=i.env(), stdout=subprocess.DEVNULL,
                         stderr=open(i.log, "a"), start_new_session=True)
    i.proc = p
    t = time.monotonic()
    while time.monotonic() - t < 60 and p.poll() is None and not i.sock.exists():
        time.sleep(0.05)
    print(f"EIO at read {n}: exit={p.poll()} sock={i.sock.exists()} {i.logs(2)}", flush=True)
    if p.poll() is None:
        try:
            print("   probe", probe(i), flush=True)
        except Exception as e:
            print("   probe", repr(e)[:90], flush=True)
    want = f"HIPPO_DIR={i.store}".encode()
    for e in Path("/proc").glob("[0-9]*/environ"):
        try:
            if want in e.read_bytes().split(b"\0"):
                os.kill(int(e.parent.name), 9)
        except OSError:
            pass
    subprocess.run(["rm", "-rf", str(i.root)])

from pathlib import Path
for a in sys.argv[1:]:
    print("==", a, flush=True)
    try:
        f, *r = a.split(":")
        globals()[f](*map(int, r))
    except Exception as e:
        print("DRIVER", a, repr(e), flush=True)
