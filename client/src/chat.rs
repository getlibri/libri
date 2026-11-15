//! The conversation as a client shows it: the history from hippo, then
//! what the host streams, folded into messages. A port of the web page's
//! fold (libri/site/src/chat.ts `fromHistory`, `apply`, `sync`); where the
//! two differ, the page is right and this follows it.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use crate::wire::{Event, Quote, Said, TaskNow, TurnRecord};

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
    /// A note: news from outside the talk (a routine, watch or a
    /// message passed on), or the client's own word.
    System,
}

/// The call a card is drawn for (apps.ts cardCall): the call itself, or,
/// for `show_card`, the tool it names, with its arguments, `shown`.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct DrawnCard {
    pub name: String,
    /// The arguments it was called with, as JSON.
    pub input: String,
    /// The model asked to show it: drawn from a fresh call.
    pub shown: bool,
}

#[uniffi::export]
pub fn drawn_card(name: String, arguments: Option<String>) -> DrawnCard {
    let input: Value = arguments
        .as_deref()
        .and_then(|a| serde_json::from_str(a).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    if matches!(name.as_str(), "mcp__libri__show_card" | "libri_show_card")
        && let Some(tool) = input["tool"].as_str()
    {
        let args = Some(&input["arguments"])
            .filter(|a| a.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        return DrawnCard {
            name: tool.to_owned(),
            input: args.to_string(),
            shown: true,
        };
    }
    DrawnCard {
        name,
        input: input.to_string(),
        shown: false,
    }
}

/// The assistant messages of turns the person's own words started, by id:
/// their calls' cards may open by themselves (App.tsx CallCard). A turn a
/// routine, a watch or a passed-on message started (a note), or one a
/// worker's report woke, shows its cards only on asking.
#[uniffi::export]
pub fn person_turns(messages: Vec<Message>) -> Vec<String> {
    let mut theirs = true;
    let mut out = Vec::new();
    for m in messages {
        match m.role {
            Role::User => theirs = true,
            Role::System => theirs = false,
            Role::Assistant => {
                if theirs && m.woke.is_none() {
                    out.push(m.id);
                }
            }
        }
    }
    out
}

/// A tool call on the way to an answer. Words said before it are a
/// message of their own, above the one the call opens.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// What it was asked, as a person reads it: the command or the file
    /// when it has one, else the whole input.
    pub input: String,
    /// Original tool arguments, before the compact label is selected.
    pub arguments: Option<String>,
    /// What came back, once it has.
    pub result: Option<String>,
    pub done: bool,
    pub error: bool,
    /// Rich display result from the stream or saved history, serialized as JSON.
    pub structured: Option<String>,
    pub thought: Option<String>,
    pub report: Option<String>,
    /// Heard as it ran, on this stream, not read back from the record: a
    /// card for it opens at once (App.tsx CallCard), a kept one waits.
    pub heard: bool,
}

impl ToolCall {
    fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        ToolCall {
            id: id.into(),
            name: name.into(),
            input: String::new(),
            arguments: None,
            result: None,
            done: false,
            error: false,
            structured: None,
            thought: None,
            report: None,
            heard: false,
        }
    }
}

/// A call's input as the person reads it (App.tsx `detail`): the command
/// or the file when it has one, else the input whole.
pub fn summary(input: &str) -> String {
    let Ok(Value::Object(o)) = serde_json::from_str::<Value>(input) else {
        return input.trim().to_owned();
    };
    for key in [
        "command",
        "file_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
    ] {
        if let Some(Value::String(v)) = o.get(key) {
            return v.clone();
        }
    }
    serde_json::to_string_pretty(&Value::Object(o)).unwrap_or_default()
}

/// What a call was about, as its row names it (App.tsx `gist`): the first
/// line of the first string among its arguments, or of its input when that is
/// no list of arguments.
#[uniffi::export]
pub fn gist(input: &str) -> String {
    let parsed = serde_json::from_str::<Value>(input);
    let text = match &parsed {
        Ok(Value::Object(o)) => o
            .values()
            .filter_map(Value::as_str)
            .find(|s| !s.trim().is_empty())
            .unwrap_or_default(),
        _ => input,
    };
    text.lines().next().unwrap_or_default().to_owned()
}

/// What a tell_agent call is called until its input has come whole.
const MESSAGE: &str = "Message";

/// Whether this is tell_agent, under any harness's name for it:
/// `mcp__libri__tell_agent` on Claude and Codex, `mcp__libri_tell_agent` on
/// omp, `libri_tell_agent` on OpenCode.
fn is_tell(name: &str) -> bool {
    name.strip_suffix("tell_agent")
        .is_some_and(|p| p.is_empty() || p.ends_with('_'))
}

/// A call's name from its first event, before there is any input to read.
fn started(name: &str) -> &str {
    if is_tell(name) { MESSAGE } else { name }
}

/// A call as the person reads it: its name and its input. Words sent to
/// another worker read as a message.
pub fn presented(name: &str, input: &str) -> (String, String) {
    if (name == MESSAGE || is_tell(name))
        && let Ok(v) = serde_json::from_str::<Value>(input)
        && let (Some(who), Some(words)) = (v["name"].as_str(), v["message"].as_str())
    {
        return (format!("Message to {who}"), words.trim().to_owned());
    }
    (name.to_owned(), summary(input))
}

/// One choice of a question the assistant asks.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct Choice {
    pub label: String,
    pub description: Option<String>,
}

/// One of AskUserQuestion's questions.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct Question {
    pub question: String,
    pub header: Option<String>,
    pub options: Vec<Choice>,
    pub multi: bool,
}

/// Replies the person may tap under the newest message (chat.ts
/// `ReplyOffer`): the model's offer_replies call, a tap on which sends the
/// reply quoting `quote`; or a question of AskUserQuestion's with a few
/// single answers, a tap on which answers `ask` (its id) and `question`.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct ReplyOffer {
    pub replies: Vec<String>,
    pub quote: Option<Quote>,
    pub ask: Option<String>,
    pub question: Option<String>,
}

/// Most replies a message offers: REPLIES_MAX in tools.rs.
const REPLIES_MAX: usize = 3;
/// What offer_replies answers once it took them (tools.rs): hippo keeps a
/// call's result but not whether it failed, so a refusal is told apart by it.
const SHOWN: &str = "Shown under your message.";

/// Whether a call is an offer_replies the chat shows as its replies, so its
/// own row would say the same twice (chat.ts `offerShown`); a refused or
/// failed one keeps its row, so the fault shows.
#[uniffi::export]
pub fn offer_shown(call: &ToolCall) -> bool {
    call.name.ends_with("offer_replies")
        && !call.error
        && call.result.as_deref().is_none_or(|r| r.trim() == SHOWN)
}

/// The replies on offer (chat.ts `offered`, then `askOffer`): the newest
/// message's, when it is the model's, finished and kept; else a waiting
/// question's. They go once the person answers or anything later is said.
#[uniffi::export]
pub fn offered(messages: &[Message], asks: &[Ask]) -> Option<ReplyOffer> {
    let by_message = messages
        .last()
        .filter(|m| m.role == Role::Assistant && !m.live)
        .and_then(|m| {
            let row = m.row?;
            let call = m.tools.iter().rev().find(|t| {
                t.name.ends_with("offer_replies")
                    && !t.error
                    && t.result.as_deref().map(str::trim) == Some(SHOWN)
            })?;
            let input: Value = serde_json::from_str(&call.input).ok()?;
            let replies: Vec<String> = input["options"]
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .filter(|o| !o.trim().is_empty())
                .take(REPLIES_MAX)
                .map(str::to_owned)
                .collect();
            let text = m
                .text
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or_default();
            (!replies.is_empty()).then(|| ReplyOffer {
                replies,
                quote: Some(Quote {
                    id: row,
                    text: text.to_owned(),
                }),
                ask: None,
                question: None,
            })
        });
    // A question's replies are drawn under the newest answer; with the
    // person's words below it, the question stays its card (App.tsx).
    let answer_last = messages.last().is_some_and(|m| m.role == Role::Assistant);
    by_message.or_else(|| {
        asks.iter().filter(|_| answer_last).find_map(|a| {
            let [q] = a.questions.as_slice() else {
                return None;
            };
            (a.tool == "AskUserQuestion"
                && !q.multi
                && (1..=REPLIES_MAX).contains(&q.options.len()))
            .then(|| ReplyOffer {
                replies: q.options.iter().map(|o| o.label.clone()).collect(),
                quote: None,
                ask: Some(a.id.clone()),
                question: Some(q.question.clone()),
            })
        })
    })
}

/// What the turn asks mid-way, as Claude Code would in a terminal: may
/// `tool` run (then `questions` is empty), or AskUserQuestion's questions.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct Ask {
    pub id: String,
    pub tool: String,
    /// The tool's input as the person reads it.
    pub detail: String,
    pub about: Option<String>,
    pub questions: Vec<Question>,
    /// A connection's tool that asks first: the card offers "Always allow".
    pub always: bool,
}

impl Ask {
    /// From an AG-UI interrupt, `{id, message, metadata: {tool, input}}`.
    pub(crate) fn of(v: &Value) -> Option<Ask> {
        let meta = v.get("metadata")?;
        let tool = meta.get("tool")?.as_str()?.to_owned();
        let input = meta.get("input").cloned().unwrap_or(Value::Null);
        let questions = if tool == "AskUserQuestion" {
            input
                .get("questions")
                .and_then(Value::as_array)
                .map(|qs| {
                    qs.iter()
                        .map(|q| Question {
                            question: q["question"].as_str().unwrap_or("").to_owned(),
                            header: q["header"].as_str().map(str::to_owned),
                            multi: q["multiSelect"].as_bool().unwrap_or(false),
                            options: q["options"]
                                .as_array()
                                .map(|os| {
                                    os.iter()
                                        .map(|o| Choice {
                                            label: o["label"].as_str().unwrap_or("").to_owned(),
                                            description: o["description"]
                                                .as_str()
                                                .map(str::to_owned),
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        Some(Ask {
            id: v.get("id")?.as_str()?.to_owned(),
            detail: summary(&input.to_string()),
            tool,
            about: v.get("message").and_then(Value::as_str).map(str::to_owned),
            questions,
            always: meta.get("always").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

/// The interrupts a RUN_FINISHED ended with.
fn asks_of(e: &Event) -> Vec<Ask> {
    e.outcome
        .as_ref()
        .and_then(|o| o.get("interrupts"))
        .and_then(Value::as_array)
        .map(|is| is.iter().filter_map(Ask::of).collect())
        .unwrap_or_default()
}

/// An act waiting for the person's word: today only a mail draft.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct Card {
    pub id: u64,
    pub account: Option<String>,
    pub to: String,
    pub subject: String,
    pub body: String,
    /// What went wrong carrying out the verdict, if it did.
    pub error: Option<String>,
    /// Temp drafted it: it shows there, not in the main chat.
    pub scratchpad: bool,
}

impl Card {
    /// From a pending act as /api/pending lists it.
    pub fn of(v: &Value) -> Option<Card> {
        let act = v.get("act")?;
        let s = |k: &str| act.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        Some(Card {
            id: v.get("id")?.as_u64()?,
            account: act
                .get("account")
                .and_then(Value::as_str)
                .map(str::to_owned),
            to: s("to"),
            subject: s("subject"),
            body: s("body"),
            error: None,
            scratchpad: v.get("scratchpad").and_then(Value::as_bool) == Some(true),
        })
    }
}

/// Where a message of the person's stands on its way to the host. Only
/// the client sets it; the host's own word that a message waits its turn
/// is `Message::waits_turn`.
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outbox {
    /// The host has it: it echoed it, took it, or the history holds it.
    /// Every message not the person's own is here too.
    Arrived,
    /// Posted; the host has not answered yet.
    Sending,
    /// The host could not be reached: sent, in order, once it answers.
    Held,
    /// The host refused it: the person may send it again or take it back.
    Refused,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct Message {
    pub id: String,
    pub role: Role,
    pub text: String,
    /// The model's thinking since its last call, as it streams.
    pub thinking: String,
    /// Words preceding a tool call, hidden in answer-only presentation.
    pub progress: bool,
    pub metrics: Option<String>,
    pub tools: Vec<ToolCall>,
    /// Still being written: the turn is under way.
    pub live: bool,
    /// Shown word for word: a draft waiting for approval.
    pub plain: bool,
    /// A picture or file the assistant sent, by its name at /api/file.
    pub picture: Option<String>,
    pub file: Option<String>,
    /// Files the person sent with it, by name.
    pub files: Vec<String>,
    pub reaction: Option<String>,
    pub outbox: Outbox,
    /// The host has it and holds it for the turn after the one under way.
    pub waits_turn: bool,
    /// Milliseconds since the epoch.
    pub date: Option<i64>,
    /// The AG-UI text message its words came in last.
    pub segment: Option<String>,
    /// How many calls the turn had made when a worker's report woke it
    /// again: a quiet reply to that report takes back only what came after.
    pub woke: Option<u32>,
    /// The earlier message the person's message replies to.
    pub quote: Option<Quote>,
    /// The kind of device the person sent it from (libri/src/origin.rs).
    pub from: Option<String>,
    /// The hippo row it begins with, which a reply's quote names: from the
    /// history, or told live once hippo takes it (the `row` event).
    pub row: Option<u64>,
    /// The rows its words and calls came in under live, to know it by when
    /// the `row` event names one.
    pub keys: Vec<String>,
}

impl Message {
    fn new(id: impl Into<String>, role: Role, text: impl Into<String>, date: Option<i64>) -> Self {
        Message {
            id: id.into(),
            role,
            text: text.into(),
            thinking: String::new(),
            progress: false,
            metrics: None,
            tools: Vec::new(),
            live: false,
            plain: false,
            picture: None,
            file: None,
            files: Vec::new(),
            reaction: None,
            outbox: Outbox::Arrived,
            waits_turn: false,
            date,
            segment: None,
            woke: None,
            quote: None,
            from: None,
            row: None,
            keys: Vec::new(),
        }
    }

    fn heard_under(&mut self, key: Option<&String>) {
        if let Some(key) = key
            && !self.keys.contains(key)
        {
            self.keys.push(key.clone());
        }
    }
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a valid pattern")
}

/// Words the person sent mid-turn, as Libri marks them for the model.
static STEERED: LazyLock<Regex> = LazyLock::new(|| re(r"^\(sent while you were working\) "));
/// A draft as Libri shows it (pending.rs `Act::show`).
static DRAFT: LazyLock<Regex> =
    LazyLock::new(|| re(r"^Draft to [^\n]+\nSubject: [\s\S]*\n\n👍 sends it, 👎 drops it\.$"));
/// How hippo logs a picture or file Libri sent (`send_outbox`).
static SENT: LazyLock<Regex> = LazyLock::new(|| re(r"^\(sent a (picture|file): (.+)\)$"));
/// Claude Code's news of a task done, which is not the model's words.
static NEWS: LazyLock<Regex> = LazyLock::new(|| re(r"^<task-notification>"));
/// Files the person sent, as hippo has them ahead of their words.
static THEIR_FILES: LazyLock<Regex> = LazyLock::new(|| {
    re(r"^\(sent ((?:a (?:photo|file): [^;]+?(?:, kept unseen)?(?:; )?)+)\) ?([\s\S]*)$")
});
static ONE_FILE: LazyLock<Regex> =
    LazyLock::new(|| re(r"^a (?:photo|file): (.+?)(?:, kept unseen)?$"));
/// What a seat is told by its routines, watches and messages,
/// framed by Libri (main.rs `who`), shown as the note it logged.
static FRAMED: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (re(r"^Routine \S+ set, due now: ([\s\S]*)$"), "routine"),
        (
            re(r"^Watch \S+ set, fired now [\s\S]*?answer only [^:]*: ([\s\S]*)$"),
            "watch",
        ),
        (
            re(r"^From other agents, quoted in <message>[\s\S]*?a routine's answer: ([\s\S]*)$"),
            "message",
        ),
    ]
});
/// A seat's answer that tells the person nothing (main.rs `QUIET`).
const QUIETS: [&str; 2] = ["NO_REPLY", "(nothing new)"];

fn framed(text: &str) -> Option<String> {
    FRAMED
        .iter()
        .find_map(|(re, kind)| re.captures(text).map(|c| format!("({kind}) {}", &c[1])))
}

/// Whether words are a quiet answer, never shown wherever in a turn they
/// stand (chat.ts `quiet`). Words still streaming are quiet while they could
/// yet become one.
#[uniffi::export]
pub fn quiet(text: &str, live: bool) -> bool {
    let text = text.trim();
    QUIETS.iter().any(|q| {
        if live {
            !text.is_empty() && q.starts_with(text)
        } else {
            text == *q
        }
    })
}

/// The person's words as hippo has them, "[image]\n(sent while you were
/// working) (sent a photo: a.png; a file: b.txt) words": the words, and
/// the files' names.
fn theirs(text: &str) -> (String, Vec<String>) {
    let mut text = text;
    while let Some(rest) = text
        .strip_prefix("[image]\n")
        .or_else(|| text.strip_prefix("[document]\n"))
    {
        text = rest;
    }
    let text = STEERED.replace(text, "");
    match THEIR_FILES.captures(&text) {
        None => (text.into_owned(), Vec::new()),
        Some(c) => {
            let files = c[1]
                .split("; ")
                .map(|f| {
                    ONE_FILE
                        .captures(f)
                        .map_or_else(|| f.to_owned(), |n| n[1].to_owned())
                })
                .collect();
            (c[2].to_owned(), files)
        }
    }
}

fn millis(date: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(date)
        .ok()
        .map(|d| d.timestamp_millis())
}

/// A message of the model's own turn: not a draft, a sent picture or file.
fn own(l: &Message) -> bool {
    l.role == Role::Assistant && !l.plain && l.picture.is_none() && l.file.is_none()
}

fn key_of(m: &Said) -> Option<&str> {
    m.src.as_ref().map(|s| s.e.as_str())
}

/// One of hippo's rows, added to the conversation so far. A seat's history
/// holds its work too, a `tool` line per call and an `echo` per result, in
/// order: they gather into the answer they led to, the words between them
/// kept above the call they led into. `ids` names calls the stream may yet
/// speak of, by their row's key.
fn add_row(out: &mut Vec<Message>, m: &Said, ids: &HashMap<&str, String>) {
    add_text_row(out, m, ids);
    if let Some(display) = &m.display
        && display["version"] == 1
        && let Some(metrics) = display.get("metrics")
        && let Some(last) = out.last_mut()
    {
        last.metrics = Some(metrics.to_string());
    }
}

fn add_text_row(out: &mut Vec<Message>, m: &Said, ids: &HashMap<&str, String>) {
    let date = millis(&m.date);
    // The answer under way: the last message, if it is the model's own.
    let turn = out.last().is_some_and(own);
    match m.kind.as_str() {
        "tool" => {
            let (name, input) = m.text.split_once(' ').unwrap_or((&m.text, ""));
            let arguments = input.to_owned();
            let (name, input) = presented(name, input);
            let id = key_of(m).and_then(|k| ids.get(k).map(String::as_str));
            let mut tool =
                ToolCall::new(id.map_or_else(|| format!("h-{}", m.i), str::to_owned), name);
            tool.input = input;
            tool.arguments = Some(arguments);
            // A call joins the calls before it; after words, it opens a
            // message of its own, so the turn reads in the order it went.
            let words = out.last().is_some_and(|l| !l.text.trim().is_empty());
            if turn && words {
                out.last_mut().unwrap().progress = true;
            }
            if turn && !words {
                let last = out.last_mut().unwrap();
                last.thinking.clear();
                last.tools.push(tool);
                last.date = date;
            } else {
                let mut msg = Message::new(format!("h-{}", m.i), Role::Assistant, "", date);
                msg.row = Some(m.i);
                msg.tools.push(tool);
                out.push(msg);
            }
        }
        // Subagents' reports belong to the Work page, but one inside a turn
        // marks where the turn was woken again.
        "work" => {
            if turn {
                let last = out.last_mut().unwrap();
                last.woke = Some(last.tools.len() as u32);
            }
        }
        "echo" => {
            if NEWS.is_match(&m.text) || !turn {
                return;
            }
            // New result rows identify their call; old rows pair by order.
            let key = key_of(m).and_then(|k| k.strip_suffix(".result"));
            let named = key.and_then(|k| ids.get(k));
            let open = if key.is_some() {
                out.iter_mut()
                    .flat_map(|m| &mut m.tools)
                    .find(|t| Some(&t.id) == named)
            } else {
                out.last_mut().unwrap().tools.iter_mut().find(|t| !t.done)
            };
            if let Some(open) = open {
                open.done = true;
                open.result = Some(m.text.clone());
                if let Some(display) = &m.display
                    && display["version"] == 1
                {
                    open.structured = display.get("result").map(Value::to_string);
                }
            }
        }
        "talk" if quiet(&m.text, false) => {
            // A routine that found nothing leaves no trace: not its
            // work, nor the note that started it. A turn a worker's report
            // woke again keeps what it had when woken: the calls after it
            // go, and the words that led into the first of them stand as
            // the turn's own again.
            if turn {
                // The turn is the model's messages since the last row of
                // anyone else's.
                let start = out.iter().rposition(|l| !own(l)).map_or(0, |i| i + 1);
                if let Some(w) = (start..out.len()).rev().find(|&i| out[i].woke.is_some()) {
                    let cut = out[w].woke.unwrap_or_default() as usize;
                    if out[w].tools.len() > cut || w + 1 < out.len() {
                        out.truncate(w + 1);
                        out[w].thinking.clear();
                        out[w].progress = false;
                        out[w].tools.truncate(cut);
                    }
                    return;
                }
                let last = out.last().unwrap();
                if !last.tools.is_empty() && last.text.trim().is_empty() {
                    out.truncate(start);
                }
            }
            if out.last().is_some_and(|l| l.role == Role::System) {
                out.pop();
            }
        }
        // The answer after the calls, once.
        "talk"
            if turn
                && out
                    .last()
                    .is_some_and(|l| (!l.tools.is_empty() || l.live) && l.text.is_empty())
                && !SENT.is_match(&m.text)
                && !DRAFT.is_match(&m.text) =>
        {
            let last = out.last_mut().unwrap();
            last.text = m.text.clone();
            last.date = date;
        }
        "user" | "talk" | "note" => {
            let id = format!("h-{}", m.i);
            let mut msg = match m.kind.as_str() {
                "user" => match framed(&m.text) {
                    Some(note) => Message::new(id, Role::System, note, date),
                    None => {
                        let (text, files) = theirs(&m.text);
                        let mut msg = Message::new(id, Role::User, text, date);
                        msg.files = files;
                        msg.quote = m.quote.clone();
                        msg.from = m.from.clone();
                        msg
                    }
                },
                "talk" => {
                    let mut msg = Message::new(id, Role::Assistant, "", date);
                    match SENT.captures(&m.text) {
                        Some(c) if &c[1] == "picture" => msg.picture = Some(c[2].to_owned()),
                        Some(c) => msg.file = Some(c[2].to_owned()),
                        None => {
                            msg.text = m.text.clone();
                            msg.plain = DRAFT.is_match(&m.text);
                        }
                    }
                    msg
                }
                _ => Message::new(id, Role::System, m.text.clone(), date),
            };
            msg.reaction = m.reaction.clone();
            msg.row = Some(m.i);
            out.push(msg);
        }
        _ => {}
    }
}

/// The chat a sync leaves: hippo's rows, then what `record` holds that they
/// lack, each thing after the row it came after. An event names the row it
/// will be, so hippo and the record never need comparing by their words;
/// this is UniiChat's `Chat.live`.
pub fn from_history(said: &[Said], record: Option<&TurnRecord>, turning: bool) -> Vec<Message> {
    let rows: HashSet<&str> = said.iter().filter_map(key_of).collect();
    let stored = |k: &Option<String>| k.as_deref().is_some_and(|k| rows.contains(k));
    let held = record.map_or(&[][..], |r| &r.shown[..]);
    // A call stays in the record until its result does, so a row can take
    // the id the stream's result will name it by.
    let mut ids: HashMap<&str, String> = held
        .iter()
        .filter(|e| e.kind == "TOOL_CALL_START" && stored(&e.key))
        .map(|e| (str_of(&e.key), str_of(&e.tool_call_id).to_owned()))
        .collect();
    for row in said.iter().filter(|m| m.kind == "tool") {
        if let Some(key) = key_of(row) {
            ids.entry(key).or_insert_with(|| format!("h-{}", row.i));
        }
    }
    let lacking: Vec<&Event> = held
        .iter()
        .filter(|e| !stored(&e.key) && e.subagent_run_id.is_none())
        .filter(|e| !(e.kind == "RUN_FINISHED" && e.interrupted()))
        .collect();
    // An event past the history's window goes first, unless a later one has
    // a row to stand on, when it is the newest and goes last.
    let anchored = lacking.iter().rposition(|e| stored(&e.after));
    let mut after: HashMap<&str, Vec<&Event>> = HashMap::new();
    let (mut oldest, mut newest) = (Vec::new(), Vec::new());
    for (n, e) in lacking.iter().enumerate() {
        if stored(&e.after) {
            after.entry(str_of(&e.after)).or_default().push(e);
        } else if anchored.is_some_and(|a| n < a) {
            oldest.push(*e);
        } else {
            newest.push(*e);
        }
    }
    let take = |out: &mut Vec<Message>, events: &[&Event]| {
        for e in events {
            // A turn's calls are in hippo as it goes; the rest join them.
            if let Some(last) = out.last_mut().filter(|l| !l.tools.is_empty()) {
                last.live = true;
            }
            fold(out, e);
        }
    };
    let mut out: Vec<Message> = Vec::new();
    take(&mut out, &oldest);
    for m in said {
        add_row(&mut out, m, &ids);
        if let Some(events) = key_of(m).and_then(|k| after.get(k)) {
            take(&mut out, events);
        }
    }
    take(&mut out, &newest);
    // Only the newest of a running turn goes on growing.
    let end = out.len().saturating_sub(1);
    for (n, m) in out.iter_mut().enumerate() {
        let turn = m.role == Role::Assistant && !m.plain && (m.live || !m.tools.is_empty());
        m.live = turning && n == end && turn;
    }
    out
}

fn str_of(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("")
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// What an event asks of the client beyond the messages.
#[derive(Debug, PartialEq, Eq)]
pub enum Applied {
    Done,
    /// The history changed under it (hippo forgot its oldest, or a temp chat
    /// joined it): read it afresh.
    Resync,
}

/// The conversation and whether a turn is running.
#[derive(Default, Clone, Debug)]
pub struct Conversation {
    pub messages: Vec<Message>,
    pub typing: bool,
    /// What the turn waits on the person for.
    pub asks: Vec<Ask>,
    /// The turn's background jobs, while they run.
    pub jobs: Vec<String>,
    /// Acts waiting for the person's word.
    pub cards: Vec<Card>,
    /// Counts each subagent start and end heard.
    pub work_news: u64,
    /// Counts each word that the scratchpad changed.
    pub temp_news: u64,
    /// The calls each worker made, by its name, as far as this client has
    /// heard them; a worker's report back is no step (chat.ts `taskEvent`).
    pub worker_steps: HashMap<String, u32>,
    /// Current-run presentation data for Temp and worker threads.
    pub worker_messages: HashMap<String, Vec<Message>>,
    /// The page show_page last published while the client listened, to open
    /// by itself (chat.ts `published`).
    pub published: Option<Published>,
    /// show_page calls heard begin on the stream: only their results open
    /// the page, so the history and a reconnect's read never do.
    publishing: HashSet<String>,
    /// The notes this client wrote (a command's answer), which no history
    /// holds: a read of it keeps them, as the page keeps its own.
    notes: HashSet<String>,
}

/// A page published live; `n` tells one publishing from the next of the
/// same page.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Eq)]
pub struct Published {
    pub page: String,
    pub n: u64,
}

/// The page a show_page result published, from the link it gives
/// (`…#page=<name>`, library.rs link).
fn published_page(result: &str) -> Option<String> {
    let at = result.find("#page=")? + "#page=".len();
    let name: String = result[at..]
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, ')' | ']' | '"' | '\'' | '<' | '>'))
        .collect();
    (!name.is_empty()).then(|| percent_decoded(&name))
}

/// A link's `%XX` escapes undone; a broken one is kept as written.
fn percent_decoded(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_owned())
}

/// show_page by any harness's name for it: bare, or an MCP server's
/// `mcp__libri__show_page`.
fn is_show_page(name: &str) -> bool {
    name == "show_page" || name.ends_with("__show_page")
}

impl Conversation {
    /// What one AG-UI event does to the talk: the person's words, words and
    /// thinking as they stream, calls and their results, a picture or file.
    pub fn apply(&mut self, e: &Event) -> Applied {
        if e.kind == "SUBAGENT_STARTED" || e.kind == "SUBAGENT_FINISHED" {
            self.work_news += 1;
        }
        if e.kind == "CUSTOM" && e.name.as_deref() == Some("scratchpad") {
            self.temp_news += 1;
        }
        if e.kind == "SUBAGENT_FINISHED"
            && let (Some(id), Some(Value::String(report))) = (&e.subagent_run_id, &e.result)
        {
            on_tool(&mut self.messages, id, |t| t.report = Some(report.clone()));
        }
        if let Some(worker) = &e.subagent_run_id {
            let messages = self.worker_messages.entry(worker.clone()).or_default();
            if matches!(e.kind.as_str(), "SUBAGENT_STARTED" | "RUN_STARTED") {
                messages.clear();
            }
            fold(messages, e);
            if e.kind == "TOOL_CALL_START"
                && e.tool_call_name.as_deref() != Some("SubagentHandback")
            {
                *self.worker_steps.entry(worker.clone()).or_default() += 1;
            }
            return Applied::Done;
        }
        if e.kind == "TOOL_CALL_START" && is_show_page(str_of(&e.tool_call_name)) {
            self.publishing.insert(str_of(&e.tool_call_id).to_owned());
        }
        if e.kind == "TOOL_CALL_RESULT"
            && self.publishing.remove(str_of(&e.tool_call_id))
            && !e.flag("error")
            && let Some(Value::String(text)) = &e.content
            && let Some(page) = published_page(text)
        {
            let n = self.published.as_ref().map_or(0, |p| p.n) + 1;
            self.published = Some(Published { page, n });
        }
        match e.kind.as_str() {
            "RUN_STARTED" => {
                self.typing = true;
                self.asks.clear();
            }
            "RUN_FINISHED" => {
                // An interrupt only pauses the turn for the person's answer.
                if e.interrupted() {
                    self.asks = asks_of(e);
                } else {
                    self.typing = false;
                    self.jobs.clear();
                    self.asks.clear();
                    fold(&mut self.messages, e);
                }
            }
            "ACTIVITY_SNAPSHOT" => match e.activity_type.as_deref() {
                Some("jobs") => {
                    self.jobs = jobs_of(e);
                }
                Some("approval") => {
                    if let Some(card) = e.content.as_ref().and_then(Card::of)
                        && !self.cards.iter().any(|c| c.id == card.id)
                    {
                        self.cards.push(card);
                    }
                }
                Some("tool_output") => fold(&mut self.messages, e),
                _ => {}
            },
            // What came of a verdict: done, the card goes and the outcome
            // stays in the talk; failed, the card says why.
            "ACTIVITY_DELTA" if e.activity_type.as_deref() == Some("approval") => {
                let outcome = e.patch.iter().flatten().find(|p| p["path"] == "/outcome");
                let id = str_of(&e.message_id)
                    .trim_start_matches("act-")
                    .parse::<u64>()
                    .ok();
                if let (Some(o), Some(id)) = (outcome, id) {
                    let text = o["value"]["text"].as_str().unwrap_or("").to_owned();
                    if o["value"]["done"].as_bool().unwrap_or(false) {
                        self.cards.retain(|c| c.id != id);
                        self.messages.push(Message::new(
                            format!("act-{id}"),
                            Role::Assistant,
                            text,
                            Some(now()),
                        ));
                    } else if let Some(c) = self.cards.iter_mut().find(|c| c.id == id) {
                        c.error = Some(text);
                    }
                }
            }
            // The rows moved, or the person's words took theirs: a read around
            // the moment they did holds them twice.
            "CUSTOM" if matches!(e.name.as_deref(), Some("forgot" | "stored" | "history")) => {
                return Applied::Resync;
            }
            "CUSTOM" if e.name.as_deref() == Some("queued") => {
                let v = e.value.as_ref();
                let id = v.and_then(|v| v["id"].as_str());
                let on = v.and_then(|v| v["on"].as_bool()).unwrap_or(false);
                if let Some(m) = self.messages.iter_mut().find(|m| Some(m.id.as_str()) == id) {
                    m.waits_turn = on;
                }
            }
            _ => fold(&mut self.messages, e),
        }
        Applied::Done
    }

    /// The conversation afresh from the history and what the host holds of
    /// the turn under way that hippo doesn't yet.
    pub fn sync(&mut self, said: &[Said], task: &TaskNow) {
        let was = std::mem::take(&mut self.messages);
        let mut now = from_history(said, task.turn.as_ref(), task.typing);
        self.typing = task.typing;
        let shown = task.turn.as_ref().map_or(&[][..], |r| &r.shown[..]);
        self.jobs = shown
            .iter()
            .rfind(|e| e.kind == "ACTIVITY_SNAPSHOT" && e.activity_type.as_deref() == Some("jobs"))
            .map(jobs_of)
            .unwrap_or_default();
        self.asks = shown
            .iter()
            .rfind(|e| e.kind == "RUN_STARTED" || e.kind == "RUN_FINISHED")
            .filter(|e| e.interrupted())
            .map(asks_of)
            .unwrap_or_default();
        // History acknowledgements replace streaming rows. Retain live presentation
        // fields by stable row/tool identity, as the web reducer does.
        for m in &mut now {
            if let Some(old) = was
                .iter()
                .find(|old| old.id == m.id || (m.row.is_some() && old.row == m.row))
                && m.metrics.is_none()
            {
                m.metrics.clone_from(&old.metrics);
            }
            for (index, tool) in m.tools.iter_mut().enumerate() {
                if let Some(old) = was
                    .iter()
                    .flat_map(|old| &old.tools)
                    .find(|old| old.id == tool.id)
                    .or_else(|| {
                        was.iter()
                            .find(|old| m.row.is_some() && old.row == m.row)
                            .and_then(|old| old.tools.get(index))
                            .filter(|old| old.name == tool.name && old.input == tool.input)
                    })
                {
                    if tool.structured.is_none() {
                        tool.structured.clone_from(&old.structured);
                    }
                    tool.thought.clone_from(&old.thought);
                    tool.report.clone_from(&old.report);
                    tool.heard |= old.heard;
                }
            }
        }
        // A message the record holds has only its words: the rest is ours.
        // Held there, it has arrived, whatever its send last said.
        for m in now.iter_mut().filter(|m| m.role == Role::User) {
            if let Some(w) = was.iter().find(|w| w.id == m.id) {
                m.waits_turn = w.waits_turn;
                if m.files.is_empty() {
                    m.files.clone_from(&w.files);
                }
            }
        }
        // Words on their way to the host or refused by it stay, after all
        // it has, and so do this client's own notes: a read that a stream's
        // reconnect started took a command's answer away the moment it came.
        let mut after: Option<String> = None;
        for m in was {
            let kept = now.iter().position(|n| n.id == m.id);
            if m.outbox != Outbox::Arrived && kept.is_none() {
                now.push(m);
                continue;
            }
            if self.notes.contains(&m.id) && kept.is_none() {
                // In its place: after what came before it, if the read has it.
                let at = after
                    .as_ref()
                    .and_then(|id| now.iter().position(|n| &n.id == id))
                    .map_or(now.len(), |i| i + 1);
                after = Some(m.id.clone());
                now.insert(at, m);
                continue;
            }
            if kept.is_some() {
                after = Some(m.id.clone());
            }
        }
        self.notes.retain(|id| now.iter().any(|m| &m.id == id));
        self.messages = now;
    }

    /// Where the person's message stands on its way, unless the host has
    /// it already: its echo can come before the send's own answer.
    pub fn outbox(&mut self, id: &str, now: Outbox) {
        if let Some(m) = self.messages.iter_mut().find(|m| m.id == id)
            && m.outbox != Outbox::Arrived
        {
            m.outbox = now;
        }
    }

    /// Whether the conversation holds the message as the host's.
    pub fn arrived(&self, id: &str) -> bool {
        self.messages
            .iter()
            .any(|m| m.id == id && m.outbox == Outbox::Arrived)
    }

    /// The person's message, shown at once, before the host has it.
    /// A redo takes the place of the person's last message and what
    /// followed it, as the history will.
    pub fn said(
        &mut self,
        id: &str,
        text: &str,
        files: Vec<String>,
        redo: bool,
        quote: Option<Quote>,
    ) {
        if redo && let Some(at) = self.messages.iter().rposition(|m| m.role == Role::User) {
            self.messages.truncate(at);
        }
        let mut m = Message::new(id, Role::User, text, Some(now()));
        m.files = files;
        m.outbox = Outbox::Sending;
        m.quote = quote;
        self.messages.push(m);
    }

    /// Takes the person's message out of the talk, answering it.
    pub fn withdrawn(&mut self, id: &str) -> Option<Message> {
        let at = self.messages.iter().position(|m| m.id == id)?;
        Some(self.messages.remove(at))
    }

    /// A note of the client's own, as the page shows a command's answer.
    pub fn note(&mut self, id: &str, text: &str) {
        self.notes.insert(id.to_owned());
        self.messages
            .push(Message::new(id, Role::System, text, Some(now())));
    }
}

fn jobs_of(e: &Event) -> Vec<String> {
    e.content
        .as_ref()
        .and_then(|c| c.get("names"))
        .and_then(Value::as_array)
        .map(|ns| {
            ns.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn segment(ms: &[Message], id: &str) -> Option<usize> {
    ms.iter().rposition(|m| m.segment.as_deref() == Some(id))
}

fn on_tool(ms: &mut [Message], id: &str, mut f: impl FnMut(&mut ToolCall)) {
    for m in ms.iter_mut() {
        if let Some(t) = m.tools.iter_mut().find(|t| t.id == id) {
            f(t);
        }
    }
}

/// The messages' part of `Conversation::apply`.
fn fold(ms: &mut Vec<Message>, e: &Event) {
    let id = str_of(&e.message_id);
    let delta = str_of(&e.delta);
    match e.kind.as_str() {
        "TEXT_MESSAGE_CHUNK" => {
            // The person's words, from this client or another door.
            if e.role.as_deref() != Some("user") {
                return;
            }
            // The echo of this client's own message proves it arrived,
            // whatever the send's own reply said.
            if let Some(m) = ms.iter_mut().find(|m| m.id == id) {
                m.outbox = Outbox::Arrived;
                return;
            }
            let msg = match framed(delta) {
                Some(note) => Message::new(id, Role::System, note, Some(now())),
                None => Message::new(id, Role::User, delta, Some(now())),
            };
            ms.push(msg);
        }
        "TEXT_MESSAGE_START" => {
            let mut fresh = Message::new(id, Role::Assistant, "", Some(now()));
            fresh.segment = Some(id.to_owned());
            fresh.live = true;
            // A draft word for word, or a message from no turn, stands apart.
            if e.flag("plain") || e.flag("aside") {
                fresh.plain = e.flag("plain");
                fresh.live = false;
                ms.push(fresh);
                return;
            }
            match ms.last_mut() {
                Some(last) if last.live => {
                    if last.text.trim().is_empty() {
                        last.id = id.to_owned();
                        last.segment = Some(id.to_owned());
                    } else {
                        // Words after words already ended: an answer of their own.
                        last.live = false;
                        ms.push(fresh);
                    }
                }
                _ => ms.push(fresh),
            }
        }
        "TEXT_MESSAGE_CONTENT" => {
            if let Some(i) = segment(ms, id) {
                ms[i].text.push_str(delta);
                ms[i].heard_under(e.key.as_ref());
            }
        }
        "TEXT_MESSAGE_END" => {
            // Words the turn never said as its answer go; its calls stay.
            if !e.flag("discarded") {
                return;
            }
            if let Some(i) = segment(ms, id) {
                if ms[i].tools.is_empty() && ms[i].thinking.is_empty() {
                    ms.remove(i);
                } else {
                    ms[i].text.clear();
                }
            }
        }
        "REASONING_MESSAGE_CONTENT" => match ms.last_mut() {
            Some(last) if last.live => last.thinking.push_str(delta),
            _ => {
                let mut m = Message::new(id, Role::Assistant, "", Some(now()));
                m.thinking = delta.to_owned();
                m.live = true;
                ms.push(m);
            }
        },
        "TOOL_CALL_START" => {
            let call = str_of(&e.tool_call_id);
            let mut tool = ToolCall::new(call, started(str_of(&e.tool_call_name)));
            tool.heard = true;
            let mut fresh = Message::new(format!("w-{call}"), Role::Assistant, "", Some(now()));
            fresh.live = true;
            match ms.last_mut() {
                Some(last) if last.live => {
                    // Words said before the call are a message of their
                    // own, and the call opens the next: the turn reads in
                    // the order it went, live as after a reload.
                    tool.thought = (!last.thinking.is_empty()).then(|| last.thinking.clone());
                    if quiet(&last.text, false) {
                        last.text.clear();
                    }
                    if !last.text.trim().is_empty() {
                        last.live = false;
                        last.progress = true;
                        fresh.tools.push(tool);
                        ms.push(fresh);
                    } else {
                        last.thinking.clear();
                        last.tools.push(tool);
                    }
                }
                _ => {
                    fresh.tools.push(tool);
                    ms.push(fresh);
                }
            }
            // Every branch leaves the call in the last message.
            if let Some(last) = ms.last_mut() {
                last.heard_under(e.key.as_ref());
            }
        }
        // The input streams as JSON; it reads whole once the call ends.
        "TOOL_CALL_ARGS" => on_tool(ms, str_of(&e.tool_call_id), |t| t.input.push_str(delta)),
        "TOOL_CALL_END" => on_tool(ms, str_of(&e.tool_call_id), |t| {
            t.arguments = Some(t.input.clone());
            (t.name, t.input) = presented(&t.name, &t.input);
        }),
        "TOOL_CALL_RESULT" => {
            let error = e.flag("error");
            let text = match &e.content {
                Some(Value::String(s)) => s.clone(),
                Some(v) => v.to_string(),
                None => String::new(),
            };
            on_tool(ms, str_of(&e.tool_call_id), |t| {
                t.done = true;
                t.error = error;
                t.result = Some(text.clone());
            });
        }
        // A running call's output so far, whole each time: the row shows
        // it as its result until the real one comes.
        "ACTIVITY_SNAPSHOT" if e.activity_type.as_deref() == Some("tool_output") => {
            let o = e.content.clone().unwrap_or_default();
            let text = o["text"].as_str().unwrap_or_default().to_owned();
            on_tool(ms, o["toolCallId"].as_str().unwrap_or_default(), |t| {
                if !t.done {
                    t.result = Some(text.clone());
                }
            });
        }
        "RUN_FINISHED" => {
            for m in ms.iter_mut().filter(|m| m.live) {
                m.live = false;
            }
        }
        "CUSTOM" => {
            let value = e.value.as_ref();
            let field = |k: &str| {
                value
                    .and_then(|v| v.get(k))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            };
            match e.name.as_deref() {
                Some("libri.metrics" | "libri.usage") => {
                    if let Some(last) = ms.last_mut() {
                        let mut metrics: Value = last
                            .metrics
                            .as_deref()
                            .and_then(|s| serde_json::from_str(s).ok())
                            .unwrap_or_else(|| json!({}));
                        if let Some(v) = value.and_then(Value::as_object) {
                            for (k, v) in v {
                                metrics[k] = v.clone();
                            }
                        }
                        last.metrics = Some(metrics.to_string());
                    }
                }
                Some("libri.tool_result") => {
                    if let Some(v) = value {
                        on_tool(ms, v["toolCallId"].as_str().unwrap_or_default(), |t| {
                            t.structured = Some(v["result"].to_string())
                        });
                    }
                }
                Some("picture") => {
                    let mut m =
                        Message::new(format!("p-{}", ms.len()), Role::Assistant, "", Some(now()));
                    m.picture = field("name");
                    ms.push(m);
                }
                Some("file") => {
                    let mut m =
                        Message::new(format!("f-{}", ms.len()), Role::Assistant, "", Some(now()));
                    m.file = field("name");
                    ms.push(m);
                }
                // hippo took a row: the message drawn live that holds it is
                // named by it from now, as a reload would name it. Rows
                // come in order, so the first a message hears begins it.
                Some("row") => {
                    let key = field("key");
                    let row = value.and_then(|v| v["row"].as_u64());
                    let heard: Vec<&str> = value
                        .and_then(|v| v["heard"].as_array())
                        .map(|a| a.iter().filter_map(Value::as_str).collect())
                        .unwrap_or_default();
                    for m in ms.iter_mut().filter(|m| m.row.is_none()) {
                        if heard.contains(&m.id.as_str())
                            || key.as_ref().is_some_and(|k| m.keys.contains(k))
                        {
                            m.row = row;
                        }
                    }
                }
                Some("react") => {
                    let to = field("to");
                    for m in ms.iter_mut() {
                        if to.as_deref() == Some(m.id.as_str()) || to == m.segment {
                            m.reaction = field("emoji");
                        }
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn show_card_draws_the_tool_it_names() {
        let d = super::drawn_card(
            "mcp__libri__show_card".into(),
            Some(r#"{"tool":"mcp__hob__cameras","arguments":{"camera":"tapo1"}}"#.into()),
        );
        assert_eq!(d.name, "mcp__hob__cameras");
        assert!(d.shown && d.input.contains("tapo1"));
        let own = super::drawn_card(
            "mcp__fastmail__read_email".into(),
            Some(r#"{"id":"M1"}"#.into()),
        );
        assert!(!own.shown && own.name == "mcp__fastmail__read_email" && own.input.contains("M1"));
    }

    #[test]
    fn only_the_persons_turns_open_cards() {
        let m = |id: &str, role: Role, woke: Option<u32>| {
            let mut m = Message::new(id, role, "", None);
            m.woke = woke;
            m
        };
        let ids = super::person_turns(vec![
            m("u1", Role::User, None),
            m("a1", Role::Assistant, None),
            m("n1", Role::System, None),
            m("a2", Role::Assistant, None),
            m("u2", Role::User, None),
            m("a3", Role::Assistant, Some(1)),
            m("a4", Role::Assistant, None),
        ]);
        assert_eq!(ids, vec!["a1", "a4"]);
    }

    use super::*;
    use crate::wire::Src;

    #[test]
    fn answer_only_marks_narration_live_and_after_reload() {
        let mut c = Conversation::default();
        for e in [
            r#"{"type":"RUN_STARTED"}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"a"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"a","delta":"Checking."}"#,
            r#"{"type":"TOOL_CALL_START","toolCallId":"t","toolCallName":"Read"}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"b"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"b","delta":"Ready."}"#,
            r#"{"type":"RUN_FINISHED"}"#,
        ] {
            c.apply(&ev(e));
        }
        assert!(c.messages[0].progress);
        assert!(!c.messages.last().unwrap().progress);
        let ms = history(&[
            said(1, "user", "Check"),
            said(2, "talk", "Checking."),
            said(3, "tool", "Read {}"),
            said(4, "echo", "ok"),
            said(5, "talk", "Ready."),
        ]);
        assert!(ms.iter().find(|m| m.text == "Checking.").unwrap().progress);
        assert!(!ms.iter().find(|m| m.text == "Ready.").unwrap().progress);
    }

    #[test]
    fn rich_tool_results_and_context_are_kept_separate_from_words() {
        let mut c = Conversation::default();
        for e in [
            r#"{"type":"TOOL_CALL_START","toolCallId":"t","toolCallName":"edit"}"#,
            r#"{"type":"CUSTOM","name":"libri.tool_result","value":{"toolCallId":"t","result":{"details":{"diff":"- old\n+ new"}}}}"#,
            r#"{"type":"CUSTOM","name":"libri.metrics","value":{"context_tokens":32000,"context_window":128000,"tokens_per_second":42.5}}"#,
            r#"{"type":"CUSTOM","name":"libri.usage","value":{"input":30000,"output":200}}"#,
        ] {
            c.apply(&ev(e));
        }
        let m = c.messages.last().unwrap();
        assert!(m.tools[0].structured.as_ref().unwrap().contains("diff"));
        // Heard on the stream, so a card for it opens at once.
        assert!(m.tools[0].heard);
        let metrics: Value = serde_json::from_str(m.metrics.as_ref().unwrap()).unwrap();
        assert_eq!(metrics["context_tokens"], 32000);
        assert_eq!(metrics["tokens_per_second"], 42.5);
        assert!(m.text.is_empty());
    }

    /// A command's answer is the client's own note, which no history
    /// holds: a read of the history (as a stream's reconnect starts) keeps
    /// it in its place.
    #[test]
    fn a_read_of_the_history_keeps_the_clients_own_notes() {
        let mut c = Conversation::default();
        c.sync(&[said(1, "user", "Check")], &TaskNow::default());
        c.note("n1", "On Manual.");
        c.sync(&[said(1, "user", "Check")], &TaskNow::default());
        let texts: Vec<&str> = c.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["Check", "On Manual."]);
        c.sync(
            &[said(1, "user", "Check"), said(2, "talk", "Done.")],
            &TaskNow::default(),
        );
        // What the host says next comes after it, as it did.
        let texts: Vec<&str> = c.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["Check", "On Manual.", "Done."]);
    }

    #[test]
    fn history_acknowledgement_retains_live_context_by_row() {
        let mut c = Conversation::default();
        let mut m = Message::new("streamed", Role::Assistant, "Ready.", None);
        m.row = Some(2);
        m.metrics = Some(r#"{"context_tokens":32000}"#.into());
        c.messages.push(m);
        c.sync(
            &[said(1, "user", "Check"), said(2, "talk", "Ready.")],
            &TaskNow::default(),
        );
        assert!(
            c.messages
                .last()
                .unwrap()
                .metrics
                .as_ref()
                .unwrap()
                .contains("32000")
        );
    }

    #[test]
    fn settled_history_retains_rich_tool_result_by_row_and_position() {
        let rows = [
            said(1, "user", "Check"),
            said(2, "tool", "Read {}"),
            said(3, "echo", "ok"),
            said(4, "talk", "Ready."),
        ];
        let mut c = Conversation::default();
        c.messages = history(&rows);
        let tool = &mut c.messages.last_mut().unwrap().tools[0];
        tool.id = "live-id".into();
        tool.structured = Some(r#"{"content":[{"type":"image"}]}"#.into());
        c.sync(&rows, &TaskNow::default());
        assert!(c.messages.last().unwrap().tools[0].structured.is_some());
    }

    #[test]
    fn worker_rich_results_are_isolated_from_main_chat() {
        let mut c = Conversation::default();
        for e in [
            r#"{"type":"TOOL_CALL_START","subagentRunId":"Ada","toolCallId":"t","toolCallName":"edit"}"#,
            r#"{"type":"CUSTOM","subagentRunId":"Ada","name":"libri.tool_result","value":{"toolCallId":"t","result":{"details":{"diff":"+ new"}}}}"#,
            r#"{"type":"CUSTOM","subagentRunId":"Ada","name":"libri.metrics","value":{"tokens_per_second":42.5}}"#,
        ] {
            c.apply(&ev(e));
        }
        assert!(c.messages.is_empty());
        let m = c.worker_messages["Ada"].last().unwrap();
        assert!(m.tools[0].structured.is_some());
        assert!(m.metrics.is_some());
    }

    #[test]
    fn full_detail_retains_arguments_beside_the_compact_label() {
        let rows = [
            said(1, "user", "Check"),
            said(2, "tool", r#"Read {"file_path":"settings.rs","offset":20}"#),
            said(3, "echo", "ok"),
        ];
        let ms = history(&rows);
        let tool = &ms.last().unwrap().tools[0];
        assert_eq!(tool.input, "settings.rs");
        assert!(tool.arguments.as_ref().unwrap().contains("offset"));
        let steps =
            crate::host::steps(r#"0|tool [read]: Read {"file_path":"settings.rs","offset":20}"#);
        assert_eq!(steps[0].text, "settings.rs");
        assert!(steps[0].arguments.as_ref().unwrap().contains("offset"));
    }

    #[test]
    fn a_message_to_a_worker_reads_as_one_on_each_harness() {
        let input = r#"{"name":"Abel","message":"\none\ntwo"}"#;
        for name in [
            "mcp__libri__tell_agent",
            "mcp__libri_tell_agent",
            "libri_tell_agent",
            "tell_agent",
        ] {
            assert_eq!(
                presented(name, input),
                ("Message to Abel".to_owned(), "one\ntwo".to_owned())
            );
        }
        assert_eq!(presented("Bash", r#"{"command":"ls"}"#).1, "ls");
        // What a row names is the first string in the order the call gave them.
        let call = r#"{"to":"","subject":"The draft\nsecond line","limit":3,"body":"Dear"}"#;
        assert_eq!(gist(&presented("mail", call).1), "The draft");
        assert_eq!(gist("ls -la\nwc -l"), "ls -la");
        assert_eq!(gist(r#"{"limit":3}"#), "");
        // Only a whole tell_agent, and only one with its arguments.
        assert_eq!(presented("retell_agent", input).0, "retell_agent");
        assert_eq!(presented("tell_agent", "{}").0, "tell_agent");
    }

    #[test]
    fn a_message_under_way_reads_as_one_from_its_first_event() {
        let mut c = Conversation::default();
        c.apply(&ev(
            r#"{"type":"TOOL_CALL_START","toolCallId":"t","toolCallName":"mcp__libri__tell_agent"}"#,
        ));
        assert_eq!(c.messages[0].tools[0].name, "Message");
        c.apply(&ev(
            r#"{"type":"TOOL_CALL_ARGS","toolCallId":"t","delta":"{\"name\":\"Abel\","}"#,
        ));
        c.apply(&ev(
            r#"{"type":"TOOL_CALL_ARGS","toolCallId":"t","delta":"\"message\":\"one\"}"}"#,
        ));
        c.apply(&ev(r#"{"type":"TOOL_CALL_END","toolCallId":"t"}"#));
        let t = &c.messages[0].tools[0];
        assert_eq!(
            (t.name.as_str(), t.input.as_str()),
            ("Message to Abel", "one")
        );
    }

    #[test]
    fn saved_display_results_follow_call_keys_instead_of_completion_order() {
        let first = said(1, "tool", "edit {}");
        let second = said(2, "tool", "read {}");
        let mut result = said(3, "echo", "picture");
        result.src = Some(Src {
            e: "k2.result".into(),
        });
        result.display = Some(
            json!({"version":1,"result":{"content":[{"type":"image","file":"tool-result.png"}]}}),
        );
        let out = from_history(&[first, second, result], None, false);
        let tools: Vec<_> = out.iter().flat_map(|m| &m.tools).collect();
        assert!(!tools[0].done);
        assert!(
            tools[1]
                .structured
                .as_ref()
                .unwrap()
                .contains("tool-result.png")
        );
    }

    #[test]
    fn display_is_restored_without_live_events() {
        let mut result = said(2, "echo", "Updated file");
        result.display = Some(json!({"version":1,"result":{"details":{"diff":"- off\n+ on"}}}));
        let mut reply = said(3, "talk", "Done");
        reply.display = Some(json!({"version":1,"metrics":{"tokens_per_second":42.5}}));
        let out = from_history(&[said(1, "tool", "edit {}"), result, reply], None, false);
        let tool = out.iter().flat_map(|m| &m.tools).next().unwrap();
        assert!(tool.structured.as_ref().unwrap().contains("diff"));
        assert!(
            out.iter()
                .any(|m| m.metrics.as_ref().is_some_and(|v| v.contains("42.5")))
        );
    }

    #[test]
    fn a_reply_quotes_the_earlier_message() {
        let rows = [
            said(1, "user", "weather?"),
            said(2, "talk", "rain"),
            Said {
                display: None,
                quote: Some(Quote {
                    id: 2,
                    text: "rain".into(),
                }),
                ..said(3, "user", "why?")
            },
        ];
        let out = from_history(&rows, None, false);
        let got: Vec<_> = out
            .iter()
            .map(|m| (m.id.as_str(), m.quote.as_ref().map(|q| q.id)))
            .collect();
        assert_eq!(got, [("h-1", None), ("h-2", None), ("h-3", Some(2))]);
    }

    fn said(i: u64, kind: &str, text: &str) -> Said {
        Said {
            display: None,
            i,
            kind: kind.to_owned(),
            text: text.to_owned(),
            date: "2026-10-29T21:50:27-04:00".to_owned(),
            reaction: None,
            src: Some(Src { e: format!("k{i}") }),
            quote: None,
            from: None,
        }
    }

    fn history(said: &[Said]) -> Vec<Message> {
        from_history(said, None, false)
    }

    #[test]
    fn a_shown_offer_has_no_row_and_a_refused_one_keeps_it() {
        let mut call = ToolCall::new("t1", "mcp__libri__offer_replies");
        assert!(offer_shown(&call));
        call.result = Some(SHOWN.into());
        assert!(offer_shown(&call));
        call.result = Some("At most three replies.".into());
        assert!(!offer_shown(&call));
        call.result = Some(SHOWN.into());
        call.error = true;
        assert!(!offer_shown(&call));
        assert!(!offer_shown(&ToolCall::new("t2", "Bash")));
    }

    #[test]
    fn offered_replies_sit_under_the_newest_answer_until_anything_follows() {
        let rows = vec![
            said(20, "user", "ready to ship?"),
            said(
                21,
                "tool",
                r#"mcp__libri__offer_replies {"options":["Yes, deploy","Not yet"]}"#,
            ),
            said(22, "echo", "Shown under your message."),
            said(23, "talk", "It passed. Deploy now?"),
        ];
        let offer = offered(&history(&rows), &[]).unwrap();
        assert_eq!(offer.replies, ["Yes, deploy", "Not yet"]);
        assert_eq!(
            offer.quote.as_ref().map(|q| (q.id, q.text.as_str())),
            Some((21, "It passed. Deploy now?"))
        );
        let mut later = rows.clone();
        later.push(said(24, "user", "Not yet"));
        assert_eq!(offered(&history(&later), &[]), None);
        let mut refused = rows.clone();
        refused[2] = said(22, "echo", "Offer one to 3 replies.");
        assert_eq!(offered(&history(&refused), &[]), None);
    }

    #[test]
    fn one_plain_question_is_offered_as_replies_and_more_stays_a_card() {
        let q = |multi, n: usize| Question {
            question: "Which branch?".into(),
            header: None,
            options: (0..n)
                .map(|i| Choice {
                    label: format!("b{i}"),
                    description: None,
                })
                .collect(),
            multi,
        };
        let ask = |questions| Ask {
            id: "a1".into(),
            tool: "AskUserQuestion".into(),
            detail: String::new(),
            about: None,
            questions,
            always: false,
        };
        let answer = history(&[said(1, "talk", "Which one?")]);
        let offer = offered(&answer, &[ask(vec![q(false, 2)])]).unwrap();
        assert_eq!((offer.replies.len(), offer.ask.as_deref()), (2, Some("a1")));
        assert_eq!(offer.question.as_deref(), Some("Which branch?"));
        assert_eq!(offered(&[], &[ask(vec![q(false, 2)])]), None);
        assert_eq!(
            offered(&answer, &[ask(vec![q(false, 2), q(false, 2)])]),
            None
        );
        assert_eq!(offered(&answer, &[ask(vec![q(true, 2)])]), None);
        assert_eq!(offered(&answer, &[ask(vec![q(false, 4)])]), None);
    }

    fn ev(json: &str) -> Event {
        serde_json::from_str(json).unwrap()
    }

    /// The chat as the page draws it, top to bottom: each message's calls
    /// as `[name]` lines, then its words.
    fn order(ms: &[Message]) -> Vec<String> {
        let mut out = Vec::new();
        for m in ms {
            out.extend(m.tools.iter().map(|t| format!("[{}]", t.name)));
            if !m.text.is_empty() {
                out.push(m.text.clone());
            }
        }
        out
    }

    #[test]
    fn words_a_call_and_the_answer_read_in_the_order_they_came() {
        let ms = history(&[
            said(1, "user", "(sent while you were working) check the disk"),
            said(2, "talk", "Looking."),
            said(3, "tool", "Bash {\"command\":\"df\"}"),
            said(4, "echo", "/dev/sda 40%"),
            said(5, "talk", "40% used."),
        ]);
        assert_eq!(ms.len(), 3);
        assert_eq!(
            (ms[0].role, ms[0].text.as_str()),
            (Role::User, "check the disk")
        );
        assert_eq!(
            order(&ms),
            ["check the disk", "Looking.", "[Bash]", "40% used."]
        );
        assert!(ms[2].tools[0].done);
        assert_eq!(ms[2].date, Some(1_793_325_027_000));
    }

    #[test]
    fn notes_files_and_quiet_answers_read_as_the_page_shows_them() {
        let ms = history(&[
            said(
                1,
                "user",
                "[image]\n(sent a photo: a.png; a file: b.txt) look",
            ),
            said(2, "talk", "(sent a picture: chart.png)"),
            said(3, "user", "Routine r1 set, due now: feed the cat"),
            said(4, "talk", "NO_REPLY"),
        ]);
        assert_eq!(ms.len(), 2);
        assert_eq!(ms[0].text, "look");
        assert_eq!(ms[0].files, ["a.png", "b.txt"]);
        assert_eq!(ms[1].picture.as_deref(), Some("chart.png"));
    }

    // 11-02 19:11-19:28: one turn answered, called on, answered again, then
    // heard a worker's report and had nothing to say. A reload dropped every
    // answer in it.
    #[test]
    fn a_quiet_answer_takes_only_its_own_calls() {
        let ms = history(&[
            said(1, "user", "can this flake check happen in a sandbox?"),
            said(2, "tool", "ToolSearch {}"),
            said(3, "echo", "Loaded"),
            said(4, "talk", "Yes, it can."),
            said(5, "tool", "ToolSearch {}"),
            said(6, "echo", "Loaded"),
            said(7, "talk", "The memory problem was me."),
            said(8, "work", "[Heidi] reply\nBoth items are done."),
            said(9, "tool", "mcp__libri__tell_agent {}"),
            said(10, "echo", "Sent."),
            said(11, "talk", "(nothing new)"),
            said(12, "user", "chat is losing its order"),
        ]);
        assert_eq!(
            order(&ms),
            [
                "can this flake check happen in a sandbox?",
                "[ToolSearch]",
                "Yes, it can.",
                "[ToolSearch]",
                "The memory problem was me.",
                "chat is losing its order"
            ]
        );
    }

    #[test]
    fn a_quiet_routine_that_narrated_leaves_no_trace() {
        let ms = history(&[
            said(1, "user", "Routine r1 set, due now: check the mail"),
            said(2, "tool", "ToolSearch {}"),
            said(3, "echo", "Loaded"),
            said(4, "talk", "Checking calendar too."),
            said(5, "tool", "ToolSearch {}"),
            said(6, "echo", "Loaded"),
            said(7, "talk", "(nothing new)"),
        ]);
        assert!(ms.is_empty(), "{ms:?}");
    }

    #[test]
    fn a_quiet_reply_to_a_report_keeps_the_answer_not_its_narration() {
        let ms = history(&[
            said(1, "user", "is the build done?"),
            said(2, "tool", "ToolSearch {}"),
            said(3, "echo", "Loaded"),
            said(4, "talk", "It is."),
            said(5, "work", "[Heidi] reply\nAll green."),
            said(6, "tool", "ToolSearch {}"),
            said(7, "echo", "Loaded"),
            said(8, "talk", "Let me look closer."),
            said(9, "tool", "ToolSearch {}"),
            said(10, "echo", "Loaded"),
            said(11, "talk", "(nothing new)"),
        ]);
        let shown: Vec<_> = ms.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(shown, ["is the build done?", "It is."]);
        assert_eq!(ms[1].tools.len(), 1);
        assert!(!ms[1].progress);
    }

    // 11-04 06:03: a worker's report woke the turn, which answered it
    // "(nothing new)"; the person's message came and the same turn worked on,
    // and the quiet words stood above its next call.
    #[test]
    fn a_quiet_answer_inside_a_working_turn_never_shows() {
        let mut c = Conversation::default();
        for e in [
            r#"{"type":"RUN_STARTED"}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"a"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"a","delta":"On it."}"#,
            r#"{"type":"TEXT_MESSAGE_END","messageId":"a"}"#,
            r#"{"type":"TOOL_CALL_START","toolCallId":"t1","toolCallName":"Read","parentMessageId":"a"}"#,
            r#"{"type":"TOOL_CALL_RESULT","toolCallId":"t1","content":"ok"}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"b"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"b","delta":"(nothing new)"}"#,
            r#"{"type":"TEXT_MESSAGE_END","messageId":"b"}"#,
            r#"{"type":"TOOL_CALL_START","toolCallId":"t2","toolCallName":"Read","parentMessageId":"b"}"#,
            r#"{"type":"TOOL_CALL_RESULT","toolCallId":"t2","content":"ok"}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"c"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"c","delta":"NO_REPLY"}"#,
            r#"{"type":"TEXT_MESSAGE_END","messageId":"c"}"#,
            r#"{"type":"TOOL_CALL_START","toolCallId":"t3","toolCallName":"Read","parentMessageId":"x"}"#,
            r#"{"type":"TOOL_CALL_RESULT","toolCallId":"t3","content":"ok"}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"d"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"d","delta":"Done."}"#,
            r#"{"type":"TEXT_MESSAGE_END","messageId":"d"}"#,
        ] {
            c.apply(&ev(e));
        }
        assert_eq!(
            order(&c.messages),
            ["On it.", "[Read]", "[Read]", "[Read]", "Done."]
        );

        // And as hippo has it after a reload.
        let ms = history(&[
            said(1, "user", "get it done"),
            said(2, "tool", "ToolSearch {}"),
            said(3, "echo", "Loaded"),
            said(4, "talk", "It is."),
            said(5, "work", "[Moss] reply\nHolding."),
            said(6, "talk", "(nothing new)"),
            said(7, "tool", "ToolSearch {}"),
            said(8, "echo", "Loaded"),
            said(9, "talk", "Done."),
        ]);
        assert_eq!(
            order(&ms),
            [
                "get it done",
                "[ToolSearch]",
                "It is.",
                "[ToolSearch]",
                "Done."
            ]
        );
    }

    /// The Beck report turn of 11-06, as the daemon now sends it (each
    /// answer ended before the call after it): live and after a reload,
    /// each answer reads once, never the first one twice.
    #[test]
    fn a_report_turn_replayed_live_says_each_answer_once() {
        let mut c = Conversation::default();
        c.messages = history(&[said(1, "user", "Connections not accounts")]);
        for e in [
            r#"{"type":"RUN_STARTED"}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"a","role":"assistant"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"a","delta":"Beck is done."}"#,
            r#"{"type":"TEXT_MESSAGE_END","messageId":"a"}"#,
            r#"{"type":"TOOL_CALL_START","toolCallId":"t1","toolCallName":"mcp__libri__tell_agent"}"#,
            r#"{"type":"TOOL_CALL_ARGS","toolCallId":"t1","delta":"{\"name\":\"Sylvie\",\"message\":\"Connections.\"}"}"#,
            r#"{"type":"TOOL_CALL_END","toolCallId":"t1"}"#,
            r#"{"type":"TOOL_CALL_RESULT","toolCallId":"t1","content":"Sent."}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"b","role":"assistant"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"b","delta":"Passed on."}"#,
            r#"{"type":"TEXT_MESSAGE_END","messageId":"b"}"#,
            r#"{"type":"RUN_FINISHED","outcome":{"type":"success"}}"#,
        ] {
            c.apply(&ev(e));
        }
        // F13: in order as it happened, live and after a reload alike:
        // the words, the call's line, the next words, each answer once.
        let kept = history(&[
            said(1, "user", "Connections not accounts"),
            said(2, "talk", "Beck is done."),
            said(
                3,
                "tool",
                r#"mcp__libri__tell_agent {"name":"Sylvie","message":"Connections."}"#,
            ),
            said(4, "echo", "Sent."),
            said(5, "talk", "Passed on."),
        ]);
        let in_order = [
            "Connections not accounts",
            "Beck is done.",
            "[Message to Sylvie]",
            "Passed on.",
        ];
        assert_eq!(order(&c.messages), in_order);
        assert_eq!(order(&kept), in_order);
        assert_eq!(c.messages.len(), kept.len());
    }

    #[test]
    fn a_running_calls_output_shows_until_its_result() {
        let mut c = Conversation::default();
        for e in [
            r#"{"type":"RUN_STARTED"}"#,
            r#"{"type":"TOOL_CALL_START","toolCallId":"t1","toolCallName":"Bash"}"#,
            r#"{"type":"TOOL_CALL_END","toolCallId":"t1"}"#,
            r#"{"type":"ACTIVITY_SNAPSHOT","messageId":"output-t1","activityType":"tool_output","content":{"toolCallId":"t1","text":"line 1\n"}}"#,
        ] {
            c.apply(&ev(e));
        }
        let t = &c.messages.last().unwrap().tools[0];
        assert_eq!((t.result.as_deref(), t.done), (Some("line 1\n"), false));
        c.apply(&ev(
            r#"{"type":"TOOL_CALL_RESULT","toolCallId":"t1","content":"done"}"#,
        ));
        // A snapshot late after the result changes nothing.
        c.apply(&ev(r#"{"type":"ACTIVITY_SNAPSHOT","messageId":"output-t1","activityType":"tool_output","content":{"toolCallId":"t1","text":"line"}}"#));
        let t = &c.messages.last().unwrap().tools[0];
        assert_eq!((t.result.as_deref(), t.done), (Some("done"), true));
    }

    /// F13: words, calls and words again read the same live as after a
    /// reload, whether the words before a call were said or led into it.
    #[test]
    fn a_turn_reads_in_order_live_and_after_a_reload() {
        let mut c = Conversation::default();
        c.messages = history(&[said(1, "user", "check the disk")]);
        for e in [
            r#"{"type":"RUN_STARTED"}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"a"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"a","delta":"Looking."}"#,
            r#"{"type":"TOOL_CALL_START","toolCallId":"t1","toolCallName":"Bash","parentMessageId":"a"}"#,
            r#"{"type":"TOOL_CALL_ARGS","toolCallId":"t1","delta":"{\"command\":\"df\"}"}"#,
            r#"{"type":"TOOL_CALL_END","toolCallId":"t1"}"#,
            r#"{"type":"TOOL_CALL_RESULT","toolCallId":"t1","content":"40%"}"#,
            r#"{"type":"TOOL_CALL_START","toolCallId":"t2","toolCallName":"Bash","parentMessageId":"a"}"#,
            r#"{"type":"TOOL_CALL_ARGS","toolCallId":"t2","delta":"{\"command\":\"du\"}"}"#,
            r#"{"type":"TOOL_CALL_END","toolCallId":"t2"}"#,
            r#"{"type":"TOOL_CALL_RESULT","toolCallId":"t2","content":"9G"}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"b"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"b","delta":"Logs are big."}"#,
            r#"{"type":"TEXT_MESSAGE_END","messageId":"b"}"#,
            r#"{"type":"TOOL_CALL_START","toolCallId":"t3","toolCallName":"Read","parentMessageId":"x"}"#,
            r#"{"type":"TOOL_CALL_ARGS","toolCallId":"t3","delta":"{\"file_path\":\"/var/log\"}"}"#,
            r#"{"type":"TOOL_CALL_END","toolCallId":"t3"}"#,
            r#"{"type":"TOOL_CALL_RESULT","toolCallId":"t3","content":"ok"}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"c"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"c","delta":"40% used."}"#,
            r#"{"type":"TEXT_MESSAGE_END","messageId":"c"}"#,
            r#"{"type":"RUN_FINISHED","outcome":{"type":"success"}}"#,
        ] {
            c.apply(&ev(e));
        }
        let kept = history(&[
            said(1, "user", "check the disk"),
            said(2, "talk", "Looking."),
            said(3, "tool", r#"Bash {"command":"df"}"#),
            said(4, "echo", "40%"),
            said(5, "tool", r#"Bash {"command":"du"}"#),
            said(6, "echo", "9G"),
            said(7, "talk", "Logs are big."),
            said(8, "tool", r#"Read {"file_path":"/var/log"}"#),
            said(9, "echo", "ok"),
            said(10, "talk", "40% used."),
        ]);
        let in_order = [
            "check the disk",
            "Looking.",
            "[Bash]",
            "[Bash]",
            "Logs are big.",
            "[Read]",
            "40% used.",
        ];
        assert_eq!(order(&c.messages), in_order);
        assert_eq!(order(&kept), in_order);
        let shape = |ms: &[Message]| -> Vec<(String, Vec<String>)> {
            ms.iter()
                .map(|m| {
                    (
                        m.text.clone(),
                        m.tools.iter().map(|t| t.input.clone()).collect(),
                    )
                })
                .collect()
        };
        assert_eq!(shape(&c.messages), shape(&kept));
    }

    /// F12: an answer drawn live is named by its row once hippo takes it,
    /// so its quick replies and Reply show without a reload, quoting the
    /// row a reload would.
    #[test]
    fn a_live_answer_is_named_by_its_row_once_hippo_takes_it() {
        let mut c = Conversation::default();
        c.messages = history(&[said(20, "user", "ready to ship?")]);
        for e in [
            r#"{"type":"RUN_STARTED"}"#,
            r#"{"type":"TOOL_CALL_START","toolCallId":"t1","toolCallName":"mcp__libri__offer_replies","key":"kc"}"#,
            r#"{"type":"TOOL_CALL_ARGS","toolCallId":"t1","delta":"{\"options\":[\"Yes\",\"Not yet\"]}","key":"kc"}"#,
            r#"{"type":"TOOL_CALL_END","toolCallId":"t1","key":"kc"}"#,
            r#"{"type":"TOOL_CALL_RESULT","toolCallId":"t1","content":"Shown under your message."}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"m1","role":"assistant","key":"kt"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"m1","delta":"Deploy now?","key":"kt"}"#,
            r#"{"type":"TEXT_MESSAGE_END","messageId":"m1","key":"kt"}"#,
            r#"{"type":"RUN_FINISHED","outcome":{"type":"success"}}"#,
        ] {
            c.apply(&ev(e));
        }
        assert!(offered(&c.messages, &[]).is_none());
        for e in [
            r#"{"type":"CUSTOM","name":"row","value":{"key":"kc","row":21,"heard":[]}}"#,
            r#"{"type":"CUSTOM","name":"row","value":{"key":"kt","row":23,"heard":[]}}"#,
        ] {
            c.apply(&ev(e));
        }
        let live = offered(&c.messages, &[]).expect("replies on offer");
        assert_eq!(live.replies, ["Yes", "Not yet"]);
        assert_eq!(live.quote.as_ref().map(|q| q.id), Some(21));
        let kept = history(&[
            said(20, "user", "ready to ship?"),
            said(
                21,
                "tool",
                r#"mcp__libri__offer_replies {"options":["Yes","Not yet"]}"#,
            ),
            said(22, "echo", "Shown under your message."),
            said(23, "talk", "Deploy now?"),
        ]);
        assert_eq!(offered(&kept, &[]), Some(live));
        // The person's own message, by the id it was sent under.
        let mut c = Conversation::default();
        c.apply(&ev(
            r#"{"type":"TEXT_MESSAGE_CHUNK","role":"user","messageId":"local-1","delta":"hi"}"#,
        ));
        c.apply(&ev(
            r#"{"type":"CUSTOM","name":"row","value":{"key":"ku","row":30,"heard":["local-1"]}}"#,
        ));
        assert_eq!(c.messages[0].row, Some(30));
    }

    #[test]
    fn a_streamed_turn_folds_into_words_calls_and_an_answer() {
        let mut c = Conversation::default();
        c.said("local-1", "hello", Vec::new(), false, None);
        for e in [
            r#"{"type":"TEXT_MESSAGE_CHUNK","role":"user","messageId":"local-1","delta":"hello"}"#,
            r#"{"type":"RUN_STARTED"}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"a"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"a","delta":"Let me "}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"a","delta":"look."}"#,
            r#"{"type":"TOOL_CALL_START","toolCallId":"t1","toolCallName":"Read","parentMessageId":"a"}"#,
            r#"{"type":"TOOL_CALL_RESULT","toolCallId":"t1","content":"ok"}"#,
            r#"{"type":"TEXT_MESSAGE_START","messageId":"b"}"#,
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"b","delta":"Hi."}"#,
            r#"{"type":"TEXT_MESSAGE_END","messageId":"b"}"#,
        ] {
            assert_eq!(c.apply(&ev(e)), Applied::Done);
        }
        assert!(c.typing);
        assert_eq!(c.messages.len(), 3);
        assert_eq!(c.messages[0].outbox, Outbox::Arrived);
        assert_eq!(
            order(&c.messages),
            ["hello", "Let me look.", "[Read]", "Hi."]
        );
        let answer = &c.messages[2];
        assert!(answer.live);
        assert!(!c.messages[1].live);
        assert!(answer.tools[0].done);
        c.apply(&ev(
            r#"{"type":"RUN_FINISHED","outcome":{"type":"success"}}"#,
        ));
        assert!(!c.typing);
        assert!(!c.messages[1].live);
    }

    #[test]
    fn words_discarded_go_and_a_forgetting_asks_for_the_history_again() {
        let mut c = Conversation::default();
        c.apply(&ev(r#"{"type":"TEXT_MESSAGE_START","messageId":"a"}"#));
        c.apply(&ev(
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"a","delta":"x"}"#,
        ));
        c.apply(&ev(
            r#"{"type":"TEXT_MESSAGE_END","messageId":"a","metadata":{"discarded":true}}"#,
        ));
        assert!(c.messages.is_empty());
        assert_eq!(
            c.apply(&ev(r#"{"type":"CUSTOM","name":"forgot","value":{}}"#)),
            Applied::Resync
        );
        assert_eq!(
            c.apply(&ev(r#"{"type":"CUSTOM","name":"history","value":null}"#)),
            Applied::Resync
        );
    }

    #[test]
    fn the_host_marks_a_message_waiting_for_the_next_turn() {
        let mut c = Conversation::default();
        c.apply(&ev(
            r#"{"type":"TEXT_MESSAGE_CHUNK","role":"user","messageId":"m1","delta":"hi"}"#,
        ));
        c.apply(&ev(
            r#"{"type":"CUSTOM","name":"queued","value":{"id":"m1","on":true}}"#,
        ));
        assert!(c.messages[0].waits_turn);
        assert_eq!(c.messages[0].outbox, Outbox::Arrived);
        c.apply(&ev(
            r#"{"type":"CUSTOM","name":"queued","value":{"id":"m1","on":false}}"#,
        ));
        assert!(!c.messages[0].waits_turn);
    }

    #[test]
    fn a_turn_hippo_has_not_got_comes_from_the_hosts_record() {
        let history = [said(1, "user", "earlier"), said(2, "talk", "answer")];
        let task = TaskNow {
            typing: true,
            turn: Some(TurnRecord {
                seq: 4,
                shown: vec![
                    ev(
                        r#"{"type":"TEXT_MESSAGE_CHUNK","role":"user","messageId":"u","delta":"now this"}"#,
                    ),
                    ev(r#"{"type":"RUN_STARTED"}"#),
                    ev(r#"{"type":"TEXT_MESSAGE_START","messageId":"a"}"#),
                    ev(r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"a","delta":"Work"}"#),
                ],
            }),
        };
        let mut c = Conversation::default();
        c.sync(&history, &task);
        let texts: Vec<_> = c.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["earlier", "answer", "now this", "Work"]);
        assert!(c.typing);
        assert!(c.messages[3].live);
    }

    #[test]
    fn a_send_that_failed_is_marked_unless_the_host_heard_it() {
        let mut c = Conversation::default();
        c.said("x", "hi", Vec::new(), false, None);
        c.outbox("x", Outbox::Refused);
        assert_eq!(c.messages[0].outbox, Outbox::Refused);
        c.apply(&ev(
            r#"{"type":"TEXT_MESSAGE_CHUNK","role":"user","messageId":"x","delta":"hi"}"#,
        ));
        assert_eq!(c.messages[0].outbox, Outbox::Arrived);
        // A send's answer coming after the echo leaves it arrived.
        c.outbox("x", Outbox::Refused);
        assert_eq!(c.messages[0].outbox, Outbox::Arrived);
    }

    #[test]
    fn a_card_temp_drafted_says_so() {
        let mut c = Conversation::default();
        c.apply(&ev(r#"{"type":"ACTIVITY_SNAPSHOT","activityType":"approval","messageId":"act-8","content":{"id":8,"act":{"kind":"mail","account":null,"to":"a@b","subject":"Hi","body":"Text"},"event":null,"scratchpad":true}}"#));
        assert!(c.cards[0].scratchpad);
    }

    #[test]
    fn a_turn_that_asks_waits_and_its_jobs_and_cards_show() {
        let mut c = Conversation::default();
        c.apply(&ev(r#"{"type":"RUN_STARTED"}"#));
        c.apply(&ev(
            r#"{"type":"ACTIVITY_SNAPSHOT","activityType":"jobs","content":{"names":["build"]}}"#,
        ));
        c.apply(&ev(r#"{"type":"ACTIVITY_SNAPSHOT","activityType":"approval","messageId":"act-7","content":{"id":7,"act":{"kind":"mail","account":null,"to":"a@b","subject":"Hi","body":"Text"},"event":null}}"#));
        c.apply(&ev(r#"{"type":"RUN_FINISHED","outcome":{"type":"interrupt","interrupts":[{"id":"q1","reason":"tool","metadata":{"tool":"AskUserQuestion","input":{"questions":[{"question":"Which?","multiSelect":true,"options":[{"label":"A"},{"label":"B","description":"bee"}]}]}}}]}}"#));
        assert!(c.typing);
        assert_eq!(c.jobs, ["build"]);
        assert_eq!(c.cards[0].subject, "Hi");
        assert!(!c.cards[0].scratchpad);
        assert_eq!(
            c.asks[0].questions[0].options[1].description.as_deref(),
            Some("bee")
        );
        assert!(c.asks[0].questions[0].multi);
        c.apply(&ev(r#"{"type":"ACTIVITY_DELTA","activityType":"approval","messageId":"act-7","patch":[{"op":"add","path":"/outcome","value":{"text":"Sent.","done":true}}]}"#));
        assert!(c.cards.is_empty());
        assert_eq!(c.messages.last().unwrap().text, "Sent.");
        c.apply(&ev(
            r#"{"type":"RUN_FINISHED","outcome":{"type":"success"}}"#,
        ));
        assert!(!c.typing && c.asks.is_empty() && c.jobs.is_empty());
    }

    #[test]
    fn a_call_reads_by_its_command_and_keeps_its_result() {
        let mut c = Conversation::default();
        c.apply(&ev(
            r#"{"type":"TOOL_CALL_START","toolCallId":"t","toolCallName":"Bash"}"#,
        ));
        c.apply(&ev(
            r#"{"type":"TOOL_CALL_ARGS","toolCallId":"t","delta":"{\"command\":"}"#,
        ));
        c.apply(&ev(
            r#"{"type":"TOOL_CALL_ARGS","toolCallId":"t","delta":"\"ls\"}"}"#,
        ));
        c.apply(&ev(r#"{"type":"TOOL_CALL_END","toolCallId":"t"}"#));
        c.apply(&ev(
            r#"{"type":"TOOL_CALL_RESULT","toolCallId":"t","content":"a b"}"#,
        ));
        let t = &c.messages[0].tools[0];
        assert_eq!((t.input.as_str(), t.result.as_deref()), ("ls", Some("a b")));
    }

    #[test]
    fn a_word_that_the_scratchpad_changed_is_news_for_its_place() {
        let mut c = Conversation::default();
        c.apply(&ev(r#"{"type":"CUSTOM","name":"scratchpad","value":{}}"#));
        assert_eq!(c.temp_news, 1);
        assert!(c.messages.is_empty());
    }

    #[test]
    fn a_subagent_starting_or_ending_is_news_for_the_work_list() {
        let mut c = Conversation::default();
        c.apply(&ev(r#"{"type":"SUBAGENT_STARTED","subagentRunId":"s"}"#));
        c.apply(&ev(
            r#"{"type":"TEXT_MESSAGE_CONTENT","subagentRunId":"s","messageId":"a","delta":"x"}"#,
        ));
        c.apply(&ev(r#"{"type":"SUBAGENT_FINISHED","subagentRunId":"s"}"#));
        assert_eq!(c.work_news, 2);
    }

    #[test]
    fn a_page_published_live_is_told_once() {
        let mut c = Conversation::default();
        for e in [
            r#"{"type":"TOOL_CALL_START","toolCallId":"p","toolCallName":"mcp__libri__show_page"}"#,
            r#"{"type":"TOOL_CALL_RESULT","toolCallId":"p","content":"Published: https://app.libri.is/#page=how%20it-works (this version is kept)"}"#,
        ] {
            c.apply(&ev(e));
        }
        assert_eq!(
            c.published,
            Some(Published {
                page: "how it-works".into(),
                n: 1
            })
        );
        // A result whose start this client never heard, as after a
        // reconnect, opens nothing.
        c.apply(&ev(r#"{"type":"TOOL_CALL_RESULT","toolCallId":"q","content":"Published: https://app.libri.is/#page=other"}"#));
        assert_eq!(c.published.as_ref().map(|p| p.n), Some(1));
    }

    #[test]
    fn a_workers_calls_are_counted_and_its_report_back_is_not() {
        let mut c = Conversation::default();
        for e in [
            r#"{"type":"TOOL_CALL_START","subagentRunId":"Ada","toolCallId":"a","toolCallName":"Bash"}"#,
            r#"{"type":"TOOL_CALL_START","subagentRunId":"Ada","toolCallId":"b","toolCallName":"Read"}"#,
            r#"{"type":"TOOL_CALL_START","subagentRunId":"Ada","toolCallId":"c","toolCallName":"SubagentHandback"}"#,
            r#"{"type":"TOOL_CALL_START","toolCallId":"d","toolCallName":"Bash"}"#,
        ] {
            c.apply(&ev(e));
        }
        assert_eq!(c.worker_steps.get("Ada"), Some(&2));
        assert_eq!(c.worker_steps.len(), 1);
    }

    #[test]
    fn words_between_calls_come_back_from_the_history_in_order() {
        let ms = history(&[
            said(1, "user", "go"),
            said(2, "talk", "First."),
            said(3, "tool", "Bash {\"command\":\"a\"}"),
            said(4, "echo", "ok"),
            said(5, "tool", "Bash {\"command\":\"b\"}"),
            said(6, "echo", "ok"),
            said(7, "talk", "Second."),
            said(8, "tool", "Read {\"file_path\":\"c\"}"),
            said(9, "echo", "ok"),
            said(10, "talk", "Last."),
        ]);
        assert_eq!(
            order(&ms),
            [
                "go", "First.", "[Bash]", "[Bash]", "Second.", "[Read]", "Last."
            ]
        );
    }

    #[test]
    fn a_reply_followed_by_a_later_turns_calls_keeps_its_words() {
        let ms = history(&[
            said(1, "user", "go"),
            said(2, "talk", "All done."),
            said(3, "tool", "Bash {\"command\":\"a\"}"),
            said(4, "echo", "ok"),
            said(5, "talk", "Woke again."),
        ]);
        assert_eq!(order(&ms), ["go", "All done.", "[Bash]", "Woke again."]);
    }

    const LOOKUP: &str = r#"ToolSearch {"max_results":1,"query":"select:tell_agent"}"#;
    const LOOKUP_ARGS: &str = r#"{\"max_results\":1,\"query\":\"select:tell_agent\"}"#;
    const REPLY: &str = "Libri hung, and your restart fixed it.";

    fn said_at(i: u64, kind: &str, text: &str, time: &str) -> Said {
        Said {
            display: None,
            date: format!("2026-10-31T{time}-04:00"),
            ..said(i, kind, text)
        }
    }

    /// A morning as hippo held it on 10-31: a reply, a worker's report,
    /// then the same tool lookup again and again, words before only the first.
    fn morning() -> Vec<Said> {
        vec![
            said_at(456, "talk", REPLY, "10:59:50"),
            said_at(457, "work", "[Dara2] reply", "11:01:04"),
            said_at(458, "tool", LOOKUP, "11:01:11"),
            said_at(459, "echo", "Loaded", "11:01:12"),
            said_at(507, "talk", "Workers go on the left.", "11:16:56"),
            said_at(508, "user", "sounds good", "11:17:13"),
            said_at(509, "tool", LOOKUP, "11:17:19"),
            said_at(510, "echo", "Loaded", "11:17:26"),
            said_at(514, "talk", "Nell has the layout.", "11:17:30"),
            said_at(515, "user", "lets build the qt app", "11:18:22"),
            said_at(516, "tool", LOOKUP, "11:18:28"),
            said_at(517, "echo", "Loaded", "11:18:28"),
            said_at(523, "talk", "One build for Linux.", "11:18:43"),
            said_at(547, "user", "laptop is den", "11:34:04"),
            said_at(548, "tool", LOOKUP, "11:34:08"),
            said_at(549, "echo", "Loaded", "11:34:09"),
            said_at(555, "talk", "My mistake.", "11:34:19"),
        ]
    }

    fn upto(i: u64, history: &[Said]) -> Vec<Said> {
        history.iter().filter(|m| m.i <= i).cloned().collect()
    }

    /// Each call, with the model's words drawn just above it, if any.
    fn words(c: &Conversation) -> Vec<(&str, Option<&str>)> {
        let ms = &c.messages;
        let mut out = Vec::new();
        for (i, m) in ms.iter().enumerate() {
            for (k, t) in m.tools.iter().enumerate() {
                let above = (k == 0)
                    .then(|| i.checked_sub(1).map(|p| &ms[p]))
                    .flatten()
                    .filter(|p| p.role == Role::Assistant && !p.text.is_empty())
                    .map(|p| p.text.as_str());
                out.push((t.id.as_str(), above));
            }
        }
        out
    }

    /// Words the record holds, to be row `key`, after row `after`.
    fn spoken(key: &str, after: &str, text: &str) -> Vec<Event> {
        vec![
            ev(&format!(
                r#"{{"type":"TEXT_MESSAGE_START","messageId":"m-{key}","key":"{key}","after":"{after}"}}"#
            )),
            ev(&format!(
                r#"{{"type":"TEXT_MESSAGE_CONTENT","messageId":"m-{key}","delta":"{text}","key":"{key}","after":"{after}"}}"#
            )),
        ]
    }

    /// A call the record holds, to be row `key`, after row `after`.
    fn called(key: &str, after: &str, id: &str) -> Vec<Event> {
        let head = format!(r#""toolCallId":"{id}","key":"{key}","after":"{after}""#);
        vec![
            ev(&format!(
                r#"{{"type":"TOOL_CALL_START",{head},"toolCallName":"ToolSearch"}}"#
            )),
            ev(&format!(
                r#"{{"type":"TOOL_CALL_ARGS",{head},"delta":"{LOOKUP_ARGS}"}}"#
            )),
            ev(&format!(r#"{{"type":"TOOL_CALL_END",{head}}}"#)),
        ]
    }

    fn result(after: &str, id: &str, text: &str) -> Event {
        ev(&format!(
            r#"{{"type":"TOOL_CALL_RESULT","toolCallId":"{id}","content":"{text}","after":"{after}"}}"#
        ))
    }

    fn thinking(after: &str, text: &str) -> Event {
        ev(&format!(
            r#"{{"type":"REASONING_MESSAGE_CONTENT","messageId":"r","delta":"{text}","after":"{after}"}}"#
        ))
    }

    fn turn(typing: bool, shown: Vec<Event>) -> TaskNow {
        TaskNow {
            typing,
            turn: Some(TurnRecord { seq: 99, shown }),
        }
    }

    fn texts(c: &Conversation) -> Vec<&str> {
        c.messages
            .iter()
            .map(|m| m.text.as_str())
            .filter(|t| !t.is_empty())
            .collect()
    }

    #[test]
    fn an_old_reply_stays_where_it_was_said_sync_after_sync() {
        let all = morning();
        let mut c = Conversation::default();
        c.sync(&upto(459, &all), &TaskNow::default());
        for stop in [508, 515, 547, 555, 555] {
            c.sync(&upto(stop, &all), &TaskNow::default());
        }
        assert_eq!(
            words(&c),
            [
                ("h-458", Some(REPLY)),
                ("h-509", None),
                ("h-516", None),
                ("h-548", None),
            ]
        );
        assert_eq!(c.messages.last().unwrap().text, "My mistake.");
    }

    #[test]
    fn a_reload_and_a_second_sync_leave_an_old_reply_there_too() {
        let mut c = Conversation::default();
        c.sync(&morning(), &TaskNow::default());
        let once: Vec<_> = words(&c)
            .into_iter()
            .map(|(id, w)| (id.to_owned(), w.map(str::to_owned)))
            .collect();
        c.sync(&morning(), &TaskNow::default());
        let twice: Vec<_> = words(&c)
            .into_iter()
            .map(|(id, w)| (id.to_owned(), w.map(str::to_owned)))
            .collect();
        assert_eq!(once, twice);
        assert_eq!(words(&c).iter().filter(|(_, w)| w.is_some()).count(), 1);
    }

    #[test]
    fn hippos_rows_are_drawn_once_whatever_the_record_still_holds() {
        let shown = [
            spoken("k514", "k510", "Nell has the layout."),
            called("k509", "k508", "toolu_1"),
        ]
        .concat();
        let mut c = Conversation::default();
        c.sync(&upto(515, &morning()), &turn(true, shown));
        let all = texts(&c);
        assert_eq!(all.iter().filter(|t| t.starts_with("Nell")).count(), 1);
        assert_eq!(words(&c), [("h-458", Some(REPLY)), ("toolu_1", None)]);
    }

    #[test]
    fn words_hippo_lacks_stand_after_the_row_they_came_after() {
        let mut c = Conversation::default();
        let shown = spoken("words", "k508", "Looking that up.");
        c.sync(&upto(509, &morning()), &turn(true, shown));
        // The call's row settled after the words, so they lead into it.
        assert_eq!(
            words(&c),
            [("h-458", Some(REPLY)), ("h-509", Some("Looking that up."))]
        );
    }

    #[test]
    fn words_still_being_said_come_last_and_go_on_growing() {
        let mut c = Conversation::default();
        let shown = spoken("words", "k510", "Half a ");
        c.sync(&upto(510, &morning()), &turn(true, shown));
        c.apply(&ev(
            r#"{"type":"TEXT_MESSAGE_CONTENT","messageId":"m-words","delta":"thought"}"#,
        ));
        let last = c.messages.last().unwrap();
        assert_eq!(last.text, "Half a thought");
        assert!(last.live);
    }

    #[test]
    fn a_reply_hippo_never_took_stays_once_the_turn_is_over() {
        let mut c = Conversation::default();
        let shown = spoken("words", "k510", "The whole reply.");
        c.sync(&upto(510, &morning()), &turn(false, shown));
        assert_eq!(c.messages.last().unwrap().text, "The whole reply.");
        assert!(!c.messages.iter().any(|m| m.live));
    }

    #[test]
    fn a_result_finds_the_call_hippo_holds_by_the_records_id() {
        // A memory call is a row; its result never is.
        let mut shown = called("k509", "k508", "toolu_1");
        shown.push(result("k509", "toolu_1", "from memory"));
        let mut c = Conversation::default();
        c.sync(&upto(509, &morning()), &turn(true, shown));
        let tool = c.messages.last().unwrap().tools.last().unwrap();
        assert_eq!(tool.id, "toolu_1");
        assert_eq!(tool.result.as_deref(), Some("from memory"));
    }

    #[test]
    fn a_turn_under_way_joins_the_turn_hippo_holds() {
        let mut c = Conversation::default();
        c.sync(&upto(510, &morning()), &turn(true, Vec::new()));
        let before = c.messages.len();
        for e in called("next", "k510", "toolu_2") {
            c.apply(&e);
        }
        assert_eq!(c.messages.len(), before);
    }

    #[test]
    fn thinking_goes_with_the_call_it_led_to() {
        let mut c = Conversation::default();
        c.sync(
            &upto(509, &morning()),
            &turn(true, vec![thinking("k508", "Hmm.")]),
        );
        assert!(c.messages.last().unwrap().thinking.is_empty());
    }

    #[test]
    fn a_turn_begun_before_the_historys_window_keeps_its_order() {
        // hippo compacted: the turn's first rows are out of the window, and
        // the words held since them are older than all it shows.
        let all = morning();
        let window = &all[all.len() - 5..];
        let shown = [
            spoken("held", "gone", "Still unsaved."),
            vec![thinking("k549", "Late.")],
        ]
        .concat();
        let mut c = Conversation::default();
        c.sync(window, &turn(true, shown));
        assert_eq!(c.messages[0].text, "Still unsaved.");
        assert_eq!(texts(&c).last(), Some(&"My mistake."));
        // Nothing the record holds has a row in the window: it is the newest.
        c.sync(window, &turn(true, spoken("held", "gone", "Just now.")));
        assert_eq!(texts(&c).last(), Some(&"Just now."));
    }

    /// On 90eb71e a reload drew these twice on the web: hippo's row, and the
    /// record's copy. The record no longer takes them.
    fn sent_mid_turn() -> Vec<Said> {
        vec![
            said_at(1, "user", "make a chart", "12:00:00"),
            said_at(2, "tool", LOOKUP, "12:00:01"),
            said_at(3, "echo", "Loaded", "12:00:02"),
            said_at(4, "talk", "(sent a picture: chart.png)", "12:00:03"),
            said_at(5, "talk", "(sent a file: report.pdf)", "12:00:04"),
            said_at(6, "tool", LOOKUP, "12:00:05"),
        ]
    }

    /// What the turn draws, in order: its calls, then what was sent, in place.
    fn drawn(c: &Conversation) -> Vec<String> {
        c.messages
            .iter()
            .flat_map(|m| {
                let calls = m.tools.iter().map(|t| t.name.clone());
                calls.chain(m.picture.clone()).chain(m.file.clone())
            })
            .collect()
    }

    const DRAWN: [&str; 4] = ["ToolSearch", "chart.png", "report.pdf", "ToolSearch"];

    #[test]
    fn a_picture_or_file_hippo_holds_draws_once_on_a_load() {
        let shown = [called("k6", "k5", "toolu_2"), vec![thinking("k6", "Hm.")]].concat();
        for typing in [true, false] {
            let mut c = Conversation::default();
            c.sync(&sent_mid_turn(), &turn(typing, shown.clone()));
            assert_eq!(drawn(&c), DRAWN);
        }
    }

    #[test]
    fn a_picture_or_file_heard_live_draws_once_after_the_next_sync() {
        let all = sent_mid_turn();
        let mut c = Conversation::default();
        c.sync(&upto(3, &all), &turn(true, Vec::new()));
        for (kind, name) in [("picture", "chart.png"), ("file", "report.pdf")] {
            c.apply(&ev(&format!(
                r#"{{"type":"CUSTOM","name":"{kind}","value":{{"name":"{name}"}}}}"#
            )));
        }
        assert_eq!(drawn(&c), DRAWN[..3]);
        c.sync(&all, &turn(true, Vec::new()));
        assert_eq!(drawn(&c), DRAWN);
    }

    #[test]
    fn a_row_with_no_key_from_an_older_libri_still_shows() {
        let row = Said {
            display: None,
            src: None,
            ..said(1, "talk", "Old.")
        };
        assert_eq!(history(&[row])[0].text, "Old.");
    }

    #[test]
    fn the_persons_words_taking_their_row_asks_for_a_new_read() {
        let mut c = Conversation::default();
        let e = ev(r#"{"type":"CUSTOM","name":"stored"}"#);
        assert!(c.apply(&e) == Applied::Resync);
    }
}
