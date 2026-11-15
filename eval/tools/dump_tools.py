"""Serve `libri tools` over stdio and print its tools/list result as JSON.
Usage: python3 dump_tools.py LIBRI_BINARY [SCOPE]"""
import json, os, subprocess, sys, tempfile
binary = sys.argv[1]
scope = sys.argv[2] if len(sys.argv) > 2 else ""
d = tempfile.mkdtemp()
env = dict(os.environ, HIPPO_DIR=d, LIBRI_SCOPE=scope)
p = subprocess.Popen([binary, "tools", d], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env, text=True)
def send(m): p.stdin.write(json.dumps(m) + "\n"); p.stdin.flush()
send({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"dump","version":"0"}}})
p.stdout.readline()
send({"jsonrpc":"2.0","method":"notifications/initialized"})
send({"jsonrpc":"2.0","id":2,"method":"tools/list"})
print(json.dumps(json.loads(p.stdout.readline())["result"]["tools"], indent=1))
p.kill()
