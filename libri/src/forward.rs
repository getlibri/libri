//! Passes a request on to another Libri on this machine, for the tailnet
//! user it belongs to: one address serves everyone the machine hosts, each
//! in their own instance, with their own memory, keys and Unix user.
//!
//! The request goes on as it came, Host and Origin included, so the Libri
//! behind sees what the person's browser sent. It sees this machine as the
//! peer, so the gate sets X-Real-IP to the client it judged, and the Libri
//! behind judges that client in turn (tailnet.rs). Upgrades (the
//! terminal's WebSocket) are joined end to end once both sides switch.

use std::sync::LazyLock;

use axum::body::Body;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use hyper_util::rt::{TokioExecutor, TokioIo};

static CLIENT: LazyLock<Client<HttpConnector, Body>> =
    LazyLock::new(|| Client::builder(TokioExecutor::new()).build_http());

/// Sends `req` to the Libri at `to` (`http://host:port`) and gives back its
/// answer, streamed.
pub async fn pass(to: &str, mut req: Request) -> Response {
    let path = req.uri().path_and_query().map_or("/", |p| p.as_str());
    let Ok(uri) = format!("{to}{path}").parse::<http::Uri>() else {
        return StatusCode::BAD_GATEWAY.into_response();
    };
    *req.uri_mut() = uri;
    let ours = req
        .headers()
        .contains_key(http::header::UPGRADE)
        .then(|| hyper::upgrade::on(&mut req));
    let mut answer = match CLIENT.request(req).await {
        Ok(a) => a,
        Err(e) => {
            eprintln!("libri: forward to {to}: {e}");
            return (
                StatusCode::BAD_GATEWAY,
                "Your Libri on this machine is not answering.",
            )
                .into_response();
        }
    };
    if answer.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(ours) = ours
    {
        let theirs = hyper::upgrade::on(&mut answer);
        tokio::spawn(async move {
            match (ours.await, theirs.await) {
                (Ok(a), Ok(b)) => {
                    let _ =
                        tokio::io::copy_bidirectional(&mut TokioIo::new(a), &mut TokioIo::new(b))
                            .await;
                }
                (a, b) => eprintln!("libri: forward upgrade: {:?} {:?}", a.err(), b.err()),
            }
        });
    }
    answer.map(Body::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::ws::{Message, WebSocketUpgrade};
    use axum::routing::get;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn serve(app: Router) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let at = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await });
        at
    }

    /// A Libri behind, answering with the Host it was sent and echoing on a
    /// WebSocket; a door in front passing everything on to it.
    async fn door() -> String {
        let behind = serve(
            Router::new()
                .route(
                    "/api/host",
                    get(|h: http::HeaderMap| async move {
                        format!("host {}", h[http::header::HOST].to_str().unwrap())
                    }),
                )
                .route(
                    "/api/terminal",
                    get(|ws: WebSocketUpgrade| async {
                        ws.on_upgrade(|mut s| async move {
                            while let Some(Ok(Message::Text(t))) = s.recv().await {
                                let _ = s.send(Message::Text(t)).await;
                            }
                        })
                    }),
                ),
        )
        .await;
        serve(Router::new().fallback(move |req: Request| {
            let to = behind.clone();
            async move { pass(&to, req).await }
        }))
        .await
    }

    #[tokio::test]
    async fn passes_requests_and_upgrades_on() {
        let front = door().await;
        let addr = front.trim_start_matches("http://").to_string();

        let mut c = tokio::net::TcpStream::connect(&addr).await.unwrap();
        c.write_all(
            b"GET /api/host?x=1 HTTP/1.1\r\nHost: libri.example:8951\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
        let mut got = String::new();
        c.read_to_string(&mut got).await.unwrap();
        assert!(got.starts_with("HTTP/1.1 200"), "{got}");
        assert!(got.ends_with("host libri.example:8951"), "{got}");

        let mut c = tokio::net::TcpStream::connect(&addr).await.unwrap();
        c.write_all(
            b"GET /api/terminal HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
              Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
        )
        .await
        .unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(c.read_u8().await.unwrap());
        }
        assert!(
            head.starts_with(b"HTTP/1.1 101"),
            "{}",
            String::from_utf8_lossy(&head)
        );
        // A masked text frame "hi" (mask 0, so the payload stands as is).
        c.write_all(&[0x81, 0x82, 0, 0, 0, 0, b'h', b'i'])
            .await
            .unwrap();
        let mut echo = [0u8; 4];
        c.read_exact(&mut echo).await.unwrap();
        assert_eq!(echo, [0x81, 0x02, b'h', b'i']);
    }

    #[tokio::test]
    async fn says_when_nobody_answers_behind() {
        let r = pass("http://127.0.0.1:9", Request::new(Body::empty())).await;
        assert_eq!(r.status(), StatusCode::BAD_GATEWAY);
    }
}
