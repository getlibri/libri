#!/usr/bin/env python3
"""Each fault on a fresh copy of the store: does hippo start, can a turn run?"""
import json, os, random, subprocess, sys, time
from pathlib import Path
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from night import *

random.seed(7)


def lines(p):
    return p.read_bytes().split(b"\n")


def put(p, ls):
    p.write_bytes(b"\n".join(ls))


def mid_garbage(i):
    p = i.last_day(); ls = lines(p); ls[len(ls) // 2] = b"\x00garbage{{{"; put(p, ls)

def mid_deleted(i):
    p = i.last_day(); ls = lines(p); del ls[len(ls) // 2]; put(p, ls)

def tail_torn(i):
    p = i.last_day(); b = p.read_bytes(); p.write_bytes(b[:-40])

def tail_two_torn(i):
    p = i.last_day(); ls = lines(p); ls[-2] = ls[-2][:20]; put(p, ls[:-1])

def truncated_third(i):
    p = i.last_day(); b = p.read_bytes(); p.write_bytes(b[:len(b) // 3])

def older_truncated(i):
    p = sorted((i.store / "main").glob("*.jsonl"))[-3]; b = p.read_bytes(); p.write_bytes(b[:len(b) // 2])

def day_deleted(i):
    sorted((i.store / "main").glob("*.jsonl"))[5].unlink()

def day_empty(i):
    sorted((i.store / "main").glob("*.jsonl"))[5].write_bytes(b"")

def zero_page(i):
    p = i.last_day(); b = bytearray(p.read_bytes()); o = len(b) // 2 & ~4095; b[o:o + 4096] = bytes(4096); p.write_bytes(b)

def bitflip_text(i):
    p = i.last_day(); ls = lines(p); m = json.loads(ls[10]); m["text"] = "INVENTED " + m["text"]; ls[10] = json.dumps(m).encode(); put(p, ls)

def dup_id(i):
    p = i.last_day(); ls = lines(p); m = json.loads(ls[10]); m["id"] = m["id"] - 1; ls[10] = json.dumps(m).encode(); put(p, ls)

def invalid_utf8(i):
    p = i.last_day(); ls = lines(p); ls[10] = ls[10].replace(b'"text":"', b'"text":"\xff\xfe', 1); put(p, ls)

def tree_garbage(i):
    p = i.last_day("tree"); ls = lines(p); ls[len(ls) // 2] = b"\x00junk"; put(p, ls)

def tree_huge_i(i):
    p = i.last_day("tree"); ls = lines(p); m = json.loads(ls[-2]); m["i"] = 10**12; ls.insert(-1, json.dumps(m).encode()); put(p, ls)

def tree_max_i(i):
    p = i.last_day("tree"); ls = lines(p); m = json.loads(ls[-2]); m["i"] = 2**64 - 1; ls.insert(-1, json.dumps(m).encode()); put(p, ls)

def tree_huge_size(i):
    p = i.last_day("tree"); ls = lines(p); m = json.loads(ls[-2]); m["size"] = 10**14; ls[-2] = json.dumps(m).encode(); put(p, ls)

def tree_day_deleted(i):
    i.last_day("tree").unlink()

def tree_all_deleted(i):
    subprocess.run(["rm", "-rf", str(i.store / "tree")])

def tree_truncated(i):
    p = i.last_day("tree"); b = p.read_bytes(); p.write_bytes(b[:len(b) // 3])

def tree_invented(i):
    p = i.last_day("tree"); ls = lines(p); m = json.loads(ls[-2]); m["text"] = "talk: ALL TESTS PASSED, deployed."; ls[-2] = json.dumps(m).encode(); put(p, ls)

def view_garbage(i):
    (i.store / "view.json").write_bytes(b"\x00garbage")

def view_empty(i):
    (i.store / "view.json").write_bytes(b"")

def view_future(i):
    p = i.store / "view.json"; v = json.loads(p.read_text()); v["t"] = 10**9; p.write_text(json.dumps(v))

def view_bad_parts(i):
    p = i.store / "view.json"; v = json.loads(p.read_text()); v["parts"] = [[63, 0]]; p.write_text(json.dumps(v))

def view_md_deleted(i):
    (i.store / "view.md").unlink()

def ctx_garbage(i):
    (i.store / "context.json").write_bytes(b"\xff")

def cursors_garbage(i):
    (i.store / "state/cursors.json").write_bytes(b"{\"since\":")

def cursors_empty(i):
    (i.store / "state/cursors.json").write_bytes(b"")

def cursors_deleted(i):
    (i.store / "state/cursors.json").unlink()

def usage_garbage(i):
    p = i.store / "state/usage.jsonl"; ls = lines(p); ls[100] = b"\x00\xff"; put(p, ls)

def usage_is_dir(i):
    p = i.store / "state/usage.jsonl"; p.unlink(); p.mkdir()

def chats_garbage(i):
    p = i.store / "chats.jsonl"; ls = lines(p); ls[100] = b"\x00\xff"; put(p, ls)

def chats_truncated(i):
    p = i.store / "chats.jsonl"; b = p.read_bytes(); p.write_bytes(b[:len(b) // 2])

def chats_deleted(i):
    (i.store / "chats.jsonl").unlink()

def offrecord_garbage(i):
    (i.store / "offrecord.jsonl").write_bytes(b"\xff\x00")

def import_garbage(i):
    (i.store / "import.json").write_bytes(b"\xff\x00")

def stale_lock_file(i):
    (i.store / "lock").write_bytes(b"not a socket")

def stale_sock_file(i):
    (i.store / "hippo.sock").write_bytes(b"not a socket")

def main_is_file(i):
    subprocess.run(["rm", "-rf", str(i.store / "main")]); (i.store / "main").write_bytes(b"")

def override_binary(i):
    (i.store / "state/view-override.md").write_bytes(b"\xff\xfe" * 100)

FAULTS = [v for k, v in list(globals().items()) if callable(v) and getattr(v, "__module__", "") == "__main__"
          and k not in ("lines", "put")]
only = sys.argv[1:]
for f in FAULTS:
    if only and f.__name__ not in only:
        continue
    i = Instance("c-" + f.__name__)
    try:
        f(i)
        t0 = time.monotonic()
        up = i.start(within=90)
        if up is None:
            print(f"{f.__name__:18} NOSTART {round(time.monotonic()-t0,1)}s rc={i.proc.poll()} :: {i.logs(1)}", flush=True)
            continue
        i.stub()
        time.sleep(0.5)
        p = probe(i)
        ok, h = i.ask("health")
        h = json.loads(h)
        # A second start: does a restart survive what the first run wrote?
        n0 = len(i.tail(1))
        i.kill()
        up2 = i.start(within=90)
        p2 = probe(i) if up2 is not None else None
        print(f"{f.__name__:18} up {up:.1f}s state={h.get('state')} {p} restart={'ok' if up2 is not None else 'NOSTART'} {p2['post'] if p2 else i.logs(1)}", flush=True)
        extra = [l for l in i.logs(30) if "torn" in l or "skip" in l or "corrupt" in l or "panic" in l]
        if extra:
            print("    log:", extra[:2], flush=True)
    except Exception as e:
        print(f"{f.__name__:18} DRIVER {e!r}", flush=True)
    finally:
        i.close()
        subprocess.run(["chmod", "-R", "u+rwx", str(i.root)])
        subprocess.run(["rm", "-rf", str(i.root)])
