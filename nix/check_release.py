#!/usr/bin/env python3
"""The release tarball on a machine that has never seen Nix.

Boots a stock cloud image of another distribution under qemu, installs the
tarball the way a person would, and takes it through the service, a reboot
with nobody logged in, a device pairing and one turn through a stand-in
Claude. Downloads nothing itself and runs outside the Nix sandbox, as a real
image is too large to be a flake input. Usage:

    check_release.py TARBALL IMAGE.qcow2

with qemu-system-x86_64, qemu-img, genisoimage, ssh and scp on PATH.
"""

import json
import os
import shlex
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

TARBALL, IMAGE = map(Path, sys.argv[1:3])
HERE = Path(__file__).parent
SITE = "http://127.0.0.1:6780"
WORDS = "the words of one full turn"


def ok(what):
    print(f"ok   {what}", flush=True)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Guest:
    def __init__(self, root: Path):
        self.root = root
        self.key = root / "key"
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", self.key], check=True)
        self.port = free_port()
        self.machine = None
        (root / "user-data").write_text(
            "#cloud-config\n"
            "users:\n"
            "  - name: person\n"
            "    shell: /bin/bash\n"
            "    sudo: ALL=(ALL) NOPASSWD:ALL\n"
            f"    ssh_authorized_keys: [{(root / 'key.pub').read_text().strip()}]\n"
        )
        (root / "meta-data").write_text("instance-id: libri-release\nlocal-hostname: stock\n")
        subprocess.run(
            ["genisoimage", "-quiet", "-output", root / "seed.iso", "-volid", "cidata", "-joliet", "-rock",
             root / "user-data", root / "meta-data"],
            check=True,
        )
        subprocess.run(
            ["qemu-img", "create", "-q", "-f", "qcow2", "-F", "qcow2", "-b", IMAGE.resolve(), root / "disk.qcow2", "8G"],
            check=True,
        )

    def boot(self):
        self.machine = subprocess.Popen(
            ["qemu-system-x86_64", "-enable-kvm", "-m", "1024", "-smp", "2", "-nographic",
             "-drive", f"file={self.root / 'disk.qcow2'},if=virtio",
             "-drive", f"file={self.root / 'seed.iso'},format=raw,if=virtio,readonly=on",
             "-nic", f"user,model=virtio,hostfwd=tcp:127.0.0.1:{self.port}-:22"],
            stdin=subprocess.DEVNULL,
            stdout=open(self.root / "console.log", "ab"),
            stderr=subprocess.STDOUT,
        )
        self.wait_ssh()

    def wait_ssh(self):
        end = time.monotonic() + 180
        while subprocess.run(self.ssh("true"), capture_output=True, timeout=30).returncode:
            if time.monotonic() > end:
                raise TimeoutError("the guest's ssh")
            time.sleep(2)

    def ssh(self, command):
        return ["ssh", "-q", "-i", self.key, "-p", str(self.port), "-o", "StrictHostKeyChecking=no",
                "-o", "UserKnownHostsFile=/dev/null", "-o", "ConnectTimeout=5", "person@127.0.0.1", command]

    def run(self, command, check=True):
        """Runs in a login shell, as a person's terminal does."""
        done = subprocess.run(self.ssh(f"bash -lc {shlex.quote(command)}"), capture_output=True, text=True, timeout=120)
        if check and done.returncode:
            raise RuntimeError(f"{command}: {done.returncode}\n{done.stdout}\n{done.stderr}")
        return done.stdout + done.stderr

    def send(self, local: Path, remote: str):
        subprocess.run(
            ["scp", "-q", "-i", self.key, "-P", str(self.port), "-o", "StrictHostKeyChecking=no",
             "-o", "UserKnownHostsFile=/dev/null", local, f"person@127.0.0.1:{remote}"],
            check=True,
        )

    def stop(self):
        if self.machine:
            self.run("sudo poweroff", check=False)
            try:
                self.machine.wait(30)
            except subprocess.TimeoutExpired:
                self.machine.kill()
            self.machine = None

    def until(self, command, what, within=30):
        end = time.monotonic() + within
        while subprocess.run(self.ssh(f"bash -lc {shlex.quote(command)}"), capture_output=True).returncode:
            if time.monotonic() > end:
                raise TimeoutError(what)
            time.sleep(1)


def main():
    with tempfile.TemporaryDirectory() as tmp:
        guest = Guest(Path(tmp))
        try:
            guest.boot()
            guest.until("test ! -e /nix && ! command -v nix && echo clean", "a machine without Nix", 5)
            ok(f"the guest has no Nix: {guest.run('. /etc/os-release; echo $PRETTY_NAME').strip()}")

            guest.send(TARBALL, "libri.tar.gz")
            guest.send(HERE / "install.sh", "install.sh")
            said = guest.run("sh install.sh libri.tar.gz")
            print(said)
            guest.run("test -x ~/.local/bin/libri && test -x ~/.local/bin/hippo")
            ok("the tarball installs in one command")

            print(guest.run("libri service install"))
            guest.until("systemctl --user is-active libri", "the service")
            guest.until("curl -sf -o /dev/null http://127.0.0.1:6780/ && echo up", "the page")
            log = guest.run("journalctl --user -u libri --no-pager")
            assert "no harness found" in log, log
            ok("the service runs and says no harness was found")
            assert '"tools":true' in guest.run(f"sleep 3; curl -s {SITE}/api/host"), "tool server"
            ok("the tool server starts")

            guest.send(HERE / "stand_in_claude.py", "stand_in_claude.py")
            guest.run(
                "printf '#!/bin/sh\\nSTAND_IN_LOG=$HOME/stand-in.jsonl exec python3 $HOME/stand_in_claude.py \"$@\"\\n'"
                " > ~/.local/bin/claude && chmod +x ~/.local/bin/claude"
            )
            guest.run("systemctl --user restart libri")
            guest.until("curl -sf -o /dev/null http://127.0.0.1:6780/ && echo up", "the page again")
            log = guest.run("journalctl --user -u libri --no-pager")
            assert "harnesses found: claude. Not found: codex, opencode, omp." in log, log
            ok("a harness put on the PATH is found")

            guest.run("sudo systemctl reboot", check=False)
            time.sleep(15)
            guest.wait_ssh()
            guest.until("curl -sf -o /dev/null http://127.0.0.1:6780/ && echo up", "the service after reboot", 60)
            ok("the service comes back after a reboot, with nobody logged in first")

            status = guest.run(f"curl -s -o /dev/null -w '%{{http_code}}' {SITE}/api/devices")
            assert status == "401", status
            link = guest.run("libri pair")
            secret = link.split("#pair=")[1].split()[0]
            pairing = json.dumps({"secret": secret, "name": "check"})
            token = json.loads(guest.run(
                f"curl -sf -H 'content-type: application/json' -d {shlex.quote(pairing)} {SITE}/api/pair"
            ))["token"]
            bearer = f"-H 'authorization: Bearer {token}'"
            assert 'id="root"' in guest.run(f"curl -sf {bearer} {SITE}/")
            ok("a device pairs and the page loads")

            run = json.dumps({"messages": [{"id": "m1", "role": "user", "content": WORDS}]})
            guest.run(f"curl -sf {bearer} -H 'content-type: application/json' -d {shlex.quote(run)} {SITE}/api/run -o /dev/null")
            guest.until("hippo tail 200 | grep 'claude answers'", "the answer in the memory")
            held = [json.loads(l) for l in guest.run("hippo tail 200").splitlines() if l.startswith("{")]
            assert sum(WORDS in m["text"] for m in held if m["kind"] == "user") == 1, held
            assert sum("claude answers" in m["text"] for m in held if m["kind"] == "talk") == 1, held
            ok("one turn runs and the memory keeps it once")

            print(guest.run("for n in libri hippo; do grep -H VmRSS /proc/$(pgrep -x $n)/status; done; du -sh ~/.local/share/libri; free -m | sed -n 2p"))
            guest.run("libri service remove")
            ok("the service removes cleanly")
        finally:
            if guest.machine:
                guest.machine.kill()
            os.sync()


main()
