#!/usr/bin/env python3
"""Other apps reach Libri as an MCP server, each as a connector: an app with
no token is sent to sign in; it finds the authorization server from /mcp's
metadata, registers, is allowed on the consent page, and redeems its code
with its PKCE verifier, as Claude's and ChatGPT's connectors, Claude Code
and Codex do. Then it sees only its modules' tools: it searches the memory,
starts a worker, tells it, and sends into the main chat, each under the
connector's name. A connector made by key in Settings works the same, over
HTTP and over `libri mcp`; changing its modules changes its tools at once,
and revoking it shuts it out. Nothing that acts on the host, or answers for
the person, is ever offered.

Drives the real daemon against a real hippo in a scratch directory, with the
stand-in `claude`. Usage: check_outside.py LIBRI_DIR (holding bin/libri,
bin/hippo).
"""

import base64
import hashlib
import json
import re
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

from throwaway_libri import Throwaway

LIBRI_DIR = Path(sys.argv[1]) if __name__ == "__main__" else None
MEMORY = {"memory_recall", "memory_open"}
WORKERS = {"workers_list", "worker_read", "worker_start", "worker_tell"}
CHATS = {"chats_list", "messages_read", "events_wait", "message_send", "asks_list"}
EXTRAS = {"web_search", "x_search", "make_image", "show_page", "pages"}
# Never offered to another app, whatever it is let use.
NEVER = {"spawn", "run_code", "watch", "make_worktree", "archive_agent", "browser_open", "files", "read_file",
         "ask_answer", "tell_agent", "settings"}
CALLBACK = "http://127.0.0.1:47123/callback"
failures = []
box = None  # the throwaway Libri under check


def check(what, ok, detail=""):
    print(f"ok   {what}" if ok else f"FAIL {what} {box.shown(detail) if box else detail}", flush=True)
    if not ok:
        failures.append(what)


class Http:
    """An MCP client over streamable HTTP, as Claude Code and Codex speak it."""

    def __init__(self, url, token=None):
        self.url, self.session, self.id, self.token = url, None, 0, token

    def post(self, body):
        headers = {"content-type": "application/json", "accept": "application/json, text/event-stream"}
        if self.session:
            headers["mcp-session-id"] = self.session
        if self.token:
            headers["authorization"] = f"Bearer {self.token}"
        req = urllib.request.Request(self.url, data=json.dumps(body).encode(), headers=headers)
        with urllib.request.urlopen(req, timeout=60) as r:
            self.session = r.headers.get("mcp-session-id") or self.session
            text = r.read().decode()
        if "id" not in body:
            return None
        if text.lstrip().startswith("{"):
            return json.loads(text)
        # An SSE answer: the event that carries this request's id.
        for line in text.splitlines():
            if line.startswith("data:") and line[5:].strip():
                msg = json.loads(line[5:])
                if msg.get("id") == body["id"]:
                    return msg
        raise ValueError(f"no answer in {text!r}")

    def rpc(self, method, params):
        self.id += 1
        return self.post({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params})

    def notify(self, method):
        self.post({"jsonrpc": "2.0", "method": method})


class Stdio:
    """An MCP client of `libri mcp`, as Claude Desktop starts a server."""

    def __init__(self, env):
        self.proc = subprocess.Popen([LIBRI_DIR / "bin/libri", "mcp"], env=env, text=True, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.id = 0

    def rpc(self, method, params):
        self.id += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        while True:
            msg = json.loads(self.proc.stdout.readline())
            if msg.get("id") == self.id:
                return msg

    def notify(self, method):
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": method}) + "\n")
        self.proc.stdin.flush()


def connect(client):
    got = client.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                    "clientInfo": {"name": "check", "version": "0"}})
    client.notify("notifications/initialized")
    return got


def tool(client, called, **arguments):
    got = client.rpc("tools/call", {"name": called, "arguments": arguments})
    if "error" in got:
        return True, got["error"]["message"]
    got = got["result"]
    return got.get("isError", False), got["content"][0]["text"]


def names(client):
    return {t["name"] for t in client.rpc("tools/list", {})["result"]["tools"]}


class Stay(urllib.request.HTTPRedirectHandler):
    """Keeps a redirect for the check to read, as the app's browser would follow it."""

    def redirect_request(self, *args, **kwargs):
        return None


def http(method, url, body=None, form=None, headers=None, follow=True):
    """A request as an app makes it: the status, headers and body, errors included."""
    data, h = None, dict(headers or {})
    if body is not None:
        data, h["content-type"] = json.dumps(body).encode(), "application/json"
    if form is not None:
        data, h["content-type"] = urllib.parse.urlencode(form).encode(), "application/x-www-form-urlencoded"
    opener = urllib.request.build_opener() if follow else urllib.request.build_opener(Stay)
    try:
        with opener.open(urllib.request.Request(url, data=data, headers=h, method=method), timeout=30) as r:
            return r.status, r.headers, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.headers, e.read().decode()


def pkce():
    verifier = secrets.token_urlsafe(48)
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
    return verifier, challenge


def sign_in(base, client_id, modules, verifier, challenge, connector="new", name="Phone Claude"):
    """The browser's part: the consent page, and the answer on it. Gives the
    redirect the app gets back."""
    query = urllib.parse.urlencode({
        "response_type": "code", "client_id": client_id, "redirect_uri": CALLBACK, "state": "st-1",
        "code_challenge": challenge, "code_challenge_method": "S256", "resource": f"{base}/mcp",
    })
    status, _, page = http("GET", f"{base}/oauth/authorize?{query}")
    ticket = re.search(r'name="ticket" value="([^"]+)"', page)
    if status != 200 or not ticket:
        return status, page
    if connector == "existing":
        listed = [v for v in re.findall(r'name="connector" value="([^"]+)"', page) if v != "new"]
        connector = listed[0] if listed else "missing"
    form = {"ticket": ticket.group(1), "connector": connector, "name": name, "verdict": "allow"}
    form |= {f"module_{m}": "on" for m in modules}
    status, headers, _ = http("POST", f"{base}/oauth/authorize", form=form, headers={"origin": base}, follow=False)
    return status, headers.get("location", "")


# hippo2 as the door asks it: one answer per call, naming what it was
# asked, for the checks to read.
HIPPO2 = """import os, sys
print("hippo2 asked:", " ".join(sys.argv[1:]), "| HIPPO2_MEMORY:", os.environ.get("HIPPO2_MEMORY", "unset"))
"""


def main():
    global box
    bin_dir = Path(tempfile.mkdtemp(prefix="libri-outside-bin-"))
    hippo2 = bin_dir / "hippo2"
    hippo2.write_text(f"#!{sys.executable}\n{HIPPO2}")
    hippo2.chmod(0o755)
    box = Throwaway(LIBRI_DIR, prefix="libri-outside-",
                    extra={"LIBRI_MEMORY_COMMAND": f"{hippo2} mcp", "HIPPO2_MEMORY": "jobs"})
    root, env, base, mark = box.root, box.env, box.base, box.mark
    try:
        # An app with no token is told where to sign in.
        status, headers, _ = http("POST", f"{base}/mcp", body={"jsonrpc": "2.0", "id": 1, "method": "initialize"},
                                  headers={"accept": "application/json, text/event-stream"})
        challenge_header = headers.get("www-authenticate", "")
        check("an app with no token is sent to sign in", status == 401 and "resource_metadata=" in challenge_header,
              (status, challenge_header))
        status, _, body = http("GET", f"{base}/.well-known/oauth-protected-resource/mcp")
        prm = json.loads(body) if status == 200 else {}
        check("the resource names itself and its authorization server",
              prm.get("resource") == f"{base}/mcp" and prm.get("authorization_servers") == [base], prm)
        status, _, body = http("GET", f"{base}/.well-known/oauth-authorization-server")
        meta = json.loads(body) if status == 200 else {}
        check("the authorization server is described as the spec has it",
              meta.get("issuer") == base and meta.get("code_challenge_methods_supported") == ["S256"]
              and meta.get("client_id_metadata_document_supported") is True
              and meta.get("authorization_response_iss_parameter_supported") is True, meta)

        status, _, body = http("POST", meta["registration_endpoint"],
                               body={"client_name": "Check app", "redirect_uris": ["http://evil.example/cb"]})
        check("an app may not register a plain-http address elsewhere", status == 400, (status, body))
        status, _, body = http("POST", meta["registration_endpoint"],
                               body={"client_name": "Check app", "redirect_uris": ["http://127.0.0.1/callback"],
                                     "token_endpoint_auth_method": "none"})
        client_id = json.loads(body).get("client_id") if status == 201 else None
        check("an app registers", bool(client_id), (status, body))

        status, _, page = http("GET", f"{base}/oauth/authorize?" + urllib.parse.urlencode({
            "response_type": "code", "client_id": client_id, "redirect_uri": "https://evil.example/cb",
            "code_challenge": "x", "code_challenge_method": "S256"}))
        check("an address the app did not register is refused on the page", status == 400, status)
        status, _, page = http("POST", f"{base}/oauth/authorize", form={"ticket": "made-up", "verdict": "allow"},
                               follow=False)
        check("a form posted without the page's ticket lets nobody in", status in (400, 403), status)

        verifier, challenge = pkce()
        status, location = sign_in(base, client_id, ["memory", "workers", "chats"], verifier, challenge)
        back = urllib.parse.urlparse(location)
        got = urllib.parse.parse_qs(back.query)
        check("the person's yes sends the browser back with a code, the state and the issuer",
              status == 303 and location.startswith(CALLBACK) and got.get("state") == ["st-1"]
              and got.get("iss") == [base] and "code" in got, (status, location))
        code = got.get("code", [""])[0]
        status, _, body = http("POST", meta["token_endpoint"], form={
            "grant_type": "authorization_code", "code": code, "client_id": client_id, "redirect_uri": CALLBACK,
            "code_verifier": "not-the-verifier", "resource": f"{base}/mcp"})
        check("a code without its verifier is refused", status == 400 and "invalid_grant" in body, (status, body))
        # Signing in again, the app may come back as the connector it made.
        status, location = sign_in(base, client_id, [], verifier, challenge, connector="existing")
        code = urllib.parse.parse_qs(urllib.parse.urlparse(location).query).get("code", [""])[0]
        status, _, body = http("POST", meta["token_endpoint"], form={
            "grant_type": "authorization_code", "code": code, "client_id": client_id, "redirect_uri": CALLBACK,
            "code_verifier": verifier, "resource": f"{base}/mcp"})
        tokens = json.loads(body) if status == 200 else {}
        check("the code and its verifier get a token", tokens.get("token_type") == "Bearer"
              and tokens.get("access_token") and tokens.get("refresh_token"), (status, body))
        status, _, body = http("POST", meta["token_endpoint"], form={
            "grant_type": "authorization_code", "code": code, "client_id": client_id, "redirect_uri": CALLBACK,
            "code_verifier": verifier})
        check("a code is redeemed once", status == 400, status)

        # Who the assistant is comes from the person's rules, their Identity section.
        http("POST", f"{base}/api/guide", body={"text": "# Identity\nYou are Wren, Ada's assistant.\n\n# Habits\nTea at four.\n"})
        app = Http(f"{base}/mcp", tokens.get("access_token"))
        info = connect(app)["result"]
        check("it names itself libri", info["serverInfo"]["name"] == "libri", info)
        check("it says who the assistant is from the rules' Identity section, and no more of them",
              "You are Wren, Ada's assistant." in info.get("instructions", "")
              and "Tea at four" not in info.get("instructions", ""), info.get("instructions"))
        offered = names(app)
        check("the modules allowed are offered", MEMORY | WORKERS | CHATS <= offered, offered)
        check("a module not allowed is not", not (offered & EXTRAS), offered & EXTRAS)
        check("nothing that acts on the host or answers for the person is offered", not (offered & NEVER),
              offered & NEVER)

        # A worker, started and told under the connector's name.
        refused, started = tool(app, "worker_start", task=f"count the shelves {mark}")
        check("a worker is started", not refused and started.startswith("Started "), started)
        worker = started.removeprefix("Started ").split(".")[0]
        refused, listed = tool(app, "workers_list")
        check("the workers list shows it", not refused and any(w["name"] == worker for w in json.loads(listed)),
              listed)
        refused, read = tool(app, "worker_read", name=worker, limit=5)
        check("its task says which app started it, not the person",
              not refused and "Started by Phone Claude (a connected app), not by the person" in read, read)
        refused, told = tool(app, "worker_tell", name=worker, text="the top shelf too")
        check("a worker is told", not refused and told.startswith("Sent."), told)
        tail = (root / "state").rglob("*")
        check("what it is told comes from the app, not the person",
              any("(From Phone Claude (a connected app), not the person) the top shelf too" in p.read_text(errors="ignore")
                  for p in tail if p.is_file() and p.stat().st_size < 4_000_000), "not found")

        # Words into the main chat land as the app's, and are answered.
        refused, sent = tool(app, "message_send", text=f"the kettle is on the blue shelf {mark}")
        check("a message is sent", not refused, sent)
        heard, cursor = [], json.loads(sent)["next"]
        for _ in range(6):
            refused, text = tool(app, "events_wait", cursor=cursor, timeout_ms=20000)
            got = json.loads(text)
            heard += got["messages"]
            cursor = got["next"]
            if any("claude answers" in m["text"] for m in heard):
                break
        mine = [m for m in heard if "blue shelf" in m["text"]]
        check("the message lands under the connector's name",
              mine and "Phone Claude (a connected app)" in mine[0]["text"], heard)
        check("the assistant answers it", any("claude answers" in m["text"] for m in heard), heard)
        refused, found = tool(app, "memory_recall", words="blue shelf")
        check("the memory is recalled from hippo2's main memory",
              not refused and "asked: --memory main recall blue shelf" in found, found)
        check("the door's recall takes no named memory from its environment", "HIPPO2_MEMORY: unset" in found, found)
        refused, whole = tool(app, "memory_open", ids=[7, 9])
        check("the memory opens messages by their ids", not refused and "asked: --memory main open 7 9" in whole, whole)
        refused, asks = tool(app, "asks_list")
        check("what waits on the person can be seen", not refused and isinstance(json.loads(asks), list), asks)

        # A refresh token is spent once.
        status, _, body = http("POST", meta["token_endpoint"], form={
            "grant_type": "refresh_token", "refresh_token": tokens.get("refresh_token", ""), "client_id": client_id})
        fresh = json.loads(body) if status == 200 else {}
        check("a refresh gets new tokens", bool(fresh.get("access_token")), (status, body))
        status, _, _ = http("POST", meta["token_endpoint"], form={
            "grant_type": "refresh_token", "refresh_token": tokens.get("refresh_token", ""), "client_id": client_id})
        check("a refresh token is spent once", status == 400, status)
        app.token = fresh.get("access_token")

        # Settings: the connector is listed, and its modules change its tools.
        status, _, body = http("GET", f"{base}/api/connectors")
        door = json.loads(body)
        mine = [c for c in door["connectors"] if c["name"] == "Phone Claude"]
        check("Settings lists the connector and the app that signed in",
              len(mine) == 1 and mine[0]["app"] == "Check app" and door["url"] == f"{base}/mcp", door)
        check("Settings never shows a key", "key" not in body and tokens.get("access_token", "x") not in body, body)
        cid = mine[0]["id"] if mine else ""
        http("POST", f"{base}/api/connectors/{cid}", body={"modules": ["memory"]})
        offered = names(app)
        check("a module taken back leaves the list at once", MEMORY <= offered and not (offered & WORKERS), offered)
        refused, text = tool(app, "worker_start", task="one more")
        check("and its tools are refused", refused, text)

        # A connector by key, for an app that only takes a header.
        status, _, body = http("POST", f"{base}/api/connectors", body={"name": "Laptop", "modules": ["memory", "extras"]})
        made = json.loads(body)
        key = made.get("key")
        check("a connector is made by key", status == 200 and key and made["connector"]["modules"] == ["memory", "extras"],
              body)
        keyed = Http(f"{base}/mcp", key)
        connect(keyed)
        offered = names(keyed)
        check("by key it gets its own modules", MEMORY <= offered and not (offered & WORKERS),
              offered)
        check("the extras with something behind them are let in", {"show_page", "pages"} <= offered, offered)
        check("an extra with nothing behind it is left out", not ({"web_search", "x_search", "make_image"} & offered),
              offered)
        refused, shown = tool(keyed, "show_page", name="shelf-plan", html="<!doctype html><p>Shelves</p>")
        check("a page is published", not refused and "shelf-plan" in shown, shown)
        stdio = Stdio(env | {"LIBRI_CONNECTOR_KEY": key})
        check("libri mcp relays the daemon with the key", connect(stdio)["result"]["serverInfo"]["name"] == "libri")
        refused, found = tool(stdio, "memory_recall", words="kettle")
        check("libri mcp recalls from the memory", not refused and "recall kettle" in found, found)
        stdio.proc.terminate()

        # Revoked, each is shut out.
        for c in json.loads(http("GET", f"{base}/api/connectors")[2])["connectors"]:
            http("POST", f"{base}/api/connectors/{c['id']}/revoke")
        for who, client in (("the signed-in app", app), ("the keyed app", keyed)):
            try:
                client.rpc("tools/list", {})
                check(f"revoking shuts out {who}", False)
            except urllib.error.HTTPError as e:
                check(f"revoking shuts out {who}", e.code == 401, e.code)

        # The guard itself: a leak planted in a stand-in home is caught.
        home, hippo = box.home_was, box.hippo_was
        box.home_was, box.hippo_was = str(root / "planted"), None
        (root / "planted/.claude/projects").mkdir(parents=True)
        (root / "planted/.claude/projects/session.jsonl").write_text(f"sent {mark}")
        check("a leak outside the scratch root is caught", box.strays(), "not caught")
        box.home_was, box.hippo_was = home, hippo
        strays = box.strays()
        check("nothing written outside the scratch root", not strays, strays)
    finally:
        log = (root / "libri.log").read_text()[-4000:]
        box.close()
        shutil.rmtree(bin_dir, ignore_errors=True)
    if failures:
        print(box.shown(log))
        sys.exit(f"{len(failures)} failed: {', '.join(failures)}")
    print("all passed")


if __name__ == "__main__":
    main()
