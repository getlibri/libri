//! Who is connecting, by the tailnet's word rather than the address typed.
//!
//! With `daemon.tailnet` set, each request's peer address is looked up in the
//! local tailscaled (its LocalAPI `whois`, the call `tailscale whois` makes
//! and Tailscale Serve's identity headers come from), and only the tailnet
//! users listed get in: one person's Libri refuses another's devices on the
//! same tailnet, with no password or sign-in. An entry `login/device` lets in
//! only that one device of the login's, so a machine shared by several people
//! but owned on the tailnet by one can reach a host its owner's other devices
//! cannot. Loopback is the machine itself
//! (its harnesses, hooks and tool servers) and is let in as before.
//!
//! A reverse proxy on this machine (nginx terminating TLS for a name) reaches
//! the host from loopback, so a request from loopback that carries
//! `X-Real-IP` is judged by that address instead, the one nginx's
//! recommended proxy headers set to `$remote_addr` and overwrite whatever
//! the client sent. X-Forwarded-For is not read: nginx appends to the one a
//! client sends, so its first entry is the client's to choose. A forward
//! sets X-Real-IP the same way for the Libri behind it. The header from any
//! other peer is ignored, so only a process on this machine can name an
//! address, and such a process is let in as the machine anyway.
//!
//! With `daemon.forward` set, the users it names are not refused but passed
//! on, request and all, to their own Libri on the same machine: one address
//! serves everyone the machine hosts, each in their own instance.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Where tailscaled listens on Linux and the BSDs unless told otherwise.
const SOCKET: &str = "/var/run/tailscale/tailscaled.sock";

/// How long an answer stands: a device's owner seldom changes, and every
/// request, assets included, passes through here.
const KEEP: Duration = Duration::from_secs(60);

/// A lookup that takes longer is a tailscaled in trouble: refuse rather
/// than hold the request.
const PATIENCE: Duration = Duration::from_secs(2);

/// The tailnet user a request comes from.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Person {
    /// Their tailnet login, as the allow-list names them.
    pub login: String,
    pub name: String,
    /// The device's name on the tailnet.
    pub device: String,
}

/// What the gate decides for one peer.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// This machine itself.
    Local,
    In(Person),
    /// A user whose own Libri is at this address: the request goes there.
    Forward(String),
    /// A tailnet user not on the list, or none the tailnet knows.
    Out(Option<Person>),
}

pub struct Tailnet {
    /// Lowercased `login` or `login/device` entries.
    users: Vec<String>,
    /// Login to the address of that user's own Libri (LIBRI_FORWARD).
    forwards: HashMap<String, String>,
    socket: PathBuf,
    seen: Mutex<HashMap<IpAddr, (Instant, Option<Person>)>>,
}

impl Tailnet {
    /// The gate, when this host names its tailnet users (LIBRI_TAILNET,
    /// comma-separated logins) or those it passes on (LIBRI_FORWARD,
    /// comma-separated `login=http://address`).
    pub fn from_env() -> Option<Self> {
        let users: Vec<String> = std::env::var("LIBRI_TAILNET")
            .unwrap_or_default()
            .split(',')
            .map(|u| u.trim().to_lowercase())
            .filter(|u| !u.is_empty())
            .collect();
        let forwards = forwards(&std::env::var("LIBRI_FORWARD").unwrap_or_default());
        if users.is_empty() && forwards.is_empty() {
            return None;
        }
        let socket = std::env::var_os("LIBRI_TAILSCALE_SOCKET")
            .map(PathBuf::from)
            .unwrap_or_else(|| SOCKET.into());
        Some(Tailnet {
            users,
            forwards,
            socket,
            seen: Mutex::default(),
        })
    }

    pub async fn judge(&self, ip: IpAddr) -> Verdict {
        let ip = canonical(ip);
        if ip.is_loopback() {
            return Verdict::Local;
        }
        let person = self.whois(ip).await;
        match person {
            Some(p) if self.admits(&p) => Verdict::In(p),
            Some(p) if let Some(to) = self.forwards.get(&p.login.to_lowercase()) => {
                Verdict::Forward(to.clone())
            }
            other => Verdict::Out(other),
        }
    }

    /// Whether an entry names this person's login, or this one device of it.
    fn admits(&self, p: &Person) -> bool {
        let login = p.login.to_lowercase();
        let device = p.device.to_lowercase();
        self.users.iter().any(|u| match u.split_once('/') {
            Some((l, d)) => l == login && d == device,
            None => *u == login,
        })
    }

    async fn whois(&self, ip: IpAddr) -> Option<Person> {
        if let Some((at, p)) = self.seen.lock().unwrap().get(&ip)
            && at.elapsed() < KEEP
        {
            return p.clone();
        }
        let found = match tokio::time::timeout(PATIENCE, ask(&self.socket, ip)).await {
            Ok(Ok(p)) => p,
            Ok(Err(e)) => {
                // Not kept: a tailscaled restarting answers again soon.
                eprintln!("libri: tailnet: whois {ip}: {e}");
                return None;
            }
            Err(_) => {
                eprintln!("libri: tailnet: whois {ip}: no answer from tailscaled");
                return None;
            }
        };
        let mut seen = self.seen.lock().unwrap();
        seen.retain(|_, (at, _)| at.elapsed() < KEEP);
        seen.insert(ip, (Instant::now(), found.clone()));
        found
    }
}

/// The address a request comes from: the socket's peer, or for a peer on
/// loopback (a proxy on this machine) the `X-Real-IP` it sets. None when that
/// header is there but names no address: refused, not taken as loopback.
pub fn client(peer: SocketAddr, real_ip: Option<&http::HeaderValue>) -> Option<IpAddr> {
    let ip = canonical(peer.ip());
    match real_ip {
        Some(h) if ip.is_loopback() => h
            .to_str()
            .ok()
            .and_then(|v| v.trim().parse::<IpAddr>().ok())
            .map(canonical),
        _ => Some(ip),
    }
}

/// An IPv4 peer reached over a dual-stack socket arrives mapped into IPv6.
/// `login=http://address` pairs, comma-separated; a malformed one is said
/// and skipped, so its user is refused rather than let in here.
fn forwards(raw: &str) -> HashMap<String, String> {
    raw.split(',')
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .filter_map(|f| match f.split_once('=') {
            Some((login, to)) if to.trim().starts_with("http://") => Some((
                login.trim().to_lowercase(),
                to.trim().trim_end_matches('/').to_string(),
            )),
            _ => {
                eprintln!("libri: tailnet: forward {f:?} is not login=http://address");
                None
            }
        })
        .collect()
}

fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

#[derive(Deserialize)]
struct WhoIs {
    #[serde(rename = "Node")]
    node: Node,
    #[serde(rename = "UserProfile")]
    user: Profile,
}

#[derive(Deserialize)]
struct Node {
    #[serde(rename = "ComputedName", default)]
    computed: String,
    #[serde(rename = "Name", default)]
    name: String,
}

#[derive(Deserialize)]
struct Profile {
    #[serde(rename = "LoginName")]
    login: String,
    #[serde(rename = "DisplayName", default)]
    name: String,
}

/// Asks tailscaled who holds an address: Ok(None) when the tailnet has no
/// such peer.
async fn ask(socket: &std::path::Path, ip: IpAddr) -> Result<Option<Person>, String> {
    let mut s = tokio::net::UnixStream::connect(socket)
        .await
        .map_err(|e| format!("{}: {e}", socket.display()))?;
    // The port is required by the call and unused by it. HTTP/1.0, so the
    // answer ends where the connection does.
    let addr = SocketAddr::new(ip, 1);
    let req = format!(
        "GET /localapi/v0/whois?addr={addr} HTTP/1.0\r\nHost: local-tailscaled.sock\r\n\r\n"
    );
    s.write_all(req.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.map_err(|e| e.to_string())?;
    parse(&String::from_utf8_lossy(&raw))
}

fn parse(raw: &str) -> Result<Option<Person>, String> {
    let (head, body) = raw.split_once("\r\n\r\n").ok_or("no answer")?;
    let status = head.split_whitespace().nth(1).unwrap_or("");
    match status {
        "200" => {}
        // tailscaled's word for an address no peer holds.
        "404" => return Ok(None),
        _ => return Err(format!("status {status}: {}", body.trim())),
    }
    let w: WhoIs = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let device = if w.node.computed.is_empty() {
        w.node.name.split('.').next().unwrap_or_default().to_owned()
    } else {
        w.node.computed
    };
    Ok(Some(Person {
        login: w.user.login,
        name: w.user.name,
        device,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADA: &str = "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"Node\":{\"Name\":\"box.example.ts.net.\",\"ComputedName\":\"box\"},\"UserProfile\":{\"LoginName\":\"ada@example.com\",\"DisplayName\":\"Ada\"},\"CapMap\":null}";

    fn gate(users: &[&str], socket: PathBuf) -> Tailnet {
        Tailnet {
            users: users.iter().map(|u| u.to_string()).collect(),
            forwards: HashMap::new(),
            socket,
            seen: Mutex::default(),
        }
    }

    #[test]
    fn reads_whois() {
        let p = parse(ADA).unwrap().unwrap();
        assert_eq!(p.login, "ada@example.com");
        assert_eq!(p.name, "Ada");
        assert_eq!(p.device, "box");
        assert_eq!(
            parse("HTTP/1.0 404 Not Found\r\n\r\nno match").unwrap(),
            None
        );
        assert!(parse("HTTP/1.0 403 Forbidden\r\n\r\ndenied").is_err());
    }

    /// A stand-in tailscaled on a socket, answering each whois with `answer`.
    fn tailscaled(dir: &std::path::Path, answer: &'static str) -> PathBuf {
        let path = dir.join("tailscaled.sock");
        let l = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for mut c in l.incoming().flatten() {
                let mut buf = [0u8; 1024];
                let _ = c.read(&mut buf);
                let _ = c.write_all(answer.as_bytes());
            }
        });
        path
    }

    #[tokio::test]
    async fn lets_in_only_the_listed() {
        let dir = std::env::temp_dir().join(format!("libri-tailnet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = tailscaled(&dir, ADA);
        let peer: SocketAddr = "100.64.0.2:5000".parse().unwrap();

        let ada = gate(&["ada@example.com"], socket.clone());
        assert!(matches!(ada.judge(peer.ip()).await, Verdict::In(p) if p.name == "Ada"));
        let bob = gate(&["bob@example.com"], socket);
        assert!(matches!(bob.judge(peer.ip()).await, Verdict::Out(Some(_))));
        assert_eq!(
            bob.judge("127.0.0.1".parse().unwrap()).await,
            Verdict::Local
        );
        assert_eq!(
            bob.judge("::ffff:127.0.0.1".parse().unwrap()).await,
            Verdict::Local
        );
        // No tailscaled: nobody from outside gets in.
        let none = gate(&["ada@example.com"], dir.join("absent.sock"));
        assert_eq!(none.judge(peer.ip()).await, Verdict::Out(None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn lets_in_only_the_listed_device() {
        let dir = std::env::temp_dir().join(format!("libri-device-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = tailscaled(&dir, ADA);
        let peer: SocketAddr = "100.64.0.2:5000".parse().unwrap();

        let shared = gate(&["bob@example.com", "ada@example.com/box"], socket.clone());
        assert!(matches!(shared.judge(peer.ip()).await, Verdict::In(p) if p.device == "box"));
        let other = gate(&["ada@example.com/laptop"], socket.clone());
        assert!(matches!(
            other.judge(peer.ip()).await,
            Verdict::Out(Some(_))
        ));
        let stranger = gate(&["bob@example.com/box"], socket);
        assert!(matches!(
            stranger.judge(peer.ip()).await,
            Verdict::Out(Some(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_proxy_on_loopback_names_the_client() {
        let h = |v: &str| http::HeaderValue::from_str(v).unwrap();
        let lo: SocketAddr = "127.0.0.1:40000".parse().unwrap();
        let far: SocketAddr = "100.64.0.9:40000".parse().unwrap();
        // Through nginx: the address it set.
        assert_eq!(
            client(lo, Some(&h("100.64.0.2"))),
            Some("100.64.0.2".parse().unwrap())
        );
        assert_eq!(
            client(lo, Some(&h(" ::ffff:100.64.0.2 "))),
            Some("100.64.0.2".parse().unwrap())
        );
        // The machine itself, with no proxy in between.
        assert_eq!(client(lo, None), Some("127.0.0.1".parse().unwrap()));
        // A tailnet peer naming another address is judged as itself.
        assert_eq!(
            client(far, Some(&h("100.64.0.2"))),
            Some("100.64.0.9".parse().unwrap())
        );
        assert_eq!(
            client(far, Some(&h("127.0.0.1"))),
            Some("100.64.0.9".parse().unwrap())
        );
        // A header naming no address is refused, not taken as loopback.
        assert_eq!(client(lo, Some(&h("100.64.0.2, 127.0.0.1"))), None);
        assert_eq!(client(lo, Some(&h(""))), None);
    }

    #[tokio::test]
    async fn the_proxied_client_is_judged_not_the_proxy() {
        let dir = std::env::temp_dir().join(format!("libri-proxied-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bob = gate(&["bob@example.com"], tailscaled(&dir, ADA));
        let lo: SocketAddr = "127.0.0.1:40000".parse().unwrap();
        let ada = http::HeaderValue::from_static("100.64.0.2");
        // Ada through the proxy is Ada, and refused here.
        let ip = client(lo, Some(&ada)).unwrap();
        assert!(
            matches!(bob.judge(ip).await, Verdict::Out(Some(p)) if p.login == "ada@example.com")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_forwards() {
        let f = forwards(" Ada@Example.com=http://127.0.0.1:8952/ , bad, bob@x=https://y");
        assert_eq!(f.len(), 1);
        assert_eq!(f["ada@example.com"], "http://127.0.0.1:8952");
    }

    #[tokio::test]
    async fn passes_on_the_forwarded() {
        let dir = std::env::temp_dir().join(format!("libri-forward-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut door = gate(&["bob@example.com"], tailscaled(&dir, ADA));
        door.forwards = forwards("ada@example.com=http://127.0.0.1:8952");
        assert_eq!(
            door.judge("100.64.0.2".parse().unwrap()).await,
            Verdict::Forward("http://127.0.0.1:8952".into())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
