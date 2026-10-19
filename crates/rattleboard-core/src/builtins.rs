//! The built-in plugins drawn by native code: clock, countdown, weather, message of the day
//! and quote of the day. (RSS, calendar and JSON URL are plain manifests; see `plugins.rs`.)

use crate::cards::{ClockStyle, Colour, Face, FILLED, RAINBOW};
use crate::plugins::{self, format_time, Data, Item};
use chrono::{Datelike, Duration as Days, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde_json::{Map, Value};
use std::sync::OnceLock;

pub const CLOCK: &str = "rattleboard.clock";
pub const COUNTDOWN: &str = "rattleboard.countdown";
pub const WEATHER: &str = "rattleboard.weather";
pub const MOTD: &str = "rattleboard.motd";
pub const QUOTE: &str = "rattleboard.quote";

fn text<'a>(s: &'a Map<String, Value>, k: &str) -> &'a str {
    s.get(k).and_then(|v| v.as_str()).unwrap_or("")
}

fn flag(s: &Map<String, Value>, k: &str) -> bool {
    s.get(k).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn lines(s: &Map<String, Value>, k: &str) -> Vec<String> {
    s.get(k).and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|l| l.as_str()).map(str::to_string).collect()).unwrap_or_default()
}

/// Days since 2000-01-01, by the local date: the same quote and message on every board each day.
pub fn day_index(now: NaiveDateTime) -> i64 {
    (now.date() - NaiveDate::from_ymd_opt(2000, 1, 1).unwrap()).num_days()
}

/// A built-in's "Colour" setting: playful (the default), accent or off.
pub fn colour(settings: &Map<String, Value>) -> Colour {
    Colour::parse(text(settings, "colour"))
}

/// Settings saved by an older version, brought up to date before they're checked: quote of
/// the day's "Colour chip" toggle, turned off, becomes Colour: Off.
pub fn migrate(plugin: &str, given: &mut Map<String, Value>) {
    if plugin == QUOTE && !given.contains_key("colour") && given.get("accent") == Some(&Value::Bool(false)) {
        given.insert("colour".into(), Value::String("off".into()));
    }
}

fn time_short(t: NaiveDateTime, clock24: bool) -> String {
    format_time(t, if clock24 { "HH:mm" } else { "h:mm" })
}

// ---------------------------------------------------------------- clock

/// What the clock shows. `zones` are the world clock's cities, as looked up (title: city,
/// body: time zone).
pub fn clock(settings: &Map<String, Value>, now: NaiveDateTime, now_ms: i64, clock24: bool, chip: Option<u8>, zones: Option<&Data>) -> Face {
    let ampm = if now.hour() < 12 { "AM" } else { "PM" };
    let colour = colour(settings);
    let chip = if colour == Colour::Off { None } else { chip };
    let style = match text(settings, "style") {
        "colour" => Some(ClockStyle::Colour),
        "daypart" => Some(ClockStyle::Daypart),
        "playful" => Some(ClockStyle::Playful),
        _ => None,
    };
    if let Some(style) = style {
        let time = time_short(now, clock24);
        let date = format_time(now, "ddd d MMM");
        return Face::Clock {
            style,
            sub: if clock24 { date.clone() } else { format!("{ampm} {date}") },
            line: if clock24 { time.clone() } else { format!("{time} {ampm}") },
            time,
            date,
            hour: now.hour(),
            minute: now.minute(),
            colour,
            accent: chip.unwrap_or(69),
        };
    }
    match text(settings, "style") {
        "date" => Face::Text {
            text: format!("{}{}\n{}", time_short(now, clock24), if clock24 { String::new() } else { format!(" {ampm}") }, format_time(now, "dddd d MMMM")),
            left: false,
            chip,
            pages: 1,
        },
        "world" => {
            let rows = zones
                .map(|d| d.items.iter().take(3).filter_map(|i| {
                    let tz: chrono_tz::Tz = i.body.parse().ok()?;
                    use chrono::TimeZone;
                    let t = tz.timestamp_millis_opt(now_ms).single()?.naive_local();
                    let (long, short) = if clock24 {
                        (format_time(t, "HH:mm"), format_time(t, "HH:mm"))
                    } else {
                        (format_time(t, "h:mm tt"), format_time(t, "h:mm") + if t.hour() < 12 { "A" } else { "P" })
                    };
                    Some((i.title.clone(), long, short))
                }).collect())
                .unwrap_or_default();
            Face::Rows { chip: if colour == Colour::Off { 0 } else { chip.unwrap_or(69) }, title: "World clock".into(), rows }
        }
        _ => {
            let sub = if clock24 { format_time(now, "ddd d MMM") } else { format!("{ampm} {}", format_time(now, "ddd d MMM")) };
            Face::BigTime { time: time_short(now, clock24), sub, fill: chip.unwrap_or(FILLED) }
        }
    }
}

/// The world clock's cities, at most three, comma-separated.
pub fn cities(settings: &Map<String, Value>) -> Vec<String> {
    text(settings, "cities").split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).take(3).collect()
}

// ---------------------------------------------------------------- countdown

#[derive(Debug, PartialEq)]
pub struct Countdown {
    pub face: Face,
    /// Changes by the minute or hour from here: re-drawn while it's on the board
    pub live: bool,
    /// On the day: the celebration (dedupe key and card), for the alert
    pub celebrating: Option<(String, Face)>,
}

/// When the countdown ends: the next occurrence for a yearly one. Whole-day countdowns
/// end at midnight starting the day; the day itself is the celebration.
fn target(settings: &Map<String, Value>, now: NaiveDateTime) -> Option<(NaiveDateTime, bool, String, bool)> {
    let preset = text(settings, "preset");
    let (date, time, yearly, name) = match preset {
        "nye" => (NaiveDate::from_ymd_opt(now.year() + 1, 1, 1)?, Some(NaiveTime::MIN), true, "New Year".to_string()),
        "christmas" => (NaiveDate::from_ymd_opt(now.year(), 12, 25)?, None, true, "Christmas".to_string()),
        "halloween" => (NaiveDate::from_ymd_opt(now.year(), 10, 31)?, None, true, "Halloween".to_string()),
        _ => {
            let d = NaiveDate::parse_from_str(text(settings, "date"), "%Y-%m-%d").ok()?;
            let t = plugins::minutes(text(settings, "time")).and_then(|m| NaiveTime::from_hms_opt(m / 60, m % 60, 0));
            (d, t, flag(settings, "yearly"), String::new())
        }
    };
    let title = Some(text(settings, "title").trim().to_string()).filter(|t| !t.is_empty()).unwrap_or(name);
    let at = |d: NaiveDate| d.and_time(time.unwrap_or(NaiveTime::MIN));
    // The celebration lasts until the end of the target's day.
    let over = |d: NaiveDate| now.date() > d;
    let mut d = date;
    if yearly {
        let (m, day) = (date.month(), date.day());
        let in_year = |y: i32| NaiveDate::from_ymd_opt(y, m, day).or_else(|| NaiveDate::from_ymd_opt(y, m, 28));
        d = in_year(now.year())?;
        if over(d) {
            d = in_year(now.year() + 1)?;
        }
        if preset == "nye" {
            // The new year's first day is the celebration; after it, count to the next one.
            let jan1 = NaiveDate::from_ymd_opt(now.year(), 1, 1)?;
            d = if now.date() == jan1 { jan1 } else { NaiveDate::from_ymd_opt(now.year() + 1, 1, 1)? };
        }
    } else if over(d) {
        return None;
    }
    Some((at(d), time.is_some(), title, yearly))
}

pub fn celebration_text(settings: &Map<String, Value>, title: &str, year: i32) -> String {
    let custom = text(settings, "celebration").trim();
    if !custom.is_empty() {
        return custom.to_string();
    }
    let lower = title.to_lowercase();
    match text(settings, "preset") {
        "nye" => return format!("Happy New Year {year}"),
        "christmas" => return "Merry Christmas".into(),
        "halloween" => return "Happy Halloween".into(),
        _ => {}
    }
    for (suffix, wish) in [("'s birthday", "Happy birthday"), ("’s birthday", "Happy birthday"), ("'s anniversary", "Happy anniversary")] {
        if let Some(who) = lower.strip_suffix(suffix) {
            return format!("{wish} {}", &title[..who.len()]);
        }
    }
    if lower.contains("birthday") {
        "Happy birthday".into()
    } else if lower.contains("anniversary") {
        "Happy anniversary".into()
    } else {
        format!("Today: {title}")
    }
}

/// "12 DAYS TO SAM'S BIRTHDAY"; on the last day hours, in the final hour minutes (live, if
/// the ticker is on); on the day, the celebration. None once a one-off countdown is over.
pub fn countdown(settings: &Map<String, Value>, now: NaiveDateTime, chip: Option<u8>) -> Option<Countdown> {
    let (at, _, title, yearly) = target(settings, now)?;
    let colour = colour(settings);
    let accent = chip.unwrap_or(68);
    let chip = if colour == Colour::Off { None } else { Some(accent) };
    if title.is_empty() {
        let face = Face::Text { text: "Name this countdown in Settings → Sources".into(), left: false, chip, pages: 1 };
        return Some(Countdown { face, live: false, celebrating: None });
    }
    if now >= at {
        let wish = celebration_text(settings, &title, at.year());
        let key = format!("celebrate-{}", at.date());
        // In the rotation, a frame a minute; as the alert, every frame in turn.
        let card = Face::Celebrate { text: wish.clone(), colour, accent, frame: Some(now.minute() as usize) };
        let alert = Face::Celebrate { text: wish, colour, accent, frame: None };
        let celebrating = flag(settings, "celebrate").then_some((key, alert));
        return Some(Countdown { face: card, live: true, celebrating });
    }
    let permille = progress(at, yearly, now);
    let face = |text: String| Face::Countdown { text, chip, permille, colour, accent };
    let left = at - now;
    let hours = left.num_hours();
    let days = (at.date() - now.date()).num_days();
    let to = format!("to {title}");
    let (text, live) = if hours >= 48 {
        (format!("{days} days {to}"), false)
    } else if hours >= 24 {
        let h = hours - 24;
        (if h == 0 { format!("1 day {to}") } else { format!("1 day {h} hr{} {to}", if h == 1 { "" } else { "s" }) }, true)
    } else if hours >= 1 {
        (format!("{hours} hr{} {to}", if hours == 1 { "" } else { "s" }), true)
    } else if flag(settings, "ticker") {
        let m = (left.num_seconds() + 59) / 60;
        (format!("{m} min {to}"), true)
    } else {
        (format!("Under an hour {to}"), true)
    };
    Some(Countdown { face: face(text), live, celebrating: None })
}

/// How far along the wait is, in thousandths: a yearly countdown from the last occurrence
/// (a year before), a one-off over its last 30 days. Whole minutes.
pub fn progress(at: NaiveDateTime, yearly: bool, now: NaiveDateTime) -> u16 {
    let start = if yearly {
        let d = at.date();
        NaiveDate::from_ymd_opt(d.year() - 1, d.month(), d.day()).or_else(|| NaiveDate::from_ymd_opt(d.year() - 1, d.month(), 28)).map(|p| p.and_time(at.time())).unwrap_or(at - Days::days(365))
    } else {
        at - Days::days(30)
    };
    let total = (at - start).num_minutes().max(1);
    let done = (now - start).num_minutes().clamp(0, total);
    (done * 1000 / total) as u16
}

/// What to call a source of a built-in plugin left with its default name: a countdown by
/// what it counts to, the weather by its city.
pub fn label(plugin: &str, settings: &Map<String, Value>) -> Option<String> {
    let t = |k: &str| Some(text(settings, k).trim().to_string()).filter(|s| !s.is_empty());
    match plugin {
        COUNTDOWN => t("title").or_else(|| match text(settings, "preset") {
            "nye" => Some("New Year".into()),
            "christmas" => Some("Christmas".into()),
            "halloween" => Some("Halloween".into()),
            _ => None,
        }),
        WEATHER => t("city").map(|c| format!("Weather · {c}")),
        _ => None,
    }
}

/// When the countdown next changes what it says, for Up next's celebration entry.
pub fn countdown_target(settings: &Map<String, Value>, now: NaiveDateTime) -> Option<NaiveDateTime> {
    target(settings, now).map(|t| t.0)
}

// ---------------------------------------------------------------- weather (Open-Meteo)

/// Looks a city up with Open-Meteo's geocoding: its name, position and time zone.
pub async fn geocode(c: &reqwest::Client, city: &str) -> Result<(String, f64, f64, String), String> {
    let url = format!("https://geocoding-api.open-meteo.com/v1/search?count=1&language=en&format=json&name={}", url::form_urlencoded::byte_serialize(city.as_bytes()).collect::<String>());
    let body = plugins::fetch(c, "GET", &url, &[]).await?;
    let v: Value = serde_json::from_slice(&body).map_err(|_| "Unexpected answer from the geocoding service".to_string())?;
    let r = v["results"].get(0).ok_or(format!("Couldn't find a place called {city}"))?;
    let (lat, lon) = (r["latitude"].as_f64().unwrap_or(0.0), r["longitude"].as_f64().unwrap_or(0.0));
    Ok((plugins::clean(r["name"].as_str().unwrap_or(city)), lat, lon, r["timezone"].as_str().unwrap_or("UTC").to_string()))
}

/// The world clock's time zones. A city can also be given as a zone, like Europe/London.
pub async fn zones(c: &reqwest::Client, cities: &[String]) -> Result<Data, String> {
    let mut items = vec![];
    for city in cities {
        if city.contains('/') && city.parse::<chrono_tz::Tz>().is_ok() {
            let name = city.rsplit('/').next().unwrap_or(city).replace('_', " ");
            items.push(Item { title: name, body: city.clone(), ..Default::default() });
            continue;
        }
        let (name, _, _, tz) = geocode(c, city).await?;
        items.push(Item { title: name, body: tz, ..Default::default() });
    }
    Ok(Data { items, single: true })
}

/// WMO weather codes as words, and a chip colour for each kind of weather.
fn condition(code: i64) -> (&'static str, u8) {
    match code {
        0 => ("Clear", 65),
        1 => ("Mostly clear", 65),
        2 => ("Partly cloudy", 69),
        3 => ("Overcast", 69),
        45 | 48 => ("Fog", 69),
        51..=57 => ("Drizzle", 67),
        61 | 63 | 80 | 81 => ("Rain", 67),
        65 | 82 => ("Heavy rain", 67),
        66 | 67 => ("Freezing rain", 67),
        71..=77 | 85 | 86 => ("Snow", 69),
        95..=99 => ("Thunderstorm", 68),
        _ => ("", 69),
    }
}

pub async fn weather(c: &reqwest::Client, settings: &Map<String, Value>) -> Result<Data, String> {
    let city = Some(text(settings, "city").trim()).filter(|c| !c.is_empty()).unwrap_or("Melbourne");
    let (name, lat, lon, _) = geocode(c, city).await?;
    let f = text(settings, "units") == "fahrenheit";
    let units = if f { "&temperature_unit=fahrenheit&wind_speed_unit=mph" } else { "" };
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.4}&longitude={lon:.4}&current=temperature_2m,weather_code,wind_speed_10m&daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max&timezone=auto&forecast_days=1{units}"
    );
    let body = plugins::fetch(c, "GET", &url, &[]).await?;
    let v: Value = serde_json::from_slice(&body).map_err(|_| "Unexpected answer from the weather service".to_string())?;
    weather_item(&name, &v, f).ok_or("Unexpected answer from the weather service".into()).map(|i| Data { items: vec![i], single: true })
}

/// One forecast as a card's text: now on the first line, the day's range and rain below.
pub fn weather_item(name: &str, v: &Value, fahrenheit: bool) -> Option<Item> {
    let cur = &v["current"];
    let t = cur["temperature_2m"].as_f64()?;
    let (words, chip) = condition(cur["weather_code"].as_i64().unwrap_or(-1));
    let d = &v["daily"];
    let hi = d["temperature_2m_max"].get(0).and_then(|x| x.as_f64());
    let lo = d["temperature_2m_min"].get(0).and_then(|x| x.as_f64());
    let rain = d["precipitation_probability_max"].get(0).and_then(|x| x.as_f64());
    let wind = cur["wind_speed_10m"].as_f64();
    let mut lines = vec![format!("{}° {words}", t.round())];
    if let (Some(h), Some(l)) = (hi, lo) {
        lines.push(format!("High {}° Low {}°", h.round(), l.round()));
    }
    let mut extra = vec![];
    if let Some(r) = rain {
        extra.push(format!("Rain {}%", r.round()));
    }
    if let Some(w) = wind {
        extra.push(format!("Wind {} {}", w.round(), if fahrenheit { "mph" } else { "km/h" }));
    }
    if !extra.is_empty() {
        lines.push(extra.join("  "));
    }
    let c = if fahrenheit { (t - 32.0) * 5.0 / 9.0 } else { t };
    let fields = [("temp_c".to_string(), format!("{}", c.round() as i64))].into_iter().collect();
    Some(Item { key: plugins::hash(&[name, &lines.join("\n")]), title: name.to_string(), body: lines.join("\n"), chip: Some(chip), fields, ..Default::default() })
}

/// The temperature scale (°C, rounded): ≤0 violet, 1–9 blue, 10–17 green, 18–24 yellow,
/// 25–31 orange, ≥32 red.
pub fn temp_colour(c: i64) -> u8 {
    match c {
        i64::MIN..=0 => 68,
        1..=9 => 67,
        10..=17 => 66,
        18..=24 => 65,
        25..=31 => 64,
        _ => 63,
    }
}

/// The weather card: the standard card with the condition's colour as its chip; Playful also
/// puts the temperature's colour before it. `chip` is the source's own chip, if set.
pub fn weather_face(it: &Item, name: String, time: String, colour: Colour, chip: Option<u8>) -> Face {
    let chip = if colour == Colour::Off { 0 } else { chip.or(it.chip).unwrap_or(0) };
    let mut body = it.body.clone();
    if colour == Colour::Playful {
        if let Some(c) = it.fields.get("temp_c").and_then(|c| c.parse::<i64>().ok()) {
            body = format!("{{{}}} {body}", temp_colour(c));
        }
    }
    Face::Standard { chip, name, time, body, pages: 1 }
}

// ---------------------------------------------------------------- message of the day

pub fn motd(settings: &Map<String, Value>, now: NaiveDateTime, chip: Option<u8>) -> Option<Face> {
    let list = lines(settings, "lines");
    if list.is_empty() {
        return None;
    }
    let day = day_index(now);
    let i = day.rem_euclid(list.len() as i64) as usize;
    Some(match colour(settings) {
        Colour::Playful => Face::Corners { text: list[i].clone(), colour: RAINBOW[day.rem_euclid(6) as usize], pages: 2 },
        Colour::Accent => Face::Text { text: list[i].clone(), left: false, chip: chip.or(Some(66)), pages: 2 },
        Colour::Off => Face::Text { text: list[i].clone(), left: false, chip: None, pages: 2 },
    })
}

// ---------------------------------------------------------------- quote of the day

#[derive(serde::Deserialize)]
struct QuoteFile {
    format: u32,
    quotes: Vec<QuoteEntry>,
}

#[derive(serde::Deserialize, Clone)]
pub struct QuoteEntry {
    pub text: String,
    pub author: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// The bundled quotes: public domain, the same file on every platform (data/quotes.json).
pub fn bundled_quotes() -> &'static [QuoteEntry] {
    static QUOTES: OnceLock<Vec<QuoteEntry>> = OnceLock::new();
    QUOTES.get_or_init(|| {
        let f: QuoteFile = serde_json::from_str(include_str!("../../../data/quotes.json")).expect("data/quotes.json");
        assert_eq!(f.format, 1, "data/quotes.json format");
        f.quotes
    })
}

/// "Text - Author" (or an em dash) from a "My quotes" line.
pub fn own_quote(line: &str) -> (String, String) {
    for sep in [" — ", " – ", " - ", " ~ "] {
        if let Some((t, a)) = line.rsplit_once(sep) {
            if !a.trim().is_empty() && a.trim().chars().count() <= 30 {
                return (clean_quote(t), plugins::clean(a.trim()));
            }
        }
    }
    (clean_quote(line), String::new())
}

fn clean_quote(t: &str) -> String {
    plugins::clean(t.trim()).trim_matches('"').trim().to_string()
}

/// The quotes this instance picks from: the bundled ones (by tag) and/or the user's own.
pub fn quote_list(settings: &Map<String, Value>) -> Vec<(String, String)> {
    quote_entries(settings).into_iter().map(|q| (q.0, q.1)).collect()
}

/// A tag's colour: work and books blue, calm and nature green, funny orange, wisdom violet,
/// time and friendship yellow, courage and love red.
pub fn tag_colour(tag: &str) -> Option<u8> {
    Some(match tag {
        "work" | "books" => 67,
        "calm" | "nature" => 66,
        "funny" => 64,
        "wisdom" => 68,
        "time" | "friendship" => 65,
        "courage" | "love" => 63,
        _ => return None,
    })
}

/// The quotes with each one's colour (its first tag with a colour), if it has one.
fn quote_entries(settings: &Map<String, Value>) -> Vec<(String, String, Option<u8>)> {
    let tag = text(settings, "tags");
    let source = text(settings, "source");
    let mut out = vec![];
    if source != "mine" {
        out.extend(bundled_quotes().iter().filter(|q| tag.is_empty() || q.tags.iter().any(|t| t == tag)).map(|q| (q.text.clone(), q.author.clone(), q.tags.iter().find_map(|t| tag_colour(t)))));
    }
    if source != "bundled" {
        out.extend(lines(settings, "mine").iter().map(|l| own_quote(l)).filter(|q| !q.0.is_empty()).map(|q| (q.0, q.1, None)));
    }
    out
}

pub fn quote(settings: &Map<String, Value>, now: NaiveDateTime, chip: Option<u8>) -> Option<Face> {
    let entries = quote_entries(settings);
    if entries.is_empty() {
        return None;
    }
    let day = day_index(now);
    let today = RAINBOW[day.rem_euclid(6) as usize];
    let (chip, accents) = match colour(settings) {
        Colour::Playful => (None, entries.iter().map(|q| q.2.unwrap_or(today)).collect()),
        Colour::Accent => (chip.or(Some(67)), vec![]),
        Colour::Off => (None, vec![]),
    };
    let quotes = entries.into_iter().map(|q| (q.0, q.1)).collect();
    Some(Face::Quote { quotes, day, chip, accents })
}

/// Once-a-day quotes: when today's is due (local time), else None.
pub fn quote_due(settings: &Map<String, Value>, now: NaiveDateTime) -> Option<String> {
    if text(settings, "mode") != "daily" {
        return None;
    }
    let at = plugins::minutes(text(settings, "at")).unwrap_or(8 * 60);
    (now.hour() * 60 + now.minute() >= at).then(|| format!("quote-{}", now.date()))
}

/// When the next once-a-day quote is due, for Up next.
pub fn quote_next(settings: &Map<String, Value>, now: NaiveDateTime) -> Option<NaiveDateTime> {
    if text(settings, "mode") != "daily" {
        return None;
    }
    let at = plugins::minutes(text(settings, "at")).unwrap_or(8 * 60);
    let today = now.date().and_hms_opt(at / 60, at % 60, 0)?;
    Some(if today > now { today } else { today + Days::days(1) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::{fitting, pick_quote, quote_fits};
    use serde_json::json;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
    }

    fn set(v: Value) -> Map<String, Value> {
        serde_json::from_value(v).unwrap()
    }

    fn says(c: &Countdown) -> String {
        match &c.face {
            Face::Text { text, .. } | Face::Celebrate { text, .. } | Face::Countdown { text, .. } => text.clone(),
            f => format!("{f:?}"),
        }
    }

    /// Every built-in style and colour level, as the spec shows them: (title, face) pairs.
    pub fn gallery() -> Vec<(String, Face)> {
        let mut out = vec![];
        let now = at(2026, 10, 18, 21, 41);
        for style in ["colour", "daypart", "playful", "big", "date"] {
            for colour in ["playful", "accent", "off"] {
                out.push((format!("Clock · style {style} · colour {colour} · Sun 18 Oct 9:41 PM"), clock(&set(json!({"style": style, "colour": colour})), now, 0, false, Some(69), None)));
            }
        }
        out.push(("Clock · style colour · 7:05 AM (hour 7: green bar, 2 tiles at :05 on 22 cols)".into(), clock(&set(json!({"style": "colour"})), at(2026, 10, 19, 7, 5), 0, false, Some(69), None)));
        for (h, m, when) in [(6, 30, "dawn"), (12, 0, "day"), (18, 15, "dusk"), (23, 50, "night")] {
            out.push((format!("Clock · style daypart · {when} {h:02}:{m:02} (24-hour)"), clock(&set(json!({"style": "daypart"})), at(2026, 10, 19, h, m), 0, true, Some(69), None)));
        }
        out.push(("Clock · style playful · 9:42 PM (one minute later: the rainbow steps along)".into(), clock(&set(json!({"style": "playful"})), at(2026, 10, 18, 21, 42), 0, false, Some(69), None)));
        let bday = json!({"title": "Sam's birthday", "date": "1990-10-30", "yearly": true, "celebrate": true});
        for colour in ["playful", "accent", "off"] {
            let mut s = bday.clone();
            s["colour"] = json!(colour);
            out.push((format!("Countdown · colour {colour} · 18 Oct, 12 days to go"), countdown(&set(s.clone()), at(2026, 10, 18, 9, 0), None).unwrap().face));
            let c = countdown(&set(s), at(2026, 10, 30, 7, 0), None).unwrap();
            out.push((format!("Countdown · colour {colour} · on the day, in the rotation (07:00, frame 0 of 3)"), c.face));
            out.push((format!("Countdown · colour {colour} · on the day, as the alert (3 frames, one page each)"), c.celebrating.unwrap().1));
        }
        let wx = |code: i64, t: f64| json!({"current": {"temperature_2m": t, "weather_code": code, "wind_speed_10m": 14.2},
            "daily": {"temperature_2m_max": [t + 4.0], "temperature_2m_min": [t - 6.0], "precipitation_probability_max": [40]}});
        for (code, t, what) in [(0, 27.0, "clear, 27°: yellow chip, orange tile"), (63, 12.0, "rain, 12°: blue chip, green tile"), (2, 17.6, "partly cloudy, 18°: white chip, yellow tile"), (95, 33.0, "thunderstorm, 33°: violet chip, red tile"), (71, -1.0, "snow, -1°: white chip, violet tile")] {
            let it = weather_item("Melbourne", &wx(code, t), false).unwrap();
            for colour in ["playful", "accent", "off"] {
                if colour != "playful" && code != 2 {
                    continue;
                }
                out.push((format!("Weather · colour {colour} · {what}"), weather_face(&it, "Melbourne".into(), "9:41".into(), Colour::parse(colour), None)));
            }
        }
        for colour in ["playful", "accent", "off"] {
            out.push((format!("Quote of the day · colour {colour} · 18 Oct 2026"), quote(&set(json!({"colour": colour})), at(2026, 10, 18, 9, 0), None).unwrap()));
        }
        for colour in ["playful", "accent", "off"] {
            out.push((format!("Message of the day · colour {colour} · 18 Oct 2026"), motd(&set(json!({"colour": colour, "lines": ["Tea at four", "Ship it"]})), at(2026, 10, 18, 9, 0), None).unwrap()));
        }
        out
    }

    /// Prints the gallery (cargo test gallery -- --nocapture), and with RB_GALLERY=<file>
    /// writes it as Markdown for the spec.
    #[test]
    fn gallery_renders_everywhere() {
        let mut md = String::new();
        for (title, face) in gallery() {
            md.push_str(&format!("#### {title}\n\n```\n"));
            for (rows, cols, size) in [(6, 22, "Standard 6×22"), (3, 15, "Compact 3×15"), (8, 30, "Large 8×30")] {
                let pages = face.render(rows, cols);
                assert!(!pages.is_empty());
                for (n, g) in pages.iter().enumerate() {
                    assert_eq!((g.len(), g[0].len()), (rows, cols), "{title}");
                    let label = if pages.len() > 1 { format!("{size}, frame {}", n + 1) } else { size.to_string() };
                    md.push_str(&format!("{label}\n"));
                    for l in crate::cards::ascii(g) {
                        md.push_str(&format!("|{l}|\n"));
                    }
                }
            }
            md.push_str("```\n\n");
        }
        println!("{md}");
        if let Ok(p) = std::env::var("RB_GALLERY") {
            std::fs::write(p, md).unwrap();
        }
    }

    fn colours_in(g: &crate::vestaboard::Grid) -> Vec<u8> {
        g.iter().flatten().copied().filter(|c| (63..=70).contains(c)).collect()
    }

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn colourful_clocks() {
        let face = |style: &str, h, m, colour: &str| clock(&set(json!({"style": style, "colour": colour})), at(2026, 10, 19, h, m), 0, false, Some(69), None);
        // The hour bar: minute*cols/60 + 1 tiles in the hour's colour (hour % 6 along the rainbow)
        for (m, n22, n15, n30) in [(0, 1, 1, 1), (5, 2, 2, 3), (30, 12, 8, 16), (59, 22, 15, 30)] {
            for (rows, cols, n) in [(6, 22, n22), (3, 15, n15), (8, 30, n30)] {
                let g = face("colour", 9, m, "playful").render(rows, cols).remove(0);
                let last = &g[rows - 1];
                assert_eq!(last.iter().filter(|&&c| c != 0).count(), n, "{m} min on {rows}x{cols}");
                assert!(last[..n].iter().all(|&c| c == crate::cards::hour_colour(9)) && crate::cards::hour_colour(9) == 66);
            }
        }
        assert_eq!(crate::cards::hour_colour(0), 63);
        assert_eq!(crate::cards::hour_colour(23), 68);
        // Accent: the chip's colour; off: filled tiles
        assert!(face("colour", 9, 30, "accent").render(6, 22)[0][5][..12].iter().all(|&c| c == 69));
        assert!(face("colour", 9, 30, "off").render(6, 22)[0][5][..12].iter().all(|&c| c == FILLED));
        // Playful digits take the next colour each minute
        let digit_colours = |m| {
            let g = face("playful", 9, m, "playful").render(6, 22).remove(0);
            let mut seen = vec![];
            for c in 0..22 {
                for r in 0..5 {
                    let t = g[r][c];
                    if (63..=68).contains(&t) && seen.last() != Some(&t) {
                        seen.push(t);
                    }
                }
            }
            seen
        };
        assert_eq!(digit_colours(41), [68, 63, 64]); // 9:41: (i + 41) % 6 = 5, 0, 1
        assert_eq!(digit_colours(42), [63, 64, 65]);
        // Time of day palettes, and the frame (r + c) % len
        assert_eq!(crate::cards::daypart_palette(6), [64, 63, 65]);
        assert_eq!(crate::cards::daypart_palette(12), [65, 67]);
        assert_eq!(crate::cards::daypart_palette(18), [68, 64, 63]);
        assert_eq!(crate::cards::daypart_palette(2), [67, 68]);
        let g = face("daypart", 12, 0, "playful").render(8, 30).remove(0);
        assert_eq!((g[0][0], g[0][1], g[7][29], g[3][0]), (65, 67, 65, 67));
        // Calm: within a minute nothing changes (seconds play no part)
        for style in ["colour", "daypart", "playful", "big"] {
            let a = clock(&set(json!({"style": style})), at(2026, 10, 19, 9, 41), 0, false, Some(69), None);
            let b = clock(&set(json!({"style": style})), NaiveDate::from_ymd_opt(2026, 10, 19).unwrap().and_hms_opt(9, 41, 59).unwrap(), 0, false, Some(69), None);
            assert_eq!(a, b, "{style}");
        }
    }

    #[test]
    fn colour_off_means_no_colour() {
        for (title, face) in gallery() {
            if !title.contains("colour off") {
                continue;
            }
            for (rows, cols) in [(6, 22), (3, 15), (8, 30)] {
                for g in face.render(rows, cols) {
                    assert!(colours_in(&g).is_empty(), "{title} {rows}x{cols}: {:?}", crate::cards::ascii(&g));
                }
            }
        }
        // ...and playful ones have some, on every size (the plain Big time style aside: on a
        // Compact board it's text)
        for (title, face) in gallery() {
            if title.contains("colour playful") && !title.contains("style big") {
                for (rows, cols) in [(6, 22), (3, 15), (8, 30)] {
                    assert!(face.render(rows, cols).iter().any(|g| !colours_in(g).is_empty()), "{title} {rows}x{cols}");
                }
            }
        }
    }

    #[test]
    fn countdown_progress_and_confetti() {
        // Yearly: from the last birthday (2025-10-30) to the next; 353 days 9 hours of 365 days
        let bday = set(json!({"title": "Sam's birthday", "date": "1990-10-30", "yearly": true}));
        let c = countdown(&bday, at(2026, 10, 18, 9, 0), None).unwrap();
        let Face::Countdown { permille, .. } = c.face else { panic!() };
        assert_eq!(permille, 968);
        let g = c.face.render(6, 22).remove(0);
        assert_eq!(g[5].iter().filter(|&&t| t != 0).count(), 21); // 968 * 22 / 1000
        assert_eq!((g[5][0], g[5][4], g[5][20]), (63, 64, 68)); // RAINBOW[c * 6 / cols]
        // A one-off fills over its last 30 days
        let launch = set(json!({"title": "Launch", "date": "2026-11-02"}));
        let p = |d, h| match countdown(&launch, at(2026, 10, d, h, 0), None).unwrap().face { Face::Countdown { permille, .. } => permille, f => panic!("{f:?}") };
        assert_eq!(p(1, 0), 0);
        assert_eq!(p(18, 0), 500);
        assert_eq!(progress(at(2026, 11, 2, 0, 0), false, at(2026, 11, 1, 23, 59)), 999);
        // The celebration: frames as pages for the alert, one a minute in the rotation
        let mut bday = bday;
        bday.insert("celebrate".into(), json!(true));
        let day = countdown(&bday, at(2026, 10, 30, 7, 4), None).unwrap();
        assert!(matches!(day.face, Face::Celebrate { frame: Some(4), .. }));
        assert_eq!(day.face.render(6, 22).len(), 1);
        let alert = day.celebrating.clone().unwrap().1;
        let frames = alert.render(6, 22);
        assert_eq!(frames.len(), crate::cards::CELEBRATE_FRAMES);
        assert_ne!(frames[0], frames[1]);
        assert_eq!(day.face.render(6, 22)[0], frames[4 % 3]);
        // Confetti never touches a word: two blank tiles each side on its row
        for g in &frames {
            for r in 1..5 {
                for c in 0..22 {
                    if (63..=68).contains(&g[r][c]) {
                        for x in c.saturating_sub(2)..(c + 3).min(22) {
                            assert!(x == c || !(1..=62).contains(&g[r][x]), "{:?}", crate::cards::ascii(g));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn weather_and_quote_colours() {
        for (c, want) in [(-5, 68), (0, 68), (1, 67), (9, 67), (10, 66), (17, 66), (18, 65), (24, 65), (25, 64), (31, 64), (32, 63)] {
            assert_eq!(temp_colour(c), want, "{c}");
        }
        // Fahrenheit is scaled to °C first: 81 °F is 27 °C, orange
        let v = json!({"current": {"temperature_2m": 81.0, "weather_code": 0}, "daily": {}});
        let it = weather_item("Austin", &v, true).unwrap();
        assert_eq!(it.fields["temp_c"], "27");
        let Face::Standard { chip, body, .. } = weather_face(&it, "Austin".into(), "9:41".into(), Colour::Playful, None) else { panic!() };
        assert_eq!((chip, body.as_str()), (65, "{64} 81° Clear"));
        let Face::Standard { chip, body, .. } = weather_face(&it, "Austin".into(), "9:41".into(), Colour::Off, Some(67)) else { panic!() };
        assert_eq!((chip, body.as_str()), (0, "81° Clear"));
        assert_eq!((tag_colour("work"), tag_colour("funny"), tag_colour("love"), tag_colour("other")), (Some(67), Some(64), Some(63), None));
        // My own quotes (no tags) take the day's rainbow colour
        let mine = set(json!({"source": "mine", "mine": ["Ship it - Sam"]}));
        let Some(Face::Quote { accents, .. }) = quote(&mine, at(2000, 1, 3, 9, 0), None) else { panic!() };
        assert_eq!(accents, [65]); // day 2: RAINBOW[2]
        // An old "Colour chip: off" becomes Colour: Off
        let mut old = set(json!({"accent": false}));
        migrate(QUOTE, &mut old);
        assert_eq!(old["colour"], "off");
        let mut newer = set(json!({"accent": false, "colour": "accent"}));
        migrate(QUOTE, &mut newer);
        assert_eq!(newer["colour"], "accent");
    }

    #[test]
    fn countdowns() {
        let bday = set(json!({"title": "Sam's birthday", "date": "1990-10-30", "yearly": true, "celebrate": true}));
        let c = countdown(&bday, at(2026, 10, 18, 9, 0), None).unwrap();
        assert_eq!((says(&c).as_str(), c.live), ("12 days to Sam's birthday", false));
        assert_eq!(says(&countdown(&bday, at(2026, 10, 28, 9, 0), None).unwrap()), "1 day 15 hrs to Sam's birthday");
        assert_eq!(says(&countdown(&bday, at(2026, 10, 29, 19, 0), None).unwrap()), "5 hrs to Sam's birthday");
        // On the day: the celebration, once (its key), as long as the day lasts
        let c = countdown(&bday, at(2026, 10, 30, 7, 0), None).unwrap();
        assert!(matches!(&c.face, Face::Celebrate { text, .. } if text == "Happy birthday Sam"));
        assert_eq!(c.celebrating.unwrap().0, "celebrate-2026-10-30");
        // ...then it rolls over to next year
        assert_eq!(says(&countdown(&bday, at(2026, 10, 31, 0, 0), None).unwrap()), "364 days to Sam's birthday");

        // New Year's Eve: to midnight, a minute ticker in the last hour
        let nye = set(json!({"preset": "nye", "ticker": true, "celebrate": true}));
        assert_eq!(says(&countdown(&nye, at(2026, 12, 19, 12, 0), None).unwrap()), "13 days to New Year");
        let c = countdown(&nye, at(2026, 12, 31, 23, 18), None).unwrap();
        assert_eq!((says(&c).as_str(), c.live), ("42 min to New Year", true));
        assert!(matches!(&countdown(&nye, at(2027, 1, 1, 0, 0), None).unwrap().face, Face::Celebrate { text, .. } if text == "Happy New Year 2027"));
        assert_eq!(says(&countdown(&nye, at(2027, 1, 2, 0, 0), None).unwrap()), "364 days to New Year");
        let quiet = set(json!({"preset": "nye", "ticker": false}));
        assert_eq!(says(&countdown(&quiet, at(2026, 12, 31, 23, 18), None).unwrap()), "Under an hour to New Year");

        assert_eq!(says(&countdown(&set(json!({"preset": "christmas"})), at(2026, 12, 1, 12, 0), None).unwrap()), "24 days to Christmas");
        assert!(matches!(&countdown(&set(json!({"preset": "halloween"})), at(2026, 10, 31, 12, 0), None).unwrap().face, Face::Celebrate { text, .. } if text == "Happy Halloween"));
        // A one-off with a time; over once its day has passed
        let launch = set(json!({"title": "Launch", "date": "2026-11-02", "time": "09:30"}));
        assert_eq!(says(&countdown(&launch, at(2026, 11, 1, 9, 30), None).unwrap()), "1 day to Launch");
        assert!(countdown(&launch, at(2026, 11, 2, 9, 30), None).unwrap().celebrating.is_none());
        assert!(countdown(&launch, at(2026, 11, 3, 0, 0), None).is_none());
        assert!(countdown(&set(json!({"title": "No date"})), at(2026, 1, 1, 0, 0), None).is_none());
        assert_eq!(celebration_text(&set(json!({"celebration": "Woo!"})), "x", 2026), "Woo!");
        assert_eq!(celebration_text(&set(json!({})), "Our anniversary", 2026), "Happy anniversary");
    }

    #[test]
    fn clocks() {
        let now = at(2026, 10, 18, 21, 41);
        let f = clock(&set(json!({"style": "big"})), now, 0, false, None, None);
        assert_eq!(f, Face::BigTime { time: "9:41".into(), sub: "PM Sun 18 Oct".into(), fill: FILLED });
        let f = clock(&set(json!({"style": "date"})), now, 0, true, Some(66), None);
        assert_eq!(f, Face::Text { text: "21:41\nSunday 18 October".into(), left: false, chip: Some(66), pages: 1 });
        let zones = Data { items: vec![Item { title: "London".into(), body: "Europe/London".into(), ..Default::default() }], single: true };
        let ms = chrono::DateTime::parse_from_rfc3339("2026-10-18T10:41:00Z").unwrap().timestamp_millis();
        let Face::Rows { rows, .. } = clock(&set(json!({"style": "world"})), now, ms, false, None, Some(&zones)) else { panic!() };
        assert_eq!(rows, vec![("London".to_string(), "11:41 AM".to_string(), "11:41A".to_string())]);
        assert_eq!(cities(&set(json!({"cities": "Melbourne, London, , Tokyo, Paris"}))), ["Melbourne", "London", "Tokyo"]);
    }

    #[test]
    fn weather_cards() {
        let v = json!({"current": {"temperature_2m": 17.6, "weather_code": 2, "wind_speed_10m": 14.2},
            "daily": {"temperature_2m_max": [22.1], "temperature_2m_min": [11.8], "precipitation_probability_max": [40]}});
        let i = weather_item("Melbourne", &v, false).unwrap();
        assert_eq!(i.body, "18° Partly cloudy\nHigh 22° Low 12°\nRain 40%  Wind 14 km/h");
        assert_eq!(i.chip, Some(69));
        assert!(weather_item("X", &json!({}), false).is_none());
    }

    #[test]
    fn messages_of_the_day() {
        let s = set(json!({"lines": ["One", "Two", "Three"]}));
        let pick = |d: NaiveDateTime| match motd(&s, d, None).unwrap() { Face::Text { text, .. } | Face::Corners { text, .. } => text, _ => unreachable!() };
        assert_eq!(day_index(at(2000, 1, 1, 12, 0)), 0);
        assert_eq!(pick(at(2000, 1, 1, 0, 0)), "One");
        assert_eq!(pick(at(2000, 1, 2, 23, 59)), "Two");
        assert_eq!(pick(at(2000, 1, 4, 0, 0)), "One");
        assert!(motd(&set(json!({})), at(2000, 1, 1, 0, 0), None).is_none());
    }

    /// Every bundled quote fits a Standard board with its author line, and Compact boards
    /// still have plenty to choose from.
    #[test]
    fn bundled_quotes_fit() {
        let q = bundled_quotes();
        assert!(q.len() >= 365, "{}", q.len());
        let all: Vec<(String, String)> = q.iter().map(|q| (q.text.clone(), q.author.clone())).collect();
        for (t, a) in &all {
            assert!(quote_fits(t, a, 6, 22), "doesn't fit 6×22: {t} - {a}");
            // Only characters a board has: every one maps to a flap (0 is only for spaces).
            let codes = crate::vestaboard::parse(&format!("{t} {a}"));
            let spaces = format!("{t} {a}").chars().filter(|c| *c == ' ').count();
            assert_eq!(codes.iter().filter(|&&c| c == 0).count(), spaces, "unsupported character in {t} - {a}");
        }
        assert!(fitting(&all, 3, 15).len() >= 60, "{}", fitting(&all, 3, 15).len());
        for tag in ["work", "calm", "funny", "wisdom"] {
            assert!(q.iter().filter(|x| x.tags.iter().any(|t| t == tag)).count() >= 20, "{tag}");
        }
    }

    #[test]
    fn quote_picking() {
        let day = at(2026, 10, 18, 9, 0);
        let any = quote_list(&set(json!({})));
        assert_eq!(any.len(), bundled_quotes().len());
        // Deterministic: the same quote all day, the next one tomorrow
        let pick = |s: &Map<String, Value>, d: NaiveDateTime, r, c| pick_quote(&quote_list(s), day_index(d), r, c).cloned();
        let s = set(json!({}));
        assert_eq!(pick(&s, day, 6, 22), pick(&s, at(2026, 10, 18, 23, 59), 6, 22));
        assert_ne!(pick(&s, day, 6, 22), pick(&s, at(2026, 10, 19, 0, 0), 6, 22));
        let fit = fitting(&any, 6, 22);
        assert_eq!(pick(&s, day, 6, 22).as_ref(), Some(fit[(day_index(day) as usize) % fit.len()]));
        // Compact gets a quote that fits it
        let (t, a) = pick(&s, day, 3, 15).unwrap();
        assert!(quote_fits(&t, &a, 3, 15));
        // Tags filter
        let funny = quote_list(&set(json!({"tags": "funny"})));
        assert!(!funny.is_empty() && funny.len() < any.len());
        assert!(funny.iter().all(|f| bundled_quotes().iter().any(|q| q.text == f.0 && q.tags.iter().any(|t| t == "funny"))));
        // My quotes: mixed in, or on their own
        let mine = set(json!({"source": "mine", "mine": ["Ship it - Sam", "Tea first", ""]}));
        assert_eq!(quote_list(&mine), vec![("Ship it".to_string(), "Sam".to_string()), ("Tea first".to_string(), String::new())]);
        let both = set(json!({"mine": ["Ship it — Sam"]}));
        assert_eq!(quote_list(&both).len(), bundled_quotes().len() + 1);
        assert!(quote(&set(json!({"source": "mine"})), day, None).is_none());
        // Once a day at a time, as a message
        let daily = set(json!({"mode": "daily", "at": "07:30"}));
        assert_eq!(quote_due(&daily, at(2026, 10, 18, 7, 29)), None);
        assert_eq!(quote_due(&daily, at(2026, 10, 18, 7, 30)).as_deref(), Some("quote-2026-10-18"));
        assert_eq!(quote_next(&daily, at(2026, 10, 18, 9, 0)), Some(at(2026, 10, 19, 7, 30)));
        assert_eq!(quote_due(&set(json!({"mode": "rotation"})), day), None);
    }
}
