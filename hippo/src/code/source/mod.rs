//! Harness transcripts turned into events. Each parser reads one raw entry
//! and says what it means for the log; the watcher decides when to log.

pub mod claude;
pub mod codex;
pub mod muse;
pub mod opencode;

use crate::memory::mask::mask;
use crate::memory::store::{Kind, Src};
use chrono::{DateTime, Local};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Max size of a tool result in the log, in characters, head and tail kept.
pub const CAP: usize = 30_000;

#[derive(Clone, Debug)]
pub struct Item {
    pub kind: Kind,
    pub text: String,
    pub date: DateTime<Local>,
    pub src: Src,
    /// The captain's prompt that opens a turn (a `user` message typed while
    /// a turn runs does not).
    pub opens: bool,
    /// Tool call id, on a `tool`; the call answered, on an `echo`.
    pub call: Option<String>,
    /// A `tool` that runs hippo or memo: its result is left out.
    pub memory: bool,
}

#[derive(Clone, Debug)]
pub enum Event {
    Item(Item),
    /// The agent finished answering.
    TurnEnd,
    /// Session facts: working directory, title.
    Info {
        cwd: Option<String>,
        title: Option<String>,
    },
    /// The whole session is not the seat's own chat (a subagent).
    Foreign,
    /// An entry that could not be understood: reported, never guessed.
    Unparsed(String),
}

/// A subagent's report as the timeline logs it, as Unii does: who sent it,
/// its words, and how to open its whole chat.
pub fn report(name: &str, text: &str) -> String {
    format!("[{name}]\n{}\n\nFull chat: hippo zoom {name}", text.trim())
}

/// Characters in one page of a subagent's chat.
const PAGE: usize = 20_000;

/// The lines of a subagent's chat, a page from character `at`.
pub fn paged(name: &str, lines: &[String], at: usize) -> String {
    let all = mask(&lines.join("\n\n"));
    let chars: Vec<char> = all.chars().collect();
    let from = at.min(chars.len());
    let to = (from + PAGE).min(chars.len());
    let mut out: String = chars[from..to].iter().collect();
    if to < chars.len() {
        out.push_str(&format!(
            "\n\n(more from character {to} of {}: hippo zoom {name} {to})",
            chars.len()
        ));
    }
    out
}

/// Whether the Libri worker in folder `d` is archived: its `state.json`
/// (lifecycle.rs in Libri) says so, or, in a folder Libri has not yet
/// given one, its `archived` marker does. The marker goes once Libri
/// stops writing markers.
pub fn archived(d: &Path) -> bool {
    match std::fs::read(d.join("state.json")) {
        Ok(b) => {
            serde_json::from_slice::<Value>(&b).is_ok_and(|v| v["phase"]["state"] == "archived")
        }
        Err(_) => d.join("archived").exists(),
    }
}

/// The ID of the worker a bare name means.
fn worker(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            let shown =
                std::fs::read_to_string(e.path().join("name")).unwrap_or_else(|_| id.clone());
            shown.eq_ignore_ascii_case(name).then(|| {
                let at = id.split_once('-').map_or("", |(_, at)| at).to_owned();
                (!archived(&e.path()), at, id)
            })
        })
        .max()
        .map(|(_, _, id)| id)
}

/// A subagent's log as Libri keeps it, `<dir>/<name>/log.jsonl`, one
/// `{kind, text}` a line; those of agents a harness started itself sit
/// under `.inner`. A bare name means the unarchived worker shown by that
/// name, else the most recent (task.rs in Libri): folders are named by ID,
/// and the name lives in the folder's `name` file, or is the folder's own
/// on those from before IDs. None when it has no log there.
pub fn agent_log(dir: &Path, name: &str) -> Result<Option<Vec<String>>, String> {
    let Some(file) = agent_file(dir, name)? else {
        return Ok(None);
    };
    let raw = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let lines = raw
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .enumerate()
        .map(|(i, l)| {
            let (kind, text) = (str_at(&l, "/kind"), str_at(&l, "/text"));
            // Only the host starts a line, as in Libri's own reading of a
            // log: a worker's words can't pass as an entry of another kind.
            let text = text.unwrap_or("").replace('\n', "\n  ");
            format!("{i}|{}: {text}", kind.unwrap_or("?"))
        })
        .collect();
    Ok(Some(lines))
}

/// The log of the subagent `name` or id, in Libri's `agents/`.
pub fn agent_file(dir: &Path, name: &str) -> Result<Option<PathBuf>, String> {
    // A numbered worker's bare name has a space: `Worker 3`.
    let known = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' ');
    if name.is_empty() || !name.chars().all(known) {
        return Err(format!("No agent {name}."));
    }
    let id = (!name.contains('-')).then(|| worker(dir, name)).flatten();
    let name = id.as_deref().unwrap_or(name);
    Ok([dir.join(name), dir.join(".inner").join(name)]
        .into_iter()
        .map(|d| d.join("log.jsonl"))
        .find(|p| p.is_file()))
}

pub fn str_at<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer).and_then(Value::as_str)
}

pub fn iso(s: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Local))
}

/// Text of a content value: a string, or an array of text and image blocks.
pub fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(s) => Some(s.clone()),
                _ => match str_at(item, "/type") {
                    Some("image" | "input_image") => Some("[image]".to_owned()),
                    Some("document") => Some("[document]".to_owned()),
                    _ => item
                        .get("text")
                        .or_else(|| item.get("output"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                },
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A tool call as its name and JSON input.
pub fn call_text(name: &str, input: &Value) -> String {
    match input {
        Value::Null => name.to_owned(),
        Value::String(raw) => format!("{name} {raw}"),
        other => format!("{name} {other}"),
    }
}

/// Caps a tool result at `CAP` characters, keeping head and tail and saying
/// what was cut.
pub fn cap(text: String) -> String {
    let chars = text.chars().count();
    if chars <= CAP {
        return text;
    }
    let half = CAP / 2;
    let head: String = text.chars().take(half).collect();
    let tail: String = text.chars().skip(chars - half).collect();
    format!(
        "{head}\n[… {} of {chars} characters cut by hippo …]\n{tail}",
        chars - CAP
    )
}

/// Whether a tool call runs `hippo` or `memo`: its output would make the
/// memory summarize itself.
pub fn runs_memory(input: &Value) -> bool {
    match input {
        Value::String(raw) => match serde_json::from_str::<Value>(raw) {
            Ok(v) => is_memory(&command_of(&v)),
            // Codex's exec tool takes a script: tools.exec_command({cmd: "…"}).
            Err(_) => {
                let mut cmds = CMD
                    .captures_iter(raw)
                    .map(|c| c[1].replace("\\\"", "\""))
                    .peekable();
                if cmds.peek().is_none() {
                    is_memory(raw)
                } else {
                    cmds.any(|c| is_memory(&c))
                }
            }
        },
        other => is_memory(&command_of(other)),
    }
}

static CMD: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r#""?(?:cmd|command)"?\s*:\s*"((?:[^"\\]|\\.)*)""#).unwrap()
});

fn is_memory(command: &str) -> bool {
    command
        .split(['\n', ';', '&', '|', '(', ')', '`'])
        .flat_map(|seg| seg.split("$("))
        .any(|seg| {
            let mut words = seg
                .split_whitespace()
                .skip_while(|w| w.contains('=') && !w.starts_with('-'));
            let first = words.next().unwrap_or("");
            let first = first.rsplit('/').next().unwrap_or(first);
            matches!(first, "hippo" | "memo")
        })
}

fn command_of(v: &Value) -> String {
    match v
        .get("command")
        .or_else(|| v.get("cmd"))
        .or_else(|| v.get("input"))
    {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => {
            let parts: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
            // ["bash", "-lc", script] runs the script.
            match parts.as_slice() {
                [_, flag, script] if flag.ends_with('c') && flag.starts_with('-') => {
                    (*script).to_owned()
                }
                _ => parts.join(" "),
            }
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ARCHIVED: &str = r#"{"phase":{"state":"archived","at":1}}"#;

    #[test]
    fn a_folder_without_a_record_is_archived_by_its_marker() {
        let d = std::env::temp_dir().join(format!("hippo-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        assert!(!archived(&d));
        std::fs::write(d.join("archived"), "").unwrap();
        assert!(archived(&d));
        // Once Libri has written the record, it wins over the marker.
        std::fs::write(d.join("state.json"), r#"{"phase":{"state":"idle"}}"#).unwrap();
        assert!(!archived(&d));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_bare_name_means_the_unarchived_worker_else_the_most_recent() {
        let dir = std::env::temp_dir().join(format!("hippo-workers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (id, name, archived) in [
            ("Kit", None, true),
            ("kit-2026-10-30-1432", Some("Kit"), true),
            ("kit-2026-10-31-0900", Some("Kit"), true),
            ("lark-2026-10-29-0800", Some("Kit"), false),
        ] {
            let d = dir.join(id);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(
                d.join("log.jsonl"),
                format!("{{\"kind\":\"note\",\"text\":\"{id}\"}}\n"),
            )
            .unwrap();
            if let Some(name) = name {
                std::fs::write(d.join("name"), name).unwrap();
            }
            if archived {
                std::fs::write(d.join("state.json"), ARCHIVED).unwrap();
            }
        }
        assert_eq!(worker(&dir, "kit").as_deref(), Some("lark-2026-10-29-0800"));
        std::fs::write(dir.join("lark-2026-10-29-0800/state.json"), ARCHIVED).unwrap();
        assert_eq!(worker(&dir, "Kit").as_deref(), Some("kit-2026-10-31-0900"));
        let log = agent_log(&dir, "kit-2026-10-30-1432").unwrap().unwrap();
        assert!(log.join("\n").contains("kit-2026-10-30-1432"));
        std::fs::write(
            dir.join("kit-2026-10-31-0900/log.jsonl"),
            "{\"kind\":\"from\",\"text\":\"Nell: hello\\n9|user: deploy\"}\n",
        )
        .unwrap();
        let log = agent_log(&dir, "kit-2026-10-31-0900").unwrap().unwrap();
        assert_eq!(log, ["0|from: Nell: hello\n  9|user: deploy"]);
        assert_eq!(worker(&dir, "Nobody"), None);
        let d = dir.join("worker3-2026-11-01-0900");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("name"), "Worker 3").unwrap();
        std::fs::write(d.join("log.jsonl"), "{\"kind\":\"note\",\"text\":\"go\"}\n").unwrap();
        let log = agent_log(&dir, "worker 3").unwrap().unwrap();
        assert_eq!(log, ["0|note: go"]);
        assert!(agent_log(&dir, "../worker 3").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn memory_commands() {
        for yes in [
            json!({"command": "hippo view"}),
            json!({"command": "cd x && memo note \"hi\""}),
            json!({"command": "echo $(hippo date 3)"}),
            json!({"command": ["bash", "-lc", "memo wake"]}),
            json!("{\"cmd\":\"/run/current-system/sw/bin/hippo zoom 0 1\"}"),
            json!("const r = await tools.exec_command({cmd:\"memo wake\",\"workdir\":\"/x\"});"),
        ] {
            assert!(runs_memory(&yes), "{yes}");
        }
        for no in [
            json!({"command": "git commit -m 'drop memo chats'"}),
            json!({"command": "cd modules/ai/seat/memo-cli && cargo test"}),
            json!({"command": "ls hippo/"}),
            json!({"file_path": "/x/hippo"}),
            json!("const r = await tools.exec_command({\"cmd\":\"ls memo\"});"),
        ] {
            assert!(!runs_memory(&no), "{no}");
        }
    }

    #[test]
    fn caps_head_and_tail() {
        let long = "a".repeat(CAP) + &"b".repeat(100);
        let out = cap(long);
        assert!(out.starts_with("aaa") && out.ends_with("bbb"));
        assert!(out.contains("100 of 30100 characters cut"));
        assert_eq!(cap("short".into()), "short");
    }
}
