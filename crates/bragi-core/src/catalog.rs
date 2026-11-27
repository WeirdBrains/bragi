//! The model picker's rows: the catalogs of every inference node among the user's devices,
//! merged. A model on several nodes is one row.

use serde::{Deserialize, Serialize};
use weirdnet_agent::Modalities;
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
    /// The smallest context window any node serving it lists: what a request must fit, since
    /// the pool may send it to any of them (failover included).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_context: Option<u32>,
    /// The inference nodes that serve it, best first (warm before loadable).
    #[serde(default)]
    pub nodes: Vec<String>,
    /// The model that answers while this one loads, if a node's policy names one.
    #[serde(default)]
    pub standin: Option<String>,
    /// Whether some node's copy of it reads images.
    #[serde(default)]
    pub vision: bool,
    /// Whether some node's copy of it reads audio.
    #[serde(default)]
    pub audio: bool,
    /// Whether some node's copy of it reads video itself (catalog/1 `inputs.video`, weirdnet-agent
    /// 0.5.10): a clip that fits its limits may go whole (Native video V4).
    #[serde(default)]
    pub video: bool,
    /// For a Ymir rollout (training plan P4b): its pinnable versions (`support@2`, `support@3`),
    /// folded under the name so the picker shows one row. Asking for the name lets the pool pick
    /// a version per conversation; a version id pins it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<ModelInfo>,
    /// Where it came from (`hub`, `upload`, `train`), as the first node that knows says
    /// (catalog/1 from Ymir nodes on weirdnet-agent 0.5.4). `None` from older nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Its license (an SPDX id, or `unknown`), when given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Media and request limits (images, image cost and edge, audio, request bytes): the
    /// strictest across the nodes serving it, each absent when no node reports it (Ymir on
    /// weirdnet-agent 0.5.7). What video frames are sized from ([`crate::video`]).
    #[serde(default, skip_serializing_if = "weirdnet_agent::MediaLimits::is_unknown")]
    pub limits: weirdnet_agent::MediaLimits,
}

impl ModelInfo {
    /// The tag the model lists show for a model the user sent or trained (`upload, MIT`,
    /// `train`); Hub models, the usual case, and models of unknown origin have none, as on the
    /// Models screen.
    pub fn origin_tag(&self) -> Option<String> {
        let o = self.origin.as_deref().filter(|o| matches!(*o, "upload" | "train"))?;
        Some(match &self.license {
            Some(l) => format!("{o}, {l}"),
            None => o.to_string(),
        })
    }

    /// Whether it reads what `needed` asks for (text always).
    pub fn takes(&self, needed: Modalities) -> bool {
        Modalities::new(self.vision, self.audio).covers(needed)
    }
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
        min_context: m.context,
        nodes: Vec::new(),
        standin: None,
        vision: m.inputs.vision,
        audio: m.inputs.audio,
        video: m.inputs.video,
        versions: Vec::new(),
        origin: m.origin.clone(),
        license: m.license.clone(),
        limits: m.limits,
    }
}

/// `support@3` → `support`, when what follows the last `@` is a version number.
pub fn rollout_name(id: &str) -> Option<&str> {
    let (name, v) = id.rsplit_once('@')?;
    (!name.is_empty() && !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())).then_some(name)
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
                        i.min_context = match (i.min_context, m.context) {
                            (Some(a), Some(b)) => Some(a.min(b)),
                            (a, b) => a.or(b),
                        };
                        i.limits = i.limits.tightest(m.limits);
                        i.vision |= m.inputs.vision;
                        i.audio |= m.inputs.audio;
                        i.video |= m.inputs.video;
                        if i.standin.is_none() {
                            i.standin = m.standin.clone();
                        }
                        if i.origin.is_none() && i.license.is_none() {
                            (i.origin, i.license) = (m.origin.clone(), m.license.clone());
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
        // Rollout versions go under their name's row, when the name is listed too.
        let (versions, mut models): (Vec<ModelInfo>, Vec<ModelInfo>) =
            by_id.into_iter().partition(|m| rollout_name(&m.id).is_some_and(|n| seen.iter().any(|(id, ..)| id == n)));
        for v in versions {
            let name = rollout_name(&v.id).expect("partitioned").to_string();
            if let Some(m) = models.iter_mut().find(|m| m.id == name) {
                m.versions.push(v);
            }
        }
        for m in &mut models {
            m.versions.sort_by(|a, b| a.id.cmp(&b.id));
        }
        models.sort_by(|a, b| rank(a.status).cmp(&rank(b.status)).then_with(|| a.id.cmp(&b.id)));
        Self { nodes, rev, models }
    }

    /// A model row, or a rollout version folded under one.
    pub fn model(&self, model: &str) -> Option<&ModelInfo> {
        self.models.iter().find(|m| m.id == model).or_else(|| self.models.iter().flat_map(|m| &m.versions).find(|v| v.id == model))
    }

    /// The model to use when the user hasn't picked one: the first warm one, else the first.
    pub fn default_model(&self) -> Option<&str> {
        self.models.first().map(|m| m.id.as_str())
    }

    /// Like [`default_model`](Self::default_model), among the models that read `needed`.
    pub fn default_model_for(&self, needed: Modalities) -> Option<&str> {
        self.models.iter().find(|m| m.takes(needed)).map(|m| m.id.as_str())
    }

    /// Why `model` can't take `needed`, and which models can.
    pub fn refusal(&self, model: &str, needed: Modalities) -> String {
        let what = self.model(model).and_then(|m| Modalities::new(m.vision, m.audio).lacking(needed)).unwrap_or("this input");
        let ok: Vec<&str> = self.models.iter().filter(|m| m.takes(needed)).map(|m| m.id.as_str()).collect();
        if ok.is_empty() {
            format!("{model} can't read {what}, and none of the models on your nodes can")
        } else {
            format!("{model} can't read {what}; pick one that can: {}", ok.join(", "))
        }
    }

    /// The media limits for `model` (the strictest its nodes report); unknown when none do.
    pub fn limits_for(&self, model: &str) -> weirdnet_agent::MediaLimits {
        self.model(model).map(|m| m.limits).unwrap_or_default()
    }

    /// The context window a request to `model` must fit (the smallest its nodes list), if known.
    pub fn context_for(&self, model: &str) -> Option<u32> {
        self.model(model).and_then(|m| m.min_context.or(m.context))
    }

    /// The nodes to ask for `model`, best first (empty if no node lists it).
    pub fn nodes_for(&self, model: &str) -> Vec<String> {
        self.model(model).map(|m| m.nodes.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_must_fit_the_smallest_window_among_the_nodes() {
        let at = |ctx: Option<u32>| {
            let mut m = CatalogModel::new("inkling-2-mini", Purpose::Chat, ModelState::Warm);
            m.context = ctx;
            Catalog::new(1, vec![m])
        };
        let v = CatalogView::merge(&[("a".into(), at(Some(131_072))), ("b".into(), at(Some(32_768))), ("c".into(), at(None))]);
        assert_eq!(v.models[0].context, Some(131_072), "the picker shows the largest");
        assert_eq!(v.context_for("inkling-2-mini"), Some(32_768));
        assert_eq!(v.context_for("other"), None);
        let v = CatalogView::merge(&[("c".into(), at(None))]);
        assert_eq!(v.context_for("inkling-2-mini"), None);
    }

    #[test]
    fn the_catalog_says_where_a_model_came_from() {
        use weirdnet_agent::remote::catalog::{ORIGIN_HUB, ORIGIN_TRAIN, ORIGIN_UPLOAD};
        let m = |id: &str| CatalogModel::new(id, Purpose::Chat, ModelState::Warm);
        // a is an older node (no origin); b knows.
        let a = Catalog::new(1, vec![m("tuned"), m("qwen")]);
        let b = Catalog::new(
            1,
            vec![m("tuned").with_origin(ORIGIN_UPLOAD).with_license("MIT"), m("qwen").with_origin(ORIGIN_HUB), m("mine").with_origin(ORIGIN_TRAIN)],
        );
        let v = CatalogView::merge(&[("a".into(), a), ("b".into(), b)]);
        let tag = |id: &str| v.model(id).unwrap().origin_tag();
        assert_eq!(tag("tuned").as_deref(), Some("upload, MIT"));
        assert_eq!(tag("mine").as_deref(), Some("train"));
        assert_eq!(v.model("qwen").unwrap().origin.as_deref(), Some("hub"));
        assert_eq!(tag("qwen"), None, "Hub models are the usual case and carry no tag");
        // Older views (no fields) still read, and an unknown origin has no tag.
        let old: ModelInfo =
            serde_json::from_value(serde_json::json!({"id": "x", "status": "warm", "replicas": 1, "tokens_per_sec": null, "context": null})).unwrap();
        assert_eq!((old.origin_tag(), old.license), (None, None));
    }

    #[test]
    fn rollout_versions_fold_under_their_name() {
        let m = |id: &str, s: ModelState| CatalogModel::new(id, Purpose::Chat, s).with_context(4096);
        let a = Catalog::new(
            1,
            vec![
                m("support", ModelState::Warm),
                m("support@2", ModelState::Warm),
                m("support@3", ModelState::Loadable),
                m("s2", ModelState::Warm),
                m("x@1", ModelState::Warm),
                m("mail@home", ModelState::Warm),
            ],
        );
        let v = CatalogView::merge(&[("a".into(), a)]);
        let ids: Vec<&str> = v.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["mail@home", "s2", "support", "x@1"], "versions fold; an @ without a listed rollout name, or not a number, stays");
        let support = v.models.iter().find(|m| m.id == "support").unwrap();
        let vs: Vec<&str> = support.versions.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(vs, ["support@2", "support@3"]);
        assert_eq!(v.nodes_for("support@3"), ["a"], "a pinned version is still routed");
        assert_eq!(v.context_for("support@2"), Some(4096));
        assert_eq!(rollout_name("support@12"), Some("support"));
        assert_eq!(rollout_name("support@"), None);
    }
}
