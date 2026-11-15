//! The compactor's model calls: the COMPACT prompt, the step it is asked
//! to do, size enforcement, and the backends that run a call. Which model
//! and backend is configuration (the Nix module), never code.

use crate::memory::flight::{self, Flights};
use crate::memory::tree::{NODE, addr, flat};
use chrono::{DateTime, Local};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

/// Version of the prompt below, stored on every node it builds.
pub const PROMPT_VERSION: &str = "hippo-11";

/// Tries per node to get under `NODE`; the shortest is kept.
pub const TRIES: usize = 5;

/// UniiChat's system prompt (prompt.txt, 0500bf6), the agent renamed, its
/// message kinds and tools Libri's, its paragraph on connected computers
/// dropped (Libri lists none). Libri's turns read it too until they kept
/// their own harness sessions, with hippo2 for memory; this copy is hippo's.
pub const COMPACT: &str = include_str!("prompt.txt");

pub enum Step<'a> {
    /// Message `id`, rendered `kind: text`.
    Compress { id: u64, msg: &'a str },
    /// Node `(l, i)` from its two halves.
    Merge {
        l: u8,
        i: u64,
        a: &'a str,
        b: &'a str,
    },
}

/// The user message of a call: the context block (`id+n|text` lines, as
/// the view shows them), then the step, naming its message or lines by id:
/// UniiChat's LEAF and MERGE. A ruler of `NODE` dashes shows the size,
/// since models can't count bytes; a sample line as ruler got its content
/// copied.
pub fn input(context: &[String], step: &Step) -> [String; 2] {
    let chat = format!("<chat>\n{}\n</chat>", context.join("\n"));
    let ruler = "-".repeat(NODE);
    let ask = match step {
        Step::Compress { id, msg } => format!(
            "Compaction: compress message {id} into one line of at most {NODE} bytes\n(about 70 words), the length of this ruler:\n{ruler}\n<input>\n{msg}\n</input>"
        ),
        Step::Merge { l, i, a, b } => {
            let (s, n) = addr(*l, *i);
            let h = n / 2;
            format!(
                "Compaction: merge lines {s}+{h} and {}+{h}, adjacent, into one line of at most\n{NODE} bytes (about 70 words), the length of this ruler:\n{ruler}\n<chat> may hold their messages, {s} to {}, in more detail: take details\nof them from there too.\n<input>\n{}\n{}\n</input>",
                s + h,
                s + n - 1,
                flat(a),
                flat(b)
            )
        }
    };
    [chat, ask]
}

/// UniiChat's `over`: the note a try after a line too long adds to the
/// first call's blocks, its size and where the limit cuts it.
pub fn over(line: &str) -> String {
    format!(
        "Too long: your last line for this <input> was {} bytes,\nover the {NODE}-byte limit. Write the whole line again, cutting just\nenough of the least valuable items to fit before this cut:\n{}| ← LIMIT",
        line.len(),
        cut(line, NODE)
    )
}

/// The node of a step no line came for (a refusal, an empty answer):
/// its source, flattened, cut at `NODE`, as UniiChat's `cut(flat(text))`.
pub fn clipped(step: &Step) -> String {
    let source = match step {
        Step::Compress { msg, .. } => flat(msg),
        Step::Merge { a, b, .. } => flat(&format!("{}\n{}", flat(a), flat(b))),
    };
    cut(&source, NODE).to_owned()
}

/// At most `max` bytes of `s`, never splitting a character.
pub fn cut(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[derive(Debug)]
pub enum Fail {
    /// A usage or rate limit: wait until then (or 5 minutes).
    Limit(Option<DateTime<Local>>),
    /// The model declined the input: retrying the same input fails again.
    Refused(String),
    /// Libri can't be reached: no call ran, and every node would fail alike.
    Unreachable(String),
    /// Libri reached the model and the model failed, or none can run now:
    /// every node would fail alike.
    Model(String),
    /// This one node's call failed.
    Other(String),
}

impl Fail {
    /// The kind `hippo health` reports. Only a limit reads as one.
    pub fn kind(&self) -> &'static str {
        match self {
            Fail::Limit(_) => "rate_limit",
            Fail::Refused(_) => "refused",
            Fail::Unreachable(_) => "unreachable",
            Fail::Model(_) | Fail::Other(_) => "model",
        }
    }

    /// The failure in a person's words.
    pub fn text(&self) -> String {
        match self {
            Fail::Limit(Some(t)) => format!("rate limited until {}", t.format("%H:%M")),
            Fail::Limit(None) => "rate limited".into(),
            Fail::Refused(e) | Fail::Unreachable(e) | Fail::Model(e) | Fail::Other(e) => e.clone(),
        }
    }
}

/// Tokens a call used, as the backend reports them.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Usage {
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
    /// Model calls behind these tokens: one, plus each retry.
    #[serde(default)]
    pub calls: u64,
}

impl Usage {
    pub fn add(&mut self, o: Usage) {
        self.input += o.input;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
        self.output += o.output;
        self.calls += o.calls;
    }
}

/// One conversation with a model.
pub trait Chat {
    fn say(&mut self, blocks: &[String]) -> Result<String, Fail>;
    fn model(&self) -> String;
    /// Runs `f` once the next answer begins, when its prefix starts to
    /// serve from the cache; backends with `flights` must.
    fn on_begin(&mut self, _f: Box<dyn FnOnce() + Send>) {}
    /// Everything this conversation used so far.
    fn usage(&self) -> Usage {
        Usage::default()
    }
}

pub trait Backend: Send + Sync {
    fn start(&self) -> Result<Box<dyn Chat>, Fail>;
    /// A conversation for the memory of worker `whose`, None being main's:
    /// a backend that reports how calls went tells them apart.
    fn start_for(&self, _whose: Option<&str>) -> Result<Box<dyn Chat>, Fail> {
        self.start()
    }
    /// A different backend for steps at tree level `l`, if there is one.
    fn for_level(&self, _l: u8) -> Option<&dyn Backend> {
        None
    }
    /// Single flight for a backend whose cache serves an entry only once
    /// its writer's answer begins: Anthropic's.
    fn flights(&self) -> Option<&Arc<Flights>> {
        None
    }
}

/// A model per tree level: cheap ones low in the tree, where a call
/// compresses one message, better ones for the merges above. Levels not
/// listed use the base backend.
pub struct Levels {
    pub base: Box<dyn Backend>,
    pub per: Vec<(u8, Box<dyn Backend>)>,
}

impl Backend for Levels {
    fn start(&self) -> Result<Box<dyn Chat>, Fail> {
        self.base.start()
    }
    fn start_for(&self, whose: Option<&str>) -> Result<Box<dyn Chat>, Fail> {
        self.base.start_for(whose)
    }
    fn for_level(&self, l: u8) -> Option<&dyn Backend> {
        let b = self.per.iter().find(|(at, _)| *at == l)?;
        Some(b.1.as_ref())
    }
    fn flights(&self) -> Option<&Arc<Flights>> {
        self.base.flights()
    }
}

/// The node a step builds, `level/index`, as the journal names it.
fn node(step: &Step) -> String {
    match step {
        Step::Compress { id, .. } => format!("0/{id}"),
        Step::Merge { l, i, .. } => format!("{l}/{i}"),
    }
}

/// The line without what models copy from <chat> before its kind: ids,
/// ranges, dates and times, bare or bracketed (`88745 talk:`,
/// `12+2|user:`, `11-04 87456-87487: user:`, `(11-04 22:33): talk:`,
/// `[83800-01] tool:`). A line opens with its kind, so no line loses a
/// number it meant to start with; a bracket holding a word is kept.
pub fn behead(line: &str) -> &str {
    static HEAD: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^(?:(?:[(\[][\d\sx+\-:/|~,]*[)\]]|~?\d[\dx+\-:/|~]*)[:|,]?\s*)+")
            .unwrap()
    });
    match HEAD.find(line) {
        Some(m) if m.end() < line.len() => &line[m.end()..],
        _ => line,
    }
}

/// UniiChat's `Memory.build`: asked till a line fits, `TRIES` lines at
/// most, the shortest kept. Each try is a call of its own with the first
/// call's blocks, never a try's words: a try after a line too long adds
/// only `over` of that line. No line (a refusal, an empty answer) is
/// `Fail::Refused`, its node its source clipped; a try that fails after a
/// line came keeps the best line so far. One addition: a copied head
/// is cut off each reply (`behead`), at no extra call.
pub fn run(
    backend: &dyn Backend,
    whose: Option<&str>,
    context: &[String],
    step: &Step,
) -> Result<(String, String, Usage), Fail> {
    let mut chat = backend.start_for(whose)?;
    let blocks = input(context, step);
    let keys = flight::keys(&blocks[0], BLOCK);
    let asked = std::time::Instant::now();
    let probe = backend.flights().map(|f| f.probe(&keys));
    let flight = backend.flights().map(|f| f.wait(&keys));
    if let (Some(f), Some(p)) = (&flight, probe) {
        chat.on_begin(Box::new(f.begun()));
        // Which context each call sent and what the cache should hold of
        // it, in blocks, so each rewrite of the context can be traced to its
        // cause against the tokens read (`usage.jsonl`, by level and index).
        eprintln!(
            "hippo: flight {} blocks={} hit={} beyond={} lapsed={} shared={} {} waited={}ms",
            node(step),
            keys.len(),
            p.hit,
            p.beyond,
            p.lapsed,
            p.shared,
            if f.leads() { "leads" } else { "reads" },
            asked.elapsed().as_millis()
        );
    }
    let reply = chat.say(&blocks);
    drop(flight);
    let model = chat.model();
    let mut used = chat.usage();
    // Its process goes back to the idle ones before a retry takes one.
    drop(chat);
    let mut lines: Vec<String> = Vec::new();
    let mut reply = reply?;
    loop {
        let line = behead(reply.trim()).to_owned();
        if line.is_empty() {
            break;
        }
        let fits = line.len() <= NODE;
        lines.push(line);
        if fits || lines.len() == TRIES {
            break;
        }
        // How often a line runs over, for judging the compactor model by
        // its journal.
        eprintln!(
            "hippo: retry {} try={} over={}",
            node(step),
            lines.len(),
            lines[lines.len() - 1].len()
        );
        let mut again = backend.start_for(whose)?;
        let mut more = blocks.to_vec();
        more.push(over(&lines[lines.len() - 1]));
        let said = again.say(&more);
        used.add(again.usage());
        match said {
            Ok(r) => reply = r,
            Err(_) => break,
        }
    }
    used.calls = lines.len().max(1) as u64;
    match lines.into_iter().min_by_key(String::len) {
        Some(line) => Ok((line, model, used)),
        None => Err(Fail::Refused("an empty line".into())),
    }
}

/// Configuration, from the environment the Nix module sets.
/// COMPACT for the agent this store remembers: `HIPPO_AGENT`, else Libri.
fn prompt() -> String {
    let head = match std::env::var("HIPPO_AGENT") {
        Ok(name) if !name.is_empty() => COMPACT.replace("Libri", &name),
        _ => COMPACT.to_owned(),
    };
    // The person's rules after the prompt, as Libri's summarizer sends them
    // (summarizer.rs), for a store built outside Libri: the memory test's
    // scratch trees. Goes when hippo reads the rules itself.
    match std::env::var("HIPPO_RULES_FILE")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
    {
        Some(rules) if !rules.trim().is_empty() => format!("{head}\n\n{rules}"),
        _ => head,
    }
}

/// The compactor when nothing configures one: Claude Haiku, thinking high.
/// Any other is configuration (`HIPPO_BACKEND`, `HIPPO_MODEL`,
/// `HIPPO_EFFORT`, `HIPPO_URL`), never a list here.
pub const MODEL: &str = "haiku";
pub const EFFORT: &str = "high";

/// `HIPPO_BACKEND` (claude, http, codex or libri) with `HIPPO_MODEL`,
/// `HIPPO_EFFORT` and `HIPPO_URL`. Then `HIPPO_LEVELS`, entries
/// `level=backend/model/effort` separated by spaces, say
/// `0=claude/haiku/xhigh`, each overriding one tree level.
pub fn from_env() -> Result<Box<dyn Backend>, String> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let base = backend(
        var("HIPPO_BACKEND").as_deref().unwrap_or("claude"),
        var("HIPPO_MODEL"),
        var("HIPPO_EFFORT"),
        var("HIPPO_URL"),
    )?;
    let Some(levels) = var("HIPPO_LEVELS") else {
        return Ok(base);
    };
    let mut per = Vec::new();
    for entry in levels.split_whitespace() {
        let bad = || format!("HIPPO_LEVELS entry {entry}: want level=backend/model/effort");
        let (l, spec) = entry.split_once('=').ok_or_else(bad)?;
        let l = l.parse().map_err(|_| bad())?;
        let [kind, model, effort] = spec.split('/').collect::<Vec<_>>()[..] else {
            return Err(bad());
        };
        per.push((
            l,
            backend(
                kind,
                Some(model.into()),
                Some(effort.into()),
                var("HIPPO_URL"),
            )?,
        ));
    }
    Ok(Box::new(Levels { base, per }))
}

fn backend(
    kind: &str,
    model: Option<String>,
    effort: Option<String>,
    url: Option<String>,
) -> Result<Box<dyn Backend>, String> {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    match kind {
        "claude" => Ok(Box::new(ClaudeCli {
            prompt: prompt(),
            command: var("HIPPO_CLAUDE").unwrap_or_else(|| "claude".into()),
            model: model.unwrap_or_else(|| MODEL.into()),
            effort: effort.unwrap_or_else(|| EFFORT.into()),
            flights: Arc::default(),
            idle: Arc::default(),
        })),
        "omp" => Ok(Box::new(Omp {
            socket: var("HIPPO_OMP_SOCKET").ok_or("HIPPO_OMP_SOCKET is not set")?,
            model: model.ok_or("HIPPO_MODEL is not set")?,
        })),
        "http" => Ok(Box::new(Http {
            prompt: prompt(),
            url: url.ok_or("HIPPO_URL is not set")?,
            model: model.ok_or("HIPPO_MODEL is not set")?,
            effort,
            key: match var("HIPPO_KEY_FILE") {
                Some(path) => Some(
                    std::fs::read_to_string(&path)
                        .map_err(|e| format!("HIPPO_KEY_FILE {path}: {e}"))?
                        .trim()
                        .to_owned(),
                ),
                None => None,
            },
            extra: match var("HIPPO_HTTP_EXTRA") {
                Some(raw) => serde_json::from_str(&raw)
                    .map_err(|e| format!("HIPPO_HTTP_EXTRA is not a JSON object: {e}"))?,
                None => serde_json::Map::new(),
            },
        })),
        "codex" => Ok(Box::new(CodexCli {
            prompt: prompt(),
            command: var("HIPPO_CODEX").unwrap_or_else(|| "codex".into()),
            model: model.unwrap_or_else(|| "gpt-6-luna".into()),
            effort: effort.unwrap_or_else(|| "low".into()),
            server: Default::default(),
        })),
        "libri" => Ok(Box::new(Libri {
            prompt: prompt(),
            sock: crate::dir().join(LIBRI_SOCK),
            model: model.unwrap_or_else(|| MODEL.into()),
            effort: effort.unwrap_or_else(|| EFFORT.into()),
        })),
        other => Err(format!("Unknown backend {other}.")),
    }
}

/// Where a Libri beside this store answers for the compactor.
const LIBRI_SOCK: &str = "compactor.sock";

/// The compactor through Libri: its daemon runs each call on its Claude
/// harness, on the model and effort configured here, so this store knows no
/// harness and no key. The socket sits in the store, so nothing
/// needs telling where. Libri is stateless per call: a retry sends what
/// came before.
pub struct Libri {
    pub prompt: String,
    pub sock: std::path::PathBuf,
    pub model: String,
    pub effort: String,
}

struct LibriChat {
    prompt: String,
    sock: std::path::PathBuf,
    /// The worker whose memory this call is for; None is main's.
    worker: Option<String>,
    memory: Option<String>,
    said: String,
    /// The model asked for, as configured; `model` is the one Libri says ran.
    asked: String,
    model: String,
    effort: String,
    used: Usage,
}

impl Backend for Libri {
    fn start(&self) -> Result<Box<dyn Chat>, Fail> {
        self.start_for(None)
    }
    fn start_for(&self, whose: Option<&str>) -> Result<Box<dyn Chat>, Fail> {
        Ok(Box::new(LibriChat {
            prompt: self.prompt.clone(),
            sock: self.sock.clone(),
            worker: whose.map(str::to_owned),
            memory: None,
            said: String::new(),
            asked: self.model.clone(),
            model: self.model.clone(),
            effort: self.effort.clone(),
            used: Usage::default(),
        }))
    }
}

impl LibriChat {
    fn ask(&mut self) -> Result<Value, Fail> {
        // Libri restarting, or down: the whole queue waits for it, where a
        // failed node would be parked and never come back.
        let shut = |e: std::io::Error| {
            eprintln!("hippo: Libri at {}: {e}", self.sock.display());
            Fail::Unreachable(format!("Libri can't be reached: {e}"))
        };
        let mut conn = std::os::unix::net::UnixStream::connect(&self.sock).map_err(shut)?;
        conn.set_read_timeout(Some(Duration::from_secs(CALL_TIMEOUT)))
            .map_err(shut)?;
        let mut line = json!({"system": self.prompt, "memory": self.memory, "prompt": self.said,
                "worker": self.worker, "model": self.asked, "effort": self.effort})
        .to_string();
        line.push('\n');
        conn.write_all(line.as_bytes()).map_err(shut)?;
        let mut reply = String::new();
        BufReader::new(&conn).read_line(&mut reply).map_err(shut)?;
        // Libri went down during the call, which it finishes or not at all.
        if reply.is_empty() {
            return Err(shut(std::io::ErrorKind::UnexpectedEof.into()));
        }
        serde_json::from_str(&reply).map_err(|e| Fail::Other(format!("Libri: {e}")))
    }
}

impl Chat for LibriChat {
    fn say(&mut self, blocks: &[String]) -> Result<String, Fail> {
        let mut blocks = blocks.iter();
        if self.memory.is_none() {
            self.memory = blocks.next().cloned();
        }
        for b in blocks {
            if !self.said.is_empty() {
                self.said.push_str("\n\n");
            }
            self.said.push_str(b);
        }
        let r = self.ask()?;
        if let Some(e) = r.get("error").and_then(Value::as_str) {
            eprintln!("hippo: Libri: {e}");
            // What can't run now (Libri has told the person) waits, as a
            // usage limit does; only a refusal is this one input's.
            return Err(refused(e)
                .or_else(|| limit(e))
                .unwrap_or_else(|| Fail::Model(e.to_owned())));
        }
        let text = r.get("text").and_then(Value::as_str).unwrap_or_default();
        if let Some(m) = r.get("model").and_then(Value::as_str) {
            self.model = m.to_owned();
        }
        let n = |k: &str| r.pointer(&format!("/usage/{k}")).and_then(Value::as_u64);
        self.used.add(Usage {
            input: n("input").unwrap_or(0),
            cache_read: n("cache_read").unwrap_or(0),
            cache_write: n("cache_write").unwrap_or(0),
            output: n("output").unwrap_or(0),
            ..Usage::default()
        });
        // What it answered stays in the conversation a retry continues.
        self.said.push_str(&format!("\n\nYou answered:\n{text}"));
        Ok(text.to_owned())
    }

    fn model(&self) -> String {
        self.model.clone()
    }

    fn usage(&self) -> Usage {
        self.used
    }
}

/// The `claude` CLI on the subscription: print mode with stream-json in and
/// out; no tools, no MCP, no settings sources (so no hooks), no session
/// saved. A process outlives its conversation: when one ends, `/clear`
/// starts the next in the same process (Claude Code answers it with a
/// `conversation_reset`), so a call costs no Node start. Idle processes
/// wait in `idle`, at most as many as the compactor runs calls at once.
pub struct ClaudeCli {
    pub prompt: String,
    pub command: String,
    pub model: String,
    pub effort: String,
    pub flights: Arc<Flights>,
    pub idle: Arc<std::sync::Mutex<Vec<ClaudeProc>>>,
}

/// Lines per content block of the context. Anthropic looks back up to 20
/// blocks from a mark for an earlier entry, so the next call, its context
/// grown by a few lines, still finds this one's.
pub const BLOCK: usize = 4;

/// The user message's content blocks: the context in blocks of `BLOCK`
/// lines, the last whole one marked. The API allows four cache marks and
/// Claude Code uses three, one on the request's end, so the context gets
/// one. It has the five-minute lifetime the calls run with (a shorter mark
/// may not precede a longer one).
pub fn marked(blocks: &[String]) -> Vec<Value> {
    let mut out = Vec::new();
    for (k, b) in blocks.iter().enumerate() {
        if k == 0 && b.starts_with("<chat>") {
            let lines: Vec<&str> = b.split_inclusive('\n').collect();
            let whole = lines.len() / BLOCK;
            for (n, chunk) in lines.chunks(BLOCK).enumerate() {
                let text = chunk.concat();
                if n + 1 == whole && chunk.len() == BLOCK {
                    out.push(json!({"type": "text", "text": text,
                        "cache_control": {"type": "ephemeral"}}));
                } else {
                    out.push(json!({"type": "text", "text": text}));
                }
            }
            continue;
        }
        out.push(json!({"type": "text", "text": b}));
    }
    out
}

/// One running `claude -p`, between conversations or in one.
pub struct ClaudeProc {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    errors: Option<std::thread::JoinHandle<String>>,
}

impl ClaudeProc {
    /// Reads events until a `result`, giving each to `seen`; None if the
    /// process ends first or takes over `secs` (then it is killed).
    fn result(&mut self, secs: u64, mut seen: impl FnMut(&Value)) -> Option<Value> {
        let pid = self.child.id();
        // A watchdog: an answer that never comes ends the process.
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            if rx.recv_timeout(Duration::from_secs(secs)).is_err() {
                unsafe {
                    libc::kill(pid as i32, libc::SIGKILL);
                }
            }
        });
        let mut buf = String::new();
        let out = loop {
            buf.clear();
            match self.stdout.read_line(&mut buf) {
                Ok(0) | Err(_) => break None,
                Ok(_) => {}
            }
            let Ok(ev) = serde_json::from_str::<Value>(&buf) else {
                continue;
            };
            if ev.get("type").and_then(Value::as_str) == Some("result") {
                break Some(ev);
            }
            seen(&ev);
        };
        let _ = tx.send(());
        out
    }

    fn send(&mut self, msg: &Value) -> bool {
        let mut line = msg.to_string();
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.flush())
            .is_ok()
    }

    /// Ends the conversation so the next starts empty: true once Claude
    /// Code has reset it.
    fn clear(&mut self) -> bool {
        let msg = json!({"type": "user", "message": {"role": "user", "content": "/clear"}});
        let mut reset = false;
        self.send(&msg)
            && self
                .result(CLEAR_TIMEOUT, |ev| {
                    reset |= ev.get("type").and_then(Value::as_str) == Some("conversation_reset")
                })
                .is_some()
            && reset
    }

    /// The end of its stderr, the process killed.
    fn stderr(&mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let err = self.errors.take().map(|e| e.join().unwrap_or_default());
        let err = err.unwrap_or_default();
        let err = err.trim();
        let start = err.char_indices().rev().nth(499).map_or(0, |(i, _)| i);
        err[start..].to_owned()
    }
}

impl Drop for ClaudeProc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct ClaudeChat {
    /// None once it failed: a process in an unknown state is not reused.
    proc: Option<ClaudeProc>,
    home: Arc<std::sync::Mutex<Vec<ClaudeProc>>>,
    model: String,
    used: Usage,
    begun: Option<Box<dyn FnOnce() + Send>>,
}

/// Seconds a single answer may take before the call is abandoned.
const CALL_TIMEOUT: u64 = 600;
/// Seconds `/clear` may take; it calls no model.
const CLEAR_TIMEOUT: u64 = 30;

impl ClaudeCli {
    fn spawn(&self) -> Result<ClaudeProc, Fail> {
        let mut child = Command::new(&self.command)
            .args([
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                // For message_start, when an answer begins (`on_begin`).
                "--include-partial-messages",
                "--no-session-persistence",
                "--system-prompt",
                &self.prompt,
                "--tools",
                "",
                "--strict-mcp-config",
                "--setting-sources",
                "",
                "--model",
                &self.model,
                "--effort",
                &self.effort,
            ])
            // Five-minute cache entries, not the one-hour ones Claude Code
            // gives subscribers: a one-hour write costs 2x input, a
            // five-minute one 1.25x, and consecutive calls come seconds apart.
            .env("FORCE_PROMPT_CACHING_5M", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Fail::Other(format!("{}: {e}", self.command)))?;
        Ok(ClaudeProc {
            stdin: child.stdin.take().unwrap(),
            stdout: BufReader::new(child.stdout.take().unwrap()),
            errors: Some(drain(child.stderr.take().unwrap())),
            child,
        })
    }
}

impl Backend for ClaudeCli {
    fn start(&self) -> Result<Box<dyn Chat>, Fail> {
        // An idle process that has since exited is dropped, not handed out.
        let idle = std::iter::from_fn(|| self.idle.lock().unwrap().pop())
            .find_map(|mut p| matches!(p.child.try_wait(), Ok(None)).then_some(p));
        let proc = match idle {
            Some(p) => p,
            None => self.spawn()?,
        };
        Ok(Box::new(ClaudeChat {
            proc: Some(proc),
            home: self.idle.clone(),
            model: self.model.clone(),
            used: Usage::default(),
            begun: None,
        }))
    }

    fn flights(&self) -> Option<&Arc<Flights>> {
        Some(&self.flights)
    }
}

/// Reads a pipe to its end on its own thread, so a chatty child never
/// blocks on a full pipe, and keeps the last few kilobytes.
fn drain(mut pipe: impl std::io::Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let (mut kept, mut chunk) = (Vec::new(), [0u8; 4096]);
        while let Ok(n @ 1..) = pipe.read(&mut chunk) {
            kept.extend_from_slice(&chunk[..n]);
            let over = kept.len().saturating_sub(8192);
            kept.drain(..over);
        }
        String::from_utf8_lossy(&kept).into_owned()
    })
}

impl Chat for ClaudeChat {
    fn say(&mut self, blocks: &[String]) -> Result<String, Fail> {
        let Some(proc) = self.proc.as_mut() else {
            return Err(Fail::Other(
                "claude failed earlier in this conversation".into(),
            ));
        };
        let content = marked(blocks);
        let msg = json!({"type": "user", "message": {"role": "user", "content": content}});
        if !proc.send(&msg) {
            let err = proc.stderr();
            self.proc = None;
            return Err(Fail::Other(format!("claude exited: {err}")));
        }
        let (begun, model) = (&mut self.begun, &mut self.model);
        let result = proc.result(CALL_TIMEOUT, |ev| {
            if ev.pointer("/event/type").and_then(Value::as_str) == Some("message_start")
                && let Some(f) = begun.take()
            {
                f();
            }
            if ev.get("type").and_then(Value::as_str) == Some("assistant")
                && let Some(m) = ev.pointer("/message/model").and_then(Value::as_str)
            {
                *model = m.to_owned();
            }
        });
        let Some(ev) = result else {
            let err = proc.stderr();
            self.proc = None;
            return Err(Fail::Other(format!("claude gave no result: {err}")));
        };
        let n = |k: &str| {
            ev.pointer(&format!("/usage/{k}"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        self.used.add(Usage {
            input: n("input_tokens"),
            cache_read: n("cache_read_input_tokens"),
            cache_write: n("cache_creation_input_tokens"),
            output: n("output_tokens"),
            ..Usage::default()
        });
        let text = ev
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if ev.get("is_error") == Some(&Value::Bool(true)) {
            return Err(refused(&text)
                .or_else(|| limit(&text))
                .unwrap_or(Fail::Other(text)));
        }
        Ok(text)
    }

    fn model(&self) -> String {
        self.model.clone()
    }

    fn on_begin(&mut self, f: Box<dyn FnOnce() + Send>) {
        self.begun = Some(f);
    }

    fn usage(&self) -> Usage {
        self.used
    }
}

impl Drop for ClaudeChat {
    /// The process goes back to the idle ones, its conversation cleared,
    /// while fewer wait than the compactor runs calls at once.
    fn drop(&mut self) {
        let Some(mut proc) = self.proc.take() else {
            return;
        };
        let room = self.home.lock().unwrap().len() < crate::memory::compactor::jobs();
        if room && proc.clear() {
            self.home.lock().unwrap().push(proc);
        }
    }
}

/// Refusals by the model's safeguards, as Claude Code words them.
const REFUSALS: &[&str] = &[
    "safeguards flagged",
    "can't respond to this message",
    "cyber safeguards",
    "legitimate cybersecurity work",
    "usage policy",
];

/// A refusal by the model's safeguards, which may fire again on every retry
/// of the same input.
fn refused(text: &str) -> Option<Fail> {
    let lower = text.to_lowercase();
    REFUSALS
        .iter()
        .any(|p| lower.contains(p))
        .then(|| Fail::Refused(text.to_owned()))
}

/// Whole phrases: a bare "limit" or "rate" is in too many other errors
/// ("generate", "unlimited"), and a false limit reads as a healthy wait.
const LIMITS: &[&str] = &[
    "usage limit",
    "rate limit",
    "rate_limit",
    "hit your limit",
    "session limit",
    "weekly limit",
    "limit reached",
    "limit exceeded",
    "overloaded",
    "too many requests",
    "quota",
];

/// A usage or rate limit message, with its reset time when it gives one
/// (as `…|<epoch seconds>`).
fn limit(text: &str) -> Option<Fail> {
    let lower = text.to_lowercase();
    if !LIMITS.iter().any(|p| lower.contains(p)) {
        return None;
    }
    let reset = text
        .rsplit('|')
        .next()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .and_then(|s| DateTime::from_timestamp(s, 0))
        .map(|d| d.with_timezone(&Local));
    Some(Fail::Limit(reset))
}

/// Any OpenAI-compatible chat completions endpoint over HTTP: a local
/// llama.cpp, or a hosted one such as OpenCode Go.
#[derive(Clone)]
pub struct Http {
    pub prompt: String,
    pub url: String,
    pub model: String,
    pub effort: Option<String>,
    /// Bearer key, read from `HIPPO_KEY_FILE` so it stays out of the
    /// environment and the Nix store.
    pub key: Option<String>,
    /// Fields merged into every request, for what a server takes beyond the
    /// OpenAI API (llama.cpp's chat_template_kwargs, say).
    pub extra: serde_json::Map<String, Value>,
}

struct HttpChat {
    backend: Http,
    /// One id per conversation, so a retry reuses the cached prefix:
    /// OpenCode Go refuses requests without one (`x-opencode-session`).
    session: String,
    messages: Vec<Value>,
    used: Usage,
}

impl Backend for Http {
    fn start(&self) -> Result<Box<dyn Chat>, Fail> {
        Ok(Box::new(HttpChat {
            backend: self.clone(),
            session: session(),
            messages: vec![json!({"role": "system", "content": self.prompt})],
            used: Usage::default(),
        }))
    }
}

/// A fresh conversation id: the time and the process, unique enough for
/// a server to route and cache by.
fn session() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "hippo-{}-{now:x}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

impl Chat for HttpChat {
    fn say(&mut self, blocks: &[String]) -> Result<String, Fail> {
        let content: Vec<Value> = blocks
            .iter()
            .map(|t| json!({"type": "text", "text": t}))
            .collect();
        self.messages
            .push(json!({"role": "user", "content": content}));
        let b = &self.backend;
        let mut body = json!({"model": b.model, "messages": self.messages});
        if let Some(effort) = &b.effort {
            body["reasoning_effort"] = json!(effort);
        }
        for (k, v) in &b.extra {
            body[k] = v.clone();
        }
        let url = format!("{}/chat/completions", b.url.trim_end_matches('/'));
        let mut req = ureq::post(&url)
            .header("user-agent", concat!("hippo/", env!("CARGO_PKG_VERSION")))
            .header("x-opencode-session", &self.session);
        if let Some(key) = &b.key {
            req = req.header("authorization", format!("Bearer {key}"));
        }
        let mut resp = req
            .config()
            .http_status_as_error(false)
            .build()
            .send_json(&body)
            .map_err(|e| Fail::Other(format!("{url}: {e}")))?;
        let status = resp.status().as_u16();
        if status == 429 {
            let reset = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<i64>().ok())
                .map(|s| Local::now() + chrono::Duration::seconds(s));
            return Err(Fail::Limit(reset));
        }
        let v: Value = resp
            .body_mut()
            .read_json()
            .map_err(|e| Fail::Other(format!("{url}: {e}")))?;
        if status >= 400 {
            return Err(Fail::Other(format!("{url}: {status} {v}")));
        }
        let n = |p: &str| v.pointer(p).and_then(Value::as_u64).unwrap_or(0);
        let cached = n("/usage/prompt_tokens_details/cached_tokens");
        self.used.add(Usage {
            input: n("/usage/prompt_tokens").saturating_sub(cached),
            cache_read: cached,
            cache_write: 0,
            output: n("/usage/completion_tokens"),
            ..Usage::default()
        });
        // A reply cut at the token cap (a model stuck thinking, say) is
        // a failed try; the node is retried.
        if v.pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            == Some("length")
        {
            return Err(Fail::Other(format!(
                "{url}: reply cut at the token cap after {} tokens",
                n("/usage/completion_tokens")
            )));
        }
        let text = v
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| Fail::Other(format!("{url}: no content in {v}")))?
            .to_owned();
        self.messages
            .push(json!({"role": "assistant", "content": text}));
        Ok(text)
    }

    fn model(&self) -> String {
        self.backend.model.clone()
    }

    fn usage(&self) -> Usage {
        self.used
    }
}

/// omp owns provider selection, credentials and request dispatch. This local
/// socket carries only the compaction conversation; no provider key enters hippo.
#[derive(Clone)]
pub struct Omp {
    socket: String,
    model: String,
}

struct OmpChat {
    backend: Omp,
    messages: Vec<Value>,
    used: Usage,
}

impl Backend for Omp {
    fn start(&self) -> Result<Box<dyn Chat>, Fail> {
        Ok(Box::new(OmpChat {
            backend: self.clone(),
            messages: vec![json!({"role": "system", "content": prompt()})],
            used: Usage::default(),
        }))
    }
}

impl Chat for OmpChat {
    fn say(&mut self, blocks: &[String]) -> Result<String, Fail> {
        use std::os::unix::net::UnixStream;
        self.messages
            .push(json!({"role": "user", "content": blocks.join("\n")}));
        let mut stream = UnixStream::connect(&self.backend.socket)
            .map_err(|e| Fail::Other(format!("omp summarizer unavailable: {e}")))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(180)))
            .map_err(|e| Fail::Other(e.to_string()))?;
        writeln!(stream, "{}", json!({"messages": self.messages}))
            .map_err(|e| Fail::Other(e.to_string()))?;
        let mut line = String::new();
        BufReader::new(stream)
            .read_line(&mut line)
            .map_err(|e| Fail::Other(e.to_string()))?;
        let response: Value =
            serde_json::from_str(&line).map_err(|e| Fail::Other(e.to_string()))?;
        if let Some(error) = response.get("error").and_then(Value::as_str) {
            return Err(Fail::Other(error.into()));
        }
        if let Some(usage) = response.get("usage") {
            self.used.add(
                serde_json::from_value(usage.clone()).map_err(|e| Fail::Other(e.to_string()))?,
            );
        }
        let text = response
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| Fail::Other("omp summarizer returned no text".into()))?
            .to_owned();
        self.messages
            .push(json!({"role": "assistant", "content": text}));
        Ok(text)
    }

    fn usage(&self) -> Usage {
        self.used
    }

    fn model(&self) -> String {
        self.backend.model.clone()
    }
}

/// Codex on the ChatGPT subscription, through `codex app-server`: one
/// server per hippo, started on the first call and again if it dies,
/// hosting an ephemeral thread per conversation, the prompt as its base
/// instructions (in place of Codex's own), in an empty read-only directory.
/// One app-server hosts many threads at once, as the Codex IDE extension
/// uses it.
pub struct CodexCli {
    pub prompt: String,
    pub command: String,
    pub model: String,
    pub effort: String,
    pub server: std::sync::Mutex<Option<Arc<CodexServer>>>,
}

type Waiters<K> = std::sync::Mutex<std::collections::HashMap<K, std::sync::mpsc::Sender<Value>>>;

/// A running `codex app-server`. Its reader thread hands each reply to the
/// request that asked and each notification to its thread's conversation.
pub struct CodexServer {
    child: std::sync::Mutex<Child>,
    stdin: std::sync::Mutex<ChildStdin>,
    next: std::sync::atomic::AtomicU64,
    replies: Waiters<u64>,
    threads: Waiters<String>,
    alive: std::sync::atomic::AtomicBool,
}

impl CodexServer {
    fn spawn(command: &str, dir: &std::path::Path) -> Result<Arc<Self>, Fail> {
        let mut child = Command::new(command)
            .arg("app-server")
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| Fail::Other(format!("{command}: {e}")))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let server = Arc::new(Self {
            child: std::sync::Mutex::new(child),
            stdin: std::sync::Mutex::new(stdin),
            next: Default::default(),
            replies: Default::default(),
            threads: Default::default(),
            alive: std::sync::atomic::AtomicBool::new(true),
        });
        let s = server.clone();
        std::thread::spawn(move || s.read(stdout));
        server.call(
            "initialize",
            json!({"clientInfo": {"name": "hippo", "version": "1"}}),
        )?;
        server.send(json!({"jsonrpc": "2.0", "method": "initialized"}))?;
        Ok(server)
    }

    fn read(&self, mut stdout: BufReader<ChildStdout>) {
        use std::sync::atomic::Ordering;
        let mut buf = String::new();
        loop {
            buf.clear();
            match stdout.read_line(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let Ok(m) = serde_json::from_str::<Value>(&buf) else {
                continue;
            };
            let id = m.get("id").and_then(Value::as_u64);
            match (m.get("method").is_some(), id) {
                (false, Some(id)) => {
                    if let Some(tx) = self.replies.lock().unwrap().remove(&id) {
                        let _ = tx.send(m);
                    }
                }
                // A request of the server's: none comes with approvals off,
                // and one unanswered would stall its turn.
                (true, Some(_)) => {
                    let _ = self.send(json!({"jsonrpc": "2.0", "id": m["id"],
                        "error": {"code": -32601, "message": "hippo answers no requests"}}));
                }
                (true, None) => {
                    let Some(t) = m.pointer("/params/threadId").and_then(Value::as_str) else {
                        continue;
                    };
                    if let Some(tx) = self.threads.lock().unwrap().get(t) {
                        let _ = tx.send(m);
                    }
                }
                (false, None) => {}
            }
        }
        // Every waiter's channel closes, so each call fails at once.
        self.alive.store(false, Ordering::SeqCst);
        self.replies.lock().unwrap().clear();
        self.threads.lock().unwrap().clear();
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
    }

    fn send(&self, msg: Value) -> Result<(), Fail> {
        let mut line = msg.to_string();
        line.push('\n');
        let mut stdin = self.stdin.lock().unwrap();
        stdin
            .write_all(line.as_bytes())
            .and_then(|_| stdin.flush())
            .map_err(|e| Fail::Other(format!("codex exited: {e}")))
    }

    /// Sends a request; with `wait`, its result, an error reply failing it.
    fn request(&self, method: &str, params: Value, wait: bool) -> Result<Value, Fail> {
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let rx = wait.then(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            self.replies.lock().unwrap().insert(id, tx);
            rx
        });
        let sent =
            self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        // A server that died since: its reader has closed every waiter.
        if sent.is_err() || !self.alive.load(std::sync::atomic::Ordering::SeqCst) {
            self.replies.lock().unwrap().remove(&id);
            sent?;
            return Err(Fail::Other("codex exited".into()));
        }
        let Some(rx) = rx else {
            return Ok(Value::Null);
        };
        let reply = rx.recv_timeout(Duration::from_secs(CALL_TIMEOUT));
        self.replies.lock().unwrap().remove(&id);
        let m = reply.map_err(|_| Fail::Other(format!("codex gave no reply to {method}")))?;
        if let Some(e) = m.get("error") {
            let text = e.to_string();
            return Err(limit(&text).unwrap_or(Fail::Other(text)));
        }
        Ok(m["result"].clone())
    }

    fn call(&self, method: &str, params: Value) -> Result<Value, Fail> {
        self.request(method, params, true)
    }
}

impl Drop for CodexCli {
    fn drop(&mut self) {
        if let Some(s) = self.server.lock().unwrap().take() {
            let mut child = s.child.lock().unwrap();
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct CodexChat {
    server: Arc<CodexServer>,
    thread: String,
    events: std::sync::mpsc::Receiver<Value>,
    effort: String,
    model: String,
    used: Usage,
}

impl CodexCli {
    /// The running server, started if there is none or it died.
    fn server(&self, dir: &std::path::Path) -> Result<Arc<CodexServer>, Fail> {
        let mut held = self.server.lock().unwrap();
        if let Some(s) = held.as_ref()
            && s.alive.load(std::sync::atomic::Ordering::SeqCst)
        {
            return Ok(s.clone());
        }
        let s = CodexServer::spawn(&self.command, dir)?;
        *held = Some(s.clone());
        Ok(s)
    }
}

impl Backend for CodexCli {
    fn start(&self) -> Result<Box<dyn Chat>, Fail> {
        let dir = std::env::temp_dir().join("hippo-codex");
        let _ = std::fs::create_dir_all(&dir);
        let server = self.server(&dir)?;
        let started = server.call(
            "thread/start",
            json!({"model": self.model, "baseInstructions": self.prompt,
                "ephemeral": true, "sandbox": "read-only", "approvalPolicy": "never",
                "cwd": dir}),
        )?;
        let thread = started
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| Fail::Other(format!("codex started no thread: {started}")))?
            .to_owned();
        let (tx, events) = std::sync::mpsc::channel();
        server.threads.lock().unwrap().insert(thread.clone(), tx);
        Ok(Box::new(CodexChat {
            server,
            thread,
            events,
            effort: self.effort.clone(),
            model: self.model.clone(),
            used: Usage::default(),
        }))
    }
}

impl Chat for CodexChat {
    fn say(&mut self, blocks: &[String]) -> Result<String, Fail> {
        let input: Vec<Value> = blocks
            .iter()
            .map(|t| json!({"type": "text", "text": t}))
            .collect();
        let params = json!({"threadId": self.thread, "effort": self.effort, "input": input});
        self.server.call("turn/start", params)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(CALL_TIMEOUT);
        let mut text = None;
        let mut last = None;
        let turn = loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let Ok(m) = self.events.recv_timeout(left) else {
                return Err(Fail::Other("codex gave no result".into()));
            };
            let p = &m["params"];
            match m.get("method").and_then(Value::as_str) {
                Some("item/completed")
                    if p.pointer("/item/type") == Some(&json!("agentMessage")) =>
                {
                    text = p
                        .pointer("/item/text")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                Some("thread/tokenUsage/updated") => {
                    last = p.pointer("/tokenUsage/last").cloned();
                }
                Some("turn/completed") => break p["turn"].clone(),
                _ => {}
            }
        };
        if let Some(u) = last {
            let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            self.used.add(Usage {
                input: n("inputTokens").saturating_sub(n("cachedInputTokens")),
                cache_read: n("cachedInputTokens"),
                cache_write: 0,
                output: n("outputTokens"),
                ..Usage::default()
            });
        }
        match text {
            Some(t) if turn["status"] == "completed" => Ok(t),
            _ => {
                let why = format!("codex turn {}: {}", turn["status"], turn["error"]);
                Err(limit(&why).unwrap_or(Fail::Other(why)))
            }
        }
    }

    fn model(&self) -> String {
        self.model.clone()
    }

    fn usage(&self) -> Usage {
        self.used
    }
}

impl Drop for CodexChat {
    /// The server unloads the thread; no reply is waited for.
    fn drop(&mut self) {
        self.server.threads.lock().unwrap().remove(&self.thread);
        let _ = self.server.request(
            "thread/unsubscribe",
            json!({"threadId": self.thread}),
            false,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_the_last_whole_block_of_the_view() {
        let lines: Vec<String> = (0..9).map(|k| format!("line {k}")).collect();
        let view = format!("<chat>\n{}\n</chat>", lines.join("\n"));
        // 11 lines: two whole blocks and a tail of three.
        let blocks = marked(&[view.clone(), "step".into()]);
        assert_eq!(blocks.len(), 4);
        let text: String = blocks[..3]
            .iter()
            .map(|b| b["text"].as_str().unwrap())
            .collect();
        assert_eq!(text, view);
        assert!(blocks[0].get("cache_control").is_none());
        assert_eq!(blocks[1]["cache_control"]["type"], "ephemeral");
        assert_eq!(blocks[1]["text"], "line 3\nline 4\nline 5\nline 6\n");
        assert!(blocks[2].get("cache_control").is_none());
        assert!(blocks[3].get("cache_control").is_none());
        // A short view, or a retry, goes unmarked.
        let short = marked(&["<chat>\n0+1|a\n</chat>".into()]);
        assert!(short.iter().all(|b| b.get("cache_control").is_none()));
        assert_eq!(marked(&["Too long: 600 bytes".into()]).len(), 1);
    }

    /// Kept processes against one per call, for the configured backend
    /// (HIPPO_BACKEND, HIPPO_MODEL, HIPPO_EFFORT), by hand: `cargo test
    /// --release -- --ignored --nocapture kept_against_cold`. Calls run one
    /// at a time, `HIPPO_MEASURE_CALLS` of them (4) each way; CPU is the
    /// model processes', counted once they are reaped.
    #[test]
    #[ignore]
    fn kept_against_cold() {
        let calls: usize = std::env::var("HIPPO_MEASURE_CALLS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4);
        let context: Vec<String> = (0..120)
            .map(|k| format!("{}+1|user: note {k}: the host's deploy of step {k} went through after a retry on the host", 1000 + k))
            .collect();
        let cpu = || {
            let mut u: libc::rusage = unsafe { std::mem::zeroed() };
            unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &mut u) };
            let t = |v: libc::timeval| v.tv_sec as f64 + v.tv_usec as f64 / 1e6;
            t(u.ru_utime) + t(u.ru_stime)
        };
        let step = |k: usize| format!("user: please summarize item {k} of the deploy log");
        let measure = |label: &str, kept: bool| {
            let (c0, t0) = (cpu(), std::time::Instant::now());
            let shared = kept.then(|| from_env().unwrap());
            let (mut each, mut used) = (Vec::new(), Usage::default());
            for k in 0..calls {
                let fresh = (!kept).then(|| from_env().unwrap());
                let b = shared.as_deref().or(fresh.as_deref()).unwrap();
                let msg = step(k);
                let t = std::time::Instant::now();
                let r = run(
                    b,
                    None,
                    &context,
                    &Step::Compress {
                        id: 2000 + k as u64,
                        msg: &msg,
                    },
                );
                each.push(t.elapsed().as_secs_f64());
                match r {
                    Ok((_, _, u)) => used.add(u),
                    Err(e) => eprintln!("{label} call {k}: {}", e.text()),
                }
            }
            drop(shared);
            std::thread::sleep(Duration::from_millis(500));
            let fmt: Vec<String> = each.iter().map(|s| format!("{s:.1}")).collect();
            eprintln!(
                "{label}: {calls} calls in {:.1}s, each [{}]s, model processes' CPU {:.1}s ({:.2}s a call), tokens in {} cache read {} written {}",
                t0.elapsed().as_secs_f64(),
                fmt.join(" "),
                cpu() - c0,
                (cpu() - c0) / calls as f64,
                used.input,
                used.cache_read,
                used.cache_write
            );
        };
        measure("cold", false);
        measure("kept", true);
    }

    struct Named(&'static str);
    impl Backend for Named {
        fn start(&self) -> Result<Box<dyn Chat>, Fail> {
            Err(Fail::Other(self.0.into()))
        }
    }

    #[test]
    fn levels_pick_a_backend_by_tree_level() {
        let levels = Levels {
            base: Box::new(Named("base")),
            per: vec![(0, Box::new(Named("low"))), (1, Box::new(Named("mid")))],
        };
        let name = |l: u8| match levels.for_level(l).unwrap_or(&levels).start() {
            Err(Fail::Other(n)) => n,
            _ => unreachable!(),
        };
        assert_eq!(name(0), "low");
        assert_eq!(name(1), "mid");
        assert_eq!(name(2), "base");
        assert!(Named("x").for_level(0).is_none());
    }

    struct Wordy;
    struct WordyChat;
    impl Chat for WordyChat {
        fn say(&mut self, _blocks: &[String]) -> Result<String, Fail> {
            Ok(format!("talk: {}", "s".repeat(2 << 20)))
        }
        fn model(&self) -> String {
            "wordy".into()
        }
    }
    impl Backend for Wordy {
        fn start(&self) -> Result<Box<dyn Chat>, Fail> {
            Ok(Box::new(WordyChat))
        }
    }

    #[test]
    fn a_model_that_will_not_shorten_keeps_its_shortest_line() {
        let step = Step::Compress {
            id: 0,
            msg: "talk: some words",
        };
        let (line, _, used) = run(&Wordy, None, &[], &step).unwrap();
        assert!(line.len() > NODE, "UniiChat cuts no line it kept");
        assert_eq!(used.calls, TRIES as u64);
    }

    #[test]
    fn cut_respects_characters() {
        assert_eq!(cut("aé", 2), "a");
        assert_eq!(cut("abc", 10), "abc");
    }

    #[test]
    fn refusals_are_recognized() {
        let msg = "API Error: Sonnet 5.5's safeguards flagged this message (https://www.anthropic.com/legal/aup). Claude Code can't respond to this message with Sonnet 5.5.";
        assert!(matches!(refused(msg), Some(Fail::Refused(_))));
        assert!(limit(msg).is_none());
        assert!(refused("connection reset").is_none());
    }

    /// As Libri passed it on at 20:44 on 2026-10-30.
    const CYBER: &str = "claude: allow us to deliver more capabilities faster, but can sometimes flag legitimate cybersecurity work. Learn more: https://support.claude.com/en/articles/14604842-real-time-cyber-safeguards-on-claude\nDetails: `[cyber]`\nRequest ID: req_011CgZPaXocLwb9CPY5kfj4d";

    #[test]
    fn a_cyber_refusal_is_a_refusal_and_no_limit() {
        assert!(matches!(refused(CYBER), Some(Fail::Refused(_))));
        assert!(limit(CYBER).is_none());
    }

    #[test]
    fn only_a_true_limit_reads_as_one() {
        for e in [
            "claude exited 1: could not generate a reply",
            "Invalid model: unlimited plan required",
            "a moderate failure in separate code",
            "claude-haiku-5-5 can't run here: claude is not connected.",
        ] {
            assert!(limit(e).is_none(), "{e}");
        }
        for e in [
            "429 Too Many Requests",
            "Error: rate limit exceeded",
            "The model is overloaded",
            "insufficient_quota",
        ] {
            assert!(limit(e).is_some(), "{e}");
        }
    }

    /// Serves `reply` once on a socket in a scratch directory, as Libri does.
    fn libri_says(name: &str, reply: Option<&'static str>) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("hippo-libri-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let sock = d.join("compactor.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&conn).read_line(&mut line).unwrap();
            if let Some(reply) = reply {
                writeln!(conn, "{reply}").unwrap();
            }
        });
        sock
    }

    fn ask(sock: std::path::PathBuf) -> Result<String, Fail> {
        let libri = Libri {
            prompt: String::new(),
            sock,
            model: MODEL.into(),
            effort: EFFORT.into(),
        };
        libri.start()?.say(&["memory".into(), "prompt".into()])
    }

    #[test]
    fn a_call_through_libri_says_whose_memory_it_is_for() {
        let d = std::env::temp_dir().join(format!("hippo-libri-whose-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let sock = d.join("compactor.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let (heard, said) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut conn, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(&conn).read_line(&mut line).unwrap();
                heard.send(line).unwrap();
                writeln!(conn, r#"{{"text":"user: a line","model":"m"}}"#).unwrap();
            }
        });
        let libri = Libri {
            prompt: String::new(),
            sock,
            model: MODEL.into(),
            effort: EFFORT.into(),
        };
        let blocks = ["memory".to_owned(), "prompt".to_owned()];
        libri
            .start_for(Some("gil-1"))
            .unwrap()
            .say(&blocks)
            .unwrap();
        let call: Value = serde_json::from_str(&said.recv().unwrap()).unwrap();
        assert_eq!(call["worker"], "gil-1");
        assert_eq!(
            (&call["model"], &call["effort"]),
            (&json!("haiku"), &json!("high"))
        );
        libri.start().unwrap().say(&blocks).unwrap();
        let call: Value = serde_json::from_str(&said.recv().unwrap()).unwrap();
        assert!(call["worker"].is_null());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn libri_failures_stay_what_they_are() {
        // The outage of 2026-10-30: a socket with no listener behind it.
        let refused_sock = {
            let sock = libri_says("gone", None);
            let d = sock.parent().unwrap().join("dead");
            drop(std::os::unix::net::UnixListener::bind(&d).unwrap());
            d
        };
        let e = ask(refused_sock).unwrap_err();
        assert!(matches!(e, Fail::Unreachable(_)), "{e:?}");
        assert!(e.text().contains("refused"), "{}", e.text());
        assert_eq!(e.kind(), "unreachable");

        let missing = std::env::temp_dir().join("hippo-libri-no-such-socket");
        assert!(matches!(ask(missing), Err(Fail::Unreachable(_))));

        // Libri going down in the middle of a call.
        let e = ask(libri_says("hangup", None)).unwrap_err();
        assert!(matches!(e, Fail::Unreachable(_)), "{e:?}");

        let e = ask(libri_says(
            "model",
            Some(r#"{"error":"claude exited 1: boom"}"#),
        ))
        .unwrap_err();
        assert!(matches!(e, Fail::Model(_)), "{e:?}");
        assert_eq!(e.kind(), "model");

        let reply = serde_json::to_string(&json!({ "error": CYBER })).unwrap();
        let reply: &'static str = Box::leak(reply.into_boxed_str());
        let e = ask(libri_says("refused", Some(reply))).unwrap_err();
        assert!(matches!(e, Fail::Refused(_)), "{e:?}");

        let e = ask(libri_says(
            "limit",
            Some(r#"{"error":"Claude AI usage limit reached|1791400000"}"#),
        ))
        .unwrap_err();
        assert!(matches!(e, Fail::Limit(Some(_))), "{e:?}");
        assert_eq!(e.kind(), "rate_limit");
    }

    #[test]
    fn libri_carries_what_the_cache_served() {
        let sock = libri_says(
            "usage",
            Some(
                r#"{"text":"line","model":"m","usage":{"input":2,"cache_read":13030,"cache_write":1172,"output":130}}"#,
            ),
        );
        let libri = Libri {
            prompt: String::new(),
            sock,
            model: MODEL.into(),
            effort: EFFORT.into(),
        };
        let mut chat = libri.start().unwrap();
        chat.say(&["memory".into(), "prompt".into()]).unwrap();
        let u = chat.usage();
        assert_eq!(
            (u.input, u.cache_read, u.cache_write, u.output),
            (2, 13030, 1172, 130)
        );
    }

    #[test]
    fn limits_are_recognized() {
        assert!(matches!(
            limit("Claude AI usage limit reached|1791400000"),
            Some(Fail::Limit(Some(_)))
        ));
        assert!(matches!(
            limit("You've hit your limit · resets 3pm"),
            Some(Fail::Limit(None))
        ));
        assert!(limit("Invalid model").is_none());
    }

    /// Answers from a script, recording what it was asked.
    struct Scripted(Mutex<Vec<String>>, Mutex<Vec<Vec<String>>>);
    struct ScriptedChat<'a>(&'a Scripted);
    impl Chat for ScriptedChat<'_> {
        fn say(&mut self, blocks: &[String]) -> Result<String, Fail> {
            self.0.1.lock().unwrap().push(blocks.to_vec());
            Ok(self.0.0.lock().unwrap().remove(0))
        }
        fn model(&self) -> String {
            "scripted".into()
        }
    }
    impl Backend for &'static Scripted {
        fn start(&self) -> Result<Box<dyn Chat>, Fail> {
            Ok(Box::new(ScriptedChat(self)))
        }
    }

    #[test]
    fn a_copied_head_is_cut_off() {
        let s: &'static Scripted = Box::leak(Box::new(Scripted(
            Mutex::new(vec!["12+1|talk: hello".into()]),
            Mutex::new(Vec::new()),
        )));
        let step = Step::Compress {
            id: 12,
            msg: "talk: hello",
        };
        let (line, _, used) = run(&s, None, &[], &step).unwrap();
        assert_eq!(line, "talk: hello");
        assert_eq!(used.calls, 1);
        assert_eq!(behead("12 talk: hi"), "talk: hi");
        assert_eq!(behead("12: talk: hi"), "talk: hi");
        assert_eq!(behead("12+2|user: a"), "user: a");
        assert_eq!(behead("user: 12 items"), "user: 12 items");
        assert_eq!(behead("11-04 87456-87487: user: a"), "user: a");
        assert_eq!(behead("84216-19 (11-04): talk: a"), "talk: a");
        assert_eq!(behead("(11-04 22:33): talk: a"), "talk: a");
        assert_eq!(behead("11-03 19:3x: work: a"), "work: a");
        assert_eq!(behead("[83800-01] tool: a"), "tool: a");
        assert_eq!(behead("83420+1|talk 83420 (11-03)"), "talk 83420 (11-03)");
        assert_eq!(
            behead("(11-04 to 11-06): talk: a"),
            "(11-04 to 11-06): talk: a"
        );
    }

    use std::sync::Mutex;

    #[test]
    fn retries_over_the_limit_and_keeps_the_shortest() {
        let long = |n: usize| format!("talk: {}", "x".repeat(n - 6));
        let s: &'static Scripted = Box::leak(Box::new(Scripted(
            Mutex::new(vec![long(600), long(570), long(580), long(560), long(550)]),
            Mutex::new(Vec::new()),
        )));
        let (line, model, _) = run(
            &s,
            None,
            &["0+1|a".into()],
            &Step::Compress {
                id: 1,
                msg: "talk [x]: hello",
            },
        )
        .unwrap();
        assert_eq!(line.len(), 550);
        assert_eq!(model, "scripted");
        let asked = s.1.lock().unwrap();
        assert_eq!(asked.len(), TRIES);
        assert_eq!(asked[0][0], "<chat>\n0+1|a\n</chat>");
        assert!(asked[0][1].contains(&format!("ruler:\n{}\n<input>", "-".repeat(NODE))));
        assert!(asked[0][1].ends_with("<input>\ntalk [x]: hello\n</input>"));
        // Each try sends the first call's blocks and only the last note.
        assert_eq!(asked[1].len(), 3);
        assert_eq!(asked[1][..2], asked[0][..]);
        assert!(
            asked[1][2].starts_with("Too long: your last line for this <input> was 600 bytes,")
        );
        assert!(asked[1][2].ends_with(&format!("{}| ← LIMIT", long(512))));
        assert!(asked[2][2].contains("was 570 bytes"));
    }

    #[test]
    fn a_line_one_byte_over_is_asked_again() {
        let s: &'static Scripted = Box::leak(Box::new(Scripted(
            Mutex::new(vec![
                format!("talk: {}", "x".repeat(NODE - 5)),
                "talk: short".into(),
            ]),
            Mutex::new(Vec::new()),
        )));
        let step = Step::Compress {
            id: 1,
            msg: "talk [x]: hello",
        };
        let (line, _, used) = run(&s, None, &[], &step).unwrap();
        assert_eq!(line, "talk: short");
        assert_eq!(used.calls, 2);
    }

    #[test]
    fn no_line_is_a_refusal_and_its_source_clipped() {
        let s: &'static Scripted = Box::leak(Box::new(Scripted(
            Mutex::new(vec!["  ".into()]),
            Mutex::new(Vec::new()),
        )));
        let step = Step::Compress {
            id: 1,
            msg: &"echo: long\n output ".repeat(100),
        };
        assert!(matches!(run(&s, None, &[], &step), Err(Fail::Refused(_))));
        let line = clipped(&step);
        assert_eq!(line.len(), NODE);
        assert!(!line.contains('\n'));
        let merge = Step::Merge {
            l: 1,
            i: 0,
            a: "user: a",
            b: "talk: b",
        };
        assert_eq!(clipped(&merge), "user: a talk: b");
    }

    #[test]
    fn tasks_are_unii_leaf_and_merge() {
        let merge = input(
            &[],
            &Step::Merge {
                l: 2,
                i: 3,
                a: "user: a",
                b: "talk: b",
            },
        )[1]
        .clone();
        let ruler = "-".repeat(NODE);
        assert_eq!(
            merge,
            format!(
                "Compaction: merge lines 12+2 and 14+2, adjacent, into one line of at most\n512 bytes (about 70 words), the length of this ruler:\n{ruler}\n<chat> may hold their messages, 12 to 15, in more detail: take details\nof them from there too.\n<input>\nuser: a\ntalk: b\n</input>"
            )
        );
        let leaf = input(
            &[],
            &Step::Compress {
                id: 7,
                msg: "user: hi",
            },
        )[1]
        .clone();
        assert_eq!(
            leaf,
            format!(
                "Compaction: compress message 7 into one line of at most 512 bytes\n(about 70 words), the length of this ruler:\n{ruler}\n<input>\nuser: hi\n</input>"
            )
        );
    }
}
