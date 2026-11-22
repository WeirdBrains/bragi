//! A fake OpenAI-compatible endpoint on 127.0.0.1 for the tests and both shells' demo profile:
//! a Ymir bridge (models with `weirdnet` fields) or a generic API, with a bearer token, streamed
//! canned answers ([`crate::demo::answer_for`]), `continue_final_message`, and a scripted stream
//! break. Plain HTTP/1.1 over a local socket; nothing leaves the machine.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::error::{CoreError, ErrorKind, Result};

/// The token the demo profile's endpoints take.
pub const DEMO_TOKEN: &str = "demo-bridge-token-not-a-secret";

pub struct FakeBridge {
    port: u16,
    pub token: String,
    ymir: bool,
    models: Vec<String>,
    delay: Duration,
    /// The next answer's stream breaks after this many words.
    die_after: Mutex<Option<usize>>,
    /// Chat requests answered, and the last one's body.
    chats: AtomicU32,
    last: Mutex<Option<Value>>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Drop for FakeBridge {
    fn drop(&mut self) {
        if let Some(t) = self.task.lock().unwrap_or_else(|e| e.into_inner()).take() {
            t.abort();
        }
    }
}

impl FakeBridge {
    /// Starts one: a Ymir bridge when `ymir`, serving `models` (chat models), answering a word
    /// every `delay`.
    pub async fn start(ymir: bool, token: &str, models: &[&str], delay: Duration) -> Result<Arc<Self>> {
        let l = TcpListener::bind("127.0.0.1:0").await.map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?;
        let port = l.local_addr().map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?.port();
        let me = Arc::new(Self {
            port,
            token: token.into(),
            ymir,
            models: models.iter().map(|m| m.to_string()).collect(),
            delay,
            die_after: Mutex::new(None),
            chats: AtomicU32::new(0),
            last: Mutex::new(None),
            task: Mutex::new(None),
        });
        let weak = Arc::downgrade(&me);
        let task = tokio::spawn(async move {
            while let Ok((sock, _)) = l.accept().await {
                let Some(me) = weak.upgrade() else { return };
                tokio::spawn(async move {
                    let _ = me.serve(sock).await;
                });
            }
        });
        *me.task.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
        Ok(me)
    }

    /// The API base to configure: `http://127.0.0.1:PORT/v1`.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    pub fn die_next_answer_after(&self, words: usize) {
        *self.die_after.lock().unwrap_or_else(|e| e.into_inner()) = Some(words);
    }

    pub fn chats(&self) -> u32 {
        self.chats.load(Ordering::SeqCst)
    }

    pub fn last_request(&self) -> Option<Value> {
        self.last.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    async fn serve(&self, mut sock: TcpStream) -> std::io::Result<()> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            let n = sock.read(&mut chunk).await?;
            if n == 0 {
                return Ok(());
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i;
            }
            if buf.len() > 1 << 20 {
                return Ok(());
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let mut lines = head.lines();
        let first = lines.next().unwrap_or("");
        let mut parts = first.split_whitespace();
        let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let mut len = 0usize;
        let mut auth = String::new();
        for l in lines {
            if let Some((k, v)) = l.split_once(':') {
                match k.trim().to_ascii_lowercase().as_str() {
                    "content-length" => len = v.trim().parse().unwrap_or(0),
                    "authorization" => auth = v.trim().to_string(),
                    _ => {}
                }
            }
        }
        let mut body = buf[head_end + 4..].to_vec();
        while body.len() < len {
            let n = sock.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
        if auth != format!("Bearer {}", self.token) {
            return reply(&mut sock, "401 Unauthorized", &json!({"error": {"message": "missing or wrong token", "type": "unauthorized"}})).await;
        }
        match (method, path) {
            ("GET", "/v1/models") => {
                let data: Vec<Value> = if self.ymir {
                    let mut d: Vec<Value> = self
                        .models
                        .iter()
                        .enumerate()
                        .map(|(i, m)| {
                            json!({"id": m, "object": "model", "created": 0, "owned_by": "weirdnet",
                                "weirdnet": {"state": if i == 0 { "warm" } else { "loadable" }, "purpose": "chat", "replicas": u32::from(i == 0), "context": 8192, "revision": null}})
                        })
                        .collect();
                    d.push(json!({"id": "demo-embed", "object": "model", "created": 0, "owned_by": "weirdnet", "weirdnet": {"state": "warm", "purpose": "embedding", "replicas": 1}}));
                    d
                } else {
                    self.models.iter().map(|m| json!({"id": m, "object": "model", "created": 0, "owned_by": "fake-openai"})).collect()
                };
                reply(&mut sock, "200 OK", &json!({"object": "list", "data": data})).await
            }
            ("POST", "/v1/chat/completions") => {
                let Ok(req) = serde_json::from_slice::<Value>(&body) else {
                    return reply(&mut sock, "400 Bad Request", &json!({"error": {"message": "invalid JSON"}})).await;
                };
                self.chats.fetch_add(1, Ordering::SeqCst);
                *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(req.clone());
                let model = req["model"].as_str().map(str::to_string).or_else(|| self.models.first().cloned()).unwrap_or_default();
                if !self.models.contains(&model) {
                    return reply(&mut sock, "404 Not Found", &json!({"error": {"message": format!("unknown model {model}")}})).await;
                }
                let msgs = req["messages"].as_array().cloned().unwrap_or_default();
                let question = msgs.iter().rev().find(|m| m["role"] == "user").and_then(|m| m["content"].as_str()).unwrap_or("").to_string();
                let answer = crate::demo::answer_for(&model, &question);
                let continuing = req["continue_final_message"] == true;
                let prefix = if continuing { msgs.last().and_then(|m| m["content"].as_str()).unwrap_or("").chars().count() } else { 0 };
                let rest: String = answer.chars().skip(prefix).collect();
                let die = self.die_after.lock().unwrap_or_else(|e| e.into_inner()).take();
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nx-weirdnet-model: {model}\r\nConnection: close\r\n\r\n"
                );
                sock.write_all(head.as_bytes()).await?;
                let chunk = |delta: Value, finish: Option<&str>| {
                    format!(
                        "data: {}\n\n",
                        json!({"id": "chatcmpl-demo", "object": "chat.completion.chunk", "created": 0, "model": model, "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
                    )
                };
                sock.write_all(chunk(json!({"role": "assistant", "content": ""}), None).as_bytes()).await?;
                for (i, w) in rest.split_inclusive(' ').enumerate() {
                    if die.is_some_and(|d| i >= d) {
                        // The bridge's node went away mid-answer: the connection just ends.
                        sock.shutdown().await?;
                        return Ok(());
                    }
                    tokio::time::sleep(self.delay).await;
                    sock.write_all(chunk(json!({"content": w}), None).as_bytes()).await?;
                }
                sock.write_all(chunk(json!({}), Some("stop")).as_bytes()).await?;
                let words = rest.split_whitespace().count() as u64;
                let usage = json!({"id": "chatcmpl-demo", "object": "chat.completion.chunk", "created": 0, "model": model, "choices": [],
                    "usage": {"prompt_tokens": question.split_whitespace().count(), "completion_tokens": words, "total_tokens": words}});
                sock.write_all(format!("data: {usage}\n\ndata: [DONE]\n\n").as_bytes()).await?;
                sock.shutdown().await
            }
            _ => reply(&mut sock, "404 Not Found", &json!({"error": {"message": "not found"}})).await,
        }
    }
}

async fn reply(sock: &mut TcpStream, status: &str, body: &Value) -> std::io::Result<()> {
    let b = body.to_string();
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len());
    sock.write_all(head.as_bytes()).await?;
    sock.write_all(b.as_bytes()).await?;
    sock.shutdown().await
}
