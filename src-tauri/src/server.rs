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

use crate::vestaboard::{self, Grid};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use std::net::SocketAddr;
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

/// What the server needs from the app; implemented in lib.rs.
pub trait Board: Send + Sync + 'static {
    fn api_key(&self) -> String;
    fn dims(&self) -> (usize, usize);
    fn current(&self) -> (Grid, Option<String>);
    fn post(&self, layout: Grid) -> String;
}

#[derive(Clone)]
struct Ctx {
    board: std::sync::Arc<dyn Board>,
}

pub async fn start(board: std::sync::Arc<dyn Board>, port: u16, lan: bool) -> Result<Handle, String> {
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
    let ctx = Ctx { board };
    let router = Router::new()
        .route("/local-api/message", get(local_get).post(post).options(preflight))
        .route("/", get(cloud_get).post(post).options(preflight))
        .fallback(|| async { err(StatusCode::NOT_FOUND, "Not found") })
        .layer(axum::middleware::map_response(cors))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .with_state(ctx);
    let (tx, rx) = oneshot::channel::<()>();
    tauri::async_runtime::spawn(async move {
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

async fn local_get(State(ctx): State<Ctx>, h: HeaderMap) -> Response {
    if !authorized(&ctx, &h) {
        return err(StatusCode::UNAUTHORIZED, "Missing or wrong API key");
    }
    Json(ctx.board.current().0).into_response()
}

async fn cloud_get(State(ctx): State<Ctx>, h: HeaderMap) -> Response {
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
    let (rows, cols) = ctx.board.dims();
    let layout = match &v {
        Value::Array(_) => vestaboard::posted_grid(&v),
        Value::Object(o) => match (o.get("characters"), o.get("text")) {
            (Some(c), _) => vestaboard::posted_grid(c),
            (None, Some(Value::String(t))) => Ok(vestaboard::text_layout(t, rows, cols)),
            _ => Err("Send {\"text\": \"...\"}, {\"characters\": [[...]]} or a layout array".into()),
        },
        _ => Err("Send {\"text\": \"...\"}, {\"characters\": [[...]]} or a layout array".into()),
    };
    match layout {
        Ok(g) => {
            let id = ctx.board.post(g);
            (StatusCode::CREATED, Json(json!({ "status": "ok", "id": id, "created": crate::now_ms() }))).into_response()
        }
        Err(e) => err(StatusCode::BAD_REQUEST, &e),
    }
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
            "content-type, authorization, x-vestaboard-local-api-key, x-vestaboard-read-write-key, x-vestaboard-token",
        ),
    );
    h.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, OPTIONS"));
    if method == Method::OPTIONS {
        h.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
    }
    res
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
    use std::sync::{Arc, Mutex};

    struct Fake(Mutex<Grid>);
    impl Board for Fake {
        fn api_key(&self) -> String {
            "k1".into()
        }
        fn dims(&self) -> (usize, usize) {
            (3, 15)
        }
        fn current(&self) -> (Grid, Option<String>) {
            (self.0.lock().unwrap().clone(), Some("m1".into()))
        }
        fn post(&self, layout: Grid) -> String {
            *self.0.lock().unwrap() = layout;
            "m2".into()
        }
    }

    #[tokio::test]
    async fn endpoints() {
        let board = Arc::new(Fake(Mutex::new(vec![vec![0; 15]; 3])));
        let h = start(board.clone(), 0, false).await.unwrap();
        let base = format!("http://{}", h.addr);
        let c = reqwest::Client::new();
        let msg = format!("{base}/local-api/message");

        assert_eq!(c.get(&msg).send().await.unwrap().status(), 401);
        assert_eq!(c.get(&msg).header("X-Vestaboard-Local-Api-Key", "nope").send().await.unwrap().status(), 401);

        let r = c.post(&msg).header("X-Vestaboard-Local-Api-Key", "k1").body(r#"{"text":"hi there"}"#).send().await.unwrap();
        assert_eq!(r.status(), 201);
        assert_eq!(board.0.lock().unwrap()[1][3..11], vestaboard::text_layout("hi there", 3, 15)[1][3..11]);

        let g: Grid = c.get(&msg).header("X-Vestaboard-Local-Api-Key", "k1").send().await.unwrap().json().await.unwrap();
        assert_eq!(g, *board.0.lock().unwrap());

        let layout = vec![vec![5u8; 22]; 6];
        let r = c.post(&base).header("X-Vestaboard-Read-Write-Key", "k1").json(&layout).send().await.unwrap();
        assert_eq!(r.status(), 201);
        let v: Value = c.get(&base).header("X-Vestaboard-Token", "k1").send().await.unwrap().json().await.unwrap();
        let s = v["currentMessage"]["layout"].as_str().unwrap();
        assert_eq!(serde_json::from_str::<Grid>(s).unwrap(), layout);

        let r = c.post(&base).bearer_auth("k1").body(r#"{"characters":[[1,2,3]]}"#).send().await.unwrap();
        assert_eq!(r.status(), 400);
        let r = c.request(Method::OPTIONS, &base).send().await.unwrap();
        assert_eq!(r.headers()["access-control-allow-origin"], "*");
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
