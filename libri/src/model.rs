//! The harness behind Libri, one call per message: the agent program
//! that runs the model and its tools. The person may switch among those
//! installed on this machine (host.rs `harnesses`): Claude Code
//! (its stream-json, here), Codex (codex.rs), OpenCode (opencode.rs), omp
//! (omp.rs), any ACP agent (acp.rs), or a bare OpenAI-compatible endpoint.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use crate::servers::{self, Scope};
use crate::sessions::{self, Start};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// Seconds one call to a model's HTTP endpoint may take. Harness turns have
/// no limit: Stop ends them.
const TIMEOUT: Duration = Duration::from_secs(300);

/// A file sent with a message, in a form models read directly.
#[derive(Clone)]
pub enum Attachment {
    Image { mime: &'static str, data: Vec<u8> },
    Pdf(Vec<u8>),
}

pub trait Harness: Send + Sync {
    /// Answers a prompt. With `acts` false the tools that change anything
    /// (reminders, lists, the mailbox, the calendar, pictures, fetching a
    /// page) are withheld: the turn was started by a watch or someone
    /// else's message, not by the person. `memory` is the hippo view, sent
    /// ahead of everything else in the message so its settled part can be
    /// read from the prompt cache on the next call.
    fn answer(
        &self,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        run: Run,
    ) -> Result<String, String>;

    /// Whether answers have tools: web search and Libri's own.
    fn tools(&self) -> bool {
        false
    }
}

/// The product's name, which the chat shows where a deployment names the
/// assistant nothing else. No prompt names the assistant: who it is opens
/// the person's rules, as their Identity section (`guide::identity`).
pub const ASSISTANT: &str = "Libri";

/// The title of the chat this daemon keeps in a hippo that holds other
/// chats too (another daemon's, or the sessions it follows when it records
/// all); None when the hippo holds this daemon's alone.
pub fn shared_chat() -> Option<String> {
    std::env::var("LIBRI_HIPPO_SHARED")
        .ok()
        .filter(|t| !t.is_empty())
}

pub fn from_env() -> Result<Box<dyn Harness>, String> {
    if crate::catalog::harnesses().len() > 1 {
        return Ok(Box::new(Clipping(Box::new(Switch {
            made: Mutex::default(),
        }))));
    }
    harness(&crate::catalog::configured_default())
        .map(|h| Box::new(Clipping(h)) as Box<dyn Harness>)
}

/// Cuts the results a turn's steps carry, whichever harness runs it. A
/// scratchpad's are left whole: its log is all its next turn knows of them,
/// and it cuts what it shows itself.
struct Clipping(Box<dyn Harness>);

impl Harness for Clipping {
    fn answer(
        &self,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        mut run: Run,
    ) -> Result<String, String> {
        if !run.ephemeral
            && let Some(steps) = run.steps.take()
        {
            run.steps = Some(Arc::new(move |s| steps(clipped(s))));
        }
        // A skill a model made in Libri's store reaches every harness's
        // folder before the next turn looks; a plain call reads none.
        if !run.plain {
            crate::skills::sync_here();
        }
        self.0.answer(system, memory, prompt, files, run)
    }

    fn tools(&self) -> bool {
        self.0.tools()
    }
}

/// ACP agents known by name, as Zed's and T3 Code's registries know them.
const ACP: &[(&str, &str)] = &[("grok", "grok agent stdio")];

/// The harness `name` names, each of its token reports kept in the usage
/// ledger.
pub fn harness(name: &str) -> Result<Box<dyn Harness>, String> {
    Ok(Box::new(Ledger {
        name: name.to_owned(),
        inner: made(name)?,
    }))
}

/// Keeps every token report of the harness it wraps in the usage ledger,
/// whether or not anyone watches the turn's steps.
struct Ledger {
    name: String,
    inner: Box<dyn Harness>,
}

impl Harness for Ledger {
    fn answer(
        &self,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        mut run: Run,
    ) -> Result<String, String> {
        let caller = run.by.clone();
        let harness = self.name.clone();
        // A harness that names no model ran the one this turn picked, else
        // its own default.
        let picked = run
            .model_for(&harness)
            .or_else(|| crate::catalog::default_model(&harness))
            .unwrap_or_else(|| "default".into());
        let watching = run.steps.take();
        run.steps = Some(Arc::new(move |s: Step| {
            if let Step::Usage {
                input,
                cache_read,
                cache_write,
                output,
                model,
            } = &s
            {
                crate::ledger::harness_used(
                    &caller,
                    &harness,
                    model.as_deref().unwrap_or(&picked),
                    (*input, *cache_read, *cache_write, *output),
                );
            }
            if let Some(w) = &watching {
                w(s);
            }
        }));
        self.inner.answer(system, memory, prompt, files, run)
    }

    fn tools(&self) -> bool {
        self.inner.tools()
    }
}

/// The harness `name` names: a native one, a known ACP agent, or "acp"
/// for the command in LIBRI_ACP.
fn made(name: &str) -> Result<Box<dyn Harness>, String> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let acp = |line: &str| -> Result<Box<dyn Harness>, String> {
        let command = line.split_whitespace().map(str::to_owned).collect();
        Ok(Box::new(crate::acp::Acp::new(command)?))
    };
    match name {
        "claude" => Ok(Box::new(Claude {
            command: var("LIBRI_CLAUDE").unwrap_or_else(|| "claude".into()),
            model: var("LIBRI_MODEL").unwrap_or_else(|| "sonnet".into()),
            effort: var("LIBRI_EFFORT"),
            coding: var("LIBRI_CODING").is_none_or(|v| v != "0"),
        })),
        "codex" => Ok(Box::new(crate::codex::Codex::from_env()?)),
        "opencode" => Ok(Box::new(crate::opencode::OpenCode::from_env()?)),
        "omp" => Ok(Box::new(crate::omp::Omp::from_env()?)),
        "pi" => Ok(Box::new(crate::pi::Pi::from_env()?)),
        "muse" => Ok(Box::new(crate::muse::Muse::from_env()?)),
        "acp" => acp(&var("LIBRI_ACP").ok_or("LIBRI_ACP is not set")?),
        "http" => Ok(Box::new(Http {
            url: var("LIBRI_URL").ok_or("LIBRI_URL is not set")?,
            model: var("LIBRI_MODEL").ok_or("LIBRI_MODEL is not set")?,
        })),
        other => match ACP.iter().find(|(n, _)| *n == other) {
            Some((_, line)) => acp(line),
            None => Err(format!("Unknown harness {other}.")),
        },
    }
}

/// The harness of the model the person picked, else the default; each
/// made on first use and kept, so a server one starts lives on.
struct Switch {
    made: Mutex<std::collections::HashMap<String, Arc<dyn Harness>>>,
}

impl Switch {
    fn now(&self) -> Result<Arc<dyn Harness>, String> {
        self.named(crate::catalog::harness_now())
    }

    fn named(&self, name: String) -> Result<Arc<dyn Harness>, String> {
        let mut made = self.made.lock().unwrap();
        if let Some(h) = made.get(&name) {
            return Ok(h.clone());
        }
        let h: Arc<dyn Harness> = harness(&name)?.into();
        made.insert(name, h.clone());
        Ok(h)
    }
}

impl Harness for Switch {
    fn answer(
        &self,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        run: Run,
    ) -> Result<String, String> {
        self.named(run.harness())?
            .answer(system, memory, prompt, files, run)
    }

    fn tools(&self) -> bool {
        self.now().is_ok_and(|h| h.tools())
    }
}

/// The thinking effort the person picked, as `claude --effort` takes it,
/// kept in the state directory for the turns after this one; the page
/// offers the levels the picked model's harness lists.
pub const EFFORT: Kept = Kept("effort");

/// A choice kept in the state directory, one file each.
pub struct Kept(pub &'static str);

impl Kept {
    /// None until the person picks.
    pub fn read(&self, state: &Path) -> Option<String> {
        let v = std::fs::read_to_string(state.join(self.0)).ok()?;
        let v = v.trim();
        (!v.is_empty()).then(|| v.to_owned())
    }

    /// Whole or not at all: written beside and renamed over, so a crash
    /// part way leaves the old value rather than a broken one.
    pub fn write(&self, state: &Path, value: &str) -> Result<(), String> {
        let path = state.join(self.0);
        let new = state.join(format!(".{}.new", self.0));
        std::fs::write(&new, value)
            .and_then(|()| std::fs::rename(&new, &path))
            .map_err(|e| e.to_string())
    }

    /// Back to the harness's own default.
    pub fn clear(&self, state: &Path) {
        let _ = std::fs::remove_file(state.join(self.0));
    }
}

/// A comma-separated environment list.
pub fn list(env: &str) -> Vec<String> {
    std::env::var(env)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_owned)
        .collect()
}

/// How a turn runs: whether it may act, what stops it, and whether it
/// has a clock on it (a subagent has none).
#[derive(Clone, Default)]
pub struct Run {
    pub acts: bool,
    pub stop: Option<Arc<AtomicBool>>,
    /// A subagent's, by its ID: it can't start or stop agents of its own.
    pub sub: Option<String>,
    /// Where each tool call and its result go as they happen, for a door
    /// that shows the work as well as the answer.
    pub steps: Option<Steps>,
    /// Where the person's next words go while this turn runs.
    pub steer: Option<Arc<Steer>>,
    /// Who answers the seat's questions and permission prompts; without
    /// one they are refused, as `claude -p` refuses what it would ask.
    pub ask: Option<Arc<dyn Asker>>,
    /// What watches this turn for silence.
    pub monitor: Option<Arc<crate::monitor::Monitor>>,
    /// A call whose words stay out of the memory, and out of what the
    /// harness keeps of its own.
    pub ephemeral: bool,
    /// The scratchpad's with the memory off: the same turn as with it on,
    /// the person's harness and tools whole, but for what reads hippo.
    pub memory_off: bool,
    /// The hippo store of the named memory this run belongs to: its chat's
    /// (`servers::Scope::Apart`) or one of its workers'. Held apart from the
    /// main memory, its memory tools read this store.
    pub memory_dir: Option<std::path::PathBuf>,
    /// One model call and nothing else, for hippo's compactor: no tools,
    /// no rules but `system`, whichever harness runs it.
    pub plain: bool,
    /// The model and thinking of this turn alone, in place of the main
    /// chat's pick: Temp's own.
    pub own: Option<crate::catalog::TurnPick>,
    /// Who asked, for the usage ledger.
    pub by: crate::ledger::Caller,
    /// A worker's own scratch folder, its harness's TMPDIR: a worker that
    /// kept probes in the shared /tmp and then searched it had Claude Code
    /// crawl 72 GB there, driving the host's load to ~290.
    pub scratch: Option<std::path::PathBuf>,
    /// The chat whose harness session this turn goes on with (sessions.rs);
    /// None for a call that keeps none.
    pub session: Option<String>,
    /// The chat whose session this one starts as a copy of when it has none
    /// of its own: a worker's, the main chat's.
    pub fork: Option<String>,
}

impl Run {
    /// The harness gave a word: the turn lives, and has read what it was sent.
    pub fn heard(&self) {
        if let Some(m) = &self.monitor {
            m.heard();
        }
        if let Some(s) = &self.steer {
            s.take_unread();
        }
    }

    /// Names the process the turn runs in.
    pub fn runs_in(&self, pid: u32) {
        if let Some(m) = &self.monitor {
            m.process(pid);
        }
    }

    /// The harness this turn runs on.
    pub fn harness(&self) -> String {
        self.own
            .as_ref()
            .map_or_else(crate::catalog::harness_now, |o| o.pick.harness.clone())
    }

    /// This turn's model on `harness`, else its default in Settings; None
    /// for the harness's own.
    pub fn model_for(&self, harness: &str) -> Option<String> {
        match &self.own {
            Some(o) if o.pick.harness != harness => None,
            Some(o) if o.pick.model.is_empty() => crate::catalog::default_model(harness),
            Some(o) => Some(o.pick.model.clone()),
            None => crate::catalog::model_for(harness),
        }
    }

    /// Points the harness `cmd` starts at this run's scratch folder, when it
    /// has one, for its temporary files and the shells it runs.
    pub fn temp(&self, cmd: &mut Command) {
        if let Some(dir) = &self.scratch {
            cmd.env("TMPDIR", dir);
        }
        // Everything the harness starts inherits it, which is how the
        // system monitor tells each worker's share of the host apart.
        if let Some(worker) = &self.sub {
            cmd.env(crate::servers::Scope::WORKER, worker);
        }
    }

    /// This turn's thinking; None for the harness's default.
    pub fn effort(&self) -> Option<String> {
        match &self.own {
            Some(o) => o.effort.clone(),
            None => std::env::var_os("STATE_DIRECTORY").and_then(|s| EFFORT.read(Path::new(&s))),
        }
    }

    /// How freely this turn acts on `harness`: one that must not act never
    /// goes past the safe mode, so whatever it would do has to ask, and with
    /// no one to ask is refused. A thread goes by its own pick, the main
    /// chat by its pick on the harness it is on.
    pub fn access(&self, harness: &str) -> crate::access::Grant {
        if !self.acts {
            return crate::access::forced(harness);
        }
        let state = std::env::var_os("STATE_DIRECTORY")
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        let pick = match &self.own {
            Some(o) => o.picks(),
            None => crate::access::main_picks(&state, harness),
        };
        crate::access::grant(harness, &state, &pick)
    }
}

/// The way into the turn under way: words written here reach the model at
/// its next step, as Claude Code takes a message typed while it works. It
/// is open from the turn's first words until its answer is out.
#[derive(Default)]
pub struct Steer {
    /// The lines on their way to the CLI, written by a thread of its own.
    into: Mutex<Option<mpsc::Sender<String>>>,
    /// The harness's own way in, on the harnesses that are not Claude Code.
    words: Mutex<Option<mpsc::Sender<Sent>>>,
    /// Counts each opening: what waited for a turn to take it goes in then.
    opened: tokio::sync::watch::Sender<u64>,
    /// The person's messages since the model last gave a word.
    unread: AtomicU64,
    /// Libri's own control requests the CLI has yet to answer, by id: what
    /// each asked, to say which one it refused.
    asked: Mutex<HashMap<String, String>>,
    /// The CLI's tasks under way (its subagents, its background shells), by
    /// the call that started them, for stop_task.
    tasks: Mutex<HashMap<String, String>>,
}

/// Numbers Libri's control requests, which the CLI's answers name.
static REQUESTS: AtomicU64 = AtomicU64::new(0);

/// One message on its way to a harness's own way in.
type Sent = (String, Vec<Attachment>);

/// Hands one message to a turn under way; false when it did not take it.
pub type Words = Box<dyn FnMut(&str, &[Attachment]) -> bool + Send>;

impl Steer {
    /// Hands `text`, with any photos or PDFs, to the turn under way; false
    /// when none will read it, and it should wait its turn instead.
    pub fn send(&self, text: &str, files: &[Attachment]) -> bool {
        if let Some(words) = self.words.lock().unwrap().as_ref() {
            return words.send((text.to_owned(), files.to_vec())).is_ok();
        }
        let mut into = self.into.lock().unwrap();
        let Some(lines) = into.as_ref() else {
            return false;
        };
        let mut content: Vec<Value> = files.iter().map(block).collect();
        content.push(json!({"type": "text", "text": text}));
        if lines.send(user(content)).is_err() {
            *into = None;
            return false;
        }
        true
    }

    /// Counts one message of the person's handed to the turn.
    pub fn sent_words(&self) {
        self.unread.fetch_add(1, Ordering::SeqCst);
    }

    /// The messages nothing has come after, starting the count again.
    pub fn take_unread(&self) -> u64 {
        self.unread.swap(0, Ordering::SeqCst)
    }

    /// Whether a turn is under way to take words.
    pub fn running(&self) -> bool {
        self.into.lock().unwrap().is_some() || self.words.lock().unwrap().is_some()
    }

    /// Opens the way in for a harness that takes words its own way.
    pub fn take_words(&self, mut words: Words) {
        // A thread of its own for the reason `open` has one.
        let (lines, waiting) = mpsc::channel::<Sent>();
        thread::spawn(move || {
            for (text, files) in waiting {
                if !words(&text, &files) {
                    break;
                }
            }
        });
        *self.words.lock().unwrap() = Some(lines);
        self.opened.send_modify(|n| *n += 1);
    }

    fn open(&self, mut stdin: ChildStdin) {
        // Apart from the lock: a CLI that stopped reading fills the pipe,
        // and a write then blocks until it dies, which Stop has to bring
        // about. Ending the lines still ends the input, after the last one.
        let (lines, waiting) = mpsc::channel::<String>();
        thread::spawn(move || {
            for line in waiting {
                let wrote = stdin.write_all(line.as_bytes());
                if wrote.and_then(|()| stdin.flush()).is_err() {
                    break;
                }
            }
        });
        *self.into.lock().unwrap() = Some(lines);
        self.opened.send_modify(|n| *n += 1);
    }

    /// Changes each time the way in opens.
    pub fn opened(&self) -> tokio::sync::watch::Receiver<u64> {
        self.opened.subscribe()
    }

    /// Answers the CLI's control request `id`, while the input is open.
    fn respond(&self, id: &str, response: Value) {
        let line = json!({"type": "control_response",
            "response": {"subtype": "success", "request_id": id, "response": response}});
        if let Some(lines) = self.into.lock().unwrap().as_ref() {
            let _ = lines.send(format!("{line}\n"));
        }
    }

    /// Sends the CLI a control request (the Agent SDK's set_model,
    /// set_permission_mode), while the input is open. Its answer
    /// comes back on stdout as a control_response.
    /// Notes a task the CLI started, by the call that started it.
    fn task_started(&self, ev: &Value) {
        if let (Some(call), Some(task)) = (ev["tool_use_id"].as_str(), ev["task_id"].as_str())
            && ev["subtype"] == "task_started"
        {
            self.tasks
                .lock()
                .unwrap()
                .insert(call.to_owned(), task.to_owned());
        }
    }

    fn control(&self, request: Value) -> bool {
        let id = format!("libri-{}", REQUESTS.fetch_add(1, Ordering::Relaxed));
        let what = request["subtype"].as_str().unwrap_or_default().to_owned();
        let line = json!({"type": "control_request", "request_id": id, "request": request});
        let sent = self
            .into
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|lines| lines.send(format!("{line}\n")).is_ok());
        if sent {
            self.asked.lock().unwrap().insert(id, what);
        }
        sent
    }

    /// Takes the CLI's answer to one of Libri's control requests: a refusal
    /// (a model it does not know) goes to the journal, and the pick waits
    /// for the next turn.
    fn answered(&self, ev: &Value) {
        let r = &ev["response"];
        let Some(what) = r["request_id"]
            .as_str()
            .and_then(|id| self.asked.lock().unwrap().remove(id))
        else {
            return;
        };
        if r["subtype"] == "error" {
            eprintln!(
                "libri: claude refused {what}: {}",
                r["error"].as_str().unwrap_or_default()
            );
        }
    }

    /// Answers a control request of the CLI's that Libri does not take (a
    /// hook's callback, an SDK server's message, one added in a later
    /// release) with an error, as the Agent SDK does, so the CLI goes on
    /// rather than waiting for an answer that never comes.
    fn refuse(&self, id: &str, subtype: &str) {
        let line = json!({"type": "control_response", "response": {"subtype": "error",
            "request_id": id, "error": format!("Unsupported control request subtype: {subtype}")}});
        if let Some(lines) = self.into.lock().unwrap().as_ref() {
            let _ = lines.send(format!("{line}\n"));
        }
    }

    /// Ends the input: the CLI exits once it has answered what it has.
    pub fn close(&self) {
        self.into.lock().unwrap().take();
        self.words.lock().unwrap().take();
    }
}

/// An empty folder for a harness asked something outside any turn (its
/// models, its usage), never /tmp: a harness may index the folder it starts
/// in, and /tmp is shared and large. Under the state directory, else the
/// folder Libri runs in.
pub fn bare_dir() -> Result<std::path::PathBuf, String> {
    let root = std::env::var_os("STATE_DIRECTORY")
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .map_or_else(std::env::current_dir, Ok)
        .map_err(|e| e.to_string())?;
    let dir = root.join("bare");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// The model a CLI's first line says the session is on.
fn init_model(line: &str) -> Option<String> {
    if !line.contains(r#""init""#) {
        return None;
    }
    let ev = serde_json::from_str::<Value>(line).ok()?;
    (ev["type"] == "system" && ev["subtype"] == "init")
        .then(|| ev["model"].as_str().map(str::to_owned))
        .flatten()
}

/// The tool servers a CLI's first line says did not start.
fn failed_servers(line: &str) -> Vec<String> {
    if !line.contains(r#""init""#) {
        return Vec::new();
    }
    let ev = serde_json::from_str::<Value>(line).unwrap_or_default();
    ev["mcp_servers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["status"] == "failed")
        .filter_map(|m| Some(m["name"].as_str()?.to_owned()))
        .collect()
}

/// One user message as a stream-json input line.
fn user(content: Vec<Value>) -> String {
    format!(
        "{}\n",
        json!({"type": "user", "message": {"role": "user", "content": content}})
    )
}

pub type Steps = Arc<dyn Fn(Step) + Send + Sync>;

/// What the seat's CLI asks the person mid-turn, as Claude Code would in a
/// terminal: whether `tool` may run with `input`, or, for AskUserQuestion,
/// the questions in `input`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Ask {
    pub id: String,
    pub tool: String,
    pub input: Value,
    /// The CLI's own words on what the call does, when it has them.
    pub about: Option<String>,
    /// A connection's tool that asks first (connections.rs): the card
    /// offers "Always allow" too, which sets it to Allow.
    #[serde(default)]
    pub always: bool,
}

/// Puts an ask to the person and waits for them: the input the tool runs
/// with (for questions, holding the answers), or None to refuse it.
pub trait Asker: Send + Sync {
    fn put(&self, ask: Ask) -> Option<Value>;
    /// The CLI took its question back, or the turn ended without answer.
    fn withdraw(&self, id: &str);
}

/// The CLI's question in one stream-json line, when it asks one.
fn ask_of(ev: &Value) -> Option<Ask> {
    let r = &ev["request"];
    (ev["type"] == "control_request" && r["subtype"] == "can_use_tool").then(|| Ask {
        id: ev["request_id"].as_str().unwrap_or_default().to_owned(),
        tool: r["tool_name"].as_str().unwrap_or_default().to_owned(),
        input: r["input"].clone(),
        about: r["description"].as_str().map(str::to_owned),
        always: false,
    })
}

/// Takes back the asks a turn leaves unanswered as it ends.
fn withdraw(asker: &Option<Arc<dyn Asker>>, open: &Mutex<Vec<String>>) {
    if let Some(a) = asker {
        open.lock()
            .unwrap()
            .drain(..)
            .for_each(|id| a.withdraw(&id));
    }
}

/// The CLI's word for an answer: run with this input, or don't.
fn verdict(input: Option<Value>) -> Value {
    match input {
        Some(input) => json!({"behavior": "allow", "updatedInput": input}),
        None => {
            json!({"behavior": "deny", "message": "Refused: the person said no, or no one was there to answer."})
        }
    }
}

/// The most of a tool's result a door is shown; the model reads it all.
pub const STEP_MAX: usize = 16 * 1024;

/// A step as a door takes it: a call's result cut to STEP_MAX, on a
/// character's edge.
pub fn clipped(step: Step) -> Step {
    match step {
        Step::Result {
            id,
            mut text,
            error,
        } => {
            let mut cut = text.len().min(STEP_MAX);
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            Step::Result { id, text, error }
        }
        Step::Progress { id, text } => Step::Progress {
            id,
            text: latest(text),
        },
        Step::Inner { parent, step } => Step::Inner {
            parent,
            step: Box::new(clipped(*step)),
        },
        other => other,
    }
}

/// The last STEP_MAX of a running call's output, on a character's edge:
/// what a command printed last is what a person watches.
fn latest(text: String) -> String {
    let mut from = text.len().saturating_sub(STEP_MAX);
    while !text.is_char_boundary(from) {
        from += 1;
    }
    text[from..].to_owned()
}

/// One step of the work behind an answer.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Step {
    /// The model calls `name` (`mcp__server__tool`) with `input`.
    Call {
        id: String,
        name: String,
        input: Value,
    },
    /// What the call `id` gave back: its text, cut to STEP_MAX.
    Result {
        id: String,
        text: String,
        error: bool,
    },
    /// The whole MCP result of call `id` (`content`, `structuredContent`,
    /// `_meta`), for a card to draw: only from a harness whose stream
    /// carries it, only live, and never kept.
    Structured { id: String, result: Value },
    /// What the running call `id` has given so far, whole each time (a
    /// shell's output as it comes): only live, never kept, and gone once
    /// its Result comes.
    Progress { id: String, text: String },
    /// More of the words being written, as they come.
    Text { text: String },
    /// More of the model's thinking, summarized by the API, as it comes.
    Thinking { text: String },
    /// The next step shown becomes hippo's row `key`: a page's turn record
    /// carries the key before the row exists, so a page that reads the
    /// record and then the history never holds the same thing twice.
    Row { key: String },
    /// Row `key` is settled: hippo has it, or quiet words will never need
    /// it. `heard` are the person's messages it holds; `row` is hippo's
    /// number for it, when hippo took it now rather than from the spool
    /// later, for the page to name it by (a reply's quote, quick replies).
    Stored {
        key: String,
        heard: Vec<String>,
        row: Option<u64>,
    },
    /// A subagent's or the seat's whole answer to one message: a subagent
    /// keeps it in its log, the seat says it at once. Steered runs give one
    /// per message. Doors never see it.
    Answer { text: String },
    /// The seat's Agent call `id` started a subagent, `name` its
    /// description and `task` its prompt.
    Agent {
        id: String,
        name: String,
        task: String,
    },
    /// The subagent of Agent call `id` stopped, with its report.
    Done { id: String, report: String },
    /// The seat's background jobs still running, by their descriptions,
    /// each time one starts or ends.
    Jobs { names: Vec<String> },
    /// The tokens the turn used, when the harness says.
    /// `input` counts every token sent, the cache's two shares among them.
    /// `model` is the one the harness names, when it names one.
    Usage {
        input: u64,
        cache_read: u64,
        cache_write: u64,
        output: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    /// The model and thinking level the harness says this run is on, in
    /// its own words, once a run: `effort` None where the harness doesn't
    /// say (Claude Code's stream names no level). A worker's roster line
    /// and log carry it, and a mismatch with what it was started on is
    /// flagged there.
    Runs {
        model: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        effort: Option<String>,
    },
    /// Live measurements reported by the harness, never estimated from
    /// cumulative billing usage. Kept as display data, outside model context.
    Metrics {
        context_tokens: Option<u64>,
        context_window: Option<u64>,
        tokens_per_second: Option<f64>,
    },
    /// A step of the subagent the seat's Agent call `parent` started: the
    /// CLI puts its work on the seat's stream, tagged, as Claude Code
    /// nests it under the call. It is the subagent's, not the answer's.
    Inner { parent: String, step: Box<Step> },
}

/// The answer in one stream-json line, when it ends a reply that didn't
/// fail.
fn answer(line: &str) -> Option<String> {
    let ev = serde_json::from_str::<Value>(line).ok()?;
    if ev["type"] != "result" || ev["is_error"] == true {
        return None;
    }
    let text = ev["result"].as_str()?.trim();
    said(text).then(|| text.to_owned())
}

/// A seat's whole answer to a background job's news that tells the person
/// nothing new (OpenClaw's convention): asked for an empty reply, the model
/// writes one saying it is silent instead.
pub const QUIET: &str = "NO_REPLY";

/// Whether an answer is one to pass on.
pub fn said(text: &str) -> bool {
    !text.is_empty() && text != QUIET
}

/// The background jobs running, by description, when the line says they
/// changed.
fn background_tasks(line: &str) -> Option<Vec<String>> {
    if !line.contains("background_tasks_changed") {
        return None;
    }
    let ev = serde_json::from_str::<Value>(line).ok()?;
    (ev["subtype"] == "background_tasks_changed").then(|| {
        ev["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                t["description"]
                    .as_str()
                    .unwrap_or("A background job")
                    .to_owned()
            })
            .collect()
    })
}

/// The steps in one stream-json line: tool calls from the model, results
/// from the tools. Claude Code reports results as a user turn.
pub fn steps(line: &str) -> Vec<Step> {
    let Ok(ev) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    let parts = ev["message"]["content"].as_array().into_iter().flatten();
    let s = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    if let Some(parent) = ev["parent_tool_use_id"].as_str() {
        // A subagent's words come whole, never as deltas.
        let words = parts
            .clone()
            .filter(|c| ev["type"] == "assistant" && c["type"] == "text")
            .map(|c| Step::Text {
                text: s(&c["text"]),
            });
        let mut own = vec![];
        if ev["type"] != "stream_event" {
            let mut plain = ev.clone();
            plain["parent_tool_use_id"] = Value::Null;
            own = steps(&plain.to_string());
        }
        return words
            .chain(own)
            .map(|step| Step::Inner {
                parent: parent.to_owned(),
                step: Box::new(step),
            })
            .collect();
    }
    match (ev["type"].as_str(), ev["subtype"].as_str()) {
        (Some("system"), Some("task_started")) if ev["task_type"] == "local_agent" => {
            return vec![Step::Agent {
                id: s(&ev["tool_use_id"]),
                name: s(&ev["description"]),
                task: s(&ev["prompt"]),
            }];
        }
        (Some("system"), Some("task_notification")) if ev["tool_use_id"].is_string() => {
            return vec![Step::Done {
                id: s(&ev["tool_use_id"]),
                report: s(&ev["summary"]),
            }];
        }
        _ => {}
    }
    match ev["type"].as_str() {
        // With --include-partial-messages, the words arrive as deltas
        // before the whole message does.
        Some("stream_event")
            if ev["event"]["type"] == "content_block_delta"
                && ev["event"]["delta"]["type"] == "text_delta" =>
        {
            vec![Step::Text {
                text: s(&ev["event"]["delta"]["text"]),
            }]
        }
        Some("stream_event")
            if ev["event"]["type"] == "content_block_delta"
                && ev["event"]["delta"]["type"] == "thinking_delta" =>
        {
            vec![Step::Thinking {
                text: s(&ev["event"]["delta"]["thinking"]),
            }]
        }
        Some("assistant") => parts
            .filter(|c| c["type"] == "tool_use")
            .map(|c| Step::Call {
                id: s(&c["id"]),
                name: s(&c["name"]),
                input: c["input"].clone(),
            })
            .collect(),
        Some("user") => {
            let mut out: Vec<Step> = parts
                .filter(|c| c["type"] == "tool_result")
                .map(|c| {
                    let text = match &c["content"] {
                        Value::String(t) => t.clone(),
                        Value::Array(a) => a
                            .iter()
                            .filter_map(|p| p["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    };
                    Step::Result {
                        id: s(&c["tool_use_id"]),
                        text,
                        error: c["is_error"] == true,
                    }
                })
                .collect();
            // An MCP tool's whole result rides beside its one result block.
            if let [Step::Result { id, .. }] = &out[..]
                && let Some(result) = structured(&ev["tool_use_result"])
            {
                out.push(Step::Structured {
                    id: id.clone(),
                    result,
                });
            }
            out
        }
        _ => vec![],
    }
}

/// The Claude turns under way that a pick can reach mid-turn: the main
/// chat's as "main", a worker's by its name. Only turns on the person's own
/// picks are here; Temp and turns held to the safe mode keep what they began
/// with.
static LIVE: Mutex<Vec<(String, Weak<Steer>)>> = Mutex::new(Vec::new());

/// A turn's place in LIVE, given up when the turn ends.
struct Live(String, Weak<Steer>);

impl Live {
    fn enter(chat: String, steer: &Arc<Steer>) -> Self {
        let steer = Arc::downgrade(steer);
        let mut live = LIVE.lock().unwrap();
        live.retain(|(c, _)| *c != chat);
        live.push((chat.clone(), steer.clone()));
        Self(chat, steer)
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        LIVE.lock()
            .unwrap()
            .retain(|(c, s)| !(*c == self.0 && s.ptr_eq(&self.1)));
    }
}

/// Hands a pick made mid-turn to `chat`'s Claude turn under way, which
/// takes it at the model's next request: a model, a permission mode, or
/// both. False when no such turn runs, and the pick waits for the next.
/// A sandbox can't change mid-turn, so it waits. Full access
/// (bypassPermissions) is reachable because every acting turn starts with
/// --allow-dangerously-skip-permissions.
/// Stops the task a Claude turn under way started with call `call` (a
/// subagent, a background shell), leaving the turn to go on. False when no
/// turn under way holds it.
pub fn stop_task(call: &str) -> bool {
    let live: Vec<Arc<Steer>> = LIVE
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(_, s)| s.upgrade())
        .collect();
    live.iter().any(|steer| {
        let task = steer.tasks.lock().unwrap().get(call).cloned();
        task.is_some_and(|task| steer.control(json!({"subtype": "stop_task", "task_id": task})))
    })
}

pub fn switch(chat: &str, model: Option<&str>, mode: Option<&str>) -> bool {
    let steer = LIVE
        .lock()
        .unwrap()
        .iter()
        .find(|(c, _)| c == chat)
        .and_then(|(_, s)| s.upgrade());
    let Some(steer) = steer else {
        return false;
    };
    let mut sent = false;
    if let Some(model) = model {
        sent |= steer.control(json!({"subtype": "set_model", "model": model}));
    }
    if let Some(mode) = mode {
        sent |= steer.control(json!({"subtype": "set_permission_mode", "mode": mode}));
    }
    sent
}

/// The oldest Claude Code Libri runs: the flags and control requests it
/// uses are all there.
const CLAUDE_OLDEST: [u32; 3] = [2, 1, 303];

/// The newest Claude Code the harness gate passed on. A newer one runs, as
/// the CLI updates itself and its control protocol has only grown so far,
/// but the journal says so once.
const CLAUDE_TESTED: [u32; 3] = [2, 1, 312];

/// The version a CLI's init line names, when the line is one.
fn claude_version(line: &str) -> Option<String> {
    if !line.contains("claude_code_version") {
        return None;
    }
    let ev = serde_json::from_str::<Value>(line).ok()?;
    (ev["subtype"] == "init").then(|| ev["claude_code_version"].as_str().map(str::to_owned))?
}

/// What to say of a CLI at `version`: Some((too_old, words)) when it is
/// older than Libri runs or newer than it was tested on.
fn version_note(version: &str) -> Option<(bool, String)> {
    let parts: Vec<u32> = version
        .split(|c: char| !c.is_ascii_digit())
        .take(3)
        .map(|p| p.parse().unwrap_or(0))
        .collect();
    let dotted = |v: [u32; 3]| format!("{}.{}.{}", v[0], v[1], v[2]);
    if parts.as_slice() < CLAUDE_OLDEST.as_slice() {
        Some((
            true,
            format!(
                "Claude Code {version} is older than {}, the oldest Libri runs: update it (`claude update`).",
                dotted(CLAUDE_OLDEST)
            ),
        ))
    } else if parts.as_slice() > CLAUDE_TESTED.as_slice() {
        Some((
            false,
            format!(
                "Claude Code {version} is newer than {}, the last Libri was tested on.",
                dotted(CLAUDE_TESTED)
            ),
        ))
    } else {
        None
    }
}

/// The last version the journal was told of, so a newer CLI is noted once.
static NOTED: Mutex<Option<String>> = Mutex::new(None);

/// The most of a tool's whole result a card is given live: past it, the
/// card has the text, as after a reload.
const STRUCTURED_MAX: usize = 256 * 1024;

/// An MCP tool's whole result as a harness reports it (Claude Code's
/// `tool_use_result`, Codex's `result`), in the MCP's own field names;
/// None for any other tool's, or one too large to pass on.
pub fn structured(r: &Value) -> Option<Value> {
    let data = r
        .get("structuredContent")
        .or_else(|| r.get("structured_content"));
    let meta = r.get("_meta");
    if data.is_none() && meta.is_none() {
        return None;
    }
    let mut out = serde_json::Map::new();
    if let Some(c) = r.get("content").filter(|c| c.is_array()) {
        out.insert("content".into(), c.clone());
    }
    if let Some(d) = data {
        out.insert("structuredContent".into(), d.clone());
    }
    if let Some(m) = meta {
        out.insert("_meta".into(), m.clone());
    }
    let out = Value::Object(out);
    (out.to_string().len() <= STRUCTURED_MAX).then_some(out)
}

/// Each call stands alone: Claude Code from its bare state, so no setting,
/// rule, hook or server of whoever's home it runs in reaches a turn, and no
/// session lands where a watching hippo would read it twice. What it needs
/// comes from Libri, as for every harness; the Agent SDK takes the same
/// stance with `settingSources: []`.
pub struct Claude {
    pub command: String,
    pub model: String,
    pub effort: Option<String>,
    /// With its own tools (shell, files) and Claude Code's prompt, which a
    /// person's coding host wants; off (`harness.coding = false`) for an
    /// assistant that has only Libri's.
    pub coding: bool,
}

/// Claude Code's own timers, which end with the session, as each turn
/// does: Libri's reminders outlive it.
const TIMERS: &str = "CronCreate,CronDelete,CronList,ScheduleWakeup,RemoteTrigger";

/// How long what a CLI left behind gets to let go of its output.
const OUTPUT_GRACE: Duration = Duration::from_secs(2);

/// Whether the reader of a CLI's output ended. Only time spent `reading`
/// counts: held up by its steps (a slow hippo, itself bounded) it still has
/// lines to take in, and that is no pipe held open.
fn read_out<T>(reader: &thread::JoinHandle<T>, reading: &AtomicBool, grace: Duration) -> bool {
    let mut given = Instant::now() + grace;
    while !reader.is_finished() && Instant::now() < given {
        if !reading.load(Ordering::Relaxed) {
            given = Instant::now() + grace;
        }
        thread::sleep(Duration::from_millis(20));
    }
    reader.is_finished()
}

/// Whether `thread` ended within `grace`.
fn ended<T>(thread: &thread::JoinHandle<T>, grace: Duration) -> bool {
    let given = Instant::now() + grace;
    while !thread.is_finished() && Instant::now() < given {
        thread::sleep(Duration::from_millis(20));
    }
    thread.is_finished()
}

/// The harness's own settings for a turn: its memory off, as hippo is the
/// only one.
fn claude_settings(sandbox: Option<&str>, compact: u64) -> String {
    // failIfUnavailable: a pick of the sandbox on a host without bubblewrap
    // ends the turn saying so, rather than running unsandboxed unsaid.
    let sandbox = match sandbox {
        Some("auto-allow") => json!({"enabled": true, "failIfUnavailable": true}),
        Some("regular") => json!({"enabled": true, "failIfUnavailable": true,
            "autoAllowBashIfSandboxed": false}),
        _ => json!({"enabled": false}),
    };
    // autoCompactWindow: the context size Claude Code compacts its session
    // at, in place of its model's whole window (Settings, compact.at).
    json!({"autoMemoryEnabled": false, "sandbox": sandbox, "autoCompactWindow": compact})
        .to_string()
}

/// Where Claude Code keeps its own, by its own rule.
pub(crate) fn claude_home() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".claude")))
        .unwrap_or_default()
}

/// A session of the CLI's whose folders go when this does, however the turn
/// ends.
struct Session {
    id: String,
    home: PathBuf,
}

impl Session {
    fn new() -> Self {
        Self {
            id: uuid(),
            home: claude_home(),
        }
    }
}

/// A random UUID, the form the CLI's session ids take.
fn uuid() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the system's random numbers");
    // The version and variant of a random UUID, which the CLI requires.
    bytes[6] = bytes[6] & 0x0f | 0x40;
    bytes[8] = bytes[8] & 0x3f | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    [
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..],
    ]
    .join("-")
}

/// A turn that goes on with its chat's session: where the record is, whose
/// it is, how the session starts, and the id it runs under.
struct Resumed<'a> {
    state: &'a Path,
    chat: &'a str,
    start: Start,
    id: String,
}

/// What the CLI says, and exits on, when the session to resume is gone: it
/// keeps its logs for 30 days unless told otherwise.
const LOST_SESSION: &str = "No conversation found with session ID";
/// What the API answers a session past its context window, which the CLI
/// compacts before then; should that fail, the chat starts a new one.
const FULL_SESSION: &str = "Prompt is too long";

impl Drop for Session {
    fn drop(&mut self) {
        let projects = std::fs::read_dir(self.home.join("projects"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|project| project.path());
        for place in projects.chain([self.home.join("session-env")]) {
            let kept = place.join(&self.id);
            if kept.exists()
                && let Err(e) = std::fs::remove_dir_all(&kept)
            {
                eprintln!("libri: {}: {e}", kept.display());
            }
        }
    }
}

impl Harness for Claude {
    /// A turn of a chat that keeps a session (`Run::session`) goes on with
    /// it: `memory` reaches only a new session, as the one resumed holds the
    /// thread, and the CLI compacts it as it would at a terminal. A session
    /// the CLI no longer has starts again, new.
    fn answer(
        &self,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        run: Run,
    ) -> Result<String, String> {
        let state = sessions::state();
        let chat = run.session.clone().filter(|_| !run.plain && !run.ephemeral);
        let (Some(state), Some(chat)) = (state, chat) else {
            return self.turn(system, memory, prompt, files, run, None);
        };
        let start = sessions::start(&state, &chat, run.fork.as_deref(), HARNESS, uuid);
        let kept = |start: Start| {
            let id = match &start {
                Start::Resume(id) | Start::New(id) => id.clone(),
                Start::Fork(_) => uuid(),
            };
            Resumed {
                state: &state,
                chat: &chat,
                start,
                id,
            }
        };
        let gone_on = start.holds_thread();
        match self.turn(
            system,
            memory,
            prompt,
            files,
            run.clone(),
            Some(kept(start)),
        ) {
            Err(e) if gone_on && (e.contains(LOST_SESSION) || e.contains(FULL_SESSION)) => {
                eprintln!(
                    "libri: {chat}: claude can't go on with its session ({e}); starting a new one"
                );
                sessions::forget(&state, &chat, HARNESS)?;
                self.turn(
                    system,
                    memory,
                    prompt,
                    files,
                    run,
                    Some(kept(Start::New(uuid()))),
                )
            }
            r => r,
        }
    }

    fn tools(&self) -> bool {
        true
    }
}

/// This harness's name in the session records.
const HARNESS: &str = "claude";

impl Claude {
    fn turn(
        &self,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        run: Run,
        kept: Option<Resumed>,
    ) -> Result<String, String> {
        let model = run
            .model_for("claude")
            .unwrap_or_else(|| self.model.clone());
        let effort = run.effort().or_else(|| self.effort.clone());
        let grant = run.access("claude");
        let acting = self.coding && !run.plain;
        let mut cmd = Command::new(&self.command);
        if let Some(dir) = std::env::var_os("LIBRI_WORKDIR").filter(|d| !d.is_empty()) {
            cmd.current_dir(dir);
        }
        run.temp(&mut cmd);
        // One-hour cache entries, pinned rather than left to Claude Code's
        // default: the memory's mark must match the lifetime of Claude
        // Code's own marks, or the API refuses the call. A plain call's
        // come seconds apart, where five minutes cost less to write.
        let caching = if run.plain {
            "FORCE_PROMPT_CACHING_5M"
        } else {
            "ENABLE_PROMPT_CACHING_1H"
        };
        cmd.env(caching, "1");
        // Stream-json is the only way to hand the CLI images and PDFs; it
        // needs stream-json out, which ends with the result event.
        cmd.args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
        ]);
        // A plain call is one model call: nothing of the home or the working
        // directory may reach it. A turn is the person's Claude Code, with
        // their settings, skills, plugins and repository instructions.
        if run.plain {
            cmd.arg("--safe-mode");
        }
        // Nothing of the scratchpad stays with the CLI, which keeps the
        // results too long to show under the session it runs as.
        let forgotten = (kept.is_none() && run.ephemeral).then(Session::new);
        if let Some(session) = &forgotten {
            cmd.args(["--session-id", &session.id]);
        }
        match &kept {
            Some(Resumed {
                start: Start::Resume(id),
                ..
            }) => cmd.args(["--resume", id]),
            Some(Resumed {
                start: Start::Fork(from),
                id,
                ..
            }) => cmd.args(["--resume", from, "--fork-session", "--session-id", id]),
            Some(Resumed { id, .. }) => cmd.args(["--session-id", id]),
            None => cmd.arg("--no-session-persistence"),
        };
        cmd.args([
            "--settings",
            // Sandboxes Bash, which only a coding turn has.
            &claude_settings(
                acting.then_some(grant.sandbox.as_deref()).flatten(),
                crate::sessions::compact_now(),
            ),
            "--model",
            &model,
        ]);
        if acting {
            // A memory-off turn has the web alone: Claude Code's own tools
            // would read the person's files with the rest.
            if Scope::of(&run) == Some(Scope::Memoryless) {
                cmd.args(["--tools", ""]);
            }
            cmd.args([
                "--disallowedTools",
                TIMERS,
                "--permission-mode",
                &grant.mode,
                // Lets bypass be switched to, as the picker offers it; it
                // turns nothing on, and without it the CLI refuses the
                // switch from any other mode.
                "--allow-dangerously-skip-permissions",
                // What it would ask in a terminal comes as control
                // requests on stdout, answered on stdin (as the Agent SDK's
                // canUseTool), which the page shows as cards.
                "--permission-prompt-tool",
                "stdio",
                "--append-system-prompt",
                system,
            ]);
        } else {
            cmd.args(["--system-prompt", system, "--tools", ""]);
        }
        // A turn keeps the person's own servers, as T3 Code does; one that
        // may not act keeps to Libri's list, so what it was refused does not
        // come back through a server of theirs.
        if !matches!(
            Scope::of(&run),
            Some(Scope::Person | Scope::Agent | Scope::Scratchpad | Scope::Apart)
        ) {
            cmd.arg("--strict-mcp-config");
        }
        if let Some(list) = servers::of_run(&run)? {
            let allowed: Vec<String> = list.keys().map(|s| format!("mcp__{s}")).collect();
            cmd.args([
                "--mcp-config",
                &servers::claude(&list),
                "--allowedTools",
                &allowed.join(","),
            ]);
        }
        if let Some(effort) = &effort {
            cmd.args(["--effort", effort]);
        }
        // The CLI omits thinking unless asked; summarized, the page shows
        // it as Paseo does. The flag is hidden in `--help`.
        cmd.args([
            "--include-partial-messages",
            "--thinking-display",
            "summarized",
        ]);
        // A group of its own, so a stop ends all of it: a wrapper's claude
        // (bwrap forks it) and the shells it starts would otherwise live on,
        // holding its output open and the turn with it.
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        crate::cgroup::harness(&mut cmd);
        // The id the session runs under is the chat's before the CLI starts,
        // so the memory's reader, following the CLI's logs, files its first
        // words as Libri's; `put` makes it the chat's own once it answers.
        if let Some(k) = &kept
            && !matches!(k.start, Start::Resume(_))
        {
            sessions::put_next(k.state, k.chat, HARNESS, &k.id)?;
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{}: {e}", self.command))?;
        run.runs_in(child.id());
        let holds_thread = kept.as_ref().is_some_and(|k| k.start.holds_thread());
        let memory = if holds_thread { "" } else { memory };
        let mut content = cached(memory, if run.plain { None } else { Some("1h") });
        content.extend(files.iter().map(block));
        content.push(json!({"type": "text", "text": prompt}));
        let line = user(content);
        let mut stdin = child.stdin.take().unwrap();
        // The input stays open for answers to the CLI's questions, even
        // when no one's words can steer it.
        let run_steer = run.steer.clone().or_else(|| self.coding.then(Arc::default));
        // A turn on the person's own picks takes a new one mid-turn; Temp
        // runs on picks of its own.
        let chat = run.sub.clone().unwrap_or_else(|| "main".into());
        let on_picks = acting && run.acts && !run.ephemeral && !run.memory_off;
        let _live = run_steer
            .as_ref()
            .filter(|_| on_picks)
            .map(|s| Live::enter(chat, s));
        // Written from a thread, as the files can outgrow the pipe; then,
        // with a steer, the input stays open for the person's next words.
        let steer = run_steer.clone();
        let writer = thread::spawn(move || {
            stdin.write_all(line.as_bytes())?;
            if let Some(s) = steer {
                s.open(stdin);
            }
            Ok::<_, std::io::Error>(())
        });
        let stdout = child.stdout.take().unwrap();
        // Weak: a reader left waiting on a held pipe must not keep the page's
        // end of the turn open after the turn has gone on.
        let steps = run.steps.as_ref().map(Arc::downgrade);
        let steer = run_steer.clone();
        let asker = run.ask.clone();
        // The asks still waiting on the person, withdrawn when the turn ends.
        let open: Arc<Mutex<Vec<String>>> = Arc::default();
        let asking = open.clone();
        let output: Arc<Mutex<String>> = Arc::default();
        let read = output.clone();
        let reading = Arc::new(AtomicBool::new(true));
        let waiting = reading.clone();
        // Apart, as `steps` above: a reader left behind has no hold on the turn.
        let hearing = Run {
            monitor: run.monitor.clone(),
            steer: run_steer.clone(),
            ..Run::default()
        };
        let reader = thread::spawn(move || -> std::io::Result<()> {
            // The seat's background jobs still running: the CLI wakes the
            // model when each ends, and answers again.
            let mut background = 0;
            for line in BufReader::new(stdout).lines() {
                let line = line?;
                waiting.store(false, Ordering::Relaxed);
                hearing.heard();
                let steps = steps.as_ref().and_then(Weak::upgrade);
                if let Some((old, words)) = claude_version(&line).and_then(|v| version_note(&v)) {
                    let mut noted = NOTED.lock().unwrap();
                    if old || noted.as_deref() != Some(words.as_str()) {
                        eprintln!("libri: {words}");
                        *noted = Some(words.clone());
                        if old && let Some(m) = &hearing.monitor {
                            m.tell(format!("({words})"));
                        }
                    }
                }
                if let Some(names) = background_tasks(&line) {
                    background = names.len();
                    if let Some(show) = &steps {
                        show(Step::Jobs { names });
                    }
                }
                if let (Some(model), Some(show)) = (init_model(&line), &steps) {
                    show(Step::Runs {
                        model,
                        effort: None,
                    });
                }
                if let (Some(names), Some(state)) =
                    (crate::commands::claude_init(&line), sessions::state())
                    && let Err(e) = crate::commands::record(&state, HARNESS, &names)
                {
                    eprintln!("libri: {e}");
                }
                let failed = failed_servers(&line);
                if !failed.is_empty()
                    && let Some(m) = &hearing.monitor
                {
                    m.tell(format!(
                        "(This turn runs without its tools: {} failed to start.)",
                        failed.join(", ")
                    ));
                }
                if line.contains("control_") {
                    let ev = serde_json::from_str::<Value>(&line).unwrap_or_default();
                    if let (Some(ask), Some(s)) = (ask_of(&ev), &steer) {
                        let (s, asker, asking) = (s.clone(), asker.clone(), asking.clone());
                        // Each waits on the person in a thread of its own, so
                        // the stream reads on meanwhile.
                        thread::spawn(move || {
                            let id = ask.id.clone();
                            asking.lock().unwrap().push(id.clone());
                            let input = asker.and_then(|a| a.put(ask));
                            asking.lock().unwrap().retain(|a| *a != id);
                            s.respond(&id, verdict(input));
                        });
                    } else if ev["type"] == "control_cancel_request"
                        && let (Some(a), Some(id)) = (&asker, ev["request_id"].as_str())
                    {
                        a.withdraw(id);
                    } else if let Some(s) = &steer {
                        match ev["type"].as_str() {
                            Some("system") => s.task_started(&ev),
                            Some("control_response") => s.answered(&ev),
                            Some("control_request") => s.refuse(
                                ev["request_id"].as_str().unwrap_or_default(),
                                ev["request"]["subtype"].as_str().unwrap_or_default(),
                            ),
                            _ => {}
                        }
                    }
                }
                if let Some(show) = &steps {
                    self::steps(&line).into_iter().for_each(|s| show(s));
                }
                // Words that came after the answer was out wait for the
                // next turn; ones already written the CLI answers next. The
                // turn stays open while its background jobs run, so the
                // person talks to it meanwhile, as in a Claude Code session.
                if let Some(s) = &steer
                    && background == 0
                    && line.contains(r#""result""#)
                    && serde_json::from_str::<Value>(&line)
                        .is_ok_and(|ev| ev.get("type").and_then(Value::as_str) == Some("result"))
                {
                    s.close();
                }
                if let Some(show) = &steps
                    && let Some(text) = answer(&line)
                {
                    show(Step::Answer { text });
                }
                let mut out = read.lock().unwrap();
                out.push_str(&line);
                out.push('\n');
                waiting.store(true, Ordering::Relaxed);
            }
            Ok(())
        });
        let errors = drain(child.stderr.take().unwrap());
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            let stopped = run.stop.as_ref().is_some_and(|s| s.load(Ordering::Relaxed));
            if stopped {
                if let Some(s) = &run_steer {
                    s.close();
                }
                withdraw(&run.ask, &open);
                // Asked first, so the CLI ends the background shells it
                // started in sessions of their own; then made to.
                let group = child.id() as i32;
                // SAFETY: killpg only sends a signal, to the group this child leads.
                unsafe { libc::killpg(group, libc::SIGTERM) };
                let given = Instant::now() + Duration::from_secs(3);
                while child.try_wait().ok().flatten().is_none() && Instant::now() < given {
                    thread::sleep(Duration::from_millis(50));
                }
                // SAFETY: as above.
                unsafe { libc::killpg(group, libc::SIGKILL) };
                let _ = child.kill();
                let _ = child.wait();
                return Err("stopped".into());
            }
            thread::sleep(Duration::from_millis(100));
        };
        ended(&writer, OUTPUT_GRACE);
        if let Some(s) = &run_steer {
            s.close();
        }
        withdraw(&run.ask, &open);
        // The CLI is gone, but a shell it left can hold its output open for
        // as long as it runs, and the turn with it, past Stop's reach.
        if !read_out(&reader, &reading, OUTPUT_GRACE) {
            // SAFETY: killpg only sends a signal, to the group this child led.
            unsafe { libc::killpg(child.id() as i32, libc::SIGKILL) };
            if !read_out(&reader, &reading, OUTPUT_GRACE) {
                eprintln!("libri: claude's output held open after it exited");
            }
        }
        if reader.is_finished() {
            reader.join().unwrap().map_err(|e| e.to_string())?;
        }
        let out = std::mem::take(&mut *output.lock().unwrap());
        // Words that reached the CLI just as an answer ended get one of
        // their own: each result is an answer, the last one holding the
        // call's usage.
        let results: Vec<Value> = out
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|ev| ev.get("type").and_then(Value::as_str) == Some("result"))
            .collect();
        let answers: Vec<&str> = results
            .iter()
            .filter(|ev| ev.get("is_error") != Some(&Value::Bool(true)))
            .filter_map(|ev| ev.get("result").and_then(Value::as_str))
            .map(str::trim)
            .filter(|t| said(t))
            .collect();
        // A subagent reports only its last answer, as in Unii: words sent
        // while it works replace what it said before, and its log keeps
        // every answer for whoever reads the whole chat.
        let answers = if run.sub.is_some() {
            answers.last().copied().unwrap_or_default().to_owned()
        } else {
            answers.join("\n\n")
        };
        // The session is the CLI's once it has answered: from here the
        // chat's next turn goes on with it.
        if let Some(k) = &kept
            && !results.is_empty()
        {
            sessions::put(k.state, k.chat, HARNESS, &k.id)?;
        }
        let ev = match results.last() {
            Some(ev) => ev,
            None => {
                let err = if ended(&errors, OUTPUT_GRACE) {
                    errors.join().unwrap_or_default()
                } else {
                    String::new()
                };
                return Err(format!("claude exited {status}: {}", tail(&err)));
            }
        };
        // One journal line per model per call, where `usage` counts each
        // assistant's share of the subscriptions.
        let models = ev.get("modelUsage").and_then(Value::as_object);
        for (model, u) in models.into_iter().flatten() {
            let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            eprintln!(
                "libri: tokens model={model} input={} cache_read={} cache_write={} output={}",
                n("inputTokens"),
                n("cacheReadInputTokens"),
                n("cacheCreationInputTokens"),
                n("outputTokens"),
            );
            if let Some(show) = &run.steps {
                show(Step::Usage {
                    input: n("inputTokens")
                        + n("cacheReadInputTokens")
                        + n("cacheCreationInputTokens"),
                    cache_read: n("cacheReadInputTokens"),
                    cache_write: n("cacheCreationInputTokens"),
                    output: n("outputTokens"),
                    model: Some(model.clone()),
                });
            }
        }
        let text = ev.get("result").and_then(Value::as_str).unwrap_or("");
        if ev.get("is_error") == Some(&Value::Bool(true)) && answers.is_empty() {
            // Whole: a safeguard's refusal says so in its first words.
            return Err(format!("claude: {}", text.trim()));
        }
        Ok(answers)
    }
}

/// Lines per content block of the memory. The API looks back up to 20
/// blocks from a cache mark for an entry an earlier call wrote, so the next
/// call, its memory grown by a few lines, still finds this one's. The same
/// scheme as hippo's compactor.
const BLOCK: usize = 4;

/// What hippo shows for a line the compactor has not written yet.
const PLACEHOLDER: &str = "(not summarized yet: zoom it)";

/// The memory as content blocks of `BLOCK` lines, the last whole block of
/// its settled part marked for the cache. Lines from the first one not yet
/// summarized on, and hippo's note after `</chat>`, change before the next
/// call, so the mark stays ahead of them. The API allows four marks and
/// Claude Code uses three. Through a base URL it uses all four, and a fifth
/// would fail every turn with a tool call, so the memory goes unmarked there.
fn cached(memory: &str, ttl: Option<&str>) -> Vec<Value> {
    let lines: Vec<&str> = memory.split_inclusive('\n').collect();
    let settled = lines
        .iter()
        .position(|l| l.contains(PLACEHOLDER) || l.starts_with("</chat>"))
        .unwrap_or(lines.len());
    let mark = match std::env::var_os("ANTHROPIC_BASE_URL") {
        None => settled / BLOCK,
        Some(_) => 0,
    };
    lines
        .chunks(BLOCK)
        .enumerate()
        .map(|(n, chunk)| {
            let text = chunk.concat();
            if n + 1 == mark {
                let mut control = json!({"type": "ephemeral"});
                if let Some(ttl) = ttl {
                    control["ttl"] = ttl.into();
                }
                json!({"type": "text", "text": text, "cache_control": control})
            } else {
                json!({"type": "text", "text": text})
            }
        })
        .collect()
}

/// A photo or PDF as a content block of a user message.
fn block(f: &Attachment) -> Value {
    match f {
        Attachment::Image { mime, data } => json!({"type": "image",
                "source": {"type": "base64", "media_type": mime, "data": B64.encode(data)}}),
        Attachment::Pdf(data) => json!({"type": "document",
                "source": {"type": "base64", "media_type": "application/pdf",
                           "data": B64.encode(data)}}),
    }
}

/// Any OpenAI-compatible chat completions endpoint over plain HTTP.
pub struct Http {
    pub url: String,
    pub model: String,
}

impl Harness for Http {
    fn answer(
        &self,
        system: &str,
        memory: &str,
        prompt: &str,
        files: &[Attachment],
        _run: Run,
    ) -> Result<String, String> {
        let url = format!("{}/chat/completions", self.url.trim_end_matches('/'));
        let mut content = vec![json!({"type": "text", "text": memory})];
        for f in files {
            match f {
                Attachment::Image { mime, data } => content.push(json!({"type": "image_url",
                    "image_url": {"url": format!("data:{mime};base64,{}", B64.encode(data))}})),
                Attachment::Pdf(_) => return Err("this model can't read PDFs".into()),
            }
        }
        content.push(json!({"type": "text", "text": prompt}));
        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": content},
            ],
        });
        let mut resp = ureq::post(&url)
            .config()
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .build()
            .send_json(&body)
            .map_err(|e| format!("{url}: {e}"))?;
        let status = resp.status().as_u16();
        let v: Value = resp
            .body_mut()
            .read_json()
            .map_err(|e| format!("{url}: {e}"))?;
        if status >= 400 {
            return Err(format!("{url}: {status} {v}"));
        }
        v.pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(|t| t.trim().to_owned())
            .ok_or_else(|| format!("{url}: no content in {v}"))
    }
}

/// Reads a pipe to its end on its own thread, so a chatty child never
/// blocks on a full pipe, and keeps the last few kilobytes.
fn drain(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<String> {
    thread::spawn(move || {
        let (mut kept, mut chunk) = (Vec::new(), [0u8; 4096]);
        while let Ok(n @ 1..) = pipe.read(&mut chunk) {
            kept.extend_from_slice(&chunk[..n]);
            let over = kept.len().saturating_sub(8192);
            kept.drain(..over);
        }
        String::from_utf8_lossy(&kept).into_owned()
    })
}

fn tail(s: &str) -> String {
    let s = s.trim();
    let start = s.char_indices().rev().nth(299).map_or(0, |(i, _)| i);
    s[start..].to_owned()
}

#[cfg(test)]
mod tests {

    #[test]
    fn claude_is_told_where_to_compact() {
        let v: serde_json::Value =
            serde_json::from_str(&super::claude_settings(None, 200_000)).unwrap();
        assert_eq!(v["autoCompactWindow"], 200_000);
        assert_eq!(v["autoMemoryEnabled"], false);
    }

    use super::*;

    #[test]
    fn a_harness_takes_words_while_its_turn_is_open() {
        let steer = Steer::default();
        assert!(!steer.send("early", &[]));
        let (tx, rx) = std::sync::mpsc::channel();
        steer.take_words(Box::new(move |text, _| tx.send(text.to_owned()).is_ok()));
        assert!(steer.running());
        assert!(steer.send("hello", &[]));
        assert_eq!(rx.recv().unwrap(), "hello");
        steer.close();
        assert!(!steer.running());
        assert!(!steer.send("late", &[]));
    }

    #[test]
    fn a_harness_whose_way_in_blocks_leaves_the_turn_free() {
        let steer = Arc::new(Steer::default());
        let (release, blocked) = std::sync::mpsc::channel::<()>();
        steer.take_words(Box::new(move |_, _| {
            let _ = blocked.recv();
            true
        }));
        let (done, finished) = std::sync::mpsc::channel();
        let s = steer.clone();
        thread::spawn(move || {
            s.send("one", &[]);
            s.send("two", &[]);
            s.close();
            let _ = done.send(());
        });
        // Stop needs this lock while the words are on their way.
        assert!(
            finished.recv_timeout(Duration::from_secs(5)).is_ok(),
            "the turn stayed blocked on the harness"
        );
        drop(release);
    }

    #[test]
    fn a_door_is_shown_a_result_cut_and_a_scratchpad_keeps_it_whole() {
        struct Long;
        impl Harness for Long {
            fn answer(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &[Attachment],
                run: Run,
            ) -> Result<String, String> {
                let result = Step::Result {
                    id: "t".into(),
                    text: format!("{}é", "x".repeat(STEP_MAX - 1)),
                    error: false,
                };
                let show = run.steps.unwrap();
                show(result.clone());
                show(Step::Inner {
                    parent: "p".into(),
                    step: Box::new(result),
                });
                Ok(String::new())
            }
        }
        let lengths = |ephemeral: bool| {
            let got = Arc::new(Mutex::new(Vec::new()));
            let into = got.clone();
            let steps: Steps = Arc::new(move |s| {
                let s = match s {
                    Step::Inner { step, .. } => *step,
                    s => s,
                };
                if let Step::Result { text, .. } = s {
                    into.lock().unwrap().push(text.len());
                }
            });
            let run = Run {
                ephemeral,
                steps: Some(steps),
                ..Run::default()
            };
            Clipping(Box::new(Long))
                .answer("", "", "", &[], run)
                .unwrap();
            std::mem::take(&mut *got.lock().unwrap())
        };
        assert_eq!(lengths(false), [STEP_MAX - 1, STEP_MAX - 1]);
        assert_eq!(lengths(true), [STEP_MAX + 1, STEP_MAX + 1]);
    }

    #[test]
    fn what_the_cli_keeps_of_a_session_goes_with_it() {
        let home = std::env::temp_dir().join(format!("libri-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let session = Session {
            id: "one".into(),
            home: home.clone(),
        };
        let kept = home.join("projects/a-repo/one/tool-results");
        let other = home.join("projects/a-repo/two/tool-results");
        std::fs::create_dir_all(&kept).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::create_dir_all(home.join("session-env/one")).unwrap();
        drop(session);
        assert!(!home.join("projects/a-repo/one").exists());
        assert!(!home.join("session-env/one").exists());
        assert!(other.exists());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_random_session_id_is_a_uuid() {
        let id = Session::new().id.clone();
        let parts: Vec<&str> = id.split('-').collect();
        assert_eq!(
            parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12]
        );
        assert!(parts[2].starts_with('4'));
        assert!("89ab".contains(&parts[3][..1]));
    }

    #[test]
    fn says_which_tool_servers_failed_to_start() {
        let init = r#"{"type":"system","subtype":"init","mcp_servers":[{"name":"libri","status":"failed"},{"name":"web","status":"connected"}]}"#;
        assert_eq!(failed_servers(init), ["libri"]);
        assert_eq!(
            init_model(r#"{"type":"system","subtype":"init","model":"claude-fable-5-2"}"#)
                .as_deref(),
            Some("claude-fable-5-2")
        );
        assert_eq!(
            init_model(r#"{"type":"assistant","model":"x","init":1}"#),
            None
        );
        assert!(
            failed_servers(r#"{"type":"system","subtype":"init","mcp_servers":[]}"#).is_empty()
        );
    }

    #[test]
    fn words_to_a_harness_not_reading_leave_the_turn_free() {
        let mut child = Command::new("sleep")
            .arg("20")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let steer = Arc::new(Steer::default());
        steer.open(child.stdin.take().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let s = steer.clone();
        thread::spawn(move || {
            let words = "x".repeat(200_000);
            let taken = s.send(&words, &[]) && s.send(&words, &[]);
            s.close();
            let _ = tx.send(taken);
        });
        let taken = rx.recv_timeout(Duration::from_secs(3));
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(
            taken,
            Ok(true),
            "a full pipe held up the words and the stop"
        );
    }

    /// A `claude` that leaves `shell` running and answers "kept".
    fn claude_leaving(name: &str, shell: &str) -> (Claude, std::path::PathBuf) {
        let script = std::env::temp_dir().join(format!("libri-{name}-{}", std::process::id()));
        let text = format!(
            "#!/bin/sh\n{shell}\necho '{{\"type\":\"result\",\"result\":\"kept\"}}'\nexit 0\n"
        );
        // Written by a child, never opened for writing here: a process
        // another test thread forks while this one held the file open would
        // keep it open for writing until its exec, and running the script
        // then fails with ETXTBSY, as it did on a loaded builder.
        let mut writer = std::process::Command::new("/bin/sh")
            .args(["-c", "cat > \"$0\" && chmod 755 \"$0\""])
            .arg(&script)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        writer
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
        assert!(writer.wait().unwrap().success());
        let claude = Claude {
            command: script.display().to_string(),
            model: "stand-in".into(),
            effort: None,
            coding: false,
        };
        // SAFETY: no other test reads it while it matters to them.
        unsafe { std::env::set_var("STATE_DIRECTORY", std::env::temp_dir()) };
        (claude, script)
    }

    #[test]
    fn an_acting_turn_can_reach_full_access_without_starting_in_it() {
        let said = std::env::temp_dir().join(format!("libri-flags-said-{}", std::process::id()));
        let shell = format!("printf '%s\\n' \"$@\" > '{}'", said.display());
        let (mut claude, script) = claude_leaving("flags", &shell);
        claude.coding = true;
        let reply = claude.answer("", "", "words", &[], Run::default());
        std::fs::remove_file(&script).unwrap();
        let args = std::fs::read_to_string(&said).unwrap();
        std::fs::remove_file(&said).unwrap();
        assert_eq!(reply.as_deref(), Ok("kept"));
        let args: Vec<&str> = args.lines().collect();
        assert!(args.contains(&"--allow-dangerously-skip-permissions"));
        // Reachable only: the turn still runs on the mode it was given.
        assert!(!args.contains(&"--dangerously-skip-permissions"));
        assert!(args.contains(&"--permission-mode"));
    }

    #[test]
    fn a_shell_left_holding_the_output_does_not_hold_the_turn() {
        let (claude, script) = claude_leaving("held", "sleep 20 &");
        let began = Instant::now();
        let reply = claude.answer("", "", "words", &[], Run::default());
        std::fs::remove_file(&script).unwrap();
        assert_eq!(reply.as_deref(), Ok("kept"));
        assert!(began.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_slow_step_after_the_exit_keeps_the_answer() {
        let (claude, script) = claude_leaving("slow", ":");
        let steps: Steps = Arc::new(|s| {
            if matches!(s, Step::Answer { .. }) {
                thread::sleep(OUTPUT_GRACE * 3);
            }
        });
        let run = Run {
            steps: Some(steps),
            ..Run::default()
        };
        let reply = claude.answer("", "", "words", &[], run);
        std::fs::remove_file(&script).unwrap();
        assert_eq!(reply.as_deref(), Ok("kept"));
    }

    // Job control gives the shell a group of its own, as a server started with
    // setsid has: the kill at the group misses it and the reader stays behind.
    // Bash, since the build sandbox's /bin/sh has no job control.
    #[test]
    fn a_reader_left_behind_lets_the_steps_go() {
        let (claude, script) =
            claude_leaving("apart", "${CONFIG_SHELL:-bash} -c 'set -m; sleep 20 &'");
        let steps: Steps = Arc::new(|_| {});
        let run = Run {
            steps: Some(steps.clone()),
            ..Run::default()
        };
        let reply = claude.answer("", "", "words", &[], run);
        std::fs::remove_file(&script).unwrap();
        assert_eq!(reply.as_deref(), Ok("kept"));
        assert_eq!(Arc::strong_count(&steps), 1);
    }

    fn marks(blocks: &[Value]) -> Vec<usize> {
        (0..blocks.len())
            .filter(|&k| blocks[k].get("cache_control").is_some())
            .collect()
    }

    #[test]
    fn memory_rejoins_and_marks_its_settled_part() {
        let lines: String = (0..10).map(|i| format!("{i}+1|line {i}\n")).collect();
        let memory =
            format!("<chat>\n{lines}</chat>\n3 recent lines are not summarized yet: zoom them.\n");
        let blocks = cached(&memory, Some("1h"));
        let text: String = blocks.iter().map(|b| b["text"].as_str().unwrap()).collect();
        assert_eq!(text, memory);
        // Eleven lines before </chat>: two whole blocks, the second marked.
        assert_eq!(marks(&blocks), vec![1]);
        assert_eq!(blocks[1]["cache_control"]["ttl"], "1h");
    }

    #[test]
    fn mark_stays_ahead_of_unsummarized_lines() {
        let memory = format!(
            "<chat>\n0+4|a\n4+1|b\n5+1|c\n6+1|d\n7+1|e\n8+1|{PLACEHOLDER}\n9+1|{PLACEHOLDER}\n</chat>\n"
        );
        let blocks = cached(&memory, Some("1h"));
        // Six settled lines: one whole block, marked.
        assert_eq!(marks(&blocks), vec![0]);
        assert!(!blocks[0]["text"].as_str().unwrap().contains(PLACEHOLDER));
    }

    #[test]
    fn short_memory_has_no_mark() {
        assert!(marks(&cached("<chat>\n0+1|a\n</chat>\n", Some("1h"))).is_empty());
    }

    #[test]
    fn a_subagents_work_is_its_own() {
        let started = r#"{"type":"system","subtype":"task_started","task_id":"a6","tool_use_id":"t1","description":"Count files","task_type":"local_agent","prompt":"Run ls"}"#;
        assert_eq!(
            steps(started),
            vec![Step::Agent {
                id: "t1".into(),
                name: "Count files".into(),
                task: "Run ls".into()
            }]
        );
        let bash = r#"{"type":"system","subtype":"task_started","task_id":"b1","tool_use_id":"t9","description":"sleep","task_type":"local_bash"}"#;
        assert_eq!(steps(bash), vec![]);
        let call = r#"{"type":"assistant","parent_tool_use_id":"t1","message":{"content":[{"type":"thinking","thinking":""},{"type":"text","text":"Counting"},{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"ls"}}]}}"#;
        let inner = |step| Step::Inner {
            parent: "t1".into(),
            step: Box::new(step),
        };
        assert_eq!(
            steps(call),
            vec![
                inner(Step::Text {
                    text: "Counting".into()
                }),
                inner(Step::Call {
                    id: "t2".into(),
                    name: "Bash".into(),
                    input: serde_json::json!({"command": "ls"}),
                }),
            ]
        );
        let result = r#"{"type":"user","parent_tool_use_id":"t1","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":"101"}]}}"#;
        assert_eq!(
            steps(result),
            vec![inner(Step::Result {
                id: "t2".into(),
                text: "101".into(),
                error: false
            })]
        );
        let done = r#"{"type":"system","subtype":"task_notification","task_id":"a6","tool_use_id":"t1","status":"completed","summary":"101"}"#;
        assert_eq!(
            steps(done),
            vec![Step::Done {
                id: "t1".into(),
                report: "101".into()
            }]
        );
    }

    #[test]
    fn a_pick_reaches_the_claude_turn_under_way_and_no_other() {
        let steer = Arc::new(Steer::default());
        let (lines, sent) = mpsc::channel();
        *steer.into.lock().unwrap() = Some(lines);
        assert!(!switch("live-test", Some("opus"), None));
        let live = Live::enter("live-test".into(), &steer);
        assert!(switch("live-test", Some("opus"), Some("acceptEdits")));
        let got: Vec<Value> = sent
            .try_iter()
            .map(|l| serde_json::from_str(&l).unwrap())
            .collect();
        assert_eq!(got[0]["type"], "control_request");
        assert_eq!(
            got[0]["request"],
            json!({"subtype": "set_model", "model": "opus"})
        );
        assert_eq!(got[1]["request"]["mode"], "acceptEdits");
        assert_ne!(got[0]["request_id"], got[1]["request_id"]);
        assert!(!switch("another-chat", Some("opus"), None));
        // The CLI's refusal clears the request it answers.
        let id = got[1]["request_id"].as_str().unwrap();
        steer.answered(&json!({"type": "control_response", "response":
            {"subtype": "error", "request_id": id, "error": "no"}}));
        assert_eq!(steer.asked.lock().unwrap().len(), 1);
        drop(live);
        assert!(!switch("live-test", Some("opus"), None));
    }

    #[test]
    fn a_subagent_stops_by_the_call_that_started_it() {
        let steer = Arc::new(Steer::default());
        let (lines, sent) = mpsc::channel();
        *steer.into.lock().unwrap() = Some(lines);
        let _live = Live::enter("stop-test".into(), &steer);
        assert!(!stop_task("toolu_stop_test"));
        steer.task_started(&json!({"type": "system", "subtype": "task_started",
            "task_id": "t9", "tool_use_id": "toolu_stop_test", "description": "Look"}));
        assert!(stop_task("toolu_stop_test"));
        let got: Value = serde_json::from_str(&sent.try_recv().unwrap()).unwrap();
        assert_eq!(
            got["request"],
            json!({"subtype": "stop_task", "task_id": "t9"})
        );
    }

    #[test]
    fn a_control_request_libri_does_not_take_is_refused() {
        let steer = Steer::default();
        let (lines, sent) = mpsc::channel();
        *steer.into.lock().unwrap() = Some(lines);
        steer.refuse("r7", "hook_callback");
        let got: Value = serde_json::from_str(&sent.try_recv().unwrap()).unwrap();
        assert_eq!(got["response"]["subtype"], "error");
        assert_eq!(got["response"]["request_id"], "r7");
    }

    #[test]
    fn the_cli_version_is_checked_against_the_tested_range() {
        let init = r#"{"type":"system","subtype":"init","claude_code_version":"2.1.312","capabilities":[]}"#;
        assert_eq!(claude_version(init).as_deref(), Some("2.1.312"));
        assert_eq!(claude_version(r#"{"type":"result"}"#), None);
        assert_eq!(version_note("2.1.312"), None);
        assert_eq!(version_note("2.1.303"), None);
        assert!(version_note("2.1.302").unwrap().0);
        assert!(version_note("1.9.999").unwrap().0);
        let newer = version_note("2.1.400").unwrap();
        assert!(!newer.0 && newer.1.contains("2.1.400"));
        assert!(!version_note("2.2.0 (Claude Code)").unwrap().0);
    }

    #[test]
    fn unknown_lines_make_no_steps() {
        assert!(steps(r#"{"type":"some_new_kind","subtype":"x","message":{"content":[{"type":"new_block"}]}}"#).is_empty());
        assert!(steps("not json").is_empty());
        assert_eq!(
            answer(r#"{"type":"result","subtype":"a_new_end","result":"hi"}"#).as_deref(),
            Some("hi")
        );
    }

    #[test]
    fn an_mcp_tools_whole_result_rides_beside_its_text() {
        // Claude Code 2.1.312, as it reported a probe server's call.
        let line = r#"{"type":"user","message":{"content":[{"tool_use_id":"t1","type":"tool_result","content":"{\"g\":1}"}]},"tool_use_result":{"content":"{\"g\":1}","_meta":{"m":1},"structuredContent":{"g":1}}}"#;
        assert_eq!(
            steps(line),
            vec![
                Step::Result {
                    id: "t1".into(),
                    text: "{\"g\":1}".into(),
                    error: false
                },
                Step::Structured {
                    id: "t1".into(),
                    result: serde_json::json!({"structuredContent": {"g": 1}, "_meta": {"m": 1}}),
                },
            ]
        );
        // A built-in tool's result is no MCP result.
        let bash = r#"{"type":"user","message":{"content":[{"tool_use_id":"t2","type":"tool_result","content":"hi"}]},"tool_use_result":{"stdout":"hi","stderr":""}}"#;
        assert_eq!(steps(bash).len(), 1);
        // Codex's snake_case comes out in MCP's own names.
        let codex = serde_json::json!({"content": [{"type": "text", "text": "x"}], "structured_content": {"n": 1}});
        assert_eq!(
            structured(&codex),
            Some(
                serde_json::json!({"content": [{"type": "text", "text": "x"}], "structuredContent": {"n": 1}})
            )
        );
    }

    #[test]
    fn steps_from_stream() {
        let call = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Looking"},{"type":"tool_use","id":"t1","name":"mcp__calendar__events","input":{"days":7}}]}}"#;
        assert_eq!(
            steps(call),
            vec![Step::Call {
                id: "t1".into(),
                name: "mcp__calendar__events".into(),
                input: serde_json::json!({"days": 7}),
            }]
        );
        let result = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"a"},{"type":"image"},{"type":"text","text":"b"}],"is_error":true}]}}"#;
        assert_eq!(
            steps(result),
            vec![Step::Result {
                id: "t1".into(),
                text: "a\nb".into(),
                error: true
            }]
        );
        let delta = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}}"#;
        assert_eq!(steps(delta), vec![Step::Text { text: "Hel".into() }]);
        let thought = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"391 factors"}}}"#;
        assert_eq!(
            steps(thought),
            vec![Step::Thinking {
                text: "391 factors".into()
            }]
        );
        let start = r#"{"type":"stream_event","event":{"type":"content_block_start","index":0}}"#;
        assert!(steps(start).is_empty());
        assert!(steps(r#"{"type":"result","result":"hi"}"#).is_empty());
        assert!(steps("not json").is_empty());
    }

    #[test]
    fn answer_from_stream() {
        assert_eq!(
            answer(r#"{"type":"result","result":" hi \n"}"#),
            Some("hi".into())
        );
        assert_eq!(
            answer(r#"{"type":"result","result":"no","is_error":true}"#),
            None
        );
        assert_eq!(answer(r#"{"type":"result","result":"  "}"#), None);
        assert_eq!(answer(r#"{"type":"result","result":"NO_REPLY\n"}"#), None);
        assert_eq!(answer(r#"{"type":"assistant","result":"hi"}"#), None);
        assert_eq!(answer("not json"), None);
    }

    #[test]
    fn asks_from_stream() {
        let ev: Value = serde_json::from_str(
            r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool",
            "tool_name":"Write","input":{"file_path":"/tmp/a"},"description":"a"}}"#,
        )
        .unwrap();
        let ask = ask_of(&ev).unwrap();
        assert_eq!((ask.id.as_str(), ask.tool.as_str()), ("r1", "Write"));
        assert_eq!(ask.about.as_deref(), Some("a"));
        assert!(ask_of(&json!({"type": "result"})).is_none());
        assert_eq!(verdict(Some(json!({"x": 1})))["behavior"], "allow");
        assert_eq!(verdict(None)["behavior"], "deny");
    }

    #[test]
    fn background_from_stream() {
        let changed = |tasks: &str| {
            format!(r#"{{"type":"system","subtype":"background_tasks_changed","tasks":{tasks}}}"#)
        };
        assert_eq!(
            background_tasks(&changed(
                r#"[{"task_id":"b1","description":"Nap five seconds"}]"#
            )),
            Some(vec!["Nap five seconds".to_owned()])
        );
        assert_eq!(background_tasks(&changed("[]")), Some(vec![]));
        assert_eq!(background_tasks(r#"{"type":"result","result":"hi"}"#), None);
    }
}
