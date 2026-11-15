#!/usr/bin/env python3
"""The apps cover the host's kinds of tile (tiles.rs): for each kind the web
page and the Android app run, a view (each app's VIEWS) and, for each row its
menu names, a glyph (ROW_ICONS, ROW_GLYPHS) and code that runs it (a
`"<id>": { run … }` the web's places give useTileRows, a `"<id>" to RowCode`
Android's item menu builds); and the rows every tile and bar item ends with.
A kind added to tiles.rs fails here until each app it runs on draws it.
Usage: check_tile_coverage.py LIBRI_BIN REPO
"""
import json
import subprocess
import sys
from pathlib import Path

libri, repo = sys.argv[1], Path(sys.argv[2])
listed = json.loads(subprocess.run([libri, "tiles"], capture_output=True, text=True, check=True).stdout)
site = repo / "libri/site/src"
web_views = (site / "tileViews.tsx").read_text()
web_code = "\n".join(p.read_text() for p in site.glob("*.tsx"))
android_dir = repo / "android/app/src/main/java/su/libri"
android_views = (android_dir / "TileViews.kt").read_text()
android_code = "\n".join(p.read_text() for p in android_dir.glob("*.kt"))

missing = []


def has(text, word):
    return f'"{word}"' in text


def web_runs(row):
    return f'"{row}": {{' in web_code or f'"{row}": newTemp' in web_code or f'"{row}": closable' in web_code


def android_runs(row):
    return f'"{row}" to RowCode' in android_code or f'"{row}" to if' in android_code


views_web = web_views[web_views.index("export const VIEWS"):].split("\n", 1)[0]
views_android = android_views[android_views.index("val VIEWS"):].split("\n", 1)[0]
for t in listed["tiles"]:
    kind, runs = t["kind"], t["runs"]
    if runs["wide"] or runs["phone"]:
        if not has(views_web, kind):
            missing.append(f"web: no view for {kind}")
        for r in t["rows"]:
            if not has(web_views, r["id"]):
                missing.append(f"web: no glyph for {kind}'s row {r['id']}")
            if not web_runs(r["id"]):
                missing.append(f"web: no code for {kind}'s row {r['id']}")
    if runs["phone"]:
        if not has(views_android, kind):
            missing.append(f"android: no view for {kind}")
        for r in t["rows"]:
            if not has(android_views, r["id"]):
                missing.append(f"android: no glyph for {kind}'s row {r['id']}")
            if not android_runs(r["id"]):
                missing.append(f"android: no code for {kind}'s row {r['id']}")
for r in listed["tile_rows"] + listed["bar_rows"]:
    if not has(web_views, r["id"]) or not web_runs(r["id"]):
        missing.append(f"web: no glyph or code for row {r['id']}")
for r in listed["bar_rows"]:
    if not has(android_views, r["id"]) or not android_runs(r["id"]):
        missing.append(f"android: no glyph or code for row {r['id']}")

for m in missing:
    print("FAIL " + m)
kinds = len(listed["tiles"])
print(f"{kinds} kinds, {sum(len(t['rows']) for t in listed['tiles'])} rows: " + ("covered" if not missing else f"{len(missing)} missing"))
sys.exit(1 if missing else 0)
