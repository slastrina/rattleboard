use super::*;
use std::time::Instant;

fn input() -> Input {
    Input {
        now: 1_792_363_260_000,
        local: "2026-10-19T09:41:00".into(),
        utc_offset: 660,
        rows: 6,
        cols: 22,
        name: "Test".into(),
        hosts: vec!["api.example.com".into()],
        ..Default::default()
    }
}

fn go(src: &str) -> Outcome {
    run(src, &input(), &MockFetcher::default())
}

fn text(o: &Outcome) -> String {
    match &o.result {
        Ok(Output::Card(c)) => match &c.pages[0] {
            Page::Text(t) => t.clone(),
            p => format!("{p:?}"),
        },
        other => format!("{other:?}"),
    }
}

fn kind(o: &Outcome) -> &'static str {
    o.result.as_ref().err().map(|e| e.kind).unwrap_or("ok")
}

#[test]
fn infinite_loop_is_interrupted() {
    let t = Instant::now();
    let o = go("export default function () { while (true) {} }");
    assert_eq!(kind(&o), "cpu");
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    assert!((250..1000).contains(&o.cpu_ms), "{}", o.cpu_ms);
    // Catching it doesn't help, and neither does an async loop between awaits.
    assert_eq!(kind(&go("export default function () { try { while (true) {} } catch (e) { return 'caught'; } }")), "cpu");
    assert_eq!(kind(&go("export default async function () { for (;;) await null; }")), "cpu");
}

#[test]
fn memory_blowup_is_contained() {
    let o = go("export default function () { const a = []; while (true) a.push(new Array(100000).fill(1)); }");
    assert_eq!(kind(&o), "memory");
    let o = go("export default function () { const a = []; for (;;) a.push({ s: 'x'.repeat(1000) + Math.random() }); }");
    assert!(["memory", "cpu"].contains(&kind(&o)), "{:?}", o.result);
    // Deep recursion is an ordinary error, not a crash.
    assert_eq!(kind(&go("export default function () { function f() { return f() + 1; } return f(); }")), "exception");
    // And the next run is fine.
    assert_eq!(text(&go("export default () => 'ok'")), "ok");
}

#[test]
fn disallowed_host_is_refused() {
    let o = go("export default async function (ctx) { try { await ctx.fetch('https://evil.example.net/x'); } catch (e) { return e.code; } }");
    assert_eq!(text(&o), "host_not_allowed");
    assert!(o.requests.is_empty());
    for (hosts, host, ok) in [
        (vec!["api.example.com"], "api.example.com", true),
        (vec!["api.example.com"], "API.Example.com.", true),
        (vec!["api.example.com"], "x.api.example.com", false),
        (vec!["*.example.com"], "a.b.example.com", true),
        (vec!["*.example.com"], "example.com", false),
        (vec!["*.example.com"], "badexample.com", false),
        (vec!["*.example.com"], "example.com.evil.net", false),
    ] {
        let hosts: Vec<String> = hosts.iter().map(|h| h.to_string()).collect();
        assert_eq!(host_allowed(&hosts, host), ok, "{hosts:?} {host}");
    }
}

#[test]
fn secrets_are_never_readable() {
    let mut i = input();
    i.declared = vec!["token".into()];
    i.secrets.insert("token".into(), "hunter2-xyz".into());
    let fetch = MockFetcher {
        routes: vec![MockRoute { method: "GET".into(), url: "https://api.example.com/echo?k=hunter2-xyz".into(), status: 200, body: "{\"k\": \"hunter2-xyz\"}".into(), headers: vec![("x-echo".into(), "hunter2-xyz".into())], ..Default::default() }],
    };
    let src = "export default async function (ctx) {
      const t = ctx.secret('token');
      const seen = [String(t), JSON.stringify(t), Object.getOwnPropertyNames(t).join(), Object.getOwnPropertySymbols(Object.getPrototypeOf(t)).length];
      const r = await ctx.fetch('https://api.example.com/echo?k=' + t, { headers: { Authorization: `Bearer ${t}` } });
      seen.push(await r.text(), r.headers['x-echo'], r.url);
      return seen.join(' | ');
    }";
    let o = run(src, &i, &fetch);
    let t = text(&o);
    assert!(!t.contains("hunter2"), "{t}");
    assert!(!o.log.join(" ").contains("hunter2"));
    assert_eq!(o.requests[0].headers, vec![("authorization".to_string(), "Bearer hunter2-xyz".to_string())]);
    assert!(t.starts_with("{{secret.token}} | \"{{secret.token}}\" |  | 1 | {\"k\": \"[secret]\"} | [secret]"), "{t}");
    // The input the prelude reads never has a value in it.
    assert!(!i.to_json().to_string().contains("hunter2"));
}

#[test]
fn sixth_fetch_is_refused() {
    let routes = (1..=6).map(|k| MockRoute { method: "GET".into(), url: format!("https://api.example.com/{k}"), status: 200, body: "x".into(), ..Default::default() }).collect();
    let o = run(
        "export default async function (ctx) { const out = []; for (let i = 1; i <= 6; i++) { try { await ctx.fetch('https://api.example.com/' + i); out.push('ok'); } catch (e) { out.push(e.code); } } return out.join(' '); }",
        &input(),
        &MockFetcher { routes },
    );
    assert_eq!(text(&o), "ok ok ok ok ok too_many_fetches");
    assert_eq!(o.requests.len(), 5);
}

#[test]
fn invalid_results_are_rejected() {
    for (src, msg) in [
        ("export default () => 42", "Return a string"),
        ("export default () => ({ text: 'x', color: 'pink' })", "color must be"),
        ("export default () => ({ characters: [[99]] })", "Unknown character code 99"),
        ("export default () => ({ items: [{}] })", "items[0] needs a title or a body"),
        ("export default () => { const o = {}; o.o = o; return o; }", "isn't plain data"),
    ] {
        let o = go(src);
        let e = o.result.unwrap_err();
        assert_eq!(e.kind, "result", "{src}");
        assert!(e.message.contains(msg), "{src}: {}", e.message);
        assert!(o.store.is_none(), "a failed run keeps no store");
    }
}

#[test]
fn store_is_kept_only_after_success() {
    let mut i = input();
    i.store.insert("n".into(), json!(1));
    let o = run("export default (ctx) => { ctx.store.set('n', 2); throw new Error('later'); }", &i, &MockFetcher::default());
    assert!(o.store.is_none());
    let o = run("export default (ctx) => { ctx.store.set('n', ctx.store.get('n') + 1); return null; }", &i, &MockFetcher::default());
    assert_eq!(o.store.unwrap()["n"], 2);
}

#[test]
fn engine_time_is_bounded_with_slow_fetches() {
    // Pretend delays: 8 s then 8 s is over 15 s, without the test waiting.
    let routes = (1..=2).map(|k| MockRoute { method: "GET".into(), url: format!("https://api.example.com/{k}"), status: 200, delay_ms: 8000, ..Default::default() }).collect();
    let t = Instant::now();
    let o = run("export default async (ctx) => { await ctx.fetch('https://api.example.com/1'); await ctx.fetch('https://api.example.com/2'); return 'x'; }", &input(), &MockFetcher { routes });
    assert_eq!(kind(&o), "wall");
    assert!(o.wall_ms >= 15_000 && t.elapsed() < Duration::from_secs(2));
}

#[test]
fn manifest_hosts() {
    assert_eq!(parse_hosts(Some(&json!(["API.example.com.", "*.Example.org", "localhost", "192.168.1.5", "api.example.com"]))).unwrap(), vec!["api.example.com", "*.example.org", "localhost", "192.168.1.5"]);
    for bad in [json!("x"), json!(["https://x.com"]), json!(["x.com/path"]), json!(["*.com"]), json!(["a..b"]), json!(["-a.com"]), json!([1])] {
        assert!(parse_hosts(Some(&bad)).is_err(), "{bad}");
    }
    assert!(parse_hosts(None).unwrap_err().contains("needs \"hosts\""));
    assert_eq!(script_url("https://raw.githubusercontent.com/o/r/main/plugins/x/plugin.rattleboard-plugin.json", "script.js").unwrap(), "https://raw.githubusercontent.com/o/r/main/plugins/x/script.js");
    assert!(script_url("https://example.com/p.json", "../evil.js").is_err());
    assert!(check_file_name("a/b.js").is_err() && check_file_name(".js").is_err() && check_file_name("script.mjs").is_err());
}

/// The real fetcher against a local server: redirects are checked hop by hop, headers don't
/// follow a redirect to another host, cookies never come back, the size limit holds.
#[tokio::test]
async fn net_fetcher() {
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::get;
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let app = axum::Router::new()
        .route("/hello", get(|h: HeaderMap| async move { ([("set-cookie", "a=b".to_string()), ("x-ua", h.get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or("").to_string())], format!("auth={}", h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("-"))) }))
        .route("/same", get(|| async { (StatusCode::FOUND, [("location", "/hello")]).into_response() }))
        .route("/other", get(move || async move { (StatusCode::FOUND, [("location", format!("http://localhost:{port}/hello"))]).into_response() }))
        .route("/big", get(|| async { "x".repeat(MAX_RESPONSE + 1) }));
    tokio::spawn(async move { axum::serve(l, app).await });
    let base = format!("http://127.0.0.1:{port}");
    let mut i = input();
    i.hosts = vec!["127.0.0.1".into()];
    i.declared = vec!["token".into()];
    i.secrets.insert("token".into(), "s3cr3t".into());
    let src = format!(
        "export default async function (ctx) {{
          const h = {{ Authorization: 'Bearer ' + ctx.secret('token') }};
          const out = [];
          let r = await ctx.fetch('{base}/hello', {{ headers: h }});
          out.push(r.status, await r.text(), r.headers['set-cookie'] === undefined, r.headers['x-ua'].startsWith('Rattleboard/'));
          r = await ctx.fetch('{base}/same', {{ headers: h }});
          out.push(r.status, await r.text(), r.url);
          try {{ await ctx.fetch('{base}/other', {{ headers: h }}); out.push('followed'); }} catch (e) {{ out.push(e.code); }}
          try {{ await ctx.fetch('{base}/big'); out.push('big'); }} catch (e) {{ out.push(e.code); }}
          return out.join(' ');
        }}"
    );
    let handle = tokio::runtime::Handle::current();
    let o = tokio::task::spawn_blocking(move || run(&src, &i, &NetFetcher { handle })).await.unwrap();
    assert_eq!(text(&o), format!("200 auth=Bearer [secret] true true 200 auth=Bearer [secret] {base}/hello redirect_not_allowed too_large"));
    // localhost is another host: allowed, a redirect there is followed, but without the key.
    let mut i = input();
    i.hosts = vec!["127.0.0.1".into(), "localhost".into()];
    i.declared = vec!["token".into()];
    i.secrets.insert("token".into(), "s3cr3t".into());
    let src = format!("export default async (ctx) => (await (await ctx.fetch('{base}/other', {{ headers: {{ Authorization: 'Bearer ' + ctx.secret('token') }} }})).text())");
    let handle = tokio::runtime::Handle::current();
    let o = tokio::task::spawn_blocking(move || run(&src, &i, &NetFetcher { handle })).await.unwrap();
    assert_eq!(text(&o), "auth=-");
}
