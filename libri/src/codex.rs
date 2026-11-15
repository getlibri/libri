//! Codex through its app-server (`codex app-server`, JSON-RPC over stdio),
//! the native interface its own IDE extensions use, on the person's
//! ChatGPT sign-in. A chat that keeps a session (`Run::session`) goes on
//! with its own thread, `thread/resume`, and a worker's begins as a
//! `thread/fork` of its head's, as T3 Code does; Codex compacts the thread
//! itself. Any other call is one ephemeral thread.
//!
//! Its items map onto the door's steps: agent message deltas to words,
//! reasoning summaries to thinking, commands, file changes, MCP and web
//! search calls to calls and results, approval requests to asks.

use crate::model::{Ask, Attachment, Harness, Run, Step};
use crate::rpc::{Incoming, Session};
use crate::servers;
use crate::sessions::{self, Start};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const START: Duration = Duration::from_secs(120);
/// This harness's name in the session records.
const HARNESS: &str = "codex";
/// What the app-server answers for a thread it has no rollout file of.
const LOST_THREAD: &str = "no rollout found for thread id";
const CANCEL: Duration = Duration::from_secs(10);

pub struct Codex {
    pub command: String,
    pub cwd: PathBuf,
    pub effort: Option<String>,
}

impl Codex {
    pub fn from_env() -> Result<Self, String> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let cwd = var("LIBRI_WORKDIR")
            .or_else(|| var("STATE_DIRECTORY"))
            .map(PathBuf::from)
            .map_or_else(std::env::current_dir, Ok)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            command: var("LIBRI_CODEX").unwrap_or_else(|| "codex".into()),
            cwd,
            effort: var("LIBRI_EFFORT"),
        })
    }
}

impl Harness for Codex {
    fn answer(
        &self,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        run: Run,
    ) -> Result<String, String> {
        let mut cmd = Command::new(&self.command);
        cmd.arg("app-server").current_dir(&self.cwd);
        run.temp(&mut cmd);
        let s = Session::spawn(cmd, false)?;
        let turn = self.turn(&s, system, memory, prompt, files, &run);
        if let Some(steer) = &run.steer {
            steer.close();
        }
        s.end();
        turn
    }

    fn tools(&self) -> bool {
        true
    }
}

impl Codex {
    /// The thread this turn runs on, and whether it already holds the chat:
    /// the chat's own resumed, a fork of its head's, else a new one, kept
    /// only for a chat that keeps a session. A thread Codex can no longer
    /// load is forgotten and a new one started.
    fn thread(
        &self,
        s: &Session,
        params: Value,
        run: &Run,
        aside: &mut Vec<Incoming>,
    ) -> Result<(Value, bool), String> {
        let kept = sessions::state().zip(run.session.clone());
        let kept = kept.filter(|_| !run.plain && !run.ephemeral);
        let with = |extra: Value| {
            let mut p = params.clone();
            p.as_object_mut()
                .expect("params are an object")
                .extend(extra.as_object().cloned().unwrap_or_default());
            p
        };
        let Some((state, chat)) = kept else {
            let t = s.call(
                "thread/start",
                with(json!({"ephemeral": true})),
                START,
                aside,
            )?;
            return Ok((t, false));
        };
        let start = sessions::start(&state, &chat, run.fork.as_deref(), HARNESS, String::new);
        let gone_on = match &start {
            Start::Resume(id) => s.call(
                "thread/resume",
                with(json!({"threadId": id, "excludeTurns": true})),
                START,
                aside,
            ),
            Start::Fork(id) => s.call(
                "thread/fork",
                with(json!({"threadId": id, "excludeTurns": true, "ephemeral": false})),
                START,
                aside,
            ),
            Start::New(_) => Err(String::new()),
        };
        let (thread, holds) = match gone_on {
            Ok(t) => (t, true),
            // Codex no longer has the thread: its rollout file is gone. Any
            // other failure fails the turn, and the thread stays the chat's.
            Err(e) if e.is_empty() || e.contains(LOST_THREAD) => {
                if let Start::Resume(_) = start {
                    eprintln!("libri: {chat}: codex lost its thread ({e}); starting a new one");
                    sessions::forget(&state, &chat, HARNESS)?;
                }
                let t = s.call(
                    "thread/start",
                    with(json!({"ephemeral": false})),
                    START,
                    aside,
                )?;
                (t, false)
            }
            Err(e) => return Err(e),
        };
        if let Some(id) = thread["thread"]["id"].as_str() {
            sessions::put(&state, &chat, HARNESS, id)?;
        }
        Ok((thread, holds))
    }

    fn turn(
        &self,
        s: &Session,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        run: &Run,
    ) -> Result<String, String> {
        let mut aside = Vec::new();
        let hello = json!({"clientInfo": {"name": "libri", "title": "Libri", "version": env!("CARGO_PKG_VERSION")}});
        s.call("initialize", hello, START, &mut aside)?;
        s.peer.notify("initialized", json!({}))?;
        let model = run.model_for("codex");
        let effort = run.effort().or_else(|| self.effort.clone());
        let mut config = Map::new();
        let tools = servers::of_run(run)?;
        // Codex merges this over the person's own servers, which stay.
        config.insert(
            "mcp_servers".into(),
            servers::codex(&tools.unwrap_or_default()),
        );
        // hippo is the only memory.
        config.insert("features".into(), json!({"memories": false}));
        // Compacts the thread at the host's point, not at its model's window.
        config.insert(
            "model_auto_compact_token_limit".into(),
            json!(crate::sessions::compact_now()),
        );
        // The thread's own level as well as the turn's, so thread/start
        // answers with the level it holds and the roster can show it.
        if let Some(e) = &effort {
            config.insert("model_reasoning_effort".into(), json!(e));
        }
        let grant = run.access("codex");
        let params = json!({
                "cwd": &self.cwd,
                "model": model,
                "approvalPolicy": match (run.plain, grant.mode.as_str()) {
                    (true, _) => "never",
                    // A preset, not a policy of Codex's own: on-request
                    // with the reviewer answering in the person's place.
                    (false, "approve-for-me") => "on-request",
                    (false, mode) => mode,
                },
                "approvalsReviewer": (grant.mode == "approve-for-me").then_some("auto_review"),
                "sandbox": grant.sandbox.as_deref(),
                // Beside Codex's own instructions, not in their place; a
                // plain call's words replace them.
                "developerInstructions": (!run.plain).then_some(system),
                "baseInstructions": run.plain.then_some(system),
                "config": config,
        });
        let (thread, holds_thread) = self.thread(s, params, run, &mut aside)?;
        // The model Codex settled on, its own default when none was named.
        let ran = thread["model"].as_str().map(str::to_owned).or(model);
        if let (Some(model), Some(show)) = (&ran, &run.steps) {
            show(Step::Runs {
                model: model.clone(),
                effort: thread["reasoningEffort"].as_str().map(str::to_owned),
            });
        }
        let thread = thread["thread"]["id"]
            .as_str()
            .ok_or("thread/start gave no thread id")?
            .to_owned();
        let mut input = input_of(files);
        // A thread gone on with holds what `memory` would bring.
        if !holds_thread && !memory.is_empty() {
            input.push(json!({"type": "text", "text": memory}));
        }
        input.push(json!({"type": "text", "text": prompt}));
        let started = s.call(
            "turn/start",
            // Thinking shows as Codex's summaries of it, as its own UI does.
            json!({"threadId": thread, "input": input, "effort": effort, "summary": "auto"}),
            START,
            &mut aside,
        )?;
        let turn = started["turn"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if let Some(steer) = &run.steer {
            let (peer, thread, turn) = (s.peer.clone(), thread.clone(), turn.clone());
            steer.take_words(Box::new(move |text, files| {
                let mut input = input_of(files);
                input.push(json!({"type": "text", "text": text}));
                peer.request(
                    "turn/steer",
                    json!({"threadId": thread, "expectedTurnId": turn, "input": input}),
                )
                .is_ok()
            }));
        }
        run.runs_in(s.child.id());
        let mut reading = Reading {
            model: ran,
            ..Reading::default()
        };
        for m in aside {
            if let Some(end) = reading.take(s, m, run) {
                return end;
            }
        }
        let mut cancelled: Option<Instant> = None;
        loop {
            let stopped = run.stop.as_ref().is_some_and(|f| f.load(Ordering::Relaxed));
            if stopped && cancelled.is_none() {
                s.peer.request(
                    "turn/interrupt",
                    json!({"threadId": thread, "turnId": turn}),
                )?;
                cancelled = Some(Instant::now());
                reading.withdraw(run);
            }
            if cancelled.is_some_and(|c| c.elapsed() > CANCEL) {
                return Err("stopped".into());
            }
            match s.incoming.recv_timeout(Duration::from_millis(100)) {
                Ok(m) => {
                    run.heard();
                    if let Some(end) = reading.take(s, m, run) {
                        reading.withdraw(run);
                        return match end {
                            Err(_) if cancelled.is_some() => Err("stopped".into()),
                            end => end,
                        };
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    reading.withdraw(run);
                    return Err("codex exited mid-turn".into());
                }
            }
        }
    }
}

/// `files` as Codex's input items.
fn input_of(files: &[Attachment]) -> Vec<Value> {
    files
        .iter()
        .map(|f| match f {
            Attachment::Image { mime, data } => json!({
                "type": "image", "url": format!("data:{mime};base64,{}", B64.encode(data)),
            }),
            Attachment::Pdf(_) => json!({"type": "text",
                "text": "(A PDF came with this message that this harness can't read.)"}),
        })
        .collect()
}

#[derive(Default)]
struct Reading {
    /// The words since the last call: the answer, once the turn ends.
    words: String,
    open: Arc<Mutex<Vec<String>>>,
    /// The thread's model, for the usage ledger.
    model: Option<String>,
}

impl Reading {
    fn withdraw(&self, run: &Run) {
        if let Some(a) = &run.ask {
            self.open
                .lock()
                .unwrap()
                .drain(..)
                .for_each(|id| a.withdraw(&id));
        }
    }

    /// Shows what `m` shows; the turn's end once it comes.
    fn take(&mut self, s: &Session, m: Incoming, run: &Run) -> Option<Result<String, String>> {
        let show = |step| {
            if let Some(f) = &run.steps {
                f(step)
            }
        };
        match m {
            Incoming::Notification { method, params } => match method.as_str() {
                "item/agentMessage/delta" => {
                    let t = params["delta"].as_str().unwrap_or_default().to_owned();
                    self.words.push_str(&t);
                    show(Step::Text { text: t });
                }
                "item/reasoning/summaryTextDelta" => show(Step::Thinking {
                    text: params["delta"].as_str().unwrap_or_default().to_owned(),
                }),
                // Summaries come in parts; a blank line keeps them apart.
                "item/reasoning/summaryPartAdded" => show(Step::Thinking {
                    text: "\n\n".into(),
                }),
                "item/started" => {
                    if let Some(call) = call(&params["item"]) {
                        self.words.clear();
                        show(call);
                    }
                }
                "item/completed" => {
                    let item = &params["item"];
                    if let Some(r) = result(item) {
                        show(r);
                        if item["type"] == "mcpToolCall"
                            && let Some(result) = crate::model::structured(&item["result"])
                        {
                            show(Step::Structured {
                                id: item["id"].as_str().unwrap_or_default().to_owned(),
                                result,
                            });
                        }
                    }
                }
                "thread/tokenUsage/updated" => {
                    let u = &params["tokenUsage"]["last"];
                    // Codex counts the cached share inside inputTokens.
                    show(Step::Usage {
                        input: u["inputTokens"].as_u64().unwrap_or(0),
                        cache_read: u["cachedInputTokens"].as_u64().unwrap_or(0),
                        cache_write: 0,
                        output: u["outputTokens"].as_u64().unwrap_or(0),
                        model: self.model.clone(),
                    });
                }
                "turn/completed" => {
                    let t = &params["turn"];
                    return Some(match t["status"].as_str().unwrap_or_default() {
                        "completed" => {
                            let text = self.words.trim().to_owned();
                            if run.sub.is_some() {
                                show(Step::Answer { text: text.clone() });
                            }
                            Ok(text)
                        }
                        "interrupted" => Err("stopped".into()),
                        _ => Err(format!(
                            "codex: {}",
                            t["error"]["message"].as_str().unwrap_or("the turn failed")
                        )),
                    });
                }
                "error" if params["willRetry"] != true => {
                    return Some(Err(format!("codex: {}", params["error"]["message"])));
                }
                _ => {}
            },
            Incoming::Request { id, method, params } => {
                let ask = match method.as_str() {
                    "item/commandExecution/requestApproval" => Some(Ask {
                        id: format!("codex-{}", params["itemId"].as_str().unwrap_or_default()),
                        tool: "Bash".into(),
                        input: json!({"command": params["command"]}),
                        about: params["reason"].as_str().map(str::to_owned),
                        always: false,
                    }),
                    "item/fileChange/requestApproval" => Some(Ask {
                        id: format!("codex-{}", params["itemId"].as_str().unwrap_or_default()),
                        tool: "Edit".into(),
                        input: json!({"files": params["grantRoot"]}),
                        about: params["reason"].as_str().map(str::to_owned),
                        always: false,
                    }),
                    _ => None,
                };
                let Some(ask) = ask else {
                    let _ = s.peer.refuse(&id, &method);
                    return None;
                };
                let (peer, asker, open) = (s.peer.clone(), run.ask.clone(), self.open.clone());
                thread::spawn(move || {
                    let key = ask.id.clone();
                    open.lock().unwrap().push(key.clone());
                    let yes = asker.and_then(|a| a.put(ask)).is_some();
                    open.lock().unwrap().retain(|k| *k != key);
                    let decision = if yes { "accept" } else { "decline" };
                    let _ = peer.respond(&id, json!({ "decision": decision }));
                });
            }
            Incoming::Response { .. } => {}
        }
        None
    }
}

/// The call an item starts, for the kinds that are calls.
fn call(item: &Value) -> Option<Step> {
    let id = item["id"].as_str()?.to_owned();
    let (name, input) = match item["type"].as_str()? {
        "commandExecution" => ("Bash".into(), json!({"command": item["command"]})),
        "fileChange" => ("Edit".into(), json!({"changes": item["changes"]})),
        "mcpToolCall" => (
            format!(
                "mcp__{}__{}",
                item["server"].as_str()?,
                item["tool"].as_str()?
            ),
            item["arguments"].clone(),
        ),
        "webSearch" => ("WebSearch".into(), json!({"query": item["query"]})),
        "collabAgentToolCall" => ("Agent".into(), json!({"prompt": item["prompt"]})),
        _ => return None,
    };
    Some(Step::Call { id, name, input })
}

/// The result an item's completion gives, for the kinds that are calls.
fn result(item: &Value) -> Option<Step> {
    let id = item["id"].as_str()?.to_owned();
    let failed = matches!(item["status"].as_str(), Some("failed" | "declined"));
    let text = match item["type"].as_str()? {
        "commandExecution" => item["aggregatedOutput"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        "fileChange" => item["status"].as_str().unwrap_or_default().to_owned(),
        "mcpToolCall" => match &item["error"] {
            Value::Null => item["result"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            e => e["message"].as_str().unwrap_or("failed").to_owned(),
        },
        "webSearch" => item["results"].to_string(),
        "collabAgentToolCall" => item["status"].as_str().unwrap_or_default().to_owned(),
        _ => return None,
    };
    let error =
        failed || !item["error"].is_null() || item["exitCode"].as_i64().is_some_and(|c| c != 0);
    Some(Step::Result { id, text, error })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_become_calls_and_results() {
        let started = json!({"type": "commandExecution", "id": "i1", "command": "echo hi", "status": "inProgress"});
        assert!(matches!(call(&started), Some(Step::Call { name, .. }) if name == "Bash"));
        let done = json!({"type": "commandExecution", "id": "i1", "aggregatedOutput": "hi\n", "exitCode": 0, "status": "completed"});
        assert!(
            matches!(result(&done), Some(Step::Result { text, error: false, .. }) if text == "hi\n")
        );
        let mcp = json!({"type": "mcpToolCall", "id": "i2", "server": "libri", "tool": "remind", "arguments": {}});
        assert!(
            matches!(call(&mcp), Some(Step::Call { name, .. }) if name == "mcp__libri__remind")
        );
        assert!(call(&json!({"type": "agentMessage", "id": "i3", "text": "x"})).is_none());
    }
}
