#!/usr/bin/env python3
"""hippo failing while a turn runs; Stop during a hung hippo; a huge view."""
import json, signal, sys, time
sys.path.insert(0, "/tmp/libri-night/hippo/repro")
from rig import *


def slowed(r, secs=5):
    r.claude.write_text(r.claude.read_text().replace("time.sleep(60)", f"time.sleep({secs})"))


def mid_kill():
    r = Rig("c3-mid", 9190)
    slowed(r)
    r.hippo(); r.up()
    print("warm", r.send("hello")[1], flush=True)
    t0 = time.monotonic()
    import threading
    box = {}
    th = threading.Thread(target=lambda: box.update(x=r.send("WORK SLOWLY please MIDWORD", wait=25)))
    th.start()
    time.sleep(2)
    print("calls so far", len(r.turns()), "user word saved", r.has("MIDWORD"), flush=True)
    r.kill_hippo()
    th.join()
    print("turn", box["x"][0], box["x"][2][-420:], flush=True)
    r.hippo()
    time.sleep(1)
    print("after restart: user", r.has("MIDWORD"), "reply kept", r.has("claude answers: WORK SLOWLY"), flush=True)
    print("next", r.send("next words")[2][-200:], flush=True)
    print("hist tail", r.hs("tail", "4")[-400:], flush=True)
    print([l[:170] for l in (r.root / "libri.log").read_text(errors="replace").splitlines()[-4:]], flush=True)
    r.close()


def mid_hang():
    r = Rig("c3-hang", 9191)
    slowed(r, 3)
    r.hippo(); r.up()
    print("warm", r.send("hello")[1], flush=True)
    r.sig(signal.SIGSTOP)
    time.sleep(0.2)
    import threading
    box = {}
    th = threading.Thread(target=lambda: box.update(x=r.send("words at a hung hippo", wait=12)))
    th.start()
    time.sleep(6)
    print("/api/stop", r.status_post("/api/stop"), flush=True)
    th.join()
    print("turn", box["x"][0], box["x"][2][-300:], flush=True)
    r.sig(signal.SIGCONT)
    time.sleep(1.5)
    print("after cont: saved", r.has("words at a hung hippo"), "calls", len(r.turns()), flush=True)
    r.close()


def big_view():
    r = Rig("c3-view", 9192)
    r.hippo(); r.up()
    (r.root / "summarizer-down").write_text("")
    r.send("small")
    r.send("q " * 800000)
    time.sleep(1)
    print("view", len(r.hs("view", "whole", wait=20)), flush=True)
    t, st, o = r.send("words after the large one", wait=20)
    c = r.turns()[-1]
    print("turn", t, st, "model was given", len(c["said"]), flush=True)
    r.close()


def huge_summary():
    r = Rig("c3-huge", 9193)
    r.claude.write_text(r.claude.read_text().replace("""end(f"{kind.group(1) if kind else 'talk'}: stand-in summary")""", """end(f"{kind.group(1) if kind else 'talk'}: " + "s" * 700000)"""))
    r.hippo(); r.up()
    for k in range(5):
        r.send(f"{k} words " + "w " * 1500)
    time.sleep(40)
    print(r.hs("status").splitlines()[:2], flush=True)
    print("view", len(r.hs("view", "whole", wait=30)), flush=True)
    t, st, o = r.send("words after the summaries", wait=30)
    c = [x for x in r.turns() if not x["plain"]][-1]
    print("turn", t, st, "model was given", len(c["said"]), flush=True)
    r.close()


def _p(self, path):
    t = time.monotonic()
    try:
        with self.req(path, {}, wait=15) as x:
            return x.status, round(time.monotonic() - t, 2)
    except urllib.error.HTTPError as e:
        return e.code, round(time.monotonic() - t, 2)
    except Exception as e:
        return repr(e)[:60], round(time.monotonic() - t, 2)
Rig.status_post = _p

for n in sys.argv[1:]:
    print("==", n, flush=True)
    try:
        globals()[n]()
    except Exception as e:
        print("DRIVER", repr(e), flush=True)
