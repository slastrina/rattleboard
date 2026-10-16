//! Non-secret preferences, stored as JSON in the app config dir.
//! API keys never live here; see `secrets.rs`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub board_name: String,
    /// "black" | "white"
    pub finish: String,
    /// "flag" (Flagship 6x22) | "note" (Note 3x15)
    pub model: String,
    /// "s" | "m" | "l"
    pub size: String,
    pub sound: bool,
    pub speed: f64,
    pub hide_sec: u32,
    pub pause_on_hover: bool,
    pub pinned: bool,
    /// "tl" | "tr" | "bl" | "br" | "custom"
    pub position: String,
    /// Top-left of the board (not the window) in logical screen coordinates
    pub custom: Point,
    /// "local" | "cloud"
    pub source: String,
    pub host: String,
    pub poll_every: u32,
    pub polling: bool,
    pub launch_login: bool,
    pub setup_done: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            board_name: "Vestaboard".into(),
            finish: "black".into(),
            model: "flag".into(),
            size: "m".into(),
            sound: false,
            speed: 1.0,
            hide_sec: 10,
            pause_on_hover: true,
            pinned: false,
            position: "br".into(),
            custom: Point { x: 300.0, y: 200.0 },
            source: "local".into(),
            host: "vestaboard.local".into(),
            poll_every: 2,
            polling: true,
            launch_login: true,
            setup_done: false,
        }
    }
}

impl Settings {
    pub fn load(path: &PathBuf) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
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
        serde_json::from_value(cur).map_err(|e| e.to_string())
    }

    pub fn dims(&self) -> (usize, usize) {
        if self.model == "note" {
            (3, 15)
        } else {
            (6, 22)
        }
    }
}
