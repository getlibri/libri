//! omp (oh-my-pi) through its own RPC mode, `omp --mode rpc-ui`: newline
//! JSON over stdio, not JSON-RPC. It is the path Paseo takes, and richer
//! than `omp acp`: thinking, each tool's intent and partial output, usage
//! per message. A chat that keeps a session (`Run::session`) goes on with
//! its own: omp keeps it in a folder of the chat's (`--session-dir`) and
//! `--continue` takes it up, compacting it itself. Any other call is one
//! ephemeral session (`--no-session`). Pi, which
//! omp forked, speaks the same protocol bar how a turn ends (`Dialect`);
//! `pi.rs` starts it and this reads it.
//!
//! rpc-ui puts omp's tool approvals to us as `select` dialogs; they become
//! the door's asks. omp's own default approval mode asks nothing, so the
//! mode is always set here, from the person's access pick.
//! Libri's MCP servers reach omp through the project config it reads
//! from its working directory (`.omp/mcp.json`). One file serves every
//! turn, so each turn's scope goes in omp's environment, which its servers
//! inherit.

use crate::model::{Ask, Attachment, Harness, Run, Step};
use crate::servers::{self, Scope};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Which of the two RPC modes is spoken: omp's `rpc-ui` or Pi's `rpc`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    /// omp ends a turn with a `prompt_result` for the prompt's id.
    Omp,
    /// Pi ends one with `agent_settled`; a failure is on the last
    /// assistant message.
    Pi,
}

impl Dialect {
    fn name(self) -> &'static str {
        match self {
            Self::Omp => "omp",
            Self::Pi => "pi",
        }
    }
}

/// How long a stopped turn may take to end before it is made to.
const CANCEL: Duration = Duration::from_secs(10);
/// The id our one prompt goes by; its `prompt_result` ends the turn.
const PROMPT: &str = "turn";
/// The approval dialog's choices, as omp words them.
const APPROVE: &str = "Approve";
const DENY: &str = "Deny";

pub struct Omp {
    pub command: String,
    /// Where omp works, and where its project config goes.
    pub cwd: PathBuf,
    /// The thinking level (LIBRI_EFFORT), unless the person picked one.
    pub effort: String,
}

impl Omp {
    pub fn from_env() -> Result<Self, String> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let cwd = var("LIBRI_WORKDIR")
            .or_else(|| var("STATE_DIRECTORY"))
            .map(PathBuf::from)
            .map_or_else(std::env::current_dir, Ok)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            command: var("LIBRI_OMP").unwrap_or_else(|| "omp".into()),
            cwd,
            // omp's own defaultThinkingLevel, which it leaves unapplied
            // under --model (omp 18.12): given here, the model thinks.
            effort: var("LIBRI_EFFORT").unwrap_or_else(|| "high".into()),
        })
    }

    /// Writes the project config omp reads from `cwd`: Libri's MCP
    /// servers, and an overlay that keeps them out of a turn that may not
    /// act. Returns the overlay's path and the servers' names.
    fn configure(&self) -> Result<(PathBuf, Vec<String>), String> {
        let dir = self.cwd.join(".omp");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let listed = servers::list(Scope::Person, None)?;
        write_if_changed(&dir.join("mcp.json"), &servers::omp(&listed))?;
        let withheld = dir.join("withheld.yml");
        write_if_changed(&withheld, "mcp:\n  enableProjectConfig: false\n")?;
        Ok((withheld, listed.keys().cloned().collect()))
    }
}

pub fn write_if_changed(path: &Path, text: &str) -> Result<(), String> {
    if std::fs::read_to_string(path).is_ok_and(|t| t == text) {
        return Ok(());
    }
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

impl Harness for Omp {
    fn answer(
        &self,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        run: Run,
    ) -> Result<String, String> {
        let (withheld, servers) = self.configure()?;
        let effort = run.effort().unwrap_or_else(|| self.effort.clone());
        let mode = run.access("omp").mode;
        let mut cmd = Command::new(&self.command);
        cmd.args(["--mode", "rpc-ui"]);
        let holds_thread = session_args(&mut cmd, &run, "omp")?;
        let memory = if holds_thread { "" } else { memory };
        cmd.arg(format!("--approval-mode={mode}"))
            .arg(format!("--thinking={effort}"))
            .current_dir(&self.cwd)
            // Wait for the MCP servers before the first model call, not
            // omp's 250 ms, so their tools are there from the start.
            .env("OMP_MCP_STARTUP_TIMEOUT_MS", "0");
        run.temp(&mut cmd);
        if run.plain {
            cmd.args(["--no-tools", "--no-skills", "--no-rules"])
                .arg(format!("--system-prompt={system}"));
        } else {
            // Beside omp's own instructions, not in their place.
            cmd.arg(format!("--append-system-prompt={system}"));
        }
        if let Some(m) = run.model_for("omp") {
            cmd.arg(format!("--model={m}"));
        }
        match Scope::of(&run) {
            Some(scope) => {
                cmd.env(Scope::VAR, scope.name());
                if let Some(worker) = &run.sub {
                    cmd.env(Scope::WORKER, worker);
                }
                // omp's tool server inherits omp's environment.
                cmd.envs(crate::servers::memory_env(&run));
            }
            None => {
                cmd.arg(format!("--config={}", withheld.display()));
            }
        }
        let mut p = Process::spawn(cmd, Dialect::Omp)?;
        let turn = turn(&p, memory, prompt, files, &run, &servers);
        if let Some(s) = &run.steer {
            s.close();
        }
        p.end();
        turn
    }

    fn tools(&self) -> bool {
        true
    }
}

/// Points `cmd` at the chat's own session on `harness`, made if new;
/// `--no-session` for a call that keeps none. Whether the session already
/// holds the thread. Neither omp nor Pi is given another's session to fork,
/// so a worker's first session starts new, with its log.
pub fn session_args(cmd: &mut Command, run: &Run, harness: &str) -> Result<bool, String> {
    use crate::sessions;
    let kept = sessions::state()
        .zip(run.session.as_deref())
        .filter(|_| !run.plain && !run.ephemeral);
    let Some((state, chat, dir)) = kept.and_then(|(state, chat)| {
        let dir = sessions::folder(&state, chat, harness)?;
        Some((state, chat, dir))
    }) else {
        cmd.arg("--no-session");
        return Ok(false);
    };
    // Another harness ran the chat since: this one's session misses those
    // turns, and the chat's newest come with a new one. Libri's own record
    // keeps the chat; these files are only the harness's copy of it.
    if !sessions::current(&state, chat, harness) && dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    sessions::ran(&state, chat, harness)?;
    // Two words, as Pi takes no `--flag=value`; omp takes both.
    cmd.arg("--session-dir").arg(&dir);
    let holds = crate::sessions::holds_session(&dir);
    if holds {
        cmd.arg("--continue");
    }
    Ok(holds)
}

/// How long omp's device code for ChatGPT stays good.
const DEVICE: Duration = Duration::from_secs(15 * 60);

/// Starts a provider's own device sign-in through omp, with nothing copied: answers the page to open and the code to type
/// there. A thread waits for omp to say how it went and tells `ended`:
/// `None` signed in, else why not.
pub fn login(
    provider: &str,
    ended: impl FnOnce(Option<String>) + Send + 'static,
) -> Result<(String, String), String> {
    if !matches!(provider, "openai-codex-device" | "kimi-code" | "kilo") {
        return Err("This provider must sign in through omp in a terminal.".into());
    }
    const LOGIN: &str = "login";
    let omp = Omp::from_env()?;
    let mut cmd = Command::new(&omp.command);
    // omp 18.16 refuses RPC startup without a model on a fresh profile.
    // No model request is made during login. Remove this bootstrap selector
    // when omp permits its login command before model selection.
    cmd.args([
        "--mode",
        "rpc-ui",
        "--no-session",
        "--model",
        "openai-codex/gpt-6.1-sol",
        "--no-extensions",
        "--no-skills",
        "--no-rules",
        "--no-context-files",
        "--no-lsp",
        "--no-tools",
    ])
    .current_dir(&omp.cwd);
    if let Ok(profile) = std::env::var("LIBRI_OMP_PROFILE") {
        cmd.args(["--profile", &profile]);
    }
    let mut p = Process::spawn(cmd, Dialect::Omp)?;
    let said = |f: &Value| {
        f["error"]
            .as_str()
            .unwrap_or("omp ended the sign-in")
            .to_owned()
    };
    let given = Instant::now() + Duration::from_secs(60);
    let found = p
        .send(&json!({"id": LOGIN, "type": "login", "providerId": provider}))
        .and_then(|()| {
            loop {
                let left = given.saturating_duration_since(Instant::now());
                match p.incoming.recv_timeout(left) {
                    Ok(f) if f["type"] == "extension_ui_request" && f["method"] == "open_url" => {
                        let code = f["instructions"]
                            .as_str()
                            .and_then(|i| i.rsplit_once("code:"))
                            .map(|(_, c)| c.trim().to_owned())
                            .filter(|c| !c.is_empty());
                        break match (f["url"].as_str(), code) {
                            (Some(url), Some(code)) => Ok((url.to_owned(), code)),
                            _ => Err("omp gave no device code".into()),
                        };
                    }
                    Ok(f) if f["type"] == "response" && f["id"] == LOGIN => break Err(said(&f)),
                    Ok(_) => {}
                    Err(_) => break Err("omp gave no device code".into()),
                }
            }
        });
    let found = match found {
        Ok(f) => f,
        Err(e) => {
            p.end();
            return Err(e);
        }
    };
    thread::spawn(move || {
        let end = Instant::now() + DEVICE;
        let how = loop {
            match p.incoming.recv_timeout(Duration::from_secs(1)) {
                Ok(f) if f["type"] == "response" && f["id"] == LOGIN => {
                    break (f["success"] != true).then(|| said(&f));
                }
                Err(RecvTimeoutError::Disconnected) => break Some("omp exited".into()),
                _ if Instant::now() > end => break Some("The code ran out.".into()),
                _ => {}
            }
        };
        p.end();
        ended(how);
    });
    Ok(found)
}

/// omp or Pi running, its stdout lines parsed on `incoming`.
pub struct Process {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    incoming: Receiver<Value>,
    dialect: Dialect,
}

impl Process {
    /// Starts `cmd` in a process group of its own, so a stop ends all it
    /// started too.
    pub fn spawn(mut cmd: Command, dialect: Dialect) -> Result<Self, String> {
        let name = dialect.name();
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        crate::cgroup::harness(&mut cmd);
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("{name}: {e}"))?;
        let stdout = child.stdout.take().unwrap();
        let (tx, incoming) = channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                match serde_json::from_str::<Value>(&line) {
                    Ok(v) => {
                        if tx.send(v).is_err() {
                            break;
                        }
                    }
                    Err(_) if line.trim().is_empty() => {}
                    Err(e) => eprintln!("libri: {name} sent a line that isn't JSON: {e}"),
                }
            }
        });
        Ok(Self {
            stdin: Arc::new(Mutex::new(child.stdin.take().unwrap())),
            child,
            incoming,
            dialect,
        })
    }

    fn send(&self, frame: &Value) -> Result<(), String> {
        send(&self.stdin, frame)
    }

    /// Sends `frame` under `id` and waits up to `wait` for its response's
    /// data, passing over the events between.
    pub fn request(&self, id: &str, mut frame: Value, wait: Duration) -> Result<Value, String> {
        frame["id"] = id.into();
        self.send(&frame)?;
        let given = Instant::now() + wait;
        loop {
            let left = given.saturating_duration_since(Instant::now());
            let got = self
                .incoming
                .recv_timeout(left)
                .map_err(|_| format!("{} did not answer {}", self.dialect.name(), frame["type"]))?;
            if got["type"] == "response" && got["id"] == id {
                return match got["success"].as_bool() {
                    Some(true) => Ok(got["data"].clone()),
                    _ => Err(format!(
                        "{}: {}",
                        self.dialect.name(),
                        got["error"].as_str().unwrap_or("refused")
                    )),
                };
            }
        }
    }

    /// Ends the harness and all it started: asked first, then made to.
    pub fn end(&mut self) {
        let group = self.child.id() as i32;
        // SAFETY: killpg only sends a signal, to the group this child leads.
        unsafe { libc::killpg(group, libc::SIGTERM) };
        let given = Instant::now() + Duration::from_secs(3);
        while self.child.try_wait().ok().flatten().is_none() && Instant::now() < given {
            thread::sleep(Duration::from_millis(50));
        }
        // SAFETY: as above.
        unsafe { libc::killpg(group, libc::SIGKILL) };
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn send(stdin: &Mutex<ChildStdin>, frame: &Value) -> Result<(), String> {
    let mut stdin = stdin.lock().unwrap();
    writeln!(stdin, "{frame}")
        .and_then(|()| stdin.flush())
        .map_err(|e| format!("the harness's input: {e}"))
}

/// One turn on `p`. `servers` are the MCP servers omp was given, whose
/// tools it names its own way (Pi names them as Libri does).
pub fn turn(
    p: &Process,
    memory: &str,
    prompt: &str,
    files: &[Attachment],
    run: &Run,
    servers: &[String],
) -> Result<String, String> {
    // Deltas only: the full snapshot on every update grows with the reply.
    // Pi's wire carries deltas alone already. The ask tool's questions come
    // whole, as one dialog, rather than one select per question.
    if p.dialect == Dialect::Omp {
        p.send(&json!({"type": "set_event_filter", "events": null, "messageUpdates": "delta"}))?;
        p.send(&json!({"type": "set_ask_dialog", "enabled": true}))?;
        // Each subagent's own events too, for its own thread in Work.
        p.send(&json!({"type": "set_subagent_subscription", "level": "events"}))?;
    }
    let mut message = memory.to_owned();
    let images = images_of(files);
    if files.iter().any(|f| matches!(f, Attachment::Pdf(_))) {
        message.push_str("\n\n(A file came with this message that this harness can't read.)");
    }
    message.push_str("\n\n");
    message.push_str(prompt);
    p.send(&json!({"id": PROMPT, "type": "prompt", "message": message, "images": images}))?;
    if let Some(s) = &run.steer {
        let stdin = p.stdin.clone();
        s.take_words(Box::new(move |text, files| {
            let frame = json!({"type": "steer", "message": text, "images": images_of(files)});
            send(&stdin, &frame).is_ok()
        }));
    }
    run.runs_in(p.child.id());
    let mut state = Reading {
        servers: servers
            .iter()
            .map(|s| (format!("mcp__{}_", omp_part(s)), s.clone()))
            .collect(),
        ..Reading::default()
    };
    let mut cancelled: Option<Instant> = None;
    let mut measured = Instant::now();
    let ended = loop {
        if p.dialect == Dialect::Omp && measured.elapsed() >= Duration::from_secs(1) {
            p.send(&json!({"id": "libri-metrics", "type": "get_state"}))?;
            measured = Instant::now();
        }
        let stopped = run.stop.as_ref().is_some_and(|f| f.load(Ordering::Relaxed));
        if stopped && cancelled.is_none() {
            p.send(&json!({"type": "abort"}))?;
            cancelled = Some(Instant::now());
            state.withdraw(run);
        }
        if cancelled.is_some_and(|c| c.elapsed() > CANCEL) {
            break Err("stopped".into());
        }
        let got = p.incoming.recv_timeout(Duration::from_millis(100));
        if got.as_ref().is_ok_and(|frame| {
            !matches!(
                frame["id"].as_str(),
                Some("libri-metrics" | "libri-final-metrics")
            )
        }) {
            run.heard();
        }
        match got {
            Ok(frame) if p.dialect == Dialect::Pi && frame["type"] == "agent_settled" => {
                break match &state.failure {
                    _ if cancelled.is_some() || frame["aborted"] == true => Err("stopped".into()),
                    Some(failure) => Err(failure.clone()),
                    None => Ok(state.answer()),
                };
            }
            Ok(frame) if frame["type"] == "prompt_result" && frame["id"] == PROMPT => {
                break match frame["status"].as_str() {
                    _ if cancelled.is_some() => Err("stopped".into()),
                    Some("completed") => Ok(state.answer()),
                    Some("aborted") => Err("stopped".into()),
                    _ => Err(frame["error"]["message"]
                        .as_str()
                        .unwrap_or("omp ended the turn with an error")
                        .to_owned()),
                };
            }
            // A failure before the prompt is admitted: no prompt_result
            // need follow.
            Ok(frame)
                if frame["type"] == "response"
                    && frame["id"] == PROMPT
                    && frame["success"] == false =>
            {
                break Err(format!(
                    "{}: {}",
                    p.dialect.name(),
                    frame["error"].as_str().unwrap_or("prompt refused")
                ));
            }
            // Pi took the prompt as a command of its own: no run follows.
            Ok(frame)
                if frame["type"] == "response"
                    && frame["id"] == PROMPT
                    && frame["data"]["disposition"] == "handled" =>
            {
                break Ok(state.answer());
            }
            Ok(frame) => state.take(p, &frame, run),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break Err("the harness exited mid-turn".into()),
        }
    };
    if p.dialect == Dialect::Omp && ended.is_ok() {
        p.send(&json!({"id": "libri-final-metrics", "type": "get_state"}))?;
        let deadline = Instant::now() + Duration::from_secs(1);
        while let Ok(frame) = p
            .incoming
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            state.take(p, &frame, run);
            if frame["id"] == "libri-final-metrics" || Instant::now() >= deadline {
                break;
            }
        }
    }
    state.withdraw(run);
    // A subagent still at work when the turn ends ends with it.
    if let Some(show) = &run.steps {
        state.subs.drain().for_each(|(_, (id, _))| {
            show(Step::Done {
                id,
                report: "Stopped with the turn.".into(),
            })
        });
    }
    // The seat's pictures go to the person after the answer, as
    // make_image's do; a worker's or a Temp chat's stay in its tools.
    if run.sub.is_none() && !run.ephemeral && !run.plain {
        keep_pictures(&state.pictures);
    }
    // omp names the model on each message it answers with; it reports no
    // thinking level back.
    if let (Some(show), Some(model)) = (&run.steps, &state.model) {
        show(Step::Runs {
            model: model.clone(),
            effort: None,
        });
    }
    if let (Some(show), Some(u)) = (&run.steps, state.usage) {
        show(Step::Usage {
            input: u.input,
            cache_read: u.cache_read,
            cache_write: u.cache_write,
            output: u.output,
            model: state.model.clone(),
        });
    }
    if let (Ok(text), true, Some(show)) = (&ended, run.sub.is_some(), &run.steps) {
        show(Step::Answer { text: text.clone() });
    }
    ended
}

/// The photos among `files`, as omp's messages carry them.
fn images_of(files: &[Attachment]) -> Vec<Value> {
    files
        .iter()
        .filter_map(|f| match f {
            Attachment::Image { mime, data } => Some(json!({
                "type": "image",
                "mimeType": mime,
                "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data),
            })),
            Attachment::Pdf(_) => None,
        })
        .collect()
}

/// Tokens a turn used: `input` counts every token sent, the cache's two
/// shares among them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Tokens {
    input: u64,
    cache_read: u64,
    cache_write: u64,
    output: u64,
}

/// How often a running call's output is shown again: omp sends the whole
/// output so far each time it grows, which a busy command does often.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// What the turn has shown so far.
#[derive(Default)]
struct Reading {
    /// The words since the last tool call: the answer, if the turn ends.
    words: String,
    /// The last call started, by name and input: the one an approval is for.
    calling: Option<(String, Value)>,
    /// Tokens in and out, summed over the turn's model calls.
    usage: Option<Tokens>,
    /// The model the last call named.
    model: Option<String>,
    /// The MCP servers by the prefix omp gives their tools'
    /// names (`mcp__libri_`), for Libri's (`mcp__libri__`).
    servers: Vec<(String, String)>,
    /// When each running call's output was last shown.
    shown: HashMap<String, Instant>,
    /// Why the last model call failed, if it did: Pi's word on a turn that
    /// ends badly, since a retry that succeeds clears it.
    failure: Option<String>,
    /// The asks waiting on the person, by omp's dialog id.
    open: Arc<Mutex<Vec<String>>>,
    /// The subagents at work, by omp's id: Libri's id for each (its
    /// `task` call's and its own, as omp names one only within a call)
    /// and what it has shown.
    subs: HashMap<String, (String, Reading)>,
    /// What each `task` call asked for, by the call's id: the names and
    /// prompts of the subagents it starts.
    tasked: HashMap<String, Value>,
    /// The `todo` calls running, by id, with their input: each shows once
    /// its result holds the whole list.
    todos: HashMap<String, Value>,
    /// The row of the model call being tried again, and how many retries
    /// the turn has had.
    retry: Option<String>,
    retries: u32,
    /// The pictures the turn's tools gave back, by type and bytes.
    pictures: Vec<(String, Vec<u8>)>,
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
                .for_each(|id| a.withdraw(&ask_id(&id)));
        }
    }

    fn take(&mut self, p: &Process, frame: &Value, run: &Run) {
        if frame["type"] == "extension_ui_request" {
            self.dialog(p, frame, run);
            return;
        }
        for step in self.update(frame) {
            if let Some(show) = &run.steps {
                show(step);
            }
        }
    }

    /// Answers one of omp's dialogs: an approval goes to the person, in a
    /// thread of its own so the stream reads on meanwhile; any other
    /// question is declined, as the ACP path declines it.
    fn dialog(&mut self, p: &Process, frame: &Value, run: &Run) {
        let id = frame["id"].as_str().unwrap_or_default().to_owned();
        let method = frame["method"].as_str().unwrap_or_default();
        match method {
            "select" if approval(frame) => {
                let (tool, input) = self.calling.clone().unwrap_or_default();
                let ask = Ask {
                    id: ask_id(&id),
                    tool: if tool.is_empty() {
                        "a tool".into()
                    } else {
                        tool
                    },
                    input,
                    about: frame["title"].as_str().map(str::to_owned),
                    always: false,
                };
                let (stdin, asker, open) = (p.stdin.clone(), run.ask.clone(), self.open.clone());
                open.lock().unwrap().push(id.clone());
                thread::spawn(move || {
                    let yes = asker.and_then(|a| a.put(ask)).is_some();
                    open.lock().unwrap().retain(|k| *k != id);
                    let value = if yes { APPROVE } else { DENY };
                    let _ = send(
                        &stdin,
                        &json!({"type": "extension_ui_response", "id": id, "value": value}),
                    );
                });
            }
            // The ask tool's questions, all at once: the person's question
            // card, its answers back in omp's shape.
            "ask" => {
                let ask = Ask {
                    id: ask_id(&id),
                    tool: "AskUserQuestion".into(),
                    input: json!({"questions": questions(frame)}),
                    about: None,
                    always: false,
                };
                let (stdin, asker, open) = (p.stdin.clone(), run.ask.clone(), self.open.clone());
                let asked = frame.clone();
                open.lock().unwrap().push(id.clone());
                thread::spawn(move || {
                    let answered = asker.and_then(|a| a.put(ask));
                    open.lock().unwrap().retain(|k| *k != id);
                    let reply = match answered {
                        Some(a) => json!({"type": "extension_ui_response", "id": id,
                            "answers": answers(&asked, &a)}),
                        None => {
                            json!({"type": "extension_ui_response", "id": id, "cancelled": true})
                        }
                    };
                    let _ = send(&stdin, &reply);
                });
            }
            "select" | "confirm" | "input" | "editor" => {
                let _ =
                    p.send(&json!({"type": "extension_ui_response", "id": id, "cancelled": true}));
            }
            // omp took a dialog back (its timeout, or the call aborted).
            "cancel" => {
                let target = frame["targetId"].as_str().unwrap_or_default();
                let mut open = self.open.lock().unwrap();
                if open.iter().any(|k| k == target) {
                    open.retain(|k| k != target);
                    if let Some(a) = &run.ask {
                        a.withdraw(&ask_id(target));
                    }
                }
            }
            // Presentation (setWidget, notify, setStatus): nothing to show.
            _ => {}
        }
    }

    /// The steps one event shows.
    fn update(&mut self, ev: &Value) -> Vec<Step> {
        match ev["type"].as_str().unwrap_or_default() {
            "response"
                if ev["success"] == true
                    && matches!(
                        ev["id"].as_str(),
                        Some("libri-metrics" | "libri-final-metrics")
                    ) =>
            {
                let state = &ev["data"];
                let context = &state["contextUsage"];
                let rate = state["tokensPerSecond"]
                    .as_f64()
                    .filter(|n| n.is_finite() && *n > 0.0);
                vec![Step::Metrics {
                    context_tokens: context["tokens"].as_u64(),
                    context_window: context["contextWindow"].as_u64().filter(|n| *n > 0),
                    tokens_per_second: rate,
                }]
            }
            "message_update" => {
                let e = &ev["assistantMessageEvent"];
                let delta = e["delta"].as_str().unwrap_or_default().to_owned();
                match e["type"].as_str().unwrap_or_default() {
                    "text_delta" => {
                        self.words.push_str(&delta);
                        vec![Step::Text { text: delta }]
                    }
                    "thinking_delta" => vec![Step::Thinking { text: delta }],
                    _ => Vec::new(),
                }
            }
            "message_end" if ev["message"]["role"] == "assistant" => {
                let u = &ev["message"]["usage"];
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                let t = self.usage.unwrap_or_default();
                self.usage = Some(Tokens {
                    input: t.input + n("input") + n("cacheRead") + n("cacheWrite"),
                    cache_read: t.cache_read + n("cacheRead"),
                    cache_write: t.cache_write + n("cacheWrite"),
                    output: t.output + n("output"),
                });
                if let Some(m) = ev["message"]["model"].as_str() {
                    self.model = Some(m.to_owned());
                }
                let m = &ev["message"];
                self.failure = (m["stopReason"] == "error").then(|| {
                    m["errorMessage"]
                        .as_str()
                        .unwrap_or("the model call failed")
                        .to_owned()
                });
                Vec::new()
            }
            "tool_execution_start" => {
                let id = ev["toolCallId"].as_str().unwrap_or_default().to_owned();
                let (name, input) = self.presented(
                    ev["toolName"].as_str().unwrap_or("tool"),
                    &ev["args"],
                    ev["intent"].as_str(),
                );
                // Words before a call lead into it; the answer comes after.
                self.words.clear();
                self.calling = Some((name.clone(), input.clone()));
                match ev["toolName"].as_str() {
                    // Each subagent the call starts shows as an Agent call
                    // of its own when it starts; the call itself shows only
                    // if it fails before any does.
                    Some("task") => {
                        self.tasked.insert(id, ev["args"].clone());
                        return Vec::new();
                    }
                    // A to-do call's input is one change; the list it makes
                    // comes with the result.
                    Some("todo") => {
                        self.todos.insert(id, ev["args"].clone());
                        return Vec::new();
                    }
                    _ => {}
                }
                vec![Step::Call { id, name, input }]
            }
            // The call's output so far, whole: a shell's as it prints.
            "tool_execution_update" => {
                let id = ev["toolCallId"].as_str().unwrap_or_default();
                let text = output(&ev["partialResult"]);
                let due = self
                    .shown
                    .get(id)
                    .is_none_or(|at| at.elapsed() >= PROGRESS_EVERY);
                if text.is_empty() || !due {
                    return Vec::new();
                }
                self.shown.insert(id.to_owned(), Instant::now());
                vec![Step::Progress {
                    id: id.to_owned(),
                    text,
                }]
            }
            "tool_execution_end" => {
                let id = ev["toolCallId"].as_str().unwrap_or_default().to_owned();
                self.shown.remove(&id);
                let r = &ev["result"];
                let error = ev["isError"] == true;
                self.pictures.extend(pictures(r));
                if let Some(args) = self.todos.remove(&id) {
                    return todo_steps(id, &args, r, error, self);
                }
                // A task call that worked shows through its subagents' own
                // Agent calls, which may start only after it returns (run in
                // the background); one that failed shows itself.
                if ev["toolName"] == "task" {
                    if !error {
                        return Vec::new();
                    }
                    let args = self.tasked.get(&id).cloned().unwrap_or_default();
                    let (name, input) = self.presented("task", &args, None);
                    return vec![
                        Step::Call {
                            id: id.clone(),
                            name,
                            input,
                        },
                        Step::Result {
                            id,
                            text: output(r),
                            error,
                        },
                    ];
                }
                let mut steps = vec![Step::Result {
                    id: id.clone(),
                    text: output(r),
                    error,
                }];
                // omp keeps an MCP result's parts in its details: what a
                // card draws from. A call through an `xd://` device keeps
                // them a level down.
                let d = match &r["details"]["xdev"]["inner"] {
                    Value::Null => &r["details"],
                    inner => inner,
                };
                if d["serverName"].is_string() {
                    let whole = json!({
                        "content": d["rawContent"],
                        "structuredContent": d["structuredContent"],
                        "_meta": d["mcpMeta"],
                    });
                    let whole = Value::Object(
                        whole
                            .as_object()
                            .into_iter()
                            .flatten()
                            .filter(|(_, v)| !v.is_null())
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    );
                    if let Some(result) = crate::model::structured(&whole) {
                        steps.push(Step::Structured {
                            id,
                            result: display_result(result),
                        });
                    }
                } else if d["diff"].is_string() || !pictures(r).is_empty() {
                    // Native edit diffs and image results are not MCP results,
                    // but need their structured content just as MCP cards do.
                    steps.push(Step::Structured {
                        id,
                        result: display_result(json!({"content": r["content"], "details": d})),
                    });
                }
                steps
            }
            "subagent_lifecycle" => self.lifecycle(&ev["payload"]),
            // A subagent's own event, read as the turn's are and shown as
            // its work.
            "subagent_event" => {
                let p = &ev["payload"];
                let Some((id, sub)) = self.subs.get_mut(p["id"].as_str().unwrap_or_default())
                else {
                    return Vec::new();
                };
                let parent = id.clone();
                sub.update(&p["event"])
                    .into_iter()
                    .map(|step| Step::Inner {
                        parent: parent.clone(),
                        step: Box::new(step),
                    })
                    .collect()
            }
            // omp tries a failed model call again by itself: one row while
            // it does, saying why and which try, ending as the last did.
            "auto_retry_start" => {
                let about = format!(
                    "Try {} of {} in {}s: {}",
                    ev["attempt"].as_u64().unwrap_or(1) + 1,
                    ev["maxAttempts"].as_u64().unwrap_or(1) + 1,
                    ev["delayMs"].as_u64().unwrap_or(0).div_ceil(1000),
                    ev["errorMessage"]
                        .as_str()
                        .unwrap_or("the model call failed")
                );
                match &self.retry {
                    Some(id) => vec![Step::Progress {
                        id: id.clone(),
                        text: about,
                    }],
                    None => {
                        self.retries += 1;
                        let id = format!("retry-{}", self.retries);
                        self.retry = Some(id.clone());
                        vec![Step::Call {
                            id,
                            name: "Retry".into(),
                            input: json!({"description": about}),
                        }]
                    }
                }
            }
            "auto_retry_end" => {
                let Some(id) = self.retry.take() else {
                    return Vec::new();
                };
                let success = ev["success"] == true;
                let text = if success {
                    "The model call went through.".to_owned()
                } else {
                    ev["finalError"]
                        .as_str()
                        .unwrap_or("The model call failed every try.")
                        .to_owned()
                };
                vec![Step::Result {
                    id,
                    text,
                    error: !success,
                }]
            }
            _ => Vec::new(),
        }
    }

    /// A subagent started or ended. Its name and prompt are those its
    /// `task` call gave it; its report is the last it said, and the tokens
    /// it used count as the turn's.
    fn lifecycle(&mut self, p: &Value) -> Vec<Step> {
        let own = p["id"].as_str().unwrap_or_default().to_owned();
        match p["status"].as_str().unwrap_or_default() {
            "started" if !self.subs.contains_key(&own) => {
                let call = p["parentToolCallId"].as_str().unwrap_or("omp");
                let id = format!("{call}-{own}");
                let index = p["index"].as_u64().unwrap_or(0) as usize;
                let asked = self.tasked.get(call).map(|a| &a["tasks"][index]);
                let said = |k: &str| asked.and_then(|t| t[k].as_str()).filter(|s| !s.is_empty());
                let name = said("name")
                    .or(p["description"].as_str())
                    .unwrap_or(&own)
                    .to_owned();
                let task = said("task")
                    .or(p["description"].as_str())
                    .or(p["agent"].as_str())
                    .unwrap_or_default()
                    .to_owned();
                let sub = Reading {
                    servers: self.servers.clone(),
                    ..Reading::default()
                };
                self.subs.insert(own, (id.clone(), sub));
                let input =
                    json!({"description": name, "prompt": task, "subagent_type": p["agent"]});
                vec![
                    Step::Call {
                        id: id.clone(),
                        name: "Agent".into(),
                        input,
                    },
                    Step::Agent { id, name, task },
                ]
            }
            status @ ("completed" | "failed" | "aborted") => {
                let Some((id, sub)) = self.subs.remove(&own) else {
                    return Vec::new();
                };
                if let Some(u) = sub.usage {
                    let t = self.usage.unwrap_or_default();
                    self.usage = Some(Tokens {
                        input: t.input + u.input,
                        cache_read: t.cache_read + u.cache_read,
                        cache_write: t.cache_write + u.cache_write,
                        output: t.output + u.output,
                    });
                }
                let report = match (sub.answer(), status) {
                    (said, _) if !said.is_empty() => said,
                    (_, "completed") => "Done, with nothing to report.".into(),
                    (_, "failed") => "The subagent failed.".into(),
                    _ => "The subagent was stopped.".into(),
                };
                vec![
                    Step::Done {
                        id: id.clone(),
                        report: report.clone(),
                    },
                    Step::Result {
                        id,
                        text: report,
                        error: status != "completed",
                    },
                ]
            }
            _ => Vec::new(),
        }
    }
}

/// A `todo` call, once its result is in: shown as Claude Code's to-do
/// list is (TodoWrite's `todos`, each with its phase), with omp's tally as
/// its result. One that failed shows as omp gave it.
fn todo_steps(id: String, args: &Value, r: &Value, error: bool, reading: &Reading) -> Vec<Step> {
    let phases = r["details"]["phases"].as_array().filter(|_| !error);
    let (name, input) = match phases {
        Some(phases) => {
            let todos: Vec<Value> = phases
                .iter()
                .flat_map(|ph| {
                    ph["tasks"].as_array().into_iter().flatten().map(|t| {
                        json!({"content": t["content"], "status": t["status"], "phase": ph["name"]})
                    })
                })
                .collect();
            ("TodoWrite".to_owned(), json!({ "todos": todos }))
        }
        None => reading.presented("todo", args, None),
    };
    let whole = output(r);
    let text = match phases {
        Some(_) => whole
            .lines()
            .find(|l| l.starts_with("Overall:"))
            .map_or(whole.clone(), str::to_owned),
        None => whole,
    };
    vec![
        Step::Call {
            id: id.clone(),
            name,
            input,
        },
        Step::Result { id, text, error },
    ]
}

/// Display images use the existing file library; neither the stream nor
/// history needs to carry a second copy of their base64 bytes.
fn display_result(mut value: Value) -> Value {
    fn images(value: &mut Value, library: &crate::files::Library) {
        if let Some(object) = value.as_object_mut() {
            if object.get("type").and_then(Value::as_str) == Some("image") {
                use base64::Engine;
                let saved = object
                    .get("data")
                    .and_then(Value::as_str)
                    .and_then(|data| base64::engine::general_purpose::STANDARD.decode(data).ok())
                    .and_then(|bytes| {
                        let ext = match object.get("mimeType").and_then(Value::as_str) {
                            Some("image/jpeg") => "jpg",
                            Some("image/gif") => "gif",
                            Some("image/webp") => "webp",
                            _ => "png",
                        };
                        library.keep(&format!("tool-result.{ext}"), &bytes).ok()
                    });
                if object.remove("data").is_some() {
                    if let Some(name) = saved {
                        object.insert("file".into(), Value::String(name));
                    } else {
                        eprintln!("libri: could not keep a tool display image");
                    }
                }
            }
            for child in object.values_mut() {
                images(child, library);
            }
        } else if let Some(items) = value.as_array_mut() {
            for child in items {
                images(child, library);
            }
        }
    }
    if let Some(state) = servers::state() {
        images(&mut value, &crate::files::Library::new(&state));
    }
    value
}

/// The pictures in a tool's result: its `image` blocks, decoded.
fn pictures(result: &Value) -> Vec<(String, Vec<u8>)> {
    use base64::Engine;
    result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["type"] == "image")
        .filter_map(|c| {
            let data = base64::engine::general_purpose::STANDARD
                .decode(c["data"].as_str()?)
                .ok()?;
            Some((c["mimeType"].as_str()?.to_owned(), data))
        })
        .collect()
}

/// Leaves the turn's pictures in the outbox, which sends them after the
/// answer as pictures (door.rs `send_outbox`), bar any the Library holds
/// already: one the person sent, read back, isn't news to them.
fn keep_pictures(pictures: &[(String, Vec<u8>)]) {
    let Some(state) = servers::state() else {
        return;
    };
    let kept: Vec<Vec<u8>> = std::fs::read_dir(state.join("files"))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.metadata()
                .is_ok_and(|m| pictures.iter().any(|(_, d)| d.len() as u64 == m.len()))
        })
        .filter_map(|e| std::fs::read(e.path()).ok())
        .collect();
    let outbox = state.join("outbox");
    for (mime, data) in pictures {
        if kept.contains(data) || crate::image_type(data).is_none() {
            continue;
        }
        let ext = match mime.as_str() {
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            _ => "png",
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        if let Err(e) = std::fs::create_dir_all(&outbox)
            .and_then(|()| std::fs::write(outbox.join(format!("{stamp}.{ext}")), data))
        {
            eprintln!("libri: omp: a tool's picture: {e}");
        }
    }
}

impl Reading {
    /// A call as Libri's pages draw one: omp's own tools under the names
    /// and inputs they share with Claude Code's (as codex.rs does for
    /// Codex's), its `intent` as a shell call's description, and an MCP
    /// tool under Libri's `mcp__server__tool`. Any other call passes as is.
    fn presented(&self, name: &str, args: &Value, intent: Option<&str>) -> (String, Value) {
        let mut input = args.clone();
        let renamed = |input: &mut Value, from: &str, to: &str| {
            if let Some(o) = input.as_object_mut()
                && let Some(v) = o.remove(from)
            {
                o.insert(to.into(), v);
            }
        };
        // omp mounts the tools it doesn't send the model whole (each MCP
        // tool among them) as `xd://` devices, run by writing their input
        // to them: the call is the device's, with that input.
        if name == "write"
            && let Some(device) = args["path"].as_str().and_then(|p| p.strip_prefix("xd://"))
            && !device.is_empty()
        {
            let inner = args["content"]
                .as_str()
                .and_then(|c| serde_json::from_str::<Value>(c).ok())
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            return self.presented(device, &inner, intent);
        }
        let name = match name {
            "bash" => {
                if let (Some(o), Some(i)) = (input.as_object_mut(), intent) {
                    o.entry("description").or_insert_with(|| i.into());
                }
                "Bash"
            }
            "read" => {
                renamed(&mut input, "path", "file_path");
                "Read"
            }
            "edit" => {
                renamed(&mut input, "path", "file_path");
                "Edit"
            }
            "write" => {
                renamed(&mut input, "path", "file_path");
                "Write"
            }
            "grep" => "Grep",
            "glob" => {
                renamed(&mut input, "path", "pattern");
                "Glob"
            }
            "ask" => {
                input = json!({"questions": questions(args)});
                "AskUserQuestion"
            }
            // The subagents a call starts, by their names; each gets a
            // thread of its own.
            "task" => {
                let names: Vec<&str> = args["tasks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|t| t["name"].as_str().or(t["agent"].as_str()))
                    .collect();
                input = json!({"description": names.join(", "), "prompt": args["context"]});
                "Task"
            }
            other => {
                let mcp = self
                    .servers
                    .iter()
                    .filter(|(prefix, _)| other.starts_with(prefix.as_str()))
                    .max_by_key(|(prefix, _)| prefix.len());
                return match mcp {
                    Some((prefix, server)) => {
                        (format!("mcp__{server}__{}", &other[prefix.len()..]), input)
                    }
                    None => (other.to_owned(), input),
                };
            }
        };
        (name.to_owned(), input)
    }
}

/// A name as omp puts it into an MCP tool's (its mcp module's
/// `sanitizeMCPToolNamePart`): lowercase, each run of anything but letters,
/// digits and `_` one `_`, none at either end.
fn omp_part(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        let c = if c.is_ascii_alphanumeric() || c == '_' {
            c
        } else {
            '_'
        };
        if !(c == '_' && out.ends_with('_')) {
            out.push(c);
        }
    }
    out.trim_matches('_').to_owned()
}

/// omp's ask questions as AskUserQuestion's: `question`, `header`,
/// `options` of `label` and `description`, `multiSelect`.
fn questions(asked: &Value) -> Vec<Value> {
    asked["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|q| {
            let options: Vec<Value> = q["options"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|o| {
                    let mut c = json!({"label": o["label"]});
                    if o["description"].is_string() {
                        c["description"] = o["description"].clone();
                    }
                    c
                })
                .collect();
            let mut out = json!({
                "question": q["question"],
                "options": options,
                "multiSelect": q["multi"] == true,
            });
            if q["header"].is_string() {
                out["header"] = q["header"].clone();
            }
            out
        })
        .collect()
}

/// The person's answers, which Libri keeps by each question's words (several
/// picks joined by ", "), as omp's ask dialog takes them: per question, in
/// order, the picked labels, else the words as typed.
fn answers(asked: &Value, answered: &Value) -> Vec<Value> {
    asked["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|q| {
            let said = answered["answers"][q["question"].as_str().unwrap_or_default()]
                .as_str()
                .unwrap_or_default()
                .trim();
            let labels: Vec<&str> = q["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|o| o["label"].as_str())
                .collect();
            let picks: Vec<&str> = if q["multi"] == true {
                said.split(", ").collect()
            } else {
                vec![said]
            };
            let mut out = json!({"id": q["id"], "selectedOptions": []});
            if !said.is_empty() && picks.iter().all(|p| labels.contains(p)) {
                out["selectedOptions"] = json!(picks);
            } else if !said.is_empty() {
                out["customInput"] = said.into();
            }
            out
        })
        .collect()
}

fn ask_id(dialog: &str) -> String {
    format!("omp-{dialog}")
}

/// Whether a select dialog is omp's tool approval.
fn approval(frame: &Value) -> bool {
    let options = frame["options"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    frame["title"]
        .as_str()
        .is_some_and(|t| t.starts_with("Allow tool:"))
        && [APPROVE, DENY]
            .iter()
            .all(|o| options.iter().any(|v| v == o))
}

/// A tool result's text blocks, joined.
fn output(result: &Value) -> String {
    result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_edit_result_keeps_its_diff_for_the_page() {
        let mut r = Reading::default();
        let steps = r.update(&json!({"type":"tool_execution_end","toolCallId":"edit","toolName":"edit",
            "result":{"content":[{"type":"text","text":"Updated file"}],"details":{"path":"src/config.rs","diff":"- off\n+ on","oldText":"off","newText":"on"}}}));
        assert!(
            matches!(&steps[..], [Step::Result { .. }, Step::Structured { result, .. }] if result["details"]["newText"] == "on")
        );
    }

    #[test]
    fn rpc_state_reports_context_and_generation_speed_without_estimating() {
        let mut r = Reading::default();
        let steps = r.update(&json!({"type":"response", "id":"libri-metrics", "success":true,
            "data":{"contextUsage":{"tokens":32000,"contextWindow":128000},"tokensPerSecond":42.5}}));
        assert!(
            matches!(&steps[..], [Step::Metrics { context_tokens: Some(32000), context_window: Some(128000), tokens_per_second: Some(n) }] if *n == 42.5)
        );
        let empty = r.update(&json!({"type":"response", "id":"libri-final-metrics", "success":true,"data":{"tokensPerSecond":0}}));
        assert!(matches!(
            &empty[..],
            [Step::Metrics {
                context_tokens: None,
                context_window: None,
                tokens_per_second: None
            }]
        ));
        assert!(
            r.update(&json!({"type":"response", "id":"unrelated", "success":true}))
                .is_empty()
        );
    }

    #[test]
    fn events_become_steps_and_the_answer_follows_the_last_call() {
        let mut r = Reading::default();
        let delta = |kind: &str, text: &str| {
            json!({"type": "message_update",
            "assistantMessageEvent": {"type": kind, "contentIndex": 0, "delta": text}})
        };
        assert!(
            matches!(&r.update(&delta("thinking_delta", "hm"))[..], [Step::Thinking { text }] if text == "hm")
        );
        r.update(&delta("text_delta", "Looking."));
        let start = json!({"type": "tool_execution_start", "toolCallId": "c1", "toolName": "bash",
            "args": {"command": "echo hi"}, "intent": "Say hi"});
        assert!(
            matches!(&r.update(&start)[..], [Step::Call { name, input, .. }] if name == "Bash" && input["description"] == "Say hi")
        );
        let end = json!({"type": "tool_execution_end", "toolCallId": "c1", "toolName": "bash",
            "result": {"content": [{"type": "text", "text": "hi\n"}], "details": {}}, "isError": false});
        assert!(
            matches!(&r.update(&end)[..], [Step::Result { text, error: false, .. }] if text == "hi\n")
        );
        r.update(&delta("text_delta", "\n\nhi"));
        assert_eq!(r.answer(), "hi");
    }

    #[test]
    fn usage_sums_the_turns_model_calls() {
        let mut r = Reading::default();
        let end = |i: u64, c: u64, o: u64| {
            json!({"type": "message_end", "message": {"role": "assistant",
            "usage": {"input": i, "output": o, "cacheRead": c, "cacheWrite": 0}}})
        };
        r.update(&end(100, 0, 10));
        r.update(&end(5, 100, 2));
        r.update(&json!({"type": "message_end", "message": {"role": "toolResult"}}));
        let want = Tokens {
            input: 205,
            cache_read: 100,
            cache_write: 0,
            output: 12,
        };
        assert_eq!(r.usage, Some(want));
    }

    #[test]
    fn a_failed_model_call_is_kept_until_one_succeeds() {
        let mut r = Reading::default();
        let end = |reason: &str| {
            json!({"type": "message_end", "message": {"role": "assistant", "usage": {},
            "stopReason": reason, "errorMessage": "529 overloaded"}})
        };
        r.update(&end("error"));
        assert_eq!(r.failure.as_deref(), Some("529 overloaded"));
        r.update(&end("stop"));
        assert_eq!(r.failure, None);
    }

    #[test]
    fn a_running_calls_output_shows_as_it_grows_and_the_result_ends_it() {
        let mut r = Reading::default();
        let update = |text: &str| {
            json!({"type": "tool_execution_update", "toolCallId": "c1", "toolName": "bash",
            "partialResult": {"content": [{"type": "text", "text": text}], "details": {}}})
        };
        assert!(
            matches!(&r.update(&update("line 1\n"))[..], [Step::Progress { id, text }] if id == "c1" && text == "line 1\n")
        );
        // Sooner than PROGRESS_EVERY, the next waits for the one after.
        assert!(r.update(&update("line 1\nline 2\n")).is_empty());
        r.shown.insert("c1".into(), Instant::now() - PROGRESS_EVERY);
        assert!(
            matches!(&r.update(&update("line 1\nline 2\n"))[..], [Step::Progress { text, .. }] if text.ends_with("line 2\n"))
        );
        let end = json!({"type": "tool_execution_end", "toolCallId": "c1", "toolName": "bash",
            "result": {"content": [{"type": "text", "text": "done"}], "details": {}}, "isError": false});
        assert!(matches!(&r.update(&end)[..], [Step::Result { .. }]));
        assert!(r.shown.is_empty());
    }

    #[test]
    fn omps_tools_read_as_libris() {
        let r = Reading {
            servers: vec![(format!("mcp__{}_", omp_part("libri")), "libri".into())],
            ..Reading::default()
        };
        let (name, input) = r.presented("read", &json!({"path": "/a.rs"}), None);
        assert_eq!(
            (name.as_str(), &input["file_path"]),
            ("Read", &json!("/a.rs"))
        );
        let (name, _) = r.presented("mcp__libri_show_page", &json!({}), None);
        assert_eq!(name, "mcp__libri__show_page");
        let (name, _) = r.presented("mcp__other_tool", &json!({}), None);
        assert_eq!(name, "mcp__other_tool");
        let device = json!({"path": "xd://mcp__libri_show_page", "content": "{\"name\":\"n\"}"});
        let (name, input) = r.presented("write", &device, None);
        assert_eq!(
            (name.as_str(), &input),
            ("mcp__libri__show_page", &json!({"name": "n"}))
        );
        assert_eq!(omp_part("My-Server.2"), "my_server_2");
    }

    #[test]
    fn an_mcp_result_brings_its_card() {
        let mut r = Reading::default();
        let end = json!({"type": "tool_execution_end", "toolCallId": "c2", "toolName": "mcp__libri_show_page",
            "result": {"content": [{"type": "text", "text": "shown"}],
                "details": {"xdev": {"tool": "mcp__libri_show_page", "mode": "execute", "inner": {
                    "serverName": "libri", "mcpToolName": "show_page",
                    "rawContent": [{"type": "text", "text": "shown"}],
                    "mcpMeta": {"ui": {"resourceUri": "ui://libri/page"}}}}}}});
        let steps = r.update(&end);
        assert!(
            matches!(&steps[..], [Step::Result { .. }, Step::Structured { id, result }] if id == "c2" && result["_meta"]["ui"]["resourceUri"] == "ui://libri/page")
        );
    }

    #[test]
    fn asks_questions_go_as_libris_and_answers_come_back_as_omps() {
        let asked = json!({"type": "extension_ui_request", "id": "ui_9", "method": "ask",
            "questions": [
                {"id": "db", "question": "Which database?", "options": [{"label": "Postgres"}, {"label": "SQLite"}]},
                {"id": "f", "question": "Which features?", "options": [{"label": "Auth"}, {"label": "Search"}], "multi": true}]});
        let qs = questions(&asked);
        assert_eq!(qs[1]["multiSelect"], true);
        assert_eq!(qs[0]["options"][1]["label"], "SQLite");
        let answered =
            json!({"answers": {"Which database?": "DuckDB", "Which features?": "Auth, Search"}});
        let got = answers(&asked, &answered);
        assert_eq!(
            got[0],
            json!({"id": "db", "selectedOptions": [], "customInput": "DuckDB"})
        );
        assert_eq!(
            got[1],
            json!({"id": "f", "selectedOptions": ["Auth", "Search"]})
        );
    }

    #[test]
    fn only_omps_tool_approval_counts_as_one() {
        let ask = json!({"type": "extension_ui_request", "id": "1", "method": "select",
            "title": "Allow tool: bash\nCommand: date", "options": ["Approve", "Approve bash for this session", "Deny"]});
        assert!(approval(&ask));
        let other = json!({"type": "extension_ui_request", "id": "2", "method": "select",
            "title": "Which database?", "options": ["Postgres", "SQLite"]});
        assert!(!approval(&other));
    }

    #[test]
    fn each_subagent_gets_its_own_agent_call_and_thread_named_as_its_task_call_named_it() {
        let mut r = Reading::default();
        let start = json!({"type": "tool_execution_start", "toolCallId": "c1", "toolName": "task",
            "args": {"context": "In /tmp.", "tasks": [{"name": "EchoRunner", "agent": "sonic", "task": "Run echo."}]}});
        assert!(
            r.update(&start).is_empty(),
            "the task call shows through its subagents"
        );
        let life = |status: &str| {
            json!({"type": "subagent_lifecycle", "payload": {"id": "EchoRunner",
            "agent": "sonic", "parentToolCallId": "c1", "status": status, "index": 0}})
        };
        assert!(matches!(&r.update(&life("started"))[..],
            [Step::Call { id: call, name: agent, input }, Step::Agent { id, name, task }]
                if call == "c1-EchoRunner" && agent == "Agent" && input["description"] == "EchoRunner"
                && id == "c1-EchoRunner" && name == "EchoRunner" && task == "Run echo."));
        let sub = |event: Value| json!({"type": "subagent_event", "payload": {"id": "EchoRunner", "event": event}});
        let call = sub(
            json!({"type": "tool_execution_start", "toolCallId": "s1", "toolName": "bash",
            "args": {"command": "echo ok"}}),
        );
        assert!(matches!(&r.update(&call)[..],
            [Step::Inner { parent, step }] if parent == "c1-EchoRunner"
                && matches!(step.as_ref(), Step::Call { name, .. } if name == "Bash")));
        r.update(&sub(json!({"type": "message_update",
            "assistantMessageEvent": {"type": "text_delta", "delta": "stdout: ok"}})));
        r.update(&sub(
            json!({"type": "message_end", "message": {"role": "assistant",
            "usage": {"input": 10, "output": 2, "cacheRead": 0, "cacheWrite": 0}}}),
        ));
        assert!(matches!(&r.update(&life("completed"))[..],
            [Step::Done { id, report }, Step::Result { error: false, .. }] if id == "c1-EchoRunner" && report == "stdout: ok"));
        let end = json!({"type": "tool_execution_end", "toolCallId": "c1", "toolName": "task", "isError": false,
            "result": {"content": [{"type": "text", "text": "stdout: ok"}]}});
        assert!(
            r.update(&end).is_empty(),
            "its report is already in its subagent's card"
        );
        assert_eq!(r.usage.map(|u| u.output), Some(2));
        assert!(
            r.update(&call).is_empty(),
            "an ended subagent shows no more"
        );
    }

    #[test]
    fn a_todo_call_shows_its_list_once_the_result_holds_it() {
        let mut r = Reading::default();
        let start = json!({"type": "tool_execution_start", "toolCallId": "t1", "toolName": "todo",
            "args": {"op": "done", "task": "Grind beans"}});
        assert!(r.update(&start).is_empty());
        let end = json!({"type": "tool_execution_end", "toolCallId": "t1", "toolName": "todo", "isError": false,
            "result": {"content": [{"type": "text", "text": "Remaining items (1):\nOverall: 1/2 done, 1 open."}],
            "details": {"op": "done", "phases": [{"name": "Make coffee", "tasks": [
                {"content": "Grind beans", "status": "completed"}, {"content": "Pour", "status": "in_progress"}]}]}}});
        let steps = r.update(&end);
        let [Step::Call { name, input, .. }, Step::Result { text, .. }] = &steps[..] else {
            panic!("{steps:?}")
        };
        assert_eq!(name, "TodoWrite");
        assert_eq!(
            input["todos"][0],
            json!({"content": "Grind beans", "status": "completed", "phase": "Make coffee"})
        );
        assert_eq!(input["todos"][1]["status"], "in_progress");
        assert_eq!(text, "Overall: 1/2 done, 1 open.");
    }

    #[test]
    fn a_retried_model_call_is_one_row_that_ends_as_the_last_try_did() {
        let mut r = Reading::default();
        let start = |n: u64| {
            json!({"type": "auto_retry_start", "attempt": n, "maxAttempts": 3,
            "delayMs": 2000, "errorMessage": "429 Too Many Requests"})
        };
        assert!(matches!(&r.update(&start(1))[..],
            [Step::Call { id, name, input }] if id == "retry-1" && name == "Retry"
                && input["description"] == "Try 2 of 4 in 2s: 429 Too Many Requests"));
        assert!(matches!(&r.update(&start(2))[..], [Step::Progress { id, .. }] if id == "retry-1"));
        let end = json!({"type": "auto_retry_end", "success": true, "attempt": 2});
        assert!(
            matches!(&r.update(&end)[..], [Step::Result { id, error: false, .. }] if id == "retry-1")
        );
        assert!(matches!(&r.update(&start(1))[..], [Step::Call { id, .. }] if id == "retry-2"));
    }

    #[test]
    fn a_tools_pictures_are_kept_for_the_person() {
        let mut r = Reading::default();
        let png = [0x89, b'P', b'N', b'G', 1, 2, 3];
        let data = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, png);
        let end = json!({"type": "tool_execution_end", "toolCallId": "c1", "toolName": "read", "isError": false,
            "result": {"content": [{"type": "text", "text": "a picture"},
                {"type": "image", "mimeType": "image/png", "data": data}]}});
        r.update(&end);
        assert_eq!(r.pictures, vec![("image/png".to_owned(), png.to_vec())]);
    }
}
