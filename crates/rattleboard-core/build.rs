// Script plugins (QuickJS, through rquickjs) build everywhere except 32-bit ARM with musl:
// rquickjs ships no bindings for it, and that target (Rattleboard Web's linux/arm/v7 image)
// runs without them. `rb_scripts` is set where they're available.
fn main() {
    println!("cargo:rustc-check-cfg=cfg(rb_scripts)");
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if !(arch == "arm" && env == "musl") {
        println!("cargo:rustc-cfg=rb_scripts");
    }
}
