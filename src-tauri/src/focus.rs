//! Focus / Do Not Disturb awareness: while the user has a Focus on, Rattleboard can hold its
//! pop-ups (the board still updates, but doesn't pop up or make a sound) and show one summary
//! when the Focus ends, or only mute the flaps.
//!
//! Detection, polled every few seconds by `watch` (none of these platforms offers a change
//! notification a plain desktop app can rely on):
//! - macOS 12+: Intents' `INFocusStatusCenter` (authorization asked once, then
//!   `focusStatus.isFocused`). A Focus is only reported while that Focus has "Share Focus
//!   Status" on (System Settings → Focus → Focus Status). Needs `NSFocusStatusUsageDescription`
//!   in Info.plist, and the app signed with the Communication Notifications entitlement
//!   (`com.apple.developer.usernotifications.communication`, which needs a provisioning
//!   profile; Developer ID profiles can have it). Without the entitlement macOS answers Focus
//!   queries with "App is missing Communication Notifications entitlement" (DNDErrorDomain
//!   1004), so a build without it doesn't ask at all and Settings says why. The class is
//!   looked up at run time, so macOS 11 just reports "needs 12".
//! - Windows: `SHQueryUserNotificationState` (busy, presentation mode, full-screen Direct3D,
//!   quiet time) and, on Windows 11 builds that have it, `FocusSessionManager.IsFocusActive`.
//!   Both are documented; nothing reads undocumented (WNF) state.
//! - Linux: GNOME's `org.gnome.desktop.notifications show-banners` (false is Do Not Disturb)
//!   through `gsettings`, and KDE Plasma's `Inhibited` property on org.freedesktop.Notifications.
//!   Elsewhere, "not detected".
//!
//! Debug builds only: RATTLEBOARD_FAKE_FOCUS=on|off, or the path of a file holding on or off
//! (read again every second), stands in for the system's state, to try the hold-and-summary
//! flow without touching the user's Focus.

use crate::vestaboard::Grid;
use serde::Serialize;
use std::time::Duration;

/// How often the system's state is read.
const EVERY: Duration = Duration::from_secs(10);
const FAKE_EVERY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    /// The board updates, but doesn't pop up or make a sound; a summary pops up afterwards
    Hold,
    /// Pops up as usual, silently
    Mute,
    Ignore,
}

impl Mode {
    pub fn of(s: &str) -> Mode {
        match s {
            "mute" => Mode::Mute,
            "ignore" => Mode::Ignore,
            _ => Mode::Hold,
        }
    }
}

/// The user's choices that the decision depends on.
#[derive(Clone, Copy, Debug)]
pub struct Rules {
    pub mode: Mode,
    /// "Let alerts through"
    pub alerts_through: bool,
    /// "Full screen too": a full-screen board follows Focus as well
    pub fullscreen_obeys: bool,
}

/// What a message does while things stand as they are.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Decision {
    /// The board pops up for it
    pub pop_up: bool,
    /// Its flaps make their sound (if the flap sound is on at all)
    pub sound: bool,
    /// It counts towards the summary shown when the Focus ends
    pub held: bool,
}

const AS_USUAL: Decision = Decision { pop_up: true, sound: true, held: false };

/// The rule: nothing changes without a Focus, with "Ignore Focus", on a full-screen board
/// (a kiosk) unless it follows Focus too, or for an alert the user lets through. Otherwise
/// "Mute sound only" pops up silently and "Hold pop-ups" doesn't pop up at all.
pub fn decide(r: Rules, focused: bool, alert: bool, fullscreen: bool) -> Decision {
    if !focused || r.mode == Mode::Ignore || (fullscreen && !r.fullscreen_obeys) || (alert && r.alerts_through) {
        return AS_USUAL;
    }
    match r.mode {
        Mode::Mute => Decision { pop_up: true, sound: false, held: false },
        _ => Decision { pop_up: false, sound: false, held: true },
    }
}

/// The messages held during a Focus: how many, and the latest.
#[derive(Default, Debug)]
pub struct Held {
    pub count: usize,
    pub latest: Option<HeldMessage>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeldMessage {
    pub layout: Grid,
    pub link: Option<String>,
}

impl Held {
    pub fn note(&mut self, layout: Grid, link: Option<String>) {
        self.count += 1;
        self.latest = Some(HeldMessage { layout, link });
    }

    /// The summary's count and message, once; nothing if nothing was held.
    pub fn take(&mut self) -> Option<(usize, HeldMessage)> {
        let held = std::mem::take(self);
        Some((held.count, held.latest?)).filter(|(n, _)| *n > 0)
    }
}

pub fn summary_text(n: usize) -> String {
    if n == 1 {
        "1 message while you were focused".into()
    } else {
        format!("{n} messages while you were focused")
    }
}

/// What the system says, for the decision and for Settings.
#[derive(Serialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// A Focus / Do Not Disturb is on
    pub on: bool,
    /// Where it was read: "macos" | "windows" | "gnome" | "kde" | "fake" | "" (nowhere)
    pub via: String,
    /// "ok" | "notDetermined" | "denied" | "restricted" | "needsMacos12" | "noEntitlement" |
    /// "unavailable" | "notDetected"
    pub access: String,
    /// What's on, in a few words ("Focus", "Do Not Disturb", "Presentation mode"), when on
    pub what: String,
}

impl Status {
    fn new(via: &str, access: &str, on: bool, what: &str) -> Status {
        Status { on, via: via.into(), access: access.into(), what: if on { what.into() } else { String::new() } }
    }
}

/// Debug builds: the stand-in state from RATTLEBOARD_FAKE_FOCUS, if it's set.
fn fake() -> Option<Status> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let v = std::env::var("RATTLEBOARD_FAKE_FOCUS").ok().filter(|v| !v.trim().is_empty())?;
    let word = match v.trim() {
        w @ ("on" | "off") => w.to_string(),
        path => std::fs::read_to_string(path).unwrap_or_default().trim().to_lowercase(),
    };
    Some(Status::new("fake", "ok", word == "on", "Focus (simulated)"))
}

/// RATTLEBOARD_FAKE_FOCUS is set (debug builds): the system isn't asked anything.
pub fn fake_set() -> bool {
    cfg!(debug_assertions) && std::env::var("RATTLEBOARD_FAKE_FOCUS").is_ok_and(|v| !v.trim().is_empty())
}

/// Reads the state now and then, calling `changed` with the first reading and with every
/// change after it. Runs on a thread of its own.
pub fn watch(changed: impl Fn(Status) + Send + 'static) {
    std::thread::spawn(move || {
        let mut det = Detector::default();
        let mut last: Option<Status> = None;
        loop {
            let now = fake().unwrap_or_else(|| det.read());
            if last.as_ref() != Some(&now) {
                last = Some(now.clone());
                changed(now);
            }
            std::thread::sleep(if fake_set() { FAKE_EVERY } else { EVERY });
        }
    });
}

/// Asks the system for access to the Focus state, where it asks (macOS), if it hasn't been
/// answered yet. `again`: the user asked (Settings); otherwise at most once per run.
pub fn request_access(_again: bool) {
    #[cfg(target_os = "macos")]
    mac::request(_again);
}

/// Whether a full-screen app is Rattleboard's own board (Windows reports any full-screen
/// window as "busy", which Rattleboard's own full screen mustn't count as).
static OWN_FULLSCREEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_own_fullscreen(on: bool) {
    OWN_FULLSCREEN.store(on, std::sync::atomic::Ordering::Relaxed);
}

// ---------------------------------------------------------------- per platform

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
#[derive(Default)]
struct Detector;

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
impl Detector {
    fn read(&mut self) -> Status {
        Status::new("", "notDetected", false, "")
    }
}

// macOS ----------------------------------------------------------------

#[cfg(target_os = "macos")]
#[derive(Default)]
struct Detector;

#[cfg(target_os = "macos")]
impl Detector {
    fn read(&mut self) -> Status {
        mac::read()
    }
}

/// INFocusStatusAuthorizationStatus as a word.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn mac_access_name(code: isize) -> &'static str {
    match code {
        0 => "notDetermined",
        1 => "restricted",
        2 => "denied",
        3 => "ok",
        _ => "unavailable",
    }
}

#[cfg(target_os = "macos")]
pub mod mac {
    use super::{mac_access_name, Status};
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};

    static ASKED: AtomicBool = AtomicBool::new(false);

    pub const ENTITLEMENT: &std::ffi::CStr = c"com.apple.developer.usernotifications.communication";

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        fn SecTaskCreateFromSelf(alloc: *const c_void) -> *const c_void;
        fn SecTaskCopyValueForEntitlement(task: *const c_void, entitlement: *const c_void, error: *mut *const c_void) -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(alloc: *const c_void, s: *const std::ffi::c_char, encoding: u32) -> *const c_void;
        fn CFRelease(cf: *const c_void);
        static kCFBooleanTrue: *const c_void;
    }

    /// This process is signed with the Communication Notifications entitlement, without which
    /// macOS doesn't share the Focus state. Read once.
    pub fn entitled() -> bool {
        static ENTITLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENTITLED.get_or_init(|| unsafe {
            let task = SecTaskCreateFromSelf(std::ptr::null());
            if task.is_null() {
                return false;
            }
            let key = CFStringCreateWithCString(std::ptr::null(), ENTITLEMENT.as_ptr(), 0x0800_0100);
            let v = if key.is_null() { std::ptr::null() } else { SecTaskCopyValueForEntitlement(task, key, std::ptr::null_mut()) };
            let yes = !v.is_null() && v == kCFBooleanTrue;
            for cf in [v, key, task] {
                if !cf.is_null() {
                    CFRelease(cf);
                }
            }
            yes
        })
    }

    /// The shared INFocusStatusCenter, or None before macOS 12. Intents is loaded here rather
    /// than linked, as Rattleboard still runs on macOS 11.
    fn center() -> Option<*mut AnyObject> {
        unsafe {
            let path: *mut AnyObject = msg_send![objc2::class!(NSString), stringWithUTF8String: c"/System/Library/Frameworks/Intents.framework".as_ptr()];
            let bundle: *mut AnyObject = msg_send![objc2::class!(NSBundle), bundleWithPath: path];
            if !bundle.is_null() {
                let _: bool = msg_send![bundle, load];
            }
            let cls = AnyClass::get(c"INFocusStatusCenter")?;
            let c: *mut AnyObject = msg_send![cls, defaultCenter];
            (!c.is_null()).then_some(c)
        }
    }

    /// The raw authorization status (INFocusStatusAuthorizationStatus), without asking.
    pub fn authorization() -> Option<isize> {
        objc2::rc::autoreleasepool(|_| center().map(|c| unsafe { msg_send![c, authorizationStatus] }))
    }

    pub fn read() -> Status {
        objc2::rc::autoreleasepool(|_| {
            let Some(c) = center() else { return Status::new("macos", "needsMacos12", false, "") };
            if !entitled() {
                return Status::new("macos", "noEntitlement", false, "");
            }
            let code: isize = unsafe { msg_send![c, authorizationStatus] };
            let access = mac_access_name(code);
            if access != "ok" {
                return Status::new("macos", access, false, "");
            }
            // isFocused is nil when the state can't be told.
            let on: Option<bool> = unsafe {
                let st: *mut AnyObject = msg_send![c, focusStatus];
                let n: *mut AnyObject = if st.is_null() { std::ptr::null_mut() } else { msg_send![st, isFocused] };
                (!n.is_null()).then(|| msg_send![n, boolValue])
            };
            match on {
                Some(on) => Status::new("macos", "ok", on, "Focus"),
                None => Status::new("macos", "unavailable", false, ""),
            }
        })
    }

    /// Shows macOS's prompt if it hasn't been answered yet (macOS asks only once anyway).
    pub fn request(again: bool) {
        if ASKED.swap(true, Ordering::SeqCst) && !again {
            return;
        }
        objc2::rc::autoreleasepool(|_| {
            let Some(c) = center() else { return };
            if !entitled() {
                eprintln!("[focus] not signed with {ENTITLEMENT:?}, so macOS won't share Focus status; not asking");
                return;
            }
            let code: isize = unsafe { msg_send![c, authorizationStatus] };
            if code != 0 {
                return;
            }
            let done = block2::RcBlock::new(|status: isize| {
                eprintln!("[focus] macOS answered the Focus status request: {}", mac_access_name(status));
            });
            let _: () = unsafe { msg_send![c, requestAuthorizationWithCompletionHandler: &*done] };
        });
    }
}

// Windows --------------------------------------------------------------

/// SHQueryUserNotificationState's answer, as what's on: None when notifications may show
/// (or the user is away, which is no Focus). `own_fullscreen`: Rattleboard's own full-screen
/// board is the full-screen app, which doesn't count.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn quns_what(state: i32, own_fullscreen: bool) -> Option<&'static str> {
    match state {
        // QUNS_BUSY: a full-screen app, or presentation settings
        2 if !own_fullscreen => Some("A full-screen app"),
        // QUNS_RUNNING_D3D_FULL_SCREEN
        3 if !own_fullscreen => Some("A full-screen game"),
        // QUNS_PRESENTATION_MODE
        4 => Some("Presentation mode"),
        // QUNS_QUIET_TIME
        6 => Some("Quiet time"),
        // QUNS_NOT_PRESENT (locked, screen saver), QUNS_ACCEPTS_NOTIFICATIONS, QUNS_APP
        _ => None,
    }
}

#[cfg(windows)]
#[derive(Default)]
struct Detector;

#[cfg(windows)]
impl Detector {
    fn read(&mut self) -> Status {
        use windows::UI::Shell::FocusSessionManager;
        use windows::Win32::UI::Shell::SHQueryUserNotificationState;
        let own = OWN_FULLSCREEN.load(std::sync::atomic::Ordering::Relaxed);
        // Windows 11's Focus sessions (Clock app, or Focus in the notification centre), where
        // the build has the API; older Windows fails to activate the class.
        let session = FocusSessionManager::IsSupported()
            .ok()
            .filter(|s| *s)
            .and_then(|_| FocusSessionManager::GetDefault().ok())
            .and_then(|m| m.IsFocusActive().ok())
            .unwrap_or(false);
        if session {
            return Status::new("windows", "ok", true, "Focus");
        }
        match unsafe { SHQueryUserNotificationState() } {
            Ok(q) => match quns_what(q.0, own) {
                Some(what) => Status::new("windows", "ok", true, what),
                None => Status::new("windows", "ok", false, ""),
            },
            Err(_) => Status::new("windows", "unavailable", false, ""),
        }
    }
}

// Linux ----------------------------------------------------------------

/// The desktop, from XDG_CURRENT_DESKTOP ("ubuntu:GNOME", "KDE", "X-Cinnamon").
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Desktop {
    Gnome,
    Kde,
    Other,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn desktop_of(xdg_current_desktop: &str) -> Desktop {
    let parts: Vec<String> = xdg_current_desktop.split(':').map(|p| p.trim().to_ascii_lowercase()).collect();
    if parts.iter().any(|p| p == "kde") {
        Desktop::Kde
    } else if parts.iter().any(|p| matches!(p.as_str(), "gnome" | "gnome-classic" | "gnome-flashback" | "unity" | "pop")) {
        Desktop::Gnome
    } else {
        Desktop::Other
    }
}

/// `gsettings get` prints a boolean as "true" or "false" (dconf as the same, or nothing for
/// the default).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_gsettings_bool(out: &str) -> Option<bool> {
    match out.trim().trim_start_matches("@b ").trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct Detector {
    bus: Option<zbus::blocking::Connection>,
}

#[cfg(target_os = "linux")]
impl Detector {
    fn read(&mut self) -> Status {
        let desktop = desktop_of(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default());
        // GNOME's setting only on GNOME: other desktops may have the schema without using it.
        if desktop == Desktop::Gnome {
            if let Some(banners) = gnome_banners() {
                return Status::new("gnome", "ok", !banners, "Do Not Disturb");
            }
        }
        if let Some(inhibited) = self.kde_inhibited() {
            return Status::new("kde", "ok", inhibited, "Do Not Disturb");
        }
        Status::new("", "notDetected", false, "")
    }

    /// Plasma's notification server says whether it's holding notifications back.
    fn kde_inhibited(&mut self) -> Option<bool> {
        if self.bus.is_none() {
            self.bus = zbus::blocking::Connection::session().ok();
        }
        let bus = self.bus.as_ref()?;
        let reply = bus
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.freedesktop.Notifications", "Inhibited"),
            )
            .ok()?;
        let v: zbus::zvariant::OwnedValue = reply.body().deserialize().ok()?;
        bool::try_from(v).ok()
    }
}

#[cfg(target_os = "linux")]
fn gnome_banners() -> Option<bool> {
    let out = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.notifications", "show-banners"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    parse_gsettings_bool(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOLD: Rules = Rules { mode: Mode::Hold, alerts_through: true, fullscreen_obeys: false };
    const HUSH: Decision = Decision { pop_up: false, sound: false, held: true };
    const QUIET: Decision = Decision { pop_up: true, sound: false, held: false };

    #[test]
    fn decisions() {
        let mute = Rules { mode: Mode::Mute, ..HOLD };
        let ignore = Rules { mode: Mode::Ignore, ..HOLD };
        // (rules, focused, alert, fullscreen) -> decision
        let cases = [
            // No Focus: as usual, whatever the mode
            (HOLD, false, false, false, AS_USUAL),
            (mute, false, false, false, AS_USUAL),
            (ignore, false, false, false, AS_USUAL),
            // Focus on
            (HOLD, true, false, false, HUSH),
            (mute, true, false, false, QUIET),
            (ignore, true, false, false, AS_USUAL),
            // An alert, let through
            (HOLD, true, true, false, AS_USUAL),
            (mute, true, true, false, AS_USUAL),
            // An alert, not let through, is held like any message
            (Rules { alerts_through: false, ..HOLD }, true, true, false, HUSH),
            (Rules { alerts_through: false, ..mute }, true, true, false, QUIET),
            // A full-screen kiosk ignores Focus...
            (HOLD, true, false, true, AS_USUAL),
            (mute, true, false, true, AS_USUAL),
            // ...unless it follows Focus too
            (Rules { fullscreen_obeys: true, ..HOLD }, true, false, true, HUSH),
            (Rules { fullscreen_obeys: true, ..mute }, true, false, true, QUIET),
            (Rules { fullscreen_obeys: true, ..HOLD }, true, true, true, AS_USUAL),
            (Rules { fullscreen_obeys: true, ..ignore }, true, false, true, AS_USUAL),
        ];
        for (i, (r, focused, alert, full, want)) in cases.into_iter().enumerate() {
            assert_eq!(decide(r, focused, alert, full), want, "case {i}: {r:?} focused={focused} alert={alert} full={full}");
        }
    }

    #[test]
    fn modes() {
        assert_eq!(Mode::of("hold"), Mode::Hold);
        assert_eq!(Mode::of("mute"), Mode::Mute);
        assert_eq!(Mode::of("ignore"), Mode::Ignore);
        assert_eq!(Mode::of("nonsense"), Mode::Hold);
    }

    #[test]
    fn summary() {
        let g = |c: u8| vec![vec![c; 3]];
        let mut h = Held::default();
        assert_eq!(h.take(), None, "nothing held, no summary");
        h.note(g(1), None);
        h.note(g(2), Some("https://example.com".into()));
        h.note(g(3), None);
        assert_eq!(h.take(), Some((3, HeldMessage { layout: g(3), link: None })));
        assert_eq!(h.take(), None, "shown once");
        assert_eq!(h.count, 0);
        h.note(g(4), Some("https://example.com/4".into()));
        assert_eq!(h.take().map(|(n, m)| (n, m.link)), Some((1, Some("https://example.com/4".into()))));
        assert_eq!(summary_text(1), "1 message while you were focused");
        assert_eq!(summary_text(5), "5 messages while you were focused");
    }

    #[test]
    fn linux_parsing() {
        assert_eq!(parse_gsettings_bool("false\n"), Some(false));
        assert_eq!(parse_gsettings_bool("true\n"), Some(true));
        assert_eq!(parse_gsettings_bool("@b false"), Some(false));
        assert_eq!(parse_gsettings_bool(""), None);
        assert_eq!(parse_gsettings_bool("No such schema “org.gnome.desktop.notifications”"), None);
        assert_eq!(desktop_of("ubuntu:GNOME"), Desktop::Gnome);
        assert_eq!(desktop_of("GNOME"), Desktop::Gnome);
        assert_eq!(desktop_of("pop:GNOME"), Desktop::Gnome);
        assert_eq!(desktop_of("KDE"), Desktop::Kde);
        assert_eq!(desktop_of("X-Cinnamon"), Desktop::Other);
        assert_eq!(desktop_of(""), Desktop::Other);
    }

    #[test]
    fn windows_states() {
        assert_eq!(quns_what(5, false), None, "accepts notifications");
        assert_eq!(quns_what(1, false), None, "locked is away, not a Focus");
        assert_eq!(quns_what(7, false), None, "a Store app");
        assert_eq!(quns_what(2, false), Some("A full-screen app"));
        assert_eq!(quns_what(2, true), None, "Rattleboard's own full screen");
        assert_eq!(quns_what(3, true), None);
        assert_eq!(quns_what(4, true), Some("Presentation mode"));
        assert_eq!(quns_what(6, false), Some("Quiet time"));
    }

    /// What this computer reports now: `cargo test -p rattleboard live_focus -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_focus() {
        println!("{:?}", Detector::default().read());
    }

    #[test]
    fn mac_access() {
        assert_eq!([0, 1, 2, 3, 9].map(mac_access_name), ["notDetermined", "restricted", "denied", "ok", "unavailable"]);
    }
}
