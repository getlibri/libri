#!/usr/bin/env python3
"""Settings > Hosts > Peers: one person pairs by address, the other sees the
request and approves it, both see the pair, and revoking it clears it on
both sides, at desktop and phone width.

Usage: check_peers_settings.py LIBRI_DIR SITE_DIR SHOTS_DIR
"""
import sys
import time
from pathlib import Path

from throwaway_libri import Throwaway

failures = []


def check(what, ok, seen=None):
    print(("ok   " if ok else "FAIL ") + what + ("" if ok else f": {seen}"), flush=True)
    if not ok:
        failures.append(what)


def shown(page, text, wait=5000):
    """Whether `text` shows within `wait` ms."""
    try:
        page.get_by_text(text).wait_for(timeout=wait)
        return True
    except Exception:
        return False


def hosts(page, base, width):
    page.goto(base, wait_until="domcontentloaded")
    # Settings: the dock's gear on a wide screen; on a phone, a row of the side panel.
    if width < 800:
        page.get_by_role("button", name="Side panel", exact=True).first.click()
        page.locator("[role=region][aria-label='Side panel'] [data-open=settings]").click()
    else:
        page.get_by_role("button", name="Settings and usage", exact=True).click()
    page.get_by_role("navigation", name="Settings categories").get_by_role("button", name="Hosts").click()
    page.get_by_text("Peers", exact=True).wait_for()


def main():
    from playwright.sync_api import sync_playwright

    libri, site, shots = sys.argv[1], sys.argv[2], Path(sys.argv[3])
    shots.mkdir(parents=True, exist_ok=True)
    for width in (1440, 390):
        a = Throwaway(libri, prefix="peers-ui-a-", site=site, extra={"LIBRI_NAME": "Ann"})
        b = Throwaway(libri, prefix="peers-ui-b-", site=site, extra={"LIBRI_NAME": "Bea"})
        try:
            with sync_playwright() as p:
                browser = p.chromium.launch(headless=True)
                pa = browser.new_page(viewport={"width": width, "height": 900})
                pb = browser.new_page(viewport={"width": width, "height": 900})
                hosts(pa, a.base, width)
                check(f"{width}: an empty list says how to start", shown(pa, "None yet. Pair with another"))
                pa.get_by_role("button", name="Pair with an assistant").click()
                pa.get_by_label("The other assistant's address").fill(b.base)
                begun = time.monotonic()
                pa.get_by_role("button", name="Ask", exact=True).click()
                pa.get_by_text("Waiting for them to approve").wait_for(timeout=5000)
                check(f"{width}: the asker sees its request waiting", True)
                check(f"{width}: asking answers within 400 ms", time.monotonic() - begun < 0.4, time.monotonic() - begun)
                pa.screenshot(path=str(shots / f"asking-{width}.png"))

                hosts(pb, b.base, width)
                pb.get_by_text("Asks to pair").wait_for(timeout=5000)
                check(f"{width}: the other side sees who asks", pb.get_by_text("Ann", exact=True).count() >= 1)
                pb.screenshot(path=str(shots / f"asked-{width}.png"))
                pb.get_by_role("button", name="Approve").click()
                pb.get_by_text("Paired").wait_for(timeout=5000)
                check(f"{width}: approving pairs", True)

                hosts(pa, a.base, width)
                pa.get_by_text("Bea", exact=True).wait_for(timeout=5000)
                check(f"{width}: the asker sees the pair by name", pa.get_by_text("Paired").count() == 1)
                pa.screenshot(path=str(shots / f"paired-{width}.png"))

                pa.get_by_role("button", name="Revoke").click()
                # Libri's own confirm, not the browser's.
                pa.get_by_role("alertdialog").get_by_role("button", name="Unpair").click()
                pa.get_by_text("None yet. Pair with another").wait_for(timeout=5000)
                check(f"{width}: revoking clears it here", True)
                hosts(pb, b.base, width)
                check(f"{width}: and on the other side", shown(pb, "None yet. Pair with another"))
                browser.close()
            check(f"{width}: nothing leaked", not a.strays() and not b.strays())
        finally:
            a.close()
            b.close()
    if failures:
        print(f"{len(failures)} failed: {', '.join(failures)}")
        sys.exit(1)
    print("peers settings pass")


if __name__ == "__main__":
    main()
