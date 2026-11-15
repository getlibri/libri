//! The built-ins: mail and calendar as slots, each filled by the provider
//! the person picks (Fastmail now; Google and Microsoft once Libri can sign
//! in to them), with a switch in Settings, as Gemini switches on one connected
//! app at a time and Claude one connector. A built-in is no code path of
//! its own, and no provider is either. It is a row of `builtins.json`
//! naming, for each provider that can fill it, an existing remote MCP
//! server by its address, which of its tools read and which write, and for
//! a calendar
//! where in its answers an agenda's events lie. Picking a provider adds the
//! connection (or takes the one already at that address); the switch then
//! only sets which of its tools the connection withholds, so every harness
//! loses or gains them through the relay at once. Libri sets no access of
//! its own: what a provider lets Libri do is what its own sign-in and
//! settings grant, and every tool it offers passes straight through: its
//! own review flows (Fastmail's compose_event widget, draft_email) do the
//! confirming.
//!
//! Several built-ins may share one connection (one sign-in serves a
//! provider's mail and calendar). What they withhold is kept here,
//! apart from what the person withholds by hand, so turning one on gives
//! back only what it took.

use crate::connections::{self, Change};
use crate::host::{private_dir, write_private};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

#[derive(Debug, Deserialize)]
pub struct Builtin {
    pub id: String,
    pub title: String,
    pub about: String,
    /// Whether it is offered before the person touches its switch.
    pub on: bool,
    pub providers: Vec<Provider>,
}

#[derive(Debug, Deserialize)]
pub struct Provider {
    pub id: String,
    pub title: String,
    pub address: String,
    pub read: Vec<String>,
    /// The rest of the built-in's tools, there so turning it off withholds
    /// them too; a Library page never calls these.
    pub write: Vec<String>,
    /// What the person should know before picking it, in one line.
    #[serde(default)]
    pub note: Option<String>,
    /// Why it cannot be picked yet: hidden from the person and refused, its
    /// row kept so the day it can sign in is a deletion here.
    #[serde(default)]
    pub waiting: Option<String>,
    /// Where a calendar's events lie in the server's answers.
    #[serde(default)]
    pub agenda: Option<Agenda>,
    /// What the connection brings in that the person may show or hide, as
    /// a calendar app lists each calendar with a box to tick.
    #[serde(default)]
    pub items: Vec<Items>,
}

/// One kind of thing a connection brings in (its calendars), listed by a
/// tool of the provider's: where the list is in its answer, and each one's
/// id, name and colour.
#[derive(Debug, Deserialize)]
pub struct Items {
    pub kind: String,
    pub title: String,
    pub tool: String,
    pub list: String,
    pub id: Pointer,
    pub name: Pointer,
    #[serde(default)]
    pub color: Pointer,
}

/// One or more JSON pointers, the first that finds a value winning, as
/// Google gives a timed event's start at `/start/dateTime` and a whole
/// day's at `/start/date`.
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
pub enum Pointer {
    #[default]
    None,
    One(String),
    Any(Vec<String>),
}

impl Pointer {
    fn find<'a>(&self, v: &'a Value) -> Option<&'a Value> {
        let one = |p: &str| v.pointer(p).filter(|x| !x.is_null());
        match self {
            Pointer::None => None,
            Pointer::One(p) => one(p),
            Pointer::Any(ps) => ps.iter().find_map(|p| one(p)),
        }
    }

    fn text(&self, v: &Value) -> Option<String> {
        match self.find(v)? {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
}

/// How a calendar provider's events are listed: the tool and its
/// arguments, `{from}`, `{to}` (RFC 3339), `{from_local}`, `{to_local}`
/// (the same wall time with no offset) filled in, and where in its answer
/// each event's fields lie.
#[derive(Debug, Deserialize)]
pub struct Agenda {
    pub tool: String,
    pub arguments: Map<String, Value>,
    /// Where the list of events is; empty for the answer itself.
    pub list: String,
    #[serde(default)]
    pub id: Pointer,
    pub title: Pointer,
    pub start: Pointer,
    #[serde(default)]
    pub end: Pointer,
    /// An ISO 8601 duration, where the server gives one in place of an end.
    #[serde(default)]
    pub duration: Pointer,
    #[serde(default)]
    pub all_day: Pointer,
    /// The zone a start with no offset is a wall time in.
    #[serde(default)]
    pub time_zone: Pointer,
    #[serde(default)]
    pub location: Pointer,
    /// Which calendars an event is on: a string, a list of them, or an
    /// object keyed by them (JMAP's `calendarIds`).
    #[serde(default)]
    pub calendars: Pointer,
}

/// The ids a `calendars` pointer finds.
fn ids(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(str::to_owned))
            .collect(),
        Some(Value::Object(o)) => o
            .iter()
            .filter(|(_, on)| on.as_bool() != Some(false))
            .map(|(k, _)| k.clone())
            .collect(),
        _ => Vec::new(),
    }
}

impl Provider {
    fn tools(&self) -> impl Iterator<Item = &String> {
        self.read.iter().chain(&self.write)
    }

    /// The tools a built-in lets harnesses have: all of them when on, none
    /// when off.
    fn given(&self, on: bool) -> BTreeSet<&String> {
        if on {
            self.tools().collect()
        } else {
            BTreeSet::new()
        }
    }
}

#[derive(Deserialize)]
struct Presets {
    builtins: Vec<Builtin>,
}

static PRESETS: LazyLock<Vec<Builtin>> = LazyLock::new(|| {
    serde_json::from_str::<Presets>(include_str!("builtins.json"))
        .expect("builtins.json is valid")
        .builtins
});

fn preset(id: &str) -> Result<&'static Builtin, String> {
    PRESETS
        .iter()
        .find(|b| b.id == id)
        .ok_or_else(|| format!("There is no built-in {id}."))
}

/// What the person chose for one built-in.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Chosen {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
    /// Which provider fills it and by which connection, as it last took
    /// them: kept while that connection is there, its address edited or not;
    /// with none kept, the connected account that covers it fills it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct Kept {
    #[serde(default)]
    chosen: BTreeMap<String, Chosen>,
    /// By connection, the tools the built-ins withheld on it that the
    /// person had not.
    #[serde(default)]
    withheld: BTreeMap<String, BTreeSet<String>>,
    /// By connection and kind, the items the person hid.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    hidden: BTreeMap<String, BTreeMap<String, BTreeSet<String>>>,
}

fn path(state: &Path) -> PathBuf {
    connections::dir(state).join("builtins.json")
}

fn kept(state: &Path) -> Kept {
    fs::read_to_string(path(state))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn keep(state: &Path, k: &Kept) -> Result<(), String> {
    private_dir(&connections::dir(state))?;
    write_private(&path(state), &serde_json::to_string_pretty(k).unwrap())
}

/// Two server addresses as the same server: a trailing slash aside.
fn same(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

/// The connection already at a provider's address.
fn found_at(all: &[connections::Connection], p: &Provider) -> Option<String> {
    all.iter()
        .find(|x| x.url.as_deref().is_some_and(|u| same(u, &p.address)))
        .map(|x| x.name.clone())
}

/// The providers of `b` an account is connected for, each with its
/// connection: what could fill it.
fn covering<'a>(
    b: &'static Builtin,
    all: &'a [connections::Connection],
) -> impl Iterator<Item = (&'static Provider, String)> + 'a {
    b.providers
        .iter()
        .filter(|p| p.waiting.is_none())
        .filter_map(|p| Some((p, found_at(all, p)?)))
}

/// What fills `b`: the connection it took while that is there, else the
/// provider the person picked among those connected, else the first
/// connected one. None until an account covers it.
fn filled(
    b: &'static Builtin,
    c: &Chosen,
    all: &[connections::Connection],
) -> Option<(&'static Provider, String)> {
    if let (Some(pid), Some(name)) = (&c.provider, &c.connection)
        && all.iter().any(|x| &x.name == name)
        && let Some(p) = b
            .providers
            .iter()
            .find(|p| &p.id == pid && p.waiting.is_none())
    {
        return Some((p, name.clone()));
    }
    let mut first = None;
    for (p, name) in covering(b, all) {
        if c.provider.as_ref() == Some(&p.id) {
            return Some((p, name));
        }
        first.get_or_insert((p, name));
    }
    first
}

/// The connection a built-in took for provider `p` and still has, its
/// address edited or not.
fn taken(k: &Kept, all: &[connections::Connection], p: &Provider) -> Option<String> {
    k.chosen
        .values()
        .filter(|c| c.provider.as_ref() == Some(&p.id))
        .filter_map(|c| c.connection.clone())
        .find(|n| all.iter().any(|x| &x.name == n))
}

/// The accounts Libri knows how to connect: each provider once, with the
/// built-ins it covers and its connection when there is one. As iOS's and
/// GNOME's Online Accounts, an account is signed in to once and every
/// built-in it covers takes it.
pub fn accounts(state: &Path) -> Value {
    let all = connections::all(state);
    let k = kept(state);
    let mut out: Vec<Value> = Vec::new();
    for b in PRESETS.iter() {
        for p in b.providers.iter().filter(|p| p.waiting.is_none()) {
            if let Some(a) = out.iter_mut().find(|a| a["id"] == p.id.as_str()) {
                a["covers"].as_array_mut().unwrap().push(json!(b.title));
                continue;
            }
            let conn = taken(&k, &all, p).or_else(|| found_at(&all, p));
            out.push(json!({
                "id": p.id,
                "title": p.title,
                "note": p.note,
                "covers": [b.title],
                "signed_in": conn.as_ref().is_some_and(|n| connections::signed_in(state, n)),
                "connection": conn,
            }));
        }
    }
    Value::Array(out)
}

/// Connects the account of provider `id`: the connection at its address,
/// added unless one is there already, which the person then signs in to.
/// Every built-in the provider covers takes it from then on. Its name.
pub fn connect(state: &Path, id: &str) -> Result<String, String> {
    let p = PRESETS
        .iter()
        .flat_map(|b| &b.providers)
        .find(|p| p.id == id && p.waiting.is_none())
        .ok_or_else(|| format!("Libri cannot connect {id} yet."))?;
    let all = connections::all(state);
    let name = match taken(&kept(state), &all, p).or_else(|| found_at(&all, p)) {
        Some(name) => name,
        None => {
            connections::put(
                state,
                Change {
                    name: free(state, &p.id),
                    address: p.address.clone(),
                    ..Change::default()
                },
            )?
            .name
        }
    };
    settle_all(state)?;
    Ok(name)
}

/// The provider a connection is the account of, by its address.
fn provider_of(k: &Kept, all: &[connections::Connection], name: &str) -> Option<&'static Provider> {
    PRESETS
        .iter()
        .find_map(|b| {
            let c = k.chosen.get(&b.id).cloned().unwrap_or_default();
            filled(b, &c, all)
                .filter(|(_, n)| n == name)
                .map(|(p, _)| p)
        })
        .or_else(|| {
            let url = all.iter().find(|c| c.name == name)?.url.clone()?;
            PRESETS
                .iter()
                .flat_map(|b| &b.providers)
                .find(|p| p.waiting.is_none() && same(&p.address, &url))
        })
}

/// The kinds of thing connection `name` brings in, each with the tool that
/// lists them; none for a connection that is no known provider's.
pub fn item_kinds(state: &Path, name: &str) -> &'static [Items] {
    provider_of(&kept(state), &connections::all(state), name).map_or(&[], |p| p.items.as_slice())
}

/// Each item of `kind` in the listing tool's `answer`, with whether it is
/// shown.
pub fn items(state: &Path, name: &str, kind: &Items, answer: &Value) -> Vec<Value> {
    let k = kept(state);
    let hidden = k.hidden.get(name).and_then(|h| h.get(&kind.kind));
    listed(answer, &kind.list)
        .iter()
        .filter_map(|x| {
            let id = kind.id.text(x)?;
            Some(json!({
                "id": id,
                "name": kind.name.text(x).unwrap_or_else(|| id.clone()),
                "color": kind.color.text(x),
                "shown": !hidden.is_some_and(|h| h.contains(&id)),
            }))
        })
        .collect()
}

/// Shows or hides one item a connection brings in.
pub fn show_item(
    state: &Path,
    name: &str,
    kind: &str,
    id: &str,
    shown: bool,
) -> Result<(), String> {
    if !item_kinds(state, name).iter().any(|i| i.kind == kind) {
        return Err(format!("{name} lists no {kind}."));
    }
    let mut k = kept(state);
    let h = k.hidden.entry(name.to_owned()).or_default();
    let set = h.entry(kind.to_owned()).or_default();
    if shown {
        set.remove(id);
    } else {
        set.insert(id.to_owned());
    }
    h.retain(|_, s| !s.is_empty());
    k.hidden.retain(|_, h| !h.is_empty());
    keep(state, &k)
}

/// The calendars the person hid on connection `name`.
fn hidden_calendars(k: &Kept, name: &str) -> BTreeSet<String> {
    k.hidden
        .get(name)
        .and_then(|h| h.get("calendars"))
        .cloned()
        .unwrap_or_default()
}

/// Whether an event on `on` stays: on at least one shown calendar, or on
/// none the server named.
fn kept_event(on: &[String], hidden: &BTreeSet<String>) -> bool {
    on.is_empty() || on.iter().any(|c| !hidden.contains(c))
}

/// A call's result from connection `name`, with the events on hidden
/// calendars taken out where `tool` is its provider's agenda tool: the
/// relay passes every answer through it, so an agent's own search of the
/// calendar shows what the Calendar view shows.
pub fn trimmed(state: &Path, name: &str, tool: &str, mut result: Value) -> Value {
    let all = connections::all(state);
    let k = kept(state);
    let Some(a) = provider_of(&k, &all, name)
        .and_then(|p| p.agenda.as_ref())
        .filter(|a| a.tool == tool)
    else {
        return result;
    };
    let hidden = hidden_calendars(&k, name);
    if hidden.is_empty() {
        return result;
    }
    let keep_in = |v: &mut Value| {
        let list = if a.list.is_empty() {
            Some(v)
        } else {
            v.pointer_mut(&a.list)
        };
        if let Some(Value::Array(items)) = list {
            items.retain(|e| kept_event(&ids(a.calendars.find(e)), &hidden));
        }
    };
    if let Some(sc) = result.get_mut("structuredContent").filter(|v| !v.is_null()) {
        keep_in(sc);
    }
    // Most servers give the same JSON as text too, which a model reads.
    if let Some(content) = result["content"].as_array_mut() {
        for c in content {
            if let Some(t) = c["text"].as_str()
                && let Ok(mut v) = serde_json::from_str::<Value>(t)
            {
                if v.is_array() {
                    if let Value::Array(items) = &mut v {
                        items.retain(|e| kept_event(&ids(a.calendars.find(e)), &hidden));
                    }
                } else {
                    keep_in(&mut v);
                }
                c["text"] = v.to_string().into();
            }
        }
    }
    result
}

/// Each built-in as Settings shows it.
pub fn shown(state: &Path) -> Value {
    let k = kept(state);
    let all = connections::all(state);
    Value::Array(
        PRESETS
            .iter()
            .map(|b| {
                let c = k.chosen.get(&b.id).cloned().unwrap_or_default();
                let fill = filled(b, &c, &all);
                json!({
                    "id": b.id,
                    "title": b.title,
                    "about": b.about,
                    "on": c.on.unwrap_or(b.on),
                    "provider": fill.as_ref().map(|(p, _)| p.id.clone()),
                    "provider_title": fill.as_ref().map(|(p, _)| p.title.clone()),
                    "connection": fill.as_ref().map(|(_, n)| n.clone()),
                    "signed_in": fill.as_ref().is_some_and(|(_, n)| connections::signed_in(state, n)),
                    // A pick shows only when two connected accounts cover it.
                    "choices": covering(b, &all).map(|(p, n)| json!({
                        "id": p.id, "title": p.title, "connection": n,
                    })).collect::<Vec<_>>(),
                    "agenda": fill.as_ref().is_some_and(|(p, _)| p.agenda.is_some()),
                })
            })
            .collect(),
    )
}

/// A change the person makes to one built-in.
#[derive(Debug, Default, Deserialize)]
pub struct Set {
    #[serde(default)]
    pub on: Option<bool>,
    /// Which of two connected accounts fills it.
    #[serde(default)]
    pub provider: Option<String>,
}

/// Changes one built-in, and what its connection withholds to match.
pub fn set(state: &Path, id: &str, change: Set) -> Result<(), String> {
    let b = preset(id)?;
    let mut k = kept(state);
    let mut c = k.chosen.get(id).cloned().unwrap_or_default();
    if let Some(pid) = &change.provider {
        let all = connections::all(state);
        let Some((_, name)) = covering(b, &all).find(|(p, _)| &p.id == pid) else {
            return Err(format!(
                "Connect that account first: {} has no {pid} account to take.",
                b.title
            ));
        };
        c.provider = Some(pid.clone());
        c.connection = Some(name);
    }
    if let Some(on) = change.on {
        c.on = Some(on);
    }
    k.chosen.insert(id.to_owned(), c);
    keep(state, &k)?;
    settle_all(state)
}

/// A connection name not yet taken, from the provider's own.
fn free(state: &Path, base: &str) -> String {
    let taken: BTreeSet<String> = connections::all(state)
        .into_iter()
        .map(|c| c.name)
        .collect();
    (1..)
        .map(|n| {
            if n == 1 {
                base.to_owned()
            } else {
                format!("{base}-{n}")
            }
        })
        .find(|n| !taken.contains(n))
        .unwrap()
}

/// The built-ins filled through connection `name`, each with what the
/// person chose and its provider's preset.
fn on_connection(
    k: &Kept,
    all: &[connections::Connection],
    name: &str,
) -> Vec<(&'static Builtin, Chosen, &'static Provider)> {
    PRESETS
        .iter()
        .filter_map(|b| {
            let c = k.chosen.get(&b.id).cloned().unwrap_or_default();
            let (p, n) = filled(b, &c, all)?;
            (n == name).then_some((b, c, p))
        })
        .collect()
}

/// Settles every connection a built-in reaches against the presets this
/// release ships and the accounts connected now: a connection added since,
/// by Accounts or by hand at a provider's address, fills what it covers,
/// and a preset that gives more than an older one did (0.9.35 and before
/// withheld sending, inviting and deleting) gives it. The daemon runs it at
/// start and after any change to the connections.
pub fn settle_all(state: &Path) -> Result<(), String> {
    let mut k = kept(state);
    let all = connections::all(state);
    // What fills each built-in now, kept so an edit of its connection's
    // address keeps it.
    for b in PRESETS.iter() {
        let c = k.chosen.entry(b.id.clone()).or_default();
        let fill = filled(b, c, &all);
        c.provider = fill.as_ref().map(|(p, _)| p.id.clone());
        c.connection = fill.map(|(_, n)| n);
    }
    k.chosen.retain(|_, c| *c != Chosen::default());
    let names: BTreeSet<String> = PRESETS
        .iter()
        .flat_map(|b| covering(b, &all).map(|(_, n)| n))
        .chain(k.withheld.keys().cloned())
        .collect();
    for name in &names {
        settle(state, &mut k, &all, name)?;
    }
    keep(state, &k)
}

/// Sets what connection `name` withholds and whether it is on, from the
/// built-ins on it: a tool one of them gives stays given; the rest of their
/// tools are withheld. What the person withholds by hand stays withheld.
fn settle(
    state: &Path,
    k: &mut Kept,
    all: &[connections::Connection],
    name: &str,
) -> Result<(), String> {
    let Some(conn) = all.iter().find(|c| c.name == name) else {
        k.withheld.remove(name);
        return Ok(());
    };
    let mut theirs = BTreeSet::new();
    let mut given = BTreeSet::new();
    let mut any = false;
    let mut any_on = false;
    for (b, c, p) in on_connection(k, all, name) {
        any = true;
        let on = c.on.unwrap_or(b.on);
        any_on |= on;
        theirs.extend(p.tools().cloned());
        given.extend(p.given(on).into_iter().cloned());
    }
    let old = k.withheld.remove(name).unwrap_or_default();
    let mine: BTreeSet<String> = conn
        .withheld
        .iter()
        .filter(|t| !old.contains(*t))
        .cloned()
        .collect();
    let now: BTreeSet<String> = theirs.difference(&given).cloned().collect();
    let withheld: Vec<String> = mine.union(&now).cloned().collect();
    // A connection no built-in uses any longer is left as the person has it.
    let on = if any { any_on } else { conn.on };
    connections::settle(state, name, withheld, on)?;
    // Only what the built-ins added: one the person withheld already stays
    // theirs when a built-in gives it back.
    let added: BTreeSet<String> = now.difference(&mine).cloned().collect();
    if !added.is_empty() {
        k.withheld.insert(name.to_owned(), added);
    }
    Ok(())
}

/// Whether a Library page that names `connection` may call `tool` on it
/// without asking: the page reads, and nothing else. A built-in's preset
/// says which tools read; on any other connection the server's own
/// `readOnlyHint` does, the person having added that server themselves.
pub fn page_may_read(state: &Path, connection: &str, tool: &str, definition: &Value) -> bool {
    let Ok(conn) = connections::get(state, connection) else {
        return false;
    };
    if !conn.on || conn.withheld.iter().any(|t| t == tool) {
        return false;
    }
    let k = kept(state);
    let mut known = false;
    for (_, _, p) in on_connection(&k, &connections::all(state), connection) {
        if p.read.iter().any(|t| t == tool) {
            return true;
        }
        known |= p.tools().any(|t| t == tool);
    }
    !known && definition["annotations"]["readOnlyHint"] == Value::Bool(true)
}

/// Where a built-in's view is, as an MCP App's: `ui://libri/builtins/<id>`.
pub const VIEW_URI: &str = "ui://libri/builtins/";

/// A built-in's view: one page for every provider, which asks for its
/// agenda (site.rs builtin_call) and draws it.
pub fn view(uri: &str) -> Option<&'static str> {
    match uri.strip_prefix(VIEW_URI)? {
        "calendar" => Some(include_str!("views/calendar.html")),
        _ => None,
    }
}

/// What a built-in's view says while no connection fills it; the pages
/// know it, and offer Settings > Connections.
pub const NO_CONNECTION: &str = "No connection yet. Add a connection to see your calendar.";

/// The connection a built-in reaches and its provider's agenda, while it
/// is on and its provider lists one.
pub fn agenda(state: &Path, id: &str) -> Result<(String, &'static Agenda), String> {
    let b = preset(id)?;
    let c = kept(state).chosen.get(id).cloned().unwrap_or_default();
    if !c.on.unwrap_or(b.on) {
        return Err(format!("{} is off.", b.title));
    }
    let Some((p, name)) = filled(b, &c, &connections::all(state)) else {
        return Err(NO_CONNECTION.into());
    };
    let a = p
        .agenda
        .as_ref()
        .ok_or_else(|| format!("{} lists no agenda.", b.title))?;
    Ok((name, a))
}

/// A tool's arguments with `{from}`, `{to}`, `{from_local}` and
/// `{to_local}` filled in. `from` and `to` are RFC 3339.
pub fn arguments(template: &Map<String, Value>, from: &str, to: &str) -> Map<String, Value> {
    // The wall time alone: `2026-11-06T00:00:00` of `...-05:00`.
    let local = |t: &str| t.get(..19).unwrap_or(t).to_owned();
    template
        .iter()
        .map(|(k, v)| {
            let v = match v.as_str() {
                Some("{from}") => json!(from),
                Some("{to}") => json!(to),
                Some("{from_local}") => json!(local(from)),
                Some("{to_local}") => json!(local(to)),
                _ => v.clone(),
            };
            (k.clone(), v)
        })
        .collect()
}

/// What a tool call gave, as data: its structured content, or the JSON of
/// its text, as most servers still answer.
pub fn answer(result: &Value) -> Value {
    if let Some(v) = result.get("structuredContent").filter(|v| !v.is_null()) {
        return v.clone();
    }
    let text: String = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str())
        .collect();
    serde_json::from_str(&text).unwrap_or(Value::Null)
}

fn listed<'a>(v: &'a Value, at: &str) -> &'a [Value] {
    let v = if at.is_empty() {
        Some(v)
    } else {
        v.pointer(at)
    };
    v.and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// The events of an answer, each as every provider's agenda gives them:
/// `title`, `start`, and those of `end`, `duration`, `all_day`,
/// `time_zone`, `location` and `id` it has. Times are as the server gave
/// them; the view reads the zone.
/// The events of an answer on the calendars connection `name` shows.
pub fn shown_events(state: &Path, name: &str, a: &Agenda, answer: &Value) -> Vec<Value> {
    events(a, answer, &hidden_calendars(&kept(state), name))
}

fn events(a: &Agenda, answer: &Value, hidden: &BTreeSet<String>) -> Vec<Value> {
    listed(answer, &a.list)
        .iter()
        .filter(|e| kept_event(&ids(a.calendars.find(e)), hidden))
        .filter_map(|e| {
            let start = a.start.text(e)?;
            let mut o = Map::new();
            o.insert("title".into(), json!(a.title.text(e).unwrap_or_default()));
            // A date alone is a whole day, whatever else the server says.
            let whole = start.len() == 10 || a.all_day.find(e) == Some(&Value::Bool(true));
            o.insert("all_day".into(), json!(whole));
            o.insert("start".into(), json!(start));
            for (k, p) in [
                ("id", &a.id),
                ("end", &a.end),
                ("duration", &a.duration),
                ("time_zone", &a.time_zone),
                ("location", &a.location),
            ] {
                if let Some(v) = p.text(e) {
                    o.insert(k.into(), json!(v));
                }
            }
            Some(Value::Object(o))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh state directory for one test.
    struct Home(PathBuf);

    impl Home {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn home() -> Home {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("libri-builtins-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        Home(d)
    }

    fn withheld(state: &Path, name: &str) -> Vec<String> {
        connections::get(state, name).unwrap().withheld
    }

    #[test]
    fn the_presets_parse_and_name_each_tool_once() {
        for b in PRESETS.iter() {
            for p in &b.providers {
                let all: Vec<_> = p.tools().collect();
                let one: BTreeSet<_> = all.iter().collect();
                assert_eq!(all.len(), one.len(), "{} {}", b.id, p.id);
            }
        }
    }

    fn off(s: &Path, id: &str, on: bool) {
        set(
            s,
            id,
            Set {
                on: Some(on),
                ..Set::default()
            },
        )
        .unwrap();
    }

    #[test]
    fn connecting_fastmail_once_fills_mail_and_calendar() {
        let s = home();
        for b in shown(s.path()).as_array().unwrap() {
            assert!(b["connection"].is_null() && b["on"] == true, "{b}");
        }
        assert!(
            agenda(s.path(), "calendar")
                .unwrap_err()
                .contains("Add a connection")
        );
        assert_eq!(connect(s.path(), "fastmail").unwrap(), "fastmail");
        // Again: the same one, not a second.
        assert_eq!(connect(s.path(), "fastmail").unwrap(), "fastmail");
        assert_eq!(connections::all(s.path()).len(), 1);
        for b in shown(s.path()).as_array().unwrap() {
            assert_eq!(
                (b["connection"].as_str(), b["provider"].as_str()),
                (Some("fastmail"), Some("fastmail")),
                "{b}"
            );
            // One account covers it: no pick to show.
            assert_eq!(b["choices"].as_array().unwrap().len(), 1);
        }
        let c = connections::get(s.path(), "fastmail").unwrap();
        assert!(c.on && c.withheld.is_empty(), "{c:?}");
        assert_eq!(agenda(s.path(), "calendar").unwrap().0, "fastmail");
        // Its address edited, it still fills them, and lists its calendars.
        connections::put(
            s.path(),
            Change {
                name: "fastmail".into(),
                address: "python3 stand_in.py".into(),
                ..Change::default()
            },
        )
        .unwrap();
        assert_eq!(agenda(s.path(), "calendar").unwrap().0, "fastmail");
        assert_eq!(item_kinds(s.path(), "fastmail").len(), 1);
        // Nor is a second one offered or added.
        assert_eq!(accounts(s.path())[0]["connection"], "fastmail");
        assert_eq!(connect(s.path(), "fastmail").unwrap(), "fastmail");
    }

    #[test]
    fn a_connection_added_by_hand_at_the_address_is_taken() {
        let s = home();
        // As 0.9.35 left an install: Calendar switched, no provider.
        off(s.path(), "calendar", true);
        connections::put(
            s.path(),
            Change {
                name: "mine".into(),
                address: "https://api.fastmail.com/mcp/".into(),
                ..Change::default()
            },
        )
        .unwrap();
        assert_eq!(agenda(s.path(), "calendar").unwrap().0, "mine");
        let a = accounts(s.path());
        assert_eq!(a[0]["connection"], "mine");
        assert_eq!(a[0]["covers"], json!(["Calendar", "Mail"]));
    }

    #[test]
    fn a_provider_that_cannot_sign_in_yet_is_not_offered() {
        let s = home();
        let ids: Vec<_> = accounts(s.path())
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(ids, ["fastmail"]);
        assert!(connect(s.path(), "google").unwrap_err().contains("yet"));
        assert!(
            set(
                s.path(),
                "mail",
                Set {
                    provider: Some("google".into()),
                    ..Set::default()
                }
            )
            .is_err()
        );
        assert!(connections::all(s.path()).is_empty());
    }

    #[test]
    fn built_ins_sharing_a_connection_withhold_only_their_own() {
        let s = home();
        connect(s.path(), "fastmail").unwrap();
        off(s.path(), "mail", false);
        let w = withheld(s.path(), "fastmail");
        assert!(w.contains(&"search_email".into()) && w.contains(&"send_email".into()));
        assert!(!w.contains(&"search_events".into()) && !w.contains(&"delete_event".into()));
        assert!(connections::get(s.path(), "fastmail").unwrap().on);
        off(s.path(), "calendar", false);
        assert!(!connections::get(s.path(), "fastmail").unwrap().on);
        off(s.path(), "calendar", true);
        off(s.path(), "mail", true);
        assert!(withheld(s.path(), "fastmail").is_empty());
        assert!(connections::get(s.path(), "fastmail").unwrap().on);
    }

    #[test]
    fn a_start_gives_back_what_an_older_preset_withheld() {
        let s = home();
        connect(s.path(), "fastmail").unwrap();
        // As 0.9.35 left it: sending and deleting withheld by the built-in.
        let mut k = kept(s.path());
        let old: BTreeSet<String> = ["send_email".into(), "delete_email".into()].into();
        k.withheld.insert("fastmail".into(), old);
        keep(s.path(), &k).unwrap();
        let w = vec![
            "delete_email".into(),
            "send_email".into(),
            "set_memo".into(),
        ];
        connections::settle(s.path(), "fastmail", w, true).unwrap();
        settle_all(s.path()).unwrap();
        assert_eq!(withheld(s.path(), "fastmail"), vec!["set_memo".to_owned()]);
    }

    #[test]
    fn what_the_person_withholds_stays_withheld() {
        let s = home();
        connections::put(
            s.path(),
            Change {
                name: "fastmail".into(),
                address: "https://api.fastmail.com/mcp".into(),
                withheld: vec!["set_memo".into(), "search_events".into()],
                ..Change::default()
            },
        )
        .unwrap();
        off(s.path(), "calendar", false);
        off(s.path(), "calendar", true);
        let w = withheld(s.path(), "fastmail");
        assert!(w.contains(&"set_memo".into()));
        // The person's own, though the built-in would give it.
        assert!(w.contains(&"search_events".into()));
        assert!(!w.contains(&"delete_event".into()));
    }

    #[test]
    fn a_page_reads_and_never_acts() {
        let s = home();
        connect(s.path(), "fastmail").unwrap();
        let none = Value::Null;
        let ro = json!({"annotations": {"readOnlyHint": true}});
        assert!(page_may_read(s.path(), "fastmail", "search_events", &none));
        assert!(!page_may_read(s.path(), "fastmail", "create_event", &ro));
        assert!(!page_may_read(s.path(), "fastmail", "delete_event", &ro));
        // Outside every preset, the server's own word.
        assert!(page_may_read(s.path(), "fastmail", "search_notes", &ro));
        assert!(!page_may_read(s.path(), "fastmail", "update_note", &none));
        assert!(!page_may_read(s.path(), "nowhere", "search_events", &ro));
        off(s.path(), "calendar", false);
        off(s.path(), "mail", false);
        assert!(!page_may_read(s.path(), "fastmail", "search_events", &none));
    }

    #[test]
    fn hidden_calendars_leave_the_agenda_and_the_agents_search() {
        let s = home();
        connect(s.path(), "fastmail").unwrap();
        let kinds = item_kinds(s.path(), "fastmail");
        assert_eq!(kinds[0].kind, "calendars");
        let listed = json!({"items": [{"id": "A", "name": "Me", "color": "#004d40"}, {"id": "B", "name": "Home"}]});
        assert_eq!(
            items(s.path(), "fastmail", &kinds[0], &listed)[1]["shown"],
            true
        );
        show_item(s.path(), "fastmail", "calendars", "B", false).unwrap();
        assert_eq!(
            items(s.path(), "fastmail", &kinds[0], &listed)[1]["shown"],
            false
        );
        assert!(show_item(s.path(), "fastmail", "folders", "x", false).is_err());
        let answer = json!({"items": [
            {"title": "Mine", "start": "2026-11-07T09:00:00", "calendarIds": {"A": true}},
            {"title": "House", "start": "2026-11-07T10:00:00", "calendarIds": {"B": true}},
            {"title": "Both", "start": "2026-11-07T11:00:00", "calendarIds": {"A": true, "B": true}}]});
        let a = agenda(s.path(), "calendar").unwrap().1;
        let titles = |v: Vec<Value>| {
            v.iter()
                .map(|e| e["title"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            titles(shown_events(s.path(), "fastmail", a, &answer)),
            ["Mine", "Both"]
        );
        let result = json!({"structuredContent": answer, "content": [{"type": "text", "text": answer["items"].to_string()}]});
        let t = trimmed(s.path(), "fastmail", "search_events", result.clone());
        assert_eq!(t["structuredContent"]["items"].as_array().unwrap().len(), 2);
        assert!(!t["content"][0]["text"].as_str().unwrap().contains("House"));
        // Any other tool's answer is as the server gave it.
        assert_eq!(
            trimmed(s.path(), "fastmail", "search_email", result.clone()),
            result
        );
        show_item(s.path(), "fastmail", "calendars", "B", true).unwrap();
        assert!(kept(s.path()).hidden.is_empty());
    }

    #[test]
    fn every_built_in_has_fastmail_and_every_provider_is_a_remote_server() {
        for b in PRESETS.iter() {
            assert!(b.providers.iter().any(|p| p.id == "fastmail"), "{}", b.id);
            for p in &b.providers {
                assert!(p.address.starts_with("https://"), "{}", p.id);
            }
        }
    }

    #[test]
    fn an_agenda_reads_each_providers_answer_alike() {
        let fast = &preset("calendar").unwrap().providers[0];
        let a = fast.agenda.as_ref().unwrap();
        let got = json!({"items": [
            {"id": "e1", "title": "Veteran's Day", "start": "2026-11-11T00:00:00", "duration": "P1D",
             "isAllDay": true, "timeZone": "", "locations": []},
            {"id": "e2", "title": "Parker", "start": "2026-11-11T16:00:00", "duration": "PT1H",
             "isAllDay": false, "timeZone": "America/New_York", "locations": [{"name": "Office"}]}
        ]});
        let e = events(a, &got, &BTreeSet::new());
        assert_eq!(e[0]["all_day"], true);
        assert!(e[0].get("time_zone").is_none());
        assert_eq!(e[1]["time_zone"], "America/New_York");
        assert_eq!(e[1]["location"], "Office");
        assert_eq!(e[1]["duration"], "PT1H");
        let args = arguments(
            &a.arguments,
            "2026-11-06T00:00:00-05:00",
            "2026-11-13T00:00:00-05:00",
        );
        assert_eq!(args["after"], "2026-11-06T00:00:00");
        assert_eq!(args["limit"], 50);

        let g = preset("calendar")
            .unwrap()
            .providers
            .iter()
            .find(|p| p.id == "google")
            .unwrap();
        let e = events(
            g.agenda.as_ref().unwrap(),
            &json!({"events": [
            {"summary": "Holiday", "start": {"date": "2026-11-11"}, "end": {"date": "2026-11-12"}}]}),
            &BTreeSet::new(),
        );
        assert_eq!(e[0]["all_day"], true);
        assert_eq!(e[0]["start"], "2026-11-11");
    }

    #[test]
    fn an_agenda_needs_the_built_in_on() {
        let s = home();
        connect(s.path(), "fastmail").unwrap();
        assert_eq!(agenda(s.path(), "calendar").unwrap().0, "fastmail");
        off(s.path(), "calendar", false);
        assert!(agenda(s.path(), "calendar").is_err());
        assert!(agenda(s.path(), "mail").is_err());
    }
}
