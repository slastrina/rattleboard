//! The built-in API server: lets other apps drive the virtual board using the same
//! requests they would send to a real Vestaboard.
//!
//! Local API style (http://host:port/local-api/message, X-Vestaboard-Local-Api-Key):
//!   GET  -> [[..codes..], ...]
//!   POST [[..codes..], ...]  or  {"characters": [[...]], "strategy": ...}
//! Read/Write and Cloud API style (http://host:port/, X-Vestaboard-Read-Write-Key or
//! X-Vestaboard-Token):
//!   GET  -> {"currentMessage": {"layout": "<json>", "id": "..."}}
//!   POST {"text": "..."}  or  {"characters": [[...]]}  or  [[...]]
//! Every style accepts the same key, which also works as `Authorization: Bearer <key>`.
//! Both POSTs take optional "strategy", "step_interval_ms" and "step_size" in the object form,
//! and `text` may carry a size hint, "rows" and "cols".
//!
//! A fixed-size board centres a layout of another size on itself; an adaptive one (the
//! desktop's "Match each message") shows every layout at its own size.
//!
//! Rattleboard-only (protocol v2 and up):
//!   GET    /rattleboard/info      -> {"app": "rattleboard", "protocol": 2, ...}
//!   POST   /rattleboard/sequence  {"name", "loop", "frames": [...]}  -> 201
//!   GET    /rattleboard/sequence  -> {"running": true, "frame": 2, ...} or {"running": false}
//!   DELETE /rattleboard/sequence  -> 204
//!
//! Sources (protocol v3), on boards that run them:
//!   GET    /rattleboard/sources                              plugins, rotation, settings
//!   PUT    /rattleboard/sources/rotation                     replaces the rotation (validated)
//!   POST   /rattleboard/sources/plugins                      installs a manifest (not command: 403)
//!   DELETE /rattleboard/sources/plugins/{id}
//!   PUT    /rattleboard/sources/instances/{id}/secrets       {"token": "..."}; never echoed
//!   GET    /rattleboard/sources/preview/{id}[?refresh=1]     {"characters": [[...]], "reason"}
//!   GET    /rattleboard/sources/upnext                       [{"at", "instanceId", "title", "reason"}]
//!   POST   /rattleboard/sources/next                         moves the rotation on now
//!
//! Agents, on boards that have them (the desktop):
//!   GET    /rattleboard/boards    -> {"boards": [{"target", "name", "kind", "rows", "cols"}], "groups": [...]}
//!   POST   /rattleboard/send      {"target": "<id>" | "group:<id>" | "all", "message": {...}} -> {"deliveries": [...]}
//!   POST   /mcp                   MCP (Streamable HTTP, JSON responses); see `mcp.rs`

use crate::sequence::{self, Sequence};
use crate::vestaboard::{self, Grid, Transition};
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, RawQuery, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{self, get};
use axum::{Json, Router};
use serde::Serialize;
use serde_json::{json, Value};
use crate::sources::{self, Refused};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::oneshot;

const KEY_HEADERS: [&str; 3] = ["x-vestaboard-local-api-key", "x-vestaboard-read-write-key", "x-vestaboard-token"];

pub struct Handle {
    stop: Option<oneshot::Sender<()>>,
    pub addr: SocketAddr,
}

impl Drop for Handle {
    fn drop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
    }
}

/// 3: sources and plugins (2 added sequences and transitions)
pub const PROTOCOL: u32 = 3;
const BODY_LIMIT: usize = 64 * 1024;
const SEQUENCE_BODY_LIMIT: usize = 256 * 1024;

#[cfg(target_os = "macos")]
pub const PLATFORM: &str = "macos";
#[cfg(target_os = "windows")]
pub const PLATFORM: &str = "windows";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const PLATFORM: &str = "linux";

/// A running sequence, as GET /rattleboard/sequence reports it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SequenceStatus {
    pub id: String,
    pub name: String,
    /// 1-based
    pub frame: usize,
    pub frames: usize,
    #[serde(rename = "loop")]
    pub looping: bool,
}

/// What the server needs from the app; implemented in lib.rs.
pub trait Board: Send + Sync + 'static {
    fn api_key(&self) -> String;
    /// The board's size, or for an adaptive board the size text is laid out at by default
    fn dims(&self) -> (usize, usize);
    /// Shows each message at its own size instead of centring it on `dims`
    fn adaptive(&self) -> bool;
    fn name(&self) -> String;
    fn current(&self) -> (Grid, Option<String>);
    /// A normal message. It also stops a running sequence.
    fn post(&self, layout: Grid, tr: Transition) -> String;
    /// Starts a sequence in place of any running one; returns its id.
    fn play(&self, seq: Sequence) -> String;
    fn sequence(&self) -> Option<SequenceStatus>;
    fn stop(&self);
    /// The sources engine, on boards that run one
    fn sources(&self) -> Option<Arc<sources::Engine>> {
        None
    }
    /// The time and this board's size, for previews and Up next
    fn sources_ctx(&self) -> Option<sources::Ctx> {
        None
    }
    /// What `/rattleboard/info` reports as "platform"
    fn platform(&self) -> &'static str {
        PLATFORM
    }
    /// A page for a browser that opens `/` without a key (Rattleboard Web's board). API
    /// clients, which send a key, still get the Read/Write API's answer there.
    fn index(&self) -> Option<Response> {
        None
    }
    /// This board, saved boards and groups, for `GET /rattleboard/boards`; None answers 404
    fn boards(&self) -> Option<Value> {
        None
    }
    /// Sends a message (a POST body) to a saved board or group, as Compose does; None
    /// answers 404
    fn send_to(&self, _target: String, _message: Value) -> Option<SendFut> {
        None
    }
}

/// How a send to saved boards went: {"deliveries": [{"target", "name", "status", "error"}]}
pub type SendFut = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, String>> + Send>>;

#[derive(Clone)]
struct Ctx {
    board: std::sync::Arc<dyn Board>,
}

pub async fn start(board: std::sync::Arc<dyn Board>, port: u16, lan: bool) -> Result<Handle, String> {
    serve(router(board, Router::new()), port, lan).await
}

/// Every API route for `board`, plus `extra` routes of the caller's own (Rattleboard Web's
/// page and live stream); requests to anything else answer 404.
pub fn router(board: std::sync::Arc<dyn Board>, extra: Router) -> Router {
    let ctx = Ctx { board };
    Router::new()
        .route("/local-api/message", get(local_get).post(post).options(preflight))
        .route("/", get(cloud_get).post(post).options(preflight))
        .route("/rattleboard/info", get(info).options(preflight))
        .route(
            "/rattleboard/sequence",
            get(sequence_get)
                .post(sequence_post)
                .delete(sequence_delete)
                .options(preflight)
                .layer(DefaultBodyLimit::max(SEQUENCE_BODY_LIMIT)),
        )
        .route("/rattleboard/sources", get(sources_get).options(preflight))
        .route("/rattleboard/sources/rotation", routing::put(rotation_put).options(preflight))
        .route("/rattleboard/sources/plugins", routing::post(plugin_post).options(preflight))
        .route("/rattleboard/sources/plugins/{id}", routing::delete(plugin_delete).options(preflight))
        .route("/rattleboard/sources/instances/{id}/secrets", routing::put(secrets_put).options(preflight))
        .route("/rattleboard/sources/preview/{id}", get(preview_get).options(preflight))
        .route("/rattleboard/sources/upnext", get(upnext_get).options(preflight))
        .route("/rattleboard/sources/next", routing::post(next_post).options(preflight))
        .route("/rattleboard/boards", get(boards_get).options(preflight))
        .route("/rattleboard/send", routing::post(send_post).options(preflight))
        .route("/mcp", routing::post(mcp_post).get(mcp_other).delete(mcp_other).options(preflight))
        .with_state(ctx)
        .merge(extra)
        .fallback(|| async { err(StatusCode::NOT_FOUND, "Not found") })
        .layer(axum::middleware::map_response(cors))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
}

/// Serves `router` on `port`, on every interface (`lan`) or only this computer's.
pub async fn serve(router: Router, port: u16, lan: bool) -> Result<Handle, String> {
    let ip = if lan { [0, 0, 0, 0] } else { [127, 0, 0, 1] };
    let listener = tokio::net::TcpListener::bind(SocketAddr::from((ip, port))).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            if port == 7000 {
                "Port 7000 is in use (AirPlay receivers use it); pick another port".to_string()
            } else {
                format!("Port {port} is already in use; pick another")
            }
        } else {
            format!("Couldn't open port {port}: {e}")
        }
    })?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let (tx, rx) = oneshot::channel::<()>();
    // Spawned on the runtime of whoever awaits this (Tauri's, on the desktop).
    tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
    });
    Ok(Handle { stop: Some(tx), addr })
}

fn err(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({ "error": msg }))).into_response()
}

/// Constant-time comparison so the key can't be guessed byte by byte from timings.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn authorized(ctx: &Ctx, h: &HeaderMap) -> bool {
    let key = ctx.board.api_key();
    if key.is_empty() {
        return false;
    }
    let bearer = h
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    KEY_HEADERS
        .iter()
        .filter_map(|n| h.get(*n).and_then(|v| v.to_str().ok()))
        .chain(bearer)
        .any(|v| same(v.trim(), &key))
}

/// Whether the request carries a key at all (right or wrong).
fn sends_key(h: &HeaderMap) -> bool {
    KEY_HEADERS.iter().any(|n| h.contains_key(*n)) || h.contains_key(header::AUTHORIZATION)
}

fn wants_html(h: &HeaderMap) -> bool {
    h.get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|a| a.contains("text/html"))
}

async fn local_get(State(ctx): State<Ctx>, h: HeaderMap) -> Response {
    if !authorized(&ctx, &h) {
        return err(StatusCode::UNAUTHORIZED, "Missing or wrong API key");
    }
    Json(ctx.board.current().0).into_response()
}

async fn cloud_get(State(ctx): State<Ctx>, h: HeaderMap) -> Response {
    // A browser opening the board's address, with no key: the board's own page, if it has one.
    if !sends_key(&h) && wants_html(&h) {
        if let Some(page) = ctx.board.index() {
            return page;
        }
    }
    if !authorized(&ctx, &h) {
        return err(StatusCode::UNAUTHORIZED, "Missing or wrong API key");
    }
    let (layout, id) = ctx.board.current();
    Json(json!({
        "currentMessage": {
            "layout": serde_json::to_string(&layout).unwrap_or_default(),
            "id": id,
        }
    }))
    .into_response()
}

async fn post(State(ctx): State<Ctx>, h: HeaderMap, body: Bytes) -> Response {
    if !authorized(&ctx, &h) {
        return err(StatusCode::UNAUTHORIZED, "Missing or wrong API key");
    }
    let Ok(v) = serde_json::from_slice::<Value>(&body) else {
        return err(StatusCode::BAD_REQUEST, "Body must be JSON");
    };
    match message(&v, ctx.board.dims(), ctx.board.adaptive()) {
        Ok((g, tr)) => {
            let id = ctx.board.post(g, tr);
            (StatusCode::CREATED, Json(json!({ "status": "ok", "id": id, "created": crate::now_ms() }))).into_response()
        }
        Err(e) => err(StatusCode::BAD_REQUEST, &e),
    }
}

/// A posted message: a layout array, {"characters": ...} or {"text": ...}, with an optional
/// transition, for a rows×cols board. A fixed board centres a layout of another size and lays
/// text out at its own size; an adaptive one keeps the layout's size and follows the text's hint.
pub fn message(v: &Value, (rows, cols): (usize, usize), adaptive: bool) -> Result<(Grid, Transition), String> {
    const HOW: &str = "Send {\"text\": \"...\"}, {\"characters\": [[...]]} or a layout array";
    let fit = |g: Grid| if adaptive { g } else { vestaboard::fit_center(&g, rows, cols) };
    match v {
        Value::Array(_) => Ok((fit(vestaboard::posted_grid(v)?), Transition::default())),
        Value::Object(o) => {
            let layout = match (o.get("characters"), o.get("text")) {
                (Some(c), _) => fit(vestaboard::posted_grid(c)?),
                (None, Some(Value::String(t))) => {
                    let hint = size_hint(o)?;
                    let (r, c) = if adaptive { hint.unwrap_or((rows, cols)) } else { (rows, cols) };
                    vestaboard::text_layout(t, r, c)
                }
                _ => return Err(HOW.into()),
            };
            Ok((layout, Transition::parse(o)?))
        }
        _ => Err(HOW.into()),
    }
}

/// The optional "rows" and "cols" a text post asks to be laid out at: 2–12 × 8–40.
pub fn size_hint(o: &serde_json::Map<String, Value>) -> Result<Option<(usize, usize)>, String> {
    let (rows, cols) = (o.get("rows").filter(|v| !v.is_null()), o.get("cols").filter(|v| !v.is_null()));
    if rows.is_none() && cols.is_none() {
        return Ok(None);
    }
    let num = |v: Option<&Value>, (lo, hi): (usize, usize), what: &str| match v.and_then(|v| v.as_u64()) {
        Some(n) if (lo as u64..=hi as u64).contains(&n) => Ok(n as usize),
        _ => Err(format!("A size hint needs {what} from {lo} to {hi}")),
    };
    Ok(Some((num(rows, crate::settings::CUSTOM_ROWS, "rows")?, num(cols, crate::settings::CUSTOM_COLS, "cols")?)))
}

async fn info(State(ctx): State<Ctx>, h: HeaderMap) -> Response {
    if !authorized(&ctx, &h) {
        return err(StatusCode::UNAUTHORIZED, "Missing or wrong API key");
    }
    // An adaptive board reports the size it's showing, else the one it defaults to.
    let shown = ctx.board.current().0;
    let (rows, cols) = if ctx.board.adaptive() && !shown.is_empty() { vestaboard::dims_of(&shown) } else { ctx.board.dims() };
    Json(json!({
        "app": "rattleboard",
        "protocol": PROTOCOL,
        "version": env!("CARGO_PKG_VERSION"),
        "platform": ctx.board.platform(),
        "name": ctx.board.name(),
        "board": { "rows": rows, "cols": cols },
        "adaptive": ctx.board.adaptive(),
        "features": features(&ctx),
    }))
    .into_response()
}

fn features(ctx: &Ctx) -> Vec<&'static str> {
    let mut f = vec!["sequence", "strategy"];
    if ctx.board.sources().is_some() {
        f.push("sources");
    }
    f
}

fn refused(r: Refused) -> Response {
    err(StatusCode::from_u16(r.0).unwrap_or(StatusCode::BAD_REQUEST), &r.1)
}

/// The engine, once the key is checked; a board without sources answers 404.
fn engine(ctx: &Ctx, h: &HeaderMap) -> Result<Arc<sources::Engine>, Box<Response>> {
    if !authorized(ctx, h) {
        return Err(Box::new(err(StatusCode::UNAUTHORIZED, "Missing or wrong API key")));
    }
    ctx.board.sources().ok_or_else(|| Box::new(err(StatusCode::NOT_FOUND, "This board doesn't run sources")))
}

fn json_body(body: &Bytes) -> Result<Value, Box<Response>> {
    serde_json::from_slice::<Value>(body).map_err(|_| Box::new(err(StatusCode::BAD_REQUEST, "Body must be JSON")))
}

async fn sources_get(State(ctx): State<Ctx>, h: HeaderMap) -> Response {
    match engine(&ctx, &h) {
        Ok(e) => Json(e.view()).into_response(),
        Err(r) => *r,
    }
}

async fn rotation_put(State(ctx): State<Ctx>, h: HeaderMap, body: Bytes) -> Response {
    let e = match engine(&ctx, &h) {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let v = match json_body(&body) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match e.put_rotation(&v, true) {
        Ok(()) => Json(e.view()).into_response(),
        Err(r) => refused(r),
    }
}

async fn plugin_post(State(ctx): State<Ctx>, h: HeaderMap, body: Bytes) -> Response {
    let e = match engine(&ctx, &h) {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let v = match json_body(&body) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match e.install(&v, true) {
        Ok(id) => (StatusCode::CREATED, Json(json!({ "status": "ok", "id": id }))).into_response(),
        Err(r) => refused(r),
    }
}

async fn plugin_delete(State(ctx): State<Ctx>, h: HeaderMap, Path(id): Path<String>) -> Response {
    match engine(&ctx, &h).map(|e| e.uninstall(&id)) {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(r)) => refused(r),
        Err(r) => *r,
    }
}

async fn secrets_put(State(ctx): State<Ctx>, h: HeaderMap, Path(id): Path<String>, body: Bytes) -> Response {
    let e = match engine(&ctx, &h) {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let v = match json_body(&body) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    // Stored and never sent back, not even in this answer.
    match e.set_secrets(&id, &v) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(r) => refused(r),
    }
}

async fn preview_get(State(ctx): State<Ctx>, h: HeaderMap, Path(id): Path<String>, RawQuery(q): RawQuery) -> Response {
    // ?refresh=1 fetches the source's data again first (at most every few seconds).
    let refresh = q.unwrap_or_default().split('&').any(|kv| matches!(kv, "refresh=1" | "refresh=true" | "refresh"));
    let e = match engine(&ctx, &h) {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let Some(c) = ctx.board.sources_ctx() else { return err(StatusCode::NOT_FOUND, "This board doesn't run sources") };
    match e.preview(&id, &c, true, refresh).await {
        Ok((pages, reason)) => Json(json!({ "characters": pages[0], "pages": pages, "reason": reason })).into_response(),
        Err(r) => refused(r),
    }
}

async fn upnext_get(State(ctx): State<Ctx>, h: HeaderMap) -> Response {
    let e = match engine(&ctx, &h) {
        Ok(e) => e,
        Err(r) => return *r,
    };
    let Some(c) = ctx.board.sources_ctx() else { return err(StatusCode::NOT_FOUND, "This board doesn't run sources") };
    Json(e.up_next(&c)).into_response()
}

async fn next_post(State(ctx): State<Ctx>, h: HeaderMap) -> Response {
    match engine(&ctx, &h) {
        Ok(e) => {
            e.request_next();
            Json(json!({ "status": "ok" })).into_response()
        }
        Err(r) => *r,
    }
}

async fn sequence_post(State(ctx): State<Ctx>, h: HeaderMap, body: Bytes) -> Response {
    if !authorized(&ctx, &h) {
        return err(StatusCode::UNAUTHORIZED, "Missing or wrong API key");
    }
    let Ok(v) = serde_json::from_slice::<Value>(&body) else {
        return err(StatusCode::BAD_REQUEST, "Body must be JSON");
    };
    match sequence::parse(&v, ctx.board.dims(), ctx.board.adaptive()) {
        Ok(seq) => {
            let frames = seq.frames.len();
            let id = ctx.board.play(seq);
            (StatusCode::CREATED, Json(json!({ "status": "ok", "id": id, "frames": frames }))).into_response()
        }
        Err(e) => err(StatusCode::BAD_REQUEST, &e),
    }
}

async fn sequence_get(State(ctx): State<Ctx>, h: HeaderMap) -> Response {
    if !authorized(&ctx, &h) {
        return err(StatusCode::UNAUTHORIZED, "Missing or wrong API key");
    }
    match ctx.board.sequence() {
        Some(s) => {
            let mut v = json!(s);
            v["running"] = json!(true);
            Json(v).into_response()
        }
        None => Json(json!({ "running": false })).into_response(),
    }
}

async fn sequence_delete(State(ctx): State<Ctx>, h: HeaderMap) -> Response {
    if !authorized(&ctx, &h) {
        return err(StatusCode::UNAUTHORIZED, "Missing or wrong API key");
    }
    ctx.board.stop();
    StatusCode::NO_CONTENT.into_response()
}

async fn preflight() -> StatusCode {
    StatusCode::NO_CONTENT
}

/// Lets browser-based apps call the server; requests still need the key.
async fn cors(method: Method, mut res: Response) -> Response {
    let h = res.headers_mut();
    h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static(
            "content-type, authorization, x-vestaboard-local-api-key, x-vestaboard-read-write-key, x-vestaboard-token, mcp-protocol-version, mcp-session-id",
        ),
    );
    h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, PUT, DELETE, OPTIONS"));
    if method == Method::OPTIONS {
        h.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
    }
    res
}

// ---------------------------------------------------------------- agents

async fn boards_get(State(ctx): State<Ctx>, h: HeaderMap) -> Response {
    if !authorized(&ctx, &h) {
        return err(StatusCode::UNAUTHORIZED, "Missing or wrong API key");
    }
    match ctx.board.boards() {
        Some(v) => Json(v).into_response(),
        None => err(StatusCode::NOT_FOUND, "Not found"),
    }
}

async fn send_post(State(ctx): State<Ctx>, h: HeaderMap, body: Bytes) -> Response {
    if !authorized(&ctx, &h) {
        return err(StatusCode::UNAUTHORIZED, "Missing or wrong API key");
    }
    let v = match json_body(&body) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let (Some(target), Some(message)) = (v.get("target").and_then(|t| t.as_str()), v.get("message")) else {
        return err(StatusCode::BAD_REQUEST, "Send {\"target\": \"...\", \"message\": {...}}");
    };
    match ctx.board.send_to(target.to_string(), message.clone()) {
        None => err(StatusCode::NOT_FOUND, "Not found"),
        Some(f) => match f.await {
            Ok(v) => Json(v).into_response(),
            Err(e) => err(StatusCode::BAD_REQUEST, &e),
        },
    }
}

/// MCP clients aren't browsers: a page from another site may not drive the board's tools
/// (DNS rebinding), even with the key.
fn local_origin(h: &HeaderMap) -> bool {
    let Some(o) = h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else { return true };
    let host = url::Url::parse(o).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]")
}

/// MCP's Streamable HTTP transport, answered with plain JSON: a request gets 200 and its
/// answer, a notification 202.
async fn mcp_post(State(ctx): State<Ctx>, h: HeaderMap, body: Bytes) -> Response {
    if !local_origin(&h) {
        return err(StatusCode::FORBIDDEN, "Cross-site requests can't use MCP");
    }
    if !authorized(&ctx, &h) {
        return err(StatusCode::UNAUTHORIZED, "Missing or wrong API key");
    }
    if let Some(v) = h.get("mcp-protocol-version").and_then(|v| v.to_str().ok()) {
        if !crate::mcp::supported(v) {
            return err(StatusCode::BAD_REQUEST, "Unsupported MCP-Protocol-Version");
        }
    }
    match crate::mcp::handle_text(&ctx, &String::from_utf8_lossy(&body)).await {
        Some(answer) => Json(answer).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// No server-sent events or sessions here.
async fn mcp_other() -> Response {
    let mut r = err(StatusCode::METHOD_NOT_ALLOWED, "Use POST");
    r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("POST, OPTIONS"));
    r
}

/// The board in-process, for `/mcp`.
impl crate::mcp::Backend for Ctx {
    fn info(&self) -> crate::mcp::Fut<'_, Result<Value, String>> {
        Box::pin(async move {
            let shown = self.board.current().0;
            let adaptive = self.board.adaptive();
            let (rows, cols) = if adaptive && !shown.is_empty() { vestaboard::dims_of(&shown) } else { self.board.dims() };
            Ok(json!({ "name": self.board.name(), "board": { "rows": rows, "cols": cols }, "adaptive": adaptive }))
        })
    }
    fn current(&self) -> crate::mcp::Fut<'_, Result<Grid, String>> {
        Box::pin(async move { Ok(self.board.current().0) })
    }
    fn post(&self, body: Value) -> crate::mcp::Fut<'_, Result<(), String>> {
        Box::pin(async move {
            let (g, tr) = message(&body, self.board.dims(), self.board.adaptive())?;
            self.board.post(g, tr);
            Ok(())
        })
    }
    fn play(&self, seq: Value) -> crate::mcp::Fut<'_, Result<(), String>> {
        Box::pin(async move {
            let seq = sequence::parse(&seq, self.board.dims(), self.board.adaptive())?;
            self.board.play(seq);
            Ok(())
        })
    }
    fn boards(&self) -> crate::mcp::Fut<'_, Result<Option<Value>, String>> {
        Box::pin(async move { Ok(self.board.boards()) })
    }
    fn send_to(&self, target: String, body: Value) -> crate::mcp::Fut<'_, Result<Value, String>> {
        Box::pin(async move {
            match self.board.send_to(target, body) {
                Some(f) => f.await,
                None => Err("This board has no saved boards".into()),
            }
        })
    }
}

/// The address other devices on the network would use, found without sending anything.
pub fn lan_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("10.255.255.255:1").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

pub fn new_key() -> String {
    let mut b = [0u8; 24];
    getrandom::fill(&mut b).expect("OS random source");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    /// An absolute program path on this platform ("/usr/bin/git" isn't absolute on Windows).
    const GIT: &str = if cfg!(windows) { "C:\\Program Files\\Git\\cmd\\git.exe" } else { "/usr/bin/git" };
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Fake {
        size: (usize, usize),
        adaptive: bool,
        grid: Mutex<Grid>,
        tr: Mutex<Transition>,
        seq: Mutex<Option<(Sequence, SequenceStatus)>>,
        engine: Option<Arc<sources::Engine>>,
    }
    impl Board for Fake {
        fn api_key(&self) -> String {
            "k1".into()
        }
        fn dims(&self) -> (usize, usize) {
            self.size
        }
        fn adaptive(&self) -> bool {
            self.adaptive
        }
        fn name(&self) -> String {
            "Desk".into()
        }
        fn current(&self) -> (Grid, Option<String>) {
            (self.grid.lock().unwrap().clone(), Some("m1".into()))
        }
        fn post(&self, layout: Grid, tr: Transition) -> String {
            *self.grid.lock().unwrap() = layout;
            *self.tr.lock().unwrap() = tr;
            self.stop();
            "m2".into()
        }
        fn play(&self, seq: Sequence) -> String {
            let st = SequenceStatus { id: "seq-1".into(), name: seq.name.clone(), frame: 1, frames: seq.frames.len(), looping: seq.looping };
            *self.seq.lock().unwrap() = Some((seq, st));
            "seq-1".into()
        }
        fn sequence(&self) -> Option<SequenceStatus> {
            self.seq.lock().unwrap().as_ref().map(|s| s.1.clone())
        }
        fn stop(&self) {
            *self.seq.lock().unwrap() = None;
        }
        fn sources(&self) -> Option<Arc<sources::Engine>> {
            self.engine.clone()
        }
        fn sources_ctx(&self) -> Option<sources::Ctx> {
            self.engine.as_ref().map(|_| sources::tests::ctx(0))
        }
    }

    fn fake() -> Arc<Fake> {
        sized(3, 15)
    }

    fn sized(rows: usize, cols: usize) -> Arc<Fake> {
        let f = Fake { size: (rows, cols), ..Default::default() };
        *f.grid.lock().unwrap() = vec![vec![0; cols]; rows];
        Arc::new(f)
    }

    #[tokio::test]
    async fn endpoints() {
        let board = fake();
        let h = start(board.clone(), 0, false).await.unwrap();
        let base = format!("http://{}", h.addr);
        let c = reqwest::Client::new();
        let msg = format!("{base}/local-api/message");

        assert_eq!(c.get(&msg).send().await.unwrap().status(), 401);
        assert_eq!(c.get(&msg).header("X-Vestaboard-Local-Api-Key", "nope").send().await.unwrap().status(), 401);

        let r = c.post(&msg).header("X-Vestaboard-Local-Api-Key", "k1").body(r#"{"text":"hi there"}"#).send().await.unwrap();
        assert_eq!(r.status(), 201);
        assert_eq!(board.grid.lock().unwrap()[1][3..11], vestaboard::text_layout("hi there", 3, 15)[1][3..11]);

        let g: Grid = c.get(&msg).header("X-Vestaboard-Local-Api-Key", "k1").send().await.unwrap().json().await.unwrap();
        assert_eq!(g, *board.grid.lock().unwrap());

        let layout = vec![vec![5u8; 15]; 3];
        let r = c.post(&base).header("X-Vestaboard-Read-Write-Key", "k1").json(&layout).send().await.unwrap();
        assert_eq!(r.status(), 201);
        let v: Value = c.get(&base).header("X-Vestaboard-Token", "k1").send().await.unwrap().json().await.unwrap();
        let s = v["currentMessage"]["layout"].as_str().unwrap();
        assert_eq!(serde_json::from_str::<Grid>(s).unwrap(), layout);
        // A bigger layout is cropped from its centre to fit.
        let r = c.post(&base).bearer_auth("k1").json(&vec![vec![7u8; 22]; 6]).send().await.unwrap();
        assert_eq!(r.status(), 201);
        assert_eq!(*board.grid.lock().unwrap(), vec![vec![7u8; 15]; 3]);

        let r = c.post(&base).bearer_auth("k1").body(r#"{"characters":[[1,2,3],[1]]}"#).send().await.unwrap();
        assert_eq!(r.status(), 400);
        let r = c.request(Method::OPTIONS, &base).send().await.unwrap();
        assert_eq!(r.headers()["access-control-allow-origin"], "*");
        assert!(r.headers()["access-control-allow-methods"].to_str().unwrap().contains("DELETE"));
        drop(h);
    }

    /// Layouts of any size within 12×40 are centred on the board; anything else is a 400.
    #[tokio::test]
    async fn sizes() {
        let board = sized(6, 22);
        let h = start(board.clone(), 0, false).await.unwrap();
        let c = reqwest::Client::new();
        let msg = format!("http://{}/local-api/message", h.addr);
        let post = |body: Value| c.post(&msg).header("X-Vestaboard-Local-Api-Key", "k1").json(&body).send();

        // Compact on Standard: centred, blanks around it
        assert_eq!(post(json!({"characters": vec![vec![1u8; 15]; 3]})).await.unwrap().status(), 201);
        let g = board.grid.lock().unwrap().clone();
        assert_eq!((g.len(), g[0].len()), (6, 22));
        assert_eq!((g[0].clone(), g[1][2], g[1][3], g[3][17], g[3][18], g[4][10]), (vec![0; 22], 0, 1, 1, 0, 0));
        // a raw array, and a single tile, are centred too
        assert_eq!(post(json!([[2]])).await.unwrap().status(), 201);
        assert_eq!(board.grid.lock().unwrap()[2][10], 2);

        for bad in [
            json!({"characters": vec![vec![1u8; 22]; 13]}),
            json!({"characters": vec![vec![1u8; 41]; 6]}),
            json!({"characters": [[1, 2, 3], [1, 2]]}),
            json!({"characters": []}),
            json!({"characters": [[]]}),
            json!([[1, 2], [3]]),
        ] {
            let r = post(bad.clone()).await.unwrap();
            assert_eq!(r.status(), 400, "{bad}");
        }
        drop(h);

        // A custom Large board (8×30) reports its size and lays text out across it.
        let board = sized(8, 30);
        let h = start(board.clone(), 0, false).await.unwrap();
        let base = format!("http://{}", h.addr);
        let v: Value = c.get(format!("{base}/rattleboard/info")).bearer_auth("k1").send().await.unwrap().json().await.unwrap();
        assert_eq!(v["board"], json!({"rows": 8, "cols": 30}));
        let text = "a long line of words that wraps on a wide board";
        let r = c.post(format!("{base}/local-api/message")).bearer_auth("k1").json(&json!({"text": text})).send().await.unwrap();
        assert_eq!(r.status(), 201);
        assert_eq!(*board.grid.lock().unwrap(), vestaboard::text_layout(text, 8, 30));
        let r = c.post(format!("{base}/local-api/message")).bearer_auth("k1").json(&vec![vec![3u8; 30]; 8]).send().await.unwrap();
        assert_eq!(r.status(), 201);
        assert_eq!(*board.grid.lock().unwrap(), vec![vec![3u8; 30]; 8]);
        let seq = json!({"frames": [{"characters": vec![vec![1u8; 22]; 6], "hold": 2}]});
        let r = c.post(format!("{base}/rattleboard/sequence")).bearer_auth("k1").json(&seq).send().await.unwrap();
        assert_eq!(r.status(), 201);
        let f = &board.seq.lock().unwrap().clone().unwrap().0.frames[0].layout;
        assert_eq!((f.len(), f[0].len(), f[0][3], f[1][4]), (8, 30, 0, 1));
        drop(h);
    }

    /// An adaptive board shows each layout at its own size and follows a text post's size hint;
    /// a fixed one centres and ignores the hint.
    #[tokio::test]
    async fn adaptive() {
        let f = Fake { size: (6, 22), adaptive: true, ..Default::default() };
        let board = Arc::new(f);
        let h = start(board.clone(), 0, false).await.unwrap();
        let c = reqwest::Client::new();
        let base = format!("http://{}", h.addr);
        let msg = format!("{base}/local-api/message");
        let post = |body: Value| c.post(&msg).bearer_auth("k1").json(&body).send();

        assert_eq!(post(json!({"characters": vec![vec![1u8; 15]; 3]})).await.unwrap().status(), 201);
        assert_eq!(*board.grid.lock().unwrap(), vec![vec![1u8; 15]; 3]);
        let v: Value = c.get(format!("{base}/rattleboard/info")).bearer_auth("k1").send().await.unwrap().json().await.unwrap();
        assert_eq!((v["adaptive"].as_bool(), v["board"].clone()), (Some(true), json!({"rows": 3, "cols": 15})));

        assert_eq!(post(json!({"text": "hello", "rows": 8, "cols": 30})).await.unwrap().status(), 201);
        assert_eq!(*board.grid.lock().unwrap(), vestaboard::text_layout("hello", 8, 30));
        // no hint: the board's default size
        assert_eq!(post(json!({"text": "hello"})).await.unwrap().status(), 201);
        assert_eq!(*board.grid.lock().unwrap(), vestaboard::text_layout("hello", 6, 22));
        for bad in [
            json!({"text": "a", "rows": 1, "cols": 15}),
            json!({"text": "a", "rows": 13, "cols": 15}),
            json!({"text": "a", "rows": 3, "cols": 7}),
            json!({"text": "a", "rows": 3, "cols": 41}),
            json!({"text": "a", "rows": 3}),
            json!({"text": "a", "rows": "3", "cols": 15}),
        ] {
            assert_eq!(post(bad.clone()).await.unwrap().status(), 400, "{bad}");
        }
        // sequence frames can each have their own size
        let seq = json!({"frames": [
            {"characters": vec![vec![2u8; 15]; 3], "hold": 1},
            {"text": "big", "rows": 8, "cols": 30, "hold": 1},
            {"text": "default", "hold": 1}
        ]});
        let r = c.post(format!("{base}/rattleboard/sequence")).bearer_auth("k1").json(&seq).send().await.unwrap();
        assert_eq!(r.status(), 201);
        let frames = board.seq.lock().unwrap().clone().unwrap().0.frames;
        let sizes: Vec<_> = frames.iter().map(|f| vestaboard::dims_of(&f.layout)).collect();
        assert_eq!(sizes, [(3, 15), (8, 30), (6, 22)]);
        let bad = json!({"frames": [{"text": "a", "rows": 20, "cols": 15, "hold": 1}]});
        assert_eq!(c.post(format!("{base}/rattleboard/sequence")).bearer_auth("k1").json(&bad).send().await.unwrap().status(), 400);
        drop(h);

        // A fixed board ignores the hint and lays text out at its own size.
        let board = fake();
        let h = start(board.clone(), 0, false).await.unwrap();
        let r = c.post(format!("http://{}/local-api/message", h.addr)).bearer_auth("k1").json(&json!({"text": "hello", "rows": 8, "cols": 30})).send().await.unwrap();
        assert_eq!(r.status(), 201);
        assert_eq!(*board.grid.lock().unwrap(), vestaboard::text_layout("hello", 3, 15));
        let v: Value = c.get(format!("http://{}/rattleboard/info", h.addr)).bearer_auth("k1").send().await.unwrap().json().await.unwrap();
        assert_eq!(v["adaptive"], false);
        drop(h);
    }

    #[tokio::test]
    async fn strategies() {
        let board = fake();
        let h = start(board.clone(), 0, false).await.unwrap();
        let c = reqwest::Client::new();
        let msg = format!("http://{}/local-api/message", h.addr);
        let post = |body: Value| c.post(&msg).header("X-Vestaboard-Local-Api-Key", "k1").json(&body).send();

        let r = post(json!({"characters": vec![vec![1u8; 15]; 3], "strategy": "edges-to-center", "step_interval_ms": 40, "step_size": 2})).await.unwrap();
        assert_eq!(r.status(), 201);
        assert_eq!(*board.tr.lock().unwrap(), Transition { strategy: "edges-to-center".into(), step_interval_ms: 40, step_size: 2 });
        assert_eq!(post(json!({"text": "hi", "strategy": "random"})).await.unwrap().status(), 201);
        assert_eq!(board.tr.lock().unwrap().strategy, "random");
        assert_eq!(post(json!({"text": "hi"})).await.unwrap().status(), 201);
        assert!(board.tr.lock().unwrap().classic());

        for bad in [
            json!({"text": "hi", "strategy": "wipe"}),
            json!({"text": "hi", "strategy": null, "step_interval_ms": 0}),
            json!({"text": "hi", "step_interval_ms": 5001}),
            json!({"text": "hi", "step_size": 0}),
            json!({"text": "hi", "step_size": 41}),
            json!({"text": "hi", "step_size": "2"}),
        ] {
            let r = post(bad.clone()).await.unwrap();
            assert_eq!(r.status(), 400, "{bad}");
            assert!(r.json::<Value>().await.unwrap()["error"].is_string());
        }
        drop(h);
    }

    #[tokio::test]
    async fn rattleboard_routes() {
        let board = fake();
        let h = start(board.clone(), 0, false).await.unwrap();
        let base = format!("http://{}/rattleboard", h.addr);
        let c = reqwest::Client::new();
        let key = |r: reqwest::RequestBuilder| r.header("X-Vestaboard-Local-Api-Key", "k1");

        assert_eq!(c.get(format!("{base}/info")).send().await.unwrap().status(), 401);
        let v: Value = key(c.get(format!("{base}/info"))).send().await.unwrap().json().await.unwrap();
        assert_eq!((v["app"].as_str(), v["protocol"].as_u64(), v["name"].as_str()), (Some("rattleboard"), Some(3), Some("Desk")));
        assert_eq!((v["board"]["rows"].as_u64(), v["board"]["cols"].as_u64()), (Some(3), Some(15)));
        assert_eq!(v["platform"], PLATFORM);
        assert_eq!(v["features"], json!(["sequence", "strategy"]));

        let seq = format!("{base}/sequence");
        for r in [c.get(&seq), c.post(&seq).body("{}"), c.delete(&seq)] {
            assert_eq!(r.send().await.unwrap().status(), 401);
        }
        let v: Value = key(c.get(&seq)).send().await.unwrap().json().await.unwrap();
        assert_eq!(v, json!({"running": false}));

        let body = json!({"name": "Morning", "loop": true, "frames": [
            {"characters": vec![vec![2u8; 15]; 3], "hold": 8, "strategy": "column", "step_interval_ms": 120, "step_size": 1},
            {"text": "Coffee is ready", "hold": 5}
        ]});
        let r = key(c.post(&seq)).json(&body).send().await.unwrap();
        assert_eq!(r.status(), 201);
        let v: Value = r.json().await.unwrap();
        assert_eq!((v["status"].as_str(), v["id"].as_str(), v["frames"].as_u64()), (Some("ok"), Some("seq-1"), Some(2)));
        let played = board.seq.lock().unwrap().clone().unwrap().0;
        assert_eq!(played.frames[1].layout, vestaboard::text_layout("Coffee is ready", 3, 15));
        assert_eq!(played.frames[0].tr.strategy, "column");

        let v: Value = key(c.get(&seq)).send().await.unwrap().json().await.unwrap();
        assert_eq!(v, json!({"running": true, "id": "seq-1", "name": "Morning", "frame": 1, "frames": 2, "loop": true}));
        assert_eq!(key(c.delete(&seq)).send().await.unwrap().status(), 204);
        assert!(board.sequence().is_none());

        // A normal message stops a running sequence.
        assert_eq!(key(c.post(&seq)).json(&body).send().await.unwrap().status(), 201);
        let msg = format!("http://{}/local-api/message", h.addr);
        assert_eq!(key(c.post(&msg)).body(r#"{"text":"stop"}"#).send().await.unwrap().status(), 201);
        assert!(board.sequence().is_none());

        let frame = |f: Value| json!({"frames": [f]});
        for bad in [
            json!("nope"),
            json!({"frames": []}),
            json!({"frames": vec![json!({"text": "a", "hold": 1}); 101]}),
            frame(json!({"text": "a"})),
            frame(json!({"text": "a", "hold": 0})),
            frame(json!({"text": "a", "hold": 3601})),
            frame(json!({"hold": 3})),
            frame(json!({"characters": [[1, 2], [3]], "hold": 3})),
            frame(json!({"characters": vec![vec![1u8; 15]; 13], "hold": 3})),
            frame(json!({"characters": vec![vec![72u8; 15]; 3], "hold": 3})),
            frame(json!({"text": "a", "hold": 3, "strategy": "wipe"})),
            frame(json!({"text": "a", "hold": 3, "step_size": 99})),
            json!({"loop": 1, "frames": [{"text": "a", "hold": 3}]}),
        ] {
            let r = key(c.post(&seq)).json(&bad).send().await.unwrap();
            assert_eq!(r.status(), 400, "{bad}");
            assert!(r.json::<Value>().await.unwrap()["error"].is_string());
        }
        let r = key(c.post(&seq)).body("{not json").send().await.unwrap();
        assert_eq!(r.status(), 400);

        // This route takes up to 256 KB; the others stay at 64 KB.
        let big = json!({"frames": vec![json!({"text": "A ".repeat(600), "hold": 1}); 100]});
        let n = serde_json::to_vec(&big).unwrap().len();
        assert!(n > 64 * 1024 && n < 256 * 1024, "{n}");
        assert_eq!(key(c.post(&seq)).json(&big).send().await.unwrap().status(), 201);
        let pad = " ".repeat(300 * 1024);
        let r = key(c.post(&seq)).body(format!("{{\"frames\": []{pad}}}")).send().await.unwrap();
        assert_eq!(r.status(), 413);
        let r = key(c.post(&msg)).body(format!("{{\"text\": \"a\"{}}}", " ".repeat(70 * 1024))).send().await.unwrap();
        assert_eq!(r.status(), 413);
        drop(h);
    }

    /// The Sources routes (protocol v3): the key, validation, 403 for command plugins, and
    /// secrets stored but never sent back.
    #[tokio::test]
    async fn sources_routes() {
        let (engine, _) = sources::tests::engine("api");
        let board = Arc::new(Fake { size: (6, 22), engine: Some(Arc::new(engine)), ..Default::default() });
        *board.grid.lock().unwrap() = vec![vec![0; 22]; 6];
        let h = start(board.clone(), 0, false).await.unwrap();
        let base = format!("http://{}/rattleboard", h.addr);
        let c = reqwest::Client::new();
        let key = |r: reqwest::RequestBuilder| r.header("X-Vestaboard-Local-Api-Key", "k1");
        let url = |p: &str| format!("{base}/sources{p}");

        for r in [
            c.get(url("")),
            c.put(url("/rotation")).body("[]"),
            c.post(url("/plugins")).body("{}"),
            c.delete(url("/plugins/com.example.x")),
            c.put(url("/instances/x/secrets")).body("{}"),
            c.get(url("/preview/x")),
            c.get(url("/upnext")),
            c.post(url("/next")),
        ] {
            let r = r.header("X-Vestaboard-Local-Api-Key", "wrong").send().await.unwrap();
            assert_eq!(r.status(), 401);
            assert_eq!(r.headers()["access-control-allow-origin"], "*");
        }
        let v: Value = key(c.get(format!("{base}/info"))).send().await.unwrap().json().await.unwrap();
        assert_eq!(v["features"], json!(["sequence", "strategy", "sources"]));

        let v: Value = key(c.get(url(""))).send().await.unwrap().json().await.unwrap();
        let ids: Vec<&str> = v["plugins"].as_array().unwrap().iter().filter_map(|p| p["id"].as_str()).collect();
        for b in ["rattleboard.clock", "rattleboard.countdown", "rattleboard.weather", "rattleboard.rss", "rattleboard.ical", "rattleboard.json", "rattleboard.motd", "rattleboard.quote"] {
            assert!(ids.contains(&b), "{b}");
        }
        assert_eq!((v["rotation"].clone(), v["quietHours"]["from"].clone(), v["holdMinutes"].clone()), (json!([]), json!("22:00"), json!(30)));

        // Install: a JSON plugin is fine, a command plugin is refused, a bad one says why.
        let iss = json!({"format": 1, "id": "com.example.iss", "name": "ISS", "kind": "json",
            "fetch": {"url": "https://api.wheretheiss.at/v1/satellites/25544", "every": "1m"}, "pick": {"title": "$.latitude", "body": "$.longitude"}});
        let r = key(c.post(url("/plugins"))).json(&iss).send().await.unwrap();
        assert_eq!(r.status(), 201);
        assert_eq!(r.json::<Value>().await.unwrap(), json!({"status": "ok", "id": "com.example.iss"}));
        let cmd = json!({"format": 1, "id": "com.example.git", "name": "Git", "kind": "command", "run": {"path": GIT, "args": ["status"]}});
        assert_eq!(key(c.post(url("/plugins"))).json(&cmd).send().await.unwrap().status(), 403);
        let r = key(c.post(url("/plugins"))).json(&json!({"format": 1, "id": "com.example.bad", "name": "Bad", "kind": "rss", "fetch": {"url": "http://example.com/"}})).send().await.unwrap();
        assert_eq!(r.status(), 400);
        assert!(r.json::<Value>().await.unwrap()["error"].as_str().unwrap().contains("https"));
        assert_eq!(key(c.post(url("/plugins"))).body("{nope").send().await.unwrap().status(), 400);

        // The rotation: validated as a whole.
        let rot = json!({"rotation": [
            {"id": "clock", "plugin": "rattleboard.clock", "settings": {"style": "date"}, "dwell": "1m"},
            {"id": "cal", "plugin": "rattleboard.ical", "enabled": false},
            {"id": "iss", "plugin": "com.example.iss", "weight": 2},
        ], "quietHours": {"enabled": true, "from": "23:00", "to": "06:30"}, "flipsPerHour": 20});
        let r = key(c.put(url("/rotation"))).json(&rot).send().await.unwrap();
        assert_eq!(r.status(), 200);
        let v: Value = r.json().await.unwrap();
        assert_eq!((v["rotation"].as_array().unwrap().len(), v["flipsPerHour"].clone(), v["quietHours"]["to"].clone()), (3, json!(20), json!("06:30")));
        assert_eq!(v["rotation"][0]["dwell"], 60);
        for bad in [json!([{"plugin": "com.nope"}]), json!([{"plugin": "rattleboard.clock", "weight": 0}]), json!({"rotation": "x"}), json!({"flipsPerHour": 0})] {
            let r = key(c.put(url("/rotation"))).json(&bad).send().await.unwrap();
            assert_eq!(r.status(), 400, "{bad}");
            assert!(r.json::<Value>().await.unwrap()["error"].is_string());
        }

        // Secrets go in, and only their state ever comes out.
        let secret = "https://calendar.example.com/private-7f3a9c/basic.ics";
        let r = key(c.put(url("/instances/cal/secrets"))).json(&json!({"url": secret})).send().await.unwrap();
        assert_eq!(r.status(), 204);
        assert!(r.text().await.unwrap().is_empty());
        let body = key(c.get(url(""))).send().await.unwrap().text().await.unwrap();
        assert!(!body.contains("private-7f3a9c"));
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["rotation"][1]["secrets"], json!({"url": {"set": true}}));
        assert_eq!(v["rotation"][0]["secrets"], json!({}));
        assert_eq!(key(c.put(url("/instances/cal/secrets"))).json(&json!({"token": "x"})).send().await.unwrap().status(), 400);
        assert_eq!(key(c.put(url("/instances/nope/secrets"))).json(&json!({"url": "x"})).send().await.unwrap().status(), 404);

        // Preview: the card as it would show now, at this board's size.
        let v: Value = key(c.get(url("/preview/clock"))).send().await.unwrap().json().await.unwrap();
        let g: Grid = serde_json::from_value(v["characters"].clone()).unwrap();
        assert_eq!((g.len(), g[0].len()), (6, 22));
        assert!(v["reason"].is_string() && v["pages"].is_array());
        assert_eq!(key(c.get(url("/preview/nope"))).send().await.unwrap().status(), 404);

        // Up next and Next
        let v: Value = key(c.get(url("/upnext"))).send().await.unwrap().json().await.unwrap();
        let first = &v.as_array().unwrap()[0];
        assert!(first["at"].is_i64() && first["instanceId"].is_string() && first["title"].is_string() && first["reason"].is_string(), "{v}");
        let r = key(c.post(url("/next"))).send().await.unwrap();
        assert_eq!(r.json::<Value>().await.unwrap(), json!({"status": "ok"}));

        // Uninstalling takes its sources with it; built-ins stay.
        assert_eq!(key(c.delete(url("/plugins/rattleboard.clock"))).send().await.unwrap().status(), 400);
        assert_eq!(key(c.delete(url("/plugins/com.example.nope"))).send().await.unwrap().status(), 404);
        assert_eq!(key(c.delete(url("/plugins/com.example.iss"))).send().await.unwrap().status(), 204);
        let v: Value = key(c.get(url(""))).send().await.unwrap().json().await.unwrap();
        assert_eq!(v["rotation"].as_array().unwrap().len(), 2);

        let r = c.request(Method::OPTIONS, url("/rotation")).send().await.unwrap();
        assert!(r.headers()["access-control-allow-methods"].to_str().unwrap().contains("PUT"));
        drop(h);

        // A board without sources says so.
        let plain = fake();
        let h = start(plain, 0, false).await.unwrap();
        let r = c.get(format!("http://{}/rattleboard/sources", h.addr)).bearer_auth("k1").send().await.unwrap();
        assert_eq!(r.status(), 404);
        drop(h);
    }

    #[test]
    fn keys() {
        let k = new_key();
        assert_eq!(k.len(), 48);
        assert_ne!(k, new_key());
        assert!(same("abc", "abc") && !same("abc", "abd") && !same("abc", "ab"));
    }
}
