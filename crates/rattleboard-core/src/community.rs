//! Community plugins: the read-only directory at github.com/slastrina/rattleboard-plugins.
//! The apps read its index.json (cached for a day), each plugin's README and preview card,
//! and its manifest, which is then installed through the usual confirm-and-install path.
//! Only addresses under the repository's two prefixes are ever fetched or opened.

use crate::plugins;
use crate::vestaboard::Grid;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

pub const REPO_URL: &str = "https://github.com/slastrina/rattleboard-plugins";
pub const CONTRIBUTE_URL: &str = "https://github.com/slastrina/rattleboard-plugins/blob/main/CONTRIBUTING.md";
pub const RAW_PREFIX: &str = "https://raw.githubusercontent.com/slastrina/rattleboard-plugins/";
pub const WEB_PREFIX: &str = "https://github.com/slastrina/rattleboard-plugins/";
/// What a development base URL stands in for (see `resolve`).
const RAW_MAIN: &str = "https://raw.githubusercontent.com/slastrina/rattleboard-plugins/main/";
pub const INDEX_URL: &str = "https://raw.githubusercontent.com/slastrina/rattleboard-plugins/main/index-v2.json";
pub const FORMAT: u64 = 1;
pub const MAX_INDEX: usize = 2 * 1024 * 1024;
pub const MAX_README: usize = 64 * 1024;
pub const MAX_PREVIEW: usize = 64 * 1024;
/// index.json is fetched again once the cached copy is a day old.
pub const CACHE_MS: i64 = 24 * 3600 * 1000;
pub const OFFLINE: &str = "Couldn't load community plugins. Check your connection.";

/// The address, normalised, if it is under one of the repository's two prefixes: https, no
/// port or user name, and still under the prefix once dot segments are resolved.
pub fn allowed(u: &str) -> Option<String> {
    let url = url::Url::parse(u.trim()).ok()?;
    if url.scheme() != "https" || url.port().is_some() || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let s = url.to_string();
    [RAW_PREFIX, WEB_PREFIX].iter().any(|p| s.len() > p.len() && s.starts_with(p)).then_some(s)
}

pub fn is_allowed(u: &str) -> bool {
    allowed(u).is_some()
}

/// Debug builds can serve the repository locally (RATTLEBOARD_COMMUNITY_BASE): raw files
/// on its main branch are then read from that base instead. Release builds pass None.
pub fn resolve(url: &str, base: Option<&str>) -> String {
    match (base, url.strip_prefix(RAW_MAIN)) {
        (Some(b), Some(rest)) => format!("{}/{rest}", b.trim_end_matches('/')),
        _ => url.to_string(),
    }
}

/// A GitHub user name, as in an author's profile link.
pub fn profile_url(login: &str) -> Option<String> {
    let ok = (1..=39).contains(&login.len())
        && login.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !login.starts_with('-')
        && !login.ends_with('-');
    ok.then(|| format!("https://github.com/{login}"))
}

// ---------------------------------------------------------------- index.json

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Author {
    pub name: String,
    pub github: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Field {
    pub key: String,
    pub label: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub author: Author,
    pub tags: Vec<String>,
    pub version: String,
    pub license: String,
    pub homepage: Option<String>,
    pub kind: String,
    pub needs_secrets: bool,
    /// The oldest Rattleboard the plugin works with (semver), if it says
    pub min_app_version: Option<String>,
    pub settings: Vec<Field>,
    pub secrets: Vec<Field>,
    pub hosts: Vec<String>,
    pub manifest_url: String,
    pub readme_url: Option<String>,
    pub preview_url: Option<String>,
    pub folder_url: Option<String>,
    pub updated: String,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Index {
    pub generated: String,
    pub plugins: Vec<Entry>,
}

fn clip(s: &str, n: usize) -> String {
    s.trim().chars().take(n).collect()
}

/// Reads index.json. Unknown fields are ignored; an entry that's malformed, a command plugin,
/// or whose manifest lives anywhere but the repository is left out; other addresses outside it
/// are dropped.
pub fn parse_index(bytes: &[u8]) -> Result<Index, String> {
    if bytes.len() > MAX_INDEX {
        return Err("The community index is larger than 2 MB".into());
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|_| "The community index isn't valid JSON".to_string())?;
    match v.get("format").and_then(|f| f.as_u64()) {
        Some(FORMAT) => {}
        Some(_) => return Err("The community index is in a newer format; update Rattleboard to browse it".into()),
        None => return Err("The community index has no format".into()),
    }
    let mut out = Index { generated: v["generated"].as_str().unwrap_or("").to_string(), plugins: vec![] };
    for raw in v["plugins"].as_array().into_iter().flatten() {
        let Ok(mut e) = serde_json::from_value::<Entry>(raw.clone()) else { continue };
        let id_ok = (3..=100).contains(&e.id.len()) && e.id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-');
        if !id_ok || e.name.trim().is_empty() || !plugins::USER_KINDS.contains(&e.kind.as_str()) || e.kind == "command" {
            continue;
        }
        if out.plugins.iter().any(|p| p.id == e.id) {
            continue;
        }
        let Some(m) = allowed(&e.manifest_url) else { continue };
        e.manifest_url = m;
        for u in [&mut e.readme_url, &mut e.preview_url, &mut e.folder_url] {
            *u = u.as_deref().and_then(allowed);
        }
        e.min_app_version = e.min_app_version.map(|v| clip(&v, 40)).filter(|v| !v.is_empty());
        e.homepage = e.homepage.filter(|h| url::Url::parse(h).is_ok_and(|u| u.scheme() == "https"));
        e.author.github = e.author.github.filter(|g| profile_url(g).is_some());
        e.name = clip(&e.name, 60);
        e.summary = clip(&e.summary, 200);
        e.tags = e.tags.iter().map(|t| clip(t, 20)).filter(|t| !t.is_empty()).take(10).collect();
        e.hosts.truncate(20);
        e.settings.truncate(40);
        e.secrets.truncate(20);
        out.plugins.push(e);
    }
    Ok(out)
}

/// The version of Rattleboard a plugin needs, when it's newer than `app` (this one is
/// `APP_VERSION`): such a plugin is shown but can't be installed. A minAppVersion that isn't
/// a version is ignored.
pub fn needs_newer_app(min: Option<&str>, app: &str) -> Option<String> {
    min.filter(|m| crate::update::is_newer(m, app)).map(|m| m.trim_start_matches('v').to_string())
}

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether the index has a newer version than the one installed (None: not installed, or
/// installed with no version known).
pub fn update_available(installed: Option<&str>, index: &str) -> bool {
    installed.is_some_and(|v| crate::update::is_newer(index, v))
}

// ---------------------------------------------------------------- plugin updates

/// What a plugin is and may do, as far as an update could change it: from an installed
/// manifest, from a fetched one, or (without the script) from the directory's entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub kind: String,
    /// Host names it fetches from, lower-cased; "set by you" for a host made from settings
    pub hosts: Vec<String>,
    /// (key, label)
    pub settings: Vec<(String, String)>,
    pub secrets: Vec<(String, String)>,
    /// A script plugin's code; None when it isn't known (the directory entry has none)
    pub script: Option<String>,
}

/// As the directory's index says it, for a host built from placeholders.
pub const SET_BY_YOU: &str = "set by you";

/// The host a fetch URL template reaches, as the directory's index tool works it out.
fn url_host(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://").map_or(url.trim(), |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains("{{") {
        return Some(SET_BY_YOU.into());
    }
    let host = authority.rsplit('@').next().unwrap_or("");
    let host = if host.starts_with('[') { host } else { host.split(':').next().unwrap_or("") };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

impl Facts {
    pub fn of_manifest(m: &plugins::Manifest) -> Self {
        let hosts = match (&m.script, &m.fetch) {
            (Some(s), _) => s.hosts.iter().map(|h| h.to_ascii_lowercase()).collect(),
            (None, Some(f)) => url_host(&f.url).into_iter().collect(),
            _ => vec![],
        };
        Facts {
            kind: m.kind.clone(),
            hosts,
            settings: m.settings.iter().map(|f| (f.key.clone(), f.label.clone())).collect(),
            secrets: m.secrets.iter().map(|f| (f.key.clone(), f.label.clone())).collect(),
            script: m.script.as_ref().map(|s| s.source.clone()),
        }
    }

    pub fn of_entry(e: &Entry) -> Self {
        Facts {
            kind: e.kind.clone(),
            hosts: e.hosts.iter().map(|h| h.to_ascii_lowercase()).collect(),
            settings: e.settings.iter().map(|f| (f.key.clone(), f.label.clone())).collect(),
            secrets: e.secrets.iter().map(|f| (f.key.clone(), f.label.clone())).collect(),
            script: None,
        }
    }
}

/// What an update changes, against the installed manifest, for the review sheet.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    /// (installed, new), when the kind changes
    pub kind: Option<(String, String)>,
    pub hosts_added: Vec<String>,
    pub hosts_removed: Vec<String>,
    /// Labels
    pub settings_added: Vec<String>,
    pub settings_removed: Vec<String>,
    pub secrets_added: Vec<String>,
    pub secrets_removed: Vec<String>,
    /// None until the new script has been fetched (or for a plugin that isn't a script)
    pub script_changed: Option<bool>,
}

impl Changes {
    /// New addresses to reach or new secrets to hand over: such an update is confirmed on its
    /// own, never with Update all.
    pub fn needs_own_confirm(&self) -> bool {
        !self.hosts_added.is_empty() || !self.secrets_added.is_empty()
    }
}

pub fn changes(old: &Facts, new: &Facts) -> Changes {
    let not_in = |a: &[String], b: &[String]| a.iter().filter(|x| !b.contains(x)).cloned().collect::<Vec<_>>();
    let fields = |a: &[(String, String)], b: &[(String, String)]| {
        a.iter().filter(|(k, _)| !b.iter().any(|(k2, _)| k2 == k)).map(|(k, l)| if l.is_empty() { k.clone() } else { l.clone() }).collect::<Vec<_>>()
    };
    let script_changed = match (&old.script, &new.script) {
        (_, None) => None,
        (Some(a), Some(b)) => Some(a != b),
        (None, Some(_)) => Some(true),
    };
    Changes {
        kind: (old.kind != new.kind).then(|| (old.kind.clone(), new.kind.clone())),
        hosts_added: not_in(&new.hosts, &old.hosts),
        hosts_removed: not_in(&old.hosts, &new.hosts),
        settings_added: fields(&new.settings, &old.settings),
        settings_removed: fields(&old.settings, &new.settings),
        secrets_added: fields(&new.secrets, &old.secrets),
        secrets_removed: fields(&old.secrets, &new.secrets),
        script_changed,
    }
}

/// A plugin installed from the directory: the version recorded when it was installed and
/// what its manifest says now.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Installed {
    pub id: String,
    pub version: String,
    pub facts: Facts,
}

/// Versions the user chose to skip, by plugin id.
pub type Skips = std::collections::BTreeMap<String, String>;

/// Skipped until a newer version than the skipped one appears.
pub fn is_skipped(skips: &Skips, id: &str, version: &str) -> bool {
    skips.get(id).is_some_and(|s| !crate::update::is_newer(version, s))
}

pub fn read_skips(path: &Path) -> Skips {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn write_skips(path: &Path, skips: &Skips) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(path, serde_json::to_string_pretty(skips).unwrap_or_default()).map_err(|e| e.to_string())
}

/// An update the directory has for an installed plugin.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    pub id: String,
    pub name: String,
    pub from: String,
    pub to: String,
    /// The Rattleboard it needs, when that's newer than this one: shown, never offered
    pub needs_app: Option<String>,
    pub changes: Changes,
    pub readme: bool,
    pub folder: bool,
}

/// The directory's newer versions of `installed` plugins, left out when skipped. An update
/// that needs a newer Rattleboard is listed with `needs_app` set (see `offered`).
pub fn pending(index: &Index, installed: &[Installed], skips: &Skips, app: &str) -> Vec<Pending> {
    let mut out: Vec<Pending> = installed
        .iter()
        .filter_map(|i| {
            let e = index.plugins.iter().find(|e| e.id == i.id)?;
            if !update_available(Some(&i.version), &e.version) || is_skipped(skips, &e.id, &e.version) {
                return None;
            }
            Some(Pending {
                id: e.id.clone(),
                name: e.name.clone(),
                from: i.version.clone(),
                to: e.version.clone(),
                needs_app: needs_newer_app(e.min_app_version.as_deref(), app),
                changes: changes(&i.facts, &Facts::of_entry(e)),
                readme: e.readme_url.is_some(),
                folder: e.folder_url.is_some(),
            })
        })
        .collect();
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

/// How many of them can be installed now: the count the tray and Settings show.
pub fn offered(list: &[Pending]) -> usize {
    list.iter().filter(|p| p.needs_app.is_none()).count()
}

/// A plugin's README on GitHub, from its folder's address.
pub fn readme_page(folder_url: &str) -> Option<String> {
    let u = folder_url.trim_end_matches('/').replacen("/tree/", "/blob/", 1) + "/README.md";
    allowed(&u)
}

// ---------------------------------------------------------------- the cache

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Cache {
    /// When it was fetched, ms since the epoch
    pub fetched: i64,
    /// The address it came from, so a development base never mixes with the real one
    pub source: String,
    pub body: String,
}

/// Fresh for a day; a clock that went backwards makes it stale.
pub fn is_fresh(fetched: i64, now: i64) -> bool {
    fetched > 0 && now >= fetched && now - fetched < CACHE_MS
}

pub fn read_cache(path: &Path, source: &str) -> Option<Cache> {
    let c: Cache = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    (c.source == source).then_some(c)
}

pub fn write_cache(path: &Path, c: &Cache) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string(c) {
        let _ = std::fs::write(path, s);
    }
}

// ---------------------------------------------------------------- fetching

/// The plugins client's user agent and timeouts, but redirects only within the repository.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(plugins::FETCH_TIMEOUT)
        .connect_timeout(plugins::FETCH_TIMEOUT)
        .user_agent(plugins::USER_AGENT)
        .redirect(reqwest::redirect::Policy::custom(|a| {
            if a.previous().len() >= 5 {
                a.error("too many redirects")
            } else if !is_allowed(a.url().as_str()) {
                a.error("redirected out of the community repository")
            } else {
                a.follow()
            }
        }))
        .build()
        .expect("http client")
}

/// GETs a file from the repository, up to `max` bytes.
pub async fn get(c: &reqwest::Client, url: &str, base: Option<&str>, max: usize) -> Result<Vec<u8>, String> {
    let u = allowed(url).ok_or("That address isn't in the community plugins repository")?;
    let body = plugins::fetch(c, "GET", &resolve(&u, base), &[]).await?;
    if body.len() > max {
        return Err(format!("The file is larger than {} KB", max / 1024));
    }
    Ok(body)
}

// ---------------------------------------------------------------- preview cards

/// preview.json: {"rows", "cols", "characters"}, 1–12 × 1–40, codes 0–71.
pub fn parse_preview(bytes: &[u8]) -> Result<Grid, String> {
    let bad = || "The preview card isn't valid".to_string();
    let v: Value = serde_json::from_slice(bytes).map_err(|_| bad())?;
    let (rows, cols) = (v["rows"].as_u64().ok_or_else(bad)? as usize, v["cols"].as_u64().ok_or_else(bad)? as usize);
    if !(1..=12).contains(&rows) || !(1..=40).contains(&cols) {
        return Err(bad());
    }
    let chars = v["characters"].as_array().filter(|a| a.len() == rows).ok_or_else(bad)?;
    chars
        .iter()
        .map(|r| {
            let r = r.as_array().filter(|r| r.len() == cols).ok_or_else(bad)?;
            r.iter().map(|c| c.as_u64().filter(|c| *c <= 71).map(|c| c as u8).ok_or_else(bad)).collect()
        })
        .collect()
}

/// A preview card at another board size: each line of it centred, the colour tile in the
/// top-left corner kept there, lines that don't fit cut short and blank lines dropped first.
pub fn fit(g: &Grid, rows: usize, cols: usize) -> Grid {
    let (r0, c0) = (g.len(), g.first().map_or(0, |r| r.len()));
    if (r0, c0) == (rows, cols) {
        return g.clone();
    }
    let chip = g.first().and_then(|r| r.first()).copied().filter(|c| (63..=70).contains(c));
    let mut lines: Vec<Vec<u8>> = g
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let r: Vec<u8> = r.iter().enumerate().map(|(j, c)| if i == 0 && j == 0 && chip.is_some() { 0 } else { *c }).collect();
            match (r.iter().position(|c| *c != 0), r.iter().rposition(|c| *c != 0)) {
                (Some(a), Some(b)) => r[a..=b].to_vec(),
                _ => vec![],
            }
        })
        .collect();
    while lines.first().is_some_and(|l| l.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    while lines.len() > rows {
        match lines.iter().position(|l| l.is_empty()) {
            Some(k) => {
                lines.remove(k);
            }
            None => lines.truncate(rows),
        }
    }
    let mut out = vec![vec![0u8; cols]; rows];
    let top = (rows - lines.len()) / 2;
    for (i, l) in lines.iter().enumerate() {
        let l = &l[..l.len().min(cols)];
        let left = (cols - l.len()) / 2;
        out[top + i][left..left + l.len()].copy_from_slice(l);
    }
    if let Some(c) = chip {
        out[0][0] = c;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: &str) -> Value {
        json!({
            "id": id, "name": "Hacker News", "summary": "Top stories", "author": {"name": "Sam", "github": "slastrina", "extra": 1},
            "tags": ["news", "tech"], "version": "1.0.0", "license": "MIT", "homepage": "https://news.ycombinator.com",
            "kind": "rss", "needsSecrets": false, "settings": [{"key": "k", "label": "K", "type": "text", "future": true}],
            "secrets": [], "hosts": ["hnrss.org"],
            "manifestUrl": format!("{RAW_MAIN}plugins/{id}/plugin.rattleboard-plugin.json"),
            "readmeUrl": format!("{RAW_MAIN}plugins/{id}/README.md"), "previewUrl": null,
            "folderUrl": format!("{WEB_PREFIX}tree/main/plugins/{id}"), "updated": "2026-10-19",
            "somethingNew": {"nested": [1, 2]}
        })
    }

    #[test]
    fn index_with_unknown_fields() {
        let v = json!({"format": 1, "generated": "2026-10-19T00:00:00Z", "repo": REPO_URL, "later": true, "plugins": [entry("community.hacker-news")]});
        let ix = parse_index(&serde_json::to_vec(&v).unwrap()).unwrap();
        assert_eq!(ix.plugins.len(), 1);
        let e = &ix.plugins[0];
        assert_eq!((e.id.as_str(), e.version.as_str(), e.kind.as_str()), ("community.hacker-news", "1.0.0", "rss"));
        assert_eq!(e.author.github.as_deref(), Some("slastrina"));
        assert_eq!(e.settings[0].kind.as_deref(), Some("text"));
        assert!(e.readme_url.is_some() && e.preview_url.is_none() && e.folder_url.is_some());
        // The real index parses too, if the repository sits next to this one.
        if let Ok(b) = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../rattleboard-plugins/index-v2.json")) {
            assert!(!parse_index(&b).unwrap().plugins.is_empty());
        }
    }

    #[test]
    fn index_leaves_out_what_it_cant_trust() {
        let mut off = entry("community.off-site");
        off["manifestUrl"] = json!("https://example.com/plugin.json");
        let mut cmd = entry("community.cmd");
        cmd["kind"] = json!("command");
        let mut broken = entry("community.broken");
        broken["tags"] = json!("not a list");
        let mut links = entry("community.links");
        links["readmeUrl"] = json!("https://evil.example/README.md");
        links["homepage"] = json!("javascript:alert(1)");
        links["author"]["github"] = json!("../evil");
        let mut upper = entry("Community.Upper");
        upper["name"] = json!("X");
        let v = json!({"format": 1, "plugins": [off, cmd, broken, links, upper, entry("community.links"), "junk"]});
        let ix = parse_index(&serde_json::to_vec(&v).unwrap()).unwrap();
        assert_eq!(ix.plugins.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["community.links"]);
        let e = &ix.plugins[0];
        assert_eq!((e.readme_url.as_deref(), e.homepage.as_deref(), e.author.github.as_deref()), (None, None, None));

        assert!(parse_index(br#"{"format": 2, "plugins": []}"#).unwrap_err().contains("newer"));
        assert!(parse_index(b"{}").is_err());
        assert!(parse_index(b"not json").is_err());
        assert!(parse_index(&vec![b' '; MAX_INDEX + 1]).unwrap_err().contains("2 MB"));
    }

    #[test]
    fn url_allow_list() {
        for ok in [
            "https://raw.githubusercontent.com/slastrina/rattleboard-plugins/main/index.json",
            "https://github.com/slastrina/rattleboard-plugins/tree/main/plugins/community.hacker-news",
            "https://RAW.githubusercontent.com/slastrina/rattleboard-plugins/main/a/../index.json",
        ] {
            assert!(is_allowed(ok), "{ok}");
        }
        for bad in [
            "http://raw.githubusercontent.com/slastrina/rattleboard-plugins/main/index.json",
            "https://raw.githubusercontent.com/slastrina/rattleboard-plugins",
            "https://raw.githubusercontent.com/slastrina/rattleboard-plugins/",
            "https://raw.githubusercontent.com/slastrina/rattleboard-plugins-evil/main/index.json",
            "https://raw.githubusercontent.com/slastrina/rattleboard/main/README.md",
            "https://raw.githubusercontent.com/slastrina/rattleboard-plugins/../rattleboard/main/x",
            "https://raw.githubusercontent.com/slastrina/rattleboard-plugins/%2e%2e/other/x",
            "https://raw.githubusercontent.com:8443/slastrina/rattleboard-plugins/main/index.json",
            "https://user@raw.githubusercontent.com/slastrina/rattleboard-plugins/main/index.json",
            "https://raw.githubusercontent.com.evil.example/slastrina/rattleboard-plugins/main/index.json",
            "https://gist.github.com/slastrina/rattleboard-plugins/x",
            "https://github.com/slastrina/rattleboard",
            "file:///etc/passwd",
            "",
        ] {
            assert!(!is_allowed(bad), "{bad}");
        }
        assert_eq!(resolve(INDEX_URL, Some("http://127.0.0.1:8099/")), "http://127.0.0.1:8099/index-v2.json");
        assert_eq!(resolve(INDEX_URL, None), INDEX_URL);
        assert_eq!(resolve(REPO_URL, Some("http://127.0.0.1:8099")), REPO_URL);
        assert_eq!(profile_url("slastrina").as_deref(), Some("https://github.com/slastrina"));
        for bad in ["", "-x", "x-", "a/b", "../x", "a b", &"x".repeat(40)] {
            assert_eq!(profile_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn min_app_version() {
        let mut e = entry("community.new-thing");
        e["minAppVersion"] = json!("0.7.0");
        let ix = parse_index(&serde_json::to_vec(&json!({"format": 1, "plugins": [e, entry("community.old")]})).unwrap()).unwrap();
        assert_eq!(ix.plugins[0].min_app_version.as_deref(), Some("0.7.0"));
        assert_eq!(ix.plugins[1].min_app_version, None);
        assert_eq!(needs_newer_app(Some("0.7.0"), "0.6.0").as_deref(), Some("0.7.0"));
        assert_eq!(needs_newer_app(Some("0.7.0"), "0.7.0"), None);
        assert_eq!(needs_newer_app(Some("0.7.0"), "0.7.1"), None);
        assert_eq!(needs_newer_app(Some("0.7.0"), "0.7.0-beta.1").as_deref(), Some("0.7.0"));
        assert_eq!(needs_newer_app(Some("garbage"), "0.6.0"), None);
        assert_eq!(needs_newer_app(None, "0.6.0"), None);
    }

    #[test]
    fn version_compare() {
        assert!(update_available(Some("1.0.0"), "1.0.1"));
        assert!(update_available(Some("1.9.0"), "1.10.0"));
        assert!(!update_available(Some("1.0.0"), "1.0.0"));
        assert!(!update_available(Some("2.0.0"), "1.0.0"));
        assert!(!update_available(None, "1.0.0"));
        assert!(!update_available(Some("garbage"), "1.0.0"));
    }

    fn facts(kind: &str, hosts: &[&str], settings: &[&str], secrets: &[&str], script: Option<&str>) -> Facts {
        let f = |l: &[&str]| l.iter().map(|k| (k.to_string(), k.to_uppercase())).collect();
        Facts { kind: kind.into(), hosts: hosts.iter().map(|h| h.to_string()).collect(), settings: f(settings), secrets: f(secrets), script: script.map(String::from) }
    }

    #[test]
    fn change_summary() {
        let old = facts("script", &["a.example.com", "b.example.com"], &["city", "units"], &["key"], Some("one"));
        let same = changes(&old, &old.clone());
        assert_eq!(same, Changes { script_changed: Some(false), ..Default::default() });
        assert!(!same.needs_own_confirm());

        let new = facts("script", &["b.example.com", "c.example.com"], &["city", "lang"], &["key", "token"], Some("two"));
        let c = changes(&old, &new);
        assert_eq!(c.hosts_added, ["c.example.com"]);
        assert_eq!(c.hosts_removed, ["a.example.com"]);
        assert_eq!((c.settings_added.as_slice(), c.settings_removed.as_slice()), (&["LANG".to_string()][..], &["UNITS".to_string()][..]));
        assert_eq!((c.secrets_added.as_slice(), c.secrets_removed.len()), (&["TOKEN".to_string()][..], 0));
        assert_eq!((c.kind.clone(), c.script_changed), (None, Some(true)));
        assert!(c.needs_own_confirm());

        // Only removing things, or a new setting, can go with Update all.
        let fewer = facts("script", &["b.example.com"], &["city", "units", "lang"], &[], Some("one"));
        assert!(!changes(&old, &fewer).needs_own_confirm());
        assert!(changes(&old, &facts("script", &[], &[], &["key", "other"], None)).needs_own_confirm());

        // From the directory's entry the script isn't known; a kind change is reported.
        let rss = facts("rss", &["a.example.com", "b.example.com"], &["city", "units"], &["key"], None);
        let c = changes(&old, &rss);
        assert_eq!((c.kind, c.script_changed), (Some(("script".into(), "rss".into())), None));
        assert_eq!(changes(&rss, &old).script_changed, Some(true));
    }

    #[test]
    fn facts_of_manifests() {
        let m = plugins::parse(&json!({"format": 1, "id": "community.x", "name": "X", "kind": "rss", "fetch": {"url": "https://Feeds.Example.com/{{settings.q}}"},
            "settings": [{"key": "q", "label": "Query", "type": "text"}], "secrets": [{"key": "tok", "label": "Token"}]}), false).unwrap();
        let f = Facts::of_manifest(&m);
        assert_eq!((f.kind.as_str(), f.hosts.as_slice(), f.script.as_deref()), ("rss", &["feeds.example.com".to_string()][..], None));
        assert_eq!((f.settings[0].1.as_str(), f.secrets[0].0.as_str()), ("Query", "tok"));
        assert_eq!(url_host("https://{{settings.host}}/api").as_deref(), Some(SET_BY_YOU));
        assert_eq!(url_host("https://user@Api.Example.com:8443/x?y").as_deref(), Some("api.example.com"));
        // As the index says it, so an entry and the installed manifest compare equal.
        let e: Entry = serde_json::from_value(entry("community.hacker-news")).unwrap();
        assert_eq!(Facts::of_entry(&e).hosts, ["hnrss.org"]);
    }

    fn index_of(entries: Vec<Value>) -> Index {
        parse_index(&serde_json::to_vec(&json!({"format": 1, "plugins": entries})).unwrap()).unwrap()
    }

    #[test]
    fn which_updates_are_offered() {
        let mut news = entry("community.hacker-news");
        news["version"] = json!("1.1.0");
        news["hosts"] = json!(["hnrss.org", "news.example.com"]);
        let mut newer_app = entry("community.needs-app");
        newer_app["version"] = json!("2.0.0");
        newer_app["minAppVersion"] = json!("99.0.0");
        let mut same = entry("community.same");
        same["version"] = json!("1.0.0");
        let mut older = entry("community.older");
        older["version"] = json!("0.9.0");
        let ix = index_of(vec![news, newer_app, same, older, entry("community.not-installed")]);
        let base = Facts::of_entry(&serde_json::from_value(entry("x")).unwrap());
        let inst = |id: &str, v: &str| Installed { id: id.into(), version: v.into(), facts: base.clone() };
        let installed = [inst("community.hacker-news", "1.0.0"), inst("community.needs-app", "1.0.0"), inst("community.same", "1.0.0"), inst("community.older", "1.0.0"), inst("community.gone", "1.0.0")];

        let list = pending(&ix, &installed, &Skips::new(), "0.8.0");
        assert_eq!(list.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["community.hacker-news", "community.needs-app"]);
        let hn = &list[0];
        assert_eq!((hn.from.as_str(), hn.to.as_str(), hn.needs_app.as_deref()), ("1.0.0", "1.1.0", None));
        assert_eq!(hn.changes.hosts_added, ["news.example.com"]);
        assert!(hn.changes.needs_own_confirm() && hn.readme && hn.folder);
        assert_eq!(list[1].needs_app.as_deref(), Some("99.0.0"));
        assert_eq!(offered(&list), 1, "an update that needs a newer Rattleboard isn't offered");
        assert_eq!(offered(&pending(&ix, &installed, &Skips::new(), "99.0.0")), 2);

        // Skipping 1.1.0 hides it, until a newer one appears.
        let skips: Skips = [("community.hacker-news".to_string(), "1.1.0".to_string())].into();
        assert_eq!(offered(&pending(&ix, &installed, &skips, "0.8.0")), 0);
        let mut later = entry("community.hacker-news");
        later["version"] = json!("1.2.0");
        assert_eq!(offered(&pending(&index_of(vec![later]), &installed, &skips, "0.8.0")), 1);
        assert!(is_skipped(&skips, "community.hacker-news", "1.0.5"));
        assert!(!is_skipped(&skips, "community.other", "1.1.0"));
    }

    #[test]
    fn skips_persist() {
        let dir = std::env::temp_dir().join(format!("rb-skips-{}", std::process::id()));
        let path = dir.join("community-skipped.json");
        assert!(read_skips(&path).is_empty());
        let skips: Skips = [("community.dad-joke".to_string(), "1.1.0".to_string())].into();
        write_skips(&path, &skips).unwrap();
        assert_eq!(read_skips(&path), skips);
        std::fs::write(&path, "not json").unwrap();
        assert!(read_skips(&path).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn readme_pages() {
        assert_eq!(readme_page(&format!("{WEB_PREFIX}tree/main/plugins/community.x")).as_deref(), Some("https://github.com/slastrina/rattleboard-plugins/blob/main/plugins/community.x/README.md"));
        assert_eq!(readme_page("https://evil.example/tree/main/x"), None);
    }

    #[test]
    fn cache_freshness() {
        let now = 1_800_000_000_000;
        assert!(is_fresh(now - 1000, now));
        assert!(is_fresh(now - CACHE_MS + 1, now));
        assert!(!is_fresh(now - CACHE_MS, now));
        assert!(!is_fresh(0, now));
        assert!(!is_fresh(now + 60_000, now), "a clock that went backwards");

        let dir = std::env::temp_dir().join(format!("rb-community-{}", std::process::id()));
        let path = dir.join("community-index.json");
        let c = Cache { fetched: now, source: INDEX_URL.into(), body: "{}".into() };
        write_cache(&path, &c);
        assert_eq!(read_cache(&path, INDEX_URL), Some(c));
        assert_eq!(read_cache(&path, "http://127.0.0.1:8099/"), None, "another source's copy isn't used");
        let _ = std::fs::remove_dir_all(dir);
        assert_eq!(read_cache(&path, INDEX_URL), None);
    }

    #[test]
    fn preview_cards() {
        let mut g = vec![vec![0u8; 22]; 6];
        g[0][0] = 65;
        g[2][5..10].copy_from_slice(&[8, 5, 12, 12, 15]);
        let v = json!({"rows": 6, "cols": 22, "characters": g});
        let p = parse_preview(&serde_json::to_vec(&v).unwrap()).unwrap();
        assert_eq!(p, g);
        assert_eq!(fit(&p, 6, 22), g);
        let small = fit(&p, 3, 15);
        assert_eq!((small.len(), small[0].len(), small[0][0]), (3, 15, 65));
        assert_eq!(small[1][5..10], [8, 5, 12, 12, 15]);
        let tiny = fit(&p, 1, 3);
        assert_eq!(tiny, vec![vec![65, 5, 12]]);
        for bad in [json!({"rows": 2, "cols": 1, "characters": [[1]]}), json!({"rows": 1, "cols": 1, "characters": [[72]]}), json!({"rows": 13, "cols": 1})] {
            assert!(parse_preview(&serde_json::to_vec(&bad).unwrap()).is_err());
        }
    }
}
