//! OpenCode: one SQLite database, read-only, as the seat. Parts are
//! rewritten while they stream, so a message is read only once it is done.
//! OpenCode 2 moved to `opencode.db` with one row per message, its parts in
//! the row's data; older databases, read by the import, keep a table of parts.
//! Sessions Libri runs say so in their metadata and are never read here:
//! Libri keeps those turns itself.

use super::{Event, Item, call_text, runs_memory, str_at, text_of};
use crate::memory::store::{Kind, Src};
use chrono::{DateTime, Local};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::path::Path;

pub struct Session {
    pub id: String,
    /// Each event with the creation time of its message, in ms.
    pub events: Vec<(i64, Event)>,
}

fn date(ms: i64) -> DateTime<Local> {
    DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .with_timezone(&Local)
}

pub fn open(db: &Path) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("{}: {e}", db.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Whether this is OpenCode 2's database.
fn second(conn: &Connection) -> Result<bool, String> {
    conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE name = 'session_v2'",
        [],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .map_err(|e| e.to_string())
}

/// Newest change in the database, in ms.
pub fn watermark(conn: &Connection) -> Result<i64, String> {
    let query = if second(conn)? {
        "SELECT max(t) FROM (SELECT max(time_updated) t FROM session_v2
           UNION ALL SELECT max(time_updated) FROM session_message)"
    } else {
        "SELECT max(t) FROM (SELECT max(time_updated) t FROM session
           UNION ALL SELECT max(time_updated) FROM message
           UNION ALL SELECT max(time_updated) FROM part)"
    };
    conn.query_row(query, [], |r| r.get::<_, Option<i64>>(0))
        .map(|t| t.unwrap_or(0))
        .map_err(|e| e.to_string())
}

/// Sessions with any row changed after `since` (ms), each read whole.
pub fn changed(conn: &Connection, since: i64) -> Result<Vec<Session>, String> {
    let err = |e: rusqlite::Error| e.to_string();
    let second = second(conn)?;
    let mut ids = conn
        .prepare(if second {
            "SELECT id FROM session_v2 WHERE id IN (
               SELECT id FROM session_v2 WHERE time_updated > ?1
               UNION SELECT session_id FROM session_message WHERE time_updated > ?1)
             AND (metadata IS NULL OR json_extract(metadata, '$.libri') IS NULL)"
        } else {
            "SELECT id FROM session WHERE time_updated > ?1
             UNION SELECT session_id FROM message WHERE time_updated > ?1
             UNION SELECT session_id FROM part WHERE time_updated > ?1"
        })
        .map_err(err)?;
    let ids: Vec<String> = ids
        .query_map([since], |r| r.get(0))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    ids.into_iter()
        .map(|id| {
            if second {
                session_two(conn, id)
            } else {
                session(conn, id)
            }
        })
        .collect()
}

/// One session of OpenCode 2's database, read whole.
fn session_two(conn: &Connection, id: String) -> Result<Session, String> {
    let err = |e: rusqlite::Error| e.to_string();
    let mut events = Vec::new();
    let head = conn.query_row(
        "SELECT title, directory, parent_id FROM session_v2 WHERE id = ?1",
        [&id],
        |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        },
    );
    let Ok((title, cwd, parent)) = head else {
        // Deleted since it changed.
        return Ok(Session { id, events });
    };
    if parent.is_some() {
        events.push((0, Event::Foreign));
    }
    events.push((
        0,
        Event::Info {
            cwd: Some(cwd),
            title,
        },
    ));
    let mut rows = conn
        .prepare(
            "SELECT id, type, time_created, data FROM session_message
             WHERE session_id = ?1 ORDER BY seq",
        )
        .map_err(err)?;
    let rows: Vec<(String, String, i64, String)> = rows
        .query_map([&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    for (mid, kind, created, data) in rows {
        let Ok(m) = serde_json::from_str::<Value>(&data) else {
            events.push((
                created,
                Event::Unparsed(format!("message {mid} is not JSON")),
            ));
            continue;
        };
        let at = date(created);
        let item = |key: String, kind, text: String, opens, call: Option<&str>| Item {
            kind,
            text,
            date: at,
            src: Src {
                h: "opencode".into(),
                s: id.clone(),
                e: key,
            },
            opens,
            call: call.map(str::to_owned),
            memory: false,
        };
        match kind.as_str() {
            "user" => {
                let text = str_at(&m, "/text").unwrap_or("");
                if !text.trim().is_empty() {
                    let it = item(mid, Kind::User, text.to_owned(), true, None);
                    events.push((created, Event::Item(it)));
                }
            }
            "assistant" => {
                if m.pointer("/time/completed").is_none() && m.get("error").is_none() {
                    // Still streaming: it and everything after wait for the
                    // next read.
                    break;
                }
                let parts = m.get("content").and_then(Value::as_array);
                for (k, p) in parts.into_iter().flatten().enumerate() {
                    let key = format!("{mid}.{k}");
                    match str_at(p, "/type") {
                        Some("text") => {
                            let t = str_at(p, "/text").unwrap_or("");
                            if !t.trim().is_empty() {
                                let it = item(key, Kind::Talk, t.to_owned(), false, None);
                                events.push((created, Event::Item(it)));
                            }
                        }
                        Some("tool") => {
                            let input = p.pointer("/state/input").unwrap_or(&Value::Null);
                            let call = str_at(p, "/id");
                            let name = str_at(p, "/name").unwrap_or("?");
                            let text = call_text(name, input);
                            let mut it = item(key.clone(), Kind::Tool, text, false, call);
                            it.memory = runs_memory(input);
                            events.push((created, Event::Item(it)));
                            let result = p
                                .pointer("/state/content")
                                .or_else(|| p.pointer("/state/error/message"));
                            if let Some(result) = result {
                                let text = text_of(result);
                                let it = item(format!("{key}.out"), Kind::Echo, text, false, call);
                                events.push((created, Event::Item(it)));
                            }
                        }
                        Some("reasoning") => {}
                        other => {
                            let what =
                                format!("assistant part {}", other.unwrap_or("without a type"));
                            events.push((created, Event::Unparsed(what)));
                        }
                    }
                }
                if str_at(&m, "/finish") != Some("tool-calls") {
                    events.push((created, Event::TurnEnd));
                }
            }
            // The harness's own: instructions, restarts, model switches.
            _ => {}
        }
    }
    Ok(Session { id, events })
}

pub fn session(conn: &Connection, id: String) -> Result<Session, String> {
    let err = |e: rusqlite::Error| e.to_string();
    let mut events = Vec::new();
    let head = conn.query_row(
        "SELECT title, directory, parent_id FROM session WHERE id = ?1",
        [&id],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        },
    );
    let Ok((title, cwd, parent)) = head else {
        // Deleted since it changed.
        return Ok(Session { id, events });
    };
    if parent.is_some() {
        events.push((0, Event::Foreign));
    }
    events.push((
        0,
        Event::Info {
            cwd: Some(cwd),
            title: Some(title),
        },
    ));
    let mut msgs = conn
        .prepare(
            "SELECT id, time_created, data FROM message WHERE session_id = ?1
             ORDER BY time_created, id",
        )
        .map_err(err)?;
    let mut parts = conn
        .prepare("SELECT id, data FROM part WHERE message_id = ?1 ORDER BY time_created, id")
        .map_err(err)?;
    let rows: Vec<(String, i64, String)> = msgs
        .query_map([&id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    for (mid, created, data) in rows {
        let Ok(m) = serde_json::from_str::<Value>(&data) else {
            events.push((
                created,
                Event::Unparsed(format!("message {mid} is not JSON")),
            ));
            continue;
        };
        let role = str_at(&m, "/role");
        let done = m.pointer("/time/completed").is_some() || m.get("error").is_some();
        if role == Some("assistant") && !done {
            // Still streaming: it and everything after wait for the next read.
            break;
        }
        let at = date(created);
        let ps: Vec<(String, String)> = parts
            .query_map([&mid], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        let item = |pid: &str, kind, text: String, opens, call: Option<&str>| Item {
            kind,
            text,
            date: at,
            src: Src {
                h: "opencode".into(),
                s: id.clone(),
                e: pid.to_owned(),
            },
            opens,
            call: call.map(str::to_owned),
            memory: false,
        };
        let mut said = Vec::new();
        let mut first = None;
        for (pid, pdata) in ps {
            let Ok(p) = serde_json::from_str::<Value>(&pdata) else {
                events.push((created, Event::Unparsed(format!("part {pid} is not JSON"))));
                continue;
            };
            if p.get("synthetic") == Some(&Value::Bool(true)) {
                continue;
            }
            match (role, str_at(&p, "/type")) {
                (Some("user"), Some("text")) => {
                    first.get_or_insert(pid.clone());
                    said.push(str_at(&p, "/text").unwrap_or("").to_owned());
                }
                (Some("user"), Some("file")) => {
                    first.get_or_insert(pid.clone());
                    said.push(format!("[file] {}", str_at(&p, "/filename").unwrap_or("")));
                }
                (Some("assistant"), Some("text")) => {
                    let t = str_at(&p, "/text").unwrap_or("");
                    if !t.trim().is_empty() {
                        let it = item(&pid, Kind::Talk, t.to_owned(), false, None);
                        events.push((created, Event::Item(it)));
                    }
                }
                (Some("assistant"), Some("tool")) => {
                    let input = p.pointer("/state/input").unwrap_or(&Value::Null);
                    let call = str_at(&p, "/callID");
                    let mut it = item(
                        &pid,
                        Kind::Tool,
                        call_text(str_at(&p, "/tool").unwrap_or("?"), input),
                        false,
                        call,
                    );
                    it.memory = runs_memory(input);
                    events.push((created, Event::Item(it)));
                    let result = p
                        .pointer("/state/output")
                        .or_else(|| p.pointer("/state/error"));
                    if let Some(result) = result {
                        let it = item(
                            &format!("{pid}.out"),
                            Kind::Echo,
                            text_of(result),
                            false,
                            call,
                        );
                        events.push((created, Event::Item(it)));
                    }
                }
                (
                    _,
                    Some(
                        "reasoning" | "step-start" | "step-finish" | "snapshot" | "patch"
                        | "compaction" | "agent" | "retry",
                    ),
                ) => {}
                (_, other) => {
                    let what = format!(
                        "{} part {}",
                        role.unwrap_or("?"),
                        other.unwrap_or("without a type")
                    );
                    events.push((created, Event::Unparsed(what)));
                }
            }
        }
        if let Some(pid) = first {
            let text = said.join("\n");
            if !text.trim().is_empty() {
                let it = item(&pid, Kind::User, text, true, None);
                events.push((created, Event::Item(it)));
            }
        }
        if role == Some("assistant") && str_at(&m, "/finish") != Some("tool-calls") {
            events.push((created, Event::TurnEnd));
        }
    }
    Ok(Session { id, events })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE session_v2 (id TEXT, parent_id TEXT, directory TEXT, title TEXT,
                metadata TEXT, time_updated INTEGER);
             CREATE TABLE session_message (id TEXT, session_id TEXT, type TEXT, seq INTEGER,
                time_created INTEGER, time_updated INTEGER, data TEXT);
             INSERT INTO session_v2 VALUES ('s1', NULL, '/w', 'Work', NULL, 10);
             INSERT INTO session_v2 VALUES ('s2', NULL, '/w', 'Turn', '{\"libri\":true}', 10);
             INSERT INTO session_message VALUES ('m1', 's1', 'user', 1, 1, 5, '{\"text\":\"hello\"}');
             INSERT INTO session_message VALUES ('m2', 's1', 'system', 2, 2, 5, '{\"text\":\"catalog\"}');
             INSERT INTO session_message VALUES ('m3', 's1', 'assistant', 3, 3, 5, '{\"time\":{\"completed\":4},
               \"finish\":\"stop\",\"content\":[
               {\"type\":\"reasoning\",\"text\":\"hm\"},
               {\"type\":\"tool\",\"id\":\"c1\",\"name\":\"shell\",\"state\":{\"input\":{\"command\":\"ls\"},
                 \"content\":[{\"type\":\"text\",\"text\":\"a.txt\"}]}},
               {\"type\":\"text\",\"text\":\"one file\"}]}');
             INSERT INTO session_message VALUES ('m4', 's1', 'assistant', 4, 6, 7, '{\"time\":{},\"content\":[]}');",
        )
        .unwrap();
        c
    }

    #[test]
    fn a_second_database_reads_as_the_first_and_skips_libris_own_sessions() {
        let c = database();
        assert_eq!(watermark(&c).unwrap(), 10);
        let all = changed(&c, 0).unwrap();
        assert_eq!(all.len(), 1, "Libri's own session is not followed");
        let items: Vec<(Kind, String)> = all[0]
            .events
            .iter()
            .filter_map(|(_, e)| match e {
                Event::Item(i) => Some((i.kind, i.text.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            items,
            vec![
                (Kind::User, "hello".to_owned()),
                (Kind::Tool, "shell {\"command\":\"ls\"}".to_owned()),
                (Kind::Echo, "a.txt".to_owned()),
                (Kind::Talk, "one file".to_owned()),
            ]
        );
        assert!(
            all[0]
                .events
                .iter()
                .any(|(_, e)| matches!(e, Event::TurnEnd))
        );
    }
}
