//! A watch's look: one read-only MCP tool of any server the deployment has,
//! called on the watch's timer with no model and no tokens. The result is a
//! list of items, each with a key that says whether it has been seen.
//!
//! Read-only is the server's own word (`readOnlyHint`), checked on every
//! look, not only when the watch is set: a tool that stops saying so stops
//! being looked at. Libri's own server is reached in the quiet scope, whose
//! list holds only what reads.

use crate::servers::{self, Scope, Servers};
use crate::watch::field;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::{Peer, RoleClient, RunningService};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{ClientHandler, ServiceExt, serve_client};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

/// How long a look may take, starting its server included.
const LIMIT: Duration = Duration::from_secs(60);

/// The server of the look's tool that is Libri's own.
const OWN: &str = "libri";

/// One tool to call and where its result holds the items.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Look {
    /// The server, as your tools are named: libri, a connection's name, or
    /// one the deployment adds.
    pub server: String,
    /// A tool of that server, which must be read-only.
    pub tool: String,
    /// Its arguments. Keep them to a window of recent items, such as
    /// `after:2d`: an item is remembered for 60 days and an older one in
    /// the result would fire again.
    #[serde(default)]
    pub arguments: Map<String, Value>,
    /// Where the result holds the list, as a dotted path; leave out for a
    /// result that is the list.
    #[serde(default)]
    pub items: Option<String>,
    /// The field of an item that names it for good, usually `id`.
    pub key: String,
}

/// An item as a look found it: its key and everything it holds.
pub type Found = (String, Value);

/// Calls the look's tool, once. Blocks nothing, and ends within `LIMIT`.
pub async fn items(look: &Look) -> Result<Vec<Found>, String> {
    tokio::time::timeout(LIMIT, ask(look))
        .await
        .map_err(|_| format!("{} gave no answer in {LIMIT:?}", look.server))?
}

async fn ask(look: &Look) -> Result<Vec<Found>, String> {
    let servers = reachable()?;
    let server = servers
        .get(&look.server)
        .ok_or_else(|| format!("There is no server {} here.", look.server))?;
    let client = connect(server).await?;
    let out = read(client.peer(), look).await;
    let _ = client.cancel().await;
    out
}

/// The servers a look may use: what a worker's turn has, with Libri's own
/// narrowed to what reads.
fn reachable() -> Result<Servers, String> {
    let mut all = servers::list(Scope::Agent, None)?;
    if let Some(env) = all.get_mut(OWN).and_then(|s| s["env"].as_object_mut()) {
        env.insert(Scope::VAR.into(), Scope::Quiet.name().into());
    }
    Ok(all)
}

/// rmcp's own client panics on a host with no system certificates, so a look
/// brings the roots the rest of Libri uses. Its other settings are kept: no
/// idle connections, and no redirect that would replay a server's headers.
pub(crate) fn http_client() -> reqwest::Result<reqwest::Client> {
    client(reqwest::redirect::Policy::none())
}

/// The same for OAuth, whose discovery follows a server's redirects within
/// its origin (rmcp checks the origin).
pub(crate) fn http_client_following() -> reqwest::Result<reqwest::Client> {
    client(reqwest::redirect::Policy::limited(5))
}

fn client(redirect: reqwest::redirect::Policy) -> reqwest::Result<reqwest::Client> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let tls = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .pool_max_idle_per_host(0)
        .redirect(redirect)
        .build()
}

async fn connect(server: &Value) -> Result<RunningService<RoleClient, ()>, String> {
    connect_as(server, ()).await
}

/// Connects to a server as `handler`, which says what the client offers:
/// a look offers nothing, the cards' client the UI extension.
pub async fn connect_as<H: ClientHandler>(
    server: &Value,
    handler: H,
) -> Result<RunningService<RoleClient, H>, String> {
    let failed = |e: String| format!("connecting: {e}");
    if let Some(url) = server["url"].as_str() {
        let headers = server["headers"].as_object().into_iter().flatten();
        let custom_headers: HashMap<_, _> = headers
            .filter_map(|(k, v)| Some((k.parse().ok()?, v.as_str()?.parse().ok()?)))
            .collect();
        let config =
            StreamableHttpClientTransportConfig::with_uri(url).custom_headers(custom_headers);
        let client = http_client().map_err(|e| failed(e.to_string()))?;
        let transport = StreamableHttpClientTransport::with_client(client, config);
        return handler
            .serve(transport)
            .await
            .map_err(|e| failed(e.to_string()));
    }
    let program = server["command"]
        .as_str()
        .ok_or("a server with no command")?;
    let mut command = tokio::process::Command::new(program);
    for arg in server["args"].as_array().into_iter().flatten() {
        command.arg(arg.as_str().unwrap_or_default());
    }
    for (k, v) in server["env"].as_object().into_iter().flatten() {
        command.env(k, v.as_str().unwrap_or_default());
    }
    let (transport, _) = TokioChildProcess::builder(command)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| failed(e.to_string()))?;
    serve_client(handler, transport)
        .await
        .map_err(|e| failed(e.to_string()))
}

/// Calls the tool on a server already connected.
pub async fn read(peer: &Peer<RoleClient>, look: &Look) -> Result<Vec<Found>, String> {
    let tools = peer.list_all_tools().await.map_err(|e| e.to_string())?;
    let tool = tools
        .iter()
        .find(|t| t.name == look.tool)
        .ok_or_else(|| format!("{} has no tool {}.", look.server, look.tool))?;
    let reads = tool.annotations.as_ref().and_then(|a| a.read_only_hint);
    if look.server != OWN && reads != Some(true) {
        return Err(format!(
            "{} does not mark {} as read-only.",
            look.server, look.tool
        ));
    }
    let call = CallToolRequestParams::new(look.tool.clone()).with_arguments(look.arguments.clone());
    let result = peer.call_tool(call).await.map_err(|e| e.to_string())?;
    found(look, &result)
}

/// A result's items. Anything but a list is a failure, never no news.
fn found(look: &Look, result: &CallToolResult) -> Result<Vec<Found>, String> {
    let text = || {
        let text = result.content.iter().find_map(|c| c.as_text())?;
        Some(text.text.as_str())
    };
    if result.is_error == Some(true) {
        return Err(text().unwrap_or("the tool failed").to_owned());
    }
    let whole = match &result.structured_content {
        Some(v) => v.clone(),
        None => {
            let text = text().ok_or("the tool answered no text")?;
            serde_json::from_str(text).map_err(|_| "the tool's answer is not JSON".to_owned())?
        }
    };
    let list = match &look.items {
        Some(path) => field(&whole, path),
        None => Some(&whole),
    };
    list.and_then(Value::as_array)
        .ok_or_else(|| "the tool's answer holds no list".to_owned())?
        .iter()
        .map(|item| {
            let key =
                field(item, &look.key).ok_or_else(|| format!("an item has no {}", look.key))?;
            Ok((crate::watch::shown(key), item.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use rmcp::ServerHandler;
    use rmcp::handler::server::wrapper::Parameters;
    use rmcp::model::ContentBlock;
    use rmcp::{tool, tool_handler, tool_router};
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn look(tool: &str) -> Look {
        Look {
            server: "mail".into(),
            tool: tool.into(),
            arguments: Map::new(),
            items: Some("results".into()),
            key: "id".into(),
        }
    }

    #[derive(Clone)]
    struct Stand {
        calls: Arc<AtomicU32>,
        tool_router: rmcp::handler::server::router::tool::ToolRouter<Self>,
    }

    #[derive(Deserialize, JsonSchema)]
    struct Nothing {}

    #[tool_router]
    impl Stand {
        #[tool(description = "Reads.", annotations(read_only_hint = true))]
        fn listing(&self, _: Parameters<Nothing>) -> Result<String, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(r#"{"results": [{"id": "a", "subject": "one"}], "hasMore": false}"#.into())
        }

        #[tool(description = "Says nothing of what it does.")]
        fn unmarked(&self, _: Parameters<Nothing>) -> Result<String, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok("[]".into())
        }

        #[tool(description = "Reads, badly.", annotations(read_only_hint = true))]
        fn prose(&self, _: Parameters<Nothing>) -> Result<String, String> {
            Ok("You have 3 new messages.".into())
        }

        #[tool(description = "Fails.", annotations(read_only_hint = true))]
        fn broken(&self, _: Parameters<Nothing>) -> Result<String, String> {
            Err("the mailbox is down".into())
        }
    }

    #[tool_handler(router = self.tool_router)]
    impl ServerHandler for Stand {}

    /// A client joined to a stand-in server by a pipe, as a look's would be
    /// to a child process.
    async fn stand() -> (RunningService<RoleClient, ()>, Arc<AtomicU32>) {
        let (ours, theirs) = tokio::io::duplex(65536);
        let calls = Arc::new(AtomicU32::new(0));
        let server = Stand {
            calls: calls.clone(),
            tool_router: Stand::tool_router(),
        };
        tokio::spawn(async move {
            if let Ok(s) = server.serve(theirs).await {
                let _ = s.waiting().await;
            }
        });
        (().serve(ours).await.unwrap(), calls)
    }

    /// A server reached by address, one JSON answer to each message, as one
    /// with nothing to stream gives.
    async fn far(headers: HeaderMap, Json(m): Json<Value>) -> Response {
        if headers.get("authorization").is_none_or(|h| h != "Bearer b") {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let Some(id) = m.get("id") else {
            return StatusCode::ACCEPTED.into_response();
        };
        let result = match m["method"].as_str() {
            Some("initialize") => json!({
                "protocolVersion": m["params"]["protocolVersion"],
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "far", "version": "1"},
            }),
            Some("tools/list") => json!({"tools": [{
                "name": "listing",
                "inputSchema": {"type": "object"},
                "annotations": {"readOnlyHint": true},
            }]}),
            _ => json!({"content": [{"type": "text", "text": r#"{"results": [{"id": "a"}]}"#}]}),
        };
        Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
    }

    #[tokio::test]
    async fn a_look_reaches_a_server_by_its_address_and_headers() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let router = axum::Router::new().route("/mcp", axum::routing::post(far));
        tokio::spawn(async move { axum::serve(listener, router).await });
        let server = json!({"type": "http", "url": url, "headers": {"Authorization": "Bearer b"}});
        let client = connect(&server).await.unwrap();
        let got = read(client.peer(), &look("listing")).await.unwrap();
        assert_eq!(got[0].0, "a");
    }

    #[tokio::test]
    async fn a_look_reads_the_list_with_each_key() {
        let (client, calls) = stand().await;
        let got = read(client.peer(), &look("listing")).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "a");
        assert_eq!(got[0].1["subject"], "one");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_tool_not_marked_read_only_is_never_called() {
        let (client, calls) = stand().await;
        let e = read(client.peer(), &look("unmarked")).await.unwrap_err();
        assert!(e.contains("read-only"), "{e}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn an_answer_that_is_no_list_is_a_failure() {
        let (client, _) = stand().await;
        assert!(read(client.peer(), &look("prose")).await.is_err());
        let mut wrong = look("listing");
        wrong.items = Some("hasMore".into());
        assert!(read(client.peer(), &wrong).await.is_err());
        let e = read(client.peer(), &look("broken")).await.unwrap_err();
        assert!(e.contains("mailbox is down"), "{e}");
    }

    #[test]
    fn an_item_without_its_key_is_a_failure() {
        let mut l = look("x");
        l.key = "uid".into();
        let r = CallToolResult::success(vec![ContentBlock::text(r#"{"results": [{"id": 1}]}"#)]);
        assert!(found(&l, &r).is_err());
        l.key = "id".into();
        assert_eq!(found(&l, &r).unwrap()[0].0, "1");
    }
}
