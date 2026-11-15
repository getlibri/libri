//! OpenCode through its own server (`opencode serve`, HTTP and SSE), the
//! interface its web and desktop apps use. One server lives as long as the
//! libri, started on first use. A chat that keeps a session goes on with
//! its own there, for as long as its model and permissions stay as they
//! were made, which OpenCode fixes at a session's start; OpenCode compacts
//! it itself. Any other turn is a session deleted after the turn.
//!
//! Its session events map onto the door's steps: text and reasoning
//! deltas to words and thinking, tool calls and their outcomes to calls
//! and results, permission requests to asks.

use crate::model::{Ask, Attachment, Harness, Run, Step};
use crate::servers::{self, Scope};
use crate::sessions;
use crate::task;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const START: Duration = Duration::from_secs(60);
/// This harness's name in the session records.
const HARNESS: &str = "opencode";
const CANCEL: Duration = Duration::from_secs(10);

pub struct OpenCode {
    pub command: String,
    pub cwd: PathBuf,
}

/// The one `opencode serve` of this Libri, which its turns and its model
/// list share: on a fixed port (LIBRI_OPENCODE_PORT) a second would not
/// start.
static SERVER: Mutex<Option<Server>> = Mutex::new(None);

/// A running `opencode serve` and how to reach it.
struct Server {
    child: Child,
    url: String,
    password: String,
    /// The tools it was started with: it takes them only at its start.
    tools: String,
}

/// Turns under way on the server: a change of tools waits for none.
static TURNS: AtomicUsize = AtomicUsize::new(0);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl OpenCode {
    pub fn from_env() -> Result<Self, String> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let cwd = var("LIBRI_WORKDIR")
            .or_else(|| var("STATE_DIRECTORY"))
            .map(PathBuf::from)
            .map_or_else(std::env::current_dir, Ok)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            command: var("LIBRI_OPENCODE").unwrap_or_else(|| "opencode".into()),
            cwd,
        })
    }

    /// The server's address and password, starting it if none runs.
    fn reach(&self) -> Result<(String, String), String> {
        // The person's scope: a turn's session denies what its own lacks.
        let tools = servers::opencode(&servers::list(Scope::Shared, None)?).to_string();
        let mut server = SERVER.lock().unwrap();
        if let Some(s) = server.as_mut()
            && s.child.try_wait().ok().flatten().is_none()
            && (s.tools == tools || TURNS.load(Ordering::Relaxed) > 0)
        {
            return Ok((s.url.clone(), s.password.clone()));
        }
        server.take();
        let mut cmd = Command::new(&self.command);
        // Port 0 where nothing fences the binds; a unit with SocketBindDeny
        // names the one port it allows, as bind rules cannot allow port 0.
        let port = std::env::var("LIBRI_OPENCODE_PORT").unwrap_or_else(|_| "0".into());
        crate::cgroup::harness(&mut cmd);
        cmd.args(["serve", "--hostname", "127.0.0.1", "--port", &port])
            .current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        // No Drop runs when Libri is stopped: systemd ends the server
        // with the rest of the unit's cgroup.
        // Libri's tools, on top of the person's own config.
        cmd.env("OPENCODE_CONFIG_CONTENT", format!(r#"{{"mcp":{tools}}}"#));
        let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", self.command))?;
        // It names its address and a password on its first lines.
        let (send, lines) = channel();
        let out = child.stdout.take().unwrap();
        thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        let (mut url, mut password) = (None, None);
        let end = Instant::now() + START;
        while url.is_none() || password.is_none() {
            let line = lines
                .recv_timeout(end.saturating_duration_since(Instant::now()))
                .map_err(|_| "opencode serve gave no address")?;
            if let Some(u) = line.strip_prefix("server listening on ") {
                url = Some(u.trim().to_owned());
            } else if let Some(p) = line.strip_prefix("server password ") {
                password = Some(p.trim().to_owned());
            }
        }
        let s = Server {
            child,
            url: url.unwrap(),
            password: password.unwrap(),
            tools,
        };
        let reached = (s.url.clone(), s.password.clone());
        *server = Some(s);
        Ok(reached)
    }

    /// OpenCode's own model list, `provider/model` a line, from the shared
    /// server.
    pub fn models(&self) -> Result<String, String> {
        let (url, password) = self.reach()?;
        let out = Command::new(&self.command)
            .args(["models", "--server", &url])
            .env("OPENCODE_SERVER_PASSWORD", password)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("{}: {e}", self.command))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let line = err
                .lines()
                .find(|l| l.contains("Error"))
                .or(err.lines().last());
            return Err(format!("opencode models: {}", line.unwrap_or("failed")));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// The Basic credentials for a server's password.
fn basic(password: &str) -> String {
    format!("Basic {}", B64.encode(format!("opencode:{password}")))
}

/// A plain HTTP client for one server.
struct Api {
    url: String,
    auth: String,
    agent: ureq::Agent,
}

impl Api {
    fn send(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
        let url = format!("{}{path}", self.url);
        let r = match (method, body) {
            ("POST", b) => self
                .agent
                .post(&url)
                .header("authorization", &self.auth)
                .send_json(b.unwrap_or(json!({}))),
            ("DELETE", _) => self
                .agent
                .delete(&url)
                .header("authorization", &self.auth)
                .call(),
            _ => self
                .agent
                .get(&url)
                .header("authorization", &self.auth)
                .call(),
        };
        let mut r = r.map_err(|e| format!("opencode {path}: {e}"))?;
        Ok(r.body_mut().read_json::<Value>().unwrap_or(Value::Null))
    }
}

impl Harness for OpenCode {
    fn answer(
        &self,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        run: Run,
    ) -> Result<String, String> {
        let (url, password) = self.reach()?;
        let auth = basic(&password);
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        let api = Api { url, auth, agent };
        let model = run.model_for("opencode");
        // Libri keeps its turns itself: this tells hippo's watcher, which
        // may be following this database, to leave the session alone.
        let mut body = json!({"metadata": {"libri": true}});
        // OpenCode's defaults allow nearly everything without asking, so a
        // turn that must not act gets a session that may not.
        // The last rule that matches wins, so what is denied comes last.
        let mut rules = Vec::new();
        if run.access("opencode").mode == "ask" {
            rules.push(json!({"action": "*", "resource": "*", "effect": "ask"}));
            // Libri's tools run unasked, as on the other harnesses; what
            // acts on the world waits for the person through Libri's cards.
            rules.push(json!({"action": "libri_*", "resource": "*", "effect": "allow"}));
            rules.push(json!({"action": "wiki_*", "resource": "*", "effect": "allow"}));
        }
        match Scope::of(&run) {
            // Nothing but the web: none of OpenCode's own tools or the
            // person's servers, which reach what the memory-off turn may not.
            Some(scope @ Scope::Memoryless) => {
                rules = vec![json!({"action": "*", "resource": "*", "effect": "deny"})];
                rules.extend(scope.only().unwrap_or_default().iter().map(
                    |t| json!({"action": format!("libri_{t}"), "resource": "*", "effect": "allow"}),
                ));
            }
            Some(scope @ (Scope::Person | Scope::Agent | Scope::Scratchpad | Scope::Apart)) => {
                rules.extend(scope.withheld().iter().map(
                    |t| json!({"action": format!("libri_{t}"), "resource": "*", "effect": "deny"}),
                ));
                // The one server reads the main hippo and the main memory's
                // workers, so a named memory's chat and workers get none of
                // what reads them here. Removed when the server is per
                // session, as Claude Code's and omp's are.
                if run.memory_dir.is_some() {
                    rules.extend(servers::MEMORY_BOUND.iter().map(|t| {
                        json!({"action": format!("libri_{t}"), "resource": "*", "effect": "deny"})
                    }));
                }
                // The one server's wiki writes: a session that may only read
                // the notes, or not at all, is held to that here.
                let state = servers::state().unwrap_or_default();
                let denied = match servers::wiki_reach(scope, run.sub.as_deref(), &state) {
                    servers::WikiReach::Write => None,
                    servers::WikiReach::Read => Some("wiki_write"),
                    servers::WikiReach::Nothing => Some("wiki_*"),
                };
                rules.extend(
                    denied.map(|a| json!({"action": a, "resource": "*", "effect": "deny"})),
                );
            }
            _ => rules = vec![json!({"action": "*", "resource": "*", "effect": "deny"})],
        }
        if !rules.is_empty() {
            body["permissions"] = rules.into();
        }
        if let Some((provider, id)) = model.as_deref().and_then(|m| m.split_once('/')) {
            body["model"] = json!({"providerID": provider, "id": id});
        }
        let kept = servers::state()
            .zip(run.session.clone())
            .filter(|_| !run.plain && !run.ephemeral);
        // What the session was made with, which it keeps for good.
        let made = body.to_string();
        let ours = kept.as_ref().and_then(|(state, chat)| {
            let id = sessions::get(state, chat, HARNESS)?;
            let same = sessions::made(state, chat, HARNESS).as_deref() == Some(made.as_str());
            (same && sessions::current(state, chat, HARNESS)).then_some(id)
        });
        // Still there: a session can be deleted in OpenCode's own app.
        let ours = ours.filter(|id| api.send("GET", &format!("/api/session/{id}"), None).is_ok());
        let holds_thread = ours.is_some();
        let id = match ours {
            Some(id) => id,
            None => {
                let session = api.send("POST", "/api/session", Some(body))?;
                let id = session["data"]["id"]
                    .as_str()
                    .ok_or("opencode gave no session id")?
                    .to_owned();
                if let Some((state, chat)) = &kept {
                    sessions::put(state, chat, HARNESS, &id)?;
                    sessions::put_made(state, chat, HARNESS, &made)?;
                }
                id
            }
        };
        let memory = if holds_thread { "" } else { memory };
        let system = if holds_thread { "" } else { system };
        if let Some(s) = SERVER.lock().unwrap().as_ref() {
            run.runs_in(s.child.id());
        }
        // Its tool server is everyone's, so the session says whose call is
        // whose: OpenCode puts its ID on every one.
        let state = servers::state().ok_or("STATE_DIRECTORY is not set")?;
        task::session_begun(&state, &id, run.sub.as_deref().unwrap_or_default())?;
        TURNS.fetch_add(1, Ordering::Relaxed);
        let turn = self.turn(&api, &id, system, memory, prompt, files, &run);
        task::session_ended(&state, &id);
        if let Some(s) = &run.steer {
            s.close();
        }
        TURNS.fetch_sub(1, Ordering::Relaxed);
        if kept.is_none() {
            let _ = api.send("DELETE", &format!("/api/session/{id}"), None);
        }
        turn
    }

    fn tools(&self) -> bool {
        true
    }
}

impl OpenCode {
    #[allow(clippy::too_many_arguments)]
    fn turn(
        &self,
        api: &Api,
        session: &str,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        run: &Run,
    ) -> Result<String, String> {
        // Its events, read from their own connection.
        let (send, events) = channel::<Value>();
        let stream = format!("{}/api/event", api.url);
        let auth = api.auth.clone();
        thread::spawn(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(None)
                .build()
                .into();
            let Ok(mut r) = agent.get(&stream).header("authorization", &auth).call() else {
                return;
            };
            for line in BufReader::new(r.body_mut().as_reader()).lines() {
                let Ok(line) = line else { break };
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                if let Ok(e) = serde_json::from_str::<Value>(data.trim())
                    && send.send(e).is_err()
                {
                    break;
                }
            }
        });
        // A session gone on with has had both.
        let mut text = [system, memory]
            .iter()
            .filter(|t| !t.is_empty())
            .map(|t| format!("{t}\n\n"))
            .collect::<String>();
        if !files.is_empty() {
            text.push_str("(Files came with this message that this harness can't read.)\n\n");
        }
        text.push_str(prompt);
        // The stream must be open before the turn starts, or its first
        // events go unseen.
        thread::sleep(Duration::from_millis(300));
        api.send(
            "POST",
            &format!("/api/session/{session}/prompt"),
            Some(json!({ "text": text })),
        )?;
        if let Some(s) = &run.steer {
            let path = format!("/api/session/{session}/prompt");
            let (url, auth) = (api.url.clone(), api.auth.clone());
            s.take_words(Box::new(move |text, files| {
                // Files go through a turn of their own, as `turn` reads none.
                let api = Api {
                    url: url.clone(),
                    auth: auth.clone(),
                    agent: ureq::Agent::new_with_defaults(),
                };
                let body = json!({"text": text, "delivery": "steer"});
                files.is_empty() && api.send("POST", &path, Some(body)).is_ok()
            }));
        }
        let mut r = Reading::default();
        let mut cancelled: Option<Instant> = None;
        loop {
            let stopped = run.stop.as_ref().is_some_and(|f| f.load(Ordering::Relaxed));
            if stopped && cancelled.is_none() {
                api.send("POST", &format!("/api/session/{session}/interrupt"), None)?;
                cancelled = Some(Instant::now());
                r.withdraw(run);
            }
            if cancelled.is_some_and(|c| c.elapsed() > CANCEL) {
                return Err("stopped".into());
            }
            match events.recv_timeout(Duration::from_millis(100)) {
                Ok(e)
                    if e["data"]["sessionID"] == session
                        || e["data"]["info"]["metadata"]["sessionID"] == session =>
                {
                    run.heard();
                    if let Some(end) = r.take(api, session, &e, run) {
                        r.withdraw(run);
                        return match end {
                            Err(_) if cancelled.is_some() => Err("stopped".into()),
                            end => end,
                        };
                    }
                }
                Ok(_) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    r.withdraw(run);
                    return Err("opencode's event stream ended mid-turn".into());
                }
            }
        }
    }
}

#[derive(Default)]
struct Reading {
    words: String,
    /// Each call's tool name, from when its input began.
    names: HashMap<String, String>,
    open: Arc<Mutex<Vec<String>>>,
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

    fn take(
        &mut self,
        api: &Api,
        session: &str,
        e: &Value,
        run: &Run,
    ) -> Option<Result<String, String>> {
        let show = |step| {
            if let Some(f) = &run.steps {
                f(step)
            }
        };
        let d = &e["data"];
        let s = |k: &str| d[k].as_str().unwrap_or_default().to_owned();
        match e["type"].as_str().unwrap_or_default() {
            "session.text.delta" => {
                let t = s("delta");
                self.words.push_str(&t);
                show(Step::Text { text: t });
            }
            "session.reasoning.delta" => show(Step::Thinking { text: s("delta") }),
            "session.tool.input.started" => {
                self.names.insert(s("id"), s("name"));
            }
            "session.tool.called" => {
                self.words.clear();
                let id = s("id");
                let name = self
                    .names
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| "tool".into());
                show(Step::Call {
                    id,
                    name,
                    input: d["input"].clone(),
                });
            }
            kind @ ("session.tool.success" | "session.tool.failed" | "session.tool.error") => {
                let text: Vec<&str> = d["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c["text"].as_str())
                    .collect();
                let text = if text.is_empty() {
                    d["error"]["message"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned()
                } else {
                    text.join("\n")
                };
                show(Step::Result {
                    id: s("id"),
                    text,
                    error: kind != "session.tool.success",
                });
            }
            "session.usage.updated" => {
                let t = &d["tokens"];
                let n = |v: &Value| v.as_u64().unwrap_or(0);
                show(Step::Usage {
                    input: n(&t["input"]) + n(&t["cache"]["read"]) + n(&t["cache"]["write"]),
                    cache_read: n(&t["cache"]["read"]),
                    cache_write: n(&t["cache"]["write"]),
                    output: n(&t["output"]) + n(&t["reasoning"]),
                    model: None,
                });
            }
            "permission.asked" => {
                let rid = s("id");
                let ask = Ask {
                    id: format!("opencode-{rid}"),
                    tool: s("action"),
                    input: json!({"resources": d["resources"]}),
                    about: None,
                    always: false,
                };
                let (asker, open) = (run.ask.clone(), self.open.clone());
                let approve = run.access("opencode").mode == "approve";
                let path = format!("/api/session/{session}/permission/{rid}/reply");
                let (url, auth) = (api.url.clone(), api.auth.clone());
                thread::spawn(move || {
                    let key = ask.id.clone();
                    open.lock().unwrap().push(key.clone());
                    // OpenCode's --auto, which lives in its client: every
                    // ask accepted, and what is denied never asks.
                    let yes = approve || asker.and_then(|a| a.put(ask)).is_some();
                    open.lock().unwrap().retain(|k| *k != key);
                    let api = Api {
                        url,
                        auth,
                        agent: ureq::Agent::new_with_defaults(),
                    };
                    let decision = if yes { "once" } else { "reject" };
                    if let Err(e) = api.send("POST", &path, Some(json!({ "decision": decision }))) {
                        eprintln!("libri: {e}");
                    }
                });
            }
            "session.execution.succeeded" => {
                let text = self.words.trim().to_owned();
                if run.sub.is_some() {
                    show(Step::Answer { text: text.clone() });
                }
                return Some(Ok(text));
            }
            "session.execution.interrupted" => return Some(Err("stopped".into())),
            "session.execution.failed" => {
                return Some(Err(format!(
                    "opencode: {}",
                    d["error"]["message"].as_str().unwrap_or("the turn failed")
                )));
            }
            _ => {}
        }
        None
    }
}
