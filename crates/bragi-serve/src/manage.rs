//! Management routes for editors (LF-40): the links, OpenAI-compatible endpoints and sessions the
//! apps keep, so an editor extension can set Bragi up without its own copy. Each route calls the
//! `Core` method the apps use. Keys arrive once in a request body, go to Bragi's secret store and
//! never come back: responses are `EndpointView`s (only "a key is set").

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use bragi_core::{EndpointInput, EndpointMode, KeyChange, Target};
use serde::Deserialize;
use serde_json::json;

use super::{App, core_error};

pub(crate) fn routes() -> Router<App> {
    Router::new()
        .route("/v1/links", get(links))
        .route("/v1/links/node", post(add_node))
        .route("/v1/links/remove", post(remove_link))
        .route("/v1/link/accept", post(link_accept))
        .route("/v1/link/confirm", post(link_confirm))
        .route("/v1/endpoints", get(endpoints).post(add_endpoint))
        .route("/v1/endpoints/test", post(test_address))
        .route("/v1/endpoints/{id}", axum::routing::put(edit_endpoint).delete(delete_endpoint))
        .route("/v1/endpoints/{id}/test", post(test_endpoint))
        .route("/v1/endpoints/{id}/models", get(endpoint_models))
        .route("/v1/sessions", get(sessions))
        .route("/v1/sessions/{id}", get(super::chats::open).delete(delete_session))
}

async fn links(State(app): State<App>) -> Json<serde_json::Value> {
    Json(json!({ "links": app.core.links() }))
}

#[derive(Deserialize)]
struct Address {
    address: String,
}

async fn add_node(State(app): State<App>, Json(b): Json<Address>) -> Response {
    match app.core.add_node(&b.address) {
        Ok(line) => Json(json!({ "message": line })).into_response(),
        Err(e) => core_error(e),
    }
}

async fn remove_link(State(app): State<App>, Json(b): Json<Address>) -> Response {
    match app.core.remove_link(&b.address) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => core_error(e),
    }
}

#[derive(Deserialize)]
struct Code {
    code: String,
}

/// Reads another device's link code; the answer's `sas` is compared on both screens.
async fn link_accept(State(app): State<App>, Json(b): Json<Code>) -> Response {
    match app.core.pair_accept(&b.code).await {
        Ok(check) => Json(json!({ "sas": check.sas, "device": check.device })).into_response(),
        Err(e) => core_error(e),
    }
}

#[derive(Deserialize)]
struct Confirm {
    matches: bool,
}

async fn link_confirm(State(app): State<App>, Json(b): Json<Confirm>) -> Response {
    match app.core.pair_accept_confirm(b.matches).await {
        Ok(()) => Json(json!({ "ok": true, "linked": b.matches })).into_response(),
        Err(e) => core_error(e),
    }
}

// ---- endpoints ---------------------------------------------------------------------------------

#[derive(Deserialize)]
struct EndpointBody {
    name: String,
    base_url: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    mode: EndpointMode,
    #[serde(default = "yes")]
    enabled: bool,
    /// Set the key (add and edit). Never echoed.
    #[serde(default)]
    api_key: Option<String>,
    /// Edit only: forget the stored key.
    #[serde(default)]
    clear_key: bool,
}

fn yes() -> bool {
    true
}

impl EndpointBody {
    fn input(&self) -> EndpointInput {
        EndpointInput {
            name: self.name.clone(),
            base_url: self.base_url.clone(),
            model: self.model.clone().filter(|m| !m.trim().is_empty()),
            mode: self.mode,
            enabled: self.enabled,
        }
    }
}

async fn endpoints(State(app): State<App>) -> Json<serde_json::Value> {
    Json(json!({ "endpoints": app.core.endpoints() }))
}

async fn add_endpoint(State(app): State<App>, Json(b): Json<EndpointBody>) -> Response {
    let key = b.api_key.clone().filter(|k| !k.trim().is_empty());
    match app.core.add_endpoint(b.input(), key) {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => core_error(e),
    }
}

async fn edit_endpoint(State(app): State<App>, Path(id): Path<String>, Json(b): Json<EndpointBody>) -> Response {
    let key = match (&b.api_key, b.clear_key) {
        (Some(k), _) if !k.trim().is_empty() => KeyChange::Set(k.clone()),
        (_, true) => KeyChange::Clear,
        _ => KeyChange::Keep,
    };
    match app.core.edit_endpoint(&id, b.input(), key) {
        Ok(v) => Json(v).into_response(),
        Err(e) => core_error(e),
    }
}

async fn delete_endpoint(State(app): State<App>, Path(id): Path<String>) -> Response {
    match app.core.delete_endpoint(&id, None::<Target>) {
        Ok(r) => Json(r).into_response(),
        Err(e) => core_error(e),
    }
}

async fn test_endpoint(State(app): State<App>, Path(id): Path<String>) -> Response {
    match app.core.test_endpoint(&id).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => core_error(e),
    }
}

#[derive(Deserialize)]
struct TestBody {
    base_url: String,
    #[serde(default)]
    api_key: Option<String>,
}

/// The add form's Test button: an address and key that aren't saved.
async fn test_address(State(app): State<App>, Json(b): Json<TestBody>) -> Response {
    match app.core.test_endpoint_address(&b.base_url, b.api_key).await {
        Ok(r) => Json(r).into_response(),
        Err(e) => core_error(e),
    }
}

async fn endpoint_models(State(app): State<App>, Path(id): Path<String>) -> Response {
    match app.core.endpoint_models(&id).await {
        Ok(m) => Json(json!({ "models": m })).into_response(),
        Err(e) => core_error(e),
    }
}

// ---- sessions ----------------------------------------------------------------------------------

/// `?offset=N&limit=N` (the `query` feature of axum isn't on, so it is read by hand).
fn page(q: Option<String>) -> (usize, usize) {
    let (mut offset, mut limit) = (0, 50);
    for pair in q.unwrap_or_default().split('&') {
        match pair.split_once('=') {
            Some(("offset", v)) => offset = v.parse().unwrap_or(0),
            Some(("limit", v)) => limit = v.parse().unwrap_or(50),
            _ => {}
        }
    }
    (offset, limit.min(200))
}

async fn sessions(State(app): State<App>, RawQuery(q): RawQuery) -> Response {
    let (offset, limit) = page(q);
    match app.core.sessions(offset, limit) {
        Ok(s) => Json(json!({ "sessions": s })).into_response(),
        Err(e) => core_error(e),
    }
}

async fn delete_session(State(app): State<App>, Path(id): Path<String>) -> Response {
    match app.core.delete_conversation(&id) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => core_error(e),
    }
}
