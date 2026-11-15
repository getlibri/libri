//! The person's workspaces, kept on the host so every screen signed in to it
//! opens the same arrangement, and so the agent can read and change it as
//! Stencil's Tern lets its assistant run the workspace. Each change has a
//! version, which open screens wait on to follow it at once, and keeps the
//! arrangement before it, for one-step undo.
//!
//! Two stores, apart (a design choice of 2026-11-10): the wide page's
//! workspaces of tiles, and a phone's, each a row of items on the bar at its
//! foot. A phone shows one item at a time, so its workspaces are lists, not
//! trees, and arranging one never moves the other.

use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

/// Which of the person's arrangements: the wide page's or a phone's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Store {
    Desk,
    Phone,
}

impl Store {
    fn file(self) -> &'static str {
        match self {
            Store::Desk => "layout.json",
            Store::Phone => "phone_layout.json",
        }
    }

    fn check(self, v: &Value) -> Result<(), String> {
        match self {
            Store::Desk => check(v),
            Store::Phone => check_phone(v),
        }
    }
}

/// What a page may keep: far past any arrangement a person builds by hand.
pub const LARGEST: usize = 64 * 1024;
/// How long a closed workspace stays in Recently closed (a layout's
/// `closed`, each with `closed_at` in Unix seconds) before it goes for good.
pub const KEPT_CLOSED: u64 = 30 * 24 * 3600;
/// How long a page's wait for the next change is held open.
pub const WAIT: Duration = Duration::from_secs(25);
const BEAT: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Layout {
    /// 0 before anything was kept.
    pub version: u64,
    /// The arrangement, `null` before anything was kept.
    pub layout: Value,
    /// Who made the latest change: `person` or `agent`.
    pub by: Option<String>,
    /// Whether there is an arrangement before it to go back to.
    pub undo: bool,
}

/// The file as kept. One written before versions holds the arrangement
/// bare, and reads as version 0.
fn kept(state: &Path, store: Store) -> Map<String, Value> {
    let text = std::fs::read_to_string(state.join(store.file())).unwrap_or_default();
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) if m.contains_key("version") => m,
        Ok(bare @ Value::Object(_)) => {
            Map::from_iter([("version".into(), 0.into()), ("layout".into(), bare)])
        }
        _ => Map::new(),
    }
}

fn shaped(m: &Map<String, Value>) -> Layout {
    Layout {
        version: m.get("version").and_then(Value::as_u64).unwrap_or(0),
        layout: m.get("layout").cloned().unwrap_or(Value::Null),
        by: m.get("by").and_then(Value::as_str).map(str::to_owned),
        undo: m.get("before").is_some_and(|b| !b.is_null()),
    }
}

pub fn read(state: &Path, store: Store) -> Layout {
    let mut l = shaped(&kept(state, store));
    prune(&mut l.layout, now());
    l
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Drops the closed workspaces kept past KEPT_CLOSED.
fn prune(layout: &mut Value, now: u64) {
    if let Some(closed) = layout.get_mut("closed").and_then(Value::as_array_mut) {
        closed.retain(|w| {
            w.get("closed_at")
                .and_then(Value::as_u64)
                .is_some_and(|at| at + KEPT_CLOSED > now)
        });
    }
}

/// The named shells the person's tiles hold, in every workspace of both
/// arrangements, open or closed and kept: a terminal's `arg`.
pub fn shells(state: &Path) -> HashSet<String> {
    fn walk(v: &Value, out: &mut HashSet<String>) {
        match v {
            Value::Object(m) => {
                if m.get("kind").and_then(Value::as_str) == Some("terminal")
                    && let Some(a) = m.get("arg").and_then(Value::as_str)
                {
                    out.insert(a.to_owned());
                }
                m.values().for_each(|x| walk(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = HashSet::new();
    for store in [Store::Desk, Store::Phone] {
        let l = read(state, store).layout;
        for key in ["spaces", "closed"] {
            walk(&l[key], &mut out);
        }
    }
    out
}

/// Whole or not at all: written beside and renamed over.
fn write(state: &Path, store: Store, m: &Map<String, Value>) -> Result<(), String> {
    let new = state.join(format!(".{}.new", store.file()));
    std::fs::write(&new, Value::Object(m.clone()).to_string())
        .and_then(|()| std::fs::rename(&new, state.join(store.file())))
        .map_err(|e| format!("layout: {e}"))
}

/// Keeps `layout` as `by` made it, the one before it kept for undo. A change
/// to what is already there keeps nothing, so a page writing back what it
/// was just sent wakes no one.
pub fn change(state: &Path, store: Store, mut layout: Value, by: &str) -> Result<Layout, String> {
    let mut m = kept(state, store);
    let before = m.get("layout").cloned().unwrap_or(Value::Null);
    // A writer that knows nothing of Recently closed (an app from before
    // it) leaves its workspaces as they were, less any it shows again.
    if let (Some(o), Some(c)) = (layout.as_object_mut(), before.get("closed"))
        && !o.contains_key("closed")
    {
        let open: HashSet<&str> = o
            .get("spaces")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|w| w["id"].as_str())
            .collect();
        let kept: Vec<Value> = c
            .as_array()
            .into_iter()
            .flatten()
            .filter(|w| !w["id"].as_str().is_some_and(|id| open.contains(id)))
            .cloned()
            .collect();
        o.insert("closed".into(), kept.into());
    }
    prune(&mut layout, now());
    store.check(&layout)?;
    if layout.to_string().len() > LARGEST {
        return Err(format!("A layout is at most {} KB.", LARGEST / 1024));
    }
    if before == layout {
        return Ok(shaped(&m));
    }
    let version = m.get("version").and_then(Value::as_u64).unwrap_or(0) + 1;
    m.insert("version".into(), version.into());
    m.insert("by".into(), by.into());
    m.insert("before".into(), before);
    m.insert("layout".into(), layout);
    write(state, store, &m)?;
    Ok(shaped(&m))
}

/// Back to the arrangement before the latest change, once.
pub fn undo(state: &Path, store: Store, by: &str) -> Result<Layout, String> {
    let mut m = kept(state, store);
    let before = m.remove("before").unwrap_or(Value::Null);
    if before.is_null() {
        return Err("There is no change to undo.".into());
    }
    let version = m.get("version").and_then(Value::as_u64).unwrap_or(0) + 1;
    m.insert("version".into(), version.into());
    m.insert("by".into(), by.into());
    m.insert("layout".into(), before);
    write(state, store, &m)?;
    Ok(shaped(&m))
}

/// The layout once its version is past `since`, or as it stands after
/// `limit` with none.
pub async fn wait(state: &Path, store: Store, since: u64, limit: Duration) -> Layout {
    let until = Instant::now() + limit;
    loop {
        let s = state.to_owned();
        let now = tokio::task::spawn_blocking(move || read(&s, store))
            .await
            .unwrap_or_else(|_| read(state, store));
        if now.version != since || Instant::now() >= until {
            return now;
        }
        tokio::time::sleep(BEAT).await;
    }
}

/// Whether the arrangement is one the page can draw (tiles.tsx `readable`),
/// stricter where an agent's mistake would otherwise pass: ids unique, the
/// workspace shown and each focus real, splits between 0 and 1.
pub fn check(v: &Value) -> Result<(), String> {
    let spaces = v
        .get("spaces")
        .and_then(Value::as_array)
        .filter(|s| !s.is_empty())
        .ok_or("A layout has `spaces`, a list of at least one workspace.")?;
    let mut ids = HashSet::new();
    for w in spaces {
        let id = text(w, "id", "A workspace")?;
        text(w, "name", "A workspace")?;
        if !ids.insert(id.to_owned()) {
            return Err(format!("Id `{id}` is used twice."));
        }
        let root = w
            .get("root")
            .ok_or(format!("Workspace `{id}` has no `root`."))?;
        let mut leaves = Vec::new();
        node(root, &mut ids, &mut leaves)?;
        match w.get("focus").and_then(Value::as_str) {
            Some(f) if leaves.contains(&f.to_owned()) => {}
            _ => {
                return Err(format!(
                    "Workspace `{id}`'s `focus` is not one of its tiles."
                ));
            }
        }
    }
    let on = text(v, "on", "A layout")?;
    if !spaces.iter().any(|w| w["id"] == on) {
        return Err(format!("`on` names no workspace: `{on}`."));
    }
    for w in closed(v)? {
        let id = text(w, "id", "A closed workspace")?;
        text(w, "name", "A closed workspace")?;
        if !ids.insert(id.to_owned()) {
            return Err(format!("Id `{id}` is used twice."));
        }
        node(
            w.get("root")
                .ok_or(format!("Closed workspace `{id}` has no `root`."))?,
            &mut ids,
            &mut Vec::new(),
        )?;
    }
    if let Some(ts) = v.get("templates") {
        for t in ts.as_array().ok_or("`templates` is a list.")? {
            let id = text(t, "id", "A template")?;
            text(t, "name", "A template")?;
            if !ids.insert(id.to_owned()) {
                return Err(format!("Id `{id}` is used twice."));
            }
            node(
                t.get("root")
                    .ok_or(format!("Template `{id}` has no `root`."))?,
                &mut ids,
                &mut Vec::new(),
            )?;
        }
    }
    Ok(())
}

/// Whether a phone's arrangement is one the phone can draw (phone.tsx
/// `readable`): ids unique, each workspace at least one item and none twice,
/// the workspace shown a real one.
pub fn check_phone(v: &Value) -> Result<(), String> {
    let spaces = v
        .get("spaces")
        .and_then(Value::as_array)
        .filter(|s| !s.is_empty())
        .ok_or("A phone layout has `spaces`, a list of at least one workspace.")?;
    let mut ids = HashSet::new();
    for w in spaces.iter().chain(closed(v)?) {
        let id = text(w, "id", "A workspace")?;
        text(w, "name", "A workspace")?;
        if !ids.insert(id.to_owned()) {
            return Err(format!("Id `{id}` is used twice."));
        }
        let items = w
            .get("items")
            .and_then(Value::as_array)
            .filter(|i| !i.is_empty())
            .ok_or(format!(
                "Workspace `{id}` has `items`, a list of at least one."
            ))?;
        let mut seen = HashSet::new();
        for p in items {
            let kind = p.get("kind").and_then(Value::as_str).unwrap_or("");
            let arg = p.get("arg").and_then(Value::as_str);
            crate::tiles::pane_fits(kind, arg, true)
                .and_then(|()| crate::tiles::tabs_fit(p))
                .map_err(|e| format!("Workspace `{id}`: {e}"))?;
            if !seen.insert((kind, arg)) {
                return Err(format!("Workspace `{id}` holds `{kind}` twice."));
            }
        }
    }
    let on = text(v, "on", "A phone layout")?;
    if !spaces.iter().any(|w| w["id"] == on) {
        return Err(format!("`on` names no workspace: `{on}`."));
    }
    Ok(())
}

/// A layout's closed workspaces, each with when it was closed.
fn closed(v: &Value) -> Result<&[Value], String> {
    let Some(c) = v.get("closed") else {
        return Ok(&[]);
    };
    let c = c.as_array().ok_or("`closed` is a list.")?;
    if c.iter()
        .any(|w| w.get("closed_at").and_then(Value::as_u64).is_none())
    {
        return Err("A closed workspace has `closed_at`, in Unix seconds.".into());
    }
    Ok(c)
}

fn text<'a>(v: &'a Value, key: &str, what: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(format!("{what} needs `{key}`, a string."))
}

fn node(n: &Value, ids: &mut HashSet<String>, leaves: &mut Vec<String>) -> Result<(), String> {
    let id = text(n, "id", "A tile")?;
    if !ids.insert(id.to_owned()) {
        return Err(format!("Id `{id}` is used twice."));
    }
    match n.get("t").and_then(Value::as_str) {
        Some("leaf") => {
            match n.get("pane") {
                None | Some(Value::Null) => {}
                Some(p) => {
                    let kind = p.get("kind").and_then(Value::as_str).unwrap_or("");
                    crate::tiles::pane_fits(kind, p.get("arg").and_then(Value::as_str), false)
                        .and_then(|()| crate::tiles::tabs_fit(p))
                        .map_err(|e| format!("Tile `{id}`: {e}"))?;
                }
            }
            leaves.push(id.to_owned());
            Ok(())
        }
        Some("split") => {
            if !matches!(n.get("dir").and_then(Value::as_str), Some("row" | "col")) {
                return Err(format!("Split `{id}`'s `dir` is `row` or `col`."));
            }
            match n.get("at").and_then(Value::as_f64) {
                Some(at) if at > 0.0 && at < 1.0 => {}
                _ => return Err(format!("Split `{id}`'s `at` is between 0 and 1.")),
            }
            for side in ["a", "b"] {
                node(
                    n.get(side)
                        .ok_or(format!("Split `{id}` has no `{side}`."))?,
                    ids,
                    leaves,
                )?;
            }
            Ok(())
        }
        _ => Err(format!("Tile `{id}`'s `t` is `leaf` or `split`.")),
    }
}

/// Where `open` puts what it opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Where {
    /// Where it already shows, if anywhere; else beside, as `Beside`.
    Focus,
    /// A tile (a bar item) of its own beside the focused one, even where one
    /// already shows it.
    Beside,
    /// A new workspace holding it.
    NewWorkspace,
}

/// A name no tile, split or workspace has.
fn fresh(p: &str) -> String {
    static MADE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let made = MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{p}{n:x}{made:x}")
}

/// The same pane: its kind and what its arg names.
fn same(a: &Value, b: &Value) -> bool {
    a["kind"] == b["kind"] && a["arg"].as_str().unwrap_or("") == b["arg"].as_str().unwrap_or("")
}

/// `pane` as one opened beside an open twin: a kind that gives its second
/// one an arg of its own (tiles.rs `Fresh`) gets it; any other shows the same.
fn another(pane: &Value, twin: bool) -> Value {
    let kind = pane["kind"].as_str().unwrap_or("");
    match crate::tiles::get(kind).and_then(|t| t.fresh) {
        Some(f) if twin && pane["arg"].is_null() => {
            let arg = match f {
                crate::tiles::Fresh::Name { prefix } => fresh(prefix),
                crate::tiles::Fresh::Word { word } => word.to_owned(),
            };
            json!({"kind": kind, "arg": arg})
        }
        _ => pane.clone(),
    }
}

/// The arrangement with `pane` opened `at`, and what was done, in words.
/// Nothing kept yet stands for the first run: on a wide screen one
/// workspace, the chat alone (tiles.tsx `START`); on a phone the chat and
/// the Library (phone.tsx `FIRST`).
pub fn open(l: &Value, store: Store, pane: &Value, at: Where) -> Result<(Value, String), String> {
    let kind = pane["kind"].as_str().unwrap_or("");
    let arg = pane["arg"].as_str();
    crate::tiles::pane_fits(kind, arg, store == Store::Phone)?;
    let pane = match arg {
        Some(a) => json!({"kind": kind, "arg": a}),
        None => json!({"kind": kind}),
    };
    let name = crate::tiles::get(kind).map_or(kind, |t| t.name);
    match store {
        Store::Desk => open_desk(l, &pane, at, name),
        Store::Phone => open_phone(l, &pane, at, name),
    }
}

fn open_desk(l: &Value, pane: &Value, at: Where, name: &str) -> Result<(Value, String), String> {
    let mut l = if l.is_null() {
        let (w, chat) = (fresh("w"), fresh("l"));
        json!({"spaces": [{"id": w, "name": "Chat", "root": {"t": "leaf", "id": chat,
            "pane": {"kind": "chat"}}, "focus": chat, "full": false}], "on": w})
    } else {
        l.clone()
    };
    /// The way down to the first tile that passes `test`, side by side.
    fn path(n: &Value, test: &dyn Fn(&Value) -> bool) -> Option<Vec<&'static str>> {
        if test(n) {
            return Some(Vec::new());
        }
        ["a", "b"].into_iter().find_map(|side| {
            let mut p = path(n.get(side)?, test)?;
            p.insert(0, side);
            Some(p)
        })
    }
    let spaces = l["spaces"].as_array().cloned().unwrap_or_default();
    if at == Where::Focus {
        for w in &spaces {
            if let Some(p) = path(&w["root"], &|n| n["t"] == "leaf" && same(&n["pane"], pane)) {
                let mut at = &w["root"];
                for side in &p {
                    at = &at[*side];
                }
                let tile = at["id"].clone();
                let id = w["id"].clone();
                let space = l["spaces"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|x| x["id"] == id)
                    .unwrap();
                space["focus"] = tile;
                space["full"] = json!(false);
                l["on"] = id;
                check(&l)?;
                return Ok((l, format!("The {name} already showed; it is focused now.")));
            }
        }
    }
    if at == Where::NewWorkspace {
        let (w, tile) = (fresh("w"), fresh("l"));
        l["spaces"]
            .as_array_mut()
            .ok_or("The layout has no workspaces.")?
            .push(json!({
            "id": w, "name": name, "root": {"t": "leaf", "id": tile, "pane": pane},
            "focus": tile, "full": false}));
        l["on"] = json!(w);
        check(&l)?;
        return Ok((l, format!("A new workspace, {name}, shows it.")));
    }
    let on = l["on"].clone();
    let w = l["spaces"]
        .as_array_mut()
        .and_then(|s| s.iter_mut().find(|w| w["id"] == on))
        .ok_or("The layout shows no workspace.")?;
    let twin = path(&w["root"], &|n| n["t"] == "leaf" && same(&n["pane"], pane)).is_some();
    let pane = another(pane, twin);
    let empty = path(&w["root"], &|n| n["t"] == "leaf" && n["pane"].is_null());
    let focus = w["focus"].clone();
    let focused = path(&w["root"], &|n| n["id"] == focus).unwrap_or_default();
    let mut place = &mut w["root"];
    for side in empty.as_deref().unwrap_or(&focused) {
        place = &mut place[*side];
    }
    let said = if empty.is_some() {
        place["pane"] = pane;
        w["focus"] = place["id"].clone();
        format!("The {name} shows in the empty tile.")
    } else {
        let old = place.take();
        let tile = fresh("l");
        *place = json!({"t": "split", "id": fresh("s"), "dir": "row", "at": 0.5, "a": old,
            "b": {"t": "leaf", "id": tile, "pane": pane}});
        w["focus"] = json!(tile);
        format!("The {name} shows beside the focused tile.")
    };
    w["full"] = json!(false);
    check(&l)?;
    Ok((l, said))
}

/// A bar item's key, as the phone keeps the one it shows (phone.tsx
/// `paneKey`): its kind, and `:arg` if it has one.
fn key(pane: &Value) -> String {
    match pane["arg"].as_str() {
        Some(a) => format!("{}:{a}", pane["kind"].as_str().unwrap_or("")),
        None => pane["kind"].as_str().unwrap_or("").to_owned(),
    }
}

/// On a phone the item opened is shown as well: a workspace's `shown`
/// names it, which the phone takes once from an agent's change.
fn open_phone(l: &Value, pane: &Value, at: Where, name: &str) -> Result<(Value, String), String> {
    let mut l = if l.is_null() {
        let w = fresh("p");
        json!({"spaces": [{"id": w, "name": "Main", "items": [{"kind": "chat"}, {"kind": "library"}]}], "on": w})
    } else {
        l.clone()
    };
    if at == Where::Focus {
        let found = l["spaces"].as_array().into_iter().flatten().find_map(|w| {
            w["items"]
                .as_array()?
                .iter()
                .find(|p| same(p, pane))
                .map(|_| w["id"].clone())
        });
        if let Some(id) = found {
            let w = l["spaces"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|w| w["id"] == id)
                .unwrap();
            w["shown"] = json!(key(pane));
            l["on"] = id;
            check_phone(&l)?;
            return Ok((l, format!("The {name} was on the bar; it shows now.")));
        }
    }
    if at == Where::NewWorkspace {
        let w = fresh("p");
        let main_chat = pane["kind"] == "chat" && pane["arg"].is_null();
        let items = if main_chat {
            json!([{"kind": "chat"}])
        } else {
            json!([{"kind": "chat"}, pane])
        };
        l["spaces"]
            .as_array_mut()
            .ok_or("The layout has no workspaces.")?
            .push(json!({"id": w, "name": name, "items": items, "shown": key(pane)}));
        l["on"] = json!(w);
        check_phone(&l)?;
        return Ok((l, format!("A new workspace, {name}, shows it.")));
    }
    let on = l["on"].clone();
    let w = l["spaces"]
        .as_array_mut()
        .and_then(|s| s.iter_mut().find(|w| w["id"] == on))
        .ok_or("The layout shows no workspace.")?;
    let items = w["items"]
        .as_array_mut()
        .ok_or("The workspace has no items.")?;
    let twin = items.iter().any(|p| same(p, pane));
    let pane = another(pane, twin);
    if !items.iter().any(|p| same(p, &pane)) {
        items.push(pane.clone());
    }
    w["shown"] = json!(key(&pane));
    check_phone(&l)?;
    Ok((l, format!("The {name} is on the bar and shows.")))
}

/// The arrangement in a few lines, as the agent reads it before changing it.
pub fn outline(l: &Layout, store: Store) -> String {
    if store == Store::Phone {
        return phone_outline(l);
    }
    if l.layout.is_null() {
        return "No layout kept yet: the page opens one workspace, the chat alone.".into();
    }
    fn tree(n: &Value, depth: usize, out: &mut String) {
        let pad = "  ".repeat(depth);
        if n["t"] == "split" {
            let dir = if n["dir"] == "row" {
                "side by side"
            } else {
                "stacked"
            };
            out.push_str(&format!(
                "{pad}split {} {dir} at {}\n",
                n["id"].as_str().unwrap_or("?"),
                n["at"]
            ));
            tree(&n["a"], depth + 1, out);
            tree(&n["b"], depth + 1, out);
        } else {
            let shows = match &n["pane"] {
                Value::Null => "empty".to_owned(),
                p => match p["arg"].as_str() {
                    Some(a) => format!("{} {a}", p["kind"].as_str().unwrap_or("?")),
                    None => p["kind"].as_str().unwrap_or("?").to_owned(),
                },
            };
            out.push_str(&format!(
                "{pad}tile {}: {shows}\n",
                n["id"].as_str().unwrap_or("?")
            ));
        }
    }
    let mut out = format!(
        "Version {}, last changed by {}.\n",
        l.version,
        l.by.as_deref().unwrap_or("person")
    );
    let on = l.layout["on"].as_str().unwrap_or("");
    for w in l.layout["spaces"].as_array().into_iter().flatten() {
        let id = w["id"].as_str().unwrap_or("?");
        let mark = if id == on { " (shown)" } else { "" };
        out.push_str(&format!(
            "workspace {id} \"{}\"{mark}\n",
            w["name"].as_str().unwrap_or("")
        ));
        tree(&w["root"], 1, &mut out);
    }
    closed_outline(&l.layout, &mut out);
    for t in l.layout["templates"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "template {} \"{}\"\n",
            t["id"].as_str().unwrap_or("?"),
            t["name"].as_str().unwrap_or("")
        ));
    }
    out.push_str(&format!("JSON: {}", json!(l.layout)));
    out
}

/// The workspaces in Recently closed, for an agent's outline.
fn closed_outline(l: &Value, out: &mut String) {
    for w in l["closed"].as_array().into_iter().flatten() {
        out.push_str(&format!(
            "closed workspace {} \"{}\" (closed at {}; reopen by moving it back into `spaces`)\n",
            w["id"].as_str().unwrap_or("?"),
            w["name"].as_str().unwrap_or(""),
            w["closed_at"].as_u64().unwrap_or(0)
        ));
    }
}

fn phone_outline(l: &Layout) -> String {
    if l.layout.is_null() {
        return "No phone layout kept yet: the phone opens one workspace, Main, \
            holding the chat, the workers and the Library."
            .into();
    }
    let mut out = format!(
        "Version {}, last changed by {}.\n",
        l.version,
        l.by.as_deref().unwrap_or("person")
    );
    let on = l.layout["on"].as_str().unwrap_or("");
    for w in l.layout["spaces"].as_array().into_iter().flatten() {
        let id = w["id"].as_str().unwrap_or("?");
        let mark = if id == on { " (shown)" } else { "" };
        let items: Vec<String> = w["items"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| match p["arg"].as_str() {
                Some(a) => format!("{} {a}", p["kind"].as_str().unwrap_or("?")),
                None => p["kind"].as_str().unwrap_or("?").to_owned(),
            })
            .collect();
        out.push_str(&format!(
            "workspace {id} \"{}\"{mark}: {}\n",
            w["name"].as_str().unwrap_or(""),
            items.join(", ")
        ));
    }
    closed_outline(&l.layout, &mut out);
    out.push_str(&format!("JSON: {}", json!(l.layout)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory removed when the test ends.
    struct Gone(std::path::PathBuf);
    impl Drop for Gone {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn closed_workspaces_are_kept_a_while_and_name_their_shells() {
        let dir = std::env::temp_dir().join(format!("libri-layout-closed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = Gone(dir);
        let now = now();
        let l = json!({"spaces": [{"id": "w", "name": "Main", "focus": "c", "full": false,
            "root": {"t": "leaf", "id": "c", "pane": {"kind": "chat"}}}], "on": "w",
            "closed": [
                {"id": "x", "name": "Shells", "focus": "t", "full": false, "closed_at": now - 60,
                 "root": {"t": "leaf", "id": "t", "pane": {"kind": "terminal", "arg": "t2"}}},
                {"id": "y", "name": "Old", "focus": "o", "full": false, "closed_at": now - KEPT_CLOSED - 1,
                 "root": {"t": "leaf", "id": "o", "pane": {"kind": "terminal", "arg": "t9"}}}]});
        let kept = change(dir.0.as_path(), Store::Desk, l.clone(), "person").unwrap();
        let closed = kept.layout["closed"].as_array().unwrap();
        assert_eq!(closed.len(), 1, "the one past 30 days is gone");
        assert_eq!(closed[0]["id"], "x");
        assert_eq!(shells(dir.0.as_path()), HashSet::from(["t2".to_owned()]));
        assert!(outline(&kept, Store::Desk).contains("closed workspace x \"Shells\""));
        let mut twice = l.clone();
        twice["closed"][0]["id"] = "w".into();
        assert!(check(&twice).unwrap_err().contains("used twice"));
        let mut undated = l;
        undated["closed"][0]
            .as_object_mut()
            .unwrap()
            .remove("closed_at");
        assert!(check(&undated).unwrap_err().contains("closed_at"));
        let phone = json!({"spaces": [{"id": "p", "name": "Main", "items": [{"kind": "chat"}]}], "on": "p",
            "closed": [{"id": "q", "name": "Shell", "items": [{"kind": "terminal", "arg": "t4"}], "closed_at": now}]});
        change(dir.0.as_path(), Store::Phone, phone, "person").unwrap();
        assert!(shells(dir.0.as_path()).contains("t4"));
        // An app that knows nothing of `closed` leaves it as it was.
        let old_app = json!({"spaces": [{"id": "p", "name": "Main", "items": [{"kind": "chat"}]}], "on": "p"});
        let kept = change(dir.0.as_path(), Store::Phone, old_app, "person").unwrap();
        assert_eq!(kept.layout["closed"][0]["id"], "q");
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("libri-layout-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn one() -> Value {
        json!({"spaces": [{"id": "w1", "name": "Main", "focus": "l1", "full": false,
            "root": {"t": "split", "id": "s1", "dir": "row", "at": 0.6,
                "a": {"t": "leaf", "id": "l1", "pane": {"kind": "chat"}},
                "b": {"t": "leaf", "id": "l2", "pane": null}}}], "on": "w1"})
    }

    #[test]
    fn a_change_is_versioned_and_undone_once() {
        let dir = scratch("versioned");
        let first = change(&dir, Store::Desk, one(), "person").unwrap();
        assert_eq!((first.version, first.undo), (1, false));
        let mut two = one();
        two["spaces"][0]["name"] = "Work".into();
        let second = change(&dir, Store::Desk, two, "agent").unwrap();
        assert_eq!(second.by.as_deref(), Some("agent"));
        let back = undo(&dir, Store::Desk, "person").unwrap();
        assert_eq!((back.version, back.layout.clone()), (3, one()));
        assert!(!back.undo);
        assert!(undo(&dir, Store::Desk, "person").is_err());
    }

    #[test]
    fn the_same_layout_again_keeps_nothing() {
        let dir = scratch("same");
        change(&dir, Store::Desk, one(), "person").unwrap();
        assert_eq!(
            change(&dir, Store::Desk, one(), "person").unwrap().version,
            1
        );
    }

    #[test]
    fn a_bare_layout_from_before_versions_reads() {
        let dir = scratch("bare");
        std::fs::write(&dir.join("layout.json"), one().to_string()).unwrap();
        let l = read(&dir, Store::Desk);
        assert_eq!((l.version, l.layout), (0, one()));
    }

    #[test]
    fn mistakes_are_named() {
        let mut dup = one();
        dup["spaces"][0]["root"]["b"]["id"] = "l1".into();
        assert!(check(&dup).unwrap_err().contains("twice"));
        let mut kind = one();
        kind["spaces"][0]["root"]["a"]["pane"]["kind"] = "nowhere".into();
        assert!(check(&kind).unwrap_err().contains("kind is one of"));
        let mut on = one();
        on["on"] = "w9".into();
        assert!(check(&on).is_err());
        let mut focus = one();
        focus["spaces"][0]["focus"] = "s1".into();
        assert!(check(&focus).is_err());
    }

    #[test]
    fn open_focuses_or_adds_a_tile() {
        let t = json!({"kind": "terminal"});
        // Nothing kept: the page's first run, with the terminal beside it.
        let (first, _) = open(&Value::Null, Store::Desk, &t, Where::Focus).unwrap();
        check(&first).unwrap();
        let root = &first["spaces"][0]["root"];
        assert_eq!(root["a"]["pane"]["kind"], "chat");
        assert_eq!(root["b"]["pane"]["kind"], "terminal");
        // Focus finds it; beside opens a shell of its own.
        let (again, said) = open(&first, Store::Desk, &t, Where::Focus).unwrap();
        assert!(said.contains("focused"), "{said}");
        assert_eq!(again["spaces"][0]["focus"], root["b"]["id"]);
        let (two, _) = open(&first, Store::Desk, &t, Where::Beside).unwrap();
        let b = &two["spaces"][0]["root"]["b"];
        assert_eq!(b["t"], "split");
        assert!(b["b"]["pane"]["arg"].as_str().unwrap().starts_with('t'));
        // An empty tile takes it before anything splits.
        let l = json!({"spaces": [{"id": "w", "name": "W", "focus": "c", "full": false,
            "root": {"t": "split", "id": "s", "dir": "col", "at": 0.5,
                "a": {"t": "leaf", "id": "c", "pane": {"kind": "chat"}},
                "b": {"t": "leaf", "id": "e", "pane": null}}}], "on": "w"});
        let (filled, _) = open(&l, Store::Desk, &t, Where::Focus).unwrap();
        assert_eq!(filled["spaces"][0]["root"]["b"]["pane"]["kind"], "terminal");
        // A new workspace holds it alone and is shown.
        let (made, _) = open(
            &l,
            Store::Desk,
            &json!({"kind": "board", "arg": "b1"}),
            Where::NewWorkspace,
        )
        .unwrap();
        assert_eq!(made["spaces"].as_array().unwrap().len(), 2);
        assert_eq!(made["on"], made["spaces"][1]["id"]);
        // A kind that isn't, or one missing its arg, is refused.
        assert!(open(&l, Store::Desk, &json!({"kind": "workers"}), Where::Focus).is_err());
        assert!(open(&l, Store::Desk, &json!({"kind": "thread"}), Where::Focus).is_err());
    }

    #[test]
    fn open_on_a_phone_adds_the_item_and_shows_it() {
        let (l, _) = open(
            &Value::Null,
            Store::Phone,
            &json!({"kind": "terminal"}),
            Where::Focus,
        )
        .unwrap();
        let w = &l["spaces"][0];
        assert_eq!(w["items"].as_array().unwrap().len(), 3);
        assert_eq!(w["shown"], "terminal");
        let (two, _) = open(
            &l,
            Store::Phone,
            &json!({"kind": "terminal"}),
            Where::Beside,
        )
        .unwrap();
        assert_eq!(two["spaces"][0]["items"].as_array().unwrap().len(), 4);
        assert!(
            two["spaces"][0]["shown"]
                .as_str()
                .unwrap()
                .starts_with("terminal:t")
        );
        assert!(open(&l, Store::Phone, &json!({"kind": "home"}), Where::Focus).is_err());
    }

    fn phone() -> Value {
        json!({"spaces": [
            {"id": "p1", "name": "Chat", "items": [{"kind": "chat"}, {"kind": "library"}]},
            {"id": "p2", "name": "Shell", "items": [{"kind": "terminal"}]}], "on": "p1"})
    }

    #[test]
    fn a_phone_layout_is_kept_apart_from_the_page() {
        let dir = scratch("phone");
        change(&dir, Store::Desk, one(), "person").unwrap();
        let p = change(&dir, Store::Phone, phone(), "person").unwrap();
        assert_eq!(p.version, 1);
        assert_eq!(read(&dir, Store::Desk).layout, one());
        assert_eq!(read(&dir, Store::Phone).layout, phone());
        let back = undo(&dir, Store::Phone, "agent");
        assert!(back.is_err(), "nothing before the first phone change");
        assert!(outline(&p, Store::Phone).contains("workspace p2 \"Shell\": terminal"));
    }

    #[test]
    fn phone_mistakes_are_named() {
        let mut twice = phone();
        twice["spaces"][0]["items"][1] = json!({"kind": "chat"});
        assert!(check_phone(&twice).unwrap_err().contains("twice"));
        let mut empty = phone();
        empty["spaces"][1]["items"] = json!([]);
        assert!(check_phone(&empty).is_err());
        let mut home = phone();
        home["spaces"][1]["items"] = json!([{"kind": "home"}]);
        assert!(check_phone(&home).unwrap_err().contains("kind is one of"));
        let mut thread = phone();
        thread["spaces"][1]["items"] = json!([{"kind": "thread"}]);
        assert!(check_phone(&thread).unwrap_err().contains("needs `arg`"));
        // The same kind with two args is two items.
        let mut two = phone();
        two["spaces"][1]["items"] =
            json!([{"kind": "library", "arg": "page:a"}, {"kind": "library"}]);
        check_phone(&two).unwrap();
    }
}
