//! What a tile can show, declared once: each kind's name, what its `arg`
//! names, where it runs, whether it opens more than once and how a new one
//! is told apart, and the rows its menu adds. The host serves the list
//! (`/api/tiles`); the layout tools take their kinds and their words from it;
//! the web page, the Android app and the desktop app each keep only a view
//! per kind and the code behind each row, and nix/check_tile_coverage.py
//! holds them to it (`libri tiles` prints the list).
//! A new kind is a line here and a view in each app that runs it.

use serde::Serialize;
use serde_json::{Value, json};

/// What a kind's `arg` names, if anything.
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "takes")]
pub enum Arg {
    /// No arg.
    None,
    /// An arg it may have, `about` saying what.
    May { about: &'static str },
    /// An arg it must have.
    Must { about: &'static str },
}

/// A row a kind adds to its tile's menu: the apps run `id`, and show
/// `label` and `keys` as given here.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Row {
    pub id: &'static str,
    pub label: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keys: Option<&'static str>,
}

/// Where a kind runs.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Runs {
    /// A tile on a wide screen (the web page's tiles, the desktop app).
    pub wide: bool,
    /// An item on a phone's bar (the web page on a phone, the Android app).
    pub phone: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Tile {
    pub kind: &'static str,
    pub name: &'static str,
    /// The glyph the apps draw it by (each app's own set).
    pub icon: &'static str,
    /// For the agent: what it is, in a few words.
    pub about: &'static str,
    pub arg: Arg,
    pub runs: Runs,
    /// Whether it opens again beside an open one, each its own.
    pub many: bool,
    /// Whether the dock and the phone's plus offer it (the rest open from a
    /// link, a card or the launcher).
    pub offered: bool,
    /// A second one beside an open one gets this arg, a fresh name made
    /// from it (`terminal`: a shell of its own) or the word itself (`board`:
    /// a board of its own); none, it shows the same as the first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fresh: Option<Fresh>,
    /// What its pane's `tabs` hold, if it keeps several in one tile, as a
    /// list of args: each a tab, in order, the one shown its `arg` ("" for
    /// none).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tabs: Option<&'static str>,
    pub rows: &'static [Row],
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "is")]
pub enum Fresh {
    /// A new name, this prefix and a unique tail.
    Name { prefix: &'static str },
    /// This word, which the view takes for "make one of your own".
    Word { word: &'static str },
}

const BOTH: Runs = Runs {
    wide: true,
    phone: true,
};

/// Every kind, in the order the apps offer them.
pub const TILES: &[Tile] = &[
    Tile {
        kind: "chat",
        name: "Chat",
        icon: "chat",
        about: "the main chat or a named memory's chat, several as tabs in one tile",
        arg: Arg::May {
            about: "a named memory's id; none for the main chat",
        },
        runs: BOTH,
        offered: true,
        many: true,
        fresh: None,
        tabs: Some("the memories open in the tile, \"\" for Main"),
        rows: &[
            Row {
                id: "search",
                label: "Search in chat",
                keys: Some("Ctrl K"),
            },
            Row {
                id: "copy-answer",
                label: "Copy last answer",
                keys: None,
            },
            Row {
                id: "new-temp",
                label: "New Temp",
                keys: None,
            },
        ],
    },
    Tile {
        kind: "temp",
        name: "Temp chat",
        icon: "temp",
        about: "a temp chat, kept out of the memory; each Temp tile is a chat of its own",
        arg: Arg::May {
            about: "the Temp's id (`scratchpad-…`); none opens a new one",
        },
        runs: BOTH,
        offered: true,
        many: true,
        fresh: Some(Fresh::Word { word: "new" }),
        tabs: None,
        rows: &[Row {
            id: "promote",
            label: "Create task from it",
            keys: None,
        }],
    },
    Tile {
        kind: "terminal",
        name: "Terminal",
        icon: "terminal",
        about: "a shell on this host; the one with no arg is the one terminal_read and terminal_send reach",
        arg: Arg::May {
            about: "the shell's name; none for the shared shell",
        },
        runs: BOTH,
        offered: true,
        many: true,
        fresh: Some(Fresh::Name { prefix: "t" }),
        tabs: None,
        rows: &[
            Row {
                id: "paste",
                label: "Paste",
                keys: Some("Ctrl Shift V"),
            },
            Row {
                id: "clear",
                label: "Clear screen and scrollback",
                keys: None,
            },
            Row {
                id: "restart",
                label: "Restart shell",
                keys: None,
            },
        ],
    },
    Tile {
        kind: "thread",
        name: "Worker",
        icon: "thread",
        about: "a worker's thread",
        arg: Arg::Must {
            about: "the worker's id",
        },
        runs: BOTH,
        offered: false,
        many: false,
        fresh: None,
        tabs: None,
        rows: &[
            Row {
                id: "stop",
                label: "Stop",
                keys: None,
            },
            Row {
                id: "archive",
                label: "Archive",
                keys: None,
            },
        ],
    },
    Tile {
        kind: "library",
        name: "Library",
        icon: "library",
        about: "the Library: its gallery, a page or a list",
        arg: Arg::May {
            about: "`page:<name>` or `list:<name>`; none for the gallery",
        },
        runs: BOTH,
        offered: true,
        many: true,
        fresh: None,
        tabs: None,
        rows: &[
            Row {
                id: "reload",
                label: "Reload",
                keys: None,
            },
            Row {
                id: "window",
                label: "Open in window",
                keys: None,
            },
        ],
    },
    Tile {
        kind: "board",
        name: "Whiteboard",
        icon: "board",
        about: "a whiteboard",
        arg: Arg::May {
            about: "the board's id; none for the first board",
        },
        runs: BOTH,
        offered: true,
        many: true,
        fresh: Some(Fresh::Word { word: "new" }),
        tabs: None,
        rows: &[
            Row {
                id: "new-board",
                label: "New board",
                keys: None,
            },
            Row {
                id: "export",
                label: "Export…",
                keys: None,
            },
        ],
    },
    Tile {
        kind: "browser",
        name: "Browser",
        icon: "browser",
        about: "Libri's own browser, the one shared with the agents",
        arg: Arg::None,
        runs: BOTH,
        offered: true,
        many: false,
        fresh: None,
        tabs: None,
        rows: &[],
    },
    Tile {
        kind: "system",
        name: "System",
        icon: "system",
        about: "the host's CPU, memory, disks and workers",
        arg: Arg::None,
        runs: BOTH,
        offered: true,
        many: false,
        fresh: None,
        tabs: None,
        rows: &[],
    },
    Tile {
        kind: "builtin",
        name: "Built-in",
        icon: "builtin",
        about: "a built-in place, such as the calendar",
        arg: Arg::Must {
            about: "the built-in's id",
        },
        runs: BOTH,
        offered: false,
        many: false,
        fresh: None,
        tabs: None,
        rows: &[],
    },
    Tile {
        kind: "pinned",
        name: "Pinned",
        icon: "pinned",
        about: "the pinned messages",
        arg: Arg::None,
        runs: BOTH,
        offered: false,
        many: false,
        fresh: None,
        tabs: None,
        rows: &[],
    },
    Tile {
        kind: "home",
        name: "Today",
        icon: "home",
        about: "today at a glance",
        arg: Arg::None,
        runs: Runs {
            wide: true,
            phone: false,
        },
        offered: false,
        many: false,
        fresh: None,
        tabs: None,
        rows: &[],
    },
];

/// The rows every wide tile's menu ends with, after its kind's.
pub const TILE_ROWS: &[Row] = &[
    Row {
        id: "split-right",
        label: "Split right",
        keys: Some("Alt Shift \\"),
    },
    Row {
        id: "split-down",
        label: "Split down",
        keys: Some("Alt Shift -"),
    },
    Row {
        id: "move",
        label: "Move to workspace…",
        keys: None,
    },
    Row {
        id: "close",
        label: "Close tile",
        keys: Some("Alt Shift X"),
    },
];
/// The rows every phone bar item's menu ends with.
pub const BAR_ROWS: &[Row] = &[
    Row {
        id: "move",
        label: "Move to workspace…",
        keys: None,
    },
    Row {
        id: "edit-bar",
        label: "Edit the bar",
        keys: None,
    },
    Row {
        id: "close",
        label: "Close",
        keys: None,
    },
];

pub fn get(kind: &str) -> Option<&'static Tile> {
    TILES.iter().find(|t| t.kind == kind)
}

/// The kinds a wide tile, or with `phone` a bar item, can show.
pub fn kinds(phone: bool) -> Vec<&'static str> {
    TILES
        .iter()
        .filter(|t| if phone { t.runs.phone } else { t.runs.wide })
        .map(|t| t.kind)
        .collect()
}

/// What `/api/tiles` answers.
pub fn listed() -> Value {
    json!({"tiles": TILES, "tile_rows": TILE_ROWS, "bar_rows": BAR_ROWS})
}

/// The kinds and their args, for the agent: one line each.
pub fn told() -> String {
    TILES
        .iter()
        .map(|t| {
            let arg = match t.arg {
                Arg::None => String::new(),
                Arg::May { about } => format!(" (arg: {about})"),
                Arg::Must { about } => format!(" (arg, needed: {about})"),
            };
            let tabs = t.tabs.map_or(String::new(), |about| {
                format!(" (tabs: {about}; arg is the tab shown)")
            });
            let place = match (t.runs.wide, t.runs.phone) {
                (true, false) => ", wide screens only",
                (false, true) => ", phones only",
                _ => "",
            };
            format!("{}: {}{arg}{tabs}{place}", t.kind, t.about)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// A pane's kind and arg as the layout may hold them, or why not.
pub fn pane_fits(kind: &str, arg: Option<&str>, phone: bool) -> Result<(), String> {
    let t = get(kind)
        .filter(|t| if phone { t.runs.phone } else { t.runs.wide })
        .ok_or_else(|| {
            format!(
                "`{kind}` is no kind; a pane's kind is one of {}.",
                kinds(phone).join(", ")
            )
        })?;
    if matches!(t.arg, Arg::Must { .. }) && arg.is_none() {
        return Err(format!("A {kind} needs `arg`."));
    }
    Ok(())
}

/// A pane's `tabs`, if it has them: a list of args for a kind that keeps
/// tabs, holding the one shown, none twice.
pub fn tabs_fit(pane: &Value) -> Result<(), String> {
    let Some(tabs) = pane.get("tabs") else {
        return Ok(());
    };
    let kind = pane.get("kind").and_then(Value::as_str).unwrap_or("");
    if get(kind).and_then(|t| t.tabs).is_none() {
        return Err(format!("A {kind} keeps no `tabs`."));
    }
    let tabs = tabs
        .as_array()
        .filter(|t| !t.is_empty() && t.iter().all(Value::is_string))
        .ok_or("`tabs` is a list of at least one arg.")?;
    let mut seen = std::collections::HashSet::new();
    if !tabs.iter().all(|t| seen.insert(t.as_str())) {
        return Err("`tabs` holds an arg twice.".into());
    }
    let shown = pane.get("arg").and_then(Value::as_str).unwrap_or("");
    if !tabs.iter().any(|t| t == shown) {
        return Err(format!("`tabs` lacks the tab shown, `{shown}`."));
    }
    Ok(())
}

/// Writes the registry's kinds into the layout tools' words and `tile_open`'s
/// schema, as `web::offer` writes the engines into `web_search`'s.
pub fn offer<S>(router: &mut rmcp::handler::server::router::tool::ToolRouter<S>) {
    let kinds = told();
    for name in ["layout_change", "tile_open"] {
        let Some(route) = router.map.get_mut(name) else {
            continue;
        };
        let mut schema = (*route.attr.input_schema).clone();
        if let Some(Value::Object(props)) = schema.get_mut("properties") {
            if let Some(Value::Object(layout)) = props.get_mut("layout") {
                let said = layout
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                layout.insert(
                    "description".into(),
                    format!("{said} Kinds: {kinds}.").into(),
                );
            }
            if let Some(Value::Object(kind)) = props.get_mut("kind") {
                let all: Vec<_> = TILES.iter().map(|t| t.kind).collect();
                kind.insert("enum".into(), json!(all));
                kind.insert(
                    "description".into(),
                    format!("What it shows. {kinds}.").into(),
                );
            }
        }
        route.attr.input_schema = std::sync::Arc::new(schema);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_follow_where_they_run() {
        assert!(kinds(false).contains(&"home"));
        assert!(!kinds(true).contains(&"home"));
        assert!(pane_fits("thread", None, false).is_err());
        assert!(pane_fits("workers", None, false).is_err());
        assert!(pane_fits("terminal", Some("t1"), true).is_ok());
    }

    #[test]
    fn a_chat_keeps_its_memories_as_tabs() {
        assert!(tabs_fit(&json!({"kind": "chat", "arg": "tutor", "tabs": ["", "tutor"]})).is_ok());
        assert!(tabs_fit(&json!({"kind": "chat", "tabs": [""]})).is_ok());
        assert!(tabs_fit(&json!({"kind": "chat", "arg": "jobs", "tabs": [""]})).is_err());
        assert!(tabs_fit(&json!({"kind": "chat", "tabs": ["", ""]})).is_err());
        assert!(tabs_fit(&json!({"kind": "chat", "tabs": []})).is_err());
        assert!(tabs_fit(&json!({"kind": "board", "arg": "b", "tabs": ["b"]})).is_err());
        assert!(told().contains("tabs: the memories open in the tile"));
    }
}
