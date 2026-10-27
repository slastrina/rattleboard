//! The screen saver: the macOS `.saver` and the Windows `.scr` that ship with the app, and
//! the local feed they read.
//!
//! The screen saver shows what this board shows while Rattleboard is running, and the time
//! and date when it isn't (its page is src/saver.html). It reads the board from a feed on
//! 127.0.0.1:7171, which runs only while the screen saver is installed:
//!
//!   GET /rattleboard/saver    {"app": "rattleboard", "rows", "cols", "display", "finish", "seq", "transition", ...}
//!
//! Only programs on this computer can reach it, and only programs: a request that carries an
//! Origin header (every cross-origin fetch a web page makes) is refused, there are no CORS
//! headers, and the Host must be this address, so a web page can neither read it nor reach it
//! through DNS rebinding. The screen savers fetch it natively (URLSession, WinHTTP) and hand
//! the JSON to their page. It reads the board; it can't change anything.
//!
//! Install (Settings → Display → Screen saver) copies the screen saver out of the app:
//! - macOS: `Rattleboard.app/Contents/Resources/Rattleboard.saver` to
//!   `~/Library/Screen Savers/`, then opens the Screen Saver settings to pick it.
//! - Windows: `Rattleboard.scr` from the app's folder to `%LOCALAPPDATA%\Rattleboard\`, and
//!   makes it the screen saver (HKCU\Control Panel\Desktop\SCRNSAVE.EXE).
//!
//! An installed screen saver follows the app: when the app is updated, the copy is replaced at
//! the next launch.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const PORT: u16 = 7171;
const PATH: &str = "/rattleboard/saver";

/// The running feed; dropping it stops it.
pub struct Feed(tauri::async_runtime::JoinHandle<()>);

impl Drop for Feed {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Starts the feed. `view` gives the board's JSON at the moment of each request.
pub async fn serve(view: Arc<dyn Fn() -> String + Send + Sync>) -> io::Result<Feed> {
    let listener = TcpListener::bind(("127.0.0.1", PORT)).await?;
    let task = tauri::async_runtime::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { continue };
            let view = view.clone();
            tauri::async_runtime::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let mut n = 0;
                // The request head, in a few reads at most, within a second.
                let read = tokio::time::timeout(Duration::from_secs(1), async {
                    while n < buf.len() {
                        let k = sock.read(&mut buf[n..]).await?;
                        if k == 0 {
                            break;
                        }
                        n += k;
                        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    io::Result::Ok(())
                })
                .await;
                if !matches!(read, Ok(Ok(()))) {
                    return;
                }
                let head = String::from_utf8_lossy(&buf[..n]);
                let (status, body) = respond(&head, &|| view());
                let reason = match status {
                    200 => "OK",
                    403 => "Forbidden",
                    404 => "Not Found",
                    _ => "Method Not Allowed",
                };
                let out = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(out.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    Ok(Feed(task))
}

/// The answer to one request head: the board for a plain GET from a program on this
/// computer, a refusal for anything else.
pub fn respond(head: &str, view: &dyn Fn() -> String) -> (u16, String) {
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
    let mut host = None;
    for l in lines {
        let Some((k, v)) = l.split_once(':') else { continue };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        match k.as_str() {
            // A web page's request: never answered.
            "origin" | "sec-fetch-site" | "sec-fetch-mode" => return (403, r#"{"error":"Only the screen saver reads this"}"#.into()),
            "host" => host = Some(v.to_ascii_lowercase()),
            _ => {}
        }
    }
    let ok_host = host.is_some_and(|h| h == format!("127.0.0.1:{PORT}") || h == format!("localhost:{PORT}") || h == "127.0.0.1" || h == "localhost");
    if !ok_host {
        return (403, r#"{"error":"Only the screen saver reads this"}"#.into());
    }
    if method != "GET" {
        return (405, r#"{"error":"GET only"}"#.into());
    }
    if target.split('?').next() != Some(PATH) {
        return (404, r#"{"error":"Not found"}"#.into());
    }
    (200, view())
}

// ---------------------------------------------------------------- install

/// Where the screen saver is installed, and whether the app has one to install.
#[derive(serde::Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// This platform has a screen saver (macOS, Windows)
    pub supported: bool,
    /// This copy of the app carries one (a build from source may not)
    pub bundled: bool,
    pub installed: bool,
    /// The installed copy is this version's
    pub current: bool,
    /// Windows: it's the screen saver Windows uses now
    pub active: bool,
    pub path: Option<String>,
}

/// The screen saver inside the app (or, in a debug build, the one the build script made).
pub fn bundled(resource_dir: Option<PathBuf>) -> Option<PathBuf> {
    let name = if cfg!(target_os = "macos") { "Rattleboard.saver" } else { "Rattleboard.scr" };
    let mut places: Vec<PathBuf> = resource_dir.into_iter().map(|d| d.join(name)).collect();
    if cfg!(debug_assertions) {
        places.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/saver").join(name));
    }
    // An empty file is the stand-in a Windows build has until the real one is built (build.rs).
    places.into_iter().find(|p| p.is_dir() || std::fs::metadata(p).is_ok_and(|m| m.len() > 0))
}

/// Where it goes.
pub fn target() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        dirs_home().map(|h| h.join("Library/Screen Savers/Rattleboard.saver"))
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Rattleboard").join("Rattleboard.scr"))
    } else {
        None
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The file whose bytes say which build a screen saver is: the `.saver`'s executable, or the `.scr`.
fn exe_of(p: &Path) -> PathBuf {
    if p.extension().is_some_and(|e| e == "saver") {
        p.join("Contents/MacOS/Rattleboard")
    } else {
        p.to_path_buf()
    }
}

/// Two copies are the same build.
fn same(a: &Path, b: &Path) -> bool {
    match (std::fs::read(exe_of(a)), std::fs::read(exe_of(b))) {
        (Ok(x), Ok(y)) => x == y && {
            // The page inside a .saver can change without the executable changing.
            let (pa, pb) = (a.join("Contents/Resources/web/saver.js"), b.join("Contents/Resources/web/saver.js"));
            !pa.exists() || std::fs::read(pa).ok() == std::fs::read(pb).ok()
        },
        _ => false,
    }
}

pub fn status(resource_dir: Option<PathBuf>) -> Status {
    let supported = cfg!(any(target_os = "macos", windows));
    let src = bundled(resource_dir);
    let Some(dst) = target() else { return Status { supported, bundled: src.is_some(), ..Default::default() } };
    let installed = dst.exists();
    Status {
        supported,
        bundled: src.is_some(),
        installed,
        current: installed && src.as_ref().is_some_and(|s| same(s, &dst)),
        active: installed && windows_active(&dst),
        path: installed.then(|| dst.display().to_string()),
    }
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        let ty = e.file_type()?;
        if ty.is_dir() {
            copy_dir(&src, &dst)?;
        } else if ty.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(&src)?, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// Copies the app's screen saver into place, replacing an older copy. Windows also makes it
/// the screen saver in use when `activate` is set (Install; not on a quiet update).
pub fn install(resource_dir: Option<PathBuf>, activate: bool) -> Result<PathBuf, String> {
    let src = bundled(resource_dir).ok_or("This copy of Rattleboard doesn't include the screen saver")?;
    let dst = target().ok_or("Screen savers are for macOS and Windows")?;
    let parent = dst.parent().ok_or("No folder to install into")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("Couldn't create {}: {e}", parent.display()))?;
    // Into a new name first, then swapped in, so a screen saver that's running never sees half a copy.
    let tmp = parent.join(".Rattleboard-new");
    let _ = remove(&tmp);
    let res = if src.is_dir() { copy_dir(&src, &tmp) } else { std::fs::copy(&src, &tmp).map(|_| ()) };
    res.map_err(|e| format!("Couldn't copy the screen saver: {e}"))?;
    let _ = remove(&dst);
    std::fs::rename(&tmp, &dst).map_err(|e| format!("Couldn't put the screen saver in place: {e}"))?;
    #[cfg(windows)]
    if activate {
        windows_activate(&dst)?;
    }
    #[cfg(not(windows))]
    let _ = activate;
    Ok(dst)
}

fn remove(p: &Path) -> io::Result<()> {
    if p.is_dir() {
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    }
}

pub fn uninstall() -> Result<(), String> {
    let dst = target().ok_or("Screen savers are for macOS and Windows")?;
    #[cfg(windows)]
    if windows_active(&dst) {
        // Back to no screen saver rather than one that's gone.
        let _ = reg(&["delete", r"HKCU\Control Panel\Desktop", "/v", "SCRNSAVE.EXE", "/f"]);
    }
    if dst.exists() {
        remove(&dst).map_err(|e| format!("Couldn't remove {}: {e}", dst.display()))?;
    }
    Ok(())
}

/// An installed copy from an older version is replaced with this version's (at launch).
pub fn refresh(resource_dir: Option<PathBuf>) -> bool {
    let s = status(resource_dir.clone());
    s.installed && s.bundled && !s.current && install(resource_dir, false).is_ok()
}

/// Opens the system's screen saver settings, where the user picks Rattleboard.
pub fn open_settings() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("x-apple.systempreferences:com.apple.ScreenSaver-Settings.extension").status();
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("control.exe").args(["desk.cpl,,@screensaver"]).creation_flags(0x0800_0000).spawn();
    }
}

#[cfg(windows)]
fn reg(args: &[&str]) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("reg.exe").args(args).creation_flags(0x0800_0000).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[cfg(windows)]
fn windows_activate(scr: &Path) -> Result<(), String> {
    let desk = r"HKCU\Control Panel\Desktop";
    let path = scr.display().to_string();
    reg(&["add", desk, "/v", "SCRNSAVE.EXE", "/t", "REG_SZ", "/d", &path, "/f"]).map_err(|e| format!("Couldn't set the screen saver: {e}"))?;
    reg(&["add", desk, "/v", "ScreenSaveActive", "/t", "REG_SZ", "/d", "1", "/f"]).map_err(|e| format!("Couldn't turn the screen saver on: {e}"))?;
    Ok(())
}

fn windows_active(_scr: &Path) -> bool {
    #[cfg(windows)]
    {
        return reg(&["query", r"HKCU\Control Panel\Desktop", "/v", "SCRNSAVE.EXE"])
            .is_ok_and(|o| o.to_ascii_lowercase().contains(&_scr.display().to_string().to_ascii_lowercase()));
    }
    #[allow(unreachable_code)]
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(lines: &[&str]) -> String {
        lines.join("\r\n") + "\r\n\r\n"
    }

    #[test]
    fn only_programs_on_this_computer() {
        let view = || r#"{"app":"rattleboard"}"#.to_string();
        let ok = head(&["GET /rattleboard/saver HTTP/1.1", "Host: 127.0.0.1:7171", "Accept: */*"]);
        assert_eq!(respond(&ok, &view), (200, view()));
        // A web page's fetch (any origin, file:// pages included) is refused.
        let page = head(&["GET /rattleboard/saver HTTP/1.1", "Host: 127.0.0.1:7171", "Origin: https://evil.example"]);
        assert_eq!(respond(&page, &view).0, 403);
        let null = head(&["GET /rattleboard/saver HTTP/1.1", "Host: 127.0.0.1:7171", "Origin: null"]);
        assert_eq!(respond(&null, &view).0, 403);
        let nav = head(&["GET /rattleboard/saver HTTP/1.1", "Host: 127.0.0.1:7171", "Sec-Fetch-Site: cross-site", "Sec-Fetch-Mode: navigate"]);
        assert_eq!(respond(&nav, &view).0, 403);
        // DNS rebinding: a name that points here is still the wrong host.
        let rebind = head(&["GET /rattleboard/saver HTTP/1.1", "Host: evil.example:7171"]);
        assert_eq!(respond(&rebind, &view).0, 403);
        assert_eq!(respond(&head(&["GET /rattleboard/saver HTTP/1.1"]), &view).0, 403);
        let post = head(&["POST /rattleboard/saver HTTP/1.1", "Host: localhost:7171"]);
        assert_eq!(respond(&post, &view).0, 405);
        let other = head(&["GET /local-api/message HTTP/1.1", "Host: localhost:7171"]);
        assert_eq!(respond(&other, &view).0, 404);
    }

    /// Install, status, an update and removal, into a stand-in home folder.
    #[cfg(target_os = "macos")]
    #[test]
    fn install_follows_the_app() {
        let tmp = std::env::temp_dir().join(format!("rb-saver-test-{}", std::process::id()));
        let res = tmp.join("Resources");
        let saver = res.join("Rattleboard.saver/Contents");
        std::fs::create_dir_all(saver.join("MacOS")).unwrap();
        std::fs::create_dir_all(saver.join("Resources/web")).unwrap();
        std::fs::write(saver.join("MacOS/Rattleboard"), b"build one").unwrap();
        std::fs::write(saver.join("Resources/web/saver.js"), b"page one").unwrap();
        let home = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.join("home"));
        let s = status(Some(res.clone()));
        assert!(s.supported && s.bundled && !s.installed, "{s:?}");
        install(Some(res.clone()), true).unwrap();
        let dst = tmp.join("home/Library/Screen Savers/Rattleboard.saver");
        assert_eq!(std::fs::read(dst.join("Contents/MacOS/Rattleboard")).unwrap(), b"build one");
        let s = status(Some(res.clone()));
        assert!(s.installed && s.current, "{s:?}");
        assert!(!refresh(Some(res.clone())), "nothing to update");
        // A new version of the app: its page changed.
        std::fs::write(saver.join("Resources/web/saver.js"), b"page two").unwrap();
        assert!(!status(Some(res.clone())).current);
        assert!(refresh(Some(res.clone())));
        assert_eq!(std::fs::read(dst.join("Contents/Resources/web/saver.js")).unwrap(), b"page two");
        assert!(status(Some(res.clone())).current);
        uninstall().unwrap();
        assert!(!dst.exists() && !status(Some(res)).installed);
        match home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn feed_answers_over_tcp() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        // Only when the port is free (an installed copy of the app may hold it).
        rt.block_on(async {
            let Ok(feed) = serve(Arc::new(|| r#"{"app":"rattleboard","rows":6}"#.to_string())).await else { return };
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", PORT)).await.unwrap();
            s.write_all(b"GET /rattleboard/saver HTTP/1.1\r\nHost: 127.0.0.1:7171\r\n\r\n").await.unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).await.unwrap();
            assert!(out.starts_with("HTTP/1.1 200 OK"), "{out}");
            assert!(out.ends_with(r#"{"app":"rattleboard","rows":6}"#), "{out}");
            assert!(!out.to_ascii_lowercase().contains("access-control"), "{out}");
            drop(feed);
        });
    }
}
