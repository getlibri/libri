//! The page's wire: AG-UI (docs.ag-ui.com), the open protocol between an
//! agent and the screens that show it. The door keeps its own account of
//! what happens (`Shown`); this turns it into AG-UI events, once for all
//! the pages, so every page sees the same message and run ids.
//!
//! How Libri maps onto it:
//! - a turn is a run; a run that waits on the person's word ends with an
//!   interrupt, and the run that answers it goes on with the same turn;
//! - words and thinking stream as text and reasoning messages, calls as
//!   tool calls, subagents as subagent events with their work attributed;
//! - an act waiting for approval (a mail draft) is an activity, not an
//!   interrupt: it outlives the turn and the person may talk on meanwhile,
//!   which AG-UI's "pending interrupts block new input" forbids;
//! - what AG-UI has no event for (reactions, pictures, files, the
//!   forgetting) is a CUSTOM event by name.
//!
//! Departure: AG-UI streams a run in answer to the POST that starts it.
//! Here one stream per page carries the whole conversation, since the
//! person talks from several devices and doors at once and each should
//! see the others' turns as they happen.

use crate::model::{Ask, Step};
use crate::pending::Pending;
use crate::scratchpad::PREFIX;
use serde_json::{Value, json};
use std::collections::HashMap;

/// The one conversation each assistant has.
pub const THREAD: &str = "main";

/// A subagent as the door lists it: its id, task and title.
#[derive(Clone, Debug, PartialEq)]
pub struct Sub {
    pub id: String,
    pub task: String,
    pub title: Option<String>,
}

/// What the door shows, as the encoder needs it.
pub enum In<'a> {
    Said {
        id: &'a str,
        text: &'a str,
        plain: bool,
        aside: bool,
    },
    Heard {
        id: &'a str,
        text: &'a str,
    },
    Working(bool),
    Step {
        step: &'a Step,
        agent: Option<&'a str>,
    },
    Agents(&'a [Sub]),
    Ask(&'a Ask),
    Asked(&'a str),
    Pending(&'a Pending),
    Settled {
        id: u64,
        text: &'a str,
        done: bool,
        scratchpad: bool,
    },
    Custom {
        name: &'a str,
        value: Value,
    },
}

/// The state that turns the door's account into AG-UI: which run, message
/// and reasoning are open.
#[derive(Default)]
pub struct Encoder {
    next: u64,
    run: Option<String>,
    /// A turn is under way.
    working: bool,
    /// The open answer: its id and its words so far.
    text: Option<(String, String)>,
    reasoning: Option<String>,
    /// Each subagent's open message.
    sub_text: HashMap<String, String>,
    asks: Vec<Ask>,
    subs: Vec<Sub>,
    /// A said message's id, for the streamed message it ended.
    alias: HashMap<String, String>,
}

impl Encoder {
    fn id(&mut self, kind: &str) -> String {
        self.next += 1;
        format!("{kind}-{}", self.next)
    }

    /// The events `what` makes, in order.
    pub fn encode(&mut self, what: In) -> Vec<Value> {
        let mut out = Vec::new();
        match what {
            In::Working(on) => {
                self.working = on;
                self.reconcile(false, &mut out);
            }
            In::Ask(ask) => {
                if !self.asks.iter().any(|a| a.id == ask.id) {
                    self.asks.push(ask.clone());
                    self.reconcile(true, &mut out);
                }
            }
            In::Asked(id) => {
                let was = self.asks.len();
                self.asks.retain(|a| a.id != id);
                if self.asks.len() != was {
                    self.reconcile(true, &mut out);
                }
            }
            In::Heard { id, text } => {
                let mut e = json!({"type": "TEXT_MESSAGE_CHUNK", "messageId": id, "role": "user"});
                if !text.is_empty() {
                    e["delta"] = text.into();
                }
                out.push(e);
            }
            In::Said {
                id,
                text,
                plain,
                aside,
            } => self.within_run(&mut out, |me, out| me.said(id, text, plain, aside, out)),
            In::Step { step, agent: None } => {
                self.within_run(&mut out, |me, out| me.step(step, out))
            }
            In::Step {
                step,
                agent: Some(agent),
            } => self.sub_step(agent, step, &mut out),
            In::Agents(now) => self.agents(now, &mut out),
            In::Pending(p) => out.push(json!({
                "type": "ACTIVITY_SNAPSHOT",
                "messageId": format!("act-{}", p.id),
                "activityType": "approval",
                "content": p,
            })),
            In::Settled {
                id,
                text,
                done,
                scratchpad,
            } => out.push(json!({
                "type": "ACTIVITY_DELTA",
                "messageId": format!("act-{id}"),
                "activityType": "approval",
                "patch": [{"op": "add", "path": "/outcome", "value": {"text": text, "done": done, "scratchpad": scratchpad}}],
            })),
            In::Custom { name, value } => {
                // A reaction to a said answer goes to the message it ended.
                let value = match (name, value) {
                    ("react", mut v) => {
                        if let Some(m) = v["to"].as_str().and_then(|t| self.alias.get(t)) {
                            v["to"] = m.clone().into();
                        }
                        v
                    }
                    (_, v) => v,
                };
                // Pictures and files are part of the talk, so of a run.
                if matches!(name, "picture" | "file") {
                    self.within_run(&mut out, |_, out| {
                        out.push(json!({"type": "CUSTOM", "name": name, "value": value}))
                    });
                } else {
                    out.push(json!({"type": "CUSTOM", "name": name, "value": value}));
                }
            }
        }
        out
    }

    /// Opens or closes the run as the turn and its asks stand: a run is
    /// open while the turn works and asks nothing. `asked` when the asks
    /// changed, which a page must hear even while no run is open.
    fn reconcile(&mut self, asked: bool, out: &mut Vec<Value>) {
        let want = self.working && self.asks.is_empty();
        match (&self.run, want) {
            (Some(_), false) => self.finish(out),
            (None, true) => self.start(out),
            // The asks changed while the run waits: a run that only says
            // so, as the next run would.
            (None, false) if asked => {
                self.start(out);
                self.finish(out);
            }
            _ => {}
        }
    }

    fn start(&mut self, out: &mut Vec<Value>) {
        let run = self.id("run");
        out.push(json!({"type": "RUN_STARTED", "threadId": THREAD, "runId": run}));
        self.run = Some(run);
    }

    /// Ends the run, with the asks still open as its interrupts. Words
    /// the turn never said as its answer (a quiet NO_REPLY, a stop) end
    /// marked `discarded`.
    fn finish(&mut self, out: &mut Vec<Value>) {
        self.close(true, out);
        for (_, id) in self.sub_text.drain() {
            out.push(json!({"type": "TEXT_MESSAGE_END", "messageId": id}));
        }
        let Some(run) = self.run.take() else {
            return;
        };
        let outcome = if self.asks.is_empty() {
            json!({"type": "success"})
        } else {
            json!({"type": "interrupt", "interrupts": self.asks.iter().map(interrupt).collect::<Vec<_>>()})
        };
        out.push(
            json!({"type": "RUN_FINISHED", "threadId": THREAD, "runId": run, "outcome": outcome}),
        );
    }

    /// Runs `f` inside a run: the turn's, or one of its own for what comes
    /// from no turn (a reminder, the files sent after an answer).
    fn within_run(&mut self, out: &mut Vec<Value>, f: impl FnOnce(&mut Self, &mut Vec<Value>)) {
        let own = self.run.is_none();
        if own {
            self.start(out);
        }
        f(self, out);
        if own {
            self.finish(out);
        }
    }

    /// Ends the open reasoning, and the open answer when `text`.
    fn close(&mut self, text: bool, out: &mut Vec<Value>) {
        if let Some(id) = self.reasoning.take() {
            out.push(json!({"type": "REASONING_MESSAGE_END", "messageId": id}));
            out.push(json!({"type": "REASONING_END", "messageId": id}));
        }
        if text && let Some((id, _)) = self.text.take() {
            out.push(json!({"type": "TEXT_MESSAGE_END", "messageId": id, "metadata": {"discarded": true}}));
        }
    }

    fn said(&mut self, id: &str, text: &str, plain: bool, aside: bool, out: &mut Vec<Value>) {
        // The answer the turn streamed ends here; the CLI's result holds
        // only its last block, so the streamed words end with it.
        if !plain
            && !aside
            && let Some((open, words)) = &self.text
            && words.trim_end().ends_with(text.trim())
        {
            let open = open.clone();
            self.close(false, out);
            self.text = None;
            out.push(json!({"type": "TEXT_MESSAGE_END", "messageId": open}));
            self.alias.insert(id.to_owned(), open);
            return;
        }
        if !aside {
            self.close(true, out);
        }
        let mut start = json!({"type": "TEXT_MESSAGE_START", "messageId": id, "role": "assistant"});
        let mut meta = serde_json::Map::new();
        if plain {
            meta.insert("plain".into(), true.into());
        }
        if aside {
            meta.insert("aside".into(), true.into());
        }
        if !meta.is_empty() {
            start["metadata"] = meta.into();
        }
        out.push(start);
        if !text.is_empty() {
            out.push(json!({"type": "TEXT_MESSAGE_CONTENT", "messageId": id, "delta": text}));
        }
        out.push(json!({"type": "TEXT_MESSAGE_END", "messageId": id}));
    }

    fn step(&mut self, step: &Step, out: &mut Vec<Value>) {
        match step {
            Step::Text { text } if !text.is_empty() => {
                if self.reasoning.is_some() {
                    self.close(false, out);
                }
                let id = match &mut self.text {
                    Some((id, words)) => {
                        words.push_str(text);
                        id.clone()
                    }
                    None => {
                        let id = self.id("msg");
                        out.push(json!({"type": "TEXT_MESSAGE_START", "messageId": id, "role": "assistant"}));
                        self.text = Some((id.clone(), text.clone()));
                        id
                    }
                };
                out.push(json!({"type": "TEXT_MESSAGE_CONTENT", "messageId": id, "delta": text}));
            }
            Step::Thinking { text } if !text.is_empty() => {
                let id = match &self.reasoning {
                    Some(id) => id.clone(),
                    None => {
                        let id = self.id("think");
                        out.push(json!({"type": "REASONING_START", "messageId": id}));
                        out.push(json!({"type": "REASONING_MESSAGE_START", "messageId": id, "role": "reasoning"}));
                        self.reasoning = Some(id.clone());
                        id
                    }
                };
                out.push(
                    json!({"type": "REASONING_MESSAGE_CONTENT", "messageId": id, "delta": text}),
                );
            }
            Step::Call { id, name, input } => {
                // Words before a call are said on the way to it, not the answer.
                self.close(false, out);
                let parent = self.text.take().map(|(m, _)| {
                    out.push(json!({"type": "TEXT_MESSAGE_END", "messageId": m}));
                    m
                });
                call(id, name, input, parent, None, out);
            }
            Step::Result { id, text, error } => result(id, text, *error, None, out),
            Step::Structured { id, result } => out.push(structured(id, result, None)),
            Step::Progress { id, text } => out.push(progress(id, text, None)),
            Step::Metrics {
                context_tokens,
                context_window,
                tokens_per_second,
            } => out.push(json!({
                "type": "CUSTOM", "name": "libri.metrics", "value": {
                    "context_tokens": context_tokens, "context_window": context_window,
                    "tokens_per_second": tokens_per_second,
                },
            })),
            Step::Usage {
                input,
                output,
                cache_read,
                cache_write,
                model,
            } => out.push(json!({
                "type": "CUSTOM", "name": "libri.usage", "value": {
                    "input": input, "output": output, "cache_read": cache_read,
                    "cache_write": cache_write, "model": model,
                },
            })),
            Step::Jobs { names } => out.push(json!({
                "type": "ACTIVITY_SNAPSHOT",
                "messageId": format!("jobs-{}", self.run.as_deref().unwrap_or_default()),
                "activityType": "jobs",
                "content": {"names": names},
            })),
            Step::Done { id, report } => {
                self.subs.retain(|s| s.id != *id);
                out.push(
                    json!({"type": "SUBAGENT_FINISHED", "subagentRunId": id, "result": report}),
                );
            }
            // A worker's roster line shows it; the main chat's picker names
            // its model already.
            Step::Runs { .. } => {}
            // The door turns these into others before they come here.
            Step::Text { .. }
            | Step::Thinking { .. }
            | Step::Answer { .. }
            | Step::Agent { .. }
            | Step::Row { .. }
            | Step::Stored { .. }
            | Step::Inner { .. } => {}
        }
    }

    /// A subagent's step, attributed to it.
    fn sub_step(&mut self, agent: &str, step: &Step, out: &mut Vec<Value>) {
        let sub = Some(agent);
        match step {
            Step::Text { text } if !text.is_empty() => {
                let id = match self.sub_text.get(agent) {
                    Some(id) => id.clone(),
                    None => {
                        let id = self.id("msg");
                        out.push(json!({"type": "TEXT_MESSAGE_START", "messageId": id, "role": "assistant", "subagentRunId": agent}));
                        self.sub_text.insert(agent.to_owned(), id.clone());
                        id
                    }
                };
                out.push(json!({"type": "TEXT_MESSAGE_CONTENT", "messageId": id, "delta": text, "subagentRunId": agent}));
            }
            Step::Call { id, name, input } => {
                let parent = self.sub_text.remove(agent).map(|m| {
                    out.push(
                        json!({"type": "TEXT_MESSAGE_END", "messageId": m, "subagentRunId": agent}),
                    );
                    m
                });
                call(id, name, input, parent, sub, out);
            }
            Step::Result { id, text, error } => result(id, text, *error, sub, out),
            Step::Structured { id, result } => out.push(structured(id, result, sub)),
            Step::Progress { id, text } => out.push(progress(id, text, sub)),
            metrics @ (Step::Metrics { .. } | Step::Usage { .. }) => {
                let mut events = Vec::new();
                self.step(metrics, &mut events);
                for mut event in events {
                    event["subagentRunId"] = agent.into();
                    out.push(event);
                }
            }
            // A worker's page shows its log, which holds no thinking; a
            // scratchpad's and a named memory's are chats like the main one.
            Step::Thinking { text }
                if !text.is_empty()
                    && (agent.starts_with(PREFIX)
                        || agent.starts_with(crate::memories::PREFIX)) =>
            {
                out.push(json!({
                    "type": "REASONING_MESSAGE_CONTENT",
                    "delta": text,
                    "subagentRunId": agent,
                }))
            }
            _ => {}
        }
    }

    /// The subagents at work: those new started, those gone finished.
    fn agents(&mut self, now: &[Sub], out: &mut Vec<Value>) {
        for gone in self
            .subs
            .iter()
            .filter(|s| !now.iter().any(|n| n.id == s.id))
        {
            // No run's end closes these words while the chat itself is idle.
            if let Some(m) = self.sub_text.remove(&gone.id) {
                out.push(
                    json!({"type": "TEXT_MESSAGE_END", "messageId": m, "subagentRunId": gone.id}),
                );
            }
            out.push(json!({"type": "SUBAGENT_FINISHED", "subagentRunId": gone.id}));
        }
        for new in now.iter().filter(|n| !self.subs.contains(n)) {
            let mut e = json!({
                "type": "SUBAGENT_STARTED",
                "subagentRunId": new.id,
                "name": new.title.as_deref().unwrap_or(&new.id),
                "description": new.task,
            });
            // The seat's own subagents go by the id of the Agent call.
            if new.title.is_some() {
                e["parentToolCallId"] = new.id.clone().into();
            }
            out.push(e);
        }
        self.subs = now.to_vec();
    }
}

fn call(
    id: &str,
    name: &str,
    input: &Value,
    parent: Option<String>,
    sub: Option<&str>,
    out: &mut Vec<Value>,
) {
    let mut start = json!({"type": "TOOL_CALL_START", "toolCallId": id, "toolCallName": name});
    if let Some(p) = parent {
        start["parentMessageId"] = p.into();
    }
    let mut args = json!({"type": "TOOL_CALL_ARGS", "toolCallId": id, "delta": input.to_string()});
    let mut end = json!({"type": "TOOL_CALL_END", "toolCallId": id});
    if let Some(s) = sub {
        for e in [&mut start, &mut args, &mut end] {
            e["subagentRunId"] = s.into();
        }
    }
    out.extend([start, args, end]);
}

fn result(id: &str, text: &str, error: bool, sub: Option<&str>, out: &mut Vec<Value>) {
    let mut e = json!({
        "type": "TOOL_CALL_RESULT",
        "messageId": format!("result-{id}"),
        "toolCallId": id,
        "content": text,
        "role": "tool",
    });
    if error {
        e["metadata"] = json!({"error": true});
    }
    if let Some(s) = sub {
        e["subagentRunId"] = s.into();
    }
    out.push(e);
}

/// A call's whole MCP result, for its card: AG-UI's tool result carries
/// text alone, so it goes as Libri's own event beside it.
fn structured(id: &str, result: &Value, sub: Option<&str>) -> Value {
    let mut e = json!({
        "type": "CUSTOM",
        "name": "libri.tool_result",
        "value": {"toolCallId": id, "result": result},
    });
    if let Some(s) = sub {
        e["subagentRunId"] = s.into();
    }
    e
}

/// A running call's output so far, as AG-UI's activity snapshot: the
/// page puts it under the call's line until the result replaces it.
fn progress(id: &str, text: &str, sub: Option<&str>) -> Value {
    let mut e = json!({
        "type": "ACTIVITY_SNAPSHOT",
        "messageId": format!("output-{id}"),
        "activityType": "tool_output",
        "content": {"toolCallId": id, "text": text},
    });
    if let Some(s) = sub {
        e["subagentRunId"] = s.into();
    }
    e
}

/// An ask as an AG-UI interrupt: questions want input, anything else a
/// yes to the call. Answered by a resume entry: `resolved` lets the tool
/// run (with `payload.answers` for questions), `cancelled` refuses it.
pub fn interrupt(a: &Ask) -> Value {
    let questions = a.tool == "AskUserQuestion";
    let mut i = json!({
        "id": a.id,
        "reason": if questions { "input_required" } else { "tool_call" },
        "metadata": {"tool": a.tool, "input": a.input},
    });
    if a.always {
        i["metadata"]["always"] = true.into();
    }
    if let Some(about) = &a.about {
        i["message"] = about.clone().into();
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(es: &[Value]) -> Vec<&str> {
        es.iter().map(|e| e["type"].as_str().unwrap()).collect()
    }

    fn text(t: &str) -> Step {
        Step::Text { text: t.into() }
    }

    #[test]
    fn metrics_keep_their_worker_attribution_and_never_become_words() {
        let mut e = Encoder::default();
        e.encode(In::Working(true));
        let metrics = Step::Metrics {
            context_tokens: Some(32000),
            context_window: Some(128000),
            tokens_per_second: Some(42.5),
        };
        let own = e.encode(In::Step {
            step: &metrics,
            agent: None,
        });
        assert_eq!(own.len(), 1);
        assert_eq!(own[0]["name"], "libri.metrics");
        assert_eq!(own[0]["value"]["context_tokens"], 32000);
        let sub = e.encode(In::Step {
            step: &metrics,
            agent: Some("Ada"),
        });
        assert_eq!(sub.len(), 1);
        assert_eq!(sub[0]["subagentRunId"], "Ada");
    }

    #[test]
    fn a_turn_streams_and_its_answer_ends_the_message() {
        let mut e = Encoder::default();
        assert_eq!(types(&e.encode(In::Working(true))), ["RUN_STARTED"]);
        let first = e.encode(In::Step {
            step: &text("Hel"),
            agent: None,
        });
        assert_eq!(
            types(&first),
            ["TEXT_MESSAGE_START", "TEXT_MESSAGE_CONTENT"]
        );
        e.encode(In::Step {
            step: &text("lo"),
            agent: None,
        });
        let said = e.encode(In::Said {
            id: "web-1",
            text: "Hello",
            plain: false,
            aside: false,
        });
        assert_eq!(types(&said), ["TEXT_MESSAGE_END"]);
        assert_eq!(said[0]["messageId"], first[0]["messageId"]);
        // A reaction to the said answer finds the streamed message.
        let r = e.encode(In::Custom {
            name: "react",
            value: json!({"to": "web-1", "emoji": "👍"}),
        });
        assert_eq!(r[0]["value"]["to"], first[0]["messageId"]);
        let end = e.encode(In::Working(false));
        assert_eq!(types(&end), ["RUN_FINISHED"]);
        assert_eq!(end[0]["outcome"]["type"], "success");
    }

    /// The Beck report turn of 11-06 replayed through the encoder: the
    /// answer streams, a call follows, more words, then a second answer.
    /// Said where the daemon now says it (before the call), the answer is
    /// one message; said after the call, as it was, the page drew it twice.
    #[test]
    fn a_report_turns_answer_said_before_the_next_call_is_one_message() {
        fn replay(said_late: bool) -> Vec<String> {
            let mut e = Encoder::default();
            let mut out = e.encode(In::Working(true));
            let answer = "Beck is done.";
            let said = |e: &mut Encoder, id: &str, text: &str| {
                e.encode(In::Said {
                    id,
                    text,
                    plain: false,
                    aside: false,
                })
            };
            let call = Step::Call {
                id: "t1".into(),
                name: "mcp__libri__tell_agent".into(),
                input: json!({"name": "Sylvie"}),
            };
            let result = Step::Result {
                id: "t1".into(),
                text: "Sent.".into(),
                error: false,
            };
            out.extend(e.encode(In::Step {
                step: &text(answer),
                agent: None,
            }));
            if !said_late {
                out.extend(said(&mut e, "h-1", answer));
            }
            for s in [&call, &result] {
                out.extend(e.encode(In::Step {
                    step: s,
                    agent: None,
                }));
            }
            if said_late {
                out.extend(said(&mut e, "h-1", answer));
            }
            out.extend(e.encode(In::Step {
                step: &text("Passed on."),
                agent: None,
            }));
            out.extend(said(&mut e, "h-2", "Passed on."));
            out.extend(e.encode(In::Working(false)));
            // The words of each assistant message, in order.
            let mut texts: Vec<(String, String)> = Vec::new();
            for ev in &out {
                let id = ev["messageId"].as_str().unwrap_or_default().to_owned();
                match ev["type"].as_str().unwrap() {
                    "TEXT_MESSAGE_START" => texts.push((id, String::new())),
                    "TEXT_MESSAGE_CONTENT" => {
                        let m = texts.iter_mut().rev().find(|(m, _)| *m == id).unwrap();
                        m.1.push_str(ev["delta"].as_str().unwrap());
                    }
                    _ => {}
                }
            }
            texts.into_iter().map(|(_, t)| t).collect()
        }
        assert_eq!(replay(false), ["Beck is done.", "Passed on."]);
        assert_eq!(
            replay(true),
            ["Beck is done.", "Beck is done.", "Passed on."]
        );
    }

    #[test]
    fn words_never_said_end_discarded() {
        let mut e = Encoder::default();
        e.encode(In::Working(true));
        e.encode(In::Step {
            step: &text("NO_REPLY"),
            agent: None,
        });
        let end = e.encode(In::Working(false));
        assert_eq!(types(&end), ["TEXT_MESSAGE_END", "RUN_FINISHED"]);
        assert_eq!(end[0]["metadata"]["discarded"], true);
    }

    #[test]
    fn a_call_takes_the_words_before_it() {
        let mut e = Encoder::default();
        e.encode(In::Working(true));
        let w = e.encode(In::Step {
            step: &text("Looking."),
            agent: None,
        });
        let call = Step::Call {
            id: "t1".into(),
            name: "Bash".into(),
            input: json!({"command": "ls"}),
        };
        let c = e.encode(In::Step {
            step: &call,
            agent: None,
        });
        assert_eq!(
            types(&c),
            [
                "TEXT_MESSAGE_END",
                "TOOL_CALL_START",
                "TOOL_CALL_ARGS",
                "TOOL_CALL_END"
            ]
        );
        assert_eq!(c[1]["parentMessageId"], w[0]["messageId"]);
        assert_eq!(c[2]["delta"], r#"{"command":"ls"}"#);
    }

    #[test]
    fn an_ask_interrupts_the_run_and_its_answer_resumes() {
        let mut e = Encoder::default();
        e.encode(In::Working(true));
        let ask = Ask {
            id: "a1".into(),
            tool: "Bash".into(),
            input: json!({}),
            about: Some("Runs ls".into()),
            always: false,
        };
        let paused = e.encode(In::Ask(&ask));
        assert_eq!(types(&paused), ["RUN_FINISHED"]);
        assert_eq!(paused[0]["outcome"]["type"], "interrupt");
        assert_eq!(paused[0]["outcome"]["interrupts"][0]["id"], "a1");
        assert_eq!(paused[0]["outcome"]["interrupts"][0]["reason"], "tool_call");
        let resumed = e.encode(In::Asked("a1"));
        assert_eq!(types(&resumed), ["RUN_STARTED"]);
        assert_ne!(resumed[0]["runId"], paused[0]["runId"]);
    }

    #[test]
    fn what_comes_from_no_turn_has_a_run_of_its_own() {
        let mut e = Encoder::default();
        let s = e.encode(In::Said {
            id: "web-2",
            text: "Reminder",
            plain: false,
            aside: true,
        });
        assert_eq!(
            types(&s),
            [
                "RUN_STARTED",
                "TEXT_MESSAGE_START",
                "TEXT_MESSAGE_CONTENT",
                "TEXT_MESSAGE_END",
                "RUN_FINISHED"
            ]
        );
        assert_eq!(s[1]["metadata"]["aside"], true);
    }

    #[test]
    fn a_subagent_at_work_opens_no_run_while_the_chat_is_idle() {
        let mut e = Encoder::default();
        let sub = Sub {
            id: "Ivo".into(),
            task: "build".into(),
            title: None,
        };
        e.encode(In::Agents(std::slice::from_ref(&sub)));
        let word = e.encode(In::Step {
            step: &text("working"),
            agent: Some("Ivo"),
        });
        assert_eq!(types(&word), ["TEXT_MESSAGE_START", "TEXT_MESSAGE_CONTENT"]);
        let gone = e.encode(In::Agents(&[]));
        assert_eq!(types(&gone), ["TEXT_MESSAGE_END", "SUBAGENT_FINISHED"]);
    }

    #[test]
    fn subagents_start_and_finish_with_their_work_attributed() {
        let mut e = Encoder::default();
        e.encode(In::Working(true));
        let sub = Sub {
            id: "c9".into(),
            task: "look".into(),
            title: Some("Search".into()),
        };
        let s = e.encode(In::Agents(std::slice::from_ref(&sub)));
        assert_eq!(types(&s), ["SUBAGENT_STARTED"]);
        assert_eq!(s[0]["parentToolCallId"], "c9");
        assert_eq!(s[0]["name"], "Search");
        let w = e.encode(In::Step {
            step: &text("hm"),
            agent: Some("c9"),
        });
        assert_eq!(w[0]["subagentRunId"], "c9");
        let f = e.encode(In::Agents(&[]));
        assert_eq!(types(&f), ["TEXT_MESSAGE_END", "SUBAGENT_FINISHED"]);
    }
}
