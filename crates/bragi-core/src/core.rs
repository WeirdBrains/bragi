//! [`Core`]: the one client core. Both shells call these actions and draw [`AppState`] and the
//! [`ChatEvent`]s; nothing else decides behaviour.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::runtime::{Handle, Runtime};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use weirdnet_agent::pool::{Interrupted, LinkedNodes, NetDialer, Pool, PoolEvent, PoolOptions, encode_cert};
use weirdnet_agent::remote::infer::v2::Request;
use weirdnet_agent::{CancelToken, Message};
use weirdnet_device_link::{HostNode, HostStatus, Pairing};
use weirdnet_net::{Keypair, Multiaddr, NodeConfig, PeerHint, PeerId};

use crate::catalog::CatalogView;
use crate::chat::{ChatEvent, coalesce};
use crate::error::{CoreError, ErrorKind, Result};
use crate::history::{ChatMessage, Conversation, ConversationSummary, History, MessageState, Role, now_ms, title_for};
use crate::identity::{Identity, SYNC_TYPE, SyncNode};

/// Bragi's link scheme (`weirdnet-bragi://link/…`). Never bare `weirdnet://`.
pub const SCHEME: &str = "weirdnet-bragi://";

/// The turn that asks the model to go on with a cut-off answer.
pub const CONTINUE_PROMPT: &str = "Continue your last answer from exactly where it stopped. Don't repeat any of it.";

/// How often the roster is synced with the user's other devices.
const SYNC_EVERY: Duration = Duration::from_secs(30);

/// How this device reaches the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    /// The in-process memory transport: tests and the demo profile.
    Memory,
    /// The real network: LAN discovery, the DHT and the Weird Brains relay.
    Internet,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// This device's Bragi data: `identity/` and `history/`.
    pub dir: PathBuf,
    /// What the other device calls this one while linking (`Bragi on the dev box`).
    pub label: String,
    pub transport: Transport,
    /// Devices to dial directly: `/ip4/…/tcp/…/p2p/PEER` (or `/memory/…/p2p/PEER`). Dialing
    /// grants nothing; the roster decides.
    pub peers: Vec<String>,
}

impl Config {
    pub fn new(dir: impl Into<PathBuf>, label: impl Into<String>) -> Self {
        Self { dir: dir.into(), label: label.into(), transport: Transport::Internet, peers: Vec::new() }
    }
}

/// Where linking stands.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PairState {
    #[default]
    Unpaired,
    /// Showing a code (QR or link) for a device that is already linked.
    Offering {
        link: String,
        expires_ms: i64,
    },
    /// The other device scanned it: compare this code on both screens.
    Confirm {
        sas: String,
    },
    /// Confirmed here; waiting for the other device.
    Waiting {
        sas: String,
    },
    Linked {
        user: String,
        device: String,
    },
    Failed {
        message: String,
    },
}

/// What the shells draw.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AppState {
    pub pair: PairState,
    pub catalog: CatalogView,
    /// The Weirdnet node is running.
    pub online: bool,
}

/// A streaming answer.
pub struct ChatHandle {
    pub request_id: String,
    pub conversation_id: String,
    pub events: mpsc::UnboundedReceiver<ChatEvent>,
}

/// The other side of a link started from here (`pair_accept`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkCheck {
    pub sas: String,
    pub device: String,
}

struct Net {
    identity: Arc<Identity>,
    node: Arc<SyncNode>,
    pool: Pool,
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for Net {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
        self.node.halt();
    }
}

struct Inner {
    cfg: Config,
    handle: Handle,
    history: History,
    state: watch::Sender<AppState>,
    net: Mutex<Option<Arc<Net>>>,
    offer: Mutex<Option<Arc<HostNode>>>,
    pairing: Mutex<Option<Pairing>>,
    requests: Mutex<HashMap<String, CancelToken>>,
}

/// The client core. Cheap to clone; every clone is the same core.
#[derive(Clone)]
pub struct Core {
    inner: Arc<Inner>,
    _runtime: Option<Arc<Runtime>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn random_id(prefix: &str) -> String {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{prefix}-{hex}")
}

fn hints(peers: &[String]) -> Result<Vec<(PeerId, Vec<Multiaddr>)>> {
    let mut out: Vec<(PeerId, Vec<Multiaddr>)> = Vec::new();
    for p in peers {
        let bad = |e: String| CoreError::new(ErrorKind::BadRequest, format!("peer `{p}`: {e}"));
        let hint = PeerHint::from_addr(p).map_err(|e| bad(e.to_string()))?;
        let id: PeerId = hint.peer_id.parse().map_err(|_| bad("not a peer id".into()))?;
        let addrs: Vec<Multiaddr> = hint.addrs.iter().filter_map(|a| a.parse().ok()).collect();
        match out.iter_mut().find(|(i, _)| *i == id) {
            Some((_, a)) => a.extend(addrs),
            None => out.push((id, addrs)),
        }
    }
    Ok(out)
}

/// The network settings for this device's node (and, with link limits, for a link session).
pub fn node_config(cfg: &Config, link: bool) -> Result<NodeConfig> {
    let mut c = match cfg.transport {
        Transport::Memory => weirdnet_sync::node_config(true),
        Transport::Internet => {
            let mut c = weirdnet_sync::node_config(false);
            c.listen = NodeConfig::default_listen(0);
            c.mdns = true;
            c.discovery = weirdnet_net::DiscoveryMode::Enabled;
            c.relays = weirdnet_device_link::KNOWN_RELAYS.iter().filter_map(|r| r.parse().ok()).collect();
            c
        }
    };
    if cfg.transport == Transport::Internet {
        c.peers = cfg.peers.iter().filter_map(|p| PeerHint::from_addr(p).ok()).collect();
    }
    c.label = cfg.label.clone();
    c.version = format!("bragi/{}", env!("CARGO_PKG_VERSION"));
    Ok(if link { weirdnet_device_link::with_link_limits(c) } else { c })
}

impl Core {
    /// Opens the core on `cfg.dir`. Uses the caller's tokio runtime when there is one, else
    /// starts its own (one per process). A linked device goes online in the background.
    pub fn start(cfg: Config) -> Result<Self> {
        let (handle, runtime) = match Handle::try_current() {
            Ok(h) => (h, None),
            Err(_) => {
                let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("bragi").enable_all().build()?;
                (rt.handle().clone(), Some(Arc::new(rt)))
            }
        };
        hints(&cfg.peers)?;
        std::fs::create_dir_all(&cfg.dir)?;
        let history = History::open(&cfg.dir)?;
        let (state, _) = watch::channel(AppState::default());
        let core = Core {
            inner: Arc::new(Inner {
                cfg,
                handle,
                history,
                state,
                net: Mutex::new(None),
                offer: Mutex::new(None),
                pairing: Mutex::new(None),
                requests: Mutex::new(HashMap::new()),
            }),
            _runtime: runtime,
        };
        if Identity::exists(&core.identity_dir()) {
            let c = core.clone();
            core.inner.handle.spawn(async move {
                if let Err(e) = c.go_online().await {
                    tracing::warn!("going online: {e}");
                    c.set_pair(PairState::Failed { message: e.message });
                }
            });
        }
        Ok(core)
    }

    /// The runtime the core runs on (the shells spawn their forwarding tasks here).
    pub fn handle(&self) -> &Handle {
        &self.inner.handle
    }

    fn identity_dir(&self) -> PathBuf {
        self.inner.cfg.dir.join("identity")
    }

    pub fn state(&self) -> AppState {
        self.inner.state.borrow().clone()
    }

    /// Follows [`AppState`].
    pub fn watch(&self) -> watch::Receiver<AppState> {
        self.inner.state.subscribe()
    }

    fn set_pair(&self, pair: PairState) {
        self.inner.state.send_modify(|s| s.pair = pair);
    }

    fn net(&self) -> Result<Arc<Net>> {
        lock(&self.inner.net).clone().ok_or_else(|| CoreError::new(ErrorKind::NotPaired, "link this device to your Weirdnet first"))
    }

    /// Starts the Weirdnet node, the pool and the catalog for the stored identity.
    async fn go_online(&self) -> Result<()> {
        let identity =
            Arc::new(Identity::open(&self.identity_dir())?.ok_or_else(|| CoreError::new(ErrorKind::NotPaired, "this device is not linked"))?);
        let ncfg = node_config(&self.inner.cfg, false)?;
        let (key, st) = (identity.node_key().clone(), identity.state());
        let node = tokio::task::spawn_blocking(move || weirdnet_sync::start_node(key, ncfg, st))
            .await
            .map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?
            .map_err(|e| CoreError::new(ErrorKind::Unavailable, format!("starting Weirdnet: {e}")))?;
        let node = Arc::new(node);
        let hints = hints(&self.inner.cfg.peers)?;
        let dialer = NetDialer::new(node.clone());
        let mut nodes = LinkedNodes::new(node.clone(), identity.roster_view());
        for (peer, addrs) in &hints {
            dialer.hint(*peer, addrs.clone());
            nodes = nodes.with_hint(*peer, addrs.clone());
        }
        let pool = Pool::new(Arc::new(dialer), nodes).with_options(PoolOptions::default().with_cert(Some(encode_cert(&identity.our_cert()))));

        let mut tasks = Vec::new();
        // The catalog: one inference node's, followed.
        let (state, p) = (self.inner.state.clone(), pool.clone());
        tasks.push(tokio::spawn(async move {
            let mut w = p.subscribe();
            while let Some((node, c)) = w.changed().await {
                let view = CatalogView::from_catalog(&node, &c);
                state.send_modify(|s| s.catalog = view);
            }
        }));
        // The roster: devices linked or revoked elsewhere.
        let (i, n, h) = (identity.clone(), node.clone(), hints.clone());
        tasks.push(tokio::spawn(async move {
            loop {
                tokio::time::sleep(SYNC_EVERY).await;
                let (i, n, h) = (i.clone(), n.clone(), h.clone());
                let _ = tokio::task::spawn_blocking(move || {
                    i.tick(&n, &|c| h.iter().find(|(p, _)| p.to_string() == c.peer_id).map(|(_, a)| a.clone()).unwrap_or_default())
                })
                .await;
            }
        }));
        let (user, device) = (identity.user_id(), identity.our_cert().label);
        *lock(&self.inner.net) = Some(Arc::new(Net { identity, node, pool, tasks }));
        self.inner.state.send_modify(|s| {
            s.online = true;
            s.pair = PairState::Linked { user, device };
        });
        Ok(())
    }

    // ---- pairing -------------------------------------------------------------------------

    /// Shows a link code for a device that is already in the user's Weirdnet (Morph, Ymir or
    /// another Bragi). Progress arrives in [`AppState::pair`].
    pub async fn pair_offer(&self) -> Result<String> {
        if Identity::exists(&self.identity_dir()) {
            return Err(CoreError::new(ErrorKind::Pairing, "this device is already linked"));
        }
        let key = Keypair::generate_ed25519();
        let (k, cfg, label) = (key.clone(), node_config(&self.inner.cfg, true)?, self.inner.cfg.label.clone());
        let host = tokio::task::spawn_blocking(move || HostNode::start(SCHEME, k, cfg, &label))
            .await
            .map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?
            .map_err(|e| CoreError::new(ErrorKind::Pairing, e.to_string()))?;
        let host = Arc::new(host);
        let link = host.link().to_string();
        *lock(&self.inner.offer) = Some(host.clone());
        self.set_pair(PairState::Offering { link: link.clone(), expires_ms: host.expires_ms() });
        let core = self.clone();
        self.inner.handle.spawn(async move { core.follow_offer(host, key).await });
        Ok(link)
    }

    async fn follow_offer(&self, host: Arc<HostNode>, key: Keypair) {
        let mut shown = false;
        loop {
            let status = host.status();
            let stop = |c: &Core, message: &str| {
                *lock(&c.inner.offer) = None;
                c.set_pair(PairState::Failed { message: message.into() });
            };
            match status {
                HostStatus::Waiting => {}
                HostStatus::Paired { sas } | HostStatus::Received { sas } if !shown => {
                    shown = true;
                    self.set_pair(PairState::Confirm { sas });
                }
                HostStatus::Received { .. } if matches!(self.state().pair, PairState::Waiting { .. }) => {
                    let adopted = host
                        .take()
                        .map_err(|e| CoreError::new(ErrorKind::Pairing, e.to_string()))
                        .and_then(|t| Ok(Identity::adopt(&self.identity_dir(), key.clone(), &t.seed, t.node_cert.clone(), &t.settings)?));
                    *lock(&self.inner.offer) = None;
                    match adopted {
                        Ok(_) => {
                            if let Err(e) = self.go_online().await {
                                self.set_pair(PairState::Failed { message: e.message });
                            }
                        }
                        Err(e) => self.set_pair(PairState::Failed { message: e.message }),
                    }
                    return;
                }
                HostStatus::Paired { .. } | HostStatus::Received { .. } => {}
                HostStatus::Done => return,
                HostStatus::Cancelled => return stop(self, "linking was cancelled"),
                HostStatus::Expired => return stop(self, "the code expired; show a new one"),
                HostStatus::Failed(why) => return stop(self, why),
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// The user compared the codes on the new device: `matches` confirms, otherwise cancels.
    pub fn pair_confirm(&self, matches: bool) -> Result<()> {
        let host = lock(&self.inner.offer).clone().ok_or_else(|| CoreError::new(ErrorKind::Pairing, "no link in progress"))?;
        let PairState::Confirm { sas } = self.state().pair else {
            return Err(CoreError::new(ErrorKind::Pairing, "nothing to confirm yet"));
        };
        if matches {
            host.confirm();
            self.set_pair(PairState::Waiting { sas });
        } else {
            host.cancel();
            *lock(&self.inner.offer) = None;
            self.set_pair(PairState::Failed { message: "the codes did not match".into() });
        }
        Ok(())
    }

    /// Stops showing a code.
    pub fn pair_cancel(&self) {
        if let Some(h) = lock(&self.inner.offer).take() {
            h.cancel();
        }
        if !matches!(self.state().pair, PairState::Linked { .. }) {
            self.set_pair(PairState::Unpaired);
        }
    }

    /// From a linked device: reads another device's code and pairs; compare the returned code
    /// with the other screen, then [`Core::pair_accept_confirm`].
    pub async fn pair_accept(&self, link: &str) -> Result<LinkCheck> {
        self.net()?;
        let (cfg, link) = (node_config(&self.inner.cfg, true)?, link.trim().to_string());
        let pairing = tokio::task::spawn_blocking(move || Pairing::start(SCHEME, &link, cfg))
            .await
            .map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?
            .map_err(|e| CoreError::new(ErrorKind::Pairing, e.to_string()))?;
        let check = LinkCheck { sas: pairing.sas().to_string(), device: pairing.device().to_string() };
        *lock(&self.inner.pairing) = Some(pairing);
        Ok(check)
    }

    /// The codes match (send the identity and link the device) or they don't (cancel).
    pub async fn pair_accept_confirm(&self, matches: bool) -> Result<()> {
        let net = self.net()?;
        let mut pairing = lock(&self.inner.pairing).take().ok_or_else(|| CoreError::new(ErrorKind::Pairing, "no link in progress"))?;
        if !matches {
            let _ = tokio::task::spawn_blocking(move || pairing.cancel()).await;
            return Ok(());
        }
        let seed = net.identity.seed()?;
        let settings = net.identity.link_settings();
        let cert = tokio::task::spawn_blocking(move || pairing.send_with(&seed, "Bragi", &settings, &[SYNC_TYPE.to_string()]))
            .await
            .map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?
            .map_err(|e| CoreError::new(ErrorKind::Pairing, e.to_string()))?;
        net.identity.approve(cert)?;
        Ok(())
    }

    // ---- chat ------------------------------------------------------------------------------

    fn pick_model(&self, model: Option<String>) -> Result<String> {
        match model {
            Some(m) if !m.is_empty() => Ok(m),
            _ => self
                .state()
                .catalog
                .default_model()
                .map(str::to_string)
                .ok_or_else(|| CoreError::new(ErrorKind::Unavailable, "no models yet: no inference node has answered")),
        }
    }

    /// Sends `prompt` in `conversation` (a new one when `None`) to `model` (the default when
    /// `None`), streaming the answer.
    pub fn send(&self, conversation: Option<&str>, prompt: &str, model: Option<String>) -> Result<ChatHandle> {
        let net = self.net()?;
        let model = self.pick_model(model)?;
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Err(CoreError::new(ErrorKind::BadRequest, "type a message first"));
        }
        let now = now_ms();
        let mut c = match conversation {
            Some(id) => self.inner.history.load(id)?,
            None => Conversation { id: random_id("c"), title: title_for(prompt), created_ms: now, updated_ms: now, messages: Vec::new() },
        };
        c.messages.push(ChatMessage { role: Role::User, text: prompt.into(), state: MessageState::Complete, model: None, node: None, at_ms: now });
        let messages = agent_messages(&c.messages);
        c.messages.push(ChatMessage {
            role: Role::Assistant,
            text: String::new(),
            state: MessageState::Streaming,
            model: Some(model.clone()),
            node: None,
            at_ms: now,
        });
        c.updated_ms = now;
        self.inner.history.save(&c)?;
        Ok(self.run(net, c.id, Request::messages(messages).with_model(model), String::new()))
    }

    /// Continues the cut-off answer at the end of `conversation` from where it stopped: the
    /// model gets the conversation with the partial answer and is asked to go on; the new text
    /// is appended to it (offsets continue after what was shown).
    ///
    /// agent 0.4's `Pool::infer` can't take a caller's `assistant_prefix` (it splices the
    /// node's offsets onto an empty buffer), so this asks in a turn rather than resuming at the
    /// node; noted for the Weirdnet lead.
    pub fn continue_answer(&self, conversation: &str, model: Option<String>) -> Result<ChatHandle> {
        let net = self.net()?;
        let mut c = self.inner.history.load(conversation)?;
        if !c.continuable() {
            return Err(CoreError::new(ErrorKind::BadRequest, "the last answer finished; nothing to continue"));
        }
        let n = c.messages.len();
        let last = &mut c.messages[n - 1];
        let model = match model.or_else(|| last.model.clone()) {
            Some(m) => m,
            None => self.pick_model(None)?,
        };
        let shown = last.text.clone();
        last.state = MessageState::Streaming;
        last.model = Some(model.clone());
        c.updated_ms = now_ms();
        let mut messages = agent_messages(&c.messages);
        if !shown.is_empty() {
            messages.push(Message::user(CONTINUE_PROMPT));
        }
        self.inner.history.save(&c)?;
        Ok(self.run(net, c.id, Request::messages(messages).with_model(model), shown))
    }

    fn run(&self, net: Arc<Net>, conversation: String, req: Request, shown: String) -> ChatHandle {
        let request_id = random_id("r");
        let cancel = CancelToken::new();
        lock(&self.inner.requests).insert(request_id.clone(), cancel.clone());
        let (out, events) = mpsc::unbounded_channel();
        let (core, rid, conv) = (self.clone(), request_id.clone(), conversation.clone());
        self.inner.handle.spawn(async move {
            let (result, text) = stream(&net.pool, req, &out, &cancel, &shown).await;
            let (state, node, model, last) = match &result {
                Ok(a) => (
                    MessageState::Complete,
                    Some(a.node.clone()),
                    Some(a.model.clone()).filter(|m| !m.is_empty()),
                    ChatEvent::Done { finish_reason: a.finish_reason.clone() },
                ),
                Err(i) => {
                    let e = CoreError::from(&i.error);
                    let state = if e.kind == ErrorKind::Cancelled { MessageState::Stopped } else { MessageState::Failed };
                    (state, None, None, ChatEvent::Error { kind: e.kind, message: e.message, partial: text.clone() })
                }
            };
            core.finish(&conv, text, state, node, model);
            let _ = out.send(last);
            lock(&core.inner.requests).remove(&rid);
        });
        ChatHandle { request_id, conversation_id: conversation, events }
    }

    fn finish(&self, conversation: &str, text: String, state: MessageState, node: Option<String>, model: Option<String>) {
        let Ok(mut c) = self.inner.history.load(conversation) else { return };
        if let Some(m) = c.messages.last_mut().filter(|m| m.role == Role::Assistant) {
            m.text = text;
            m.state = state;
            if node.is_some() {
                m.node = node;
            }
            if model.is_some() {
                m.model = model;
            }
        }
        c.updated_ms = now_ms();
        if let Err(e) = self.inner.history.save(&c) {
            tracing::warn!("saving {conversation}: {e}");
        }
    }

    /// Stops an answer (the node aborts generation). False when it already ended.
    pub fn cancel(&self, request_id: &str) -> bool {
        match lock(&self.inner.requests).get(request_id) {
            Some(c) => {
                c.cancel();
                true
            }
            None => false,
        }
    }

    // ---- history ---------------------------------------------------------------------------

    pub fn conversations(&self, offset: usize, limit: usize) -> Result<Vec<ConversationSummary>> {
        self.inner.history.page(offset, limit)
    }

    pub fn conversation(&self, id: &str) -> Result<Conversation> {
        self.inner.history.load(id)
    }

    pub fn delete_conversation(&self, id: &str) -> Result<()> {
        self.inner.history.delete(id)
    }

    // ---- lifecycle -------------------------------------------------------------------------

    /// The OS backgrounded or suspended the app.
    pub fn paused(&self) {
        if let Ok(n) = self.net() {
            let _ = n.node.pause();
        }
    }

    pub fn resumed(&self) {
        if let Ok(n) = self.net() {
            let _ = n.node.resume();
        }
    }

    pub fn network_changed(&self) {
        if let Ok(n) = self.net() {
            let _ = n.node.network_changed();
        }
    }

    /// This device's Weirdnet peer id, once linked.
    pub fn peer_id(&self) -> Option<String> {
        self.net().ok().map(|n| n.node.peer_id().to_string())
    }

    /// Stops the node and every answer.
    pub fn shutdown(&self) {
        for c in lock(&self.inner.requests).values() {
            c.cancel();
        }
        lock(&self.inner.net).take();
        self.inner.state.send_modify(|s| s.online = false);
    }
}

/// Earlier turns as the model sees them: finished and cut-off answers count, empty ones don't.
fn agent_messages(msgs: &[ChatMessage]) -> Vec<Message> {
    msgs.iter()
        .filter(|m| !(m.role == Role::Assistant && m.text.is_empty()))
        .map(|m| match m.role {
            Role::User => Message::user(m.text.clone()),
            Role::Assistant => Message::assistant(m.text.clone(), Vec::new()),
        })
        .collect()
}

/// One pool call, its events coalesced onto `out`. Returns the result and the full text shown
/// (`shown` plus what streamed).
async fn stream(
    pool: &Pool,
    req: Request,
    out: &mpsc::UnboundedSender<ChatEvent>,
    cancel: &CancelToken,
    shown: &str,
) -> (std::result::Result<weirdnet_agent::pool::Answer, Interrupted>, String) {
    let (tx, rx) = mpsc::unbounded_channel::<PoolEvent>();
    let offset = shown.chars().count() as u64;
    let co = tokio::spawn(coalesce(rx, out.clone(), offset));
    let result = pool.infer(req, &tx, cancel).await;
    drop(tx);
    let (streamed, _) = co.await.unwrap_or_default();
    (result, format!("{shown}{streamed}"))
}
