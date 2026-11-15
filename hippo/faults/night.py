"""Test instances of hippo. Each lives under NIGHT/run/<name> with its own HOME,
XDG dirs and HIPPO_DIR, under an address-space limit; nothing here may name the
live store, and every signal is checked against the pid's own environment."""
import json, os, shutil, signal, socket, subprocess, time
from pathlib import Path

NIGHT = Path("/tmp/libri-night/hippo")
HIPPO = os.environ.get("HIPPO", str(NIGHT / "build/debug/hippo"))
# A synthetic store to start from (README); none starts empty.
SEED = os.environ.get("HIPPO_FAULT_SEED")
LIMIT_KB = 6 * 1024 * 1024


class Instance:
    def __init__(self, name, seed=SEED, backend="libri", limit=LIMIT_KB, env=None):
        self.name, self.backend, self.limit, self.more = name, backend, limit, env or {}
        self.root = NIGHT / "run" / name
        self.close(quiet=True)
        if self.root.exists():
            subprocess.run(["chmod", "-R", "u+rwx", str(self.root)])
            shutil.rmtree(self.root)
        for d in ("home", "cache", "config", "data", "runtime"):
            (self.root / d).mkdir(parents=True)
        self.store = self.root / "store"
        if seed:
            subprocess.run(["cp", "-a", "--reflink=auto", str(seed), str(self.store)], check=True)
        else:
            self.store.mkdir()
        self.mode = self.root / "mode"
        self.mode.write_text("ok")
        self.log = self.root / "hippo.log"
        self.proc = self.stub_proc = None

    @property
    def sock(self):
        return self.store / "hippo.sock"

    def env(self):
        r = self.root
        assert str(r).startswith("/tmp/libri-night/hippo/run/")
        return {
            "PATH": os.environ["PATH"], "HOME": f"{r}/home", "XDG_CACHE_HOME": f"{r}/cache",
            "XDG_CONFIG_HOME": f"{r}/config", "XDG_DATA_HOME": f"{r}/data",
            "XDG_RUNTIME_DIR": f"{r}/runtime", "HIPPO_DIR": str(self.store),
            "HIPPO_AGENT": "Iris", "HIPPO_BACKEND": self.backend, **self.more,
        }

    def spawn(self, mode="code"):
        lim = f"ulimit -v {self.limit}; ulimit -c 0; exec {HIPPO} serve {mode}"
        return subprocess.Popen(
            ["bash", "-c", lim], env=self.env(), stdout=subprocess.DEVNULL,
            stderr=open(self.log, "a"), start_new_session=True,
        )

    def start(self, mode="code", within=60):
        """Returns seconds to answer, or None if it died or never answered."""
        assert not (self.store / "hippo.sock").is_symlink()
        self.proc = self.spawn(mode)
        t0 = time.monotonic()
        while time.monotonic() - t0 < within:
            if self.proc.poll() is not None:
                return None
            if self.sock.exists():
                try:
                    self.ask("health", wait=2)
                    return time.monotonic() - t0
                except OSError:
                    pass
            time.sleep(0.02)
        return None

    def stub(self):
        self.stub_proc = subprocess.Popen(
            ["python3", str(NIGHT / "repro/compactor_stub.py"), str(self.store), str(self.mode)],
            stdout=subprocess.DEVNULL, stderr=open(self.root / "stub.log", "a"),
        )
        for _ in range(100):
            if (self.store / "compactor.sock").exists():
                return
            time.sleep(0.02)

    def set_mode(self, m):
        self.mode.write_text(m)

    def calls(self):
        p = Path(str(self.mode) + ".calls")
        return int(p.read_text() or 0) if p.exists() else 0

    def mine(self, pid):
        try:
            raw = Path(f"/proc/{pid}/environ").read_bytes()
        except OSError:
            return False
        return f"HIPPO_DIR={self.store}".encode() in raw.split(b"\0")

    def signal(self, sig, proc=None):
        p = proc or self.proc
        if p and p.poll() is None and self.mine(p.pid):
            os.kill(p.pid, sig)
            return True
        return False

    def kill(self, proc=None):
        p = proc or self.proc
        self.signal(signal.SIGCONT, p)
        self.signal(signal.SIGKILL, p)
        if p:
            try:
                p.wait(5)
            except subprocess.TimeoutExpired:
                pass

    def close(self, quiet=False):
        for attr in ("proc", "stub_proc"):
            p = getattr(self, attr, None)
            if not p:
                continue
            if attr == "proc":
                self.kill(p)
            elif p.poll() is None:
                p.kill()
                p.wait()

    def ask(self, cmd, *args, wait=30):
        """One request as Libri sends it; OSError on any transport failure."""
        s = socket.socket(socket.AF_UNIX)
        s.settimeout(wait)
        s.connect(str(self.sock))
        s.sendall((json.dumps({"cmd": cmd, "args": list(args)}) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            got = s.recv(1 << 20)
            if not got:
                break
            buf += got
        s.close()
        if not buf:
            raise ConnectionResetError("empty reply")
        r = json.loads(buf)
        return r["ok"], r["out"]

    def post(self, text, kind="user", title="Iris", wait=30):
        return self.ask("post", title, kind, text, wait=wait)

    def tail(self, n=5):
        ok, out = self.ask("tail", str(n))
        return out

    def logs(self, n=3):
        return [l[:200] for l in self.log.read_text(errors="replace").splitlines()[-n:]]

    def last_day(self, sub="main"):
        return sorted((self.store / sub).glob("*.jsonl"))[-1]


def probe(i, wait=45):
    """What a turn would meet: view, save, tail; each as seen, with seconds."""
    out = {}
    for name, f in (
        ("view", lambda: i.ask("view", "whole", wait=wait)),
        ("post", lambda: i.post("probe words", wait=wait)),
        ("tail", lambda: i.ask("tail", "2", wait=wait)),
    ):
        t = time.monotonic()
        try:
            ok, o = f()
            out[name] = (("ok" if ok else "ERR " + o[:110]), round(time.monotonic() - t, 2), len(o))
        except Exception as e:
            out[name] = ("TRANSPORT " + repr(e)[:90], round(time.monotonic() - t, 2), 0)
    return out
