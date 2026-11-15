//! The slash commands each harness has of its own, as it last reported
//! them, so a message that is one of them alone reaches the harness as the
//! command rather than as words in Libri's prompt: Claude Code runs a bare
//! `/compact`, `/context` or a skill's `/name` in a resumed session as it
//! would at its terminal. The page lists them in its `/` menu beside Libri's
//! own.
//!
//! A harness reports its list when a session starts (Claude Code's init
//! line); the list is kept in the state directory, `harness-commands.json`,
//! `{"<harness>": ["name", ...]}`, so the page has it before the next turn.

use serde_json::{Map, Value};
use std::path::Path;

const FILE: &str = "harness-commands.json";

fn read(state: &Path) -> Map<String, Value> {
    std::fs::read_to_string(state.join(FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// Keeps `names` as `harness`'s commands, written only when they changed.
pub fn record(state: &Path, harness: &str, names: &[String]) -> Result<(), String> {
    let mut all = read(state);
    let now = Value::from(names.to_vec());
    if all.get(harness) == Some(&now) {
        return Ok(());
    }
    all.insert(harness.into(), now);
    crate::task::put(&state.join(FILE), &Value::Object(all).to_string())
}

/// The harnesses that report commands of their own; the rest have none
/// Libri can send them (Codex's app-server, OpenCode, omp, Pi, Muse and
/// ACP agents as Libri drives them today).
const REPORTING: [&str; 1] = ["claude"];

/// Whether `harness`'s list is known: it has none to report, or it has
/// reported them. Until then a command is given to it as words.
pub fn known(state: &Path, harness: &str) -> bool {
    !REPORTING.contains(&harness) || read(state).contains_key(harness)
}

/// Every harness's commands as last reported.
pub fn all(state: &Path) -> Map<String, Value> {
    read(state)
}

/// `harness`'s commands as last reported; empty before it has reported.
pub fn of(state: &Path, harness: &str) -> Vec<String> {
    read(state)
        .get(harness)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|n| n.as_str().map(str::to_owned))
        .collect()
}

/// The command `text` is, when it is one alone: `/name` and its arguments on
/// a single line.
pub fn named(text: &str) -> Option<&str> {
    let text = text.trim();
    let rest = text.strip_prefix('/')?;
    if text.contains('\n') {
        return None;
    }
    let name = rest.split_whitespace().next()?;
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
        .then_some(name)
}

/// Whether `text` is one of `harness`'s own commands alone, to be sent to it
/// as is.
pub fn is_harness_command(state: &Path, harness: &str, text: &str) -> bool {
    named(text).is_some_and(|name| of(state, harness).iter().any(|n| n == name))
}

/// The commands a Claude Code init line lists.
pub fn claude_init(line: &str) -> Option<Vec<String>> {
    if !line.contains("slash_commands") {
        return None;
    }
    let ev = serde_json::from_str::<Value>(line).ok()?;
    if ev["type"] != "system" || ev["subtype"] != "init" {
        return None;
    }
    Some(
        ev["slash_commands"]
            .as_array()?
            .iter()
            .filter_map(|n| n.as_str().map(str::to_owned))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "libri-commands-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_command_alone_is_one_and_a_sentence_is_not() {
        assert_eq!(named("/compact"), Some("compact"));
        assert_eq!(named("  /code-review high  "), Some("code-review"));
        assert_eq!(
            named("/anthropic-skills:pdf a.pdf"),
            Some("anthropic-skills:pdf")
        );
        assert_eq!(named("compact"), None);
        assert_eq!(named("/compact\nand more"), None);
        assert_eq!(named("/path/to/file"), None);
        assert_eq!(named("/"), None);
    }

    #[test]
    fn a_harness_command_is_one_its_harness_reported() {
        let s = scratch();
        assert!(!is_harness_command(&s, "claude", "/compact"));
        let line = r#"{"type":"system","subtype":"init","slash_commands":["compact","context","simplify"]}"#;
        record(&s, "claude", &claude_init(line).unwrap()).unwrap();
        assert!(is_harness_command(&s, "claude", "/compact"));
        assert!(is_harness_command(&s, "claude", "/simplify the diff"));
        assert!(!is_harness_command(&s, "codex", "/compact"));
        assert!(!is_harness_command(&s, "claude", "/stop the car"));
        assert_eq!(
            claude_init(r#"{"type":"assistant","slash_commands":[]}"#),
            None
        );
    }
}
