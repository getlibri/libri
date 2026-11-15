"""Runs one browser check with the navigation time every browser check gets.

  browser_check_run.py CHECK.py [ARG...]

A page's first load can take past Playwright's 30 s navigation default on a
loaded host: Chromium, not the throwaway Libri, is what is slow then (the
throwaway answers in well under a second at a load of 13). So every context
a check opens, and every page it opens straight from a browser, gives a
navigation FIRST_LOAD_MS before it fails. Only navigations wait longer: a
check's own set_default_timeout still bounds its clicks and waits.
"""

import runpy
import sys
from pathlib import Path

from playwright.sync_api import Browser

FIRST_LOAD_MS = 60_000

new_context, new_page = Browser.new_context, Browser.new_page


def context_with_first_load(self, *args, **kwargs):
    context = new_context(self, *args, **kwargs)
    context.set_default_navigation_timeout(FIRST_LOAD_MS)
    return context


def page_with_first_load(self, *args, **kwargs):
    page = new_page(self, *args, **kwargs)
    page.context.set_default_navigation_timeout(FIRST_LOAD_MS)
    return page


Browser.new_context = context_with_first_load
Browser.new_page = page_with_first_load

check = Path(sys.argv[1]).resolve()
sys.argv = sys.argv[1:]
sys.path.insert(0, str(check.parent))
runpy.run_path(str(check), run_name="__main__")
