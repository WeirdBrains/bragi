//! Shared by the endpoint and session tests: a linked `Core` on the demo node, and a mock
//! OpenAI-compatible server (`/v1/models`, `/v1/chat/completions`) on a loopback port.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bragi_core::demo::DemoNode;
use bragi_core::{AppState, ChatEvent, Core, PairState};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub async fn until(core: &Core, what: &str, f: impl Fn(&AppState) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
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

/// A core linked to a demo node with its catalog loaded.
pub async fn linked(tmp: &std::path::Path) -> (Core, Arc<DemoNode>) {
    let demo = Arc::new(DemoNode::start(&tmp.join("demo"), Duration::from_millis(5)).await.unwrap());
    let core = Core::start(demo.app_config(&tmp.join("app"), "Bragi test")).unwrap();
    demo.auto_accept(&core);
    core.pair_offer().await.unwrap();
    until(&core, "check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    core.pair_confirm(true).unwrap();
    until(&core, "linked", |s| matches!(s.pair, PairState::Linked { .. }) && s.online).await;
    until(&core, "catalog", |s| !s.catalog.models.is_empty()).await;
    (core, demo)
}

/// Collects a handle's events up to the last one.
pub async fn drain(h: &mut bragi_core::ChatHandle) -> Vec<ChatEvent> {
    let mut v = Vec::new();
    while let Some(ev) = h.events.recv().await {
        let end = matches!(ev, ChatEvent::Done { .. } | ChatEvent::Error { .. });
        v.push(ev);
        if end {
            break;
        }
    }
    v
}

pub fn answer_text(events: &[ChatEvent]) -> String {
    events.iter().filter_map(|e| if let ChatEvent::Delta { text, .. } = e { Some(text.as_str()) } else { None }).collect()
}

#[derive(Clone, Debug)]
pub enum Chat {
    /// SSE chunks of content, then a finish and `[DONE]`.
    Words(Vec<&'static str>),
    /// Reasoning chunks, then content chunks.
    Reasoning(Vec<&'static str>, Vec<&'static str>),
    /// A plain error status and body.
    Status(u16, &'static str),
    /// Some words, then the connection ends with no finish and no `[DONE]`.
    CutOff(Vec<&'static str>),
    /// Asks for the tool `name` with `args` (a JSON object as text) until a request carries a tool
    /// result, then answers with the words.
    Tools(&'static str, &'static str, Vec<&'static str>),
    /// A request carrying a whole clip (`video_url`) gets the status and body; others get the words.
    RefuseVideo(u16, &'static str, Vec<&'static str>),
}

/// What a `video-*` model says in `/v1/models` (the mock's Ymir bridge with a model that reads
/// video): `weirdnet.video`, the context, the request size and the media limits, video included.
pub const VIDEO_MODEL_META: &str = r#"{"video":true,"vision":true,"context":32768,"max_request_bytes":4194304,"limits":{"image_tokens":256,"max_images":8,"video_max_seconds":60,"video_max_bytes":1000000,"video_frames_per_minute":120}}"#;

#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub auth: Option<String>,
    pub body: String,
}

pub struct Mock {
    pub url: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
}

impl Mock {
    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

/// `key`: when set, requests without `Authorization: Bearer <key>` get a 401.
pub async fn mock_openai(key: Option<&'static str>, models: Vec<&'static str>, chat: Chat) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let (log, models, chat) = (log.clone(), models.clone(), chat.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let (head, body) = loop {
                    let mut tmp = [0u8; 4096];
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..p]).to_string();
                        let len = head
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        while buf.len() < p + 4 + len {
                            let n = sock.read(&mut tmp).await.unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&tmp[..n]);
                        }
                        break (head, String::from_utf8_lossy(&buf[p + 4..]).to_string());
                    }
                };
                let mut first = head.lines().next().unwrap_or("").split(' ');
                let (method, path) = (first.next().unwrap_or("").to_string(), first.next().unwrap_or("").to_string());
                let auth = head.lines().find_map(|l| l.to_ascii_lowercase().starts_with("authorization:").then(|| l[14..].trim().to_string()));
                log.lock().unwrap().push(Seen { method, path: path.clone(), auth: auth.clone(), body: body.clone() });
                if let Some(k) = key
                    && auth.as_deref() != Some(&format!("Bearer {k}"))
                {
                    let b = r#"{"error":{"message":"Incorrect API key provided"}}"#;
                    let _ = sock
                        .write_all(
                            format!(
                                "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{b}",
                                b.len()
                            )
                            .as_bytes(),
                        )
                        .await;
                    return;
                }
                if path.ends_with("/models") {
                    let data: Vec<String> = models
                        .iter()
                        .map(|m| {
                            if m.starts_with("video-") {
                                format!(r#"{{"id":"{m}","object":"model","weirdnet":{VIDEO_MODEL_META}}}"#)
                            } else {
                                format!(r#"{{"id":"{m}","object":"model"}}"#)
                            }
                        })
                        .collect();
                    let b = format!(r#"{{"object":"list","data":[{}]}}"#, data.join(","));
                    let _ = sock
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{b}",
                                b.len()
                            )
                            .as_bytes(),
                        )
                        .await;
                    return;
                }
                if !path.ends_with("/chat/completions") {
                    let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
                    return;
                }
                let sse = |delta: String, finish: &str| format!("data: {{\"choices\":[{{\"delta\":{delta},\"finish_reason\":{finish}}}]}}\n\n");
                let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
                let words =
                    |w: &[&str], field: &str| w.iter().map(|w| sse(format!("{{\"{field}\":{}}}", serde_json::json!(w)), "null")).collect::<String>();
                let done = format!(
                    "{}data: {{\"choices\":[],\"usage\":{{\"prompt_tokens\":7,\"completion_tokens\":3}}}}\n\ndata: [DONE]\n\n",
                    sse("{}".into(), "\"stop\"")
                );
                let out = match chat {
                    Chat::Status(code, body) => format!(
                        "HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    ),
                    Chat::Words(w) => format!("{head}{}{done}", words(&w, "content")),
                    Chat::RefuseVideo(code, reply, _) if body_has_video(&body) => format!(
                        "HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                        reply.len()
                    ),
                    Chat::RefuseVideo(_, _, w) => format!("{head}{}{done}", words(&w, "content")),
                    Chat::Reasoning(r, w) => format!("{head}{}{}{done}", words(&r, "reasoning_content"), words(&w, "content")),
                    Chat::CutOff(w) => format!("{head}{}", words(&w, "content")),
                    Chat::Tools(_, _, w) if body.contains("\"role\":\"tool\"") => format!("{head}{}{done}", words(&w, "content")),
                    Chat::Tools(name, args, _) => {
                        let call = serde_json::json!({ "tool_calls": [{ "index": 0, "id": "call_1", "type": "function", "function": { "name": name, "arguments": args } }] });
                        format!("{head}{}{}", sse(call.to_string(), "null"), done.replacen("\"stop\"", "\"tool_calls\"", 1))
                    }
                };
                let _ = sock.write_all(out.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    Mock { url, seen }
}

/// Whether a chat request body carries a whole clip.
fn body_has_video(body: &str) -> bool {
    body.contains("\"video_url\"")
}
