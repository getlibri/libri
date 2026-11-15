//! The scratchpad, shown as Temp: a thread that runs as a worker's does,
//! with the same turn path and log, and stays out of the memory. Its folders
//! are a `task` tree of their own under `state/scratchpad`, handed to the
//! same `task` functions as the workers' tree is: the roster, the Work page,
//! the reports and hippo's `agents` link all read `state/task`, so nothing
//! there can reach it.
//!
//! Several may be open at once, each a Temp tile of its own (tiles.rs: a
//! Temp opened beside one is a new one); each runs on its own, side by side.
//! `current` is the one a page with no Temp named opens, as before there
//! were several. A scratchpad's log is its whole context: every turn sends
//! it, words, calls and results, so the model can change between turns, and
//! it goes when the person closes it or promotes it.

use crate::model::Kept;
use crate::task;
use std::fs;
use std::path::{Path, PathBuf};

/// What a scratchpad's ID starts with, which the pages route its steps by.
pub const PREFIX: &str = "scratchpad-";

/// Names the open scratchpad's folder.
const CURRENT: &str = "current";

/// The memory switch, which outlives any one scratchpad.
pub const MEMORY: Kept = Kept("temp-memory");

/// Set in a scratchpad's folder while its run has the memory off.
const MEMORY_OFF: &str = "memory-off";

/// Where a build before memory-off turns ran in the person's own setup kept
/// a run directory each; a restart cut off may have left some. Goes once
/// every host has run a build with this.
const PRIVATE: &str = "private";

/// The tree `task` functions take for the scratchpad.
pub fn store(state: &Path) -> PathBuf {
    state.join("scratchpad")
}

/// Whether the memory is on: the person's last word on it, on at first.
pub fn memory(state: &Path) -> bool {
    MEMORY.read(state).as_deref() != Some("off")
}

pub fn set_memory(state: &Path, on: bool) -> Result<(), String> {
    MEMORY.write(state, if on { "on" } else { "off" })
}

/// The open scratchpad's ID.
pub fn current(state: &Path) -> Option<String> {
    let store = store(state);
    let id = fs::read_to_string(store.join(CURRENT)).ok()?;
    task::directory(&store, &id)
        .join("log.jsonl")
        .is_file()
        .then_some(id)
}

/// The open scratchpad's ID, opening one if none is.
pub fn open(state: &Path) -> Result<String, String> {
    if let Some(id) = current(state) {
        return Ok(id);
    }
    let id = make(state)?;
    task::put(&store(state).join(CURRENT), &id)?;
    Ok(id)
}

/// A new scratchpad, beside any open; its ID.
pub fn open_new(state: &Path) -> Result<String, String> {
    make(state)
}

fn make(state: &Path) -> Result<String, String> {
    let store = store(state);
    let id = format!(
        "{PREFIX}{}",
        chrono::Local::now().format("%Y-%m-%d-%H%M%S-%f")
    );
    let d = task::directory(&store, &id);
    fs::create_dir_all(d.join("told")).map_err(|e| format!("{}: {e}", d.display()))?;
    task::put(&d.join("log.jsonl"), "")?;
    Ok(id)
}

/// Whether scratchpad `id` is open.
pub fn exists(state: &Path, id: &str) -> bool {
    id.starts_with(PREFIX)
        && !id.contains(['/', '.'])
        && task::directory(&store(state), id)
            .join("log.jsonl")
            .is_file()
}

/// The scratchpad a page names, or the current one when it names none.
pub fn named(state: &Path, id: Option<&str>) -> Option<String> {
    match id {
        Some(id) => exists(state, id).then(|| id.to_owned()),
        None => current(state),
    }
}

/// Every open scratchpad's ID.
pub fn all(state: &Path) -> Vec<String> {
    // Each lives where `task` keeps a thread, under the store's root.
    let Some(root) = task::directory(&store(state), "x")
        .parent()
        .map(Path::to_path_buf)
    else {
        return Vec::new();
    };
    let Ok(dir) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = dir
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| exists(state, n))
        .collect();
    ids.sort();
    ids
}

/// Closes scratchpad `id` and removes all of it. A run still going sees it
/// gone and stops.
pub fn close_one(state: &Path, id: &str) {
    if !exists(state, id) {
        return;
    }
    let store = store(state);
    let d = task::directory(&store, id);
    if let Err(e) = fs::remove_dir_all(&d) {
        eprintln!("libri: {}: {e}", d.display());
    }
    if current(state).is_none() {
        let _ = fs::remove_file(store.join(CURRENT));
    }
}

/// Closes the open scratchpad and removes all of it. A run still going sees
/// it gone and stops.
pub fn close(state: &Path) {
    let store = store(state);
    if let Some(id) = current(state) {
        let d = task::directory(&store, &id);
        if let Err(e) = fs::remove_dir_all(&d) {
            eprintln!("libri: {}: {e}", d.display());
        }
    }
    let _ = fs::remove_file(store.join(CURRENT));
}

/// Whether worker `id` is a scratchpad, for a tool to see whose call it has.
pub fn is(state: &Path, id: &str) -> bool {
    !id.is_empty() && task::directory(&store(state), id).is_dir()
}

/// Whether scratchpad `id` has a run at work or waiting for one.
pub fn working(state: &Path, id: &str) -> bool {
    task::at_work(&store(state), id)
}

/// Marks whether scratchpad `id`'s run has the memory off, for the tool
/// server, which on OpenCode serves every session alike.
pub fn set_memory_off(state: &Path, id: &str, off: bool) -> Result<(), String> {
    let p = task::directory(&store(state), id).join(MEMORY_OFF);
    if off {
        task::put(&p, "")
    } else {
        match fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("{}: {e}", p.display()))
            }
            _ => Ok(()),
        }
    }
}

pub fn memory_off(state: &Path, id: &str) -> bool {
    task::directory(&store(state), id).join(MEMORY_OFF).exists()
}

/// The scratchpad's log as the main chat's rows: each message under a new
/// key, in order, and each result under its call's key and `.result`, as
/// a main-chat turn keeps them (hippo.rs `Keeper`), so a page draws the
/// promoted talk exactly as if it had happened there. Words a worker sent
/// in have no place in the main chat and stay behind.
pub fn promoted(log: &[task::Line]) -> Vec<(String, &'static str, String)> {
    let mut calls = std::collections::HashMap::new();
    let mut rows = Vec::new();
    for l in log.iter().filter(|l| !l.text.trim().is_empty()) {
        let (key, kind) = match l.kind.as_str() {
            "user" => (crate::hippo::row_key(), "user"),
            "talk" => (crate::hippo::row_key(), "talk"),
            "tool" => {
                let key = crate::hippo::row_key();
                if let Some(id) = &l.id {
                    calls.insert(id.clone(), key.clone());
                }
                (key, "tool")
            }
            "echo" => {
                let call = l.id.as_ref().and_then(|id| calls.remove(id));
                let key = call.map_or_else(crate::hippo::row_key, |c| {
                    format!("{c}{}", crate::hippo::RESULT)
                });
                (key, "echo")
            }
            _ => continue,
        };
        rows.push((key, kind, l.text.clone()));
    }
    rows
}

/// Removes the run directories an older build left, and sets runs a
/// restart cut off going again.
pub fn recover(state: &Path) {
    let store = store(state);
    let _ = fs::remove_dir_all(store.join(PRIVATE));
    task::resume(&store);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("scratchpad-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_scratchpad_stays_open_until_closed_and_its_folder_goes_with_it() {
        let s = state("open");
        assert_eq!(current(&s), None);
        let id = open(&s).unwrap();
        assert_eq!(open(&s).unwrap(), id);
        assert_eq!(current(&s).as_deref(), Some(id.as_str()));
        assert!(is(&s, &id));
        close(&s);
        assert_eq!(current(&s), None);
        assert!(!is(&s, &id));
        assert_ne!(open(&s).unwrap(), id);
    }

    #[test]
    fn several_stay_open_apart_and_each_closes_alone() {
        let s = state("several");
        let a = open(&s).unwrap();
        let b = open_new(&s).unwrap();
        assert_ne!(a, b);
        assert_eq!(all(&s), {
            let mut v = vec![a.clone(), b.clone()];
            v.sort();
            v
        });
        assert_eq!(named(&s, None).as_deref(), Some(a.as_str()));
        assert_eq!(named(&s, Some(&b)).as_deref(), Some(b.as_str()));
        assert_eq!(named(&s, Some("scratchpad-none")), None);
        assert_eq!(named(&s, Some("../task")), None);
        close_one(&s, &b);
        assert!(!exists(&s, &b) && exists(&s, &a));
        close_one(&s, &a);
        assert_eq!(current(&s), None);
        assert!(all(&s).is_empty());
    }

    #[test]
    fn workers_do_not_see_a_scratchpad_and_it_is_not_one_of_theirs() {
        let s = state("apart");
        let pick = crate::catalog::TurnPick {
            pick: crate::catalog::Pick {
                harness: "claude".into(),
                model: "opus".into(),
            },
            effort: None,
            access: None,
            sandbox: None,
        };
        let worker = task::spawn(&s, "work", &pick).unwrap();
        let id = open(&s).unwrap();
        task::tell(&store(&s), &id, "secret words").unwrap();
        assert!(!task::roster(&s, None, None).contains("scratchpad"));
        assert!(task::threads(&s).iter().all(|t| t.id == worker));
        assert!(task::tell(&s, &id, "x").is_err());
        assert!(!is(&s, &worker));
        assert!(!fs::read_dir(s.join("task")).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("scratchpad")
        }));
    }

    #[test]
    fn a_turn_cut_by_a_restart_goes_on_and_leaves_no_run_directory() {
        let s = state("restart");
        let id = open(&s).unwrap();
        task::tell(&store(&s), &id, "words").unwrap();
        assert!(task::take(&store(&s), &id));
        let left = store(&s).join(PRIVATE).join("20261103000000");
        fs::create_dir_all(&left).unwrap();
        recover(&s);
        assert!(!left.exists());
        assert!(task::take(&store(&s), &id));
    }

    #[test]
    fn promoting_keeps_each_message_in_order_and_pairs_results_with_calls() {
        let line = |kind: &str, text: &str, id: Option<&str>| task::Line {
            kind: kind.into(),
            text: text.into(),
            id: id.map(Into::into),
        };
        let log = [
            line("user", "What's the weather?", None),
            line("tool", "web_search {}", Some("c1")),
            line("tool", "web_fetch {}", Some("c2")),
            line("echo", "fetched", Some("c2")),
            line("echo", "found", Some("c1")),
            line("talk", "Sunny.", None),
            line("from", "Kit: hello", None),
            line("user", "  ", None),
            line("user", "Thanks", None),
        ];
        let rows = promoted(&log);
        let kinds: Vec<_> = rows.iter().map(|r| (r.1, r.2.as_str())).collect();
        assert_eq!(
            kinds,
            [
                ("user", "What's the weather?"),
                ("tool", "web_search {}"),
                ("tool", "web_fetch {}"),
                ("echo", "fetched"),
                ("echo", "found"),
                ("talk", "Sunny."),
                ("user", "Thanks"),
            ]
        );
        assert_eq!(rows[3].0, format!("{}.result", rows[2].0));
        assert_eq!(rows[4].0, format!("{}.result", rows[1].0));
        let keys: std::collections::HashSet<_> = rows.iter().map(|r| &r.0).collect();
        assert_eq!(keys.len(), rows.len());
    }

    #[test]
    fn the_memory_off_mark_is_the_run_s() {
        let s = state("mark");
        let id = open(&s).unwrap();
        assert!(!memory_off(&s, &id));
        set_memory_off(&s, &id, true).unwrap();
        assert!(memory_off(&s, &id));
        set_memory_off(&s, &id, false).unwrap();
        set_memory_off(&s, &id, false).unwrap();
        assert!(!memory_off(&s, &id));
    }
}
