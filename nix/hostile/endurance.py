"""The suite run_endurance_test.sh runs inside its VM: first that the VM is
sealed, then one instance born on the previous build, filled with made-up
data of the real shape, upgraded to this build and rolled back again, then
held on this build under steady turns and broken on purpose every few
minutes, then a load of workers. Every word sent goes in a ledger, which is
checked against hippo at the end. Writes events.jsonl and samples.jsonl to
ENDURANCE_OUTPUT as it goes, so a run cut short still reports."""

import collections, hashlib, re, sqlite3, subprocess, traceback

from rig import *

OUTPUT = Path(os.environ["ENDURANCE_OUTPUT"])
PLAN = json.loads((OUTPUT / "plan.json").read_text())
PREVIOUS = Path(os.environ["LIBRI_PREVIOUS"])
PORT = 9216
DISK_MIB = 1024
# What people send: every shape keeps its word on the last line, where the
# stand-in reads it, and no other word that looks like one.
SHAPES = [
    "{w} is the pharmacy still open on Sunday",
    "Here is the list:\n- eggs\n- rye flour\n- two lemons\n\n{w} thanks",
    "{w} Zoë sent 300 € for the café, 東京 in May 🌸 – can you put it down",
    "```python\nfor n in range(3):\n    print(n)\n```\n{w} why does this stop early",
    "{w} " + "a long letter about the roof and the gutters " * 40,
    "{w} see https://example.org/a?b=1&c=<d> and \"quoted\" words",
]
STEERED = "(sent while you were working) "
ANSWER = re.compile(r"(?:answered|steered): (?:" + re.escape(STEERED) + r")?(?:F-\w+ )?(w\d+)\b")
# What Libri puts before a word it runs again after a restart: a second row
# in hippo by design, not the person saying it twice.
RESUMED = "(a restart cut"
# Words in Libri's log that mean it could not read what it found.
COMPLAINT = re.compile(r"panicked|corrupt|malformed|cannot|could not|couldn't|unreadable|unknown|invalid|migrat|parse|failed", re.I)


def event(kind, **fields):
    with (OUTPUT / "events.jsonl").open("a") as f:
        f.write(json.dumps({"at": time.time(), "kind": kind} | fields, default=str) + "\n")


def reachable(host, port):
    try:
        socket.create_connection((host, port), timeout=3).close()
        return True
    except OSError:
        return False


def check_sealed():
    """What the VM must not reach. The canary is a listener on the host's
    loopback, opened for this run: a hit there is a hole in the seal."""
    seen = {
        "canary_on_host": reachable("10.0.2.2", PLAN["canary_port"]),
        "host_ssh": reachable("10.0.2.2", 22),
        "internet": reachable("1.1.1.1", 443),
        "peer_env": sorted(k for k in os.environ if k.startswith("LIBRI_A2A")),
        "store_paths": len(os.listdir("/nix/store")),
        "home_dirs": os.listdir("/home") if os.path.exists("/home") else [],
    }
    event("sealed", **seen)
    return not (seen["canary_on_host"] or seen["host_ssh"] or seen["internet"] or seen["peer_env"])


def small_disks(r):
    """State and hippo each on a real filesystem of its own, so a full disk
    fails as one does, with no blocks reserved for root."""
    for d in ("state", "hippo"):
        image = r.root.parent / f"{d}.img"
        subprocess.run(["truncate", "-s", f"{DISK_MIB}M", str(image)], check=True)
        subprocess.run(["mkfs.ext4", "-q", "-F", "-m", "0", str(image)], check=True)
        subprocess.run(["mount", "-o", "loop", str(image), str(r.root / d)], check=True)


class SoakRig(Rig):
    """A rig that remembers only which words were answered: its stream runs
    for hours, and the base class keeps every event."""

    def __init__(self, *args, **kw):
        self.answered = set()
        self.last_event = 0.0
        super().__init__(*args, **kw)

    def listen(self):
        try:
            with self.request("/api/events", timeout=None) as stream:
                self.listeners += 1
                for line in stream:
                    if line.startswith(b"data: "):
                        self.last_event = time.monotonic()
                        self.answered.update(ANSWER.findall(line.decode(errors="replace")))
        except Exception:
            pass

    def full_chat(self):
        """Every message in hippo, from its files: `tail` stops at 500."""
        rows = []
        for day in sorted((self.root / "hippo/main").glob("*.jsonl")):
            for line in day.read_text(errors="replace").splitlines():
                try:
                    rows.append(json.loads(line))
                except ValueError:
                    pass  # A line being written.
        return rows


class Soak:
    def __init__(self):
        self.r = SoakRig("soak", PORT, build=PREVIOUS, prepare=small_disks)
        self.n = 0
        # word -> the fault it was sent into, or None.
        self.ledger = {}
        self.sent_order = []
        self.lost_to_refusal = 0
        self.send_status = None
        # Typed words seen twice in hippo, each steered into a turn a restart
        # cut: steered() saves the word before pour clears the journal, so a
        # restart between the two runs it again. Accepted, not keyed away.
        self.saved_twice = set()
        # word -> why no answer can come: the stand-in never answers F-HANG.
        self.silent = {}
        self.latencies = []
        self.turns = 0
        self.timeouts = 0
        self.fault_name = None
        self.shape_of = {}

    def word(self):
        self.n += 1
        return f"w{self.n}"

    def send(self, text, word, timeout=10):
        status = self.send_status = self.r.send(text)
        if status == 202:
            self.ledger[word] = self.fault_name
            self.sent_order.append(word)
        else:
            self.lost_to_refusal += 1
        return status

    def turn(self, within=60, shape="{w} " + "s" * 500):
        """One plain turn. Answers seconds, or None if no answer came."""
        w = self.word()
        t = time.monotonic()
        status = self.send(shape.format(w=w), w)
        if status != 202:
            return None
        if not until(lambda: w in self.r.answered, within=within):
            return None
        return round(time.monotonic() - t, 3)

    def after_fault(self):
        """The next turn: the promise that matters most."""
        t = self.turn(within=90)
        if t is not None:
            return {"next_turn": t}
        typing = self.r.typing()
        self.r.stop()
        t = self.turn(within=60)
        return {"next_turn": t, "needed_stop": True, "typing_then": typing}

    def hold(self, word, within=30):
        """Whether a word is in hippo as the person's, once."""
        counts = []
        end = time.monotonic() + within
        while time.monotonic() < end:
            rows = self.r.chat() if self.r.hippo.poll() is None else None
            c = None if rows is None else sum(1 for m in rows if m["kind"] == "user" and word + " " in m["text"] and not m["text"].startswith(RESUMED))
            counts.append(c)
            if c == 1:
                return 1
            time.sleep(0.5)
        return counts[-1] if counts else None

    def stuck(self):
        """What a turn that gave no answer was waiting on."""
        r = self.r
        record = r.root / "state/open-turn.json"
        ps = subprocess.run(["ps", "-eo", "pid,ppid,etimes,args"], capture_output=True, text=True).stdout
        return {"task": r.task(), "read": [s[-120:] for s in r.read()[-6:]], "open_turn": record.read_text()[:500] if record.exists() else None,
                "harnesses": [l[:80] for l in ps.splitlines() if str(r.claude) in l][:10]}

    def in_flight(self, slow=8):
        w = self.word()
        self.send(f"F-SLOW{slow} {w} words", w)
        until(lambda: self.r.reached(w), within=15)
        return w

    def restart_all(self):
        r = self.r
        for p, starter in ((r.daemon, None), (r.hippo, None)):
            if p.poll() is None:
                r.signal(p.pid, signals.SIGKILL)
                p.wait()
        r.start_hippo()
        r.start_daemon()

    # Moving between builds. Each side writes a share of the data in the normal
    # way, so it is what that build really writes; nothing is hand-made.

    def threads(self):
        got = self.r.get("/api/work")
        return json.loads(got)["threads"] if got else []

    def write_data(self, turns, workers):
        r = self.r
        answered = sum(self.turn(shape=SHAPES[n % len(SHAPES)]) is not None for n in range(turns))
        known = len(self.threads())
        errands = [f"errand {i}, look over the accounts" for i in range(workers)] + ["W-HANG errand, wait on the supplier"]
        t = r.tools()
        spawned = t.call("spawn", {"tasks": [{"task": e} for e in errands]})
        if "deserialize" in spawned:
            # Builds before the per-worker choices took plain strings.
            spawned = t.call("spawn", {"tasks": errands})
        t.close()
        until(lambda: len(self.threads()) >= known + workers + 1, within=60)
        self.settle()
        return {"turns_answered": answered, "of": turns, "threads": len(self.threads()), "spawned": spawned[:300],
                "work_status": r.status("/api/work"), "task_dirs": len(os.listdir(r.root / "state/task")) if (r.root / "state/task").exists() else 0}

    def settle(self):
        """Until workers have reported and the answers to their reports are in."""
        last, since = None, time.monotonic()
        end = since + 180
        while time.monotonic() < end:
            rows = len(self.r.full_chat())
            if rows != last or self.r.typing():
                last, since = rows, time.monotonic()
            elif time.monotonic() - since > 6:
                return
            time.sleep(1)

    def snapshot(self):
        logs = {}
        for t in self.threads():
            got = self.r.get(f"/api/work/{t['name']}")
            logs[t["name"]] = json.loads(got)["text"] if got else None
        return {"rows": [(m["kind"], m["text"]) for m in self.r.full_chat()], "logs": logs,
                "history": self.r.status("/api/history")}

    def shape(self):
        """What is on disk, by kind: for each file its keys or schema. Read
        with both stopped. Comparing these across builds names a format change."""
        names = [t for t in os.listdir(self.r.root / "state" / "task")] if (self.r.root / "state/task").exists() else []
        out = {}
        for d in ("state", "hippo"):
            for p in sorted((self.r.root / d).rglob("*")):
                if not p.is_file() or "filler" in p.name or "lost+found" in p.parts:
                    continue
                kind = str(p.relative_to(self.r.root))
                for n in names:
                    kind = kind.replace(n, "*")
                kind = re.sub(r"[0-9a-f]{8,}", "*", kind)
                out.setdefault(kind, set()).update(keys_of(p))
        return {k: sorted(v) for k, v in out.items()}

    def cross(self, label, build):
        """Moves the instance to another build with a turn under way, as a
        deploy that does not wait would, and checks nothing was lost."""
        r = self.r
        before = self.snapshot()
        cut = self.in_flight()
        typed = self.word()
        self.send(f"{typed} typed as the build changed", typed)
        r.kill_daemon(signals.SIGTERM)
        r.signal(r.hippo.pid, signals.SIGTERM)
        r.hippo.wait(timeout=30)
        shape = self.shape()
        self.shape_of[r.build] = shape
        open_turn = (r.root / "state/open-turn.json").exists()
        held = len(os.listdir(r.root / "state/site/inbox")) if (r.root / "state/site/inbox").exists() else 0
        log_at = (r.root / "libri.log").stat().st_size
        r.build = Path(build)
        r.env["PATH"] = f"{r.build}/bin:{os.environ['PATH']}"
        r.start_hippo()
        r.start_daemon()
        next_turn = self.turn(within=90)
        stuck = self.stuck() if next_turn is None else None
        self.settle()
        after = self.snapshot()
        lost_rows = collections.Counter(before["rows"]) - collections.Counter(after["rows"])
        rest = iter(after["rows"])
        in_order = all(row in rest for row in before["rows"])
        lost_threads = [n for n in before["logs"] if n not in after["logs"]]
        shortened = [n for n, log in before["logs"].items()
                     if log and n in after["logs"] and not (after["logs"][n] or "").startswith(log)]
        kept = {w: self.hold(w) for w in (cut, typed)}
        if kept[typed] == 2 and r.reached(f"{STEERED}{typed} "):
            self.saved_twice.add(typed)
        with open(r.root / "libri.log", "rb") as f:
            f.seek(log_at)
            complaints = [l[:220] for l in f.read().decode(errors="replace").splitlines() if COMPLAINT.search(l)]
        broke = []
        if next_turn is None:
            broke.append("no answer after the change")
        if lost_rows:
            broke.append(f"{sum(lost_rows.values())} hippo rows lost")
        if not in_order:
            broke.append("hippo rows out of order")
        if lost_threads or shortened:
            broke.append(f"worker chats lost {lost_threads}, shortened {shortened}")
        if after["history"] != 200:
            broke.append(f"history answers {after['history']}")
        if kept[cut] != 1 or not (kept[typed] == 1 or typed in self.saved_twice):
            broke.append(f"words not in hippo once: {kept}")
        event("crossing", label=label, broke=broke, next_turn=next_turn, rows_before=len(before["rows"]),
              rows_after=len(after["rows"]), threads_before=len(before["logs"]), threads_after=len(after["logs"]),
              open_turn_found=open_turn, inbox_held=held, kept=kept, cut_answered=cut in r.answered,
              typed_answered=typed in r.answered, complaints=complaints[:20], stuck=stuck)

    def format_changes(self):
        """What this build writes that the previous does not, or the reverse."""
        old, new = self.shape_of.get(PREVIOUS, {}), self.shape_of.get(BUILD, {})
        event("formats", same_build=PREVIOUS == BUILD,
              only_previous={k: old[k] for k in old.keys() - new.keys()},
              only_dev={k: new[k] for k in new.keys() - old.keys()},
              changed={k: {"previous": old[k], "dev": new[k]} for k in old.keys() & new.keys() if old[k] != new[k]})

    def crossings(self):
        """Born on the previous build; upgraded; written to by this build;
        rolled back, which must read what this build wrote; forward again,
        for the soak. Rollback is what a deploy tool does on its own."""
        seed = PLAN.get("seed_turns", 120)
        try:
            event("written", on="previous", **self.write_data(seed, 6))
            self.cross("upgrade to dev", BUILD)
            event("written", on="dev", **self.write_data(seed // 2, 4))
            self.cross("rollback to previous", PREVIOUS)
            event("written", on="previous after rollback", **self.write_data(seed // 6, 2))
            self.format_changes()
            self.cross("forward to dev", BUILD)
        except (Exception, SystemExit):
            event("crossing failed", why=traceback.format_exc()[-800:], log=[l[:200] for l in self.r.log(6)])
            self.r.build = BUILD
            self.r.env["PATH"] = f"{BUILD}/bin:{os.environ['PATH']}"
            self.restart_all()

    # The faults. Each breaks something, puts it right as a person or systemd
    # would, and answers what it saw; `words` are those that must stay in hippo.

    def libri_killed(self, sig):
        w = self.in_flight()
        self.r.kill_daemon(sig)
        self.r.start_daemon()
        return {"words": [w], "cut_turn_answered": until(lambda: w in self.r.answered, within=30)}

    def hippo_killed(self):
        r = self.r
        w = self.in_flight()
        r.signal(r.hippo.pid, signals.SIGKILL)
        r.hippo.wait()
        ended = until(lambda: not r.typing(), within=30)
        r.start_hippo()
        return {"words": [w], "turn_ended_without_hippo": ended, "cut_turn_answered": w in r.answered}

    def both_killed(self):
        r = self.r
        w = self.in_flight()
        r.kill_daemon(signals.SIGKILL)
        r.signal(r.hippo.pid, signals.SIGKILL)
        r.hippo.wait()
        r.start_hippo()
        r.start_daemon()
        return {"words": [w], "cut_turn_answered": until(lambda: w in r.answered, within=30)}

    def both_restarted(self):
        r = self.r
        r.kill_daemon(signals.SIGTERM)
        r.signal(r.hippo.pid, signals.SIGTERM)
        r.hippo.wait(timeout=30)
        r.start_hippo()
        r.start_daemon()
        return {}

    def harness_killed(self):
        r = self.r
        w = self.word()
        self.send(f"F-HANG {w} words", w)
        until(lambda: r.reached(w), within=15)
        pid = r.harness_pid()
        r.signal(pid, signals.SIGKILL)
        self.silent[w] = "its harness was killed"
        return {"words": [w], "turn_ended": until(lambda: not r.typing(), within=30)}

    def harness_hung(self):
        r = self.r
        w, typed = self.word(), self.word()
        self.send(f"F-HANG {w} words", w)
        until(lambda: r.reached(w), within=15)
        self.send(f"{typed} typed into the hang", typed)
        time.sleep(3)
        r.stop()
        self.silent[w] = "stopped in a hung turn"
        self.silent[typed] = "typed into a hung turn and never read"
        return {"words": [w, typed], "stopped": until(lambda: not r.typing(), within=30)}

    def hippo_frozen(self):
        r = self.r
        w = self.in_flight()
        r.signal(r.hippo.pid, signals.SIGSTOP)
        time.sleep(70)
        r.signal(r.hippo.pid, signals.SIGCONT)
        return {"words": [w], "turn_ended": until(lambda: not r.typing(), within=30)}

    def disk_full(self, where):
        r = self.r
        filler = r.root / where / "filler"
        try:
            with open(filler, "wb") as f:
                while True:
                    f.write(bytes(1 << 20))
        except OSError:
            pass
        try:
            with open(r.root / where / "filler-tail", "wb") as f:
                while True:
                    f.write(bytes(4096))
                    f.flush()
        except OSError:
            pass
        w = self.word()
        status = self.send(f"{w} sent to a full disk", w)
        answered = until(lambda: w in r.answered, within=20)
        seen = {"status": status, "answered_while_full": answered, "host": r.status("/api/host"),
                "history": r.status("/api/history"), "said_while_full": [l[:160] for l in r.log(2)]}
        filler.unlink()
        (r.root / where / "filler-tail").unlink(missing_ok=True)
        return {"words": [w] if status == 202 else []} | seen

    def network_cut(self):
        """A client vanishes with its packets, as a phone leaving wifi does:
        no reset, the daemon only stops hearing. The cut outlasts the turn."""
        r = self.r
        fds = r.fds()
        def nft(*args):
            subprocess.run(["nft", *args], check=True)
        nft("add", "table", "inet", "cut")
        nft("add", "chain", "inet", "cut", "input", "{ type filter hook input priority 0 ; }")
        w = self.in_flight(slow=8)
        nft("add", "rule", "inet", "cut", "input", "tcp", "dport", str(PORT), "drop")
        nft("add", "rule", "inet", "cut", "input", "tcp", "sport", str(PORT), "drop")
        try:
            time.sleep(90)
        finally:
            nft("delete", "table", "inet", "cut")
        threading.Thread(target=r.listen, daemon=True).start()
        return {"words": [w], "fds_before": fds, "ended_in_the_cut": until(lambda: not r.typing(), within=30)}

    FAULTS = [
        ("libri killed", lambda s: s.libri_killed(signals.SIGKILL)),
        ("hippo killed", lambda s: s.hippo_killed()),
        ("harness hung, Stop", lambda s: s.harness_hung()),
        ("disk full: hippo", lambda s: s.disk_full("hippo")),
        ("network cut", lambda s: s.network_cut()),
        ("libri terminated", lambda s: s.libri_killed(signals.SIGTERM)),
        ("both killed", lambda s: s.both_killed()),
        ("harness killed", lambda s: s.harness_killed()),
        ("disk full: state", lambda s: s.disk_full("state")),
        ("hippo frozen", lambda s: s.hippo_frozen()),
        ("both restarted", lambda s: s.both_restarted()),
    ]

    def fault(self, name, run):
        r = self.r
        self.fault_name = name
        t = time.monotonic()
        seen = {}
        try:
            seen = run(self)
        except Exception:
            seen = {"raised": traceback.format_exc()[-600:]}
        self.fault_name = None
        recovered = {}
        try:
            recovered = self.after_fault()
        except Exception:
            recovered = {"raised_after": traceback.format_exc()[-600:]}
        words = seen.pop("words", [])
        kept = {w: self.hold(w) for w in words}
        broke = []
        if "raised" in seen or "raised_after" in recovered:
            broke.append("the fault's own code raised")
        if recovered.get("next_turn") is None:
            broke.append("no answer after the fault")
        elif recovered.get("needed_stop"):
            broke.append("next turn needed a Stop")
        missing = [w for w, c in kept.items() if c != 1]
        if missing:
            broke.append(f"words not in hippo once: { {w: kept[w] for w in missing} }")
        event("fault", name=name, took=round(time.monotonic() - t, 1), broke=broke, kept=kept,
              **seen, **recovered, log=[l[:200] for l in r.log(3)] if broke else [])
        if recovered.get("next_turn") is None:
            # Nothing else can be learned from a dead pair; a person restarts it.
            self.restart_all()
            event("restarted by the suite", after=name, answered=self.turn())

    def sample(self):
        r = self.r
        def proc(p):
            try:
                status = Path(f"/proc/{p.pid}/status").read_text()
                return {"rss_kib": int(re.search(r"VmRSS:\s+(\d+)", status)[1]),
                        "threads": int(re.search(r"Threads:\s+(\d+)", status)[1]),
                        "fds": len(os.listdir(f"/proc/{p.pid}/fd"))}
            except (OSError, TypeError):
                return None
        def size(d):
            return sum(p.stat().st_size for p in (r.root / d).rglob("*") if p.is_file() and "filler" not in p.name) // 1024
        window, self.latencies = sorted(self.latencies), []
        stand_ins = subprocess.run(["pgrep", "-c", "-f", str(r.claude)], capture_output=True, text=True).stdout.strip()
        with (OUTPUT / "samples.jsonl").open("a") as f:
            f.write(json.dumps({
                "at": time.time(), "turns": self.turns, "timeouts": self.timeouts,
                "libri": proc(r.daemon), "hippo": proc(r.hippo), "state_kib": size("state"), "hippo_kib": size("hippo"),
                "stand_ins": int(stand_ins or 0),
                "turn_p50": window[len(window) // 2] if window else None,
                "turn_max": window[-1] if window else None,
                "available_mib": available_mib(), "load": load(),
            }) + "\n")

    def run(self, seconds):
        begin = time.monotonic()
        next_sample = next_fault = begin
        next_fault += PLAN["fault_every_seconds"]
        faults = 0
        while time.monotonic() - begin < seconds:
            now = time.monotonic()
            if now >= next_sample:
                self.sample()
                next_sample = now + 30
            if now >= next_fault:
                name, run = self.FAULTS[faults % len(self.FAULTS)]
                faults += 1
                self.fault(name, run)
                next_fault = time.monotonic() + PLAN["fault_every_seconds"]
                continue
            t = self.turn()
            self.turns += 1
            if t is None:
                self.timeouts += 1
                event("turn timed out", word=f"w{self.n}", status=self.send_status, stuck=self.stuck(), log=[l[:200] for l in self.r.log(3)])
                self.r.stop()
                # A refusal comes at once; this keeps it from spinning.
                time.sleep(1)
            else:
                self.latencies.append(t)
        self.sample()

    def check_ledger(self):
        """Every word Libri took is in hippo once, in the order it was sent."""
        rows = self.r.full_chat()
        at = {}
        resumed = 0
        for i, m in enumerate(rows):
            if m["kind"] == "user":
                if m["text"].startswith(RESUMED):
                    resumed += 1
                    continue
                for w in re.findall(r"\bw\d+\b", m["text"]):
                    at.setdefault(w, []).append(i)
        missing = [w for w in self.sent_order if w not in at]
        doubled = [w for w in self.sent_order if len(at.get(w, [])) > 1]
        accepted = [w for w in doubled if w in self.saved_twice and len(at[w]) == 2]
        doubled = [w for w in doubled if w not in accepted]
        first = [at[w][0] for w in self.sent_order if w in at]
        out_of_order = sum(1 for a, b in zip(first, first[1:]) if b < a)
        said = {w for m in rows if m["kind"] != "user" for w in ANSWER.findall(m["text"])}
        unanswered = [w for w in self.sent_order if w not in said]
        unexplained = [w for w in unanswered if w not in self.silent]
        event("ledger", sent=len(self.sent_order), refused=self.lost_to_refusal, rows=len(rows), resumed=resumed,
              missing=len(missing), missing_words={w: self.ledger[w] for w in missing[:40]},
              doubled=len(doubled), doubled_words={w: self.ledger[w] for w in doubled[:40]},
              saved_twice=accepted,
              doubled_rows=[rows[i]["text"][:300] for w in doubled[:3] for i in at[w]],
              out_of_order=out_of_order, unanswered=len(unanswered),
              unanswered_words={w: self.silent.get(w) for w in unanswered[:40]}, unexplained=len(unexplained),
              unexplained_words={w: self.ledger[w] for w in unexplained[:40]})

    def finish(self):
        r = self.r
        try:
            self.check_ledger()
        except Exception:
            event("ledger failed", why=traceback.format_exc()[-600:])
        for name in ("libri.log", "hippo.log"):
            f = r.root / name
            text = f.read_text(errors="replace") if f.exists() else ""
            event("log", name=name, lines=text.count("\n"), panics=[l[:240] for l in text.splitlines() if "panicked" in l][:10])
            shutil.copy(f, OUTPUT / name)
        r.close()
        # What the ledger read, kept so its verdict can be checked by eye.
        shutil.copytree(r.root / "hippo/main", OUTPUT / "hippo-main", dirs_exist_ok=True)
        shutil.copy(r.root / "read.jsonl", OUTPUT / "read.jsonl")


def keys_of(path):
    """The shape of one file: JSON keys, or a database's schema."""
    try:
        if path.suffix == ".json":
            value = json.loads(path.read_text())
            return value.keys() if isinstance(value, dict) else [type(value).__name__]
        if path.suffix == ".jsonl":
            with path.open() as f:
                return {k for line in f if line.strip() for k in json.loads(line)}
        with path.open("rb") as f:
            if f.read(16) == b"SQLite format 3\0":
                db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
                try:
                    schema = "".join(sorted(r[0] or "" for r in db.execute("select sql from sqlite_master")))
                    version = db.execute("pragma user_version").fetchone()[0]
                    return [f"sqlite v{version} {hashlib.sha256(schema.encode()).hexdigest()[:12]}"]
                finally:
                    db.close()
    except (OSError, ValueError, AttributeError, sqlite3.Error) as e:
        return [f"unreadable: {type(e).__name__}"]
    return []


def available_mib():
    return int(re.search(r"MemAvailable:\s+(\d+)", Path("/proc/meminfo").read_text())[1]) // 1024


def load_of_workers():
    """The rig's own load scenarios, for what fifty workers cost. They print
    what they saw and judge nothing; the report carries it as it stands."""
    lowest = [available_mib()]
    stop = threading.Event()
    def watch():
        while not stop.wait(5):
            lowest.append(available_mib())
    threading.Thread(target=watch, daemon=True).start()
    for name in PLAN.get("load", ["W-quick", "W-hang"]):
        t = time.monotonic()
        done = subprocess.run([sys.executable, str(HERE / "scenarios.py"), name],
                              capture_output=True, text=True, timeout=1800)
        event("load", name=name, took=round(time.monotonic() - t), code=done.returncode,
              lines=[l[:400] for l in done.stdout.splitlines()], err=done.stderr[-400:])
    stop.set()
    event("load memory", lowest_available_mib=min(lowest))


def main():
    event("start", plan=PLAN, build=str(BUILD), previous=str(PREVIOUS), available_mib=available_mib())
    if not check_sealed():
        event("abort", why="the VM is not sealed")
        return
    soak = Soak()
    try:
        soak.crossings()
        soak.run(PLAN["minutes"] * 60)
    except Exception:
        event("suite failed", why=traceback.format_exc()[-800:])
    finally:
        soak.finish()
    load_of_workers()
    event("end", available_mib=available_mib())


if __name__ == "__main__":
    main()
