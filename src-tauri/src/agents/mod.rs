//! Agents: AI agent apps (Claude Code, Cursor, Codex, …) put things on the board as MCP tool
//! calls. This module has the command-line modes (`cli`: `--mcp`, `--notify`), the API
//! server's view of saved boards for them (`api_boards`, `api_send`), and Settings → Agents,
//! which connects each app by editing its config (`harness`, with `jsonc` for the edits).

pub mod cli;
pub mod harness;
pub mod jsonc;

use crate::{AppState, Delivery, HERE};
use harness::{Change, Env, Launch, State, Want};
use rattleboard_core::{send, server};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Manager};

// ---------------------------------------------------------------- API server

/// GET /rattleboard/boards: this board, saved boards and groups, by name.
pub fn api_boards(app: &AppHandle) -> Value {
    let st = app.state::<AppState>();
    let s = st.settings.lock().unwrap().clone();
    let boards = st.boards.lock().unwrap().clone();
    let groups = st.groups.lock().unwrap().clone();
    let (rows, cols) = s.dims();
    let mut list = vec![json!({ "target": HERE, "name": "This board", "kind": "this", "rows": rows, "cols": cols, "adaptive": s.adaptive })];
    for b in &boards {
        let kind = match b.kind.as_str() {
            "rattleboard" => "rattleboard",
            "cloud" => "cloud board",
            _ => "local board",
        };
        list.push(json!({ "target": b.id, "name": b.name, "kind": kind, "rows": b.rows, "cols": b.cols, "adaptive": b.adaptive }));
    }
    let name = |id: &str| if id == HERE { "This board".to_string() } else { boards.iter().find(|b| b.id == id).map(|b| b.name.clone()).unwrap_or_default() };
    let groups: Vec<Value> = groups
        .iter()
        .map(|g| json!({ "target": format!("{}{}", send::GROUP_PREFIX, g.id), "name": g.name, "members": g.members.iter().map(|m| name(m)).collect::<Vec<_>>() }))
        .collect();
    json!({ "boards": list, "groups": groups })
}

/// POST /rattleboard/send: a message to a saved board, a group or all of them, as Compose
/// sends it (each board gets its own copy, laid out for it), without changing Compose's
/// default target.
pub fn api_send(app: &AppHandle, target: String, message: Value) -> server::SendFut {
    let app = app.clone();
    Box::pin(async move {
        let dests = crate::destinations(&app, &target)?;
        let (fit, adaptive) = crate::target_fit(&app, &dests[0])?;
        server::message(&message, fit, adaptive)?;
        let batch = crate::next_id(&app, "agent");
        let (a, b, m) = (app.clone(), batch.clone(), message.clone());
        let results = send::fan_out(dests, Duration::from_secs(40), move |id| crate::deliver(a.clone(), b.clone(), id, m.clone())).await;
        let out: Vec<Delivery> = results
            .into_iter()
            .map(|(id, r)| {
                let (status, error) = match r {
                    Ok(()) => ("sent", None),
                    Err(e) => (send::status_of(&e), Some(e)),
                };
                Delivery { batch: batch.clone(), name: crate::target_name(&app, &id), target: id, status: status.into(), error }
            })
            .collect();
        crate::broadcast(&app);
        Ok(json!({ "deliveries": out }))
    })
}

// ---------------------------------------------------------------- Settings → Agents

fn launch() -> Launch {
    let exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok().or(Some(p))).unwrap_or_default();
    let mut exe = exe.to_string_lossy().to_string();
    // canonicalize() on Windows gives a \\?\ path, which some apps can't start.
    if let Some(s) = exe.strip_prefix(r"\\?\") {
        exe = s.to_string();
    }
    Launch { exe, profile: crate::profile() }
}

fn state_path(app: &AppHandle) -> Option<PathBuf> {
    Some(crate::profile_dir(app.path().app_config_dir().ok()?).join("agents.json"))
}

fn load_state(app: &AppHandle) -> State {
    state_path(app).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save_state(app: &AppHandle, st: &State) {
    if let Some(p) = state_path(app) {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(s) = serde_json::to_string_pretty(st) {
            let _ = std::fs::write(p, s);
        }
    }
}

/// Where this copy of Rattleboard would be started from, if that's somewhere it won't stay.
fn exe_warning(exe: &str) -> Option<&'static str> {
    if exe.contains("/AppTranslocation/") || exe.starts_with("/Volumes/") {
        Some("Rattleboard is running from the disk image or Downloads. Move it to Applications and open it from there first, so agent apps can find it later.")
    } else {
        None
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessView {
    id: &'static str,
    name: &'static str,
    installed: bool,
    connected: bool,
    alerts_supported: bool,
    alerts_on: bool,
    skill: bool,
    /// Why alerts can't be turned on here, if they can't
    alerts_blocked: Option<String>,
    config: String,
    note: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentsView {
    harnesses: Vec<HarnessView>,
    exe: String,
    warning: Option<&'static str>,
}

#[tauri::command]
pub fn agents_list() -> AgentsView {
    let l = launch();
    let hs = harness::all(&Env::system(), &l);
    let harnesses = hs
        .iter()
        .map(|h| {
            let (connected, alerts_on) = harness::status(h);
            let blocked = (h.id == "codex" && !alerts_on && harness::codex_notify_taken(&h.mcp_file))
                .then(|| "Codex already runs another notify program, which Rattleboard leaves alone.".to_string());
            HarnessView {
                id: h.id,
                name: h.name,
                installed: h.installed(),
                connected,
                alerts_supported: h.has(harness::Part::Alerts),
                alerts_on,
                skill: h.has(harness::Part::Skill),
                alerts_blocked: blocked,
                config: h.mcp_file.to_string_lossy().to_string(),
                note: h.note,
            }
        })
        .collect();
    AgentsView { harnesses, warning: exe_warning(&l.exe), exe: l.exe }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePreview {
    path: String,
    /// "create" | "edit" | "delete"
    action: &'static str,
    diff: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    files: Vec<FilePreview>,
    /// Ties Confirm to exactly these files' contents
    token: String,
}

fn fingerprint(changes: &[Change]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for c in changes {
        (&c.path, &c.before, &c.after).hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

fn plan_for(app: &AppHandle, id: &str, mcp: bool, alerts: bool) -> Result<Vec<Change>, String> {
    let hs = harness::all(&Env::system(), &launch());
    let h = hs.iter().find(|h| h.id == id).ok_or("Unknown agent app")?;
    harness::plan(h, Want { mcp, alerts: alerts && mcp }, &load_state(app))
}

/// What connecting (or disconnecting, or switching alerts) would change, file by file.
#[tauri::command]
pub fn agents_preview(app: AppHandle, id: String, mcp: bool, alerts: bool) -> Result<Preview, String> {
    let changes = plan_for(&app, &id, mcp, alerts)?;
    let files = changes
        .iter()
        .map(|c| FilePreview {
            path: c.path.to_string_lossy().to_string(),
            action: match (&c.before, &c.after) {
                (None, _) => "create",
                (_, None) => "delete",
                _ => "edit",
            },
            diff: harness::diff(c.before.as_deref().unwrap_or(""), c.after.as_deref().unwrap_or("")),
        })
        .collect();
    Ok(Preview { token: fingerprint(&changes), files })
}

/// A copy of `path` next to it, named with the time, before it's changed.
fn backup(path: &std::path::Path) -> Result<(), String> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let name = format!("{}.rattleboard-backup-{stamp}", path.file_name().unwrap_or_default().to_string_lossy());
    std::fs::copy(path, path.with_file_name(name)).map(|_| ()).map_err(|e| format!("Couldn't back up {}: {e}", path.display()))
}

/// Makes the changes the preview showed, after backing up each file it edits. If any file
/// changed since the preview, nothing is written and the preview has to be looked at again.
#[tauri::command]
pub fn agents_apply(app: AppHandle, id: String, mcp: bool, alerts: bool, token: String) -> Result<AgentsView, String> {
    let changes = plan_for(&app, &id, mcp, alerts)?;
    if fingerprint(&changes) != token {
        return Err("A file changed since the preview. Look it over again, then confirm.".into());
    }
    for c in &changes {
        if c.before.is_some() && !c.own {
            backup(&c.path)?;
        }
    }
    let mut st = load_state(&app);
    for c in &changes {
        let key = c.path.to_string_lossy().to_string();
        match &c.after {
            Some(text) => {
                if let Some(d) = c.path.parent() {
                    std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
                }
                std::fs::write(&c.path, text).map_err(|e| format!("Couldn't write {}: {e}", c.path.display()))?;
            }
            None => {
                let _ = std::fs::remove_file(&c.path);
                // The skill's own folder goes with it.
                if let Some(d) = c.path.parent().filter(|d| d.file_name().is_some_and(|n| n == "rattleboard")) {
                    let _ = std::fs::remove_dir(d);
                }
            }
        }
        if c.state == harness::FileState::default() {
            st.remove(&key);
        } else {
            st.insert(key, c.state.clone());
        }
    }
    save_state(&app, &st);
    // Agents reach the board through the API server; this computer only is enough.
    let enabled = app.state::<AppState>().settings.lock().unwrap().server_enabled;
    if mcp && !enabled {
        crate::apply_settings(&app, json!({ "serverEnabled": true }))?;
    }
    Ok(agents_list())
}
