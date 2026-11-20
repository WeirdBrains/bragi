//! Agent mode: a whole task run by a companion on one of the user's nodes (`weirdnet-agent`
//! `remote::task`, task/1). The node's tools and policy apply; the approval cards come here, and
//! the user answers them with [`Core::approve`]. The conversation stays with this device: its
//! history goes out with each task and the answer is saved like any other.
//!
//! A node has to serve task/1. Ymir's nodes don't yet (the published `weirdnet-agent::serve::Server`
//! only serves infer and catalog), so against a real Ymir node agent mode answers "unavailable";
//! the demo nodes serve it.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};
use weirdnet_agent::policy::{ApprovalPrompt, Scope};
use weirdnet_agent::remote::task::{TaskRequest, run as run_task};
use weirdnet_agent::tool::ToolSpec;
use weirdnet_agent::{AgentEvent, AgentId, ApprovalAnswer, Approver, CancelToken, RemoteError, StopReason};

use crate::attach::Attachment;
use crate::chat::{COALESCE_EVERY, COALESCE_PIECES, ChatEvent};
use crate::core::{ChatHandle, Core, lock, random_id};
use crate::error::{CoreError, ErrorKind, Result};
use crate::history::MessageState;

/// The companion asked to run a task when the shell doesn't name one.
pub const DEFAULT_COMPANION: &str = "assistant";

/// What a tool call's state is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolState {
    Started,
    Done,
    Failed,
}

/// The user's answer to an approval card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    Deny,
    /// Just this step.
    Once,
    /// The rest of this task.
    ThisTask,
}

/// Cards waiting for an answer in one task.
#[derive(Default)]
pub struct Cards {
    next: AtomicU64,
    waiting: std::sync::Mutex<HashMap<u64, oneshot::Sender<ApprovalAnswer>>>,
}

/// The task's approver: the pump has already shown the card and queued where its answer arrives.
struct CardApprover {
    queue: AsyncMutex<mpsc::UnboundedReceiver<(u64, oneshot::Receiver<ApprovalAnswer>)>>,
    out: mpsc::UnboundedSender<ChatEvent>,
}

#[async_trait]
impl Approver for CardApprover {
    async fn decide(&self, prompt: &ApprovalPrompt, tool: &ToolSpec) -> ApprovalAnswer {
        // The pump queues the card after flushing the text before it; showing it from here
        // (which knows the tool) keeps the order.
        let Some((ask, rx)) = self.queue.lock().await.recv().await else { return ApprovalAnswer::Deny };
        let _ = self.out.send(ChatEvent::Approval {
            ask,
            tool: tool.name.clone(),
            tier: tier_name(prompt.request.tier),
            site: prompt.request.site.clone(),
            summary: summary(prompt, &tool.name),
        });
        rx.await.unwrap_or(ApprovalAnswer::Deny)
    }
}

fn tier_name(t: Option<weirdnet_agent::Tier>) -> String {
    use weirdnet_agent::Tier::*;
    match t {
        Some(Read) => "read",
        Some(Navigate) => "navigate",
        Some(Research) => "research",
        Some(Interact) => "interact",
        _ => "other",
    }
    .into()
}

fn summary(p: &ApprovalPrompt, tool: &str) -> String {
    match (&p.request.site, &p.request.provider) {
        (Some(site), _) => format!("{tool} wants to use {site}"),
        (None, Some(provider)) => format!("{tool} wants to search with {provider}"),
        _ => format!("{tool} wants to run"),
    }
}

/// Turns the task's events into [`ChatEvent`]s: text coalesced like a chat answer, the rest sent
/// in order after the text before it. Returns the text and the tools used.
async fn pump(
    mut rx: mpsc::UnboundedReceiver<AgentEvent>,
    out: mpsc::UnboundedSender<ChatEvent>,
    node: String,
    companion: String,
    cards: Arc<Cards>,
    queue: mpsc::UnboundedSender<(u64, oneshot::Receiver<ApprovalAnswer>)>,
) -> (String, Vec<String>) {
    let (mut text, mut pending, mut pieces, mut at) = (String::new(), String::new(), 0usize, 0u64);
    let mut tools: Vec<String> = Vec::new();
    let flush = |pending: &mut String, at: &mut u64| {
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
                if let AgentEvent::TextDelta(t) = ev {
                    text.push_str(&t);
                    pending.push_str(&t);
                    pieces += 1;
                    if pieces >= COALESCE_PIECES {
                        flush(&mut pending, &mut at);
                        pieces = 0;
                    }
                    continue;
                }
                flush(&mut pending, &mut at);
                pieces = 0;
                match ev {
                    AgentEvent::TaskStarted { .. } => {
                        let _ = out.send(ChatEvent::Started { node: node.clone(), model: companion.clone(), resumed: false });
                    }
                    AgentEvent::ApprovalRequested(_) => {
                        let (tx, answer) = oneshot::channel();
                        let ask = cards.next.fetch_add(1, Ordering::SeqCst);
                        lock(&cards.waiting).insert(ask, tx);
                        let _ = queue.send((ask, answer));
                    }
                    AgentEvent::ApprovalSkipped(n) => {
                        let _ = out.send(ChatEvent::Notice { text: format!("allowed by a standing rule ({} on {})", tier_name(Some(n.tier)), n.site.unwrap_or_else(|| "any site".into())) });
                    }
                    AgentEvent::Blocked { tool, reason } => {
                        let _ = out.send(ChatEvent::Notice { text: format!("{tool} was blocked: {reason}") });
                    }
                    AgentEvent::ToolStarted { call_id, tool } => {
                        if !tools.contains(&tool) {
                            tools.push(tool.clone());
                        }
                        let _ = out.send(ChatEvent::Tool { call_id, tool, state: ToolState::Started });
                    }
                    AgentEvent::ToolFinished { call_id, tool, is_error } => {
                        let _ = out.send(ChatEvent::Tool { call_id, tool, state: if is_error { ToolState::Failed } else { ToolState::Done } });
                    }
                    _ => {}
                }
            }
            _ = tick.tick() => {
                flush(&mut pending, &mut at);
                pieces = 0;
            }
        }
    }
    flush(&mut pending, &mut at);
    (text, tools)
}

impl Core {
    /// Sends `prompt` as an agent task to `node` (the first inference node when `None`), run by
    /// `companion` there ([`DEFAULT_COMPANION`] when `None`). Events are the chat's, plus
    /// `Tool`, `Approval` and `Notice`; answer an `Approval` with [`approve`](Self::approve).
    pub fn send_agent(
        &self,
        conversation: Option<&str>,
        prompt: &str,
        node: Option<String>,
        companion: Option<String>,
        attachments: Vec<Attachment>,
    ) -> Result<ChatHandle> {
        let net = self.net()?;
        let node = match node.filter(|n| !n.is_empty()) {
            Some(n) => n,
            None => self
                .state()
                .catalog
                .nodes
                .first()
                .map(|n| n.id.clone())
                .ok_or_else(|| CoreError::new(ErrorKind::Unavailable, "no inference node found yet"))?,
        };
        let companion = companion.filter(|c| !c.is_empty()).unwrap_or_else(|| DEFAULT_COMPANION.into());
        let (conv, mut messages) = self.begin_turn(conversation, prompt, &companion, attachments)?;
        let input = messages.pop().map(|m| m.content).unwrap_or_default();
        let request_id = random_id("r");
        let cancel = CancelToken::new();
        lock(&self.inner.requests).insert(request_id.clone(), cancel.clone());
        let cards = Arc::new(Cards::default());
        lock(&self.inner.cards).insert(request_id.clone(), cards.clone());
        let (out, events) = mpsc::unbounded_channel();
        let (core, rid, conversation_id) = (self.clone(), request_id.clone(), conv.clone());
        self.inner.handle.spawn(async move {
            let (ev_tx, ev_rx) = mpsc::unbounded_channel();
            let (q_tx, q_rx) = mpsc::unbounded_channel();
            let approver = CardApprover { queue: AsyncMutex::new(q_rx), out: out.clone() };
            let pump = tokio::spawn(pump(ev_rx, out.clone(), node.clone(), companion.clone(), cards, q_tx));
            let request = TaskRequest::new(AgentId::new(companion.as_str()), input, messages);
            let result = run_task(&*net.dialer, &node, request, Some(ev_tx), &approver, &cancel).await;
            let (text, tools) = pump.await.unwrap_or_default();
            let (state, last) = match &result {
                Ok(o) => match &o.outcome.stop {
                    StopReason::Completed => (MessageState::Complete, ChatEvent::Done { finish_reason: Some("agent".into()) }),
                    StopReason::Cancelled => {
                        (MessageState::Stopped, ChatEvent::Error { kind: ErrorKind::Cancelled, message: "stopped".into(), partial: text.clone() })
                    }
                    other => (
                        MessageState::Failed,
                        ChatEvent::Error { kind: ErrorKind::Other, message: format!("the task stopped early: {other:?}"), partial: text.clone() },
                    ),
                },
                Err(e) => {
                    let e = agent_error(e);
                    let state = if e.kind == ErrorKind::Cancelled { MessageState::Stopped } else { MessageState::Failed };
                    (state, ChatEvent::Error { kind: e.kind, message: e.message, partial: text.clone() })
                }
            };
            core.finish_agent(&conv, text, state, node, companion, tools);
            lock(&core.inner.requests).remove(&rid);
            lock(&core.inner.cards).remove(&rid);
            let _ = out.send(last);
        });
        Ok(ChatHandle { request_id, conversation_id, events })
    }

    /// Answers the approval card `ask` of an agent task. False when the card isn't waiting
    /// (answered already, or the task ended).
    pub fn approve(&self, request_id: &str, ask: u64, choice: Choice) -> bool {
        let Some(cards) = lock(&self.inner.cards).get(request_id).cloned() else { return false };
        let Some(tx) = lock(&cards.waiting).remove(&ask) else { return false };
        let answer = match choice {
            Choice::Deny => ApprovalAnswer::Deny,
            Choice::Once => ApprovalAnswer::Approve { scope: Scope::Once },
            Choice::ThisTask => ApprovalAnswer::Approve { scope: Scope::Task },
        };
        tx.send(answer).is_ok()
    }

    fn finish_agent(&self, conversation: &str, text: String, state: MessageState, node: String, companion: String, tools: Vec<String>) {
        let Ok(mut c) = self.inner.history.load(conversation) else { return };
        if let Some(m) = c.messages.last_mut().filter(|m| m.role == crate::history::Role::Assistant) {
            m.text = text;
            m.state = state;
            m.node = Some(node);
            m.model = Some(companion);
            m.tools = tools;
            m.agent = true;
        }
        c.updated_ms = crate::history::now_ms();
        if let Err(e) = self.inner.history.save(&c) {
            tracing::warn!("saving {conversation}: {e}");
        }
    }
}

/// A task error in words the shell can show: a node that doesn't serve tasks says so.
fn agent_error(e: &RemoteError) -> CoreError {
    let mut c = CoreError::from(e);
    if matches!(e, RemoteError::Unreachable { .. } | RemoteError::Closed | RemoteError::Unsupported { .. } | RemoteError::Failed { .. }) {
        c.kind = ErrorKind::Unavailable;
        c.message = format!("that node can't run agent tasks ({e})");
    }
    c
}
