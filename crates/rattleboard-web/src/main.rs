//! Rattleboard Web: a self-hosted board for any browser, with the same API as every
//! Rattleboard. Settings come from the environment (see config.rs and the README).
//!
//!   rattleboard-web            serves the board and the API (the container's command)
//!   rattleboard-web pair       prints the pairing link and QR code for a phone
//!   rattleboard-web key        prints the API key
//!   rattleboard-web viewer-url prints the board's viewer link (token mode)
//!   rattleboard-web health     exits 0 when the server answers (the image's health check)

mod board;
mod config;
mod page;

use config::{Config, Viewers};
use rattleboard_core::{boards, secrets, server};
use std::path::Path;
use std::sync::Arc;

const HELP: &str = "Rattleboard Web: a split-flap board for any browser, with the Rattleboard API.

Usage: rattleboard-web [serve | pair | key | viewer-url | health | --version]

Settings (environment):
  RATTLEBOARD_PORT          port to listen on (7070)
  RATTLEBOARD_DATA          data folder (/data)
  RATTLEBOARD_KEY           API key for writes (else generated into the data folder)
  RATTLEBOARD_NAME          the board's name (Rattleboard Web)
  RATTLEBOARD_SIZE          standard (6x22) | compact (3x15) | large (8x30) | custom
  RATTLEBOARD_ROWS/COLS     custom size: 2-12 rows, 8-40 columns
  RATTLEBOARD_ADAPTIVE      true: show each message at its own size (false)
  RATTLEBOARD_FINISH        black | white (black)
  RATTLEBOARD_VIEWERS       open | token: who may watch the board page (open)
  RATTLEBOARD_VIEWER_TOKEN  the viewer token (else generated into the data folder)
  RATTLEBOARD_URL           this board's address as phones reach it, for pairing
  RATTLEBOARD_MDNS          true: advertise on the network (needs host networking) (false)
  TZ                        time zone for clocks, calendars and quiet hours (UTC)";

/// A secret kept in a file of its own in the data folder (0600), made on first use.
fn file_secret(path: &Path) -> Result<(String, bool), String> {
    if let Ok(s) = std::fs::read_to_string(path) {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return Ok((s, false));
        }
    }
    let k = server::new_key();
    secrets::write_private(path, &k).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    Ok((k, true))
}

struct Keys {
    key: String,
    key_new: bool,
    viewer: Option<String>,
}

fn keys(cfg: &Config) -> Result<Keys, String> {
    std::fs::create_dir_all(&cfg.data).map_err(|e| format!("Couldn't use the data folder {}: {e}", cfg.data.display()))?;
    let (key, key_new) = match &cfg.key {
        Some(k) => (k.clone(), false),
        None => file_secret(&cfg.data.join("api-key"))?,
    };
    let viewer = match (&cfg.viewers, &cfg.viewer_token) {
        (Viewers::Open, _) => None,
        (Viewers::Token, Some(t)) => Some(t.clone()),
        (Viewers::Token, None) => Some(file_secret(&cfg.data.join("viewer-token"))?.0),
    };
    Ok(Keys { key, key_new, viewer })
}

pub fn pair_link(b: &board::Board, base: &str) -> String {
    boards::pair_url(base, b.api_key(), &b.cfg.name, b.cfg.rows, b.cfg.cols)
}

pub fn qr_svg(text: &str) -> String {
    let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else { return String::new() };
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(232, 232)
        .dark_color(qrcode::render::svg::Color("#1b1b1d"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();
    svg.find("<svg").map(|i| svg[i..].to_string()).unwrap_or(svg)
}

/// The base address for links printed here, where no browser says how it reached us.
fn base_url(cfg: &Config) -> (String, bool) {
    match &cfg.public_url {
        Some(u) => (u.clone(), true),
        None => (format!("http://{}:{}", server::lan_ip().unwrap_or_else(|| "localhost".into()), cfg.port), false),
    }
}

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "serve".into());
    if matches!(arg.as_str(), "-h" | "--help" | "help") {
        println!("{HELP}");
        return;
    }
    if matches!(arg.as_str(), "-V" | "--version" | "version") {
        println!("rattleboard-web {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => fail(&e),
    };
    match arg.as_str() {
        "serve" => serve(cfg),
        "health" => health(&cfg),
        "key" | "pair" | "viewer-url" => {
            let k = keys(&cfg).unwrap_or_else(|e| fail(&e));
            let (base, given) = base_url(&cfg);
            match arg.as_str() {
                "key" => println!("{}", k.key),
                "viewer-url" => match k.viewer {
                    Some(t) => println!("{base}/#v={t}"),
                    None => println!("{base}/   (open to anyone who can reach it; set RATTLEBOARD_VIEWERS=token to require a link)"),
                },
                _ => {
                    secrets::use_file(cfg.data.join("secrets.json"));
                    let link = boards::pair_url(&base, &k.key, &cfg.name, cfg.rows, cfg.cols);
                    if let Ok(code) = qrcode::QrCode::new(link.as_bytes()) {
                        let text = code
                            .render::<qrcode::render::unicode::Dense1x2>()
                            .dark_color(qrcode::render::unicode::Dense1x2::Light)
                            .light_color(qrcode::render::unicode::Dense1x2::Dark)
                            .build();
                        println!("{text}");
                    }
                    println!("Scan with the Rattleboard app to add {} ({}).", cfg.name, base);
                    println!("{link}");
                    if !given {
                        eprintln!(
                            "Note: {base} is this container's own address. If phones reach the board at another \
                             address, set RATTLEBOARD_URL (e.g. http://192.168.1.20:7070) or open /pair in a browser."
                        );
                    }
                    println!("The code includes the API key: only show it to devices you trust.");
                }
            }
        }
        other => fail(&format!("Unknown command \"{other}\"\n\n{HELP}")),
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("rattleboard-web: {msg}");
    std::process::exit(2)
}

fn health(cfg: &Config) {
    use std::io::{Read, Write};
    let ok = (|| {
        let mut s = std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], cfg.port).into(), std::time::Duration::from_secs(3)).ok()?;
        s.set_read_timeout(Some(std::time::Duration::from_secs(3))).ok()?;
        s.write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").ok()?;
        let mut buf = String::new();
        s.read_to_string(&mut buf).ok()?;
        buf.starts_with("HTTP/1.1 200").then_some(())
    })()
    .is_some();
    std::process::exit(if ok { 0 } else { 1 })
}

#[tokio::main]
async fn serve(cfg: Config) {
    let k = keys(&cfg).unwrap_or_else(|e| fail(&e));
    // No OS keychain in a container: plugin secrets go to a 0600 file in the data folder.
    secrets::use_file(cfg.data.join("secrets.json"));
    let b = board::Board::new(cfg.clone(), k.key.clone(), k.viewer.clone());
    let router = server::router(Arc::new(board::Api(b.clone())), page::routes(b.clone()));
    let handle = match server::serve(router, cfg.port, true).await {
        Ok(h) => h,
        Err(e) => fail(&e),
    };
    tokio::spawn(b.clone().sources_loop());
    // One line in the log when a newer release is out; RATTLEBOARD_UPDATE_CHECK=off skips it.
    let off = std::env::var("RATTLEBOARD_UPDATE_CHECK").is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "off" | "false" | "no"));
    if !off {
        tokio::spawn(async {
            if let Ok(r) = rattleboard_core::update::latest(&rattleboard_core::update::client()).await {
                if rattleboard_core::update::is_newer(&r.version, env!("CARGO_PKG_VERSION")) {
                    println!("Update: Rattleboard Web {} is available (this is {}): {}", r.version, env!("CARGO_PKG_VERSION"), r.url);
                }
            }
        });
    }
    let _advert = cfg.mdns.then(|| boards::advertise(&cfg.name, cfg.port)).flatten();

    println!("Rattleboard Web {} on port {} · {} · {}", env!("CARGO_PKG_VERSION"), handle.addr.port(), cfg.name, cfg.size_name());
    match (&cfg.key, k.key_new) {
        (Some(_), _) => println!("API key: from RATTLEBOARD_KEY"),
        (None, true) => println!("API key: generated into {} (show it with: rattleboard-web key)", cfg.data.join("api-key").display()),
        (None, false) => println!("API key: from {} (show it with: rattleboard-web key)", cfg.data.join("api-key").display()),
    }
    match &cfg.viewers {
        Viewers::Open => println!("Viewers: open to anyone who can reach this port"),
        Viewers::Token => println!("Viewers: need the viewer link (show it with: rattleboard-web viewer-url)"),
    }
    println!("Pairing: open /pair in a browser, or run: rattleboard-web pair");
    if cfg.mdns {
        println!("mDNS: {}", if _advert.is_some() { "advertising this board on the network" } else { "couldn't start (use host networking)" });
    }

    shutdown().await;
    println!("Stopping");
    drop(handle);
}

async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate()).expect("signal handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
