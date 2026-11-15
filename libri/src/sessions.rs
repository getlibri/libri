//! Each chat's own harness session, kept from turn to turn as Paseo and T3
//! Code keep theirs: the main chat and each worker hold one session per
//! harness, which every turn resumes, so the harness keeps the thread, its
//! prompt cache and its own compaction, and a restart of Libri loses
//! nothing of it. Memory older than the thread is hippo2's, an MCP server
//! beside it (servers.rs).
//!
//! The record is a file per chat, `sessions/<chat>.json` in the state
//! directory, `{"<harness>": "<session id>", "last": "<harness>"}`: a
//! program reading the harnesses' logs (hippo2) can tell Libri's sessions by
//! it. `last` names the harness that ran the chat last: a session on any
//! other misses the turns since, so the chat starts that harness anew.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// The folder of the records, in the state directory.
const DIR: &str = "sessions";

/// The main chat's key.
pub const MAIN: &str = "main";

/// Worker `name`'s key.
pub fn worker(name: &str) -> String {
    format!("worker-{name}")
}

fn file(state: &Path, chat: &str) -> Option<PathBuf> {
    let known = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    (!chat.is_empty() && chat.chars().all(known))
        .then(|| state.join(DIR).join(format!("{chat}.json")))
}

fn read(state: &Path, chat: &str) -> Map<String, Value> {
    file(state, chat)
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn write(state: &Path, chat: &str, map: &Map<String, Value>) -> Result<(), String> {
    let f = file(state, chat).ok_or_else(|| format!("no chat named {chat:?}"))?;
    if let Some(dir) = f.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    crate::task::put(&f, &Value::Object(map.clone()).to_string())
}

/// The session `chat` keeps on `harness`, if it has one.
pub fn get(state: &Path, chat: &str, harness: &str) -> Option<String> {
    read(state, chat)
        .get(harness)
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

/// The key of the harness that ran the chat last.
const LAST: &str = "last";

/// Records `id` as `chat`'s session on `harness`, the one that runs it now.
pub fn put(state: &Path, chat: &str, harness: &str, id: &str) -> Result<(), String> {
    let mut map = read(state, chat);
    if map.get(harness).and_then(Value::as_str) == Some(id)
        && map.get(LAST).and_then(Value::as_str) == Some(harness)
    {
        return Ok(());
    }
    map.insert(harness.into(), id.into());
    map.insert(LAST.into(), harness.into());
    write(state, chat, &map)
}

/// Notes that `harness` runs `chat` now, for a harness whose sessions are
/// files in a folder of the chat's rather than an id (`folder`).
pub fn ran(state: &Path, chat: &str, harness: &str) -> Result<(), String> {
    let mut map = read(state, chat);
    if map.get(LAST).and_then(Value::as_str) == Some(harness) {
        return Ok(());
    }
    map.insert(LAST.into(), harness.into());
    write(state, chat, &map)
}

/// Whether `chat`'s session on `harness` holds its newest turns: none ran
/// on another harness since. A chat from before the record kept `last` has
/// one harness's session at most.
pub fn current(state: &Path, chat: &str, harness: &str) -> bool {
    read(state, chat)
        .get(LAST)
        .and_then(Value::as_str)
        .is_none_or(|last| last == harness)
}

/// What `chat`'s session on `harness` was made with, for a harness that
/// fixes some settings at a session's start.
pub fn made(state: &Path, chat: &str, harness: &str) -> Option<String> {
    get(state, chat, &format!("{harness}.made"))
}

/// Records `id` as the session `chat` is starting on `harness`, before the
/// harness has answered (`<harness>.next`): every id in the record is
/// Libri's to a reader of the harnesses' logs, and the chat goes on with
/// it only once `put` has made it its own.
pub fn put_next(state: &Path, chat: &str, harness: &str, id: &str) -> Result<(), String> {
    let mut map = read(state, chat);
    map.insert(format!("{harness}.next"), id.into());
    write(state, chat, &map)
}

pub fn put_made(state: &Path, chat: &str, harness: &str, made: &str) -> Result<(), String> {
    let mut map = read(state, chat);
    map.insert(format!("{harness}.made"), made.into());
    write(state, chat, &map)
}

/// Drops `chat`'s session on `harness`: the harness lost it, and the next
/// turn starts another.
pub fn forget(state: &Path, chat: &str, harness: &str) -> Result<(), String> {
    let mut map = read(state, chat);
    if map.remove(harness).is_none() {
        return Ok(());
    }
    write(state, chat, &map)
}

/// Starts `chat` afresh on every harness (`/clear`): the next turn opens a
/// new session, seeded as any new one is. The ids it held stay in the record
/// as `<key>.was.<seconds>`, so a reader of the harnesses' logs (hippo2,
/// which takes every id in the record for Libri's) still leaves those
/// sessions to Libri's own record rather than filing them twice; the session
/// folders of a harness that keeps its own (omp, Pi) go, or `--continue`
/// would find them.
pub fn clear(state: &Path, chat: &str) -> Result<(), String> {
    let Some(f) = file(state, chat) else {
        return Err(format!("no chat named {chat:?}"));
    };
    let when = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (old, current): (Map<String, Value>, Map<String, Value>) = read(state, chat)
        .into_iter()
        .partition(|(key, _)| key.contains(".was."));
    let mut kept = old;
    for (key, value) in current {
        if key == LAST || key.ends_with(".made") || !value.is_string() {
            continue;
        }
        // Two clears in a second each keep theirs.
        let name = (0..)
            .map(|n| match n {
                0 => format!("{key}.was.{when}"),
                n => format!("{key}.was.{when}-{n}"),
            })
            .find(|k| !kept.contains_key(k))
            .unwrap_or_default();
        kept.insert(name, value);
    }
    if f.exists() {
        write(state, chat, &kept)?;
    }
    let own = format!("{chat}.");
    let dir = f.parent().unwrap_or(state);
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() && name.to_string_lossy().starts_with(&own) {
            std::fs::remove_dir_all(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// A folder of `chat`'s own for `harness`'s session files, for a harness
/// that names its sessions itself and goes on with the newest in a folder
/// (omp's `--session-dir` and `--continue`).
pub fn folder(state: &Path, chat: &str, harness: &str) -> Option<PathBuf> {
    file(state, chat).map(|f| {
        f.with_extension("")
            .with_file_name(format!("{chat}.{harness}"))
    })
}

/// Whether `dir` holds a session file to go on with.
pub fn holds_session(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
}

/// The state directory, where the records are; None outside a daemon.
/// How large a chat's session grows before its harness compacts it, in
/// tokens of context: the person's pick in Settings, else the deployment's
/// `compact.at`, else [`COMPACT_DEFAULT`]. A session the harness would let
/// grow to its model's whole window costs every turn that much again.
pub const COMPACT_AT: crate::model::Kept = crate::model::Kept("compact-at");

/// What a host compacts at when neither the person nor the deployment says.
pub const COMPACT_DEFAULT: u64 = 200_000;

/// The smallest pick taken: below it a harness would compact every turn.
pub const COMPACT_LEAST: u64 = 20_000;

/// The deployment's own figure (LIBRI_COMPACT_AT), when it names one.
fn compact_deployed() -> Option<u64> {
    std::env::var("LIBRI_COMPACT_AT").ok()?.trim().parse().ok()
}

/// The compaction point in force on `state`.
pub fn compact_at(state: &Path) -> u64 {
    COMPACT_AT
        .read(state)
        .and_then(|v| v.parse().ok())
        .or_else(compact_deployed)
        .unwrap_or(COMPACT_DEFAULT)
}

/// The compaction point for a turn, read where the harness is started.
pub fn compact_now() -> u64 {
    state().map_or_else(
        || compact_deployed().unwrap_or(COMPACT_DEFAULT),
        |s| compact_at(&s),
    )
}

/// Sets the person's pick, in tokens; None goes back to the default.
pub fn compact_pick(state: &Path, tokens: Option<u64>) -> Result<(), String> {
    match tokens {
        None => {
            COMPACT_AT.clear(state);
            Ok(())
        }
        Some(n) if n < COMPACT_LEAST => Err(format!("compact at {COMPACT_LEAST} tokens or more")),
        Some(n) => COMPACT_AT.write(state, &n.to_string()),
    }
}

pub fn state() -> Option<PathBuf> {
    std::env::var_os("STATE_DIRECTORY").map(PathBuf::from)
}

/// Where a turn's session comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Start {
    /// The chat's own session on this harness, resumed.
    Resume(String),
    /// None yet: a copy of another chat's, as a worker forks the main
    /// chat's thread at its start.
    Fork(String),
    /// None to go on from: a new one, under this id.
    New(String),
}

impl Start {
    /// Whether the session already holds the thread, so the turn sends only
    /// what is new.
    pub fn holds_thread(&self) -> bool {
        !matches!(self, Self::New(_))
    }
}

/// How `chat`'s next turn on `harness` starts: its own session, else a fork
/// of `from`'s, else a new one with the id `new` gives.
pub fn start(
    state: &Path,
    chat: &str,
    from: Option<&str>,
    harness: &str,
    new: impl FnOnce() -> String,
) -> Start {
    let own = |c: &str| get(state, c, harness).filter(|_| current(state, c, harness));
    if let Some(id) = own(chat) {
        return Start::Resume(id);
    }
    match from.and_then(own) {
        Some(id) => Start::Fork(id),
        None => Start::New(new()),
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn compaction_point_is_the_pick_else_the_default() {
        let dir = std::env::temp_dir().join(format!("libri-compact-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(super::compact_at(&dir), super::COMPACT_DEFAULT);
        super::compact_pick(&dir, Some(300_000)).unwrap();
        assert_eq!(super::compact_at(&dir), 300_000);
        assert!(
            super::compact_pick(&dir, Some(5)).is_err(),
            "below the least is refused"
        );
        assert_eq!(super::compact_at(&dir), 300_000);
        super::compact_pick(&dir, None).unwrap();
        assert_eq!(super::compact_at(&dir), super::COMPACT_DEFAULT);
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;

    fn scratch() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "libri-sessions-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn keeps_one_session_per_harness() {
        let s = scratch();
        assert_eq!(get(&s, MAIN, "claude"), None);
        put(&s, MAIN, "claude", "a").unwrap();
        put(&s, MAIN, "codex", "b").unwrap();
        assert_eq!(get(&s, MAIN, "claude").as_deref(), Some("a"));
        assert_eq!(get(&s, MAIN, "codex").as_deref(), Some("b"));
        forget(&s, MAIN, "claude").unwrap();
        assert_eq!(get(&s, MAIN, "claude"), None);
        assert_eq!(get(&s, MAIN, "codex").as_deref(), Some("b"));
    }

    #[test]
    fn a_worker_forks_the_main_session_until_it_has_its_own() {
        let s = scratch();
        let w = worker("ada-1");
        let new = || "fresh".to_owned();
        assert_eq!(
            start(&s, &w, Some(MAIN), "claude", new),
            Start::New("fresh".into())
        );
        put(&s, MAIN, "claude", "m").unwrap();
        assert_eq!(
            start(&s, &w, Some(MAIN), "claude", new),
            Start::Fork("m".into())
        );
        assert_eq!(
            start(&s, &w, None, "claude", new),
            Start::New("fresh".into())
        );
        put(&s, &w, "claude", "own").unwrap();
        assert_eq!(
            start(&s, &w, Some(MAIN), "claude", new),
            Start::Resume("own".into())
        );
    }

    #[test]
    fn clearing_starts_every_harness_anew_and_keeps_the_old_ids() {
        let s = scratch();
        let new = || "fresh".to_owned();
        put(&s, MAIN, "codex", "x").unwrap();
        put(&s, MAIN, "claude", "c").unwrap();
        put_made(&s, MAIN, "claude", "m").unwrap();
        put(&s, &worker("ada-1"), "claude", "w").unwrap();
        let omp = folder(&s, MAIN, "omp").unwrap();
        std::fs::create_dir_all(&omp).unwrap();
        std::fs::write(omp.join("a.jsonl"), "").unwrap();
        clear(&s, MAIN).unwrap();
        assert_eq!(
            start(&s, MAIN, None, "claude", new),
            Start::New("fresh".into())
        );
        assert_eq!(
            start(&s, MAIN, None, "codex", new),
            Start::New("fresh".into())
        );
        assert_eq!(made(&s, MAIN, "claude"), None);
        assert!(!omp.exists());
        let ids: Vec<_> = read(&s, MAIN)
            .values()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        assert!(
            ids.contains(&"c".to_owned()) && ids.contains(&"x".to_owned()),
            "{ids:?}"
        );
        // Another chat's is its own; a second clear keeps the first's ids.
        assert_eq!(get(&s, &worker("ada-1"), "claude").as_deref(), Some("w"));
        put(&s, MAIN, "claude", "d").unwrap();
        clear(&s, MAIN).unwrap();
        let ids: Vec<_> = read(&s, MAIN)
            .values()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        assert_eq!(ids.len(), 3, "{ids:?}");
    }

    #[test]
    fn a_session_on_a_harness_that_ran_the_chat_before_another_starts_anew() {
        let s = scratch();
        let new = || "fresh".to_owned();
        put(&s, MAIN, "claude", "c").unwrap();
        assert_eq!(
            start(&s, MAIN, None, "claude", new),
            Start::Resume("c".into())
        );
        // The chat goes to Codex, then back: Claude's session misses its turns.
        put(&s, MAIN, "codex", "x").unwrap();
        assert!(!current(&s, MAIN, "claude"));
        assert_eq!(
            start(&s, MAIN, None, "claude", new),
            Start::New("fresh".into())
        );
        // Nor does a worker copy a stale one.
        assert_eq!(
            start(&s, &worker("w"), Some(MAIN), "claude", new),
            Start::New("fresh".into())
        );
        assert_eq!(
            start(&s, &worker("w"), Some(MAIN), "codex", new),
            Start::Fork("x".into())
        );
        // A harness of folders says so itself.
        ran(&s, MAIN, "omp").unwrap();
        assert!(current(&s, MAIN, "omp") && !current(&s, MAIN, "codex"));
        // A session starting is named before it is the chat's.
        put_next(&s, MAIN, "claude", "n").unwrap();
        assert_eq!(get(&s, MAIN, "claude.next").as_deref(), Some("n"));
        assert_eq!(get(&s, MAIN, "claude").as_deref(), Some("c"));
        // What a session was made with leaves `last` alone.
        put_made(&s, MAIN, "omp", "m").unwrap();
        assert_eq!(made(&s, MAIN, "omp").as_deref(), Some("m"));
        assert!(current(&s, MAIN, "omp"));
    }

    #[test]
    fn a_harness_that_names_its_own_sessions_gets_a_folder_per_chat() {
        let s = scratch();
        let f = folder(&s, MAIN, "omp").unwrap();
        assert_eq!(f, s.join(DIR).join("main.omp"));
        assert!(!holds_session(&f));
        std::fs::create_dir_all(&f).unwrap();
        std::fs::write(f.join("a.jsonl"), "").unwrap();
        assert!(holds_session(&f));
        assert!(folder(&s, "../x", "omp").is_none());
    }

    #[test]
    fn refuses_names_that_leave_the_folder() {
        let s = scratch();
        assert!(put(&s, "../x", "claude", "a").is_err());
        assert!(put(&s, "", "claude", "a").is_err());
        assert_eq!(get(&s, "../x", "claude"), None);
    }
}
