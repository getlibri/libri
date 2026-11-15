"""A throwaway Libri for checks and proofs: the real daemon and a real hippo,
with every directory they write inside one scratch root, and the stand-in
`claude` as the harness.

Whatever a run sends carries `mark`, a word made fresh for the run and never
printed. On the way out `strays()` looks for it where a run must not write:
the caller's home (where the stock apps keep their sessions) and the caller's
own memory, if it has one. Finding it there means the run leaked into a real
Libri, and the check fails.
"""

import os
import secrets
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

STAND_IN = Path(__file__).with_name("stand_in_claude.py").read_text()
# Build trees: big, rewritten all the time, and never where a session lands.
SKIP = {"target", "node_modules", ".git", ".worktrees", "result"}
BIGGEST = 64 << 20


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def until(test, what, wait=30):
    end = time.monotonic() + wait
    while time.monotonic() < end:
        if test():
            return
        time.sleep(0.1)
    raise TimeoutError(what)


class Throwaway:
    def __init__(self, libri_dir, prefix="libri-throwaway-", site=None, extra=None):
        self.libri_dir = Path(libri_dir)
        self.libri = self.libri_dir / "bin/libri"
        self.hippo_bin = self.libri_dir / "bin/hippo"
        self.mark = "mark" + secrets.token_hex(8)
        self.began = time.time()
        # Where a leak would land: kept before the run's own settings hide them.
        self.home_was = os.environ.get("HOME")
        self.hippo_was = os.environ.get("HIPPO_DIR")
        self.root = Path(tempfile.mkdtemp(prefix=prefix)).resolve()
        for name in ("hippo", "home", "state", "pages", "work"):
            (self.root / name).mkdir()
        claude = self.root / "claude"
        claude.write_text(f"#!{sys.executable}\n{STAND_IN}")
        claude.chmod(0o755)
        self.port = free_port()
        self.base = f"http://127.0.0.1:{self.port}"
        env = {k: v for k, v in os.environ.items() if not k.startswith(("HIPPO_", "LIBRI_", "STATE_", "XDG_"))}
        env |= {
            "HOME": str(self.root / "home"),
            "HIPPO_DIR": str(self.root / "hippo"),
            "HIPPO_ARCHIVE": str(self.root / "archive"),
            "HIPPO_BACKEND": "none",
            "STAND_IN_LOG": str(self.root / "stand-in.jsonl"),
            "TMPDIR": str(self.root / "work"),
            # The host's pressure is not a throwaway's news (system.rs).
            "LIBRI_SYSTEM_ALERTS": "0",
        }
        self.env = env
        # Libri's record alone, as Libri keeps its hippo: the harnesses'
        # own sessions are the memory server's to read.
        self.hippo = subprocess.Popen([self.hippo_bin, "serve", "chat"], env=env, stdout=subprocess.DEVNULL,
                                      stderr=open(self.root / "hippo.log", "w"))
        until(lambda: (self.root / "hippo/hippo.sock").exists(), "hippo up")
        env |= {
            "STATE_DIRECTORY": str(self.root / "state"),
            "LIBRI_NAME": "box-label",
            "LIBRI_PERSON": "Ada",
            # A shared chat, which hippo keeps from what Libri logs.
            "LIBRI_HIPPO_SHARED": "Iris",
            # The memory's server, never started by the stand-in harness:
            # its entry is what the checks read.
            "LIBRI_MEMORY_COMMAND": "memory-stand-in mcp",
            "LIBRI_BACKEND": "claude",
            "LIBRI_CLAUDE": str(claude),
            "LIBRI_MODEL": "stand-in",
            "LIBRI_SITE": f"127.0.0.1:{self.port}",
            "LIBRI_SITE_URL": self.base,
            # Another build of the page may be served, as drive_ui.py does a branch's.
            "LIBRI_SITE_ROOT": str(site or self.libri_dir / "site"),
            "LIBRI_OPEN": "1",
            "LIBRI_PAGES": str(self.root / "pages"),
            "LIBRI_PAGES_URL": "https://libri.example.com/",
        }
        env |= extra or {}
        self.daemon = subprocess.Popen([self.libri], env=env, stdout=subprocess.DEVNULL,
                                       stderr=open(self.root / "libri.log", "w"))
        until(self.up, "libri up")

    def up(self):
        try:
            urllib.request.urlopen(f"{self.base}/api/host", timeout=2)
            return True
        except OSError:
            return False

    def shown(self, text):
        """Text fit to print: the mark never reaches a log or a transcript."""
        return str(text).replace(self.mark, "<mark>")

    def strays(self):
        """Places outside the scratch root that hold the mark."""
        found = []
        if self.home_was and Path(self.home_was).is_dir():
            for top, dirs, files in os.walk(self.home_was):
                dirs[:] = [d for d in dirs if d not in SKIP and Path(top, d).resolve() != self.root]
                for name in files:
                    path = Path(top, name)
                    try:
                        st = path.lstat()
                        if (stat.S_ISREG(st.st_mode) and st.st_mtime >= self.began - 1 and st.st_size <= BIGGEST
                                and self.mark.encode() in path.read_bytes()):
                            found.append(str(path))
                    except OSError:
                        continue
        if self.hippo_was and Path(self.hippo_was, "hippo.sock").exists():
            env = {k: v for k, v in os.environ.items() if k != "HIPPO_DIR"} | {"HIPPO_DIR": self.hippo_was}
            # The caller's memory takes in new sessions as they are written.
            time.sleep(5)
            got = subprocess.run([self.hippo_bin, "search", self.mark], env=env, capture_output=True, text=True)
            if self.mark in got.stdout:
                found.append(f"the memory at {self.hippo_was}")
        return found

    def close(self, keep=False):
        for proc in (self.daemon, self.hippo):
            proc.terminate()
            proc.wait()
        if not keep:
            shutil.rmtree(self.root, ignore_errors=True)
