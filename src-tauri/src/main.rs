// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `--mcp` and `--notify` (for AI agent apps) run without any window or tray, and without
    // the single-instance check, so they work beside the running app.
    if let Some(code) = rattleboard_lib::agents::cli::main() {
        std::process::exit(code);
    }
    rattleboard_lib::run()
}
