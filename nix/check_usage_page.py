#!/usr/bin/env python3
"""Settings > Usage against a throwaway host whose usage ledger holds a
seeded month: the spend total and every cut come from the host, the span
and cut pickers change what shows, nothing overflows at 390 px, and the
limits draw a bar per window with its pace mark. The limits come from a
stand-in report (the host has no sign-ins here, and the real providers are
never asked). Needs Playwright and Chromium, so it stays out of `nix flake
check`. Usage: check_usage_page.py LIBRI_BIN SITE_DIR SHOTS_DIR
"""

import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright

LIBRI_BIN, SITE_DIR, SHOTS = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
failures = []


def check(what, ok, detail=""):
    print(f"ok   {what}" if ok else f"FAIL {what} {detail}", flush=True)
    if not ok:
        failures.append(what)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def seed(state):
    """A month of calls from every kind of caller, today's among them."""
    now = int(time.time())
    rows = []
    calls = [
        ("main", None, "claude", "anthropic", "claude-opus-5-5", 40_000, 900_000, 30_000, 6_000),
        ("worker", "Ada", "claude", "anthropic", "claude-opus-5-5", 20_000, 400_000, 20_000, 4_000),
        ("worker", "Kit", "omp", "opencode-go", "deepseek-v4.1-flash", 30_000, 200_000, 0, 5_000),
        ("memory", None, "claude", "anthropic", "claude-haiku-5-5", 60_000, 10_000, 0, 3_000),
        ("card", "Ada", "claude", "anthropic", "claude-haiku-5-5", 2_000, 0, 0, 80),
        ("temp", None, "codex", "openai", "gpt-6.1-sol", 15_000, 50_000, 0, 2_000),
        ("search", None, "xai", "xai", "grok-4.7", 8_000, 0, 0, 1_500),
        ("watch", None, "claude", "anthropic", "claude-sonnet-5-5", 10_000, 100_000, 5_000, 900),
    ]
    for day in range(30):
        for i, (by, worker, harness, provider, model, inp, cr, cw, out) in enumerate(calls):
            if (day + i) % 3 == 0 and day:
                continue
            row = {"at": now - day * 86400 - i * 60, "by": by, "harness": harness, "provider": provider,
                   "model": model, "input": inp, "cache_read": cr, "cache_write": cw, "output": out}
            if worker:
                row["worker"] = worker
            if by == "search":
                row["paid"] = "subscription"
            rows.append(row)
    by_month = {}
    for r in rows:
        by_month.setdefault(time.strftime("%Y-%m", time.localtime(r["at"])), []).append(json.dumps(r))
    (state / "usage").mkdir(parents=True)
    for month, lines in by_month.items():
        (state / "usage" / f"{month}.jsonl").write_text("\n".join(lines) + "\n")


def report():
    """What usage.rs reports for a host signed in to Claude and Codex."""
    now = int(time.time())
    def window(id, name, hours, pct, left_hours, model=None):
        at = now + int(left_hours * 3600)
        w = {"window": id, "name": name, "hours": hours, "pct": pct, "resets_at": at,
             "resets": time.strftime("%a %b %-d %H:%M", time.localtime(at))}
        if model:
            w["model"] = model
        return w
    return {"shown": True, "notes": [], "accounts": [
        {"account": "claude", "name": "Claude", "plan": "Max 20x", "read_at": now, "stale": False,
         "windows": [window("5h", "5 hours", 5, 41, 2), window("week", "Week", 168, 77, 48),
                     window("week-fable", "Week · Fable", 168, 4, 48, "Fable")],
         "notes": ["This week by app: Claude Code 97% · Claude Duet 3%"]},
        {"account": "codex", "name": "ChatGPT", "plan": "Pro Lite", "read_at": now - 900, "stale": True,
         "windows": [window("week", "Week", 168, 2, 110)], "notes": ["2 free full resets unused"]},
    ]}


def host(state, port):
    env = {k: v for k, v in os.environ.items() if not k.startswith(("HIPPO_", "LIBRI_", "STATE_", "CLAUDE_", "CODEX_"))}
    env |= {
        # No real sign-in is in reach: the readers find none.
        "HOME": str(state),
        "XDG_DATA_HOME": str(state / "data"),
        "STATE_DIRECTORY": str(state),
        "LIBRI_NAME": "Rook",
        "LIBRI_PERSON": "Sam",
        "LIBRI_CLAUDE": "/bin/false",
        "LIBRI_MODEL": "haiku",
        "LIBRI_SITE": f"127.0.0.1:{port}",
        "LIBRI_SITE_URL": f"http://127.0.0.1:{port}",
        "LIBRI_SITE_ROOT": str(SITE_DIR),
        "LIBRI_OPEN": "1",
    }
    daemon = subprocess.Popen([LIBRI_BIN], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(150):
        try:
            urllib.request.urlopen(f"http://127.0.0.1:{port}/api/host", timeout=2)
            return daemon
        except OSError:
            time.sleep(0.2)
    raise TimeoutError("host up")


def open_usage(page, wide):
    page.evaluate("() => dispatchEvent(new Event('libri:connections'))")
    # A dialog on a wide screen; on a phone a panel under the places, one
    # page at a time, so back to the list of pages first.
    dialog = page.get_by_role("dialog", name="Settings") if wide else page.get_by_role("complementary", name="Settings")
    dialog.wait_for(timeout=20000)
    if not wide:
        dialog.get_by_role("button", name="Back to settings").click(timeout=10000)
    dialog.get_by_role("button", name="Usage").first.click(timeout=10000)
    page.get_by_text("at API prices").first.wait_for(timeout=20000)
    return dialog


def main():
    SHOTS.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        state, port = Path(tmp), free_port()
        seed(state)
        at = f"http://127.0.0.1:{port}"
        daemon = host(state, port)
        try:
            week = json.load(urllib.request.urlopen(f"{at}/api/usage/spend?span=week"))
            month = json.load(urllib.request.urlopen(f"{at}/api/usage/spend?span=month"))
            day = json.load(urllib.request.urlopen(f"{at}/api/usage/spend?span=day"))
            check("the host reads its own ledger", week["total"]["calls"] > 0, week["total"])
            check("a longer span holds more", day["total"]["cost"] < week["total"]["cost"] < month["total"]["cost"],
                  (day["total"]["cost"], week["total"]["cost"], month["total"]["cost"]))
            callers = {r["key"] for r in week["by_caller"]}
            check("each worker is its own row", {"worker:Ada", "worker:Kit"} <= callers, callers)
            check("memory and cards are counted", {"memory", "card"} <= callers, callers)
            makers = [r["name"] for r in week["by_company"]]
            check("makers are named", {"Anthropic", "OpenAI", "DeepSeek", "xAI"} <= set(makers), makers)
            check("a month has a bar a day", len(month["days"]) == 30, len(month["days"]))
            limits = json.load(urllib.request.urlopen(f"{at}/api/usage/report"))
            check("a host with no sign-ins reads no limits", limits["accounts"] == [], limits)

            with sync_playwright() as p:
                browser = p.chromium.launch()
                for name, size, wide in [("1440", {"width": 1440, "height": 1000}, True), ("390", {"width": 390, "height": 844}, False)]:
                    page = browser.new_context(viewport=size, device_scale_factor=2).new_page()
                    page.route(f"{at}/api/usage/report", lambda r: r.fulfill(json=report()))
                    page.goto(at + "/")
                    page.wait_for_selector("textarea", timeout=30000)
                    dialog = open_usage(page, wide)
                    total = page.get_by_text(f"${week['total']['cost']:,.2f}").first
                    check(f"{name}: the week's total shows", total.is_visible())
                    check(f"{name}: a bar per window", page.get_by_text("Week · Fable").count() == 1)
                    check(f"{name}: an old reading says so", page.get_by_text("as of").count() >= 1)
                    over = page.evaluate("() => document.documentElement.scrollWidth > innerWidth")
                    check(f"{name}: nothing runs off the side", not over)
                    region = dialog.get_by_role("region").first if wide else dialog
                    region.screenshot(path=str(SHOTS / f"usage-{name}-limits.png"))
                    page.get_by_text("at API prices").first.scroll_into_view_if_needed()
                    page.wait_for_timeout(300)
                    page.screenshot(path=str(SHOTS / f"usage-{name}-spend.png"))
                    page.get_by_role("radio", name="Maker").click()
                    check(f"{name}: cut by maker", page.get_by_text("DeepSeek").first.is_visible())
                    page.get_by_role("radio", name="30 days").click()
                    page.get_by_text(f"${month['total']['cost']:,.2f}").first.wait_for(timeout=10000)
                    check(f"{name}: the span changes the total", True)
                    page.get_by_role("radio", name="Paid by").scroll_into_view_if_needed()
                    page.wait_for_timeout(300)
                    page.screenshot(path=str(SHOTS / f"usage-{name}-month-maker.png"))
                    page.context.close()
                browser.close()
        finally:
            daemon.terminate()
            daemon.wait()
    if failures:
        sys.exit(f"{len(failures)} failed: " + "; ".join(failures))
    print("all good")


main()
