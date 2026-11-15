//! What a Libri host sends: its AG-UI events (docs.ag-ui.com) over
//! server-sent events at /api/events, and the lines of its history.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An AG-UI event as the host sends it: the fields the client reads.
/// `subagent_run_id` marks a subagent's work.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    #[serde(rename = "type")]
    pub kind: String,
    pub message_id: Option<String>,
    pub delta: Option<String>,
    pub role: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_call_name: Option<String>,
    pub content: Option<Value>,
    pub activity_type: Option<String>,
    pub patch: Option<Vec<Value>>,
    pub subagent_run_id: Option<String>,
    pub name: Option<String>,
    pub value: Option<Value>,
    pub outcome: Option<Value>,
    pub result: Option<Value>,
    pub metadata: Option<Value>,
    /// Its number in the turn's record, if the record took it.
    pub seq: Option<u64>,
    /// The row this event will be, known before the row exists.
    pub key: Option<String>,
    /// The row hippo had last settled when this event came.
    pub after: Option<String>,
}

impl Event {
    /// A flag in the event's metadata (`plain`, `aside`, `discarded`, `error`).
    pub fn flag(&self, key: &str) -> bool {
        self.metadata
            .as_ref()
            .and_then(|m| m.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// Whether a RUN_FINISHED only pauses the run for the person's answer.
    pub fn interrupted(&self) -> bool {
        self.outcome
            .as_ref()
            .and_then(|o| o.get("type"))
            .and_then(Value::as_str)
            == Some("interrupt")
    }
}

/// One message of the history as hippo keeps it (/api/history).
#[derive(Debug, Clone, Deserialize)]
pub struct Said {
    #[serde(default)]
    pub display: Option<serde_json::Value>,
    pub i: u64,
    pub kind: String,
    pub text: String,
    pub date: String,
    pub reaction: Option<String>,
    /// Where it came from, which names the row whatever hippo numbers it.
    pub src: Option<Src>,
    /// The earlier message the person's replies to.
    #[serde(default)]
    pub quote: Option<Quote>,
    /// The kind of device the person sent it from (libri/src/origin.rs).
    #[serde(default)]
    pub from: Option<String>,
}

/// The earlier message of the chat one replies to: its id in the history
/// (`h-<id>`) and its first words. The host marks the message with it and
/// the history gives it back (main.rs Quote).
#[derive(uniffi::Record, Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Quote {
    pub id: u64,
    pub text: String,
}

/// A row's origin; `e` is the key the host gave it.
#[derive(Debug, Clone, Deserialize)]
pub struct Src {
    pub e: String,
}

/// What the turn under way holds that hippo doesn't yet (/api/task `turn`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TurnRecord {
    /// The number of the last event it took.
    #[serde(default)]
    pub seq: u64,
    pub shown: Vec<Event>,
}

/// What /api/task says of the host now.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TaskNow {
    #[serde(default)]
    pub typing: bool,
    pub turn: Option<TurnRecord>,
}

/// One server-sent event: its name (`message` when it has none), its id
/// when it carries one, and its data.
#[derive(Debug, PartialEq)]
pub struct Sse {
    pub name: String,
    pub id: Option<String>,
    pub data: String,
}

/// Reads server-sent events from a stream's lines, each once its blank line
/// ends it. Comments (the host's keep-alives) and other fields are passed
/// over.
#[derive(Default)]
pub struct Decoder {
    name: Option<String>,
    id: Option<String>,
    data: Vec<String>,
}

impl Decoder {
    /// Takes one line, without its line ending; answers an event when the
    /// line ends one.
    pub fn line(&mut self, line: &str) -> Option<Sse> {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            let name = self.name.take();
            let id = self.id.take();
            if self.data.is_empty() {
                return None;
            }
            let data = self.data.join("\n");
            self.data.clear();
            return Some(Sse {
                name: name.unwrap_or_else(|| "message".into()),
                id,
                data,
            });
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value).to_owned();
        match field {
            "data" => self.data.push(value),
            "event" => self.name = Some(value),
            "id" => self.id = Some(value),
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(text: &str) -> Vec<String> {
        let mut d = Decoder::default();
        text.split('\n')
            .filter_map(|l| d.line(l))
            .map(|e| e.data)
            .collect()
    }

    #[test]
    fn an_event_carries_its_name_and_id() {
        let mut d = Decoder::default();
        let got: Vec<Sse> = "event: replay\nid: b.7\ndata: []\n\ndata: {}\n\n"
            .split('\n')
            .filter_map(|l| d.line(l))
            .collect();
        assert_eq!(
            got,
            [
                Sse {
                    name: "replay".into(),
                    id: Some("b.7".into()),
                    data: "[]".into()
                },
                Sse {
                    name: "message".into(),
                    id: None,
                    data: "{}".into()
                },
            ]
        );
    }

    #[test]
    fn events_end_at_a_blank_line_and_keep_alives_are_passed_over() {
        let got =
            events(":\n\ndata: {\"type\":\"RUN_STARTED\"}\n\n: ping\ndata:a\ndata: b\r\n\r\n");
        assert_eq!(got, ["{\"type\":\"RUN_STARTED\"}", "a\nb"]);
    }

    #[test]
    fn an_event_not_yet_ended_waits() {
        assert!(events("data: half").is_empty());
    }

    #[test]
    fn an_event_reads_its_fields_and_ignores_the_rest() {
        let e: Event = serde_json::from_str(
            r#"{"type":"TEXT_MESSAGE_END","messageId":"m","metadata":{"discarded":true},"timestamp":5}"#,
        )
        .unwrap();
        assert_eq!(e.kind, "TEXT_MESSAGE_END");
        assert_eq!(e.message_id.as_deref(), Some("m"));
        assert!(e.flag("discarded"));
        assert!(!e.flag("plain"));
    }
}
