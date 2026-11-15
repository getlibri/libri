//! Which kind of device a person's message came from: the Android app, a
//! browser on a phone or a desktop, the desktop app, or another program. A
//! device kind only, never where it connects from. A call's words carry
//! their own mark, "(call)" (call.rs).
//!
//! The model hears it as a mark before the words, and only when it changes
//! from the chat's last message: a chat typed from one phone costs nothing
//! after its first message. The mark is kept with the words, so the record
//! (and hippo2, which reads it) has it too, and the page reads each
//! message's device from the last mark at or before it.

use axum::http::{HeaderMap, header};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The mark's start; the kind and a closing ") " follow.
const FROM: &str = "(from ";

/// Every kind there is, as the mark and the page name them.
pub const KINDS: &[&str] = &["android", "desktop", "web-phone", "web-desktop", "other"];

/// The kind of device behind a request, from its user agent. Libri's own
/// client says what it runs on (`libri-android/…`, `libri-desktop/…`); a
/// browser on a phone says "Mobi" in its agent, as every phone browser does
/// and as MDN advises reading it.
pub fn of(headers: &HeaderMap) -> &'static str {
    let agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    kind_of(agent)
}

fn kind_of(agent: &str) -> &'static str {
    if agent.starts_with("libri-android/") {
        "android"
    } else if agent.starts_with("libri-desktop/") {
        "desktop"
    } else if agent.starts_with("Mozilla/") && agent.contains("Mobi") {
        "web-phone"
    } else if agent.starts_with("Mozilla/") {
        "web-desktop"
    } else {
        "other"
    }
}

/// Serialises the read and write of a chat's last kind: two messages from
/// two devices at once must not both see the old one.
static LAST: Mutex<()> = Mutex::new(());

fn file(state: &Path, chat: &str) -> PathBuf {
    state.join("origin").join(chat)
}

/// The mark for a message of `kind` in `chat`: "(from android) " when the
/// chat's last message came from another kind of device, or is its first;
/// "" when it came from the same. Records `kind` as the chat's last.
pub fn mark(state: &Path, chat: &str, kind: &str) -> String {
    // A chat's name may come from a request's path: one that could leave
    // the folder is marked every time rather than kept.
    if chat.is_empty() || chat.starts_with('.') || chat.contains(['/', '\\']) {
        return format!("{FROM}{kind}) ");
    }
    let _held = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let path = file(state, chat);
    if std::fs::read_to_string(&path).is_ok_and(|last| last == kind) {
        return String::new();
    }
    // A failed write only costs a mark repeated on the next message.
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, kind);
    format!("{FROM}{kind}) ")
}

/// The device kind a message's words open with, and the words after it.
pub fn read(text: &str) -> (Option<&'static str>, &str) {
    let Some(rest) = text.strip_prefix(FROM) else {
        return (None, text);
    };
    for kind in KINDS {
        if let Some(words) = rest.strip_prefix(kind).and_then(|r| r.strip_prefix(") ")) {
            return (Some(kind), words);
        }
    }
    (None, text)
}

/// The words without their mark.
pub fn strip(text: &str) -> &str {
    read(text).1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_name_their_kind() {
        assert_eq!(kind_of("libri-android/0.9.70"), "android");
        assert_eq!(kind_of("libri-desktop/0.9.70"), "desktop");
        assert_eq!(
            kind_of(
                "Mozilla/5.0 (Linux; Android 16; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Mobile Safari/537.36"
            ),
            "web-phone"
        );
        assert_eq!(
            kind_of(
                "Mozilla/5.0 (iPhone; CPU iPhone OS 26_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.1 Mobile/15E148 Safari/604.1"
            ),
            "web-phone"
        );
        assert_eq!(
            kind_of("Mozilla/5.0 (X11; Linux x86_64; rv:145.0) Gecko/20100101 Firefox/145.0"),
            "web-desktop"
        );
        assert_eq!(kind_of("curl/8.17.0"), "other");
        assert_eq!(kind_of(""), "other");
    }

    #[test]
    fn the_mark_comes_only_when_the_device_changes() {
        let state = std::env::temp_dir().join(format!("libri-origin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state);
        let s = state.as_path();
        assert_eq!(mark(s, "main", "android"), "(from android) ");
        assert_eq!(mark(s, "main", "android"), "");
        assert_eq!(mark(s, "main", "android"), "");
        assert_eq!(mark(s, "main", "web-desktop"), "(from web-desktop) ");
        assert_eq!(mark(s, "main", "web-desktop"), "");
        assert_eq!(mark(s, "main", "android"), "(from android) ");
        // Each chat keeps its own.
        assert_eq!(mark(s, "temp", "android"), "(from android) ");
        // A name that could leave the folder keeps nothing.
        assert_eq!(mark(s, "../x", "android"), "(from android) ");
        assert_eq!(mark(s, "../x", "android"), "(from android) ");
        assert!(!s.join("x").exists());
        let _ = std::fs::remove_dir_all(&state);
    }

    #[test]
    fn a_mark_reads_back_and_strips() {
        assert_eq!(read("(from android) hi"), (Some("android"), "hi"));
        assert_eq!(
            read("(from web-phone) /compact"),
            (Some("web-phone"), "/compact")
        );
        assert_eq!(read("hi"), (None, "hi"));
        // Words that only look like one stay words.
        assert_eq!(
            read("(from the store) milk"),
            (None, "(from the store) milk")
        );
        assert_eq!(strip("(from desktop) x"), "x");
    }
}
