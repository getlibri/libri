"""Hostile scenarios against a throwaway Libri. Usage: scenarios.py NAME...
Each prints what it saw; only that counts."""

import sys
from rig import *

ports = iter(range(9201, 9220))


def see(what, **kw):
    print(json.dumps({"see": what} | kw, default=str), flush=True)


def outcome(r, *needles):
    """Where each of these words ended up."""
    c = r.chat()
    return {n: {"harness": r.reached(n), "page": r.on_page(n),
                "hippo_user": None if c is None else sum(1 for m in c if m["kind"] == "user" and n in m["text"]),
                "hippo_talk": None if c is None else sum(1 for m in c if m["kind"] != "user" and n in m["text"])}
            for n in needles}


def timed(ready, within):
    t = time.monotonic()
    ok = until(ready, within=within)
    return round(time.monotonic() - t, 2) if ok else None


def fault(word, wait=8):
    def run():
        r = Rig("h-" + word.lower(), next(ports))
        try:
            before = r.fds()
            r.send(f"{word} first words")
            until(lambda: r.typing(), within=5)
            ended = timed(lambda: not r.typing(), wait)
            see("turn ended?", fault=word, ended_after=ended, shown=[s[:120] for s in r.shown()], log=r.log(6))
            r.send("typed-during")
            reached = timed(lambda: r.reached("typed-during"), 5)
            see("typed words during", fault=word, reached_after=reached)
            t = time.monotonic()
            r.stop()
            stopped = timed(lambda: not r.typing(), 15)
            see("stop", fault=word, stopped_after=stopped)
            until(lambda: r.reached("typed-during"), within=5)
            r.send("afterwards")
            ok = timed(lambda: r.on_page("answered: afterwards"), 15)
            time.sleep(1)
            see("after", fault=word, next_answer_after=ok, shown=[s[:120] for s in r.shown()],
                where=outcome(r, "first words", "typed-during", "afterwards"),
                task_turn_len=len((r.task() or {}).get("turn", {}).get("shown", [])),
                fds=(before, r.fds()), rss=r.rss_kib(), log=r.log(8))
            pid = r.harness_pid()
            if pid:
                try:
                    see("harness after", fault=word, alive=Path(f"/proc/{pid}/stat").exists())
                except OSError:
                    pass
        finally:
            r.close()
    return run


ALL = {w: fault("F-" + w) for w in
       ("HANG", "DEAF", "PART", "CRASH", "SEGV", "GARBAGE", "NOANSWER", "LOOP", "TOOLLOOP", "HOLDOUT", "DENIED")}



def page(r):
    t = r.task() or {}
    return {"typing": t.get("typing"),
            "turn": [(e.get("type"), (e.get("delta") or "")[:40]) for e in t.get("turn", {}).get("shown", [])]}


def kill_stage(stage, sig):
    def run():
        name = f"k-{stage}-{sig.name.lower()}"
        r = Rig(name, 9210)
        conted = False
        try:
            r.send("warmup")
            until(lambda: r.on_page("answered: warmup"), within=10)
            time.sleep(0.3)
            if stage == "view":
                r.signal(r.hippo.pid, signals.SIGSTOP)
                r.send("W1 words")
                time.sleep(0.5)
                r.send("W2 words")
                time.sleep(0.5)
            elif stage == "harness-start":
                (r.root / "hold-start").touch()
                r.send("W1 words")
                until(lambda: (r.root / "started").exists(), within=10)
                r.send("W2 words")
                time.sleep(0.5)
            elif stage == "silent":
                r.send("F-HANG W1 words")
                until(lambda: r.reached("W1"), within=10)
                r.send("W2 words")
                time.sleep(0.5)
            elif stage == "partial":
                r.send("F-PART W1 words")
                until(lambda: r.on_page("half an answer"), within=10)
                r.send("W2 words")
                time.sleep(0.5)
            elif stage == "job-held":
                r.send("F-BGJOB W1 words")
                until(lambda: r.on_page("job started"), within=10)
                r.send("W2 words")
                until(lambda: r.on_page("steered"), within=10)
                time.sleep(0.5)
            pre = page(r)
            t = time.monotonic()
            hp = r.harness_pid()
            r.kill_daemon(sig)
            took = round(time.monotonic() - t, 2)
            orphan = bool(hp and Path(f"/proc/{hp}/stat").exists())
            (r.root / "hold-start").unlink(missing_ok=True)
            if stage == "view":
                r.signal(r.hippo.pid, signals.SIGCONT)
                conted = True
            r.events.clear()
            r.start_daemon()
            time.sleep(4)
            r.send("after restart")
            ok = timed(lambda: r.on_page("answered: after restart"), 15)
            time.sleep(1)
            see("killed", stage=stage, sig=sig.name, exit_took=took, before=pre, orphan_harness=orphan,
                next_answer=ok, after=page(r), shown=[s[:90] for s in r.shown()],
                where=outcome(r, "W1 words", "W2 words"), log=r.log(5),
                stand_in_read=[s[-30:] for s in r.read()])
        finally:
            if not conted and r.hippo.poll() is None:
                r.signal(r.hippo.pid, signals.SIGCONT)
            r.close()
    return run


for stage in ("view", "harness-start", "silent", "partial", "job-held"):
    for sig in (signals.SIGKILL, signals.SIGTERM):
        ALL[f"K-{stage}-{sig.name}"] = kill_stage(stage, sig)


def hippo_down(how):
    def run():
        r = Rig("hippo-" + how, 9211)
        try:
            r.send("warmup")
            until(lambda: r.on_page("answered: warmup"), within=10)
            time.sleep(0.3)
            pid = r.hippo.pid
            r.signal(pid, signals.SIGKILL if how == "killed" else signals.SIGSTOP)
            if how == "killed":
                r.hippo.wait()
            see("history while down", how=how, status=r.status("/api/history", 8), host=r.status("/api/host"))
            print(r.send("down one"))
            ended = timed(lambda: r.on_page("down one") or r.on_page("couldn't"), 15)
            see("turn while down", how=how, ended_after=ended, shown=[s[:140] for s in r.shown()], page=page(r), log=r.log(4))
            print(r.send("down two"))
            time.sleep(3)
            see("second", how=how, page=page(r), shown=[s[:140] for s in r.shown()])
            t = time.monotonic()
            r.stop()
            stopped = timed(lambda: not r.typing(), 8)
            see("stop while down", how=how, stopped_after=stopped)
            if how == "killed":
                r.start_hippo()
            else:
                r.signal(pid, signals.SIGCONT)
            back = timed(lambda: r.on_page("answered: down two"), 15)
            r.send("after back")
            ok = timed(lambda: r.on_page("answered: after back"), 15)
            time.sleep(1)
            see("back", how=how, second_answered=back, next_answer=ok, shown=[s[:140] for s in r.shown()],
                where=outcome(r, "down one", "down two", "after back"), hits=[(m["kind"], m["text"][:50]) for m in r.chat()], log=r.log(6))
        finally:
            if r.hippo.poll() is None:
                try:
                    os.kill(r.hippo.pid, signals.SIGCONT)
                except OSError:
                    pass
            r.close()
    return run


ALL["HIPPO-killed"] = hippo_down("killed")
ALL["HIPPO-frozen"] = hippo_down("frozen")


def hippo_mid(how):
    """Hippo goes while the harness is answering: is the answer kept?"""
    def run():
        r = Rig("hmid-" + how, 9212)
        try:
            r.send("warmup")
            until(lambda: r.on_page("answered: warmup"), within=10)
            time.sleep(0.3)
            r.send("F-SLOW3 mid words")
            until(lambda: r.reached("mid words"), within=10)
            pid = r.hippo.pid
            r.signal(pid, signals.SIGKILL if how == "killed" else signals.SIGSTOP)
            if how == "killed":
                r.hippo.wait()
            ended = timed(lambda: not r.typing(), 15)
            see("mid answer", how=how, ended=ended, shown=[s[:140] for s in r.shown()], log=r.log(4), page=page(r))
            if how == "killed":
                r.start_hippo()
            else:
                r.stop()
                time.sleep(2)
                see("stop", typing=r.typing())
                r.signal(pid, signals.SIGCONT)
            time.sleep(2)
            r.send("after back")
            ok = timed(lambda: r.on_page("answered: after back"), 15)
            time.sleep(1)
            see("back", how=how, next_answer=ok, shown=[s[:140] for s in r.shown()],
                hits=[(m["kind"], m["text"][:50]) for m in r.chat()], log=r.log(6))
        finally:
            if r.hippo.poll() is None:
                try:
                    os.kill(r.hippo.pid, signals.SIGCONT)
                except OSError:
                    pass
            r.close()
    return run


ALL["HMID-killed"] = hippo_mid("killed")
ALL["HMID-frozen"] = hippo_mid("frozen")


def wait_tick(r, text, within=45):
    return timed(lambda: r.reached(text), within)


def col_routine_hang():
    r = Rig("c-routine-hang", 9214)
    try:
        t = r.tools()
        print(t.routine("F-HANG routine"))
        see("routine began", after=wait_tick(r, "F-HANG routine"))
        r.send("T1 words")
        see("words into hung routine", reached=timed(lambda: r.reached("T1 words"), 4), saved=r.count("user", "T1 words"))
        r.stop()
        see("stop", after=timed(lambda: not r.typing(), 8))
        time.sleep(1)
        r.send("T2 words")
        see("next", after=timed(lambda: r.on_page("answered: T2"), 10), shown=[s[:100] for s in r.shown()],
            where=outcome(r, "T1 words", "T2 words"), read=[s[-40:] for s in r.read()])
        t.close()
    finally:
        r.close()


def col_person_hang():
    """A person's hung turn: what else stops behind it."""
    r = Rig("c-person-hang", 9216)
    try:
        t = r.tools()
        r.send("F-HANG person")
        until(lambda: r.reached("F-HANG person"), within=5)
        at = time.strftime("%Y-%m-%d %H:%M", time.localtime())
        print(t.call("watch", {"when": {"at": at}, "then": {"note": "first plain nudge"}}))
        print(t.routine("routine behind the hang"))
        print(t.call("watch", {"when": {"at": at}, "then": {"note": "second plain nudge"}}))
        print(t.call("spawn", {"tasks": [{"task": "worker W-ONE"}]}))
        t0 = time.monotonic()
        time.sleep(70)
        see("after 70 s", typing=r.typing(), shown=[s[:100] for s in r.shown()], read=[s[-40:] for s in r.read()],
            work=json.loads(r.get("/api/work") or b"null"), log=r.log(4))
        r.send("T1 words")
        time.sleep(2)
        r.stop()
        t1 = time.monotonic()
        see("after stop", nudge=timed(lambda: r.on_page("plain nudge"), 45), routine=timed(lambda: r.reached("routine behind"), 45),
            worker=timed(lambda: r.reached("W-ONE"), 45), t1=timed(lambda: r.reached("T1 words"), 5))
        time.sleep(2)
        see("end", shown=[s[:100] for s in r.shown()], read=[s[-40:] for s in r.read()], log=r.log(4))
        t.close()
    finally:
        r.close()


def ask_stage(stage):
    def run():
        r = Rig("a-" + stage, 9217)
        try:
            r.send("warmup")
            until(lambda: r.on_page("answered: warmup"), within=8)
            r.send("F-ASK words")
            until(lambda: r.reached("F-ASK"), within=8)
            time.sleep(0.5)
            see("asked", events=[e for e in r.events if "ask" in e.get("type", "").lower() or e.get("name") == "ask"][:2],
                kinds=sorted({e["type"] + ":" + str(e.get("name")) for e in r.events}), pending=(r.get("/api/pending") or b"")[:300], task=page(r))
            if stage == "typed":
                r.send("T1 words")
                time.sleep(3)
                see("typed in an open ask", reached=r.reached("T1 words"), task=page(r))
                r.stop()
                see("stop", after=timed(lambda: not r.typing(), 8))
            elif stage == "stop":
                r.stop()
                see("stop", after=timed(lambda: not r.typing(), 8))
            elif stage == "restart":
                r.kill_daemon(signals.SIGTERM)
                r.events.clear()
                r.start_daemon()
                time.sleep(2)
                see("after restart", pending=(r.get("/api/pending") or b"")[:300], task=page(r))
            elif stage == "wait":
                time.sleep(20)
                see("still waiting", typing=r.typing())
                r.stop()
            r.send("next words")
            see("next", after=timed(lambda: r.on_page("answered: next"), 10), shown=[s[:100] for s in r.shown()],
                read=[s[-90:] for s in r.read()], where=outcome(r, "T1 words", "next words"), log=r.log(3))
        finally:
            r.close()
    return run


def col_ask_routine():
    r = Rig("c-ask-routine", 9214)
    try:
        t = r.tools()
        print(t.routine("F-ASK routine question"))
        see("routine", after=wait_tick(r, "F-ASK routine"))
        time.sleep(1.5)
        see("end", shown=[s[:100] for s in r.shown()], read=[s[-140:] for s in r.read()], typing=r.typing())
        t.close()
    finally:
        r.close()


ALL |= {"C-routine-hang": col_routine_hang,
        "C-person-hang": col_person_hang, "C-ask-routine": col_ask_routine}
for stage in ("typed", "stop", "restart", "wait"):
    ALL["A-" + stage] = ask_stage(stage)


def corrupt_state():
    """Every file in the state directory, in three broken forms, at a restart."""
    r = Rig("s-corrupt", 9218)
    try:
        t = r.tools()
        at = time.strftime("%Y-%m-%d", time.localtime(time.time() + 86400 * 30)) + " 09:00"
        print(t.call("watch", {"when": {"at": at}, "then": {"note": "future nudge"}}))
        print(t.call("spawn", {"tasks": [{"task": "worker quick"}]}))
        r.send("warmup")
        until(lambda: r.on_page("answered: warmup"), within=8)
        time.sleep(36)
        t.close()
        r.kill_daemon(signals.SIGTERM)
        files = sorted(p for p in (r.root / "state").rglob("*") if p.is_file() and p.stat().st_size < 4_000_000 and p.suffix not in (".lock",))
        see("state files", files=[str(p.relative_to(r.root / "state")) for p in files])
        saved = r.root / "state.saved"
        shutil.copytree(r.root / "state", saved)
        for p in files:
            rel = p.relative_to(r.root / "state")
            if rel.parts[0] == "files" or "site" in rel.parts[0]:
                continue
            for form in ("garbage", "half", "empty"):
                shutil.rmtree(r.root / "state")
                shutil.copytree(saved, r.root / "state")
                b = p.read_bytes()
                p.write_bytes({"garbage": b"\xff\x00garbage{{{", "half": b[: len(b) // 2], "empty": b""}[form])
                started = time.monotonic()
                r.events.clear()
                r.start_daemon(wait=False)
                up = until(lambda: r.get("/api/host") is not None, within=6)
                alive = r.daemon.poll() is None
                ok = None
                if up:
                    threading.Thread(target=r.listen, daemon=True).start()
                    time.sleep(0.3)
                    r.send("probe " + form)
                    ok = timed(lambda: r.on_page("answered: probe"), 6)
                see("corrupt", file=str(rel), form=form, up=up, alive=alive, answered=ok, log=[l[:160] for l in r.log(2)])
                if r.daemon.poll() is None:
                    r.kill_daemon(signals.SIGKILL)
    finally:
        r.close()


def login_gone():
    r = Rig("l-login", 9219)
    try:
        r.send("warmup")
        until(lambda: r.on_page("answered: warmup"), within=8)
        (r.root / "login-gone").touch()
        r.send("words while out")
        ended = timed(lambda: r.on_page("couldn't") or r.on_page("logged in"), 10)
        see("login gone", after=ended, shown=[s[:160] for s in r.shown()], signin=r.get("/api/signin"), where=outcome(r, "words while out"), log=r.log(3))
        t = r.tools()
        print(t.routine("routine while out"))
        time.sleep(35)
        see("routine while out", shown=[s[:160] for s in r.shown()], log=r.log(3))
        (r.root / "login-gone").unlink()
        r.send("after login")
        see("back", after=timed(lambda: r.on_page("answered: after login"), 10), memory=(r.get("/api/memory") or b"")[:300], summarizer=(r.get("/api/summarizer") or b"")[:300])
        t.close()
    finally:
        r.close()


def mcp_down():
    r = Rig("m-mcp", 9219)
    try:
        r.send("F-MCPDOWN words")
        until(lambda: not r.typing() and r.shown(), within=8)
        time.sleep(0.5)
        see("tool server failed at start", shown=r.shown(), kinds=sorted({e["type"] for e in r.events}), log=r.log(3))
    finally:
        r.close()


def queue_overflow():
    r = Rig("q-overflow", 9218)
    try:
        r.send("F-HANG first")
        until(lambda: r.reached("F-HANG first"), within=5)
        start = r.rss_kib()
        t = time.monotonic()
        codes = {}
        for i in range(3000):
            c = r.send(f"queued {i} " + "z" * 2000)
            codes[c] = codes.get(c, 0) + 1
        see("flood", seconds=round(time.monotonic() - t, 1), codes=codes, rss=(start, r.rss_kib()), read=len(r.read()))
        time.sleep(3)
        see("after flood", rss=r.rss_kib(), host=r.status("/api/host"), turn_size=len(r.task()["turn"]["shown"]),
            turn_file=(r.root / "state/turn.json").stat().st_size if (r.root / "state/turn.json").exists() else None, saved=r.count("user", "queued"))
        r.stop()
        see("stop", after=timed(lambda: not r.typing(), 30))
        time.sleep(2)
        r.send("afterwards")
        see("next", after=timed(lambda: r.on_page("answered: afterwards"), 60), shown=[s[:90] for s in r.shown()][-3:], rss=r.rss_kib(), read_tail=[s[-40:] for s in r.read()[-3:]])
    finally:
        r.close()


def sse_load():
    if load() > 12:
        return see("skipped: load", load=load())
    r = Rig("e-sse", 9211)
    try:
        import socket as sk
        def client(read):
            s = sk.create_connection(("127.0.0.1", r.port))
            s.setsockopt(sk.SOL_SOCKET, sk.SO_RCVBUF, 4096)
            s.sendall(f"GET /api/events HTTP/1.1\r\nHost: 127.0.0.1:{r.port}\r\nOrigin: http://127.0.0.1:{r.port}\r\n\r\n".encode())
            return s
        base = (r.fds(), r.rss_kib())
        many = [client(True) for _ in range(300)]
        time.sleep(1)
        see("300 idle clients", base=base, now=(r.fds(), r.rss_kib()), host=r.status("/api/host"))
        r.send("F-FAST words")
        ok = timed(lambda: r.on_page("fast done"), 60)
        see("big turn with 300 clients that never read", answered=ok, now=(r.fds(), r.rss_kib()), reader_events=len(r.events))
        time.sleep(5)
        alive = 0
        for s in many:
            s.setblocking(False)
            try:
                d = s.recv(1 << 20)
                while d:
                    d = s.recv(1 << 20)
                alive += 0
            except BlockingIOError:
                alive += 1
            except OSError:
                pass
        see("slow clients", kept_open=alive, of=len(many), fds=r.fds(), rss=r.rss_kib())
        for s in many:
            s.close()
        time.sleep(3)
        see("abandoned", fds=r.fds(), rss=r.rss_kib(), base=base)
    finally:
        r.close()


def uploads():
    r = Rig("u-upload", 9212)
    try:
        import concurrent.futures as cf
        base = r.rss_kib()
        size = 40 * 1024 * 1024
        body = b'--b\r\ncontent-disposition: form-data; name="file"; filename="p.bin"\r\ncontent-type: application/octet-stream\r\n\r\n' + bytes(size) + b"\r\n--b--\r\n"
        def up(_):
            req = urllib.request.Request(f"http://127.0.0.1:{r.port}/api/upload", data=body,
                headers={"content-type": "multipart/form-data; boundary=b", "origin": f"http://127.0.0.1:{r.port}"})
            try:
                with urllib.request.urlopen(req, timeout=60) as x:
                    return x.status
            except urllib.error.HTTPError as e:
                return e.code
        with cf.ThreadPoolExecutor(5) as ex:
            codes = list(ex.map(up, range(25)))
        see("25 x 40 MB", codes=sorted(set(codes)), rss_mib=(base // 1024, r.rss_kib() // 1024))
        time.sleep(310)
        see("after retention, no further upload", rss_mib=r.rss_kib() // 1024)
        up(0)
        time.sleep(1)
        see("after one more upload", rss_mib=r.rss_kib() // 1024)
    finally:
        r.close()


def workers(n, word):
    def run():
        if load() > 12:
            return see("skipped: load", load=load())
        r = Rig("w-" + word.lower(), 9217)
        try:
            t = r.tools()
            print(t.call("spawn", {"tasks": [{"task": f"worker {word} {i}"} for i in range(n)]})[:200])
            s0 = time.monotonic()
            time.sleep(35)
            kids = int(subprocess.run(f"pgrep -c -P {r.daemon.pid}", shell=True, capture_output=True, text=True).stdout or 0)
            see("workers started", children=kids, fds=r.fds(), rss=r.rss_kib(), load=load(), read=len(r.read()))
            r.send("typed among workers")
            see("typed", after=timed(lambda: r.on_page("answered: typed among workers"), 60), shown=len(r.shown()), read=len(r.read()))
            if word == "W-QUICK":
                reports = timed(lambda: len(r.shown()) >= n, 240)
                see("reports", all_in=reports, shown=len(r.shown()), took=round(time.monotonic() - s0))
            t.close()
        finally:
            r.close()
    return run


def soak():
    r = Rig("z-soak", 9216)
    try:
        t0 = time.monotonic()
        n = 0
        see("soak start", rss=r.rss_kib(), fds=r.fds())
        while time.monotonic() - t0 < 1500:
            if load() > 12:
                time.sleep(10)
                continue
            r.send(f"soak {n} " + "s" * 500, )
            until(lambda: not r.typing(), within=20)
            n += 1
            if n % 200 == 0:
                r.events.clear()
                see("soak", n=n, rss=r.rss_kib(), fds=r.fds(), state_kib=sum(p.stat().st_size for p in (r.root / "state").rglob("*") if p.is_file()) // 1024,
                    took=round(time.monotonic() - t0))
            time.sleep(0.05)
        see("soak end", n=n, rss=r.rss_kib(), fds=r.fds(), took=round(time.monotonic() - t0))
    finally:
        r.close()


ALL |= {"S-corrupt": corrupt_state, "L-login": login_gone, "M-mcp": mcp_down, "Q-overflow": queue_overflow, "E-sse": sse_load,
        "U-upload": uploads, "W-quick": workers(50, "W-QUICK"), "W-hang": workers(50, "W-HANG"), "Z-soak": soak}


def view_gap(delay):
    """The gap between turns: how long the next turn waits on the summarizer,
    and whether the person's words are safe during it."""
    def run():
        r = Rig(f"v-gap-{delay}", 9213)
        try:
            (r.root / "slow-summary").write_text(str(delay))
            pad = " ".join(f"padding{n}" for n in range(1500))
            for i in range(10):
                r.send(f"{pad} gap{i} words")
                t = timed(lambda: r.reached(f"gap{i} words"), 45)
                answered = timed(lambda: r.on_page(f"answered: gap{i}"), 5)
                saved_when_reached = r.count("user", f"gap{i}")
                see("gap", delay=delay, turn=i, sent_to_harness_after=t, answered=answered)
                time.sleep(0.3)
            r.send(f"{pad} killed in the gap")
            time.sleep(1)
            saved = r.count("user", "killed in the gap")
            r.kill_daemon(signals.SIGKILL)
            time.sleep(0.5)
            see("killed one second into the gap", saved_before=saved, saved_after=r.count("user", "killed in the gap"))
        finally:
            r.close()
    return run


def summarizer_hang():
    r = Rig("v-hang", 9213)
    try:
        (r.root / "slow-summary").write_text("3600")
        pad = " ".join(f"padding{n}" for n in range(1500))
        for i in range(7):
            r.send(f"{pad} gap{i} words")
            if i == 6:
                time.sleep(2)
                r.stop()
                see("stop in the gap", typing_after_3s=(time.sleep(3), r.typing())[1])
            see("hung summarizer", turn=i, after=timed(lambda: r.reached(f"gap{i} words"), 45), host=r.status("/api/host"), shown=len(r.shown()))
            time.sleep(1)
    finally:
        r.close()


ALL |= {"V-gap-8": view_gap(8), "V-gap-40": view_gap(40), "V-hang": summarizer_hang}


def pipe_full(count, size):
    """A hung harness that does not read, and a person who keeps writing."""
    def run():
        r = Rig(f"q-pipe-{count}", 9218)
        try:
            r.send("F-HANG first")
            until(lambda: r.reached("F-HANG first"), within=5)
            for i in range(count):
                r.send(f"typed {i} " + "z" * size)
            time.sleep(3)
            see("written", count=count, kib=count * size // 1024, saved=r.count("user", "typed "), typing=r.typing())
            r.stop()
            see("stop", after=timed(lambda: not r.typing(), 60), harness_alive=Path(f"/proc/{r.harness_pid()}/stat").exists())
            r.send("afterwards")
            see("next", after=timed(lambda: r.on_page("answered: afterwards"), 60), shown=[s[:80] for s in r.shown()][-3:], saved=r.count("user", "typed "), log=r.log(3), host=r.status("/api/host"))
            subprocess.run(f"cat /proc/{r.daemon.pid}/task/*/wchan | sort | uniq -c | sort -rn | head -6", shell=True)
        finally:
            r.close()
    return run


ALL |= {"Q-pipe-40": pipe_full(40, 3000), "Q-pipe-5": pipe_full(5, 30000)}


def fill(path):
    try:
        with open(path / "filler", "wb") as f:
            while True:
                f.write(bytes(4096))
                f.flush()
    except OSError:
        pass


def disk_full():
    """Run inside `unshare -Urm`: state and hippo each on a small tmpfs."""
    def prepare(r):
        for d in ("state", "hippo"):
            subprocess.run(["mount", "-t", "tmpfs", "-o", "size=64m", "tmpfs", str(r.root / d)], check=True)
    r = Rig("d-disk", 9209, prepare=prepare)
    try:
        r.send("warmup")
        until(lambda: r.on_page("answered: warmup"), within=8)
        time.sleep(0.3)
        for where in ("state", "hippo"):
            fill(r.root / where)
            r.send(f"full {where} one")
            ended = timed(lambda: not r.typing() and any(("full " + where) in s or "couldn't" in s for s in r.shown()), 10)
            see("disk full", where=where, ended=ended, shown=[s[:150] for s in r.shown()][-2:], log=[l[:150] for l in r.log(3)],
                host=r.status("/api/host"), hist=r.status("/api/history"), task=page(r)["typing"])
            see("kept", where=where, c=outcome(r, f"full {where} one"))
            (r.root / where / "filler").unlink()
            time.sleep(1)
            r.send(f"freed {where}")
            see("freed", where=where, after=timed(lambda: r.on_page(f"answered: freed {where}"), 10), c=outcome(r, f"freed {where}"), log=[l[:150] for l in r.log(2)])
        # Restart with the state disk full: turn.json cannot be kept, the book cannot open for writes.
        fill(r.root / "state")
        r.kill_daemon(signals.SIGTERM)
        r.events.clear()
        r.start_daemon(wait=False)
        up = until(lambda: r.get("/api/host") is not None, within=8)
        see("restart on a full state disk", up=up, alive=r.daemon.poll() is None, log=[l[:150] for l in r.log(3)])
        if up:
            threading.Thread(target=r.listen, daemon=True).start()
            time.sleep(0.3)
            r.send("restart full")
            see("answered", after=timed(lambda: r.on_page("answered: restart full"), 8))
    finally:
        r.close()


def disk_full_large():
    """As disk_full, with words too large for the slack in a file's last page."""
    def prepare(r):
        subprocess.run(["mount", "-t", "tmpfs", "-o", "size=64m", "tmpfs", str(r.root / "hippo")], check=True)
    r = Rig("d-large", 9209, prepare=prepare)
    try:
        r.send("warmup")
        until(lambda: r.on_page("answered: warmup"), within=8)
        fill(r.root / "hippo")
        big = "F-FAST large " + "word " * 5000
        r.send(big)
        ended = timed(lambda: not r.typing() and len(r.shown()) > 1, 12)
        see("hippo disk full, 25 KB of words", ended=ended, shown=[s[:150] for s in r.shown()][-2:],
            hippo_log=(r.root / "hippo.log").read_text()[-300:], libri_log=[l[:200] for l in r.log(3)],
            alive=r.hippo.poll() is None, host=r.status("/api/host"), hist=r.status("/api/history"))
        see("kept", c=outcome(r, "F-FAST large"))
        (r.root / "hippo/filler").unlink()
        r.send("after freeing")
        see("freed", after=timed(lambda: r.on_page("answered: after freeing"), 10), shown=[s[:100] for s in r.shown()][-2:])
        r.signal(r.hippo.pid, signals.SIGTERM)
        r.hippo.wait()
        r.start_hippo()
        see("after hippo restart", c=outcome(r, "F-FAST large", "after freeing", "warmup"))
    finally:
        r.close()


def stop_races():
    r = Rig("s-stop", 9203)
    try:
        r.stop()
        time.sleep(0.2)
        r.send("after idle stop")
        see("Stop with no turn, then words", after=timed(lambda: r.on_page("answered: after idle stop"), 6))
        for gap in (0, 0.005, 0.05, 0.3):
            word = f"race {gap}"
            r.send(word)
            time.sleep(gap)
            r.stop()
            ended = timed(lambda: not r.typing(), 6)
            time.sleep(0.5)
            see("Stop right after sending", gap=gap, ended=ended, tail=[s[:60] for s in r.shown()][-1:], c=outcome(r, word))
        # Words queued behind a turn when Stop comes.
        r.send("F-SLOW3 first")
        until(lambda: r.reached("F-SLOW3"), within=5)
        r.send("behind one")
        r.send("behind two")
        time.sleep(0.3)
        r.stop()
        time.sleep(2)
        see("Stop with words behind the turn", typing=r.typing(), tail=[s[:60] for s in r.shown()][-3:], c=outcome(r, "behind one", "behind two"))
    finally:
        r.close()


def same_id():
    r = Rig("i-id", 9202)
    try:
        codes = []
        def post(c):
            codes.append(r.send("twice", id="same-id"))
        ts = [threading.Thread(target=post, args=(i,)) for i in range(2)]
        [t.start() for t in ts]
        [t.join() for t in ts]
        time.sleep(1.5)
        c = r.chat() or []
        see("same id from two clients at once", codes=codes, saved=sum(1 for m in c if m["kind"] == "user" and m["text"] == "twice"),
            harness=sum(1 for e in r.read() if "twice" in json.dumps(e)))
        # Web and Android typing at once through a slow turn; a second listener.
        threading.Thread(target=r.listen, daemon=True).start()
        r.send("F-SLOW2 opener")
        until(lambda: r.reached("F-SLOW2"), within=5)
        def client(name):
            for i in range(10):
                r.send(f"{name} {i}")
                time.sleep(0.05)
        ts = [threading.Thread(target=client, args=(n,)) for n in ("web", "android")]
        [t.start() for t in ts]
        [t.join() for t in ts]
        until(lambda: not r.typing(), within=20)
        time.sleep(1)
        words = [f"{n} {i}" for n in ("web", "android") for i in range(10)]
        where = outcome(r, *words)
        see("two clients through a turn", saved=sum(v["hippo_user"] >= 1 for v in where.values()), reached=sum(v["harness"] for v in where.values()),
            typing=r.typing(), shown_end=[s[-80:] for s in r.shown()][-2:])
    finally:
        r.close()


def turn_record_midway():
    r = Rig("m-record", 9201)
    try:
        r.send("F-SLOW3 long turn")
        until(lambda: r.reached("F-SLOW3"), within=5)
        f = r.root / "state/site/turn.json"
        for kind, data in (("garbage", b"\x00{{{"), ("half", f.read_bytes()[:20])):
            f.write_bytes(data)
            time.sleep(0.3)
            see("turn record overwritten midway", kind=kind, task=r.status("/api/task"), hist=r.status("/api/history"))
        ended = timed(lambda: not r.typing(), 8)
        see("turn ended", ended=ended, shown=[s[:60] for s in r.shown()][-1:])
        r.send("F-HANG words")
        until(lambda: r.reached("F-HANG"), within=5)
        f.write_bytes(b"\x00{{{")
        r.kill_daemon(signals.SIGKILL)
        r.events.clear()
        r.start_daemon()
        r.send("after")
        see("restart with a broken turn record", after=timed(lambda: r.on_page("answered: after"), 8), log=[l[:150] for l in r.log(2)])
    finally:
        r.close()


def huge_line():
    r = Rig("h-huge", 9200)
    try:
        r.send("F-HUGE words")
        t = timed(lambda: not r.typing() and bool(r.shown()), 30)
        see("very long output line", ended=t, shown=[(s[:60], len(s)) for s in r.shown()], rss=r.rss_kib())
    finally:
        r.close()



def rollback(older):
    """New state read by an older build, then the new build again."""
    def run():
        r = Rig("b-rollback", 9208)
        try:
            t = r.tools()
            print(t.call("watch", {"when": {"at": time.strftime("%Y-%m-%d", time.localtime(time.time() + 86400 * 9)) + " 09:00"}, "then": {"note": "keeps a date"}}))
            print(t.call("spawn", {"tasks": [{"task": "worker quick"}]}))
            r.send("warmup")
            until(lambda: r.on_page("answered: warmup"), within=8)
            time.sleep(1)
            t.close()
            r.kill_daemon(signals.SIGTERM)
            r.signal(r.hippo.pid, signals.SIGTERM)
            r.hippo.wait()
            new = r.build
            for build, label in ((Path(older), "older"), (new, "newer again")):
                r.build = build
                r.env["PATH"] = f"{build}/bin:{os.environ['PATH']}"
                r.start_hippo()
                r.events.clear()
                r.start_daemon(wait=False)
                up = timed(lambda: r.get("/api/host") is not None, 40)
                see("started", label=label, up=up, alive=r.daemon.poll() is None, log=[l[:200] for l in r.log(3)], hippo_log=(r.root / "hippo.log").read_text()[-200:])
                if up:
                    threading.Thread(target=r.listen, daemon=True).start()
                    time.sleep(0.4)
                    r.send(f"probe {label}")
                    see("answered", label=label, after=timed(lambda: r.on_page(f"answered: probe {label}"), 10), shown=[s[:100] for s in r.shown()][-2:],
                        tail=[m["text"][:30] for m in (r.chat() or [])][-4:])
                if r.daemon.poll() is None:
                    r.kill_daemon(signals.SIGTERM)
                r.signal(r.hippo.pid, signals.SIGTERM)
                r.hippo.wait()
        finally:
            r.close()
    return run


def config_unknown():
    r = Rig("b-config", 9207)
    try:
        cfg = r.root / "xdg/config/libri"
        cfg.mkdir(parents=True)
        (cfg / "config.toml").write_text('[daemon]\norigins = ["https://a"]\nnewer_setting = 1\n')
        r.kill_daemon(signals.SIGTERM)
        r.start_daemon(wait=False)
        up = until(lambda: r.get("/api/host") is not None, within=6)
        see("unknown config key at start", up=up, alive=r.daemon.poll() is None, exit=r.daemon.poll(), log=[l[:200] for l in r.log(2)])
    finally:
        r.close()


def holdout_recovers():
    """How long a child holding stdout keeps the chat stopped."""
    r = Rig("h-holdout-time", 9206)
    try:
        r.send("F-HOLDOUT words")
        until(lambda: r.on_page("answered before leaving"), within=5)
        r.send("typed")
        t = timed(lambda: not r.typing(), 100)
        time.sleep(1)
        see("holdout", chat_stopped_for=t, shown=[s[:80] for s in r.shown()], where=outcome(r, "F-HOLDOUT", "typed", "answered before leaving"))
    finally:
        r.close()


def sigterm_time():
    r = Rig("t-term", 9205)
    try:
        r.send("F-DEAF words")
        until(lambda: r.reached("F-DEAF"), within=5)
        hp = r.harness_pid()
        t = time.monotonic()
        r.kill_daemon(signals.SIGTERM)
        see("SIGTERM with a stubborn harness", exit_took=round(time.monotonic() - t, 2), harness_left=Path(f"/proc/{hp}/stat").exists(), code=r.daemon.returncode)
    finally:
        r.close()


def self_hippo_dies():
    """Libri serving its own hippo, as a second assistant on a host does: nobody restarts it."""
    r = Rig("x-selfserve", 9204)
    try:
        r.kill_daemon(signals.SIGTERM)
        r.signal(r.hippo.pid, signals.SIGKILL)
        r.hippo.wait()
        del r.env["HIPPO_DIR"]
        r.env.pop("HIPPO_ARCHIVE")
        r.start_daemon()
        up = r.status("/api/host")
        hdir = r.root / "state/hippo"
        kids = subprocess.run(f"pgrep -P {r.daemon.pid} -a", shell=True, capture_output=True, text=True).stdout
        see("self-served", up=up, kids=kids[:200], sock=(hdir / "hippo.sock").exists(), sockets=own_socket_paths(r.daemon.pid))
        hp = int(kids.split()[0])
        r.send("before")
        see("before", after=timed(lambda: r.on_page("answered: before"), 8))
        r.signal(hp, signals.SIGKILL)
        time.sleep(40)
        r.send("after hippo died")
        see("after", shown=[s[:120] for s in r.shown()], alive=r.daemon.poll() is None, typing=(time.sleep(2), r.typing())[1])
    finally:
        r.close()


ALL |= {"I-stop": stop_races, "I-id": same_id, "M-record": turn_record_midway, "H-huge": huge_line, "D-disk": disk_full, "D-large": disk_full_large, "B-config": config_unknown, "H-holdout": holdout_recovers, "T-term": sigterm_time, "X-self": self_hippo_dies}
if os.environ.get("OLDER"):
    ALL["B-rollback"] = rollback(os.environ["OLDER"])


def hippo_frozen_long():
    """Hippo frozen past the turn's bounded wait: before the turn, then during it."""
    r = Rig("hippo-frozen-long", 9212)
    try:
        r.send("warmup")
        until(lambda: r.on_page("answered: warmup"), within=10)
        time.sleep(0.3)
        pid = r.hippo.pid
        r.signal(pid, signals.SIGSTOP)
        r.send("frozen one")
        ended = timed(lambda: r.on_page("couldn't"), 80)
        see("frozen before the turn", ended_after=ended, typing=r.typing(), shown=[s[:140] for s in r.shown()], log=r.log(3))
        r.signal(pid, signals.SIGCONT)
        r.send("thawed")
        ok = timed(lambda: r.on_page("answered: thawed"), 15)
        time.sleep(1)
        r.send("F-SLOW3 mid words")
        until(lambda: r.reached("mid words"), within=10)
        r.signal(pid, signals.SIGSTOP)
        ended = timed(lambda: not r.typing(), 130)
        see("frozen during the turn", ended_after=ended, shown=[s[:140] for s in r.shown()], log=r.log(3))
        r.signal(pid, signals.SIGCONT)
        time.sleep(1)
        see("after", next_answer_earlier=ok, where=outcome(r, "frozen one", "thawed", "mid words"))
    finally:
        if r.hippo.poll() is None:
            try:
                os.kill(r.hippo.pid, signals.SIGCONT)
            except OSError:
                pass
        r.close()


ALL["HIPPO-frozen-long"] = hippo_frozen_long


QUICK = {"LIBRI_SILENCE_SECS": "3"}


def notices(r):
    return [s[:140] for s in r.shown() if "may be stuck" in s]


def turn_silent():
    """A hung turn with words typed into it: heard, and Stop keeps the words."""
    r = Rig("t-silent", 9201, env=QUICK)
    try:
        r.send("F-HANG first words")
        until(lambda: r.reached("F-HANG"), within=5)
        r.send("typed into the hang")
        heard = timed(lambda: notices(r), 8)
        time.sleep(1)
        see("hung turn", heard_after=heard, notices=notices(r), typing=r.typing())
        r.stop()
        see("stop", after=timed(lambda: not r.typing(), 8))
        time.sleep(1)
        see("end", shown=[s[:120] for s in r.shown()], where=outcome(r, "first words", "typed into the hang"))
    finally:
        r.close()


def turn_quiet_kinds():
    """What must not be called stuck, and a hung tool, which must."""
    r = Rig("t-kinds", 9202, env=QUICK)
    try:
        for word, within in (("F-ASK", 7), ("F-BGJOB", 7), ("F-TOOLBUSY", 22), ("F-SLOW6", 8)):
            r.events.clear()
            r.send(f"{word} kind")
            until(lambda: r.reached(word), within=5)
            time.sleep(within)
            see("long silence", word=word, typing=r.typing(), notices=notices(r), shown=[s[:80] for s in r.shown()])
            (r.root / "release").touch()
            r.stop()
            until(lambda: not r.typing(), within=8)
            (r.root / "release").unlink()
        r.events.clear()
        r.send("F-TOOLHANG kind")
        until(lambda: r.reached("F-TOOLHANG"), within=5)
        see("hung tool", heard_after=timed(lambda: notices(r), 22), notices=notices(r))
        r.stop()
    finally:
        r.close()


def turn_stop_waiting():
    """Words waiting behind a turn that has not opened to them when Stop comes."""
    r = Rig("t-waiting", 9203)
    try:
        (r.root / "hold-start").touch()
        r.send("first")
        until(lambda: (r.root / "started").exists(), within=5)
        r.send("behind one")
        r.send("behind two")
        time.sleep(0.5)
        r.stop()
        (r.root / "hold-start").unlink()
        until(lambda: not r.typing(), within=8)
        time.sleep(1.5)
        see("Stop with two waiting", shown=[s[:100] for s in r.shown()],
            where=outcome(r, "behind one", "behind two"),
            memory=[(m["kind"], m["text"][:90]) for m in r.chat()[-5:]])
    finally:
        r.close()


def turn_error_kept():
    """A failed turn's error reaches the memory, so the next turn knows."""
    r = Rig("t-error", 9204)
    try:
        r.send("F-CRASH words")
        until(lambda: r.on_page("couldn't answer"), within=8)
        time.sleep(0.5)
        see("error", shown=[s[:100] for s in r.shown()], memory=[(m["kind"], m["text"][:90]) for m in r.chat()[-3:]])
    finally:
        r.close()


def turn_question_words():
    """Words typed as the answer to an open question are the person's words."""
    r = Rig("t-question", 9205)
    try:
        r.send("F-ASK words")
        until(lambda: r.task() and r.reached("F-ASK"), within=5)
        time.sleep(0.5)
        r.send("typed as the answer")
        until(lambda: r.on_page("after the question"), within=8)
        time.sleep(0.5)
        see("typed answer", saved=r.count("user", "typed as the answer"), read=[s[-60:] for s in r.read()])
    finally:
        r.close()


def turn_kill_ask(sig=signals.SIGKILL):
    """Killed with a question open: nothing to resume, and the person hears."""
    r = Rig("t-kill-ask", 9206)
    try:
        r.send("F-ASK W1 words")
        until(lambda: r.reached("F-ASK"), within=5)
        time.sleep(0.5)
        r.kill_daemon(sig)
        r.events.clear()
        r.start_daemon()
        time.sleep(3)
        see("killed on a question", shown=[s[:140] for s in r.shown()], typing=r.typing(),
            read=[s[-40:] for s in r.read()], memory=[(m["kind"], m["text"][:120]) for m in r.chat()[-3:]])
    finally:
        r.close()


def turn_kill_loop():
    """A turn that hangs each time: picked up three times, not a fourth."""
    r = Rig("t-kill-loop", 9207)
    try:
        r.send("F-HANG W1 words")
        until(lambda: r.reached("W1"), within=5)
        for n in range(5):
            r.kill_daemon(signals.SIGKILL)
            r.events.clear()
            r.start_daemon()
            time.sleep(2.5)
            see("restart", n=n + 1, starts=sum("W1" in s for s in r.read()), typing=r.typing(),
                memory=[(m["kind"], m["text"][:120]) for m in r.chat()[-2:]])
    finally:
        r.close()


def turn_kill_routine():
    """Killed during a routine: it comes round again, and the person hears."""
    r = Rig("t-kill-routine", 9208)
    try:
        t = r.tools()
        print(t.routine("F-SLOW40 routine words"))
        see("routine began", after=wait_tick(r, "routine words"))
        t.close()
        r.kill_daemon(signals.SIGKILL)
        r.events.clear()
        r.start_daemon()
        time.sleep(3)
        see("killed on a routine", shown=[s[:140] for s in r.shown()], typing=r.typing(),
            memory=[(m["kind"], m["text"][:120]) for m in r.chat()[-3:]])
    finally:
        r.close()


def turn_kill_waiting(sig=signals.SIGKILL):
    """Killed with a turn cut off and words behind it: all of it runs, in order."""
    r = Rig("t-kill-waiting", 9209)
    try:
        (r.root / "hold-start").touch()
        r.send("first words")
        until(lambda: (r.root / "started").exists(), within=5)
        r.send("behind one")
        r.send("behind two")
        time.sleep(0.5)
        r.kill_daemon(sig)
        (r.root / "hold-start").unlink()
        r.events.clear()
        r.start_daemon()
        time.sleep(4)
        see("words behind a killed turn", where=outcome(r, "first words", "behind one", "behind two"),
            typing=r.typing(), read=[s[-60:] for s in r.read()],
            memory=[(m["kind"], m["text"][:120]) for m in r.chat()[-7:]])
    finally:
        r.close()


ALL.update({"T-silent": turn_silent, "T-kinds": turn_quiet_kinds, "T-waiting": turn_stop_waiting,
            "T-error": turn_error_kept, "T-question": turn_question_words, "T-kill-ask": turn_kill_ask,
            "T-kill-loop": turn_kill_loop, "T-kill-routine": turn_kill_routine, "T-kill-waiting": turn_kill_waiting})


if __name__ == "__main__":
    for name in sys.argv[1:] or list(ALL):
        ALL[name]()
