#!/usr/bin/env python3
"""The ride-through on the review branch: PKG=/tmp/libri-night/hippo/pkg-fixed."""
import json, signal, sys, threading, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from rig import *
from c2_chat_children import children


def model_saw(r, word):
    return any(word in json.dumps(t) for t in r.turns())


def tail_words(r, *words):
    t = r.hs("tail", "30")
    return {w: t.count(w) for w in words}


def report(r, *words):
    print("  kept", tail_words(r, *words), "spool", sorted(p.name for p in (r.root / "state/spool").iterdir()), flush=True)


def down():
    r = Rig("c4-down", 9194)
    r.hippo(); r.up()
    print("warm", r.send("hello")[0], flush=True)
    r.kill_hippo()
    a = r.send("HELDONE words while hippo is down", wait=20)
    print("turn 1", a[0], a[2][-300:], "calls", len(r.turns()), flush=True)
    b = r.send("HELDTWO more words", wait=20)
    print("turn 2", b[0], b[2][-200:], "calls", len(r.turns()), "model saw HELDONE", model_saw(r, "HELDONE words"), flush=True)
    print("history", r.status("/api/history", wait=5), flush=True)
    r.hippo()
    time.sleep(21)
    report(r, "HELDONE", "HELDTWO", "claude answers: HELDONE")
    print("  order", [json.loads(l).get("text", "")[:40] for l in r.hs("tail", "6").splitlines()], flush=True)
    print("next", r.send("after")[2][-160:], flush=True)
    r.close()


def hung():
    r = Rig("c4-hung", 9195)
    r.hippo(); r.up()
    print("warm", r.send("hello")[0], flush=True)
    r.sig(signal.SIGSTOP)
    a = r.send("HUNGWORD words at a hung hippo", wait=60)
    print("turn", a[0], a[2][-240:], flush=True)
    b = r.send("HUNGNEXT next", wait=30)
    print("next turn", b[0], flush=True)
    r.sig(signal.SIGCONT)
    time.sleep(21)
    report(r, "HUNGWORD", "HUNGNEXT")
    r.close()


def mid_kill():
    r = Rig("c4-mid", 9196)
    r.claude.write_text(r.claude.read_text().replace("time.sleep(60)", "time.sleep(5)"))
    r.hippo(); r.up()
    print("warm", r.send("hello")[1], flush=True)
    box = {}
    th = threading.Thread(target=lambda: box.update(x=r.send("WORK SLOWLY please MIDWORD", wait=40)))
    th.start()
    time.sleep(2)
    r.kill_hippo()
    th.join()
    print("turn", box["x"][0], box["x"][2][-300:], flush=True)
    r.hippo()
    time.sleep(21)
    report(r, "MIDWORD", "claude answers: WORK SLOWLY")
    r.close()


def refusing():
    r = Rig("c4-ro", 9197)
    r.hippo(); r.up()
    print("warm", r.send("hello")[0], flush=True)
    day = sorted((r.dir / "main").iterdir())[-1]
    day.chmod(0o444)
    subprocess.run(["chmod", "500", str(r.dir / "main")])
    r.kill_hippo(); r.hippo()
    print("up on read-only", r.h.poll(), flush=True)
    a = r.send("ROWORD words hippo refuses", wait=25)
    print("turn", a[0], a[2][-240:], flush=True)
    subprocess.run(["chmod", "700", str(r.dir / "main")])
    day.chmod(0o644)
    r.kill_hippo(); r.hippo()
    time.sleep(21)
    report(r, "ROWORD")
    r.close()


def libri_gone():
    """Words held when Libri itself dies: they go in from the file."""
    r = Rig("c4-gone", 9198)
    r.hippo(); r.up()
    r.send("hello")
    r.kill_hippo()
    print("turn", r.send("GONEWORD words", wait=20)[0], flush=True)
    r.sig(signal.SIGKILL, r.d); r.d.wait()
    r.hippo()
    r.up()
    time.sleep(21)
    report(r, "GONEWORD")
    r.close()


def no_disk():
    """hippo down and the spool unwritable: the reply must still arrive, saying so."""
    r = Rig("c4-nodisk", 9199)
    r.hippo(); r.up()
    r.send("hello")
    r.kill_hippo()
    subprocess.run(["chmod", "500", str(r.root / "state/spool")])
    a = r.send("NODISK words", wait=20)
    print("turn", a[0], a[2][-300:], "calls", len(r.turns()), flush=True)
    subprocess.run(["chmod", "700", str(r.root / "state/spool")])
    r.close()


def lone():
    r = Rig("c4-lone", 9188)
    r.up(lone=True, extra={"HIPPO_AGENT": "Kit", "LIBRI_HIPPO_SERVE": "1"})
    time.sleep(1)
    print("children", children(r), "turn", r.send("hello from kit side", wait=15)[0], flush=True)
    kids = children(r)
    raw = Path(f"/proc/{kids[0]}/environ").read_bytes()
    assert f"HIPPO_DIR={r.dir}".encode() in raw.split(b"\0"), "child is mine"
    os.kill(kids[0], signal.SIGKILL)
    a = r.send("LONEWORD after the child died", wait=15)
    print("turn", a[0], a[2][-200:], flush=True)
    time.sleep(24)
    print("children", children(r), "history", r.status("/api/history", wait=5), flush=True)
    report(r, "LONEWORD")
    pids = children(r)
    r.close()
    time.sleep(1)
    print("after Libri dies, children", children(r), flush=True)


def cold():
    """Libri started with hippo already down, as after a boot where hippo failed."""
    r = Rig("c4-cold", 9187)
    print("up", r.up(), flush=True)
    a = r.send("COLDWORD words", wait=20)
    print("turn", a[0], a[2][-200:], "history", r.status("/api/history", wait=8), "calls", len(r.turns()), flush=True)
    r.hippo()
    time.sleep(21)
    report(r, "COLDWORD")
    print("next", r.send("after")[0], flush=True)
    time.sleep(1)
    print("said", [l[:140] for l in r.ev if "emory" in l][-3:], flush=True)
    r.close()


for n in sys.argv[1:]:
    try:
        print("==", n, flush=True)
        globals()[n]()
    except Exception as e:
        print("DRIVER", repr(e), flush=True)
