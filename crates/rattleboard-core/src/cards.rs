//! Cards: what a source puts on a board, laid out for any board size.
//!
//! A card is kept as what it says (a face), not as a grid, so the same card can go to boards
//! of different sizes: this one, and the saved boards a hub drives. Long text continues on
//! up to three pages; words are never cut, except one longer than a whole row.

use crate::vestaboard::{self, fit_center, parse, place, wrap, Grid};
use serde::Serialize;

/// A filled tile: the board's ink colour.
pub const FILLED: u8 = 71;
const RAINBOW: [u8; 6] = [63, 64, 65, 66, 67, 68];

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "face", rename_all = "camelCase")]
pub enum Face {
    /// The standard source card: chip, name and time across the top row, the body
    /// word-wrapped and centred below.
    Standard { chip: u8, name: String, time: String, body: String, pages: usize },
    /// Text over the whole board, as the standard text layout lays it out (templates,
    /// countdowns, messages of the day). A chip, if any, marks the top-left corner.
    Text { text: String, left: bool, chip: Option<u8>, pages: usize },
    /// Exact characters, centred (a command's {"characters"}).
    Grid { grid: Grid },
    /// The time in big digits made of tiles, with a line under it.
    BigTime { time: String, sub: String, fill: u8 },
    /// Label and value per row, like a world clock. Each value comes long ("9:41 PM") and
    /// short ("9:41P"); the short one is used when the long one would crowd the label.
    Rows { chip: u8, title: String, rows: Vec<(String, String, String)> },
    /// Bands of colour above and below the text.
    Celebrate { text: String },
    /// Quote of the day: the first quote in the list that fits the board, counted from `day`.
    Quote { quotes: Vec<(String, String)>, day: i64, chip: Option<u8> },
}

impl Face {
    /// The pages of this card on a rows×cols board (always at least one).
    pub fn render(&self, rows: usize, cols: usize) -> Vec<Grid> {
        let (rows, cols) = (rows.clamp(1, vestaboard::MAX_ROWS), cols.clamp(1, vestaboard::MAX_COLS));
        let pages = match self {
            Face::Standard { chip, name, time, body, pages } => standard(*chip, name, time, body, *pages, rows, cols),
            Face::Text { text, left, chip, pages } => text_pages(text, *left, *chip, *pages, rows, cols),
            Face::Grid { grid } => vec![fit_center(grid, rows, cols)],
            Face::BigTime { time, sub, fill } => vec![big_time(time, sub, *fill, rows, cols)],
            Face::Rows { chip, title, rows: list } => vec![table(*chip, title, list, rows, cols)],
            Face::Celebrate { text } => vec![celebrate(text, rows, cols)],
            Face::Quote { quotes, day, chip } => vec![quote_card(quotes, *day, *chip, rows, cols)],
        };
        if pages.is_empty() {
            vec![vec![vec![0; cols]; rows]]
        } else {
            pages
        }
    }
}

fn put(g: &mut Grid, r: usize, c: usize, codes: &[u8]) {
    let cols = g[r].len();
    for (i, &v) in codes.iter().enumerate() {
        if c + i < cols {
            g[r][c + i] = v;
        }
    }
}

/// Up to `n` codes of a line, ending at a word where it can.
fn clip(codes: Vec<u8>, n: usize) -> Vec<u8> {
    if codes.len() <= n {
        return codes;
    }
    let cut = &codes[..n];
    match cut.iter().rposition(|&c| c == 0) {
        Some(i) if i > n / 2 => cut[..i].to_vec(),
        _ => cut.to_vec(),
    }
}

/// Lines split into pages of `per` rows, at most `max` pages; anything beyond is dropped at a
/// line break, so no word is cut.
fn paginate(lines: Vec<Vec<u8>>, per: usize, max: usize) -> Vec<Vec<Vec<u8>>> {
    if per == 0 {
        return vec![];
    }
    let mut out: Vec<Vec<Vec<u8>>> = lines.chunks(per).map(|c| c.to_vec()).collect();
    out.truncate(max.clamp(1, 3));
    out
}

fn standard(chip: u8, name: &str, time: &str, body: &str, pages: usize, rows: usize, cols: usize) -> Vec<Grid> {
    let mut head = vec![0u8; cols];
    let mut c = 0;
    if chip != 0 && cols > 4 {
        head[0] = chip;
        c = 2;
    }
    let time = parse(time);
    // The time goes in the corner when the whole name still fits beside it (or, for a long
    // name, a good part of it); otherwise the name has the row.
    let name_len = parse(name).len();
    let show_time = !time.is_empty() && (cols >= c + name_len + 1 + time.len() || (name_len > cols - c && cols >= c + time.len() + 12));
    let room = cols - c - if show_time { time.len() + 1 } else { 0 };
    let name = clip(parse(name), room);
    head[c..c + name.len()].copy_from_slice(&name);
    if show_time {
        head[cols - time.len()..].copy_from_slice(&time);
    }
    if rows == 1 {
        return vec![vec![head]];
    }
    let lines = wrap(body, cols);
    let lines: Vec<Vec<u8>> = if lines.iter().all(|l| l.is_empty()) { vec![] } else { lines };
    let chunks = paginate(lines, rows - 1, pages);
    if chunks.is_empty() {
        let mut g = vec![vec![0u8; cols]; rows];
        g[0] = head;
        return vec![g];
    }
    chunks
        .into_iter()
        .map(|chunk| {
            let body = place(&chunk, rows - 1, cols, false);
            std::iter::once(head.clone()).chain(body).collect()
        })
        .collect()
}

fn text_pages(text: &str, left: bool, chip: Option<u8>, pages: usize, rows: usize, cols: usize) -> Vec<Grid> {
    let lines = wrap(text, cols);
    paginate(lines, rows, pages)
        .into_iter()
        .map(|chunk| {
            let mut g = place(&chunk, rows, cols, left);
            if let Some(c) = chip {
                if g[0][0] == 0 && !left {
                    g[0][0] = c;
                }
            }
            g
        })
        .collect()
}

/// 3×5 digits.
const DIGITS: [[&str; 5]; 10] = [
    ["###", "#.#", "#.#", "#.#", "###"],
    [".#.", "##.", ".#.", ".#.", "###"],
    ["###", "..#", "###", "#..", "###"],
    ["###", "..#", "###", "..#", "###"],
    ["#.#", "#.#", "###", "..#", "..#"],
    ["###", "#..", "###", "..#", "###"],
    ["###", "#..", "###", "#.#", "###"],
    ["###", "..#", "..#", "..#", "..#"],
    ["###", "#.#", "###", "#.#", "###"],
    ["###", "#.#", "###", "..#", "###"],
];

/// "9:41" in digits five tiles tall, when the board has room; otherwise as text.
fn big_time(time: &str, sub: &str, fill: u8, rows: usize, cols: usize) -> Grid {
    let glyphs: Vec<char> = time.chars().filter(|c| c.is_ascii_digit() || *c == ':').collect();
    let width: usize = glyphs.iter().map(|c| if *c == ':' { 1 } else { 3 }).sum::<usize>() + glyphs.len().saturating_sub(1);
    let sub_codes = clip(parse(sub), cols);
    let with_sub = !sub_codes.is_empty() && rows >= 6;
    if rows < 5 || width > cols || glyphs.is_empty() {
        let text = if sub.is_empty() { time.to_string() } else { format!("{time}\n{sub}") };
        return vestaboard::text_layout(&text, rows, cols);
    }
    let mut g = vec![vec![0u8; cols]; rows];
    let height = if with_sub { 6 } else { 5 };
    let top = (rows - height) / 2;
    let mut c = (cols - width) / 2;
    for ch in glyphs {
        if ch == ':' {
            g[top + 1][c] = fill;
            g[top + 3][c] = fill;
            c += 2;
            continue;
        }
        let d = DIGITS[ch.to_digit(10).unwrap_or(0) as usize];
        for (r, line) in d.iter().enumerate() {
            for (k, px) in line.chars().enumerate() {
                if px == '#' {
                    g[top + r][c + k] = fill;
                }
            }
        }
        c += 4;
    }
    if with_sub {
        let st = (cols - sub_codes.len()) / 2;
        put(&mut g, top + 5, st, &sub_codes);
    }
    g
}

fn table(chip: u8, title: &str, list: &[(String, String, String)], rows: usize, cols: usize) -> Grid {
    let mut g = vec![vec![0u8; cols]; rows];
    let header = rows > list.len() && !title.is_empty();
    let mut r0 = 0;
    if header {
        g[0] = standard(chip, title, "", "", 1, 1, cols).remove(0).remove(0);
        r0 = 1;
    }
    let free = rows - r0;
    let shown = &list[..list.len().min(free)];
    let top = r0 + (free - shown.len()) / 2;
    for (i, (label, long, short)) in shown.iter().enumerate() {
        let fits_long = parse(label).len() + 1 + parse(long).len() <= cols;
        let v = clip(parse(if fits_long { long } else { short }), cols);
        let l = clip(parse(label), cols.saturating_sub(v.len() + 1));
        put(&mut g, top + i, 0, &l);
        put(&mut g, top + i, cols - v.len(), &v);
    }
    g
}

fn celebrate(text: &str, rows: usize, cols: usize) -> Grid {
    let lines = wrap(text, cols);
    if rows >= lines.len() + 2 && rows >= 3 {
        let mut g = vec![vec![0u8; cols]; rows];
        for c in 0..cols {
            g[0][c] = RAINBOW[c % 6];
            g[rows - 1][c] = RAINBOW[(cols - 1 - c) % 6];
        }
        let inner = place(&lines, rows - 2, cols, false);
        for (i, row) in inner.into_iter().enumerate() {
            g[i + 1] = row;
        }
        g
    } else {
        let mut g = place(&lines, rows, cols, false);
        let last = rows - 1;
        for (r, c) in [(0, 0), (0, cols - 1), (last, 0), (last, cols - 1)] {
            if g[r][c] == 0 {
                g[r][c] = RAINBOW[(r + c) % 6];
            }
        }
        g
    }
}

/// The author line as the board writes it.
pub fn author_line(author: &str) -> String {
    if author.trim().is_empty() {
        String::new()
    } else {
        format!("- {}", author.trim())
    }
}

/// A quote fits when its text wraps into the rows above the author line, with no word cut,
/// and the author line fits on one row.
pub fn quote_fits(text: &str, author: &str, rows: usize, cols: usize) -> bool {
    let a = author_line(author);
    let text_rows = if a.is_empty() { rows } else { rows.saturating_sub(1) };
    text_rows > 0 && parse(&a).len() <= cols && vestaboard::fits(text, text_rows, cols)
}

/// The quotes that fit a board.
pub fn fitting(quotes: &[(String, String)], rows: usize, cols: usize) -> Vec<&(String, String)> {
    quotes.iter().filter(|(t, a)| quote_fits(t, a, rows, cols)).collect()
}

/// The day's quote for this board size: day mod N among the quotes that fit it.
pub fn pick_quote(quotes: &[(String, String)], day: i64, rows: usize, cols: usize) -> Option<&(String, String)> {
    let fit = fitting(quotes, rows, cols);
    if fit.is_empty() {
        return None;
    }
    Some(fit[day.rem_euclid(fit.len() as i64) as usize])
}

fn quote_card(quotes: &[(String, String)], day: i64, chip: Option<u8>, rows: usize, cols: usize) -> Grid {
    let Some((text, author)) = pick_quote(quotes, day, rows, cols) else {
        return vec![vec![0; cols]; rows];
    };
    let a = parse(&author_line(author));
    let text_rows = if a.is_empty() { rows } else { rows - 1 };
    let mut g = place(&wrap(text, cols), text_rows, cols, false);
    if !a.is_empty() {
        // Right-aligned when there's room for a tile of margin, otherwise centred.
        let st = if a.len() < cols { cols - a.len() - 1 } else { 0 };
        let mut last = vec![0u8; cols];
        put_row(&mut last, st, &a);
        g.push(last);
    }
    if let Some(c) = chip {
        if g[0][0] == 0 {
            g[0][0] = c;
        }
    }
    g
}

fn put_row(row: &mut [u8], c: usize, codes: &[u8]) {
    for (i, &v) in codes.iter().enumerate() {
        if c + i < row.len() {
            row[c + i] = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(row: &[u8]) -> String {
        row.iter().map(|&c| match c {
            0 => ' ',
            1..=26 => (b'A' + c - 1) as char,
            27..=35 => (b'1' + c - 27) as char,
            36 => '0',
            50 => ':',
            44 => '-',
            62 => '°',
            63..=71 => '#',
            _ => '?',
        }).collect()
    }

    #[test]
    fn standard_cards() {
        let f = Face::Standard { chip: 64, name: "Hacker News".into(), time: "9:41".into(), body: "Show HN: a split flap board for your desk".into(), pages: 1 };
        let g = f.render(6, 22).remove(0);
        assert_eq!(text(&g[0]), "# HACKER NEWS     9:41");
        assert!(g[1..].iter().any(|r| text(r).contains("SHOW HN")));
        // Compact: the whole name rather than the time
        let g = f.render(3, 15).remove(0);
        assert_eq!(text(&g[0]), "# HACKER NEWS  ");
        let w = Face::Standard { chip: 65, name: "Melbourne".into(), time: "10:37".into(), body: String::new(), pages: 1 };
        assert_eq!(text(&w.render(3, 15)[0][0]), "# MELBOURNE    ");
        assert_eq!(text(&w.render(6, 22)[0][0]), "# MELBOURNE      10:37");
        assert_eq!(g.len(), 3);
        // Long text continues on a second page instead of being cut
        let long = Face::Standard { chip: 64, name: "News".into(), time: String::new(), body: "word ".repeat(40), pages: 2 };
        let p = long.render(3, 15);
        assert_eq!(p.len(), 2);
        assert!(p.iter().all(|g| text(&g[0]).starts_with("# NEWS")));
        // No body: just the header
        let g = Face::Standard { chip: 0, name: "Empty".into(), time: String::new(), body: String::new(), pages: 1 }.render(6, 22).remove(0);
        assert_eq!(text(&g[0]).trim(), "EMPTY");
    }

    #[test]
    fn big_clock() {
        let g = Face::BigTime { time: "12:45".into(), sub: "PM SAT 18 OCT".into(), fill: FILLED }.render(6, 22).remove(0);
        assert_eq!(g.len(), 6);
        assert_eq!(text(&g[0]), "   #  ###   # # ###   ");
        assert!(text(&g[5]).contains("PM SAT 18 OCT"));
        // too small for digits: plain text
        let g = Face::BigTime { time: "12:45".into(), sub: "PM".into(), fill: FILLED }.render(3, 15).remove(0);
        assert!(text(&g[0]).contains("12:45") || text(&g[1]).contains("12:45"));
        let g = Face::BigTime { time: "9:41".into(), sub: String::new(), fill: 66 }.render(8, 30).remove(0);
        assert!(g.iter().flatten().filter(|&&c| c == 66).count() > 20);
    }

    #[test]
    fn other_faces() {
        let row = |a: &str, b: &str, c: &str| (a.to_string(), b.to_string(), c.to_string());
        let rows = vec![row("Melbourne", "9:41 PM", "9:41P"), row("London", "11:41 AM", "11:41A")];
        let g = Face::Rows { chip: 69, title: "World clock".into(), rows: rows.clone() }.render(3, 15).remove(0);
        assert_eq!(text(&g[0]), "# WORLD CLOCK  ");
        assert!(text(&g[1]).starts_with("MELBOURNE") && text(&g[2]).starts_with("LONDON"));
        let g = Face::Celebrate { text: "Happy birthday Sam".into() }.render(6, 22).remove(0);
        assert_eq!((g[0][0], g[5][21]), (63, 63));
        assert!(text(&g[2]).contains("HAPPY BIRTHDAY SAM") || text(&g[3]).contains("HAPPY BIRTHDAY SAM"));
        let g = Face::Celebrate { text: "Merry Christmas".into() }.render(2, 8).remove(0);
        assert_eq!(g.len(), 2);
        let g = Face::Text { text: "{r} hello".into(), left: false, chip: Some(66), pages: 1 }.render(3, 15).remove(0);
        assert_eq!((g[0][0], g[1][4]), (66, 63));
        let g = Face::Grid { grid: vec![vec![1, 2, 3]] }.render(3, 15).remove(0);
        assert_eq!(g[1][6..9], [1, 2, 3]);
    }

    #[test]
    fn quotes() {
        let q = |t: &str, a: &str| (t.to_string(), a.to_string());
        let list = vec![
            q("Well begun is half done.", "Aristotle"),
            q("Whatever you are, be a good one, and do it with all the heart you have in you.", "Marcus Aurelius"),
            q("Brevity is the soul of wit.", "Shakespeare"),
        ];
        assert!(quote_fits(&list[0].0, &list[0].1, 3, 15));
        assert!(!quote_fits(&list[1].0, &list[1].1, 3, 15));
        assert!(quote_fits(&list[1].0, &list[1].1, 6, 22));
        // An author line longer than a row never fits
        assert!(!quote_fits("Hi", "Someone With A Very Long Name", 6, 22));
        assert_eq!(fitting(&list, 3, 15).len(), 2);
        // Day mod N among those that fit: Compact skips the long one
        assert_eq!(pick_quote(&list, 0, 3, 15).unwrap().1, "Aristotle");
        assert_eq!(pick_quote(&list, 1, 3, 15).unwrap().1, "Shakespeare");
        assert_eq!(pick_quote(&list, 2, 3, 15).unwrap().1, "Aristotle");
        assert_eq!(pick_quote(&list, 1, 6, 22).unwrap().1, "Marcus Aurelius");
        assert_eq!(pick_quote(&list, -1, 6, 22).unwrap().1, "Shakespeare");
        let g = Face::Quote { quotes: list.clone(), day: 1, chip: Some(67) }.render(6, 22).remove(0);
        assert_eq!(text(&g[5]).trim_end(), format!("{:>21}", "- MARCUS AURELIUS"));
        // a full first row leaves no room for the chip
        assert_ne!(g[0][0], 67);
        let g = Face::Quote { quotes: list, day: 0, chip: Some(67) }.render(3, 15).remove(0);
        assert_eq!(g[0][0], 67);
        assert_eq!(text(&g[2]), "   - ARISTOTLE ");
        assert!(text(&g[0]).contains("WELL BEGUN") && text(&g[1]).contains("HALF DONE"));
    }
}
