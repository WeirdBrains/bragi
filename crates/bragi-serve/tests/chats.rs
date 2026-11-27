//! What an editor does with a chat besides sending (LF-40): files, a selection, an image and audio
//! sent with a turn; the chat opened with its messages; a reaction; a message deleted with its
//! answer and put back; folders; and a link code shown for another device. Memory transport, temp
//! dirs, no real network.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bragi_core::demo::{DemoNode, WARM_MODEL};
use bragi_core::{AppState, Core, PairState};
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

/// The `data:` payloads of a finished server-sent event stream.
async fn stream(app: &Router, body: Value) -> (StatusCode, Vec<Value>) {
    let res =
        tokio::time::timeout(Duration::from_secs(60), app.clone().oneshot(req("POST", "/v1/code/chat").body(Body::from(body.to_string())).unwrap()))
            .await
            .expect("answered")
            .unwrap();
    let status = res.status();
    let bytes = tokio::time::timeout(Duration::from_secs(60), res.into_body().collect()).await.expect("the answer ended").unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if status != StatusCode::OK {
        return (status, vec![serde_json::from_str(&text).unwrap_or(Value::Null)]);
    }
    (status, text.split("\n\n").filter_map(|e| e.strip_prefix("data: ")).map(|d| serde_json::from_str(d).unwrap()).collect())
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

fn png() -> String {
    let mut b = b"\x89PNG\r\n\x1a\n".to_vec();
    b.extend_from_slice(&[0u8; 32]);
    STANDARD.encode(b)
}

fn wav() -> String {
    let mut b = b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec();
    b.extend_from_slice(&[0u8; 32]);
    STANDARD.encode(b)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_chat_takes_files_and_media_opens_reacts_deletes_and_files_into_folders() {
    let d = tempfile::tempdir().unwrap();
    let (_demo, core) = linked(d.path()).await;
    let app = bragi_serve::router(core.clone(), TOKEN.into(), PORT);

    // A file, a selection, an image and a sound go with the turn.
    let attachments = json!([
        {"name": "notes.md", "text": "# Notes\nadd sums\n"},
        {"name": "selection in src/math.rs:1-2", "text": "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n"},
        {"name": "shot.png", "data": png()},
        {"name": "hi.wav", "data": wav()},
    ]);
    let (s, events) = stream(&app, json!({"prompt": "what do these say?", "model": WARM_MODEL, "attachments": attachments})).await;
    assert_eq!(s, StatusCode::OK, "{events:?}");
    assert_eq!(events.last().unwrap()["type"], "done", "{events:?}");
    let conv = events[0]["id"].as_str().unwrap().to_string();

    // Bad attachments are refused before anything is sent.
    for bad in [json!({"name": "x.bin", "data": STANDARD.encode([0u8, 1, 2, 0])}), json!({"name": "x", "data": "not base64!"}), json!({"name": "x"})]
    {
        let (s, v) = stream(&app, json!({"prompt": "hi", "attachments": [bad.clone()]})).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {v:?}");
    }

    // Opened: the messages, with what was attached.
    let (s, v) = call(&app, "GET", &format!("/v1/sessions/{conv}"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let msgs = v["conversation"]["messages"].as_array().unwrap().clone();
    assert_eq!(msgs.len(), 2, "{v}");
    let (q, a) = (msgs[0]["id"].as_str().unwrap().to_string(), msgs[1]["id"].as_str().unwrap().to_string());
    let names: Vec<&str> = msgs[0]["attachments"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap()).collect();
    for n in ["notes.md", "shot.png", "hi.wav"] {
        assert!(names.contains(&n), "{names:?}");
    }
    let kinds: Vec<&str> = msgs[0]["attachments"].as_array().unwrap().iter().filter_map(|a| a["media"]["kind"].as_str()).collect();
    assert_eq!(kinds, ["image", "audio"]);
    assert_eq!(call(&app, "GET", "/v1/sessions/nope", None).await.0, StatusCode::NOT_FOUND);

    // A reaction toggles.
    let (s, v) = call(&app, "POST", &format!("/v1/sessions/{conv}/messages/{a}/react"), Some(json!({"emoji": "👍"}))).await;
    assert_eq!((s, v["reactions"].clone()), (StatusCode::OK, json!(["👍"])), "{v}");
    let (_, v) = call(&app, "GET", &format!("/v1/sessions/{conv}"), None).await;
    assert_eq!(v["conversation"]["messages"][1]["reactions"], json!(["👍"]));
    let (_, v) = call(&app, "POST", &format!("/v1/sessions/{conv}/messages/{a}/react"), Some(json!({"emoji": "👍"}))).await;
    assert_eq!(v["reactions"], json!([]));
    assert_eq!(call(&app, "POST", &format!("/v1/sessions/{conv}/messages/nope/react"), Some(json!({"emoji": "👍"}))).await.0, StatusCode::NOT_FOUND);

    // The question goes with its answer, and comes back with undo.
    let (s, v) = call(&app, "DELETE", &format!("/v1/sessions/{conv}/messages/{q}?with_reply=true"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["ids"], json!([q, a]));
    let undo = v["undo"].as_str().unwrap().to_string();
    let (s, v) = call(&app, "GET", &format!("/v1/sessions/{conv}"), None).await;
    assert!(s == StatusCode::NOT_FOUND || v["conversation"]["messages"].as_array().is_some_and(Vec::is_empty), "{s} {v}");
    assert_eq!(call(&app, "POST", "/v1/undo", Some(json!({"undo": undo}))).await.0, StatusCode::OK);
    let (_, v) = call(&app, "GET", &format!("/v1/sessions/{conv}"), None).await;
    assert_eq!(v["conversation"]["messages"].as_array().map(Vec::len), Some(2), "{v}");

    // Folders: made, renamed, nested, a chat filed in one, deleted (the chat stays).
    let (s, top) = call(&app, "POST", "/v1/folders", Some(json!({"name": "Work"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{top}");
    let top = top["id"].as_str().unwrap().to_string();
    let (_, sub) = call(&app, "POST", "/v1/folders", Some(json!({"name": "Bragi", "parent": top}))).await;
    let sub = sub["id"].as_str().unwrap().to_string();
    assert_eq!(call(&app, "POST", &format!("/v1/folders/{sub}/rename"), Some(json!({"name": "Bragi code"}))).await.0, StatusCode::OK);
    assert_eq!(call(&app, "POST", &format!("/v1/sessions/{conv}/folder"), Some(json!({"folder": sub}))).await.0, StatusCode::OK);
    let (_, v) = call(&app, "GET", &format!("/v1/sessions/{conv}"), None).await;
    assert_eq!(v["folder"], sub.as_str());
    let (_, v) = call(&app, "GET", "/v1/sessions", None).await;
    assert_eq!(v["sessions"][0]["folder"], sub.as_str());
    let (_, v) = call(&app, "GET", "/v1/folders", None).await;
    let fs = v["folders"].as_array().unwrap();
    assert!(fs.iter().any(|f| f["name"] == "Bragi code" && f["parent"] == top.as_str()), "{v}");
    assert_eq!(call(&app, "POST", &format!("/v1/folders/{sub}/move"), Some(json!({"parent": null}))).await.0, StatusCode::OK);
    assert_eq!(call(&app, "DELETE", &format!("/v1/folders/{sub}"), None).await.0, StatusCode::OK);
    let (_, v) = call(&app, "GET", &format!("/v1/sessions/{conv}"), None).await;
    assert_eq!(v["conversation"]["messages"].as_array().map(Vec::len), Some(2), "deleting a folder keeps the chat");
    assert_eq!(call(&app, "POST", &format!("/v1/sessions/{conv}/folder"), Some(json!({"folder": null}))).await.0, StatusCode::OK);

    // Already linked: no code to show.
    assert_ne!(call(&app, "POST", "/v1/link/offer", None).await.0, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_code_is_shown_compared_and_confirmed_or_cancelled() {
    let d = tempfile::tempdir().unwrap();
    let demo = DemoNode::start_cloud(&d.path().join("demo"), Duration::from_millis(2)).await.unwrap();
    let core = Core::start(demo.app_config(&d.path().join("app"), "Bragi serve test")).unwrap();
    let app = bragi_serve::router(core.clone(), TOKEN.into(), PORT);

    // Nothing to confirm before a code is shown.
    assert_eq!(call(&app, "POST", "/v1/link/offer/confirm", Some(json!({"matches": true}))).await.0, StatusCode::CONFLICT);

    // Shown, then cancelled.
    let (s, v) = call(&app, "POST", "/v1/link/offer", None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(!v["link"].as_str().unwrap().is_empty());
    assert!(v["expires_ms"].as_i64().is_some(), "{v}");
    let (_, st) = call(&app, "GET", "/v1/state", None).await;
    assert_eq!((st["pair"]["state"].as_str(), &st["pair"]["link"]), (Some("offering"), &v["link"]), "{st}");
    let (s, v) = call(&app, "POST", "/v1/link/offer/cancel", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["pair"]["state"], "unpaired", "{v}");

    // Shown again; the other device (the demo) reads it; the codes match and it links.
    demo.auto_accept(&core);
    let (s, _) = call(&app, "POST", "/v1/link/offer", None).await;
    assert_eq!(s, StatusCode::OK);
    until(&core, "the code check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    let (_, st) = call(&app, "GET", "/v1/state", None).await;
    assert!(st["pair"]["sas"].as_str().is_some_and(|s| !s.is_empty()), "{st}");
    let (s, v) = call(&app, "POST", "/v1/link/offer/confirm", Some(json!({"matches": true}))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    until(&core, "linked", |s| matches!(s.pair, PairState::Linked { .. })).await;
    assert_eq!(call(&app, "GET", "/v1/state", None).await.1["linked"], true);
}
