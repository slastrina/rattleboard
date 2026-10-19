//! Script plugins (`"kind": "script"`): a sandboxed JavaScript module that makes a card.
//!
//!   export default async function (ctx) {
//!     const r = await ctx.fetch("https://icanhazdadjoke.com/", { headers: { Accept: "application/json" } });
//!     return (await r.json()).joke;
//!   }
//!
//! Every run gets a fresh QuickJS runtime (rquickjs): 16 MB of memory, 250 ms of JavaScript
//! (time spent waiting for fetches doesn't count), 15 s in all, at most 5 fetches, each only to
//! a host in the manifest's `hosts` and at most 2 MB. There are no imports, timers, eval or
//! access to files, the network or the environment beyond `ctx`. Secrets are never readable:
//! `ctx.secret("token")` is an opaque token that turns into the value only inside a fetch's
//! URL or headers (or as an HMAC key), on the host side.
//!
//! The JavaScript side of the API is `script_prelude.js`, shared word for word with the phone
//! and TV ports; the host side (limits, the allow-list, secrets, checking the result) is here.
//! The porting spec and the conformance vectors in the community repository's `conformance/`
//! folder define both.

use crate::plugins::{self, Item};
use crate::vestaboard::Grid;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::time::Duration;

pub const PRELUDE: &str = include_str!("script_prelude.js");
pub const MAX_SCRIPT: usize = 64 * 1024;
pub const MEMORY_LIMIT: usize = 16 * 1024 * 1024;
pub const STACK_LIMIT: usize = 512 * 1024;
pub const CPU_LIMIT: Duration = Duration::from_millis(250);
pub const WALL_LIMIT: Duration = Duration::from_secs(15);
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_FETCHES: usize = 5;
pub const MAX_RESPONSE: usize = 2 * 1024 * 1024;
pub const MAX_BODY: usize = 64 * 1024;
pub const MAX_HEADERS: usize = 20;
pub const MAX_STORE: usize = 16 * 1024;
pub const MAX_LOG_LINES: usize = 50;
pub const MAX_LOG_LINE: usize = 200;
pub const MAX_RESULT: usize = 256 * 1024;
pub const MAX_HOSTS: usize = 20;
pub const MAX_TEXT: usize = 4000;
pub const MAX_PAGES: usize = 6;
pub const MAX_ITEMS: usize = 50;
pub const METHODS: [&str; 6] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];
/// Headers a script can't set: the host sets them, or they'd let it act as someone else.
pub const FORBIDDEN_HEADERS: [&str; 8] = ["host", "content-length", "cookie", "connection", "transfer-encoding", "user-agent", "te", "upgrade"];

/// Whether this build runs script plugins. Every target does except 32-bit ARM with musl
/// (Rattleboard Web's linux/arm/v7 image), which QuickJS's bindings don't cover.
pub const SUPPORTED: bool = cfg!(rb_scripts);
pub const UNSUPPORTED: &str = "Script plugins don't run on this build of Rattleboard (32-bit ARM). Use the 64-bit image or the desktop app";

// ---------------------------------------------------------------- the manifest's hosts

/// A `hosts` entry, lower-cased: `api.example.com`, `*.example.com` (any subdomain, at any
/// depth, not example.com itself), `localhost` or an IPv4 address.
pub fn check_host_pattern(s: &str) -> Result<String, String> {
    let p = s.trim().trim_end_matches('.').to_ascii_lowercase();
    let bad = || format!("hosts: {s:?} isn't a host name like api.example.com or *.example.com");
    let (wild, name) = match p.strip_prefix("*.") {
        Some(rest) => (true, rest),
        None => (false, p.as_str()),
    };
    if name.is_empty() || name.len() > 253 {
        return Err(bad());
    }
    let labels: Vec<&str> = name.split('.').collect();
    let label_ok = |l: &&str| !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !labels.iter().all(label_ok) {
        return Err(bad());
    }
    if wild && labels.len() < 2 {
        return Err(format!("hosts: {s:?} is too broad; a wildcard needs a domain like *.example.com"));
    }
    if !wild && labels.len() == 1 && name != "localhost" {
        // A single-word name is a host on the local network; fine on the desktop, but say so.
        return Ok(p);
    }
    Ok(p)
}

pub fn parse_hosts(v: Option<&Value>) -> Result<Vec<String>, String> {
    let a = match v {
        None | Some(Value::Null) => return Err("A script plugin needs \"hosts\": the addresses it may fetch from, like [\"api.example.com\"], or [] if it fetches nothing".into()),
        Some(Value::Array(a)) => a,
        Some(_) => return Err("hosts must be a list of host names".into()),
    };
    if a.len() > MAX_HOSTS {
        return Err(format!("At most {MAX_HOSTS} hosts"));
    }
    let mut out = vec![];
    for h in a {
        let h = check_host_pattern(h.as_str().ok_or("hosts must be a list of host names")?)?;
        if !out.contains(&h) {
            out.push(h);
        }
    }
    Ok(out)
}

/// Whether `host` (from a URL, any case, maybe with a trailing dot) is on the list.
pub fn host_allowed(hosts: &[String], host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    hosts.iter().any(|p| match p.strip_prefix("*.") {
        Some(base) => h.len() > base.len() + 1 && h.ends_with(base) && h.as_bytes()[h.len() - base.len() - 1] == b'.',
        None => *p == h,
    })
}

/// A `script` file name: a plain name ending in .js, no folders.
pub fn check_file_name(s: &str) -> Result<(), String> {
    let ok = s.len() <= 100 && s.ends_with(".js") && s.len() > 3 && !s.starts_with('.') && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b));
    if ok {
        Ok(())
    } else {
        Err(format!("script {s:?} must be a file name next to the manifest, like script.js"))
    }
}

/// The address of a manifest's script file, next to the manifest at `manifest_url`.
pub fn script_url(manifest_url: &str, file: &str) -> Result<String, String> {
    check_file_name(file)?;
    let base = url::Url::parse(manifest_url).map_err(|_| "The manifest's address isn't valid".to_string())?;
    base.join(file).map(|u| u.to_string()).map_err(|_| "The script's address isn't valid".to_string())
}

/// A script plugin's file still to fetch (its `script`, when there's no `scriptInline`).
pub fn file_to_fetch(manifest: &Value) -> Option<String> {
    let inline = manifest.get("scriptInline").is_some_and(|s| !s.is_null());
    (manifest["kind"] == "script" && !inline).then(|| manifest["script"].as_str().map(str::to_string)).flatten()
}

/// Puts a script file's text into a manifest as `scriptInline`, ready to install.
pub fn inline(manifest: &mut Value, source: &[u8]) -> Result<(), String> {
    if source.len() > MAX_SCRIPT {
        return Err("The script is larger than 64 KB".into());
    }
    let text = std::str::from_utf8(source).map_err(|_| "The script isn't UTF-8 text".to_string())?;
    manifest.as_object_mut().ok_or("A plugin manifest is a JSON object")?.insert("scriptInline".into(), json!(text));
    Ok(())
}

// ---------------------------------------------------------------- a run's input and output

/// What a run is given. `secrets` holds the values (host side only); `declared` is every
/// secret the manifest has.
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub now: i64,
    /// Local time, "2026-10-19T09:41:00"
    pub local: String,
    /// Minutes east of UTC
    pub utc_offset: i64,
    pub rows: usize,
    pub cols: usize,
    pub name: String,
    pub settings: Map<String, Value>,
    pub declared: Vec<String>,
    pub secrets: HashMap<String, String>,
    pub store: Map<String, Value>,
    pub hosts: Vec<String>,
}

impl Input {
    /// The JSON the prelude reads (`__rbInput`): never any secret's value.
    pub fn to_json(&self) -> Value {
        let mut set: Vec<&String> = self.declared.iter().filter(|k| self.secrets.get(*k).is_some_and(|v| !v.is_empty())).collect();
        set.sort();
        json!({
            "now": self.now, "local": self.local, "utcOffset": self.utc_offset,
            "board": {"rows": self.rows, "cols": self.cols}, "name": self.name,
            "settings": self.settings, "secrets": self.declared, "secretsSet": set, "store": self.store,
        })
    }
}

/// A request as it's sent: secrets filled in.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    pub status: u16,
    pub url: String,
    /// Lower-case names
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Why a fetch failed: a code from the spec, and its message.
#[derive(Clone, Debug, PartialEq)]
pub struct FetchFail {
    pub code: &'static str,
    pub message: String,
}

impl FetchFail {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        FetchFail { code, message: message.into() }
    }
    pub fn timeout() -> Self {
        Self::new("timeout", "Timed out after 10 s")
    }
    pub fn too_large() -> Self {
        Self::new("too_large", "The response is larger than 2 MB")
    }
    pub fn redirect() -> Self {
        Self::new("redirect_not_allowed", "Redirected to an address that isn't allowed")
    }
}

/// Performs a run's fetches. All the requests a script makes before it next waits come as one
/// batch and are answered together; each answer comes with how long it took, which counts
/// against the run's 15 s (the real thing measures it, a test's mock says it).
pub trait Fetcher {
    fn batch(&self, requests: &[Request], hosts: &[String], limit: Duration) -> Vec<(Result<Response, FetchFail>, Duration)>;
}

/// How a run failed. `kind` is one of the spec's error kinds.
#[derive(Clone, Debug, PartialEq)]
pub struct RunError {
    pub kind: &'static str,
    pub message: String,
}

impl RunError {
    fn new(kind: &'static str, message: impl Into<String>) -> Self {
        RunError { kind, message: message.into() }
    }
    pub fn cpu() -> Self {
        Self::new("cpu", "The script ran for more than 250 ms")
    }
    pub fn wall() -> Self {
        Self::new("wall", "The script took more than 15 s")
    }
    pub fn memory() -> Self {
        Self::new("memory", "The script used more than 16 MB")
    }
    pub fn stalled() -> Self {
        Self::new("stalled", "The script never finished: it waited for something that never happens")
    }
    pub fn unsupported() -> Self {
        Self::new("unsupported", UNSUPPORTED)
    }
}

/// One page of a script's card.
#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    Text(String),
    Grid(Grid),
}

/// A script's card, as it returned it (checked).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Card {
    pub pages: Vec<Page>,
    /// None: the source's chip; Some(None): no chip ("none")
    pub color: Option<Option<u8>>,
    /// None: the manifest's card.align
    pub left: Option<bool>,
    pub link: Option<String>,
    pub alert: bool,
    pub id: Option<String>,
    /// Seconds
    pub dwell: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    pub id: Option<String>,
    pub title: String,
    pub body: String,
    pub time: Option<i64>,
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    None,
    Card(Card),
    Items { items: Vec<ListItem>, color: Option<Option<u8>> },
}

/// What a run did. `store` is the store to keep (only after a run that succeeded).
#[derive(Clone, Debug)]
pub struct Outcome {
    pub result: Result<Output, RunError>,
    pub store: Option<Map<String, Value>>,
    pub log: Vec<String>,
    /// JavaScript time and the whole run, in ms
    pub cpu_ms: u64,
    pub wall_ms: u64,
    /// Every request sent (secrets filled in): for tests only, never logged or shown
    pub requests: Vec<Request>,
}

// ---------------------------------------------------------------- checking the result

fn color_of(v: &Value) -> Result<Option<Option<u8>>, String> {
    let bad = || "color must be one of red, orange, yellow, green, blue, violet, white, black or none".to_string();
    match v {
        Value::Null => Ok(None),
        Value::String(s) if s.eq_ignore_ascii_case("none") => Ok(Some(None)),
        Value::String(s) => plugins::CHIPS.iter().find(|c| c.0.eq_ignore_ascii_case(s)).map(|c| Some(Some(c.1))).ok_or_else(bad),
        _ => Err(bad()),
    }
}

fn text_of(v: &Value, what: &str) -> Result<String, String> {
    let s = v.as_str().ok_or(format!("{what} must be a string"))?;
    if s.chars().count() > MAX_TEXT {
        return Err(format!("{what} is longer than {MAX_TEXT} characters"));
    }
    Ok(s.to_string())
}

fn page_of(v: &Value, what: &str) -> Result<Page, String> {
    match v {
        Value::String(_) => Ok(Page::Text(text_of(v, what)?)),
        Value::Object(o) => {
            let text = o.get("text").filter(|t| !t.is_null());
            let chars = o.get("characters").filter(|t| !t.is_null());
            match (text, chars) {
                (Some(t), None) => Ok(Page::Text(text_of(t, &format!("{what}.text"))?)),
                (None, Some(c)) => crate::vestaboard::posted_grid(c).map(Page::Grid).map_err(|e| format!("{what}.characters: {e}")),
                (Some(_), Some(_)) => Err(format!("{what}: use text or characters, not both")),
                (None, None) => Err(format!("{what} needs text or characters")),
            }
        }
        _ => Err(format!("{what} must be text or {{\"text\": ...}} or {{\"characters\": [[...]]}}")),
    }
}

fn id_of(v: Option<&Value>, what: &str) -> Result<Option<String>, String> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.chars().count() <= 200 => Ok(Some(s.clone())),
        Some(Value::Number(n)) => Ok(Some(plugins::value_text(&Value::Number(n.clone())))),
        Some(_) => Err(format!("{what} must be a string (200 characters at most) or a number")),
    }
}

fn cut(s: &str, n: usize) -> String {
    s.trim().chars().take(n).collect()
}

/// Checks what a script returned: a string, a card object, a list of items, or null.
pub fn validate(v: &Value) -> Result<Output, String> {
    let o = match v {
        Value::Null => return Ok(Output::None),
        Value::String(_) => {
            let t = text_of(v, "The text")?;
            return Ok(if t.trim().is_empty() { Output::None } else { Output::Card(Card { pages: vec![Page::Text(t)], ..Default::default() }) });
        }
        Value::Object(o) => o,
        _ => return Err("Return a string, an object like {\"text\": \"...\"}, {\"items\": [...]} or null".into()),
    };
    let color = color_of(o.get("color").unwrap_or(&Value::Null))?;
    if let Some(items) = o.get("items").filter(|i| !i.is_null()) {
        let a = items.as_array().filter(|a| a.len() <= MAX_ITEMS).ok_or(format!("items must be a list of at most {MAX_ITEMS}"))?;
        let mut out = vec![];
        for (k, it) in a.iter().enumerate() {
            let what = format!("items[{k}]");
            let it = it.as_object().ok_or(format!("{what} must be an object like {{\"title\": \"...\"}}"))?;
            let s = |f: &str, n: usize| -> Result<String, String> {
                match it.get(f) {
                    None | Some(Value::Null) => Ok(String::new()),
                    Some(Value::String(s)) => Ok(cut(s, n)),
                    Some(_) => Err(format!("{what}.{f} must be a string")),
                }
            };
            let (title, body) = (s("title", 400)?, s("body", 2000)?);
            if title.is_empty() && body.is_empty() {
                return Err(format!("{what} needs a title or a body"));
            }
            let time = match it.get("time") {
                None | Some(Value::Null) => None,
                Some(t @ (Value::Number(_) | Value::String(_))) => Some(plugins::parse_time(t).ok_or(format!("{what}.time isn't a time (use milliseconds since 1970 or ISO 8601)"))?),
                Some(_) => return Err(format!("{what}.time isn't a time (use milliseconds since 1970 or ISO 8601)")),
            };
            let link = match it.get("link") {
                None | Some(Value::Null) => None,
                Some(Value::String(l)) => plugins::safe_link(l),
                Some(_) => return Err(format!("{what}.link must be a string")),
            };
            out.push(ListItem { id: id_of(it.get("id"), &format!("{what}.id"))?, title, body, time, link });
        }
        return Ok(Output::Items { items: out, color });
    }
    let present = |k: &str| o.get(k).filter(|v| !v.is_null());
    let pages = match (present("text"), present("characters"), present("pages")) {
        (Some(t), None, None) => vec![Page::Text(text_of(t, "text")?)],
        (None, Some(c), None) => vec![crate::vestaboard::posted_grid(c).map(Page::Grid).map_err(|e| format!("characters: {e}"))?],
        (None, None, Some(p)) => {
            let a = p.as_array().filter(|a| (1..=MAX_PAGES).contains(&a.len())).ok_or(format!("pages must be a list of 1 to {MAX_PAGES} pages"))?;
            a.iter().enumerate().map(|(k, p)| page_of(p, &format!("pages[{k}]"))).collect::<Result<Vec<_>, _>>()?
        }
        (None, None, None) => return Err("Return text, characters, pages or items".into()),
        _ => return Err("Use only one of text, characters and pages".into()),
    };
    if pages.iter().all(|p| matches!(p, Page::Text(t) if t.trim().is_empty())) {
        return Ok(Output::None);
    }
    let left = match present("align").map(|a| a.as_str()) {
        None => None,
        Some(Some("center" | "centre")) => Some(false),
        Some(Some("left")) => Some(true),
        Some(_) => return Err("align must be center or left".into()),
    };
    let link = match present("link") {
        None => None,
        Some(Value::String(l)) => plugins::safe_link(l),
        Some(_) => return Err("link must be a string".into()),
    };
    let alert = match present("priority").map(|a| a.as_str()) {
        None | Some(Some("ambient")) => false,
        Some(Some("alert")) => true,
        Some(_) => return Err("priority must be ambient or alert".into()),
    };
    let dwell = match present("dwell") {
        None => None,
        Some(d) => {
            let d = plugins::duration(d).map_err(|_| "dwell must be from 10s to 1h (seconds, or like \"30s\")".to_string())?;
            if !(10..=3600).contains(&d) {
                return Err("dwell must be from 10s to 1h (seconds, or like \"30s\")".into());
            }
            Some(d)
        }
    };
    Ok(Output::Card(Card { pages, color, left, link, alert, id: id_of(o.get("id"), "id")?, dwell }))
}

impl Card {
    /// The chip: the script's colour, or `default` (the source's) when it didn't say.
    pub fn chip(&self, default: Option<u8>) -> Option<u8> {
        match self.color {
            None => default,
            Some(c) => c,
        }
    }

    /// As a board face: each page in turn, text laid out like a template (`overflow`: up to
    /// that many pages for text that doesn't fit), characters centred as they are.
    pub fn face(&self, chip: Option<u8>, left: bool, overflow: usize) -> crate::cards::Face {
        use crate::cards::Face;
        let left = self.left.unwrap_or(left);
        let mut faces: Vec<Face> = self
            .pages
            .iter()
            .map(|p| match p {
                Page::Text(t) => Face::Text { text: t.clone(), left, chip, pages: overflow.clamp(1, 3) },
                Page::Grid(g) => Face::Grid { grid: g.clone() },
            })
            .collect();
        if faces.len() == 1 {
            faces.remove(0)
        } else {
            Face::Pages { pages: faces }
        }
    }
}

impl Output {
    /// The canonical form the conformance vectors compare.
    pub fn to_json(&self) -> Value {
        let color = |c: &Option<Option<u8>>| match c {
            None => Value::Null,
            Some(None) => json!("none"),
            Some(Some(c)) => json!(plugins::chip_name(*c)),
        };
        match self {
            Output::None => json!({"type": "none"}),
            Output::Card(c) => json!({
                "type": "card",
                "pages": c.pages.iter().map(|p| match p { Page::Text(t) => json!({"text": t}), Page::Grid(g) => json!({"characters": g}) }).collect::<Vec<_>>(),
                "color": color(&c.color),
                "align": c.left.map(|l| if l { "left" } else { "center" }),
                "link": c.link, "priority": if c.alert { "alert" } else { "ambient" }, "id": c.id, "dwell": c.dwell,
            }),
            Output::Items { items, color: c } => json!({
                "type": "items",
                "items": items.iter().map(|i| json!({"id": i.id, "title": i.title, "body": i.body, "time": i.time, "link": i.link})).collect::<Vec<_>>(),
                "color": color(c),
            }),
        }
    }

    /// As the Sources engine's data: a card is a status (shown as it is now), items are a
    /// feed (each shown once), nothing is an empty status.
    pub fn into_data(self) -> plugins::Data {
        match self {
            Output::None => plugins::Data { items: vec![], single: true },
            Output::Card(c) => {
                let title = c.pages.iter().find_map(|p| match p { Page::Text(t) => Some(t.clone()), _ => None }).unwrap_or_default();
                let key = match &c.id {
                    Some(id) => plugins::hash(&[id]),
                    None => plugins::hash(&[&format!("{:?}", c.pages)]),
                };
                let item = Item { key, title, alert: c.alert, chip: c.color.flatten(), link: c.link.clone(), script: Some(c), ..Default::default() };
                plugins::Data { items: vec![item], single: true }
            }
            Output::Items { items, color } => plugins::Data {
                items: items
                    .into_iter()
                    .map(|i| Item {
                        key: match &i.id {
                            Some(id) => plugins::hash(&[id]),
                            None => plugins::hash(&[&i.title, &i.body]),
                        },
                        title: i.title,
                        body: i.body,
                        time: i.time,
                        link: i.link,
                        chip: color.flatten(),
                        ..Default::default()
                    })
                    .collect(),
                single: false,
            },
        }
    }
}

// ---------------------------------------------------------------- requests (host side)

/// Percent-encodes a secret for a URL: everything but letters, digits and - . _ ~ * , so it
/// can never change the address's host or path structure.
fn encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>().replace('+', "%20")
}

/// Fills in `{{secret.name}}`: `values` are the secrets, `url` percent-encodes them. Returns
/// the text and the secret values used.
fn fill(s: &str, input: &Input, url: bool, used: &mut Vec<String>) -> Result<String, FetchFail> {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("{{secret.") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 9..];
        let Some(j) = after.find("}}") else {
            out.push_str(&rest[i..]);
            rest = "";
            break;
        };
        let name = after[..j].trim();
        if !input.declared.iter().any(|d| d == name) {
            return Err(FetchFail::new("unknown_secret", format!("No secret called {name} is declared")));
        }
        let v = input.secrets.get(name).cloned().unwrap_or_default();
        if v.is_empty() {
            return Err(FetchFail::new("secret_missing", format!("Add the {name} secret in this source's settings")));
        }
        out.push_str(&if url { encode(&v) } else { v.clone() });
        if !used.contains(&v) {
            used.push(v);
        }
        rest = &after[j + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Checks one ctx.fetch call and fills in its secrets, in the spec's order: the count, the
/// address, the scheme, the allow-list, the method, headers and body, then secrets.
pub fn check_request(req: &Value, input: &Input, count: usize) -> Result<(Request, Vec<String>), FetchFail> {
    if count >= MAX_FETCHES {
        return Err(FetchFail::new("too_many_fetches", "A script can fetch at most 5 times per run"));
    }
    let bad_url = || FetchFail::new("bad_url", "Not a valid http(s) address");
    let raw = req["url"].as_str().unwrap_or("").trim();
    let u = url::Url::parse(raw).map_err(|_| bad_url())?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none_or(|h| h.is_empty()) || !u.username().is_empty() || u.password().is_some() {
        return Err(bad_url());
    }
    // Secrets go in the path, query or fragment only: the host is checked as written.
    let authority_end = raw.find("://").map(|i| i + 3).and_then(|s| raw[s..].find(['/', '?', '#']).map(|e| s + e)).unwrap_or(raw.len());
    if raw[..authority_end].contains("{{") {
        return Err(bad_url());
    }
    plugins::check_scheme(&u).map_err(|e| FetchFail::new("insecure", e))?;
    let host = u.host_str().unwrap_or("").trim_start_matches('[').trim_end_matches(']').to_string();
    if !host_allowed(&input.hosts, &host) {
        return Err(FetchFail::new("host_not_allowed", format!("{host} isn't in this plugin's hosts")));
    }
    let method = req["method"].as_str().unwrap_or("GET").trim().to_ascii_uppercase();
    if !METHODS.contains(&method.as_str()) {
        return Err(FetchFail::new("bad_request", format!("Method {method} isn't allowed; use GET, POST, PUT, PATCH, DELETE or HEAD")));
    }
    let mut headers = vec![];
    let hs = req["headers"].as_object().cloned().unwrap_or_default();
    if hs.len() > MAX_HEADERS {
        return Err(FetchFail::new("bad_request", format!("At most {MAX_HEADERS} headers")));
    }
    for (k, v) in &hs {
        let name = k.trim().to_ascii_lowercase();
        let v = v.as_str().unwrap_or("");
        if reqwest::header::HeaderName::from_bytes(name.as_bytes()).is_err() || FORBIDDEN_HEADERS.contains(&name.as_str()) || name.starts_with("proxy-") || name.starts_with("sec-") {
            return Err(FetchFail::new("bad_request", format!("Header {k} isn't allowed")));
        }
        if v.len() > 4000 {
            return Err(FetchFail::new("bad_request", format!("Header {k} is too long")));
        }
        headers.push((name, v.to_string()));
    }
    let body = req["body"].as_str().map(str::to_string);
    if let Some(b) = &body {
        if method == "GET" || method == "HEAD" {
            return Err(FetchFail::new("bad_request", format!("A {method} request has no body")));
        }
        if b.len() > MAX_BODY {
            return Err(FetchFail::new("bad_request", "The body is larger than 64 KB"));
        }
    }
    let mut used = vec![];
    let url = fill(raw, input, true, &mut used)?;
    let mut filled = vec![];
    for (k, v) in headers {
        let v = fill(&v, input, false, &mut used)?;
        if reqwest::header::HeaderValue::from_str(&v).is_err() {
            return Err(FetchFail::new("bad_request", format!("Header {k} isn't valid")));
        }
        filled.push((k, v));
    }
    filled.sort();
    url::Url::parse(&url).map_err(|_| bad_url())?;
    Ok((Request { method, url, headers: filled, body }, used))
}

/// Replaces secret values (and their URL-encoded forms) in what came back, so a server that
/// echoes a request can't hand the script its own key.
fn redact(s: &str, used: &[String]) -> String {
    let mut out = s.to_string();
    for v in used.iter().filter(|v| v.len() >= 4) {
        out = out.replace(v.as_str(), "[secret]");
        let e = encode(v);
        if e != *v {
            out = out.replace(&e, "[secret]");
        }
    }
    out
}

fn hmac(algo: &str, key: &Value, data: &str, input: &Input) -> Result<String, String> {
    let key: Vec<u8> = if let Some(name) = key["secret"].as_str() {
        if !input.declared.iter().any(|d| d == name) {
            return Err(format!("No secret called {name} is declared"));
        }
        input.secrets.get(name).cloned().unwrap_or_default().into_bytes()
    } else {
        key["text"].as_str().unwrap_or("").as_bytes().to_vec()
    };
    use hmac::{Hmac, Mac};
    let bytes = match algo.trim().to_ascii_lowercase().trim_start_matches("hmac-") {
        "sha1" => {
            let mut m = Hmac::<sha1::Sha1>::new_from_slice(&key).expect("any key length");
            m.update(data.as_bytes());
            m.finalize().into_bytes().to_vec()
        }
        "sha256" => {
            let mut m = Hmac::<sha2::Sha256>::new_from_slice(&key).expect("any key length");
            m.update(data.as_bytes());
            m.finalize().into_bytes().to_vec()
        }
        "sha512" => {
            let mut m = Hmac::<sha2::Sha512>::new_from_slice(&key).expect("any key length");
            m.update(data.as_bytes());
            m.finalize().into_bytes().to_vec()
        }
        a => return Err(format!("hmac: unknown algorithm {a:?}; use sha1, sha256 or sha512")),
    };
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Checks the store the script left, and keeps it if it's within 16 KB.
fn check_store(v: &Value) -> Result<Map<String, Value>, String> {
    let o = v.as_object().cloned().unwrap_or_default();
    if serde_json::to_vec(&o).map(|b| b.len()).unwrap_or(usize::MAX) > MAX_STORE {
        return Err("The store is larger than 16 KB".into());
    }
    Ok(o)
}

// ---------------------------------------------------------------- running

/// Runs a script. Blocks: call it from a blocking thread (`tokio::task::spawn_blocking`).
pub fn run(source: &str, input: &Input, fetcher: &dyn Fetcher) -> Outcome {
    #[cfg(rb_scripts)]
    {
        engine::run(source, input, fetcher)
    }
    #[cfg(not(rb_scripts))]
    {
        let _ = (source, input, fetcher);
        Outcome { result: Err(RunError::unsupported()), store: None, log: vec![], cpu_ms: 0, wall_ms: 0, requests: vec![] }
    }
}

#[cfg(rb_scripts)]
mod engine {
    use super::*;
    use rquickjs::allocator::{Allocator, RustAllocator};
    use rquickjs::context::intrinsic;
    use rquickjs::{CatchResultExt, Context, Ctx, Function, Module, Object, Runtime};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Instant;

    type Intrinsics = (
        intrinsic::Date,
        intrinsic::Eval,
        intrinsic::RegExpCompiler,
        intrinsic::RegExp,
        intrinsic::Json,
        intrinsic::Proxy,
        intrinsic::MapSet,
        intrinsic::TypedArrays,
        intrinsic::Promise,
        intrinsic::WeakRef,
    );

    /// Rust's allocator with the run's memory limit: an allocation past it fails (QuickJS then
    /// throws "out of memory", or worse) and `hit` remembers that it did, so the run is
    /// reported as over its memory, whatever the script made of the error.
    struct Limited {
        used: Rc<Cell<usize>>,
        hit: Rc<Cell<bool>>,
    }

    impl Limited {
        fn room(&self, more: usize) -> bool {
            let ok = self.used.get().saturating_add(more) <= MEMORY_LIMIT;
            if !ok {
                self.hit.set(true);
            }
            ok
        }
    }

    unsafe impl Allocator for Limited {
        fn alloc(&mut self, size: usize) -> *mut u8 {
            if !self.room(size) {
                return std::ptr::null_mut();
            }
            let p = RustAllocator.alloc(size);
            if !p.is_null() {
                self.used.set(self.used.get() + unsafe { RustAllocator::usable_size(p) });
            }
            p
        }

        fn calloc(&mut self, count: usize, size: usize) -> *mut u8 {
            let Some(total) = count.checked_mul(size) else { return std::ptr::null_mut() };
            if total == 0 || !self.room(total) {
                return std::ptr::null_mut();
            }
            let p = RustAllocator.calloc(count, size);
            if !p.is_null() {
                self.used.set(self.used.get() + unsafe { RustAllocator::usable_size(p) });
            }
            p
        }

        unsafe fn dealloc(&mut self, ptr: *mut u8) {
            if ptr.is_null() {
                return;
            }
            self.used.set(self.used.get().saturating_sub(RustAllocator::usable_size(ptr)));
            RustAllocator.dealloc(ptr)
        }

        unsafe fn realloc(&mut self, ptr: *mut u8, new_size: usize) -> *mut u8 {
            if ptr.is_null() {
                return self.alloc(new_size);
            }
            if new_size == 0 {
                self.dealloc(ptr);
                return std::ptr::null_mut();
            }
            let old = RustAllocator::usable_size(ptr);
            if new_size > old && !self.room(new_size - old) {
                return std::ptr::null_mut();
            }
            let p = RustAllocator.realloc(ptr, new_size);
            if !p.is_null() {
                self.used.set(self.used.get() - old + RustAllocator::usable_size(p));
            }
            p
        }

        unsafe fn usable_size(ptr: *mut u8) -> usize {
            RustAllocator::usable_size(ptr)
        }
    }

    /// The run's clock: JavaScript time, and the whole run's time (with a mock's pretend
    /// fetch time added).
    struct Budget {
        start: Instant,
        js_since: Cell<Option<Instant>>,
        js: Cell<Duration>,
        /// Fetch time as reported minus as measured (a mock's delays)
        extra: Cell<Duration>,
        tripped: Cell<Option<&'static str>>,
    }

    impl Budget {
        fn cpu(&self) -> Duration {
            self.js.get() + self.js_since.get().map(|t| t.elapsed()).unwrap_or_default()
        }
        fn wall(&self) -> Duration {
            self.start.elapsed() + self.extra.get()
        }
        fn check(&self) -> bool {
            if self.tripped.get().is_some() {
                return true;
            }
            if self.cpu() > CPU_LIMIT {
                self.tripped.set(Some("cpu"));
            } else if self.wall() > WALL_LIMIT {
                self.tripped.set(Some("wall"));
            }
            self.tripped.get().is_some()
        }
        fn pause(&self) {
            if let Some(t) = self.js_since.take() {
                self.js.set(self.js.get() + t.elapsed());
            }
        }
        fn resume(&self) {
            self.js_since.set(Some(Instant::now()));
        }
        fn tripped_error(&self) -> Option<RunError> {
            match self.tripped.get() {
                Some("cpu") => Some(RunError::cpu()),
                Some(_) => Some(RunError::wall()),
                None => None,
            }
        }
    }

    #[derive(Default)]
    struct HostState {
        count: usize,
        pending: Vec<(u64, Request, Vec<String>)>,
        log: Vec<String>,
        log_full: bool,
        requests: Vec<Request>,
    }

    /// The text of the exception just caught: "Error: message" style, from the engine.
    fn exception_text(ctx: &Ctx, e: rquickjs::CaughtError) -> String {
        let _ = ctx;
        match e {
            rquickjs::CaughtError::Exception(x) => {
                let m = x.message().unwrap_or_default();
                if m.is_empty() {
                    x.to_string()
                } else {
                    m
                }
            }
            rquickjs::CaughtError::Value(v) => v.as_string().and_then(|s| s.to_string().ok()).unwrap_or_else(|| format!("{v:?}")),
            rquickjs::CaughtError::Error(e) => e.to_string(),
        }
    }

    fn is_oom(msg: &str) -> bool {
        msg.contains("out of memory") || msg.contains("Allocation failed")
    }

    pub fn run(source: &str, input: &Input, fetcher: &dyn Fetcher) -> Outcome {
        let budget = Rc::new(Budget { start: Instant::now(), js_since: Cell::new(None), js: Cell::new(Duration::ZERO), extra: Cell::new(Duration::ZERO), tripped: Cell::new(None) });
        let state = Rc::new(RefCell::new(HostState::default()));
        let result = run_inner(source, input, fetcher, &budget, &state);
        budget.pause();
        let st = state.take();
        let (result, store) = match result {
            Ok((out, store)) => (Ok(out), Some(store)),
            Err(e) => (Err(e), None),
        };
        Outcome { result, store, log: st.log, cpu_ms: budget.js.get().as_millis() as u64, wall_ms: budget.wall().as_millis() as u64, requests: st.requests }
    }

    fn run_inner(source: &str, input: &Input, fetcher: &dyn Fetcher, budget: &Rc<Budget>, state: &Rc<RefCell<HostState>>) -> Result<(Output, Map<String, Value>), RunError> {
        if source.len() > MAX_SCRIPT {
            return Err(RunError::new("syntax", "The script is larger than 64 KB"));
        }
        let hit = Rc::new(Cell::new(false));
        let rt = Runtime::new_with_alloc(Limited { used: Rc::new(Cell::new(0)), hit: hit.clone() }).map_err(|_| RunError::memory())?;
        rt.set_max_stack_size(STACK_LIMIT);
        rt.set_gc_threshold(4 * 1024 * 1024);
        let b = budget.clone();
        rt.set_interrupt_handler(Some(Box::new(move || b.check())));
        let context = Context::custom::<Intrinsics>(&rt).map_err(|_| RunError::memory())?;
        budget.resume();
        let res = context.with(|ctx| run_in(&ctx, source, input, fetcher, budget, state));
        budget.pause();
        // Whatever went wrong, running out of time or memory is the reason.
        if let Some(e) = budget.tripped_error() {
            return Err(e);
        }
        if hit.get() && res.is_err() {
            return Err(RunError::memory());
        }
        res
    }

    fn run_in<'js>(ctx: &Ctx<'js>, source: &str, input: &Input, fetcher: &dyn Fetcher, budget: &Rc<Budget>, state: &Rc<RefCell<HostState>>) -> Result<(Output, Map<String, Value>), RunError> {
        let fail = |kind: &'static str, msg: String| -> RunError {
            if let Some(e) = budget.tripped_error() {
                return e;
            }
            if is_oom(&msg) {
                return RunError::memory();
            }
            match kind {
                "syntax" => RunError::new("syntax", format!("The script doesn't load: {msg}")),
                _ => RunError::new("exception", format!("The script failed: {msg}")),
            }
        };
        let js = |e: rquickjs::Error| -> RunError {
            if let Some(e) = budget.tripped_error() {
                return e;
            }
            match e {
                rquickjs::Error::Allocation => RunError::memory(),
                e => RunError::new("exception", format!("The script failed: {e}")),
            }
        };

        // The host's three functions.
        let host = Object::new(ctx.clone()).map_err(js)?;
        let (st, inp) = (state.clone(), input.clone());
        host.set(
            "fetch",
            Function::new(ctx.clone(), move |id: f64, req: String| -> String {
                let mut s = st.borrow_mut();
                let req: Value = serde_json::from_str(&req).unwrap_or(Value::Null);
                let checked = check_request(&req, &inp, s.count);
                s.count += 1;
                match checked {
                    Ok((r, used)) => {
                        s.pending.push((id as u64, r, used));
                        r#"{"ok":true}"#.to_string()
                    }
                    Err(f) => json!({"error": f.code, "message": f.message}).to_string(),
                }
            })
            .map_err(js)?,
        )
        .map_err(js)?;
        let inp = input.clone();
        host.set(
            "hmac",
            Function::new(ctx.clone(), move |algo: String, key: String, data: String| -> String {
                let key: Value = serde_json::from_str(&key).unwrap_or(Value::Null);
                match hmac(&algo, &key, &data, &inp) {
                    Ok(hex) => json!({ "hex": hex }).to_string(),
                    Err(m) => json!({"error": "bad_hmac", "message": m}).to_string(),
                }
            })
            .map_err(js)?,
        )
        .map_err(js)?;
        let st = state.clone();
        host.set(
            "log",
            Function::new(ctx.clone(), move |line: String| {
                let mut s = st.borrow_mut();
                if s.log.len() < MAX_LOG_LINES {
                    s.log.push(line.chars().take(MAX_LOG_LINE).collect());
                } else if !s.log_full {
                    s.log_full = true;
                    s.log.push("… (more lines not kept)".into());
                }
            })
            .map_err(js)?,
        )
        .map_err(js)?;
        let g = ctx.globals();
        g.set("__rbHost", host).map_err(js)?;
        g.set("__rbInput", input.to_json().to_string()).map_err(js)?;
        let runner: Object = ctx.eval(PRELUDE).catch(ctx).map_err(|e| fail("exception", exception_text(ctx, e)))?;

        // The plugin's module, evaluated (top-level code, then its default export).
        let declared = Module::declare(ctx.clone(), "script.js", source).catch(ctx).map_err(|e| fail("syntax", exception_text(ctx, e)))?;
        let (module, done) = declared.eval().catch(ctx).map_err(|e| fail("syntax", exception_text(ctx, e)))?;
        while ctx.execute_pending_job() {}
        match done.state() {
            rquickjs::promise::PromiseState::Resolved => {}
            rquickjs::promise::PromiseState::Rejected => {
                let r: rquickjs::Result<Value> = done.result::<rquickjs::Value>().map(|r| r.map(|_| Value::Null)).unwrap_or(Ok(Value::Null));
                let msg = match r.catch(ctx) {
                    Err(e) => exception_text(ctx, e),
                    Ok(_) => "it threw".into(),
                };
                return Err(fail("syntax", msg));
            }
            rquickjs::promise::PromiseState::Pending => return Err(budget.tripped_error().unwrap_or_else(RunError::stalled)),
        }
        let main: rquickjs::Value = module.get("default").catch(ctx).map_err(|e| fail("syntax", exception_text(ctx, e)))?;
        let start: Function = runner.get("start").map_err(js)?;
        let started: String = start.call((main,)).catch(ctx).map_err(|e| fail("exception", exception_text(ctx, e)))?;
        if started == "nofn" {
            return Err(RunError::new("syntax", "The script must export a default function: export default async function (ctx) { ... }"));
        }
        let poll: Function = runner.get("poll").map_err(js)?;
        let settle: Function = runner.get("settle").map_err(js)?;
        loop {
            while ctx.execute_pending_job() {
                if budget.tripped.get().is_some() {
                    break;
                }
            }
            if let Some(e) = budget.tripped_error() {
                return Err(e);
            }
            let out: String = poll.call(()).catch(ctx).map_err(|e| fail("exception", exception_text(ctx, e)))?;
            if !out.is_empty() {
                let v: Value = serde_json::from_str(&out).map_err(|_| RunError::new("result", "The result isn't plain data (JSON)"))?;
                if out.len() > MAX_RESULT {
                    return Err(RunError::new("result", "The result is larger than 256 KB"));
                }
                if let Some(e) = v.get("error") {
                    let msg = e["message"].as_str().unwrap_or("").to_string();
                    let name = e["name"].as_str().unwrap_or("Error");
                    let text = if msg.is_empty() { name.to_string() } else { msg };
                    return Err(fail("exception", text));
                }
                if let Some(m) = v.get("invalid").and_then(|m| m.as_str()) {
                    return Err(RunError::new("result", format!("The script's result isn't valid: {m}")));
                }
                let output = validate(&v["value"]).map_err(|m| RunError::new("result", format!("The script's result isn't valid: {m}")))?;
                let store = check_store(&v["store"]).map_err(|m| RunError::new("result", m))?;
                return Ok((output, store));
            }
            let batch: Vec<(u64, Request, Vec<String>)> = std::mem::take(&mut state.borrow_mut().pending);
            if batch.is_empty() {
                return Err(RunError::stalled());
            }
            // Waiting for the network isn't JavaScript time.
            budget.pause();
            let left = WALL_LIMIT.saturating_sub(budget.wall());
            let limit = left.min(FETCH_TIMEOUT);
            let reqs: Vec<Request> = batch.iter().map(|b| b.1.clone()).collect();
            state.borrow_mut().requests.extend(reqs.iter().cloned());
            let measured = Instant::now();
            let answers = fetcher.batch(&reqs, &input.hosts, limit);
            let took = answers.iter().map(|a| a.1).max().unwrap_or_default();
            budget.extra.set(budget.extra.get() + took.saturating_sub(measured.elapsed()));
            budget.resume();
            if took >= left && left < FETCH_TIMEOUT || budget.wall() > WALL_LIMIT {
                return Err(RunError::wall());
            }
            for ((id, _, used), (answer, _)) in batch.iter().zip(answers) {
                let payload = match answer {
                    Ok(r) => {
                        let headers: Map<String, Value> = r.headers.iter().map(|(k, v)| (k.clone(), json!(redact(v, used)))).collect();
                        json!({"status": r.status, "url": redact(&r.url, used), "headers": headers, "body": redact(&String::from_utf8_lossy(&r.body), used)})
                    }
                    Err(f) => json!({"error": f.code, "message": f.message}),
                };
                let _: rquickjs::Value = settle.call((*id as f64, payload.to_string())).catch(ctx).map_err(|e| fail("exception", exception_text(ctx, e)))?;
            }
        }
    }
}

// ---------------------------------------------------------------- fetchers

/// The real network: the run's own client (no cookies, Rattleboard's user agent, redirects
/// followed by hand so every hop is checked against the scheme rule and the allow-list).
pub struct NetFetcher {
    pub handle: tokio::runtime::Handle,
}

fn net_client() -> &'static reqwest::Client {
    static C: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(FETCH_TIMEOUT)
            .user_agent(plugins::USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client")
    })
}

async fn fetch_one(req: Request, hosts: Vec<String>) -> Result<Response, FetchFail> {
    let c = net_client();
    let (mut method, mut url, mut headers, mut body) = (req.method, req.url, req.headers, req.body);
    for _ in 0..=5 {
        let u = url::Url::parse(&url).map_err(|_| FetchFail::new("bad_url", "Not a valid http(s) address"))?;
        let m = reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET);
        let mut rb = c.request(m, u.clone());
        for (k, v) in &headers {
            rb = rb.header(k.as_str(), v.as_str());
        }
        if let Some(b) = &body {
            rb = rb.body(b.clone());
        }
        let mut res = rb.send().await.map_err(net_error)?;
        let status = res.status().as_u16();
        if (300..400).contains(&status) && status != 304 {
            let loc = res.headers().get("location").and_then(|l| l.to_str().ok()).ok_or_else(FetchFail::redirect)?;
            let next = u.join(loc).map_err(|_| FetchFail::redirect())?;
            let host = next.host_str().unwrap_or("").to_string();
            if plugins::check_scheme(&next).is_err() || !host_allowed(&hosts, &host) || !next.username().is_empty() {
                return Err(FetchFail::redirect());
            }
            if next.host_str() != u.host_str() {
                // Another host: none of the script's headers (they may hold a key) go along.
                headers.clear();
            }
            if status == 303 || ((status == 301 || status == 302) && method == "POST") {
                method = "GET".into();
                body = None;
            }
            url = next.to_string();
            continue;
        }
        if res.content_length().is_some_and(|n| n as usize > MAX_RESPONSE) {
            return Err(FetchFail::too_large());
        }
        let out_headers: Vec<(String, String)> = res
            .headers()
            .iter()
            .filter(|(k, _)| k.as_str() != "set-cookie")
            .map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned()))
            .fold(Vec::<(String, String)>::new(), |mut acc, (k, v)| {
                match acc.iter_mut().find(|(n, _)| *n == k) {
                    Some(e) => {
                        e.1.push_str(", ");
                        e.1.push_str(&v);
                    }
                    None => acc.push((k, v)),
                }
                acc
            });
        let mut bytes = vec![];
        while let Some(chunk) = res.chunk().await.map_err(net_error)? {
            if bytes.len() + chunk.len() > MAX_RESPONSE {
                return Err(FetchFail::too_large());
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(Response { status, url, headers: out_headers, body: bytes });
    }
    Err(FetchFail::new("redirect_not_allowed", "Too many redirects"))
}

fn net_error(e: reqwest::Error) -> FetchFail {
    if e.is_timeout() {
        FetchFail::timeout()
    } else if plugins::is_certificate_error(&e) {
        FetchFail::new("network", "The server's certificate isn't trusted on this computer")
    } else if e.is_connect() {
        FetchFail::new("network", "Couldn't connect to the server")
    } else {
        FetchFail::new("network", "Network error")
    }
}

impl Fetcher for NetFetcher {
    fn batch(&self, requests: &[Request], hosts: &[String], limit: Duration) -> Vec<(Result<Response, FetchFail>, Duration)> {
        let tasks: Vec<_> = requests
            .iter()
            .map(|r| {
                let (r, h) = (r.clone(), hosts.to_vec());
                self.handle.spawn(async move {
                    let t = std::time::Instant::now();
                    let res = match tokio::time::timeout(limit, fetch_one(r, h)).await {
                        Ok(r) => r,
                        Err(_) => Err(FetchFail::timeout()),
                    };
                    (res, t.elapsed())
                })
            })
            .collect();
        self.handle.block_on(async move {
            let mut out = vec![];
            for t in tasks {
                out.push(t.await.unwrap_or((Err(FetchFail::new("network", "Network error")), Duration::ZERO)));
            }
            out
        })
    }
}

/// Canned answers, for tests and the conformance vectors: matched by method and exact URL
/// (after secrets are filled in). `delay_ms` is pretend time; nothing sleeps.
#[derive(Clone, Debug, Default)]
pub struct MockFetcher {
    pub routes: Vec<MockRoute>,
}

#[derive(Clone, Debug, Default)]
pub struct MockRoute {
    pub method: String,
    pub url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub delay_ms: u64,
    /// "network", "timeout" and so on: fail like that instead of answering
    pub fail: Option<String>,
}

impl MockFetcher {
    /// From a conformance vector's "fetch" list.
    pub fn from_json(v: &Value) -> MockFetcher {
        let routes = v
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| {
                let res = &r["response"];
                MockRoute {
                    method: r["method"].as_str().unwrap_or("GET").to_ascii_uppercase(),
                    url: r["url"].as_str().unwrap_or("").to_string(),
                    status: res["status"].as_u64().unwrap_or(200) as u16,
                    headers: res["headers"].as_object().map(|h| h.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.as_str().unwrap_or("").to_string())).collect()).unwrap_or_default(),
                    body: match &res["body"] {
                        Value::String(s) => s.clone(),
                        Value::Null => String::new(),
                        other => other.to_string(),
                    },
                    delay_ms: res["delayMs"].as_u64().unwrap_or(0),
                    fail: res["fail"].as_str().map(str::to_string),
                }
            })
            .collect();
        MockFetcher { routes }
    }
}

impl Fetcher for MockFetcher {
    fn batch(&self, requests: &[Request], _hosts: &[String], limit: Duration) -> Vec<(Result<Response, FetchFail>, Duration)> {
        requests
            .iter()
            .map(|r| {
                let Some(m) = self.routes.iter().find(|m| m.method == r.method && m.url == r.url) else {
                    return (Err(FetchFail::new("network", "Couldn't connect to the server")), Duration::ZERO);
                };
                let took = Duration::from_millis(m.delay_ms);
                if took > limit {
                    return (Err(FetchFail::timeout()), limit);
                }
                let res = match m.fail.as_deref() {
                    Some("timeout") => Err(FetchFail::timeout()),
                    Some("too_large") => Err(FetchFail::too_large()),
                    Some("redirect_not_allowed") => Err(FetchFail::redirect()),
                    Some(_) => Err(FetchFail::new("network", "Couldn't connect to the server")),
                    None if m.body.len() > MAX_RESPONSE => Err(FetchFail::too_large()),
                    None => Ok(Response { status: m.status, url: r.url.clone(), headers: m.headers.clone(), body: m.body.clone().into_bytes() }),
                };
                (res, took)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
