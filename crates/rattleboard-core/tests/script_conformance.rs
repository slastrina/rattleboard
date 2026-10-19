//! The script-plugin conformance vectors, run through this engine.
//!
//! The vectors live in the community repository (rattleboard-plugins/conformance/script/), so
//! every port runs the same ones; a copy is kept here in tests/conformance/ (refresh it with
//! scripts/sync-conformance.sh). Set RATTLEBOARD_CONFORMANCE=<dir> to run another folder,
//! such as the plugins repository's own.

use rattleboard_core::cards::ascii;
use rattleboard_core::script::{self, Input, MockFetcher, Output};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

fn dir() -> PathBuf {
    std::env::var("RATTLEBOARD_CONFORMANCE").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/conformance/script"))
}

fn input_of(v: &Value) -> Input {
    let i = &v["input"];
    let secrets: Map<String, Value> = i["secrets"].as_object().cloned().unwrap_or_default();
    Input {
        now: i["now"].as_i64().unwrap(),
        local: i["local"].as_str().unwrap().into(),
        utc_offset: i["utcOffset"].as_i64().unwrap(),
        rows: i["board"]["rows"].as_u64().unwrap() as usize,
        cols: i["board"]["cols"].as_u64().unwrap() as usize,
        name: i["name"].as_str().unwrap_or("").into(),
        settings: i["settings"].as_object().cloned().unwrap_or_default(),
        declared: secrets.keys().cloned().collect(),
        secrets: secrets.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect(),
        store: i["store"].as_object().cloned().unwrap_or_default(),
        hosts: i["hosts"].as_array().map(|a| a.iter().map(|h| script::check_host_pattern(h.as_str().unwrap()).unwrap()).collect()).unwrap_or_default(),
    }
}

/// Checks one vector; returns what's wrong.
fn check(v: &Value) -> Vec<String> {
    let input = input_of(v);
    let out = script::run(v["script"].as_str().unwrap(), &input, &MockFetcher::from_json(&v["fetch"]));
    let e = &v["expect"];
    let mut bad = vec![];
    match (&out.result, e.get("error")) {
        (Err(got), Some(want)) => {
            let kinds: Vec<&str> = match want {
                Value::String(s) => vec![s.as_str()],
                Value::Array(a) => a.iter().filter_map(|k| k.as_str()).collect(),
                _ => vec![],
            };
            if !kinds.contains(&got.kind) {
                bad.push(format!("error kind {} ({}), expected {kinds:?}", got.kind, got.message));
            }
            if let Some(m) = e.get("message").and_then(|m| m.as_str()) {
                if got.message != m {
                    bad.push(format!("message {:?}, expected {m:?}", got.message));
                }
            }
        }
        (Err(got), None) => bad.push(format!("failed: {} {}", got.kind, got.message)),
        (Ok(o), Some(_)) => bad.push(format!("succeeded with {}, expected an error", o.to_json())),
        (Ok(o), None) => {
            if o.to_json() != e["output"] {
                bad.push(format!("output\n  got  {}\n  want {}", o.to_json(), e["output"]));
            }
            if let Some(want) = e.get("store") {
                if json!(out.store) != *want {
                    bad.push(format!("store {} expected {want}", json!(out.store)));
                }
            }
            if let Some(want) = e.get("render") {
                let got = render(o, v);
                if got != want["pages"] {
                    bad.push(format!("render\n  got  {got:#}\n  want {:#}", want["pages"]));
                }
            }
        }
    }
    if let Some(want) = e.get("requests") {
        let got: Vec<Value> = out
            .requests
            .iter()
            .map(|r| json!({"method": r.method, "url": r.url, "headers": r.headers.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<Map<_, _>>(), "body": r.body}))
            .collect();
        if json!(got) != *want {
            bad.push(format!("requests\n  got  {}\n  want {want}", json!(got)));
        }
    }
    if let Some(want) = e.get("log") {
        if json!(out.log) != *want {
            bad.push(format!("log {} expected {want}", json!(out.log)));
        }
    }
    bad
}

fn render(o: &Output, v: &Value) -> Value {
    let Output::Card(c) = o else { return Value::Null };
    let rows = v["input"]["board"]["rows"].as_u64().unwrap() as usize;
    let cols = v["input"]["board"]["cols"].as_u64().unwrap() as usize;
    let face = c.face(c.chip(None), false, 1);
    json!(face.render(rows, cols).iter().map(ascii).collect::<Vec<_>>())
}

#[test]
fn conformance_vectors() {
    let d = dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&d).unwrap_or_else(|e| panic!("{}: {e}", d.display())).filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    assert!(files.len() >= 50, "expected the conformance vectors in {}", d.display());
    let mut failures = vec![];
    for f in &files {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
        if !script::SUPPORTED {
            continue;
        }
        let bad = check(&v);
        if !bad.is_empty() {
            failures.push(format!("{}:\n  {}", f.file_name().unwrap().to_string_lossy(), bad.join("\n  ")));
        }
    }
    assert!(failures.is_empty(), "{} of {} vectors failed:\n{}", failures.len(), files.len(), failures.join("\n"));
}

/// The vectors' copy of the prelude must be this engine's, word for word.
#[test]
fn prelude_matches() {
    let p = dir().join("../script-prelude.js");
    if let Ok(text) = std::fs::read_to_string(&p) {
        assert_eq!(text, script::PRELUDE, "{} differs from script_prelude.js", p.display());
    }
}
