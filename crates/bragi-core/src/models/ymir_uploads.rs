//! TEMPORARY: a minimal client for Ymir's `/ymir/uploads/1` (Ymir `ymir_core::upload_net`), the
//! step after a models/1 upload that records the entry's revision on the node and saves it in
//! the node's `node.toml`. Ymir's crate is not published, so these few wire types are mirrored
//! here, kept to what Bragi sends and reads. This goes away once `weirdnet-backends`'
//! `ModelsService` has an "upload placed" hook (asked of the Weirdnet lead, 2026-11-25); then
//! the node does this itself and Bragi only uploads.

use serde::{Deserialize, Serialize};
use weirdnet_agent::remote::{Dialer, RemoteError};

/// Ymir's protocol id.
pub const UPLOADS_PROTOCOL: &str = "/ymir/uploads/1";

/// The request (JSON), after an upload of `id` placed `revision`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Register {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert: Option<Vec<u8>>,
    pub id: String,
    pub revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_for: Option<String>,
    pub persist: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// What the node did with a [`Register`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Registered {
    pub id: String,
    pub revision: String,
    /// The entry added to node.toml, as TOML (`None`: not saved; see `note`).
    #[serde(default)]
    pub saved: Option<String>,
    /// Why it was not saved, or what to know about the entry.
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum RegisterReply {
    Registered(Registered),
    Failed { message: String },
}

/// Asks node `peer` to record (and, with `persist`, save) an uploaded entry. The error is in
/// words for node `label`.
pub async fn register(dialer: &dyn Dialer, cert: Option<Vec<u8>>, peer: &str, label: &str, mut req: Register) -> Result<Registered, String> {
    req.cert = cert;
    let remote = |e: RemoteError| match e {
        RemoteError::Unsupported { .. } | RemoteError::Protocol { .. } => format!(
            "{label} runs a Ymir without {UPLOADS_PROTOCOL}: the upload is stored there, but it was not saved in its node.toml (update Ymir there)"
        ),
        e => format!("{label}: saving the upload failed ({e}); the files are stored there"),
    };
    let mut ch = dialer.open(peer, UPLOADS_PROTOCOL).await.map_err(remote)?;
    ch.send(&req).await.map_err(remote)?;
    match ch.recv::<RegisterReply>().await {
        Ok(Some(RegisterReply::Registered(r))) => Ok(r),
        Ok(Some(RegisterReply::Failed { message })) => Err(format!("{label}: {message}")),
        Ok(None) => Err(remote(RemoteError::Closed)),
        Err(e) => Err(remote(e)),
    }
}

/// The demo nodes' and tests' stand-in for Ymir's side: checks that `svc` holds the entry at that
/// revision and answers as if it saved it (nothing is written).
pub(crate) async fn serve_stand_in(mut ch: weirdnet_agent::remote::Channel, svc: &weirdnet_backends::models::ModelsService) {
    let Ok(Some(req)) = ch.recv::<Register>().await else { return };
    let have = svc.list().iter().any(|e| e.id() == req.id && e.provenance().is_some_and(|p| p.revision == req.revision));
    let reply = if have {
        RegisterReply::Registered(Registered {
            saved: req.persist.then(|| format!("[[model.local]]\nid = \"{}\"\n", req.id)),
            id: req.id,
            revision: req.revision,
            note: None,
        })
    } else {
        RegisterReply::Failed { message: format!("there is no `{}` on this node", req.id) }
    };
    let _ = ch.send(&reply).await;
}
