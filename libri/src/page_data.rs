//! What an interactive page keeps: one JSON document per page, which the
//! page (by MCP Apps messages, through the site) and the agent (through its
//! tools) both read and change. A page's todo list is one, its checks the
//! person's and the agent's alike.
//!
//! A change is a JSON Merge Patch (RFC 7396), applied in one write
//! transaction, so two edits to different parts of a page (the person
//! checks one item while the agent checks another) both land; a whole
//! document sent back would lose the other's. Arrays are replaced whole, as
//! the RFC has it, so a page that both sides edit keeps its items in an
//! object keyed by id.
//!
//! The person's edits reach the agent at its next turn: each row keeps the
//! version the agent last saw (its own change, or a read), and the turn
//! takes what the person changed since, as a page that changes while the
//! agent is away would otherwise go unnoticed until it looked.
//!
//! A page's data goes with the page: once its folder holds no index.html,
//! `prune` drops the row.
//!
//! Each change bumps the page's version. A watcher waits for a version past
//! the one it has: the tool server that the agent's change comes through is
//! a process of its own, so the wait reads the database again on a short
//! beat rather than hearing of it in memory.

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::Path;
use std::time::{Duration, Instant};

/// The database, in the state folder: the pages folder is served as a
/// site, and a page's data is not for whoever reaches it.
const FILE: &str = "pages.db";

/// The most a page may keep, as its JSON text.
pub const LARGEST: usize = 1 << 20;

/// How often a watcher looks again.
const BEAT: Duration = Duration::from_millis(250);

/// How long one watch waits before answering with no change, held under
/// the browser card's long poll and the shortest harness tool limit.
pub const WAIT: Duration = Duration::from_secs(25);

const SCHEMA: &str = "create table if not exists page_data (
    page text primary key,
    version integer not null,
    data text not null,
    changed integer not null,
    by text not null,
    seen integer not null default 0
)";

/// A page's data as it stands.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Data {
    /// 0 for a page that has kept nothing yet.
    pub version: u64,
    /// `{}` for a page that has kept nothing yet.
    pub data: Value,
    /// Who made the latest change: `person` or `agent`.
    pub by: Option<String>,
}

fn open(state: &Path) -> Result<Connection, String> {
    let db = Connection::open(state.join(FILE)).map_err(|e| format!("pages: {e}"))?;
    db.busy_timeout(Duration::from_secs(10))
        .map_err(|e| format!("pages: {e}"))?;
    db.execute_batch(SCHEMA)
        .map_err(|e| format!("pages: {e}"))?;
    Ok(db)
}

fn row(db: &Connection, page: &str) -> rusqlite::Result<Data> {
    let got = db
        .query_row(
            "select version, data, by from page_data where page = ?1",
            [page],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    Ok(match got {
        Some((version, data, by)) => Data {
            version: version as u64,
            data: serde_json::from_str(&data).unwrap_or_else(|_| Value::Object(Map::new())),
            by: Some(by),
        },
        None => Data {
            version: 0,
            data: Value::Object(Map::new()),
            by: None,
        },
    })
}

/// The page's data now.
pub fn read(state: &Path, page: &str) -> Result<Data, String> {
    row(&open(state)?, page).map_err(|e| format!("pages: {e}"))
}

/// Applies `patch` to the page's data, as `by` asks, and gives what it
/// became.
pub fn change(state: &Path, page: &str, patch: &Value, by: &str) -> Result<Data, String> {
    let mut db = open(state)?;
    let t = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| format!("pages: {e}"))?;
    let mut now = row(&t, page).map_err(|e| format!("pages: {e}"))?;
    merge(&mut now.data, patch);
    if !now.data.is_object() {
        return Err("A page's data is a JSON object.".into());
    }
    let text = now.data.to_string();
    if text.len() > LARGEST {
        return Err(format!(
            "A page keeps at most {} KB; this would be {} KB.",
            LARGEST / 1024,
            text.len() / 1024
        ));
    }
    now.version += 1;
    now.by = Some(by.to_owned());
    t.execute(
        "insert into page_data (page, version, data, changed, by, seen)
         values (?1, ?2, ?3, unixepoch(), ?4, case when ?4 = 'agent' then ?2 else 0 end)
         on conflict (page) do update set
           version = excluded.version, data = excluded.data,
           changed = excluded.changed, by = excluded.by,
           seen = case when excluded.by = 'agent' then excluded.version
                       else page_data.seen end",
        rusqlite::params![page, now.version as i64, text, by],
    )
    .map_err(|e| format!("pages: {e}"))?;
    t.commit().map_err(|e| format!("pages: {e}"))?;
    Ok(now)
}

/// Notes that the agent has seen the page as of `version`.
pub fn seen(state: &Path, page: &str, version: u64) -> Result<(), String> {
    open(state)?
        .execute(
            "update page_data set seen = max(seen, ?2) where page = ?1",
            rusqlite::params![page, version as i64],
        )
        .map(drop)
        .map_err(|e| format!("pages: {e}"))
}

/// The pages the person changed since the agent last saw them, by name,
/// each marked seen as it is given: the turn that takes them is told.
pub fn unseen(state: &Path) -> Result<Vec<(String, Data)>, String> {
    if !state.join(FILE).exists() {
        return Ok(Vec::new());
    }
    let mut db = open(state)?;
    let t = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| format!("pages: {e}"))?;
    let names: Vec<String> = {
        let mut q = t
            .prepare("select page from page_data where version > seen order by page")
            .map_err(|e| format!("pages: {e}"))?;
        q.query_map([], |r| r.get(0))
            .and_then(Iterator::collect)
            .map_err(|e| format!("pages: {e}"))?
    };
    let mut got = Vec::new();
    for name in names {
        let d = row(&t, &name).map_err(|e| format!("pages: {e}"))?;
        got.push((name, d));
    }
    t.execute("update page_data set seen = version", [])
        .map_err(|e| format!("pages: {e}"))?;
    t.commit().map_err(|e| format!("pages: {e}"))?;
    Ok(got)
}

/// Drops the data of every page whose folder in `pages` no longer holds
/// a page. Gives how many went.
pub fn prune(state: &Path, pages: &Path) -> Result<usize, String> {
    if !state.join(FILE).exists() {
        return Ok(0);
    }
    let db = open(state)?;
    let names: Vec<String> = {
        let mut q = db
            .prepare("select page from page_data")
            .map_err(|e| format!("pages: {e}"))?;
        q.query_map([], |r| r.get(0))
            .and_then(Iterator::collect)
            .map_err(|e| format!("pages: {e}"))?
    };
    let mut gone = 0;
    for name in names {
        if !pages.join(&name).join("index.html").is_file() {
            gone += db
                .execute("delete from page_data where page = ?1", [&name])
                .map_err(|e| format!("pages: {e}"))?;
        }
    }
    Ok(gone)
}

/// The page's data once its version is past `since`, or as it stands after
/// `limit` with none.
pub async fn wait(state: &Path, page: &str, since: u64, limit: Duration) -> Result<Data, String> {
    let until = Instant::now() + limit;
    loop {
        let (s, p) = (state.to_owned(), page.to_owned());
        let now = tokio::task::spawn_blocking(move || read(&s, &p))
            .await
            .map_err(|e| e.to_string())??;
        if now.version != since || Instant::now() >= until {
            return Ok(now);
        }
        tokio::time::sleep(BEAT).await;
    }
}

/// RFC 7396: an object patch merges key by key, a `null` removes its key,
/// and anything else replaces the target whole.
pub fn merge(target: &mut Value, patch: &Value) {
    let Value::Object(changes) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    let Value::Object(into) = target else {
        unreachable!()
    };
    for (k, v) in changes {
        if v.is_null() {
            into.remove(k);
        } else {
            merge(into.entry(k.clone()).or_insert(Value::Null), v);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("libri-page-data-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn merge_follows_rfc_7396() {
        // The RFC's own example (section 3).
        let mut doc = json!({"title": "Goodbye!", "author": {"givenName": "John", "familyName": "Doe"}, "tags": ["example", "sample"], "content": "This will be unchanged"});
        merge(
            &mut doc,
            &json!({"title": "Hello!", "phoneNumber": "+01-123-456-7890", "author": {"familyName": null}, "tags": ["example"]}),
        );
        assert_eq!(
            doc,
            json!({"title": "Hello!", "author": {"givenName": "John"}, "tags": ["example"], "content": "This will be unchanged", "phoneNumber": "+01-123-456-7890"})
        );
    }

    #[test]
    fn two_edits_to_different_items_both_land() {
        let d = dir("two_edits_to_differe");
        change(
            &d,
            "todo",
            &json!({"items": {"a": {"text": "Milk", "done": false}, "b": {"text": "Eggs", "done": false}}}),
            "agent",
        )
        .unwrap();
        change(
            &d,
            "todo",
            &json!({"items": {"a": {"done": true}}}),
            "person",
        )
        .unwrap();
        let now = change(
            &d,
            "todo",
            &json!({"items": {"b": {"done": true}}}),
            "agent",
        )
        .unwrap();
        assert_eq!(now.version, 3);
        assert_eq!(now.by.as_deref(), Some("agent"));
        assert_eq!(
            read(&d, "todo").unwrap().data,
            json!({"items": {"a": {"text": "Milk", "done": true}, "b": {"text": "Eggs", "done": true}}})
        );
    }

    #[test]
    fn a_new_page_has_kept_nothing() {
        let d = dir("a_new_page_has_kept_");
        let now = read(&d, "plan").unwrap();
        assert_eq!((now.version, now.data, now.by), (0, json!({}), None));
    }

    #[test]
    fn data_stays_an_object_and_within_its_size() {
        let d = dir("data_stays_an_object");
        assert!(change(&d, "p", &json!([1, 2]), "agent").is_err());
        let big = "x".repeat(LARGEST);
        assert!(change(&d, "p", &json!({ "big": big }), "agent").is_err());
        assert_eq!(read(&d, "p").unwrap().version, 0);
    }

    #[tokio::test]
    async fn a_watch_answers_when_the_other_side_changes() {
        let d = dir("a_watch_answers_when");
        let state = d.clone();
        let s = state.clone();
        let other = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            tokio::task::spawn_blocking(move || change(&s, "todo", &json!({"x": 1}), "agent"))
                .await
                .unwrap()
                .unwrap();
        });
        let seen = wait(&state, "todo", 0, Duration::from_secs(5))
            .await
            .unwrap();
        other.await.unwrap();
        assert_eq!((seen.version, seen.data), (1, json!({"x": 1})));
        // Nothing new: it answers at the limit with what stands.
        let same = wait(&state, "todo", 1, Duration::from_millis(300))
            .await
            .unwrap();
        assert_eq!(same.version, 1);
    }

    #[test]
    fn the_persons_edits_are_unseen_until_a_turn_takes_them() {
        let d = dir("unseen");
        change(
            &d,
            "todo",
            &json!({"items": {"a": {"done": false}}}),
            "agent",
        )
        .unwrap();
        assert!(unseen(&d).unwrap().is_empty(), "the agent's own change");
        change(
            &d,
            "todo",
            &json!({"items": {"a": {"done": true}}}),
            "person",
        )
        .unwrap();
        let got = unseen(&d).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "todo");
        assert_eq!(got[0].1.data, json!({"items": {"a": {"done": true}}}));
        assert!(unseen(&d).unwrap().is_empty(), "told once");
        change(&d, "todo", &json!({"x": 1}), "person").unwrap();
        seen(&d, "todo", 3).unwrap();
        assert!(unseen(&d).unwrap().is_empty(), "the agent read it");
    }

    #[test]
    fn a_pages_data_goes_with_its_files() {
        let d = dir("prune");
        let pages = d.join("pages");
        std::fs::create_dir_all(pages.join("kept")).unwrap();
        std::fs::write(pages.join("kept/index.html"), "<p>").unwrap();
        change(&d, "kept", &json!({"a": 1}), "person").unwrap();
        change(&d, "gone", &json!({"a": 1}), "person").unwrap();
        assert_eq!(prune(&d, &pages).unwrap(), 1);
        assert_eq!(read(&d, "kept").unwrap().version, 1);
        assert_eq!(read(&d, "gone").unwrap().version, 0);
    }
}
