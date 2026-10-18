//! Reading the current message from a Vestaboard, and writing one to it.
//!
//! Local API: GET http://{host}:7000/local-api/message
//!            X-Vestaboard-Local-Api-Key: {key}
//!            -> [[..22 codes..] x 6]   (older firmware wraps it: {"message": [...]})
//!            POST the same URL: {"characters": [[...]], "strategy": "column", ...}
//! Cloud API: GET https://cloud.vestaboard.com/
//!            X-Vestaboard-Token: {token}
//!            -> {"currentMessage": {"layout": "[[...]]", "id": "..."}}
//!            POST the same URL: {"characters": [[...]]}

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::time::Duration;

pub type Grid = Vec<Vec<u8>>;

/// Boards come in any size up to this; the presets are Standard 6×22, Compact 3×15 and Large 8×30.
pub const MAX_ROWS: usize = 12;
pub const MAX_COLS: usize = 40;

/// A grid's size, held within the limits.
pub fn dims_of(g: &Grid) -> (usize, usize) {
    let cols = g.first().map(|r| r.len()).unwrap_or(0);
    (g.len().clamp(1, MAX_ROWS), cols.clamp(1, MAX_COLS))
}

/// Centres `g` on a rows×cols board: padded with blanks, or cropped from the centre. The
/// offset is floor((board - grid) / 2) either way, as on every Rattleboard: an odd extra
/// blank row or column goes after the content, and an odd extra one cropped comes off the start.
pub fn fit_center(g: &Grid, rows: usize, cols: usize) -> Grid {
    let (gr, gc) = (g.len(), g.first().map(|r| r.len()).unwrap_or(0));
    let mut out = vec![vec![0u8; cols]; rows];
    // Offsets into the board (pad) or into the grid (crop).
    let (pr, sr) = if gr <= rows { ((rows - gr) / 2, 0) } else { (0, (gr - rows).div_ceil(2)) };
    let (pc, sc) = if gc <= cols { ((cols - gc) / 2, 0) } else { (0, (gc - cols).div_ceil(2)) };
    for r in 0..gr.min(rows) {
        for c in 0..gc.min(cols) {
            out[pr + r][pc + c] = g[sr + r].get(sc + c).copied().unwrap_or(0);
        }
    }
    out
}

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
    format!("{}/local-api/message", base_url(host, 7000))
}

/// "http://host:port" for a host typed with or without scheme, port and path.
pub fn base_url(host: &str, default_port: u16) -> String {
    let h = host_only(host);
    if has_port(h) {
        format!("http://{h}")
    } else {
        format!("http://{h}:{default_port}")
    }
}

fn host_only(host: &str) -> &str {
    let h = host.trim();
    let h = h
        .strip_prefix("http://")
        .or_else(|| h.strip_prefix("https://"))
        .unwrap_or(h);
    h.split('/').next().unwrap_or("").trim()
}

pub fn has_port(host: &str) -> bool {
    let h = host_only(host);
    h.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) && !h.ends_with(']')
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

/// Writes a message through the Local API. `body` is `{"characters": ...}` plus any transition.
pub async fn write_local(c: &reqwest::Client, host: &str, key: &str, body: &Value) -> Result<(), String> {
    let res = c
        .post(local_url(host))
        .header("X-Vestaboard-Local-Api-Key", key)
        .json(body)
        .send()
        .await
        .map_err(describe)?;
    ok_or_status(res).await
}

/// Writes a message through the Cloud API. It ignores transitions, so only the layout goes.
pub async fn write_cloud(c: &reqwest::Client, token: &str, layout: &Grid) -> Result<(), String> {
    let res = c
        .post("https://cloud.vestaboard.com/")
        .header("X-Vestaboard-Token", token)
        .json(&json!({ "characters": layout }))
        .send()
        .await
        .map_err(describe)?;
    ok_or_status(res).await
}

pub async fn ok_or_status(res: reqwest::Response) -> Result<(), String> {
    match res.status().as_u16() {
        200..=299 => Ok(()),
        401 | 403 => Err("Key was rejected".into()),
        429 => Err("The board is rate-limiting messages; wait a little".into()),
        s => Err(format!("Board answered HTTP {s}")),
    }
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

pub fn describe(e: reqwest::Error) -> String {
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
        let close = (chars[i] == '{').then(|| chars[i..].iter().position(|&c| c == '}').map(|p| i + p)).flatten();
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

/// Validates a layout posted by an app: a rectangular grid of codes 0–71, 1–12 rows of 1–40.
/// It may be any size within that; callers centre it on their board with `fit_center`.
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
    let cols = g.first().map(|r| r.len()).unwrap_or(0);
    if g.is_empty() || cols == 0 || g.len() > MAX_ROWS || cols > MAX_COLS {
        return Err(format!("Layout must be 1 to {MAX_ROWS} rows of 1 to {MAX_COLS} codes"));
    }
    if g.iter().any(|r| r.len() != cols) {
        return Err("Every row of the layout must be the same length".into());
    }
    Ok(g)
}

/// A sample message for a rows×cols board: the long version where it fits, else the short one.
pub fn demo(rows: usize, cols: usize, idx: usize) -> Reading {
    let (long, short, rainbow) = PRESETS[idx % PRESETS.len()];
    let lines: Vec<&str> = if rows >= long.len() && cols >= 22 { long.to_vec() } else { short.iter().take(rows).cloned().collect() };
    let mut g = vec![vec![0u8; cols]; rows];
    let put = |g: &mut Grid, r: usize, ln: &str| {
        let c: Vec<u8> = parse(ln).into_iter().take(cols).collect();
        let st = (cols - c.len()) / 2;
        for (j, v) in c.into_iter().enumerate() {
            g[r][st + j] = v;
        }
    };
    let top = (rows - lines.len().min(rows)) / 2;
    if rainbow {
        // Every row without text is a band of colour.
        for r in 0..rows {
            match r.checked_sub(top).and_then(|i| lines.get(i)).filter(|ln| !ln.is_empty()) {
                Some(ln) => put(&mut g, r, ln),
                None => {
                    for (c, v) in g[r].iter_mut().enumerate() {
                        *v = 63 + ((c + r) % 6) as u8;
                    }
                }
            }
        }
    } else {
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
        let r = demo(6, 22, 0);
        assert_eq!((r.layout.len(), r.layout[0].len()), (6, 22));
        assert_eq!(&r.layout[1][5..17], &parse("GOOD MORNING")[..]);
        let n = demo(3, 15, 4);
        assert_eq!((n.layout.len(), n.layout[0].len()), (3, 15));
        assert_eq!(n.layout[0][0], 63);
        for (rows, cols) in [(8, 30), (2, 8), (12, 40)] {
            for i in 0..5 {
                let g = demo(rows, cols, i).layout;
                assert_eq!((g.len(), g[0].len()), (rows, cols));
            }
        }
        assert_eq!(demo(8, 30, 4).layout[7][0], 63 + (7 % 6) as u8);
        assert_eq!(parse("{g}A°"), vec![66, 1, 62]);
        assert_eq!(parse("{63}B{7}"), vec![63, 2, 7]);
    }

    #[test]
    fn text() {
        let g = text_layout("hello world", 3, 15);
        assert_eq!(g[1][2..13], parse("HELLO WORLD")[..]);
        assert_eq!(g[0], vec![0; 15]);
        // stray braces are just characters, never a crash or a bogus code
        assert_eq!(parse("a}"), vec![1, code('}')]);
        assert_eq!(parse("5}"), vec![code('5'), code('}')]);
        assert_eq!(parse("{63}{g}"), vec![63, 66]);
        let g = text_layout("the quick brown fox jumps over", 3, 15);
        assert_eq!(g[0], parse("THE QUICK BROWN"));
        assert_eq!(g[1][..14], parse("FOX JUMPS OVER")[..]);
        let g = text_layout("A\n\nB", 6, 22);
        assert_eq!((g[1][10], g[3][10]), (1, 2));
        let v = |g: Vec<Vec<u8>>| serde_json::to_value(g).unwrap();
        assert!(posted_grid(&v(vec![vec![0u8; 15]; 3])).is_ok());
        assert!(posted_grid(&v(vec![vec![0u8; 15]; 2])).is_ok());
        assert!(posted_grid(&v(vec![vec![0u8; 40]; 12])).is_ok());
        assert!(posted_grid(&v(vec![vec![0u8; 41]; 3])).is_err());
        assert!(posted_grid(&v(vec![vec![0u8; 5]; 13])).is_err());
        assert!(posted_grid(&v(vec![vec![]; 3])).is_err());
        assert!(posted_grid(&v(vec![])).is_err());
        assert!(posted_grid(&v(vec![vec![1, 2], vec![1]])).is_err());
        assert!(posted_grid(&v(vec![vec![99u8; 15]; 3])).is_err());
        let g = text_layout("a b c d e f g h", 8, 30);
        assert_eq!((g.len(), g[0].len()), (8, 30));
        let g = text_layout("wonderful", 2, 8);
        assert_eq!(g[0], parse("WONDERFU"));
    }

    #[test]
    fn centring() {
        let small = vec![vec![1u8; 15]; 3];
        let g = fit_center(&small, 6, 22);
        assert_eq!((g.len(), g[0].len()), (6, 22));
        assert_eq!((g[0].clone(), g[1][2], g[1][3], g[3][17], g[3][18], g[4][4]), (vec![0; 22], 0, 1, 1, 0, 0));
        // larger grids are cropped from the centre
        let big: Grid = (0..8).map(|r| (0..30).map(|c| (r * 30 + c) as u8 % 60).collect()).collect();
        let g = fit_center(&big, 6, 22);
        assert_eq!((g.len(), g[0].len(), g[0][0]), (6, 22, big[1][4]));
        assert_eq!(fit_center(&big, 8, 30), big);
        // Odd differences: the extra blank goes after the content; the extra crop off the start.
        let g = fit_center(&vec![vec![1u8; 2]; 1], 2, 5);
        assert_eq!(g, vec![vec![0, 1, 1, 0, 0], vec![0; 5]]);
        let wide: Grid = vec![(1..=5).collect(), (6..=10).collect()];
        assert_eq!(fit_center(&wide, 1, 2), vec![vec![8, 9]]);
        let tall: Grid = vec![vec![1], vec![2], vec![3]];
        assert_eq!(fit_center(&tall, 2, 1), vec![vec![2], vec![3]]);
        assert_eq!(dims_of(&big), (8, 30));
    }
}

// ---------------------------------------------------------------- transitions

pub const STRATEGIES: [&str; 7] = ["classic", "column", "reverse-column", "edges-to-center", "row", "diagonal", "random"];

/// How the flaps of a new message start: all at once ("classic") or in staggered groups.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    pub strategy: String,
    pub step_interval_ms: u32,
    pub step_size: u32,
}

impl Default for Transition {
    fn default() -> Self {
        Self { strategy: "classic".into(), step_interval_ms: 120, step_size: 1 }
    }
}

impl Transition {
    pub fn classic(&self) -> bool {
        self.strategy == "classic"
    }

    /// The optional fields of a Local API post; nothing for classic.
    pub fn add_to(&self, body: &mut Map<String, Value>) {
        if !self.classic() {
            body.insert("strategy".into(), json!(self.strategy));
            body.insert("step_interval_ms".into(), json!(self.step_interval_ms));
            body.insert("step_size".into(), json!(self.step_size));
        }
    }

    /// Reads `strategy`, `step_interval_ms` and `step_size` from a posted object.
    pub fn parse(o: &Map<String, Value>) -> Result<Self, String> {
        let mut t = Self::default();
        match o.get("strategy") {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) if STRATEGIES.contains(&s.as_str()) => t.strategy = s.clone(),
            Some(v) => return Err(format!("Unknown strategy {v}; use one of {}", STRATEGIES.join(", "))),
        }
        let num = |k: &str, max: u64, def: u32| -> Result<u32, String> {
            match o.get(k) {
                None | Some(Value::Null) => Ok(def),
                Some(v) => match v.as_u64() {
                    Some(n) if (1..=max).contains(&n) => Ok(n as u32),
                    _ => Err(format!("{k} must be a whole number from 1 to {max}")),
                },
            }
        };
        t.step_interval_ms = num("step_interval_ms", 5000, 120)?;
        t.step_size = num("step_size", 22, 1)?;
        Ok(t)
    }

    /// The step a tile starts on, as in split-flap.js; None for classic and random,
    /// whose start times are random.
    fn group(&self, r: usize, c: usize, cols: usize) -> Option<usize> {
        let size = self.step_size.max(1) as usize;
        let k = match self.strategy.as_str() {
            "column" => c,
            "reverse-column" => cols - 1 - c,
            "edges-to-center" => c.min(cols - 1 - c),
            "row" => r,
            "diagonal" => r + c,
            _ => return None,
        };
        Some(k / size)
    }
}

/// The drum order of a flap: blank, A–Z, 1–9, 0, punctuation, then the colour chips.
fn drum_index(code: u8) -> usize {
    const PUNCT: [u8; 20] = [37, 38, 39, 40, 41, 42, 44, 46, 47, 48, 49, 50, 52, 53, 54, 55, 56, 59, 60, 62];
    match code {
        0..=36 => code as usize,
        63..=71 => 37 + PUNCT.len() + (code - 63) as usize,
        _ => PUNCT.iter().position(|&p| p == code).map(|i| 37 + i).unwrap_or(0),
    }
}
const DRUM_LEN: usize = 37 + 20 + 9;

/// How long the replica takes to flip from `from` to `to` at `speed`: an upper bound that
/// mirrors the timing in split-flap.js, so a sequence's hold starts once the flaps are done.
pub fn flip_ms(from: &Grid, to: &Grid, tr: &Transition, speed: f64) -> u64 {
    let rows = to.len();
    let cols = to.first().map(|r| r.len()).unwrap_or(0);
    if rows == 0 || cols == 0 {
        return 0;
    }
    // A board of another size is rebuilt, so every flap starts from blank.
    let same_size = from.len() == rows && from.iter().all(|r| r.len() == cols);
    let step = 62.0 / speed.max(0.25);
    let jitter = (tr.step_interval_ms as f64 / 2.0).min(30.0);
    let last_group = match tr.strategy.as_str() {
        "random" => (rows * cols - 1) / tr.step_size.max(1) as usize,
        _ => 0,
    };
    let mut end: f64 = 0.0;
    for (r, row) in to.iter().enumerate() {
        for (c, &code) in row.iter().enumerate() {
            let cur = if same_size { drum_index(from[r][c]) } else { 0 };
            let steps = (drum_index(code) + DRUM_LEN - cur) % DRUM_LEN;
            if steps == 0 {
                continue;
            }
            let start = if tr.classic() {
                c as f64 * 12.0 + 160.0
            } else {
                let k = tr.group(r, c, cols).unwrap_or(last_group);
                k as f64 * tr.step_interval_ms as f64 + jitter
            };
            // Each flap runs up to 10% slow; allow a frame for the last one to land.
            end = end.max(start + steps as f64 * step * 1.1 + 17.0);
        }
    }
    if end == 0.0 {
        0
    } else {
        (end + step).ceil() as u64
    }
}

#[cfg(test)]
mod transition_tests {
    use super::*;

    #[test]
    fn parsing() {
        let o = |v: Value| v.as_object().unwrap().clone();
        assert_eq!(Transition::parse(&o(json!({}))).unwrap(), Transition::default());
        let t = Transition::parse(&o(json!({"strategy": "column", "step_interval_ms": 50, "step_size": 2}))).unwrap();
        assert_eq!((t.strategy.as_str(), t.step_interval_ms, t.step_size), ("column", 50, 2));
        assert!(Transition::parse(&o(json!({"strategy": "wipe"}))).is_err());
        assert!(Transition::parse(&o(json!({"strategy": 3}))).is_err());
        assert!(Transition::parse(&o(json!({"step_interval_ms": 0}))).is_err());
        assert!(Transition::parse(&o(json!({"step_interval_ms": 5001}))).is_err());
        assert!(Transition::parse(&o(json!({"step_size": 23}))).is_err());
        assert!(Transition::parse(&o(json!({"step_size": 1.5}))).is_err());
        let mut body = Map::new();
        Transition::default().add_to(&mut body);
        assert!(body.is_empty());
        t.add_to(&mut body);
        assert_eq!(body["step_size"], 2);
    }

    #[test]
    fn timing() {
        let blank = vec![vec![0u8; 22]; 6];
        assert_eq!(flip_ms(&blank, &blank, &Transition::default(), 1.0), 0);
        let mut a = blank.clone();
        a[0][0] = 1;
        // one step, starting within 160 ms
        let t = flip_ms(&blank, &a, &Transition::default(), 1.0);
        assert!((200..400).contains(&t), "{t}");
        // the last column of a column wipe starts 21 steps later
        let mut z = blank.clone();
        z[0][21] = 1;
        let col = Transition { strategy: "column".into(), step_interval_ms: 100, step_size: 1 };
        assert!(flip_ms(&blank, &z, &col, 1.0) > 2100);
        assert!(flip_ms(&blank, &z, &col, 1.0) < flip_ms(&blank, &z, &Transition { step_size: 1, ..col.clone() }, 0.5));
        // colour chips sit at the end of the drum
        assert_eq!(drum_index(71), DRUM_LEN - 1);
        assert_eq!(drum_index(62), 56);
        assert!(flip_ms(&vec![vec![0u8; 15]; 3], &a, &col, 1.0) > 0);
    }
}
