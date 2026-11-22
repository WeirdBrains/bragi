//! [`Core`]: the one client core. Both shells call these actions and draw [`AppState`] and the
//! [`ChatEvent`]s; nothing else decides behaviour.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::runtime::{Handle, Runtime};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use weirdnet_agent::pool::{Interrupted, LinkedNodes, NetDialer, NodeSource, Pool, PoolEvent, PoolOptions, encode_cert};
use weirdnet_agent::remote::catalog::Catalog;
use weirdnet_agent::remote::infer::v2::Request;
use weirdnet_agent::remote::{Dialer, RemoteError};
use weirdnet_agent::{CancelToken, Content, ContentPart, Message, Modalities};
use weirdnet_device_link::{HostNode, HostStatus, Pairing};
use weirdnet_net::{Keypair, Multiaddr, NodeConfig, PeerHint, PeerId};

use crate::attach::Attachment;
use crate::catalog::CatalogView;
use crate::chat::{ChatEvent, Streamed, coalesce};
use crate::error::{CoreError, ErrorKind, Result};
use crate::history::{ChatMessage, Conversation, ConversationSummary, Deleted, History, MessageState, Role, now_ms, title_for};
use crate::identity::{Identity, LinkSettings, SYNC_TYPE, SyncNode};

/// Bragi's link scheme (`weirdnet-bragi://link/…`). Never bare `weirdnet://`.
pub const SCHEME: &str = "weirdnet-bragi://";

/// Ymir's and Morph's link schemes. Their link codes carry the same payload as Bragi's.
const OTHER_SCHEMES: [&str; 2] = ["weirdnet-ymir://", "weirdnet-morph://"];

/// Reads a pasted code as a Bragi one: a `weirdnet-ymir://link/` or `weirdnet-morph://link/` code is
/// the same payload under another app's scheme, so a Bragi device can open codes shown by Ymir or
/// Morph. Anything else (invites, other schemes) is left for the link check to refuse.
pub fn normalize_link(code: &str) -> String {
    let code = code.trim();
    for other in OTHER_SCHEMES {
        if let Some(rest) = code.strip_prefix(other).filter(|r| r.starts_with("link/")) {
            return format!("{SCHEME}{rest}");
        }
    }
    code.to_string()
}

/// Node addresses this device keeps in its data folder, one per line: what links brought and
/// what the user added ([`Core::add_peer`]), dialed on every start.
pub const PEERS_FILE: &str = "peers.txt";

/// A peer id short enough for a status line: `12D3KooW…C5mz7`.
fn short_peer(peer: &str) -> String {
    if peer.len() <= 16 { peer.to_string() } else { format!("{}…{}", &peer[..8], &peer[peer.len() - 5..]) }
}

/// What the shells show once [`Core::add_node`] took an address.
pub const NODE_ADDED: &str = "Added the node's address; looking for it…";

/// Whether text pasted into Link a device is a node address (`/ip4/…/tcp/4848/p2p/…`, from
/// `ymirctl pool addresses`) rather than a link code: then it goes to [`Core::add_node`].
pub fn is_node_address(text: &str) -> bool {
    text.trim().starts_with('/')
}

/// The addresses in `dir`'s [`PEERS_FILE`] that parse (blank lines and `#` comments skipped).
pub(crate) fn load_peers(dir: &std::path::Path) -> Vec<String> {
    let text = std::fs::read_to_string(dir.join(PEERS_FILE)).unwrap_or_default();
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#') && hints(&[l.to_string()]).is_ok()).map(str::to_string).collect()
}

/// Adds the `new` addresses `list` doesn't have yet; returns whether any was added.
fn merge_peers(list: &mut Vec<String>, new: impl IntoIterator<Item = String>) -> bool {
    let mut added = false;
    for a in new {
        let a = a.trim().to_string();
        if !a.is_empty() && hints(std::slice::from_ref(&a)).is_ok() && !list.contains(&a) {
            list.push(a);
            added = true;
        }
    }
    added
}

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
    /// What the other device calls this one while linking (`Bragi on dev-box`).
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
    /// What became of the last node address added in Link a device.
    #[serde(default)]
    pub node_search: NodeSearch,
}

/// The last node address added with [`Core::add_node`]: looked for, found, or not found in
/// time. The shells show [`NodeSearch::line`]; any other user action clears it
/// ([`Core::dismiss_node_search`]), and a found node clears itself after a few seconds.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum NodeSearch {
    #[default]
    Idle,
    /// Dialing it; it shows up once it answers as an inference node.
    Looking { node: String },
    /// It answered: its models are in the list.
    Found { node: String },
    /// Not found in time (or not one of the user's devices); `message` says what to check.
    NotFound { node: String, message: String },
}

impl NodeSearch {
    /// The line to show, if any.
    pub fn line(&self) -> Option<String> {
        match self {
            NodeSearch::Idle => None,
            NodeSearch::Looking { node } => Some(format!("Looking for {node}…")),
            NodeSearch::Found { node } => Some(format!("Connected to {node}.")),
            NodeSearch::NotFound { message, .. } => Some(message.clone()),
        }
    }

    /// Whether the line reports a problem (the shells draw it as a warning, not progress).
    pub fn is_problem(&self) -> bool {
        matches!(self, NodeSearch::NotFound { .. })
    }
}

/// How long [`Core::add_node`] looks for the node before saying what to check.
pub const NODE_SEARCH_LIMIT: Duration = Duration::from_secs(30);
/// How long "Connected to …" stays up.
pub const NODE_FOUND_SHOWN: Duration = Duration::from_secs(6);

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

pub(crate) struct Net {
    pub(crate) identity: Arc<Identity>,
    pub(crate) node: Arc<SyncNode>,
    /// Finds and calls inference nodes: a catalog follower per node, a pool per request.
    pub(crate) nodes: Arc<LinkedNodes<Vec<u8>, Vec<u8>>>,
    pub(crate) dialer: Arc<dyn Dialer>,
    pub(crate) opts: PoolOptions,
    pub(crate) tasks: Vec<JoinHandle<()>>,
}

impl Drop for Net {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
        self.node.halt();
    }
}

pub(crate) struct Inner {
    pub(crate) cfg: Config,
    pub(crate) handle: Handle,
    pub(crate) history: Arc<History>,
    /// `cfg.peers`, and what [`Core::add_peer`] adds: where sync passes dial the other devices.
    pub(crate) peers: Arc<Mutex<Vec<String>>>,
    pub(crate) state: watch::Sender<AppState>,
    pub(crate) net: Mutex<Option<Arc<Net>>>,
    pub(crate) offer: Mutex<Option<Arc<HostNode>>>,
    pub(crate) pairing: Mutex<Option<Pairing>>,
    pub(crate) requests: Mutex<HashMap<String, CancelToken>>,
    /// Approval cards waiting for an answer, by request id (agent mode).
    pub(crate) cards: Mutex<HashMap<String, Arc<crate::agent::Cards>>>,
    /// The node search's generation (a newer search or a dismissal ends the older one) and limit.
    pub(crate) search: Mutex<(u64, Duration)>,
    /// Reactions added and removed, for whoever wants to forward them (see [`crate::reactions`]).
    pub(crate) reactions: tokio::sync::broadcast::Sender<crate::reactions::ReactionEvent>,
    /// The user's OpenAI-compatible endpoints and their sealed keys (LF-11).
    pub(crate) endpoints: crate::endpoints::Endpoints,
    /// Folders, and the target each conversation is bound to (LF-11).
    pub(crate) sessions: crate::sessions::Sessions,
}

/// The client core. Cheap to clone; every clone is the same core.
#[derive(Clone)]
pub struct Core {
    pub(crate) inner: Arc<Inner>,
    _runtime: Option<Arc<Runtime>>,
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn random_id(prefix: &str) -> String {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{prefix}-{hex}")
}

/// One pass against the other devices: pulls the roster and history, writes what was waiting
/// for the first key, and wakes the shells' watchers when history changed.
async fn sync_pass(
    identity: Arc<Identity>,
    node: Arc<SyncNode>,
    peers: Arc<Mutex<Vec<String>>>,
    history: Arc<History>,
    state: watch::Sender<AppState>,
) {
    let hints = hints(&lock(&peers)).unwrap_or_default();
    let h = history.clone();
    let changed = tokio::task::spawn_blocking(move || {
        let before = h.fingerprint();
        identity.tick(&node, &|c| hints.iter().find(|(p, _)| p.to_string() == c.peer_id).map(|(_, a)| a.clone()).unwrap_or_default());
        h.flush();
        let changed = h.fingerprint() != before;
        if changed {
            // A delete from another device: drop what it removed from this device's log too.
            h.compact();
        }
        changed
    })
    .await
    .unwrap_or(false);
    if changed {
        // Wakes the watchers; the state itself is the same.
        state.send_modify(|_| {});
    }
}

pub(crate) fn peer_hints(peers: &[String]) -> Result<Vec<(PeerId, Vec<Multiaddr>)>> {
    hints(peers)
}

pub(crate) fn check_peer(address: &str) -> Result<()> {
    hints(&[address.trim().to_string()]).map(|_| ())
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
        let dir = cfg.dir.clone();
        let history = Arc::new(History::open(&cfg.dir)?);
        let (state, _) = watch::channel(AppState::default());
        let mut known = cfg.peers.clone();
        if cfg.transport == Transport::Internet {
            merge_peers(&mut known, load_peers(&cfg.dir));
        }
        let peers = Arc::new(Mutex::new(known));
        let core = Core {
            inner: Arc::new(Inner {
                cfg,
                peers,
                handle,
                history,
                state,
                net: Mutex::new(None),
                offer: Mutex::new(None),
                pairing: Mutex::new(None),
                requests: Mutex::new(HashMap::new()),
                cards: Mutex::new(HashMap::new()),
                search: Mutex::new((0, NODE_SEARCH_LIMIT)),
                reactions: tokio::sync::broadcast::channel(64).0,
                endpoints: crate::endpoints::Endpoints::open(&dir),
                sessions: crate::sessions::Sessions::open(&dir),
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

    /// This device has been linked before (it goes online by itself on start).
    pub fn has_identity(&self) -> bool {
        Identity::exists(&self.identity_dir())
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

    pub(crate) fn net(&self) -> Result<Arc<Net>> {
        lock(&self.inner.net).clone().ok_or_else(|| CoreError::new(ErrorKind::NotPaired, "link this device to your Weirdnet first"))
    }

    /// Starts the Weirdnet node, the pool and the catalog for the stored identity.
    async fn go_online(&self) -> Result<()> {
        let identity =
            Arc::new(Identity::open(&self.identity_dir())?.ok_or_else(|| CoreError::new(ErrorKind::NotPaired, "this device is not linked"))?);
        let peers_now = lock(&self.inner.peers).clone();
        let ncfg = node_config(&Config { peers: peers_now.clone(), ..self.inner.cfg.clone() }, false)?;
        let (key, st) = (identity.node_key().clone(), identity.state());
        let node = tokio::task::spawn_blocking(move || weirdnet_sync::start_node(key, ncfg, st))
            .await
            .map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?
            .map_err(|e| CoreError::new(ErrorKind::Unavailable, format!("starting Weirdnet: {e}")))?;
        let node = Arc::new(node);
        let hints = hints(&peers_now)?;
        let dialer = NetDialer::new(node.clone());
        let mut nodes = LinkedNodes::new(node.clone(), identity.roster_view());
        for (peer, addrs) in &hints {
            dialer.hint(*peer, addrs.clone());
            nodes = nodes.with_hint(*peer, addrs.clone());
        }
        let nodes = Arc::new(nodes);
        let dialer: Arc<dyn Dialer> = Arc::new(dialer);
        let opts = PoolOptions::default().with_cert(Some(encode_cert(&identity.our_cert())));

        let mut tasks = Vec::new();
        // The catalog: every inference node's, followed, and merged.
        tasks.push(tokio::spawn(follow_catalogs(self.inner.state.clone(), nodes.clone(), dialer.clone(), opts.clone())));
        // The roster: devices linked or revoked elsewhere.
        // History rides along: the first pass after linking brings the key that seals it, and
        // what other devices wrote comes in; a shell is told to redraw when it did.
        let imported = self.inner.history.attach(identity.clone());
        if imported > 0 {
            tracing::info!("imported {imported} conversations from this device's earlier history");
        }
        let (net_for_loop, hist, state, peers) = (identity.clone(), self.inner.history.clone(), self.inner.state.clone(), self.inner.peers.clone());
        let node_for_loop = node.clone();
        tasks.push(tokio::spawn(async move {
            loop {
                sync_pass(net_for_loop.clone(), node_for_loop.clone(), peers.clone(), hist.clone(), state.clone()).await;
                tokio::time::sleep(SYNC_EVERY).await;
            }
        }));
        let (user, device) = (identity.user_id(), identity.our_cert().label);
        *lock(&self.inner.net) = Some(Arc::new(Net { identity, node, nodes, dialer, opts, tasks }));
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
                    let adopted = host.take().map_err(|e| CoreError::new(ErrorKind::Pairing, e.to_string())).and_then(|t| {
                        let id = Identity::adopt(&self.identity_dir(), key.clone(), &t.seed, t.node_cert.clone(), &t.settings)?;
                        // Where the user's nodes are, from the device that linked this one: without
                        // it a device on another network than its nodes never finds them.
                        self.remember_peers(LinkSettings::peers_in(&t.settings));
                        Ok(id)
                    });
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
        let (cfg, link) = (node_config(&self.inner.cfg, true)?, normalize_link(link));
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
        let mut peers = net.node.reachable_addrs();
        merge_peers(&mut peers, lock(&self.inner.peers).clone());
        let settings = net.identity.link_settings_with(peers);
        let cert = tokio::task::spawn_blocking(move || pairing.send_with(&seed, "Bragi", &settings, &[SYNC_TYPE.to_string()]))
            .await
            .map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?
            .map_err(|e| CoreError::new(ErrorKind::Pairing, e.to_string()))?;
        net.identity.approve(cert)?;
        Ok(())
    }

    // ---- chat ------------------------------------------------------------------------------

    /// The model for a turn: the one the user picked, else the first warm one that reads `needed`
    /// (images, audio). A picked model the catalog says can't read them is refused here, with the
    /// models that can in the message, instead of failing in the pool.
    pub(crate) fn pick_model(&self, model: Option<String>, needed: Modalities) -> Result<String> {
        let catalog = self.state().catalog;
        match model {
            Some(m) if !m.is_empty() => match catalog.models.iter().find(|i| i.id == m) {
                Some(info) if !info.takes(needed) => Err(CoreError::new(ErrorKind::BadRequest, catalog.refusal(&m, needed))),
                _ => Ok(m),
            },
            _ => catalog.default_model_for(needed).map(str::to_string).ok_or_else(|| match catalog.models.is_empty() {
                true => CoreError::new(ErrorKind::Unavailable, self.no_node_message()),
                false => CoreError::new(ErrorKind::BadRequest, "none of the models on your nodes can read images or audio"),
            }),
        }
    }

    /// Sends `prompt` in `conversation` (a new one when `None`) to `model` (the default when
    /// `None`), streaming the answer.
    pub fn send(&self, conversation: Option<&str>, prompt: &str, model: Option<String>) -> Result<ChatHandle> {
        self.send_with(conversation, prompt, model, Vec::new())
    }

    /// [`send`](Self::send) with text files attached (see [`Attachment`]): the model gets each as
    /// a named block ahead of the prompt, and the message keeps them for later turns and other devices.
    pub fn send_with(&self, conversation: Option<&str>, prompt: &str, model: Option<String>, attachments: Vec<Attachment>) -> Result<ChatHandle> {
        self.send_to(conversation, prompt, model, attachments, None)
    }

    /// One answer to `messages` that is kept out of history (code suggestions and rewrites):
    /// the same streaming, routing and cancel as a chat, with an empty `conversation_id`, sent to
    /// the default target ([`complete_to`](Self::complete_to) picks one).
    pub fn complete(&self, messages: Vec<crate::code::PromptMessage>, model: Option<String>) -> Result<ChatHandle> {
        self.complete_to(messages, model, None)
    }

    /// Saves the user's message and an empty streaming answer in `conversation` (a new one when
    /// `None`), and returns its id and the messages the model gets (the new user message last).
    pub(crate) fn begin_turn(
        &self,
        conversation: Option<&str>,
        prompt: &str,
        model: &str,
        attachments: Vec<Attachment>,
    ) -> Result<(String, Vec<Message>)> {
        crate::attach::check(&attachments)?;
        let prompt = prompt.trim();
        if prompt.is_empty() && !attachments.iter().any(|a| a.media.is_some()) {
            return Err(CoreError::new(ErrorKind::BadRequest, "type a message first"));
        }
        let now = now_ms();
        let mut c = match conversation {
            Some(id) => self.inner.history.load(id)?,
            None => Conversation {
                id: random_id("c"),
                title: title_for(if prompt.is_empty() { attachments.first().map_or("New chat", |a| a.name.as_str()) } else { prompt }),
                created_ms: now,
                updated_ms: now,
                messages: Vec::new(),
            },
        };
        c.messages.push(ChatMessage {
            id: String::new(),
            role: Role::User,
            text: prompt.into(),
            reasoning: String::new(),
            thinking_ms: None,
            state: MessageState::Complete,
            model: None,
            node: None,
            cache_hit: false,
            standin: None,
            resumes: 0,
            attachments,
            tools: Vec::new(),
            agent: false,
            reactions: Vec::new(),
            at_ms: now,
        });
        let messages = agent_messages(&c.messages);
        c.messages.push(ChatMessage {
            id: String::new(),
            role: Role::Assistant,
            text: String::new(),
            reasoning: String::new(),
            thinking_ms: None,
            state: MessageState::Streaming,
            model: Some(model.to_string()),
            node: None,
            cache_hit: false,
            standin: None,
            resumes: 0,
            attachments: Vec::new(),
            tools: Vec::new(),
            agent: false,
            reactions: Vec::new(),
            at_ms: now,
        });
        c.updated_ms = now;
        self.inner.history.save(&c)?;
        Ok((c.id, messages))
    }

    /// Continues the cut-off answer at the end of `conversation` from where it stopped: the
    /// model gets the conversation with the partial answer as an assistant prefix
    /// (`Request::continuing`) and carries on from it; the new text is appended to it (offsets
    /// continue after what was shown). A node that can't continue a prefix is skipped.
    pub fn continue_answer(&self, conversation: &str, model: Option<String>) -> Result<ChatHandle> {
        let net = self.net()?;
        let mut c = self.inner.history.load(conversation)?;
        if !c.continuable() {
            return Err(CoreError::new(ErrorKind::BadRequest, "the last answer finished; nothing to continue"));
        }
        let n = c.messages.len();
        let needed = conversation_needs(&c.messages);
        let last = &mut c.messages[n - 1];
        let model = match model.or_else(|| last.model.clone()) {
            Some(m) => m,
            None => self.pick_model(None, needed)?,
        };
        let shown = last.text.clone();
        let shown_reasoning = last.reasoning.clone();
        last.state = MessageState::Streaming;
        last.model = Some(model.clone());
        c.updated_ms = now_ms();
        let messages = agent_messages(&c.messages);
        self.inner.history.save(&c)?;
        // The partial answer is the last message: `continuing` moves it into the prefix.
        Ok(self.run(net, c.id, Request::messages(messages).with_model(model).continuing(), shown, shown_reasoning, None))
    }

    /// Streams `req` into the last message of `conversation`. `shown` and `shown_reasoning` are
    /// what that message already holds (a continued answer); the new text and reasoning follow them.
    pub(crate) fn run(
        &self,
        net: Arc<Net>,
        conversation: String,
        req: Request,
        shown: String,
        shown_reasoning: String,
        fallback: Option<Vec<Message>>,
    ) -> ChatHandle {
        let request_id = random_id("r");
        let cancel = CancelToken::new();
        lock(&self.inner.requests).insert(request_id.clone(), cancel.clone());
        let (out, events) = mpsc::unbounded_channel();
        let (core, rid, conv) = (self.clone(), request_id.clone(), conversation.clone());
        self.inner.handle.spawn(async move {
            let order = |model: &str| core.nodes_for(model);
            let (result, streamed) = stream(&net, &order, req, &out, &cancel, &shown, shown_reasoning.chars().count() as u64).await;
            // The pool couldn't start the answer: an enabled fallback endpoint takes over.
            if let (Err(i), Some(messages)) = (&result, fallback) {
                let e = CoreError::from(&i.error);
                let fallbacks = core.inner.endpoints.fallbacks();
                if !i.streamed() && streamed.text.is_empty() && e.kind != ErrorKind::Cancelled && !fallbacks.is_empty() {
                    let _ = out.send(ChatEvent::Retry {
                        attempt: 1,
                        message: format!("The pool couldn't answer ({}); trying a fallback endpoint", e.message),
                    });
                    core.answer_from_endpoints(&conv, &rid, fallbacks, None, messages, &cancel, &out, shown).await;
                    return;
                }
            }
            let text = format!("{shown}{}", streamed.text);
            let (state, node, model, last, flags) = match &result {
                Ok(a) => (
                    MessageState::Complete,
                    Some(a.node.clone()),
                    // A stand-in answered: the message stays the requested model's, flagged.
                    Some(a.model.clone()).filter(|m| !m.is_empty() && a.meta.standin.is_none()),
                    ChatEvent::Done { finish_reason: a.finish_reason.clone() },
                    Flags::of(&a.meta),
                ),
                Err(i) => {
                    let e = CoreError::from(&i.error);
                    let state = if e.kind == ErrorKind::Cancelled { MessageState::Stopped } else { MessageState::Failed };
                    (state, None, None, ChatEvent::Error { kind: e.kind, message: e.message, partial: text.clone() }, Flags::of(&i.partial.meta))
                }
            };
            if !conv.is_empty() {
                core.finish(&conv, text, &streamed, state, node, model, flags);
            }
            // Over before the shell hears it: a `cancel` after the last event must say so.
            lock(&core.inner.requests).remove(&rid);
            let _ = out.send(last);
        });
        ChatHandle { request_id, conversation_id: conversation, events }
    }

    /// The nodes to ask for `model`, best first; every known node when the catalog doesn't list it.
    fn nodes_for(&self, model: &str) -> Vec<String> {
        let c = self.state().catalog;
        let n = c.nodes_for(model);
        if n.is_empty() { c.nodes.iter().map(|n| n.id.clone()).collect() } else { n }
    }

    /// [`finish`](Self::finish) for an answer that didn't come from the pool (no cache or stand-in flags).
    pub(crate) fn finish_turn(
        &self,
        conversation: &str,
        text: String,
        streamed: &Streamed,
        state: MessageState,
        node: Option<String>,
        model: Option<String>,
    ) {
        self.finish(conversation, text, streamed, state, node, model, Flags::default());
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        conversation: &str,
        text: String,
        streamed: &Streamed,
        state: MessageState,
        node: Option<String>,
        model: Option<String>,
        flags: Flags,
    ) {
        let Ok(mut c) = self.inner.history.load(conversation) else { return };
        // The answer was deleted while it streamed: nothing to write (and not over another one).
        let Some(m) = c.messages.last_mut().filter(|m| m.role == Role::Assistant && m.state == MessageState::Streaming) else { return };
        {
            m.text = text;
            // Reasoning is kept with the answer (a continued one adds to it), not sent to the model.
            m.reasoning.push_str(&streamed.reasoning);
            if let Some(ms) = streamed.thinking_ms {
                m.thinking_ms = Some(m.thinking_ms.unwrap_or(0) + ms);
            }
            m.state = state;
            if node.is_some() {
                m.node = node;
            }
            if model.is_some() {
                m.model = model;
            }
            m.cache_hit = flags.cache_hit;
            m.standin = flags.standin;
            m.resumes = flags.resumes;
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

    /// Toggles `emoji` on a message of a conversation: added if absent, removed if present.
    /// Returns the message's reactions afterwards. Saved to history (so it syncs) and sent to
    /// [`Core::reaction_events`]; the next turn's context carries it as a short note.
    pub fn react(&self, conversation: &str, message: &str, emoji: &str) -> Result<Vec<String>> {
        let emoji = crate::reactions::check(emoji)?;
        let mut c = self.inner.history.load(conversation)?;
        let m =
            c.messages.iter_mut().find(|m| m.id == message).ok_or_else(|| CoreError::new(ErrorKind::NotFound, format!("no message `{message}`")))?;
        if m.state == MessageState::Streaming {
            return Err(CoreError::new(ErrorKind::Refused, "wait for the answer to finish"));
        }
        let on = crate::reactions::toggle(&mut m.reactions, &emoji)?;
        let event = crate::reactions::ReactionEvent {
            conversation: c.id.clone(),
            message: m.id.clone(),
            role: m.role,
            model: m.model.clone(),
            node: m.node.clone(),
            emoji: emoji.clone(),
            on,
            feedback: crate::reactions::Feedback::of(&emoji),
            at_ms: now_ms(),
        };
        let now = m.reactions.clone();
        self.inner.history.save(&c)?;
        let _ = self.inner.reactions.send(event);
        Ok(now)
    }

    /// The reaction feed: one event per add or remove. This is the hook a shell or a bridge uses
    /// to forward 👍/👎 as feedback (Ymir has no feedback path yet, so nothing is sent today).
    pub fn reaction_events(&self) -> tokio::sync::broadcast::Receiver<crate::reactions::ReactionEvent> {
        self.inner.reactions.subscribe()
    }

    pub fn delete_conversation(&self, id: &str) -> Result<()> {
        self.delete_conversations(&[id.to_string()]).map(|_| ())
    }

    /// Deletes conversations (see [`History::delete_conversations`]): gone on every linked device
    /// after the next sync pass, and from this device's store now. [`Core::undo_delete`] takes it
    /// back for [`UNDO_MS`](crate::history::UNDO_MS).
    pub fn delete_conversations(&self, ids: &[String]) -> Result<Deleted> {
        let d = self.inner.history.delete_conversations(ids)?;
        self.after_delete(&d);
        Ok(d)
    }

    /// Deletes message `id` of `conversation`, and with `with_reply` the answer that followed it
    /// (see [`Conversation::reply_to`]). An answer still streaming is stopped first by the shell;
    /// here it is refused.
    pub fn delete_message(&self, conversation: &str, id: &str, with_reply: bool) -> Result<Deleted> {
        let c = self.inner.history.load(conversation)?;
        let mut ids = vec![id.to_string()];
        if with_reply && let Some(r) = c.reply_to(id) {
            ids.push(r.id.clone());
        }
        let busy = !lock(&self.inner.requests).is_empty();
        if busy && c.messages.iter().any(|m| ids.contains(&m.id) && m.state == MessageState::Streaming) {
            return Err(CoreError::new(ErrorKind::BadRequest, "stop the answer before deleting it"));
        }
        let d = self.inner.history.delete_messages(conversation, &ids)?;
        self.after_delete(&d);
        Ok(d)
    }

    /// Puts back the last delete, within [`UNDO_MS`](crate::history::UNDO_MS) of it.
    pub fn undo_delete(&self, undo: &str) -> Result<()> {
        self.inner.history.undo(undo)?;
        self.inner.state.send_modify(|_| {});
        Ok(())
    }

    /// Deletes all history, on every linked device, with no undo. Returns how many conversations.
    pub fn clear_history(&self) -> Result<usize> {
        let n = self.inner.history.clear()?;
        self.inner.state.send_modify(|_| {});
        Ok(n)
    }

    /// Redraws, and drops the undo copy when its time is up.
    fn after_delete(&self, d: &Deleted) {
        self.inner.state.send_modify(|_| {});
        let (h, token) = (self.inner.history.clone(), d.undo.clone());
        self.inner.handle.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(crate::history::UNDO_MS)).await;
            h.forget_undo(&token);
        });
    }

    // ---- lifecycle -------------------------------------------------------------------------

    /// The OS backgrounded or suspended the app.
    /// One sync pass with the user's other devices now (history, the roster), instead of
    /// waiting for the next one. Shells call it on resume; it returns when the pass is over.
    pub async fn sync_now(&self) -> Result<()> {
        let net = self.net()?;
        sync_pass(net.identity.clone(), net.node.clone(), self.inner.peers.clone(), self.inner.history.clone(), self.inner.state.clone()).await;
        Ok(())
    }

    /// Where other devices can dial this one (`/ip4/…/p2p/PEER`), once the node listens.
    pub fn address(&self) -> Option<String> {
        let net = self.net().ok()?;
        let a = net.node.status().listening.first()?.clone();
        Some(format!("{}/p2p/{}", a.split("/p2p/").next().unwrap_or(&a), net.node.peer_id()))
    }

    /// Tells sync passes where another of the user's devices can be dialed, for networks where
    /// finding it by id doesn't work (a LAN without mDNS, the memory transport in tests).
    ///
    /// Also where inference nodes are found: the address is kept in the data folder
    /// ([`PEERS_FILE`]) and, once online, dialed at once, so a node that discovery can't find (another
    /// network, mDNS and the DHT off) shows up without a restart.
    pub fn add_peer(&self, address: &str) -> Result<()> {
        let address = address.trim();
        let hint = PeerHint::from_addr(address).map_err(|e| CoreError::new(ErrorKind::BadRequest, format!("peer `{address}`: {e}")))?;
        hints(&[address.to_string()])?;
        self.remember_peers(vec![address.to_string()]);
        if let Ok(net) = self.net() {
            let node = net.node.clone();
            self.inner.handle.spawn_blocking(move || {
                if let Err(e) = node.add_peer(hint) {
                    tracing::warn!("dialing an added node: {e}");
                }
            });
        }
        Ok(())
    }

    /// The Link a device box on every shell: a node address (see [`is_node_address`]) is added
    /// with [`add_peer`](Self::add_peer) and dialed at once. Returns the line to show the user.
    ///
    /// Progress goes to [`AppState::node_search`]: `Looking`, then `Found` once the node
    /// answers as an inference node (cleared after [`NODE_FOUND_SHOWN`]), or `NotFound` with what
    /// to check after [`NODE_SEARCH_LIMIT`].
    pub fn add_node(&self, address: &str) -> Result<String> {
        self.add_peer(address)?;
        let peer = PeerHint::from_addr(address.trim()).map(|h| h.peer_id).unwrap_or_default();
        let (generation, limit) = {
            let mut s = lock(&self.inner.search);
            s.0 += 1;
            *s
        };
        let roster = self.net().ok().map(|n| (n.identity.roster(), n.node.peer_id().to_string()));
        let linked = roster.as_ref().and_then(|(r, _)| r.active().find(|c| c.peer_id == peer).map(|c| c.label.clone()));
        let node = linked.clone().unwrap_or_else(|| short_peer(&peer));
        let search = match (&roster, linked) {
            (Some((_, me)), _) if *me == peer => NodeSearch::NotFound {
                node: node.clone(),
                message: "That address is this device's own; paste a Ymir node's address (`ymirctl pool addresses` on the node).".into(),
            },
            (Some(_), None) => NodeSearch::NotFound {
                node: node.clone(),
                message: format!(
                    "{node} isn't one of your linked devices. Link it first (on a linked Ymir node: `ymirctl pool add` with its code), then add its address."
                ),
            },
            _ => NodeSearch::Looking { node: node.clone() },
        };
        let looking = matches!(search, NodeSearch::Looking { .. });
        self.set_node_search(search);
        if looking {
            let core = self.clone();
            self.inner.handle.spawn(async move { core.follow_node_search(generation, peer, node, limit).await });
        }
        Ok(NODE_ADDED.to_string())
    }

    /// Waits for `peer` to show up among the inference nodes (or for the limit), as long as no
    /// newer search or dismissal replaced this one.
    async fn follow_node_search(&self, generation: u64, peer: String, node: String, limit: Duration) {
        let current = |c: &Core| lock(&c.inner.search).0 == generation;
        let mut w = self.inner.state.subscribe();
        let deadline = tokio::time::Instant::now() + limit;
        let found = loop {
            if !current(self) {
                return;
            }
            if w.borrow_and_update().catalog.nodes.iter().any(|n| n.id == peer) {
                break true;
            }
            tokio::select! {
                r = w.changed() => if r.is_err() { return },
                _ = tokio::time::sleep_until(deadline) => break false,
            }
        };
        if !current(self) {
            return;
        }
        if found {
            self.set_node_search(NodeSearch::Found { node });
            tokio::time::sleep(NODE_FOUND_SHOWN).await;
            if current(self) {
                self.set_node_search(NodeSearch::Idle);
            }
        } else {
            self.set_node_search(NodeSearch::NotFound {
                node: node.clone(),
                message: format!(
                    "Couldn't reach {node} at that address yet. Check that ymird runs there with the inference role (`ymirctl status`), that the address is one `ymirctl pool addresses` prints, and that this device can reach it. Bragi keeps trying."
                ),
            });
        }
    }

    fn set_node_search(&self, search: NodeSearch) {
        self.inner.state.send_if_modified(|s| {
            let changed = s.node_search != search;
            s.node_search = search;
            changed
        });
    }

    /// The user did something else: the node search's line goes away (the address stays).
    pub fn dismiss_node_search(&self) {
        lock(&self.inner.search).0 += 1;
        self.set_node_search(NodeSearch::Idle);
    }

    /// How long [`add_node`](Self::add_node) looks before saying what to check (default
    /// [`NODE_SEARCH_LIMIT`]; tests and demos shorten it).
    pub fn set_node_search_limit(&self, limit: Duration) {
        lock(&self.inner.search).1 = limit;
    }

    /// Adds `addresses` to the ones sync passes and the node dial, and keeps them in
    /// [`PEERS_FILE`] (real network only).
    fn remember_peers(&self, addresses: Vec<String>) {
        let all = {
            let mut p = lock(&self.inner.peers);
            if !merge_peers(&mut p, addresses) {
                return;
            }
            p.clone()
        };
        if self.inner.cfg.transport == Transport::Internet {
            let mut kept = load_peers(&self.inner.cfg.dir);
            merge_peers(&mut kept, all);
            let text: String = kept.iter().map(|a| format!("{a}\n")).collect();
            if let Err(e) = std::fs::create_dir_all(&self.inner.cfg.dir).and_then(|_| std::fs::write(self.inner.cfg.dir.join(PEERS_FILE), text)) {
                tracing::warn!("saving node addresses: {e}");
            }
        }
    }

    /// The node addresses this device dials (from its config, links and [`add_peer`](Self::add_peer)).
    pub fn peers(&self) -> Vec<String> {
        lock(&self.inner.peers).clone()
    }

    /// What to tell the user when no inference node has answered: how many linked devices there
    /// are, and what to do when the nodes are on another network.
    pub fn no_node_message(&self) -> String {
        let Ok(net) = self.net() else {
            return "link this device to your Weirdnet first".into();
        };
        let me = net.node.peer_id().to_string();
        let others = net.identity.roster().active().filter(|c| c.peer_id != me).count();
        let fix = "Bragi finds Ymir nodes on this local network, or at an address you give it: run `ymirctl pool addresses` on a Ymir node and paste one into Link a device.";
        match others {
            0 => format!("No inference node yet: this is the only device in your Weirdnet. Link a Ymir node first. {fix}"),
            n => format!(
                "No inference node reachable yet: {n} other linked device{}, none answering as one so far. {fix}",
                if n == 1 { "" } else { "s" }
            ),
        }
    }

    pub fn paused(&self) {
        if let Ok(n) = self.net() {
            let _ = n.node.pause();
        }
    }

    pub fn resumed(&self) {
        if let Ok(n) = self.net() {
            let _ = n.node.resume();
            let core = self.clone();
            self.inner.handle.spawn(async move {
                let _ = core.sync_now().await;
            });
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
/// Reasoning stays out: models expect only the answers of earlier turns, and it would fill the context.
pub(crate) fn agent_messages(msgs: &[ChatMessage]) -> Vec<Message> {
    msgs.iter()
        .filter(|m| !(m.role == Role::Assistant && m.text.is_empty()))
        .map(|m| match m.role {
            Role::User => Message::user(user_content(m)),
            Role::Assistant => Message::assistant(crate::reactions::with_note(m, m.text.clone()), Vec::new()),
        })
        .collect()
}

/// What a model needs to read the whole conversation: text unless some message has images or audio.
fn conversation_needs(msgs: &[ChatMessage]) -> Modalities {
    msgs.iter().fold(Modalities::TEXT, |n, m| n.union(crate::attach::needs(&m.attachments)))
}

/// A user message as the model gets it: text, or text and parts when images or sound are attached.
/// Reactions go in as a note at the end of the text.
fn user_content(m: &ChatMessage) -> Content {
    match crate::attach::content(&m.text, &m.attachments) {
        Content::Text(t) => Content::Text(crate::reactions::with_note(m, t)),
        Content::Parts(mut parts) => {
            if let Some(ContentPart::Text { text }) = parts.first_mut() {
                *text = crate::reactions::with_note(m, std::mem::take(text));
            } else if !m.reactions.is_empty() {
                parts.insert(0, ContentPart::text(crate::reactions::with_note(m, String::new()).trim_start().to_string()));
            }
            Content::Parts(parts)
        }
    }
}

/// What the pool said about an answer, kept with the message.
#[derive(Default)]
struct Flags {
    cache_hit: bool,
    standin: Option<String>,
    resumes: u32,
}

impl Flags {
    fn of(m: &weirdnet_agent::remote::infer::v2::Meta) -> Self {
        Flags { cache_hit: m.cache_hit, standin: m.standin.clone(), resumes: m.resumes.len() as u32 }
    }
}

/// How often the pool is asked again after an attempt fails before any text, and the waits.
const RETRY_WAITS: [Duration; 2] = [Duration::from_millis(400), Duration::from_millis(1200)];

/// Whether another try could work: the nodes were unreachable, busy or failed, not "no such model"
/// or a refusal.
fn worth_retrying(e: &RemoteError) -> bool {
    matches!(
        e,
        RemoteError::Unreachable { .. }
            | RemoteError::Busy
            | RemoteError::Failed { .. }
            | RemoteError::Closed
            | RemoteError::Timeout
            | RemoteError::Stalled { .. }
            | RemoteError::Unavailable { .. }
    )
}

/// One answer: a pool over the nodes that serve the model (best first), which fails over between
/// them and resumes mid-answer; if every node fails before any text, tries again after a short
/// wait with a fresh node order. Events are coalesced onto `out` (reasoning offsets start at
/// `shown_reasoning`). Returns the result and what streamed after `shown`.
async fn stream(
    net: &Net,
    order: &(dyn Fn(&str) -> Vec<String> + Send + Sync),
    req: Request,
    out: &mpsc::UnboundedSender<ChatEvent>,
    cancel: &CancelToken,
    shown: &str,
    shown_reasoning: u64,
) -> (std::result::Result<weirdnet_agent::pool::Answer, Interrupted>, Streamed) {
    let offset = shown.chars().count() as u64;
    let mut attempt = 0usize;
    loop {
        let candidates = req.model.as_deref().map(order).unwrap_or_default();
        let pool = if candidates.is_empty() {
            Pool::new(net.dialer.clone(), SharedNodes(net.nodes.clone()))
        } else {
            Pool::new(net.dialer.clone(), candidates)
        }
        .with_options(net.opts.clone());
        let (tx, rx) = mpsc::unbounded_channel::<PoolEvent>();
        let co = tokio::spawn(coalesce(rx, out.clone(), offset, shown_reasoning));
        let result = pool.infer(req.clone(), &tx, cancel).await;
        drop(tx);
        let streamed = co.await.unwrap_or_default();
        match &result {
            Err(i) if !i.streamed() && attempt < RETRY_WAITS.len() && worth_retrying(&i.error) && !cancel.is_cancelled() => {
                attempt += 1;
                let _ = out.send(ChatEvent::Retry { attempt: attempt as u32, message: i.error.to_string() });
                tokio::select! {
                    _ = tokio::time::sleep(RETRY_WAITS[attempt - 1]) => {}
                    _ = cancel.cancelled() => {}
                }
            }
            _ => return (result, streamed),
        }
    }
}

/// Follows every inference node's catalog: a watcher per node that the roster lists and that
/// answers, merged into [`AppState::catalog`]. Nodes that go away drop out within a few seconds.
async fn follow_catalogs(state: watch::Sender<AppState>, nodes: Arc<LinkedNodes<Vec<u8>, Vec<u8>>>, dialer: Arc<dyn Dialer>, opts: PoolOptions) {
    let feeds: Arc<Mutex<BTreeMap<String, Catalog>>> = Arc::default();
    let mut watchers: HashMap<String, JoinHandle<()>> = HashMap::new();
    let publish = {
        let (feeds, state) = (feeds.clone(), state.clone());
        move || {
            let all: Vec<(String, Catalog)> = lock(&feeds).iter().map(|(n, c)| (n.clone(), c.clone())).collect();
            let view = CatalogView::merge(&all);
            state.send_if_modified(|s| {
                let changed = s.catalog != view;
                if changed {
                    s.catalog = view;
                }
                changed
            });
        }
    };
    loop {
        let listed = nodes.nodes().await;
        for gone in watchers.keys().filter(|n| !listed.contains(n)).cloned().collect::<Vec<_>>() {
            if let Some(w) = watchers.remove(&gone) {
                w.abort();
            }
            lock(&feeds).remove(&gone);
        }
        for node in listed {
            if watchers.contains_key(&node) {
                continue;
            }
            let (feeds, publish, n) = (feeds.clone(), publish.clone(), node.clone());
            let pool = Pool::new(dialer.clone(), vec![node.clone()]).with_options(opts.clone());
            watchers.insert(
                node,
                tokio::spawn(async move {
                    let mut w = pool.subscribe();
                    while let Some((_, c)) = w.changed().await {
                        lock(&feeds).insert(n.clone(), c);
                        publish();
                    }
                }),
            );
        }
        publish();
        tokio::time::sleep(NODE_POLL).await;
    }
}

/// The roster's inference nodes as a pool's node source.
struct SharedNodes(Arc<LinkedNodes<Vec<u8>, Vec<u8>>>);

#[async_trait::async_trait]
impl weirdnet_agent::pool::NodeSource for SharedNodes {
    async fn nodes(&self) -> Vec<String> {
        self.0.nodes().await
    }
}

/// How often the roster is checked for inference nodes that came or went.
const NODE_POLL: Duration = Duration::from_secs(2);

#[cfg(test)]
mod link_tests {
    use super::normalize_link;

    #[test]
    fn other_apps_link_codes_read_as_bragi_codes() {
        assert_eq!(normalize_link(" weirdnet-ymir://link/abc\n"), "weirdnet-bragi://link/abc");
        assert_eq!(normalize_link("weirdnet-morph://link/abc"), "weirdnet-bragi://link/abc");
        assert_eq!(normalize_link("weirdnet-bragi://link/abc"), "weirdnet-bragi://link/abc");
        // Only link codes convert; the rest fails the link check as before.
        assert_eq!(normalize_link("weirdnet-ymir://invite/abc"), "weirdnet-ymir://invite/abc");
        assert_eq!(normalize_link("weirdnet://link/abc"), "weirdnet://link/abc");
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;

    #[test]
    fn reasoning_stays_out_of_the_context() {
        let mut answer = ChatMessage {
            id: String::new(),
            role: Role::Assistant,
            text: "391".into(),
            reasoning: "17*20 + 17*3".into(),
            thinking_ms: Some(1200),
            state: MessageState::Complete,
            model: None,
            node: None,
            cache_hit: false,
            standin: None,
            resumes: 0,
            attachments: Vec::new(),
            tools: Vec::new(),
            agent: false,
            reactions: Vec::new(),
            at_ms: 0,
        };
        let question = ChatMessage { role: Role::User, text: "17*23?".into(), reasoning: String::new(), thinking_ms: None, ..answer.clone() };
        let sent = agent_messages(&[question.clone(), answer.clone()]);
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1].content.text(), "391");
        // An answer that only reasoned before it was stopped isn't sent at all.
        answer.text.clear();
        assert_eq!(agent_messages(&[question, answer]).len(), 1);
    }
}

#[cfg(test)]
mod reaction_context_tests {
    use super::*;

    fn msg(role: Role, text: &str, reactions: &[&str]) -> ChatMessage {
        ChatMessage {
            id: "m".into(),
            role,
            text: text.into(),
            reasoning: String::new(),
            thinking_ms: None,
            state: MessageState::Complete,
            model: None,
            node: None,
            cache_hit: false,
            standin: None,
            resumes: 0,
            attachments: Vec::new(),
            tools: Vec::new(),
            agent: false,
            reactions: reactions.iter().map(|s| s.to_string()).collect(),
            at_ms: 1,
        }
    }

    #[test]
    fn reactions_reach_the_model_as_a_note_and_leave_when_removed() {
        let mut v = vec![msg(Role::User, "hi", &[]), msg(Role::Assistant, "hello", &["👍", "🎉"])];
        let m = agent_messages(&v);
        assert_eq!(m[0].content.text(), "hi");
        assert_eq!(m[1].content.text(), "hello\n\n[The user reacted to this reply with 👍 🎉]");
        v[1].reactions.clear();
        assert_eq!(agent_messages(&v)[1].content.text(), "hello");
        v[0].reactions.push("🤔".into());
        assert_eq!(agent_messages(&v)[0].content.text(), "hi\n\n[The user reacted to their own message with 🤔]");
    }
}
