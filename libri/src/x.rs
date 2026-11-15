//! xAI, connected in Settings > Connections: Grok's `x_search` on the
//! Responses API, text to speech, the short-lived passes a page holds to
//! talk with Grok Voice, and pictures from Grok Imagine. Grok runs the
//! searches and answers with the posts it found and their links. It reads
//! X and nothing else, so it acts on nothing.
//!
//! The connection is the Grok CLI's device-code sign-in, paid by the
//! person's SuperGrok or X Premium+ plan, as Hermes Agent signs in; an API
//! key from console.x.ai is the way in for anyone else. xAI documents
//! voice for API keys only, so its subscription login may stop working.

use crate::host::{private_dir, write_private};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, File};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tungstenite::client::IntoClientRequest;
use tungstenite::handshake::HandshakeError;
use tungstenite::{Message, WebSocket};
use ureq::Agent;

const AUTH: &str = "https://auth.x.ai/oauth2";
/// The Grok CLI's public client, whose sign-in the subscription covers.
const CLIENT: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
const API: &str = "https://api.x.ai/v1/responses";
const TTS_HOST: &str = "api.x.ai";
const SECRETS: &str = "https://api.x.ai/v1/realtime/client_secrets";
const VOICES: &str = "https://api.x.ai/v1/tts/voices";
const STT: &str = "https://api.x.ai/v1/stt";
const IMAGES: &str = "https://api.x.ai/v1/images/generations";
const IMAGE_MODELS: &str = "https://api.x.ai/v1/image-generation-models";
const KEY_CHECK: &str = "https://api.x.ai/v1/api-key";
/// The picture model when none is asked for, or xAI's list can't be had.
pub const IMAGE_MODEL: &str = "grok-imagine-image-2.0";
/// The longest description the image tool sends.
const DRAW_MAX: usize = 4000;
/// xAI's limit on one text to speak, in characters.
pub const SPEAK_MAX: usize = 60_000;

/// The model that searches X.
pub const MODEL: &str = "grok-4.7";
const TIMEOUT: Duration = Duration::from_secs(300);
/// Refresh the access token this long before it expires.
const MARGIN: u64 = 300;
const INSTRUCTIONS: &str = "Search X for the request below. Report what \
you found, newest first: for each post that matters, the author's handle, \
the date, what it says (quoted when short) and its link. Report, do not \
editorialize; if nothing relevant turns up, say so plainly.";

/// A search, as the tool takes it and the socket carries it.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct Ask {
    /// What to look for on X, in a sentence.
    pub query: String,
    /// Only posts from these handles (no @), at most 20.
    #[serde(default)]
    pub handles: Vec<String>,
    /// Leave out posts from these handles (no @), at most 20; not with handles.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Only posts from this day on, YYYY-MM-DD.
    pub from: Option<String>,
    /// Only posts up to this day, YYYY-MM-DD.
    pub to: Option<String>,
}

/// Whether `model` could name one of xAI's models; only these go out.
pub fn model_ok(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 64
        && model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b))
}

/// Whether `voice` could name one of xAI's voices: a built-in's name or a
/// custom voice's 8-character id. Speech asks only for these.
pub fn voice_ok(voice: &str) -> bool {
    !voice.is_empty() && voice.len() <= 32 && voice.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn agent() -> Agent {
    Agent::new_with_config(
        Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .build(),
    )
}

/// One search of X, answered with what Grok found.
pub fn ask(state: &Path, ask: &Ask) -> Result<String, String> {
    let token = token(state)?;
    let body = json!({
        "model": MODEL,
        "store": false,
        "input": [
            {"role": "system", "content": INSTRUCTIONS},
            {"role": "user", "content": request(ask)},
        ],
        "tools": [tool(ask)?],
    });
    let mut resp = agent()
        .post(API)
        .header("Authorization", &format!("Bearer {token}"))
        .send_json(&body)
        .map_err(|e| format!("xai: {e}"))?;
    let status = resp.status().as_u16();
    let reply: Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("xai: {status}: {e}"))?;
    if status >= 400 {
        let msg = reply["error"]["message"]
            .as_str()
            .or(reply["error"].as_str())
            .unwrap_or("failed");
        return Err(format!("xai: {status}: {msg}"));
    }
    let u = &reply["usage"];
    eprintln!(
        "libri: x tokens model={MODEL} input={} cache_read={} output={} tools={}",
        u["input_tokens"],
        u["input_tokens_details"]["cached_tokens"],
        u["output_tokens"],
        u["server_side_tool_usage_details"]
    );
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let cached = n(&u["input_tokens_details"]["cached_tokens"]);
    let signed_in = fs::read_to_string(file(state))
        .ok()
        .and_then(|t| serde_json::from_str::<Connection>(&t).ok())
        .is_some_and(|c| matches!(c, Connection::Grok(_)));
    crate::ledger::record(
        state,
        &crate::ledger::Entry {
            at: chrono::Local::now().timestamp(),
            by: crate::ledger::By::Search,
            harness: "xai".into(),
            provider: "xai".into(),
            model: MODEL.into(),
            input: n(&u["input_tokens"]).saturating_sub(cached),
            cache_read: cached,
            output: n(&u["output_tokens"]),
            // The Grok sign-in is the person's SuperGrok or Premium+ plan.
            paid: Some(if signed_in { "subscription" } else { "key" }.into()),
            ..Default::default()
        },
    );
    answer(&reply)
}

/// `text` spoken by xAI in `voice`, as MP3 handed to `each` a piece at a
/// time while xAI makes it, so playing can start before the end is made.
/// The streaming endpoint is a WebSocket; the plain POST answers only once
/// the whole is made, which for a long reply is a minute of silence.
pub fn speak(
    state: &Path,
    text: &str,
    voice: &str,
    mut each: impl FnMut(&[u8]),
) -> Result<(), String> {
    if text.chars().count() > SPEAK_MAX {
        return Err(format!("Too long to speak: over {SPEAK_MAX} characters."));
    }
    if !voice_ok(voice) {
        return Err(format!("xai tts: bad voice {voice:?}"));
    }
    let token = token(state)?;
    let mut ws = tts_socket(voice, &token)?;
    let say = |ws: &mut Socket, m: Value| {
        ws.send(Message::text(m.to_string()))
            .map_err(|e| format!("xai tts: {e}"))
    };
    say(&mut ws, json!({"type": "text.delta", "delta": text}))?;
    say(&mut ws, json!({"type": "text.done"}))?;
    let mut bytes = 0;
    loop {
        let event = match ws.read().map_err(|e| format!("xai tts: {e}"))? {
            Message::Text(t) => {
                serde_json::from_str::<Value>(&t).map_err(|e| format!("xai tts: {e}"))?
            }
            Message::Close(why) => return Err(format!("xai tts: closed early: {why:?}")),
            _ => continue,
        };
        match event["type"].as_str() {
            Some("audio.delta") => {
                let mp3 = BASE64
                    .decode(event["delta"].as_str().unwrap_or_default())
                    .map_err(|e| format!("xai tts: {e}"))?;
                bytes += mp3.len();
                each(&mp3);
            }
            Some("audio.done") => break,
            Some("error") => {
                let msg = event["message"].as_str().unwrap_or("unknown error");
                return Err(format!("xai tts: {msg}"));
            }
            _ => {}
        }
    }
    let _ = ws.close(None);
    eprintln!(
        "libri: x spoke chars={} voice={voice} bytes={bytes}",
        text.chars().count()
    );
    Ok(())
}

type Socket = WebSocket<StreamOwned<ClientConnection, TcpStream>>;

/// The streaming text to speech socket, MP3 in `voice`, tuned for the
/// first sound soonest.
fn tts_socket(voice: &str, token: &str) -> Result<Socket, String> {
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let conn = ClientConnection::new(Arc::new(config), TTS_HOST.try_into().unwrap())
        .map_err(|e| format!("xai tts: {e}"))?;
    let tcp = TcpStream::connect((TTS_HOST, 443)).map_err(|e| format!("xai tts: {e}"))?;
    tcp.set_read_timeout(Some(TIMEOUT))
        .and(tcp.set_write_timeout(Some(TIMEOUT)))
        .map_err(|e| format!("xai tts: {e}"))?;
    let mut ask = format!(
        "wss://{TTS_HOST}/v1/tts?voice={voice}&language=en&codec=mp3&optimize_streaming_latency=1"
    )
    .into_client_request()
    .map_err(|e| format!("xai tts: {e}"))?;
    ask.headers_mut().insert(
        "Authorization",
        format!("Bearer {token}")
            .parse()
            .map_err(|_| "xai tts: bad token")?,
    );
    let (ws, _) = tungstenite::client(ask, StreamOwned::new(conn, tcp)).map_err(|e| match e {
        HandshakeError::Failure(tungstenite::Error::Http(r)) => {
            let body = r
                .body()
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            format!("xai tts: {}: {body}", r.status())
        }
        e => format!("xai tts: {e}"),
    })?;
    Ok(ws)
}

/// A pass to Grok Voice from xAI, at most an hour long, as it gives it,
/// `{value, expires_at}`. The page opens the call with it, so the sign-in
/// itself never leaves this host.
pub fn secret(state: &Path, seconds: u64) -> Result<Value, String> {
    let token = token(state)?;
    let mut resp = agent()
        .post(SECRETS)
        .header("Authorization", &format!("Bearer {token}"))
        .send_json(json!({"expires_after": {"seconds": seconds.clamp(10, 3600)}}))
        .map_err(|e| format!("xai secret: {e}"))?;
    let status = resp.status().as_u16();
    let reply: Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("xai secret: {status}: {e}"))?;
    if status >= 400 || !reply["value"].is_string() {
        let msg = reply["error"]["message"]
            .as_str()
            .or(reply["error"].as_str())
            .unwrap_or("no pass");
        return Err(format!("xai secret: {status}: {msg}"));
    }
    eprintln!("libri: x voice pass for {seconds}s");
    Ok(json!({"value": reply["value"], "expires_at": reply["expires_at"]}))
}

/// The file name xAI is told for a recording of this type, or None for a
/// type browsers and phones don't record. A codec parameter is ignored.
pub fn audio_name(mime: &str) -> Option<&'static str> {
    Some(match mime.split(';').next()?.trim() {
        "audio/webm" => "speech.webm",
        "audio/ogg" => "speech.ogg",
        "audio/mp4" | "audio/m4a" | "audio/aac" => "speech.m4a",
        "audio/mpeg" => "speech.mp3",
        "audio/wav" => "speech.wav",
        _ => return None,
    })
}

// ureq's own multipart is still behind its unversioned API.
fn stt_form(boundary: &str, name: &str, audio: &[u8]) -> Vec<u8> {
    let mut form = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; \
         filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    form.extend_from_slice(audio);
    form.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    form
}

/// A recording written down by xAI, in whatever language it was spoken.
pub fn transcribe(state: &Path, audio: &[u8], name: &str) -> Result<String, String> {
    let token = token(state)?;
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("xai stt: {e}"))?;
    let boundary = format!("libri{:032x}", u128::from_ne_bytes(bytes));
    let mut resp = agent()
        .post(STT)
        .header("Authorization", &format!("Bearer {token}"))
        .header(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .send(&stt_form(&boundary, name, audio)[..])
        .map_err(|e| format!("xai stt: {e}"))?;
    let status = resp.status().as_u16();
    let reply: Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("xai stt: {status}: {e}"))?;
    match reply["text"].as_str() {
        Some(text) if status < 400 => Ok(text.to_owned()),
        _ => {
            let msg = reply["error"]["message"]
                .as_str()
                .or(reply["error"].as_str())
                .unwrap_or("no text");
            Err(format!("xai stt: {status}: {msg}"))
        }
    }
}

/// xAI's built-in voices as `[{id, name}]`, in its order, good for
/// reading aloud and for calls alike.
pub fn voices(state: &Path) -> Result<Value, String> {
    let token = token(state)?;
    let mut resp = agent()
        .get(VOICES)
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| format!("xai voices: {e}"))?;
    let status = resp.status().as_u16();
    let reply: Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("xai voices: {status}: {e}"))?;
    let Some(all) = reply["voices"].as_array() else {
        let msg = reply["error"]["message"]
            .as_str()
            .or(reply["error"].as_str())
            .unwrap_or("no voices");
        return Err(format!("xai voices: {status}: {msg}"));
    };
    Ok(all
        .iter()
        .filter_map(|v| {
            let id = v["voice_id"].as_str().filter(|id| voice_ok(id))?;
            let name = v["name"].as_str().unwrap_or(id);
            Some(json!({ "id": id, "name": name }))
        })
        .collect())
}

/// xAI's picture models' ids, as /v1/image-generation-models lists them.
pub fn image_models(state: &Path) -> Result<Vec<String>, String> {
    let token = token(state)?;
    let mut resp = agent()
        .get(IMAGE_MODELS)
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| format!("xai image models: {e}"))?;
    let status = resp.status().as_u16();
    let reply: Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("xai image models: {status}: {e}"))?;
    let Some(all) = reply["models"].as_array() else {
        let msg = reply["error"]["message"]
            .as_str()
            .or(reply["error"].as_str())
            .unwrap_or("no models");
        return Err(format!("xai image models: {status}: {msg}"));
    };
    Ok(all
        .iter()
        .filter_map(|m| m["id"].as_str().filter(|id| model_ok(id)))
        .map(str::to_owned)
        .collect())
}

/// One picture from Grok Imagine by `model`, as the bytes xAI sent (a JPEG).
pub fn draw(state: &Path, description: &str, model: &str) -> Result<Vec<u8>, String> {
    let model = Some(model).filter(|m| model_ok(m)).unwrap_or(IMAGE_MODEL);
    let prompt: String = description.chars().take(DRAW_MAX).collect();
    let token = token(state)?;
    let mut resp = agent()
        .post(IMAGES)
        .header("Authorization", &format!("Bearer {token}"))
        .send_json(json!({"model": model, "prompt": prompt, "n": 1,
            "response_format": "b64_json"}))
        .map_err(|e| format!("xai image: {e}"))?;
    let status = resp.status().as_u16();
    let reply: Value = resp
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_json()
        .map_err(|e| format!("xai image: {status}: {e}"))?;
    let Some(image) = reply["data"][0]["b64_json"].as_str() else {
        let msg = reply["error"]["message"]
            .as_str()
            .or(reply["error"].as_str())
            .unwrap_or("no picture");
        return Err(format!("xai image: {status}: {msg}"));
    };
    let image = BASE64
        .decode(image)
        .map_err(|e| format!("xai image: {e}"))?;
    eprintln!("libri: x drew model={model} bytes={}", image.len());
    Ok(image)
}

/// The request as Grok reads it: the tool's filters bind the search, but
/// Grok sees only the text, so the text names them too.
fn request(ask: &Ask) -> String {
    let mut text = ask.query.trim().to_owned();
    let at = |hs: &[String]| {
        hs.iter()
            .map(|h| format!("@{}", h.trim().trim_start_matches('@')))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if !ask.handles.is_empty() {
        text.push_str(&format!("\nOnly posts from {}.", at(&ask.handles)));
    }
    if !ask.exclude.is_empty() {
        text.push_str(&format!("\nNot posts from {}.", at(&ask.exclude)));
    }
    if let Some(from) = &ask.from {
        text.push_str(&format!("\nFrom {from}."));
    }
    if let Some(to) = &ask.to {
        text.push_str(&format!("\nUp to {to}."));
    }
    text
}

/// The x_search tool with the search's filters.
fn tool(ask: &Ask) -> Result<Value, String> {
    let clean = |hs: &[String]| -> Vec<String> {
        hs.iter()
            .map(|h| h.trim().trim_start_matches('@').to_owned())
            .filter(|h| !h.is_empty())
            .collect()
    };
    let (handles, exclude) = (clean(&ask.handles), clean(&ask.exclude));
    if !handles.is_empty() && !exclude.is_empty() {
        return Err("Give handles or exclude, not both.".into());
    }
    if handles.len() > 20 || exclude.len() > 20 {
        return Err("At most 20 handles.".into());
    }
    let mut tool = json!({"type": "x_search"});
    if !handles.is_empty() {
        tool["allowed_x_handles"] = json!(handles);
    }
    if !exclude.is_empty() {
        tool["excluded_x_handles"] = json!(exclude);
    }
    for (key, day) in [("from_date", &ask.from), ("to_date", &ask.to)] {
        if let Some(day) = day.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
                .map_err(|_| format!("{day}: a day is YYYY-MM-DD."))?;
            tool[key] = day.into();
        }
    }
    Ok(tool)
}

/// The answer's text, then the links it cites.
fn answer(reply: &Value) -> Result<String, String> {
    let mut text = Vec::new();
    let mut links: Vec<&str> = Vec::new();
    let parts = reply["output"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| o["type"] == "message")
        .flat_map(|o| o["content"].as_array().into_iter().flatten())
        .filter(|c| c["type"] == "output_text");
    for part in parts {
        text.extend(part["text"].as_str());
        for a in part["annotations"].as_array().into_iter().flatten() {
            if let Some(url) = a["url"].as_str()
                && !links.contains(&url)
            {
                links.push(url);
            }
        }
    }
    if text.is_empty() {
        return Err("xai: no answer".into());
    }
    let mut out = text.join("\n");
    if !links.is_empty() {
        out.push_str("\n\nSources:\n");
        out.push_str(&links.join("\n"));
    }
    Ok(out)
}

#[derive(Serialize, Deserialize)]
struct Login {
    /// The address signed in with; empty if xAI didn't say.
    #[serde(default)]
    account: String,
    access: String,
    refresh: String,
    /// Unix seconds.
    expires: u64,
}

/// How this host reaches xAI, kept in `connections/xai.json`.
#[derive(Serialize, Deserialize)]
#[serde(tag = "how", rename_all = "lowercase")]
enum Connection {
    Grok(Login),
    Key { key: String },
}

fn file(state: &Path) -> PathBuf {
    crate::connections::dir(state).join("xai.json")
}

fn keep(state: &Path, c: &Connection) -> Result<(), String> {
    private_dir(&crate::connections::dir(state))?;
    write_private(&file(state), &serde_json::to_string_pretty(c).unwrap())
}

/// What Settings shows for the connection, if there is one.
pub fn connected(state: &Path) -> Option<String> {
    let text = fs::read_to_string(file(state)).ok()?;
    Some(match serde_json::from_str::<Connection>(&text).ok()? {
        Connection::Grok(l) if l.account.is_empty() => "Grok sign-in".into(),
        Connection::Grok(l) => l.account,
        Connection::Key { .. } => "API key".into(),
    })
}

/// Forgets the connection here. xAI keeps a sign-in listed until the
/// person removes it there.
pub fn disconnect(state: &Path) -> Result<(), String> {
    match fs::remove_file(file(state)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// Connects by an API key, once xAI has accepted it.
pub fn use_key(state: &Path, key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() || !key.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("That doesn't look like an xAI key.".into());
    }
    let status = agent()
        .get(KEY_CHECK)
        .header("Authorization", &format!("Bearer {key}"))
        .call()
        .map_err(|e| format!("xai: {e}"))?
        .status();
    if !status.is_success() {
        return Err(format!("xAI refused that key ({status})."));
    }
    keep(state, &Connection::Key { key: key.into() })
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A live token: the key, or the sign-in's access token, refreshed under a
/// lock when near its end, since the refresh token may turn over and the
/// daemon and each tool process must not race.
fn token(state: &Path) -> Result<String, String> {
    let dir = crate::connections::dir(state);
    private_dir(&dir)?;
    let lock = File::create(dir.join("xai.lock")).map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    let text = fs::read_to_string(file(state))
        .map_err(|_| "xAI is not connected: connect it in Settings > Connections.".to_owned())?;
    let login = match serde_json::from_str(&text).map_err(|e| format!("xai.json: {e}"))? {
        Connection::Key { key } => return Ok(key),
        Connection::Grok(login) => login,
    };
    if login.expires > now() + MARGIN {
        return Ok(login.access);
    }
    let fresh = grant(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", &login.refresh),
    ])
    .and_then(|reply| granted(&reply, Some(&login.refresh)))
    .map_err(|e| format!("{e}; connect xAI again in Settings > Connections."))?;
    let fresh = Login {
        account: login.account,
        ..fresh
    };
    let access = fresh.access.clone();
    keep(state, &Connection::Grok(fresh))?;
    Ok(access)
}

/// Renews the sign-in if it is near its end.
pub fn renew(state: &Path) -> Result<(), String> {
    token(state).map(|_| ())
}

/// A post to the token endpoint, the client named.
fn grant(form: &[(&str, &str)]) -> Result<Value, String> {
    let mut form = form.to_vec();
    form.push(("client_id", CLIENT));
    let mut resp = agent()
        .post(&format!("{AUTH}/token"))
        .send_form(form)
        .map_err(|e| format!("xai auth: {e}"))?;
    resp.body_mut()
        .read_json()
        .map_err(|e| format!("xai auth: {e}"))
}

/// The login a token reply grants; a refresh that hands back no new
/// refresh token keeps the old one.
fn granted(reply: &Value, old: Option<&str>) -> Result<Login, String> {
    if let Some(err) = reply["error"].as_str() {
        return Err(format!("xai auth: {err}"));
    }
    let access = reply["access_token"].as_str().ok_or("xai auth: no token")?;
    let refresh = reply["refresh_token"]
        .as_str()
        .or(old)
        .ok_or("xai auth: no refresh token")?;
    Ok(Login {
        account: account(reply).unwrap_or_default(),
        access: access.to_owned(),
        refresh: refresh.to_owned(),
        expires: now() + reply["expires_in"].as_u64().unwrap_or(3600),
    })
}

/// The address in a token reply's ID token. Read for display alone, so
/// its signature goes unchecked.
fn account(reply: &Value) -> Option<String> {
    let payload = reply["id_token"].as_str()?.split('.').nth(1)?;
    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&claims).ok()?;
    claims["email"].as_str().map(str::to_owned)
}

/// A device-code sign-in started: the page to approve it on, and its code.
pub struct SignIn {
    pub url: String,
    pub code: String,
    device: String,
    wait: u64,
    end: u64,
}

/// Starts a sign-in the person approves on any device.
pub fn sign_in() -> Result<SignIn, String> {
    let mut resp = agent()
        .post(&format!("{AUTH}/device/code"))
        .send_form([("client_id", CLIENT), ("scope", SCOPE)])
        .map_err(|e| format!("xai auth: {e}"))?;
    let start: Value = resp
        .body_mut()
        .read_json()
        .map_err(|e| format!("xai auth: {e}"))?;
    let text = |k: &str| start[k].as_str().map(str::to_owned);
    Ok(SignIn {
        device: text("device_code").ok_or("xai auth: no device code")?,
        code: text("user_code").ok_or("xai auth: no user code")?,
        url: text("verification_uri_complete")
            .or(text("verification_uri"))
            .ok_or("xai auth: no page to approve on")?,
        wait: start["interval"].as_u64().unwrap_or(5),
        end: now() + start["expires_in"].as_u64().unwrap_or(900),
    })
}

/// Waits for the person to approve `sign_in`, then keeps the connection.
pub fn approved(state: &Path, sign_in: &SignIn) -> Result<(), String> {
    let mut wait = sign_in.wait;
    while now() < sign_in.end {
        std::thread::sleep(Duration::from_secs(wait));
        // A dropped connection is no answer yet: ask again.
        let Ok(reply) = grant(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", &sign_in.device),
        ]) else {
            continue;
        };
        match reply["error"].as_str() {
            Some("authorization_pending") => continue,
            Some("slow_down") => wait += 5,
            _ => return keep(state, &Connection::Grok(granted(&reply, None)?)),
        }
    }
    Err("The code expired; connect again.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(handles: &[&str], exclude: &[&str], from: Option<&str>) -> Ask {
        Ask {
            query: "q".into(),
            handles: handles.iter().map(|&h| h.into()).collect(),
            exclude: exclude.iter().map(|&h| h.into()).collect(),
            from: from.map(Into::into),
            to: None,
        }
    }

    #[test]
    fn a_recording_is_named_by_its_type_without_the_codec() {
        assert_eq!(audio_name("audio/webm;codecs=opus"), Some("speech.webm"));
        assert_eq!(audio_name("audio/mp4"), Some("speech.m4a"));
        assert_eq!(audio_name("text/plain"), None);
    }

    #[test]
    fn a_recording_goes_out_whole_between_its_boundaries() {
        let form = stt_form("b", "speech.ogg", &[0, 13, 10, 255]);
        let head = b"--b\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.ogg\"\r\nContent-Type: application/octet-stream\r\n\r\n";
        assert!(form.starts_with(head));
        assert!(form.ends_with(b"\x00\r\n\xff\r\n--b--\r\n"));
    }

    #[test]
    fn only_a_plausible_model_goes_out() {
        assert!(model_ok("grok-imagine-image-2.0"));
        assert!(!model_ok("a b"));
        assert!(!model_ok(""));
    }

    #[test]
    fn the_tool_carries_clean_handles_and_checked_days() {
        let t = tool(&ask(&["@xai", " nasa "], &[], Some("2026-10-01"))).unwrap();
        assert_eq!(
            t,
            json!({"type": "x_search", "allowed_x_handles": ["xai", "nasa"], "from_date": "2026-10-01"})
        );
        assert!(tool(&ask(&["a"], &["b"], None)).is_err());
        assert!(tool(&ask(&[], &[], Some("Oct 1"))).is_err());
        assert_eq!(
            tool(&ask(&["@"], &[], Some(" "))).unwrap(),
            json!({"type": "x_search"})
        );
    }

    #[test]
    fn the_request_names_its_filters() {
        assert_eq!(
            request(&ask(&["xai"], &[], Some("2026-10-01"))),
            "q\nOnly posts from @xai.\nFrom 2026-10-01."
        );
    }

    #[test]
    fn an_answer_is_its_text_then_its_links_once_each() {
        let reply = json!({"output": [
            {"type": "reasoning"},
            {"type": "message", "content": [{"type": "output_text", "text": "Two posts.",
                "annotations": [
                    {"type": "url_citation", "url": "https://x.com/a/status/1"},
                    {"type": "url_citation", "url": "https://x.com/a/status/1"},
                    {"type": "url_citation", "url": "https://x.com/b/status/2"}]}]}]});
        assert_eq!(
            answer(&reply).unwrap(),
            "Two posts.\n\nSources:\nhttps://x.com/a/status/1\nhttps://x.com/b/status/2"
        );
        assert!(answer(&json!({"output": []})).is_err());
    }

    #[test]
    fn a_refresh_without_a_new_refresh_token_keeps_the_old() {
        let l = granted(&json!({"access_token": "a", "expires_in": 60}), Some("r")).unwrap();
        assert_eq!((l.access.as_str(), l.refresh.as_str()), ("a", "r"));
        assert!(granted(&json!({"error": "invalid_grant"}), Some("r")).is_err());
        assert!(granted(&json!({"access_token": "a"}), None).is_err());
    }

    #[test]
    fn a_kept_connection_gives_its_token_and_its_label() {
        let dir = std::env::temp_dir().join(format!("libri-xai-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(connected(&dir), None);
        assert!(token(&dir).unwrap_err().contains("not connected"));
        keep(
            &dir,
            &Connection::Key {
                key: "xai-k".into(),
            },
        )
        .unwrap();
        assert_eq!(connected(&dir).as_deref(), Some("API key"));
        assert_eq!(token(&dir).unwrap(), "xai-k");
        let login = Login {
            account: "a@b.c".into(),
            access: "tok".into(),
            refresh: "r".into(),
            expires: now() + 3600,
        };
        keep(&dir, &Connection::Grok(login)).unwrap();
        assert_eq!(connected(&dir).as_deref(), Some("a@b.c"));
        assert_eq!(token(&dir).unwrap(), "tok");
        disconnect(&dir).unwrap();
        disconnect(&dir).unwrap();
        assert_eq!(connected(&dir), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_account_is_read_from_the_id_token() {
        let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(json!({"email": "a@b.c"}).to_string());
        let reply = json!({"id_token": format!("h.{claims}.s")});
        assert_eq!(account(&reply).as_deref(), Some("a@b.c"));
        assert_eq!(account(&json!({"id_token": "nonsense"})), None);
        assert_eq!(account(&json!({})), None);
    }

    #[test]
    fn a_key_with_spaces_inside_never_goes_out() {
        let dir = std::env::temp_dir();
        assert!(use_key(&dir, "two words").is_err());
        assert!(use_key(&dir, "  ").is_err());
    }
}
