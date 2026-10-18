//! Update check: asks GitHub for the latest Rattleboard release and says whether it's newer
//! than this one. It only ever suggests: nothing is downloaded or installed.

use serde::Deserialize;
use std::cmp::Ordering;
use std::time::Duration;

const LATEST: &str = "https://api.github.com/repos/slastrina/rattleboard/releases/latest";
/// Where to send people when the release has no page of its own.
pub const RELEASES: &str = "https://github.com/slastrina/rattleboard/releases/latest";
/// Automatic checks: once a day, and never again within this long of the last one, even
/// across restarts.
pub const EVERY_MS: i64 = 24 * 3600 * 1000;
pub const MIN_GAP_MS: i64 = 12 * 3600 * 1000;

/// A release version: major.minor.patch, plus a pre-release tag that sorts before the
/// release itself (0.5.0-beta.1 < 0.5.0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub nums: [u64; 3],
    pub pre: Option<String>,
}

impl Version {
    /// "v0.4.2", "0.4.2", "0.5", "1.0.0-rc.1" or "1.2.3+build"; None for anything else.
    pub fn parse(tag: &str) -> Option<Self> {
        let t = tag.trim();
        let t = t.strip_prefix(['v', 'V']).unwrap_or(t);
        let t = t.split('+').next()?;
        let (core, pre) = match t.split_once('-') {
            Some((c, p)) if !p.is_empty() => (c, Some(p.to_string())),
            Some(_) => return None,
            None => (t, None),
        };
        let parts: Vec<&str> = core.split('.').collect();
        if parts.is_empty() || parts.len() > 3 {
            return None;
        }
        let mut nums = [0u64; 3];
        for (i, p) in parts.iter().enumerate() {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            nums[i] = p.parse().ok()?;
        }
        Some(Self { nums, pre })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b, c] = self.nums;
        write!(f, "{a}.{b}.{c}")?;
        if let Some(p) = &self.pre {
            write!(f, "-{p}")?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.nums.cmp(&other.nums).then_with(|| match (&self.pre, &other.pre) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => cmp_pre(a, b),
        })
    }
}

/// Semver pre-release order: dot-separated, numbers numerically and below words.
fn cmp_pre(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.split('.'), b.split('.'));
    loop {
        match (x.next(), y.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) => {
                let o = match (p.parse::<u64>(), q.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    _ => p.cmp(q),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
        }
    }
}

/// Whether `latest` (a release tag or version) is newer than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (Version::parse(latest), Version::parse(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// Whether an automatic check is due, given when the last one was (ms since the epoch, 0 for
/// never): at launch when the last is 12 hours old, after that when it's a day old. A clock
/// that went backwards counts as due.
pub fn should_check(last_ms: i64, now_ms: i64, at_launch: bool) -> bool {
    let gap = if at_launch { MIN_GAP_MS } else { EVERY_MS };
    last_ms <= 0 || now_ms < last_ms || now_ms - last_ms >= gap
}

/// The only pages an update link may open: https on github.com, under slastrina/rattleboard.
pub fn is_release_url(u: &str) -> bool {
    let Ok(url) = url::Url::parse(u) else { return false };
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && (url.path() == "/slastrina/rattleboard" || url.path().starts_with("/slastrina/rattleboard/"))
}

/// The newest published release.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// Without the leading "v"
    pub version: String,
    /// The release's page, or the releases page when GitHub's link isn't one we'd open
    pub url: String,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// Reads GitHub's `releases/latest` answer.
pub fn parse_release(body: &[u8]) -> Result<Release, String> {
    let r: GhRelease = serde_json::from_slice(body).map_err(|_| "GitHub's answer wasn't understood".to_string())?;
    if r.draft || r.prerelease {
        return Err("No published release".into());
    }
    let v = Version::parse(&r.tag_name).ok_or_else(|| format!("Unrecognised release tag {:?}", r.tag_name))?;
    let url = if is_release_url(&r.html_url) { r.html_url } else { RELEASES.into() };
    Ok(Release { version: v.to_string(), url })
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(10))
        .user_agent(concat!("Rattleboard/", env!("CARGO_PKG_VERSION"), " (+https://www.iokig.com/rattleboard)"))
        .build()
        .expect("http client")
}

/// Asks GitHub for the latest release.
pub async fn latest(c: &reqwest::Client) -> Result<Release, String> {
    let res = c
        .get(LATEST)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| if e.is_timeout() { "Timed out after 10 s".to_string() } else if e.is_connect() { "No connection to GitHub".into() } else { "Network error".into() })?;
    match res.status().as_u16() {
        200 => {}
        404 => return Err("No published release".into()),
        403 | 429 => return Err("GitHub is rate-limiting requests; try again later".into()),
        s => return Err(format!("GitHub answered HTTP {s}")),
    }
    let body = res.bytes().await.map_err(|_| "Network error".to_string())?;
    parse_release(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        assert_eq!(Version::parse("v0.4.2").unwrap().nums, [0, 4, 2]);
        assert_eq!(Version::parse("0.4.2").unwrap().to_string(), "0.4.2");
        assert_eq!(Version::parse(" V1.2 ").unwrap().to_string(), "1.2.0");
        assert_eq!(Version::parse("1.0.0-rc.1+abc").unwrap().to_string(), "1.0.0-rc.1");
        for bad in ["", "v", "latest", "1.2.3.4", "1..2", "1.x", "1.2.3-", "-1.2"] {
            assert!(Version::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn ordering() {
        assert!(is_newer("v0.10.0", "0.9.0"));
        assert!(is_newer("0.4.2", "0.4.1"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("v0.4.1", "0.4.1"));
        assert!(!is_newer("0.4.0", "0.4.1"));
        assert!(!is_newer("0.9.0", "0.10.0"));
        assert!(is_newer("0.5.0", "0.5.0-beta.2"));
        assert!(!is_newer("0.5.0-beta.2", "0.5.0"));
        assert!(is_newer("0.5.0-beta.10", "0.5.0-beta.2"));
        assert!(is_newer("0.5.0-beta", "0.5.0-alpha.9"));
        assert!(!is_newer("garbage", "0.4.1"));
        assert!(!is_newer("0.4.2", "garbage"));
    }

    #[test]
    fn when_to_check() {
        let h = 3600 * 1000;
        let now = 1_000_000 * h;
        assert!(should_check(0, now, true));
        assert!(should_check(0, now, false));
        assert!(!should_check(now - h, now, true));
        assert!(!should_check(now - 11 * h, now, true));
        assert!(should_check(now - 12 * h, now, true));
        assert!(!should_check(now - 12 * h, now, false));
        assert!(!should_check(now - 23 * h, now, false));
        assert!(should_check(now - 24 * h, now, false));
        assert!(should_check(now - 30 * h, now, true));
        assert!(should_check(now + h, now, false), "clock went backwards");
    }

    #[test]
    fn release_urls() {
        assert!(is_release_url("https://github.com/slastrina/rattleboard/releases/tag/v0.4.2"));
        assert!(is_release_url(RELEASES));
        for bad in [
            "http://github.com/slastrina/rattleboard/releases",
            "https://github.com/slastrina/rattleboard-evil/releases",
            "https://github.com/other/rattleboard/releases",
            "https://github.com.evil.com/slastrina/rattleboard",
            "https://user@github.com/slastrina/rattleboard",
            "https://github.com:8443/slastrina/rattleboard",
            "javascript:alert(1)",
            "file:///slastrina/rattleboard",
            "",
        ] {
            assert!(!is_release_url(bad), "{bad}");
        }
    }

    /// Asks the real GitHub API: `cargo test -p rattleboard-core -- --ignored live_latest`
    #[tokio::test]
    #[ignore]
    async fn live_latest() {
        let r = latest(&client()).await.unwrap();
        println!("latest release: {} at {}", r.version, r.url);
        assert!(Version::parse(&r.version).is_some() && is_release_url(&r.url));
    }

    #[test]
    fn release_json() {
        let r = parse_release(br#"{"tag_name":"v0.4.2","html_url":"https://github.com/slastrina/rattleboard/releases/tag/v0.4.2","draft":false,"prerelease":false}"#).unwrap();
        assert_eq!(r, Release { version: "0.4.2".into(), url: "https://github.com/slastrina/rattleboard/releases/tag/v0.4.2".into() });
        let r = parse_release(br#"{"tag_name":"0.5.0","html_url":"https://evil.example/x"}"#).unwrap();
        assert_eq!(r.url, RELEASES);
        assert!(parse_release(br#"{"tag_name":"v0.5.0","prerelease":true}"#).is_err());
        assert!(parse_release(br#"{"tag_name":"v0.5.0","draft":true}"#).is_err());
        assert!(parse_release(br#"{"tag_name":"nightly"}"#).is_err());
        assert!(parse_release(b"not json").is_err());
    }
}
