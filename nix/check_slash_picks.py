#!/usr/bin/env python3
"""The main chat's /model, /effort and /harness read and set the pick as the
composer's pickers do (api/models, api/model, api/effort) and say only what
the host did; a worker thread's pickers set that worker's own choice
(api/work/<id>/choice) and leave the main chat's pick be.

Usage: check_slash_picks.py LIBRI_DIR SITE_DIR SHOTS_DIR
"""
import json
import re
from pathlib import Path
import sys
from throwaway_libri import Throwaway

OFFERS = [
    {"harness": "claude", "model": "opus", "name": "Opus", "provider": "anthropic", "efforts": ["low", "high"], "default": True},
    {"harness": "claude", "model": "sonnet", "name": "Sonnet", "provider": "anthropic", "efforts": ["low", "high"], "default": False},
    {"harness": "codex", "model": "gpt-x", "name": "GPT X", "provider": "openai", "efforts": ["medium"], "default": True},
]
LOG = "\n".join(["0|note: subagent", "1|user: Check configuration.", "2|talk: The check passed."])
failures = []


def check(what, ok, seen=None):
    print(("ok   " if ok else "FAIL ") + what + ("" if ok else f": {seen}"))
    if not ok:
        failures.append(what)


def main():
    from playwright.sync_api import sync_playwright
    shots = Path(sys.argv[3])
    shots.mkdir(parents=True, exist_ok=True)
    rig = Throwaway(sys.argv[1], prefix="libri-slash-picks-", site=sys.argv[2])
    try:
        with sync_playwright() as p:
            browser = p.chromium.launch(headless=True)
            for kind in ["chat", "thread"]:
                main_pick = {"harness": "claude", "model": "opus"}
                state = {"effort": "high", "refuse": False, "posts": [], "worker": {"pick": {"harness": "claude", "model": "opus"}, "effort": "", "access": "auto", "sandbox": None}}
                pane = {"kind": kind, **({"arg": "fixture"} if kind == "thread" else {})}
                layout = {"spaces": [{"id": "space", "name": "Chat", "root": {"t": "leaf", "id": "leaf", "pane": pane}, "focus": "leaf", "full": False}], "on": "space"}

                def reply(route):
                    req = route.request
                    path = req.url.split(rig.base)[-1]
                    body = json.loads(req.post_data or "null") if req.method == "POST" else None
                    if req.method == "POST" and path.split("?")[0] in ("/api/model", "/api/effort", "/api/work/fixture/choice"):
                        state["posts"].append((path, body))
                    if path.startswith("/api/layout"):
                        if "?" in path:
                            return route.fulfill(status=204)
                        data = {"version": 1, "layout": layout, "undo": False}
                    elif path == "/api/me":
                        got = route.fetch()
                        data = {**got.json(), "models": True}
                    elif path == "/api/models":
                        data = {"pick": dict(main_pick), "effort": state["effort"], "temp": {"pick": dict(main_pick), "effort": "", "access": "auto", "sandbox": None},
                                "access": "auto", "sandbox": None, "modes": [], "others": None, "othersOffered": [], "offers": OFFERS}
                    elif path == "/api/model" and req.method == "POST":
                        if state["refuse"]:
                            return route.fulfill(status=400, body="That model is not on offer.")
                        main_pick.update(harness=body["harness"], model=body["model"])
                        return route.fulfill(status=204)
                    elif path == "/api/effort" and req.method == "POST":
                        state["effort"] = body["effort"]
                        return route.fulfill(status=204)
                    elif path == "/api/work":
                        data = {"threads": [{"id": "fixture", "name": "Reviewer", "brief": "Check configuration.", "state": "waiting", "summary": "", "gist": "Check", "at": 0, "read": True}]}
                    elif path == "/api/work/fixture":
                        data = {"text": LOG}
                    elif path == "/api/work/fixture/choice":
                        if req.method == "POST":
                            if body.get("pick"):
                                state["worker"]["pick"] = body["pick"]
                            if "effort" in body:
                                state["worker"]["effort"] = body["effort"]
                        data = state["worker"]
                    else:
                        return route.continue_()
                    route.fulfill(json=data)

                page = browser.new_page(viewport={"width": 1440, "height": 900})
                errors = []
                page.on("pageerror", lambda e: errors.append(str(e)))
                page.route("**/api/**", reply)
                page.goto(rig.base)
                if kind == "chat":
                    box = page.get_by_placeholder(re.compile("^Message ")).first
                    box.wait_for()

                    def say(words, expect):
                        box.fill(words)
                        # The / menu takes Enter while it is open.
                        box.press("Escape")
                        box.press("Enter")
                        try:
                            page.get_by_text(expect).last.wait_for(timeout=10000)
                        except Exception:
                            page.screenshot(path=str(shots / "slash-picks-failed.png"))
                            print(page.locator("main").last.inner_text()[-1500:])
                            raise

                    say("/model", "Running on Opus (claude).")
                    check("/model names the pick, not the default", True)
                    say("/effort", "Thinking at high.")
                    check("/effort names the pick's thinking", True)
                    say("/harness", "Answering through claude.")
                    check("/harness names the pick's harness", True)
                    say("/model sonnet", "Switched to Sonnet, from the next turn on.")
                    check("/model X sets the pick as the picker does", state["posts"][-1] == ("/api/model", {"harness": "claude", "model": "sonnet"}), state["posts"])
                    say("/model", "Running on Sonnet (claude).")
                    check("and the next /model sees it", True)
                    say("/effort low", "Switched to low, from the next turn on.")
                    check("/effort X sets it", state["posts"][-1] == ("/api/effort", {"effort": "low"}), state["posts"])
                    say("/harness codex", "Switched to codex, on GPT X, from the next turn on.")
                    check("/harness X moves to that harness's default model", main_pick == {"harness": "codex", "model": "gpt-x"}, main_pick)
                    say("/model nonesuch", "No model nonesuch.")
                    check("an unknown model is named as such", True)
                    state["refuse"] = True
                    before = dict(main_pick)
                    say("/model opus", "That model is not on offer.")
                    check("a refusal is reported, never a switch", main_pick == before and not page.get_by_text("Switched to Opus").count(), main_pick)
                else:
                    surface = page.locator("[data-thread=fixture]").last
                    surface.wait_for()
                    button = surface.locator('button[title^="Model, run in"]').first
                    button.wait_for(timeout=10000)
                    button.click()
                    page.get_by_role("listbox", name="Models").get_by_text("Sonnet", exact=True).first.click()
                    page.wait_for_timeout(500)
                    check("a worker's model pick goes to its own choice", ("/api/work/fixture/choice", {"pick": {"harness": "claude", "model": "sonnet"}}) in state["posts"], state["posts"])
                    check("and leaves the main chat's pick be", main_pick == {"harness": "claude", "model": "opus"} and not any(p == "/api/model" for p, _ in state["posts"]), (main_pick, state["posts"]))
                page.screenshot(path=str(shots / f"slash-picks-{kind}.png"))
                check(f"no page errors ({kind})", not errors, errors)
                page.close()
            browser.close()
    finally:
        rig.close()
    if failures:
        print(f"{len(failures)} failed: {', '.join(failures)}")
        sys.exit(1)
    print("all pass")


if __name__ == "__main__":
    main()
