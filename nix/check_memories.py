#!/usr/bin/env python3
"""Named memories: each a chat on a hippo of its own, apart from the main one.

Drives the real daemon on a throwaway root (throwaway_libri.py) with the
stand-in Claude: makes a memory, gives it instructions, talks in its chat,
and proves that its turns read its own memory and the person's rules and
nothing of the main chat's, that its words are kept in its own store and
never the main one, that its tools are held apart, and that deleting it
takes its store and its hippo. Usage: check_memories.py LIBRI_DIR.
"""

import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

from throwaway_libri import Throwaway, until


def check(what, ok, detail=""):
    print(("ok  " if ok else "FAIL") + f" {what}" + (f": {detail}" if detail and not ok else ""))
    if not ok:
        raise SystemExit(1)


class Rig:
    def __init__(self, libri_dir):
        # Workers start on the next tick of their loop: a short one for a check.
        self.t = Throwaway(libri_dir, prefix="libri-memories-", extra={"LIBRI_WORK_TICK": "1"})
        self.hippo_bin = Path(libri_dir) / "bin/hippo"

    def call(self, method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(
            f"{self.t.base}{path}",
            data=data,
            method=method,
            headers={"content-type": "application/json", "origin": self.t.base},
        )
        try:
            with urllib.request.urlopen(req, timeout=20) as r:
                raw = r.read()
                return r.status, (json.loads(raw) if raw and raw[:1] in b"{[" else raw.decode())
        except urllib.error.HTTPError as e:
            return e.code, e.read().decode()

    def turns(self):
        log = self.t.root / "stand-in.jsonl"
        return [json.loads(l) for l in log.read_text().splitlines()] if log.exists() else []

    def told(self, words):
        """The newest model turn given `words`, not a compaction's."""
        return next((t for t in reversed(self.turns()) if words in t["said"] and not t["plain"]), None)

    def hippo(self, store, *args):
        env = {k: v for k, v in self.t.env.items() if k != "HIPPO_DIR"} | {"HIPPO_DIR": str(store)}
        return subprocess.run([self.hippo_bin, *args], env=env, capture_output=True, text=True, timeout=20).stdout

    def answered(self, memory, words):
        """Tells memory `memory`'s chat `words` and waits for its answer."""
        status, _ = self.call("POST", f"/api/memories/{memory}/chat", {"text": words, "files": []})
        check(f"{memory}'s chat takes words", status == 202, str(status))

        def done():
            _, chat = self.call("GET", f"/api/memories/{memory}/chat")
            return not chat["working"] and f"claude answers: {words}" in chat["text"]

        until(done, f"{memory} answered", wait=60)
        return self.told(words)

    def said_back(self, memory, words, reply):
        """Tells memory `memory`'s chat `words` and waits for a reply holding
        `reply`; the chat's text."""
        status, _ = self.call("POST", f"/api/memories/{memory}/chat", {"text": words, "files": []})
        check(f"{memory}'s chat takes {words!r}", status == 202, str(status))
        got = {}

        def done():
            _, chat = self.call("GET", f"/api/memories/{memory}/chat")
            got["chat"] = chat
            return not chat["working"] and reply in chat["text"].rsplit(f"|user: {words}", 1)[-1]

        try:
            until(done, f"{memory} said {reply!r}", wait=60)
        except TimeoutError:
            print(json.dumps(got.get("chat"))[-1500:], file=sys.stderr)
            raise
        return got["chat"]["text"].rsplit(f"|user: {words}", 1)[-1]


def hippo_children(store):
    """The processes running a hippo on `store`."""
    found = []
    for proc in Path("/proc").iterdir():
        try:
            env = (proc / "environ").read_bytes().split(b"\0")
        except OSError:
            continue
        if f"HIPPO_DIR={store}".encode() in env:
            found.append(proc.name)
    return found


def main():
    rig = Rig(sys.argv[1])
    t = rig.t
    mark = t.mark
    try:
        state = t.root / "state"
        status, _ = rig.call("POST", "/api/guide", {"text": "RULES-EVERY-MEMORY: be brief."})
        check("the shared rules are set", status in (200, 204), str(status))

        status, made = rig.call("POST", "/api/memories", {"name": "Tutor"})
        check("a memory is added", status == 200 and made["id"] == "tutor", f"{status} {made}")
        status, _ = rig.call("POST", "/api/memories", {"name": "main"})
        check("Main is the main chat's own name", status == 400)
        status, _ = rig.call("POST", "/api/memories", {"name": "tutor"})
        check("two memories don't share a name", status == 400)
        status, _ = rig.call("POST", "/api/memories/tutor", {"instructions": "TUTOR-OWN: ask, don't tell."})
        check("a memory takes instructions", status == 204, str(status))
        _, listed = rig.call("GET", "/api/memories")
        check(
            "the memories are listed with their instructions",
            listed["memories"] == [{"id": "tutor", "name": "Tutor", "instructions": "TUTOR-OWN: ask, don't tell.", "asking": False, "working": False}],
            str(listed),
        )

        # The main chat says something the named memory must never see.
        rig.call("POST", "/api/run", {"messages": [{"id": "m1", "role": "user", "content": f"main-only {mark}"}]})
        until(lambda: rig.told(f"main-only {mark}"), "the main chat answered", wait=60)

        first = rig.answered("tutor", f"tutor-first {mark}")
        system = first["args"][first["args"].index("--append-system-prompt") + 1]
        check("its turn has the shared rules", "RULES-EVERY-MEMORY" in system)
        check("its turn has the memory's own instructions", "TUTOR-OWN" in system)
        check("its rules come before its own instructions", system.index("RULES-EVERY-MEMORY") < system.index("TUTOR-OWN"))
        check("its turn knows which memory it is on", "the memory named Tutor" in system)
        check("its turn reads nothing of the main chat", "main-only" not in first["said"] and "main-only" not in system)
        check("its turn keeps no harness session", "--no-session-persistence" in first["args"])
        servers = json.loads(first["args"][first["args"].index("--mcp-config") + 1])["mcpServers"]
        check("its tools are held apart from the main memory", servers["libri"]["env"].get("LIBRI_SCOPE") == "apart", str(servers["libri"].get("env")))
        store = state / "memories/tutor/hippo"
        check("its tool server reads its own hippo", servers["libri"]["env"].get("HIPPO_DIR") == str(store), str(servers["libri"].get("env")))
        check("it has no wiki dreamed from the main memory", "wiki" not in servers)

        second = rig.answered("tutor", f"tutor-second {mark}")
        check("the next turn remembers the first, from its own memory", f"tutor-first {mark}" in second["said"] and "<chat>" in second["said"])
        check("the next turn has the newest words whole", f"<orders>\nuser: tutor-second {mark}\n</orders>" in second["said"])

        until(lambda: f"claude answers: tutor-second {mark}" in rig.hippo(store, "tail", "20"), "the memory kept the answer")
        kept = rig.hippo(store, "tail", "20")
        check("its own store keeps both sides", f"tutor-first {mark}" in kept and f"claude answers: tutor-first {mark}" in kept)
        check("its own store has none of the main chat", "main-only" not in kept)
        main = rig.hippo(t.root / "hippo", "search", "tutor-first")
        check("the main memory has none of it", "tutor-first" not in main, main[:300])
        found = rig.hippo(t.root / "hippo", "search", "main-only")
        check("while a search of it finds the main chat's own words", "main-only" in found, found[:300])

        # The memory's server reads its own memory, never the main one.
        recalled = rig.said_back("tutor", "RECALL tutor-first", "recalled from")
        check("its memory's server reads its own memory", "recalled from memory tutor" in recalled, recalled[:300])

        # A permission prompt waits on the person on its own chat, not refused.
        status, _ = rig.call("POST", "/api/memories/tutor/chat", {"text": "PERMIT", "files": []})
        check("its chat takes PERMIT", status == 202, str(status))
        asked = {}

        def asking():
            _, chat = rig.call("GET", "/api/memories/tutor/chat")
            asked["asks"] = chat.get("asks", [])
            return asked["asks"]

        until(asking, "the memory's chat asked the person", wait=60)
        ask = asked["asks"][0]
        check("the ask is the memory chat's own", ask["id"].startswith("memory:memory-tutor:"), ask["id"])
        check("the ask names the tool", "Bash" in json.dumps(ask), json.dumps(ask)[:300])
        status, _ = rig.call("POST", "/api/run", {"threadId": "t", "runId": "r", "messages": [], "resume": [{"interruptId": ask["id"], "status": "resolved"}]})
        check("the person answers it", status in (200, 202, 204), str(status))

        def permitted():
            _, chat = rig.call("GET", "/api/memories/tutor/chat")
            return not chat["working"] and "permitted" in chat["text"].rsplit("PERMIT", 1)[-1] and not chat["asks"]

        until(permitted, "the turn ran on the person's yes", wait=60)

        # A memory's chat starts a worker: it forks that memory, hangs from it,
        # and reports to its chat; the main chat and memory see none of it.
        status, jobs = rig.call("POST", "/api/memories", {"name": "Jobs"})
        check("a second memory is added", status == 200 and jobs["id"] == "jobs", f"{status} {jobs}")
        rig.call("POST", "/api/memories/jobs", {"instructions": "JOBS-OWN: rank by fit."})
        rig.answered("jobs", f"jobs-first {mark}")
        jobs_store = state / "memories/jobs/hippo"
        spawned = rig.said_back("jobs", f"SPAWN boards-task {mark}", "spawned from")
        check("its chat starts a worker", "refused=False" in spawned and "Started" in spawned, spawned[:300])
        check("its spawn reads its own store", f"{jobs_store} memory=jobs" in spawned, spawned[:300])
        found = {}

        def worker():
            _, work = rig.call("GET", "/api/work")
            found["t"] = next((w for w in work["threads"] if f"boards-task {mark}" in w["brief"]), None)
            return found["t"]

        until(worker, "the worker is listed", wait=30)
        check("the worker belongs to Jobs", found["t"].get("memory") == "jobs", str(found["t"]))
        def worker_turn():
            for x in reversed(rig.turns()):
                if x["plain"] or "--mcp-config" not in x["args"]:
                    continue
                env = json.loads(x["args"][x["args"].index("--mcp-config") + 1])["mcpServers"]["libri"]["env"]
                if env.get("LIBRI_WORKER") == found["t"]["id"]:
                    return x
            return None

        until(worker_turn, "the worker ran", wait=60)
        ran = worker_turn()
        wsys = ran["args"][ran["args"].index("--append-system-prompt") + 1]
        wenv = json.loads(ran["args"][ran["args"].index("--mcp-config") + 1])["mcpServers"]["libri"]["env"]
        check("the worker knows it is the Jobs memory's", "memory named Jobs" in wsys)
        check("the worker has the rules and Jobs' own instructions", "RULES-EVERY-MEMORY" in wsys and "JOBS-OWN" in wsys)
        check("the worker's tools read the Jobs store", wenv.get("HIPPO_DIR") == str(jobs_store) and wenv.get("LIBRI_MEMORY") == "jobs", str(wenv))
        check("the worker is a worker", wenv.get("LIBRI_SCOPE") == "agent", str(wenv))
        check("the worker forked the Jobs memory", f"jobs-first {mark}" in ran["said"], t.shown(ran["said"][:600]))
        check("the worker read nothing of the main chat", "main-only" not in ran["said"] and "tutor-first" not in ran["said"])

        def reported():
            _, chat = rig.call("GET", "/api/memories/jobs/chat")
            tail = chat["text"].rsplit("SPAWN", 1)[-1]
            return not chat["working"] and "<agent-report>" in tail and f"claude answers: boards-task {mark}" in tail

        until(reported, "the report reached the Jobs chat", wait=90)
        check("the report reached the Jobs chat", True)
        until(lambda: rig.told("<agent-report>"), "the Jobs chat took a turn on the report", wait=60)
        turn = rig.told("<agent-report>")
        jsys = turn["args"][turn["args"].index("--append-system-prompt") + 1]
        check("the turn on the report is the Jobs chat's", "the memory named Jobs" in jsys and "JOBS-OWN" in jsys)
        check("the main chat had no turn on it", not any("<agent-report>" in x["said"] and "memory named Jobs" not in x["args"][x["args"].index("--append-system-prompt") + 1] for x in rig.turns() if not x["plain"] and "--append-system-prompt" in x["args"]))
        until(lambda: "boards-task" in rig.hippo(jobs_store, "search", "boards-task"), "the Jobs memory finds the worker", wait=30)
        main = rig.hippo(t.root / "hippo", "search", "boards-task")
        check("the main memory finds nothing of the worker", "boards-task" not in main, t.shown(main[:300]))
        tutor_store = state / "memories/tutor/hippo"
        other = rig.hippo(tutor_store, "search", "boards-task")
        check("another memory finds nothing of it", "boards-task" not in other, t.shown(other[:300]))

        status, _ = rig.call("POST", "/api/memories/tutor", {"name": "Socratic tutor"})
        _, listed = rig.call("GET", "/api/memories")
        check("a rename keeps its id", {"id": "tutor", "name": "Socratic tutor", "instructions": "TUTOR-OWN: ask, don't tell.", "asking": False, "working": False} in listed["memories"], str(listed))

        check("its hippo runs while it lives", hippo_children(store) != [])
        status, _ = rig.call("DELETE", "/api/memories/tutor")
        check("a memory is deleted", status == 204, str(status))
        check("its folder goes with it", not (state / "memories/tutor").exists())
        until(lambda: hippo_children(store) == [], "its hippo stopped", wait=15)
        status, _ = rig.call("GET", "/api/memories/tutor/chat")
        check("its chat goes too", status == 404)

        strays = t.strays()
        check("nothing landed outside the scratch root", not strays, str(strays))
    except Exception:
        print(t.shown((t.root / "libri.log").read_text()[-4000:]), file=sys.stderr)
        raise
    finally:
        t.close(keep=bool(os.environ.get("KEEP")))
    print("check_memories: all good")


if __name__ == "__main__":
    main()
