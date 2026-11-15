import os
from pathlib import Path

def children(r):
    out = []
    for e in Path("/proc").glob("[0-9]*/environ"):
        try:
            if (e.parent / "comm").read_text().strip() == "hippo" and f"HIPPO_DIR={r.dir}".encode() in e.read_bytes().split(b"\0"):
                out.append(int(e.parent.name))
        except OSError:
            pass
    return out


