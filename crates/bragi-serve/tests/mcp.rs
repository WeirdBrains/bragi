//! Outside MCP servers and agent approvals through `bragi serve` (LF-40 on LF-37): the list and
//! its secrets, on/off, "always allow" given and taken back, a test that can't connect, and an
//! agent task on the demo cloud whose approval card is answered over HTTP. Memory transport,
//! temp dirs, no real network.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use bragi_core::demo::DemoNode;
use bragi_core::{AppState, Core, PairState};
use futures::StreamExt;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const PORT: u16 = 4877;

fn req(method: &str, path: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(path)
        .header("host", format!("127.0.0.1:{PORT}"))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TOKEN}"))
}

async fn call(app: &Router, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let res =
        tokio::time::timeout(Duration::from_secs(60), app.clone().oneshot(req(method, path).body(body).unwrap())).await.expect("answered").unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread")]
async fn servers_are_added_switched_allowed_tested_and_removed_and_secrets_never_come_back() {
    let d = tempfile::tempdir().unwrap();
    let core = Core::start(bragi_core::Config { transport: bragi_core::Transport::Memory, ..bragi_core::Config::new(d.path(), "t") }).unwrap();
    let app = bragi_serve::router(core.clone(), TOKEN.into(), PORT);

    let (s, v) = call(&app, "GET", "/v1/mcp-servers", None).await;
    assert_eq!((s, v["servers"].as_array().map(Vec::len)), (StatusCode::OK, Some(0)), "{v}");

    // A web server with a bearer token: the token is stored, never shown.
    let (s, v) = call(
        &app,
        "POST",
        "/v1/mcp-servers",
        Some(json!({"name": "Docs", "transport": "http", "url": "http://127.0.0.1:9/mcp", "secrets": {"bearer": "tok-secret-123456"}})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    assert_eq!(v["has_bearer"], true);
    assert_eq!(v["scope"]["kind"], "all");
    let (_, all) = call(&app, "GET", "/v1/mcp-servers", None).await;
    assert!(!all.to_string().contains("tok-secret-123456"), "{all}");

    // A local command scoped to a folder.
    let (s, v) = call(&app, "POST", "/v1/mcp-servers", Some(json!({"name": "Files", "transport": "stdio", "command": "bragi-no-such-command", "args": ["--root", "/p"], "scope": "folder", "folder": "/p", "secrets": {"env": {"API_KEY": "env-secret-98765"}}}))).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!((v["scope"]["kind"].as_str(), v["scope"]["value"].as_str()), (Some("folder"), Some("/p")));
    assert_eq!(v["env_names"], json!(["API_KEY"]));
    assert!(v["warning"].as_str().is_some_and(|w| !w.is_empty()), "a local command says it runs as you: {v}");
    let files = v["id"].as_str().unwrap().to_string();
    assert!(!call(&app, "GET", "/v1/mcp-servers", None).await.1.to_string().contains("env-secret-98765"));

    // Bad input is refused.
    for bad in [
        json!({"name": "x", "transport": "stdio", "scope": "folder", "command": "c"}),
        json!({"name": "x", "transport": "http", "url": "http://example.com/mcp"}),
        json!({"name": "x", "transport": "pigeon"}),
    ] {
        let (s, v) = call(&app, "POST", "/v1/mcp-servers", Some(bad.clone())).await;
        assert!(s == StatusCode::BAD_REQUEST || s == StatusCode::UNPROCESSABLE_ENTITY, "{bad} -> {s} {v}");
    }

    // Off and on.
    let (s, v) = call(&app, "POST", &format!("/v1/mcp-servers/{id}/enabled"), Some(json!({"enabled": false}))).await;
    assert_eq!((s, v["enabled"].as_bool()), (StatusCode::OK, Some(false)), "{v}");
    assert_eq!(call(&app, "POST", &format!("/v1/mcp-servers/{id}/enabled"), Some(json!({"enabled": true}))).await.1["enabled"], true);

    // "Always allow" given and taken back.
    let (s, v) = call(&app, "POST", &format!("/v1/mcp-servers/{id}/always-allow"), Some(json!({"tool": "docs.search", "allow": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["always_allow"], json!(["docs.search"]));
    let (_, v) = call(&app, "POST", &format!("/v1/mcp-servers/{id}/always-allow"), Some(json!({"tool": "docs.search", "allow": false}))).await;
    assert_eq!(v["always_allow"], json!([]));

    // An edit without secrets keeps the token.
    let (s, v) =
        call(&app, "PUT", &format!("/v1/mcp-servers/{id}"), Some(json!({"name": "Docs", "transport": "http", "url": "http://127.0.0.1:9/v2/mcp"})))
            .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["has_bearer"], true);
    assert!(v["target"].as_str().unwrap().contains("/v2/mcp"), "{v}");

    // A test that can't connect reports why (not an error status).
    let (s, v) = call(&app, "POST", &format!("/v1/mcp-servers/{files}/test"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["ok"], false);
    assert!(!v["message"].as_str().unwrap().is_empty());
    assert_eq!(call(&app, "POST", "/v1/mcp-servers/nope/test", None).await.0, StatusCode::NOT_FOUND);

    // Removed.
    assert_eq!(call(&app, "DELETE", &format!("/v1/mcp-servers/{id}"), None).await.0, StatusCode::OK);
    assert_eq!(call(&app, "DELETE", &format!("/v1/mcp-servers/{files}"), None).await.0, StatusCode::OK);
    assert_eq!(call(&app, "GET", "/v1/mcp-servers", None).await.1["servers"], json!([]));
    assert_eq!(call(&app, "POST", &format!("/v1/mcp-servers/{id}/enabled"), Some(json!({"enabled": true}))).await.0, StatusCode::NOT_FOUND);
    core.shutdown();
}

async fn until(core: &Core, what: &str, f: impl Fn(&AppState) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut w = core.watch();
    loop {
        let s = w.borrow_and_update().clone();
        if f(&s) {
            return;
        }
        assert!(Instant::now() < deadline, "waiting for {what}: {s:?}");
        let _ = tokio::time::timeout(Duration::from_millis(200), w.changed()).await;
    }
}

async fn linked(dir: &std::path::Path) -> (Arc<DemoNode>, Core) {
    let demo = DemoNode::start_cloud(&dir.join("demo"), Duration::from_millis(2)).await.unwrap();
    let core = Core::start(demo.app_config(&dir.join("app"), "Bragi serve test")).unwrap();
    demo.auto_accept(&core);
    core.pair_offer().await.unwrap();
    until(&core, "the code check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    core.pair_confirm(true).unwrap();
    until(&core, "the catalog", |s| matches!(s.pair, PairState::Linked { .. }) && s.catalog.default_model().is_some()).await;
    (demo, core)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_chat_streams_its_approval_card_and_the_answer_lets_the_tool_run() {
    let d = tempfile::tempdir().unwrap();
    let (_demo, core) = linked(d.path()).await;
    let app = bragi_serve::router(core.clone(), TOKEN.into(), PORT);

    let body = json!({"prompt": "find my notes about groceries", "agent": true});
    let res = app.clone().oneshot(req("POST", "/v1/code/chat").body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let mut stream = res.into_body().into_data_stream();
    let mut buf = String::new();
    let mut events: Vec<Value> = Vec::new();
    let mut request_id = String::new();
    let mut answered = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    'read: while let Some(chunk) = tokio::time::timeout(deadline - Instant::now(), stream.next()).await.expect("stream stalled") {
        buf.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        while let Some(end) = buf.find("\n\n") {
            let frame: String = buf.drain(..end + 2).collect();
            let Some(data) = frame.trim().strip_prefix("data: ") else { continue };
            let ev: Value = serde_json::from_str(data).unwrap();
            match ev["type"].as_str() {
                Some("conversation") => request_id = ev["request_id"].as_str().unwrap().to_string(),
                Some("approval") => {
                    assert_eq!(ev["tool"], "notes.search");
                    assert_eq!(ev["always"], false, "a node's own tool: always allow isn't offered");
                    let (s, v) =
                        call(&app, "POST", "/v1/agent/approve", Some(json!({"request_id": request_id, "ask": ev["ask"], "choice": "once"}))).await;
                    assert_eq!(s, StatusCode::OK, "{v}");
                    let (s, _) =
                        call(&app, "POST", "/v1/agent/approve", Some(json!({"request_id": request_id, "ask": ev["ask"], "choice": "once"}))).await;
                    assert_eq!(s, StatusCode::NOT_FOUND, "a card is answered once");
                    answered = true;
                }
                _ => {}
            }
            let end = matches!(ev["type"].as_str(), Some("done" | "error"));
            events.push(ev);
            if end {
                break 'read;
            }
        }
    }
    assert!(answered, "{events:?}");
    assert_eq!(events.last().unwrap()["type"], "done", "{events:?}");
    assert!(events.iter().any(|e| e["type"] == "tool" && e["state"] == "done"), "the tool ran: {events:?}");
    assert!(events.iter().filter(|e| e["type"] == "delta").any(|e| e["text"].as_str().unwrap().contains("2 notes mention")), "{events:?}");
    let (s, _) = call(&app, "POST", "/v1/agent/approve", Some(json!({"request_id": "nope", "ask": 1, "choice": "deny"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&app, "POST", "/v1/agent/approve", Some(json!({"request_id": request_id, "ask": 1, "choice": "sometimes"}))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    core.shutdown();
}
