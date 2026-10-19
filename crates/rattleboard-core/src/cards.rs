//! Cards: what a source puts on a board, laid out for any board size.
//!
//! A card is kept as what it says (a face), not as a grid, so the same card can go to boards
//! of different sizes: this one, and the saved boards a hub drives. Long text continues on
//! up to three pages; words are never cut, except one longer than a whole row.

use crate::vestaboard::{self, fit_center, parse, place, wrap, Grid};
use serde::Serialize;

/// A filled tile: the board's ink colour.
pub const FILLED: u8 = 71;
/// Red, orange, yellow, green, blue, violet.
pub const RAINBOW: [u8; 6] = [63, 64, 65, 66, 67, 68];
/// Frames of a celebration (pages of the alert; one a minute in the rotation).
pub const CELEBRATE_FRAMES: usize = 3;

/// How much colour a built-in's card uses (its "Colour" setting).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Colour {
    /// Colour as designed: rainbows, palettes, confetti
    #[default]
    Playful,
    /// Every colour tile in one colour (the source's chip)
    Accent,
    /// No colour tiles: bars and digits filled, decoration left out
    Off,
}

impl Colour {
    pub fn parse(s: &str) -> Colour {
        match s {
            "accent" => Colour::Accent,
            "off" => Colour::Off,
            _ => Colour::Playful,
        }
    }

    /// The tile to draw for colour `want`. `structural` tiles (a progress bar, digits) are
    /// filled when colour is off; decorative ones (frames, confetti, corners) are left blank.
    pub fn ink(self, want: u8, accent: u8, structural: bool) -> u8 {
        match self {
            Colour::Playful => want,
            Colour::Accent => accent,
            Colour::Off if structural => FILLED,
            Colour::Off => 0,
        }
    }
}

/// The clock's colourful styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClockStyle {
    /// Big digits over an hour-progress bar in the hour's colour
    Colour,
    /// Big digits framed by the time of day's palette
    Daypart,
    /// Rainbow digits, a step along the rainbow each minute
    Playful,
}

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
    /// Quote of the day: the first quote in the list that fits the board, counted from `day`.
    /// `accents` (Playful): each quote's colour, put in the top-left and bottom-right corners
    /// instead of the chip.
    Quote { quotes: Vec<(String, String)>, day: i64, chip: Option<u8>, accents: Vec<u8> },
    /// A colourful clock. `time` "9:41", `sub` "PM Sun 18 Oct" (under big digits), `line`
    /// "9:41 PM" and `date` "Sun 18 Oct" (on boards too small for digits).
    Clock { style: ClockStyle, time: String, sub: String, line: String, date: String, hour: u32, minute: u32, colour: Colour, accent: u8 },
    /// A countdown with a progress bar along the bottom row, `permille` of the way there.
    Countdown { text: String, chip: Option<u8>, permille: u16, colour: Colour, accent: u8 },
    /// Text with a colour tile in the top-left and bottom-right corners (where they're free).
    Corners { text: String, colour: u8, pages: usize },
    /// Bands of colour above and below the text, and confetti. `frame` None: every frame, as
    /// pages; Some(k): frame k alone.
    Celebrate { text: String, colour: Colour, accent: u8, frame: Option<usize> },
    /// Several faces, one after another (a script plugin's "pages").
    Pages { pages: Vec<Face> },
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
            Face::Celebrate { text, colour, accent, frame } => match frame {
                Some(k) => vec![celebrate(text, *colour, *accent, k % CELEBRATE_FRAMES, rows, cols)],
                None => (0..CELEBRATE_FRAMES).map(|k| celebrate(text, *colour, *accent, k, rows, cols)).collect(),
            },
            Face::Quote { quotes, day, chip, accents } => vec![quote_card(quotes, *day, *chip, accents, rows, cols)],
            Face::Clock { style, time, sub, line, date, hour, minute, colour, accent } => vec![colour_clock(*style, time, sub, line, date, *hour, *minute, *colour, *accent, rows, cols)],
            Face::Countdown { text, chip, permille, colour, accent } => vec![countdown_card(text, *chip, *permille, *colour, *accent, rows, cols)],
            Face::Corners { text, colour, pages } => text_pages(text, false, None, *pages, rows, cols)
                .into_iter()
                .map(|mut g| {
                    corners(&mut g, *colour);
                    g
                })
                .collect(),
            Face::Pages { pages } => pages.iter().flat_map(|f| f.render(rows, cols)).collect(),
        };
        if pages.is_empty() {
            vec![vec![vec![0; cols]; rows]]
        } else {
            pages
        }
    }
}

/// One tile as a character, for tests, the spec and tools: letters, digits and punctuation
/// as they are, colour tiles as lower-case initials (r o y g b v w, k for black) and the
/// filled tile as █.
pub fn tile_char(c: u8) -> char {
    const SYM: [(u8, char); 20] = [
        (37, '!'), (38, '@'), (39, '#'), (40, '$'), (41, '('), (42, ')'), (44, '-'), (46, '+'), (47, '&'), (48, '='),
        (49, ';'), (50, ':'), (52, '\''), (53, '"'), (54, '%'), (55, ','), (56, '.'), (59, '/'), (60, '?'), (62, '°'),
    ];
    match c {
        0 => ' ',
        1..=26 => (b'A' + c - 1) as char,
        27..=35 => (b'1' + c - 27) as char,
        36 => '0',
        63..=70 => ['r', 'o', 'y', 'g', 'b', 'v', 'w', 'k'][(c - 63) as usize],
        71 => '█',
        _ => SYM.iter().find(|x| x.0 == c).map(|x| x.1).unwrap_or('?'),
    }
}

/// A grid as lines of `tile_char`s.
pub fn ascii(g: &Grid) -> Vec<String> {
    g.iter().map(|r| r.iter().map(|&c| tile_char(c)).collect()).collect()
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

/// Frame `k` of a celebration. Playful: rainbow bands top and bottom (walking a step each
/// frame) when there's room, otherwise rainbow corners, and confetti in the free space.
#[allow(clippy::needless_range_loop)]
fn celebrate(text: &str, colour: Colour, accent: u8, k: usize, rows: usize, cols: usize) -> Grid {
    let lines = wrap(text, cols);
    let banded = rows >= lines.len() + 2 && rows >= 3;
    let mut g = if banded {
        let mut g = vec![vec![0u8; cols]; rows];
        for (i, row) in place(&lines, rows - 2, cols, false).into_iter().enumerate() {
            g[i + 1] = row;
        }
        g
    } else {
        place(&lines, rows, cols, false)
    };
    let text = g.clone();
    let blank = |r: usize, c: usize| text[r][c] == 0;
    // Clear of words: this tile and two either side on its row are blank.
    let clear = |r: usize, c: usize| (c.saturating_sub(2)..(c + 3).min(cols)).all(|x| blank(r, x));
    let ink = |want: u8| colour.ink(want, accent, false);
    if banded {
        for c in 0..cols {
            g[0][c] = ink(RAINBOW[(c + k) % 6]);
            g[rows - 1][c] = ink(RAINBOW[(cols - 1 - c + k) % 6]);
        }
    } else {
        let last = rows - 1;
        for (r, c) in [(0, 0), (0, cols - 1), (last, 0), (last, cols - 1)] {
            if clear(r, c) {
                g[r][c] = ink(RAINBOW[(r + c + k) % 6]);
            }
        }
    }
    // Confetti, scattered differently each frame, kept clear of the words.
    let (r0, r1) = if banded { (1, rows - 1) } else { (0, rows) };
    for r in r0..r1 {
        for c in 0..cols {
            if clear(r, c) && g[r][c] == 0 && (r * 7 + c * 3 + k * 5 + r * c).is_multiple_of(11) {
                g[r][c] = ink(RAINBOW[(r + 2 * c + k) % 6]);
            }
        }
    }
    g
}

/// Colour tiles in two corners, each only where it and the tile beside it on its row are
/// blank (so it never touches a word): the top-left, and the bottom-right or else the
/// bottom-left. Boards under 2 rows or 2 columns get none.
fn corners(g: &mut Grid, colour: u8) {
    let (rows, cols) = (g.len(), g[0].len());
    if rows < 2 || cols < 2 {
        return;
    }
    let free = |g: &Grid, r: usize, c: usize, n: usize| g[r][c] == 0 && g[r][n] == 0;
    if free(g, 0, 0, 1) {
        g[0][0] = colour;
    }
    let last = rows - 1;
    if free(g, last, cols - 1, cols - 2) {
        g[last][cols - 1] = colour;
    } else if free(g, last, 0, 1) {
        g[last][0] = colour;
    }
}

/// The glyphs of a time in big digits, and their width with a tile between each.
fn big_glyphs(time: &str) -> (Vec<char>, usize) {
    let glyphs: Vec<char> = time.chars().filter(|c| c.is_ascii_digit() || *c == ':').collect();
    let width = glyphs.iter().map(|c| if *c == ':' { 1 } else { 3 }).sum::<usize>() + glyphs.len().saturating_sub(1);
    (glyphs, width)
}

/// Draws big digits with their top-left at (top, left); digit `i` (colons not counted) in
/// `ink(i)`, colons in FILLED.
fn draw_digits(g: &mut Grid, glyphs: &[char], top: usize, left: usize, ink: impl Fn(usize) -> u8) {
    let mut c = left;
    let mut i = 0;
    for &ch in glyphs {
        if ch == ':' {
            g[top + 1][c] = FILLED;
            g[top + 3][c] = FILLED;
            c += 2;
            continue;
        }
        let d = DIGITS[ch.to_digit(10).unwrap_or(0) as usize];
        for (r, line) in d.iter().enumerate() {
            for (k, px) in line.chars().enumerate() {
                if px == '#' {
                    g[top + r][c + k] = ink(i);
                }
            }
        }
        c += 4;
        i += 1;
    }
}

/// The time-of-day palette: dawn 05–07, day 08–16, dusk 17–19, night 20–04.
pub fn daypart_palette(hour: u32) -> &'static [u8] {
    match hour {
        5..=7 => &[64, 63, 65],
        8..=16 => &[65, 67],
        17..=19 => &[68, 64, 63],
        _ => &[67, 68],
    }
}

/// The hour's colour for the hour bar: red, orange, yellow, green, blue, violet, again.
pub fn hour_colour(hour: u32) -> u8 {
    RAINBOW[hour as usize % 6]
}

#[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
fn colour_clock(style: ClockStyle, time: &str, sub: &str, line: &str, date: &str, hour: u32, minute: u32, colour: Colour, accent: u8, rows: usize, cols: usize) -> Grid {
    let mut g = vec![vec![0u8; cols]; rows];
    let (glyphs, w) = big_glyphs(time);
    let text_lines = |n: usize| -> Vec<Vec<u8>> {
        let mut l = vec![clip(parse(line), cols)];
        if n > 1 && !date.is_empty() {
            l.push(clip(parse(date), cols));
        }
        l
    };
    let sub_codes = clip(parse(sub), cols);
    match style {
        ClockStyle::Colour => {
            // The bar: the last row, filled as far as the hour has gone.
            if rows >= 2 {
                let n = (minute as usize * cols / 60 + 1).min(cols);
                for c in 0..n {
                    g[rows - 1][c] = colour.ink(hour_colour(hour), accent, true);
                }
            }
            let area = if rows >= 2 { rows - 1 } else { rows };
            if area >= 5 && w <= cols && !glyphs.is_empty() {
                let with_sub = area >= 7 && !sub_codes.is_empty();
                let h = if with_sub { 6 } else { 5 };
                let top = (area - h) / 2;
                draw_digits(&mut g, &glyphs, top, (cols - w) / 2, |_| FILLED);
                if with_sub {
                    put(&mut g, top + 5, (cols - sub_codes.len()) / 2, &sub_codes);
                }
            } else {
                for (r, row) in place(&text_lines(area), area, cols, false).into_iter().enumerate() {
                    g[r] = row;
                }
            }
        }
        ClockStyle::Daypart => {
            let pal = daypart_palette(hour);
            let digits = rows >= 5 && w <= cols && !glyphs.is_empty();
            let (cw, ch) = if digits {
                (w.max(sub_codes.len()), if rows >= 6 && !sub_codes.is_empty() { 6 } else { 5 })
            } else {
                let l = text_lines(rows);
                (l.iter().map(|x| x.len()).max().unwrap_or(0), l.len())
            };
            let side = cols >= cw + 2;
            let band = rows >= ch + 2;
            let (r0, r1) = if band { (1, rows - 1) } else { (0, rows) };
            let (c0, c1) = if side { (1, cols - 1) } else { (0, cols) };
            for r in 0..rows {
                for c in 0..cols {
                    let edge = (band && (r == 0 || r == rows - 1)) || (side && (c == 0 || c == cols - 1));
                    if edge {
                        g[r][c] = colour.ink(pal[(r + c) % pal.len()], accent, false);
                    }
                }
            }
            let (ih, iw) = (r1 - r0, c1 - c0);
            if digits && ih >= 5 && iw >= w {
                let with_sub = ih >= 6 && !sub_codes.is_empty();
                let top = r0 + (ih - if with_sub { 6 } else { 5 }) / 2;
                draw_digits(&mut g, &glyphs, top, c0 + (iw - w) / 2, |_| FILLED);
                if with_sub {
                    let s = clip(parse(sub), iw);
                    put(&mut g, top + 5, c0 + (iw - s.len()) / 2, &s);
                }
            } else {
                let lines: Vec<Vec<u8>> = text_lines(ih).into_iter().map(|l| clip(l, iw)).collect();
                for (i, row) in place(&lines, ih, iw, false).into_iter().enumerate() {
                    put(&mut g, r0 + i, c0, &row);
                }
            }
        }
        ClockStyle::Playful => {
            let walk = |c: usize| RAINBOW[(c + minute as usize) % 6];
            if rows >= 5 && w <= cols && !glyphs.is_empty() {
                let with_sub = rows >= 6 && !sub_codes.is_empty();
                let h = if with_sub { 6 } else { 5 };
                let bands = rows >= h + 2;
                let top = (rows - h) / 2;
                draw_digits(&mut g, &glyphs, top, (cols - w) / 2, |i| colour.ink(walk(i), accent, true));
                if with_sub {
                    put(&mut g, top + 5, (cols - sub_codes.len()) / 2, &sub_codes);
                }
                if bands {
                    for c in 0..cols {
                        g[0][c] = colour.ink(walk(c), accent, false);
                        g[rows - 1][c] = colour.ink(walk(cols - 1 - c), accent, false);
                    }
                }
            } else {
                // Too small for digits: the time and date over a rainbow row that walks.
                let area = if rows >= 2 { rows - 1 } else { rows };
                for (r, row) in place(&text_lines(area), area, cols, false).into_iter().enumerate() {
                    g[r] = row;
                }
                if rows >= 2 {
                    for c in 0..cols {
                        g[rows - 1][c] = colour.ink(walk(c), accent, false);
                    }
                }
            }
        }
    }
    g
}

/// A countdown: its text above, a progress bar on the last row (Playful: a rainbow from red
/// at the left to violet at the right, revealed as the day comes closer).
fn countdown_card(text: &str, chip: Option<u8>, permille: u16, colour: Colour, accent: u8, rows: usize, cols: usize) -> Grid {
    if rows < 2 {
        return text_pages(text, false, chip, 1, rows, cols).remove(0);
    }
    let mut g = text_pages(text, false, chip, 1, rows - 1, cols).into_iter().next().unwrap_or_else(|| vec![vec![0u8; cols]; rows - 1]);
    let n = (permille.min(1000) as usize * cols / 1000).min(cols);
    let mut bar = vec![0u8; cols];
    for (c, t) in bar.iter_mut().enumerate().take(n) {
        *t = colour.ink(RAINBOW[c * 6 / cols], accent, true);
    }
    g.push(bar);
    g
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

fn quote_card(quotes: &[(String, String)], day: i64, chip: Option<u8>, accents: &[u8], rows: usize, cols: usize) -> Grid {
    let Some(picked) = pick_quote(quotes, day, rows, cols) else {
        return vec![vec![0; cols]; rows];
    };
    let (text, author) = picked;
    let accent = quotes.iter().position(|q| std::ptr::eq(q, picked)).and_then(|i| accents.get(i).copied());
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
    match accent {
        Some(a) => corners(&mut g, a),
        None => {
            if let Some(c) = chip {
                if g[0][0] == 0 {
                    g[0][0] = c;
                }
            }
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
        let g = Face::Celebrate { text: "Happy birthday Sam".into(), colour: Colour::Playful, accent: 68, frame: Some(0) }.render(6, 22).remove(0);
        assert_eq!((g[0][0], g[5][21]), (63, 63));
        assert!(text(&g[2]).contains("HAPPY BIRTHDAY SAM") || text(&g[3]).contains("HAPPY BIRTHDAY SAM"));
        let g = Face::Celebrate { text: "Merry Christmas".into(), colour: Colour::Playful, accent: 68, frame: Some(0) }.render(2, 8).remove(0);
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
        let g = Face::Quote { quotes: list.clone(), day: 1, chip: Some(67), accents: vec![] }.render(6, 22).remove(0);
        assert_eq!(text(&g[5]).trim_end(), format!("{:>21}", "- MARCUS AURELIUS"));
        // a full first row leaves no room for the chip
        assert_ne!(g[0][0], 67);
        let g = Face::Quote { quotes: list, day: 0, chip: Some(67), accents: vec![] }.render(3, 15).remove(0);
        assert_eq!(g[0][0], 67);
        assert_eq!(text(&g[2]), "   - ARISTOTLE ");
        assert!(text(&g[0]).contains("WELL BEGUN") && text(&g[1]).contains("HALF DONE"));
    }
}
