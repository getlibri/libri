#!/usr/bin/env python3
"""The built-ins and connections (2026-11-06): Mail and Calendar filled by
a connection a provider covers them with, each tool of a connection Allow,
Ask or Off, and a Library page that reads a connection it names.

Settings > Connections offers Fastmail under "Add a connection"; one tap
adds one connection, and Mail and Calendar both take it with nothing to
pick. Turning Mail off withholds Mail's tools on it and leaves Calendar's.
Libri sets no access of its own beyond the per-tool rules: their defaults
come from the server's annotations (reads Allow, unhinted or destructive
Ask), and a tool set Off is withheld.

With no connection, Calendar's view says so and offers "Add a connection",
which opens Settings > Connections.

The page rule: the agent publishes a page naming a stand-in connection, the
page calls its read-only tool and shows the answer; its call of a tool that
changes things is refused, as is any call to a connection it did not name.

The Calendar view: Calendar's Fastmail connection, pointed at a stand-in
that answers list_calendars and search_events as Fastmail's server does;
the connection lists its two calendars, one is hidden, and the view (and the
relay's answer to an agent's own search) leaves its events out.

An ask: a relay, as a harness's child, calls a tool that asks first; the
main chat shows the card, and "Always allow" lets the call through and sets
the tool to Allow.

Drives the real daemon and its real tool server in a scratch directory, a
stand-in MCP server run as a connection, and the built site in Chromium.
Needs Playwright and its Chromium, so it stays out of `nix flake check`.
Usage: check_builtins.py LIBRI_DIR (holding bin/libri and site/) [SHOTS_DIR];
CHECK_BROWSER=firefox or webkit runs it in that browser instead.
"""

import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

LIBRI = Path(sys.argv[1]) / "bin/libri"
SITE = Path(sys.argv[1]) / "site"
SHOTS = Path(sys.argv[2]) if len(sys.argv) > 2 else None
failures = []

# A calendar's MCP server in miniature: one tool that reads, one that changes.
STAND_IN = r'''
import json, sys
TOOLS = [
    {"name": "search_events", "description": "Events.", "inputSchema": {"type": "object"},
     "annotations": {"readOnlyHint": True}},
    {"name": "delete_event", "description": "Delete one.", "inputSchema": {"type": "object"}},
]
for line in sys.stdin:
    m = json.loads(line)
    if "id" not in m:
        continue
    r = {}
    if m["method"] == "initialize":
        r = {"protocolVersion": m["params"]["protocolVersion"], "capabilities": {"tools": {}},
             "serverInfo": {"name": "stand-in", "version": "0"}}
    elif m["method"] == "tools/list":
        r = {"tools": TOOLS}
    elif m["method"] == "tools/call":
        if m["params"]["name"] == "search_events":
            r = {"content": [{"type": "text", "text": "Dentist 09:30"}],
                 "structuredContent": {"events": [{"title": "Dentist", "at": "09:30"}]}}
        else:
            open(sys.argv[1], "w").write("deleted")
            r = {"content": [{"type": "text", "text": "deleted"}]}
    print(json.dumps({"jsonrpc": "2.0", "id": m["id"], "result": r}), flush=True)
'''

# Fastmail's server in miniature, as it answered on 2026-11-06: two
# calendars; events as wall times with a zone and a duration, each on its
# calendars by `calendarIds`; tools annotated as Fastmail's are.
FASTMAIL = r'''
import json, sys, datetime
day = (datetime.date.today() + datetime.timedelta(days=1)).isoformat()
after = (datetime.date.today() + datetime.timedelta(days=2)).isoformat()
CALENDARS = [{"id": "P", "name": "Personal", "color": "#004d40", "isDefault": True},
             {"id": "F", "name": "Family", "color": "", "isDefault": False}]
ITEMS = [
    {"id": "e1", "title": "Dentist", "start": day + "T10:00:00", "duration": "PT1H", "isAllDay": False,
     "timeZone": "America/New_York", "locations": [{"name": "Main St"}], "calendarIds": {"P": True}},
    {"id": "e2", "title": "Standup", "start": after + "T09:00:00", "duration": "PT15M", "isAllDay": False,
     "timeZone": "America/New_York", "locations": [], "calendarIds": {"F": True}},
]
RO = {"readOnlyHint": True}
TOOLS = [
    {"name": "list_calendars", "annotations": RO},
    {"name": "search_events", "annotations": RO},
    {"name": "create_event", "annotations": {"readOnlyHint": False, "destructiveHint": False}},
    {"name": "update_event", "annotations": {"readOnlyHint": False, "destructiveHint": True}},
    {"name": "delete_event"},
]
for t in TOOLS:
    t["inputSchema"] = {"type": "object"}
for line in sys.stdin:
    m = json.loads(line)
    if "id" not in m:
        continue
    r = {}
    if m["method"] == "initialize":
        r = {"protocolVersion": m["params"]["protocolVersion"], "capabilities": {"tools": {}},
             "serverInfo": {"name": "fastmail-stand-in", "version": "0"}}
    elif m["method"] == "tools/list":
        r = {"tools": TOOLS}
    elif m["method"] == "tools/call":
        name = m["params"]["name"]
        data = {"items": ITEMS} if name == "search_events" else {"items": CALENDARS} if name == "list_calendars" else {"done": name}
        r = {"content": [{"type": "text", "text": json.dumps(data)}], "structuredContent": data}
    print(json.dumps({"jsonrpc": "2.0", "id": m["id"], "result": r}), flush=True)
'''


class Relay:
    """A connection's relay as a harness starts it, asking the main chat."""

    def __init__(self, env, state, name):
        self.proc = subprocess.Popen([LIBRI, "connection", str(state), name], env=env,
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.id = 0

    def send(self, method, params):
        self.id += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        return self.id

    def answer(self):
        return json.loads(self.proc.stdout.readline())

    def close(self):
        self.proc.stdin.close()
        self.proc.wait(timeout=10)


# The agenda page: names the connection `desk` and reads it.
AGENDA = """<!doctype html><html><body><ul id="events"></ul><p id="refused"></p><p id="other"></p>
<script>
let n = 0; const waiting = {};
addEventListener("message", (e) => { const m = e.data; if (m && m.id in waiting) { waiting[m.id](m.result || m.error); delete waiting[m.id]; } });
const ask = (method, params) => new Promise((ok) => { const id = ++n; waiting[id] = ok; parent.postMessage({ jsonrpc: "2.0", id, method, params }, "*"); });
(async () => {
  await ask("ui/initialize", { appInfo: { name: "agenda", version: "1" }, appCapabilities: {}, protocolVersion: "2026-01-26" });
  parent.postMessage({ jsonrpc: "2.0", method: "ui/notifications/initialized" }, "*");
  const r = await ask("tools/call", { name: "mcp__desk__search_events", arguments: {} });
  for (const ev of (r.structuredContent || {}).events || []) {
    const li = document.createElement("li"); li.textContent = ev.title + " " + ev.at; document.getElementById("events").append(li);
  }
  const d = await ask("tools/call", { name: "mcp__desk__delete_event", arguments: {} });
  document.getElementById("refused").textContent = d.isError ? "refused" : "allowed";
  const o = await ask("tools/call", { name: "mcp__other__search_events", arguments: {} });
  document.getElementById("other").textContent = o.isError ? "refused" : "allowed";
})();
</script></body></html>"""


def check(what, ok, detail=""):
    print(f"ok   {what}" if ok else f"FAIL {what} {detail}", flush=True)
    if not ok:
        failures.append(what)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def until(test, what, limit=30.0):
    end = time.time() + limit
    while time.time() < end:
        if test():
            return
        time.sleep(0.1)
    raise SystemExit(f"timed out: {what}")


class Tools:
    """Libri's tools at the person's scope, as a turn's harness reaches them."""

    def __init__(self, env, state):
        self.proc = subprocess.Popen([LIBRI, "tools", str(state)], env=env | {"LIBRI_SCOPE": "person"},
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
        r = self.rpc("tools/call", {"name": name, "arguments": arguments})["result"]
        return r["content"][0]["text"], r.get("isError", False)


def page_frame(page, selector):
    for f in page.frames:
        try:
            if f.query_selector(selector) is not None:
                return f
        except Exception:
            pass
    return None


def api(url, path, body=None):
    req = urllib.request.Request(f"{url}/{path}", data=None if body is None else json.dumps(body).encode(),
                                 headers={"content-type": "application/json"}, method="GET" if body is None else "POST")
    with urllib.request.urlopen(req, timeout=10) as r:
        text = r.read().decode()
        return json.loads(text) if text else None


def connection(url, name):
    return next((s for s in api(url, "api/connections")["servers"] if s["name"] == name), None)


def open_connections(page):
    # On a phone, Settings sits in the side panel.
    settings = page.get_by_role("button", name="Settings", exact=True).locator("visible=true")
    panel = page.get_by_role("button", name="Side panel").locator("visible=true")
    # The book loader shows until the host answers.
    until(lambda: settings.count() or panel.count(), "the chat drawn", 30)
    if settings.count() == 0:
        page.get_by_role("button", name="Side panel").click()
        page.wait_for_timeout(400)
    settings.first.click()
    page.wait_for_timeout(400)
    page.get_by_role("button", name="Connections", exact=False).locator("visible=true").first.click()
    page.get_by_text("Built-ins", exact=True).wait_for(timeout=10000)
    page.wait_for_timeout(400)


def open_calendar(page, phone):
    if phone:
        until(lambda: page.get_by_role("button", name="Side panel").locator("visible=true").count(), "the phone chat drawn", 30)
        page.get_by_role("button", name="Side panel").locator("visible=true").first.click()
        page.wait_for_timeout(400)
    page.get_by_role("navigation", name="Places").get_by_role("button", name="Calendar").click()
    page.wait_for_timeout(400)


def main():
    root = Path(tempfile.mkdtemp(prefix="libri-builtins-"))
    for d in ("home", "state", "hippo"):
        (root / d).mkdir()
    (root / "stand_in.py").write_text(STAND_IN)
    (root / "fastmail.py").write_text(FASTMAIL)
    port = free_port()
    url = f"http://127.0.0.1:{port}"
    env = {
        "HOME": str(root / "home"),
        "PATH": os.environ["PATH"],
        "HIPPO_DIR": str(root / "hippo"),
        "STATE_DIRECTORY": str(root / "state"),
        "LIBRI_NAME": "box-label",
        "LIBRI_PERSON": "Ada",
        "LIBRI_PAGES": str(root / "state/pages"),
        "LIBRI_PAGES_URL": url,
    }
    daemon = subprocess.Popen([LIBRI], env=env | {
        "LIBRI_BACKEND": "claude",
        "LIBRI_CLAUDE": "/bin/false",
        "LIBRI_MODEL": "stand-in",
        "LIBRI_SITE": f"127.0.0.1:{port}",
        "LIBRI_SITE_URL": url,
        "LIBRI_SITE_ROOT": str(SITE),
        "LIBRI_OPEN": "1",
    }, stdout=subprocess.DEVNULL, stderr=open(root / "libri.log", "w"))

    def up():
        try:
            return urllib.request.urlopen(f"{url}/api/host", timeout=2).status == 200
        except OSError:
            return False

    marker = root / "deleted"
    try:
        until(up, "libri up")
        b = {x["id"]: x for x in api(url, "api/builtins")}
        check("Mail and Calendar are the built-ins, both on, no connection yet",
              sorted(b) == ["calendar", "mail"] and all(x["on"] and x["connection"] is None for x in b.values()), b)

        # The page rule, on a connection of the person's own.
        api(url, "api/connections/servers", {"name": "desk", "address": f"{sys.executable} {root / 'stand_in.py'} {marker}"})
        tools = Tools(env, root / "state")
        said, bad = tools.call("show_page", {"name": "agenda", "html": AGENDA, "connections": ["desk"]})
        check("the agent publishes a page naming the connection", not bad, said)
        said, bad = tools.call("show_page", {"name": "bad", "html": "x", "connections": ["no good"]})
        check("a connection name that isn't one is refused", bad, said)

        with sync_playwright() as p:
            browser = getattr(p, os.environ.get("CHECK_BROWSER", "chromium")).launch()
            page = browser.new_page(viewport={"width": 1440, "height": 900})
            errors = []
            page.on("pageerror", lambda e: errors.append(str(e)))
            page.goto(f"{url}/#page=agenda")
            until(lambda: page_frame(page, "#other") is not None and page_frame(page, "#other").inner_text("#other") != "",
                  "the page's three calls answered", 40)
            f = page_frame(page, "#other")
            check("the page reads the connection it names", f.inner_text("#events").strip() == "Dentist 09:30", f.inner_text("#events"))
            check("its call of a tool that changes things is refused",
                  f.inner_text("#refused") == "refused" and not marker.exists(), f.inner_text("#refused"))
            check("a connection it did not name is refused", f.inner_text("#other") == "refused")
            if SHOTS:
                SHOTS.mkdir(parents=True, exist_ok=True)
                page.screenshot(path=SHOTS / "1-agenda-page.png")

            # Calendar with no connection: its one way forward.
            page.close()
            page = browser.new_page(viewport={"width": 1440, "height": 900})
            page.on("pageerror", lambda e: errors.append(str(e)))
            page.goto(url, wait_until="domcontentloaded")
            page.get_by_role("navigation", name="Places").wait_for(timeout=30000)
            open_calendar(page, False)
            add = page.get_by_role("button", name="Add a connection")
            add.wait_for(timeout=10000)
            check("Calendar with no connection says so", page.get_by_text("No connection yet").count() == 1)
            if SHOTS:
                page.screenshot(path=SHOTS / "2a-calendar-empty-1440.png")
            add.click()
            offer = page.get_by_role("group", name="Add a connection")
            offer.wait_for(timeout=10000)
            check("its button opens Settings > Connections at Add a connection", offer.get_by_text("Fastmail").count() == 1)
            # Google and Microsoft cannot sign in yet: not offered at all.
            check("Fastmail is the one provider offered",
                  [p["id"] for p in api(url, "api/connections/providers")] == ["fastmail"]
                  and page.get_by_text("Google").count() == 0 and page.get_by_text("Microsoft").count() == 0)
            check("no provider to pick under a built-in", page.get_by_text("Use Fastmail").count() == 0
                  and page.get_by_text("No connection yet: add one above").count() == 2)
            if SHOTS:
                page.screenshot(path=SHOTS / "2-connections-1440.png")
            try:
                api(url, "api/connections/providers/google", {})
                refused = False
            except urllib.error.HTTPError as e:
                refused = e.code == 400 and "yet" in e.read().decode()
            check("the host refuses a provider that cannot sign in yet", refused)
            # One tap: the connection, and its sign-in in a tab of its own.
            with page.context.expect_page() as tab:
                offer.get_by_role("button", name="Fastmail").click()
            tab.value.close()
            until(lambda: connection(url, "fastmail") is not None, "the Fastmail connection added", 10)
            c = connection(url, "fastmail")
            check("one tap adds one connection at Fastmail's address",
                  c["address"] == "https://api.fastmail.com/mcp" and c["on"] and c["withheld"] == [], c)
            b = {x["id"]: x for x in api(url, "api/builtins")}
            check("Mail and Calendar both take it, nothing picked",
                  all(x["connection"] == "fastmail" and len(x["choices"]) == 1 for x in b.values()), b)
            page.wait_for_timeout(600)
            check("the built-ins say where they come from",
                  page.get_by_text("From Fastmail · not signed in").count() == 2, page.get_by_text("From Fastmail").all_inner_texts())
            page.get_by_role("switch", name="Mail on", exact=True).click()
            until(lambda: "search_email" in connection(url, "fastmail")["withheld"], "Mail's tools withheld", 10)
            c = connection(url, "fastmail")
            check("Mail off withholds its tools, sending among them, and leaves Calendar's",
                  "send_email" in c["withheld"] and "search_events" not in c["withheld"] and c["on"], c["withheld"])
            page.get_by_role("switch", name="Calendar on", exact=True).click()
            until(lambda: not connection(url, "fastmail")["on"], "the connection off", 10)
            check("with both off, the connection is off", True)
            page.get_by_role("switch", name="Calendar on", exact=True).click()
            page.get_by_role("switch", name="Mail on", exact=True).click()
            until(lambda: connection(url, "fastmail")["on"] and connection(url, "fastmail")["withheld"] == [],
                  "both back on", 10)
            check("turned back on, they give back all they took", True)
            if SHOTS:
                page.screenshot(path=SHOTS / "3-connections-fastmail-1440.png")
            errors = [e for e in errors if "Both frames are sandboxed" not in e]
            check("1440: no page errors", not errors, "; ".join(errors))

            # Calendar's connection, pointed at the Fastmail stand-in: the
            # agenda goes by the connection's name, not its address.
            c = connection(url, "fastmail")
            api(url, "api/connections/servers", {"name": "fastmail", "address": f"{sys.executable} {root / 'fastmail.py'}",
                                                 "withheld": c["withheld"], "on": True})
            check("an edited address keeps Mail and Calendar on it",
                  all(x["connection"] == "fastmail" for x in api(url, "api/builtins")))

            # Its tools, from the server's annotations, and what it brings in.
            page.goto(url, wait_until="domcontentloaded")
            open_connections(page)
            page.get_by_role("button", name="Test").last.click()
            until(lambda: len(connection(url, "fastmail")["tools"]) == 5, "the tools listed", 20)
            rules = {t["name"]: t["rule"] for t in connection(url, "fastmail")["tools"]}
            check("defaults from annotations: reads Allow, every write Ask",
                  rules == {"list_calendars": "allow", "search_events": "allow", "create_event": "ask",
                            "update_event": "ask", "delete_event": "ask"}, rules)
            page.get_by_role("button", name="5 tools · 3 ask first").click()
            page.get_by_role("radiogroup", name="delete_event").get_by_role("radio", name="Off").click()
            until(lambda: "delete_event" in connection(url, "fastmail")["withheld"], "delete_event off", 10)
            check("Off withholds the tool", connection(url, "fastmail")["withheld"] == ["delete_event"])
            shows = page.get_by_role("group", name="fastmail Calendars")
            shows.wait_for(timeout=20000)
            check("the connection lists its calendars", shows.get_by_role("switch").count() == 2)
            shows.get_by_role("switch", name="Show Family").click()
            until(lambda: not page.get_by_role("switch", name="Show Family").is_checked(), "Family hidden", 10)
            if SHOTS:
                page.screenshot(path=SHOTS / "3c-tools-calendars-1440.png", full_page=True)

            phone = browser.new_page(viewport={"width": 390, "height": 844})
            phone.goto(url)
            open_connections(phone)
            phone.get_by_role("button", name="5 tools", exact=False).click()
            wide = phone.evaluate("document.documentElement.scrollWidth")
            check("390: Connections fits the phone's width", wide <= 390, wide)
            if SHOTS:
                phone.screenshot(path=SHOTS / "4-connections-390.png", full_page=True)

            # An ask, as a harness's relay makes one: the main chat's card.
            page.goto(url, wait_until="domcontentloaded")
            page.get_by_role("textbox").first.wait_for(timeout=30000)
            # The chat's event stream open, as a turn's would be.
            page.wait_for_timeout(1500)
            relay = Relay(env, root / "state", "fastmail")
            relay.send("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "check", "version": "0"}})
            relay.answer()
            relay.send("tools/list", {})
            relay.answer()
            relay.send("tools/call", {"name": "update_event", "arguments": {"id": "e1", "title": "Dentist, moved"}})
            card = page.get_by_role("article", name="May update_event run?")
            card.wait_for(timeout=20000)
            check("an ask shows Deny, Always allow and Allow once",
                  [card.get_by_role("button", name=n, exact=True).count() for n in ("Deny", "Always allow", "Allow once")] == [1, 1, 1])
            if SHOTS:
                page.screenshot(path=SHOTS / "4b-ask-1440.png")
            card.get_by_role("button", name="Always allow").click()
            got = relay.answer()
            check("the call runs once allowed", got["result"].get("structuredContent") == {"done": "update_event"}, got)
            check("Always allow sets the tool to Allow",
                  {t["name"]: t["rule"] for t in connection(url, "fastmail")["tools"]}["update_event"] == "allow")
            relay.send("tools/call", {"name": "delete_event", "arguments": {}})
            got = relay.answer()
            check("a tool set Off is refused", got["result"].get("isError") is True, got)
            relay.send("tools/call", {"name": "search_events", "arguments": {}})
            got = relay.answer()
            check("an agent's own search leaves out the hidden calendar's events",
                  [e["title"] for e in got["result"]["structuredContent"]["items"]] == ["Dentist"], got)
            relay.close()

            # The side panel and the view, wide.
            page.goto(url, wait_until="domcontentloaded")
            places = page.get_by_role("navigation", name="Places")
            places.wait_for(timeout=10000)
            check("the side panel lists Chat and Calendar", places.get_by_role("button").all_inner_texts() == ["Chat", "Calendar"],
                  places.get_by_role("button").all_inner_texts())
            places.get_by_role("button", name="Calendar").click()
            until(lambda: page_frame(page, "#days li") is not None, "the agenda drawn", 30)
            f = page_frame(page, "#days")
            text = f.inner_text("#days")
            check("Calendar shows the shown calendar's events alone", "Dentist" in text and "Main St" in text and "Standup" not in text, text)
            if SHOTS:
                page.screenshot(path=SHOTS / "5-calendar-1440.png")
            places.get_by_role("button", name="Chat").click()
            check("Chat comes back", page.get_by_role("textbox").first.is_visible())

            # On a phone: the side panel's Calendar.
            phone.goto(url, wait_until="domcontentloaded")
            until(lambda: phone.get_by_role("button", name="Side panel").locator("visible=true").count(), "the phone chat drawn", 30)
            phone.get_by_role("button", name="Side panel").locator("visible=true").first.click()
            phone.wait_for_timeout(400)
            phone.get_by_role("navigation", name="Places").get_by_role("button", name="Calendar").click()
            until(lambda: page_frame(phone, "#days li") is not None, "the phone agenda drawn", 30)
            check("390: Calendar shows the events", "Dentist" in page_frame(phone, "#days").inner_text("#days"))
            wide = phone.evaluate("document.documentElement.scrollWidth")
            check("390: the view fits", wide <= 390, wide)
            lit = phone.locator('button[aria-current="page"]:visible').all_inner_texts()
            check("390: with Calendar open the bottom bar lights nothing", "Chat" not in lit, lit)
            if SHOTS:
                phone.screenshot(path=SHOTS / "6-calendar-390.png")

            # The app's paper (`&paper=`), so the view is no pale box on it.
            alone = browser.new_page(viewport={"width": 390, "height": 844})
            alone.goto(f"{url}/#builtin-alone=calendar&paper=E6D8C1", wait_until="domcontentloaded")
            until(lambda: page_frame(alone, "#days li") is not None, "the view alone drawn", 30)
            bg = page_frame(alone, "#days").evaluate("getComputedStyle(document.body).backgroundColor")
            check("a view alone takes the app's paper", bg.replace(" ", "") == "rgb(230,216,193)", bg)
            alone.close()

            # Dark, as the person's stored theme.
            if SHOTS:
                for w, h, name in ((1440, 900, "1440"), (390, 844, "390")):
                    dark = browser.new_page(viewport={"width": w, "height": h})
                    dark.add_init_script("localStorage.setItem('theme', 'dark')")
                    dark.goto(url, wait_until="domcontentloaded")
                    open_connections(dark)
                    dark.screenshot(path=SHOTS / f"7-builtins-dark-{name}.png", full_page=True)
                    dark.goto(url, wait_until="domcontentloaded")
                    if w < 1024:
                        until(lambda: dark.get_by_role("button", name="Side panel").locator("visible=true").count(), "the dark chat drawn", 30)
                        dark.get_by_role("button", name="Side panel").locator("visible=true").first.click()
                        dark.wait_for_timeout(400)
                    dark.get_by_role("navigation", name="Places").get_by_role("button", name="Calendar").click()
                    until(lambda: page_frame(dark, "#days li") is not None, "the dark agenda drawn", 30)
                    dark.wait_for_timeout(300)
                    dark.screenshot(path=SHOTS / f"8-calendar-dark-{name}.png")
                    dark.close()

            # Off, it leaves the side panel.
            api(url, "api/builtins/calendar", {"on": False})
            page.goto(url, wait_until="domcontentloaded")
            places = page.get_by_role("navigation", name="Places")
            places.wait_for(timeout=10000)
            page.wait_for_timeout(800)
            check("Calendar off leaves the side panel", places.get_by_role("button").all_inner_texts() == ["Chat"],
                  places.get_by_role("button").all_inner_texts())
            errors = [e for e in errors if "Both frames are sandboxed" not in e]
            check("no page errors after", not errors, "; ".join(errors))
            browser.close()
    finally:
        daemon.terminate()
        daemon.wait()

    print(f"{'FAILED' if failures else 'passed'}: {len(failures)} failures", flush=True)
    sys.exit(1 if failures else 0)


main()
