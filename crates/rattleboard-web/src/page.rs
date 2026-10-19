//! The board page and what it needs: its files (the desktop's split-flap renderer and font),
//! the live stream every viewer watches, and the pairing page.
//!
//!   GET /                     the board, full screen (browsers; API clients get the API)
//!   GET /rattleboard/view     the board now, as JSON         viewer token in token mode
//!   GET /rattleboard/stream   the board on every change (SSE) viewer token in token mode
//!   GET /pair                 a page that shows the pairing QR code once given the key
//!   GET /rattleboard/pair     {"url", "svg"}: the pairing link and its QR code (key)
//!   GET /healthz              "ok"
//!
//! The viewer token travels in the page's URL fragment (`/#v=<token>`), which browsers never
//! send to the server; the page passes it on as `?v=` for the view and the stream. The API
//! key is never in any page: the pairing page asks for it and keeps it in memory only.

use crate::board::{Board, View};
use crate::config::Viewers;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const INDEX: &str = include_str!("../web/index.html");
const PAGE_JS: &str = include_str!("../web/board.js");
const PAIR: &str = include_str!("../web/pair.html");
/// The pairing page, with this build's version filled in.
static PAIR_PAGE: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| PAIR.replace("{{version}}", env!("CARGO_PKG_VERSION")));
const PAIR_JS: &str = include_str!("../web/pair.js");
// The same renderer and font the desktop app uses.
const SPLIT_FLAP: &str = include_str!("../../../src/shared/split-flap.js");
const BASE_CSS: &str = include_str!("../../../src/shared/base.css");
const FONT_500: &[u8] = include_bytes!("../../../src/fonts/barlow-semi-condensed-500.woff2");
const FONT_600: &[u8] = include_bytes!("../../../src/fonts/barlow-semi-condensed-600.woff2");
const ICON: &[u8] = include_bytes!("../../../src-tauri/icons/128x128.png");

/// More viewers than any wall of screens needs; beyond it, new ones are turned away.
const MAX_VIEWERS: usize = 64;
static VIEWERS: AtomicUsize = AtomicUsize::new(0);

const CSP: &str = "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'; script-src 'self'; base-uri 'none'; form-action 'none'";

fn html(body: &'static str) -> Response {
    let mut r = (StatusCode::OK, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    r
}

/// The board page, also what a browser gets at `/` (see `server::Board::index`).
pub fn index() -> Response {
    html(INDEX)
}

fn file(kind: &'static str, body: &'static [u8], cache: &'static str) -> Response {
    (StatusCode::OK, [(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, cache), (header::X_CONTENT_TYPE_OPTIONS, "nosniff")], body).into_response()
}

const SHORT: &str = "no-cache";
const LONG: &str = "public, max-age=604800, immutable";

pub fn routes(board: Arc<Board>) -> Router {
    Router::new()
        .route("/index.html", get(|| async { index() }))
        .route("/board.js", get(|| async { file("text/javascript; charset=utf-8", PAGE_JS.as_bytes(), SHORT) }))
        .route("/pair", get(|| async { html(PAIR_PAGE.as_str()) }))
        .route("/pair.js", get(|| async { file("text/javascript; charset=utf-8", PAIR_JS.as_bytes(), SHORT) }))
        .route("/shared/split-flap.js", get(|| async { file("text/javascript; charset=utf-8", SPLIT_FLAP.as_bytes(), SHORT) }))
        .route("/shared/base.css", get(|| async { file("text/css; charset=utf-8", BASE_CSS.as_bytes(), SHORT) }))
        .route("/fonts/barlow-semi-condensed-500.woff2", get(|| async { file("font/woff2", FONT_500, LONG) }))
        .route("/fonts/barlow-semi-condensed-600.woff2", get(|| async { file("font/woff2", FONT_600, LONG) }))
        .route("/icon.png", get(|| async { file("image/png", ICON, LONG) }))
        .route("/healthz", get(|| async { "ok" }))
        .route("/rattleboard/view", get(view))
        .route("/rattleboard/stream", get(stream))
        .route("/rattleboard/pair", get(pair))
        .with_state(board)
}

#[derive(Deserialize)]
struct Viewer {
    v: Option<String>,
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Open boards can be watched by anyone; token boards need the viewer token (or the API key).
fn may_watch(b: &Board, v: &Viewer) -> bool {
    if b.cfg.viewers == Viewers::Open {
        return true;
    }
    let given = v.v.as_deref().unwrap_or_default();
    !given.is_empty() && (b.viewer_token.as_deref().is_some_and(|t| same(given, t)) || same(given, b.api_key()))
}

fn refused() -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({ "error": "This board needs its viewer link (with #v=...)" }))).into_response()
}

async fn view(State(b): State<Arc<Board>>, Query(v): Query<Viewer>) -> Response {
    if !may_watch(&b, &v) {
        return refused();
    }
    Json((*b.view()).clone()).into_response()
}

/// Counts a viewer for as long as its stream is open.
struct Watching;
impl Drop for Watching {
    fn drop(&mut self) {
        VIEWERS.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn stream(State(b): State<Arc<Board>>, Query(v): Query<Viewer>) -> Response {
    if !may_watch(&b, &v) {
        return refused();
    }
    if VIEWERS.fetch_add(1, Ordering::Relaxed) >= MAX_VIEWERS {
        VIEWERS.fetch_sub(1, Ordering::Relaxed);
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "Too many viewers" }))).into_response();
    }
    let mut rx = b.subscribe();
    // The board as it is now first, then each change.
    rx.mark_changed();
    let events = futures_util::stream::unfold((rx, Watching), |(mut rx, w)| async move {
        rx.changed().await.ok()?;
        let v: Arc<View> = rx.borrow_and_update().clone();
        let ev = Event::default().event("board").json_data(&*v).unwrap_or_else(|_| Event::default().comment("?"));
        Some((Ok::<_, Infallible>(ev), (rx, w)))
    });
    let mut r = Sse::new(events).keep_alive(KeepAlive::new().interval(Duration::from_secs(20))).into_response();
    // Proxies (nginx, Home Assistant's ingress) shouldn't hold the stream back.
    r.headers_mut().insert("x-accel-buffering", HeaderValue::from_static("no"));
    r
}

/// The pairing link and its QR code, for a phone or another Rattleboard. Needs the key.
async fn pair(State(b): State<Arc<Board>>, h: HeaderMap) -> Response {
    let key = b.api_key();
    let given = ["x-vestaboard-local-api-key", "x-vestaboard-read-write-key", "x-vestaboard-token"]
        .iter()
        .filter_map(|n| h.get(*n).and_then(|v| v.to_str().ok()))
        .chain(h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")))
        .any(|v| same(v.trim(), key));
    if !given {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "Missing or wrong API key" }))).into_response();
    }
    // The address the browser used is one this network can reach, unless RATTLEBOARD_URL says.
    let base = b.cfg.public_url.clone().or_else(|| {
        let host = h.get(header::HOST).and_then(|v| v.to_str().ok())?;
        let proto = h.get("x-forwarded-proto").and_then(|v| v.to_str().ok()).filter(|p| *p == "https").unwrap_or("http");
        Some(format!("{proto}://{host}"))
    });
    let Some(base) = base else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Set RATTLEBOARD_URL to this board's address" }))).into_response();
    };
    let url = crate::pair_link(&b, &base);
    let mut r = Json(json!({ "url": url, "svg": crate::qr_svg(&url) })).into_response();
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}
