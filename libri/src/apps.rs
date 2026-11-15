//! MCP Apps (the `io.modelcontextprotocol/ui` extension, stable
//! 2026-01-26): a tool that names a `ui://` resource gets drawn as a card
//! beside its call, in a sandboxed frame the page hosts.
//!
//! Harnesses render no UI and declare no extension, so their models get
//! the tool's text. Libri opens sessions of its own to the same servers,
//! declaring the extension: from them it learns which tools carry a card,
//! reads the cards' HTML, and runs what a card asks (its tool calls and
//! resource reads), never through the harness. The page matches a call in
//! the turn's stream to its card by the name the harness gave the call.
//!
//! A card shows by itself only when showing it is the point: its tool says
//! so (`OPENS`) and the person's own turn made the call, or the model asked
//! (`show_card`, tools.rs). A lookup the model makes for itself is its row
//! alone, whose details open the card on asking.
//!
//! A card's HTML runs in the spec's double frame: the page frames this
//! host's sandbox proxy, which frames the card. The proxy is served here
//! with its CSP as a header, as the reference host serves it, and is
//! sandboxed into an opaque origin on top: the reference proxy has an
//! origin of its own, and the only one Libri has to give is this host's,
//! whose session cookie would let a card act as the person.

use crate::look::connect_as;
use crate::servers::{self, Scope};
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation,
    ReadResourceRequestParams,
};
use rmcp::service::{RoleClient, RunningService};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The extension's identifier, as a client declares it.
pub const EXTENSION: &str = "io.modelcontextprotocol/ui";

/// The only content type the extension defines so far.
pub const MIME: &str = "text/html;profile=mcp-app";

/// How long a server's tool list and a card's HTML are taken as current.
const FRESH: Duration = Duration::from_secs(300);

/// How long one request to a server may take, connecting included.
const LIMIT: Duration = Duration::from_secs(30);

/// A tool that carries a card, as the page needs it to draw one.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Card {
    pub server: String,
    pub tool: String,
    /// The `ui://` resource that holds the card.
    pub uri: String,
    /// The tool as the server listed it, for the card's host context.
    pub definition: Value,
    /// Whether the card opens by itself under each call the person's turn
    /// makes: a tool whose card is the point of it says so (`OPENS`). Any
    /// other is a lookup the model makes for itself, which shows no card
    /// unless the model asks (`show_card`) or the person opens it.
    pub opens: bool,
}

/// The `_meta` key by which a tool says its card is the point of a call
/// (a camera, a dashboard), not a view of what the model looked up. MCP
/// Apps (2026-01-26) has no such field; a server written for Libri sets it.
pub const OPENS: &str = "is.libri/opens";

/// One server's session, and its tools as last listed.
struct Held {
    client: RunningService<RoleClient, ClientConfig>,
    tools: Mutex<(Instant, Vec<Value>)>,
}

/// Libri's own sessions to the servers a turn of the person's has.
#[derive(Default)]
pub struct Apps {
    held: tokio::sync::Mutex<HashMap<String, Arc<Held>>>,
    /// Cards' HTML and settings as read, by server and URI.
    views: Mutex<HashMap<(String, String), (Instant, Value)>>,
}

/// What Libri says it is and offers on connecting: the UI extension, for
/// HTML cards.
fn info() -> ClientConfig {
    let mut capabilities = ClientCapabilities::default();
    let mut settings = Map::new();
    settings.insert("mimeTypes".into(), json!([MIME]));
    capabilities.extensions = Some(BTreeMap::from([(EXTENSION.to_owned(), settings)]));
    ClientConfig::new(
        capabilities,
        Implementation::new("libri", env!("CARGO_PKG_VERSION")),
    )
}

/// The card a tool names, from its `_meta.ui.resourceUri` (or the flat
/// `ui/resourceUri` the spec still reads, deprecated).
pub fn resource_uri(tool: &Value) -> Option<&str> {
    let meta = &tool["_meta"];
    meta["ui"]["resourceUri"]
        .as_str()
        .or_else(|| meta["ui/resourceUri"].as_str())
        .filter(|u| u.starts_with("ui://"))
}

/// Whether a tool says its card is the point of a call (`OPENS`).
pub fn opens(tool: &Value) -> bool {
    tool["_meta"][OPENS] == Value::Bool(true)
}

/// Whether a tool's visibility holds `who` ("model" or "app"); both by
/// default.
pub fn visible_to(tool: &Value, who: &str) -> bool {
    match tool["_meta"]["ui"]["visibility"].as_array() {
        Some(v) => v.iter().any(|w| w == who),
        None => true,
    }
}

/// The names a harness gives a call of `tool` on `server`: Claude Code,
/// Codex and ACP's `mcp__server__tool`, OpenCode's `server_tool`, and
/// Pi's `mcp__server__tool` with the server's name held to `[A-Za-z0-9_]`.
pub fn call_names(server: &str, tool: &str) -> Vec<String> {
    let plain: String = server
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut names = vec![format!("mcp__{server}__{tool}"), format!("{server}_{tool}")];
    if plain != server {
        names.push(format!("mcp__{plain}__{tool}"));
    }
    names
}

impl Apps {
    /// The session to `server`, opened if there is none.
    async fn session(&self, server: &str) -> Result<Arc<Held>, String> {
        if let Some(h) = self.held.lock().await.get(server) {
            return Ok(h.clone());
        }
        // A card's call is the person's own press: its relay asks nobody.
        let all = servers::list_asking(Scope::Person, None, Some("person"))?;
        let config = all
            .get(server)
            .ok_or_else(|| format!("There is no server {server} here."))?;
        let client = tokio::time::timeout(LIMIT, connect_as(config, info()))
            .await
            .map_err(|_| format!("{server} gave no answer in {LIMIT:?}"))??;
        let held = Arc::new(Held {
            client,
            tools: Mutex::new((Instant::now() - FRESH * 2, vec![])),
        });
        self.held
            .lock()
            .await
            .insert(server.to_owned(), held.clone());
        Ok(held)
    }

    /// Forgets a session that failed, so the next request opens a new one.
    async fn drop_session(&self, server: &str) {
        if let Some(h) = self.held.lock().await.remove(server) {
            h.client.cancellation_token().cancel();
        }
    }

    /// `server`'s tools, listed again when older than FRESH.
    async fn tools(&self, server: &str) -> Result<Vec<Value>, String> {
        let held = self.session(server).await?;
        {
            let t = held.tools.lock().unwrap();
            if t.0.elapsed() < FRESH {
                return Ok(t.1.clone());
            }
        }
        let listed = tokio::time::timeout(LIMIT, held.client.peer().list_all_tools()).await;
        let tools = match listed {
            Ok(Ok(tools)) => tools
                .into_iter()
                .filter_map(|t| serde_json::to_value(t).ok())
                .collect::<Vec<_>>(),
            Ok(Err(e)) => {
                self.drop_session(server).await;
                return Err(e.to_string());
            }
            Err(_) => {
                self.drop_session(server).await;
                return Err(format!("{server} gave no tools in {LIMIT:?}"));
            }
        };
        *held.tools.lock().unwrap() = (Instant::now(), tools.clone());
        Ok(tools)
    }

    /// Every tool with a card the model may call, by each name a harness
    /// gives its calls. A server that fails is left out, not the rest.
    pub async fn cards(&self) -> Result<HashMap<String, Card>, String> {
        let names: Vec<String> = servers::list(Scope::Person, None)?
            .keys()
            .cloned()
            .collect();
        let listed =
            futures_util::future::join_all(names.iter().map(|n| self.tools(n.as_str()))).await;
        let mut out = HashMap::new();
        for (server, tools) in names.iter().zip(listed) {
            let tools = match tools {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("libri: apps: {server}: {e}");
                    continue;
                }
            };
            for t in tools.iter().filter(|t| visible_to(t, "model")) {
                let (Some(uri), Some(tool)) = (resource_uri(t), t["name"].as_str()) else {
                    continue;
                };
                let card = Card {
                    server: server.clone(),
                    tool: tool.to_owned(),
                    uri: uri.to_owned(),
                    definition: t.clone(),
                    opens: opens(t),
                };
                for name in call_names(server, tool) {
                    out.insert(name, card.clone());
                }
            }
        }
        Ok(out)
    }

    /// The card at `uri` on `server`: its HTML and what the server asks of
    /// its frame (`csp`, `permissions`, `prefersBorder`).
    pub async fn view(&self, server: &str, uri: &str) -> Result<Value, String> {
        let key = (server.to_owned(), uri.to_owned());
        if let Some((at, v)) = self.views.lock().unwrap().get(&key)
            && at.elapsed() < FRESH
        {
            return Ok(v.clone());
        }
        let read = self.read(server, uri).await?;
        let view = view_of(&read, uri)?;
        self.views
            .lock()
            .unwrap()
            .insert(key, (Instant::now(), view.clone()));
        Ok(view)
    }

    /// A resource on `server`, as the server gave it.
    pub async fn read(&self, server: &str, uri: &str) -> Result<Value, String> {
        let held = self.session(server).await?;
        let params = ReadResourceRequestParams::new(uri);
        match tokio::time::timeout(LIMIT, held.client.peer().read_resource(params)).await {
            Ok(Ok(r)) => serde_json::to_value(r).map_err(|e| e.to_string()),
            Ok(Err(e)) => {
                self.drop_session(server).await;
                Err(e.to_string())
            }
            Err(_) => Err(format!("{server} gave no {uri} in {LIMIT:?}")),
        }
    }

    /// `tool`'s definition on `server`, as the server lists it.
    pub async fn definition(&self, server: &str, tool: &str) -> Result<Value, String> {
        self.tools(server)
            .await?
            .into_iter()
            .find(|t| t["name"] == tool)
            .ok_or_else(|| format!("{server} has no tool {tool}."))
    }

    /// A card's call of `tool` on its own server. Only a tool the server
    /// lets apps call: the spec holds a card to its server, and the page
    /// sends each card's calls to the server the card came from.
    pub async fn call(
        &self,
        server: &str,
        tool: &str,
        arguments: Map<String, Value>,
    ) -> Result<Value, String> {
        let tools = self.tools(server).await?;
        let found = tools
            .iter()
            .find(|t| t["name"] == tool)
            .ok_or_else(|| format!("{server} has no tool {tool}."))?;
        if !visible_to(found, "app") {
            return Err(format!("{server} does not let a card call {tool}."));
        }
        let held = self.session(server).await?;
        let params = CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments);
        match tokio::time::timeout(LIMIT, held.client.peer().call_tool(params)).await {
            Ok(Ok(r)) => serde_json::to_value(r).map_err(|e| e.to_string()),
            Ok(Err(e)) => {
                self.drop_session(server).await;
                Err(e.to_string())
            }
            Err(_) => Err(format!("{server} did not finish {tool} in {LIMIT:?}")),
        }
    }
}

/// A `resources/read` result as a card: the HTML content and its `_meta.ui`.
fn view_of(read: &Value, uri: &str) -> Result<Value, String> {
    let content = read["contents"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|c| {
            c["mimeType"]
                .as_str()
                .is_some_and(|m| m.replace(' ', "") == MIME)
        })
        .ok_or_else(|| format!("{uri} holds no {MIME}"))?;
    let html = match (content["text"].as_str(), content["blob"].as_str()) {
        (Some(t), _) => t.to_owned(),
        (None, Some(b)) => {
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b)
                .map_err(|e| format!("{uri}: {e}"))?;
            String::from_utf8(bytes).map_err(|e| format!("{uri}: {e}"))?
        }
        (None, None) => return Err(format!("{uri} is empty")),
    };
    let ui = &content["_meta"]["ui"];
    Ok(json!({
        "html": html,
        "csp": ui["csp"],
        "permissions": ui["permissions"],
        "prefersBorder": ui["prefersBorder"],
    }))
}

/// A server's domains for one directive: those that can't break out of
/// it (no `;`, quote, space or line end), less this host's own origin.
fn domains(list: &Value, own: &[String]) -> Vec<String> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|d| !d.contains([';', '\'', '"', ' ', '\r', '\n', ',']))
        .map(|d| d.trim_end_matches('/'))
        .filter(|d| !own.iter().any(|o| o == d))
        .map(str::to_owned)
        .collect()
}

/// The sandbox proxy's CSP for a card's declared `csp`: the reference
/// host's directives, with no `'self'`, which here would be this host,
/// and the frame sandboxed to an opaque origin. `own` are the origins this
/// host answers at, struck from what a server declares. `fonts` is the
/// origin the host's own fonts come from, which the host context's
/// `styles.css.fonts` names; it serves only public build assets.
pub fn policy(csp: &Value, own: &[String], fonts: Option<&str>) -> String {
    let joined = |k: &str| domains(&csp[k], own).join(" ");
    let res = joined("resourceDomains");
    let fonts = fonts
        .map(|f| domains(&json!([f]), &[]).join(" "))
        .unwrap_or_default();
    let line = |head: &str, tail: &str| format!("{head} {tail}").trim().to_owned();
    let frames = joined("frameDomains");
    let base = joined("baseUriDomains");
    let connect = joined("connectDomains");
    [
        "sandbox allow-scripts allow-forms".to_owned(),
        "default-src 'none'".to_owned(),
        line("script-src 'unsafe-inline' 'unsafe-eval' blob: data:", &res),
        line("style-src 'unsafe-inline' blob: data:", &res),
        line("img-src data: blob:", &res),
        line(
            "font-src data: blob:",
            [res.as_str(), fonts.as_str()].join(" ").trim(),
        ),
        line("media-src data: blob:", &res),
        if connect.is_empty() {
            "connect-src 'none'".to_owned()
        } else {
            format!("connect-src {connect}")
        },
        line("worker-src blob:", &res),
        if frames.is_empty() {
            "frame-src 'none'".to_owned()
        } else {
            format!("frame-src {frames}")
        },
        "object-src 'none'".to_owned(),
        if base.is_empty() {
            "base-uri 'none'".to_owned()
        } else {
            format!("base-uri {base}")
        },
    ]
    .join("; ")
}

/// The sandbox proxy: relays the spec's JSON-RPC between the page and the
/// card, and loads the card when the page sends it. It takes messages only
/// from its parent and its own frame, and keeps the `ui/notifications/
/// sandbox-*` pair to itself, as the spec has it.
pub const SANDBOX: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><meta name="color-scheme" content="light dark">
<style>html,body{height:100%;margin:0;padding:0;background:transparent;overflow:hidden}iframe{display:block;width:100%;height:100%;border:0}</style>
</head><body><script>
(() => {
  if (window.self === window.top) return;
  const inner = document.createElement("iframe");
  inner.setAttribute("sandbox", "allow-scripts allow-same-origin allow-forms");
  document.body.appendChild(inner);
  const ALLOW = { camera: "camera", microphone: "microphone", geolocation: "geolocation", clipboardWrite: "clipboard-write" };
  window.addEventListener("message", (event) => {
    const data = event.data;
    if (event.source === window.parent) {
      if (data && data.method === "ui/notifications/sandbox-resource-ready") {
        const p = data.params || {};
        const allow = Object.keys(p.permissions || {}).map((k) => ALLOW[k]).filter(Boolean).join("; ");
        if (allow) inner.setAttribute("allow", allow);
        if (typeof p.html === "string") {
          const doc = inner.contentDocument;
          if (doc) { doc.open(); doc.write(p.html); doc.close(); } else { inner.srcdoc = p.html; }
        }
      } else if (inner.contentWindow) {
        inner.contentWindow.postMessage(data, "*");
      }
    } else if (event.source === inner.contentWindow) {
      if (data && typeof data.method === "string" && data.method.startsWith("ui/notifications/sandbox-")) return;
      window.parent.postMessage(data, "*");
    }
  });
  window.parent.postMessage({ jsonrpc: "2.0", method: "ui/notifications/sandbox-proxy-ready", params: {} }, "*");
})();
</script></body></html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_opens_by_itself_only_when_its_tool_says_so() {
        let shown = json!({"_meta": {"ui": {"resourceUri": "ui://h/c"}, "is.libri/opens": true}});
        let lookup = json!({"_meta": {"ui": {"resourceUri": "ui://f/m"}}});
        let odd = json!({"_meta": {"is.libri/opens": "yes"}});
        assert!(opens(&shown));
        assert!(!opens(&lookup) && !opens(&odd));
    }

    #[test]
    fn a_tool_names_its_card_by_either_form() {
        let nested = json!({"name": "a", "_meta": {"ui": {"resourceUri": "ui://s/a"}}});
        let flat = json!({"name": "b", "_meta": {"ui/resourceUri": "ui://s/b"}});
        let web = json!({"name": "c", "_meta": {"ui": {"resourceUri": "https://x"}}});
        assert_eq!(resource_uri(&nested), Some("ui://s/a"));
        assert_eq!(resource_uri(&flat), Some("ui://s/b"));
        assert_eq!(resource_uri(&web), None);
        assert_eq!(resource_uri(&json!({"name": "d"})), None);
    }

    #[test]
    fn visibility_defaults_to_both() {
        let app = json!({"_meta": {"ui": {"visibility": ["app"]}}});
        assert!(visible_to(&json!({}), "model") && visible_to(&json!({}), "app"));
        assert!(!visible_to(&app, "model"));
        assert!(visible_to(&app, "app"));
    }

    #[test]
    fn a_view_reads_text_or_blob() {
        let read = json!({"contents": [{"uri": "ui://s/a", "mimeType": "text/html;profile=mcp-app",
            "text": "<p>hi</p>", "_meta": {"ui": {"prefersBorder": true}}}]});
        let v = view_of(&read, "ui://s/a").unwrap();
        assert_eq!(v["html"], "<p>hi</p>");
        assert_eq!(v["prefersBorder"], true);
        let blob = json!({"contents": [{"uri": "u", "mimeType": "text/html;profile=mcp-app", "blob": "PHA+aGk8L3A+"}]});
        assert_eq!(view_of(&blob, "u").unwrap()["html"], "<p>hi</p>");
        let plain = json!({"contents": [{"uri": "u", "mimeType": "text/plain", "text": "x"}]});
        assert!(view_of(&plain, "u").is_err());
    }

    #[test]
    fn the_policy_is_restrictive_by_default() {
        let p = policy(&Value::Null, &[], None);
        assert!(p.starts_with("sandbox allow-scripts allow-forms; default-src 'none'"));
        assert!(p.contains("connect-src 'none'"));
        assert!(p.contains("frame-src 'none'"));
        assert!(p.contains("object-src 'none'"));
        assert!(!p.contains("'self'"));
    }

    #[test]
    fn the_policy_takes_clean_domains_and_never_this_host() {
        let csp = json!({
            "connectDomains": ["https://home.example.com", "https://libri.example.com/", "https://x; script-src *"],
            "resourceDomains": ["https://cdn.example.com", "'unsafe-hashes'"],
        });
        let p = policy(&csp, &["https://libri.example.com".into()], None);
        assert!(p.contains("connect-src https://home.example.com;"));
        assert!(!p.contains("libri.example.com"));
        assert!(!p.contains("script-src *"));
        assert!(p.contains("img-src data: blob: https://cdn.example.com"));
        assert!(!p.contains("unsafe-hashes"));
        // The host's fonts, and only as fonts.
        let p = policy(
            &csp,
            &["https://libri.example.com".into()],
            Some("https://libri.example.com"),
        );
        assert!(
            p.contains("font-src data: blob: https://cdn.example.com https://libri.example.com;")
        );
        assert!(p.contains("connect-src https://home.example.com;"));
        let bad = policy(&Value::Null, &[], Some("https://x; script-src *"));
        assert!(!bad.contains("script-src *"));
    }

    #[test]
    fn calls_go_by_each_harness_name() {
        assert_eq!(
            call_names("home", "camera"),
            ["mcp__home__camera".to_owned(), "home_camera".to_owned()]
        );
        // Pi's, seen on a live turn: a hyphen goes to an underscore.
        assert_eq!(call_names("card-test", "show")[2], "mcp__card_test__show");
    }
}
