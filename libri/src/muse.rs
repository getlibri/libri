//! Muse Code (Meta) through its own session host, `muse serve`: MSP, its
//! JSON-RPC 2.0 over stdio, the way Paseo's built-in Muse provider drives
//! it. Muse has no ACP mode. A chat that keeps a session goes on with its
//! own (`session/resume`, as Paseo's Muse provider does) for as long as its
//! profile and model stay as it was started with; Muse compacts it itself.
//! Any other turn is one session, whose files go when the turn ends.
//!
//! Muse's permission profiles are the modes (access.rs). An ephemeral host
//! (`--no-session-log`) fixes its profile, so the host keeps sessions, in a
//! data home of Libri's own (`XDG_DATA_HOME`): hippo reads Muse's sessions
//! from the person's home, and would take a turn's for a coding session of
//! its own. Auth and settings stay where Muse keeps them (`~/.config/muse`).
//! Approvals the profile puts to a person become the door's asks; Muse's
//! questions are declined, as omp's dialogs are.

use crate::model::{Ask, Attachment, Harness, Run, Step};
use crate::rpc::{Incoming, Session};
use crate::servers::{self, Scope};
use crate::sessions;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const START: Duration = Duration::from_secs(60);
/// This harness's name in the session records.
const HARNESS: &str = "muse";
const CANCEL: Duration = Duration::from_secs(10);

/// Muse's reasoning tiers, offered when a model doesn't list its own; as
/// Paseo offers them, without "none".
pub const EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh", "max", "ultra"];

pub struct Muse {
    pub command: String,
    pub cwd: PathBuf,
    /// Muse's data home for Libri's turns, apart from the person's.
    pub data: PathBuf,
    pub effort: Option<String>,
}

impl Muse {
    pub fn from_env() -> Result<Self, String> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let cwd = var("LIBRI_WORKDIR")
            .or_else(|| var("STATE_DIRECTORY"))
            .map(PathBuf::from)
            .map_or_else(std::env::current_dir, Ok)
            .map_err(|e| e.to_string())?;
        let data = var("STATE_DIRECTORY")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("muse-data");
        Ok(Self {
            command: var("LIBRI_MUSE").unwrap_or_else(|| "muse".into()),
            cwd,
            data,
            effort: var("LIBRI_EFFORT"),
        })
    }

    fn spawn(&self, args: &[&str], cwd: &PathBuf, run: Option<&Run>) -> Result<Session, String> {
        std::fs::create_dir_all(&self.data).map_err(|e| format!("{}: {e}", self.data.display()))?;
        let mut cmd = Command::new(&self.command);
        cmd.arg("serve")
            .args(args)
            .env("XDG_DATA_HOME", &self.data)
            .current_dir(cwd);
        if let Some(run) = run {
            run.temp(&mut cmd);
        }
        let s = Session::spawn(cmd, true)?;
        let mut aside = Vec::new();
        let hello = json!({
            "clientInfo": {"name": "libri", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"experimentalApi": true, "requestedCapabilities": ["sessionMcp"]},
        });
        if let Err(e) = s
            .call("initialize", hello, START, &mut aside)
            .and_then(|_| s.peer.notify("initialized", json!({})))
        {
            s.end();
            return Err(format!("muse: {e}"));
        }
        Ok(s)
    }

    /// Muse's own list, from its host's `model/list`.
    pub fn models(&self) -> Result<Value, String> {
        let s = self.spawn(&["--no-session-log"], &crate::model::bare_dir()?, None)?;
        let listed = s.call("model/list", json!({}), START, &mut Vec::new());
        s.end();
        listed.map_err(|e| format!("muse: {e}"))
    }
}

/// An MSP command's idempotency handle, a UUIDv7, which Muse requires.
fn command_id() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the system's random numbers");
    let ms = chrono::Utc::now().timestamp_millis() as u64;
    b[..6].copy_from_slice(&ms.to_be_bytes()[2..]);
    b[6] = 0x70 | (b[6] & 0x0f);
    b[8] = 0x80 | (b[8] & 0x3f);
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

impl Harness for Muse {
    fn answer(
        &self,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        run: Run,
    ) -> Result<String, String> {
        // A memory-off turn has the web alone, as far as Muse lets: no shell
        // and no writes. Its file tools still read, and no flag takes them away.
        let args: &[&str] = if run.plain || Scope::of(&run) == Some(Scope::Memoryless) {
            &["--disable-shell", "--disable-write"]
        } else {
            // The project's own skills and rules, as Paseo loads them.
            &["--trust-workspace"]
        };
        let s = self.spawn(args, &self.cwd, Some(&run))?;
        let mut session = None;
        let turn = self.turn(&s, system, memory, prompt, files, &run, &mut session);
        if let Some(steer) = &run.steer {
            steer.close();
        }
        s.end();
        // A kept session's files stay for its next turn.
        if let Some(id) = session.filter(|_| kept(&run).is_none()) {
            forget(&self.data.join("muse/sessions"), &id);
        }
        turn
    }

    fn tools(&self) -> bool {
        true
    }
}

impl Muse {
    #[allow(clippy::too_many_arguments)]
    fn turn(
        &self,
        s: &Session,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        run: &Run,
        slot: &mut Option<String>,
    ) -> Result<String, String> {
        let mut aside = Vec::new();
        let tools = servers::of_run(run)?.unwrap_or_default();
        let profile = if run.plain {
            "read-only".to_owned()
        } else {
            run.access("muse").mode
        };
        let mut start = json!({
            "commandId": command_id(),
            "workspaceRoot": self.cwd,
            "permissionProfile": format!(":{profile}"),
            "config": {"mcpServers": servers::muse(&tools)},
        });
        if let Some(m) = run.model_for("muse") {
            start["modelId"] = m.into();
        }
        // What the session was started with, which it keeps for good.
        let made = json!([start["permissionProfile"], start["modelId"]]).to_string();
        let kept = kept(run);
        let mut gone_on = None;
        if let Some((state, chat)) = &kept
            && let Some(id) = sessions::get(state, chat, HARNESS)
            && sessions::made(state, chat, HARNESS).as_deref() == Some(made.as_str())
            && sessions::current(state, chat, HARNESS)
        {
            let resume = json!({"commandId": command_id(), "sessionId": id,
                "config": start["config"], "excludeItems": true});
            match s.call("session/resume", resume, START, &mut aside) {
                Ok(r) => gone_on = Some(r),
                Err(e) => eprintln!(
                    "libri: {chat}: muse could not resume its session ({e}); starting a new one"
                ),
            }
        }
        let holds_thread = gone_on.is_some();
        let started = match gone_on {
            Some(r) => r,
            None => s
                .call("session/start", start, START, &mut aside)
                .map_err(|e| format!("muse: {e}"))?,
        };
        if let (Some(model), Some(show)) = (started["session"]["modelId"].as_str(), &run.steps) {
            // MSP reports no level back; it refuses at turn/start a tier the
            // model doesn't take, so a turn that starts took the one sent.
            show(Step::Runs {
                model: model.to_owned(),
                effort: None,
            });
        }
        let session = started["session"]["sessionId"]
            .as_str()
            .ok_or("muse: session/start gave no session id")?
            .to_owned();
        if let Some((state, chat)) = &kept
            && !holds_thread
        {
            sessions::put(state, chat, HARNESS, &session)?;
            sessions::put_made(state, chat, HARNESS, &made)?;
        }
        // A session gone on with has had both.
        let (system, memory) = if holds_thread {
            ("", "")
        } else {
            (system, memory)
        };
        let turn = self.prompt(s, &session, system, memory, prompt, files, run, &mut aside);
        *slot = Some(session);
        turn
    }

    #[allow(clippy::too_many_arguments)]
    fn prompt(
        &self,
        s: &Session,
        session: &str,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        run: &Run,
        aside: &mut Vec<Incoming>,
    ) -> Result<String, String> {
        // MSP takes no instructions on the wire: Paseo sends its own as the
        // first input part, and so does Libri.
        let mut input = Vec::new();
        if !system.is_empty() {
            input.push(json!({"type": "text",
                "text": format!("<system_instructions>\n{system}\n</system_instructions>")}));
        }
        input.extend(input_of(files));
        let text = if memory.is_empty() {
            prompt.to_owned()
        } else {
            format!("{memory}\n\n{prompt}")
        };
        input.push(json!({"type": "text", "text": text}));
        let effort = run
            .effort()
            .or_else(|| self.effort.clone())
            .filter(|e| EFFORTS.contains(&e.as_str()));
        let started = s
            .call(
                "turn/start",
                json!({"commandId": command_id(), "sessionId": session, "input": input,
                    "displayText": prompt, "reasoningEffort": effort}),
                START,
                aside,
            )
            .map_err(|e| format!("muse: {e}"))?;
        let turn = started["turnId"].as_str().unwrap_or_default().to_owned();
        if let Some(steer) = &run.steer {
            let (peer, session) = (s.peer.clone(), session.to_owned());
            steer.take_words(Box::new(move |text, files| {
                let mut input = input_of(files);
                input.push(json!({"type": "text", "text": text}));
                peer.request(
                    "turn/start",
                    json!({"commandId": command_id(), "sessionId": session, "input": input,
                        "displayText": text, "ifBusy": "steer"}),
                )
                .is_ok()
            }));
        }
        run.runs_in(s.child.id());
        let mut reading = Reading {
            session: session.to_owned(),
            turn: turn.clone(),
            ..Reading::default()
        };
        let mut ended = None;
        for m in aside.drain(..) {
            if let Some(end) = reading.take(s, m, run) {
                ended = Some(end);
                break;
            }
        }
        let mut cancelled: Option<Instant> = None;
        let ended = loop {
            if let Some(end) = ended.take() {
                break end;
            }
            let stopped = run.stop.as_ref().is_some_and(|f| f.load(Ordering::Relaxed));
            if stopped && cancelled.is_none() {
                s.peer.request(
                    "turn/interrupt",
                    json!({"commandId": command_id(), "sessionId": session, "turnId": turn}),
                )?;
                cancelled = Some(Instant::now());
                reading.withdraw(run);
            }
            if cancelled.is_some_and(|c| c.elapsed() > CANCEL) {
                break Err("stopped".into());
            }
            match s.incoming.recv_timeout(Duration::from_millis(100)) {
                Ok(m) => {
                    run.heard();
                    if let Some(end) = reading.take(s, m, run) {
                        break match end {
                            Err(_) if cancelled.is_some() => Err("stopped".into()),
                            end => end,
                        };
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break Err("muse exited mid-turn".into()),
            }
        };
        reading.withdraw(run);
        if let (Some(show), Some((input, cache_read, cache_write, output))) =
            (&run.steps, reading.usage)
        {
            show(Step::Usage {
                input,
                cache_read,
                cache_write,
                output,
                model: None,
            });
        }
        if let (Ok(text), true, Some(show)) = (&ended, run.sub.is_some(), &run.steps) {
            show(Step::Answer { text: text.clone() });
        }
        ended
    }
}

/// `files` as Muse's input parts.
fn input_of(files: &[Attachment]) -> Vec<Value> {
    files
        .iter()
        .map(|f| match f {
            Attachment::Image { mime, data } => {
                json!({"type": "image", "base64Data": B64.encode(data), "mediaType": mime})
            }
            Attachment::Pdf(_) => json!({"type": "text",
                "text": "(A PDF came with this message that this harness can't read.)"}),
        })
        .collect()
}

#[derive(Default)]
struct Reading {
    session: String,
    turn: String,
    /// The words since the last call: the answer, once the turn ends.
    words: String,
    /// Each item's kind, by id, for the deltas that name only the id.
    kinds: HashMap<String, String>,
    /// How much of each message's text has been shown.
    shown: HashMap<String, usize>,
    /// The summary part each reasoning item is on.
    part: HashMap<String, String>,
    /// The session's running totals: sent, read from cache, written to it,
    /// and out.
    usage: Option<(u64, u64, u64, u64)>,
    /// The approvals put to the person, by Muse's id.
    open: Arc<Mutex<Vec<String>>>,
}

impl Reading {
    fn withdraw(&self, run: &Run) {
        if let Some(a) = &run.ask {
            self.open
                .lock()
                .unwrap()
                .drain(..)
                .for_each(|id| a.withdraw(&ask_id(&id)));
        }
    }

    /// Shows what `m` shows; the turn's end once it comes.
    fn take(&mut self, s: &Session, m: Incoming, run: &Run) -> Option<Result<String, String>> {
        match m {
            Incoming::Notification { method, params } => {
                // A subagent's own session: its work shows as its call.
                if params["sessionId"]
                    .as_str()
                    .is_some_and(|id| id != self.session)
                {
                    return None;
                }
                match method.as_str() {
                    "approval/requested" | "approval/updated" => self.approval(s, &params, run),
                    "approval/resolved" => {
                        let id = params["approvalId"].as_str().unwrap_or_default();
                        let mut open = self.open.lock().unwrap();
                        if open.iter().any(|k| k == id) {
                            open.retain(|k| k != id);
                            if let Some(a) = &run.ask {
                                a.withdraw(&ask_id(id));
                            }
                        }
                    }
                    "userInput/requested" => {
                        let _ = s.peer.request(
                            "userInput/cancel",
                            json!({"commandId": command_id(), "sessionId": self.session,
                                "userInputId": params["userInputId"],
                                "reason": "Libri passes on only tool approvals."}),
                        );
                    }
                    "turn/completed" if params["turnId"] == self.turn.as_str() => {
                        return Some(self.ended(&params));
                    }
                    _ => {
                        for step in self.update(&method, &params) {
                            if let Some(show) = &run.steps {
                                show(step);
                            }
                        }
                    }
                }
            }
            // Muse's request for an approval is answered at once, as Paseo
            // does: the decision goes back by `approval/decide`.
            Incoming::Request { id, method, .. } => match method.as_str() {
                "approval/request" | "userInput/request" => {
                    let _ = s.peer.respond(&id, json!({}));
                }
                _ => {
                    let _ = s.peer.refuse(&id, &method);
                }
            },
            Incoming::Response { .. } => {}
        }
        None
    }

    fn ended(&self, params: &Value) -> Result<String, String> {
        match params["terminal"].as_str() {
            Some("cancelled") => Err("stopped".into()),
            _ if !params["error"].is_null() => Err(format!(
                "muse: {}",
                params["error"]["message"]
                    .as_str()
                    .unwrap_or("the turn failed")
            )),
            Some("failed") => Err("muse: the turn failed".into()),
            _ => Ok(self.words.trim().to_owned()),
        }
    }

    /// Puts an approval the profile left to a person to them, in a thread
    /// of its own so the stream reads on meanwhile. Muse's reviewer decides
    /// some itself, and sends updates on an approval already put: those
    /// reach no one.
    fn approval(&mut self, s: &Session, params: &Value, run: &Run) {
        let Some(id) = params["approvalId"].as_str().map(str::to_owned) else {
            return;
        };
        if params["change"]["kind"].is_string() || self.open.lock().unwrap().contains(&id) {
            return;
        }
        let choice = |decision: &str| {
            params["availableChoices"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|c| c["decision"] == decision && c["scope"] == "once")
                .and_then(|c| c["choiceId"].as_str())
                .map(str::to_owned)
        };
        let (Some(yes), Some(no)) = (choice("approved"), choice("abort").or(choice("denied")))
        else {
            return;
        };
        let subject = &params["subject"];
        let args: Value = params["rawArgs"]
            .as_str()
            .and_then(|a| serde_json::from_str(a).ok())
            .unwrap_or_else(|| subject.clone());
        let ask = Ask {
            id: ask_id(&id),
            tool: params["toolName"].as_str().unwrap_or("a tool").to_owned(),
            input: args,
            about: subject["kind"].as_str().map(str::to_owned),
            always: false,
        };
        let requirement = params["currentRequirementId"].clone();
        let (peer, asker, open, session) = (
            s.peer.clone(),
            run.ask.clone(),
            self.open.clone(),
            self.session.clone(),
        );
        open.lock().unwrap().push(id.clone());
        thread::spawn(move || {
            let granted = asker.and_then(|a| a.put(ask)).is_some();
            open.lock().unwrap().retain(|k| *k != id);
            let _ = peer.request(
                "approval/decide",
                json!({"commandId": command_id(), "sessionId": session, "approvalId": id,
                    "choiceId": if granted { yes } else { no }, "requirementId": requirement}),
            );
        });
    }

    /// The steps one notification shows.
    fn update(&mut self, method: &str, params: &Value) -> Vec<Step> {
        match method {
            "item/started" | "item/updated" | "item/completed" => {
                let item = &params["item"];
                let id = item["itemId"].as_str().unwrap_or_default().to_owned();
                let kind = item["kind"].as_str().unwrap_or_default().to_owned();
                let first = !self.kinds.contains_key(&id);
                self.kinds.insert(id.clone(), kind.clone());
                let done = method == "item/completed";
                match kind.as_str() {
                    "toolCall" | "subagent" | "workflow" => {
                        let mut steps = Vec::new();
                        if first {
                            // Words before a call lead into it; the answer
                            // comes after.
                            self.words.clear();
                            let name = match kind.as_str() {
                                "toolCall" => item["tool"].as_str().unwrap_or("tool"),
                                other => other,
                            };
                            let input = item["args"]
                                .as_str()
                                .and_then(|a| serde_json::from_str(a).ok())
                                .unwrap_or_else(|| json!({"objective": item["objective"]}));
                            steps.push(Step::Call {
                                id: id.clone(),
                                name: name.to_owned(),
                                input,
                            });
                        }
                        if done {
                            let text = [
                                &item["visibleOutput"],
                                &item["message"],
                                &item["fallbackText"],
                            ]
                            .into_iter()
                            .filter_map(Value::as_str)
                            .find(|t| !t.is_empty())
                            .unwrap_or_default()
                            .to_owned();
                            steps.push(Step::Result {
                                id,
                                text,
                                error: item["status"] != "completed",
                            });
                        }
                        steps
                    }
                    // What streamed short of the message as it stands.
                    "agentMessage" if done => {
                        let text = item["text"].as_str().unwrap_or_default();
                        let shown = self.shown.get(&id).copied().unwrap_or(0);
                        match text.get(shown..) {
                            Some(rest) if !rest.is_empty() => {
                                self.shown.insert(id, text.len());
                                self.words.push_str(rest);
                                vec![Step::Text {
                                    text: rest.to_owned(),
                                }]
                            }
                            _ => Vec::new(),
                        }
                    }
                    _ => Vec::new(),
                }
            }
            "item/delta" => {
                let id = params["itemId"].as_str().unwrap_or_default().to_owned();
                let delta = params["delta"].as_str().unwrap_or_default().to_owned();
                let field = params["field"].as_str().unwrap_or("text");
                match (self.kinds.get(&id).map(String::as_str), field) {
                    (Some("agentMessage"), "text") => {
                        *self.shown.entry(id).or_default() += delta.len();
                        self.words.push_str(&delta);
                        vec![Step::Text { text: delta }]
                    }
                    (Some("reasoning"), part) if part.starts_with("summary.") => {
                        // Summaries come in parts; a blank line keeps them
                        // apart.
                        let new = self.part.insert(id, part.to_owned());
                        let text = match new {
                            Some(p) if p != part => format!("\n\n{delta}"),
                            _ => delta,
                        };
                        vec![Step::Thinking { text }]
                    }
                    _ => Vec::new(),
                }
            }
            "session/tokenUsage" => {
                let c = &params["cumulative"];
                let n = |k: &str| c[k].as_u64().unwrap_or(0);
                self.usage = Some((
                    n("promptTokens"),
                    n("cacheReadTokens"),
                    n("cacheWriteTokens"),
                    n("outputTokens"),
                ));
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

fn ask_id(approval: &str) -> String {
    format!("muse-{approval}")
}

/// The chat whose session a run goes on with, and where its record is.
fn kept(run: &Run) -> Option<(std::path::PathBuf, String)> {
    sessions::state()
        .zip(run.session.clone())
        .filter(|_| !run.plain && !run.ephemeral)
}

/// Removes session `id` from Muse's store under `sessions`: its dated
/// folder and its view. Muse 1.4 answers `session/delete` with "method not
/// found"; once it deletes its own sessions, this gives way to that call.
fn forget(sessions: &std::path::Path, id: &str) {
    fn walk(dir: &std::path::Path, id: &str, depth: u8) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let path = e.path();
            if !e.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            if e.file_name() == id {
                let _ = std::fs::remove_dir_all(&path);
            } else if depth > 0 {
                walk(&path, id, depth - 1);
            }
        }
    }
    // Sessions sit at YYYY/MM/DD/<id>, views at .msp-view-*/<id>.
    walk(sessions, id, 3);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(method: &str, item: Value) -> (String, Value) {
        (method.to_owned(), json!({"item": item}))
    }

    #[test]
    fn a_turns_session_leaves_nothing_behind() {
        let root = std::env::temp_dir().join(format!("libri-muse-forget-{}", std::process::id()));
        let (id, other) = ("01a1a6e3-9940", "01a1a6e3-aaaa");
        for d in [
            format!("2026/11/04/{id}"),
            format!(".msp-view-g6/{id}"),
            format!("2026/11/04/{other}"),
        ] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        forget(&root, id);
        assert!(!root.join(format!("2026/11/04/{id}")).exists());
        assert!(!root.join(format!(".msp-view-g6/{id}")).exists());
        assert!(root.join(format!("2026/11/04/{other}")).exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn items_become_steps_and_the_answer_follows_the_last_call() {
        let mut r = Reading::default();
        let mut feed = |(m, p): (String, Value)| r.update(&m, &p);
        feed(item(
            "item/started",
            json!({"itemId": "m1", "kind": "agentMessage", "status": "inProgress", "text": ""}),
        ));
        let words = feed((
            "item/delta".into(),
            json!({"itemId": "m1", "field": "text", "delta": "Looking."}),
        ));
        assert!(matches!(&words[..], [Step::Text { text }] if text == "Looking."));
        let call = json!({"itemId": "c1", "kind": "toolCall", "tool": "bash", "status": "inProgress",
            "args": "{\"command\":\"date +%Y\"}"});
        let started = feed(item("item/started", call.clone()));
        assert!(
            matches!(&started[..], [Step::Call { name, input, .. }] if name == "bash" && input["command"] == "date +%Y")
        );
        assert!(feed(item("item/updated", call.clone())).is_empty());
        let mut done = call;
        done["status"] = "completed".into();
        done["visibleOutput"] = "2026\n".into();
        assert!(
            matches!(&feed(item("item/completed", done))[..], [Step::Result { text, error: false, .. }] if text == "2026\n")
        );
        feed(item(
            "item/started",
            json!({"itemId": "m2", "kind": "agentMessage", "status": "inProgress", "text": ""}),
        ));
        feed((
            "item/delta".into(),
            json!({"itemId": "m2", "field": "text", "delta": "It prints "}),
        ));
        // The rest arrives only on the finished message.
        let rest = feed(item(
            "item/completed",
            json!({"itemId": "m2", "kind": "agentMessage", "status": "completed", "text": "It prints 2026."}),
        ));
        assert!(matches!(&rest[..], [Step::Text { text }] if text == "2026."));
        assert_eq!(r.words.trim(), "It prints 2026.");
    }

    #[test]
    fn thinking_parts_are_kept_apart_and_usage_is_the_sessions_total() {
        let mut r = Reading::default();
        r.update(
            "item/started",
            &json!({"item": {"itemId": "t", "kind": "reasoning", "status": "inProgress"}}),
        );
        let d = |part: &str, text: &str| json!({"itemId": "t", "field": part, "delta": text});
        r.update("item/delta", &d("summary.0", "One"));
        let second = r.update("item/delta", &d("summary.1", "Two"));
        assert!(matches!(&second[..], [Step::Thinking { text }] if text == "\n\nTwo"));
        r.update(
            "session/tokenUsage",
            &json!({"cumulative": {"promptTokens": 46511, "cacheReadTokens": 23028, "outputTokens": 127}}),
        );
        assert_eq!(r.usage, Some((46511, 23028, 0, 127)));
    }

    #[test]
    fn a_turn_ends_as_muse_says() {
        let r = Reading {
            words: " done ".into(),
            ..Reading::default()
        };
        assert_eq!(
            r.ended(&json!({"terminal": "completed"})),
            Ok("done".into())
        );
        assert_eq!(
            r.ended(&json!({"terminal": "cancelled"})),
            Err("stopped".into())
        );
        let failed = json!({"terminal": "failed", "error": {"kind": "x", "message": "quota"}});
        assert_eq!(r.ended(&failed), Err("muse: quota".into()));
    }

    #[test]
    fn command_ids_are_uuid_v7() {
        let id = command_id();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "7");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
    }
}
