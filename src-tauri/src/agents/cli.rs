//! The command-line modes, decided before anything of the app starts (no window, tray or
//! single-instance check):
//!
//!   rattleboard --mcp [--target <board>] [--url <http://host:port>]
//!       an MCP server on stdio, for agent apps
//!   rattleboard --notify "<text>" [--color green] [--seconds 20] [--source "Claude Code"] [--hook] [--target …] [--url …]
//!       one notification, for agent apps' hooks: shown once (POST /rattleboard/notify), with
//!       --seconds going away again, and labelled in the board's history "<source> via hook"
//!
//! By default both talk to the running app's API server on this computer, with the key from
//! the OS credential store, so the key never has to be written into an agent's config.
//! `--target` makes calls that name no board go to a saved board or group (by id or name) via
//! the app's Send. `--url` talks to another Rattleboard directly instead, with no app needed
//! here: its key is the matching saved board's (from the credential store) or else
//! $RATTLEBOARD_KEY. Neither mode prints a key, and `--notify` prints nothing on stdout (some
//! hooks read it).

use rattleboard_core::boards::{self, SavedBoard};
use rattleboard_core::{mcp, secrets, settings::Settings, vestaboard};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

const IDENTIFIER: &str = "io.github.slastrina.rattleboard";
pub const KEY_ENV: &str = "RATTLEBOARD_KEY";

/// Runs a command-line mode if the arguments ask for one, returning its exit code.
pub fn main() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--mcp") => Some(run_mcp(&args[1..])),
        Some("--notify") => Some(run_notify(&args[1..])),
        _ => None,
    }
}

/// The app's config folder, as Tauri names it, and the profile's own in a debug build.
fn config_dir() -> PathBuf {
    crate::profile_dir(dirs::config_dir().unwrap_or_default().join(IDENTIFIER))
}

/// Where the tools go: `--target` and `--url`, shared by both modes.
#[derive(Debug, Default, PartialEq)]
pub struct Route {
    pub target: Option<String>,
    pub url: Option<String>,
}

impl Route {
    /// Takes `--target`/`--url` and its value if `a` is one; Ok(false) if it isn't.
    fn take(&mut self, a: &str, it: &mut std::slice::Iter<String>) -> Result<bool, String> {
        match a {
            "--target" => {
                let t = it.next().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).ok_or("--target needs a saved board or group")?;
                self.target = Some(t);
            }
            "--url" => self.url = Some(check_url(it.next().ok_or("--url needs an address, like http://192.168.1.20:7070")?)?),
            _ => return Ok(false),
        }
        Ok(true)
    }
}

/// A Rattleboard's base address: http or https, a host, an optional port, nothing else.
pub fn check_url(s: &str) -> Result<String, String> {
    let bad = || format!("--url needs a Rattleboard's address, like http://192.168.1.20:7070 (got \"{s}\")");
    let u = url::Url::parse(s.trim()).map_err(|_| bad())?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none_or(str::is_empty) {
        return Err(bad());
    }
    if !u.username().is_empty() || u.password().is_some() || u.query().is_some() || u.fragment().is_some() || !u.path().trim_end_matches('/').is_empty() {
        return Err(format!("{} (just the address: no path, query or user name)", bad()));
    }
    let host = u.host_str().unwrap_or_default();
    Ok(match u.port() {
        Some(p) => format!("{}://{host}:{p}", u.scheme()),
        None => format!("{}://{host}", u.scheme()),
    })
}

/// Where a remote Rattleboard's key came from.
#[derive(Debug, PartialEq)]
pub enum KeyFrom {
    /// The saved board with this name, from the credential store
    SavedBoard(String),
    /// $RATTLEBOARD_KEY
    Env,
    Nowhere,
}

/// The saved board at `url`, if there is one: same host and port (a saved board's default
/// port is 7070).
fn saved_board<'a>(url: &str, saved: &'a [SavedBoard]) -> Option<&'a SavedBoard> {
    let u = url::Url::parse(url).ok()?;
    let want = (u.host_str()?.to_lowercase(), u.port_or_known_default()?);
    saved.iter().filter(|b| b.kind == "rattleboard").find(|b| {
        url::Url::parse(&vestaboard::base_url(&b.host, boards::RATTLEBOARD_PORT))
            .ok()
            .and_then(|s| Some((s.host_str()?.to_lowercase(), s.port_or_known_default()?)))
            == Some(want.clone())
    })
}

/// The key for `url`: the saved board's own (from the credential store, via `get`) first,
/// then $RATTLEBOARD_KEY.
pub fn remote_key(url: &str, saved: &[SavedBoard], get: &dyn Fn(&str) -> String, env: Option<String>) -> (String, KeyFrom) {
    if let Some(b) = saved_board(url, saved) {
        let k = get(&secrets::board_account(&b.id));
        if !k.is_empty() {
            return (k, KeyFrom::SavedBoard(b.name.clone()));
        }
    }
    match env.map(|e| e.trim().to_string()).filter(|e| !e.is_empty()) {
        Some(k) => (k, KeyFrom::Env),
        None => (String::new(), KeyFrom::Nowhere),
    }
}

fn backend(route: &Route) -> mcp::Http {
    if crate::profile().is_some() {
        secrets::use_file(config_dir().join("secrets.json"));
    }
    let mut b = match route.url.clone() {
        None => mcp::Http::locating(keeping_key(Box::new(|| {
            // The local API server's address and key, read fresh each time.
            let s = Settings::load(&config_dir().join("settings.json"));
            (format!("http://127.0.0.1:{}", s.server_port), secrets::get(secrets::SERVER_KEY))
        }))),
        Some(url) => {
            let u = url.clone();
            let mut b = mcp::Http::locating(keeping_key(Box::new(move || {
                let saved = boards::load(&config_dir().join("boards.json"));
                let (key, _) = remote_key(&u, &saved, &secrets::get, std::env::var(KEY_ENV).ok());
                (u.clone(), key)
            })));
            b.unreachable = format!("Couldn't reach the Rattleboard at {url}. Check that it's running with its API server on (and Other devices on, if it's another computer).");
            b.no_key = format!(
                "No key for {url}. Add that Rattleboard in Settings → Boards on this computer, so its key is kept in the keychain, or set {KEY_ENV}."
            );
            b
        }
    };
    b.target = route.target.clone();
    b.wait = std::time::Duration::from_secs(10);
    b
}

/// Remembers the last key that could be read, and reads it once straight away. After
/// `brew upgrade` deletes the app this process was started from, macOS may no longer let it
/// read the keychain, which made every call say Rattleboard wasn't reachable until the agent
/// app reconnected. A new key (Regenerate) is still picked up whenever it can be read.
fn keeping_key(locate: mcp::Locate) -> mcp::Locate {
    let kept = std::sync::Mutex::new(String::new());
    let f: mcp::Locate = Box::new(move || {
        let (base, key) = locate();
        let mut k = kept.lock().unwrap();
        if key.is_empty() {
            (base, k.clone())
        } else {
            k.clone_from(&key);
            (base, key)
        }
    });
    f();
    f
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime")
}

pub const MCP_USAGE: &str = "Usage: rattleboard --mcp [--target <saved board or group>] [--url http://host:7070]";

pub fn parse_mcp(args: &[String]) -> Result<Route, String> {
    let mut r = Route::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if !r.take(a, &mut it)? {
            return Err(format!("Unknown option {a}"));
        }
    }
    Ok(r)
}

fn run_mcp(args: &[String]) -> i32 {
    let route = match parse_mcp(args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}\n{MCP_USAGE}");
            return 2;
        }
    };
    let b = backend(&route);
    let me = Exe::now();
    let mut client = std::env::var(CLIENT_ENV).ok().filter(|c| !c.is_empty());
    let (mut input, mut output) = (tokio::io::BufReader::new(tokio::io::stdin()), tokio::io::stdout());
    let rt = runtime();
    let mut watch = true;
    loop {
        match rt.block_on(mcp::serve_until(&b, client.clone(), &mut input, &mut output, &|| watch && me.replaced())) {
            Ok(None) => return 0,
            Ok(Some(c)) => {
                // Only comes back if the new version couldn't be started: carry on as we are.
                me.hand_over(args, c.clone());
                client = c;
                watch = false;
            }
            Err(e) => {
                eprintln!("rattleboard --mcp: {e}");
                return 1;
            }
        }
    }
}

/// The client's name (from `initialize`), for a copy handed over to by `Exe::hand_over`.
const CLIENT_ENV: &str = "RATTLEBOARD_MCP_CLIENT";

/// This program's file, to notice when an update replaces it under a running `--mcp`.
struct Exe {
    path: Option<PathBuf>,
    id: Option<(u64, u64)>,
}

impl Exe {
    fn now() -> Exe {
        let path = std::env::current_exe().ok();
        let id = path.as_deref().and_then(file_id);
        Exe { path, id }
    }

    /// The app at this path is now another file: an update (brew upgrade, or the app's own)
    /// put a new version there. While it's missing (mid-update) it isn't replaced yet, nor
    /// while it's still being written.
    fn replaced(&self) -> bool {
        let (Some(p), Some(id)) = (&self.path, self.id) else { return false };
        let size = |p: &std::path::Path| std::fs::metadata(p).map(|m| m.len()).ok();
        match file_id(p) {
            Some(now) if now != id => {
                let before = size(p);
                std::thread::sleep(std::time::Duration::from_secs(1));
                file_id(p) == Some(now) && size(p) == before
            }
            _ => false,
        }
    }

    /// Becomes the new version, keeping stdin and stdout, so the agent app's connection to
    /// Rattleboard carries on without a reconnect. Unix only. Returns only if it couldn't.
    #[cfg(unix)]
    fn hand_over(&self, args: &[String], client: Option<String>) {
        use std::os::unix::process::CommandExt;
        let Some(p) = &self.path else { return };
        let mut c = std::process::Command::new(p);
        c.arg("--mcp").args(args);
        match client {
            Some(n) => c.env(CLIENT_ENV, n),
            None => c.env_remove(CLIENT_ENV),
        };
        let e = c.exec();
        eprintln!("rattleboard --mcp: couldn't start the updated app ({e}); carrying on");
    }

    #[cfg(not(unix))]
    fn hand_over(&self, _args: &[String], _client: Option<String>) {}
}

#[cfg(unix)]
fn file_id(p: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().map(|m| (m.dev(), m.ino()))
}

/// Windows can't replace a running program, so its installer closes `--mcp` instead.
#[cfg(not(unix))]
fn file_id(_: &std::path::Path) -> Option<(u64, u64)> {
    None
}

#[derive(Debug, PartialEq)]
pub struct Notify {
    pub text: String,
    pub color: Option<String>,
    pub seconds: Option<u64>,
    /// Run from an agent's hook: the hook's JSON comes on stdin, and failures don't fail it
    pub hook: bool,
    /// A JSON payload as the last argument (Codex's notify)
    pub payload: Option<Value>,
    /// The agent app, for the board's history
    pub source: Option<String>,
    pub route: Route,
}

pub const NOTIFY_USAGE: &str = "Usage: rattleboard --notify \"<text>\" [--color red|orange|yellow|green|blue|violet|white|black] [--seconds 3-3600] [--source <agent app>] [--hook] [--target <saved board or group>] [--url http://host:7070]";

/// `--notify`'s arguments (after `--notify`).
pub fn parse_notify(args: &[String]) -> Result<Notify, String> {
    let mut n = Notify { text: String::new(), color: None, seconds: None, hook: false, payload: None, source: None, route: Route::default() };
    let mut it = args.iter();
    let mut words: Vec<&str> = vec![];
    while let Some(a) = it.next() {
        if n.route.take(a, &mut it)? {
            continue;
        }
        match a.as_str() {
            "--color" | "--colour" => n.color = Some(it.next().ok_or("--color needs a colour")?.clone()),
            "--seconds" => {
                let s = it.next().ok_or("--seconds needs a number")?;
                n.seconds = Some(s.parse().ok().filter(|v| (3..=3600).contains(v)).ok_or("--seconds must be a whole number from 3 to 3600")?);
            }
            "--hook" => n.hook = true,
            "--source" => n.source = Some(it.next().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).ok_or("--source needs the agent app's name")?),
            s if s.starts_with('{') && serde_json::from_str::<Value>(s).is_ok_and(|v| v.is_object()) => n.payload = serde_json::from_str(s).ok(),
            s if s.starts_with("--") => return Err(format!("Unknown option {s}")),
            s => words.push(s),
        }
    }
    n.text = words.join(" ").trim().to_string();
    if n.text.is_empty() {
        return Err("Say what to show".into());
    }
    if n.seconds.is_some() && n.route.target.is_some() {
        return Err("--seconds shows a status on this board (or the --url one), so it can't go with --target".into());
    }
    Ok(n)
}

/// The project an agent is working in, from its hook or notify payload, as a short line.
fn project(payload: Option<&Value>) -> Option<String> {
    let cwd = payload?.get("cwd")?.as_str()?;
    let name = std::path::Path::new(cwd).file_name()?.to_string_lossy().to_string();
    let name: String = name.chars().take(22).collect();
    (!name.is_empty()).then_some(name)
}

/// The tool call `--notify` makes: a message that stays, or with --seconds one that goes.
/// (--target reaches it as the backend's default target.)
pub fn notify_call(n: &Notify, stdin: Option<Value>) -> (&'static str, Map<String, Value>) {
    let payload = n.payload.clone().or(stdin);
    let text = match project(payload.as_ref()) {
        Some(p) => format!("{}\n{p}", n.text),
        None => n.text.clone(),
    };
    let mut args = Map::new();
    args.insert("text".into(), json!(text));
    if let Some(c) = &n.color {
        args.insert("color".into(), json!(c));
    }
    match n.seconds {
        Some(s) => {
            args.insert("seconds".into(), json!(s));
            ("show_status", args)
        }
        None => ("send_message", args),
    }
}

/// What the board's history calls a `--notify`: "Claude Code via hook".
pub fn hook_source(n: &Notify) -> String {
    let app: String = n.source.as_deref().unwrap_or(mcp::AGENT).chars().filter(|c| !c.is_control()).take(24).collect();
    format!("{} via hook", app.trim())
}

fn run_notify(args: &[String]) -> i32 {
    let n = match parse_notify(args) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{e}\n{NOTIFY_USAGE}");
            return 2;
        }
    };
    let stdin = n.hook.then(|| {
        use std::io::Read;
        let mut s = String::new();
        let _ = std::io::stdin().take(1 << 20).read_to_string(&mut s);
        serde_json::from_str::<Value>(&s).ok()
    });
    let (tool, call) = notify_call(&n, stdin.flatten());
    let b = backend(&n.route);
    match runtime().block_on(mcp::call_tool_as(&b, tool, &call, &hook_source(&n))) {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("rattleboard --notify: {e}");
            // A failing hook mustn't get in the agent's way.
            if n.hook {
                0
            } else {
                1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn notify_arguments() {
        let n = parse_notify(&args(&["Build passed", "--color", "green"])).unwrap();
        assert_eq!(n, Notify { text: "Build passed".into(), color: Some("green".into()), seconds: None, hook: false, payload: None, source: None, route: Route::default() });
        assert_eq!(hook_source(&n), "Agent via hook");
        let (tool, a) = notify_call(&n, None);
        assert_eq!(tool, "send_message");
        assert_eq!(Value::Object(a), json!({"text": "Build passed", "color": "green"}));

        let n = parse_notify(&args(&["Deploying", "--seconds", "30"])).unwrap();
        let (tool, a) = notify_call(&n, None);
        assert_eq!((tool, a["seconds"].as_u64()), ("show_status", Some(30)));

        // the agent app, for the board's history
        let n = parse_notify(&args(&["Done", "--source", "Claude Code", "--hook"])).unwrap();
        assert_eq!(hook_source(&n), "Claude Code via hook");
        assert!(parse_notify(&args(&["x", "--source"])).is_err());

        // words join; a hook's stdin names the project
        let n = parse_notify(&args(&["Claude", "Code", "is", "done", "--hook"])).unwrap();
        assert!(n.hook);
        let (_, a) = notify_call(&n, Some(json!({"cwd": "/Users/sam/src/rattleboard", "hook_event_name": "Stop"})));
        assert_eq!(a["text"], "Claude Code is done\nrattleboard");

        // Codex appends its JSON payload as the last argument
        let n = parse_notify(&args(&["Codex is done", "--color", "green", r#"{"type":"agent-turn-complete","cwd":"/work/api"}"#])).unwrap();
        assert_eq!(n.payload.as_ref().unwrap()["type"], "agent-turn-complete");
        assert_eq!(notify_call(&n, None).1["text"], "Codex is done\napi");

        // where it goes
        let n = parse_notify(&args(&["Done", "--target", "group:g1", "--url", "http://TV.local:7070/"])).unwrap();
        assert_eq!(n.route, Route { target: Some("group:g1".into()), url: Some("http://tv.local:7070".into()) });

        assert!(parse_notify(&args(&[])).is_err());
        assert!(parse_notify(&args(&["--color", "green"])).is_err());
        assert!(parse_notify(&args(&["x", "--color"])).is_err());
        assert!(parse_notify(&args(&["x", "--seconds", "1"])).is_err());
        assert!(parse_notify(&args(&["x", "--loud"])).is_err());
        assert!(parse_notify(&args(&["x", "--target"])).is_err());
        assert!(parse_notify(&args(&["x", "--seconds", "10", "--target", "tv"])).is_err());
        // text that merely starts with a brace is still text
        assert_eq!(parse_notify(&args(&["{63} Done"])).unwrap().text, "{63} Done");
    }

    #[test]
    fn key_kept() {
        // what the keychain gives each time: the key, nothing (refused after an update), a new key
        let reads = std::sync::Mutex::new(vec!["k2", "", "", "k1"]);
        let f = keeping_key(Box::new(move || ("http://x".into(), reads.lock().unwrap().pop().unwrap_or("").to_string())));
        // read once straight away, then kept while the keychain says nothing
        assert_eq!(f().1, "k1");
        assert_eq!(f().1, "k1");
        assert_eq!(f().1, "k2");
        assert_eq!(f().1, "k2");
    }

    #[cfg(unix)]
    #[test]
    fn replaced_exe() {
        let d = std::env::temp_dir().join(format!("rb-exe-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("rattleboard");
        std::fs::write(&p, "v1").unwrap();
        let me = Exe { path: Some(p.clone()), id: file_id(&p) };
        assert!(!me.replaced());
        // mid-update: gone, not replaced yet
        std::fs::rename(&p, d.join("old")).unwrap();
        assert!(!me.replaced());
        // a new file in its place, as brew upgrade or the updater leaves it
        std::fs::write(&p, "v2").unwrap();
        assert!(me.replaced());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn mcp_arguments() {
        assert_eq!(parse_mcp(&[]).unwrap(), Route::default());
        assert_eq!(parse_mcp(&args(&["--target", "Kitchen TV"])).unwrap().target.as_deref(), Some("Kitchen TV"));
        assert_eq!(parse_mcp(&args(&["--url", "https://board.example.com"])).unwrap().url.as_deref(), Some("https://board.example.com"));
        for bad in ["ftp://x", "tv.local:7070", "http://", "http://x/mcp", "http://u:p@x", "http://x?k=1", "file:///etc/passwd"] {
            assert!(parse_mcp(&args(&["--url", bad])).is_err(), "{bad}");
        }
        assert!(parse_mcp(&args(&["--url"])).is_err());
        assert!(parse_mcp(&args(&["--verbose"])).is_err());
    }

    #[test]
    fn remote_key_order() {
        let tv = SavedBoard { id: "tv1".into(), name: "Kitchen TV".into(), kind: "rattleboard".into(), host: "192.168.1.20".into(), rows: 6, cols: 22, adaptive: false };
        let real = SavedBoard { id: "vb".into(), name: "Hall".into(), kind: "local".into(), host: "192.168.1.30:7070".into(), ..tv.clone() };
        let saved = [tv, real];
        let store = |acct: &str| if acct == "board-tv1" { "tv-key".to_string() } else { String::new() };
        let env = || Some("env-key".to_string());
        // the saved board's key first (its port defaults to 7070)
        assert_eq!(remote_key("http://192.168.1.20:7070", &saved, &store, env()), ("tv-key".into(), KeyFrom::SavedBoard("Kitchen TV".into())));
        // another port, another host, or not a Rattleboard: the environment
        assert_eq!(remote_key("http://192.168.1.20:8080", &saved, &store, env()).1, KeyFrom::Env);
        assert_eq!(remote_key("http://192.168.1.30:7070", &saved, &store, env()).1, KeyFrom::Env);
        // a saved board without a key in the store falls through too
        let nokey = |_: &str| String::new();
        assert_eq!(remote_key("http://192.168.1.20:7070", &saved, &nokey, env()), ("env-key".into(), KeyFrom::Env));
        assert_eq!(remote_key("http://192.168.1.20:7070", &saved, &nokey, Some("  ".into())), (String::new(), KeyFrom::Nowhere));
    }
}
