//! B4: push wake-ups over the relay wake path. Two Bragi cores linked to the same demo node
//! share wake cards over sync (sealed, full devices only); one turns relay wake on and keeps a
//! `/weirdnet/wake/1` connection to a stand-in relay; the other wakes it through that relay.
//! Memory transport and test identities only.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bragi_core::demo::DemoNode;
use bragi_core::demo_wake::WakeRelay;
use bragi_core::wake::WakeSettings;
use bragi_core::{AppState, Core, PairState};
use weirdnet_push::Reason;
use weirdnet_push::relaywake::Delivery;

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

async fn link(demo: &Arc<DemoNode>, dir: &std::path::Path, label: &str, relay: &str) -> Core {
    let cfg = bragi_core::Config { wake_relay: Some(relay.to_string()), ..demo.app_config(dir, label) };
    let core = Core::start(cfg).unwrap();
    demo.auto_accept(&core);
    core.pair_offer().await.unwrap();
    until(&core, "the code check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    core.pair_confirm(true).unwrap();
    until(&core, "linked", |s| matches!(s.pair, PairState::Linked { .. }) && s.online).await;
    core
}

async fn sync_until(demo: &DemoNode, cores: &[&Core], what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !f() {
        assert!(Instant::now() < deadline, "waiting for {what}");
        for c in cores {
            c.sync_now().await.unwrap();
            // Writes the card once the key chain allows (the sync loop does this too).
            c.set_wake_settings(c.wake_settings()).unwrap();
        }
        let addrs: Vec<String> = cores.iter().filter_map(|c| c.address()).collect();
        demo.sync_with(&addrs).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_linked_device_wakes_another_through_its_relay() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = WakeRelay::start(Duration::from_secs(1)).await.unwrap();
    let demo = Arc::new(DemoNode::start(&tmp.path().join("demo"), Duration::from_millis(5)).await.unwrap());
    let a = link(&demo, &tmp.path().join("a"), "Bragi A", &relay.address()).await;
    let b = link(&demo, &tmp.path().join("b"), "Bragi B", &relay.address()).await;
    a.add_peer(&b.address().unwrap()).unwrap();
    b.add_peer(&a.address().unwrap()).unwrap();

    // Defaults: push on, relay wake off (opt-in); nothing connects to the relay.
    assert_eq!(b.wake_settings(), WakeSettings { push: true, relay_wake: false });
    assert!(!b.state().wake.relay_connected);
    assert_eq!(relay.subscribed(), 0);

    b.set_wake_settings(WakeSettings { push: true, relay_wake: true }).unwrap();
    until(&b, "the relay wake connection", |s| s.wake.relay_connected).await;
    assert_eq!(relay.subscribed(), 1);

    sync_until(&demo, &[&a, &b], "B's wake card on A", || a.wakeable_devices().iter().any(|d| d.label == "Bragi B" && d.through_relay)).await;
    let bid = a.wakeable_devices().into_iter().find(|d| d.label == "Bragi B").unwrap().peer_id;
    // The demo node (the user's other device, not a Bragi) shares no card.
    assert_eq!(a.wakeable_devices().len(), 1);

    assert_eq!(a.wake_device(&bid, Reason::MessageWaiting).await.unwrap(), Delivery::Delivered);
    until(&b, "the wake on B", |s| s.wake.last_wake_ms.is_some()).await;
    assert_eq!(b.state().wake.refused, 0);
    assert_eq!(relay.delivered(), 1);

    // B turns push off: its card is withdrawn and the relay connection closes.
    b.set_wake_settings(WakeSettings { push: false, relay_wake: true }).unwrap();
    until(&b, "the relay wake connection closing", |s| !s.wake.relay_connected).await;
    sync_until(&demo, &[&a, &b], "B's card withdrawn on A", || a.wakeable_devices().is_empty()).await;
    assert!(a.wake_device(&bid, Reason::MessageWaiting).await.is_err());
}
