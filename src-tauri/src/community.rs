//! Settings → Sources → Community plugins: browsing github.com/slastrina/rattleboard-plugins
//! (see rattleboard_core::community). index.json is cached in the profile's data folder for a
//! day; installing fetches the plugin's manifest, checks it as a plugin link would be, and the
//! panel then installs it with import_plugin after asking.

use super::{link_json, link_manifest, profile_dir, sources_ctx, AppState};
use rattleboard_core::community::{self, Cache, Entry, Index};
use rattleboard_core::now_ms;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tauri::{AppHandle, Manager};

fn http() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(community::client)
}

/// Debug builds only: RATTLEBOARD_COMMUNITY_BASE=http://127.0.0.1:8099/ reads the repository's
/// raw files from a local copy instead. Release builds always use raw.githubusercontent.com.
fn base() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("RATTLEBOARD_COMMUNITY_BASE").ok().map(|b| b.trim().to_string()).filter(|b| !b.is_empty())
}

fn data_file(app: &AppHandle, name: &str) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| profile_dir(d).join(name))
}

fn source() -> String {
    base().unwrap_or_else(|| community::INDEX_URL.to_string())
}

/// The cached index, whatever its age.
fn cached(app: &AppHandle) -> Option<(Index, i64)> {
    let c = community::read_cache(&data_file(app, "community-index.json")?, &source())?;
    community::parse_index(c.body.as_bytes()).ok().map(|ix| (ix, c.fetched))
}

fn entry(app: &AppHandle, id: &str) -> Result<Entry, String> {
    cached(app).and_then(|(ix, _)| ix.plugins.into_iter().find(|e| e.id == id)).ok_or_else(|| "That plugin isn't in the community directory".to_string())
}

/// The versions installed from the directory (a manifest has no version of its own).
fn recorded(app: &AppHandle) -> HashMap<String, String> {
    data_file(app, "community-installed.json").and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// The directory: the cached copy while it's less than a day old (or always, offline), else
/// fetched again; `refresh` fetches it now. The warning says a refresh failed.
async fn load(app: &AppHandle, refresh: bool) -> Result<(Index, i64, Option<String>), String> {
    let now = now_ms();
    let path = data_file(app, "community-index.json").ok_or("No data folder")?;
    match cached(app) {
        Some((ix, f)) if !refresh && community::is_fresh(f, now) => Ok((ix, f, None)),
        have => match community::get(http(), community::INDEX_URL, base().as_deref(), community::MAX_INDEX).await.and_then(|b| {
            let ix = community::parse_index(&b)?;
            Ok((ix, b))
        }) {
            Ok((ix, body)) => {
                let body = String::from_utf8_lossy(&body).into_owned();
                community::write_cache(&path, &Cache { fetched: now, source: source(), body });
                Ok((ix, now, None))
            }
            Err(e) => match have {
                Some((ix, f)) => Ok((ix, f, Some(format!("Couldn't refresh the list ({e}); showing the copy from earlier.")))),
                None => Err(format!("{} ({e})", community::OFFLINE)),
            },
        },
    }
}

/// The directory, each entry with whether it's installed and whether there's an update.
/// Cached for a day; `refresh` fetches it again now. Offline with a cached copy, that copy is
/// shown with a warning; with none, the error is the offline message. Opening it also checks
/// the installed plugins for updates.
#[tauri::command]
pub async fn community_index(app: AppHandle, refresh: Option<bool>) -> Result<Value, String> {
    let (ix, fetched, warning) = load(&app, refresh.unwrap_or(false)).await?;
    recheck(&app, &ix);
    let installed: HashMap<String, (Option<String>, usize)> =
        app.state::<AppState>().sources.installed().into_iter().map(|(id, v, n)| (id, (v, n))).collect();
    let rec = recorded(&app);
    let plugins: Vec<Value> = ix
        .plugins
        .iter()
        .map(|e| {
            let mut v = serde_json::to_value(e).unwrap_or_default();
            let here = installed.get(&e.id);
            let version = here.and_then(|(v, _)| v.clone().or_else(|| rec.get(&e.id).cloned()));
            v["installed"] = json!(here.is_some());
            v["installedVersion"] = json!(version);
            v["inRotation"] = json!(here.map_or(0, |h| h.1));
            v["update"] = json!(here.is_some() && community::update_available(version.as_deref(), &e.version));
            v["needsApp"] = json!(community::needs_newer_app(e.min_app_version.as_deref(), community::APP_VERSION));
            v
        })
        .collect();
    Ok(json!({ "plugins": plugins, "generated": ix.generated, "fetched": fetched, "warning": warning, "repo": community::REPO_URL }))
}

// ---------------------------------------------------------------- plugin updates

/// How many updates the tray and Settings offer (see `recheck`). An atomic, so the state
/// snapshot reads it without taking another lock.
static OFFERED: AtomicUsize = AtomicUsize::new(0);

pub fn offered() -> usize {
    OFFERED.load(Ordering::Relaxed)
}

fn checks_on(app: &AppHandle) -> bool {
    app.state::<AppState>().settings.lock().unwrap().plugin_update_check
}

/// The plugins installed from the directory (plugins from links or files aren't), with the
/// version recorded then and what their manifests say.
fn installed_from_directory(app: &AppHandle) -> Vec<community::Installed> {
    let st = app.state::<AppState>();
    let rec = recorded(app);
    st.sources
        .installed()
        .into_iter()
        .filter_map(|(id, own, _)| {
            // Only the version recorded by an install from the directory counts.
            let _ = own;
            let version = rec.get(&id).cloned()?;
            let m = st.sources.manifest(&id)?;
            Some(community::Installed { id, version, facts: community::Facts::of_manifest(&m) })
        })
        .collect()
}

fn skips_file(app: &AppHandle) -> Option<PathBuf> {
    data_file(app, "community-skipped.json")
}

fn updates_in(app: &AppHandle, ix: &Index) -> Vec<community::Pending> {
    let skips = skips_file(app).map(|p| community::read_skips(&p)).unwrap_or_default();
    community::pending(ix, &installed_from_directory(app), &skips, community::APP_VERSION)
}

/// Counts the updates in `ix` again and, when the count changed, refreshes the tray and the
/// pages. No lock is held while it does.
fn recheck(app: &AppHandle, ix: &Index) {
    let n = if checks_on(app) { community::offered(&updates_in(app, ix)) } else { 0 };
    if OFFERED.swap(n, Ordering::Relaxed) != n {
        if cfg!(debug_assertions) {
            eprintln!("Rattleboard: plugin updates: {n} offered");
        }
        crate::updates::sync_tray(app);
        crate::broadcast(app);
    }
}

/// Again from the cached directory, after an install, a skip or the setting changing.
pub fn recheck_cached(app: &AppHandle) {
    match cached(app) {
        Some((ix, _)) => recheck(app, &ix),
        None => recheck(app, &Index::default()),
    }
}

/// Automatic checks: from the cached directory straight away, then about 2 minutes after
/// launch and every hour after that, while the setting is on. The directory itself is only
/// fetched again once the cached copy is a day old, so that's once a day. Failures are quiet.
pub async fn run(app: AppHandle) {
    recheck_cached(&app);
    tokio::time::sleep(Duration::from_secs(120)).await;
    loop {
        if checks_on(&app) {
            match load(&app, false).await {
                Ok((ix, _, _)) => recheck(&app, &ix),
                Err(e) if cfg!(debug_assertions) => eprintln!("Rattleboard: plugin update check: {e}"),
                Err(_) => {}
            }
        }
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

/// Settings → Sources → Review: the updates for plugins installed from the directory, each
/// with what changes as far as the directory's entry says (`community_review` fetches the
/// manifest itself). Skipped versions aren't listed.
#[tauri::command]
pub async fn community_updates(app: AppHandle) -> Result<Value, String> {
    let (ix, fetched, warning) = load(&app, false).await?;
    recheck(&app, &ix);
    let list = updates_in(&app, &ix);
    Ok(json!({ "updates": list, "offered": community::offered(&list), "fetched": fetched, "warning": warning }))
}

/// One update, fetched and checked as an install would be, with its change summary against
/// the installed manifest (now including whether the script changed).
#[tauri::command]
pub async fn community_review(app: AppHandle, id: String) -> Result<Value, String> {
    let old = app.state::<AppState>().sources.manifest(&id).ok_or("That plugin isn't installed")?;
    let confirm = community_manifest(app, id).await?;
    let new = rattleboard_core::plugins::parse(&confirm["manifest"], false)?;
    let changes = community::changes(&community::Facts::of_manifest(&old), &community::Facts::of_manifest(&new));
    Ok(json!({ "confirm": confirm, "changes": changes }))
}

/// Skip this version: it isn't offered again until a newer one appears.
#[tauri::command]
pub fn community_skip(app: AppHandle, id: String, version: String) -> Result<(), String> {
    let path = skips_file(&app).ok_or("No data folder")?;
    let mut skips = community::read_skips(&path);
    let installed: Vec<String> = app.state::<AppState>().sources.installed().into_iter().map(|(p, _, _)| p).collect();
    skips.retain(|k, _| installed.contains(k));
    skips.insert(id, version);
    community::write_skips(&path, &skips)?;
    recheck_cached(&app);
    Ok(())
}

/// A plugin's README (as text) and preview card, fitted to this board's size.
#[tauri::command]
pub async fn community_detail(app: AppHandle, id: String) -> Result<Value, String> {
    let e = entry(&app, &id)?;
    let (b, c) = (base(), http());
    let readme = async {
        match &e.readme_url {
            Some(u) => community::get(c, u, b.as_deref(), community::MAX_README).await.map(|b| Some(String::from_utf8_lossy(&b).into_owned())),
            None => Ok(None),
        }
    };
    let preview = async {
        match &e.preview_url {
            Some(u) => community::get(c, u, b.as_deref(), community::MAX_PREVIEW).await.and_then(|b| community::parse_preview(&b)).map(Some),
            None => Ok(None),
        }
    };
    let (readme, preview) = tokio::join!(readme, preview);
    let (rows, cols) = sources_ctx(&app).dims;
    let mut out = Map::new();
    match readme {
        Ok(r) => out.insert("readme".into(), json!(r)),
        Err(e) => out.insert("readmeError".into(), json!(e)),
    };
    match preview {
        Ok(p) => out.insert("preview".into(), json!(p.map(|g| community::fit(&g, rows, cols)))),
        Err(e) => out.insert("previewError".into(), json!(e)),
    };
    out.insert("rows".into(), json!(rows));
    out.insert("cols".into(), json!(cols));
    Ok(Value::Object(out))
}

/// Fetches a plugin's manifest and checks it as a plugin link would be (command plugins are
/// refused), for the confirm step. Its id has to be the directory entry's.
#[tauri::command]
pub async fn community_manifest(app: AppHandle, id: String) -> Result<Value, String> {
    let e = entry(&app, &id)?;
    if let Some(v) = community::needs_newer_app(e.min_app_version.as_deref(), community::APP_VERSION) {
        return Err(format!("This plugin needs Rattleboard {v} or later"));
    }
    let body = community::get(http(), &e.manifest_url, base().as_deref(), rattleboard_core::plugins::MAX_MANIFEST).await?;
    let mut raw = link_json(&body)?;
    // A script plugin's code is the script file in the plugin's folder, from the same repository.
    if let Some(file) = rattleboard_core::script::file_to_fetch(&raw) {
        let url = rattleboard_core::script::script_url(&e.manifest_url, &file)?;
        if !community::is_allowed(&url) {
            return Err("That plugin's script isn't in the community repository".into());
        }
        let code = community::get(http(), &url, base().as_deref(), rattleboard_core::script::MAX_SCRIPT).await?;
        rattleboard_core::script::inline(&mut raw, &code)?;
    }
    let mut v = link_manifest(raw)?;
    if v["id"] != e.id.as_str() {
        return Err("That plugin's manifest doesn't match its directory entry".into());
    }
    v["version"] = json!(e.version);
    Ok(v)
}

/// Installed from a link or a file instead: no longer the directory's plugin, so it's never
/// offered the directory's updates.
pub fn forget(app: &AppHandle, id: &str) {
    let mut rec = recorded(app);
    if rec.remove(id).is_some() {
        if let Some(path) = data_file(app, "community-installed.json") {
            let _ = std::fs::write(&path, serde_json::to_string_pretty(&rec).unwrap_or_default());
        }
        recheck_cached(app);
    }
}

/// After import_plugin installed it: remembers the directory's version, for Update available.
#[tauri::command]
pub fn community_installed(app: AppHandle, id: String) -> Result<(), String> {
    let e = entry(&app, &id)?;
    let st = app.state::<AppState>();
    if !st.sources.installed().iter().any(|(p, _, _)| *p == id) {
        return Err("That plugin isn't installed".into());
    }
    let path = data_file(&app, "community-installed.json").ok_or("No data folder")?;
    let mut rec = recorded(&app);
    rec.retain(|k, _| st.sources.installed().iter().any(|(p, _, _)| p == k));
    rec.insert(id, e.version);
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    std::fs::write(&path, serde_json::to_string_pretty(&rec).unwrap_or_default()).map_err(|e| e.to_string())?;
    recheck_cached(&app);
    Ok(())
}

/// Opens the repository, its contributor guide, a plugin's folder or README, or an author's GitHub
/// profile in the browser. Nothing else: folders come from the checked index, and a profile
/// only from a valid GitHub user name.
#[tauri::command]
pub fn open_community(app: AppHandle, target: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let url = match target.split_once(':') {
        None if target == "repo" => community::REPO_URL.to_string(),
        None if target == "contribute" => community::CONTRIBUTE_URL.to_string(),
        Some(("author", login)) => community::profile_url(login).ok_or("Not a GitHub user name")?,
        Some(("folder", id)) => entry(&app, id)?.folder_url.ok_or("That plugin has no folder link")?,
        Some(("readme", id)) => {
            let e = entry(&app, id)?;
            match (&e.readme_url, &e.folder_url) {
                (Some(_), Some(f)) => community::readme_page(f).ok_or("That plugin has no README link")?,
                _ => e.folder_url.ok_or("That plugin has no README link")?,
            }
        }
        _ => return Err("Nothing to open".into()),
    };
    if !(community::is_allowed(&url) || url == community::REPO_URL || community::profile_url(url.trim_start_matches("https://github.com/")).as_deref() == Some(url.as_str())) {
        return Err("That address isn't allowed".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}
