//! Updates: checks GitHub for a newer release (see `rattleboard_core::update`) and, when there
//! is one, puts "Update available" at the top of the tray menu. That item and "Check for
//! updates…" open the update window (panel.js, view "update"): what's new in each release since
//! this one, and a way to update. Nothing is downloaded or installed until the user says so.
//!
//! Installing goes through Tauri's updater: it reads `latest.json` from the latest release
//! (made by the release workflow), downloads this platform's bundle, checks its signature
//! against the public key in tauri.conf.json, replaces this copy and restarts. Copies that
//! something else manages update their own way (`how`): the Homebrew cask with `brew upgrade`,
//! and a .deb (or a debug build) from the release page.

use crate::{broadcast, AppState};
use rattleboard_core::{now_ms, update};
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// The newer release the tray offers, if automatic checks are on and one was found.
pub fn available(app: &AppHandle) -> Option<String> {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    (s.update_check && update::is_newer(&s.update_latest, CURRENT)).then_some(s.update_latest)
}

/// Debug builds only: RATTLEBOARD_UPDATE_FAKE_LATEST=9.9.9 pretends GitHub answered that,
/// for trying the prompt out without a real release.
fn fake_latest() -> Option<update::Release> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let v = std::env::var("RATTLEBOARD_UPDATE_FAKE_LATEST").ok()?;
    let v = update::Version::parse(&v)?;
    Some(update::Release { version: v.to_string(), url: update::RELEASES.into() })
}

/// Asks GitHub, remembers the answer in settings and refreshes the tray.
async fn check(app: &AppHandle) -> Result<update::Release, String> {
    let r = match fake_latest() {
        Some(r) => r,
        None => update::latest(&update::client()).await?,
    };
    if cfg!(debug_assertions) {
        eprintln!("Rattleboard: update check: latest {} ({}), this is {CURRENT}", r.version, r.url);
    }
    {
        let st = app.state::<AppState>();
        let mut s = st.settings.lock().unwrap();
        s.update_last_check = now_ms();
        s.update_latest = r.version.clone();
        s.update_url = r.url.clone();
        s.save(&st.settings_path);
    }
    sync_tray(app);
    broadcast(app);
    Ok(r)
}

/// Automatic checks: about 30 s after launch, then once a day while the setting is on.
/// A check less than 12 hours old (from before a restart) counts, so restarts don't ask again.
pub async fn run(app: AppHandle) {
    // A newer version found before a restart shows straight away.
    sync_tray(&app);
    tokio::time::sleep(Duration::from_secs(30)).await;
    let mut at_launch = true;
    loop {
        let (on, last) = {
            let s = app.state::<AppState>().settings.lock().unwrap().clone();
            (s.update_check, s.update_last_check)
        };
        if on && update::should_check(last, now_ms(), at_launch) {
            // Automatic checks fail quietly; the next tick tries again.
            let _ = check(&app).await;
        }
        at_launch = false;
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checked {
    current: &'static str,
    latest: String,
    newer: bool,
}

/// Settings → "Check for updates": checks now, whatever the setting.
#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> Result<Checked, String> {
    let r = check(&app).await?;
    Ok(Checked { current: CURRENT, newer: update::is_newer(&r.version, CURRENT), latest: r.version })
}

/// How this copy updates: "install" (in place, by the updater), "homebrew" (`brew upgrade`)
/// or "download" (from the release page: a .deb, which apt manages, or a debug build).
pub fn how() -> &'static str {
    // Debug builds update in place only to try it (RATTLEBOARD_UPDATE_TEST=1, with a test
    // key and endpoint passed to `tauri build --debug --config`).
    if cfg!(debug_assertions) && std::env::var("RATTLEBOARD_UPDATE_TEST").as_deref() != Ok("1") {
        "download"
    } else if from_homebrew() {
        "homebrew"
    } else if cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_none() {
        "download"
    } else {
        "install"
    }
}

/// The update window: the notes of every release since this one, newest first. Debug builds
/// with RATTLEBOARD_UPDATE_FAKE_LATEST show the real notes since 0.10.0.
#[tauri::command]
pub async fn update_notes() -> Result<Vec<update::Notes>, String> {
    let since = if fake_latest().is_some() { "0.10.0" } else { CURRENT };
    update::since(&update::client(), since).await
}

/// "Update and restart": downloads the latest release for this platform, checks its signature,
/// installs it over this copy and restarts. Progress goes to the update window as
/// rb://update-progress { got, total } (bytes; total may be null).
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    if how() != "install" {
        return Err("This copy of Rattleboard can't update itself; download the new version instead.".into());
    }
    let found = app
        .updater()
        .map_err(|e| format!("Updates aren't set up in this build ({e})"))?
        .check()
        .await
        .map_err(|e| format!("Couldn't check for the update: {e}"))?;
    let Some(up) = found else { return Err("You're already up to date.".into()) };
    let (a, mut got) = (app.clone(), 0usize);
    up.download_and_install(
        move |n, total| {
            got += n;
            let _ = a.emit_to("panel", "rb://update-progress", serde_json::json!({ "got": got, "total": total }));
        },
        || {},
    )
    .await
    .map_err(|e| format!("Couldn't install the update: {e}"))?;
    let _ = app.emit_to("panel", "rb://update-progress", serde_json::json!({ "installed": true }));
    tokio::time::sleep(Duration::from_millis(800)).await;
    crate::before_exit(&app);
    app.restart();
}

/// Opens a release's page (`url`, from the update window), or the one found by the last check.
/// Only ever a github.com/slastrina/rattleboard https page, so the pages need no opener
/// permission of their own.
#[tauri::command]
pub fn open_update(app: AppHandle, url: Option<String>) {
    use tauri_plugin_opener::OpenerExt;
    let url = url.unwrap_or_else(|| app.state::<AppState>().settings.lock().unwrap().update_url.clone());
    let url = if update::is_release_url(&url) { url } else { update::RELEASES.to_string() };
    let _ = app.opener().open_url(url, None::<&str>);
}

/// Whether this copy came from the Homebrew cask (macOS): the cask leaves its record in the
/// Caskroom. Only then does Settings suggest `brew upgrade`.
/// Looked up once.
pub fn from_homebrew() -> bool {
    static BREW: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *BREW.get_or_init(|| {
        cfg!(target_os = "macos")
            && ["/opt/homebrew/Caskroom/rattleboard", "/usr/local/Caskroom/rattleboard"]
                .iter()
                .any(|p| std::path::Path::new(p).is_dir())
    })
}

/// Puts the "Update available" item at the top of the native tray menu (Windows, Linux), and
/// "Plugin updates (N)…" under it (see community.rs), or takes them away, and notes them in
/// the tray icon's tooltip. No lock is held while the menu changes (see `available`). Runs on
/// the main thread, so two calls never interleave.
pub fn sync_tray(app: &AppHandle) {
    let h = app.clone();
    let _ = app.run_on_main_thread(move || sync_tray_now(&h));
}

/// The ids of the items at the top of the tray menu, in order; a separator follows them.
#[cfg(not(target_os = "macos"))]
const TOP: [&str; 2] = ["update", "plugin_updates"];

fn sync_tray_now(app: &AppHandle) {
    let label = available(app).map(|v| format!("Update available: v{v}…"));
    let plugins = crate::community::offered();
    let plugins = (plugins > 0 && app.state::<AppState>().settings.lock().unwrap().plugin_update_check).then_some(plugins);
    if let Some(t) = app.tray_by_id("main") {
        let tip = match (&label, plugins) {
            (Some(_), _) => "Rattleboard · update available".to_string(),
            (None, Some(1)) => "Rattleboard · a plugin update".to_string(),
            (None, Some(n)) => format!("Rattleboard · {n} plugin updates"),
            (None, None) => "Rattleboard".to_string(),
        };
        let _ = t.set_tooltip(Some(tip));
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(m) = app.try_state::<crate::TrayMenu>() {
        use tauri::menu::{MenuItem, PredefinedMenuItem};
        let m = &m.0;
        // Take away the top items there are, and the separator after them, then put back the
        // ones wanted now.
        let have = m.items().map(|v| v.iter().take_while(|i| TOP.iter().any(|t| i.id() == *t)).count()).unwrap_or(0);
        if have > 0 {
            for _ in 0..=have {
                let _ = m.remove_at(0);
            }
        }
        let want: Vec<(&str, String)> = [label.map(|l| (TOP[0], l)), plugins.map(|n| (TOP[1], format!("Plugin updates ({n})…")))].into_iter().flatten().collect();
        if !want.is_empty() {
            let mut at = 0;
            for (id, text) in want {
                if let Ok(item) = MenuItem::with_id(app, id, text, true, None::<&str>) {
                    let _ = m.insert(&item, at);
                    at += 1;
                }
            }
            if let Ok(sep) = PredefinedMenuItem::separator(app) {
                let _ = m.insert(&sep, at);
            }
        }
    }
}
