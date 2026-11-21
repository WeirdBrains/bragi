//! Everything the Flutter app calls. Each function maps onto one `bragi_core::Core` call;
//! the types here are flat copies of the core's, shaped for Dart.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use bragi_core::demo::DemoNode;
use bragi_core::{
    AppState, Attachment, ChatEvent, ChatHandle, Choice, Config, Conversation, ConversationSummary, Core, ErrorKind, Media, MessageState,
    ModelStatus, NodeSearch, PairState, Role, ToolState,
};
use flutter_rust_bridge::frb;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::frb_generated::StreamSink;

struct Running {
    core: Core,
    _demo: Option<Arc<DemoNode>>,
    _runtime: Option<tokio::runtime::Runtime>,
}

fn slot() -> &'static Mutex<Option<Running>> {
    static S: OnceLock<Mutex<Option<Running>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

/// Answers waiting for `follow_chat`, by request id.
fn pending() -> &'static Mutex<HashMap<String, UnboundedReceiver<ChatEvent>>> {
    static P: OnceLock<Mutex<HashMap<String, UnboundedReceiver<ChatEvent>>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

fn core() -> Result<Core, String> {
    slot().lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|r| r.core.clone()).ok_or_else(|| "Bragi has not started".to_string())
}

fn err(e: bragi_core::CoreError) -> String {
    e.message
}

// ---- types -------------------------------------------------------------------------------------

/// [`PairState`] flattened: `state` is `unpaired`, `offering`, `confirm`, `waiting`, `linked`
/// or `failed`; the other fields are set when that state has them.
#[derive(Debug, Clone, Default)]
pub struct PairView {
    pub state: String,
    pub link: Option<String>,
    pub expires_ms: Option<i64>,
    pub sas: Option<String>,
    pub user: Option<String>,
    pub device: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ModelView {
    pub id: String,
    /// `warm`, `loadable`, `loading` or `busy`.
    pub status: String,
    pub replicas: u32,
    pub tokens_per_sec: Option<f64>,
    pub context: Option<u32>,
    /// Inference nodes that serve it.
    pub nodes: u32,
    /// The model that answers while this one loads, if a node names one.
    pub standin: Option<String>,
    /// Whether it reads images, and whether it reads audio.
    pub vision: bool,
    pub audio: bool,
}

#[derive(Debug, Clone)]
pub struct StateView {
    pub pair: PairView,
    pub online: bool,
    /// Inference nodes found among the user's devices.
    pub node_count: u32,
    pub models: Vec<ModelView>,
    pub default_model: Option<String>,
    /// The last node address added in Link a device: `idle`, `looking`, `found` or `not_found`,
    /// and the line to show (none when idle). `found` is success, `not_found` a warning.
    pub node_search: String,
    pub node_search_line: Option<String>,
}

/// One [`ChatEvent`]: `kind` is `queued`, `started`, `delta`, `reasoning`, `thought`, `resumed`, `usage`,
/// `done`, `error`, `retry` or `meta`. A delta's text goes at `offset` (characters), replacing what was there.
#[derive(Debug, Clone, Default)]
pub struct ChatEventView {
    pub kind: String,
    pub offset: u64,
    pub text: String,
    pub node: Option<String>,
    pub model: Option<String>,
    pub position: u32,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub finish_reason: Option<String>,
    /// For errors: `cancelled`, `unavailable`, `busy`, `refused`, … (`bragi_core::ErrorKind`).
    pub error_kind: Option<String>,
    /// For errors: the text that streamed before it.
    pub partial: String,
    /// `meta`: served from the cache.
    pub cache_hit: bool,
    /// `meta`: the stand-in that answers.
    pub standin: Option<String>,
    /// `retry`: which try this is.
    pub attempt: u32,
    /// `tool`: the call, the tool and `started`, `done` or `failed`. `approval`: the tool too.
    pub call_id: String,
    pub tool: Option<String>,
    pub tool_state: Option<String>,
    /// `approval`: the card's id (answer with `approve`), its tier and site. The summary is in `text`.
    pub ask: u64,
    pub tier: Option<String>,
    pub site: Option<String>,
    /// `thought`: how long the model reasoned before answering, in milliseconds. A `reasoning`
    /// event's text goes at `offset` in the reasoning, like a delta's in the answer.
    pub elapsed_ms: u64,
}

/// A file attached to a message: `kind` is `text`, `image` or `audio`. Text is in `text`; for
/// an image or sound `mime` is the image's MIME type or the audio's format (`wav`, `mp3`) and
/// `data` the base64 file. A message read back from history has `data` empty (its size is
/// not worth carrying to the screen; the name and kind are shown).
#[derive(Debug, Clone)]
pub struct AttachmentView {
    pub name: String,
    pub text: String,
    pub kind: String,
    pub mime: String,
    pub data: String,
}

#[derive(Debug, Clone)]
pub struct ChatStarted {
    pub request_id: String,
    pub conversation_id: String,
}

#[derive(Debug, Clone)]
pub struct MessageView {
    /// The message's id in the store (what `delete_message` takes).
    pub id: String,
    /// A user message followed by its answer: deleting it offers to delete the answer too.
    pub has_reply: bool,
    /// Emoji the user reacted with, in the order added.
    pub reactions: Vec<String>,
    /// `user` or `assistant`.
    pub role: String,
    pub text: String,
    /// The model's reasoning, shown in the collapsible Thinking section; empty when it didn't.
    pub reasoning: String,
    /// How long it reasoned, in milliseconds.
    pub thinking_ms: Option<u64>,
    /// `complete`, `streaming`, `stopped` or `failed`.
    pub state: String,
    pub model: Option<String>,
    pub node: Option<String>,
    pub cache_hit: bool,
    pub standin: Option<String>,
    /// Times the answer moved to another node mid-stream.
    pub resumes: u32,
    pub attachments: Vec<AttachmentView>,
    /// Agent mode: the tools the task used, in order.
    pub tools: Vec<String>,
    /// The answer came from an agent task on a node.
    pub agent: bool,
    pub at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct ConversationView {
    pub id: String,
    pub title: String,
    pub updated_ms: i64,
    pub messages: Vec<MessageView>,
    /// The last answer was stopped or cut off and can be continued.
    pub continuable: bool,
}

#[derive(Debug, Clone)]
pub struct SummaryView {
    pub id: String,
    pub title: String,
    pub updated_ms: i64,
}

/// What a delete removed; `undo` undoes it within `undo_ms`.
#[derive(Debug, Clone)]
pub struct DeletedView {
    pub undo: String,
    pub conversations: u32,
    pub messages: u32,
    pub undo_ms: u32,
}

#[derive(Debug, Clone)]
pub struct LinkCheckView {
    pub sas: String,
    pub device: String,
}

fn status_name(s: ModelStatus) -> String {
    match s {
        ModelStatus::Warm => "warm",
        ModelStatus::Loadable => "loadable",
        ModelStatus::Loading => "loading",
        ModelStatus::Busy => "busy",
    }
    .into()
}

fn message_state_name(s: MessageState) -> String {
    match s {
        MessageState::Complete => "complete",
        MessageState::Streaming => "streaming",
        MessageState::Stopped => "stopped",
        MessageState::Failed => "failed",
    }
    .into()
}

fn error_kind_name(k: ErrorKind) -> String {
    match k {
        ErrorKind::NotPaired => "not_paired",
        ErrorKind::Pairing => "pairing",
        ErrorKind::Unavailable => "unavailable",
        ErrorKind::Busy => "busy",
        ErrorKind::Cancelled => "cancelled",
        ErrorKind::Refused => "refused",
        ErrorKind::BadRequest => "bad_request",
        ErrorKind::Storage => "storage",
        ErrorKind::NotFound => "not_found",
        ErrorKind::Other => "other",
    }
    .into()
}

fn pair_view(p: &PairState) -> PairView {
    let mut v = PairView::default();
    match p {
        PairState::Unpaired => v.state = "unpaired".into(),
        PairState::Offering { link, expires_ms } => {
            v.state = "offering".into();
            v.link = Some(link.clone());
            v.expires_ms = Some(*expires_ms);
        }
        PairState::Confirm { sas } => {
            v.state = "confirm".into();
            v.sas = Some(sas.clone());
        }
        PairState::Waiting { sas } => {
            v.state = "waiting".into();
            v.sas = Some(sas.clone());
        }
        PairState::Linked { user, device } => {
            v.state = "linked".into();
            v.user = Some(user.clone());
            v.device = Some(device.clone());
        }
        PairState::Failed { message } => {
            v.state = "failed".into();
            v.message = Some(message.clone());
        }
    }
    v
}

fn state_view(s: &AppState) -> StateView {
    StateView {
        pair: pair_view(&s.pair),
        online: s.online,
        node_count: s.catalog.nodes.len() as u32,
        models: s
            .catalog
            .models
            .iter()
            .map(|m| ModelView {
                id: m.id.clone(),
                status: status_name(m.status),
                replicas: m.replicas,
                tokens_per_sec: m.tokens_per_sec.map(f64::from),
                context: m.context,
                nodes: m.nodes.len() as u32,
                standin: m.standin.clone(),
                vision: m.vision,
                audio: m.audio,
            })
            .collect(),
        default_model: s.catalog.default_model().map(str::to_string),
        node_search: match s.node_search {
            NodeSearch::Idle => "idle",
            NodeSearch::Looking { .. } => "looking",
            NodeSearch::Found { .. } => "found",
            NodeSearch::NotFound { .. } => "not_found",
        }
        .into(),
        node_search_line: s.node_search.line(),
    }
}

fn event_view(e: ChatEvent) -> ChatEventView {
    let mut v = ChatEventView::default();
    match e {
        ChatEvent::Queued { position } => {
            v.kind = "queued".into();
            v.position = position;
        }
        ChatEvent::Started { node, model, .. } => {
            v.kind = "started".into();
            v.node = Some(node);
            v.model = Some(model);
        }
        ChatEvent::Delta { offset, text } => {
            v.kind = "delta".into();
            v.offset = offset;
            v.text = text;
        }
        ChatEvent::Reasoning { offset, text } => {
            v.kind = "reasoning".into();
            v.offset = offset;
            v.text = text;
        }
        ChatEvent::Thought { elapsed_ms } => {
            v.kind = "thought".into();
            v.elapsed_ms = elapsed_ms;
        }
        ChatEvent::Retry { attempt, message } => {
            v.kind = "retry".into();
            v.attempt = attempt;
            v.text = message;
        }
        ChatEvent::Meta { cache_hit, standin, served_by } => {
            v.kind = "meta".into();
            v.cache_hit = cache_hit;
            v.standin = standin;
            v.node = served_by;
        }
        ChatEvent::Resumed { node, offset } => {
            v.kind = "resumed".into();
            v.node = Some(node);
            v.offset = offset;
        }
        ChatEvent::Usage { prompt_tokens, completion_tokens } => {
            v.kind = "usage".into();
            v.prompt_tokens = prompt_tokens;
            v.completion_tokens = completion_tokens;
        }
        ChatEvent::Done { finish_reason } => {
            v.kind = "done".into();
            v.finish_reason = finish_reason;
        }
        ChatEvent::Tool { call_id, tool, state } => {
            v.kind = "tool".into();
            v.call_id = call_id;
            v.tool = Some(tool);
            v.tool_state = Some(
                match state {
                    ToolState::Started => "started",
                    ToolState::Done => "done",
                    ToolState::Failed => "failed",
                }
                .into(),
            );
        }
        ChatEvent::Approval { ask, tool, tier, site, summary } => {
            v.kind = "approval".into();
            v.ask = ask;
            v.tool = Some(tool);
            v.tier = Some(tier);
            v.site = site;
            v.text = summary;
        }
        ChatEvent::Notice { text } => {
            v.kind = "notice".into();
            v.text = text;
        }
        ChatEvent::Error { kind, message, partial } => {
            v.kind = "error".into();
            v.error_kind = Some(error_kind_name(kind));
            v.text = message;
            v.partial = partial;
        }
    }
    v
}

fn conversation_view(c: &Conversation) -> ConversationView {
    ConversationView {
        id: c.id.clone(),
        title: c.title.clone(),
        updated_ms: c.updated_ms,
        continuable: c.continuable(),
        messages: c
            .messages
            .iter()
            .map(|m| MessageView {
                id: m.id.clone(),
                has_reply: c.reply_to(&m.id).is_some(),
                reactions: m.reactions.clone(),
                role: if m.role == Role::User { "user" } else { "assistant" }.into(),
                text: m.text.clone(),
                reasoning: m.reasoning.clone(),
                thinking_ms: m.thinking_ms,
                state: message_state_name(m.state),
                model: m.model.clone(),
                node: m.node.clone(),
                cache_hit: m.cache_hit,
                standin: m.standin.clone(),
                resumes: m.resumes,
                attachments: m.attachments.iter().map(|a| attachment_view(a, false)).collect(),
                tools: m.tools.clone(),
                agent: m.agent,
                at_ms: m.at_ms,
            })
            .collect(),
    }
}

fn attachment_view(a: &Attachment, with_data: bool) -> AttachmentView {
    let (kind, mime, data) = match &a.media {
        None => ("text", String::new(), String::new()),
        Some(Media::Image { mime, data }) => ("image", mime.clone(), data.clone()),
        Some(Media::Audio { format, data }) => ("audio", format.clone(), data.clone()),
    };
    AttachmentView { name: a.name.clone(), text: a.text.clone(), kind: kind.into(), mime, data: if with_data { data } else { String::new() } }
}

fn attachments(v: Vec<AttachmentView>) -> Result<Vec<Attachment>, String> {
    v.into_iter()
        .map(|a| match a.kind.as_str() {
            "image" | "audio" => Attachment::from_media(&a.name, &a.kind, &a.mime, &a.data).map_err(err),
            _ => Ok(Attachment { name: a.name, text: a.text, media: None }),
        })
        .collect()
}

fn summary_view(s: &ConversationSummary) -> SummaryView {
    SummaryView { id: s.id.clone(), title: s.title.clone(), updated_ms: s.updated_ms }
}

fn started(h: ChatHandle) -> ChatStarted {
    let out = ChatStarted { request_id: h.request_id.clone(), conversation_id: h.conversation_id.clone() };
    pending().lock().unwrap_or_else(|e| e.into_inner()).insert(h.request_id, h.events);
    out
}

// ---- lifecycle ---------------------------------------------------------------------------------

#[frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}

/// Starts Bragi on the real network with its data in `dir`. Starting twice is a no-op.
pub fn start(dir: String, label: String) -> Result<(), String> {
    let mut s = slot().lock().unwrap_or_else(|e| e.into_inner());
    if s.is_some() {
        return Ok(());
    }
    let core = Core::start(Config::new(PathBuf::from(dir), label)).map_err(err)?;
    *s = Some(Running { core, _demo: None, _runtime: None });
    Ok(())
}

/// Starts the demo: an in-process inference node with a test identity, which opens the
/// link code after `accept_after_ms`. Nothing touches the real network.
pub fn start_demo(dir: String, label: String, accept_after_ms: u32, word_delay_ms: u32) -> Result<(), String> {
    let mut s = slot().lock().unwrap_or_else(|e| e.into_inner());
    if s.is_some() {
        return Ok(());
    }
    let dir = PathBuf::from(dir);
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e| e.to_string())?;
    // Two inference nodes of one user, so the pool has something to merge and fail over.
    let node = rt.block_on(DemoNode::start_cloud(&dir.join("demo"), Duration::from_millis(word_delay_ms.into()))).map_err(err)?;
    let core = {
        let _g = rt.enter();
        Core::start(node.app_config(&dir.join("app"), &label)).map_err(err)?
    };
    node.auto_accept_after(&core, Duration::from_millis(accept_after_ms.into()));
    // The demo's other laptop: a conversation that mentions it makes it start one, which then
    // syncs over (the Android scenario shows history from another device this way).
    {
        let (node, core) = (node.clone(), core.clone());
        rt.spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let asked = core
                    .conversations(0, 20)
                    .unwrap_or_default()
                    .iter()
                    .any(|c| c.title.to_lowercase().contains(bragi_core::demo::OTHER_DEVICE_TRIGGER));
                if asked {
                    if let Err(e) = node.play_other_device(&core).await {
                        tracing::warn!("the demo's other laptop: {e}");
                    }
                    return;
                }
            }
        });
    }
    *s = Some(Running { core, _demo: Some(node), _runtime: Some(rt) });
    Ok(())
}

/// Demo only: the next answer's node dies after `words` words, and the other node finishes it.
pub fn demo_fail_next_answer_after(words: u32) -> Result<(), String> {
    let s = slot().lock().unwrap_or_else(|e| e.into_inner());
    let demo = s.as_ref().and_then(|r| r._demo.clone()).ok_or("not the demo profile")?;
    demo.faults.die_next_answer_after(words as usize);
    Ok(())
}

/// Where the demo's first inference node can be dialed (`/memory/…/p2p/…`), for a test that
/// pastes a node address; none outside the demo.
#[frb(sync)]
pub fn demo_node_address() -> Option<String> {
    slot().lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|r| r._demo.as_ref().map(|d| d.address()))
}

/// The demo node's answer to `prompt`, which a test compares a continued answer with.
pub fn demo_answer(model: String, prompt: String) -> String {
    bragi_core::demo::answer_for(&model, &prompt)
}

pub fn shutdown() {
    if let Some(r) = slot().lock().unwrap_or_else(|e| e.into_inner()).take() {
        r.core.shutdown();
    }
}

pub fn paused() -> Result<(), String> {
    core()?.paused();
    Ok(())
}

pub fn resumed() -> Result<(), String> {
    core()?.resumed();
    Ok(())
}

pub fn network_changed() -> Result<(), String> {
    core()?.network_changed();
    Ok(())
}

// ---- state -------------------------------------------------------------------------------------

#[frb(sync)]
pub fn state() -> Result<StateView, String> {
    Ok(state_view(&core()?.state()))
}

/// Sends the state now and on every change, until the app stops listening.
pub fn watch_state(sink: StreamSink<StateView>) -> Result<(), String> {
    let core = core()?;
    let mut w = core.watch();
    core.handle().spawn(async move {
        loop {
            let v = state_view(&w.borrow_and_update());
            if sink.add(v).is_err() || w.changed().await.is_err() {
                return;
            }
        }
    });
    Ok(())
}

// ---- pairing -----------------------------------------------------------------------------------

/// Shows a link code for a device that is already linked; returns it (also in the state).
pub async fn pair_offer() -> Result<String, String> {
    let core = core()?;
    let c = core.clone();
    core.handle().spawn(async move { c.pair_offer().await }).await.map_err(|e| e.to_string())?.map_err(err)
}

pub fn pair_confirm(matches: bool) -> Result<(), String> {
    core()?.pair_confirm(matches).map_err(err)
}

pub fn pair_cancel() -> Result<(), String> {
    core()?.pair_cancel();
    Ok(())
}

/// Opens another device's code from this (linked) one; compare the returned check code.
pub async fn pair_accept(link: String) -> Result<LinkCheckView, String> {
    let core = core()?;
    let c = core.clone();
    let check = core.handle().spawn(async move { c.pair_accept(&link).await }).await.map_err(|e| e.to_string())?.map_err(err)?;
    Ok(LinkCheckView { sas: check.sas, device: check.device })
}

/// The Thinking section's title (`bragi_core::think::label`): `Thinking… 4s` while the model
/// reasons, `Thought for 4s` once `done`.
#[frb(sync)]
pub fn thinking_label(elapsed_ms: u64, done: bool) -> String {
    bragi_core::think::label(elapsed_ms, done)
}

/// Whether text from the Link a device box is a node address (`/ip4/…/p2p/…`) rather than a
/// link code: then call `add_node` instead of `pair_accept`.
#[frb(sync)]
pub fn is_node_address(text: String) -> bool {
    bragi_core::is_node_address(&text)
}

/// Adds a node address pasted into Link a device: kept with the app's data and dialed at once,
/// no restart. Returns the line to show.
#[frb(sync)]
pub fn add_node(address: String) -> Result<String, String> {
    core()?.add_node(&address).map_err(err)
}

/// The user did something else: the node search's line goes away.
#[frb(sync)]
pub fn dismiss_node_search() {
    if let Ok(c) = core() {
        c.dismiss_node_search();
    }
}

/// How long `add_node` looks for the node before saying what to check (tests shorten it).
#[frb(sync)]
pub fn set_node_search_limit_ms(ms: u32) {
    if let Ok(c) = core() {
        c.set_node_search_limit(Duration::from_millis(ms.into()));
    }
}

/// The node addresses this device dials (its links' and the ones added).
#[frb(sync)]
pub fn peers() -> Vec<String> {
    core().map(|c| c.peers()).unwrap_or_default()
}

pub async fn pair_accept_confirm(matches: bool) -> Result<(), String> {
    let core = core()?;
    let c = core.clone();
    core.handle().spawn(async move { c.pair_accept_confirm(matches).await }).await.map_err(|e| e.to_string())?.map_err(err)
}

// ---- chat --------------------------------------------------------------------------------------

/// Asks `prompt` in `conversation` (a new one when none); stream it with `follow_chat`.
pub fn send(conversation: Option<String>, prompt: String, model: Option<String>) -> Result<ChatStarted, String> {
    core()?.send(conversation.as_deref(), &prompt, model).map(started).map_err(err)
}

/// Reads a file as an attachment (`path` on this device): text, or a PNG, JPEG, GIF or WebP image,
/// or WAV or MP3 audio. Other binaries and big files are refused.
#[frb(sync)]
pub fn attach_file(path: String) -> Result<AttachmentView, String> {
    Attachment::from_path(std::path::Path::new(&path)).map(|a| attachment_view(&a, true)).map_err(err)
}

/// An attachment from bytes the picker returned (`name` is shown; the bytes are UTF-8 text, an
/// image or audio, as for [`attach_file`]).
#[frb(sync)]
pub fn attach_bytes(name: String, bytes: Vec<u8>) -> Result<AttachmentView, String> {
    Attachment::from_bytes(&name, &bytes).map(|a| attachment_view(&a, true)).map_err(err)
}

/// A reply's Markdown as JSON (`bragi_core::markdown::Document`): the same parse the desktop
/// and the terminal UI draw. `streaming` is for an answer still arriving (no half-typed markers;
/// an open fence is code to the end). Raw HTML stays text and images are links, never fetched.
#[frb(sync)]
pub fn markdown_json(text: String, streaming: bool) -> String {
    if streaming { bragi_core::markdown::parse_streaming(&text) } else { bragi_core::markdown::parse(&text) }.to_json()
}

/// [`send`] with files attached (text, images, audio).
pub fn send_with(conversation: Option<String>, prompt: String, model: Option<String>, files: Vec<AttachmentView>) -> Result<ChatStarted, String> {
    core()?.send_with(conversation.as_deref(), &prompt, model, attachments(files)?).map(started).map_err(err)
}

/// Agent mode: runs `prompt` as a task on `node` (the first inference node when none); stream it
/// with `follow_chat`, which also carries `tool`, `approval` and `notice` events.
pub fn send_agent(conversation: Option<String>, prompt: String, node: Option<String>, files: Vec<AttachmentView>) -> Result<ChatStarted, String> {
    core()?.send_agent(conversation.as_deref(), &prompt, node, None, attachments(files)?).map(started).map_err(err)
}

/// Answers an agent task's approval card: `choice` is `deny`, `once` or `task`. False when the
/// card isn't waiting.
#[frb(sync)]
pub fn approve(request_id: String, ask: u64, choice: String) -> bool {
    let choice = match choice.as_str() {
        "once" => Choice::Once,
        "task" => Choice::ThisTask,
        _ => Choice::Deny,
    };
    core().map(|c| c.approve(&request_id, ask, choice)).unwrap_or(false)
}

/// Continues the stopped or cut-off last answer in `conversation`.
pub fn continue_answer(conversation: String, model: Option<String>) -> Result<ChatStarted, String> {
    core()?.continue_answer(&conversation, model).map(started).map_err(err)
}

/// Streams a started answer's events, ending after `done` or `error`.
pub fn follow_chat(request_id: String, sink: StreamSink<ChatEventView>) -> Result<(), String> {
    let core = core()?;
    let mut rx = pending().lock().unwrap_or_else(|e| e.into_inner()).remove(&request_id).ok_or_else(|| "no such answer".to_string())?;
    core.handle().spawn(async move {
        while let Some(e) = rx.recv().await {
            let end = matches!(e, ChatEvent::Done { .. } | ChatEvent::Error { .. });
            if sink.add(event_view(e)).is_err() || end {
                return;
            }
        }
    });
    Ok(())
}

/// Stops an answer; what streamed so far is kept.
#[frb(sync)]
pub fn cancel(request_id: String) -> bool {
    core().map(|c| c.cancel(&request_id)).unwrap_or(false)
}

// ---- history -----------------------------------------------------------------------------------

pub fn conversations(offset: u32, limit: u32) -> Result<Vec<SummaryView>, String> {
    Ok(core()?.conversations(offset as usize, limit as usize).map_err(err)?.iter().map(summary_view).collect())
}

pub fn conversation(id: String) -> Result<ConversationView, String> {
    core()?.conversation(&id).map(|c| conversation_view(&c)).map_err(err)
}

/// The quick reaction bar, in order.
#[flutter_rust_bridge::frb(sync)]
pub fn quick_reactions() -> Vec<String> {
    bragi_core::QUICK_REACTIONS.iter().map(|s| s.to_string()).collect()
}

/// Toggles a reaction on a message; returns the message's reactions afterwards.
pub fn react(conversation: String, message: String, emoji: String) -> Result<Vec<String>, String> {
    core()?.react(&conversation, &message, &emoji).map_err(err)
}

pub fn delete_conversation(id: String) -> Result<(), String> {
    core()?.delete_conversation(&id).map_err(err)
}

fn deleted_view(d: bragi_core::Deleted) -> DeletedView {
    DeletedView { undo: d.undo, conversations: d.conversations as u32, messages: d.messages as u32, undo_ms: bragi_core::UNDO_MS as u32 }
}

/// Deletes conversations (on every linked device, and from this one's store).
pub fn delete_conversations(ids: Vec<String>) -> Result<DeletedView, String> {
    core()?.delete_conversations(&ids).map(deleted_view).map_err(err)
}

/// Deletes one message, and with `with_reply` the answer that followed it.
pub fn delete_message(conversation: String, id: String, with_reply: bool) -> Result<DeletedView, String> {
    core()?.delete_message(&conversation, &id, with_reply).map(deleted_view).map_err(err)
}

pub fn undo_delete(undo: String) -> Result<(), String> {
    core()?.undo_delete(&undo).map_err(err)
}

/// Deletes all history, with no undo. Returns how many conversations.
pub fn clear_history() -> Result<u32, String> {
    core()?.clear_history().map(|n| n as u32).map_err(err)
}

// ---- settings: endpoints, links, targets, sessions, folders (LF-11) ------------------------------

/// An OpenAI-compatible endpoint as Settings draws it. `mode` is `fallback` or `alongside`.
/// The API key is never here, only whether one is set.
#[derive(Debug, Clone)]
pub struct EndpointView {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: Option<String>,
    pub mode: String,
    pub enabled: bool,
    pub has_key: bool,
    pub insecure: bool,
    /// Sessions bound to it (for the delete confirmation).
    pub sessions: u32,
}

/// What the add/edit form sends. `key_change` is `keep`, `set` (with `api_key`) or `clear`.
#[derive(Debug, Clone)]
pub struct EndpointForm {
    pub name: String,
    pub base_url: String,
    pub model: Option<String>,
    pub mode: String,
    pub enabled: bool,
    pub api_key: Option<String>,
    pub key_change: String,
}

#[derive(Debug, Clone)]
pub struct TestResultView {
    pub ok: bool,
    pub models: Vec<String>,
    pub latency_ms: u64,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct LinkView {
    pub address: String,
    pub peer: String,
    pub online: bool,
}

/// A target for a session: `kind` is `pool` or `endpoint` (with `endpoint` and `model`).
#[derive(Debug, Clone)]
pub struct TargetView {
    pub kind: String,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct FolderView {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SessionView {
    pub id: String,
    pub title: String,
    pub updated_ms: i64,
    pub folder: Option<String>,
    pub target: Option<TargetView>,
    pub target_label: String,
    pub unbound: bool,
}

#[derive(Debug, Clone)]
pub struct DeleteReportView {
    pub rebound: u32,
    pub unbound: u32,
}

fn mode_of(s: &str) -> Result<bragi_core::EndpointMode, String> {
    match s {
        "fallback" => Ok(bragi_core::EndpointMode::Fallback),
        "alongside" => Ok(bragi_core::EndpointMode::Alongside),
        other => Err(format!("unknown mode `{other}`")),
    }
}

fn mode_str(m: bragi_core::EndpointMode) -> String {
    match m {
        bragi_core::EndpointMode::Fallback => "fallback".into(),
        bragi_core::EndpointMode::Alongside => "alongside".into(),
    }
}

fn endpoint_view(core: &Core, e: bragi_core::EndpointView) -> EndpointView {
    EndpointView {
        sessions: core.endpoint_sessions(&e.id) as u32,
        id: e.id,
        name: e.name,
        base_url: e.base_url,
        model: e.model,
        mode: mode_str(e.mode),
        enabled: e.enabled,
        has_key: e.has_key,
        insecure: e.insecure,
    }
}

fn form(f: EndpointForm) -> Result<(bragi_core::EndpointInput, bragi_core::KeyChange, Option<String>), String> {
    let key = match f.key_change.as_str() {
        "keep" => bragi_core::KeyChange::Keep,
        "clear" => bragi_core::KeyChange::Clear,
        "set" => bragi_core::KeyChange::Set(f.api_key.clone().unwrap_or_default()),
        other => return Err(format!("unknown key change `{other}`")),
    };
    let input = bragi_core::EndpointInput { name: f.name, base_url: f.base_url, model: f.model, mode: mode_of(&f.mode)?, enabled: f.enabled };
    Ok((input, key, f.api_key))
}

fn target_of(t: &TargetView) -> Result<bragi_core::Target, String> {
    match t.kind.as_str() {
        "pool" => Ok(bragi_core::Target::Pool),
        "endpoint" => Ok(bragi_core::Target::Endpoint {
            id: t.endpoint.clone().ok_or("an endpoint target needs an endpoint")?,
            model: t.model.clone().unwrap_or_default(),
        }),
        other => Err(format!("unknown target `{other}`")),
    }
}

fn target_view(t: &bragi_core::Target, label: String) -> TargetView {
    match t {
        bragi_core::Target::Pool => TargetView { kind: "pool".into(), endpoint: None, model: None, label },
        bragi_core::Target::Endpoint { id, model } => {
            TargetView { kind: "endpoint".into(), endpoint: Some(id.clone()), model: Some(model.clone()), label }
        }
    }
}

pub fn endpoints() -> Result<Vec<EndpointView>, String> {
    let core = core()?;
    Ok(core.endpoints().into_iter().map(|e| endpoint_view(&core, e)).collect())
}

pub fn add_endpoint(form_in: EndpointForm) -> Result<EndpointView, String> {
    let core = core()?;
    let (input, _, key) = form(form_in)?;
    core.add_endpoint(input, key).map(|e| endpoint_view(&core, e)).map_err(err)
}

pub fn edit_endpoint(id: String, form_in: EndpointForm) -> Result<EndpointView, String> {
    let core = core()?;
    let (input, key, _) = form(form_in)?;
    core.edit_endpoint(&id, input, key).map(|e| endpoint_view(&core, e)).map_err(err)
}

/// Deletes an endpoint. `rebind_to` is where its sessions go; `None` marks them unbound. No history is deleted.
pub fn delete_endpoint(id: String, rebind_to: Option<TargetView>) -> Result<DeleteReportView, String> {
    let to = rebind_to.as_ref().map(target_of).transpose()?;
    core()?.delete_endpoint(&id, to).map(|r| DeleteReportView { rebound: r.rebound as u32, unbound: r.unbound as u32 }).map_err(err)
}

pub fn move_endpoint(id: String, index: u32) -> Result<(), String> {
    core()?.move_endpoint(&id, index as usize).map_err(err)
}

fn test_view(r: bragi_core::TestReport) -> TestResultView {
    TestResultView { ok: r.ok, models: r.models, latency_ms: r.latency_ms, message: r.message }
}

pub async fn test_endpoint(id: String) -> Result<TestResultView, String> {
    let core = core()?;
    let c = core.clone();
    core.handle().spawn(async move { c.test_endpoint(&id).await }).await.map_err(|e| e.to_string())?.map(test_view).map_err(err)
}

/// Tests an address and key that aren't saved yet.
pub async fn test_endpoint_address(base_url: String, api_key: Option<String>) -> Result<TestResultView, String> {
    let core = core()?;
    let c = core.clone();
    core.handle().spawn(async move { c.test_endpoint_address(&base_url, api_key).await }).await.map_err(|e| e.to_string())?.map(test_view).map_err(err)
}

pub async fn endpoint_models(id: String) -> Result<Vec<String>, String> {
    let core = core()?;
    let c = core.clone();
    core.handle().spawn(async move { c.endpoint_models(&id).await }).await.map_err(|e| e.to_string())?.map_err(err)
}

pub fn links() -> Vec<LinkView> {
    core().map(|c| c.links().into_iter().map(|l| LinkView { address: l.address, peer: l.peer, online: l.online }).collect()).unwrap_or_default()
}

pub fn edit_link(old: String, new: String) -> Result<String, String> {
    core()?.edit_link(&old, &new).map_err(err)
}

pub fn remove_link(address: String) -> Result<(), String> {
    core()?.remove_link(&address).map_err(err)
}

/// What a new session can be bound to: the pool, and endpoints offered alongside it.
pub fn targets() -> Vec<TargetView> {
    core().map(|c| c.targets().into_iter().map(|t| target_view(&t.target, t.label)).collect()).unwrap_or_default()
}

/// [`send_with`] to a target (`None`: the session's own, or the default for a new one).
pub fn send_to(
    conversation: Option<String>,
    prompt: String,
    model: Option<String>,
    files: Vec<AttachmentView>,
    target: Option<TargetView>,
) -> Result<ChatStarted, String> {
    let target = target.as_ref().map(target_of).transpose()?;
    let attachments = attachments(files)?;
    core()?.send_to(conversation.as_deref(), &prompt, model, attachments, target).map(started).map_err(err)
}

pub fn sessions(offset: u32, limit: u32) -> Result<Vec<SessionView>, String> {
    Ok(core()?
        .sessions(offset as usize, limit as usize)
        .map_err(err)?
        .into_iter()
        .map(|s| SessionView {
            target: s.target.as_ref().map(|t| target_view(t, s.target_label.clone())),
            id: s.id,
            title: s.title,
            updated_ms: s.updated_ms,
            folder: s.folder,
            target_label: s.target_label,
            unbound: s.unbound,
        })
        .collect())
}

pub fn bind_session(conversation: String, target: TargetView) -> Result<(), String> {
    core()?.bind_session(&conversation, target_of(&target)?).map_err(err)
}

pub fn folders() -> Vec<FolderView> {
    core().map(|c| c.folders().into_iter().map(|f| FolderView { id: f.id, name: f.name, parent: f.parent }).collect()).unwrap_or_default()
}

pub fn create_folder(name: String, parent: Option<String>) -> Result<FolderView, String> {
    core()?.create_folder(&name, parent.as_deref()).map(|f| FolderView { id: f.id, name: f.name, parent: f.parent }).map_err(err)
}

pub fn rename_folder(id: String, name: String) -> Result<(), String> {
    core()?.rename_folder(&id, &name).map_err(err)
}

pub fn move_folder(id: String, parent: Option<String>) -> Result<(), String> {
    core()?.move_folder(&id, parent.as_deref()).map_err(err)
}

/// Deletes a folder; its sessions and subfolders move up a level. No history is deleted.
pub fn delete_folder(id: String) -> Result<(), String> {
    core()?.delete_folder(&id).map_err(err)
}

pub fn set_session_folder(conversation: String, folder: Option<String>) -> Result<(), String> {
    core()?.set_session_folder(&conversation, folder.as_deref()).map_err(err)
}
