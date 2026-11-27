//! B7's check against real Ymir nodes: a headless Bragi core with throwaway state is linked to the
//! user's pool, then runs the Models screen's calls (overview, Hub search with fit badges, pull with
//! progress, cancel and resume, remove with a confirm, upload, the training view) and, once the
//! script has revoked it, checks that the nodes refuse it. Not shipped; it prints one `KEY value`
//! line per step for the operator.
//!
//! `b7_real_node DIR NODE_ADDR WORK TARGET_PEER REPO FILE UPLOAD_GGUF`: `DIR` is Bragi's data
//! folder (fresh), `NODE_ADDR` a node to dial, `WORK` a folder shared with the operator for the
//! link code and the revoke hand-off, `TARGET_PEER` the model node to pull to and upload to,
//! `REPO`/`FILE` a small GGUF on the Hub, `UPLOAD_GGUF` a GGUF on this device.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use bragi_core::models::upload::UploadSpec;
use bragi_core::models::{ModelsEvent, NodeStatus, PullSpec, PullTarget};
use bragi_core::{Config, Core, PairState, Transport};

async fn until<T>(what: &str, secs: u64, mut f: impl FnMut() -> Option<T>) -> Result<T> {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f() {
            return Ok(v);
        }
        if Instant::now() > end {
            bail!("timed out waiting for {what}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn status(s: &NodeStatus) -> String {
    match s {
        NodeStatus::NeedsOwnerDevice(m) => format!("needs-owner-device ({m})"),
        NodeStatus::Unreachable(m) => format!("unreachable ({m})"),
        other => format!("{other:?}").to_lowercase(),
    }
}

async fn overview(core: &Core, tag: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for n in core.models_overview().await? {
        println!("{tag} {} [{}] {} models={}", n.node.label, &n.node.peer[n.node.peer.len() - 6..], status(&n.status), n.models.len());
        out.push((n.node.peer.clone(), status(&n.status)));
    }
    Ok(out)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [dir, addr, work, target, repo, file, upload] = <[String; 7]>::try_from(a).ok().context("7 arguments, see the header")?;
    let (dir, work, upload) = (PathBuf::from(dir), PathBuf::from(work), PathBuf::from(upload));

    let mut cfg = Config::new(&dir, "Bragi B7 test");
    cfg.transport = Transport::Internet;
    cfg.peers = vec![addr];
    let core = Core::start(cfg)?;

    // Link: Bragi shows the code, `ymirctl pool add` on a device with the identity opens it.
    let link = core.pair_offer().await?;
    std::fs::write(work.join("bragi.link"), &link)?;
    let sas = until("the code check", 180, || match core.state().pair {
        PairState::Confirm { sas } => Some(sas),
        PairState::Failed { message } => panic!("pairing failed: {message}"),
        _ => None,
    })
    .await?;
    std::fs::write(work.join("bragi.sas"), &sas)?;
    core.pair_confirm(true)?;
    until("linked and online", 90, || matches!(core.state().pair, PairState::Linked { .. }).then(|| core.state().online).filter(|o| *o)).await?;
    println!("LINKED");
    tokio::time::sleep(Duration::from_secs(10)).await;

    // What each node has.
    let t = Instant::now();
    overview(&core, "NODE").await?;
    println!("OVERVIEW_MS {}", t.elapsed().as_millis());

    // The training view (a read).
    let tr = core.training_overview().await?;
    for n in &tr.nodes {
        println!("TRAIN {} {} remote={} runs={} jobs={}", n.node.label, status(&n.status), n.remote, n.runs.len(), n.jobs.len());
    }
    println!("ROLLOUTS {}", tr.rollouts.iter().map(|r| format!("{}@{}:{}", r.name, r.node.label, r.versions.len())).collect::<Vec<_>>().join(" "));

    // Hub search with a fit badge per node.
    let t = Instant::now();
    let mut run = core.models_search(&repo)?;
    let (mut hits, mut fits) = (0, 0);
    while let Some(e) = run.events.recv().await {
        match e {
            ModelsEvent::Hits(h) => hits = h.len(),
            ModelsEvent::Fit { repo: r, fit } if r == repo => {
                fits += 1;
                println!("FIT {} {} {:?} best={:?}", fit.node.label, status(&fit.status), fit.verdict(), fit.best);
            }
            ModelsEvent::Fit { .. } => fits += 1,
            ModelsEvent::SearchDone { error } => {
                println!("SEARCH hits={hits} fits={fits} error={error:?} ms={}", t.elapsed().as_millis());
                break;
            }
            _ => {}
        }
    }

    // Pull to one node, cancel after the first progress, then pull again (it resumes).
    let mut spec = PullSpec::new(repo.clone(), PullTarget::Node(target.clone()));
    spec.model = Some(file.clone());
    spec.serve = false;
    let mut run = core.models_pull(spec.clone())?;
    let mut cancelled = false;
    while let Some(e) = run.events.recv().await {
        match e {
            ModelsEvent::Progress { done, total, .. } if !cancelled && done > 0 => {
                println!("PULL1_PROGRESS {done}/{total}");
                cancelled = core.cancel(&run.id);
            }
            ModelsEvent::Cancelled => {
                println!("PULL1 cancelled");
                break;
            }
            ModelsEvent::NodeDone { result, .. } => println!("PULL1_NODE {result:?}"),
            ModelsEvent::Failed { message } => println!("PULL1_FAILED {message}"),
            ModelsEvent::Done => {
                println!("PULL1 done before the cancel");
                break;
            }
            _ => {}
        }
    }
    let t = Instant::now();
    let mut run = core.models_pull(spec)?;
    let mut pulled = None;
    let mut last = 0u64;
    while let Some(e) = run.events.recv().await {
        match e {
            ModelsEvent::Progress { done, total, .. } => {
                if last == 0 {
                    println!("PULL2_FIRST_PROGRESS {done}/{total}");
                }
                last = done;
            }
            ModelsEvent::NodeDone { result, .. } => {
                println!("PULL2_NODE {result:?}");
                pulled = result.ok().map(|v| v.id);
            }
            ModelsEvent::Failed { message } => println!("PULL2_FAILED {message}"),
            ModelsEvent::Done | ModelsEvent::Cancelled => break,
            _ => {}
        }
    }
    println!("PULL2 last={last} ms={}", t.elapsed().as_millis());
    overview(&core, "AFTER_PULL").await?;

    // Remove: refused without the confirm, then with it.
    if let Some(id) = &pulled {
        println!("REMOVE_UNCONFIRMED {:?}", core.models_remove(&target, id, false).await);
        println!("REMOVE {:?}", core.models_remove(&target, id, true).await);
    }

    // Upload a GGUF from this device (not persisted in the node's node.toml), then remove it.
    let p = core.models_inspect_upload(upload.clone()).await;
    println!("UPLOAD_PREVIEW {:?}", p.as_ref().map(|p| (&p.suggested_id, p.size)));
    let spec = UploadSpec {
        path: upload,
        peer: target.clone(),
        name: Some("b7-upload-test".into()),
        adapter_for: None,
        license: Some("apache-2.0".into()),
        persist: false,
        rollout: None,
        copy_to: Vec::new(),
    };
    let t = Instant::now();
    let mut run = core.models_upload(spec)?;
    while let Some(e) = run.events.recv().await {
        match e {
            ModelsEvent::Uploaded { result, .. } => {
                println!("UPLOADED {result:?} ms={}", t.elapsed().as_millis());
                if let Ok(v) = result {
                    println!("UPLOAD_REMOVE {:?}", core.models_remove(&target, &v.id, true).await);
                }
                break;
            }
            ModelsEvent::Failed { message } => {
                println!("UPLOAD_FAILED {message}");
                break;
            }
            ModelsEvent::Cancelled => break,
            _ => {}
        }
    }

    // Hand off: the operator revokes this device; then the nodes must refuse it.
    std::fs::write(work.join("ready-to-revoke"), core.peer_id().unwrap_or_default())?;
    println!("WAITING_FOR_REVOKE");
    until("the revoke", 600, || work.join("revoked").exists().then_some(())).await?;
    tokio::time::sleep(Duration::from_secs(20)).await;
    overview(&core, "REVOKED").await?;
    println!("END");
    Ok(())
}
