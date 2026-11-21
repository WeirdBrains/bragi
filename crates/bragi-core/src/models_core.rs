//! [`Core`]'s side of model management (LF-30): the live network wired into
//! [`ModelsManager`](crate::models::ModelsManager), and the runs the shells can cancel.

use std::sync::Arc;

use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use weirdnet_agent::CancelToken;
use weirdnet_agent::pool::encode_cert;
use weirdnet_backends::models::ModelsClient;
use weirdnet_model::ModelStore;

use crate::core::{Core, lock, random_id};
use crate::error::Result;
use crate::models::{ModelNode, ModelsEvent, ModelsManager, NodeModels, PullSpec, SEARCH_LIMIT};

/// A search or a pull under way: its events, and the id [`Core::cancel`] takes.
pub struct ModelsRun {
    pub id: String,
    pub events: UnboundedReceiver<ModelsEvent>,
}

impl Core {
    fn models_manager(&self) -> Result<ModelsManager> {
        let net = self.net()?;
        let client = ModelsClient::new(net.dialer.clone()).with_cert(Some(encode_cert(&net.identity.our_cert())));
        let (identity, me) = (net.identity.clone(), net.node.peer_id().to_string());
        let devices = Arc::new(move || {
            identity
                .roster()
                .active()
                .filter(|c| c.peer_id != me)
                .map(|c| ModelNode { peer: c.peer_id.clone(), label: c.label.clone() })
                .collect::<Vec<_>>()
        });
        // The demo nodes start a mock Hub; otherwise `WEIRDNET_HF_URL` (a mirror) or Hugging Face.
        let hub = crate::demo::hub_override().unwrap_or_else(|| ModelStore::from_env().hub().to_string());
        Ok(ModelsManager::new(client, devices, hub))
    }

    /// What each of the user's other devices has, and whether it takes model management
    /// (`needs owner device` when it refuses this one).
    pub async fn models_overview(&self) -> Result<Vec<NodeModels>> {
        Ok(self.models_manager()?.overview().await)
    }

    /// Searches the Hub and measures every hit against every node. Events: `Hits`, then a `Fit`
    /// per repo and node, then `SearchDone`.
    pub fn models_search(&self, query: &str) -> Result<ModelsRun> {
        let mgr = self.models_manager()?;
        let (tx, events) = unbounded_channel();
        let (id, cancel) = self.new_models_run();
        let (core, run, query) = (self.clone(), id.clone(), query.trim().to_string());
        self.inner.handle.spawn(async move {
            tokio::select! {
                () = mgr.search(&query, SEARCH_LIMIT, &tx) => {}
                () = cancel.cancelled() => { let _ = tx.send(ModelsEvent::Cancelled); }
            }
            lock(&core.inner.requests).remove(&run);
        });
        Ok(ModelsRun { id, events })
    }

    /// Pulls to a node, the best one or all of them. Events: `Chosen` (auto), `Progress`,
    /// `NodeDone` per node, then `Done`; [`Core::cancel`] with the run's id stops it (`Cancelled`).
    pub fn models_pull(&self, spec: PullSpec) -> Result<ModelsRun> {
        let mgr = self.models_manager()?;
        let (tx, events) = unbounded_channel();
        let (id, cancel) = self.new_models_run();
        let (core, run) = (self.clone(), id.clone());
        self.inner.handle.spawn(async move {
            mgr.pull(spec, &tx, &cancel).await;
            lock(&core.inner.requests).remove(&run);
        });
        Ok(ModelsRun { id, events })
    }

    /// Removes a stored model from a node; returns the bytes freed. `confirmed` must be true: the
    /// shells ask the user first.
    pub async fn models_remove(&self, peer: &str, id: &str, confirmed: bool) -> std::result::Result<u64, String> {
        self.models_manager().map_err(|e| e.message)?.remove(peer, id, confirmed).await
    }

    fn new_models_run(&self) -> (String, CancelToken) {
        let id = random_id("models");
        let cancel = CancelToken::new();
        lock(&self.inner.requests).insert(id.clone(), cancel.clone());
        (id, cancel)
    }
}
