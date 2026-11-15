//! Whiteboards: boards the person draws on in a Whiteboard tile and the
//! agent writes to through its tools ("jot this down" becomes a sticky
//! note). The canvas is Excalidraw's, so a board is an Excalidraw scene: a
//! set of elements (shapes, arrows, text, pen strokes) and the images they
//! show.
//!
//! The board is kept as rows, one per element, rather than as one
//! `.excalidraw` file: the site and the agent's tool server are processes
//! of their own and both write, and each element carries Excalidraw's own
//! `version` and `versionNonce`, so concurrent edits merge element by
//! element the way Excalidraw's collaboration does (the higher version wins;
//! on a tie, the lower nonce). A whole file written back by one side would
//! lose the other's strokes. The `.excalidraw` file is what `export` gives.
//!
//! Each change bumps the board's version. A view waits for a version past
//! the one it has: the agent's change comes through another process, so
//! the wait reads the database again on a short beat, as page_data's does.
//!
//! History: before a change, the board as it stood is kept as a snapshot,
//! at most one every few minutes, so a board can be put back to how it was.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::path::Path;
use std::time::{Duration, Instant};

const FILE: &str = "boards.db";

/// How often a waiting view looks again.
const BEAT: Duration = Duration::from_millis(250);

/// How long one wait holds before answering with no change.
pub const WAIT: Duration = Duration::from_secs(25);

/// The least time between two history snapshots of a board.
const SNAPSHOT_EVERY: i64 = 300;

/// The most snapshots a board keeps; the oldest go first.
const SNAPSHOTS_KEPT: i64 = 200;

/// The most one element or one image may weigh, as JSON text: a pasted
/// photo is the heaviest thing a board holds.
const LARGEST_FILE: usize = 8 << 20;

/// The name a board the agent writes to without one gets.
pub const DEFAULT_BOARD: &str = "Notes";

const SCHEMA: &str = "
create table if not exists boards (
    id text primary key,
    name text not null,
    position integer not null,
    version integer not null default 0,
    changed integer not null,
    removed integer not null default 0
);
create table if not exists board_elements (
    board text not null,
    id text not null,
    version integer not null,
    nonce integer not null,
    body text not null,
    primary key (board, id)
);
create table if not exists board_files (
    board text not null,
    id text not null,
    body text not null,
    primary key (board, id)
);
create table if not exists board_history (
    board text not null,
    version integer not null,
    at integer not null,
    scene text not null,
    primary key (board, version)
);
create table if not exists board_ops (
    board text not null,
    version integer not null,
    op text not null,
    by text not null,
    at integer not null,
    primary key (board, version)
);
create table if not exists board_meta (
    key text primary key,
    value text not null
);
create table if not exists board_links (
    token text primary key,
    board text not null,
    role text not null,
    made integer not null,
    expires integer,
    revoked integer not null default 0
);
";

/// A board as the tabs list it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Board {
    pub id: String,
    pub name: String,
    pub version: u64,
    /// Unix seconds of the latest change.
    pub changed: i64,
}

/// A board's drawing as it stands.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Scene {
    pub version: u64,
    /// Excalidraw elements, deleted ones included (their `isDeleted` tells
    /// a view to drop them), so a delete reaches every view.
    pub elements: Vec<Value>,
    /// The images elements show, by file id.
    pub files: Map<String, Value>,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("boards: {e}")
}

fn open(state: &Path) -> Result<Connection, String> {
    let db = Connection::open(state.join(FILE)).map_err(err)?;
    db.busy_timeout(Duration::from_secs(10)).map_err(err)?;
    db.execute_batch(SCHEMA).map_err(err)?;
    Ok(db)
}

fn fresh(prefix: &str) -> String {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("the system's random numbers");
    let tail: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{prefix}{tail}")
}

fn nonce() -> i64 {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).expect("the system's random numbers");
    i64::from(u32::from_le_bytes(b) >> 1)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn board_row(r: &rusqlite::Row) -> rusqlite::Result<Board> {
    Ok(Board {
        id: r.get(0)?,
        name: r.get(1)?,
        version: r.get::<_, i64>(2)? as u64,
        changed: r.get(3)?,
    })
}

/// The boards, in their tab order.
pub fn list(state: &Path) -> Result<Vec<Board>, String> {
    if !state.join(FILE).exists() {
        return Ok(Vec::new());
    }
    let db = open(state)?;
    let mut q = db
        .prepare(
            "select id, name, version, changed from boards
             where removed = 0 order by position, changed",
        )
        .map_err(err)?;
    q.query_map([], board_row)
        .and_then(Iterator::collect)
        .map_err(err)
}

fn get(db: &Connection, id: &str) -> Result<Board, String> {
    db.query_row(
        "select id, name, version, changed from boards where id = ?1 and removed = 0",
        [id],
        board_row,
    )
    .optional()
    .map_err(err)?
    .ok_or_else(|| format!("There is no board {id}."))
}

fn clean_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("A board needs a name.".into());
    }
    Ok(n.chars().take(80).collect())
}

/// Makes a board, last in the tab order.
pub fn create(state: &Path, name: &str) -> Result<Board, String> {
    let name = clean_name(name)?;
    let db = open(state)?;
    let id = fresh("b");
    let at = now();
    db.execute(
        "insert into boards (id, name, position, changed)
         values (?1, ?2, coalesce((select max(position) + 1 from boards), 0), ?3)",
        rusqlite::params![id, name, at],
    )
    .map_err(err)?;
    get(&db, &id)
}

/// Renames a board.
pub fn rename(state: &Path, id: &str, name: &str) -> Result<Board, String> {
    let name = clean_name(name)?;
    let db = open(state)?;
    get(&db, id)?;
    db.execute(
        "update boards set name = ?2 where id = ?1",
        rusqlite::params![id, name],
    )
    .map_err(err)?;
    get(&db, id)
}

/// Puts the boards in the order given; boards not named keep their places
/// after those that are.
pub fn reorder(state: &Path, ids: &[String]) -> Result<Vec<Board>, String> {
    let mut db = open(state)?;
    let t = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    for (i, id) in ids.iter().enumerate() {
        t.execute(
            "update boards set position = ?2 where id = ?1",
            rusqlite::params![id, i as i64],
        )
        .map_err(err)?;
    }
    t.execute(
        "update boards set position = position + ?1 where id not in (select value from json_each(?2))",
        rusqlite::params![ids.len() as i64, serde_json::to_string(ids).unwrap_or_default()],
    )
    .map_err(err)?;
    t.commit().map_err(err)?;
    list(state)
}

/// Takes a board off the tabs. Its drawing and history stay, so it can be
/// brought back by hand; nothing is lost to a slip.
pub fn remove(state: &Path, id: &str) -> Result<(), String> {
    let db = open(state)?;
    get(&db, id)?;
    db.execute("update boards set removed = 1 where id = ?1", [id])
        .map_err(err)?;
    Ok(())
}

fn scene_of(db: &Connection, board: &Board) -> Result<Scene, String> {
    let elements: Vec<Value> = {
        let mut q = db
            .prepare("select body from board_elements where board = ?1 order by rowid")
            .map_err(err)?;
        q.query_map([&board.id], |r| r.get::<_, String>(0))
            .and_then(Iterator::collect::<rusqlite::Result<Vec<String>>>)
            .map_err(err)?
            .iter()
            .filter_map(|b| serde_json::from_str(b).ok())
            .collect()
    };
    let mut files = Map::new();
    let mut q = db
        .prepare("select id, body from board_files where board = ?1")
        .map_err(err)?;
    let rows = q
        .query_map([&board.id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(err)?;
    for row in rows {
        let (id, body) = row.map_err(err)?;
        if let Ok(v) = serde_json::from_str(&body) {
            files.insert(id, v);
        }
    }
    Ok(Scene {
        version: board.version,
        elements,
        files,
    })
}

/// A board's drawing.
pub fn scene(state: &Path, id: &str) -> Result<Scene, String> {
    let db = open(state)?;
    let b = get(&db, id)?;
    scene_of(&db, &b)
}

/// Whether `incoming` should replace `held`, as Excalidraw reconciles two
/// copies of one element: the higher version, and on a tie the lower nonce.
fn wins(incoming: (i64, i64), held: (i64, i64)) -> bool {
    incoming.0 > held.0 || (incoming.0 == held.0 && incoming.1 < held.1)
}

/// The most ops a board's log keeps; a view further behind than that is
/// sent the whole board instead.
const OPS_KEPT: i64 = 10_000;

/// One change to a board, as the shared-object layer carries it between a
/// person's own devices and, later, between peers (A2A extension
/// `https://libri.is/a2a/share/v1`): element upserts and deletes carrying
/// Excalidraw's `version` and `versionNonce`, and the images elements show.
/// The owner applies ops without a model turn, in a versioned log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Op {
    /// An element as it now is, new or changed.
    Upsert { element: Value },
    /// An element gone, at a version that must win over the copy held.
    Delete {
        id: String,
        version: i64,
        #[serde(rename = "versionNonce")]
        version_nonce: i64,
    },
    /// An image an element shows, by its file id; images never change.
    File { file: Value },
}

/// What a board's log has past a version: the ops, or, for a view too far
/// behind (or new), the whole board.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Since {
    Ops { version: u64, ops: Vec<Op> },
    Snapshot { version: u64, snapshot: Scene },
}

/// Turns elements and images, as the canvas has them, into ops.
pub fn ops_of(elements: &[Value], files: &Map<String, Value>) -> Vec<Op> {
    elements
        .iter()
        .map(|e| Op::Upsert { element: e.clone() })
        .chain(files.values().map(|f| Op::File { file: f.clone() }))
        .collect()
}

/// Applies one op inside a transaction; gives whether it changed the board.
fn apply_one(t: &Connection, id: &str, op: &Op) -> Result<bool, String> {
    let held = |eid: &str| -> Result<Option<(i64, i64, String)>, String> {
        t.query_row(
            "select version, nonce, body from board_elements where board = ?1 and id = ?2",
            rusqlite::params![id, eid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(err)
    };
    let put = |eid: &str, version: i64, n: i64, body: &str| {
        t.execute(
            "insert into board_elements (board, id, version, nonce, body)
             values (?1, ?2, ?3, ?4, ?5)
             on conflict (board, id) do update set
               version = excluded.version, nonce = excluded.nonce, body = excluded.body",
            rusqlite::params![id, eid, version, n, body],
        )
        .map_err(err)
    };
    match op {
        Op::Upsert { element } => {
            let Some(eid) = element["id"].as_str() else {
                return Err("An element needs an id.".into());
            };
            let version = element["version"].as_i64().unwrap_or(1);
            let n = element["versionNonce"].as_i64().unwrap_or(0);
            let body = element.to_string();
            if body.len() > LARGEST_FILE {
                return Err("An element is too large to keep.".into());
            }
            if held(eid)?.is_some_and(|h| !wins((version, n), (h.0, h.1))) {
                return Ok(false);
            }
            put(eid, version, n, &body)?;
            Ok(true)
        }
        Op::Delete {
            id: eid,
            version,
            version_nonce,
        } => {
            let Some((hv, hn, body)) = held(eid)? else {
                return Ok(false);
            };
            if !wins((*version, *version_nonce), (hv, hn)) {
                return Ok(false);
            }
            let mut e: Value = serde_json::from_str(&body).unwrap_or_else(|_| json!({"id": eid}));
            e["isDeleted"] = json!(true);
            e["version"] = json!(version);
            e["versionNonce"] = json!(version_nonce);
            put(eid, *version, *version_nonce, &e.to_string())?;
            Ok(true)
        }
        Op::File { file } => {
            let Some(fid) = file["id"].as_str() else {
                return Err("An image needs an id.".into());
            };
            let body = file.to_string();
            if body.len() > LARGEST_FILE {
                return Err("An image is too large to keep (8 MB at most).".into());
            }
            let n = t
                .execute(
                    "insert into board_files (board, id, body) values (?1, ?2, ?3)
                     on conflict (board, id) do nothing",
                    rusqlite::params![id, fid, body],
                )
                .map_err(err)?;
            Ok(n > 0)
        }
    }
}

/// Applies ops to a board as `by` (person, agent, or later a peer) sends
/// them. Each op that changes the board takes the next version and goes in
/// the log; ops that lose to what is held are dropped. Gives the board's
/// version after.
pub fn apply(state: &Path, id: &str, ops: &[Op], by: &str) -> Result<u64, String> {
    let mut db = open(state)?;
    let t = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    let board = get(&t, id)?;
    snapshot(&t, &board)?;
    let mut version = board.version;
    let at = now();
    for op in ops {
        if apply_one(&t, id, op)? {
            version += 1;
            t.execute(
                "insert into board_ops (board, version, op, by, at) values (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    id,
                    version as i64,
                    serde_json::to_string(op).map_err(err)?,
                    by,
                    at
                ],
            )
            .map_err(err)?;
        }
    }
    if version != board.version {
        t.execute(
            "update boards set version = ?2, changed = ?3 where id = ?1",
            rusqlite::params![id, version as i64, at],
        )
        .map_err(err)?;
        t.execute(
            "delete from board_ops where board = ?1 and version <= ?2",
            rusqlite::params![id, version as i64 - OPS_KEPT],
        )
        .map_err(err)?;
    }
    t.commit().map_err(err)?;
    Ok(version)
}

/// Merges elements and images into a board as the canvas has them.
#[cfg(test)]
fn merge(
    state: &Path,
    id: &str,
    elements: &[Value],
    files: &Map<String, Value>,
) -> Result<u64, String> {
    apply(state, id, &ops_of(elements, files), "person")
}

/// What the board's log has past `since`: its ops, or the whole board when
/// the log no longer reaches back that far (or `since` is 0).
// The shared-object trait's ops_since, for a peer catching up; the site's
// views wait with `wait` instead. The allow goes when the sharing layer's
// sync (layer 2 of the sharing plan) calls it.
#[allow(dead_code)]
pub fn ops_since(state: &Path, id: &str, since: u64) -> Result<Since, String> {
    let db = open(state)?;
    let b = get(&db, id)?;
    since_on(&db, &b, since)
}

fn since_on(db: &Connection, b: &Board, since: u64) -> Result<Since, String> {
    let oldest: Option<i64> = db
        .query_row(
            "select min(version) from board_ops where board = ?1",
            [&b.id],
            |r| r.get(0),
        )
        .map_err(err)?;
    let reaches = since > 0 && since <= b.version && oldest.is_none_or(|o| o as u64 <= since + 1);
    if !reaches {
        return Ok(Since::Snapshot {
            version: b.version,
            snapshot: scene_of(db, b)?,
        });
    }
    let mut q = db
        .prepare("select op from board_ops where board = ?1 and version > ?2 order by version")
        .map_err(err)?;
    let ops = q
        .query_map(rusqlite::params![b.id, since as i64], |r| {
            r.get::<_, String>(0)
        })
        .and_then(Iterator::collect::<rusqlite::Result<Vec<String>>>)
        .map_err(err)?
        .iter()
        .filter_map(|o| serde_json::from_str(o).ok())
        .collect();
    Ok(Since::Ops {
        version: b.version,
        ops,
    })
}

/// Keeps the board as it stands, if the last snapshot is old enough.
fn snapshot(db: &Connection, board: &Board) -> Result<(), String> {
    if board.version == 0 {
        return Ok(());
    }
    let last: Option<i64> = db
        .query_row(
            "select max(at) from board_history where board = ?1",
            [&board.id],
            |r| r.get(0),
        )
        .map_err(err)?;
    if last.is_some_and(|at| now() - at < SNAPSHOT_EVERY) {
        return Ok(());
    }
    let scene = scene_of(db, board)?;
    let text = json!({"elements": scene.elements, "files": scene.files}).to_string();
    db.execute(
        "insert or replace into board_history (board, version, at, scene) values (?1, ?2, ?3, ?4)",
        rusqlite::params![board.id, board.version as i64, now(), text],
    )
    .map_err(err)?;
    db.execute(
        "delete from board_history where board = ?1 and version not in
           (select version from board_history where board = ?1 order by version desc limit ?2)",
        rusqlite::params![board.id, SNAPSHOTS_KEPT],
    )
    .map_err(err)?;
    Ok(())
}

/// Waits until the board is past `since`, or `limit` has gone by; gives
/// what changed then (ops_since), or nothing for no change.
pub fn wait(state: &Path, id: &str, since: u64, limit: Duration) -> Result<Option<Since>, String> {
    let until = Instant::now() + limit;
    loop {
        let db = open(state)?;
        let b = get(&db, id)?;
        if b.version > since {
            return since_on(&db, &b, since).map(Some);
        }
        drop(db);
        if Instant::now() >= until {
            return Ok(None);
        }
        std::thread::sleep(BEAT);
    }
}

/// The board as an `.excalidraw` file, which excalidraw.com and its apps
/// open.
pub fn export(state: &Path, id: &str) -> Result<(String, Value), String> {
    let db = open(state)?;
    let b = get(&db, id)?;
    let s = scene_of(&db, &b)?;
    let live: Vec<&Value> = s
        .elements
        .iter()
        .filter(|e| !e["isDeleted"].as_bool().unwrap_or(false))
        .collect();
    Ok((
        b.name,
        json!({
            "type": "excalidraw",
            "version": 2,
            "source": "libri",
            "elements": live,
            "appState": {"viewBackgroundColor": "#ffffff", "gridSize": null},
            "files": s.files,
        }),
    ))
}

/// Notes the board the person last looked at, so "jot this down" lands
/// where they are working.
pub fn focus(state: &Path, id: &str) -> Result<(), String> {
    let db = open(state)?;
    get(&db, id)?;
    db.execute(
        "insert into board_meta (key, value) values ('focused', ?1)
         on conflict (key) do update set value = excluded.value",
        [id],
    )
    .map_err(err)?;
    Ok(())
}

/// The board the agent writes to when none is named: the one the person
/// last looked at, else the board called Notes, made if there is none.
pub fn target(state: &Path, named: Option<&str>) -> Result<Board, String> {
    let db = open(state)?;
    if let Some(n) = named.map(str::trim).filter(|n| !n.is_empty()) {
        if let Ok(b) = get(&db, n) {
            return Ok(b);
        }
        let found = db
            .query_row(
                "select id, name, version, changed from boards
                 where removed = 0 and lower(name) = lower(?1) order by position limit 1",
                [n],
                board_row,
            )
            .optional()
            .map_err(err)?;
        return found.ok_or_else(|| format!("There is no board named {n}."));
    }
    let focused: Option<String> = db
        .query_row(
            "select value from board_meta where key = 'focused'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    if let Some(b) = focused.and_then(|id| get(&db, &id).ok()) {
        return Ok(b);
    }
    let notes = db
        .query_row(
            "select id, name, version, changed from boards
             where removed = 0 and lower(name) = lower(?1) order by position limit 1",
            [DEFAULT_BOARD],
            board_row,
        )
        .optional()
        .map_err(err)?;
    drop(db);
    match notes {
        Some(b) => Ok(b),
        None => create(state, DEFAULT_BOARD),
    }
}

/// What a link lets its holder do with the board.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    View,
    Edit,
}

impl Role {
    fn word(self) -> &'static str {
        match self {
            Role::View => "view",
            Role::Edit => "edit",
        }
    }
}

/// A link that opens one board to whoever holds it, with no sign-in: the
/// share listener (site.rs `serve_shares`) serves that board and nothing
/// else to it. The token is the whole secret, so it is long and random, and
/// the person can let it lapse or take it back.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Link {
    pub token: String,
    pub board: String,
    pub role: Role,
    /// Unix seconds.
    pub made: i64,
    /// Unix seconds after which it opens nothing; None for no end.
    pub expires: Option<i64>,
}

/// 32 random bytes, as URL-safe base64: past guessing.
fn link_token() -> String {
    use base64::Engine;
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).expect("the system's random numbers");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn link_row(r: &rusqlite::Row) -> rusqlite::Result<Link> {
    let role: String = r.get(2)?;
    Ok(Link {
        token: r.get(0)?,
        board: r.get(1)?,
        role: if role == "edit" {
            Role::Edit
        } else {
            Role::View
        },
        made: r.get(3)?,
        expires: r.get(4)?,
    })
}

/// Makes a link to the board; `days` past now it stops opening, if given.
pub fn link_create(state: &Path, id: &str, role: Role, days: Option<u32>) -> Result<Link, String> {
    let db = open(state)?;
    get(&db, id)?;
    let made = now();
    let link = Link {
        token: link_token(),
        board: id.to_owned(),
        role,
        made,
        expires: days.map(|d| made + i64::from(d) * 86_400),
    };
    db.execute(
        "insert into board_links (token, board, role, made, expires) values (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![link.token, link.board, role.word(), link.made, link.expires],
    )
    .map_err(err)?;
    Ok(link)
}

/// The board's links that still open it, newest first.
pub fn links(state: &Path, id: &str) -> Result<Vec<Link>, String> {
    let db = open(state)?;
    let mut q = db
        .prepare(
            "select token, board, role, made, expires from board_links
             where board = ?1 and revoked = 0 and (expires is null or expires > ?2)
             order by made desc",
        )
        .map_err(err)?;
    q.query_map(rusqlite::params![id, now()], link_row)
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)
}

/// Takes a link back: it opens nothing from now on.
pub fn link_revoke(state: &Path, token: &str) -> Result<(), String> {
    let db = open(state)?;
    let n = db
        .execute(
            "update board_links set revoked = 1 where token = ?1",
            [token],
        )
        .map_err(err)?;
    if n == 0 {
        return Err("There is no such link.".into());
    }
    Ok(())
}

/// What a token opens: its board and role, if it is live and the board is
/// still there. The same answer for a token never made, lapsed or taken
/// back, so a holder learns nothing from trying.
pub fn link_open(state: &Path, token: &str) -> Result<(Board, Role), String> {
    let db = open(state)?;
    let link = db
        .query_row(
            "select token, board, role, made, expires from board_links
             where token = ?1 and revoked = 0 and (expires is null or expires > ?2)",
            rusqlite::params![token, now()],
            link_row,
        )
        .optional()
        .map_err(err)?
        .ok_or_else(|| "This link doesn't open anything.".to_owned())?;
    let b = get(&db, &link.board).map_err(|_| "This link doesn't open anything.".to_owned())?;
    Ok((b, link.role))
}

/// The colours a note may take, by name, as Excalidraw's own palette has
/// them for a fill.
const NOTE_COLOURS: [(&str, &str); 6] = [
    ("yellow", "#ffec99"),
    ("pink", "#ffc9c9"),
    ("green", "#b2f2bb"),
    ("blue", "#a5d8ff"),
    ("purple", "#d0bfff"),
    ("orange", "#ffd8a8"),
];

const NOTE_W: f64 = 200.0;
const NOTE_H: f64 = 160.0;
const NOTE_GAP: f64 = 24.0;
const NOTE_PAD: f64 = 12.0;
/// A line of 20px Excalifont, as the canvas spaces it (font size times its
/// line height of 1.25).
const LINE: f64 = 25.0;
/// About how many characters of 20px Excalifont fit across a note.
const NOTE_CHARS: usize = 16;

/// Text broken into lines of at most `width` characters, at spaces where it
/// can be; the person's own line breaks stay.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let mut word = word.to_owned();
            while word.chars().count() > width {
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                }
                let head: String = word.chars().take(width).collect();
                word = word.chars().skip(width).collect();
                out.push(head);
            }
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(&word);
        }
        out.push(line);
    }
    out
}

/// The fields every Excalidraw element carries, filled for one the agent
/// makes. The canvas restores anything missing on load, but a full element
/// merges and exports cleanly.
fn element(kind: &str, x: f64, y: f64, w: f64, h: f64) -> Map<String, Value> {
    let mut m = Map::new();
    let fields = json!({
        "id": fresh("a"),
        "type": kind,
        "x": x, "y": y, "width": w, "height": h,
        "angle": 0,
        "strokeColor": "#1e1e1e",
        "backgroundColor": "transparent",
        "fillStyle": "solid",
        "strokeWidth": 2,
        "strokeStyle": "solid",
        "roughness": 1,
        "opacity": 100,
        "groupIds": [],
        "frameId": null,
        "roundness": null,
        "seed": nonce(),
        "version": 1,
        "versionNonce": nonce(),
        "isDeleted": false,
        "boundElements": [],
        "updated": chrono::Utc::now().timestamp_millis(),
        "link": null,
        "locked": false,
    });
    if let Value::Object(f) = fields {
        m.extend(f);
    }
    m
}

fn text_element(
    text: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    container: Option<&str>,
) -> Map<String, Value> {
    let mut t = element("text", x, y, w, h);
    t.insert("text".into(), json!(text));
    t.insert("originalText".into(), json!(text));
    t.insert("fontSize".into(), json!(20));
    // 5: Excalifont, the hand-drawn face the canvas draws new text in.
    t.insert("fontFamily".into(), json!(5));
    t.insert("lineHeight".into(), json!(1.25));
    t.insert("autoResize".into(), json!(container.is_none()));
    t.insert(
        "textAlign".into(),
        json!(if container.is_some() {
            "center"
        } else {
            "left"
        }),
    );
    t.insert(
        "verticalAlign".into(),
        json!(if container.is_some() { "middle" } else { "top" }),
    );
    t.insert("containerId".into(), json!(container));
    t
}

fn live(elements: &[Value]) -> impl Iterator<Item = &Value> {
    elements
        .iter()
        .filter(|e| !e["isDeleted"].as_bool().unwrap_or(false))
}

/// Free room for a note: right of the newest note in its row, wrapping to
/// a new row after four, or at the origin on an empty board, clear of
/// everything already drawn.
fn free_spot(elements: &[Value]) -> (f64, f64) {
    let notes: Vec<&Value> = live(elements)
        .filter(|e| e["customData"]["note"].as_bool().unwrap_or(false))
        .collect();
    let overlaps = |x: f64, y: f64| {
        live(elements).any(|e| {
            let (ex, ey) = (
                e["x"].as_f64().unwrap_or(0.0),
                e["y"].as_f64().unwrap_or(0.0),
            );
            let (ew, eh) = (
                e["width"].as_f64().unwrap_or(0.0),
                e["height"].as_f64().unwrap_or(0.0),
            );
            x < ex + ew && ex < x + NOTE_W && y < ey + eh && ey < y + NOTE_H
        })
    };
    let (mut x, mut y) = match notes.last() {
        Some(n) => {
            let (nx, ny) = (
                n["x"].as_f64().unwrap_or(0.0),
                n["y"].as_f64().unwrap_or(0.0),
            );
            let in_row = notes
                .iter()
                .filter(|m| (m["y"].as_f64().unwrap_or(0.0) - ny).abs() < 1.0)
                .count();
            if in_row >= 4 {
                let first = notes
                    .iter()
                    .filter(|m| (m["y"].as_f64().unwrap_or(0.0) - ny).abs() < 1.0)
                    .map(|m| m["x"].as_f64().unwrap_or(0.0))
                    .fold(f64::INFINITY, f64::min);
                (first, ny + NOTE_H + NOTE_GAP)
            } else {
                (nx + NOTE_W + NOTE_GAP, ny)
            }
        }
        None => (0.0, 0.0),
    };
    // Below whatever is there, if that spot is taken.
    for _ in 0..200 {
        if !overlaps(x, y) {
            break;
        }
        y += NOTE_H + NOTE_GAP;
        if notes.is_empty() {
            x = 0.0;
        }
    }
    (x, y)
}

/// A sticky note: a filled, rounded box with the text centred in it, bound
/// together as the canvas binds a label to its box.
fn note_elements(text: &str, colour: &str, x: f64, y: f64) -> (Value, Value) {
    let fill = NOTE_COLOURS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(colour))
        .map_or(NOTE_COLOURS[0].1, |(_, c)| c);
    // The text wrapped to the note's width and centred in it, as the canvas
    // lays out a label it measured itself: it keeps a bound label where the
    // label says until someone edits it.
    let lines = wrap(text, NOTE_CHARS);
    let tall = (lines.len() as f64 * LINE).max(LINE);
    let h = NOTE_H.max(tall + 2.0 * NOTE_PAD);
    let mut b = element("rectangle", x, y, NOTE_W, h);
    let mut t = text_element(
        &lines.join("\n"),
        x + NOTE_PAD,
        y + (h - tall) / 2.0,
        NOTE_W - 2.0 * NOTE_PAD,
        tall,
        None,
    );
    t.insert("originalText".into(), json!(text));
    let (bid, tid) = (b["id"].clone(), t["id"].clone());
    b.insert("backgroundColor".into(), json!(fill));
    b.insert("strokeColor".into(), json!("transparent"));
    b.insert("roundness".into(), json!({"type": 3}));
    b.insert("boundElements".into(), json!([{"type": "text", "id": tid}]));
    b.insert("customData".into(), json!({"note": true}));
    t.insert("containerId".into(), bid);
    t.insert("autoResize".into(), json!(false));
    t.insert("textAlign".into(), json!("center"));
    t.insert("verticalAlign".into(), json!("middle"));
    (Value::Object(b), Value::Object(t))
}

/// Adds a sticky note to a board (the one named, else the one the person
/// last looked at, else Notes). Gives the board it went on.
pub fn note(
    state: &Path,
    board: Option<&str>,
    text: &str,
    colour: Option<&str>,
) -> Result<Board, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("A note needs some text.".into());
    }
    let b = target(state, board)?;
    let s = scene(state, &b.id)?;
    let (x, y) = free_spot(&s.elements);
    let (rect, label) = note_elements(text, colour.unwrap_or("yellow"), x, y);
    apply(state, &b.id, &ops_of(&[rect, label], &Map::new()), "agent")?;
    let db = open(state)?;
    get(&db, &b.id)
}

/// The board in words, for the agent: its notes, labelled shapes, loose
/// text and the arrows between them, each with its id and place.
pub fn summary(state: &Path, id: &str) -> Result<String, String> {
    let db = open(state)?;
    let b = get(&db, id)?;
    let s = scene_of(&db, &b)?;
    let els: Vec<&Value> = live(&s.elements).collect();
    let label_of = |id: &str| -> Option<String> {
        els.iter()
            .find(|e| e["type"] == "text" && e["containerId"].as_str() == Some(id))
            .and_then(|t| t["text"].as_str())
            .map(str::to_owned)
    };
    let mut lines = vec![format!(
        "Board \"{}\" ({}), version {}.",
        b.name, b.id, b.version
    )];
    let mut strokes = 0;
    for e in &els {
        let kind = e["type"].as_str().unwrap_or("");
        let eid = e["id"].as_str().unwrap_or("");
        let at = format!(
            "at {:.0},{:.0}",
            e["x"].as_f64().unwrap_or(0.0),
            e["y"].as_f64().unwrap_or(0.0)
        );
        match kind {
            "text" if e["containerId"].is_null() => lines.push(format!(
                "- text {eid} {at}: {}",
                e["text"].as_str().unwrap_or("")
            )),
            "text" => {}
            "arrow" | "line" => {
                let from = e["startBinding"]["elementId"].as_str();
                let to = e["endBinding"]["elementId"].as_str();
                let name = |x: Option<&str>| {
                    x.map(|i| label_of(i).map_or(i.to_owned(), |l| format!("\"{l}\"")))
                        .unwrap_or_else(|| "nothing".into())
                };
                let label = label_of(eid)
                    .map(|l| format!(" labelled \"{l}\""))
                    .unwrap_or_default();
                lines.push(format!(
                    "- {kind} {eid}{label} from {} to {}",
                    name(from),
                    name(to)
                ));
            }
            "freedraw" => strokes += 1,
            "image" => lines.push(format!("- image {eid} {at}")),
            _ => {
                let what = if e["customData"]["note"].as_bool().unwrap_or(false) {
                    "note".to_owned()
                } else {
                    kind.to_owned()
                };
                match label_of(eid) {
                    Some(l) => lines.push(format!("- {what} {eid} {at}: {l}")),
                    None => lines.push(format!("- {what} {eid} {at}")),
                }
            }
        }
    }
    if strokes > 0 {
        lines.push(format!("- {strokes} pen stroke(s), drawn by hand"));
    }
    if lines.len() == 1 {
        lines.push("It is empty.".into());
    }
    Ok(lines.join("\n"))
}

/// One change the agent asks of a board.
#[derive(Clone, Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Edit {
    /// A shape (rectangle, ellipse or diamond) with an optional label, or
    /// loose text.
    Add {
        kind: String,
        #[serde(default)]
        text: Option<String>,
        x: f64,
        y: f64,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        height: Option<f64>,
        /// A fill colour: yellow, pink, green, blue, purple, orange.
        #[serde(default)]
        colour: Option<String>,
    },
    /// An arrow from one element to another, by id, with an optional label.
    Arrow {
        from: String,
        to: String,
        #[serde(default)]
        text: Option<String>,
    },
    /// Moves an element (and its label) to a place.
    Move { id: String, x: f64, y: f64 },
    /// Changes the text of a note, a shape's label or loose text.
    Text { id: String, text: String },
    /// Deletes an element and its label.
    Delete { id: String },
}

fn bumped(mut e: Value) -> Value {
    let v = e["version"].as_i64().unwrap_or(1) + 1;
    e["version"] = json!(v);
    e["versionNonce"] = json!(nonce());
    e["updated"] = json!(chrono::Utc::now().timestamp_millis());
    e
}

fn centre(e: &Value) -> (f64, f64) {
    (
        e["x"].as_f64().unwrap_or(0.0) + e["width"].as_f64().unwrap_or(0.0) / 2.0,
        e["y"].as_f64().unwrap_or(0.0) + e["height"].as_f64().unwrap_or(0.0) / 2.0,
    )
}

/// Applies the agent's edits to a board, in order. Gives the ids of what
/// each made, in order, for edits that make something.
pub fn edit(state: &Path, id: &str, edits: &[Edit]) -> Result<(Board, Vec<String>), String> {
    let b = target(state, Some(id))?;
    let s = scene(state, &b.id)?;
    let mut els: Vec<Value> = s.elements;
    let mut out: Vec<Value> = Vec::new();
    let mut made = Vec::new();
    let find = |els: &[Value], eid: &str| -> Result<usize, String> {
        els.iter()
            .position(|e| e["id"] == eid && !e["isDeleted"].as_bool().unwrap_or(false))
            .ok_or_else(|| format!("There is no element {eid} on {}.", b.name))
    };
    let label_at = |els: &[Value], eid: &str| {
        els.iter().position(|e| {
            e["type"] == "text"
                && e["containerId"] == eid
                && !e["isDeleted"].as_bool().unwrap_or(false)
        })
    };
    for ed in edits {
        match ed {
            Edit::Add {
                kind,
                text,
                x,
                y,
                width,
                height,
                colour,
            } => {
                let kind = kind.to_lowercase();
                if kind == "note" {
                    let (r, t) = note_elements(
                        text.as_deref().unwrap_or(""),
                        colour.as_deref().unwrap_or("yellow"),
                        *x,
                        *y,
                    );
                    made.push(r["id"].as_str().unwrap_or("").to_owned());
                    els.push(r.clone());
                    els.push(t.clone());
                    out.extend([r, t]);
                    continue;
                }
                if kind == "text" {
                    let t = text.as_deref().unwrap_or("");
                    let lines = t.lines().count().max(1) as f64;
                    let w =
                        width.unwrap_or(t.lines().map(str::len).max().unwrap_or(1) as f64 * 11.0);
                    let e = Value::Object(text_element(
                        t,
                        *x,
                        *y,
                        w,
                        height.unwrap_or(lines * 25.0),
                        None,
                    ));
                    made.push(e["id"].as_str().unwrap_or("").to_owned());
                    els.push(e.clone());
                    out.push(e);
                    continue;
                }
                if !["rectangle", "ellipse", "diamond"].contains(&kind.as_str()) {
                    return Err(format!(
                        "A board can add note, text, rectangle, ellipse or diamond, not {kind}."
                    ));
                }
                let (w, h) = (width.unwrap_or(180.0), height.unwrap_or(100.0));
                let mut shape = element(&kind, *x, *y, w, h);
                if let Some(c) = colour {
                    let fill = NOTE_COLOURS
                        .iter()
                        .find(|(n, _)| n.eq_ignore_ascii_case(c))
                        .map_or(c.as_str(), |(_, f)| f);
                    shape.insert("backgroundColor".into(), json!(fill));
                }
                if kind == "rectangle" {
                    shape.insert("roundness".into(), json!({"type": 3}));
                }
                let sid = shape["id"].as_str().unwrap_or("").to_owned();
                if let Some(t) = text.as_deref().filter(|t| !t.is_empty()) {
                    let label =
                        text_element(t, *x + 10.0, *y + 10.0, w - 20.0, h - 20.0, Some(&sid));
                    shape.insert(
                        "boundElements".into(),
                        json!([{"type": "text", "id": label["id"]}]),
                    );
                    let (s, l) = (Value::Object(shape), Value::Object(label));
                    els.extend([s.clone(), l.clone()]);
                    out.extend([s, l]);
                } else {
                    let s = Value::Object(shape);
                    els.push(s.clone());
                    out.push(s);
                }
                made.push(sid);
            }
            Edit::Arrow { from, to, text } => {
                let (a, z) = (els[find(&els, from)?].clone(), els[find(&els, to)?].clone());
                let ((ax, ay), (zx, zy)) = (centre(&a), centre(&z));
                let mut arrow = element("arrow", ax, ay, (zx - ax).abs(), (zy - ay).abs());
                arrow.insert("points".into(), json!([[0.0, 0.0], [zx - ax, zy - ay]]));
                arrow.insert(
                    "startBinding".into(),
                    json!({"elementId": from, "focus": 0, "gap": 8}),
                );
                arrow.insert(
                    "endBinding".into(),
                    json!({"elementId": to, "focus": 0, "gap": 8}),
                );
                arrow.insert("startArrowhead".into(), Value::Null);
                arrow.insert("endArrowhead".into(), json!("arrow"));
                arrow.insert("roundness".into(), json!({"type": 2}));
                arrow.insert("elbowed".into(), json!(false));
                let aid = arrow["id"].as_str().unwrap_or("").to_owned();
                // Each end lists the arrow as bound to it, as the canvas keeps them.
                for end in [from, to] {
                    let i = find(&els, end)?;
                    let mut e = els[i].clone();
                    let mut bound = e["boundElements"].as_array().cloned().unwrap_or_default();
                    bound.push(json!({"type": "arrow", "id": aid}));
                    e["boundElements"] = json!(bound);
                    let e = bumped(e);
                    els[i] = e.clone();
                    out.push(e);
                }
                if let Some(t) = text.as_deref().filter(|t| !t.is_empty()) {
                    let (mx, my) = ((ax + zx) / 2.0, (ay + zy) / 2.0);
                    let label = text_element(t, mx, my, t.len() as f64 * 11.0, 25.0, Some(&aid));
                    arrow.insert(
                        "boundElements".into(),
                        json!([{"type": "text", "id": label["id"]}]),
                    );
                    let l = Value::Object(label);
                    els.push(l.clone());
                    out.push(l);
                }
                let a = Value::Object(arrow);
                els.push(a.clone());
                out.push(a);
                made.push(aid);
            }
            Edit::Move { id: eid, x, y } => {
                let i = find(&els, eid)?;
                let (dx, dy) = (
                    x - els[i]["x"].as_f64().unwrap_or(0.0),
                    y - els[i]["y"].as_f64().unwrap_or(0.0),
                );
                let mut e = els[i].clone();
                e["x"] = json!(x);
                e["y"] = json!(y);
                let e = bumped(e);
                els[i] = e.clone();
                out.push(e);
                if let Some(j) = label_at(&els, eid) {
                    let mut l = els[j].clone();
                    l["x"] = json!(l["x"].as_f64().unwrap_or(0.0) + dx);
                    l["y"] = json!(l["y"].as_f64().unwrap_or(0.0) + dy);
                    let l = bumped(l);
                    els[j] = l.clone();
                    out.push(l);
                }
            }
            Edit::Text { id: eid, text } => {
                let i = find(&els, eid)?;
                let j = if els[i]["type"] == "text" {
                    Some(i)
                } else {
                    label_at(&els, eid)
                };
                let Some(j) = j else {
                    return Err(format!("{eid} has no text to change; add one with add."));
                };
                let mut t = els[j].clone();
                t["text"] = json!(text);
                t["originalText"] = json!(text);
                let t = bumped(t);
                els[j] = t.clone();
                out.push(t);
            }
            Edit::Delete { id: eid } => {
                let i = find(&els, eid)?;
                let mut gone = vec![i];
                gone.extend(label_at(&els, eid));
                for k in gone {
                    let mut e = els[k].clone();
                    e["isDeleted"] = json!(true);
                    let e = bumped(e);
                    els[k] = e.clone();
                    out.push(e);
                }
            }
        }
    }
    apply(state, &b.id, &ops_of(&out, &Map::new()), "agent")?;
    let db = open(state)?;
    Ok((get(&db, &b.id)?, made))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> tempfile_dir::Dir {
        tempfile_dir::Dir::new()
    }

    mod tempfile_dir {
        pub struct Dir(pub std::path::PathBuf);
        impl Dir {
            pub fn new() -> Self {
                let mut b = [0u8; 8];
                getrandom::fill(&mut b).unwrap();
                let p = std::env::temp_dir().join(format!(
                    "libri-boards-{}",
                    b.iter().map(|x| format!("{x:02x}")).collect::<String>()
                ));
                std::fs::create_dir_all(&p).unwrap();
                Dir(p)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn boards_keep_their_tab_order_and_names() {
        let d = state();
        let a = create(&d.0, "Plans").unwrap();
        let b = create(&d.0, "House").unwrap();
        assert_eq!(
            list(&d.0)
                .unwrap()
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>(),
            ["Plans", "House"]
        );
        reorder(&d.0, &[b.id.clone(), a.id.clone()]).unwrap();
        assert_eq!(list(&d.0).unwrap()[0].id, b.id);
        rename(&d.0, &a.id, " Launch ").unwrap();
        remove(&d.0, &b.id).unwrap();
        let left = list(&d.0).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].name, "Launch");
    }

    #[test]
    fn the_higher_version_of_an_element_wins_and_a_stale_one_is_dropped() {
        let d = state();
        let b = create(&d.0, "B").unwrap();
        let e = |v: i64, n: i64, x: f64| json!({"id": "e1", "type": "rectangle", "version": v, "versionNonce": n, "x": x});
        assert_eq!(
            merge(&d.0, &b.id, &[e(2, 5, 10.0)], &Map::new()).unwrap(),
            1
        );
        // A stale copy changes nothing, and the version stays.
        assert_eq!(
            merge(&d.0, &b.id, &[e(1, 1, 99.0)], &Map::new()).unwrap(),
            1
        );
        // On a tie the lower nonce wins.
        assert_eq!(
            merge(&d.0, &b.id, &[e(2, 3, 20.0)], &Map::new()).unwrap(),
            2
        );
        // A delete that wins marks it gone, and the log keeps both changes.
        let gone = Op::Delete {
            id: "e1".into(),
            version: 3,
            version_nonce: 9,
        };
        assert_eq!(apply(&d.0, &b.id, &[gone], "person").unwrap(), 3);
        let Since::Ops { ops, .. } = ops_since(&d.0, &b.id, 1).unwrap() else {
            panic!("the log reaches back to version 1");
        };
        assert_eq!(ops.len(), 2);
        let s = scene(&d.0, &b.id).unwrap();
        assert_eq!(s.elements.len(), 1);
        assert_eq!(s.elements[0]["x"], 20.0);
        assert_eq!(s.elements[0]["isDeleted"], true);
    }

    #[test]
    fn a_note_lands_on_the_focused_board_else_on_notes() {
        let d = state();
        let made = note(&d.0, None, "buy milk", None).unwrap();
        assert_eq!(made.name, DEFAULT_BOARD);
        let other = create(&d.0, "House").unwrap();
        focus(&d.0, &other.id).unwrap();
        let next = note(&d.0, None, "fix the gate", Some("green")).unwrap();
        assert_eq!(next.id, other.id);
        let s = scene(&d.0, &other.id).unwrap();
        let rect = s
            .elements
            .iter()
            .find(|e| e["type"] == "rectangle")
            .unwrap();
        assert_eq!(rect["backgroundColor"], "#b2f2bb");
        let text = s.elements.iter().find(|e| e["type"] == "text").unwrap();
        assert_eq!(text["containerId"], rect["id"]);
        assert_eq!(text["text"], "fix the gate");
        assert!(summary(&d.0, &other.id).unwrap().contains("note"));
    }

    #[test]
    fn a_link_opens_its_board_until_taken_back() {
        let d = state();
        let b = create(&d.0, "Plans").unwrap();
        let view = link_create(&d.0, &b.id, Role::View, None).unwrap();
        let edit = link_create(&d.0, &b.id, Role::Edit, Some(7)).unwrap();
        assert!(view.token.len() >= 43, "a token past guessing");
        assert_ne!(view.token, edit.token);
        assert_eq!(
            link_open(&d.0, &view.token).unwrap(),
            (b.clone(), Role::View)
        );
        assert_eq!(link_open(&d.0, &edit.token).unwrap().1, Role::Edit);
        assert_eq!(edit.expires, Some(edit.made + 7 * 86_400));
        assert_eq!(links(&d.0, &b.id).unwrap().len(), 2);
        link_revoke(&d.0, &view.token).unwrap();
        assert!(link_open(&d.0, &view.token).is_err());
        assert_eq!(links(&d.0, &b.id).unwrap().len(), 1);
        assert!(link_open(&d.0, "made-up").is_err());
        assert!(link_revoke(&d.0, "made-up").is_err());
    }

    #[test]
    fn a_lapsed_link_or_a_removed_board_opens_nothing() {
        let d = state();
        let b = create(&d.0, "Old").unwrap();
        let l = link_create(&d.0, &b.id, Role::View, None).unwrap();
        let db = open(&d.0).unwrap();
        db.execute("update board_links set expires = ?1", [now() - 1])
            .unwrap();
        assert!(link_open(&d.0, &l.token).is_err());
        assert!(links(&d.0, &b.id).unwrap().is_empty());
        let l2 = link_create(&d.0, &b.id, Role::Edit, None).unwrap();
        remove(&d.0, &b.id).unwrap();
        assert!(link_open(&d.0, &l2.token).is_err());
    }

    #[test]
    fn a_note_wraps_its_words_and_sits_in_the_middle() {
        assert_eq!(
            wrap("Call the plumber about the kitchen sink", 16),
            ["Call the plumber", "about the", "kitchen sink"]
        );
        assert_eq!(wrap("a\nb", 16), ["a", "b"]);
        let (r, t) = note_elements(
            "Call the plumber about the kitchen sink",
            "yellow",
            0.0,
            0.0,
        );
        let mid = |e: &Value| e["y"].as_f64().unwrap() + e["height"].as_f64().unwrap() / 2.0;
        assert!((mid(&r) - mid(&t)).abs() < 0.01);
        assert_eq!(t["originalText"], "Call the plumber about the kitchen sink");
    }

    #[test]
    fn notes_line_up_in_rows_without_overlapping() {
        let d = state();
        let b = create(&d.0, "N").unwrap();
        for i in 0..6 {
            note(&d.0, Some(&b.id), &format!("n{i}"), None).unwrap();
        }
        let s = scene(&d.0, &b.id).unwrap();
        let rects: Vec<(f64, f64)> = s
            .elements
            .iter()
            .filter(|e| e["type"] == "rectangle")
            .map(|e| (e["x"].as_f64().unwrap(), e["y"].as_f64().unwrap()))
            .collect();
        assert_eq!(rects.len(), 6);
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(
                    (a.0 - b.0).abs() >= NOTE_W || (a.1 - b.1).abs() >= NOTE_H,
                    "{a:?} overlaps {b:?}"
                );
            }
        }
        // The fifth wraps to a second row.
        assert!(rects[4].1 > rects[0].1);
    }

    #[test]
    fn edits_add_link_move_relabel_and_delete() {
        let d = state();
        let b = create(&d.0, "Plan").unwrap();
        let (_, ids) = edit(
            &d.0,
            &b.id,
            &[
                Edit::Add {
                    kind: "rectangle".into(),
                    text: Some("Design".into()),
                    x: 0.0,
                    y: 0.0,
                    width: None,
                    height: None,
                    colour: None,
                },
                Edit::Add {
                    kind: "note".into(),
                    text: Some("Ship".into()),
                    x: 400.0,
                    y: 0.0,
                    width: None,
                    height: None,
                    colour: Some("pink".into()),
                },
            ],
        )
        .unwrap();
        let (_, arrow) = edit(
            &d.0,
            &b.id,
            &[Edit::Arrow {
                from: ids[0].clone(),
                to: ids[1].clone(),
                text: Some("then".into()),
            }],
        )
        .unwrap();
        let said = summary(&d.0, &b.id).unwrap();
        assert!(said.contains("from \"Design\" to \"Ship\""), "{said}");
        edit(
            &d.0,
            &b.id,
            &[
                Edit::Move {
                    id: ids[0].clone(),
                    x: 50.0,
                    y: 60.0,
                },
                Edit::Text {
                    id: ids[1].clone(),
                    text: "Ship it".into(),
                },
            ],
        )
        .unwrap();
        let said = summary(&d.0, &b.id).unwrap();
        assert!(
            said.contains("at 50,60: Design") && said.contains("Ship it"),
            "{said}"
        );
        edit(
            &d.0,
            &b.id,
            &[Edit::Delete {
                id: arrow[0].clone(),
            }],
        )
        .unwrap();
        assert!(!summary(&d.0, &b.id).unwrap().contains("arrow"));
        let (_, file) = export(&d.0, &b.id).unwrap();
        assert_eq!(file["type"], "excalidraw");
    }

    #[test]
    fn a_wait_answers_on_a_change_and_holds_without_one() {
        let d = state();
        let b = create(&d.0, "W").unwrap();
        assert!(
            wait(&d.0, &b.id, 0, Duration::from_millis(50))
                .unwrap()
                .is_none()
        );
        note(&d.0, Some(&b.id), "hi", None).unwrap();
        let Some(Since::Snapshot { version, .. }) =
            wait(&d.0, &b.id, 0, Duration::from_millis(50)).unwrap()
        else {
            panic!("a first wait gets the whole board");
        };
        assert_eq!(version, 2);
        note(&d.0, Some(&b.id), "again", None).unwrap();
        let Some(Since::Ops { version, ops }) =
            wait(&d.0, &b.id, 2, Duration::from_millis(50)).unwrap()
        else {
            panic!("a view one step behind gets the ops");
        };
        assert_eq!((version, ops.len()), (4, 2));
    }
}
