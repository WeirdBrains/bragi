//! A minimal OpenAI-compatible client (LF-11): list models, test a connection and stream a chat
//! completion from any `…/v1`-style base URL, with an optional bearer key. The key is only ever
//! put in the `Authorization` header; it is never logged and never part of an error message.

use std::time::{Duration, Instant};

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use weirdnet_agent::pool::PoolEvent;
use weirdnet_agent::remote::infer::v2::Usage;
use weirdnet_agent::{CancelToken, Message};

use crate::error::{CoreError, ErrorKind, Result};

const CONNECT: Duration = Duration::from_secs(10);
const LIST_TIMEOUT: Duration = Duration::from_secs(15);

/// Where to call and with what key (already unsealed; lives only for the call).
#[derive(Clone)]
pub struct Target {
    pub base_url: String,
    pub api_key: Option<String>,
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Target").field("base_url", &self.base_url).field("api_key", &self.api_key.as_ref().map(|_| "…")).finish()
    }
}

/// The result of testing a connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestReport {
    pub ok: bool,
    pub models: Vec<String>,
    pub latency_ms: u64,
    pub message: String,
}

/// Checks and normalizes a base URL: http(s) only, no credentials in it, no trailing slash.
pub fn normalize_url(url: &str) -> Result<String> {
    let u = url.trim().trim_end_matches('/');
    let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://"));
    let Some(rest) = rest.filter(|r| !r.is_empty()) else {
        return Err(CoreError::new(ErrorKind::BadRequest, "the address must start with http:// or https://"));
    };
    let host = rest.split('/').next().unwrap_or_default();
    if host.contains('@') {
        return Err(CoreError::new(ErrorKind::BadRequest, "put the API key in the key field, not in the address"));
    }
    if host.is_empty() || u.contains(char::is_whitespace) || u.contains('?') || u.contains('#') {
        return Err(CoreError::new(ErrorKind::BadRequest, "that isn't a usable address"));
    }
    Ok(u.to_string())
}

/// True for plain http to a host that isn't this machine: traffic (and the key) are readable on the way.
pub fn is_insecure(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else { return false };
    let host = rest.split('/').next().unwrap_or_default();
    let name =
        if host.starts_with('[') { host.split(']').next().unwrap_or("").trim_start_matches('[') } else { host.split(':').next().unwrap_or("") };
    !(name == "localhost" || name == "127.0.0.1" || name == "::1")
}

fn client(timeout: Option<Duration>) -> Result<reqwest::Client> {
    let mut b = reqwest::Client::builder().connect_timeout(CONNECT).user_agent(concat!("bragi/", env!("CARGO_PKG_VERSION")));
    if let Some(t) = timeout {
        b = b.timeout(t);
    }
    b.build().map_err(|e| CoreError::new(ErrorKind::Other, format!("http client: {e}")))
}

fn authed(r: reqwest::RequestBuilder, t: &Target) -> reqwest::RequestBuilder {
    match t.api_key.as_deref().filter(|k| !k.is_empty()) {
        Some(k) => r.bearer_auth(k),
        None => r,
    }
}

/// An error from a transport failure. `reqwest` errors can carry the URL; the URL has no secret
/// (see [`normalize_url`]) but is dropped anyway so messages stay short.
fn transport(e: reqwest::Error) -> CoreError {
    let what = if e.is_timeout() {
        "the endpoint timed out"
    } else if e.is_connect() {
        "couldn't connect to the endpoint"
    } else {
        "the connection to the endpoint failed"
    };
    CoreError::new(ErrorKind::Unavailable, what)
}

/// An HTTP error status as a typed error, with the API's own message when it sent one.
async fn status_error(resp: reqwest::Response) -> CoreError {
    let code = resp.status();
    let body = resp.text().await.unwrap_or_default();
    let detail = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.pointer("/error/message").or_else(|| v.get("error")).and_then(|m| m.as_str().map(str::to_string)))
        .unwrap_or_default();
    let detail: String = detail.chars().take(300).collect();
    let (kind, head) = match code.as_u16() {
        401 | 403 => (ErrorKind::Refused, "the endpoint refused the API key".to_string()),
        404 => (ErrorKind::BadRequest, "the endpoint has no such address or model (404)".to_string()),
        429 => (ErrorKind::Busy, "the endpoint is rate limiting you (429)".to_string()),
        400..=499 => (ErrorKind::BadRequest, format!("the endpoint rejected the request ({})", code.as_u16())),
        _ => (ErrorKind::Unavailable, format!("the endpoint failed ({})", code.as_u16())),
    };
    CoreError::new(kind, if detail.is_empty() { head } else { format!("{head}: {detail}") })
}

/// `GET {base}/models`: the model ids, sorted.
pub async fn list_models(t: &Target) -> Result<Vec<String>> {
    let resp = authed(client(Some(LIST_TIMEOUT))?.get(format!("{}/models", t.base_url)), t).send().await.map_err(transport)?;
    if !resp.status().is_success() {
        return Err(status_error(resp).await);
    }
    let v: serde_json::Value =
        resp.json().await.map_err(|_| CoreError::new(ErrorKind::BadRequest, "that address answered, but not like an OpenAI-compatible API"))?;
    let data = v
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| CoreError::new(ErrorKind::BadRequest, "that address answered, but not like an OpenAI-compatible API"))?;
    let mut ids: Vec<String> = data.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(str::to_string)).collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// Lists the models and says how long it took; never an error, the report carries it.
pub async fn test(t: &Target) -> TestReport {
    let start = Instant::now();
    match list_models(t).await {
        Ok(models) => {
            let message = format!("Connected; {} model{}.", models.len(), if models.len() == 1 { "" } else { "s" });
            TestReport { ok: true, models, latency_ms: start.elapsed().as_millis() as u64, message }
        }
        Err(e) => TestReport { ok: false, models: Vec::new(), latency_ms: start.elapsed().as_millis() as u64, message: e.message },
    }
}

/// What a streamed completion ended with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Finished {
    pub finish_reason: Option<String>,
}

/// `POST {base}/chat/completions` with `stream: true`; text, reasoning and usage go to `tx` as the
/// pool's events so the shared coalescer and think-splitter handle them. Errors before any text are
/// plain errors; so is a cut-off stream (the caller keeps what arrived).
pub async fn chat_stream(
    t: &Target,
    model: &str,
    messages: &[Message],
    tx: &mpsc::UnboundedSender<PoolEvent>,
    cancel: &CancelToken,
) -> Result<Finished> {
    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    let req = authed(client(None)?.post(format!("{}/chat/completions", t.base_url)), t).json(&body);
    let resp = tokio::select! {
        r = req.send() => r.map_err(transport)?,
        _ = cancel.cancelled() => return Err(CoreError::new(ErrorKind::Cancelled, "stopped")),
    };
    if !resp.status().is_success() {
        return Err(status_error(resp).await);
    }
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut out = Finished::default();
    loop {
        let chunk = tokio::select! {
            c = stream.next() => c,
            _ = cancel.cancelled() => return Err(CoreError::new(ErrorKind::Cancelled, "stopped")),
        };
        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(|_| CoreError::new(ErrorKind::Unavailable, "the endpoint's stream was cut off"))?;
        buf.extend_from_slice(&chunk);
        while let Some(nl) = buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buf.drain(..=nl).collect();
            let line = String::from_utf8_lossy(&line);
            let Some(data) = line.trim().strip_prefix("data:") else { continue };
            let data = data.trim();
            if data == "[DONE]" {
                return Ok(out);
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else { continue };
            if let Some(e) = v.get("error") {
                let m = e.get("message").and_then(|m| m.as_str()).unwrap_or("the endpoint reported an error");
                return Err(CoreError::new(ErrorKind::Other, m.chars().take(300).collect::<String>()));
            }
            if let Some(c) = v.pointer("/choices/0") {
                let d = c.get("delta");
                let reasoning = d.and_then(|d| d.get("reasoning_content").or_else(|| d.get("reasoning"))).and_then(|r| r.as_str());
                if let Some(r) = reasoning.filter(|r| !r.is_empty()) {
                    let _ = tx.send(PoolEvent::Reasoning(r.to_string()));
                }
                if let Some(text) = d.and_then(|d| d.get("content")).and_then(|c| c.as_str()).filter(|c| !c.is_empty()) {
                    let _ = tx.send(PoolEvent::Text(text.to_string()));
                }
                if let Some(f) = c.get("finish_reason").and_then(|f| f.as_str()) {
                    out.finish_reason = Some(f.to_string());
                }
            }
            if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
                let n = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
                let mut usage = Usage::default();
                usage.prompt_tokens = n("prompt_tokens");
                usage.completion_tokens = n("completion_tokens");
                let _ = tx.send(PoolEvent::Usage(usage));
            }
        }
    }
    // The body ended without `[DONE]`: some servers do that after the last chunk.
    if out.finish_reason.is_none() {
        return Err(CoreError::new(ErrorKind::Unavailable, "the endpoint's stream ended early"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(normalize_url(" https://api.example.com/v1/ ").unwrap(), "https://api.example.com/v1");
        assert!(normalize_url("ftp://x").is_err());
        assert!(normalize_url("https://key@host/v1").is_err());
        assert!(normalize_url("https://").is_err());
        assert!(normalize_url("https://h/v1?k=1").is_err());
        assert!(is_insecure("http://example.com/v1"));
        assert!(!is_insecure("http://localhost:8080/v1"));
        assert!(!is_insecure("http://127.0.0.1:1/v1"));
        assert!(!is_insecure("https://example.com"));
    }

    #[test]
    fn debug_hides_key() {
        let t = Target { base_url: "https://h/v1".into(), api_key: Some("sk-secret".into()) };
        assert!(!format!("{t:?}").contains("sk-secret"));
    }
}
