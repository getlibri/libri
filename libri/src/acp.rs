//! Any harness that speaks ACP (agentclientprotocol.com) over stdio: Grok
//! Build, Gemini CLI and the long tail. A chat that keeps a session goes on
//! with its own where the harness can take one up again, as T3 Code does:
//! `session/resume`, else `session/load`, else a new one each turn. The
//! harness compacts its session itself.
//!
//! What the harness reports maps onto the door's steps: message chunks to
//! words, thought chunks to thinking, tool calls and their updates to calls
//! and results, permission requests to asks. Libri's MCP servers go to
//! the harness in `session/new`, so its tools reach every harness.

use crate::access;
use crate::model::{Ask, Attachment, Harness, Run, Step};
use crate::rpc::{Incoming, Session};
use crate::servers;
use crate::sessions;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How long a harness may take to start and open a session.
const START: Duration = Duration::from_secs(120);
/// How long a cancelled turn may take to end before it is made to.
const CANCEL: Duration = Duration::from_secs(10);

pub struct Acp {
    /// The command that runs the harness as an ACP agent, as words.
    pub command: Vec<String>,
    /// Where the harness works.
    pub cwd: PathBuf,
}

impl Acp {
    /// The harness `command` runs, in LIBRI_WORKDIR (else the state
    /// directory).
    pub fn new(command: Vec<String>) -> Result<Self, String> {
        if command.is_empty() {
            return Err("no ACP command".into());
        }
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let cwd = var("LIBRI_WORKDIR")
            .or_else(|| var("STATE_DIRECTORY"))
            .map(PathBuf::from)
            .map_or_else(std::env::current_dir, Ok)
            .map_err(|e| e.to_string())?;
        Ok(Self { command, cwd })
    }
}

impl Harness for Acp {
    fn answer(
        &self,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        run: Run,
    ) -> Result<String, String> {
        let mut cmd = Command::new(&self.command[0]);
        cmd.args(&self.command[1..]).current_dir(&self.cwd);
        run.temp(&mut cmd);
        let s = Session::spawn(cmd, true)?;
        let mut opened = None;
        let turn = self.turn(&s, &mut opened, system, memory, prompt, files, &run);
        // Closed where the harness can, unless the chat goes on with it.
        if let Some(id) = opened {
            let _ = s.call(
                "session/close",
                json!({"sessionId": id}),
                CANCEL,
                &mut Vec::new(),
            );
        }
        s.end();
        turn
    }

    fn tools(&self) -> bool {
        true
    }
}

impl Acp {
    #[allow(clippy::too_many_arguments)]
    fn turn(
        &self,
        s: &Session,
        opened: &mut Option<String>,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        run: &Run,
    ) -> Result<String, String> {
        let mut aside = Vec::new();
        let init = s.call(
            "initialize",
            json!({
                "protocolVersion": 1,
                // The harness keeps to its own files and shell.
                "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
                "clientInfo": {"name": "libri", "version": env!("CARGO_PKG_VERSION")},
            }),
            START,
            &mut aside,
        )?;
        let can = &init["agentCapabilities"]["promptCapabilities"];
        // A turn that may not act gets none of Libri's tools.
        let servers = match servers::of_run(run)? {
            Some(list) => servers::acp(&list, &init["agentCapabilities"]["mcpCapabilities"]),
            _ => Vec::new(),
        };
        let harness = run.harness();
        let caps = &init["agentCapabilities"];
        let kept = sessions::state()
            .zip(run.session.clone())
            .filter(|_| !run.plain && !run.ephemeral);
        // How this harness takes a session up again, if it can.
        let again = if caps["sessionCapabilities"]["resume"].is_object() {
            Some("session/resume")
        } else if caps["loadSession"] == true {
            Some("session/load")
        } else {
            None
        };
        let kept = kept.filter(|_| again.is_some());
        let mut gone_on = None;
        if let (Some((state, chat)), Some(method)) = (&kept, again)
            && let Some(id) = sessions::get(state, chat, &harness)
            && sessions::current(state, chat, &harness)
        {
            let params = json!({"sessionId": id, "cwd": self.cwd, "mcpServers": servers});
            match s.call(method, params, START, &mut aside) {
                Ok(session) => {
                    // A load replays the session as updates: what is past
                    // is not this turn's.
                    aside.clear();
                    gone_on = Some((id, session));
                }
                Err(e) => {
                    eprintln!(
                        "libri: {chat}: {harness} could not take up its session ({e}); starting a new one"
                    );
                    sessions::forget(state, chat, &harness)?;
                }
            }
        }
        let holds_thread = gone_on.is_some();
        let (id, session) = match gone_on {
            Some(gone_on) => gone_on,
            None => {
                let session = s.call(
                    "session/new",
                    json!({"cwd": self.cwd, "mcpServers": servers}),
                    START,
                    &mut aside,
                )?;
                let id = session["sessionId"]
                    .as_str()
                    .ok_or("session/new gave no sessionId")?
                    .to_owned();
                (id, session)
            }
        };
        match &kept {
            Some((state, chat)) => sessions::put(state, chat, &harness, &id)?,
            None if caps["sessionCapabilities"]["close"].is_object() => {
                *opened = Some(id.clone());
            }
            None => {}
        }
        if let Some(state) = std::env::var_os("STATE_DIRECTORY") {
            access::remember_acp(Path::new(&state), &harness, &session["modes"]);
        }
        if let Some(mode) = run.access(&harness).mode.strip_prefix(access::ACP_MODE) {
            s.call(
                "session/set_mode",
                json!({"sessionId": id, "modeId": mode}),
                START,
                &mut aside,
            )?;
        }
        let model = run.model_for(&harness);
        if let Some((config, value)) =
            model.and_then(|m| pick(&session["configOptions"], "model", &m))
        {
            s.call(
                "session/set_config_option",
                json!({"sessionId": id, "configId": config, "value": value}),
                START,
                &mut aside,
            )?;
        }
        // ACP has no system prompt: it leads a new session's first message.
        let mut content = Vec::new();
        if !holds_thread {
            content.push(json!({"type": "text", "text": format!("{system}\n\n{memory}")}));
        }
        for f in files {
            match f {
                Attachment::Image { mime, data } if can["image"] == true => content
                    .push(json!({"type": "image", "mimeType": mime, "data": B64.encode(data)})),
                Attachment::Pdf(data) if can["embeddedContext"] == true => content.push(json!({
                    "type": "resource",
                    "resource": {"uri": "file:///attachment.pdf", "mimeType": "application/pdf", "blob": B64.encode(data)},
                })),
                _ => content.push(json!({"type": "text",
                    "text": "(A file came with this message that this harness can't read.)"})),
            }
        }
        content.push(json!({"type": "text", "text": prompt}));
        let asked = s.peer.request(
            "session/prompt",
            json!({"sessionId": id, "prompt": content}),
        )?;
        run.runs_in(s.child.id());
        let mut state = Reading::default();
        for m in aside {
            state.take(s, m, run);
        }
        let mut cancelled: Option<Instant> = None;
        loop {
            let stopped = run.stop.as_ref().is_some_and(|f| f.load(Ordering::Relaxed));
            if stopped && cancelled.is_none() {
                s.peer.notify("session/cancel", json!({"sessionId": id}))?;
                cancelled = Some(Instant::now());
                state.withdraw(run);
            }
            if cancelled.is_some_and(|c| c.elapsed() > CANCEL) {
                return Err("stopped".into());
            }
            let got = s.incoming.recv_timeout(Duration::from_millis(100));
            if got.is_ok() {
                run.heard();
            }
            match got {
                Ok(Incoming::Response { id: got, result }) if got == asked => {
                    state.withdraw(run);
                    let result = result.map_err(|e| format!("session/prompt: {e}"))?;
                    if let (Some(show), Some(u)) = (&run.steps, result.get("usage")) {
                        show(Step::Usage {
                            input: u["inputTokens"].as_u64().unwrap_or(0),
                            cache_read: 0,
                            cache_write: 0,
                            output: u["outputTokens"].as_u64().unwrap_or(0),
                            model: None,
                        });
                    }
                    if cancelled.is_some() || result["stopReason"] == "cancelled" {
                        return Err("stopped".into());
                    }
                    let text = state.answer();
                    if let (true, Some(show)) = (run.sub.is_some(), &run.steps) {
                        show(Step::Answer { text: text.clone() });
                    }
                    return Ok(text);
                }
                Ok(m) => state.take(s, m, run),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    state.withdraw(run);
                    return Err("the harness exited mid-turn".into());
                }
            }
        }
    }
}

/// What the turn has shown so far.
#[derive(Default)]
struct Reading {
    /// The words since the last tool call: the answer, if the turn ends.
    words: String,
    /// The calls shown, so an update shows each one's result once.
    called: HashSet<String>,
    done: HashSet<String>,
    /// The asks waiting on the person.
    open: Arc<Mutex<Vec<String>>>,
}

impl Reading {
    fn answer(&self) -> String {
        self.words.trim().to_owned()
    }

    fn withdraw(&self, run: &Run) {
        if let Some(a) = &run.ask {
            self.open
                .lock()
                .unwrap()
                .drain(..)
                .for_each(|id| a.withdraw(&id));
        }
    }

    fn take(&mut self, s: &Session, m: Incoming, run: &Run) {
        match m {
            Incoming::Notification { method, params } if method == "session/update" => {
                for step in self.update(&params["update"]) {
                    if let Some(show) = &run.steps {
                        show(step);
                    }
                }
            }
            Incoming::Request { id, method, params } if method == "session/request_permission" => {
                // Each waits on the person in a thread of its own, so the
                // stream reads on meanwhile.
                let (peer, asker, open) = (s.peer.clone(), run.ask.clone(), self.open.clone());
                let approve = run.access(&run.harness()).mode == "approve";
                thread::spawn(move || {
                    let call = &params["toolCall"];
                    let ask = Ask {
                        id: format!("acp-{session}-{id}", session = params["sessionId"]),
                        tool: call["title"].as_str().unwrap_or("a tool").to_owned(),
                        input: call["rawInput"].clone(),
                        about: call["kind"].as_str().map(|k| format!("A {k} call")),
                        always: false,
                    };
                    let key = ask.id.clone();
                    open.lock().unwrap().push(key.clone());
                    // ACP has no such mode of its own to set, so the reply
                    // is the mode.
                    let yes = approve || asker.and_then(|a| a.put(ask)).is_some();
                    open.lock().unwrap().retain(|k| *k != key);
                    let _ = peer.respond(&id, permission(&params["options"], yes));
                });
            }
            Incoming::Request { id, method, .. } => {
                let _ = s.peer.refuse(&id, &method);
            }
            _ => {}
        }
    }

    /// The steps one session/update shows.
    fn update(&mut self, u: &Value) -> Vec<Step> {
        let text = || u["content"]["text"].as_str().unwrap_or_default().to_owned();
        match u["sessionUpdate"].as_str().unwrap_or_default() {
            "agent_message_chunk" => {
                let t = text();
                self.words.push_str(&t);
                vec![Step::Text { text: t }]
            }
            "agent_thought_chunk" => vec![Step::Thinking { text: text() }],
            "tool_call" | "tool_call_update" => {
                let id = u["toolCallId"].as_str().unwrap_or_default().to_owned();
                let mut steps = Vec::new();
                if self.called.insert(id.clone()) {
                    // Words before a call lead into it; the answer comes after.
                    self.words.clear();
                    steps.push(Step::Call {
                        id: id.clone(),
                        name: u["title"]
                            .as_str()
                            .or(u["kind"].as_str())
                            .unwrap_or("tool")
                            .to_owned(),
                        input: u["rawInput"].clone(),
                    });
                }
                let status = u["status"].as_str().unwrap_or_default();
                if matches!(status, "completed" | "failed") && self.done.insert(id.clone()) {
                    steps.push(Step::Result {
                        id,
                        text: output(u),
                        error: status == "failed",
                    });
                }
                steps
            }
            _ => Vec::new(),
        }
    }
}

/// The option of `category` that offers `want`, by its id, and the value
/// to set: the harness's own model list decides what may be picked.
fn pick(options: &Value, category: &str, want: &str) -> Option<(String, String)> {
    let o = options
        .as_array()?
        .iter()
        .find(|o| o["category"] == category)?;
    let offered = o["options"]
        .as_array()?
        .iter()
        .filter_map(|v| v["value"].as_str())
        .find(|v| *v == want)?;
    Some((o["id"].as_str()?.to_owned(), offered.to_owned()))
}

/// A tool call's output as text: its content blocks, else its raw output.
fn output(u: &Value) -> String {
    let blocks: Vec<&str> = u["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["content"]["text"].as_str())
        .collect();
    if !blocks.is_empty() {
        return blocks.join("\n");
    }
    match &u["rawOutput"] {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        v => v["content"][0]["text"]
            .as_str()
            .map_or_else(|| v.to_string(), str::to_owned),
    }
}

/// The answer to a permission request: the first option that allows once
/// (else always), or the first that refuses; cancelled when none fits.
fn permission(options: &Value, yes: bool) -> Value {
    let kinds: &[&str] = if yes {
        &["allow_once", "allow_always"]
    } else {
        &["reject_once", "reject_always"]
    };
    let options = options.as_array().map(Vec::as_slice).unwrap_or_default();
    kinds
        .iter()
        .find_map(|k| options.iter().find(|o| o["kind"] == *k))
        .and_then(|o| o["optionId"].as_str())
        .map_or_else(
            || json!({"outcome": {"outcome": "cancelled"}}),
            |id| json!({"outcome": {"outcome": "selected", "optionId": id}}),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_become_steps_and_the_answer_follows_the_last_call() {
        let mut r = Reading::default();
        r.update(&json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Looking."}}));
        let call = json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "echo hi", "kind": "execute", "status": "pending", "rawInput": {"command": "echo hi"}});
        let s = r.update(&call);
        assert!(matches!(&s[..], [Step::Call { name, .. }] if name == "echo hi"));
        let done = json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "completed", "rawOutput": {"content": [{"type": "text", "text": "hi\n"}]}});
        let s = r.update(&done);
        assert!(matches!(&s[..], [Step::Result { text, error: false, .. }] if text == "hi\n"));
        // Shown once, however many updates come.
        assert!(r.update(&done).is_empty());
        r.update(&json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "\n\nhi"}}));
        assert_eq!(r.answer(), "hi");
    }

    #[test]
    fn a_model_is_picked_only_when_offered() {
        let options =
            json!([{"id": "model", "category": "model", "options": [{"value": "go/flash"}]}]);
        assert_eq!(
            pick(&options, "model", "go/flash"),
            Some(("model".into(), "go/flash".into()))
        );
        assert_eq!(pick(&options, "model", "opus"), None);
    }

    #[test]
    fn permission_picks_the_option_of_the_kind_asked() {
        let options = json!([
            {"optionId": "a", "kind": "allow_always"},
            {"optionId": "o", "kind": "allow_once"},
            {"optionId": "r", "kind": "reject_once"},
        ]);
        assert_eq!(permission(&options, true)["outcome"]["optionId"], "o");
        assert_eq!(permission(&options, false)["outcome"]["optionId"], "r");
        assert_eq!(
            permission(&json!([]), true)["outcome"]["outcome"],
            "cancelled"
        );
    }
}
