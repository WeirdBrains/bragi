//! Models on the user's Ymir pool (LF-30): search the Hugging Face Hub, see which node a model
//! fits, pull it to one node, the best one (`auto`) or all of them with streamed progress and
//! cancel, list what each node has, and remove with a confirm.
//!
//! Everything goes over Weirdnet through `weirdnet-backends`' `ModelsClient` (the
//! `/weirdnet-backends/models/1` protocol), presenting this device's certificate. The node admits
//! the owner's linked devices only (`RosterAuth`), so a Bragi device that is linked to the owner's
//! roster is let in; one that is not (or whose node has not synced the roster yet) is refused,
//! which shows up as [`NodeStatus::NeedsOwnerDevice`]. The Hugging Face token never travels:
//! the node that downloads uses its own, and the search from this device is anonymous.
//!
//! [`ModelsManager`] holds the logic and is transport-neutral (tests drive it with an in-memory
//! dialer and a mock Hub); [`crate::Core`] wires it to the live network and keeps the runs so
//! the shells can cancel them.

use std::sync::Arc;

use futures::future::join_all;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use weirdnet_agent::CancelToken;
use weirdnet_agent::remote::RemoteError;
use weirdnet_backends::models::{ModelsClient, Pull, RepoFits, StoredEntry};
use weirdnet_model::hf::{Hub, RepoModel, Search, SearchHit};

/// Most results a search asks for.
pub const SEARCH_LIMIT: u32 = 12;
/// Searches ask this many nodes about a repo at once (each fit reads a model header from the Hub).
const FIT_PARALLEL: usize = 4;

/// A device of the user's that may run models: a roster device other than this one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelNode {
    pub peer: String,
    pub label: String,
}

/// Whether a device manages models for this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    Ready,
    /// The device does not speak models/1 (an older `ymird`, the model role off, or a plain
    /// device such as a phone).
    NotModelNode,
    /// The node refused this device: only devices linked to the owner may manage models. The
    /// words are the node's reason (`unknown-device`, `revoked`, `needs-admission`, ...).
    NeedsOwnerDevice(String),
    Unreachable(String),
}

impl NodeStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, NodeStatus::Ready)
    }
}

/// One model file set on a node's disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredView {
    pub id: String,
    pub size: u64,
    pub files: Vec<String>,
    pub complete: bool,
    /// The id the node serves it under, when it does.
    pub served_as: Option<String>,
    /// `unloaded`, `warm`, ... when served.
    pub state: Option<String>,
    /// For an adapter: the base model it runs on.
    #[serde(default)]
    pub base: Option<String>,
    /// Where it came from (`hf`, `upload`, `train`, ...), from its provenance record.
    #[serde(default)]
    pub origin: Option<String>,
    /// Its license, when the record has one.
    #[serde(default)]
    pub license: Option<String>,
}

/// What one node has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeModels {
    pub node: ModelNode,
    pub status: NodeStatus,
    pub models: Vec<StoredView>,
}

/// A Hub repo from a search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HubHit {
    pub repo: String,
    pub downloads: u64,
    pub likes: u64,
    /// Needs an accepted licence and a Hub token on the node.
    pub gated: bool,
    pub updated: Option<String>,
}

/// One quantization of a repo measured against a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuantFit {
    pub id: String,
    pub quant: Option<String>,
    pub size: u64,
    /// Weights, cache and headroom.
    pub footprint: u64,
    pub fits: bool,
    pub complete: bool,
}

/// How a repo sits on one node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeFit {
    pub node: ModelNode,
    pub status: NodeStatus,
    /// The largest quantization that fits, and its size; none when nothing fits.
    pub best: Option<String>,
    pub best_size: u64,
    /// The node's model memory in bytes (0 when it has no budget).
    pub usable_bytes: u64,
    /// Largest first, as the node ranks them.
    pub quants: Vec<QuantFit>,
}

impl NodeFit {
    fn failed(node: &ModelNode, status: NodeStatus) -> Self {
        Self { node: node.clone(), status, best: None, best_size: 0, usable_bytes: 0, quants: Vec::new() }
    }

    fn from_repo(node: &ModelNode, f: &RepoFits) -> Self {
        let quants: Vec<QuantFit> = f
            .fits
            .iter()
            .map(|x| QuantFit {
                id: x.model.id.clone(),
                quant: x.model.quant.clone(),
                size: x.model.size,
                footprint: x.footprint,
                fits: x.fits,
                complete: x.model.complete,
            })
            .collect();
        let best_size = f.best.as_ref().and_then(|b| quants.iter().find(|q| &q.id == b)).map_or(0, |q| q.size);
        Self {
            node: node.clone(),
            status: NodeStatus::Ready,
            best: f.best.clone(),
            best_size,
            usable_bytes: f.budget.map_or(0, |b| b.usable_bytes),
            quants,
        }
    }

    /// The verdict a badge shows.
    pub fn verdict(&self) -> Verdict {
        match &self.status {
            NodeStatus::Ready if self.best.is_some() => Verdict::Fits,
            NodeStatus::Ready if self.usable_bytes == 0 => Verdict::NoBudget,
            NodeStatus::Ready => Verdict::TooBig,
            _ => Verdict::Unknown,
        }
    }
}

/// A fit badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Fits,
    TooBig,
    /// The node reported no memory budget, so it cannot say; name the model to pull.
    NoBudget,
    /// The node could not be asked.
    Unknown,
}

/// PullTarget a pull goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PullTarget {
    /// The node where the biggest model of the repo fits.
    Auto,
    /// One node, by peer id.
    Node(String),
    /// Every ready node.
    All,
}

/// What to pull.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullSpec {
    pub repo: String,
    /// A quantization id or file name; the largest that fits when `None`.
    pub model: Option<String>,
    pub context: Option<u32>,
    /// Serve it from the node's `ymird` once it is downloaded (not kept over a restart; the node's
    /// own `node.toml` is never edited from here).
    pub serve: bool,
    pub load: bool,
    pub target: PullTarget,
}

impl PullSpec {
    pub fn new(repo: impl Into<String>, target: PullTarget) -> Self {
        Self { repo: repo.into(), model: None, context: None, serve: true, load: false, target }
    }
}

/// A finished pull on one node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PulledView {
    pub id: String,
    pub size: u64,
    pub served: bool,
    pub loaded: bool,
}

/// What a search or a pull tells the shell as it goes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ModelsEvent {
    /// The repos found (the Hub's order: most downloaded first).
    Hits(Vec<HubHit>),
    /// How one repo fits one node; one event per repo and node.
    Fit {
        repo: String,
        fit: NodeFit,
    },
    /// The search is over; `error` when the Hub could not be reached.
    SearchDone {
        error: Option<String>,
    },
    /// `auto` picked a node, and why.
    Chosen {
        node: ModelNode,
        model: String,
        size: u64,
        why: String,
    },
    Progress {
        peer: String,
        label: String,
        done: u64,
        total: u64,
    },
    /// One node finished (or failed) its pull.
    NodeDone {
        node: ModelNode,
        result: Result<PulledView, String>,
    },
    /// Every node is done.
    Done,
    /// The user cancelled; partial files stay on the nodes and pulling again resumes them.
    Cancelled,
    /// The pull or upload could not start (no node fits, none answered, an unreadable file, ...).
    Failed {
        message: String,
    },
    /// An upload finished (or was refused) on its node.
    Uploaded {
        node: ModelNode,
        result: Result<upload::UploadedView, String>,
    },
}

/// Words a person can act on for a node's error.
pub fn explain(node: &ModelNode, e: &RemoteError) -> String {
    match e {
        RemoteError::Refused { message } => {
            format!("{} refused this device ({message}): only devices linked to its owner may manage models", node.label)
        }
        RemoteError::Unsupported { .. } | RemoteError::Protocol { .. } => {
            format!("{} does not take model management (an older Ymir, or its model role is off)", node.label)
        }
        RemoteError::Unreachable { message } => format!("{} is unreachable: {message}", node.label),
        RemoteError::Busy => format!("{} is already pulling that model", node.label),
        RemoteError::Timeout | RemoteError::Closed => format!("{} did not answer", node.label),
        RemoteError::BadRequest { message } | RemoteError::Failed { message } | RemoteError::Unavailable { message } => message.clone(),
        other => other.to_string(),
    }
}

fn status_of(node: &ModelNode, e: &RemoteError) -> NodeStatus {
    match e {
        RemoteError::Refused { message } => NodeStatus::NeedsOwnerDevice(message.clone()),
        RemoteError::Unsupported { .. } | RemoteError::Protocol { .. } => NodeStatus::NotModelNode,
        other => NodeStatus::Unreachable(explain(node, other)),
    }
}

fn wants(want: Option<&str>, m: &RepoModel) -> bool {
    want.is_none_or(|w| m.id == w || m.parts.iter().any(|p| p.name == w || p.name.rsplit('/').next() == Some(w)))
}

/// The node that can hold the biggest model of the repo (the one `want` names when given); a larger
/// memory budget wins a tie, then the earlier node.
fn choose_node(want: Option<&str>, fits: &[(usize, RepoFits)]) -> Option<usize> {
    fits.iter()
        .filter_map(|(i, f)| {
            let fit = f.fits.iter().find(|x| x.fits && wants(want, &x.model))?;
            Some((*i, fit.model.size, f.budget.map_or(0, |b| b.usable_bytes)))
        })
        .max_by(|a, b| (a.1, a.2).cmp(&(b.1, b.2)).then(b.0.cmp(&a.0)))
        .map(|(i, ..)| i)
}

/// A size for a sentence: `4.4 GB`.
pub fn human_bytes(n: u64) -> String {
    const U: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1000.0 && i < U.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", U[i]) }
}

/// The part of the models feature that does not depend on the live network.
pub struct ModelsManager {
    client: Arc<ModelsClient>,
    /// Where an upload is recorded after it is placed (Ymir's `/ymir/uploads/1`, temporary): the
    /// dialer and this device's certificate.
    uploads: Option<(Arc<dyn weirdnet_agent::remote::Dialer>, Option<Vec<u8>>)>,
    /// The user's other devices, as the roster has them now.
    devices: Arc<dyn Fn() -> Vec<ModelNode> + Send + Sync>,
    hub: String,
}

impl ModelsManager {
    pub fn new(client: ModelsClient, devices: Arc<dyn Fn() -> Vec<ModelNode> + Send + Sync>, hub: impl Into<String>) -> Self {
        Self { client: Arc::new(client), uploads: None, devices, hub: hub.into() }
    }

    /// The roster label of a device.
    pub fn label_of(&self, peer: &str) -> Option<String> {
        (self.devices)().into_iter().find(|n| n.peer == peer).map(|n| n.label)
    }

    /// Lets uploads be recorded and saved on the node (Ymir's `/ymir/uploads/1`).
    pub fn with_uploads(mut self, dialer: Arc<dyn weirdnet_agent::remote::Dialer>, cert: Option<Vec<u8>>) -> Self {
        self.uploads = Some((dialer, cert));
        self
    }

    /// What every device of the user's has, and whether it takes model management.
    pub async fn overview(&self) -> Vec<NodeModels> {
        let nodes = (self.devices)();
        join_all(nodes.into_iter().map(|node| async move {
            match self.client.list(&node.peer).await {
                Ok(list) => NodeModels { models: list.iter().map(stored_view).collect(), node, status: NodeStatus::Ready },
                Err(e) => NodeModels { status: status_of(&node, &e), node, models: Vec::new() },
            }
        }))
        .await
    }

    /// The devices that took part in `overview` and answered.
    async fn ready(&self) -> Vec<ModelNode> {
        self.overview().await.into_iter().filter(|n| n.status.is_ready()).map(|n| n.node).collect()
    }

    /// Searches the Hub from this device (anonymous), asking every ready node how each repo fits.
    /// If the Hub cannot be reached from here, a node searches for us.
    pub async fn search(&self, query: &str, limit: u32, tx: &UnboundedSender<ModelsEvent>) {
        let nodes = self.overview().await;
        let ready: Vec<ModelNode> = nodes.iter().filter(|n| n.status.is_ready()).map(|n| n.node.clone()).collect();
        let (hub, q) = (Hub::new(self.hub.clone()), query.to_string());
        let direct = tokio::task::spawn_blocking(move || hub.search(&Search::new(q).with_limit(limit.clamp(1, 100) as usize)))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()));
        let (hits, note): (Vec<SearchHit>, Option<String>) = match direct {
            Ok(h) => (h, None),
            Err(direct_err) => match ready.first() {
                Some(n) => match self.client.search(&n.peer, query, limit).await {
                    Ok(h) => (h, None),
                    Err(e) => (Vec::new(), Some(explain(n, &e))),
                },
                None => (Vec::new(), Some(format!("could not reach the Hugging Face Hub ({direct_err}) and no node is available to search for you"))),
            },
        };
        let _ = tx.send(ModelsEvent::Hits(hits.iter().map(hub_hit).collect()));
        // The nodes that could not be asked still get a badge, so the screen says why.
        for hit in &hits {
            for n in nodes.iter().filter(|n| !n.status.is_ready()) {
                let _ = tx.send(ModelsEvent::Fit { repo: hit.id.clone(), fit: NodeFit::failed(&n.node, n.status.clone()) });
            }
        }
        let jobs: Vec<(String, ModelNode)> = hits.iter().flat_map(|h| ready.iter().map(|n| (h.id.clone(), n.clone()))).collect();
        for chunk in jobs.chunks(FIT_PARALLEL) {
            let answers = join_all(chunk.iter().map(|(repo, node)| async move {
                let fit = match self.client.files(&node.peer, repo, None, None).await {
                    Ok(f) => NodeFit::from_repo(node, &f),
                    Err(e) => NodeFit::failed(node, status_of(node, &e)),
                };
                (repo.clone(), fit)
            }))
            .await;
            for (repo, fit) in answers {
                let _ = tx.send(ModelsEvent::Fit { repo, fit });
            }
        }
        let _ = tx.send(ModelsEvent::SearchDone { error: note });
    }

    /// Pulls to the nodes `spec.target` names, streaming progress; cancelling stops every pull
    /// (each node keeps its partial file).
    pub async fn pull(&self, spec: PullSpec, tx: &UnboundedSender<ModelsEvent>, cancel: &CancelToken) {
        let run = self.pull_inner(&spec, tx);
        tokio::select! {
            () = run => {}
            () = cancel.cancelled() => { let _ = tx.send(ModelsEvent::Cancelled); }
        }
    }

    async fn pull_inner(&self, spec: &PullSpec, tx: &UnboundedSender<ModelsEvent>) {
        let fail = |m: String| {
            let _ = tx.send(ModelsEvent::Failed { message: m });
        };
        let targets = match &spec.target {
            PullTarget::Node(peer) => match (self.devices)().into_iter().find(|n| &n.peer == peer) {
                Some(n) => vec![n],
                None => return fail("that device is not on your roster".into()),
            },
            PullTarget::All => {
                let t = self.ready().await;
                if t.is_empty() {
                    return fail("no model nodes answered among your devices".into());
                }
                t
            }
            PullTarget::Auto => match self.auto(spec).await {
                Ok((node, model, size, why)) => {
                    let _ = tx.send(ModelsEvent::Chosen { node: node.clone(), model, size, why });
                    vec![node]
                }
                Err(m) => return fail(m),
            },
        };
        let pull = {
            let mut p = Pull::new(spec.repo.clone());
            p.model = spec.model.clone();
            p.context = spec.context;
            p.serve = spec.serve || spec.load;
            p.load = spec.load;
            p
        };
        join_all(targets.iter().map(|node| {
            let (pull, tx) = (pull.clone(), tx.clone());
            let progress_tx = tx.clone();
            let (peer, label) = (node.peer.clone(), node.label.clone());
            async move {
                let r = self
                    .client
                    .pull(&node.peer, pull, move |p| {
                        let _ = progress_tx.send(ModelsEvent::Progress { peer: peer.clone(), label: label.clone(), done: p.done, total: p.total });
                    })
                    .await;
                let result = match r {
                    Ok(p) => Ok(PulledView { id: p.id, size: p.size, served: p.served, loaded: p.loaded }),
                    Err(e) => Err(explain(node, &e)),
                };
                let _ = tx.send(ModelsEvent::NodeDone { node: node.clone(), result });
            }
        }))
        .await;
        let _ = tx.send(ModelsEvent::Done);
    }

    /// Asks every ready node how the repo fits and takes the one that holds the biggest model.
    async fn auto(&self, spec: &PullSpec) -> Result<(ModelNode, String, u64, String), String> {
        let nodes = self.ready().await;
        if nodes.is_empty() {
            return Err("no model nodes answered among your devices".into());
        }
        let answers = join_all(nodes.iter().map(|n| self.client.files(&n.peer, &spec.repo, spec.context, None))).await;
        let mut fits = Vec::new();
        let mut why = Vec::new();
        for (i, (n, a)) in nodes.iter().zip(answers).enumerate() {
            match a {
                Ok(f) => fits.push((i, f)),
                Err(e) => why.push(explain(n, &e)),
            }
        }
        match choose_node(spec.model.as_deref(), &fits) {
            Some(i) => {
                let f = &fits.iter().find(|(j, _)| *j == i).expect("chosen from fits").1;
                let fit = f.fits.iter().find(|x| x.fits && wants(spec.model.as_deref(), &x.model)).expect("chosen node has a fit");
                let usable = f.budget.map_or(0, |b| b.usable_bytes);
                let others = fits.iter().filter(|(j, _)| *j != i).count();
                let reason = format!(
                    "{} can hold the biggest one: {} ({}) fits its {} of model memory{}",
                    nodes[i].label,
                    fit.model.id,
                    human_bytes(fit.model.size),
                    human_bytes(usable),
                    if others == 0 { String::new() } else { format!(", more than the other {others} could take") }
                );
                Ok((nodes[i].clone(), fit.model.id.clone(), fit.model.size, reason))
            }
            None if fits.is_empty() => Err(format!("no node answered: {}", why.join("; "))),
            None => Err(format!("nothing in {} fits any node{}", spec.repo, spec.model.as_deref().map(|m| format!(" ({m})")).unwrap_or_default())),
        }
    }

    /// Removes a stored model from a node and returns the bytes freed. The shells ask for a
    /// confirm first; `confirmed` is the guard that keeps a stray call from deleting.
    pub async fn remove(&self, peer: &str, id: &str, confirmed: bool) -> Result<u64, String> {
        if !confirmed {
            return Err("removing a model needs a confirm".into());
        }
        let node = (self.devices)().into_iter().find(|n| n.peer == peer).ok_or("that device is not on your roster")?;
        self.client.remove(peer, id).await.map_err(|e| explain(&node, &e))
    }
}

fn stored_view(e: &StoredEntry) -> StoredView {
    let files = match (&e.model, &e.adapter) {
        (Some(m), _) => m.files.clone(),
        (None, Some(a)) => a.path.file_name().map(|n| vec![n.to_string_lossy().into_owned()]).unwrap_or_default(),
        _ => Vec::new(),
    };
    StoredView {
        id: e.id().to_string(),
        size: e.size(),
        files,
        complete: e.model.as_ref().is_none_or(|m| m.complete),
        served_as: e.served_as.clone(),
        state: e.state.as_ref().map(|s| format!("{s:?}").to_lowercase()),
        base: e.base.clone().or_else(|| e.adapter.as_ref().and_then(|a| a.base_model.clone())),
        origin: e.provenance().map(|p| p.origin.clone()),
        license: e.provenance().and_then(|p| p.license.clone()),
    }
}

fn hub_hit(h: &SearchHit) -> HubHit {
    HubHit { repo: h.id.clone(), downloads: h.downloads, likes: h.likes, gated: h.gated, updated: h.last_modified.clone() }
}

pub mod upload;
pub mod ymir_uploads;

#[cfg(test)]
mod tests;
