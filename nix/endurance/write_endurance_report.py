"""Writes report.md from what a run left behind. It reads the files, not the
VM, so a run the host stopped early still reports what it saw."""

import json, statistics, sys, time
from pathlib import Path

run = Path(sys.argv[1])


def rows(name):
    f = run / "shared" / name
    return [json.loads(l) for l in f.read_text().splitlines()] if f.exists() else []


def slope_per_hour(points):
    """Least squares, in the value's unit per hour; two points at least."""
    if len(points) < 2 or points[-1][0] == points[0][0]:
        return 0.0
    return statistics.linear_regression(*zip(*points)).slope * 3600


def first(kind):
    return next((e for e in events if e["kind"] == kind), None)


events, samples = rows("events.jsonl"), rows("samples.jsonl")
faults = [e for e in events if e["kind"] == "fault"]
broken = [e for e in faults if e["broke"]]
crossings = [e for e in events if e["kind"] == "crossing"]
formats = first("formats")
timeouts = [e for e in events if e["kind"] == "turn timed out"]
stopped = (run / "stopped").read_text().strip() if (run / "stopped").exists() else None
canary = (run / "canary.hits").exists()
start, end, sealed, ledger = first("start"), first("end"), first("sealed"), first("ledger")
out = []
say = out.append

problems = []
if stopped:
    problems.append(f"the host stopped the run: {stopped}")
if end is None:
    problems.append("the suite did not reach its end")
if canary:
    problems.append("the VM reached the host's canary")
if first("crossing failed"):
    problems.append("a change of build could not be finished")
for e in crossings:
    if e["broke"]:
        problems.append(f"{e['label']}: " + "; ".join(e["broke"]))
if formats and (formats["changed"] or formats["only_previous"] or formats["only_dev"]):
    problems.append("the two builds write different formats")
if broken:
    problems.append(f"{len(broken)} of {len(faults)} faults broke something")
if timeouts:
    problems.append(f"{len(timeouts)} plain turns got no answer")
if ledger and (ledger["missing"] or ledger["doubled"] or ledger["out_of_order"] or ledger["unexplained"]):
    problems.append(f"words: {ledger['missing']} missing, {ledger['doubled']} doubled, {ledger['out_of_order']} out of order, "
                    f"{ledger['unexplained']} unanswered with no cause")
for e in events:
    if e["kind"] == "log" and e["panics"]:
        problems.append(f"{e['name']}: {len(e['panics'])} panics")

say(f"# Endurance run {run.name}\n")
say("Verdict: " + ("nothing broke." if not problems else "; ".join(problems) + ".") + "\n")

say("## What ran\n")
if start:
    plan = start["plan"]
    began = time.strftime("%Y-%m-%d %H:%M", time.localtime(start["at"]))
    say(f"- Commit {plan['rev'][:10]}, build `{start['build']}`, started {began} guest clock.")
    same = start["previous"] == start["build"]
    say(f"- Previous build {plan.get('previous', '?')[:10]}, `{start['previous']}`"
        + (": THE SAME BUILD AS DEV, so upgrade and rollback proved nothing." if same else "."))
    say(f"- Planned {plan['minutes']} minutes of soak, a fault every {plan.get('fault_every_seconds')} s.")
if samples:
    last = samples[-1]
    say(f"- {last['turns']} plain turns over {(last['at'] - samples[0]['at']) / 60:.0f} minutes, {len(faults)} faults, stand-in harness, no model.")
if sealed:
    say(f"- Sealed: canary on the host {'REACHED' if canary or sealed['canary_on_host'] else 'not reached'}, "
        f"host ssh {sealed['host_ssh']}, internet {sealed['internet']}, peer settings {sealed['peer_env']}, "
        f"{sealed['store_paths']} store paths in the guest.")

say("\n## Upgrade and rollback\n")
if crossings:
    say("| Change | Next turn, s | Rows | Worker chats | Open turn found | Cut word in hippo, answered | Typed word in hippo, answered | Broke |\n|---|---|---|---|---|---|---|---|")
    for e in crossings:
        cut, typed = list(e["kept"].items())
        say(f"| {e['label']} | {e['next_turn']} | {e['rows_before']} to {e['rows_after']} | {e['threads_before']} to {e['threads_after']} "
            f"| {e['open_turn_found']} | {cut[1]}, {e['cut_answered']} | {typed[1]}, {e['typed_answered']} | {'; '.join(e['broke']) or 'nothing'} |")
    for e in crossings:
        for line in e["complaints"]:
            say(f"\n{e['label']}, log: `{line}`")
        if e["stuck"]:
            say(f"\n{e['label']}, the turn that got no answer was waiting on: `{e['stuck']}`")
for e in events:
    if e["kind"] == "written":
        say(f"\nWritten on {e['on']}: {e['turns_answered']} of {e['of']} turns, {e['threads']} worker chats listed by then "
            f"({e['task_dirs']} on disk, /api/work {e['work_status']}). Spawn said: `{e['spawned']}`")
if formats:
    if formats["changed"] or formats["only_previous"] or formats["only_dev"]:
        say(f"\nFormats differ. Only previous writes: `{formats['only_previous']}`. Only dev writes: `{formats['only_dev']}`. Changed: `{formats['changed']}`.")
    else:
        say("\nEvery kind of file in state and hippo has the same keys and schema on both builds.")

say("\n## What broke\n")
if not (broken or timeouts):
    say("No fault broke the rules this suite checks.")
for e in broken:
    say(f"- **{e['name']}** ({e['took']} s): " + "; ".join(e["broke"]))
    for line in e.get("log", []):
        say(f"  - `{line}`")
if timeouts:
    say(f"- {len(timeouts)} plain turns timed out; waiting on, first: `{timeouts[0]['stuck']}`")

say("\n## Faults\n")
if faults:
    say("| Fault | Runs | Broke | Slowest next turn, s |\n|---|---|---|---|")
    for name in dict.fromkeys(e["name"] for e in faults):
        group = [e for e in faults if e["name"] == name]
        times = [e["next_turn"] for e in group if e.get("next_turn") is not None]
        say(f"| {name} | {len(group)} | {sum(1 for e in group if e['broke'])} | {max(times) if times else 'none'} |")
    cut = [e for e in faults if "cut_turn_answered" in e]
    if cut:
        say(f"\nA turn cut by the fault still got its answer in {sum(1 for e in cut if e['cut_turn_answered'])} of {len(cut)} runs.")

say("\n## Words\n")
if ledger:
    say(f"{ledger['sent']} sent and accepted, {ledger['refused']} refused, {ledger['rows']} rows in hippo: "
        f"{ledger['missing']} missing, {ledger['doubled']} doubled, {ledger['out_of_order']} out of order, "
        f"{ledger['unanswered']} with no answer in hippo. {ledger['resumed']} words were run again after a restart, each a second row carrying Libri's note.")
    for key in ("missing_words", "doubled_words", "saved_twice", "unanswered_words", "unexplained_words"):
        if ledger[key]:
            say(f"\n{key}: `{ledger[key]}`")
    for row in ledger["doubled_rows"]:
        say(f"\nDoubled row: `{row}`")
else:
    say("The ledger was never checked.")

say("\n## Trends\n")
if len(samples) >= 2:
    say("| Measure | First | Last | Peak | Per hour |\n|---|---|---|---|---|")
    series = {}
    for who in ("libri", "hippo"):
        for key, label, scale in (("rss_kib", "rss MiB", 1024), ("fds", "fds", 1), ("threads", "threads", 1)):
            series[f"{who} {label}"] = [(s["at"], s[who][key] / scale) for s in samples if s.get(who)]
    series["state KiB"] = [(s["at"], s["state_kib"]) for s in samples]
    series["hippo KiB"] = [(s["at"], s["hippo_kib"]) for s in samples]
    series["stand-ins alive"] = [(s["at"], s["stand_ins"]) for s in samples]
    series["turn p50 s"] = [(s["at"], s["turn_p50"]) for s in samples if s["turn_p50"] is not None]
    series["turn max s"] = [(s["at"], s["turn_max"]) for s in samples if s["turn_max"] is not None]
    series["guest available MiB"] = [(s["at"], s["available_mib"]) for s in samples]
    for label, points in series.items():
        if points:
            v = [p[1] for p in points]
            say(f"| {label} | {v[0]:.1f} | {v[-1]:.1f} | {max(v):.1f} | {slope_per_hour(points):+.1f} |")
else:
    say("Too few samples.")

say("\n## Load\n")
for e in events:
    if e["kind"] == "load":
        say(f"{e['name']}: exit {e['code']}, {e['took']} s.")
        for line in e["lines"]:
            say(f"    {line}")
        if e["err"]:
            say(f"    stderr: {e['err']}")
    elif e["kind"] == "load memory":
        say(f"\nLowest guest memory available in the load phase: {e['lowest_available_mib']} MiB.")

other = [e for e in events if e["kind"] in ("suite failed", "ledger failed", "abort", "restarted by the suite", "crossing failed")]
if other:
    say("\n## The suite itself\n")
    for e in other:
        say(f"- {e}")

(run / "report.md").write_text("\n".join(out) + "\n")
