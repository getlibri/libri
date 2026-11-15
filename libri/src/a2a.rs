//! A2A, the Agent2Agent protocol (a2a-protocol.org, version 1.0): how this
//! assistant reaches other agents and is reached by them. Where a token is
//! configured it serves its Agent Card at `/.well-known/agent-card.json` and
//! takes `SendMessage` over JSON-RPC at `/a2a`. A message taken lands in
//! the assistant's own chat as a note (`inbox`), from whoever sent it.
//!
//! Each caller is a peer (peers.rs) and presents its pair's own bearer token,
//! which also names it. Only `SendMessage` is served; the answer comes back
//! later as a message of its own, as a note does. Pairing itself goes over
//! three small calls beside it: `/a2a/pair`, `/a2a/pair/approve` and
//! `/a2a/unpair`.

use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Where the JSON-RPC endpoint is, under each peer's address.
pub const PATH: &str = "/a2a";
/// Where a request to pair goes.
pub const PAIR: &str = "/a2a/pair";
/// Where the approval of a request goes back.
pub const APPROVE: &str = "/a2a/pair/approve";
/// Where a revoked pair is told.
pub const UNPAIR: &str = "/a2a/unpair";
/// Where the Agent Card is.
pub const CARD: &str = "/.well-known/agent-card.json";
const VERSION: &str = "1.0";
/// The Agent Card extension naming the person an assistant works for, so a
/// peer can be addressed by them ("Ada") as well as by its own name.
pub const PERSON: &str = "urn:libri:person";
/// The most a message may carry, in bytes.
pub const MAX: usize = 64 * 1024;

/// The peers the old settings named (LIBRI_A2A, `name=url,...`), read only
/// to bring a pair set up that way into peers.rs.
pub fn peers() -> Vec<(String, String)> {
    env::var("LIBRI_A2A")
        .unwrap_or_default()
        .split(',')
        .filter_map(|p| {
            let (name, url) = p.trim().split_once('=')?;
            Some((
                name.trim().to_owned(),
                url.trim().trim_end_matches('/').to_owned(),
            ))
        })
        .collect()
}

/// Sends `text` to each paired peer named, as this assistant (LIBRI_NAME),
/// each with its pair's token. All are found before any is sent.
pub fn share(state: &Path, names: &[String], text: &str) -> Result<(), String> {
    crate::peers::check(state, names)?;
    let me = env::var("LIBRI_NAME").map_err(|_| "LIBRI_NAME is not set")?;
    for n in names {
        let Some(p) = crate::peers::paired(state, n)? else {
            return Err(format!("{n} is no longer paired."));
        };
        send(&p.url, &p.token, &me, text)?;
        crate::peers::sent(state, &p.token)?;
    }
    Ok(())
}

/// The old shared token, from the file LIBRI_A2A_TOKEN, read only to bring
/// a pair set up that way into peers.rs.
pub fn token() -> Option<Result<String, String>> {
    let path = env::var_os("LIBRI_A2A_TOKEN")?;
    Some(read_token(Path::new(&path)))
}

fn read_token(path: &Path) -> Result<String, String> {
    match fs::read_to_string(path) {
        Ok(t) if !t.trim().is_empty() => Ok(t.trim().to_owned()),
        Ok(_) => Err(format!("a2a token: {} is empty", path.display())),
        Err(e) => Err(format!("a2a token: {}: {e}", path.display())),
    }
}

/// Where messages taken wait for the assistant's next tick.
pub fn inbox(state: &Path) -> PathBuf {
    state.join("inbox")
}

/// Leaves `text` from `from` in `dir`, named by the time, whole before it
/// is seen (a name starting with a dot is a write still in progress).
pub fn leave(dir: &Path, from: &str, text: &str) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    // The sender's name, kept to letters and digits, so it can't make a path.
    let tag: String = from
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(32)
        .collect();
    let name = format!("{ns}-{tag}");
    let tmp = dir.join(format!(".{name}"));
    fs::write(&tmp, format!("From {from}: {}", text.trim()))?;
    fs::rename(&tmp, dir.join(&name))
}

/// The person this assistant works for (LIBRI_PERSON), if set; the empty
/// string where it isn't, as pairing carries it.
pub fn own_person() -> String {
    env::var("LIBRI_PERSON")
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// This assistant's Agent Card; it names `person` where there is one.
pub fn card(name: &str, person: &str, url: &str) -> Value {
    let extensions: Vec<Value> = match person.is_empty() {
        true => vec![],
        false => vec![json!({
            "uri": PERSON,
            "description": "The person this assistant works for.",
            "params": { "person": person },
        })],
    };
    let whose = match person.is_empty() {
        true => "a person",
        false => person,
    };
    json!({
        "name": name,
        "description": format!("{whose}'s assistant."),
        "version": env!("CARGO_PKG_VERSION"),
        "supportedInterfaces": [{
            "url": format!("{url}{PATH}"),
            "protocolBinding": "JSONRPC",
            "protocolVersion": VERSION,
        }],
        "capabilities": {
            "streaming": false,
            "pushNotifications": false,
            "extensions": extensions,
        },
        "securitySchemes": { "bearer": { "httpAuthSecurityScheme": { "scheme": "Bearer" } } },
        "securityRequirements": [{ "schemes": { "bearer": { "list": [] } } }],
        "defaultInputModes": ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": [{
            "id": "message",
            "name": "Message",
            "description": format!("Takes a message for {name}, who answers in a message of its own."),
            "tags": ["message"],
        }],
    })
}

/// The person named in an Agent Card, if it names one.
pub fn person_in(card: &Value) -> Option<String> {
    card["capabilities"]["extensions"]
        .as_array()?
        .iter()
        .find(|e| e["uri"] == PERSON)?["params"]["person"]
        .as_str()
        .map(str::trim)
        .filter(|p| !p.is_empty() && p.len() <= 64)
        .map(str::to_owned)
}

/// The person the assistant at `url` works for, from its Agent Card; None
/// if it can't be reached or its card names nobody.
pub fn person_of(url: &str) -> Option<String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .new_agent();
    let card: Value = agent
        .get(&format!("{url}{CARD}"))
        .call()
        .ok()?
        .body_mut()
        .read_json()
        .ok()?;
    person_in(&card)
}

/// Sends `text` from `from` (this assistant's name) to the peer at `url`.
pub fn send(url: &str, token: &str, from: &str, text: &str) -> Result<(), String> {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "SendMessage",
        "params": {
            "message": {
                "messageId": format!("{from}-{ns}"),
                "role": "ROLE_USER",
                "parts": [{ "text": text }],
                "metadata": { "from": from },
            },
        },
    });
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut r = agent
        .post(&format!("{url}{PATH}"))
        .header("Authorization", &format!("Bearer {token}"))
        .header("A2A-Version", VERSION)
        .send_json(&body)
        .map_err(|e| format!("{url}: {e}"))?;
    let status = r.status();
    let answer: Value = r
        .body_mut()
        .read_json()
        .map_err(|e| format!("{url}: {status}: {e}"))?;
    match answer.get("error") {
        Some(e) => Err(format!(
            "{url}: {}",
            e["message"].as_str().unwrap_or("refused")
        )),
        None if status.is_success() => Ok(()),
        None => Err(format!("{url}: {status}")),
    }
}

/// Posts `body` to `path` under the peer at `url`, with the pair's token
/// where there is one; Ok with the answer's JSON on a 2xx.
fn post(url: &str, path: &str, token: Option<&str>, body: &Value) -> Result<Value, String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut req = agent.post(&format!("{url}{path}"));
    if let Some(t) = token {
        req = req.header("Authorization", &format!("Bearer {t}"));
    }
    let mut r = req.send_json(body).map_err(|e| format!("{url}: {e}"))?;
    let status = r.status();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    if !status.is_success() {
        let why = text.trim();
        return Err(match why.is_empty() {
            true => format!("{url}: {status}"),
            false => format!("{url}: {why}"),
        });
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// Asks the assistant at `url` to pair, giving this one's name and its
/// person's, the address to call it back at, and the pair's token.
pub fn request(url: &str, name: &str, person: &str, at: &str, token: &str) -> Result<(), String> {
    post(
        url,
        PAIR,
        None,
        &json!({ "name": name, "person": person, "url": at, "token": token }),
    )
    .map(|_| ())
}

/// Tells the assistant at `url` that its request is approved, as `name`,
/// working for `person`.
pub fn approve(url: &str, token: &str, name: &str, person: &str) -> Result<(), String> {
    post(
        url,
        APPROVE,
        Some(token),
        &json!({ "name": name, "person": person }),
    )
    .map(|_| ())
}

/// Tells the assistant at `url` that the pair is revoked.
pub fn unpair(url: &str, token: &str) -> Result<(), String> {
    post(url, UNPAIR, Some(token), &json!({})).map(|_| ())
}

/// What a request asks: a message's sender and words, or the JSON-RPC
/// error to answer with.
pub fn take(request: &Value) -> Result<(String, String), Value> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let error = |code: i64, message: &str| json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } });
    match request["method"].as_str() {
        Some("SendMessage") => {}
        // UnsupportedOperationError: the rest of A2A is not served here.
        Some(_) => return Err(error(-32004, "Only SendMessage is served here.")),
        None => return Err(error(-32600, "Not a JSON-RPC request.")),
    }
    let message = &request["params"]["message"];
    let text: String = message["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    // Who sent it is the pair whose token came with it; metadata.from is
    // only what the sender calls itself.
    let from = message["metadata"]["from"].as_str().unwrap_or_default();
    if text.trim().is_empty() {
        return Err(error(-32602, "A message needs text."));
    }
    if text.len() > MAX {
        return Err(error(-32602, "The message is too long."));
    }
    Ok((from.to_owned(), text))
}

/// The answer to a message taken: a message saying it was delivered.
pub fn taken(request: &Value, name: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request.get("id").cloned().unwrap_or(Value::Null),
        "result": {
            "message": {
                "messageId": format!("{}-taken", request["params"]["message"]["messageId"].as_str().unwrap_or("message")),
                "role": "ROLE_AGENT",
                "parts": [{ "text": format!("Delivered to {name}, who answers in a message of its own.") }],
            },
        },
    })
}

/// An error JSON-RPC answer to a request that couldn't be delivered.
pub fn failed(request: &Value, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request.get("id").cloned().unwrap_or(Value::Null),
        "error": { "code": -32603, "message": message },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_a_message_and_refuses_the_rest() {
        let ok = json!({"jsonrpc": "2.0", "id": 7, "method": "SendMessage", "params": {"message": {
            "messageId": "m", "role": "ROLE_USER", "parts": [{"text": "hi"}], "metadata": {"from": "kit"}}}});
        assert_eq!(take(&ok).unwrap(), ("kit".into(), "hi".into()));
        assert_eq!(taken(&ok, "Iris")["id"], 7);
        let other = json!({"jsonrpc": "2.0", "id": 1, "method": "GetTask", "params": {}});
        assert_eq!(take(&other).unwrap_err()["error"]["code"], -32004);
        let empty = json!({"jsonrpc": "2.0", "id": 1, "method": "SendMessage", "params": {"message": {"parts": [{"text": " "}]}}});
        assert!(take(&empty).is_err());
    }

    #[test]
    fn the_card_names_the_person() {
        let c = card("Kit", "Ada", "http://127.0.0.1:1");
        assert_eq!(person_in(&c).as_deref(), Some("Ada"));
        assert_eq!(person_in(&json!({"capabilities": {}})), None);
        assert_eq!(person_in(&card("Kit", "", "http://127.0.0.1:1")), None);
    }

    #[test]
    fn a_message_left_names_its_sender_and_no_path() {
        let dir = std::env::temp_dir().join(format!("a2a-inbox-{}", std::process::id()));
        leave(&dir, "../Kit", " hi ").unwrap();
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1);
        let name = names[0].to_string_lossy().into_owned();
        assert!(name.ends_with("-Kit"), "{name}");
        assert_eq!(
            fs::read_to_string(dir.join(&name)).unwrap(),
            "From ../Kit: hi"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
