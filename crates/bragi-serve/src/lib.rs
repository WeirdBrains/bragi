//! `bragi serve`: a small HTTP API on loopback for editor extensions and other local tools.
//! Everything goes through [`Core`], so requests reach the same Weirdnet pool (and, later, the
//! same endpoints and sessions) as the app, and chats land in the same history.
//!
//! - Loopback only: [`bind`] refuses any other address, requests must name a loopback `Host`
//!   (no DNS rebinding), and any request with an `Origin` header is refused (no web page can
//!   reach it, even one that guessed the token).
//! - Every route but `/v1/health` needs `Authorization: Bearer <token>`; the token lives in
//!   `<home>/serve/token` (mode 0600), made on first start.
//!
//! Routes:
//! - `GET  /v1/health`: `{ok, version}`.
//! - `GET  /v1/state`: linked or not, the models and their status, the default model.
//! - `GET  /v1/targets`: where requests can go (the Weirdnet pool for now).
//! - `GET  /v1/models`, `POST /v1/chat/completions`: OpenAI-compatible (streamed or not), kept
//!   out of history, so any OpenAI client can use Bragi.
//! - `POST /v1/code/suggest`, `/v1/code/rewrite`: [`bragi_core::code`] on an [`EditorContext`].
//! - `POST /v1/code/chat`: a chat turn with editor context, kept in history, streamed as
//!   server-sent [`ChatEvent`]s.
//! - `POST /v1/cancel`: stops a streaming request.

pub mod cli;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bragi_core::code::{self, EditorContext, PromptMessage, PromptRole, Scope};
use bragi_core::{ChatEvent, ChatHandle, Core, CoreError, ErrorKind, PairState};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The port `bragi serve` and the editor extension use unless told otherwise.
pub const DEFAULT_PORT: u16 = 4877;
/// Largest request body (an editor context with a few open files fits easily).
pub const MAX_BODY: usize = 4 * 1024 * 1024;
/// A suggestion or rewrite that hasn't finished by then is stopped.
pub const ANSWER_TIMEOUT: Duration = Duration::from_secs(300);

/// Where the token lives under a Bragi home.
pub fn token_path(home: &Path) -> PathBuf {
    home.join("serve").join("token")
}

/// Reads the token, making one (32 random bytes, hex, file mode 0600) on first use.
pub fn load_or_create_token(home: &Path) -> anyhow::Result<String> {
    let path = token_path(home);
    if let Ok(t) = std::fs::read_to_string(&path) {
        let t = t.trim().to_string();
        if t.len() >= 32 {
            return Ok(t);
        }
    }
    let dir = path.parent().expect("a parent");
    std::fs::create_dir_all(dir)?;
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("random bytes: {e}"))?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let tmp = dir.join(".token.tmp");
    {
        use std::io::Write;
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(&tmp)?;
        f.write_all(token.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(token)
}

/// Binds `addr`, refusing anything that isn't loopback.
pub async fn bind(addr: SocketAddr) -> anyhow::Result<tokio::net::TcpListener> {
    anyhow::ensure!(addr.ip().is_loopback(), "bragi serve only listens on loopback (127.0.0.1 or ::1), not {}", addr.ip());
    Ok(tokio::net::TcpListener::bind(addr).await?)
}

/// Serves the API on `listener` until `shutdown` resolves.
pub async fn serve(
    core: Core,
    token: String,
    listener: tokio::net::TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let port = listener.local_addr()?.port();
    axum::serve(listener, router(core, token, port)).with_graceful_shutdown(shutdown).await?;
    Ok(())
}

#[derive(Clone)]
struct App {
    core: Core,
    token: Arc<str>,
    port: u16,
}

/// The API's routes, guarded. `port` is the one the listener has (for the `Host` check).
pub fn router(core: Core, token: String, port: u16) -> Router {
    let app = App { core, token: token.into(), port };
    let guarded = Router::new()
        .route("/v1/state", get(state))
        .route("/v1/targets", get(targets))
        .route("/v1/models", get(models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/code/suggest", post(suggest))
        .route("/v1/code/rewrite", post(rewrite))
        .route("/v1/code/chat", post(code_chat))
        .route("/v1/cancel", post(cancel))
        .route_layer(middleware::from_fn_with_state(app.clone(), auth));
    Router::new()
        .route("/v1/health", get(|| async { Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") })) }))
        .merge(guarded)
        .layer(middleware::from_fn_with_state(app.clone(), local_only))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(app)
}

fn error(status: StatusCode, kind: &str, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": { "kind": kind, "message": message.into() } }))).into_response()
}

fn core_error(e: CoreError) -> Response {
    let status = match e.kind {
        ErrorKind::NotPaired | ErrorKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        ErrorKind::Busy => StatusCode::TOO_MANY_REQUESTS,
        ErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        ErrorKind::NotFound => StatusCode::NOT_FOUND,
        ErrorKind::Refused => StatusCode::FORBIDDEN,
        ErrorKind::Cancelled => StatusCode::REQUEST_TIMEOUT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let kind = serde_json::to_value(e.kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "other".into());
    error(status, &kind, e.message)
}

/// Loopback `Host` only, and no browser `Origin`.
async fn local_only(State(app): State<App>, req: Request, next: Next) -> Response {
    if req.headers().contains_key(header::ORIGIN) {
        return error(StatusCode::FORBIDDEN, "refused", "requests from web pages are refused");
    }
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    if !host_is_local(host, app.port) {
        return error(StatusCode::FORBIDDEN, "refused", "bragi serve answers only on 127.0.0.1, ::1 or localhost");
    }
    next.run(req).await
}

fn host_is_local(host: &str, port: u16) -> bool {
    let (name, p) = match host.rsplit_once(':') {
        Some((n, p)) if !n.ends_with(':') && (n.starts_with('[') == n.ends_with(']')) => (n, p.parse::<u16>().ok()),
        _ => (host, None),
    };
    let name_ok = matches!(name.to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost" | "[::1]");
    name_ok && p.is_none_or(|p| p == port || port == 0)
}

fn token_matches(given: &[u8], want: &[u8]) -> bool {
    // Constant time in the token's content.
    given.len() == want.len() && given.iter().zip(want).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

async fn auth(State(app): State<App>, req: Request, next: Next) -> Response {
    let given = req.headers().get(header::AUTHORIZATION).and_then(|h| h.to_str().ok()).and_then(|h| h.strip_prefix("Bearer ")).unwrap_or("");
    if !token_matches(given.trim().as_bytes(), app.token.as_bytes()) {
        let mut r = error(StatusCode::UNAUTHORIZED, "unauthorized", "a valid Bragi token is needed (bragi serve token)");
        r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        return r;
    }
    next.run(req).await
}

// ---- state -------------------------------------------------------------------------------------

async fn state(State(app): State<App>) -> Json<Value> {
    let s = app.core.state();
    let linked = matches!(s.pair, PairState::Linked { .. });
    let default = s.catalog.default_model().map(str::to_string);
    let message = if default.is_none() { Some(app.core.no_node_message()) } else { None };
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "online": s.online,
        "linked": linked,
        "pair": s.pair,
        "models": s.catalog.models,
        "nodes": s.catalog.nodes.len(),
        "default_model": default,
        "message": message,
    }))
}

async fn targets(State(app): State<App>) -> Json<Value> {
    let s = app.core.state();
    Json(
        json!({ "targets": [{ "id": "weirdnet", "label": "Weirdnet pool", "kind": "weirdnet", "models": s.catalog.models.iter().map(|m| &m.id).collect::<Vec<_>>() }] }),
    )
}

async fn models(State(app): State<App>) -> Json<Value> {
    let data: Vec<Value> = app
        .core
        .state()
        .catalog
        .models
        .iter()
        .map(|m| json!({ "id": m.id, "object": "model", "owned_by": "weirdnet", "status": m.status }))
        .collect();
    Json(json!({ "object": "list", "data": data }))
}

// ---- answers -----------------------------------------------------------------------------------

/// Cancels the request when dropped before it finished (the client went away).
struct CancelOnDrop {
    core: Core,
    request_id: String,
    done: bool,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if !self.done {
            self.core.cancel(&self.request_id);
        }
    }
}

struct Finished {
    text: String,
    model: Option<String>,
    node: Option<String>,
    finish_reason: Option<String>,
    usage: Option<(u64, u64)>,
}

/// Waits for a whole answer; cancels it when the caller is dropped or it takes too long.
async fn collect(core: &Core, mut h: ChatHandle) -> Result<Finished, CoreError> {
    let mut guard = CancelOnDrop { core: core.clone(), request_id: h.request_id.clone(), done: false };
    let mut f = Finished { text: String::new(), model: None, node: None, finish_reason: None, usage: None };
    let deadline = tokio::time::Instant::now() + ANSWER_TIMEOUT;
    loop {
        let ev = match tokio::time::timeout_at(deadline, h.events.recv()).await {
            Ok(Some(ev)) => ev,
            Ok(None) => {
                guard.done = true;
                return Err(CoreError::new(ErrorKind::Unavailable, "the answer stopped without finishing"));
            }
            Err(_) => return Err(CoreError::new(ErrorKind::Unavailable, "the answer took too long")),
        };
        match ev {
            ChatEvent::Started { node, model, .. } => {
                f.node = Some(node);
                f.model = Some(model);
            }
            ChatEvent::Delta { offset, text } => {
                let keep: String = f.text.chars().take(offset as usize).collect();
                f.text = keep + &text;
            }
            ChatEvent::Usage { prompt_tokens, completion_tokens } => f.usage = Some((prompt_tokens, completion_tokens)),
            ChatEvent::Done { finish_reason } => {
                guard.done = true;
                f.finish_reason = finish_reason;
                return Ok(f);
            }
            ChatEvent::Error { kind, message, .. } => {
                guard.done = true;
                return Err(CoreError::new(kind, message));
            }
            _ => {}
        }
    }
}

#[derive(Deserialize)]
struct SuggestBody {
    context: EditorContext,
    #[serde(default)]
    model: Option<String>,
}

async fn suggest(State(app): State<App>, Json(b): Json<SuggestBody>) -> Response {
    let h = match app.core.complete(code::suggest_prompt(&b.context), b.model) {
        Ok(h) => h,
        Err(e) => return core_error(e),
    };
    match collect(&app.core, h).await {
        Ok(f) => Json(json!({ "text": code::clean_suggestion(&f.text, &b.context), "model": f.model, "node": f.node })).into_response(),
        Err(e) => core_error(e),
    }
}

#[derive(Deserialize)]
struct RewriteBody {
    context: EditorContext,
    instruction: String,
    #[serde(default)]
    scope: Scope,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Serialize)]
struct RewriteOut {
    /// What replaces the target (the selection, or the file).
    code: String,
    /// `selection` or `file`: the scope it really had.
    scope: Scope,
    /// The whole file after the rewrite.
    file: String,
    /// A unified diff of the file.
    diff: String,
    stats: code::DiffStats,
    model: Option<String>,
    node: Option<String>,
}

async fn rewrite(State(app): State<App>, Json(b): Json<RewriteBody>) -> Response {
    let prompt = match code::rewrite_prompt(&b.context, &b.instruction, b.scope) {
        Ok(p) => p,
        Err(e) => return core_error(e),
    };
    let h = match app.core.complete(prompt, b.model) {
        Ok(h) => h,
        Err(e) => return core_error(e),
    };
    let f = match collect(&app.core, h).await {
        Ok(f) => f,
        Err(e) => return core_error(e),
    };
    let ctx = &b.context;
    let scope = ctx.effective_scope(b.scope);
    let code = code::rewrite_result(&f.text, &ctx.target(scope));
    let (old, file) = (ctx.file(), ctx.apply(scope, &code));
    let path = if ctx.path.is_empty() { "untitled" } else { &ctx.path };
    let stats = code::diff_stats(&code::diff_lines(&old, &file));
    Json(RewriteOut { diff: code::unified_diff(&old, &file, path, 3), code, scope, file, stats, model: f.model, node: f.node }).into_response()
}

#[derive(Deserialize)]
struct CodeChatBody {
    prompt: String,
    #[serde(default)]
    context: Option<EditorContext>,
    /// Continue this conversation; a new one when absent.
    #[serde(default)]
    conversation: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

fn sse_response(events: impl futures::Stream<Item = String> + Send + 'static) -> Response {
    let body = Body::from_stream(events.map(|data| Ok::<_, std::convert::Infallible>(format!("data: {data}\n\n"))));
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    (headers, body).into_response()
}

/// The handle's events as a stream that cancels the request if the client goes away first.
fn guarded_events(core: Core, h: ChatHandle) -> impl futures::Stream<Item = ChatEvent> + Send + 'static {
    let guard = CancelOnDrop { core, request_id: h.request_id.clone(), done: false };
    futures::stream::unfold((h.events, guard), |(mut rx, mut guard)| async move {
        if guard.done {
            return None;
        }
        let ev = rx.recv().await;
        match ev {
            Some(ev) => {
                if matches!(ev, ChatEvent::Done { .. } | ChatEvent::Error { .. }) {
                    guard.done = true;
                }
                Some((ev, (rx, guard)))
            }
            None => {
                // The core already finished the request; nothing to cancel.
                guard.done = true;
                drop(guard);
                None
            }
        }
    })
}

async fn code_chat(State(app): State<App>, Json(b): Json<CodeChatBody>) -> Response {
    let attachments = b.context.as_ref().map(code::chat_attachments).unwrap_or_default();
    let h = match app.core.send_with(b.conversation.as_deref(), &b.prompt, b.model, attachments) {
        Ok(h) => h,
        Err(e) => return core_error(e),
    };
    let first = json!({ "type": "conversation", "id": h.conversation_id, "request_id": h.request_id }).to_string();
    let rest = guarded_events(app.core.clone(), h).map(|ev| serde_json::to_string(&ev).unwrap_or_default());
    sse_response(futures::stream::once(async move { first }).chain(rest))
}

#[derive(Deserialize)]
struct CancelBody {
    request_id: String,
}

async fn cancel(State(app): State<App>, Json(b): Json<CancelBody>) -> Json<Value> {
    Json(json!({ "cancelled": app.core.cancel(&b.request_id) }))
}

// ---- OpenAI-compatible -------------------------------------------------------------------------

#[derive(Deserialize)]
struct OaiMessage {
    role: String,
    #[serde(default)]
    content: Value,
}

#[derive(Deserialize)]
struct OaiRequest {
    #[serde(default)]
    model: Option<String>,
    messages: Vec<OaiMessage>,
    #[serde(default)]
    stream: bool,
}

/// Text of an OpenAI message's content: a string, or the text parts of a list.
fn oai_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

async fn chat_completions(State(app): State<App>, Json(b): Json<OaiRequest>) -> Response {
    let mut messages = Vec::new();
    for m in &b.messages {
        let role = match m.role.as_str() {
            "system" | "developer" => PromptRole::System,
            "user" => PromptRole::User,
            "assistant" => PromptRole::Assistant,
            other => return error(StatusCode::BAD_REQUEST, "bad_request", format!("unsupported role {other}")),
        };
        messages.push(PromptMessage { role, content: oai_text(&m.content) });
    }
    let model_name = b.model.clone().filter(|m| !m.is_empty() && m != "default");
    let h = match app.core.complete(messages, model_name) {
        Ok(h) => h,
        Err(e) => return core_error(e),
    };
    let id = format!("chatcmpl-{}", h.request_id);
    let created = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if !b.stream {
        return match collect(&app.core, h).await {
            Ok(f) => {
                let usage = f.usage.map(|(p, c)| json!({ "prompt_tokens": p, "completion_tokens": c, "total_tokens": p + c }));
                Json(json!({
                    "id": id, "object": "chat.completion", "created": created, "model": f.model.unwrap_or_default(),
                    "choices": [{ "index": 0, "message": { "role": "assistant", "content": f.text }, "finish_reason": f.finish_reason.unwrap_or_else(|| "stop".into()) }],
                    "usage": usage,
                }))
                .into_response()
            }
            Err(e) => core_error(e),
        };
    }
    let model = Arc::new(std::sync::Mutex::new(b.model.unwrap_or_default()));
    let chunks = guarded_events(app.core.clone(), h).filter_map(move |ev| {
        let (id, model) = (id.clone(), model.clone());
        async move {
            let chunk = |delta: Value, finish: Value| {
                let m = model.lock().map(|m| m.clone()).unwrap_or_default();
                json!({ "id": id, "object": "chat.completion.chunk", "created": created, "model": m,
                        "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }] })
                .to_string()
            };
            match ev {
                ChatEvent::Started { model: m, .. } => {
                    if let Ok(mut g) = model.lock() {
                        *g = m;
                    }
                    Some(chunk(json!({ "role": "assistant" }), Value::Null))
                }
                ChatEvent::Delta { text, .. } => Some(chunk(json!({ "content": text }), Value::Null)),
                ChatEvent::Reasoning { text, .. } => Some(chunk(json!({ "reasoning_content": text }), Value::Null)),
                ChatEvent::Done { finish_reason } => {
                    Some(format!("{}\n\ndata: [DONE]", chunk(json!({}), json!(finish_reason.unwrap_or_else(|| "stop".into())))))
                }
                ChatEvent::Error { kind, message, .. } => {
                    Some(format!("{}\n\ndata: [DONE]", json!({ "error": { "kind": kind, "message": message } })))
                }
                _ => None,
            }
        }
    });
    sse_response(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_hosts_pass() {
        assert!(host_is_local("127.0.0.1:4877", 4877));
        assert!(host_is_local("localhost:4877", 4877));
        assert!(host_is_local("[::1]:4877", 4877));
        assert!(host_is_local("localhost", 4877));
        assert!(!host_is_local("evil.example:4877", 4877));
        assert!(!host_is_local("127.0.0.1.evil.example", 4877));
        assert!(!host_is_local("127.0.0.1:9999", 4877));
        assert!(!host_is_local("", 4877));
    }

    #[test]
    fn tokens_compare_whole() {
        assert!(token_matches(b"abc", b"abc"));
        assert!(!token_matches(b"abd", b"abc"));
        assert!(!token_matches(b"ab", b"abc"));
        assert!(!token_matches(b"", b"abc"));
    }

    #[test]
    fn the_token_is_made_once_and_private() {
        let d = tempfile::tempdir().unwrap();
        let t = load_or_create_token(d.path()).unwrap();
        assert_eq!(t.len(), 64);
        assert_eq!(load_or_create_token(d.path()).unwrap(), t);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(token_path(d.path())).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[tokio::test]
    async fn binding_anything_but_loopback_is_refused() {
        assert!(bind("0.0.0.0:0".parse().unwrap()).await.is_err());
        assert!(bind("127.0.0.1:0".parse().unwrap()).await.is_ok());
    }
}
