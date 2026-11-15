//! The share listener: what a board link (boards.rs `Link`) opens to
//! someone with no sign-in, served on a socket of its own so that nothing
//! else of Libri is reachable through it. The deployment points a public
//! name at it (a tunnel to this socket); the person's own site stays on its
//! own listener behind its sign-in and the tailnet.
//!
//! It serves the site's public build (the page and its assets, which hold
//! nothing of the person's) and one board, the link's, read by the token in
//! the `X-Share-Token` header. The token travels in the address's fragment
//! (`#board=<token>`), which a browser never sends, so it stays out of
//! request lines and the tunnel's logs. No cookies are set or read, and
//! every answer forbids the referrer, caching and framing.
//!
//! A view link reads and follows the board; an edit link may also change it,
//! recorded as by "link". Removing the board, or the person taking the link
//! back or letting it lapse, closes it at the next request.

use crate::boards::{self, Role};
use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, middleware};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::services::ServeDir;

/// The variable naming the share listener's address; unset, there is none.
pub const LISTEN: &str = "LIBRI_SHARE_LISTEN";
/// The variable naming the public address links are made under.
pub const URL: &str = "LIBRI_SHARE_URL";

/// The address a link opens at, when the deployment has given one.
pub fn link_url(token: &str) -> Option<String> {
    let base = std::env::var(URL).ok()?;
    let base = base.trim().trim_end_matches('/');
    (!base.is_empty()).then(|| format!("{base}/#board={token}"))
}

struct Shares {
    state: PathBuf,
    index: PathBuf,
}

/// Serves board links on `addr` until the process ends.
pub async fn serve(addr: &str, root: PathBuf, state: PathBuf) -> Result<(), String> {
    let shares = Arc::new(Shares {
        state,
        index: root.join("board.html"),
    });
    let app = Router::new()
        .route("/", get(index))
        .nest_service(
            "/_site",
            ServeDir::new(root.join("_site"))
                .precompressed_br()
                .precompressed_gzip(),
        )
        .nest_service("/icon", ServeDir::new(root.join("icon")))
        .route("/api/share/board", get(board).post(change))
        .route("/api/share/board/export", get(export))
        .fallback(|| async { StatusCode::NOT_FOUND })
        .layer(middleware::map_response(guarded))
        .with_state(shares);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("share {addr}: {e}"))?;
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("share: {e}"))
}

/// Headers every answer carries: a shared board's address stays out of the
/// pages it links to, out of caches, and out of other sites' frames.
async fn guarded(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static("no-store"));
    r
}

async fn index(State(s): State<Arc<Shares>>) -> Response {
    match std::fs::read_to_string(&s.index) {
        Ok(html) => axum::response::Html(html.replacen(
            "<title>Whiteboard</title>",
            "<title>Shared board</title><meta name=\"robots\" content=\"noindex\" />",
            1,
        ))
        .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// The board and role the request's token opens, or the one refusal every
/// unknown, lapsed or taken-back token gets.
async fn opened(
    s: &Shares,
    headers: &HeaderMap,
) -> Result<(boards::Board, Role), (StatusCode, String)> {
    let token = headers
        .get("x-share-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let state = s.state.clone();
    match tokio::task::spawn_blocking(move || boards::link_open(&state, &token)).await {
        Ok(Ok(o)) => Ok(o),
        Ok(Err(e)) => Err((StatusCode::NOT_FOUND, e)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

#[derive(Deserialize)]
struct Since {
    since: Option<u64>,
}

/// The board as it stands, with its name and what the link may do; with
/// `since`, what changed past that version (or nothing, after a wait).
async fn board(State(s): State<Arc<Shares>>, headers: HeaderMap, q: Query<Since>) -> Response {
    let (b, role) = match opened(&s, &headers).await {
        Ok(o) => o,
        Err(r) => return r.into_response(),
    };
    let state = s.state.clone();
    let id = b.id.clone();
    let got = tokio::task::spawn_blocking(move || match q.since {
        Some(v) => boards::wait(&state, &id, v, boards::WAIT)
            .map(|w| w.map(|x| serde_json::to_value(x).unwrap_or_default())),
        None => boards::scene(&state, &id)
            .map(|scene| Some(json!({ "name": b.name, "role": role, "scene": scene }))),
    })
    .await;
    match got {
        Ok(Ok(Some(v))) => Json(v).into_response(),
        Ok(Ok(None)) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(e)) => (StatusCode::NOT_FOUND, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct Change {
    #[serde(default)]
    ops: Vec<boards::Op>,
}

/// An edit link's changes to its board; a view link may not.
async fn change(
    State(s): State<Arc<Shares>>,
    headers: HeaderMap,
    Json(c): Json<Change>,
) -> Response {
    let (b, role) = match opened(&s, &headers).await {
        Ok(o) => o,
        Err(r) => return r.into_response(),
    };
    if role != Role::Edit {
        return (
            StatusCode::FORBIDDEN,
            "This link can view the board, not change it.",
        )
            .into_response();
    }
    let state = s.state.clone();
    match tokio::task::spawn_blocking(move || boards::apply(&state, &b.id, &c.ops, "link")).await {
        Ok(Ok(v)) => Json(json!({ "version": v })).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// The board as an `.excalidraw` file, for either kind of link.
async fn export(State(s): State<Arc<Shares>>, headers: HeaderMap) -> Response {
    let (b, _) = match opened(&s, &headers).await {
        Ok(o) => o,
        Err(r) => return r.into_response(),
    };
    let state = s.state.clone();
    match tokio::task::spawn_blocking(move || boards::export(&state, &b.id)).await {
        Ok(Ok((_, file))) => {
            let v: Value = file;
            ([(header::CONTENT_TYPE, "application/json")], v.to_string()).into_response()
        }
        Ok(Err(e)) => (StatusCode::NOT_FOUND, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
