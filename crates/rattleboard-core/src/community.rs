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
pub const INDEX_URL: &str = "https://raw.githubusercontent.com/slastrina/rattleboard-plugins/main/index.json";
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

/// Whether the index has a newer version than the one installed (None: not installed, or
/// installed with no version known).
pub fn update_available(installed: Option<&str>, index: &str) -> bool {
    installed.is_some_and(|v| crate::update::is_newer(index, v))
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
        if let Ok(b) = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../rattleboard-plugins/index.json")) {
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
        assert_eq!(resolve(INDEX_URL, Some("http://127.0.0.1:8099/")), "http://127.0.0.1:8099/index.json");
        assert_eq!(resolve(INDEX_URL, None), INDEX_URL);
        assert_eq!(resolve(REPO_URL, Some("http://127.0.0.1:8099")), REPO_URL);
        assert_eq!(profile_url("slastrina").as_deref(), Some("https://github.com/slastrina"));
        for bad in ["", "-x", "x-", "a/b", "../x", "a b", &"x".repeat(40)] {
            assert_eq!(profile_url(bad), None, "{bad}");
        }
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
