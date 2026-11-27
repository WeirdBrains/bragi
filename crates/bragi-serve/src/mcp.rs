//! Outside MCP servers and agent approvals for editors (LF-40, on LF-37's `mcp_servers`): the
//! same list, secrets and "always allow" the apps keep, and the answer to an agent's approval
//! card. Secrets arrive once in a request body, go to Bragi's secret store and never come back:
//! responses are [`McpServerView`]s (only which names have a value).

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bragi_core::mcp_servers::slug;
use bragi_core::{ChatEvent, Choice, Core, CoreError, ErrorKind, McpScope, McpSecrets, McpServerInput, McpServerView, McpTransport};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{App, core_error, error};

pub(crate) fn routes() -> Router<App> {
    Router::new()
        .route("/v1/mcp-servers", get(list).post(add))
        .route("/v1/mcp-servers/{id}", axum::routing::put(edit).delete(remove))
        .route("/v1/mcp-servers/{id}/enabled", post(enabled))
        .route("/v1/mcp-servers/{id}/test", post(test))
        .route("/v1/mcp-servers/{id}/always-allow", post(always_allow))
        .route("/v1/agent/approve", post(approve))
}

/// Where a server's tools are offered: `"all"`, `"folder"` (with `folder`), or the core's
/// `{kind, value}`.
#[derive(Deserialize)]
#[serde(untagged)]
enum ScopeIn {
    Word(String),
    Full(McpScope),
}

#[derive(Deserialize, Default)]
struct SecretsIn {
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    bearer: Option<String>,
}

#[derive(Deserialize)]
struct ServerBody {
    name: String,
    transport: McpTransport,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    scope: Option<ScopeIn>,
    /// The folder for `scope: "folder"`.
    #[serde(default)]
    folder: Option<String>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    /// Typed once; kept by Bragi's secret store. On an edit, absent keeps what is stored.
    #[serde(default)]
    secrets: Option<SecretsIn>,
}

fn yes() -> bool {
    true
}

fn bad(m: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::BadRequest, m)
}

fn nonempty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

impl ServerBody {
    fn scope(&self) -> Result<McpScope, CoreError> {
        match &self.scope {
            None => Ok(McpScope::All),
            Some(ScopeIn::Full(s)) => Ok(s.clone()),
            Some(ScopeIn::Word(w)) => match w.as_str() {
                "all" => Ok(McpScope::All),
                "folder" => nonempty(self.folder.clone()).map(McpScope::Folder).ok_or_else(|| bad("a folder scope needs `folder`")),
                other => Err(bad(format!("no scope `{other}` (all or folder)"))),
            },
        }
    }

    fn input(&self) -> Result<McpServerInput, CoreError> {
        Ok(McpServerInput {
            name: self.name.trim().to_string(),
            transport: self.transport,
            enabled: self.enabled,
            command: nonempty(self.command.clone()),
            args: self.args.clone(),
            cwd: nonempty(self.cwd.clone()),
            url: nonempty(self.url.clone()),
            scope: self.scope()?,
            timeout_secs: self.timeout_secs,
        })
    }

    fn secrets(&mut self) -> Option<McpSecrets> {
        self.secrets.take().map(|s| McpSecrets { env: s.env, headers: s.headers, bearer: nonempty(s.bearer) })
    }
}

fn view(core: &Core, id: &str) -> Response {
    match core.mcp_servers().into_iter().find(|s| s.id == id) {
        Some(v) => Json(v).into_response(),
        None => error(StatusCode::NOT_FOUND, "not_found", "no MCP server with that id"),
    }
}

async fn list(State(app): State<App>) -> Json<Value> {
    Json(json!({ "servers": app.core.mcp_servers() }))
}

async fn add(State(app): State<App>, Json(mut b): Json<ServerBody>) -> Response {
    let input = match b.input() {
        Ok(i) => i,
        Err(e) => return core_error(e),
    };
    let secrets = b.secrets().unwrap_or_default();
    match app.core.add_mcp_server(input, secrets) {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => core_error(e),
    }
}

async fn edit(State(app): State<App>, Path(id): Path<String>, Json(mut b): Json<ServerBody>) -> Response {
    let input = match b.input() {
        Ok(i) => i,
        Err(e) => return core_error(e),
    };
    let secrets = b.secrets();
    match app.core.edit_mcp_server(&id, input, secrets) {
        Ok(v) => Json(v).into_response(),
        Err(e) => core_error(e),
    }
}

async fn remove(State(app): State<App>, Path(id): Path<String>) -> Response {
    match app.core.delete_mcp_server(&id) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => core_error(e),
    }
}

#[derive(Deserialize)]
struct EnabledBody {
    enabled: bool,
}

async fn enabled(State(app): State<App>, Path(id): Path<String>, Json(b): Json<EnabledBody>) -> Response {
    match app.core.set_mcp_server_enabled(&id, b.enabled) {
        Ok(()) => view(&app.core, &id),
        Err(e) => core_error(e),
    }
}

/// Connects now and lists the tools (a local command is started). Never an error status for a
/// server that won't connect: the report says why.
async fn test(State(app): State<App>, Path(id): Path<String>) -> Response {
    if !app.core.mcp_servers().iter().any(|s| s.id == id) {
        return error(StatusCode::NOT_FOUND, "not_found", "no MCP server with that id");
    }
    Json(app.core.test_mcp_server(&id).await).into_response()
}

#[derive(Deserialize)]
struct AlwaysBody {
    /// As offered, `<slug>.<tool>`.
    tool: String,
    allow: bool,
}

/// Gives or takes back "always allow" for one tool of a server.
async fn always_allow(State(app): State<App>, Path(id): Path<String>, Json(b): Json<AlwaysBody>) -> Response {
    match app.core.set_mcp_tool_always_allowed(&id, b.tool.trim(), b.allow) {
        Ok(()) => view(&app.core, &id),
        Err(e) => core_error(e),
    }
}

#[derive(Deserialize)]
struct ApproveBody {
    request_id: String,
    ask: u64,
    /// `deny`, `once`, `this_task` or `always`.
    choice: Choice,
}

/// Answers an agent's approval card (an `approval` event of `/v1/code/chat`).
async fn approve(State(app): State<App>, Json(b): Json<ApproveBody>) -> Response {
    if app.core.approve(&b.request_id, b.ask, b.choice) {
        Json(json!({ "ok": true })).into_response()
    } else {
        error(StatusCode::NOT_FOUND, "not_found", "that card isn't waiting any more (answered, or the task ended)")
    }
}

/// Whether "always allow" means anything for `tool`: it belongs to one of the outside MCP
/// servers (a node's own tools ask each task).
pub(crate) fn always_offered(servers: &[McpServerView], tool: &str) -> bool {
    servers.iter().any(|s| tool.starts_with(&format!("{}.", slug(&s.name))))
}

/// A chat event as the stream sends it; an approval card also says whether "always allow" is
/// offered for its tool.
pub(crate) fn event_json(core: &Core, ev: &ChatEvent) -> String {
    let mut v = serde_json::to_value(ev).unwrap_or_default();
    if let ChatEvent::Approval { tool, .. } = ev {
        v["always"] = json!(always_offered(&core.mcp_servers(), tool));
    }
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(v: Value) -> ServerBody {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn scopes_read_as_words_or_the_cores_shape() {
        assert_eq!(body(json!({"name": "a", "transport": "stdio"})).scope().unwrap(), McpScope::All);
        assert_eq!(
            body(json!({"name": "a", "transport": "stdio", "scope": "folder", "folder": "/p"})).scope().unwrap(),
            McpScope::Folder("/p".into())
        );
        assert!(body(json!({"name": "a", "transport": "stdio", "scope": "folder"})).scope().is_err());
        assert!(body(json!({"name": "a", "transport": "stdio", "scope": "galaxy"})).scope().is_err());
        assert_eq!(
            body(json!({"name": "a", "transport": "http", "scope": {"kind": "session", "value": "c1"}})).scope().unwrap(),
            McpScope::Session("c1".into())
        );
    }
}
