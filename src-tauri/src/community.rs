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
use std::sync::OnceLock;
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

/// The directory, each entry with whether it's installed and whether there's an update.
/// Cached for a day; `refresh` fetches it again now. Offline with a cached copy, that copy is
/// shown with a warning; with none, the error is the offline message.
#[tauri::command]
pub async fn community_index(app: AppHandle, refresh: Option<bool>) -> Result<Value, String> {
    let now = now_ms();
    let path = data_file(&app, "community-index.json").ok_or("No data folder")?;
    let have = cached(&app);
    let mut warning = None;
    let (ix, fetched) = match have {
        Some((ix, f)) if !refresh.unwrap_or(false) && community::is_fresh(f, now) => (ix, f),
        have => match community::get(http(), community::INDEX_URL, base().as_deref(), community::MAX_INDEX).await.and_then(|b| {
            let ix = community::parse_index(&b)?;
            Ok((ix, b))
        }) {
            Ok((ix, body)) => {
                let body = String::from_utf8_lossy(&body).into_owned();
                community::write_cache(&path, &Cache { fetched: now, source: source(), body });
                (ix, now)
            }
            Err(e) => match have {
                Some((ix, f)) => {
                    warning = Some(format!("Couldn't refresh the list ({e}); showing the copy from earlier."));
                    (ix, f)
                }
                None => return Err(format!("{} ({e})", community::OFFLINE)),
            },
        },
    };
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
    std::fs::write(&path, serde_json::to_string_pretty(&rec).unwrap_or_default()).map_err(|e| e.to_string())
}

/// Opens the repository, its contributor guide, a plugin's folder or an author's GitHub
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
        _ => return Err("Nothing to open".into()),
    };
    if !(community::is_allowed(&url) || url == community::REPO_URL || community::profile_url(url.trim_start_matches("https://github.com/")).as_deref() == Some(url.as_str())) {
        return Err("That address isn't allowed".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}
