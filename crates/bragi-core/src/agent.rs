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

/// Most model turns one task may take when Bragi runs the loop itself.
pub const MAX_STEPS: u32 = 8;

/// Called with the tool name when the user answers [`Choice::Always`].
type OnAlways = Arc<dyn Fn(&str) + Send + Sync>;

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
    /// This tool of an outside MCP server, from now on (this step runs, and the server's
    /// "always allow" list gets the tool). Other tools still ask.
    Always,
}

/// Cards waiting for an answer in one task.
#[derive(Default)]
pub struct Cards {
    next: AtomicU64,
    waiting: std::sync::Mutex<HashMap<u64, oneshot::Sender<ApprovalAnswer>>>,
    /// Cards answered [`Choice::Always`].
    always: std::sync::Mutex<std::collections::HashSet<u64>>,
}

/// The task's approver: the pump has already shown the card and queued where its answer arrives.
struct CardApprover {
    queue: AsyncMutex<mpsc::UnboundedReceiver<(u64, oneshot::Receiver<ApprovalAnswer>)>>,
    out: mpsc::UnboundedSender<ChatEvent>,
    cards: Arc<Cards>,
    /// Tools the user chose "always allow" for: their cards are answered without being shown.
    always: std::collections::HashSet<String>,
    /// Called with the tool when the user answers [`Choice::Always`].
    on_always: Option<OnAlways>,
}

#[async_trait]
impl Approver for CardApprover {
    async fn decide(&self, prompt: &ApprovalPrompt, tool: &ToolSpec) -> ApprovalAnswer {
        // The pump queues the card after flushing the text before it; showing it from here
        // (which knows the tool) keeps the order.
        let Some((ask, rx)) = self.queue.lock().await.recv().await else { return ApprovalAnswer::Deny };
        if self.always.contains(&tool.name) {
            lock(&self.cards.waiting).remove(&ask);
            let _ = self.out.send(ChatEvent::Notice { text: format!("{} ran: you chose always allow for it", tool.name) });
            return ApprovalAnswer::Approve { scope: Scope::Once };
        }
        let _ = self.out.send(ChatEvent::Approval {
            ask,
            tool: tool.name.clone(),
            tier: tier_name(prompt.request.tier),
            site: prompt.request.site.clone(),
            summary: summary(prompt, &tool.name),
        });
        let answer = rx.await.unwrap_or(ApprovalAnswer::Deny);
        if lock(&self.cards.always).remove(&ask)
            && matches!(answer, ApprovalAnswer::Approve { .. })
            && let Some(f) = &self.on_always
        {
            f(&tool.name);
        }
        answer
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

/// What a task's events added up to.
#[derive(Debug, Default)]
struct Pumped {
    text: String,
    tools: Vec<String>,
    /// The model's reasoning across the task's turns (kept with the answer, like a chat's).
    reasoning: String,
    thinking_ms: Option<u64>,
}

/// Turns the task's events into [`ChatEvent`]s: text coalesced like a chat answer, reasoning
/// streamed apart (and timed, like a chat's Thinking), the rest sent in order after the text
/// before it.
async fn pump(
    mut rx: mpsc::UnboundedReceiver<AgentEvent>,
    out: mpsc::UnboundedSender<ChatEvent>,
    node: String,
    companion: String,
    cards: Arc<Cards>,
    queue: mpsc::UnboundedSender<(u64, oneshot::Receiver<ApprovalAnswer>)>,
) -> Pumped {
    let (mut text, mut pending, mut pieces, mut at) = (String::new(), String::new(), 0usize, 0u64);
    let mut tools: Vec<String> = Vec::new();
    let (mut reasoning, mut reasoning_at) = (String::new(), 0u64);
    let mut thinking_since: Option<std::time::Instant> = None;
    let mut thinking_ms: Option<u64> = None;
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
                if let AgentEvent::ReasoningDelta(t) = &ev {
                    if t.is_empty() {
                        continue;
                    }
                    flush(&mut pending, &mut at);
                    pieces = 0;
                    thinking_since.get_or_insert_with(std::time::Instant::now);
                    reasoning.push_str(t);
                    let n = t.chars().count() as u64;
                    let _ = out.send(ChatEvent::Reasoning { offset: reasoning_at, text: t.clone() });
                    reasoning_at += n;
                    continue;
                }
                if let AgentEvent::TextDelta(t) = ev {
                    // The first answer text after reasoning ends that Thinking spell.
                    if let Some(since) = thinking_since.take() {
                        let ms = since.elapsed().as_millis() as u64;
                        thinking_ms = Some(thinking_ms.unwrap_or(0) + ms);
                        let _ = out.send(ChatEvent::Thought { elapsed_ms: thinking_ms.unwrap_or(ms) });
                    }
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
    if let Some(since) = thinking_since.take() {
        let ms = since.elapsed().as_millis() as u64;
        thinking_ms = Some(thinking_ms.unwrap_or(0) + ms);
        let _ = out.send(ChatEvent::Thought { elapsed_ms: thinking_ms.unwrap_or(ms) });
    }
    Pumped { text, tools, reasoning, thinking_ms }
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
        if crate::attach::needs(&attachments) != weirdnet_agent::Modalities::TEXT {
            return Err(CoreError::new(ErrorKind::BadRequest, "agent mode takes text and text files only for now; send images and audio in a chat"));
        }
        let node = match node.filter(|n| !n.is_empty()) {
            Some(n) => n,
            None => self
                .state()
                .catalog
                .nodes
                .first()
                .map(|n| n.id.clone())
                .ok_or_else(|| CoreError::new(ErrorKind::Unavailable, self.no_node_message()))?,
        };
        let companion = companion.filter(|c| !c.is_empty()).unwrap_or_else(|| DEFAULT_COMPANION.into());
        let label = node.clone();
        let name = companion.clone();
        self.spawn_agent(
            conversation,
            prompt,
            attachments,
            node,
            companion,
            Default::default(),
            None,
            move |input, messages, ev_tx, approver, cancel, _out| async move {
                let request = TaskRequest::new(AgentId::new(name.as_str()), input, messages);
                run_task(&*net.dialer, &label, request, Some(ev_tx), &approver, &cancel).await.map(|o| o.outcome.stop).map_err(|e| agent_error(&e))
            },
        )
    }

    /// Agent mode on an OpenAI-compatible endpoint (no Ymir node): Bragi runs the tool-call loop
    /// itself with the outside MCP servers that apply (enabled, in scope for `folder` and this
    /// conversation). Events are [`send_agent`](Self::send_agent)'s; every tool call asks first,
    /// and a server that won't connect is left out with a notice.
    pub fn send_agent_local(
        &self,
        conversation: Option<&str>,
        prompt: &str,
        endpoint_id: &str,
        model: Option<String>,
        folder: Option<String>,
        attachments: Vec<Attachment>,
    ) -> Result<ChatHandle> {
        if crate::attach::needs(&attachments) != weirdnet_agent::Modalities::TEXT {
            return Err(CoreError::new(ErrorKind::BadRequest, "agent mode takes text and text files only for now; send images and audio in a chat"));
        }
        let ep = self
            .inner
            .endpoints
            .get(endpoint_id)
            .filter(|e| e.enabled)
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "that endpoint isn't there or is turned off"))?;
        let target = self.inner.endpoints.target(&ep.id)?;
        let servers = self.inner.mcp.applying(folder.as_deref(), conversation);
        if servers.is_empty() {
            return Err(CoreError::new(ErrorKind::BadRequest, "no MCP server applies here; add one in Settings first"));
        }
        let always = self.inner.mcp.always_allowed();
        let core = self.clone();
        let label = ep.name.clone();
        let model = model.or(ep.model.clone());
        let on_always: Arc<dyn Fn(&str) + Send + Sync> = {
            let core = self.clone();
            Arc::new(move |tool: &str| {
                if let Some(s) = core.inner.mcp.list().into_iter().find(|s| tool.starts_with(&format!("{}.", crate::mcp_servers::slug(&s.name)))) {
                    let _ = core.inner.mcp.set_always_allow(&s.id, tool, true);
                }
            })
        };
        let companion = DEFAULT_COMPANION.to_string();
        self.spawn_agent(
            conversation,
            prompt,
            attachments,
            label.clone(),
            companion,
            always,
            Some(on_always),
            move |input, messages, ev_tx, approver, cancel, out| async move {
                let model = match model {
                    Some(m) => m,
                    None => crate::openai::list_models(&target)
                        .await
                        .ok()
                        .and_then(|m| m.into_iter().next())
                        .ok_or_else(|| CoreError::new(ErrorKind::BadRequest, format!("{label} has no model picked and lists none")))?,
                };
                let mut registry = weirdnet_agent::ToolRegistry::new();
                for s in &servers {
                    let notice = |text: String| {
                        let _ = out.send(ChatEvent::Notice { text });
                    };
                    match core.inner.mcp.connect(&core.inner.endpoints, &s.id).await {
                        Ok(source) => match source.client().list_tools().await {
                            Ok(_) => registry.add_source(Arc::new(source)),
                            Err(e) => notice(format!("{} left out: couldn't list its tools ({e:?})", s.name)),
                        },
                        Err(e) => notice(format!("{} left out: {}", s.name, e.message)),
                    }
                }
                registry.refresh().await.map_err(|e| CoreError::new(ErrorKind::Unavailable, e.to_string()))?;
                let identity = weirdnet_agent::AgentIdentity::new(DEFAULT_COMPANION, "Assistant", weirdnet_agent::AgentKind::Local);
                let mut config = weirdnet_agent::AgentConfig::new(identity);
                config.budget.max_steps = MAX_STEPS;
                let backend = Arc::new(crate::openai::EndpointBackend::new(target, model, label));
                let mut session = weirdnet_agent::AgentSession::new(
                    config,
                    backend,
                    Arc::new(registry),
                    weirdnet_agent::Policy::new().shared(),
                    Arc::new(approver),
                );
                session.restore_messages(messages);
                Ok(session.run(input, Some(ev_tx), cancel).await.stop)
            },
        )
    }

    /// The shared start of an agent task: records the turn, runs `run` (which streams events and
    /// asks cards through the approver it is given) and writes the answer.
    #[allow(clippy::too_many_arguments)]
    fn spawn_agent<F, Fut>(
        &self,
        conversation: Option<&str>,
        prompt: &str,
        attachments: Vec<Attachment>,
        node: String,
        companion: String,
        always: std::collections::HashSet<String>,
        on_always: Option<OnAlways>,
        run: F,
    ) -> Result<ChatHandle>
    where
        F: FnOnce(
                String,
                Vec<weirdnet_agent::Message>,
                mpsc::UnboundedSender<AgentEvent>,
                CardApprover,
                CancelToken,
                mpsc::UnboundedSender<ChatEvent>,
            ) -> Fut
            + Send
            + 'static,
        Fut: std::future::Future<Output = Result<StopReason>> + Send + 'static,
    {
        let (conv, mut messages) = self.begin_turn(conversation, prompt, &companion, attachments)?;
        let input = messages.pop().map(|m| m.content.into_text()).unwrap_or_default();
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
            let approver = CardApprover { queue: AsyncMutex::new(q_rx), out: out.clone(), cards: cards.clone(), always, on_always };
            let pump = tokio::spawn(pump(ev_rx, out.clone(), node.clone(), companion.clone(), cards, q_tx));
            let result = run(input, messages, ev_tx, approver, cancel, out.clone()).await;
            let Pumped { text, tools, reasoning, thinking_ms } = pump.await.unwrap_or_default();
            let (state, last) = match &result {
                Ok(StopReason::Completed) => (MessageState::Complete, ChatEvent::Done { finish_reason: Some("agent".into()) }),
                Ok(StopReason::Cancelled) => {
                    (MessageState::Stopped, ChatEvent::Error { kind: ErrorKind::Cancelled, message: "stopped".into(), partial: text.clone() })
                }
                Ok(other) => (
                    MessageState::Failed,
                    ChatEvent::Error { kind: ErrorKind::Other, message: format!("the task stopped early: {other:?}"), partial: text.clone() },
                ),
                Err(e) => {
                    let state = if e.kind == ErrorKind::Cancelled { MessageState::Stopped } else { MessageState::Failed };
                    (state, ChatEvent::Error { kind: e.kind, message: e.message.clone(), partial: text.clone() })
                }
            };
            core.finish_agent(&conv, Pumped { text, tools, reasoning, thinking_ms }, state, node, companion);
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
        if choice == Choice::Always {
            lock(&cards.always).insert(ask);
        }
        let answer = match choice {
            Choice::Deny => ApprovalAnswer::Deny,
            Choice::Once | Choice::Always => ApprovalAnswer::Approve { scope: Scope::Once },
            Choice::ThisTask => ApprovalAnswer::Approve { scope: Scope::Task },
        };
        tx.send(answer).is_ok()
    }

    fn finish_agent(&self, conversation: &str, p: Pumped, state: MessageState, node: String, companion: String) {
        let Ok(mut c) = self.inner.history.load(conversation) else { return };
        // The answer was deleted while the task ran: nothing to write.
        let Some(m) = c.messages.last_mut().filter(|m| m.role == crate::history::Role::Assistant && m.state == MessageState::Streaming) else {
            return;
        };
        {
            m.text = p.text;
            // The Thinking stays with the answer after it ends, folded, like a chat's.
            m.reasoning.push_str(&p.reasoning);
            if let Some(ms) = p.thinking_ms {
                m.thinking_ms = Some(m.thinking_ms.unwrap_or(0) + ms);
            }
            m.state = state;
            m.node = Some(node);
            m.model = Some(companion);
            m.tools = p.tools;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reasoning_in_a_task_is_streamed_timed_and_kept() {
        let (tx, rx) = mpsc::unbounded_channel();
        let (out, mut events) = mpsc::unbounded_channel();
        let (q, _q_rx) = mpsc::unbounded_channel();
        tx.send(AgentEvent::ReasoningDelta("let me ".into())).unwrap();
        tx.send(AgentEvent::ReasoningDelta("think".into())).unwrap();
        tx.send(AgentEvent::TextDelta("391".into())).unwrap();
        drop(tx);
        let p = pump(rx, out, "node".into(), "assistant".into(), Arc::new(Cards::default()), q).await;
        assert_eq!(p.text, "391");
        assert_eq!(p.reasoning, "let me think");
        assert!(p.thinking_ms.is_some());
        let mut seen = Vec::new();
        while let Ok(e) = events.try_recv() {
            seen.push(e);
        }
        assert!(matches!(&seen[0], ChatEvent::Reasoning { offset: 0, text } if text == "let me "), "{seen:?}");
        assert!(matches!(&seen[1], ChatEvent::Reasoning { offset: 7, text } if text == "think"), "{seen:?}");
        assert!(matches!(&seen[2], ChatEvent::Thought { .. }), "{seen:?}");
        assert!(seen.iter().any(|e| matches!(e, ChatEvent::Delta { text, .. } if text == "391")), "{seen:?}");
    }
}
