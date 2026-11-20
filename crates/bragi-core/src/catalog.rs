//! The model picker's rows: the catalogs of every inference node among the user's devices,
//! merged. A model on several nodes is one row.

use serde::{Deserialize, Serialize};
use weirdnet_agent::remote::catalog::{Catalog, CatalogModel, ModelState, Purpose};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStatus {
    Warm,
    Loadable,
    Loading,
    Busy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub status: ModelStatus,
    /// Model nodes that have it loaded.
    pub replicas: u32,
    pub tokens_per_sec: Option<f32>,
    pub context: Option<u32>,
    /// The inference nodes that serve it, best first (warm before loadable).
    #[serde(default)]
    pub nodes: Vec<String>,
    /// The model that answers while this one loads, if a node's policy names one.
    #[serde(default)]
    pub standin: Option<String>,
}

/// One inference node's share of the pool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeInfo {
    pub id: String,
    /// Chat models it serves.
    pub models: u32,
    /// Of those, how many are warm there.
    pub warm: u32,
}

/// The catalog as the shells show it: chat models only, warm ones first.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CatalogView {
    /// The inference nodes the models come from.
    pub nodes: Vec<NodeInfo>,
    /// The sum of the nodes' catalog revisions: changes whenever any node's catalog does.
    pub rev: u64,
    pub models: Vec<ModelInfo>,
}

fn status(s: ModelState) -> ModelStatus {
    match s {
        ModelState::Warm => ModelStatus::Warm,
        ModelState::Loading => ModelStatus::Loading,
        ModelState::Busy => ModelStatus::Busy,
        _ => ModelStatus::Loadable,
    }
}

fn info(m: &CatalogModel) -> ModelInfo {
    ModelInfo {
        id: m.id.clone(),
        status: status(m.state),
        replicas: m.replicas,
        tokens_per_sec: m.tokens_per_sec,
        context: m.context,
        nodes: Vec::new(),
        standin: None,
    }
}

fn rank(s: ModelStatus) -> u8 {
    match s {
        ModelStatus::Warm => 0,
        ModelStatus::Busy => 1,
        ModelStatus::Loading => 2,
        ModelStatus::Loadable => 3,
    }
}

impl CatalogView {
    /// Merges the catalogs of several inference nodes (`(node, catalog)`): chat models only. A
    /// model's status is its best across nodes, replicas add up, speed and context are the
    /// largest, and `nodes` lists where to ask, best first. Warm models come first in the list.
    pub fn merge(feeds: &[(String, Catalog)]) -> Self {
        let mut by_id: Vec<ModelInfo> = Vec::new();
        let mut nodes = Vec::new();
        let mut rev = 0u64;
        // (model, node, status) rows, to order each model's nodes.
        let mut seen: Vec<(String, String, ModelStatus)> = Vec::new();
        for (node, c) in feeds {
            rev = rev.wrapping_add(c.rev);
            let chat: Vec<&CatalogModel> = c.models.iter().filter(|m| m.purpose == Purpose::Chat).collect();
            nodes.push(NodeInfo {
                id: node.clone(),
                models: chat.len() as u32,
                warm: chat.iter().filter(|m| status(m.state) == ModelStatus::Warm).count() as u32,
            });
            for m in chat {
                seen.push((m.id.clone(), node.clone(), status(m.state)));
                match by_id.iter_mut().find(|i| i.id == m.id) {
                    None => by_id.push(ModelInfo { standin: m.standin.clone(), ..info(m) }),
                    Some(i) => {
                        if rank(status(m.state)) < rank(i.status) {
                            i.status = status(m.state);
                        }
                        i.replicas += m.replicas;
                        i.tokens_per_sec = match (i.tokens_per_sec, m.tokens_per_sec) {
                            (Some(a), Some(b)) => Some(a.max(b)),
                            (a, b) => a.or(b),
                        };
                        i.context = i.context.max(m.context);
                        if i.standin.is_none() {
                            i.standin = m.standin.clone();
                        }
                    }
                }
            }
        }
        for m in &mut by_id {
            let mut at: Vec<&(String, String, ModelStatus)> = seen.iter().filter(|(id, ..)| *id == m.id).collect();
            at.sort_by_key(|(_, _, s)| rank(*s));
            m.nodes = at.into_iter().map(|(_, n, _)| n.clone()).collect();
        }
        by_id.sort_by(|a, b| rank(a.status).cmp(&rank(b.status)).then_with(|| a.id.cmp(&b.id)));
        Self { nodes, rev, models: by_id }
    }

    /// The model to use when the user hasn't picked one: the first warm one, else the first.
    pub fn default_model(&self) -> Option<&str> {
        self.models.first().map(|m| m.id.as_str())
    }

    /// The nodes to ask for `model`, best first (empty if no node lists it).
    pub fn nodes_for(&self, model: &str) -> Vec<String> {
        self.models.iter().find(|m| m.id == model).map(|m| m.nodes.clone()).unwrap_or_default()
    }
}
