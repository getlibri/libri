"""Drives the Workers tree's cards in Chromium on a throwaway Libri: each
worker's colour by what it does, its model, title and status, its lane's
glow while it works, and the lanes running dashes while a head and its
sub-worker message each other. Usage: drive_worker_cards.py LIBRI_DIR NIX_DIR OUT
(Python with playwright, as `nix run .#ui-driver` has)."""
import json, subprocess, sys, time, urllib.request
from pathlib import Path
sys.path.insert(0, sys.argv[2])
from throwaway_libri import Throwaway, until
from playwright.sync_api import sync_playwright

out = Path(sys.argv[3]); out.mkdir(parents=True, exist_ok=True)
rig = Throwaway(sys.argv[1], prefix="libri-cards-ui-", extra={"LIBRI_WORK_TICK": "1"})


class Agents:
    """Libri's tools at the main chat's scope, as its harness reaches them."""

    def __init__(self):
        env = rig.env | {"LIBRI_SCOPE": "person", "STATE_DIRECTORY": str(rig.root / "state"),
                         "LIBRI_NAME": "box-label", "LIBRI_PERSON": "Ada"}
        self.proc = subprocess.Popen([str(rig.libri), "tools", str(rig.root / "state")], env=env, text=True,
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.id = 0
        self.call("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "check", "version": "0"}})
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.proc.stdin.flush()

    def call(self, method, params):
        self.id += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        return json.loads(self.proc.stdout.readline())

    def tool(self, called, **a):
        return self.call("tools/call", {"name": called, "arguments": a})["result"]["content"][0]["text"]

    def spawn(self, task):
        return self.tool("spawn", tasks=[{"task": task}]).split()[1].strip(".,:")


def get(path):
    return json.loads(urllib.request.urlopen(rig.base + path, timeout=20).read())
def thread(name):
    return next((t for t in get("/api/work")["threads"] if t["name"] == name), None)


agents = Agents()
try:
    with sync_playwright() as p:
        b = p.chromium.launch(headless=True)
        page = b.new_page(viewport={"width": 1440, "height": 900})
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.goto(rig.base + "/#work")
        page.locator("[data-surface=worker-tree]").wait_for(timeout=20000)
        names = {act: agents.spawn(task) for act, task in [
            ("reading", "TOOL Read 400"), ("editing", "TOOL Edit 400"), ("running", "TOOL Bash 400"),
            ("thinking", "WORK SLOWLY on the release notes"), ("failed", "CRASH"), ("person", "NEED Which board first?")]}
        head = agents.spawn("ASK CHAT HEAD")
        said = [""]
        def approved():
            said[0] = agents.tool("answer_subworker_request", name=head, approve=True)
            return said[0].startswith("Started ")
        until(approved, f"{head} gets its sub-worker", wait=90)
        sub = said[0].split()[1].strip(".,:")
        print("workers:", names, "head:", head, "sub:", sub)
        until(lambda: thread(names["failed"]).get("failed"), "a run fails", wait=60)
        until(lambda: thread(names["person"])["state"] == "waiting", "a worker asks the person", wait=60)
        work = get("/api/work")
        print("talk:", json.dumps([(t["from"], t["to"]) for t in work["talk"]]))
        print("picks:", json.dumps({t["name"]: t.get("pick") for t in work["threads"]}))

        ids = {act: thread(n)["id"] for act, n in names.items()}
        acts = lambda: page.evaluate("Object.fromEntries([...document.querySelectorAll('[data-row]')].map(e=>[e.dataset.row,e.dataset.act]))")
        try:
            until(lambda: all(acts().get(i) == a for a, i in ids.items()), "every worker shows what it does", wait=60)
        finally:
            print("want:", json.dumps({i: a for a, i in ids.items()}), "acts:", json.dumps(acts()))
            print("threads:", json.dumps([(t["name"], t["state"], t.get("waiting_on"), t.get("failed")) for t in get("/api/work")["threads"]]))
        glows = page.evaluate("[...document.querySelectorAll('[data-glow]')].map(e=>e.dataset.lane)")
        print("glowing lanes:", json.dumps(glows))
        assert set(glows) >= {ids[a] for a in ("reading", "editing", "running", "thinking")}, glows
        assert not set(glows) & {ids["failed"], ids["person"]}, glows

        # The main chat writes to the head, and the sub-worker to its head
        # until the chain's hops run out: the lanes between them run dashes.
        head_id, sub_id = thread(head)["id"], thread(sub)["id"]
        # The spawns' briefs and the reports to Main run their dashes first.
        page.locator("[data-surface=worker-tree]").screenshot(path=out / "cards-0-spawned.png")
        until(lambda: page.evaluate("document.querySelectorAll('[data-flow]').length") == 0, "the tree goes quiet", wait=30)
        agents.tool("tell_agent", name=sub, message="CHAT HEAD again please")
        agents.tool("tell_agent", name=head, message="how goes it?")
        seen, shot = {}, False
        end = time.time() + 40
        while time.time() < end:
            flows = page.evaluate("Object.fromEntries([...document.querySelectorAll('[data-flow]')].map(e=>[e.dataset.lane,e.dataset.flow]))")
            for k, v in flows.items():
                seen.setdefault(k, set()).add(v)
            if not shot and sub_id in flows and head_id in flows:
                page.locator("[data-surface=worker-tree]").screenshot(path=out / "cards-2-talk.png")
                shot = True
            page.wait_for_timeout(250)
        print("flows seen:", json.dumps({k: sorted(v) for k, v in seen.items()}))
        assert head_id in seen and "down" in seen[head_id], seen
        assert sub_id in seen, seen
        assert not any(i in seen for i in ids.values() if i not in (head_id, sub_id)), "ordinary work ran dashes"
        page.wait_for_timeout(5000)
        still = page.evaluate("document.querySelectorAll('[data-flow]').length")
        print("dashes after the talk:", still)

        card = page.locator(f"[data-row='{ids['reading']}']")
        print("card:", json.dumps({k: card.locator(f"[data-{k}]").inner_text() for k in ("model", "title", "status")}))
        page.locator("[data-surface=worker-tree]").screenshot(path=out / "cards-1-tree.png")
        page.locator("[data-legend]").hover()
        page.wait_for_timeout(300)
        page.screenshot(path=out / "cards-3-legend.png", clip={"x": 0, "y": 0, "width": 1440, "height": 900})

        phone = b.new_page(viewport={"width": 390, "height": 844})
        phone.on("pageerror", lambda e: errors.append(str(e)))
        phone.goto(rig.base + "/#work")
        phone.wait_for_timeout(1500)
        if not phone.locator("[data-surface=worker-tree]").is_visible():
            phone.get_by_role("tab", name="Workers").or_(phone.get_by_text("Workers", exact=True)).first.click()
        phone.locator("[data-surface=worker-tree]").wait_for(timeout=20000)
        phone.wait_for_timeout(700)
        # Loaded after the calls began, it colours them by the host's word.
        phone_acts = phone.evaluate("Object.fromEntries([...document.querySelectorAll('[data-row]')].map(e=>[e.dataset.row,e.dataset.act]))")
        print("phone acts:", json.dumps({k: phone_acts.get(i) for k, i in ids.items()}))
        assert all(phone_acts.get(i) == a for a, i in ids.items() if a != "thinking"), phone_acts
        phone.screenshot(path=out / "cards-4-phone.png")
        print("errors:", errors)
        assert not errors
        b.close()
    print("strays:", rig.strays())
finally:
    agents.proc.terminate()
    rig.close()
