"""A throwaway Libri and hippo with stand-in harness, reusing nix/check_memory.py's
stand-in Claude. Own HOME, HIPPO_DIR, state; port from 9180-9199."""
import json, os, re, shutil, signal, socket, subprocess, sys, time, urllib.request, urllib.error
from pathlib import Path

NIGHT = Path("/tmp/libri-night/hippo")
PKG = Path(os.environ.get("PKG", NIGHT / "pkg"))
WT = Path(__file__).resolve().parents[2]
src = (WT / "nix/check_memory.py").read_text()
CLAUDE = re.search(r'CLAUDE = r"""(.*?)"""', src, re.S).group(1)
HISTORY_BAD = 502


class Rig:
    def __init__(self, name, port, seed=None):
        assert 9180 <= port <= 9199
        self.port = port
        self.root = NIGHT / "rig" / name
        shutil.rmtree(self.root, ignore_errors=True)
        (self.root / "home").mkdir(parents=True)
        (self.root / "state").mkdir()
        self.dir = self.root / "hippo"
        if seed:
            subprocess.run(["cp", "-a", "--reflink=auto", str(seed), str(self.dir)], check=True)
        else:
            self.dir.mkdir()
        c = self.root / "claude"
        c.write_text(f"#!{sys.executable}\n{CLAUDE}")
        c.chmod(0o755)
        self.claude, self.calls = c, self.root / "calls.jsonl"
        self.env = {
            "HOME": str(self.root / "home"), "PATH": os.environ["PATH"],
            "XDG_CACHE_HOME": str(self.root / "c"), "XDG_CONFIG_HOME": str(self.root / "cf"),
            "XDG_DATA_HOME": str(self.root / "d"), "XDG_RUNTIME_DIR": str(self.root / "r"),
            "HIPPO_DIR": str(self.dir), "HIPPO_ARCHIVE": str(self.root / "archive"),
            "HIPPO_BACKEND": "libri", "STAND_IN_LOG": str(self.calls),
        }
        self.h = self.d = None

    def hippo(self):
        assert str(self.dir).startswith(str(NIGHT))
        self.h = subprocess.Popen([PKG / "bin/hippo", "serve", "code"], env=self.env,
                                  stdout=subprocess.DEVNULL, stderr=open(self.root / "hippo.log", "a"),
                                  start_new_session=True)
        for _ in range(600):
            try:
                self.hs("health")
                if self.h.poll() is None:
                    return True
            except Exception:
                pass
            time.sleep(0.05)
        return False

    def daemon(self, extra=None, lone=False):
        env = self.env | {
            "STATE_DIRECTORY": str(self.root / "state"), "LIBRI_NAME": "night-label",
            "LIBRI_PERSON": "Ada", "LIBRI_HIPPO_SHARED": "Iris", "LIBRI_BACKEND": "claude",
            "LIBRI_CLAUDE": str(self.claude), "LIBRI_MODEL": "stand-in",
            "LIBRI_SITE": f"127.0.0.1:{self.port}", "LIBRI_SITE_URL": f"http://127.0.0.1:{self.port}",
            "LIBRI_SITE_ROOT": str(PKG / "site"), "LIBRI_OPEN": "1", "LIBRI_USERS": "ada",
        } | (extra or {})
        if lone:
            for k in ("HIPPO_BACKEND", "LIBRI_HIPPO_SHARED"):
                env.pop(k, None)
        self.d = subprocess.Popen([PKG / "bin/libri"], env=env, stdout=subprocess.DEVNULL,
                                  stderr=open(self.root / "libri.log", "a"), start_new_session=True)
        for _ in range(400):
            if self.get("/api/host") is not None:
                return True
            time.sleep(0.05)
        return False

    def up(self, **kw):
        ok = self.daemon(**kw)
        if ok:
            self.watch()
        return ok

    def mine(self, p):
        try:
            raw = Path(f"/proc/{p.pid}/environ").read_bytes()
        except OSError:
            return False
        return f"HIPPO_DIR={self.dir}".encode() in raw.split(b"\0")

    def sig(self, s, p=None):
        p = p or self.h
        if p and p.poll() is None and self.mine(p):
            os.kill(p.pid, s)
            return True
        return False

    def kill_hippo(self):
        self.sig(signal.SIGCONT)
        self.sig(signal.SIGKILL)
        if self.h:
            self.h.wait(5)

    def close(self):
        for p in (self.d, self.h):
            if p:
                self.sig(signal.SIGCONT, p)
                self.sig(signal.SIGKILL, p)
                try:
                    p.wait(5)
                except subprocess.TimeoutExpired:
                    pass

    def req(self, path, body=None, wait=20):
        data = None if body is None else json.dumps(body).encode()
        r = urllib.request.Request(f"http://127.0.0.1:{self.port}{path}", data=data,
                                   headers={"content-type": "application/json",
                                            "origin": f"http://127.0.0.1:{self.port}"})
        return urllib.request.urlopen(r, timeout=wait)

    def get(self, path, wait=20):
        try:
            with self.req(path, wait=wait) as r:
                return r.read()
        except (OSError, urllib.error.URLError):
            return None

    def status(self, path, wait=20):
        t = time.monotonic()
        try:
            with self.req(path, wait=wait) as r:
                body = r.read()
                return r.status, len(body), round(time.monotonic() - t, 2)
        except urllib.error.HTTPError as e:
            return e.code, 0, round(time.monotonic() - t, 2)
        except Exception as e:
            return repr(e)[:60], 0, round(time.monotonic() - t, 2)

    def watch(self):
        """Collects /api/events as the page reads it."""
        import threading
        self.ev = []
        def run():
            try:
                with self.req("/api/events", wait=3600) as r:
                    for l in r:
                        l = l.decode(errors="replace").rstrip()
                        if l and not l.startswith(":"):
                            self.ev.append(l)
            except Exception as e:
                self.ev.append("stream ended " + repr(e)[:80])
        threading.Thread(target=run, daemon=True).start()
        time.sleep(0.3)

    def send(self, text, wait=30):
        """One turn: posts it, waits for the turn's end on the event stream or
        `wait`. Returns (seconds, http status, what the stream said)."""
        t = time.monotonic()
        mark = len(self.ev)
        body = {"messages": [{"id": f"m{time.monotonic_ns()}", "role": "user", "content": text}]}
        try:
            with self.req("/api/run", body, wait=wait) as r:
                st = r.status
        except urllib.error.HTTPError as e:
            st = e.code
        except Exception as e:
            st = repr(e)[:80]
        while time.monotonic() - t < wait:
            if any(END in l for l in self.ev[mark:] for END in ("RUN_FINISHED", "RUN_ERROR", "TEXT_MESSAGE_END")):
                time.sleep(0.2)
                break
            time.sleep(0.05)
        return round(time.monotonic() - t, 2), st, " | ".join(l[:160] for l in self.ev[mark:][-6:])

    def hs(self, *args, wait=8):
        return subprocess.run([PKG / "bin/hippo", *args], env=self.env, capture_output=True,
                              text=True, timeout=wait).stdout

    def turns(self):
        return [json.loads(l) for l in self.calls.read_text().splitlines()] if self.calls.exists() else []

    def has(self, text):
        return text in self.hs("search", re.escape(text))
