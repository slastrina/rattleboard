mod fullscreen;
mod notify;
mod updates;

// The board engine, shared with Rattleboard Web (crates/rattleboard-core).
use rattleboard_core::{boards, plugins, secrets, send, sequence, server, settings, sources, vestaboard};
pub(crate) use rattleboard_core::now_ms;

use boards::SavedBoard;
use sequence::Sequence;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use settings::{NotifyApp, Point, Settings};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::image::Image;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, Monitor, PhysicalPosition, PhysicalRect, PhysicalSize, RunEvent,
    State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tokio::sync::Notify;
use vestaboard::{Grid, Transition};

/// Transparent margin around the board inside its window, so the drop shadow isn't clipped.
const SHADOW: f64 = 36.0;
/// Gap between the board and the screen edge when parked in a corner (from the design).
const MARGIN: f64 = 20.0;
const HISTORY_MAX: usize = 12;
const RECENT_MAX: usize = 50;
const TRAY_W: f64 = 336.0;
const TRAY_PAD: f64 = 14.0;
/// Rattleboard's page, linked from the tray menu, Settings and full screen
const SITE: &str = "https://www.iokig.com/rattleboard";
const KOFI: &str = "https://ko-fi.com/slastrina";

#[cfg(target_os = "macos")]
const OS: &str = "mac";
#[cfg(target_os = "windows")]
const OS: &str = "win";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const OS: &str = "linux";

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct HistoryEntry {
    id: i64,
    at: i64,
    src: String,
    layout: Grid,
}

/// A message sent from Compose, for its Recent list.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RecentEntry {
    at: i64,
    target_name: String,
    layout: Grid,
}

#[derive(Default)]
struct Runtime {
    layout: Option<Grid>,
    /// A layout chosen from history ("Flip again"); cleared by the next real change.
    display: Option<Grid>,
    msg_id: Option<String>,
    /// How the flaps of the current message start
    transition: Transition,
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
    /// Why the last sequence stopped early, if it did
    play_error: Option<String>,
    /// The display full screen covers: its position and size
    covered: Option<(i32, i32, u32, u32)>,
    /// What was on the board when a source's alert interrupted it, to put back afterwards
    alert_return: Option<(Grid, Transition)>,
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

/// A sequence this computer is playing, on this board or on a real Vestaboard.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Playing {
    /// "this" or a saved board's id
    target: String,
    target_name: String,
    /// What the user picked: this board, a board, a group or all, so the tray can show a
    /// group's players together
    via: String,
    via_name: String,
    #[serde(flatten)]
    status: server::SequenceStatus,
}

struct Player {
    info: Playing,
    task: tauri::async_runtime::JoinHandle<()>,
}

/// Notification forwarding, which only Windows supports.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct NotifyInfo {
    supported: bool,
    /// "allowed" | "denied" | "unspecified" | "unavailable"
    access: String,
    /// Apps seen sending notifications since launch
    apps: Vec<NotifyApp>,
}

struct AppState {
    settings: Mutex<Settings>,
    rt: Mutex<Runtime>,
    secrets: Mutex<Secrets>,
    http: reqwest::Client,
    wake: Notify,
    server: Mutex<Option<server::Handle>>,
    server_status: Mutex<ServerStatus>,
    advert: Mutex<Option<boards::Advert>>,
    boards: Mutex<Vec<SavedBoard>>,
    /// Saved boards' keys, read from the credential store when first needed
    board_keys: Mutex<HashMap<String, String>>,
    players: Mutex<HashMap<String, Player>>,
    groups: Mutex<Vec<send::Group>>,
    /// When each real board may next take a message (see `queue_turn`)
    sent_at: Mutex<HashMap<String, Instant>>,
    notify: Mutex<NotifyInfo>,
    /// Sources: the rotation of plugin cards (see sources.rs)
    sources: Arc<sources::Engine>,
    /// Saved boards the sources engine drives as a hub, and how each is doing
    hub: Mutex<HashMap<String, HubTarget>>,
    settings_path: PathBuf,
    history_path: PathBuf,
    boards_path: PathBuf,
    drafts_path: PathBuf,
    recent_path: PathBuf,
    groups_path: PathBuf,
    /// This computer's name, as in the pair QR code and `/rattleboard/info`
    device: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    os: &'static str,
    version: &'static str,
    settings: Settings,
    /// The board's size, from its preset or custom size
    rows: usize,
    cols: usize,
    display: Grid,
    has_layout: bool,
    transition: Transition,
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
    boards: Vec<SavedBoard>,
    groups: Vec<send::Group>,
    playing: Vec<Playing>,
    play_error: Option<String>,
    notify: NotifyInfo,
    sources: sources::Status,
    /// False on a Linux desktop with no tray (see `fullscreen::linux::tray_available`)
    has_tray: bool,
    update: UpdateInfo,
}

/// A newer release, for the tray's "Update available" item and Settings.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct UpdateInfo {
    /// The newer version, when automatic checks are on and found one
    available: Option<String>,
    /// Installed with the Homebrew cask, so `brew upgrade` is the way to update
    homebrew: bool,
}

/// Whether the tray icon was put up, set once at launch.
static HAS_TRAY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// This computer's name as people know it ("Sam's MacBook").
fn device_name() -> String {
    #[cfg(target_os = "macos")]
    let name = std::process::Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok());
    #[cfg(not(target_os = "macos"))]
    let name = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        // Linux desktops rarely export HOSTNAME.
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok());
    name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Rattleboard".into())
}

/// Debug builds only: RATTLEBOARD_PROFILE=<name> keeps settings and keys apart from the real
/// ones, in config and data folders of its own (keys in a file there, see `secrets.rs`), for
/// trying changes out without touching them.
pub(crate) fn profile() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("RATTLEBOARD_PROFILE").ok().filter(|p| !p.is_empty())
}

fn profile_dir(dir: PathBuf) -> PathBuf {
    match (profile(), dir.file_name()) {
        (Some(p), Some(name)) => dir.with_file_name(format!("{}.{p}", name.to_string_lossy())),
        _ => dir,
    }
}

fn blank(s: &Settings) -> Grid {
    let (r, c) = s.dims();
    vec![vec![0; c]; r]
}

fn snapshot(st: &AppState) -> Snapshot {
    let s = st.settings.lock().unwrap().clone();
    let rt = st.rt.lock().unwrap();
    let sec = st.secrets.lock().unwrap();
    let display = rt.display.clone().or_else(|| rt.layout.clone()).unwrap_or_else(|| blank(&s));
    // An adaptive board is the size of whatever it shows.
    let (rows, cols) = if s.adaptive { vestaboard::dims_of(&display) } else { s.dims() };
    Snapshot {
        os: OS,
        version: env!("CARGO_PKG_VERSION"),
        display,
        has_layout: rt.layout.is_some(),
        transition: rt.transition.clone(),
        rows,
        cols,
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
        boards: st.boards.lock().unwrap().clone(),
        groups: st.groups.lock().unwrap().clone(),
        playing: st.players.lock().unwrap().values().map(|p| p.info.clone()).collect(),
        play_error: rt.play_error.clone(),
        notify: st.notify.lock().unwrap().clone(),
        sources: st.sources.status(),
        has_tray: HAS_TRAY.load(std::sync::atomic::Ordering::Relaxed),
        update: UpdateInfo {
            available: (s.update_check && rattleboard_core::update::is_newer(&s.update_latest, env!("CARGO_PKG_VERSION"))).then(|| s.update_latest.clone()),
            homebrew: updates::from_homebrew(),
        },
        settings: s,
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
            let (rows, cols) = st.settings.lock().unwrap().dims();
            let secs = (now_ms() / 1000) as u64;
            Ok(vestaboard::demo(rows, cols, (secs / vestaboard::DEMO_EVERY_SECS) as usize))
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
        show_message(app, reading.layout.clone(), reading.id, src, Transition::default());
    } else {
        broadcast(app);
    }
    Ok(reading.layout)
}

/// Puts a new message on the virtual board: history, unread dot, and the flip. It holds the
/// sources rotation for a while (see `sources::Engine::note_message`).
fn show_message(app: &AppHandle, layout: Grid, id: Option<String>, src: &str, tr: Transition) {
    put_on_board(app, layout, id, src, tr, Shown::Message);
}

/// How something gets onto this board.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Shown {
    /// A message from outside the engine: Compose, the API, a sequence, the mirrored board
    Message,
    /// The rotation's next card, or a live card re-drawn: no pop-up, no history
    Ambient,
    /// A source's alert, or its once-a-day message: pops up and goes in the history
    Alert,
    /// What was there before an alert
    Restore,
}

fn put_on_board(app: &AppHandle, layout: Grid, id: Option<String>, src: &str, tr: Transition, how: Shown) {
    let st = app.state::<AppState>();
    let loud = matches!(how, Shown::Message | Shown::Alert);
    if how == Shown::Message {
        st.sources.note_message(now_ms());
    }
    // A fixed board takes on the size of a board it mirrors (posted messages arrive fitted to
    // it already); an adaptive one just shows each message at its own size.
    let detected = vestaboard::dims_of(&layout);
    let (setup_done, stays) = {
        let mut s = st.settings.lock().unwrap();
        if !s.adaptive && s.dims() != detected {
            s.set_dims(detected);
            s.save(&st.settings_path);
        }
        (s.setup_done, s.floating() || s.fullscreen())
    };
    // Ask the window system first: on macOS that call waits for the main thread, which may
    // itself be waiting for `rt`, so it must never happen while a lock is held.
    let tray_open = app.get_webview_window("tray").and_then(|w| w.is_visible().ok()).unwrap_or(false);
    {
        let mut rt = st.rt.lock().unwrap();
        let first = rt.layout.is_none();
        if rt.layout.as_ref() == Some(&layout) && rt.display.is_none() {
            // Same picture as before (e.g. an app re-sent it): replay the flip.
            rt.seq += 1;
        }
        rt.layout = Some(layout.clone());
        rt.msg_id = id;
        rt.transition = tr;
        rt.display = None;
        if !first || rt.last_change_at.is_none() {
            rt.last_change_at = Some(now_ms());
        }
        if how == Shown::Message {
            // Nothing to go back to: the message replaced the alert.
            rt.alert_return = None;
        }
        let already = rt.history.first().map(|h| h.layout == layout).unwrap_or(false);
        if !already && loud {
            let at = now_ms();
            rt.history.insert(0, HistoryEntry { id: at, at, src: src.into(), layout });
            rt.history.truncate(HISTORY_MAX);
        }
        if loud {
            rt.unread = !tray_open;
        }
    }
    if loud {
        save_history(&st);
    }
    broadcast(app);
    // A source's next card never pops the board up, but a pinned or full-screen board is on
    // screen anyway, so it's made sure of.
    if setup_done && (loud || stays) {
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

// ---------------------------------------------------------------- sources

/// A saved board the engine pushes cards to (hub mode).
#[derive(Default)]
struct HubTarget {
    /// Offline: retried after this, without holding anything up
    backoff_until: Option<Instant>,
    fails: u32,
    flips: std::collections::VecDeque<Instant>,
    /// The latest card meant for it; an older one still waiting for its turn is dropped
    gen: u64,
    last: Option<Grid>,
    before_alert: Option<Grid>,
}

/// The time, this board's size, and whether the rotation is paused: it is while this board
/// mirrors another one (or the demo), unless "Show sources while mirroring" is on.
fn sources_ctx(app: &AppHandle) -> sources::Ctx {
    let st = app.state::<AppState>();
    let s = st.settings.lock().unwrap().clone();
    let mirroring = matches!(s.source.as_str(), "local" | "cloud" | "demo");
    let paused = if !s.setup_done {
        Some("Finish setting up Rattleboard first".to_string())
    } else if mirroring && !st.sources.while_mirroring() {
        Some(format!("Paused while mirroring {}", if s.source == "demo" { "the demo" } else { "another board" }))
    } else {
        None
    };
    sources::Ctx { now: now_ms(), local: chrono::Local::now().naive_local(), paused, dims: s.dims() }
}

/// The engine: fetches run in tasks of their own, so a slow feed never holds the rotation
/// up; the rotation moves on once a second, or at once when something changes.
async fn sources_loop(app: AppHandle) {
    let engine = app.state::<AppState>().sources.clone();
    loop {
        let ctx = sources_ctx(&app);
        if ctx.paused.is_none() {
            for (id, job) in engine.due_jobs(ctx.now) {
                let (e, local, now) = (engine.clone(), ctx.local, ctx.now);
                tauri::async_runtime::spawn(async move {
                    let r = e.run_job(job, local, now).await;
                    e.store(&id, r, now_ms());
                });
            }
        }
        let actions = engine.tick(&ctx);
        run_actions(&app, actions);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            _ = engine.wake.notified() => {}
        }
    }
}

/// Where the engine's cards go: "Show sources on" (this board by default), as boards.
fn sources_destinations(app: &AppHandle) -> Vec<String> {
    let targets = app.state::<AppState>().sources.config().targets;
    let mut out: Vec<String> = vec![];
    for t in targets {
        for d in destinations(app, &t).unwrap_or_default() {
            if !out.contains(&d) {
                out.push(d);
            }
        }
    }
    out
}

fn page_of(card: &sources::Card, page: usize, rows: usize, cols: usize) -> Grid {
    let mut pages = card.face.render(rows, cols);
    let k = page.min(pages.len() - 1);
    pages.swap_remove(k)
}

fn run_actions(app: &AppHandle, actions: Vec<sources::Action>) {
    if actions.is_empty() {
        return;
    }
    let dests = sources_destinations(app);
    for a in actions {
        match a {
            sources::Action::Show { card, page, how } => {
                for d in &dests {
                    if d == HERE {
                        let (rows, cols) = app.state::<AppState>().settings.lock().unwrap().dims();
                        let layout = page_of(&card, page, rows, cols);
                        let shown = match how {
                            sources::How::Alert | sources::How::Message => Shown::Alert,
                            _ => Shown::Ambient,
                        };
                        let (a, src, id, alert) = (app.clone(), format!("{} · Sources", card.name), next_id(app, "src"), how == sources::How::Alert);
                        let _ = app.run_on_main_thread(move || {
                            if alert {
                                let st = a.state::<AppState>();
                                let mut rt = st.rt.lock().unwrap();
                                if rt.alert_return.is_none() {
                                    rt.alert_return = rt.layout.clone().map(|l| (l, rt.transition.clone()));
                                }
                            }
                            put_on_board(&a, layout, Some(id), &src, Transition::default(), shown);
                        });
                    } else {
                        hub_push(app, d, &card, page, how);
                    }
                }
            }
            sources::Action::Return => {
                let a = app.clone();
                let _ = app.run_on_main_thread(move || {
                    let back = a.state::<AppState>().rt.lock().unwrap().alert_return.take();
                    if let Some((layout, tr)) = back {
                        let id = next_id(&a, "src");
                        put_on_board(&a, layout, Some(id), "Sources", tr, Shown::Restore);
                    }
                });
                for d in dests.iter().filter(|d| *d != HERE) {
                    let back = app.state::<AppState>().hub.lock().unwrap().get_mut(d).and_then(|t| t.before_alert.take());
                    if let Some(g) = back {
                        hub_send(app, d, g, sources::How::Ambient);
                    }
                }
            }
        }
    }
}

/// One card for a saved board, laid out for its size.
fn hub_push(app: &AppHandle, id: &str, card: &sources::Card, page: usize, how: sources::How) {
    let Ok(b) = find_board(app, id) else { return };
    let layout = page_of(card, page, b.rows, b.cols);
    hub_send(app, id, layout, how);
}

/// Sends to a saved board in the background, within its limits: a real Vestaboard's minimum
/// gap (shared with Compose), the hourly cap, and a backoff while it's offline. A card
/// superseded while it waited is dropped. Nothing here ever holds up the rotation.
fn hub_send(app: &AppHandle, id: &str, layout: Grid, how: sources::How) {
    let Ok(b) = find_board(app, id) else { return };
    let st = app.state::<AppState>();
    let cap = st.sources.config().flips_per_hour as usize;
    let gen = {
        let mut hub = st.hub.lock().unwrap();
        let t = hub.entry(id.to_string()).or_default();
        let now = Instant::now();
        if t.backoff_until.is_some_and(|u| u > now) {
            return;
        }
        while t.flips.front().is_some_and(|f| now.duration_since(*f) > Duration::from_secs(3600)) {
            t.flips.pop_front();
        }
        if how != sources::How::Update && t.flips.len() >= cap {
            return;
        }
        if t.last.as_ref() == Some(&layout) {
            return;
        }
        t.gen += 1;
        t.gen
    };
    let (app, id) = (app.clone(), id.to_string());
    tauri::async_runtime::spawn(async move {
        let wait = queue_turn(&app, &b);
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        let st = app.state::<AppState>();
        if st.hub.lock().unwrap().get(&id).is_some_and(|t| t.gen != gen) {
            return;
        }
        let key = board_key(&app, &b.id);
        let res = send::post(&st.http, &b, &key, &layout, &Transition::default()).await;
        let mut hub = st.hub.lock().unwrap();
        let t = hub.entry(id).or_default();
        match res {
            Ok(()) => {
                t.fails = 0;
                t.backoff_until = None;
                t.flips.push_back(Instant::now());
                if how == sources::How::Alert && t.before_alert.is_none() {
                    t.before_alert = t.last.clone();
                }
                t.last = Some(layout);
            }
            Err(_) => {
                // Offline or refusing: 30 s, then 1, 2, 4... minutes, up to 10.
                t.fails += 1;
                let secs = (30u64 << (t.fails - 1).min(5)).min(600);
                t.backoff_until = Some(Instant::now() + Duration::from_secs(secs));
            }
        }
    });
}

fn refused(r: sources::Refused) -> String {
    r.1
}

/// Settings → Sources: everything, and Up next.
#[tauri::command]
fn get_sources(app: AppHandle) -> Value {
    let st = app.state::<AppState>();
    let mut v = st.sources.view();
    let ctx = sources_ctx(&app);
    v["upNext"] = json!(st.sources.up_next(&ctx));
    v["targets"] = json!(st.sources.config().targets);
    v
}

#[tauri::command]
fn save_sources(app: AppHandle, sources: Value) -> Result<(), String> {
    app.state::<AppState>().sources.put_rotation(&sources, false).map_err(refused)
}

#[tauri::command]
fn add_source(app: AppHandle, plugin: String, settings: Option<Value>) -> Result<String, String> {
    app.state::<AppState>().sources.add(&plugin, settings.as_ref()).map_err(refused)
}

/// Import: a manifest, or a source exported from another board (with its plugin's manifest).
/// A command plugin can be installed here, but it doesn't run until it's approved.
#[tauri::command]
fn import_plugin(app: AppHandle, manifest: Value) -> Result<Value, String> {
    let e = app.state::<AppState>().sources.clone();
    if manifest["kind"] == "rattleboard-source" {
        let source = manifest.get("source").cloned().ok_or("That file has no source in it")?;
        if let Some(p) = manifest.get("plugin").filter(|p| p.is_object()) {
            e.install(p, false).map_err(refused)?;
        }
        let plugin = source["plugin"].as_str().ok_or("The source names no plugin")?.to_string();
        let id = e.add(&plugin, source.get("settings")).map_err(refused)?;
        // Its name, weight, dwell and schedule, as exported.
        let mut v = e.view()["rotation"].clone();
        if let Some(list) = v.as_array_mut() {
            for x in list.iter_mut().filter(|x| x["id"] == id.as_str()) {
                for k in ["name", "weight", "dwell", "schedule", "chip", "include", "exclude", "alertWords"] {
                    if let Some(val) = source.get(k) {
                        x[k] = val.clone();
                    }
                }
            }
        }
        e.put_rotation(&json!({ "rotation": v }), false).map_err(refused)?;
        return Ok(json!({ "source": id, "plugin": plugin }));
    }
    let id = e.install(&manifest, false).map_err(refused)?;
    Ok(json!({ "plugin": id }))
}

/// A rattleboard://plugin?url=<https url> link: fetches the manifest to show before installing
/// it. Command plugins are refused from links.
#[tauri::command]
async fn read_plugin_link(app: AppHandle, link: String) -> Result<Value, String> {
    let link = link.trim();
    let target = if link.starts_with("rattleboard://") {
        let u = url::Url::parse(link).map_err(|_| "That isn't a plugin link")?;
        if u.host_str() != Some("plugin") {
            return Err("Plugin links start with rattleboard://plugin".into());
        }
        u.query_pairs().find(|(k, _)| k == "url").map(|(_, v)| v.into_owned()).ok_or("The link has no url")?
    } else {
        link.to_string()
    };
    if !target.starts_with("https://") {
        return Err("Plugin links must point to an https:// address".into());
    }
    let e = app.state::<AppState>().sources.clone();
    let body = plugins::fetch(&e.http, "GET", &target, &[]).await?;
    let v: Value = serde_json::from_slice(&body).map_err(|_| "That address doesn't hold a plugin manifest".to_string())?;
    let m = plugins::parse(&v, false)?;
    if m.kind == "command" {
        return Err("Command plugins can't be installed from a link. If you trust it, download the file and use Import.".into());
    }
    Ok(json!({ "manifest": v, "name": m.name, "kind": m.kind, "description": m.description, "url": m.fetch.map(|f| f.url) }))
}

#[tauri::command]
fn remove_plugin(app: AppHandle, id: String) -> Result<(), String> {
    app.state::<AppState>().sources.uninstall(&id).map_err(refused)
}

/// The approval dialog's answer, for the manifest exactly as it was shown.
#[tauri::command]
fn approve_plugin(app: AppHandle, id: String, fingerprint: String, approve: bool) -> Result<(), String> {
    app.state::<AppState>().sources.approve(&id, &fingerprint, approve).map_err(refused)
}

#[tauri::command]
fn set_source_secret(app: AppHandle, instance: String, key: String, value: String) -> Result<(), String> {
    app.state::<AppState>().sources.set_secrets(&instance, &json!({ key: value })).map_err(refused)
}

/// Settings' Show button, on this computer only.
#[tauri::command]
fn reveal_source_secret(app: AppHandle, instance: String, key: String) -> String {
    app.state::<AppState>().sources.reveal_secret(&instance, &key)
}

#[tauri::command]
async fn source_preview(app: AppHandle, instance: String, refresh: Option<bool>) -> Result<Value, String> {
    let ctx = sources_ctx(&app);
    let e = app.state::<AppState>().sources.clone();
    let (pages, reason) = e.preview(&instance, &ctx, false, refresh.unwrap_or(false)).await.map_err(refused)?;
    let (fetches, fetched) = e.fetch_state(&instance);
    Ok(json!({ "pages": pages, "reason": reason, "rows": ctx.dims.0, "cols": ctx.dims.1, "fetches": fetches, "fetched": fetched }))
}

#[tauri::command]
fn sources_next(app: AppHandle) {
    hide_tray(&app);
    app.state::<AppState>().sources.request_next();
}

/// Export: a plugin's manifest, or a source without its secrets, saved to Downloads.
#[tauri::command]
fn export_source(app: AppHandle, id: String) -> Result<String, String> {
    let (name, v) = app.state::<AppState>().sources.export(&id).ok_or("Nothing to export with that id")?;
    let dir = app.path().download_dir().map_err(|e| e.to_string())?;
    let slug: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect::<String>().trim_matches('-').to_string();
    let path = dir.join(format!("{}.rattleboard-plugin.json", if slug.is_empty() { "source" } else { &slug }));
    let text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    std::fs::write(&path, text + "\n").map_err(|e| format!("Couldn't save to Downloads: {e}"))?;
    Ok(path.to_string_lossy().into_owned())
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

/// Puts the board in its corner (a pop-up) or at its spot (a pinned widget). `size` is the
/// window's new size when it has just been resized: the window may not report it yet.
fn place_board(app: &AppHandle, size: Option<PhysicalSize<u32>>) {
    let Some(w) = board(app) else { return };
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    if s.fullscreen() {
        return;
    }
    let size = size.unwrap_or_else(|| w.outer_size().unwrap_or_default());
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
    // Full screen stays above other apps, but not above Settings or Compose opened over it.
    let top = if s.fullscreen() { app.get_webview_window("panel").is_none() } else { !desktop };
    let _ = w.set_always_on_bottom(desktop);
    let _ = w.set_always_on_top(top);
    if s.fullscreen() && top {
        over_menu_bar(&w);
    }
}

/// A full-screen board on top covers the menu bar and Dock too (on Windows, full screen
/// already covers the taskbar).
fn over_menu_bar(_w: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    if let Ok(p) = _w.ns_window() {
        fullscreen::above_menu_bar(p);
    }
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

// ---------------------------------------------------------------- full screen

/// Behind the board in full screen: near-black for a black board, light for a white one
/// (as `--backdrop` in board.html).
fn backdrop(s: &Settings) -> tauri::window::Color {
    if s.finish == "white" {
        tauri::window::Color(0xe4, 0xe2, 0xdc, 255)
    } else {
        tauri::window::Color(0x05, 0x05, 0x06, 255)
    }
}

fn rect_of(m: &Monitor) -> (i32, i32, u32, u32) {
    (m.position().x, m.position().y, m.size().width, m.size().height)
}

/// The display full screen covers: the chosen one, else the main display.
fn chosen_screen(app: &AppHandle, s: &Settings) -> Option<Monitor> {
    let all = app.available_monitors().unwrap_or_default();
    let list: Vec<_> = all.iter().map(|m| (m.name().cloned().unwrap_or_default(), (m.position().x, m.position().y))).collect();
    let pos = (s.screen_pos.x as i32, s.screen_pos.y as i32);
    match fullscreen::pick_screen(&list, &s.screen_name, pos) {
        Some(i) => all.into_iter().nth(i),
        None => app.primary_monitor().ok().flatten().or_else(|| all.into_iter().next()),
    }
}

/// Covers the chosen display with the board, menu bar and dock or taskbar included, and keeps
/// it awake; again after the display changes, to re-fit it. On the main thread, and never
/// with a lock held: every call here goes to the window system.
fn cover_screen(app: &AppHandle) {
    let Some(w) = board(app) else { return };
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    let Some(m) = chosen_screen(app, &s) else { return };
    let _ = w.set_focusable(true);
    let _ = w.set_background_color(Some(backdrop(&s)));
    apply_layering(app);
    // Shown first: a hidden window isn't on any display yet.
    let _ = w.show();
    #[cfg(target_os = "macos")]
    {
        // Simple full screen covers the display the window is on, without a Space of its own.
        let _ = w.set_simple_fullscreen(false);
        let _ = w.set_position(m.position().to_logical::<f64>(m.scale_factor()));
        let _ = w.set_simple_fullscreen(true);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let p = m.position();
        let _ = w.set_fullscreen_on_monitor(PhysicalPosition::new(p.x as f64, p.y as f64));
    }
    focus_board(&w);
    app.state::<AppState>().rt.lock().unwrap().covered = Some(rect_of(&m));
    fullscreen::keep_awake(true);
    let _ = app.emit_to("board", "rb://present", false);
}

/// Full screen takes keys (Esc, F11): the window, and the page in it.
fn focus_board(w: &WebviewWindow) {
    let _ = w.set_focus();
    let _ = w.as_ref().set_focus();
}

/// Back from full screen to the pop-up or widget; the display may sleep again.
fn uncover_screen(app: &AppHandle) {
    fullscreen::keep_awake(false);
    app.state::<AppState>().rt.lock().unwrap().covered = None;
    let Some(w) = board(app) else { return };
    #[cfg(target_os = "macos")]
    let _ = w.set_simple_fullscreen(false);
    #[cfg(not(target_os = "macos"))]
    let _ = w.set_fullscreen(false);
    let _ = w.set_focusable(false);
    let _ = w.set_background_color(Some(tauri::window::Color(0, 0, 0, 0)));
    apply_layering(app);
    place_board(app, None);
}

/// Follows display changes while full screen: a new resolution, or the chosen display going
/// away (the board moves to the main display), as after unplugging a monitor or waking up.
async fn watch_screens(app: AppHandle) {
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if !app.state::<AppState>().settings.lock().unwrap().fullscreen() {
            continue;
        }
        let a = app.clone();
        let _ = app.run_on_main_thread(move || {
            // Looked at again here: full screen may have ended while this waited its turn.
            let st = a.state::<AppState>();
            let s = st.settings.lock().unwrap().clone();
            let covered = st.rt.lock().unwrap().covered;
            if s.fullscreen() && chosen_screen(&a, &s).map(|m| rect_of(&m)) != covered {
                cover_screen(&a);
            }
        });
    }
}

/// Tray, menus and keys (F11, and Control-Command-F on macOS): into full screen, or back to
/// the pop-up or widget it came from.
#[tauri::command]
fn toggle_fullscreen(app: AppHandle) {
    hide_tray(&app);
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    let mode = if s.fullscreen() { s.windowed_mode.clone() } else { "fullscreen".into() };
    let _ = apply_settings(&app, json!({ "mode": mode }));
    if s.fullscreen() {
        present_board(&app, true);
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Screen {
    name: String,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    primary: bool,
}

/// The displays full screen can cover, for Settings → Display.
#[tauri::command]
fn list_screens(app: AppHandle) -> Vec<Screen> {
    let main = app.primary_monitor().ok().flatten().map(|m| rect_of(&m));
    let all = app.available_monitors().unwrap_or_default();
    all.iter()
        .map(|m| {
            let (x, y, width, height) = rect_of(m);
            Screen { name: m.name().cloned().unwrap_or_default(), x, y, width, height, primary: main == Some((x, y, width, height)) }
        })
        .collect()
}

/// The board page hides the pointer after a while without movement.
#[tauri::command]
fn hide_pointer(app: AppHandle) {
    let _ = app.run_on_main_thread(fullscreen::hide_pointer);
}

/// macOS's menu, with full screen as Rattleboard's own rather than a Space of its own.
#[cfg(target_os = "macos")]
fn app_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem};
    let menu = Menu::default(app)?;
    for item in menu.items()? {
        if let Some(view) = item.as_submenu().filter(|s| s.text().is_ok_and(|t| t == "View")) {
            view.remove_at(0)?;
            view.append(&MenuItem::with_id(app, "fullscreen", "Full Screen", true, Some("Ctrl+Cmd+F"))?)?;
        }
    }
    Ok(menu)
}

/// The Windows tray menu's "Full screen" item, ticked while it is.
#[cfg(not(target_os = "macos"))]
struct FullItem(tauri::menu::CheckMenuItem<tauri::Wry>);

/// The native tray menu (Windows, Linux), which gains an "Update available" item when there is one.
#[cfg(not(target_os = "macos"))]
struct TrayMenu(tauri::menu::Menu<tauri::Wry>);

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
    #[cfg(not(target_os = "macos"))]
    if let Some(item) = app.try_state::<FullItem>() {
        let full = st.settings.lock().unwrap().fullscreen();
        let _ = item.0.set_checked(full);
    }
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
        "history" => ("Message History", 680.0),
        "setup" => ("Set up Rattleboard", 640.0),
        "compose" => ("Compose", 780.0),
        _ => ("Rattleboard Settings", 600.0),
    };
    if let Some(w) = app.get_webview_window("panel") {
        let _ = w.set_title(title);
        let _ = app.emit_to("panel", "rb://view", view);
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        lower_fullscreen(app);
        return;
    }
    let url = WebviewUrl::App(format!("panel.html#{view}").into());
    if let Ok(w) = WebviewWindowBuilder::new(app, "panel", url)
        .title(title)
        .inner_size(width, 480.0)
        .min_inner_size(480.0, 320.0)
        // Plugin files dropped on Settings → Sources reach the page.
        .disable_drag_drop_handler()
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
    lower_fullscreen(app);
}

/// A full-screen board makes way for Settings or Compose, and comes back when they close.
fn lower_fullscreen(app: &AppHandle) {
    if app.state::<AppState>().settings.lock().unwrap().fullscreen() {
        apply_layering(app);
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
    fn adaptive(&self) -> bool {
        self.0.state::<AppState>().settings.lock().unwrap().adaptive
    }
    fn name(&self) -> String {
        format!("Rattleboard ({})", self.0.state::<AppState>().device)
    }
    fn current(&self) -> (Grid, Option<String>) {
        let st = self.0.state::<AppState>();
        let s = st.settings.lock().unwrap().clone();
        let rt = st.rt.lock().unwrap();
        (rt.layout.clone().unwrap_or_else(|| blank(&s)), rt.msg_id.clone())
    }
    fn post(&self, layout: Grid, tr: Transition) -> String {
        let id = next_id(&self.0, "api");
        stop_player(&self.0, HERE);
        let (app, l, i) = (self.0.clone(), layout, id.clone());
        // Windows must be touched from the main thread.
        let _ = self.0.run_on_main_thread(move || show_message(&app, l, Some(i), "API server", tr));
        id
    }
    fn play(&self, seq: Sequence) -> String {
        start_player(&self.0, HERE, "This board", seq, None, (HERE, "This board"))
    }
    fn sequence(&self) -> Option<server::SequenceStatus> {
        let st = self.0.state::<AppState>();
        let players = st.players.lock().unwrap();
        players.get(HERE).map(|p| p.info.status.clone())
    }
    fn stop(&self) {
        stop_player(&self.0, HERE);
    }
    fn sources(&self) -> Option<Arc<sources::Engine>> {
        Some(self.0.state::<AppState>().sources.clone())
    }
    fn sources_ctx(&self) -> Option<sources::Ctx> {
        Some(sources_ctx(&self.0))
    }
}

fn next_id(app: &AppHandle, prefix: &str) -> String {
    let st = app.state::<AppState>();
    let mut rt = st.rt.lock().unwrap();
    rt.api_seq += 1;
    format!("{prefix}-{}-{}", now_ms(), rt.api_seq)
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
    drop(st.advert.lock().unwrap().take());
    let (on, port, lan, name) = {
        let s = st.settings.lock().unwrap();
        (s.server_enabled, s.server_port, s.server_lan, st.device.clone())
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
                    // So other Rattleboards can find this one, as they find a TV.
                    *st.advert.lock().unwrap() = boards::advertise(&name, p);
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

// ---------------------------------------------------------------- sequences

/// The player key for sequences shown on this computer's own board.
const HERE: &str = "this";

fn stop_player(app: &AppHandle, target: &str) -> bool {
    let st = app.state::<AppState>();
    let p = st.players.lock().unwrap().remove(target);
    let Some(p) = p else { return false };
    p.task.abort();
    broadcast(app);
    true
}

/// Plays `seq` on this board, or on a real Vestaboard (`to`, with its key) by posting each
/// frame. Replaces whatever that target was playing; returns the sequence id.
fn start_player(
    app: &AppHandle,
    target: &str,
    target_name: &str,
    seq: Sequence,
    to: Option<(SavedBoard, String)>,
    (via, via_name): (&str, &str),
) -> String {
    let st = app.state::<AppState>();
    let id = next_id(app, "seq");
    let status = server::SequenceStatus {
        id: id.clone(),
        name: seq.name.clone(),
        frame: 1,
        frames: seq.frames.len(),
        looping: seq.looping,
    };
    let info = Playing { target: target.into(), target_name: target_name.into(), via: via.into(), via_name: via_name.into(), status };
    {
        // Held while spawning, so the task can't report a frame before it's registered.
        let mut players = st.players.lock().unwrap();
        let task = tauri::async_runtime::spawn(play(app.clone(), target.to_string(), id.clone(), seq, to));
        if let Some(old) = players.insert(target.to_string(), Player { info, task }) {
            old.task.abort();
        }
    }
    st.rt.lock().unwrap().play_error = None;
    broadcast(app);
    id
}

/// Marks frame `n` as showing; false once this sequence has been replaced or stopped.
fn on_frame(app: &AppHandle, target: &str, id: &str, n: usize) -> bool {
    let st = app.state::<AppState>();
    {
        let mut players = st.players.lock().unwrap();
        match players.get_mut(target) {
            Some(p) if p.info.status.id == id => p.info.status.frame = n,
            _ => return false,
        }
    }
    broadcast(app);
    true
}

fn end_player(app: &AppHandle, target: &str, id: &str, error: Option<String>) {
    let st = app.state::<AppState>();
    {
        let mut players = st.players.lock().unwrap();
        if players.get(target).is_some_and(|p| p.info.status.id == id) {
            players.remove(target);
        }
    }
    if error.is_some() {
        st.rt.lock().unwrap().play_error = error;
    }
    broadcast(app);
}

/// Each frame is held for `hold` seconds after its flaps finish. This board's flip time is
/// worked out from the replica's own timing (`vestaboard::flip_ms`), which keeps working
/// while the board window is hidden and its animation isn't running. A real Vestaboard is
/// also given its minimum time between messages.
async fn play(app: AppHandle, target: String, id: String, seq: Sequence, to: Option<(SavedBoard, String)>) {
    let st = app.state::<AppState>();
    let mut prev: Option<Grid> = None;
    let mut i = 0;
    loop {
        let f = &seq.frames[i];
        if !on_frame(&app, &target, &id, i + 1) {
            return;
        }
        let wait_ms = match &to {
            None => {
                let (from, speed) = {
                    let s = st.settings.lock().unwrap().clone();
                    let rt = st.rt.lock().unwrap();
                    let from = match (&rt.display, &rt.layout) {
                        (Some(d), _) => d.clone(),
                        // The same picture again flips from blank (see show_message).
                        (None, Some(l)) if *l != f.layout => l.clone(),
                        _ => blank(&s),
                    };
                    (from, s.speed)
                };
                let flip = vestaboard::flip_ms(&from, &f.layout, &f.tr, speed);
                let (a, l, tr, mid) = (app.clone(), f.layout.clone(), f.tr.clone(), format!("{id}-{}", i + 1));
                let _ = app.run_on_main_thread(move || show_message(&a, l, Some(mid), "Sequence", tr));
                flip as f64 + f.hold * 1000.0
            }
            Some((b, key)) => {
                if let Err(e) = send::post(&st.http, b, key, &f.layout, &f.tr).await {
                    return end_player(&app, &target, &id, Some(format!("{}: {e}", b.name)));
                }
                let from = prev.take().unwrap_or_else(|| vec![vec![0; b.cols]; b.rows]);
                let flip = vestaboard::flip_ms(&from, &f.layout, &f.tr, 1.0);
                (flip as f64 + f.hold * 1000.0).max(b.min_hold() * 1000.0)
            }
        };
        prev = Some(f.layout.clone());
        tokio::time::sleep(Duration::from_millis(wait_ms as u64)).await;
        i += 1;
        if i == seq.frames.len() {
            if !seq.looping {
                break;
            }
            i = 0;
        }
    }
    end_player(&app, &target, &id, None);
}

// ---------------------------------------------------------------- saved boards and Send

fn find_board(app: &AppHandle, id: &str) -> Result<SavedBoard, String> {
    let st = app.state::<AppState>();
    let boards = st.boards.lock().unwrap();
    boards.iter().find(|b| b.id == id).cloned().ok_or_else(|| "That board was removed".to_string())
}

fn board_key(app: &AppHandle, id: &str) -> String {
    let st = app.state::<AppState>();
    if let Some(k) = st.board_keys.lock().unwrap().get(id) {
        return k.clone();
    }
    let k = secrets::get(&secrets::board_account(id));
    st.board_keys.lock().unwrap().insert(id.to_string(), k.clone());
    k
}

fn save_boards(app: &AppHandle) {
    let st = app.state::<AppState>();
    let boards = st.boards.lock().unwrap().clone();
    boards::save(&st.boards_path, &boards);
    broadcast(app);
}

/// Adds a board, or updates the saved one at the same address. The key goes to the
/// credential store only.
fn keep_board(app: &AppHandle, mut b: SavedBoard, key: &str) -> Result<SavedBoard, String> {
    let st = app.state::<AppState>();
    let existing = {
        let boards = st.boards.lock().unwrap();
        boards.iter().find(|o| o.kind != "cloud" && !b.host.is_empty() && o.host == b.host).map(|o| (o.id.clone(), o.name.clone()))
    };
    if let Some((id, name)) = existing {
        b.id = id;
        if b.name.is_empty() {
            b.name = name;
        }
    }
    secrets::set(&secrets::board_account(&b.id), key)?;
    st.board_keys.lock().unwrap().insert(b.id.clone(), key.to_string());
    {
        let mut boards = st.boards.lock().unwrap();
        match boards.iter_mut().find(|o| o.id == b.id) {
            Some(o) => *o = b.clone(),
            None => boards.push(b.clone()),
        }
    }
    save_boards(app);
    Ok(b)
}

/// Shows a message on this board or a saved one, as a normal post: it ends a running sequence.
async fn send_to_target(app: &AppHandle, target: &str, layout: Grid, tr: Transition, src: &str) -> Result<(), String> {
    if target == HERE {
        stop_player(app, HERE);
        let (a, src, id) = (app.clone(), src.to_string(), next_id(app, "msg"));
        return app.run_on_main_thread(move || show_message(&a, layout, Some(id), &src, tr)).map_err(|e| e.to_string());
    }
    let b = find_board(app, target)?;
    stop_player(app, target);
    let key = board_key(app, &b.id);
    send::post(&app.state::<AppState>().http, &b, &key, &layout, &tr).await
}

/// What a board shows right now.
#[cfg_attr(not(windows), allow(dead_code))]
async fn read_target(app: &AppHandle, target: &str) -> Result<Grid, String> {
    let st = app.state::<AppState>();
    if target == HERE {
        let s = st.settings.lock().unwrap().clone();
        return Ok(st.rt.lock().unwrap().layout.clone().unwrap_or_else(|| blank(&s)));
    }
    let b = find_board(app, target)?;
    let key = board_key(app, &b.id);
    let r = if b.kind == "cloud" {
        vestaboard::read_cloud(&st.http, &key).await?
    } else {
        vestaboard::read_local(&st.http, &b.host, &key).await?
    };
    Ok(r.layout)
}

#[tauri::command]
async fn discover_boards(app: AppHandle) -> Vec<boards::Found> {
    let skip = {
        let st = app.state::<AppState>();
        let port = st.server.lock().unwrap().as_ref().map(|h| h.addr.port());
        server::lan_ip().zip(port)
    };
    tauri::async_runtime::spawn_blocking(move || boards::discover(Duration::from_secs(3), skip)).await.unwrap_or_default()
}

/// Adds a board by address and key, finding out what kind of board it is.
#[tauri::command]
async fn add_board(app: AppHandle, host: String, key: String, name: String) -> Result<SavedBoard, String> {
    let key = key.trim().to_string();
    let p = boards::probe(&app.state::<AppState>().http, &host, &key).await?;
    let fallback = if p.kind == "rattleboard" { "Rattleboard" } else { "Board" };
    let name = Some(name.trim().to_string()).filter(|n| !n.is_empty()).or(p.name).unwrap_or_else(|| fallback.into());
    let b = SavedBoard { id: boards::new_id(), name, kind: p.kind, host: p.host, rows: p.rows, cols: p.cols, adaptive: p.adaptive };
    keep_board(&app, b, &key)
}

#[tauri::command]
async fn add_cloud_board(app: AppHandle, token: String, name: String) -> Result<SavedBoard, String> {
    let token = token.trim().to_string();
    let r = vestaboard::read_cloud(&app.state::<AppState>().http, &token).await?;
    let name = Some(name.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Cloud board".into());
    let (rows, cols) = vestaboard::dims_of(&r.layout);
    let b = SavedBoard { id: boards::new_id(), name, kind: "cloud".into(), host: String::new(), rows, cols, adaptive: false };
    keep_board(&app, b, &token)
}

/// What a pasted pair link would add, for the confirmation before saving it (no key).
#[tauri::command]
fn read_pair_link(link: String) -> Result<Value, String> {
    let l = boards::parse_pair(&link)?;
    Ok(json!({ "name": l.name, "host": l.host, "rows": l.rows, "cols": l.cols }))
}

/// Adds the board in a rattleboard://pair link, as shown by another Rattleboard's QR code.
#[tauri::command]
async fn add_pair_link(app: AppHandle, link: String) -> Result<SavedBoard, String> {
    let l = boards::parse_pair(&link)?;
    let p = boards::probe(&app.state::<AppState>().http, &l.host, &l.key).await?;
    let b = SavedBoard { id: boards::new_id(), name: l.name, kind: p.kind, host: p.host, rows: p.rows, cols: p.cols, adaptive: p.adaptive };
    keep_board(&app, b, &l.key)
}

/// Looks again at what a saved board is (an older Rattleboard may have been updated).
#[tauri::command]
async fn check_board(app: AppHandle, id: String) -> Result<SavedBoard, String> {
    let mut b = find_board(&app, &id)?;
    let key = board_key(&app, &id);
    let http = app.state::<AppState>().http.clone();
    if b.kind == "cloud" {
        let r = vestaboard::read_cloud(&http, &key).await?;
        (b.rows, b.cols) = vestaboard::dims_of(&r.layout);
    } else {
        let p = boards::probe(&http, &b.host, &key).await?;
        (b.kind, b.rows, b.cols, b.adaptive) = (p.kind, p.rows, p.cols, p.adaptive);
    }
    let st = app.state::<AppState>();
    let changed = {
        let mut boards = st.boards.lock().unwrap();
        match boards.iter_mut().find(|o| o.id == id) {
            Some(o) if *o != b => {
                *o = b.clone();
                true
            }
            _ => false,
        }
    };
    if changed {
        save_boards(&app);
    }
    Ok(b)
}

#[tauri::command]
fn rename_board(app: AppHandle, st: State<AppState>, id: String, name: String) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the board a name".into());
    }
    if let Some(b) = st.boards.lock().unwrap().iter_mut().find(|b| b.id == id) {
        b.name = name.to_string();
    }
    save_boards(&app);
    Ok(())
}

#[tauri::command]
fn remove_board(app: AppHandle, st: State<AppState>, id: String) -> Result<(), String> {
    stop_player(&app, &id);
    secrets::set(&secrets::board_account(&id), "")?;
    st.board_keys.lock().unwrap().remove(&id);
    st.boards.lock().unwrap().retain(|b| b.id != id);
    {
        let mut groups = st.groups.lock().unwrap();
        for g in groups.iter_mut() {
            g.members.retain(|m| *m != id);
        }
        send::save_groups(&st.groups_path, &groups);
    }
    {
        let mut s = st.settings.lock().unwrap();
        if s.notify_target == id || s.send_target == id {
            if s.notify_target == id {
                s.notify_target = HERE.into();
            }
            if s.send_target == id {
                s.send_target = HERE.into();
            }
            s.save(&st.settings_path);
        }
    }
    save_boards(&app);
    Ok(())
}

/// The size a board lays text out at, and whether it shows each message at its own size.
fn target_fit(app: &AppHandle, id: &str) -> Result<((usize, usize), bool), String> {
    if id == HERE {
        let s = app.state::<AppState>().settings.lock().unwrap().clone();
        Ok((s.dims(), s.adaptive))
    } else {
        find_board(app, id).map(|b| ((b.rows, b.cols), b.adaptive))
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn target_dims(app: &AppHandle, id: &str) -> Result<(usize, usize), String> {
    target_fit(app, id).map(|f| f.0)
}

/// The boards a target ("this", a board, "group:<id>" or "all") stands for.
fn destinations(app: &AppHandle, target: &str) -> Result<Vec<String>, String> {
    let st = app.state::<AppState>();
    let boards = st.boards.lock().unwrap().clone();
    let groups = st.groups.lock().unwrap().clone();
    send::destinations(target, &boards, &groups)
}

fn target_name(app: &AppHandle, target: &str) -> String {
    let st = app.state::<AppState>();
    if target == HERE {
        return "This board".into();
    }
    if target == send::ALL {
        return "All boards".into();
    }
    if let Some(gid) = target.strip_prefix(send::GROUP_PREFIX) {
        let groups = st.groups.lock().unwrap();
        return groups.iter().find(|g| g.id == gid).map(|g| g.name.clone()).unwrap_or_else(|| "Group".into());
    }
    let boards = st.boards.lock().unwrap();
    boards.iter().find(|b| b.id == target).map(|b| b.name.clone()).unwrap_or_else(|| "Board".into())
}

/// How a send went on one board, for Compose.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Delivery {
    batch: String,
    target: String,
    name: String,
    /// "queued" (waiting for a real board's minimum gap) | "sent" | "failed" | "rate-limited"
    status: String,
    error: Option<String>,
}

/// Real Vestaboards drop messages that come too fast, so each waits for its turn.
fn queue_turn(app: &AppHandle, b: &SavedBoard) -> Duration {
    if b.kind == "rattleboard" {
        return Duration::ZERO;
    }
    let st = app.state::<AppState>();
    let mut sent = st.sent_at.lock().unwrap();
    let now = Instant::now();
    let turn = sent.get(&b.id).map(|t| *t + Duration::from_secs_f64(b.min_hold())).filter(|t| *t > now).unwrap_or(now);
    sent.insert(b.id.clone(), turn);
    turn - now
}

/// One board's copy of a message: laid out for it, queued if it needs a gap, then sent.
async fn deliver(app: AppHandle, batch: String, id: String, message: Value) -> Result<(), String> {
    let (fit, adaptive) = target_fit(&app, &id)?;
    let (layout, tr) = server::message(&message, fit, adaptive)?;
    if id == HERE {
        return send_to_target(&app, HERE, layout, tr, "Compose").await;
    }
    let b = find_board(&app, &id)?;
    let wait = queue_turn(&app, &b);
    if !wait.is_zero() {
        let d = Delivery { batch, target: id.clone(), name: b.name.clone(), status: "queued".into(), error: None };
        let _ = app.emit("rb://delivery", d);
        tokio::time::sleep(wait).await;
    }
    send_to_target(&app, &id, layout, tr, "Compose").await
}

/// Compose → Send, and Message History → Send to: `message` is a POST body ({"characters"}
/// or {"text"}, plus a transition and a size hint) for one board, a group or all of them.
/// Every board gets its copy at once, laid out for its size (see `server::message`), and
/// one failing never holds up the rest. The target becomes the default for next time;
/// `recent` adds the message to Compose's list.
#[tauri::command]
async fn send_message(app: AppHandle, target: String, message: Value, recent: Option<bool>) -> Result<Vec<Delivery>, String> {
    let dests = destinations(&app, &target)?;
    // Check the message once, so a mistake is one error rather than one per board.
    let (fit, adaptive) = target_fit(&app, &dests[0])?;
    let (first, _) = server::message(&message, fit, adaptive)?;
    let batch = next_id(&app, "send");
    let (a, b, m) = (app.clone(), batch.clone(), message.clone());
    let results = send::fan_out(dests, Duration::from_secs(40), move |id| deliver(a.clone(), b.clone(), id, m.clone())).await;
    let out: Vec<Delivery> = results
        .into_iter()
        .map(|(id, r)| {
            let (status, error) = match r {
                Ok(()) => ("sent", None),
                Err(e) => (send::status_of(&e), Some(e)),
            };
            Delivery { batch: batch.clone(), name: target_name(&app, &id), target: id, status: status.into(), error }
        })
        .collect();
    let st = app.state::<AppState>();
    {
        let mut s = st.settings.lock().unwrap();
        if s.send_target != target {
            s.send_target = target.clone();
            s.save(&st.settings_path);
        }
    }
    if recent.unwrap_or(false) && out.iter().any(|d| d.status == "sent") {
        let mut list = read_recent(&st);
        // The grid that was sent, newest first, each layout once.
        list.retain(|r| r.layout != first);
        list.insert(0, RecentEntry { at: now_ms(), target_name: target_name(&app, &target), layout: first });
        list.truncate(RECENT_MAX);
        if let Ok(s) = serde_json::to_string(&list) {
            let _ = std::fs::write(&st.recent_path, s);
        }
    }
    broadcast(&app);
    Ok(out)
}

fn read_recent(st: &AppState) -> Vec<RecentEntry> {
    std::fs::read_to_string(&st.recent_path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

#[tauri::command]
fn get_recent(st: State<AppState>) -> Vec<RecentEntry> {
    read_recent(&st)
}

fn rattleboard_url(b: &SavedBoard, path: &str) -> String {
    format!("{}/rattleboard/{path}", vestaboard::base_url(&b.host, boards::RATTLEBOARD_PORT))
}

/// One board's part of a sequence. Another Rattleboard plays it by itself; for this board and
/// real Vestaboards (under their minimum gap), this computer plays it.
async fn start_on(app: AppHandle, via: String, id: String, sequence: Value) -> Result<(), String> {
    let (fit, adaptive) = target_fit(&app, &id)?;
    let seq = sequence::parse(&sequence, fit, adaptive)?;
    let via_name = target_name(&app, &via);
    if id == HERE {
        start_player(&app, HERE, "This board", seq, None, (&via, &via_name));
        return Ok(());
    }
    let b = find_board(&app, &id)?;
    let key = board_key(&app, &b.id);
    if b.kind == "rattleboard" {
        let http = app.state::<AppState>().http.clone();
        let res = http
            .post(rattleboard_url(&b, "sequence"))
            .header("X-Vestaboard-Local-Api-Key", &key)
            .json(&seq.to_json())
            .send()
            .await
            .map_err(vestaboard::describe)?;
        match res.status().as_u16() {
            201 | 200 => {
                stop_player(&app, &id);
                return Ok(());
            }
            400 => {
                let v: Value = res.json().await.unwrap_or_default();
                return Err(v["error"].as_str().unwrap_or("The board refused the sequence").to_string());
            }
            401 | 403 => return Err("Key was rejected".into()),
            // Not a protocol v2 board after all: play it from here.
            _ => {}
        }
    }
    let hold = b.min_hold();
    let seq = Sequence {
        frames: seq.frames.into_iter().map(|f| sequence::Frame { hold: f.hold.max(hold), ..f }).collect(),
        ..seq
    };
    let name = b.name.clone();
    start_player(&app, &id, &name, seq, Some((b, key)), (&via, &via_name));
    Ok(())
}

/// Compose → Play sequence, on one board, a group or all of them.
#[tauri::command]
async fn play_sequence(app: AppHandle, target: String, sequence: Value) -> Result<Vec<Delivery>, String> {
    let dests = destinations(&app, &target)?;
    let (fit, adaptive) = target_fit(&app, &dests[0])?;
    sequence::parse(&sequence, fit, adaptive)?;
    let batch = next_id(&app, "play");
    let (a, v, s) = (app.clone(), target.clone(), sequence.clone());
    let results = send::fan_out(dests, send::TIMEOUT, move |id| start_on(a.clone(), v.clone(), id, s.clone())).await;
    Ok(results
        .into_iter()
        .map(|(id, r)| Delivery {
            batch: batch.clone(),
            name: target_name(&app, &id),
            status: if r.is_ok() { "sent".into() } else { "failed".into() },
            error: r.err(),
            target: id,
        })
        .collect())
}

/// Stops what one board, a group or all of them are playing.
#[tauri::command]
async fn stop_sequence(app: AppHandle, target: String) -> Result<(), String> {
    let dests = destinations(&app, &target).unwrap_or_else(|_| vec![target.clone()]);
    let a = app.clone();
    let results = send::fan_out(dests, send::TIMEOUT, move |id| {
        let app = a.clone();
        async move {
            if stop_player(&app, &id) || id == HERE {
                return Ok(());
            }
            let b = find_board(&app, &id)?;
            if b.kind != "rattleboard" {
                return Ok(());
            }
            let key = board_key(&app, &b.id);
            let http = app.state::<AppState>().http.clone();
            let res = http.delete(rattleboard_url(&b, "sequence")).header("X-Vestaboard-Local-Api-Key", &key).send().await.map_err(vestaboard::describe)?;
            vestaboard::ok_or_status(res).await
        }
    })
    .await;
    results.into_iter().find_map(|(_, r)| r.err()).map_or(Ok(()), Err)
}

/// What another Rattleboard is playing, for Compose.
#[tauri::command]
async fn remote_sequence(app: AppHandle, target: String) -> Result<Value, String> {
    let b = find_board(&app, &target)?;
    if b.kind != "rattleboard" {
        return Ok(json!({ "running": false }));
    }
    let key = board_key(&app, &b.id);
    let http = app.state::<AppState>().http.clone();
    let res = http.get(rattleboard_url(&b, "sequence")).header("X-Vestaboard-Local-Api-Key", &key).send().await.map_err(vestaboard::describe)?;
    if !res.status().is_success() {
        return Err(format!("Board answered HTTP {}", res.status().as_u16()));
    }
    res.json().await.map_err(|_| "Response wasn't JSON".to_string())
}

/// Settings → Boards → Groups: the whole list, as edited.
#[tauri::command]
fn set_groups(app: AppHandle, st: State<AppState>, groups: Vec<send::Group>) {
    let boards = st.boards.lock().unwrap().clone();
    let groups = send::clean_groups(groups, &boards);
    send::save_groups(&st.groups_path, &groups);
    *st.groups.lock().unwrap() = groups.clone();
    {
        // Targets that pointed at a deleted group fall back to this board.
        let mut s = st.settings.lock().unwrap();
        let gone = |t: &str| t.strip_prefix(send::GROUP_PREFIX).is_some_and(|g| !groups.iter().any(|x| x.id == g));
        if gone(&s.send_target) || gone(&s.notify_target) {
            if gone(&s.send_target) {
                s.send_target = HERE.into();
            }
            if gone(&s.notify_target) {
                s.notify_target = HERE.into();
            }
            s.save(&st.settings_path);
        }
    }
    broadcast(&app);
}

/// The same layout POST {"text"} produces, for Compose's live preview.
#[tauri::command]
fn layout_text(text: String, rows: usize, cols: usize) -> Grid {
    vestaboard::text_layout(&text, rows.clamp(1, vestaboard::MAX_ROWS), cols.clamp(1, vestaboard::MAX_COLS))
}

#[tauri::command]
fn get_drafts(st: State<AppState>) -> Value {
    std::fs::read_to_string(&st.drafts_path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .filter(|v| v.is_array())
        .unwrap_or_else(|| json!([]))
}

/// Compose drafts: text, grids and sequences, never keys.
#[tauri::command]
fn save_drafts(st: State<AppState>, drafts: Value) -> Result<(), String> {
    if !drafts.is_array() {
        return Err("Drafts must be a list".into());
    }
    if let Some(dir) = st.drafts_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let s = serde_json::to_string(&drafts).map_err(|e| e.to_string())?;
    std::fs::write(&st.drafts_path, s).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- pairing

#[derive(Serialize)]
struct PairQr {
    svg: String,
    url: String,
}

/// The QR code a phone scans to add this board. Only shown on demand, so it isn't part of
/// the state snapshot; the key it carries never leaves this computer otherwise.
#[tauri::command]
fn pair_qr(st: State<AppState>) -> Result<PairQr, String> {
    let s = st.settings.lock().unwrap().clone();
    if !s.server_lan {
        return Err("Turn on Other devices first".into());
    }
    let base = {
        let sv = st.server_status.lock().unwrap();
        if !sv.running {
            return Err("The API server isn't running".into());
        }
        sv.urls.first().filter(|u| !u.contains("127.0.0.1")).cloned()
    };
    let base = base.ok_or("No network address found; is this computer on Wi-Fi or Ethernet?")?;
    let key = st.secrets.lock().unwrap().server_key.clone();
    let (rows, cols) = s.dims();
    let name = st.device.clone();
    let url = boards::pair_url(&base, &key, &name, rows, cols);
    let code = qrcode::QrCode::new(url.as_bytes()).map_err(|e| e.to_string())?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(232, 232)
        .dark_color(qrcode::render::svg::Color("#1b1b1d"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();
    let svg = svg.find("<svg").map(|i| svg[i..].to_string()).unwrap_or(svg);
    Ok(PairQr { svg, url })
}

// ---------------------------------------------------------------- notifications (Windows)

#[tauri::command]
fn notify_request_access(app: AppHandle) -> Result<(), String> {
    notify::request_access(&app)
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_state(st: State<AppState>) -> Snapshot {
    snapshot(&st)
}

#[tauri::command]
fn update_settings(app: AppHandle, patch: serde_json::Value) -> Result<(), String> {
    apply_settings(&app, patch)
}

fn apply_settings(app: &AppHandle, patch: serde_json::Value) -> Result<(), String> {
    let st = app.state::<AppState>();
    // Window queries happen before any lock is taken (see show_message).
    let board_pos = board(app).filter(|w| w.is_visible().unwrap_or(false)).and_then(|w| w.outer_position().ok());
    let (old, new) = {
        let mut s = st.settings.lock().unwrap();
        let old = s.clone();
        let mut new = s.merge(patch)?;
        if old.mode == "toast" && new.floating() {
            // Pinning a visible toast keeps it right where it is.
            if let Some(p) = board_pos {
                new.custom = Point { x: p.x as f64, y: p.y as f64 };
            }
        }
        if !old.fullscreen() && new.fullscreen() {
            new.windowed_mode = old.mode.clone();
        }
        *s = new.clone();
        new.save(&st.settings_path);
        (old, new)
    };
    // Into or out of full screen first, so a pop-up or widget is then placed as usual.
    let screen_moved = old.screen_name != new.screen_name || old.screen_pos != new.screen_pos;
    if old.fullscreen() != new.fullscreen() || (new.fullscreen() && screen_moved) {
        if new.fullscreen() {
            cover_screen(app);
        } else {
            uncover_screen(app);
        }
    } else if new.fullscreen() && old.finish != new.finish {
        if let Some(w) = board(app) {
            let _ = w.set_background_color(Some(backdrop(&new)));
        }
    }
    if old.mode != new.mode || old.float_layer != new.float_layer {
        apply_layering(app);
    }
    if old.mode != new.mode || old.position != new.position || old.custom != new.custom {
        place_board(app, None);
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
        set_autostart(app, new.launch_login);
    }
    if old.update_check != new.update_check {
        updates::sync_tray(app);
    }
    broadcast(app);
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

/// Boards to mirror, for setup: Rattleboards announcing themselves first, then a Vestaboard
/// at its default vestaboard.local name.
#[tauri::command]
async fn scan_boards(app: AppHandle) -> Vec<Found> {
    let (rattleboards, vestaboards) = tokio::join!(discover_boards(app), vestaboard::scan());
    let ours = rattleboards.into_iter().map(|f| Found { name: f.name, ip: f.host });
    ours.chain(vestaboards.into_iter().map(|(name, ip)| Found { name: format!("{name} (Local API)"), ip })).collect()
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
        rt.transition = Transition::default();
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

/// Opens Rattleboard's page in the default browser. Only ever that one address, so the
/// pages need no opener permission of their own.
#[tauri::command]
fn open_site(app: AppHandle) {
    use tauri_plugin_opener::OpenerExt;
    let _ = app.opener().open_url(SITE, None::<&str>);
}

/// Opens the Ko-fi page for supporting Rattleboard. Like open_site, a single fixed address.
#[tauri::command]
fn open_kofi(app: AppHandle) {
    use tauri_plugin_opener::OpenerExt;
    let _ = app.opener().open_url(KOFI, None::<&str>);
}

/// Opens a section of the README on GitHub, e.g. "home-assistant". Only a plain anchor name
/// is accepted, so the address can't be steered anywhere else.
#[tauri::command]
fn open_docs(app: AppHandle, section: String) {
    use tauri_plugin_opener::OpenerExt;
    if section.is_empty() || section.len() > 60 || !section.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return;
    }
    let _ = app.opener().open_url(format!("https://github.com/slastrina/rattleboard#{section}"), None::<&str>);
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
    fullscreen::keep_awake(false);
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
    if app.state::<AppState>().settings.lock().unwrap().fullscreen() {
        // The window is the display's size; the page fits the board into it.
        return;
    }
    let size = tauri::LogicalSize::new(width + SHADOW * 2.0, height + SHADOW * 2.0);
    let _ = w.set_size(size);
    // A message of another size resizes the board: a pop-up stays anchored to its corner and a
    // pinned widget keeps its top-left, both kept on screen.
    place_board(&app, Some(size.to_physical(w.scale_factor().unwrap_or(1.0))));
}

/// Where the pointer really is, relative to the calling window's content, in CSS pixels; None
/// while the window is hidden or the position can't be read. The board checks it because an
/// unfocused, always-on-top window often misses mouseleave, and a stuck hover would hold a
/// pop-up's countdown for ever. Only window queries, no lock.
#[tauri::command]
fn cursor_in_window(app: AppHandle, window: tauri::WebviewWindow) -> Option<Point> {
    if !window.is_visible().unwrap_or(false) {
        return None;
    }
    let c = app.cursor_position().ok()?;
    let p = window.inner_position().ok()?;
    let f = window.scale_factor().ok().filter(|f| *f > 0.0).unwrap_or(1.0);
    Some(Point { x: (c.x - p.x as f64) / f, y: (c.y - p.y as f64) / f })
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
    place_board(&app, None);
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
    // Wayland doesn't let apps place their own windows, so the pop-up couldn't go in its
    // corner; run under XWayland where there is one (GTK falls back to Wayland if not).
    // RATTLEBOARD_NATIVE_WAYLAND=1, or a GDK_BACKEND of the user's own, keeps native Wayland.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("GDK_BACKEND").is_none() && std::env::var_os("RATTLEBOARD_NATIVE_WAYLAND").is_none() {
        std::env::set_var("GDK_BACKEND", "x11,wayland");
    }
    #[cfg(target_os = "macos")]
    fullscreen::no_system_fullscreen_item();
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder.menu(app_menu).on_menu_event(|app, e| {
        if e.id() == "fullscreen" {
            toggle_fullscreen(app.clone());
        }
    });
    // A profile (debug builds) may run beside the real app, or beside another profile.
    let builder = if profile().is_none() {
        builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let setup = !app.state::<AppState>().settings.lock().unwrap().setup_done;
            open_panel(app, if setup { "setup" } else { "settings" });
        }))
    } else {
        builder
    };
    let app = builder
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_opener::init())
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
            cursor_in_window,
            board_drag_start,
            board_drag_move,
            board_drag_end,
            tray_resize,
            panel_resize,
            regenerate_server_key,
            discover_boards,
            add_board,
            add_cloud_board,
            add_pair_link,
            check_board,
            rename_board,
            remove_board,
            send_message,
            play_sequence,
            stop_sequence,
            remote_sequence,
            layout_text,
            get_drafts,
            save_drafts,
            get_recent,
            pair_qr,
            notify_request_access,
            set_groups,
            read_pair_link,
            toggle_fullscreen,
            list_screens,
            hide_pointer,
            open_site,
            open_kofi,
            open_docs,
            updates::check_for_updates,
            updates::open_update,
            get_sources,
            save_sources,
            add_source,
            import_plugin,
            read_plugin_link,
            remove_plugin,
            approve_plugin,
            set_source_secret,
            reveal_source_secret,
            source_preview,
            sources_next,
            export_source,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let cfg = profile_dir(app.path().app_config_dir()?);
            let data = profile_dir(app.path().app_data_dir()?);
            if profile().is_some() {
                let _ = secrets::PROFILE_FILE.set(cfg.join("secrets.json"));
            }
            let settings_path = cfg.join("settings.json");
            let history_path = data.join("history.json");
            let boards_path = cfg.join("boards.json");
            let drafts_path = cfg.join("drafts.json");
            let groups_path = cfg.join("groups.json");
            let recent_path = data.join("recent.json");
            let engine = Arc::new(sources::Engine::new(sources::Paths {
                config: Some(cfg.join("sources.json")),
                plugins: Some(cfg.join("plugins.json")),
                seen: Some(data.join("seen.json")),
            }));
            let settings = Settings::load(&settings_path).at_launch();
            let history: Vec<HistoryEntry> = std::fs::read_to_string(&history_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            let setup_done = settings.setup_done;
            let floating = settings.floating();
            let full = settings.fullscreen();
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
                advert: Mutex::new(None),
                boards: Mutex::new(boards::load(&boards_path)),
                board_keys: Mutex::new(HashMap::new()),
                players: Mutex::new(HashMap::new()),
                groups: Mutex::new(send::load_groups(&groups_path)),
                sent_at: Mutex::new(HashMap::new()),
                notify: Mutex::new(NotifyInfo {
                    supported: notify::SUPPORTED,
                    access: if notify::SUPPORTED { "unspecified" } else { "unavailable" }.into(),
                    apps: vec![],
                }),
                sources: engine.clone(),
                hub: Mutex::new(HashMap::new()),
                settings_path,
                history_path,
                boards_path,
                drafts_path,
                recent_path,
                groups_path,
                device: device_name(),
            });

            let h = app.handle().clone();
            {
                let h = h.clone();
                engine.on_change(Arc::new(move || {
                    let _ = h.emit("rb://sources", ());
                    broadcast(&h);
                }));
            }
            create_board(&h)?;
            #[cfg(target_os = "macos")]
            if let Some(p) = board(&h).and_then(|w| w.ns_window().ok()) {
                let a = h.clone();
                fullscreen::watch_escape(p, move || {
                    let full = a.state::<AppState>().settings.lock().unwrap().fullscreen();
                    if full {
                        toggle_fullscreen(a.clone());
                    }
                    full
                });
            }
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
                use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
                let full_item = CheckMenuItem::with_id(app, "fullscreen", "Full screen", true, full, Some("F11"))?;
                app.manage(FullItem(full_item.clone()));
                let menu = Menu::with_items(
                    app,
                    &[
                        &MenuItem::with_id(app, "show", "Show board", true, None::<&str>)?,
                        &MenuItem::with_id(app, "next", "Next source", true, None::<&str>)?,
                        &full_item,
                        &MenuItem::with_id(app, "compose", "Compose…", true, None::<&str>)?,
                        &MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?,
                        &PredefinedMenuItem::separator(app)?,
                        &MenuItem::with_id(app, "site", "Rattleboard on iokig.com", true, None::<&str>)?,
                        &MenuItem::with_id(app, "kofi", "Support Rattleboard on Ko-fi", true, None::<&str>)?,
                        &PredefinedMenuItem::separator(app)?,
                        &MenuItem::with_id(app, "quit", "Quit Rattleboard", true, None::<&str>)?,
                    ],
                )?;
                app.manage(TrayMenu(menu.clone()));
                tray = tray.menu(&menu).on_menu_event(|app, e| match e.id().as_ref() {
                    "update" => updates::open_update(app.clone()),
                    "show" => present_board(app, true),
                    "next" => sources_next(app.clone()),
                    "fullscreen" => toggle_fullscreen(app.clone()),
                    "compose" => open_panel(app, "compose"),
                    "settings" => open_panel(app, "settings"),
                    "site" => open_site(app.clone()),
                    "kofi" => open_kofi(app.clone()),
                    "quit" => quit(app.clone()),
                    _ => {}
                });
            }
            if let Some(img) = tray_image(false) {
                tray = tray.icon(img).icon_as_template(OS == "mac");
            }
            // Linux: only where a tray icon would show (see `fullscreen::linux::tray_available`);
            // otherwise Settings opens as a normal window instead, and launching Rattleboard
            // again brings it back.
            #[cfg(target_os = "linux")]
            let has_tray = fullscreen::linux::tray_available();
            #[cfg(not(target_os = "linux"))]
            let has_tray = true;
            HAS_TRAY.store(has_tray, std::sync::atomic::Ordering::Relaxed);
            if has_tray {
                tray.build(app)?;
            } else {
                eprintln!("Rattleboard: no system tray here (on GNOME, install the AppIndicator extension); opening Settings as a window");
            }

            tauri::async_runtime::spawn(poll_loop(h.clone()));
            tauri::async_runtime::spawn(sources_loop(h.clone()));
            tauri::async_runtime::spawn(watch_screens(h.clone()));
            notify::start(&h);
            tauri::async_runtime::spawn(updates::run(h.clone()));
            let h2 = h.clone();
            tauri::async_runtime::spawn(async move { restart_server(&h2).await });
            // Debug profiles: RATTLEBOARD_OPEN=settings/sources opens a window at launch, for
            // trying things out (and screenshots) without clicking through the tray.
            let open = profile().and(std::env::var("RATTLEBOARD_OPEN").ok());
            if let Some(view) = open {
                open_panel(&h, &view);
            } else if !setup_done {
                open_panel(&h, "setup");
            } else if full {
                // Once the event loop runs and the window exists on screen.
                let h2 = h.clone();
                let _ = h.run_on_main_thread(move || cover_screen(&h2));
            } else if floating {
                // A floating board is always on screen, even before the first check answers.
                present_board(&h, false);
            }
            if !has_tray && setup_done && !full {
                open_panel(&h, "settings");
            }
            Ok(())
        })
        .on_window_event(|w, e| {
            if w.label() == "panel" {
                if let WindowEvent::Destroyed = e {
                    // Settings or Compose closed over a full-screen board: it's back on top.
                    let app = w.app_handle();
                    if app.state::<AppState>().settings.lock().unwrap().fullscreen() {
                        if let Some(b) = board(app) {
                            let _ = b.set_always_on_top(true);
                            over_menu_bar(&b);
                            focus_board(&b);
                        }
                    }
                }
            }
            if w.label() == "board" {
                if let WindowEvent::Focused(true) = e {
                    // A click that brings Rattleboard forward leaves keys with the window, not
                    // the page, so Esc would go nowhere.
                    let app = w.app_handle();
                    if app.state::<AppState>().settings.lock().unwrap().fullscreen() {
                        if let Some(b) = board(app) {
                            let _ = b.as_ref().set_focus();
                        }
                    }
                }
            }
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
