//! `bragi serve` against the demo inference nodes (memory transport, test identity): the guard
//! (token, loopback Host, no Origin), the state, suggestions, rewrites, chat with editor context
//! kept in history, and the OpenAI-compatible routes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use bragi_core::demo::DemoNode;
use bragi_core::{AppState, Core, PairState};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

// The core's mock OpenAI-compatible server (loopback, no real network).
#[path = "../../bragi-core/tests/common/mod.rs"]
mod common;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const PORT: u16 = 4877;

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

fn req(method: &str, path: &str) -> axum::http::request::Builder {
    Request::builder().method(method).uri(path).header("host", format!("127.0.0.1:{PORT}"))
}

async fn call(app: &Router, method: &str, path: &str, body: Option<Value>) -> (StatusCode, String) {
    let r = req(method, path).header("authorization", format!("Bearer {TOKEN}")).header("content-type", "application/json");
    send(app, r, body).await
}

async fn send(app: &Router, r: axum::http::request::Builder, body: Option<Value>) -> (StatusCode, String) {
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let res = tokio::time::timeout(Duration::from_secs(60), app.clone().oneshot(r.body(body).unwrap())).await.expect("answered").unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn json_of(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|e| panic!("{e}: {s}"))
}

/// The `data:` payloads of a server-sent event stream.
fn events(s: &str) -> Vec<String> {
    s.split("\n\n").filter_map(|e| e.strip_prefix("data: ")).map(str::to_string).collect()
}

fn context() -> Value {
    json!({
        "path": "src/math.rs",
        "language": "rust",
        "before": "pub fn add(a: i32, b: i32) -> i32 {\n",
        "selection": "    a - b\n",
        "after": "}\n",
        "open_files": [{ "path": "src/lib.rs", "language": "rust", "text": "pub mod math;\n" }],
        "diagnostics": [{ "line": 2, "severity": "warning", "message": "suspicious subtraction in add" }],
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn the_guard_wants_the_token_a_loopback_host_and_no_origin() {
    let d = tempfile::tempdir().unwrap();
    let core = Core::start(bragi_core::Config { transport: bragi_core::Transport::Memory, ..bragi_core::Config::new(d.path(), "t") }).unwrap();
    let app = bragi_serve::router(core, TOKEN.into(), PORT);

    let (s, b) = send(&app, req("GET", "/v1/health"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(json_of(&b)["ok"], true);

    assert_eq!(send(&app, req("GET", "/v1/state"), None).await.0, StatusCode::UNAUTHORIZED);
    let wrong = req("GET", "/v1/state").header("authorization", "Bearer nope");
    assert_eq!(send(&app, wrong, None).await.0, StatusCode::UNAUTHORIZED);
    let web = req("GET", "/v1/health").header("origin", "https://example.com");
    assert_eq!(send(&app, web, None).await.0, StatusCode::FORBIDDEN);
    let rebound = Request::builder().uri("/v1/state").header("host", "attacker.example:4877").header("authorization", format!("Bearer {TOKEN}"));
    assert_eq!(send(&app, rebound, None).await.0, StatusCode::FORBIDDEN);

    // Not linked: the state says so, and asking for code says what to do.
    let (s, b) = call(&app, "GET", "/v1/state", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(json_of(&b)["linked"], false);
    let (s, b) = call(&app, "POST", "/v1/code/suggest", Some(json!({ "context": context() }))).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    assert_eq!(json_of(&b)["error"]["kind"], "not_paired");
}

#[tokio::test(flavor = "multi_thread")]
async fn suggestions_and_rewrites_come_from_the_pool_and_stay_out_of_history() {
    let d = tempfile::tempdir().unwrap();
    let (_demo, core) = linked(d.path()).await;
    let app = bragi_serve::router(core.clone(), TOKEN.into(), PORT);

    let (s, b) = call(&app, "GET", "/v1/state", None).await;
    let st = json_of(&b);
    assert_eq!(s, StatusCode::OK);
    assert_eq!(st["linked"], true);
    assert!(st["models"].as_array().unwrap().len() >= 2, "{st}");
    let model = st["default_model"].as_str().unwrap().to_string();
    let (_, b) = call(&app, "GET", "/v1/targets", None).await;
    let t = json_of(&b);
    assert_eq!(t["targets"][0]["target"]["kind"], "pool", "{t}");
    assert_eq!(t["targets"][0]["label"], "Weirdnet pool");
    assert!(!t["targets"][0]["models"].as_array().unwrap().is_empty());

    let (s, b) = call(&app, "POST", "/v1/code/suggest", Some(json!({ "context": context() }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let sug = json_of(&b);
    assert!(sug["text"].as_str().unwrap().contains("Bragi asks the inference nodes"), "{sug}");
    assert_eq!(sug["model"], model.as_str());

    let (s, b) =
        call(&app, "POST", "/v1/code/rewrite", Some(json!({ "context": context(), "instruction": "fix the bug", "scope": "selection" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let rw = json_of(&b);
    let code = rw["code"].as_str().unwrap();
    assert!(code.ends_with('\n'), "keeps the selection's newline");
    assert_eq!(rw["scope"], "selection");
    assert_eq!(rw["file"].as_str().unwrap(), format!("pub fn add(a: i32, b: i32) -> i32 {{\n{code}}}\n"));
    let diff = rw["diff"].as_str().unwrap();
    assert!(diff.starts_with("--- a/src/math.rs\n+++ b/src/math.rs\n@@ "), "{diff}");
    assert!(diff.contains("\n-    a - b\n"), "{diff}");
    assert_eq!(rw["stats"]["removed"], 1);

    let (s, b) = call(&app, "POST", "/v1/code/rewrite", Some(json!({ "context": context(), "instruction": " " }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");

    // Code answers are not conversations.
    assert!(core.conversations(0, 10).unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_with_editor_context_streams_and_lands_in_history() {
    let d = tempfile::tempdir().unwrap();
    let (_demo, core) = linked(d.path()).await;
    let app = bragi_serve::router(core.clone(), TOKEN.into(), PORT);

    let (s, b) = call(&app, "POST", "/v1/code/chat", Some(json!({ "prompt": "What does add do?", "context": context() }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let evs: Vec<Value> = events(&b).iter().map(|e| json_of(e)).collect();
    assert_eq!(evs[0]["type"], "conversation");
    let id = evs[0]["id"].as_str().unwrap().to_string();
    assert!(evs.iter().any(|e| e["type"] == "delta"));
    assert_eq!(evs.last().unwrap()["type"], "done", "{b}");
    let text: String = evs.iter().filter(|e| e["type"] == "delta").map(|e| e["text"].as_str().unwrap()).collect();
    assert!(text.contains("I read src∕math.rs"), "the model saw the file: {text}");

    let c = core.conversation(&id).unwrap();
    assert_eq!(c.messages.len(), 2);
    let names: Vec<_> = c.messages[0].attachments.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["src∕math.rs", "selection in src∕math.rs", "problems in src∕math.rs", "src∕lib.rs"]);

    // The next turn continues it.
    let (_, b) = call(&app, "POST", "/v1/code/chat", Some(json!({ "prompt": "And sub?", "conversation": id }))).await;
    assert_eq!(json_of(&events(&b)[0])["id"], id.as_str());
    assert_eq!(core.conversation(&id).unwrap().messages.len(), 4);

    let (_, b) = call(&app, "POST", "/v1/cancel", Some(json!({ "request_id": "r-gone" }))).await;
    assert_eq!(json_of(&b)["cancelled"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn openai_compatible_routes() {
    let d = tempfile::tempdir().unwrap();
    let (_demo, core) = linked(d.path()).await;
    let app = bragi_serve::router(core.clone(), TOKEN.into(), PORT);

    let (_, b) = call(&app, "GET", "/v1/models", None).await;
    let m = json_of(&b);
    assert_eq!(m["object"], "list");
    assert!(m["data"].as_array().unwrap().iter().any(|x| x["id"] == bragi_core::demo::WARM_MODEL));

    let body = json!({ "model": bragi_core::demo::WARM_MODEL, "messages": [{ "role": "system", "content": "be brief" }, { "role": "user", "content": [{ "type": "text", "text": "hello" }] }] });
    let (s, b) = call(&app, "POST", "/v1/chat/completions", Some(body.clone())).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let r = json_of(&b);
    assert_eq!(r["choices"][0]["message"]["content"], bragi_core::demo::answer_for(bragi_core::demo::WARM_MODEL, "hello"));
    assert_eq!(r["choices"][0]["finish_reason"], "stop");

    let mut stream = body;
    stream["stream"] = json!(true);
    let (s, b) = call(&app, "POST", "/v1/chat/completions", Some(stream)).await;
    assert_eq!(s, StatusCode::OK);
    let evs = events(&b);
    assert_eq!(evs.last().unwrap(), "[DONE]");
    let text: String =
        evs.iter().filter(|e| e.starts_with('{')).filter_map(|e| json_of(e)["choices"][0]["delta"]["content"].as_str().map(str::to_string)).collect();
    assert_eq!(text, bragi_core::demo::answer_for(bragi_core::demo::WARM_MODEL, "hello"));
    assert!(core.conversations(0, 10).unwrap().is_empty());

    let (s, _) = call(&app, "POST", "/v1/chat/completions", Some(json!({ "messages": [{ "role": "tool", "content": "x" }] }))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread")]
async fn it_listens_on_loopback_over_a_real_socket() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let d = tempfile::tempdir().unwrap();
    let core = Core::start(bragi_core::Config { transport: bragi_core::Transport::Memory, ..bragi_core::Config::new(d.path(), "t") }).unwrap();
    let listener = bragi_serve::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(bragi_serve::serve(core, TOKEN.into(), listener, async move {
        let _ = stopped.await;
    }));
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(format!("GET /v1/state HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {TOKEN}\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    assert!(out.contains("\"linked\":false"), "{out}");
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn code_requests_go_to_the_target_they_name() {
    let d = tempfile::tempdir().unwrap();
    let (_demo, core) = linked(d.path()).await;
    let mock = common::mock_openai(None, vec!["mock-1"], common::Chat::Words(vec!["```rust\n", "    a + b\n", "```"])).await;
    let ep = core
        .add_endpoint(
            bragi_core::EndpointInput {
                name: "Mock".into(),
                base_url: mock.url.clone(),
                model: Some("mock-1".into()),
                mode: bragi_core::EndpointMode::Alongside,
                enabled: true,
            },
            None,
        )
        .unwrap();
    let app = bragi_serve::router(core.clone(), TOKEN.into(), PORT);

    let (_, b) = call(&app, "GET", "/v1/targets", None).await;
    let t = json_of(&b);
    let target = t["targets"][1]["target"].clone();
    assert_eq!(target, json!({ "kind": "endpoint", "id": ep.id, "model": "mock-1" }), "{t}");
    assert_eq!(t["targets"][1]["label"], "Mock · mock-1");

    let (s, b) = call(&app, "POST", "/v1/code/rewrite", Some(json!({ "context": context(), "instruction": "fix the bug", "target": target }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let rw = json_of(&b);
    assert_eq!(rw["code"], "    a + b\n");
    assert_eq!(rw["model"], "mock-1");
    let asked = mock.requests().into_iter().filter(|r| r.path.ends_with("/chat/completions")).count();
    assert_eq!(asked, 1, "the endpoint answered, not the pool");
    assert!(core.conversations(0, 10).unwrap().is_empty());
}
