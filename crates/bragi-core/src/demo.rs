//! A stand-in for the user's other devices, in process on the memory transport: an inference
//! node with a test identity, a fake [`InferHost`] with canned answers, and the "existing
//! device" side of linking. The tests, and both shells' demo profile (screenshots, the shared
//! end-to-end scenario), use it; it never touches the real network or a real identity.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use weirdnet_agent::model::ModelBackend;
use weirdnet_agent::remote::catalog::{CatalogFeed, CatalogModel, ModelState, Purpose};
use weirdnet_agent::remote::infer::v2::{Finish, InferBackend, Output, Request, Usage};
use weirdnet_agent::remote::infer::{Admission, InferHost};
use weirdnet_agent::serve::{RosterAuth, Server, Serving};
use weirdnet_agent::{Authorize, Caller, CancelToken, RemoteError, Role};
use weirdnet_device_link::Pairing;
use weirdnet_net::WEIRDNET;

use crate::core::{CONTINUE_PROMPT, Config, Core, PairState, SCHEME, Transport};
use crate::error::{CoreError, ErrorKind, Result};
use crate::identity::{Identity, SYNC_TYPE, SyncNode};

/// The demo's models: one warm, one that would have to load.
pub const WARM_MODEL: &str = "gemma-4-e4b";
pub const COLD_MODEL: &str = "qwen3-32b";

/// The canned answer `model` gives to `prompt`.
pub fn answer_for(model: &str, prompt: &str) -> String {
    format!(
        "{model} here, answering \"{prompt}\". Bragi asks the inference nodes among your own devices, \
         which pick a model node and stream the answer back over Weirdnet, token by token. Nothing \
         leaves your devices, and the stop button resets the stream so the node stops generating. \
         A cut-off answer can be continued from where it stopped, on this device or another one."
    )
}

/// Answers with [`answer_for`], one word every `delay`, honouring cancel.
struct FakeBackend {
    model: String,
    delay: Duration,
}

fn last_user(req: &Request) -> String {
    req.inline_messages().and_then(|m| m.iter().rev().find(|m| m.role == Role::User)).map(|m| m.content.clone()).unwrap_or_default()
}

#[async_trait]
impl InferBackend for FakeBackend {
    fn name(&self) -> String {
        self.model.clone()
    }

    async fn infer(&self, request: &Request, out: &Output, cancel: &CancelToken) -> std::result::Result<Finish, RemoteError> {
        let msgs = request.inline_messages().unwrap_or_default();
        let prompt = last_user(request);
        // "Continue": the rest of the answer to the prompt before it, after what was shown.
        let text = if prompt == CONTINUE_PROMPT {
            let shown = msgs.iter().rev().find(|m| m.role == Role::Assistant).map(|m| m.content.clone()).unwrap_or_default();
            let asked = msgs.iter().rev().filter(|m| m.role == Role::User).nth(1).map(|m| m.content.clone()).unwrap_or_default();
            let full = answer_for(&self.model, &asked);
            full.strip_prefix(shown.as_str()).unwrap_or(&full).to_string()
        } else {
            answer_for(&self.model, &prompt)
        };
        let mut sent = 0u64;
        for piece in text.split_inclusive(' ') {
            if cancel.is_cancelled() {
                return Err(RemoteError::Cancelled);
            }
            tokio::time::sleep(self.delay).await;
            out.text(piece);
            sent += 1;
        }
        Ok(Finish::new("stop").with_usage(Usage::new(prompt.split_whitespace().count() as u64, sent)))
    }
}

/// The fake inference host: the demo models, roster-checked.
pub struct FakeHost {
    auth: Arc<dyn Authorize>,
    delay: Duration,
}

impl InferHost for FakeHost {
    fn authorize(&self, caller: &Caller) -> std::result::Result<(), RemoteError> {
        self.auth.authorize(caller)
    }

    fn open(&self, _: &Caller, _: Option<&str>) -> std::result::Result<Arc<dyn ModelBackend>, RemoteError> {
        Err(RemoteError::Unsupported { message: "infer/1 is not served by the demo node".into() })
    }

    fn open_request(&self, _: &Caller, request: &Request) -> std::result::Result<Arc<dyn InferBackend>, RemoteError> {
        let model = request.model.clone().unwrap_or_else(|| WARM_MODEL.to_string());
        if model != WARM_MODEL && model != COLD_MODEL {
            return Err(RemoteError::Unavailable { message: format!("unknown model `{model}`") });
        }
        Ok(Arc::new(FakeBackend { model, delay: self.delay }))
    }
}

/// The demo inference node.
pub struct DemoNode {
    pub identity: Arc<Identity>,
    pub node: Arc<SyncNode>,
    pub feed: Arc<CatalogFeed>,
    _server: Server,
}

fn other(e: impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorKind::Other, e.to_string())
}

impl DemoNode {
    /// Starts it with a fresh test identity in `dir`, streaming a word every `delay`.
    pub async fn start(dir: &Path, delay: Duration) -> Result<Self> {
        let identity = Arc::new(Identity::create(&dir.join("identity"), "demo inference node")?);
        let (key, st) = (identity.node_key().clone(), identity.state());
        let node = tokio::task::spawn_blocking(move || weirdnet_sync::start_node(key, weirdnet_sync::node_config(true), st))
            .await
            .map_err(other)?
            .map_err(other)?;
        let node = Arc::new(node);
        let auth: Arc<dyn Authorize> = Arc::new(RosterAuth::new(&WEIRDNET, identity.user_key(), identity.roster_view()));
        let feed = Arc::new(CatalogFeed::new(auth.clone()));
        feed.publish(vec![
            CatalogModel::new(WARM_MODEL, Purpose::Chat, ModelState::Warm).with_replicas(1).with_speed(48.0).with_context(32768),
            CatalogModel::new(COLD_MODEL, Purpose::Chat, ModelState::Loadable).with_speed(14.0).with_context(32768),
            CatalogModel::new("nomic-embed", Purpose::Embedding, ModelState::Warm),
        ]);
        let host: Arc<dyn InferHost> = Arc::new(FakeHost { auth, delay });
        let server = Server::start(&node, Serving { infer: Some((host, Admission::with_queue(2, 8))), catalog: Some(feed.clone()), announce: true })
            .map_err(other)?;
        Ok(Self { identity, node, feed, _server: server })
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
        Config { dir: dir.to_path_buf(), label: label.into(), transport: Transport::Memory, peers: vec![self.address()] }
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
        self.identity.approve(cert)?;
        Ok(sas)
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
