"""Synthetic cases for the ways a model can misuse Libri's tools. Each gives the scope (main chat or worker), the
context the turn opens on, canned tool results, and a check over the calls
made (name without its server, and arguments) and the final reply."""

NOW = "2026-11-04 Wed 09:10"

ROSTER = """\
Fern (working): Garden planner: one shared layout for every bed plan, …
Gus (waiting): Review the garden planner's pages … waiting on Hana
Hana (working): Release manager: lands reviewed branches on main, publishes under Ada's go …
Ivo (sleeping): Fix the sync bugs from Tess's audit …
Juno (sleeping): Side project for Ada: a comparison of city bike-share schemes …"""

WATCHES = """\
1: 08:00 daily, ask: Morning brief: today's mail and calendar, ranked.
2: 12:00 daily, ask: Check-in: mail since the last one, only what needs me now.
4: 09:00 daily, ask: Check the vendor advisory feed for anything touching our software."""


def names(calls):
    return [c["name"] for c in calls]


def quiet(reply):
    return reply.strip() == "(nothing new)"


def no(calls, *tools):
    return not any(c["name"] in tools for c in calls)


def first(calls, tool):
    return next((c["args"] for c in calls if c["name"] == tool), None)


def watch_set(calls):
    """The watch call that sets or changes one, if any."""
    for c in calls:
        a = c["args"]
        if c["name"] == "watch" and (a.get("when") or a.get("then") or a.get("stop") or "on" in a):
            return a
    return None


CASES = [
    # Follow-ons to work a running worker owns go to that worker.
    dict(
        id="follow-on-to-owner",
        scope="main",
        chat="""\
84233+1|user: (sent a photo) every bed in the planner should use the same grid, only shrinking when it has to
84234+1|tool: spawn worker: one shared grid for every bed plan; web first.
84235+1|echo: Started Fern.
84236+1|talk: I've started a worker, Fern, on the bed grid.""",
        message="also the labels are hidden on the north bed but not the south one, should show on both",
        check=lambda calls, reply: (first(calls, "tell_agent") or {}).get("name") == "Fern"
        and no(calls, "spawn"),
    ),
    # A new piece of long work no one owns goes to a new worker.
    dict(
        id="new-work-spawns",
        scope="main",
        chat="""\
84320+1|user: We don't need to copy it exactly just the general best ideas
84323+1|talk: I've told Milo to pick out the general ideas.""",
        roster="\n".join(l for l in ROSTER.splitlines() if not l.startswith("Juno")),
        message="Little side project. The national cycling-city ranking: I feel their criteria are misaligned. "
        "Research how they build it, what data exists, and what we'd need to make our own, very comprehensive version.",
        # The misuse is handing it to a worker who owns other work. Spawning or
        # answering here are both within the prompt's "would hold this chat up".
        check=lambda calls, reply: no(calls, "tell_agent"),
    ),
    # Moving a worker to another model is change_agent_model, not a new worker.
    dict(
        id="move-model",
        scope="main",
        chat="84294+1|user: We can bring the bigger model into review work. Upgraded that plan",
        message="put Gus on the bigger model, big-model-2",
        results={"change_agent_model": "Gus moves to big-model-2 on its next run."},
        check=lambda calls, reply: (first(calls, "change_agent_model") or {}).get("name") == "Gus"
        and "big-model-2" in (first(calls, "change_agent_model") or {}).get("model", "").lower()
        and no(calls, "spawn", "tell_agent"),
    ),
    # Archive, not cancel.
    dict(
        id="archive-done",
        scope="main",
        chat="82190+1|talk: Ivo and Juno both reported; nothing left for them.",
        message="archive juno, she's done",
        results={"archive_agent": "Juno archived."},
        check=lambda calls, reply: (first(calls, "archive_agent") or {}).get("name") == "Juno"
        and no(calls, "cancel_task"),
    ),
    # Cancel the running one.
    dict(
        id="stop-running",
        scope="main",
        chat="""\
83690+1|talk: box is out of memory: Fern's build holds 7GB.
83691+1|echo: Fern (working) make all""",
        message="stop fern now, she's locking up the machine",
        results={"cancel_task": "Fern stopped."},
        check=lambda calls, reply: (first(calls, "cancel_task") or {}).get("name") == "Fern"
        and no(calls, "archive_agent"),
    ),
    # Reminder: a watch at a time with a note, once.
    dict(
        id="reminder-note",
        scope="main",
        chat="84100+1|talk: the electric bill, $142.10, is due 11-06.",
        message="remind me at 5pm today to pay the electric bill",
        results={"watch": "Watch 6 set Wed 11-04 17:00, note: Pay the electric bill."},
        check=lambda calls, reply: (w := watch_set(calls)) is not None
        and (w.get("when") or {}).get("at", "").startswith("2026-11-04 17:00")
        and not (w.get("when") or {}).get("every")
        and (w.get("then") or {}).get("note")
        and not (w.get("then") or {}).get("ask"),
    ),
    # A thing done each morning: a watch with an ask worded as the request.
    dict(
        id="routine-ask",
        scope="main",
        chat="""\
84100+1|user: home is Springfield, put that in for anything local
84110+1|talk: Weekly watch 5 set for the package updates.""",
        message="every morning at 7 check the weather in Springfield and tell me if I need a coat",
        results={"watch": "Watch 6 set Thu 11-05 07:00 daily, ask: …"},
        check=lambda calls, reply: (w := watch_set(calls)) is not None
        and (w.get("when") or {}).get("every") == "daily"
        and (w.get("when") or {}).get("at", "").endswith("07:00")
        and (w.get("then") or {}).get("ask")
        and not (w.get("then") or {}).get("note"),
    ),
    # Watching mail: a look on a read-only tool, every 5m or more, with tests.
    dict(
        id="look-mail",
        scope="main",
        chat="83900+1|user: the bank said they'd email the new card details today",
        message="tell me as soon as the bank's email comes in, check every minute",
        results={"watch": "Watch 6 set every 5m: look fastmail search_email. It finds 0 now."},
        check=lambda calls, reply: (w := watch_set(calls)) is not None
        and (w.get("look") or {}).get("server") == "fastmail"
        and (w.get("when") or {}).get("every") not in (None, "1m", "2m", "3m", "4m")
        and not (w.get("when") or {}).get("push"),
    ),
    # An outside program telling us something is a push watch, with no look.
    dict(
        id="push-webhook",
        scope="main",
        chat="79461+1|user: no hardwired use cases",
        message="I want GitHub to tell you whenever a release is published on example/garden-planner, and you tell me",
        results={"watch": "Watch 6 set: push, address https://libri.example.com/push/k3x9. Give it to the sender now."},
        check=lambda calls, reply: (w := watch_set(calls)) is not None
        and (w.get("when") or {}).get("push") is not None
        and not w.get("look")
        and "push/k3x9" in reply,
    ),
    # Stop watches by the id the tool shows, read first, not from memory.
    dict(
        id="stop-watches",
        scope="main",
        chat="81500+1|user: no brief, no check-ins",
        message="are the morning brief and check-in still on? turn them off",
        results={"watch:list": WATCHES, "watch": "Done."},
        # "Turn them off": stopping and switching off both answer it.
        # Read the list from the tool before stopping; a zoom first is allowed.
        check=lambda calls, reply: (ws := [c for c in calls if c["name"] == "watch"])
        and not watch_set(ws[:1])
        and {c["args"].get("id") for c in calls if c["name"] == "watch"
             and (c["args"].get("stop") or c["args"].get("on") is False)} == {1, 2},
    ),
    # A worker's "still waiting" adds nothing: quiet, no action.
    dict(
        id="still-waiting-quiet",
        scope="main",
        chat="""\
84207+1|talk: Both harnesses are in, and box's build check is running.
84210+1|talk: box's build check is running on the whole change.""",
        message="[Gus] still waiting (Gus) Gus has waited 88 minutes for a reply from Pia (waiting on Hana), "
        "Hana (working) and stays paused until it comes. A word from you wakes it, or stops the wait with cancel_task.",
        check=lambda calls, reply: quiet(reply) and no(calls, "tell_agent", "cancel_task", "spawn"),
    ),
    # A question of Ada's is never quiet.
    dict(
        id="question-not-quiet",
        scope="main",
        chat="83975+1|talk: (nothing new)",
        message="so what hasn't been deployed",
        results={"workers": ROSTER, "list_agents": ROSTER},
        check=lambda calls, reply: not quiet(reply) and len(reply.strip()) > 20,
    ),
    # Needless zoom: the line already holds the message whole.
    dict(
        id="no-needless-zoom",
        scope="main",
        chat="""\
84348+1|work: [Juno] report: the 2026 cycling-city ranking weights all ten categories equally at 10% each; safety 4%, parking 2%. The 2023 edition weighted by survey: safety 20.3%, parking 2.8%.""",
        message="what weight did the 2023 edition give safety?",
        check=lambda calls, reply: no(calls, "memory_zoom", "memory_search", "agent_chat") and "20.3" in reply,
    ),
    # A condensed line that only mentions the fact: zoom before answering.
    dict(
        id="zoom-when-condensed",
        scope="main",
        chat="""\
82944+64|82944-83007: user asked about bikes; touring bike BOUGHT second-hand (model and price in the message); fitting owed.""",
        message="what was the exact bike I bought and what did it cost?",
        results={"memory_zoom": "82990+1|user: bought it: Marlow Tourer 56cm, steel frame, $2,620.99"},
        check=lambda calls, reply: (first(calls, "memory_zoom") or {}).get("id") == 82944
        and "2,620.99" in reply.replace("2620.99", "2,620.99"),
    ),
    # Workers: a <work> line opens with work_zoom; memory_zoom takes only memory_search ids.
    dict(
        id="worker-work-zoom",
        scope="worker",
        work="""\
84363+1|note: subagent
84364+1|user: Fix the Fold call in the phone app. Details of the build failure are in your memory.
84365+8|tool 84365-72: ran assembleDebug; failed at LibriApp.kt (line and message in the output).""",
        orders="Fix the Fold call in the phone app. Details of the build failure are in your memory.",
        results={"work_zoom": "84368+1|echo: e: LibriApp.kt:660:13 No value passed for parameter 'talk'"},
        check=lambda calls, reply: (first(calls, "work_zoom") or {}).get("id") == 84365
        and no(calls, "memory_zoom"),
    ),
    # Workers ask the main chat with tell_agent "main", not a sub-worker request.
    dict(
        id="worker-asks-main",
        scope="worker",
        work="""\
84363+1|note: subagent
84364+1|user: Rebase Zed's offline branch onto main and push it. Ask if Ada's rule on Settings is unclear.
84365+4|tool 84365-68: rebased; conflict in servers.rs: offline mode keeps Settings rules or drops them? Not decided in memory.""",
        orders="Rebase Zed's offline branch onto main and push it. Ask if Ada's rule on Settings is unclear.",
        check=lambda calls, reply: (first(calls, "tell_agent") or {}).get("name", "").lower() == "main"
        and no(calls, "request_subworker"),
    ),
    # Workers don't poll for a reply: one message, then end the run.
    dict(
        id="worker-no-poll",
        scope="worker",
        work="""\
84363+1|note: subagent
84364+1|user: Land your branch through Hana: tell Hana it's ready (branch fold-all 4cfff5b, Gus cleared) and report what Hana says.""",
        orders="Land your branch through Hana: tell Hana it's ready (branch fold-all 4cfff5b, Gus cleared) and report what Hana says.",
        results={"tell_agent": "Sent. Hana reads it between its tool calls, and its reply reaches you.",
                 "agent_chat": "Hana: working on release 19.", "workers": ROSTER},
        check=lambda calls, reply: (first(calls, "tell_agent") or {}).get("name") == "Hana"
        and names(calls).count("tell_agent") == 1
        and names(calls).count("agent_chat") + names(calls).count("workers") <= 1,
    ),
]
