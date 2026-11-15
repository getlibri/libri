//! Libri's memory: its own hippo service, asked over `hippo.sock` as the
//! hippo CLI does. hippo stores every message and folds them into the view.

use crate::model::{Step, said};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// How long hippo may be silent before the chat hears: a restart is over
/// well within this.
const WARN_AFTER: Duration = Duration::from_secs(180);
/// How long a health read waits: a status that hangs is a status not answering.
const HEALTH_WAIT: Duration = Duration::from_secs(5);
/// How long a turn waits on hippo before going on without it.
#[cfg(not(test))]
const TURN_WAIT: Duration = Duration::from_secs(12);
#[cfg(test)]
const TURN_WAIT: Duration = Duration::from_millis(200);
/// A view may wait 30 s on the compactor settling, which is hippo working.
#[cfg(not(test))]
const VIEW_WAIT: Duration = Duration::from_secs(40);
#[cfg(test)]
const VIEW_WAIT: Duration = Duration::from_millis(200);
/// After hippo fails a turn, how long before asking it again: a turn of many
/// steps would otherwise wait `TURN_WAIT` at each.
#[cfg(not(test))]
const REST: Duration = Duration::from_secs(15);
#[cfg(test)]
const REST: Duration = Duration::ZERO;
/// The spool's copies of what hippo last gave.
const VIEW: &str = "view.last";
const TAIL: &str = "tail.last";
/// How much of the words held back a view carries.
const HELD_IN_VIEW: usize = 32 * 1024;

/// How long a view or a save waits with no spool to go on with: past the
/// compactor's half minute, and then a frozen hippo fails the call plainly
/// instead of holding it for good.
const NO_SPOOL_WAIT: Duration = if cfg!(test) {
    Duration::from_secs(1)
} else {
    Duration::from_secs(60)
};

#[derive(Clone)]
pub struct Hippo {
    pub sock: PathBuf,
    /// The title of this daemon's chat, in a hippo that keeps others too.
    pub chat: Option<String>,
    /// What lets a daemon's turns go on without hippo; none in a passing
    /// command, whose failure is its own to report.
    pub spool: Option<std::sync::Arc<Spool>>,
}

/// A daemon's own record of the memory, for when hippo fails: the last view it
/// gave, and each word since, fsynced as the store does, until hippo has it.
/// The write-ahead log's pattern: nothing is acknowledged before it is on
/// disk, and replay is safe to repeat because each entry carries its key.
pub struct Spool {
    dir: PathBuf,
    /// When to ask hippo again; holding this keeps entries in order.
    rest: Mutex<Option<std::time::Instant>>,
    /// What the person has yet to hear, or has heard, of this outage.
    told: AtomicBool,
    back: AtomicBool,
    unkept: AtomicBool,
}

#[derive(serde::Serialize, Deserialize)]
struct Entry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display: Option<Value>,
    key: String,
    date: String,
    kind: String,
    text: String,
}

impl Entry {
    fn said(self, i: u64) -> Said {
        Said {
            display: self.display,
            i,
            kind: self.kind,
            text: self.text,
            date: self.date,
            src: Some(Src { e: self.key }),
        }
    }
}

impl Spool {
    pub fn new(dir: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Spool {
            dir,
            rest: Mutex::new(None),
            told: Default::default(),
            back: Default::default(),
            unkept: Default::default(),
        })
    }

    fn entries(&self) -> PathBuf {
        self.dir.join("words.jsonl")
    }

    fn last(&self, what: &str) -> String {
        std::fs::read_to_string(self.dir.join(what)).unwrap_or_default()
    }

    fn entry(&self, key: &str, kind: &str, text: &str) -> Entry {
        let now = chrono::Local::now();
        Entry {
            display: None,
            key: key.to_owned(),
            date: now.to_rfc3339(),
            kind: kind.to_owned(),
            text: text.to_owned(),
        }
    }

    fn hold(&self, entry: &Entry) -> Result<(), String> {
        let path = self.entries();
        let fail = |e: std::io::Error| format!("{}: {e}", path.display());
        let mut line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
        line.push('\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(fail)?;
        file.write_all(line.as_bytes()).map_err(fail)?;
        file.sync_data().map_err(fail)
    }

    /// The words held, oldest first; a line cut by a crash was never
    /// acknowledged, so it is passed over.
    fn held(&self) -> Vec<Entry> {
        let raw = std::fs::read_to_string(self.entries()).unwrap_or_default();
        raw.lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn remember(&self, what: &str, text: &str) {
        let _order = self.rest.lock().unwrap();
        let path = self.dir.join(what);
        let tmp = path.with_extension("tmp");
        let kept = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &path));
        if let Err(e) = kept {
            eprintln!("libri: {}: {e}", path.display());
        }
    }

    /// The last view with the words held since, so a model answering while
    /// hippo is away still has the conversation.
    fn last_view(&self) -> String {
        let view = self.last(VIEW);
        let mut words = String::new();
        for e in self.held().iter().rev() {
            if words.len() > HELD_IN_VIEW {
                break;
            }
            words.insert_str(0, &format!("{}: {}\n", e.kind, e.text));
        }
        format!(
            "{view}\n(The memory isn't answering: this view is its last, and these are the words since.)\n{words}"
        )
    }
}

/// The memory's health for the site. A brief silence from hippo, a restart
/// say, is no failure: its last answer stands until the silence outlasts
/// `WARN_AFTER`, and a silence is counted from that answer.
pub struct Health {
    hippo: std::sync::Arc<Hippo>,
    heard: Mutex<(chrono::DateTime<chrono::Local>, Option<Value>)>,
}

impl Health {
    pub fn new(hippo: std::sync::Arc<Hippo>) -> Self {
        Health {
            hippo,
            heard: Mutex::new((chrono::Local::now(), None)),
        }
    }

    pub fn read(&self) -> Value {
        let answer = self.hippo.health();
        let mut heard = self.heard.lock().unwrap();
        let now = chrono::Local::now();
        match answer {
            Ok(health) => {
                *heard = (now, Some(health.clone()));
                health
            }
            Err(_) if (now - heard.0).to_std().is_ok_and(|d| d >= WARN_AFTER) => {
                silent("failing", Some(heard.0))
            }
            Err(_) => heard.1.clone().unwrap_or_else(|| silent("working", None)),
        }
    }
}

/// What can be said of a hippo that doesn't answer.
fn silent(state: &str, since: Option<chrono::DateTime<chrono::Local>>) -> Value {
    json!({
        "state": state,
        "reason": since.map(|_| "The memory isn't answering."),
        "since": since.map(|t| t.to_rfc3339()),
        "waiting": null,
        "newest_summary": null,
        "latest_message": null,
        "model": null,
        "lines": null,
        "bytes": null,
        "last_error": null,
        "retries_last_hour": null,
    })
}

#[derive(Deserialize)]
struct Reply {
    ok: bool,
    out: String,
}

impl Hippo {
    fn ask(&self, cmd: &str, args: &[&str]) -> Result<String, String> {
        self.ask_within(cmd, args, None)
    }

    fn ask_within(
        &self,
        cmd: &str,
        args: &[&str],
        wait: Option<Duration>,
    ) -> Result<String, String> {
        let mut conn = UnixStream::connect(&self.sock)
            .map_err(|e| format!("hippo at {}: {e}", self.sock.display()))?;
        conn.set_read_timeout(wait)
            .and_then(|()| conn.set_write_timeout(wait))
            .map_err(|e| e.to_string())?;
        let mut raw = json!({"cmd": cmd, "args": args}).to_string();
        raw.push('\n');
        conn.write_all(raw.as_bytes()).map_err(|e| e.to_string())?;
        let mut line = String::new();
        BufReader::new(&conn)
            .read_line(&mut line)
            .map_err(|e| match e.kind() {
                ErrorKind::WouldBlock | ErrorKind::TimedOut => {
                    format!("hippo did not answer {cmd} in time")
                }
                _ => e.to_string(),
            })?;
        let reply: Reply = serde_json::from_str(&line).map_err(|e| format!("hippo: {e}"))?;
        if reply.ok {
            Ok(reply.out)
        } else {
            Err(format!("hippo: {}", reply.out))
        }
    }

    /// How the memory is working, as hippo's `health` says it.
    pub fn health(&self) -> Result<Value, String> {
        let out = self.ask_within("health", &[], Some(HEALTH_WAIT))?;
        serde_json::from_str(&out).map_err(|e| format!("hippo: {e}"))
    }

    /// The whole view, `<chat>` to `</chat>`, once the compactor settles.
    /// With a spool it never fails: past `TURN_WAIT` the turn goes on with
    /// the last view hippo gave.
    pub fn view(&self) -> Result<String, String> {
        let Some(spool) = &self.spool else {
            return self.ask_within("view", &["whole"], Some(NO_SPOOL_WAIT));
        };
        // Unlocked while asking, or a slow view would hold up the words of
        // every other turn.
        let ready = self.replay_held(spool, &mut spool.rest.lock().unwrap());
        if ready {
            match self.ask_within("view", &["whole"], Some(VIEW_WAIT)) {
                Ok(view) => {
                    spool.remember(VIEW, &view);
                    return Ok(view);
                }
                Err(e) => {
                    eprintln!("libri: {e}");
                    *spool.rest.lock().unwrap() = Some(std::time::Instant::now() + REST);
                }
            }
        }
        Ok(spool.last_view())
    }

    /// Keeps one message in this daemon's chat under a new key, which it
    /// answers; hippo gives that key back with the row.
    pub fn save(&self, kind: &str, text: &str) -> Result<String, String> {
        self.save_row(kind, text).map(|(key, _)| key)
    }

    /// As `save`, with hippo's row for the words when it took them now.
    pub fn save_row(&self, kind: &str, text: &str) -> Result<(String, Option<u64>), String> {
        let key = row_key();
        self.save_as(&key, kind, text).map(|row| (key, row))
    }

    /// Keeps one message under `key`, or says why it could not. With a
    /// spool, words hippo can't take now are held for it: an error here
    /// means they are on no disk at all. A key kept before keeps nothing.
    /// Answers hippo's row for the words when it took them now; words held
    /// for it, or kept before, have none yet.
    pub fn save_as(&self, key: &str, kind: &str, text: &str) -> Result<Option<u64>, String> {
        self.save_display_as(key, kind, text, None)
    }

    fn save_display_as(
        &self,
        key: &str,
        kind: &str,
        text: &str,
        display: Option<Value>,
    ) -> Result<Option<u64>, String> {
        let Some(spool) = self.spool.as_ref().filter(|_| !text.trim().is_empty()) else {
            if display.is_some() {
                return self.keep(&Entry {
                    key: key.into(),
                    date: chrono::Local::now().to_rfc3339(),
                    kind: kind.into(),
                    text: text.into(),
                    display,
                });
            }
            return match &self.chat {
                Some(title) => self.ask_within("post", &[title, kind, text], Some(NO_SPOOL_WAIT)),
                None => self.ask_within("log", &[kind, text], Some(NO_SPOOL_WAIT)),
            }
            .map(|out| row_of(&out));
        };
        let mut entry = spool.entry(key, kind, text);
        entry.display = display;
        let mut rest = spool.rest.lock().unwrap();
        if self.replay_held(spool, &mut rest) {
            match self.keep(&entry) {
                Ok(row) => return Ok(row),
                Err(e) => {
                    eprintln!("libri: {e}");
                    *rest = Some(std::time::Instant::now() + REST);
                }
            }
        }
        let held = spool.hold(&entry);
        if held.is_err() {
            spool.unkept.store(true, Ordering::Relaxed);
        }
        held.map(|()| None)
    }

    fn keep(&self, e: &Entry) -> Result<Option<u64>, String> {
        let title = self.chat.as_deref().unwrap_or_default();
        let args = [e.key.as_str(), &e.date, title, &e.kind, &e.text];
        let record;
        let mut args = args.to_vec();
        let cmd = if let Some(display) = &e.display {
            record = display.to_string();
            args.insert(4, &record);
            "keep_display"
        } else {
            "keep"
        };
        self.ask_within(cmd, &args, Some(TURN_WAIT))
            .map(|out| row_of(&out))
    }

    /// Whether hippo has all the words held, sending what it can: their keys
    /// make a second sending harmless, so the file goes only once all are in.
    fn replay_held(&self, spool: &Spool, rest: &mut Option<std::time::Instant>) -> bool {
        if rest.is_some_and(|t| std::time::Instant::now() < t) {
            return false;
        }
        let held = spool.held();
        for e in &held {
            if let Err(err) = self.keep(e) {
                eprintln!("libri: {err}");
                *rest = Some(std::time::Instant::now() + REST);
                return false;
            }
        }
        if !held.is_empty() {
            eprintln!("libri: {} held words given back to hippo", held.len());
            spool.told.store(false, Ordering::Relaxed);
            spool.back.store(true, Ordering::Relaxed);
            if let Err(e) = std::fs::remove_file(spool.entries()) {
                eprintln!("libri: {e}");
            }
        }
        *rest = None;
        true
    }

    /// Hands over what is held, between turns.
    pub fn replay(&self) {
        if let Some(spool) = &self.spool {
            let mut rest = spool.rest.lock().unwrap();
            if spool.entries().exists() {
                self.replay_held(spool, &mut rest);
            }
        }
    }

    /// Whether words wait for hippo.
    #[cfg(test)]
    fn holding(&self) -> bool {
        self.spool.as_ref().is_some_and(|s| s.entries().exists())
    }

    /// What a person should hear of the memory now: once when it goes, once
    /// when it's back, and each time words could not be kept.
    pub fn notice(&self) -> Option<&'static str> {
        let spool = self.spool.as_ref()?;
        if spool.unkept.swap(false, Ordering::Relaxed) {
            Some("(libri) My memory is down and I couldn't keep what was just said anywhere.")
        } else if spool.entries().exists() && !spool.told.swap(true, Ordering::Relaxed) {
            Some(
                "(libri) My memory is down. I'm working from my last view of it. Your words are held on this machine and go in when it's back.",
            )
        } else if spool.back.swap(false, Ordering::Relaxed) {
            Some("(libri) My memory is back, and the words held meanwhile are in.")
        } else {
            None
        }
    }

    /// A subagent's whole chat, `{i}|{kind}: {text}` per message, from its
    /// name or id. Its log is read by the hippo command, as the server
    /// keeps only the report.
    pub fn agent(&self, id: &str) -> Result<String, String> {
        let known = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
        if id.is_empty() || !id.chars().all(known) {
            return Err(format!("No agent {id}."));
        }
        let out = std::process::Command::new("hippo")
            .args(["zoom", id])
            .output()
            .map_err(|e| format!("hippo: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
        }
    }

    /// Forks this memory for worker `id`, not yet at work, or with `head`
    /// that worker's memory, one level down.
    pub fn fork(&self, id: &str, head: Option<&str>) -> Result<String, String> {
        match head {
            None => self.ask_within("fork", &[id], Some(TURN_WAIT)),
            Some(head) => self.ask_within("fork", &[id, head], Some(TURN_WAIT)),
        }
    }

    /// What a run of worker `id` reads, as `<work>`: its memory, built as
    /// this one is, from here on as its log grows.
    /// With it, whether that memory is a fork of this one, as it stands
    /// now: a fork that landed after its spawn gave up waiting counts.
    pub fn work(&self, id: &str) -> Result<(String, Option<Fork>), String> {
        work_answer(&self.ask_within("work", &[id], Some(VIEW_WAIT))?)
    }

    /// Brings worker `id`'s messages since its fork, or since its last
    /// merge, into this memory as this chat's own, after what it has;
    /// hippo's answer.
    pub fn merge(&self, id: &str, name: &str) -> Result<String, String> {
        let args: Vec<&str> = [id, name].into_iter().chain(self.chat.as_deref()).collect();
        self.ask_within("merge", &args, Some(VIEW_WAIT))
    }

    /// Opens line `line+n` of worker `id`'s memory, as `zoom`.
    pub fn work_zoom(&self, id: &str, line: u64, n: u64) -> Result<String, String> {
        self.ask("work-zoom", &[id, &line.to_string(), &n.to_string()])
    }

    /// Every entry of worker `id`'s log matching a regex, as `search`.
    pub fn work_search(&self, id: &str, regex: &str) -> Result<String, String> {
        self.ask("work-search", &[id, regex])
    }

    /// Drops what is older than `minutes`, oldest first; how many messages
    /// went.
    pub fn forget(&self, minutes: u64) -> Result<u64, String> {
        let out = self.ask("forget", &[&minutes.to_string()])?;
        out.split_whitespace()
            .nth(1)
            .and_then(|n| n.parse().ok())
            .ok_or_else(|| format!("hippo forget: {out}"))
    }

    /// Every message matching a case-insensitive regex, newest last.
    pub fn search(&self, regex: &str) -> Result<String, String> {
        self.ask("search", &[regex])
    }

    /// Main's messages and the workers' lines matching words, ranked
    /// (`best`) or newest first (`new`), as hippo's JSON.
    pub fn find(&self, words: &str, order: &str) -> Result<String, String> {
        self.ask("find", &[words, order])
    }

    /// Lines of a source around one, as hippo's JSON; source "" is main.
    pub fn around(&self, src: &str, at: u64, before: u64, after: u64) -> Result<String, String> {
        let n = [at, before, after].map(|n| n.to_string());
        self.ask("around", &[src, &n[0], &n[1], &n[2]])
    }

    /// The workers `find` reaches, as hippo's JSON.
    pub fn sources(&self) -> Result<String, String> {
        self.ask("sources", &[])
    }

    /// When message `id` was sent.
    pub fn date(&self, id: u64) -> Result<String, String> {
        self.ask("date", &[&id.to_string()])
    }

    /// As `tail`, from the memory itself and never a page's copy of it, for
    /// what decides on it: waits up to `within` for a hippo that is still
    /// starting, as after a restart, and counts in the words held for it.
    pub fn tail_kept(&self, n: u64, within: Duration) -> Result<Vec<Said>, String> {
        let end = std::time::Instant::now() + within;
        let n = n.to_string();
        let args: Vec<&str> = std::iter::once(n.as_str())
            .chain(self.chat.as_deref())
            .collect();
        let out = loop {
            self.replay();
            match self.ask_within("tail", &args, Some(TURN_WAIT)) {
                Ok(out) => break out,
                Err(e) if std::time::Instant::now() >= end => return Err(e),
                Err(_) => std::thread::sleep(Duration::from_millis(200)),
            }
        };
        let mut all = said_lines(&out)?;
        if let Some(spool) = &self.spool {
            let next = all.last().map_or(0, |s| s.i + 1);
            let held = spool.held().into_iter().enumerate();
            all.extend(held.map(|(k, e)| e.said(next + k as u64)));
        }
        Ok(all)
    }

    /// The last `n` messages whole, oldest first: the seat's own, when
    /// hippo keeps other chats too.
    pub fn tail(&self, n: u64) -> Result<Vec<Said>, String> {
        // A page's record lets go of what the spool holds, so the history
        // has to hold it first.
        self.replay();
        let n = n.to_string();
        let args: Vec<&str> = std::iter::once(n.as_str())
            .chain(self.chat.as_deref())
            .collect();
        let Some(spool) = &self.spool else {
            return said_lines(&self.ask("tail", &args)?);
        };
        // A page that can't load the history can't reach the chat at all.
        match self.ask_within("tail", &args, Some(TURN_WAIT)) {
            Ok(out) => {
                spool.remember(TAIL, &out);
                said_lines(&out)
            }
            Err(e) => {
                eprintln!("libri: {e}");
                let mut all = said_lines(&spool.last(TAIL))?;
                let next = all.last().map_or(0, |s| s.i + 1);
                let held = spool.held().into_iter().enumerate();
                all.extend(held.map(|(k, e)| e.said(next + k as u64)));
                Ok(all)
            }
        }
    }
}

/// hippo's answer to `work`: a line `fork <id>` or `fork none`, then the
/// memory.
/// What a worker's memory forked: main's at `at`, or with `head` the memory
/// of that worker, by id, which forked main's at the second value, if at all.
#[derive(Clone, Debug, PartialEq)]
pub struct Fork {
    pub at: u64,
    pub head: Option<(String, Option<u64>)>,
}

fn work_answer(out: &str) -> Result<(String, Option<Fork>), String> {
    let (head, text) = out.split_once('\n').unwrap_or((out, ""));
    let bad = || "hippo: a worker's memory came with no fork line".to_owned();
    let words: Vec<&str> = head
        .strip_prefix("fork ")
        .ok_or_else(bad)?
        .split(' ')
        .collect();
    let fork = match words.as_slice() {
        ["none"] => None,
        [at] => Some(Fork {
            at: at.parse().map_err(|_| bad())?,
            head: None,
        }),
        [at, head, main] => Some(Fork {
            at: at.parse().map_err(|_| bad())?,
            head: Some(((*head).to_owned(), main.parse().ok())),
        }),
        _ => return Err(bad()),
    };
    Ok((text.to_owned(), fork))
}

/// A key for a message hippo has yet to keep, unique across restarts.
pub fn row_key() -> String {
    static MADE: AtomicU64 = AtomicU64::new(0);
    let made = MADE.fetch_add(1, Ordering::Relaxed);
    let stamp = chrono::Local::now()
        .timestamp_nanos_opt()
        .unwrap_or_default();
    format!("spool-{stamp}-{made}")
}

fn said_lines(raw: &str) -> Result<Vec<Said>, String> {
    raw.lines()
        .map(|l| serde_json::from_str(l).map_err(|e| format!("hippo tail: {e}")))
        .collect()
}

/// Keeps a turn in hippo as it happens, from the steps every harness
/// gives: each call and its result, and the words between them, which a
/// transcript of Claude Code's leaves out. So all harnesses reach the
/// memory one way, each thing once, and a turn cut off by a restart has
/// kept what it did. A harness's reasoning isn't kept: hippo has no kind
/// for it, and transcripts never gave it one.
pub struct Keeper {
    hippo: Hippo,
    state: Option<PathBuf>,
    now: Mutex<Kept>,
}

#[derive(Default)]
struct Kept {
    words: String,
    /// The row the words will be, from their first letter on.
    words_key: Option<String>,
    /// The row of the step in hand, told to the page before it is kept.
    next: Option<String>,
    /// Rows settled since the page last heard, each with hippo's row for
    /// it when hippo has it already.
    settled: Vec<(String, Option<u64>)>,
    /// Calls to the memory itself: their results stay out of it.
    recalls: HashSet<String>,
    /// Each call's row, by its id, so its result's row names it.
    calls: HashMap<String, String>,
    /// The harness says its answers whole; the turn's end needn't.
    answered: bool,
    // Structured immediately follows its result in the parsed harness frame;
    // keep both in one durable row before accepting the next step.
    pending_result: Option<(String, String, String)>,
    metrics: serde_json::Map<String, Value>,
    display: Option<Value>,
    failed: Option<String>,
}

/// The row hippo says it kept words as: "Kept as message 12." (`keep`),
/// "Posted as message 12." (`post`); "Already kept." names none.
fn row_of(out: &str) -> Option<u64> {
    out.trim()
        .strip_suffix('.')?
        .rsplit_once(" as message ")?
        .1
        .parse()
        .ok()
}

/// The row a result will be: its call's row and `.result`, so a page pairs
/// the two by key whatever was skipped between, as harnesses pair them by
/// call id. A result of no call seen this turn gets a row of its own.
fn result_key(calls: &HashMap<String, String>, id: &str) -> String {
    calls
        .get(id)
        .map_or_else(row_key, |call| format!("{call}{RESULT}"))
}

/// What a result's row key adds to its call's.
pub const RESULT: &str = ".result";

/// Whether a call reads the memory, so keeping its result would store the
/// store: a tool of the memory's server, whose name each harness puts in
/// the tool's (`mcp__hippo2__recall`, Codex's `hippo2.recall`), or its
/// command line. As hippo's watcher judges a transcript.
fn recalls(name: &str, input: &Value) -> bool {
    recalls_from(crate::servers::memory_name().as_deref(), name, input)
}

fn recalls_from(server: Option<&str>, name: &str, input: &Value) -> bool {
    if server.is_some_and(|s| name.contains(s)) {
        return true;
    }
    // OpenCode's code mode calls MCP tools from inside a script.
    if name.contains("memory_") || input.to_string().contains("memory_") {
        return true;
    }
    let command = input["command"].as_str().or(input["cmd"].as_str());
    command.is_some_and(|c| {
        let c = c.trim_start();
        c.starts_with("hippo ") || c.starts_with("hippo2 ")
    })
}

impl Keeper {
    pub fn new(hippo: Hippo, state: Option<PathBuf>) -> Self {
        Self {
            hippo,
            state,
            now: Mutex::default(),
        }
    }

    fn save(&self, now: &mut Kept, key: String, kind: &str, text: &str) {
        let display = (kind == "talk" && !now.metrics.is_empty())
            .then(|| json!({"version": 1, "metrics": now.metrics}));
        match self.hippo.save_display_as(&key, kind, text, display) {
            Ok(row) => now.settled.push((key, row)),
            Err(e) => {
                eprintln!("libri: {e}");
                now.failed.get_or_insert(e);
            }
        }
    }

    fn flush_result(&self, now: &mut Kept) {
        if let Some((_, key, text)) = now.pending_result.take() {
            match self
                .hippo
                .save_display_as(&key, "echo", &text, now.display.take())
            {
                Ok(row) => now.settled.push((key, row)),
                Err(e) => {
                    now.failed.get_or_insert(e);
                }
            }
        }
    }

    /// The words so far, kept as what led into the next thing.
    fn flush(&self, now: &mut Kept) {
        let words = std::mem::take(&mut now.words);
        let Some(key) = now.words_key.take() else {
            return;
        };
        let words = words.trim();
        if said(words) {
            self.save(now, key, "talk", words);
        } else {
            now.settled.push((key, None));
        }
    }

    /// An answer as the turn's words have it. The CLI's result holds only
    /// its last block, and the page showed all the words that end in it.
    fn answer(&self, now: &mut Kept, text: &str) {
        let words = std::mem::take(&mut now.words);
        let key = now.words_key.take().unwrap_or_else(row_key);
        let text = text.trim();
        if !said(text) {
            now.settled.push((key, None));
        } else if words.trim_end().ends_with(text) {
            self.save(now, key, "talk", words.trim());
        } else {
            self.save(now, key, "talk", text);
        }
    }

    /// The key of the row `step` will be, if it will be one, for the page
    /// to hear before the step is kept.
    pub fn row(&self, step: &Step) -> Option<String> {
        let mut now = self.now.lock().unwrap();
        let key = match step {
            Step::Text { text } if !text.is_empty() => {
                return Some(now.words_key.get_or_insert_with(row_key).clone());
            }
            Step::Result { id, text, .. } if now.recalls.contains(id) || text.trim().is_empty() => {
                return None;
            }
            Step::Call { .. } | Step::Done { .. } => row_key(),
            Step::Result { id, .. } => result_key(&now.calls, id),
            _ => return None,
        };
        now.next = Some(key.clone());
        Some(key)
    }

    /// The rows settled since this was last asked.
    pub fn settled(&self) -> Vec<(String, Option<u64>)> {
        std::mem::take(&mut self.now.lock().unwrap().settled)
    }

    /// Takes one step of the turn.
    pub fn step(&self, step: &Step) {
        let mut now = self.now.lock().unwrap();
        if !matches!(step, Step::Structured { .. }) {
            self.flush_result(&mut now);
        }
        match step {
            Step::Text { text } => now.words.push_str(text),
            Step::Call { id, name, input } => {
                self.flush(&mut now);
                if recalls(name, input) {
                    now.recalls.insert(id.clone());
                }
                let key = now.next.take().unwrap_or_else(row_key);
                now.calls.insert(id.clone(), key.clone());
                self.save(&mut now, key, "tool", &format!("{name} {input}"));
            }
            Step::Result { id, text, .. } => {
                if !now.recalls.remove(id) && !text.trim().is_empty() {
                    let key = match now.next.take() {
                        Some(key) => key,
                        None => result_key(&now.calls, id),
                    };
                    now.pending_result = Some((id.clone(), key, text.clone()));
                }
                now.calls.remove(id);
            }
            Step::Answer { text } => {
                now.answered = true;
                self.answer(&mut now, text);
            }
            Step::Agent { id, name, task } => {
                if let Some(state) = &self.state {
                    crate::task::inner_begun(state, id, name, task);
                }
            }
            Step::Inner { parent, step } => {
                if let Some(state) = &self.state {
                    crate::task::inner_step(state, parent, step);
                }
            }
            Step::Done { id, report } => {
                let kept = format!("[{id}]\n{}\n\nFull chat: hippo zoom {id}", report.trim());
                let key = now.next.take().unwrap_or_else(row_key);
                self.save(&mut now, key, "work", &kept);
            }
            Step::Structured { id, result } => {
                if now
                    .pending_result
                    .as_ref()
                    .is_some_and(|(call, _, _)| call == id)
                {
                    now.display = Some(json!({"version": 1, "result": result}));
                }
            }
            Step::Metrics {
                context_tokens,
                context_window,
                tokens_per_second,
            } => {
                for (key, value) in [
                    ("context_tokens", json!(context_tokens)),
                    ("context_window", json!(context_window)),
                    ("tokens_per_second", json!(tokens_per_second)),
                ] {
                    if !value.is_null() {
                        now.metrics.insert(key.into(), value);
                    }
                }
            }
            Step::Usage {
                input,
                cache_read,
                cache_write,
                output,
                model,
            } => {
                for (key, value) in [
                    ("input", json!(input)),
                    ("cache_read", json!(cache_read)),
                    ("cache_write", json!(cache_write)),
                    ("output", json!(output)),
                    ("model", json!(model)),
                ] {
                    if !value.is_null() {
                        now.metrics.insert(key.into(), value);
                    }
                }
            }
            Step::Thinking { .. }
            | Step::Progress { .. }
            | Step::Jobs { .. }
            | Step::Runs { .. }
            | Step::Row { .. }
            | Step::Stored { .. } => {}
        }
    }

    /// Ends the turn: the answer, unless it came whole already, and
    /// whether everything was kept.
    pub fn end(&self, reply: Option<&str>) -> Result<(), String> {
        let mut now = self.now.lock().unwrap();
        self.flush_result(&mut now);
        match reply {
            Some(reply) if !now.answered => self.answer(&mut now, reply),
            // The answers are kept; words after them were never one.
            Some(_) => {
                now.words.clear();
                if let Some(key) = now.words_key.take() {
                    now.settled.push((key, None));
                }
            }
            // Cut short: what it had said by then.
            None => self.flush(&mut now),
        }
        now.failed.take().map_or(Ok(()), Err)
    }
}

/// One message as hippo keeps it, for showing a conversation.
#[derive(Clone, Debug, Deserialize, serde::Serialize)]
pub struct Said {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<Value>,
    pub i: u64,
    pub kind: String,
    pub text: String,
    pub date: String,
    /// Where it came from, for a page matching it to a turn's record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub src: Option<Src>,
}

/// The key a row was kept under.
#[derive(Clone, Debug, Deserialize, serde::Serialize)]
pub struct Src {
    pub e: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::net::UnixListener;

    #[test]
    fn a_workers_memory_says_whether_it_is_a_fork() {
        let forked = work_answer("fork 80553\n<work>\n0+4096|user: a\n</work>").unwrap();
        let main = Fork {
            at: 80553,
            head: None,
        };
        assert_eq!(
            forked,
            ("<work>\n0+4096|user: a\n</work>".into(), Some(main))
        );
        let own = work_answer("fork none\n<work>\n0+1|note: subagent\n</work>").unwrap();
        assert!(own.1.is_none() && own.0.starts_with("<work>"));
        let sub = work_answer("fork 80700 ada-2026-11-02-0707 80553\n<work>\n</work>").unwrap();
        let head = Some(("ada-2026-11-02-0707".to_owned(), Some(80553)));
        assert_eq!(sub.1, Some(Fork { at: 80700, head }));
        let bare = work_answer("fork 12 ada-1 none\n<work>\n</work>").unwrap();
        assert_eq!(bare.1.unwrap().head, Some(("ada-1".to_owned(), None)));
        assert!(work_answer("<work>\n</work>").is_err());
    }

    #[test]
    fn all_the_words_ending_in_an_answer_are_kept_under_their_key() {
        let (hippo, dir) = spooled("words");
        let keeper = Keeper::new(hippo, None);
        let first = Step::Text {
            text: "Earlier. ".into(),
        };
        let key = keeper.row(&first).unwrap();
        keeper.step(&first);
        let second = Step::Text {
            text: "Done.".into(),
        };
        assert_eq!(keeper.row(&second).unwrap(), key);
        keeper.step(&second);
        keeper.step(&Step::Answer {
            text: "Done.".into(),
        });
        // Held in the spool, hippo being away: no row yet.
        assert_eq!(keeper.settled(), [(key.clone(), None)]);
        keeper.end(None).unwrap();
        let kept = keeper.hippo.tail(9).unwrap();
        assert_eq!(kept[0].text, "Earlier. Done.");
        assert_eq!(kept[0].src.as_ref().unwrap().e, key);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn quiet_words_are_settled_with_no_row() {
        let keeper = Keeper::new(
            Hippo {
                sock: "/nonexistent".into(),
                chat: None,
                spool: None,
            },
            None,
        );
        let words = Step::Text {
            text: crate::model::QUIET.into(),
        };
        let key = keeper.row(&words).unwrap();
        keeper.step(&words);
        keeper.step(&Step::Answer {
            text: crate::model::QUIET.into(),
        });
        assert_eq!(keeper.settled(), [(key, None)]);
    }

    /// The page learns an answer's row as hippo takes it, so what hangs on
    /// a row (quick replies, Reply) needs no reload.
    #[test]
    fn a_row_hippo_takes_now_is_settled_with_its_number() {
        let (hippo, dir) = spooled("row");
        let _heard = answering(&hippo.sock, r#"{"ok":true,"out":"Kept as message 41."}"#);
        let keeper = Keeper::new(hippo, None);
        let words = Step::Text {
            text: "Done.".into(),
        };
        let key = keeper.row(&words).unwrap();
        keeper.step(&words);
        keeper.step(&Step::Answer {
            text: "Done.".into(),
        });
        assert_eq!(keeper.settled(), [(key, Some(41))]);
        assert_eq!(row_of("Posted as message 7."), Some(7));
        assert_eq!(row_of("Already kept."), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn display_and_text_share_one_spooled_result() {
        let (hippo, dir) = spooled("display");
        let keeper = Keeper::new(hippo, None);
        for step in [
            Step::Call {
                id: "edit".into(),
                name: "edit".into(),
                input: json!({"path":"settings.rs"}),
            },
            Step::Result {
                id: "edit".into(),
                text: "Updated file".into(),
                error: false,
            },
            Step::Structured {
                id: "edit".into(),
                result: json!({"details":{"diff":"- off\n+ on"}}),
            },
        ] {
            keeper.row(&step);
            keeper.step(&step);
        }
        keeper.end(None).unwrap();
        let rows = keeper.hippo.tail(10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].text, "Updated file");
        assert_eq!(
            rows[1].display.as_ref().unwrap()["result"]["details"]["diff"],
            "- off\n+ on"
        );
        let view = keeper.hippo.view().unwrap();
        assert!(view.contains("Updated file"));
        assert!(!view.contains("- off"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_memory_calls_result_is_no_row() {
        let keeper = Keeper::new(
            Hippo {
                sock: "/nonexistent".into(),
                chat: None,
                spool: None,
            },
            None,
        );
        let call = Step::Call {
            id: "m".into(),
            name: "mcp__memory__recall".into(),
            input: Value::Null,
        };
        keeper.row(&call);
        keeper.step(&call);
        let result = Step::Result {
            id: "m".into(),
            text: "old words".into(),
            error: false,
        };
        assert_eq!(keeper.row(&result), None);
    }

    #[test]
    fn a_call_to_the_memorys_server_is_known_by_its_name_on_any_harness() {
        let none = Value::Null;
        for name in ["mcp__hippo2__recall", "hippo2.recall", "hippo2_open"] {
            assert!(recalls_from(Some("hippo2"), name, &none), "{name}");
        }
        assert!(!recalls_from(Some("hippo2"), "mcp__libri__lists", &none));
        let shell = json!({"command": "hippo2 recall pier"});
        assert!(recalls_from(None, "Bash", &shell));
    }

    #[test]
    fn a_result_is_kept_under_its_calls_key_whatever_came_between() {
        let (hippo, dir) = spooled("paired");
        let keeper = Keeper::new(hippo, None);
        let call = |id: &str, name: &str| Step::Call {
            id: id.into(),
            name: name.into(),
            input: Value::Null,
        };
        let result = |id: &str, text: &str| Step::Result {
            id: id.into(),
            text: text.into(),
            error: false,
        };
        // A result with no words is no row: the next still finds its call.
        let steps = [
            call("a", "ToolSearch"),
            call("b", "mcp__card__show"),
            result("a", ""),
            result("b", "Hello"),
        ];
        let keys: Vec<Option<String>> = steps
            .iter()
            .map(|s| {
                let key = keeper.row(s);
                keeper.step(s);
                key
            })
            .collect();
        let b = keys[1].clone().unwrap();
        assert_eq!(keys[2], None);
        assert_eq!(keys[3], Some(format!("{b}{RESULT}")));
        keeper.end(None).unwrap();
        let kept = keeper.hippo.tail(9).unwrap();
        let echo = kept.iter().find(|m| m.kind == "echo").unwrap();
        assert_eq!(echo.src.as_ref().unwrap().e, format!("{b}{RESULT}"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_hippo_that_never_answers_ends_the_wait() {
        let (hippo, _server) = silent_hippo();
        let began = std::time::Instant::now();
        let (view, saved) = (hippo.view(), hippo.save("user", "words"));
        assert_eq!(view, Err("hippo did not answer view in time".into()));
        assert_eq!(saved, Err("hippo did not answer log in time".into()));
        assert!(began.elapsed() < Duration::from_secs(10));
    }

    /// A hippo that accepts and never answers, as a frozen one does.
    fn silent_hippo() -> (Hippo, UnixListener) {
        let dir = std::env::temp_dir().join(format!(
            "libri-silent-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("hippo.sock");
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock).unwrap();
        (
            Hippo {
                sock,
                chat: None,
                spool: None,
            },
            listener,
        )
    }

    /// A hippo answering one request; what it was asked.
    fn hippo(chat: Option<&str>, reply: &str) -> (Hippo, std::thread::JoinHandle<String>) {
        let dir = std::env::temp_dir().join(format!(
            "libri-hippo-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("hippo.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let reply = format!("{reply}\n");
        let server = std::thread::spawn(move || {
            let (mut c, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(c.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            c.write_all(reply.as_bytes()).unwrap();
            line.trim().to_owned()
        });
        let h = Hippo {
            sock,
            chat: chat.map(str::to_owned),
            spool: None,
        };
        (h, server)
    }

    const OK: &str = r#"{"ok":true,"out":"Posted."}"#;

    #[test]
    fn a_seat_saves_into_its_own_chat() {
        let (h, server) = hippo(Some("Iris"), OK);
        h.save("user", "hello").unwrap();
        assert_eq!(
            server.join().unwrap(),
            r#"{"args":["Iris","user","hello"],"cmd":"post"}"#
        );
    }

    #[test]
    fn off_a_seat_it_logs() {
        let (h, server) = hippo(None, OK);
        h.save("talk", "hello").unwrap();
        assert_eq!(
            server.join().unwrap(),
            r#"{"args":["talk","hello"],"cmd":"log"}"#
        );
    }

    #[test]
    fn a_refused_save_is_an_error() {
        let (h, server) = hippo(Some("Iris"), r#"{"ok":false,"out":"No."}"#);
        assert_eq!(h.save("user", "hello").unwrap_err(), "hippo: No.");
        server.join().unwrap();
    }

    #[test]
    fn an_unreachable_hippo_is_an_error() {
        let h = Hippo {
            sock: "/nonexistent/hippo.sock".into(),
            chat: Some("Iris".into()),
            spool: None,
        };
        assert!(h.save("user", "hello").is_err());
    }

    fn spooled(name: &str) -> (Hippo, PathBuf) {
        let dir = std::env::temp_dir().join(format!("libri-spool-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let spool = Spool::new(dir.join("spool")).unwrap();
        let h = Hippo {
            sock: dir.join("hippo.sock"),
            chat: Some("Iris".into()),
            spool: Some(std::sync::Arc::new(spool)),
        };
        (h, dir)
    }

    /// Answers each request with `reply`, keeping the requests.
    fn answering(sock: &std::path::Path, reply: &'static str) -> std::sync::mpsc::Receiver<String> {
        let listener = UnixListener::bind(sock).unwrap();
        let (told, heard) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for mut c in listener.incoming().flatten() {
                let mut line = String::new();
                BufReader::new(c.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let _ = told.send(line);
                let _ = c.write_all(format!("{reply}\n").as_bytes());
            }
        });
        heard
    }

    #[test]
    fn words_hippo_cannot_take_wait_and_go_in_once_in_order() {
        let (h, dir) = spooled("order");
        h.save("user", "first").unwrap();
        h.save("talk", "second").unwrap();
        assert!(h.holding());
        let heard = answering(&h.sock, OK);
        h.save("user", "third").unwrap();
        let said: Vec<Value> = (0..3)
            .map(|_| serde_json::from_str(&heard.recv().unwrap()).unwrap())
            .collect();
        let words: Vec<&str> = said
            .iter()
            .map(|s| s["args"][4].as_str().unwrap())
            .collect();
        assert_eq!(words, ["first", "second", "third"]);
        assert!(said.iter().all(|s| s["cmd"] == "keep"));
        assert!(!h.holding());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_view_outlives_hippo_with_the_words_since() {
        let (h, dir) = spooled("view");
        let _heard = answering(
            &h.sock,
            r#"{"ok":true,"out":"<chat>\n0+1|the past\n</chat>"}"#,
        );
        assert!(h.view().unwrap().contains("the past"));
        std::fs::remove_file(&h.sock).unwrap();
        h.save("user", "said meanwhile").unwrap();
        let view = h.view().unwrap();
        assert!(
            view.contains("the past") && view.contains("user: said meanwhile"),
            "{view}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_history_outlives_hippo_with_the_words_since() {
        let (h, dir) = spooled("tail");
        let line = r#"{"i":7,"kind":"user","text":"the past","date":"2026-10-30T22:00:00-04:00"}"#;
        let reply: &'static str = Box::leak(
            json!({"ok": true, "out": line})
                .to_string()
                .into_boxed_str(),
        );
        let _heard = answering(&h.sock, reply);
        assert_eq!(h.tail(5).unwrap().len(), 1);
        std::fs::remove_file(&h.sock).unwrap();
        h.save("user", "said meanwhile").unwrap();
        let all = h.tail(5).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!((all[0].i, all[1].i), (7, 8));
        assert_eq!(all[1].text, "said meanwhile");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_person_hears_once_that_the_memory_went_and_once_that_it_came_back() {
        let (h, dir) = spooled("notice");
        assert_eq!(h.notice(), None);
        h.save("user", "words").unwrap();
        assert!(h.notice().unwrap().contains("down"));
        h.save("user", "more").unwrap();
        assert_eq!(h.notice(), None);
        let _heard = answering(&h.sock, OK);
        h.replay();
        assert!(h.notice().unwrap().contains("back"));
        assert_eq!(h.notice(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn words_on_no_disk_at_all_are_said_to_be_lost() {
        let (h, dir) = spooled("nodisk");
        std::fs::remove_dir_all(dir.join("spool")).unwrap();
        assert!(h.save("user", "words").is_err());
        assert!(h.notice().unwrap().contains("couldn't keep"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_hung_hippo_costs_a_wait_not_the_turn() {
        let (h, dir) = spooled("hung");
        // A hippo that reads each request and never answers, until a last
        // connection says the end: connections are taken in the order they
        // came, so it has every one of the client's.
        let listening = UnixListener::bind(&h.sock).unwrap();
        let reading = std::thread::spawn(move || {
            let (mut asked, mut held) = (Vec::new(), Vec::new());
            for conn in listening.incoming() {
                let conn = conn.unwrap();
                let mut line = String::new();
                BufReader::new(&conn).read_line(&mut line).unwrap();
                if line == "end\n" {
                    return asked;
                }
                let req: Value = serde_json::from_str(&line).unwrap();
                asked.push(req["cmd"].as_str().unwrap().to_owned());
                held.push(conn);
            }
            asked
        });
        let at = std::time::Instant::now();
        assert!(h.view().is_ok());
        h.save("user", "words").unwrap();
        // A ceiling far over the two waits, for a client that waits on
        // hippo without end: no builder is that slow.
        assert!(at.elapsed() < Duration::from_secs(30), "{:?}", at.elapsed());
        assert!(h.holding());
        let mut end = UnixStream::connect(&h.sock).unwrap();
        end.write_all(b"end\n").unwrap();
        // One wait for the view and one for the words, then the spool: no
        // asking again. Counted, not timed: in a test the waits are short, and
        // what a loaded builder adds is disk and scheduling, not waits.
        assert_eq!(reading.join().unwrap(), ["view", "keep"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
