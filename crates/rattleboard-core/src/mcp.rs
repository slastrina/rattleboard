//! Rattleboard as an MCP server, so AI agent apps (Claude Code, Cursor, Codex, …) can put
//! things on the board as tool calls.
//!
//! The protocol is JSON-RPC 2.0 with MCP's lifecycle and tools methods: `initialize`,
//! `notifications/initialized`, `ping`, `tools/list` and `tools/call`. `handle` answers one
//! message; `serve` runs it over stdio, one JSON message per line (the desktop's `--mcp`), and
//! the API server answers it at `POST /mcp` (Streamable HTTP with JSON responses).
//!
//! The tools only ever write to the board or read what it shows; they never see or return an
//! API key. Where the board lives is a `Backend`: the running app's local API over HTTP
//! (`Http`, for `--mcp`) or the board in-process (the server's own `/mcp`).

use crate::vestaboard::{self, Grid};
use serde_json::{json, Map, Value};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// The versions that start with `initialize`, newest first; the first is offered when a
/// client asks for one we don't know.
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
/// The stateless versions: no `initialize`; each request names its version in
/// `params._meta`, and `server/discover` says what the server speaks.
pub const STATELESS_VERSIONS: [&str; 1] = ["2026-07-28"];
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";

pub fn supported(v: &str) -> bool {
    PROTOCOL_VERSIONS.contains(&v) || STATELESS_VERSIONS.contains(&v)
}

fn all_versions() -> Vec<&'static str> {
    STATELESS_VERSIONS.iter().chain(PROTOCOL_VERSIONS.iter()).copied().collect()
}
pub const SERVER_NAME: &str = "rattleboard";

/// What the user sees when the app (or its API server) can't be reached.
pub const NOT_RUNNING: &str = "Rattleboard isn't reachable. Open Rattleboard and turn on Settings → API server.";

pub type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What `Backend::notify` says for a Rattleboard from before notifications (0.6.1 and
/// earlier), which then gets a plain message or a sequence as it used to.
pub const NO_NOTIFY: &str = "This Rattleboard has no notifications";

/// Who the board's history says sent something when the agent app didn't say who it is.
pub const AGENT: &str = "Agent";

/// The agent app's name as people know it, from MCP's `clientInfo.name` ("claude-code").
pub fn app_name(client: &str) -> String {
    let c = client.trim();
    let l = c.to_ascii_lowercase();
    let known = [
        ("claude-code", "Claude Code"),
        ("claude-ai", "Claude"),
        ("claude", "Claude"),
        ("cursor", "Cursor"),
        ("codex", "Codex"),
        ("gemini", "Gemini CLI"),
        ("visual studio code", "VS Code"),
        ("vscode", "VS Code"),
        ("copilot", "Copilot"),
        ("opencode", "opencode"),
        ("windsurf", "Windsurf"),
        ("zed", "Zed"),
    ];
    if let Some((_, n)) = known.iter().find(|(k, _)| l == *k || l.starts_with(&format!("{k}-")) || l.starts_with(&format!("{k} "))) {
        return n.to_string();
    }
    let clean: String = c.chars().filter(|ch| !ch.is_control()).take(24).collect();
    let clean = clean.trim();
    if clean.is_empty() { AGENT.into() } else { clean.to_string() }
}

/// What the history calls a message from an agent app over MCP: "Claude Code via MCP".
pub fn mcp_source(client: Option<&str>) -> String {
    format!("{} via MCP", client.map(app_name).unwrap_or_else(|| AGENT.into()))
}

/// One MCP connection's state: only who the client is (from `initialize`), so the board's
/// history can say which app a message came from.
#[derive(Default)]
pub struct Session {
    client: std::sync::Mutex<Option<String>>,
}

impl Session {
    fn source(&self, params: &Value) -> String {
        // A stateless request may say who it is on each call.
        let per_call = params.get("_meta").and_then(|m| m.get("io.modelcontextprotocol/clientInfo")).and_then(|c| c.get("name")).and_then(|n| n.as_str());
        let known = self.client.lock().unwrap().clone();
        mcp_source(per_call.or(known.as_deref()))
    }
}

/// Where the board is. Errors are sentences for the model (and the person) to read.
pub trait Backend: Send + Sync {
    /// `/rattleboard/info`: name, board size, adaptive
    fn info(&self) -> Fut<'_, Result<Value, String>>;
    /// The layout on this board now
    fn current(&self) -> Fut<'_, Result<Grid, String>>;
    /// A message for this board, as `POST /local-api/message` takes it
    fn post(&self, body: Value) -> Fut<'_, Result<(), String>>;
    /// A notification for this board, as `POST /rattleboard/notify` takes it: shown once,
    /// with "seconds" going away again. Err(NO_NOTIFY) from a board too old to have it.
    fn notify(&self, body: Value) -> Fut<'_, Result<(), String>>;
    /// A sequence, as `POST /rattleboard/sequence` takes it
    fn play(&self, seq: Value) -> Fut<'_, Result<(), String>>;
    /// `/rattleboard/boards` (this board, saved boards and groups), or None where the board
    /// has no saved boards (Rattleboard Web)
    fn boards(&self) -> Fut<'_, Result<Option<Value>, String>>;
    /// A message for a saved board or group ("<id>", "group:<id>", "all"); the deliveries
    fn send_to(&self, target: String, body: Value) -> Fut<'_, Result<Value, String>>;
    /// Where send_message and send_layout go when the call names no board (`--target`)
    fn default_target(&self) -> Option<String> {
        None
    }
}

// ---------------------------------------------------------------- tools

const CHIPS: [(&str, u8); 8] =
    [("red", 63), ("orange", 64), ("yellow", 65), ("green", 66), ("blue", 67), ("violet", 68), ("white", 69), ("black", 70)];
const SIZES: [(&str, (usize, usize)); 3] = [("standard", (6, 22)), ("compact", (3, 15)), ("large", (8, 30))];

const CODES: &str = "Character codes: 0 blank, 1-26 A-Z, 27-35 the digits 1-9, 36 the digit 0, \
37 ! 38 @ 39 # 40 $ 41 ( 42 ) 44 - 46 + 47 & 48 = 49 ; 50 : 52 ' 53 \" 54 % 55 , 56 . 59 / 60 ? 62 °, \
63-70 solid colour chips (63 red, 64 orange, 65 yellow, 66 green, 67 blue, 68 violet, 69 white, 70 black), 71 a filled tile.";

const TEXT_RULES: &str = "The board is a split-flap display, 6 rows x 22 characters unless it says otherwise \
(list_boards shows each board's size). Text is upper-cased, word-wrapped and centred; a real line break (a newline character in the string) starts a new row. \
Don't write /n for one: / is a character on the board, so it shows. Anything past the last row is cut off, so keep it short: ideally 1-4 lines of up to 22 characters. \
Only A-Z, 0-9, space and ! @ # $ ( ) - + & = ; : ' \" % , . / ? ° can be shown; anything else (emoji, accents, \
lower-case-only symbols) shows as a blank. {63}-{70} inside the text inserts a colour chip (63 red, 64 orange, \
65 yellow, 66 green, 67 blue, 68 violet, 69 white, 70 black).";

fn link_schema() -> Value {
    json!({ "type": "string", "maxLength": 2048, "description": "Optional http:// or https:// address the user can open by clicking the board while the message shows, e.g. the failing build's page or the pull request." })
}

/// The call's optional "link", checked as the board checks it.
fn link_arg(c: &Call<'_>) -> Result<Option<String>, String> {
    match c.str("link")?.map(str::trim).filter(|l| !l.is_empty()) {
        None => Ok(None),
        Some(l) => crate::plugins::safe_link(l).map(Some).ok_or_else(|| "link must be an http:// or https:// address".to_string()),
    }
}

fn priority_schema() -> Value {
    json!({ "type": "string", "enum": ["normal", "alert"], "description": "Optional. \"alert\" only for something that needs the user now (production down, a failed deploy they're waiting on): it still pops up while the user's Focus / Do Not Disturb holds other messages, if they let alerts through. Defaults to normal." })
}

/// The call's optional "priority", as the board takes it.
fn priority_arg(c: &Call<'_>) -> Result<Option<&'static str>, String> {
    match c.str("priority")?.map(str::trim) {
        None | Some("") => Ok(None),
        Some(p) if p.eq_ignore_ascii_case("normal") => Ok(None),
        Some(p) if p.eq_ignore_ascii_case("alert") => Ok(Some("alert")),
        Some(p) => Err(format!("Unknown priority \"{p}\"; use normal or alert")),
    }
}

fn strategy_schema() -> Value {
    json!({
        "type": "string",
        "enum": vestaboard::STRATEGIES,
        "description": "How the flaps turn over: classic (all at once, the default), column, reverse-column, edges-to-center, row, diagonal or random."
    })
}

fn color_schema(what: &str) -> Value {
    json!({
        "type": "string",
        "enum": CHIPS.iter().map(|c| c.0).collect::<Vec<_>>(),
        "description": format!("Optional colour chip put in front of the text {what}: green for success, red for failure, yellow or orange for needs-attention, blue or violet for info.")
    })
}

pub fn tools() -> Value {
    json!([
        {
            "name": "send_message",
            "title": "Send a message to the board",
            "description": format!("Puts a short message on the user's Rattleboard split-flap board, once, where it stays until something else replaces it (it's never repeated or shown again later). Use it to tell the user something worth a glance from across the room: a long task finished, it failed, or you need their input. Don't send routine progress; for a temporary status that should go away by itself, use show_status. {TEXT_RULES}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "minLength": 1, "maxLength": 1000, "description": "The message, e.g. \"Build passed\". Put each line on a line of its own: \"Tests failed\" then a line break then \"in auth module\"." },
                    "color": color_schema("as an accent"),
                    "size": { "type": "string", "enum": ["standard", "compact", "large"], "description": "Lay the text out for a board size: standard 6x22 (default), compact 3x15, large 8x30. Boards set to match each message take this size; others use their own." },
                    "strategy": strategy_schema(),
                    "link": link_schema(),
                    "priority": priority_schema(),
                    "board": { "type": "string", "description": "Which board: \"this\" (this board, the default unless the user set another), or the name of a saved board or group from list_boards, or \"all\"." }
                },
                "required": ["text"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }
        },
        {
            "name": "show_status",
            "title": "Show a temporary status",
            "description": format!("Shows a short status on this computer's board once, for a few seconds, then the board goes back to what it would otherwise show (the previous message, or the user's Sources rotation). It's a one-off notification: it isn't repeated or shown again later, and it doesn't hold up the rotation. Prefer this over send_message for transient states like \"Running tests\" or \"Deploying\". Starting it stops any sequence that was playing. {TEXT_RULES}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "minLength": 1, "maxLength": 1000, "description": "The status, e.g. \"Deploying…\"." },
                    "seconds": { "type": "integer", "minimum": 3, "maximum": 3600, "default": 15, "description": "How long to show it after the flaps settle, 3-3600 (default 15)." },
                    "color": color_schema("as an accent"),
                    "link": link_schema(),
                    "priority": priority_schema()
                },
                "required": ["text"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }
        },
        {
            "name": "send_layout",
            "title": "Send an exact layout",
            "description": format!("Puts an exact grid of character codes on the board, for pixel-art, charts or precise placement. Rows are arrays of integers 0-71, all the same length, from 1x1 up to 12 rows x 40 columns (6x22 fills a standard board). {CODES} For ordinary words use send_message instead."),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "characters": {
                        "type": "array", "minItems": 1, "maxItems": 12,
                        "items": { "type": "array", "minItems": 1, "maxItems": 40, "items": { "type": "integer", "minimum": 0, "maximum": 71 } },
                        "description": "Rows of character codes, top to bottom."
                    },
                    "strategy": strategy_schema(),
                    "board": { "type": "string", "description": "Which board, as for send_message: \"this\" (the default), a saved board or group from list_boards, or \"all\"." }
                },
                "required": ["characters"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
        },
        {
            "name": "list_boards",
            "title": "List boards",
            "description": "Lists the boards a message can go to: this computer's board, the boards the user saved in Rattleboard (other Rattleboards and real split-flap boards) and groups of them, each with its kind and size. Use a name from here as send_message's board.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "play_sequence",
            "title": "Play a sequence",
            "description": format!("Plays frames one after another on this computer's board, each held for `hold` seconds after its flaps settle. Without loop the last frame stays up; with loop it repeats until another message arrives. Use sparingly: a countdown, a short animation, a two-part message. 1-100 frames. Each frame is text (same rules as send_message) or characters (as send_layout). {TEXT_RULES}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "frames": {
                        "type": "array", "minItems": 1, "maxItems": 100,
                        "items": {
                            "type": "object",
                            "properties": {
                                "text": { "type": "string", "description": "This frame's text." },
                                "characters": { "type": "array", "items": { "type": "array", "items": { "type": "integer", "minimum": 0, "maximum": 71 } }, "description": "Or an exact grid of codes." },
                                "hold": { "type": "integer", "minimum": 1, "maximum": 3600, "description": "Seconds to hold this frame." },
                                "strategy": strategy_schema()
                            },
                            "required": ["hold"]
                        }
                    },
                    "loop": { "type": "boolean", "default": false, "description": "Repeat until something else is sent." },
                    "name": { "type": "string", "description": "A name shown in Rattleboard's menu while it plays." }
                },
                "required": ["frames"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }
        },
        {
            "name": "get_board",
            "title": "Read the board",
            "description": "Returns what this computer's board shows right now, as lines of text (colour chips as {red}, {green}, …; a filled tile as {filled}) plus the raw character codes. Use it to check before overwriting something the user put up.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        }
    ])
}

/// One flap code as text, for `get_board`.
fn decode(c: u8) -> String {
    const SYM: [(u8, char); 20] = [
        (37, '!'), (38, '@'), (39, '#'), (40, '$'), (41, '('), (42, ')'), (44, '-'), (46, '+'), (47, '&'), (48, '='),
        (49, ';'), (50, ':'), (52, '\''), (53, '"'), (54, '%'), (55, ','), (56, '.'), (59, '/'), (60, '?'), (62, '°'),
    ];
    match c {
        1..=26 => ((b'A' + c - 1) as char).to_string(),
        27..=35 => ((b'1' + c - 27) as char).to_string(),
        36 => "0".into(),
        63..=70 => format!("{{{}}}", CHIPS[(c - 63) as usize].0),
        71 => "{filled}".into(),
        _ => SYM.iter().find(|(n, _)| *n == c).map(|(_, ch)| ch.to_string()).unwrap_or_else(|| " ".into()),
    }
}

/// The board's rows as text, trailing blanks trimmed.
pub fn grid_text(g: &Grid) -> Vec<String> {
    g.iter().map(|row| row.iter().map(|&c| decode(c)).collect::<String>().trim_end().to_string()).collect()
}

struct Call<'a> {
    args: &'a Map<String, Value>,
    /// Who sent it, for the board's history
    source: &'a str,
}

impl<'a> Call<'a> {
    fn str(&self, k: &str) -> Result<Option<&'a str>, String> {
        match self.args.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.as_str())),
            Some(_) => Err(format!("{k} must be a string")),
        }
    }
    fn text(&self) -> Result<&'a str, String> {
        match self.str("text")? {
            Some(t) if !t.trim().is_empty() => Ok(t),
            _ => Err("text is required: the words to put on the board".into()),
        }
    }
    fn color(&self) -> Result<Option<u8>, String> {
        match self.str("color")? {
            None => Ok(None),
            Some(c) => CHIPS
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(c.trim()))
                .map(|(_, v)| Some(*v))
                .ok_or_else(|| format!("Unknown color \"{c}\"; use one of red, orange, yellow, green, blue, violet, white, black")),
        }
    }
    fn strategy(&self, body: &mut Map<String, Value>) -> Result<(), String> {
        if let Some(s) = self.str("strategy")? {
            if !vestaboard::STRATEGIES.contains(&s) {
                return Err(format!("Unknown strategy \"{s}\"; use one of {}", vestaboard::STRATEGIES.join(", ")));
            }
            body.insert("strategy".into(), json!(s));
        }
        Ok(())
    }
}

fn with_chip(text: &str, chip: Option<u8>) -> String {
    match chip {
        Some(c) => format!("{{{c}}} {}", text.trim()),
        None => text.trim().to_string(),
    }
}

/// The size text will be laid out at on this board: the hint, or the board's own.
async fn layout_size(b: &dyn Backend, hint: Option<(usize, usize)>) -> (usize, usize) {
    let info = b.info().await.ok();
    let board = info.as_ref().and_then(|i| {
        let d = i.get("board")?;
        Some((d.get("rows")?.as_u64()? as usize, d.get("cols")?.as_u64()? as usize))
    });
    let adaptive = info.as_ref().and_then(|i| i.get("adaptive")?.as_bool()).unwrap_or(false);
    match (hint, adaptive) {
        (Some(h), true) => h,
        _ => board.or(hint).unwrap_or((6, 22)),
    }
}

/// A warning for the model when text won't fit, so it can shorten it next time.
fn fit_note(text: &str, (rows, cols): (usize, usize)) -> String {
    if vestaboard::fits(text, rows, cols) {
        String::new()
    } else {
        format!(" Note: it didn't fit and was cut off; this board shows {rows} rows of {cols} characters.")
    }
}

/// Resolves a board or group name from list_boards (or an id) to a send target.
fn resolve(list: &Value, want: &str) -> Result<String, String> {
    let w = want.trim();
    if w.eq_ignore_ascii_case("all") {
        return Ok("all".into());
    }
    let entries = ["boards", "groups"].iter().flat_map(|k| list.get(*k).and_then(|v| v.as_array()).cloned().unwrap_or_default());
    let mut names = vec![];
    for e in entries {
        let (Some(t), Some(n)) = (e.get("target").and_then(|v| v.as_str()), e.get("name").and_then(|v| v.as_str())) else { continue };
        if n.eq_ignore_ascii_case(w) || t == w {
            return Ok(t.to_string());
        }
        names.push(format!("\"{n}\""));
    }
    Err(format!("No board or group is called \"{w}\". Choose one of: \"this\", {}, \"all\"", names.join(", ")))
}

async fn send_message(b: &dyn Backend, c: &Call<'_>) -> Result<Value, String> {
    let text = with_chip(c.text()?, c.color()?);
    let mut body = Map::new();
    body.insert("text".into(), json!(text));
    let hint = match c.str("size")? {
        None => None,
        Some(s) => Some(
            SIZES.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)).map(|(_, d)| *d).ok_or_else(|| format!("Unknown size \"{s}\"; use standard, compact or large"))?,
        ),
    };
    if let Some((r, cl)) = hint {
        body.insert("rows".into(), json!(r));
        body.insert("cols".into(), json!(cl));
    }
    c.strategy(&mut body)?;
    if let Some(l) = link_arg(c)? {
        body.insert("link".into(), json!(l));
    }
    let priority = priority_arg(c)?;
    deliver(b, c, body, Some(&text), hint, priority).await
}

/// Sends a message body to the board the call names, or the default target, or this board.
/// A priority goes to this board only: saved boards get the layout alone.
async fn deliver(b: &dyn Backend, c: &Call<'_>, body: Map<String, Value>, text: Option<&str>, hint: Option<(usize, usize)>, priority: Option<&str>) -> Result<Value, String> {
    let named = c.str("board")?.map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    let here = |s: &str| s.eq_ignore_ascii_case("this") || s.eq_ignore_ascii_case("this board");
    let board = named.or_else(|| b.default_target()).filter(|s| !here(s));
    let Some(want) = board else {
        let size = layout_size(b, hint).await;
        // A notification that stays, so the history says who sent it; it's never re-shown.
        let mut body = body;
        if let Some(p) = priority {
            body.insert("priority".into(), json!(p));
        }
        let mut n = body.clone();
        n.insert("source".into(), json!(c.source));
        match b.notify(Value::Object(n)).await {
            Err(e) if e == NO_NOTIFY => b.post(Value::Object(body)).await?,
            r => r?,
        }
        let note = text.map(|t| fit_note(t, size)).unwrap_or_default();
        return Ok(json!({ "text": format!("Sent to this board.{note}"), "structured": { "sent": ["This board"] } }));
    };
    let list = b.boards().await?.ok_or("This board has no saved boards; leave out board to send to it")?;
    let target = resolve(&list, &want)?;
    let out = b.send_to(target, Value::Object(body)).await?;
    let deliveries = out.get("deliveries").and_then(|d| d.as_array()).cloned().unwrap_or_default();
    let mut ok = vec![];
    let mut failed = vec![];
    for d in &deliveries {
        let name = d.get("name").and_then(|v| v.as_str()).unwrap_or("Board");
        match d.get("error").and_then(|v| v.as_str()) {
            Some(e) => failed.push(format!("{name}: {e}")),
            None => ok.push(name.to_string()),
        }
    }
    if ok.is_empty() {
        return Err(format!("Nothing was sent. {}", failed.join("; ")));
    }
    let mut msg = format!("Sent to {}.", ok.join(", "));
    if !failed.is_empty() {
        msg.push_str(&format!(" Failed: {}.", failed.join("; ")));
    }
    Ok(json!({ "text": msg, "structured": { "sent": ok, "failed": failed } }))
}

async fn show_status(b: &dyn Backend, c: &Call<'_>) -> Result<Value, String> {
    let text = with_chip(c.text()?, c.color()?);
    let secs = match c.args.get("seconds") {
        None | Some(Value::Null) => 15,
        Some(v) => match v.as_u64() {
            Some(n) if (3..=3600).contains(&n) => n,
            _ => return Err("seconds must be a whole number from 3 to 3600".into()),
        },
    };
    let link = link_arg(c)?;
    let size = layout_size(b, None).await;
    // A notification: shown once, then the board goes back to what it would otherwise show
    // (the previous message, or the Sources rotation). Not a sequence, so nothing about it
    // is ever re-shown as a new message.
    let priority = priority_arg(c)?;
    match b.notify(json!({ "text": text, "seconds": secs, "source": c.source, "link": link, "priority": priority })).await {
        Ok(()) => {}
        Err(e) if e == NO_NOTIFY => {
            // An older Rattleboard: the two-frame sequence it understands.
            let before = b.current().await?;
            b.play(json!({ "name": "Status", "frames": [{ "text": text, "hold": secs, "link": link }, { "characters": before, "hold": 1 }] })).await?;
        }
        Err(e) => return Err(e),
    }
    Ok(json!({ "text": format!("Showing it for {secs} s, then the board goes back to what it showed.{}", fit_note(&text, size)) }))
}

async fn send_layout(b: &dyn Backend, c: &Call<'_>) -> Result<Value, String> {
    let chars = c.args.get("characters").ok_or("characters is required: rows of codes 0-71")?;
    let g = vestaboard::posted_grid(chars)?;
    let mut body = Map::new();
    body.insert("characters".into(), json!(g));
    c.strategy(&mut body)?;
    let (r, cl) = vestaboard::dims_of(&g);
    let mut out = deliver(b, c, body, None, None, None).await?;
    out["text"] = json!(format!("{r} x {cl} layout. {}", out["text"].as_str().unwrap_or_default()));
    Ok(out)
}

async fn list_boards(b: &dyn Backend) -> Result<Value, String> {
    let list = match b.boards().await? {
        Some(l) => l,
        None => {
            let info = b.info().await?;
            let d = info.get("board").cloned().unwrap_or(json!({}));
            json!({ "boards": [{ "target": "this", "name": "This board", "kind": "this", "rows": d.get("rows"), "cols": d.get("cols"), "adaptive": info.get("adaptive") }], "groups": [] })
        }
    };
    let mut lines = vec![];
    for e in list.get("boards").and_then(|v| v.as_array()).into_iter().flatten() {
        let size = match (e.get("rows").and_then(|v| v.as_u64()), e.get("cols").and_then(|v| v.as_u64())) {
            (Some(r), Some(c)) => format!("{r} x {c}"),
            _ => "?".into(),
        };
        let adaptive = if e.get("adaptive").and_then(|v| v.as_bool()).unwrap_or(false) { ", matches each message" } else { "" };
        lines.push(format!(
            "- {} ({}, {size}{adaptive})",
            e.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
            e.get("kind").and_then(|v| v.as_str()).unwrap_or("?")
        ));
    }
    for g in list.get("groups").and_then(|v| v.as_array()).into_iter().flatten() {
        let members: Vec<&str> = g.get("members").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|m| m.as_str()).collect();
        lines.push(format!("- {} (group: {})", g.get("name").and_then(|v| v.as_str()).unwrap_or("?"), members.join(", ")));
    }
    Ok(json!({ "text": format!("Boards:\n{}", lines.join("\n")), "structured": list }))
}

async fn play_sequence(b: &dyn Backend, c: &Call<'_>) -> Result<Value, String> {
    let frames = c.args.get("frames").and_then(|f| f.as_array()).ok_or("frames is required: a list of {text or characters, hold}")?;
    let mut seq = Map::new();
    seq.insert("frames".into(), json!(frames));
    if let Some(l) = c.args.get("loop") {
        seq.insert("loop".into(), l.clone());
    }
    // The history labels each frame with the sequence's name: by default, who sent it.
    seq.insert("name".into(), json!(c.str("name")?.unwrap_or(c.source)));
    b.play(Value::Object(seq)).await?;
    let looping = c.args.get("loop").and_then(|v| v.as_bool()).unwrap_or(false);
    Ok(json!({ "text": format!("Playing {} frame{}{}.", frames.len(), if frames.len() == 1 { "" } else { "s" }, if looping { " on a loop" } else { "" }) }))
}

async fn get_board(b: &dyn Backend) -> Result<Value, String> {
    let g = b.current().await?;
    let lines = grid_text(&g);
    let (r, c) = vestaboard::dims_of(&g);
    let words: Vec<&str> = lines.iter().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    let shown = if words.is_empty() { "(nothing, it's blank)".to_string() } else { words.join("\n") };
    Ok(json!({
        "text": format!("The board ({r} x {c}) shows:\n{shown}"),
        "structured": { "rows": r, "cols": c, "lines": lines, "characters": g }
    }))
}

/// Runs one tool for an agent that didn't say who it is.
pub async fn call_tool(b: &dyn Backend, name: &str, args: &Map<String, Value>) -> Result<Value, String> {
    call_tool_as(b, name, args, &mcp_source(None)).await
}

/// Runs one tool, with what the board's history should say sent it ("Claude Code via MCP");
/// also used by the desktop's `--notify`.
pub async fn call_tool_as(b: &dyn Backend, name: &str, args: &Map<String, Value>, source: &str) -> Result<Value, String> {
    let c = Call { args, source };
    match name {
        "send_message" => send_message(b, &c).await,
        "show_status" => show_status(b, &c).await,
        "send_layout" => send_layout(b, &c).await,
        "list_boards" => list_boards(b).await,
        "play_sequence" => play_sequence(b, &c).await,
        "get_board" => get_board(b).await,
        _ => Err(format!("Unknown tool {name}")),
    }
}

// ---------------------------------------------------------------- JSON-RPC

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// The version to answer `initialize` with: the client's if we speak it, else our newest.
pub fn negotiate(requested: Option<&str>) -> &'static str {
    requested.and_then(|r| PROTOCOL_VERSIONS.iter().find(|v| **v == r).copied()).unwrap_or(PROTOCOL_VERSIONS[0])
}

const INSTRUCTIONS: &str = "Rattleboard is a split-flap message board the user glances at from across the room. \
Use it sparingly, for things worth interrupting them for: a long task finished, something failed, or you need \
their input. Keep messages to a few words (6 rows x 22 characters at most). Prefer show_status for temporary \
states. Don't send more than a few messages per session.";

/// Answers one JSON-RPC message (or a batch) with no connection to remember anything for
/// (the API server's `/mcp`); None for notifications and responses, which get no answer.
pub async fn handle(b: &dyn Backend, msg: Value) -> Option<Value> {
    handle_in(b, &Session::default(), msg).await
}

/// Answers one message in a connection (`serve`'s stdio), which remembers who the client is.
pub async fn handle_in(b: &dyn Backend, session: &Session, msg: Value) -> Option<Value> {
    if let Value::Array(items) = msg {
        if items.is_empty() {
            return Some(rpc_error(Value::Null, -32600, "Empty batch"));
        }
        let mut out = vec![];
        for m in items {
            if let Some(r) = Box::pin(handle_in(b, session, m)).await {
                out.push(r);
            }
        }
        return (!out.is_empty()).then_some(Value::Array(out));
    }
    let Some(o) = msg.as_object() else { return Some(rpc_error(Value::Null, -32600, "Invalid request")) };
    let method = o.get("method").and_then(|m| m.as_str());
    let id = o.get("id").cloned();
    let (Some(method), Some(id)) = (method, id) else {
        // A notification (no id) or a response to something we never asked: no answer.
        return if method.is_none() && o.get("id").is_none() { Some(rpc_error(Value::Null, -32600, "Invalid request")) } else { None };
    };
    if o.get("jsonrpc").and_then(|v| v.as_str()) != Some("2.0") {
        return Some(rpc_error(id, -32600, "jsonrpc must be \"2.0\""));
    }
    let params = o.get("params").cloned().unwrap_or(json!({}));
    // A stateless-era request names its version on every call.
    let stateless = match params.get("_meta").and_then(|m| m.get(META_VERSION)).and_then(|v| v.as_str()) {
        Some(v) if !supported(v) => {
            let mut e = rpc_error(id, -32022, "Unsupported protocol version");
            e["error"]["data"] = json!({ "supported": all_versions(), "requested": v });
            return Some(e);
        }
        Some(v) => STATELESS_VERSIONS.contains(&v),
        None => false,
    };
    let mut answer = handle_method(b, session, method, id, &params).await;
    if stateless && answer.get("result").is_some_and(|r| r.is_object()) {
        answer["result"]["resultType"] = json!("complete");
        // Lists say how long a client may cache them; ours never change while running.
        if method.ends_with("/list") {
            answer["result"]["ttlMs"] = json!(300_000);
            answer["result"]["cacheScope"] = json!("public");
        }
    }
    Some(answer)
}

async fn handle_method(b: &dyn Backend, session: &Session, method: &str, id: Value, params: &Value) -> Value {
    match method {
        "initialize" => {
            if let Some(n) = params.get("clientInfo").and_then(|c| c.get("name")).and_then(|n| n.as_str()) {
                *session.client.lock().unwrap() = Some(n.to_string());
            }
            let version = negotiate(params.get("protocolVersion").and_then(|v| v.as_str()));
            rpc_result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": SERVER_NAME, "title": "Rattleboard", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS
                }),
            )
        }
        "server/discover" => rpc_result(
            id,
            json!({
                "supportedVersions": all_versions(),
                "capabilities": { "tools": { "listChanged": false } },
                "_meta": { "io.modelcontextprotocol/serverInfo": { "name": SERVER_NAME, "title": "Rattleboard", "version": env!("CARGO_PKG_VERSION") } },
                "instructions": INSTRUCTIONS
            }),
        ),
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, json!({ "tools": tools() })),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
                return rpc_error(id, -32602, "tools/call needs a tool name");
            };
            if !tools().as_array().unwrap().iter().any(|t| t["name"] == name) {
                return rpc_error(id, -32602, &format!("Unknown tool: {name}"));
            }
            let empty = Map::new();
            let args = match params.get("arguments") {
                None | Some(Value::Null) => &empty,
                Some(Value::Object(a)) => a,
                Some(_) => return rpc_error(id, -32602, "arguments must be an object"),
            };
            // Tool failures are results with isError, so the model can read and fix them.
            let result = match call_tool_as(b, name, args, &session.source(params)).await {
                Ok(v) => {
                    let mut r = json!({ "content": [{ "type": "text", "text": v["text"] }], "isError": false });
                    if let Some(s) = v.get("structured").filter(|s| s.is_object()) {
                        r["structuredContent"] = s.clone();
                    }
                    r
                }
                Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
            };
            rpc_result(id, result)
        }
        // We offer no resources or prompts, but answer politely to clients that ask anyway.
        "resources/list" => rpc_result(id, json!({ "resources": [] })),
        "prompts/list" => rpc_result(id, json!({ "prompts": [] })),
        _ => rpc_error(id, -32601, &format!("Method not found: {method}")),
    }
}

/// Parses one incoming line or body and answers it; a parse error gets a JSON-RPC error.
pub async fn handle_text(b: &dyn Backend, text: &str) -> Option<Value> {
    handle_text_in(b, &Session::default(), text).await
}

pub async fn handle_text_in(b: &dyn Backend, session: &Session, text: &str) -> Option<Value> {
    match serde_json::from_str::<Value>(text) {
        Ok(v) => handle_in(b, session, v).await,
        Err(_) => Some(rpc_error(Value::Null, -32700, "Parse error")),
    }
}

/// MCP over stdio: newline-delimited JSON-RPC on `input`, answers on `output`, until `input`
/// closes. Nothing else is ever written to `output`.
pub async fn serve<R, W>(b: &dyn Backend, input: R, mut output: W) -> std::io::Result<()>
where
    R: tokio::io::AsyncBufRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let mut lines = input.lines();
    let session = Session::default();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(answer) = handle_text_in(b, &session, &line).await {
            let mut s = answer.to_string();
            s.push('\n');
            output.write_all(s.as_bytes()).await?;
            output.flush().await?;
        }
    }
    Ok(())
}

/// `serve` for `rattleboard --mcp`, which can hand over to a newer copy of itself: after each
/// answer, with nothing more waiting on `input`, `stale()` is asked whether the app has been
/// replaced (by an update). If it has, this returns Some(the client's name) so the caller
/// can start the new copy on the same stdin and stdout, and the agent app never notices.
/// `client` is the name a previous copy was given by `initialize`.
pub async fn serve_until<R, W>(b: &dyn Backend, client: Option<String>, input: &mut tokio::io::BufReader<R>, output: &mut W, stale: &dyn Fn() -> bool) -> std::io::Result<Option<Option<String>>>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let session = Session { client: std::sync::Mutex::new(client) };
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line).await? == 0 {
            return Ok(None);
        }
        if line.trim().is_empty() {
            continue;
        }
        if let Some(answer) = handle_text_in(b, &session, line.trim_end()).await {
            let mut s = answer.to_string();
            s.push('\n');
            output.write_all(s.as_bytes()).await?;
            output.flush().await?;
        }
        if input.buffer().is_empty() && stale() {
            return Ok(Some(session.client.lock().unwrap().clone()));
        }
    }
}

// ---------------------------------------------------------------- the local API over HTTP

/// Where the API server is and its key, looked up again for every request so a new key, a
/// new port or an app started after the agent all just work.
pub type Locate = Box<dyn Fn() -> (String, String) + Send + Sync>;

/// The running app's API server over HTTP. The key is only ever sent to that server and
/// never appears in a result or an error.
pub struct Http {
    locate: Locate,
    client: reqwest::Client,
    /// The default target (`--target`)
    pub target: Option<String>,
    /// Said when the server can't be reached, or has no key here
    pub unreachable: String,
    pub no_key: String,
    /// How long to keep trying while the server refuses connections (restarting)
    pub wait: Duration,
}

impl Http {
    /// A server at `base` (e.g. http://127.0.0.1:7070) with `key`
    pub fn new(base: &str, key: &str) -> Self {
        let (base, key) = (base.trim_end_matches('/').to_string(), key.to_string());
        Self::locating(Box::new(move || (base.clone(), key.clone())))
    }

    pub fn locating(locate: Locate) -> Self {
        let client = reqwest::Client::builder().timeout(Duration::from_secs(45)).no_proxy().build().unwrap_or_default();
        Http { locate, client, target: None, unreachable: NOT_RUNNING.into(), no_key: NOT_RUNNING.into(), wait: Duration::ZERO }
    }

    async fn req(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<(u16, Value), String> {
        // Rattleboard restarting (after an update, say) is a few seconds of refused
        // connections: wait that out rather than tell the agent it isn't there.
        let give_up = std::time::Instant::now() + self.wait;
        let res = loop {
            let (base, key) = (self.locate)();
            if key.is_empty() {
                return Err(self.no_key.clone());
            }
            let mut r = self.client.request(method.clone(), format!("{base}{path}")).bearer_auth(&key);
            if let Some(b) = &body {
                r = r.json(b);
            }
            match r.send().await {
                Ok(res) => break res,
                Err(e) if e.is_connect() && std::time::Instant::now() < give_up => tokio::time::sleep(Duration::from_millis(500)).await,
                Err(e) => return Err(if e.is_timeout() { "Rattleboard didn't answer in time.".to_string() } else { self.unreachable.clone() }),
            }
        };
        let status = res.status().as_u16();
        let v: Value = res.json().await.unwrap_or(Value::Null);
        match status {
            200..=299 => Ok((status, v)),
            401 => Err(format!("{} rejected the API key. Check the key in Settings → API server there.", if self.unreachable == NOT_RUNNING { "Rattleboard" } else { "That Rattleboard" })),
            _ => Err(v.get("error").and_then(|e| e.as_str()).map(String::from).unwrap_or_else(|| format!("Rattleboard answered HTTP {status}"))),
        }
    }
}

impl Backend for Http {
    fn default_target(&self) -> Option<String> {
        self.target.clone()
    }
    fn info(&self) -> Fut<'_, Result<Value, String>> {
        Box::pin(async move { self.req(reqwest::Method::GET, "/rattleboard/info", None).await.map(|r| r.1) })
    }
    fn current(&self) -> Fut<'_, Result<Grid, String>> {
        Box::pin(async move {
            let (_, v) = self.req(reqwest::Method::GET, "/local-api/message", None).await?;
            vestaboard::posted_grid(&v)
        })
    }
    fn post(&self, body: Value) -> Fut<'_, Result<(), String>> {
        Box::pin(async move { self.req(reqwest::Method::POST, "/local-api/message", Some(body)).await.map(|_| ()) })
    }
    fn notify(&self, body: Value) -> Fut<'_, Result<(), String>> {
        Box::pin(async move {
            match self.req(reqwest::Method::POST, "/rattleboard/notify", Some(body)).await {
                Ok(_) => Ok(()),
                // A Rattleboard from before notifications
                Err(e) if e == "Not found" => Err(NO_NOTIFY.into()),
                Err(e) => Err(e),
            }
        })
    }
    fn play(&self, seq: Value) -> Fut<'_, Result<(), String>> {
        Box::pin(async move { self.req(reqwest::Method::POST, "/rattleboard/sequence", Some(seq)).await.map(|_| ()) })
    }
    fn boards(&self) -> Fut<'_, Result<Option<Value>, String>> {
        Box::pin(async move {
            match self.req(reqwest::Method::GET, "/rattleboard/boards", None).await {
                Ok((_, v)) => Ok(Some(v)),
                // An older app, or one without saved boards
                Err(e) if e == "Not found" => Ok(None),
                Err(e) => Err(e),
            }
        })
    }
    fn send_to(&self, target: String, body: Value) -> Fut<'_, Result<Value, String>> {
        Box::pin(async move {
            self.req(reqwest::Method::POST, "/rattleboard/send", Some(json!({ "target": target, "message": body }))).await.map(|r| r.1)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequence::Sequence;
    use crate::server::{self, SequenceStatus};
    use crate::vestaboard::Transition;
    use std::sync::{Arc, Mutex};

    /// A board behind the real API server, recording what it's sent.
    #[derive(Default)]
    struct Mock {
        shown: Mutex<Grid>,
        posts: Mutex<Vec<Grid>>,
        links: Mutex<Vec<Option<String>>>,
        played: Mutex<Vec<Sequence>>,
        notices: Mutex<Vec<server::Notice>>,
        sent: Mutex<Vec<(String, Value)>>,
        saved: bool,
        /// Answers 404 to notifications, like a Rattleboard from before them
        old: bool,
    }

    impl server::Board for Mock {
        fn api_key(&self) -> String {
            "secret-key-123".into()
        }
        fn dims(&self) -> (usize, usize) {
            (6, 22)
        }
        fn adaptive(&self) -> bool {
            false
        }
        fn name(&self) -> String {
            "Mock".into()
        }
        fn current(&self) -> (Grid, Option<String>) {
            (self.shown.lock().unwrap().clone(), None)
        }
        fn post(&self, g: Grid, _: Transition, link: Option<String>) -> String {
            self.links.lock().unwrap().push(link);
            *self.shown.lock().unwrap() = g.clone();
            self.posts.lock().unwrap().push(g);
            "m1".into()
        }
        fn notify(&self, n: server::Notice) -> String {
            *self.shown.lock().unwrap() = n.layout.clone();
            self.notices.lock().unwrap().push(n);
            "note-1".into()
        }
        fn play(&self, s: Sequence) -> String {
            self.played.lock().unwrap().push(s);
            "seq-1".into()
        }
        fn sequence(&self) -> Option<SequenceStatus> {
            None
        }
        fn stop(&self) {}
        fn boards(&self) -> Option<Value> {
            self.saved.then(|| {
                json!({
                    "boards": [
                        { "target": "this", "name": "This board", "kind": "this", "rows": 6, "cols": 22, "adaptive": false },
                        { "target": "b1", "name": "Kitchen TV", "kind": "rattleboard", "rows": 3, "cols": 15, "adaptive": false }
                    ],
                    "groups": [{ "target": "group:g1", "name": "Downstairs", "members": ["This board", "Kitchen TV"] }]
                })
            })
        }
        fn send_to(&self, target: String, message: Value) -> Option<server::SendFut> {
            self.sent.lock().unwrap().push((target.clone(), message));
            Some(Box::pin(async move {
                Ok(json!({ "deliveries": [{ "target": target, "name": "Kitchen TV", "status": "sent", "error": null }] }))
            }))
        }
    }

    async fn start(saved: bool) -> (Arc<Mock>, server::Handle, Http) {
        start_with(Mock { saved, ..Default::default() }).await
    }

    async fn start_with(m: Mock) -> (Arc<Mock>, server::Handle, Http) {
        let m = Arc::new(Mock { shown: Mutex::new(vec![vec![0; 22]; 6]), ..m });
        let old = m.old;
        let router = server::router(m.clone(), axum::Router::new());
        // An old Rattleboard: no /rattleboard/notify.
        let router = if old {
            axum::Router::new().route("/rattleboard/notify", axum::routing::post(|| async { (axum::http::StatusCode::NOT_FOUND, axum::Json(json!({"error": "Not found"}))) })).fallback_service(router)
        } else {
            router
        };
        let h = server::serve(router, 0, false).await.unwrap();
        let http = Http::new(&format!("http://{}", h.addr), "secret-key-123");
        (m, h, http)
    }

    fn call(name: &str, args: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": name, "arguments": args } })
    }

    #[tokio::test]
    async fn initialize_and_list() {
        let (_m, _h, b) = start(false).await;
        let r = handle(&b, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}})).await.unwrap();
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(r["result"]["serverInfo"]["name"], "rattleboard");
        assert!(r["result"]["capabilities"]["tools"].is_object());
        // an unknown version gets our newest
        let r = handle(&b, json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}})).await.unwrap();
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
        // notifications get no answer
        assert!(handle(&b, json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await.is_none());
        let r = handle(&b, json!({"jsonrpc": "2.0", "id": "p", "method": "ping"})).await.unwrap();
        assert_eq!(r, json!({"jsonrpc": "2.0", "id": "p", "result": {}}));
        let r = handle(&b, json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"})).await.unwrap();
        let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["send_message", "show_status", "send_layout", "list_boards", "play_sequence", "get_board"]);
        for t in r["result"]["tools"].as_array().unwrap() {
            assert_eq!(t["inputSchema"]["type"], "object");
            assert!(t["description"].as_str().unwrap().len() > 40);
        }
        let r = handle(&b, json!({"jsonrpc": "2.0", "id": 4, "method": "nope"})).await.unwrap();
        assert_eq!(r["error"]["code"], -32601);
        assert_eq!(handle_text(&b, "{not json").await.unwrap()["error"]["code"], -32700);
    }

    /// The stateless protocol (2026-07-28): server/discover, the version on each request.
    #[tokio::test]
    async fn stateless() {
        let (m, _h, b) = start(false).await;
        let meta = json!({ "io.modelcontextprotocol/protocolVersion": "2026-07-28" });
        let r = handle(&b, json!({"jsonrpc": "2.0", "id": 1, "method": "server/discover", "params": {"_meta": meta}})).await.unwrap();
        assert_eq!(r["result"]["resultType"], "complete");
        assert_eq!(r["result"]["supportedVersions"][0], "2026-07-28");
        assert!(r["result"]["capabilities"]["tools"].is_object());
        let r = handle(&b, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"_meta": meta, "name": "send_message", "arguments": {"text": "Stateless"}}})).await.unwrap();
        assert_eq!((r["result"]["resultType"].as_str(), r["result"]["isError"].as_bool()), (Some("complete"), Some(false)));
        assert_eq!(*m.shown.lock().unwrap(), vestaboard::text_layout("Stateless", 6, 22));
        let r = handle(&b, json!({"jsonrpc": "2.0", "id": 9, "method": "tools/list", "params": {"_meta": meta}})).await.unwrap();
        assert_eq!((r["result"]["ttlMs"].as_u64(), r["result"]["cacheScope"].as_str()), (Some(300_000), Some("public")));
        assert_eq!(r["result"]["tools"].as_array().unwrap().len(), 6);
        let r = handle(&b, json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list", "params": {"_meta": {"io.modelcontextprotocol/protocolVersion": "2099-01-01"}}})).await.unwrap();
        assert_eq!(r["error"]["code"], -32022);
        assert_eq!(r["error"]["data"]["requested"], "2099-01-01");
    }

    #[tokio::test]
    async fn send_and_read() {
        let (m, _h, b) = start(false).await;
        let r = handle(&b, call("send_message", json!({"text": "Build passed", "color": "green"}))).await.unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        let shown = m.shown.lock().unwrap().clone();
        assert_eq!(shown, vestaboard::text_layout("{66} Build passed", 6, 22));
        let r = handle(&b, call("get_board", json!({}))).await.unwrap();
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("{green} BUILD PASSED"), "{text}");
        assert_eq!(r["result"]["structuredContent"]["rows"], 6);

        // too long: still sent, with a note to shorten it
        let long = "word ".repeat(60);
        let r = handle(&b, call("send_message", json!({"text": long}))).await.unwrap();
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("cut off"));

        let r = handle(&b, call("send_layout", json!({"characters": [[1, 2, 3], [63, 64, 65]]}))).await.unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        // a fixed 6 x 22 board centres it
        assert!(m.notices.lock().unwrap().last().unwrap().layout[3].windows(3).any(|w| w == [63, 64, 65]));
    }

    /// show_status and send_message are notifications, never sequences: shown once, labelled
    /// with the agent app from `initialize`.
    #[tokio::test]
    async fn links() {
        let (m, _h, b) = start(false).await;
        let r = handle(&b, call("send_message", json!({"text": "PR ready", "link": "https://github.com/slastrina/rattleboard/pull/1"}))).await.unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        assert_eq!(m.notices.lock().unwrap().last().unwrap().link.as_deref(), Some("https://github.com/slastrina/rattleboard/pull/1"));
        let r = handle(&b, call("send_message", json!({"text": "x", "link": "javascript:alert(1)"}))).await.unwrap();
        assert_eq!(r["result"]["isError"], true, "{r}");
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("http"));
        let r = handle(&b, call("show_status", json!({"text": "Deploying", "link": "https://ci.example.com/run/42"}))).await.unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        let n = m.notices.lock().unwrap().last().unwrap().clone();
        assert_eq!((n.seconds, n.link.as_deref()), (Some(15), Some("https://ci.example.com/run/42")));
        assert!(m.played.lock().unwrap().is_empty(), "no sequence");
        let tools = tools();
        assert!(tools.as_array().unwrap().iter().filter(|t| t["name"] == "send_message" || t["name"] == "show_status").all(|t| t["inputSchema"]["properties"]["link"].is_object()));
    }

    /// priority "alert" reaches this board's notification, for Focus / Do Not Disturb.
    #[tokio::test]
    async fn priority() {
        let (m, _h, b) = start(false).await;
        handle(&b, call("send_message", json!({"text": "Hi"}))).await.unwrap();
        assert!(!m.notices.lock().unwrap().last().unwrap().alert);
        let r = handle(&b, call("send_message", json!({"text": "Prod down", "priority": "alert"}))).await.unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        assert!(m.notices.lock().unwrap().last().unwrap().alert);
        let r = handle(&b, call("show_status", json!({"text": "Deploy failed", "priority": "ALERT"}))).await.unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        assert!(m.notices.lock().unwrap().last().unwrap().alert);
        handle(&b, call("show_status", json!({"text": "Deploying", "priority": "normal"}))).await.unwrap();
        assert!(!m.notices.lock().unwrap().last().unwrap().alert);
        let r = handle(&b, call("send_message", json!({"text": "x", "priority": "urgent"}))).await.unwrap();
        assert_eq!(r["result"]["isError"], true, "{r}");
        assert!(tools().as_array().unwrap().iter().filter(|t| t["name"] == "send_message" || t["name"] == "show_status").all(|t| t["inputSchema"]["properties"]["priority"].is_object()));
    }

    #[tokio::test]
    async fn status_and_sequences() {
        let (m, _h, b) = start(false).await;
        *m.shown.lock().unwrap() = vestaboard::text_layout("Before", 6, 22);
        let s = Session::default();
        let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "claude-code", "version": "2.1"}}});
        handle_in(&b, &s, init).await.unwrap();
        let r = handle_in(&b, &s, call("show_status", json!({"text": "Deploying", "seconds": 5}))).await.unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        assert!(m.played.lock().unwrap().is_empty(), "no sequence");
        let n = m.notices.lock().unwrap()[0].clone();
        assert_eq!((n.seconds, n.source.as_str()), (Some(5), "Claude Code via MCP"));
        assert_eq!(n.layout, vestaboard::text_layout("Deploying", 6, 22));
        // the default is 15 seconds
        handle_in(&b, &s, call("show_status", json!({"text": "Testing"}))).await.unwrap();
        assert_eq!(m.notices.lock().unwrap()[1].seconds, Some(15));
        // send_message stays (no seconds), still labelled
        handle_in(&b, &s, call("send_message", json!({"text": "Done"}))).await.unwrap();
        let n = m.notices.lock().unwrap()[2].clone();
        assert_eq!((n.seconds, n.source.as_str()), (None, "Claude Code via MCP"));
        assert!(m.posts.lock().unwrap().is_empty() && m.played.lock().unwrap().is_empty());
        // a client that didn't say who it is
        handle(&b, call("send_message", json!({"text": "Hi"}))).await.unwrap();
        assert_eq!(m.notices.lock().unwrap()[3].source, "Agent via MCP");

        let r = handle(&b, call("play_sequence", json!({"frames": [{"text": "3", "hold": 1}, {"text": "2", "hold": 1}], "loop": true}))).await.unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        let seq = m.played.lock().unwrap()[0].clone();
        assert!(seq.looping);
        assert_eq!(seq.name, "Agent via MCP");
    }

    /// A Rattleboard from before notifications still gets a message, or the old sequence.
    #[tokio::test]
    async fn older_boards() {
        let (m, _h, b) = start_with(Mock { old: true, ..Default::default() }).await;
        *m.shown.lock().unwrap() = vestaboard::text_layout("Before", 6, 22);
        let r = handle(&b, call("send_message", json!({"text": "Hello"}))).await.unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        assert_eq!(m.posts.lock().unwrap().len(), 1);
        let r = handle(&b, call("show_status", json!({"text": "Deploying", "seconds": 5}))).await.unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        let seq = m.played.lock().unwrap()[0].clone();
        assert_eq!(seq.frames.len(), 2);
        assert!(m.notices.lock().unwrap().is_empty());
    }

    #[test]
    fn app_names() {
        for (client, want) in [("claude-code", "Claude Code"), ("cursor-vscode", "Cursor"), ("codex-mcp-client", "Codex"), ("Visual Studio Code", "VS Code"), ("gemini-cli-mcp-client", "Gemini CLI"), ("my-tool", "my-tool"), ("  ", "Agent")] {
            assert_eq!(app_name(client), want, "{client}");
        }
        assert_eq!(mcp_source(None), "Agent via MCP");
        assert_eq!(mcp_source(Some("claude-ai")), "Claude via MCP");
    }

    #[tokio::test]
    async fn errors_are_results() {
        let (_m, _h, b) = start(false).await;
        for (tool, args, want) in [
            ("send_message", json!({}), "text is required"),
            ("send_message", json!({"text": "x", "color": "pink"}), "Unknown color"),
            ("send_message", json!({"text": "x", "strategy": "spin"}), "Unknown strategy"),
            ("send_message", json!({"text": "x", "size": "huge"}), "Unknown size"),
            ("send_message", json!({"text": "x", "board": "Kitchen"}), "no saved boards"),
            ("show_status", json!({"text": "x", "seconds": 1}), "seconds"),
            ("send_layout", json!({"characters": [[99]]}), ""),
            ("play_sequence", json!({"frames": []}), ""),
        ] {
            let r = handle(&b, call(tool, args.clone())).await.unwrap();
            assert_eq!(r["result"]["isError"], true, "{tool} {args}: {r}");
            assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains(want), "{r}");
        }
        let r = handle(&b, call("nope", json!({}))).await.unwrap();
        assert_eq!(r["error"]["code"], -32602);

        // the app isn't running: a clear message, and the key never shows up
        let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
        let off = Http::new(&format!("http://{closed}"), "secret-key-123");
        let r = handle(&off, call("send_message", json!({"text": "hi"}))).await.unwrap();
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("turn on Settings → API server"), "{text}");
        assert!(!r.to_string().contains("secret-key-123"));
        // the wrong key
        let (_m2, h2, _) = start(false).await;
        let wrong = Http::new(&format!("http://{}", h2.addr), "nope");
        let r = handle(&wrong, call("get_board", json!({}))).await.unwrap();
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("rejected the API key"));
    }

    #[tokio::test]
    async fn saved_boards() {
        let (m, _h, b) = start(true).await;
        let r = handle(&b, call("list_boards", json!({}))).await.unwrap();
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Kitchen TV (rattleboard, 3 x 15)") && text.contains("Downstairs (group"), "{text}");
        let r = handle(&b, call("send_message", json!({"text": "Dinner", "board": "kitchen tv", "size": "compact"}))).await.unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        let (target, msg) = m.sent.lock().unwrap()[0].clone();
        assert_eq!(target, "b1");
        assert_eq!(msg, json!({"text": "Dinner", "rows": 3, "cols": 15}));
        handle(&b, call("send_message", json!({"text": "Hi", "board": "Downstairs"}))).await.unwrap();
        assert_eq!(m.sent.lock().unwrap()[1].0, "group:g1");
        let r = handle(&b, call("send_message", json!({"text": "Hi", "board": "Attic"}))).await.unwrap();
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("\"Kitchen TV\""));
        // "this" goes straight to this board
        handle(&b, call("send_message", json!({"text": "Here", "board": "this"}))).await.unwrap();
        assert_eq!(m.notices.lock().unwrap().len(), 1);

        // A default target (--target): calls without a board go there, "this" still here
        let mut t = Http::new(&format!("http://{}", _h.addr), "secret-key-123");
        t.target = Some("group:g1".into());
        handle(&t, call("send_message", json!({"text": "Default"}))).await.unwrap();
        assert_eq!(m.sent.lock().unwrap().last().unwrap().0, "group:g1");
        let r = handle(&t, call("send_layout", json!({"characters": [[1, 2]]}))).await.unwrap();
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().starts_with("1 x 2 layout. Sent to Kitchen TV"), "{r}");
        assert_eq!(m.sent.lock().unwrap().last().unwrap().1, json!({"characters": [[1, 2]]}));
        handle(&t, call("send_message", json!({"text": "Here", "board": "this"}))).await.unwrap();
        assert_eq!(m.notices.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn stdio() {
        let (m, _h, b) = start(false).await;
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#, "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, "\n",
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"send_message","arguments":{"text":"Hello"}}}"#, "\n",
        );
        let mut out = vec![];
        serve(&b, input.as_bytes(), &mut out).await.unwrap();
        let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(lines[1]["id"], 2);
        assert_eq!(*m.shown.lock().unwrap(), vestaboard::text_layout("Hello", 6, 22));
    }

    /// `--mcp` hands over to an updated copy between requests, never in the middle of one,
    /// and keeps the client's name across it.
    #[tokio::test]
    async fn stdio_hand_over() {
        let (_m, _h, b) = start(false).await;
        let init = concat!(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"Claude Code"}}}"#, "\n");
        let two = concat!(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#, "\n", r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#, "\n");
        let mut out = vec![];
        // not replaced: serves to the end
        let mut input = tokio::io::BufReader::new(init.as_bytes());
        assert_eq!(serve_until(&b, None, &mut input, &mut out, &|| false).await.unwrap(), None);
        // replaced: hands over after the answer, with the name from initialize
        let mut input = tokio::io::BufReader::new(init.as_bytes());
        assert_eq!(serve_until(&b, None, &mut input, &mut out, &|| true).await.unwrap(), Some(Some("Claude Code".into())));
        // a request already waiting is answered first
        out.clear();
        let mut input = tokio::io::BufReader::new(two.as_bytes());
        assert_eq!(serve_until(&b, Some("Codex".into()), &mut input, &mut out, &|| true).await.unwrap(), Some(Some("Codex".into())));
        assert_eq!(String::from_utf8(out).unwrap().lines().count(), 2);
    }

    /// While the app restarts, a call waits for it to come back instead of failing.
    #[tokio::test]
    async fn waits_for_a_restart() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let mut b = Http::new(&format!("http://127.0.0.1:{port}"), "secret-key-123");
        b.wait = Duration::from_secs(10);
        let later = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1200)).await;
            server::serve(server::router(Arc::new(Mock { shown: Mutex::new(vec![vec![0; 22]; 6]), ..Default::default() }), axum::Router::new()), port, false).await.unwrap()
        });
        let t = std::time::Instant::now();
        assert!(b.info().await.is_ok());
        assert!(t.elapsed() >= Duration::from_millis(1000));
        drop(later);
        // with no wait, it says so straight away
        let mut b = Http::new("http://127.0.0.1:9", "k");
        b.wait = Duration::ZERO;
        assert_eq!(b.info().await.unwrap_err(), NOT_RUNNING);
    }

    /// The API server's own /mcp: Streamable HTTP with JSON answers, behind the key.
    #[tokio::test]
    async fn over_http() {
        let (m, h, _) = start(false).await;
        let c = reqwest::Client::new();
        let url = format!("http://{}/mcp", h.addr);
        let post = |body: Value| c.post(&url).header("Accept", "application/json, text/event-stream").json(&body);
        let r = post(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}))
            .bearer_auth("secret-key-123").send().await.unwrap();
        assert_eq!(r.status(), 200);
        assert!(r.headers()["content-type"].to_str().unwrap().starts_with("application/json"));
        assert_eq!(r.json::<Value>().await.unwrap()["result"]["protocolVersion"], "2025-06-18");
        let r = post(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).bearer_auth("secret-key-123").send().await.unwrap();
        assert_eq!(r.status(), 202);
        let r = post(call("send_message", json!({"text": "Over HTTP"}))).bearer_auth("secret-key-123")
            .header("MCP-Protocol-Version", "2025-06-18").send().await.unwrap();
        assert_eq!(r.json::<Value>().await.unwrap()["result"]["isError"], false);
        assert_eq!(*m.shown.lock().unwrap(), vestaboard::text_layout("Over HTTP", 6, 22));
        // no key, a page on another site, an unknown version, GET
        assert_eq!(post(json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})).send().await.unwrap().status(), 401);
        let r = post(json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})).bearer_auth("secret-key-123").header("Origin", "https://evil.example").send().await.unwrap();
        assert_eq!(r.status(), 403);
        let r = post(json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})).bearer_auth("secret-key-123").header("Origin", "http://localhost:3000").send().await.unwrap();
        assert_eq!(r.status(), 200);
        let r = post(json!({"jsonrpc": "2.0", "id": 1, "method": "ping"})).bearer_auth("secret-key-123").header("MCP-Protocol-Version", "1999-01-01").send().await.unwrap();
        assert_eq!(r.status(), 400);
        assert_eq!(c.get(&url).bearer_auth("secret-key-123").send().await.unwrap().status(), 405);
        // boards without saved boards: 404 on the endpoints, "this" from list_boards
        let r = c.get(format!("http://{}/rattleboard/boards", h.addr)).bearer_auth("secret-key-123").send().await.unwrap();
        assert_eq!(r.status(), 404);
    }

    #[test]
    fn decoding() {
        let g = vec![vestaboard::parse("HI! 0-9 {63}{71}°")];
        assert_eq!(grid_text(&g), ["HI! 0-9 {red}{filled}°"]);
    }
}
