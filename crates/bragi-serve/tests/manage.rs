//! The management routes an editor uses (LF-40): endpoints, links and sessions. Memory transport,
//! temp dirs, no real network.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const PORT: u16 = 4877;

fn app(dir: &std::path::Path) -> Router {
    let core = bragi_core::Core::start(bragi_core::Config { transport: bragi_core::Transport::Memory, ..bragi_core::Config::new(dir, "t") }).unwrap();
    bragi_serve::router(core, TOKEN.into(), PORT)
}

async fn call(app: &Router, method: &str, path: &str, body: Option<Value>, auth: bool) -> (StatusCode, Value) {
    let mut r = Request::builder().method(method).uri(path).header("host", format!("127.0.0.1:{PORT}")).header("content-type", "application/json");
    if auth {
        r = r.header("authorization", format!("Bearer {TOKEN}"));
    }
    let body = body.map(|b| Body::from(b.to_string())).unwrap_or_else(Body::empty);
    let res = app.clone().oneshot(r.body(body).unwrap()).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test(flavor = "multi_thread")]
async fn management_routes_want_the_token() {
    let d = tempfile::tempdir().unwrap();
    let app = app(d.path());
    for (m, p) in [("GET", "/v1/endpoints"), ("GET", "/v1/links"), ("GET", "/v1/sessions"), ("POST", "/v1/link/accept")] {
        assert_eq!(call(&app, m, p, Some(json!({})), false).await.0, StatusCode::UNAUTHORIZED, "{p}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn endpoints_are_added_edited_and_deleted_and_the_key_never_comes_back() {
    let d = tempfile::tempdir().unwrap();
    let app = app(d.path());
    let (s, v) = call(&app, "GET", "/v1/endpoints", None, true).await;
    assert_eq!((s, v["endpoints"].as_array().unwrap().len()), (StatusCode::OK, 0));

    let secret = "sk-test-not-a-real-key-123";
    let body = json!({ "name": "Local", "base_url": "http://127.0.0.1:8484/v1", "model": "m1", "mode": "alongside", "api_key": secret });
    let (s, v) = call(&app, "POST", "/v1/endpoints", Some(body), true).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["has_key"], true);
    assert!(!v.to_string().contains(secret));
    let id = v["id"].as_str().unwrap().to_string();

    let (_, v) = call(&app, "GET", "/v1/endpoints", None, true).await;
    assert!(!v.to_string().contains(secret));
    assert_eq!(v["endpoints"][0]["name"], "Local");

    // Keep the key, rename.
    let edit = json!({ "name": "Renamed", "base_url": "http://127.0.0.1:8484/v1", "mode": "fallback" });
    let (s, v) = call(&app, "PUT", &format!("/v1/endpoints/{id}"), Some(edit), true).await;
    assert_eq!((s, v["name"].clone(), v["has_key"].clone()), (StatusCode::OK, json!("Renamed"), json!(true)));
    // Clear it.
    let edit = json!({ "name": "Renamed", "base_url": "http://127.0.0.1:8484/v1", "clear_key": true });
    let (_, v) = call(&app, "PUT", &format!("/v1/endpoints/{id}"), Some(edit), true).await;
    assert_eq!(v["has_key"], false);

    let (s, _) = call(&app, "POST", "/v1/endpoints", Some(json!({ "name": "Bad", "base_url": "ftp://x" })), true).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (s, _) = call(&app, "DELETE", &format!("/v1/endpoints/{id}"), None, true).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = call(&app, "DELETE", &format!("/v1/endpoints/{id}"), None, true).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn links_and_sessions_start_empty_and_bad_input_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let app = app(d.path());
    let (_, v) = call(&app, "GET", "/v1/links", None, true).await;
    assert_eq!(v["links"].as_array().unwrap().len(), 0);
    // History is in the context store, which is attached only after the device is linked.
    let (s, v) = call(&app, "GET", "/v1/sessions?limit=5", None, true).await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{v}");
    assert_eq!(v["error"]["kind"], "not_paired");
    let (s, _) = call(&app, "POST", "/v1/links/remove", Some(json!({ "address": "/ip4/1.2.3.4/tcp/1" })), true).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(&app, "POST", "/v1/links/node", Some(json!({ "address": "not an address" })), true).await;
    assert!(s.is_client_error(), "{s}");
    let (s, _) = call(&app, "POST", "/v1/link/confirm", Some(json!({ "matches": true })), true).await;
    assert!(s.is_client_error() || s.is_server_error(), "no link in progress: {s}");
}
