//! A host's requests besides the conversation's stream: what the app asks
//! and tells it on the person's word (uploads, picks, Work, Temp, the
//! past). Each call blocks until the host answers; the app calls them off
//! its main thread.

use std::collections::HashMap;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::chat::{Ask, Card, presented};
use crate::wire::{Said, TaskNow};
use crate::{ClientError, HostInfo, uuid};

/// A file the person sends: its bytes, and for a photo too big or of a
/// kind the model can't see, the app's JPEG of it (`view`).
#[derive(uniffi::Record, Clone, Debug, Deserialize, Serialize)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    /// Kept beside a waiting message's record, not in it.
    #[serde(skip)]
    pub data: Vec<u8>,
    #[serde(skip)]
    pub view: Option<Vec<u8>>,
}

/// How large a session grows before its harness compacts it.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CompactPoint {
    pub tokens: u64,
    /// Whether it is the person's own pick rather than the default.
    #[serde(default)]
    pub picked: bool,
    /// The smallest the host takes.
    #[serde(default)]
    pub least: u64,
}

/// One item on a phone workspace's bar: what it shows (a kind the host
/// declares, `tiles()`), and for some which one (a worker's id, a built-in's
/// id, `page:<name>`, a named memory's id for a chat, a Temp's or a shell's).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct PhonePane {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arg: Option<String>,
    /// A chat's memories open as tabs, `arg` the one shown ("" for Main;
    /// tiles.rs `tabs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[uniffi(default = None)]
    pub tabs: Option<Vec<String>>,
}

/// A phone workspace: a name and the items on its bar, in order.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct PhoneSpace {
    pub id: String,
    pub name: String,
    pub items: Vec<PhonePane>,
    /// The item an agent's tile_open asked shown (layout.rs `open_phone`),
    /// as its key: its kind, and `:arg` if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[uniffi(default = None)]
    pub shown: Option<String>,
}

/// A phone workspace closed into Recently closed (layout.rs `closed`), kept
/// until `closed_at` (Unix seconds) is 30 days past.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct PhoneClosed {
    pub id: String,
    pub name: String,
    pub items: Vec<PhonePane>,
    pub closed_at: u64,
}

/// The phone's workspaces as the host keeps them (layout.rs, Store::Phone),
/// apart from the wide page's tiles. No spaces: nothing kept yet, and the
/// app opens its first-run workspace.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct PhoneLayout {
    pub version: u64,
    /// Who made the latest change: `person` or `agent`.
    pub by: Option<String>,
    /// Whether the latest change can be undone.
    pub undo: bool,
    pub spaces: Vec<PhoneSpace>,
    /// The workspace shown.
    pub on: String,
    /// Recently closed, newest first.
    pub closed: Vec<PhoneClosed>,
}

impl PhoneLayout {
    fn read(v: Value) -> Result<Self, ClientError> {
        #[derive(Deserialize)]
        struct Kept {
            spaces: Vec<PhoneSpace>,
            on: String,
            #[serde(default)]
            closed: Vec<PhoneClosed>,
        }
        let kept: Option<Kept> = match &v["layout"] {
            Value::Null => None,
            l => Some(serde_json::from_value(l.clone()).map_err(unreadable_json)?),
        };
        let (spaces, on, mut closed) = kept.map_or((Vec::new(), String::new(), Vec::new()), |k| {
            (k.spaces, k.on, k.closed)
        });
        closed.sort_by_key(|c| std::cmp::Reverse(c.closed_at));
        Ok(PhoneLayout {
            version: v["version"].as_u64().unwrap_or(0),
            by: v["by"].as_str().map(str::to_owned),
            undo: v["undo"].as_bool().unwrap_or(false),
            spaces,
            on,
            closed,
        })
    }
}

fn unreadable_json(e: serde_json::Error) -> ClientError {
    ClientError::Unreadable {
        reason: e.to_string(),
    }
}

/// A model on offer, through the harness that runs it.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Offer {
    pub harness: String,
    pub model: String,
    pub name: String,
    pub provider: String,
    #[serde(default)]
    pub efforts: Vec<String>,
    #[serde(default, rename = "default")]
    pub preferred: bool,
    /// The subscription in the usage report it draws on.
    #[serde(default)]
    pub usage: Option<String>,
}

/// A permission mode or sandbox setting of a harness, by its own id and name.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct AccessMode {
    pub id: String,
    pub name: String,
    pub note: String,
    /// Nothing asks or bounds it.
    pub open: bool,
    /// Taken but not listed in the composer's picker, unless in force. A
    /// host from before this sends none.
    #[serde(default)]
    pub hidden: bool,
}

/// What one harness offers and starts on. No sandbox settings means it has
/// none, and its default sandbox is then empty.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessModes {
    pub harness: String,
    pub access: Vec<AccessMode>,
    pub sandbox: Vec<AccessMode>,
    pub default_access: String,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub default_sandbox: String,
}

/// One harness in Settings: whether it's on, its default model (empty for
/// its own), and how to sign it in should it show signed out.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct HarnessSetting {
    pub harness: String,
    pub on: bool,
    #[serde(default, deserialize_with = "text_or_empty")]
    pub model: String,
    #[serde(default)]
    pub remedy: String,
}

fn text_or_empty<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

/// One skill a harness finds, for the composer's `$` menu: "libri" for one
/// Libri installed, "yours" for the person's own.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct SkillOffer {
    pub name: String,
    pub description: String,
    pub source: String,
}

/// The skills one harness finds, and how a message names one: `/` for
/// Claude Code, `$` for the rest.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct SkillMenu {
    pub harness: String,
    pub mention: String,
    pub skills: Vec<SkillOffer>,
}

/// The models on offer and what is picked: model, thinking effort (empty
/// for the harness's own default), and the harness's own access mode and
/// sandbox setting (empty where it has none).
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct Models {
    pub harness: String,
    pub model: String,
    pub effort: String,
    pub access: String,
    pub sandbox: String,
    /// What each harness offers; a host from before these lists gives none.
    pub modes: Vec<HarnessModes>,
    pub offers: Vec<Offer>,
    /// The harness the other makers' models run in, and those it may be.
    pub others: Option<String>,
    pub others_offered: Vec<String>,
    /// The harnesses switched on but not signed in; a host from before
    /// this list gives none.
    pub signed_out: Vec<String>,
    /// Each harness the host runs, for Settings; an older host gives none.
    pub harnesses: Vec<HarnessSetting>,
    pub default_harness: String,
    /// Temp's own pick; a host that has none gives the main chat's.
    pub temp_harness: String,
    pub temp_model: String,
    pub temp_effort: String,
    pub temp_access: String,
    pub temp_sandbox: String,
}

/// A model that makes something besides answers, by who serves it.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Maker {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub name: String,
}

/// What makes what here besides answers (/api/capabilities): pictures by
/// a model the person picks; the rest fixed, each by its one service.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct Makers {
    pub pictures: Vec<Maker>,
    pub picked: Option<Maker>,
    /// What each fixed capability is done by, as (capability, name).
    pub fixed: Vec<Fixed>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Fixed {
    pub capability: String,
    pub name: String,
}

/// What a call needs (/api/call): a short-lived pass to the voice service,
/// the voice, and the instructions with the newest of the memory.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct CallPass {
    pub secret: String,
    pub voice: String,
    pub instructions: String,
    /// Where the call goes, when the host names a place other than xAI.
    #[serde(default)]
    pub url: Option<String>,
}

/// One thing said on a call: `user` or `talk`.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct CallLine {
    pub kind: String,
    pub text: String,
}

/// Another person's assistant this one is paired with, or a request to
/// pair either way (/api/peers).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Peer {
    /// Its name; empty while this side's request waits.
    #[serde(default)]
    pub name: String,
    /// Where it takes calls, which names it here.
    pub url: String,
    /// `asking` (this side asked), `asked` (it asks) or `paired`.
    pub state: String,
    /// When a message last passed either way, in Unix seconds.
    #[serde(default)]
    pub last: Option<u64>,
}

/// A server the host's deployment adds to every turn
/// (/api/connections/host): shown in Settings, never changed there.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct HostServer {
    pub name: String,
    /// How many tools it lists; None when it did not answer.
    #[serde(default)]
    pub tools: Option<u32>,
}

/// The accounts elsewhere this host is connected to (/api/connections).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Connections {
    /// The MCP servers added, by name.
    #[serde(default)]
    pub servers: Vec<Server>,
    #[serde(default)]
    pub xai: Option<String>,
    /// The xAI sign-in under way: the page to approve it on, and its code.
    #[serde(default)]
    pub xai_signing: Option<XaiSigning>,
    /// What went wrong with the last xAI sign-in.
    #[serde(default)]
    pub xai_said: Option<String>,
}

/// The apps let in at the host's /mcp (/api/connectors): the address to
/// give them, the modules of tools there are, and each connector.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct OtherApps {
    pub url: String,
    pub modules: Vec<ConnectorModule>,
    pub connectors: Vec<Connector>,
}

/// A set of tools a connector may be let use.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct ConnectorModule {
    pub id: String,
    pub title: String,
    pub about: String,
    /// Whether a new connector gets it unless the person picks otherwise.
    pub start: bool,
}

/// An app let in: its name, its modules, the app that signed in to it (none
/// for one that came by key), and when it was made and last used (Unix
/// seconds, 0 for never).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Connector {
    pub id: String,
    pub name: String,
    pub modules: Vec<String>,
    pub app: Option<String>,
    pub made: u64,
    pub used: u64,
}

/// A connector just made by key, and its key, shown to the person once.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct ConnectorMade {
    pub connector: Connector,
    pub key: String,
}

/// A built-in (/api/builtins): Mail or Calendar, filled by the connected
/// account that covers it.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Builtin {
    pub id: String,
    pub title: String,
    pub about: String,
    pub on: bool,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub provider_title: Option<String>,
    /// The connection it is reached by, listed among the connections;
    /// None until one covers it.
    #[serde(default)]
    pub connection: Option<String>,
    #[serde(default)]
    pub signed_in: bool,
    /// The connected accounts that cover it: a pick shows with two.
    #[serde(default)]
    pub choices: Vec<BuiltinChoice>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct BuiltinChoice {
    pub id: String,
    pub title: String,
    pub connection: String,
}

/// A provider Libri can connect (/api/connections/providers): one sign-in
/// fills every built-in it covers.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Provider {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub note: Option<String>,
    /// The built-ins it covers, by title.
    #[serde(default)]
    pub covers: Vec<String>,
    #[serde(default)]
    pub connection: Option<String>,
    #[serde(default)]
    pub signed_in: bool,
}

/// One tool of a connection and its rule: "allow", "ask" or "off".
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct ToolRule {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub rule: String,
    /// The rule its annotations give.
    pub default: String,
}

/// One kind of thing a connection brings in (its calendars).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct ItemKind {
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub failed: Option<String>,
    #[serde(default)]
    pub items: Vec<Item>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Item {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    pub shown: bool,
}

/// One connection, an MCP server: its secrets by name alone.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Server {
    pub name: String,
    /// A URL, or the command it runs by.
    pub address: String,
    pub remote: bool,
    #[serde(default)]
    pub secrets: Vec<String>,
    #[serde(default)]
    pub withheld: Vec<String>,
    /// Each tool its server listed last, with its rule.
    #[serde(default)]
    pub tools: Vec<ToolRule>,
    pub on: bool,
    #[serde(default)]
    pub signed_in: bool,
    #[serde(default)]
    pub account: Option<String>,
}

/// What testing a connection found (/api/connections/servers/NAME/test).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Tested {
    /// The server wants a sign-in before it lists anything.
    pub sign_in: bool,
    #[serde(default)]
    pub tools: Vec<ServerTool>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct ServerTool {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub withheld: bool,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct XaiSigning {
    pub url: String,
    pub code: String,
}

/// What a Work thread's next run goes on: its own, else the main chat's.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct WorkChoice {
    pub harness: String,
    pub model: String,
    pub effort: String,
    pub access: String,
    pub sandbox: String,
}

impl WorkChoice {
    fn of(v: &Value) -> Self {
        let s = |x: &Value| x.as_str().unwrap_or("").to_owned();
        WorkChoice {
            harness: s(&v["pick"]["harness"]),
            model: s(&v["pick"]["model"]),
            effort: s(&v["effort"]),
            access: s(&v["access"]),
            sandbox: s(&v["sandbox"]),
        }
    }
}

/// A model on offer, by the harness that runs it.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct ModelRef {
    pub harness: String,
    pub model: String,
}

/// Who writes workers' titles and summaries (/api/synopsis). The host holds
/// the choice, so every device shows the same one.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct SynopsisModel {
    /// The person's pick; none for the default.
    pub picked: Option<ModelRef>,
    /// The cheapest this host can run; none when nothing cheap can.
    #[serde(rename = "default")]
    pub cheapest: Option<ModelRef>,
}

/// What new workers start on (/api/worker-model). The host holds the choice,
/// so every device shows the same one.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct WorkerModel {
    /// The person's pick, even while it is off the list; none follows the
    /// main chat.
    pub picked: Option<ModelRef>,
}

/// The names new workers are given (/api/worker-names). The host holds the
/// list, so every device shows the same one.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct WorkerNames {
    pub builtin: Vec<String>,
    pub added: Vec<String>,
    /// The person's own list, kept while it is off.
    pub own: Vec<String>,
    /// Whether their own list stands in for the built-in and added names.
    pub use_own: bool,
    pub excluded: Vec<String>,
    /// How many new workers can be given; none means they are numbered.
    pub allowed: u32,
    /// What a pasted list held that can't be names.
    #[serde(default)]
    pub left_out: Vec<String>,
}

/// The person's standing rules (/api/guide), given to every model. The host
/// holds them, so every device shows the same text.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Guide {
    pub text: String,
}

/// How the memory is working (/api/memory). The host decides the state and
/// the words; only `state` is always there. Moments stay as the host's RFC
/// 3339, for the page to show in its own format.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct MemoryHealth {
    pub state: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub waiting: Option<u64>,
    #[serde(default)]
    pub newest_summary: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// The lines in the log, one for each message.
    #[serde(default)]
    pub lines: Option<u64>,
    /// The bytes the memory keeps on the host's disk.
    #[serde(default)]
    pub bytes: Option<u64>,
}

/// A voice the assistant may read aloud in.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Voice {
    pub id: String,
    pub name: String,
}

/// How much of one subscription window is used (usage.rs `Window`).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Limit {
    pub window: String,
    pub name: String,
    pub pct: f64,
    pub resets: String,
    /// Its length, for the pace mark; none from a host before 0.9.39.
    #[serde(default)]
    pub hours: Option<f64>,
    /// Unix seconds.
    #[serde(default)]
    pub resets_at: Option<i64>,
}

/// A subscription, its windows shortest first (usage.rs `Account`).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Account {
    pub account: String,
    pub name: String,
    pub windows: Vec<Limit>,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub plan: Option<String>,
    /// When the host last read the provider, Unix seconds; 0 when it
    /// doesn't say.
    #[serde(default)]
    pub read_at: i64,
    /// The last read failed: these windows are from `read_at`.
    #[serde(default)]
    pub stale: bool,
}

/// One group's sums over a span (ledger.rs `Sum`).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct SpendSum {
    pub key: String,
    pub name: String,
    pub calls: u64,
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
    /// USD at list API prices.
    pub cost: f64,
    pub unpriced: u64,
}

/// One day's cost (ledger.rs `Day`).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct SpendDay {
    pub day: String,
    pub cost: f64,
}

/// What model calls cost at API prices over a span, every way it is cut
/// (ledger.rs `Spend`).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Spend {
    pub total: SpendSum,
    pub by_caller: Vec<SpendSum>,
    pub by_model: Vec<SpendSum>,
    pub by_company: Vec<SpendSum>,
    pub by_harness: Vec<SpendSum>,
    pub by_paid: Vec<SpendSum>,
    pub days: Vec<SpendDay>,
    pub prices_checked: String,
}

/// Every subscription, and whether the composers show the rings.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Usage {
    pub shown: bool,
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// A page show_page published (library.rs Page).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct LibraryPage {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    /// Seconds since the epoch.
    pub changed: u64,
}

/// A list in the assistant's book (library.rs List).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct LibraryList {
    pub name: String,
    pub items: Vec<String>,
}

/// What the Library holds: the pages, the latest first, and the lists.
#[derive(uniffi::Record, Clone, Debug, Default, PartialEq, Deserialize)]
pub struct LibraryShelf {
    pub pages: Vec<LibraryPage>,
    pub lists: Vec<LibraryList>,
}

/// A watch or reminder, as the Tasks page lists it (library.rs Shown).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct WatchItem {
    pub id: u64,
    /// When it next fires, in seconds since the epoch; none for a push.
    #[serde(default)]
    pub at: Option<u64>,
    #[serde(default)]
    pub every: Option<String>,
    /// `note` or `ask`.
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub worker: Option<String>,
    pub active: bool,
    #[serde(default)]
    pub looks: Option<String>,
    #[serde(default)]
    pub failing: Option<String>,
    /// The look's arguments, as JSON.
    #[serde(default)]
    pub arguments: Option<String>,
    /// Each test as `field op value`.
    #[serde(default)]
    pub tests: Vec<String>,
    /// The fields of an item its turn sees.
    #[serde(default)]
    pub fields: Vec<String>,
    /// Who else a turn's answer goes to.
    #[serde(default)]
    pub shared: Vec<String>,
    /// A push wakes it.
    #[serde(default)]
    pub pushed: bool,
}

/// One thread on the Work page (task::Thread).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Thread {
    /// What every call about it takes; `name` comes back round. A host from
    /// before IDs sends none, and names it by `name`: stable hosts, until
    /// they take the worker names change.
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub brief: String,
    /// A cheap model's few words on the work; the card shows the brief's
    /// first line while there is none.
    #[serde(default)]
    pub title: Option<String>,
    /// working, waiting, napping, sleeping or archived.
    pub state: String,
    /// What it is doing now, while it is working.
    #[serde(default)]
    pub summary: Option<String>,
    /// What its card says of it, chosen by the host; empty from a host from
    /// before it.
    #[serde(default)]
    pub gist: String,
    /// Seconds since the epoch.
    pub at: u64,
    #[serde(default)]
    pub report: Option<String>,
    pub read: bool,
    /// Who it is paused on: its job isn't over, but no run is going.
    #[serde(default)]
    pub waiting_on: Vec<String>,
    /// When its nap ends, in seconds since the epoch, while it has one.
    #[serde(default)]
    pub nap: Option<u64>,
    /// What a Waiting card waits for: review, approval or answer; for the
    /// last two `gist` is the worker's ask. None from a host from before it.
    #[serde(default)]
    pub need: Option<String>,
    /// The branches an archived worker left with work not on main.
    #[serde(default)]
    pub unlanded: Vec<String>,
    /// It has been merged into the main chat and done nothing since.
    #[serde(default)]
    pub merged: bool,
    /// The worker that asked for it, for a sub-worker; it hangs from that
    /// worker in the tree.
    #[serde(default)]
    pub head: Option<String>,
    /// The named memory whose chat started it; none for Main's.
    #[serde(default)]
    pub memory: Option<String>,
    /// What it runs on; none from a host from before the tree.
    #[serde(default)]
    pub pick: Option<ThreadPick>,
    /// The thinking level its next run is held to; none is the harness's
    /// default.
    #[serde(default)]
    pub effort: Option<String>,
    /// How its harness's report of the model and level it runs on differs
    /// from what it was started on, if it does.
    #[serde(default)]
    pub mismatch: Option<String>,
    /// Its last run failed.
    #[serde(default)]
    pub failed: bool,
    /// The tool its open call runs, while it works and has one open.
    #[serde(default)]
    pub calling: Option<String>,
}

/// A worker's harness and model (catalog::Pick).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct ThreadPick {
    pub harness: String,
    pub model: String,
}

/// A message between two agents in the last few seconds (task::Talk): a
/// worker's id, or `memory:<id>` for a memory's chat (`memory:main` the main
/// chat's).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Talk {
    /// Milliseconds since the epoch.
    pub at: u64,
    pub from: String,
    pub to: String,
}

/// The Work page's list (/api/work): the threads and the talk among them.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct WorkList {
    pub threads: Vec<Thread>,
    /// None from a host from before the tree.
    #[serde(default)]
    pub talk: Vec<Talk>,
}

/// The main chat's harness and the slash commands it has of its own
/// (/api/commands): one of them alone goes to it as typed. `known` is false
/// until a harness that reports its commands has; `all` holds every
/// harness's.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct HarnessCommands {
    pub harness: String,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub known: bool,
    #[serde(default)]
    pub all: HashMap<String, Vec<String>>,
}

/// A tool that draws a card, by the name a call carries, and whether its
/// card opens by itself under a call of the person's turn.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct CardTool {
    pub name: String,
    pub opens: bool,
}

/// A named memory (/api/memories), a root of the Workers tree with its
/// chat's state.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct NamedMemory {
    pub id: String,
    pub name: String,
    /// Its chat is at work.
    #[serde(default)]
    pub working: bool,
    /// Its chat waits on the person: a permission prompt or a question.
    #[serde(default)]
    pub asking: bool,
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    /// What the thread was asked to do.
    Task,
    /// Its words between calls, and its report last.
    Words,
    /// A tool call, with what came back.
    Call,
    /// The person's words to it, typed on its page.
    Told,
    /// A file it sent, by name.
    File,
    /// A scratchpad's thinking, while its run is at it.
    Thought,
}

/// One step of a thread's work, in order.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct Step {
    pub arguments: Option<String>,
    pub id: Option<String>,
    pub kind: StepKind,
    /// The tool's name, for a call.
    pub name: String,
    pub text: String,
    pub result: Option<String>,
}

/// A message search found: `src` is empty for the chat, else the worker's
/// folder; `at` is its id there. The matched words sit between U+0002 and
/// U+0003 in `text`, the words around the match.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct FoundHit {
    pub src: String,
    pub at: u64,
    pub kind: String,
    /// Unix seconds.
    pub time: i64,
    pub text: String,
}

/// A search's hits, best or newest first. `loose`: no message had every
/// word, so these have some.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Found {
    pub hits: Vec<FoundHit>,
    pub loose: bool,
}

/// A line of the chat or a worker's log, around a hit.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct FoundLine {
    pub at: u64,
    pub kind: String,
    pub time: i64,
    pub text: String,
    /// The text was cut short.
    #[serde(default)]
    pub cut: bool,
}

/// A worker search reaches, by the name people know it by.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct FoundSource {
    pub src: String,
    pub name: String,
    pub began: i64,
    pub archived: bool,
}

/// Who answers here.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Me {
    #[serde(default)]
    pub app: String,
    pub name: String,
    /// The assistant's voice; none when it has no voice to read aloud with.
    #[serde(default)]
    pub voice: Option<String>,
    /// Whether the person may pick models here.
    #[serde(default)]
    pub models: bool,
    /// The memory windows the person may pick, in minutes.
    #[serde(default)]
    pub forgets: Vec<u64>,
    /// What this device may do: `read` alone, or none for everything.
    #[serde(default)]
    pub scope: Option<String>,
    /// Whether the host can write speech down, for the composer's mic.
    #[serde(default)]
    pub dictate: bool,
    /// Whether the host can take a call with the chat; an older host says
    /// nothing, and calls where it has a voice.
    #[serde(default)]
    pub call: Option<bool>,
}

/// Where the host writes speech down (`api/stt`).
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct Speech {
    pub choice: SpeechChoice,
    pub models: Vec<SpeechModel>,
    /// Whether the host carries its own speech engine.
    pub local: bool,
    pub xai: bool,
    pub openai_key: bool,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct SpeechChoice {
    /// `local`, `xai` or `openai`.
    pub provider: String,
    /// The local model, by id.
    pub model: String,
}

/// One of the host's own speech models.
#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct SpeechModel {
    pub id: String,
    pub name: String,
    pub about: String,
    pub bytes: u64,
    pub installed: bool,
    /// Bytes so far and in all while it is fetched.
    #[serde(default)]
    pub downloading: Option<SpeechFetch>,
}

#[derive(uniffi::Record, Clone, Debug, PartialEq, Deserialize)]
pub struct SpeechFetch {
    pub got: u64,
    pub total: u64,
}

/// The temp chat as the host has it.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct Scratchpad {
    pub id: Option<String>,
    pub steps: Vec<Step>,
    pub working: bool,
    /// Whether its next turn has the memory.
    pub memory: bool,
    pub asks: Vec<Ask>,
    /// Whether its run was told to stop, from any client.
    pub stopping: bool,
}

impl Scratchpad {
    fn of(v: &Value) -> Self {
        let mut thread = steps(v["text"].as_str().unwrap_or(""));
        // What the run is at goes after its log, and what waits for it last.
        let live = |kind, text: &str| Step {
            arguments: None,
            id: None,
            kind,
            name: String::new(),
            text: text.to_owned(),
            result: None,
        };
        for (kind, text) in [(StepKind::Thought, "thinking"), (StepKind::Words, "words")] {
            let text = v[text].as_str().unwrap_or("");
            if !text.trim().is_empty() {
                thread.push(live(kind, text.trim()));
            }
        }
        for words in v["queued"].as_array().into_iter().flatten() {
            thread.extend(steps(&format!(
                "0|user: {}",
                words.as_str().unwrap_or("").replace('\n', "\n  ")
            )));
        }
        // Nobody set it a task: its first words are the person's as the rest.
        for s in &mut thread {
            if s.kind == StepKind::Task {
                s.kind = StepKind::Told;
            }
        }
        Self {
            id: v["id"].as_str().map(str::to_owned),
            steps: thread,
            working: v["working"].as_bool().unwrap_or(false),
            memory: v["memory"].as_bool().unwrap_or(true),
            stopping: v["stopping"].as_bool().unwrap_or(false),
            asks: v["asks"]
                .as_array()
                .map(|all| all.iter().filter_map(Ask::of).collect())
                .unwrap_or_default(),
        }
    }
}

/// Whether a harness is signed in.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct SignedIn {
    pub harness: String,
    pub yes: bool,
}

/// A thread's or subagent's log, `{i}|{kind}: {text}` per message, read
/// as its page shows it (chat.ts `fromAgentChat`).
pub fn steps(text: &str) -> Vec<Step> {
    let mut out: Vec<Step> = Vec::new();
    let mut tasked = false;
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let starts = line
            .split_once('|')
            .is_some_and(|(i, _)| !i.is_empty() && i.bytes().all(|b| b.is_ascii_digit()));
        match lines.last_mut() {
            Some(last) if !starts => {
                last.push('\n');
                last.push_str(line);
            }
            _ => lines.push(line.to_owned()),
        }
    }
    // Where each call with an id stands in `out`.
    let mut calls: Vec<(usize, String)> = Vec::new();
    for m in lines {
        let Some((_, rest)) = m.split_once('|') else {
            continue;
        };
        let Some((head, body)) = rest.split_once(": ") else {
            continue;
        };
        let kind = head.split(' ').next().unwrap_or("");
        // `tool [id]` and `echo [id]`: the call a result answers. A log
        // from before ids pairs by order.
        let key = head.split_once(" [").and_then(|(_, k)| k.strip_suffix(']'));
        // The host indents an entry's further lines: only it starts a line.
        let body = &body.replace("\n  ", "\n");
        let step = |kind, name: &str, text: &str| Step {
            arguments: None,
            id: key.map(str::to_owned),
            kind,
            name: name.to_owned(),
            text: text.to_owned(),
            result: None,
        };
        match kind {
            "user" if !tasked => {
                tasked = true;
                out.push(step(StepKind::Task, "", body));
            }
            "user" => out.push(step(StepKind::Told, "", body)),
            // A row, not a bubble: a bubble is the person's own words.
            "from" => {
                let (who, said) = body.split_once(": ").unwrap_or(("a worker", body));
                let name = match who.strip_suffix(" (reply)") {
                    Some(who) => format!("Reply from {who}"),
                    None => format!("Message from {who}"),
                };
                // The words stand as its input, for the row's gist; a
                // result, if empty, is what says it is done.
                let mut row = step(StepKind::Call, &name, said.trim());
                row.result = Some(String::new());
                out.push(row);
            }
            "tool" => {
                let (name, input) = body.split_once(' ').unwrap_or((body, ""));
                // A subagent hands its report back in a call: its last words.
                if name == "SubagentHandback" {
                    let report = serde_json::from_str::<Value>(input)
                        .ok()
                        .and_then(|v| v["message"].as_str().map(str::to_owned))
                        .unwrap_or_else(|| input.to_owned());
                    out.push(step(StepKind::Words, "", &report));
                } else {
                    let arguments = input.to_owned();
                    let (name, input) = presented(name, input);
                    if let Some(key) = key {
                        calls.push((out.len(), key.to_owned()));
                    }
                    let mut call = step(StepKind::Call, &name, &input);
                    call.arguments = Some(arguments);
                    out.push(call);
                }
            }
            "echo" => {
                let keyed = key.and_then(|key| {
                    calls
                        .iter()
                        .find(|(at, id)| id == key && out[*at].result.is_none())
                        .map(|(at, _)| *at)
                });
                let open = keyed.or_else(|| {
                    out.iter()
                        .position(|s| s.kind == StepKind::Call && s.result.is_none())
                });
                if let Some(at) = open {
                    out[at].result = Some(body.to_owned());
                }
            }
            "file" => out.push(step(StepKind::File, "", body)),
            "talk" => match out.last_mut() {
                Some(last) if last.kind == StepKind::Words => {
                    last.text.push_str("\n\n");
                    last.text.push_str(body);
                }
                _ => out.push(step(StepKind::Words, "", body)),
            },
            _ => {}
        }
    }
    out
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(crate::USER_AGENT)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(timeout))
        .timeout_recv_body(Some(timeout))
        .http_status_as_error(true)
        .build()
        .into()
}

fn unreadable(e: ureq::Error) -> ClientError {
    match e {
        ureq::Error::Json(e) => ClientError::Unreadable {
            reason: e.to_string(),
        },
        e => e.into(),
    }
}

/// A host the app reaches by its address: every request but the stream.
#[derive(uniffi::Object)]
pub struct Host {
    pub(crate) base: String,
    agent: ureq::Agent,
    /// For what the host takes long over: a model list.
    slow: ureq::Agent,
}

impl Host {
    /// A socket's address: ws or wss as the host is reached.
    fn socket_url(&self, path: &str) -> String {
        let at = self.url(path);
        at.strip_prefix("https://")
            .map(|r| format!("wss://{r}"))
            .or_else(|| at.strip_prefix("http://").map(|r| format!("ws://{r}")))
            .unwrap_or(at)
    }

    fn change_worker_names(&self, body: serde_json::Value) -> Result<WorkerNames, ClientError> {
        let v = self.post_saying("api/worker-names", body)?;
        serde_json::from_value(v).map_err(|e| ClientError::Unreadable {
            reason: e.to_string(),
        })
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.base)
    }

    pub(crate) fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        self.get_by(&self.agent, path)
    }

    /// As `get`, but a refusal in words (search's "still indexing") comes
    /// back as `Said`, for the person to read.
    fn get_said<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let mut answer = self
            .agent
            .get(self.url(path))
            .header("cache-control", "no-store")
            .config()
            .http_status_as_error(false)
            .build()
            .call()?;
        let status = answer.status();
        if status.is_success() {
            return answer.body_mut().read_json().map_err(unreadable);
        }
        let page = answer
            .headers()
            .get("content-type")
            .is_some_and(|k| k.as_bytes().starts_with(b"text/html"));
        let said = answer.body_mut().read_to_string().unwrap_or_default();
        if said.trim().is_empty() || page {
            return Err(ureq::Error::StatusCode(status.as_u16()).into());
        }
        Err(ClientError::Said { reason: said })
    }

    fn get_by<T: DeserializeOwned>(&self, by: &ureq::Agent, path: &str) -> Result<T, ClientError> {
        by.get(self.url(path))
            .header("cache-control", "no-store")
            .call()?
            .body_mut()
            .read_json()
            .map_err(unreadable)
    }

    fn post(&self, path: &str, body: Value) -> Result<(), ClientError> {
        self.agent.post(self.url(path)).send_json(body)?;
        Ok(())
    }

    /// A POST whose refusal the host words for the person; answers what
    /// the host said back, if anything.
    fn post_saying(&self, path: &str, body: Value) -> Result<Value, ClientError> {
        // Some answers are a bare OK with no JSON, which callers ignore.
        Ok(serde_json::from_str(&self.post_said(path, body)?).unwrap_or(Value::Null))
    }

    /// A POST whose refusal the host words for the person; answers the
    /// host's words back as they came.
    fn post_said(&self, path: &str, body: Value) -> Result<String, ClientError> {
        let mut answer = self
            .slow
            .post(self.url(path))
            .config()
            .http_status_as_error(false)
            .build()
            .send_json(body)?;
        let status = answer.status();
        // A proxy's page while the daemon restarts: the host said nothing.
        let page = answer
            .headers()
            .get("content-type")
            .is_some_and(|k| k.as_bytes().starts_with(b"text/html"));
        let said = answer.body_mut().read_to_string().unwrap_or_default();
        if status.is_success() {
            // A page answered in the host's stead (a proxy, or an address
            // that is no longer this host's) is not an answer: read as null,
            // it surfaced as "invalid type: null, expected struct Tested".
            if page {
                return Err(ClientError::Said {
                    reason: "The address answered with a web page, not the host. Check the host's address.".into(),
                });
            }
            return Ok(said);
        }
        if said.trim().is_empty() || page {
            return Err(ureq::Error::StatusCode(status.as_u16()).into());
        }
        Err(ClientError::Said { reason: said })
    }

    fn post_empty(&self, path: &str) -> Result<(), ClientError> {
        self.agent.post(self.url(path)).send_empty()?;
        Ok(())
    }

    /// The conversation and the turn under way, as the host has them now.
    pub(crate) fn read(&self) -> Result<(Vec<Said>, TaskNow, Vec<Card>), ClientError> {
        // The record first: what leaves it while the history is read is then
        // in the history, so no read falls between the two.
        let task: TaskNow = self.get("api/task")?;
        let said: Vec<Said> = self.get("api/history")?;
        let pending: Vec<Value> = self.get("api/pending").unwrap_or_default();
        Ok((said, task, pending.iter().filter_map(Card::of).collect()))
    }

    /// An AG-UI run on the main thread: the person's message, or answers
    /// to what the turn asked (`resume`).
    fn run_with(
        &self,
        messages: Value,
        resume: Option<Value>,
        props: Value,
    ) -> Result<(), ClientError> {
        let mut body = json!({
            "threadId": "main",
            "runId": uuid(),
            "state": {},
            "tools": [],
            "context": [],
            "forwardedProps": props,
            "messages": messages,
        });
        if let Some(r) = resume {
            body["resume"] = r;
        }
        self.post("api/run", body)
    }
}

/// The parts of a multipart form, by hand: ureq sends bytes as given.
fn form(parts: &[(&str, &str, &str, &[u8])]) -> (String, Vec<u8>) {
    let boundary = format!("libri-{}", uuid());
    let mut body = Vec::new();
    for (field, file, mime, data) in parts {
        let file = file.replace(['"', '\r', '\n'], "_");
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{file}\"\r\nContent-Type: {mime}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn path(name: &str) -> String {
    name.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[uniffi::export]
impl Host {
    #[uniffi::constructor]
    pub fn new(address: String) -> Result<std::sync::Arc<Self>, ClientError> {
        Ok(std::sync::Arc::new(Host {
            base: crate::address(address)?,
            agent: agent(Duration::from_secs(30)),
            slow: agent(Duration::from_secs(300)),
        }))
    }

    /// The address the host is reached at, made whole.
    pub fn address(&self) -> String {
        self.base.clone()
    }

    /// Who the host is, timed.
    pub fn info(&self) -> Result<HostInfo, ClientError> {
        let start = std::time::Instant::now();
        let mut info: HostInfo = self.get("api/host")?;
        if info.assistant.is_empty() {
            info.assistant = info.name.clone();
        }
        info.millis = start.elapsed().as_millis() as u64;
        Ok(info)
    }

    /// Where a file the host keeps (a picture it sent, a photo the person
    /// sent) is fetched from.
    pub fn file_url(&self, name: String) -> String {
        self.url(&format!("api/file/{}", path(&name)))
    }

    /// Sends a file ahead of its message; answers the id the message names
    /// it by.
    pub fn upload(&self, file: Attachment) -> Result<String, ClientError> {
        let mut parts = vec![(
            "file",
            file.name.as_str(),
            file.mime.as_str(),
            &file.data[..],
        )];
        if let Some(view) = &file.view {
            parts.push(("view", "view.jpg", "image/jpeg", &view[..]));
        }
        let (kind, body) = form(&parts);
        #[derive(Deserialize)]
        struct Id {
            id: String,
        }
        let id: Id = self
            .slow
            .post(self.url("api/upload"))
            .header("content-type", &kind)
            .send(&body[..])?
            .body_mut()
            .read_json()
            .map_err(unreadable)?;
        Ok(id.id)
    }

    /// The person's message, with files already uploaded, as an AG-UI run.
    /// `redo` is `edit` (the message takes the place of their last one) or
    /// `again` (the last answer is drawn again).
    pub fn run(
        &self,
        id: String,
        text: String,
        files: Vec<Attachment>,
        uploaded: Vec<String>,
        redo: Option<String>,
        quote: Option<crate::Quote>,
    ) -> Result<(), ClientError> {
        let content = if uploaded.is_empty() {
            json!(text)
        } else {
            let mut parts = vec![json!({ "type": "text", "text": text })];
            for (f, id) in files.iter().zip(uploaded) {
                parts.push(
                    json!({ "type": "binary", "id": id, "mimeType": f.mime, "filename": f.name }),
                );
            }
            Value::Array(parts)
        };
        let mut props = redo.map_or(json!({}), |r| json!({ "redo": r }));
        if let Some(q) = quote {
            props["quote"] = json!(q);
        }
        self.run_with(
            json!([{ "id": id, "role": "user", "content": content }]),
            None,
            props,
        )
    }

    /// Stops the turn under way; it answers "(Stopped.)" over the stream.
    pub fn stop_turn(&self) -> Result<(), ClientError> {
        self.post_empty("api/stop")
    }

    /// `/clear`: the main chat's next turn starts a new session. Refused
    /// (409) while a turn runs.
    pub fn clear_session(&self) -> Result<(), ClientError> {
        self.post_empty("api/session/clear")
    }

    /// The main chat's harness and its own slash commands, as it last
    /// reported them (site.rs `commands_now`); None from a host from before
    /// them.
    pub fn harness_commands(&self) -> Result<Option<HarnessCommands>, ClientError> {
        match self.get::<HarnessCommands>("api/commands") {
            Err(ClientError::Refused { status: 404, .. }) => Ok(None),
            got => got.map(Some),
        }
    }

    /// Answers what the turn asked: yes or no to a tool, or the questions'
    /// answers by question.
    pub fn reply(
        &self,
        ask: String,
        yes: bool,
        answers: HashMap<String, String>,
        always: bool,
    ) -> Result<(), ClientError> {
        let mut resume =
            json!({ "interruptId": ask, "status": if yes { "resolved" } else { "cancelled" } });
        if !answers.is_empty() {
            resume["payload"] = json!({ "answers": answers });
        }
        // "Always allow" on a connection's ask sets its tool to Allow.
        if always {
            resume["payload"]["always"] = true.into();
        }
        self.run_with(json!([]), Some(json!([resume])), json!({}))
    }

    /// The person's verdict on an act waiting for it.
    pub fn decide(&self, id: u64, yes: bool) -> Result<(), ClientError> {
        self.post("api/decide", json!({ "id": id, "yes": yes }))
    }

    /// The skills each harness finds; a host from before skills has none.
    pub fn skill_menus(&self) -> Result<Vec<SkillMenu>, ClientError> {
        #[derive(Deserialize)]
        struct Menu {
            mention: String,
            skills: Vec<SkillOffer>,
        }
        let all: std::collections::BTreeMap<String, Menu> = self.get("api/skills/menu")?;
        Ok(all
            .into_iter()
            .map(|(harness, m)| SkillMenu {
                harness,
                mention: m.mention,
                skills: m.skills,
            })
            .collect())
    }

    pub fn models(&self) -> Result<Models, ClientError> {
        let v: Value = self.get_by(&self.slow, "api/models")?;
        let s = |k: &str| v[k].as_str().unwrap_or("").to_owned();
        let text = |x: &Value, or: &Value| x.as_str().or(or.as_str()).unwrap_or("").to_owned();
        Ok(Models {
            temp_harness: text(&v["temp"]["pick"]["harness"], &v["pick"]["harness"]),
            temp_model: text(&v["temp"]["pick"]["model"], &v["pick"]["model"]),
            temp_effort: text(&v["temp"]["effort"], &v["effort"]),
            temp_access: text(&v["temp"]["access"], &v["access"]),
            temp_sandbox: text(&v["temp"]["sandbox"], &v["sandbox"]),
            harness: v["pick"]["harness"].as_str().unwrap_or("").to_owned(),
            model: v["pick"]["model"].as_str().unwrap_or("").to_owned(),
            effort: s("effort"),
            access: s("access"),
            sandbox: s("sandbox"),
            modes: serde_json::from_value(v["modes"].clone()).unwrap_or_default(),
            offers: serde_json::from_value(v["offers"].clone()).map_err(|e| {
                ClientError::Unreadable {
                    reason: e.to_string(),
                }
            })?,
            others: v["others"].as_str().map(str::to_owned),
            others_offered: v["othersOffered"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            signed_out: v["signedOut"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            harnesses: serde_json::from_value(v["harnesses"].clone()).unwrap_or_default(),
            default_harness: v["defaultHarness"].as_str().unwrap_or("").to_owned(),
        })
    }

    /// Switches a harness on or off in Settings.
    pub fn switch_harness(&self, harness: String, on: bool) -> Result<(), ClientError> {
        self.post("api/harness", json!({ "harness": harness, "on": on }))
    }

    /// Makes a harness the one a chat runs on with no model picked.
    pub fn default_harness(&self, harness: String) -> Result<(), ClientError> {
        self.post(
            "api/harness",
            json!({ "harness": harness, "default": true }),
        )
    }

    /// Sets a harness's default model; empty goes back to its own.
    pub fn default_model(&self, harness: String, model: String) -> Result<(), ClientError> {
        self.post("api/harness", json!({ "harness": harness, "model": model }))
    }

    /// Switches the harness the other makers' models run in.
    pub fn pick_others(&self, others: String) -> Result<(), ClientError> {
        self.post("api/others", json!({ "others": others }))
    }

    pub fn makers(&self) -> Result<Makers, ClientError> {
        let v: Value = self.get_by(&self.slow, "api/capabilities")?;
        let read = |x: &Value| serde_json::from_value::<Maker>(x.clone()).ok();
        Ok(Makers {
            pictures: v["image"]["choices"]
                .as_array()
                .map(|a| a.iter().filter_map(read).collect())
                .unwrap_or_default(),
            picked: read(&v["image"]["picked"]),
            fixed: serde_json::from_value(v["fixed"].clone()).unwrap_or_default(),
        })
    }

    /// Picks the model that makes pictures.
    pub fn pick_pictures(&self, provider: String, model: String) -> Result<(), ClientError> {
        self.post(
            "api/capabilities",
            json!({ "capability": "image", "provider": provider, "model": model }),
        )
    }

    pub fn connections(&self) -> Result<Connections, ClientError> {
        self.get("api/connections")
    }

    /// The servers this host's deployment adds, each with its tool count.
    pub fn host_servers(&self) -> Result<Vec<HostServer>, ClientError> {
        self.get("api/connections/host")
    }

    /// The paired assistants and the requests either way.
    pub fn peers(&self) -> Result<Vec<Peer>, ClientError> {
        self.get("api/peers")
    }

    /// Asks the assistant at `url` to pair; the host words a refusal.
    pub fn ask_peer(&self, url: String) -> Result<(), ClientError> {
        self.post_said("api/peers", json!({ "url": url }))
            .map(|_| ())
    }

    /// Approves the request from the assistant at `url`.
    pub fn approve_peer(&self, url: String) -> Result<(), ClientError> {
        self.post_said("api/peers/approve", json!({ "url": url }))
            .map(|_| ())
    }

    /// Revokes the pair at `url`, or declines or withdraws its request.
    pub fn revoke_peer(&self, url: String) -> Result<(), ClientError> {
        self.post_said("api/peers/revoke", json!({ "url": url }))
            .map(|_| ())
    }

    /// The phone's workspaces. With `since`, waits (up to 25 s) for a
    /// version past it, which is how the app follows a change made by the
    /// agent or on another screen.
    pub fn phone_layout(&self, since: Option<u64>) -> Result<PhoneLayout, ClientError> {
        let at = since.map_or(String::new(), |v| format!("?since={v}"));
        PhoneLayout::read(self.get_by(&self.slow, &format!("api/phone-layout{at}"))?)
    }

    /// Keeps the phone's workspaces; answers them as kept. The host's
    /// refusal of one it can't draw comes back as `Said`.
    pub fn keep_phone_layout(
        &self,
        spaces: Vec<PhoneSpace>,
        on: String,
        closed: Vec<PhoneClosed>,
    ) -> Result<PhoneLayout, ClientError> {
        PhoneLayout::read(self.post_saying(
            "api/phone-layout",
            json!({ "spaces": spaces, "on": on, "closed": closed }),
        )?)
    }

    /// Back to the phone's workspaces before the latest change, once.
    pub fn undo_phone_layout(&self) -> Result<PhoneLayout, ClientError> {
        PhoneLayout::read(self.post_saying("api/phone-layout/undo", json!({}))?)
    }

    pub fn builtins(&self) -> Result<Vec<Builtin>, ClientError> {
        self.get("api/builtins")
    }

    pub fn other_apps(&self) -> Result<OtherApps, ClientError> {
        self.get("api/connectors")
    }

    /// A connector for an app that takes a header, and its key.
    pub fn connector_add(
        &self,
        name: String,
        modules: Vec<String>,
    ) -> Result<ConnectorMade, ClientError> {
        let got = self.post_saying(
            "api/connectors",
            json!({ "name": name, "modules": modules }),
        )?;
        serde_json::from_value(got).map_err(|e| ClientError::Unreadable {
            reason: e.to_string(),
        })
    }

    /// What a connector may use, from its next call on.
    pub fn connector_set(&self, id: String, modules: Vec<String>) -> Result<(), ClientError> {
        self.post_saying(
            &format!("api/connectors/{id}"),
            json!({ "modules": modules }),
        )
        .map(drop)
    }

    /// Takes a connector back: its app is signed out at once.
    pub fn connector_revoke(&self, id: String) -> Result<(), ClientError> {
        self.post_saying(&format!("api/connectors/{id}/revoke"), Value::Null)
            .map(drop)
    }

    pub fn providers(&self) -> Result<Vec<Provider>, ClientError> {
        self.get("api/connections/providers")
    }

    /// Adds a provider's connection unless there is one: its name, to sign in to.
    pub fn provider_connect(&self, id: String) -> Result<String, ClientError> {
        let v = self.post_saying(
            &format!("api/connections/providers/{}", path(&id)),
            json!({}),
        )?;
        Ok(v["name"].as_str().unwrap_or("").to_owned())
    }

    /// Sets one tool of a connection to "allow", "ask" or "off".
    pub fn tool_rule(&self, server: String, tool: String, rule: String) -> Result<(), ClientError> {
        self.post_saying(
            &format!("api/connections/servers/{}/tools", path(&server)),
            json!({ "tool": tool, "rule": rule }),
        )
        .map(drop)
    }

    /// What a connection brings in, each shown or hidden.
    pub fn items(&self, server: String) -> Result<Vec<ItemKind>, ClientError> {
        self.get(&format!("api/connections/servers/{}/items", path(&server)))
    }

    pub fn item_show(
        &self,
        server: String,
        kind: String,
        id: String,
        shown: bool,
    ) -> Result<(), ClientError> {
        self.post_saying(
            &format!("api/connections/servers/{}/items", path(&server)),
            json!({ "kind": kind, "id": id, "shown": shown }),
        )
        .map(drop)
    }

    /// Switches a built-in or picks its provider; each left None stays.
    pub fn builtin_set(
        &self,
        id: String,
        on: Option<bool>,
        provider: Option<String>,
    ) -> Result<(), ClientError> {
        self.post_saying(
            &format!("api/builtins/{id}"),
            json!({ "on": on, "provider": provider }),
        )
        .map(drop)
    }

    /// Adds a connection or changes the one of that name. `secrets`, one
    /// per line (`Name: value` headers for a URL, `NAME=value` for a
    /// command), replaces the kept ones; None keeps them.
    pub fn server_put(
        &self,
        name: String,
        address: String,
        secrets: Option<String>,
        withheld: Vec<String>,
        on: bool,
    ) -> Result<(), ClientError> {
        self.post_saying(
            "api/connections/servers",
            json!({ "name": name, "address": address, "secrets": secrets,
                "withheld": withheld, "on": on }),
        )
        .map(drop)
    }

    /// Connects as a harness would and lists the tools.
    pub fn server_test(&self, name: String) -> Result<Tested, ClientError> {
        let v = self.post_saying(&format!("api/connections/servers/{name}/test"), json!({}))?;
        serde_json::from_value(v).map_err(|e| ClientError::Said {
            reason: e.to_string(),
        })
    }

    /// Starts signing in from the Android app: answers the page to sign in
    /// on, which returns to the app by its own scheme, the address then
    /// handed to `server_finish`.
    pub fn server_sign_in(&self, name: String) -> Result<String, ClientError> {
        let v = self.post_saying(
            &format!("api/connections/servers/{name}/sign-in"),
            json!({ "app": true }),
        )?;
        Ok(v["url"].as_str().unwrap_or("").to_owned())
    }

    /// Finishes signing in, by the address of the page the server sent the
    /// browser back to, which loads only on the host itself.
    pub fn server_finish(&self, address: String) -> Result<(), ClientError> {
        self.post_saying("api/connections/finish", json!({ "address": address }))
            .map(drop)
    }

    pub fn server_remove(&self, name: String) -> Result<(), ClientError> {
        self.post_saying(&format!("api/connections/servers/{name}/remove"), json!({}))
            .map(drop)
    }

    /// Starts the Grok sign-in: `connections` then shows its page and code
    /// until the person has approved it on any device.
    pub fn xai_start(&self) -> Result<(), ClientError> {
        self.post_saying("api/connections/xai", json!({})).map(drop)
    }

    pub fn xai_key(&self, key: String) -> Result<(), ClientError> {
        self.post_saying("api/connections/xai/key", json!({ "key": key }))
            .map(drop)
    }

    pub fn xai_drop(&self) -> Result<(), ClientError> {
        self.post_saying("api/connections/xai/drop", json!({}))
            .map(drop)
    }

    /// Saves the voice on the host, for every device.
    pub fn pick_voice(&self, voice: String) -> Result<(), ClientError> {
        self.post("api/voice", json!({ "voice": voice }))
    }

    /// A pass for a call; the call itself goes straight to the voice service.
    pub fn call_pass(&self) -> Result<CallPass, ClientError> {
        self.slow
            .post(self.url("api/call"))
            .send_empty()?
            .body_mut()
            .read_json()
            .map_err(unreadable)
    }

    /// A finished call into the memory, so the chat holds it.
    pub fn call_end(&self, lines: Vec<CallLine>, minutes: u64) -> Result<(), ClientError> {
        let lines: Vec<Value> = lines
            .iter()
            .map(|l| json!({ "kind": l.kind, "text": l.text }))
            .collect();
        self.post(
            "api/call/end",
            json!({ "lines": lines, "minutes": minutes }),
        )
    }

    /// Words from a recording, sent whole as the body with its kind
    /// (`audio/mp4`, `audio/ogg`); neither the host nor this keeps it.
    pub fn transcribe(&self, audio: Vec<u8>, kind: String) -> Result<String, ClientError> {
        let v: Value = self
            .slow
            .post(self.url("api/transcribe"))
            .header("content-type", &kind)
            .send(&audio[..])?
            .body_mut()
            .read_json()
            .map_err(unreadable)?;
        Ok(v["text"].as_str().unwrap_or("").to_owned())
    }

    /// Where the host writes speech down; none on an older host.
    pub fn speech(&self) -> Result<Option<Speech>, ClientError> {
        match self.get("api/stt") {
            Ok(s) => Ok(Some(s)),
            Err(ClientError::Refused { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Picks where speech is written down (`local`, `xai`, `openai`) and,
    /// for `local`, the model, which the host fetches if it lacks it.
    pub fn pick_speech(&self, provider: String, model: String) -> Result<(), ClientError> {
        self.post_saying("api/stt", json!({ "provider": provider, "model": model }))
            .map(|_| ())
    }

    /// How the memory is working; none on an older host without the route.
    pub fn memory(&self) -> Result<Option<MemoryHealth>, ClientError> {
        match self.get("api/memory") {
            Ok(m) => Ok(Some(m)),
            Err(ClientError::Refused { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Who writes workers' titles and summaries; none on a host without
    /// the choice.
    pub fn synopsis_model(&self) -> Result<Option<SynopsisModel>, ClientError> {
        match self.get_by(&self.slow, "api/synopsis") {
            Ok(v) => Ok(Some(v)),
            Err(ClientError::Refused { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// What new workers start on; none on a host without the choice.
    pub fn worker_model(&self) -> Result<Option<WorkerModel>, ClientError> {
        match self.get_by(&self.slow, "api/worker-model") {
            Ok(v) => Ok(Some(v)),
            Err(ClientError::Refused { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Picks any model on offer; none goes back to the default.
    pub fn pick_synopsis_model(
        &self,
        pick: Option<ModelRef>,
    ) -> Result<SynopsisModel, ClientError> {
        let body = pick.map_or(
            json!({}),
            |p| json!({ "harness": p.harness, "model": p.model }),
        );
        let v = self.post_saying("api/synopsis", body)?;
        serde_json::from_value(v).map_err(|e| ClientError::Unreadable {
            reason: e.to_string(),
        })
    }

    /// Picks any model on offer; none goes back to following the main chat.
    pub fn pick_worker_model(&self, pick: Option<ModelRef>) -> Result<WorkerModel, ClientError> {
        let body = pick.map_or(
            json!({}),
            |p| json!({ "harness": p.harness, "model": p.model }),
        );
        let v = self.post_saying("api/worker-model", body)?;
        serde_json::from_value(v).map_err(|e| ClientError::Unreadable {
            reason: e.to_string(),
        })
    }

    /// The person's standing rules; none on a host without them.
    pub fn guide(&self) -> Result<Option<Guide>, ClientError> {
        match self.get_by(&self.slow, "api/guide") {
            Ok(v) => Ok(Some(v)),
            Err(ClientError::Refused { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Reaches the next turn. Empty leaves no rules.
    pub fn set_guide(&self, text: String) -> Result<Guide, ClientError> {
        let v = self.post_saying("api/guide", json!({ "text": text }))?;
        serde_json::from_value(v).map_err(|e| ClientError::Unreadable {
            reason: e.to_string(),
        })
    }

    /// What is true of this host alone (HOST.md); none on a host from
    /// before it.
    pub fn host_guide(&self) -> Result<Option<Guide>, ClientError> {
        match self.get_by(&self.slow, "api/guide/host") {
            Ok(v) => Ok(Some(v)),
            Err(ClientError::Refused { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Reaches the next turn. Empty leaves none.
    pub fn set_host_guide(&self, text: String) -> Result<Guide, ClientError> {
        let v = self.post_saying("api/guide/host", json!({ "text": text }))?;
        serde_json::from_value(v).map_err(|e| ClientError::Unreadable {
            reason: e.to_string(),
        })
    }

    /// The names new workers are given; none on a host without the list.
    pub fn worker_names(&self) -> Result<Option<WorkerNames>, ClientError> {
        match self.get_by(&self.slow, "api/worker-names") {
            Ok(v) => Ok(Some(v)),
            Err(ClientError::Refused { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// `change` is add, remove, exclude or restore.
    pub fn change_worker_name(
        &self,
        change: String,
        name: String,
    ) -> Result<WorkerNames, ClientError> {
        self.change_worker_names(json!({ "change": change, "name": name }))
    }

    /// Whether the person's own list stands in for the others; neither is lost.
    pub fn use_own_worker_names(&self, on: bool) -> Result<WorkerNames, ClientError> {
        self.change_worker_names(json!({ "change": "use_own", "on": on }))
    }

    /// The whole of the person's own list, by commas or a name to a line.
    pub fn set_own_worker_names(&self, text: String) -> Result<WorkerNames, ClientError> {
        self.change_worker_names(json!({ "change": "set_own", "text": text }))
    }

    /// The voices the assistant may read aloud in.
    pub fn voices(&self) -> Result<Vec<Voice>, ClientError> {
        self.get_by(&self.slow, "api/voices")
    }

    pub fn pick_model(
        &self,
        harness: String,
        model: String,
        temp: bool,
    ) -> Result<(), ClientError> {
        self.post(
            "api/model",
            json!({ "harness": harness, "model": model, "temp": temp }),
        )
    }

    /// Empty goes back to the harness's own default.
    pub fn pick_effort(&self, effort: String, temp: bool) -> Result<(), ClientError> {
        self.post("api/effort", json!({ "effort": effort, "temp": temp }))
    }

    pub fn pick_access(&self, access: String, temp: bool) -> Result<(), ClientError> {
        self.post("api/access", json!({ "access": access, "temp": temp }))
    }

    pub fn pick_sandbox(&self, sandbox: String, temp: bool) -> Result<(), ClientError> {
        self.post("api/access", json!({ "sandbox": sandbox, "temp": temp }))
    }

    /// What `harness` starts on, in Settings; None leaves that half be.
    pub fn pick_access_default(
        &self,
        harness: String,
        access: Option<String>,
        sandbox: Option<String>,
    ) -> Result<(), ClientError> {
        self.post(
            "api/access/default",
            json!({ "harness": harness, "access": access, "sandbox": sandbox }),
        )
    }

    pub fn usage(&self) -> Result<Usage, ClientError> {
        self.get_by(&self.slow, "api/usage/report")
    }

    /// What model calls cost over `span` ("day", "week" or "month").
    pub fn spend(&self, span: String) -> Result<Spend, ClientError> {
        self.get_by(&self.slow, &format!("api/usage/spend?span={span}"))
    }

    /// Shows the usage rings in every composer, or hides them.
    pub fn show_usage(&self, shown: bool) -> Result<(), ClientError> {
        self.post("api/usage", json!({ "shown": shown }))
    }

    /// The point every harness compacts a session at, in tokens.
    pub fn compact(&self) -> Result<CompactPoint, ClientError> {
        self.get::<CompactPoint>("api/compact")
    }

    /// Sets the compaction point; None goes back to the host's default.
    pub fn pick_compact(&self, tokens: Option<u64>) -> Result<(), ClientError> {
        self.post("api/compact", json!({ "tokens": tokens }))
    }

    pub fn work(&self) -> Result<WorkList, ClientError> {
        let mut list = self.get::<WorkList>("api/work")?;
        for t in list.threads.iter_mut().filter(|t| t.id.is_empty()) {
            t.id = t.name.clone();
        }
        Ok(list)
    }

    /// The named memories; none from a host from before them.
    pub fn memories(&self) -> Result<Vec<NamedMemory>, ClientError> {
        #[derive(Deserialize)]
        struct List {
            memories: Vec<NamedMemory>,
        }
        match self.get::<List>("api/memories") {
            Err(ClientError::Refused { status: 404, .. }) => Ok(Vec::new()),
            got => got.map(|l| l.memories),
        }
    }

    /// Makes a named memory, with its instructions if any; answers its id.
    /// The host's refusal (a name taken) comes back as `Said`.
    pub fn add_memory(&self, name: String, instructions: String) -> Result<String, ClientError> {
        let made = self.post_saying("api/memories", json!({ "name": name }))?;
        let id = made["id"].as_str().unwrap_or_default().to_owned();
        if !instructions.trim().is_empty() {
            self.post_saying(
                &format!("api/memories/{}", path(&id)),
                json!({ "instructions": instructions.trim() }),
            )?;
        }
        Ok(id)
    }

    /// The Library. A host from before it has none, which reads as empty.
    pub fn library(&self) -> Result<LibraryShelf, ClientError> {
        match self.get::<LibraryShelf>("api/library") {
            Err(ClientError::Refused { status: 404, .. }) => Ok(LibraryShelf::default()),
            got => got,
        }
    }

    /// The tools that draw a card (MCP Apps), by the names calls carry,
    /// and whether each card opens by itself (apps.rs `Card::opens`); none
    /// from a host before cards.
    pub fn card_tools(&self) -> Result<Vec<CardTool>, ClientError> {
        #[derive(Deserialize)]
        struct Cards {
            cards: std::collections::BTreeMap<String, Value>,
        }
        match self.get::<Cards>("api/apps") {
            Err(ClientError::Refused { status: 404, .. }) => Ok(Vec::new()),
            got => got.map(|c| {
                c.cards
                    .into_iter()
                    .map(|(name, card)| CardTool {
                        opens: card["opens"].as_bool().unwrap_or(false),
                        name,
                    })
                    .collect()
            }),
        }
    }

    /// The page of one tool's card alone, full screen (the site's
    /// `#card=`), for a card that shows something live.
    pub fn card_url(&self, tool: String) -> String {
        self.url(&format!("#card={}", path(&tool)))
    }

    /// One call's card alone, for the chat to draw in a web view (the
    /// site's `#call-card=`, card.tsx CallCardAlone): the call's name as the
    /// harness gave it, its arguments, and what came back, whole
    /// (`structured`) or as its words. `fresh` asks a read-only tool again
    /// for what it says now, for a call read back from the record; `full`
    /// fills the view. A result too long for an address goes as its words.
    #[allow(clippy::too_many_arguments)]
    pub fn call_card_url(
        &self,
        name: String,
        arguments: Option<String>,
        structured: Option<String>,
        result: Option<String>,
        error: bool,
        fresh: bool,
        full: bool,
    ) -> String {
        /// What an address carries of a result: past it, the words alone.
        const LONGEST: usize = 512 * 1024;
        let words = || {
            json!({"content": [{"type": "text", "text": result.clone().unwrap_or_default()}]})
                .to_string()
        };
        let outcome = structured
            .filter(|s| s.len() <= LONGEST)
            .unwrap_or_else(words);
        let mut at = format!(
            "#call-card={}&input={}&outcome={}",
            path(&name),
            path(arguments.as_deref().unwrap_or("{}")),
            path(&outcome)
        );
        for (on, flag) in [(error, "&error=1"), (fresh, "&fresh=1"), (full, "&full=1")] {
            if on {
                at.push_str(flag);
            }
        }
        self.url(&at)
    }

    /// Where a published page is opened, on this host: the site's page of
    /// it alone (`#page-alone=`), which runs it as an MCP App with its data.
    pub fn page_url(&self, name: String) -> String {
        self.url(&format!("#page-alone={}", path(&name)))
    }

    /// A built-in's view alone (the site's `#builtin-alone=`), for the app
    /// to show in a web view in the chat's place.
    pub fn builtin_url(&self, id: String) -> String {
        self.url(&format!("#builtin-alone={}", path(&id)))
    }

    /// The Whiteboard tile alone (the site's `board.html#board-tile=`), on the board
    /// `id` or, with none, the host's first; the app shows it in a web view
    /// as one of its places, and keeps the board it reports.
    pub fn board_url(&self, id: Option<String>) -> String {
        self.url(&format!(
            "board.html#board-tile={}",
            id.as_deref().map(path).unwrap_or_default()
        ))
    }

    /// The Browser tile alone (the site's `#browser-tile`), for the app's
    /// place of it: Libri's own browser, shared with the agents.
    pub fn browser_tile_url(&self) -> String {
        self.url("#browser-tile")
    }

    /// The System tile's page: the host's CPU, memory, disks and workers.
    pub fn system_tile_url(&self) -> String {
        self.url("#system-tile")
    }

    /// A published page's still at a version, as the Library's card shows
    /// it (site.rs `snapshot`): the same version is the same image, so an
    /// image cache keeps it.
    pub fn snapshot_url(&self, name: String, changed: u64) -> String {
        self.url(&format!("api/library/{}/snapshot?v={changed}", path(&name)))
    }

    /// The watches and reminders, soonest first; none from a host before them.
    pub fn watches(&self) -> Result<Vec<WatchItem>, ClientError> {
        #[derive(Deserialize)]
        struct List {
            watches: Vec<WatchItem>,
        }
        match self.get::<List>("api/watches") {
            Err(ClientError::Refused { status: 404, .. }) => Ok(Vec::new()),
            got => got.map(|l| l.watches),
        }
    }

    /// Stops and forgets a watch, with no model call.
    pub fn watch_delete(&self, id: u64) -> Result<(), ClientError> {
        self.post_empty(&format!("api/watches/{id}/delete"))
    }

    /// Moves a timed watch's next firing to `until`, in seconds since the epoch.
    pub fn watch_snooze(&self, id: u64, until: u64) -> Result<(), ClientError> {
        self.post(
            &format!("api/watches/{id}/snooze"),
            json!({ "until": until }),
        )
    }

    /// Starts a worker on the person's own words and files (upload ids), not
    /// through the main chat, on `choice` as the Tasks sheet's pickers left
    /// it, else the Workers default; answers its name. Its report comes to
    /// its card.
    pub fn work_start(
        &self,
        brief: String,
        files: Vec<String>,
        choice: Option<WorkChoice>,
    ) -> Result<String, ClientError> {
        let mut body = json!({ "brief": brief, "files": files });
        if let Some(c) = choice {
            body["pick"] = json!({ "harness": c.harness, "model": c.model });
            body["effort"] = json!(c.effort);
            // Empty is the harness's default: the server takes no mode then.
            if !c.access.is_empty() {
                body["access"] = json!(c.access);
            }
            if !c.sandbox.is_empty() {
                body["sandbox"] = json!(c.sandbox);
            }
        }
        let v = self.post_saying("api/work", body)?;
        Ok(v["name"].as_str().unwrap_or_default().to_owned())
    }

    /// What a worker started from Tasks would run on, for the sheet's pickers.
    pub fn work_default_choice(&self) -> Result<WorkChoice, ClientError> {
        let v: Value = self.get_by(&self.slow, "api/work-choice")?;
        Ok(WorkChoice::of(&v))
    }

    /// A Work thread's whole log.
    pub fn work_log(&self, name: String) -> Result<Vec<Step>, ClientError> {
        let v: Value = self.get(&format!("api/work/{}", path(&name)))?;
        Ok(steps(v["text"].as_str().unwrap_or("")))
    }

    /// A seat subagent's whole chat, by the id its Agent call named.
    pub fn agent_chat(&self, id: String) -> Result<Vec<Step>, ClientError> {
        let v: Value = self.get(&format!("api/agent/{}/chat", path(&id)))?;
        Ok(steps(v["text"].as_str().unwrap_or("")))
    }

    /// `read`, `clear` (read, and what it asked of the person set aside),
    /// `archive` or `restore` a thread.
    pub fn work_act(&self, name: String, act: String) -> Result<(), ClientError> {
        self.post_empty(&format!("api/work/{}/{}", path(&name), path(&act)))
    }

    /// Merges a thread into the main chat's memory, as though done there,
    /// and archives it; the host's word on what landed.
    pub fn work_merge(&self, name: String) -> Result<String, ClientError> {
        self.post_said(&format!("api/work/{}/merge", path(&name)), Value::Null)
    }

    /// Puts a thread down for a nap until `until`, in seconds since the
    /// epoch: out of Waiting until it wakes.
    pub fn work_nap(&self, name: String, until: u64) -> Result<(), ClientError> {
        self.post(
            &format!("api/work/{}/nap", path(&name)),
            json!({ "until": until }),
        )
    }

    pub fn agent_stop(&self, name: String) -> Result<(), ClientError> {
        self.post_empty(&format!("api/agent/{}/stop", path(&name)))
    }

    /// Words to a thread, with files already uploaded: read between its
    /// calls while it works, or taken up from its log if it is done.
    pub fn agent_tell(
        &self,
        name: String,
        text: String,
        files: Vec<String>,
    ) -> Result<(), ClientError> {
        self.post(
            &format!("api/agent/{}/tell", path(&name)),
            json!({ "text": text, "files": files }),
        )
    }

    pub fn work_choice(&self, name: String) -> Result<WorkChoice, ClientError> {
        let v: Value = self.get_by(&self.slow, &format!("api/work/{}/choice", path(&name)))?;
        Ok(WorkChoice::of(&v))
    }

    /// Changes what is given of a thread's choice, from its next run on;
    /// the rest stays. Answers the choice as it now stands.
    pub fn work_choose(
        &self,
        name: String,
        harness: Option<String>,
        model: Option<String>,
        effort: Option<String>,
        access: Option<String>,
        sandbox: Option<String>,
    ) -> Result<WorkChoice, ClientError> {
        let mut body = json!({});
        if let (Some(harness), Some(model)) = (harness, model) {
            body["pick"] = json!({ "harness": harness, "model": model });
        }
        if let Some(effort) = effort {
            body["effort"] = json!(effort);
        }
        if let Some(access) = access {
            body["access"] = json!(access);
        }
        if let Some(sandbox) = sandbox {
            body["sandbox"] = json!(sandbox);
        }
        let v = self.post_saying(&format!("api/work/{}/choice", path(&name)), body)?;
        Ok(WorkChoice::of(&v))
    }

    /// Everything said, the chat's and every worker's, matching `words` as a
    /// person searches: best first, or with `newest` newest first.
    pub fn find(&self, words: String, newest: bool) -> Result<Found, ClientError> {
        let order = if newest { "new" } else { "best" };
        // Kotlin sees a flat error's words only, so a bare refusal is put in
        // the words the web shows for it.
        self.get_said(&format!("api/find?q={}&order={order}", path(&words)))
            .map_err(|e| match e {
                ClientError::Refused { status, .. } => ClientError::Said {
                    reason: format!("Search failed ({status})."),
                },
                e => e,
            })
    }

    /// The lines around a hit, `before` ahead and `after` behind, oldest first.
    pub fn around(
        &self,
        src: String,
        at: u64,
        before: u64,
        after: u64,
    ) -> Result<Vec<FoundLine>, ClientError> {
        self.get(&format!(
            "api/around?src={}&at={at}&before={before}&after={after}",
            path(&src)
        ))
    }

    /// The workers search reaches, newest first.
    pub fn sources(&self) -> Result<Vec<FoundSource>, ClientError> {
        self.get("api/sources")
    }

    /// A Temp, by the id its tab holds; none, the host's current one.
    pub fn scratchpad(&self, id: Option<String>) -> Result<Scratchpad, ClientError> {
        let v: Value = self.get(&temp_at("api/scratchpad", &id))?;
        Ok(Scratchpad::of(&v))
    }

    /// A new Temp beside any open, for a tab opened beside one: its id.
    pub fn scratchpad_new(&self) -> Result<String, ClientError> {
        let v: Value = self.post_saying("api/scratchpad/new", json!({}))?;
        v["id"]
            .as_str()
            .map(str::to_owned)
            .ok_or(ClientError::Unreadable {
                reason: "no id".into(),
            })
    }

    /// Words to a Temp, which opens if none is (with no id), with files
    /// already uploaded: read between its calls while a turn runs.
    pub fn scratchpad_tell(
        &self,
        text: String,
        files: Vec<String>,
        id: Option<String>,
    ) -> Result<(), ClientError> {
        self.post(
            &temp_at("api/scratchpad", &id),
            json!({ "text": text, "files": files }),
        )
    }

    /// The kinds of tile and their menus' rows as the host declares them
    /// (/api/tiles), as JSON the app reads.
    pub fn tiles(&self) -> Result<String, ClientError> {
        let v: Value = self.get("api/tiles")?;
        Ok(v.to_string())
    }

    /// A named memory's own chat (/api/memories/{id}/chat), read as Temp's
    /// is: the same log, run and asks.
    pub fn memory_chat(&self, id: String) -> Result<Scratchpad, ClientError> {
        let v: Value = self.get(&format!("api/memories/{}/chat", path(&id)))?;
        Ok(Scratchpad::of(&v))
    }

    /// Words to a named memory's chat, with files already uploaded.
    pub fn memory_chat_tell(
        &self,
        id: String,
        text: String,
        files: Vec<String>,
    ) -> Result<(), ClientError> {
        self.post(
            &format!("api/memories/{}/chat", path(&id)),
            json!({ "text": text, "files": files }),
        )
    }

    pub fn memory_chat_stop(&self, id: String) -> Result<(), ClientError> {
        self.post_empty(&format!("api/memories/{}/stop", path(&id)))
    }

    pub fn scratchpad_stop(&self, id: Option<String>) -> Result<(), ClientError> {
        self.post_empty(&temp_at("api/scratchpad/stop", &id))
    }

    /// Removes a Temp and all of it: the one named, or the current.
    pub fn scratchpad_close(&self, id: Option<String>) -> Result<(), ClientError> {
        self.agent
            .delete(self.url(&temp_at("api/scratchpad", &id)))
            .call()?;
        Ok(())
    }

    /// Moves a Temp into the main chat as the talk it was; no turn starts.
    /// The stream's `history` event has every client read it again.
    pub fn scratchpad_promote(&self, id: Option<String>) -> Result<(), ClientError> {
        self.post_empty(&temp_at("api/scratchpad/promote", &id))
    }

    /// Whether the temp chat's next turn has the memory.
    pub fn scratchpad_memory(&self, on: bool) -> Result<(), ClientError> {
        self.post("api/scratchpad/memory", json!({ "on": on }))
    }

    pub fn me(&self) -> Result<Me, ClientError> {
        self.get("api/me")
    }

    /// How long the assistant remembers, in minutes; none if it keeps all.
    pub fn forget_window(&self) -> Result<Option<u64>, ClientError> {
        match self.get::<Value>("api/forget") {
            Ok(v) => Ok(v["minutes"].as_u64()),
            Err(ClientError::Refused { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn pick_forget(&self, minutes: u64) -> Result<(), ClientError> {
        self.post("api/forget", json!({ "minutes": minutes }))
    }

    pub fn signed_in(&self) -> Result<Vec<SignedIn>, ClientError> {
        let v: HashMap<String, bool> = self.get_by(&self.slow, "api/signin")?;
        let mut all: Vec<SignedIn> = v
            .into_iter()
            .map(|(harness, yes)| SignedIn { harness, yes })
            .collect();
        all.sort_by(|a, b| a.harness.cmp(&b.harness));
        Ok(all)
    }

    /// Starts a provider device authorization in omp; its credentials stay there.
    pub fn omp_sign_in(&self, provider: String) -> Result<String, ClientError> {
        self.post_saying(
            "api/signin",
            json!({"harness": "omp", "provider": provider}),
        )
        .map(|v| v.to_string())
    }

    pub fn sign_in_progress(&self) -> Result<String, ClientError> {
        self.get_by::<Value>(&self.agent, "api/signin/progress")
            .map(|v| v.to_string())
    }

    /// The words read aloud in the assistant's voice: where to play them.
    /// `voice` is one of `voices`; none reads in the assistant's own.
    pub fn speak(&self, text: String, voice: Option<String>) -> Result<String, ClientError> {
        let body = match voice {
            Some(v) => json!({ "text": text, "voice": v }),
            None => json!({ "text": text }),
        };
        let v: Value = self
            .slow
            .post(self.url("api/speak"))
            .send_json(body)?
            .body_mut()
            .read_json()
            .map_err(unreadable)?;
        Ok(self.url(&format!("api/speak/{}", v["key"].as_str().unwrap_or(""))))
    }

    /// The host's Web Push key (VAPID), which a UnifiedPush registration
    /// names; none when the host sends no notifications.
    pub fn push_key(&self) -> Result<Option<String>, ClientError> {
        match self.agent.get(self.url("api/push")).call() {
            Ok(mut r) => Ok(Some(
                r.body_mut()
                    .read_to_string()
                    .map_err(unreadable)?
                    .trim()
                    .to_owned(),
            )),
            Err(ureq::Error::StatusCode(404)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Subscribes this phone to the host's notifications, as a browser's
    /// Web Push subscription: the push server sees only ciphertext.
    pub fn push_add(
        &self,
        endpoint: String,
        p256dh: String,
        auth: String,
    ) -> Result<(), ClientError> {
        self.post(
            "api/push",
            json!({ "endpoint": endpoint, "keys": { "p256dh": p256dh, "auth": auth } }),
        )
    }

    pub fn push_drop(&self, endpoint: String) -> Result<(), ClientError> {
        self.post("api/push/drop", json!({ "endpoint": endpoint }))
    }

    /// The host's terminal socket: ws or wss as the host is reached.
    pub fn terminal_url(&self) -> String {
        self.socket_url("api/terminal")
    }

    /// The host's socket for a call with the chat (site.rs `call_socket`).
    pub fn call_socket_url(&self) -> String {
        self.socket_url("api/call/socket")
    }

    /// The voice in front hands a request to the chat and has its answer
    /// (site.rs `call_ask`): empty when the chat is still at it.
    pub fn call_ask(&self, text: String) -> Result<String, ClientError> {
        let v: Value = self
            .slow
            .post(self.url("api/call/ask"))
            .send_json(json!({ "text": text }))?
            .body_mut()
            .read_json()
            .map_err(unreadable)?;
        Ok(v["text"].as_str().unwrap_or_default().to_owned())
    }
}

/// A Temp's route, `?id=` naming one of its own.
fn temp_at(route: &str, id: &Option<String>) -> String {
    match id {
        Some(id) => format!("{route}?id={}", path(id)),
        None => route.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_reads_as_the_host_answers_it() {
        let f: Found = serde_json::from_value(json!({"hits": [
            {"src": "", "at": 82389, "kind": "user", "time": 1793714710, "text": "no way to \u{2}logout\u{3}"},
            {"src": "finn-2026-11-03-0905", "at": 1, "kind": "user", "time": 1793715755, "text": "x"},
        ], "loose": false}))
        .unwrap();
        assert_eq!(f.hits[0].text, "no way to \u{2}logout\u{3}");
        assert_eq!(f.hits[1].src, "finn-2026-11-03-0905");
        let l: Vec<FoundLine> = serde_json::from_value(json!([
            {"at": 1, "kind": "talk", "time": 1, "text": "a"},
            {"at": 2, "kind": "echo", "time": 1, "text": "b", "cut": true},
        ]))
        .unwrap();
        assert_eq!((l[0].cut, l[1].cut), (false, true));
        assert_eq!(path("tunnel \"attic\" ssh"), "tunnel%20%22attic%22%20ssh");
    }

    #[test]
    fn worker_names_read_as_the_host_lists_them() {
        let names: WorkerNames = serde_json::from_value(json!({
            "builtin": ["Kit", "Hana"], "added": ["Thaddeus"], "own": ["Alder"], "use_own": false,
            "excluded": ["Kit"], "allowed": 2,
        }))
        .unwrap();
        assert_eq!(names.excluded, ["Kit"]);
        assert_eq!(names.allowed, 2);
        assert!(names.left_out.is_empty());
    }

    #[test]
    fn a_guide_reads_as_the_host_holds_it() {
        let guide: Guide = serde_json::from_value(json!({"text": "Plain words."})).unwrap();
        assert_eq!(guide.text, "Plain words.");
    }

    #[test]
    fn a_worker_model_reads_with_a_pick_or_without() {
        let none: WorkerModel = serde_json::from_value(json!({"picked": null})).unwrap();
        assert_eq!(none.picked, None);
        let some: WorkerModel =
            serde_json::from_value(json!({"picked": {"harness": "claude", "model": "m"}})).unwrap();
        assert_eq!(
            some.picked,
            Some(ModelRef {
                harness: "claude".into(),
                model: "m".into()
            })
        );
    }

    #[test]
    fn a_threads_log_reads_as_its_page_shows_it() {
        let s = steps(concat!(
            "1|user: build it\nwith care\n",
            "2|talk: Looking.\n",
            "3|tool: Bash {\"command\":\"ls\"}\n",
            "4|echo: a b\n",
            "5|user: (From the person, typed on your page) also docs\n",
            "6|tool: SubagentHandback {\"message\":\"Done.\"}\n",
            "7|from: Abel (reply): one\n  two\n",
            "8|to: Abel: not drawn: the call is\n",
        ));
        let kinds: Vec<_> = s.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            [
                StepKind::Task,
                StepKind::Words,
                StepKind::Call,
                StepKind::Told,
                StepKind::Words,
                StepKind::Call,
            ]
        );
        assert_eq!(s[0].text, "build it\nwith care");
        assert_eq!((s[2].name.as_str(), s[2].text.as_str()), ("Bash", "ls"));
        assert_eq!(s[2].result.as_deref(), Some("a b"));
        assert_eq!(s[4].text, "Done.");
        assert_eq!(s[5].name, "Reply from Abel");
        assert_eq!(s[5].text, "one\ntwo");
        assert_eq!(s[5].result.as_deref(), Some(""));
    }

    // Two calls side by side whose results came back the other way round:
    // each result goes to its own call by id; a log without ids pairs by order.
    #[test]
    fn a_result_pairs_with_its_call_by_id() {
        let s = steps(concat!(
            "1|user: look\n",
            "2|tool [a]: Read {\"file_path\":\"x\"}\n",
            "3|tool [b]: Read {\"file_path\":\"y\"}\n",
            "4|echo [b]: why\n",
            "5|echo [a]: ex\n",
            "6|tool: Bash {\"command\":\"ls\"}\n",
            "7|echo: a b",
        ));
        let got: Vec<_> = s[1..].iter().map(|s| s.result.as_deref()).collect();
        assert_eq!(got, [Some("ex"), Some("why"), Some("a b")]);
    }

    #[test]
    fn the_scratchpad_reads_as_a_thread_of_the_persons_words() {
        let p = Scratchpad::of(&json!({
            "text": "1|user: hello\n2|talk: Here.\n3|file: a.png\n4|user: more\n",
            "working": true,
            "memory": false,
            "asks": [{"id": "scratchpad:q", "metadata": {"tool": "Bash", "input": {"command": "ls"}}}],
        }));
        let kinds: Vec<_> = p.steps.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            [
                StepKind::Told,
                StepKind::Words,
                StepKind::File,
                StepKind::Told,
            ]
        );
        assert_eq!(p.steps[2].text, "a.png");
        assert!(p.working);
        assert!(!p.memory);
        assert_eq!(p.asks[0].id, "scratchpad:q");
        assert!(!p.stopping);
    }

    #[test]
    fn the_scratchpad_ends_on_what_its_run_is_at_and_the_words_waiting() {
        let p = Scratchpad::of(&json!({
            "text": "0|user: hello\n1|talk: Looking.",
            "working": true,
            "words": "Nearly there.",
            "thinking": "Weighing it.",
            "stopping": true,
            "queued": ["and this\ntoo"],
        }));
        let got: Vec<_> = p.steps.iter().map(|s| (s.kind, s.text.as_str())).collect();
        assert_eq!(
            got,
            [
                (StepKind::Told, "hello"),
                (StepKind::Words, "Looking."),
                (StepKind::Thought, "Weighing it."),
                (StepKind::Words, "Nearly there."),
                (StepKind::Told, "and this\ntoo"),
            ]
        );
        assert!(p.stopping);
    }

    #[test]
    fn a_form_carries_its_file_whole() {
        let (kind, body) = form(&[("file", "a\".png", "image/png", b"\x89PNG")]);
        let boundary = kind.split("boundary=").nth(1).unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.starts_with(&format!("--{boundary}\r\n")));
        assert!(text.contains("filename=\"a_.png\""));
        assert!(text.ends_with(&format!("\r\n--{boundary}--\r\n")));
    }

    #[test]
    fn a_thread_reads_with_or_without_a_title_and_summary() {
        let bare = json!({"name": "Kit", "brief": "b", "state": "working", "at": 1, "read": true});
        let t: Thread = serde_json::from_value(bare.clone()).unwrap();
        assert_eq!((t.title, t.summary), (None, None));
        let mut full = bare;
        full["title"] = json!("Fixing the redirect");
        full["summary"] = json!("Reading the login handler");
        full["gist"] = json!("Reading the login handler");
        full["effort"] = json!("high");
        full["mismatch"] = json!("started on gpt-6.1-sol (codex), but the harness reports x");
        let t: Thread = serde_json::from_value(full).unwrap();
        assert_eq!(t.effort.as_deref(), Some("high"));
        assert!(t.mismatch.is_some());
        assert_eq!(t.title.as_deref(), Some("Fixing the redirect"));
        assert_eq!(t.summary.as_deref(), Some("Reading the login handler"));
        assert_eq!(t.gist, "Reading the login handler");
    }

    #[test]
    fn a_synopsis_model_reads_as_the_host_sends_it() {
        let picked = json!({"harness": "claude", "model": "a"});
        let m: SynopsisModel =
            serde_json::from_value(json!({"picked": picked, "default": null})).unwrap();
        assert_eq!(m.picked.unwrap().model, "a");
        assert_eq!(m.cheapest, None);
    }

    #[test]
    fn a_work_list_reads_the_tree_and_a_host_from_before_it() {
        let l: WorkList = serde_json::from_value(json!({
            "threads": [{
                "id": "kit-1200", "name": "Kit", "brief": "b", "state": "working",
                "gist": "", "at": 1, "read": true, "head": "ada-1100", "memory": "jobs",
                "pick": {"harness": "claude", "model": "claude-opus-5-5"},
                "failed": true, "calling": "Edit",
            }],
            "talk": [{"at": 5, "from": "ada-1100", "to": "kit-1200"}],
        }))
        .unwrap();
        let t = &l.threads[0];
        assert_eq!(
            (t.head.as_deref(), t.memory.as_deref()),
            (Some("ada-1100"), Some("jobs"))
        );
        assert_eq!(t.pick.as_ref().unwrap().harness, "claude");
        assert!(t.failed);
        assert_eq!(t.calling.as_deref(), Some("Edit"));
        assert_eq!(l.talk[0].to, "kit-1200");
        let l: WorkList = serde_json::from_value(json!({
            "threads": [{"name": "Kit", "brief": "b", "state": "sleeping", "gist": "", "at": 1, "read": true}],
        }))
        .unwrap();
        assert!(l.talk.is_empty() && l.threads[0].pick.is_none() && !l.threads[0].failed);
        let m: NamedMemory =
            serde_json::from_value(json!({"id": "jobs", "name": "Jobs", "instructions": ""}))
                .unwrap();
        assert!(!m.working && !m.asking);
    }

    #[test]
    fn a_memory_status_reads_with_only_its_state() {
        let m: MemoryHealth = serde_json::from_value(json!({"state": "working"})).unwrap();
        assert_eq!(
            (m.state.as_str(), m.reason, m.waiting),
            ("working", None, None)
        );
        let m: MemoryHealth = serde_json::from_value(json!({
            "state": "working", "lines": 76303, "bytes": 193_412_000u64,
        }))
        .unwrap();
        assert_eq!((m.lines, m.bytes), (Some(76303), Some(193_412_000)));
        let m: MemoryHealth = serde_json::from_value(json!({
            "state": "failing", "reason": "The memory isn't answering.",
            "since": "2026-10-30T16:35:29-04:00", "waiting": null,
            "newest_summary": null, "latest_message": null, "model": null,
            "last_error": null, "retries_last_hour": null,
        }))
        .unwrap();
        assert_eq!(m.reason.as_deref(), Some("The memory isn't answering."));
    }

    #[test]
    fn terminal_addresses_follow_the_scheme() {
        let h = Host::new("libri.example.com".into()).unwrap();
        assert_eq!(h.terminal_url(), "wss://libri.example.com/api/terminal");
        let h = Host::new("box:8951".into()).unwrap();
        assert_eq!(h.terminal_url(), "ws://box:8951/api/terminal");
        assert_eq!(h.call_socket_url(), "ws://box:8951/api/call/socket");
        assert_eq!(
            h.file_url("a b.png".into()),
            "http://box:8951/api/file/a%20b.png"
        );
    }

    #[test]
    fn a_call_card_carries_its_call_or_its_words() {
        let h = Host::new("box:8951".into()).unwrap();
        let whole = h.call_card_url(
            "mcp__hob__cameras".into(),
            Some(r#"{"a":1}"#.into()),
            Some(r#"{"structuredContent":{"x":1}}"#.into()),
            Some("words".into()),
            false,
            false,
            true,
        );
        assert!(whole.starts_with("http://box:8951/#call-card=mcp__hob__cameras&input="));
        assert!(whole.contains("structuredContent") && whole.ends_with("&full=1"));
        // Too long for an address: the words alone go, as content.
        let long = h.call_card_url(
            "t".into(),
            None,
            Some("x".repeat(600 * 1024)),
            Some("words".into()),
            true,
            true,
            false,
        );
        assert!(long.contains("%22text%22%3A%22words%22") && long.contains("&error=1&fresh=1"));
        assert!(long.len() < 2048);
    }
}
