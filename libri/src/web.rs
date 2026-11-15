//! Libri's web: search and fetch through Parallel's MCP server, keyless,
//! or through the host's key proxy for the higher rate limit. The proxy is a
//! Unix socket that adds the account key itself, so Libri and its workers
//! never hold it. Where the host's proxy also serves Exa or Brave, search
//! offers them as other engines: one tool with an optional engine, as a
//! search tool with several backends usually is, since the caller wants an
//! answer, not a choice of vendor. A failed engine passes the search to the
//! next. Fetch stays on Parallel.
//! Search passes through. Fetch opens only links that a search in this same
//! process returned, or that the person wrote themselves: a link the model
//! made up could carry what it has read (mail, memory) out in its path or
//! query, but a link that existed before it read anything carries nothing.
//! A scratchpad's words are not in hippo, so its thread is read for them;
//! with the memory off that is all that is read.
//! Pages can still say anything; what they say is information, not
//! instruction.

use crate::hippo::Hippo;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use ureq::Agent;

const PARALLEL: &str = "https://search.parallel.ai/mcp";
/// Results asked of Exa and Brave, near what Parallel returns.
const COUNT: usize = 8;
const ACCEPT: &str = "application/json, text/event-stream";
const TIMEOUT: Duration = Duration::from_secs(90);
/// Waits before retrying a rate-limited call; short, since a person is
/// waiting on the reply.
const BACKOFF: [Duration; 2] = [Duration::from_secs(3), Duration::from_secs(10)];
/// What the model hears when the limit holds.
const LIMITED: &str = "Web search is rate-limited right now. Answer from what you know, \
or say it couldn't be checked.";

/// The scratchpad a fetch comes from.
pub struct Thread {
    /// What the person wrote there.
    pub words: Vec<String>,
    /// Whether its run has the memory on.
    pub memory: bool,
}

/// A search engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Parallel,
    Exa,
    Brave,
}

impl Engine {
    const ALL: [Engine; 3] = [Engine::Parallel, Engine::Exa, Engine::Brave];

    pub fn name(self) -> &'static str {
        match self {
            Engine::Parallel => "parallel",
            Engine::Exa => "exa",
            Engine::Brave => "brave",
        }
    }

    /// What it is good for, as the model reads it.
    fn strength(self) -> &'static str {
        match self {
            Engine::Parallel => "the default; excerpts aimed at the objective",
            Engine::Exa => {
                "search by meaning; finds pages like a description, papers, companies, people"
            }
            Engine::Brave => "an independent keyword index; fresh news, exact phrases",
        }
    }

    fn parse(name: &str) -> Result<Engine, String> {
        Engine::ALL
            .into_iter()
            .find(|e| e.name() == name.trim().to_lowercase())
            .ok_or_else(|| format!("no search engine {name}"))
    }
}

/// The host's key-proxy sockets, one per engine. Parallel works without
/// one, keyless; Exa and Brave exist only with theirs.
#[derive(Clone, Debug, Default)]
pub struct Sockets {
    pub parallel: Option<PathBuf>,
    pub exa: Option<PathBuf>,
    pub brave: Option<PathBuf>,
}

impl Sockets {
    /// From LIBRI_SEARCH_SOCKET, LIBRI_EXA_SOCKET and LIBRI_BRAVE_SOCKET.
    pub fn from_env() -> Self {
        let var = |name| {
            std::env::var_os(name)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        };
        Sockets {
            parallel: var("LIBRI_SEARCH_SOCKET"),
            exa: var("LIBRI_EXA_SOCKET"),
            brave: var("LIBRI_BRAVE_SOCKET"),
        }
    }

    fn of(&self, engine: Engine) -> Option<&PathBuf> {
        match engine {
            Engine::Parallel => self.parallel.as_ref(),
            Engine::Exa => self.exa.as_ref(),
            Engine::Brave => self.brave.as_ref(),
        }
    }

    /// The engines search can use here, Parallel first.
    pub fn engines(&self) -> Vec<Engine> {
        Engine::ALL
            .into_iter()
            .filter(|&e| e == Engine::Parallel || self.of(e).is_some())
            .collect()
    }
}

/// Fits web_search's `engine` argument to the engines this host has: gone
/// with only Parallel, else an enum of those there are, so an engine
/// without its proxy is never offered.
pub fn offer<S>(
    router: &mut rmcp::handler::server::router::tool::ToolRouter<S>,
    sockets: &Sockets,
) {
    let Some(route) = router.map.get_mut("web_search") else {
        return;
    };
    let mut schema = (*route.attr.input_schema).clone();
    let engines = sockets.engines();
    if let Some(Value::Object(props)) = schema.get_mut("properties") {
        if engines.len() < 2 {
            props.remove("engine");
        } else {
            let names: Vec<_> = engines.iter().map(|e| e.name()).collect();
            let told: Vec<_> = engines
                .iter()
                .map(|e| format!("{}: {}", e.name(), e.strength()))
                .collect();
            props.insert(
                "engine".into(),
                json!({
                    "type": "string",
                    "enum": names,
                    "description": format!(
                        "Which engine; another answers if it fails. {}.",
                        told.join("; ")
                    ),
                }),
            );
        }
    }
    route.attr.input_schema = std::sync::Arc::new(schema);
}

pub struct Web {
    hippo: Hippo,
    agent: Agent,
    /// The key proxy's sockets. Parallel's serves its `/mcp` with the key
    /// added; without it, calls go to Parallel keyless.
    sockets: Sockets,
    /// Links the searches of this process returned, as `key` compares them.
    seen: Mutex<HashSet<String>>,
}

impl Web {
    pub fn new(hippo: Hippo, sockets: Sockets) -> Self {
        let config = Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .build();
        Web {
            hippo,
            agent: Agent::new_with_config(config),
            sockets,
            seen: Mutex::new(HashSet::new()),
        }
    }

    /// One tool call on Parallel's server, retried while rate-limited.
    fn call(&self, tool: &str, args: Value) -> Result<String, String> {
        let mut waits = BACKOFF.iter();
        loop {
            match self.post(tool, &args) {
                Err(Fail::Limited) => match waits.next() {
                    Some(wait) => thread::sleep(*wait),
                    None => return Err(LIMITED.into()),
                },
                Err(Fail::Other(e)) => return Err(e),
                Ok(text) => return Ok(text),
            }
        }
    }

    /// One JSON-RPC post (it answers plain JSON and needs no session), its
    /// text content joined.
    fn post(&self, tool: &str, args: &Value) -> Result<String, Fail> {
        let body = json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": tool, "arguments": args},
        });
        let (status, raw) = match &self.sockets.parallel {
            Some(socket) => through(socket, &Ask::parallel(&body)),
            None => self.direct(&body),
        }
        .map_err(|e| Fail::Other(format!("parallel: {e}")))?;
        if status == 429 {
            return Err(Fail::Limited);
        }
        if status >= 400 {
            return Err(Fail::Other(format!("parallel: {status}")));
        }
        let reply: Value =
            serde_json::from_slice(&raw).map_err(|e| Fail::Other(format!("parallel: {e}")))?;
        content(&reply).map_err(|e| {
            if limited(&e) {
                Fail::Limited
            } else {
                Fail::Other(e)
            }
        })
    }

    /// A post straight to Parallel: its status and body.
    fn direct(&self, body: &Value) -> Result<(u16, Vec<u8>), String> {
        let mut resp = self
            .agent
            .post(PARALLEL)
            .header("Accept", ACCEPT)
            .send_json(body)
            .map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let raw = resp.body_mut().read_to_vec().map_err(|e| e.to_string())?;
        Ok((status, raw))
    }

    /// A search on the engine asked for, or Parallel; on failure the next
    /// engine here tries, and a round where each was only rate-limited
    /// waits as one engine's retries do.
    pub fn search(
        &self,
        objective: &str,
        queries: &[String],
        engine: Option<&str>,
    ) -> Result<String, String> {
        let first = engine.map(Engine::parse).transpose()?;
        let mut order = self.sockets.engines();
        if let Some(first) = first {
            if !order.contains(&first) {
                return Err(format!("the {} engine isn't set up here", first.name()));
            }
            order.retain(|&e| e != first);
            order.insert(0, first);
        }
        let mut waits = BACKOFF.iter();
        let text = 'found: loop {
            let mut errors = Vec::new();
            let mut limited = true;
            for &engine in &order {
                match self.once(engine, objective, queries) {
                    Ok(text) => break 'found text,
                    Err(Fail::Limited) => errors.push(format!("{}: rate-limited", engine.name())),
                    Err(Fail::Other(e)) => {
                        limited = false;
                        errors.push(e);
                    }
                }
            }
            match waits.next() {
                Some(wait) if limited => thread::sleep(*wait),
                _ if limited => return Err(LIMITED.into()),
                _ => return Err(errors.join("; ")),
            }
        };
        if let Ok(found) = serde_json::from_str::<Value>(&text) {
            let mut seen = self.seen.lock().unwrap();
            for url in results(&found) {
                seen.insert(key(url));
            }
        }
        Ok(text)
    }

    /// One search on one engine, without retries. Exa takes the objective,
    /// which it reads by meaning; Brave the first query, as one call of
    /// its free plan's one a second.
    fn once(&self, engine: Engine, objective: &str, queries: &[String]) -> Result<String, Fail> {
        if engine == Engine::Parallel {
            return self.post(
                "web_search",
                &json!({"objective": objective, "search_queries": queries}),
            );
        }
        let name = engine.name();
        let socket = self
            .sockets
            .of(engine)
            .ok_or_else(|| Fail::Other(format!("{name}: no key proxy")))?;
        let ask = match engine {
            Engine::Exa => Ask::exa(objective),
            _ => Ask::brave(queries.first().map_or(objective, String::as_str)),
        };
        let (status, raw) =
            through(socket, &ask).map_err(|e| Fail::Other(format!("{name}: {e}")))?;
        if status == 429 {
            return Err(Fail::Limited);
        }
        if status >= 400 {
            return Err(Fail::Other(format!("{name}: {status}")));
        }
        let reply: Value =
            serde_json::from_slice(&raw).map_err(|e| Fail::Other(format!("{name}: {e}")))?;
        let found = match engine {
            Engine::Exa => from_exa(&reply),
            _ => from_brave(&reply),
        };
        Ok(found.to_string())
    }

    pub fn fetch(
        &self,
        urls: &[String],
        objective: Option<&str>,
        thread: Option<Thread>,
    ) -> Result<String, String> {
        let mut refused = Vec::new();
        for url in urls {
            let seen = self.seen.lock().unwrap().contains(&key(url));
            if !seen && !self.written(url, thread.as_ref()) {
                refused.push(url.as_str());
            }
        }
        if !refused.is_empty() {
            return Ok(format!(
                "Refused, not from a search here or from the person: {}",
                refused.join(", ")
            ));
        }
        let mut args = json!({"urls": urls});
        if let Some(objective) = objective {
            args["objective"] = objective.into();
        }
        self.call("web_fetch", args)
    }

    /// Whether the person wrote this link in a message of theirs.
    fn written(&self, url: &str, thread: Option<&Thread>) -> bool {
        let url = url.trim();
        thread.is_some_and(|t| t.words.iter().any(|w| w.contains(url)))
            || thread.is_none_or(|t| t.memory)
                && self
                    .hippo
                    .search(&literal(url))
                    .is_ok_and(|out| wrote(&out))
    }
}

/// A request for the key proxy: the proxy sets the host and key itself.
struct Ask {
    method: &'static str,
    target: String,
    host: &'static str,
    accept: &'static str,
    body: Option<String>,
}

impl Ask {
    fn parallel(body: &Value) -> Ask {
        Ask {
            method: "POST",
            target: "/mcp".into(),
            host: "search.parallel.ai",
            accept: ACCEPT,
            body: Some(body.to_string()),
        }
    }

    fn exa(query: &str) -> Ask {
        let body = json!({
            "query": query, "numResults": COUNT,
            "contents": {"highlights": {"numSentences": 3, "highlightsPerUrl": 3}},
        });
        Ask {
            method: "POST",
            target: "/search".into(),
            host: "api.exa.ai",
            accept: "application/json",
            body: Some(body.to_string()),
        }
    }

    fn brave(query: &str) -> Ask {
        Ask {
            method: "GET",
            target: format!("/res/v1/web/search?q={}&count={COUNT}", escape(query)),
            host: "api.search.brave.com",
            accept: "application/json",
            body: None,
        }
    }
}

/// A query-string value, percent-encoded.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Exa's results in Parallel's shape, so fetch and the model read one kind.
fn from_exa(reply: &Value) -> Value {
    let results: Vec<Value> = reply["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            json!({
                "url": r["url"], "title": r["title"],
                "publish_date": r["publishedDate"], "excerpts": r["highlights"],
            })
        })
        .collect();
    json!({"engine": "exa", "results": results})
}

/// Brave's web results in Parallel's shape.
fn from_brave(reply: &Value) -> Value {
    let results: Vec<Value> = reply["web"]["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let mut excerpts = vec![r["description"].clone()];
            excerpts.extend(
                r["extra_snippets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            json!({
                "url": r["url"], "title": r["title"],
                "publish_date": r["page_age"], "excerpts": excerpts,
            })
        })
        .collect();
    json!({"engine": "brave", "results": results})
}

/// A request through a key proxy's socket: its status and body. HTTP/1.0,
/// so nginx sends no chunking and the answer ends where the connection does.
fn through(socket: &std::path::Path, ask: &Ask) -> Result<(u16, Vec<u8>), String> {
    let mut s = UnixStream::connect(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    s.set_read_timeout(Some(TIMEOUT))
        .map_err(|e| e.to_string())?;
    s.set_write_timeout(Some(TIMEOUT))
        .map_err(|e| e.to_string())?;
    let body = ask.body.as_deref().unwrap_or("");
    let mut head = format!(
        "{} {} HTTP/1.0\r\nHost: {}\r\nAccept: {}\r\n",
        ask.method, ask.target, ask.host, ask.accept
    );
    if ask.body.is_some() {
        head.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes())
        .and_then(|()| s.write_all(body.as_bytes()))
        .map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    answer(raw)
}

/// An HTTP/1.0 answer's status and body.
fn answer(mut raw: Vec<u8>) -> Result<(u16, Vec<u8>), String> {
    let end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("no answer")?;
    let head = String::from_utf8_lossy(&raw[..end]);
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("bad status line: {}", head.lines().next().unwrap_or("")))?;
    Ok((status, raw.split_off(end + 4)))
}

enum Fail {
    Limited,
    Other(String),
}

/// Whether an error is Parallel's rate limit, which it reports as a
/// JSON-RPC error rather than a 429.
fn limited(error: &str) -> bool {
    error.to_lowercase().contains("rate limit")
}

/// A tool reply's text, or its error.
fn content(reply: &Value) -> Result<String, String> {
    if let Some(msg) = reply["error"]["message"].as_str() {
        return Err(format!("parallel: {msg}"));
    }
    let result = &reply["result"];
    let text = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| c["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if result["isError"].as_bool().unwrap_or(false) {
        return Err(format!("parallel: {text}"));
    }
    Ok(text)
}

/// The links a search result lists.
fn results(found: &Value) -> impl Iterator<Item = &str> {
    found["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["url"].as_str())
}

/// A link as compared: scheme and host lowercased, no fragment or trailing
/// slash.
fn key(url: &str) -> String {
    let url = url.trim().split('#').next().unwrap_or("");
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let (host, tail) = rest
        .find(['/', '?'])
        .map_or((rest, ""), |i| rest.split_at(i));
    let (path, query) = tail
        .split_once('?')
        .map_or((tail, None), |(p, q)| (p, Some(q)));
    let mut key = format!(
        "{}://{}{}",
        scheme.to_lowercase(),
        host.to_lowercase(),
        path.trim_end_matches('/')
    );
    if let Some(query) = query {
        key.push('?');
        key.push_str(query);
    }
    key
}

/// A regex matching the text exactly.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for c in text.chars() {
        if r"\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Whether a hippo search's lines (`id kind ...`) include one of the person's.
fn wrote(out: &str) -> bool {
    out.lines()
        .filter(|l| l.starts_with(|c: char| c.is_ascii_digit()))
        .any(|l| l.split(' ').nth(1) == Some("user"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_compares_without_case_fragment_or_trailing_slash() {
        assert_eq!(
            key(" HTTPS://Example.com/a/b/#top "),
            "https://example.com/a/b"
        );
        assert_eq!(
            key("https://example.com/a?x=1&y=2"),
            "https://example.com/a?x=1&y=2"
        );
        assert_eq!(key("https://example.com/"), "https://example.com");
        assert_eq!(key("https://example.com?q"), "https://example.com?q");
    }

    #[test]
    fn a_link_written_in_the_scratchpad_counts_without_the_memory() {
        let hippo = Hippo {
            sock: "/nonexistent".into(),
            chat: None,
            spool: None,
        };
        let web = Web::new(hippo, Sockets::default());
        let thread = Thread {
            words: vec!["read https://a.b/c please".into()],
            memory: false,
        };
        assert!(web.written(" https://a.b/c ", Some(&thread)));
        assert!(!web.written("https://a.b/d", Some(&thread)));
    }

    #[test]
    fn a_literal_escapes_every_regex_special() {
        assert_eq!(
            literal("https://a.b/c?d=1+2&e=(f)"),
            r"https://a\.b/c\?d=1\+2\&e=\(f\)"
        );
    }

    #[test]
    fn only_the_persons_own_lines_count_as_written() {
        assert!(wrote("12 user [ada] 2026-10-11 12:00: see https://a.b"));
        assert!(!wrote("12 talk [iris] 2026-10-11 12:00: see https://a.b"));
        assert!(!wrote("no matches\n"));
    }

    #[test]
    fn a_reply_yields_its_text_or_its_error() {
        let ok = json!({"result": {"content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}});
        assert_eq!(content(&ok).unwrap(), "a\nb");
        let bad = json!({"result": {"isError": true, "content": [{"type": "text", "text": "no"}]}});
        assert_eq!(content(&bad).unwrap_err(), "parallel: no");
        let err = json!({"error": {"code": -32600, "message": "bad"}});
        assert_eq!(content(&err).unwrap_err(), "parallel: bad");
        let limit = json!({"error": {"code": -32000, "message": "You've hit the free-tier rate limit for Parallel Search MCP."}});
        assert!(limited(&content(&limit).unwrap_err()));
        assert!(!limited("parallel: bad"));
    }

    /// A key proxy that answers one post with `answer`, and the request it got.
    fn proxy(dir: &std::path::Path, answer: &'static str) -> (PathBuf, thread::JoinHandle<String>) {
        named(dir, "parallel", answer)
    }

    /// A key proxy at `<name>.sock` that answers one request.
    fn named(
        dir: &std::path::Path,
        name: &str,
        answer: &'static str,
    ) -> (PathBuf, thread::JoinHandle<String>) {
        let path = dir.join(format!("{name}.sock"));
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let got = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut raw = Vec::new();
            let mut buf = [0; 4096];
            // Read the head, then as much body as it declares.
            loop {
                let n = s.read(&mut buf).unwrap();
                raw.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&raw).to_string();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len: usize = head
                        .lines()
                        .find_map(|l| l.strip_prefix("Content-Length: "))
                        .map_or(0, |n| n.parse().unwrap());
                    if body.len() >= len {
                        break;
                    }
                }
            }
            s.write_all(answer.as_bytes()).unwrap();
            String::from_utf8(raw).unwrap()
        });
        (path, got)
    }

    /// A fresh directory for one test's socket.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("libri-web-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn web(socket: PathBuf) -> Web {
        let hippo = Hippo {
            sock: "/nonexistent".into(),
            chat: None,
            spool: None,
        };
        Web::new(
            hippo,
            Sockets {
                parallel: Some(socket),
                ..Sockets::default()
            },
        )
    }

    #[test]
    fn a_call_through_the_proxy_carries_no_key() {
        let dir = scratch("no_key");
        let (socket, got) = proxy(
            &dir,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n\
             {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"found\"}]}}",
        );
        let text = web(socket).post("web_search", &json!({"objective": "x"}));
        assert_eq!(text.ok().as_deref(), Some("found"));
        let req = got.join().unwrap();
        assert!(req.starts_with("POST /mcp HTTP/1.0\r\n"), "{req}");
        assert!(req.contains("\r\nHost: search.parallel.ai\r\n"), "{req}");
        assert!(!req.to_lowercase().contains("authorization"), "{req}");
        let body: Value = serde_json::from_str(req.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["params"]["name"], "web_search");
    }

    #[test]
    fn the_proxys_429_is_the_rate_limit() {
        let dir = scratch("limit");
        let (socket, got) = proxy(&dir, "HTTP/1.1 429 Too Many Requests\r\n\r\n");
        assert!(matches!(
            web(socket).post("web_search", &json!({})),
            Err(Fail::Limited)
        ));
        got.join().unwrap();
    }

    #[test]
    fn the_proxys_refusal_is_an_error() {
        let dir = scratch("refused");
        let (socket, got) = proxy(&dir, "HTTP/1.1 403 Forbidden\r\n\r\nno");
        match web(socket).post("web_search", &json!({})) {
            Err(Fail::Other(e)) => assert_eq!(e, "parallel: 403"),
            _ => panic!("a 403 passed"),
        }
        got.join().unwrap();
    }

    #[test]
    fn a_missing_proxy_is_an_error() {
        let dir = scratch("missing");
        let e = web(dir.join("none.sock")).call("web_search", json!({}));
        assert!(e.unwrap_err().starts_with("parallel: "));
    }

    fn hippo() -> Hippo {
        Hippo {
            sock: "/nonexistent".into(),
            chat: None,
            spool: None,
        }
    }

    const EXA: &str = "HTTP/1.1 200 OK\r\n\r\n{\"results\":[{\"url\":\"https://e.x/a\",\
        \"title\":\"A\",\"publishedDate\":\"2026-11-01\",\"highlights\":[\"one\"]}]}";
    const BRAVE: &str = "HTTP/1.1 200 OK\r\n\r\n{\"web\":{\"results\":[{\"url\":\"https://b.r/a\",\
        \"title\":\"B\",\"description\":\"two\",\"extra_snippets\":[\"three\"]}]}}";

    #[test]
    fn exa_is_asked_the_objective_and_answers_in_parallels_shape() {
        let dir = scratch("exa");
        let (exa, got) = named(&dir, "exa", EXA);
        let (parallel, _) = named(&dir, "parallel", "HTTP/1.1 500 X\r\n\r\n");
        let sockets = Sockets {
            parallel: Some(parallel),
            exa: Some(exa),
            brave: None,
        };
        let web = Web::new(hippo(), sockets);
        let text = web
            .search("who makes x", &["x maker".into()], Some("exa"))
            .unwrap();
        let found: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(found["engine"], "exa");
        assert_eq!(found["results"][0]["excerpts"][0], "one");
        let req = got.join().unwrap();
        assert!(req.starts_with("POST /search HTTP/1.0\r\n"), "{req}");
        assert!(req.contains("\r\nHost: api.exa.ai\r\n"), "{req}");
        assert!(!req.to_lowercase().contains("x-api-key"), "{req}");
        let body: Value = serde_json::from_str(req.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["query"], "who makes x");
        // Its links may now be fetched.
        assert!(web.seen.lock().unwrap().contains("https://e.x/a"));
    }

    #[test]
    fn brave_gets_the_first_query_escaped() {
        let dir = scratch("brave");
        let (brave, got) = named(&dir, "brave", BRAVE);
        let sockets = Sockets {
            brave: Some(brave),
            ..Sockets::default()
        };
        let web = Web::new(hippo(), sockets);
        let text = web
            .search("o", &["rust & c++".into(), "unused".into()], Some("brave"))
            .unwrap();
        let found: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(found["engine"], "brave");
        assert_eq!(found["results"][0]["excerpts"], json!(["two", "three"]));
        let req = got.join().unwrap();
        assert!(
            req.starts_with("GET /res/v1/web/search?q=rust%20%26%20c%2B%2B&count=8 HTTP/1.0\r\n"),
            "{req}"
        );
        assert!(!req.contains("Content-Length"), "{req}");
    }

    #[test]
    fn a_failed_engine_passes_the_search_on() {
        let dir = scratch("fallback");
        let (brave, got_brave) = named(&dir, "brave", "HTTP/1.1 502 Bad Gateway\r\n\r\n");
        let (exa, _) = named(&dir, "exa", EXA);
        let (parallel, _) = named(&dir, "parallel", "HTTP/1.1 503 X\r\n\r\n");
        let sockets = Sockets {
            parallel: Some(parallel),
            exa: Some(exa),
            brave: Some(brave),
        };
        let text = Web::new(hippo(), sockets)
            .search("o", &["q".into()], Some("brave"))
            .unwrap();
        got_brave.join().unwrap();
        // Brave, then Parallel, then Exa.
        assert!(text.contains("\"engine\":\"exa\""), "{text}");
    }

    #[test]
    fn an_engine_not_set_up_is_refused() {
        let web = Web::new(hippo(), Sockets::default());
        let e = web.search("o", &[], Some("exa")).unwrap_err();
        assert!(e.contains("isn't set up"), "{e}");
        assert!(web.search("o", &[], Some("bing")).is_err());
    }

    #[test]
    fn search_results_list_their_links() {
        let found = json!({"results": [{"url": "https://a.b"}, {"title": "none"}]});
        assert_eq!(results(&found).collect::<Vec<_>>(), ["https://a.b"]);
    }
}
