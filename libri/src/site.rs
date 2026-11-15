//! The web door: a chat page for the person, served on `LIBRI_SITE`. The
//! page is static files built from `site/` (React on assistant-ui); this
//! serves them and the API under `/api`. Replies reach open pages over SSE;
//! history comes from hippo.

use crate::agui;
use crate::door::{Door, Fetched, Message};
use crate::eventlog::{EventLog, Logged, Missed};
use crate::files::{Library, mime_of};
use crate::hippo::{Hippo, Said};
use crate::model::{Ask as SeatAsk, Asker, Attachment, Step};
use crate::pending::{self, Pending};
use crate::scratchpad;
use crate::{Read, seen};
use crate::{keep, task};
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::convert::Infallible;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::{SetResponseHeader, SetResponseHeaderLayer};
use tower_layer::Layer;

use crate::push::{self, Push};

mod connectors;
mod outside;
/// Where other apps' reach is kept, for the settings list.
pub use outside::relay as mcp_relay;

const COOKIE: &str = "session";

/// How long a paired device's cookie lasts: the most browsers keep one
/// (400 days). The pairing lasts until revoked; a page whose cookie ran
/// out pairs again.
const PAIRED: Duration = Duration::from_secs(400 * 24 * 3600);
/// How long a request to sign in waits for its answer.
const ASK: Duration = Duration::from_secs(5 * 60);
/// Messages of history the page loads: a seat's tool calls and results
/// count, so enough for a few dozen turns.
const HISTORY: u64 = 400;
/// The most of a tool call or result the history carries, in bytes; the
/// rest opens from hippo.
const HISTORY_STEP: usize = 4 * 1024;
/// How long a reply read aloud stays cached.
const SPEECH_KEEP: Duration = Duration::from_secs(30 * 24 * 3600);
const SEND_MAX: usize = 32 * 1024;
/// Readings under way at once.
const SPEAKING_MAX: usize = 4;
/// The most files one message carries.
const FILES_MAX: usize = 10;
/// The largest request to /api/send, all its files together: what the
/// Cloudflare tunnel passes. Files stay whole to be kept and converted;
/// the model sees only what fits its request (`answer`).
const UPLOAD_MAX: usize = 100_000_000;
/// How often a quiet event stream says it is alive. Three missed, about
/// three seconds, and a client may tell the person its link is stale
/// rather than show a still screen as live, as a remote terminal does.
const PULSE: Duration = Duration::from_secs(1);
/// About an hour of a browser's speech recording.
const DICTATION_MAX: usize = 64_000_000;

/// What the door shows, in its own terms; the pages get it as AG-UI
/// events (agui.rs), push notifications as it is.
#[derive(Clone)]
enum Shown {
    /// A message from Libri; `plain` is shown word for word. One
    /// `aside` came from no turn (a reminder, a message's answer), so it
    /// stands apart from any answer being worked out.
    Said {
        id: String,
        text: String,
        plain: bool,
        aside: bool,
    },
    /// What the person wrote, as it comes in: by another door, or by a
    /// page, which every other page open shows too.
    Heard {
        /// The message's id: the other door's, so a reaction to it lands
        /// here too, or the one the sending page gave it.
        id: String,
        text: String,
    },
    /// The person's words, in or out of the queue for the next turn.
    Waiting {
        id: String,
        on: bool,
    },
    /// An act waiting for the person's word.
    Pending {
        pending: Pending,
    },
    /// The verdict on act `id` is carried out; when not `done` the act
    /// failed and still waits.
    Settled {
        id: u64,
        text: String,
        done: bool,
        scratchpad: bool,
    },
    React {
        to: String,
        emoji: String,
    },
    /// A picture kept as `name`, fetched from `/api/file/{name}`.
    Picture {
        name: String,
    },
    /// A kept file sent back, fetched from `/api/file/{name}`.
    File {
        name: String,
    },
    Typing {
        on: bool,
    },
    /// A tool call or its result, while the answer is worked out; `agent`
    /// names the subagent whose work it is, when it isn't the turn's.
    Step {
        step: Step,
        agent: Option<String>,
    },
    /// The subagents at work, by name, with their tasks.
    Agents {
        agents: Vec<Agent>,
    },
    /// The seat asks the person: may a tool run, or its questions.
    Ask {
        ask: SeatAsk,
    },
    /// Ask `id` is answered or taken back.
    Asked {
        id: String,
    },
    /// The oldest messages were forgotten: the history is read again.
    Forgot,
    /// Rows joined the history from outside a turn: pages read it again.
    History,
    /// The scratchpad changed: a page reads it again. Its steps come as a
    /// subagent's, by its id; none of this goes in the turn record, which
    /// is the main chat's.
    Scratchpad,
    /// A named memory's chat changed: pages showing it read it again. Its
    /// steps come as a subagent's, by its id, as the scratchpad's do.
    Memory {
        id: String,
    },
}

/// A subagent at work, as a page shows it. One the seat's Agent call
/// started goes by the call's id, `title` its description; the page can't
/// stop it, as nothing outside the CLI can.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Agent {
    name: String,
    task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
}

impl Agent {
    fn own((name, task): (String, String)) -> Self {
        Self {
            name,
            task,
            title: None,
        }
    }
}

/// Where the person's answer to an ask goes: the input to run with, or
/// None to refuse.
type Waiter = std::sync::mpsc::Sender<Option<Value>>;

/// The open pages, as a door.
#[derive(Clone)]
pub struct Web {
    shown: broadcast::Sender<Shown>,
    /// The same as AG-UI events, each one JSON text, numbered and kept a
    /// while so a client that drops off gets back only what it missed.
    wire: Arc<EventLog>,
    encoder: Arc<Mutex<agui::Encoder>>,
    ids: Arc<AtomicU64>,
    aside: bool,
    /// The agents as last shown, so a tick that changes nothing is silent.
    agents: Arc<Mutex<Vec<Agent>>>,
    /// Libri's own agents as the tick last gave them, and the seat's
    /// subagents at work, both shown as one list.
    own: Arc<Mutex<Vec<Agent>>>,
    seated: Arc<Mutex<Vec<Agent>>>,
    /// Whether a turn is under way, for a page that missed the news.
    typing: Arc<AtomicBool>,
    /// The seat's asks waiting on the person, each with its waiter.
    asks: Arc<Mutex<Vec<(SeatAsk, Waiter)>>>,
    /// What the turn under way showed and hippo lacks, for a page that
    /// loads now.
    turn: Arc<Mutex<Turn>>,
    /// Where the turn record is kept, so a restart (a deploy, a crash)
    /// doesn't take what hippo lacks off the page.
    turn_file: Arc<std::sync::OnceLock<PathBuf>>,
    /// The row the step on its way becomes.
    row: Arc<Mutex<Option<String>>>,
    /// What each scratchpad's run is at, by its ID, for every client alike.
    scratchpad_runs: Arc<Mutex<HashMap<String, Arc<Mutex<ScratchpadRun>>>>>,
    /// What each named memory's chat run is at, by the chat's ID.
    memory_runs: Arc<Mutex<HashMap<String, Arc<Mutex<ScratchpadRun>>>>>,
}

/// What the scratchpad's run is at and its log lacks: its words since its
/// last call, its thinking, which no log keeps, and whether it was told to
/// stop. A client reads this with the log, so one that comes in during the
/// run stands where the others do.
#[derive(Default)]
pub struct ScratchpadRun {
    pub words: String,
    pub thinking: String,
    pub stopping: bool,
}

/// What hippo lacks of the talk: a page shows hippo's rows and then this,
/// as Unii's clients show the log and then `Chat.live`. An event that will
/// be a row carries that row's `key` and goes once hippo has it; every event
/// says what row it came `after`, which is how thinking and a memory call's
/// result, which hippo never has, keep their place among the rows.
#[derive(Default, Clone, Serialize, Deserialize)]
struct Turn {
    /// When its first message came, in Unix milliseconds.
    start: u64,
    /// Its AG-UI events.
    shown: Vec<Value>,
    /// The row settled last.
    #[serde(default)]
    after: String,
    /// The number of the newest event, which every event carries as `seq`:
    /// what a page hears after reading this comes on top of it.
    #[serde(default)]
    seq: u64,
    #[serde(skip)]
    ended: bool,
}

impl Turn {
    /// Lets go of what row `key` holds. A call stays until its result does:
    /// a page has to know its id for the result, and hippo's row doesn't say.
    fn stored(&mut self, key: &str, heard: &[String]) {
        self.after = key.to_owned();
        let of = |e: &&Value| e["key"] == key;
        let opens = self
            .shown
            .iter()
            .filter(of)
            .any(|e| e["type"] == "TOOL_CALL_START");
        let call = self
            .shown
            .iter()
            .filter(of)
            .find(|e| e["type"] == "TOOL_CALL_RESULT")
            .map(|e| e["toolCallId"].clone());
        // Messages waiting are folded into one turn in order, under the last.
        let upto = self.shown.iter().rposition(|e| {
            e["role"] == "user" && heard.iter().any(|id| e["messageId"] == id.as_str())
        });
        let mut n = 0;
        self.shown.retain(|e| {
            let theirs = e["role"] == "user" && upto.is_some_and(|last| n <= last);
            n += 1;
            let settled = e["key"] == key && !opens;
            !(theirs || settled || call.as_ref().is_some_and(|c| e["toolCallId"] == *c))
        });
    }

    /// What outlives the turn's end: words not yet a row, and the person's
    /// words hippo has yet to get, which wait for their own turn.
    fn close(&mut self) {
        self.ended = true;
        self.shown.retain(|e| {
            let text = e["type"]
                .as_str()
                .is_some_and(|k| k.starts_with("TEXT_MESSAGE"));
            text && (e["key"].is_string() || e["role"] == "user")
        });
    }

    /// The next turn's record. The person's words a turn left wait through
    /// one more: words that never become a row (a command) go then.
    fn next(&mut self) -> Turn {
        let mut shown = std::mem::take(&mut self.shown);
        shown.retain(|e| e["role"] != "user" || e["waited"].is_null());
        for e in shown.iter_mut().filter(|e| e["role"] == "user") {
            e["waited"] = true.into();
        }
        Turn {
            start: unix_ms(),
            shown,
            after: std::mem::take(&mut self.after),
            seq: self.seq,
            ended: false,
        }
    }
}

/// The most a turn record keeps; a page loading later sees hippo's.
const TURN_KEPT: usize = 400;

/// Writes the turn record whole or not at all. A failed write costs only
/// what a restart would show, so it is logged and the turn goes on.
fn save_turn(file: &Path, t: &Turn) {
    let tmp = file.with_extension("tmp");
    let done = serde_json::to_vec(t)
        .map_err(|e| e.to_string())
        .and_then(|b| fs::write(&tmp, b).map_err(|e| e.to_string()))
        .and_then(|()| fs::rename(&tmp, file).map_err(|e| e.to_string()));
    if let Err(e) = done {
        eprintln!("libri: turn record {}: {e}", file.display());
    }
}

impl Web {
    pub fn new() -> Self {
        Self {
            shown: broadcast::channel(256).0,
            wire: Arc::new(EventLog::new()),
            encoder: Arc::default(),
            ids: Arc::new(AtomicU64::new(0)),
            aside: false,
            agents: Arc::default(),
            own: Arc::default(),
            seated: Arc::default(),
            typing: Arc::default(),
            asks: Arc::default(),
            scratchpad_runs: Arc::default(),
            memory_runs: Arc::default(),
            turn: Arc::default(),
            turn_file: Arc::default(),
            row: Arc::default(),
        }
    }

    /// Keeps the turn record in `file`, taking up the one a restart left.
    /// That turn died with the restart, so it counts as ended: the next
    /// thing shown begins a new one.
    fn keep_turn_in(&self, file: PathBuf) {
        if let Ok(text) = fs::read_to_string(&file)
            && let Ok(mut t) = serde_json::from_str::<Turn>(&text)
        {
            // A record from before keys holds nothing hippo lacks by name.
            if t.seq == 0 {
                t.shown.clear();
            }
            t.ended = true;
            *self.turn.lock().unwrap() = t;
        }
        let _ = self.turn_file.set(file);
    }

    /// Keeps the words a restart cut off in mid-turn: nothing else will,
    /// and hippo's key makes doing it twice harmless.
    pub fn keep_cut_words(&self, hippo: &Hippo) {
        let mut t = self.turn.lock().unwrap();
        let mut cut: Vec<(String, String)> = Vec::new();
        for e in t
            .shown
            .iter()
            .filter(|e| e["type"] == "TEXT_MESSAGE_CONTENT")
        {
            let (Some(key), Some(delta)) = (e["key"].as_str(), e["delta"].as_str()) else {
                continue;
            };
            match cut.iter_mut().find(|(k, _)| k == key) {
                Some((_, words)) => words.push_str(delta),
                None => cut.push((key.to_owned(), delta.to_owned())),
            }
        }
        for (key, words) in cut {
            // Words of a turn left behind, kept after it: no page is on
            // them live to name them by a row.
            match hippo.save_as(&key, "talk", &words) {
                Ok(_) => t.stored(&key, &[]),
                Err(e) => eprintln!("libri: {e}"),
            }
        }
    }

    /// The same pages, for what comes from no turn.
    pub fn aside(&self) -> Self {
        Self {
            aside: true,
            ..self.clone()
        }
    }

    /// Tells the pages the oldest messages went.
    pub fn forgot(&self) {
        self.show(Shown::Forgot);
    }

    /// Marks the person's message `id` as waiting its turn, or no longer.
    pub fn waiting(&self, id: &str, on: bool) {
        self.show(Shown::Waiting { id: id.into(), on });
    }

    /// Shows the agents at work, if that changed.
    fn at_work(&self) {
        let now: Vec<Agent> = [
            self.own.lock().unwrap().clone(),
            self.seated.lock().unwrap().clone(),
        ]
        .concat();
        let mut last = self.agents.lock().unwrap();
        if *last != now {
            *last = now.clone();
            self.show(Shown::Agents { agents: now });
        }
    }

    fn id(&self) -> String {
        format!("web-{}", self.ids.fetch_add(1, Ordering::Relaxed))
    }

    /// Settles ask `id` with `input` (None refuses it); false when it is
    /// gone already.
    fn resolve(&self, id: &str, input: Option<Value>) -> bool {
        let found = {
            let mut asks = self.asks.lock().unwrap();
            let i = asks.iter().position(|(a, _)| a.id == id);
            i.map(|i| asks.remove(i))
        };
        let Some((_, waiter)) = found else {
            return false;
        };
        let _ = waiter.send(input);
        if id.starts_with(SCRATCHPAD_ASK) {
            self.scratchpad_changed();
        } else if let Some((thread, _)) =
            id.strip_prefix(MEMORY_ASK).and_then(|r| r.split_once(':'))
        {
            self.memory_changed(thread);
        } else {
            self.show(Shown::Asked { id: id.into() });
        }
        true
    }

    /// Words typed while asks wait answer them, as a chat's reply does: a
    /// question takes them as the answer to each of its questions, and a
    /// tool waiting for a yes is refused, as Claude Code's "no, and tell it
    /// what to do differently": the words then go on to the turn as any
    /// sent mid-turn. They are never read as a yes, which only a press
    /// gives. True when they answered a question, and so are not news.
    fn typed(&self, text: &str) -> bool {
        let open: Vec<(String, Option<Value>)> = self
            .asks
            .lock()
            .unwrap()
            .iter()
            .filter(|(a, _)| !a.id.starts_with(SCRATCHPAD_ASK) && !a.id.starts_with(MEMORY_ASK))
            .map(|(a, _)| (a.id.clone(), typed_answer(a, text)))
            .collect();
        let mut answered = false;
        for (id, input) in open {
            answered |= input.is_some();
            self.resolve(&id, input);
        }
        answered
    }

    /// Tells the pages to read named memory chat `id` again.
    pub fn memory_changed(&self, id: &str) {
        self.show(Shown::Memory { id: id.to_owned() });
    }

    /// What named memory chat `id`'s run is at, as `scratchpad_run`.
    pub fn memory_run(&self, id: &str) -> Arc<Mutex<ScratchpadRun>> {
        self.memory_runs
            .lock()
            .unwrap()
            .entry(id.to_owned())
            .or_default()
            .clone()
    }

    /// Tells the pages to read the scratchpad again.
    pub fn scratchpad_changed(&self) {
        self.show(Shown::Scratchpad);
    }

    /// What scratchpad `id`'s run is at.
    pub fn scratchpad_run(&self, id: &str) -> Arc<Mutex<ScratchpadRun>> {
        self.scratchpad_runs
            .lock()
            .unwrap()
            .entry(id.to_owned())
            .or_default()
            .clone()
    }

    /// Forgets a closed scratchpad's run.
    pub fn scratchpad_gone(&self, id: &str) {
        self.scratchpad_runs.lock().unwrap().remove(id);
    }

    /// What scratchpad `id`'s run asks through: its asks wait here with the
    /// main chat's and are answered the same way, but show on its own tile.
    pub fn scratchpad_asker(&self, id: &str) -> ColumnAsker {
        ColumnAsker {
            web: self.clone(),
            prefix: format!("{SCRATCHPAD_ASK}{id}:"),
            thread: None,
        }
    }

    /// The same for named memory chat `thread`, whose asks show on its tile.
    pub fn memory_asker(&self, thread: &str) -> ColumnAsker {
        ColumnAsker {
            web: self.clone(),
            prefix: format!("{MEMORY_ASK}{thread}:"),
            thread: Some(thread.into()),
        }
    }

    /// The asks waiting on the person whose ids start with `prefix`.
    fn asks_under(&self, prefix: &str) -> Vec<Value> {
        self.asks
            .lock()
            .unwrap()
            .iter()
            .filter(|(a, _)| a.id.starts_with(prefix))
            .map(|(a, _)| agui::interrupt(a))
            .collect()
    }

    /// Scratchpad `id`'s asks waiting on the person.
    fn scratchpad_asks(&self, id: &str) -> Vec<Value> {
        self.asks_under(&format!("{SCRATCHPAD_ASK}{id}:"))
    }

    fn show(&self, s: Shown) {
        let subs: Vec<agui::Sub>;
        let input = match &s {
            Shown::Said {
                id,
                text,
                plain,
                aside,
            } => agui::In::Said {
                id,
                text,
                plain: *plain,
                aside: *aside,
            },
            Shown::Heard { id, text } => agui::In::Heard { id, text },
            Shown::Pending { pending } => agui::In::Pending(pending),
            Shown::Settled {
                id,
                text,
                done,
                scratchpad,
            } => agui::In::Settled {
                id: *id,
                text,
                done: *done,
                scratchpad: *scratchpad,
            },
            Shown::React { to, emoji } => agui::In::Custom {
                name: "react",
                value: serde_json::json!({"to": to, "emoji": emoji}),
            },
            Shown::Picture { name } => agui::In::Custom {
                name: "picture",
                value: serde_json::json!({ "name": name }),
            },
            Shown::File { name } => agui::In::Custom {
                name: "file",
                value: serde_json::json!({ "name": name }),
            },
            Shown::Typing { on } => agui::In::Working(*on),
            Shown::Step { step, agent } => agui::In::Step {
                step,
                agent: agent.as_deref(),
            },
            Shown::Agents { agents } => {
                subs = agents
                    .iter()
                    .map(|a| agui::Sub {
                        id: a.name.clone(),
                        task: a.task.clone(),
                        title: a.title.clone(),
                    })
                    .collect();
                agui::In::Agents(&subs)
            }
            Shown::Ask { ask } => agui::In::Ask(ask),
            Shown::Asked { id } => agui::In::Asked(id),
            Shown::Forgot => agui::In::Custom {
                name: "forgot",
                value: Value::Null,
            },
            Shown::History => agui::In::Custom {
                name: "history",
                value: Value::Null,
            },
            Shown::Scratchpad => agui::In::Custom {
                name: "scratchpad",
                value: Value::Null,
            },
            Shown::Memory { id } => agui::In::Custom {
                name: "memory-chat",
                value: serde_json::json!({ "id": id }),
            },
            Shown::Waiting { id, on } => agui::In::Custom {
                name: "queued",
                value: serde_json::json!({"id": id, "on": on}),
            },
        };
        // One lock for encoding and sending, so every page gets the events
        // in the order they were made.
        let mut encoder = self.encoder.lock().unwrap();
        let mut events = encoder.encode(input);
        // AG-UI's own timestamp, so a page can say how long a call took,
        // live and from the turn's record alike.
        let at = unix_ms();
        for e in &mut events {
            e["timestamp"] = at.into();
        }
        let kept = matches!(
            s,
            Shown::Heard { .. }
                | Shown::Step { agent: None, .. }
                | Shown::Typing { .. }
                | Shown::Ask { .. }
                | Shown::Asked { .. }
        );
        if kept {
            self.keep(&s, &mut events);
        }
        for e in events {
            self.wire.publish(e.to_string());
        }
        drop(encoder);
        let _ = self.shown.send(s);
    }

    /// Adds what belongs to the turn to its record, numbering it and saying
    /// what row it is and comes after; the first thing after a turn's end
    /// begins the next. What is said goes unrecorded: hippo has it first.
    fn keep(&self, s: &Shown, events: &mut [Value]) {
        let row = match s {
            Shown::Step { .. } => self.row.lock().unwrap().take(),
            _ => None,
        };
        let mut t = self.turn.lock().unwrap();
        if t.ended || t.start == 0 {
            // A turn no one's words began (a routine, a report) begins
            // at its start, before its prompt reaches hippo, so a page
            // loading later knows hippo's prompt and calls for this turn's
            // own. Only the start of a turn may begin one.
            if matches!(s, Shown::Typing { on: false } | Shown::Asked { .. }) {
                return;
            }
            *t = t.next();
        }
        for e in events.iter_mut() {
            t.seq += 1;
            e["seq"] = t.seq.into();
            e["after"] = t.after.clone().into();
            let kind = e["type"].as_str().unwrap_or_default().to_owned();
            if let Some(key) = &row
                && (kind.starts_with("TEXT_MESSAGE")
                    || kind.starts_with("TOOL_CALL")
                    || kind == "SUBAGENT_FINISHED")
            {
                e["key"] = key.clone().into();
            }
            // The words stream in a few letters at a time: they join the
            // words before them, so a long answer doesn't use up the record.
            if kind.ends_with("_MESSAGE_CONTENT")
                && let Some(before) = t.shown.last_mut()
                && before["type"] == kind
                && before["messageId"] == e["messageId"]
                && let (Some(a), Some(b)) = (before["delta"].as_str(), e["delta"].as_str())
            {
                before["delta"] = format!("{a}{b}").into();
                continue;
            }
            // Only the latest count of background jobs is true.
            if kind == "ACTIVITY_SNAPSHOT" && e["activityType"] == "jobs" {
                t.shown.retain(|x| x["activityType"] != "jobs");
            }
            // A call's output so far comes whole each time: the latest
            // stands for the rest, and its result for all of them.
            if kind == "ACTIVITY_SNAPSHOT" && e["activityType"] == "tool_output" {
                t.shown.retain(|x| x["messageId"] != e["messageId"]);
            }
            if kind == "TOOL_CALL_RESULT" {
                let output = format!("output-{}", e["toolCallId"].as_str().unwrap_or_default());
                t.shown.retain(|x| x["messageId"] != output.as_str());
            }
            // Everyone's words are kept past the cap: a long turn's calls
            // may fill it, and hippo may never get what the model said
            // between them.
            if t.shown.len() < TURN_KEPT || kind.starts_with("TEXT_MESSAGE") {
                t.shown.push(e.clone());
            }
        }
        if let Shown::Typing { on: false } = s {
            t.close();
        }
        // Saved at each step, not at each few letters of streamed words or
        // arguments: the step that ends them saves them whole.
        let streamed = |e: &Value| {
            let kind = e["type"].as_str().unwrap_or_default();
            kind.ends_with("_CONTENT")
                || kind.ends_with("_ARGS")
                || e["activityType"] == "tool_output"
        };
        if let Some(file) = self.turn_file.get()
            && !events.iter().all(streamed)
        {
            save_turn(file, &t);
        }
    }

    /// Row `key` is settled: the record lets go of what it holds, and
    /// pages hear the row hippo gave it, so a message they drew live is
    /// named by its row (a reply's quote, quick replies) as after a reload.
    pub fn stored(&self, key: &str, heard: &[String], row: Option<u64>) {
        let mut t = self.turn.lock().unwrap();
        t.stored(key, heard);
        if let Some(file) = self.turn_file.get() {
            save_turn(file, &t);
        }
        drop(t);
        if let Some(row) = row {
            let input = agui::In::Custom {
                name: "row",
                value: serde_json::json!({"key": key, "row": row, "heard": heard}),
            };
            for e in self.encoder.lock().unwrap().encode(input) {
                self.wire.publish(e.to_string());
            }
        }
        // A page reading the task, then the history, around this moment would
        // hold their words twice, and only a new read knows it.
        if !heard.is_empty() {
            let input = agui::In::Custom {
                name: "stored",
                value: Value::Null,
            };
            for e in self.encoder.lock().unwrap().encode(input) {
                self.wire.publish(e.to_string());
            }
        }
    }

    /// The person's words from a page, to every page open, under the id
    /// the page gave them. A redo goes to the record alone; the other pages
    /// see its answer, and the history, on their next load.
    pub fn sent(&self, id: &str, text: &str, redo: bool) {
        let heard = Shown::Heard {
            id: id.into(),
            text: text.into(),
        };
        // hippo has a redone message already.
        if !redo {
            self.show(heard);
        }
    }
}

impl Asker for Web {
    fn put(&self, ask: SeatAsk) -> Option<Value> {
        let (waiter, answer) = std::sync::mpsc::channel();
        self.asks.lock().unwrap().push((ask.clone(), waiter));
        self.show(Shown::Ask { ask });
        // A waiter dropped unanswered refuses.
        answer.recv().ok().flatten()
    }

    fn withdraw(&self, id: &str) {
        self.resolve(id, None);
    }
}

/// Marks an ask's id as a scratchpad's.
const SCRATCHPAD_ASK: &str = "scratchpad:";
/// Marks an ask's id as a named memory chat's: `memory:<thread>:<id>`.
const MEMORY_ASK: &str = "memory:";

/// Asks of a chat shown in a column of its own (Temp, a named memory's):
/// they wait with the main chat's and are answered the same way, but the
/// column reads them, not the main thread.
pub struct ColumnAsker {
    web: Web,
    prefix: String,
    /// The memory chat to re-read; None for the scratchpad.
    thread: Option<String>,
}

impl ColumnAsker {
    fn changed(&self) {
        match &self.thread {
            Some(t) => self.web.memory_changed(t),
            None => self.web.scratchpad_changed(),
        }
    }
}

impl Asker for ColumnAsker {
    fn put(&self, mut ask: SeatAsk) -> Option<Value> {
        ask.id = format!("{}{}", self.prefix, ask.id);
        let (waiter, answer) = std::sync::mpsc::channel();
        self.web.asks.lock().unwrap().push((ask, waiter));
        self.changed();
        // A waiter dropped unanswered refuses.
        answer.recv().ok().flatten()
    }

    fn withdraw(&self, id: &str) {
        self.web.resolve(&format!("{}{id}", self.prefix), None);
    }
}

impl Door for Web {
    type File = Upload;

    fn asker(&self) -> Option<Arc<dyn Asker>> {
        Some(Arc::new(self.clone()))
    }

    async fn say(&self, text: &str) -> Result<String, String> {
        let id = self.id();
        self.show(Shown::Said {
            id: id.clone(),
            text: text.into(),
            plain: false,
            aside: self.aside,
        });
        Ok(id)
    }

    async fn quote(&self, text: &str) -> Result<String, String> {
        let id = self.id();
        self.show(Shown::Said {
            id: id.clone(),
            text: text.into(),
            plain: true,
            aside: self.aside,
        });
        Ok(id)
    }

    async fn react(&self, to: &str, emoji: &str) -> Result<(), String> {
        self.show(Shown::React {
            to: to.into(),
            emoji: emoji.into(),
        });
        Ok(())
    }

    async fn picture(&self, name: &str, _data: Vec<u8>, _mime: &'static str) -> Result<(), String> {
        self.show(Shown::Picture { name: name.into() });
        Ok(())
    }

    fn step(&self, step: Step) {
        match step {
            Step::Row { key } => *self.row.lock().unwrap() = Some(key),
            Step::Stored { key, heard, row } => self.stored(&key, &heard, row),
            Step::Inner { parent, step } => {
                // Claude Code's subagents end on a SubagentHandback call,
                // whose message is the report; the notice that follows only
                // points to it.
                if let Step::Call { name, input, .. } = step.as_ref()
                    && name == "SubagentHandback"
                    && let Some(report) = input["message"].as_str()
                {
                    self.show(Shown::Step {
                        step: Step::Done {
                            id: parent.clone(),
                            report: report.to_owned(),
                        },
                        agent: None,
                    });
                }
                self.agent_step(&parent, *step)
            }
            Step::Agent { id, name, task } => {
                self.seated.lock().unwrap().push(Agent {
                    name: id,
                    task,
                    title: Some(name),
                });
                self.at_work();
            }
            Step::Result { id, text, error } => {
                // An Agent call run in the foreground ends with its report;
                // a background one's result only says it started.
                let mut seated = self.seated.lock().unwrap();
                let was = seated.len();
                if !text.starts_with("Async agent launched") {
                    seated.retain(|a| a.name != id);
                }
                let gone = seated.len() != was;
                drop(seated);
                if gone {
                    self.at_work();
                }
                self.show(Shown::Step {
                    step: Step::Result { id, text, error },
                    agent: None,
                });
            }
            Step::Done { id, report } => {
                // The report goes to the call, as its result does.
                self.seated.lock().unwrap().retain(|a| a.name != id);
                self.at_work();
                self.show(Shown::Step {
                    step: Step::Done { id, report },
                    agent: None,
                });
            }
            step => self.show(Shown::Step { step, agent: None }),
        }
    }

    fn agent_step(&self, agent: &str, step: Step) {
        self.show(Shown::Step {
            step,
            agent: Some(agent.into()),
        });
    }

    fn agents(&self, now: Vec<(String, String)>) {
        *self.own.lock().unwrap() = now.into_iter().map(Agent::own).collect();
        self.at_work();
    }

    async fn typing(&self, on: bool) {
        // The beat asks twice a second; the page hears only the change.
        if self.typing.swap(on, Ordering::Relaxed) != on {
            self.show(Shown::Typing { on });
        }
        // The seat's run is over, and its subagents with it.
        if !on && !self.seated.lock().unwrap().is_empty() {
            self.seated.lock().unwrap().clear();
            self.at_work();
        }
    }

    /// A card with the draft and buttons, not a message: it answers to
    /// the act's own id.
    async fn ask(&self, p: &Pending) -> Result<String, String> {
        self.show(Shown::Pending { pending: p.clone() });
        Ok(format!("act-{}", p.id))
    }

    async fn settle(&self, p: &Pending, outcome: &str, done: bool) -> Result<(), String> {
        self.show(Shown::Settled {
            id: p.id,
            text: outcome.into(),
            done,
            scratchpad: p.scratchpad,
        });
        Ok(())
    }

    /// Already here: the page sent the bytes with the message, and for a
    /// photo too big or of a kind the model can't see, a JPEG to see.
    async fn fetch(&self, file: &Upload) -> Result<Fetched, String> {
        let read = match &file.view {
            Some(view) => seen(view, Some("image/jpeg")),
            None => seen(&file.data, file.mime.as_deref()),
        };
        let photo =
            file.view.is_some() || matches!(read, Some(Read::Model(Attachment::Image { .. })));
        Ok(Fetched {
            name: file.name.clone(),
            photo,
            data: file.data.clone(),
            read,
        })
    }

    async fn file(&self, name: &str, _data: Vec<u8>, _mime: &str) -> Result<(), String> {
        self.show(Shown::File { name: name.into() });
        Ok(())
    }
}

/// A file sent from the page with a message.
pub struct Upload {
    name: String,
    mime: Option<String>,
    data: Vec<u8>,
    /// The page's JPEG of a photo the model can't take as it is.
    view: Option<Vec<u8>>,
}

struct Site {
    web: Web,
    /// The host's own state, sampled every few seconds (system.rs).
    system: crate::system::Shared,
    hippo: Arc<Hippo>,
    state: PathBuf,
    /// What show_page publishes, which the Library lists.
    pages: PathBuf,
    /// Asks the loop that keeps the pages' stills to look now.
    stills: Arc<tokio::sync::Notify>,
    messages: mpsc::UnboundedSender<(Web, Message<Web>)>,
    /// Verdicts given on the page, by act id.
    verdicts: mpsc::UnboundedSender<(u64, bool)>,
    halt: crate::Halt,
    memory: crate::hippo::Health,
    writer: Arc<crate::synopsis::Harnesses>,
    /// Wakes the loop that tends the scratchpad.
    scratchpad: Arc<tokio::sync::Notify>,
    /// The named memories' hippos, and what wakes the loop running their
    /// chats.
    memories: Arc<crate::memories::Hippos>,
    memories_wake: Arc<tokio::sync::Notify>,
    /// Every request counts as signed in (LIBRI_OPEN). For the development
    /// instance, which the tailnet alone guards; passkeys replace it.
    open: bool,
    /// The tailnet users this host is for (LIBRI_TAILNET): with it set, a
    /// request from anyone else is refused before any route, and one let in
    /// counts as signed in. The tailnet is the sign-in.
    tailnet: Option<crate::tailnet::Tailnet>,
    /// The other origins whose pages may call this host (LIBRI_ORIGINS):
    /// the app served from one address, talking to hosts at others.
    origins: Vec<String>,
    /// Files sent ahead of the run that brings them, by id.
    uploads: Mutex<HashMap<String, (Instant, Upload)>>,
    /// The ids of the latest messages taken: a page that lost the answer
    /// to a send sends it again, and the second copy is dropped.
    taken: crate::journal::Taken,
    who: Who,
    index: PathBuf,
    /// None when the site's address is unknown or the key would not load.
    push: Option<Arc<Push>>,
    /// The xAI sign-in under way, if any. Held in memory: a restart in
    /// between means connecting again.
    xai: Mutex<Option<XaiSignIn>>,
    /// Replies being read aloud now, by key.
    speaking: Mutex<HashMap<String, tokio::sync::watch::Sender<Sofar>>>,
    /// xAI's voices as last asked, and when.
    voices: Mutex<Option<(std::time::Instant, serde_json::Value)>>,
    /// The models on offer, kept a while: each harness is a process to ask.
    offers: Mutex<Option<(std::time::Instant, Vec<crate::catalog::Offer>)>>,
    /// The other harnesses not signed in, kept as long as the offers.
    signed_out: Mutex<Option<(std::time::Instant, Vec<String>)>>,
    /// The subscription windows as last read, and when.
    /// How a connection's sign-in returns the browser here.
    back: crate::connections::Back,
    connecting: crate::connections::Pending,
    /// The shell the page's terminal opens, which agents share.
    terminals: Arc<crate::terminal::Terminals>,
    /// Speech to text, for dictation and calls (stt.rs).
    stt: Arc<crate::stt::Service>,
    /// Text to speech on the host, for calls and reading aloud (tts.rs).
    tts: Arc<crate::tts::Service>,
    /// The host's stable id (host.rs), and when this daemon started.
    host: String,
    started: Instant,
    /// Whether the tool server started when this daemon did; unset until
    /// that is known. A deployment's readiness check can fail a deploy on false.
    tools: Arc<std::sync::OnceLock<bool>>,
    /// Libri's own sessions to the servers, for MCP Apps cards.
    apps: crate::apps::Apps,
    /// Libri's own browser, when this host has one.
    browser: Option<Arc<crate::browser::Browser>>,
}

/// This host, by its deployment's name (LIBRI_NAME, else its hostname).
#[derive(Serialize)]
struct Who {
    name: String,
    /// What the site and its installed app are called: LIBRI_APP, else
    /// Libri, the product.
    app: String,
    /// Whether the person may switch models: always now that every harness
    /// lists its own (the list itself is /api/models). The field goes with
    /// the site's `me.models` checks once chat-one-renderer has landed.
    models: bool,
    /// The windows, in minutes, the person may pick for forgetting; empty
    /// for one that remembers.
    forgets: Vec<u64>,
}

impl Who {
    fn from_env() -> Self {
        let name = std::env::var("LIBRI_NAME").unwrap_or_else(|_| "Chat".into());
        Who {
            app: std::env::var("LIBRI_APP").unwrap_or_else(|_| "Libri".into()),
            name,
            models: true,
            forgets: match std::env::var_os("LIBRI_FORGET") {
                Some(_) => crate::forget::CHOICES.to_vec(),
                None => Vec::new(),
            },
        }
    }
}

impl Who {
    /// What the chat calls the assistant (host.rs).
    fn chat_label(&self) -> &str {
        crate::host::assistant_label(&self.name)
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn token() -> Result<String, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(URL_SAFE_NO_PAD.encode(b))
}

/// Writes a file only its owner reads, whole or not at all, by way of a
/// temporary one named for this process, so two writers never share it.
pub(crate) fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::io::Write::write_all(&mut f, text.as_bytes()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The entry document, titled with the site's name and carrying the
/// assistant's, which the page takes from it: the host no longer names the
/// assistant when several share one. Revalidated each load, as it carries no hash in its name, so
/// a deploy's new asset hashes are picked up.
async fn index(State(site): State<Arc<Site>>) -> Response {
    match fs::read_to_string(&site.index) {
        Ok(html) => (
            [(header::CACHE_CONTROL, "no-cache")],
            axum::response::Html(titled(&html, &site.who.app, site.who.chat_label())),
        )
            .into_response(),
        Err(e) => {
            eprintln!("libri: {}: {e}", site.index.display());
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn titled(html: &str, app: &str, name: &str) -> String {
    let clean = |s: &str| -> String { s.chars().filter(char::is_ascii_alphanumeric).collect() };
    html.replacen(
        "<title>Libri</title>",
        &format!(
            "<title>{}</title><meta name=\"assistant\" content=\"{}\" />",
            clean(app),
            clean(name)
        ),
        1,
    )
}

impl Site {
    /// The voice replies are read aloud in, none while xAI isn't connected.
    fn voice(&self) -> Option<String> {
        crate::x::connected(&self.state)?;
        Some(
            VOICE
                .read(&self.state)
                .or_else(|| std::env::var("LIBRI_VOICE").ok())
                .unwrap_or_else(|| FIRST_VOICE.to_owned()),
        )
    }
}

/// Who answers here: the assistant's name, its voice, this device's scope.
async fn me(State(site): State<Arc<Site>>, headers: HeaderMap) -> Json<serde_json::Value> {
    let mut body = serde_json::json!(site.who);
    // What this device may do, for the page to show a read-only one as
    // such; none when it came in another way (sign-in, an open site).
    body["scope"] = serde_json::json!(site.device(&headers).map(|d| d.scope));
    body["voice"] = serde_json::json!(site.voice());
    // Whether the composer offers dictation.
    body["dictate"] = crate::stt::available(&site.state).into();
    // Whether a call can be made: a speech service, or xAI's voice.
    body["call"] = serde_json::json!(
        crate::speech::Engine::pick(&site.state, site.voice(), &site.tts).is_some()
    );
    Json(body)
}

#[derive(Deserialize)]
struct NewsSince {
    /// The version the device last ran.
    since: String,
}

/// What changed since the version a device last ran, up to this host's,
/// for the one note an app shows after it updates. Empty for a device with
/// no version kept, which is new.
async fn news(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<NewsSince>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "releases": crate::news::since(&q.since).unwrap_or_default(),
    }))
    .into_response()
}

/// The host as the app's connection list shows it: its stable id, name (the
/// machine's hostname), assistant, version and uptime to anyone who reaches
/// it, as T3 Code's environment descriptor does; the other addresses it
/// answers on to a paired client, to learn as further routes.
async fn host(
    State(site): State<Arc<Site>>,
    person: Option<axum::Extension<crate::tailnet::Person>>,
    headers: HeaderMap,
) -> Response {
    let mut body = serde_json::json!({
        "id": site.host,
        "name": crate::host::machine_label(),
        "assistant": site.who.chat_label(),
        "version": env!("CARGO_PKG_VERSION"),
        // The flake's short rev, so web deploys between releases tell apart.
        // The wrapper sets it, not the compile: stamped into the binary, it
        // made every commit recompile the server.
        "commit": std::env::var("LIBRI_COMMIT").ok(),
        "uptime": site.started.elapsed().as_secs(),
        "tools": site.tools.get(),
        // Whether the page asking may use this host: its origin is this
        // host's own or listed (daemon.origins). The page reads this field
        // whatever its origin, so it can say why a host it cannot use refuses
        // it, instead of failing on every call after.
        "admits": site.admits(&headers),
    });
    if site.signed(&headers) {
        body["addresses"] = serde_json::json!(crate::host::addresses());
        body["url"] = serde_json::json!(std::env::var("LIBRI_SITE_URL").ok());
        // Who the tailnet says is asking, on a tailnet-gated host.
        if let Some(axum::Extension(p)) = person {
            body["you"] = serde_json::json!(p);
        }
    }
    // Another host's page lists this one with its latency, so any origin
    // may read it; without credentials it sees only the public fields.
    ([(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")], Json(body)).into_response()
}

#[derive(Deserialize)]
struct Pairing {
    /// The secret from a pairing link.
    #[serde(default)]
    secret: String,
    /// What the person calls the device, to tell it apart when revoking.
    #[serde(default)]
    name: String,
}

/// A device trades the secret from `libri pair` for a token of its own,
/// which it keeps: the page as its session cookie, any other client as a
/// bearer token.
async fn pair(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(p): Json<Pairing>,
) -> Response {
    let name = match p.name.trim() {
        "" => headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("A device")
            .to_owned(),
        n => n.to_owned(),
    };
    let state = site.state.clone();
    let made =
        tokio::task::spawn_blocking(move || crate::host::redeem(&state, &p.secret, &name)).await;
    let (device, token) = match made {
        Ok(Ok(d)) => d,
        Ok(Err(e)) => return (StatusCode::FORBIDDEN, e).into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    // A browser keeps a Secure cookie only from an https page; a host on
    // its own over plain http (a tailnet or LAN address) needs it plain.
    let secure = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|o| o.starts_with("https://"));
    let cookie = format!(
        "{COOKIE}={token}; Path=/; Max-Age={}; HttpOnly;{} SameSite=Lax",
        PAIRED.as_secs(),
        if secure { " Secure;" } else { "" }
    );
    let body = serde_json::json!({
        "token": token,
        "device": { "id": device.id, "name": device.name, "scope": device.scope },
        "host": { "id": site.host, "name": crate::host::machine_label() },
    });
    ([(header::SET_COOKIE, cookie)], Json(body)).into_response()
}

#[derive(Deserialize)]
struct NewPairing {
    #[serde(default)]
    read: bool,
}

/// A link to pair another device, made on the page as `libri pair` makes
/// it on the host: one per address the host answers on.
async fn pair_new(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(p): Json<NewPairing>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let scope = if p.read {
        crate::host::Scope::Read
    } else {
        crate::host::Scope::Full
    };
    let state = site.state.clone();
    match tokio::task::spawn_blocking(move || crate::host::pair(&state, scope)).await {
        Ok(Ok(secret)) => {
            let links: Vec<String> = crate::host::bases()
                .iter()
                .map(|b| crate::host::link(b, &secret))
                .collect();
            Json(serde_json::json!({ "links": links, "minutes": 10 })).into_response()
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// The paired devices, this one marked, for Settings to list and revoke.
async fn devices(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let this = site.device(&headers).map(|d| d.id);
    let all: Vec<serde_json::Value> = crate::host::devices(&site.state)
        .into_iter()
        .map(|d| {
            serde_json::json!({
                "id": d.id,
                "name": d.name,
                "scope": d.scope,
                "paired": d.paired,
                "this": this.as_deref() == Some(d.id.as_str()),
            })
        })
        .collect();
    Json(all).into_response()
}

/// Unpairs a device: its token stops working at once, this one's too.
async fn revoke(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    match tokio::task::spawn_blocking(move || crate::host::revoke(&state, &id)).await {
        Ok(Ok(_)) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(e)) => (StatusCode::NOT_FOUND, e).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// A device paired to look (`libri pair --read`) may read anything and
/// change nothing.
/// Lets the app at another origin (LIBRI_ORIGINS) call this host, as
/// Paseo's daemon lets app.paseo.sh: CORS for those origins, Chrome's
/// local-network preflight, and the token as `?token=` on a GET, for what
/// cannot send a header (the event stream, images, audio). No cookies
/// cross: the app holds each host's token itself.
async fn across(
    State(site): State<Arc<Site>>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .filter(|o| site.origins.iter().any(|a| a == o))
        .and_then(|o| HeaderValue::from_str(o).ok());
    if let Some(origin) = &origin
        && req.method() == axum::http::Method::OPTIONS
    {
        let mut r = StatusCode::NO_CONTENT.into_response();
        let h = r.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
        h.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            // Clear removes the temp chat with DELETE.
            HeaderValue::from_static("GET, POST, DELETE"),
        );
        h.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("authorization, content-type"),
        );
        h.insert(
            "access-control-allow-private-network",
            HeaderValue::from_static("true"),
        );
        h.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("600"),
        );
        h.insert(header::VARY, HeaderValue::from_static("origin"));
        return r;
    }
    if req.method() == axum::http::Method::GET
        && !req.headers().contains_key(header::AUTHORIZATION)
        && let Some(token) = req
            .uri()
            .query()
            .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")))
        && let Ok(v) = HeaderValue::from_str(&format!("Bearer {token}"))
    {
        req.headers_mut().insert(header::AUTHORIZATION, v);
    }
    let mut r = next.run(req).await;
    if let Some(origin) = origin {
        r.headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        r.headers_mut()
            .append(header::VARY, HeaderValue::from_static("origin"));
    }
    r
}

/// Refuses anyone the tailnet does not name as one of this host's people,
/// and tells the routes who it is.
async fn tailnet_gate(
    State(site): State<Arc<Site>>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let Some(gate) = &site.tailnet else {
        return next.run(req).await;
    };
    // Served without the peer's address, nobody can be told apart.
    let Some(&axum::extract::ConnectInfo(peer)) = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
    else {
        return StatusCode::FORBIDDEN.into_response();
    };
    // Behind a proxy on this machine, the client it names (tailnet.rs).
    let real_ip = http::HeaderName::from_static("x-real-ip");
    let Some(client) = crate::tailnet::client(peer, req.headers().get(&real_ip)) else {
        eprintln!("libri: tailnet: refused {peer}: X-Real-IP names no address");
        return StatusCode::FORBIDDEN.into_response();
    };
    match gate.judge(client).await {
        crate::tailnet::Verdict::Local => {}
        crate::tailnet::Verdict::In(person) => {
            req.extensions_mut().insert(person);
        }
        crate::tailnet::Verdict::Forward(to) => {
            // The Libri behind sees this one as its peer on loopback: it
            // judges the client named here, never one the client named.
            if let Ok(v) = http::HeaderValue::from_str(&client.to_string()) {
                req.headers_mut().insert(real_ip, v);
            }
            return crate::forward::pass(&to, req).await;
        }
        crate::tailnet::Verdict::Out(who) => {
            let whom = who.map_or_else(|| client.to_string(), |p| p.login);
            eprintln!("libri: tailnet: refused {whom}");
            return (
                StatusCode::FORBIDDEN,
                "This Libri is not yours: your tailnet account is not one it lets in.",
            )
                .into_response();
        }
    }
    next.run(req).await
}

async fn read_only(
    State(site): State<Arc<Site>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let reads = matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD
    );
    // A read-only device may still pair again, by a new link.
    if !reads
        && req.uri().path() != "/api/pair"
        && !site.open
        && site
            .device(req.headers())
            .is_some_and(|d| d.scope == crate::host::Scope::Read)
    {
        return (StatusCode::FORBIDDEN, "This device may only read.").into_response();
    }
    next.run(req).await
}

/// What makes the page installable as an app, under the site's name.
async fn manifest(State(site): State<Arc<Site>>) -> Response {
    let name = &site.who.app;
    let icon = |size: u32| {
        serde_json::json!({
            "src": format!("/icon/app-{size}.png"),
            "sizes": format!("{size}x{size}"),
            "type": "image/png",
            // Full bleed with the mark in the safe circle: launchers may
            // crop it to their shape rather than plating it.
            "purpose": "any maskable",
        })
    };
    let body = serde_json::json!({
        "name": name,
        "short_name": name,
        "start_url": "/",
        "scope": "/",
        "display": "standalone",
        "id": "/",
        // Chrome paints the installed app's splash and Android navigation
        // bar from these at install and never again: the bar ignores the
        // page's live theme-color (the status bar follows it) and the
        // manifest's dark variants. So the bar is light paper, the default
        // theme, in either theme. Revisit when Chrome draws installed apps
        // edge to edge (crbug 506197579) or recolours the bar live.
        "background_color": "#ede7dd",
        "theme_color": "#ede7dd",
        "icons": [
            icon(192),
            icon(512),
            {"src": "/icon/app.svg", "sizes": "any", "type": "image/svg+xml"},
        ],
    });
    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        body.to_string(),
    )
        .into_response()
}

fn session(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|c| c.trim().strip_prefix(COOKIE)?.strip_prefix('='))
}

impl Site {
    /// Whether a request comes from a page this host serves or lets in: no
    /// origin (not a browser page), this host's own, or one listed in
    /// LIBRI_ORIGINS.
    fn admits(&self, headers: &HeaderMap) -> bool {
        let text = |h| headers.get(h).and_then(|v: &HeaderValue| v.to_str().ok());
        match text(header::ORIGIN) {
            None => true,
            Some(o) => {
                self.origins.iter().any(|a| a == o)
                    || o.split_once("://").map(|(_, h)| h) == text(header::HOST)
            }
        }
    }

    fn signed(&self, headers: &HeaderMap) -> bool {
        // Every request a tailnet-gated host answers has passed the gate.
        self.open || self.tailnet.is_some() || self.device(headers).is_some()
    }

    /// The paired device a request comes from: its token as the session
    /// cookie (the page) or a bearer token (any other client).
    fn device(&self, headers: &HeaderMap) -> Option<crate::host::Device> {
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        [bearer, session(headers)]
            .into_iter()
            .flatten()
            .find_map(|t| crate::host::device(&self.state, t))
    }
}

/// The channels the page reaches the core by: what the person sends,
/// with the door to answer through, and their verdicts on acts.
pub struct Inbox {
    pub messages: mpsc::UnboundedSender<(Web, Message<Web>)>,
    pub verdicts: mpsc::UnboundedSender<(u64, bool)>,
    /// Set to stop the turn under way.
    pub halt: crate::Halt,
    /// Wakes the loop that tends the scratchpad.
    pub scratchpad: Arc<tokio::sync::Notify>,
    /// The named memories' hippos, and what wakes the loop running their
    /// chats.
    pub memories: Arc<crate::memories::Hippos>,
    pub memories_wake: Arc<tokio::sync::Notify>,
    /// What writes workers' titles and summaries.
    pub writer: Arc<crate::synopsis::Harnesses>,
}

/// What show_page wrote, sandboxed: a page shares the chat's address, so
/// unsandboxed its scripts could read the chat and act as the person. In
/// the sandbox it has an origin of its own, which the session cookie
/// (SameSite=Lax) never reaches.
const PAGE_SANDBOX: &str =
    "sandbox allow-scripts allow-forms allow-popups allow-modals allow-downloads";

fn pages_of(dir: PathBuf) -> SetResponseHeader<ServeDir, HeaderValue> {
    SetResponseHeaderLayer::overriding(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(PAGE_SANDBOX),
    )
    .layer(ServeDir::new(dir))
}

/// Serves the chat page from `root` at `/`, the API, and every other path
/// from `pages` (what show_page wrote) on `addr`, through the pages `web`.
#[allow(clippy::too_many_arguments)]
pub async fn serve(
    addr: &str,
    root: PathBuf,
    pages: PathBuf,
    state: &Path,
    hippo: Arc<Hippo>,
    web: Web,
    inbox: Inbox,
) -> Result<(), String> {
    let dir = state.join("site");
    let who = Who::from_env();
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    web.keep_turn_in(dir.join("turn.json"));
    // A connection's tool that asks first asks here, in the main chat when
    // its relay names no turn.
    if let Err(e) = crate::connections::serve_asks(state, Arc::new(web.clone())) {
        eprintln!("libri: connections cannot ask: {e}");
    }
    let tools = Arc::new(std::sync::OnceLock::new());
    let told = tools.clone();
    std::thread::spawn(move || {
        let checked = crate::servers::check();
        if let Err(e) = &checked {
            eprintln!("libri: the tool server failed: {e}");
        }
        let _ = told.set(checked.is_ok());
    });
    web.keep_cut_words(&hippo);
    let stills = Arc::new(tokio::sync::Notify::new());
    tokio::spawn(crate::snapshot::keep(
        state.to_owned(),
        pages.clone(),
        stills.clone(),
    ));
    let push = push_from_env(&dir, &who.name).map(Arc::new);
    let needs = state.to_owned();
    let browser = crate::browser::Browser::new(
        state,
        push.clone(),
        Box::new(move |worker, ask| {
            let set = match ask {
                Some(a) => {
                    crate::task::set_need(&needs, worker, crate::lifecycle::NeedKind::Answer, a)
                }
                None => crate::task::unset_need(&needs, worker),
            };
            if let Err(e) = set {
                eprintln!("libri: browser: {worker}'s card: {e}");
            }
        }),
    );
    if let Some(b) = &browser {
        let (b, state) = (b.clone(), state.to_owned());
        tokio::spawn(async move {
            if let Err(e) = b.serve(&state).await {
                eprintln!("libri: browser: {e}");
            }
        });
    } else {
        let _ = std::fs::remove_file(state.join(crate::browser::SOCKET));
    }
    let terminals = Arc::new(crate::terminal::Terminals::default());
    {
        let (t, dir) = (terminals.clone(), state.to_owned());
        tokio::spawn(async move {
            if let Err(e) = t.serve(&dir).await {
                eprintln!("libri: terminal: {e}");
            }
        });
        // A shell no tile names any more (its tile closed, its workspace
        // deleted from Recently closed or aged out) ends; one a page opened
        // a moment ago has a minute for its layout to be kept.
        let (t, dir) = (terminals.clone(), state.to_owned());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let s = dir.clone();
                if let Ok(kept) =
                    tokio::task::spawn_blocking(move || crate::layout::shells(&s)).await
                {
                    t.end_unnamed(&kept, Duration::from_secs(60));
                }
            }
        });
    }
    let system = crate::system::Monitor::new();
    crate::system::start(system.clone(), state.to_owned());
    let site = Arc::new(Site {
        stt: crate::stt::Service::new(state.to_owned()),
        tts: crate::tts::Service::new(state.to_owned()),
        web,
        system,
        memory: crate::hippo::Health::new(hippo.clone()),
        hippo,
        state: state.to_owned(),
        pages: pages.clone(),
        messages: inbox.messages,
        verdicts: inbox.verdicts,
        halt: inbox.halt,
        scratchpad: inbox.scratchpad,
        memories: inbox.memories,
        memories_wake: inbox.memories_wake,
        writer: inbox.writer,
        open: std::env::var_os("LIBRI_OPEN").is_some(),
        tailnet: crate::tailnet::Tailnet::from_env(),
        origins: std::env::var("LIBRI_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|o| o.trim().trim_end_matches('/').to_owned())
            .filter(|o| !o.is_empty())
            .collect(),
        host: crate::host::id(state)?,
        started: Instant::now(),
        tools: tools.clone(),
        uploads: Mutex::default(),
        taken: crate::journal::Taken::load(state),
        push,
        browser,
        who,
        index: root.join("index.html"),
        xai: Mutex::default(),
        speaking: Mutex::default(),
        voices: Mutex::default(),
        offers: Mutex::default(),
        signed_out: Mutex::default(),
        stills: stills.clone(),
        back: crate::connections::Back::new(
            std::env::var("LIBRI_RETURN_PAGE").ok().as_deref(),
            addr.rsplit_once(':')
                .and_then(|(_, p)| p.parse().ok())
                .unwrap_or(0),
        ),
        connecting: Default::default(),
        terminals,
        apps: Default::default(),
    });
    if let Some(push) = &site.push {
        tokio::spawn(ring(site.web.shown.subscribe(), push.clone()));
    }
    // The entry document, the worker and the icons carry no hash in their
    // names, so the browser must revalidate them each load; otherwise a
    // deploy's new asset hashes, or a new icon, never get picked up. The
    // hashed assets under /_site are safe to keep forever, but only once
    // found: a miss mid-deploy must not stick.
    let revalidate = SetResponseHeaderLayer::overriding(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );
    let forever = SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, |r: &Response<_>| {
        r.status()
            .is_success()
            .then(|| HeaderValue::from_static("public, max-age=31536000, immutable"))
    });
    let app = Router::new()
        .route("/", get(index))
        // At the root, so its scope is the whole site.
        .route_service(
            "/sw.js",
            revalidate.clone().layer(ServeFile::new(root.join("sw.js"))),
        )
        // Public build assets, open to any origin: a card or page runs in
        // an opaque-origin frame, and fetches the host's fonts by CORS.
        .nest_service(
            "/_site",
            SetResponseHeaderLayer::overriding(
                header::ACCESS_CONTROL_ALLOW_ORIGIN,
                HeaderValue::from_static("*"),
            )
            .layer(
                forever.layer(
                    // The build keeps each asset beside its .br and .gz (flake
                    // site), served to a browser that takes them.
                    ServeDir::new(root.join("_site"))
                        .precompressed_br()
                        .precompressed_gzip(),
                ),
            ),
        )
        .nest_service(
            "/icon",
            revalidate.clone().layer(ServeDir::new(root.join("icon"))),
        )
        .route("/manifest.webmanifest", get(manifest))
        // A whiteboard alone (site/board.html): the Android app's place of
        // one, without the whole app.
        .route_service(
            "/board.html",
            revalidate.layer(ServeFile::new(root.join("board.html"))),
        )
        .route("/api/me", get(me))
        .route("/api/host", get(host))
        .route("/api/news", get(news))
        .route("/api/pair", post(pair))
        .route("/api/pair/new", post(pair_new))
        .route("/api/devices", get(devices))
        .route("/api/devices/{id}/revoke", post(revoke))
        .route("/api/history", get(history))
        .route("/api/find", get(find))
        .route("/api/around", get(around))
        .route("/api/sources", get(sources))
        .route("/api/agent/{id}/chat", get(agent_chat))
        .route("/api/file/{name}", get(kept))
        .route("/api/speak", post(speak))
        .route("/api/speak/{key}", get(spoken))
        .route("/api/voices", get(voices))
        .route(
            "/api/transcribe",
            post(transcribe).layer(DefaultBodyLimit::max(DICTATION_MAX)),
        )
        .route("/api/stt", get(stt_choice).post(stt_choose))
        .route("/api/stt/models/{id}", post(stt_download))
        .route("/api/stt/stream", get(stt_stream))
        .route("/api/tts", get(tts_voices).post(tts_speak))
        .route("/api/usage", get(usage).post(usage_shown))
        .route("/api/system", get(system_state))
        .route("/api/compact", get(compact).post(compact_pick))
        .route("/api/usage/report", get(usage_report))
        .route("/api/usage/spend", get(usage_spend))
        .route(crate::a2a::CARD, get(agent_card))
        .route(crate::a2a::PATH, post(a2a_message))
        .route(crate::a2a::PAIR, post(a2a_pair))
        .route(crate::a2a::APPROVE, post(a2a_approve))
        .route(crate::a2a::UNPAIR, post(a2a_unpair))
        .route("/api/peers", get(peers_listed).post(peer_ask))
        .route("/api/peers/approve", post(peer_approve))
        .route("/api/peers/revoke", post(peer_revoke))
        .route("/api/models", get(models))
        .route("/api/settings", get(settings_listed))
        .route("/docs/settings.md", get(settings_page))
        .route("/api/model", post(model_pick))
        .route("/api/effort", post(effort_pick))
        .route("/api/access", post(access_pick))
        .route("/api/access/default", post(access_default))
        .route("/api/others", post(others_pick))
        .route("/api/harness", post(harness_set))
        .route("/api/capabilities", get(capabilities).post(capability_pick))
        .route("/api/signin", get(signin_now).post(signin))
        .route("/api/signin/progress", get(signin_progress))
        .route("/api/forget", get(forget_now).post(forget_pick))
        .route("/api/synopsis", get(synopsis_now).post(synopsis_pick))
        .route(
            "/api/worker-model",
            get(worker_model).post(worker_model_pick),
        )
        .route(
            "/api/worker-names",
            get(worker_names).post(worker_name_change),
        )
        .route("/api/guide", get(guide_rules).post(guide_change))
        .route("/api/guide/host", get(host_facts).post(host_facts_change))
        .route("/api/skills", get(skills_now).post(skill_save))
        .route("/api/skills/{name}/delete", post(skill_delete))
        .route("/api/skills/menu", get(skills_menu))
        .route("/api/skills/managed", post(skills_managed))
        .route("/api/skills/source", post(skills_source))
        .route("/api/skills/pull", post(skills_pull))
        .route("/api/memory", get(memory_health))
        .route("/api/call", post(call))
        .route("/api/call/end", post(call_end))
        .route("/api/call/socket", get(call_socket))
        .route("/api/call/ask", post(call_ask))
        .route("/api/events", get(events))
        .route("/api/run", post(run))
        .route(
            "/api/upload",
            post(upload).layer(DefaultBodyLimit::max(UPLOAD_MAX)),
        )
        .route("/api/pending", get(waiting))
        .route("/api/decide", post(decide))
        .route("/api/stop", post(stop))
        .route("/api/session/clear", post(session_clear))
        .route("/api/commands", get(commands_now))
        .route("/api/task", get(task_now))
        .route("/api/library", get(library))
        .route("/api/library/{name}/snapshot", get(snapshot))
        .route("/api/library/{name}/call", post(page_call))
        .route("/api/watches", get(watches))
        .route("/api/watches/{id}/delete", post(watch_delete))
        .route("/api/watches/{id}/snooze", post(watch_snooze))
        .route("/api/work", get(work_list).post(work_start))
        .route("/api/work-choice", get(work_default_choice))
        .route("/api/work/{name}", get(work_log))
        .route("/api/work/{name}/archive", post(work_archive))
        .route("/api/work/{name}/restore", post(work_restore))
        .route("/api/work/{name}/merge", post(work_merge))
        .route("/api/work/{name}/nap", post(work_nap))
        .route("/api/work/{name}/wake", post(work_wake))
        .route("/api/work/{name}/read", post(work_read))
        .route("/api/work/{name}/clear", post(work_clear))
        .route(
            "/api/work/{name}/choice",
            get(work_choice).post(work_choose),
        )
        .route(
            "/api/scratchpad",
            get(scratchpad_read)
                .post(scratchpad_tell)
                .delete(scratchpad_close)
                .layer(DefaultBodyLimit::max(SCRATCHPAD_BODY_MAX)),
        )
        .route("/api/scratchpad/stop", post(scratchpad_stop))
        .route("/api/scratchpad/new", post(scratchpad_new))
        .route("/api/scratchpad/memory", post(scratchpad_memory))
        .route("/api/scratchpad/promote", post(scratchpad_promote))
        .route("/api/memories", get(memories_listed).post(memory_add))
        .route(
            "/api/memories/{id}",
            post(memory_change).delete(memory_delete),
        )
        .route(
            "/api/memories/{id}/chat",
            get(memory_chat_read)
                .post(memory_chat_tell)
                .layer(DefaultBodyLimit::max(SCRATCHPAD_BODY_MAX)),
        )
        .route("/api/memories/{id}/stop", post(memory_chat_stop))
        .route(
            "/api/memories/{id}/choice",
            get(memory_choice).post(memory_choose),
        )
        .route("/api/agent/{name}/stop", post(agent_stop))
        .route("/api/agent/{name}/tell", post(agent_tell))
        .route("/api/push", get(push_key).post(push_add))
        .route("/api/push/drop", post(push_drop))
        .route("/api/terminal", get(terminal))
        .route("/api/browser/live", get(browser_live))
        .route(
            "/api/browser/choose",
            post(browser_choose).layer(DefaultBodyLimit::max(UPLOAD_MAX)),
        )
        .route("/api/connections", get(connections_now))
        .route("/api/connections/host", get(host_servers_now))
        .route("/api/builtins", get(builtins_now))
        .route("/api/builtins/{id}", post(builtin_set))
        .route("/api/builtins/{id}/call", post(builtin_call))
        .route("/api/connections/providers", get(providers_now))
        .route("/api/connections/providers/{id}", post(provider_connect))
        .route("/api/connections/servers/{name}/tools", post(tool_rule))
        .route(
            "/api/connections/servers/{name}/items",
            get(items_now).post(item_show),
        )
        .route("/api/connections/servers", post(server_put))
        .route("/api/connections/servers/{name}/test", post(server_test))
        .route(
            "/api/connections/servers/{name}/sign-in",
            post(server_sign_in),
        )
        .route(
            "/api/connections/servers/{name}/remove",
            post(server_remove),
        )
        .route("/api/connections/finish", post(server_finish))
        .route("/api/connections/back", get(server_back))
        .route("/api/connections/xai", post(xai_start))
        .route("/api/connections/xai/key", post(xai_key))
        .route("/api/connections/xai/drop", post(xai_drop))
        .route("/api/voice", post(voice_pick))
        .route("/api/tiles", get(tiles_listed))
        .route(
            "/api/layout",
            get(layout_now)
                .post(layout_keep)
                .layer(DefaultBodyLimit::max(crate::layout::LARGEST)),
        )
        .route("/api/layout/undo", post(layout_undo))
        .route("/api/boards", get(boards_list).post(board_create))
        .route("/api/boards/order", post(boards_order))
        .route(
            "/api/boards/{id}",
            get(board_scene)
                .post(board_merge)
                .layer(DefaultBodyLimit::max(32 << 20)),
        )
        .route("/api/boards/{id}/name", post(board_rename))
        .route("/api/boards/{id}/remove", post(board_remove))
        .route("/api/boards/{id}/focus", post(board_focus))
        .route("/api/boards/{id}/export", get(board_export))
        .route(
            "/api/boards/{id}/links",
            get(board_links).post(board_link_create),
        )
        .route("/api/boards/links/{token}/revoke", post(board_link_revoke))
        .route(
            "/api/phone-layout",
            get(phone_layout_now)
                .post(phone_layout_keep)
                .layer(DefaultBodyLimit::max(crate::layout::LARGEST)),
        )
        .route("/api/phone-layout/undo", post(phone_layout_undo))
        .route("/api/apps", get(apps_cards))
        .route("/api/apps/view", get(apps_view))
        .route("/api/apps/call", post(apps_call))
        .route("/api/apps/read", post(apps_read))
        .route("/api/apps/sandbox", get(apps_sandbox))
        .merge(connectors::routes())
        .nest_service("/mcp", outside_service(site.clone()))
        .merge(crate::hook::router(site.state.clone()))
        .fallback_service(pages_of(pages))
        .layer(axum::middleware::from_fn_with_state(
            site.clone(),
            read_only,
        ))
        .layer(axum::middleware::from_fn_with_state(site.clone(), across))
        // Outermost, so nobody the tailnet gate refuses reaches any route.
        .layer(axum::middleware::from_fn_with_state(
            site.clone(),
            tailnet_gate,
        ))
        .with_state(site);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                format!("site {addr}: in use, perhaps by another Libri; give this one `--port PORT`, and `--instance NAME` to keep its state apart")
            } else {
                format!("site {addr}: {e}")
            }
        })?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .map_err(|e| format!("site: {e}"))
}

/// Libri as an MCP server for other apps, behind the gate every route is,
/// and then only for a connector's own token: a session or device token,
/// or the tailnet alone, is no sign-in here (the MCP spec's resource
/// server takes only tokens issued for it).
fn outside_service(site: Arc<Site>) -> Router {
    use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
    };
    let made = site.clone();
    // rmcp checks Host against loopback names to stop DNS rebinding of a
    // server with no sign-in of its own; this one is reached by the host's
    // own names and takes only a connector's bearer token, which a page
    // rebound to it cannot hold.
    let config = StreamableHttpServerConfig::default().disable_allowed_hosts();
    let service = StreamableHttpService::new(
        move || Ok(outside::Outside::new(made.clone())),
        LocalSessionManager::default().into(),
        config,
    );
    Router::new()
        .fallback_service(service)
        .layer(axum::middleware::from_fn_with_state(site, outside_gate))
}

async fn outside_gate(
    State(site): State<Arc<Site>>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let state = site.state.clone();
    let headers = req.headers().clone();
    let grant = tokio::task::spawn_blocking(move || connectors::bearer(&state, &headers))
        .await
        .ok()
        .flatten();
    let Some(grant) = grant else {
        return connectors::challenge(
            req.headers(),
            "Sign in: add Libri as a connector, or give the key Settings › Other apps made.",
        );
    };
    req.extensions_mut().insert(grant);
    next.run(req).await
}

fn push_from_env(dir: &Path, title: &str) -> Option<Push> {
    let url = std::env::var("LIBRI_SITE_URL").ok()?;
    Push::load(dir, &url, title)
        .inspect_err(|e| eprintln!("libri: push: {e}"))
        .ok()
}

/// Rings every device for what Libri says and what waits on the
/// person's word; the rest (steps, typing, reactions) only matters on an
/// open page.
async fn ring(mut rx: broadcast::Receiver<Shown>, push: Arc<Push>) {
    loop {
        let (body, tag) = match rx.recv().await {
            Ok(Shown::Said { text, .. }) => (push::gist(&text), "said"),
            Ok(Shown::Pending { pending }) => (push::gist(&pending.act.show()), "pending"),
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        };
        let push = push.clone();
        tokio::task::spawn_blocking(move || {
            push.send(&push::Note {
                title: &push.title,
                body,
                tag,
            })
        });
    }
}

/// The key a device subscribes with; 404 when push is off.
async fn push_key(State(site): State<Arc<Site>>) -> Response {
    match &site.push {
        Some(p) => p.public().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// A device's subscription, as the browser's `PushSubscription.toJSON()`.
async fn push_add(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(sub): Json<serde_json::Value>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(push) = site.push.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::task::spawn_blocking(move || push.subscribe(sub)).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, e).into_response(),
        Err(e) => {
            eprintln!("libri: push: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Deserialize)]
struct Endpoint {
    endpoint: String,
}

async fn push_drop(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(e): Json<Endpoint>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(push) = site.push.clone() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::task::spawn_blocking(move || push.unsubscribe(&e.endpoint)).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(e)) => {
            eprintln!("libri: push: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        Err(e) => {
            eprintln!("libri: push: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// The last messages, oldest first: the person's, Libri's and notes.
async fn history(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let h = site.hippo.clone();
    match tokio::task::spawn_blocking(move || h.tail(HISTORY)).await {
        Ok(Ok(all)) => Json(lines(all)).into_response(),
        Ok(Err(e)) => {
            eprintln!("libri: site history: {e}");
            StatusCode::BAD_GATEWAY.into_response()
        }
        Err(e) => {
            eprintln!("libri: site history: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// A seat subagent's whole chat, for its page once it is done.
async fn agent_chat(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let h = site.hippo.clone();
    match tokio::task::spawn_blocking(move || h.agent(&id)).await {
        Ok(Ok(text)) => Json(serde_json::json!({ "text": text })).into_response(),
        Ok(Err(_)) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            eprintln!("libri: site agent chat: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Deserialize)]
struct Find {
    q: String,
    /// `best` (the default) or `new`.
    order: Option<String>,
}

/// Messages of main's chat and the workers' logs matching words, as a
/// person searches: hippo's `find`, its JSON passed on.
async fn find(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Query(Find { q, order }): axum::extract::Query<Find>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if q.trim().is_empty() || q.len() > 200 {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let order = order.unwrap_or_else(|| "best".into());
    let h = site.hippo.clone();
    raw_json(
        tokio::task::spawn_blocking(move || h.find(&q, &order)).await,
        "find",
    )
}

#[derive(Deserialize)]
struct Around {
    /// Empty for main's chat, else the worker's folder.
    #[serde(default)]
    src: String,
    at: u64,
    before: u64,
    after: u64,
}

/// A found message in its place: the lines around it, whole up to a cut.
async fn around(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Query(a): axum::extract::Query<Around>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let h = site.hippo.clone();
    raw_json(
        tokio::task::spawn_blocking(move || h.around(&a.src, a.at, a.before, a.after)).await,
        "around",
    )
}

/// The workers search reaches, with the names people know them by.
async fn sources(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let h = site.hippo.clone();
    raw_json(
        tokio::task::spawn_blocking(move || h.sources()).await,
        "sources",
    )
}

/// hippo's own JSON as the answer. Its refusals (words it can't search,
/// an index still being built) are the asker's to see.
fn raw_json(out: Result<Result<String, String>, tokio::task::JoinError>, what: &str) -> Response {
    match out {
        Ok(Ok(json)) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
        Ok(Err(e)) => (StatusCode::SERVICE_UNAVAILABLE, e).into_response(),
        Err(e) => {
            eprintln!("libri: site {what}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// A kept file, for the person to save: only by its exact kept name.
async fn kept(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(path) = Library::new(&site.state).path(&name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(&path).await {
        Ok(data) => {
            let mime = mime_of(&name, &data);
            // Saved, never shown in place: a kept HTML or SVG file is the
            // person's data, not part of this site.
            let saved = format!("attachment; filename*=UTF-8''{}", percent(&name));
            (
                [
                    (header::CONTENT_TYPE, mime),
                    (header::CONTENT_DISPOSITION, saved),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
                ],
                data,
            )
                .into_response()
        }
        Err(e) => {
            eprintln!("libri: file {name}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// A name as RFC 5987 wants it in a header: unreserved bytes as they
/// are, the rest as %XX.
fn percent(name: &str) -> String {
    name.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Deserialize)]
struct Speak {
    text: String,
    /// One of the voices the page picked; else the assistant's own.
    voice: Option<String>,
}

/// A reply being read aloud: the MP3 so far, and how it ended once it has
/// (true when whole). Every listener plays it from the start.
#[derive(Default)]
struct Sofar {
    mp3: Vec<u8>,
    end: Option<bool>,
}

/// Starts reading a reply aloud in this assistant's voice and answers its
/// key, which `/api/speak/{key}` plays while xAI is still making it. Kept
/// by voice and text rather than message id, since a live reply's id is
/// the door's and changes once it is in the history.
async fn speak(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(Speak {
        text,
        voice: picked,
    }): Json<Speak>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(own) = site.voice() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let voice = match picked {
        Some(v) if crate::x::voice_ok(&v) => v,
        Some(_) => return StatusCode::BAD_REQUEST.into_response(),
        None => own,
    };
    let text = plain(&text);
    if text.is_empty() || text.chars().count() > crate::x::SPEAK_MAX {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let key = format!("{:016x}", speech_key(&voice, &text));
    let dir = site.state.join("speech");
    let file = dir.join(format!("{key}.mp3"));
    let answer = Json(serde_json::json!({ "key": key })).into_response();
    let tx = {
        let mut speaking = site.speaking.lock().unwrap();
        if file.exists() || speaking.contains_key(&key) {
            return answer;
        }
        // Each reading holds a blocking thread and an xAI session.
        if speaking.len() >= SPEAKING_MAX {
            return StatusCode::TOO_MANY_REQUESTS.into_response();
        }
        let (tx, _) = tokio::sync::watch::channel(Sofar::default());
        speaking.insert(key.clone(), tx.clone());
        tx
    };
    let site = site.clone();
    tokio::task::spawn_blocking(move || {
        let made = crate::x::speak(&site.state, &text, &voice, |b| {
            tx.send_modify(|s| s.mp3.extend_from_slice(b));
        });
        // Only a whole reading is kept; a cut one is made afresh next time.
        if let Err(e) = made.and_then(|()| {
            prune(&dir, SPEECH_KEEP);
            let tmp = file.with_extension("part");
            fs::create_dir_all(&dir)
                .and_then(|_| fs::write(&tmp, &tx.borrow().mp3))
                .and_then(|_| fs::rename(&tmp, &file))
                .map_err(|e| format!("{}: {e}", file.display()))
        }) {
            eprintln!("libri: site speak: {e}");
            tx.send_modify(|s| s.end = Some(false));
        } else {
            tx.send_modify(|s| s.end = Some(true));
        }
        site.speaking.lock().unwrap().remove(&key);
    });
    answer
}

/// A reply read aloud, as MP3: from the cache once whole, else as xAI
/// makes it. nginx is told not to buffer, or the page would hear nothing
/// until the end.
async fn spoken(
    State(site): State<Arc<Site>>,
    axum::extract::Path(key): axum::extract::Path<String>,
    req: axum::extract::Request,
) -> Response {
    if !site.signed(req.headers()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if key.len() != 16 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let rx = site
        .speaking
        .lock()
        .unwrap()
        .get(&key)
        .map(|tx| tx.subscribe());
    let Some(rx) = rx else {
        let file = site.state.join("speech").join(format!("{key}.mp3"));
        // ServeFile answers Range requests, which the player seeks with.
        return match ServeFile::new(file).try_call(req).await {
            Ok(r) => r.into_response(),
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        };
    };
    (
        [
            (header::CONTENT_TYPE, "audio/mpeg"),
            (header::CACHE_CONTROL, "no-store"),
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        axum::body::Body::from_stream(follow(rx)),
    )
        .into_response()
}

/// A reading as one listener hears it: from the first byte, each new piece
/// as it comes, then the end, or an error if it was cut. `done` once over.
fn follow(
    rx: tokio::sync::watch::Receiver<Sofar>,
) -> impl futures_util::Stream<Item = std::io::Result<Vec<u8>>> {
    futures_util::stream::unfold((rx, 0, false), |(mut rx, at, done)| async move {
        if done {
            return None;
        }
        let mut gone = false;
        loop {
            // Some(next piece, or None at the end); None to wait for more.
            let step = {
                let s = rx.borrow_and_update();
                if s.mp3.len() > at {
                    Some(Some(Ok(s.mp3[at..].to_vec())))
                } else {
                    match s.end {
                        Some(true) => Some(None),
                        // An error, not an end, so the player knows it was cut.
                        Some(false) => Some(Some(Err(std::io::Error::other("speech cut")))),
                        None if gone => Some(None),
                        None => None,
                    }
                }
            };
            match step {
                Some(Some(Ok(b))) => {
                    let at = at + b.len();
                    return Some((Ok(b), (rx, at, false)));
                }
                Some(Some(Err(e))) => return Some((Err(e), (rx, at, true))),
                Some(None) => return None,
                None => gone = rx.changed().await.is_err(),
            }
        }
    })
}

/// How long a call's pass to Grok Voice lasts. It only opens the call,
/// which runs on once open.
const CALL_PASS: u64 = 120;

/// The voice the person picked, as Settings > Voice saves it.
pub const VOICE: crate::model::Kept = crate::model::Kept("voice");
/// The voice until the person picks one.
const FIRST_VOICE: &str = "rex";

/// A device-code sign-in shown in Settings > Connections.
struct XaiSignIn {
    url: String,
    code: String,
    /// Why it ended without connecting.
    failed: Option<String>,
}

/// How long xAI's voice list is trusted before it is asked again.
const VOICES_FRESH: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// The voices the page may pick for reading aloud and calls, `[{id, name}]`,
/// as xAI lists them; only where the assistant has a voice.
/// Dictation: a recording in, its words out, on the person's choice of
/// where speech is written down (stt.rs). The audio is held only for this
/// request.
async fn transcribe(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    audio: axum::body::Bytes,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let kind = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    if audio.is_empty() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let stt = site.stt.clone();
    match tokio::task::spawn_blocking(move || crate::stt::transcribe(&stt, &audio, &kind)).await {
        Ok(Ok(text)) => Json(serde_json::json!({ "text": text })).into_response(),
        Ok(Err(e)) => {
            eprintln!("libri: dictation: {e}");
            (StatusCode::BAD_GATEWAY, e).into_response()
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Where speech is written down, the local models with what is installed
/// and on its way, and whether this build carries the local engine.
async fn stt_choice(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = &site.state;
    let choice = crate::stt::choice(state);
    let downloads = site.stt.downloads();
    let models: Vec<serde_json::Value> = crate::stt::MODELS
        .iter()
        .map(|m| {
            let mut v = serde_json::to_value(m).unwrap_or_default();
            v["installed"] = crate::stt::installed(state, m).into();
            if let Some((got, total)) = downloads.get(m.id) {
                v["downloading"] = serde_json::json!({ "got": got, "total": total });
            }
            v
        })
        .collect();
    Json(serde_json::json!({
        "choice": choice,
        "models": models,
        "local": crate::stt::LOCAL_BUILT,
        "xai": crate::x::connected(state).is_some(),
        "openai_key": crate::stt::has_openai_key(state),
    }))
    .into_response()
}

/// A change to the choice: what is left out stays as it was.
#[derive(Deserialize)]
struct SttChoose {
    provider: Option<crate::stt::Provider>,
    model: Option<String>,
    openai_url: Option<String>,
    openai_model: Option<String>,
    /// A new key for the OpenAI-compatible API; empty forgets it.
    openai_key: Option<String>,
}

/// Sets where speech is written down; choosing a local model that isn't
/// here starts fetching it.
async fn stt_choose(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(asked): Json<SttChoose>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !site.admits(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let state = &site.state;
    let mut choice = crate::stt::choice(state);
    choice.provider = asked.provider.unwrap_or(choice.provider);
    choice.model = asked.model.unwrap_or(choice.model);
    choice.openai_url = asked.openai_url.unwrap_or(choice.openai_url);
    choice.openai_model = asked.openai_model.unwrap_or(choice.openai_model);
    if let Err(e) = crate::stt::choose(state, &choice) {
        return (StatusCode::BAD_REQUEST, e).into_response();
    }
    if let Some(key) = &asked.openai_key
        && let Err(e) = crate::stt::set_openai_key(state, key)
    {
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }
    if choice.provider == crate::stt::Provider::Local && crate::stt::LOCAL_BUILT {
        let _ = site.stt.download(&choice.model);
    }
    stt_choice(State(site), headers).await
}

/// Starts fetching a local model.
async fn stt_download(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match site.stt.download(&id) {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

#[derive(Deserialize)]
struct SttStreamQuery {
    lang: Option<String>,
    silence_ms: Option<u32>,
    max_ms: Option<u32>,
}

/// Live speech to text: 16 kHz mono s16le PCM in as binary frames, JSON
/// events out (stt.rs): `ready`, `speech_started`, `partial`, `final`,
/// `done` after the client's `{"type":"end"}`, or `error`.
async fn stt_stream(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<SttStreamQuery>,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !site.admits(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut options = crate::stt::Options {
        lang: q.lang.filter(|l| !l.is_empty() && l != "auto"),
        ..Default::default()
    };
    if let Some(ms) = q.silence_ms {
        options.endpoint.trailing_silence_ms = ms.clamp(200, 5000);
    }
    if let Some(ms) = q.max_ms {
        options.endpoint.max_utterance_ms = ms.clamp(2000, 60_000);
    }
    let stt = site.stt.clone();
    ws.on_upgrade(move |socket| stt_serve(stt, options, socket))
}

async fn stt_serve(
    stt: Arc<crate::stt::Service>,
    options: crate::stt::Options,
    mut socket: axum::extract::ws::WebSocket,
) {
    use axum::extract::ws::Message as Ws;
    let say = |v: serde_json::Value| Ws::Text(v.to_string().into());
    let opened = {
        let stt = stt.clone();
        tokio::task::spawn_blocking(move || stt.open(options)).await
    };
    let mut session = match opened {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            let _ = socket
                .send(say(serde_json::json!({ "type": "error", "message": e })))
                .await;
            return;
        }
        Err(_) => return,
    };
    let (provider, model) = session.about();
    let ready = serde_json::json!({ "type": "ready", "provider": provider, "model": model });
    if socket.send(say(ready)).await.is_err() {
        return;
    }
    // The model works on its own thread, so a slow decode never holds the
    // socket; audio and the end go to it in order.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Option<Vec<i16>>>();
    // None from the model's thread: the session has finished.
    let (etx, mut erx) =
        tokio::sync::mpsc::unbounded_channel::<Result<Option<Vec<crate::stt::Event>>, String>>();
    std::thread::spawn(move || {
        while let Some(chunk) = rx.blocking_recv() {
            let done = chunk.is_none();
            let events = match chunk {
                Some(pcm) => session.push(&pcm),
                None => session.finish(),
            };
            let failed = events.is_err();
            if etx.send(events.map(Some)).is_err() || failed {
                return;
            }
            if done {
                let _ = etx.send(Ok(None));
                return;
            }
        }
    });
    let mut ended = false;
    loop {
        tokio::select! {
            got = socket.recv() => match got {
                Some(Ok(Ws::Binary(bytes))) if !ended => {
                    let pcm = bytes.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
                    let _ = tx.send(Some(pcm));
                }
                // The end, as `{"type":"end"}` or the bare word.
                Some(Ok(Ws::Text(text))) if !ended && (text.trim() == "end" || text.contains("\"end\"")) => {
                    ended = true;
                    let _ = tx.send(None);
                }
                Some(Ok(Ws::Close(_))) | None | Some(Err(_)) => return,
                _ => {}
            },
            events = erx.recv() => match events {
                Some(Ok(Some(events))) => {
                    for e in events {
                        let v = serde_json::to_value(&e).unwrap_or_default();
                        if socket.send(say(v)).await.is_err() {
                            return;
                        }
                    }
                }
                Some(Ok(None)) => {
                    let _ = socket.send(say(serde_json::json!({ "type": "done" }))).await;
                    return;
                }
                Some(Err(e)) => {
                    let _ = socket.send(say(serde_json::json!({ "type": "error", "message": e }))).await;
                    return;
                }
                None => return,
            },
        }
    }
}

/// The host's own speaking voices, and whether their model is here.
async fn tts_voices(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let downloading = site
        .tts
        .downloading()
        .map(|(got, total)| serde_json::json!({ "got": got, "total": total }));
    Json(serde_json::json!({
        "voices": crate::tts::voices(),
        "default": crate::tts::DEFAULT_VOICE,
        "installed": crate::tts::installed(&site.state),
        "downloading": downloading,
        "local": crate::stt::LOCAL_BUILT,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct TtsSpeak {
    text: String,
    voice: Option<String>,
    speed: Option<f32>,
}

/// A sentence spoken by the host's own voice model, as a WAV file.
async fn tts_speak(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(asked): Json<TtsSpeak>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if asked.text.trim().is_empty() || asked.text.len() > 4000 {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let tts = site.tts.clone();
    let voice = asked
        .voice
        .unwrap_or_else(|| crate::tts::DEFAULT_VOICE.to_owned());
    let speed = asked.speed.unwrap_or(1.0);
    let spoken = tokio::task::spawn_blocking(move || tts.speak(&asked.text, &voice, speed)).await;
    match spoken {
        Ok(Ok((pcm, rate))) => (
            [(header::CONTENT_TYPE, "audio/wav")],
            crate::stt::wav16(&pcm, rate),
        )
            .into_response(),
        Ok(Err(e)) => (StatusCode::SERVICE_UNAVAILABLE, e).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn voices(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if site.voice().is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some((at, list)) = &*site.voices.lock().unwrap()
        && at.elapsed() < VOICES_FRESH
    {
        return Json(list.clone()).into_response();
    }
    let state = site.state.clone();
    match tokio::task::spawn_blocking(move || crate::x::voices(&state)).await {
        Ok(Ok(list)) => {
            *site.voices.lock().unwrap() = Some((std::time::Instant::now(), list.clone()));
            Json(list).into_response()
        }
        Ok(Err(e)) => {
            eprintln!("libri: site voices: {e}");
            (StatusCode::BAD_GATEWAY, e).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// This assistant's A2A Agent Card.
async fn agent_card() -> Response {
    let (Ok(name), Ok(at)) = (std::env::var("LIBRI_NAME"), std::env::var("LIBRI_SITE")) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let person = crate::a2a::own_person();
    Json(crate::a2a::card(&name, &person, &format!("http://{at}"))).into_response()
}

/// The bearer token a peer presents.
fn bearer(headers: &HeaderMap) -> String {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default()
        .to_owned()
}

/// Runs a peers.rs call off the async threads.
async fn on_peers<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| Err(format!("peers: {e}")))
}

/// A message from a paired peer over A2A, with its pair's token: it waits
/// in the inbox for the next tick, which tells the person. The pair names
/// the sender.
async fn a2a_message(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let (state, given) = (site.state.clone(), bearer(&headers));
    let peer = match on_peers(move || crate::peers::caller(&state, &given)).await {
        Ok(Some(p)) => p,
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(e) => {
            eprintln!("libri: a2a: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let Ok(request) = serde_json::from_slice::<Value>(&body) else {
        let parse = serde_json::json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Not JSON."}});
        return Json(parse).into_response();
    };
    let answer = match crate::a2a::take(&request) {
        Ok((_, text)) => {
            match crate::a2a::leave(&crate::a2a::inbox(&site.state), &peer.name, &text) {
                Ok(()) => {
                    let name = std::env::var("LIBRI_NAME").unwrap_or_default();
                    crate::a2a::taken(&request, &name)
                }
                Err(e) => crate::a2a::failed(&request, &e.to_string()),
            }
        }
        Err(e) => e,
    };
    Json(answer).into_response()
}

#[derive(serde::Deserialize)]
struct PairAsked {
    name: String,
    /// Absent from an older Libri; its Agent Card is asked later.
    #[serde(default)]
    person: String,
    url: String,
    token: String,
}

/// Another assistant asks to pair: it waits for this person's approval, and
/// the person hears of it on the next tick.
async fn a2a_pair(State(site): State<Arc<Site>>, Json(r): Json<PairAsked>) -> Response {
    let state = site.state.clone();
    let (name, person, url, token) = (r.name.clone(), r.person, r.url, r.token);
    match on_peers(move || crate::peers::asked(&state, &name, &person, &url, &token)).await {
        Ok(()) => {
            let note = "asks to pair with this assistant, so the two of you can message each other. Approve or decline in Settings > Hosts > Peers.";
            if let Err(e) = crate::a2a::leave(&crate::a2a::inbox(&site.state), &r.name, note) {
                eprintln!("libri: a2a pair: {e}");
            }
            let me = std::env::var("LIBRI_NAME").unwrap_or_default();
            Json(serde_json::json!({ "name": me, "person": crate::a2a::own_person() }))
                .into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct PairApproved {
    name: String,
    /// Absent from an older Libri; its Agent Card is asked later.
    #[serde(default)]
    person: String,
}

/// The other side approved this side's request, with the pair's token.
async fn a2a_approve(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(r): Json<PairApproved>,
) -> Response {
    let (state, given) = (site.state.clone(), bearer(&headers));
    match on_peers(move || crate::peers::approved(&state, &given, &r.name, &r.person)).await {
        Ok(_) => Json(serde_json::json!({})).into_response(),
        Err(_) => StatusCode::UNAUTHORIZED.into_response(),
    }
}

/// The other side revoked the pair whose token it presents.
async fn a2a_unpair(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    let (state, given) = (site.state.clone(), bearer(&headers));
    match on_peers(move || crate::peers::revoked(&state, &given)).await {
        Ok(true) => Json(serde_json::json!({})).into_response(),
        Ok(false) => StatusCode::UNAUTHORIZED.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// Settings: the paired assistants and the requests either way, without
/// their tokens.
async fn peers_listed(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    match on_peers(move || crate::peers::list(&state)).await {
        Ok(all) => Json(
            all.into_iter()
                .map(|p| serde_json::json!({ "name": p.name, "person": p.person, "url": p.url, "state": p.state, "last": p.last }))
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(serde::Deserialize)]
struct PeerAt {
    url: String,
}

/// Settings: asks the assistant at an address to pair.
async fn peer_ask(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(r): Json<PeerAt>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    let asked = on_peers(move || {
        let me =
            std::env::var("LIBRI_NAME").map_err(|_| "This assistant has no name.".to_owned())?;
        let p = crate::peers::ask(&state, &r.url)?;
        let at = crate::peers::own_address(&p.url)?;
        if let Err(e) = crate::a2a::request(&p.url, &me, &crate::a2a::own_person(), &at, &p.token) {
            // Nothing was asked, so nothing waits.
            let _ = crate::peers::revoke(&state, &p.url);
            return Err(e);
        }
        Ok(())
    })
    .await;
    match asked {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// Settings: approves the request from an address, and tells its assistant.
async fn peer_approve(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(r): Json<PeerAt>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    let done = on_peers(move || {
        let me =
            std::env::var("LIBRI_NAME").map_err(|_| "This assistant has no name.".to_owned())?;
        let p = crate::peers::approve(&state, &r.url)?;
        if let Err(e) = crate::a2a::approve(&p.url, &p.token, &me, &crate::a2a::own_person()) {
            // The other side never heard: back to waiting, to try again.
            let _ = crate::peers::revoke(&state, &p.url);
            let _ = crate::peers::asked(&state, &p.name, &p.person, &p.url, &p.token);
            return Err(e);
        }
        Ok(())
    })
    .await;
    match done {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// Settings: revokes a pair, or declines or withdraws a request. It stops
/// here at once; the other side is told, and an unreachable one still can't
/// use the token, as nothing here takes it any more.
async fn peer_revoke(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(r): Json<PeerAt>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    match on_peers(move || crate::peers::revoke(&state, &r.url)).await {
        Ok(p) => {
            tokio::task::spawn_blocking(move || {
                if let Err(e) = crate::a2a::unpair(&p.url, &p.token) {
                    eprintln!("libri: unpair {}: {e}", p.url);
                }
            });
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// How full each subscription window is, and whether the composers show
/// it.
async fn usage_report(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match read_usage().await {
        Ok(r) => Json(serde_json::json!({
            "shown": crate::usage::shown(&site.state),
            "accounts": r.accounts,
            "notes": r.notes,
        }))
        .into_response(),
        Err(e) => e.into_response(),
    }
}

/// The same windows in the flat list clients up to APK 0.9.17 read.
/// The host's state with its recent history, for the System tile.
async fn system_state(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let body = site.system.lock().map(|m| m.report()).unwrap_or_default();
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

async fn usage(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match read_usage().await {
        Ok(r) => Json(crate::usage::limits(&r)).into_response(),
        Err(e) => e.into_response(),
    }
}

/// The usage report; `usage::report` keeps each account's reading for
/// a while, so this is cheap to ask often.
async fn read_usage() -> Result<crate::usage::Report, (StatusCode, String)> {
    tokio::task::spawn_blocking(crate::usage::report)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[derive(Deserialize)]
struct SpendSpan {
    span: Option<crate::ledger::Span>,
}

/// What the model calls over a span cost at API prices, cut by caller,
/// model, company, harness and payment.
async fn usage_spend(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<SpendSpan>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    let span = q.span.unwrap_or(crate::ledger::Span::Week);
    match tokio::task::spawn_blocking(move || crate::ledger::report(&state, span)).await {
        Ok(s) => Json(s).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct UsageShown {
    shown: bool,
}

/// Shows the rings in every composer, or hides them.
/// The point every harness compacts a session at, in tokens, and whether
/// it is the person's own pick.
async fn compact(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(serde_json::json!({
        "tokens": crate::sessions::compact_at(&site.state),
        "picked": crate::sessions::COMPACT_AT.read(&site.state).is_some(),
        "least": crate::sessions::COMPACT_LEAST,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct CompactPick {
    tokens: Option<u64>,
}

/// Sets the compaction point; null goes back to the default.
async fn compact_pick(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(p): Json<CompactPick>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::sessions::compact_pick(&site.state, p.tokens) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

async fn usage_shown(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(s): Json<UsageShown>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::usage::show(&site.state, s.shown) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// What the page needs to start a call: a pass to Grok Voice, the voice,
/// and the instructions with the newest of the memory. Only where the
/// assistant has a voice, so never for one kept off xAI.
async fn call(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(voice) = site.voice() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let h = site.hippo.clone();
    let state = site.state.clone();
    let made = tokio::task::spawn_blocking(move || {
        let view = h.view()?;
        let pass = crate::x::secret(&state, CALL_PASS)?;
        let env = |k| std::env::var(k).unwrap_or_default();
        let instructions = crate::call::instructions(
            &env("LIBRI_PERSON"),
            &env("LIBRI_STYLE"),
            &crate::guide::identity(&state),
            &view,
        );
        Ok::<_, String>(serde_json::json!({
            "secret": pass["value"],
            "voice": voice,
            "instructions": instructions,
        }))
    })
    .await;
    match made {
        Ok(Ok(body)) => Json(body).into_response(),
        Ok(Err(e)) => {
            eprintln!("libri: site call: {e}");
            (StatusCode::BAD_GATEWAY, e).into_response()
        }
        Err(e) => {
            eprintln!("libri: site call: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// A call with the chat itself. The page sends the microphone, 16 kHz
/// mono PCM in 20 ms frames, and `{"type":"barge"}` when the person talks
/// over the voice; the host finds each utterance, writes it down, and hands
/// it to the main chat as the person's message marked "(call)", so the
/// chat's own model, memory and tools answer. The chat's reply comes back
/// sentence by sentence: `{"type":"say","seq","text"}`, then that
/// sentence's audio as one binary message. Also `heard` (the person's
/// words as written), `speech` (the person began speaking), `done` (the
/// turn ended) and `error`.
async fn call_socket(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !site.admits(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !site.open
        && site
            .device(&headers)
            .is_some_and(|d| d.scope == crate::host::Scope::Read)
    {
        return (StatusCode::FORBIDDEN, "This device may only read.").into_response();
    }
    let Some(engine) = crate::speech::Engine::pick(&site.state, site.voice(), &site.tts) else {
        return (StatusCode::NOT_FOUND, "No voice: connect xAI in Settings.").into_response();
    };
    let kind = crate::origin::of(&headers);
    ws.on_upgrade(move |socket| serve_call(site, socket, engine, kind))
}

#[derive(Deserialize)]
struct CallAsk {
    text: String,
}

/// A request the call's voice hands to the chat: on a call with Grok Voice
/// in front, it talks and the chat works. The request goes into the main
/// chat as the person's message marked "(call)"; the answer is the chat's
/// reply to it, as written, once its turn ends.
async fn call_ask(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(ask): Json<CallAsk>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !site.open
        && site
            .device(&headers)
            .is_some_and(|d| d.scope == crate::host::Scope::Read)
    {
        return (StatusCode::FORBIDDEN, "This device may only read.").into_response();
    }
    let text = ask.text.trim();
    if text.is_empty() || text.len() > SEND_MAX {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let mut shown = site.web.shown.subscribe();
    let m = InMessage {
        id: format!("call-{}", unix_ms()),
        role: "user".into(),
        content: Content::Text(format!("(call) {text}")),
    };
    let status = take_message(&site, m, Forwarded::default(), crate::origin::of(&headers)).await;
    if status != StatusCode::ACCEPTED {
        return status.into_response();
    }
    // The reply as the chat writes it: streamed words, or the whole said at
    // once by a harness that doesn't stream; done when the turn ends.
    let reply = async {
        // A turn already under way takes the request as a steer, and its
        // end is the answer's end too.
        let (mut streamed, mut said) = (String::new(), String::new());
        loop {
            match shown.recv().await {
                Ok(Shown::Step {
                    step: Step::Text { text },
                    agent: None,
                }) => streamed.push_str(&text),
                Ok(Shown::Said {
                    text, aside: false, ..
                }) => {
                    if !said.is_empty() {
                        said.push_str("\n\n");
                    }
                    said.push_str(&text);
                }
                Ok(Shown::Typing { on: false }) => break,
                Err(broadcast::error::RecvError::Closed) => break,
                _ => {}
            }
        }
        if said.trim().is_empty() {
            streamed
        } else {
            said
        }
    };
    match tokio::time::timeout(Duration::from_secs(600), reply).await {
        Ok(text) => Json(serde_json::json!({ "text": text })).into_response(),
        Err(_) => Json(serde_json::json!({ "text": "", "still": true })).into_response(),
    }
}

/// One sentence for the voice: the turn of speech it belongs to, which a
/// barge ends, and its place among that turn's sentences.
struct Sentence {
    turn: u64,
    seq: u64,
    text: String,
}

/// The quiet that ends a call's utterance: shorter than dictation's, so
/// the answer starts soon, long enough for a breath between clauses.
const CALL_SILENCE_MS: u32 = 500;

async fn serve_call(
    site: Arc<Site>,
    mut ws: axum::extract::ws::WebSocket,
    engine: crate::speech::Engine,
    kind: &'static str,
) {
    use axum::extract::ws::Message as Ws;
    let send = |v: Value| Ws::Text(v.to_string().into());
    // Subscribed before anything is said, so no word of the reply is missed.
    let mut shown = site.web.shown.subscribe();
    // The voice speaks one sentence at a time, in order, off this task;
    // a barge moves `turn` on and what is queued for the old one is dropped.
    let turn = Arc::new(AtomicU64::new(0));
    {
        let engine = engine.clone();
        tokio::task::spawn_blocking(move || engine.warm());
    }
    let (to_voice, mut sentences) = mpsc::unbounded_channel::<Sentence>();
    let (spoken_tx, mut spoken) = mpsc::unbounded_channel::<(u64, String, Vec<u8>)>();
    {
        let (engine, turn) = (engine.clone(), turn.clone());
        tokio::spawn(async move {
            while let Some(s) = sentences.recv().await {
                if s.turn != turn.load(Ordering::SeqCst) {
                    continue;
                }
                let (engine, text) = (engine.clone(), s.text.clone());
                let made = tokio::task::spawn_blocking(move || engine.speak(&text)).await;
                match made {
                    Ok(Ok(audio)) if s.turn == turn.load(Ordering::SeqCst) => {
                        if spoken_tx.send((s.seq, s.text, audio)).is_err() {
                            return;
                        }
                    }
                    Ok(Err(e)) => eprintln!("libri: call voice: {e}"),
                    _ => {}
                }
            }
        });
    }
    // The ear: the host's speech-to-text, on its own thread so a slow
    // decode never holds the socket. Its voice detector ends each
    // utterance; a streaming model has the words written by then.
    let options = crate::stt::Options {
        endpoint: crate::stt::Endpoint {
            trailing_silence_ms: CALL_SILENCE_MS,
            ..Default::default()
        },
        ..Default::default()
    };
    let opened = {
        let stt = site.stt.clone();
        tokio::task::spawn_blocking(move || stt.open(options)).await
    };
    let mut session = match opened {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            let _ = ws
                .send(send(serde_json::json!({"type": "error", "text": e})))
                .await;
            return;
        }
        Err(_) => return,
    };
    // Each run of samples goes with when it came, and its events come back
    // with that time and the session's length then, so an utterance's end
    // in session seconds maps to the moment the person stopped: the start
    // of the wait for the answer.
    let (to_ear, mut audio) = mpsc::unbounded_channel::<(Vec<i16>, Instant)>();
    let (heard_tx, mut heard) =
        mpsc::unbounded_channel::<(Result<Vec<crate::stt::Event>, String>, Instant, f32)>();
    std::thread::spawn(move || {
        let mut samples = 0usize;
        // A failed utterance (a cloud service's refusal) loses only itself.
        while let Some((pcm, came)) = audio.blocking_recv() {
            samples += pcm.len();
            let heard = (
                session.push(&pcm),
                came,
                samples as f32 / crate::stt::RATE as f32,
            );
            if heard_tx.send(heard).is_err() {
                return;
            }
        }
    });
    // When the person stopped, until the answer's first sentence is out,
    // and whether the chat's first sentence was timed yet: one log line per
    // stage of the wait, which is what a call's speed comes down to.
    let mut waiting: Option<Instant> = None;
    let mut written = false;
    let timed = |waiting: Option<Instant>, written: &mut bool| {
        if let Some(w) = waiting.filter(|_| !*written) {
            *written = true;
            eprintln!(
                "libri: call: the chat's first sentence {} ms after the speech ended",
                w.elapsed().as_millis()
            );
        }
    };
    let mut words = crate::speech::Speakable::default();
    let mut seq = 0u64;
    // Whether the turn under way said anything yet, and when the voice last
    // spoke, so a tool's progress line is said only into a silence.
    let mut streamed = false;
    let mut last_said = Instant::now();
    let queue = |seq: &mut u64, text: String, last_said: &mut Instant| {
        *seq += 1;
        *last_said = Instant::now();
        let _ = to_voice.send(Sentence {
            turn: turn.load(Ordering::SeqCst),
            seq: *seq,
            text,
        });
    };
    if ws
        .send(send(serde_json::json!({"type": "ready"})))
        .await
        .is_err()
    {
        return;
    }
    loop {
        tokio::select! {
            m = ws.recv() => match m {
                Some(Ok(Ws::Binary(pcm))) => {
                    let pcm = pcm.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
                    if to_ear.send((pcm, Instant::now())).is_err() {
                        break;
                    }
                }
                Some(Ok(Ws::Text(t))) => {
                    let v: Value = serde_json::from_str(&t).unwrap_or_default();
                    if v["type"] == "barge" {
                        // What the voice was saying is cut; the person's words
                        // that follow steer the turn as any message does.
                        turn.fetch_add(1, Ordering::SeqCst);
                        words = crate::speech::Speakable::default();
                    }
                }
                Some(Ok(Ws::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            h = heard.recv() => match h {
                Some((Ok(events), came, heard_to)) => for e in events {
                    let (text, end) = match e {
                        crate::stt::Event::SpeechStarted { .. } => {
                            let _ = ws.send(send(serde_json::json!({"type": "speech"}))).await;
                            continue;
                        }
                        crate::stt::Event::Partial { text, .. } => {
                            let _ = ws.send(send(serde_json::json!({"type": "partial", "text": text}))).await;
                            continue;
                        }
                        crate::stt::Event::Final { text, end, .. } => (text, end),
                    };
                    let text = text.trim().to_owned();
                    if text.is_empty() {
                        continue;
                    }
                    let stopped = came
                        .checked_sub(Duration::from_secs_f32((heard_to - end).max(0.0)))
                        .unwrap_or(came);
                    eprintln!(
                        "libri: call: words {} ms after the speech ended",
                        stopped.elapsed().as_millis()
                    );
                    waiting = Some(stopped);
                    written = false;
                    let _ = ws.send(send(serde_json::json!({"type": "heard", "text": text}))).await;
                    let m = InMessage {
                        id: format!("call-{}", unix_ms()),
                        role: "user".into(),
                        content: Content::Text(format!("(call) {text}")),
                    };
                    let status = take_message(&site, m, Forwarded::default(), kind).await;
                    if status != StatusCode::ACCEPTED {
                        let _ = ws.send(send(serde_json::json!({"type": "error", "text": "The chat didn't take that."}))).await;
                    }
                }
                Some((Err(e), _, _)) => {
                    eprintln!("libri: call ear: {e}");
                    let _ = ws.send(send(serde_json::json!({"type": "error", "text": "Couldn't make that out."}))).await;
                }
                None => break,
            },
            s = shown.recv() => match s {
                Ok(Shown::Typing { on: true }) => streamed = false,
                Ok(Shown::Step { step: Step::Text { text }, agent: None }) => {
                    streamed = true;
                    for sentence in words.push(&text) {
                        timed(waiting, &mut written);
                        queue(&mut seq, sentence, &mut last_said);
                    }
                }
                Ok(Shown::Step { step: Step::Call { name, .. }, agent: None }) => {
                    if last_said.elapsed() > Duration::from_secs(6) {
                        queue(&mut seq, crate::speech::progress(&name).to_owned(), &mut last_said);
                    }
                }
                Ok(Shown::Said { text, .. }) if !streamed => {
                    for sentence in words.push(&text).into_iter().chain(words.finish()) {
                        timed(waiting, &mut written);
                        queue(&mut seq, sentence, &mut last_said);
                    }
                }
                Ok(Shown::Typing { on: false }) => {
                    for sentence in words.finish() {
                        timed(waiting, &mut written);
                        queue(&mut seq, sentence, &mut last_said);
                    }
                    let _ = ws.send(send(serde_json::json!({"type": "done"}))).await;
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
            v = spoken.recv() => match v {
                Some((seq, text, audio)) => {
                    if let Some(w) = waiting.take() {
                        eprintln!("libri: call: first audio {} ms after the speech ended", w.elapsed().as_millis());
                    }
                    let said = serde_json::json!({"type": "say", "seq": seq, "text": text});
                    if ws.send(send(said)).await.is_err()
                        || ws.send(Ws::Binary(audio.into())).await.is_err()
                    {
                        break;
                    }
                }
                None => break,
            },
        }
    }
}

#[derive(Deserialize)]
struct CallEnd {
    lines: Vec<crate::call::Line>,
    minutes: u64,
}

/// A finished call into hippo, so the chat and the memory hold it.
async fn call_end(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(CallEnd { lines, minutes }): Json<CallEnd>,
) -> StatusCode {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED;
    }
    let h = site.hippo.clone();
    let logged = tokio::task::spawn_blocking(move || {
        crate::call::record(lines, minutes)
            .into_iter()
            .try_for_each(|(kind, text)| h.save(kind, &text).map(drop))
    })
    .await;
    match logged {
        Ok(Ok(())) => StatusCode::NO_CONTENT,
        Ok(Err(e)) => {
            eprintln!("libri: site call end: {e}");
            StatusCode::BAD_GATEWAY
        }
        Err(e) => {
            eprintln!("libri: site call end: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

/// Names a spoken text in the cache. std's SipHash keys are fixed, so the
/// name holds across restarts; a Rust release that changed it would only
/// cost a miss.
fn speech_key(voice: &str, text: &str) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    (voice, text).hash(&mut h);
    h.finish()
}

/// Drops the files in `dir` older than `keep`.
fn prune(dir: &Path, keep: Duration) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > keep));
        if old {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// Markdown as it should be heard: code blocks, images, link targets,
/// table rules and the marks themselves left out.
pub(crate) fn plain(md: &str) -> String {
    let mut out = Vec::new();
    let mut code = false;
    for line in md.lines() {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            code = !code;
            continue;
        }
        if code || t.is_empty() {
            continue;
        }
        // A table's rule row says nothing; its other rows read as a list.
        if t.starts_with('|') && t.chars().all(|c| "|-: ".contains(c)) {
            continue;
        }
        let t = t.trim_start_matches(['#', '>']).trim_start();
        let t = ["- ", "* ", "+ "]
            .iter()
            .find_map(|m| t.strip_prefix(m))
            .unwrap_or(t);
        let t = if t.starts_with('|') {
            t.trim_matches('|')
                .split('|')
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            t.to_owned()
        };
        let t = inline(&t);
        if !t.is_empty() {
            out.push(t);
        }
    }
    out.join("\n")
}

/// `inline` markdown as plain words: links keep their text, images go.
fn inline(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('[') {
        let image = rest[..i].ends_with('!');
        out.push_str(&rest[..if image { i - 1 } else { i }]);
        let after = &rest[i + 1..];
        match after
            .find("](")
            .and_then(|j| after[j + 2..].find(')').map(|k| (j, j + 2 + k + 1)))
        {
            Some((j, end)) => {
                if !image {
                    out.push_str(&after[..j]);
                }
                rest = &after[end..];
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "")
        .replace("__", "")
        .replace("~~", "")
        .replace(['`', '*'], "")
        .trim()
        .to_owned()
}

/// A message of the history, with the emoji Libri answered it by.
#[derive(Serialize)]
struct Line {
    #[serde(flatten)]
    said: Said,
    reaction: Option<String>,
    /// The earlier message the person's replies to.
    #[serde(skip_serializing_if = "Option::is_none")]
    quote: Option<crate::Quote>,
    /// The kind of device the person's message came from (origin.rs): the
    /// last mark at or before it, as the mark comes only on a change.
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<&'static str>,
}

/// The conversation for the page. hippo keeps a lone-emoji answer as a
/// message, but it went out as a reaction, so it goes back on the person's
/// message it answered. An edited or asked-again message replaces the
/// person's last one and all that followed it.
fn lines(all: Vec<Said>) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    let mut device = None;
    for mut said in all {
        if said.kind == "user"
            && let Some(text) = crate::redone(&said.text)
        {
            said.text = text.to_owned();
            if let Some(last) = out.iter().rposition(|l| l.said.kind == "user") {
                out.truncate(last);
            }
        }
        let mut quote = None;
        if said.kind == "user"
            && let (Some(q), words) = crate::Quote::read(&said.text)
        {
            said.text = words.to_owned();
            quote = Some(q);
        }
        let mut from = None;
        if said.kind == "user" {
            if let (Some(kind), words) = crate::origin::read(&said.text) {
                said.text = words.to_owned();
                device = Some(kind);
            }
            from = device;
        }
        match said.kind.as_str() {
            "talk" if crate::is_reaction(&said.text) => {
                if let Some(last) = out
                    .last_mut()
                    .filter(|l| l.said.kind == "user" && l.reaction.is_none())
                {
                    last.reaction = Some(said.text);
                    continue;
                }
            }
            "user" | "talk" | "note" => {}
            // A seat's work, as hippo read it from the session: the page
            // draws each call with its result, among the words, and a
            // subagent's report in its Agent call's frame.
            "tool" | "echo" | "work" => {
                if said.text.len() > HISTORY_STEP {
                    let mut cut = HISTORY_STEP;
                    while !said.text.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    said.text.truncate(cut);
                    said.text.push('…');
                }
            }
            _ => continue,
        }
        out.push(Line {
            said,
            reaction: None,
            quote,
            from,
        });
    }
    out
}

/// Where a client following /api/events left off: `after`, the id of the
/// last event it took. One that names an event gets `replay`, the events
/// after it, or `sync` when it must read the chat whole; one with nothing to
/// name (`after=` empty) reads the chat on its own as the stream opens. Both
/// learn where they stand from each event's id and each ping's, and one
/// that falls behind is caught up on the same stream. A client that sends no
/// `after` at all is from before the log: it reads the chat on every
/// opening and is cut off when it falls behind.
#[derive(Deserialize)]
struct Following {
    after: Option<String>,
}

async fn events(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Query(f): axum::extract::Query<Following>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    // The browser's own reconnect names the last event it took in a
    // header, which is newer than the address the page opened.
    let last = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .or(f.after);
    let log = site.web.wire.clone();
    // Subscribed before the log is read: what comes between is heard
    // twice, never missed, and dropped by its number.
    let rx = log.subscribe();
    let mut follow = Follow {
        rx,
        quiet: tokio::time::interval(PULSE),
        site,
        log,
        queue: Default::default(),
        resumes: last.is_some(),
        floor: 0,
        sent: 0,
    };
    // Whatever comes after this is heard live, so a client stands here at
    // least.
    follow.sent = follow.log.newest();
    follow
        .quiet
        .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    if let Some(id) = last.filter(|id| !id.is_empty()) {
        let resume = match follow.log.parse(&id) {
            Some(after) => follow.log.since(after),
            None => Missed::Snapshot(follow.log.newest()),
        };
        follow.catch_up(resume);
    }
    let stream = futures_util::stream::unfold(follow, |mut f| async move {
        let e = f.next().await?;
        Some((Ok::<_, Infallible>(e), f))
    });
    Sse::new(stream).into_response()
}

/// One client following the page stream.
struct Follow {
    rx: broadcast::Receiver<Logged>,
    quiet: tokio::time::Interval,
    site: Arc<Site>,
    log: Arc<EventLog>,
    /// What goes out before the next live event.
    queue: std::collections::VecDeque<Event>,
    /// Whether the client named where it left off, and so takes `replay`
    /// and `sync`.
    resumes: bool,
    /// Live events up to this number went out in a replay already.
    floor: u64,
    /// The number of the newest event the client has, or read past: every
    /// event after it is still to go out to it.
    sent: u64,
}

impl Follow {
    /// Sends the client what it lacks: the events, in one `replay` so the
    /// page draws them at once, or `sync`, to read the chat whole.
    fn catch_up(&mut self, missed: Missed) {
        let e = match missed {
            Missed::Replay(es) => {
                let mut data = String::from("[");
                for (i, (_, text)) in es.iter().enumerate() {
                    if i > 0 {
                        data.push(',');
                    }
                    data.push_str(text);
                }
                data.push(']');
                let e = Event::default().event("replay").data(data);
                match es.last() {
                    Some(&(seq, _)) => {
                        self.floor = seq;
                        self.sent = seq;
                        e.id(self.log.id(seq))
                    }
                    None => e,
                }
            }
            // The live events from here on go out whole, as before the log:
            // the client's read and they overlap, and it drops what its
            // read holds by the turn record's numbers.
            Missed::Snapshot(newest) => {
                self.sent = self.sent.max(newest);
                Event::default().event("sync").data("{}")
            }
        };
        self.queue.push_back(e);
    }

    async fn next(&mut self) -> Option<Event> {
        loop {
            if let Some(e) = self.queue.pop_front() {
                return Some(e);
            }
            tokio::select! {
                r = self.rx.recv() => match r {
                    Ok((seq, text)) => {
                        if seq <= self.floor {
                            continue;
                        }
                        self.quiet.reset();
                        self.sent = self.sent.max(seq);
                        return Some(Event::default().id(self.log.id(seq)).data(&*text));
                    }
                    // A client too slow to keep up gets what it missed from
                    // the log; one from before the log is cut off, and its
                    // reconnect reads the chat.
                    Err(broadcast::error::RecvError::Lagged(_)) if self.resumes => {
                        let resume = self.log.since(self.sent);
                        self.catch_up(resume);
                    }
                    Err(_) => return None,
                },
                _ = self.quiet.tick() => {
                    let ping = pulse(&self.site.web);
                    // Where the client stands, for one that has heard no
                    // event since it read the chat.
                    return Some(if self.resumes { ping.id(self.log.id(self.sent)) } else { ping });
                }
            }
        }
    }
}

/// What a quiet stream says each PULSE: an event, not a comment, since
/// EventSource hides comments and a page or app must hear the silence of a
/// stream cut on the way to it. It carries the host's clock, so a client
/// times what runs by the host's and a timer reads the same after a
/// reconnect as before it, and the number of the turn record's newest
/// event, so a client that heard less knows it missed some and syncs.
fn pulse(web: &Web) -> Event {
    Event::default()
        .event("ping")
        .data(pulse_data(web).to_string())
}

fn pulse_data(web: &Web) -> Value {
    let seq = web.turn.lock().unwrap().seq;
    serde_json::json!({ "now": unix_ms(), "seq": seq })
}

/// A file sent ahead of its message, a form with one `file` and maybe
/// its `view`, the page's JPEG of a photo too big or of a kind the model
/// can't see. Answers the id an AG-UI binary part names it by.
async fn upload(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    mut form: Multipart,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut file = None;
    loop {
        let field = match form.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return (e.status(), e.body_text()).into_response(),
        };
        match (field.name(), &mut file) {
            (Some("file"), None) => {
                let name = field.file_name().unwrap_or("file").to_owned();
                let mime = field.content_type().map(str::to_owned);
                match field.bytes().await {
                    Ok(b) => {
                        file = Some(Upload {
                            name,
                            mime,
                            data: b.to_vec(),
                            view: None,
                        })
                    }
                    Err(e) => return (e.status(), e.body_text()).into_response(),
                }
            }
            (Some("view"), Some(f)) => match field.bytes().await {
                Ok(b) => f.view = Some(b.to_vec()),
                Err(e) => return (e.status(), e.body_text()).into_response(),
            },
            _ => return StatusCode::BAD_REQUEST.into_response(),
        }
    }
    let (Some(file), Ok(id)) = (file, token()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let mut uploads = site.uploads.lock().unwrap();
    uploads.retain(|_, (made, _)| made.elapsed() < ASK);
    uploads.insert(id.clone(), (Instant::now(), file));
    Json(serde_json::json!({ "id": id })).into_response()
}

/// AG-UI's RunAgentInput, as far as the server reads it: the history is
/// hippo's, so `messages` brings only the person's new one; `state`,
/// `tools` and `context` are the server's own.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunInput {
    #[serde(default)]
    messages: Vec<InMessage>,
    #[serde(default)]
    resume: Vec<Resume>,
    #[serde(default)]
    forwarded_props: Forwarded,
}

#[derive(Deserialize)]
struct InMessage {
    id: String,
    role: String,
    #[serde(default)]
    content: Content,
}

/// A user message's content: words, or AG-UI input parts.
#[derive(Deserialize)]
#[serde(untagged)]
enum Content {
    Text(String),
    Parts(Vec<InPart>),
}

impl Default for Content {
    fn default() -> Self {
        Content::Text(String::new())
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum InPart {
    Text {
        text: String,
    },
    /// A file sent ahead to /api/upload, by the id it answered.
    Binary {
        id: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Resume {
    interrupt_id: String,
    status: String,
    #[serde(default)]
    payload: Value,
}

/// What Libri takes beside the protocol's fields.
#[derive(Deserialize, Default)]
struct Forwarded {
    /// "edit" or "again": the message takes the place of the person's last.
    #[serde(default)]
    redo: Option<String>,
    /// The earlier message this one replies to.
    #[serde(default)]
    quote: Option<crate::Quote>,
}

/// A run, as AG-UI starts one: answers to the open interrupts (`resume`),
/// the person's new message, or both. Its events reach every page over
/// /api/events. 202 once taken; 409 when a resume names no open interrupt.
async fn run(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(input): Json<RunInput>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    // A resume answered already is not news: answering is idempotent.
    let mut stale = 0;
    for r in &input.resume {
        let answer = {
            let asks = site.web.asks.lock().unwrap();
            asks.iter()
                .find(|(k, _)| k.id == r.interrupt_id)
                .map(|(ask, _)| {
                    let answers = r.payload["answers"].as_object().cloned();
                    let yes = r.status == "resolved";
                    // "Always allow" on a connection's ask: its relay sets
                    // the tool to Allow (connections.rs).
                    if yes && ask.always && r.payload["always"] == Value::Bool(true) {
                        Some(serde_json::json!({"always": true}))
                    } else {
                        answered(&ask.input, yes, answers)
                    }
                })
        };
        match answer {
            Some(a) => {
                site.web.resolve(&r.interrupt_id, a);
            }
            None => stale += 1,
        }
    }
    let Some(m) = input.messages.into_iter().rev().find(|m| m.role == "user") else {
        return if stale > 0 && stale == input.resume.len() {
            StatusCode::CONFLICT.into_response()
        } else {
            StatusCode::ACCEPTED.into_response()
        };
    };
    if m.id.is_empty() || m.id.len() > 64 {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Claimed before anything is taken from it, uploads included. A copy
    // sent while the first is on its way waits for it, and runs if it failed.
    // A request its client dropped drops its claim too.
    let Some(claim) = site.taken.claim(&m.id).await else {
        return StatusCode::ACCEPTED.into_response();
    };
    let kind = crate::origin::of(&headers);
    let status = take_message(&site, m, input.forwarded_props, kind).await;
    if status == StatusCode::ACCEPTED {
        claim.accept();
    }
    status.into_response()
}

/// The person's message claimed by `run`, taken: ACCEPTED once it is this
/// daemon's to lose.
async fn take_message(
    site: &Arc<Site>,
    m: InMessage,
    forwarded: Forwarded,
    kind: &str,
) -> StatusCode {
    let (mut text, mut files) = (String::new(), Vec::new());
    match m.content {
        Content::Text(t) => text = t,
        Content::Parts(parts) => {
            for p in parts {
                match p {
                    InPart::Text { text: t } => text.push_str(&t),
                    InPart::Binary { id } => match site.uploads.lock().unwrap().remove(&id) {
                        Some((_, f)) => files.push(f),
                        None => return StatusCode::BAD_REQUEST,
                    },
                }
            }
        }
    }
    let redo = match forwarded.redo.as_deref() {
        None => "",
        Some("edit") => crate::EDITED,
        Some("again") => crate::AGAIN,
        Some(_) => return StatusCode::BAD_REQUEST,
    };
    let text = text.trim();
    if (text.is_empty() && files.is_empty()) || text.len() > SEND_MAX || files.len() > FILES_MAX {
        return StatusCode::BAD_REQUEST;
    }
    site.web.sent(&m.id, text, !redo.is_empty());
    // An answer reaches the turn as the question's answer; sent on as well,
    // the turn would hear it twice. Files with it still go on.
    let answered = redo.is_empty() && !text.eq_ignore_ascii_case("stop") && site.web.typed(text);
    if answered {
        let (hippo, said) = (site.hippo.clone(), text.to_owned());
        let (web, id) = (site.web.clone(), m.id.clone());
        tokio::task::spawn_blocking(move || match hippo.save_row("user", &said) {
            Ok((key, row)) => web.stored(&key, &[id], row),
            Err(e) => eprintln!("libri: {e}"),
        });
    }
    if answered && files.is_empty() {
        return StatusCode::ACCEPTED;
    }
    // The device's mark goes inside the quote's, which reads only at the
    // start, and is written only when the device changed (origin.rs).
    let from = crate::origin::mark(&site.state, "main", kind);
    let worded = format!("{from}{text}");
    let marked = match forwarded.quote {
        Some(q) => q.tidied().mark(&worded),
        None => worded,
    };
    let msg = Message {
        id: m.id,
        text: format!("{redo}{marked}"),
        files,
    };
    // Once it says 202 the words are this daemon's to lose. Stop has no words
    // to keep and must not wait for a disk.
    if !text.eq_ignore_ascii_case("stop") || !msg.files.is_empty() {
        let (state, kept) = (site.state.clone(), msg.id.clone());
        let (said, count) = (msg.text.clone(), msg.files.len());
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let written = tokio::task::spawn_blocking(move || {
            crate::journal::keep(&state, &kept, &said, count, at)
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        if let Err(e) = written {
            eprintln!("libri: {e}");
            return StatusCode::SERVICE_UNAVAILABLE;
        }
    }
    if site.messages.send((site.web.clone(), msg)).is_err() {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    StatusCode::ACCEPTED
}

/// The acts waiting for the person's word, oldest first.
async fn waiting(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match pending::all(&site.state) {
        Ok(all) => Json(all).into_response(),
        Err(e) => {
            eprintln!("libri: site pending: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// A question's input answered in the person's own words, or None for any
/// other ask, which they refuse.
fn typed_answer(ask: &SeatAsk, text: &str) -> Option<Value> {
    if ask.tool != "AskUserQuestion" {
        return None;
    }
    let answers = ask.input["questions"]
        .as_array()?
        .iter()
        .filter_map(|q| q["question"].as_str())
        .map(|q| (q.to_owned(), Value::String(text.to_owned())))
        .collect();
    answered(&ask.input, true, Some(answers))
}

/// The input a tool runs with once answered, or None when refused.
fn answered(
    input: &Value,
    yes: bool,
    answers: Option<serde_json::Map<String, Value>>,
) -> Option<Value> {
    let mut input = input.clone();
    if let (Some(answers), Some(o)) = (answers, input.as_object_mut()) {
        o.insert("answers".into(), Value::Object(answers));
    }
    yes.then_some(input)
}

#[derive(Deserialize)]
struct Verdict {
    id: u64,
    yes: bool,
}

/// The person's verdict on an act, as a 👍 or 👎
/// it: 202 while it is carried out (the page hears how over SSE), 404 when
/// the act is gone, decided already.
async fn decide(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(v): Json<Verdict>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let known = pending::all(&site.state).is_ok_and(|all| all.iter().any(|p| p.id == v.id));
    if !known {
        return StatusCode::NOT_FOUND.into_response();
    }
    if site.verdicts.send((v.id, v.yes)).is_err() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    StatusCode::ACCEPTED.into_response()
}

/// Stops the turn under way, from the page; it answers
/// "(Stopped.)". With no turn running it does nothing.
/// How long a list of the models on offer is kept.
const OFFERS: Duration = Duration::from_secs(600);

/// The models on offer, asked of the harnesses when the kept list is old.
async fn offers(site: &Arc<Site>) -> Vec<crate::catalog::Offer> {
    if let Some((at, o)) = &*site.offers.lock().unwrap()
        && at.elapsed() < OFFERS
    {
        return o.clone();
    }
    let state = site.state.clone();
    let o = tokio::task::spawn_blocking(move || {
        let o = crate::catalog::offers(&state);
        if !o.is_empty() {
            crate::catalog::remember(&state, &o);
        }
        o
    })
    .await
    .unwrap_or_default();
    *site.offers.lock().unwrap() = Some((Instant::now(), o.clone()));
    o
}

/// The harnesses switched on but not signed in, asked of each when the kept
/// list is old.
async fn signed_out(site: &Arc<Site>) -> Vec<String> {
    if let Some((at, o)) = &*site.signed_out.lock().unwrap()
        && at.elapsed() < OFFERS
    {
        return o.clone();
    }
    let o = tokio::task::spawn_blocking(crate::catalog::signed_out)
        .await
        .unwrap_or_default();
    *site.signed_out.lock().unwrap() = Some((Instant::now(), o.clone()));
    o
}

/// The model the next turn runs on: the person's pick, else the default
/// harness's own default model.
fn current(site: &Site, offers: &[crate::catalog::Offer]) -> crate::catalog::Pick {
    crate::catalog::picked(&site.state).unwrap_or_else(|| {
        let harness = crate::catalog::harness_now();
        let model = offers
            .iter()
            .find(|o| o.harness == harness && o.default)
            .map(|o| o.model.clone())
            .unwrap_or_default();
        crate::catalog::Pick { harness, model }
    })
}

/// Temp's own pick while its model is still on offer. None has it follow
/// the main chat.
fn temp_current(site: &Site, offers: &[crate::catalog::Offer]) -> Option<crate::catalog::Pick> {
    crate::catalog::temp_picked(&site.state).filter(|p| {
        offers
            .iter()
            .any(|o| o.harness == p.harness && o.model == p.model)
    })
}

/// The models on offer, the one the next turn runs on, its effort, Temp's
/// pick and effort, and which harness runs the other makers' models.
async fn models(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let offers = offers(&site).await;
    let pick = current(&site, &offers);
    let effort = crate::model::EFFORT
        .read(&site.state)
        .or_else(|| std::env::var("LIBRI_EFFORT").ok())
        .unwrap_or_default();
    let own = temp_current(&site, &offers);
    let temp_harness = own.as_ref().unwrap_or(&pick).harness.clone();
    let temp_grant = crate::access::grant(
        &temp_harness,
        &site.state,
        &crate::access::temp_picks(&site.state, &temp_harness),
    );
    let temp = match own {
        Some(p) => serde_json::json!({
            "pick": p,
            "effort": crate::catalog::TEMP_EFFORT.read(&site.state).unwrap_or_default(),
            "access": temp_grant.mode,
            "sandbox": temp_grant.sandbox,
        }),
        None => serde_json::json!({
            "pick": pick,
            "effort": effort,
            "access": temp_grant.mode,
            "sandbox": temp_grant.sandbox,
        }),
    };
    let held = crate::catalog::main_choice(&site.state).pick.harness;
    let main = crate::access::grant(
        &held,
        &site.state,
        &crate::access::main_picks(&site.state, &held),
    );
    let mut harnesses: Vec<&str> = vec![held.as_str()];
    for o in &offers {
        if !harnesses.contains(&o.harness.as_str()) {
            harnesses.push(&o.harness);
        }
    }
    let modes: Vec<Value> = harnesses
        .into_iter()
        .map(|h| {
            let default = crate::access::grant(h, &site.state, &crate::access::Picks::default());
            serde_json::json!({
                "harness": h,
                "access": crate::access::access_modes(h, &site.state),
                "sandbox": crate::access::sandbox_modes(h),
                "defaultAccess": default.mode,
                "defaultSandbox": default.sandbox,
            })
        })
        .collect();
    Json(serde_json::json!({
        "pick": pick,
        "effort": effort,
        "temp": temp,
        "access": main.mode,
        "sandbox": main.sandbox,
        "modes": modes,
        "others": crate::catalog::others(&site.state),
        "othersOffered": crate::catalog::others_offered(),
        "signedOut": signed_out(&site).await,
        "harnesses": harness_settings(&site.state),
        "defaultHarness": crate::catalog::default_harness(),
        "offers": offers,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct ModelPick {
    #[serde(flatten)]
    pick: crate::catalog::Pick,
    /// Temp's pick rather than the main chat's.
    #[serde(default)]
    temp: bool,
}

/// Every setting, where it applies and its value on this host
/// (settings.rs).
/// The settings and config keys as Markdown, the page the repository keeps
/// (settings.rs `markdown`): the same on every host, so open like the app.
async fn settings_page() -> Response {
    (
        [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        crate::settings::markdown(),
    )
        .into_response()
}

async fn settings_listed(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(crate::settings::listed(&site.state)).into_response()
}

/// Switches the model, and with it the harness, from the next turn on;
/// the one under way keeps its. An effort the new model doesn't take goes
/// back to its harness's default.
async fn model_pick(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(ModelPick { pick: p, temp }): Json<ModelPick>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let offers = offers(&site).await;
    let Some(offer) = offers
        .iter()
        .find(|o| o.harness == p.harness && o.model == p.model)
    else {
        return (StatusCode::BAD_REQUEST, "That model is not on offer.").into_response();
    };
    let (kept, effort) = if temp {
        (
            crate::catalog::temp_pick(&site.state, &p),
            &crate::catalog::TEMP_EFFORT,
        )
    } else {
        (crate::catalog::pick(&site.state, &p), &crate::model::EFFORT)
    };
    if let Err(e) = kept {
        return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
    }
    if effort
        .read(&site.state)
        .is_some_and(|e| !offer.efforts.contains(&e))
    {
        effort.clear(&site.state);
    }
    // A Claude turn under way takes the model at its next request.
    if !temp && p.harness == "claude" {
        crate::model::switch("main", Some(&p.model), None);
    }
    StatusCode::NO_CONTENT.into_response()
}

/// What makes what here besides answers, and by which model: pictures
/// among the models this host reaches, the rest fixed by the one service
/// that does them.
async fn capabilities(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    let Ok((choices, picked)) = tokio::task::spawn_blocking(move || {
        let choices = crate::capability::image_choices(&state);
        let picked = crate::capability::image(&state)
            .filter(|p| {
                choices
                    .iter()
                    .any(|c| c.provider == p.provider && c.model == p.model)
            })
            .or_else(|| {
                choices
                    .iter()
                    .find(|c| c.model == crate::x::IMAGE_MODEL)
                    .cloned()
            })
            .or_else(|| choices.first().cloned());
        (choices, picked)
    })
    .await
    else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut fixed = Vec::new();
    if crate::x::connected(&site.state).is_some() {
        fixed.push(serde_json::json!({"capability": "Read aloud", "provider": "xai", "name": "xAI text to speech"}));
        fixed.push(
            serde_json::json!({"capability": "Calls", "provider": "xai", "name": "Grok Voice"}),
        );
        fixed.push(serde_json::json!({"capability": "X search", "provider": "xai", "model": crate::x::MODEL, "name": crate::x::MODEL}));
    }
    Json(serde_json::json!({
        "image": { "picked": picked, "choices": choices },
        "fixed": fixed,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct CapabilityPick {
    capability: String,
    #[serde(flatten)]
    choice: crate::capability::Choice,
}

/// Picks the model for one kind of making; pictures alone take a pick.
async fn capability_pick(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(p): Json<CapabilityPick>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if p.capability != "image" {
        return (StatusCode::BAD_REQUEST, "Only pictures take a pick.").into_response();
    }
    let state = site.state.clone();
    let set = tokio::task::spawn_blocking(move || {
        let choices = crate::capability::image_choices(&state);
        crate::capability::set_image(&state, &p.choice, &choices)
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match set {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// Each harness this host runs, for Settings: whether it's on, its default
/// model, and how to sign it in should it show signed out.
fn harness_settings(state: &Path) -> Value {
    let off = crate::catalog::off(state);
    let models = crate::catalog::default_models(state);
    crate::catalog::configured()
        .into_iter()
        .map(|h| {
            serde_json::json!({
                "harness": h,
                "on": !off.contains(&h),
                "model": models.get(&h).cloned().unwrap_or(Value::Null),
                "remedy": crate::signin::remedy(&h),
            })
        })
        .collect()
}

/// One change to a harness in Settings: switched on or off, made the
/// default, or given a default model (empty for its own).
#[derive(Deserialize)]
struct HarnessSetting {
    harness: String,
    on: Option<bool>,
    #[serde(default)]
    default: bool,
    model: Option<String>,
}

async fn harness_set(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(h): Json<HarnessSetting>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    let set = (|| {
        if let Some(on) = h.on {
            crate::catalog::switch(&state, &h.harness, on)?;
        }
        if h.default {
            crate::catalog::set_default(&state, &h.harness)?;
        }
        if let Some(m) = &h.model {
            crate::catalog::set_default_model(&state, &h.harness, m)?;
        }
        Ok::<_, String>(())
    })();
    match set {
        Ok(()) => {
            // What's offered and who's signed out follow what's on.
            *site.offers.lock().unwrap() = None;
            *site.signed_out.lock().unwrap() = None;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(Deserialize)]
struct OthersPick {
    others: String,
}

/// Switches the harness that runs the other makers' models.
async fn others_pick(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(p): Json<OthersPick>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    let set = tokio::task::spawn_blocking(move || crate::catalog::set_others(&state, &p.others))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    match set {
        Ok(o) => {
            *site.offers.lock().unwrap() = Some((Instant::now(), o));
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// Which of the harnesses offered are signed in.
async fn signin_now(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    // Asking OpenCode and omp means running their listers: off the runtime.
    let all = tokio::task::spawn_blocking(|| {
        crate::catalog::harnesses()
            .into_iter()
            .map(|h| {
                let yes = crate::signin::signed(&h);
                (h, Value::from(yes))
            })
            .collect::<serde_json::Map<String, Value>>()
    })
    .await
    .unwrap_or_default();
    // What was just asked is fresher than the kept list.
    let out = all
        .iter()
        .filter(|(_, v)| v.as_bool() == Some(false))
        .map(|(h, _)| h.clone())
        .collect();
    *site.signed_out.lock().unwrap() = Some((Instant::now(), out));
    Json(all).into_response()
}

#[derive(Deserialize)]
struct SignIn {
    harness: String,
    #[serde(default)]
    provider: Option<String>,
}

/// How the device sign-ins started stand. One just done changes who's
/// signed in and what's offered.
async fn signin_progress(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let all = crate::signin::progress();
    if all
        .as_object()
        .into_iter()
        .flatten()
        .any(|(_, s)| s["state"] == "done")
    {
        *site.offers.lock().unwrap() = None;
        *site.signed_out.lock().unwrap() = None;
    }
    Json(all).into_response()
}

/// Signs a harness in: Codex and omp answer the page and code of a ChatGPT
/// device sign-in, each its own. OpenCode signs in through its own config.
async fn signin(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(s): Json<SignIn>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let harness = s.harness;
    if !matches!(harness.as_str(), "codex" | "omp") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let h = harness.clone();
    let started =
        tokio::task::spawn_blocking(move || crate::signin::device(&h, s.provider.as_deref()))
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
    match started {
        Ok((url, code)) => Json(serde_json::json!({ "url": url, "code": code })).into_response(),
        Err(e) => {
            eprintln!("libri: {harness} sign-in: {e}");
            (StatusCode::BAD_GATEWAY, e).into_response()
        }
    }
}

/// The accounts elsewhere this host is connected to.
/// The terminal's WebSocket. Browsers let any page open a WebSocket to
/// any address, with no CORS check, so the page asking must come from this
/// host or an origin it serves; otherwise any site a tailnet device visits
/// could type into the shell.
/// Which terminal a page opens: `?id=` names a shell of its own.
#[derive(Deserialize)]
struct TerminalAt {
    id: Option<String>,
}

async fn terminal(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Query(at): axum::extract::Query<TerminalAt>,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !site.open
        && site
            .device(&headers)
            .is_some_and(|d| d.scope == crate::host::Scope::Read)
    {
        return (StatusCode::FORBIDDEN, "This device may only read.").into_response();
    }
    if !site.admits(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let terminals = site.terminals.clone();
    let id = crate::terminal::shell_name(at.id.as_deref());
    ws.on_upgrade(move |socket| crate::terminal::serve(terminals, id, socket))
}

/// The Browser tile's live view of Libri's browser (browser_live.rs): a
/// WebSocket, checked as the terminal's is, since the person's input
/// drives a browser that may hold their sign-ins.
async fn browser_live(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !site.open
        && site
            .device(&headers)
            .is_some_and(|d| d.scope == crate::host::Scope::Read)
    {
        return (StatusCode::FORBIDDEN, "This device may only read.").into_response();
    }
    if !site.admits(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(b) = site.browser.clone() else {
        return (StatusCode::NOT_FOUND, "This host has no browser.").into_response();
    };
    ws.on_upgrade(move |socket| b.live(socket))
}

/// Files the person picked for a page's file input: a form of `node` (the
/// input DevTools named in its ask) and one or more `file`s, kept aside
/// in the browser's folder and handed to the input.
async fn browser_choose(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    mut form: Multipart,
) -> Response {
    if !site.signed(&headers) || !site.admits(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(b) = site.browser.clone() else {
        return (StatusCode::NOT_FOUND, "This host has no browser.").into_response();
    };
    let dir = site
        .state
        .join("browser")
        .join("uploads")
        .join(match token() {
            Ok(t) => t,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        });
    let mut node = None;
    let mut files = Vec::new();
    loop {
        let field = match form.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return (e.status(), e.body_text()).into_response(),
        };
        match field.name() {
            Some("node") => {
                node = field
                    .text()
                    .await
                    .ok()
                    .and_then(|t| t.trim().parse::<i64>().ok())
            }
            Some("file") => {
                let name = crate::files::plain(field.file_name().unwrap_or("file"));
                let Ok(bytes) = field.bytes().await else {
                    return StatusCode::BAD_REQUEST.into_response();
                };
                let path = dir.join(&name);
                if std::fs::create_dir_all(&dir)
                    .and_then(|()| std::fs::write(&path, &bytes))
                    .is_err()
                {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
                files.push(path);
            }
            _ => return StatusCode::BAD_REQUEST.into_response(),
        }
    }
    let Some(node) = node else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    match b.choose(node, files).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e).into_response(),
    }
}

/// The tools with a card, by every name a harness gives their calls.
async fn apps_cards(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match site.apps.cards().await {
        Ok(cards) => Json(serde_json::json!({ "cards": cards })).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e).into_response(),
    }
}

#[derive(Deserialize)]
struct AppView {
    server: String,
    uri: String,
}

/// A card's HTML and what it asks of its frame.
async fn apps_view(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Query(v): axum::extract::Query<AppView>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    // A Library page is Libri's own, read from the pages folder: the same
    // view the tool server gives any MCP Apps host, without the hop.
    if v.server == "libri" && v.uri.starts_with(crate::library::PAGE_URI) {
        let pages = site.pages.clone();
        let read = tokio::task::spawn_blocking(move || crate::library::view(&pages, &v.uri)).await;
        return match read {
            Ok(Ok(p)) => Json(serde_json::json!({
                "html": p.html,
                "csp": p.csp,
                "prefersBorder": false,
            }))
            .into_response(),
            Ok(Err(e)) => (StatusCode::NOT_FOUND, e).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        };
    }
    // A built-in's view ships in Libri.
    if v.server == "libri" && v.uri.starts_with(crate::builtins::VIEW_URI) {
        return match crate::builtins::view(&v.uri) {
            Some(html) => {
                Json(serde_json::json!({"html": html, "prefersBorder": false})).into_response()
            }
            None => (StatusCode::NOT_FOUND, format!("{} is no view.", v.uri)).into_response(),
        };
    }
    match site.apps.view(&v.server, &v.uri).await {
        Ok(view) => Json(view).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e).into_response(),
    }
}

#[derive(Deserialize)]
struct AppCall {
    server: String,
    tool: String,
    #[serde(default)]
    arguments: serde_json::Map<String, Value>,
}

/// A card's tool call, on the card's own server. It asks the person first
/// as the main chat's harness would ask of the model's own call: unless
/// the chat is on a mode that asks nothing, the card waits for a yes.
async fn apps_call(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(mut c): Json<AppCall>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    // The person's own hold on the browser comes in through the signed-in
    // site and nowhere else, so no model can take it in their name.
    if c.server == "libri"
        && let Some(r) = browser_person(&site, &c.tool, &c.arguments).await
    {
        return r;
    }
    eprintln!("libri: apps: a card calls {} on {}", c.tool, c.server);
    let harness = crate::catalog::main_choice(&site.state).pick.harness;
    let picks = crate::access::main_picks(&site.state, &harness);
    let mode = crate::access::grant(&harness, &site.state, &picks).mode;
    if !crate::access::asks_nothing(&harness, &site.state, &mode) {
        let ask = SeatAsk {
            id: format!("card-{}", crate::hippo::row_key()),
            tool: crate::apps::call_names(&c.server, &c.tool)[0].clone(),
            input: Value::Object(c.arguments.clone()),
            about: Some(format!("The {} card asks to run {}.", c.server, c.tool)),
            always: false,
        };
        let web = site.web.clone();
        match tokio::task::spawn_blocking(move || web.put(ask)).await {
            Ok(Some(Value::Object(input))) => c.arguments = input,
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => {
                return (StatusCode::FORBIDDEN, "The person said no.").into_response();
            }
        }
    }
    match site.apps.call(&c.server, &c.tool, c.arguments).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e).into_response(),
    }
}

/// The browser card's calls of the person's: the view, taking and handing
/// back control, and input. None for any other tool.
async fn browser_person(
    site: &Site,
    tool: &str,
    args: &serde_json::Map<String, Value>,
) -> Option<Response> {
    let b = match tool {
        "browser_view" | "browser_control" | "browser_input" | "browser_allow"
        | "browser_sites" => site.browser.clone(),
        _ => return None,
    };
    let Some(b) = b else {
        return Some((StatusCode::NOT_FOUND, "This host has no browser.").into_response());
    };
    let done = match tool {
        "browser_view" => Ok(b
            .view(args.get("since").and_then(Value::as_u64).unwrap_or(0))
            .await),
        "browser_control" => {
            let size = |k| args.get(k).and_then(Value::as_u64).map(|n| n as u32);
            let fit = size("width").zip(size("height"));
            b.control(
                args.get("take").and_then(Value::as_bool).unwrap_or(false),
                fit,
            )
            .await
            .map(|s| serde_json::json!({"status": s}))
        }
        "browser_allow" => b
            .allow(args.get("yes").and_then(Value::as_bool).unwrap_or(false))
            .map(|s| serde_json::json!({"status": s})),
        "browser_sites" => {
            let s = b
                .sites(
                    args.get("remove").and_then(Value::as_str),
                    args.get("stop").and_then(Value::as_bool).unwrap_or(false),
                )
                .await;
            Ok(serde_json::json!({"status": s}))
        }
        _ => match serde_json::from_value::<crate::browser::Input>(Value::Object(args.clone())) {
            Ok(i) => b.input(i).await.map(|()| serde_json::json!({})),
            Err(e) => Err(e.to_string()),
        },
    };
    // Shaped as a tool's result, as a card expects of any call.
    Some(
        Json(match done {
            Ok(v) => serde_json::json!({"content": [], "structuredContent": v}),
            Err(e) => {
                serde_json::json!({"content": [{"type": "text", "text": e}], "isError": true})
            }
        })
        .into_response(),
    )
}

#[derive(Deserialize)]
struct PageCall {
    tool: String,
    #[serde(default)]
    arguments: serde_json::Map<String, Value>,
}

/// A Library page's tool call. A page reaches its own data and nothing
/// else: its name comes from the address the Library framed it by, never
/// from the page, and no other tool of Libri's answers here. The person is
/// not asked first, as a card's call asks: what the page changes is the
/// page's own, by the person's own hand.
async fn page_call(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(c): Json<PageCall>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (state, pages) = (site.state.clone(), site.pages.clone());
    let page = name.clone();
    let held = tokio::task::spawn_blocking(move || crate::library::held(&pages, &page)).await;
    match held {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return (StatusCode::NOT_FOUND, e).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
    let done = match c.tool.as_str() {
        "page_data_watch" => {
            let since = c
                .arguments
                .get("since")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            crate::page_data::wait(&state, &name, since, crate::page_data::WAIT)
                .await
                .and_then(|d| serde_json::to_value(d).map_err(|e| e.to_string()))
        }
        "page_data" | "page_data_change" => {
            let patch = c.arguments.get("patch").cloned();
            let changing = c.tool == "page_data_change";
            tokio::task::spawn_blocking(move || match (changing, patch) {
                (false, _) => crate::page_data::read(&state, &name),
                (true, Some(p)) => crate::page_data::change(&state, &name, &p, "person"),
                (true, None) => Err("page_data_change takes a patch.".into()),
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
            .and_then(|d| serde_json::to_value(d).map_err(|e| e.to_string()))
        }
        other => page_reads(&site, &name, other, c.arguments).await,
    };
    // Shaped as a tool's result, as an MCP App expects of any call.
    Json(match done {
        Ok(d) => serde_json::json!({
            "content": [{"type": "text", "text": serde_json::to_string(&d).unwrap_or_default()}],
            "structuredContent": d,
        }),
        Err(e) => serde_json::json!({"content": [{"type": "text", "text": e}], "isError": true}),
    })
    .into_response()
}

/// A page's call of a connection's tool, `mcp__NAME__tool`: only of a
/// connection the page named when published, and only a tool that reads
/// (builtins::page_may_read), so a page shows the person's calendar or
/// mail but never acts in their name.
async fn page_reads(
    site: &Site,
    page: &str,
    called: &str,
    arguments: serde_json::Map<String, Value>,
) -> Result<Value, String> {
    let refused = || {
        format!(
            "A page reaches its own data and the read tools of connections it names; {called} is neither."
        )
    };
    let (server, tool) = called
        .strip_prefix("mcp__")
        .and_then(|r| r.split_once("__"))
        .ok_or_else(refused)?;
    let (pages, p) = (site.pages.clone(), page.to_owned());
    let named = tokio::task::spawn_blocking(move || crate::library::reads(&pages, &p))
        .await
        .map_err(|e| e.to_string())?;
    if !named.iter().any(|n| n == server) {
        return Err(refused());
    }
    let definition = site.apps.definition(server, tool).await?;
    let (state, s, t) = (site.state.clone(), server.to_owned(), tool.to_owned());
    let may = tokio::task::spawn_blocking(move || {
        crate::builtins::page_may_read(&state, &s, &t, &definition)
    })
    .await
    .map_err(|e| e.to_string())?;
    if !may {
        return Err(refused());
    }
    let r = site.apps.call(server, tool, arguments).await?;
    // The server's own result, unwrapped below as the page's data is.
    Ok(r.get("structuredContent").cloned().unwrap_or(r))
}

/// A card's resource read, on the card's own server.
async fn apps_read(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(v): Json<AppView>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match site.apps.read(&v.server, &v.uri).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e).into_response(),
    }
}

#[derive(Deserialize)]
struct Sandbox {
    #[serde(default)]
    csp: Option<String>,
    /// The origin the host page's fonts come from.
    #[serde(default)]
    font: Option<String>,
}

/// The cards' sandbox proxy, with the card's CSP as its header. It holds
/// nothing of the person's, so it asks for no sign-in: the frame can't
/// carry the page's token without handing it to the card.
async fn apps_sandbox(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Query(s): axum::extract::Query<Sandbox>,
) -> Response {
    let csp: Value = s
        .csp
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or(Value::Null);
    // This host as the card would name it: struck from what it may reach.
    let mut own = site.origins.clone();
    if let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) {
        own.push(format!("https://{host}"));
        own.push(format!("http://{host}"));
    }
    if let Ok(url) = std::env::var("LIBRI_SITE_URL") {
        own.push(url.trim_end_matches('/').to_owned());
    }
    let policy = crate::apps::policy(&csp, &own, s.font.as_deref());
    let Ok(policy) = HeaderValue::from_str(&policy) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    (
        [
            (header::CONTENT_SECURITY_POLICY, policy),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            ),
        ],
        axum::response::Html(crate::apps::SANDBOX),
    )
        .into_response()
}

async fn connections_now(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (signing, said) = match &*site.xai.lock().unwrap() {
        Some(XaiSignIn {
            failed: Some(e), ..
        }) => (None, Some(e.clone())),
        Some(XaiSignIn { url, code, .. }) => {
            (Some(serde_json::json!({ "url": url, "code": code })), None)
        }
        None => (None, None),
    };
    Json(serde_json::json!({
        "servers": crate::connections::all(&site.state)
            .iter()
            .map(|c| crate::connections::shown(&site.state, c))
            .collect::<Vec<_>>(),
        "xai": crate::x::connected(&site.state),
        "xai_signing": signing,
        "xai_said": said,
    }))
    .into_response()
}

/// The servers this host's deployment adds to every turn, by name, each
/// with how many tools it lists (null when it doesn't answer): Settings
/// shows them beside the connections, to see and not to change.
async fn host_servers_now(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let servers = match crate::servers::deployment() {
        Ok(s) => s,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let counts = servers
        .iter()
        .filter(|(name, _)| name.as_str() != "libri")
        .map(|(name, entry)| async move {
            let tools = async {
                let client = crate::look::connect_as(entry, ()).await.ok()?;
                let tools = client.peer().list_all_tools().await.ok();
                let _ = client.cancel().await;
                tools.map(|t| t.len())
            };
            let tools = tokio::time::timeout(std::time::Duration::from_secs(10), tools)
                .await
                .ok()
                .flatten();
            serde_json::json!({ "name": name, "tools": tools })
        });
    Json(futures_util::future::join_all(counts).await).into_response()
}

/// The built-ins, each with its switch and provider.
async fn builtins_now(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    match tokio::task::spawn_blocking(move || crate::builtins::shown(&state)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// The providers Libri can connect, each with what it covers and its
/// connection once there is one.
async fn providers_now(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(crate::builtins::accounts(&site.state)).into_response()
}

/// Adds the connection of a provider, unless there is one: answers its
/// name, for the page to sign in to.
async fn provider_connect(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    match tokio::task::spawn_blocking(move || crate::builtins::connect(&state, &id)).await {
        Ok(Ok(name)) => Json(serde_json::json!({ "name": name })).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct ToolRule {
    tool: String,
    rule: crate::connections::Rule,
}

/// Sets one tool of a connection to Allow, Ask or Off.
async fn tool_rule(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(r): Json<ToolRule>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::connections::set_rule(&site.state, &name, &r.tool, r.rule) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// What a connection brings in (its calendars), each kind listed by its
/// provider's tool, with each item shown or hidden.
async fn items_now(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut out = Vec::new();
    for kind in crate::builtins::item_kinds(&site.state, &name) {
        let got = site
            .apps
            .call(&name, &kind.tool, serde_json::Map::new())
            .await;
        let (items, failed) = match got {
            Ok(r) if r["isError"] != Value::Bool(true) => (
                crate::builtins::items(&site.state, &name, kind, &crate::builtins::answer(&r)),
                None,
            ),
            Ok(_) => (
                Vec::new(),
                Some(format!("{name} would not list its {}.", kind.kind)),
            ),
            Err(e) => (Vec::new(), Some(e)),
        };
        out.push(serde_json::json!({"kind": kind.kind, "title": kind.title, "items": items, "failed": failed}));
    }
    Json(out).into_response()
}

#[derive(Deserialize)]
struct ItemShow {
    kind: String,
    id: String,
    shown: bool,
}

/// Shows or hides one thing a connection brings in.
async fn item_show(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(i): Json<ItemShow>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::builtins::show_item(&site.state, &name, &i.kind, &i.id, i.shown) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// Switches a built-in or picks its provider.
async fn builtin_set(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(change): Json<crate::builtins::Set>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    match tokio::task::spawn_blocking(move || crate::builtins::set(&state, &id, change)).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// A built-in's view's call: `agenda` alone, `{from, to}` in RFC 3339,
/// run on the connection of the provider the person picked and read
/// through its preset (builtins::events), so the view is one whatever the
/// provider. It reads and never acts.
async fn builtin_call(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(c): Json<PageCall>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let done = builtin_agenda(&site, id, &c).await;
    Json(match done {
        Ok(d) => serde_json::json!({
            "content": [{"type": "text", "text": serde_json::to_string(&d).unwrap_or_default()}],
            "structuredContent": d,
        }),
        Err(e) => serde_json::json!({"content": [{"type": "text", "text": e}], "isError": true}),
    })
    .into_response()
}

async fn builtin_agenda(site: &Site, id: String, c: &PageCall) -> Result<Value, String> {
    use crate::builtins::{answer, arguments};
    if c.tool != "agenda" {
        return Err(format!(
            "A built-in's view asks for its agenda alone, not {}.",
            c.tool
        ));
    }
    let at = |k: &str| {
        c.arguments
            .get(k)
            .and_then(Value::as_str)
            .filter(|t| chrono::DateTime::parse_from_rfc3339(t).is_ok())
            .map(str::to_owned)
            .ok_or_else(|| format!("agenda takes {k} in RFC 3339."))
    };
    let (from, to) = (at("from")?, at("to")?);
    let state = site.state.clone();
    let (conn, a) = tokio::task::spawn_blocking(move || crate::builtins::agenda(&state, &id))
        .await
        .map_err(|e| e.to_string())??;
    let failed = |r: &Value| {
        (r["isError"] == Value::Bool(true)).then(|| {
            let said = crate::builtins::answer(r);
            let text: String = r["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["text"].as_str())
                .collect();
            if text.is_empty() {
                said.to_string()
            } else {
                text
            }
        })
    };
    let r = site
        .apps
        .call(&conn, &a.tool, arguments(&a.arguments, &from, &to))
        .await?;
    if let Some(e) = failed(&r) {
        return Err(e);
    }
    let state = site.state.clone();
    let all = crate::builtins::shown_events(&state, &conn, a, &answer(&r));
    Ok(serde_json::json!({"events": all}))
}

/// Starts connecting xAI by device code, or shows the one under way, and
/// waits for the approval on a thread of its own.
async fn xai_start(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Some(s) = &*site.xai.lock().unwrap()
        && s.failed.is_none()
    {
        return Json(serde_json::json!({ "url": s.url, "code": s.code })).into_response();
    }
    let sign_in = match tokio::task::spawn_blocking(crate::x::sign_in).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            eprintln!("libri: xai: {e}");
            return (StatusCode::BAD_GATEWAY, e).into_response();
        }
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let (url, code) = (sign_in.url.clone(), sign_in.code.clone());
    *site.xai.lock().unwrap() = Some(XaiSignIn {
        url: url.clone(),
        code: code.clone(),
        failed: None,
    });
    let s = site.clone();
    tokio::task::spawn_blocking(move || {
        let done = crate::x::approved(&s.state, &sign_in);
        let mut held = s.xai.lock().unwrap();
        match done {
            Ok(()) => *held = None,
            Err(e) => {
                eprintln!("libri: xai: {e}");
                if let Some(h) = &mut *held {
                    h.failed = Some(e);
                }
            }
        }
    });
    Json(serde_json::json!({ "url": url, "code": code })).into_response()
}

#[derive(Deserialize)]
struct XaiKey {
    key: String,
}

/// Connects xAI by an API key.
async fn xai_key(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(k): Json<XaiKey>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    match tokio::task::spawn_blocking(move || crate::x::use_key(&state, &k.key))
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
    {
        Ok(()) => {
            *site.xai.lock().unwrap() = None;
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

async fn xai_drop(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::x::disconnect(&site.state) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct VoicePick {
    voice: String,
}

/// Picks the voice replies are read aloud and calls are held in.
async fn voice_pick(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(p): Json<VoicePick>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !crate::x::voice_ok(&p.voice) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match VOICE.write(&site.state, &p.voice) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct LayoutSince {
    /// Wait for a version past this one; none answers at once.
    since: Option<u64>,
}

/// The wide page's workspaces and their tiles (layout.rs). With `since`,
/// the answer waits for the next change, which is how an open page follows
/// one the agent or another screen made.
async fn layout_now(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    since: axum::extract::Query<LayoutSince>,
) -> Response {
    layout_read(&site, &headers, crate::layout::Store::Desk, since.0.since).await
}

async fn layout_keep(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(layout): Json<Value>,
) -> Response {
    layout_change(&site, &headers, crate::layout::Store::Desk, Some(layout)).await
}

async fn layout_undo(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    layout_change(&site, &headers, crate::layout::Store::Desk, None).await
}

/// A phone's workspaces, the items on its bar (layout.rs), kept apart from
/// the wide page's and followed the same way.
async fn phone_layout_now(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    since: axum::extract::Query<LayoutSince>,
) -> Response {
    layout_read(&site, &headers, crate::layout::Store::Phone, since.0.since).await
}

async fn phone_layout_keep(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(layout): Json<Value>,
) -> Response {
    layout_change(&site, &headers, crate::layout::Store::Phone, Some(layout)).await
}

async fn phone_layout_undo(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    layout_change(&site, &headers, crate::layout::Store::Phone, None).await
}

/// The kinds of tile and their menus' rows (tiles.rs), which the apps draw
/// from rather than lists of their own.
async fn tiles_listed(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(crate::tiles::listed()).into_response()
}

async fn layout_read(
    site: &Site,
    headers: &HeaderMap,
    store: crate::layout::Store,
    since: Option<u64>,
) -> Response {
    if !site.signed(headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let l = match since {
        Some(v) => crate::layout::wait(&site.state, store, v, crate::layout::WAIT).await,
        None => crate::layout::read(&site.state, store),
    };
    Json(l).into_response()
}

/// Keeps `layout`, or with none undoes the latest change.
async fn layout_change(
    site: &Site,
    headers: &HeaderMap,
    store: crate::layout::Store,
    layout: Option<Value>,
) -> Response {
    if !site.signed(headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    let undo = layout.is_none();
    let done = tokio::task::spawn_blocking(move || match layout {
        Some(l) => crate::layout::change(&state, store, l, "person"),
        None => crate::layout::undo(&state, store, "person"),
    })
    .await;
    match done {
        Ok(Ok(l)) => Json(l).into_response(),
        Ok(Err(e)) if undo => (StatusCode::CONFLICT, e).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Runs a board call off the async threads, answering 401 to a page that
/// isn't signed in and 400 with the reason for a refusal.
async fn board_call<T: Serialize + Send + 'static>(
    site: &Site,
    headers: &HeaderMap,
    f: impl FnOnce(&std::path::Path) -> Result<T, String> + Send + 'static,
) -> Response {
    if !site.signed(headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    match tokio::task::spawn_blocking(move || f(&state)).await {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// The whiteboards, in tab order (boards.rs).
async fn boards_list(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    board_call(&site, &headers, crate::boards::list).await
}

#[derive(Deserialize)]
struct BoardName {
    name: String,
}

async fn board_create(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(b): Json<BoardName>,
) -> Response {
    board_call(&site, &headers, move |s| crate::boards::create(s, &b.name)).await
}

async fn board_rename(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(b): Json<BoardName>,
) -> Response {
    board_call(&site, &headers, move |s| {
        crate::boards::rename(s, &id, &b.name)
    })
    .await
}

#[derive(Deserialize)]
struct BoardOrder {
    ids: Vec<String>,
}

async fn boards_order(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(o): Json<BoardOrder>,
) -> Response {
    board_call(&site, &headers, move |s| crate::boards::reorder(s, &o.ids)).await
}

async fn board_remove(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    board_call(&site, &headers, move |s| crate::boards::remove(s, &id)).await
}

/// Marks the board the person is looking at, where "jot this down" lands.
async fn board_focus(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    board_call(&site, &headers, move |s| crate::boards::focus(s, &id)).await
}

/// A board's drawing. With `since`, the answer waits for the next change
/// (204 when none came) and gives the ops past it, or the whole board when
/// the log no longer reaches back: how an open board follows the agent's
/// notes and another screen's strokes.
async fn board_scene(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    since: axum::extract::Query<LayoutSince>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    let got = tokio::task::spawn_blocking(move || match since.0.since {
        Some(v) => crate::boards::wait(&state, &id, v, crate::boards::WAIT)
            .map(|w| w.map(|s| serde_json::to_value(s).unwrap_or_default())),
        None => crate::boards::scene(&state, &id)
            .map(|s| Some(serde_json::to_value(s).unwrap_or_default())),
    })
    .await;
    match got {
        Ok(Ok(Some(s))) => Json(s).into_response(),
        Ok(Ok(None)) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(e)) => (StatusCode::NOT_FOUND, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct BoardChange {
    /// Ops, as the shared-object layer has them (boards.rs `Op`).
    #[serde(default)]
    ops: Vec<crate::boards::Op>,
    /// Or the elements and images the canvas changed, made into ops here.
    #[serde(default)]
    elements: Vec<Value>,
    #[serde(default)]
    files: serde_json::Map<String, Value>,
}

/// Applies the ops a view sent (or the elements and images it changed) to
/// the board; answers the board's version.
async fn board_merge(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(c): Json<BoardChange>,
) -> Response {
    board_call(&site, &headers, move |s| {
        let mut ops = c.ops;
        ops.extend(crate::boards::ops_of(&c.elements, &c.files));
        crate::boards::apply(s, &id, &ops, "person").map(|v| serde_json::json!({ "version": v }))
    })
    .await
}

/// A board link as the Share menu shows it: with the address it opens at,
/// when the deployment has given one.
fn link_shown(l: crate::boards::Link) -> Value {
    let url = crate::share::link_url(&l.token);
    let mut v = serde_json::to_value(l).unwrap_or_default();
    v["url"] = serde_json::json!(url);
    v
}

/// The board's live links, and whether links can be made at all here.
async fn board_links(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    board_call(&site, &headers, move |s| {
        let links: Vec<Value> = crate::boards::links(s, &id)?
            .into_iter()
            .map(link_shown)
            .collect();
        Ok(serde_json::json!({
            "links": links,
            "public": crate::share::link_url("").is_some(),
        }))
    })
    .await
}

#[derive(Deserialize)]
struct LinkAsked {
    role: crate::boards::Role,
    /// Days until it lapses; none for no end.
    #[serde(default)]
    days: Option<u32>,
}

/// Makes a link to the board.
async fn board_link_create(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(a): Json<LinkAsked>,
) -> Response {
    if crate::share::link_url("").is_none() {
        return (
            StatusCode::CONFLICT,
            "Board links need a public address (share.url) on this host.",
        )
            .into_response();
    }
    board_call(&site, &headers, move |s| {
        crate::boards::link_create(s, &id, a.role, a.days).map(link_shown)
    })
    .await
}

/// Takes a board link back.
async fn board_link_revoke(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(token): axum::extract::Path<String>,
) -> Response {
    board_call(&site, &headers, move |s| {
        crate::boards::link_revoke(s, &token).map(|()| serde_json::json!({}))
    })
    .await
}

/// The board as an `.excalidraw` file to save.
async fn board_export(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    match tokio::task::spawn_blocking(move || crate::boards::export(&state, &id)).await {
        Ok(Ok((name, file))) => {
            let safe: String = name
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == ' ' || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            (
                [
                    (header::CONTENT_TYPE, "application/json".to_owned()),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("attachment; filename=\"{}.excalidraw\"", safe.trim()),
                    ),
                ],
                file.to_string(),
            )
                .into_response()
        }
        Ok(Err(e)) => (StatusCode::NOT_FOUND, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Adds a connection or changes one: answers it as the list shows it.
async fn server_put(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(change): Json<crate::connections::Change>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::connections::put(&site.state, change) {
        Ok(c) => {
            // One at a provider's address fills what that provider covers.
            if let Err(e) = crate::builtins::settle_all(&site.state) {
                eprintln!("libri: built-ins: {e}");
            }
            Json(crate::connections::shown(&site.state, &c)).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// Connects to a connection as a harness would and lists its tools, or
/// says it wants a sign-in.
async fn server_test(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::connections::test(&site.state, &name).await {
        Ok(t) => Json(t).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, e).into_response(),
    }
}

#[derive(Deserialize, Default)]
struct SigningIn {
    /// The address the page reaches this host at, which the sign-in
    /// returns the browser to. The page says it because the app served
    /// from one origin and talking to a host at another sends its own
    /// origin as `Origin`, which is not this host's.
    origin: Option<String>,
    /// Started by the Android app, which the server returns to by its own
    /// scheme (`connections::APP_BACK`).
    #[serde(default)]
    app: bool,
}

/// Starts signing in to a connection: answers the page to sign in on.
async fn server_sign_in(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
    body: axum::body::Bytes,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let said: SigningIn = serde_json::from_slice(&body).unwrap_or_default();
    let origin = said.origin.or_else(|| {
        headers
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    });
    let started = crate::connections::sign_in(
        &site.state,
        &site.back,
        origin.as_deref(),
        &site.connecting,
        &name,
        said.app,
    );
    match started.await {
        Ok(url) => Json(serde_json::json!({ "url": url })).into_response(),
        Err(e) => {
            eprintln!("libri: {e}");
            (StatusCode::BAD_GATEWAY, e).into_response()
        }
    }
}

#[derive(Deserialize)]
struct Finish {
    address: String,
}

/// Finishes a sign-in from the address the server sent the browser to,
/// pasted by a person signing in on another device.
async fn server_finish(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(f): Json<Finish>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::connections::finish(&site.connecting, &f.address).await {
        Ok(name) => Json(serde_json::json!({ "name": name })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// Where a sign-in returns the browser, from the page (`BACK`) or straight
/// from the server on this machine. The `state` it carries, made for one
/// started sign-in, is what lets it in.
async fn server_back(
    State(site): State<Arc<Site>>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
) -> Response {
    let address = format!("?{}", query.unwrap_or_default());
    match crate::connections::finish(&site.connecting, &address).await {
        Ok(name) => format!("{name} is connected. You can close this page.").into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

async fn server_remove(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::connections::remove(&site.state, &name) {
        Ok(()) => {
            if let Err(e) = crate::builtins::settle_all(&site.state) {
                eprintln!("libri: built-ins: {e}");
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(Deserialize)]
struct EffortPick {
    effort: String,
    /// Temp's effort rather than the main chat's.
    #[serde(default)]
    temp: bool,
}

/// Switches the effort from the next turn on; the one under way keeps its.
/// Only the levels the model takes.
async fn effort_pick(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(p): Json<EffortPick>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let kept = if p.temp {
        &crate::catalog::TEMP_EFFORT
    } else {
        &crate::model::EFFORT
    };
    let offers = offers(&site).await;
    let main = current(&site, &offers);
    let pick = if p.temp {
        let own = temp_current(&site, &offers);
        // Thinking set for Temp holds it to this model, so a later change
        // of the main chat's can't leave the thinking on the wrong one.
        if own.is_none()
            && let Err(e) = crate::catalog::temp_pick(&site.state, &main)
        {
            return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response();
        }
        own.unwrap_or(main)
    } else {
        main
    };
    // Empty goes back to the harness's own default.
    if p.effort.is_empty() {
        kept.clear(&site.state);
        return StatusCode::NO_CONTENT.into_response();
    }
    let takes = offers
        .iter()
        .find(|o| o.harness == pick.harness && o.model == pick.model)
        .is_some_and(|o| o.efforts.contains(&p.effort));
    if !takes {
        return (
            StatusCode::BAD_REQUEST,
            "The model doesn't take that effort.",
        )
            .into_response();
    }
    match kept.write(&site.state, &p.effort) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(Deserialize)]
struct AccessPick {
    /// Temp's pick rather than the main chat's.
    #[serde(default)]
    temp: bool,
    #[serde(flatten)]
    picks: crate::access::Picks,
}

/// Switches how freely the main chat's harness, or Temp's, acts from the next
/// turn on: its mode, its sandbox, or both.
async fn access_pick(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(p): Json<AccessPick>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let harness = if p.temp {
        scratchpad_choice(&site).await.pick.harness
    } else {
        crate::catalog::main_choice(&site.state).pick.harness
    };
    let picked = if p.temp {
        crate::access::pick_temp(&site.state, &harness, p.picks)
    } else {
        crate::access::pick_main(&site.state, &harness, p.picks)
    };
    match picked {
        Ok(()) => {
            // A Claude turn under way takes the mode at its next request.
            if !p.temp && harness == "claude" {
                let picks = crate::access::main_picks(&site.state, &harness);
                let grant = crate::access::grant(&harness, &site.state, &picks);
                crate::model::switch("main", None, Some(&grant.mode));
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(Deserialize)]
struct AccessDefault {
    harness: String,
    #[serde(flatten)]
    picks: crate::access::Picks,
}

/// Sets what a harness starts on, from Settings.
async fn access_default(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(d): Json<AccessDefault>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::access::pick_default(&site.state, &d.harness, d.picks) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

async fn forget_now(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::forget::window(&site.state) {
        Some(m) => Json(serde_json::json!({ "minutes": m })).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Deserialize)]
struct Window {
    minutes: u64,
}

/// Sets how long it remembers; the next sweep, within a minute, uses it.
async fn forget_pick(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(w): Json<Window>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::forget::pick(&site.state, w.minutes) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// How the memory is working, for the sidebar: hippo's `health`, and failing
/// when hippo has stopped answering.
async fn memory_health(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let read = site.clone();
    match tokio::task::spawn_blocking(move || read.memory.read()).await {
        Ok(health) => Json(health).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Who writes workers' titles and summaries: the person's pick and the
/// default that stands without one. Finding the default asks the harnesses,
/// so off the async threads.
async fn synopsis_now(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let writer = site.writer.clone();
    match tokio::task::spawn_blocking(move || crate::synopsis::Writer::cheapest(&*writer)).await {
        Ok(cheapest) => Json(serde_json::json!({
            "picked": crate::catalog::synopsis_picked(&site.state),
            "default": cheapest,
        }))
        .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Any model on offer; neither field goes back to the default.
#[derive(Deserialize)]
struct SynopsisPick {
    harness: Option<String>,
    model: Option<String>,
}

/// The model new workers start on, while it is the person's pick; none
/// follows the main chat.
async fn worker_model(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(serde_json::json!({ "picked": crate::catalog::worker_picked(&site.state) }))
        .into_response()
}

/// Any model on offer; neither field goes back to following the main chat.
#[derive(Deserialize)]
struct WorkerModelPick {
    harness: Option<String>,
    model: Option<String>,
}

async fn synopsis_pick(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(p): Json<SynopsisPick>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let pick = match (p.harness, p.model) {
        (Some(harness), Some(model)) => Some(crate::catalog::Pick { harness, model }),
        _ => None,
    };
    if let Some(p) = &pick
        && !offers(&site)
            .await
            .iter()
            .any(|o| o.harness == p.harness && o.model == p.model)
    {
        return (StatusCode::BAD_REQUEST, "That model is not on offer.").into_response();
    }
    match crate::catalog::synopsis_pick(&site.state, pick.as_ref()) {
        Ok(()) => synopsis_now(State(site), headers).await,
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// New workers only: a worker keeps the model it holds.
async fn worker_model_pick(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(p): Json<WorkerModelPick>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let pick = match (p.harness, p.model) {
        (Some(harness), Some(model)) => Some(crate::catalog::Pick { harness, model }),
        _ => None,
    };
    if let Some(p) = &pick
        && !offers(&site)
            .await
            .iter()
            .any(|o| o.harness == p.harness && o.model == p.model)
    {
        return (StatusCode::BAD_REQUEST, "That model is not on offer.").into_response();
    }
    match crate::catalog::worker_pick(&site.state, pick.as_ref()) {
        Ok(()) => worker_model(State(site), headers).await,
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// The names new workers are given, and what the person changed.
async fn worker_names(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(crate::names::listed(&site.state)).into_response()
}

/// New workers only: a worker keeps the name it has.
async fn worker_name_change(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(c): Json<crate::names::Change>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::names::change(&site.state, &c) {
        Ok(left_out) => Json(crate::names::answer(&site.state, left_out)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(Deserialize)]
struct Guide {
    text: String,
}

/// The person's standing rules, as Settings edits them.
async fn guide_rules(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(serde_json::json!({ "text": crate::guide::rules(&site.state) })).into_response()
}

/// Taken up by the next turn, which reads the file for its prompt.
async fn guide_change(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(g): Json<Guide>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::guide::set_rules(&site.state, &g.text) {
        Ok(()) => guide_rules(State(site), headers).await,
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// What is true of this host alone, as Settings edits it.
async fn host_facts(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(serde_json::json!({ "text": crate::guide::host(&site.state) })).into_response()
}

/// Taken up by the next turn, as the person's rules are.
async fn host_facts_change(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(g): Json<Guide>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::guide::set_host(&site.state, &g.text) {
        Ok(()) => host_facts(State(site), headers).await,
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(Deserialize)]
struct SkillSaved {
    name: String,
    description: String,
    body: String,
}

#[derive(Deserialize)]
struct SkillsManaged {
    on: bool,
}

/// Libri's skills and the person's own, as Settings lists them, the
/// harnesses' folders brought in line first: a skill a model just made in
/// Libri's store shows, and reaches the harnesses, from here on.
fn skills_shelf(site: &Site, report: Option<crate::skills::Report>) -> Response {
    let Some(places) = crate::skills::Places::here() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let on = crate::skills::managed(&site.state);
    let report = match report {
        Some(r) => Ok(r),
        None => crate::skills::sync(&places, on),
    };
    match report {
        Ok(r) => Json(crate::skills::shelf(&places, r)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn skills_now(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    skills_shelf(&site, None)
}

/// The skills each harness finds, for the composer's `$` menu.
async fn skills_menu(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(places) = crate::skills::Places::here() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let all: std::collections::BTreeMap<String, crate::skills::Menu> = crate::catalog::harnesses()
        .into_iter()
        .map(|h| {
            let menu = crate::skills::menu(&places, &h);
            (h, menu)
        })
        .collect();
    Json(all).into_response()
}

/// Taken up by each harness the next time it looks, without a restart.
async fn skill_save(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(s): Json<SkillSaved>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(places) = crate::skills::Places::here() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match crate::skills::save(&places, &s.name, &s.description, &s.body) {
        Ok(()) => skills_shelf(&site, None),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

async fn skill_delete(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(places) = crate::skills::Places::here() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match crate::skills::delete(&places.store, &name) {
        Ok(()) => skills_shelf(&site, None),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// Skill management on or off. Off takes back what Libri installed and
/// leaves everything else.
async fn skills_managed(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(s): Json<SkillsManaged>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(places) = crate::skills::Places::here() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match crate::skills::set_managed(&site.state, &places, s.on) {
        Ok(r) => skills_shelf(&site, Some(r)),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct SkillsSource {
    url: Option<String>,
}

/// Names the repository Libri takes skills from, or forgets it, and pulls.
async fn skills_source(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(s): Json<SkillsSource>,
) -> Response {
    skills_pulled(site, headers, Some(s.url)).await
}

/// Pulls the source now, rather than at the hour.
async fn skills_pull(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    skills_pulled(site, headers, None).await
}

/// A pull runs git over the network, so off the async workers.
async fn skills_pulled(
    site: Arc<Site>,
    headers: HeaderMap,
    set: Option<Option<String>>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(places) = crate::skills::Places::here() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let state = site.state.clone();
    let done = tokio::task::spawn_blocking(move || match set {
        Some(url) => crate::skills::set_source(&state, &places, url.as_deref()),
        None => crate::skills::pull(&state, &places),
    })
    .await;
    match done {
        Ok(Ok(r)) => skills_shelf(&site, Some(r)),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn stop(State(site): State<Arc<Site>>, headers: HeaderMap) -> StatusCode {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED;
    }
    site.halt.press();
    StatusCode::ACCEPTED
}

/// `/clear`: the main chat's next turn starts a new session on whichever
/// harness runs it. Refused while a turn runs, as that turn would record
/// its session again on ending.
async fn session_clear(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if site.web.typing.load(Ordering::Relaxed) {
        return (StatusCode::CONFLICT, "a turn is running").into_response();
    }
    match crate::sessions::clear(&site.state, crate::sessions::MAIN) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// The harness the main chat runs on and its own slash commands, as it last
/// reported them, for the page's `/` menu; `all` holds every harness's.
async fn commands_now(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let harness = crate::catalog::harness_now();
    Json(serde_json::json!({
        "commands": crate::commands::of(&site.state, &harness),
        "known": crate::commands::known(&site.state, &harness),
        "harness": harness,
        "all": crate::commands::all(&site.state),
    }))
    .into_response()
}

/// The subagents at work, whether a turn is under way, and whether a
/// restart would cut nothing short: `quiet`, which deploys wait for.
async fn task_now(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    // An ask waiting on the person holds the chat as a turn does: a
    // connection's relay may ask the main chat when no turn of its runs
    // (connections.rs), and the page keeps open asks only while one is.
    let typing = site.web.typing.load(Ordering::Relaxed)
        || site
            .web
            .asks
            .lock()
            .unwrap()
            .iter()
            .any(|(a, _)| !a.id.starts_with(SCRATCHPAD_ASK));
    let state = site.state.clone();
    let busy = tokio::task::spawn_blocking(move || busy(&state, typing))
        .await
        .unwrap_or_else(|_| vec!["an unknown run".into()]);
    Json(serde_json::json!({
        "agents": site.web.agents.lock().unwrap().clone(),
        "typing": typing,
        "quiet": busy.is_empty(),
        "busy": busy,
        "turn": site.web.turn.lock().unwrap().clone(),
        // The host's clock, to time the turn's start and its calls by.
        "now": unix_ms(),
    }))
    .into_response()
}

/// What is at work that a restart would cut off, for a deploy to wait on or
/// name: the main chat's turn (`typing`), each worker's run, waiting or
/// running, and a Temp run. Each would resume from its log, but the person
/// waits on a turn, and a worker's run starts over from its last entry.
fn busy(state: &Path, typing: bool) -> Vec<String> {
    let mut v = Vec::new();
    if typing {
        v.push("the main turn".to_owned());
    }
    v.extend(
        task::working(state)
            .into_iter()
            .map(|(name, _)| format!("worker {name}")),
    );
    if scratchpad::current(state).is_some_and(|id| scratchpad::working(state, &id)) {
        v.push("Temp".to_owned());
    }
    v
}

#[cfg(test)]
fn quiet(state: &Path, typing: bool) -> bool {
    busy(state, typing).is_empty()
}

/// Every thread on the Work page: Libri's subagents, at work or done.
async fn work_list(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    match tokio::task::spawn_blocking(move || (task::threads(&state), task::talk(&state))).await {
        Ok((threads, talk)) => {
            let threads: Vec<_> = threads.into_iter().map(card_only).collect();
            // `talk`: the messages lately gone between agents, for the tree.
            Json(serde_json::json!({ "threads": threads, "talk": talk })).into_response()
        }
        Err(e) => {
            eprintln!("libri: site work: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// A thread as its card needs it. The cards show a brief's first line alone
/// (chat.ts and WorkPlace.kt `firstLine`) and no report, and the thread's
/// page reads its log apart; sent whole, the briefs made most of a 260 KB
/// list the page asks for every 4 seconds while a worker runs.
fn card_only(mut t: task::Thread) -> task::Thread {
    t.brief = brief_head(&t.brief).to_owned();
    t.report = None;
    t
}

/// A brief up to its first line that is neither blank nor a code fence's,
/// the line `firstLine` takes.
fn brief_head(brief: &str) -> &str {
    let mut end = 0;
    for line in brief.split_inclusive('\n') {
        end += line.len();
        let t = line.trim();
        let fence = ["```", "~~~"].iter().any(|f| {
            t.strip_prefix(f)
                .is_some_and(|rest| !rest.contains(char::is_whitespace))
        });
        if !t.is_empty() && !fence {
            break;
        }
    }
    brief[..end].trim_end_matches(['\n', '\r'])
}

/// One thread's whole log, as `i|kind: text` lines, the shape hippo gives
/// the seat's agents in, so the page reads both alike.
async fn work_log(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match task::log(&site.state, &name) {
        Ok(log) => Json(serde_json::json!({ "text": task::chat_keyed(&log) })).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// What a thread's next run goes on: its own choice, else the main chat's.
async fn thread_choice(site: &Arc<Site>, name: &str) -> crate::catalog::TurnPick {
    thread_choice_in(site, &site.state, name).await
}

/// As `thread_choice`, for a thread of the tree in `store`.
async fn thread_choice_in(site: &Arc<Site>, store: &Path, name: &str) -> crate::catalog::TurnPick {
    let mut own =
        task::choice(store, name).unwrap_or_else(|| crate::catalog::main_choice(&site.state));
    // Held to its harness's own default: the picker shows which that is.
    if own.pick.model.is_empty()
        && let Some(o) = offers(site)
            .await
            .into_iter()
            .find(|o| o.harness == own.pick.harness && o.default)
    {
        own.pick.model = o.model;
    }
    let grant = crate::access::grant(&own.pick.harness, &site.state, &own.picks());
    own.access = Some(grant.mode);
    own.sandbox = grant.sandbox;
    own
}

fn choice_json(c: &crate::catalog::TurnPick) -> Value {
    serde_json::json!({
        "pick": c.pick,
        "effort": c.effort.clone().unwrap_or_default(),
        "access": c.access,
        "sandbox": c.sandbox,
    })
}

async fn work_choice(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(choice_json(&thread_choice(&site, &name).await)).into_response()
}

/// What changes of a thread's choice; the rest stays. Empty thinking is the
/// harness's own default.
#[derive(Deserialize)]
struct Choose {
    #[serde(default)]
    pick: Option<crate::catalog::Pick>,
    #[serde(default)]
    effort: Option<String>,
    #[serde(flatten)]
    picks: crate::access::Picks,
}

/// Holds a thread to its own model, thinking and access from its next run
/// on, starting from what it runs on now, so a later change in the main
/// chat leaves it be.
async fn work_choose(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(change): Json<Choose>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    choose_in(&site, &state, &name, change).await
}

/// Changes the choice of thread `name` of the tree in `store`.
async fn choose_in(site: &Arc<Site>, store: &Path, name: &str, change: Choose) -> Response {
    let mut now = thread_choice_in(site, store, name).await;
    let was = now.pick.harness.clone();
    if let Some(pick) = change.pick {
        now.pick = pick;
    }
    if let Some(effort) = change.effort {
        now.effort = Some(effort).filter(|e| !e.is_empty());
    }
    // Only a pick of the person's holds it, not the default shown here.
    let held = task::choice(store, name)
        .map(|c| c.picks())
        .unwrap_or_default();
    let held = if now.pick.harness == was {
        held
    } else {
        crate::access::carried(&now.pick.harness, &site.state, held)
    };
    let changed = match crate::access::checked(&now.pick.harness, &site.state, change.picks) {
        Ok(changed) => changed,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    now.access = changed.access.or(held.access);
    now.sandbox = changed.sandbox.or(held.sandbox);
    let offers = offers(site).await;
    let Some(offer) = offers
        .iter()
        .find(|o| o.harness == now.pick.harness && o.model == now.pick.model)
    else {
        return (StatusCode::BAD_REQUEST, "That model is not on offer.").into_response();
    };
    // An effort the new model doesn't take goes back to the default.
    now.effort = now.effort.filter(|e| offer.efforts.contains(e));
    match task::choose(store, name, &now) {
        Ok(()) => {
            let grant = crate::access::grant(&now.pick.harness, &site.state, &now.picks());
            // Its Claude turn under way takes both at its next request; a
            // move to another harness waits for its next run.
            if now.pick.harness == "claude" && was == "claude" {
                crate::model::switch(name, Some(&now.pick.model), Some(&grant.mode));
            }
            now.access = Some(grant.mode);
            now.sandbox = grant.sandbox;
            Json(choice_json(&now)).into_response()
        }
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// Archives a finished thread: off the list, never deleted.
async fn work_archive(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match task::archive(&site.state, &name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::CONFLICT, e).into_response(),
    }
}

/// Merges a thread into the main chat's memory and archives it; answers
/// what landed.
async fn work_merge(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (state, h) = (site.state.clone(), site.hippo.clone());
    let merged =
        tokio::task::spawn_blocking(move || task::merge(&state, &name, |id, n| h.merge(id, n)))
            .await;
    match merged {
        Ok(Ok(said)) => said.into_response(),
        Ok(Err(e)) => (StatusCode::CONFLICT, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Brings an archived thread back onto the list.
async fn work_restore(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match task::restore(&site.state, &name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::CONFLICT, e).into_response(),
    }
}

/// A nap's wake time, in seconds since the epoch.
#[derive(Deserialize)]
struct Nap {
    until: u64,
}

#[derive(Deserialize)]
struct Start {
    /// The person's own words, which are the worker's brief.
    brief: String,
    /// Files sent ahead to /api/upload, by the ids they answered.
    #[serde(default)]
    files: Vec<String>,
    /// What the Tasks sheet's pickers chose, as a thread's choice is
    /// changed; what is left out stays the Workers default.
    #[serde(default)]
    pick: Option<crate::catalog::Pick>,
    #[serde(default)]
    effort: Option<String>,
    #[serde(flatten)]
    picks: crate::access::Picks,
}

/// The person starts a worker from Tasks, with their own words for its
/// brief, not through the main chat. It forks the main chat's memory as any
/// worker does and reports to the person on its card; the main chat gets a
/// note that it exists, and reads its report in memory when it needs it.
/// Deploys and the like still need the person's word in the main chat.
async fn work_start(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(start): Json<Start>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let brief = start.brief.trim().to_owned();
    if brief.is_empty() && start.files.is_empty() {
        return (StatusCode::BAD_REQUEST, "Say what the worker is to do.").into_response();
    }
    if start.files.len() > FILES_MAX {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let choice = match work_start_choice(&site, &start).await {
        Ok(c) => c,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let mut got = Vec::new();
    for id in &start.files {
        let Some((_, file)) = site.uploads.lock().unwrap().remove(id) else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        match site.web.fetch(&file).await {
            Ok(f) => got.push(f),
            Err(e) => return (StatusCode::BAD_GATEWAY, e).into_response(),
        }
    }
    let (state, hippo) = (site.state.clone(), site.hippo.clone());
    let started = tokio::task::spawn_blocking(move || {
        let taken = crate::take(&brief, keep(&state, got));
        let words = format!("{}{}", taken.said, taken.inline);
        let id = task::create(&state, &words, &choice, false, None, None)?;
        // Its first run takes them, as it takes files told later.
        task::attach(&state, &id, &taken.files)?;
        task::answer_the_person(&state, &id)?;
        // Without hippo it starts from its words alone, as a spawn does.
        if let Err(e) = hippo.fork(&id, None) {
            eprintln!("libri: agent {id}: {e}");
        }
        task::start(&state, &id)?;
        let name = task::shown(&state, &id);
        let line = brief.lines().next().unwrap_or_default();
        let _ = hippo.save(
            "note",
            &format!("(Tasks) The person started {name} themselves, without you: {line}"),
        );
        Ok::<_, String>((id, name))
    })
    .await;
    match started {
        Ok(Ok((id, name))) => Json(serde_json::json!({ "id": id, "name": name })).into_response(),
        Ok(Err(e)) => (StatusCode::CONFLICT, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// What a worker started from Tasks runs on: the Workers default, changed
/// by what the sheet's pickers chose, checked as `work_choose` checks a
/// thread's. A pick of another harness drops access the default held.
async fn work_start_choice(
    site: &Arc<Site>,
    start: &Start,
) -> Result<crate::catalog::TurnPick, String> {
    let mut choice = {
        let state = site.state.clone();
        tokio::task::spawn_blocking(move || crate::catalog::worker_choice(&state, None, None))
            .await
            .map_err(|e| e.to_string())??
            .0
    };
    if start.pick.is_none() && start.effort.is_none() && start.picks == Default::default() {
        return Ok(choice);
    }
    let was = choice.pick.harness.clone();
    if let Some(pick) = &start.pick {
        choice.pick = pick.clone();
    }
    if let Some(effort) = &start.effort {
        choice.effort = Some(effort.clone()).filter(|e| !e.is_empty());
    }
    let held = if choice.pick.harness == was {
        choice.picks()
    } else {
        crate::access::carried(&choice.pick.harness, &site.state, choice.picks())
    };
    let changed = crate::access::checked(&choice.pick.harness, &site.state, start.picks.clone())?;
    choice.access = changed.access.or(held.access);
    choice.sandbox = changed.sandbox.or(held.sandbox);
    let offers = offers(site).await;
    let offer = offers
        .iter()
        .find(|o| o.harness == choice.pick.harness && o.model == choice.pick.model)
        .ok_or("That model is not on offer.")?;
    choice.effort = choice.effort.filter(|e| offer.efforts.contains(e));
    Ok(choice)
}

/// What a worker started from Tasks would run on, for the sheet's pickers
/// to start from.
async fn work_default_choice(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    let chosen =
        tokio::task::spawn_blocking(move || crate::catalog::worker_choice(&state, None, None))
            .await;
    let mut choice = match chosen {
        Ok(Ok((c, _))) => c,
        Ok(Err(e)) => return (StatusCode::CONFLICT, e).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    if choice.pick.model.is_empty()
        && let Some(o) = offers(&site)
            .await
            .into_iter()
            .find(|o| o.harness == choice.pick.harness && o.default)
    {
        choice.pick.model = o.model;
    }
    let grant = crate::access::grant(&choice.pick.harness, &site.state, &choice.picks());
    choice.access = Some(grant.mode);
    choice.sandbox = grant.sandbox;
    Json(choice_json(&choice)).into_response()
}

/// The Library: the published pages, the latest first, and the lists.
async fn library(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (state, pages) = (site.state.clone(), site.pages.clone());
    // A page published since the last look gets its still now.
    site.stills.notify_one();
    let read = tokio::task::spawn_blocking(move || {
        let lists = crate::library::lists(&state)?;
        Ok::<_, String>(serde_json::json!({
            "pages": crate::library::pages(&pages),
            "lists": lists,
            "stills": crate::snapshot::on(),
        }))
    })
    .await;
    match read {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct Version {
    v: Option<u64>,
}

/// A page's still, as the Library's card shows it. Asked for at a version
/// (`?v=` its `changed`), the image never changes, so it is kept as long as
/// the browser likes; a version with no still yet answers 404, and the card
/// keeps its placeholder until the Library is read again.
async fn snapshot(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
    axum::extract::Query(Version { v }): axum::extract::Query<Version>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let (state, pages) = (site.state.clone(), site.pages.clone());
    let read = tokio::task::spawn_blocking(move || {
        let changed = match v {
            Some(v) => v,
            None => {
                crate::library::pages(&pages)
                    .into_iter()
                    .find(|p| p.name == name)?
                    .changed
            }
        };
        fs::read(crate::snapshot::file(&state, &name, changed)).ok()
    })
    .await;
    match read {
        Ok(Some(image)) => (
            [
                (header::CONTENT_TYPE, "image/webp"),
                (
                    header::CACHE_CONTROL,
                    if v.is_some() {
                        "private, max-age=31536000, immutable"
                    } else {
                        "no-cache"
                    },
                ),
            ],
            image,
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The watches and reminders, soonest first.
async fn watches(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    match tokio::task::spawn_blocking(move || crate::library::watches(&state)).await {
        Ok(Ok(w)) => Json(serde_json::json!({ "watches": w })).into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Stops and forgets a watch, with no model call.
async fn watch_delete(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<u64>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    match tokio::task::spawn_blocking(move || crate::library::delete(&state, id)).await {
        Ok(Ok(true)) => StatusCode::NO_CONTENT.into_response(),
        Ok(Ok(false)) => StatusCode::NOT_FOUND.into_response(),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Moves a timed watch's next firing later, with no model call.
async fn watch_snooze(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<u64>,
    Json(nap): Json<Nap>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    match tokio::task::spawn_blocking(move || crate::library::snooze(&state, id, nap.until)).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(e)) => (StatusCode::CONFLICT, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// Puts a thread down for a nap: out of Waiting until it wakes.
async fn work_nap(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(nap): Json<Nap>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match task::nap(&site.state, &name, nap.until) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::CONFLICT, e).into_response(),
    }
}

/// Wakes a napping thread now, back into Waiting, or cancels the nap set on
/// one at work.
async fn work_wake(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match task::wake(&site.state, &name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// Marks a thread read, on every device.
async fn work_read(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match task::mark_read(&site.state, &name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// Clears a Waiting card, on every device: its log read and what it asked of
/// the person set aside. Opening a thread only reads it; a question read is
/// not a question answered.
async fn work_clear(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match task::clear(&site.state, &name) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// What a scratchpad's route takes in a body, as nginx does on the chat's
/// routes.
const SCRATCHPAD_BODY_MAX: usize = 100 * 1024 * 1024;

/// What a scratchpad's turn runs on: Temp's own pick, or the main chat's, and
/// Temp's own access.
async fn scratchpad_choice(site: &Arc<Site>) -> crate::catalog::TurnPick {
    let offers = offers(site).await;
    let mut choice = match temp_current(site, &offers) {
        Some(pick) => crate::catalog::TurnPick {
            pick,
            effort: crate::catalog::TEMP_EFFORT.read(&site.state),
            access: None,
            sandbox: None,
        },
        None => crate::catalog::main_choice(&site.state),
    };
    let held = crate::access::temp_picks(&site.state, &choice.pick.harness);
    choice.access = held.access;
    choice.sandbox = held.sandbox;
    choice
}

/// The scratchpad as every client reads it, page and app alike: its log,
/// whether a run is at work and what that run is at, the words waiting for
/// it, what it asks, and the memory switch, which holds with none open.
fn scratchpad_view(
    state: &Path,
    open: Option<String>,
    run: &ScratchpadRun,
    asks: Vec<Value>,
) -> Value {
    let store = scratchpad::store(state);
    let (text, working, queued) = match &open {
        Some(id) => (
            task::log(&store, id)
                .map(|l| task::chat_keyed(&l))
                .unwrap_or_default(),
            scratchpad::working(state, id),
            task::told(&store, id)
                .into_iter()
                .map(|(_, told)| told.text)
                .collect(),
        ),
        None => (String::new(), false, Vec::new()),
    };
    serde_json::json!({
        "id": open,
        "text": text,
        "working": working,
        "words": if working { run.words.as_str() } else { "" },
        "thinking": if working { run.thinking.as_str() } else { "" },
        "stopping": working && run.stopping,
        "queued": queued,
        "memory": scratchpad::memory(state),
        "asks": asks,
    })
}

/// Which scratchpad a request is about: `?id=` names one; none, the current.
#[derive(Deserialize)]
struct TempAt {
    id: Option<String>,
}

async fn scratchpad_read(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Query(at): axum::extract::Query<TempAt>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let open = scratchpad::named(&site.state, at.id.as_deref());
    let (run, asks) = match &open {
        Some(id) => (site.web.scratchpad_run(id), site.web.scratchpad_asks(id)),
        None => (Arc::default(), Vec::new()),
    };
    let view = scratchpad_view(&site.state, open, &run.lock().unwrap(), asks);
    Json(view).into_response()
}

/// A new scratchpad beside any open, for a Temp tile opened beside one: its ID.
async fn scratchpad_new(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = site.state.clone();
    match tokio::task::spawn_blocking(move || scratchpad::open_new(&state)).await {
        Ok(Ok(id)) => {
            site.web.scratchpad_changed();
            Json(serde_json::json!({ "id": id })).into_response()
        }
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// The person's words to the scratchpad, opening one if none is. Under a run
/// they wait for its next step, as words on a worker's page do. With the
/// memory off an attachment goes to the model alone, not the files.
async fn scratchpad_tell(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Query(at): axum::extract::Query<TempAt>,
    Json(Tell { text, files }): Json<Tell>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if text.trim().is_empty() && files.is_empty() || files.len() > FILES_MAX {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let mut got = Vec::new();
    for id in &files {
        let Some((_, file)) = site.uploads.lock().unwrap().remove(id) else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        match site.web.fetch(&file).await {
            Ok(f) => got.push(f),
            Err(e) => return (StatusCode::BAD_GATEWAY, e).into_response(),
        }
    }
    let choice = scratchpad_choice(&site).await;
    let state = site.state.clone();
    let kind = crate::origin::of(&headers);
    let told = tokio::task::spawn_blocking(move || {
        let files = if scratchpad::memory(&state) {
            keep(&state, got)
        } else {
            got.into_iter().map(|f| (f.name, f.photo, f.read)).collect()
        };
        let taken = crate::take(text.trim(), files);
        // A Temp named is that one, closed meanwhile or not; none, the
        // current, opened if none is.
        let id = match at.id {
            Some(id) if scratchpad::exists(&state, &id) => id,
            Some(_) => return Err("That Temp was closed.".to_owned()),
            None => scratchpad::open(&state)?,
        };
        let store = scratchpad::store(&state);
        task::choose(&store, &id, &choice)?;
        task::attach(&store, &id, &taken.files)?;
        // Each Temp chat is its own: a new one hears the device afresh.
        let from = crate::origin::mark(&state, &format!("temp-{id}"), kind);
        task::tell(
            &store,
            &id,
            &format!("{from}{}{}", taken.said, taken.inline),
        )
    })
    .await;
    match told {
        Ok(Ok(_)) => {
            site.scratchpad.notify_one();
            site.web.scratchpad_changed();
            StatusCode::ACCEPTED.into_response()
        }
        Ok(Err(e)) => (StatusCode::CONFLICT, e).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Stops the scratchpad's run; what it said so far stays in the thread.
async fn scratchpad_stop(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Query(at): axum::extract::Query<TempAt>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(id) = scratchpad::named(&site.state, at.id.as_deref()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match task::cancel(&scratchpad::store(&site.state), &id) {
        Ok(()) => {
            site.web.scratchpad_run(&id).lock().unwrap().stopping = true;
            site.scratchpad.notify_one();
            site.web.scratchpad_changed();
            StatusCode::ACCEPTED.into_response()
        }
        Err(e) => (StatusCode::CONFLICT, e).into_response(),
    }
}

/// Removes a scratchpad, on New, when its tile closes and once promoted:
/// the one named, or with none the current.
async fn scratchpad_close(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Query(at): axum::extract::Query<TempAt>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match &at.id {
        Some(id) => {
            scratchpad::close_one(&site.state, id);
            site.web.scratchpad_gone(id);
        }
        None => scratchpad::close(&site.state),
    }
    site.scratchpad.notify_one();
    site.web.scratchpad_changed();
    StatusCode::NO_CONTENT.into_response()
}

/// Moves the scratchpad into the main chat as the talk it was: each
/// message joins the history as a main-chat turn keeps it, so the page draws
/// it as if it had happened there and the next turn reads it in the view.
/// No turn starts: nothing new was said. Refused while either chat is at
/// work, which would cut into what that run is still keeping.
async fn scratchpad_promote(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Query(at): axum::extract::Query<TempAt>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(id) = scratchpad::named(&site.state, at.id.as_deref()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if scratchpad::working(&site.state, &id) {
        return (StatusCode::CONFLICT, "Temp is still answering.").into_response();
    }
    if site.web.typing.load(Ordering::Relaxed) {
        return (StatusCode::CONFLICT, "The main chat is answering.").into_response();
    }
    let (state, hippo) = (site.state.clone(), site.hippo.clone());
    let moved = tokio::task::spawn_blocking(move || {
        let log = task::log(&scratchpad::store(&state), &id)?;
        for (key, kind, text) in scratchpad::promoted(&log) {
            hippo.save_as(&key, kind, &text)?;
        }
        scratchpad::close_one(&state, &id);
        Ok::<_, String>(())
    })
    .await;
    match moved {
        Ok(Ok(())) => {
            site.scratchpad.notify_one();
            site.web.scratchpad_changed();
            site.web.show(Shown::History);
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_GATEWAY, e).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[derive(Deserialize)]
struct MemorySwitch {
    on: bool,
}

/// Sets the memory switch, for the next turn on: a run under way keeps what
/// it began with.
async fn scratchpad_memory(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(MemorySwitch { on }): Json<MemorySwitch>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match scratchpad::set_memory(&site.state, on) {
        Ok(()) => {
            site.web.scratchpad_changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// The named memories, each with its own instructions, for the Agents pane
/// and the memory picker; the main memory is every page's own.
async fn memories_listed(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = &site.state;
    let all: Vec<Value> = crate::memories::list(state)
        .into_iter()
        .map(|m| {
            // Its chat's state, for a tree that draws the memory without
            // reading its whole chat (the Android Workers tree).
            let thread = crate::memories::thread(&m.id);
            let working = task::at_work(&crate::memories::store(state, &m.id), &thread);
            let asking = !site
                .web
                .asks_under(&format!("{MEMORY_ASK}{thread}:"))
                .is_empty();
            serde_json::json!({
                "id": m.id,
                "name": m.name,
                "instructions": crate::memories::instructions(state, &m.id),
                "working": working,
                "asking": asking,
            })
        })
        .collect();
    Json(serde_json::json!({ "memories": all })).into_response()
}

#[derive(Deserialize)]
struct MemoryNamed {
    name: String,
}

async fn memory_add(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Json(MemoryNamed { name }): Json<MemoryNamed>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match crate::memories::add(&site.state, &name) {
        Ok(m) => Json(m).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// What changes of a memory; the rest stays.
#[derive(Deserialize)]
struct MemoryChange {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    instructions: Option<String>,
}

async fn memory_change(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(change): Json<MemoryChange>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = &site.state;
    if !crate::memories::exists(state, &id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Some(text) = &change.instructions
        && let Err(e) = crate::memories::set_instructions(state, &id, text)
    {
        return (StatusCode::BAD_REQUEST, e).into_response();
    }
    if let Some(name) = &change.name
        && let Err(e) = crate::memories::rename(state, &id, name)
    {
        return (StatusCode::BAD_REQUEST, e).into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

/// Deletes a memory and all of it, its chat stopped first.
async fn memory_delete(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !crate::memories::exists(&site.state, &id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let thread = crate::memories::thread(&id);
    let _ = task::cancel(&crate::memories::store(&site.state, &id), &thread);
    site.memories.close(&id);
    match crate::memories::delete(&site.state, &id) {
        Ok(()) => {
            site.memories_wake.notify_one();
            site.web.memory_changed(&thread);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// A named memory's chat as every client reads it, as `scratchpad_view`.
async fn memory_chat_read(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let state = &site.state;
    if !crate::memories::exists(state, &id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let store = crate::memories::store(state, &id);
    let thread = crate::memories::thread(&id);
    let working = task::at_work(&store, &thread);
    let run = site.web.memory_run(&thread);
    let run = run.lock().unwrap();
    Json(serde_json::json!({
        "id": thread,
        "text": task::log(&store, &thread).map(|l| task::chat_keyed(&l)).unwrap_or_default(),
        "working": working,
        "words": if working { run.words.as_str() } else { "" },
        "thinking": if working { run.thinking.as_str() } else { "" },
        "stopping": working && run.stopping,
        "queued": task::told(&store, &thread)
            .into_iter()
            .map(|(_, told)| told.text)
            .collect::<Vec<_>>(),
        "asks": site.web.asks_under(&format!("{MEMORY_ASK}{thread}:")),
    }))
    .into_response()
}

/// The person's words to a named memory's chat. Under a run they wait for
/// its next step, as Temp's do; attachments go to the files.
async fn memory_chat_tell(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(Tell { text, files }): Json<Tell>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !crate::memories::exists(&site.state, &id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if text.trim().is_empty() && files.is_empty() || files.len() > FILES_MAX {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let mut got = Vec::new();
    for f in &files {
        let Some((_, file)) = site.uploads.lock().unwrap().remove(f) else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        match site.web.fetch(&file).await {
            Ok(f) => got.push(f),
            Err(e) => return (StatusCode::BAD_GATEWAY, e).into_response(),
        }
    }
    let store = crate::memories::store(&site.state, &id);
    let thread = crate::memories::thread(&id);
    let choice = thread_choice_in(&site, &store, &thread).await;
    let state = site.state.clone();
    let kind = crate::origin::of(&headers);
    let told = tokio::task::spawn_blocking(move || {
        let taken = crate::take(text.trim(), keep(&state, got));
        task::choose(&store, &thread, &choice)?;
        task::attach(&store, &thread, &taken.files)?;
        let from = crate::origin::mark(&state, &format!("memory-{thread}"), kind);
        task::tell(
            &store,
            &thread,
            &format!("{from}{}{}", taken.said, taken.inline),
        )
    })
    .await;
    match told {
        Ok(Ok(_)) => {
            site.memories_wake.notify_one();
            site.web.memory_changed(&crate::memories::thread(&id));
            StatusCode::ACCEPTED.into_response()
        }
        Ok(Err(e)) => (StatusCode::CONFLICT, e).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn memory_chat_stop(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !crate::memories::exists(&site.state, &id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let thread = crate::memories::thread(&id);
    match task::cancel(&crate::memories::store(&site.state, &id), &thread) {
        Ok(()) => {
            site.web.memory_run(&thread).lock().unwrap().stopping = true;
            site.memories_wake.notify_one();
            site.web.memory_changed(&thread);
            StatusCode::ACCEPTED.into_response()
        }
        Err(e) => (StatusCode::CONFLICT, e).into_response(),
    }
}

async fn memory_choice(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !crate::memories::exists(&site.state, &id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let store = crate::memories::store(&site.state, &id);
    let choice = thread_choice_in(&site, &store, &crate::memories::thread(&id)).await;
    Json(choice_json(&choice)).into_response()
}

async fn memory_choose(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(change): Json<Choose>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !crate::memories::exists(&site.state, &id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let store = crate::memories::store(&site.state, &id);
    choose_in(&site, &store, &crate::memories::thread(&id), change).await
}

/// Stops a subagent on the next tick, as cancel_task does.
async fn agent_stop(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match task::cancel(&site.state, &name) {
        Ok(()) => StatusCode::ACCEPTED.into_response(),
        // A subagent of a Claude turn under way, by the call that started
        // it: the CLI stops it and the turn goes on.
        Err(_) if crate::model::stop_task(&name) => StatusCode::ACCEPTED.into_response(),
        Err(e) => (StatusCode::CONFLICT, e).into_response(),
    }
}

#[derive(Deserialize)]
struct Tell {
    text: String,
    /// Files sent ahead to /api/upload, by the ids they answered.
    #[serde(default)]
    files: Vec<String>,
}

/// The person's words to a subagent, typed on its page: it reads them as
/// Libri's would come, marked as theirs. They stay on that page, and
/// the main chat learns of them from the subagent's report.
async fn agent_tell(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(Tell { text, files }): Json<Tell>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if text.trim().is_empty() && files.is_empty() || files.len() > FILES_MAX {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let mut got = Vec::new();
    for id in &files {
        let Some((_, file)) = site.uploads.lock().unwrap().remove(id) else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        match site.web.fetch(&file).await {
            Ok(f) => got.push(f),
            Err(e) => return (StatusCode::BAD_GATEWAY, e).into_response(),
        }
    }
    let state = site.state.clone();
    let kind = crate::origin::of(&headers);
    let told = {
        let (name, text) = (name.clone(), text.clone());
        tokio::task::spawn_blocking(move || {
            let taken = crate::take(text.trim(), keep(&state, got));
            task::attach(&state, &name, &taken.files)?;
            let from = crate::origin::mark(&state, &format!("worker-{name}"), kind);
            task::tell(
                &state,
                &name,
                &format!(
                    "(From the person, typed on your page) {from}{}{}",
                    taken.said, taken.inline
                ),
            )
        })
        .await
    };
    match told {
        Ok(Ok(_)) => StatusCode::ACCEPTED.into_response(),
        Ok(Err(e)) => (StatusCode::NOT_FOUND, e).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_client_reads_what_the_run_is_at_and_the_words_waiting_for_it() {
        let state = std::env::temp_dir().join(format!("libri-view-{}", std::process::id()));
        let _ = fs::remove_dir_all(&state);
        fs::create_dir_all(&state).unwrap();
        let id = scratchpad::open(&state).unwrap();
        let store = scratchpad::store(&state);
        task::tell(&store, &id, "first").unwrap();
        assert!(task::take(&store, &id));
        task::tell(&store, &id, "second").unwrap();
        let run = ScratchpadRun {
            words: "So far".into(),
            thinking: "Hm".into(),
            stopping: true,
        };
        let v = scratchpad_view(&state, scratchpad::current(&state), &run, Vec::new());
        assert_eq!(v["id"], id.as_str());
        assert_eq!(v["queued"], serde_json::json!(["second"]));
        assert_eq!(
            (&v["words"], &v["thinking"], &v["stopping"]),
            (&"So far".into(), &"Hm".into(), &true.into())
        );
        assert!(v["text"].as_str().unwrap().contains("first"));
        assert!(!v["text"].as_str().unwrap().contains("second"));
        scratchpad::close(&state);
        let v = scratchpad_view(&state, scratchpad::current(&state), &run, Vec::new());
        assert_eq!(v["id"], Value::Null);
        assert_eq!((&v["words"], &v["stopping"]), (&"".into(), &false.into()));
        assert_eq!(v["queued"], serde_json::json!([]));
    }

    #[test]
    fn a_deploy_waits_for_workers_and_temp_as_well_as_the_turn() {
        let state = std::env::temp_dir().join(format!("libri-quiet-{}", std::process::id()));
        let _ = fs::remove_dir_all(&state);
        fs::create_dir_all(&state).unwrap();
        assert!(quiet(&state, false));
        assert!(!quiet(&state, true));
        let pick = crate::catalog::TurnPick {
            pick: crate::catalog::Pick {
                harness: "claude".into(),
                model: "haiku".into(),
            },
            effort: None,
            access: None,
            sandbox: None,
        };
        let worker = task::spawn(&state, "work", &pick).unwrap();
        assert!(!quiet(&state, false), "a queued worker was cut off");
        assert_eq!(
            busy(&state, true),
            ["the main turn".to_owned(), format!("worker {worker}")]
        );
        assert!(task::take(&state, &worker));
        assert!(!quiet(&state, false), "a running worker was cut off");
        task::cancel(&state, &worker).unwrap();
        assert!(task::cancelled(&state, &worker));
        task::settle(&state, &worker, "", task::Ending::Stopped);
        assert!(quiet(&state, false));
        let id = scratchpad::open(&state).unwrap();
        task::tell(&scratchpad::store(&state), &id, "a question").unwrap();
        assert!(!quiet(&state, false), "a Temp run was cut off");
        assert_eq!(busy(&state, false), ["Temp"]);
        fs::remove_dir_all(&state).unwrap();
    }

    #[test]
    fn a_card_gets_its_briefs_first_line_alone() {
        assert_eq!(
            brief_head("Fix the bug.\n\nThen the rest, at length."),
            "Fix the bug."
        );
        assert_eq!(
            brief_head("\n\n```rust\nfn main() {}\n```\nMore."),
            "\n\n```rust\nfn main() {}"
        );
        assert_eq!(brief_head("```\n```\n# Head\nbody"), "```\n```\n# Head");
        assert_eq!(brief_head("```rust code here\nnext"), "```rust code here");
        assert_eq!(brief_head("One line"), "One line");
        assert_eq!(brief_head(""), "");
    }

    #[test]
    fn typed_words_answer_a_question_and_refuse_a_tool() {
        let ask = |tool: &str, input: Value| SeatAsk {
            id: "a".into(),
            tool: tool.into(),
            input,
            about: None,
            always: false,
        };
        let q = serde_json::json!({"questions": [{"question": "Tea or coffee?"}, {"question": "Milk?"}]});
        let got = typed_answer(&ask("AskUserQuestion", q), "coffee, black").unwrap();
        assert_eq!(got["answers"]["Tea or coffee?"], "coffee, black");
        assert_eq!(got["answers"]["Milk?"], "coffee, black");
        assert!(typed_answer(&ask("Bash", serde_json::json!({"command": "ls"})), "no").is_none());
    }

    #[test]
    fn plain_reads_markdown_as_words() {
        let md = "## Weather\n\n**Cold** today, see [the map](https://x.y/m) ![radar](r.png).\n\n- Wear a `coat`\n1. Bring tea\n\n```sh\nrm -rf /\n```\n> quoted\n\n| Day | High |\n|---|---:|\n| Mon | 45 |";
        assert_eq!(
            super::plain(md),
            "Weather\nCold today, see the map .\nWear a coat\n1. Bring tea\nquoted\nDay, High\nMon, 45"
        );
    }

    #[tokio::test]
    async fn every_listener_hears_a_reading_whole_or_cut() {
        use futures_util::StreamExt;
        let (tx, _) = tokio::sync::watch::channel(super::Sofar::default());
        let early = super::follow(tx.subscribe());
        tx.send_modify(|s| s.mp3.extend_from_slice(b"ab"));
        let late = super::follow(tx.subscribe());
        let feed = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            tx.send_modify(|s| s.mp3.extend_from_slice(b"cd"));
            tx.send_modify(|s| s.end = Some(true));
        });
        for heard in [early, late] {
            let got: Vec<_> = heard.map(Result::unwrap).collect().await;
            assert_eq!(got.concat(), b"abcd");
        }
        feed.await.unwrap();
        let (tx, rx) = tokio::sync::watch::channel(super::Sofar::default());
        tx.send_modify(|s| {
            s.mp3.extend_from_slice(b"ab");
            s.end = Some(false);
        });
        let got: Vec<_> = super::follow(rx).collect().await;
        assert!(matches!(got.as_slice(), [Ok(_), Err(_)]));
    }

    #[test]
    fn the_session_cookie_is_read_among_others() {
        let token = "t0ken";
        let mut headers = axum::http::HeaderMap::new();
        let cookie = format!("theme=dark; {}={token}; x=y", super::COOKIE);
        headers.insert(axum::http::header::COOKIE, cookie.parse().unwrap());
        assert_eq!(super::session(&headers), Some(token));
        assert_eq!(super::session(&axum::http::HeaderMap::new()), None);
    }

    #[test]
    fn speech_key_tells_voices_apart() {
        assert_eq!(
            super::speech_key("leo", "hi"),
            super::speech_key("leo", "hi")
        );
        assert_ne!(
            super::speech_key("leo", "hi"),
            super::speech_key("ara", "hi")
        );
    }

    fn said(i: u64, kind: &str, text: &str) -> Said {
        Said {
            display: None,
            i,
            kind: kind.into(),
            text: text.into(),
            date: String::new(),
            src: None,
        }
    }

    #[test]
    fn lone_emoji_answers_become_reactions() {
        let out = lines(vec![
            said(1, "user", "nevermind"),
            said(2, "talk", "👍"),
            said(3, "tool", "x"),
            said(4, "talk", "👍"),
            said(5, "talk", "Done 👍"),
        ]);
        let got: Vec<_> = out
            .iter()
            .map(|l| (l.said.i, l.reaction.as_deref()))
            .collect();
        // The second 👍 has no unanswered message of the person's, so it stays.
        assert_eq!(got, [(1, Some("👍")), (3, None), (4, None), (5, None)]);
    }

    #[test]
    fn a_seats_work_comes_clipped() {
        let long = "é".repeat(HISTORY_STEP);
        let out = lines(vec![
            said(1, "user", "check"),
            said(2, "tool", r#"Bash {"command":"ls"}"#),
            said(3, "echo", &long),
            said(4, "work", "[Counter] COUNTED"),
            said(5, "talk", "Done."),
        ]);
        let kinds: Vec<_> = out.iter().map(|l| l.said.kind.as_str()).collect();
        assert_eq!(kinds, ["user", "tool", "echo", "work", "talk"]);
        let echo = &out[2].said.text;
        assert!(echo.len() <= HISTORY_STEP + '…'.len_utf8() && echo.ends_with('…'));
    }

    #[test]
    fn a_reply_keeps_its_quote_and_the_earlier_message() {
        let reply = crate::Quote {
            id: 2,
            text: "rain \"all\" day".into(),
        };
        let out = lines(vec![
            said(1, "user", "hi"),
            said(2, "talk", "rain \"all\" day"),
            said(
                3,
                "user",
                &reply.mark("why? ) (replying to message 9: \"x\") "),
            ),
            said(4, "user", "(replying to message x: \"y\") plain"),
        ]);
        let got: Vec<_> = out
            .iter()
            .map(|l| (l.said.i, l.said.text.as_str(), l.quote.clone()))
            .collect();
        assert_eq!(
            got,
            [
                (1, "hi", None),
                (2, "rain \"all\" day", None),
                (3, "why? ) (replying to message 9: \"x\") ", Some(reply)),
                (4, "(replying to message x: \"y\") plain", None),
            ]
        );
    }

    #[test]
    fn a_quote_is_one_line_cut_short() {
        let q = crate::Quote {
            id: 1,
            text: format!("a\n\n  b {}", "é".repeat(300)),
        }
        .tidied();
        assert!(q.text.starts_with("a b é"));
        assert_eq!(q.text.chars().count(), 201);
        assert!(q.text.ends_with('…'));
    }

    #[test]
    fn a_redo_replaces_the_last_exchange() {
        let out = lines(vec![
            said(1, "user", "hi"),
            said(2, "talk", "hello"),
            said(3, "user", "weather?"),
            said(4, "talk", "rain"),
            said(5, "note", "(routine) x"),
            said(6, "user", &format!("{}weather in Boston?", crate::EDITED)),
            said(7, "talk", "sun"),
            said(8, "user", &format!("{}weather in Boston?", crate::AGAIN)),
            said(9, "talk", "clouds"),
        ]);
        let got: Vec<_> = out
            .iter()
            .map(|l| (l.said.i, l.said.text.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                (1, "hi"),
                (2, "hello"),
                (8, "weather in Boston?"),
                (9, "clouds")
            ]
        );
    }

    #[test]
    fn answers_join_the_input() {
        let input = serde_json::json!({"questions": [{"question": "Tea?"}]});
        let mut answers = serde_json::Map::new();
        answers.insert("Tea?".into(), "Yes".into());
        let out = answered(&input, true, Some(answers)).unwrap();
        assert_eq!(out["answers"]["Tea?"], "Yes");
        assert_eq!(out["questions"], input["questions"]);
        assert_eq!(answered(&input, false, None), None);
        assert_eq!(answered(&input, true, None), Some(input));
    }

    #[test]
    fn the_page_carries_its_title_and_assistant() {
        let html = "<head><title>Libri</title></head>";
        assert_eq!(
            titled(html, "Hippo", "Ben"),
            "<head><title>Hippo</title><meta name=\"assistant\" content=\"Ben\" /></head>"
        );
        assert_eq!(
            titled(html, "<b>", "\"x"),
            "<head><title>b</title><meta name=\"assistant\" content=\"x\" /></head>"
        );
    }

    use super::*;

    #[test]
    fn a_turn_cut_short_by_a_restart_is_still_shown() {
        let dir = std::env::temp_dir().join(format!("libri-turn-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("turn.json");
        let web = Web::new();
        web.keep_turn_in(file.clone());
        web.sent("m1", "we need a proper pipeline", false);
        // The service stops here, the turn under way.
        let again = Web::new();
        again.keep_turn_in(file);
        let t = again.turn.lock().unwrap().clone();
        assert!(t.ended);
        assert!(
            t.shown
                .iter()
                .any(|e| e.to_string().contains("we need a proper pipeline"))
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The record's events of one kind, as (key, after).
    fn recorded(web: &Web, kind: &str) -> Vec<(Option<String>, String)> {
        let t = web.turn.lock().unwrap();
        t.shown
            .iter()
            .filter(|e| e["type"] == kind)
            .map(|e| {
                (
                    e["key"].as_str().map(str::to_owned),
                    e["after"].as_str().unwrap_or_default().to_owned(),
                )
            })
            .collect()
    }

    fn call(id: &str) -> Step {
        Step::Call {
            id: id.into(),
            name: "Bash".into(),
            input: Value::Null,
        }
    }

    fn result(id: &str) -> Step {
        Step::Result {
            id: id.into(),
            text: "done".into(),
            error: false,
        }
    }

    fn row(key: &str) -> Step {
        Step::Row { key: key.into() }
    }

    fn stored(key: &str) -> Step {
        Step::Stored {
            key: key.into(),
            heard: Vec::new(),
            row: None,
        }
    }

    #[test]
    fn the_record_lets_go_of_each_thing_as_hippo_gets_it() {
        let web = Web::new();
        web.step(stored("prompt"));
        web.step(row("words"));
        web.step(Step::Text {
            text: "Looking.".into(),
        });
        assert_eq!(
            recorded(&web, "TEXT_MESSAGE_CONTENT"),
            [(Some("words".into()), "prompt".into())]
        );
        web.step(row("call"));
        web.step(call("t1"));
        web.step(stored("words"));
        web.step(stored("call"));
        assert!(recorded(&web, "TEXT_MESSAGE_CONTENT").is_empty());
        // A call stays until its result does: a page needs its id for that.
        assert_eq!(recorded(&web, "TOOL_CALL_START").len(), 1);
        web.step(row("echo"));
        web.step(result("t1"));
        assert_eq!(recorded(&web, "TOOL_CALL_RESULT").len(), 1);
        web.step(stored("echo"));
        assert!(recorded(&web, "TOOL_CALL_START").is_empty());
        assert!(recorded(&web, "TOOL_CALL_RESULT").is_empty());
    }

    #[test]
    fn what_hippo_never_gets_keeps_its_place_until_the_turn_ends() {
        let web = Web::new();
        web.step(stored("prompt"));
        web.step(Step::Thinking {
            text: "Hmm.".into(),
        });
        web.step(row("call"));
        web.step(call("memory"));
        web.step(stored("call"));
        // A memory call's result is never a row.
        web.step(result("memory"));
        assert_eq!(
            recorded(&web, "REASONING_MESSAGE_CONTENT"),
            [(None, "prompt".into())]
        );
        assert_eq!(recorded(&web, "TOOL_CALL_RESULT"), [(None, "call".into())]);
        web.show(Shown::Typing { on: false });
        assert!(recorded(&web, "REASONING_MESSAGE_CONTENT").is_empty());
        assert!(recorded(&web, "TOOL_CALL_RESULT").is_empty());
    }

    #[test]
    fn the_persons_words_wait_for_their_row_past_the_turn_they_came_in() {
        let web = Web::new();
        web.sent("first", "one", false);
        web.sent("second", "two", false);
        web.show(Shown::Typing { on: false });
        assert_eq!(recorded(&web, "TEXT_MESSAGE_CHUNK").len(), 2);
        // Messages waiting fold into one turn under the last one's id.
        web.stored("prompt", &["second".into()], None);
        assert!(recorded(&web, "TEXT_MESSAGE_CHUNK").is_empty());
    }

    #[test]
    fn the_pulse_gives_the_hosts_clock_and_the_newest_event() {
        let web = Web::new();
        let before = unix_ms();
        assert_eq!(pulse_data(&web)["seq"], 0);
        web.sent("m", "hello", false);
        web.step(Step::Thinking { text: "a".into() });
        let p = pulse_data(&web);
        let newest = web.turn.lock().unwrap().shown.last().unwrap()["seq"].clone();
        assert!(newest.as_u64().unwrap() > 0);
        assert_eq!(p["seq"], newest);
        assert!(p["now"].as_u64().unwrap() >= before);
    }

    #[test]
    fn words_never_stored_leave_with_their_second_turn() {
        let web = Web::new();
        web.sent("m", "stop", false);
        web.show(Shown::Typing { on: false });
        web.step(Step::Thinking { text: "a".into() });
        assert_eq!(recorded(&web, "TEXT_MESSAGE_CHUNK").len(), 1);
        web.show(Shown::Typing { on: false });
        web.step(Step::Thinking { text: "b".into() });
        assert!(recorded(&web, "TEXT_MESSAGE_CHUNK").is_empty());
    }

    #[test]
    fn a_said_answer_goes_to_hippo_not_the_record() {
        let web = Web::new();
        web.show(Shown::Said {
            id: "a".into(),
            text: "Done.".into(),
            plain: false,
            aside: false,
        });
        assert!(web.turn.lock().unwrap().shown.is_empty());
    }

    #[test]
    fn a_picture_or_file_sent_mid_turn_goes_to_hippo_not_the_record() {
        let web = Web::new();
        web.step(stored("prompt"));
        web.step(row("call"));
        web.step(call("t1"));
        web.show(Shown::Picture {
            name: "chart.png".into(),
        });
        web.show(Shown::File {
            name: "report.pdf".into(),
        });
        let t = web.turn.lock().unwrap();
        assert!(
            t.shown.iter().all(|e| e["type"] != "CUSTOM"),
            "{:?}",
            t.shown
        );
    }

    #[test]
    fn words_a_restart_cut_off_become_their_row() {
        let dir = std::env::temp_dir().join(format!("libri-cut-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let web = Web::new();
        web.keep_turn_in(dir.join("turn.json"));
        web.step(row("words"));
        web.step(Step::Text {
            text: "half a ".into(),
        });
        web.step(row("words"));
        web.step(Step::Text {
            text: "thought".into(),
        });
        web.step(Step::Thinking {
            text: "more".into(),
        });
        // Saved at a step's end; the service stops here.
        let again = Web::new();
        again.keep_turn_in(dir.join("turn.json"));
        assert_eq!(recorded(&again, "TEXT_MESSAGE_CONTENT").len(), 2);
        let spool = crate::hippo::Spool::new(dir.join("spool")).unwrap();
        let hippo = Hippo {
            sock: dir.join("no-hippo"),
            chat: None,
            spool: Some(Arc::new(spool)),
        };
        again.keep_cut_words(&hippo);
        assert!(recorded(&again, "TEXT_MESSAGE_CONTENT").is_empty());
        let kept = hippo.tail(9).unwrap();
        assert_eq!(kept[0].text, "half a thought");
        assert_eq!(kept[0].src.as_ref().unwrap().e, "words");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn agents_are_shown_only_when_they_change() {
        let web = Web::new();
        let mut seen = web.shown.subscribe();
        let ada = || ("Ada".to_owned(), "book Cure".to_owned());
        web.agents(vec![ada()]);
        web.aside().agents(vec![ada()]);
        web.agents(vec![ada(), ("Basil".into(), "price flights".into())]);
        web.agents(Vec::new());
        let mut shown = Vec::new();
        while let Ok(Shown::Agents { agents }) = seen.try_recv() {
            shown.push(agents.len());
        }
        assert_eq!(shown, [1, 2, 0]);
    }

    #[test]
    fn a_turns_words_keep_as_one_step_between_calls() {
        let web = Web::new();
        let text = |t: &str| Shown::Step {
            step: Step::Text { text: t.into() },
            agent: None,
        };
        web.sent("web-1", "how big?", false);
        web.show(Shown::Typing { on: true });
        for t in ["Let me ", "look."] {
            web.show(text(t));
        }
        web.show(Shown::Step {
            step: Step::Call {
                id: "t1".into(),
                name: "Bash".into(),
                input: Value::Null,
            },
            agent: None,
        });
        for t in ["Big", "."] {
            web.show(text(t));
        }
        let turn = web.turn.lock().unwrap().clone();
        let words: Vec<_> = turn
            .shown
            .iter()
            .filter(|e| e["type"] == "TEXT_MESSAGE_CONTENT")
            .filter_map(|e| e["delta"].as_str())
            .collect();
        // The person's words, the run's start, the first words' start,
        // content and end, the call's start, arguments and end, the next
        // words' start and content.
        assert_eq!(
            (turn.shown.len(), words),
            (10, vec!["Let me look.", "Big."])
        );
    }
}
