//! The push timer: any program that can POST wakes a watch, at
//! `/hook/<id>/<token>` on the site, and what it sends is an item as a look's
//! is, passing the same tests and seen keys. Nothing here knows a sender:
//! the token, the signature, the seen key, the tests and the fields are all
//! data on the watch, set when it is made.
//!
//! The shape follows T3's webhook triggers: the secret is in the address, a
//! wrong token answers as an unknown hook does, the rate is checked before
//! any signature work, and only named fields reach a turn. A sender gives
//! seconds, so the turn never runs in the request: what passes is stored,
//! answered 202, and the tick runs `then`. What is seen or fails its tests
//! is answered 200 and costs no model.
//!
//! The site is the only door, so a hook reaches as far as the site does.

use crate::book::Book;
use crate::watch::{self, Arrival, Watch};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use base64::Engine;
use chrono::Local;
use ring::{digest, hmac};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PATH: &str = "/hook";

/// Most a body may be, chunked or not.
const MOST_BODY: usize = 1024 * 1024;

/// Most deliveries one hook takes in `SLOT`.
const MOST_DELIVERIES: usize = 60;
const SLOT: Duration = Duration::from_secs(60);

const SECRET_BYTES: usize = 32;

/// How a signature is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    #[default]
    Hex,
    Base64,
}

/// A sender's HMAC-SHA256 over the raw body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Signature {
    /// The header that carries it.
    pub header: String,
    #[serde(default)]
    pub encoding: Encoding,
    /// What comes before the signature in the header, such as `sha256=`.
    #[serde(default)]
    pub prefix: String,
    /// The shared secret, if the sender gives one; leave out and one is made
    /// to give the sender.
    #[serde(default)]
    pub secret: String,
}

impl Signature {
    fn holds(&self, headers: &HeaderMap, body: &[u8]) -> bool {
        let tag = headers
            .get(&self.header)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().strip_prefix(&self.prefix))
            .and_then(|v| match self.encoding {
                Encoding::Hex => hex_bytes(v),
                Encoding::Base64 => base64::engine::general_purpose::STANDARD.decode(v).ok(),
            });
        let key = hmac::Key::new(hmac::HMAC_SHA256, self.secret.as_bytes());
        tag.is_some_and(|tag| hmac::verify(&key, body, &tag).is_ok())
    }
}

fn hex_bytes(s: &str) -> Option<Vec<u8>> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn secret() -> Result<String, String> {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| format!("hook: {e}"))?;
    Ok(hex(&bytes))
}

/// What a watch that wakes on a push is asked to do with one.
#[derive(Deserialize, JsonSchema)]
pub struct Push {
    /// The path into an item that names a delivery, for seen keys, such as
    /// `header.x-delivery-id` or `body.event.id`; leave out to tell
    /// deliveries apart by their bytes.
    pub key: Option<String>,
    /// Leave out for a sender that signs nothing: the secret address is then
    /// its only check.
    pub signature: Option<Signature>,
}

/// A watch's push settings, with its secret address.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hook {
    pub token: String,
    pub key: Option<String>,
    pub signature: Option<Signature>,
}

impl Hook {
    pub fn made(push: Push) -> Result<Hook, String> {
        let mut signature = push.signature;
        if let Some(s) = &mut signature
            && s.secret.is_empty()
        {
            s.secret = secret()?;
        }
        Ok(Hook {
            token: secret()?,
            key: push.key,
            signature,
        })
    }

    /// Where its sender posts, said this once: the book keeps the token and
    /// no listing of watches shows it.
    pub fn told(&self, id: u64) -> String {
        let path = format!("{PATH}/{id}/{}", self.token);
        let var = |name| std::env::var(name).ok().filter(|v: &String| !v.is_empty());
        let mut out = format!("Watch {id} wakes on a POST to {path} on the site");
        if let Some(url) = var("LIBRI_SITE_URL") {
            out.push_str(&format!(": {}{path}", url.trim_end_matches('/')));
        }
        if let Some(local) = var("LIBRI_SITE") {
            out.push_str(&format!(", or from this machine http://{local}{path}"));
        }
        out.push_str(". This is the only time the address is shown.");
        if let Some(s) = &self.signature {
            // "secret: " is the shape hippo's mask knows; this reply is logged.
            out.push_str(&format!(" Signing secret: {}", s.secret));
        }
        out
    }

    /// The seen key of a delivery: what the watch names, or else its bytes.
    fn key_of(&self, item: &Value, body: &[u8]) -> String {
        self.key
            .as_deref()
            .and_then(|path| watch::field(item, path))
            .map(watch::shown)
            .filter(|key| !key.is_empty())
            .unwrap_or_else(|| hex(digest::digest(&digest::SHA256, body).as_ref()))
    }
}

/// A delivery as an item: `body` is its JSON, or its text when it is not
/// JSON, and `header` its headers by lowercase name, for tests and fields to
/// reach by path.
fn item(headers: &HeaderMap, body: &[u8]) -> Value {
    let header: serde_json::Map<_, _> = headers
        .iter()
        .map(|(name, value)| {
            let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
            (name.as_str().to_owned(), Value::String(value))
        })
        .collect();
    let body = serde_json::from_slice(body)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(body).into_owned()));
    json!({"body": body, "header": header})
}

/// Compared in constant time, so a token can't be guessed a byte at a time.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0
}

struct Door {
    state: PathBuf,
    /// When each hook last took deliveries, within `SLOT`.
    taken: Mutex<HashMap<u64, VecDeque<Instant>>>,
}

impl Door {
    /// Takes a place in the hook's rate. It comes after the token, so a
    /// stranger guessing at an id can't use up its sender's places.
    fn slot(&self, id: u64) -> bool {
        let now = Instant::now();
        let mut taken = self.taken.lock().unwrap();
        let taken = taken.entry(id).or_default();
        while taken.front().is_some_and(|t| now - *t >= SLOT) {
            taken.pop_front();
        }
        let free = taken.len() < MOST_DELIVERIES;
        if free {
            taken.push_back(now);
        }
        free
    }

    fn deliver(
        &self,
        id: &str,
        token: &str,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<StatusCode, String> {
        let mut book = Book::open(&self.state)?;
        let found = match id.parse() {
            Ok(id) => book.read(|db| watch::get(db, id))?,
            Err(_) => None,
        };
        let open = |w: Watch| {
            let hook = w
                .hook
                .clone()
                .filter(|h| w.active && same(&h.token, token))?;
            Some((w, hook))
        };
        let Some((w, hook)) = found.and_then(open) else {
            return Ok(StatusCode::NOT_FOUND);
        };
        if !self.slot(w.id) {
            return Ok(StatusCode::TOO_MANY_REQUESTS);
        }
        if let Some(s) = &hook.signature
            && !s.holds(headers, body)
        {
            return Ok(StatusCode::UNAUTHORIZED);
        }
        let item = item(headers, body);
        let key = hook.key_of(&item, body);
        let now = Local::now().naive_local();
        Ok(
            match book.change(|db| watch::pushed(db, &w, &key, &item, now))? {
                Arrival::Seen | Arrival::Failed => StatusCode::OK,
                Arrival::Fired => StatusCode::ACCEPTED,
                Arrival::Full => StatusCode::TOO_MANY_REQUESTS,
            },
        )
    }
}

async fn receive(
    State(door): State<Arc<Door>>,
    Path((id, token)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let taken = tokio::task::spawn_blocking(move || door.deliver(&id, &token, &headers, &body));
    taken
        .await
        .map_err(|e| e.to_string())
        .and_then(|status| status)
        .unwrap_or_else(|e| {
            eprintln!("libri: hook: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

/// The route, for the site to carry.
pub fn router<S: Clone + Send + Sync + 'static>(state: PathBuf) -> Router<S> {
    let door = Arc::new(Door {
        state,
        taken: Mutex::default(),
    });
    Router::new()
        .route(
            &format!("{PATH}/{{id}}/{{token}}"),
            post(receive).layer(DefaultBodyLimit::max(MOST_BODY)),
        )
        .with_state(door)
}

/// The door on a port of its own, for a test to post to.
#[cfg(test)]
pub async fn serving(state: PathBuf) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let door = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let app: Router = router(state);
        axum::serve(listener, app).await.unwrap();
    });
    door
}

/// A delivery, and the status it was answered with.
#[cfg(test)]
pub fn deliver(url: &str, headers: &[(&str, String)], body: Vec<u8>) -> u16 {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut asked = agent.post(url);
    for (name, value) in headers {
        asked = asked.header(*name, value);
    }
    asked.send(&body[..]).unwrap().status().as_u16()
}

/// A signature as a sender would write it.
#[cfg(test)]
pub fn signed(s: &Signature, body: &[u8]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, s.secret.as_bytes());
    format!("{}{}", s.prefix, hex(hmac::sign(&key, body).as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn tag(s: &Signature, body: &[u8]) -> Vec<u8> {
        let key = hmac::Key::new(hmac::HMAC_SHA256, s.secret.as_bytes());
        hmac::sign(&key, body).as_ref().to_vec()
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("libri-hook-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn hooked(state: &std::path::Path, w: Watch) -> (u64, String) {
        let token = w.hook.as_ref().unwrap().token.clone();
        let mut book = Book::open(state).unwrap();
        (book.change(|db| watch::add(db, &w)).unwrap(), token)
    }

    fn bare() -> Watch {
        let push = Push {
            key: None,
            signature: None,
        };
        Watch {
            text: "t".into(),
            hook: Some(Hook::made(push).unwrap()),
            ..Watch::default()
        }
    }

    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into()
    }

    #[test]
    fn signatures_read_as_the_watch_says_they_are_written() {
        let body = b"raw bytes, spaces and all ";
        let mut s = Signature {
            header: "X-Sig".into(),
            encoding: Encoding::Hex,
            prefix: "sha256=".into(),
            secret: "shared".into(),
        };
        let mut headers = HeaderMap::new();
        let given = format!("sha256={}", hex(&tag(&s, body)));
        headers.insert("x-sig", given.parse().unwrap());
        assert!(s.holds(&headers, body));
        assert!(!s.holds(&headers, b"another body"));
        assert!(!s.holds(&HeaderMap::new(), body));
        s.prefix = "other=".into();
        assert!(!s.holds(&headers, body));
        s.prefix = String::new();
        s.encoding = Encoding::Base64;
        let given = base64::engine::general_purpose::STANDARD.encode(tag(&s, body));
        headers.insert("x-sig", given.parse().unwrap());
        assert!(s.holds(&headers, body));
        s.secret = "wrong".into();
        assert!(!s.holds(&headers, body));
    }

    #[test]
    fn a_delivery_is_its_body_and_headers_and_its_key_may_be_either() {
        let mut headers = HeaderMap::new();
        headers.insert("X-Delivery", "d1".parse().unwrap());
        let json = item(&headers, br#"{"a":{"b":1}}"#);
        assert_eq!(json["body"]["a"]["b"], 1);
        assert_eq!(json["header"]["x-delivery"], "d1");
        assert_eq!(item(&headers, b"plain words")["body"], "plain words");
        let by = |key: &str| Hook {
            token: String::new(),
            key: Some(key.into()),
            signature: None,
        };
        assert_eq!(by("header.x-delivery").key_of(&json, b"x"), "d1");
        assert_eq!(by("body.a.b").key_of(&json, b"x"), "1");
        let hash = by("body.missing").key_of(&json, b"x");
        assert_eq!(hash.len(), 64);
        assert_ne!(hash, by("body.missing").key_of(&json, b"y"));
    }

    #[tokio::test]
    async fn a_wrong_token_and_an_unknown_hook_answer_alike() {
        let d = dir("doors");
        let (id, token) = hooked(&d, bare());
        let (off, off_token) = hooked(
            &d,
            Watch {
                active: true,
                ..bare()
            },
        );
        let mut book = Book::open(&d).unwrap();
        book.change(|db| {
            let mut w = watch::get(db, off)?.unwrap();
            w.active = false;
            watch::change(db, &w)
        })
        .unwrap();
        let door = serving(d).await;
        let (a, b, c, d) = (
            format!("{door}/hook/{id}/{}", "0".repeat(64)),
            format!("{door}/hook/999/{token}"),
            format!("{door}/hook/nonsense/{token}"),
            format!("{door}/hook/{off}/{off_token}"),
        );
        let right = format!("{door}/hook/{id}/{token}");
        tokio::task::spawn_blocking(move || {
            for url in [a, b, c, d] {
                let mut r = agent().post(&url).send(&b"x"[..]).unwrap();
                let body = r.body_mut().read_to_string().unwrap();
                assert_eq!((r.status().as_u16(), body.as_str()), (404, ""), "{url}");
            }
            assert_eq!(deliver(&right, &[], b"x".to_vec()), 202);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn the_rate_stops_a_flood_before_any_signature_work() {
        let d = dir("rate");
        let push = Push {
            key: None,
            signature: Some(Signature {
                header: "x-sig".into(),
                encoding: Encoding::Hex,
                prefix: String::new(),
                secret: "shared".into(),
            }),
        };
        let (id, token) = hooked(
            &d,
            Watch {
                hook: Some(Hook::made(push).unwrap()),
                ..bare()
            },
        );
        let door = serving(d).await;
        let url = format!("{door}/hook/{id}/{token}");
        tokio::task::spawn_blocking(move || {
            for _ in 0..MOST_DELIVERIES {
                assert_eq!(deliver(&url, &[], b"x".to_vec()), 401);
            }
            assert_eq!(deliver(&url, &[], b"x".to_vec()), 429);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_body_past_the_cap_is_refused_chunked_or_not() {
        let d = dir("cap");
        let (id, token) = hooked(&d, bare());
        let door = serving(d).await;
        let url = format!("{door}/hook/{id}/{token}");
        tokio::task::spawn_blocking(move || {
            let big = vec![b'x'; MOST_BODY + 1];
            assert_eq!(deliver(&url, &[], big.clone()), 413);
            // Through a reader ureq sends it chunked, with no length to refuse.
            let chunked = agent()
                .post(&url)
                .send(ureq::SendBody::from_reader(&mut (&big[..]).take(u64::MAX)))
                .unwrap();
            assert_eq!(chunked.status().as_u16(), 413);
        })
        .await
        .unwrap();
    }
}
