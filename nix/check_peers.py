#!/usr/bin/env python3
"""Two assistants pair, message each other, unpair and pair again, each
pair with its own credential (peers.rs): one asks by address, the other's
person approves, and both hold the pair; a message either way lands in the
other's inbox under the pair's name; a wrong or revoked token is refused;
revoking on one side forgets the pair on both. A host still set up with the
old shared token and peer list takes that pair over as paired, once.

A pair is addressed by the other person as well as by their assistant: the
pairing carries the person, a pair made without it learns them from the
other side's Agent Card, and a shared routine naming the person arrives. A
routine shared with nobody paired is refused, and one whose answer can't be
delivered says so in the sender's own chat.

Drives two real daemons in scratch directories. Usage: check_peers.py
LIBRI_DIR (holding bin/libri, bin/hippo).
"""

import json
import subprocess
import sys
import time
from datetime import datetime, timedelta
import urllib.error
import urllib.request
from pathlib import Path

from throwaway_libri import Throwaway

failures = []


def check(what, ok, detail=""):
    print(f"ok   {what}" if ok else f"FAIL {what} {detail}", flush=True)
    if not ok:
        failures.append(what)


def call(url, body=None, token=None):
    """POSTs JSON (or GETs without a body); answers (status, text)."""
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(url, data=data, method="GET" if body is None else "POST")
    req.add_header("content-type", "application/json")
    if token:
        req.add_header("authorization", f"Bearer {token}")
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def peers(box):
    status, text = call(f"{box.base}/api/peers")
    return json.loads(text) if status == 200 else None


def tokens(box):
    path = box.root / "state/peers.json"
    return [p["token"] for p in json.loads(path.read_text())] if path.exists() else []


def message(to, token, text):
    body = {"jsonrpc": "2.0", "id": 1, "method": "SendMessage",
            "params": {"message": {"messageId": "m", "role": "ROLE_USER", "parts": [{"text": text}],
                                   "metadata": {"from": "Someone Else"}}}}
    return call(f"{to.base}/a2a", body, token)


def inbox(box):
    d = box.root / "state/inbox"
    return [f.read_text() for f in sorted(d.iterdir()) if not f.name.startswith(".")] if d.exists() else []


def history(box):
    status, text = call(f"{box.base}/api/history")
    return [m.get("text", "") for m in json.loads(text)] if status == 200 else []


def wait(what, test, seconds=150):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if test():
            return True
        time.sleep(0.1)
    return False


class Tools:
    """The box's own tool server, as a harness reaches it."""

    def __init__(self, libri, box):
        # The person's own scope: an unset one is the quiet scope, which
        # withholds watch.
        env = {**box.env, "LIBRI_SCOPE": "person"}
        self.proc = subprocess.Popen([libri / "bin/libri", "tools", str(box.root / "state")], env=env,
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.id = 0
        self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                "clientInfo": {"name": "check", "version": "0"}})
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.proc.stdin.flush()

    def rpc(self, method, params):
        self.id += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        return json.loads(self.proc.stdout.readline())

    def call(self, name, arguments):
        """(refused, text) of a tool call."""
        got = self.rpc("tools/call", {"name": name, "arguments": arguments}).get("result", {})
        return bool(got.get("isError")), "".join(c.get("text", "") for c in got.get("content", []))

    def close(self):
        self.proc.stdin.close()
        self.proc.wait()


def routine(tools, ask, names):
    """Sets a daily routine at the next minute, shared with `names`."""
    at = (datetime.now() + timedelta(minutes=1)).strftime("%Y-%m-%d %H:%M")
    return tools.call("watch", {"when": {"at": at, "every": "daily"}, "then": {"ask": ask, "share_with": names}})


def main():
    libri = Path(sys.argv[1])
    a = Throwaway(libri, prefix="peers-a-", extra={"LIBRI_NAME": "Ann", "LIBRI_PERSON": "Al"})
    b = Throwaway(libri, prefix="peers-b-", extra={"LIBRI_NAME": "Bea", "LIBRI_PERSON": "Bo"})
    c = tools = None
    try:
        status, said = call(f"{a.base}/api/peers", {"url": b.base})
        check("asking to pair is taken", status == 204, f"{status} {said}")
        pa, pb = peers(a), peers(b)
        check("the asker waits", [p["state"] for p in pa] == ["asking"], pa)
        check("the other side holds the request by name", [(p["name"], p["state"]) for p in pb] == [("Ann", "asked")], pb)
        check("the other person hears of the request", any("asks to pair" in t for t in inbox(b)), inbox(b))
        check("no token is shown in Settings", all("token" not in p for p in pa + pb))
        token = tokens(a)[0]
        status, _ = message(b, token, "too soon")
        check("a request's token opens nothing before approval", status == 401, status)

        status, said = call(f"{b.base}/api/peers/approve", {"url": a.base})
        check("approving is taken", status == 204, f"{status} {said}")
        pa, pb = peers(a), peers(b)
        check("both sides hold the pair", [(p["name"], p["state"]) for p in pa] == [("Bea", "paired")]
              and [(p["name"], p["state"]) for p in pb] == [("Ann", "paired")], (pa, pb))
        check("one credential for the pair", tokens(a) == tokens(b), (len(tokens(a)), len(tokens(b))))

        status, said = message(b, token, "hello Bea")
        check("a message reaches the other side", status == 200 and "Delivered to Bea" in said, f"{status} {said}")
        check("the pair names the sender, not what it claims", any(t == "From Ann: hello Bea" for t in inbox(b)), inbox(b))
        status, said = message(a, token, "hello Ann")
        check("and the other way", status == 200 and any(t == "From Bea: hello Ann" for t in inbox(a)), inbox(a))
        check("the last message is shown", all(p.get("last") for p in peers(a) + peers(b)), peers(a))
        status, _ = message(b, "0" * 64, "intruder")
        check("a wrong token is refused", status == 401, status)
        status, _ = message(b, "", "nobody")
        check("no token is refused", status == 401, status)

        status, said = call(f"{a.base}/api/peers/revoke", {"url": b.base})
        check("revoking is taken", status == 204, f"{status} {said}")
        check("the revoker forgets the pair at once", peers(a) == [], peers(a))
        status, _ = message(a, token, "after revoke")
        check("the revoker refuses the old token at once", status == 401, status)
        end = time.monotonic() + 10
        while peers(b) and time.monotonic() < end:
            time.sleep(0.1)
        check("the other side forgets it too", peers(b) == [], peers(b))
        status, _ = message(b, token, "after revoke")
        check("the other side refuses the old token", status == 401, status)

        call(f"{b.base}/api/peers", {"url": a.base})
        call(f"{a.base}/api/peers/approve", {"url": b.base})
        again = tokens(a)
        check("pairing again works the other way round", [p["state"] for p in peers(a)] == ["paired"]
              and [p["state"] for p in peers(b)] == ["paired"], (peers(a), peers(b)))
        check("with a new credential", again and again[0] != token, again)
        status, _ = message(a, again[0], "again")
        check("the new pair carries messages", status == 200, status)
        status, _ = message(a, token, "old")
        check("the old credential stays dead", status == 401, status)

        pa, pb = peers(a), peers(b)
        check("the pairing carries each side's person", [p.get("person") for p in pa] == ["Bo"]
              and [p.get("person") for p in pb] == ["Al"], (pa, pb))
        # A pair made before pairing carried the person.
        held = a.root / "state/peers.json"
        older = json.loads(held.read_text())
        for p in older:
            p.pop("person", None)
        held.write_text(json.dumps(older))
        tools = Tools(libri, a)
        refused, said = routine(tools, "Say PEERSHARED.", ["bo"])
        check("a routine shared with the other person is taken", not refused, said)
        check("an older pair learns its person from the Agent Card",
              [p.get("person") for p in peers(a)] == ["Bo"], peers(a))
        refused, said = routine(tools, "Say nothing.", ["Cy"])
        check("a routine shared with nobody paired is refused, naming who is",
              refused and "Bea (Bo's assistant)" in said, said)
        # Bea's own tick takes the inbox into a turn, so the message is
        # looked for there and in Bea's chat.
        arrived = wait("shared", lambda: any("From Ann: From Al's shared routine" in t and "PEERSHARED" in t
                                             for t in inbox(b) + history(b)))
        check("the shared routine's answer reaches the other side", arrived, (inbox(b), history(b)[-5:]))
        b.close()
        refused, said = routine(tools, "Say PEERLOST.", ["Bo"])
        check("a routine shared with a paired person is taken while they are away", not refused, said)
        surfaced = wait("surfaced", lambda: any("didn't reach Bo" in t for t in history(a)))
        check("an answer that can't be delivered says so in the sender's chat", surfaced, history(a)[-5:])

        status, said = call(f"{a.base}/api/peers", {"url": "http://127.0.0.1:1/a2a"})
        check("an address with a path is refused", status == 400, f"{status} {said}")
        status, said = call(f"{a.base}/api/peers", {"url": "http://127.0.0.1:1"})
        check("an unreachable address asks nothing", status == 400 and len(peers(a)) == 1, f"{status} {peers(a)}")

        # The old setup: one shared token in a file, peers by name and address.
        shared = a.root / "shared-token"
        shared.write_text("s" * 40)
        c = Throwaway(libri, prefix="peers-c-", extra={
            "LIBRI_NAME": "Cy", "LIBRI_A2A_TOKEN": str(shared), "LIBRI_A2A": f"Ann={a.base}"})
        pc = peers(c)
        check("a host on the old shared token keeps its pair", [(p["name"], p["url"], p["state"]) for p in pc]
              == [("Ann", a.base, "paired")], pc)
        status, said = message(c, "s" * 40, "from the old setup")
        check("and takes messages under it", status == 200 and any(t == "From Ann: from the old setup" for t in inbox(c)),
              f"{status} {said}")
    finally:
        if tools:
            tools.close()
        for box in (a, b, c):
            if box:
                box.close()
    if failures:
        print(f"{len(failures)} failed: {', '.join(failures)}")
        sys.exit(1)
    print("all paired")


if __name__ == "__main__":
    main()
