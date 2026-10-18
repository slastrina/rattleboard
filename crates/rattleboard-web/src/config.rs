//! Rattleboard Web's settings, all from the environment (see the README's Web section).

use rattleboard_core::settings::{CUSTOM_COLS, CUSTOM_ROWS, DEFAULT_SERVER_PORT, PRESETS};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub enum Viewers {
    /// Anyone who can reach the port can watch the board
    Open,
    /// The board page needs the viewer token in its URL fragment (`/#v=<token>`)
    Token,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub data: PathBuf,
    pub name: String,
    /// The board's size, or for an adaptive board the size text is laid out at
    pub rows: usize,
    pub cols: usize,
    pub adaptive: bool,
    /// "black" | "white"
    pub finish: String,
    pub viewers: Viewers,
    /// RATTLEBOARD_VIEWER_TOKEN, if set; otherwise one is generated into the data folder
    pub viewer_token: Option<String>,
    /// RATTLEBOARD_KEY, if set; otherwise one is generated into the data folder
    pub key: Option<String>,
    /// The address phones and other boards use, for the pairing QR code
    pub public_url: Option<String>,
    pub mdns: bool,
}

fn flag(v: &str) -> Result<bool, String> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" | "" => Ok(false),
        _ => Err(format!("expected true or false, not \"{v}\"")),
    }
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        Config::from(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()))
    }

    /// Reads each setting with `get` (the environment, or a map in tests).
    pub fn from(get: impl Fn(&str) -> Option<String>) -> Result<Config, String> {
        let bad = |k: &str, e: String| format!("{k}: {e}");
        let port = match get("RATTLEBOARD_PORT") {
            None => DEFAULT_SERVER_PORT,
            Some(p) => p.trim().parse::<u16>().ok().filter(|p| *p > 0).ok_or_else(|| bad("RATTLEBOARD_PORT", format!("not a port: \"{p}\"")))?,
        };
        let num = |k: &str, (lo, hi): (usize, usize)| -> Result<Option<usize>, String> {
            match get(k) {
                None => Ok(None),
                Some(v) => match v.trim().parse::<usize>() {
                    Ok(n) if (lo..=hi).contains(&n) => Ok(Some(n)),
                    _ => Err(bad(k, format!("must be from {lo} to {hi}, not \"{v}\""))),
                },
            }
        };
        let (rows, cols) = (num("RATTLEBOARD_ROWS", CUSTOM_ROWS)?, num("RATTLEBOARD_COLS", CUSTOM_COLS)?);
        let size = get("RATTLEBOARD_SIZE").map(|s| s.trim().to_ascii_lowercase());
        let (rows, cols) = match size.as_deref() {
            // A custom size given with no size name is a custom size.
            None if rows.is_some() || cols.is_some() => (rows.unwrap_or(6), cols.unwrap_or(22)),
            None => (6, 22),
            Some("custom") => match (rows, cols) {
                (Some(r), Some(c)) => (r, c),
                _ => return Err("RATTLEBOARD_SIZE=custom needs RATTLEBOARD_ROWS (2–12) and RATTLEBOARD_COLS (8–40)".into()),
            },
            Some(name) => match PRESETS.iter().find(|p| p.0 == name) {
                Some(p) => (p.1, p.2),
                None => return Err(bad("RATTLEBOARD_SIZE", format!("use standard, compact, large or custom, not \"{name}\""))),
            },
        };
        let finish = match get("RATTLEBOARD_FINISH").map(|f| f.trim().to_ascii_lowercase()).as_deref() {
            None | Some("black") => "black".to_string(),
            Some("white") => "white".to_string(),
            Some(f) => return Err(bad("RATTLEBOARD_FINISH", format!("use black or white, not \"{f}\""))),
        };
        let viewer_token = get("RATTLEBOARD_VIEWER_TOKEN").map(|t| t.trim().to_string());
        if viewer_token.as_ref().is_some_and(|t| t.len() < 16) {
            return Err("RATTLEBOARD_VIEWER_TOKEN must be at least 16 characters".into());
        }
        let viewers = match get("RATTLEBOARD_VIEWERS").map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            // A token given is a token wanted.
            None if viewer_token.is_some() => Viewers::Token,
            None | Some("open") => Viewers::Open,
            Some("token") => Viewers::Token,
            Some(v) => return Err(bad("RATTLEBOARD_VIEWERS", format!("use open or token, not \"{v}\""))),
        };
        let key = get("RATTLEBOARD_KEY").map(|k| k.trim().to_string());
        if key.as_ref().is_some_and(|k| k.len() < 16) {
            return Err("RATTLEBOARD_KEY must be at least 16 characters".into());
        }
        let public_url = match get("RATTLEBOARD_URL").map(|u| u.trim().trim_end_matches('/').to_string()) {
            Some(u) if !(u.starts_with("http://") || u.starts_with("https://")) => {
                return Err(bad("RATTLEBOARD_URL", format!("must start with http:// or https://, not \"{u}\"")))
            }
            u => u,
        };
        Ok(Config {
            port,
            data: PathBuf::from(get("RATTLEBOARD_DATA").unwrap_or_else(|| "/data".into())),
            name: get("RATTLEBOARD_NAME").map(|n| n.trim().to_string()).unwrap_or_else(|| "Rattleboard Web".into()),
            rows,
            cols,
            adaptive: get("RATTLEBOARD_ADAPTIVE").map(|v| flag(&v)).transpose().map_err(|e| bad("RATTLEBOARD_ADAPTIVE", e))?.unwrap_or(false),
            finish,
            viewers,
            viewer_token,
            key,
            public_url,
            mdns: get("RATTLEBOARD_MDNS").map(|v| flag(&v)).transpose().map_err(|e| bad("RATTLEBOARD_MDNS", e))?.unwrap_or(false),
        })
    }

    pub fn dims(&self) -> (usize, usize) {
        (self.rows, self.cols)
    }

    /// The size's name, as the apps show it
    pub fn size_name(&self) -> String {
        let name = PRESETS.iter().find(|p| (p.1, p.2) == (self.rows, self.cols)).map(|p| p.0).unwrap_or("custom");
        let mut n = name.to_string();
        n[..1].make_ascii_uppercase();
        format!("{n} {}×{}", self.rows, self.cols)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg(vars: &[(&str, &str)]) -> Result<Config, String> {
        let m: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Config::from(|k| m.get(k).cloned())
    }

    #[test]
    fn defaults() {
        let c = cfg(&[]).unwrap();
        assert_eq!((c.port, c.dims(), c.adaptive, c.mdns), (7070, (6, 22), false, false));
        assert_eq!(c.viewers, Viewers::Open);
        assert_eq!(c.data, PathBuf::from("/data"));
        assert_eq!(c.size_name(), "Standard 6×22");
    }

    #[test]
    fn sizes() {
        assert_eq!(cfg(&[("RATTLEBOARD_SIZE", "Compact")]).unwrap().dims(), (3, 15));
        assert_eq!(cfg(&[("RATTLEBOARD_SIZE", "large")]).unwrap().dims(), (8, 30));
        assert_eq!(cfg(&[("RATTLEBOARD_SIZE", "custom"), ("RATTLEBOARD_ROWS", "4"), ("RATTLEBOARD_COLS", "40")]).unwrap().dims(), (4, 40));
        assert_eq!(cfg(&[("RATTLEBOARD_ROWS", "12")]).unwrap().dims(), (12, 22));
        assert!(cfg(&[("RATTLEBOARD_SIZE", "custom"), ("RATTLEBOARD_ROWS", "4")]).is_err());
        assert!(cfg(&[("RATTLEBOARD_ROWS", "1")]).is_err());
        assert!(cfg(&[("RATTLEBOARD_COLS", "41")]).is_err());
        assert!(cfg(&[("RATTLEBOARD_SIZE", "flagship")]).is_err());
        assert_eq!(cfg(&[("RATTLEBOARD_SIZE", "custom"), ("RATTLEBOARD_ROWS", "5"), ("RATTLEBOARD_COLS", "20")]).unwrap().size_name(), "Custom 5×20");
    }

    #[test]
    fn access() {
        assert_eq!(cfg(&[("RATTLEBOARD_VIEWERS", "token")]).unwrap().viewers, Viewers::Token);
        let token = "t".repeat(16);
        let c = cfg(&[("RATTLEBOARD_VIEWER_TOKEN", &token)]).unwrap();
        assert_eq!((c.viewers, c.viewer_token), (Viewers::Token, Some(token)));
        assert!(cfg(&[("RATTLEBOARD_VIEWER_TOKEN", "short")]).is_err());
        assert!(cfg(&[("RATTLEBOARD_KEY", "short")]).is_err());
        assert!(cfg(&[("RATTLEBOARD_VIEWERS", "lan")]).is_err());
        assert!(cfg(&[("RATTLEBOARD_URL", "tv.local:7070")]).is_err());
        assert_eq!(cfg(&[("RATTLEBOARD_URL", "http://tv.local:7070/")]).unwrap().public_url.as_deref(), Some("http://tv.local:7070"));
        assert!(cfg(&[("RATTLEBOARD_PORT", "0")]).is_err());
        assert!(cfg(&[("RATTLEBOARD_MDNS", "maybe")]).is_err());
        assert!(cfg(&[("RATTLEBOARD_MDNS", "1"), ("RATTLEBOARD_ADAPTIVE", "true")]).map(|c| c.mdns && c.adaptive).unwrap());
    }
}
