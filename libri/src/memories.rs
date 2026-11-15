//! Named memories beside the main one: each a hippo store of its own with a
//! chat that runs on it, as a model is picked per chat. The main chat keeps
//! the main memory, which nothing here reads or writes. A memory's chat
//! reads only its own store, and its prompt is the person's rules, which
//! hold for every memory (`guide::RULES`), with the memory's own
//! instructions on top.
//!
//! Each memory is a folder under `state/memories/<id>`: its name, its
//! instructions (`agent.md`), its hippo store and spool, and a `task` tree
//! holding its one chat, run with the same `task` functions as Temp's.
//! Removing the folder removes all of it.

use crate::hippo::{Hippo, Spool};
use crate::host::HippoChild;
use crate::task;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The main memory's id, which every chat but a named memory's runs on.
pub const MAIN: &str = "main";

/// What a memory chat's ID starts with, which pages route its steps by.
pub const PREFIX: &str = "memory-";

const NAME: &str = "name";
const INSTRUCTIONS: &str = "agent.md";

/// The most a name runs to, so it fits a pill.
const NAME_MAX: usize = 40;
/// As the rules: Claude Code and omp take the prompt as one argument.
const INSTRUCTIONS_MAX: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Memory {
    pub id: String,
    pub name: String,
}

fn root(state: &Path) -> PathBuf {
    state.join("memories")
}

fn folder(state: &Path, id: &str) -> PathBuf {
    root(state).join(id)
}

/// The `task` tree holding memory `id`'s chat.
pub fn store(state: &Path, id: &str) -> PathBuf {
    folder(state, id)
}

/// The ID of memory `id`'s chat, unique among every thread a page hears of.
pub fn thread(id: &str) -> String {
    format!("{PREFIX}{id}")
}

/// An ID is a folder name: lowercase letters, digits and dashes.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id != MAIN
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Whether memory `id` exists.
pub fn exists(state: &Path, id: &str) -> bool {
    valid_id(id) && folder(state, id).join(NAME).is_file()
}

/// Every named memory, by name.
pub fn list(state: &Path) -> Vec<Memory> {
    let Ok(entries) = fs::read_dir(root(state)) else {
        return Vec::new();
    };
    let mut all: Vec<Memory> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let id = e.file_name().into_string().ok()?;
            let name = fs::read_to_string(e.path().join(NAME)).ok()?;
            valid_id(&id).then(|| Memory {
                id,
                name: name.trim().to_owned(),
            })
        })
        .collect();
    all.sort_by_key(|m| m.name.to_lowercase());
    all
}

/// A name as kept, or why it can't be one.
fn checked_name(state: &Path, name: &str, own: Option<&str>) -> Result<String, String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("A memory needs a name.".into());
    }
    if name.chars().count() > NAME_MAX {
        return Err(format!("A name is at most {NAME_MAX} characters."));
    }
    if name.eq_ignore_ascii_case(MAIN) {
        return Err("Main is the main chat's memory.".into());
    }
    let taken = list(state)
        .into_iter()
        .any(|m| Some(m.id.as_str()) != own && m.name.eq_ignore_ascii_case(&name));
    if taken {
        return Err(format!("There is a memory named {name} already."));
    }
    Ok(name)
}

/// An ID from `name`, kept for good: a rename changes the name alone.
fn new_id(state: &Path, name: &str) -> String {
    let mut base = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            base.push(c);
        } else if !base.ends_with('-') && !base.is_empty() {
            base.push('-');
        }
    }
    let base = base.trim_end_matches('-');
    let base = if base.is_empty() || base == MAIN {
        "memory"
    } else {
        base
    };
    let mut id = base.to_owned();
    let mut n = 2;
    while folder(state, &id).exists() {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// Makes a memory named `name`, empty, with its chat ready for words.
pub fn add(state: &Path, name: &str) -> Result<Memory, String> {
    let name = checked_name(state, name, None)?;
    let id = new_id(state, &name);
    let d = folder(state, &id);
    let chat = task::directory(&store(state, &id), &thread(&id));
    fs::create_dir_all(chat.join("told")).map_err(|e| format!("{}: {e}", chat.display()))?;
    task::put(&chat.join("log.jsonl"), "")?;
    task::put(&d.join(INSTRUCTIONS), "")?;
    // Last: a folder without its name is no memory yet.
    task::put(&d.join(NAME), &name)?;
    Ok(Memory { id, name })
}

pub fn rename(state: &Path, id: &str, name: &str) -> Result<Memory, String> {
    if !exists(state, id) {
        return Err(format!("No memory {id}."));
    }
    let name = checked_name(state, name, Some(id))?;
    task::put(&folder(state, id).join(NAME), &name)?;
    Ok(Memory {
        id: id.to_owned(),
        name,
    })
}

/// Removes memory `id` and all of it: its store, its chat, its
/// instructions. Its hippo must be stopped first (`Hippos::close`).
pub fn delete(state: &Path, id: &str) -> Result<(), String> {
    if !exists(state, id) {
        return Err(format!("No memory {id}."));
    }
    let d = folder(state, id);
    fs::remove_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))
}

/// Memory `id`'s own instructions, empty until written.
pub fn instructions(state: &Path, id: &str) -> String {
    fs::read_to_string(folder(state, id).join(INSTRUCTIONS)).unwrap_or_default()
}

pub fn set_instructions(state: &Path, id: &str, text: &str) -> Result<(), String> {
    if !exists(state, id) {
        return Err(format!("No memory {id}."));
    }
    if text.len() > INSTRUCTIONS_MAX {
        return Err(format!(
            "This is {} KB; agent.md can hold {} KB.",
            text.len() / 1024,
            INSTRUCTIONS_MAX / 1024
        ));
    }
    task::put(&folder(state, id).join(INSTRUCTIONS), text)
}

/// What the person said since the last answer, whole: all a turn needs
/// beside the view, whose lines may have folded the words.
pub fn newest(log: &[task::Line]) -> String {
    let last = log
        .iter()
        .rposition(|l| l.kind == "talk")
        .map_or(0, |i| i + 1);
    log[last..]
        .iter()
        .filter(|l| l.kind == "user")
        .map(|l| format!("user: {}", l.text.replace('\n', "\n  ")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The hippo store of memory `id`.
pub fn hippo_dir(state: &Path, id: &str) -> PathBuf {
    folder(state, id).join("hippo")
}

/// The named memories' hippos, each a child of this daemon started on first
/// use, as the main one is when Libri serves it (`host::serve_hippo`). Its
/// compactor answers through this daemon, as the main one's does.
pub struct Hippos {
    state: PathBuf,
    open: Mutex<HashMap<String, Open>>,
}

struct Open {
    hippo: Arc<Hippo>,
    child: Arc<HippoChild>,
}

impl Hippos {
    pub fn new(state: PathBuf) -> Self {
        Self {
            state,
            open: Mutex::default(),
        }
    }

    /// Memory `id`'s hippo, started if it isn't yet.
    pub fn get(&self, id: &str) -> Result<Arc<Hippo>, String> {
        if !exists(&self.state, id) {
            return Err(format!("No memory {id}."));
        }
        let mut open = self.open.lock().unwrap();
        if let Some(o) = open.get(id) {
            return Ok(o.hippo.clone());
        }
        let dir = hippo_dir(&self.state, id);
        crate::host::private_dir(&dir)?;
        // Its workers' logs are in its hippo's reach, as the main memory's
        // are in the main hippo's, for forks, `<work>` and search. Every
        // worker's folder is under `state/task`: the `memory` file names
        // this store's, and hippo reads only the workers that name it too.
        task::put(&dir.join("memory"), id)?;
        let agents = dir.join("agents");
        if fs::symlink_metadata(&agents).is_err() {
            std::os::unix::fs::symlink(task::folders(&self.state), &agents)
                .map_err(|e| format!("{}: {e}", agents.display()))?;
        }
        let hippo = Arc::new(Hippo {
            sock: dir.join("hippo.sock"),
            chat: None,
            spool: Some(Arc::new(Spool::new(folder(&self.state, id).join("spool"))?)),
        });
        let child = crate::host::run_hippo(&dir, "chat")?;
        open.insert(
            id.to_owned(),
            Open {
                hippo: hippo.clone(),
                child,
            },
        );
        Ok(hippo)
    }

    /// Stops memory `id`'s hippo, before its folder goes.
    pub fn close(&self, id: &str) {
        if let Some(o) = self.open.lock().unwrap().remove(id) {
            o.child.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("memories-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_memory_is_added_renamed_and_deleted_whole() {
        let s = state("life");
        let m = add(&s, "Tutor").unwrap();
        assert_eq!(m.id, "tutor");
        assert!(exists(&s, "tutor"));
        assert!(
            task::directory(&store(&s, "tutor"), &thread("tutor"))
                .join("log.jsonl")
                .is_file()
        );
        let r = rename(&s, "tutor", "Socratic tutor").unwrap();
        assert_eq!(
            (r.id.as_str(), r.name.as_str()),
            ("tutor", "Socratic tutor")
        );
        set_instructions(&s, "tutor", "Ask, don't tell.").unwrap();
        assert_eq!(instructions(&s, "tutor"), "Ask, don't tell.");
        delete(&s, "tutor").unwrap();
        assert!(!exists(&s, "tutor"));
        assert!(list(&s).is_empty());
    }

    #[test]
    fn names_are_unique_and_main_is_kept_for_the_main_chat() {
        let s = state("names");
        add(&s, "Jobs").unwrap();
        assert!(add(&s, "jobs").is_err());
        assert!(add(&s, "Main").is_err());
        assert!(add(&s, "  ").is_err());
        assert!(add(&s, &"x".repeat(NAME_MAX + 1)).is_err());
        let other = add(&s, " JOBS ").unwrap_err();
        assert!(other.contains("already"), "{other}");
        let two = add(&s, "Jobs search").unwrap();
        assert_eq!(two.id, "jobs-search");
        assert_eq!(
            list(&s).iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            ["Jobs", "Jobs search"]
        );
    }

    #[test]
    fn an_id_is_kept_through_a_rename_and_never_reused_while_taken() {
        let s = state("ids");
        add(&s, "Tutor").unwrap();
        rename(&s, "tutor", "Teacher").unwrap();
        let again = add(&s, "Tutor").unwrap();
        assert_eq!(again.id, "tutor-2");
    }

    #[test]
    fn only_a_folder_name_is_a_memory_id() {
        let s = state("ids-only");
        add(&s, "Tutor").unwrap();
        assert!(exists(&s, "tutor"));
        assert!(!exists(&s, "../memories/tutor"));
        assert!(!exists(&s, "main"));
        assert!(set_instructions(&s, "../tutor", "x").is_err());
    }

    #[test]
    fn a_turn_has_the_words_since_the_last_answer_whole() {
        let line = |kind: &str, text: &str| task::Line {
            kind: kind.into(),
            text: text.into(),
            id: None,
        };
        let log = [
            line("user", "first"),
            line("talk", "an answer"),
            line("user", "two\nlines"),
            line("tool", "web_search {}"),
            line("user", "more"),
        ];
        assert_eq!(newest(&log), "user: two\n  lines\nuser: more");
        assert_eq!(newest(&log[..2]), "");
    }
}
