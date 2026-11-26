//! The training view (Ymir's training plan, after Bragi v1): what each of the user's Ymir model
//! nodes has trained and is training (`/ymir/train-status/1`, read-only), and the versioned
//! rollouts on the inference nodes (`/ymir/rollouts/1`), with each version's share of new
//! conversations, whether it is the canary, and the run that trained it when a node says so.
//! Nothing here changes a node: it is a read, refreshed when the shell asks.

use std::sync::Arc;

use futures::future::join_all;
use serde::{Deserialize, Serialize};
use weirdnet_agent::remote::{Dialer, RemoteError};

use super::ymir_rollouts;
use super::ymir_train::{self, JobSummary, RunSummary};
use super::{ModelNode, NodeStatus, explain};

/// One device's training, or why it shows none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeTraining {
    pub node: ModelNode,
    /// `Ready`; `NotModelNode` for a device without the protocol (a phone, a Ymir before 0.16.0,
    /// the model role off); `NeedsOwnerDevice`; `Unreachable`.
    pub status: NodeStatus,
    /// It takes runs from the owner's devices.
    pub remote: bool,
    pub runs: Vec<RunSummary>,
    pub jobs: Vec<JobSummary>,
}

/// Where a version of a rollout stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage {
    /// It gets new conversations (all of them when it is the only one).
    Live,
    /// The newest version with a share, beside an older live one: the canary.
    Canary,
    /// No share: kept for rollback, or not started.
    Standby,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionView {
    pub version: u32,
    pub model: String,
    /// Share of new conversations, in percent.
    pub percent: f64,
    pub stage: Stage,
    /// The node and run that trained this version's model, when a node reported it.
    pub trained_by: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RolloutView {
    /// The inference node it is on.
    pub node: ModelNode,
    pub name: String,
    pub versions: Vec<VersionView>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TrainingOverview {
    pub nodes: Vec<NodeTraining>,
    pub rollouts: Vec<RolloutView>,
}

impl TrainingOverview {
    /// Nodes that answered, so the view can hide the rest behind a count.
    pub fn ready(&self) -> impl Iterator<Item = &NodeTraining> {
        self.nodes.iter().filter(|n| n.status.is_ready())
    }
}

/// Each version's share and stage.
pub fn stages(r: &ymir_rollouts::Rollout) -> Vec<(u32, f64, Stage)> {
    let total: u64 = r.versions.iter().map(|v| u64::from(v.weight)).sum();
    let live: Vec<u32> = r.versions.iter().filter(|v| v.weight > 0).map(|v| v.version).collect();
    let canary = (live.len() > 1).then(|| live.iter().copied().max()).flatten();
    r.versions
        .iter()
        .map(|v| {
            let pct = if total == 0 { 0.0 } else { 100.0 * f64::from(v.weight) / total as f64 };
            let stage = match () {
                () if v.weight == 0 => Stage::Standby,
                () if Some(v.version) == canary => Stage::Canary,
                () => Stage::Live,
            };
            (v.version, pct, stage)
        })
        .collect()
}

/// Reads every device: its training status, and its rollouts when it is an inference node.
pub async fn overview(dialer: Arc<dyn Dialer>, cert: Option<Vec<u8>>, nodes: Vec<ModelNode>) -> TrainingOverview {
    let reads = join_all(nodes.into_iter().map(|node| {
        let (dialer, cert) = (dialer.clone(), cert.clone());
        async move {
            let st = ymir_train::status(dialer.as_ref(), &node.peer, cert.clone()).await;
            let ro = ymir_rollouts::read(dialer.as_ref(), &node.peer, &node.label, cert).await.ok();
            (node, st, ro)
        }
    }))
    .await;
    let mut out = TrainingOverview::default();
    let mut rollouts = Vec::new();
    for (node, st, ro) in reads {
        let t = match st {
            Ok(s) => NodeTraining { node: node.clone(), status: NodeStatus::Ready, remote: s.remote, runs: s.runs, jobs: s.jobs },
            Err(e) => NodeTraining { status: status_of(&node, &e), node: node.clone(), remote: false, runs: Vec::new(), jobs: Vec::new() },
        };
        out.nodes.push(t);
        for r in ro.into_iter().flatten() {
            rollouts.push((node.clone(), r));
        }
    }
    for (node, r) in rollouts {
        let versions = stages(&r)
            .into_iter()
            .zip(&r.versions)
            .map(|((version, percent, stage), v)| VersionView {
                version,
                model: v.model.clone(),
                percent,
                stage,
                trained_by: out.nodes.iter().find_map(|n| {
                    n.runs.iter().find(|run| run.adapter.as_deref() == Some(v.model.as_str())).map(|run| (n.node.label.clone(), run.id.clone()))
                }),
            })
            .collect();
        out.rollouts.push(RolloutView { node, name: r.name, versions });
    }
    out
}

fn status_of(node: &ModelNode, e: &RemoteError) -> NodeStatus {
    match e {
        RemoteError::Refused { message } => NodeStatus::NeedsOwnerDevice(message.clone()),
        RemoteError::Unsupported { .. } | RemoteError::Protocol { .. } => NodeStatus::NotModelNode,
        other => NodeStatus::Unreachable(explain(node, other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ymir_rollouts::{Rollout, Version};
    use crate::models::ymir_train::TrainStatus;
    use weirdnet_agent::remote::MemoryDialer;

    fn run(id: &str, status: &str, adapter: Option<&str>) -> RunSummary {
        RunSummary {
            id: id.into(),
            status: status.into(),
            base: "alpha".into(),
            adapter: adapter.map(str::to_string),
            adapter_revision: adapter.map(|_| "sha256:0011223344556677".into()),
            dataset: "dn.dataset.jsonl".into(),
            dataset_license: "CC0-1.0".into(),
            teacher: Some("teacher-8b".into()),
            rows_used: 4,
            epochs: 2,
            steps: 8,
            final_loss: Some(0.5),
            started_ms: 1,
            ended_ms: Some(2),
            error: None,
            checkpoints: 2,
            resumed: false,
        }
    }

    fn rollout(weights: &[(u32, &str, u32)]) -> Rollout {
        Rollout { name: "chat".into(), versions: weights.iter().map(|&(version, m, weight)| Version { version, model: m.into(), weight }).collect() }
    }

    #[test]
    fn the_newest_version_with_a_share_beside_a_live_one_is_the_canary() {
        assert_eq!(stages(&rollout(&[(1, "a", 80), (2, "b", 20)])), [(1, 80.0, Stage::Live), (2, 20.0, Stage::Canary)]);
        assert_eq!(stages(&rollout(&[(1, "a", 0), (2, "b", 5)])), [(1, 0.0, Stage::Standby), (2, 100.0, Stage::Live)]);
        assert_eq!(stages(&rollout(&[(1, "a", 0), (2, "b", 0)])), [(1, 0.0, Stage::Standby), (2, 0.0, Stage::Standby)]);
        let three = stages(&rollout(&[(1, "a", 50), (2, "b", 40), (3, "c", 10)]));
        assert_eq!(three.iter().map(|x| x.2).collect::<Vec<_>>(), [Stage::Live, Stage::Live, Stage::Canary]);
    }

    #[tokio::test]
    async fn the_overview_joins_runs_to_rollout_versions() {
        let dialer = MemoryDialer::new();
        let gpu = TrainStatus {
            node: "gpu".into(),
            remote: true,
            runs: vec![run("20261125-0101-ab", "done", Some("alpha+dn")), run("20261125-0001-cd", "failed", None)],
            jobs: vec![JobSummary {
                id: "train-1".into(),
                state: "paused".into(),
                base: "alpha".into(),
                adapter: "alpha+ft".into(),
                holder: None,
                attempts: 1,
                checkpoint_epoch: Some(1),
                error: None,
            }],
        };
        dialer.route_only("peer-gpu", &[ymir_train::TRAIN_STATUS_PROTOCOL], move |ch, _| {
            let s = gpu.clone();
            tokio::spawn(async move { ymir_train::serve_stand_in(ch, &s).await });
        });
        let rs = Arc::new(std::sync::Mutex::new(vec![rollout(&[(1, "alpha", 90), (2, "alpha+dn", 10)])]));
        dialer.route_only("peer-inf", &[ymir_rollouts::ROLLOUTS_PROTOCOL], move |ch, _| {
            let r = rs.clone();
            tokio::spawn(async move { ymir_rollouts::serve_stand_in(ch, &r).await });
        });
        dialer.route_only("peer-phone", &[], |_, _| {});
        let nodes = vec![
            ModelNode { peer: "peer-gpu".into(), label: "gpu".into() },
            ModelNode { peer: "peer-inf".into(), label: "inf".into() },
            ModelNode { peer: "peer-phone".into(), label: "phone".into() },
        ];
        let o = overview(Arc::new(dialer), None, nodes).await;
        assert_eq!(
            o.nodes.iter().map(|n| (n.node.label.as_str(), n.status.clone())).collect::<Vec<_>>(),
            [("gpu", NodeStatus::Ready), ("inf", NodeStatus::NotModelNode), ("phone", NodeStatus::NotModelNode)]
        );
        let gpu = &o.nodes[0];
        assert!(gpu.remote && gpu.runs.len() == 2 && gpu.jobs[0].checkpoint_epoch == Some(1));
        assert_eq!(o.ready().count(), 1);
        assert_eq!(o.rollouts.len(), 1);
        let r = &o.rollouts[0];
        assert_eq!((r.node.label.as_str(), r.name.as_str()), ("inf", "chat"));
        assert_eq!(r.versions[1].stage, Stage::Canary);
        assert_eq!(r.versions[1].percent, 10.0);
        assert_eq!(r.versions[1].trained_by, Some(("gpu".to_string(), "20261125-0101-ab".to_string())));
        assert_eq!(r.versions[0].trained_by, None);
    }
}
