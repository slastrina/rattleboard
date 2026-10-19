//! Plugins (protocol v3): what a source is and how it gets its data.
//!
//! A plugin is a `.rattleboard-plugin.json` manifest (format 1), or one of the built-ins.
//!
//!   {"format": 1, "id": "com.example.hn", "name": "Hacker News", "kind": "rss",
//!    "fetch": {"url": "https://hnrss.org/frontpage", "every": "15m"},
//!    "card": {"chip": "orange"}, "settings": [...], "secrets": [{"key": "token", "label": "API token"}]}
//!
//! Kinds: rss, json and ical fetch `fetch.url` (HTTPS, or HTTP to a private-network host;
//! 10 s timeout, 2 MB cap); text is a card made of placeholders alone; command runs a program
//! on this computer, only once the user has approved it here (never through the API or a link).
//! Placeholders ({{settings.city}}, {{secret.token}}, {{title}}, {{now:ddd d MMM}}...) fill in
//! the URL, headers and card template. Secret values come from the credential store and appear
//! only in the URL and headers; they're never logged or shown.
//!
//! Unknown fields are ignored, so newer manifests still load; anything invalid is refused with
//! a message saying what's wrong.

use crate::vestaboard::Grid;
use chrono::{Datelike, NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::time::Duration;

pub const FORMAT: u64 = 1;
pub const USER_KINDS: [&str; 5] = ["rss", "json", "ical", "text", "command"];
pub const MAX_MANIFEST: usize = 64 * 1024;
pub const MAX_RESPONSE: usize = 2 * 1024 * 1024;
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_STDOUT: usize = 64 * 1024;
/// Ids starting with this are Rattleboard's own built-ins.
pub const BUILTIN_PREFIX: &str = "rattleboard.";
pub const SETTING_TYPES: [&str; 7] = ["text", "number", "toggle", "select", "date", "time", "lines"];
pub const CHIPS: [(&str, u8); 8] = [("red", 63), ("orange", 64), ("yellow", 65), ("green", 66), ("blue", 67), ("violet", 68), ("white", 69), ("black", 70)];
const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

// ---------------------------------------------------------------- manifest

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub description: String,
    /// rss | json | ical | text | command, or for built-ins "native"
    pub kind: String,
    pub fetch: Option<Fetch>,
    pub run: Option<Run>,
    pub pick: Pick,
    pub card: CardSpec,
    pub alert: bool,
    /// Lower-cased keywords from "alertWhen": ["keyword:launch"]
    pub alert_words: Vec<String>,
    pub max_items: usize,
    /// Seconds a card stays up, if the manifest says
    pub dwell: Option<u64>,
    /// Seconds between fetches (or runs, or re-renders of a text card)
    pub every: u64,
    pub schedule: Option<Schedule>,
    pub settings: Vec<SettingSpec>,
    pub secrets: Vec<SecretSpec>,
    pub builtin: bool,
    /// The manifest as installed, for the API and Export
    pub raw: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fetch {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    /// Format 1.1: an HMAC signature appended to the URL as a query parameter
    pub sign: Option<Sign>,
}

/// `fetch.sign`: an HMAC over the URL's path and query (as sent), in hex, appended as one
/// more query parameter. PTV's Timetable API v3 signs requests this way.
#[derive(Clone, Debug, PartialEq)]
pub struct Sign {
    /// "hmac-sha1" or "hmac-sha256"
    pub kind: String,
    /// A template, usually {{secret.key}}
    pub key: String,
    pub param: String,
    pub upper: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Run {
    pub path: String,
    pub args: Vec<String>,
    /// Seconds
    pub timeout: u64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Pick {
    pub items: Option<String>,
    pub id: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub time: Option<String>,
    /// Format 1.1: the item's web address (http or https), for opening from the board
    pub link: Option<String>,
    /// Format 1.1: extra values for templates ({{field.name}}), each the first of up to five
    /// paths that has something
    pub fields: Vec<(String, Vec<String>)>,
    /// Format 1.1: conditions every item must meet (pick.where)
    pub conds: Vec<Cond>,
    pub sort: Option<SortSpec>,
    /// Format 1.1: "pick.as": "status": the list is one status card, not a feed
    pub status: bool,
}

/// One `pick.where` condition.
#[derive(Clone, Debug, PartialEq)]
pub struct Cond {
    pub path: String,
    /// == != > >= < <= contains exists missing
    pub op: String,
    /// Null for exists and missing
    pub value: Value,
    /// Applies only while setting `.0` is one of `.1`
    pub when: Option<(String, Vec<String>)>,
}

/// `pick.sort`. `numeric` None is automatic: numeric when every key that's there is a number.
#[derive(Clone, Debug, PartialEq)]
pub struct SortSpec {
    pub path: String,
    pub desc: bool,
    pub numeric: Option<bool>,
}

pub const OPS: [&str; 9] = ["==", "!=", ">", ">=", "<", "<=", "contains", "exists", "missing"];
pub const FILTERS: [&str; 9] = ["upper", "lower", "trim", "num", "round", "default", "pad", "time", "mins"];
pub const MAX_FIELDS: usize = 20;
pub const MAX_CONDS: usize = 20;

#[derive(Clone, Debug, PartialEq)]
pub struct CardSpec {
    pub template: Option<String>,
    pub chip: Option<u8>,
    pub left: bool,
    pub pages: usize,
    /// Format 1.1: a status list's line for each item, joined into {{rows}}
    pub row: Option<String>,
    /// Format 1.1: what a status list says when no item is left
    pub empty: Option<String>,
}

impl Default for CardSpec {
    fn default() -> Self {
        Self { template: None, chip: None, left: false, pages: 1, row: None, empty: None }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SettingSpec {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub default: Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SecretSpec {
    pub key: String,
    pub label: String,
}

/// When a source may show: on these days (Monday first; empty is every day), between
/// `from` and `to` (minutes since midnight; may wrap past midnight, in which case the days
/// are the days the window starts).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Schedule {
    pub days: Vec<usize>,
    pub from: Option<u32>,
    pub to: Option<u32>,
}

impl Schedule {
    pub fn parse(v: &Value) -> Result<Option<Schedule>, String> {
        let o = match v {
            Value::Null => return Ok(None),
            Value::Object(o) => o,
            _ => return Err("schedule must be an object".into()),
        };
        let mut s = Schedule::default();
        if let Some(days) = o.get("days").filter(|d| !d.is_null()) {
            for d in days.as_array().ok_or("schedule.days must be a list like [\"mon\", \"tue\"]")? {
                let d = d.as_str().map(|d| d.to_ascii_lowercase()).unwrap_or_default();
                let i = DAYS.iter().position(|x| d.starts_with(x)).ok_or(format!("Unknown day {d:?}; use mon, tue, wed, thu, fri, sat, sun"))?;
                if !s.days.contains(&i) {
                    s.days.push(i);
                }
            }
            s.days.sort();
        }
        let time = |k: &str| -> Result<Option<u32>, String> {
            match o.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(t)) if t.trim().is_empty() => Ok(None),
                Some(Value::String(t)) => minutes(t).map(Some).ok_or(format!("schedule.{k} must be HH:MM")),
                _ => Err(format!("schedule.{k} must be HH:MM")),
            }
        };
        s.from = time("from")?;
        s.to = time("to")?;
        if s.from.is_some() != s.to.is_some() {
            return Err("A schedule needs both from and to, or neither".into());
        }
        Ok(Some(s))
    }

    pub fn to_json(&self) -> Value {
        let hm = |m: Option<u32>| m.map(|m| format!("{:02}:{:02}", m / 60, m % 60));
        json!({ "days": self.days.iter().map(|d| DAYS[*d]).collect::<Vec<_>>(), "from": hm(self.from), "to": hm(self.to) })
    }

    pub fn active(&self, at: NaiveDateTime) -> bool {
        let today = at.weekday().num_days_from_monday() as usize;
        let day_ok = |d: usize| self.days.is_empty() || self.days.contains(&d);
        let now = at.hour() * 60 + at.minute();
        match (self.from, self.to) {
            (Some(f), Some(t)) if f < t => day_ok(today) && (f..t).contains(&now),
            (Some(f), Some(t)) if f > t => (now >= f && day_ok(today)) || (now < t && day_ok((today + 6) % 7)),
            // the same time twice: all day
            _ => day_ok(today),
        }
    }

    /// "Weekdays 07:00–10:00", for Up next.
    pub fn describe(&self) -> String {
        let days = match self.days.as_slice() {
            [] => String::new(),
            [0, 1, 2, 3, 4] => "Weekdays".into(),
            [5, 6] => "Weekends".into(),
            d => d.iter().map(|d| capitalise(DAYS[*d])).collect::<Vec<_>>().join(" "),
        };
        let hm = |m: u32| format!("{:02}:{:02}", m / 60, m % 60);
        let time = match (self.from, self.to) {
            (Some(f), Some(t)) if f != t => format!("{}–{}", hm(f), hm(t)),
            _ => String::new(),
        };
        [days, time].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Minutes since midnight for "HH:MM".
pub fn minutes(hhmm: &str) -> Option<u32> {
    let (h, m) = hhmm.trim().split_once(':')?;
    let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// "30s", "15m", "1h", "1h30m", "90" (seconds) or a number of seconds.
pub fn duration(v: &Value) -> Result<u64, String> {
    match v {
        Value::Number(n) => n.as_u64().ok_or_else(|| "Durations are whole seconds".to_string()),
        Value::String(s) => {
            let s = s.trim().to_ascii_lowercase();
            if let Ok(n) = s.parse::<u64>() {
                return Ok(n);
            }
            let (mut total, mut num) = (0u64, String::new());
            for c in s.chars() {
                if c.is_ascii_digit() {
                    num.push(c);
                    continue;
                }
                let n: u64 = num.parse().map_err(|_| format!("Unreadable duration {s:?}; use e.g. 30s, 15m or 1h"))?;
                num.clear();
                total += n * match c {
                    's' => 1,
                    'm' => 60,
                    'h' => 3600,
                    'd' => 86400,
                    _ => return Err(format!("Unreadable duration {s:?}; use e.g. 30s, 15m or 1h")),
                };
            }
            if !num.is_empty() || total == 0 && !s.starts_with('0') {
                return Err(format!("Unreadable duration {s:?}; use e.g. 30s, 15m or 1h"));
            }
            Ok(total)
        }
        _ => Err("Durations are like \"30s\", \"15m\" or \"1h\"".into()),
    }
}

/// "orange" or 64, as a chip's flap code.
pub fn chip(v: &Value) -> Result<Option<u8>, String> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) if s.is_empty() || s == "none" => Ok(None),
        Value::String(s) => CHIPS.iter().find(|c| c.0 == s.to_ascii_lowercase()).map(|c| Some(c.1)).ok_or(format!("Unknown chip colour {s:?}; use red, orange, yellow, green, blue, violet, white or black")),
        Value::Number(n) => match n.as_u64() {
            Some(c @ 63..=70) => Ok(Some(c as u8)),
            _ => Err("A chip code must be from 63 to 70".into()),
        },
        _ => Err("chip must be a colour name".into()),
    }
}

pub fn chip_name(c: u8) -> &'static str {
    CHIPS.iter().find(|x| x.1 == c).map(|x| x.0).unwrap_or("")
}

fn opt_str<'a>(o: &'a Map<String, Value>, k: &str, what: &str) -> Result<Option<&'a str>, String> {
    match o.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("{what} must be a string")),
    }
}

fn valid_key(k: &str) -> bool {
    !k.is_empty() && k.len() <= 40 && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Validates a manifest. `builtin` allows Rattleboard's own ids and native kinds.
pub fn parse(v: &Value, builtin: bool) -> Result<Manifest, String> {
    let o = v.as_object().ok_or("A plugin manifest is a JSON object")?;
    if serde_json::to_vec(v).map(|b| b.len()).unwrap_or(0) > MAX_MANIFEST {
        return Err("The manifest is too large (64 KB at most)".into());
    }
    match o.get("format").and_then(|f| f.as_u64()) {
        Some(FORMAT) => {}
        Some(n) => return Err(format!("This manifest is format {n}; this Rattleboard reads format {FORMAT}")),
        None => return Err("The manifest needs \"format\": 1".into()),
    }
    let id = opt_str(o, "id", "id")?.unwrap_or("").trim().to_string();
    if id.len() < 3 || id.len() > 100 || !id.chars().all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c)) {
        return Err("id must be 3–100 letters, digits, dots, dashes or underscores, like com.example.news".into());
    }
    if id.starts_with(BUILTIN_PREFIX) != builtin {
        return Err(format!("Ids starting with {BUILTIN_PREFIX} are kept for Rattleboard's built-ins"));
    }
    let name = opt_str(o, "name", "name")?.unwrap_or("").trim().to_string();
    if name.is_empty() || name.chars().count() > 60 {
        return Err("name must be 1 to 60 characters".into());
    }
    let description = opt_str(o, "description", "description")?.unwrap_or("").trim().chars().take(300).collect();
    let kind = opt_str(o, "kind", "kind")?.unwrap_or("").to_string();
    if !USER_KINDS.contains(&kind.as_str()) && !(builtin && kind == "native") {
        return Err(format!("kind must be one of {}", USER_KINDS.join(", ")));
    }
    let http = ["rss", "json", "ical"].contains(&kind.as_str());

    let fetch_o = match o.get("fetch") {
        None | Some(Value::Null) => None,
        Some(Value::Object(f)) => Some(f),
        Some(_) => return Err("fetch must be an object".into()),
    };
    let fetch = match (http, fetch_o) {
        (false, _) => None,
        (true, None) => return Err(format!("A {kind} plugin needs fetch.url")),
        (true, Some(f)) => {
            let url = opt_str(f, "url", "fetch.url")?.unwrap_or("").trim().to_string();
            if url.is_empty() {
                return Err(format!("A {kind} plugin needs fetch.url"));
            }
            if url.len() > 2000 {
                return Err("fetch.url is too long".into());
            }
            // A literal URL can be checked now; one built from placeholders, when it's fetched.
            if !url.starts_with("{{") {
                let u = url::Url::parse(&strip_placeholders(&url)).map_err(|_| "fetch.url isn't a valid URL")?;
                check_scheme(&u)?;
            }
            let method = opt_str(f, "method", "fetch.method")?.unwrap_or("GET").to_ascii_uppercase();
            if method != "GET" && method != "POST" {
                return Err("fetch.method must be GET or POST".into());
            }
            let mut headers = vec![];
            match f.get("headers") {
                None | Some(Value::Null) => {}
                Some(Value::Object(h)) => {
                    if h.len() > 20 {
                        return Err("At most 20 fetch.headers".into());
                    }
                    for (k, v) in h {
                        let v = v.as_str().ok_or(format!("Header {k} must be a string"))?;
                        if reqwest::header::HeaderName::from_bytes(k.as_bytes()).is_err() || v.len() > 4000 {
                            return Err(format!("Header {k} isn't valid"));
                        }
                        headers.push((k.clone(), v.to_string()));
                    }
                }
                Some(_) => return Err("fetch.headers must be an object of strings".into()),
            }
            for (_, v) in &headers {
                check_filters(v, "fetch.headers")?;
            }
            check_filters(&url, "fetch.url")?;
            let sign = match f.get("sign") {
                None | Some(Value::Null) => None,
                Some(Value::Object(g)) => Some(parse_sign(g)?),
                Some(_) => return Err("fetch.sign must be an object".into()),
            };
            Some(Fetch { url, method, headers, sign })
        }
    };

    let run = if kind == "command" {
        let r = o.get("run").and_then(|r| r.as_object()).ok_or("A command plugin needs \"run\": {\"path\": \"/abs/path\", \"args\": [...]}")?;
        let path = opt_str(r, "path", "run.path")?.unwrap_or("").to_string();
        if path.is_empty() || !std::path::Path::new(&path).is_absolute() {
            return Err("run.path must be the program's full path, like /usr/bin/git".into());
        }
        let args = match r.get("args") {
            None | Some(Value::Null) => vec![],
            Some(Value::Array(a)) if a.len() <= 50 => a.iter().map(|x| x.as_str().map(str::to_string).ok_or("run.args must be strings")).collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err("run.args must be a list of at most 50 strings".into()),
        };
        let timeout = match r.get("timeout") {
            None | Some(Value::Null) => 10,
            Some(t) => duration(t)?,
        };
        if !(1..=60).contains(&timeout) {
            return Err("run.timeout must be from 1s to 60s".into());
        }
        Some(Run { path, args, timeout })
    } else {
        None
    };

    // How often: fetch.every, run.every, or for text cards "every" at the top level.
    let every_v = fetch_o.and_then(|f| f.get("every")).or_else(|| o.get("run").and_then(|r| r.get("every"))).or_else(|| o.get("every")).filter(|v| !v.is_null());
    let (min_every, def_every) = match kind.as_str() {
        "text" | "command" | "native" => (10, if kind == "command" { 300 } else { 60 }),
        _ => (60, 900),
    };
    let every = match every_v {
        None => def_every,
        Some(v) => duration(v)?,
    };
    if every < min_every || every > 7 * 86400 {
        return Err(format!("every must be at least {} for a {kind} plugin", if min_every == 60 { "1m" } else { "10s" }));
    }

    let mut pick = Pick::default();
    match o.get("pick") {
        None | Some(Value::Null) => {}
        Some(Value::Object(p)) => {
            let path = |k: &str| -> Result<Option<String>, String> {
                let s = opt_str(p, k, &format!("pick.{k}"))?.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                if let Some(s) = &s {
                    if !s.contains("{{") {
                        parse_path(s).map_err(|e| format!("pick.{k}: {e}"))?;
                    }
                }
                Ok(s)
            };
            pick = Pick { items: path("items")?, id: path("id")?, title: path("title")?, body: path("body")?, time: path("time")?, link: path("link")?, ..Default::default() };
            pick.fields = parse_fields(p.get("fields"))?;
            pick.conds = parse_where(p.get("where"))?;
            pick.sort = parse_sort(p.get("sort"))?;
            pick.status = match opt_str(p, "as", "pick.as")? {
                None | Some("feed") => false,
                Some("status") => true,
                Some(a) => return Err(format!("pick.as {a:?} must be feed or status")),
            };
            let extra = !pick.fields.is_empty() || !pick.conds.is_empty() || pick.sort.is_some() || pick.status || pick.link.is_some();
            if extra && kind != "json" {
                return Err("pick.fields, pick.where, pick.sort, pick.as and pick.link are for json plugins".into());
            }
            if (!pick.conds.is_empty() || pick.sort.is_some() || pick.status) && pick.items.is_none() {
                return Err("pick.where, pick.sort and pick.as need pick.items (the list)".into());
            }
        }
        Some(_) => return Err("pick must be an object".into()),
    }
    if kind == "json" && pick.title.is_none() && card_template(o).is_none() {
        return Err("A json plugin needs pick.title (a path like $.title) or a card.template".into());
    }

    let mut card = CardSpec::default();
    match o.get("card") {
        None | Some(Value::Null) => {}
        Some(Value::Object(c)) => {
            card.template = opt_str(c, "template", "card.template")?.map(str::to_string).filter(|t| !t.trim().is_empty());
            if let Some(t) = &card.template {
                if t.len() > 2000 {
                    return Err("card.template is too long".into());
                }
                if t.contains("{{secret.") {
                    return Err("Secrets can only be used in fetch.url and fetch.headers, never in card.template".into());
                }
                check_filters(t, "card.template")?;
            }
            for (k, slot) in [("row", &mut card.row), ("empty", &mut card.empty)] {
                *slot = opt_str(c, k, &format!("card.{k}"))?.map(str::to_string).filter(|t| !t.trim().is_empty());
                if let Some(t) = slot {
                    if t.len() > 2000 {
                        return Err(format!("card.{k} is too long"));
                    }
                    if t.contains("{{secret.") {
                        return Err(format!("Secrets can only be used in fetch.url and fetch.headers, never in card.{k}"));
                    }
                    check_filters(t, &format!("card.{k}"))?;
                }
            }
            card.chip = chip(c.get("chip").unwrap_or(&Value::Null))?;
            card.left = match opt_str(c, "align", "card.align")? {
                None | Some("center") | Some("centre") => false,
                Some("left") => true,
                Some(a) => return Err(format!("card.align {a:?} must be center or left")),
            };
            card.pages = match c.get("pages") {
                None | Some(Value::Null) => 1,
                Some(p) => match p.as_u64() {
                    Some(n @ 1..=3) => n as usize,
                    _ => return Err("card.pages must be 1, 2 or 3".into()),
                },
            };
        }
        Some(_) => return Err("card must be an object".into()),
    }

    let alert = match opt_str(o, "priority", "priority")? {
        None | Some("ambient") => false,
        Some("alert") => true,
        Some(p) => return Err(format!("priority {p:?} must be ambient or alert")),
    };
    let mut alert_words = vec![];
    match o.get("alertWhen") {
        None | Some(Value::Null) => {}
        Some(Value::Array(a)) if a.len() <= 50 => {
            for w in a {
                let w = w.as_str().unwrap_or("");
                let k = w.strip_prefix("keyword:").map(|k| k.trim().to_lowercase()).filter(|k| !k.is_empty());
                alert_words.push(k.ok_or(format!("alertWhen entries look like \"keyword:launch\", not {w:?}"))?);
            }
        }
        Some(_) => return Err("alertWhen must be a list like [\"keyword:launch\"]".into()),
    }
    let max_items = match o.get("maxItems") {
        None | Some(Value::Null) => 10,
        Some(n) => match n.as_u64() {
            Some(n @ 1..=50) => n as usize,
            _ => return Err("maxItems must be from 1 to 50".into()),
        },
    };
    let dwell = match o.get("dwell") {
        None | Some(Value::Null) => None,
        Some(d) => {
            let d = duration(d)?;
            if !(10..=3600).contains(&d) {
                return Err("dwell must be from 10s to 1h".into());
            }
            Some(d)
        }
    };
    let schedule = Schedule::parse(o.get("schedule").unwrap_or(&Value::Null))?;

    let mut settings: Vec<SettingSpec> = vec![];
    if let Some(list) = o.get("settings").filter(|v| !v.is_null()) {
        let list = list.as_array().filter(|a| a.len() <= 30).ok_or("settings must be a list of at most 30")?;
        for s in list {
            let s = s.as_object().ok_or("each setting must be an object")?;
            let key = opt_str(s, "key", "settings key")?.unwrap_or("").to_string();
            if !valid_key(&key) || settings.iter().any(|x| x.key == key) {
                return Err(format!("Setting key {key:?} must be unique letters, digits or _"));
            }
            let label = opt_str(s, "label", "settings label")?.unwrap_or(&key).chars().take(60).collect();
            let kind = opt_str(s, "type", "settings type")?.unwrap_or("text").to_string();
            if !SETTING_TYPES.contains(&kind.as_str()) {
                return Err(format!("Setting {key}: type must be one of {}", SETTING_TYPES.join(", ")));
            }
            let options: Vec<Value> = match s.get("options") {
                None | Some(Value::Null) => vec![],
                Some(Value::Array(a)) => a.clone(),
                Some(_) => return Err(format!("Setting {key}: options must be a list")),
            };
            if kind == "select" && options.is_empty() {
                return Err(format!("Setting {key}: a select needs options"));
            }
            let mut spec = SettingSpec { key: key.clone(), label, kind, default: Value::Null, options };
            let default = s.get("default").cloned().unwrap_or(Value::Null);
            spec.default = if default.is_null() { spec.empty() } else { spec.check(&default).map_err(|e| format!("Setting {key}: {e}"))? };
            settings.push(spec);
        }
    }
    let mut secrets: Vec<SecretSpec> = vec![];
    if let Some(list) = o.get("secrets").filter(|v| !v.is_null()) {
        let list = list.as_array().filter(|a| a.len() <= 10).ok_or("secrets must be a list of at most 10")?;
        for s in list {
            let key = s.get("key").and_then(|k| k.as_str()).unwrap_or("").to_string();
            if !valid_key(&key) || secrets.iter().any(|x| x.key == key) || settings.iter().any(|x| x.key == key) {
                return Err(format!("Secret key {key:?} must be unique letters, digits or _"));
            }
            let label = s.get("label").and_then(|l| l.as_str()).unwrap_or(&key).chars().take(60).collect();
            secrets.push(SecretSpec { key, label });
        }
    }
    for c in &pick.conds {
        if let Some((k, _)) = &c.when {
            if !settings.iter().any(|s| &s.key == k) {
                return Err(format!("pick.where: when.setting {k:?} isn't one of the settings"));
            }
        }
    }
    if (card.row.is_some() || card.empty.is_some()) && !pick.status {
        return Err("card.row and card.empty are for \"pick\": {\"as\": \"status\"}".into());
    }
    Ok(Manifest { id, name, description, kind, fetch, run, pick, card, alert, alert_words, max_items, dwell, every, schedule, settings, secrets, builtin, raw: v.clone() })
}

fn parse_sign(g: &Map<String, Value>) -> Result<Sign, String> {
    let kind = opt_str(g, "type", "fetch.sign.type")?.unwrap_or("").to_string();
    if kind != "hmac-sha1" && kind != "hmac-sha256" {
        return Err("fetch.sign.type must be hmac-sha1 or hmac-sha256".into());
    }
    let key = opt_str(g, "key", "fetch.sign.key")?.unwrap_or("").trim().to_string();
    if key.is_empty() || key.len() > 500 {
        return Err("fetch.sign.key must be the signing key, like {{secret.key}}".into());
    }
    match opt_str(g, "over", "fetch.sign.over")? {
        None | Some("path+query") => {}
        Some(o) => return Err(format!("fetch.sign.over {o:?} must be path+query")),
    }
    let param = opt_str(g, "param", "fetch.sign.param")?.unwrap_or("signature").to_string();
    if param.is_empty() || param.len() > 40 || !param.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)) {
        return Err("fetch.sign.param must be 1–40 letters, digits, _ . or -".into());
    }
    let upper = match opt_str(g, "case", "fetch.sign.case")? {
        None | Some("upper") => true,
        Some("lower") => false,
        Some(c) => return Err(format!("fetch.sign.case {c:?} must be upper or lower")),
    };
    Ok(Sign { kind, key, param, upper })
}

/// A pick path: checked now unless it's made from placeholders.
fn check_path(s: &str, what: &str) -> Result<String, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err(format!("{what} needs a path like $.name"));
    }
    if !s.contains("{{") {
        parse_path(s).map_err(|e| format!("{what}: {e}"))?;
    }
    Ok(s.to_string())
}

fn parse_fields(v: Option<&Value>) -> Result<Vec<(String, Vec<String>)>, String> {
    let o = match v {
        None | Some(Value::Null) => return Ok(vec![]),
        Some(Value::Object(o)) if o.len() <= MAX_FIELDS => o,
        Some(_) => return Err(format!("pick.fields must be an object of at most {MAX_FIELDS} names and paths")),
    };
    let mut out = vec![];
    for (k, v) in o {
        if !valid_key(k) {
            return Err(format!("pick.fields name {k:?} must be letters, digits or _"));
        }
        let what = format!("pick.fields.{k}");
        let paths = match v {
            Value::String(s) => vec![check_path(s, &what)?],
            Value::Array(a) if (1..=5).contains(&a.len()) => a.iter().map(|p| p.as_str().ok_or(format!("{what} must be a path or a list of up to 5 paths")).and_then(|p| check_path(p, &what))).collect::<Result<_, _>>()?,
            _ => return Err(format!("{what} must be a path or a list of up to 5 paths")),
        };
        out.push((k.clone(), paths));
    }
    Ok(out)
}

fn parse_where(v: Option<&Value>) -> Result<Vec<Cond>, String> {
    let a = match v {
        None | Some(Value::Null) => return Ok(vec![]),
        Some(Value::Array(a)) if a.len() <= MAX_CONDS => a,
        Some(_) => return Err(format!("pick.where must be a list of at most {MAX_CONDS} conditions")),
    };
    let mut out = vec![];
    for c in a {
        let o = c.as_object().ok_or("each pick.where condition is an object like {\"path\": \"$.x\", \"op\": \"==\", \"value\": 1}")?;
        let path = check_path(opt_str(o, "path", "pick.where path")?.unwrap_or(""), "pick.where")?;
        let op = opt_str(o, "op", "pick.where op")?.unwrap_or("").trim().to_string();
        if !OPS.contains(&op.as_str()) {
            return Err(format!("pick.where op {op:?} must be one of {}", OPS.join(" ")));
        }
        let value = o.get("value").cloned().unwrap_or(Value::Null);
        let unary = op == "exists" || op == "missing";
        match &value {
            Value::Null if unary => {}
            Value::Null => return Err(format!("pick.where {op} needs a value")),
            _ if unary => return Err(format!("pick.where {op} takes no value")),
            Value::String(s) if s.len() > 500 => return Err("pick.where value is too long".into()),
            Value::String(s) if s.contains("{{secret.") => return Err("Secrets can't be used in pick.where".into()),
            Value::String(_) | Value::Number(_) | Value::Bool(_) => {}
            _ => return Err("pick.where value must be a string, number or true/false".into()),
        }
        if let Value::String(s) = &value {
            check_filters(s, "pick.where value")?;
        }
        let when = match o.get("when") {
            None | Some(Value::Null) => None,
            Some(Value::Object(w)) => {
                let k = opt_str(w, "setting", "pick.where when.setting")?.unwrap_or("").to_string();
                let is: Vec<String> = match w.get("is") {
                    Some(Value::String(s)) => vec![s.trim().to_string()],
                    Some(Value::Array(a)) if (1..=20).contains(&a.len()) => a.iter().map(|x| x.as_str().map(|s| s.trim().to_string()).ok_or("when.is must be strings")).collect::<Result<_, _>>()?,
                    _ => return Err("pick.where when.is must be a value or a list of values".into()),
                };
                Some((k, is))
            }
            Some(_) => return Err("pick.where when must be {\"setting\": \"key\", \"is\": [\"value\"]}".into()),
        };
        out.push(Cond { path, op, value, when });
    }
    Ok(out)
}

fn parse_sort(v: Option<&Value>) -> Result<Option<SortSpec>, String> {
    let o = match v {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Object(o)) => o,
        Some(_) => return Err("pick.sort must be {\"path\": \"$.x\", \"order\": \"asc\"}".into()),
    };
    let path = check_path(opt_str(o, "path", "pick.sort.path")?.unwrap_or(""), "pick.sort")?;
    let desc = match opt_str(o, "order", "pick.sort.order")? {
        None | Some("asc") => false,
        Some("desc") => true,
        Some(x) => return Err(format!("pick.sort.order {x:?} must be asc or desc")),
    };
    let numeric = match o.get("numeric") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err("pick.sort.numeric must be true or false".into()),
    };
    Ok(Some(SortSpec { path, desc, numeric }))
}

/// Every {{...|filter}} in a template names a known filter with a valid argument.
pub fn check_filters(t: &str, what: &str) -> Result<(), String> {
    let mut rest = t;
    while let Some(i) = rest.find("{{") {
        let after = &rest[i + 2..];
        let Some(j) = after.find("}}") else { break };
        let mut parts = after[..j].split('|');
        parts.next();
        for f in parts {
            let (name, arg) = match f.split_once(':') {
                Some((n, a)) => (n.trim(), Some(a)),
                None => (f.trim(), None),
            };
            let ok = match (name, arg) {
                ("upper" | "lower" | "trim" | "num" | "mins", None) => true,
                ("round", None) => true,
                ("round", Some(a)) => a.trim().parse::<u32>().is_ok_and(|n| n <= 6),
                ("default", Some(_)) => true,
                ("pad", Some(a)) => a.trim().parse::<i32>().is_ok_and(|n| n != 0 && (-40..=40).contains(&n)),
                ("time", _) => true,
                _ => false,
            };
            if !ok {
                return Err(if FILTERS.contains(&name) {
                    format!("{what}: |{f} has a missing or invalid argument")
                } else {
                    format!("{what}: unknown filter |{name}; use {}", FILTERS.join(", "))
                });
            }
        }
        rest = &after[j + 2..];
    }
    Ok(())
}

fn card_template(o: &Map<String, Value>) -> Option<&str> {
    o.get("card").and_then(|c| c.get("template")).and_then(|t| t.as_str())
}

impl SettingSpec {
    pub fn empty(&self) -> Value {
        match self.kind.as_str() {
            "toggle" => json!(false),
            "number" => json!(0),
            "lines" => json!([]),
            "select" => self.options.first().map(option_value).unwrap_or(Value::Null),
            _ => json!(""),
        }
    }

    /// Checks one value for this setting, tidying it (lines from text, trimmed strings).
    pub fn check(&self, v: &Value) -> Result<Value, String> {
        match (self.kind.as_str(), v) {
            ("toggle", Value::Bool(_)) | ("number", Value::Number(_)) => Ok(v.clone()),
            ("text", Value::String(s)) if s.len() <= 2000 => Ok(json!(s.trim())),
            ("date", Value::String(s)) if s.is_empty() || chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() => Ok(v.clone()),
            ("time", Value::String(s)) if s.is_empty() || minutes(s).is_some() => Ok(v.clone()),
            ("select", _) if self.options.iter().any(|o| option_value(o) == *v) => Ok(v.clone()),
            ("lines", Value::String(s)) => self.check(&json!(s.lines().collect::<Vec<_>>())),
            ("lines", Value::Array(a)) if a.len() <= 500 && a.iter().all(|l| l.as_str().is_some_and(|l| l.len() <= 500)) => {
                Ok(json!(a.iter().filter_map(|l| l.as_str()).map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>()))
            }
            ("select", _) => Err(format!("must be one of {}", self.options.iter().map(option_value).map(|v| v.to_string()).collect::<Vec<_>>().join(", "))),
            (k, _) => Err(format!("must be a {}", match k { "toggle" => "true or false", "number" => "number", "date" => "date like 2026-12-25", "time" => "time like 07:30", "lines" => "list of lines", _ => "string" })),
        }
    }
}

/// A select option is a value, or {"value": ..., "label": ...}.
pub fn option_value(o: &Value) -> Value {
    o.get("value").cloned().unwrap_or_else(|| o.clone())
}

// ---------------------------------------------------------------- items

/// One thing a source has to show: a feed entry, an event, a JSON item, a command's output.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    /// For dedupe: the entry's id, or a hash of what it says
    pub key: String,
    pub title: String,
    pub body: String,
    /// Milliseconds: published, or an event's start
    pub time: Option<i64>,
    /// An event's end
    pub end: Option<i64>,
    pub all_day: bool,
    /// The source asked for it to interrupt (a command's "priority": "alert")
    pub alert: bool,
    /// Exact characters (a command's {"characters"})
    pub characters: Option<Grid>,
    pub chip: Option<u8>,
    /// Format 1.1: pick.fields, by name ({{field.name}})
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub fields: std::collections::BTreeMap<String, String>,
    /// Its web address, checked by `safe_link`: a feed entry's link, an event's URL, pick.link
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

/// A card's link, if it's one the board may open: an absolute http:// or https:// address with
/// a host, no user name or password, at most 2048 characters. Returned normalised (as the
/// URL standard writes it); anything else (javascript:, file:, data:, mailto:, relative) is
/// None.
pub fn safe_link(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.len() > 2048 {
        return None;
    }
    let u = url::Url::parse(s).ok()?;
    let ok = matches!(u.scheme(), "http" | "https") && u.host_str().is_some_and(|h| !h.is_empty()) && u.username().is_empty() && u.password().is_none();
    ok.then(|| u.to_string()).filter(|l| l.len() <= 2048)
}

/// What a fetch produced. A feed (`single` false) has new items to show once each; a status
/// (`single` true: one JSON value, a command's output, the weather) is always shown as it is.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct Data {
    pub items: Vec<Item>,
    pub single: bool,
}

pub fn hash(parts: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0]);
    }
    h.finalize().iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// Text from the outside world, made safe and readable for the board: tags stripped,
/// entities decoded, typographic punctuation made plain, braces (which would be colour codes)
/// and control characters removed, whitespace collapsed.
pub fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if in_tag => {}
            _ => out.push(c),
        }
    }
    let ents = [("&amp;", "&"), ("&lt;", "<"), ("&gt;", ">"), ("&quot;", "\""), ("&#39;", "'"), ("&apos;", "'"), ("&nbsp;", " "), ("&#8217;", "'"), ("&#8216;", "'"), ("&#8220;", "\""), ("&#8221;", "\""), ("&#8211;", "-"), ("&#8212;", "-"), ("&#x27;", "'"), ("&#x2F;", "/")];
    for (e, r) in ents {
        out = out.replace(e, r);
    }
    let out: String = out
        .chars()
        .map(|c| match c {
            '‘' | '’' | '‚' | '′' | '`' | '´' => '\'',
            '“' | '”' | '„' | '″' => '"',
            '–' | '—' | '−' | '‐' | '‑' => '-',
            '{' => '(',
            '}' => ')',
            '…' => '.',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let folded: String = out.chars().map(fold_accent).collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The board has plain letters only: "é" shows as "E".
fn fold_accent(c: char) -> char {
    const MAP: [(&str, char); 12] = [
        ("àáâãäåāă", 'a'), ("ÀÁÂÃÄÅĀĂ", 'A'), ("èéêëēė", 'e'), ("ÈÉÊËĒĖ", 'E'), ("ìíîïī", 'i'), ("ÌÍÎÏĪ", 'I'),
        ("òóôõöøō", 'o'), ("ÒÓÔÕÖØŌ", 'O'), ("ùúûüū", 'u'), ("ÙÚÛÜŪ", 'U'), ("çćč", 'c'), ("ñń", 'n'),
    ];
    MAP.iter().find(|(set, _)| set.contains(c)).map(|(_, r)| *r).unwrap_or(c)
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let cut: String = s.chars().take(n).collect();
    match cut.rfind(' ') {
        Some(i) if i > n / 2 => cut[..i].to_string(),
        _ => cut,
    }
}

// ---------------------------------------------------------------- JSONPath subset

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Key(String),
    Index(usize),
    All,
}

/// `$`, `.key`, `['key']`, `[n]` and `[*]`.
pub fn parse_path(s: &str) -> Result<Vec<Step>, String> {
    let s = s.trim();
    let rest = s.strip_prefix('$').ok_or("a path starts with $")?;
    let chars: Vec<char> = rest.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '.' => {
                let start = i + 1;
                let mut j = start;
                while j < chars.len() && chars[j] != '.' && chars[j] != '[' {
                    j += 1;
                }
                let key: String = chars[start..j].iter().collect();
                if key.is_empty() {
                    return Err(format!("empty key in {s}"));
                }
                out.push(if key == "*" { Step::All } else { Step::Key(key) });
                i = j;
            }
            '[' => {
                let end = chars[i..].iter().position(|&c| c == ']').map(|p| i + p).ok_or(format!("unclosed [ in {s}"))?;
                let inner: String = chars[i + 1..end].iter().collect();
                let inner = inner.trim();
                if inner == "*" {
                    out.push(Step::All);
                } else if let Ok(n) = inner.parse::<usize>() {
                    out.push(Step::Index(n));
                } else if inner.len() >= 2 && ((inner.starts_with('\'') && inner.ends_with('\'')) || (inner.starts_with('"') && inner.ends_with('"'))) {
                    out.push(Step::Key(inner[1..inner.len() - 1].to_string()));
                } else {
                    return Err(format!("[{inner}] must be a number, * or a quoted key"));
                }
                i = end + 1;
            }
            c => return Err(format!("unexpected {c:?} in {s}")),
        }
    }
    Ok(out)
}

pub fn select<'a>(v: &'a Value, path: &[Step]) -> Vec<&'a Value> {
    let mut cur = vec![v];
    for step in path {
        let mut next = vec![];
        for x in cur {
            match (step, x) {
                (Step::Key(k), Value::Object(o)) => next.extend(o.get(k)),
                (Step::Index(n), Value::Array(a)) => next.extend(a.get(*n)),
                (Step::All, Value::Array(a)) => next.extend(a.iter()),
                (Step::All, Value::Object(o)) => next.extend(o.values()),
                _ => {}
            }
        }
        cur = next;
    }
    cur
}

/// A JSON value as board text: strings as they are, numbers with at most 2 decimals.
pub fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => clean(s),
        Value::Number(n) if n.is_f64() => {
            let f = n.as_f64().unwrap_or(0.0);
            let s = format!("{f:.2}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
        Value::Null => String::new(),
        Value::Array(a) => a.iter().map(value_text).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", "),
        other => clean(&other.to_string()),
    }
}

fn first_text(v: &Value, path: &Option<String>) -> String {
    path.as_ref().and_then(|p| parse_path(p).ok()).and_then(|p| select(v, &p).first().map(|x| value_text(x))).unwrap_or_default()
}

/// Times as ISO 8601, RFC 2822, or epoch seconds or milliseconds.
pub fn parse_time(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => {
            let f = n.as_f64()?;
            Some(if f > 1e11 { f as i64 } else { (f * 1000.0) as i64 })
        }
        Value::String(s) => {
            let s = s.trim();
            chrono::DateTime::parse_from_rfc3339(s)
                .or_else(|_| chrono::DateTime::parse_from_rfc2822(s))
                .map(|d| d.timestamp_millis())
                .ok()
                .or_else(|| s.parse::<f64>().ok().and_then(|f| parse_time(&json!(f))))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- placeholders

/// What placeholders can refer to.
pub struct Ctx<'a> {
    pub name: &'a str,
    pub item: Option<&'a Item>,
    pub settings: &'a Map<String, Value>,
    /// Looks a secret up; None outside the URL and headers
    pub secret: Option<&'a dyn Fn(&str) -> String>,
    /// Local time
    pub now: NaiveDateTime,
    /// The same moment, ms since the epoch ({{now:epoch}}, |mins)
    pub now_ms: i64,
    pub clock24: bool,
    /// A status list's items ({{rows}}, {{count}}); empty otherwise
    pub list: &'a [Item],
    /// card.row, for {{rows}}
    pub row: Option<&'a str>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// Values are percent-encoded, unless the placeholder is the whole URL
    Url,
    Header,
    /// Card text: no secrets
    Card,
}

pub fn render(template: &str, ctx: &Ctx, mode: Mode) -> String {
    let whole = template.trim().starts_with("{{") && template.trim().ends_with("}}") && template.matches("{{").count() == 1;
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find("}}") else {
            out.push_str(&rest[i..]);
            rest = "";
            break;
        };
        let value = lookup(after[..j].trim(), ctx, mode);
        out.push_str(&if mode == Mode::Url && !whole { encode(&value) } else { value });
        rest = &after[j + 2..];
    }
    out.push_str(rest);
    out
}

fn encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>().replace('+', "%20")
}

/// One placeholder: `name[:format][|filter[:arg]]...`.
fn lookup(key: &str, ctx: &Ctx, mode: Mode) -> String {
    let mut parts = key.split('|');
    let head = parts.next().unwrap_or("").trim();
    let mut v = value_of(head, ctx, mode);
    for f in parts {
        v = filter(&v, f, ctx);
    }
    v
}

/// "now", "now+2h", "now-90m" as an offset in ms from now (None if it isn't one of those).
fn now_offset(name: &str) -> Option<i64> {
    let rest = name.strip_prefix("now")?.trim();
    if rest.is_empty() {
        return Some(0);
    }
    let (sign, d) = match rest.split_at(1) {
        ("+", d) => (1, d),
        ("-", d) => (-1, d),
        _ => return None,
    };
    let secs = duration(&json!(d.trim())).ok()?;
    Some(sign * secs as i64 * 1000)
}

/// A time formatted, or as seconds ("epoch") or milliseconds ("epochms") since the epoch.
fn time_text(local: NaiveDateTime, ms: i64, fmt: &str) -> String {
    match fmt.trim() {
        "epoch" => ms.div_euclid(1000).to_string(),
        "epochms" => ms.to_string(),
        f => format_time(local, f),
    }
}

fn value_of(head: &str, ctx: &Ctx, mode: Mode) -> String {
    let (name, fmt) = match head.split_once(':') {
        Some((n, f)) => (n.trim(), Some(f.trim())),
        None => (head, None),
    };
    let default_fmt = if ctx.clock24 { "HH:mm" } else { "h:mm" };
    match name {
        "name" => ctx.name.to_string(),
        "title" => ctx.item.map(|i| i.title.clone()).unwrap_or_default(),
        "body" => ctx.item.map(|i| i.body.clone()).unwrap_or_default(),
        "time" => ctx.item.and_then(|i| i.time).map(|t| time_text(local_of(t), t, fmt.unwrap_or(default_fmt))).unwrap_or_default(),
        "count" => ctx.list.len().to_string(),
        "rows" => {
            let row = ctx.row.unwrap_or("{{title}}");
            ctx.list
                .iter()
                .map(|it| render(row, &Ctx { item: Some(it), list: &[], row: None, ..*ctx }, Mode::Card))
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => {
            if let Some(off) = now_offset(name) {
                let ms = ctx.now_ms + off;
                return time_text(ctx.now + chrono::Duration::milliseconds(off), ms, fmt.unwrap_or(default_fmt));
            }
            if let Some(k) = name.strip_prefix("field.") {
                return ctx.item.and_then(|i| i.fields.get(k).cloned()).unwrap_or_default();
            }
            if let Some(k) = name.strip_prefix("settings.") {
                return ctx.settings.get(k).map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Array(a) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("\n"),
                    other => other.to_string(),
                }).unwrap_or_default();
            }
            if let (Some(k), Some(f), true) = (name.strip_prefix("secret."), ctx.secret, mode != Mode::Card) {
                return f(k);
            }
            String::new()
        }
    }
}

/// |x| rounded half away from zero to `places` decimals, as digits: (negative, integer part,
/// fraction digits). None past 1e15, where doubles stop being exact enough.
fn rounded(x: f64, places: u32) -> Option<(bool, u64, String)> {
    let scale = 10f64.powi(places as i32);
    let m = (x.abs() * scale + 0.5).floor();
    if !m.is_finite() || x.abs() >= 1e15 {
        return None;
    }
    let m = m as u64;
    let p = 10u64.pow(places);
    let frac = if places == 0 { String::new() } else { format!("{:0width$}", m % p, width = places as usize) };
    Some((x < 0.0 && m != 0, m / p, frac))
}

fn thousands(n: u64) -> String {
    let d = n.to_string();
    let mut out = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// One template filter (see FILTERS). A filter that doesn't apply (|num on words) leaves the
/// value as it is.
fn filter(v: &str, f: &str, ctx: &Ctx) -> String {
    let (name, arg) = match f.split_once(':') {
        Some((n, a)) => (n.trim(), Some(a)),
        None => (f.trim(), None),
    };
    match name {
        "upper" => v.to_uppercase(),
        "lower" => v.to_lowercase(),
        "trim" => v.trim().to_string(),
        "num" => match numeric(v).and_then(|x| rounded(x, 2)) {
            Some((neg, int, frac)) => {
                let frac = frac.trim_end_matches('0');
                format!("{}{}{}{frac}", if neg { "-" } else { "" }, thousands(int), if frac.is_empty() { "" } else { "." })
            }
            None => v.to_string(),
        },
        "round" => {
            let places = arg.and_then(|a| a.trim().parse::<u32>().ok()).unwrap_or(0).min(6);
            match numeric(v).and_then(|x| rounded(x, places)) {
                Some((neg, int, frac)) => format!("{}{int}{}{frac}", if neg { "-" } else { "" }, if frac.is_empty() { "" } else { "." }),
                None => v.to_string(),
            }
        }
        "default" => {
            if v.trim().is_empty() {
                arg.unwrap_or("").to_string()
            } else {
                v.to_string()
            }
        }
        "pad" => {
            let n = arg.and_then(|a| a.trim().parse::<i32>().ok()).unwrap_or(0);
            let width = crate::vestaboard::parse(v).len() as i32;
            let fill = "{0}".repeat((n.abs() - width).max(0) as usize);
            if n < 0 {
                fill + v
            } else {
                v.to_string() + &fill
            }
        }
        "time" | "mins" => {
            let t = numeric(v).and_then(|n| parse_time(&json!(n))).or_else(|| parse_time(&json!(v.trim())));
            match (name, t) {
                ("time", Some(t)) => {
                    let default_fmt = if ctx.clock24 { "HH:mm" } else { "h:mm" };
                    let fmt = arg.map(str::trim).filter(|a| !a.is_empty()).unwrap_or(default_fmt);
                    time_text(local_of(t), t, fmt)
                }
                (_, Some(t)) => ((t - ctx.now_ms).max(0) / 60_000).to_string(),
                _ => v.to_string(),
            }
        }
        _ => v.to_string(),
    }
}

/// Removes {{...}} so a URL template can be checked for its scheme.
fn strip_placeholders(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        match rest[i..].find("}}") {
            Some(j) => {
                out.push('x');
                rest = &rest[i + j + 2..];
            }
            None => {
                rest = "";
            }
        }
    }
    out + rest
}

pub fn local_of(ms: i64) -> NaiveDateTime {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map(|d| d.naive_local()).unwrap_or_default()
}

/// Formats a time with tokens like those of Moment and .NET: yyyy yy MMMM MMM MM M dddd ddd
/// dd d HH H hh h mm ss tt (AM/PM). Anything in [brackets] is literal.
pub fn format_time(t: NaiveDateTime, fmt: &str) -> String {
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    let chars: Vec<char> = fmt.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let h12 = match t.hour() % 12 {
        0 => 12,
        h => h,
    };
    while i < chars.len() {
        if chars[i] == '[' {
            let end = chars[i..].iter().position(|&c| c == ']').map(|p| i + p).unwrap_or(chars.len());
            out.extend(&chars[i + 1..end]);
            i = end + 1;
            continue;
        }
        let run = chars[i..].iter().take_while(|&&c| c == chars[i]).count();
        let tok: String = chars[i..i + run].iter().collect();
        let s = match tok.as_str() {
            "yyyy" | "YYYY" => format!("{}", t.year()),
            "yy" | "YY" => format!("{:02}", t.year() % 100),
            "MMMM" => MONTHS[t.month0() as usize].to_string(),
            "MMM" => MONTHS[t.month0() as usize][..3].to_string(),
            "MM" => format!("{:02}", t.month()),
            "M" => t.month().to_string(),
            "dddd" => WEEKDAYS[t.weekday().num_days_from_monday() as usize].to_string(),
            "ddd" => WEEKDAYS[t.weekday().num_days_from_monday() as usize][..3].to_string(),
            "dd" | "DD" => format!("{:02}", t.day()),
            "d" | "D" => t.day().to_string(),
            "HH" => format!("{:02}", t.hour()),
            "H" => t.hour().to_string(),
            "hh" => format!("{h12:02}"),
            "h" => h12.to_string(),
            "mm" => format!("{:02}", t.minute()),
            "m" => t.minute().to_string(),
            "ss" => format!("{:02}", t.second()),
            "tt" | "A" => (if t.hour() < 12 { "AM" } else { "PM" }).to_string(),
            _ => tok.clone(),
        };
        out.push_str(&s);
        i += run;
    }
    out
}

// ---------------------------------------------------------------- fetching

/// HTTPS anywhere; plain HTTP only to this computer or the local network.
pub fn check_scheme(u: &url::Url) -> Result<(), String> {
    match u.scheme() {
        "https" => Ok(()),
        "http" if u.host().is_some_and(|h| private_host(&h)) => Ok(()),
        "http" => Err("Plain http:// is only allowed for addresses on your own network; use https://".into()),
        _ => Err("Only http:// and https:// addresses can be fetched".into()),
    }
}

fn private_host(h: &url::Host<&str>) -> bool {
    match h {
        url::Host::Ipv4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        url::Host::Ipv6(ip) => ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80,
        url::Host::Domain(d) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            d == "localhost" || !d.contains('.') || [".local", ".lan", ".home.arpa", ".internal", ".localhost"].iter().any(|s| d.ends_with(s))
        }
    }
}

pub const USER_AGENT: &str = concat!("Rattleboard/", env!("CARGO_PKG_VERSION"), " (+https://www.iokig.com/rattleboard)");

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .connect_timeout(FETCH_TIMEOUT)
        .user_agent(USER_AGENT)
        // A redirect has to follow the same rule as the address itself.
        .redirect(reqwest::redirect::Policy::custom(|a| {
            if a.previous().len() >= 5 {
                a.error("too many redirects")
            } else if check_scheme(a.url()).is_err() {
                a.error("redirected to a plain http:// address")
            } else {
                a.follow()
            }
        }))
        .build()
        .expect("http client")
}

/// GETs (or POSTs) a URL, up to 2 MB. Errors never include the URL, which may hold a secret.
pub async fn fetch(c: &reqwest::Client, method: &str, url: &str, headers: &[(String, String)]) -> Result<Vec<u8>, String> {
    let u = url::Url::parse(url).map_err(|_| "The address isn't a valid URL".to_string())?;
    check_scheme(&u)?;
    let mut req = if method == "POST" { c.post(u) } else { c.get(u) };
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let mut res = req.send().await.map_err(describe)?;
    let status = res.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(match status {
            401 | 403 => "The server refused access (check the key or address)".into(),
            404 => "Nothing at that address (HTTP 404)".into(),
            429 => "The server is rate-limiting requests".into(),
            s => format!("The server answered HTTP {s}"),
        });
    }
    if res.content_length().is_some_and(|n| n as usize > MAX_RESPONSE) {
        return Err("The response is larger than 2 MB".into());
    }
    let mut body = vec![];
    while let Some(chunk) = res.chunk().await.map_err(describe)? {
        if body.len() + chunk.len() > MAX_RESPONSE {
            return Err("The response is larger than 2 MB".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// HMAC of `msg` under `key`, in hex.
pub fn hmac_hex(kind: &str, key: &[u8], msg: &[u8], upper: bool) -> String {
    use hmac::{Hmac, Mac};
    let bytes = if kind == "hmac-sha256" {
        let mut m = Hmac::<sha2::Sha256>::new_from_slice(key).expect("any key length");
        m.update(msg);
        m.finalize().into_bytes().to_vec()
    } else {
        let mut m = Hmac::<sha1::Sha1>::new_from_slice(key).expect("any key length");
        m.update(msg);
        m.finalize().into_bytes().to_vec()
    };
    bytes.iter().map(|b| if upper { format!("{b:02X}") } else { format!("{b:02x}") }).collect()
}

/// Signs a URL whose placeholders are filled in: the HMAC of its path and query, exactly as
/// they'll be sent ("/v3/departures/route_type/0/stop/1071?devid=123"), appended as one more
/// query parameter. A fragment is dropped first. The error never includes the URL or key.
pub fn sign_url(url: &str, s: &Sign, key: &str) -> Result<String, String> {
    if key.is_empty() {
        return Err("Add this source's signing key in Settings → Sources".into());
    }
    let mut u = url::Url::parse(url).map_err(|_| "The address isn't a valid URL".to_string())?;
    u.set_fragment(None);
    if u.query() == Some("") {
        u.set_query(None);
    }
    let msg = match u.query() {
        Some(q) => format!("{}?{q}", u.path()),
        None => u.path().to_string(),
    };
    let sig = hmac_hex(&s.kind, key.as_bytes(), msg.as_bytes(), s.upper);
    let sep = if u.query().is_some() { '&' } else { '?' };
    Ok(format!("{u}{sep}{}={sig}", s.param))
}

fn describe(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "Timed out after 10 s".into()
    } else if e.is_redirect() {
        "Redirected to an address that isn't allowed".into()
    } else if e.is_connect() {
        "Couldn't connect to the server".into()
    } else {
        "Network error".into()
    }
}

// ---------------------------------------------------------------- parsing

/// RSS and Atom (and JSON Feed), newest first.
pub fn parse_feed(bytes: &[u8], max: usize) -> Result<Data, String> {
    let feed = feed_rs::parser::parse(bytes).map_err(|_| "That isn't an RSS or Atom feed".to_string())?;
    let mut items: Vec<Item> = feed
        .entries
        .into_iter()
        .map(|e| {
            let title = e.title.map(|t| clean(&t.content)).unwrap_or_default();
            let body = e.summary.map(|s| clean(&s.content)).or_else(|| e.content.and_then(|c| c.body).map(|b| clean(&b))).unwrap_or_default();
            let time = e.published.or(e.updated).map(|d| d.timestamp_millis());
            let key = if e.id.is_empty() { hash(&[&title, &body]) } else { hash(&[&e.id]) };
            // The entry's page: its first alternate link (or first link without a rel)
            let link = e.links.iter().find(|l| l.rel.as_deref().is_none_or(|r| r == "alternate")).and_then(|l| safe_link(&l.href));
            Item { key, title, body: short(&body, 400), time, link, ..Default::default() }
        })
        .filter(|i| !i.title.is_empty() || !i.body.is_empty())
        .collect();
    // Feeds are usually newest first already; one with dates is put in order.
    if items.iter().all(|i| i.time.is_some()) {
        items.sort_by_key(|i| std::cmp::Reverse(i.time));
    }
    items.truncate(max);
    Ok(Data { items, single: false })
}

/// A JSON document: a list of items at `pick.items`, or one value.
/// `pick` has its placeholders filled in and its conditions settled (see `Pick::resolve`).
/// A list is filtered by pick.where, then sorted by pick.sort, then cut to `max` (maxItems).
pub fn parse_json(bytes: &[u8], pick: &Pick, max: usize) -> Result<Data, String> {
    let v: Value = serde_json::from_slice(bytes).map_err(|_| "The response isn't JSON".to_string())?;
    let item = |x: &Value| {
        let title = first_text(x, &pick.title);
        let body = first_text(x, &pick.body);
        let time = pick.time.as_ref().and_then(|p| parse_path(p).ok()).and_then(|p| select(x, &p).first().and_then(|t| parse_time(t)));
        let id = first_text(x, &pick.id);
        let key = if id.is_empty() { hash(&[&title, &body]) } else { hash(&[&id]) };
        let fields = pick.fields.iter().map(|(k, paths)| (k.clone(), short(&field_text(x, paths), 400))).collect();
        let link = pick.link.as_ref().and_then(|p| parse_path(p).ok()).and_then(|p| select(x, &p).into_iter().next().and_then(|v| v.as_str()).and_then(safe_link));
        Item { key, title: short(&title, 400), body: short(&body, 400), time, fields, link, ..Default::default() }
    };
    match &pick.items {
        Some(p) => {
            let path = parse_path(p).map_err(|e| format!("pick.items: {e}"))?;
            let found = select(&v, &path);
            if found.is_empty() && !pick.status {
                return Err(format!("Nothing found at {p}"));
            }
            let mut kept: Vec<&Value> = found.into_iter().filter(|x| pick.conds.iter().all(|c| cond_holds(x, c))).collect();
            if let Some(s) = &pick.sort {
                sort_items(&mut kept, s);
            }
            Ok(Data { items: kept.into_iter().take(max).map(item).collect(), single: pick.status })
        }
        None => Ok(Data { items: vec![item(&v)], single: true }),
    }
}

/// A field: the first of its paths whose first match is there and says something.
fn field_text(x: &Value, paths: &[String]) -> String {
    for p in paths {
        let Ok(p) = parse_path(p) else { continue };
        if let Some(v) = select(x, &p).into_iter().next().filter(|v| !v.is_null()) {
            let t = value_text(v);
            if !t.is_empty() {
                return t;
            }
        }
    }
    String::new()
}

/// A number written plainly: [+-] digits [. digits] [e[+-]digits], nothing else (no "inf",
/// no thousands separators), after trimming.
pub fn numeric(s: &str) -> Option<f64> {
    let s = s.trim();
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let int = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    i += int;
    let mut frac = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        frac = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        i += frac;
    }
    if int + frac == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let e = b[i..].iter().take_while(|c| c.is_ascii_digit()).count();
        if e == 0 {
            return None;
        }
        i += e;
    }
    if i != b.len() {
        return None;
    }
    s.parse::<f64>().ok().filter(|f| f.is_finite())
}

/// A JSON value's number: a number, or a string that's a plain number.
fn num_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => numeric(s),
        _ => None,
    }
}

/// A scalar's text for comparing: strings trimmed, numbers as the board writes them.
fn cmp_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(_) => Some(value_text(v)),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Whether an item meets one pick.where condition (already resolved: no placeholders, and
/// `when` settled).
pub fn cond_holds(x: &Value, c: &Cond) -> bool {
    let Ok(path) = parse_path(&c.path) else { return false };
    let left = select(x, &path).into_iter().next().filter(|v| !v.is_null());
    match (c.op.as_str(), left) {
        ("exists", l) => l.is_some(),
        ("missing", l) => l.is_none(),
        (_, None) => false,
        ("contains", Some(Value::Array(a))) => {
            let r = cmp_text(&c.value).unwrap_or_default().to_ascii_lowercase();
            a.iter().filter_map(cmp_text).any(|t| t.to_ascii_lowercase() == r)
        }
        (_, Some(Value::Array(_) | Value::Object(_))) => false,
        (op, Some(l)) => {
            if op != "contains" {
                if let (Some(a), Some(b)) = (num_of(l), num_of(&c.value)) {
                    return match op {
                        "==" => a == b,
                        "!=" => a != b,
                        ">" => a > b,
                        ">=" => a >= b,
                        "<" => a < b,
                        _ => a <= b,
                    };
                }
            }
            let (Some(a), Some(b)) = (cmp_text(l), cmp_text(&c.value)) else { return false };
            let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
            match op {
                "==" => a == b,
                "!=" => a != b,
                ">" => a > b,
                ">=" => a >= b,
                "<" => a < b,
                "<=" => a <= b,
                _ => a.contains(&b),
            }
        }
    }
}

/// Sorts a list by pick.sort: stable, and items without the key last whichever the order.
pub fn sort_items(list: &mut [&Value], s: &SortSpec) {
    let Ok(path) = parse_path(&s.path) else { return };
    let key = |x: &Value| select(x, &path).into_iter().next().filter(|v| !v.is_null() && !v.is_array() && !v.is_object()).cloned();
    let numeric = s.numeric.unwrap_or_else(|| {
        let present: Vec<Value> = list.iter().filter_map(|x| key(x)).collect();
        !present.is_empty() && present.iter().all(|v| num_of(v).is_some())
    });
    let mut keyed: Vec<(usize, Option<f64>, Option<String>)> = list
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let k = key(x);
            if numeric {
                (i, k.as_ref().and_then(num_of), None)
            } else {
                (i, None, k.as_ref().and_then(cmp_text).map(|t| t.to_ascii_lowercase()))
            }
        })
        .collect();
    use std::cmp::Ordering;
    keyed.sort_by(|a, b| {
        let o = if numeric {
            match (a.1, b.1) {
                (Some(x), Some(y)) => {
                    let o = x.partial_cmp(&y).unwrap_or(Ordering::Equal);
                    if s.desc { o.reverse() } else { o }
                }
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        } else {
            match (&a.2, &b.2) {
                (Some(x), Some(y)) => {
                    let o = x.cmp(y);
                    if s.desc { o.reverse() } else { o }
                }
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
        };
        o.then(a.0.cmp(&b.0))
    });
    let orig: Vec<&Value> = list.to_vec();
    for (slot, (i, _, _)) in list.iter_mut().zip(keyed) {
        *slot = orig[i];
    }
}

impl Pick {
    /// The pick for one fetch: placeholders in its paths and condition values filled in from
    /// the source's settings, conditions whose `when` doesn't match the setting dropped, and
    /// conditions whose value came out blank from a placeholder dropped (a filter on a blank
    /// setting is off).
    pub fn resolve(&self, ctx: &Ctx) -> Pick {
        let path = |p: &Option<String>| p.as_ref().map(|p| render(p, ctx, Mode::Card)).filter(|p| !p.trim().is_empty());
        let one = |p: &String| render(p, ctx, Mode::Card).trim().to_string();
        let conds = self
            .conds
            .iter()
            .filter(|c| match &c.when {
                None => true,
                Some((k, is)) => {
                    let v = lookup(&format!("settings.{k}"), ctx, Mode::Card);
                    is.iter().any(|x| x == v.trim())
                }
            })
            .filter_map(|c| {
                let value = match &c.value {
                    Value::String(s) if s.contains("{{") => {
                        let r = render(s, ctx, Mode::Card).trim().to_string();
                        if r.is_empty() {
                            return None;
                        }
                        Value::String(r)
                    }
                    Value::String(s) => Value::String(s.trim().to_string()),
                    v => v.clone(),
                };
                Some(Cond { path: one(&c.path), op: c.op.clone(), value, when: None })
            })
            .collect();
        Pick {
            items: path(&self.items),
            id: path(&self.id),
            title: path(&self.title),
            body: path(&self.body),
            time: path(&self.time),
            link: path(&self.link),
            fields: self.fields.iter().map(|(k, ps)| (k.clone(), ps.iter().map(one).collect())).collect(),
            conds,
            sort: self.sort.as_ref().map(|s| SortSpec { path: one(&s.path), ..s.clone() }),
            status: self.status,
        }
    }
}

/// An iCal calendar: the events from yesterday to 60 days ahead, soonest first, with
/// recurring events expanded.
pub fn parse_ical(bytes: &[u8], now_ms: i64) -> Result<Data, String> {
    let mut items = vec![];
    let from = now_ms - 86_400_000;
    let until = now_ms + 60 * 86_400_000;
    let mut found = false;
    for cal in ical::IcalParser::new(std::io::BufReader::new(bytes)) {
        let cal = cal.map_err(|_| "That isn't an iCal calendar".to_string())?;
        found = true;
        for ev in cal.events {
            let prop = |n: &str| ev.properties.iter().find(|p| p.name.eq_ignore_ascii_case(n));
            let text = |n: &str| prop(n).and_then(|p| p.value.clone()).map(|v| clean(&unescape_ical(&v))).unwrap_or_default();
            let Some(start) = prop("DTSTART") else { continue };
            let Some((start_ms, all_day)) = ical_time(start) else { continue };
            let end_ms = prop("DTEND").and_then(ical_time).map(|e| e.0).unwrap_or(if all_day { start_ms + 86_400_000 } else { start_ms });
            let length = (end_ms - start_ms).max(0);
            let (title, body, uid) = (text("SUMMARY"), text("LOCATION"), text("UID"));
            let link = prop("URL").and_then(|p| p.value.as_deref()).and_then(safe_link);
            let mut starts = vec![start_ms];
            if let Some(rule) = prop("RRULE").and_then(|r| r.value.clone()) {
                starts = expand(start, &rule, &ev.properties, from, until).unwrap_or(starts);
            }
            for s in starts {
                if s + length < from || s > until {
                    continue;
                }
                let key = hash(&[&uid, &s.to_string()]);
                items.push(Item { key, title: title.clone(), body: body.clone(), time: Some(s), end: Some(s + length), all_day, link: link.clone(), ..Default::default() });
            }
        }
    }
    if !found {
        return Err("That isn't an iCal calendar".into());
    }
    items.sort_by_key(|i| i.time);
    items.truncate(200);
    Ok(Data { items, single: true })
}

fn unescape_ical(s: &str) -> String {
    s.replace("\\n", " ").replace("\\N", " ").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

fn param<'a>(p: &'a ical::property::Property, name: &str) -> Option<&'a str> {
    p.params.as_ref()?.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).and_then(|(_, v)| v.first()).map(|s| s.as_str())
}

/// DTSTART/DTEND: UTC ("...Z"), with a TZID, floating (local time) or a whole day.
fn ical_time(p: &ical::property::Property) -> Option<(i64, bool)> {
    use chrono::TimeZone;
    let v = p.value.as_ref()?.trim();
    if v.len() == 8 || param(p, "VALUE").is_some_and(|x| x.eq_ignore_ascii_case("DATE")) {
        let d = chrono::NaiveDate::parse_from_str(&v[..8.min(v.len())], "%Y%m%d").ok()?;
        let t = chrono::Local.from_local_datetime(&d.and_hms_opt(0, 0, 0)?).earliest()?;
        return Some((t.timestamp_millis(), true));
    }
    let naive = NaiveDateTime::parse_from_str(v.trim_end_matches('Z'), "%Y%m%dT%H%M%S").ok()?;
    let ms = if v.ends_with('Z') {
        naive.and_utc().timestamp_millis()
    } else if let Some(tz) = param(p, "TZID").and_then(|z| z.trim_matches('"').parse::<chrono_tz::Tz>().ok()) {
        tz.from_local_datetime(&naive).earliest()?.timestamp_millis()
    } else {
        chrono::Local.from_local_datetime(&naive).earliest()?.timestamp_millis()
    };
    Some((ms, false))
}

/// The starts of a recurring event within from..until.
fn expand(start: &ical::property::Property, rule: &str, props: &[ical::property::Property], from: i64, until: i64) -> Option<Vec<i64>> {
    use chrono::TimeZone;
    let dt = |p: &ical::property::Property| -> Option<String> {
        let v = p.value.as_ref()?.trim().to_string();
        Some(match param(p, "TZID") {
            Some(tz) => format!(";TZID={}:{v}", tz.trim_matches('"')),
            None => format!(":{v}"),
        })
    };
    let mut text = format!("DTSTART{}\nRRULE:{rule}", dt(start)?);
    for ex in props.iter().filter(|p| p.name.eq_ignore_ascii_case("EXDATE")) {
        if let Some(d) = dt(ex) {
            text.push_str(&format!("\nEXDATE{d}"));
        }
    }
    let set: rrule::RRuleSet = text.parse().ok()?;
    let tz = rrule::Tz::UTC;
    let after = tz.timestamp_millis_opt(from).single()?;
    let before = tz.timestamp_millis_opt(until).single()?;
    let r = set.after(after).before(before).all(500);
    Some(r.dates.iter().map(|d| d.timestamp_millis()).collect())
}

/// A command's stdout: JSON {"text", "priority", "id"} or {"characters"}, else plain text.
pub fn parse_output(out: &[u8]) -> Data {
    let s = String::from_utf8_lossy(out);
    let trimmed = s.trim();
    if let Ok(Value::Object(o)) = serde_json::from_str::<Value>(trimmed) {
        let characters = o.get("characters").and_then(|c| crate::vestaboard::posted_grid(c).ok());
        let text = o.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
        if characters.is_some() || !text.is_empty() {
            let id = o.get("id").map(value_text).unwrap_or_default();
            let key = if id.is_empty() { hash(&[&text, &format!("{characters:?}")]) } else { hash(&[&id]) };
            let alert = o.get("priority").and_then(|p| p.as_str()) == Some("alert");
            let link = o.get("link").and_then(|l| l.as_str()).and_then(safe_link);
            let item = Item { key, title: text, alert, characters, link, ..Default::default() };
            return Data { items: vec![item], single: true };
        }
    }
    // Plain text keeps its lines (and any {r} colour codes the program printed).
    let text: String = trimmed.lines().map(|l| l.trim_end()).filter(|l| !l.is_empty()).take(24).collect::<Vec<_>>().join("\n");
    Data { items: vec![Item { key: hash(&[&text]), title: text, ..Default::default() }], single: true }
}

// ---------------------------------------------------------------- commands

/// Runs an approved command plugin: no shell, no stdin, stdout read up to 64 KB, killed when
/// it runs past its timeout.
pub async fn run_command(run: &Run) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncReadExt;
    let mut cmd = tokio::process::Command::new(&run.path);
    cmd.args(&run.args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    {
        // No console window flashing up every few minutes.
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("{} wasn't found", run.path),
        std::io::ErrorKind::PermissionDenied => format!("{} can't be run (permission denied)", run.path),
        _ => format!("Couldn't start {}", run.path),
    })?;
    let mut stdout = child.stdout.take().ok_or("No output")?;
    let work = async {
        let mut out = vec![];
        let mut buf = [0u8; 8192];
        loop {
            let n = stdout.read(&mut buf).await.map_err(|_| "Couldn't read its output".to_string())?;
            if n == 0 {
                break;
            }
            let room = MAX_STDOUT - out.len();
            out.extend_from_slice(&buf[..n.min(room)]);
            if out.len() >= MAX_STDOUT {
                // Enough: the rest is never read.
                return Ok::<_, String>((out, None));
            }
        }
        let status = child.wait().await.map_err(|_| "It stopped unexpectedly".to_string())?;
        Ok((out, Some(status)))
    };
    let res = tokio::time::timeout(Duration::from_secs(run.timeout), work).await;
    match res {
        Err(_) => {
            let _ = child.kill().await;
            Err(format!("Stopped after {} s without finishing", run.timeout))
        }
        Ok(Err(e)) => {
            let _ = child.kill().await;
            Err(e)
        }
        Ok(Ok((out, None))) => {
            let _ = child.kill().await;
            Ok(out)
        }
        Ok(Ok((out, Some(status)))) if status.success() => Ok(out),
        Ok(Ok((_, Some(status)))) => Err(match status.code() {
            Some(c) => format!("It exited with status {c}"),
            None => "It was stopped by a signal".into(),
        }),
    }
}

/// What's approved: a fingerprint of the whole manifest, so any change needs approving again.
pub fn fingerprint(raw: &Value) -> String {
    hash(&[&serde_json::to_string(raw).unwrap_or_default()])
}

// ---------------------------------------------------------------- built-ins

/// Rattleboard's own plugins, listed alongside the installed ones. Clock, countdown, weather,
/// message and quote of the day are drawn by native code (see `builtins.rs`); RSS, calendar
/// and JSON URL are ordinary manifests whose settings fill in the address.
pub fn builtins() -> Vec<Manifest> {
    let list = json!([
        {"format": 1, "id": "rattleboard.clock", "name": "Clock", "kind": "native", "description": "The time: big digits with an hour bar, a time-of-day palette or rainbow digits; time and date; or a world clock",
         "every": "1m", "dwell": "5m", "card": {"chip": "white"},
         "settings": [
            {"key": "style", "label": "Style", "type": "select", "default": "colour", "options": [
                {"value": "colour", "label": "Colour (hour bar)"}, {"value": "daypart", "label": "Time of day"}, {"value": "playful", "label": "Playful"},
                {"value": "big", "label": "Big time"}, {"value": "date", "label": "Time and date"}, {"value": "world", "label": "World clock"}]},
            {"key": "cities", "label": "Cities (world clock, up to 3)", "type": "text", "default": "Melbourne, London, New York"},
            {"key": "colour", "label": "Colour", "type": "select", "default": "playful", "options": [
                {"value": "playful", "label": "Playful"}, {"value": "accent", "label": "Accent"}, {"value": "off", "label": "Off"}]}]},
        {"format": 1, "id": "rattleboard.countdown", "name": "Countdown", "kind": "native", "description": "Days to a birthday, a holiday or any date, with a celebration on the day",
         "every": "1m", "dwell": "1m", "card": {"chip": "violet"},
         "settings": [
            {"key": "preset", "label": "Preset", "type": "select", "default": "", "options": [
                {"value": "", "label": "My own date"}, {"value": "nye", "label": "New Year's Eve"}, {"value": "christmas", "label": "Christmas"}, {"value": "halloween", "label": "Halloween"}]},
            {"key": "title", "label": "Name", "type": "text", "default": ""},
            {"key": "date", "label": "Date", "type": "date", "default": ""},
            {"key": "time", "label": "Time (optional)", "type": "time", "default": ""},
            {"key": "yearly", "label": "Every year", "type": "toggle", "default": false},
            {"key": "ticker", "label": "Minute ticker in the last hour", "type": "toggle", "default": true},
            {"key": "celebrate", "label": "Celebration card on the day", "type": "toggle", "default": true},
            {"key": "celebration", "label": "Celebration text", "type": "text", "default": ""},
            {"key": "colour", "label": "Colour", "type": "select", "default": "playful", "options": [
                {"value": "playful", "label": "Playful"}, {"value": "accent", "label": "Accent"}, {"value": "off", "label": "Off"}]}]},
        {"format": 1, "id": "rattleboard.weather", "name": "Weather", "kind": "native", "description": "Now and today from Open-Meteo, no key needed",
         "every": "15m", "dwell": "1m", "card": {"chip": "yellow"},
         "settings": [
            {"key": "city", "label": "City", "type": "text", "default": "Melbourne"},
            {"key": "units", "label": "Units", "type": "select", "default": "celsius", "options": [
                {"value": "celsius", "label": "°C, km/h"}, {"value": "fahrenheit", "label": "°F, mph"}]},
            {"key": "colour", "label": "Colour", "type": "select", "default": "playful", "options": [
                {"value": "playful", "label": "Playful"}, {"value": "accent", "label": "Accent"}, {"value": "off", "label": "Off"}]}]},
        {"format": 1, "id": "rattleboard.rss", "name": "RSS or Atom feed", "kind": "rss", "description": "New stories from any feed",
         "fetch": {"url": "{{settings.url}}", "every": "15m"}, "dwell": "30s", "card": {"chip": "orange", "pages": 2},
         "settings": [{"key": "url", "label": "Feed address", "type": "text", "default": ""}]},
        {"format": 1, "id": "rattleboard.ical", "name": "Calendar (iCal)", "kind": "ical", "description": "Your next event, and an alert 10 minutes before it starts",
         "fetch": {"url": "{{secret.url}}", "every": "15m"}, "dwell": "30s", "card": {"chip": "blue"},
         "secrets": [{"key": "url", "label": "Secret iCal address"}]},
        {"format": 1, "id": "rattleboard.json", "name": "JSON URL", "kind": "json", "description": "Any JSON API: pick the fields and lay out the card",
         "fetch": {"url": "{{settings.url}}", "every": "15m"}, "dwell": "30s", "card": {"chip": "green", "template": "{{settings.template}}"},
         "pick": {"items": "{{settings.items}}", "title": "{{settings.title}}", "body": "{{settings.body}}"},
         "settings": [
            {"key": "url", "label": "Address", "type": "text", "default": ""},
            {"key": "items", "label": "Items path (optional)", "type": "text", "default": ""},
            {"key": "title", "label": "Title path", "type": "text", "default": "$.title"},
            {"key": "body", "label": "Body path (optional)", "type": "text", "default": ""},
            {"key": "template", "label": "Card template (optional)", "type": "text", "default": ""}]},
        {"format": 1, "id": "rattleboard.motd", "name": "Message of the day", "kind": "native", "description": "Your own messages, a different one each day",
         "every": "1m", "dwell": "1m", "card": {"chip": "green"},
         "settings": [{"key": "lines", "label": "Messages, one per line", "type": "lines", "default": []},
            {"key": "colour", "label": "Colour", "type": "select", "default": "playful", "options": [
                {"value": "playful", "label": "Playful"}, {"value": "accent", "label": "Accent"}, {"value": "off", "label": "Off"}]}]},
        {"format": 1, "id": "rattleboard.quote", "name": "Quote of the day", "kind": "native", "description": "A public-domain quote each day, the same on every board",
         "every": "1m", "dwell": "1m", "card": {"chip": "blue"},
         "settings": [
            {"key": "tags", "label": "Quotes", "type": "select", "default": "", "options": [
                {"value": "", "label": "Any"}, {"value": "work", "label": "Work"}, {"value": "calm", "label": "Calm"}, {"value": "funny", "label": "Funny"}, {"value": "wisdom", "label": "Wisdom"}]},
            {"key": "mode", "label": "Show", "type": "select", "default": "rotation", "options": [
                {"value": "rotation", "label": "In the rotation"}, {"value": "daily", "label": "Once a day as a message"}]},
            {"key": "at", "label": "At (once a day)", "type": "time", "default": "08:00"},
            {"key": "mine", "label": "My quotes (text - author), one per line", "type": "lines", "default": []},
            {"key": "source", "label": "Use", "type": "select", "default": "both", "options": [
                {"value": "both", "label": "Built-in and mine"}, {"value": "bundled", "label": "Built-in only"}, {"value": "mine", "label": "Only mine"}]},
            {"key": "colour", "label": "Colour", "type": "select", "default": "playful", "options": [
                {"value": "playful", "label": "Playful"}, {"value": "accent", "label": "Accent"}, {"value": "off", "label": "Off"}]}]}
    ]);
    list.as_array().unwrap().iter().map(|v| parse(v, true).expect("built-in manifest")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    /// An absolute program path on this platform ("/usr/bin/git" isn't absolute on Windows).
    const GIT: &str = if cfg!(windows) { "C:\\Program Files\\Git\\cmd\\git.exe" } else { "/usr/bin/git" };
    use chrono::NaiveDate;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    fn hn() -> Value {
        json!({"format": 1, "id": "com.example.hn", "name": "Hacker News", "kind": "rss",
            "fetch": {"url": "https://hnrss.org/frontpage", "every": "15m"}, "card": {"chip": "orange", "pages": 2},
            "alertWhen": ["keyword:launch"], "dwell": "30s", "futureField": {"ignored": true},
            "schedule": {"days": ["mon", "tue"], "from": "07:00", "to": "10:00"},
            "settings": [{"key": "city", "label": "City", "type": "text", "default": "Melbourne"}],
            "secrets": [{"key": "token", "label": "API token"}]})
    }

    #[test]
    fn manifests() {
        let m = parse(&hn(), false).unwrap();
        assert_eq!((m.id.as_str(), m.kind.as_str(), m.every, m.dwell, m.card.chip, m.card.pages), ("com.example.hn", "rss", 900, Some(30), Some(64), 2));
        assert_eq!((m.alert_words.clone(), m.settings[0].default.clone(), m.secrets[0].key.as_str()), (vec!["launch".to_string()], json!("Melbourne"), "token"));
        assert_eq!(m.schedule.as_ref().unwrap().days, vec![0, 1]);
        for b in builtins() {
            assert!(b.builtin && b.id.starts_with(BUILTIN_PREFIX), "{}", b.id);
        }

        let with = |k: &str, v: Value| {
            let mut m = hn();
            if v.is_null() {
                m.as_object_mut().unwrap().remove(k);
            } else {
                m[k] = v;
            }
            parse(&m, false)
        };
        for (k, v, msg) in [
            ("format", json!(2), "format 2"),
            ("format", Value::Null, "\"format\": 1"),
            ("id", json!("x"), "id must be"),
            ("id", json!("rattleboard.clock"), "built-ins"),
            ("id", json!("has space"), "id must be"),
            ("name", json!(""), "name must be"),
            ("kind", json!("shell"), "kind must be"),
            ("kind", json!("native"), "kind must be"),
            ("fetch", Value::Null, "needs fetch.url"),
            ("fetch", json!({"url": "http://example.com/feed"}), "only allowed for addresses on your own network"),
            ("fetch", json!({"url": "ftp://example.com/feed"}), "Only http"),
            ("fetch", json!({"url": "https://a.b/", "every": "30s"}), "at least 1m"),
            ("fetch", json!({"url": "https://a.b/", "method": "DELETE"}), "GET or POST"),
            ("fetch", json!({"url": "https://a.b/", "headers": {"X": 3}}), "Header X"),
            ("card", json!({"template": "{{secret.token}}"}), "never in card.template"),
            ("card", json!({"chip": "pink"}), "Unknown chip"),
            ("card", json!({"pages": 4}), "pages"),
            ("card", json!({"align": "right"}), "center or left"),
            ("priority", json!("urgent"), "ambient or alert"),
            ("alertWhen", json!(["launch"]), "keyword:"),
            ("maxItems", json!(0), "maxItems"),
            ("dwell", json!("5s"), "dwell"),
            ("schedule", json!({"days": ["funday"]}), "Unknown day"),
            ("schedule", json!({"from": "7am", "to": "10:00"}), "HH:MM"),
            ("schedule", json!({"from": "07:00"}), "both"),
            ("settings", json!([{"key": "a b"}]), "Setting key"),
            ("settings", json!([{"key": "a", "type": "colour"}]), "type must be"),
            ("settings", json!([{"key": "a", "type": "select"}]), "needs options"),
            ("settings", json!([{"key": "a", "type": "toggle", "default": "yes"}]), "true or false"),
            ("secrets", json!([{"key": "city"}]), "Secret key"),
        ] {
            let e = with(k, v.clone()).unwrap_err();
            assert!(e.contains(msg), "{k}={v}: {e}");
        }
        // Private-network HTTP is fine, and so are URLs built from placeholders.
        assert!(with("fetch", json!({"url": "http://192.168.1.20:8123/api/feed"})).is_ok());
        assert!(with("fetch", json!({"url": "http://nas.local/feed"})).is_ok());
        assert!(with("fetch", json!({"url": "{{secret.url}}"})).is_ok());
        assert!(parse(&json!([1]), false).is_err());

        let json_kind = json!({"format": 1, "id": "com.example.iss", "name": "ISS", "kind": "json", "fetch": {"url": "https://api.wheretheiss.at/v1/satellites/25544"}});
        assert!(parse(&json_kind, false).unwrap_err().contains("pick.title"));
        let mut ok = json_kind.clone();
        ok["pick"] = json!({"title": "$.latitude", "body": "$[x]"});
        assert!(parse(&ok, false).unwrap_err().contains("pick.body"));
        ok["pick"] = json!({"title": "$.latitude", "body": "$.longitude"});
        assert!(parse(&ok, false).is_ok());

        let cmd = json!({"format": 1, "id": "com.example.git", "name": "Git", "kind": "command", "run": {"path": GIT, "args": ["status"], "every": "5m", "timeout": "10s"}});
        let m = parse(&cmd, false).unwrap();
        assert_eq!((m.run.clone().unwrap().timeout, m.every), (10, 300));
        let mut bad = cmd.clone();
        bad["run"]["path"] = json!("git");
        assert!(parse(&bad, false).unwrap_err().contains("full path"));
        bad["run"] = json!({"path": GIT, "timeout": "2m"});
        assert!(parse(&bad, false).unwrap_err().contains("timeout"));
        let text = json!({"format": 1, "id": "com.example.t", "name": "T", "kind": "text", "every": "5s", "card": {"template": "{{now:HH:mm}}"}});
        assert!(parse(&text, false).unwrap_err().contains("10s"));
    }

    /// The examples in plugins/examples are valid manifests.
    #[test]
    fn examples() {
        for (src, kind) in [
            (include_str!("../../../plugins/examples/hacker-news.rattleboard-plugin.json"), "rss"),
            (include_str!("../../../plugins/examples/iss-position.rattleboard-plugin.json"), "json"),
            (include_str!("../../../plugins/examples/git-status.rattleboard-plugin.json"), "command"),
        ] {
            // The git example's /usr/bin/git isn't an absolute path on Windows.
            if cfg!(windows) && kind == "command" {
                continue;
            }
            let m = parse(&serde_json::from_str(src).unwrap(), false).unwrap();
            assert_eq!(m.kind, kind);
        }
    }

    #[test]
    fn durations_and_schedules() {
        for (s, n) in [("30s", 30), ("15m", 900), ("1h", 3600), ("1h30m", 5400), ("90", 90), ("0", 0)] {
            assert_eq!(duration(&json!(s)).unwrap(), n, "{s}");
        }
        assert_eq!(duration(&json!(45)).unwrap(), 45);
        for bad in ["", "m", "5x", "1.5m", "abc"] {
            assert!(duration(&json!(bad)).is_err(), "{bad}");
        }
        let s = Schedule::parse(&json!({"days": ["mon", "tue", "wed", "thu", "fri"], "from": "07:00", "to": "10:00"})).unwrap().unwrap();
        // 2026-10-19 is a Monday
        assert!(s.active(at(2026, 10, 19, 8, 0)));
        assert!(!s.active(at(2026, 10, 19, 10, 0)));
        assert!(!s.active(at(2026, 10, 18, 8, 0)));
        assert_eq!(s.describe(), "Weekdays 07:00–10:00");
        // Overnight on Friday: Saturday 02:00 counts, Friday 02:00 doesn't.
        let night = Schedule::parse(&json!({"days": ["fri"], "from": "22:00", "to": "06:00"})).unwrap().unwrap();
        assert!(night.active(at(2026, 10, 23, 23, 0)) && night.active(at(2026, 10, 24, 2, 0)));
        assert!(!night.active(at(2026, 10, 23, 2, 0)) && !night.active(at(2026, 10, 24, 23, 0)));
        assert!(Schedule::parse(&Value::Null).unwrap().is_none());
        assert_eq!(Schedule::parse(&s.to_json()).unwrap().unwrap(), s);
    }

    #[test]
    fn paths() {
        let v = json!({"list": [{"id": 1, "title": "A", "tags": ["x", "y"]}, {"id": 2, "title": "B"}], "a.b": {"c": 1.23456}});
        let sel = |p: &str| select(&v, &parse_path(p).unwrap()).into_iter().cloned().collect::<Vec<_>>();
        assert_eq!(sel("$.list[*].title"), [json!("A"), json!("B")]);
        assert_eq!(sel("$.list[1].id"), [json!(2)]);
        assert_eq!(sel("$['a.b'].c"), [json!(1.23456)]);
        assert_eq!(sel("$.list[0].tags[*]"), [json!("x"), json!("y")]);
        assert_eq!(sel("$").len(), 1);
        assert!(sel("$.nope").is_empty() && sel("$.list[9]").is_empty());
        for bad in ["list", "$.", "$[x]", "$[1", "$..a"] {
            assert!(parse_path(bad).is_err(), "{bad}");
        }
        assert_eq!(value_text(&json!(1.23456)), "1.23");
        assert_eq!(value_text(&json!(-37.0)), "-37");
        assert_eq!(value_text(&json!("<b>Hi</b> {r} &amp; “you”")), "Hi (r) & \"you\"");
        let d = parse_json(br#"{"list": [{"id": 7, "title": "Launch", "date": "2026-10-18T09:00:00Z"}]}"#, &Pick { items: Some("$.list[*]".into()), id: Some("$.id".into()), title: Some("$.title".into()), time: Some("$.date".into()), ..Default::default() }, 5).unwrap();
        assert_eq!((d.single, d.items[0].title.as_str(), d.items[0].time), (false, "Launch", Some(1_792_314_000_000)));
        let one = parse_json(br#"{"latitude": -37.81, "longitude": 144.96}"#, &Pick { title: Some("$.latitude".into()), body: Some("$.longitude".into()), ..Default::default() }, 5).unwrap();
        assert_eq!((one.single, one.items[0].title.as_str(), one.items[0].body.as_str()), (true, "-37.81", "144.96"));
        assert!(parse_json(b"<html>", &Pick::default(), 5).is_err());
        assert!(parse_json(b"{}", &Pick { items: Some("$.list[*]".into()), ..Default::default() }, 5).unwrap_err().contains("Nothing found"));
    }

    #[test]
    fn placeholders() {
        let settings: Map<String, Value> = serde_json::from_value(json!({"city": "New York", "n": 3})).unwrap();
        let secret = |k: &str| if k == "token" { "s3cr3t".to_string() } else { String::new() };
        let item = Item { title: "Hello".into(), body: "World".into(), time: None, ..Default::default() };
        let ctx = Ctx { name: "News", item: Some(&item), settings: &settings, secret: Some(&secret), now: at(2026, 10, 18, 21, 5), now_ms: 0, clock24: false, list: &[], row: None };
        assert_eq!(render("https://x.io/w?q={{settings.city}}&k={{secret.token}}", &ctx, Mode::Url), "https://x.io/w?q=New%20York&k=s3cr3t");
        assert_eq!(render("{{settings.city}}", &ctx, Mode::Url), "New York");
        assert_eq!(render("Bearer {{secret.token}}", &ctx, Mode::Header), "Bearer s3cr3t");
        assert_eq!(render("{{name}}: {{title}} / {{body}} {{secret.token}}", &ctx, Mode::Card), "News: Hello / World ");
        assert_eq!(render("{{now:ddd d MMM}} {{now:HH:mm}} {{now}} {{now:h:mm tt}}", &ctx, Mode::Card), "Sun 18 Oct 21:05 9:05 9:05 PM");
        assert_eq!(render("{{nope}}{{settings.n}} {{ unclosed", &ctx, Mode::Card), "3 {{ unclosed");
        assert_eq!(format_time(at(2026, 1, 2, 0, 7), "yyyy-MM-dd hh:mm [at] dddd MMMM"), "2026-01-02 12:07 at Friday January");
    }

    // ------------------------------------------------------------ format 1.1

    fn ctx_at<'a>(settings: &'a Map<String, Value>, item: Option<&'a Item>, list: &'a [Item], row: Option<&'a str>) -> Ctx<'a> {
        // 2026-10-19 09:38:30 UTC; the local time is given as the same wall time for the test
        let now_ms = chrono::DateTime::parse_from_rfc3339("2026-10-19T09:38:30Z").unwrap().timestamp_millis();
        Ctx { name: "Trains", item, settings, secret: None, now: NaiveDate::from_ymd_opt(2026, 10, 19).unwrap().and_hms_opt(9, 38, 30).unwrap(), now_ms, clock24: false, list, row }
    }

    fn json_plugin(pick: Value, card: Value) -> Result<Manifest, String> {
        parse(&json!({"format": 1, "id": "com.example.list", "name": "List", "kind": "json", "fetch": {"url": "https://example.com/x"},
            "pick": pick, "card": card, "settings": [{"key": "phase", "type": "select", "default": "departures", "options": ["departures", "arrivals", "airborne"]}, {"key": "dir", "type": "text", "default": ""}]}), false)
    }

    #[test]
    fn manifest_format_1_1() {
        let ok = json_plugin(json!({"items": "$.ac[*]", "title": "$.flight", "as": "status",
            "fields": {"type": "$.t", "alt": ["$.alt_baro", "$.alt_geom"]},
            "where": [{"path": "$.alt_baro", "op": "!=", "value": "ground"}, {"path": "$.baro_rate", "op": ">", "value": 300, "when": {"setting": "phase", "is": "departures"}}, {"path": "$.r", "op": "exists"}],
            "sort": {"path": "$.alt_baro", "order": "asc", "numeric": true}}),
            json!({"template": "{{settings.phase|upper}}\n{{rows}}", "row": "{{title|pad:8}}{{field.type}} {{field.alt|num}} FT", "empty": "Nothing nearby"})).unwrap();
        assert_eq!(ok.pick.fields, vec![("alt".to_string(), vec!["$.alt_baro".to_string(), "$.alt_geom".to_string()]), ("type".to_string(), vec!["$.t".to_string()])]);
        assert_eq!(ok.pick.conds.len(), 3);
        assert_eq!(ok.pick.conds[1].when, Some(("phase".to_string(), vec!["departures".to_string()])));
        assert_eq!(ok.pick.sort, Some(SortSpec { path: "$.alt_baro".into(), desc: false, numeric: Some(true) }));
        assert!(ok.pick.status && ok.card.row.is_some() && ok.card.empty.is_some());
        for (pick, card, msg) in [
            (json!({"items": "$.a[*]", "title": "$.t", "where": [{"path": "$.x", "op": "=~", "value": 1}]}), json!({}), "op \"=~\""),
            (json!({"items": "$.a[*]", "title": "$.t", "where": [{"path": "$.x", "op": "=="}]}), json!({}), "needs a value"),
            (json!({"items": "$.a[*]", "title": "$.t", "where": [{"path": "$.x", "op": "exists", "value": 1}]}), json!({}), "takes no value"),
            (json!({"items": "$.a[*]", "title": "$.t", "where": [{"path": "x", "op": "exists"}]}), json!({}), "starts with $"),
            (json!({"items": "$.a[*]", "title": "$.t", "where": [{"path": "$.x", "op": "==", "value": [1]}]}), json!({}), "string, number"),
            (json!({"items": "$.a[*]", "title": "$.t", "where": [{"path": "$.x", "op": "==", "value": "{{secret.k}}"}]}), json!({}), "Secrets"),
            (json!({"items": "$.a[*]", "title": "$.t", "where": [{"path": "$.x", "op": "==", "value": 1, "when": {"setting": "nope", "is": "a"}}]}), json!({}), "when.setting"),
            (json!({"title": "$.t", "where": [{"path": "$.x", "op": "exists"}]}), json!({}), "need pick.items"),
            (json!({"items": "$.a[*]", "title": "$.t", "sort": {"path": "$.x", "order": "up"}}), json!({}), "asc or desc"),
            (json!({"items": "$.a[*]", "title": "$.t", "sort": {"path": "$.x", "numeric": "yes"}}), json!({}), "numeric"),
            (json!({"items": "$.a[*]", "title": "$.t", "fields": {"a b": "$.x"}}), json!({}), "pick.fields name"),
            (json!({"items": "$.a[*]", "title": "$.t", "fields": {"a": "$[x]"}}), json!({}), "pick.fields.a"),
            (json!({"items": "$.a[*]", "title": "$.t", "fields": {"a": []}}), json!({}), "up to 5 paths"),
            (json!({"items": "$.a[*]", "title": "$.t", "as": "table"}), json!({}), "feed or status"),
            (json!({"items": "$.a[*]", "title": "$.t"}), json!({"row": "{{title}}"}), "card.row and card.empty"),
            (json!({"title": "$.t"}), json!({"template": "{{title|shout}}"}), "unknown filter |shout"),
            (json!({"title": "$.t"}), json!({"template": "{{title|pad:0}}"}), "|pad:0"),
            (json!({"title": "$.t"}), json!({"template": "{{title|round:9}}"}), "|round:9"),
            (json!({"title": "$.t"}), json!({"template": "{{title|default}}"}), "|default"),
        ] {
            let e = json_plugin(pick.clone(), card.clone()).unwrap_err();
            assert!(e.contains(msg), "{pick} {card}: {e}");
        }
        // Only json plugins pick fields
        let rss = json!({"format": 1, "id": "com.example.r", "name": "R", "kind": "rss", "fetch": {"url": "https://example.com/f"}, "pick": {"fields": {"a": "$.a"}}});
        assert!(parse(&rss, false).unwrap_err().contains("json plugins"));
        // fetch.sign
        let signed = |sign: Value| parse(&json!({"format": 1, "id": "com.example.ptv", "name": "PTV", "kind": "json",
            "fetch": {"url": "https://timetableapi.ptv.vic.gov.au/v3/departures/route_type/0/stop/{{settings.stop}}?devid={{settings.devid}}", "sign": sign},
            "pick": {"title": "$.status.version"}, "settings": [{"key": "stop"}, {"key": "devid"}], "secrets": [{"key": "key"}]}), false);
        let m = signed(json!({"type": "hmac-sha1", "key": "{{secret.key}}"})).unwrap();
        assert_eq!(m.fetch.unwrap().sign, Some(Sign { kind: "hmac-sha1".into(), key: "{{secret.key}}".into(), param: "signature".into(), upper: true }));
        let m = signed(json!({"type": "hmac-sha256", "key": "{{secret.key}}", "over": "path+query", "param": "sig", "case": "lower"})).unwrap();
        assert_eq!(m.fetch.unwrap().sign.unwrap(), Sign { kind: "hmac-sha256".into(), key: "{{secret.key}}".into(), param: "sig".into(), upper: false });
        for (sign, msg) in [
            (json!({"type": "md5", "key": "k"}), "hmac-sha1 or hmac-sha256"),
            (json!({"type": "hmac-sha1"}), "fetch.sign.key"),
            (json!({"type": "hmac-sha1", "key": "k", "over": "url"}), "path+query"),
            (json!({"type": "hmac-sha1", "key": "k", "param": "a b"}), "fetch.sign.param"),
            (json!({"type": "hmac-sha1", "key": "k", "case": "mixed"}), "upper or lower"),
            (json!("hmac"), "must be an object"),
        ] {
            let e = signed(sign.clone()).unwrap_err();
            assert!(e.contains(msg), "{sign}: {e}");
        }
    }

    /// adsb.lol's /v2/point answer, cut down: one on the ground, one landing, two climbing.
    const PLANES: &str = r#"{"ac": [
        {"hex": "7c6b2d", "flight": "QFA401  ", "r": "VH-VXA", "t": "B738", "alt_baro": 3200, "baro_rate": 1856},
        {"hex": "7c4ef1", "flight": "JST512  ", "r": "VH-VFN", "t": "A320", "alt_baro": "ground"},
        {"hex": "7c7a3f", "flight": "VOZ825 ", "r": "VH-IJR", "t": "B38M", "alt_baro": 5900, "baro_rate": 2240},
        {"hex": "7c0156", "flight": "QFA2", "r": "VH-OQA", "t": "A388", "alt_baro": 2400, "baro_rate": -768},
        {"hex": "7c3311", "r": "VH-ZNA", "t": "B789", "alt_baro": 1100, "baro_rate": 960}
        ], "msg": "No error", "now": 1792402710000, "total": 5}"#;

    fn planes_pick(phase: &str) -> Pick {
        let m = json_plugin(json!({"items": "$.ac[*]", "id": "$.hex", "title": "$.flight", "as": "status",
            "fields": {"flight": ["$.flight", "$.r"], "type": "$.t", "alt": "$.alt_baro"},
            "where": [{"path": "$.alt_baro", "op": "!=", "value": "ground"},
                      {"path": "$.baro_rate", "op": ">", "value": 300, "when": {"setting": "phase", "is": "departures"}},
                      {"path": "$.baro_rate", "op": "<", "value": -300, "when": {"setting": "phase", "is": ["arrivals"]}}],
            "sort": {"path": "$.alt_baro"}}), json!({})).unwrap();
        let settings: Map<String, Value> = serde_json::from_value(json!({"phase": phase})).unwrap();
        m.pick.resolve(&ctx_at(&settings, None, &[], None))
    }

    #[test]
    fn where_sort_and_fields() {
        let titles = |d: &Data| d.items.iter().map(|i| i.fields["flight"].clone()).collect::<Vec<_>>();
        let d = parse_json(PLANES.as_bytes(), &planes_pick("departures"), 10).unwrap();
        // Climbing only, lowest first; the one with no callsign falls back to its registration
        assert_eq!(titles(&d), ["VH-ZNA", "QFA401", "VOZ825"]);
        assert!(d.single);
        assert_eq!((d.items[1].fields["type"].as_str(), d.items[1].fields["alt"].as_str()), ("B738", "3200"));
        assert_eq!(titles(&parse_json(PLANES.as_bytes(), &planes_pick("arrivals"), 10).unwrap()), ["QFA2"]);
        assert_eq!(titles(&parse_json(PLANES.as_bytes(), &planes_pick("airborne"), 10).unwrap()), ["VH-ZNA", "QFA2", "QFA401", "VOZ825"]);
        // maxItems applies after where and sort
        assert_eq!(titles(&parse_json(PLANES.as_bytes(), &planes_pick("airborne"), 2).unwrap()), ["VH-ZNA", "QFA2"]);
        // A status list may come out empty; a feed's path that finds nothing is still an error
        let none = parse_json(br#"{"ac": []}"#, &planes_pick("departures"), 10).unwrap();
        assert!(none.items.is_empty() && none.single);
        assert!(parse_json(br#"{"ac": []}"#, &Pick { items: Some("$.ac[*]".into()), ..Default::default() }, 10).unwrap_err().contains("Nothing found"));

        // Each operator, against numbers, numeric strings, words, arrays and missing values
        let x = json!({"n": 5, "s": " 12 ", "w": "Ground", "b": true, "tags": ["Rain", "wind"], "z": null, "o": {"a": 1}});
        let c = |path: &str, op: &str, value: Value| cond_holds(&x, &Cond { path: path.into(), op: op.into(), value, when: None });
        assert!(c("$.n", "==", json!(5)) && c("$.n", "==", json!("5.0")) && c("$.n", "<", json!(12)) && c("$.s", ">", json!(5)));
        assert!(c("$.s", "==", json!(12)) && c("$.s", ">=", json!("12")) && !c("$.s", "<", json!(12)));
        assert!(c("$.w", "==", json!("ground")) && !c("$.w", "!=", json!("GROUND")) && c("$.w", "contains", json!("ROUN")));
        assert!(c("$.w", ">", json!("apple")) && c("$.w", "<", json!("h")));
        assert!(c("$.n", "!=", json!("five")) && c("$.n", "contains", json!("5")));
        assert!(c("$.b", "==", json!(true)) && c("$.b", "==", json!("TRUE")));
        assert!(c("$.tags", "contains", json!("rain")) && !c("$.tags", "contains", json!("ra")) && !c("$.tags", "==", json!("Rain")));
        assert!(!c("$.o", "==", json!(1)) && c("$.o", "exists", Value::Null));
        assert!(c("$.z", "missing", Value::Null) && c("$.nope", "missing", Value::Null) && !c("$.nope", "exists", Value::Null));
        assert!(!c("$.nope", "!=", json!(1)) && !c("$.z", "!=", json!(1)), "a missing value never compares");

        // Sorting: numeric, text, descending, missing keys last, stable ties
        let list = json!([{"k": "10", "i": 0}, {"k": 9, "i": 1}, {"i": 2}, {"k": "b", "i": 3}, {"k": 10, "i": 4}]);
        let order = |s: SortSpec, items: &Value| {
            let mut v: Vec<&Value> = items.as_array().unwrap().iter().collect();
            sort_items(&mut v, &s);
            v.iter().map(|x| x["i"].as_u64().unwrap()).collect::<Vec<_>>()
        };
        let sort = |numeric, desc| SortSpec { path: "$.k".into(), desc, numeric };
        // Automatic: "b" isn't a number, so it's text order ("10" < "9" < "b")
        assert_eq!(order(sort(None, false), &list), [0, 4, 1, 3, 2]);
        // Numeric: "b" isn't a number, so like the missing key it goes last, in list order
        assert_eq!(order(sort(Some(true), false), &list), [1, 0, 4, 2, 3]);
        assert_eq!(order(sort(Some(true), true), &list), [0, 4, 1, 2, 3]);
        assert_eq!(order(sort(Some(false), true), &list), [3, 1, 0, 4, 2]);
        let nums = json!([{"k": 3, "i": 0}, {"k": "1.5", "i": 1}, {"i": 2}, {"k": -2, "i": 3}]);
        assert_eq!(order(sort(None, false), &nums), [3, 1, 0, 2]);
        assert_eq!(order(sort(None, true), &nums), [0, 1, 3, 2]);
    }

    #[test]
    fn where_takes_settings() {
        let m = json_plugin(json!({"items": "$.d[*]", "title": "$.p", "as": "status",
            "where": [{"path": "$.direction_id", "op": "==", "value": "{{settings.dir}}"}]}), json!({})).unwrap();
        let data = br#"{"d": [{"p": "a", "direction_id": 1}, {"p": "b", "direction_id": 2}]}"#;
        let with = |dir: &str| {
            let settings: Map<String, Value> = serde_json::from_value(json!({"dir": dir, "phase": "departures"})).unwrap();
            let pick = m.pick.resolve(&ctx_at(&settings, None, &[], None));
            parse_json(data, &pick, 10).unwrap().items.iter().map(|i| i.title.clone()).collect::<Vec<_>>()
        };
        assert_eq!(with("2"), ["b"]);
        // A blank setting switches its condition off
        assert_eq!(with(""), ["a", "b"]);
        assert_eq!(with("  "), ["a", "b"]);
    }

    #[test]
    fn filters_and_epochs() {
        let mut fields = std::collections::BTreeMap::new();
        for (k, v) in [("alt", "3200"), ("big", "-1234567.891"), ("tiny", "-0.004"), ("pi", "3.14159"), ("half", "2.5"), ("neg", "-2.5"),
            ("word", "ground"), ("blank", ""), ("est", "2026-10-19T09:41:00Z"), ("epoch", "1792402800"), ("flight", "QFA401"), ("e5", "1e5")] {
            fields.insert(k.to_string(), v.to_string());
        }
        let item = Item { title: "  Hello  World ".into(), fields, ..Default::default() };
        let settings: Map<String, Value> = serde_json::from_value(json!({"city": " melbourne "})).unwrap();
        let ctx = ctx_at(&settings, Some(&item), &[], None);
        let r = |t: &str| render(t, &ctx, Mode::Card);
        assert_eq!(r("{{field.alt|num}} {{field.big|num}} {{field.tiny|num}} {{field.word|num}} {{field.e5|num}}"), "3,200 -1,234,567.89 0 ground 100,000");
        assert_eq!(r("{{field.pi|round}} {{field.pi|round:2}} {{field.half|round}} {{field.neg|round}} {{field.alt|round:1}} {{field.word|round}}"), "3 3.14 3 -3 3200.0 ground");
        assert_eq!(r("[{{settings.city|trim|upper}}] [{{settings.city|lower}}] {{title|trim}}"), "[MELBOURNE] [ melbourne ] Hello  World");
        assert_eq!(r("{{field.blank|default:No data}} {{field.word|default:x}} {{field.nope|default:-}}"), "No data ground -");
        assert_eq!(r("{{field.flight|pad:8}}|{{field.alt|pad:-6}}|{{field.flight|pad:3}}"), "QFA401{0}{0}|{0}{0}3200|QFA401");
        assert_eq!(crate::vestaboard::parse(&r("{{field.flight|pad:8}}B738")).len(), 12);
        // Times: an ISO or epoch field in local time (the test's "local" is UTC), and minutes from now
        assert_eq!(r("{{field.est|time}} {{field.est|time:HH:mm:ss}} {{field.epoch|time:h:mm tt}} {{field.word|time}}"), plugins_local(&["2026-10-19T09:41:00Z", "2026-10-19T09:41:00Z", "2026-10-19T09:40:00Z"], &["h:mm", "HH:mm:ss", "h:mm tt"]) + " ground");
        assert_eq!(r("{{field.est|mins}} {{field.epoch|mins}} {{field.word|mins}}"), "2 1 ground");
        // Epochs and offsets
        assert_eq!(r("{{now:epoch}} {{now:epochms}} {{now-2h:epoch}} {{now+90m:epoch}} {{now+1h30m:HH:mm}} {{now-1d:ddd}}"), "1792402710 1792402710000 1792395510 1792408110 11:08 Sun");
        assert_eq!(r("{{now*2:epoch}}{{now+2x:epoch}}"), "");
        // Filters run left to right; a filter that doesn't apply leaves the value alone
        assert_eq!(r("{{field.alt|num|default:none}} {{field.blank|default:12000|num}}"), "3,200 12,000");
        // URLs: filtered, then encoded
        assert_eq!(render("https://x.io/?q={{settings.city|trim|upper}}", &ctx, Mode::Url), "https://x.io/?q=MELBOURNE");
    }

    /// The local-time rendering of instants, for tests that run in any time zone.
    fn plugins_local(isos: &[&str], fmts: &[&str]) -> String {
        isos.iter().zip(fmts).map(|(t, f)| format_time(local_of(chrono::DateTime::parse_from_rfc3339(t).unwrap().timestamp_millis()), f)).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn status_rows() {
        let row = "{{field.flight|pad:8}}{{field.type|pad:5}}{{field.alt|num}} FT";
        let pick = planes_pick("departures");
        let d = parse_json(PLANES.as_bytes(), &pick, 3).unwrap();
        let settings: Map<String, Value> = serde_json::from_value(json!({"phase": "departures"})).unwrap();
        let ctx = ctx_at(&settings, d.items.first(), &d.items, Some(row));
        let text = render("MEL {{settings.phase}} ({{count}})\n{{rows}}", &ctx, Mode::Card);
        assert_eq!(text, "MEL departures (3)\nVH-ZNA{0}{0}B789{0}1,100 FT\nQFA401{0}{0}B738{0}3,200 FT\nVOZ825{0}{0}B38M{0}5,900 FT");
        let g = crate::vestaboard::text_layout(&text, 6, 22);
        let line = |r: usize| g[r].iter().map(|&c| match c { 0 => ' ', 1..=26 => (b'A' + c - 1) as char, 27..=35 => (b'1' + c - 27) as char, 36 => '0', 44 => '-', 55 => ',', 41 => '(', 42 => ')', _ => '?' }).collect::<String>();
        assert_eq!(line(1), "  MEL DEPARTURES (3)  ");
        assert_eq!(line(2), "VH-ZNA  B789 1,100 FT ");
    }

    #[test]
    fn links() {
        for (s, want) in [
            ("https://example.com/a", Some("https://example.com/a")),
            ("  http://Example.COM  ", Some("http://example.com/")),
            ("https://news.ycombinator.com/item?id=1&x=%20y", Some("https://news.ycombinator.com/item?id=1&x=%20y")),
            ("javascript:alert(1)", None),
            ("JavaScript:alert(1)", None),
            ("file:///etc/passwd", None),
            ("data:text/html,<b>x</b>", None),
            ("mailto:sam@example.com", None),
            ("ftp://example.com/x", None),
            ("/relative/path", None),
            ("//example.com/protocol-relative", None),
            ("example.com", None),
            ("https://user:pw@example.com/", None),
            ("https://", None),
            ("", None),
        ] {
            assert_eq!(safe_link(s).as_deref(), want, "{s}");
        }
        assert!(safe_link(&format!("https://example.com/{}", "a".repeat(2100))).is_none());
        // Feeds: RSS <link>, Atom's alternate link; a bad one is dropped
        let d = parse_feed(RSS.as_bytes(), 10).unwrap();
        let show = d.items.iter().find(|i| i.title.starts_with("Show HN")).unwrap();
        assert_eq!(show.link.as_deref(), Some("https://a/"));
        assert_eq!(d.items.iter().find(|i| i.title.starts_with("Rust")).unwrap().link, None);
        let atom = r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"><title>T</title><id>t</id><updated>2026-10-18T00:00:00Z</updated>
            <entry><title>One</title><id>urn:1</id><updated>2026-10-18T08:00:00Z</updated><link rel="self" href="https://example.com/self"/><link rel="alternate" href="https://example.com/one"/></entry>
            <entry><title>Two</title><id>urn:2</id><updated>2026-10-18T07:00:00Z</updated><link href="javascript:alert(1)"/></entry></feed>"#;
        let d = parse_feed(atom.as_bytes(), 10).unwrap();
        assert_eq!((d.items[0].link.as_deref(), d.items[1].link.as_deref()), (Some("https://example.com/one"), None));
        // iCal URL
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-19T00:00:00Z").unwrap().timestamp_millis();
        let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:u1\r\nSUMMARY:Standup\r\nURL:https://meet.example.com/standup\r\nDTSTART:20261019T093000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        assert_eq!(parse_ical(ics.as_bytes(), now).unwrap().items[0].link.as_deref(), Some("https://meet.example.com/standup"));
        // pick.link: a string at the path, checked
        let m = json_plugin(json!({"items": "$.hits[*]", "title": "$.title", "link": "$.url"}), json!({})).unwrap();
        assert_eq!(m.pick.link.as_deref(), Some("$.url"));
        let d = parse_json(br#"{"hits": [{"title": "A", "url": "https://a.example/1"}, {"title": "B", "url": "javascript:x"}, {"title": "C", "url": 5}]}"#, &m.pick, 10).unwrap();
        assert_eq!(d.items.iter().map(|i| i.link.as_deref()).collect::<Vec<_>>(), [Some("https://a.example/1"), None, None]);
        assert!(json_plugin(json!({"items": "$.hits[*]", "title": "$.title", "link": "$[bad"}), json!({})).unwrap_err().contains("pick.link"));
        // A command's JSON output can carry one too
        assert_eq!(parse_output(br#"{"text": "Build failed", "link": "https://ci.example.com/1"}"#).items[0].link.as_deref(), Some("https://ci.example.com/1"));
    }

    #[test]
    fn hmac_signing() {
        // RFC 2104 examples (as on Wikipedia's HMAC page)
        assert_eq!(hmac_hex("hmac-sha1", b"key", b"The quick brown fox jumps over the lazy dog", false), "de7c9b85b8b78aa6bc8a7a36f70a90701c9db4d9");
        assert_eq!(hmac_hex("hmac-sha256", b"key", b"The quick brown fox jumps over the lazy dog", false), "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8");
        // PTV's scheme: HMAC-SHA1 of "/v3/...?...&devid=...", upper-case hex, as &signature=
        let sign = Sign { kind: "hmac-sha1".into(), key: "{{secret.key}}".into(), param: "signature".into(), upper: true };
        let key = "aaaabbbb-cccc-dddd-eeee-ffff00001111";
        assert_eq!(
            sign_url("https://timetableapi.ptv.vic.gov.au/v3/departures/route_type/0/stop/1071?max_results=3&devid=1000123", &sign, key).unwrap(),
            "https://timetableapi.ptv.vic.gov.au/v3/departures/route_type/0/stop/1071?max_results=3&devid=1000123&signature=1D59EE8EAE144597AB8AC82636D50369F8C0A1F0"
        );
        assert_eq!(
            sign_url("https://timetableapi.ptv.vic.gov.au/v3/search/Flinders%20Street?route_types=0&devid=1000123#x", &sign, key).unwrap(),
            "https://timetableapi.ptv.vic.gov.au/v3/search/Flinders%20Street?route_types=0&devid=1000123&signature=E3AD26614071C8DCCCEA2CD1E3D0846719CCA5F9"
        );
        // No query yet: the signature starts it; lower case on request
        let lower = Sign { upper: false, param: "sig".into(), ..sign.clone() };
        let u = sign_url("https://example.com/v3/disruptions/stop/1071", &lower, key).unwrap();
        assert_eq!(u, format!("https://example.com/v3/disruptions/stop/1071?sig={}", hmac_hex("hmac-sha1", key.as_bytes(), b"/v3/disruptions/stop/1071", false)));
        assert_eq!(sign_url("https://example.com/a?", &lower, key).unwrap(), format!("https://example.com/a?sig={}", hmac_hex("hmac-sha1", key.as_bytes(), b"/a", false)));
        // No key: refused, without the URL in the message
        let e = sign_url("https://example.com/a?devid=1000123", &sign, "").unwrap_err();
        assert!(e.contains("signing key") && !e.contains("1000123"));
    }

    #[test]
    fn schemes() {
        let ok = |u: &str| check_scheme(&url::Url::parse(u).unwrap()).is_ok();
        assert!(ok("https://example.com/x") && ok("http://127.0.0.1:9/") && ok("http://10.0.0.2/") && ok("http://172.20.1.1/"));
        assert!(ok("http://192.168.1.4/") && ok("http://[::1]/") && ok("http://nas.local/") && ok("http://homeassistant:8123/"));
        assert!(!ok("http://example.com/") && !ok("http://8.8.8.8/") && !ok("http://172.32.0.1/") && !ok("file:///etc/passwd"));
    }

    const RSS: &str = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>HN</title>
        <item><title>Show HN: A split-flap board</title><link>https://a</link><guid>a1</guid><pubDate>Sat, 18 Oct 2026 09:00:00 +0000</pubDate><description>&lt;p&gt;Flaps &amp;amp; sound&lt;/p&gt;</description></item>
        <item><title>Rust 2.0 – released</title><guid>a2</guid><pubDate>Sat, 18 Oct 2026 10:00:00 +0000</pubDate></item>
        </channel></rss>"#;

    #[test]
    fn feeds() {
        let d = parse_feed(RSS.as_bytes(), 10).unwrap();
        assert!(!d.single);
        assert_eq!(d.items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Rust 2.0 - released", "Show HN: A split-flap board"]);
        assert_eq!(d.items[1].body, "Flaps & sound");
        assert_ne!(d.items[0].key, d.items[1].key);
        assert_eq!(parse_feed(RSS.as_bytes(), 1).unwrap().items.len(), 1);
        let atom = r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"><title>T</title><id>t</id><updated>2026-10-18T00:00:00Z</updated>
            <entry><title>Atom entry</title><id>urn:1</id><updated>2026-10-18T08:00:00Z</updated><summary>Sum</summary></entry></feed>"#;
        let d = parse_feed(atom.as_bytes(), 10).unwrap();
        assert_eq!((d.items[0].title.as_str(), d.items[0].body.as_str()), ("Atom entry", "Sum"));
        assert!(parse_feed(b"{\"not\": \"a feed\"}", 10).is_err());
    }

    #[test]
    fn calendars() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-19T00:00:00Z").unwrap().timestamp_millis();
        let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n\
            BEGIN:VEVENT\r\nUID:u1\r\nSUMMARY:Standup\r\nLOCATION:Room 4B\\, level 2\r\nDTSTART:20261019T093000Z\r\nDTEND:20261019T094500Z\r\nRRULE:FREQ=DAILY;COUNT=3\r\nEND:VEVENT\r\n\
            BEGIN:VEVENT\r\nUID:u2\r\nSUMMARY:Dentist\r\nDTSTART;TZID=Australia/Melbourne:20261020T140000\r\nDTEND;TZID=Australia/Melbourne:20261020T150000\r\nEND:VEVENT\r\n\
            BEGIN:VEVENT\r\nUID:u3\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20261021\r\nEND:VEVENT\r\n\
            BEGIN:VEVENT\r\nUID:old\r\nSUMMARY:Long ago\r\nDTSTART:20200101T000000Z\r\nEND:VEVENT\r\n\
            END:VCALENDAR\r\n";
        let d = parse_ical(ics.as_bytes(), now).unwrap();
        let titles: Vec<_> = d.items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles.iter().filter(|t| **t == "Standup").count(), 3);
        assert!(!titles.contains(&"Long ago"));
        assert_eq!(d.items[0].body, "Room 4B, level 2");
        assert_eq!(d.items[0].end.unwrap() - d.items[0].time.unwrap(), 15 * 60_000);
        let dentist = d.items.iter().find(|i| i.title == "Dentist").unwrap();
        // 14:00 in Melbourne (UTC+11 in October) is 03:00 UTC
        assert_eq!(dentist.time, Some(chrono::DateTime::parse_from_rfc3339("2026-10-20T03:00:00Z").unwrap().timestamp_millis()));
        assert!(d.items.iter().find(|i| i.title == "Holiday").unwrap().all_day);
        assert!(d.items.windows(2).all(|w| w[0].time <= w[1].time));
        assert!(parse_ical(b"not a calendar", now).is_err());
    }

    #[test]
    fn outputs() {
        let d = parse_output(b"## main...origin/main [ahead 1]\n M src/lib.rs\n\n");
        assert_eq!((d.single, d.items[0].title.as_str()), (true, "## main...origin/main [ahead 1]\n M src/lib.rs"));
        let d = parse_output(br#"{"text": "Build failed", "priority": "alert", "id": "b42"}"#);
        assert!(d.items[0].alert && d.items[0].title == "Build failed" && d.items[0].key == hash(&["b42"]));
        let d = parse_output(br#"{"characters": [[1,2,3]]}"#);
        assert_eq!(d.items[0].characters, Some(vec![vec![1, 2, 3]]));
    }

    /// The 2 MB cap and the timeout, against a local server (plain HTTP is fine on loopback).
    #[tokio::test]
    async fn fetching() {
        use axum::routing::get;
        let app = axum::Router::new()
            .route("/small", get(|| async { "ok" }))
            .route("/big", get(|| async { "x".repeat(MAX_RESPONSE + 1) }))
            .route("/stream", get(|| async {
                let chunks = (0..300).map(|_| Ok::<_, std::io::Error>(axum::body::Bytes::from(vec![b'y'; 8192])));
                axum::body::Body::from_stream(futures_util::stream::iter(chunks))
            }))
            .route("/redirect", get(|| async { axum::response::Redirect::temporary("http://example.com/") }))
            .route("/slow", get(|| async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                "late"
            }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await });
        let c = client();
        assert_eq!(fetch(&c, "GET", &format!("{base}/small"), &[]).await.unwrap(), b"ok");
        assert!(fetch(&c, "GET", &format!("{base}/big"), &[]).await.unwrap_err().contains("2 MB"));
        assert!(fetch(&c, "GET", &format!("{base}/stream"), &[]).await.unwrap_err().contains("2 MB"));
        assert!(fetch(&c, "GET", &format!("{base}/redirect"), &[]).await.unwrap_err().contains("Redirected"));
        assert!(fetch(&c, "GET", "http://example.com/", &[]).await.unwrap_err().contains("own network"));
        let start = std::time::Instant::now();
        let e = fetch(&c, "GET", &format!("{base}/slow?token=SECRET"), &[]).await.unwrap_err();
        assert!(e.contains("Timed out") && !e.contains("SECRET"), "{e}");
        assert!(start.elapsed() < Duration::from_secs(15));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn commands() {
        let run = |path: &str, args: &[&str], timeout: u64| Run { path: path.into(), args: args.iter().map(|s| s.to_string()).collect(), timeout };
        assert_eq!(run_command(&run("/bin/echo", &["hello", "board"], 5)).await.unwrap(), b"hello board\n");
        let start = std::time::Instant::now();
        let e = run_command(&run("/bin/sleep", &["30"], 1)).await.unwrap_err();
        assert!(e.contains("Stopped after 1 s") && start.elapsed() < Duration::from_secs(5), "{e}");
        // Endless output is cut at 64 KB and the program stopped.
        let out = run_command(&run("/usr/bin/yes", &[], 5)).await.unwrap();
        assert_eq!(out.len(), MAX_STDOUT);
        assert!(run_command(&run("/usr/bin/false", &[], 5)).await.unwrap_err().contains("status 1"));
        assert!(run_command(&run("/no/such/program", &[], 5)).await.unwrap_err().contains("wasn't found"));
    }
}
