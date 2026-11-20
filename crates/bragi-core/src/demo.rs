//! A stand-in for the user's other devices, in process on the memory transport: an inference
//! node with a test identity, a fake [`InferHost`] with canned answers, and the "existing
//! device" side of linking. The tests, and both shells' demo profile (screenshots, the shared
//! end-to-end scenario), use it; it never touches the real network or a real identity.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use weirdnet_agent::model::ModelBackend;
use weirdnet_agent::remote::catalog::{CatalogFeed, CatalogModel, ModelState, Purpose};
use weirdnet_agent::remote::infer::v2::{Finish, InferBackend, Meta, Output, Request, Usage};
use weirdnet_agent::remote::infer::{Admission, InferHost};
use weirdnet_agent::serve::{RosterAuth, Server, Serving};
use weirdnet_agent::{Authorize, Caller, CancelToken, RemoteError, Role};
use weirdnet_device_link::Pairing;
use weirdnet_net::{Keypair, WEIRDNET};
use weirdnet_sync::Roster;

use crate::core::{Config, Core, PairState, SCHEME, Transport};
use crate::error::{CoreError, ErrorKind, Result};
use crate::identity::{Identity, SYNC_TYPE, SyncNode};

/// The demo's models: one warm, one that would have to load.
pub const WARM_MODEL: &str = "gemma-4-e4b";
pub const COLD_MODEL: &str = "qwen3-32b";
/// A second node's own model (the cloud demo).
pub const BIG_MODEL: &str = "llama-3.3-70b";

/// What a demo node serves.
#[derive(Debug, Clone)]
pub struct NodeSpec {
    pub label: String,
    pub models: Vec<(String, ModelState)>,
    pub embedding: bool,
    /// A model that isn't warm here is answered by a warm one, flagged (else it just answers).
    pub standins: bool,
}

impl NodeSpec {
    /// The single-node demo: like [`primary`](Self::primary), where the cold model answers itself.
    pub fn single() -> Self {
        Self { standins: false, ..Self::primary() }
    }

    /// The first node: [`WARM_MODEL`] warm, [`COLD_MODEL`] to load, and an embedding model.
    pub fn primary() -> Self {
        Self {
            label: "demo inference node".into(),
            models: vec![(WARM_MODEL.into(), ModelState::Warm), (COLD_MODEL.into(), ModelState::Loadable)],
            embedding: true,
            standins: true,
        }
    }

    /// The second node: [`BIG_MODEL`] warm, and [`WARM_MODEL`] too (so that one has two nodes).
    pub fn secondary() -> Self {
        Self {
            label: "demo inference node 2".into(),
            models: vec![(BIG_MODEL.into(), ModelState::Warm), (WARM_MODEL.into(), ModelState::Warm)],
            embedding: false,
            standins: true,
        }
    }
}

/// Trouble to arrange on demand. Each applies once, to whichever node gets there first.
#[derive(Default)]
pub struct Faults {
    refuse: AtomicU32,
    die_after: Mutex<Option<usize>>,
    /// (model, prompt) pairs answered before: the pool's exact-match answer cache.
    seen: Mutex<HashSet<(String, String)>>,
}

impl Faults {
    /// The next `n` requests are refused before any text (as if the node were gone).
    pub fn refuse_next(&self, n: u32) {
        self.refuse.store(n, Ordering::SeqCst);
    }

    /// The next answer's node dies after `words` words; another node must finish it.
    pub fn die_next_answer_after(&self, words: usize) {
        *self.die_after.lock().unwrap_or_else(|e| e.into_inner()) = Some(words);
    }

    /// Whether this question was answered before (and notes it).
    fn seen_before(&self, model: &str, prompt: &str) -> bool {
        !self.seen.lock().unwrap_or_else(|e| e.into_inner()).insert((model.to_string(), prompt.to_string()))
    }

    fn take_refusal(&self) -> bool {
        self.refuse.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)).is_ok()
    }

    fn take_death(&self) -> Option<usize> {
        self.die_after.lock().unwrap_or_else(|e| e.into_inner()).take()
    }
}

/// The canned answer `model` gives to `prompt`.
/// What the other laptop of [`DemoNode::play_other_device`] asks.
pub const OTHER_DEVICE_QUESTION: &str = "A question typed on the other laptop";

/// The demo's way to show history from another device: a conversation whose first message
/// mentions this makes the demo's other laptop start one and sync it over.
pub const OTHER_DEVICE_TRIGGER: &str = "other laptop";

pub fn answer_for(model: &str, prompt: &str) -> String {
    format!(
        "{model} here, answering \"{prompt}\". Bragi asks the inference nodes among your own devices, \
         which pick a model node and stream the answer back over Weirdnet, token by token. Nothing \
         leaves your devices, and the stop button resets the stream so the node stops generating. \
         A cut-off answer can be continued from where it stopped, on this device or another one."
    )
}

/// Answers with [`answer_for`], one word every `delay`, honouring cancel, resume and the
/// scripted [`Faults`]. A repeated prompt is a cache hit; a model that isn't warm here is
/// answered by a stand-in, flagged.
struct FakeBackend {
    model: String,
    /// The model that actually answers, when it isn't `model`.
    standin: Option<String>,
    delay: Duration,
    cache_hit: bool,
    faults: Arc<Faults>,
}

fn last_user(req: &Request) -> String {
    req.inline_messages().and_then(|m| m.iter().rev().find(|m| m.role == Role::User)).map(|m| m.content.clone()).unwrap_or_default()
}

#[async_trait]
impl InferBackend for FakeBackend {
    fn name(&self) -> String {
        self.standin.clone().unwrap_or_else(|| self.model.clone())
    }

    fn resumes(&self) -> bool {
        true
    }

    async fn infer(&self, request: &Request, out: &Output, cancel: &CancelToken) -> std::result::Result<Finish, RemoteError> {
        let prompt = last_user(request);
        let answering = self.standin.clone().unwrap_or_else(|| self.model.clone());
        let mut text = answer_for(&answering, &prompt);
        // A resumed or continued attempt generates only what comes after the prefix already shown.
        if let Some(p) = &request.assistant_prefix {
            text = text.chars().skip(p.text.chars().count()).collect();
        }
        let mut meta = Meta::default();
        meta.cache_hit = self.cache_hit;
        meta.standin = self.standin.clone();
        if self.cache_hit || self.standin.is_some() {
            out.meta(meta.clone());
        }
        let die_after = self.faults.take_death();
        let mut sent = 0u64;
        for piece in text.split_inclusive(' ') {
            if cancel.is_cancelled() {
                return Err(RemoteError::Cancelled);
            }
            if die_after == Some(sent as usize) {
                return Err(RemoteError::Failed { message: "the demo node died mid-answer".into() });
            }
            if !self.cache_hit {
                tokio::time::sleep(self.delay).await;
            }
            out.text(piece);
            sent += 1;
        }
        let mut f = Finish::new("stop").with_usage(Usage::new(prompt.split_whitespace().count() as u64, sent));
        f.meta = meta;
        Ok(f)
    }
}

/// The fake inference host: the demo models, roster-checked.
pub struct FakeHost {
    auth: Arc<dyn Authorize>,
    delay: Duration,
    spec: NodeSpec,
    faults: Arc<Faults>,
}

impl InferHost for FakeHost {
    fn authorize(&self, caller: &Caller) -> std::result::Result<(), RemoteError> {
        self.auth.authorize(caller)
    }

    fn open(&self, _: &Caller, _: Option<&str>) -> std::result::Result<Arc<dyn ModelBackend>, RemoteError> {
        Err(RemoteError::Unsupported { message: "infer/1 is not served by the demo node".into() })
    }

    fn open_request(&self, _: &Caller, request: &Request) -> std::result::Result<Arc<dyn InferBackend>, RemoteError> {
        if self.faults.take_refusal() {
            return Err(RemoteError::Unavailable { message: "the demo node is not taking requests".into() });
        }
        let warm = |m: &str| self.spec.models.iter().any(|(id, s)| id == m && *s == ModelState::Warm);
        let model = request.model.clone().unwrap_or_else(|| self.spec.models.first().map(|(m, _)| m.clone()).unwrap_or_default());
        if !self.spec.models.iter().any(|(id, _)| *id == model) {
            return Err(RemoteError::Unavailable { message: format!("unknown model `{model}`") });
        }
        // The demo's way to show a node dying: this prompt makes the node that takes it die
        // partway, once, and another node finishes the answer.
        if self.spec.standins && request.assistant_prefix.is_none() && last_user(request).contains("even if a node dies") {
            self.faults.die_next_answer_after(14);
        }
        let standin = if warm(&model) || !self.spec.standins {
            None
        } else {
            self.spec.models.iter().find(|(_, s)| *s == ModelState::Warm).map(|(m, _)| m.clone())
        };
        let prompt = last_user(request);
        let cache_hit = request.assistant_prefix.is_none() && self.faults.seen_before(&model, &prompt);
        Ok(Arc::new(FakeBackend { model, standin, delay: self.delay, cache_hit, faults: self.faults.clone() }))
    }
}

/// The demo inference node.
pub struct DemoNode {
    pub identity: Arc<Identity>,
    pub node: Arc<SyncNode>,
    pub feed: Arc<CatalogFeed>,
    pub faults: Arc<Faults>,
    server: Mutex<Option<Server>>,
    /// The other nodes of the cloud demo, which share this one's identity.
    siblings: Mutex<Vec<Arc<DemoNode>>>,
}

fn other(e: impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorKind::Other, e.to_string())
}

impl DemoNode {
    /// Starts it with a fresh test identity in `dir`, streaming a word every `delay`.
    pub async fn start(dir: &Path, delay: Duration) -> Result<Self> {
        let identity = Arc::new(Identity::create(&dir.join("identity"), "demo inference node")?);
        Self::serve(identity, delay, NodeSpec::single(), Arc::default()).await
    }

    /// The cloud demo: a first node ([`NodeSpec::primary`]) and a second one of the same user
    /// ([`NodeSpec::secondary`]) in `dir/second`, sharing [`Faults`]. An app that links to the
    /// first (through [`accept`](Self::accept)) is admitted by both, and `app_config` dials both.
    pub async fn start_cloud(dir: &Path, delay: Duration) -> Result<Arc<Self>> {
        let identity = Arc::new(Identity::create(&dir.join("identity"), "demo inference node")?);
        let first = Arc::new(Self::serve(identity, delay, NodeSpec::primary(), Arc::default()).await?);
        let second = first.add_sibling(&dir.join("second"), delay, NodeSpec::secondary()).await?;
        lock(&first.siblings).push(second);
        Ok(first)
    }

    /// Another inference node of the same user: its identity is certified by this one's.
    pub async fn add_sibling(&self, dir: &Path, delay: Duration, spec: NodeSpec) -> Result<Arc<Self>> {
        let key = Keypair::generate_ed25519();
        let mut seed = self.identity.seed()?.0;
        let user = Keypair::ed25519_from_bytes(&mut seed).map_err(other)?;
        let cert = Roster::self_certify(&WEIRDNET, &user, &key.public().to_peer_id().to_string(), &spec.label, 1).map_err(other)?;
        self.identity.approve(cert.clone())?;
        let identity = Arc::new(Identity::adopt(&dir.join("identity"), key, &self.identity.seed()?.0, cert, &self.identity.link_settings())?);
        let node = Self::serve(identity, delay, spec, self.faults.clone()).await?;
        Ok(Arc::new(node))
    }

    async fn serve(identity: Arc<Identity>, delay: Duration, spec: NodeSpec, faults: Arc<Faults>) -> Result<Self> {
        let (key, st) = (identity.node_key().clone(), identity.state());
        let node = tokio::task::spawn_blocking(move || weirdnet_sync::start_node(key, weirdnet_sync::node_config(true), st))
            .await
            .map_err(other)?
            .map_err(other)?;
        let node = Arc::new(node);
        let auth: Arc<dyn Authorize> = Arc::new(RosterAuth::new(&WEIRDNET, identity.user_key(), identity.roster_view()));
        let feed = Arc::new(CatalogFeed::new(auth.clone()));
        let mut models: Vec<CatalogModel> = spec
            .models
            .iter()
            .map(|(id, state)| {
                let warm = *state == ModelState::Warm;
                let m = CatalogModel::new(id.clone(), Purpose::Chat, *state).with_context(32768);
                if warm { m.with_replicas(1).with_speed(if id == BIG_MODEL { 22.0 } else { 48.0 }) } else { m.with_speed(14.0) }
            })
            .collect();
        if spec.embedding {
            models.push(CatalogModel::new("nomic-embed", Purpose::Embedding, ModelState::Warm));
        }
        feed.publish(models);
        let host: Arc<dyn InferHost> = Arc::new(FakeHost { auth, delay, spec, faults: faults.clone() });
        let server = Server::start(&node, Serving { infer: Some((host, Admission::with_queue(2, 8))), catalog: Some(feed.clone()), announce: true })
            .map_err(other)?;
        Ok(Self { identity, node, feed, faults, server: Mutex::new(Some(server)), siblings: Mutex::default() })
    }

    /// Takes this node off the network (a node that went away).
    pub fn stop(&self) {
        lock(&self.server).take();
        self.node.halt();
    }

    /// The other nodes of the cloud demo.
    pub fn siblings(&self) -> Vec<Arc<DemoNode>> {
        lock(&self.siblings).clone()
    }

    /// Where an app device dials it: `/memory/N/p2p/PEER`.
    pub fn address(&self) -> String {
        for _ in 0..200 {
            if let Some(a) = self.node.status().listening.first() {
                return format!("{}/p2p/{}", a.split("/p2p/").next().unwrap_or(a), self.node.peer_id());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        String::new()
    }

    /// An app config that reaches this node over the memory transport.
    pub fn app_config(&self, dir: &Path, label: &str) -> Config {
        let mut peers = vec![self.address()];
        peers.extend(self.siblings().iter().map(|s| s.address()));
        Config { dir: dir.to_path_buf(), label: label.into(), transport: Transport::Memory, peers }
    }

    /// Links the device showing `link`, as the user's existing device would once both screens
    /// show the same code. Returns that code.
    pub async fn accept(&self, link: &str) -> Result<String> {
        let link = link.to_string();
        let mut pairing = tokio::task::spawn_blocking(move || Pairing::start(SCHEME, &link, weirdnet_sync::node_config(true)))
            .await
            .map_err(other)?
            .map_err(|e| CoreError::new(ErrorKind::Pairing, e.to_string()))?;
        let sas = pairing.sas().to_string();
        let (seed, settings) = (self.identity.seed()?, self.identity.link_settings());
        let cert = tokio::task::spawn_blocking(move || pairing.send_with(&seed, "Demo", &settings, &[SYNC_TYPE.to_string()]))
            .await
            .map_err(other)?
            .map_err(|e| CoreError::new(ErrorKind::Pairing, e.to_string()))?;
        self.identity.approve(cert.clone())?;
        for s in self.siblings() {
            s.identity.approve(cert.clone())?;
        }
        Ok(sas)
    }

    /// One sync pass from this node to the app devices at `addresses` (what [`Core::address`]
    /// gives): the user's other devices pull from each other, and the one with the lowest peer
    /// id has to start the key chain, which only a device that pulls can do.
    pub async fn sync_with(&self, addresses: &[String]) {
        let hints = crate::core::peer_hints(addresses).unwrap_or_default();
        let nodes: Vec<(Arc<Identity>, Arc<SyncNode>)> = std::iter::once((self.identity.clone(), self.node.clone()))
            .chain(self.siblings().iter().map(|s| (s.identity.clone(), s.node.clone())))
            .collect();
        for (identity, node) in nodes {
            let hints = hints.clone();
            let _ = tokio::task::spawn_blocking(move || {
                identity.tick(&node, &|c| hints.iter().find(|(p, _)| p.to_string() == c.peer_id).map(|(_, a)| a.clone()).unwrap_or_default())
            })
            .await;
        }
    }

    /// Demo only: the user's other laptop. A second core links to this node's identity, asks
    /// one question, and syncs with `core` until the conversation is there. Returns its id.
    pub async fn play_other_device(self: &Arc<Self>, core: &Core) -> Result<String> {
        fn timed_out(what: &str) -> CoreError {
            CoreError::new(ErrorKind::Unavailable, format!("timed out waiting for {what}"))
        }
        async fn until(f: impl Fn() -> bool, what: &str) -> Result<()> {
            let deadline = std::time::Instant::now() + Duration::from_secs(40);
            while !f() {
                if std::time::Instant::now() > deadline {
                    return Err(timed_out(what));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok(())
        }
        struct Scratch(std::path::PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let mut salt = [0u8; 4];
        let _ = getrandom::fill(&mut salt);
        let dir = Scratch(std::env::temp_dir().join(format!("bragi-other-laptop-{:08x}", u32::from_le_bytes(salt))));
        tracing::info!("demo: the other laptop starts");
        let laptop = Core::start(self.app_config(&dir.0, "Bragi on the laptop"))?;
        self.auto_accept(&laptop);
        laptop.pair_offer().await?;
        until(|| matches!(laptop.state().pair, PairState::Confirm { .. }), "the other laptop's code check").await?;
        laptop.pair_confirm(true)?;
        until(|| laptop.state().online && laptop.state().catalog.models.len() >= 2, "the other laptop online").await?;
        // On the memory transport devices find each other by address only.
        let addr = |c: &Core| c.address().ok_or_else(|| timed_out("an address"));
        core.add_peer(&addr(&laptop)?)?;
        laptop.add_peer(&addr(core)?)?;
        let mut chat = laptop.send(None, OTHER_DEVICE_QUESTION, None)?;
        while let Some(ev) = chat.events.recv().await {
            if matches!(ev, crate::ChatEvent::Done { .. } | crate::ChatEvent::Error { .. }) {
                break;
            }
        }
        let id = chat.conversation_id.clone();
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            if core.conversation(&id).is_ok_and(|c| c.messages.len() == 2) {
                tracing::info!("demo: the other laptop's conversation arrived");
                return Ok(id);
            }
            if std::time::Instant::now() > deadline {
                return Err(timed_out("the conversation on this device"));
            }
            let addrs: Vec<String> = [core, &laptop].iter().filter_map(|c| c.address()).collect();
            let _ = tokio::join!(core.sync_now(), laptop.sync_now(), self.sync_with(&addrs));
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Plays the existing device for `core`: when it shows a code, accept it.
    pub fn auto_accept(self: &Arc<Self>, core: &Core) {
        self.auto_accept_after(core, Duration::ZERO);
    }

    /// [`auto_accept`](Self::auto_accept), opening each code only after `delay`, so a demo
    /// shows the code the way a person scanning it would see it.
    pub fn auto_accept_after(self: &Arc<Self>, core: &Core, delay: Duration) {
        let (me, mut w) = (self.clone(), core.watch());
        core.handle().spawn(async move {
            loop {
                let offer = match &w.borrow_and_update().pair {
                    PairState::Offering { link, .. } => Some(link.clone()),
                    PairState::Linked { .. } => return,
                    _ => None,
                };
                if let Some(link) = offer {
                    tokio::time::sleep(delay).await;
                    if let Err(e) = me.accept(&link).await {
                        tracing::warn!("demo link: {e}");
                    }
                }
                if w.changed().await.is_err() {
                    return;
                }
            }
        });
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
