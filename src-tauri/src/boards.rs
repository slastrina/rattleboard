//! Saved boards: other Rattleboards and real Vestaboards this computer can send to.
//! The list (names, addresses, sizes) lives in boards.json; each board's key lives only in
//! the OS credential store, under its own account (see `secrets::board_account`).
//!
//! A board is one of:
//!   "rattleboard"  another Rattleboard (a TV, another desktop); plays sequences itself
//!   "local"        a real Vestaboard's Local API, or anything that doesn't answer
//!                  GET /rattleboard/info; this computer plays sequences for it
//!   "cloud"        a real Vestaboard through cloud.vestaboard.com

use crate::vestaboard::{self, dims_of, MAX_COLS, MAX_ROWS};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::time::{Duration, Instant};

pub const RATTLEBOARD_PORT: u16 = crate::settings::DEFAULT_SERVER_PORT;
const SERVICE_TYPE: &str = "_http._tcp.local.";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedBoard {
    pub id: String,
    pub name: String,
    /// "rattleboard" | "local" | "cloud"
    pub kind: String,
    /// host:port for "rattleboard" and "local"; empty for "cloud"
    pub host: String,
    pub rows: usize,
    pub cols: usize,
    /// Shows each message at its own size (a Rattleboard desktop's "Match each message")
    #[serde(default)]
    pub adaptive: bool,
}

impl SavedBoard {
    /// Seconds a real board needs between messages; it rejects or drops faster ones.
    pub fn min_hold(&self) -> f64 {
        match self.kind.as_str() {
            "cloud" => 15.0,
            "local" => 5.0,
            _ => 1.0,
        }
    }
}

pub fn load(path: &Path) -> Vec<SavedBoard> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, boards: &[SavedBoard]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string_pretty(boards) {
        let _ = std::fs::write(path, s);
    }
}

pub fn new_id() -> String {
    let mut b = [0u8; 6];
    getrandom::fill(&mut b).expect("OS random source");
    format!("b-{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>())
}

// ---------------------------------------------------------------- detecting a board

/// What answered at an address.
#[derive(Debug, PartialEq)]
pub struct Probe {
    pub kind: String,
    pub adaptive: bool,
    pub host: String,
    pub name: Option<String>,
    pub rows: usize,
    pub cols: usize,
}

/// Finds out what's at `host`: a Rattleboard answers GET /rattleboard/info; anything else
/// that serves the Local API with this key is treated as a real Vestaboard. Without a port,
/// tries Rattleboard's 7070 and then a real board's 7000.
pub async fn probe(c: &reqwest::Client, host: &str, key: &str) -> Result<Probe, String> {
    if host.trim().is_empty() {
        return Err("Enter the board's address".into());
    }
    if key.is_empty() {
        return Err("Enter the board's key".into());
    }
    let candidates = if vestaboard::has_port(host) {
        vec![vestaboard::base_url(host, 0)]
    } else {
        vec![vestaboard::base_url(host, RATTLEBOARD_PORT), vestaboard::base_url(host, 7000)]
    };
    let mut last = "Can't reach the board".to_string();
    for base in candidates {
        let addr = base.trim_start_matches("http://").to_string();
        let res = c
            .get(format!("{base}/rattleboard/info"))
            .header("X-Vestaboard-Local-Api-Key", key)
            .timeout(Duration::from_secs(4))
            .send()
            .await;
        let res = match res {
            Ok(r) => r,
            Err(e) => {
                last = vestaboard::describe(e);
                continue;
            }
        };
        if res.status().is_success() {
            if let Ok(v) = res.json::<Value>().await {
                if v["app"] == "rattleboard" {
                    let rows = (v["board"]["rows"].as_u64().unwrap_or(6) as usize).clamp(1, MAX_ROWS);
                    let cols = (v["board"]["cols"].as_u64().unwrap_or(22) as usize).clamp(1, MAX_COLS);
                    let name = v["name"].as_str().map(str::to_string);
                    let adaptive = v["adaptive"].as_bool().unwrap_or(false);
                    return Ok(Probe { kind: "rattleboard".into(), adaptive, host: addr, name, rows, cols });
                }
            }
        }
        // A real board (or an older Rattleboard) answers 404 or 401 there; its Local API
        // tells us whether the key works and how big it is.
        match vestaboard::read_local(c, &addr, key).await {
            Ok(reading) => {
                let (rows, cols) = dims_of(&reading.layout);
                return Ok(Probe { kind: "local".into(), adaptive: false, host: addr, name: None, rows, cols });
            }
            Err(e) if e == "Key was rejected" => return Err(e),
            Err(e) => last = e,
        }
    }
    Err(last)
}

// ---------------------------------------------------------------- pair links

/// rattleboard://pair?v=2&url=http%3A%2F%2F192.168.1.20%3A7070&key=...&name=...&rows=6&cols=22
pub fn pair_url(base: &str, key: &str, name: &str, rows: usize, cols: usize) -> String {
    format!("rattleboard://pair?v=2&url={}&key={}&name={}&rows={rows}&cols={cols}", enc(base), enc(key), enc(name))
}

/// Percent-encodes everything but the URL-safe characters (spaces become %20).
fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Debug, PartialEq)]
pub struct PairLink {
    pub host: String,
    pub key: String,
    pub name: String,
    pub rows: usize,
    pub cols: usize,
}

pub fn parse_pair(link: &str) -> Result<PairLink, String> {
    let u = url::Url::parse(link.trim()).map_err(|_| "That isn't a pair link")?;
    if u.scheme() != "rattleboard" || u.host_str() != Some("pair") {
        return Err("Pair links start with rattleboard://pair".into());
    }
    let q = |k: &str| u.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned());
    if q("v").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0) < 2 {
        return Err("This pair link is from an older Rattleboard".into());
    }
    let target = q("url").ok_or("The pair link has no address")?;
    let target = url::Url::parse(&target).map_err(|_| "The pair link's address is unreadable")?;
    let host = match (target.host_str(), target.port_or_known_default()) {
        (Some(h), Some(p)) if target.scheme() == "http" => format!("{h}:{p}"),
        _ => return Err("The pair link's address must be http://host:port".into()),
    };
    let key = q("key").filter(|k| !k.is_empty()).ok_or("The pair link has no key")?;
    let rows = q("rows").and_then(|v| v.parse::<usize>().ok()).unwrap_or(6).clamp(1, MAX_ROWS);
    let cols = q("cols").and_then(|v| v.parse::<usize>().ok()).unwrap_or(22).clamp(1, MAX_COLS);
    let name = q("name").map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Rattleboard".into());
    Ok(PairLink { host, key, name, rows, cols })
}

// ---------------------------------------------------------------- mDNS

#[derive(Serialize, Clone, Debug)]
pub struct Found {
    pub name: String,
    pub host: String,
}

/// Rattleboards on the network: `_http._tcp` services whose name starts with "Rattleboard".
/// Blocks for `wait`; call it off the main thread. `skip` leaves out this computer's own server.
pub fn discover(wait: Duration, skip: Option<(String, u16)>) -> Vec<Found> {
    let Ok(mdns) = mdns_sd::ServiceDaemon::new() else { return vec![] };
    let Ok(rx) = mdns.browse(SERVICE_TYPE) else { return vec![] };
    // One entry per service: it may be resolved once per network interface, so keep the
    // address most likely to work from here (the home network over VPNs and loopback).
    let mut out: Vec<(Found, u8)> = vec![];
    let deadline = Instant::now() + wait;
    while let Ok(ev) = rx.recv_deadline(deadline) {
        let mdns_sd::ServiceEvent::ServiceResolved(s) = ev else { continue };
        let name = s.fullname.strip_suffix(&format!(".{SERVICE_TYPE}")).unwrap_or(&s.fullname).replace("\\.", ".");
        if !name.starts_with("Rattleboard") {
            continue;
        }
        let ips: Vec<std::net::Ipv4Addr> =
            s.addresses.iter().filter_map(|a| match a.to_ip_addr() { std::net::IpAddr::V4(v) => Some(v), _ => None }).collect();
        if skip.as_ref().is_some_and(|(me, port)| *port == s.port && ips.iter().any(|a| a.to_string() == *me)) {
            continue;
        }
        let Some(ip) = ips.iter().min_by_key(|a| address_rank(a)) else { continue };
        let (found, rank) = (Found { name: name.clone(), host: format!("{ip}:{}", s.port) }, address_rank(ip));
        match out.iter_mut().find(|(f, _)| f.name == name && f.host.ends_with(&format!(":{}", s.port))) {
            Some(e) if rank < e.1 => *e = (found, rank),
            Some(_) => {}
            None => out.push((found, rank)),
        }
    }
    let out: Vec<Found> = out.into_iter().map(|(f, _)| f).collect();
    let _ = mdns.stop_browse(SERVICE_TYPE);
    let _ = mdns.shutdown();
    out
}

/// Lower is better: a home network address, then other private ones, then anything else.
fn address_rank(a: &std::net::Ipv4Addr) -> u8 {
    let o = a.octets();
    if a.is_loopback() || a.is_link_local() {
        3
    } else if o[0] == 192 && o[1] == 168 {
        0
    } else if a.is_private() {
        1
    } else {
        2
    }
}

/// Advertises this computer's API server so other Rattleboards can find it. Dropping it
/// withdraws the advertisement.
pub struct Advert {
    mdns: mdns_sd::ServiceDaemon,
    fullname: String,
}

impl Drop for Advert {
    fn drop(&mut self) {
        let _ = self.mdns.unregister(&self.fullname);
        let _ = self.mdns.shutdown();
    }
}

pub fn advertise(name: &str, port: u16) -> Option<Advert> {
    let mdns = mdns_sd::ServiceDaemon::new().ok()?;
    let instance = format!("Rattleboard ({name})");
    let host = format!("rattleboard-{}.local.", &new_id()[2..]);
    let props = [("app", "rattleboard"), ("protocol", "2")];
    let info = mdns_sd::ServiceInfo::new(SERVICE_TYPE, &instance, &host, (), port, &props[..]).ok()?.enable_addr_auto();
    let fullname = info.get_fullname().to_string();
    mdns.register(info).ok()?;
    Some(Advert { mdns, fullname })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_links() {
        let url = pair_url("http://192.168.1.20:7070", "k&1 2", "Living room TV", 6, 22);
        assert_eq!(
            url,
            "rattleboard://pair?v=2&url=http%3A%2F%2F192.168.1.20%3A7070&key=k%261%202&name=Living%20room%20TV&rows=6&cols=22"
        );
        let p = parse_pair(&url).unwrap();
        assert_eq!(p, PairLink { host: "192.168.1.20:7070".into(), key: "k&1 2".into(), name: "Living room TV".into(), rows: 6, cols: 22 });
        let p = parse_pair("rattleboard://pair?v=2&url=http%3A%2F%2Ftv.local&key=abc&rows=3&cols=15").unwrap();
        assert_eq!((p.host.as_str(), p.name.as_str(), p.rows), ("tv.local:80", "Rattleboard", 3));
        assert!(parse_pair("https://example.com").is_err());
        assert!(parse_pair("rattleboard://pair?v=1&url=http%3A%2F%2Fa%3A1&key=k").is_err());
        assert!(parse_pair("rattleboard://pair?v=2&url=http%3A%2F%2Fa%3A1").is_err());
        assert!(parse_pair("rattleboard://pair?v=2&url=ftp%3A%2F%2Fa&key=k").is_err());
    }

    #[test]
    fn addresses() {
        let r = |s: &str| address_rank(&s.parse().unwrap());
        assert!(r("192.168.1.20") < r("10.211.55.2") && r("10.211.55.2") < r("127.0.0.1"));
    }

    #[test]
    fn holds() {
        let b = |kind: &str| SavedBoard { id: "b".into(), name: "n".into(), kind: kind.into(), host: String::new(), rows: 6, cols: 22, adaptive: false };
        assert_eq!((b("cloud").min_hold(), b("local").min_hold(), b("rattleboard").min_hold()), (15.0, 5.0, 1.0));
        assert!(new_id().starts_with("b-") && new_id().len() == 14);
    }

    /// A Rattleboard answers /rattleboard/info; a real board's Local API doesn't.
    #[tokio::test]
    async fn probing() {
        use crate::sequence::Sequence;
        use crate::server::{self, SequenceStatus};
        use crate::vestaboard::{Grid, Transition};
        struct Tv;
        impl server::Board for Tv {
            fn api_key(&self) -> String {
                "tv-key".into()
            }
            fn dims(&self) -> (usize, usize) {
                (3, 15)
            }
            fn adaptive(&self) -> bool {
                false
            }
            fn name(&self) -> String {
                "Rattleboard (TV)".into()
            }
            fn current(&self) -> (Grid, Option<String>) {
                (vec![vec![0; 15]; 3], None)
            }
            fn post(&self, _: Grid, _: Transition) -> String {
                "m".into()
            }
            fn play(&self, _: Sequence) -> String {
                "seq".into()
            }
            fn sequence(&self) -> Option<SequenceStatus> {
                None
            }
            fn stop(&self) {}
        }
        let tv = server::start(std::sync::Arc::new(Tv), 0, false).await.unwrap();
        let c = vestaboard::client();
        let host = tv.addr.to_string();
        let p = probe(&c, &host, "tv-key").await.unwrap();
        assert_eq!(p, Probe { kind: "rattleboard".into(), adaptive: false, host: host.clone(), name: Some("Rattleboard (TV)".into()), rows: 3, cols: 15 });
        assert_eq!(probe(&c, &host, "wrong").await.unwrap_err(), "Key was rejected");

        // Only the Local API: like a real board, or a Rattleboard from before protocol v2.
        let app = axum::Router::new().route(
            "/local-api/message",
            axum::routing::get(|| async { axum::Json(vec![vec![0u8; 22]; 6]) }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = l.local_addr().unwrap().to_string();
        tokio::spawn(async move { axum::serve(l, app).await });
        let p = probe(&c, &host, "any").await.unwrap();
        assert_eq!((p.kind.as_str(), p.rows, p.cols), ("local", 6, 22));
        assert!(probe(&c, "", "k").await.is_err());
        drop(tv);
    }
}
