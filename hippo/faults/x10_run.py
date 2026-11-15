#!/usr/bin/env python3
import json, re, socket, sys, threading, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
import night
from night import *
from pathlib import Path
night.HIPPO = str(NIGHT / "pkg/bin/hippo")
name, make = sys.argv[1], Instance
i = Instance.__new__(Instance)
i.name, i.backend, i.limit, i.more = name, "libri", "unlimited", {}
i.root = NIGHT / "run" / name
i.store = i.root / "store"
i.mode = i.root / "mode"; i.mode.write_text("ok")
i.log = i.root / "hippo.log"; i.proc = i.stub_proc = None
for d in ("home", "cache", "config", "data", "runtime"):
    (i.root / d).mkdir(exist_ok=True)
t = i.start(within=300)
print("start seconds", t, i.logs(1), flush=True)
st = Path(f"/proc/{i.proc.pid}/status").read_text()
print(re.findall(r"(?:VmHWM|VmRSS):\s+\d+", st), flush=True)
i.stub()
print("probe", probe(i, wait=120), flush=True)
for _ in range(3):
    tt = time.monotonic(); ok, o = i.ask("search", "zzzqqqnothing", wait=120); print("search miss", round(time.monotonic() - tt, 2), flush=True)
lat = []
stop = []
def pinger():
    while not stop:
        tt = time.monotonic(); i.post("ping", wait=120); lat.append(time.monotonic() - tt); time.sleep(0.05)
th = threading.Thread(target=pinger); th.start()
for _ in range(4):
    i.ask("search", "zzzqqqnothing", wait=120)
stop.append(1); th.join()
print("post during search: max", round(max(lat), 2), "n", len(lat), flush=True)
time.sleep(5)
ok, s = i.ask("status"); print(s.splitlines()[:2], flush=True)
print(re.findall(r"(?:VmHWM|VmRSS):\s+\d+", Path(f"/proc/{i.proc.pid}/status").read_text()), flush=True)
i.close()
