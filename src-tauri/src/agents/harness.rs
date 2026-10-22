//! The agent apps Settings → Agents can connect, where each keeps its config, and the exact
//! edits connecting makes: the MCP server entry, the optional "finished / needs input" alerts
//! (hooks that run `rattleboard --notify`), and the Rattleboard skill where the app has skills.
//!
//! `plan` works out each file's text after a change without touching anything, so the
//! panel can show a preview first. Every edit is targeted (see `jsonc`; Codex's TOML goes
//! through toml_edit), so the rest of each file stays as it was, and disconnecting takes out
//! exactly what connecting put in, including any containers it had to create.
//!
//! Config formats, checked against each app's documentation (October 2026):
//! - Claude Code: ~/.claude.json "mcpServers" (user scope); hooks Stop and Notification in
//!   ~/.claude/settings.json; skills in ~/.claude/skills/<name>/SKILL.md.
//!   https://code.claude.com/docs/en/mcp, /hooks, /skills
//! - opencode: ~/.config/opencode/opencode.json(c) "mcp" {"type": "local", "command": [...]};
//!   alerts need a plugin (~/.config/opencode/plugins/); skills in ~/.config/opencode/skills.
//!   https://opencode.ai/docs/mcp-servers/, /plugins/, /skills/
//! - Claude Desktop: claude_desktop_config.json "mcpServers" (no hooks or skills).
//!   https://modelcontextprotocol.io/docs/develop/connect-local-servers
//! - Cursor: ~/.cursor/mcp.json "mcpServers"; ~/.cursor/hooks.json "stop"; ~/.cursor/skills.
//!   https://cursor.com/docs/context/mcp, /agent/hooks, /context/skills
//! - Codex CLI: $CODEX_HOME/config.toml [mcp_servers.<name>]; top-level `notify` (argv, with
//!   a JSON payload appended); skills in ~/.agents/skills. https://learn.chatgpt.com/docs/config-file/config-reference
//! - Gemini CLI: ~/.gemini/settings.json "mcpServers"; hooks AfterAgent and Notification;
//!   skills in ~/.gemini/skills. https://geminicli.com/docs/tools/mcp-server/, /hooks/, /cli/skills/
//! - VS Code: <user dir>/mcp.json "servers" {"type": "stdio"}; hooks (preview) in
//!   ~/.copilot/hooks/*.json; skills in ~/.copilot/skills.
//!   https://code.visualstudio.com/docs/copilot/reference/mcp-configuration, /customization/hooks

use super::jsonc::{self, path, J};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SERVER: &str = "rattleboard";
pub const SKILL: &str = include_str!("../../../skills/rattleboard/SKILL.md");

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

/// Where things are on this computer. Tests point it at a temporary folder.
#[derive(Clone, Debug)]
pub struct Env {
    pub os: Os,
    pub home: PathBuf,
    /// %APPDATA% (Windows)
    pub appdata: Option<PathBuf>,
    /// $XDG_CONFIG_HOME
    pub xdg_config: Option<PathBuf>,
    pub codex_home: Option<PathBuf>,
    pub claude_config_dir: Option<PathBuf>,
}

fn env_path(k: &str) -> Option<PathBuf> {
    std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from)
}

impl Env {
    pub fn system() -> Env {
        let os = if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        };
        Env {
            os,
            home: env_path(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).or_else(dirs::home_dir).unwrap_or_default(),
            appdata: env_path("APPDATA").or_else(|| if cfg!(windows) { dirs::config_dir() } else { None }),
            xdg_config: env_path("XDG_CONFIG_HOME"),
            codex_home: env_path("CODEX_HOME"),
            claude_config_dir: env_path("CLAUDE_CONFIG_DIR"),
        }
    }

    /// ~/.config, or $XDG_CONFIG_HOME (opencode uses it on every platform)
    fn dot_config(&self) -> PathBuf {
        self.xdg_config.clone().unwrap_or_else(|| self.home.join(".config"))
    }

    /// Where desktop apps keep their settings: Application Support, %APPDATA% or ~/.config
    fn app_support(&self) -> PathBuf {
        match self.os {
            Os::Mac => self.home.join("Library/Application Support"),
            Os::Windows => self.appdata.clone().unwrap_or_else(|| self.home.join("AppData/Roaming")),
            Os::Linux => self.dot_config(),
        }
    }

    fn claude_dir(&self) -> PathBuf {
        self.claude_config_dir.clone().unwrap_or_else(|| self.home.join(".claude"))
    }

    fn codex_dir(&self) -> PathBuf {
        self.codex_home.clone().unwrap_or_else(|| self.home.join(".codex"))
    }
}

/// How agent apps start Rattleboard: this executable, in a debug build its profile, and
/// the saved board or group messages go to by default (`--target`).
#[derive(Clone, Debug, Default)]
pub struct Launch {
    pub exe: String,
    pub profile: Option<String>,
    pub target: Option<String>,
}

impl Launch {
    /// `--mcp`, and `--target <t>` when there's a default target
    fn mcp_args(&self) -> Vec<String> {
        let mut a = vec!["--mcp".to_string()];
        if let Some(t) = &self.target {
            a.extend(["--target".to_string(), t.clone()]);
        }
        a
    }

    fn env(&self) -> Option<J> {
        self.profile.as_ref().map(|p| J::o(vec![("RATTLEBOARD_PROFILE", J::s(p.clone()))]))
    }

    /// A shell command that runs `--notify`, for hooks run through a shell. `source` is the
    /// agent app, for the board's history ("Claude Code via hook").
    fn notify_cmd(&self, os: Os, source: &str, text: &str, color: &str) -> String {
        let q = |s: &str| {
            if os == Os::Windows {
                format!("\"{}\"", s.replace('"', ""))
            } else {
                format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$").replace('`', "\\`"))
            }
        };
        let env = match (&self.profile, os) {
            (Some(p), Os::Mac | Os::Linux) => format!("RATTLEBOARD_PROFILE={} ", q(p)),
            _ => String::new(),
        };
        let target = self.target.as_ref().map(|t| format!(" --target {}", q(t))).unwrap_or_default();
        format!("{env}{} --notify {} --color {color} --source {} --hook{target}", q(&self.exe), q(text), q(source))
    }
}

/// An item that's one of ours: it runs this app with --notify.
pub fn is_ours(v: &Value) -> bool {
    let s = v.to_string().to_lowercase();
    s.contains("--notify") && s.contains("rattleboard")
}

#[derive(Clone, Debug)]
enum Piece {
    /// Object member `key` of the object at `at`
    Member { at: Vec<String>, key: String, value: J },
    /// Object member added only when missing and never removed (Cursor's hooks "version")
    Skeleton { key: String, value: J },
    /// An item of the array at `at`, recognised by `is_ours`
    Item { at: Vec<String>, value: J },
    /// [mcp_servers.rattleboard] in Codex's config.toml
    TomlServer { argv: Vec<String>, env: Option<(String, String)> },
    /// Codex's top-level notify program
    TomlNotify { argv: Vec<String> },
    /// A file of our own (a skill, a plugin)
    Own { contents: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Part {
    Mcp,
    Alerts,
    Skill,
}

pub struct Harness {
    pub id: &'static str,
    pub name: &'static str,
    /// Any of these existing means the app is installed
    detect: Vec<PathBuf>,
    pub mcp_file: PathBuf,
    pieces: Vec<(Part, PathBuf, Piece)>,
    pub note: &'static str,
}

impl Harness {
    pub fn has(&self, part: Part) -> bool {
        self.pieces.iter().any(|p| p.0 == part)
    }
    pub fn installed(&self) -> bool {
        self.detect.iter().any(|p| p.exists())
    }
    pub fn files(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = vec![];
        for (_, f, _) in &self.pieces {
            if !v.contains(f) {
                v.push(f.clone());
            }
        }
        v
    }
}

fn stdio_server(l: &Launch, with_type: bool) -> J {
    let mut o = vec![];
    if with_type {
        o.push(("type".to_string(), J::s("stdio")));
    }
    o.push(("command".to_string(), J::s(l.exe.clone())));
    o.push(("args".to_string(), J::A(l.mcp_args().into_iter().map(J::S).collect())));
    if let Some(e) = l.env() {
        o.push(("env".to_string(), e));
    }
    J::O(o)
}

fn skill(dir: PathBuf) -> (Part, PathBuf, Piece) {
    (Part::Skill, dir.join("rattleboard").join("SKILL.md"), Piece::Own { contents: SKILL.replace("\r\n", "\n") })
}

/// Brings the Rattleboard skill installed by an earlier version up to date with this one.
/// Only an existing SKILL.md that is ours (name: rattleboard) and differs is rewritten;
/// nothing is created, so a skill the user disconnected stays gone. Returns what changed.
pub fn refresh_skills(hs: &[Harness]) -> Vec<PathBuf> {
    let mut done = vec![];
    for (_, f, piece) in hs.iter().flat_map(|h| h.pieces.iter().filter(|p| p.0 == Part::Skill)) {
        let Piece::Own { contents } = piece else { continue };
        let Ok(cur) = std::fs::read_to_string(f) else { continue };
        if cur.replace("\r\n", "\n").starts_with("---\nname: rattleboard\n") && cur != *contents && !done.contains(f) && std::fs::write(f, contents).is_ok() {
            done.push(f.clone());
        }
    }
    done
}

/// opencode's alerts: a plugin that runs --notify when a session goes idle or asks permission.
fn opencode_plugin(l: &Launch) -> String {
    let env = match &l.profile {
        Some(p) => serde_json::json!({ "RATTLEBOARD_PROFILE": p }),
        None => serde_json::json!({}),
    };
    let more: Vec<String> = l.target.iter().flat_map(|t| ["--target".to_string(), t.clone()]).collect();
    format!(
        r#"// Added by Rattleboard (Settings → Agents → opencode); disconnecting there removes it.
// Puts a short card on your Rattleboard when opencode is done or needs you.
import {{ spawn }} from "node:child_process";
import {{ basename }} from "node:path";

const EXE = {exe};
const ENV = {env};
const MORE = {more};

function tell(text, color) {{
  try {{
    spawn(EXE, ["--notify", text, "--color", color, "--source", "opencode", ...MORE], {{ stdio: "ignore", detached: true, env: {{ ...process.env, ...ENV }} }}).unref();
  }} catch {{}}
}}

export const Rattleboard = async ({{ directory }}) => {{
  const project = directory ? "\n" + basename(directory) : "";
  return {{
    event: async ({{ event }}) => {{
      if (event.type === "session.idle") tell("opencode is done" + project, "green");
      else if (event.type === "permission.asked" || event.type === "permission.updated") tell("opencode needs you" + project, "yellow");
    }},
  }};
}};
"#,
        exe = Value::String(l.exe.clone()),
        env = env,
        more = serde_json::to_string(&more).unwrap()
    )
}

fn hook(cmd: String) -> J {
    J::o(vec![("hooks", J::A(vec![J::o(vec![("type", J::s("command")), ("command", J::s(cmd))])]))])
}

/// Every harness, with its files and edits for this computer.
pub fn all(env: &Env, l: &Launch) -> Vec<Harness> {
    let h = &env.home;
    let os = env.os;
    let claude_settings = env.claude_dir().join("settings.json");
    let claude_json = match &env.claude_config_dir {
        Some(d) => d.join(".claude.json"),
        None => h.join(".claude.json"),
    };
    let oc_dir = env.dot_config().join("opencode");
    let oc_file = if oc_dir.join("opencode.jsonc").exists() && !oc_dir.join("opencode.json").exists() { oc_dir.join("opencode.jsonc") } else { oc_dir.join("opencode.json") };
    let desktop_dir = env.app_support().join("Claude");
    let codex = env.codex_dir();
    let gemini = h.join(".gemini");
    let code_user = env.app_support().join("Code").join("User");
    let cursor = h.join(".cursor");
    let copilot = h.join(".copilot");
    let mcp = |at: &str, value: J| Piece::Member { at: path(&[at]), key: SERVER.into(), value };
    let mut codex_argv = vec![l.exe.clone(), "--notify".into(), "Codex is done".into(), "--color".into(), "green".into(), "--source".into(), "Codex".into()];
    if let Some(t) = &l.target {
        codex_argv.extend(["--target".to_string(), t.clone()]);
    }
    if let (Some(p), Os::Mac | Os::Linux) = (&l.profile, os) {
        codex_argv.splice(0..0, ["env".to_string(), format!("RATTLEBOARD_PROFILE={p}")]);
    }
    vec![
        Harness {
            id: "claude-code",
            name: "Claude Code",
            detect: vec![claude_json.clone(), env.claude_dir()],
            mcp_file: claude_json.clone(),
            pieces: vec![
                (Part::Mcp, claude_json, mcp("mcpServers", stdio_server(l, true))),
                (Part::Alerts, claude_settings.clone(), Piece::Item { at: path(&["hooks", "Stop"]), value: hook(l.notify_cmd(os, "Claude Code", "Claude Code is done", "green")) }),
                (
                    Part::Alerts,
                    claude_settings,
                    Piece::Item {
                        at: path(&["hooks", "Notification"]),
                        value: J::o(vec![
                            ("matcher", J::s("permission_prompt|idle_prompt|elicitation_dialog")),
                            ("hooks", J::A(vec![J::o(vec![("type", J::s("command")), ("command", J::s(l.notify_cmd(os, "Claude Code", "Claude Code needs you", "yellow")))])])),
                        ]),
                    },
                ),
                skill(env.claude_dir().join("skills")),
            ],
            note: "Start a new Claude Code session to pick it up.",
        },
        Harness {
            id: "opencode",
            name: "opencode",
            detect: vec![oc_dir.clone(), h.join(".opencode")],
            mcp_file: oc_file.clone(),
            pieces: vec![
                (Part::Mcp, oc_file, {
                    let mut o = vec![("type", J::s("local")), ("command", J::A(std::iter::once(l.exe.clone()).chain(l.mcp_args()).map(J::S).collect())), ("enabled", J::B(true))];
                    if let Some(e) = l.env() {
                        o.push(("environment", e));
                    }
                    mcp("mcp", J::o(o))
                }),
                (Part::Alerts, oc_dir.join("plugins").join("rattleboard.js"), Piece::Own { contents: opencode_plugin(l) }),
                skill(oc_dir.join("skills")),
            ],
            note: "Restart opencode to pick it up.",
        },
        Harness {
            id: "claude-desktop",
            name: "Claude Desktop",
            detect: vec![desktop_dir.clone()],
            mcp_file: desktop_dir.join("claude_desktop_config.json"),
            pieces: vec![(Part::Mcp, desktop_dir.join("claude_desktop_config.json"), mcp("mcpServers", stdio_server(l, false)))],
            note: "Quit and reopen Claude Desktop to pick it up.",
        },
        Harness {
            id: "cursor",
            name: "Cursor",
            detect: vec![cursor.clone()],
            mcp_file: cursor.join("mcp.json"),
            pieces: vec![
                (Part::Mcp, cursor.join("mcp.json"), mcp("mcpServers", stdio_server(l, true))),
                (Part::Alerts, cursor.join("hooks.json"), Piece::Skeleton { key: "version".into(), value: J::N(1) }),
                (
                    Part::Alerts,
                    cursor.join("hooks.json"),
                    Piece::Item { at: path(&["hooks", "stop"]), value: J::o(vec![("command", J::s(l.notify_cmd(os, "Cursor", "Cursor is done", "green")))]) },
                ),
                skill(cursor.join("skills")),
            ],
            note: "Cursor picks it up in a new chat; check Settings → MCP if it doesn't.",
        },
        Harness {
            id: "codex",
            name: "Codex CLI",
            detect: vec![codex.clone()],
            mcp_file: codex.join("config.toml"),
            pieces: vec![
                (Part::Mcp, codex.join("config.toml"), Piece::TomlServer {
                    argv: std::iter::once(l.exe.clone()).chain(l.mcp_args()).collect(),
                    env: l.profile.clone().map(|p| ("RATTLEBOARD_PROFILE".to_string(), p)),
                }),
                (Part::Alerts, codex.join("config.toml"), Piece::TomlNotify { argv: codex_argv }),
                skill(h.join(".agents").join("skills")),
            ],
            note: "Start a new Codex session to pick it up.",
        },
        Harness {
            id: "gemini",
            name: "Gemini CLI",
            detect: vec![gemini.clone()],
            mcp_file: gemini.join("settings.json"),
            pieces: vec![
                (Part::Mcp, gemini.join("settings.json"), mcp("mcpServers", stdio_server(l, false))),
                (Part::Alerts, gemini.join("settings.json"), Piece::Item { at: path(&["hooks", "AfterAgent"]), value: gemini_hook(l.notify_cmd(os, "Gemini CLI", "Gemini is done", "green")) }),
                (Part::Alerts, gemini.join("settings.json"), Piece::Item { at: path(&["hooks", "Notification"]), value: gemini_hook(l.notify_cmd(os, "Gemini CLI", "Gemini needs you", "yellow")) }),
                skill(gemini.join("skills")),
            ],
            note: "Restart Gemini CLI to pick it up.",
        },
        Harness {
            id: "vscode",
            name: "VS Code",
            detect: vec![code_user.clone()],
            mcp_file: code_user.join("mcp.json"),
            pieces: vec![
                (Part::Mcp, code_user.join("mcp.json"), mcp("servers", stdio_server(l, true))),
                (Part::Alerts, copilot.join("hooks").join("rattleboard.json"), Piece::Own {
                    contents: format!(
                        "{}\n",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "hooks": { "Stop": [{ "type": "command", "command": l.notify_cmd(os, "Copilot", "Copilot is done", "green"), "timeout": 15 }] }
                        }))
                        .unwrap()
                    ),
                }),
                skill(copilot.join("skills")),
            ],
            note: "VS Code asks to start the server the first time; Copilot's hooks are a preview feature.",
        },
    ]
}

fn gemini_hook(cmd: String) -> J {
    J::o(vec![("hooks", J::A(vec![J::o(vec![("name", J::s("rattleboard")), ("type", J::s("command")), ("command", J::s(cmd))])]))])
}

// ---------------------------------------------------------------- planning

/// What this app created in a file, so disconnecting can take exactly that out again.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileState {
    #[serde(default)]
    pub created_file: bool,
    #[serde(default)]
    pub containers: Vec<Vec<String>>,
}

pub type State = BTreeMap<String, FileState>;

/// One file's change: its text before and after (None: no file).
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub path: PathBuf,
    pub before: Option<String>,
    pub after: Option<String>,
    pub state: FileState,
    /// One of our own files (a skill or plugin): no backup needed
    pub own: bool,
}

/// Which parts should be in place.
#[derive(Clone, Copy, Debug)]
pub struct Want {
    pub mcp: bool,
    pub alerts: bool,
}

impl Want {
    fn part(&self, p: Part) -> bool {
        match p {
            Part::Mcp | Part::Skill => self.mcp,
            Part::Alerts => self.alerts,
        }
    }
}

pub fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

fn toml_doc(text: &str, file: &Path) -> Result<toml_edit::DocumentMut, String> {
    text.parse::<toml_edit::DocumentMut>().map_err(|e| format!("{} isn't valid TOML ({}); fix it, or add Rattleboard by hand", file.display(), e.message()))
}

fn toml_array(argv: &[String]) -> toml_edit::Array {
    argv.iter().map(|s| s.as_str()).collect()
}

fn toml_notify_ours(doc: &toml_edit::DocumentMut) -> Option<bool> {
    let n = doc.get("notify")?;
    let s = n.to_string().to_lowercase();
    Some(s.contains("--notify") && s.contains("rattleboard"))
}

/// Codex's notify slot is taken by another program, so alerts can't go there.
pub fn codex_notify_taken(file: &Path) -> bool {
    read(file).and_then(|t| toml_doc(&t, file).ok()).and_then(|d| toml_notify_ours(&d)) == Some(false)
}

fn apply_toml(text: &str, file: &Path, piece: &Piece, want: bool, st: &mut FileState) -> Result<String, String> {
    let mut doc = toml_doc(text, file)?;
    match piece {
        Piece::TomlServer { argv, env } => {
            let mut t = toml_edit::Table::new();
            t.insert("command", toml_edit::value(argv[0].as_str()));
            t.insert("args", toml_edit::value(toml_array(&argv[1..])));
            if let Some((k, v)) = env {
                let mut e = toml_edit::InlineTable::new();
                e.insert(k, v.as_str().into());
                t.insert("env", toml_edit::value(e));
            }
            let same = |cur: &toml_edit::Item| cur.to_string().trim() == toml_edit::Item::Table(t.clone()).to_string().trim();
            if want {
                if doc.get("mcp_servers").is_none() {
                    let mut servers = toml_edit::Table::new();
                    servers.set_implicit(true);
                    doc.insert("mcp_servers", toml_edit::Item::Table(servers));
                    st.containers.push(vec!["mcp_servers".into()]);
                }
                let servers = doc["mcp_servers"].as_table_mut().ok_or("mcp_servers in config.toml isn't a table")?;
                if servers.get(SERVER).is_some_and(same) {
                    return Ok(text.to_string());
                }
                servers.insert(SERVER, toml_edit::Item::Table(t));
            } else {
                let Some(servers) = doc.get_mut("mcp_servers").and_then(|s| s.as_table_mut()) else { return Ok(text.to_string()) };
                if servers.remove(SERVER).is_none() {
                    return Ok(text.to_string());
                }
                if servers.is_empty() && st.containers.contains(&vec!["mcp_servers".to_string()]) {
                    doc.remove("mcp_servers");
                }
            }
        }
        Piece::TomlNotify { argv } => {
            let ours = toml_notify_ours(&doc);
            if want {
                if ours == Some(false) {
                    return Err("Codex already runs another notify program, so Rattleboard can't add its alert there".into());
                }
                let new = toml_edit::value(toml_array(argv));
                if doc.get("notify").is_some_and(|n| n.to_string().trim() == new.to_string().trim()) {
                    return Ok(text.to_string());
                }
                // A top-level key must come before the first [table].
                doc.insert("notify", new);
            } else if ours == Some(true) {
                doc.remove("notify");
            } else {
                return Ok(text.to_string());
            }
        }
        _ => unreachable!(),
    }
    Ok(doc.to_string())
}

/// The changes that bring `h`'s files to `want`. Files that wouldn't change are left out.
pub fn plan(h: &Harness, want: Want, state: &State) -> Result<Vec<Change>, String> {
    let mut out = vec![];
    for file in h.files() {
        let key = file.to_string_lossy().to_string();
        let before = read(&file);
        let mut st = state.get(&key).cloned().unwrap_or_default();
        let pieces: Vec<&(Part, PathBuf, Piece)> = h.pieces.iter().filter(|p| p.1 == file).collect();
        // Own files: there, or not.
        if let Some((part, _, Piece::Own { contents })) = pieces.first().copied().filter(|p| matches!(p.2, Piece::Own { .. })) {
            let after = want.part(*part).then(|| contents.clone());
            if after != before && !(after.is_none() && before.is_none()) {
                out.push(Change { path: file.clone(), before, after, state: FileState::default(), own: true });
            }
            continue;
        }
        let mut text = before.clone().unwrap_or_default();
        let toml = file.extension().is_some_and(|e| e == "toml");
        for (part, _, piece) in &pieces {
            let w = want.part(*part);
            let ctx = |e: String| format!("{}: {e}", file.display());
            text = match piece {
                Piece::TomlServer { .. } | Piece::TomlNotify { .. } => apply_toml(&text, &file, piece, w, &mut st)?,
                Piece::Member { at, key, value } => {
                    if w {
                        let (t, created) = jsonc::set_member(&text, at, key, value).map_err(ctx)?;
                        st.containers.extend(created);
                        t
                    } else {
                        jsonc::remove_member(&text, at, key, &st.containers).map_err(ctx)?
                    }
                }
                Piece::Item { at, value } => {
                    if w {
                        let (t, created) = jsonc::add_item(&text, at, value, &is_ours).map_err(ctx)?;
                        st.containers.extend(created);
                        t
                    } else {
                        jsonc::remove_items(&text, at, &is_ours, &st.containers).map_err(ctx)?
                    }
                }
                Piece::Skeleton { key, value } => {
                    let has = jsonc::parse(if text.trim().is_empty() { "{}" } else { &text }).map(|n| jsonc::value_of(&text, &n).get(key).is_some()).unwrap_or(false);
                    if w && !has {
                        jsonc::set_member(&text, &[], key, value).map_err(ctx)?.0
                    } else {
                        text
                    }
                }
                Piece::Own { .. } => unreachable!(),
            };
        }
        let mut seen = vec![];
        st.containers.retain(|c| if seen.contains(c) { false } else { seen.push(c.clone()); true });
        let any_wanted = pieces.iter().any(|p| want.part(p.0) && !matches!(p.2, Piece::Skeleton { .. }));
        if before.is_none() {
            st.created_file = true;
        }
        // A file this app created goes again once nothing of the user's is in it.
        let leftover_only_skeleton = || {
            let skel: Vec<(&String, &J)> = pieces.iter().filter_map(|p| if let Piece::Skeleton { key, value } = &p.2 { Some((key, value)) } else { None }).collect();
            let Ok(Value::Object(m)) = jsonc::parse(&text).map(|n| jsonc::value_of(&text, &n)) else { return false };
            m.iter().all(|(k, v)| skel.iter().any(|(sk, sv)| *sk == k && sv.to_value() == *v))
        };
        let gone = text.trim().is_empty() || (!toml && leftover_only_skeleton());
        let after = if !any_wanted && (before.is_none() || (st.created_file && gone)) { None } else { Some(text) };
        if after != before {
            if !toml {
                if let Some(a) = &after {
                    jsonc::parse(a).map_err(|e| format!("{}: the edit didn't come out as valid JSON ({e}); nothing was changed", file.display()))?;
                }
            }
            let state = if after.is_none() { FileState::default() } else { st };
            out.push(Change { path: file, before, after, state, own: false });
        }
    }
    Ok(out)
}

/// The `--target` the MCP entry passes now, if any.
pub fn current_target(h: &Harness) -> Option<String> {
    let text = read(&h.mcp_file)?;
    let args: Vec<String> = h.pieces.iter().find_map(|(part, file, piece)| {
        if *part != Part::Mcp || *file != h.mcp_file {
            return None;
        }
        let strs = |v: &Value| v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect::<Vec<_>>());
        match piece {
            Piece::Member { at, key, .. } => {
                let v = jsonc::value_of(&text, &jsonc::parse(&text).ok()?);
                let e = at.iter().try_fold(&v, |v, k| v.get(k))?.get(key)?.clone();
                strs(e.get("args").unwrap_or(&Value::Null)).or_else(|| strs(e.get("command")?))
            }
            Piece::TomlServer { .. } => {
                let d = toml_doc(&text, file).ok()?;
                let a = d.get("mcp_servers")?.get(SERVER)?.get("args")?.as_array()?;
                Some(a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
            }
            _ => None,
        }
    })?;
    let i = args.iter().position(|a| a == "--target")?;
    args.get(i + 1).cloned()
}

/// Whether each part is in place now.
pub fn status(h: &Harness) -> (bool, bool) {
    let mut mcp = false;
    let mut alerts = false;
    for (part, file, piece) in &h.pieces {
        let Some(text) = read(file) else { continue };
        let here = match piece {
            Piece::Member { at, key, .. } => jsonc::parse(&text).ok().map(|n| jsonc::value_of(&text, &n)).is_some_and(|v| {
                at.iter().try_fold(&v, |v, k| v.get(k)).and_then(|o| o.get(key)).is_some()
            }),
            Piece::Item { at, .. } => jsonc::parse(&text).ok().map(|n| jsonc::value_of(&text, &n)).is_some_and(|v| {
                at.iter().try_fold(&v, |v, k| v.get(k)).and_then(|a| a.as_array()).is_some_and(|a| a.iter().any(is_ours))
            }),
            Piece::TomlServer { .. } => toml_doc(&text, file).ok().is_some_and(|d| d.get("mcp_servers").and_then(|s| s.get(SERVER)).is_some()),
            Piece::TomlNotify { .. } => toml_doc(&text, file).ok().and_then(|d| toml_notify_ours(&d)) == Some(true),
            Piece::Own { .. } => true,
            Piece::Skeleton { .. } => false,
        };
        match part {
            Part::Mcp => mcp |= here,
            Part::Alerts => alerts |= here,
            Part::Skill => {}
        }
    }
    (mcp, alerts)
}

/// A unified-style diff of two texts, for the preview: lines with "+", "-" or " ", around
/// each change with a little context.
pub fn diff(before: &str, after: &str) -> Vec<String> {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let mut ops: Vec<(char, &str)> = vec![];
    if am.len() * bm.len() <= 4_000_000 {
        // Longest common subsequence over the part that differs
        let (n, m) = (am.len(), bm.len());
        let mut t = vec![vec![0u32; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                t[i][j] = if am[i] == bm[j] { t[i + 1][j + 1] + 1 } else { t[i + 1][j].max(t[i][j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && am[i] == bm[j] {
                ops.push((' ', am[i]));
                i += 1;
                j += 1;
            } else if j < m && (i == n || t[i][j + 1] >= t[i + 1][j]) {
                ops.push(('+', bm[j]));
                j += 1;
            } else {
                ops.push(('-', am[i]));
                i += 1;
            }
        }
    } else {
        ops.extend(am.iter().map(|l| ('-', *l)));
        ops.extend(bm.iter().map(|l| ('+', *l)));
    }
    let all: Vec<(char, &str)> = a[..pre].iter().map(|l| (' ', *l)).chain(ops).chain(a[a.len() - suf..].iter().map(|l| (' ', *l))).collect();
    const CTX: usize = 3;
    let near = |i: usize| all[i.saturating_sub(CTX)..(i + CTX + 1).min(all.len())].iter().any(|o| o.0 != ' ');
    let mut out = vec![];
    let mut gap = false;
    for (i, (c, l)) in all.iter().enumerate() {
        if near(i) {
            if gap && !out.is_empty() {
                out.push("…".to_string());
            }
            gap = false;
            out.push(format!("{c} {l}"));
        } else {
            gap = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tmp(name: &str) -> (Tmp, Env) {
        let d = std::env::temp_dir().join(format!("rb-agents-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let env = Env { os: Os::Mac, home: d.clone(), appdata: None, xdg_config: None, codex_home: None, claude_config_dir: None };
        (Tmp(d), env)
    }

    fn launch() -> Launch {
        Launch { exe: "/Applications/Rattleboard.app/Contents/MacOS/rattleboard".into(), ..Default::default() }
    }

    fn write(c: &[Change]) {
        for ch in c {
            match &ch.after {
                Some(t) => {
                    std::fs::create_dir_all(ch.path.parent().unwrap()).unwrap();
                    std::fs::write(&ch.path, t).unwrap();
                }
                None => {
                    let _ = std::fs::remove_file(&ch.path);
                }
            }
        }
    }

    fn states(c: &[Change], st: &mut State) {
        for ch in c {
            st.insert(ch.path.to_string_lossy().to_string(), ch.state.clone());
        }
    }

    /// Connect with alerts, connect again (nothing changes), disconnect: every file is as it
    /// was, and files that didn't exist are gone again.
    fn cycle(env: &Env, id: &str, fixtures: &[(&str, &str)]) -> Vec<(PathBuf, String)> {
        for (rel, text) in fixtures {
            let p = env.home.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, text).unwrap();
        }
        let hs = all(env, &launch());
        let h = hs.iter().find(|h| h.id == id).unwrap();
        assert!(h.installed() || fixtures.is_empty(), "{id} detected");
        let mut st = State::new();
        let on = Want { mcp: true, alerts: true };
        let c = plan(h, on, &st).unwrap();
        assert!(!c.is_empty());
        write(&c);
        states(&c, &mut st);
        let connected: Vec<(PathBuf, String)> = c.iter().filter_map(|ch| ch.after.clone().map(|a| (ch.path.clone(), a))).collect();
        assert_eq!(status(h), (true, h.has(Part::Alerts)), "{id} status");
        assert!(plan(h, on, &st).unwrap().is_empty(), "{id}: connecting twice changes nothing");
        // alerts off again, then everything
        let c = plan(h, Want { mcp: true, alerts: false }, &st).unwrap();
        write(&c);
        states(&c, &mut st);
        assert!(!status(h).1);
        let c = plan(h, Want { mcp: false, alerts: false }, &st).unwrap();
        write(&c);
        states(&c, &mut st);
        assert_eq!(status(h), (false, false));
        for (rel, text) in fixtures {
            assert_eq!(std::fs::read_to_string(env.home.join(rel)).unwrap(), *text, "{id}: {rel} restored");
        }
        for f in h.files() {
            if !fixtures.iter().any(|(rel, _)| env.home.join(rel) == f) {
                assert!(!f.exists(), "{id}: {} removed", f.display());
            }
        }
        connected
    }

    #[test]
    fn claude_code() {
        let (_t, env) = tmp("claude");
        let claude_json = "{\n  \"numStartups\": 12,\n  \"mcpServers\": {\n    \"github\": {\n      \"type\": \"http\",\n      \"url\": \"https://example.com/mcp\"\n    }\n  },\n  \"projects\": {}\n}\n";
        let settings = "{\n  \"model\": \"opus\",\n  \"hooks\": {\n    \"Stop\": [\n      {\n        \"hooks\": [\n          {\n            \"type\": \"command\",\n            \"command\": \"afplay /System/Library/Sounds/Glass.aiff\"\n          }\n        ]\n      }\n    ]\n  }\n}\n";
        let after = cycle(&env, "claude-code", &[(".claude.json", claude_json), (".claude/settings.json", settings)]);
        let get = |name: &str| after.iter().find(|(p, _)| p.ends_with(name)).unwrap().1.clone();
        let cj: Value = serde_json::from_str(&get(".claude.json")).unwrap();
        assert_eq!(cj["mcpServers"]["rattleboard"], serde_json::json!({"type": "stdio", "command": launch().exe, "args": ["--mcp"]}));
        assert_eq!(cj["mcpServers"]["github"]["type"], "http");
        let s: Value = serde_json::from_str(&get("settings.json")).unwrap();
        assert_eq!(s["hooks"]["Stop"].as_array().unwrap().len(), 2);
        let cmd = s["hooks"]["Stop"][1]["hooks"][0]["command"].as_str().unwrap();
        assert_eq!(cmd, "\"/Applications/Rattleboard.app/Contents/MacOS/rattleboard\" --notify \"Claude Code is done\" --color green --source \"Claude Code\" --hook");
        assert_eq!(s["hooks"]["Notification"][0]["matcher"], "permission_prompt|idle_prompt|elicitation_dialog");
        assert!(get("SKILL.md").starts_with("---\nname: rattleboard\n"));
        // the key never goes in a config file
        for (_, text) in &after {
            assert!(!text.to_lowercase().contains("api-server-key") && !text.contains("Bearer"));
        }
    }

    #[test]
    fn claude_code_fresh() {
        let (_t, env) = tmp("claude-fresh");
        std::fs::create_dir_all(env.home.join(".claude")).unwrap();
        cycle(&env, "claude-code", &[]);
    }

    #[test]
    fn opencode_jsonc() {
        let (_t, env) = tmp("opencode");
        let cfg = "{\n  // my config\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"theme\": \"tokyonight\",\n  \"mcp\": {\n    \"jira\": { \"type\": \"remote\", \"url\": \"https://jira.example.com/mcp\", }, // work\n  },\n}\n";
        let after = cycle(&env, "opencode", &[(".config/opencode/opencode.jsonc", cfg)]);
        let text = &after.iter().find(|(p, _)| p.ends_with("opencode.jsonc")).unwrap().1;
        assert!(text.contains("// my config") && text.contains("}, // work"), "{text}");
        let v = jsonc::value_of(text, &jsonc::parse(text).unwrap());
        assert_eq!(v["mcp"]["rattleboard"]["command"], serde_json::json!([launch().exe, "--mcp"]));
        assert_eq!(v["mcp"]["rattleboard"]["type"], "local");
        let plugin = &after.iter().find(|(p, _)| p.ends_with("rattleboard.js")).unwrap().1;
        assert!(plugin.contains("session.idle") && plugin.contains("\"/Applications/Rattleboard.app/Contents/MacOS/rattleboard\""));
    }

    #[test]
    fn desktop_cursor_gemini_vscode() {
        let (_t, env) = tmp("others");
        cycle(&env, "claude-desktop", &[("Library/Application Support/Claude/claude_desktop_config.json", "{\n  \"mcpServers\": {}\n}")]);
        cycle(&env, "cursor", &[(".cursor/mcp.json", "{\"mcpServers\": {\"x\": {\"command\": \"y\"}}}\n")]);
        // Cursor's hooks.json of the user's own keeps its version and hooks
        let (_t2, env2) = tmp("cursor-hooks");
        cycle(&env2, "cursor", &[(".cursor/hooks.json", "{\n  \"version\": 1,\n  \"hooks\": {\n    \"afterFileEdit\": [{ \"command\": \"./format.sh\" }]\n  }\n}\n")]);
        let after = cycle(&env, "gemini", &[(".gemini/settings.json", "{\n  // comments are allowed here\n  \"theme\": \"GitHub\"\n}\n")]);
        let v = {
            let t = &after.iter().find(|(p, _)| p.ends_with("settings.json")).unwrap().1;
            jsonc::value_of(t, &jsonc::parse(t).unwrap())
        };
        assert_eq!(v["hooks"]["AfterAgent"][0]["hooks"][0]["name"], "rattleboard");
        assert_eq!(v["mcpServers"]["rattleboard"]["args"], serde_json::json!(["--mcp"]));
        let vs = "// User MCP servers\n{\n\t\"servers\": {\n\t\t\"fetch\": {\"type\": \"stdio\", \"command\": \"uvx\", \"args\": [\"mcp-server-fetch\"]},\n\t},\n\t\"inputs\": []\n}\n";
        let after = cycle(&env, "vscode", &[("Library/Application Support/Code/User/mcp.json", vs)]);
        let t = &after.iter().find(|(p, _)| p.ends_with("mcp.json")).unwrap().1;
        assert!(t.contains("\t\t\"rattleboard\": {\n\t\t\t\"type\": \"stdio\","), "{t}");
    }

    #[test]
    fn codex_toml() {
        let (_t, env) = tmp("codex");
        let cfg = "# my settings\nmodel = \"gpt-5\"  # the best\n\n[mcp_servers.docs]\ncommand = \"docs-mcp\"\nargs = [\"--stdio\"]\n\n[profiles.fast]\nmodel = \"gpt-5-mini\"\n";
        let after = cycle(&env, "codex", &[(".codex/config.toml", cfg)]);
        let t = &after.iter().find(|(p, _)| p.ends_with("config.toml")).unwrap().1;
        let doc: toml_edit::DocumentMut = t.parse().unwrap();
        assert_eq!(doc["mcp_servers"]["rattleboard"]["command"].as_str(), Some(launch().exe.as_str()));
        assert_eq!(doc["mcp_servers"]["docs"]["command"].as_str(), Some("docs-mcp"));
        let notify: Vec<&str> = doc["notify"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(notify[1..], ["--notify", "Codex is done", "--color", "green", "--source", "Codex"]);
        // notify is a top-level key, before the first table
        assert!(t.find("notify").unwrap() < t.find("[mcp_servers.docs]").unwrap(), "{t}");
        assert!(t.starts_with("# my settings\nmodel = \"gpt-5\"  # the best\n"));
        // a fresh config.toml
        let (_t2, env2) = tmp("codex-fresh");
        std::fs::create_dir_all(env2.home.join(".codex")).unwrap();
        cycle(&env2, "codex", &[]);
        // someone else's notify stays, and alerts say why they can't be added
        let (_t3, env3) = tmp("codex-taken");
        let taken = "notify = [\"terminal-notifier\"]\n";
        std::fs::create_dir_all(env3.home.join(".codex")).unwrap();
        std::fs::write(env3.home.join(".codex/config.toml"), taken).unwrap();
        let hs = all(&env3, &launch());
        let h = hs.iter().find(|h| h.id == "codex").unwrap();
        assert!(codex_notify_taken(&h.mcp_file));
        assert!(plan(h, Want { mcp: true, alerts: true }, &State::new()).unwrap_err().contains("another notify"));
        let c = plan(h, Want { mcp: true, alerts: false }, &State::new()).unwrap();
        assert!(c[0].after.as_ref().unwrap().starts_with(taken));
    }

    #[test]
    fn broken_files_are_left_alone() {
        let (_t, env) = tmp("broken");
        std::fs::create_dir_all(env.home.join(".cursor")).unwrap();
        std::fs::write(env.home.join(".cursor/mcp.json"), "{\"mcpServers\": {").unwrap();
        let hs = all(&env, &launch());
        let h = hs.iter().find(|h| h.id == "cursor").unwrap();
        assert!(plan(h, Want { mcp: true, alerts: false }, &State::new()).is_err());
    }

    #[test]
    fn windows_and_linux_paths() {
        let home = PathBuf::from("/home/sam");
        let lin = Env { os: Os::Linux, home: home.clone(), appdata: None, xdg_config: None, codex_home: Some("/opt/codex".into()), claude_config_dir: None };
        let hs = all(&lin, &launch());
        let f = |id: &str| hs.iter().find(|h| h.id == id).unwrap().mcp_file.clone();
        assert_eq!(f("claude-desktop"), home.join(".config/Claude/claude_desktop_config.json"));
        assert_eq!(f("vscode"), home.join(".config/Code/User/mcp.json"));
        assert_eq!(f("opencode"), home.join(".config/opencode/opencode.json"));
        assert_eq!(f("codex"), PathBuf::from("/opt/codex/config.toml"));
        let win = Env { os: Os::Windows, home: "C:/Users/Sam".into(), appdata: Some("C:/Users/Sam/AppData/Roaming".into()), xdg_config: None, codex_home: None, claude_config_dir: None };
        let l = Launch { exe: "C:\\Users\\Sam\\AppData\\Local\\Rattleboard\\rattleboard.exe".into(), ..Default::default() };
        let hs = all(&win, &l);
        let f = |id: &str| hs.iter().find(|h| h.id == id).unwrap().mcp_file.clone();
        assert_eq!(f("claude-desktop"), PathBuf::from("C:/Users/Sam/AppData/Roaming/Claude/claude_desktop_config.json"));
        assert_eq!(f("vscode"), PathBuf::from("C:/Users/Sam/AppData/Roaming/Code/User/mcp.json"));
        assert_eq!(f("claude-code"), PathBuf::from("C:/Users/Sam/.claude.json"));
        assert_eq!(f("opencode"), PathBuf::from("C:/Users/Sam/.config/opencode/opencode.json"));
        assert_eq!(l.notify_cmd(Os::Windows, "Cursor", "Done", "green"), "\"C:\\Users\\Sam\\AppData\\Local\\Rattleboard\\rattleboard.exe\" --notify \"Done\" --color green --source \"Cursor\" --hook");
        let p = Launch { exe: "/tmp/rb $x".into(), profile: Some("agentstest".into()), target: None };
        assert_eq!(p.notify_cmd(Os::Mac, "Cursor", "Done", "green"), "RATTLEBOARD_PROFILE=\"agentstest\" \"/tmp/rb \\$x\" --notify \"Done\" --color green --source \"Cursor\" --hook");
    }

    /// Connects every harness found under $RB_AGENTS_HOME (a throwaway home folder) to
    /// $RB_EXE, for trying the result with the real agent apps:
    ///   RB_AGENTS_HOME=/tmp/fakehome RB_EXE=$PWD/target/debug/rattleboard cargo test -p rattleboard connect_fake_home -- --ignored
    #[test]
    #[ignore]
    fn connect_fake_home() {
        let home = PathBuf::from(std::env::var("RB_AGENTS_HOME").expect("RB_AGENTS_HOME"));
        assert!(home.starts_with(std::env::temp_dir()) || home.to_string_lossy().contains("/tmp/"), "only a temporary folder");
        let env = Env { os: Os::Mac, home, appdata: None, xdg_config: None, codex_home: None, claude_config_dir: None };
        let l = Launch { exe: std::env::var("RB_EXE").expect("RB_EXE"), profile: std::env::var("RATTLEBOARD_PROFILE").ok(), target: None };
        for h in all(&env, &l).iter().filter(|h| h.installed()) {
            let c = plan(h, Want { mcp: true, alerts: true }, &State::new()).unwrap();
            write(&c);
            println!("{}: {:?}", h.id, status(h));
        }
    }

    #[test]
    fn skills_refreshed() {
        let (_t, env) = tmp("skills");
        let hs = all(&env, &launch());
        let ours = env.home.join(".claude/skills/rattleboard/SKILL.md");
        let other = env.home.join(".cursor/skills/rattleboard/SKILL.md");
        // nothing installed: nothing created
        assert!(refresh_skills(&hs).is_empty());
        assert!(!ours.exists());
        for f in [&ours, &other] {
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        }
        std::fs::write(&ours, "---\nname: rattleboard\ndescription: old\n---\nUse `\\n`").unwrap();
        std::fs::write(&other, "---\nname: someone-elses\n---\n").unwrap();
        assert_eq!(refresh_skills(&hs), vec![ours.clone()]);
        assert_eq!(std::fs::read_to_string(&ours).unwrap(), SKILL.replace("\r\n", "\n"));
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "---\nname: someone-elses\n---\n");
        // up to date: left alone
        assert!(refresh_skills(&hs).is_empty());
    }

    #[test]
    fn default_target() {
        let (_t, env) = tmp("target");
        for d in [".claude", ".codex", ".config/opencode"] {
            std::fs::create_dir_all(env.home.join(d)).unwrap();
        }
        let l = Launch { target: Some("group:g1".into()), ..launch() };
        for h in all(&env, &l).iter().filter(|h| ["claude-code", "codex", "opencode"].contains(&h.id)) {
            let c = plan(h, Want { mcp: true, alerts: true }, &State::new()).unwrap();
            write(&c);
            assert_eq!(current_target(h).as_deref(), Some("group:g1"), "{}", h.id);
            for ch in &c {
                let t = ch.after.as_deref().unwrap_or_default();
                if t.contains("--notify") {
                    assert!(t.contains("--target") && t.contains("group:g1"), "{}: {t}", h.id);
                }
            }
            // back to this board: the entry is replaced in place
            let c = plan(h, Want { mcp: true, alerts: true }, &State::new());
            assert!(c.unwrap().is_empty());
            let plain = all(&env, &launch());
            let h2 = plain.iter().find(|x| x.id == h.id).unwrap();
            write(&plan(h2, Want { mcp: true, alerts: true }, &State::new()).unwrap());
            assert_eq!(current_target(h2), None, "{}", h.id);
        }
    }

    #[test]
    fn diffs() {
        let d = diff("a\nb\nc\nd\ne\nf\ng\nh\ni\n", "a\nb\nc\nd\ne\nX\nf\ng\nh\ni\n");
        assert_eq!(d, ["  c", "  d", "  e", "+ X", "  f", "  g", "  h"]);
        assert_eq!(diff("", "x\n"), ["+ x"]);
    }
}
