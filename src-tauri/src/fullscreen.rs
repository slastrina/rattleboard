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
    #[cfg(not(any(target_os = "macos", windows)))]
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
}
