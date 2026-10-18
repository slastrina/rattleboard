//! The command-line modes, decided before anything of the app starts (no window, tray or
//! single-instance check):
//!
//!   rattleboard --mcp                 an MCP server on stdio, for agent apps
//!   rattleboard --notify "<text>" [--color green] [--seconds 20] [--hook]
//!                                     one message, for agent apps' hooks
//!
//! Both talk to the running app's API server on this computer, with the key from the OS
//! credential store, so the key never has to be written into an agent's config. Neither
//! prints the key, and `--notify` prints nothing on stdout (some hooks read it).

use rattleboard_core::{mcp, secrets, settings::Settings};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

const IDENTIFIER: &str = "io.github.slastrina.rattleboard";

/// Runs a command-line mode if the arguments ask for one, returning its exit code.
pub fn main() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--mcp") => Some(run_mcp()),
        Some("--notify") => Some(run_notify(&args[1..])),
        _ => None,
    }
}

/// The app's config folder, as Tauri names it, and the profile's own in a debug build.
fn config_dir() -> PathBuf {
    crate::profile_dir(dirs::config_dir().unwrap_or_default().join(IDENTIFIER))
}

/// The local API server's address and key, read fresh each time.
fn locate() -> (String, String) {
    let cfg = config_dir();
    let s = Settings::load(&cfg.join("settings.json"));
    (format!("http://127.0.0.1:{}", s.server_port), secrets::get(secrets::SERVER_KEY))
}

fn backend() -> mcp::Http {
    if crate::profile().is_some() {
        secrets::use_file(config_dir().join("secrets.json"));
    }
    mcp::Http::locating(Box::new(locate))
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime")
}

fn run_mcp() -> i32 {
    let b = backend();
    let input = tokio::io::BufReader::new(tokio::io::stdin());
    match runtime().block_on(mcp::serve(&b, input, tokio::io::stdout())) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("rattleboard --mcp: {e}");
            1
        }
    }
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
}

pub const NOTIFY_USAGE: &str = "Usage: rattleboard --notify \"<text>\" [--color red|orange|yellow|green|blue|violet|white|black] [--seconds 3-3600] [--hook]";

/// `--notify`'s arguments (after `--notify`).
pub fn parse_notify(args: &[String]) -> Result<Notify, String> {
    let mut n = Notify { text: String::new(), color: None, seconds: None, hook: false, payload: None };
    let mut it = args.iter();
    let mut words: Vec<&str> = vec![];
    while let Some(a) = it.next() {
        match a.as_str() {
            "--color" | "--colour" => n.color = Some(it.next().ok_or("--color needs a colour")?.clone()),
            "--seconds" => {
                let s = it.next().ok_or("--seconds needs a number")?;
                n.seconds = Some(s.parse().ok().filter(|v| (3..=3600).contains(v)).ok_or("--seconds must be a whole number from 3 to 3600")?);
            }
            "--hook" => n.hook = true,
            s if s.starts_with('{') && serde_json::from_str::<Value>(s).is_ok_and(|v| v.is_object()) => n.payload = serde_json::from_str(s).ok(),
            s if s.starts_with("--") => return Err(format!("Unknown option {s}")),
            s => words.push(s),
        }
    }
    n.text = words.join(" ").trim().to_string();
    if n.text.is_empty() {
        return Err("Say what to show".into());
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
    let b = backend();
    match runtime().block_on(mcp::call_tool(&b, tool, &call)) {
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
        assert_eq!(n, Notify { text: "Build passed".into(), color: Some("green".into()), seconds: None, hook: false, payload: None });
        let (tool, a) = notify_call(&n, None);
        assert_eq!(tool, "send_message");
        assert_eq!(Value::Object(a), json!({"text": "Build passed", "color": "green"}));

        let n = parse_notify(&args(&["Deploying", "--seconds", "30"])).unwrap();
        let (tool, a) = notify_call(&n, None);
        assert_eq!((tool, a["seconds"].as_u64()), ("show_status", Some(30)));

        // words join; a hook's stdin names the project
        let n = parse_notify(&args(&["Claude", "Code", "is", "done", "--hook"])).unwrap();
        assert!(n.hook);
        let (_, a) = notify_call(&n, Some(json!({"cwd": "/Users/sam/src/rattleboard", "hook_event_name": "Stop"})));
        assert_eq!(a["text"], "Claude Code is done\nrattleboard");

        // Codex appends its JSON payload as the last argument
        let n = parse_notify(&args(&["Codex is done", "--color", "green", r#"{"type":"agent-turn-complete","cwd":"/work/api"}"#])).unwrap();
        assert_eq!(n.payload.as_ref().unwrap()["type"], "agent-turn-complete");
        assert_eq!(notify_call(&n, None).1["text"], "Codex is done\napi");

        assert!(parse_notify(&args(&[])).is_err());
        assert!(parse_notify(&args(&["--color", "green"])).is_err());
        assert!(parse_notify(&args(&["x", "--color"])).is_err());
        assert!(parse_notify(&args(&["x", "--seconds", "1"])).is_err());
        assert!(parse_notify(&args(&["x", "--loud"])).is_err());
        // text that merely starts with a brace is still text
        assert_eq!(parse_notify(&args(&["{63} Done"])).unwrap().text, "{63} Done");
    }
}
