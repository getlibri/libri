#!/usr/bin/env python3
"""A `claude` that misbehaves on demand. The word in the prompt's last line
picks the fault; without one it answers at once. Every start and every line
read goes to read.jsonl, so a scenario can tell what reached the harness."""

import json, os, re, select, signal, subprocess, sys, time
from pathlib import Path

args = sys.argv
root = Path(os.environ["STAND_IN_ROOT"])
if (root / "login-gone").exists():
    if args[1:3] == ["auth", "status"]:
        print('{"loggedIn": false}')
        sys.exit(1)
    print(json.dumps({"type": "result", "subtype": "success", "is_error": True,
                      "result": "Not logged in · Please run /login", "session_id": "s",
                      "usage": {}, "modelUsage": {}}), flush=True)
    sys.exit(1)
if args[1:3] == ["auth", "status"]:
    sys.exit(0)
plain = args[args.index("--tools") + 1] == "" if "--tools" in args else False


def say(ev):
    print(json.dumps(ev), flush=True)


def delta(text):
    say({"type": "stream_event", "event": {"type": "content_block_delta",
         "delta": {"type": "text_delta", "text": text}}})


def answer(text):
    delta(text)
    say({"type": "result", "subtype": "success", "is_error": False, "result": text,
         "session_id": "s", "usage": {}, "modelUsage": {}})


def words(line):
    content = json.loads(line)["message"]["content"]
    return content if isinstance(content, str) else "".join(c.get("text", "") for c in content)


def note(text):
    with (root / "read.jsonl").open("a") as f:
        f.write(json.dumps({"at": time.time(), "pid": os.getpid(), "said": text}) + "\n")


if (root / "hold-start").exists() and not plain:
    (root / "started").touch()
    while (root / "hold-start").exists():
        time.sleep(0.05)
said = words(sys.stdin.readline())
if plain:
    slow = root / "slow-summary"
    if slow.exists():
        time.sleep(float(slow.read_text() or 0))
    kind = re.search(r"<input>\n(\w+):", said)
    answer(f"{kind.group(1) if kind else 'talk'}: stand-in summary")
    sys.exit()
last = said.strip().removesuffix("</orders>").strip().split("\n")[-1]
if last.startswith("</"):
    # An alert or a household message: its words sit inside the tags.
    last = said.strip().split("\n")[-2]
note(last[-300:])
(root / "harness.pid").write_text(str(os.getpid()))


def wait_forever():
    while True:
        time.sleep(3600)


if "F-HANG" in last:
    wait_forever()
if "F-DEAF" in last:
    # Ignores SIGTERM, as a child stuck in a kernel wait would.
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    wait_forever()
if "F-PART" in last:
    delta("half an answer ")
    wait_forever()
if "F-ASK" in last:
    say({"type": "control_request", "request_id": "r1", "request": {
        "subtype": "can_use_tool", "tool_name": "AskUserQuestion",
        "input": {"questions": [{"question": "Which?", "header": "Which",
                                 "options": [{"label": "A", "description": "a"}, {"label": "B", "description": "b"}]}]}}})
    line = sys.stdin.readline()
    note("ask answered: " + line[:200])
    answer("after the question")
    sys.exit()
# Only a worker's own task, `user: <task>`: a later main turn may mention the
# word in a report or in memory.
if re.match(r"user: .*W-HANG", last):
    wait_forever()
if "F-TOOLHANG" in last or "F-TOOLBUSY" in last:
    say({"type": "assistant", "message": {"content": [
        {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "wait"}}]}})
    if "F-TOOLBUSY" in last:
        # A long build: silent on the stream, busy in a child.
        subprocess.Popen([sys.executable, "-c", "while True: pass"])
    wait_forever()
if "F-MCPDOWN" in last:
    say({"type": "system", "subtype": "init", "session_id": "s",
         "mcp_servers": [{"name": "libri", "status": "failed"}], "tools": []})
    answer("I could not reach my tools, but here is an answer.")
    sys.exit()
if "F-FAST" in last:
    # Output as fast as the pipe takes it, for slow-client tests.
    for n in range(400):
        delta(f"{n} " + "y" * 16000)
    answer("fast done")
    sys.exit()
if "F-CRASH" in last:
    print("stand-in: fatal", file=sys.stderr)
    sys.exit(3)
if "F-SEGV" in last:
    os.kill(os.getpid(), signal.SIGSEGV)
if "F-GARBAGE" in last:
    sys.stdout.buffer.write(b"\xff\xfe not json\n{\"type\": \n\x00\x01\x02\n[1,2]\n42\n")
    sys.stdout.flush()
    sys.exit(0)
if "F-NOANSWER" in last:
    sys.exit(0)
if "F-LOOP" in last:
    # Bounded by the scenario's Stop; a tenth of a second keeps the rate honest.
    n = 0
    while True:
        delta(f"again {n} ")
        n += 1
        time.sleep(0.1)
if "F-TOOLLOOP" in last:
    n = 0
    while True:
        say({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": f"t{n}", "name": "Bash", "input": {"command": "true"}}]}})
        say({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": f"t{n}", "content": "ok"}]}})
        n += 1
        time.sleep(0.1)
if "F-HOLDOUT" in last:
    # A background child keeps stdout open past the harness's exit, as a
    # server started from a tool call does.
    subprocess.Popen(["sleep", "90"], start_new_session=True)
    answer("answered before leaving")
    os._exit(0)
if "F-HUGE" in last:
    delta("x" * (64 * 1024 * 1024))
    answer("after a huge line")
    sys.exit()
if "F-DENIED" in last:
    say({"type": "assistant", "message": {"content": [
        {"type": "tool_use", "id": "t1", "name": "Write",
         "input": {"file_path": "/var/tmp/probe", "content": "x"}}]}})
    say({"type": "user", "message": {"content": [
        {"type": "tool_result", "tool_use_id": "t1", "is_error": True,
         "content": "Permission to use Write has been denied."}]}})
    answer("Done. I wrote /var/tmp/probe.")
    sys.exit()
m = re.search(r"F-SLOW(\d+)", last)
if m:
    delta("starting ")
    time.sleep(int(m.group(1)))
if "F-BGJOB" in last:
    say({"type": "system", "subtype": "background_tasks_changed", "tasks": [{"description": "a long build"}]})
    answer("job started")
    while not (root / "release").exists():
        if select.select([sys.stdin], [], [], 0.1)[0]:
            line = sys.stdin.readline()
            if not line:
                break
            if line.strip():
                text = words(line)
                note(text)
                answer("steered: " + text.strip().split("\n")[-1][:60])
    say({"type": "system", "subtype": "background_tasks_changed", "tasks": []})
    answer("job finished")
    sys.stdin.read()
    sys.exit()
# A turn run again after a restart comes with a note in front of the words.
answer("answered: " + re.sub(r"^\(a restart .*?\) ", "", last.split(": ", 1)[-1])[:80])
# As the CLI does: words written while it worked get an answer of their own.
for line in sys.stdin:
    if line.strip():
        text = words(line)
        note(text)
        answer("steered: " + text.strip().split("\n")[-1][:60])
