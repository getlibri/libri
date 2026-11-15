//! Each harness's own permission modes and sandbox settings, passed through
//! under their own names: Libri has no modes of its own and builds no
//! sandbox. The picker shows those of the harness in use, Settings holds a
//! default for each harness, and the harness takes the mode as it stands. A
//! choice marked hidden is one the harness's own picker leaves out: Libri's
//! does too, unless it is in force, and a slash command reaches it.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::Kept;

/// The old pick of "auto" or "full", from before the harnesses' own modes.
/// It is only read, as what a harness runs on until the person says, so a
/// host rolled back finds it as it was.
pub const LEGACY: Kept = Kept("access");

/// The main chat's pick, which holds only on the harness it was made on.
pub const PICKED: Kept = Kept("access-picked");
const TEMP_PICKED: Kept = Kept("temp-access-picked");

/// The default of each harness, by its id.
pub const DEFAULTS: Kept = Kept("access-defaults");

/// The modes each ACP agent declared at its last session/new: an agent lists
/// them only there, and the picker needs them before a turn.
pub const ACP_MODES: Kept = Kept("acp-modes");

/// An ACP agent's own mode goes by this prefix and its id.
pub const ACP_MODE: &str = "mode:";

/// A choice of the harness's: its own id, name and what it means here.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Mode {
    pub id: String,
    pub name: String,
    pub note: String,
    /// Nothing asks or bounds it: the picker fills its shield.
    pub open: bool,
    /// Taken but not listed in the composer's picker. Absent from what an
    /// ACP agent declared and was kept.
    #[serde(default)]
    pub hidden: bool,
}

/// A pick's two halves; None leaves each to what stands under it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Picks {
    #[serde(default)]
    pub access: Option<String>,
    #[serde(default)]
    pub sandbox: Option<String>,
}

/// What a turn runs on, by the harness's own ids. `sandbox` is None on a
/// harness with no sandbox of its own.
#[derive(Clone, Debug, PartialEq)]
pub struct Grant {
    pub mode: String,
    pub sandbox: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct MainPick {
    harness: String,
    #[serde(flatten)]
    picks: Picks,
}

fn mode(id: &str, name: &str, note: &str, open: bool) -> Mode {
    Mode {
        id: id.into(),
        name: name.into(),
        note: note.into(),
        open,
        hidden: false,
    }
}

fn hidden(id: &str, name: &str, note: &str, open: bool) -> Mode {
    Mode {
        hidden: true,
        ..mode(id, name, note, open)
    }
}

/// The modes a harness takes, in the order its own picker gives them.
pub fn access_modes(harness: &str, state: &Path) -> Vec<Mode> {
    let m = mode;
    match harness {
        // `claude --help`, 2.1.303; "default" is now "manual". Shift+Tab
        // cycles the first four, and bypass with them in a session started
        // with --allow-dangerously-skip-permissions, as every acting turn
        // is. dontAsk never joins the cycle.
        "claude" => vec![
            m("manual", "Manual", "Asks before edits and commands", false),
            m(
                "acceptEdits",
                "Accept edits",
                "Edits and file commands in the project run; asks before the rest",
                false,
            ),
            m("plan", "Plan", "Reads and plans; changes nothing", false),
            m(
                "auto",
                "Auto",
                "A classifier decides each action; it rarely asks",
                false,
            ),
            // Codex's word for its own open setting, so the two read alike.
            m("bypassPermissions", "Full access", "Asks nothing", true),
            hidden(
                "dontAsk",
                "Don't ask",
                "Runs what is already allowed and refuses the rest",
                false,
            ),
        ],
        // AskForApproval, and the --approve-for-me preset: on-request with
        // the reviewer in the person's place.
        "codex" => vec![
            m(
                "untrusted",
                "Untrusted",
                "Asks before anything not known safe",
                false,
            ),
            m(
                "on-request",
                "On request",
                "Asks to go past the sandbox",
                false,
            ),
            m(
                "approve-for-me",
                "Approve for me",
                "A reviewer agent decides in your place; uses extra Codex usage",
                false,
            ),
            m(
                "never",
                "Never",
                "Never asks; what the sandbox stops goes back to the model",
                true,
            ),
        ],
        "omp" => vec![
            m("always-ask", "Always ask", "Asks before every tool", false),
            m("write", "Write", "Asks before writes and commands", false),
            m("yolo", "Yolo", "Asks nothing", true),
        ],
        // Pi never asks (docs/security.md); what it may do is its tool set,
        // `--tools`. Its own default is every tool.
        "pi" => vec![
            m(
                "read",
                "Read only",
                "Reads, searches and lists; no shell or edits",
                false,
            ),
            m("all", "All tools", "Shell and edits run unasked", true),
        ],
        // Muse's permission profiles, picked on the wire (`session/start`),
        // with its own labels and words.
        "muse" => vec![
            m("read-only", "Read only", "Reads files only", false),
            m(
                "ask-me",
                "Ask me",
                "Edits the workspace; asks before anything else",
                false,
            ),
            m(
                "auto-review",
                "Auto-review",
                "As Ask me, with an AI reviewer deciding what it can",
                false,
            ),
            m(
                "unrestricted",
                "Unrestricted",
                "No sandbox and no asking; commands reach the network",
                true,
            ),
        ],
        // OpenCode's server has no mode. Its own config rules stand unless
        // a session adds to them, and --auto lives in its client, which
        // accepts every ask: Libri's reply does what that does.
        "opencode" => vec![
            m(
                "default",
                "Its own rules",
                "OpenCode's config decides; unset, nearly everything runs unasked",
                false,
            ),
            m("ask", "Ask", "Asks before every tool", false),
            m(
                "approve",
                "Auto-accept",
                "As --auto: accepts all it would ask; what is denied stays denied",
                true,
            ),
        ],
        agent => {
            let mut modes = vec![
                m("ask", "Ask", "Passes on what the agent asks", false),
                m("approve", "Accept all", "Accepts what the agent asks", true),
            ];
            modes.extend(acp_modes(state).remove(agent).unwrap_or_default());
            modes
        }
    }
}

/// The sandbox settings a harness takes; empty when it has none. omp's own
/// docs say its approval mode sandboxes nothing, and OpenCode has none.
pub fn sandbox_modes(harness: &str) -> Vec<Mode> {
    let m = mode;
    match harness {
        // Claude Code's /sandbox: it covers Bash only, and is no part of its
        // permission modes, so its picker never shows it.
        "claude" => vec![
            hidden("off", "Sandbox off", "Commands run on the machine", true),
            hidden(
                "auto-allow",
                "Sandbox, auto-allow",
                "Bash runs unasked inside the sandbox",
                false,
            ),
            hidden(
                "regular",
                "Sandbox, regular",
                "Bash runs inside the sandbox; the mode still asks",
                false,
            ),
        ],
        "codex" => vec![
            m(
                "read-only",
                "Read only",
                "Reads anything, writes nothing",
                false,
            ),
            m(
                "workspace-write",
                "Workspace write",
                "Writes in the project",
                false,
            ),
            m("danger-full-access", "Full access", "No sandbox", true),
        ],
        _ => Vec::new(),
    }
}

/// `saved` of the old two on `harness`: "auto" worked in the project and
/// asked past it, "full" asked nothing. Claude Code's own "auto" is the old
/// one, and OpenCode's rules are what the old "auto" ran on.
fn legacy_access(harness: &str, saved: &str) -> Option<&'static str> {
    Some(match (harness, saved) {
        ("claude", "auto") => "auto",
        ("claude", "full") => "bypassPermissions",
        ("codex", "auto") => "on-request",
        ("codex", "full") => "never",
        ("omp", "auto") => "write",
        ("omp", "full") => "yolo",
        ("pi", "auto") => "all",
        ("pi", "full") => "all",
        ("muse", "auto") => "auto-review",
        ("muse", "full") => "unrestricted",
        ("opencode", "auto") => "default",
        ("opencode", "full") => "approve",
        (_, "auto") => "ask",
        (_, "full") => "approve",
        _ => return None,
    })
}

/// Only Codex's old "full" reached past its sandbox.
fn legacy_sandbox(harness: &str, saved: &str) -> Option<&'static str> {
    match (harness, saved) {
        ("codex", "auto") => Some("workspace-write"),
        ("codex", "full") => Some("danger-full-access"),
        _ => None,
    }
}

fn plain_sandbox(harness: &str) -> Option<&'static str> {
    match harness {
        "claude" => Some("off"),
        "codex" => Some("workspace-write"),
        _ => None,
    }
}

/// The mode `id` stands for on `harness`, an old name included.
pub fn access_named(harness: &str, state: &Path, id: &str) -> Option<String> {
    let listed = access_modes(harness, state).iter().any(|m| m.id == id);
    if listed {
        return Some(id.to_owned());
    }
    legacy_access(harness, id).map(str::to_owned)
}

pub fn sandbox_named(harness: &str, id: &str) -> Option<String> {
    sandbox_modes(harness)
        .iter()
        .any(|m| m.id == id)
        .then(|| id.to_owned())
}

/// `picks` kept only where `harness` has them under the same id, for a
/// thread moved to it; the rest fall to that harness's default.
pub fn carried(harness: &str, state: &Path, picks: Picks) -> Picks {
    let listed = access_modes(harness, state);
    Picks {
        access: picks.access.filter(|a| listed.iter().any(|m| m.id == *a)),
        sandbox: picks.sandbox.and_then(|s| sandbox_named(harness, &s)),
    }
}

/// What `pick` comes to on `harness`. Each half goes to the pick, else the
/// harness's Settings default, else the old saved pick, else its safe mode.
/// A value the harness doesn't have counts as no value.
pub fn grant(harness: &str, state: &Path, pick: &Picks) -> Grant {
    let saved = LEGACY.read(state);
    let saved = saved.as_deref().unwrap_or("auto");
    let default = defaults(state).remove(harness).unwrap_or_default();
    let named = |m: &Option<String>| m.as_deref().and_then(|m| access_named(harness, state, m));
    let mode = named(&pick.access)
        .or_else(|| named(&default.access))
        .or_else(|| access_named(harness, state, saved))
        .unwrap_or_else(|| forced(harness).mode);
    let boxed = |s: &Option<String>| s.as_deref().and_then(|s| sandbox_named(harness, s));
    let sandbox = boxed(&pick.sandbox)
        // A worker's saved "full" meant both halves.
        .or_else(|| {
            let old = legacy_sandbox(harness, pick.access.as_deref()?)?;
            Some(old.to_owned())
        })
        .or_else(|| boxed(&default.sandbox))
        .or_else(|| legacy_sandbox(harness, saved).map(str::to_owned))
        .or_else(|| plain_sandbox(harness).map(str::to_owned));
    Grant { mode, sandbox }
}

/// Whether `mode` on `harness` asks nothing, as Full access doesn't.
pub fn asks_nothing(harness: &str, state: &Path, mode: &str) -> bool {
    access_modes(harness, state)
        .iter()
        .any(|m| m.id == mode && m.open)
}

/// A turn that must not act: what it would do has to ask, and with no one
/// to ask is refused; Codex's sandbox holds it to reading.
pub fn forced(harness: &str) -> Grant {
    // Pi can't ask, and Muse's reviewer allows writes unasked: each is held
    // to what reads.
    let mode = match harness {
        "pi" => "read",
        "muse" => "read-only",
        other => legacy_access(other, "auto").unwrap_or("ask"),
    };
    Grant {
        mode: mode.to_owned(),
        sandbox: match harness {
            "codex" => Some("read-only".into()),
            other => plain_sandbox(other).map(str::to_owned),
        },
    }
}

/// The main chat's pick on `harness`, if it made one there.
pub fn main_picks(state: &Path, harness: &str) -> Picks {
    held(&PICKED, state, harness)
}

fn held(kept: &Kept, state: &Path, harness: &str) -> Picks {
    kept.read(state)
        .and_then(|t| serde_json::from_str::<MainPick>(&t).ok())
        .filter(|p| p.harness == harness)
        .map(|p| p.picks)
        .unwrap_or_default()
}

/// Temp's pick on `harness`, if it made one there.
pub fn temp_picks(state: &Path, harness: &str) -> Picks {
    held(&TEMP_PICKED, state, harness)
}

/// `change` in the harness's own terms; a value it lacks is refused.
pub fn checked(harness: &str, state: &Path, change: Picks) -> Result<Picks, String> {
    let access = match change.access {
        Some(a) => {
            Some(access_named(harness, state, &a).ok_or(format!("{harness} has no mode {a}."))?)
        }
        None => None,
    };
    let sandbox = match change.sandbox {
        Some(s) => {
            Some(sandbox_named(harness, &s).ok_or(format!("{harness} has no sandbox {s}."))?)
        }
        None => None,
    };
    Ok(Picks { access, sandbox })
}

fn over(mut under: Picks, change: Picks) -> Picks {
    under.access = change.access.or(under.access);
    under.sandbox = change.sandbox.or(under.sandbox);
    under
}

fn write_main(state: &Path, harness: &str, picks: Picks) -> Result<(), String> {
    hold(&PICKED, state, harness, picks)
}

fn hold(kept: &Kept, state: &Path, harness: &str, picks: Picks) -> Result<(), String> {
    let main = MainPick {
        harness: harness.to_owned(),
        picks,
    };
    kept.write(
        state,
        &serde_json::to_string(&main).map_err(|e| e.to_string())?,
    )
}

/// Sets the main chat's mode or sandbox from the next turn on.
pub fn pick_main(state: &Path, harness: &str, change: Picks) -> Result<(), String> {
    let change = checked(harness, state, change)?;
    write_main(state, harness, over(main_picks(state, harness), change))
}

/// Sets Temp's mode or sandbox from the next turn on.
pub fn pick_temp(state: &Path, harness: &str, change: Picks) -> Result<(), String> {
    let change = checked(harness, state, change)?;
    hold(
        &TEMP_PICKED,
        state,
        harness,
        over(temp_picks(state, harness), change),
    )
}

/// Each harness's default as kept. One that does not read is left out on
/// its own, not with the rest.
pub fn defaults(state: &Path) -> BTreeMap<String, Picks> {
    kept_defaults(state)
        .into_iter()
        .filter_map(|(h, v)| match serde_json::from_value(v) {
            Ok(p) => Some((h, p)),
            Err(e) => {
                eprintln!("libri: access default for {h} does not read: {e}");
                None
            }
        })
        .collect()
}

/// The file as it stands, so a change to one harness's default keeps the
/// others whole, a bad one and fields this build does not know included.
fn kept_defaults(state: &Path) -> serde_json::Map<String, serde_json::Value> {
    DEFAULTS
        .read(state)
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Sets a harness's default. A pick the main chat holds on that harness goes,
/// so the setting shows at once rather than on the next harness switch.
pub fn pick_default(state: &Path, harness: &str, change: Picks) -> Result<(), String> {
    let change = checked(harness, state, change)?;
    let old = defaults(state).remove(harness).unwrap_or_default();
    let now = over(old, change.clone());
    let mut all = kept_defaults(state);
    let entry = all
        .entry(harness.to_owned())
        .or_insert_with(|| serde_json::json!({}));
    if !entry.is_object() {
        *entry = serde_json::json!({});
    }
    let fields = entry.as_object_mut().expect("made an object above");
    for (key, value) in [("access", now.access), ("sandbox", now.sandbox)] {
        match value {
            Some(v) => fields.insert(key.into(), v.into()),
            None => fields.remove(key),
        };
    }
    DEFAULTS.write(state, &serde_json::Value::Object(all).to_string())?;
    let without = |mut held: Picks| {
        if change.access.is_some() {
            held.access = None;
        }
        if change.sandbox.is_some() {
            held.sandbox = None;
        }
        held
    };
    hold(
        &TEMP_PICKED,
        state,
        harness,
        without(temp_picks(state, harness)),
    )?;
    write_main(state, harness, without(main_picks(state, harness)))
}

fn acp_modes(state: &Path) -> BTreeMap<String, Vec<Mode>> {
    ACP_MODES
        .read(state)
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Keeps what an ACP agent's session/new declared: `modes` as ACP gives it.
pub fn remember_acp(state: &Path, harness: &str, modes: &serde_json::Value) {
    let Some(available) = modes["availableModes"].as_array() else {
        return;
    };
    let declared: Vec<Mode> = available
        .iter()
        .filter_map(|m| {
            Some(mode(
                &format!("{ACP_MODE}{}", m["modeId"].as_str().or(m["id"].as_str())?),
                m["name"].as_str()?,
                m["description"].as_str().unwrap_or_default(),
                false,
            ))
        })
        .collect();
    let mut all = acp_modes(state);
    if all.get(harness) == Some(&declared) {
        return;
    }
    all.insert(harness.to_owned(), declared);
    let kept = serde_json::to_string(&all).map_err(|e| e.to_string());
    if let Err(e) = kept.and_then(|text| ACP_MODES.write(state, &text)) {
        eprintln!("libri: keeping {harness}'s modes: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("libri-access-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn on(access: &str) -> Picks {
        Picks {
            access: Some(access.into()),
            sandbox: None,
        }
    }

    #[test]
    fn the_old_picks_keep_working_on_every_harness() {
        let dir = state("old");
        LEGACY.write(&dir, "full").unwrap();
        let open = |h, m: &str, s: Option<&str>| {
            let g = grant(h, &dir, &Picks::default());
            assert_eq!((g.mode.as_str(), g.sandbox.as_deref()), (m, s), "{h}");
        };
        open("claude", "bypassPermissions", Some("off"));
        open("codex", "never", Some("danger-full-access"));
        open("omp", "yolo", None);
        open("opencode", "approve", None);
        open("pi", "all", None);
        open("muse", "unrestricted", None);
        open("grok", "approve", None);
        LEGACY.write(&dir, "auto").unwrap();
        open("claude", "auto", Some("off"));
        open("codex", "on-request", Some("workspace-write"));
        open("omp", "write", None);
        open("opencode", "default", None);
        open("pi", "all", None);
        open("muse", "auto-review", None);
        open("grok", "ask", None);
        // A worker saved on the old names means what it meant.
        let g = grant("codex", &dir, &on("full"));
        assert_eq!(g.mode, "never");
        assert_eq!(g.sandbox.as_deref(), Some("danger-full-access"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn one_bad_default_leaves_the_rest_and_survives_a_save() {
        let dir = state("bad");
        DEFAULTS
            .write(
                &dir,
                r#"{"codex":{"access":"never","later":1},"omp":{"access":7}}"#,
            )
            .unwrap();
        let all = defaults(&dir);
        assert_eq!(all["codex"].access.as_deref(), Some("never"));
        assert!(!all.contains_key("omp"));
        pick_default(&dir, "claude", on("plan")).unwrap();
        let kept: serde_json::Value = serde_json::from_str(&DEFAULTS.read(&dir).unwrap()).unwrap();
        assert_eq!(kept["omp"]["access"], 7);
        assert_eq!(kept["codex"]["later"], 1);
        assert_eq!(kept["claude"]["access"], "plan");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_pick_stands_on_the_default_which_stands_on_the_old_pick() {
        let dir = state("order");
        LEGACY.write(&dir, "full").unwrap();
        pick_default(&dir, "codex", on("on-request")).unwrap();
        assert_eq!(grant("codex", &dir, &Picks::default()).mode, "on-request");
        pick_main(&dir, "codex", on("untrusted")).unwrap();
        let main = main_picks(&dir, "codex");
        assert_eq!(grant("codex", &dir, &main).mode, "untrusted");
        // The main chat's pick holds on the harness it was made on only.
        assert_eq!(main_picks(&dir, "claude"), Picks::default());
        // A mode the harness lacks falls to that harness's default.
        assert_eq!(grant("codex", &dir, &on("manual")).mode, "on-request");
        // A new default shows at once.
        pick_default(&dir, "codex", on("never")).unwrap();
        let main = main_picks(&dir, "codex");
        assert_eq!(grant("codex", &dir, &main).mode, "never");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_hidden_mode_stands_as_any_other() {
        let dir = state("hidden");
        let listed = |h| -> Vec<String> {
            let shown = access_modes(h, &dir).into_iter().filter(|m| !m.hidden);
            shown.map(|m| m.id).collect()
        };
        assert_eq!(
            listed("claude"),
            ["manual", "acceptEdits", "plan", "auto", "bypassPermissions"]
        );
        assert_eq!(listed("codex").len(), 4);
        assert!(sandbox_modes("claude").iter().all(|m| m.hidden));
        assert!(sandbox_modes("codex").iter().all(|m| !m.hidden));
        for id in ["bypassPermissions", "dontAsk"] {
            pick_main(&dir, "claude", on(id)).unwrap();
            let main = main_picks(&dir, "claude");
            assert_eq!(grant("claude", &dir, &main).mode, id);
        }
        let boxed = Picks {
            access: None,
            sandbox: Some("auto-allow".into()),
        };
        pick_main(&dir, "claude", boxed).unwrap();
        let main = main_picks(&dir, "claude");
        let g = grant("claude", &dir, &main);
        assert_eq!(g.mode, "dontAsk");
        assert_eq!(g.sandbox.as_deref(), Some("auto-allow"));
        assert_eq!(carried("claude", &dir, on("dontAsk")), on("dontAsk"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn full_access_reads_the_same_on_claude_and_codex() {
        let dir = state("full");
        let claude = access_modes("claude", &dir);
        let bypass = claude.iter().find(|m| m.id == "bypassPermissions").unwrap();
        let codex = sandbox_modes("codex");
        let open = codex.iter().find(|m| m.id == "danger-full-access").unwrap();
        assert_eq!(bypass.name, "Full access");
        assert_eq!(open.name, bypass.name);
        assert!(bypass.open && !bypass.hidden);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn modes_a_harness_lacks_are_refused() {
        let dir = state("refused");
        assert!(pick_main(&dir, "omp", on("manual")).is_err());
        let boxed = Picks {
            access: None,
            sandbox: Some("workspace-write".into()),
        };
        assert!(pick_default(&dir, "omp", boxed.clone()).is_err());
        pick_default(&dir, "codex", boxed).unwrap();
        // An old client's name comes in as the harness's own.
        pick_main(&dir, "omp", on("full")).unwrap();
        assert_eq!(main_picks(&dir, "omp").access.as_deref(), Some("yolo"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn moving_a_thread_keeps_only_what_the_new_harness_has() {
        let dir = state("moved");
        let held = |a: &str, s: &str| Picks {
            access: Some(a.into()),
            sandbox: Some(s.into()),
        };
        assert_eq!(
            carried("codex", &dir, held("auto", "off")),
            Picks::default()
        );
        assert_eq!(
            carried("codex", &dir, held("never", "read-only")),
            held("never", "read-only")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_turn_that_may_not_act_gets_the_safe_mode() {
        assert_eq!(forced("codex").sandbox.as_deref(), Some("read-only"));
        assert_eq!(forced("codex").mode, "on-request");
        assert_eq!(forced("claude").mode, "auto");
        assert_eq!(forced("opencode").mode, "default");
        assert_eq!(forced("pi").mode, "read");
        assert_eq!(forced("muse").mode, "read-only");
    }

    #[test]
    fn an_acp_agents_modes_are_remembered_by_agent() {
        let dir = state("acp");
        let modes = serde_json::json!({"currentModeId": "plan", "availableModes": [
            {"id": "plan", "name": "Plan", "description": "Reads only"},
        ]});
        remember_acp(&dir, "grok", &modes);
        let listed = access_modes("grok", &dir);
        assert_eq!(listed.last().unwrap().id, "mode:plan");
        assert_eq!(access_modes("gemini", &dir).len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
