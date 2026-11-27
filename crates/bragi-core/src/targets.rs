//! Settings and targets on [`Core`] (LF-11): Weirdnet links and OpenAI-compatible endpoints
//! (add, edit, delete, test, reorder), the sessions bound to them, and the folders that hold the
//! sessions. Sending routes by a session's target: the Weirdnet pool, or an endpoint and model.
//! Enabled fallback endpoints answer when the pool can't. See [`crate::endpoints`] for the keys.

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use weirdnet_agent::pool::PoolEvent;
use weirdnet_agent::{CancelToken, Message};

use crate::attach::Attachment;
use crate::chat::{ChatEvent, Streamed, coalesce};
use crate::core::{ChatHandle, Core, lock, random_id};
use crate::endpoints::{Endpoint, EndpointInput, EndpointMode, EndpointView, KeyChange};
use crate::error::{CoreError, ErrorKind, Result};
use crate::history::MessageState;
use crate::openai::{self, TestReport};
use crate::sessions::{Folder, SessionMeta, Target};

/// A Weirdnet node address the user added (or a linked device's), as Settings lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkView {
    pub address: String,
    pub peer: String,
    /// The node answers as an inference node right now.
    pub online: bool,
}

/// What deleting an endpoint did to the sessions bound to it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DeleteReport {
    pub rebound: usize,
    pub unbound: usize,
}

/// A target a session can be bound to, for the picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetOption {
    pub target: Target,
    pub label: String,
}

/// A conversation with where it lives and what it talks to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub updated_ms: i64,
    pub messages: usize,
    pub folder: Option<String>,
    pub target: Option<Target>,
    /// The words for the target: `Weirdnet pool`, `Name · model`, or what's missing.
    pub target_label: String,
    pub unbound: bool,
}

impl Core {
    // ---- Weirdnet links ----

    /// The node addresses this device dials, with whether each answers as an inference node.
    pub fn links(&self) -> Vec<LinkView> {
        let online: Vec<String> = self.state().catalog.nodes.iter().map(|n| n.id.clone()).collect();
        self.peers()
            .into_iter()
            .map(|address| {
                let peer = address.rsplit("/p2p/").next().unwrap_or_default().to_string();
                LinkView { online: online.contains(&peer), address, peer }
            })
            .collect()
    }

    /// Stops dialing `address` and forgets it (a connection that is up stays until the app
    /// restarts). History is untouched.
    pub fn remove_link(&self, address: &str) -> Result<()> {
        let address = address.trim();
        let known = {
            let mut p = lock(&self.inner.peers);
            let n = p.len();
            p.retain(|a| a != address);
            p.len() != n
        };
        if !known {
            return Err(CoreError::new(ErrorKind::NotFound, "that link isn't in the list"));
        }
        self.forget_saved_peer(address);
        Ok(())
    }

    /// Replaces one link's address with another (checked first, so a bad one changes nothing).
    pub fn edit_link(&self, old: &str, new: &str) -> Result<String> {
        crate::core::check_peer(new)?;
        if !self.peers().iter().any(|a| a == old.trim()) {
            return Err(CoreError::new(ErrorKind::NotFound, "that link isn't in the list"));
        }
        let line = self.add_node(new)?;
        if old.trim() != new.trim() {
            self.remove_link(old)?;
        }
        Ok(line)
    }

    // ---- Endpoints ----

    /// Opts in to the OS keychain for API keys (shells call this once after opening the core).
    /// Keys already in the sealed file move into it. Returns whether a keychain is in use; when
    /// there is none (no Secret Service, or iOS and Android for now) keys stay in the sealed file.
    pub fn use_os_keychain(&self) -> bool {
        match crate::secrets::os_keychain() {
            Some(store) => {
                self.inner.endpoints.use_keychain(store);
                true
            }
            None => false,
        }
    }

    /// Like [`Core::use_os_keychain`] with a store of the caller's choosing.
    pub fn use_secret_store(&self, store: std::sync::Arc<dyn crate::secrets::SecretStore>) {
        self.inner.endpoints.use_keychain(store);
    }

    /// The look: the chosen theme and mode, and the user's themes under this device's data.
    pub fn appearance(&self) -> crate::appearance::Appearance {
        crate::appearance::Appearance::open(&self.inner.cfg.dir)
    }

    pub fn endpoints(&self) -> Vec<EndpointView> {
        self.inner.endpoints.views()
    }

    /// Adds an endpoint. `api_key` is optional and is sealed on disk; it is never returned.
    pub fn add_endpoint(&self, input: EndpointInput, api_key: Option<String>) -> Result<EndpointView> {
        self.inner.endpoints.add(input, api_key)
    }

    pub fn edit_endpoint(&self, id: &str, input: EndpointInput, key: KeyChange) -> Result<EndpointView> {
        let v = self.inner.endpoints.edit(id, input, key)?;
        // A bound session keeps its target; the model it names may no longer exist, which the
        // next send says.
        Ok(v)
    }

    /// How many sessions are bound to the endpoint (for the delete confirmation).
    pub fn endpoint_sessions(&self, id: &str) -> usize {
        self.inner.sessions.bound_to(id)
    }

    /// Deletes an endpoint and its key. Sessions bound to it move to `rebind_to` (an endpoint or
    /// the pool) or, with `None`, are marked unbound so the next send asks for a target. No
    /// conversation or message is deleted.
    pub fn delete_endpoint(&self, id: &str, rebind_to: Option<Target>) -> Result<DeleteReport> {
        if self.inner.endpoints.get(id).is_none() {
            return Err(CoreError::new(ErrorKind::NotFound, "no such endpoint"));
        }
        if let Some(Target::Endpoint { id: to, .. }) = &rebind_to {
            if to == id {
                return Err(CoreError::new(ErrorKind::BadRequest, "pick a different target to move the sessions to"));
            }
            if self.inner.endpoints.get(to).is_none() {
                return Err(CoreError::new(ErrorKind::NotFound, "no such endpoint to move the sessions to"));
            }
        }
        let n = self.inner.sessions.rebind_endpoint(id, rebind_to.as_ref())?;
        self.inner.endpoints.delete(id)?;
        Ok(if rebind_to.is_some() { DeleteReport { rebound: n, unbound: 0 } } else { DeleteReport { rebound: 0, unbound: n } })
    }

    /// Moves an endpoint to `index` in the order (fallbacks are tried in this order).
    pub fn move_endpoint(&self, id: &str, index: usize) -> Result<()> {
        self.inner.endpoints.move_to(id, index)
    }

    /// Tests a saved endpoint: lists its models.
    pub async fn test_endpoint(&self, id: &str) -> Result<TestReport> {
        let t = self.inner.endpoints.target(id)?;
        Ok(openai::test(&t).await)
    }

    /// Tests an address and key that aren't saved yet (the add form's Test button).
    pub async fn test_endpoint_address(&self, base_url: &str, api_key: Option<String>) -> Result<TestReport> {
        let t = openai::Target { base_url: openai::normalize_url(base_url)?, api_key: api_key.filter(|k| !k.trim().is_empty()) };
        Ok(openai::test(&t).await)
    }

    /// The model ids a saved endpoint serves.
    pub async fn endpoint_models(&self, id: &str) -> Result<Vec<String>> {
        openai::list_models(&self.inner.endpoints.target(id)?).await
    }

    // ---- Targets and sessions ----

    /// What a new session can be bound to: the pool, then each enabled endpoint that is offered
    /// alongside it (with its default model; `endpoint_models` lists the rest).
    pub fn targets(&self) -> Vec<TargetOption> {
        let mut v = vec![TargetOption { target: Target::Pool, label: "Weirdnet pool".into() }];
        for e in self.inner.endpoints.list().into_iter().filter(|e| e.enabled && e.mode == EndpointMode::Alongside) {
            let model = e.model.clone().unwrap_or_default();
            let label = if model.is_empty() { e.name.clone() } else { format!("{} · {model}", e.name) };
            v.push(TargetOption { target: Target::Endpoint { id: e.id, model }, label });
        }
        if let Some(m) = self.on_device_ready() {
            v.push(TargetOption { target: Target::Device { model: m.id }, label: format!("{} · {}", crate::on_device::LABEL, m.label) });
        }
        v
    }

    fn target_label(&self, m: &SessionMeta) -> String {
        match &m.target {
            _ if m.unbound => "Target deleted: pick another".into(),
            None | Some(Target::Pool) => "Weirdnet pool".into(),
            Some(Target::Endpoint { id, model }) => match self.inner.endpoints.get(id) {
                Some(e) if model.is_empty() => e.name,
                Some(e) => format!("{} · {model}", e.name),
                None => "Endpoint missing".into(),
            },
            Some(Target::Device { model }) => match self.on_device_models().into_iter().find(|m| &m.id == model) {
                Some(m) => format!("{} · {}", crate::on_device::LABEL, m.label),
                None => crate::on_device::LABEL.into(),
            },
        }
    }

    /// The conversations with their folder and target, newest first.
    pub fn sessions(&self, offset: usize, limit: usize) -> Result<Vec<SessionSummary>> {
        let metas = self.inner.sessions.all();
        Ok(self
            .conversations(offset, limit)?
            .into_iter()
            .map(|c| {
                let m = metas.get(&c.id).cloned().unwrap_or_default();
                SessionSummary {
                    target_label: self.target_label(&m),
                    id: c.id,
                    title: c.title,
                    updated_ms: c.updated_ms,
                    messages: c.messages,
                    folder: m.folder,
                    target: m.target,
                    unbound: m.unbound,
                }
            })
            .collect())
    }

    pub fn folders(&self) -> Vec<Folder> {
        self.inner.sessions.folders()
    }

    pub fn create_folder(&self, name: &str, parent: Option<&str>) -> Result<Folder> {
        self.inner.sessions.create_folder(name, parent)
    }

    pub fn rename_folder(&self, id: &str, name: &str) -> Result<()> {
        self.inner.sessions.rename_folder(id, name)
    }

    pub fn move_folder(&self, id: &str, parent: Option<&str>) -> Result<()> {
        self.inner.sessions.move_folder(id, parent)
    }

    /// Deletes a folder; what it holds moves up one level. Never deletes a conversation.
    pub fn delete_folder(&self, id: &str) -> Result<()> {
        self.inner.sessions.delete_folder(id)
    }

    pub fn set_session_folder(&self, conversation: &str, folder: Option<&str>) -> Result<()> {
        self.conversation(conversation)?;
        self.inner.sessions.set_folder(conversation, folder)
    }

    /// Binds an existing session to a target (also how an unbound one is fixed).
    pub fn bind_session(&self, conversation: &str, target: Target) -> Result<()> {
        self.conversation(conversation)?;
        self.check_target(&target)?;
        self.inner.sessions.bind(conversation, target)
    }

    pub fn session_target(&self, conversation: &str) -> SessionMeta {
        self.inner.sessions.meta(conversation)
    }

    fn check_target(&self, t: &Target) -> Result<()> {
        match t {
            Target::Endpoint { id, .. } if self.inner.endpoints.get(id).is_none() => {
                Err(CoreError::new(ErrorKind::NotFound, "that endpoint isn't in Settings any more"))
            }
            Target::Device { model } => self.check_on_device(model).map(|_| ()),
            _ => Ok(()),
        }
    }

    // ---- Sending ----

    /// [`send_with`](Core::send_with) to a target. With `target` `None` a session uses the one it
    /// is bound to, and a new one the default: the pool when this device is linked, else the first
    /// endpoint offered alongside it. A new session is bound to the target it was sent to. A
    /// session whose endpoint was deleted must be given a target (it is then bound to it).
    pub fn send_to(
        &self,
        conversation: Option<&str>,
        prompt: &str,
        model: Option<String>,
        attachments: Vec<Attachment>,
        target: Option<Target>,
    ) -> Result<ChatHandle> {
        let meta = conversation.map(|c| self.inner.sessions.meta(c));
        let (target, explicit) = match (target, &meta) {
            (Some(t), _) => (t, true),
            (None, Some(m)) if m.unbound => {
                return Err(CoreError::new(ErrorKind::BadRequest, "this chat's endpoint was deleted; choose where to send it"));
            }
            (None, Some(SessionMeta { target: Some(t), .. })) => (t.clone(), false),
            (None, _) => (self.default_target(), false),
        };
        self.check_target(&target)?;
        let handle = match &target {
            Target::Pool => self.send_pool(conversation, prompt, model, attachments)?,
            Target::Endpoint { id, model: m } => {
                let ep =
                    self.inner.endpoints.get(id).ok_or_else(|| CoreError::new(ErrorKind::NotFound, "that endpoint isn't in Settings any more"))?;
                let m = Some(m.clone()).filter(|m| !m.is_empty()).or(model);
                // Outside MCP servers apply (enabled, in scope): the model gets their tools and
                // Bragi runs the tool-call loop, asking before each call. Images and audio keep
                // the plain chat (agent mode takes text only).
                let text_only = crate::attach::needs(&attachments) == weirdnet_agent::Modalities::TEXT;
                if text_only && !self.inner.mcp.applying(None, conversation).is_empty() {
                    self.send_agent_local(conversation, prompt, &ep.id, m, None, attachments)?
                } else {
                    self.send_endpoints(conversation, prompt, attachments, vec![ep], m)?
                }
            }
            Target::Device { model } => self.send_device(conversation, prompt, attachments, self.check_on_device(model)?)?,
        };
        // Bind new sessions, and sessions the caller moved, to where they went. A pool session that
        // fell back to an endpoint stays a pool session.
        let bound = self.inner.sessions.meta(&handle.conversation_id);
        if explicit || bound.target.is_none() {
            self.inner.sessions.bind(&handle.conversation_id, target)?;
        }
        Ok(handle)
    }

    /// [`complete`](Core::complete) sent to `target` (the default target when `None`): the pool,
    /// with fallback endpoints when it can't answer, or one endpoint and model. Nothing is saved.
    pub fn complete_to(&self, messages: Vec<crate::code::PromptMessage>, model: Option<String>, target: Option<Target>) -> Result<ChatHandle> {
        use crate::code::PromptRole;
        if !messages.iter().any(|m| m.role == PromptRole::User && !m.content.trim().is_empty()) {
            return Err(CoreError::new(ErrorKind::BadRequest, "nothing to answer"));
        }
        let messages: Vec<Message> = messages
            .into_iter()
            .map(|m| match m.role {
                PromptRole::System => Message::system(m.content),
                PromptRole::User => Message::user(m.content),
                PromptRole::Assistant => Message::assistant(m.content, Vec::new()),
            })
            .collect();
        let target = target.unwrap_or_else(|| self.default_target());
        self.check_target(&target)?;
        let (endpoints, model) = match target {
            Target::Pool => {
                let fallbacks = self.inner.endpoints.fallbacks();
                let ready = self.net().and_then(|net| self.pick_model(model.clone(), weirdnet_agent::Modalities::TEXT).map(|m| (net, m)));
                match ready {
                    Ok((net, model)) => {
                        let req = weirdnet_agent::remote::infer::v2::Request::messages(messages.clone()).with_model(model);
                        let fallback = (!fallbacks.is_empty()).then_some(messages);
                        return Ok(self.run(net, String::new(), req, String::new(), String::new(), fallback, Default::default()));
                    }
                    Err(e) if matches!(e.kind, ErrorKind::NotPaired | ErrorKind::Unavailable) && !fallbacks.is_empty() => (fallbacks, None),
                    Err(e) if matches!(e.kind, ErrorKind::NotPaired | ErrorKind::Unavailable) => match self.on_device_ready() {
                        Some(spec) => return Ok(self.start_device(String::new(), spec, messages)),
                        None => return Err(e),
                    },
                    Err(e) => return Err(e),
                }
            }
            Target::Endpoint { id, model: m } => {
                let ep =
                    self.inner.endpoints.get(&id).ok_or_else(|| CoreError::new(ErrorKind::NotFound, "that endpoint isn't in Settings any more"))?;
                (vec![ep], Some(m).filter(|m| !m.is_empty()).or(model))
            }
            Target::Device { model } => return Ok(self.start_device(String::new(), self.check_on_device(&model)?, messages)),
        };
        let request_id = random_id("r");
        let cancel = CancelToken::new();
        lock(&self.inner.requests).insert(request_id.clone(), cancel.clone());
        let (out, events) = mpsc::unbounded_channel();
        let (core, rid) = (self.clone(), request_id.clone());
        self.inner.handle.spawn(async move {
            core.answer_from_endpoints("", &rid, endpoints, model, messages, &cancel, &out, String::new(), Default::default()).await;
        });
        Ok(ChatHandle { request_id, conversation_id: String::new(), events })
    }

    fn default_target(&self) -> Target {
        if self.net().is_err()
            && let Some(t) = self.targets().into_iter().nth(1)
        {
            return t.target;
        }
        Target::Pool
    }

    fn send_pool(&self, conversation: Option<&str>, prompt: &str, model: Option<String>, attachments: Vec<Attachment>) -> Result<ChatHandle> {
        let fallbacks = self.inner.endpoints.fallbacks();
        let needed = crate::attach::needs(&attachments);
        let ready = self.net().and_then(|net| self.pick_model(model.clone(), needed).map(|m| (net, m)));
        match ready {
            Ok((net, model)) => {
                let whole = crate::video_core::sends_whole(&attachments).then(|| attachments.clone());
                let (id, messages) = self.begin_turn(conversation, prompt, &model, attachments)?;
                let (messages, video) = match &whole {
                    Some(a) => self.whole_video(&id, prompt, a, messages),
                    None => (messages, Default::default()),
                };
                // Fallback endpoints get the frames: what they read was never checked.
                let fallback = (!fallbacks.is_empty()).then(|| video.frames.clone().unwrap_or_else(|| messages.clone()));
                let req = weirdnet_agent::remote::infer::v2::Request::messages(messages).with_model(model);
                Ok(self.run(net, id, req, String::new(), String::new(), fallback, video))
            }
            Err(e) if matches!(e.kind, ErrorKind::NotPaired | ErrorKind::Unavailable) && !fallbacks.is_empty() => {
                self.send_endpoints(conversation, prompt, attachments, fallbacks, None)
            }
            // No node and no endpoint: the on-device model answers when it is on and downloaded.
            Err(e) if matches!(e.kind, ErrorKind::NotPaired | ErrorKind::Unavailable) && needed == weirdnet_agent::Modalities::TEXT => {
                match self.on_device_ready() {
                    Some(spec) => self.send_device(conversation, prompt, attachments, spec),
                    None => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Starts a turn answered by the on-device model (text only).
    fn send_device(
        &self,
        conversation: Option<&str>,
        prompt: &str,
        attachments: Vec<Attachment>,
        spec: weirdnet_model::ModelSpec,
    ) -> Result<ChatHandle> {
        if crate::attach::needs(&attachments) != weirdnet_agent::Modalities::TEXT {
            return Err(CoreError::new(ErrorKind::BadRequest, "the on-device model reads text only; send pictures, sound and video to a node"));
        }
        let (id, messages) = self.begin_turn(conversation, prompt, &spec.label, attachments)?;
        Ok(self.start_device(id, spec, messages))
    }

    /// Runs `messages` on the on-device model; the answer is saved in `conversation` (nowhere
    /// when empty).
    fn start_device(&self, conversation: String, spec: weirdnet_model::ModelSpec, messages: Vec<Message>) -> ChatHandle {
        let request_id = random_id("r");
        let cancel = CancelToken::new();
        lock(&self.inner.requests).insert(request_id.clone(), cancel.clone());
        let (out, events) = mpsc::unbounded_channel();
        let (core, rid, conv) = (self.clone(), request_id.clone(), conversation.clone());
        self.inner.handle.spawn(async move {
            core.answer_on_device(&conv, &rid, spec, messages, &cancel, &out).await;
        });
        ChatHandle { request_id, conversation_id: conversation, events }
    }

    /// Starts a turn answered by the first of `endpoints` that works.
    fn send_endpoints(
        &self,
        conversation: Option<&str>,
        prompt: &str,
        attachments: Vec<Attachment>,
        endpoints: Vec<Endpoint>,
        model: Option<String>,
    ) -> Result<ChatHandle> {
        let first = endpoints.first().ok_or_else(|| CoreError::new(ErrorKind::Unavailable, "no endpoint to send to"))?;
        let label = model.clone().or_else(|| first.model.clone()).unwrap_or_default();
        let whole = crate::video_core::sends_whole(&attachments).then(|| attachments.clone());
        let (id, messages) = self.begin_turn(conversation, prompt, &label, attachments)?;
        let (messages, video) = match &whole {
            Some(a) => self.whole_video(&id, prompt, a, messages),
            None => (messages, Default::default()),
        };
        let request_id = random_id("r");
        let cancel = CancelToken::new();
        lock(&self.inner.requests).insert(request_id.clone(), cancel.clone());
        let (out, events) = mpsc::unbounded_channel();
        let (core, rid, conv) = (self.clone(), request_id.clone(), id.clone());
        self.inner.handle.spawn(async move {
            core.answer_from_endpoints(&conv, &rid, endpoints, model, messages, &cancel, &out, String::new(), video).await;
        });
        Ok(ChatHandle { request_id, conversation_id: id, events })
    }

    /// Tries `endpoints` in order until one streams; saves the answer, ends the request and sends
    /// the last event. Used for an endpoint target and as the pool's fallback. `video`: when
    /// `messages` carry a whole clip, the frames to send instead if an endpoint turns it down
    /// (Native video V4: a 400 that says send frames, or a 413), and a note to show first.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn answer_from_endpoints(
        &self,
        conversation: &str,
        request_id: &str,
        endpoints: Vec<Endpoint>,
        model: Option<String>,
        messages: Vec<Message>,
        cancel: &CancelToken,
        out: &mpsc::UnboundedSender<ChatEvent>,
        shown: String,
        video: crate::video_core::VideoSend,
    ) {
        let mut messages = messages;
        let mut frames = video.frames.clone();
        if let Some(note) = &video.note {
            let _ = out.send(ChatEvent::Retry { attempt: 0, message: note.clone() });
        }
        let mut last: Option<CoreError> = None;
        let mut won: Option<(Endpoint, String, Streamed, openai::Finished)> = None;
        let mut partial: Option<Streamed> = None;
        for ep in endpoints {
            if cancel.is_cancelled() {
                last = Some(CoreError::new(ErrorKind::Cancelled, "stopped"));
                break;
            }
            let t = match self.inner.endpoints.target(&ep.id) {
                Ok(t) => t,
                Err(e) => {
                    last = Some(e);
                    continue;
                }
            };
            let chosen = match model.clone().or_else(|| ep.model.clone()) {
                Some(m) => m,
                None => match openai::list_models(&t).await.ok().and_then(|m| m.into_iter().next()) {
                    Some(m) => m,
                    None => {
                        last = Some(CoreError::new(ErrorKind::BadRequest, format!("{} has no model picked and lists none", ep.name)));
                        continue;
                    }
                },
            };
            let (r, streamed) = loop {
                let (tx, rx) = mpsc::unbounded_channel::<PoolEvent>();
                let _ = tx.send(PoolEvent::Accepted { node: ep.name.clone(), model: chosen.clone(), resume: false });
                let co = tokio::spawn(coalesce(rx, out.clone(), shown.chars().count() as u64, 0));
                let r = openai::chat_stream(&t, &chosen, &messages, &tx, cancel).await;
                drop(tx);
                let streamed = co.await.unwrap_or_default();
                // A whole clip turned down before any text: the frames go instead, here and to
                // any endpoint after this one, and the message says so.
                if let Err(e) = &r
                    && streamed.text.is_empty()
                    && e.kind != ErrorKind::Cancelled
                    && crate::video::whole_refused(e)
                    && let Some(f) = frames.take()
                {
                    let _ = out.send(ChatEvent::Retry { attempt: 1, message: video.retry_message(e) });
                    self.sent_as_frames(conversation);
                    messages = f;
                    continue;
                }
                break (r, streamed);
            };
            match r {
                Ok(f) => {
                    won = Some((ep, chosen, streamed, f));
                    break;
                }
                Err(e) => {
                    let had_text = !streamed.text.is_empty();
                    partial = Some(streamed);
                    let retry_next = !had_text && e.kind != ErrorKind::Cancelled;
                    last = Some(e);
                    if !retry_next {
                        break;
                    }
                    let _ = out.send(ChatEvent::Retry { attempt: 1, message: format!("{} didn't answer; trying the next one", ep.name) });
                }
            }
        }
        let tail = match won {
            Some((ep, chosen, s, f)) => {
                // An empty conversation is an answer kept out of history (`Core::complete_to`).
                if !conversation.is_empty() {
                    self.finish_turn(conversation, format!("{shown}{}", s.text), &s, MessageState::Complete, Some(ep.name), Some(chosen));
                }
                ChatEvent::Done { finish_reason: f.finish_reason }
            }
            None => {
                let e = last.unwrap_or_else(|| CoreError::new(ErrorKind::Unavailable, "no endpoint answered"));
                let s = partial.unwrap_or_default();
                let text = format!("{shown}{}", s.text);
                let state = if e.kind == ErrorKind::Cancelled { MessageState::Stopped } else { MessageState::Failed };
                if !conversation.is_empty() {
                    self.finish_turn(conversation, text.clone(), &s, state, None, None);
                }
                ChatEvent::Error { kind: e.kind, message: e.message, partial: text }
            }
        };
        lock(&self.inner.requests).remove(request_id);
        let _ = out.send(tail);
    }

    fn forget_saved_peer(&self, address: &str) {
        if self.inner.cfg.transport != crate::core::Transport::Internet {
            return;
        }
        let kept: String = crate::core::load_peers(&self.pool_dir()).into_iter().filter(|a| a != address).map(|a| format!("{a}\n")).collect();
        if let Err(e) = std::fs::write(self.pool_dir().join(crate::core::PEERS_FILE), kept) {
            tracing::warn!("saving node addresses: {e}");
        }
    }
}
