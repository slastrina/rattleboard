//! Suggests updates: checks GitHub for a newer release (see `rattleboard_core::update`) and,
//! when there is one, puts "Update available" at the top of the tray menu. Clicking it opens
//! the release page; nothing is ever downloaded or installed here.

use crate::{broadcast, AppState};
use rattleboard_core::{now_ms, update};
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Manager};

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

/// Opens the release page found by the last check. Only ever a github.com/slastrina/rattleboard
/// https page, so the pages need no opener permission of their own.
#[tauri::command]
pub fn open_update(app: AppHandle) {
    use tauri_plugin_opener::OpenerExt;
    let url = app.state::<AppState>().settings.lock().unwrap().update_url.clone();
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

/// Puts the "Update available" item at the top of the native tray menu (Windows, Linux), or
/// takes it away, and notes it in the tray icon's tooltip. No lock is held while the menu
/// changes (see `available`). Runs on the main thread, so two calls never interleave.
pub fn sync_tray(app: &AppHandle) {
    let h = app.clone();
    let _ = app.run_on_main_thread(move || sync_tray_now(&h));
}

fn sync_tray_now(app: &AppHandle) {
    let label = available(app).map(|v| format!("Update available: v{v}…"));
    if let Some(t) = app.tray_by_id("main") {
        let tip = match &label {
            Some(_) => "Rattleboard · update available".to_string(),
            None => "Rattleboard".to_string(),
        };
        let _ = t.set_tooltip(Some(tip));
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(m) = app.try_state::<crate::TrayMenu>() {
        use tauri::menu::{MenuItem, PredefinedMenuItem};
        let m = &m.0;
        match (m.get("update"), label) {
            (Some(item), Some(l)) => {
                if let Some(i) = item.as_menuitem() {
                    let _ = i.set_text(l);
                }
            }
            (None, Some(l)) => {
                if let (Ok(item), Ok(sep)) = (MenuItem::with_id(app, "update", l, true, None::<&str>), PredefinedMenuItem::separator(app)) {
                    let _ = m.insert_items(&[&item, &sep], 0);
                }
            }
            (Some(_), None) => {
                // The item and the separator after it.
                let _ = m.remove_at(1);
                let _ = m.remove_at(0);
            }
            (None, None) => {}
        }
    }
}
