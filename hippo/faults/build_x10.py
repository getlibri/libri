#!/usr/bin/env python3
"""A store ten times the seed: blocks of 65536 messages with their complete
subtrees, ids offset by block, days moved one year per block."""
import json, os, shutil, sys
from pathlib import Path
SEED = Path(os.environ["HIPPO_FAULT_SEED"])
OUT = Path("/tmp/libri-night/hippo/run/x10/store")
BLOCK, K = 65536, int(sys.argv[1]) if len(sys.argv) > 1 else 10
shutil.rmtree(OUT.parent, ignore_errors=True)
(OUT / "main").mkdir(parents=True); (OUT / "tree").mkdir(); (OUT / "state").mkdir()
ms = []
for p in sorted((SEED / "main").glob("*.jsonl")):
    for l in p.read_bytes().decode().split("\n"):
        if l.strip():
            ms.append(json.loads(l))
ms = ms[:BLOCK]
ts = []
for p in sorted((SEED / "tree").glob("*.jsonl")):
    for l in p.read_bytes().decode().split("\n"):
        try:
            n = json.loads(l)
        except ValueError:
            continue
        if n["l"] <= 16 and ((n["i"] + 1) << n["l"]) <= BLOCK:
            ts.append(n)
print(len(ms), len(ts))
for k in range(K):
    year = 2000 + k
    name = f"{year}-06-01.jsonl"
    with open(OUT / "main" / name, "w") as f:
        for m in ms:
            c = dict(m, i=m["i"] + k * BLOCK, date=f"{year}" + m["date"][4:])
            if c.get("src"):
                c["src"] = dict(c["src"], e=f"{c['src'].get('e')}-{k}")
            f.write(json.dumps(c, ensure_ascii=False, separators=(",", ":")) + "\n")
    with open(OUT / "tree" / name, "w") as f:
        for n in ts:
            f.write(json.dumps(dict(n, i=n["i"] + k * (BLOCK >> n["l"])), ensure_ascii=False, separators=(",", ":")) + "\n")
shutil.copy(SEED / "chats.jsonl", OUT / "chats.jsonl")
