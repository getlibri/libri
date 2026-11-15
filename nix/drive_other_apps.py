"""Pictures of Settings > Connections > Other apps on a throwaway Libri, at a
phone's width and a desktop's, light and dark: one connector an app signed
in to by OAuth, one made by key, the key shown once after Add, and the
consent page an app sends the browser to. Usage: drive_other_apps.py
LIBRI_DIR NIX_DIR OUT (Python with playwright, as `nix run .#ui-driver`
has)."""
import json, sys, urllib.parse, urllib.request
from pathlib import Path
sys.path.insert(0, sys.argv[2])
from throwaway_libri import Throwaway
from check_outside import http, pkce, sign_in, CALLBACK
from playwright.sync_api import sync_playwright


def open_connections(page, phone):
    """Settings > Connections, as check_settings_resource.py opens Settings."""
    page.get_by_role("button", name="Settings and usage", exact=True).filter(visible=True).first.click()
    if phone:
        page.get_by_role("button", name="Settings", exact=True).filter(visible=True).first.click()
    page.get_by_role("navigation", name="Settings categories").get_by_role(
        "button", name="Connections", exact=False).click()
    page.get_by_text("Other apps", exact=True).wait_for(timeout=10000)
    page.wait_for_timeout(400)


out = Path(sys.argv[3]); out.mkdir(parents=True, exist_ok=True)
rig = Throwaway(sys.argv[1], prefix="libri-other-apps-ui-")
base = rig.base
try:
    meta = json.loads(http("GET", f"{base}/.well-known/oauth-authorization-server")[2])
    client = json.loads(http("POST", meta["registration_endpoint"],
                             body={"client_name": "Claude", "redirect_uris": [CALLBACK]})[2])["client_id"]
    verifier, challenge = pkce()
    sign_in(base, client, ["memory", "workers"], verifier, challenge, name="Claude on the phone")
    http("POST", f"{base}/api/connectors", body={"name": "Laptop", "modules": ["memory", "extras"]})
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for scheme in ("light", "dark"):
            for name, size in (("phone", (390, 844)), ("desktop", (1280, 900))):
                page = browser.new_page(viewport={"width": size[0], "height": size[1]}, color_scheme=scheme)
                page.goto(base)
                open_connections(page, name == "phone")
                head = page.get_by_text("Other apps", exact=True).locator("visible=true").first
                head.scroll_into_view_if_needed()
                page.evaluate("(h) => h.scrollIntoView({block: 'start'})", head.element_handle())
                page.wait_for_timeout(400)
                page.screenshot(path=out / f"other-apps-{name}-{scheme}.png")
                if name == "phone" and scheme == "light":
                    page.get_by_role("button", name="Add by key").click()
                    page.get_by_placeholder("Claude Code on the laptop").fill("Claude Code on the laptop")
                    page.get_by_role("button", name="Add", exact=True).click()
                    page.get_by_text("Key for Claude Code on the laptop").wait_for(timeout=10000)
                    page.get_by_text("Key for Claude Code on the laptop").scroll_into_view_if_needed()
                    page.evaluate("(h) => h.scrollIntoView({block: 'start'})",
                                  page.get_by_text("Key for Claude Code on the laptop").element_handle())
                    page.wait_for_timeout(300)
                    page.screenshot(path=out / "other-apps-phone-key.png")
                page.close()
            # The consent page an app sends the browser to.
            query = urllib.parse.urlencode({"response_type": "code", "client_id": client, "redirect_uri": CALLBACK,
                                            "state": "s", "code_challenge": challenge, "code_challenge_method": "S256"})
            page = browser.new_page(viewport={"width": 390, "height": 844}, color_scheme=scheme)
            page.goto(f"{base}/oauth/authorize?{query}")
            page.screenshot(path=out / f"consent-phone-{scheme}.png", full_page=True)
            page.close()
        browser.close()
    print("strays", rig.strays())
finally:
    rig.close()
