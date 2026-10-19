//! Sources (protocol v3): the engine that turns plugins into a calm rotation of cards.
//!
//! The rotation is an ordered list of plugin instances, each with an enabled flag, a weight
//! and a dwell (how long its card stays up). Three priorities:
//!   Ambient  the rotation itself
//!   Message  anything else put on the board (Compose, the API, a sequence) holds the rotation
//!            for `holdMinutes` (30 by default)
//!   Alert    a keyword match, an event starting in 10 minutes, a celebration: interrupts for
//!            `alertSeconds`, then the board goes back to what it showed before. A timed
//!            notification (POST /rattleboard/notify with "seconds", an agent's show_status)
//!            interrupts the same way (`begin_notice`/`end_notice`): it never holds the
//!            rotation, and the board it put back is never re-shown as a message
//! Calm by default: feed items are shown once (remembered in seen.json), a burst of new ones
//! becomes one digest card, at most `flipsPerHour` card changes an hour, nothing at all in
//! quiet hours, and each instance can have a schedule.
//!
//! The engine is driven by `tick` (once a second, from lib.rs), which returns what to show;
//! fetches run separately (`due_jobs`, `run_job`, `store`), so a slow feed never holds up the
//! rotation. Nothing here touches windows, and no lock is held across an await.
//!
//! Files: sources.json (the rotation and engine settings) and plugins.json (installed
//! manifests) in the config folder; seen.json in the data folder. None of them hold secrets:
//! each instance's secrets live in the credential store, one entry per instance and key.

use crate::builtins;
use crate::cards::Face;
use crate::plugins::{self, Data, Item, Manifest, Schedule};
use crate::secrets;
use crate::vestaboard::Grid;
use chrono::{NaiveDateTime, Timelike};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// More new items than this in one fetch become a single digest card.
pub const DIGEST_AT: usize = 3;
const SEEN_MAX: usize = 400;
pub const MAX_INSTANCES: usize = 50;
pub const MAX_PLUGINS: usize = 100;
pub const DEFAULT_DWELL: u64 = 300;
pub const MIN_DWELL: u64 = 10;
/// A calendar event alerts this long before it starts.
pub const EVENT_LEAD_MS: i64 = 10 * 60_000;
const HOUR_MS: i64 = 3_600_000;

// ---------------------------------------------------------------- errors

/// An API error: the HTTP status and what to say.
#[derive(Debug, PartialEq)]
pub struct Refused(pub u16, pub String);

fn bad(msg: impl Into<String>) -> Refused {
    Refused(400, msg.into())
}

// ---------------------------------------------------------------- instances and config

#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    pub id: String,
    pub plugin: String,
    pub name: String,
    pub enabled: bool,
    /// 1–10: how often it comes round, relative to the others
    pub weight: u32,
    /// Seconds on the board; 0 uses the plugin's own (or 5 minutes)
    pub dwell: u64,
    pub schedule: Option<Schedule>,
    pub settings: Map<String, Value>,
    pub chip: Option<u8>,
    /// Feed items must mention one of these (if any), and none of `exclude`
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// Feed items mentioning one of these interrupt as an alert
    pub alert_words: Vec<String>,
}

fn words(v: Option<&Value>, what: &str) -> Result<Vec<String>, Refused> {
    let list: Vec<String> = match v {
        None | Some(Value::Null) => vec![],
        Some(Value::String(s)) => s.split(',').map(str::to_string).collect(),
        Some(Value::Array(a)) => a.iter().map(|x| x.as_str().map(str::to_string).ok_or_else(|| bad(format!("{what} must be words")))).collect::<Result<_, _>>()?,
        Some(_) => return Err(bad(format!("{what} must be a list of words"))),
    };
    let list: Vec<String> = list.iter().map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect();
    if list.len() > 20 || list.iter().any(|w| w.len() > 60) {
        return Err(bad(format!("{what}: at most 20 words")));
    }
    Ok(list)
}

pub fn new_id() -> String {
    let mut b = [0u8; 6];
    getrandom::fill(&mut b).expect("OS random source");
    format!("s-{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl Instance {
    /// Validates one instance from the UI or the API; settings missing from it take the
    /// plugin's defaults, and unknown fields are ignored (secrets included: those go
    /// through their own route).
    pub fn parse(v: &Value, plugins: &[Manifest]) -> Result<Instance, Refused> {
        let o = v.as_object().ok_or_else(|| bad("Each source in the rotation is an object"))?;
        let plugin_id = o.get("plugin").and_then(|p| p.as_str()).ok_or_else(|| bad("Each source needs \"plugin\": a plugin id"))?;
        let p = plugins.iter().find(|p| p.id == plugin_id).ok_or_else(|| bad(format!("Unknown plugin {plugin_id}")))?;
        let id = match o.get("id") {
            None | Some(Value::Null) => new_id(),
            Some(Value::String(s)) if valid_id(s) => s.clone(),
            Some(_) => return Err(bad("A source id is up to 40 letters, digits, - or _")),
        };
        let name = match o.get("name") {
            None | Some(Value::Null) => p.name.clone(),
            Some(Value::String(s)) if s.trim().is_empty() => p.name.clone(),
            Some(Value::String(s)) if s.trim().chars().count() <= 40 => s.trim().to_string(),
            Some(_) => return Err(bad("A source's name is up to 40 characters")),
        };
        let enabled = match o.get("enabled") {
            None | Some(Value::Null) => true,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(bad("enabled must be true or false")),
        };
        let weight = match o.get("weight") {
            None | Some(Value::Null) => 1,
            Some(w) => match w.as_u64() {
                Some(n @ 1..=10) => n as u32,
                _ => return Err(bad("weight must be from 1 to 10")),
            },
        };
        let dwell = match o.get("dwell") {
            None | Some(Value::Null) => 0,
            Some(d) => {
                let d = plugins::duration(d).map_err(bad)?;
                if d != 0 && !(MIN_DWELL..=3600).contains(&d) {
                    return Err(bad("dwell must be from 10s to 1h"));
                }
                d
            }
        };
        let schedule = Schedule::parse(o.get("schedule").unwrap_or(&Value::Null)).map_err(bad)?;
        let mut given = match o.get("settings") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(m)) => m.clone(),
            Some(_) => return Err(bad("settings must be an object")),
        };
        builtins::migrate(&p.id, &mut given);
        let mut settings = Map::new();
        for spec in &p.settings {
            let v = match given.get(&spec.key) {
                None | Some(Value::Null) => spec.default.clone(),
                Some(v) => spec.check(v).map_err(|e| bad(format!("{}: {e}", spec.label)))?,
            };
            settings.insert(spec.key.clone(), v);
        }
        let chip = plugins::chip(o.get("chip").unwrap_or(&Value::Null)).map_err(bad)?;
        Ok(Instance {
            id,
            plugin: p.id.clone(),
            name,
            enabled,
            weight,
            dwell,
            schedule,
            settings,
            chip,
            include: words(o.get("include"), "include")?,
            exclude: words(o.get("exclude"), "exclude")?,
            alert_words: words(o.get("alertWords"), "alertWords")?,
        })
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "plugin": self.plugin, "name": self.name, "enabled": self.enabled,
            "weight": self.weight, "dwell": self.dwell, "schedule": self.schedule.as_ref().map(|s| s.to_json()),
            "settings": self.settings, "chip": self.chip.map(plugins::chip_name),
            "include": self.include, "exclude": self.exclude, "alertWords": self.alert_words,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Quiet {
    pub enabled: bool,
    pub from: String,
    pub to: String,
}

impl Quiet {
    pub fn active(&self, at: NaiveDateTime) -> bool {
        self.enabled && crate::settings::in_quiet_hours(&self.from, &self.to, at.hour() * 60 + at.minute())
    }
}

/// Refresh preview fetches again only when the last fetch is at least this old.
pub const REFRESH_FLOOR_MS: i64 = 5_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub rotation: Vec<Instance>,
    pub quiet: Quiet,
    pub hold_minutes: u32,
    pub flips_per_hour: u32,
    pub alert_seconds: u32,
    pub shuffle: bool,
    pub while_mirroring: bool,
    pub clock24: bool,
    /// Where cards go: "this" and/or saved boards and groups (see send.rs)
    pub targets: Vec<String>,
    /// Command plugins the user approved, by id: the manifest's fingerprint at the time
    pub approvals: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rotation: vec![],
            quiet: Quiet { enabled: false, from: "22:00".into(), to: "07:00".into() },
            hold_minutes: 30,
            flips_per_hour: 30,
            alert_seconds: 15,
            shuffle: false,
            while_mirroring: false,
            clock24: false,
            targets: vec![crate::send::HERE.into()],
            approvals: BTreeMap::new(),
        }
    }
}

impl Config {
    /// The engine's settings as JSON (the rotation is added separately, with secrets' state).
    fn settings_json(&self) -> Map<String, Value> {
        let v = json!({
            "quietHours": { "enabled": self.quiet.enabled, "from": self.quiet.from, "to": self.quiet.to },
            "holdMinutes": self.hold_minutes, "flipsPerHour": self.flips_per_hour, "alertSeconds": self.alert_seconds,
            "shuffle": self.shuffle, "showWhileMirroring": self.while_mirroring, "clock24": self.clock24, "targets": self.targets,
        });
        v.as_object().cloned().unwrap_or_default()
    }

    /// Applies the engine settings present in `o`; the others stay as they are.
    fn apply(&mut self, o: &Map<String, Value>) -> Result<(), Refused> {
        let num = |k: &str, lo: u64, hi: u64, cur: u32| -> Result<u32, Refused> {
            match o.get(k) {
                None | Some(Value::Null) => Ok(cur),
                Some(v) => match v.as_u64() {
                    Some(n) if (lo..=hi).contains(&n) => Ok(n as u32),
                    _ => Err(bad(format!("{k} must be from {lo} to {hi}"))),
                },
            }
        };
        let flag = |k: &str, cur: bool| -> Result<bool, Refused> {
            match o.get(k) {
                None | Some(Value::Null) => Ok(cur),
                Some(Value::Bool(b)) => Ok(*b),
                Some(_) => Err(bad(format!("{k} must be true or false"))),
            }
        };
        let mut c = self.clone();
        c.hold_minutes = num("holdMinutes", 0, 480, c.hold_minutes)?;
        c.flips_per_hour = num("flipsPerHour", 1, 360, c.flips_per_hour)?;
        c.alert_seconds = num("alertSeconds", 5, 3600, c.alert_seconds)?;
        c.shuffle = flag("shuffle", c.shuffle)?;
        c.while_mirroring = flag("showWhileMirroring", c.while_mirroring)?;
        c.clock24 = flag("clock24", c.clock24)?;
        match o.get("quietHours") {
            None | Some(Value::Null) => {}
            Some(Value::Object(q)) => {
                let t = |k: &str, cur: &str| -> Result<String, Refused> {
                    match q.get(k) {
                        None | Some(Value::Null) => Ok(cur.to_string()),
                        Some(Value::String(s)) if plugins::minutes(s).is_some() => Ok(s.trim().to_string()),
                        _ => Err(bad(format!("quietHours.{k} must be HH:MM"))),
                    }
                };
                c.quiet.from = t("from", &c.quiet.from)?;
                c.quiet.to = t("to", &c.quiet.to)?;
                c.quiet.enabled = match q.get("enabled") {
                    None | Some(Value::Null) => c.quiet.enabled,
                    Some(Value::Bool(b)) => *b,
                    Some(_) => return Err(bad("quietHours.enabled must be true or false")),
                };
            }
            Some(_) => return Err(bad("quietHours must be {\"enabled\", \"from\", \"to\"}")),
        }
        match o.get("targets") {
            None | Some(Value::Null) => {}
            Some(Value::Array(a)) if a.len() <= 50 => {
                let mut t: Vec<String> = vec![];
                for x in a {
                    let s = x.as_str().filter(|s| !s.is_empty() && s.len() <= 80).ok_or_else(|| bad("targets are board ids, \"this\", \"all\" or \"group:<id>\""))?;
                    if !t.iter().any(|y| y == s) {
                        t.push(s.to_string());
                    }
                }
                c.targets = t;
            }
            Some(_) => return Err(bad("targets must be a list")),
        }
        *self = c;
        Ok(())
    }
}

// ---------------------------------------------------------------- cards

/// What a source shows, before it's laid out for a board.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub instance: String,
    pub name: String,
    #[serde(flatten)]
    pub face: Face,
    /// Re-drawn while it's up (a clock, a countdown's last hours)
    pub live: bool,
    pub reason: String,
    /// Remembered as seen once it has been shown
    #[serde(skip)]
    pub seen: Vec<String>,
    /// Where clicking the board goes: the item's web address (see `plugins::safe_link`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum How {
    /// The rotation's next card
    Ambient,
    /// The same card, re-drawn (a clock's next minute); no pop-up
    Update,
    /// Interrupts; a Return follows
    Alert,
    /// Shown like a posted message: it holds the rotation
    Message,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Show { card: Box<Card>, page: usize, how: How },
    /// The alert is over: put back what was on the board before it.
    Return,
}

#[derive(Clone, Debug)]
struct Showing {
    card: Card,
    how: How,
    page: usize,
    pages: usize,
    since: i64,
    page_at: i64,
    /// Each page's time
    page_ms: i64,
    until: i64,
    /// When the live card was last re-drawn
    drawn_at: i64,
}

/// A fetch over HTTP, with its placeholders still to fill in.
#[derive(Clone, Debug)]
pub struct HttpJob {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    kind: String,
    sign: Option<plugins::Sign>,
    pick: plugins::Pick,
    max: usize,
    name: String,
    settings: Map<String, Value>,
    secret_keys: Vec<String>,
    instance: String,
}

/// What a fetch is for.
#[derive(Clone, Debug)]
pub enum Job {
    Http(Box<HttpJob>),
    Command(plugins::Run),
    Weather(Map<String, Value>),
    Zones(Vec<String>),
}

#[derive(Default, Clone)]
struct Feed {
    data: Option<Data>,
    fetched: Option<i64>,
    next_at: i64,
    error: Option<String>,
    busy: bool,
    fails: u32,
    /// New items still to be shown, oldest first
    queue: VecDeque<Item>,
    /// A burst of new items: how many, and the newest
    digest: Option<(usize, Item)>,
}

/// Feed items already shown (or summed up in a digest), per instance; persisted.
#[derive(Default)]
struct Seen {
    map: HashMap<String, VecDeque<(String, i64)>>,
    path: Option<PathBuf>,
}

impl Seen {
    fn load(path: Option<PathBuf>) -> Self {
        let map = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<HashMap<String, VecDeque<(String, i64)>>>(&s).ok())
            .unwrap_or_default();
        Seen { map, path }
    }
    fn has(&self, inst: &str, key: &str) -> bool {
        self.map.get(inst).is_some_and(|l| l.iter().any(|(k, _)| k == key))
    }
    fn any(&self, inst: &str) -> bool {
        self.map.get(inst).is_some_and(|l| !l.is_empty())
    }
    fn mark(&mut self, inst: &str, keys: &[String], now: i64) {
        if keys.is_empty() {
            return;
        }
        let l = self.map.entry(inst.to_string()).or_default();
        for k in keys {
            if !l.iter().any(|(x, _)| x == k) {
                l.push_back((k.clone(), now));
            }
        }
        while l.len() > SEEN_MAX {
            l.pop_front();
        }
        self.save();
    }
    fn forget(&mut self, inst: &str) {
        if self.map.remove(inst).is_some() {
            self.save();
        }
    }
    fn save(&self) {
        let Some(p) = &self.path else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string(&self.map) {
            let _ = std::fs::write(p, s);
        }
    }
}

/// The engine's state for the tray, the board's hover bar and Settings.
#[derive(Serialize, Clone, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// The rotation is running (something enabled, not paused)
    pub active: bool,
    pub paused: Option<String>,
    pub quiet: bool,
    pub showing: Option<String>,
    pub showing_id: Option<String>,
    pub how: Option<How>,
    pub until: Option<i64>,
    pub hold_until: Option<i64>,
    pub enabled: usize,
}

/// One entry in Up next.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub at: i64,
    pub instance_id: String,
    pub title: String,
    pub reason: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub now: bool,
}

/// When and how big: what `tick` and Up next need to know about the world.
#[derive(Clone)]
pub struct Ctx {
    pub now: i64,
    pub local: NaiveDateTime,
    /// Why the rotation is paused (mirroring a board), if it is
    pub paused: Option<String>,
    /// This board's size
    pub dims: (usize, usize),
}

impl Ctx {
    fn at(&self, t: i64) -> Ctx {
        Ctx { now: t, local: self.local + chrono::Duration::milliseconds(t - self.now), ..self.clone() }
    }
}

struct Inner {
    config: Config,
    plugins: Vec<Manifest>,
    feeds: HashMap<String, Feed>,
    seen: Seen,
    alerts: VecDeque<Card>,
    messages: VecDeque<Card>,
    showing: Option<Showing>,
    /// The ambient card an alert interrupted
    resume: Option<Showing>,
    hold_until: Option<i64>,
    /// A timed notification is up (see `begin_notice`): when it began, and the latest it
    /// can last should `end_notice` never come
    notice: Option<(i64, i64)>,
    flips: VecDeque<i64>,
    /// Smooth weighted round robin: each instance's current weight
    wrr: HashMap<String, i64>,
    rng: u64,
    next_requested: bool,
    status: Status,
    /// False on boards that never run command plugins (Rattleboard Web, like the phones)
    commands: bool,
}

pub struct Paths {
    pub config: Option<PathBuf>,
    pub plugins: Option<PathBuf>,
    pub seen: Option<PathBuf>,
}

pub struct Engine {
    inner: Mutex<Inner>,
    pub wake: Notify,
    paths: Paths,
    pub http: reqwest::Client,
    /// Called after a change the UI should see, with no lock held
    on_change: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Instances' secrets, read from the credential store when first needed
    secret_cache: Mutex<HashMap<String, String>>,
}

const NO_COMMANDS: &str = "Command plugins don't run on this board";

fn secret_account(instance: &str, key: &str) -> String {
    format!("source-{instance}-{key}")
}

/// A feed has items to show once each (RSS, JSON with an items path); anything else is a
/// status, shown as it is now.
fn is_feed(p: &Manifest, data: Option<&Data>) -> bool {
    p.kind == "rss" || (p.kind == "json" && data.is_some_and(|d| !d.single))
}

impl Engine {
    pub fn new(paths: Paths) -> Engine {
        let mut plugins = plugins::builtins();
        let installed: Vec<Value> = paths.plugins.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        plugins.extend(installed.iter().filter_map(|v| plugins::parse(v, false).ok()));
        let mut config = Config::default();
        if let Some(v) = paths.config.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str::<Value>(&s).ok()) {
            if let Some(o) = v.as_object() {
                let _ = config.apply(o);
                config.rotation = o.get("rotation").and_then(|r| r.as_array()).map(|a| a.iter().filter_map(|i| Instance::parse(i, &plugins).ok()).collect()).unwrap_or_default();
                config.approvals = o.get("approvals").and_then(|a| serde_json::from_value(a.clone()).ok()).unwrap_or_default();
            }
        }
        let mut seed = [0u8; 8];
        let _ = getrandom::fill(&mut seed);
        Engine {
            inner: Mutex::new(Inner {
                config,
                plugins,
                feeds: HashMap::new(),
                seen: Seen::load(paths.seen.clone()),
                alerts: VecDeque::new(),
                messages: VecDeque::new(),
                showing: None,
                resume: None,
                hold_until: None,
                notice: None,
                flips: VecDeque::new(),
                wrr: HashMap::new(),
                rng: u64::from_le_bytes(seed) | 1,
                next_requested: false,
                status: Status::default(),
                commands: true,
            }),
            wake: Notify::new(),
            paths,
            http: plugins::client(),
            on_change: Mutex::new(None),
            secret_cache: Mutex::new(HashMap::new()),
        }
    }

    /// For a board that never runs command plugins (Rattleboard Web, like the phones): they
    /// can't be installed or approved, and one already in plugins.json never runs.
    pub fn without_commands(paths: Paths) -> Engine {
        let e = Engine::new(paths);
        e.inner.lock().unwrap().commands = false;
        e
    }

    pub fn on_change(&self, f: Arc<dyn Fn() + Send + Sync>) {
        *self.on_change.lock().unwrap() = Some(f);
    }

    fn changed(&self) {
        let f = self.on_change.lock().unwrap().clone();
        if let Some(f) = f {
            f();
        }
        self.wake.notify_one();
    }

    fn save(&self, i: &Inner) {
        if let Some(p) = &self.paths.config {
            let mut o = i.config.settings_json();
            o.insert("rotation".into(), json!(i.config.rotation.iter().map(|x| x.to_json()).collect::<Vec<_>>()));
            o.insert("approvals".into(), json!(i.config.approvals));
            write(p, &Value::Object(o));
        }
    }

    fn save_plugins(&self, i: &Inner) {
        if let Some(p) = &self.paths.plugins {
            write(p, &json!(i.plugins.iter().filter(|m| !m.builtin).map(|m| m.raw.clone()).collect::<Vec<_>>()));
        }
    }

    // ------------------------------------------------------------ secrets

    pub fn secret(&self, instance: &str, key: &str) -> String {
        let account = secret_account(instance, key);
        if let Some(v) = self.secret_cache.lock().unwrap().get(&account) {
            return v.clone();
        }
        let v = secrets::get(&account);
        self.secret_cache.lock().unwrap().insert(account, v.clone());
        v
    }

    fn secret_set(&self, instance: &str, key: &str) -> bool {
        !self.secret(instance, key).is_empty()
    }

    /// Sets an instance's secrets: only keys its plugin declares; an empty value clears one.
    /// The values go to the credential store and are never sent back.
    pub fn set_secrets(&self, instance: &str, values: &Value) -> Result<(), Refused> {
        let o = values.as_object().ok_or_else(|| bad("Send {\"<key>\": \"<value>\"}"))?;
        let keys = {
            let i = self.inner.lock().unwrap();
            let inst = i.config.rotation.iter().find(|x| x.id == instance).ok_or_else(|| Refused(404, "No source with that id".into()))?;
            i.plugins.iter().find(|p| p.id == inst.plugin).map(|p| p.secrets.iter().map(|s| s.key.clone()).collect::<Vec<_>>()).unwrap_or_default()
        };
        for (k, v) in o {
            if !keys.contains(k) {
                return Err(bad(format!("This source has no secret called {k}")));
            }
            if !v.is_string() || v.as_str().is_some_and(|s| s.len() > 4000) {
                return Err(bad(format!("{k} must be a string")));
            }
        }
        for (k, v) in o {
            let value = v.as_str().unwrap_or("").trim();
            let account = secret_account(instance, k);
            secrets::set(&account, value).map_err(|e| Refused(500, format!("Couldn't save to the credential store: {e}")))?;
            self.secret_cache.lock().unwrap().insert(account, value.to_string());
        }
        {
            // A new address or token: fetch again now.
            let mut i = self.inner.lock().unwrap();
            if let Some(f) = i.feeds.get_mut(instance) {
                f.next_at = 0;
                f.fails = 0;
            }
        }
        self.changed();
        Ok(())
    }

    /// For Settings' Show button on this computer only; never part of the API.
    pub fn reveal_secret(&self, instance: &str, key: &str) -> String {
        self.secret(instance, key)
    }

    fn forget_secrets(&self, inst: &Instance, plugin: Option<&Manifest>) {
        for s in plugin.map(|p| p.secrets.as_slice()).unwrap_or(&[]) {
            let account = secret_account(&inst.id, &s.key);
            let _ = secrets::set(&account, "");
            self.secret_cache.lock().unwrap().remove(&account);
        }
    }

    // ------------------------------------------------------------ the API's view

    fn approved(i: &Inner, p: &Manifest) -> bool {
        p.kind != "command" || (i.commands && i.config.approvals.get(&p.id) == Some(&plugins::fingerprint(&p.raw)))
    }

    fn plugin_json(i: &Inner, p: &Manifest) -> Value {
        let mut v = p.raw.clone();
        if let Some(o) = v.as_object_mut() {
            o.insert("builtin".into(), json!(p.builtin));
            if p.kind == "command" {
                o.insert("approved".into(), json!(Self::approved(i, p)));
                o.insert("fingerprint".into(), json!(plugins::fingerprint(&p.raw)));
            }
        }
        v
    }

    /// GET /rattleboard/sources: every plugin, the rotation (with each secret only as
    /// {"set": true/false}) and the engine's settings.
    pub fn view(&self) -> Value {
        let (plugins, rotation, mut out, status, specs) = {
            let i = self.inner.lock().unwrap();
            let plugins: Vec<Value> = i.plugins.iter().map(|p| Self::plugin_json(&i, p)).collect();
            let specs: HashMap<String, Vec<String>> = i.plugins.iter().map(|p| (p.id.clone(), p.secrets.iter().map(|s| s.key.clone()).collect())).collect();
            let feeds: HashMap<String, (Option<String>, Option<i64>)> = i.feeds.iter().map(|(k, f)| (k.clone(), (f.error.clone(), f.fetched))).collect();
            let rotation: Vec<(Instance, Option<String>, Option<i64>, String)> = i.config.rotation.iter().map(|x| {
                let (e, f) = feeds.get(&x.id).cloned().unwrap_or_default();
                let name = i.plugins.iter().find(|p| p.id == x.plugin).map(|p| label(x, p)).unwrap_or_else(|| x.name.clone());
                (x.clone(), e, f, name)
            }).collect();
            (plugins, rotation, i.config.settings_json(), i.status.clone(), specs)
        };
        // Credential store reads happen with no lock held.
        let rotation: Vec<Value> = rotation
            .into_iter()
            .map(|(x, error, fetched, name)| {
                let mut v = x.to_json();
                v["label"] = json!(name);
                let secrets: Map<String, Value> = specs.get(&x.plugin).into_iter().flatten().map(|k| (k.clone(), json!({ "set": self.secret_set(&x.id, k) }))).collect();
                v["secrets"] = Value::Object(secrets);
                v["error"] = json!(error);
                v["fetched"] = json!(fetched);
                v
            })
            .collect();
        out.insert("plugins".into(), json!(plugins));
        out.insert("rotation".into(), json!(rotation));
        out.insert("status".into(), json!(status));
        Value::Object(out)
    }

    /// Installed (not built-in) plugins: id, the manifest's own "version" if it has one, and
    /// how many sources in the rotation use it. For the community directory's badges.
    pub fn installed(&self) -> Vec<(String, Option<String>, usize)> {
        let i = self.inner.lock().unwrap();
        i.plugins
            .iter()
            .filter(|p| !p.builtin)
            .map(|p| (p.id.clone(), p.raw["version"].as_str().map(String::from), i.config.rotation.iter().filter(|x| x.plugin == p.id).count()))
            .collect()
    }

    pub fn status(&self) -> Status {
        self.inner.lock().unwrap().status.clone()
    }

    pub fn while_mirroring(&self) -> bool {
        self.inner.lock().unwrap().config.while_mirroring
    }

    pub fn config(&self) -> Config {
        self.inner.lock().unwrap().config.clone()
    }

    /// PUT /rattleboard/sources/rotation, and Settings → Sources: the whole rotation (an array,
    /// or {"rotation": [...], "quietHours": ..., ...}), validated. `remote` (the API) may not
    /// add a command source or turn one on: only this computer can.
    pub fn put_rotation(&self, v: &Value, remote: bool) -> Result<(), Refused> {
        let (list, rest) = match v {
            Value::Array(a) => (Some(a.clone()), Map::new()),
            Value::Object(o) => match o.get("rotation") {
                None | Some(Value::Null) => (None, o.clone()),
                Some(Value::Array(a)) => (Some(a.clone()), o.clone()),
                Some(_) => return Err(bad("rotation must be a list of sources")),
            },
            _ => return Err(bad("Send {\"rotation\": [...]} or a list of sources")),
        };
        let removed = {
            let mut i = self.inner.lock().unwrap();
            let mut config = i.config.clone();
            config.apply(&rest)?;
            let mut removed = vec![];
            if let Some(list) = list {
                if list.len() > MAX_INSTANCES {
                    return Err(bad(format!("At most {MAX_INSTANCES} sources")));
                }
                let mut rotation = vec![];
                for x in &list {
                    let inst = Instance::parse(x, &i.plugins)?;
                    if rotation.iter().any(|r: &Instance| r.id == inst.id) {
                        return Err(bad(format!("Two sources have the id {}", inst.id)));
                    }
                    let p = i.plugins.iter().find(|p| p.id == inst.plugin).cloned();
                    if remote && p.as_ref().is_some_and(|p| p.kind == "command") {
                        let old = i.config.rotation.iter().find(|o| o.id == inst.id && o.plugin == inst.plugin);
                        if old.is_none_or(|o| inst.enabled && !o.enabled) {
                            return Err(Refused(403, "Command sources can only be added or turned on at the board itself".into()));
                        }
                    }
                    rotation.push(inst);
                }
                removed = i.config.rotation.iter().filter(|o| !rotation.iter().any(|n| n.id == o.id)).map(|o| (o.clone(), i.plugins.iter().find(|p| p.id == o.plugin).cloned())).collect();
                config.rotation = rotation;
            }
            i.config = config;
            let ids: Vec<String> = i.config.rotation.iter().map(|x| x.id.clone()).collect();
            i.feeds.retain(|k, _| ids.contains(k));
            for (o, _) in &removed {
                i.seen.forget(&o.id);
                i.wrr.remove(&o.id);
            }
            // Settings changed: fetch again, and re-draw what's showing.
            for f in i.feeds.values_mut() {
                f.next_at = 0;
            }
            if i.showing.as_ref().is_some_and(|s| !i.config.rotation.iter().any(|x| x.id == s.card.instance && x.enabled)) {
                i.showing = None;
            }
            self.save(&i);
            removed
        };
        for (o, p) in removed {
            self.forget_secrets(&o, p.as_ref());
        }
        self.changed();
        Ok(())
    }

    /// Adds an instance of a plugin with its defaults (Settings → Sources → Add source).
    pub fn add(&self, plugin: &str, settings: Option<&Value>) -> Result<String, Refused> {
        let (mut rotation, unapproved) = {
            let i = self.inner.lock().unwrap();
            let rotation: Vec<Value> = i.config.rotation.iter().map(|x| x.to_json()).collect();
            (rotation, i.plugins.iter().any(|p| p.id == plugin && p.kind == "command" && !Self::approved(&i, p)))
        };
        let id = new_id();
        let mut v = json!({ "id": id, "plugin": plugin });
        if let Some(s) = settings {
            v["settings"] = s.clone();
        }
        // A command source starts off until it's approved.
        if unapproved {
            v["enabled"] = json!(false);
        }
        rotation.push(v);
        self.put_rotation(&json!({ "rotation": rotation }), false)?;
        Ok(id)
    }

    /// POST /rattleboard/sources/plugins: installs (or updates) a manifest. A command plugin
    /// is refused over the API (403); installed here, it still needs approving before it runs.
    pub fn install(&self, v: &Value, remote: bool) -> Result<String, Refused> {
        let m = plugins::parse(v, false).map_err(bad)?;
        if m.kind == "command" && !self.inner.lock().unwrap().commands {
            return Err(Refused(403, NO_COMMANDS.into()));
        }
        if m.kind == "command" && remote {
            return Err(Refused(403, "Command plugins can only be installed at the board itself".into()));
        }
        {
            let mut i = self.inner.lock().unwrap();
            let existing = i.plugins.iter().position(|p| p.id == m.id);
            if existing.is_none() && i.plugins.iter().filter(|p| !p.builtin).count() >= MAX_PLUGINS {
                return Err(bad(format!("At most {MAX_PLUGINS} plugins")));
            }
            let id = m.id.clone();
            match existing {
                Some(k) => i.plugins[k] = m,
                None => i.plugins.push(m),
            }
            // Its sources take on new settings fields with their defaults.
            let plugins = i.plugins.clone();
            let inner = &mut *i;
            for x in inner.config.rotation.iter_mut().filter(|x| x.plugin == id) {
                if let Ok(n) = Instance::parse(&x.to_json(), &plugins) {
                    *x = n;
                }
                if let Some(f) = inner.feeds.get_mut(&x.id) {
                    f.next_at = 0;
                }
            }
            self.save_plugins(&i);
            self.save(&i);
        }
        self.changed();
        Ok(v["id"].as_str().unwrap_or_default().to_string())
    }

    /// DELETE /rattleboard/sources/plugins/{id}: removes it and its sources.
    pub fn uninstall(&self, id: &str) -> Result<(), Refused> {
        let removed = {
            let mut i = self.inner.lock().unwrap();
            let k = i.plugins.iter().position(|p| p.id == id).ok_or_else(|| Refused(404, "No plugin with that id".into()))?;
            if i.plugins[k].builtin {
                return Err(bad("Built-in plugins can't be removed"));
            }
            let p = i.plugins.remove(k);
            let removed: Vec<Instance> = i.config.rotation.iter().filter(|x| x.plugin == id).cloned().collect();
            i.config.rotation.retain(|x| x.plugin != id);
            i.config.approvals.remove(id);
            for x in &removed {
                i.feeds.remove(&x.id);
                i.seen.forget(&x.id);
            }
            if i.showing.as_ref().is_some_and(|s| removed.iter().any(|x| x.id == s.card.instance)) {
                i.showing = None;
            }
            self.save_plugins(&i);
            self.save(&i);
            (removed, p)
        };
        for x in &removed.0 {
            self.forget_secrets(x, Some(&removed.1));
        }
        self.changed();
        Ok(())
    }

    /// Approves a command plugin as it is now. `fingerprint` must be the one the user was
    /// shown, so an approval never covers a manifest that changed in the meantime.
    pub fn approve(&self, id: &str, fingerprint: &str, on: bool) -> Result<(), Refused> {
        {
            let mut i = self.inner.lock().unwrap();
            let p = i.plugins.iter().find(|p| p.id == id).ok_or_else(|| Refused(404, "No plugin with that id".into()))?;
            if p.kind != "command" {
                return Err(bad("Only command plugins need approving"));
            }
            if !i.commands {
                return Err(Refused(403, NO_COMMANDS.into()));
            }
            let fp = plugins::fingerprint(&p.raw);
            if on {
                if fp != fingerprint {
                    return Err(Refused(409, "The plugin changed since it was shown; look at it again".into()));
                }
                i.config.approvals.insert(id.to_string(), fp);
            } else {
                i.config.approvals.remove(id);
            }
            for x in i.config.rotation.iter().filter(|x| x.plugin == id).map(|x| x.id.clone()).collect::<Vec<_>>() {
                if let Some(f) = i.feeds.get_mut(&x) {
                    f.next_at = 0;
                }
            }
            self.save(&i);
        }
        self.changed();
        Ok(())
    }

    /// Export: a plugin's manifest, or a source (its settings, never its secrets) along with
    /// its plugin's manifest unless that's a built-in.
    pub fn export(&self, id: &str) -> Option<(String, Value)> {
        let i = self.inner.lock().unwrap();
        if let Some(p) = i.plugins.iter().find(|p| p.id == id && !p.builtin) {
            return Some((p.name.clone(), p.raw.clone()));
        }
        let x = i.config.rotation.iter().find(|x| x.id == id)?;
        let p = i.plugins.iter().find(|p| p.id == x.plugin)?;
        let mut source = x.to_json();
        if let Some(o) = source.as_object_mut() {
            o.remove("id");
        }
        let mut v = json!({ "format": 1, "kind": "rattleboard-source", "source": source });
        if !p.builtin {
            v["plugin"] = p.raw.clone();
        }
        Some((x.name.clone(), v))
    }

    pub fn request_next(&self) {
        self.inner.lock().unwrap().next_requested = true;
        self.wake.notify_one();
    }

    /// Something else went on the board (Compose, the API, a sequence, the mirrored board):
    /// the rotation holds for `holdMinutes`, and an alert showing is over (the message
    /// replaced it, so there's nothing to go back to).
    pub fn note_message(&self, now: i64) {
        let mut i = self.inner.lock().unwrap();
        let hold = i.config.hold_minutes as i64 * 60_000;
        if i.showing.as_ref().is_some_and(|s| s.how == How::Alert) {
            i.showing = None;
            i.resume = None;
        }
        // A notification showing is over too: the message replaced it.
        i.notice = None;
        if hold > 0 {
            i.hold_until = Some(now + hold);
            i.showing = None;
        }
    }

    /// A timed notification went up for about `ms`: like an alert it interrupts, so the
    /// rotation waits (no card replaces it) without being held afterwards. The board puts
    /// back what was there and then calls `end_notice`; if it never does, the rotation goes
    /// on by itself a minute after the notification should have ended.
    pub fn begin_notice(&self, now: i64, ms: i64) {
        let mut i = self.inner.lock().unwrap();
        let start = i.notice.map(|n| n.0).unwrap_or(now);
        i.notice = Some((start, now + ms.max(0) + 60_000));
    }

    /// The notification is over and what it covered is back: the rotation carries on where
    /// it was, the card it interrupted getting the time it lost.
    pub fn end_notice(&self, now: i64) {
        let mut guard = self.inner.lock().unwrap();
        let i = &mut *guard;
        let Some((start, _)) = i.notice.take() else { return };
        let lost = (now - start).max(0);
        for s in [i.showing.as_mut(), i.resume.as_mut()].into_iter().flatten() {
            s.since += lost;
            s.page_at += lost;
            s.until += lost;
        }
        drop(guard);
        self.wake.notify_one();
    }

    // ------------------------------------------------------------ fetching

    /// Fetches due now. Each is marked busy until `store` hears back.
    pub fn due_jobs(&self, now: i64) -> Vec<(String, Job)> {
        let mut i = self.inner.lock().unwrap();
        let mut jobs = vec![];
        let list: Vec<(Instance, Manifest)> = i.config.rotation.iter().filter(|x| x.enabled).filter_map(|x| i.plugins.iter().find(|p| p.id == x.plugin).map(|p| (x.clone(), p.clone()))).collect();
        for (x, p) in list {
            if !Self::approved(&i, &p) {
                continue;
            }
            let job = job_for(&x, &p);
            let Some(job) = job else { continue };
            let f = i.feeds.entry(x.id.clone()).or_default();
            if f.busy || f.next_at > now {
                continue;
            }
            f.busy = true;
            jobs.push((x.id.clone(), job));
        }
        jobs
    }

    /// Runs a fetch, with no lock held: placeholders (and secrets) are filled in here.
    pub async fn run_job(&self, job: Job, local: NaiveDateTime, now: i64) -> Result<Data, String> {
        match job {
            Job::Http(job) => {
                let HttpJob { method, url, headers, kind, sign, pick, max, name, settings, secret_keys, instance } = *job;
                // Filled in before the request, so nothing borrowed is held across it.
                let (url, headers, pick, key) = {
                    let secrets: HashMap<String, String> = secret_keys.iter().map(|k| (k.clone(), self.secret(&instance, k))).collect();
                    let lookup = |k: &str| secrets.get(k).cloned().unwrap_or_default();
                    let ctx = plugins::Ctx { name: &name, item: None, settings: &settings, secret: Some(&lookup), now: local, now_ms: now, clock24: false, list: &[], row: None };
                    let url = plugins::render(&url, &ctx, plugins::Mode::Url);
                    let headers: Vec<(String, String)> = headers.iter().map(|(k, v)| (k.clone(), plugins::render(v, &ctx, plugins::Mode::Header))).collect();
                    let key = sign.as_ref().map(|s| plugins::render(&s.key, &ctx, plugins::Mode::Header));
                    (url, headers, pick.resolve(&ctx), key)
                };
                if url.trim().is_empty() {
                    return Err("Set the address in this source's settings".into());
                }
                let url = match (&sign, key) {
                    (Some(s), Some(k)) => plugins::sign_url(url.trim(), s, k.trim())?,
                    _ => url,
                };
                let body = plugins::fetch(&self.http, &method, url.trim(), &headers).await?;
                match kind.as_str() {
                    "rss" => plugins::parse_feed(&body, max),
                    "ical" => plugins::parse_ical(&body, now),
                    _ => plugins::parse_json(&body, &pick, max),
                }
            }
            Job::Command(run) => plugins::run_command(&run).await.map(|out| plugins::parse_output(&out)),
            Job::Weather(settings) => builtins::weather(&self.http, &settings).await,
            Job::Zones(cities) => builtins::zones(&self.http, &cities).await,
        }
    }

    /// A fetch's result: new feed items are queued (or summed up in a digest), alerts noted.
    pub fn store(&self, instance: &str, res: Result<Data, String>, now: i64) {
        {
            let mut guard = self.inner.lock().unwrap();
            let i = &mut *guard;
            let Some(x) = i.config.rotation.iter().find(|x| x.id == instance).cloned() else { return };
            let Some(p) = i.plugins.iter().find(|p| p.id == x.plugin).cloned() else { return };
            let every = (p.every as i64) * 1000;
            let mut alerts = vec![];
            {
                let f = i.feeds.entry(instance.to_string()).or_default();
                f.busy = false;
                match res {
                    Err(e) => {
                        f.error = Some(e);
                        f.fails += 1;
                        // Retry sooner than usual, backing off: 1, 2, 4... minutes, up to `every`.
                        let back = (60_000i64 << (f.fails - 1).min(6)).min(every.max(60_000));
                        f.next_at = now + back;
                    }
                    Ok(mut data) => {
                        f.error = None;
                        f.fails = 0;
                        f.fetched = Some(now);
                        f.next_at = now + every;
                        if is_feed(&p, Some(&data)) {
                            data.items.retain(|it| keep(&x, it));
                        }
                        f.data = Some(data);
                    }
                }
            }
            let data = i.feeds.get(instance).and_then(|f| f.data.clone());
            let feed = is_feed(&p, data.as_ref());
            if let (Some(data), true) = (data, feed) {
                let first = !i.seen.any(instance);
                let queued: Vec<String> = i.feeds.get(instance).map(|f| f.queue.iter().map(|q| q.key.clone()).collect()).unwrap_or_default();
                let new: Vec<Item> = data.items.iter().filter(|it| !i.seen.has(instance, &it.key) && !queued.contains(&it.key)).cloned().collect();
                if first && !new.is_empty() {
                    // A new source shows its newest item and remembers the rest as seen.
                    let rest: Vec<String> = new.iter().skip(1).map(|it| it.key.clone()).collect();
                    i.seen.mark(instance, &rest, now);
                    if let Some(f) = i.feeds.get_mut(instance) {
                        f.queue.push_back(new[0].clone());
                    }
                } else {
                    let (hot, calm): (Vec<Item>, Vec<Item>) = new.into_iter().partition(|it| p.alert || it.alert || mentions(it, p.alert_words.iter().chain(&x.alert_words)));
                    for it in hot.iter().take(3) {
                        alerts.push(item_card(&x, &p, it, now, i.config.clock24, "alert · keyword"));
                    }
                    let skipped: Vec<String> = hot.iter().skip(3).map(|it| it.key.clone()).collect();
                    i.seen.mark(instance, &skipped, now);
                    let f = i.feeds.get_mut(instance).unwrap();
                    if calm.len() + f.queue.len() > DIGEST_AT {
                        let mut all: Vec<Item> = f.queue.drain(..).collect();
                        all.extend(calm);
                        all.sort_by_key(|it| std::cmp::Reverse(it.time));
                        let keys: Vec<String> = all.iter().map(|it| it.key.clone()).collect();
                        let (n, newest) = (all.len() + f.digest.as_ref().map(|d| d.0).unwrap_or(0), all[0].clone());
                        f.digest = Some((n, newest));
                        i.seen.mark(instance, &keys, now);
                    } else {
                        // Oldest first, so they show in the order they came.
                        let mut calm = calm;
                        calm.sort_by_key(|it| it.time);
                        f.queue.extend(calm);
                        while f.queue.len() > p.max_items {
                            if let Some(old) = f.queue.pop_front() {
                                i.seen.mark(instance, &[old.key], now);
                            }
                        }
                    }
                }
            } else if let Some(it) = i.feeds.get(instance).and_then(|f| f.data.as_ref()).and_then(|d| d.items.first()).filter(|it| it.alert).cloned() {
                // A command asking to interrupt, once per id.
                if !i.seen.has(instance, &it.key) {
                    let mut c = item_card(&x, &p, &it, now, i.config.clock24, "alert");
                    c.seen = vec![it.key.clone()];
                    alerts.push(c);
                }
            }
            for c in alerts {
                if !i.alerts.iter().any(|a| a.seen == c.seen) {
                    i.alerts.push_back(c);
                }
            }
        }
        self.changed();
    }

    // ------------------------------------------------------------ preview

    /// What a source would show now, for Settings and GET /rattleboard/sources/preview: it's
    /// fetched first if it hasn't been. Over the API (`remote`) a command source is refused.
    /// Whether a source fetches anything (a feed, an API, a command), and when it last did.
    pub fn fetch_state(&self, instance: &str) -> (bool, Option<i64>) {
        let i = self.inner.lock().unwrap();
        let fetches = i.config.rotation.iter().find(|x| x.id == instance)
            .and_then(|x| i.plugins.iter().find(|p| p.id == x.plugin).map(|p| job_for(x, p).is_some()))
            .unwrap_or(false);
        (fetches, i.feeds.get(instance).and_then(|f| f.fetched))
    }

    /// What a source would show now. Its data is fetched first if it has none yet, or, with
    /// `refresh` (Settings' Refresh preview), again now unless it was fetched in the last
    /// `REFRESH_FLOOR_MS`.
    pub async fn preview(&self, instance: &str, ctx: &Ctx, remote: bool, refresh: bool) -> Result<(Vec<Grid>, String), Refused> {
        let (x, p, job, fetch) = {
            let i = self.inner.lock().unwrap();
            let x = i.config.rotation.iter().find(|x| x.id == instance).cloned().ok_or_else(|| Refused(404, "No source with that id".into()))?;
            let p = i.plugins.iter().find(|p| p.id == x.plugin).cloned().ok_or_else(|| Refused(404, "Its plugin was removed".into()))?;
            if remote && p.kind == "command" {
                return Err(Refused(403, "Command sources can only be previewed at the board itself".into()));
            }
            if !Self::approved(&i, &p) {
                return Ok((vec![message_grid("Approve this plugin to run it", ctx.dims)], "Not approved yet".into()));
            }
            let feed = i.feeds.get(instance);
            let has = feed.is_some_and(|f| f.data.is_some());
            let recent = feed.and_then(|f| f.fetched).is_some_and(|t| (ctx.now - t).abs() < REFRESH_FLOOR_MS);
            let job = job_for(&x, &p);
            (x, p, job, !has || (refresh && !recent))
        };
        if let (Some(job), true) = (job, fetch) {
            let res = self.run_job(job, ctx.local, ctx.now).await;
            if let Err(e) = &res {
                return Ok((vec![message_grid(e, ctx.dims)], e.clone()));
            }
            self.store(instance, res, ctx.now);
        }
        let i = self.inner.lock().unwrap();
        let card = card_for(&i, &x, &p, ctx, 0, true);
        let reason = if !x.enabled {
            "Off".to_string()
        } else if i.status.showing_id.as_deref() == Some(instance) {
            "On the board now".to_string()
        } else if x.schedule.as_ref().or(p.schedule.as_ref()).is_some_and(|s| !s.active(ctx.local)) {
            "Outside its schedule now".to_string()
        } else if let Some(e) = i.feeds.get(instance).and_then(|f| f.error.clone()) {
            e
        } else {
            card.as_ref().map(|c| c.reason.clone()).unwrap_or_default()
        };
        match card {
            Some(c) => Ok((c.face.render(ctx.dims.0, ctx.dims.1), reason)),
            None => Ok((vec![message_grid("Nothing to show yet", ctx.dims)], if reason.is_empty() { "Nothing to show yet".into() } else { reason })),
        }
    }

    // ------------------------------------------------------------ the rotation

    /// Moves the board along: called once a second (and when woken). Returns what to show.
    pub fn tick(&self, ctx: &Ctx) -> Vec<Action> {
        let out = {
            let mut guard = self.inner.lock().unwrap();
            let out = tick(&mut guard, ctx);
            let i = &mut *guard;
            i.status = Status {
                active: ctx.paused.is_none() && i.config.rotation.iter().any(|x| x.enabled),
                paused: ctx.paused.clone(),
                quiet: i.config.quiet.active(ctx.local),
                showing: i.showing.as_ref().map(|s| s.card.name.clone()),
                showing_id: i.showing.as_ref().map(|s| s.card.instance.clone()),
                how: i.showing.as_ref().map(|s| s.how),
                until: i.showing.as_ref().map(|s| s.until),
                hold_until: i.hold_until,
                enabled: i.config.rotation.iter().filter(|x| x.enabled).count(),
            };
            out
        };
        if !out.is_empty() {
            self.changed();
        }
        out
    }

    /// The next hour: what comes on when, and why.
    pub fn up_next(&self, ctx: &Ctx) -> Vec<Entry> {
        let i = self.inner.lock().unwrap();
        plan(&i, ctx)
    }

    #[cfg(test)]
    fn showing(&self) -> Option<(String, How)> {
        self.inner.lock().unwrap().showing.as_ref().map(|s| (s.card.instance.clone(), s.how))
    }
}

fn write(p: &PathBuf, v: &Value) {
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string_pretty(v) {
        let _ = std::fs::write(p, s);
    }
}

fn message_grid(text: &str, (rows, cols): (usize, usize)) -> Grid {
    crate::vestaboard::text_layout(&plugins::clean(text), rows, cols)
}

fn mentions<'a>(it: &Item, mut words: impl Iterator<Item = &'a String>) -> bool {
    let hay = format!("{} {}", it.title, it.body).to_lowercase();
    words.any(|w| hay.contains(w.as_str()))
}

/// An instance's include and exclude filters.
fn keep(x: &Instance, it: &Item) -> bool {
    (x.include.is_empty() || mentions(it, x.include.iter())) && !mentions(it, x.exclude.iter())
}

fn job_for(x: &Instance, p: &Manifest) -> Option<Job> {
    match (p.kind.as_str(), p.id.as_str()) {
        ("command", _) => p.run.clone().map(Job::Command),
        ("native", builtins::WEATHER) => Some(Job::Weather(x.settings.clone())),
        ("native", builtins::CLOCK) if x.settings.get("style").and_then(|s| s.as_str()) == Some("world") => Some(Job::Zones(builtins::cities(&x.settings))),
        ("rss" | "json" | "ical", _) => {
            let f = p.fetch.as_ref()?;
            Some(Job::Http(Box::new(HttpJob {
                method: f.method.clone(),
                url: f.url.clone(),
                headers: f.headers.clone(),
                kind: p.kind.clone(),
                sign: f.sign.clone(),
                pick: p.pick.clone(),
                max: p.max_items,
                name: x.name.clone(),
                settings: x.settings.clone(),
                secret_keys: p.secrets.iter().map(|s| s.key.clone()).collect(),
                instance: x.id.clone(),
            })))
        }
        _ => None,
    }
}

/// The name a source goes by: its own, or for a built-in left with the plugin's name, one
/// from its settings ("Sam's birthday").
fn label(x: &Instance, p: &Manifest) -> String {
    if x.name != p.name {
        return x.name.clone();
    }
    builtins::label(&p.id, &x.settings).unwrap_or_else(|| x.name.clone())
}

fn dwell_of(x: &Instance, p: &Manifest) -> u64 {
    if x.dwell > 0 { x.dwell } else { p.dwell.unwrap_or(DEFAULT_DWELL) }.max(MIN_DWELL)
}

fn chip_of(x: &Instance, p: &Manifest) -> Option<u8> {
    x.chip.or(p.card.chip)
}

fn short_time(ms: i64, clock24: bool) -> String {
    plugins::format_time(plugins::local_of(ms), if clock24 { "HH:mm" } else { "h:mm" })
}

/// A feed item (or a status) as a card: the plugin's template if it has one, otherwise the
/// standard card with the source's name and the item's time across the top.
fn item_card(x: &Instance, p: &Manifest, it: &Item, now: i64, clock24: bool, reason: &str) -> Card {
    item_card_in(x, p, it, &[], plugins::local_of(now), now, clock24, reason)
}

/// `list` is a status list's items, for {{rows}} and {{count}}.
#[allow(clippy::too_many_arguments)]
fn item_card_in(x: &Instance, p: &Manifest, it: &Item, list: &[Item], local: NaiveDateTime, now: i64, clock24: bool, reason: &str) -> Card {
    let face = template_face(x, p, Some(it), list, local, now, clock24).unwrap_or_else(|| {
        if let Some(g) = &it.characters {
            return Face::Grid { grid: g.clone() };
        }
        let body = if it.title.is_empty() { it.body.clone() } else { it.title.clone() };
        let chip = x.chip.or(it.chip).or(p.card.chip).unwrap_or(0);
        if p.kind == "command" {
            return Face::Text { text: body, left: p.card.left, chip: chip_of(x, p), pages: p.card.pages };
        }
        Face::Standard { chip, name: x.name.clone(), time: short_time(it.time.unwrap_or(now), clock24), body, pages: p.card.pages }
    });
    Card { instance: x.id.clone(), name: label(x, p), face, live: false, reason: reason.into(), seen: vec![it.key.clone()], link: it.link.clone() }
}

fn template_face(x: &Instance, p: &Manifest, it: Option<&Item>, list: &[Item], now: NaiveDateTime, now_ms: i64, clock24: bool) -> Option<Face> {
    let t = p.card.template.as_ref()?;
    template_text(x, p, t, it, list, now, now_ms, clock24)
}

#[allow(clippy::too_many_arguments)]
fn template_text(x: &Instance, p: &Manifest, t: &str, it: Option<&Item>, list: &[Item], now: NaiveDateTime, now_ms: i64, clock24: bool) -> Option<Face> {
    let ctx = plugins::Ctx { name: &x.name, item: it, settings: &x.settings, secret: None, now, now_ms, clock24, list, row: p.card.row.as_deref() };
    let mut text = plugins::render(t, &ctx, plugins::Mode::Card);
    // A template from settings (the JSON URL built-in) has placeholders of its own.
    if text.contains("{{") {
        text = plugins::render(&text, &ctx, plugins::Mode::Card);
    }
    (!text.trim().is_empty()).then(|| Face::Text { text, left: p.card.left, chip: chip_of(x, p), pages: p.card.pages })
}

/// "Today 9:30 AM", "Tomorrow", "Tue 21 Oct 2:00 PM" for a calendar event.
fn when(it: &Item, now: i64, clock24: bool) -> String {
    let (start, end) = (it.time.unwrap_or(now), it.end.unwrap_or(now));
    if start <= now && now < end {
        return if it.all_day { "Today".into() } else { format!("Now, until {}", short_time(end, clock24)) };
    }
    let s = plugins::local_of(start);
    let today = plugins::local_of(now).date();
    let day = match (s.date() - today).num_days() {
        0 => "Today".to_string(),
        1 => "Tomorrow".to_string(),
        _ => plugins::format_time(s, "ddd d MMM"),
    };
    if it.all_day {
        day
    } else {
        format!("{day} {}", plugins::format_time(s, if clock24 { "HH:mm" } else { "h:mm tt" }))
    }
}

/// The card an instance would show at `ctx`'s time, or None when it has nothing to show.
/// `skip` passes over that many queued feed items (Up next looking ahead). `any` (a preview)
/// shows a feed's newest item even when it has been shown already.
fn card_for(i: &Inner, x: &Instance, p: &Manifest, ctx: &Ctx, skip: usize, any: bool) -> Option<Card> {
    let clock24 = i.config.clock24;
    let chip = chip_of(x, p);
    let feed = i.feeds.get(&x.id);
    let data = feed.and_then(|f| f.data.as_ref());
    let card = |face: Face, live: bool, reason: &str| Card { instance: x.id.clone(), name: label(x, p), face, live, reason: reason.into(), seen: vec![], link: None };
    match (p.kind.as_str(), p.id.as_str()) {
        ("native", builtins::CLOCK) => {
            let world = x.settings.get("style").and_then(|s| s.as_str()) == Some("world");
            if world && data.is_none() {
                return None;
            }
            Some(card(builtins::clock(&x.settings, ctx.local, ctx.now, clock24, chip, data), true, "ambient"))
        }
        ("native", builtins::COUNTDOWN) => builtins::countdown(&x.settings, ctx.local, chip).map(|c| card(c.face, c.live, "ambient")),
        ("native", builtins::MOTD) => builtins::motd(&x.settings, ctx.local, chip).map(|f| card(f, false, "ambient")),
        ("native", builtins::QUOTE) => {
            let daily = x.settings.get("mode").and_then(|m| m.as_str()) == Some("daily");
            if daily && !any {
                return None;
            }
            builtins::quote(&x.settings, ctx.local, chip).map(|f| card(f, false, if daily { "message · once a day" } else { "ambient" }))
        }
        ("native", builtins::WEATHER) => {
            let it = data?.items.first()?;
            let name = if x.name == p.name { it.title.clone() } else { x.name.clone() };
            let face = builtins::weather_face(it, name, short_time(ctx.now, clock24), builtins::colour(&x.settings), x.chip.or(it.chip).or(p.card.chip));
            Some(card(face, false, "ambient"))
        }
        ("text", _) => {
            let face = template_face(x, p, None, &[], ctx.local, ctx.now, clock24).unwrap_or_else(|| Face::Text { text: x.name.clone(), left: false, chip, pages: 1 });
            Some(card(face, true, "ambient"))
        }
        ("ical", _) => {
            let d = data?;
            let next = d.items.iter().find(|it| it.end.unwrap_or(0) > ctx.now)?;
            let face = template_face(x, p, Some(next), &[], ctx.local, ctx.now, clock24).unwrap_or_else(|| Face::Standard {
                chip: chip.unwrap_or(0),
                name: x.name.clone(),
                time: short_time(ctx.now, clock24),
                body: [next.title.clone(), when(next, ctx.now, clock24), next.body.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n"),
                pages: 1,
            });
            Some(Card { link: next.link.clone(), ..card(face, false, "ambient · next event") })
        }
        _ if is_feed(p, data) => {
            let f = feed?;
            if let (Some((n, newest)), 0) = (&f.digest, skip) {
                let mut c = item_card(x, p, newest, ctx.now, clock24, &format!("ambient · digest of {n}"));
                if let Face::Standard { name, .. } = &mut c.face {
                    *name = format!("{n} new - {}", x.name);
                }
                if let Face::Text { text, .. } = &mut c.face {
                    *text = format!("{n} new - {}\n{text}", x.name);
                }
                c.seen = vec![format!("digest-{}", newest.key)];
                return Some(c);
            }
            let skip = skip.saturating_sub(f.digest.is_some() as usize);
            match f.queue.get(skip) {
                Some(it) => Some(item_card(x, p, it, ctx.now, clock24, if f.queue.len() > 1 { "ambient · new" } else { "ambient · new item" })),
                None if any => data.and_then(|d| d.items.first()).map(|it| {
                    let mut c = item_card(x, p, it, ctx.now, clock24, "Nothing new; the latest item");
                    c.seen.clear();
                    c
                }),
                None => None,
            }
        }
        // A status list (pick.as "status"): the first item, with the list for {{rows}};
        // card.empty when nothing's left. Re-drawn while it's up, as its data and times change.
        _ if p.pick.status => {
            let d = data?;
            let mut c = match d.items.first() {
                Some(it) => item_card_in(x, p, it, &d.items, ctx.local, ctx.now, clock24, "ambient · status"),
                None => {
                    let face = template_text(x, p, p.card.empty.as_deref()?, None, &[], ctx.local, ctx.now, clock24)?;
                    card(face, true, "ambient · nothing listed")
                }
            };
            c.seen.clear();
            c.live = true;
            Some(c)
        }
        // A status: one JSON value, a command's output
        _ => {
            let it = data?.items.first()?;
            let mut c = item_card(x, p, it, ctx.now, clock24, "ambient");
            c.seen.clear();
            Some(c)
        }
    }
}

/// The instances that have something to show at `ctx`'s time, in rotation order, with
/// their cards. `taken` counts feed items Up next has already used up.
fn candidates(i: &Inner, ctx: &Ctx, taken: &HashMap<String, usize>) -> Vec<(Instance, Manifest, Card)> {
    i.config
        .rotation
        .iter()
        .filter(|x| x.enabled)
        .filter_map(|x| {
            let p = i.plugins.iter().find(|p| p.id == x.plugin)?;
            if !Engine::approved(i, p) || x.schedule.as_ref().or(p.schedule.as_ref()).is_some_and(|s| !s.active(ctx.local)) {
                return None;
            }
            let c = card_for(i, x, p, ctx, taken.get(&x.id).copied().unwrap_or(0), false)?;
            Some((x.clone(), p.clone(), c))
        })
        .collect()
}

/// Smooth weighted round robin (as nginx balances servers): over time each instance comes
/// round in proportion to its weight, spread out rather than in runs; equal weights take
/// turns in rotation order.
fn pick_wrr(wrr: &mut HashMap<String, i64>, list: &[(String, u32)], last: Option<&str>) -> usize {
    let total: i64 = list.iter().map(|(_, w)| *w as i64).sum();
    for (id, w) in list {
        *wrr.entry(id.clone()).or_insert(0) += *w as i64;
    }
    // Ties go to the one not just shown, then to the first in rotation order.
    let rank = |k: usize| (wrr[&list[k].0], Some(list[k].0.as_str()) != last, std::cmp::Reverse(k));
    let best = (0..list.len()).max_by_key(|&k| rank(k)).unwrap_or(0);
    *wrr.get_mut(&list[best].0).unwrap() -= total;
    best
}

/// Shuffle: a weighted random pick, never the same instance twice running when there's a choice.
fn pick_shuffle(rng: &mut u64, list: &[(String, u32)], last: Option<&str>) -> usize {
    let pool: Vec<usize> = (0..list.len()).filter(|&k| list.len() == 1 || Some(list[k].0.as_str()) != last).collect();
    let total: u64 = pool.iter().map(|&k| list[k].1 as u64).sum();
    // xorshift64
    *rng ^= *rng << 13;
    *rng ^= *rng >> 7;
    *rng ^= *rng << 17;
    let mut r = *rng % total.max(1);
    for &k in &pool {
        let w = list[k].1 as u64;
        if r < w {
            return k;
        }
        r -= w;
    }
    pool[0]
}

fn choose(i: &mut Inner, ctx: &Ctx, next: bool) -> Option<(Instance, Manifest, Card)> {
    let mut list = candidates(i, ctx, &HashMap::new());
    // "Next" moves on to another source when there is one.
    let current = i.showing.as_ref().map(|s| s.card.instance.clone());
    if next && list.len() > 1 {
        list.retain(|(x, _, _)| Some(&x.id) != current.as_ref());
    }
    if list.is_empty() {
        return None;
    }
    let weights: Vec<(String, u32)> = list.iter().map(|(x, _, _)| (x.id.clone(), x.weight)).collect();
    let last = i.showing.as_ref().map(|s| s.card.instance.clone());
    let k = if i.config.shuffle { pick_shuffle(&mut i.rng, &weights, last.as_deref()) } else { pick_wrr(&mut i.wrr, &weights, last.as_deref()) };
    Some(list.swap_remove(k))
}

fn flips_in_hour(flips: &VecDeque<i64>, now: i64) -> usize {
    flips.iter().filter(|&&t| t > now - HOUR_MS).count()
}

fn show(i: &mut Inner, card: Card, how: How, ctx: &Ctx, page_ms: i64, total_ms: Option<i64>) -> Action {
    let pages = card.face.render(ctx.dims.0, ctx.dims.1).len().max(1);
    i.seen.mark(&card.instance.clone(), &card.seen, ctx.now);
    for k in &card.seen {
        if let Some(f) = i.feeds.get_mut(&card.instance) {
            f.queue.retain(|it| &it.key != k);
            if f.digest.as_ref().is_some_and(|d| format!("digest-{}", d.1.key) == *k) {
                f.digest = None;
            }
        }
    }
    i.flips.push_back(ctx.now);
    while i.flips.front().is_some_and(|&t| t <= ctx.now - HOUR_MS) {
        i.flips.pop_front();
    }
    let total = total_ms.unwrap_or(page_ms * pages as i64);
    let page_ms = total / pages as i64;
    i.showing = Some(Showing { card: card.clone(), how, page: 0, pages, since: ctx.now, page_at: ctx.now, page_ms, until: ctx.now + total, drawn_at: ctx.now });
    Action::Show { card: Box::new(card), page: 0, how }
}

/// Time-based alerts and once-a-day messages that are due: a calendar event 10 minutes out,
/// a countdown's day, the day's quote.
fn collect_due(i: &mut Inner, ctx: &Ctx) {
    let clock24 = i.config.clock24;
    let mut alerts = vec![];
    let mut messages = vec![];
    for x in i.config.rotation.iter().filter(|x| x.enabled) {
        let Some(p) = i.plugins.iter().find(|p| p.id == x.plugin) else { continue };
        let already = |key: &str| i.seen.has(&x.id, key);
        match (p.kind.as_str(), p.id.as_str()) {
            ("ical", _) => {
                for it in i.feeds.get(&x.id).and_then(|f| f.data.as_ref()).map(|d| d.items.as_slice()).unwrap_or(&[]) {
                    let start = it.time.unwrap_or(0);
                    let key = format!("alert-{}", it.key);
                    if !it.all_day && start > ctx.now && start - ctx.now <= EVENT_LEAD_MS && !already(&key) {
                        let mins = (start - ctx.now + 59_999) / 60_000;
                        let body = [it.title.clone(), it.body.clone()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
                        let face = Face::Standard { chip: 63, name: format!("In {mins} min"), time: short_time(start, clock24), body, pages: 1 };
                        alerts.push(Card { instance: x.id.clone(), name: x.name.clone(), face, live: false, reason: "alert · starts in 10 min".into(), seen: vec![key], link: it.link.clone() });
                    }
                }
            }
            ("native", builtins::COUNTDOWN) => {
                if let Some((key, face)) = builtins::countdown(&x.settings, ctx.local, chip_of(x, p)).and_then(|c| c.celebrating) {
                    if !already(&key) {
                        alerts.push(Card { instance: x.id.clone(), name: x.name.clone(), face, live: false, reason: "alert · celebration".into(), seen: vec![key], link: None });
                    }
                }
            }
            ("native", builtins::QUOTE) => {
                if let (Some(key), Some(face)) = (builtins::quote_due(&x.settings, ctx.local), builtins::quote(&x.settings, ctx.local, chip_of(x, p))) {
                    if !already(&key) {
                        messages.push(Card { instance: x.id.clone(), name: x.name.clone(), face, live: false, reason: "message · quote of the day".into(), seen: vec![key], link: None });
                    }
                }
            }
            _ => {}
        }
    }
    for c in alerts {
        if !i.alerts.iter().any(|a| a.seen == c.seen) && !i.showing.as_ref().is_some_and(|s| s.card.seen == c.seen) {
            i.alerts.push_back(c);
        }
    }
    for c in messages {
        if !i.messages.iter().any(|a| a.seen == c.seen) && !i.showing.as_ref().is_some_and(|s| s.card.seen == c.seen) {
            i.messages.push_back(c);
        }
    }
}

fn tick(i: &mut Inner, ctx: &Ctx) -> Vec<Action> {
    let mut out = vec![];
    let now = ctx.now;
    if ctx.paused.is_some() || !i.config.rotation.iter().any(|x| x.enabled) {
        // Mirroring (or nothing to show): the engine lets go of the board.
        i.showing = None;
        i.resume = None;
        i.alerts.clear();
        i.messages.clear();
        i.next_requested = false;
        return out;
    }
    // A source removed or turned off while it showed
    if i.showing.as_ref().is_some_and(|s| !i.config.rotation.iter().any(|x| x.id == s.card.instance && x.enabled)) && i.showing.as_ref().is_some_and(|s| s.how == How::Ambient) {
        i.showing = None;
    }
    // A timed notification is up: nothing replaces it, not even a manual Next (which waits).
    if let Some((_, latest)) = i.notice {
        if now < latest {
            return out;
        }
        i.notice = None;
    }
    let quiet = i.config.quiet.active(ctx.local);
    collect_due(i, ctx);
    let force = std::mem::take(&mut i.next_requested);

    // An alert running its course, page by page, then back to what was there before.
    if let Some(s) = i.showing.as_mut().filter(|s| s.how == How::Alert) {
        if now >= s.until || force {
            out.push(Action::Return);
            i.showing = i.resume.take().map(|mut r| {
                // The interrupted card gets its full time again.
                let len = r.until - r.since;
                r.since = now;
                r.page_at = now;
                r.until = now + len;
                r
            });
            if !force {
                return out;
            }
        } else {
            if s.page + 1 < s.pages && now >= s.page_at + s.page_ms {
                s.page += 1;
                s.page_at = now;
                out.push(Action::Show { card: Box::new(s.card.clone()), page: s.page, how: How::Update });
            }
            return out;
        }
    }
    if quiet && !force {
        // Quiet hours: nothing new goes up. Feed alerts are let go (they'd be stale by
        // morning); a celebration or the day's quote waits until quiet hours end.
        let dropped: Vec<Card> = i.alerts.drain(..).collect();
        for c in dropped.iter().filter(|c| c.reason == "alert · keyword") {
            i.seen.mark(&c.instance, &c.seen, now);
        }
        return out;
    }
    if let Some(card) = i.alerts.pop_front() {
        let before = i.showing.take().filter(|s| s.how == How::Ambient);
        i.resume = before;
        let secs = i.config.alert_seconds as i64 * 1000;
        out.push(show(i, card, How::Alert, ctx, secs, Some(secs)));
        return out;
    }
    if let Some(card) = i.messages.pop_front() {
        let hold = (i.config.hold_minutes.max(1) as i64) * 60_000;
        out.push(show(i, card, How::Message, ctx, hold, Some(hold)));
        i.hold_until = Some(now + hold);
        return out;
    }
    if force {
        i.hold_until = None;
    }
    if let Some(h) = i.hold_until {
        if now < h {
            return out;
        }
        i.hold_until = None;
        i.showing = None;
    }
    if let Some(s) = i.showing.as_mut().filter(|_| !force) {
        if s.page + 1 < s.pages && now >= s.page_at + s.page_ms {
            s.page += 1;
            s.page_at = now;
            i.flips.push_back(now);
            out.push(Action::Show { card: Box::new(s.card.clone()), page: s.page, how: How::Ambient });
            return out;
        }
        // A live card (a clock) is re-drawn as it changes, at most once a minute, for as long
        // as it's on the board: also past its dwell, while the hourly cap holds the board.
        if s.card.live && now - s.drawn_at >= 5_000 {
            s.drawn_at = now;
            let id = s.card.instance.clone();
            let fresh = i.config.rotation.iter().find(|x| x.id == id).and_then(|x| i.plugins.iter().find(|p| p.id == x.plugin).and_then(|p| card_for(i, x, p, ctx, 0, false)));
            let s = i.showing.as_mut().unwrap();
            if let Some(c) = fresh {
                let page = s.page;
                if c.face.render(ctx.dims.0, ctx.dims.1) != s.card.face.render(ctx.dims.0, ctx.dims.1) {
                    s.card = c.clone();
                    out.push(Action::Show { card: Box::new(c), page, how: How::Update });
                }
            }
        }
        let s = i.showing.as_ref().unwrap();
        if now < s.until || !out.is_empty() {
            return out;
        }
    }
    // Time for the next card, within the hourly cap (a manual Next is always allowed).
    if !force && flips_in_hour(&i.flips, now) >= i.config.flips_per_hour as usize {
        return out;
    }
    let Some((x, p, card)) = choose(i, ctx, force) else {
        return out;
    };
    let dwell = dwell_of(&x, &p) as i64 * 1000;
    if let Some(s) = i.showing.as_mut() {
        if s.card.instance == card.instance && !force && (s.card.face == card.face || (card.live && s.how == How::Ambient)) {
            // The same card again (it's the only one): it just stays up. A live one that has
            // moved on (a clock, a celebration's next frame) is redrawn in place, which isn't
            // a new card and doesn't count against the hourly cap.
            s.since = now;
            s.until = now + dwell * s.pages as i64;
            if s.card.face != card.face {
                s.card = card.clone();
                s.drawn_at = now;
                let page = s.page;
                out.push(Action::Show { card: Box::new(card), page, how: How::Update });
            }
            return out;
        }
    }
    out.push(show(i, card, How::Ambient, ctx, dwell, None));
    if force {
        // A manual Next doesn't count against the hourly cap.
        i.flips.pop_back();
    }
    out
}

/// Up next: the next hour of the rotation, worked out from the current state as `tick`
/// would play it (schedules, quiet hours, holds and the hourly cap included), plus the
/// alerts and once-a-day messages due in that time.
fn plan(i: &Inner, ctx: &Ctx) -> Vec<Entry> {
    let mut out = vec![];
    if ctx.paused.is_some() || !i.config.rotation.iter().any(|x| x.enabled) {
        return out;
    }
    let end = ctx.now + HOUR_MS;
    let mut t = ctx.now;
    let mut last: Option<String> = None;
    if let Some(s) = &i.showing {
        let reason = match s.how {
            How::Alert => "On now · alert".to_string(),
            How::Message => "On now · message".to_string(),
            _ => format!("On now · {}", s.card.reason),
        };
        out.push(Entry { at: s.since, instance_id: s.card.instance.clone(), title: s.card.name.clone(), reason, now: true });
        t = s.until.max(ctx.now);
        if s.how == How::Alert {
            if let Some(r) = &i.resume {
                out.push(Entry { at: t, instance_id: r.card.instance.clone(), title: r.card.name.clone(), reason: "After the alert".into(), now: false });
                t += r.until - r.since;
                last = Some(r.card.instance.clone());
            }
        } else {
            last = Some(s.card.instance.clone());
        }
    }
    let mut after = "";
    if let Some(h) = i.hold_until.filter(|&h| h > t) {
        if i.showing.is_none() {
            out.push(Entry { at: ctx.now, instance_id: String::new(), title: "Message".into(), reason: format!("Held until {}", short_time(h, i.config.clock24)), now: true });
        }
        t = h;
        after = "after the held message";
    }
    let mut wrr = i.wrr.clone();
    let mut rng = i.rng;
    let mut flips = i.flips.clone();
    let mut taken: HashMap<String, usize> = HashMap::new();
    let mut guard = 0;
    while t < end && out.len() < 40 && guard < 500 {
        guard += 1;
        let at = ctx.at(t);
        if i.config.quiet.active(at.local) {
            t += 60_000;
            after = "after quiet hours";
            continue;
        }
        if flips_in_hour(&flips, t) >= i.config.flips_per_hour as usize {
            t = flips.iter().copied().filter(|&f| f > t - HOUR_MS).min().unwrap_or(t) + HOUR_MS;
            after = "after the hourly cap";
            continue;
        }
        let mut list = candidates(i, &at, &taken);
        if list.is_empty() {
            t += 60_000;
            continue;
        }
        let weights: Vec<(String, u32)> = list.iter().map(|(x, _, _)| (x.id.clone(), x.weight)).collect();
        let k = if i.config.shuffle { pick_shuffle(&mut rng, &weights, last.as_deref()) } else { pick_wrr(&mut wrr, &weights, last.as_deref()) };
        let (x, p, card) = list.swap_remove(k);
        let pages = card.face.render(ctx.dims.0, ctx.dims.1).len().max(1) as i64;
        let dwell = dwell_of(&x, &p) as i64 * 1000 * pages;
        let feed = is_feed(&p, i.feeds.get(&x.id).and_then(|f| f.data.as_ref()));
        if feed {
            *taken.entry(x.id.clone()).or_default() += 1;
        }
        if last.as_deref() == Some(x.id.as_str()) && weights.len() == 1 && !feed {
            // The only source: it just stays up.
            t += dwell;
            continue;
        }
        let mut reason = card.reason.clone();
        if let Some(s) = x.schedule.as_ref().or(p.schedule.as_ref()) {
            reason.push_str(&format!(" · {}", s.describe()));
        }
        if !after.is_empty() {
            reason = format!("{reason} · {after}");
            after = "";
        }
        out.push(Entry { at: t, instance_id: x.id.clone(), title: label(&x, &p), reason: capitalise(&reason), now: false });
        flips.push_back(t);
        last = Some(x.id.clone());
        t += dwell;
    }
    // Alerts and messages due within the hour.
    for x in i.config.rotation.iter().filter(|x| x.enabled) {
        let Some(p) = i.plugins.iter().find(|p| p.id == x.plugin) else { continue };
        match (p.kind.as_str(), p.id.as_str()) {
            ("ical", _) => {
                for it in i.feeds.get(&x.id).and_then(|f| f.data.as_ref()).map(|d| d.items.as_slice()).unwrap_or(&[]) {
                    let at = it.time.unwrap_or(0) - EVENT_LEAD_MS;
                    if !it.all_day && at >= ctx.now && at < end {
                        out.push(Entry { at, instance_id: x.id.clone(), title: it.title.clone(), reason: format!("Alert · {} starts at {}", x.name, short_time(at + EVENT_LEAD_MS, i.config.clock24)), now: false });
                    }
                }
            }
            ("native", builtins::COUNTDOWN) if x.settings.get("celebrate").and_then(|c| c.as_bool()).unwrap_or(false) => {
                if let Some(at) = builtins::countdown_target(&x.settings, ctx.local) {
                    let ms = ctx.now + (at - ctx.local).num_milliseconds();
                    if ms > ctx.now && ms < end {
                        out.push(Entry { at: ms, instance_id: x.id.clone(), title: label(x, p), reason: "Alert · celebration".into(), now: false });
                    }
                }
            }
            ("native", builtins::QUOTE) => {
                if let Some(at) = builtins::quote_next(&x.settings, ctx.local) {
                    let ms = ctx.now + (at - ctx.local).num_milliseconds();
                    if ms < end {
                        out.push(Entry { at: ms, instance_id: x.id.clone(), title: label(x, p), reason: "Message · quote of the day".into(), now: false });
                    }
                }
            }
            _ => {}
        }
    }
    out.sort_by_key(|e| (!e.now, e.at));
    out
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

#[cfg(test)]
pub mod tests {
    use super::*;
    /// An absolute program path on this platform ("/usr/bin/git" isn't absolute on Windows).
    const GIT: &str = if cfg!(windows) { "C:\\Program Files\\Git\\cmd\\git.exe" } else { "/usr/bin/git" };
    use chrono::NaiveDate;

    /// A fresh engine whose files live in a temp folder, and whose secrets go to a file
    /// (the profile store), never the real credential store.
    pub fn engine(name: &str) -> (Engine, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rattleboard-test-{name}-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = secrets::PROFILE_FILE.set(std::env::temp_dir().join(format!("rattleboard-test-secrets-{}.json", std::process::id())));
        let e = Engine::new(Paths { config: Some(dir.join("sources.json")), plugins: Some(dir.join("plugins.json")), seen: Some(dir.join("seen.json")) });
        (e, dir)
    }

    /// Refresh preview fetches again (Settings' button), but not more than once every 5 s;
    /// a plain preview reuses what the source already has.
    #[tokio::test]
    async fn preview_refresh_fetches_again() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static HITS: AtomicUsize = AtomicUsize::new(0);
        let app = axum::Router::new().route("/n", axum::routing::get(|| async {
            let n = HITS.fetch_add(1, Ordering::SeqCst) + 1;
            axum::Json(json!({ "title": format!("COUNT {n}") }))
        }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/n", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await });
        let (e, _) = engine("refresh");
        let id = e.add("rattleboard.json", Some(&json!({ "url": url, "title": "$.title" }))).unwrap();
        let shows = |g: &Vec<Grid>, t: &str| {
            let want = crate::vestaboard::parse(t);
            g[0].iter().any(|row| row.windows(want.len()).any(|w| w == want.as_slice()))
        };
        let c = ctx(0);
        let (g, _) = e.preview(&id, &c, false, false).await.unwrap();
        assert!(shows(&g, "COUNT 1"));
        e.preview(&id, &c, false, false).await.unwrap();
        assert_eq!(HITS.load(Ordering::SeqCst), 1, "a plain preview reuses the data");
        e.preview(&id, &at(&c, 2), false, true).await.unwrap();
        assert_eq!(HITS.load(Ordering::SeqCst), 1, "refreshed within 5 s of the last fetch");
        let (g, _) = e.preview(&id, &at(&c, 10), false, true).await.unwrap();
        assert_eq!(HITS.load(Ordering::SeqCst), 2);
        assert!(shows(&g, "COUNT 2"));
    }

    /// Format 1.1 end to end: a PTV-style signed request (checked by the server), a status
    /// list filtered, sorted and cut, drawn with {{rows}} and filters; card.empty when the
    /// filter leaves nothing; and a status card is live (re-drawn while it's up).
    #[tokio::test]
    async fn signed_status_lists() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static BAD: AtomicUsize = AtomicUsize::new(0);
        const KEY: &str = "aaaabbbb-cccc-dddd-eeee-ffff00001111";
        let app = axum::Router::new().fallback(|uri: axum::http::Uri| async move {
            // What PTV does: the HMAC-SHA1 of path?query (without &signature=...) must match.
            let pq = uri.path_and_query().unwrap().as_str().to_string();
            let (rest, sig) = pq.rsplit_once("&signature=").unwrap_or((&pq, ""));
            if plugins::hmac_hex("hmac-sha1", KEY.as_bytes(), rest.as_bytes(), true) != sig {
                BAD.fetch_add(1, Ordering::SeqCst);
                return (axum::http::StatusCode::FORBIDDEN, "bad signature").into_response();
            }
            axum::Json(json!({"departures": [
                {"route_id": 6, "direction_id": 1, "run_ref": "a", "platform_number": "2", "scheduled_departure_utc": "2026-10-19T12:09:00Z", "estimated_departure_utc": null},
                {"route_id": 6, "direction_id": 1, "run_ref": "b", "platform_number": "1", "scheduled_departure_utc": "2026-10-19T12:03:00Z", "estimated_departure_utc": "2026-10-19T12:04:00Z"},
                {"route_id": 16, "direction_id": 8, "run_ref": "c", "platform_number": "13", "scheduled_departure_utc": "2026-10-19T12:01:00Z", "estimated_departure_utc": null},
                {"route_id": 6, "direction_id": 1, "run_ref": "d", "platform_number": "2", "scheduled_departure_utc": "2026-10-19T12:30:00Z", "estimated_departure_utc": null}
            ], "status": {"version": "3.0", "health": 1}})).into_response()
        });
        use axum::response::IntoResponse;
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await });
        let manifest = json!({"format": 1, "id": "com.example.trains", "name": "Trains", "kind": "json",
            "fetch": {"url": format!("{base}/v3/departures/route_type/0/stop/{{{{settings.stop}}}}?max_results=6&devid={{{{settings.devid}}}}"), "every": "1m",
                      "sign": {"type": "hmac-sha1", "key": "{{secret.key}}"}},
            "pick": {"items": "$.departures[*]", "id": "$.run_ref", "title": "$.platform_number", "as": "status",
                     "fields": {"when": ["$.estimated_departure_utc", "$.scheduled_departure_utc"], "platform": "$.platform_number"},
                     "where": [{"path": "$.direction_id", "op": "==", "value": "{{settings.direction}}"}],
                     "sort": {"path": "$.scheduled_departure_utc"}},
            "maxItems": 2,
            "card": {"template": "{{settings.label|upper}}\n{{rows}}", "row": "{{field.when|mins|pad:-2}} MIN PLAT {{field.platform}}", "empty": "No trains", "align": "left"},
            "settings": [{"key": "devid"}, {"key": "stop", "default": "1071"}, {"key": "label", "default": "Flinders St"}, {"key": "direction"}],
            "secrets": [{"key": "key"}]});
        let (e, _) = engine("signed");
        e.install(&manifest, false).unwrap();
        let id = e.add("com.example.trains", Some(&json!({"devid": "1000123", "direction": "1"}))).unwrap();
        let text = |g: &Grid| crate::cards::ascii(g).join("\n");
        // No key yet: refused before anything is sent
        let (g, reason) = e.preview(&id, &ctx(0), false, false).await.unwrap();
        assert!(reason.contains("signing key"), "{reason}: {}", text(&g[0]));
        e.set_secrets(&id, &json!({"key": KEY})).unwrap();
        let c = ctx(0);
        let (g, reason) = e.preview(&id, &c, false, true).await.unwrap();
        assert_eq!(BAD.load(Ordering::SeqCst), 0, "the server accepted the signature");
        assert_eq!(reason, "ambient · status");
        // 12:00 now (local taken as UTC): run b at 12:04 (estimated), run a at 12:09; c is the other way, d cut by maxItems
        assert_eq!(text(&g[0]), ["                      ", "FLINDERS ST           ", " 4 MIN PLAT 1         ", " 9 MIN PLAT 2         ", "                      ", "                      "].join("\n"));
        {
            let i = e.inner.lock().unwrap();
            let x = i.config.rotation[0].clone();
            let p = i.plugins.iter().find(|p| p.id == x.plugin).unwrap().clone();
            assert!(card_for(&i, &x, &p, &c, 0, false).unwrap().live, "a status list is live");
        }
        // A direction with nothing in it: card.empty
        let mut rot = e.view()["rotation"].clone();
        rot[0]["settings"]["direction"] = json!("5");
        e.put_rotation(&json!({"rotation": rot}), false).unwrap();
        let (g, _) = e.preview(&id, &at(&c, 10), false, true).await.unwrap();
        assert!(text(&g[0]).contains("NO TRAINS"), "{}", text(&g[0]));
    }

    /// A feed item's link goes with its card, and so does a digest's (its newest item's).
    #[test]
    fn cards_carry_links() {
        let (e, _) = engine("links");
        e.install(&feed_plugin("com.example.feed"), false).unwrap();
        e.put_rotation(&json!([{"id": "f", "plugin": "com.example.feed"}]), false).unwrap();
        let item = |k: &str, link: Option<&str>| Item { key: k.into(), title: format!("Story {k}"), time: Some(1), link: link.map(String::from), ..Default::default() };
        e.store("f", Ok(Data { items: vec![item("a", Some("https://news.example.com/a"))], single: false }), 0);
        let a = e.tick(&ctx(0));
        let Some(Action::Show { card, .. }) = a.first() else { panic!("{a:?}") };
        assert_eq!(card.link.as_deref(), Some("https://news.example.com/a"));
        assert_eq!(serde_json::to_value(card.as_ref()).unwrap()["link"], "https://news.example.com/a");
        // The clock has none
        let (e, _) = engine("links2");
        e.put_rotation(&json!([{"id": "c", "plugin": "rattleboard.clock"}]), false).unwrap();
        let a = e.tick(&ctx(0));
        let Some(Action::Show { card, .. }) = a.first() else { panic!() };
        assert_eq!(card.link, None);
    }

    pub fn ctx(min: i64) -> Ctx {
        // 2026-10-19 (a Monday) 12:00 local, plus `min` minutes; local time is taken as UTC here
        let base = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap().and_hms_opt(12, 0, 0).unwrap();
        let local = base + chrono::Duration::minutes(min);
        Ctx { now: local.and_utc().timestamp_millis(), local, paused: None, dims: (6, 22) }
    }

    fn at(c: &Ctx, secs: i64) -> Ctx {
        c.at(c.now + secs * 1000)
    }

    fn shows(a: &[Action]) -> Vec<(String, How)> {
        a.iter().map(|a| match a {
            Action::Show { card, how, .. } => (card.instance.clone(), *how),
            Action::Return => ("return".into(), How::Update),
        }).collect()
    }

    fn text_plugin(id: &str) -> Value {
        json!({"format": 1, "id": id, "name": id, "kind": "text", "every": "10s", "card": {"template": id}})
    }

    fn feed_plugin(id: &str) -> Value {
        json!({"format": 1, "id": id, "name": "Feed", "kind": "rss", "fetch": {"url": "https://example.com/feed"}, "alertWhen": ["keyword:outage"]})
    }

    fn items(keys: &[&str]) -> Data {
        Data { items: keys.iter().enumerate().map(|(n, k)| Item { key: k.to_string(), title: format!("Story {k}"), time: Some(1000 - n as i64), ..Default::default() }).collect(), single: false }
    }

    #[test]
    fn rotation_weights_and_dwell() {
        let (e, _) = engine("rotation");
        for id in ["com.t.a", "com.t.b"] {
            e.install(&text_plugin(id), false).unwrap();
        }
        e.put_rotation(&json!([
            {"id": "a", "plugin": "com.t.a", "dwell": "30s", "weight": 2},
            {"id": "b", "plugin": "com.t.b", "dwell": "30s"},
        ]), false).unwrap();
        let c0 = ctx(0);
        let mut order = vec![];
        for k in 0..6 {
            for (id, how) in shows(&e.tick(&at(&c0, k * 30))) {
                assert_eq!(how, How::Ambient);
                order.push(id);
            }
            // within the dwell, nothing changes
            assert!(e.tick(&at(&c0, k * 30 + 10)).is_empty());
        }
        // weight 2 comes round twice as often: a, b, a, a (it just stays up), b, a
        assert_eq!(order, ["a", "b", "a", "b", "a"]);
        // Next: the other one at once, with its dwell restarted
        e.request_next();
        assert_eq!(shows(&e.tick(&at(&c0, 181))), [("b".to_string(), How::Ambient)]);
        assert!(e.tick(&at(&c0, 200)).is_empty());
        assert_eq!(shows(&e.tick(&at(&c0, 211))).len(), 1);
        // turned off: it stops coming round
        e.put_rotation(&json!([{"id": "a", "plugin": "com.t.a", "dwell": "30s"}, {"id": "b", "plugin": "com.t.b", "enabled": false}]), false).unwrap();
        e.tick(&at(&c0, 300));
        for k in 0..4 {
            for (id, _) in shows(&e.tick(&at(&c0, 330 + k * 30))) {
                assert_eq!(id, "a");
            }
        }
    }

    #[test]
    fn schedules_quiet_hours_and_cap() {
        let (e, _) = engine("schedule");
        e.install(&text_plugin("com.t.a"), false).unwrap();
        e.install(&text_plugin("com.t.night"), false).unwrap();
        e.put_rotation(&json!({"rotation": [
            {"id": "a", "plugin": "com.t.a", "dwell": "10s"},
            {"id": "night", "plugin": "com.t.night", "dwell": "10s", "schedule": {"from": "22:00", "to": "06:00"}},
        ], "flipsPerHour": 3}), false).unwrap();
        let c0 = ctx(0); // noon: only "a" is in its schedule
        assert_eq!(shows(&e.tick(&c0)), [("a".to_string(), How::Ambient)]);
        assert!(shows(&e.tick(&at(&c0, 11))).is_empty());
        // At night both take turns; three flips, then the cap holds the board until the hour's up.
        let n0 = ctx(10 * 60 + 30);
        let mut n = 0;
        for k in 0..12 {
            n += shows(&e.tick(&at(&n0, k * 11))).len();
        }
        assert_eq!(n, 3, "three flips, then the cap");
        // Quiet hours: nothing new goes up, until a manual Next.
        e.put_rotation(&json!({"flipsPerHour": 30, "quietHours": {"enabled": true, "from": "22:00", "to": "07:00"}}), false).unwrap();
        let q0 = ctx(11 * 60);
        for k in 0..5 {
            assert!(e.tick(&at(&q0, k * 11)).is_empty());
        }
        e.request_next();
        assert_eq!(shows(&e.tick(&at(&q0, 60))).len(), 1);
        assert!(e.up_next(&at(&q0, 61)).iter().filter(|x| !x.now).all(|x| x.reason.contains("quiet hours") || x.at >= ctx(19 * 60).now));
    }

    /// An agent's timed notification (show_status) interrupts like an alert: nothing replaces
    /// it, it never holds the rotation, and once it's over the rotation carries on, the
    /// interrupted card getting the time it lost. A message that stays (send_message) holds
    /// as any message does, and after the hold the rotation comes back: nothing re-shows the
    /// message or anything from before it.
    #[test]
    fn notices_interrupt_without_holding() {
        let (e, _) = engine("notice");
        e.install(&text_plugin("com.t.a"), false).unwrap();
        e.install(&text_plugin("com.t.b"), false).unwrap();
        e.put_rotation(&json!({"rotation": [
            {"id": "a", "plugin": "com.t.a", "dwell": "60s"},
            {"id": "b", "plugin": "com.t.b", "dwell": "60s"},
        ], "holdMinutes": 30}), false).unwrap();
        let c0 = ctx(0);
        assert_eq!(shows(&e.tick(&c0)), [("a".to_string(), How::Ambient)]);
        // show_status at 30 s for 15 s (and its flaps): no card goes up over it, not even when
        // a's time would be up.
        e.begin_notice(at(&c0, 30).now, 17_000);
        for t in [31, 40, 46, 61, 70] {
            assert!(e.tick(&at(&c0, t)).is_empty(), "{t}");
        }
        assert_eq!(e.status().hold_until, None, "a notification never holds the rotation");
        // It ended at 75 s (late, as a slow board might): a gets back the 45 s it lost, so
        // it's up until 105 s, then the rotation goes on.
        e.end_notice(at(&c0, 75).now);
        assert_eq!(e.inner.lock().unwrap().showing.as_ref().map(|s| s.until), Some(at(&c0, 105).now));
        assert!(e.tick(&at(&c0, 76)).is_empty());
        assert_eq!(shows(&e.tick(&at(&c0, 106))), [("b".to_string(), How::Ambient)]);
        assert_eq!(e.status().hold_until, None);
        assert_eq!(shows(&e.tick(&at(&c0, 167))), [("a".to_string(), How::Ambient)]);

        // A notification that's never ended lets go by itself a minute after it should have.
        e.begin_notice(at(&c0, 140).now, 10_000);
        assert!(e.tick(&at(&c0, 205)).is_empty());
        assert!(e.inner.lock().unwrap().notice.is_some());
        e.tick(&at(&c0, 211));
        assert!(e.inner.lock().unwrap().notice.is_none());
        assert_eq!(shows(&e.tick(&at(&c0, 228))), [("b".to_string(), How::Ambient)]);

        // send_message: a message, held for 30 minutes, then the rotation, and nothing else:
        // simulated minute by minute for an hour, the only things shown are rotation cards.
        let t0 = 300;
        e.note_message(at(&c0, t0).now);
        let mut shown = vec![];
        for m in 0..60 {
            for (id, how) in shows(&e.tick(&at(&c0, t0 + m * 60 + 1))) {
                shown.push((m, id, how));
            }
        }
        assert!(shown.iter().all(|(m, _, how)| *m >= 30 && *how == How::Ambient), "{shown:?}");
        assert_eq!(shown.first().map(|s| s.0), Some(30));
        // A message during a notification ends it: the hold applies, nothing returns.
        e.begin_notice(at(&c0, 4000).now, 15_000);
        e.note_message(at(&c0, 4005).now);
        assert!(e.inner.lock().unwrap().notice.is_none());
        assert!(e.tick(&at(&c0, 4100)).is_empty(), "held");
    }

    #[test]
    fn messages_hold_and_alerts_return() {
        let (e, dir) = engine("priority");
        e.install(&text_plugin("com.t.a"), false).unwrap();
        e.install(&feed_plugin("com.t.feed"), false).unwrap();
        e.put_rotation(&json!({"rotation": [
            {"id": "a", "plugin": "com.t.a", "dwell": "60s"},
            {"id": "feed", "plugin": "com.t.feed", "dwell": "60s"},
        ], "holdMinutes": 30, "alertSeconds": 20}), false).unwrap();
        let c0 = ctx(0);
        assert_eq!(shows(&e.tick(&c0)), [("a".to_string(), How::Ambient)]);
        // The feed's first fetch: its newest item queued, the rest remembered as seen.
        e.store("feed", Ok(items(&["k3", "k2", "k1"])), c0.now);
        assert!(e.tick(&at(&c0, 30)).is_empty());
        assert_eq!(shows(&e.tick(&at(&c0, 61))), [("feed".to_string(), How::Ambient)]);
        // An item mentioning "outage" interrupts as an alert, then the board goes back.
        let mut d = items(&["k4"]);
        d.items[0].title = "Power outage downtown".into();
        e.store("feed", Ok(d), at(&c0, 70).now);
        assert_eq!(shows(&e.tick(&at(&c0, 71))), [("feed".to_string(), How::Alert)]);
        assert_eq!(e.showing().unwrap().1, How::Alert);
        assert!(e.tick(&at(&c0, 80)).is_empty());
        assert_eq!(shows(&e.tick(&at(&c0, 92))), [("return".to_string(), How::Update)]);
        assert_eq!(e.showing().unwrap(), ("feed".to_string(), How::Ambient));
        // A message (Compose, the API) holds the rotation for 30 minutes.
        e.note_message(at(&c0, 100).now);
        for s in [130, 600, 1800] {
            assert!(e.tick(&at(&c0, s)).is_empty(), "{s}");
        }
        let up = e.up_next(&at(&c0, 200));
        assert!(up[0].reason.starts_with("Held until"), "{up:?}");
        assert!(up[1].reason.contains("after the held message"), "{up:?}");
        assert_eq!(shows(&e.tick(&at(&c0, 1901))).len(), 1);
        // An alert during a held message goes back to the message (lib.rs keeps the layout).
        e.note_message(at(&c0, 2000).now);
        let mut d = items(&["k5"]);
        d.items[0].title = "Another outage".into();
        e.store("feed", Ok(d), at(&c0, 2001).now);
        assert_eq!(shows(&e.tick(&at(&c0, 2002))), [("feed".to_string(), How::Alert)]);
        assert_eq!(shows(&e.tick(&at(&c0, 2023))), [("return".to_string(), How::Update)]);
        assert!(e.tick(&at(&c0, 2030)).is_empty(), "still held");
        // Seen items are remembered across restarts: none come back.
        drop(e);
        let e2 = Engine::new(Paths { config: Some(dir.join("sources.json")), plugins: Some(dir.join("plugins.json")), seen: Some(dir.join("seen.json")) });
        e2.store("feed", Ok(items(&["k5", "k4", "k3", "k2", "k1"])), c0.now);
        let i = e2.inner.lock().unwrap();
        assert!(i.feeds["feed"].queue.is_empty() && i.feeds["feed"].digest.is_none());
    }

    #[test]
    fn digests() {
        let (e, _) = engine("digest");
        e.install(&feed_plugin("com.t.feed"), false).unwrap();
        e.put_rotation(&json!([{"id": "feed", "plugin": "com.t.feed", "name": "HN", "dwell": "30s"}]), false).unwrap();
        let c0 = ctx(0);
        e.store("feed", Ok(items(&["a"])), c0.now);
        assert_eq!(shows(&e.tick(&c0)).len(), 1);
        // Five new at once: one digest card, "5 NEW · HN" and the newest item.
        e.store("feed", Ok(items(&["f", "e", "d", "c", "b", "a"])), at(&c0, 10).now);
        let a = e.tick(&at(&c0, 31));
        let Some(Action::Show { card, .. }) = a.first() else { panic!("{a:?}") };
        let Face::Standard { name, body, .. } = &card.face else { panic!() };
        assert_eq!((name.as_str(), body.as_str()), ("5 new - HN", "Story f"));
        assert!(card.reason.contains("digest"));
        // ...and then nothing more until something new arrives
        assert!(e.tick(&at(&c0, 62)).is_empty());
        assert!(e.tick(&at(&c0, 93)).is_empty());
        // two new: shown one at a time, oldest first
        e.store("feed", Ok(items(&["h", "g", "f"])), at(&c0, 100).now);
        let bodies: Vec<String> = (0..2).flat_map(|k| e.tick(&at(&c0, 124 + k * 31))).filter_map(|a| match a {
            Action::Show { card, .. } => match card.face {
                Face::Standard { body, .. } => Some(body),
                _ => None,
            },
            _ => None,
        }).collect();
        assert_eq!(bodies, ["Story g", "Story h"]);
    }

    #[test]
    fn countdowns_celebrate_and_quotes_hold() {
        let (e, _) = engine("countdown");
        e.put_rotation(&json!({"rotation": [
            {"id": "bday", "plugin": "rattleboard.countdown", "settings": {"title": "Sam's birthday", "date": "1990-10-19", "yearly": true, "celebrate": true}},
            {"id": "quote", "plugin": "rattleboard.quote", "settings": {"mode": "daily", "at": "12:30"}},
        ], "alertSeconds": 30, "holdMinutes": 10}), false).unwrap();
        let c0 = ctx(0); // the birthday itself, at noon
        let a = e.tick(&c0);
        assert_eq!(shows(&a), [("bday".to_string(), How::Alert)]);
        let Some(Action::Show { card, .. }) = a.first() else { panic!() };
        assert!(matches!(&card.face, Face::Celebrate { text, .. } if text == "Happy birthday Sam"));
        // Back after 30 s, and the celebration doesn't alert again that day.
        assert_eq!(shows(&e.tick(&at(&c0, 31))), [("return".to_string(), How::Update)]);
        assert_eq!(shows(&e.tick(&at(&c0, 32))), [("bday".to_string(), How::Ambient)]);
        let up = e.up_next(&at(&c0, 40));
        assert!(up.iter().any(|x| x.instance_id == "quote" && x.reason.contains("quote of the day")), "{up:?}");
        // 12:30: the day's quote, as a message that holds the rotation for 10 minutes
        assert_eq!(shows(&e.tick(&ctx(30))), [("quote".to_string(), How::Message)]);
        assert!(e.tick(&ctx(35)).is_empty() && e.tick(&ctx(39)).is_empty());
        assert_eq!(shows(&e.tick(&ctx(41))), [("bday".to_string(), How::Ambient)]);
        // Once a day only: from here the celebration in the rotation just steps through its
        // confetti frames, one redraw a minute (updates, not new cards or alerts).
        let later = e.tick(&ctx(45));
        assert_eq!(shows(&later), [("bday".to_string(), How::Update)]);
    }

    /// The colourful clock (the new default) changes once a minute, as updates that never
    /// count against flipsPerHour, and keeps going once its dwell is over and the hourly cap
    /// holds the board (it used to freeze there).
    #[test]
    fn colour_clock_is_calm() {
        let (e, _) = engine("calmclock");
        e.put_rotation(&json!({"rotation": [{"id": "clock", "plugin": "rattleboard.clock", "dwell": "1m"}], "flipsPerHour": 1}), false).unwrap();
        assert_eq!(e.view()["rotation"][0]["settings"]["style"], "colour");
        assert_eq!(e.view()["rotation"][0]["settings"]["colour"], "playful");
        let c = ctx(0);
        assert_eq!(shows(&e.tick(&c)), [("clock".to_string(), How::Ambient)]);
        let mut updates = 0;
        for s in 1..=(5 * 60) {
            let a = e.tick(&at(&c, s));
            for (id, how) in shows(&a) {
                assert_eq!((id.as_str(), how), ("clock", How::Update), "second {s}");
                updates += 1;
            }
        }
        // 12:00 → 12:05: one change at each minute boundary, nothing in between
        assert_eq!(updates, 5);
        assert_eq!(e.inner.lock().unwrap().flips.len(), 1, "only the first show counted");
        // A quote saved by an older version with its chip off is now Colour: Off
        e.put_rotation(&json!({"rotation": [{"id": "q", "plugin": "rattleboard.quote", "settings": {"accent": false}}]}), false).unwrap();
        assert_eq!(e.view()["rotation"][0]["settings"]["colour"], "off");
    }

    #[test]
    fn paused_while_mirroring() {
        let (e, _) = engine("mirror");
        e.put_rotation(&json!([{"id": "clock", "plugin": "rattleboard.clock"}]), false).unwrap();
        let mut c = ctx(0);
        c.paused = Some("Mirroring Local API".into());
        assert!(e.tick(&c).is_empty());
        assert!(e.up_next(&c).is_empty());
        assert!(!e.status().active);
        c.paused = None;
        assert_eq!(shows(&e.tick(&c)), [("clock".to_string(), How::Ambient)]);
        assert!(e.status().active);
        // The clock is live: the next minute is an update, not a new card.
        assert_eq!(shows(&e.tick(&at(&c, 61))), [("clock".to_string(), How::Update)]);
        assert!(e.tick(&at(&c, 70)).is_empty());
    }

    #[test]
    fn validation_and_secrets() {
        let (e, _) = engine("validate");
        for (body, msg) in [
            (json!([{"plugin": "nope"}]), "Unknown plugin"),
            (json!([{"plugin": "rattleboard.clock", "weight": 11}]), "weight"),
            (json!([{"plugin": "rattleboard.clock", "dwell": "2s"}]), "dwell"),
            (json!([{"plugin": "rattleboard.clock", "settings": {"style": "huge"}}]), "Style"),
            (json!([{"plugin": "rattleboard.clock", "schedule": {"days": ["xyz"]}}]), "Unknown day"),
            (json!([{"plugin": "rattleboard.clock", "chip": "pink"}]), "chip"),
            (json!([{"id": "x", "plugin": "rattleboard.clock"}, {"id": "x", "plugin": "rattleboard.clock"}]), "Two sources"),
            (json!({"rotation": [], "holdMinutes": -1}), "holdMinutes"),
            (json!({"quietHours": {"from": "late"}}), "quietHours.from"),
            (json!("x"), "Send"),
        ] {
            let r = e.put_rotation(&body, true).unwrap_err();
            assert!(r.0 == 400 && r.1.contains(msg), "{body}: {r:?}");
        }
        e.put_rotation(&json!([{"id": "cal", "plugin": "rattleboard.ical"}]), true).unwrap();
        assert_eq!(e.set_secrets("cal", &json!({"nope": "x"})).unwrap_err().0, 400);
        assert_eq!(e.set_secrets("missing", &json!({"url": "x"})).unwrap_err().0, 404);
        e.set_secrets("cal", &json!({"url": "https://calendar.example.com/private-abc123/basic.ics"})).unwrap();
        let v = e.view();
        assert_eq!(v["rotation"][0]["secrets"], json!({"url": {"set": true}}));
        assert!(!v.to_string().contains("private-abc123"));
        // removing the source deletes its secret
        e.put_rotation(&json!([]), true).unwrap();
        assert_eq!(e.secret("cal", "url"), "");
    }

    #[test]
    fn commands_need_approval() {
        let (e, _) = engine("command");
        let cmd = json!({"format": 1, "id": "com.t.git", "name": "Git", "kind": "command", "run": {"path": GIT, "args": ["status"]}});
        assert_eq!(e.install(&cmd, true).unwrap_err().0, 403);
        e.install(&cmd, false).unwrap();
        let id = e.add("com.t.git", None).unwrap();
        let v = e.view();
        assert_eq!(v["rotation"][0]["enabled"], false, "never enabled by import alone");
        // Over the API it can't be turned on...
        let on = json!([{"id": id, "plugin": "com.t.git", "enabled": true}]);
        assert_eq!(e.put_rotation(&on, true).unwrap_err().0, 403);
        // ...and here it still doesn't run until approved.
        e.put_rotation(&on, false).unwrap();
        assert!(e.due_jobs(0).is_empty());
        let fp = plugins::fingerprint(&cmd);
        assert_eq!(e.approve("com.t.git", "stale", true).unwrap_err().0, 409);
        e.approve("com.t.git", &fp, true).unwrap();
        assert_eq!(e.due_jobs(0).len(), 1);
        // A changed manifest needs approving again.
        let mut changed = cmd.clone();
        changed["run"]["args"] = json!(["push"]);
        e.install(&changed, false).unwrap();
        e.store(&id, Err("x".into()), 0);
        assert!(e.due_jobs(i64::MAX).is_empty());
        assert_eq!(e.uninstall("rattleboard.clock").unwrap_err().0, 400);
        assert_eq!(e.uninstall("com.nope").unwrap_err().0, 404);
        e.uninstall("com.t.git").unwrap();
        assert!(e.view()["rotation"].as_array().unwrap().is_empty());
    }

    #[test]
    fn boards_without_commands_never_run_them() {
        let dir = std::env::temp_dir().join(format!("rattleboard-nocmd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = || Paths { config: Some(dir.join("sources.json")), plugins: Some(dir.join("plugins.json")), seen: None };
        let cmd = json!({"format": 1, "id": "com.t.git", "name": "Git", "kind": "command", "run": {"path": GIT, "args": ["status"]}});
        // Installed and approved where commands are allowed (the desktop)...
        let e = Engine::new(paths());
        e.install(&cmd, false).unwrap();
        let id = e.add("com.t.git", None).unwrap();
        e.approve("com.t.git", &plugins::fingerprint(&cmd), true).unwrap();
        e.put_rotation(&json!([{"id": id, "plugin": "com.t.git", "enabled": true}]), false).unwrap();
        assert_eq!(e.due_jobs(0).len(), 1);
        // ...the same files on a board without commands: nothing runs, nothing installs.
        let w = Engine::without_commands(paths());
        assert!(w.due_jobs(0).is_empty());
        assert_eq!(w.install(&cmd, false).unwrap_err().0, 403);
        assert_eq!(w.install(&cmd, true).unwrap_err().0, 403);
        assert_eq!(w.approve("com.t.git", &plugins::fingerprint(&cmd), true).unwrap_err().0, 403);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn weighted_round_robin() {
        let mut wrr = HashMap::new();
        let list = vec![("a".to_string(), 5), ("b".to_string(), 1), ("c".to_string(), 1)];
        let seq: String = (0..7).map(|_| ["a", "b", "c"][pick_wrr(&mut wrr, &list, None)]).collect();
        assert_eq!(seq, "aabacaa");
        let mut rng = 42;
        let mut last: Option<String> = None;
        for _ in 0..50 {
            let k = pick_shuffle(&mut rng, &list, last.as_deref());
            assert_ne!(Some(list[k].0.clone()), last);
            last = Some(list[k].0.clone());
        }
    }
}
