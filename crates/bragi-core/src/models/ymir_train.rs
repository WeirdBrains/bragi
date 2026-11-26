//! TEMPORARY: a minimal client for Ymir's `/ymir/train-status/1` (Ymir `ymir_core::train_net`,
//! Ymir 0.16.0), which reads a model node's training runs and resumable jobs, read-only, for any
//! of the owner's linked devices. Ymir's crate is not published, so the wire types Bragi reads
//! are mirrored here. It goes away with the Weirdnet job record (`weirdnet-agent` 0.6.0's
//! `JobRecord`, asked of the Weirdnet lead, 2026-11-25), which the training view then reads.

use serde::{Deserialize, Serialize};
use weirdnet_agent::remote::{Dialer, RemoteError};

/// Ymir's protocol id.
pub const TRAIN_STATUS_PROTOCOL: &str = "/ymir/train-status/1";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StatusAsk {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert: Option<Vec<u8>>,
}

/// One training run on a node (no paths).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    pub id: String,
    /// `running`, `done`, `failed` or `cancelled`.
    pub status: String,
    pub base: String,
    #[serde(default)]
    pub adapter: Option<String>,
    #[serde(default)]
    pub adapter_revision: Option<String>,
    #[serde(default)]
    pub dataset: String,
    #[serde(default)]
    pub dataset_license: String,
    #[serde(default)]
    pub teacher: Option<String>,
    #[serde(default)]
    pub rows_used: usize,
    #[serde(default)]
    pub epochs: u32,
    #[serde(default)]
    pub steps: u64,
    #[serde(default)]
    pub final_loss: Option<f64>,
    #[serde(default)]
    pub started_ms: i64,
    #[serde(default)]
    pub ended_ms: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub checkpoints: usize,
    #[serde(default)]
    pub resumed: bool,
}

/// One resumable job on a node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobSummary {
    pub id: String,
    /// `queued`, `running`, `paused`, `failed`, `done` or `cancelled`.
    pub state: String,
    pub base: String,
    pub adapter: String,
    #[serde(default)]
    pub holder: Option<String>,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub checkpoint_epoch: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TrainStatus {
    pub node: String,
    /// The node takes runs from the owner's devices (`[train] remote`).
    #[serde(default)]
    pub remote: bool,
    #[serde(default)]
    pub runs: Vec<RunSummary>,
    #[serde(default)]
    pub jobs: Vec<JobSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum StatusReply {
    Status { status: Box<TrainStatus> },
    Refused { message: String },
}

/// `peer`'s training status. The error is the dialer's, or `Refused` with the node's words.
pub async fn status(dialer: &dyn Dialer, peer: &str, cert: Option<Vec<u8>>) -> Result<TrainStatus, RemoteError> {
    let mut ch = dialer.open(peer, TRAIN_STATUS_PROTOCOL).await?;
    ch.send(&StatusAsk { cert }).await?;
    match ch.recv::<StatusReply>().await? {
        Some(StatusReply::Status { status }) => Ok(*status),
        Some(StatusReply::Refused { message }) => Err(RemoteError::Refused { message }),
        None => Err(RemoteError::Closed),
    }
}

/// The demo nodes' and tests' stand-in for Ymir's side: answers with `status`.
pub(crate) async fn serve_stand_in(mut ch: weirdnet_agent::remote::Channel, status: &TrainStatus) {
    let Ok(Some(_)) = ch.recv::<StatusAsk>().await else { return };
    let _ = ch.send(&StatusReply::Status { status: Box::new(status.clone()) }).await;
}
