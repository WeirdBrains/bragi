//! Uploading a model or LoRA adapter on this device to one of the user's model nodes (Ymir's
//! model-upload plan, slice U4: the desktop dialog). The same path as `ymirctl models upload FILE
//! --node NAME` (Ymir 0.12):
//!
//! 1. [`inspect`] reads and hashes the file here with `weirdnet-model` and shows what it is
//!    (kind, format, architecture, size) before anything is sent.
//! 2. [`ModelsManager::upload`] sends it over models/1 (`ModelsClient::upload`), asking the node
//!    to serve it. The node checks every chunk, keeps the free space its owner set (by default the
//!    larger of 10% and 20 GiB) and refuses with a reason (`no-space`, `exists`, `wrong-arch`, ...).
//!    Cancel, a dropped link or quitting Bragi leaves the parts on the node; uploading the same
//!    files again resumes (the node keeps them for 7 days), and [`Pending`] remembers what was
//!    under way so the shells can offer Resume after a restart.
//! 3. It then asks the node over Ymir's `/ymir/uploads/1` ([`super::ymir_uploads`], temporary)
//!    to record the revision and save the entry in the node's `node.toml`.
//!
//! Rollouts are not set from here yet: the result says which `ymirctl` command adds the upload to
//! one on the inference node.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;
use weirdnet_agent::CancelToken;
use weirdnet_agent::remote::RemoteError;
use weirdnet_backends::AdapterChange;
use weirdnet_backends::models::{Upload, refusal, refusal_reason};
use weirdnet_model::artifact::{Artifact, ArtifactFormat, ArtifactKind, Provenance, inspect as inspect_files};

use super::ymir_rollouts;
use super::ymir_uploads::{self, Register};
use super::{ModelNode, ModelsEvent, ModelsManager, explain, human_bytes};

/// Provenance origin for files a person uploads.
pub const ORIGIN_UPLOAD: &str = "upload";

/// What a file is, read here before anything is sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UploadPreview {
    pub path: PathBuf,
    /// `model`, `adapter` or `projector`.
    pub kind: String,
    /// `GGUF`, `GGUF (3 parts)`, `safetensors`, `MLX`.
    pub format: String,
    pub architecture: Option<String>,
    /// The base model the file names (a Hub id), when it does.
    pub base_model: Option<String>,
    pub size: u64,
    pub files: Vec<String>,
    /// `sha256:…` of the files: the upload's revision and its resume key.
    pub revision: String,
    /// The id it gets unless a name is given: the file name, lower case, without `.gguf`.
    pub suggested_id: String,
}

impl UploadPreview {
    pub fn is_adapter(&self) -> bool {
        self.kind == "adapter"
    }

    /// One line: `model, GGUF, llama, 4.4 GB`.
    pub fn summary(&self) -> String {
        let arch = self.architecture.as_deref().map(|a| format!(", {a}")).unwrap_or_default();
        format!("{}, {}{arch}, {}", self.kind, self.format, human_bytes(self.size))
    }
}

/// Reads and hashes `path` (a file, or a model directory). Nothing leaves this device.
pub fn inspect(path: &Path) -> Result<(UploadPreview, Artifact), String> {
    let artifact = inspect_files(&[path.to_path_buf()]).map_err(|e| format!("{}: {e:#}", path.display()))?;
    let kind = match artifact.kind {
        ArtifactKind::Model => "model",
        ArtifactKind::Adapter { .. } => "adapter",
        ArtifactKind::Projector => "projector",
        _ => "other",
    };
    let format = match artifact.format {
        ArtifactFormat::Gguf { parts: 1 } => "GGUF".to_string(),
        ArtifactFormat::Gguf { parts } => format!("GGUF ({parts} parts)"),
        ArtifactFormat::Safetensors => "safetensors".into(),
        ArtifactFormat::Mlx => "MLX".into(),
        _ => "other".into(),
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = name.strip_suffix(".gguf").or_else(|| name.strip_suffix(".GGUF")).unwrap_or(&name);
    let suggested_id: String =
        stem.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || "._+-".contains(c) { c } else { '-' }).collect();
    let preview = UploadPreview {
        path: path.to_path_buf(),
        kind: kind.into(),
        format,
        architecture: artifact.architecture.clone(),
        base_model: artifact.base_model.clone(),
        size: artifact.size(),
        files: artifact.files.iter().map(|f| f.path.clone()).collect(),
        revision: artifact.revision.clone(),
        suggested_id,
    };
    Ok((preview, artifact))
}

/// A license id, if one is given: optional; letters, digits and `. + - _ ( )` only.
pub fn check_license(license: Option<&str>) -> Result<Option<String>, String> {
    let l = license.map(str::trim).unwrap_or("");
    if l.is_empty() {
        return Ok(None);
    }
    let ok = l.len() <= 64 && l.chars().all(|c| c.is_ascii_alphanumeric() || ".+-_ ()".contains(c));
    if ok { Ok(Some(l.to_string())) } else { Err(format!("{l:?} does not look like a license id (such as mit or apache-2.0)")) }
}

/// An id: one path segment of letters, digits and `._+-`.
pub fn check_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty() && id.len() <= 128 && !id.starts_with('.') && id.chars().all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c));
    if ok { Ok(()) } else { Err(format!("{id:?} is not a usable name: letters, digits and . _ + - only, not starting with a dot")) }
}

/// What to upload, where, and how it is recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadSpec {
    pub path: PathBuf,
    /// The model node, by peer id.
    pub peer: String,
    /// The id it is stored and served as (default: [`UploadPreview::suggested_id`]).
    #[serde(default)]
    pub name: Option<String>,
    /// For an adapter: the served base model on that node.
    #[serde(default)]
    pub adapter_for: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    /// Save it in the node's `node.toml` (the default) so it is served after a restart.
    pub persist: bool,
    /// Add it to a rollout on an inference node once it is stored.
    #[serde(default)]
    pub rollout: Option<RolloutAsk>,
    /// Other model nodes (peer ids) that copy it from `peer` once it is stored there, node to node
    /// (Ymir's pool upload, U6; needs Ymir 0.14.0 on them).
    #[serde(default)]
    pub copy_to: Vec<String>,
}

impl UploadSpec {
    pub fn new(path: impl Into<PathBuf>, peer: impl Into<String>) -> Self {
        Self { path: path.into(), peer: peer.into(), name: None, adapter_for: None, license: None, persist: true, rollout: None, copy_to: Vec::new() }
    }
}

/// A rollout to add an upload to: its next version, on `inference` (a peer id), at `weight`
/// (0 to [`ymir_rollouts::MAX_WEIGHT`]: a canary; more needs an eval, from `ymirctl`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolloutAsk {
    pub inference: String,
    pub name: String,
    pub weight: u32,
}

/// A finished upload on one node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadedView {
    pub id: String,
    pub revision: String,
    pub size: u64,
    /// Bytes the node already held from an earlier, interrupted upload of the same files.
    pub resumed: u64,
    /// The node already had these exact files under this id; nothing was sent.
    pub already_there: bool,
    /// What happened in the node's `node.toml`: saved, not changed and why, or `None` while
    /// unknown.
    pub saved: Option<String>,
    /// A problem recording it there (the files are stored).
    pub register_error: Option<String>,
    /// For a llama.cpp adapter added to a running base: when it is used.
    pub adapter_note: Option<String>,
    /// The rollout it was added to (`NAME@N` and what the inference node said), or why not.
    #[serde(default)]
    pub rollout: Option<Result<String, String>>,
    /// `ymirctl` commands that add it to a rollout, when no rollout was asked for or adding failed.
    pub rollout_commands: Vec<String>,
    /// The other nodes that copied it ([`UploadSpec::copy_to`]): each one's label and what happened.
    #[serde(default)]
    pub copies: Vec<(String, Result<String, String>)>,
}

/// Uploads that were under way (cancelled, cut off, or Bragi quit): kept in the pool's data so
/// the shells can offer Resume. Uploading the same files again resumes from the node's parts.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Pending {
    pub uploads: Vec<PendingUpload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingUpload {
    pub spec: UploadSpec,
    pub label: String,
    pub size: u64,
    pub started_ms: u64,
}

/// File name of [`Pending`] in the active pool's directory.
pub const PENDING_FILE: &str = "uploads.json";

impl Pending {
    pub fn load(dir: &Path) -> Self {
        std::fs::read(dir.join(PENDING_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let path = dir.join(PENDING_FILE);
        if self.uploads.is_empty() {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        std::fs::create_dir_all(dir)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)
    }

    /// Adds or replaces the entry for the same file and node.
    pub fn put(&mut self, p: PendingUpload) {
        self.forget(&p.spec.path, &p.spec.peer);
        self.uploads.push(p);
    }

    pub fn forget(&mut self, path: &Path, peer: &str) {
        self.uploads.retain(|u| !(u.spec.path == path && u.spec.peer == peer));
    }
}

/// What a refused upload means, in words a person can act on (`None`: not an upload refusal).
pub fn explain_refusal(label: &str, e: &RemoteError) -> Option<String> {
    let RemoteError::Refused { message } = e else { return None };
    let reason = refusal_reason(e)?;
    let detail = message.split_once(": ").map_or(message.as_str(), |(_, d)| d);
    let hint = match reason {
        refusal::EXISTS => "pick another name, or remove that one there first",
        refusal::NO_SPACE => {
            "free space there first: uploads leave the free space its owner keeps (by default the larger of 10% and 20 GiB, Ymir's [model] upload_keep_free)"
        }
        refusal::TOO_BIG | refusal::QUOTA => "that node's upload limits refuse it",
        refusal::WRONG_ARCH | refusal::NO_BASE | refusal::NOT_ADAPTER => "an adapter has to be made for a base model served on that node",
        refusal::WRONG_ENGINE => "that node's engine can't serve this format",
        refusal::CHECKSUM => "the bytes arrived changed; the parts were dropped, so upload it again",
        refusal::NOT_ALLOWED => "only devices linked to the owner may upload",
        _ => "",
    };
    let sep = if hint.is_empty() { "" } else { "; " };
    Some(format!("{label} refused the upload ({reason}): {detail}{sep}{hint}"))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn shell_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._+-=:".contains(c)) { s.to_string() } else { format!("'{}'", s.replace('\'', "'\\''")) }
}

impl ModelsManager {
    /// Uploads `spec.path` to `spec.peer`, streaming `Progress` and then one `Uploaded` (or
    /// `Failed`); cancelling stops it (`Cancelled`) and the node keeps the parts.
    pub async fn upload(&self, spec: UploadSpec, tx: &UnboundedSender<ModelsEvent>, cancel: &CancelToken) {
        tokio::select! {
            () = self.upload_inner(spec, tx) => {}
            () = cancel.cancelled() => { let _ = tx.send(ModelsEvent::Cancelled); }
        }
    }

    async fn upload_inner(&self, spec: UploadSpec, tx: &UnboundedSender<ModelsEvent>) {
        let fail = |m: String| {
            let _ = tx.send(ModelsEvent::Failed { message: m });
        };
        let Some(node) = (self.devices)().into_iter().find(|n| n.peer == spec.peer) else {
            return fail("that device is not on your roster".into());
        };
        let license = match check_license(spec.license.as_deref()) {
            Ok(l) => l,
            Err(m) => return fail(m),
        };
        let path = spec.path.clone();
        let inspected = tokio::task::spawn_blocking(move || inspect(&path)).await;
        let (preview, artifact) = match inspected {
            Ok(Ok(x)) => x,
            Ok(Err(m)) => return fail(m),
            Err(e) => return fail(format!("reading the file failed: {e}")),
        };
        if preview.is_adapter() && spec.adapter_for.as_deref().is_none_or(str::is_empty) {
            return fail("pick the base model this adapter runs on".into());
        }
        let id = spec.name.clone().filter(|n| !n.trim().is_empty()).map_or(preview.suggested_id.clone(), |n| n.trim().to_string());
        if let Err(m) = check_id(&id) {
            return fail(m);
        }
        let mut prov = Provenance::for_artifact(&artifact, id.clone(), ORIGIN_UPLOAD);
        prov.license = license;
        prov.uploaded_ms = Some(now_ms());
        if let Some(arch) = &preview.architecture {
            prov.extra.insert("architecture".into(), serde_json::Value::String(arch.clone()));
        }
        if let Some(base) = spec.adapter_for.clone() {
            let mut b = prov.base.clone().unwrap_or_default();
            b.id = Some(base);
            prov.base = Some(b);
        }
        let mut up = Upload::new(id.clone(), &artifact, prov).serving();
        up.adapter_for = spec.adapter_for.clone();
        let copy = up.clone();
        let local = vec![spec.path.clone()];
        let revision = artifact.revision.clone();
        let total = artifact.size();
        let (ptx, peer, label) = (tx.clone(), node.peer.clone(), node.label.clone());
        let (mut first, mut shown) = (None::<u64>, 0u64);
        let sent = self
            .client
            .upload(&node.peer, up, &local, |p| {
                first.get_or_insert(p.done);
                // The node's own count (and its check of the bytes at the end) never moves it back.
                if p.done >= shown {
                    shown = p.done;
                    let _ = ptx.send(ModelsEvent::Progress { peer: peer.clone(), label: label.clone(), done: p.done, total: p.total });
                }
            })
            .await;
        let (pulled, already_there) = match sent {
            Ok(p) => (Some(p), false),
            Err(e) if refusal_reason(&e) == Some(refusal::EXISTS) => {
                let same = self
                    .client
                    .list(&node.peer)
                    .await
                    .ok()
                    .is_some_and(|l| l.iter().any(|x| x.id() == id && x.provenance().is_some_and(|p| p.revision == revision)));
                if !same {
                    return self.upload_failed(tx, &node, &e);
                }
                (None, true)
            }
            Err(e) => return self.upload_failed(tx, &node, &e),
        };
        let adapter_note = pulled.as_ref().filter(|p| p.change == Some(AdapterChange::NextLoad)).map(|p| {
            let base = p.adapter_for.clone().or(spec.adapter_for.clone()).unwrap_or_default();
            format!(
                "`{id}` is added to `{base}` on {}: llama.cpp loads adapters when it starts, so it is used from `{base}`'s next start; while `{base}` runs, the first request for `{id}` restarts it",
                node.label
            )
        });
        let reg = Register {
            cert: None,
            id: id.clone(),
            revision: revision.clone(),
            adapter_for: spec.adapter_for.clone(),
            persist: spec.persist,
            role: None,
        };
        let registered = match &self.uploads {
            Some((dialer, cert)) => ymir_uploads::register(&**dialer, cert.clone(), &node.peer, &node.label, reg).await,
            None => Err("this Bragi can't save uploads on the node".to_string()),
        };
        let (saved, register_error) = match registered {
            Ok(_) if !spec.persist => (Some(format!("not saved in {}'s node.toml: it is served until that node restarts", node.label)), None),
            Ok(r) => (Some(r.note.unwrap_or_else(|| format!("saved in {}'s node.toml", node.label))), None),
            Err(m) => (None, Some(m)),
        };
        let copies = self.copy_to(&spec, &node, &copy, &local, &revision, tx).await;
        let rollout = match (&spec.rollout, &self.uploads) {
            (None, _) => None,
            (Some(_), None) => Some(Err("this Bragi can't reach rollouts".to_string())),
            (Some(r), Some((dialer, cert))) => {
                let label = self.label_of(&r.inference).unwrap_or_else(|| r.inference.clone());
                Some(ymir_rollouts::add_version(&**dialer, cert.clone(), &label, r, &id, spec.persist).await)
            }
        };
        let rollout_commands = if matches!(rollout, Some(Ok(_))) {
            Vec::new()
        } else {
            vec![
                format!(
                    "ymirctl models upload {} --node {} --name {id} --rollout NAME",
                    shell_quote(&spec.path.display().to_string()),
                    shell_quote(&node.label)
                ),
                format!("ymirctl models rollout add NAME@N --model {id} --weight 0 --inference NODE"),
            ]
        };
        let view = UploadedView {
            id,
            revision,
            size: total,
            resumed: if already_there { 0 } else { first.unwrap_or(0).min(total) },
            already_there,
            saved,
            register_error,
            adapter_note,
            rollout,
            rollout_commands,
            copies,
        };
        let _ = tx.send(ModelsEvent::Uploaded { node, result: Ok(view) });
    }

    /// Has the nodes in `spec.copy_to` copy `up` from `stage`, node to node, and saves it in each
    /// one's `node.toml` like an upload. A node without room after what it keeps free is left out
    /// first. A node that can't copy from `stage` (it can't reach it, or runs an older Ymir) gets
    /// `local` from this device instead, as `ymirctl` does; a refusal is reported as it is.
    async fn copy_to(
        &self,
        spec: &UploadSpec,
        stage: &ModelNode,
        up: &Upload,
        local: &[PathBuf],
        revision: &str,
        tx: &UnboundedSender<ModelsEvent>,
    ) -> Vec<(String, Result<String, String>)> {
        let devices = (self.devices)();
        let label = |peer: &str| devices.iter().find(|n| n.peer == peer).map_or_else(|| peer.chars().take(12).collect(), |n| n.label.clone());
        let total = up.files.iter().map(|f| f.size).sum::<u64>();
        let mut out = Vec::new();
        let mut go = Vec::new();
        for peer in spec.copy_to.iter().filter(|p| **p != stage.peer) {
            match self.client.disk(peer).await {
                Ok(d) if d.free.saturating_sub(d.keep_free) < total => out.push((
                    label(peer),
                    Err(format!(
                        "left out: {} free, it keeps {} free, and this needs {}",
                        human_bytes(d.free),
                        human_bytes(d.keep_free),
                        human_bytes(total)
                    )),
                )),
                _ => go.push(peer.clone()),
            }
        }
        if go.is_empty() {
            return out;
        }
        let ptx = tx.clone();
        let copies = self
            .client
            .replicate(&stage.peer, &go, up, |peer, p| {
                let _ = ptx.send(ModelsEvent::Progress { peer: peer.to_string(), label: label(peer), done: p.done, total: p.total });
            })
            .await;
        for (peer, got) in copies {
            let name = label(&peer);
            let node = ModelNode { peer: peer.clone(), label: name.clone() };
            let mut sent_instead = false;
            let r = match got {
                Ok(_) => Ok(()),
                Err(e) if refusal_reason(&e) == Some(refusal::EXISTS) => Ok(()),
                Err(e @ (RemoteError::Refused { .. } | RemoteError::Cancelled)) => {
                    Err(explain_refusal(&name, &e).unwrap_or_else(|| explain(&node, &e)))
                }
                Err(_) => {
                    // It can't copy from the stage: send it from here.
                    sent_instead = true;
                    let ptx = tx.clone();
                    let (p2, l2) = (peer.clone(), name.clone());
                    match self
                        .client
                        .upload(&peer, up.clone(), local, |p| {
                            let _ = ptx.send(ModelsEvent::Progress { peer: p2.clone(), label: l2.clone(), done: p.done, total: p.total });
                        })
                        .await
                    {
                        Ok(_) => Ok(()),
                        Err(e) if refusal_reason(&e) == Some(refusal::EXISTS) => Ok(()),
                        Err(e) => Err(explain_refusal(&name, &e).unwrap_or_else(|| explain(&node, &e))),
                    }
                }
            };
            let how = if sent_instead {
                format!("{} could not copy it to {name}, so it was sent from this device", stage.label)
            } else {
                format!("copied from {}", stage.label)
            };
            let r =
                match r {
                    Err(m) => Err(m),
                    Ok(()) => {
                        let reg = Register {
                            cert: None,
                            id: up.id.clone(),
                            revision: revision.to_string(),
                            adapter_for: spec.adapter_for.clone(),
                            persist: spec.persist,
                            role: None,
                        };
                        match &self.uploads {
                            Some((dialer, cert)) => ymir_uploads::register(&**dialer, cert.clone(), &peer, &name, reg)
                                .await
                                .map(|_| {
                                    if spec.persist { format!("{how}, saved in its node.toml") } else { format!("{how}, not saved in its node.toml") }
                                })
                                .map_err(|e| if sent_instead { format!("{how}; {e}") } else { e }),
                            None => Err("copied, but this Bragi can't save uploads on the node".to_string()),
                        }
                    }
                };
            out.push((name, r));
        }
        out
    }

    fn upload_failed(&self, tx: &UnboundedSender<ModelsEvent>, node: &ModelNode, e: &RemoteError) {
        let message = explain_refusal(&node.label, e).unwrap_or_else(|| explain(node, e));
        let _ = tx.send(ModelsEvent::Uploaded { node: node.clone(), result: Err(message) });
    }
}
