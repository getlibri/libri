#!/usr/bin/env python3
"""Hostile input on the socket: huge, binary, invalid UTF-8, stalled clients,
a burst of writers. Reports the daemon's peak memory and what a turn met."""
import json, os, re, socket, subprocess, sys, threading, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from night import *
from pathlib import Path


def rss(i):
    s = Path(f"/proc/{i.proc.pid}/status").read_text() if i.proc.poll() is None else ""
    m = {k: re.search(k + r":\s+(\d+)", s) for k in ("VmHWM", "VmRSS", "Threads")}
    return {k: int(v.group(1)) for k, v in m.items() if v}


def send_bytes(i, b, wait=120):
    s = socket.socket(socket.AF_UNIX)
    s.settimeout(wait)
    s.connect(str(i.sock))
    s.sendall(b)
    buf = b""
    while not buf.endswith(b"\n"):
        g = s.recv(1 << 20)
        if not g:
            break
        buf += g
    return buf[:200]


def big(size_mb, compactor="ok"):
    i = Instance(f"b-{size_mb}")
    i.start()
    i.set_mode(compactor)
    i.stub()
    time.sleep(6)
    t = time.monotonic()
    text = "y" * (size_mb << 20)
    try:
        ok, out = i.post(text, wait=240)
    except Exception as e:
        ok, out = False, repr(e)
    print(f"== {size_mb} MB post: ok={ok} {out[:90]} {round(time.monotonic()-t,2)}s rss={rss(i)}", flush=True)
    del text
    p = probe(i, wait=120)
    print("   probe", p, rss(i), flush=True)
    time.sleep(3)
    ok, v = i.ask("view", "whole", wait=120)
    print("   view bytes", len(v), "calls", i.calls(), flush=True)
    ok, st = i.ask("status")
    print("  ", [l[:110] for l in st.splitlines() if l.startswith("tree")], flush=True)
    print("   ", i.logs(2), flush=True)
    i.kill(); time.sleep(0.3)
    t = time.monotonic()
    print("   restart", i.start(within=60), round(time.monotonic() - t, 1), flush=True)
    i.close()
    subprocess.run(["rm", "-rf", str(i.root)])


def odd():
    i = Instance("b-odd")
    i.start()
    i.stub()
    time.sleep(6)
    cases = {
        "invalid_utf8_line": b'{"cmd":"post","args":["Iris","user","\xff\xfe bad"]}\n',
        "binary_line": bytes(range(256)) + b"\n",
        "nul_in_json": json.dumps({"cmd": "post", "args": ["Iris", "user", "a\x00b\x01c"]}).encode() + b"\n",
        "lone_surrogate": b'{"cmd":"post","args":["Iris","user","a\\ud800b"]}\n',
        "not_json": b"hello\n",
        "wrong_shape": b'{"cmd":5}\n',
        "empty_line": b"\n",
        "unknown_cmd": b'{"cmd":"explode","args":[]}\n',
        "bad_zoom": b'{"cmd":"zoom","args":["18446744073709551615","18446744073709551615"]}\n',
        "zoom_big_n": b'{"cmd":"zoom","args":["0","9223372036854775808"]}\n',
        "tail_huge": b'{"cmd":"tail","args":["18446744073709551615"]}\n',
        "bad_regex": b'{"cmd":"search","args":["(((("]}\n',
        "huge_regex": json.dumps({"cmd": "search", "args": ["a{1000}{1000}{1000}"]}).encode() + b"\n",
        "bad_kind": b'{"cmd":"post","args":["Iris","nonsense","x"]}\n',
        "title_newline": json.dumps({"cmd": "post", "args": ["Br\nid/../ge", "user", "x"]}).encode() + b"\n",
        "rtl_and_emoji": json.dumps({"cmd": "post", "args": ["Iris", "user", "‮\U0001f40d" * 2000]}, ensure_ascii=False).encode() + b"\n",
    }
    for name, b in cases.items():
        try:
            r = send_bytes(i, b, wait=20)
        except Exception as e:
            r = repr(e)[:80]
        alive = i.proc.poll() is None
        print(f"{name:20} -> {r[:110]!r} alive={alive}", flush=True)
        if not alive:
            print("   DIED", i.logs(3), flush=True)
            break
    print("probe", probe(i), flush=True)
    ok, t = i.ask("tail", "3")
    print("tail ok", ok, flush=True)
    i.kill(); time.sleep(0.3)
    print("restart", i.start(within=30), i.logs(1), flush=True)
    i.close()
    subprocess.run(["rm", "-rf", str(i.root)])


def idle():
    i = Instance("b-idle")
    i.start()
    i.stub()
    held = []
    for n in range(4000):
        s = socket.socket(socket.AF_UNIX)
        try:
            s.settimeout(5)
            s.connect(str(i.sock))
            s.sendall(b'{"cmd":"view"')
            held.append(s)
        except Exception as e:
            print("connect failed at", n, repr(e), flush=True)
            break
    time.sleep(1)
    print("idle clients", len(held), rss(i), flush=True)
    print("probe", probe(i), flush=True)
    i.close()
    subprocess.run(["rm", "-rf", str(i.root)])


def burst():
    i = Instance("b-burst")
    i.start()
    i.stub()
    time.sleep(6)
    res, lat = [], []
    lock = threading.Lock()
    def writer(w):
        for k in range(100):
            t = time.monotonic()
            try:
                ok, out = i.post(f"B{w}x{k}e words", wait=60)
            except Exception as e:
                ok, out = False, repr(e)
            with lock:
                res.append((f"B{w}x{k}e", ok, out))
                lat.append(time.monotonic() - t)
    ts = [threading.Thread(target=writer, args=(w,)) for w in range(32)]
    t0 = time.monotonic()
    [t.start() for t in ts]
    [t.join() for t in ts]
    took = time.monotonic() - t0
    bad = [r for r in res if not r[1]]
    lat.sort()
    print(f"burst 3200 posts {round(took,2)}s failed={len(bad)} {bad[:2]} p50={lat[len(lat)//2]:.3f} max={lat[-1]:.3f}", flush=True)
    ok, text = i.ask("search", r"B\d+x\d+e")
    found = re.findall(r"B\d+x\d+e", text)
    print("search shows", len(found), "(capped at 100 by SEARCH_MAX)", flush=True)
    day = i.last_day().read_text()
    inf = re.findall(r'"text":"(B\d+x\d+e)', day)
    print("in the day file", len(inf), "distinct", len(set(inf)), flush=True)
    i.kill(); time.sleep(0.3)
    print("restart", i.start(within=30), flush=True)
    i.close()
    subprocess.run(["rm", "-rf", str(i.root)])


for a in sys.argv[1:]:
    try:
        if a == "odd":
            odd()
        elif a == "idle":
            idle()
        elif a == "burst":
            burst()
        else:
            big(int(a))
    except Exception as e:
        print("DRIVER", a, repr(e), flush=True)
