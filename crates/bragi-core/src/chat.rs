//! What a shell hears while an answer streams. Deltas are coalesced (every ~16 ms or 32 pieces)
//! so a UI thread or the Dart isolate isn't flooded; nothing is dropped.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use weirdnet_agent::pool::PoolEvent;

use crate::error::ErrorKind;

/// One streamed event, mirroring the infer/2 frames (offsets in characters of the answer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    /// Waiting on a full node.
    Queued {
        position: u32,
    },
    Started {
        node: String,
        model: String,
        resumed: bool,
    },
    Delta {
        offset: u64,
        text: String,
    },
    Reasoning {
        offset: u64,
        text: String,
    },
    /// The attempt failed before any text and is being tried again.
    Retry {
        attempt: u32,
        message: String,
    },
    /// What the pool knew about the answer: served from the cache, or by a stand-in.
    Meta {
        cache_hit: bool,
        standin: Option<String>,
        served_by: Option<String>,
    },
    /// The answer continues on another node.
    Resumed {
        node: String,
        offset: u64,
    },
    Usage {
        prompt_tokens: u64,
        completion_tokens: u64,
    },
    Done {
        finish_reason: Option<String>,
    },
    /// Stopped (kind `cancelled`) or failed; `partial` is what was shown.
    Error {
        kind: ErrorKind,
        message: String,
        partial: String,
    },
}

pub const COALESCE_EVERY: Duration = Duration::from_millis(16);
pub const COALESCE_PIECES: usize = 32;

/// Turns pool events into coalesced [`ChatEvent`]s on `out`, starting the text at `offset`
/// (a continued answer starts after what was shown). Returns the text and reasoning streamed.
pub async fn coalesce(mut rx: mpsc::UnboundedReceiver<PoolEvent>, out: mpsc::UnboundedSender<ChatEvent>, offset: u64) -> (String, String) {
    let (mut text, mut reasoning) = (String::new(), String::new());
    let (mut pending, mut pending_reasoning, mut pieces) = (String::new(), String::new(), 0usize);
    let (mut at, mut reasoning_at) = (offset, 0u64);
    let flush = |pending: &mut String, pending_reasoning: &mut String, at: &mut u64, reasoning_at: &mut u64| {
        if !pending_reasoning.is_empty() {
            let t = std::mem::take(pending_reasoning);
            let n = t.chars().count() as u64;
            let _ = out.send(ChatEvent::Reasoning { offset: *reasoning_at, text: t });
            *reasoning_at += n;
        }
        if !pending.is_empty() {
            let t = std::mem::take(pending);
            let n = t.chars().count() as u64;
            let _ = out.send(ChatEvent::Delta { offset: *at, text: t });
            *at += n;
        }
    };
    let mut tick = tokio::time::interval(COALESCE_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            ev = rx.recv() => {
                let Some(ev) = ev else { break };
                match ev {
                    PoolEvent::Text(t) => {
                        text.push_str(&t);
                        pending.push_str(&t);
                        pieces += 1;
                    }
                    PoolEvent::Reasoning(t) => {
                        reasoning.push_str(&t);
                        pending_reasoning.push_str(&t);
                        pieces += 1;
                    }
                    other => {
                        flush(&mut pending, &mut pending_reasoning, &mut at, &mut reasoning_at);
                        pieces = 0;
                        let ev = match other {
                            PoolEvent::Queued { position, .. } => Some(ChatEvent::Queued { position }),
                            PoolEvent::Accepted { node, model, resume } => Some(ChatEvent::Started { node, model, resumed: resume }),
                            PoolEvent::Resumed { node, offset } => Some(ChatEvent::Resumed { node, offset }),
                            PoolEvent::Meta(m) => {
                                Some(ChatEvent::Meta { cache_hit: m.cache_hit, standin: m.standin, served_by: m.served_by })
                            }
                            PoolEvent::Usage(u) => Some(ChatEvent::Usage { prompt_tokens: u.prompt_tokens, completion_tokens: u.completion_tokens }),
                            _ => None,
                        };
                        if let Some(ev) = ev {
                            let _ = out.send(ev);
                        }
                    }
                }
                if pieces >= COALESCE_PIECES {
                    flush(&mut pending, &mut pending_reasoning, &mut at, &mut reasoning_at);
                    pieces = 0;
                }
            }
            _ = tick.tick() => {
                flush(&mut pending, &mut pending_reasoning, &mut at, &mut reasoning_at);
                pieces = 0;
            }
        }
    }
    flush(&mut pending, &mut pending_reasoning, &mut at, &mut reasoning_at);
    (text, reasoning)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn coalesces_without_dropping() {
        let (tx, rx) = mpsc::unbounded_channel();
        let (out, mut events) = mpsc::unbounded_channel();
        tx.send(PoolEvent::Accepted { node: "n".into(), model: "m".into(), resume: false }).unwrap();
        for i in 0..100 {
            tx.send(PoolEvent::Text(format!("{i} "))).unwrap();
        }
        drop(tx);
        let (text, _) = coalesce(rx, out, 5).await;
        let mut shown = String::new();
        let mut deltas = 0;
        let mut next = 5;
        while let Ok(ev) = events.try_recv() {
            if let ChatEvent::Delta { offset, text } = ev {
                assert_eq!(offset, next, "offsets continue the buffer");
                next += text.chars().count() as u64;
                shown.push_str(&text);
                deltas += 1;
            }
        }
        assert_eq!(shown, text);
        assert!(deltas <= 100usize.div_ceil(COALESCE_PIECES) + 1, "{deltas} deltas");
    }
}
