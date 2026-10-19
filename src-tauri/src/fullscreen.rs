//! Full screen's platform parts: which display it covers, keeping that display awake, and
//! hiding the pointer. Window placement itself lives with the board window in `lib.rs`.

/// The display to cover, from each display's name and top-left corner: the one with the
/// remembered name, and of two with that name the one at the remembered position. None
/// means the main display, including when the remembered one is gone.
pub fn pick_screen(screens: &[(String, (i32, i32))], name: &str, pos: (i32, i32)) -> Option<usize> {
    if name.is_empty() {
        return None;
    }
    let named: Vec<usize> = (0..screens.len()).filter(|&i| screens[i].0 == name).collect();
    named.iter().copied().find(|&i| screens[i].1 == pos).or_else(|| named.first().copied())
}

/// A pinned board's spot, kept relative to its display: dropped at `spot` while that display's
/// top-left corner was at `was`, it goes to the same place on a display now at `now` (the same
/// display moved in the arrangement, or the main display while it's unplugged).
pub fn same_spot(spot: (i32, i32), was: (i32, i32), now: (i32, i32)) -> (i32, i32) {
    (spot.0 - was.0 + now.0, spot.1 - was.1 + now.1)
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, c_void, CStr};
    use std::ptr::null;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(alloc: *const c_void, s: *const c_char, encoding: u32) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOPMAssertionCreateWithName(kind: *const c_void, level: u32, name: *const c_void, id: *mut u32) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }
    const UTF8: u32 = 0x0800_0100;
    const ASSERTION_ON: u32 = 255;

    fn cf(s: &CStr) -> *const c_void {
        unsafe { CFStringCreateWithCString(null(), s.as_ptr(), UTF8) }
    }

    /// Keeps the display (and so the computer) from sleeping while idle, as `pmset -g
    /// assertions` shows; the id releases it.
    pub fn hold() -> Option<u32> {
        let (kind, name) = (cf(c"PreventUserIdleDisplaySleep"), cf(c"Rattleboard is showing the board full screen"));
        if kind.is_null() || name.is_null() {
            return None;
        }
        let mut id = 0;
        let ok = unsafe { IOPMAssertionCreateWithName(kind, ASSERTION_ON, name, &mut id) } == 0;
        unsafe {
            CFRelease(kind);
            CFRelease(name);
        }
        ok.then_some(id)
    }

    pub fn release(id: u32) {
        unsafe { IOPMAssertionRelease(id) };
    }
}

#[cfg(target_os = "macos")]
static AWAKE: std::sync::Mutex<Option<u32>> = std::sync::Mutex::new(None);

/// Linux: the session's D-Bus. Keeping the display awake asks the screen saver
/// (org.freedesktop.ScreenSaver: GNOME, KDE, Xfce and most others) and, failing that, the
/// desktop portal (Flatpak, sandboxes) to inhibit idling. Both last until released or until
/// the connection closes, so a crash never leaves the screen kept on.
#[cfg(target_os = "linux")]
pub mod linux {
    use zbus::blocking::Connection;

    pub enum Held {
        ScreenSaver(Connection, u32),
        Portal(Connection, zbus::zvariant::OwnedObjectPath),
    }

    const REASON: &str = "Rattleboard is showing the board full screen";

    pub fn hold() -> Option<Held> {
        let conn = Connection::session().ok()?;
        for path in ["/org/freedesktop/ScreenSaver", "/ScreenSaver"] {
            let r = conn.call_method(Some("org.freedesktop.ScreenSaver"), path, Some("org.freedesktop.ScreenSaver"), "Inhibit", &("Rattleboard", REASON));
            if let Ok(cookie) = r.and_then(|m| m.body().deserialize::<u32>()) {
                return Some(Held::ScreenSaver(conn, cookie));
            }
        }
        // 8: idle
        let mut options = std::collections::HashMap::<&str, zbus::zvariant::Value>::new();
        options.insert("reason", REASON.into());
        let r = conn.call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.Inhibit"),
            "Inhibit",
            &("", 8u32, options),
        );
        let handle = r.and_then(|m| m.body().deserialize::<zbus::zvariant::OwnedObjectPath>()).ok()?;
        Some(Held::Portal(conn, handle))
    }

    pub fn release(h: Held) {
        match h {
            Held::ScreenSaver(conn, cookie) => {
                for path in ["/org/freedesktop/ScreenSaver", "/ScreenSaver"] {
                    let r = conn.call_method(Some("org.freedesktop.ScreenSaver"), path, Some("org.freedesktop.ScreenSaver"), "UnInhibit", &(cookie,));
                    if r.is_ok() {
                        break;
                    }
                }
            }
            Held::Portal(conn, handle) => {
                let _ = conn.call_method(Some("org.freedesktop.portal.Desktop"), handle.as_str(), Some("org.freedesktop.portal.Request"), "Close", &());
            }
        }
    }

    /// Whether a tray icon would show: the AppIndicator library loads, and something on the
    /// session bus shows StatusNotifier icons (KDE, most desktops; GNOME only with the
    /// AppIndicator extension). Without either, tray-icon would panic or show nothing.
    pub fn tray_available() -> bool {
        let lib = ["libayatana-appindicator3.so.1", "libappindicator3.so.1", "libayatana-appindicator3.so", "libappindicator3.so"]
            .iter()
            .any(|name| unsafe { libloading::Library::new(name) }.is_ok());
        if !lib {
            return false;
        }
        let Ok(conn) = Connection::session() else { return false };
        conn.call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &("org.kde.StatusNotifierWatcher",))
            .and_then(|m| m.body().deserialize::<bool>())
            .unwrap_or(false)
    }
}


/// Keeps the display awake while the board is full screen. Call it on the main thread: on
/// Windows the request belongs to the thread that makes it.
pub fn keep_awake(on: bool) {
    #[cfg(target_os = "macos")]
    {
        let mut held = AWAKE.lock().unwrap();
        match (on, *held) {
            (true, None) => *held = mac::hold(),
            (false, Some(id)) => {
                mac::release(id);
                *held = None;
            }
            _ => {}
        }
    }
    #[cfg(windows)]
    {
        use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED};
        let flags = if on { ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS };
        unsafe { SetThreadExecutionState(flags) };
    }
    #[cfg(target_os = "linux")]
    {
        // D-Bus calls are quick, but never on the main thread: one worker takes them in order.
        static WORKER: std::sync::OnceLock<std::sync::Mutex<std::sync::mpsc::Sender<bool>>> = std::sync::OnceLock::new();
        let tx = WORKER.get_or_init(|| {
            let (tx, rx) = std::sync::mpsc::channel::<bool>();
            std::thread::spawn(move || {
                let mut held: Option<linux::Held> = None;
                for on in rx {
                    match (on, held.is_some()) {
                        (true, false) => held = linux::hold(),
                        (false, true) => linux::release(held.take().expect("held")),
                        _ => {}
                    }
                }
            });
            std::sync::Mutex::new(tx)
        });
        let _ = tx.lock().unwrap_or_else(|e| e.into_inner()).send(on);
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    let _ = on;
}

/// Puts a full-screen window above the menu bar and Dock, so they don't show over the board
/// even while another app is active. Queued on the main dispatch queue like tao's own level
/// changes, so it lands after any `set_always_on_top` before it.
#[cfg(target_os = "macos")]
pub fn above_menu_bar(ns_window: *mut std::ffi::c_void) {
    use std::ffi::c_void;
    extern "C" {
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(queue: *const c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
    }
    extern "C" fn raise(w: *mut c_void) {
        // NSMainMenuWindowLevel + 1
        unsafe {
            let _: () = objc2::msg_send![w as *mut objc2::runtime::AnyObject, setLevel: 25isize];
        }
    }
    unsafe { dispatch_async_f(&_dispatch_main_q, ns_window, raise) };
}

/// Stops AppKit adding its own "Enter Full Screen" to the menus, whose Control-Command-F
/// would win over Rattleboard's. Only for this run (registered, not saved), before the menu.
#[cfg(target_os = "macos")]
pub fn no_system_fullscreen_item() {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    unsafe {
        let key: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: c"NSFullScreenMenuItemEverywhere".as_ptr()];
        let no: *mut AnyObject = msg_send![class!(NSNumber), numberWithBool: false];
        let dict: *mut AnyObject = msg_send![class!(NSDictionary), dictionaryWithObject: no, forKey: key];
        let defaults: *mut AnyObject = msg_send![class!(NSUserDefaults), standardUserDefaults];
        let _: () = msg_send![defaults, registerDefaults: dict];
    }
}

/// WebKit keeps a plain Esc for itself (as `cancelOperation:`), so the board page never sees
/// it; this catches it for the board window instead. `on_escape` runs on the main thread and
/// says whether it used the key.
#[cfg(target_os = "macos")]
pub fn watch_escape(ns_window: *mut std::ffi::c_void, on_escape: impl Fn() -> bool + 'static) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    let board = ns_window as usize;
    let handler = block2::RcBlock::new(move |event: *mut AnyObject| -> *mut AnyObject {
        let (kind, code, flags, window): (usize, u16, usize, *mut AnyObject) = unsafe {
            (msg_send![event, type], msg_send![event, keyCode], msg_send![event, modifierFlags], msg_send![event, window])
        };
        // A key press (NSEventTypeKeyDown) of Esc (key code 53) alone; Caps Lock doesn't count
        if kind == 10 && code == 53 && flags & 0xfffe_0000 == 0 && window as usize == board && on_escape() {
            return std::ptr::null_mut();
        }
        event
    });
    // NSEventMaskKeyDown; AppKit keeps the monitor (and its copy of the block) for good.
    let _: *mut AnyObject = unsafe { msg_send![class!(NSEvent), addLocalMonitorForEventsMatchingMask: 1u64 << 10, handler: &*handler] };
}

/// Hides the pointer until it next moves. The board page hides it too (`cursor: none`), but
/// WebKit only applies that once the pointer moves. Main thread only.
pub fn hide_pointer() {
    #[cfg(target_os = "macos")]
    unsafe {
        let _: () = objc2::msg_send![objc2::class!(NSCursor), setHiddenUntilMouseMoves: true];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screens() {
        let s = |n: &str, x| (n.to_string(), (x, 0));
        let list = [s("Built-in Retina Display", 0), s("DELL P2419H", 1512), s("DELL P2419H", 3432)];
        assert_eq!(pick_screen(&list, "", (0, 0)), None);
        assert_eq!(pick_screen(&list, "Built-in Retina Display", (99, 99)), Some(0));
        // Two of the same model: the one where it was, else the first.
        assert_eq!(pick_screen(&list, "DELL P2419H", (3432, 0)), Some(2));
        assert_eq!(pick_screen(&list, "DELL P2419H", (0, 0)), Some(1));
        // Unplugged: the main display.
        assert_eq!(pick_screen(&list, "LG HDR 4K", (1512, 0)), None);
    }

    #[test]
    fn same_spot_on_another_display() {
        // 100 px in from the external display's corner, wherever that display is.
        assert_eq!(same_spot((1612, 50), (1512, 0), (1512, 0)), (1612, 50));
        assert_eq!(same_spot((1612, 50), (1512, 0), (-2560, -300)), (-2460, -250));
        // Unplugged: the same spot on the main display.
        assert_eq!(same_spot((1612, 50), (1512, 0), (0, 0)), (100, 50));
    }
}
