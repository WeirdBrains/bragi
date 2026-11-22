//! Model management through `Core` over the real Weirdnet stack (the in-memory transport): a Bragi
//! device linked to the owner is admitted by the demo model nodes' `RosterAuth` (the same hook
//! `ymird` uses for `/weirdnet-backends/models/1`), searches a mock Hub with a fit per node, pulls
//! with progress, lists and removes. A device that is not linked is refused.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bragi_core::demo::DemoNode;
use bragi_core::{AppState, Core, ErrorKind, ModelsEvent, NodeStatus, PairState, PullSpec, PullTarget, Verdict};

async fn until(core: &Core, what: &str, f: impl Fn(&AppState) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut w = core.watch();
    loop {
        let s = w.borrow_and_update().clone();
        if f(&s) {
            return;
        }
        assert!(Instant::now() < deadline, "waiting for {what}: {s:?}");
        let _ = tokio::time::timeout(Duration::from_millis(200), w.changed()).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_linked_device_manages_models_and_an_unlinked_one_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = DemoNode::start_cloud(&tmp.path().join("demo"), Duration::from_millis(5)).await.unwrap();
    let core = Core::start(demo.app_config(&tmp.path().join("app"), "Bragi test")).unwrap();

    // Not linked yet: the owner's nodes are not ours to ask.
    let err = core.models_overview().await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotPaired);

    demo.auto_accept(&core);
    core.pair_offer().await.unwrap();
    until(&core, "check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    core.pair_confirm(true).unwrap();
    until(&core, "linked", |s| matches!(s.pair, PairState::Linked { .. }) && s.online).await;

    // Both demo nodes take models/1 (the roster arrives with the first sync pass).
    let deadline = Instant::now() + Duration::from_secs(20);
    let nodes = loop {
        let nodes = core.models_overview().await.unwrap();
        if nodes.len() == 2 && nodes.iter().all(|n| n.status == NodeStatus::Ready) {
            break nodes;
        }
        assert!(Instant::now() < deadline, "owner auth never admitted the device: {nodes:?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    assert!(nodes.iter().all(|n| n.models.is_empty()));
    let (big, small) = {
        let by = |label: &str| nodes.iter().find(|n| n.node.label.ends_with(label)).map(|n| n.node.clone()).unwrap();
        (by("inference node"), by("inference node 2"))
    };

    // Search: the Hub's hits, and a fit per node (24 GB and 11 GB of model memory).
    let mut run = core.models_search("qwen").unwrap();
    let mut hits = Vec::new();
    let mut fits = Vec::new();
    while let Some(e) = run.events.recv().await {
        match e {
            ModelsEvent::Hits(h) => hits = h,
            ModelsEvent::Fit { repo, fit } => fits.push((repo, fit)),
            ModelsEvent::SearchDone { error } => {
                assert_eq!(error, None);
                break;
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(hits.iter().map(|h| h.repo.as_str()).collect::<Vec<_>>(), ["mradermacher/DeepSeek-V4-Pro-Qwen3.5-9B-GGUF", "unsloth/Qwen3-14B-GGUF"]);
    let fit_on = |peer: &str| fits.iter().find(|(r, f)| r == "unsloth/Qwen3-14B-GGUF" && f.node.peer == peer).map(|(_, f)| f.clone()).unwrap();
    assert_eq!(fit_on(&big.peer).best.as_deref(), Some("Qwen3-14B-Q8_0"), "24 GB holds the 15.7 GB one");
    assert_eq!(fit_on(&small.peer).verdict(), Verdict::TooBig, "the 9 GB Q4 does not fit 11 GB with its cache and headroom");
    let deepseek = |peer: &str| fits.iter().find(|(r, f)| r.contains("DeepSeek") && f.node.peer == peer).map(|(_, f)| f.best.clone()).unwrap();
    assert_eq!(deepseek(&big.peer).as_deref(), Some("DeepSeek-V4-Pro-Qwen3.5-9B.Q8_0"));
    assert_eq!(deepseek(&small.peer).as_deref(), Some("DeepSeek-V4-Pro-Qwen3.5-9B.Q6_K"), "11 GB holds the Q6 but not the Q8");
    assert_eq!(fit_on(&big.peer).verdict(), Verdict::Fits);

    // Pull to the best node with streamed progress, then it is listed and served.
    let mut run = core.models_pull(PullSpec::new("demo/Tiny-Chat-GGUF", PullTarget::Auto)).unwrap();
    let (mut chosen, mut progress, mut done) = (None, 0, None);
    while let Some(e) = run.events.recv().await {
        match e {
            ModelsEvent::Chosen { node, .. } => chosen = Some(node),
            ModelsEvent::Progress { done, total, .. } => {
                assert!(done <= total);
                progress += 1;
            }
            ModelsEvent::NodeDone { result, .. } => done = Some(result.unwrap()),
            ModelsEvent::Done => break,
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(chosen.unwrap().peer, big.peer);
    assert!(progress > 1);
    let done = done.unwrap();
    assert_eq!((done.id.as_str(), done.size, done.served), ("Tiny-Chat-Q8_0", 5_000_000, true));
    let nodes = core.models_overview().await.unwrap();
    let on_big = nodes.iter().find(|n| n.node.peer == big.peer).unwrap();
    assert_eq!(on_big.models.len(), 1);
    assert_eq!(on_big.models[0].served_as.as_deref(), Some("Tiny-Chat-Q8_0"));

    // Remove: refused without the confirm, freed with it.
    assert!(core.models_remove(&big.peer, "Tiny-Chat-Q8_0", false).await.is_err());
    assert_eq!(core.models_remove(&big.peer, "Tiny-Chat-Q8_0", true).await, Ok(5_000_000));
    assert!(core.models_overview().await.unwrap().iter().all(|n| n.models.is_empty()));
    let _ = Arc::new(demo);
}
