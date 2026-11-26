//! Uploads against real `ModelsService`s over in-memory channels: a roomy node that also answers
//! Ymir's `/ymir/uploads/1` (a stand-in that saves), one that keeps more free space than it has,
//! and one without `/ymir/uploads/1` (an older Ymir).

use std::sync::Arc;

use tokio::sync::mpsc::unbounded_channel;
use weirdnet_agent::CancelToken;
use weirdnet_agent::remote::{Authorize, Caller, Dialer, MemoryDialer, RemoteError};
use weirdnet_backends::models::{MODELS_PROTOCOL, ModelsClient, ModelsService};

use super::upload::*;
use super::ymir_uploads::UPLOADS_PROTOCOL;
use super::*;
use crate::demo_models::service_peering;

/// A GGUF model header for `arch`, padded to `len` bytes.
pub(crate) fn gguf(len: usize, arch: &str) -> Vec<u8> {
    let mut out = b"GGUF".to_vec();
    out.extend(3u32.to_le_bytes());
    out.extend(0u64.to_le_bytes());
    let kv: [(&str, Option<u32>); 5] = [
        ("general.architecture", None),
        ("block_count", Some(1)),
        ("attention.head_count", Some(1)),
        ("attention.head_count_kv", Some(1)),
        ("attention.key_length", Some(1)),
    ];
    out.extend((kv.len() as u64).to_le_bytes());
    for (k, v) in kv {
        let k = if v.is_some() { format!("{arch}.{k}") } else { k.to_string() };
        out.extend((k.len() as u64).to_le_bytes());
        out.extend(k.as_bytes());
        match v {
            None => {
                out.extend(8u32.to_le_bytes());
                out.extend((arch.len() as u64).to_le_bytes());
                out.extend(arch.as_bytes());
            }
            Some(n) => {
                out.extend(4u32.to_le_bytes());
                out.extend(n.to_le_bytes());
            }
        }
    }
    out.resize(len, 0);
    out
}

/// A LoRA adapter GGUF header for an `arch` base.
pub(crate) fn lora(len: usize, arch: &str) -> Vec<u8> {
    let mut out = b"GGUF".to_vec();
    out.extend(3u32.to_le_bytes());
    out.extend(0u64.to_le_bytes());
    out.extend(3u64.to_le_bytes());
    for (k, v) in [("general.architecture", arch), ("general.type", "adapter"), ("adapter.type", "lora")] {
        out.extend((k.len() as u64).to_le_bytes());
        out.extend(k.as_bytes());
        out.extend(8u32.to_le_bytes());
        out.extend((v.len() as u64).to_le_bytes());
        out.extend(v.as_bytes());
    }
    out.resize(len, 0);
    out
}

struct Fixture {
    mgr: ModelsManager,
    /// The rollouts on the inference node "inf" (peer-inf).
    rollouts: Arc<std::sync::Mutex<Vec<super::ymir_rollouts::Rollout>>>,
    svcs: Vec<Arc<ModelsService>>,
    files: tempfile::TempDir,
    _dirs: Vec<tempfile::TempDir>,
}

fn fixture() -> Fixture {
    let owner: Arc<dyn Authorize> = Arc::new(|c: &Caller| if c.peer == "me" { Ok(()) } else { Err(RemoteError::refused("unknown-device")) });
    let dialer = MemoryDialer::new();
    let (mut dirs, mut svcs) = (Vec::new(), Vec::new());
    // "peer-copy" (and "peer-roomy") copy uploads from other nodes; the others can't.
    for (peer, keep, uploads) in [("peer-roomy", 0, true), ("peer-full", u64::MAX / 4, true), ("peer-old", 0, false), ("peer-copy", 0, true)] {
        let dir = tempfile::tempdir().unwrap();
        let peers: Option<Arc<dyn Dialer>> = matches!(peer, "peer-roomy" | "peer-copy").then(|| Arc::new(dialer.clone()) as Arc<dyn Dialer>);
        let svc = service_peering(dir.path(), "http://127.0.0.1:9", 1 << 30, owner.clone(), keep, peers);
        let s = svc.clone();
        let protocols: Vec<&str> = if uploads { vec![MODELS_PROTOCOL, UPLOADS_PROTOCOL] } else { vec![MODELS_PROTOCOL] };
        dialer.route_only(peer, &protocols, move |ch, protocol| {
            let s = s.clone();
            tokio::spawn(async move {
                if protocol == UPLOADS_PROTOCOL {
                    super::ymir_uploads::serve_stand_in(ch, &s).await;
                } else {
                    let _ = s.serve(ch, Caller::new("me")).await;
                }
            });
        });
        dirs.push(dir);
        svcs.push(svc);
    }
    let rollouts: Arc<std::sync::Mutex<Vec<super::ymir_rollouts::Rollout>>> = Arc::default();
    let r = rollouts.clone();
    dialer.route_only("peer-inf", &[super::ymir_rollouts::ROLLOUTS_PROTOCOL], move |ch, _| {
        let r = r.clone();
        tokio::spawn(async move { super::ymir_rollouts::serve_stand_in(ch, &r).await });
    });
    let dialer: Arc<dyn Dialer> = Arc::new(dialer);
    let devices = Arc::new(|| {
        vec![
            ModelNode { peer: "peer-roomy".into(), label: "gpu".into() },
            ModelNode { peer: "peer-full".into(), label: "full".into() },
            ModelNode { peer: "peer-old".into(), label: "old".into() },
            ModelNode { peer: "peer-inf".into(), label: "inf".into() },
            ModelNode { peer: "peer-copy".into(), label: "copy".into() },
        ]
    });
    let mgr = ModelsManager::new(ModelsClient::new(dialer.clone()), devices, "http://127.0.0.1:9").with_uploads(dialer, None);
    Fixture { mgr, rollouts, svcs, files: tempfile::tempdir().unwrap(), _dirs: dirs }
}

async fn run(f: &Fixture, spec: UploadSpec) -> Vec<ModelsEvent> {
    let (tx, mut rx) = unbounded_channel();
    f.mgr.upload(spec, &tx, &CancelToken::new()).await;
    drop(tx);
    let mut out = Vec::new();
    while let Some(e) = rx.recv().await {
        out.push(e);
    }
    out
}

fn uploaded(events: &[ModelsEvent]) -> Result<UploadedView, String> {
    events
        .iter()
        .find_map(|e| match e {
            ModelsEvent::Uploaded { result, .. } => Some(result.clone()),
            ModelsEvent::Failed { message } => Some(Err(message.clone())),
            _ => None,
        })
        .expect("an Uploaded or Failed event")
}

#[test]
fn a_file_is_read_here_before_anything_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("My-Tune-Q4_K_M.gguf");
    std::fs::write(&p, gguf(70_000, "llama")).unwrap();
    let (v, _) = inspect(&p).unwrap();
    assert_eq!((v.kind.as_str(), v.format.as_str(), v.architecture.as_deref(), v.size), ("model", "GGUF", Some("llama"), 70_000));
    assert_eq!(v.suggested_id, "my-tune-q4_k_m");
    assert!(v.revision.starts_with("sha256:") && v.summary() == "model, GGUF, llama, 70.0 kB", "{v:?}");
    let a = dir.path().join("tune.gguf");
    std::fs::write(&a, lora(20_000, "llama")).unwrap();
    assert!(inspect(&a).unwrap().0.is_adapter());
    std::fs::write(dir.path().join("x.bin"), b"pickle").unwrap();
    assert!(inspect(&dir.path().join("x.bin")).is_err());
}

#[test]
fn a_license_is_optional_and_checked() {
    assert_eq!(check_license(None).unwrap(), None);
    assert_eq!(check_license(Some("  ")).unwrap(), None);
    assert_eq!(check_license(Some("apache-2.0")).unwrap().as_deref(), Some("apache-2.0"));
    assert!(check_license(Some("mit; rm -rf")).is_err());
    assert!(check_id("my-model").is_ok() && check_id("../x").is_err() && check_id(".hidden").is_err());
}

#[test]
fn pending_uploads_survive_a_restart_until_forgotten() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Pending::load(dir.path());
    assert!(p.uploads.is_empty());
    let spec = UploadSpec::new("/tmp/m.gguf", "peer-a");
    p.put(PendingUpload { spec: spec.clone(), label: "gpu".into(), size: 10, started_ms: 1 });
    p.put(PendingUpload { spec: spec.clone(), label: "gpu".into(), size: 10, started_ms: 2 });
    p.save(dir.path()).unwrap();
    let back = Pending::load(dir.path());
    assert_eq!(back.uploads.len(), 1, "one entry per file and node");
    assert_eq!(back.uploads[0].started_ms, 2);
    let mut back = back;
    back.forget(&spec.path, "peer-a");
    back.save(dir.path()).unwrap();
    assert!(!dir.path().join(PENDING_FILE).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_model_is_uploaded_saved_and_not_sent_twice() {
    let f = fixture();
    let p = f.files.path().join("My-Model-Q4_0.gguf");
    std::fs::write(&p, gguf(300_000, "llama")).unwrap();
    let mut spec = UploadSpec::new(&p, "peer-roomy");
    spec.license = Some("mit".into());
    let ev = run(&f, spec.clone()).await;
    let v = uploaded(&ev).unwrap();
    assert_eq!((v.id.as_str(), v.size, v.already_there), ("my-model-q4_0", 300_000, false));
    assert!(v.saved.as_deref().is_some_and(|s| s.contains("saved in gpu's node.toml")), "{v:?}");
    assert!(v.register_error.is_none() && v.adapter_note.is_none(), "{v:?}");
    assert!(v.rollout_commands[0].contains("--node gpu --name my-model-q4_0 --rollout NAME"), "{v:?}");
    let last = ev.iter().rev().find_map(|e| if let ModelsEvent::Progress { done, total, .. } = e { Some((*done, *total)) } else { None });
    assert_eq!(last, Some((300_000, 300_000)), "progress reaches the end");
    let stored = f.svcs[0].list();
    let e = stored.iter().find(|e| e.id() == "my-model-q4_0").expect("stored on the node");
    let prov = e.provenance().unwrap();
    assert_eq!((prov.origin.as_str(), prov.license.as_deref()), (ORIGIN_UPLOAD, Some("mit")));
    // The list shows it with its origin and license.
    let o = f.mgr.overview().await;
    let row = o.iter().find(|n| n.node.peer == "peer-roomy").unwrap().models.iter().find(|s| s.id == "my-model-q4_0").cloned().unwrap();
    assert_eq!((row.origin.as_deref(), row.license.as_deref()), (Some("upload"), Some("mit")));

    let v = uploaded(&run(&f, spec).await).unwrap();
    assert!(v.already_there, "the same files again: nothing sent");
    // Other files under that name are refused.
    let other = f.files.path().join("other.gguf");
    std::fs::write(&other, gguf(200_000, "llama")).unwrap();
    let mut spec = UploadSpec::new(&other, "peer-roomy");
    spec.name = Some("my-model-q4_0".into());
    let e = uploaded(&run(&f, spec).await).unwrap_err();
    assert!(e.starts_with("gpu refused the upload (exists)") && e.contains("another name"), "{e}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refusals_say_what_to_do() {
    let f = fixture();
    let p = f.files.path().join("m.gguf");
    std::fs::write(&p, gguf(100_000, "llama")).unwrap();
    let e = uploaded(&run(&f, UploadSpec::new(&p, "peer-full")).await).unwrap_err();
    assert!(e.starts_with("full refused the upload (no-space)") && e.contains("free space"), "{e}");
    assert!(f.svcs[1].list().is_empty());
    // An older Ymir stores it but can't save it.
    let v = uploaded(&run(&f, UploadSpec::new(&p, "peer-old")).await).unwrap();
    assert!(v.saved.is_none() && v.register_error.as_deref().is_some_and(|m| m.contains("without /ymir/uploads/1")), "{v:?}");
    // An adapter needs its base, and the base must be on that node.
    let a = f.files.path().join("tune.gguf");
    std::fs::write(&a, lora(20_000, "llama")).unwrap();
    let e = uploaded(&run(&f, UploadSpec::new(&a, "peer-roomy")).await).unwrap_err();
    assert!(e.contains("pick the base model"), "{e}");
    let mut spec = UploadSpec::new(&a, "peer-roomy");
    spec.adapter_for = Some("nothing-here".into());
    let e = uploaded(&run(&f, spec).await).unwrap_err();
    assert!(e.starts_with("gpu refused the upload (no-base)"), "{e}");
    let e = uploaded(&run(&f, UploadSpec::new(&p, "peer-gone")).await).unwrap_err();
    assert!(e.contains("not on your roster"), "{e}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_upload_joins_a_rollout_as_its_next_version() {
    use super::upload::RolloutAsk;
    let f = fixture();
    f.rollouts.lock().unwrap().push(super::ymir_rollouts::Rollout {
        name: "support".into(),
        versions: vec![super::ymir_rollouts::Version { version: 2, model: "old-r2".into(), weight: 100 }],
    });
    let p = f.files.path().join("support-r3.gguf");
    std::fs::write(&p, gguf(100_000, "llama")).unwrap();
    let mut spec = UploadSpec::new(&p, "peer-roomy");
    // Any share from 0 to 100, set exactly: the old version keeps the rest.
    spec.rollout = Some(RolloutAsk { inference: "peer-inf".into(), name: "support".into(), percent: 20 });
    let v = uploaded(&run(&f, spec).await).unwrap();
    let said = v.rollout.clone().expect("asked").unwrap();
    assert!(said.contains("support@3 now gets 20.0%") && said.contains("on inf, saved in its node.toml"), "{said}");
    assert!(v.rollout_commands.is_empty(), "no commands needed: {v:?}");
    let rs = f.rollouts.lock().unwrap().clone();
    let v3 = rs[0].versions.iter().find(|v| v.version == 3).expect("added");
    assert_eq!((v3.model.as_str(), v3.weight), ("support-r3", 20));
    assert_eq!(rs[0].versions.iter().find(|v| v.version == 2).map(|v| v.weight), Some(80));
    let old: RolloutAsk = serde_json::from_str(r#"{"inference":"p","name":"s","weight":5}"#).unwrap();
    assert_eq!(old.percent, 5, "an unfinished upload saved by Bragi 0.9.0 still reads");

    // A node without /ymir/rollouts/1: the upload stands, the rollout says why, the commands show.
    let p = f.files.path().join("other-r1.gguf");
    std::fs::write(&p, gguf(90_000, "llama")).unwrap();
    let mut spec = UploadSpec::new(&p, "peer-roomy");
    spec.rollout = Some(RolloutAsk { inference: "peer-old".into(), name: "support".into(), percent: 0 });
    let v = uploaded(&run(&f, spec).await).unwrap();
    let e = v.rollout.clone().expect("asked").unwrap_err();
    assert!(e.contains("without /ymir/rollouts/1"), "{e}");
    assert!(v.rollout_commands.iter().any(|c| c.contains("--inference NODE")), "{v:?}");
}

/// Copied on to other nodes, node to node: saved there; a node without room is left out first; a
/// node that can't copy from a peer gets it from this device instead.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_upload_is_copied_on_to_other_nodes() {
    let f = fixture();
    let p = f.files.path().join("Pool-Q4_0.gguf");
    std::fs::write(&p, gguf(400_000, "llama")).unwrap();
    let mut spec = UploadSpec::new(&p, "peer-roomy");
    spec.copy_to = vec!["peer-copy".into(), "peer-full".into(), "peer-old".into(), "peer-roomy".into()];
    let v = uploaded(&run(&f, spec).await).unwrap();
    let got: std::collections::HashMap<_, _> = v.copies.iter().cloned().collect();
    assert_eq!(got.len(), 3, "not to the node it was sent to: {v:?}");
    assert_eq!(got["copy"].as_deref(), Ok("copied from gpu, saved in its node.toml"), "{v:?}");
    assert!(got["full"].as_ref().is_err_and(|e| e.starts_with("left out:") && e.contains("it keeps")), "{v:?}");
    // "old" can't copy from a peer, so it is sent from this device; it can't record uploads.
    assert!(
        got["old"].as_ref().is_err_and(|e| e.starts_with("gpu could not copy it to old, so it was sent from this device; old runs a Ymir without")),
        "{v:?}"
    );
    assert!(f.svcs[2].list().iter().any(|e| e.id() == "pool-q4_0"), "sent to peer-old from this device");
    let e = f.svcs[3].list().into_iter().find(|e| e.id() == "pool-q4_0").expect("copied to peer-copy");
    assert_eq!(e.provenance().unwrap().origin, ORIGIN_UPLOAD);
    assert!(!f.svcs[1].list().iter().any(|e| e.id() == "pool-q4_0"));
}

#[test]
fn an_upload_reads_as_the_ymirctl_command_for_a_computer_that_holds_the_file() {
    let label = |p: &str| match p {
        "peer-gpu" => "gpu".to_string(),
        "peer-mini" => "mini".to_string(),
        "peer-inf" => "inference box".to_string(),
        other => other.to_string(),
    };
    let mut s = UploadSpec::new("/phone/tune.gguf", "peer-gpu");
    assert_eq!(s.ymirctl_command("tune.gguf", &label), "ymirctl models upload tune.gguf --node gpu");
    s.name = Some("base+tune".into());
    s.adapter_for = Some("base".into());
    s.license = Some("apache-2.0".into());
    s.persist = false;
    s.copy_to = vec!["peer-mini".into(), "peer-gpu".into()];
    s.rollout = Some(RolloutAsk { inference: "peer-inf".into(), name: "support".into(), percent: 20 });
    assert_eq!(
        s.ymirctl_command("~/models/My Tune.gguf", &label),
        "ymirctl models upload ~/'models/My Tune.gguf' --node gpu,mini --stage gpu --name base+tune --adapter-for base --license apache-2.0 --no-persist --rollout support --inference 'inference box' --canary 20"
    );
}
