//! Notification forwarding: puts the system's notifications from chosen apps on a board.
//!
//! Windows only, through `UserNotificationListener` (the user grants access once, under
//! Settings → Privacy → Notifications). macOS has no supported API for reading other apps'
//! notifications, so there the feature doesn't exist and its settings stay hidden.
//!
//! The listener is polled rather than subscribed to: its NotificationChanged event needs a
//! packaged app, and Rattleboard is a plain installer.

#[cfg(windows)]
use crate::{settings, vestaboard::Transition};
use tauri::AppHandle;

pub const SUPPORTED: bool = cfg!(windows);

/// What goes on the board: the app's name, then the title and text unless hidden.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn message_text(app: &str, title: &str, body: &str, hide: bool) -> String {
    let app = app.trim();
    if hide {
        return format!("{app}\nNew notification");
    }
    [app, title.trim(), body.trim()].iter().filter(|t| !t.is_empty()).cloned().collect::<Vec<_>>().join("\n")
}

#[cfg(not(windows))]
pub fn start(_app: &AppHandle) {}

#[cfg(not(windows))]
pub fn request_access(_app: &AppHandle) -> Result<(), String> {
    Err("Notification forwarding needs Windows".into())
}

#[cfg(windows)]
pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || win::run(app));
}

/// Shows Windows' consent prompt. Called on the main (UI) thread, which the prompt needs;
/// the answer is awaited elsewhere so the UI never blocks.
#[cfg(windows)]
pub fn request_access(app: &AppHandle) -> Result<(), String> {
    use windows::UI::Notifications::Management::UserNotificationListener;
    let op = UserNotificationListener::Current()
        .and_then(|l| l.RequestAccessAsync())
        .map_err(|e| e.message())?;
    let app = app.clone();
    std::thread::spawn(move || {
        if let Ok(s) = op.join() {
            win::set_access(&app, win::access_name(s));
        }
    });
    Ok(())
}

#[cfg(windows)]
mod win {
    use super::*;
    use crate::{AppState, NotifyApp};
    use std::collections::HashSet;
    use std::time::Duration;
    use tauri::Manager;
    use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
    use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};

    const EVERY: Duration = Duration::from_secs(2);

    pub fn access_name(s: UserNotificationListenerAccessStatus) -> &'static str {
        match s {
            UserNotificationListenerAccessStatus::Allowed => "allowed",
            UserNotificationListenerAccessStatus::Denied => "denied",
            _ => "unspecified",
        }
    }

    pub fn set_access(app: &AppHandle, access: &str) {
        let st = app.state::<AppState>();
        let changed = {
            let mut n = st.notify.lock().unwrap();
            let changed = n.access != access;
            n.access = access.to_string();
            changed
        };
        if changed {
            crate::broadcast(app);
        }
    }

    fn note_app(app: &AppHandle, a: &NotifyApp) {
        let st = app.state::<AppState>();
        let added = {
            let mut n = st.notify.lock().unwrap();
            let new = !n.apps.iter().any(|x| x.id == a.id);
            if new {
                n.apps.push(a.clone());
            }
            new
        };
        if added {
            crate::broadcast(app);
        }
    }

    struct Toast {
        id: u32,
        from: NotifyApp,
        title: String,
        body: String,
    }

    fn read(n: &UserNotification) -> windows::core::Result<Toast> {
        let info = n.AppInfo()?;
        let from = NotifyApp {
            id: info.AppUserModelId()?.to_string(),
            name: info.DisplayInfo().and_then(|d| d.DisplayName()).map(|s| s.to_string()).unwrap_or_default(),
        };
        let texts: Vec<String> = n
            .Notification()?
            .Visual()?
            .GetBinding(&KnownNotificationBindings::ToastGeneric()?)?
            .GetTextElements()?
            .into_iter()
            .filter_map(|t| t.Text().ok().map(|s| s.to_string()))
            .collect();
        let title = texts.first().cloned().unwrap_or_default();
        let body = texts.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
        Ok(Toast { id: n.Id()?, from, title, body })
    }

    fn local_minutes() -> u32 {
        let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        t.wHour as u32 * 60 + t.wMinute as u32
    }

    pub fn run(app: AppHandle) {
        // Notifications already in the Action Center at launch (or when access is granted)
        // are old news; only ones that arrive afterwards are forwarded.
        let mut seen: HashSet<u32> = HashSet::new();
        let mut primed = false;
        loop {
            std::thread::sleep(EVERY);
            let Ok(listener) = UserNotificationListener::Current() else {
                set_access(&app, "unavailable");
                continue;
            };
            let access = listener.GetAccessStatus().map(access_name).unwrap_or("unavailable");
            set_access(&app, access);
            if access != "allowed" {
                primed = false;
                continue;
            }
            let Ok(list) = listener.GetNotificationsAsync(NotificationKinds::Toast).and_then(|op| op.join()) else {
                continue;
            };
            let toasts: Vec<Toast> = list.into_iter().filter_map(|n| read(&n).ok()).collect();
            let s = app.state::<AppState>().settings.lock().unwrap().clone();
            for t in &toasts {
                note_app(&app, &t.from);
                let fresh = seen.insert(t.id) && primed;
                let chosen = s.notify_apps.iter().any(|a| a.id == t.from.id);
                let quiet = s.notify_quiet && settings::in_quiet_hours(&s.notify_quiet_from, &s.notify_quiet_to, local_minutes());
                if fresh && s.notify_enabled && chosen && !quiet {
                    let name = if t.from.name.is_empty() { s.notify_apps.iter().find(|a| a.id == t.from.id).map(|a| a.name.clone()).unwrap_or_default() } else { t.from.name.clone() };
                    let text = message_text(&name, &t.title, &t.body, s.notify_hide_content);
                    // A group or "all" gets it on every board, each going back on its own.
                    for id in crate::destinations(&app, &s.notify_target).unwrap_or_default() {
                        tauri::async_runtime::spawn(forward(app.clone(), id, text.clone(), name.clone(), s.notify_return_sec));
                    }
                }
            }
            // Dismissed notifications don't come back, so only the current ones need remembering.
            let now: HashSet<u32> = toasts.iter().map(|t| t.id).collect();
            seen.retain(|id| now.contains(id));
            primed = true;
        }
    }

    /// Shows the notification, then puts the previous message back after `back_after`
    /// seconds, but only if the board still shows the notification. On this board that's a
    /// notification of the board's own (`notify_here`); a saved board gets two messages.
    async fn forward(app: AppHandle, target: String, text: String, from: String, back_after: u32) {
        let Ok((rows, cols)) = crate::target_dims(&app, &target) else { return };
        let layout = crate::vestaboard::text_layout(&text, rows, cols);
        if target == crate::HERE {
            // This board: a notification, shown once. It goes back by itself (to the previous
            // message or the Sources rotation) without the old message counting as new.
            let seconds = (back_after > 0).then_some(back_after.max(3) as u64);
            let source = if from.trim().is_empty() { "Notification".to_string() } else { format!("{} notification", from.trim()) };
            crate::notify_here(&app, crate::server::Notice { layout, tr: Transition::default(), seconds, source, link: None, alert: false });
            return;
        }
        let before = if back_after > 0 { crate::read_target(&app, &target).await.ok() } else { None };
        if crate::send_to_target(&app, &target, layout.clone(), Transition::default(), "Notification").await.is_err() {
            return;
        }
        let Some(before) = before.filter(|b| *b != layout) else { return };
        tokio::time::sleep(Duration::from_secs(back_after as u64)).await;
        if crate::read_target(&app, &target).await.ok().as_ref() == Some(&layout) {
            let _ = crate::send_to_target(&app, &target, before, Transition::default(), "Notification").await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text() {
        assert_eq!(message_text("Mail", "Anna", "Lunch?", false), "Mail\nAnna\nLunch?");
        assert_eq!(message_text("Mail", "Anna", "Lunch?", true), "Mail\nNew notification");
        assert_eq!(message_text("Teams", "", " ", false), "Teams");
    }
}
