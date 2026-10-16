mod secrets;
mod server;
mod settings;
mod vestaboard;

use serde::{Deserialize, Serialize};
use settings::{Point, Settings};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::image::Image;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalRect, PhysicalSize, RunEvent, State,
    WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tokio::sync::Notify;
use vestaboard::Grid;

/// Transparent margin around the board inside its window, so the drop shadow isn't clipped.
const SHADOW: f64 = 36.0;
/// Gap between the board and the screen edge when parked in a corner (from the design).
const MARGIN: f64 = 20.0;
const HISTORY_MAX: usize = 12;
const TRAY_W: f64 = 336.0;
const TRAY_PAD: f64 = 14.0;

#[cfg(target_os = "macos")]
const OS: &str = "mac";
#[cfg(not(target_os = "macos"))]
const OS: &str = "win";

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct HistoryEntry {
    id: i64,
    at: i64,
    src: String,
    layout: Grid,
}

#[derive(Default)]
struct Runtime {
    layout: Option<Grid>,
    /// A layout chosen from history ("Flip again"); cleared by the next real change.
    display: Option<Grid>,
    msg_id: Option<String>,
    seq: u64,
    conn: String,
    error: Option<String>,
    last_checked: Option<i64>,
    last_change_at: Option<i64>,
    unread: bool,
    history: Vec<HistoryEntry>,
    /// The last message read from the mirrored board, kept apart from what's on screen
    /// so a message sent to the API server isn't mistaken for a change on the real board.
    remote_id: Option<String>,
    remote_layout: Option<Grid>,
    api_seq: u64,
    tray_hidden_at: Option<Instant>,
    drag_origin: Option<PhysicalPosition<i32>>,
}

struct Secrets {
    local_key: String,
    cloud_token: String,
    server_key: String,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct ServerStatus {
    running: bool,
    error: Option<String>,
    /// Base URLs apps can use, e.g. http://192.168.1.20:7070
    urls: Vec<String>,
}

struct AppState {
    settings: Mutex<Settings>,
    rt: Mutex<Runtime>,
    secrets: Mutex<Secrets>,
    http: reqwest::Client,
    wake: Notify,
    server: Mutex<Option<server::Handle>>,
    server_status: Mutex<ServerStatus>,
    settings_path: PathBuf,
    history_path: PathBuf,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    os: &'static str,
    version: &'static str,
    settings: Settings,
    display: Grid,
    has_layout: bool,
    seq: u64,
    conn: String,
    error: Option<String>,
    last_checked: Option<i64>,
    last_change_at: Option<i64>,
    unread: bool,
    history: Vec<HistoryEntry>,
    has_local_key: bool,
    has_cloud_token: bool,
    server: ServerStatus,
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn blank(s: &Settings) -> Grid {
    let (r, c) = s.dims();
    vec![vec![0; c]; r]
}

fn snapshot(st: &AppState) -> Snapshot {
    let s = st.settings.lock().unwrap().clone();
    let rt = st.rt.lock().unwrap();
    let sec = st.secrets.lock().unwrap();
    Snapshot {
        os: OS,
        version: env!("CARGO_PKG_VERSION"),
        display: rt.display.clone().or_else(|| rt.layout.clone()).unwrap_or_else(|| blank(&s)),
        has_layout: rt.layout.is_some(),
        settings: s,
        seq: rt.seq,
        conn: if rt.conn.is_empty() { "idle".into() } else { rt.conn.clone() },
        error: rt.error.clone(),
        last_checked: rt.last_checked,
        last_change_at: rt.last_change_at,
        unread: rt.unread,
        history: rt.history.clone(),
        has_local_key: !sec.local_key.is_empty(),
        has_cloud_token: !sec.cloud_token.is_empty(),
        server: st.server_status.lock().unwrap().clone(),
    }
}

fn broadcast(app: &AppHandle) {
    let st = app.state::<AppState>();
    let _ = app.emit("rb://state", snapshot(&st));
    refresh_tray_icon(app);
}

fn save_history(st: &AppState) {
    let rt = st.rt.lock().unwrap();
    if let Some(dir) = st.history_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string(&rt.history) {
        let _ = std::fs::write(&st.history_path, s);
    }
}

// ---------------------------------------------------------------- polling

async fn check(app: &AppHandle) -> Result<Grid, String> {
    let st = app.state::<AppState>();
    let (source, host) = {
        let s = st.settings.lock().unwrap();
        (s.source.clone(), s.host.clone())
    };
    let (key, token) = {
        let sec = st.secrets.lock().unwrap();
        (sec.local_key.clone(), sec.cloud_token.clone())
    };
    let res = match source.as_str() {
        "virtual" => {
            // Nothing to mirror: the board shows what apps send to the API server.
            let mut rt = st.rt.lock().unwrap();
            let s = st.settings.lock().unwrap();
            rt.last_checked = Some(now_ms());
            let (ok, err) = if !s.server_enabled {
                ("error", Some("Turn on the API server in Settings → API".to_string()))
            } else {
                match st.server_status.lock().unwrap().error.clone() {
                    Some(e) => ("error", Some(e)),
                    None => ("ok", None),
                }
            };
            rt.conn = ok.into();
            rt.error = err.clone();
            let layout = rt.layout.clone().unwrap_or_else(|| blank(&s));
            drop((rt, s));
            broadcast(app);
            return match err {
                Some(e) => Err(e),
                None => Ok(layout),
            };
        }
        "demo" => {
            let model = st.settings.lock().unwrap().model.clone();
            let secs = (now_ms() / 1000) as u64;
            Ok(vestaboard::demo(&model, (secs / vestaboard::DEMO_EVERY_SECS) as usize))
        }
        "cloud" => vestaboard::read_cloud(&st.http, &token).await,
        _ => vestaboard::read_local(&st.http, &host, &key).await,
    };

    let reading = match res {
        Ok(r) => r,
        Err(e) => {
            {
                let mut rt = st.rt.lock().unwrap();
                rt.conn = "error".into();
                rt.error = Some(e.clone());
                rt.last_checked = Some(now_ms());
            }
            broadcast(app);
            return Err(e);
        }
    };

    let changed = {
        let mut rt = st.rt.lock().unwrap();
        rt.conn = "ok".into();
        rt.error = None;
        rt.last_checked = Some(now_ms());
        let changed = match (&reading.id, &rt.remote_id) {
            (Some(new), Some(old)) => new != old,
            _ => rt.remote_layout.as_ref() != Some(&reading.layout),
        };
        rt.remote_id = reading.id.clone();
        rt.remote_layout = Some(reading.layout.clone());
        changed
    };
    let src = match source.as_str() {
        "cloud" => "Cloud API",
        "demo" => "Demo",
        _ => "Local API",
    };
    if changed {
        show_message(app, reading.layout.clone(), reading.id, src);
    } else {
        broadcast(app);
    }
    Ok(reading.layout)
}

/// Puts a new message on the virtual board: history, unread dot, and the flip.
fn show_message(app: &AppHandle, layout: Grid, id: Option<String>, src: &str) {
    let st = app.state::<AppState>();
    let rows = layout.len();
    let cols = layout.first().map(|r| r.len()).unwrap_or(0);
    let detected = if rows == 3 && cols == 15 { "note" } else { "flag" };
    let setup_done = {
        let mut s = st.settings.lock().unwrap();
        if s.model != detected {
            s.model = detected.into();
            s.save(&st.settings_path);
        }
        s.setup_done
    };
    {
        let mut rt = st.rt.lock().unwrap();
        let first = rt.layout.is_none();
        if rt.layout.as_ref() == Some(&layout) && rt.display.is_none() {
            // Same picture as before (e.g. an app re-sent it): replay the flip.
            rt.seq += 1;
        }
        rt.layout = Some(layout.clone());
        rt.msg_id = id;
        rt.display = None;
        if !first || rt.last_change_at.is_none() {
            rt.last_change_at = Some(now_ms());
        }
        let already = rt.history.first().map(|h| h.layout == layout).unwrap_or(false);
        if !already {
            let at = now_ms();
            rt.history.insert(0, HistoryEntry { id: at, at, src: src.into(), layout });
            rt.history.truncate(HISTORY_MAX);
        }
        let tray_open = app.get_webview_window("tray").and_then(|w| w.is_visible().ok()).unwrap_or(false);
        rt.unread = !tray_open;
    }
    save_history(&st);
    broadcast(app);
    if setup_done {
        present_board(app, false);
    }
}

async fn poll_loop(app: AppHandle) {
    loop {
        let (go, every) = {
            let st = app.state::<AppState>();
            let s = st.settings.lock().unwrap();
            (s.polling && s.setup_done, if s.source == "virtual" { 5 } else { s.poll_every.max(1) })
        };
        if go {
            let _ = check(&app).await;
        }
        let st = app.state::<AppState>();
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(every as u64)) => {}
            _ = st.wake.notified() => {}
        }
    }
}

// ---------------------------------------------------------------- board window

fn board(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("board")
}

fn work_area(app: &AppHandle, w: &WebviewWindow) -> (PhysicalRect<i32, u32>, f64) {
    let mon = w
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())
        .or_else(|| app.available_monitors().ok().and_then(|m| m.into_iter().next()));
    match mon {
        Some(m) => (*m.work_area(), m.scale_factor()),
        None => (
            PhysicalRect { position: PhysicalPosition::new(0, 0), size: PhysicalSize::new(1440, 900) },
            1.0,
        ),
    }
}

fn corner_positions(wa: &PhysicalRect<i32, u32>, size: PhysicalSize<u32>, scale: f64) -> [(&'static str, i32, i32); 4] {
    let off = ((MARGIN - SHADOW) * scale).round() as i32;
    let (x0, y0) = (wa.position.x, wa.position.y);
    let x1 = x0 + wa.size.width as i32 - size.width as i32;
    let y1 = y0 + wa.size.height as i32 - size.height as i32;
    [("tl", x0 + off, y0 + off), ("tr", x1 - off, y0 + off), ("bl", x0 + off, y1 - off), ("br", x1 - off, y1 - off)]
}

fn clamp_to(wa: &PhysicalRect<i32, u32>, size: PhysicalSize<u32>, scale: f64, x: i32, y: i32) -> (i32, i32) {
    let sh = (SHADOW * scale) as i32;
    let min_x = wa.position.x - sh;
    let min_y = wa.position.y - sh;
    let max_x = (wa.position.x + wa.size.width as i32 - size.width as i32 + sh).max(min_x);
    let max_y = (wa.position.y + wa.size.height as i32 - size.height as i32 + sh).max(min_y);
    (x.clamp(min_x, max_x), y.clamp(min_y, max_y))
}

fn place_board(app: &AppHandle) {
    let Some(w) = board(app) else { return };
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    let size = w.outer_size().unwrap_or_default();
    let (x, y) = if s.floating() {
        let (cx, cy) = (s.custom.x as i32, s.custom.y as i32);
        let mon = app.monitor_from_point(s.custom.x, s.custom.y).ok().flatten();
        let (wa, scale) = match mon {
            Some(m) => (*m.work_area(), m.scale_factor()),
            None => work_area(app, &w),
        };
        clamp_to(&wa, size, scale, cx, cy)
    } else {
        let (wa, scale) = work_area(app, &w);
        corner_positions(&wa, size, scale)
            .into_iter()
            .find(|c| c.0 == s.position)
            .map(|c| (c.1, c.2))
            .unwrap_or((wa.position.x, wa.position.y))
    };
    let _ = w.set_position(PhysicalPosition::new(x, y));
}

fn apply_layering(app: &AppHandle) {
    let Some(w) = board(app) else { return };
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    let desktop = s.floating() && s.float_layer == "desktop";
    let _ = w.set_always_on_bottom(desktop);
    let _ = w.set_always_on_top(!desktop);
}

/// Shows the board window and tells it to flip into view. `countdown` starts the
/// hide timer immediately ("Show board") instead of waiting for the flaps to settle.
fn present_board(app: &AppHandle, countdown: bool) {
    if let Some(w) = board(app) {
        apply_layering(app);
        let _ = w.show();
        let _ = app.emit_to("board", "rb://present", countdown);
    }
}

fn create_board(app: &AppHandle) -> tauri::Result<()> {
    let w = WebviewWindowBuilder::new(app, "board", WebviewUrl::App("board.html".into()))
        .title("Rattleboard")
        .inner_size(700.0, 300.0)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .focused(false)
        .focusable(false)
        .accept_first_mouse(true)
        .visible(false)
        .build()?;
    let _ = w.set_background_color(Some(tauri::window::Color(0, 0, 0, 0)));
    Ok(())
}

// ---------------------------------------------------------------- tray

fn tray_image(unread: bool) -> Option<Image<'static>> {
    let bytes: &'static [u8] = match (OS, unread) {
        ("mac", false) => include_bytes!("../icons/tray-mac.png"),
        ("mac", true) => include_bytes!("../icons/tray-mac-unread.png"),
        (_, false) => include_bytes!("../icons/tray-win.png"),
        (_, true) => include_bytes!("../icons/tray-win-unread.png"),
    };
    Image::from_bytes(bytes).ok()
}

fn refresh_tray_icon(app: &AppHandle) {
    let st = app.state::<AppState>();
    let unread = st.rt.lock().unwrap().unread;
    if let Some(t) = app.tray_by_id("main") {
        let _ = t.set_icon(tray_image(unread));
        #[cfg(target_os = "macos")]
        let _ = t.set_icon_as_template(true);
    }
}

fn create_tray_window(app: &AppHandle) -> tauri::Result<()> {
    WebviewWindowBuilder::new(app, "tray", WebviewUrl::App("tray.html".into()))
        .title("Rattleboard")
        .inner_size(TRAY_W + TRAY_PAD * 2.0, 420.0)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .accept_first_mouse(true)
        .visible(false)
        .build()?;
    Ok(())
}

fn toggle_tray(app: &AppHandle, rect: tauri::Rect) {
    let Some(w) = app.get_webview_window("tray") else { return };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
        return;
    }
    {
        // A click on the icon first blurs the open popover; don't immediately reopen it.
        let st = app.state::<AppState>();
        let rt = st.rt.lock().unwrap();
        if rt.tray_hidden_at.is_some_and(|t| t.elapsed() < Duration::from_millis(250)) {
            return;
        }
    }
    let mon = app.monitor_from_point(
        rect.position.to_physical::<f64>(1.0).x,
        rect.position.to_physical::<f64>(1.0).y,
    );
    let (wa, scale) = match mon.ok().flatten().or_else(|| app.primary_monitor().ok().flatten()) {
        Some(m) => (*m.work_area(), m.scale_factor()),
        None => return,
    };
    let ipos = rect.position.to_physical::<f64>(scale);
    let isize = rect.size.to_physical::<f64>(scale);
    let size = w.outer_size().unwrap_or_default();
    let pad = TRAY_PAD * scale;
    let mut x = ipos.x + isize.width / 2.0 - size.width as f64 / 2.0;
    let wa_x0 = wa.position.x as f64;
    let wa_x1 = wa_x0 + wa.size.width as f64;
    x = x.clamp(wa_x0 - pad + 8.0 * scale, wa_x1 - size.width as f64 + pad - 8.0 * scale);
    let mid = wa.position.y as f64 + wa.size.height as f64 / 2.0;
    let y = if ipos.y < mid {
        // menu bar at the top (macOS): hang just below it
        wa.position.y as f64 - pad + 4.0 * scale
    } else {
        // taskbar at the bottom (Windows): sit above it
        wa.position.y as f64 + wa.size.height as f64 - size.height as f64 + pad - 8.0 * scale
    };
    let _ = w.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    let _ = app.emit_to("tray", "rb://tray-anchor", if ipos.y < mid { "top" } else { "bottom" });
    let _ = w.show();
    let _ = w.set_focus();
    let st = app.state::<AppState>();
    st.rt.lock().unwrap().unread = false;
    broadcast(app);
}

fn hide_tray(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("tray") {
        let _ = w.hide();
    }
}

// ---------------------------------------------------------------- panel window

fn open_panel(app: &AppHandle, view: &str) {
    hide_tray(app);
    let (title, width) = match view {
        "history" => ("Message History", 580.0),
        "setup" => ("Set up Rattleboard", 640.0),
        _ => ("Rattleboard Settings", 600.0),
    };
    if let Some(w) = app.get_webview_window("panel") {
        let _ = w.set_title(title);
        let _ = app.emit_to("panel", "rb://view", view);
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return;
    }
    let url = WebviewUrl::App(format!("panel.html#{view}").into());
    if let Ok(w) = WebviewWindowBuilder::new(app, "panel", url)
        .title(title)
        .inner_size(width, 480.0)
        .min_inner_size(480.0, 320.0)
        .resizable(true)
        .maximizable(false)
        .minimizable(false)
        .center()
        .focused(true)
        // Shown by the first panel_resize, once it has its final size, so it doesn't jump.
        .visible(false)
        .build()
    {
        let w2 = w.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(800)).await;
            if !w2.is_visible().unwrap_or(true) {
                let _ = w2.show();
                let _ = w2.set_focus();
            }
        });
    }
}

// ---------------------------------------------------------------- API server

struct ApiBoard(AppHandle);

impl server::Board for ApiBoard {
    fn api_key(&self) -> String {
        self.0.state::<AppState>().secrets.lock().unwrap().server_key.clone()
    }
    fn dims(&self) -> (usize, usize) {
        self.0.state::<AppState>().settings.lock().unwrap().dims()
    }
    fn current(&self) -> (Grid, Option<String>) {
        let st = self.0.state::<AppState>();
        let s = st.settings.lock().unwrap().clone();
        let rt = st.rt.lock().unwrap();
        (rt.layout.clone().unwrap_or_else(|| blank(&s)), rt.msg_id.clone())
    }
    fn post(&self, layout: Grid) -> String {
        let st = self.0.state::<AppState>();
        let id = {
            let mut rt = st.rt.lock().unwrap();
            rt.api_seq += 1;
            format!("api-{}-{}", now_ms(), rt.api_seq)
        };
        let (app, l, i) = (self.0.clone(), layout, id.clone());
        // Windows must be touched from the main thread.
        let _ = self.0.run_on_main_thread(move || show_message(&app, l, Some(i), "API server"));
        id
    }
}

fn ensure_server_key(st: &AppState) -> Result<(), String> {
    if st.secrets.lock().unwrap().server_key.is_empty() {
        let k = server::new_key();
        secrets::set(secrets::SERVER_KEY, &k)?;
        st.secrets.lock().unwrap().server_key = k;
    }
    Ok(())
}

async fn restart_server(app: &AppHandle) {
    let st = app.state::<AppState>();
    // Stop the old listener first so the port is free again.
    drop(st.server.lock().unwrap().take());
    let (on, port, lan) = {
        let s = st.settings.lock().unwrap();
        (s.server_enabled, s.server_port, s.server_lan)
    };
    let status = if !on {
        ServerStatus::default()
    } else if let Err(e) = ensure_server_key(&st) {
        ServerStatus { error: Some(format!("Couldn't save the API key: {e}")), ..Default::default() }
    } else {
        let mut res = server::start(Arc::new(ApiBoard(app.clone())), port, lan).await;
        if res.is_err() {
            // The previous listener may take a moment to let go of the port.
            tokio::time::sleep(Duration::from_millis(300)).await;
            res = server::start(Arc::new(ApiBoard(app.clone())), port, lan).await;
        }
        match res {
            Ok(h) => {
                let p = h.addr.port();
                *st.server.lock().unwrap() = Some(h);
                let mut urls = vec![format!("http://127.0.0.1:{p}")];
                if lan {
                    if let Some(ip) = server::lan_ip() {
                        urls.insert(0, format!("http://{ip}:{p}"));
                    }
                }
                ServerStatus { running: true, error: None, urls }
            }
            Err(e) => ServerStatus { error: Some(e), ..Default::default() },
        }
    };
    *st.server_status.lock().unwrap() = status;
    st.wake.notify_one();
    broadcast(app);
}

#[tauri::command]
async fn regenerate_server_key(app: AppHandle) -> Result<(), String> {
    let k = server::new_key();
    secrets::set(secrets::SERVER_KEY, &k)?;
    app.state::<AppState>().secrets.lock().unwrap().server_key = k;
    broadcast(&app);
    Ok(())
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_state(st: State<AppState>) -> Snapshot {
    snapshot(&st)
}

#[tauri::command]
fn update_settings(app: AppHandle, st: State<AppState>, patch: serde_json::Value) -> Result<(), String> {
    let (old, new) = {
        let mut s = st.settings.lock().unwrap();
        let old = s.clone();
        let mut new = s.merge(patch)?;
        if !old.floating() && new.floating() {
            // Pinning a visible toast keeps it right where it is.
            if let Some(w) = board(&app).filter(|w| w.is_visible().unwrap_or(false)) {
                if let Ok(p) = w.outer_position() {
                    new.custom = Point { x: p.x as f64, y: p.y as f64 };
                }
            }
        }
        *s = new.clone();
        new.save(&st.settings_path);
        (old, new)
    };
    if old.mode != new.mode || old.float_layer != new.float_layer {
        apply_layering(&app);
    }
    if old.mode != new.mode || old.position != new.position || old.custom != new.custom {
        place_board(&app);
    }
    if old.mode != new.mode {
        let _ = app.emit_to("board", "rb://mode", ());
    }
    if old.server_enabled != new.server_enabled || old.server_port != new.server_port || old.server_lan != new.server_lan {
        let h = app.clone();
        tauri::async_runtime::spawn(async move { restart_server(&h).await });
    }
    if old.source != new.source || old.host != new.host || old.poll_every != new.poll_every || old.polling != new.polling {
        let mut rt = st.rt.lock().unwrap();
        if old.source != new.source || old.host != new.host {
            rt.remote_id = None;
            rt.remote_layout = None;
            rt.conn = "idle".into();
            rt.error = None;
        }
        drop(rt);
        st.wake.notify_one();
    }
    if old.launch_login != new.launch_login && new.setup_done {
        set_autostart(&app, new.launch_login);
    }
    broadcast(&app);
    Ok(())
}

fn set_autostart(app: &AppHandle, on: bool) {
    let al = app.autolaunch();
    let _ = if on { al.enable() } else { al.disable() };
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecretsOut {
    local_key: String,
    cloud_token: String,
    server_key: String,
}

#[tauri::command]
fn get_secrets(st: State<AppState>) -> SecretsOut {
    let s = st.secrets.lock().unwrap();
    SecretsOut { local_key: s.local_key.clone(), cloud_token: s.cloud_token.clone(), server_key: s.server_key.clone() }
}

#[tauri::command]
fn set_secret(app: AppHandle, st: State<AppState>, kind: String, value: String) -> Result<(), String> {
    let value = value.trim().to_string();
    let account = if kind == "cloud" { secrets::CLOUD_TOKEN } else { secrets::LOCAL_KEY };
    secrets::set(account, &value)?;
    {
        let mut s = st.secrets.lock().unwrap();
        if kind == "cloud" {
            s.cloud_token = value;
        } else {
            s.local_key = value;
        }
    }
    {
        let mut rt = st.rt.lock().unwrap();
        rt.remote_id = None;
        rt.remote_layout = None;
    }
    st.wake.notify_one();
    broadcast(&app);
    Ok(())
}

#[derive(Serialize)]
struct TestResult {
    layout: Grid,
}

#[tauri::command]
async fn test_connection(app: AppHandle) -> Result<TestResult, String> {
    {
        let st = app.state::<AppState>();
        st.rt.lock().unwrap().conn = "testing".into();
    }
    broadcast(&app);
    check(&app).await.map(|layout| TestResult { layout })
}

#[derive(Serialize)]
struct Found {
    name: String,
    ip: String,
}

#[tauri::command]
async fn scan_boards() -> Vec<Found> {
    vestaboard::scan().await.into_iter().map(|(name, ip)| Found { name, ip }).collect()
}

#[tauri::command]
fn show_board(app: AppHandle) {
    hide_tray(&app);
    present_board(&app, true);
}

#[tauri::command]
fn replay(app: AppHandle, st: State<AppState>) {
    hide_tray(&app);
    st.rt.lock().unwrap().seq += 1;
    broadcast(&app);
    present_board(&app, false);
}

#[tauri::command]
fn show_history_entry(app: AppHandle, st: State<AppState>, id: i64) {
    {
        let mut rt = st.rt.lock().unwrap();
        let Some(h) = rt.history.iter().find(|h| h.id == id).cloned() else { return };
        let current = rt.display.clone().or_else(|| rt.layout.clone());
        if current.as_ref() == Some(&h.layout) {
            rt.seq += 1;
        }
        rt.display = if rt.layout.as_ref() == Some(&h.layout) { None } else { Some(h.layout) };
    }
    broadcast(&app);
    present_board(&app, false);
}

#[tauri::command]
fn clear_history(app: AppHandle, st: State<AppState>) {
    st.rt.lock().unwrap().history.clear();
    save_history(&st);
    broadcast(&app);
}

#[tauri::command]
fn open_window(app: AppHandle, view: String) {
    open_panel(&app, &view);
}

#[tauri::command]
fn close_window(app: AppHandle, label: String) {
    if let Some(w) = app.get_webview_window(&label) {
        let _ = if label == "panel" { w.close() } else { w.hide() };
    }
}

#[tauri::command]
fn finish_setup(app: AppHandle, st: State<AppState>) {
    let login = {
        let mut s = st.settings.lock().unwrap();
        s.setup_done = true;
        s.save(&st.settings_path);
        s.launch_login
    };
    set_autostart(&app, login);
    if let Some(w) = app.get_webview_window("panel") {
        let _ = w.close();
    }
    st.wake.notify_one();
    broadcast(&app);
    let h = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(350)).await;
        present_board(&h, false);
    });
}

#[tauri::command]
fn quit(app: AppHandle) {
    for w in app.webview_windows().values() {
        let _ = w.hide();
    }
    drop(app.state::<AppState>().server.lock().unwrap().take());
    app.exit(0);
    // Backstop: if the event loop doesn't wind down (seen on Windows), leave anyway.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(1500));
        std::process::exit(0);
    });
}

#[tauri::command]
fn board_resize(app: AppHandle, width: f64, height: f64) {
    let Some(w) = board(&app) else { return };
    let size = tauri::LogicalSize::new(width + SHADOW * 2.0, height + SHADOW * 2.0);
    let _ = w.set_size(size);
    place_board(&app);
}

#[tauri::command]
fn board_hidden(app: AppHandle) {
    if let Some(w) = board(&app) {
        let _ = w.hide();
    }
}

#[tauri::command]
fn board_drag_start(app: AppHandle, st: State<AppState>) {
    if let Some(w) = board(&app) {
        st.rt.lock().unwrap().drag_origin = w.outer_position().ok();
    }
}

#[tauri::command]
fn board_drag_move(app: AppHandle, st: State<AppState>, dx: f64, dy: f64) {
    let Some(w) = board(&app) else { return };
    let Some(o) = st.rt.lock().unwrap().drag_origin else { return };
    let (wa, scale) = work_area(&app, &w);
    let size = w.outer_size().unwrap_or_default();
    let (x, y) = clamp_to(&wa, size, scale, o.x + (dx * scale) as i32, o.y + (dy * scale) as i32);
    let _ = w.set_position(PhysicalPosition::new(x, y));
}

/// Release: a toast snaps to the nearest corner; a floating board stays where it was dropped.
#[tauri::command]
fn board_drag_end(app: AppHandle, st: State<AppState>) {
    let Some(w) = board(&app) else { return };
    if st.rt.lock().unwrap().drag_origin.take().is_none() {
        return;
    }
    let (wa, scale) = work_area(&app, &w);
    let size = w.outer_size().unwrap_or_default();
    let pos = w.outer_position().unwrap_or_default();
    {
        let mut s = st.settings.lock().unwrap();
        if s.floating() {
            s.custom = Point { x: pos.x as f64, y: pos.y as f64 };
        } else {
            let dist = |(_, x, y): &(&str, i32, i32)| ((x - pos.x) as f64).hypot((y - pos.y) as f64);
            let corners = corner_positions(&wa, size, scale);
            if let Some(c) = corners.iter().min_by(|a, b| dist(a).total_cmp(&dist(b))) {
                s.position = c.0.into();
            }
        }
        s.save(&st.settings_path);
    }
    place_board(&app);
    broadcast(&app);
}

#[tauri::command]
fn tray_resize(app: AppHandle, height: f64) {
    if let Some(w) = app.get_webview_window("tray") {
        let _ = w.set_size(tauri::LogicalSize::new(TRAY_W + TRAY_PAD * 2.0, height));
    }
}

#[tauri::command]
fn panel_resize(app: AppHandle, width: f64, height: f64) {
    if let Some(w) = app.get_webview_window("panel") {
        // Fit the content, but never taller than the screen; the page scrolls beyond that.
        let max_h = w
            .current_monitor()
            .ok()
            .flatten()
            .map(|m| m.work_area().size.height as f64 / m.scale_factor() - 80.0)
            .unwrap_or(800.0);
        let _ = w.set_size(tauri::LogicalSize::new(width, height.min(max_h)));
        // Sized once per view, so re-centring here never moves a window the user placed mid-use.
        let _ = w.center();
        if !w.is_visible().unwrap_or(true) {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

// ---------------------------------------------------------------- app

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let setup = !app.state::<AppState>().settings.lock().unwrap().setup_done;
            open_panel(app, if setup { "setup" } else { "settings" });
        }))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .invoke_handler(tauri::generate_handler![
            get_state,
            update_settings,
            get_secrets,
            set_secret,
            test_connection,
            scan_boards,
            show_board,
            replay,
            show_history_entry,
            clear_history,
            open_window,
            close_window,
            finish_setup,
            quit,
            board_resize,
            board_hidden,
            board_drag_start,
            board_drag_move,
            board_drag_end,
            tray_resize,
            panel_resize,
            regenerate_server_key,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let cfg = app.path().app_config_dir()?;
            let data = app.path().app_data_dir()?;
            let settings_path = cfg.join("settings.json");
            let history_path = data.join("history.json");
            let settings = Settings::load(&settings_path);
            let history: Vec<HistoryEntry> = std::fs::read_to_string(&history_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let setup_done = settings.setup_done;
            let floating = settings.floating();
            app.manage(AppState {
                settings: Mutex::new(settings),
                rt: Mutex::new(Runtime { history, ..Default::default() }),
                secrets: Mutex::new(Secrets {
                    local_key: secrets::get(secrets::LOCAL_KEY),
                    cloud_token: secrets::get(secrets::CLOUD_TOKEN),
                    server_key: secrets::get(secrets::SERVER_KEY),
                }),
                http: vestaboard::client(),
                wake: Notify::new(),
                server: Mutex::new(None),
                server_status: Mutex::new(ServerStatus::default()),
                settings_path,
                history_path,
            });

            let h = app.handle().clone();
            create_board(&h)?;
            create_tray_window(&h)?;

            let mut tray = TrayIconBuilder::with_id("main")
                .tooltip("Rattleboard")
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { rect, button, button_state: MouseButtonState::Up, .. } = event {
                        // On Windows a right-click opens the native menu instead (the usual convention).
                        if button == MouseButton::Left || (OS == "mac" && button == MouseButton::Right) {
                            toggle_tray(tray.app_handle(), rect);
                        }
                    }
                });
            #[cfg(not(target_os = "macos"))]
            {
                use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
                let menu = Menu::with_items(
                    app,
                    &[
                        &MenuItem::with_id(app, "show", "Show board", true, None::<&str>)?,
                        &MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?,
                        &PredefinedMenuItem::separator(app)?,
                        &MenuItem::with_id(app, "quit", "Quit Rattleboard", true, None::<&str>)?,
                    ],
                )?;
                tray = tray.menu(&menu).on_menu_event(|app, e| match e.id().as_ref() {
                    "show" => present_board(app, true),
                    "settings" => open_panel(app, "settings"),
                    "quit" => quit(app.clone()),
                    _ => {}
                });
            }
            if let Some(img) = tray_image(false) {
                tray = tray.icon(img).icon_as_template(OS == "mac");
            }
            tray.build(app)?;

            tauri::async_runtime::spawn(poll_loop(h.clone()));
            let h2 = h.clone();
            tauri::async_runtime::spawn(async move { restart_server(&h2).await });
            if !setup_done {
                open_panel(&h, "setup");
            } else if floating {
                // A floating board is always on screen, even before the first check answers.
                present_board(&h, false);
            }
            Ok(())
        })
        .on_window_event(|w, e| {
            if w.label() == "tray" {
                if let WindowEvent::Focused(false) = e {
                    let _ = w.hide();
                    w.app_handle().state::<AppState>().rt.lock().unwrap().tray_hidden_at = Some(Instant::now());
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building Rattleboard");

    app.run(|_app, event| {
        // Closing the last window must not quit a menu-bar/tray app.
        if let RunEvent::ExitRequested { api, code: None, .. } = event {
            api.prevent_exit();
        }
    });
}
