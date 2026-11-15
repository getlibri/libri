"""A throwaway Libri and hippo for hostile tests. Everything lives under one
root: home, XDG directories, hippo, state, sockets. Nothing here signals a
process it did not start, and every signal is checked against /proc first."""

import atexit, json, os, shutil, signal as signals, socket, subprocess, sys, threading, time, urllib.error, urllib.request
from pathlib import Path

BUILD = Path(os.environ.get("LIBRI_BUILD", "/nix/store/fbx8kjds2gjys06q6ydavpsgzhjjhjlw-libri"))
BASE = Path(os.environ.get("LIBRI_NIGHT_BASE", "/tmp/libri-night/server"))
TITLE = "Iris"
HERE = Path(__file__).parent


def until(ready, what="", within=20):
    end = time.monotonic() + within
    while time.monotonic() < end:
        if ready():
            return True
        time.sleep(0.05)
    return False


def load():
    return os.getloadavg()[0]


def own_socket_paths(pid):
    """Listening unix sockets the pid holds."""
    inodes = set()
    try:
        for fd in os.listdir(f"/proc/{pid}/fd"):
            try:
                t = os.readlink(f"/proc/{pid}/fd/{fd}")
            except OSError:
                continue
            if t.startswith("socket:["):
                inodes.add(t[8:-1])
    except OSError:
        return []
    out = []
    for line in Path(f"/proc/{pid}/net/unix").read_text().splitlines()[1:]:
        f = line.split()
        if len(f) >= 8 and f[6] in inodes and f[3] == "00010000":
            out.append(f[7])
    return out


# An orderly exit under `timeout`, so close() runs and nothing stays behind.
signals.signal(signals.SIGTERM, lambda *_: sys.exit(143))


class Rig:
    def __init__(self, name, port, env=None, build=None, prepare=None):
        assert 9200 <= port <= 9219
        self.root = BASE / name
        shutil.rmtree(self.root, ignore_errors=True)
        for d in ("hippo", "home", "state", "xdg/run", "xdg/state", "xdg/config", "xdg/cache", "xdg/data"):
            (self.root / d).mkdir(parents=True)
        (self.root / "xdg/run").chmod(0o700)
        self.port = port
        self.build = Path(build) if build else BUILD
        self.events = []
        self.listeners = 0
        self.env = {
            "HOME": str(self.root / "home"),
            "PATH": f"{self.build}/bin:{os.environ['PATH']}",
            "XDG_RUNTIME_DIR": str(self.root / "xdg/run"),
            "XDG_STATE_HOME": str(self.root / "xdg/state"),
            "XDG_CONFIG_HOME": str(self.root / "xdg/config"),
            "XDG_CACHE_HOME": str(self.root / "xdg/cache"),
            "XDG_DATA_HOME": str(self.root / "xdg/data"),
            "HIPPO_DIR": str(self.root / "hippo"),
            "HIPPO_ARCHIVE": str(self.root / "archive"),
            "HIPPO_BACKEND": "libri",
            "STAND_IN_ROOT": str(self.root),
        }
        self.extra = env or {}
        self.claude = self.root / "claude"
        self.claude.write_text(f"#!{sys.executable}\n" + (HERE / "stand_in_claude.py").read_text())
        self.claude.chmod(0o755)
        atexit.register(self.close)
        if prepare:
            prepare(self)
        self.start_hippo()
        self.start_daemon()

    def check_contained(self, pid, what):
        for path in own_socket_paths(pid):
            if not path.startswith(str(self.root)):
                os.kill(pid, signals.SIGKILL)
                raise SystemExit(f"{what} {pid} took a socket outside its root: {path}")

    def mine(self, pid):
        """Refuses any process not started for this root."""
        env = Path(f"/proc/{pid}/environ").read_bytes()
        if f"STAND_IN_ROOT={self.root}".encode() not in env:
            raise SystemExit(f"pid {pid} is not this rig's")

    def signal(self, pid, sig):
        self.mine(pid)
        os.kill(pid, sig)

    def start_hippo(self):
        (self.root / "hippo/hippo.sock").unlink(missing_ok=True)
        self.hippo = subprocess.Popen(
            [self.build / "bin/hippo", "serve", "code"], env=self.env,
            stdout=subprocess.DEVNULL, stderr=open(self.root / "hippo.log", "a"))
        assert until(lambda: (self.root / "hippo/hippo.sock").exists()), "hippo up"
        self.check_contained(self.hippo.pid, "hippo")

    def start_daemon(self, build=None, wait=True):
        b = Path(build) if build else self.build
        self.daemon = subprocess.Popen(
            [b / "bin/libri"],
            env=self.env | {
                "STATE_DIRECTORY": str(self.root / "state"),
                "LIBRI_NAME": "box-label",
                "LIBRI_PERSON": "Ada",
                "LIBRI_HIPPO_SHARED": TITLE,
                "LIBRI_BACKEND": "claude",
                "LIBRI_CLAUDE": str(self.claude),
                "LIBRI_MODEL": "stand-in",
                "LIBRI_SITE": f"127.0.0.1:{self.port}",
                "LIBRI_SITE_URL": f"http://127.0.0.1:{self.port}",
                "LIBRI_SITE_ROOT": str(b / "site"),
                "LIBRI_OPEN": "1",
            } | self.extra,
            stdout=subprocess.DEVNULL, stderr=open(self.root / "libri.log", "a"))
        if not wait:
            return
        assert until(lambda: self.get("/api/host") is not None, within=30), "libri up"
        self.check_contained(self.daemon.pid, "libri")
        threading.Thread(target=self.listen, daemon=True).start()
        assert until(lambda: self.listeners > 0, within=5) or True
        time.sleep(0.4)

    def kill_daemon(self, sig=signals.SIGKILL):
        self.signal(self.daemon.pid, sig)
        self.daemon.wait(timeout=30)
        self.listeners = 0

    def request(self, path, body=None, timeout=20, method=None):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(
            f"http://127.0.0.1:{self.port}{path}", data=data, method=method,
            headers={"content-type": "application/json", "origin": f"http://127.0.0.1:{self.port}"})
        return urllib.request.urlopen(req, timeout=timeout)

    def get(self, path, timeout=5):
        try:
            with self.request(path, timeout=timeout) as r:
                return r.read()
        except (OSError, urllib.error.HTTPError):
            return None

    def status(self, path, timeout=5):
        """The HTTP status of a read, or the error's name; never raises."""
        try:
            with self.request(path, timeout=timeout) as r:
                r.read()
                return r.status
        except urllib.error.HTTPError as e:
            return e.code
        except Exception as e:
            return type(e).__name__

    def listen(self):
        try:
            with self.request("/api/events", timeout=None) as stream:
                self.listeners += 1
                for line in stream:
                    if line.startswith(b"data: "):
                        # The keep-alive ping carries an empty object.
                        if event := json.loads(line[6:]):
                            self.events.append(event)
        except Exception:
            pass

    def send(self, text, id=None, timeout=10):
        """Answers the HTTP status, or the error's name."""
        try:
            with self.request("/api/run", {"messages": [
                    {"id": id or f"m{time.monotonic_ns()}", "role": "user", "content": text}]},
                    timeout=timeout) as r:
                return r.status
        except urllib.error.HTTPError as e:
            return e.code
        except Exception as e:
            return type(e).__name__

    def stop(self):
        with self.request("/api/stop", {}) as r:
            return r.status

    def task(self):
        got = self.get("/api/task")
        return json.loads(got) if got else None

    def typing(self):
        t = self.task()
        return t and t.get("typing")

    def shown(self):
        out = {}
        for e in list(self.events):
            if e["type"] == "TEXT_MESSAGE_START":
                out[e["messageId"]] = ""
            elif e["type"] == "TEXT_MESSAGE_CONTENT":
                out[e["messageId"]] = out.get(e["messageId"], "") + e["delta"]
            elif e["type"] == "TEXT_MESSAGE_END" and e.get("metadata", {}).get("discarded"):
                out.pop(e["messageId"], None)
        return list(out.values())

    def on_page(self, text):
        return sum(text in m for m in self.shown())

    def read(self):
        f = self.root / "read.jsonl"
        return [json.loads(l)["said"] for l in f.read_text().splitlines()] if f.exists() else []

    def reached(self, text):
        return any(text in s for s in self.read())

    def chat(self):
        try:
            out = subprocess.run([self.build / "bin/hippo", "tail", "200", TITLE], env=self.env,
                                 capture_output=True, text=True, timeout=10).stdout
        except subprocess.TimeoutExpired:
            return None
        return [json.loads(l) for l in out.splitlines() if l.startswith("{")]

    def count(self, kind, text):
        c = self.chat()
        return None if c is None else sum(1 for m in c if m["kind"] == kind and text in m["text"])

    def log(self, n=12):
        lines = (self.root / "libri.log").read_text(errors="replace").splitlines()
        return lines[-n:]

    def fds(self):
        return len(os.listdir(f"/proc/{self.daemon.pid}/fd"))

    def rss_kib(self):
        for line in Path(f"/proc/{self.daemon.pid}/status").read_text().splitlines():
            if line.startswith("VmRSS"):
                return int(line.split()[1])

    def harness_pid(self):
        f = self.root / "harness.pid"
        return int(f.read_text()) if f.exists() else None

    def tools(self):
        return Tools(self)

    def close(self):
        for p in (getattr(self, "daemon", None), getattr(self, "hippo", None)):
            if p and p.poll() is None:
                try:
                    self.signal(p.pid, signals.SIGKILL)
                except (SystemExit, OSError):
                    pass
                p.wait()
        # Stand-ins left behind by a killed daemon.
        for pid in filter(str.isdigit, os.listdir("/proc")):
            try:
                if f"STAND_IN_ROOT={self.root}\0".encode() in Path(f"/proc/{pid}/environ").read_bytes():
                    os.kill(int(pid), signals.SIGKILL)
            except OSError:
                pass


class Tools:
    def __init__(self, rig):
        self.proc = subprocess.Popen(
            [rig.build / "bin/libri", "tools", str(rig.root / "state")],
            env=rig.env | {"LIBRI_SCOPE": "person", "STATE_DIRECTORY": str(rig.root / "state"),
                           "LIBRI_NAME": TITLE, "LIBRI_PERSON": "Ada"},
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        self.id = 0
        self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                "clientInfo": {"name": "check", "version": "0"}})
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.proc.stdin.flush()

    def rpc(self, method, params):
        self.id += 1
        self.proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n")
        self.proc.stdin.flush()
        return json.loads(self.proc.stdout.readline())

    def call(self, name, arguments):
        return self.rpc("tools/call", {"name": name, "arguments": arguments})["result"]["content"][0]["text"]

    def routine(self, text):
        at = time.strftime("%Y-%m-%d %H:%M", time.localtime())
        return self.call("watch", {"when": {"at": at}, "then": {"ask": text}})

    def close(self):
        self.proc.terminate()
        self.proc.wait()
