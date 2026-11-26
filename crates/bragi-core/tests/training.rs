//! The training view through `Core` over the real Weirdnet stack (the in-memory transport): a
//! linked device reads each demo node's training status (Ymir's `/ymir/train-status/1`, a
//! stand-in) and their rollouts (`/ymir/rollouts/1`, a stand-in). A device that is not linked
//! gets nothing.

use std::time::{Duration, Instant};

use bragi_core::demo::DemoNode;
use bragi_core::{AppState, Core, ErrorKind, NodeStatus, PairState};

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
async fn a_linked_device_sees_each_nodes_training() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = DemoNode::start_cloud(&tmp.path().join("demo"), Duration::from_millis(5)).await.unwrap();
    let core = Core::start(demo.app_config(&tmp.path().join("app"), "Bragi test")).unwrap();
    assert_eq!(core.training_overview().await.unwrap_err().kind, ErrorKind::NotPaired);

    demo.auto_accept(&core);
    core.pair_offer().await.unwrap();
    until(&core, "check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    core.pair_confirm(true).unwrap();
    until(&core, "linked", |s| matches!(s.pair, PairState::Linked { .. }) && s.online).await;

    let deadline = Instant::now() + Duration::from_secs(20);
    let o = loop {
        let o = core.training_overview().await.unwrap();
        if o.nodes.len() == 2 && o.nodes.iter().all(|n| n.status == NodeStatus::Ready) {
            break o;
        }
        assert!(Instant::now() < deadline, "owner auth never admitted the device: {o:?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    for n in &o.nodes {
        assert!(n.remote);
        assert_eq!(n.runs.len(), 1);
        assert_eq!(n.runs[0].status, "done");
        assert!(n.runs[0].adapter.as_deref().is_some_and(|a| a.ends_with("+demo")));
    }
    // The demo nodes start with no rollouts.
    assert!(o.rollouts.is_empty());
}
