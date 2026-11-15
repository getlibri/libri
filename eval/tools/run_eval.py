"""Runs the tool-description cases on stock Claude Code and Codex, as Libri
drives them, against a stub of `libri tools` that serves the real tool list.
It spends subscription use, so it is not a flake check: run it by hand when
the descriptions change.

    python3 -I eval/tools/run_eval.py TOOLS_DIR OUT_DIR [claude|sol ...] [-k CASE ...]

TOOLS_DIR holds main.json and worker.json, from dump_tools.py with scopes
person and agent."""
import json, os, re, subprocess, sys, tempfile, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from cases import CASES, NOW, ROSTER  # noqa: E402

MAIN_RS = HERE.parent.parent / "libri" / "src" / "main.rs"
MODELS = {
    "claude": ["claude", "claude-opus-5-5"],
    "sol": ["codex", "gpt-6.1-sol"],
}


def constant(src, name):
    """A Rust &str constant of main.rs, as the program sees it."""
    m = re.search(r"const " + name + r': &str = "(.*?)(?<!\\)";', src, re.S)
    body = re.sub(r"\\\n\s*", "", m.group(1))
    return body.replace('\\"', '"').replace("\\n", "\n")


def prompts():
    src = MAIN_RS.read_text()
    fill = {
        "name": "Iris", "person": "Ada", "QUIET": "(nothing new)", "hops": "6",
        "agent": "Reed", "boss": "Iris", "lineage": "", "read": " and read its chat with agent_chat",
    }

    def fmt(text, **more):
        values = {**fill, **more}
        return re.sub(r"\{(\w+)\}", lambda m: values.get(m.group(1), m.group(0)), text)

    main = fmt(constant(src, "PROMPT") + constant(src, "TOOLS") + "\n\n" + constant(src, "AGENTS"), memory="<chat>")
    sub = fmt(
        constant(src, "SUB"),
        opening="It opens with your task, then goes on with ",
        main="memory_search and memory_zoom reach Iris's memory, for a fact from before your start. ",
        starts=fmt(constant(src, "STARTS_SUBS")),
    ) + fmt(constant(src, "TOOLS"), memory="<work>")
    return main, sub


def opening(case):
    if case["scope"] == "main":
        return (f"<chat>\n{case['chat']}\n</chat>\nNow: {NOW}\n\n<workers>\n{case.get('roster', ROSTER)}\n</workers>\n\n"
                f"user: {case['message']}")
    return (f"<work>\n{case['work']}\n</work>\nNow: {NOW}\n\n<workers>\n{ROSTER}\n</workers>\n\n"
            f"<orders>\nuser: {case['orders']}\n</orders>")


def run(case, model, tools_dir, out):
    main, sub = prompts()
    system = main if case["scope"] == "main" else sub
    tools = tools_dir / ("main.json" if case["scope"] == "main" else "worker.json")
    log = out / f"{case['id']}.{model}.calls.jsonl"
    log.unlink(missing_ok=True)
    env = {
        "STUB_TOOLS": str(tools), "STUB_LOG": str(log),
        "STUB_RESULTS": json.dumps(case.get("results", {})),
    }
    cli, model_id = MODELS[model]
    server = [sys.executable, "-I", str(HERE / "stub_server.py")]
    # The main chat also has Fastmail, which a watch's look may read.
    mail = {**env, "STUB_NAME": "fastmail", "STUB_TOOLS": str(HERE / "fastmail.json")}
    blank = tempfile.mkdtemp()
    if cli == "claude":
        config = {"mcpServers": {"libri": {"command": server[0], "args": server[1:], "env": env},
                                 "fastmail": {"command": server[0], "args": server[1:], "env": mail}}}
        cmd = ["claude", "-p", "--model", model_id, "--output-format", "json",
               "--no-session-persistence", "--setting-sources", "", "--strict-mcp-config",
               "--mcp-config", json.dumps(config), "--allowedTools", "mcp__libri,mcp__fastmail",
               "--tools", "", "--append-system-prompt", system, "--max-turns", "6"]
    else:
        toml = lambda e: "{" + ",".join(f"{k}={json.dumps(v)}" for k, v in e.items()) + "}"
        cmd = ["codex", "exec", "--skip-git-repo-check", "--ephemeral", "--json", "-m", model_id,
               "-s", "read-only", "-C", blank,
               "-c", 'approval_policy="never"',
               "-c", f"developer_instructions={json.dumps(system)}",
               "-c", f"mcp_servers.libri.command={json.dumps(server[0])}",
               "-c", f"mcp_servers.libri.args={json.dumps(server[1:])}",
               "-c", f"mcp_servers.libri.env={toml(env)}",
               "-c", "mcp_servers.libri.default_tools_approval_mode=\"approve\"",
               "-c", f"mcp_servers.fastmail.command={json.dumps(server[0])}",
               "-c", f"mcp_servers.fastmail.args={json.dumps(server[1:])}",
               "-c", f"mcp_servers.fastmail.env={toml(mail)}",
               "-c", "mcp_servers.fastmail.default_tools_approval_mode=\"approve\""]
    began = time.time()
    p = subprocess.run(cmd, input=opening(case), capture_output=True, text=True, timeout=600, cwd=blank)
    (out / f"{case['id']}.{model}.raw").write_text(p.stdout + "\n--stderr--\n" + p.stderr)
    reply, cost, tokens = "", None, None
    if cli == "claude":
        try:
            r = json.loads(p.stdout)
            reply, cost = r.get("result", ""), r.get("total_cost_usd")
            u = r.get("usage", {})
            tokens = sum(u.get(k, 0) for k in ("input_tokens", "cache_read_input_tokens",
                                                "cache_creation_input_tokens", "output_tokens"))
        except json.JSONDecodeError:
            reply = p.stdout
    else:
        for line in p.stdout.splitlines():
            try:
                ev = json.loads(line)
            except json.JSONDecodeError:
                continue
            item = ev.get("item", {})
            if ev.get("type") == "item.completed" and item.get("type") == "agent_message":
                reply = item.get("text", "")
            if ev.get("type") == "turn.completed":
                u = ev.get("usage", {})
                tokens = u.get("input_tokens", 0) + u.get("output_tokens", 0)
    calls = [json.loads(l) for l in log.read_text().splitlines()] if log.exists() else []
    try:
        passed = bool(case["check"](calls, reply))
    except Exception as e:  # a malformed call fails its case
        passed, reply = False, f"{reply}\n[check raised {e!r}]"
    return dict(case=case["id"], model=model, passed=passed, calls=calls, reply=reply,
                cost=cost, tokens=tokens, seconds=round(time.time() - began), rc=p.returncode)


def main():
    args = sys.argv[1:]
    only = []
    if "-k" in args:
        i = args.index("-k")
        only, args = args[i + 1:], args[:i]
    tools_dir, out = Path(args[0]), Path(args[1])
    models = args[2:] or list(MODELS)
    out.mkdir(parents=True, exist_ok=True)
    rows = []
    for case in CASES:
        if only and case["id"] not in only:
            continue
        for model in models:
            r = run(case, model, tools_dir, out)
            rows.append(r)
            print(("PASS" if r["passed"] else "FAIL"), r["case"], r["model"],
                  [c["name"] for c in r["calls"]], repr(r["reply"][:100]), flush=True)
    (out / "results.json").write_text(json.dumps(rows, indent=1))
    for model in models:
        mine = [r for r in rows if r["model"] == model]
        print(f"{model}: {sum(r['passed'] for r in mine)}/{len(mine)}")


if __name__ == "__main__":
    main()
