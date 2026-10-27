fn main() {
    // Windows bundles the screen saver (tauri.windows.conf.json), and Tauri wants every
    // resource to exist when the app compiles, which is before the bundling step that builds
    // it (scripts/build-saver.mjs). Until then, an empty stand-in, which the app treats as no
    // screen saver (saver.rs).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let scr = std::path::Path::new("saver").join("Rattleboard.scr");
        if !scr.exists() {
            let _ = std::fs::create_dir_all("saver");
            let _ = std::fs::write(&scr, b"");
        }
    }
    tauri_build::build()
}
