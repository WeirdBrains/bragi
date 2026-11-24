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
    /// This device's Bragi data folder, and its side of the desktop link.
    dir: PathBuf,
    link: Arc<bragi_core::link::LinkClient>,
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

/// The profile folder and the desktop link client (see `api::code`).
pub(crate) fn running_parts() -> Result<(Core, PathBuf, Arc<bragi_core::link::LinkClient>), String> {
    slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|r| (r.core.clone(), r.dir.clone(), r.link.clone()))
        .ok_or_else(|| "Bragi has not started".to_string())
}

/// Registers a started answer for `follow_chat` (code mode's asks use it too).
pub(crate) fn register(h: ChatHandle) -> ChatStarted {
    started(h)
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

/// A file in a chat, for its card: what was attached, or what an answer wrote.
#[derive(Debug, Clone)]
pub struct FileView {
    /// What `file_bytes` and `save_file` take.
    pub id: String,
    pub message_id: String,
    pub name: String,
    /// The path an answer gave it (`src/main.rs`), when it had one.
    pub path: Option<String>,
    pub mime: String,
    pub size: u64,
    /// `text`, `image` or `audio`.
    pub kind: String,
    /// `attached` or `reply`.
    pub origin: String,
    /// A text file's first lines.
    pub preview: Option<String>,
    /// How long a sound file plays, from its headers.
    pub duration_ms: Option<u64>,
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
    /// The message's file cards: its attachments, then the files its answer wrote.
    pub files: Vec<FileView>,
    /// Agent mode: the tools the task used, in order.
    pub tools: Vec<String>,
    /// The answer came from an agent task on a node.
    pub agent: bool,
    /// `chat` or `code`: the mode the turn was made in (one conversation holds both).
    pub mode: String,
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
    /// The user marked it for training (off unless set; see `set_training`).
    pub training: bool,
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
        training: c.training,
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
                files: bragi_core::files::describe(m).iter().map(file_view).collect(),
                tools: m.tools.clone(),
                agent: m.agent,
                mode: m.mode.name().to_string(),
                at_ms: m.at_ms,
            })
            .collect(),
    }
}

fn file_view(f: &bragi_core::ChatFile) -> FileView {
    FileView {
        id: f.id.clone(),
        message_id: f.message_id.clone(),
        name: f.name.clone(),
        path: f.path.clone(),
        mime: f.mime.clone(),
        size: f.size,
        kind: f.kind.name().into(),
        origin: match f.origin {
            bragi_core::FileOrigin::Attached => "attached",
            bragi_core::FileOrigin::Reply => "reply",
        }
        .into(),
        preview: f.preview.clone(),
        duration_ms: f.duration_ms,
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
    let dir = PathBuf::from(dir);
    let core = Core::start(Config::new(dir.clone(), label)).map_err(err)?;
    // API keys go to the OS keychain where there is one (Android and iOS keep the sealed file for now).
    core.use_os_keychain();
    let link = Arc::new(bragi_core::link::LinkClient::open(bragi_core::link::server::link_dir(&dir), false));
    *s = Some(Running { core, dir, link, _demo: None, _runtime: None });
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
    let app = dir.join("app");
    let link = Arc::new(bragi_core::link::LinkClient::open(bragi_core::link::server::link_dir(&app), true));
    *s = Some(Running { core, dir: app, link, _demo: Some(node), _runtime: Some(rt) });
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

/// An audio player's `elapsed / total` (`0:07 / 1:02`), written as on the desktop.
#[flutter_rust_bridge::frb(sync)]
pub fn audio_times(position_ms: u64, duration_ms: u64) -> String {
    bragi_core::audio::times(position_ms, duration_ms)
}

/// Where a seek bar at `fraction` (0 to 1) goes, in milliseconds.
#[flutter_rust_bridge::frb(sync)]
pub fn audio_seek_ms(fraction: f64, duration_ms: u64) -> u64 {
    bragi_core::audio::seek_ms(fraction, duration_ms)
}

/// The demo's sound file: `ms` milliseconds of a `hz` tone as a WAV.
#[flutter_rust_bridge::frb(sync)]
pub fn demo_tone_wav(ms: u32, hz: u32) -> Vec<u8> {
    bragi_core::demo::tone_wav(ms, hz)
}

/// Every file in conversation `id`, oldest first.
pub fn chat_files(id: String) -> Result<Vec<FileView>, String> {
    core()?.files(&id).map(|f| f.iter().map(file_view).collect()).map_err(err)
}

/// A file's bytes: an image preview, or saving through the phone's share/save sheet.
pub fn file_bytes(conversation: String, file: String) -> Result<Vec<u8>, String> {
    core()?.file_bytes(&conversation, &file).map(|(_, b)| b).map_err(err)
}

/// Saves a file at `dest` (a path a save dialog returned is replaced; a folder gets the file's
/// name without replacing anything). Returns where it went.
pub fn save_file(conversation: String, file: String, dest: String) -> Result<String, String> {
    core()?.save_file(&conversation, &file, std::path::Path::new(&dest)).map(|p| p.display().to_string()).map_err(err)
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

/// Marks a conversation for training, or withdraws it (off unless set; it syncs to every
/// linked device, and only marked conversations are exported, on the user's own devices).
pub fn set_training(id: String, marked: bool) -> Result<(), String> {
    core()?.set_training(&id, marked).map_err(err)
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
    core.handle()
        .spawn(async move { c.test_endpoint_address(&base_url, api_key).await })
        .await
        .map_err(|e| e.to_string())?
        .map(test_view)
        .map_err(err)
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

// ---- models on the pool (LF-30) ------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ModelNodeView {
    pub peer: String,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct StoredModelView {
    pub id: String,
    pub size: u64,
    pub complete: bool,
    /// The id the node serves it under, when it does.
    pub served_as: Option<String>,
    pub state: Option<String>,
}

/// One device of the user's. `status` is `ready`, `not_model_node`, `needs_owner_device` (the node
/// refused this device; `detail` has its reason) or `unreachable`.
#[derive(Debug, Clone)]
pub struct NodeModelsView {
    pub node: ModelNodeView,
    pub status: String,
    pub detail: Option<String>,
    pub models: Vec<StoredModelView>,
}

#[derive(Debug, Clone)]
pub struct HubHitView {
    pub repo: String,
    pub downloads: u64,
    pub likes: u64,
    pub gated: bool,
    pub updated: Option<String>,
}

#[derive(Debug, Clone)]
pub struct QuantFitView {
    pub id: String,
    pub quant: Option<String>,
    pub size: u64,
    pub footprint: u64,
    pub fits: bool,
}

/// A repo on one node. `verdict` is `fits`, `too_big`, `no_budget` or `unknown`; `status` and
/// `detail` are as in [`NodeModelsView`].
#[derive(Debug, Clone)]
pub struct NodeFitView {
    pub node: ModelNodeView,
    pub status: String,
    pub detail: Option<String>,
    pub verdict: String,
    pub best: Option<String>,
    pub best_size: u64,
    pub usable_bytes: u64,
    pub quants: Vec<QuantFitView>,
}

/// One event of a search or a pull. `kind` is `hits`, `fit`, `search_done`, `chosen`, `progress`,
/// `node_done`, `done`, `cancelled` or `failed`; the other fields are set when that kind has them.
#[derive(Debug, Clone, Default)]
pub struct ModelsEventView {
    pub kind: String,
    pub hits: Vec<HubHitView>,
    pub repo: Option<String>,
    pub fit: Option<NodeFitView>,
    pub node: Option<ModelNodeView>,
    /// `chosen`: the model and why. `search_done` and `failed`: the error. `node_done`: the error, if it failed.
    pub model: Option<String>,
    pub text: Option<String>,
    pub size: u64,
    pub done: u64,
    pub total: u64,
    pub served: bool,
}

fn node_view(n: &bragi_core::ModelNode) -> ModelNodeView {
    ModelNodeView { peer: n.peer.clone(), label: n.label.clone() }
}

fn status_parts(s: &bragi_core::NodeStatus) -> (String, Option<String>) {
    use bragi_core::NodeStatus as S;
    match s {
        S::Ready => ("ready".into(), None),
        S::NotModelNode => ("not_model_node".into(), None),
        S::NeedsOwnerDevice(r) => ("needs_owner_device".into(), Some(r.clone())),
        S::Unreachable(m) => ("unreachable".into(), Some(m.clone())),
    }
}

fn fit_view(f: &bragi_core::NodeFit) -> NodeFitView {
    use bragi_core::Verdict as V;
    let (status, detail) = status_parts(&f.status);
    let verdict = match f.verdict() {
        V::Fits => "fits",
        V::TooBig => "too_big",
        V::NoBudget => "no_budget",
        _ => "unknown",
    };
    NodeFitView {
        node: node_view(&f.node),
        status,
        detail,
        verdict: verdict.into(),
        best: f.best.clone(),
        best_size: f.best_size,
        usable_bytes: f.usable_bytes,
        quants: f
            .quants
            .iter()
            .map(|q| QuantFitView { id: q.id.clone(), quant: q.quant.clone(), size: q.size, footprint: q.footprint, fits: q.fits })
            .collect(),
    }
}

fn models_event_view(e: bragi_core::ModelsEvent) -> ModelsEventView {
    use bragi_core::ModelsEvent as E;
    let base = ModelsEventView::default();
    match e {
        E::Hits(h) => ModelsEventView {
            kind: "hits".into(),
            hits: h
                .into_iter()
                .map(|h| HubHitView { repo: h.repo, downloads: h.downloads, likes: h.likes, gated: h.gated, updated: h.updated })
                .collect(),
            ..base
        },
        E::Fit { repo, fit } => ModelsEventView { kind: "fit".into(), repo: Some(repo), fit: Some(fit_view(&fit)), ..base },
        E::SearchDone { error } => ModelsEventView { kind: "search_done".into(), text: error, ..base },
        E::Chosen { node, model, size, why } => {
            ModelsEventView { kind: "chosen".into(), node: Some(node_view(&node)), model: Some(model), size, text: Some(why), ..base }
        }
        E::Progress { peer, label, done, total } => {
            ModelsEventView { kind: "progress".into(), node: Some(ModelNodeView { peer, label }), done, total, ..base }
        }
        E::NodeDone { node, result } => match result {
            Ok(p) => {
                ModelsEventView { kind: "node_done".into(), node: Some(node_view(&node)), model: Some(p.id), size: p.size, served: p.served, ..base }
            }
            Err(m) => ModelsEventView { kind: "node_done".into(), node: Some(node_view(&node)), text: Some(m), ..base },
        },
        E::Done => ModelsEventView { kind: "done".into(), ..base },
        E::Cancelled => ModelsEventView { kind: "cancelled".into(), ..base },
        E::Failed { message } => ModelsEventView { kind: "failed".into(), text: Some(message), ..base },
    }
}

fn pending_models() -> &'static Mutex<HashMap<String, UnboundedReceiver<bragi_core::ModelsEvent>>> {
    static P: OnceLock<Mutex<HashMap<String, UnboundedReceiver<bragi_core::ModelsEvent>>>> = OnceLock::new();
    P.get_or_init(Default::default)
}

fn keep_run(run: bragi_core::ModelsRun) -> String {
    pending_models().lock().unwrap_or_else(|e| e.into_inner()).insert(run.id.clone(), run.events);
    run.id
}

/// Every other device of the user's: whether it takes model management, and the models it has.
/// `needs_owner_device` means the node refused this device: only devices linked to its owner
/// may manage models.
pub async fn models_overview() -> Result<Vec<NodeModelsView>, String> {
    Ok(core()?
        .models_overview()
        .await
        .map_err(err)?
        .into_iter()
        .map(|n| {
            let (status, detail) = status_parts(&n.status);
            NodeModelsView {
                node: node_view(&n.node),
                status,
                detail,
                models: n
                    .models
                    .into_iter()
                    .map(|m| StoredModelView { id: m.id, size: m.size, complete: m.complete, served_as: m.served_as, state: m.state })
                    .collect(),
            }
        })
        .collect())
}

/// Starts a search of the Hugging Face Hub (GGUF repos) with a fit per node; stream it with
/// `follow_models`. Returns the run's id.
pub fn models_search(query: String) -> Result<String, String> {
    core()?.models_search(&query).map(keep_run).map_err(err)
}

/// Starts a pull of `repo` (`model` is a quantization such as `Q4_K_M`'s id, or the best that fits
/// when none) to `target`: `auto` (the node where the biggest fits), `all`, or a node's peer id.
/// `serve` serves it from the node's Ymir right after the download. Stream it with `follow_models`;
/// `cancel` stops it (partial files stay and resume).
pub fn models_pull(repo: String, model: Option<String>, target: String, serve: bool) -> Result<String, String> {
    let target = match target.as_str() {
        "auto" => bragi_core::PullTarget::Auto,
        "all" => bragi_core::PullTarget::All,
        peer => bragi_core::PullTarget::Node(peer.to_string()),
    };
    let mut spec = bragi_core::PullSpec::new(repo, target);
    spec.model = model;
    spec.serve = serve;
    core()?.models_pull(spec).map(keep_run).map_err(err)
}

/// Streams a search's or pull's events, ending after `search_done`, `done`, `cancelled` or `failed`.
pub fn follow_models(run_id: String, sink: StreamSink<ModelsEventView>) -> Result<(), String> {
    let core = core()?;
    let mut rx = pending_models().lock().unwrap_or_else(|e| e.into_inner()).remove(&run_id).ok_or_else(|| "no such run".to_string())?;
    core.handle().spawn(async move {
        while let Some(e) = rx.recv().await {
            let end = matches!(
                e,
                bragi_core::ModelsEvent::SearchDone { .. }
                    | bragi_core::ModelsEvent::Done
                    | bragi_core::ModelsEvent::Cancelled
                    | bragi_core::ModelsEvent::Failed { .. }
            );
            if sink.add(models_event_view(e)).is_err() || end {
                return;
            }
        }
    });
    Ok(())
}

/// Removes a stored model from a node and returns the bytes freed. `confirmed` must be true.
pub async fn models_remove(peer: String, id: String, confirmed: bool) -> Result<u64, String> {
    core()?.models_remove(&peer, &id, confirmed).await
}

// ---- themes (LF-33) ----------------------------------------------------------------------------
//
// The look is `bragi_core::Appearance`'s (weirdnet-themes' format, shared with the desktop and
// terminal apps). The app paints a palette and calls these to switch, edit, duplicate, rename,
// delete, reset, import and export.

#[derive(Debug, Clone)]
pub struct ThemeRowView {
    pub name: String,
    pub display_name: String,
    pub builtin: bool,
    /// A user theme, or a built-in the user has edited.
    pub user_file: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ThemePrefView {
    pub theme: String,
    /// `light`, `dark` or `system`.
    pub mode: String,
}

#[derive(Debug, Clone)]
pub struct RoleColorView {
    pub role: String,
    /// `#rrggbb`.
    pub hex: String,
}

/// Every role of one variant as `#rrggbb`, in editor order.
#[derive(Debug, Clone)]
pub struct PaletteView {
    pub theme: String,
    pub dark: bool,
    pub roles: Vec<RoleColorView>,
    pub raised: String,
}

#[derive(Debug, Clone)]
pub struct RoleInfoView {
    pub role: String,
    pub label: String,
    /// Optional roles are derived from the others until set, and can be cleared back.
    pub optional: bool,
}

#[derive(Debug, Clone)]
pub struct RoleEdit {
    pub role: String,
    /// `#rgb` / `#rrggbb`, or empty to clear an optional role.
    pub value: String,
}

fn palette_view(p: bragi_core::Palette) -> PaletteView {
    PaletteView {
        theme: p.theme.clone(),
        dark: p.is_dark(),
        raised: p.raised.clone(),
        roles: p.roles.into_iter().map(|(role, hex)| RoleColorView { role, hex }).collect(),
    }
}

fn variant_of(dark: bool) -> bragi_core::appearance::Variant {
    if dark { bragi_core::appearance::Variant::Dark } else { bragi_core::appearance::Variant::Light }
}

fn pref_view(p: bragi_core::Preference) -> ThemePrefView {
    ThemePrefView { theme: p.theme, mode: p.mode.name().into() }
}

pub fn themes() -> Result<Vec<ThemeRowView>, String> {
    let rows = core()?.appearance().rows().map_err(err)?;
    Ok(rows
        .into_iter()
        .map(|r| ThemeRowView { name: r.name, display_name: r.display_name, builtin: r.builtin, user_file: r.user_file, error: r.error })
        .collect())
}

/// The roles an editor lists, in order.
#[frb(sync)]
pub fn theme_roles() -> Vec<RoleInfoView> {
    bragi_core::appearance::roles()
        .into_iter()
        .map(|r| RoleInfoView { role: r.into(), label: bragi_core::appearance::role_label(r).into(), optional: is_optional_role(r) })
        .collect()
}

fn is_optional_role(role: &str) -> bool {
    ["surface", "code_bg", "diff_add_bg", "diff_remove_bg", "link", "dim"].contains(&role)
}

#[frb(sync)]
pub fn theme_preference() -> Result<ThemePrefView, String> {
    Ok(pref_view(core()?.appearance().preference()))
}

pub fn set_theme_preference(theme: String, mode: String) -> Result<ThemePrefView, String> {
    let a = core()?.appearance();
    let mode = bragi_core::ModePref::parse(&mode).ok_or_else(|| format!("mode is light, dark or system, not {mode}"))?;
    // Only these two change; any other preference is kept.
    let mut p = a.preference();
    p.theme = theme;
    p.mode = mode;
    a.set_preference(&p).map_err(err)?;
    Ok(pref_view(p))
}

/// The palette to draw with now; `system_dark` is the platform's brightness, for system mode.
#[frb(sync)]
pub fn current_palette(system_dark: Option<bool>) -> Result<PaletteView, String> {
    Ok(palette_view(core()?.appearance().palette_with_system(system_dark)))
}

/// `name`'s palette in the dark or light variant (the picker's previews and the editor).
#[frb(sync)]
pub fn theme_palette(name: String, dark: bool) -> Result<PaletteView, String> {
    core()?.appearance().palette_for(&name, variant_of(dark)).map(palette_view).map_err(err)
}

/// The theme to edit when `name` is asked for: itself, or a copy of a built-in.
pub fn theme_editable(name: String) -> Result<String, String> {
    core()?.appearance().editable(&name).map_err(err)
}

/// Sets roles of one variant together (all or nothing) and returns the new palette.
pub fn theme_apply(name: String, dark: bool, edits: Vec<RoleEdit>) -> Result<PaletteView, String> {
    let a = core()?.appearance();
    let edits: Vec<(String, String)> = edits.into_iter().map(|e| (e.role, e.value)).collect();
    a.apply(&name, variant_of(dark), &edits).map_err(err)?;
    a.palette_for(&name, variant_of(dark)).map(palette_view).map_err(err)
}

pub fn theme_duplicate(name: String, as_name: Option<String>) -> Result<String, String> {
    core()?.appearance().duplicate(&name, as_name.as_deref()).map_err(err)
}

pub fn theme_rename(from: String, to: String) -> Result<(), String> {
    core()?.appearance().rename(&from, &to).map_err(err)
}

/// Deletes a user theme or a built-in's edits.
pub fn theme_delete(name: String) -> Result<(), String> {
    core()?.appearance().delete(&name).map_err(err)
}

/// Puts a built-in back as it ships.
pub fn theme_reset(name: String) -> Result<(), String> {
    core()?.appearance().reset(&name).map_err(err)
}

pub fn theme_export_text(name: String) -> Result<String, String> {
    core()?.appearance().export_text(&name).map_err(err)
}

/// Imports a theme file's text; a name already taken gets a number.
pub fn theme_import_text(text: String, as_name: Option<String>) -> Result<String, String> {
    core()?.appearance().import_text(&text, as_name.as_deref()).map_err(err)
}

/// Contrast notes for one variant (text on background and the like).
pub fn theme_warnings(name: String, dark: bool) -> Result<Vec<String>, String> {
    core()?.appearance().warnings(&name, variant_of(dark)).map_err(err)
}
