//! The person's connections: MCP servers elsewhere, added by name and
//! address, as Claude Desktop's custom connectors and Claude Code's
//! `claude mcp add` take them. An address is a URL for a remote server or a
//! command for one run here; a remote server either takes a header the
//! person pastes or signs them in by OAuth, discovered from the server as
//! the MCP authorization spec has it (rmcp's `auth`). Nothing here knows
//! any one service: mail, a calendar or a house are entries in the list.
//!
//! Every harness reaches a connection the same way, through Libri's relay
//! (`libri connection STATE NAME`), a stdio server that passes each message
//! on with the connection's secrets added. So no harness config holds a
//! secret, an OAuth session outlives the hour its token lasts, and the
//! tools a connection withholds are withheld on every harness.
//!
//! OAuth returns the browser by way of one fixed page, `BACK`, as Home
//! Assistant's sign-ins return by my.home-assistant.io: most installs have
//! no public address a server could send the browser to, and a phone cannot
//! reach a host's `127.0.0.1`. Libri registers that page (dynamic client
//! registration, as a `web` client), and carries the address the browser
//! reached Libri at in the OAuth `state` beside the random nonce the sign-in
//! is kept by. The page, static and keeping nothing, sends the browser on to
//! that address's `/api/connections/back` and nowhere else. The code it sees
//! on the way is useless without the PKCE verifier, which never leaves the
//! host.
//!
//! A browser on the host itself, or a start with no address known, returns
//! straight to this host's `127.0.0.1` port, the return address servers
//! accept from a native client (RFC 8252 section 7.3), as Claude Code and
//! Codex sign in. Where the browser cannot come back on its own, the person
//! pastes the address it ended on into Libri, as Claude Code and Codex take
//! a pasted code on a remote machine. A server that refuses the page as a
//! return address (Fastmail takes no https one from a dynamic client) gets
//! the loopback one, and the paste.
//!
//! The Android app signs in as a native app does (RFC 8252 section 7.1):
//! the server returns to `APP_BACK`, a private-use scheme the app answers,
//! and the app hands the address on to the host to finish.

use crate::host::{private_dir, write_private};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rmcp::transport::auth::{
    OAuthHttpClient, OAuthHttpClientFuture, OAuthHttpRedirectPolicy, OAuthHttpRequest,
    OAuthTokenResponse,
};
use rmcp::transport::{
    AuthError, AuthorizationManager, AuthorizationRequest, AuthorizationSession,
    CredentialRefreshGuard, CredentialStore, StoredCredentials,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How often the daemon renews the sign-ins, which keeps each refresh
/// token in use while no harness is.
pub const RENEWAL: Duration = Duration::from_secs(10 * 60);
/// How long a started sign-in waits for its code.
const PATIENCE: Duration = Duration::from_secs(15 * 60);
/// The page every install's sign-ins return through (`connections.return_page`).
pub const BACK: &str = "https://libri.is/connect/back";
/// Where a sign-in the Android app starts returns: the app's own scheme,
/// its application id.
pub const APP_BACK: &str = "is.libri:/oauth/back";
/// The name Libri's own server goes by, which no connection may take.
const OWN: &str = "libri";

pub fn dir(state: &Path) -> PathBuf {
    state.join("connections")
}

fn servers(state: &Path) -> PathBuf {
    dir(state).join("servers")
}

fn file(state: &Path, name: &str, ext: &str) -> PathBuf {
    servers(state).join(format!("{name}.{ext}"))
}

/// One connection as kept, secrets and all: the file is the person's alone.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    /// What its tools are named by, as `mcp__NAME__tool`.
    pub name: String,
    /// A remote server's address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Or the program and its arguments, for a server run here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    /// Headers sent to a remote server, a pasted token among them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// The environment a local server runs with.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Tools no harness is offered or may call: those set Off.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub withheld: Vec<String>,
    /// The person's Allow or Ask for a tool, over what its annotations
    /// make of it (`default_rule`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, Rule>,
    /// Whether harnesses get it. One switched off is kept, ready, unused.
    #[serde(default = "on")]
    pub on: bool,
}

fn on() -> bool {
    true
}

/// What a harness may do with one tool, as Claude's and ChatGPT's
/// connectors set it per tool: run it, ask the person before each call, or
/// not have it at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rule {
    Allow,
    Ask,
    Off,
}

/// A tool's rule before the person sets one, from the server's own
/// annotations (MCP's ToolAnnotations): one that only reads runs; any other
/// asks first, a draft or a new event as much as a send or a delete, since
/// the person decides which writes run unasked.
pub fn default_rule(tool: &Value) -> Rule {
    let a = &tool["annotations"];
    if a["readOnlyHint"] == Value::Bool(true) {
        Rule::Allow
    } else {
        Rule::Ask
    }
}

impl Connection {
    /// `tool`'s rule here, `listed` being its entry in the server's list.
    pub fn rule(&self, tool: &str, listed: Option<&Value>) -> Rule {
        if self.withheld.iter().any(|t| t == tool) {
            return Rule::Off;
        }
        self.rules
            .get(tool)
            .copied()
            .unwrap_or_else(|| listed.map_or(Rule::Ask, default_rule))
    }
}

impl Connection {
    fn address(&self) -> String {
        match &self.url {
            Some(url) => url.clone(),
            None => shlex::try_join(self.command.iter().map(String::as_str))
                .unwrap_or_else(|_| self.command.join(" ")),
        }
    }

    /// The names of its secrets, which are all a device is ever shown.
    fn secrets(&self) -> Vec<String> {
        self.headers
            .keys()
            .chain(self.env.keys())
            .cloned()
            .collect()
    }
}

/// The connections, by name.
pub fn all(state: &Path) -> Vec<Connection> {
    let mut out: Vec<Connection> = fs::read_dir(servers(state))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter(|e| !e.file_name().to_string_lossy().ends_with(".oauth.json"))
        .filter_map(|e| serde_json::from_str(&fs::read_to_string(e.path()).ok()?).ok())
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn get(state: &Path, name: &str) -> Result<Connection, String> {
    let text = fs::read_to_string(file(state, &named(name)?, "json"))
        .map_err(|_| format!("There is no connection {name}."))?;
    serde_json::from_str(&text).map_err(|e| format!("{name}: {e}"))
}

fn keep(state: &Path, c: &Connection) -> Result<(), String> {
    private_dir(&servers(state))?;
    write_private(
        &file(state, &c.name, "json"),
        &serde_json::to_string_pretty(c).unwrap(),
    )
}

fn forget(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("{}: {e}", path.display()))
        }
        _ => Ok(()),
    }
}

/// A name a harness can carry in its tool names.
fn named(name: &str) -> Result<String, String> {
    let name = name.trim().to_lowercase();
    let fits = !name.is_empty()
        && name.len() <= 40
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !fits {
        return Err("A name is letters, digits, - and _, up to 40.".into());
    }
    if name == OWN {
        return Err(format!("{OWN} is Libri's own."));
    }
    Ok(name)
}

/// A connection as a person adds or changes it.
#[derive(Debug, Default, Deserialize)]
pub struct Change {
    pub name: String,
    /// A URL, or a command line.
    pub address: String,
    /// One per line, `Name: value` for a header or `NAME=value` for the
    /// environment. Left out, the ones kept stay.
    #[serde(default)]
    pub secrets: Option<String>,
    #[serde(default)]
    pub withheld: Vec<String>,
    /// Left out, a new one is on and a kept one stays as it was.
    #[serde(default)]
    pub on: Option<bool>,
}

/// Adds a connection, or changes the one of that name.
pub fn put(state: &Path, change: Change) -> Result<Connection, String> {
    let name = named(&change.name)?;
    let was = get(state, &name).ok();
    let address = change.address.trim();
    let mut c = Connection {
        name,
        withheld: change
            .withheld
            .iter()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect(),
        on: change
            .on
            .unwrap_or_else(|| was.as_ref().is_none_or(|w| w.on)),
        rules: was.as_ref().map(|w| w.rules.clone()).unwrap_or_default(),
        ..Connection::default()
    };
    if address.starts_with("http://") || address.starts_with("https://") {
        let url = reqwest::Url::parse(address).map_err(|e| format!("{address}: {e}"))?;
        c.url = Some(url.to_string());
    } else {
        c.command = shlex::split(address)
            .filter(|w| !w.is_empty())
            .ok_or("The address is a URL, or a command to run.")?;
    }
    match (&change.secrets, &was) {
        (Some(text), _) => {
            let (headers, env) = secrets(text, c.url.is_some())?;
            c.headers = headers;
            c.env = env;
        }
        // Kept while it is the same kind of server: a header means nothing
        // to a program, nor its environment to a URL.
        (None, Some(w)) if w.url.is_some() == c.url.is_some() => {
            c.headers = w.headers.clone();
            c.env = w.env.clone();
        }
        _ => {}
    }
    // A sign-in is for the server it was made with.
    if was.as_ref().is_some_and(|w| w.url != c.url) {
        forget(&file(state, &c.name, "oauth.json"))?;
    }
    keep(state, &c)?;
    Ok(c)
}

/// Headers for a remote server, or the environment for a local one.
type Secrets = (BTreeMap<String, String>, BTreeMap<String, String>);

fn secrets(text: &str, remote: bool) -> Result<Secrets, String> {
    let mut pairs = BTreeMap::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (k, v) = line
            .split_once(if remote { ':' } else { '=' })
            .ok_or_else(|| {
                if remote {
                    "Each line is a header, as Authorization: Bearer …".to_owned()
                } else {
                    "Each line is a variable, as API_KEY=…".to_owned()
                }
            })?;
        let k = k.trim();
        if k.is_empty() || k.contains(char::is_whitespace) {
            return Err(format!("{k:?} is no name."));
        }
        pairs.insert(k.to_owned(), v.trim().to_owned());
    }
    Ok(if remote {
        (pairs, BTreeMap::new())
    } else {
        (BTreeMap::new(), pairs)
    })
}

/// Removes a connection and its sign-in here. The service keeps the grant
/// listed until the person removes it there.
pub fn remove(state: &Path, name: &str) -> Result<(), String> {
    let name = named(name)?;
    for ext in ["json", "oauth.json", "lock", "tools.json"] {
        forget(&file(state, &name, ext))?;
    }
    Ok(())
}

/// Sets what a connection withholds and whether it is on, all else kept.
pub fn settle(state: &Path, name: &str, withheld: Vec<String>, on: bool) -> Result<(), String> {
    let mut c = get(state, name)?;
    c.withheld = withheld;
    c.on = on;
    keep(state, &c)
}

/// Sets one tool's rule on connection `name`: Off withholds it, Allow and
/// Ask are kept as the person's over its annotations.
pub fn set_rule(state: &Path, name: &str, tool: &str, rule: Rule) -> Result<(), String> {
    let mut c = get(state, name)?;
    c.withheld.retain(|t| t != tool);
    c.rules.remove(tool);
    if rule == Rule::Off {
        c.withheld.push(tool.to_owned());
        c.withheld.sort();
    } else {
        c.rules.insert(tool.to_owned(), rule);
    }
    keep(state, &c)
}

/// The tools connection `name`'s server listed last, as the relay or a
/// test saw them, annotations and all: Settings lists them from here.
pub fn listed(state: &Path, name: &str) -> Vec<Value> {
    fs::read_to_string(file(state, name, "tools.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn remember(state: &Path, name: &str, tools: &[Value]) {
    let slim: Vec<Value> = tools
        .iter()
        .map(|t| json!({"name": t["name"], "description": t["description"], "annotations": t["annotations"]}))
        .collect();
    if listed(state, name) != slim {
        let _ = write_private(
            &file(state, name, "tools.json"),
            &Value::Array(slim).to_string(),
        );
    }
}

/// Each tool of a connection as Settings lists it: its rule and the rule
/// its annotations give, the ones withheld included.
fn tools_shown(state: &Path, c: &Connection) -> Vec<Value> {
    let listed = listed(state, &c.name);
    let mut out: Vec<Value> = listed
        .iter()
        .filter_map(|t| {
            let name = t["name"].as_str()?;
            Some(json!({
                "name": name,
                "description": t["description"].as_str().and_then(|d| d.lines().next()),
                "rule": c.rule(name, Some(t)),
                "default": default_rule(t),
            }))
        })
        .collect();
    for w in &c.withheld {
        if !listed.iter().any(|t| t["name"] == w.as_str()) {
            out.push(json!({"name": w, "rule": Rule::Off, "default": Rule::Ask}));
        }
    }
    out
}

/// Whether the person has signed in to a connection's server.
pub fn signed_in(state: &Path, name: &str) -> bool {
    credentials(state, name).is_some_and(|s| s.token_response.is_some())
}

/// A connection as a device sees it: no secret, only its name.
pub fn shown(state: &Path, c: &Connection) -> Value {
    let creds = credentials(state, &c.name);
    let account = creds
        .as_ref()
        .and_then(|s| s.token_response.as_ref())
        .and_then(|t| {
            let extra = &t.extra_fields().0;
            ["username", "email"]
                .iter()
                .find_map(|k| extra.get(*k)?.as_str().map(str::to_owned))
        });
    json!({
        "name": c.name,
        "address": c.address(),
        "remote": c.url.is_some(),
        "secrets": c.secrets(),
        "withheld": c.withheld,
        "tools": tools_shown(state, c),
        "on": c.on,
        "signed_in": signed_in(state, &c.name),
        "account": account,
    })
}

/// The connections switched on, as harnesses get them: each one Libri's
/// relay.
pub fn entries(
    state: &Path,
    env: &serde_json::Map<String, Value>,
) -> Result<Vec<(String, Value)>, String> {
    all(state)
        .into_iter()
        .filter(|c| c.on)
        .map(|c| entry(state, &c.name, env))
        .collect()
}

fn entry(
    state: &Path,
    name: &str,
    env: &serde_json::Map<String, Value>,
) -> Result<(String, Value), String> {
    let program = crate::servers::program()?;
    Ok((
        name.to_owned(),
        json!({"type": "stdio", "command": program,
        "args": ["connection", state, name], "env": env}),
    ))
}

// OAuth, by rmcp's state machine, its credentials kept in a file beside the
// connection and renewed under a lock there: a refresh replaces the refresh
// token at many servers, and the daemon and each harness's relay renew.

fn credentials(state: &Path, name: &str) -> Option<StoredCredentials> {
    let text = fs::read_to_string(file(state, name, "oauth.json")).ok()?;
    serde_json::from_str(&text).ok()
}

struct Store {
    file: PathBuf,
    lock: PathBuf,
}

impl Store {
    fn of(state: &Path, name: &str) -> Self {
        Self {
            file: file(state, name, "oauth.json"),
            lock: file(state, name, "lock"),
        }
    }
}

fn store_failed(e: impl std::fmt::Display) -> AuthError {
    AuthError::CredentialStoreError(e.to_string())
}

#[async_trait::async_trait]
impl CredentialStore for Store {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        match fs::read_to_string(&self.file) {
            Ok(text) => serde_json::from_str(&text).map(Some).map_err(store_failed),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(store_failed(e)),
        }
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let dir = self.file.parent().expect("a connection's file is in a dir");
        private_dir(dir).map_err(store_failed)?;
        let text = serde_json::to_string_pretty(&credentials).map_err(store_failed)?;
        write_private(&self.file, &text).map_err(store_failed)
    }

    async fn clear(&self) -> Result<(), AuthError> {
        forget(&self.file).map_err(store_failed)
    }

    async fn acquire_refresh_guard(&self) -> Result<Option<CredentialRefreshGuard>, AuthError> {
        let lock = self.lock.clone();
        let held = tokio::task::spawn_blocking(move || -> Result<fs::File, String> {
            private_dir(lock.parent().expect("a lock is in a dir"))?;
            let f = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .mode(0o600)
                .open(&lock)
                .map_err(|e| format!("{}: {e}", lock.display()))?;
            // SAFETY: flock on a descriptor this closure owns; dropping the
            // file closes it and so releases the lock.
            if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(f)
        })
        .await
        .map_err(store_failed)?
        .map_err(store_failed)?;
        Ok(Some(CredentialRefreshGuard::new(held)))
    }
}

/// The client OAuth goes out by: the roots the rest of Libri uses, as
/// rmcp's default panics on a host with no system certificates.
fn oauth_client() -> Result<reqwest::Client, String> {
    crate::look::http_client_following().map_err(|e| e.to_string())
}

/// rmcp's OAuth HTTP, on Libri's own clients. rmcp's default client trusts
/// only the system's certificate store, which a bare machine (or the build
/// sandbox) lacks; Libri carries its roots. A second client keeps rmcp's
/// no-redirect rule for token requests, which `with_client` would drop.
struct OAuthHttp {
    follow: reqwest::Client,
    stop: reqwest::Client,
}

/// The most of a response OAuth reads, as rmcp's own client allows.
const OAUTH_BODY_MAX: usize = 1024 * 1024;

impl OAuthHttpClient for OAuthHttp {
    fn execute(&self, request: OAuthHttpRequest) -> OAuthHttpClientFuture<'_> {
        Box::pin(async move {
            let OAuthHttpRequest {
                request,
                redirect_policy,
                timeout,
                ..
            } = request;
            let client = match redirect_policy {
                OAuthHttpRedirectPolicy::Stop => &self.stop,
                _ => &self.follow,
            };
            let mut request = reqwest::Request::try_from(request)?;
            *request.timeout_mut() = Some(timeout.unwrap_or(Duration::from_secs(30)));
            let mut response = client.execute(request).await?;
            let mut back = http::Response::builder()
                .status(response.status())
                .version(response.version());
            for (name, value) in response.headers() {
                back = back.header(name, value);
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if body.len() + chunk.len() > OAUTH_BODY_MAX {
                    return Err("The sign-in server answered too much.".into());
                }
                body.extend_from_slice(&chunk);
            }
            Ok(back.body(body)?)
        })
    }
}

async fn manager(state: &Path, c: &Connection) -> Result<AuthorizationManager, String> {
    let url = c.url.as_deref().ok_or("A command needs no sign-in.")?;
    let http = OAuthHttp {
        follow: oauth_client()?,
        stop: crate::look::http_client().map_err(|e| e.to_string())?,
    };
    let mut m = AuthorizationManager::new_with_oauth_http_client(url, Arc::new(http))
        .await
        .map_err(|e| e.to_string())?;
    m.set_credential_store(Store::of(state, &c.name));
    Ok(m)
}

/// A manager ready to give tokens, or None where there is no sign-in.
async fn signed(state: &Path, c: &Connection) -> Result<Option<AuthorizationManager>, String> {
    if credentials(state, &c.name).is_none() {
        return Ok(None);
    }
    let mut m = manager(state, c).await?;
    if !m.initialize_from_store().await.map_err(|e| e.to_string())? {
        return Err(again(&c.name));
    }
    Ok(Some(m))
}

fn again(name: &str) -> String {
    format!("{name} needs signing in again: Settings > Connections.")
}

async fn access(m: &AuthorizationManager, name: &str) -> Result<String, String> {
    m.get_access_token().await.map_err(|e| match e {
        AuthError::AuthorizationRequired | AuthError::TokenRefreshRejected(_) => again(name),
        e => format!("{name}: {e}"),
    })
}

/// Renews each sign-in near or past its end.
pub async fn renew(state: &Path) {
    for c in all(state).into_iter().filter(|c| c.url.is_some()) {
        let renewed = match signed(state, &c).await {
            Ok(Some(m)) => access(&m, &c.name).await.map(drop),
            other => other.map(drop),
        };
        if let Err(e) = renewed {
            eprintln!("libri: {e}");
        }
    }
}

/// Sign-ins started and not yet finished, by their `state`: the
/// connection's name and rmcp's session, which holds the PKCE verifier.
/// Held in memory: a restart in between means starting again.
#[derive(Default)]
pub struct Pending(tokio::sync::Mutex<HashMap<String, (String, AuthorizationSession, Instant)>>);

/// How a sign-in returns the browser to Libri.
pub struct Back {
    /// The page that forwards the browser to the address in `state`
    /// (`connections.return_page`, `BACK` unless set; set empty, none, and every
    /// sign-in returns on loopback).
    bounce: Option<String>,
    /// This site's port, for the loopback return.
    port: u16,
}

impl Back {
    pub fn new(configured: Option<&str>, port: u16) -> Self {
        let bounce = match configured.map(str::trim) {
            None => Some(BACK.to_owned()),
            Some("") => None,
            Some(b) => Some(b.to_owned()),
        };
        Back { bounce, port }
    }

    fn loopback(&self) -> String {
        format!("http://127.0.0.1:{}/api/connections/back", self.port)
    }

    /// The return address for a sign-in the browser at `origin` starts, or
    /// the Android app if `app`, and the origin to carry in `state`, if the
    /// return goes by the page.
    pub fn choose(&self, origin: Option<&str>, app: bool) -> (String, Option<String>) {
        if app {
            return (APP_BACK.to_owned(), None);
        }
        match (&self.bounce, origin.and_then(checked)) {
            (Some(b), Some(o)) if !on_host(&o) => (b.clone(), Some(o)),
            _ => (self.loopback(), None),
        }
    }
}

/// The origin the page may send a browser on to: http or https, a host and
/// perhaps a port, nothing else. As the page checks it again.
pub fn checked(origin: &str) -> Option<String> {
    let u = reqwest::Url::parse(origin.trim()).ok()?;
    let plain = matches!(u.scheme(), "http" | "https")
        && u.host_str().is_some_and(|h| !h.is_empty())
        && u.username().is_empty()
        && u.password().is_none()
        && u.path() == "/"
        && u.query().is_none()
        && u.fragment().is_none();
    plain.then(|| u.origin().ascii_serialization())
}

/// Whether an origin is this machine's own, which a server may return to
/// directly.
fn on_host(origin: &str) -> bool {
    reqwest::Url::parse(origin)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .is_some_and(|h| matches!(h.as_str(), "127.0.0.1" | "localhost" | "[::1]"))
}

/// The OAuth `state` carrying `origin` beside the nonce: `NONCE.ORIGIN`,
/// the origin in unpadded base64url. rmcp's nonce is base64url, so has no
/// dot of its own.
pub fn carry(nonce: &str, origin: &str) -> String {
    format!("{nonce}.{}", URL_SAFE_NO_PAD.encode(origin))
}

/// The nonce a returned `state` was made with, and the origin it carries.
pub fn carried(state: &str) -> (&str, Option<String>) {
    match state.split_once('.') {
        Some((nonce, origin)) => {
            let origin = URL_SAFE_NO_PAD
                .decode(origin)
                .ok()
                .and_then(|b| String::from_utf8(b).ok());
            (nonce, origin)
        }
        None => (state, None),
    }
}

/// Starts signing in to a connection: the page to open.
pub async fn sign_in(
    state: &Path,
    back: &Back,
    origin: Option<&str>,
    pending: &Pending,
    name: &str,
    app: bool,
) -> Result<String, String> {
    let c = get(state, name)?;
    let probe = probe(&c).await?;
    let (redirect, mut carried) = back.choose(origin, app);
    let session = match session(state, &c, &probe, &redirect, carried.is_some()).await {
        Err(e) if carried.is_some() && e.contains("invalid_redirect_uri") => {
            carried = None;
            session(state, &c, &probe, &back.loopback(), false).await?
        }
        other => other?,
    };
    let mut url = reqwest::Url::parse(session.get_authorization_url())
        .map_err(|e| format!("{}: {e}", c.name))?;
    let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
    let key = pairs
        .iter()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.clone())
        .ok_or("The sign-in page has no state.")?;
    if let Some(origin) = &carried {
        let state = carry(&key, origin);
        url.query_pairs_mut().clear().extend_pairs(
            pairs
                .iter()
                .map(|(k, v)| (k.as_str(), if k == "state" { &state } else { v })),
        );
    }
    let mut held = pending.0.lock().await;
    held.retain(|_, (_, _, at)| at.elapsed() < PATIENCE);
    held.insert(key, (c.name, session, Instant::now()));
    Ok(url.into())
}

/// A sign-in session registered to return to `redirect`, as a `web` client
/// or a native one.
async fn session(
    state: &Path,
    c: &Connection,
    probe: &Probe,
    redirect: &str,
    web: bool,
) -> Result<AuthorizationSession, String> {
    let mut m = manager(state, c).await?;
    let found = m
        .resolve_metadata_from_challenge(probe.challenge.as_deref())
        .await
        .map_err(|e| format!("{} offers no sign-in: {e}", c.name))?;
    m.set_metadata(found.metadata);
    let request = AuthorizationRequest::new(redirect)
        .with_client_name("Libri")
        .with_application_type(if web { "web" } else { "native" });
    AuthorizationSession::new(m, request)
        .await
        .map_err(|(_, e)| format!("{}: {e}", c.name))
}

/// Finishes a sign-in from the address the server sent the browser to, or
/// just its query: trades the code for tokens and keeps them. Answers the
/// connection's name.
pub async fn finish(pending: &Pending, address: &str) -> Result<String, String> {
    let query = address.split_once('?').map_or(address, |(_, q)| q);
    let query = query.split('#').next().unwrap_or_default();
    let back =
        reqwest::Url::parse(&format!("http://127.0.0.1/?{query}")).map_err(|e| e.to_string())?;
    let field = |name: &str| {
        back.query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.into_owned())
    };
    if let Some(e) = field("error") {
        let why = field("error_description").unwrap_or(e);
        return Err(format!("The server said: {why}"));
    }
    let (Some(code), Some(state)) = (field("code"), field("state")) else {
        return Err("That address has no sign-in code in it.".into());
    };
    // Returned by way of the page, the state carries an origin too; the
    // sign-in is kept by its nonce alone.
    let (key, _) = carried(&state);
    let (name, session) = match pending.0.lock().await.remove(key) {
        Some((name, session, at)) if at.elapsed() < PATIENCE => (name, session),
        _ => return Err("That sign-in is unknown or too old; sign in again.".into()),
    };
    session
        .handle_callback_with_issuer(&code, key, field("iss").as_deref())
        .await
        .map_err(|e| format!("{name}: {e}"))?;
    Ok(name)
}

/// What a remote server says to a first message with only the headers
/// pasted: whether it wants a sign-in, and its challenge.
struct Probe {
    refused: bool,
    challenge: Option<String>,
}

async fn probe(c: &Connection) -> Result<Probe, String> {
    let url = c.url.as_deref().ok_or("A command needs no sign-in.")?;
    let client = oauth_client()?;
    let mut req = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(hello().to_string());
    for (k, v) in &c.headers {
        req = req.header(k, v);
    }
    let r = req
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("{}: {e}", c.name))?;
    Ok(Probe {
        refused: r.status() == reqwest::StatusCode::UNAUTHORIZED,
        challenge: r
            .headers()
            .get("www-authenticate")
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned),
    })
}

fn hello() -> Value {
    json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "libri", "version": env!("CARGO_PKG_VERSION")}}})
}

/// What testing a connection found: its tools, each offered or withheld,
/// or that the server wants a sign-in first.
#[derive(Debug, Default, Serialize)]
pub struct Tested {
    pub sign_in: bool,
    pub tools: Vec<Value>,
}

/// Connects as a harness would, through the relay, and lists the tools.
pub async fn test(state: &Path, name: &str) -> Result<Tested, String> {
    let c = get(state, name)?;
    if c.url.is_some() && credentials(state, &c.name).is_none() && probe(&c).await?.refused {
        return Ok(Tested {
            sign_in: true,
            ..Tested::default()
        });
    }
    let env = crate::servers::passed_env();
    let (_, entry) = entry(state, &c.name, &env)?;
    let client = crate::look::connect_as(&entry, ()).await?;
    let tools = tokio::time::timeout(Duration::from_secs(30), client.peer().list_all_tools()).await;
    let _ = client.cancel().await;
    let tools = tools
        .map_err(|_| format!("{} took too long to list its tools.", c.name))?
        .map_err(|e| format!("{}: {e}", c.name))?;
    // The relay leaves the withheld ones out, so they are named from the
    // connection instead.
    let mut out: Vec<Value> = tools
        .iter()
        .map(|t| json!({"name": t.name, "description": t.description, "withheld": false}))
        .collect();
    out.extend(
        c.withheld
            .iter()
            .map(|t| json!({"name": t, "withheld": true})),
    );
    Ok(Tested {
        sign_in: false,
        tools: out,
    })
}

// The relay: a stdio MCP server per connection, for every harness.

type Out = Arc<Mutex<Box<dyn Write + Send>>>;

fn write(out: &Out, v: &Value) {
    let mut out = out.lock().unwrap();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

/// The answer to a tool call that does not run, said as the tool's own
/// error so the model reads why.
fn refused(message: &Value, why: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": message["id"],
        "result": {"isError": true, "content": [{"type": "text", "text": why}]},
    })
}

/// What the relay holds of one connection while a harness talks to it.
/// The connection is read again at each call, so a rule set in Settings,
/// or by an "Always allow", holds from the next call on.
struct Gate {
    state: PathBuf,
    name: String,
    /// The tools the server listed, by name, with their annotations.
    seen: Mutex<HashMap<String, Value>>,
    /// The tool each call on its way was for, by the call's id.
    calls: Mutex<HashMap<String, String>>,
}

impl Gate {
    fn new(state: PathBuf, name: &str) -> Arc<Self> {
        let seen = listed(&state, name)
            .into_iter()
            .filter_map(|t| Some((t["name"].as_str()?.to_owned(), t)))
            .collect();
        Arc::new(Self {
            state,
            name: name.to_owned(),
            seen: Mutex::new(seen),
            calls: Mutex::default(),
        })
    }

    /// The answer to give in place of `message`, a call that is off here
    /// or that the person declined; None to pass it on. A call whose tool
    /// asks first waits here on the person.
    fn check(&self, message: &Value) -> Option<Value> {
        if message["method"] != "tools/call" {
            return None;
        }
        let tool = message["params"]["name"].as_str().unwrap_or_default();
        let c = match get(&self.state, &self.name) {
            Ok(c) => c,
            Err(e) => return Some(refused(message, e)),
        };
        let rule = c.rule(tool, self.seen.lock().unwrap().get(tool));
        let pass = match rule {
            Rule::Off => Err(format!(
                "{tool} is off here; the person can turn it on in Settings > Connections."
            )),
            Rule::Allow => Ok(()),
            Rule::Ask => match ask(
                &self.state,
                &self.name,
                tool,
                &message["params"]["arguments"],
            ) {
                Verdict::Once => Ok(()),
                Verdict::Always => set_rule(&self.state, &self.name, tool, Rule::Allow),
                Verdict::No(why) => Err(why),
            },
        };
        match pass {
            Ok(()) => {
                self.calls
                    .lock()
                    .unwrap()
                    .insert(message["id"].to_string(), tool.to_owned());
                None
            }
            Err(why) => Some(refused(message, why)),
        }
    }

    /// A message from the server as the harness gets it: a list of tools
    /// kept for Settings and without those that are off, and a call's
    /// result with what the person hid taken out (builtins::trimmed).
    fn answered(&self, mut v: Value) -> Value {
        if let Some(tools) = v["result"]["tools"].as_array_mut() {
            remember(&self.state, &self.name, tools);
            let mut seen = self.seen.lock().unwrap();
            for t in tools.iter() {
                if let Some(n) = t["name"].as_str() {
                    seen.insert(n.to_owned(), t.clone());
                }
            }
            let off = get(&self.state, &self.name)
                .map(|c| c.withheld)
                .unwrap_or_default();
            tools.retain(|t| {
                !t["name"]
                    .as_str()
                    .is_some_and(|n| off.iter().any(|w| w == n))
            });
            return v;
        }
        let tool = v
            .get("id")
            .and_then(|id| self.calls.lock().unwrap().remove(&id.to_string()));
        if let (Some(tool), Some(result)) = (tool, v.get_mut("result")) {
            *result = crate::builtins::trimmed(&self.state, &self.name, &tool, result.take());
        }
        v
    }
}

// Asking the person: the relay, a harness's child, asks the daemon over
// a socket in the state directory, and the daemon puts the ask to the turn
// that runs the harness, as a card in its chat.

/// The variable that tells a relay whom to ask: a turn's key (`asking`),
/// `person` for the host's own calls a person made by hand (a card's
/// button), `none` for a turn nobody can be asked in. Unset, as for
/// OpenCode's one server for all turns, the main chat is asked.
pub const ASKING: &str = "LIBRI_ASK";

/// The person's verdict on one call.
enum Verdict {
    Once,
    Always,
    No(String),
}

fn ask_socket(state: &Path) -> PathBuf {
    dir(state).join("ask.sock")
}

fn ask(state: &Path, connection: &str, tool: &str, input: &Value) -> Verdict {
    let turn = std::env::var(ASKING).ok();
    match turn.as_deref() {
        Some("person") => return Verdict::Once,
        Some("none") => {
            return Verdict::No(format!(
                "{tool} asks the person before each call, and they can't be asked from here. Tell them what you would do; they can set it to Allow in Settings > Connections."
            ));
        }
        _ => {}
    }
    let asked = (|| -> Result<Verdict, String> {
        let mut sock = std::os::unix::net::UnixStream::connect(ask_socket(state))
            .map_err(|e| format!("Libri is not running here to ask the person: {e}"))?;
        let line = json!({"turn": turn, "connection": connection, "tool": tool, "input": input});
        writeln!(sock, "{line}").map_err(|e| e.to_string())?;
        let mut back = String::new();
        BufReader::new(sock)
            .read_line(&mut back)
            .map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_str(&back).map_err(|e| e.to_string())?;
        Ok(match v["verdict"].as_str() {
            Some("once") => Verdict::Once,
            Some("always") => Verdict::Always,
            _ => Verdict::No(
                v["why"]
                    .as_str()
                    .unwrap_or("The person declined it.")
                    .to_owned(),
            ),
        })
    })();
    asked.unwrap_or_else(Verdict::No)
}

fn random_key() -> String {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("the system's random numbers");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The asker of each turn running now, by the key its relays carry. Weak:
/// the turn's end drops it.
static TURNS: std::sync::LazyLock<
    Mutex<HashMap<String, std::sync::Weak<dyn crate::model::Asker>>>,
> = std::sync::LazyLock::new(Mutex::default);

/// What a turn's relays carry in `ASKING`: its key, or `none` when it has
/// nobody to ask.
pub fn asking(asker: Option<&Arc<dyn crate::model::Asker>>) -> String {
    let Some(a) = asker else {
        return "none".into();
    };
    let mut turns = TURNS.lock().unwrap();
    turns.retain(|_, w| w.strong_count() > 0);
    let key = random_key();
    turns.insert(key.clone(), Arc::downgrade(a));
    key
}

/// Answers the relays' asks, each by putting it to its turn, or to `main`
/// when the relay names none. On a thread per ask: an ask waits on the
/// person.
pub fn serve_asks(state: &Path, main: Arc<dyn crate::model::Asker>) -> Result<(), String> {
    let at = ask_socket(state);
    private_dir(&dir(state))?;
    let _ = fs::remove_file(&at);
    let listener = std::os::unix::net::UnixListener::bind(&at)
        .map_err(|e| format!("{}: {e}", at.display()))?;
    let state = state.to_owned();
    std::thread::spawn(move || {
        for sock in listener.incoming().flatten() {
            let (state, main) = (state.clone(), main.clone());
            std::thread::spawn(move || answer_ask(&state, sock, main));
        }
    });
    Ok(())
}

fn answer_ask(
    state: &Path,
    sock: std::os::unix::net::UnixStream,
    main: Arc<dyn crate::model::Asker>,
) {
    let mut line = String::new();
    let Ok(mut back) = sock.try_clone() else {
        return;
    };
    if BufReader::new(sock).read_line(&mut line).is_err() {
        return;
    }
    let Ok(m) = serde_json::from_str::<Value>(&line) else {
        return;
    };
    let tool = m["tool"].as_str().unwrap_or_default();
    let connection = m["connection"].as_str().unwrap_or_default();
    let asker = match m["turn"].as_str() {
        None => Some(main),
        Some(key) => TURNS
            .lock()
            .unwrap()
            .get(key)
            .and_then(std::sync::Weak::upgrade),
    };
    let verdict = match asker {
        None => {
            json!({"verdict": "no", "why": format!("The turn that asked for {tool} has ended.")})
        }
        Some(a) => {
            let _ = state;
            let ask = crate::model::Ask {
                id: format!("tool-{}", random_key()),
                tool: tool.to_owned(),
                input: m["input"].clone(),
                about: Some(format!("On {connection}, which asks before each {tool}.")),
                always: true,
            };
            match a.put(ask) {
                None => json!({"verdict": "no", "why": format!("The person declined {tool}.")}),
                Some(v) if v["always"] == Value::Bool(true) => json!({"verdict": "always"}),
                Some(_) => json!({"verdict": "once"}),
            }
        }
    };
    let _ = writeln!(back, "{verdict}");
}

/// Speaks MCP on stdin and stdout for a harness and passes each message to
/// the connection's server.
pub fn relay(state: PathBuf, name: &str) -> Result<(), String> {
    let c = get(&state, name)?;
    let out: Out = Arc::new(Mutex::new(Box::new(std::io::stdout())));
    let input = std::io::stdin().lock();
    let gate = Gate::new(state.clone(), name);
    if c.url.is_some() {
        Remote::new(state, c, gate)?.run(input, out)
    } else {
        local(&c, gate, input, out)
    }
}

/// A server run here: its own process, its stdin and stdout passed through
/// but for what is withheld.
fn local(c: &Connection, gate: Arc<Gate>, input: impl BufRead, out: Out) -> Result<(), String> {
    let (program, args) = c
        .command
        .split_first()
        .ok_or("The connection has no command.")?;
    let mut child = std::process::Command::new(program)
        .args(args)
        .envs(&c.env)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let to = Arc::new(Mutex::new(child.stdin.take().expect("piped")));
    let from = child.stdout.take().expect("piped");
    let answers = out.clone();
    let g = gate.clone();
    let back = std::thread::spawn(move || {
        for line in BufReader::new(from).lines().map_while(Result::ok) {
            match serde_json::from_str::<Value>(&line) {
                Ok(v) => write(&answers, &g.answered(v)),
                Err(_) => {
                    let mut o = answers.lock().unwrap();
                    let _ = writeln!(o, "{line}");
                    let _ = o.flush();
                }
            }
        }
    });
    let pass = |to: &Mutex<std::process::ChildStdin>, line: &str| {
        let mut to = to.lock().unwrap();
        writeln!(to, "{line}").and_then(|_| to.flush()).is_ok()
    };
    let mut calls = Vec::new();
    for line in input.lines() {
        let line = line.map_err(|e| e.to_string())?;
        if let Ok(m) = serde_json::from_str::<Value>(&line)
            && m["method"] == "tools/call"
        {
            // A call may wait on the person: on its own thread, so the
            // rest (a cancel among them) goes on meanwhile.
            let (g, to, out) = (gate.clone(), to.clone(), out.clone());
            calls.retain(|c: &std::thread::JoinHandle<()>| !c.is_finished());
            calls.push(std::thread::spawn(move || match g.check(&m) {
                Some(answer) => write(&out, &answer),
                None => {
                    pass(&to, &line);
                }
            }));
            continue;
        }
        if !pass(&to, &line) {
            break;
        }
    }
    for c in calls {
        let _ = c.join();
    }
    drop(to);
    let _ = back.join();
    let _ = child.wait();
    Ok(())
}

/// A remote server, by streamable HTTP, a message per request.
struct Remote {
    state: PathBuf,
    c: Connection,
    gate: Arc<Gate>,
    agent: ureq::Agent,
    runtime: tokio::runtime::Runtime,
    /// rmcp's manager, made on the first message, as finding the server's
    /// metadata takes requests of its own.
    oauth: tokio::sync::Mutex<Option<Option<AuthorizationManager>>>,
    session: Mutex<Option<String>>,
    version: Mutex<Option<String>>,
}

impl Remote {
    fn new(state: PathBuf, c: Connection, gate: Arc<Gate>) -> Result<Self, String> {
        Ok(Self {
            state,
            c,
            gate,
            agent: ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(300)))
                .http_status_as_error(false)
                .build()
                .new_agent(),
            runtime: tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?,
            oauth: Default::default(),
            session: Mutex::default(),
            version: Mutex::default(),
        })
    }

    fn run(self, input: impl BufRead, out: Out) -> Result<(), String> {
        let me = Arc::new(self);
        for line in input.lines() {
            let line = line.map_err(|e| e.to_string())?;
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            // The session is named in the answer to initialize, which every
            // later message carries, so that one is passed on alone.
            if message["method"] == "initialize" {
                me.pass(&line, &message, &out);
            } else {
                let (r, o) = (me.clone(), out.clone());
                std::thread::spawn(move || r.pass(&line, &message, &o));
            }
        }
        Ok(())
    }

    fn pass(&self, line: &str, message: &Value, out: &Out) {
        if let Err(e) = self.send(line, message, out)
            && let Some(id) = message.get("id")
        {
            write(
                out,
                &json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32603, "message": e}}),
            );
        }
    }

    /// The bearer token, where the connection is signed in.
    fn token(&self) -> Result<Option<String>, String> {
        self.runtime.block_on(async {
            let mut held = self.oauth.lock().await;
            if held.is_none() {
                *held = Some(signed(&self.state, &self.c).await?);
            }
            match held.as_ref().and_then(Option::as_ref) {
                Some(m) => access(m, &self.c.name).await.map(Some),
                None => Ok(None),
            }
        })
    }

    fn send(&self, line: &str, message: &Value, out: &Out) -> Result<(), String> {
        let name = &self.c.name;
        if let Some(answer) = self.gate.check(message) {
            write(out, &answer);
            return Ok(());
        }
        let url = self.c.url.as_deref().expect("a remote connection");
        let mut req = self
            .agent
            .post(url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        for (k, v) in &self.c.headers {
            req = req.header(k, v);
        }
        if let Some(token) = self.token()? {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        if let Some(s) = self.session.lock().unwrap().clone() {
            req = req.header("Mcp-Session-Id", s);
        }
        if let Some(v) = self.version.lock().unwrap().clone() {
            req = req.header("MCP-Protocol-Version", v);
        }
        let mut r = req.send(line).map_err(|e| format!("{name}: {e}"))?;
        if let Some(s) = r
            .headers()
            .get("mcp-session-id")
            .and_then(|h| h.to_str().ok())
        {
            *self.session.lock().unwrap() = Some(s.to_owned());
        }
        let status = r.status();
        let stream = r
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|t| t.starts_with("text/event-stream"));
        let mut body = BufReader::new(r.body_mut().as_reader());
        if status == 401 {
            return Err(format!("{name} wants a sign-in: Settings > Connections."));
        }
        if !status.is_success() {
            let mut why = String::new();
            let _ = body.take(2000).read_to_string(&mut why);
            return Err(format!("{name} said {status}: {}", why.trim()));
        }
        if status == 202 {
            return Ok(());
        }
        if !stream {
            let mut text = String::new();
            body.read_to_string(&mut text).map_err(|e| e.to_string())?;
            return self.answer(&text, message, out).map(|_| ());
        }
        // Server-sent events: each event's data lines make one message,
        // and the stream has done its work once the answer has come.
        let mut data = String::new();
        for l in body.lines() {
            let l = l.map_err(|e| e.to_string())?;
            if let Some(d) = l.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(d.strip_prefix(' ').unwrap_or(d));
            } else if l.is_empty()
                && !data.is_empty()
                && self.answer(&std::mem::take(&mut data), message, out)?
            {
                break;
            }
        }
        Ok(())
    }

    /// Writes one message from the server; true when it answers `asked`.
    fn answer(&self, text: &str, asked: &Value, out: &Out) -> Result<bool, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("{}: {e}", self.c.name))?;
        let done = asked.get("id").is_some()
            && v.get("id") == asked.get("id")
            && (v.get("result").is_some() || v.get("error").is_some());
        if done
            && asked["method"] == "initialize"
            && let Some(p) = v["result"]["protocolVersion"].as_str()
        {
            *self.version.lock().unwrap() = Some(p.to_owned());
        }
        write(out, &self.gate.answered(v));
        Ok(done)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The Fastmail sign-in of 0.9.26 and before, kept as its own files, made
/// an ordinary connection with its tokens as they are, so nobody signs in
/// again. Remove
/// this once every install has started a release with it.
pub fn migrate(state: &Path) -> Result<(), String> {
    #[derive(Deserialize)]
    struct Grant {
        account: String,
        client_id: String,
        refresh_token: String,
        access_token: String,
        expires: u64,
    }
    let old = dir(state).join("fastmail.json");
    let Ok(text) = fs::read_to_string(&old) else {
        return Ok(());
    };
    if get(state, "fastmail").is_err() {
        let g: Grant = serde_json::from_str(&text).map_err(|e| format!("fastmail.json: {e}"))?;
        let token: OAuthTokenResponse = serde_json::from_value(json!({
            "access_token": g.access_token,
            "token_type": "bearer",
            "expires_in": g.expires.saturating_sub(now()),
            "refresh_token": g.refresh_token,
            "username": g.account,
        }))
        .map_err(|e| e.to_string())?;
        let creds = StoredCredentials::new(
            g.client_id,
            Some(token),
            vec![
                "https://www.fastmail.com/dev/mcp".into(),
                "offline_access".into(),
            ],
            Some(now()),
        );
        private_dir(&servers(state))?;
        write_private(
            &file(state, "fastmail", "oauth.json"),
            &serde_json::to_string_pretty(&creds).map_err(|e| e.to_string())?,
        )?;
        keep(
            state,
            &Connection {
                name: "fastmail".into(),
                url: Some("https://api.fastmail.com/mcp".into()),
                ..Connection::default()
            },
        )?;
    }
    for f in ["fastmail.json", "fastmail-client.json", "fastmail.lock"] {
        forget(&dir(state).join(f))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use std::io::Cursor;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("libri-conn-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn change(name: &str, address: &str, secrets: Option<&str>) -> Change {
        Change {
            name: name.into(),
            address: address.into(),
            secrets: secrets.map(str::to_owned),
            ..Change::default()
        }
    }

    /// What a relay wrote, a message a line.
    #[derive(Clone, Default)]
    struct Said(Arc<Mutex<Vec<u8>>>);

    impl Write for Said {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Said {
        fn out(&self) -> Out {
            Arc::new(Mutex::new(Box::new(self.clone())))
        }
        fn lines(&self) -> Vec<Value> {
            String::from_utf8_lossy(&self.0.lock().unwrap())
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        }
        fn answer(&self, id: u64) -> Value {
            self.lines()
                .into_iter()
                .find(|v| v["id"] == id)
                .unwrap_or_else(|| panic!("no answer {id} in {:?}", self.lines()))
        }
    }

    #[test]
    fn an_address_is_a_url_or_a_command() {
        let s = scratch("address");
        let c = put(&s, change("Mail", "https://libri.example.com/mcp", None)).unwrap();
        assert_eq!(c.name, "mail");
        assert_eq!(c.url.as_deref(), Some("https://libri.example.com/mcp"));
        assert!(c.on);
        let c = put(
            &s,
            change("box", "npx -y '@scope/server' --dir \"/a b\"", None),
        )
        .unwrap();
        assert_eq!(c.command, ["npx", "-y", "@scope/server", "--dir", "/a b"]);
        assert_eq!(c.address(), "npx -y @scope/server --dir '/a b'");
        assert_eq!(all(&s).len(), 2);
        assert!(put(&s, change("libri", "x", None)).is_err());
        assert!(put(&s, change("a b", "x", None)).is_err());
        assert!(put(&s, change("ok", "  ", None)).is_err());
        remove(&s, "box").unwrap();
        assert_eq!(all(&s).len(), 1);
        let _ = fs::remove_dir_all(&s);
    }

    #[test]
    fn secrets_are_kept_but_never_shown() {
        let s = scratch("secrets");
        let c = put(
            &s,
            change(
                "ha",
                "https://libri.example.com/mcp",
                Some("Authorization: Bearer abc:def\n"),
            ),
        )
        .unwrap();
        assert_eq!(c.headers["Authorization"], "Bearer abc:def");
        let shown = shown(&s, &c).to_string();
        assert!(!shown.contains("abc"), "{shown}");
        assert!(shown.contains("Authorization"));
        // Changed without its secrets, it keeps them; switched off, it stays.
        let mut edit = change("ha", "https://libri.example.com/other", None);
        edit.on = Some(false);
        let c = put(&s, edit).unwrap();
        assert_eq!(c.headers["Authorization"], "Bearer abc:def");
        assert!(!c.on);
        assert!(
            !put(&s, change("ha", "https://libri.example.com/other", None))
                .unwrap()
                .on
        );
        // A program has no use for a header.
        let c = put(&s, change("ha", "kit-server", None)).unwrap();
        assert!(c.headers.is_empty());
        let c = put(&s, change("ha", "kit-server", Some("TOKEN=a=b"))).unwrap();
        assert_eq!(c.env["TOKEN"], "a=b");
        assert!(put(&s, change("ha", "kit-server", Some("no separator"))).is_err());
        let mode = fs::metadata(file(&s, "ha", "json")).unwrap().permissions();
        assert_eq!(std::os::unix::fs::PermissionsExt::mode(&mode) & 0o077, 0);
        let _ = fs::remove_dir_all(&s);
    }

    #[test]
    fn a_switched_off_connection_is_given_to_no_harness() {
        let s = scratch("off");
        put(&s, change("on", "kit-server", None)).unwrap();
        let mut off = change("off", "kit-server", None);
        off.on = Some(false);
        put(&s, off).unwrap();
        let names: Vec<String> = entries(&s, &Default::default())
            .unwrap()
            .into_iter()
            .map(|(n, e)| {
                assert_eq!(e["args"][0], "connection");
                n
            })
            .collect();
        assert_eq!(names, ["on"]);
        let _ = fs::remove_dir_all(&s);
    }

    #[test]
    fn a_tool_off_is_neither_offered_nor_passed_on() {
        let s = scratch("gate-off");
        put(
            &s,
            Change {
                name: "m".into(),
                address: "https://mail.example.com/mcp".into(),
                ..Change::default()
            },
        )
        .unwrap();
        set_rule(&s, "m", "send_email", Rule::Off).unwrap();
        let g = Gate::new(s.clone(), "m");
        let list = json!({"id": 1, "result": {"tools": [
            {"name": "send_email"}, {"name": "search_email", "annotations": {"readOnlyHint": true}}]}});
        let tools = g.answered(list);
        assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 1);
        // Settings still lists it, from what the server gave.
        assert_eq!(listed(&s, "m").len(), 2);
        let call = json!({"id": 2, "method": "tools/call", "params": {"name": "send_email"}});
        assert_eq!(g.check(&call).unwrap()["result"]["isError"], true);
        let search = json!({"id": 3, "method": "tools/call", "params": {"name": "search_email"}});
        assert!(g.check(&search).is_none());
        let _ = fs::remove_dir_all(&s);
    }

    #[test]
    fn annotations_set_the_default_and_the_person_overrides_it() {
        assert_eq!(
            default_rule(&json!({"annotations": {"readOnlyHint": true}})),
            Rule::Allow
        );
        assert_eq!(
            default_rule(
                &json!({"annotations": {"readOnlyHint": false, "destructiveHint": false}})
            ),
            Rule::Ask
        );
        assert_eq!(
            default_rule(&json!({"annotations": {"readOnlyHint": false, "destructiveHint": true}})),
            Rule::Ask
        );
        assert_eq!(default_rule(&json!({"name": "send"})), Rule::Ask);
        let s = scratch("rules");
        put(
            &s,
            Change {
                name: "m".into(),
                address: "https://mail.example.com/mcp".into(),
                ..Change::default()
            },
        )
        .unwrap();
        set_rule(&s, "m", "search", Rule::Ask).unwrap();
        set_rule(&s, "m", "send", Rule::Allow).unwrap();
        // A change of address keeps the rules.
        put(
            &s,
            Change {
                name: "m".into(),
                address: "https://mail.example.com/mcp/".into(),
                ..Change::default()
            },
        )
        .unwrap();
        let c = get(&s, "m").unwrap();
        assert_eq!(
            c.rule(
                "search",
                Some(&json!({"annotations": {"readOnlyHint": true}}))
            ),
            Rule::Ask
        );
        assert_eq!(c.rule("send", None), Rule::Allow);
        assert_eq!(c.rule("other", None), Rule::Ask);
        set_rule(&s, "m", "send", Rule::Off).unwrap();
        let c = get(&s, "m").unwrap();
        assert_eq!(
            (c.rule("send", None), c.withheld.clone()),
            (Rule::Off, vec!["send".to_owned()])
        );
        let _ = fs::remove_dir_all(&s);
    }

    #[test]
    fn a_call_that_asks_waits_on_the_turn_and_always_allow_sticks() {
        struct Says(Option<Value>, Mutex<Vec<crate::model::Ask>>);
        impl crate::model::Asker for Says {
            fn put(&self, ask: crate::model::Ask) -> Option<Value> {
                self.1.lock().unwrap().push(ask);
                self.0.clone()
            }
            fn withdraw(&self, _: &str) {}
        }
        let s = scratch("ask");
        put(
            &s,
            Change {
                name: "m".into(),
                address: "https://mail.example.com/mcp".into(),
                ..Change::default()
            },
        )
        .unwrap();
        let no: Arc<dyn crate::model::Asker> = Arc::new(Says(None, Mutex::default()));
        serve_asks(&s, no).unwrap();
        let g = Gate::new(s.clone(), "m");
        let call = json!({"id": 1, "method": "tools/call", "params": {"name": "send", "arguments": {"to": "a"}}});
        // Unset, the main chat is asked: here it declines.
        let got = g.check(&call).unwrap();
        assert!(
            got["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("declined")
        );
        // A turn's own key reaches that turn's asker.
        let yes = Arc::new(Says(Some(json!({"always": true})), Mutex::default()));
        let turn: Arc<dyn crate::model::Asker> = yes.clone();
        let key = asking(Some(&turn));
        // SAFETY: the tests that read ASKING are this one alone.
        unsafe { std::env::set_var(ASKING, &key) };
        assert!(g.check(&call).is_none());
        unsafe { std::env::remove_var(ASKING) };
        let asked = yes.1.lock().unwrap();
        assert_eq!((asked[0].tool.as_str(), asked[0].always), ("send", true));
        assert_eq!(get(&s, "m").unwrap().rule("send", None), Rule::Allow);
        let _ = fs::remove_dir_all(&s);
    }

    #[test]
    fn the_old_fastmail_sign_in_becomes_an_ordinary_connection() {
        let s = scratch("migrate");
        private_dir(&dir(&s)).unwrap();
        fs::write(
            dir(&s).join("fastmail.json"),
            json!({"account": "ada@libri.example.com", "client_id": "cid",
                "refresh_token": "r1", "access_token": "a1", "expires": now() + 3000})
            .to_string(),
        )
        .unwrap();
        fs::write(dir(&s).join("fastmail-client.json"), "{}").unwrap();
        migrate(&s).unwrap();
        let c = get(&s, "fastmail").unwrap();
        assert!(c.withheld.is_empty());
        assert!(c.url.is_some());
        let creds = credentials(&s, "fastmail").unwrap();
        assert_eq!(creds.client_id, "cid");
        let t = serde_json::to_value(creds.token_response.unwrap()).unwrap();
        assert_eq!(t["access_token"], "a1");
        assert_eq!(t["refresh_token"], "r1");
        assert!(t["expires_in"].as_u64().unwrap() > 2900);
        assert_eq!(shown(&s, &c)["account"], "ada@libri.example.com");
        assert!(!dir(&s).join("fastmail.json").exists());
        assert!(!dir(&s).join("fastmail-client.json").exists());
        migrate(&s).unwrap();
        let _ = fs::remove_dir_all(&s);
    }

    #[tokio::test]
    async fn a_pasted_address_without_a_code_is_refused() {
        let p = Pending::default();
        let e = finish(&p, "http://127.0.0.1:1/api/connections/back?state=x")
            .await
            .unwrap_err();
        assert!(e.contains("no sign-in code"), "{e}");
        let e = finish(&p, "?code=c&state=unknown").await.unwrap_err();
        assert!(e.contains("unknown or too old"), "{e}");
        let e = finish(&p, "?error=access_denied&state=x")
            .await
            .unwrap_err();
        assert!(e.contains("access_denied"), "{e}");
    }

    #[test]
    fn a_local_server_runs_with_its_environment_and_withholds() {
        // A stand-in server: answers each line with its id, the tools, and
        // the variable it was given.
        let script = r#"while read -r l; do id=$(printf '%s' "$l" | sed 's/.*"id":\([0-9]*\).*/\1/'); printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"keep"},{"name":"drop"}],"env":"%s"}}\n' "$id" "$KIT_TOKEN"; done"#;
        let c = Connection {
            name: "kit".into(),
            command: vec!["sh".into(), "-c".into(), script.into()],
            env: [("KIT_TOKEN".to_owned(), "t0".to_owned())].into(),
            withheld: vec!["drop".into()],
            on: true,
            ..Connection::default()
        };
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"drop"}}"#,
            "\n"
        );
        let said = Said::default();
        let s = scratch("kit");
        keep(&s, &c).unwrap();
        local(
            &c,
            Gate::new(s.clone(), "kit"),
            Cursor::new(input),
            said.out(),
        )
        .unwrap();
        let _ = fs::remove_dir_all(&s);
        let list = said.answer(1);
        assert_eq!(list["result"]["tools"], json!([{"name": "keep"}]));
        assert_eq!(list["result"]["env"], "t0");
        assert_eq!(said.answer(2)["result"]["isError"], true);
    }

    /// A stand-in remote server and its authorization server, on one port:
    /// MCP at /mcp, wanting `Bearer` of a token it issued, or the header
    /// `x-key: k`.
    #[derive(Default)]
    struct Far {
        issued: Mutex<Vec<String>>,
        base: Mutex<String>,
        /// Each client registration asked for.
        registered: Mutex<Vec<Value>>,
        /// Refuses an https return address, as Fastmail does.
        no_https: std::sync::atomic::AtomicBool,
    }

    async fn mcp(
        State(far): State<Arc<Far>>,
        headers: HeaderMap,
        Json(m): Json<Value>,
    ) -> Response {
        let bearer = headers
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(str::to_owned);
        let keyed = headers.get("x-key").is_some_and(|h| h == "k");
        let issued = far.issued.lock().unwrap().last().cloned();
        if !keyed && (bearer.is_none() || bearer != issued) {
            let base = far.base.lock().unwrap().clone();
            return (
                StatusCode::UNAUTHORIZED,
                [("www-authenticate", format!(
                    "Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/mcp\""))],
            )
                .into_response();
        }
        let Some(id) = m.get("id") else {
            return StatusCode::ACCEPTED.into_response();
        };
        let result = match m["method"].as_str() {
            Some("initialize") => json!({"protocolVersion": "2025-06-18",
                "capabilities": {"tools": {}}, "serverInfo": {"name": "far", "version": "1"}}),
            Some("tools/list") => {
                json!({"tools": [{"name": "listing", "inputSchema": {"type": "object"}},
                {"name": "send", "inputSchema": {"type": "object"}}],
                "token": bearer})
            }
            _ => json!({"content": []}),
        };
        Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
    }

    async fn far() -> (String, Arc<Far>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Far::default());
        *state.base.lock().unwrap() = base.clone();
        let b = base.clone();
        let resource = move || {
            let b = b.clone();
            async move {
                Json(
                    json!({"resource": format!("{b}/mcp"), "authorization_servers": [b],
                    "scopes_supported": ["mcp"]}),
                )
            }
        };
        let b = base.clone();
        let issuer = move || {
            let b = b.clone();
            async move {
                Json(
                    json!({"issuer": b, "authorization_endpoint": format!("{b}/authorize"),
                    "token_endpoint": format!("{b}/token"), "registration_endpoint": format!("{b}/register"),
                    "response_types_supported": ["code"], "code_challenge_methods_supported": ["S256"],
                    "grant_types_supported": ["authorization_code", "refresh_token"],
                    "token_endpoint_auth_methods_supported": ["none"], "scopes_supported": ["mcp"]}),
                )
            }
        };
        let register = |State(far): State<Arc<Far>>, Json(v): Json<Value>| async move {
            far.registered.lock().unwrap().push(v.clone());
            let https = v["redirect_uris"][0]
                .as_str()
                .is_some_and(|u| u.starts_with("https:"));
            if https && far.no_https.load(std::sync::atomic::Ordering::Relaxed) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "invalid_redirect_uri",
                    "error_description": "redirect_uri is not a valid scheme/host"})),
                );
            }
            (
                StatusCode::CREATED,
                Json(json!({"client_id": "bob-client",
                "redirect_uris": v["redirect_uris"], "token_endpoint_auth_method": "none"})),
            )
        };
        let token = |State(far): State<Arc<Far>>, body: String| async move {
            let mut issued = far.issued.lock().unwrap();
            let next = format!("tok{}", issued.len() + 1);
            issued.push(next.clone());
            let grant = if body.contains("grant_type=refresh_token") {
                "refresh"
            } else {
                "code"
            };
            assert!(
                grant == "refresh" || body.contains("code_verifier="),
                "{body}"
            );
            // The first lasts too little to use, so the relay renews it.
            let lasts = if issued.len() == 1 { 5 } else { 3600 };
            Json(
                json!({"access_token": next, "token_type": "Bearer", "expires_in": lasts,
                "refresh_token": format!("r{}", issued.len()), "scope": "mcp"}),
            )
        };
        let router = axum::Router::new()
            .route("/mcp", axum::routing::post(mcp))
            .route(
                "/.well-known/oauth-protected-resource/mcp",
                axum::routing::get(resource),
            )
            .route(
                "/.well-known/oauth-authorization-server",
                axum::routing::get(issuer),
            )
            .route("/register", axum::routing::post(register))
            .route("/token", axum::routing::post(token))
            .with_state(state.clone());
        tokio::spawn(async move { axum::serve(listener, router).await });
        (base, state)
    }

    fn talk(state: &Path, name: &str) -> Said {
        let c = get(state, name).unwrap();
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"send"}}"#,
            "\n"
        );
        let said = Said::default();
        Remote::new(state.to_owned(), c, Gate::new(state.to_owned(), name))
            .unwrap()
            .run(Cursor::new(input), said.out())
            .unwrap();
        // Messages after the first go on threads of their own.
        for _ in 0..100 {
            if said.lines().len() >= 3 {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        said
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_remote_server_is_reached_by_a_pasted_header() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (base, _) = far().await;
        let s = scratch("header");
        let mut c = change("far", &format!("{base}/mcp"), Some("x-key: k"));
        c.withheld = vec!["send".into()];
        put(&s, c).unwrap();
        assert!(!probe(&get(&s, "far").unwrap()).await.unwrap().refused);
        let st = s.clone();
        let said = tokio::task::spawn_blocking(move || talk(&st, "far"))
            .await
            .unwrap();
        assert_eq!(
            said.answer(2)["result"]["tools"].as_array().unwrap().len(),
            1
        );
        assert_eq!(said.answer(3)["result"]["isError"], true);
        let _ = fs::remove_dir_all(&s);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_remote_server_signs_in_by_oauth_and_renews() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (base, far) = far().await;
        let s = scratch("oauth");
        put(&s, change("far", &format!("{base}/mcp"), None)).unwrap();
        let c = get(&s, "far").unwrap();
        assert!(probe(&c).await.unwrap().refused);
        // Unsigned, the relay says what to do.
        let st = s.clone();
        let said = tokio::task::spawn_blocking(move || talk(&st, "far"))
            .await
            .unwrap();
        let why = said.answer(1)["error"]["message"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(why.contains("sign-in"), "{why}");

        let pending = Pending::default();
        let back = Back::new(None, 4321);
        // Started from a browser on the host, the server returns straight
        // to its loopback port.
        let page = sign_in(
            &s,
            &back,
            Some("http://127.0.0.1:4321"),
            &pending,
            "far",
            false,
        )
        .await
        .unwrap();
        let page = reqwest::Url::parse(&page).unwrap();
        assert!(
            page.as_str().starts_with(&format!("{base}/authorize")),
            "{page}"
        );
        let q: HashMap<String, String> = page.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], "bob-client");
        assert_eq!(
            q["redirect_uri"],
            "http://127.0.0.1:4321/api/connections/back"
        );
        assert_eq!(q["code_challenge_method"], "S256");
        assert!(!q["state"].contains('.'), "no origin carried");
        let asked = far.registered.lock().unwrap()[0].clone();
        assert_eq!(asked["redirect_uris"], json!([q["redirect_uri"]]));
        assert_eq!(asked["application_type"], "native");
        // The browser comes back to the loopback address; pasted, the same.
        let back = format!("{}?code=c1&state={}", q["redirect_uri"], q["state"]);
        assert_eq!(finish(&pending, &back).await.unwrap(), "far");
        assert!(
            finish(&pending, &back).await.is_err(),
            "a code is used once"
        );
        assert!(shown(&s, &c)["signed_in"].as_bool().unwrap());
        let text = fs::read_to_string(file(&s, "far", "oauth.json")).unwrap();
        assert!(text.contains("tok1"));

        // tok1 is all but spent, so the relay renews before it sends.
        let st = s.clone();
        let said = tokio::task::spawn_blocking(move || talk(&st, "far"))
            .await
            .unwrap();
        let list = said.answer(2);
        assert_eq!(list["result"]["token"], "tok2", "{list}");
        assert_eq!(far.issued.lock().unwrap().len(), 2);
        assert!(
            fs::read_to_string(file(&s, "far", "oauth.json"))
                .unwrap()
                .contains("r2")
        );
        // A new address forgets the sign-in made with the old one.
        put(&s, change("far", &format!("{base}/other"), None)).unwrap();
        assert!(!file(&s, "far", "oauth.json").exists());
        let _ = fs::remove_dir_all(&s);
    }
    #[test]
    fn a_state_carries_the_origin_beside_its_nonce() {
        let state = carry("n0nce-_x", "http://box.tail.example.net:6780");
        assert!(state.starts_with("n0nce-_x."));
        assert_eq!(
            carried(&state),
            (
                "n0nce-_x",
                Some("http://box.tail.example.net:6780".to_owned())
            )
        );
        assert_eq!(carried("plain"), ("plain", None));
        assert_eq!(carried("n.!!!"), ("n", None));
    }

    #[test]
    fn only_a_plain_origin_is_carried() {
        assert_eq!(
            checked("https://libri.example.com"),
            Some("https://libri.example.com".to_owned())
        );
        assert_eq!(
            checked("http://192.168.1.20:6780/"),
            Some("http://192.168.1.20:6780".to_owned())
        );
        for refused in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://ada:pw@libri.example.com",
            "https://libri.example.com/elsewhere",
            "https://libri.example.com/?next=x",
            "https://libri.example.com/#x",
            "libri.example.com",
            "",
        ] {
            assert_eq!(checked(refused), None, "{refused}");
        }
    }

    #[test]
    fn a_sign_in_returns_by_the_page_unless_on_the_host() {
        let back = Back::new(None, 6780);
        let local = "http://127.0.0.1:6780/api/connections/back".to_owned();
        assert_eq!(
            back.choose(Some("http://box.tail.example.net:6780"), false),
            (
                BACK.to_owned(),
                Some("http://box.tail.example.net:6780".to_owned())
            )
        );
        assert_eq!(
            back.choose(Some("http://localhost:6780"), false),
            (local.clone(), None)
        );
        assert_eq!(
            back.choose(Some("https://ada:pw@x.example"), false),
            (local.clone(), None)
        );
        assert_eq!(back.choose(None, false), (local.clone(), None));
        let fork = Back::new(Some("https://fork.example.com/connect/back"), 6780);
        assert_eq!(
            fork.choose(Some("http://10.0.0.2:6780"), false).0,
            "https://fork.example.com/connect/back"
        );
        let off = Back::new(Some(""), 6780);
        assert_eq!(
            off.choose(Some("http://10.0.0.2:6780"), false),
            (local, None)
        );
        assert_eq!(
            back.choose(Some("http://box.tail.example.net:6780"), true),
            (APP_BACK.to_owned(), None)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_app_signs_in_by_its_own_scheme() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (base, far) = far().await;
        let s = scratch("oauth-app");
        put(&s, change("far", &format!("{base}/mcp"), None)).unwrap();
        let pending = Pending::default();
        let page = sign_in(&s, &Back::new(None, 6780), None, &pending, "far", true)
            .await
            .unwrap();
        let page = reqwest::Url::parse(&page).unwrap();
        let q: HashMap<String, String> = page.query_pairs().into_owned().collect();
        assert_eq!(q["redirect_uri"], APP_BACK);
        let asked = far.registered.lock().unwrap()[0].clone();
        assert_eq!(asked["application_type"], "native");
        // The app hands on the address it was opened with.
        let opened = format!("{APP_BACK}?code=c1&state={}", q["state"]);
        assert_eq!(finish(&pending, &opened).await.unwrap(), "far");
        let _ = fs::remove_dir_all(&s);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_server_refusing_the_page_gets_the_loopback_return() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (base, far) = far().await;
        far.no_https
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let s = scratch("oauth-no-page");
        put(&s, change("far", &format!("{base}/mcp"), None)).unwrap();
        let pending = Pending::default();
        let phone = "http://box.tail.example.net:6780";
        let page = sign_in(
            &s,
            &Back::new(None, 6780),
            Some(phone),
            &pending,
            "far",
            false,
        )
        .await
        .unwrap();
        let page = reqwest::Url::parse(&page).unwrap();
        let q: HashMap<String, String> = page.query_pairs().into_owned().collect();
        let local = "http://127.0.0.1:6780/api/connections/back";
        assert_eq!(q["redirect_uri"], local);
        assert_eq!(carried(&q["state"]).1, None);
        let tried: Vec<Value> = far
            .registered
            .lock()
            .unwrap()
            .iter()
            .map(|r| r["redirect_uris"][0].clone())
            .collect();
        assert_eq!(tried, [json!(BACK), json!(local)]);
        // The phone's browser cannot load it, so its address is pasted.
        let pasted = format!("{local}?code=c1&state={}", q["state"]);
        assert_eq!(finish(&pending, &pasted).await.unwrap(), "far");
        let _ = fs::remove_dir_all(&s);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_sign_in_bounced_by_the_page_finishes() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (base, far) = far().await;
        let s = scratch("oauth-bounce");
        put(&s, change("far", &format!("{base}/mcp"), None)).unwrap();
        let c = get(&s, "far").unwrap();
        let pending = Pending::default();
        let phone = "http://box.tail.example.net:6780";
        let page = sign_in(
            &s,
            &Back::new(None, 6780),
            Some(phone),
            &pending,
            "far",
            false,
        )
        .await
        .unwrap();
        let page = reqwest::Url::parse(&page).unwrap();
        let q: HashMap<String, String> = page.query_pairs().into_owned().collect();
        assert_eq!(q["redirect_uri"], BACK);
        assert_eq!(q["code_challenge_method"], "S256");
        let asked = far.registered.lock().unwrap()[0].clone();
        assert_eq!(asked["redirect_uris"], json!([BACK]));
        assert_eq!(asked["application_type"], "web");
        assert_eq!(carried(&q["state"]).1.as_deref(), Some(phone));
        // The page sends the browser on with the query as the server gave it.
        let query = format!("code=c1&state={}", q["state"]);
        let bounced = format!("{phone}/api/connections/back?{query}");
        assert_eq!(finish(&pending, &bounced).await.unwrap(), "far");
        assert!(shown(&s, &c)["signed_in"].as_bool().unwrap());
        assert!(
            finish(&pending, &bounced).await.is_err(),
            "a code is used once"
        );
        let _ = fs::remove_dir_all(&s);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_page_address_pasted_finishes_too() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (base, _far) = far().await;
        let s = scratch("oauth-pasted");
        put(&s, change("far", &format!("{base}/mcp"), None)).unwrap();
        let pending = Pending::default();
        let page = sign_in(
            &s,
            &Back::new(None, 6780),
            Some("http://10.0.0.2:6780"),
            &pending,
            "far",
            false,
        )
        .await
        .unwrap();
        let page = reqwest::Url::parse(&page).unwrap();
        let q: HashMap<String, String> = page.query_pairs().into_owned().collect();
        // Where the page could not send the browser on, its own address is
        // pasted into Libri.
        let mut stuck = reqwest::Url::parse(BACK).unwrap();
        stuck
            .query_pairs_mut()
            .append_pair("code", "c1")
            .append_pair("state", &q["state"]);
        assert_eq!(finish(&pending, stuck.as_str()).await.unwrap(), "far");
        let _ = fs::remove_dir_all(&s);
    }
}
