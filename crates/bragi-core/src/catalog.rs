//! The model picker's rows, from the catalog one inference node serves (B2 merges several).

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
}

/// The catalog as the shells show it: chat models only, warm ones first.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CatalogView {
    /// The inference node it came from.
    pub node: Option<String>,
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
    ModelInfo { id: m.id.clone(), status: status(m.state), replicas: m.replicas, tokens_per_sec: m.tokens_per_sec, context: m.context }
}

impl CatalogView {
    pub fn from_catalog(node: &str, c: &Catalog) -> Self {
        let mut models: Vec<ModelInfo> = c.models.iter().filter(|m| m.purpose == Purpose::Chat).map(info).collect();
        let rank = |s: ModelStatus| match s {
            ModelStatus::Warm => 0,
            ModelStatus::Busy => 1,
            ModelStatus::Loading => 2,
            ModelStatus::Loadable => 3,
        };
        models.sort_by(|a, b| rank(a.status).cmp(&rank(b.status)).then_with(|| a.id.cmp(&b.id)));
        Self { node: Some(node.to_string()), rev: c.rev, models }
    }

    /// The model to use when the user hasn't picked one: the first warm one, else the first.
    pub fn default_model(&self) -> Option<&str> {
        self.models.first().map(|m| m.id.as_str())
    }
}
