//! The host: what lets Libri run on any machine.
//!
//! One config file (TOML, `$XDG_CONFIG_HOME/libri/config.toml` or
//! `--config PATH`) sets what a deployment's environment sets. The host
//! keeps a stable id the app knows it by, whatever address reaches it. And
//! devices pair with a one-time link for a revocable token of their own, in
//! place of a sign-in, as T3 Code and Paseo do.

use ring::digest;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Each config key, the setting it stands for and what it does. The server
/// reads its settings from the environment, which its harnesses and their
/// tool servers inherit; the file fills in what the environment leaves
/// unset, so a deployment may still set any of them directly. The same
/// list documents them, for the person and the assistant (settings.rs).
pub struct Key {
    pub key: &'static str,
    pub env: &'static str,
    pub about: &'static str,
}

const fn key(key: &'static str, env: &'static str, about: &'static str) -> Key {
    Key { key, env, about }
}

pub const KEYS: &[Key] = &[
    key(
        "name",
        "LIBRI_NAME",
        "What the chat calls the assistant where a deployment names it; unset, `Libri`. The host itself goes by the machine's hostname. No prompt names the assistant: who it is opens the person's rules, as their Identity section.",
    ),
    key(
        "app",
        "LIBRI_APP",
        "What the installed app is called; unset, the name.",
    ),
    key(
        "person",
        "LIBRI_PERSON",
        "Who it works for; unset, `$USER`.",
    ),
    key(
        "state",
        "STATE_DIRECTORY",
        "Where the host keeps its state; unset, `$XDG_STATE_HOME/libri` (`libri-NAME` for `--instance NAME`).",
    ),
    key("style", "LIBRI_STYLE", "A line on tone."),
    key(
        "workdir",
        "LIBRI_WORKDIR",
        "The harnesses' working directory.",
    ),
    key(
        "daemon.listen",
        "LIBRI_SITE",
        "The address it listens on; unset, `127.0.0.1:6780`. `--listen ADDRESS:PORT` sets it for one run, `--port PORT` its port alone.",
    ),
    key(
        "daemon.url",
        "LIBRI_SITE_URL",
        "Its public address, for push and pairing links.",
    ),
    key(
        "daemon.site",
        "LIBRI_SITE_ROOT",
        "The web app it serves; unset, the packaged one.",
    ),
    key(
        "daemon.pages",
        "LIBRI_PAGES",
        "Where the pages the assistant publishes are kept; unset, `<state>/pages`.",
    ),
    key(
        "daemon.pages_url",
        "LIBRI_PAGES_URL",
        "Where those pages are served.",
    ),
    key(
        "daemon.open",
        "LIBRI_OPEN",
        "`true` lets every request in unpaired; unset, false.",
    ),
    key(
        "daemon.origins",
        "LIBRI_ORIGINS",
        "Comma-separated origins of the app served elsewhere that may use this host (`https://app.libri.is`); unset, none.",
    ),
    key(
        "daemon.tailnet",
        "LIBRI_TAILNET",
        "Comma-separated tailnet logins (`ada@example.com`) this host is for; `ada@example.com/box` names only that login's device `box`, for a machine shared by people its owner's login does not stand for. Set, each request's peer is looked up in the local tailscaled (`whois`), anyone else is refused, and those listed count as signed in with no pairing; loopback stays in. Unset, no tailnet gate.",
    ),
    key(
        "daemon.forward",
        "LIBRI_FORWARD",
        "Comma-separated `login=http://address` pairs: a tailnet login whose requests, WebSocket upgrades included, pass whole to that person's own Libri, so one address serves each person their own. Needs tailscaled as `daemon.tailnet` does.",
    ),
    key(
        "daemon.tailscale_socket",
        "LIBRI_TAILSCALE_SOCKET",
        "tailscaled's LocalAPI socket; unset, `/var/run/tailscale/tailscaled.sock`.",
    ),
    key(
        "connections.return_page",
        "LIBRI_RETURN_PAGE",
        "The page a connection's OAuth sign-in returns through, which sends the browser on to the address it reached Libri at; unset, `https://libri.is/connect/back`. Empty: every sign-in returns to `127.0.0.1`, and another device pastes the address it ended on.",
    ),
    key(
        "hippo.dir",
        "HIPPO_DIR",
        "The memory's directory; unset, `<state>/hippo`.",
    ),
    key(
        "hippo.serve",
        "LIBRI_HIPPO_SERVE",
        "Whether this daemon runs that hippo as a child; unset, true when `hippo.dir` is left default.",
    ),
    key(
        "hippo.shared",
        "LIBRI_HIPPO_SHARED",
        "When the hippo keeps other chats too: the title this daemon's chat goes under.",
    ),
    key(
        "memory.command",
        "LIBRI_MEMORY_COMMAND",
        "The memory every turn recalls from: an MCP server on stdio, as a command line, handed to every harness beside Libri's own tools. Unset, `hippo2 mcp` when `hippo2` is on the PATH; empty, none.",
    ),
    key(
        "wiki.vault",
        "LIBRI_WIKI",
        "The folder of the wiki, the assistant's notes: an Obsidian vault the `wiki` program serves beside hippo, written by the main chat and read by workers and Temp; unset, `wiki/` in the state directory.",
    ),
    key(
        "harness.default",
        "LIBRI_BACKEND",
        "The harness a new chat and a new worker run on until the person picks one; unset, omp when found, else the first found.",
    ),
    key(
        "harness.model",
        "LIBRI_MODEL",
        "The Claude model until the person picks one.",
    ),
    key(
        "harness.effort",
        "LIBRI_EFFORT",
        "The thinking effort until the person picks one.",
    ),
    key(
        "harness.efforts",
        "LIBRI_EFFORTS",
        "Comma-separated efforts offered where a harness has no list of its own.",
    ),
    key(
        "harness.coding",
        "LIBRI_CODING",
        "`0` leaves Claude Code without its own shell and file tools, for an assistant with Libri's alone.",
    ),
    key(
        "harness.instructions",
        "LIBRI_INSTRUCTIONS",
        "Read once, to fill an empty HOST.md (Settings > Agents, This host) when a release before 0.9.70 kept the host's facts here; nothing reads it after. Goes once no deployment sets it.",
    ),
    key(
        "harness.identity",
        "LIBRI_IDENTITY",
        "Read once, to open the person's rules (AGENTS.md) with an Identity section when a release before 0.9.70 kept who the assistant is here; nothing reads it after. Goes once no deployment sets it.",
    ),
    key(
        "harness.mcp",
        "LIBRI_MCP",
        "MCP servers added to Libri's own on every harness, in Claude Code's `mcpServers` shape.",
    ),
    key(
        "harness.mcp_withheld",
        "LIBRI_MCP_WITHHELD",
        "MCP servers no turn may reach, for a host whose harness configs hold some for a person at a terminal; unset, none.",
    ),
    key(
        "harness.claude.command",
        "LIBRI_CLAUDE",
        "Claude Code's command; unset, `claude`.",
    ),
    key(
        "harness.codex.command",
        "LIBRI_CODEX",
        "Codex's command; unset, `codex`.",
    ),
    key(
        "harness.opencode.command",
        "LIBRI_OPENCODE",
        "OpenCode's command; unset, `opencode`.",
    ),
    key(
        "harness.opencode.port",
        "LIBRI_OPENCODE_PORT",
        "A fixed port for OpenCode's server; unset, any free one.",
    ),
    key(
        "harness.omp.command",
        "LIBRI_OMP",
        "omp's command; unset, `omp`.",
    ),
    key(
        "harness.pi.command",
        "LIBRI_PI",
        "Pi's command; unset, `pi`.",
    ),
    key(
        "harness.muse.command",
        "LIBRI_MUSE",
        "Muse's command; unset, `muse`.",
    ),
    key(
        "harness.acp.command",
        "LIBRI_ACP",
        "An ACP agent's command line.",
    ),
    key(
        "harness.http.url",
        "LIBRI_URL",
        "A chat-completions endpoint.",
    ),
    key(
        "tools.forget",
        "LIBRI_FORGET",
        "Minutes after which a chat is forgotten; set, the person picks among the lengths in Settings.",
    ),
    key(
        "tools.search_socket",
        "LIBRI_SEARCH_SOCKET",
        "A Unix socket serving Parallel's search with the host's key added.",
    ),
    key(
        "tools.exa_socket",
        "LIBRI_EXA_SOCKET",
        "A Unix socket serving Exa's search with the host's key added; set, search offers Exa.",
    ),
    key(
        "tools.brave_socket",
        "LIBRI_BRAVE_SOCKET",
        "A Unix socket serving Brave's web search with the host's key added; set, search offers Brave.",
    ),
    // Remove once no host's config still names the old usage service.
    key(
        "tools.usage",
        "LIBRI_USAGE_UNUSED",
        "Accepted and ignored: Libri reads subscription limits itself.",
    ),
    key("tools.mail", "LIBRI_MAIL", "The mail accounts file."),
    key(
        "tools.imap",
        "LIBRI_IMAP",
        "The IMAP server mail is read from.",
    ),
    key(
        "tools.smtp",
        "LIBRI_SMTP",
        "The SMTP server mail is sent through.",
    ),
    // The voice until the person picks one (site.rs `voice`).
    key(
        "x.voice",
        "LIBRI_VOICE",
        "The voice until the person picks one; xAI itself is connected in Settings, Connections.",
    ),
    key(
        "computer.run",
        "LIBRI_RUN",
        "The address of the assistant's computer, for `run_code`; unset, no `run_code`.",
    ),
    key(
        "computer.runs",
        "LIBRI_RUNS",
        "The directory that computer shares with Libri for each run's files.",
    ),
    key(
        "browser.enable",
        "LIBRI_AGENT_BROWSER",
        "`true` gives agents Libri's own browser; unset, off.",
    ),
    key(
        "browser.chromium",
        "LIBRI_CHROMIUM",
        "Chromium's command; unset, `chromium`. Named, it also renders a still of each Library page for its card.",
    ),
    key(
        "browser.xvfb",
        "LIBRI_XVFB",
        "Xvfb's command, for the browser's hidden display; unset, `Xvfb`.",
    ),
    key(
        "browser.relay",
        "LIBRI_BROWSER_RELAY",
        "The address of a DevTools relay to the person's own browser (an extension relay such as OpenCode Browse's, `http://127.0.0.1:19988`); unset, agents use Libri's browser alone.",
    ),
    key(
        "browser.relay_token",
        "LIBRI_BROWSER_RELAY_TOKEN",
        "The file holding the token the relay asks for.",
    ),
    key(
        "browser.use",
        "LIBRI_BROWSER_USE",
        "Where a task with the person's logins browses: `logins` (unset), their own browser while it is connected, else Libri's; `own`, Libri's only; `yours`, their own only.",
    ),
    key(
        "share.listen",
        "LIBRI_SHARE_LISTEN",
        "Where board links are served (address:port), on a listener that serves them and nothing else; a public name points here. Off unless set.",
    ),
    key(
        "share.url",
        "LIBRI_SHARE_URL",
        "The public address board links are made under, e.g. `https://share.example.net`; without it no link can be made.",
    ),
    key(
        "compact.at",
        "LIBRI_COMPACT_AT",
        "The context size, in tokens, at which every harness compacts a chat's session (Claude Code's autoCompactWindow, Codex's model_auto_compact_token_limit); the person's pick in Settings goes first; unset, 200000.",
    ),
    key(
        "a2a.token_file",
        "LIBRI_A2A_TOKEN",
        "Turns on A2A: a file of the bearer token other agents present and this one sends. Off unless set.",
    ),
    key(
        "a2a.peers",
        "LIBRI_A2A",
        "Other agents this one may send to, `Name=url` each.",
    ),
];

/// Where Libri listens unless told otherwise: loopback, so a fresh install
/// is reachable from this machine alone until the person opens it up.
pub const LISTEN: &str = "127.0.0.1:6780";

/// What the command line sets before its command, as Syncthing's and
/// code-server's flags do: these win over the environment and the file.
#[derive(Debug, Default, PartialEq)]
pub struct Flags {
    pub config: Option<PathBuf>,
    /// A second Libri for the same user: its own config, state and service,
    /// named `libri-NAME` where the default one is `libri`.
    pub instance: Option<String>,
    pub listen: Option<String>,
    pub port: Option<u16>,
}

impl Flags {
    /// Takes the flags off the front of `args`, leaving the command.
    pub fn take(args: &mut Vec<String>) -> Result<Self, String> {
        let mut f = Flags::default();
        while let Some(flag) = args.first().filter(|a| a.starts_with("--")).cloned() {
            let (name, inline) = match flag.split_once('=') {
                Some((n, v)) => (n.to_owned(), Some(v.to_owned())),
                None => (flag.clone(), None),
            };
            if !matches!(
                name.as_str(),
                "--config" | "--instance" | "--listen" | "--port"
            ) {
                break;
            }
            args.remove(0);
            let value = match inline {
                Some(v) => v,
                None if !args.is_empty() => args.remove(0),
                None => return Err(format!("{name} needs a value")),
            };
            match name.as_str() {
                "--config" => f.config = Some(PathBuf::from(value)),
                "--instance" => f.instance = Some(instance_name(&value)?),
                "--listen" => f.listen = Some(value),
                _ => {
                    f.port = Some(
                        value
                            .parse()
                            .map_err(|_| format!("--port {value}: not a port number"))?,
                    )
                }
            }
        }
        if f.listen.is_some() && f.port.is_some() {
            return Err("give --listen ADDRESS:PORT or --port PORT, not both".into());
        }
        Ok(f)
    }

    /// The flags again, as a service unit passes them to the binary.
    pub fn args(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(c) = &self.config {
            // A unit runs from no particular directory.
            let c = std::path::absolute(c).unwrap_or_else(|_| c.clone());
            out.extend(["--config".into(), c.to_string_lossy().into_owned()]);
        }
        if let Some(i) = &self.instance {
            out.extend(["--instance".into(), i.clone()]);
        }
        if let Some(l) = &self.listen {
            out.extend(["--listen".into(), l.clone()]);
        }
        if let Some(p) = self.port {
            out.extend(["--port".into(), p.to_string()]);
        }
        out
    }
}

/// An instance's name goes into directory and unit names, so it keeps to
/// what both take: lowercase letters, digits and dashes.
fn instance_name(name: &str) -> Result<String, String> {
    let ok = !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(name.to_owned())
    } else {
        Err(format!(
            "--instance {name:?}: lowercase letters, digits and dashes only"
        ))
    }
}

/// `libri`, or `libri-NAME` for a named instance: the directory name its
/// config and state go under and its service's name.
pub fn instance_dir() -> String {
    match env::var("LIBRI_INSTANCE") {
        Ok(name) if !name.is_empty() => format!("libri-{name}"),
        _ => "libri".into(),
    }
}

/// The config file named, else the person's
/// (`$XDG_CONFIG_HOME/libri/config.toml`, `libri-NAME` for an instance).
fn config_path(named: Option<&Path>) -> Option<PathBuf> {
    named.map(Path::to_owned).or_else(|| {
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|base| base.join(instance_dir()).join("config.toml"))
    })
}

/// `address:port` with its port replaced, for `--port`.
fn with_port(listen: &str, port: u16) -> String {
    let host = listen.rsplit_once(':').map_or(listen, |(h, _)| h);
    format!("{host}:{port}")
}

/// The settings a config file gives, as environment pairs; an unknown key
/// is an error, so a misspelt one is not quietly ignored.
pub fn settings(text: &str) -> Result<Vec<(&'static str, String)>, String> {
    let table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    flatten("", &table, &mut out)?;
    Ok(out)
}

fn flatten(
    prefix: &str,
    table: &toml::Table,
    out: &mut Vec<(&'static str, String)>,
) -> Result<(), String> {
    for (k, v) in table {
        let key = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        if let toml::Value::Table(t) = v {
            flatten(&key, t, out)?;
            continue;
        }
        // The household left Libri in 0.9.31; a config from before it still
        // starts, so a host can take the new build before its new config.
        // Remove once no config in use has a [household] table.
        if key.starts_with("household.") {
            eprintln!("libri: {key} is no longer read; household settings left Libri");
            continue;
        }
        // Stills took the agents' Chromium in 0.9.39; a config from before
        // names it here. Remove once no config in use has daemon.browser.
        let key = if key == "daemon.browser" {
            eprintln!("libri: daemon.browser is now browser.chromium");
            "browser.chromium".to_owned()
        } else {
            key
        };
        let Some(Key { env: var, .. }) = KEYS.iter().find(|k| k.key == key) else {
            return Err(format!("unknown key {key}"));
        };
        let text = match v {
            toml::Value::String(s) => home(s),
            toml::Value::Integer(i) => i.to_string(),
            // A setting's presence turns it on.
            toml::Value::Boolean(true) => "1".into(),
            toml::Value::Boolean(false) => continue,
            toml::Value::Array(a) => a
                .iter()
                .map(|x| match x {
                    toml::Value::String(s) => Ok(s.clone()),
                    _ => Err(format!("{key}: a list of strings")),
                })
                .collect::<Result<Vec<_>, _>>()?
                .join(","),
            _ => return Err(format!("{key}: a string, number, true/false or list")),
        };
        out.push((var, text));
    }
    Ok(())
}

/// `~/` at the start of a path, as a shell would read it.
fn home(s: &str) -> String {
    match (s.strip_prefix("~/"), env::var("HOME")) {
        (Some(rest), Ok(h)) => format!("{h}/{rest}"),
        _ => s.to_owned(),
    }
}

/// Reads the config into the environment, leaving what is already set,
/// then fills in the defaults a lone install needs. Runs before any
/// thread starts: setting the environment is not safe once one has.
pub fn load(flags: &Flags) -> Result<(), String> {
    // SAFETY: load() runs before the runtime starts any thread.
    if let Some(name) = &flags.instance {
        unsafe { env::set_var("LIBRI_INSTANCE", name) };
    }
    if let Some(listen) = &flags.listen {
        unsafe { env::set_var("LIBRI_SITE", listen) };
    }
    let named = flags.config.as_deref();
    let path = config_path(named);
    let text = match &path {
        Some(p) => match fs::read_to_string(p) {
            Ok(t) => t,
            Err(e) if named.is_none() && e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("{}: {e}", p.display())),
        },
        None => String::new(),
    };
    let given = settings(&text).map_err(|e| match &path {
        Some(p) => format!("{}: {e}", p.display()),
        None => e,
    })?;
    for (var, value) in given {
        set_default(var, &value);
    }
    defaults()?;
    // The port alone, on whatever address the config or default gives.
    if let Some(port) = flags.port {
        let listen = with_port(
            &env::var("LIBRI_SITE").unwrap_or_else(|_| LISTEN.into()),
            port,
        );
        // SAFETY: as above, before any thread starts.
        unsafe { env::set_var("LIBRI_SITE", listen) };
    }
    Ok(())
}

fn set_default(var: &str, value: &str) {
    if env::var_os(var).is_none_or(|v| v.is_empty()) {
        // SAFETY: load() runs before the runtime starts any thread.
        unsafe { env::set_var(var, value) };
    }
}

fn defaults() -> Result<(), String> {
    let state = match env::var_os("STATE_DIRECTORY") {
        Some(s) => PathBuf::from(s),
        None => env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
            .ok_or("no state directory: set `state` in the config")?
            .join(instance_dir()),
    };
    private_dir(&state)?;
    set_default("STATE_DIRECTORY", &state.to_string_lossy());
    // A memory of its own beside the state, which this daemon serves:
    // never another's, which its owner serves.
    if env::var_os("HIPPO_DIR").is_none() {
        set_default("HIPPO_DIR", &state.join("hippo").to_string_lossy());
        set_default("LIBRI_HIPPO_SERVE", "1");
    }
    // hippo keeps Libri's record of the chat and writes no summaries: each
    // harness compacts its own session, and the memory's server recalls.
    // SAFETY: as set_default, before any thread starts.
    unsafe { env::set_var("HIPPO_BACKEND", "none") };
    set_default("LIBRI_SITE", LISTEN);
    let packaged = env::var_os("LIBRI_PACKAGED_SITE")
        .map(PathBuf::from)
        .or_else(|| Some(unpacked()?.join("share/libri/site")).filter(|p| p.exists()));
    if let Some(site) = packaged {
        set_default("LIBRI_SITE_ROOT", &site.to_string_lossy());
    }
    // An unpacked release keeps hippo beside libri, off any PATH, and the
    // harnesses this daemon runs call it by name too.
    if let Some(bin) = env::current_exe()
        .ok()
        .and_then(|e| Some(e.parent()?.to_owned()))
        .filter(|b| b.join("hippo").exists())
    {
        let path = env::var_os("PATH").unwrap_or_default();
        let dirs = std::iter::once(bin).chain(env::split_paths(&path));
        let joined = env::join_paths(dirs).map_err(|e| e.to_string())?;
        // SAFETY: load() runs before the runtime starts any thread.
        unsafe { env::set_var("PATH", joined) };
    }
    // Every harness installed is offered, as T3 Code and Paseo offer each
    // provider they find; the person switches any off in Settings. None found
    // leaves the default, whose failure names what is missing. Only a check
    // rig narrows the list, through the environment.
    let (found, _) = harnesses();
    if !found.is_empty() {
        set_default("LIBRI_HARNESSES", &found.join(","));
        set_default("LIBRI_BACKEND", found[0]);
    }
    set_default("LIBRI_PAGES", &state.join("pages").to_string_lossy());
    // The daemon serves the pages itself, so a link in chat works with no
    // site set up: at the public address, else how this machine is reached.
    let listen = env::var("LIBRI_SITE").unwrap_or_else(|_| LISTEN.into());
    let site = env::var("LIBRI_SITE_URL").unwrap_or_else(|_| local_url(&listen, &machine_label()));
    set_default("LIBRI_PAGES_URL", &site);
    set_default("LIBRI_NAME", &machine_label());
    let person = env::var("USER").unwrap_or_else(|_| "you".into());
    set_default("LIBRI_PERSON", &person);
    Ok(())
}

/// The address of a listener on this machine: an all-interfaces one by the
/// machine's name, which the tailnet or LAN resolves.
fn local_url(listen: &str, machine: &str) -> String {
    match listen.rsplit_once(':') {
        Some(("0.0.0.0" | "[::]", port)) => format!("http://{machine}:{port}"),
        _ => format!("http://{listen}"),
    }
}

/// The directory an unpacked release sits in: libri at `bin/libri` under it.
fn unpacked() -> Option<PathBuf> {
    Some(env::current_exe().ok()?.parent()?.parent()?.to_owned())
}

/// The native harnesses, each with the setting that names its command.
const HARNESSES: &[(&str, &str)] = &[
    ("claude", "LIBRI_CLAUDE"),
    ("codex", "LIBRI_CODEX"),
    ("opencode", "LIBRI_OPENCODE"),
    ("omp", "LIBRI_OMP"),
    ("pi", "LIBRI_PI"),
    ("muse", "LIBRI_MUSE"),
];

/// Whether a command names an executable file, directly or on PATH.
fn runnable(command: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let executable = |p: &Path| {
        p.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if command.contains('/') {
        return executable(Path::new(command));
    }
    let path = env::var_os("PATH").unwrap_or_default();
    env::split_paths(&path).any(|d| executable(&d.join(command)))
}

/// The native harnesses this machine has, and those it lacks.
pub fn harnesses() -> (Vec<&'static str>, Vec<&'static str>) {
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for (name, setting) in HARNESSES {
        let command = env::var(setting).ok().filter(|c| !c.is_empty());
        if runnable(command.as_deref().unwrap_or(name)) {
            found.push(*name);
        } else {
            missing.push(*name);
        }
    }
    (found, missing)
}

/// Says which harnesses this machine has: Libri ships none, so a person
/// must hear which of them it can answer through.
pub fn report_harnesses() {
    let (found, missing) = harnesses();
    if found.is_empty() {
        eprintln!(
            "libri: no harness found (looked for {}). Libri answers through one of them: \
             install one, sign in to it, and start Libri again.",
            missing.join(", ")
        );
    } else if missing.is_empty() {
        eprintln!("libri: harnesses found: {}.", found.join(", "));
    } else {
        eprintln!(
            "libri: harnesses found: {}. Not found: {}.",
            found.join(", "),
            missing.join(", ")
        );
    }
}

/// The machine's hostname, when it can be read.
fn hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer outlives the call, which writes within its length.
    let done = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if done != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0)?;
    String::from_utf8(buf[..end].to_vec()).ok()
}

/// A host's name in the app: the machine's hostname.
pub fn machine_label() -> String {
    default_name(hostname())
}

/// What the chat calls the assistant, given the deployment's `name`: Libri,
/// except where a deployment named it otherwise, which keeps showing that.
pub fn assistant_label(name: &str) -> &str {
    if name == machine_label() {
        crate::model::ASSISTANT
    } else {
        name
    }
}

/// A hostname, without any domain, else the product.
fn default_name(hostname: Option<String>) -> String {
    hostname
        .as_deref()
        .and_then(|h| h.split('.').next())
        .filter(|h| !h.is_empty())
        .unwrap_or("Libri")
        .to_owned()
}

pub(crate) fn private_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))
}

/// Starts this daemon's own hippo when it keeps one (`hippo.serve`), as a
/// child that dies with it; `hippo` on PATH, where an unpacked release
/// puts the one beside this binary first (`load`).
pub fn serve_hippo() -> Result<(), String> {
    if env::var_os("LIBRI_HIPPO_SERVE").is_none() {
        return Ok(());
    }
    let dir = PathBuf::from(env::var_os("HIPPO_DIR").ok_or("HIPPO_DIR is not set")?);
    private_dir(&dir)?;
    // Chat mode: only what this daemon logs, its record of the chat. The
    // harnesses' own sessions are the memory server's to read (hippo2).
    // Lives as long as the daemon.
    run_hippo(&dir, "chat").map(drop)
}

/// A hippo this daemon keeps running, until `stop`.
#[derive(Default)]
pub struct HippoChild {
    stopped: std::sync::atomic::AtomicBool,
    pid: std::sync::atomic::AtomicU32,
}

impl HippoChild {
    /// Ends the hippo and starts no other in its place.
    pub fn stop(&self) {
        use std::sync::atomic::Ordering;
        self.stopped.store(true, Ordering::SeqCst);
        let pid = self.pid.swap(0, Ordering::SeqCst);
        if let Ok(pid) = i32::try_from(pid)
            && pid > 0
        {
            // SAFETY: a signal to the child this daemon started.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
}

/// Runs `hippo serve <mode>` on the store in `dir`, as a child that dies
/// with this daemon, started again after it ends, once its socket is there.
pub fn run_hippo(dir: &Path, mode: &str) -> Result<std::sync::Arc<HippoChild>, String> {
    use std::sync::atomic::Ordering;
    let mut cmd = std::process::Command::new("hippo");
    cmd.args(["serve", mode]).env("HIPPO_DIR", dir);
    // SAFETY: prctl alone, between fork and exec.
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
    let sock = dir.join("hippo.sock");
    let child = std::sync::Arc::new(HippoChild::default());
    let kept = child.clone();
    let (told, started) = std::sync::mpsc::channel();
    // A hippo of this daemon's has no systemd unit, so this thread does what
    // Restart=always does. It must outlive each child: PDEATHSIG follows the
    // thread that forked, not the process.
    std::thread::spawn(move || {
        let mut told = Some(told);
        while !kept.stopped.load(Ordering::SeqCst) {
            match cmd.spawn() {
                Ok(mut child) => {
                    kept.pid.store(child.id(), Ordering::SeqCst);
                    // Stopped while it started: the stop found no pid yet.
                    if kept.stopped.load(Ordering::SeqCst) {
                        let _ = child.kill();
                    }
                    // The socket appears once the store is open.
                    for _ in 0..100 {
                        if told.is_none() || sock.exists() {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    if let Some(t) = told.take() {
                        let _ = t.send(Ok(()));
                    }
                    match child.wait() {
                        Ok(status) => eprintln!("libri: hippo ended: {status}"),
                        Err(e) => eprintln!("libri: hippo: {e}"),
                    }
                }
                Err(e) => {
                    let e = format!("hippo: {e}");
                    match told.take() {
                        Some(t) => {
                            let _ = t.send(Err(e));
                            return;
                        }
                        None => eprintln!("libri: {e}"),
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    });
    started.recv().map_err(|e| e.to_string())??;
    Ok(child)
}

// ---- Identity ----

/// The host's own files: its id, its paired devices, the links not yet
/// used.
pub(crate) fn dir(state: &Path) -> PathBuf {
    state.join("host")
}

/// The id the app knows this host by, made once: addresses change, and a
/// host reached two ways is still one host.
pub fn id(state: &Path) -> Result<String, String> {
    let file = dir(state).join("id");
    if let Ok(id) = fs::read_to_string(&file) {
        return Ok(id.trim().to_owned());
    }
    private_dir(&dir(state))?;
    let id = hex(&random::<16>()?);
    write_private(&file, &id)?;
    Ok(id)
}

// ---- Pairing ----

/// What a paired device may do: `read` sees the chat and changes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Full,
    Read,
}

/// How long a pairing link stays good.
const LINK_LIFE: u64 = 10 * 60;

/// A link made and not yet used: its secret kept only as a hash.
#[derive(Serialize, Deserialize)]
struct Link {
    hash: String,
    scope: Scope,
    expires: u64,
}

/// A device paired with the host, its token kept only as a hash.
#[derive(Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub scope: Scope,
    hash: String,
    pub paired: u64,
}

/// Makes a one-time pairing secret for a device to trade for a token.
pub fn pair(state: &Path, scope: Scope) -> Result<String, String> {
    let secret = token()?;
    let now = unix();
    let mut links: Vec<Link> = read_json(&dir(state).join("links.json"));
    links.retain(|l| l.expires > now);
    links.push(Link {
        hash: hash(&secret),
        scope,
        expires: now + LINK_LIFE,
    });
    write_json(state, "links.json", &links)?;
    Ok(secret)
}

/// Trades a pairing secret, once, for a device and its token.
pub fn redeem(state: &Path, secret: &str, name: &str) -> Result<(Device, String), String> {
    let now = unix();
    let mut links: Vec<Link> = read_json(&dir(state).join("links.json"));
    let h = hash(secret);
    let at = links
        .iter()
        .position(|l| l.expires > now && same(&l.hash, &h))
        .ok_or("That pairing link is used or out of date.")?;
    let link = links.remove(at);
    links.retain(|l| l.expires > now);
    write_json(state, "links.json", &links)?;
    admit(state, name, link.scope)
}

/// Pairs a new device: a token for it, kept here only as a hash.
fn admit(state: &Path, name: &str, scope: Scope) -> Result<(Device, String), String> {
    let token = token()?;
    let device = Device {
        id: hex(&random::<4>()?),
        name: name.chars().take(80).collect::<String>().trim().to_owned(),
        scope,
        hash: hash(&token),
        paired: unix(),
    };
    let mut all = devices(state);
    all.push(device.clone());
    write_json(state, "devices.json", &all)?;
    Ok((device, token))
}

pub fn devices(state: &Path) -> Vec<Device> {
    read_json(&dir(state).join("devices.json"))
}

/// The device a token belongs to, if any still does.
pub fn device(state: &Path, token: &str) -> Option<Device> {
    let h = hash(token);
    devices(state).into_iter().find(|d| same(&d.hash, &h))
}

/// Unpairs a device: its token stops working at once.
pub fn revoke(state: &Path, id: &str) -> Result<Device, String> {
    let mut all = devices(state);
    let at = all
        .iter()
        .position(|d| d.id == id)
        .ok_or_else(|| format!("no device {id}"))?;
    let gone = all.remove(at);
    write_json(state, "devices.json", &all)?;
    Ok(gone)
}

/// The link a device opens to pair: the address in the URL, the secret in
/// its fragment, which browsers never send to a server or keep in logs.
pub fn link(base: &str, secret: &str) -> String {
    format!("{}/#pair={secret}", base.trim_end_matches('/'))
}

/// Where this host's page is reached: its configured address first, then
/// each address it answers on.
pub fn bases() -> Vec<String> {
    let url = env::var("LIBRI_SITE_URL").ok().filter(|u| !u.is_empty());
    url.into_iter()
        .chain(addresses().into_iter().map(|a| format!("http://{a}")))
        .collect()
}

/// The addresses this host answers on, for a client to learn as further
/// routes: each interface's when listening on all, loopback and
/// link-local aside.
pub fn addresses() -> Vec<String> {
    let listen = env::var("LIBRI_SITE").unwrap_or_else(|_| LISTEN.into());
    let Ok(sock) = listen.parse::<std::net::SocketAddr>() else {
        return vec![listen];
    };
    if !sock.ip().is_unspecified() {
        return vec![sock.to_string()];
    }
    let port = sock.port();
    interfaces()
        .into_iter()
        .filter(|ip| !ip.is_loopback() && (sock.is_ipv6() || ip.is_ipv4()))
        .filter(|ip| match ip {
            std::net::IpAddr::V4(v) => !v.is_link_local(),
            std::net::IpAddr::V6(v) => !v.is_unicast_link_local(),
        })
        .map(|ip| std::net::SocketAddr::new(ip, port).to_string())
        .collect()
}

fn interfaces() -> Vec<std::net::IpAddr> {
    let mut out = Vec::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills a list freed below; each entry's address is
    // read as the family it names.
    unsafe {
        if libc::getifaddrs(&mut head) != 0 {
            return out;
        }
        let mut at = head;
        while !at.is_null() {
            let a = (*at).ifa_addr;
            if !a.is_null() {
                match (*a).sa_family as i32 {
                    libc::AF_INET => {
                        let v4 = &*(a as *const libc::sockaddr_in);
                        out.push(std::net::Ipv4Addr::from(u32::from_be(v4.sin_addr.s_addr)).into());
                    }
                    libc::AF_INET6 => {
                        let v6 = &*(a as *const libc::sockaddr_in6);
                        out.push(std::net::Ipv6Addr::from(v6.sin6_addr.s6_addr).into());
                    }
                    _ => {}
                }
            }
            at = (*at).ifa_next;
        }
        libc::freeifaddrs(head);
    }
    out
}

// ---- Files and secrets ----

pub(crate) fn read_json<T: for<'de> Deserialize<'de>>(file: &Path) -> Vec<T> {
    fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub(crate) fn write_json<T: Serialize>(state: &Path, name: &str, value: &T) -> Result<(), String> {
    private_dir(&dir(state))?;
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    write_private(&dir(state).join(name), &text)
}

pub(crate) fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::io::Write::write_all(&mut f, text.as_bytes()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

pub(crate) fn random<const N: usize>() -> Result<[u8; N], String> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(b)
}

pub(crate) fn token() -> Result<String, String> {
    use base64::Engine;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random::<32>()?))
}

pub(crate) fn hash(secret: &str) -> String {
    hex(digest::digest(&digest::SHA256, secret.as_bytes()).as_ref())
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Compares in time independent of where two hashes first differ.
pub(crate) fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |d, (x, y)| d | (x ^ y)) == 0
}

pub fn unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn takes_the_flags_before_the_command() {
        let mut args = strings(&["--instance", "work", "--port=6781", "pair", "--read"]);
        let f = Flags::take(&mut args).unwrap();
        assert_eq!(f.instance.as_deref(), Some("work"));
        assert_eq!(f.port, Some(6781));
        assert_eq!(args, strings(&["pair", "--read"]));
        assert_eq!(f.args(), strings(&["--instance", "work", "--port", "6781"]));

        let mut none = strings(&["serve"]);
        assert_eq!(Flags::take(&mut none).unwrap(), Flags::default());
        assert!(Flags::take(&mut strings(&["--port", "x"])).is_err());
        assert!(Flags::take(&mut strings(&["--instance"])).is_err());
        assert!(Flags::take(&mut strings(&["--instance", "../x"])).is_err());
        assert!(Flags::take(&mut strings(&["--listen", "0.0.0.0:1", "--port", "2"])).is_err());
    }

    #[test]
    fn a_port_keeps_its_address() {
        assert_eq!(with_port("127.0.0.1:6780", 6781), "127.0.0.1:6781");
        assert_eq!(with_port("[::]:6780", 9), "[::]:9");
        assert_eq!(with_port("0.0.0.0:8951", 8952), "0.0.0.0:8952");
    }

    #[test]
    fn a_listener_is_reached_at_its_address_or_the_machines_name() {
        assert_eq!(local_url(LISTEN, "box"), "http://127.0.0.1:6780");
        assert_eq!(local_url("0.0.0.0:6780", "box"), "http://box:6780");
        assert_eq!(local_url("[::]:6780", "box"), "http://box:6780");
        assert_eq!(local_url("100.1.2.3:80", "box"), "http://100.1.2.3:80");
    }

    #[test]
    fn a_host_is_named_for_its_machine_until_told_otherwise() {
        assert_eq!(default_name(Some("box".into())), "box");
        assert_eq!(default_name(Some("fw3.tail.ts.net".into())), "fw3");
        assert_eq!(default_name(Some(String::new())), "Libri");
        assert_eq!(default_name(None), "Libri");
    }

    #[test]
    fn the_assistant_keeps_a_deployment_s_name_and_no_hostname() {
        assert_eq!(assistant_label("Iris"), "Iris");
        assert_eq!(assistant_label(&machine_label()), crate::model::ASSISTANT);
    }

    fn temp() -> PathBuf {
        let d = env::temp_dir().join(format!("libri-host-{}", hex(&random::<6>().unwrap())));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn maps_every_key_to_its_setting() {
        let mut got = settings(
            r#"
            name = "Ada"
            [daemon]
            listen = "0.0.0.0:6780"
            open = false
            browser = "/bin/chromium"
            [harness.opencode]
            port = 4096
            [a2a]
            peers = ["Kit=http://127.0.0.1:6781"]
            [household]
            members = "a=A=Ada,b=B=Bob"
            "#,
        )
        .unwrap();
        got.sort();
        assert_eq!(
            got,
            vec![
                ("LIBRI_A2A", "Kit=http://127.0.0.1:6781".into()),
                ("LIBRI_CHROMIUM", "/bin/chromium".into()),
                ("LIBRI_NAME", "Ada".into()),
                ("LIBRI_OPENCODE_PORT", "4096".into()),
                ("LIBRI_SITE", "0.0.0.0:6780".into()),
            ]
        );
        assert!(
            settings("nmae = 1")
                .unwrap_err()
                .contains("unknown key nmae")
        );
        assert!(settings("[daemon]\nlisten = 1.5").is_err());
    }

    #[test]
    fn keys_and_settings_are_each_named_once() {
        for (i, k) in KEYS.iter().enumerate() {
            assert!(
                !KEYS[i + 1..]
                    .iter()
                    .any(|o| o.key == k.key || o.env == k.env),
                "{}",
                k.key
            );
            assert!(!k.about.is_empty(), "{}", k.key);
        }
    }

    #[test]
    fn a_link_pairs_one_device_once_and_revoking_ends_it() {
        let state = temp();
        let secret = pair(&state, Scope::Read).unwrap();
        let (d, token) = redeem(&state, &secret, "Phone").unwrap();
        assert_eq!((d.name.as_str(), d.scope), ("Phone", Scope::Read));
        assert!(redeem(&state, &secret, "Again").is_err());
        assert!(redeem(&state, "made-up", "Guess").is_err());
        assert_eq!(device(&state, &token).unwrap().id, d.id);
        // Only hashes are kept.
        let kept = fs::read_to_string(dir(&state).join("devices.json")).unwrap();
        assert!(!kept.contains(&token));
        revoke(&state, &d.id).unwrap();
        assert!(device(&state, &token).is_none());
        fs::remove_dir_all(state).unwrap();
    }

    #[test]
    fn the_id_stays() {
        let state = temp();
        let a = id(&state).unwrap();
        assert_eq!(a.len(), 32);
        assert_eq!(id(&state).unwrap(), a);
        fs::remove_dir_all(state).unwrap();
    }

    #[test]
    fn the_secret_rides_in_the_fragment() {
        assert_eq!(
            link("http://100.1.2.3:6780/", "s3cr3t"),
            "http://100.1.2.3:6780/#pair=s3cr3t"
        );
    }
}
