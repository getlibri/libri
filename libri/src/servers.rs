//! The one list of MCP servers, Libri's own: handed to every harness on
//! every turn in that harness's terms, so spawn, watch, the memory and
//! the connections exist whichever harness runs. Harnesses bring nothing
//! here of their own to rely on: on a host where one has a config of its
//! own those tools come on top.
//!
//! What a turn may do is held here too, not in one harness's settings: the
//! tool server takes the turn's scope and offers only what that scope
//! holds, and a connection's relay never passes on the tools it withholds. T3 Code and
//! Paseo hand their servers over per session the same way; ACP's
//! `session/new` is the standard form of it.

use crate::model::Run;
use crate::rpc::Session;
use serde_json::{Map, Value, json};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// What a turn may reach of Libri's tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The person's own turn: everything.
    Person,
    /// A subagent's: everything but starting and stopping agents, and timers.
    Agent,
    /// One a watch or someone else's message started: only what reads.
    Quiet,
    /// OpenCode's one server, for all its turns: every tool, and each
    /// session held to its scope by its rules. Never a turn's own.
    Shared,
    /// The scratchpad's with the memory on: the person's own, but for the
    /// hold on workers, which the scratchpad leaves to the main chat and
    /// its promotion.
    Scratchpad,
    /// The scratchpad's with the memory off: the web alone, as nothing of
    /// the person's may reach a turn that keeps no memory of them.
    Memoryless,
    /// A named memory's chat (`memories`): the scratchpad's, its memory
    /// tools reading that memory's own hippo, and nothing of the workers
    /// that fork the main memory or the wiki dreamed from it.
    Apart,
}

impl Scope {
    /// The variable the tool server reads its scope from: an argument
    /// can't serve, as omp's config is one file for all its turns.
    pub const VAR: &str = "LIBRI_SCOPE";

    /// The variable that names the worker whose turn it is. The host sets
    /// it, so a tool's sender can't be what a model says it is. OpenCode's
    /// one server can't have it: `task::session_worker` stands in there.
    pub const WORKER: &str = "LIBRI_WORKER";

    /// The turn's scope; None for a call that holds no tools at all.
    pub fn of(run: &Run) -> Option<Self> {
        if run.plain {
            None
        } else if !run.acts {
            Some(Self::Quiet)
        } else if run.memory_dir.is_some() && run.ephemeral {
            Some(Self::Apart)
        } else if run.ephemeral && run.memory_off {
            Some(Self::Memoryless)
        } else if run.ephemeral {
            Some(Self::Scratchpad)
        } else if run.sub.is_some() {
            Some(Self::Agent)
        } else {
            Some(Self::Person)
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Agent => "agent",
            Self::Quiet => "quiet",
            Self::Shared => "shared",
            Self::Scratchpad => "scratchpad",
            Self::Memoryless => "memoryless",
            Self::Apart => "apart",
        }
    }

    pub fn named(name: &str) -> Self {
        match name {
            "person" => Self::Person,
            "agent" => Self::Agent,
            "shared" => Self::Shared,
            "scratchpad" => Self::Scratchpad,
            "memoryless" => Self::Memoryless,
            "apart" => Self::Apart,
            // Anything unknown gets the least.
            _ => Self::Quiet,
        }
    }

    /// The tools of Libri's server this scope may not reach.
    pub fn withheld(self) -> Vec<&'static str> {
        match self {
            Self::Person => vec!["request_subworker", "ask_person"],
            // A worker's words go to whoever started it, not to a person
            // who could tap a reply.
            Self::Agent => [BOSS.as_slice(), &["offer_replies"]].concat(),
            Self::Quiet => ACTS.to_vec(),
            Self::Scratchpad => WORKERS.to_vec(),
            // A memory's chat starts and reads workers as the main chat does,
            // but those of its own memory alone (`tools::Tools::memory`); the
            // replies it would offer have no buttons on its tile.
            Self::Apart => vec!["request_subworker", "ask_person", "offer_replies"],
            Self::Memoryless | Self::Shared => Vec::new(),
        }
    }

    /// The only tools of Libri's server this scope may reach, when it is
    /// held to a list.
    pub fn only(self) -> Option<&'static [&'static str]> {
        (self == Self::Memoryless).then_some(&WEB)
    }

    /// Whether `tool` of Libri's server is in reach of this scope.
    pub fn reaches(self, tool: &str) -> bool {
        self.only().map_or_else(
            || !self.withheld().contains(&tool),
            |only| only.contains(&tool),
        )
    }
}

/// What starts, stops and steers workers, which the scratchpad does not.
const WORKERS: [&str; 9] = [
    "ask_person",
    "spawn",
    "request_subworker",
    "answer_subworker_request",
    "cancel_task",
    "archive_agent",
    "restore_agent",
    "change_agent_model",
    "tell_agent",
];

/// What reads or steers one memory's workers, which a tool server shared by
/// every memory can't keep to a named one.
pub const MEMORY_BOUND: [&str; 11] = [
    "spawn",
    "request_subworker",
    "answer_subworker_request",
    "cancel_task",
    "archive_agent",
    "restore_agent",
    "change_agent_model",
    "tell_agent",
    "agent_chat",
    "list_agents",
    "workers",
];

/// All the memory-off scratchpad holds: none of them read anything of the
/// person's.
const WEB: [&str; 2] = ["web_search", "web_fetch"];

/// Libri's own hold on its subagents and timers, which they don't
/// share: as in Unii, one level of agents. They talk among themselves.
const BOSS: [&str; 14] = [
    "spawn",
    "answer_subworker_request",
    "cancel_task",
    "archive_agent",
    "restore_agent",
    "list_agents",
    "change_agent_model",
    "watch",
    // The person's own screen is arranged by the one they talk to.
    "layout_change",
    "layout_undo",
    "tile_open",
    // So is their terminal typed in: a worker reads it, to follow a build
    // the person runs, but asks for what it needs typed.
    "terminal_send",
    "terminal_open",
    // Another person's assistant hears from the one its person talks to.
    "message_peer",
];

/// The tools that act on the world.
const ACTS: [&str; 24] = [
    "ask_person",
    "request_subworker",
    "answer_subworker_request",
    "list_add",
    "list_remove",
    "draft_mail",
    "make_image",
    "web_fetch",
    "spawn",
    "tell_agent",
    "cancel_task",
    "archive_agent",
    "restore_agent",
    "change_agent_model",
    "show_page",
    "page_data_change",
    "layout_change",
    "layout_undo",
    "tile_open",
    "terminal_send",
    "terminal_open",
    "watch",
    "run_code",
    "message_peer",
];

/// Servers by name, each in Claude Code's `mcpServers` shape, which the
/// others are made from.
pub type Servers = Map<String, Value>;

/// Whether a variable is one the tool server may need: harnesses differ in
/// what they pass on to a server, Codex next to nothing.
fn passed(name: &str) -> bool {
    ["HIPPO_", "LIBRI_", "XDG_"]
        .iter()
        .any(|p| name.starts_with(p))
        || [
            "STATE_DIRECTORY",
            "HOME",
            "PATH",
            "TZ",
            "TZDIR",
            "LOCALE_ARCHIVE",
        ]
        .contains(&name)
}

/// This binary's path, for a harness to run it by. Nothing else names it:
/// the build link may already lead to a newer build than this daemon's.
pub fn program() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| runnable(p).to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

/// `current_exe` reads /proc/self/exe, which gains " (deleted)" once the
/// running binary's file is unlinked. `nix store optimise` does that when it
/// swaps a file for a hard link to the same bytes, so the path without the
/// suffix runs this same program, and the path with it runs nothing: Claude
/// Code failed the MCP servers with ENOENT.
fn runnable(exe: PathBuf) -> PathBuf {
    if exe.exists() {
        return exe;
    }
    match exe.as_os_str().as_bytes().strip_suffix(b" (deleted)") {
        Some(path) => PathBuf::from(OsStr::from_bytes(path)),
        None => exe,
    }
}

pub fn state() -> Option<PathBuf> {
    std::env::var_os("STATE_DIRECTORY").map(PathBuf::from)
}

pub(crate) fn passed_env() -> Map<String, Value> {
    std::env::vars()
        .filter(|(k, _)| passed(k))
        .map(|(k, v)| (k, Value::String(v)))
        .collect()
}

/// Libri's own server, for a turn of `scope`, by `worker` if it is one's.
fn own(scope: Scope, worker: Option<&str>, state: &Path) -> Result<Value, String> {
    let mut env = passed_env();
    env.insert(Scope::VAR.into(), scope.name().into());
    if let Some(worker) = worker {
        env.insert(Scope::WORKER.into(), worker.into());
    }
    Ok(json!({"type": "stdio", "command": program()?,
        "args": ["tools", state], "env": env}))
}

/// The variable naming the wiki's vault (the host key `wiki.vault`); unset,
/// `wiki/` in the state directory.
pub const WIKI: &str = "LIBRI_WIKI";

/// What a turn may do with the wiki: the notes the `wiki` server keeps, a
/// program beside hippo, in a folder Obsidian opens as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WikiReach {
    Write,
    Read,
    Nothing,
}

/// The person's own turn writes the notes; a worker, Temp and a quiet turn
/// read them; a memory-off turn and a blind worker, which hold none of the
/// memory, get none of it. OpenCode's one server writes, and each session is
/// held to its own reach by its rules (opencode.rs).
pub fn wiki_reach(scope: Scope, worker: Option<&str>, state: &Path) -> WikiReach {
    if worker.is_some_and(|w| crate::task::blind(state, w)) {
        return WikiReach::Nothing;
    }
    match scope {
        Scope::Person | Scope::Shared => WikiReach::Write,
        Scope::Agent | Scope::Scratchpad | Scope::Quiet => WikiReach::Read,
        Scope::Memoryless | Scope::Apart => WikiReach::Nothing,
    }
}

/// The wiki's server for a turn of `scope`, when `wiki` is installed: a
/// lone daemon without it offers no notes rather than a server that fails.
fn wiki(scope: Scope, worker: Option<&str>, state: &Path) -> Option<Value> {
    let reach = wiki_reach(scope, worker, state);
    if reach == WikiReach::Nothing {
        return None;
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let command = std::env::split_paths(&path)
        .map(|d| d.join("wiki"))
        .find(|p| p.is_file())?;
    let vault = std::env::var_os(WIKI)
        .map(PathBuf::from)
        .unwrap_or_else(|| state.join("wiki"));
    Some(wiki_entry(reach, &command, &vault))
}

fn wiki_entry(reach: WikiReach, command: &Path, vault: &Path) -> Value {
    let mut args = vec![json!("serve"), json!(vault)];
    if reach == WikiReach::Read {
        args.push(json!("--read-only"));
    }
    json!({"type": "stdio", "command": command, "args": args})
}

/// The variable naming the memory's server (the host key `memory.command`).
pub const MEMORY_COMMAND: &str = "LIBRI_MEMORY_COMMAND";

/// The variable telling the memory's server which named memory a turn
/// belongs to; unset, its main one.
pub const MEMORY_NAME: &str = "HIPPO2_MEMORY";

/// The memory's server, hippo2 by default: every message ever said, word
/// for word, recalled by tools beside the harness's own thread, which the
/// harness compacts as it would at a terminal. A memory-off turn and a blind
/// worker hold none of it; a named memory's turns read that memory alone.
fn memory(scope: Scope, worker: Option<&str>, state: &Path) -> Option<(String, Value)> {
    let blind = worker.is_some_and(|w| crate::task::blind(state, w));
    let words = memory_command()?;
    let entry = memory_entry(scope, blind, words.clone())?;
    Some((server_name(&words[0]), entry))
}

/// The name the memory's server goes by in a harness: its program's, as a
/// person adds it to a harness of their own (`claude mcp add hippo2`), so a
/// harness that has that entry too has it once, Libri's in its place.
fn server_name(command: &str) -> String {
    Path::new(command)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "memory".into())
}

/// The memory's server's name, as a turn's tool names carry it; None when
/// there is none.
pub fn memory_name() -> Option<String> {
    memory_command()
        .and_then(|w| w.first().cloned())
        .map(|c| server_name(&c))
}

/// hippo2's command line for one answer rather than a server: the memory's
/// command with its `mcp` swapped for `args`, for a caller that is no
/// harness (the outside door). None when the memory is off or is not
/// hippo2, whose subcommands are all this knows.
pub fn memory_cli(args: &[&str]) -> Option<Vec<String>> {
    let mut words = memory_command()?;
    if words.last().is_some_and(|w| w == "mcp") {
        words.pop();
    }
    if server_name(words.first()?) != "hippo2" {
        return None;
    }
    words.extend(args.iter().map(|a| (*a).to_owned()));
    Some(words)
}

/// The memory's command line: `memory.command`, else hippo2 on the PATH.
fn memory_command() -> Option<Vec<String>> {
    match std::env::var(MEMORY_COMMAND) {
        Ok(line) => shlex::split(&line),
        Err(_) => {
            let path = std::env::var_os("PATH").unwrap_or_default();
            let found = std::env::split_paths(&path)
                .map(|d| d.join("hippo2"))
                .find(|p| p.is_file())?;
            Some(vec![found.to_string_lossy().into_owned(), "mcp".into()])
        }
    }
}

fn memory_entry(scope: Scope, blind: bool, words: Vec<String>) -> Option<Value> {
    if scope == Scope::Memoryless || blind {
        return None;
    }
    let (command, args) = words.split_first()?;
    Some(json!({"type": "stdio", "command": command, "args": args, "env": passed_env()}))
}

/// Starts Libri's own server as a turn would and asks for its tools: a
/// server that exits at start costs every turn its tools and says nothing.
pub fn check() -> Result<(), String> {
    let wait = Duration::from_secs(20);
    let state = state().ok_or("STATE_DIRECTORY is not set")?;
    let entry = own(Scope::Person, None, &state)?;
    let mut cmd = Command::new(program()?);
    cmd.args(["tools", &state.to_string_lossy()]);
    cmd.envs(
        entry["env"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| Some((k, v.as_str()?))),
    );
    let s = Session::spawn(cmd, true)?;
    let mut aside = Vec::new();
    let answered = s
        .call(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "libri", "version": env!("CARGO_PKG_VERSION")}}),
            wait,
            &mut aside,
        )
        .and_then(|_| s.peer.notify("notifications/initialized", json!({})))
        .and_then(|_| s.call("tools/list", json!({}), wait, &mut aside));
    s.end();
    match answered?["tools"].as_array() {
        Some(tools) if !tools.is_empty() => Ok(()),
        _ => Err("it lists no tools".into()),
    }
}

/// Servers a deployment keeps from every turn (LIBRI_MCP_WITHHELD): what
/// its harness configs hold for a person at a terminal and a turn must not
/// reach, as a seat's Brave with its saved logins. None by default: a
/// person's own servers are theirs.
fn withheld() -> Vec<String> {
    std::env::var("LIBRI_MCP_WITHHELD")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The servers of a `harness.mcp` file a turn may have.
fn added(config: &Value, withheld: &[String]) -> Servers {
    config["mcpServers"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(name, _)| !withheld.contains(name))
        .map(|(name, server)| (name.clone(), server.clone()))
        .collect()
}

/// What Codex and OpenCode would add from their own config of the `withheld`
/// servers: this entry takes each one's place.
fn off(mut servers: Map<String, Value>, entry: Value, withheld: &[String]) -> Value {
    for name in withheld {
        servers.insert(name.clone(), entry.clone());
    }
    servers.into()
}

/// The servers `run` gets, for a harness that takes them per turn; None for
/// a call that holds no tools.
pub fn of_run(run: &Run) -> Result<Option<Servers>, String> {
    let asking = crate::connections::asking(run.ask.as_ref());
    let Some(scope) = Scope::of(run) else {
        return Ok(None);
    };
    let mut list = list_asking(scope, run.sub.as_deref(), Some(&asking))?;
    // A named memory's chat and workers read its own hippo, never the main one.
    if let Some(env) = list.get_mut("libri").and_then(|s| s["env"].as_object_mut()) {
        for (k, v) in memory_env(run) {
            env.insert(k.into(), v.into());
        }
    }
    if let Some(env) = memory_name()
        .and_then(|n| list.get_mut(&n))
        .and_then(|s| s["env"].as_object_mut())
        && let Some((_, id)) = memory_env(run).into_iter().find(|(k, _)| *k == MEMORY)
    {
        env.insert(MEMORY_NAME.into(), id.into());
    }
    Ok(Some(list))
}

/// The variable naming the hippo store a tool server reads.
pub const HIPPO_DIR: &str = "HIPPO_DIR";

/// The variable naming the named memory a tool server serves, whose workers
/// alone it reaches; unset, the main one.
pub const MEMORY: &str = "LIBRI_MEMORY";

/// What a tool server of a run on a named memory is told of it: that
/// memory's hippo, and its ID, the name of the folder holding the store.
pub fn memory_env(run: &Run) -> Vec<(&'static str, String)> {
    let Some(dir) = &run.memory_dir else {
        return Vec::new();
    };
    let id = dir
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    vec![(HIPPO_DIR, dir.to_string_lossy().into()), (MEMORY, id)]
}

/// The servers the deployment adds (`harness.mcp`, a file in Claude Code's
/// shape named by LIBRI_MCP), less those it withholds from every turn. Its
/// `libri` entry, if any, only adds settings to Libri's own server.
pub fn deployment() -> Result<Servers, String> {
    let file = std::env::var("LIBRI_MCP").unwrap_or_default();
    if file.is_empty() {
        return Ok(Servers::new());
    }
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{file}: {e}"))?;
    let config: Value = serde_json::from_str(&text).map_err(|e| format!("{file}: {e}"))?;
    Ok(added(&config, &withheld()))
}

/// The servers a turn of `scope` by `worker` gets: Libri's own, the
/// connections, and those the deployment adds (`harness.mcp`, a file in
/// Claude Code's shape). A quiet turn gets Libri's alone, for what reads;
/// a memory-off one too, for the web.
pub fn list(scope: Scope, worker: Option<&str>) -> Result<Servers, String> {
    list_asking(scope, worker, None)
}

/// As `list`, the connections' relays asking whom `asking` names
/// (connections::ASKING); None asks the main chat.
pub fn list_asking(
    scope: Scope,
    worker: Option<&str>,
    asking: Option<&str>,
) -> Result<Servers, String> {
    let state = state().ok_or("STATE_DIRECTORY is not set")?;
    let mut out = Servers::new();
    if !matches!(scope, Scope::Quiet | Scope::Memoryless) {
        out.extend(deployment()?);
        let mut env = passed_env();
        if let Some(a) = asking {
            env.insert(crate::connections::ASKING.into(), a.into());
        }
        out.extend(crate::connections::entries(&state, &env)?);
    }
    // The wiki's, the memory's and ours, whatever a file says, so their
    // scope holds.
    if let Some(w) = wiki(scope, worker, &state) {
        out.insert("wiki".into(), w);
    }
    if let Some((name, m)) = memory(scope, worker, &state) {
        out.insert(name, m);
    }
    // A file's `libri`
    // adds its settings.
    let mut libri = own(scope, worker, &state)?;
    if let Some(env) = out.get("libri").and_then(|s| s["env"].as_object()).cloned() {
        libri["env"]
            .as_object_mut()
            .expect("own server has env")
            .extend(env);
    }
    out.insert("libri".into(), libri);
    Ok(out)
}

/// The list as `--mcp-config` takes it.
pub fn claude(servers: &Servers) -> String {
    json!({ "mcpServers": servers }).to_string()
}

/// The list as Codex's `mcp_servers` config.
pub fn codex(servers: &Servers) -> Value {
    let mut out = Map::new();
    for (name, s) in servers {
        let mut server = match s["type"].as_str().unwrap_or("stdio") {
            "stdio" => json!({"command": s["command"], "args": s["args"], "env": s["env"]}),
            _ => json!({"url": s["url"], "http_headers": s["headers"]}),
        };
        // Libri's tools run unasked, as Claude Code's --allowedTools lets
        // them; what acts on the world waits for the person through
        // Libri's own cards.
        server["default_tools_approval_mode"] = "approve".into();
        // Codex refuses nulls in its config.
        let server: Map<String, Value> = server
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, v)| !v.is_null())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        out.insert(name.clone(), server.into());
    }
    off(out, codex_off(), &withheld())
}

/// A server switched off, in Codex's terms. Codex wants a transport even on
/// an entry that is off.
pub fn codex_off() -> Value {
    json!({"url": "http://127.0.0.1:9/mcp", "enabled": false})
}

/// The list as OpenCode's `mcp` config.
pub fn opencode(servers: &Servers) -> Value {
    let mut out = Map::new();
    for (name, s) in servers {
        let server = match s["type"].as_str().unwrap_or("stdio") {
            "stdio" => {
                let mut command = vec![s["command"].clone()];
                command.extend(s["args"].as_array().cloned().unwrap_or_default());
                json!({"type": "local", "command": command,
                    "environment": s["env"].as_object().cloned().unwrap_or_default()})
            }
            _ => json!({"type": "remote", "url": s["url"],
                "headers": s["headers"].as_object().cloned().unwrap_or_default()}),
        };
        out.insert(name.clone(), server);
    }
    off(
        out,
        json!({"type": "remote", "url": "http://127.0.0.1:9/mcp", "enabled": false}),
        &withheld(),
    )
}

/// The list as omp's project config, which shares Claude Code's shape. It
/// goes in a file, so without the environment: omp's servers inherit its
/// own, the turn's scope among it.
pub fn omp(servers: &Servers) -> String {
    let mut plain = servers.clone();
    for s in plain.values_mut().filter_map(Value::as_object_mut) {
        s.remove("env");
    }
    json!({ "mcpServers": plain }).to_string()
}

/// The list as Pi's project config: omp's, each server's tools declared to
/// the model as built-ins are. Pi's default leaves MCP tools to scripts
/// and search (docs/mcp.md, "Control tool exposure").
pub fn pi(servers: &Servers) -> String {
    let mut config: Value = serde_json::from_str(&omp(servers)).expect("omp's config is JSON");
    for s in config["mcpServers"]
        .as_object_mut()
        .into_iter()
        .flat_map(|m| m.values_mut())
    {
        s["exposure"] = "direct".into();
    }
    config.to_string()
}

/// The list as Muse's `session/start` takes it, environment and all: one
/// session is one turn, so the scope travels with its servers.
pub fn muse(servers: &Servers) -> Value {
    let mut out = Map::new();
    for (name, s) in servers {
        let server = match s["type"].as_str().unwrap_or("stdio") {
            "stdio" => json!({"transport": "stdio", "command": s["command"],
                "args": s["args"].as_array().cloned().unwrap_or_default(),
                "env": s["env"].as_object().cloned().unwrap_or_default(),
                "framing": "lineDelimitedJson"}),
            _ => json!({"transport": "streamableHttp", "url": s["url"],
                "headers": s["headers"].as_object().cloned().unwrap_or_default()}),
        };
        out.insert(name.clone(), server);
    }
    out.into()
}

/// The list as ACP's `session/new` has it: stdio always, http and sse when
/// the harness takes them (`can`).
pub fn acp(servers: &Servers, can: &Value) -> Vec<Value> {
    let pairs = |o: &Value, k: &str, v: &str| -> Vec<Value> {
        o.as_object()
            .into_iter()
            .flatten()
            .map(|(name, value)| json!({ k: name, v: value }))
            .collect()
    };
    let mut out = Vec::new();
    for (name, s) in servers {
        match s["type"].as_str().unwrap_or("stdio") {
            "stdio" => out.push(json!({
                "name": name,
                "command": s["command"],
                "args": s["args"].as_array().cloned().unwrap_or_default(),
                "env": pairs(&s["env"], "name", "value"),
            })),
            kind @ ("http" | "sse") if can[kind] == true => out.push(json!({
                "type": kind,
                "name": name,
                "url": s["url"],
                "headers": pairs(&s["headers"], "name", "value"),
            })),
            kind => eprintln!("libri: the harness takes no {kind} MCP server; {name} left out"),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binary_relinked_under_a_daemon_is_run_by_its_own_path() {
        let gone = PathBuf::from("/nonexistent/bin/libri (deleted)");
        assert_eq!(runnable(gone), Path::new("/nonexistent/bin/libri"));
        let plain = PathBuf::from("/nonexistent/bin/libri");
        assert_eq!(runnable(plain.clone()), plain);
    }

    #[test]
    fn a_file_truly_named_deleted_keeps_its_name() {
        let dir = std::env::temp_dir().join(format!("libri-exe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("libri (deleted)");
        std::fs::write(&exe, "").unwrap();
        assert_eq!(runnable(exe.clone()), exe);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn servers() -> Servers {
        let v = json!({
            "own": {"type": "stdio", "command": "libri", "args": ["tools", "/s"], "env": {"A": "1"}},
            "far": {"type": "http", "url": "https://x/mcp", "headers": {"Auth": "b"}}
        });
        v.as_object().unwrap().clone()
    }

    #[test]
    fn every_harness_gets_the_same_servers_in_its_own_terms() {
        let s = servers();
        let claude: Value = serde_json::from_str(&claude(&s)).unwrap();
        assert_eq!(claude["mcpServers"]["own"]["args"][0], "tools");
        let c = codex(&s);
        assert_eq!(c["own"]["command"], "libri");
        assert_eq!(c["far"]["http_headers"]["Auth"], "b");
        let o = opencode(&s);
        assert_eq!(o["own"]["command"], json!(["libri", "tools", "/s"]));
        assert_eq!(o["far"]["type"], "remote");
        let omp: Value = serde_json::from_str(&omp(&s)).unwrap();
        assert!(omp["mcpServers"]["own"].get("env").is_none());
        let pi: Value = serde_json::from_str(&pi(&s)).unwrap();
        assert_eq!(pi["mcpServers"]["own"]["exposure"], "direct");
        assert!(pi["mcpServers"]["own"].get("env").is_none());
        let muse = muse(&s);
        assert_eq!(muse["own"]["transport"], "stdio");
        assert_eq!(muse["own"]["env"]["A"], "1");
        assert_eq!(muse["far"]["transport"], "streamableHttp");
        let a = acp(&s, &json!({"http": true}));
        assert_eq!(a.len(), 2);
        assert_eq!(acp(&s, &json!({})).len(), 1);
    }

    #[test]
    fn a_deployment_withholds_the_servers_it_names_and_no_others() {
        let file = json!({"mcpServers": {
            "browser": {"type": "http"}, "cloudflare": {"type": "stdio"}}});
        let names = ["browser".to_owned()];
        let got = added(&file, &names);
        assert_eq!(got.keys().collect::<Vec<_>>(), ["cloudflare"]);
        assert_eq!(added(&file, &[]).len(), 2);
        let entry = json!({"enabled": false});
        let turned = off(Map::new(), entry.clone(), &names);
        assert_eq!(turned["browser"]["enabled"], false);
        assert_eq!(off(Map::new(), entry, &[]), json!({}));
    }

    #[test]
    fn a_scope_holds_back_what_it_must() {
        assert_eq!(
            Scope::Person.withheld(),
            ["request_subworker", "ask_person"]
        );
        assert!(Scope::Quiet.withheld().contains(&"spawn"));
        assert!(!Scope::Quiet.withheld().contains(&"agent_chat"));
        assert!(Scope::Shared.withheld().is_empty());
        assert_eq!(Scope::named(Scope::Shared.name()), Scope::Shared);
        assert!(Scope::Agent.withheld().contains(&"spawn"));
        assert!(
            Scope::Agent
                .withheld()
                .contains(&"answer_subworker_request")
        );
        assert!(!Scope::Agent.withheld().contains(&"request_subworker"));
        assert!(Scope::Quiet.withheld().contains(&"request_subworker"));
        assert!(
            Scope::Scratchpad
                .withheld()
                .contains(&"answer_subworker_request")
        );
        assert!(Scope::Agent.withheld().contains(&"cancel_task"));
        assert!(!Scope::Agent.withheld().contains(&"web_fetch"));
        assert!(!Scope::Agent.withheld().contains(&"tell_agent"));
        assert!(!Scope::Agent.withheld().contains(&"agent_chat"));
        assert!(Scope::Quiet.withheld().contains(&"tell_agent"));
        assert!(Scope::Quiet.withheld().contains(&"web_fetch"));
        assert_eq!(Scope::named("nonsense"), Scope::Quiet);
    }

    #[test]
    fn a_scratchpad_turn_has_the_persons_reach_but_for_the_workers() {
        let run = Run {
            acts: true,
            ephemeral: true,
            ..Run::default()
        };
        assert_eq!(Scope::of(&run), Some(Scope::Scratchpad));
        assert_eq!(Scope::named(Scope::Scratchpad.name()), Scope::Scratchpad);
        for tool in ["draft_mail", "remind", "run_code", "list_add"] {
            assert!(Scope::Scratchpad.reaches(tool), "{tool}");
        }
        for tool in WORKERS {
            assert!(!Scope::Scratchpad.reaches(tool), "{tool}");
        }
    }

    #[test]
    fn a_named_memorys_chat_reaches_its_own_memory_and_workers() {
        let run = Run {
            acts: true,
            ephemeral: true,
            memory_dir: Some("/nowhere/memories/jobs/hippo".into()),
            ..Run::default()
        };
        assert_eq!(Scope::of(&run), Some(Scope::Apart));
        assert_eq!(
            memory_env(&run),
            vec![
                (HIPPO_DIR, "/nowhere/memories/jobs/hippo".to_owned()),
                (MEMORY, "jobs".to_owned())
            ]
        );
        // Its workers are workers, on its memory.
        let worker = Run {
            sub: Some("milo-2026-11-09-1300".into()),
            ephemeral: false,
            ..run.clone()
        };
        assert_eq!(Scope::of(&worker), Some(Scope::Agent));
        assert_eq!(Scope::named(Scope::Apart.name()), Scope::Apart);
        for tool in [
            "draft_mail",
            "run_code",
            "list_add",
            "web_fetch",
            "show_page",
            "spawn",
            "tell_agent",
            "agent_chat",
            "workers",
        ] {
            assert!(Scope::Apart.reaches(tool), "{tool}");
        }
        for tool in ["request_subworker", "offer_replies"] {
            assert!(!Scope::Apart.reaches(tool), "{tool}");
        }
        let state = Path::new("/nowhere");
        assert_eq!(wiki_reach(Scope::Apart, None, state), WikiReach::Nothing);
    }

    #[test]
    fn a_memory_off_scratchpad_turn_has_the_web_alone() {
        let run = Run {
            acts: true,
            ephemeral: true,
            memory_off: true,
            ..Run::default()
        };
        assert_eq!(Scope::of(&run), Some(Scope::Memoryless));
        assert_eq!(Scope::named(Scope::Memoryless.name()), Scope::Memoryless);
        for tool in ["web_search", "web_fetch"] {
            assert!(Scope::Memoryless.reaches(tool), "{tool}");
        }
        for tool in [
            "mail_read",
            "files",
            "transactions",
            "list_add",
            "run_code",
            "agent_chat",
        ] {
            assert!(!Scope::Memoryless.reaches(tool), "{tool}");
        }
    }

    #[test]
    fn the_person_writes_the_wiki_and_the_rest_read_it() {
        let state = std::env::temp_dir().join(format!("libri-wiki-{}", std::process::id()));
        assert_eq!(wiki_reach(Scope::Person, None, &state), WikiReach::Write);
        assert_eq!(wiki_reach(Scope::Shared, None, &state), WikiReach::Write);
        for scope in [Scope::Agent, Scope::Scratchpad, Scope::Quiet] {
            assert_eq!(wiki_reach(scope, Some("ada"), &state), WikiReach::Read);
        }
        assert_eq!(
            wiki_reach(Scope::Memoryless, None, &state),
            WikiReach::Nothing
        );
        let (cmd, vault) = (Path::new("/bin/wiki"), Path::new("/s/wiki"));
        let read = wiki_entry(WikiReach::Read, cmd, vault);
        assert_eq!(read["args"], json!(["serve", "/s/wiki", "--read-only"]));
        let write = wiki_entry(WikiReach::Write, cmd, vault);
        assert_eq!(write["args"], json!(["serve", "/s/wiki"]));
        assert_eq!(write["command"], "/bin/wiki");
    }

    #[test]
    fn every_turn_but_a_memory_off_or_blind_one_recalls_from_the_memory() {
        let words = || vec!["/bin/hippo2".to_owned(), "mcp".to_owned()];
        for scope in [
            Scope::Person,
            Scope::Agent,
            Scope::Quiet,
            Scope::Shared,
            Scope::Scratchpad,
            Scope::Apart,
        ] {
            let entry = memory_entry(scope, false, words()).unwrap();
            assert_eq!(entry["command"], "/bin/hippo2");
            assert_eq!(entry["args"], json!(["mcp"]));
        }
        assert!(memory_entry(Scope::Memoryless, false, words()).is_none());
        assert!(memory_entry(Scope::Agent, true, words()).is_none());
        // An empty `memory.command` turns the memory off.
        assert!(memory_entry(Scope::Person, false, Vec::new()).is_none());
        // It goes by its program's name, as a person's own entry does.
        assert_eq!(server_name("/nix/store/x-hippo2/bin/hippo2"), "hippo2");
        assert_eq!(server_name("hippo2"), "hippo2");
    }
}
