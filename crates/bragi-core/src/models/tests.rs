//! `ModelsManager` against mock nodes (real `ModelsService`s over in-memory channels, the same
//! protocol messages Weirdnet carries) and a mock Hub: search with a fit per node, `auto`,
//! one node, all, progress and cancel, list, remove with its confirm, and the refusals.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use weirdnet_agent::remote::{Authorize, Caller, Dialer, MemoryDialer, RemoteError};
use weirdnet_backends::models::{MODELS_PROTOCOL, ModelsClient, ModelsService};

use super::*;
use crate::demo_models::{MockRepo, mock_hub, service};

struct Fixture {
    mgr: ModelsManager,
    _dirs: Vec<tempfile::TempDir>,
    svcs: Vec<Arc<ModelsService>>,
}

fn node(peer: &str, label: &str) -> ModelNode {
    ModelNode { peer: peer.into(), label: label.into() }
}

/// `big` (43 kB of model memory) and `small` (41 kB; an 8192-token cache costs 32 kB here), plus `old` (no models/1) and a `locked` node that
/// refuses this caller. The repo `o/n` has a 4 kB Q4_0 and a 6 kB Q8_0; `o/huge` has a 90 kB one.
fn fixture(delay: Duration, caller: &'static str) -> Fixture {
    let hub = mock_hub(
        vec![
            MockRepo::new("o/n", 900, 4, &[("n-Q4_0.gguf", 4000), ("n-Q8_0.gguf", 6000)]).downloadable(),
            MockRepo::new("o/slow", 5, 0, &[("slow-Q4_0.gguf", 400_000)]).downloadable(),
            MockRepo::new("o/huge", 100, 1, &[("huge-Q4_K_M.gguf", 90_000)]),
        ],
        delay,
    );
    let owner: Arc<dyn Authorize> =
        Arc::new(move |c: &Caller| if c.peer == "me" && caller == "me" { Ok(()) } else { Err(RemoteError::refused("unknown-device")) });
    let mut dirs = Vec::new();
    let dialer = MemoryDialer::new();
    let mut svcs = Vec::new();
    for (peer, usable) in [("peer-big", 43_000u64), ("peer-small", 41_000)] {
        let dir = tempfile::tempdir().unwrap();
        let svc = service(dir.path(), &hub, usable, owner.clone());
        let s = svc.clone();
        dialer.route_only(peer, &[MODELS_PROTOCOL], move |ch, _| {
            let s = s.clone();
            tokio::spawn(async move {
                let _ = s.serve(ch, Caller::new("me")).await;
            });
        });
        dirs.push(dir);
        svcs.push(svc);
    }
    // A node whose owner check refuses this device.
    let dir = tempfile::tempdir().unwrap();
    let locked: Arc<dyn Authorize> = Arc::new(|_: &Caller| Err(RemoteError::refused("needs-admission")));
    let svc = service(dir.path(), &hub, 43_000, locked);
    dialer.route_only("peer-locked", &[MODELS_PROTOCOL], move |ch, _| {
        let s = svc.clone();
        tokio::spawn(async move {
            let _ = s.serve(ch, Caller::new("me")).await;
        });
    });
    dirs.push(dir);
    dialer.route_only("peer-old", &["/weirdnet-backends/node/1"], |_, _| {});
    let dialer: Arc<dyn Dialer> = Arc::new(dialer);
    let devices = Arc::new(|| vec![node("peer-big", "Big"), node("peer-small", "small"), node("peer-old", "old"), node("peer-locked", "locked")]);
    Fixture { mgr: ModelsManager::new(ModelsClient::new(dialer), devices, hub), _dirs: dirs, svcs }
}

async fn drain(mut rx: UnboundedReceiver<ModelsEvent>) -> Vec<ModelsEvent> {
    let mut out = Vec::new();
    while let Some(e) = rx.recv().await {
        out.push(e);
    }
    out
}

async fn search(f: &Fixture, q: &str) -> Vec<ModelsEvent> {
    let (tx, rx) = unbounded_channel();
    f.mgr.search(q, 10, &tx).await;
    drop(tx);
    drain(rx).await
}

async fn pull(f: &Fixture, spec: PullSpec) -> Vec<ModelsEvent> {
    let (tx, rx) = unbounded_channel();
    f.mgr.pull(spec, &tx, &CancelToken::new()).await;
    drop(tx);
    drain(rx).await
}

fn fit_of<'a>(events: &'a [ModelsEvent], repo: &str, peer: &str) -> &'a NodeFit {
    events
        .iter()
        .find_map(|e| match e {
            ModelsEvent::Fit { repo: r, fit } if r == repo && fit.node.peer == peer => Some(fit),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no fit for {repo} on {peer}: {events:?}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn overview_says_who_takes_model_management() {
    let f = fixture(Duration::ZERO, "me");
    let o = f.mgr.overview().await;
    let status = |p: &str| o.iter().find(|n| n.node.peer == p).unwrap().status.clone();
    assert_eq!(status("peer-big"), NodeStatus::Ready);
    assert_eq!(status("peer-old"), NodeStatus::NotModelNode);
    assert_eq!(status("peer-locked"), NodeStatus::NeedsOwnerDevice("needs-admission".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn search_shows_a_fit_badge_per_node() {
    let f = fixture(Duration::ZERO, "me");
    let ev = search(&f, "o").await;
    let ModelsEvent::Hits(hits) = &ev[0] else { panic!("{ev:?}") };
    assert_eq!(hits.iter().map(|h| h.repo.as_str()).collect::<Vec<_>>(), ["o/n", "o/slow", "o/huge"]);
    assert_eq!((hits[0].downloads, hits[0].likes), (900, 4));
    // o/n: the big node holds the 6 kB one, the small node only the 4 kB one.
    let big = fit_of(&ev, "o/n", "peer-big");
    assert_eq!((big.verdict(), big.best.as_deref(), big.best_size), (Verdict::Fits, Some("n-Q8_0"), 6000));
    assert_eq!(big.usable_bytes, 43_000);
    assert_eq!(fit_of(&ev, "o/n", "peer-small").best.as_deref(), Some("n-Q4_0"));
    // o/huge fits neither.
    assert_eq!(fit_of(&ev, "o/huge", "peer-big").verdict(), Verdict::TooBig);
    // The nodes that could not be asked still get a badge that says why.
    assert_eq!(fit_of(&ev, "o/n", "peer-old").verdict(), Verdict::Unknown);
    assert_eq!(fit_of(&ev, "o/n", "peer-locked").status, NodeStatus::NeedsOwnerDevice("needs-admission".into()));
    assert_eq!(ev.last(), Some(&ModelsEvent::SearchDone { error: None }));
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_picks_the_node_that_fits_and_says_why() {
    let f = fixture(Duration::ZERO, "me");
    let ev = pull(&f, PullSpec::new("o/n", PullTarget::Auto)).await;
    let ModelsEvent::Chosen { node, model, size, why } = &ev[0] else { panic!("{ev:?}") };
    assert_eq!((node.label.as_str(), model.as_str(), *size), ("Big", "n-Q8_0", 6000));
    assert!(why.contains("Big") && why.contains("6.0 kB") && why.contains("43.0 kB"), "{why}");
    assert!(ev.iter().any(|e| matches!(e, ModelsEvent::Progress { done, total, .. } if done <= total && *total == 6000)), "{ev:?}");
    let done = ev.iter().find_map(|e| match e {
        ModelsEvent::NodeDone { result, .. } => Some(result.clone()),
        _ => None,
    });
    assert_eq!(done, Some(Ok(PulledView { id: "n-Q8_0".into(), size: 6000, served: true, loaded: false })));
    assert_eq!(ev.last(), Some(&ModelsEvent::Done));
    // Nothing fits: no node is picked and nothing is downloaded.
    let ev = pull(&f, PullSpec::new("o/huge", PullTarget::Auto)).await;
    assert!(matches!(&ev[..], [ModelsEvent::Failed { message }] if message.contains("fits any node")), "{ev:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_named_node_and_all_nodes() {
    let f = fixture(Duration::ZERO, "me");
    let mut spec = PullSpec::new("o/n", PullTarget::Node("peer-small".into()));
    spec.serve = false;
    let ev = pull(&f, spec).await;
    // The small node's best is the 4 kB one; the repo's best fit is picked on that node.
    assert!(ev.iter().any(|e| matches!(e, ModelsEvent::NodeDone { node, result: Ok(p) } if node.label == "small" && p.id == "n-Q4_0" && !p.served)), "{ev:?}");
    let ev = pull(&f, PullSpec::new("o/n", PullTarget::All)).await;
    let done: Vec<_> = ev
        .iter()
        .filter_map(|e| match e {
            ModelsEvent::NodeDone { node, result } => Some((node.label.clone(), result.is_ok())),
            _ => None,
        })
        .collect();
    assert_eq!(done.len(), 2, "only the nodes that took models/1: {done:?}");
    assert!(done.iter().all(|(_, ok)| *ok));
    // Listed per node afterwards.
    let o = f.mgr.overview().await;
    let ids = |p: &str| o.iter().find(|n| n.node.peer == p).unwrap().models.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids("peer-big"), ["n-Q8_0"]);
    assert_eq!(ids("peer-small"), ["n-Q4_0"]);
    assert_eq!(o.iter().find(|n| n.node.peer == "peer-big").unwrap().models[0].served_as.as_deref(), Some("n-Q8_0"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_node_that_refuses_reports_it_without_stopping_the_others() {
    let f = fixture(Duration::ZERO, "me");
    let ev = pull(&f, PullSpec::new("o/n", PullTarget::Node("peer-locked".into()))).await;
    assert!(
        ev.iter().any(|e| matches!(e, ModelsEvent::NodeDone { result: Err(m), .. } if m.contains("only devices linked to its owner"))),
        "{ev:?}"
    );
    // Not the owner at all: every node refuses.
    let f = fixture(Duration::ZERO, "stranger");
    let ev = pull(&f, PullSpec::new("o/n", PullTarget::Auto)).await;
    assert!(matches!(&ev[..], [ModelsEvent::Failed { .. }]), "{ev:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_stops_the_pull_and_keeps_the_partial_file() {
    let f = Arc::new(fixture(Duration::from_millis(60), "me"));
    let (tx, mut rx) = unbounded_channel();
    let cancel = CancelToken::new();
    let mut spec = PullSpec::new("o/slow", PullTarget::Node("peer-big".into()));
    spec.model = Some("slow-Q4_0".into());
    let run = tokio::spawn({
        let (f, cancel) = (f.clone(), cancel.clone());
        async move { f.mgr.pull(spec, &tx, &cancel).await }
    });
    // Wait for the first progress, then cancel.
    loop {
        match rx.recv().await {
            Some(ModelsEvent::Progress { .. }) => break,
            Some(_) => {}
            None => panic!("no progress"),
        }
    }
    cancel.cancel();
    run.await.unwrap();
    let mut rest = Vec::new();
    while let Some(e) = rx.recv().await {
        rest.push(e);
    }
    assert!(rest.contains(&ModelsEvent::Cancelled), "{rest:?}");
    assert!(!rest.iter().any(|e| matches!(e, ModelsEvent::Done)));
    // Nothing complete is listed; pulling again finishes it.
    assert!(f.mgr.overview().await.iter().all(|n| n.models.is_empty()));
    let mut again = PullSpec::new("o/slow", PullTarget::Node("peer-big".into()));
    again.model = Some("slow-Q4_0".into());
    let ev = pull(&f, again).await;
    assert!(ev.iter().any(|e| matches!(e, ModelsEvent::NodeDone { result: Ok(_), .. })), "{ev:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_needs_a_confirm_and_frees_the_space() {
    let f = fixture(Duration::ZERO, "me");
    pull(&f, PullSpec::new("o/n", PullTarget::Node("peer-big".into()))).await;
    assert!(f.mgr.remove("peer-big", "n-Q8_0", false).await.unwrap_err().contains("confirm"));
    assert_eq!(f.mgr.overview().await[0].models.len(), 1, "the unconfirmed call removed nothing");
    assert_eq!(f.mgr.remove("peer-big", "n-Q8_0", true).await, Ok(6000));
    assert!(f.mgr.overview().await[0].models.is_empty());
    assert!(f.mgr.remove("peer-big", "n-Q8_0", true).await.is_err(), "already gone");
    assert!(f.mgr.remove("nobody", "x", true).await.unwrap_err().contains("roster"));
    assert_eq!(f.svcs.len(), 2);
}

#[test]
fn sizes_read_like_sentences() {
    assert_eq!(human_bytes(512), "512 B");
    assert_eq!(human_bytes(4_400_000_000), "4.4 GB");
}
