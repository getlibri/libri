//! Every setting the person can change, in one list: where it applies (the
//! host, so every device on it sees it, or each device alone, so it stays
//! put when the app switches hosts), what it does, the values it takes and
//! how it is changed. The app and the assistant read the same list, the
//! assistant before it changes one.
//!
//! A host keeps each setting in a file of its own and only once it is
//! changed: an unset one is its default, one bad value leaves the rest as
//! they are, and a build that does not know a file leaves it alone. The
//! deployment's own config (host.rs KEYS) is changed outside the app; it is
//! documented beside these, so the assistant knows what a host is set to do
//! and can say what to change. `markdown` is the page `docs/settings.md`
//! keeps and the host serves at /docs/settings.md, as Tern publishes its own.

use std::path::Path;

use serde::Serialize;

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Kept on the host: every device on it shares it.
    Host,
    /// Kept on each device: it follows the device across hosts.
    Device,
}

#[derive(Serialize, Debug)]
pub struct Setting {
    pub id: &'static str,
    pub scope: Scope,
    pub about: &'static str,
    /// The values it takes.
    pub takes: &'static str,
    /// What holds while it is unset.
    pub default: &'static str,
    /// How it is changed: the host's call, or the device's own control.
    pub change: &'static str,
    /// A host setting's file in the state directory.
    #[serde(skip)]
    pub file: Option<&'static str>,
}

const fn host(
    id: &'static str,
    file: &'static str,
    about: &'static str,
    takes: &'static str,
    default: &'static str,
    change: &'static str,
) -> Setting {
    Setting {
        id,
        scope: Scope::Host,
        about,
        takes,
        default,
        change,
        file: Some(file),
    }
}

const fn device(
    id: &'static str,
    about: &'static str,
    takes: &'static str,
    default: &'static str,
    change: &'static str,
) -> Setting {
    Setting {
        id,
        scope: Scope::Device,
        about,
        takes,
        default,
        change,
        file: None,
    }
}

pub const ALL: &[Setting] = &[
    host(
        "model",
        crate::catalog::PICK.0,
        "The main chat's model, and with it its harness, from the next turn.",
        "a {harness, model} of GET /api/models",
        "the default harness's default model",
        "POST /api/model {harness, model}",
    ),
    host(
        "effort",
        crate::model::EFFORT.0,
        "How hard the main chat's model thinks, from the next turn.",
        "one of the levels GET /api/models lists for the model",
        "the harness's own default",
        "POST /api/effort {effort}",
    ),
    host(
        "access",
        crate::access::PICKED.0,
        "How freely the main chat's harness acts: its own permission mode and sandbox. Holds only on the harness it was picked on.",
        "the modes GET /api/models lists for the harness",
        "the harness's default (access.defaults)",
        "POST /api/access {access?, sandbox?}",
    ),
    host(
        "access.defaults",
        crate::access::DEFAULTS.0,
        "What each harness starts on, by harness id.",
        "per harness, one of its modes and sandboxes",
        "each harness's own default",
        "POST /api/access/default {harness, access?, sandbox?}",
    ),
    host(
        "harnesses.off",
        crate::catalog::OFF.0,
        "The harnesses switched off; one off is offered nowhere.",
        "harness ids installed here, one per line",
        "the ones off at first",
        "POST /api/harness {harness, on}",
    ),
    host(
        "harness.default",
        crate::catalog::DEFAULT.0,
        "The harness a new chat and a new worker run on.",
        "an installed harness that is switched on",
        "the deployment's harness.default",
        "POST /api/harness {harness, default: true}",
    ),
    host(
        "harness.models",
        crate::catalog::DEFAULT_MODELS.0,
        "Each harness's default model, by harness id.",
        "per harness, one of its models in GET /api/models",
        "each harness's own default",
        "POST /api/harness {harness, model}",
    ),
    host(
        "others",
        crate::catalog::OTHER.0,
        "The harness that runs other makers' models.",
        "an installed harness that reaches other makers",
        "the first that does",
        "POST /api/others {others}",
    ),
    host(
        "temp.model",
        crate::catalog::TEMP_PICK.0,
        "Temp's model, apart from the main chat's.",
        "a {harness, model} of GET /api/models",
        "the main chat's",
        "POST /api/model {harness, model, temp: true}",
    ),
    host(
        "temp.effort",
        crate::catalog::TEMP_EFFORT.0,
        "How hard Temp's model thinks.",
        "one of the levels GET /api/models lists for Temp's model",
        "the harness's own default",
        "POST /api/effort {effort, temp: true}",
    ),
    host(
        "temp.memory",
        crate::scratchpad::MEMORY.0,
        "Whether Temp reads the memory. Off, it holds the web tools alone and no rules.",
        "on, off",
        "on",
        "POST /api/scratchpad/memory {on}",
    ),
    host(
        "worker.model",
        crate::catalog::WORKER_PICK.0,
        "The model a new worker starts on.",
        "a {harness, model} of GET /api/models",
        "the main chat's",
        "POST /api/worker-model {harness, model}",
    ),
    host(
        "worker.names",
        crate::names::LIST.0,
        "The names new workers are drawn from.",
        "a list of names",
        "Libri's own list",
        "POST /api/worker-names",
    ),
    host(
        "synopsis.model",
        crate::catalog::SYNOPSIS_PICK.0,
        "The model that writes a chat's title and a worker's one-line job.",
        "a {harness, model} of GET /api/models",
        "a small model on the default harness",
        "POST /api/synopsis {harness, model}",
    ),
    host(
        "image.model",
        crate::capability::IMAGE.0,
        "The model that makes pictures.",
        "one of GET /api/capabilities's image choices",
        "the first on offer",
        "POST /api/capabilities {capability: \"image\", provider, model}",
    ),
    host(
        "voice",
        crate::site::VOICE.0,
        "The voice replies are read aloud and calls held in.",
        "one of GET /api/voices",
        "the deployment's x.voice",
        "POST /api/voice {voice}",
    ),
    host(
        "forget",
        crate::forget::FILE,
        "How long a Libri that forgets keeps what is said, in minutes. Only where the deployment sets tools.forget.",
        "10, 30, 60, 180, 720, 1440, 4320, 10080",
        "the deployment's tools.forget",
        "POST /api/forget {minutes}",
    ),
    host(
        "compact.at",
        crate::sessions::COMPACT_AT.0,
        "How large a chat's session grows, in tokens, before its harness compacts it: the main chat, Temp, memory chats and workers alike (Claude Code and Codex; omp, Pi and OpenCode keep their own rule).",
        "a whole number of tokens, 20000 or more",
        "the deployment's compact.at, else 200000",
        "POST /api/compact {tokens} (null for the default)",
    ),
    host(
        "usage.shown",
        crate::usage::SHOWN.0,
        "Whether every composer shows the subscription usage rings.",
        "on, off",
        "on",
        "POST /api/usage {shown}",
    ),
    host(
        "rules",
        crate::guide::RULES.0,
        "The person's rules (AGENTS.md), added to every prompt but memory-off Temp's.",
        "text up to 64 KB",
        "none",
        "POST /api/guide {text}",
    ),
    host(
        "host_facts",
        crate::guide::HOST.0,
        "What is true of this host alone (HOST.md), added to every prompt after the person's rules, but a named memory's and memory-off Temp's.",
        "text up to 64 KB",
        "none",
        "POST /api/guide/host {text}",
    ),
    device(
        "theme",
        "Light or dark.",
        "light, dark; on Android also system",
        "light on the web, system on Android",
        "the theme switch in Settings",
    ),
    device(
        "text.size",
        "The talk's text size; the page around it keeps its own.",
        "13 to 29 px on the web (its pinch, 0.8 to 1.8 times 16 px); 12 to 24 px on Android",
        "16 px",
        "pinch, or the size stepper in Settings",
    ),
    device(
        "voice.speed",
        "How fast the assistant speaks read aloud and on calls.",
        "0.5 to 3 in steps of 0.1; a call takes 0.7 to 1.5",
        "1",
        "the speed control in Settings",
    ),
    device(
        "sides",
        "The width of each side column on a wide screen.",
        "240 to 480 px each",
        "390 px",
        "drag a column's edge",
    ),
    device(
        "pins",
        "The chats and pages pinned, and whether the pinned list shows.",
        "a list; shown or hidden",
        "none; shown",
        "pin or unpin",
    ),
    device(
        "hosts",
        "The hosts this device knows and the one open.",
        "host addresses",
        "the host that served the page",
        "Settings, Hosts",
    ),
];

/// A host setting's value as kept: JSON when it reads as JSON, else its
/// text; None while unset.
fn value(state: &Path, file: &str) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(state.join(file)).ok()?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(serde_json::from_str(text).unwrap_or_else(|_| text.into()))
}

/// The list as the app reads it: each host setting with its value here,
/// null while it holds its default.
pub fn listed(state: &Path) -> serde_json::Value {
    ALL.iter()
        .map(|s| {
            let mut v = serde_json::to_value(s).unwrap_or_default();
            if let Some(file) = s.file {
                v["value"] = value(state, file).unwrap_or_default();
            }
            v
        })
        .collect()
}

/// The list as the assistant reads it.
pub fn text(state: &Path) -> String {
    use std::fmt::Write;
    let mut out = String::from(
        "Each setting: where it applies (host: every device on this host; device: each device alone, so it is changed there, not by you), what it does, what it takes, its default, how it is changed, and its value here.\n",
    );
    for s in ALL {
        let scope = match s.scope {
            Scope::Host => "host",
            Scope::Device => "device",
        };
        let _ = write!(
            out,
            "\n{} ({scope}): {}\n  takes: {}\n  default: {}\n  change: {}\n",
            s.id, s.about, s.takes, s.default, s.change
        );
        if let Some(file) = s.file {
            let now = value(state, file).map_or_else(|| "default".into(), |v| v.to_string());
            let _ = writeln!(out, "  now: {now}");
        }
    }
    out.push_str(&config_text());
    out
}

/// Every setting and config key as a Markdown page: what `docs/settings.md`
/// holds and the host serves. No values, so it reads the same anywhere.
pub fn markdown() -> String {
    use std::fmt::Write;
    let cell = |t: &str| t.replace('|', "\\|");
    let mut out = String::from(
        "# Libri settings\n\nGenerated from `libri/src/settings.rs` and `libri/src/host.rs`; `cargo test` fails while this page differs.\n\n## In the app\n\nHost settings are kept on the host, so every device on it shares them; device settings stay on each device.\n\n| Setting | Where | What it does | Takes | Default | Changed by |\n|---|---|---|---|---|---|\n",
    );
    for s in ALL {
        let scope = match s.scope {
            Scope::Host => "host",
            Scope::Device => "device",
        };
        let _ = writeln!(
            out,
            "| `{}` | {scope} | {} | {} | {} | {} |",
            s.id,
            cell(s.about),
            cell(s.takes),
            cell(s.default),
            cell(s.change)
        );
    }
    out.push_str(
        "\n## The deployment's config\n\nTOML, read from `--config FILE`, else `$XDG_CONFIG_HOME/libri/config.toml`. Every key is optional; an unknown key is an error. A setting already in the environment wins over the file.\n\n| Key | Environment | What it does |\n|---|---|---|\n",
    );
    for k in crate::host::KEYS {
        let _ = writeln!(out, "| `{}` | {} | {} |", k.key, k.env, cell(k.about));
    }
    out
}

/// The deployment's config as the assistant reads it: each key and whether
/// this host sets it. Values stay out, since some name keys and logins.
fn config_text() -> String {
    use std::fmt::Write;
    let mut out = String::from(
        "\nThe deployment's config, changed outside the app (the person edits the host's config file or its NixOS module; you cannot), each with whether this host sets it:\n",
    );
    for k in crate::host::KEYS {
        let set = std::env::var_os(k.env).is_some_and(|v| !v.is_empty());
        let _ = writeln!(
            out,
            "\n{} ({}, {}): {}",
            k.key,
            k.env,
            if set { "set" } else { "unset" },
            k.about
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_setting_is_named_once_and_host_ones_by_their_file() {
        let mut ids: Vec<_> = ALL.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), ALL.len());
        let mut files: Vec<_> = ALL.iter().filter_map(|s| s.file).collect();
        files.sort_unstable();
        files.dedup();
        assert_eq!(
            files.len(),
            ALL.iter().filter(|s| s.scope == Scope::Host).count()
        );
    }

    #[test]
    fn the_kept_page_is_the_list() {
        assert!(
            include_str!("../docs/settings.md") == markdown(),
            "libri/docs/settings.md is stale: run `libri docs > libri/docs/settings.md`"
        );
    }

    #[test]
    fn reads_what_is_kept() {
        let dir = std::env::temp_dir().join(format!("libri-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::usage::show(&dir, false).unwrap();
        let all = listed(&dir);
        let of = |id: &str| {
            all.as_array()
                .unwrap()
                .iter()
                .find(|s| s["id"] == id)
                .unwrap()
                .clone()
        };
        assert_eq!(of("usage.shown")["value"], "off");
        assert_eq!(of("effort")["value"], serde_json::Value::Null);
        assert_eq!(of("theme")["scope"], "device");
        assert!(of("theme").get("value").is_none());
        let t = text(&dir);
        assert!(t.contains("usage.shown (host)"));
        assert!(t.contains("  now: \"off\""));
        assert!(t.contains("daemon.tailnet (LIBRI_TAILNET, "));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
