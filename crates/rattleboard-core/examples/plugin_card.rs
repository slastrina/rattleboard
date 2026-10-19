//! Renders a plugin's card through the real Sources engine, for plugin authors and the
//! community repository's preview.json files.
//!
//!   cargo run -p rattleboard-core --example plugin_card -- <manifest.json>
//!       [--settings '{"stop": "1071"}'] [--secret key=VALUE] [--sample response.json]
//!       [--now 2026-10-19T09:38:30Z] [--preview preview.json] [--mock responses.json]
//!
//! A script plugin's script file is read from beside the manifest. It runs for real (its
//! fetches go out), or with --mock against canned answers in the conformance vectors' "fetch"
//! format: [{"method": "GET", "url": "https://…", "response": {"status": 200, "body": "…"}}].
//! Its log, and how long it ran, are printed too.
//!
//! Without --sample it fetches the real address. With --sample the response comes from a file,
//! served on 127.0.0.1 at the manifest's own path and query, so signing and every pick step
//! run as they would for real. Secrets go to a temporary file, never the credential store.
//! Prints the card at Standard (6×22), Compact (3×15) and Large (8×30); --preview writes the
//! Standard card's first page.

use rattleboard_core::cards::ascii;
use rattleboard_core::script;
use rattleboard_core::sources::{Ctx, Engine, Paths};
use serde_json::{json, Value};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first().filter(|a| !a.starts_with("--")) else {
        eprintln!("usage: plugin_card <manifest.json> [--settings JSON] [--secret k=v]... [--sample FILE] [--now RFC3339] [--preview FILE]");
        std::process::exit(2);
    };
    let tmp = std::env::temp_dir().join(format!("rb-plugin-card-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    rattleboard_core::secrets::use_file(tmp.join("secrets.json"));
    let mut manifest: Value = serde_json::from_str(&std::fs::read_to_string(path).expect("manifest")).expect("manifest JSON");
    if let Some(file) = script::file_to_fetch(&manifest) {
        let dir = std::path::Path::new(path).parent().unwrap_or(std::path::Path::new("."));
        let code = std::fs::read(dir.join(&file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        script::inline(&mut manifest, &code).expect("script");
    }
    if let Some(sample) = arg(&args, "--sample") {
        let body = std::fs::read(&sample).expect("sample");
        let app = axum::Router::new().fallback(move || {
            let body = body.clone();
            async move { ([("content-type", "application/json")], body) }
        });
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await });
        let url = manifest["fetch"]["url"].as_str().unwrap().to_string();
        let rest = url.splitn(4, '/').nth(3).unwrap_or("");
        manifest["fetch"]["url"] = json!(format!("{base}/{rest}"));
    }
    let e = Engine::new(Paths { config: Some(tmp.join("sources.json")), plugins: Some(tmp.join("plugins.json")), seen: Some(tmp.join("seen.json")) });
    let id = e.install(&manifest, false).expect("install");
    let settings: Option<Value> = arg(&args, "--settings").map(|s| serde_json::from_str(&s).expect("--settings JSON"));
    let inst = e.add(&id, settings.as_ref()).expect("add source");
    let mut secrets = serde_json::Map::new();
    for (i, a) in args.iter().enumerate() {
        if a == "--secret" {
            let (k, v) = args[i + 1].split_once('=').expect("--secret key=value");
            secrets.insert(k.into(), json!(v));
        }
    }
    if !secrets.is_empty() {
        e.set_secrets(&inst, &Value::Object(secrets.clone())).expect("secrets");
    }
    let now_arg = arg(&args, "--now");
    let now = now_arg.map(|t| chrono::DateTime::parse_from_rfc3339(&t).expect("--now").timestamp_millis()).unwrap_or_else(rattleboard_core::now_ms);
    let local = rattleboard_core::plugins::local_of(now);
    if manifest["kind"] == "script" && !run_script(&e, &inst, &manifest, settings.as_ref(), &secrets, arg(&args, "--mock"), now, local).await {
        let _ = std::fs::remove_dir_all(&tmp);
        std::process::exit(1);
    }
    for (rows, cols, name) in [(6, 22, "Standard"), (3, 15, "Compact"), (8, 30, "Large")] {
        let ctx = Ctx { now, local, paused: None, dims: (rows, cols) };
        let (pages, reason) = e.preview(&inst, &ctx, false, false).await.expect("preview");
        println!("{name} {rows}x{cols} ({reason}):");
        for (n, g) in pages.iter().enumerate() {
            if pages.len() > 1 {
                println!("  page {}", n + 1);
            }
            for l in ascii(g) {
                println!("  |{l}|");
            }
        }
        if let (Some(out), 6) = (arg(&args, "--preview"), rows) {
            let g = &pages[0];
            let lines: Vec<String> = g.iter().map(|r| format!("    {}", serde_json::to_string(r).unwrap())).collect();
            std::fs::write(&out, format!("{{\n  \"rows\": {},\n  \"cols\": {},\n  \"characters\": [\n{}\n  ]\n}}\n", g.len(), g[0].len(), lines.join(",\n"))).unwrap();
            println!("  wrote {out}");
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Runs a script plugin once at Standard size (for real, or against --mock answers), prints its
/// log and timing, and hands the engine its result so the previews below show it.
#[allow(clippy::too_many_arguments)]
async fn run_script(e: &Engine, inst: &str, manifest: &Value, settings: Option<&Value>, secrets: &serde_json::Map<String, Value>, mock: Option<String>, now: i64, local: chrono::NaiveDateTime) -> bool {
    let m = rattleboard_core::plugins::parse(manifest, false).expect("manifest");
    let spec = m.script.clone().expect("script");
    let mut values: serde_json::Map<String, Value> = m.settings.iter().map(|s| (s.key.clone(), s.default.clone())).collect();
    if let Some(Value::Object(o)) = settings {
        values.extend(o.clone());
    }
    let utc = chrono::DateTime::from_timestamp_millis(now).unwrap().naive_utc();
    let input = script::Input {
        now,
        local: local.format("%Y-%m-%dT%H:%M:%S").to_string(),
        utc_offset: (local - utc).num_minutes(),
        rows: 6,
        cols: 22,
        name: m.name.clone(),
        settings: values,
        declared: m.secrets.iter().map(|s| s.key.clone()).collect(),
        secrets: secrets.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect(),
        store: Default::default(),
        hosts: spec.hosts.clone(),
    };
    let out = match mock {
        Some(f) => {
            let v: Value = serde_json::from_str(&std::fs::read_to_string(&f).expect("--mock file")).expect("--mock JSON");
            script::run(&spec.source, &input, &script::MockFetcher::from_json(&v))
        }
        None => {
            let handle = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || script::run(&spec.source, &input, &script::NetFetcher { handle })).await.unwrap()
        }
    };
    println!("Script: {} ms of JavaScript, {} ms in all, {} fetch(es)", out.cpu_ms, out.wall_ms, out.requests.len());
    for l in &out.log {
        println!("  log: {l}");
    }
    match out.result {
        Ok(o) => {
            println!("  result: {}", o.to_json());
            e.store(inst, Ok(o.into_data()), now);
            true
        }
        Err(err) => {
            println!("  error ({}): {}", err.kind, err.message);
            false
        }
    }
}
