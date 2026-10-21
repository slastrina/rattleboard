//! Calls and presentations: while another app is using the microphone or the camera (a Teams
//! or Zoom meeting, a Slack huddle, Meet in a browser, FaceTime) or a slideshow is playing,
//! Rattleboard treats it like a Focus (see focus.rs), if the user lets it (Settings →
//! Behavior → Calls and slideshows).
//!
//! Detection, read with the Focus state by `focus::watch`; nothing here asks for permission
//! or reads what's said or shown, only whether a device is in use:
//! - macOS: Core Audio's process objects (macOS 14+) say which apps are taking input, so
//!   system services that listen (Siri, dictation, a screen recording's audio tap) can be left
//!   out; before 14, any running input-only microphone counts. Core Media IO says whether a
//!   camera is running. A Keynote or PowerPoint window covering a whole display is a slideshow.
//! - Windows: the privacy settings' record of what's using the microphone or the camera
//!   (CapabilityAccessManager's ConsentStore, where `LastUsedTimeStop` is 0 while in use).
//!   Presentation mode and full-screen apps are Focus readings already.
//! - Linux: PulseAudio / PipeWire recording streams (`pactl`), other than level meters and
//!   monitors of an output.
//!
//! A call stays on for a little while after its device goes quiet, so a moment of silence
//! (rejoining, switching a headset) doesn't let a pop-up through.

use std::time::{Duration, Instant};

/// How long a call counts after the microphone or camera was last seen in use.
const LINGER: Duration = Duration::from_secs(20);

/// What's going on, in a few words, or nothing.
#[derive(Clone, Debug, PartialEq)]
pub enum Busy {
    /// The app on a call, if it could be told
    Call(Option<String>),
    /// The app playing a slideshow
    Slideshow(String),
}

impl Busy {
    pub fn words(&self) -> String {
        match self {
            Busy::Call(Some(app)) => format!("In a call ({app})"),
            Busy::Call(None) => "In a call".into(),
            Busy::Slideshow(app) => format!("Presenting ({app})"),
        }
    }
}

/// Readings over time: a call lingers after it was last seen.
#[derive(Default)]
pub struct Watch {
    last: Option<(Busy, Instant)>,
}

impl Watch {
    /// The state now, in words ("" for nothing).
    pub fn read(&mut self) -> String {
        let seen = read();
        self.update(seen, Instant::now()).map(|b| b.words()).unwrap_or_default()
    }

    fn update(&mut self, seen: Option<Busy>, now: Instant) -> Option<Busy> {
        match seen {
            Some(b) => {
                self.last = Some((b.clone(), now));
                Some(b)
            }
            None => {
                let (b, at) = self.last.clone()?;
                // A slideshow ends when it ends; a call lingers.
                if matches!(b, Busy::Call(_)) && now.duration_since(at) < LINGER {
                    Some(b)
                } else {
                    self.last = None;
                    None
                }
            }
        }
    }
}

/// An app's name as people know it, from a process's name or bundle id: helpers and GPU
/// processes go by their app's name, and a few well-known apps by their usual one.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn app_name(name: &str, bundle: &str) -> Option<String> {
    let known = [
        ("com.microsoft.teams", "Microsoft Teams"),
        ("com.tinyspeck.slackmacgap", "Slack"),
        ("us.zoom", "Zoom"),
        ("com.google.Chrome", "Google Chrome"),
        ("com.microsoft.edgemac", "Microsoft Edge"),
        ("org.mozilla.firefox", "Firefox"),
        ("com.hnc.Discord", "Discord"),
        ("com.cisco.webex", "Webex"),
        ("com.apple.avconferenced", "FaceTime"),
        ("com.apple.FaceTime", "FaceTime"),
        ("net.whatsapp.WhatsApp", "WhatsApp"),
    ];
    if let Some((_, n)) = known.iter().find(|(b, _)| bundle.starts_with(b)) {
        return Some(n.to_string());
    }
    let mut n = name.trim_matches(|c: char| c.is_whitespace() || c == '\u{200e}');
    for cut in [" Helper", " Graphics and Media", " Web Content", " WebView", " ("] {
        if let Some(i) = n.find(cut) {
            n = &n[..i];
        }
    }
    let n = n.trim();
    (!n.is_empty()).then(|| n.to_string())
}

/// A program as Windows' privacy settings name it: a packaged app's family name
/// ("MSTeams_8wekyb3d8bbwe") or a program's path with # for \ ("C:#…#slack.exe").
#[cfg_attr(not(windows), allow(dead_code))]
pub fn consent_name(key: &str) -> String {
    let stem = if key.contains('#') {
        let file = key.rsplit('#').next().unwrap_or(key);
        file.strip_suffix(".exe").or_else(|| file.strip_suffix(".EXE")).unwrap_or(file)
    } else {
        key.split('_').next().unwrap_or(key)
    };
    let known = [
        ("msteams", "Microsoft Teams"),
        ("ms-teams", "Microsoft Teams"),
        ("teams", "Microsoft Teams"),
        ("slack", "Slack"),
        ("zoom", "Zoom"),
        ("chrome", "Google Chrome"),
        ("msedge", "Microsoft Edge"),
        ("firefox", "Firefox"),
        ("discord", "Discord"),
        ("webex", "Webex"),
        ("ciscocollabhost", "Webex"),
    ];
    let lower = stem.to_ascii_lowercase();
    known.iter().find(|(k, _)| lower == *k || lower.ends_with(&format!(".{k}"))).map_or(stem.to_string(), |(_, n)| n.to_string())
}

/// `pactl list source-outputs`: the recording streams, as (source index, app name). Level
/// meters (a volume control's "Peak detect") aren't recordings.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_source_outputs(out: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for block in out.split("Source Output #").skip(1) {
        let mut source = String::new();
        let (mut app, mut media) = (String::new(), String::new());
        for line in block.lines().map(str::trim) {
            if let Some(v) = line.strip_prefix("Source:") {
                source = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("application.name = ") {
                app = v.trim_matches('"').to_string();
            } else if let Some(v) = line.strip_prefix("media.name = ") {
                media = v.trim_matches('"').to_string();
            }
        }
        if media.eq_ignore_ascii_case("Peak detect") || app.to_ascii_lowercase().contains("volume control") {
            continue;
        }
        found.push((source, app));
    }
    found
}

/// `pactl list short sources`: the indices of the sources that are an output's monitor.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_monitors(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let (idx, name) = (f.next()?, f.next()?);
            name.ends_with(".monitor").then(|| idx.trim().to_string())
        })
        .collect()
}

// ---------------------------------------------------------------- per platform

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn read() -> Option<Busy> {
    None
}

#[cfg(target_os = "macos")]
fn read() -> Option<Busy> {
    objc2::rc::autoreleasepool(|_| mac::slideshow().map(Busy::Slideshow).or_else(|| mac::call().map(Busy::Call)))
}

#[cfg(target_os = "macos")]
mod mac {
    use super::app_name;
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use std::ffi::{c_char, c_void, CStr};
    use std::ptr::{null, null_mut};

    #[repr(C)]
    struct Addr {
        selector: u32,
        scope: u32,
        element: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }

    #[link(name = "CoreAudio", kind = "framework")]
    extern "C" {
        fn AudioObjectGetPropertyDataSize(id: u32, addr: *const Addr, qsize: u32, q: *const c_void, size: *mut u32) -> i32;
        fn AudioObjectGetPropertyData(id: u32, addr: *const Addr, qsize: u32, q: *const c_void, size: *mut u32, data: *mut c_void) -> i32;
    }
    #[link(name = "CoreMediaIO", kind = "framework")]
    extern "C" {
        fn CMIOObjectGetPropertyDataSize(id: u32, addr: *const Addr, qsize: u32, q: *const c_void, size: *mut u32) -> i32;
        fn CMIOObjectGetPropertyData(id: u32, addr: *const Addr, qsize: u32, q: *const c_void, size: u32, used: *mut u32, data: *mut c_void) -> i32;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *mut AnyObject;
        fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayBounds(display: u32) -> Rect;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    const fn code(s: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*s)
    }
    const SYSTEM: u32 = 1;
    const GLOBAL: u32 = code(b"glob");
    const INPUT: u32 = code(b"inpt");
    const OUTPUT: u32 = code(b"outp");

    fn addr(selector: &[u8; 4], scope: u32) -> Addr {
        Addr { selector: code(selector), scope, element: 0 }
    }

    /// A list of object ids, or None if the property isn't there.
    fn ids(obj: u32, selector: &[u8; 4], scope: u32) -> Option<Vec<u32>> {
        let a = addr(selector, scope);
        let mut size = 0u32;
        if unsafe { AudioObjectGetPropertyDataSize(obj, &a, 0, null(), &mut size) } != 0 {
            return None;
        }
        let mut v = vec![0u32; size as usize / 4];
        let ok = unsafe { AudioObjectGetPropertyData(obj, &a, 0, null(), &mut size, v.as_mut_ptr().cast()) } == 0;
        v.truncate(size as usize / 4);
        ok.then_some(v)
    }

    fn get_u32(obj: u32, selector: &[u8; 4]) -> Option<u32> {
        let (a, mut v, mut size) = (addr(selector, GLOBAL), 0u32, 4u32);
        (unsafe { AudioObjectGetPropertyData(obj, &a, 0, null(), &mut size, (&mut v as *mut u32).cast()) } == 0).then_some(v)
    }

    fn string(s: *mut AnyObject) -> String {
        if s.is_null() {
            return String::new();
        }
        unsafe {
            let p: *const c_char = msg_send![s, UTF8String];
            if p.is_null() {
                String::new()
            } else {
                CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        }
    }

    /// An audio process's bundle id ("" when it has none).
    fn bundle_of(process: u32) -> String {
        let a = addr(b"pbid", GLOBAL);
        let mut s: *mut AnyObject = null_mut();
        let mut size = std::mem::size_of::<*mut AnyObject>() as u32;
        if unsafe { AudioObjectGetPropertyData(process, &a, 0, null(), &mut size, (&mut s as *mut *mut AnyObject).cast()) } != 0 || s.is_null() {
            return String::new();
        }
        let b = string(s);
        unsafe { CFRelease(s as *const c_void) };
        b
    }

    /// The name a running app goes by, and its bundle id.
    fn running_app(pid: i32) -> (String, String) {
        unsafe {
            let app: *mut AnyObject = msg_send![objc2::class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid];
            if app.is_null() {
                return (String::new(), String::new());
            }
            let name: *mut AnyObject = msg_send![app, localizedName];
            let bundle: *mut AnyObject = msg_send![app, bundleIdentifier];
            (string(name), string(bundle))
        }
    }

    /// Apple's own processes that take input count only when they're a call or a recording
    /// someone started; the rest (Siri, dictation, Voice Control, a screen recording's audio)
    /// listen for other reasons.
    fn counts(bundle: &str) -> bool {
        if bundle.is_empty() {
            return false;
        }
        if !bundle.starts_with("com.apple.") {
            return true;
        }
        ["com.apple.avconferenced", "com.apple.FaceTime", "com.apple.WebKit.GPU", "com.apple.Safari", "com.apple.VoiceMemos", "com.apple.QuickTimePlayerX", "com.apple.PhotoBooth"]
            .iter()
            .any(|b| bundle.starts_with(b))
    }

    /// Another app is using a microphone or a camera: Some(its name, if it could be told).
    pub fn call() -> Option<Option<String>> {
        if let Some(app) = mic_users() {
            return Some(app);
        }
        camera_on().then_some(None)
    }

    /// The microphone's users, by Core Audio's process objects (macOS 14+), else whether an
    /// input-only microphone is running at all.
    fn mic_users() -> Option<Option<String>> {
        let me = std::process::id() as i32;
        let Some(processes) = ids(SYSTEM, b"prs#", GLOBAL).filter(|p| !p.is_empty()) else {
            return mic_running().then_some(None);
        };
        let mut found: Option<Option<String>> = None;
        for p in processes {
            if get_u32(p, b"piri") != Some(1) {
                continue;
            }
            let pid = get_u32(p, b"ppid").map_or(-1, |v| v as i32);
            let bundle = bundle_of(p);
            if pid == me || !counts(&bundle) {
                continue;
            }
            let (name, _) = running_app(pid);
            match app_name(&name, &bundle) {
                Some(n) => return Some(Some(n)),
                None => found = Some(None),
            }
        }
        found
    }

    /// Before macOS 14: a real (not virtual or aggregate) input-only device is running.
    fn mic_running() -> bool {
        let Some(devices) = ids(SYSTEM, b"dev#", GLOBAL) else { return false };
        devices.into_iter().any(|d| {
            let inputs = ids(d, b"stm#", INPUT).is_some_and(|s| !s.is_empty());
            let outputs = ids(d, b"stm#", OUTPUT).is_some_and(|s| !s.is_empty());
            let transport = get_u32(d, b"tran").unwrap_or(0);
            inputs && !outputs && transport != code(b"virt") && transport != code(b"grup") && get_u32(d, b"gone") == Some(1)
        })
    }

    fn camera_on() -> bool {
        let a = addr(b"dev#", GLOBAL);
        let mut size = 0u32;
        if unsafe { CMIOObjectGetPropertyDataSize(SYSTEM, &a, 0, null(), &mut size) } != 0 || size == 0 {
            return false;
        }
        let mut devices = vec![0u32; size as usize / 4];
        let mut used = 0u32;
        if unsafe { CMIOObjectGetPropertyData(SYSTEM, &a, 0, null(), size, &mut used, devices.as_mut_ptr().cast()) } != 0 {
            return false;
        }
        devices.truncate(used as usize / 4);
        devices.into_iter().any(|d| {
            let a = addr(b"gone", GLOBAL);
            let (mut v, mut used) = (0u32, 0u32);
            (unsafe { CMIOObjectGetPropertyData(d, &a, 0, null(), 4, &mut used, (&mut v as *mut u32).cast()) } == 0) && v != 0
        })
    }

    /// A Keynote or PowerPoint window covers a whole display: a slideshow.
    pub fn slideshow() -> Option<String> {
        const SHOWS: [(&str, &str); 3] = [("com.apple.Keynote", "Keynote"), ("com.apple.iWork.Keynote", "Keynote"), ("com.microsoft.Powerpoint", "PowerPoint")];
        let mut displays = [0u32; 16];
        let mut n = 0u32;
        if unsafe { CGGetActiveDisplayList(16, displays.as_mut_ptr(), &mut n) } != 0 {
            return None;
        }
        let screens: Vec<Rect> = displays[..n as usize].iter().map(|&d| unsafe { CGDisplayBounds(d) }).collect();
        // On-screen windows only, without the desktop's own.
        let list = unsafe { CGWindowListCopyWindowInfo(1 | 16, 0) };
        if list.is_null() {
            return None;
        }
        let mut found = None;
        unsafe {
            let count: usize = msg_send![list, count];
            let key = |k: &CStr| -> *mut AnyObject { msg_send![objc2::class!(NSString), stringWithUTF8String: k.as_ptr()] };
            let (k_pid, k_bounds) = (key(c"kCGWindowOwnerPID"), key(c"kCGWindowBounds"));
            let (kx, ky, kw, kh) = (key(c"X"), key(c"Y"), key(c"Width"), key(c"Height"));
            for i in 0..count {
                let w: *mut AnyObject = msg_send![list, objectAtIndex: i];
                let pid: *mut AnyObject = msg_send![w, objectForKey: k_pid];
                let b: *mut AnyObject = msg_send![w, objectForKey: k_bounds];
                if pid.is_null() || b.is_null() {
                    continue;
                }
                let num = |k: *mut AnyObject| -> f64 {
                    let v: *mut AnyObject = msg_send![b, objectForKey: k];
                    if v.is_null() {
                        0.0
                    } else {
                        msg_send![v, doubleValue]
                    }
                };
                let r = Rect { x: num(kx), y: num(ky), w: num(kw), h: num(kh) };
                let covers = screens.iter().any(|s| r.x <= s.x && r.y <= s.y && r.x + r.w >= s.x + s.w && r.y + r.h >= s.y + s.h);
                if !covers {
                    continue;
                }
                let pid: i32 = msg_send![pid, intValue];
                let (_, bundle) = running_app(pid);
                if let Some((_, name)) = SHOWS.iter().find(|(b, _)| *b == bundle) {
                    found = Some(name.to_string());
                    break;
                }
            }
            CFRelease(list as *const c_void);
        }
        found
    }

    #[cfg(test)]
    pub fn live() -> (Option<Option<String>>, bool, Option<String>) {
        objc2::rc::autoreleasepool(|_| (mic_users(), camera_on(), slideshow()))
    }
}

#[cfg(windows)]
fn read() -> Option<Busy> {
    win::in_use("microphone").or_else(|| win::in_use("webcam")).map(Busy::Call)
}

#[cfg(windows)]
mod win {
    use super::consent_name;
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_QWORD};

    const STORE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore";

    fn open(path: &str) -> Option<HKEY> {
        let mut k = HKEY::default();
        (unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, KEY_READ, &mut k) } == ERROR_SUCCESS).then_some(k)
    }

    fn subkeys(k: HKEY) -> Vec<String> {
        let mut names = Vec::new();
        for i in 0.. {
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            if unsafe { RegEnumKeyExW(k, i, Some(PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None) } != ERROR_SUCCESS {
                break;
            }
            names.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
        names
    }

    fn qword(k: HKEY, sub: &str, name: &str) -> Option<u64> {
        let mut v = 0u64;
        let mut size = 8u32;
        let ok = unsafe { RegGetValueW(k, &HSTRING::from(sub), &HSTRING::from(name), RRF_RT_REG_QWORD, None, Some((&mut v as *mut u64).cast()), Some(&mut size)) } == ERROR_SUCCESS;
        ok.then_some(v)
    }

    /// Started and not stopped: in use now.
    fn using(k: HKEY, sub: &str) -> bool {
        qword(k, sub, "LastUsedTimeStop") == Some(0) && qword(k, sub, "LastUsedTimeStart").is_some_and(|t| t > 0)
    }

    /// The first program using the microphone or the webcam ("microphone" | "webcam"), by name.
    pub fn in_use(what: &str) -> Option<Option<String>> {
        let root = open(&format!(r"{STORE}\{what}"))?;
        let me = std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()));
        let mut found = None;
        for name in subkeys(root) {
            if name == "NonPackaged" {
                if let Some(np) = open(&format!(r"{STORE}\{what}\NonPackaged")) {
                    for prog in subkeys(np) {
                        let own = me.as_deref().is_some_and(|m| prog.to_ascii_lowercase().ends_with(&format!("#{m}.exe")));
                        if !own && using(np, &prog) {
                            found = Some(Some(consent_name(&prog)));
                            break;
                        }
                    }
                    unsafe { let _ = RegCloseKey(np); }
                }
            } else if using(root, &name) {
                found = Some(Some(consent_name(&name)));
            }
            if found.is_some() {
                break;
            }
        }
        unsafe { let _ = RegCloseKey(root); }
        found
    }
}

#[cfg(target_os = "linux")]
fn read() -> Option<Busy> {
    let pactl = |args: &[&str]| {
        std::process::Command::new("pactl")
            .args(args)
            .env("LC_ALL", "C")
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let outputs = parse_source_outputs(&pactl(&["list", "source-outputs"])?);
    if outputs.is_empty() {
        return None;
    }
    let monitors = parse_monitors(&pactl(&["list", "short", "sources"]).unwrap_or_default());
    let (_, app) = outputs.into_iter().find(|(src, _)| !monitors.contains(src))?;
    Some(Busy::Call((!app.is_empty()).then_some(app)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lingering() {
        let mut w = Watch::default();
        let t = Instant::now();
        let teams = Busy::Call(Some("Microsoft Teams".into()));
        assert_eq!(w.update(None, t), None);
        assert_eq!(w.update(Some(teams.clone()), t), Some(teams.clone()));
        assert_eq!(w.update(None, t + Duration::from_secs(10)), Some(teams.clone()), "a moment's quiet is still the call");
        assert_eq!(w.update(None, t + LINGER + Duration::from_secs(1)), None, "then it's over");
        assert_eq!(w.update(None, t + LINGER * 2), None);
        let show = Busy::Slideshow("Keynote".into());
        assert_eq!(w.update(Some(show.clone()), t), Some(show));
        assert_eq!(w.update(None, t + Duration::from_secs(1)), None, "a slideshow ends when it ends");
        assert_eq!(teams.words(), "In a call (Microsoft Teams)");
        assert_eq!(Busy::Call(None).words(), "In a call");
        assert_eq!(Busy::Slideshow("PowerPoint".into()).words(), "Presenting (PowerPoint)");
    }

    #[test]
    fn names() {
        assert_eq!(app_name("Microsoft Teams WebView Helper", "com.microsoft.teams2.helper").as_deref(), Some("Microsoft Teams"));
        assert_eq!(app_name("Google Chrome Helper", "com.google.Chrome.helper").as_deref(), Some("Google Chrome"));
        assert_eq!(app_name("", "com.apple.avconferenced").as_deref(), Some("FaceTime"));
        assert_eq!(app_name("Safari Graphics and Media", "com.apple.WebKit.GPU").as_deref(), Some("Safari"));
        assert_eq!(app_name("Arc Helper (Renderer)", "company.thebrowser.browser.helper").as_deref(), Some("Arc"));
        assert_eq!(app_name("\u{200e}WhatsApp", "net.whatsapp.WhatsApp").as_deref(), Some("WhatsApp"));
        assert_eq!(app_name("", "com.example.thing"), None);
        assert_eq!(consent_name("MSTeams_8wekyb3d8bbwe"), "Microsoft Teams");
        assert_eq!(consent_name(r"C:#Users#sam#AppData#Local#slack#app-4.41.105#slack.exe"), "Slack");
        assert_eq!(consent_name(r"C:#Program Files#Google#Chrome#Application#chrome.exe"), "Google Chrome");
        assert_eq!(consent_name(r"C:#Tools#Recorder.exe"), "Recorder");
        assert_eq!(consent_name("Microsoft.WindowsSoundRecorder_8wekyb3d8bbwe"), "Microsoft.WindowsSoundRecorder");
    }

    #[test]
    fn pulseaudio() {
        let outputs = "Source Output #41\n\tDriver: protocol-native.c\n\tSource: 2\n\tProperties:\n\t\tmedia.name = \"Peak detect\"\n\t\tapplication.name = \"PulseAudio Volume Control\"\n\nSource Output #57\n\tSource: 3\n\tProperties:\n\t\tmedia.name = \"WebRTC\"\n\t\tapplication.name = \"Firefox\"\n";
        assert_eq!(parse_source_outputs(outputs), vec![("3".to_string(), "Firefox".to_string())]);
        assert_eq!(parse_source_outputs(""), vec![]);
        let sources = "1\talsa_output.pci-0000_00_1f.3.analog-stereo.monitor\tPipeWire\ts32le 2ch 48000Hz\tSUSPENDED\n2\talsa_input.pci-0000_00_1f.3.analog-stereo\tPipeWire\ts32le 2ch 48000Hz\tRUNNING\n";
        assert_eq!(parse_monitors(sources), vec!["1".to_string()]);
    }

    /// What this computer reports now: `cargo test -p rattleboard live_calls -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_calls() {
        #[cfg(target_os = "macos")]
        println!("mic, camera, slideshow: {:?}", mac::live());
        println!("{:?}", read());
    }
}
