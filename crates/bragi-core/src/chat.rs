//! What a shell hears while an answer streams. Deltas are coalesced (every ~16 ms or 32 pieces)
//! so a UI thread or the Dart isolate isn't flooded; nothing is dropped.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use weirdnet_agent::pool::PoolEvent;

use crate::error::ErrorKind;
use crate::think::{Piece, ThinkSplit};

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
    /// The model's reasoning ("thinking"), streamed apart from the answer; offsets in characters
    /// of the reasoning. Kept with the message, never sent back to the model.
    Reasoning {
        offset: u64,
        text: String,
    },
    /// The reasoning is over (the answer started, or the stream ended): how long it took from
    /// the first reasoning, in milliseconds.
    Thought {
        elapsed_ms: u64,
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
    /// Agent mode: a tool call on the node started or finished.
    Tool {
        call_id: String,
        tool: String,
        state: crate::agent::ToolState,
    },
    /// Agent mode: the node asks to do something. Answer with `Core::approve(request_id, ask, …)`.
    Approval {
        ask: u64,
        tool: String,
        /// `read`, `navigate`, `research`, `interact` or `other`.
        tier: String,
        site: Option<String>,
        summary: String,
    },
    /// Agent mode: something worth saying that isn't a question (a step blocked or let through).
    Notice {
        text: String,
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

/// What one stream produced: the answer text and the reasoning (both without what was shown
/// before it), and how long the model thought, when it did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Streamed {
    pub text: String,
    pub reasoning: String,
    pub thinking_ms: Option<u64>,
}

/// Turns pool events into coalesced [`ChatEvent`]s on `out`. The answer's offsets start at
/// `offset` and the reasoning's at `reasoning_offset` (a continued answer starts after what was
/// shown). Reasoning comes from the pool's reasoning frames (a server-side parser such as
/// llama.cpp's `--reasoning-format` or vLLM's) and from a `<think>…</think>` block that opens the
/// text ([`crate::think`]). The first answer text after reasoning, or the end of the stream, sends
/// [`ChatEvent::Thought`] with the time since the first reasoning arrived.
pub async fn coalesce(
    mut rx: mpsc::UnboundedReceiver<PoolEvent>,
    out: mpsc::UnboundedSender<ChatEvent>,
    offset: u64,
    reasoning_offset: u64,
) -> Streamed {
    let mut s = Coalescer {
        out,
        done: Streamed::default(),
        pending: String::new(),
        pending_reasoning: String::new(),
        pieces: 0,
        at: offset,
        reasoning_at: reasoning_offset,
        thinking_since: None,
        // A continued answer is past any opening block.
        split: if offset > 0 { ThinkSplit::continuing() } else { ThinkSplit::new() },
    };
    let mut tick = tokio::time::interval(COALESCE_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            ev = rx.recv() => {
                let Some(ev) = ev else { break };
                match ev {
                    PoolEvent::Text(t) => {
                        for p in s.split.push(&t) {
                            s.piece(p);
                        }
                    }
                    PoolEvent::Reasoning(t) => s.piece(Piece::Reasoning(t)),
                    other => {
                        s.flush();
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
                            let _ = s.out.send(ev);
                        }
                    }
                }
                if s.pieces >= COALESCE_PIECES {
                    s.flush();
                }
            }
            _ = tick.tick() => s.flush(),
        }
    }
    for p in s.split.finish() {
        s.piece(p);
    }
    s.flush();
    s.thought();
    s.done
}

struct Coalescer {
    out: mpsc::UnboundedSender<ChatEvent>,
    done: Streamed,
    pending: String,
    pending_reasoning: String,
    pieces: usize,
    at: u64,
    reasoning_at: u64,
    /// When the first reasoning arrived.
    thinking_since: Option<Instant>,
    split: ThinkSplit,
}

impl Coalescer {
    fn piece(&mut self, p: Piece) {
        match p {
            Piece::Reasoning(t) => {
                self.thinking_since.get_or_insert_with(Instant::now);
                self.done.reasoning.push_str(&t);
                self.pending_reasoning.push_str(&t);
            }
            Piece::Text(t) => {
                if self.done.thinking_ms.is_none() && self.thinking_since.is_some() {
                    self.flush();
                    self.thought();
                }
                self.done.text.push_str(&t);
                self.pending.push_str(&t);
            }
        }
        self.pieces += 1;
    }

    /// The reasoning is over: says how long it took, once.
    fn thought(&mut self) {
        if let (Some(since), None) = (self.thinking_since, self.done.thinking_ms) {
            let ms = since.elapsed().as_millis() as u64;
            self.done.thinking_ms = Some(ms);
            let _ = self.out.send(ChatEvent::Thought { elapsed_ms: ms });
        }
    }

    fn flush(&mut self) {
        if !self.pending_reasoning.is_empty() {
            let t = std::mem::take(&mut self.pending_reasoning);
            let n = t.chars().count() as u64;
            let _ = self.out.send(ChatEvent::Reasoning { offset: self.reasoning_at, text: t });
            self.reasoning_at += n;
        }
        if !self.pending.is_empty() {
            let t = std::mem::take(&mut self.pending);
            let n = t.chars().count() as u64;
            let _ = self.out.send(ChatEvent::Delta { offset: self.at, text: t });
            self.at += n;
        }
        self.pieces = 0;
    }
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
        let text = coalesce(rx, out, 5, 0).await.text;
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
