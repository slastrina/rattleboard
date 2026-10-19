//! Non-secret preferences, stored as JSON in the app config dir.
//! API keys never live here; see `secrets.rs`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// An app whose notifications can be forwarded (Windows).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NotifyApp {
    /// AppUserModelID
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub board_name: String,
    /// "black" | "white"
    pub finish: String,
    /// "Match each message" (the default): every message shows at its own size, and the board
    /// window follows. Off: the board is always `model`'s size and centres other sizes.
    pub adaptive: bool,
    /// Board size: "standard" (6×22) | "compact" (3×15) | "large" (8×30) | "custom" (rows × cols).
    /// When adaptive, the size for text that doesn't ask for one, and for the demo.
    /// Older versions wrote "flag" and "note"; those still read as standard and compact.
    pub model: String,
    /// The custom size: 2–12 rows of 8–40 columns
    pub rows: usize,
    pub cols: usize,
    /// Board size relative to the default tile size (resizable by dragging the board's corner)
    pub scale: f64,
    pub sound: bool,
    pub speed: f64,
    pub hide_sec: u32,
    pub pause_on_hover: bool,
    /// A click on the board opens the card's web address (a feed story's page), when it has one
    pub click_links: bool,
    /// "toast": pops up in a corner when the message changes, then hides.
    /// "float": stays on screen wherever it was placed, like a desktop widget.
    /// "fullscreen": covers a whole display, for a spare computer used as a board.
    pub mode: String,
    /// The mode leaving full screen goes back to: "toast" | "float"
    pub windowed_mode: String,
    /// The display full screen covers, by name, and where it was, to tell two of the same
    /// model apart. Empty: the main display.
    pub screen_name: String,
    pub screen_pos: Point,
    /// Open in full screen at launch; otherwise full screen lasts until Rattleboard quits
    pub fullscreen_launch: bool,
    /// Toast corner: "tl" | "tr" | "bl" | "br"
    pub position: String,
    /// Floating position: top-left of the board window in physical screen pixels
    pub custom: Point,
    /// Floating layer: "desktop" (below other windows) | "top" (above them)
    pub float_layer: String,
    /// "local" | "cloud" | "demo" | "virtual" (only shows what apps send to the API server)
    pub source: String,
    pub host: String,
    pub poll_every: u32,
    pub polling: bool,
    pub launch_login: bool,
    pub setup_done: bool,
    /// Built-in, Vestaboard-compatible HTTP API
    pub server_enabled: bool,
    pub server_port: u16,
    /// Listen on all interfaces instead of only this computer
    pub server_lan: bool,
    /// Where Compose and Message History last sent a message: "this" or a saved board's id
    pub send_target: String,
    /// The message size Compose last used for an adaptive board
    pub send_rows: usize,
    pub send_cols: usize,
    /// Windows only: put the system's notifications on a board
    pub notify_enabled: bool,
    /// The apps whose notifications are forwarded
    pub notify_apps: Vec<NotifyApp>,
    /// "this" or a saved board's id
    pub notify_target: String,
    /// Show only the app's name, not the title and text
    pub notify_hide_content: bool,
    pub notify_quiet: bool,
    /// Quiet hours, "HH:MM" local time; may wrap past midnight
    pub notify_quiet_from: String,
    pub notify_quiet_to: String,
    /// Put the previous message back after this many seconds; 0 leaves the notification up
    pub notify_return_sec: u32,
    /// Check GitHub for a newer release once a day (it only suggests; nothing is installed)
    pub update_check: bool,
    /// When the last automatic or manual check answered (ms since the epoch; 0 for never)
    pub update_last_check: i64,
    /// The newest release that check found, and its page, so a restart shows it at once
    pub update_latest: String,
    pub update_url: String,
}

/// Board sizes by name: rows × columns.
pub const PRESETS: [(&str, usize, usize); 3] = [("standard", 6, 22), ("compact", 3, 15), ("large", 8, 30)];
pub const CUSTOM_ROWS: (usize, usize) = (2, 12);
pub const CUSTOM_COLS: (usize, usize) = (8, 40);

/// Not 7000 (a real board's port): AirPlay receivers on Macs and many TVs already hold it.
pub const DEFAULT_SERVER_PORT: u16 = 7070;

impl Default for Settings {
    fn default() -> Self {
        Self {
            board_name: "Rattleboard".into(),
            finish: "black".into(),
            adaptive: true,
            model: "standard".into(),
            rows: 6,
            cols: 22,
            scale: 1.0,
            sound: false,
            speed: 1.0,
            hide_sec: 10,
            pause_on_hover: true,
            click_links: true,
            mode: "toast".into(),
            windowed_mode: "toast".into(),
            screen_name: String::new(),
            screen_pos: Point { x: 0.0, y: 0.0 },
            fullscreen_launch: false,
            position: "br".into(),
            custom: Point { x: 300.0, y: 200.0 },
            float_layer: "desktop".into(),
            source: "local".into(),
            host: "vestaboard.local".into(),
            poll_every: 2,
            polling: true,
            launch_login: true,
            setup_done: false,
            server_enabled: false,
            server_port: DEFAULT_SERVER_PORT,
            server_lan: false,
            send_target: "this".into(),
            send_rows: 6,
            send_cols: 22,
            notify_enabled: false,
            notify_apps: vec![],
            notify_target: "this".into(),
            notify_hide_content: false,
            notify_quiet: false,
            notify_quiet_from: "22:00".into(),
            notify_quiet_to: "07:00".into(),
            notify_return_sec: 30,
            update_check: true,
            update_last_check: 0,
            update_latest: String::new(),
            update_url: String::new(),
        }
    }
}

impl Settings {
    pub fn load(path: &PathBuf) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
            .unwrap_or_default()
            .migrated()
            .normalized()
    }

    /// Older versions named the board "Vestaboard" by default; nobody chose that name.
    fn migrated(mut self) -> Self {
        if self.board_name == "Vestaboard" {
            self.board_name = "Rattleboard".into();
        }
        self
    }

    /// Keeps values in range after loading or merging a patch.
    pub fn normalized(mut self) -> Self {
        if !self.scale.is_finite() || self.scale <= 0.0 {
            self.scale = 1.0;
        }
        self.scale = self.scale.clamp(0.4, 3.0);
        if !["tl", "tr", "bl", "br"].contains(&self.position.as_str()) {
            self.position = "br".into();
        }
        self.model = match self.model.as_str() {
            "flag" => "standard".into(),
            "note" => "compact".into(),
            m if PRESETS.iter().any(|p| p.0 == m) || m == "custom" => m.into(),
            _ => "standard".into(),
        };
        self.rows = self.rows.clamp(CUSTOM_ROWS.0, CUSTOM_ROWS.1);
        self.send_rows = self.send_rows.clamp(CUSTOM_ROWS.0, CUSTOM_ROWS.1);
        self.send_cols = self.send_cols.clamp(CUSTOM_COLS.0, CUSTOM_COLS.1);
        self.cols = self.cols.clamp(CUSTOM_COLS.0, CUSTOM_COLS.1);
        if !["float", "fullscreen"].contains(&self.mode.as_str()) {
            self.mode = "toast".into();
        }
        if self.windowed_mode != "float" {
            self.windowed_mode = "toast".into();
        }
        if self.server_port == 0 {
            self.server_port = DEFAULT_SERVER_PORT;
        }
        if self.send_target.is_empty() {
            self.send_target = "this".into();
        }
        if self.notify_target.is_empty() {
            self.notify_target = "this".into();
        }
        self.notify_return_sec = self.notify_return_sec.min(3600);
        if !crate::update::is_release_url(&self.update_url) {
            self.update_url.clear();
        }
        self
    }

    pub fn floating(&self) -> bool {
        self.mode == "float"
    }

    pub fn fullscreen(&self) -> bool {
        self.mode == "fullscreen"
    }

    /// The mode to start in: full screen only with "Open in full screen at launch", and
    /// never before setup is done, as it would cover the setup window.
    pub fn at_launch(mut self) -> Self {
        let full = self.fullscreen_launch && self.setup_done;
        if full && !self.fullscreen() {
            self.windowed_mode = self.mode.clone();
            self.mode = "fullscreen".into();
        } else if !full && self.fullscreen() {
            self.mode = self.windowed_mode.clone();
        }
        self
    }

    pub fn save(&self, path: &PathBuf) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, s);
        }
    }

    /// Shallow-merges a JSON object of camelCase fields into these settings.
    pub fn merge(&self, patch: serde_json::Value) -> Result<Self, String> {
        let mut cur = serde_json::to_value(self).map_err(|e| e.to_string())?;
        if let (Some(cur), Some(patch)) = (cur.as_object_mut(), patch.as_object()) {
            for (k, v) in patch {
                cur.insert(k.clone(), v.clone());
            }
        }
        serde_json::from_value::<Self>(cur).map(Self::normalized).map_err(|e| e.to_string())
    }

    pub fn dims(&self) -> (usize, usize) {
        match PRESETS.iter().find(|p| p.0 == self.model) {
            Some(p) => (p.1, p.2),
            None if self.model == "custom" => (self.rows, self.cols),
            None => (6, 22),
        }
    }

    /// Takes on the size of a board it mirrors: a preset when one matches, else custom.
    pub fn set_dims(&mut self, (rows, cols): (usize, usize)) {
        match PRESETS.iter().find(|p| (p.1, p.2) == (rows, cols)) {
            Some(p) => self.model = p.0.into(),
            None => {
                self.model = "custom".into();
                self.rows = rows.clamp(1, crate::vestaboard::MAX_ROWS);
                self.cols = cols.clamp(1, crate::vestaboard::MAX_COLS);
            }
        }
    }
}

/// Minutes since midnight for "HH:MM".
fn minutes(hhmm: &str) -> Option<u32> {
    let (h, m) = hhmm.trim().split_once(':')?;
    let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// Whether `now` (minutes since midnight) falls in from..to, which may wrap past midnight.
pub fn in_quiet_hours(from: &str, to: &str, now: u32) -> bool {
    match (minutes(from), minutes(to)) {
        (Some(f), Some(t)) if f <= t => (f..t).contains(&now),
        (Some(f), Some(t)) => now >= f || now < t,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_hours() {
        assert!(in_quiet_hours("22:00", "07:00", 23 * 60));
        assert!(in_quiet_hours("22:00", "07:00", 6 * 60 + 59));
        assert!(!in_quiet_hours("22:00", "07:00", 7 * 60));
        assert!(in_quiet_hours("09:30", "17:00", 12 * 60));
        assert!(!in_quiet_hours("09:30", "17:00", 9 * 60));
        assert!(!in_quiet_hours("nope", "07:00", 0));
        assert!(!in_quiet_hours("08:00", "08:00", 8 * 60));
    }

    #[test]
    fn old_default_name_becomes_rattleboard() {
        assert_eq!(Settings::default().board_name, "Rattleboard");
        let dir = std::env::temp_dir().join(format!("rb-name-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"boardName": "Vestaboard"}"#).unwrap();
        assert_eq!(Settings::load(&path).board_name, "Rattleboard");
        std::fs::write(&path, r#"{"boardName": "Kitchen Vestaboard"}"#).unwrap();
        assert_eq!(Settings::load(&path).board_name, "Kitchen Vestaboard");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_check_defaults_on() {
        // Settings files from before the update check have none of its fields.
        let old: Settings = serde_json::from_str(r#"{"boardName": "Kitchen"}"#).unwrap();
        assert!(old.update_check && old.update_last_check == 0 && old.update_latest.is_empty());
        let s = Settings::default().merge(serde_json::json!({"updateUrl": "https://evil.example/"})).unwrap();
        assert!(s.update_url.is_empty());
    }

    #[test]
    fn sizes() {
        let old: Settings = serde_json::from_str(r#"{"model": "note"}"#).unwrap();
        assert_eq!(old.normalized().dims(), (3, 15));
        let old: Settings = serde_json::from_str(r#"{"model": "flag"}"#).unwrap();
        let s = old.normalized();
        assert_eq!((s.model.as_str(), s.dims()), ("standard", (6, 22)));
        // Settings from before adaptive boards match each message, like new installs.
        assert!(s.adaptive && Settings::default().adaptive);
        let s = Settings::default().merge(serde_json::json!({"model": "large"})).unwrap();
        assert_eq!(s.dims(), (8, 30));
        let s = Settings::default().merge(serde_json::json!({"model": "custom", "rows": 40, "cols": 3})).unwrap();
        assert_eq!(s.dims(), (12, 8));
        let s = Settings::default().merge(serde_json::json!({"model": "huge"})).unwrap();
        assert_eq!(s.dims(), (6, 22));
        let mut s = Settings::default();
        s.set_dims((3, 15));
        assert_eq!(s.model, "compact");
        s.set_dims((5, 20));
        assert_eq!((s.model.as_str(), s.dims()), ("custom", (5, 20)));
    }

    #[test]
    fn modes() {
        let s: Settings = serde_json::from_str(r#"{"mode": "kiosk", "windowedMode": "fullscreen"}"#).unwrap();
        let s = s.normalized();
        assert_eq!((s.mode.as_str(), s.windowed_mode.as_str()), ("toast", "toast"));
        // Settings from before full screen read as they were.
        let s: Settings = serde_json::from_str(r#"{"mode": "float"}"#).unwrap();
        let s = s.normalized();
        assert_eq!((s.mode.as_str(), s.screen_name.as_str(), s.fullscreen_launch), ("float", "", false));
        let s = Settings::default().merge(serde_json::json!({"mode": "fullscreen", "screenName": "LG HDR 4K", "screenPos": {"x": 1440.0, "y": 0.0}})).unwrap();
        assert!(s.fullscreen() && !s.floating());
        assert_eq!((s.screen_name.as_str(), s.screen_pos), ("LG HDR 4K", Point { x: 1440.0, y: 0.0 }));
        assert!(Settings::default().merge(serde_json::json!({"screenPos": "left"})).is_err());
    }

    #[test]
    fn launching() {
        let set = |v: serde_json::Value| Settings { setup_done: true, ..Default::default() }.merge(v).unwrap();
        // Full screen lasts until quit, back to the mode it came from...
        let s = set(serde_json::json!({"mode": "fullscreen", "windowedMode": "float"})).at_launch();
        assert_eq!(s.mode, "float");
        // ...unless it opens in full screen at launch.
        let s = set(serde_json::json!({"mode": "fullscreen", "fullscreenLaunch": true})).at_launch();
        assert!(s.fullscreen());
        let s = set(serde_json::json!({"mode": "float", "fullscreenLaunch": true})).at_launch();
        assert_eq!((s.mode.as_str(), s.windowed_mode.as_str()), ("fullscreen", "float"));
        // Never over the first-run setup.
        let s = set(serde_json::json!({"mode": "fullscreen", "fullscreenLaunch": true, "setupDone": false})).at_launch();
        assert_eq!(s.mode, "toast");
    }

    #[test]
    fn merging() {
        let s = Settings::default().merge(serde_json::json!({"notifyTarget": "", "notifyApps": [{"id": "a", "name": "Mail"}]})).unwrap();
        assert_eq!((s.notify_target.as_str(), s.notify_apps.len()), ("this", 1));
    }
}
