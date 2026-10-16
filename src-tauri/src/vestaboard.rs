//! Reading the current message from a Vestaboard.
//!
//! Local API: GET http://{host}:7000/local-api/message
//!            X-Vestaboard-Local-Api-Key: {key}
//!            -> [[..22 codes..] x 6]   (older firmware wraps it: {"message": [...]})
//! Cloud API: GET https://cloud.vestaboard.com/
//!            X-Vestaboard-Token: {token}
//!            -> {"currentMessage": {"layout": "[[...]]", "id": "..."}}

use serde_json::Value;
use std::time::Duration;

pub type Grid = Vec<Vec<u8>>;

pub struct Reading {
    pub layout: Grid,
    pub id: Option<String>,
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(concat!("Rattleboard/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("http client")
}

/// Accepts "192.168.1.42", "vestaboard.local:7000", "http://host/" etc.
pub fn local_url(host: &str) -> String {
    let h = host.trim();
    let h = h
        .strip_prefix("http://")
        .or_else(|| h.strip_prefix("https://"))
        .unwrap_or(h);
    let h = h.split('/').next().unwrap_or("").trim();
    let has_port = h.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) && !h.ends_with(']');
    if has_port {
        format!("http://{h}/local-api/message")
    } else {
        format!("http://{h}:7000/local-api/message")
    }
}

pub async fn read_local(c: &reqwest::Client, host: &str, key: &str) -> Result<Reading, String> {
    if host.trim().is_empty() {
        return Err("No board IP set".into());
    }
    if key.is_empty() {
        return Err("No Local API key set".into());
    }
    let res = c
        .get(local_url(host))
        .header("X-Vestaboard-Local-Api-Key", key)
        .send()
        .await
        .map_err(describe)?;
    let v = json_or_status(res).await?;
    let layout = match &v {
        Value::Object(o) => o.get("message").map(grid).unwrap_or(Err("Unexpected response".into()))?,
        other => grid(other)?,
    };
    Ok(Reading { layout, id: None })
}

pub async fn read_cloud(c: &reqwest::Client, token: &str) -> Result<Reading, String> {
    if token.is_empty() {
        return Err("No Cloud API token set".into());
    }
    let res = c
        .get("https://cloud.vestaboard.com/")
        .header("X-Vestaboard-Token", token)
        .header("Content-Type", "application/json")
        .send()
        .await
        .map_err(describe)?;
    let v = json_or_status(res).await?;
    let cm = v.get("currentMessage").ok_or("No current message")?;
    let layout = grid(cm.get("layout").ok_or("No layout in response")?)?;
    let id = cm.get("id").and_then(|i| match i {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    });
    Ok(Reading { layout, id })
}

async fn json_or_status(res: reqwest::Response) -> Result<Value, String> {
    let status = res.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err("Key was rejected".into());
    }
    if !status.is_success() {
        return Err(format!("Board answered HTTP {}", status.as_u16()));
    }
    res.json::<Value>().await.map_err(|_| "Response wasn't JSON".to_string())
}

/// A layout may arrive as a nested array or as a JSON string holding one.
fn grid(v: &Value) -> Result<Grid, String> {
    let parsed;
    let v = match v {
        Value::String(s) => {
            parsed = serde_json::from_str::<Value>(s).map_err(|_| "Unreadable layout")?;
            &parsed
        }
        v => v,
    };
    let rows = v.as_array().ok_or("Layout isn't an array")?;
    let g: Grid = rows
        .iter()
        .map(|r| {
            r.as_array()
                .map(|cells| cells.iter().map(|c| c.as_u64().unwrap_or(0).min(255) as u8).collect())
                .unwrap_or_default()
        })
        .collect();
    if g.is_empty() || g[0].is_empty() {
        return Err("Empty layout".into());
    }
    Ok(g)
}

fn describe(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "Timed out reaching the board".into()
    } else if e.is_connect() {
        "Can't reach the board".into()
    } else {
        "Network error".into()
    }
}

/// Looks for boards advertising the default `vestaboard.local` name over mDNS.
pub async fn scan() -> Vec<(String, String)> {
    let mut out = vec![];
    if let Ok(Ok(addrs)) =
        tokio::time::timeout(Duration::from_secs(3), tokio::net::lookup_host("vestaboard.local:7000")).await
    {
        for a in addrs {
            if a.is_ipv4() {
                let ip = a.ip().to_string();
                if !out.iter().any(|(_, i)| i == &ip) {
                    out.push(("vestaboard.local".to_string(), ip));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(local_url("192.168.1.42"), "http://192.168.1.42:7000/local-api/message");
        assert_eq!(local_url(" http://10.0.0.2:8080/ "), "http://10.0.0.2:8080/local-api/message");
        assert_eq!(local_url("vestaboard.local"), "http://vestaboard.local:7000/local-api/message");
    }

    #[test]
    fn grids() {
        let a = serde_json::json!([[0, 8, 5], [1, 2, 3]]);
        assert_eq!(grid(&a).unwrap(), vec![vec![0, 8, 5], vec![1, 2, 3]]);
        let s = serde_json::json!("[[0,1],[2,3]]");
        assert_eq!(grid(&s).unwrap(), vec![vec![0, 1], vec![2, 3]]);
        assert!(grid(&serde_json::json!([])).is_err());
    }
}

// ---------------------------------------------------------------- demo mode

/// Sample messages from the original design, used when there's no real board.
/// `{o}`, `{g}` etc. are colour flaps.
const PRESETS: [(&[&str], &[&str], bool); 5] = [
    (&["GOOD MORNING", "", "COFFEE IS READY", "FRI OCT 16  64°F"], &["GOOD MORNING", "COFFEE READY", "FRI  64°F"], false),
    (&["{o} STANDUP IN 5 MIN {o}", "", "ROOM 4B", "BRING YOUR UPDATES"], &["{o} STANDUP {o}", "IN 5 MINUTES", "ROOM 4B"], false),
    (&["UA 2481 TO DENVER", "GATE B12", "BOARDING 6:40 PM", "{g} ON TIME"], &["UA 2481  B12", "BOARDS 6:40", "{g} ON TIME"], false),
    (&["DINNER AT 7:00", "TACOS TONIGHT", "", "{g}{g}{g} WHO'S IN? {g}{g}{g}"], &["DINNER 7:00", "TACOS TONIGHT", "{g} WHO'S IN?"], false),
    (&["", "", "IT'S FRIDAY", "SEE YOU MONDAY", "", ""], &["", "IT'S FRIDAY", ""], true),
];
pub const DEMO_EVERY_SECS: u64 = 20;

fn code(ch: char) -> u8 {
    const SYM: [(char, u8); 20] = [
        ('!', 37), ('@', 38), ('#', 39), ('$', 40), ('(', 41), (')', 42), ('-', 44), ('+', 46), ('&', 47), ('=', 48),
        (';', 49), (':', 50), ('\'', 52), ('"', 53), ('%', 54), (',', 55), ('.', 56), ('/', 59), ('?', 60), ('°', 62),
    ];
    match ch {
        'A'..='Z' => ch as u8 - b'A' + 1,
        '1'..='9' => 26 + (ch as u8 - b'0'),
        '0' => 36,
        _ => SYM.iter().find(|(c, _)| *c == ch).map(|(_, v)| *v).unwrap_or(0),
    }
}

fn parse(line: &str) -> Vec<u8> {
    let chars: Vec<char> = line.to_uppercase().chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        let close = chars[i..].iter().position(|&c| c == '}').map(|p| i + p);
        let num: Option<u8> = close.and_then(|e| chars[i + 1..e].iter().collect::<String>().parse().ok());
        if let (Some(e), Some(n)) = (close, num) {
            // "{63}": a raw character code, as accepted by Vestaboard's own text endpoints
            out.push(if n <= 71 { n } else { 0 });
            i = e + 1;
        } else if chars[i] == '{' && i + 2 < chars.len() && chars[i + 2] == '}' {
            out.push(match chars[i + 1] {
                'R' => 63, 'O' => 64, 'Y' => 65, 'G' => 66, 'B' => 67, 'V' => 68, 'W' => 69, 'K' => 70, _ => 0,
            });
            i += 3;
        } else {
            out.push(code(chars[i]));
            i += 1;
        }
    }
    out
}

/// Lays plain text out like Vestaboard's text endpoints: upper-cased, word-wrapped,
/// centred both ways. Explicit newlines start a new row; `{63}` inserts a raw code.
pub fn text_layout(text: &str, rows: usize, cols: usize) -> Grid {
    let mut lines: Vec<Vec<u8>> = vec![];
    for para in text.split('\n') {
        let mut cur: Vec<u8> = vec![];
        for word in para.split_whitespace() {
            let mut w = parse(word);
            while w.len() > cols {
                if !cur.is_empty() {
                    lines.push(std::mem::take(&mut cur));
                }
                lines.push(w.drain(..cols).collect());
            }
            if w.is_empty() {
                continue;
            }
            if !cur.is_empty() && cur.len() + 1 + w.len() > cols {
                lines.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push(0);
            }
            cur.extend(w);
        }
        lines.push(cur);
    }
    while lines.len() > 1 && lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.truncate(rows);
    let mut g = vec![vec![0u8; cols]; rows];
    let top = (rows - lines.len()) / 2;
    for (i, ln) in lines.into_iter().enumerate() {
        let st = (cols - ln.len()) / 2;
        for (j, v) in ln.into_iter().enumerate() {
            g[top + i][st + j] = v;
        }
    }
    g
}

/// Validates a layout posted by an app: a rows×cols grid of codes 0–71.
pub fn posted_grid(v: &Value) -> Result<Grid, String> {
    let rows = v.as_array().ok_or("Expected an array of rows")?;
    let mut g: Grid = vec![];
    for r in rows {
        let cells = r.as_array().ok_or("Each row must be an array")?;
        let mut row = vec![];
        for c in cells {
            let n = c.as_u64().ok_or("Character codes must be numbers")?;
            if n > 71 {
                return Err(format!("Unknown character code {n}"));
            }
            row.push(n as u8);
        }
        g.push(row);
    }
    match (g.len(), g.first().map(|r| r.len())) {
        (6, Some(22)) | (3, Some(15)) if g.iter().all(|r| r.len() == g[0].len()) => Ok(g),
        _ => Err("Layout must be 6 rows of 22 (Flagship) or 3 rows of 15 (Note)".into()),
    }
}

pub fn demo(model: &str, idx: usize) -> Reading {
    let note = model == "note";
    let (rows, cols) = if note { (3, 15) } else { (6, 22) };
    let (flag, short, rainbow) = PRESETS[idx % PRESETS.len()];
    let lines = if note { short } else { flag };
    let mut g = vec![vec![0u8; cols]; rows];
    let put = |g: &mut Grid, r: usize, ln: &str| {
        let c: Vec<u8> = parse(ln).into_iter().take(cols).collect();
        let st = (cols - c.len()) / 2;
        for (j, v) in c.into_iter().enumerate() {
            g[r][st + j] = v;
        }
    };
    if rainbow {
        for (r, ln) in lines.iter().enumerate() {
            if ln.is_empty() {
                for c in 0..cols {
                    g[r][c] = 63 + ((c + r) % 6) as u8;
                }
            } else {
                put(&mut g, r, ln);
            }
        }
    } else {
        let top = (rows - lines.len()) / 2;
        for (i, ln) in lines.iter().enumerate() {
            put(&mut g, top + i, ln);
        }
    }
    Reading { layout: g, id: Some(format!("demo-{}", idx % PRESETS.len())) }
}

#[cfg(test)]
mod demo_tests {
    use super::*;

    #[test]
    fn demo_layouts() {
        let r = demo("flag", 0);
        assert_eq!((r.layout.len(), r.layout[0].len()), (6, 22));
        assert_eq!(&r.layout[1][5..17], &parse("GOOD MORNING")[..]);
        let n = demo("note", 4);
        assert_eq!((n.layout.len(), n.layout[0].len()), (3, 15));
        assert_eq!(n.layout[0][0], 63);
        assert_eq!(parse("{g}A°"), vec![66, 1, 62]);
        assert_eq!(parse("{63}B{7}"), vec![63, 2, 7]);
    }

    #[test]
    fn text() {
        let g = text_layout("hello world", 3, 15);
        assert_eq!(g[1][2..13], parse("HELLO WORLD")[..]);
        assert_eq!(g[0], vec![0; 15]);
        let g = text_layout("the quick brown fox jumps over", 3, 15);
        assert_eq!(g[0], parse("THE QUICK BROWN"));
        assert_eq!(g[1][..14], parse("FOX JUMPS OVER")[..]);
        let g = text_layout("A\n\nB", 6, 22);
        assert_eq!((g[1][10], g[3][10]), (1, 2));
        assert!(posted_grid(&serde_json::to_value(vec![vec![0u8; 15]; 3]).unwrap()).is_ok());
        assert!(posted_grid(&serde_json::to_value(vec![vec![0u8; 15]; 2]).unwrap()).is_err());
        assert!(posted_grid(&serde_json::to_value(vec![vec![99u8; 15]; 3]).unwrap()).is_err());
    }
}
