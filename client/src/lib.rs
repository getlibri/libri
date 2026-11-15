//! The Libri client core: what every native Libri app shares. It reaches a
//! host (a machine running the Libri daemon) at its address, reads the
//! conversation, follows it live over the host's event stream, and sends
//! the person's words. The apps bind it through UniFFI (Kotlin on Android)
//! and draw what it hands them.
//!
//! The tailnet is the trust boundary: an open host needs no sign-in.

mod chat;
mod host;
mod wire;

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chat::{Applied, Conversation};
pub use chat::{Ask, Card, Choice, Message, Outbox, Question, ReplyOffer, Role, ToolCall};
pub use host::{
    AccessMode, Account, Attachment, CallLine, CallPass, Connections, Fixed, Guide, Host, Limit,
    Maker, Makers, Me, ModelRef, Models, Offer, Scratchpad, SignedIn, Spend, SpendDay, SpendSum,
    Step, StepKind, SynopsisModel, Thread, Usage, Voice, WorkChoice, WorkerModel, WorkerNames,
};
pub use host::{Found, FoundHit, FoundLine, FoundSource};
use serde::{Deserialize, Serialize};
pub use wire::Quote;
use wire::{Decoder, Event};

uniffi::setup_scaffolding!();

/// How long to wait before reaching a host that did not answer again.
const RETRY: Duration = Duration::from_secs(3);
/// How often a message waiting for the host is tried again.
const FLUSH_EVERY: Duration = Duration::from_secs(5);
/// The longest the client follows one event stream before it reconnects
/// and reads the conversation afresh. ureq cannot time out a read that
/// stalls, so this bounds how long a connection that died unseen (a phone
/// that changed networks) holds the client, and how long a closed session
/// keeps its thread.
const STREAM_MAX: Duration = Duration::from_secs(10 * 60);

// Flat: Kotlin gets each error with its Display text as the message.
#[derive(uniffi::Error, Debug, Clone, PartialEq)]
#[uniffi(flat_error)]
pub enum ClientError {
    /// The address is not one the client can reach.
    Address { reason: String },
    /// The host did not answer, or a proxy answered for it while it restarts.
    Unreachable { reason: String },
    /// The host wants this device signed in.
    SignedOut,
    /// The host refused the request.
    Refused { status: u16, reason: String },
    /// The host refused and said why, in words for the person.
    Said { reason: String },
    /// The host answered with something the client cannot read.
    Unreadable { reason: String },
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Address { reason } => write!(f, "Not an address: {reason}"),
            ClientError::Unreachable { reason } => write!(f, "The host does not answer: {reason}"),
            ClientError::SignedOut => f.write_str("The host wants this device signed in."),
            ClientError::Said { reason } => f.write_str(reason),
            ClientError::Refused { status, reason } => {
                write!(f, "The host refused ({status}): {reason}")
            }
            ClientError::Unreadable { reason } => {
                write!(f, "The host's answer is unreadable: {reason}")
            }
        }
    }
}

impl std::error::Error for ClientError {}

impl From<ureq::Error> for ClientError {
    fn from(e: ureq::Error) -> Self {
        match e {
            ureq::Error::StatusCode(401) => ClientError::SignedOut,
            // nginx's answers while the daemon behind it restarts.
            ureq::Error::StatusCode(s @ (502..=504)) => ClientError::Unreachable {
                reason: format!("HTTP {s}"),
            },
            ureq::Error::StatusCode(status) => ClientError::Refused {
                status,
                reason: format!("HTTP {status}"),
            },
            e => ClientError::Unreachable {
                reason: e.to_string(),
            },
        }
    }
}

/// A host as /api/host describes itself, and how long it took to answer.
#[derive(uniffi::Record, Debug, Clone, PartialEq, Deserialize)]
pub struct HostInfo {
    pub id: String,
    pub name: String,
    /// The assistant's name; `info` fills it from `name` on a host from
    /// before it sent one apart, which gave the assistant's as its own.
    #[serde(default)]
    pub assistant: String,
    pub version: String,
    /// Seconds since the daemon started.
    pub uptime: u64,
    /// The address it gives for itself, if it gives one.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(skip)]
    pub millis: u64,
}

/// Where the client is with its host.
#[derive(uniffi::Enum, Debug, Clone, PartialEq)]
pub enum Link {
    Connecting,
    Live,
    Down {
        reason: String,
    },
    /// The host answers but will not have this device: retrying changes nothing.
    SignedOut,
}

/// What a client shows: the conversation, whether a turn is running, and
/// whether the host is with it.
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct Chat {
    pub messages: Vec<Message>,
    pub typing: bool,
    /// What the turn waits on the person for.
    pub asks: Vec<Ask>,
    /// The turn's background jobs, by name.
    pub jobs: Vec<String>,
    /// Acts waiting for the person's word.
    pub cards: Vec<Card>,
    /// Counts each subagent start and end heard: the Work list is out of date.
    pub work_news: u64,
    pub temp_news: u64,
    /// The calls each worker made, by its name, as far as this client heard.
    pub worker_steps: HashMap<String, u32>,
    pub worker_messages: HashMap<String, Vec<Message>>,
    pub link: Link,
    /// The replies the person may tap under the newest message.
    pub offer: Option<chat::ReplyOffer>,
    /// The page show_page last published live, to open by itself.
    pub published: Option<chat::Published>,
}

/// Told each time the chat changes, from the session's own thread.
#[uniffi::export(callback_interface)]
pub trait ChatListener: Send + Sync {
    fn changed(&self, chat: Chat);
}

/// A host's address as the person typed it, made whole: a bare name is
/// reached over https; one with a port, or a bare IP address, is a
/// tailnet or LAN address and reached over plain http, which the tailnet
/// encrypts.
#[uniffi::export]
pub fn address(typed: String) -> Result<String, ClientError> {
    let typed = typed.trim().trim_end_matches('/');
    if typed.is_empty() {
        return Err(ClientError::Address {
            reason: "it is empty".into(),
        });
    }
    if let Some((scheme, rest)) = typed.split_once("://") {
        if scheme != "http" && scheme != "https" || rest.is_empty() {
            return Err(ClientError::Address {
                reason: format!("{typed} is not an http or https address"),
            });
        }
        return Ok(typed.to_owned());
    }
    if typed.contains(char::is_whitespace) {
        return Err(ClientError::Address {
            reason: format!("{typed} has a space in it"),
        });
    }
    let host = typed.split('/').next().unwrap_or(typed);
    let ip = host
        .split(':')
        .next()
        .unwrap_or(host)
        .parse::<std::net::Ipv4Addr>()
        .is_ok();
    let port = host
        .rsplit_once(':')
        .is_some_and(|(_, p)| p.parse::<u16>().is_ok());
    Ok(format!(
        "{}://{typed}",
        if ip || port { "http" } else { "https" }
    ))
}

/// Asks the host at `address` who it is, timed.
#[uniffi::export]
pub fn probe(address: String) -> Result<HostInfo, ClientError> {
    Host::new(address)?.info()
}

/// A random version 4 UUID, as the page's `crypto.randomUUID`.
pub(crate) fn uuid() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the system's random source");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// The longest message text, in bytes, that /api/run takes (SEND_MAX in
/// site.rs); longer words go as a text file, read whole.
const SEND_MAX: usize = 32 * 1024;

/// The commands the composer knows (chat.ts COMMANDS): name, argument,
/// what it does.
const COMMANDS: [(&str, &str, &str); 12] = [
    (
        "model",
        "[name]",
        "show or switch the model for the next turns",
    ),
    (
        "effort",
        "[level]",
        "show or switch the thinking effort for the next turns",
    ),
    (
        "harness",
        "[name]",
        "show or switch the agent harness for the next turns",
    ),
    (
        "permission-mode",
        "[mode]",
        "show or switch the permission mode for the next turns",
    ),
    (
        "sandbox",
        "[setting]",
        "show or switch the harness's sandbox for the next turns",
    ),
    ("signin", "", "show which harnesses are signed in"),
    (
        "work",
        "[directions]",
        "create a task from this part of the chat, a thread of its own on the Tasks page",
    ),
    ("stop", "", "stop the turn running now"),
    (
        "memory",
        "[name]",
        "show which memory this chat is on, or open another memory's chat",
    ),
    ("help", "", "list these commands"),
    (
        "clear",
        "",
        "start a new session: the next turn opens afresh, given the chat's newest words",
    ),
    ("new", "", "the same as /clear"),
];

/// A slash command the composer offers as it is typed.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct Command {
    pub name: String,
    pub arg: String,
    pub does: String,
}

#[uniffi::export]
pub fn commands() -> Vec<Command> {
    COMMANDS
        .iter()
        .map(|(n, a, d)| Command {
            name: (*n).into(),
            arg: (*a).into(),
            does: (*d).into(),
        })
        .collect()
}

/// The memory `/memory <name>` names, by its id or its name in any case;
/// none for Main, a name that fits none, or words that aren't the command.
/// The app opens that memory's chat, as the web's tile does (App.tsx
/// useMemoryCommand); anything else goes on as the command.
#[uniffi::export]
pub fn memory_asked(text: String, memories: Vec<host::NamedMemory>) -> Option<String> {
    let rest = text.trim().strip_prefix("/memory")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let want = rest.trim();
    if want.is_empty() || want.contains(char::is_whitespace) {
        return None;
    }
    memories
        .into_iter()
        .find(|m| m.id.eq_ignore_ascii_case(want) || m.name.eq_ignore_ascii_case(want))
        .map(|m| m.id)
}

/// A message the host does not have: on its way, held for it, or refused/// A message the host does not have: on its way, held for it, or refused
/// by it and waiting for the person. Its record and files are kept on the
/// device, so they outlast the app.
#[derive(Clone, Serialize, Deserialize)]
struct Unsent {
    id: String,
    text: String,
    #[serde(default)]
    files: Vec<Attachment>,
    /// The earlier message it replies to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quote: Option<Quote>,
    /// Never `Arrived`: an arrived message leaves the list.
    #[serde(rename = "failed", default = "held", with = "refused")]
    state: Outbox,
}

fn held() -> Outbox {
    Outbox::Held
}

/// On disk only refusal counts: a message caught on its way when the app
/// died is held, and goes again.
mod refused {
    use super::Outbox;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(state: &Outbox, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bool(*state == Outbox::Refused)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Outbox, D::Error> {
        Ok(if bool::deserialize(d)? {
            Outbox::Refused
        } else {
            Outbox::Held
        })
    }
}

/// A refused message taken back: what goes into the composer again.
#[derive(uniffi::Record)]
pub struct Withdrawn {
    pub text: String,
    pub files: Vec<Attachment>,
}

struct State {
    conversation: Conversation,
    link: Link,
    unsent: Vec<Unsent>,
}

/// One host's conversation, followed live: open, it reads the history and
/// follows the event stream on a thread of its own, reconnecting and
/// reading afresh whenever the stream breaks, and tells its listener each
/// change. Detach it when the app stops showing it.
#[derive(uniffi::Object)]
pub struct Session {
    host: Arc<Host>,
    state: Mutex<State>,
    listener: Box<dyn ChatListener>,
    closed: AtomicBool,
    flushing: AtomicBool,
    /// Where the messages the host does not have are kept.
    store: Option<PathBuf>,
}

#[uniffi::export]
impl Session {
    /// `store` is a directory of the app's own, where messages the host does
    /// not have outlast the app; empty keeps them in memory only.
    #[uniffi::constructor]
    pub fn open(
        address: String,
        store: String,
        listener: Box<dyn ChatListener>,
    ) -> Result<Arc<Self>, ClientError> {
        let store = (!store.is_empty()).then(|| PathBuf::from(store).join("unsent.json"));
        let mut unsent: Vec<Unsent> = store
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if let Some(p) = &store {
            read_unsent_files(&p.with_extension(""), &mut unsent);
        }
        let mut conversation = Conversation::default();
        for u in &unsent {
            let names = u.files.iter().map(|f| f.name.clone()).collect();
            conversation.said(&u.id, &u.text, names, false, u.quote.clone());
            conversation.outbox(&u.id, u.state);
        }
        let session = Arc::new(Session {
            host: Host::new(address)?,
            state: Mutex::new(State {
                conversation,
                link: Link::Connecting,
                unsent,
            }),
            listener,
            closed: AtomicBool::new(false),
            flushing: AtomicBool::new(false),
            store,
        });
        let follower = session.clone();
        std::thread::Builder::new()
            .name("libri-session".into())
            .spawn(move || follower.follow())
            .map_err(|e| ClientError::Unreachable {
                reason: e.to_string(),
            })?;
        Ok(session)
    }

    /// The host, for the requests besides the conversation.
    pub fn host(&self) -> Arc<Host> {
        self.host.clone()
    }

    /// The address the session reaches its host at.
    pub fn address(&self) -> String {
        self.host.base.clone()
    }

    /// The chat as it stands.
    pub fn chat(&self) -> Chat {
        let s = self.state.lock().unwrap();
        let c = &s.conversation;
        Chat {
            messages: c.messages.clone(),
            typing: c.typing,
            asks: c.asks.clone(),
            jobs: c.jobs.clone(),
            cards: c.cards.clone(),
            work_news: c.work_news,
            temp_news: c.temp_news,
            worker_steps: c.worker_steps.clone(),
            worker_messages: c.worker_messages.clone(),
            link: s.link.clone(),
            offer: chat::offered(&c.messages, &c.asks),
            published: c.published.clone(),
        }
    }

    /// Sends the person's words and files: shown at once; queued while the
    /// host is down, and sent in order once it answers; marked failed if
    /// the host refused them. A command (/stop, /work, /help…) is carried
    /// out here, as the page does. `quote` names the earlier message it
    /// replies to. Blocks until the host answers; call it off the
    /// app's main thread.
    pub fn send(
        self: Arc<Self>,
        text: String,
        files: Vec<Attachment>,
        quote: Option<Quote>,
    ) -> Result<(), ClientError> {
        let mut text = text.trim().to_owned();
        let mut files = files;
        if text.is_empty() && files.is_empty() {
            return Ok(());
        }
        if files.is_empty() && text.starts_with('/') {
            match self.command(&text)? {
                Some(t) => text = t,
                None => return Ok(()),
            }
        }
        if text.len() > SEND_MAX {
            files.push(Attachment {
                name: "pasted.txt".into(),
                mime: "text/plain".into(),
                data: std::mem::take(&mut text).into_bytes(),
                view: None,
            });
        }
        let id = uuid();
        let names = files.iter().map(|f| f.name.clone()).collect();
        // What comes after a message on its way or held waits behind it.
        let behind = self
            .state
            .lock()
            .unwrap()
            .unsent
            .iter()
            .any(|u| u.state != Outbox::Refused);
        let unsent = Unsent {
            id: id.clone(),
            text: text.clone(),
            files,
            quote: quote.clone(),
            state: if behind {
                Outbox::Held
            } else {
                Outbox::Sending
            },
        };
        // On disk before the first try: the app may die during it.
        self.keep_files(&unsent);
        let files = if behind {
            Vec::new()
        } else {
            unsent.files.clone()
        };
        self.change(|s| {
            s.conversation.said(&id, &text, names, false, quote.clone());
            s.conversation.outbox(&id, unsent.state);
            s.unsent.push(unsent);
        });
        self.keep_unsent();
        if behind {
            self.flush_soon();
            return Ok(());
        }
        let sent = self.deliver(&id, &text, &files, None, quote);
        self.settle(&id, &sent);
        match sent {
            Err(ClientError::Unreachable { .. }) => Ok(()),
            r => r,
        }
    }

    /// Tries a refused message again, in its place, behind what waits.
    /// Answers false for one the core does not hold: a refused edit, whose
    /// words only the talk has.
    pub fn retry(self: Arc<Self>, id: String) -> bool {
        let mut held = false;
        self.change(|s| {
            if let Some(u) = s
                .unsent
                .iter_mut()
                .find(|u| u.id == id && u.state == Outbox::Refused)
            {
                u.state = Outbox::Held;
                held = true;
                s.conversation.outbox(&id, Outbox::Held);
            }
        });
        if held {
            self.keep_unsent();
            self.flush_soon();
        }
        held
    }

    /// Takes a message back from the talk and the store, for the composer.
    pub fn withdraw(&self, id: String) -> Option<Withdrawn> {
        let mut taken = None;
        self.change(|s| {
            let files = s.unsent.iter().position(|u| u.id == id);
            let files = files.map_or(Vec::new(), |at| s.unsent.remove(at).files);
            taken = s.conversation.withdrawn(&id).map(|m| Withdrawn {
                text: m.text,
                files,
            });
        });
        self.keep_unsent();
        self.drop_files(&id);
        taken
    }

    /// Where a waiting message's file named `name` is on the device, for
    /// showing it while the host does not have it.
    pub fn kept_file(&self, name: String) -> Option<String> {
        let s = self.state.lock().unwrap();
        s.unsent.iter().find_map(|u| {
            let at = u.files.iter().position(|f| f.name == name)?;
            let path = self.files_of(&u.id)?.join(at.to_string());
            Some(path.to_str()?.to_owned())
        })
    }

    /// The person's last message again: `edit` puts new words in its place,
    /// `again` draws the answer to it afresh.
    pub fn redo(&self, text: String, kind: String) -> Result<(), ClientError> {
        let id = uuid();
        self.change(|s| s.conversation.said(&id, &text, Vec::new(), true, None));
        let sent = self.deliver(&id, &text, &[], Some(kind), None);
        // Refused, the history is as it was: read it afresh.
        let now = if sent.is_ok() {
            Outbox::Arrived
        } else {
            Outbox::Refused
        };
        self.change(|s| s.conversation.outbox(&id, now));
        sent
    }

    /// Answers what the turn asked; one the turn took back goes.
    pub fn reply(
        &self,
        ask: String,
        yes: bool,
        answers: HashMap<String, String>,
        always: bool,
    ) -> Result<(), ClientError> {
        match self.host.reply(ask.clone(), yes, answers, always) {
            Err(ClientError::Refused { status: 409, .. }) => {
                self.change(|s| s.conversation.asks.retain(|a| a.id != ask));
                Ok(())
            }
            r => r,
        }
    }

    /// Stops following the host. The listener hears nothing more. (Not
    /// `close`: UniFFI's Kotlin gives every object a `close` that frees it.)
    pub fn detach(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

impl Session {
    fn closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Changes the state and tells the listener, unless closed.
    fn change(&self, f: impl FnOnce(&mut State)) {
        f(&mut self.state.lock().unwrap());
        if !self.closed() {
            self.listener.changed(self.chat());
        }
    }

    /// Carries out a command; answers the words to send instead, if any.
    fn command(&self, text: &str) -> Result<Option<String>, ClientError> {
        let (name, arg) = text[1..]
            .split_once(char::is_whitespace)
            .unwrap_or((&text[1..], ""));
        let name = name.to_lowercase();
        let arg = arg.trim();
        let note = |said: String| self.change(|s| s.conversation.note(&uuid(), &said));
        match name.as_str() {
            // Words to the assistant, which writes the brief and spawns it.
            "work" => Ok(Some(format!(
                "Create task: {}",
                if arg.is_empty() {
                    "carry out what we just settled"
                } else {
                    arg
                }
            ))),
            "stop" => self.host.stop_turn().map(|()| None),
            "help" => {
                let mut said = COMMANDS
                    .iter()
                    .map(|(n, a, d)| format!("/{n}{}{a}: {d}", if a.is_empty() { "" } else { " " }))
                    .collect::<Vec<_>>()
                    .join("\n");
                // The harness's own, as the web lists them (chat.ts help).
                if let Some(own) = self.host.harness_commands().ok().flatten() {
                    let theirs: Vec<String> = own
                        .commands
                        .iter()
                        .filter(|n| COMMANDS.iter().all(|(c, _, _)| c != n))
                        .map(|n| format!("/{n}"))
                        .collect();
                    if !theirs.is_empty() {
                        said.push_str(&format!(
                            "\n\n{}'s own, sent to it as typed: {}",
                            own.harness,
                            theirs.join(", ")
                        ));
                    }
                }
                note(said);
                Ok(None)
            }
            "clear" | "new" if arg.is_empty() => {
                note(match self.host.clear_session() {
                    Ok(()) => "Cleared: the next turn starts a new session, given the chat's newest words. Everything said stays in memory.".into(),
                    Err(ClientError::Refused { status: 409, .. }) => {
                        "A turn is running. /stop it or wait for it, then /clear.".into()
                    }
                    Err(e) => return Err(e),
                });
                Ok(None)
            }
            // A name that fits a memory the app opens before it gets here
            // (memory_asked); the main chat itself stays on Main.
            "memory" if !arg.contains(char::is_whitespace) => {
                let names: Vec<String> = self
                    .host
                    .memories()?
                    .into_iter()
                    .map(|m| m.name.to_lowercase())
                    .collect();
                let choices = if names.is_empty() {
                    " No other memory is set up.".to_owned()
                } else {
                    format!(
                        " /memory {} opens that memory's chat.",
                        names.join(", /memory ")
                    )
                };
                note(if arg.is_empty() || arg.eq_ignore_ascii_case("main") {
                    format!("This chat is on Main, and stays there.{choices}")
                } else {
                    format!("No memory {arg}.{choices}")
                });
                Ok(None)
            }
            "signin" => {
                let all = self.host.signed_in()?;
                note(if all.is_empty() {
                    "Only one harness here; it is signed in by the service.".into()
                } else {
                    all.iter()
                        .map(|s| {
                            format!(
                                "{}: {}",
                                s.harness,
                                if s.yes { "signed in" } else { "not signed in" }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                });
                Ok(None)
            }
            "model" | "harness" | "effort" => {
                let m = self.host.models()?;
                let want = arg.to_lowercase();
                let said = if name == "effort" {
                    let efforts = m
                        .offers
                        .iter()
                        .find(|o| o.harness == m.harness && o.model == m.model)
                        .map(|o| o.efforts.clone())
                        .unwrap_or_default();
                    if want.is_empty() {
                        format!(
                            "Thinking at {}. The choices are {}.",
                            if m.effort.is_empty() {
                                "the default effort"
                            } else {
                                &m.effort
                            },
                            efforts.join(", ")
                        )
                    } else if efforts.contains(&want) {
                        self.host.pick_effort(want.clone(), false)?;
                        format!("Switched to {want}, from the next turn on.")
                    } else {
                        format!("No effort {want}. The choices are {}.", efforts.join(", "))
                    }
                } else {
                    let field = |o: &Offer| {
                        if name == "model" {
                            o.model.clone()
                        } else {
                            o.harness.clone()
                        }
                    };
                    let now = m
                        .offers
                        .iter()
                        .find(|o| o.harness == m.harness && o.model == m.model)
                        .map(field)
                        .unwrap_or_default();
                    let mut choices: Vec<String> = m.offers.iter().map(field).collect();
                    choices.dedup();
                    match m.offers.iter().find(|o| field(o).to_lowercase() == want) {
                        _ if want.is_empty() => {
                            format!("Running on {now}. The choices are {}.", choices.join(", "))
                        }
                        Some(o) => {
                            self.host
                                .pick_model(o.harness.clone(), o.model.clone(), false)?;
                            format!("Switched to {want}, from the next turn on.")
                        }
                        None => {
                            format!("No {name} {want}. The choices are {}.", choices.join(", "))
                        }
                    }
                };
                note(said);
                Ok(None)
            }
            // Two words or more is a sentence that opens with the name.
            "permission-mode" | "sandbox" if !arg.contains(char::is_whitespace) => {
                let boxed = name == "sandbox";
                let m = self.host.models()?;
                let (word, now) = if boxed {
                    ("sandbox", &m.sandbox)
                } else {
                    ("mode", &m.access)
                };
                let held = m.modes.iter().find(|h| h.harness == m.harness);
                let all = held.map_or(&[][..], |h| if boxed { &h.sandbox } else { &h.access });
                let want = all.iter().find(|o| o.id.eq_ignore_ascii_case(arg));
                note(if all.is_empty() {
                    format!("{} has no {word} here.", m.harness)
                } else if arg.is_empty() {
                    let name = all.iter().find(|o| o.id == *now);
                    let lines = all.iter().map(|o| format!("{}: {}", o.id, o.note));
                    format!(
                        "On {}.\n{}",
                        name.map_or(now.as_str(), |o| &o.name),
                        lines.collect::<Vec<_>>().join("\n")
                    )
                } else if let Some(o) = want {
                    if o.id == *now {
                        format!("Already on {}.", o.name)
                    } else {
                        if boxed {
                            self.host.pick_sandbox(o.id.clone(), false)?;
                        } else {
                            self.host.pick_access(o.id.clone(), false)?;
                        }
                        format!("Switched to {}, from the next turn on.", o.name)
                    }
                } else {
                    let ids: Vec<&str> = all.iter().map(|o| o.id.as_str()).collect();
                    format!("No {word} {arg}. The choices are {}.", ids.join(", "))
                });
                Ok(None)
            }
            _ => self.harness_command(&name, text, &note),
        }
    }

    /// A slash command that isn't Libri's: one the main chat's harness has
    /// goes to it as typed; one it hasn't, but another harness has, gets a
    /// note rather than going to the model as words, as does /compact on a
    /// harness that compacts only on its own. Anything else is words.
    fn harness_command(
        &self,
        name: &str,
        text: &str,
        note: &dyn Fn(String),
    ) -> Result<Option<String>, ClientError> {
        let words = Ok(Some(text.to_owned()));
        let Some(own) = self.host.harness_commands().ok().flatten() else {
            return words;
        };
        if !own.known || own.commands.iter().any(|n| n == name) {
            return words;
        }
        let mut others: Vec<&str> = own
            .all
            .iter()
            .filter(|(h, ns)| **h != own.harness && ns.iter().any(|n| n == name))
            .map(|(h, _)| h.as_str())
            .collect();
        others.sort_unstable();
        if name == "compact" {
            note(format!(
                "{} compacts its session on its own as it fills; it can't be asked to here. /clear starts a new one.",
                own.harness
            ));
        } else if !others.is_empty() {
            note(format!(
                "/{name} is {}'s; {} can't run it here.",
                others.join(" and "),
                own.harness
            ));
        } else {
            return words;
        }
        Ok(None)
    }

    /// Uploads the files, then sends the message.
    fn deliver(
        &self,
        id: &str,
        text: &str,
        files: &[Attachment],
        redo: Option<String>,
        quote: Option<Quote>,
    ) -> Result<(), ClientError> {
        let mut uploaded = Vec::new();
        for f in files {
            uploaded.push(self.host.upload(f.clone())?);
        }
        self.host.run(
            id.to_owned(),
            text.to_owned(),
            files.to_vec(),
            uploaded,
            redo,
            quote,
        )
    }

    /// Writes the record of what the host does not have, with `also`, one
    /// not yet in the state. Under the lock, so the last writer is the last
    /// change.
    fn keep_unsent(&self) {
        let Some(p) = &self.store else { return };
        let s = self.state.lock().unwrap();
        if let Ok(b) = serde_json::to_vec(&s.unsent) {
            let tmp = p.with_extension("tmp");
            if std::fs::write(&tmp, b).is_ok() {
                std::fs::rename(tmp, p).ok();
            }
        }
    }

    fn files_of(&self, id: &str) -> Option<PathBuf> {
        Some(self.store.as_ref()?.with_extension("").join(id))
    }

    /// Writes a message's files beside the record: each by its place in
    /// the list, its view after it.
    fn keep_files(&self, u: &Unsent) {
        let Some(dir) = self.files_of(&u.id).filter(|_| !u.files.is_empty()) else {
            return;
        };
        std::fs::create_dir_all(&dir).ok();
        for (i, f) in u.files.iter().enumerate() {
            std::fs::write(dir.join(i.to_string()), &f.data).ok();
            if let Some(view) = &f.view {
                std::fs::write(dir.join(format!("{i}.view")), view).ok();
            }
        }
    }

    fn drop_files(&self, id: &str) {
        if let Some(dir) = self.files_of(id) {
            std::fs::remove_dir_all(dir).ok();
        }
    }

    /// Settles a message after a try at sending it. Arrived, it leaves the
    /// list; the host out of reach, it is held, and tried every few seconds;
    /// refused, it waits for the person. One the host echoed during the try
    /// has left the list already, and stays arrived.
    fn settle(self: &Arc<Self>, id: &str, sent: &Result<(), ClientError>) {
        let now = match sent {
            Ok(()) => Outbox::Arrived,
            Err(ClientError::Unreachable { .. }) => Outbox::Held,
            Err(_) => Outbox::Refused,
        };
        let mut gone = false;
        self.change(|s| {
            match s.unsent.iter().position(|u| u.id == id) {
                Some(at) if now == Outbox::Arrived => {
                    s.unsent.remove(at);
                    gone = true;
                }
                Some(at) => s.unsent[at].state = now,
                None => gone = true,
            }
            if !gone || now == Outbox::Arrived {
                s.conversation.outbox(id, now);
            }
        });
        self.keep_unsent();
        if gone {
            self.drop_files(id);
        }
        // Held behind this one's first try, or held again by it: either way
        // the flusher takes it from here.
        if self.waiting() {
            self.flush_soon();
        }
    }

    /// Starts the one flusher, unless it runs already. Whoever finds the
    /// `flushing` flag up leaves its message to the flusher that raised it.
    fn flush_soon(self: &Arc<Self>) {
        if self.flushing.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        std::thread::spawn(move || {
            loop {
                while !me.closed() && !me.flush() {
                    std::thread::sleep(FLUSH_EVERY);
                }
                me.flushing.store(false, Ordering::SeqCst);
                // A message queued after the last pass found nothing, but
                // before the flag fell, left itself to this flusher: look
                // again, unless another flusher has taken the flag since.
                if me.closed() || !me.waiting() || me.flushing.swap(true, Ordering::SeqCst) {
                    break;
                }
            }
        });
    }

    /// Whether a held message is next to go: the first one the host has
    /// not refused, so none goes while one before it is on its first try.
    fn waiting(&self) -> bool {
        let s = self.state.lock().unwrap();
        next_out(&s.unsent).is_some_and(|at| s.unsent[at].state == Outbox::Held)
    }

    /// Sends what is held, in order, until the host is down again; answers
    /// whether nothing more can go now. One it refuses stays, for the
    /// person; one still on its first try stops it, and its settling starts
    /// the flusher again. Only the flusher calls it.
    fn flush(self: &Arc<Self>) -> bool {
        loop {
            let mut next = None;
            self.change(|s| {
                let at = next_out(&s.unsent).filter(|&at| s.unsent[at].state == Outbox::Held);
                if let Some(u) = at.map(|at| &mut s.unsent[at]) {
                    u.state = Outbox::Sending;
                    s.conversation.outbox(&u.id, Outbox::Sending);
                    next = Some(u.clone());
                }
            });
            let Some(u) = next else {
                return true;
            };
            let sent = self.deliver(&u.id, &u.text, &u.files, None, u.quote.clone());
            self.settle(&u.id, &sent);
            if matches!(sent, Err(ClientError::Unreachable { .. })) {
                return false;
            }
        }
    }

    /// Reads the conversation, then follows the stream, until closed. The
    /// stream opens first, so nothing goes out between the read and it. A
    /// stream opened again names the last event taken, and the host sends
    /// only what came after it, or says to read the conversation whole.
    fn follow(self: Arc<Self>) {
        // The id of the newest event taken; empty until the first.
        let mut last = String::new();
        while !self.closed() {
            let failure = match self.open_stream(&last) {
                Ok(answer) => {
                    // Standing nowhere yet, it reads the conversation as the
                    // stream opens; otherwise the host says what it lacks.
                    let recorded = if last.is_empty() { self.read() } else { Ok(0) };
                    match recorded.and_then(|recorded| self.stream(answer, recorded, &mut last)) {
                        // A stream that ran its course is opened again at once.
                        Ok(()) => continue,
                        Err(e) => e,
                    }
                }
                Err(e) => e,
            };
            if self.closed() {
                break;
            }
            self.change(|s| {
                s.link = match failure {
                    ClientError::SignedOut => Link::SignedOut,
                    e => Link::Down {
                        reason: e.to_string(),
                    },
                }
            });
            std::thread::sleep(RETRY);
        }
    }

    /// Reads the conversation whole and takes it as it stands; answers the
    /// number of the turn record's newest event, which the stream's events
    /// up to are in the read already.
    fn read(self: &Arc<Self>) -> Result<u64, ClientError> {
        let (said, task, cards) = self.host.read()?;
        let recorded = task.turn.as_ref().map_or(0, |t| t.seq);
        let mut arrived = Vec::new();
        self.change(|s| {
            s.conversation.sync(&said, &task);
            s.conversation.cards = cards;
            s.link = Link::Live;
            // The host's record of the turn under way holds it:
            // it arrived, whatever its send last heard.
            let State {
                unsent,
                conversation,
                ..
            } = s;
            unsent.retain(|u| {
                let has = conversation.arrived(&u.id);
                if has {
                    arrived.push(u.id.clone());
                }
                !has
            });
        });
        if !arrived.is_empty() {
            self.keep_unsent();
            arrived.iter().for_each(|id| self.drop_files(id));
        }
        if self.waiting() {
            self.flush_soon();
        }
        Ok(recorded)
    }

    fn open_stream(&self, after: &str) -> Result<ureq::http::Response<ureq::Body>, ClientError> {
        Ok(stream_agent()
            .get(self.host.url("api/events"))
            .query("after", after)
            .header("accept", "text/event-stream")
            .call()?)
    }

    /// Follows /api/events, folding each event into the conversation, past
    /// number `recorded` of the turn record when a read held it already, and
    /// keeping in `last` where it stands. Answers Ok when the stream ends of
    /// itself.
    fn stream(
        self: &Arc<Self>,
        mut answer: ureq::http::Response<ureq::Body>,
        mut recorded: u64,
        last: &mut String,
    ) -> Result<(), ClientError> {
        let reader = BufReader::new(answer.body_mut().as_reader());
        let mut decoder = Decoder::default();
        for line in reader.lines() {
            if self.closed() {
                return Ok(());
            }
            let line = match line {
                Ok(l) => l,
                // The stream ran its time (STREAM_MAX): it opens again where
                // it stands.
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => return Ok(()),
                Err(e) => {
                    return Err(ClientError::Unreachable {
                        reason: e.to_string(),
                    });
                }
            };
            let Some(sse) = decoder.line(&line) else {
                continue;
            };
            let events = match sse.name.as_str() {
                "message" => serde_json::from_str::<Event>(&sse.data).map(|e| vec![e]),
                // What it missed while away, oldest first.
                "replay" => serde_json::from_str::<Vec<Event>>(&sse.data),
                // Too far behind, or the host restarted: it reads the whole.
                "sync" => {
                    recorded = self.read()?;
                    Ok(Vec::new())
                }
                // A ping: where it stands, when no event has said.
                _ => Ok(Vec::new()),
            };
            for event in events.unwrap_or_default() {
                if self.take(&event, recorded) {
                    recorded = self.read()?;
                }
            }
            if let Some(id) = sse.id {
                *last = id;
            }
        }
        Err(ClientError::Unreachable {
            reason: "the stream ended".into(),
        })
    }

    /// Folds one event into the conversation; answers whether the
    /// conversation must be read again.
    fn take(&self, event: &Event, recorded: u64) -> bool {
        let mut resync = false;
        let mut arrived = None;
        self.change(|s| {
            // The host's echo of a waiting message: it arrived.
            if event.kind == "TEXT_MESSAGE_CHUNK"
                && let Some(id) = &event.message_id
                && let Some(at) = s.unsent.iter().position(|u| &u.id == id)
            {
                arrived = Some(s.unsent.remove(at).id);
            }
            resync = event.seq.is_none_or(|n| n > recorded)
                && s.conversation.apply(event) == Applied::Resync;
        });
        if let Some(id) = arrived {
            self.keep_unsent();
            self.drop_files(&id);
        }
        resync
    }
}

/// Gives each kept message its files back, and clears away any file no
/// message names. One whose file is gone is marked refused: told, and not
/// sent short of it.
fn read_unsent_files(dir: &Path, unsent: &mut [Unsent]) {
    for u in unsent.iter_mut() {
        let at = dir.join(&u.id);
        for (i, f) in u.files.iter_mut().enumerate() {
            match std::fs::read(at.join(i.to_string())) {
                Ok(data) => f.data = data,
                Err(_) => u.state = Outbox::Refused,
            }
            f.view = std::fs::read(at.join(format!("{i}.view"))).ok();
        }
    }
    let Ok(kept) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in kept.flatten() {
        if !unsent.iter().any(|u| *u.id == *entry.file_name()) {
            std::fs::remove_dir_all(entry.path()).ok();
        }
    }
}

/// Where the next message to go stands in the list: the first the host has
/// not refused.
fn next_out(unsent: &[Unsent]) -> Option<usize> {
    unsent.iter().position(|u| u.state != Outbox::Refused)
}

/// How the client names itself to a host: the host reads from it which kind
/// of device a message came from (libri/src/origin.rs), on Android the app,
/// elsewhere the desktop app.
pub(crate) const USER_AGENT: &str = if cfg!(target_os = "android") {
    concat!("libri-android/", env!("CARGO_PKG_VERSION"))
} else {
    concat!("libri-desktop/", env!("CARGO_PKG_VERSION"))
};

fn stream_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(USER_AGENT)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(20)))
        .timeout_recv_body(Some(STREAM_MAX))
        .http_status_as_error(true)
        .build()
        .into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn memory_asked_names_a_memory_by_id_or_name() {
        let m = |id: &str, name: &str| crate::host::NamedMemory {
            id: id.into(),
            name: name.into(),
            working: false,
            asking: false,
        };
        let all = vec![m("jobs", "Jobs"), m("t1", "Tutor")];
        assert_eq!(
            super::memory_asked("/memory jobs".into(), all.clone()).as_deref(),
            Some("jobs")
        );
        assert_eq!(
            super::memory_asked(" /memory TUTOR ".into(), all.clone()).as_deref(),
            Some("t1")
        );
        for not in [
            "/memory",
            "/memory main",
            "/memory nope",
            "/memoryjobs",
            "/memory jobs now",
            "jobs",
        ] {
            assert_eq!(super::memory_asked(not.into(), all.clone()), None, "{not}");
        }
    }

    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    #[test]
    fn addresses_are_made_whole() {
        let a = |s: &str| address(s.into());
        assert_eq!(a("libri.example.com").unwrap(), "https://libri.example.com");
        assert_eq!(
            a(" https://libri.example.com/ ").unwrap(),
            "https://libri.example.com"
        );
        assert_eq!(a("box:8951").unwrap(), "http://box:8951");
        assert_eq!(a("100.64.0.1").unwrap(), "http://100.64.0.1");
        assert!(a("").is_err());
        assert!(a("ftp://x").is_err());
        assert!(a("two words").is_err());
    }

    #[test]
    fn ids_are_version_4_uuids() {
        let id = uuid();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert_ne!(id, uuid());
    }

    /// What an `answer` gives for the host to refuse the request.
    const REFUSED: &str = "refused";
    /// What an `answer` gives for a gateway that lost the host's answer.
    const UNREACHED: &str = "unreached";

    /// A host on loopback answering each request by its path, as many as
    /// `n` requests; what each request's first line and body were go to
    /// the channel.
    /// A stand-in host's answers for a test that talks to it at length.
    const MANY: usize = 1_000;

    fn host(n: usize, answer: fn(&str) -> String) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let at = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            // `n` counts the requests answered, the event stream's apart: the
            // stand-in closes each stream at once and the session opens
            // another (and reads the chat again), as often as its retry
            // allows, so under load a tight count ran out before the requests
            // a test waits on came. Tests that talk to the host at length give
            // it room (MANY) and it serves until they let go of `requests`.
            let mut served = 0;
            for stream in listener.incoming() {
                if served == n {
                    break;
                }
                let mut s = stream.unwrap();
                // The head, then as much body as it says it has.
                let mut got = Vec::new();
                let mut buf = [0u8; 4096];
                let request = loop {
                    let n = s.read(&mut buf).unwrap_or(0);
                    if n == 0 && !String::from_utf8_lossy(&got).contains("\r\n\r\n") {
                        // Closed before a whole head: no request to answer.
                        break String::new();
                    }
                    got.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&got).to_string();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .map(str::to_owned)
                            })
                            .and_then(|l| l.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if body.len() >= length || n == 0 {
                            break text;
                        }
                    }
                };
                if request.is_empty() {
                    continue;
                }
                let line = request.lines().next().unwrap_or("").to_owned();
                // The stream's place (`?after=`) is no part of what it answers.
                let path = line.split(' ').nth(1).unwrap_or("");
                let path = path.strip_suffix("?after=").unwrap_or(path).to_owned();
                if path != "/api/events" {
                    served += 1;
                }
                // A test that has let go of what the host heard is done
                // with it, and the host ends, as it does after `n` answers.
                let heard = tx.send(format!(
                    "{line}\n{}",
                    request.split_once("\r\n\r\n").map_or("", |(_, body)| body)
                ));
                let body = answer(&path);
                let status = match body.as_str() {
                    REFUSED => "403 Forbidden",
                    UNREACHED => "504 Gateway Timeout",
                    _ => "200 OK",
                };
                let kind = if path == "/api/events" {
                    "text/event-stream"
                } else {
                    "application/json"
                };
                write!(
                    s,
                    "HTTP/1.1 {status}\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                if heard.is_err() {
                    break;
                }
            }
        });
        (at, rx)
    }

    #[test]
    fn a_probe_reads_who_the_host_is() {
        let (at, _) = host(1, |_| {
            r#"{"id":"abc","name":"Ada","version":"0.1.0","uptime":5,"addresses":[]}"#.into()
        });
        let info = probe(at).unwrap();
        assert_eq!((info.name.as_str(), info.uptime), ("Ada", 5));
        // A host from before the two were apart gave the assistant's name.
        assert_eq!(info.assistant, "Ada");
    }

    #[test]
    fn a_probe_keeps_the_host_and_the_assistant_apart() {
        let (at, _) = host(1, |_| {
            r#"{"id":"abc","name":"box","assistant":"Ada","version":"0.1.0","uptime":5}"#.into()
        });
        let info = probe(at).unwrap();
        assert_eq!(
            (info.name.as_str(), info.assistant.as_str()),
            ("box", "Ada")
        );
    }

    #[test]
    fn a_recording_goes_as_the_body_and_its_words_come_back() {
        let (at, requests) = host(1, |_| r#"{"text":"move it to Tuesday"}"#.into());
        let said = Host::new(at)
            .unwrap()
            .transcribe(b"recording".to_vec(), "audio/mp4".into())
            .unwrap();
        assert_eq!(said, "move it to Tuesday");
        let asked = requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(asked.starts_with("POST /api/transcribe"));
        assert!(asked.ends_with("\nrecording"));
    }

    struct Heard(Mutex<mpsc::Sender<Chat>>);
    impl ChatListener for Heard {
        fn changed(&self, chat: Chat) {
            self.0.lock().unwrap().send(chat).ok();
        }
    }

    #[test]
    fn a_host_that_refuses_the_device_is_signed_out_not_down() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let at = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut s = stream.unwrap();
                // Only the request's arrival matters, not its bytes.
                let _request = s.read(&mut [0u8; 4096]).unwrap();
                write!(
                    s,
                    "HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                )
                .unwrap();
            }
        });
        let (tx, rx) = mpsc::channel();
        let session = Session::open(at, String::new(), Box::new(Heard(Mutex::new(tx)))).unwrap();
        until(&rx, |chat| {
            assert!(!matches!(chat.link, Link::Down { .. }));
            chat.link == Link::SignedOut
        });
        session.detach();
    }

    /// An address nothing answers at.
    /// A host that is down. Port 1 is below what an unprivileged process can
    /// listen on, so no test's stand-in can be there: a port bound and let
    /// go was taken by another test's stand-in under load, which answered.
    fn nobody() -> String {
        "http://127.0.0.1:1".to_owned()
    }

    fn store() -> String {
        let at = std::env::temp_dir().join(format!("libri-store-{}", uuid()));
        std::fs::create_dir_all(&at).unwrap();
        at.to_str().unwrap().to_owned()
    }

    fn open(at: String, store: &str) -> (Arc<Session>, mpsc::Receiver<Chat>) {
        let (tx, rx) = mpsc::channel();
        let session = Session::open(at, store.into(), Box::new(Heard(Mutex::new(tx)))).unwrap();
        (session, rx)
    }

    fn photo() -> Attachment {
        Attachment {
            name: "photo.jpg".into(),
            mime: "image/jpeg".into(),
            data: b"the photo".to_vec(),
            view: Some(b"its view".to_vec()),
        }
    }

    /// A host with nothing to say that takes what it is sent, or refuses
    /// sends if `refuse`.
    fn receiving(n: usize, refuse: bool) -> (String, mpsc::Receiver<String>) {
        fn takes(path: &str) -> String {
            match path {
                "/api/history" => "[]".into(),
                "/api/task" => r#"{"typing":false,"turn":null,"agents":[]}"#.into(),
                "/api/upload" => r#"{"id":"f1"}"#.into(),
                "/api/events" => ":\n\n".into(),
                _ => "{}".into(),
            }
        }
        fn refuses(path: &str) -> String {
            if path == "/api/run" {
                REFUSED.into()
            } else {
                takes(path)
            }
        }
        host(n, if refuse { refuses } else { takes })
    }

    /// The first change that is `want`, within one deadline for the whole
    /// wait: changes that keep coming (a stream retrying against a host
    /// gone) would otherwise keep a wait for one that never comes going for
    /// ever, as each came inside its own timeout.
    fn until(rx: &mpsc::Receiver<Chat>, want: impl Fn(&Chat) -> bool) -> Chat {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut last: Option<Chat> = None;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let Ok(chat) = rx.recv_timeout(left) else {
                // What the session last showed says which wait it was.
                let shown = last.map(|c| {
                    let tail: Vec<String> = c
                        .messages
                        .iter()
                        .rev()
                        .take(3)
                        .map(|m| m.text.clone())
                        .collect();
                    format!("{:?}, last messages {tail:?}", c.link)
                });
                panic!("the change waited for, within 30 s; last seen: {shown:?}");
            };
            if want(&chat) {
                return chat;
            }
            last = Some(chat);
        }
    }

    fn posted(requests: &mpsc::Receiver<String>, line: &str) -> Option<String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        std::iter::from_fn(|| {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            requests.recv_timeout(left).ok()
        })
        .find(|r| r.starts_with(line))
    }

    /// Waits for the kept record to hold nothing.
    fn emptied(store: &str) {
        let at = format!("{store}/unsent.json");
        for _ in 0..50 {
            if std::fs::read_to_string(&at).unwrap() == "[]" {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("the record still holds a message");
    }

    #[test]
    fn a_queued_photo_outlasts_the_app() {
        let store = store();
        let (down, rx) = open(nobody(), &store);
        down.clone()
            .send("see this".into(), vec![photo()], None)
            .unwrap();
        until(&rx, |c| {
            c.messages.last().is_some_and(|m| m.outbox == Outbox::Held)
        });
        down.detach();
        drop(down);

        let (at, requests) = receiving(40, false);
        let (up, rx) = open(at, &store);
        until(&rx, |c| c.messages.iter().any(|m| m.files == ["photo.jpg"]));
        let sent = posted(&requests, "POST /api/upload").expect("the photo goes");
        assert!(sent.contains("the photo") && sent.contains("its view"));
        let run = posted(&requests, "POST /api/run").expect("the words go after");
        assert!(run.contains("see this") && run.contains("photo.jpg"));
        emptied(&store);
        up.detach();
        assert_eq!(
            std::fs::read_dir(format!("{store}/unsent"))
                .unwrap()
                .count(),
            0
        );
    }

    /// Every request the host has seen until it goes quiet for a second.
    fn all(requests: &mpsc::Receiver<String>) -> Vec<String> {
        std::iter::from_fn(|| requests.recv_timeout(Duration::from_secs(1)).ok()).collect()
    }

    #[test]
    fn a_waiting_message_goes_once_however_many_ask_to_flush() {
        let store = store();
        let (down, rx) = open(nobody(), &store);
        down.clone().send("first".into(), Vec::new(), None).unwrap();
        until(&rx, |c| {
            c.messages.last().is_some_and(|m| m.outbox == Outbox::Held)
        });
        down.detach();
        drop(down);

        // A host slow to take a message: the reconnect's flush is still
        // sending the first when a send asks for a flush of its own.
        fn slow(path: &str) -> String {
            if path == "/api/run" {
                std::thread::sleep(Duration::from_millis(500));
            }
            match path {
                "/api/history" => "[]".into(),
                "/api/task" => r#"{"typing":false,"turn":null,"agents":[]}"#.into(),
                "/api/events" => ":\n\n".into(),
                _ => "{}".into(),
            }
        }
        let (at, requests) = host(40, slow);
        let (up, _rx) = open(at, &store);
        let mut runs = vec![posted(&requests, "POST /api/run").expect("the first goes")];
        up.clone().send("second".into(), Vec::new(), None).unwrap();
        emptied(&store);
        up.detach();
        runs.extend(
            all(&requests)
                .into_iter()
                .filter(|r| r.starts_with("POST /api/run")),
        );
        let count = |w: &str| runs.iter().filter(|r| r.contains(w)).count();
        assert_eq!((count("first"), count("second")), (1, 1), "{runs:?}");
        assert!(runs[0].contains("first"));
    }

    /// The id of the message the host below took but never answered for.
    static TOOK: Mutex<String> = Mutex::new(String::new());

    #[test]
    fn a_message_the_hosts_record_holds_is_not_sent_again() {
        // The host takes the message, but its answer is lost (a 504); the
        // record of the turn under way, read on reconnecting, holds it. A
        // 504 is held like any unreachable host, so the flusher may post it
        // once more before that read: the host drops a repeat by message
        // id. After the read, it never goes again.
        fn takes_unanswered(path: &str) -> String {
            match path {
                "/api/run" => UNREACHED.into(),
                "/api/history" => "[]".into(),
                "/api/task" => {
                    let id = TOOK.lock().unwrap().clone();
                    let chunk = serde_json::json!({"type": "TEXT_MESSAGE_CHUNK",
                        "role": "user", "messageId": id, "delta": "hi"});
                    serde_json::json!({"typing": true, "agents": [],
                        "turn": {"seq": 1, "shown": [chunk]}})
                    .to_string()
                }
                "/api/events" => ":\n\n".into(),
                _ => "{}".into(),
            }
        }
        let store = store();
        let (at, requests) = host(40, takes_unanswered);
        let (session, rx) = open(at, &store);
        until(&rx, |c| c.link == Link::Live);
        // Every post, from the first: the flusher may resend before the
        // test learns the id.
        let (posts, seen) = mpsc::channel();
        std::thread::spawn(move || {
            for r in requests.iter().filter(|r| r.starts_with("POST /api/run")) {
                if posts.send(r).is_err() {
                    break;
                }
            }
        });
        session.clone().send("hi".into(), Vec::new(), None).unwrap();
        let run = seen.recv_timeout(Duration::from_secs(8)).expect("it goes");
        let body: serde_json::Value =
            serde_json::from_str(run.split_once('\n').unwrap().1).unwrap();
        let id = body["messages"][0]["id"].as_str().unwrap().to_owned();
        *TOOK.lock().unwrap() = id.clone();

        let chat = until(&rx, |c| {
            c.messages
                .iter()
                .any(|m| m.id == id && m.outbox == Outbox::Arrived)
        });
        assert_eq!(chat.messages.iter().filter(|m| m.text == "hi").count(), 1);
        emptied(&store);
        // Up to the read that emptied the record, one repost is allowed.
        let before = std::iter::from_fn(|| seen.try_recv().ok()).count();
        assert!(before <= 1, "sent {before} more times before the read");
        // Past the flusher's next try, none at all.
        std::thread::sleep(FLUSH_EVERY + Duration::from_secs(1));
        let after = std::iter::from_fn(|| seen.try_recv().ok()).count();
        session.detach();
        assert_eq!(after, 0, "sent {after} more times after the read");
    }

    /// Lets the host below answer the photo's upload.
    static UPLOADED: (Mutex<bool>, std::sync::Condvar) =
        (Mutex::new(false), std::sync::Condvar::new());

    #[test]
    fn a_message_sent_behind_an_upload_waits_for_it() {
        // The first message's photo is still going up when the second is
        // sent: the second must not reach the host first.
        fn slow_upload(path: &str) -> String {
            match path {
                "/api/upload" => {
                    let (done, go) = &UPLOADED;
                    let _held = go.wait_while(done.lock().unwrap(), |d| !*d).unwrap();
                    r#"{"id":"f1"}"#.into()
                }
                "/api/history" => "[]".into(),
                "/api/task" => r#"{"typing":false,"turn":null,"agents":[]}"#.into(),
                "/api/events" => ":\n\n".into(),
                _ => "{}".into(),
            }
        }
        let store = store();
        let (at, requests) = host(40, slow_upload);
        let (session, rx) = open(at, &store);
        until(&rx, |c| c.link == Link::Live);
        let first = session.clone();
        let sending = std::thread::spawn(move || first.send("first".into(), vec![photo()], None));
        posted(&requests, "POST /api/upload").expect("the photo goes up");
        session
            .clone()
            .send("second".into(), Vec::new(), None)
            .unwrap();
        // The old flusher posted the second here, ahead of the first: give
        // it the time to, then let the upload finish.
        std::thread::sleep(Duration::from_millis(300));
        let (done, go) = &UPLOADED;
        *done.lock().unwrap() = true;
        go.notify_all();
        sending.join().unwrap().unwrap();
        let order: Vec<String> = std::iter::from_fn(|| posted(&requests, "POST /api/run"))
            .take(2)
            .map(|r| {
                let body: serde_json::Value =
                    serde_json::from_str(r.split_once('\n').unwrap().1).unwrap();
                body["messages"][0]["content"].to_string()
            })
            .collect();
        session.detach();
        assert_eq!(order.len(), 2, "both went");
        assert!(
            order[0].contains("first") && order[1].contains("second"),
            "{order:?}"
        );
    }

    #[test]
    fn a_refused_message_and_its_photo_outlast_the_app() {
        let store = store();
        let (at, _requests) = receiving(40, true);
        let (first, rx) = open(at, &store);
        until(&rx, |c| c.link == Link::Live);
        assert!(
            first
                .clone()
                .send("no".into(), vec![photo()], None)
                .is_err()
        );
        assert_eq!(
            first.chat().messages.last().unwrap().outbox,
            Outbox::Refused
        );
        first.detach();
        drop(first);

        let (at, requests) = receiving(40, false);
        let (again, rx) = open(at, &store);
        let chat = until(&rx, |c| c.link == Link::Live);
        let m = chat.messages.last().expect("the refused message stays");
        assert!(m.outbox == Outbox::Refused && m.text == "no");
        assert_eq!(m.files, ["photo.jpg"]);
        assert!(posted(&requests, "POST /api/run").is_none());

        // Tapped, it goes with its photo, and nothing is left behind.
        assert!(again.clone().retry(m.id.clone()));
        let sent = posted(&requests, "POST /api/upload").expect("the photo goes");
        assert!(sent.contains("the photo"));
        emptied(&store);
        again.detach();
    }

    #[test]
    fn a_refused_message_goes_back_in_the_box_whole() {
        let store = store();
        let (at, _requests) = receiving(40, true);
        let (session, rx) = open(at, &store);
        until(&rx, |c| c.link == Link::Live);
        assert!(
            session
                .clone()
                .send("no".into(), vec![photo()], None)
                .is_err()
        );
        let id = session.chat().messages.last().unwrap().id.clone();

        let taken = session.withdraw(id).unwrap();
        assert_eq!((taken.text.as_str(), taken.files.len()), ("no", 1));
        assert_eq!(taken.files[0].data, b"the photo");
        assert!(session.chat().messages.is_empty());
        session.detach();
        let left = std::fs::read_to_string(format!("{store}/unsent.json")).unwrap();
        assert_eq!(left, "[]");
        assert_eq!(
            std::fs::read_dir(format!("{store}/unsent"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn a_kept_record_reads_refusal_and_holding_as_before() {
        let kept = r#"[{"id":"a","text":"x","failed":true},{"id":"b","text":"y","failed":false},{"id":"c","text":"z"}]"#;
        let unsent: Vec<Unsent> = serde_json::from_str(kept).unwrap();
        let states: Vec<Outbox> = unsent.iter().map(|u| u.state).collect();
        assert_eq!(states, [Outbox::Refused, Outbox::Held, Outbox::Held]);
        // Caught on its way, a message is kept as held.
        let mut on_way = unsent[1].clone();
        on_way.state = Outbox::Sending;
        let back = serde_json::to_value([&unsent[0], &on_way]).unwrap();
        assert_eq!(back[0]["failed"], true);
        assert_eq!(back[1]["failed"], false);
    }

    #[test]
    fn a_refused_message_stays_through_a_reread() {
        let mut c = Conversation::default();
        c.said("x", "no", Vec::new(), false, None);
        c.outbox("x", Outbox::Refused);
        c.sync(&[], &wire::TaskNow::default());
        assert_eq!(c.messages[0].outbox, Outbox::Refused);
    }

    #[test]
    fn the_access_commands_reach_what_the_picker_hides() {
        let (at, requests) = host(MANY, |path| {
            match path {
            "/api/models" => r#"{"pick":{"harness":"claude","model":"opus"},"access":"manual",
                "sandbox":"off","offers":[],"modes":[{"harness":"claude","defaultAccess":"manual",
                "defaultSandbox":"off",
                "access":[{"id":"manual","name":"Manual","note":"Asks","open":false},
                    {"id":"dontAsk","name":"Don't ask","note":"Refuses","open":false,"hidden":true}],
                "sandbox":[{"id":"off","name":"Sandbox off","note":"Bare","open":true,"hidden":true},
                    {"id":"auto-allow","name":"Sandbox, auto-allow","note":"Unasked","open":false,"hidden":true}]}]}"#
                .into(),
            "/api/history" => "[]".into(),
            "/api/task" => r#"{"typing":false,"turn":null,"agents":[]}"#.into(),
            "/api/events" => ":\n\n".into(),
            _ => "{}".into(),
        }
        });
        let (tx, rx) = mpsc::channel();
        let session = Session::open(at, String::new(), Box::new(Heard(Mutex::new(tx)))).unwrap();
        let said = |text: &str, want: &str| {
            session.clone().send(text.into(), Vec::new(), None).unwrap();
            until(&rx, |chat| {
                chat.messages.last().is_some_and(|m| m.text.contains(want))
            });
        };
        said(
            "/permission-mode",
            "On Manual.\nmanual: Asks\ndontAsk: Refuses",
        );
        said("/sandbox", "On Sandbox off.");
        said(
            "/permission-mode nope",
            "No mode nope. The choices are manual, dontAsk.",
        );
        said("/sandbox off", "Already on Sandbox off.");
        said("/permission-mode dontask", "Switched to Don't ask");
        said("/sandbox auto-allow", "Switched to Sandbox, auto-allow");
        session.detach();
        let posts: Vec<String> =
            std::iter::from_fn(|| requests.recv_timeout(Duration::from_secs(5)).ok())
                .filter(|r| r.starts_with("POST"))
                .collect();
        assert_eq!(posts.len(), 2);
        assert!(posts[0].starts_with("POST /api/access"), "{}", posts[0]);
        let body = |p: &str| -> serde_json::Value {
            serde_json::from_str(p.split_once('\n').unwrap().1).unwrap()
        };
        assert_eq!(
            body(&posts[0]),
            serde_json::json!({"access": "dontAsk", "temp": false})
        );
        assert_eq!(
            body(&posts[1]),
            serde_json::json!({"sandbox": "auto-allow", "temp": false})
        );
    }

    #[test]
    fn a_session_reads_the_history_then_follows_the_stream_and_sends() {
        let (at, requests) = host(MANY, |path| {
            match path {
            "/api/history" => {
                r#"[{"i":1,"kind":"user","text":"hi","date":"2026-10-29T21:00:00-04:00","reaction":null}]"#.into()
            }
            "/api/task" => r#"{"typing":false,"turn":null,"agents":[]}"#.into(),
            "/api/events" => concat!(
                ":\n\n",
                "data: {\"type\":\"TEXT_MESSAGE_START\",\"messageId\":\"a\"}\n\n",
                "data: {\"type\":\"TEXT_MESSAGE_CONTENT\",\"messageId\":\"a\",\"delta\":\"hello\"}\n\n",
            )
            .into(),
            _ => "{}".into(),
        }
        });
        let (tx, rx) = mpsc::channel();
        let session = Session::open(at, String::new(), Box::new(Heard(Mutex::new(tx)))).unwrap();
        let wait = |want: &dyn Fn(&Chat) -> bool| until(&rx, want);
        let chat = wait(&|c| c.messages.last().is_some_and(|m| m.text == "hello"));
        assert_eq!(chat.messages[0].text, "hi");
        assert_eq!(chat.messages[1].text, "hello");
        session
            .clone()
            .send("go on".into(), Vec::new(), None)
            .unwrap();
        session.detach();
        let posted = std::iter::from_fn(|| requests.recv_timeout(Duration::from_secs(5)).ok())
            .find(|r| r.starts_with("POST"))
            .unwrap();
        let (line, body) = posted.split_once('\n').unwrap();
        assert!(line.starts_with("POST /api/run"));
        let body: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["threadId"], "main");
        assert_eq!(body["messages"][0]["content"], "go on");
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[test]
    fn a_stream_opened_again_takes_what_it_missed_without_reading_again() {
        let (at, requests) = host(MANY, |path| {
            match path {
            "/api/history" => "[]".into(),
            "/api/task" => r#"{"typing":true,"turn":null,"agents":[]}"#.into(),
            "/api/events" => concat!(
                "id: b.1\n",
                "data: {\"type\":\"TEXT_MESSAGE_START\",\"messageId\":\"a\"}\n\n",
            )
            .into(),
            "/api/events?after=b.1" => concat!(
                "event: replay\nid: b.2\n",
                "data: [{\"type\":\"TEXT_MESSAGE_CONTENT\",\"messageId\":\"a\",\"delta\":\"back\"}]\n\n",
            )
            .into(),
            _ => "{}".into(),
        }
        });
        let (tx, rx) = mpsc::channel();
        let session = Session::open(at, String::new(), Box::new(Heard(Mutex::new(tx)))).unwrap();
        let chat = until(&rx, |chat| {
            chat.messages.last().is_some_and(|m| m.text == "back")
        });
        assert_eq!(chat.messages.len(), 1);
        session.detach();
        let asked: Vec<String> =
            std::iter::from_fn(|| requests.recv_timeout(Duration::from_millis(500)).ok())
                .map(|r| r.lines().next().unwrap_or("").to_owned())
                .collect();
        assert_eq!(
            asked
                .iter()
                .filter(|r| r.starts_with("GET /api/history"))
                .count(),
            1,
            "{asked:?}"
        );
        assert!(
            asked
                .iter()
                .any(|r| r.starts_with("GET /api/events?after=b.1"))
        );
    }
}
