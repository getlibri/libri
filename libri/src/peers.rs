//! Peers: other people's assistants this one may message over A2A (a2a.rs),
//! each paired with its own credential.
//!
//! Pairing takes one request and one approval. The person here enters the
//! other assistant's address; this host makes a token for the pair, keeps
//! the peer as `Asking`, and sends the token with its own name and address
//! to the other's `/a2a/pair`. There the request waits as `Asked` until that
//! person approves it in Settings; the approval goes back to `/a2a/pair/approve`
//! with the token, and both sides hold the peer as `Paired`. Either side
//! revokes alone: it forgets the pair at once, so the token stops working in
//! both directions, and tells the other side so it forgets it too.
//!
//! The token is the pair's only credential: it signs every message either
//! way, and it names the sender, so a message can't claim to be from anyone
//! else. Peers are kept in `peers.json` in the state directory, readable by
//! this host alone.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// How many unanswered requests are kept, so nobody who can reach the port
/// can fill the list.
const MAX_ASKED: usize = 8;

/// Where a pair stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// This side asked; the other person hasn't approved yet.
    Asking,
    /// The other side asked; this person hasn't approved yet.
    Asked,
    /// Both agreed: messages go both ways.
    Paired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    /// The other assistant's name. A routine's `share_with` and a message
    /// name a peer by it or by its person.
    pub name: String,
    /// The person the other assistant works for, as its pairing or its
    /// Agent Card gives them; empty until known.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub person: String,
    /// Where the other assistant takes A2A calls, without the path.
    pub url: String,
    /// The pair's credential, presented by both sides.
    pub token: String,
    pub state: State,
    /// When a message last passed either way, in Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<u64>,
}

/// One lock for every read and write of the file, so two requests at once
/// can't lose a change.
static LOCK: Mutex<()> = Mutex::new(());

fn file(state: &Path) -> PathBuf {
    state.join("peers.json")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The peers as kept. A host that has no file yet but was given the old
/// shared token and peer list (`a2a.token_file`, `a2a.peers`) takes each
/// listed peer as paired under that token, once, so a pair set up that way
/// keeps working without anyone pairing again.
fn read(state: &Path) -> Result<Vec<Peer>, String> {
    match fs::read_to_string(file(state)) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("peers.json: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let imported = imported();
            if !imported.is_empty() {
                write(state, &imported)?;
            }
            Ok(imported)
        }
        Err(e) => Err(format!("peers.json: {e}")),
    }
}

// Remove with the a2a.token_file and a2a.peers settings once no host is
// configured with them.
fn imported() -> Vec<Peer> {
    let Some(Ok(token)) = crate::a2a::token() else {
        return Vec::new();
    };
    crate::a2a::peers()
        .into_iter()
        .map(|(name, url)| Peer {
            name,
            url,
            token: token.clone(),
            state: State::Paired,
            person: String::new(),
            last: None,
        })
        .collect()
}

fn write(state: &Path, peers: &[Peer]) -> Result<(), String> {
    let path = file(state);
    let tmp = path.with_extension("json.new");
    let text = serde_json::to_string_pretty(peers).map_err(|e| e.to_string())?;
    let mut f = {
        let mut o = fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        o.open(&tmp)
            .map_err(|e| format!("{}: {e}", tmp.display()))?
    };
    f.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Reads, changes and writes the list under the lock.
fn change<T>(
    state: &Path,
    f: impl FnOnce(&mut Vec<Peer>) -> Result<T, String>,
) -> Result<T, String> {
    let _held = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut peers = read(state)?;
    let out = f(&mut peers)?;
    write(state, &peers)?;
    Ok(out)
}

/// Every peer, in any state.
pub fn list(state: &Path) -> Result<Vec<Peer>, String> {
    let _held = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    read(state)
}

/// Whether `name` is this peer's assistant or its person, in any case.
fn called(p: &Peer, name: &str) -> bool {
    let name = name.trim();
    p.name.eq_ignore_ascii_case(name)
        || (!p.person.is_empty() && p.person.eq_ignore_ascii_case(name))
}

/// The paired peer called `name`, by its assistant's name or its person's,
/// in any case. A name that fits none first fills in the persons not yet
/// known, from each such peer's Agent Card, so a pair made before pairing
/// carried the person is found by it too.
pub fn paired(state: &Path, name: &str) -> Result<Option<Peer>, String> {
    let find = |peers: Vec<Peer>| {
        peers
            .into_iter()
            .find(|p| p.state == State::Paired && called(p, name))
    };
    if let Some(p) = find(list(state)?) {
        return Ok(Some(p));
    }
    Ok(find(learn(state)?))
}

/// Asks each paired peer whose person isn't known yet for its Agent Card,
/// and keeps the person it names; returns the peers as they now stand. A
/// peer that doesn't answer is asked again next time.
fn learn(state: &Path) -> Result<Vec<Peer>, String> {
    let unknown: Vec<String> = list(state)?
        .into_iter()
        .filter(|p| p.state == State::Paired && p.person.is_empty())
        .map(|p| p.url)
        .collect();
    if unknown.is_empty() {
        return list(state);
    }
    // Asked outside the lock: a card is a network call.
    let found: Vec<(String, String)> = unknown
        .into_iter()
        .filter_map(|url| crate::a2a::person_of(&url).map(|person| (url, person)))
        .collect();
    change(state, |peers| {
        for (url, person) in &found {
            if let Some(p) = peers.iter_mut().find(|p| &p.url == url) {
                p.person.clone_from(person);
            }
        }
        Ok(peers.clone())
    })
}

/// The paired peer whose credential `given` is, compared in constant time,
/// marked as heard from now.
pub fn caller(state: &Path, given: &str) -> Result<Option<Peer>, String> {
    if given.is_empty() {
        return Ok(None);
    }
    change(state, |peers| {
        let Some(p) = peers
            .iter_mut()
            .find(|p| p.state == State::Paired && same(&p.token, given))
        else {
            return Ok(None);
        };
        p.last = Some(now());
        Ok(Some(p.clone()))
    })
}

/// Marks the paired peer with `token` as just messaged.
pub fn sent(state: &Path, token: &str) -> Result<(), String> {
    change(state, |peers| {
        if let Some(p) = peers.iter_mut().find(|p| p.token == token) {
            p.last = Some(now());
        }
        Ok(())
    })
}

/// Fails, naming who can be reached, unless every name is a paired peer's
/// assistant or person.
pub fn check(state: &Path, names: &[String]) -> Result<(), String> {
    let paired = |all: Vec<Peer>| -> Vec<Peer> {
        all.into_iter()
            .filter(|p| p.state == State::Paired)
            .collect()
    };
    let missing = |peers: &[Peer]| -> Vec<String> {
        names
            .iter()
            .filter(|n| !peers.iter().any(|p| called(p, n)))
            .cloned()
            .collect()
    };
    let mut peers = paired(list(state)?);
    if !missing(&peers).is_empty() {
        peers = paired(learn(state)?);
    }
    let unknown = missing(&peers);
    if unknown.is_empty() {
        return Ok(());
    }
    let known: Vec<String> = peers
        .iter()
        .map(|p| match p.person.is_empty() {
            true => p.name.clone(),
            false => format!("{} ({}'s assistant)", p.name, p.person),
        })
        .collect();
    Err(match known.is_empty() {
        true => "No other assistant is paired here; pair one in Settings > Hosts > Peers.".into(),
        false => format!(
            "Nobody paired is called {}; those paired are {}.",
            unknown.join(", "),
            known.join(", ")
        ),
    })
}

/// This side asks the assistant at `url` to pair: keeps it as `Asking` under
/// a new token and returns the request to send it.
pub fn ask(state: &Path, url: &str) -> Result<Peer, String> {
    let url = address(url)?;
    let token = new_token()?;
    change(state, |peers| {
        if peers
            .iter()
            .any(|p| p.url == url && p.state == State::Paired)
        {
            return Err("That assistant is already paired.".into());
        }
        peers.retain(|p| p.url != url);
        let p = Peer {
            name: String::new(),
            person: String::new(),
            url,
            token,
            state: State::Asking,
            last: None,
        };
        peers.push(p.clone());
        Ok(p)
    })
}

/// A request from another assistant, kept as `Asked` for this person to
/// approve. A second request from the same address replaces the first.
pub fn asked(state: &Path, name: &str, person: &str, url: &str, token: &str) -> Result<(), String> {
    let url = address(url)?;
    let (name, person) = (name.trim(), person.trim());
    if name.is_empty() || name.len() > 64 || token.len() < 32 || token.len() > 128 {
        return Err("A request needs a name and a token.".into());
    }
    if person.len() > 64 {
        return Err("A person's name is at most 64 bytes.".into());
    }
    change(state, |peers| {
        if peers
            .iter()
            .any(|p| p.url == url && p.state == State::Paired)
        {
            return Err("Already paired.".into());
        }
        peers.retain(|p| !(p.url == url && p.state == State::Asked));
        if peers.iter().filter(|p| p.state == State::Asked).count() >= MAX_ASKED {
            return Err("Too many requests are waiting.".into());
        }
        peers.push(Peer {
            name: name.to_owned(),
            person: person.to_owned(),
            url,
            token: token.to_owned(),
            state: State::Asked,
            last: None,
        });
        Ok(())
    })
}

/// This person approves the request from `url`; returns the pair, now
/// `Paired`, for the approval to be sent back.
pub fn approve(state: &Path, url: &str) -> Result<Peer, String> {
    change(state, |peers| {
        let p = peers
            .iter_mut()
            .find(|p| p.url == url && p.state == State::Asked)
            .ok_or("No request from that address is waiting.")?;
        p.state = State::Paired;
        Ok(p.clone())
    })
}

/// The other side approved this side's request, presenting its token, and
/// gave its name and its person's.
pub fn approved(state: &Path, given: &str, name: &str, person: &str) -> Result<Peer, String> {
    let (name, person) = (name.trim(), person.trim());
    if name.is_empty() || name.len() > 64 || person.len() > 64 {
        return Err("An approval needs a name.".into());
    }
    change(state, |peers| {
        let p = peers
            .iter_mut()
            .find(|p| p.state == State::Asking && same(&p.token, given))
            .ok_or("No request with that token is waiting.")?;
        p.name = name.to_owned();
        p.person = person.to_owned();
        p.state = State::Paired;
        Ok(p.clone())
    })
}

/// Forgets the pair at `url`, in any state; returns it, so the other side
/// can be told.
pub fn revoke(state: &Path, url: &str) -> Result<Peer, String> {
    change(state, |peers| {
        let at = peers
            .iter()
            .position(|p| p.url == url)
            .ok_or("No peer at that address.")?;
        Ok(peers.remove(at))
    })
}

/// The other side revoked: forgets the pair whose token it presents.
pub fn revoked(state: &Path, given: &str) -> Result<bool, String> {
    change(state, |peers| {
        let before = peers.len();
        peers.retain(|p| !same(&p.token, given));
        Ok(peers.len() < before)
    })
}

/// An address an assistant takes calls at: http or https, no path.
fn address(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or("An address starts with http:// or https://.")?;
    if rest.is_empty() || rest.contains('/') || rest.contains(char::is_whitespace) {
        return Err("An address is a host and port, with no path.".into());
    }
    Ok(url.to_owned())
}

/// The address the assistant at `peer` should call this one back at: the
/// loopback port when the peer is on this machine, else this host's own
/// address (`daemon.url`).
pub fn own_address(peer: &str) -> Result<String, String> {
    let local = ["http://127.0.0.1:", "http://localhost:", "http://[::1]:"]
        .iter()
        .any(|p| peer.starts_with(p));
    if local {
        let at = std::env::var("LIBRI_SITE").unwrap_or_else(|_| crate::host::LISTEN.into());
        let port = at.rsplit(':').next().unwrap_or_default();
        return Ok(format!("http://127.0.0.1:{port}"));
    }
    std::env::var("LIBRI_SITE_URL")
        .map(|u| u.trim_end_matches('/').to_owned())
        .map_err(|_| "This host has no address another machine can reach (daemon.url).".into())
}

fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .map_err(|e| format!("/dev/urandom: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Compared in constant time, so a token can't be guessed a byte at a time.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("peers-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// Serves one request at a fresh address with `card` as the Agent Card.
    fn card_at(card: serde_json::Value) -> String {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut s, _) = server.accept().unwrap();
            let mut asked = [0u8; 4096];
            let _ = s.read(&mut asked);
            let body = card.to_string();
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        });
        url
    }

    #[test]
    fn a_pair_made_without_the_person_learns_them_from_the_card() {
        let d = dir("learn");
        let url = card_at(crate::a2a::card("Bea", "Bo", "http://127.0.0.1:9"));
        let p = ask(&d, &url).unwrap();
        approved(&d, &p.token, "Bea", "").unwrap();
        assert!(paired(&d, "Bea").unwrap().is_some());
        // Not known yet, so the card is asked, once, and the person kept.
        assert_eq!(paired(&d, "bo").unwrap().unwrap().name, "Bea");
        assert_eq!(list(&d).unwrap()[0].person, "Bo");
        check(&d, &["Bo".into()]).unwrap();
        let refused = check(&d, &["Cy".into()]).unwrap_err();
        assert!(refused.contains("Bea (Bo's assistant)"), "{refused}");
    }

    #[test]
    fn a_pair_is_asked_approved_and_revoked_on_both_sides() {
        let (a, b) = (dir("a"), dir("b"));
        let request = ask(&a, "http://127.0.0.1:2").unwrap();
        assert_eq!(request.state, State::Asking);
        asked(&b, "Ann", "Al", "http://127.0.0.1:1", &request.token).unwrap();
        // Not paired until approved: the token opens nothing yet.
        assert!(caller(&b, &request.token).unwrap().is_none());
        let pair = approve(&b, "http://127.0.0.1:1").unwrap();
        approved(&a, &pair.token, "Bea", "Bo").unwrap();
        assert_eq!(caller(&a, &request.token).unwrap().unwrap().name, "Bea");
        assert_eq!(caller(&b, &request.token).unwrap().unwrap().name, "Ann");
        assert!(caller(&a, "wrong").unwrap().is_none());
        check(&a, &["bea".into()]).unwrap();
        // The person stands for their assistant, in any case.
        check(&a, &["bo".into()]).unwrap();
        check(&b, &["Al".into()]).unwrap();
        assert_eq!(paired(&a, "BO").unwrap().unwrap().name, "Bea");
        let refused = check(&a, &["Cy".into()]).unwrap_err();
        assert!(refused.contains("Bea (Bo's assistant)"), "{refused}");
        // Revoking one side stops the token there at once.
        revoke(&a, "http://127.0.0.1:2").unwrap();
        assert!(caller(&a, &request.token).unwrap().is_none());
        assert!(revoked(&b, &request.token).unwrap());
        assert!(list(&b).unwrap().is_empty());
        for d in [a, b] {
            fs::remove_dir_all(d).unwrap();
        }
    }

    #[test]
    fn requests_are_bounded_and_addresses_have_no_path() {
        let d = dir("bound");
        let token = "t".repeat(64);
        for i in 0..MAX_ASKED {
            asked(
                &d,
                "X",
                "",
                &format!("http://127.0.0.1:{}", 100 + i),
                &token,
            )
            .unwrap();
        }
        assert!(asked(&d, "X", "", "http://127.0.0.1:999", &token).is_err());
        assert!(asked(&d, "X", "", "http://127.0.0.1:1/a2a", &token).is_err());
        assert!(asked(&d, "X", "", "http://127.0.0.1:1", "short").is_err());
        fs::remove_dir_all(d).unwrap();
    }
}
