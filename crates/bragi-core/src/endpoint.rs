//! OpenAI-compatible endpoints: a Ymir bridge (`https://ymir.example.ts.net:9443/v1`) or another
//! OpenAI-compatible API, each with its bearer token. A session can send to one
//! ([`crate::session::Target::Endpoint`]), and a Ymir bridge can be the Weirdnet session's
//! fallback for when no inference node answers.
//!
//! Settings live in `<dir>/endpoints.json` (names, URLs, which one is the fallback). Tokens never
//! go there: each is in `<dir>/secrets/<id>.token`, a 0600 file in a 0700 folder (Bragi has no OS
//! keychain integration), and is never logged or put into [`EndpointState`].
//!
//! TLS uses rustls with the system trust store (a custom CA such as Caddy's internal one, once
//! installed system-wide, or Tailscale's certificates) plus the webpki roots. Plain `http://` is
//! accepted only for loopback addresses.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use weirdnet_agent::CancelToken;
use weirdnet_agent::pool::PoolEvent;
use weirdnet_agent::remote::infer::v2::Usage;

use crate::catalog::{ModelInfo, ModelStatus};
use crate::error::{CoreError, ErrorKind, Result};
use crate::history::{ChatMessage, Role};

/// The settings file in the data folder.
pub const ENDPOINTS_FILE: &str = "endpoints.json";
/// The folder with one token file per endpoint.
pub const SECRETS_DIR: &str = "secrets";

/// What answered `/v1/models`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointKind {
    /// A Ymir bridge: every model is `owned_by: "weirdnet"` with a `weirdnet` object (state,
    /// purpose). Only these can be the Weirdnet session's fallback.
    YmirBridge,
    /// Any other OpenAI-compatible API.
    OpenAi,
}

/// One configured endpoint, as stored (no token).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Endpoint {
    pub id: String,
    pub name: String,
    /// The API base, ending in `/v1` (no trailing slash).
    pub url: String,
    pub kind: EndpointKind,
    /// The model it asks for by default (a session's own pick wins); its first when unset or no
    /// longer listed.
    #[serde(default)]
    pub model: Option<String>,
}

impl Endpoint {
    /// The model to ask when the session has no pick: the chosen default while `models` lists it
    /// (or nothing is known yet), else the first listed.
    pub fn pick_model(&self, models: &[ModelInfo]) -> Option<String> {
        match &self.model {
            Some(m) if models.is_empty() || models.iter().any(|x| &x.id == m) => Some(m.clone()),
            _ => models.first().map(|m| m.id.clone()),
        }
    }
}

/// What to do with the sessions that send to an endpoint being deleted. Their conversations are
/// never touched either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rebind {
    /// Leave them pointing at the deleted endpoint: they say so and send nowhere until the user
    /// picks another target for them.
    Unbound,
    /// Send them to this target from now on (their model resets to its default).
    To(crate::session::Target),
}

/// What a test connection found, without saving anything.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EndpointTest {
    /// The normalized URL that was checked.
    pub url: String,
    pub kind: EndpointKind,
    /// Its chat model ids, warm first.
    pub models: Vec<String>,
}

/// Whether an endpoint answered its last check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointHealth {
    #[default]
    Unchecked,
    Ok,
    Failed,
}

/// An endpoint as the shells draw it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EndpointState {
    pub id: String,
    pub name: String,
    pub url: String,
    pub kind: EndpointKind,
    pub health: EndpointHealth,
    /// Why the last check failed.
    pub message: Option<String>,
    /// Its chat models (a Ymir bridge's state per model; other APIs' models count as warm).
    pub models: Vec<ModelInfo>,
    /// The Weirdnet session falls back to it.
    pub fallback: bool,
    /// The default model chosen for it (`None`: its first).
    #[serde(default)]
    pub model: Option<String>,
    /// How many sessions send to it.
    #[serde(default)]
    pub sessions: usize,
}

impl EndpointState {
    /// The model it asks for when a session has no pick.
    pub fn default_model(&self) -> Option<&str> {
        match &self.model {
            Some(m) if self.models.is_empty() || self.models.iter().any(|x| &x.id == m) => Some(m.as_str()),
            _ => self.models.first().map(|m| m.id.as_str()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    endpoints: Vec<Endpoint>,
    /// The endpoint the Weirdnet session falls back to (a Ymir bridge).
    #[serde(default)]
    fallback: Option<String>,
}

/// The endpoints and their tokens in a data folder.
pub(crate) struct EndpointStore {
    dir: PathBuf,
    stored: Stored,
}

fn bad(msg: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::BadRequest, msg)
}

/// Writes `bytes` to `path` through a temp file and a rename; `private` makes it 0600.
pub(crate) fn write_file(path: &Path, bytes: &[u8], private: bool) -> Result<()> {
    use std::io::Write;
    let dir = path.parent().ok_or_else(|| CoreError::new(ErrorKind::Storage, "no parent folder"))?;
    std::fs::create_dir_all(dir)?;
    let mut rnd = [0u8; 6];
    let _ = getrandom::fill(&mut rnd);
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        rnd.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut f = o.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

fn private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A clean name: trimmed, one line, at most 80 characters, not empty.
pub(crate) fn clean_name(name: &str, what: &str) -> Result<String> {
    let n: String = name.lines().next().unwrap_or("").trim().chars().take(80).collect();
    if n.is_empty() { Err(bad(format!("give the {what} a name"))) } else { Ok(n) }
}

/// The API base for what the user typed: `https://host[:port][/path]` with `/v1` at the end
/// (added when missing; a pasted `/v1/models` or `/v1/chat/completions` is cut back to `/v1`).
/// `http://` only for loopback hosts, so a token never crosses a network in the clear.
pub fn normalize_url(url: &str) -> Result<String> {
    let mut u = url.trim().trim_end_matches('/').to_string();
    for tail in ["/chat/completions", "/completions", "/models"] {
        if let Some(s) = u.strip_suffix(tail) {
            u = s.to_string();
        }
    }
    let parsed = reqwest::Url::parse(&u).map_err(|e| bad(format!("`{}` is not a URL: {e}", url.trim())))?;
    let host = parsed.host_str().unwrap_or("").to_string();
    if host.is_empty() || parsed.query().is_some() || parsed.fragment().is_some() || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(bad("use a plain URL like https://ymir.example.ts.net:9443/v1 (no user, query or fragment)"));
    }
    match parsed.scheme() {
        "https" => {}
        "http" if is_loopback(&host) => {}
        "http" => return Err(bad("use https:// for a remote endpoint; plain http:// is only for 127.0.0.1 or localhost")),
        s => return Err(bad(format!("`{s}://` is not an HTTP URL"))),
    }
    if !u.ends_with("/v1") {
        u.push_str("/v1");
    }
    Ok(u)
}

fn is_loopback(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    h == "localhost" || h.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

impl EndpointStore {
    pub(crate) fn open(dir: &Path) -> Self {
        let stored = std::fs::read(dir.join(ENDPOINTS_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { dir: dir.to_path_buf(), stored }
    }

    fn save(&self) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.stored).map_err(|e| CoreError::new(ErrorKind::Storage, e.to_string()))?;
        write_file(&self.dir.join(ENDPOINTS_FILE), &bytes, true)
    }

    fn token_path(&self, id: &str) -> PathBuf {
        self.dir.join(SECRETS_DIR).join(format!("{id}.token"))
    }

    pub(crate) fn list(&self) -> &[Endpoint] {
        &self.stored.endpoints
    }

    pub(crate) fn get(&self, id: &str) -> Option<&Endpoint> {
        self.stored.endpoints.iter().find(|e| e.id == id)
    }

    pub(crate) fn fallback(&self) -> Option<&Endpoint> {
        self.stored.fallback.as_deref().and_then(|id| self.get(id))
    }

    pub(crate) fn token(&self, id: &str) -> Result<String> {
        if !valid_id(id) {
            return Err(bad("no such endpoint"));
        }
        let t = std::fs::read_to_string(self.token_path(id))
            .map_err(|_| CoreError::new(ErrorKind::Storage, "this endpoint's token is missing; set it again"))?;
        Ok(t.trim().to_string())
    }

    fn write_token(&self, id: &str, token: &str) -> Result<()> {
        private_dir(&self.dir.join(SECRETS_DIR))?;
        write_file(&self.token_path(id), token.as_bytes(), true)
    }

    pub(crate) fn add(&mut self, ep: Endpoint, token: &str) -> Result<()> {
        if self.stored.endpoints.iter().any(|e| e.name.eq_ignore_ascii_case(&ep.name)) {
            return Err(bad(format!("an endpoint is already called `{}`", ep.name)));
        }
        self.write_token(&ep.id, token)?;
        self.stored.endpoints.push(ep);
        self.save()
    }

    pub(crate) fn update(
        &mut self,
        id: &str,
        name: Option<String>,
        url: Option<String>,
        kind: Option<EndpointKind>,
        token: Option<&str>,
    ) -> Result<()> {
        if let Some(n) = &name
            && self.stored.endpoints.iter().any(|e| e.id != id && e.name.eq_ignore_ascii_case(n))
        {
            return Err(bad(format!("an endpoint is already called `{n}`")));
        }
        let e = self.stored.endpoints.iter_mut().find(|e| e.id == id).ok_or_else(|| bad("no such endpoint"))?;
        if let Some(n) = name {
            e.name = n;
        }
        if let Some(u) = url {
            e.url = u;
        }
        if let Some(k) = kind {
            e.kind = k;
        }
        if self.stored.fallback.as_deref() == Some(id) && self.get(id).is_some_and(|e| e.kind != EndpointKind::YmirBridge) {
            self.stored.fallback = None;
        }
        if let Some(t) = token {
            self.write_token(id, t)?;
        }
        self.save()
    }

    /// The default model (`None`: its first listed).
    pub(crate) fn set_model(&mut self, id: &str, model: Option<String>) -> Result<()> {
        let e = self.stored.endpoints.iter_mut().find(|e| e.id == id).ok_or_else(|| bad("no such endpoint"))?;
        e.model = model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
        self.save()
    }

    pub(crate) fn remove(&mut self, id: &str) -> Result<()> {
        let before = self.stored.endpoints.len();
        self.stored.endpoints.retain(|e| e.id != id);
        if self.stored.endpoints.len() == before {
            return Err(bad("no such endpoint"));
        }
        if self.stored.fallback.as_deref() == Some(id) {
            self.stored.fallback = None;
        }
        self.save()?;
        let _ = std::fs::remove_file(self.token_path(id));
        Ok(())
    }

    pub(crate) fn set_fallback(&mut self, id: Option<&str>) -> Result<()> {
        if let Some(id) = id {
            let e = self.get(id).ok_or_else(|| bad("no such endpoint"))?;
            if e.kind != EndpointKind::YmirBridge {
                return Err(bad(format!(
                    "`{}` is not a Ymir bridge (its /v1/models has no weirdnet fields), so it can't be the Weirdnet fallback",
                    e.name
                )));
            }
        }
        self.stored.fallback = id.map(str::to_string);
        self.save()
    }
}

// ---- HTTP --------------------------------------------------------------------------------------

/// The shared HTTP client: rustls, the system trust store and the webpki roots, no cookies.
pub(crate) fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .use_rustls_tls()
        .connect_timeout(Duration::from_secs(10))
        .user_agent(concat!("bragi/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| CoreError::new(ErrorKind::Other, format!("starting the HTTP client: {e}")))
}

/// What `/v1/models` said.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    pub kind: EndpointKind,
    pub models: Vec<ModelInfo>,
}

fn http_error(status: reqwest::StatusCode, body: &str) -> CoreError {
    let msg = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| body.chars().take(200).collect::<String>().trim().to_string());
    let msg = if msg.is_empty() { status.to_string() } else { format!("{status}: {msg}") };
    let kind = match status.as_u16() {
        401 | 403 => return CoreError::new(ErrorKind::Refused, format!("the endpoint refused the token ({msg})")),
        429 => ErrorKind::Busy,
        400 | 404 | 422 => ErrorKind::BadRequest,
        _ => ErrorKind::Unavailable,
    };
    CoreError::new(kind, format!("the endpoint answered {msg}"))
}

fn send_error(e: reqwest::Error) -> CoreError {
    // Without the URL: it is the user's own and goes to the UI, but logs stay plain.
    let e = e.without_url();
    let mut why = e.to_string();
    let mut src: Option<&dyn std::error::Error> = std::error::Error::source(&e);
    while let Some(s) = src {
        why = format!("{why}: {s}");
        src = s.source();
    }
    CoreError::new(ErrorKind::Unavailable, format!("the endpoint is not reachable ({why})"))
}

fn status_of(state: &str) -> ModelStatus {
    match state {
        "warm" => ModelStatus::Warm,
        "loading" => ModelStatus::Loading,
        "busy" => ModelStatus::Busy,
        _ => ModelStatus::Loadable,
    }
}

/// Reads a `/v1/models` body: a Ymir bridge when there is at least one model and every one is
/// `owned_by: "weirdnet"` with a `weirdnet` object; its chat models only, warm first.
pub fn read_models(v: &Value) -> Result<Checked> {
    let data = v["data"]
        .as_array()
        .ok_or_else(|| CoreError::new(ErrorKind::BadRequest, "this is not an OpenAI-compatible API: /v1/models has no `data` list"))?;
    let ymir = !data.is_empty() && data.iter().all(|m| m["owned_by"] == "weirdnet" && m["weirdnet"].is_object());
    let mut models: Vec<ModelInfo> = data
        .iter()
        .filter(|m| !ymir || m["weirdnet"]["purpose"].as_str().is_none_or(|p| p == "chat"))
        .filter_map(|m| {
            let id = m["id"].as_str()?.to_string();
            let w = &m["weirdnet"];
            Some(ModelInfo {
                id,
                status: if ymir { status_of(w["state"].as_str().unwrap_or("")) } else { ModelStatus::Warm },
                replicas: w["replicas"].as_u64().unwrap_or(if ymir { 0 } else { 1 }) as u32,
                tokens_per_sec: None,
                context: w["context"].as_u64().map(|c| c as u32),
                nodes: Vec::new(),
                standin: None,
            })
        })
        .collect();
    let rank = |s: ModelStatus| match s {
        ModelStatus::Warm => 0,
        ModelStatus::Busy => 1,
        ModelStatus::Loading => 2,
        ModelStatus::Loadable => 3,
    };
    models.sort_by(|a, b| rank(a.status).cmp(&rank(b.status)).then_with(|| a.id.cmp(&b.id)));
    Ok(Checked { kind: if ymir { EndpointKind::YmirBridge } else { EndpointKind::OpenAi }, models })
}

/// Asks `base`'s `/v1/models` with `token`.
pub async fn check(http: &reqwest::Client, base: &str, token: &str) -> Result<Checked> {
    let r = tokio::time::timeout(Duration::from_secs(20), http.get(format!("{base}/models")).bearer_auth(token).send())
        .await
        .map_err(|_| CoreError::new(ErrorKind::Unavailable, "the endpoint did not answer in 20 s"))?
        .map_err(send_error)?;
    let status = r.status();
    let body = r.text().await.map_err(send_error)?;
    if !status.is_success() {
        return Err(http_error(status, &body));
    }
    let v: Value = serde_json::from_str(&body)
        .map_err(|_| CoreError::new(ErrorKind::BadRequest, "this is not an OpenAI-compatible API: /v1/models is not JSON"))?;
    read_models(&v)
}

/// The conversation as OpenAI chat messages (attachments rendered into the user text, empty
/// answers left out).
pub fn openai_messages(msgs: &[ChatMessage]) -> Vec<Value> {
    msgs.iter()
        .filter(|m| !(m.role == Role::Assistant && m.text.is_empty()))
        .map(|m| match m.role {
            Role::User => json!({"role": "user", "content": crate::attach::render(&m.text, &m.attachments)}),
            Role::Assistant => json!({"role": "assistant", "content": m.text}),
        })
        .collect()
}

/// How a streamed answer ended.
#[derive(Debug)]
pub struct Ended {
    pub model: String,
    pub finish_reason: Option<String>,
    pub cache_hit: bool,
    pub standin: Option<String>,
}

/// Why it didn't: the error, and whether any text streamed first.
#[derive(Debug)]
pub struct Broke {
    pub error: CoreError,
    pub streamed: bool,
}

/// One chat request to `base`. `continuing`: the last message is a cut-off answer to carry on
/// (`continue_final_message`, which Ymir bridges and vLLM take). Streams [`PoolEvent`]s onto
/// `out` (`Accepted` with `node`, text, reasoning, usage, meta) so [`crate::chat::coalesce`]
/// turns them into the same events a Weirdnet answer gives.
#[allow(clippy::too_many_arguments)]
pub async fn stream_chat(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    node: &str,
    model: Option<&str>,
    messages: Vec<Value>,
    continuing: bool,
    out: &mpsc::UnboundedSender<PoolEvent>,
    cancel: &CancelToken,
) -> std::result::Result<Ended, Broke> {
    let fail = |error: CoreError, streamed: bool| Broke { error, streamed };
    let mut body = json!({"messages": messages, "stream": true, "stream_options": {"include_usage": true}});
    if let Some(m) = model {
        body["model"] = m.into();
    }
    if continuing {
        body["continue_final_message"] = true.into();
        body["add_generation_prompt"] = false.into();
    }
    let req = http.post(format!("{base}/chat/completions")).bearer_auth(token).json(&body).send();
    let r = tokio::select! {
        r = req => r.map_err(|e| fail(send_error(e), false))?,
        _ = cancel.cancelled() => return Err(fail(CoreError::new(ErrorKind::Cancelled, "stopped"), false)),
    };
    let status = r.status();
    if !status.is_success() {
        let body = r.text().await.unwrap_or_default();
        return Err(fail(http_error(status, &body), false));
    }
    let header = |k: &str| r.headers().get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
    let mut end = Ended {
        model: header("x-weirdnet-model").or_else(|| model.map(str::to_string)).unwrap_or_default(),
        finish_reason: None,
        cache_hit: header("x-weirdnet-cache").as_deref() == Some("hit"),
        standin: header("x-weirdnet-standin"),
    };
    let mut accepted = false;
    let mut streamed = false;
    let mut buf: Vec<u8> = Vec::new();
    let mut bytes = r.bytes_stream();
    let idle = Duration::from_secs(300);
    loop {
        let chunk = tokio::select! {
            c = tokio::time::timeout(idle, bytes.next()) => match c {
                Err(_) => return Err(fail(CoreError::new(ErrorKind::Unavailable, "the endpoint stopped sending for 5 minutes"), streamed)),
                Ok(None) => break,
                Ok(Some(Err(e))) => return Err(fail(CoreError::new(ErrorKind::Unavailable, format!("the stream broke ({})", e.without_url())), streamed)),
                Ok(Some(Ok(b))) => b,
            },
            _ = cancel.cancelled() => return Err(fail(CoreError::new(ErrorKind::Cancelled, "stopped"), streamed)),
        };
        buf.extend_from_slice(&chunk);
        while let Some(at) = find_event_end(&buf) {
            let event: Vec<u8> = buf.drain(..at.0 + at.1).collect();
            let text = String::from_utf8_lossy(&event[..at.0]).to_string();
            let data: String =
                text.lines().filter_map(|l| l.strip_prefix("data:")).map(|d| d.strip_prefix(' ').unwrap_or(d)).collect::<Vec<_>>().join("\n");
            if data.is_empty() {
                continue;
            }
            if data.trim() == "[DONE]" {
                return if end.finish_reason.is_some() || streamed {
                    Ok(end)
                } else {
                    Err(fail(CoreError::new(ErrorKind::Unavailable, "the endpoint ended the answer without any text"), false))
                };
            }
            let Ok(v) = serde_json::from_str::<Value>(&data) else {
                return Err(fail(CoreError::new(ErrorKind::Other, "the endpoint sent a chunk that is not JSON"), streamed));
            };
            if let Some(err) = v.get("error") {
                let m = err["message"].as_str().unwrap_or("stream error").to_string();
                return Err(fail(CoreError::new(ErrorKind::Unavailable, format!("the endpoint failed: {m}")), streamed));
            }
            if !accepted {
                accepted = true;
                if let Some(m) = v["model"].as_str().filter(|m| !m.is_empty())
                    && end.model.is_empty()
                {
                    end.model = m.to_string();
                }
                let _ = out.send(PoolEvent::Accepted { node: node.to_string(), model: end.model.clone(), resume: continuing });
            }
            let choice = &v["choices"][0];
            let delta = &choice["delta"];
            if let Some(t) = delta["reasoning_content"].as_str().or_else(|| delta["reasoning"].as_str()).filter(|t| !t.is_empty()) {
                let _ = out.send(PoolEvent::Reasoning(t.to_string()));
            }
            if let Some(t) = delta["content"].as_str().filter(|t| !t.is_empty()) {
                streamed = true;
                let _ = out.send(PoolEvent::Text(t.to_string()));
            }
            if let Some(f) = choice["finish_reason"].as_str() {
                end.finish_reason = Some(f.to_string());
            }
            if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
                let _ =
                    out.send(PoolEvent::Usage(Usage::new(u["prompt_tokens"].as_u64().unwrap_or(0), u["completion_tokens"].as_u64().unwrap_or(0))));
            }
            let w = &v["weirdnet"];
            if w["cache_hit"].as_bool() == Some(true) {
                end.cache_hit = true;
            }
            if let Some(s) = w["standin"].as_str() {
                end.standin = Some(s.to_string());
            }
        }
    }
    if end.finish_reason.is_some() {
        Ok(end)
    } else {
        Err(fail(CoreError::new(ErrorKind::Unavailable, "the stream ended before the answer finished"), streamed))
    }
}

/// The end of the first SSE event in `buf`: (where it ends, the separator's length).
fn find_event_end(buf: &[u8]) -> Option<(usize, usize)> {
    let lf = buf.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| (i, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_end_in_v1_and_remote_ones_need_https() {
        assert_eq!(normalize_url(" https://ymir.example.ts.net:9443 ").unwrap(), "https://ymir.example.ts.net:9443/v1");
        assert_eq!(normalize_url("https://ymir.example.ts.net:9443/v1/").unwrap(), "https://ymir.example.ts.net:9443/v1");
        assert_eq!(normalize_url("https://ymir.example.ts.net/v1/models").unwrap(), "https://ymir.example.ts.net/v1");
        assert_eq!(normalize_url("https://ymir.example.ts.net/v1/chat/completions").unwrap(), "https://ymir.example.ts.net/v1");
        assert_eq!(normalize_url("http://127.0.0.1:8484/v1").unwrap(), "http://127.0.0.1:8484/v1");
        assert_eq!(normalize_url("http://localhost:8484").unwrap(), "http://localhost:8484/v1");
        assert!(normalize_url("http://ymir.example.ts.net:8484/v1").is_err(), "a token never crosses a network in the clear");
        assert!(normalize_url("ftp://ymir.example.ts.net").is_err());
        assert!(normalize_url("https://user:pw@ymir.example.ts.net").is_err());
        assert!(normalize_url("not a url").is_err());
    }

    #[test]
    fn a_ymir_bridge_is_told_apart_from_a_generic_api() {
        let ymir = json!({"object": "list", "data": [
            {"id": "small", "object": "model", "owned_by": "weirdnet", "weirdnet": {"state": "loadable", "purpose": "chat", "replicas": 0}},
            {"id": "embed", "object": "model", "owned_by": "weirdnet", "weirdnet": {"state": "warm", "purpose": "embedding", "replicas": 1}},
            {"id": "big", "object": "model", "owned_by": "weirdnet", "weirdnet": {"state": "warm", "purpose": "chat", "replicas": 1, "context": 32768}},
        ]});
        let c = read_models(&ymir).unwrap();
        assert_eq!(c.kind, EndpointKind::YmirBridge);
        assert_eq!(c.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["big", "small"], "chat models only, warm first");
        assert_eq!(c.models[0].context, Some(32768));

        let openai = json!({"object": "list", "data": [{"id": "gpt-x", "object": "model", "owned_by": "someone"}]});
        assert_eq!(read_models(&openai).unwrap().kind, EndpointKind::OpenAi);
        // One model without the weirdnet object is enough to not be a Ymir bridge.
        let mixed = json!({"data": [{"id": "a", "owned_by": "weirdnet", "weirdnet": {"purpose": "chat"}}, {"id": "b", "owned_by": "weirdnet"}]});
        assert_eq!(read_models(&mixed).unwrap().kind, EndpointKind::OpenAi);
        assert_eq!(read_models(&json!({"data": []})).unwrap().kind, EndpointKind::OpenAi);
        assert!(read_models(&json!({"models": []})).is_err());
    }

    #[test]
    fn tokens_are_private_files_outside_the_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = EndpointStore::open(tmp.path());
        let ep = Endpoint {
            id: "e-1".into(),
            name: "Home".into(),
            url: "https://ymir.example.ts.net/v1".into(),
            kind: EndpointKind::YmirBridge,
            model: None,
        };
        s.add(ep, "sekrit-token-0123456789").unwrap();
        let settings = std::fs::read_to_string(tmp.path().join(ENDPOINTS_FILE)).unwrap();
        assert!(!settings.contains("sekrit"), "the token is not in the settings");
        assert_eq!(s.token("e-1").unwrap(), "sekrit-token-0123456789");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&tmp.path().join(SECRETS_DIR).join("e-1.token")), 0o600);
            assert_eq!(mode(&tmp.path().join(SECRETS_DIR)), 0o700);
            assert_eq!(mode(&tmp.path().join(ENDPOINTS_FILE)), 0o600);
        }
        s.set_fallback(Some("e-1")).unwrap();
        let again = EndpointStore::open(tmp.path());
        assert_eq!(again.fallback().map(|e| e.name.as_str()), Some("Home"));
        let generic =
            Endpoint { id: "e-2".into(), name: "Other".into(), url: "https://api.example.com/v1".into(), kind: EndpointKind::OpenAi, model: None };
        s.add(generic, "t").unwrap();
        assert!(s.set_fallback(Some("e-2")).is_err(), "only a Ymir bridge can be the fallback");
        assert!(
            s.add(
                Endpoint { id: "e-3".into(), name: "home".into(), url: "https://x.example/v1".into(), kind: EndpointKind::OpenAi, model: None },
                "t"
            )
            .is_err()
        );
        s.remove("e-1").unwrap();
        assert!(s.fallback().is_none());
        assert!(!tmp.path().join(SECRETS_DIR).join("e-1.token").exists(), "removing an endpoint removes its token");
    }

    #[test]
    fn sse_events_split_on_blank_lines() {
        assert_eq!(find_event_end(b"data: a\n\ndata: b"), Some((7, 2)));
        assert_eq!(find_event_end(b"data: a\r\n\r\n"), Some((7, 4)));
        assert_eq!(find_event_end(b"data: a\n"), None);
    }
}

// ---- the core's endpoint actions -----------------------------------------------------------------

impl crate::Core {
    pub(crate) fn publish_endpoints(&self) {
        let list: Vec<EndpointState> = {
            let store = crate::core::lock(&self.inner.endpoints);
            let health = crate::core::lock(&self.inner.health);
            let fb = store.fallback().map(|e| e.id.clone());
            let sessions = crate::core::lock(&self.inner.sessions).data.clone();
            let bound =
                |id: &str| sessions.sessions.iter().filter(|s| matches!(&s.target, crate::session::Target::Endpoint { id: e } if e == id)).count();
            store
                .list()
                .iter()
                .map(|e| {
                    let (h, msg, models) = health.get(&e.id).cloned().unwrap_or_default();
                    EndpointState {
                        id: e.id.clone(),
                        name: e.name.clone(),
                        url: e.url.clone(),
                        kind: e.kind,
                        health: h,
                        message: msg,
                        models,
                        fallback: fb.as_deref() == Some(e.id.as_str()),
                        model: e.model.clone(),
                        sessions: bound(&e.id),
                    }
                })
                .collect()
        };
        self.inner.state.send_if_modified(|s| {
            let changed = s.endpoints != list;
            if changed {
                s.endpoints = list;
            }
            changed
        });
    }

    pub(crate) fn set_health(&self, id: &str, health: EndpointHealth, message: Option<String>, models: Option<Vec<ModelInfo>>) {
        {
            let mut h = crate::core::lock(&self.inner.health);
            let e = h.entry(id.to_string()).or_default();
            e.0 = health;
            e.1 = message;
            if let Some(m) = models {
                e.2 = m;
            }
        }
        self.publish_endpoints();
    }

    /// Checks one endpoint's `/v1/models` now. A Ymir bridge that turns into something else
    /// (or back) is noted, and stops being the fallback if it is no longer a bridge.
    pub async fn check_endpoint(&self, id: &str) -> Result<()> {
        let (url, token) = {
            let store = crate::core::lock(&self.inner.endpoints);
            let e = store.get(id).ok_or_else(|| bad("no such endpoint"))?;
            (e.url.clone(), store.token(id))
        };
        let r = match token {
            Ok(t) => check(&self.inner.http, &url, &t).await,
            Err(e) => Err(e),
        };
        match r {
            Ok(c) => {
                let kind_changed = crate::core::lock(&self.inner.endpoints).get(id).is_some_and(|e| e.kind != c.kind);
                if kind_changed {
                    crate::core::lock(&self.inner.endpoints).update(id, None, None, Some(c.kind), None)?;
                }
                self.set_health(id, EndpointHealth::Ok, None, Some(c.models));
                Ok(())
            }
            Err(e) => {
                self.set_health(id, EndpointHealth::Failed, Some(e.message.clone()), None);
                Err(e)
            }
        }
    }

    /// Checks every endpoint (Bragi does this every 30 seconds by itself).
    pub async fn check_endpoints(&self) {
        let ids: Vec<String> = crate::core::lock(&self.inner.endpoints).list().iter().map(|e| e.id.clone()).collect();
        for id in ids {
            let _ = self.check_endpoint(&id).await;
        }
    }

    /// Adds an endpoint after checking that `url` answers `/v1/models` with `token`; returns it
    /// with what the check found (a Ymir bridge or another OpenAI-compatible API).
    pub async fn add_endpoint(&self, name: &str, url: &str, token: &str) -> Result<EndpointState> {
        let name = clean_name(name, "endpoint")?;
        let url = normalize_url(url)?;
        let token = token.trim();
        if token.is_empty() {
            return Err(bad("paste the endpoint's token"));
        }
        let c = check(&self.inner.http, &url, token).await?;
        let id = crate::core::random_id("e");
        crate::core::lock(&self.inner.endpoints).add(Endpoint { id: id.clone(), name, url, kind: c.kind, model: None }, token)?;
        self.set_health(&id, EndpointHealth::Ok, None, Some(c.models));
        self.state().endpoints.into_iter().find(|e| e.id == id).ok_or_else(|| CoreError::new(ErrorKind::Other, "the endpoint vanished"))
    }

    /// Changes an endpoint's name, URL or token (each when given), checking it again when the URL
    /// or token changed.
    pub async fn update_endpoint(&self, id: &str, name: Option<String>, url: Option<String>, token: Option<String>) -> Result<EndpointState> {
        let name = name.map(|n| clean_name(&n, "endpoint")).transpose()?;
        let url = url.map(|u| normalize_url(&u)).transpose()?;
        let token = token.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        let recheck = url.is_some() || token.is_some();
        let kind = if recheck {
            let (u, t) = {
                let store = crate::core::lock(&self.inner.endpoints);
                let e = store.get(id).ok_or_else(|| bad("no such endpoint"))?;
                (
                    url.clone().unwrap_or_else(|| e.url.clone()),
                    match &token {
                        Some(t) => t.clone(),
                        None => store.token(id)?,
                    },
                )
            };
            let c = check(&self.inner.http, &u, &t).await?;
            self.set_health(id, EndpointHealth::Ok, None, Some(c.models));
            Some(c.kind)
        } else {
            None
        };
        crate::core::lock(&self.inner.endpoints).update(id, name, url, kind, token.as_deref())?;
        self.publish_endpoints();
        self.state().endpoints.into_iter().find(|e| e.id == id).ok_or_else(|| bad("no such endpoint"))
    }

    /// A test connection: checks `url` with `token` (or, for an existing endpoint `id` and no new
    /// token, its saved one) against `/v1/models`, and says what answered. Saves nothing.
    pub async fn test_endpoint(&self, id: Option<&str>, url: &str, token: Option<&str>) -> Result<EndpointTest> {
        let url = normalize_url(url)?;
        let token = match token.map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) => t.to_string(),
            None => match id {
                Some(id) => crate::core::lock(&self.inner.endpoints).token(id)?,
                None => return Err(bad("paste the endpoint's token")),
            },
        };
        let c = check(&self.inner.http, &url, &token).await?;
        Ok(EndpointTest { url, kind: c.kind, models: c.models.into_iter().map(|m| m.id).collect() })
    }

    /// Edits an endpoint in one go, the way the settings screen saves it: name, URL, a new token
    /// (`None` keeps the saved one; it is never read back), the default model and whether it is
    /// the fallback. A changed URL or token is tested first and nothing changes if the test fails.
    pub async fn edit_endpoint(
        &self,
        id: &str,
        name: &str,
        url: &str,
        token: Option<String>,
        model: Option<String>,
        fallback: bool,
    ) -> Result<EndpointState> {
        let (old_name, old_url) = {
            let store = crate::core::lock(&self.inner.endpoints);
            let e = store.get(id).ok_or_else(|| bad("no such endpoint"))?;
            (e.name.clone(), e.url.clone())
        };
        let url_n = normalize_url(url)?;
        let name = clean_name(name, "endpoint")?;
        let st = self
            .update_endpoint(id, (name != old_name).then_some(name), (url_n != old_url).then_some(url_n), token.filter(|t| !t.trim().is_empty()))
            .await?;
        let model = model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
        if let Some(m) = &model
            && !st.models.is_empty()
            && !st.models.iter().any(|x| &x.id == m)
        {
            return Err(bad(format!("{} doesn't list a model called `{m}`", st.name)));
        }
        crate::core::lock(&self.inner.endpoints).set_model(id, model)?;
        let is_fb = st.fallback;
        if fallback != is_fb {
            self.set_fallback(if fallback { Some(id) } else { None })?;
        }
        self.publish_endpoints();
        self.state().endpoints.into_iter().find(|e| e.id == id).ok_or_else(|| bad("no such endpoint"))
    }

    /// The default model of an endpoint (`None`: its first listed).
    pub fn set_endpoint_model(&self, id: &str, model: Option<String>) -> Result<()> {
        if let Some(m) = &model
            && let Some(e) = self.state().endpoints.into_iter().find(|e| e.id == id)
            && !e.models.is_empty()
            && !e.models.iter().any(|x| &x.id == m)
        {
            return Err(bad(format!("{} doesn't list a model called `{m}`", e.name)));
        }
        crate::core::lock(&self.inner.endpoints).set_model(id, model)?;
        self.publish_endpoints();
        Ok(())
    }

    /// Deletes an endpoint and its token. The sessions that send to it are rebound to another
    /// target or left unbound, as `rebind` says; no conversation is deleted. Returns how many
    /// sessions it had.
    pub fn delete_endpoint(&self, id: &str, rebind: Rebind) -> Result<usize> {
        if crate::core::lock(&self.inner.endpoints).get(id).is_none() {
            return Err(bad("no such endpoint"));
        }
        let users: Vec<String> = self
            .sessions()
            .sessions
            .iter()
            .filter(|s| matches!(&s.target, crate::session::Target::Endpoint { id: e } if e == id))
            .map(|s| s.id.clone())
            .collect();
        if let Rebind::To(target) = &rebind {
            if matches!(target, crate::session::Target::Endpoint { id: e } if e == id) {
                return Err(bad("pick another target for its sessions"));
            }
            for s in &users {
                self.set_session_target(s, target.clone())?;
            }
        }
        crate::core::lock(&self.inner.endpoints).remove(id)?;
        crate::core::lock(&self.inner.health).remove(id);
        self.publish_endpoints();
        self.publish_sessions();
        Ok(users.len())
    }

    /// Removes an endpoint and its token. Refused while a session sends to it.
    pub fn remove_endpoint(&self, id: &str) -> Result<()> {
        let users: Vec<String> = self
            .sessions()
            .sessions
            .iter()
            .filter(|s| matches!(&s.target, crate::session::Target::Endpoint { id: e } if e == id))
            .map(|s| s.name.clone())
            .collect();
        if !users.is_empty() {
            return Err(bad(format!(
                "{} still send{} to it; delete or change {} first",
                users.join(", "),
                if users.len() == 1 { "s" } else { "" },
                if users.len() == 1 { "that session" } else { "those sessions" }
            )));
        }
        crate::core::lock(&self.inner.endpoints).remove(id)?;
        crate::core::lock(&self.inner.health).remove(id);
        self.publish_endpoints();
        Ok(())
    }

    /// The Ymir bridge the Weirdnet session falls back to when no inference node answers (`None`
    /// turns the fallback off).
    pub fn set_fallback(&self, id: Option<&str>) -> Result<()> {
        crate::core::lock(&self.inner.endpoints).set_fallback(id)?;
        self.publish_endpoints();
        Ok(())
    }
}
