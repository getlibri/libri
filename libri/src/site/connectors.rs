//! Connectors: the outside apps let in at /mcp, each with a login of its
//! own that the person can take back, and the modules of tools it may use.
//!
//! An app signs in as the MCP authorization spec (2026-07-28) has it, the
//! way Claude's and ChatGPT's custom connectors, Claude Code, Codex and
//! Cursor expect: /mcp answers 401 with its Protected Resource Metadata
//! (RFC 9728), which names Libri itself as the authorization server
//! (RFC 8414 metadata). The app identifies itself by a Client ID Metadata
//! Document, the spec's first choice, or registers (RFC 7591), deprecated
//! but still how most apps do it; then the person's browser comes to the
//! consent page here, and the code returns to the app with `iss`
//! (RFC 9207), redeemed only with its PKCE verifier (S256). Tokens are
//! bound to this /mcp and kept only as hashes.
//!
//! Linear's, Notion's and GitHub's hosted MCP servers are their own
//! authorization servers in the same way. Where Libri departs: the consent
//! page is the person's own host, behind the gate every page is (the
//! tailnet, a paired device), so there is no account to sign in to; and an
//! app that only takes a header gets a key pasted from Settings, as
//! GitHub's server takes a personal access token. Tailscale's OAuth apps
//! (alpha) could stand in as the authorization server, but sign in tailnet
//! members only and leave apps outside the tailnet no better off.
//!
//! What a connector may use is its modules, picked at consent or in
//! Settings; a change applies to its next call. None of them deploys,
//! merges, releases or answers for the person: what an app sends goes in
//! under the connector's name, never as the person's own word.

use super::Site;
use axum::extract::{Form, Path as UrlPath, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// How long an access token lasts; the refresh token, rotated at each use,
/// lasts until the connector is revoked.
const ACCESS_LIFE: u64 = 3600;
/// How long a code waits to be redeemed, and a consent page to be answered.
const CODE_LIFE: Duration = Duration::from_secs(120);
const ASK_LIFE: Duration = Duration::from_secs(900);
/// Registered apps kept; past this the oldest no connector uses goes.
const CLIENTS_MAX: usize = 200;
/// The longest name a connector or app is known by.
const NAME_MAX: usize = 60;
/// How often a connector's last use is written down.
const USED_EVERY: u64 = 60;

/// A set of tools a connector may be let use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Module {
    /// Search, open and date the main memory.
    Memory,
    /// List the workers, read their chats, start one, tell one.
    Workers,
    /// Read and wait on the chats, send into the main chat, see what waits
    /// on the person.
    Chats,
    /// The web, X, pictures and the Library's pages.
    Extras,
}

impl Module {
    pub const ALL: [Module; 4] = [
        Module::Memory,
        Module::Workers,
        Module::Chats,
        Module::Extras,
    ];
    /// What a new connector gets unless the person picks otherwise.
    pub const START: [Module; 2] = [Module::Memory, Module::Workers];

    pub fn id(self) -> &'static str {
        match self {
            Module::Memory => "memory",
            Module::Workers => "workers",
            Module::Chats => "chats",
            Module::Extras => "extras",
        }
    }

    fn parse(id: &str) -> Option<Module> {
        Module::ALL.into_iter().find(|m| m.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            Module::Memory => "Memory",
            Module::Workers => "Workers",
            Module::Chats => "Chats",
            Module::Extras => "Web, X, pictures and pages",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Module::Memory => "Search and open your memory. Read only.",
            Module::Workers => {
                "List workers and read their chats, start one, tell one. Under the connector's name, never as you."
            }
            Module::Chats => {
                "Read and wait on your chats, see what waits on you, and send into the main chat under the connector's name."
            }
            Module::Extras => "Search the web and X, draw, and publish to your Library.",
        }
    }
}

/// How a key was given.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    /// Shown once in Settings, for an app that takes a header; never
    /// expires.
    Pasted,
    Access,
    Refresh,
}

#[derive(Clone, Serialize, Deserialize)]
struct Key {
    hash: String,
    kind: Kind,
    /// Unix seconds; 0 for never.
    #[serde(default)]
    expires: u64,
}

/// An app let in: what it is called, what it may use, its keys as hashes.
#[derive(Clone, Serialize, Deserialize)]
pub struct Connector {
    pub id: String,
    pub name: String,
    pub modules: Vec<Module>,
    /// The app that signed in to it by OAuth, by its client id and name.
    #[serde(default)]
    pub client: Option<String>,
    #[serde(default)]
    pub app: Option<String>,
    pub made: u64,
    #[serde(default)]
    pub used: u64,
    #[serde(default)]
    keys: Vec<Key>,
}

/// Who a request to /mcp comes from, as the gate found it.
#[derive(Clone, Debug)]
pub struct Grant {
    pub id: String,
    pub name: String,
    pub modules: Vec<Module>,
}

impl Grant {
    pub fn may(&self, m: Module) -> bool {
        self.modules.contains(&m)
    }
}

/// An app that registered (RFC 7591).
#[derive(Clone, Serialize, Deserialize)]
struct Client {
    id: String,
    name: String,
    redirect_uris: Vec<String>,
    made: u64,
}

const CONNECTORS: &str = "connectors.json";
const CLIENTS: &str = "clients.json";

/// One writer at a time: a token issued while another is revoked must not
/// bring the revoked one back.
static STORE: Mutex<()> = Mutex::new(());

fn connectors(state: &Path) -> Vec<Connector> {
    crate::host::read_json(&crate::host::dir(state).join(CONNECTORS))
}

fn keep(state: &Path, all: &[Connector]) -> Result<(), String> {
    crate::host::write_json(state, CONNECTORS, &all)
}

fn now() -> u64 {
    crate::host::unix()
}

fn named(name: &str) -> String {
    name.trim()
        .chars()
        .take(NAME_MAX)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn modules_of(picked: &[Module]) -> Vec<Module> {
    Module::ALL
        .into_iter()
        .filter(|m| picked.contains(m))
        .collect()
}

/// The connectors as Settings lists them, newest last.
pub fn list(state: &Path) -> Vec<Connector> {
    connectors(state)
}

/// A new connector and its key, shown to the person once.
pub fn add(state: &Path, name: &str, modules: &[Module]) -> Result<(Connector, String), String> {
    let key = crate::host::token()?;
    let c = make(state, name, modules, Some(&key), None)?;
    Ok((c, key))
}

/// A new connector: with a pasted key, or signed in to by `app`.
fn make(
    state: &Path,
    name: &str,
    modules: &[Module],
    key: Option<&str>,
    app: Option<&Client>,
) -> Result<Connector, String> {
    let _held = STORE.lock().unwrap();
    let name = named(name);
    if name.is_empty() {
        return Err("A connector needs a name.".into());
    }
    let c = Connector {
        id: crate::host::hex(&crate::host::random::<6>()?),
        name,
        modules: modules_of(modules),
        client: app.map(|a| a.id.clone()),
        app: app.map(|a| a.name.clone()),
        made: now(),
        used: 0,
        keys: key
            .map(|k| Key {
                hash: crate::host::hash(k),
                kind: Kind::Pasted,
                expires: 0,
            })
            .into_iter()
            .collect(),
    };
    let mut all = connectors(state);
    all.push(c.clone());
    keep(state, &all)?;
    Ok(c)
}

/// Changes what a connector may use, from its next call on.
pub fn set_modules(state: &Path, id: &str, modules: &[Module]) -> Result<Connector, String> {
    let _held = STORE.lock().unwrap();
    let mut all = connectors(state);
    let c = all
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or("No such connector.")?;
    c.modules = modules_of(modules);
    let c = c.clone();
    keep(state, &all)?;
    Ok(c)
}

/// Takes a connector back: every key it holds stops working at once.
pub fn revoke(state: &Path, id: &str) -> Result<Connector, String> {
    let _held = STORE.lock().unwrap();
    let mut all = connectors(state);
    let at = all
        .iter()
        .position(|c| c.id == id)
        .ok_or("No such connector.")?;
    let gone = all.remove(at);
    keep(state, &all)?;
    Ok(gone)
}

/// The connector a bearer token belongs to, while the token holds.
pub fn grant(state: &Path, token: &str) -> Option<Grant> {
    let h = crate::host::hash(token);
    let t = now();
    let mut all = connectors(state);
    let c = all.iter_mut().find(|c| {
        c.keys.iter().any(|k| {
            k.kind != Kind::Refresh
                && (k.expires == 0 || k.expires > t)
                && crate::host::same(&k.hash, &h)
        })
    })?;
    let g = Grant {
        id: c.id.clone(),
        name: c.name.clone(),
        modules: c.modules.clone(),
    };
    if t >= c.used + USED_EVERY {
        let _held = STORE.lock().unwrap();
        // Read again under the lock: only the time of use is written.
        let mut fresh = connectors(state);
        if let Some(f) = fresh.iter_mut().find(|f| f.id == g.id) {
            f.used = t;
            let _ = keep(state, &fresh);
        }
        c.used = t;
    }
    Some(g)
}

/// What the token endpoint hands an app.
#[derive(Serialize)]
struct Tokens {
    access_token: String,
    token_type: &'static str,
    expires_in: u64,
    refresh_token: String,
}

/// New tokens for connector `id`, signed in by `client`; `spent`, a refresh
/// token, goes in the same write.
fn issue(state: &Path, id: &str, client: &Client, spent: Option<&str>) -> Result<Tokens, String> {
    let _held = STORE.lock().unwrap();
    let mut all = connectors(state);
    let c = all
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or("The connector was revoked.")?;
    let t = now();
    // Checked again under the lock: two refreshes with one token get one
    // answer between them.
    if let Some(s) = spent
        && !c
            .keys
            .iter()
            .any(|k| k.kind == Kind::Refresh && crate::host::same(&k.hash, s))
    {
        return Err("The refresh token is used.".into());
    }
    c.keys.retain(|k| {
        !(k.kind == Kind::Access && k.expires <= t)
            && spent.is_none_or(|s| !(k.kind == Kind::Refresh && crate::host::same(&k.hash, s)))
    });
    let (access, refresh) = (crate::host::token()?, crate::host::token()?);
    c.keys.push(Key {
        hash: crate::host::hash(&access),
        kind: Kind::Access,
        expires: t + ACCESS_LIFE,
    });
    c.keys.push(Key {
        hash: crate::host::hash(&refresh),
        kind: Kind::Refresh,
        expires: 0,
    });
    c.client = Some(client.id.clone());
    c.app = Some(client.name.clone());
    keep(state, &all)?;
    Ok(Tokens {
        access_token: access,
        token_type: "Bearer",
        expires_in: ACCESS_LIFE,
        refresh_token: refresh,
    })
}

// ---- Apps ----

fn clients(state: &Path) -> Vec<Client> {
    crate::host::read_json(&crate::host::dir(state).join(CLIENTS))
}

/// Whether an app may be sent back to `uri`: https, a custom scheme (a
/// desktop app's), or http to this machine only (RFC 8252).
fn returnable(uri: &str) -> bool {
    let Some((scheme, rest)) = uri.split_once(':') else {
        return false;
    };
    if uri.contains('#') {
        return false;
    }
    match scheme.to_ascii_lowercase().as_str() {
        "https" => rest.starts_with("//"),
        "http" => loopback(uri).is_some(),
        "javascript" | "data" | "file" | "vbscript" => false,
        s => {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
        }
    }
}

/// A loopback http address without its port, which RFC 8252 lets vary.
fn loopback(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("http://")?;
    let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let bare = match host.rsplit_once(':') {
        Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
        _ => host,
    };
    matches!(bare, "127.0.0.1" | "localhost" | "[::1]").then(|| format!("{bare}{path}"))
}

fn same_return(registered: &str, asked: &str) -> bool {
    registered == asked || loopback(registered).is_some_and(|r| loopback(asked) == Some(r))
}

#[derive(Deserialize)]
struct Registration {
    #[serde(default)]
    redirect_uris: Vec<String>,
    #[serde(default)]
    client_name: Option<String>,
}

fn register(state: &Path, r: Registration) -> Result<Client, String> {
    if r.redirect_uris.is_empty() || r.redirect_uris.len() > 10 {
        return Err("redirect_uris must name one to ten addresses".into());
    }
    if let Some(bad) = r.redirect_uris.iter().find(|u| !returnable(u)) {
        return Err(format!(
            "{bad} is not an address an app may be sent back to"
        ));
    }
    let _held = STORE.lock().unwrap();
    let c = Client {
        id: format!("libri-{}", crate::host::hex(&crate::host::random::<12>()?)),
        name: named(r.client_name.as_deref().unwrap_or("An app")),
        redirect_uris: r.redirect_uris,
        made: now(),
    };
    let mut all = clients(state);
    let used: Vec<String> = connectors(state)
        .into_iter()
        .filter_map(|c| c.client)
        .collect();
    while all.len() >= CLIENTS_MAX {
        let Some(at) = all.iter().position(|c| !used.contains(&c.id)) else {
            break;
        };
        all.remove(at);
    }
    all.push(c.clone());
    crate::host::write_json(state, CLIENTS, &all)?;
    Ok(c)
}

/// The app a client id names: one that registered, or one whose id is the
/// address of its metadata document, read from there.
async fn client(state: &Path, id: &str) -> Result<Client, String> {
    if let Some(c) = clients(state).into_iter().find(|c| c.id == id) {
        return Ok(c);
    }
    if !id.starts_with("https://") {
        return Err(
            "Libri does not know this app: it neither registered nor names its metadata.".into(),
        );
    }
    let url = id.to_owned();
    let doc: serde_json::Value = tokio::task::spawn_blocking(move || {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .max_redirects(0)
            .build()
            .new_agent();
        agent
            .get(&url)
            .header("Accept", "application/json")
            .call()
            .map_err(|e| format!("{url}: {e}"))?
            .body_mut()
            .with_config()
            .limit(64 * 1024)
            .read_json()
            .map_err(|e| format!("{url}: {e}"))
    })
    .await
    .map_err(|e| e.to_string())??;
    if doc["client_id"].as_str() != Some(id) {
        return Err("The app's metadata names another client_id.".into());
    }
    let redirect_uris: Vec<String> = doc["redirect_uris"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|u| u.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if redirect_uris.is_empty() || !redirect_uris.iter().all(|u| returnable(u)) {
        return Err("The app's metadata names no address it may be sent back to.".into());
    }
    Ok(Client {
        id: id.to_owned(),
        name: named(doc["client_name"].as_str().unwrap_or(id)),
        redirect_uris,
        made: now(),
    })
}

// ---- Sign-in in progress ----

/// A consent page shown and not yet answered, by its ticket.
struct Asked {
    at: Instant,
    client: Client,
    redirect_uri: String,
    challenge: String,
    state: Option<String>,
    issuer: String,
}

/// A code handed back and not yet redeemed, by its hash.
struct Code {
    at: Instant,
    connector: String,
    client: Client,
    redirect_uri: String,
    challenge: String,
}

/// Held in memory: a restart in between means signing in again.
static ASKED: LazyLock<Mutex<HashMap<String, Asked>>> = LazyLock::new(Default::default);
static CODES: LazyLock<Mutex<HashMap<String, Code>>> = LazyLock::new(Default::default);

/// The address the app reached this host at, which names the issuer and
/// the resource: a client checks both against the address it used.
pub fn base(headers: &HeaderMap) -> String {
    let text = |h: &str| headers.get(h).and_then(|v| v.to_str().ok());
    let host = text("x-forwarded-host")
        .or_else(|| text(header::HOST.as_str()))
        .unwrap_or("localhost");
    let scheme = text("x-forwarded-proto").unwrap_or("http");
    format!("{scheme}://{host}")
}

pub fn resource(headers: &HeaderMap) -> String {
    format!("{}/mcp", base(headers))
}

fn metadata_url(headers: &HeaderMap) -> String {
    format!("{}/.well-known/oauth-protected-resource/mcp", base(headers))
}

/// The 401 that starts an app's sign-in.
pub fn challenge(headers: &HeaderMap, why: &str) -> Response {
    let mut r = (StatusCode::UNAUTHORIZED, why.to_owned()).into_response();
    if let Ok(v) = HeaderValue::from_str(&format!(
        "Bearer resource_metadata=\"{}\"",
        metadata_url(headers)
    )) {
        r.headers_mut().insert(header::WWW_AUTHENTICATE, v);
    }
    r
}

/// The connector a request to /mcp comes as, from its bearer token.
pub fn bearer(state: &Path, headers: &HeaderMap) -> Option<Grant> {
    let token = headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")?;
    grant(state, token.trim())
}

// ---- Routes ----

/// The discovery documents and the sign-in, reached by apps that hold no
/// session: open to any origin, as they carry no cookie.
pub fn routes() -> Router<Arc<Site>> {
    Router::new()
        .route("/.well-known/oauth-protected-resource", get(protected))
        .route("/.well-known/oauth-protected-resource/mcp", get(protected))
        .route("/.well-known/oauth-authorization-server", get(server))
        .route("/.well-known/oauth-authorization-server/mcp", get(server))
        .route("/oauth/register", post(registered).options(preflight))
        .route("/oauth/token", post(token).options(preflight))
        .route("/oauth/authorize", get(ask).post(answer))
        .route("/api/connectors", get(listed).post(added))
        .route("/api/connectors/{id}", post(changed))
        .route("/api/connectors/{id}/revoke", post(revoked))
}

fn open(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("authorization, content-type, mcp-protocol-version"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

async fn preflight() -> Response {
    open(StatusCode::NO_CONTENT.into_response())
}

async fn protected(headers: HeaderMap) -> Response {
    open(
        Json(serde_json::json!({
            "resource": resource(&headers),
            "authorization_servers": [base(&headers)],
            "bearer_methods_supported": ["header"],
            "resource_name": "Libri",
        }))
        .into_response(),
    )
}

async fn server(headers: HeaderMap) -> Response {
    let b = base(&headers);
    open(
        Json(serde_json::json!({
            "issuer": b,
            "authorization_endpoint": format!("{b}/oauth/authorize"),
            "token_endpoint": format!("{b}/oauth/token"),
            "registration_endpoint": format!("{b}/oauth/register"),
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["none"],
            "client_id_metadata_document_supported": true,
            "authorization_response_iss_parameter_supported": true,
        }))
        .into_response(),
    )
}

fn oauth_error(status: StatusCode, error: &str, about: &str) -> Response {
    open(
        (
            status,
            Json(serde_json::json!({"error": error, "error_description": about})),
        )
            .into_response(),
    )
}

async fn registered(State(site): State<Arc<Site>>, Json(r): Json<Registration>) -> Response {
    let state = site.state.clone();
    match tokio::task::spawn_blocking(move || register(&state, r)).await {
        Ok(Ok(c)) => open(
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "client_id": c.id,
                    "client_name": c.name,
                    "redirect_uris": c.redirect_uris,
                    "token_endpoint_auth_method": "none",
                    "grant_types": ["authorization_code", "refresh_token"],
                    "response_types": ["code"],
                    "client_id_issued_at": c.made,
                })),
            )
                .into_response(),
        ),
        Ok(Err(e)) => oauth_error(StatusCode::BAD_REQUEST, "invalid_redirect_uri", &e),
        Err(e) => oauth_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &e.to_string(),
        ),
    }
}

/// The sign-in page an app sends the person's browser to.
async fn ask(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    if !site.signed(&headers) {
        return page(
            StatusCode::UNAUTHORIZED,
            "Open Libri on this device first",
            "<p>This device is not signed in to Libri. Open Libri here, then try the app's sign-in again.</p>",
        );
    }
    let arg = |k: &str| q.get(k).map(String::as_str).filter(|v| !v.is_empty());
    // Faults in the app or its address are shown here, never sent on: the
    // address may not be the app's.
    let Some(client_id) = arg("client_id") else {
        return refused("The app sent no client_id.");
    };
    let app = match client(&site.state, client_id).await {
        Ok(c) => c,
        Err(e) => return refused(&e),
    };
    let redirect_uri = match arg("redirect_uri") {
        Some(u) if app.redirect_uris.iter().any(|r| same_return(r, u)) => u.to_owned(),
        None if app.redirect_uris.len() == 1 => app.redirect_uris[0].clone(),
        _ => return refused("The app asked to be sent back to an address it did not register."),
    };
    let issuer = base(&headers);
    let back = |error: &str| back_to(&redirect_uri, &[("error", error)], arg("state"), &issuer);
    if arg("response_type") != Some("code") {
        return back("unsupported_response_type");
    }
    let (Some(challenge), Some("S256")) = (arg("code_challenge"), arg("code_challenge_method"))
    else {
        return back("invalid_request");
    };
    if let Some(r) = arg("resource")
        && !same_resource(r, &resource(&headers))
    {
        return back("invalid_target");
    }
    let Ok(ticket) = crate::host::token() else {
        return back("server_error");
    };
    let shown = choices(&site.state, &app);
    {
        let mut asked = ASKED.lock().unwrap();
        asked.retain(|_, a| a.at.elapsed() < ASK_LIFE);
        asked.insert(
            ticket.clone(),
            Asked {
                at: Instant::now(),
                client: app.clone(),
                redirect_uri: redirect_uri.clone(),
                challenge: challenge.to_owned(),
                state: arg("state").map(str::to_owned),
                issuer,
            },
        );
    }
    let to = escape(&redirect_uri);
    let to_host = escape(
        redirect_uri
            .split_once("://")
            .map_or(redirect_uri.as_str(), |(_, r)| {
                r.split('/').next().unwrap_or(r)
            }),
    );
    let mut body = format!(
        "<p><b>{name}</b> asks to use Libri. It will be sent back to <span title=\"{to}\">{to_host}</span>.</p>\
         <form method=\"post\" action=\"/oauth/authorize\">\
         <input type=\"hidden\" name=\"ticket\" value=\"{ticket}\">",
        name = escape(&app.name),
    );
    for (i, c) in shown.iter().enumerate() {
        body.push_str(&format!(
            "<label class=\"pick\"><input type=\"radio\" name=\"connector\" value=\"{id}\"{on}> \
             As <b>{name}</b><small>{mods}</small></label>",
            id = escape(&c.id),
            on = if i == 0 { " checked" } else { "" },
            name = escape(&c.name),
            mods = escape(&titles(&c.modules)),
        ));
    }
    body.push_str(&format!(
        "<label class=\"pick\"><input type=\"radio\" name=\"connector\" value=\"new\"{on}> As a new connector \
         <input type=\"text\" name=\"name\" value=\"{name}\" maxlength=\"{NAME_MAX}\" aria-label=\"Its name\"></label>\
         <fieldset><legend>A new connector may use</legend>",
        on = if shown.is_empty() { " checked" } else { "" },
        name = escape(&app.name),
    ));
    for m in Module::ALL {
        body.push_str(&format!(
            "<label class=\"module\"><input type=\"checkbox\" name=\"module_{id}\"{on}> {title}<small>{about}</small></label>",
            id = m.id(),
            on = if Module::START.contains(&m) { " checked" } else { "" },
            title = escape(m.title()),
            about = escape(m.about()),
        ));
    }
    body.push_str(
        "</fieldset><p class=\"note\">Nothing here deploys, merges or answers for you. Revoke it any time in Settings › Other apps.</p>\
         <div class=\"act\"><button name=\"verdict\" value=\"deny\">Deny</button>\
         <button name=\"verdict\" value=\"allow\" class=\"go\">Allow</button></div></form>",
    );
    page(StatusCode::OK, "Let an app in", &body)
}

/// The connectors an app may sign in as again: those it made. One made by
/// key in Settings stays the key's.
fn choices(state: &Path, app: &Client) -> Vec<Connector> {
    connectors(state)
        .into_iter()
        .filter(|c| c.client.as_deref() == Some(app.id.as_str()))
        .collect()
}

fn titles(modules: &[Module]) -> String {
    if modules.is_empty() {
        return "Nothing yet".into();
    }
    modules
        .iter()
        .map(|m| m.title())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The person's answer on the sign-in page.
async fn answer(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Form(f): Form<HashMap<String, String>>,
) -> Response {
    if !site.signed(&headers) || !site.admits(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    // The ticket is what the page was shown with: a form posted from
    // anywhere else has none, so cannot let an app in.
    let Some(a) = f
        .get("ticket")
        .and_then(|t| ASKED.lock().unwrap().remove(t))
        .filter(|a| a.at.elapsed() < ASK_LIFE)
    else {
        return refused("This sign-in page is out of date: start again from the app.");
    };
    let back =
        |pairs: &[(&str, &str)]| back_to(&a.redirect_uri, pairs, a.state.as_deref(), &a.issuer);
    if f.get("verdict").map(String::as_str) != Some("allow") {
        return back(&[("error", "access_denied")]);
    }
    let state = site.state.clone();
    let picked = f.get("connector").cloned().unwrap_or_default();
    let name = f
        .get("name")
        .cloned()
        .unwrap_or_else(|| a.client.name.clone());
    let modules: Vec<Module> = Module::ALL
        .into_iter()
        .filter(|m| f.contains_key(&format!("module_{}", m.id())))
        .collect();
    let app = a.client.clone();
    let connector = tokio::task::spawn_blocking(move || {
        if picked == "new" {
            // An app that signs in needs no pasted key.
            return make(&state, &name, &modules, None, Some(&app)).map(|c| c.id);
        }
        choices(&state, &app)
            .into_iter()
            .find(|c| c.id == picked)
            .map(|c| c.id)
            .ok_or_else(|| "That connector is gone.".to_owned())
    })
    .await;
    let id = match connector {
        Ok(Ok(id)) => id,
        Ok(Err(e)) => return refused(&e),
        Err(e) => return refused(&e.to_string()),
    };
    let Ok(code) = crate::host::token() else {
        return back(&[("error", "server_error")]);
    };
    {
        let mut codes = CODES.lock().unwrap();
        codes.retain(|_, c| c.at.elapsed() < CODE_LIFE);
        codes.insert(
            crate::host::hash(&code),
            Code {
                at: Instant::now(),
                connector: id,
                client: a.client.clone(),
                redirect_uri: a.redirect_uri.clone(),
                challenge: a.challenge.clone(),
            },
        );
    }
    back(&[("code", &code)])
}

/// Sends the browser back to the app, with `iss` as RFC 9207 has it.
fn back_to(uri: &str, pairs: &[(&str, &str)], state: Option<&str>, issuer: &str) -> Response {
    let mut to = uri.to_owned();
    let mut sep = if to.contains('?') { '&' } else { '?' };
    let state = state.map(|s| ("state", s));
    for (k, v) in pairs.iter().copied().chain(state).chain([("iss", issuer)]) {
        to.push(sep);
        to.push_str(k);
        to.push('=');
        to.push_str(&urlencode(v));
        sep = '&';
    }
    Redirect::to(&to).into_response()
}

fn same_resource(asked: &str, ours: &str) -> bool {
    let fold = |u: &str| {
        let u = u.trim_end_matches('/');
        match u.split_once("://") {
            Some((s, rest)) => {
                let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
                format!(
                    "{}://{}{path}",
                    s.to_ascii_lowercase(),
                    host.to_ascii_lowercase()
                )
            }
            None => u.to_owned(),
        }
    };
    let (asked, ours) = (fold(asked), fold(ours));
    // The bare host names the server too (the spec's own example).
    asked == ours || ours.strip_suffix("/mcp") == Some(asked.as_str())
}

async fn token(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    Form(f): Form<HashMap<String, String>>,
) -> Response {
    let arg = |k: &str| f.get(k).map(String::as_str).filter(|v| !v.is_empty());
    if let Some(r) = arg("resource")
        && !same_resource(r, &resource(&headers))
    {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_target",
            "That resource is not this server.",
        );
    }
    let state = site.state.clone();
    let issued = match arg("grant_type") {
        Some("authorization_code") => {
            let Some(code) =
                arg("code").and_then(|c| CODES.lock().unwrap().remove(&crate::host::hash(c)))
            else {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "The code is unknown or used.",
                );
            };
            let verifier = arg("code_verifier").unwrap_or_default();
            let proof = URL_SAFE_NO_PAD.encode(ring::digest::digest(
                &ring::digest::SHA256,
                verifier.as_bytes(),
            ));
            let fits = code.at.elapsed() < CODE_LIFE
                && arg("client_id").is_none_or(|c| c == code.client.id)
                && arg("redirect_uri").is_none_or(|u| u == code.redirect_uri)
                && crate::host::same(&proof, &code.challenge);
            if !fits {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "The code does not fit this request.",
                );
            }
            tokio::task::spawn_blocking(move || issue(&state, &code.connector, &code.client, None))
                .await
        }
        Some("refresh_token") => {
            let Some(spent) = arg("refresh_token").map(crate::host::hash) else {
                return oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "No refresh_token.",
                );
            };
            let client_id = arg("client_id").map(str::to_owned);
            tokio::task::spawn_blocking(move || {
                let c = connectors(&state)
                    .into_iter()
                    .find(|c| {
                        c.keys
                            .iter()
                            .any(|k| k.kind == Kind::Refresh && crate::host::same(&k.hash, &spent))
                    })
                    .ok_or("The refresh token is unknown, used or revoked.")?;
                if client_id.is_some() && client_id != c.client {
                    return Err("The refresh token is another app's.".to_owned());
                }
                let app = Client {
                    id: c.client.clone().unwrap_or_default(),
                    name: c.app.clone().unwrap_or_default(),
                    redirect_uris: Vec::new(),
                    made: 0,
                };
                issue(&state, &c.id, &app, Some(&spent))
            })
            .await
        }
        _ => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "unsupported_grant_type",
                "authorization_code or refresh_token.",
            );
        }
    };
    match issued {
        Ok(Ok(t)) => open(Json(t).into_response()),
        Ok(Err(e)) => oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", &e),
        Err(e) => oauth_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &e.to_string(),
        ),
    }
}

// ---- Settings ----

/// A connector as Settings shows it: never its keys.
#[derive(Serialize)]
struct Shown {
    id: String,
    name: String,
    modules: Vec<Module>,
    app: Option<String>,
    made: u64,
    used: u64,
}

impl From<Connector> for Shown {
    fn from(c: Connector) -> Self {
        Self {
            id: c.id,
            name: c.name,
            modules: c.modules,
            app: c.app,
            made: c.made,
            used: c.used,
        }
    }
}

#[derive(Serialize)]
struct ModuleShown {
    id: &'static str,
    title: &'static str,
    about: &'static str,
    start: bool,
}

async fn listed(State(site): State<Arc<Site>>, headers: HeaderMap) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let modules: Vec<ModuleShown> = Module::ALL
        .into_iter()
        .map(|m| ModuleShown {
            id: m.id(),
            title: m.title(),
            about: m.about(),
            start: Module::START.contains(&m),
        })
        .collect();
    let all: Vec<Shown> = list(&site.state).into_iter().map(Shown::from).collect();
    Json(serde_json::json!({
        "url": resource(&headers),
        "modules": modules,
        "connectors": all,
    }))
    .into_response()
}

#[derive(Deserialize)]
struct Made {
    name: String,
    #[serde(default)]
    modules: Vec<String>,
}

fn parsed(ids: &[String]) -> Result<Vec<Module>, String> {
    ids.iter()
        .map(|id| Module::parse(id).ok_or_else(|| format!("No module {id}.")))
        .collect()
}

async fn added(State(site): State<Arc<Site>>, headers: HeaderMap, Json(m): Json<Made>) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let modules = match parsed(&m.modules) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    match add(&site.state, &m.name, &modules) {
        Ok((c, key)) => {
            Json(serde_json::json!({"connector": Shown::from(c), "key": key})).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

#[derive(Deserialize)]
struct Changed {
    modules: Vec<String>,
}

async fn changed(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
    Json(c): Json<Changed>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let modules = match parsed(&c.modules) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    match set_modules(&site.state, &id, &modules) {
        Ok(c) => Json(Shown::from(c)).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

async fn revoked(
    State(site): State<Arc<Site>>,
    headers: HeaderMap,
    UrlPath(id): UrlPath<String>,
) -> Response {
    if !site.signed(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match revoke(&site.state, &id) {
        Ok(c) => Json(Shown::from(c)).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e).into_response(),
    }
}

// ---- The page ----

fn refused(why: &str) -> Response {
    page(
        StatusCode::BAD_REQUEST,
        "This sign-in cannot go on",
        &format!("<p>{}</p>", escape(why)),
    )
}

/// The sign-in page, whole: it loads nothing, and no other site may frame it.
fn page(status: StatusCode, title: &str, body: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{title} · Libri</title><style>{STYLE}</style></head>\
         <body><main><h1>{title}</h1>{body}</main></body></html>",
        title = escape(title),
    );
    let mut r = (status, Html(html)).into_response();
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'",
        ),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    r
}

const STYLE: &str = "\
:root{color-scheme:light dark;--paper:#e6d8c1;--ink:#2a2019;--mid:#554537;--hair:#d2c2a8;--block:#dacab0;--frame:#bea98a}\
@media(prefers-color-scheme:dark){:root{--paper:#171411;--ink:#ebe3d5;--mid:#b0a390;--hair:#38322b;--block:#2e2924;--frame:#564c41}}\
body{margin:0;background:var(--paper);color:var(--ink);font:16px/1.5 system-ui,sans-serif}\
main{max-width:30rem;margin:0 auto;padding:2.5rem 1.25rem}h1{font-size:1.35rem;font-weight:600;margin:0 0 1rem}\
label{display:block;padding:.6rem .75rem;border:1px solid var(--frame);border-radius:.6rem;margin:.5rem 0}\
label small{display:block;color:var(--mid);font-size:.85rem}fieldset{border:0;padding:0;margin:1rem 0}\
legend{color:var(--mid);font-size:.9rem;padding:0}input[type=text]{display:block;width:100%;box-sizing:border-box;\
margin-top:.4rem;padding:.45rem .6rem;border:1px solid var(--hair);border-radius:.45rem;background:var(--block);color:inherit;font:inherit}\
.note{color:var(--mid);font-size:.9rem}.act{display:flex;gap:.75rem;justify-content:flex-end;margin-top:1.25rem}\
button{font:inherit;padding:.55rem 1.2rem;border-radius:.6rem;border:1px solid var(--hair);background:var(--block);color:inherit}\
button.go{background:var(--ink);color:var(--paper);border-color:var(--ink)}";

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_are_sent_back_only_where_they_may_be() {
        assert!(returnable("https://claude.ai/api/mcp/auth_callback"));
        assert!(returnable("http://127.0.0.1:33418/callback"));
        assert!(returnable("http://localhost/callback"));
        assert!(returnable(
            "cursor://anysphere.cursor-retrieval/oauth/callback"
        ));
        assert!(!returnable("http://evil.example/callback"));
        assert!(!returnable("javascript:alert(1)"));
        assert!(!returnable("https://claude.ai/cb#frag"));
        assert!(same_return(
            "http://127.0.0.1:1/cb",
            "http://127.0.0.1:2/cb"
        ));
        assert!(!same_return(
            "http://127.0.0.1:1/cb",
            "http://127.0.0.1:2/other"
        ));
        assert!(!same_return("https://a.example/cb", "https://b.example/cb"));
    }

    #[test]
    fn a_resource_is_this_server_or_its_host() {
        let ours = "http://box:8951/mcp";
        assert!(same_resource("http://box:8951/mcp", ours));
        assert!(same_resource("HTTP://BOX:8951/mcp/", ours));
        assert!(same_resource("http://box:8951", ours));
        assert!(!same_resource("http://other:8951/mcp", ours));
    }

    #[test]
    fn a_revoked_connector_lets_nothing_in() {
        let dir = std::env::temp_dir().join(format!("libri-connectors-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (c, key) = add(&dir, " Phone app ", &[Module::Workers, Module::Memory]).unwrap();
        assert_eq!(c.name, "Phone app");
        assert_eq!(c.modules, vec![Module::Memory, Module::Workers]);
        let g = grant(&dir, &key).unwrap();
        assert!(g.may(Module::Memory) && !g.may(Module::Chats));
        set_modules(&dir, &c.id, &[Module::Chats]).unwrap();
        assert!(grant(&dir, &key).unwrap().may(Module::Chats));
        let app = Client {
            id: "libri-x".into(),
            name: "X".into(),
            redirect_uris: vec![],
            made: 0,
        };
        let t = issue(&dir, &c.id, &app, None).unwrap();
        assert!(grant(&dir, &t.access_token).is_some());
        assert!(
            grant(&dir, &t.refresh_token).is_none(),
            "a refresh token is no access token"
        );
        revoke(&dir, &c.id).unwrap();
        assert!(grant(&dir, &key).is_none());
        assert!(grant(&dir, &t.access_token).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
