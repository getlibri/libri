#!/usr/bin/env python3
"""One of Libri's skills, made in Settings, seen by every harness installed
here by its own discovery while skill management is on, and by none while it
is off; the person's own skills untouched throughout. Libri in a scratch HOME
saves a skill through its API with management off, then on, then off, and
each harness, run in that HOME, is asked what skills it has each time. A
skill of the person's in the standard's folder stays seen by all; one of
theirs in Claude Code's folder, by a name Libri also uses, keeps the name.

Each harness answers without a model call or a login, so this spends nothing;
the scratch HOME holds no login and is deleted. Needs real harnesses on the
PATH, so it stays out of `nix flake check`; run it by hand. A harness that
isn't installed is skipped and said to be.

    claude    the init event of `claude -p --output-format stream-json`,
              which lists its skills before it would call the model
    codex     `codex debug prompt-input`, the prompt the model would be given
    opencode  a scratch `opencode serve`, asked GET /api/skill
    muse      `muse skills list`
    omp       `omp skill list`

Usage: check_skills_reach.py LIBRI_DIR [--only a,b] [--keep]
"""

import json
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

LIBRI = Path(sys.argv[1]).resolve() / "bin/libri"
SITE = Path(sys.argv[1]).resolve() / "site"
ONLY = next((a.split("=", 1)[1] if "=" in a else sys.argv[sys.argv.index(a) + 1] for a in sys.argv if a.startswith("--only")), None)
KEEP = "--keep" in sys.argv
NAME = "tide-table-probe"
MINE = "harbour-notes"
CLASH = "weekly-report"
HARNESSES = ["claude", "codex", "opencode", "muse", "omp"]
COMMANDS = {"opencode": os.environ.get("LIBRI_OPENCODE") or shutil.which("opencode") or shutil.which("opencode2")}


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def until(test, what, secs):
    end = time.time() + secs
    while time.time() < end:
        try:
            if test():
                return
        except OSError:
            pass
        time.sleep(0.5)
    raise RuntimeError(f"timed out: {what}")


def call(port, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=data, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read())


def run(cmd, env, cwd, secs=90):
    p = subprocess.run(cmd, env=env, cwd=cwd, capture_output=True, text=True, timeout=secs)
    return p.stdout + p.stderr


def skills_of(h, env, cwd):
    """The skill names harness h finds, by its own discovery."""
    if h == "claude":
        out = run(["claude", "-p", "--output-format", "stream-json", "--verbose", "hi"], env, cwd)
        for line in out.splitlines():
            if '"subtype":"init"' in line:
                return set(json.loads(line).get("skills", []))
        raise RuntimeError(f"claude gave no init event: {out[-400:]}")
    if h == "codex":
        out = run(["codex", "debug", "prompt-input"], env, cwd)
        return set(re.findall(r"- ([a-z0-9-]+): ", out))
    if h == "opencode":
        port = free_port()
        log = open(Path(cwd) / "opencode.log", "w")
        server = subprocess.Popen([COMMANDS["opencode"], "serve", "--port", str(port)], env=env, cwd=cwd, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            text = lambda: (Path(cwd) / "opencode.log").read_text()
            until(lambda: "password" in text(), "opencode up", 60)
            password = re.search(r"password (\S+)", text()).group(1)
            req = urllib.request.Request(f"http://127.0.0.1:{port}/api/skill?location%5Bdirectory%5D={cwd}")
            req.add_header("Authorization", "Basic " + __import__("base64").b64encode(f"opencode:{password}".encode()).decode())
            # The skills are registered a moment after the server answers.
            names = set()
            for _ in range(20):
                with urllib.request.urlopen(req, timeout=30) as r:
                    names = {s["name"] for s in json.loads(r.read())["data"]}
                if len(names) > 2:
                    break
                time.sleep(1)
            return names
        finally:
            os.killpg(server.pid, signal.SIGTERM)
            server.wait()
    if h == "muse":
        return set(re.findall(r"^([a-z0-9-]+)\s", run(["muse", "skills", "list"], env, cwd), re.M))
    if h == "omp":
        return set(re.findall(r"^\s*([a-z0-9-]+)\s", run(["omp", "skill", "list"], env, cwd), re.M))
    raise ValueError(h)


def main():
    root = Path(tempfile.mkdtemp(prefix="libri-skills-"))
    home, state, work = root / "home", root / "state", root / "work"
    for d in (home, state, work):
        d.mkdir()
    subprocess.run(["git", "init", "-q", str(work)], check=True)
    # Claude Code's folder exists on a host with Claude Code; Libri links
    # into it only then.
    (home / ".claude").mkdir()
    env = {
        "PATH": os.environ["PATH"],
        "HOME": str(home),
        "LANG": "C.UTF-8",
        "TZ": "UTC",
    }
    port = free_port()
    daemon = subprocess.Popen(
        [str(LIBRI)],
        env=env | {
            "STATE_DIRECTORY": str(state),
            "LIBRI_NAME": "Libri",
            "LIBRI_PERSON": "Ada",
            "LIBRI_HARNESSES": "claude,codex",
            "LIBRI_BACKEND": "claude",
            "LIBRI_WORKDIR": str(work),
            "LIBRI_OPEN": "1",
            "LIBRI_SITE": f"127.0.0.1:{port}",
            "LIBRI_SITE_URL": f"http://127.0.0.1:{port}",
            "LIBRI_SITE_ROOT": str(SITE),
        },
        cwd=root,
        stdout=subprocess.DEVNULL,
        stderr=open(root / "libri.log", "w"),
        start_new_session=True,
    )
    failed = []
    agents, claude = home / ".agents/skills", home / ".claude/skills"
    # The person's own: one in the standard's folder, one in Claude Code's
    # by a name Libri will also use.
    for folder, name in ((agents, MINE), (claude, CLASH)):
        (folder / name).mkdir(parents=True)
        (folder / name / "SKILL.md").write_text(f"---\nname: {name}\ndescription: The person's own.\n---\nTheirs.\n")
    theirs = {p: p.read_bytes() for p in (agents / MINE / "SKILL.md", claude / CLASH / "SKILL.md")}
    harnesses = [h for h in HARNESSES if not ONLY or h in ONLY.split(",")]
    present = [h for h in harnesses if COMMANDS.get(h) or shutil.which(h)]

    def expect(when, sees):
        for h in present:
            seen = skills_of(h, env, str(work))
            ok = (NAME in seen) == sees
            print(f"{h} {when}: {'sees' if NAME in seen else 'does not see'} {NAME}{'' if ok else '  WRONG'}")
            if not ok:
                failed.append(f"{h} {when}: {sorted(seen)}")
            if h != "claude" and MINE not in seen:
                failed.append(f"{h} {when}: lost the person's {MINE}")

    try:
        until(lambda: call(port, "/api/host"), "libri up", 60)
        for h in sorted(set(harnesses) - set(present)):
            print(f"{h}: not installed, skipped")
        shelf = call(port, "/api/skills", {"name": NAME, "description": "Reading a tide table: use when asked when the tide turns.", "body": "Say the next high tide."})
        call(port, "/api/skills", {"name": CLASH, "description": "Libri's by the same name.", "body": "Libri's."})
        assert not shelf["on"] and [s["name"] for s in shelf["skills"]] == [NAME], shelf
        assert not (agents / NAME).exists() and not (claude / NAME).exists(), "off wrote into the harnesses' folders"
        print(f"off: saved {NAME} in {shelf['dir']}, nothing installed")
        expect("off", False)

        shelf = call(port, "/api/skills/managed", {"on": True})
        assert shelf["on"], shelf
        assert json.loads((agents / NAME / ".libri-managed.json").read_text())["owner"] == "libri"
        assert os.readlink(claude / NAME) == str(agents / NAME)
        assert [c["name"] for c in shelf["clashes"]] == [CLASH], shelf["clashes"]
        assert not (agents / CLASH).exists(), "a clashing skill was installed"
        assert not (home / ".codex/skills" / NAME).exists(), "copied for Codex, which reads the standard's folder"
        print(f"on: {NAME} installed once and linked for Claude Code; {CLASH} left to the person's")
        menu = call(port, "/api/skills/menu")
        for h, m in menu.items():
            got = {s["name"]: s["source"] for s in m["skills"]}
            print(f"menu {h} ({m['mention']}): {got}")
            if got.get(NAME) != "libri":
                failed.append(f"menu {h}: {got}")
        expect("on", True)

        shelf = call(port, "/api/skills/managed", {"on": False})
        assert not (agents / NAME).exists() and not os.path.lexists(claude / NAME), "off left Libri's files"
        assert (state / "skills" / NAME / "SKILL.md").is_file(), "off lost Libri's own copy"
        print(f"off again: {NAME} taken back")
        expect("off again", False)

        call(port, "/api/skills/managed", {"on": True})
        call(port, f"/api/skills/{NAME}/delete", {})
        assert not (agents / NAME).exists() and not os.path.lexists(claude / NAME)
        print(f"deleted with management on: {NAME} taken back")
        for path, was in theirs.items():
            if path.read_bytes() != was:
                failed.append(f"the person's {path} changed")
    finally:
        os.killpg(daemon.pid, signal.SIGTERM)
        daemon.wait()
        if KEEP:
            print(f"kept {root}")
        else:
            shutil.rmtree(root, ignore_errors=True)
    if failed:
        print("FAILED:", *failed, sep="\n  ")
        sys.exit(1)
    print("every installed harness saw Libri's skill only while on, and the person's skills throughout")


if __name__ == "__main__":
    main()
