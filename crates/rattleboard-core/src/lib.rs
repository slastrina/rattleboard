//! Rattleboard's board engine, shared by the desktop app (`src-tauri`) and the headless
//! Rattleboard Web server (`crates/rattleboard-web`): the Vestaboard-compatible API server,
//! layouts and transitions, sequences, saved boards and Send, and the Sources engine with its
//! plugins and built-ins. Nothing here touches windows.

pub mod boards;
pub mod builtins;
pub mod cards;
pub mod plugins;
pub mod secrets;
pub mod send;
pub mod sequence;
pub mod server;
pub mod settings;
pub mod sources;
pub mod update;
pub mod vestaboard;

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}
