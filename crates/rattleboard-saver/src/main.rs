//! Rattleboard's Windows screen saver, `Rattleboard.scr` (this program, renamed). It shows the
//! screen saver page (src/saver.html) in WebView2, fed once a second with what the Rattleboard
//! app on this computer is showing, from its local feed (http://127.0.0.1:7171/rattleboard/saver,
//! see src-tauri/src/saver.rs); with the app not running, the page shows the time and date.
//!
//! Windows starts a screen saver with one of:
//!   /s           run it, full screen on every display, until the mouse or a key is used
//!   /p <hwnd>    draw the little preview inside Settings' window
//!   /c[:<hwnd>]  (or nothing) its settings: here, a note pointing to Rattleboard's Settings
//!
//! The Rattleboard app installs it (Settings → Display → Screen saver) into
//! %LOCALAPPDATA%\Rattleboard\Rattleboard.scr and makes it the screen saver.
//!
//! It builds on macOS and Linux too, where `/s` opens it in a window, to try the page.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::borrow::Cow;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::{Duration, Instant};
use tao::event::{ElementState, Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Fullscreen, Window, WindowBuilder};
use wry::http::{header, Response, StatusCode};
use wry::{WebView, WebViewBuilder};

const FEED: &str = "127.0.0.1:7171";
const PATH: &str = "/rattleboard/saver";

// The page, the renderer and the font, built in, as the app's screen saver page has them.
const FILES: &[(&str, &str, &[u8])] = &[
    ("saver.html", "text/html; charset=utf-8", include_bytes!("../../../src/saver.html")),
    ("saver.js", "text/javascript; charset=utf-8", include_bytes!("../../../src/saver.js")),
    ("shared/split-flap.js", "text/javascript; charset=utf-8", include_bytes!("../../../src/shared/split-flap.js")),
    ("shared/base.css", "text/css; charset=utf-8", include_bytes!("../../../src/shared/base.css")),
    ("fonts/barlow-semi-condensed-500.woff2", "font/woff2", include_bytes!("../../../src/fonts/barlow-semi-condensed-500.woff2")),
    ("fonts/barlow-semi-condensed-600.woff2", "font/woff2", include_bytes!("../../../src/fonts/barlow-semi-condensed-600.woff2")),
];

/// Input that ends it: the page reports clicks, keys and a mouse that has really moved.
const QUIT_JS: &str = r#"
(() => {
  let start = null;
  const quit = () => window.ipc.postMessage('quit');
  addEventListener('mousemove', e => {
    if (!start) { start = [e.screenX, e.screenY]; return; }
    if (Math.abs(e.screenX - start[0]) + Math.abs(e.screenY - start[1]) > 12) quit();
  }, true);
  for (const t of ['mousedown', 'keydown', 'wheel', 'touchstart', 'pointerdown']) addEventListener(t, quit, true);
})();
"#;

#[derive(Debug, PartialEq)]
enum Mode {
    Run,
    Preview(isize),
    Configure,
}

/// Windows passes "/s", "/p 1234", "/p:1234", "/c", "/c:1234", in any case; nothing means /c.
fn mode(args: &[String]) -> Mode {
    let first = args.get(1).map(|a| a.to_ascii_lowercase()).unwrap_or_default();
    let (flag, rest) = match first.split_once(':') {
        Some((f, r)) => (f.to_string(), Some(r.to_string())),
        None => (first.clone(), args.get(2).cloned()),
    };
    match flag.trim_start_matches(['/', '-']) {
        "s" => Mode::Run,
        "p" => rest.and_then(|h| h.trim().parse().ok()).map_or(Mode::Configure, Mode::Preview),
        _ => Mode::Configure,
    }
}

/// The app's board now (its JSON), or None when the app isn't running or the feed isn't on.
fn fetch() -> Option<String> {
    let addr: SocketAddr = FEED.parse().ok()?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(500)).ok()?;
    s.set_read_timeout(Some(Duration::from_millis(800))).ok()?;
    s.write_all(format!("GET {PATH} HTTP/1.1\r\nHost: {FEED}\r\nConnection: close\r\n\r\n").as_bytes()).ok()?;
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let _ = s.shutdown(Shutdown::Both);
    let text = String::from_utf8(out).ok()?;
    let (head, body) = text.split_once("\r\n\r\n")?;
    if !head.starts_with("HTTP/1.1 200") {
        return None;
    }
    // Only a JSON object goes into the page.
    serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(body).ok()?;
    Some(body.to_string())
}

enum Msg {
    View(Option<String>),
    Quit,
}

fn serve(path: &str) -> Response<Cow<'static, [u8]>> {
    let path = path.trim_start_matches('/');
    match FILES.iter().find(|(p, _, _)| *p == path) {
        Some((_, ty, body)) => Response::builder()
            .header(header::CONTENT_TYPE, *ty)
            .header(header::CACHE_CONTROL, "no-store")
            .body(Cow::Borrowed(*body))
            .unwrap(),
        None => Response::builder().status(StatusCode::NOT_FOUND).body(Cow::Borrowed(&b""[..])).unwrap(),
    }
}

fn page_url() -> &'static str {
    if cfg!(windows) {
        "http://rbsaver.localhost/saver.html"
    } else {
        "rbsaver://localhost/saver.html"
    }
}

fn webview(window: &Window, proxy: EventLoopProxy<Msg>, preview: bool, ctx: &mut wry::WebContext) -> wry::Result<WebView> {
    let mut b = WebViewBuilder::new_with_web_context(ctx)
        .with_custom_protocol("rbsaver".into(), |_id, req| serve(req.uri().path()))
        .with_url(page_url())
        .with_background_color((0, 0, 0, 255))
        .with_devtools(false);
    if !preview {
        b = b.with_initialization_script(QUIT_JS).with_ipc_handler(move |req| {
            if req.body() == "quit" {
                let _ = proxy.send_event(Msg::Quit);
            }
        });
    }
    b.build(window)
}

#[cfg(windows)]
fn configure_note() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let text = wide(
        "The Rattleboard screen saver shows what your Rattleboard is showing, or the time and date when Rattleboard isn't running.\n\n\
         To remove it, open Rattleboard → Settings → Display → Screen saver.",
    );
    let title = wide("Rattleboard screen saver");
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONINFORMATION) };
}

#[cfg(not(windows))]
fn configure_note() {
    println!("Rattleboard screen saver: run with /s to show it.");
}

/// The size of the Settings preview pane.
#[cfg(windows)]
fn client_size(hwnd: isize) -> Option<(u32, u32)> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let ok = unsafe { GetClientRect(hwnd as _, &mut r) } != 0;
    ok.then(|| ((r.right - r.left).max(1) as u32, (r.bottom - r.top).max(1) as u32))
}

#[cfg(windows)]
fn parent_alive(hwnd: isize) -> bool {
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(hwnd as _) != 0 }
}

#[cfg(not(windows))]
fn parent_alive(_hwnd: isize) -> bool {
    true
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = mode(&args);
    if mode == Mode::Configure {
        configure_note();
        return;
    }
    let preview = matches!(mode, Mode::Preview(_));

    let event_loop = EventLoopBuilder::<Msg>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // WebView2 keeps its data next to the installed screen saver's folder, not in System32.
    let data = std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join("Rattleboard").join("saver-webview"));
    let mut ctx = wry::WebContext::new(data);

    let mut windows: Vec<(Window, WebView)> = Vec::new();
    match mode {
        Mode::Preview(_hwnd) => {
            #[cfg(windows)]
            let window = {
                use tao::platform::windows::WindowBuilderExtWindows;
                let (w, h) = client_size(_hwnd).unwrap_or((152, 112));
                WindowBuilder::new()
                    .with_parent_window(_hwnd)
                    .with_decorations(false)
                    .with_inner_size(tao::dpi::PhysicalSize::new(w, h))
                    .with_position(tao::dpi::PhysicalPosition::new(0, 0))
                    .build(&event_loop)
            };
            #[cfg(not(windows))]
            let window = WindowBuilder::new().with_title("Rattleboard preview").with_inner_size(tao::dpi::LogicalSize::new(320.0, 200.0)).build(&event_loop);
            let Ok(window) = window else { return };
            let Ok(wv) = webview(&window, proxy.clone(), true, &mut ctx) else { return };
            windows.push((window, wv));
        }
        _ => {
            let monitors: Vec<_> = event_loop.available_monitors().collect();
            for m in monitors {
                let b = WindowBuilder::new().with_title("Rattleboard").with_decorations(false);
                let b = if cfg!(windows) {
                    b.with_fullscreen(Some(Fullscreen::Borderless(Some(m)))).with_always_on_top(true)
                } else {
                    b.with_inner_size(tao::dpi::LogicalSize::new(1280.0, 720.0))
                };
                let window = match b.build(&event_loop) {
                    Ok(w) => w,
                    Err(e) => {
                        eprintln!("rattleboard-saver: window: {e}");
                        continue;
                    }
                };
                window.set_cursor_visible(false);
                let wv = match webview(&window, proxy.clone(), false, &mut ctx) {
                    Ok(w) => w,
                    Err(e) => {
                        eprintln!("rattleboard-saver: webview: {e}");
                        continue;
                    }
                };
                windows.push((window, wv));
                if !cfg!(windows) {
                    break;
                }
            }
        }
    }
    if windows.is_empty() {
        eprintln!("rattleboard-saver: nothing to show it on");
        return;
    }

    // The feed, once a second, off the UI thread.
    let feed_proxy = proxy.clone();
    std::thread::spawn(move || loop {
        if feed_proxy.send_event(Msg::View(fetch())).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    });

    let parent = if let Mode::Preview(h) = mode { Some(h) } else { None };
    let started = Instant::now();
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_secs(1));
        match event {
            Event::UserEvent(Msg::View(v)) => {
                let js = format!("window.saverUpdate && window.saverUpdate({})", v.as_deref().unwrap_or("null"));
                for (_, wv) in &windows {
                    let _ = wv.evaluate_script(&js);
                }
                // The preview pane closed: so does the preview.
                if parent.is_some_and(|h| !parent_alive(h)) {
                    *flow = ControlFlow::Exit;
                }
            }
            Event::UserEvent(Msg::Quit) => *flow = ControlFlow::Exit,
            // A key or click that reaches the window rather than the page, after a moment's
            // grace (Windows sends a stray event or two as the screen saver starts).
            Event::WindowEvent { event: WindowEvent::KeyboardInput { event, .. }, .. }
                if !preview && event.state == ElementState::Pressed && started.elapsed() > Duration::from_millis(500) =>
            {
                *flow = ControlFlow::Exit
            }
            Event::WindowEvent { event: WindowEvent::MouseInput { .. }, .. } if !preview && started.elapsed() > Duration::from_millis(500) => {
                *flow = ControlFlow::Exit
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *flow = ControlFlow::Exit,
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        std::iter::once("Rattleboard.scr").chain(a.iter().copied()).map(String::from).collect()
    }

    #[test]
    fn windows_arguments() {
        assert_eq!(mode(&args(&["/s"])), Mode::Run);
        assert_eq!(mode(&args(&["/S"])), Mode::Run);
        assert_eq!(mode(&args(&["-s"])), Mode::Run);
        assert_eq!(mode(&args(&["/p", "1234"])), Mode::Preview(1234));
        assert_eq!(mode(&args(&["/P:5678"])), Mode::Preview(5678));
        assert_eq!(mode(&args(&["/p"])), Mode::Configure);
        assert_eq!(mode(&args(&["/c:99"])), Mode::Configure);
        assert_eq!(mode(&args(&["/c"])), Mode::Configure);
        assert_eq!(mode(&args(&[])), Mode::Configure);
    }

    #[test]
    fn page_files() {
        assert_eq!(serve("/saver.html").status(), StatusCode::OK);
        assert_eq!(serve("/shared/split-flap.js").status(), StatusCode::OK);
        assert_eq!(serve("/fonts/barlow-semi-condensed-600.woff2").status(), StatusCode::OK);
        assert_eq!(serve("/../secret").status(), StatusCode::NOT_FOUND);
        let html = String::from_utf8_lossy(FILES[0].2);
        for (p, _, _) in FILES.iter().skip(1).take(3) {
            assert!(html.contains(p), "saver.html doesn't load {p}");
        }
    }
}
