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
    /// Board size relative to the default tile size (resizable by dragging the board's corner)
    pub scale: f64,
    pub sound: bool,
    pub speed: f64,
    pub hide_sec: u32,
    pub pause_on_hover: bool,
    /// "toast": pops up in a corner when the message changes, then hides.
    /// "float": stays on screen wherever it was placed, like a desktop widget.
    pub mode: String,
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
}

#[cfg(target_os = "macos")]
pub const DEFAULT_SERVER_PORT: u16 = 7070; // 7000 is taken by AirPlay Receiver on macOS
#[cfg(not(target_os = "macos"))]
pub const DEFAULT_SERVER_PORT: u16 = 7000;

impl Default for Settings {
    fn default() -> Self {
        Self {
            board_name: "Vestaboard".into(),
            finish: "black".into(),
            model: "flag".into(),
            scale: 1.0,
            sound: false,
            speed: 1.0,
            hide_sec: 10,
            pause_on_hover: true,
            mode: "toast".into(),
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
        }
    }
}

impl Settings {
    pub fn load(path: &PathBuf) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
            .unwrap_or_default()
            .normalized()
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
        if self.mode != "float" {
            self.mode = "toast".into();
        }
        if self.server_port == 0 {
            self.server_port = DEFAULT_SERVER_PORT;
        }
        self
    }

    pub fn floating(&self) -> bool {
        self.mode == "float"
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
        if self.model == "note" {
            (3, 15)
        } else {
            (6, 22)
        }
    }
}
