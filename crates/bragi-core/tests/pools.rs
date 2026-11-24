//! Saved Ymir pools: a device can be linked to more than one cloud, switch between them,
//! and keep each pool's history apart. Memory transport, test identities.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bragi_core::demo::{DemoNode, WARM_MODEL, answer_for};
use bragi_core::{AppState, ChatEvent, ChatHandle, Core, ErrorKind, PairState, Preference};

async fn until(core: &Core, what: &str, f: impl Fn(&AppState) -> bool) -> AppState {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut w = core.watch();
    loop {
        let s = w.borrow_and_update().clone();
        if f(&s) {
            return s;
        }
        assert!(Instant::now() < deadline, "waiting for {what}: {s:?}");
        let _ = tokio::time::timeout(Duration::from_millis(200), w.changed()).await;
    }
}

async fn collect(h: &mut ChatHandle) -> String {
    let mut text = String::new();
    while let Some(ev) = tokio::time::timeout(Duration::from_secs(20), h.events.recv()).await.expect("stream stalled") {
        if let ChatEvent::Delta { offset, text: t } = &ev {
            let keep: String = text.chars().take(*offset as usize).collect();
            text = keep + t;
        }
        if matches!(ev, ChatEvent::Done { .. } | ChatEvent::Error { .. }) {
            break;
        }
    }
    text
}

async fn pair_with(demo: &Arc<DemoNode>, core: &Core) {
    demo.auto_accept(core);
    core.pair_offer().await.unwrap();
    until(core, "check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    core.pair_confirm(true).unwrap();
    until(core, "linked", |s| matches!(s.pair, PairState::Linked { .. }) && s.online).await;
    until(core, "catalog", |s| !s.catalog.models.is_empty()).await;
}

async fn add_pool_with(demo: &Arc<DemoNode>, core: &Core) {
    // auto_accept returns once the core is already Linked, so a second pool has to
    // be accepted from the link code add_pool returns.
    let previous = core.active_pool().map(|p| p.id);
    let link = core.add_pool().await.unwrap();
    let accept = demo.accept(&link);
    let confirm = async {
        until(core, "new-pool check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
        core.pair_confirm(true).unwrap();
        until(core, "new pool linked", |s| {
            matches!(s.pair, PairState::Linked { .. }) && s.online && s.pool.as_ref().is_some_and(|p| Some(&p.id) != previous.as_ref())
        })
        .await;
    };
    let (accepted, ()) = tokio::join!(accept, confirm);
    accepted.unwrap();
    until(core, "new catalog", |s| !s.catalog.models.is_empty() && s.pool.as_ref().is_some_and(|p| Some(&p.id) != previous.as_ref())).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fresh_device_has_no_pools_until_it_links() {
    let tmp = tempfile::tempdir().unwrap();
    let core = Core::start(bragi_core::Config {
        dir: tmp.path().join("app"),
        label: "Bragi test".into(),
        transport: bragi_core::Transport::Memory,
        peers: Vec::new(),
    })
    .unwrap();
    assert!(core.pools().is_empty());
    assert_eq!(core.state().pair, PairState::Unpaired);
    assert!(core.state().pool.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn linking_creates_one_pool_and_a_legacy_profile_is_the_first_pool() {
    let tmp = tempfile::tempdir().unwrap();
    // A profile linked before pools existed: identity at the profile root.
    let demo = Arc::new(DemoNode::start(&tmp.path().join("demo"), Duration::from_millis(5)).await.unwrap());
    let core = Core::start(demo.app_config(&tmp.path().join("app"), "Bragi test")).unwrap();
    pair_with(&demo, &core).await;
    let pools = core.pools();
    assert_eq!(pools.len(), 1);
    assert!(pools[0].active && pools[0].linked);
    assert_eq!(pools[0].name, "Ymir pool");
    assert_eq!(core.state().pool.as_ref().map(|p| p.id.as_str()), Some(pools[0].id.as_str()));
    // Files stayed at the profile root.
    assert!(tmp.path().join("app/identity").is_dir());
    assert!(!tmp.path().join("app/pools").exists());

    // Re-open: the same pool is active, still at the root.
    drop(core);
    let again = Core::start(demo.app_config(&tmp.path().join("app"), "Bragi test")).unwrap();
    until(&again, "reopen linked", |s| matches!(s.pair, PairState::Linked { .. }) && s.online).await;
    let pools = again.pools();
    assert_eq!(pools.len(), 1);
    assert!(pools[0].active && pools[0].linked);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_device_can_add_another_pool_switch_and_keep_history_apart() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Arc::new(DemoNode::start(&tmp.path().join("demo-a"), Duration::from_millis(5)).await.unwrap());
    let b = Arc::new(DemoNode::start(&tmp.path().join("demo-b"), Duration::from_millis(5)).await.unwrap());
    let core = Core::start(a.app_config(&tmp.path().join("app"), "Bragi test")).unwrap();
    pair_with(&a, &core).await;

    let mut chat = core.send(None, "hello from a", Some(WARM_MODEL.into())).unwrap();
    let text = collect(&mut chat).await;
    assert_eq!(text, answer_for(WARM_MODEL, "hello from a"));
    let first_id = chat.conversation_id.clone();
    let first_pool = core.active_pool().unwrap();
    assert_eq!(core.conversations(0, 10).unwrap().len(), 1);

    add_pool_with(&b, &core).await;
    let pools = core.pools();
    assert_eq!(pools.len(), 2, "{pools:?}");
    let second = pools.iter().find(|p| p.active).unwrap();
    assert_ne!(second.id, first_pool.id);
    assert!(second.linked);
    // The new pool has its own empty history.
    assert!(core.conversations(0, 10).unwrap().is_empty());
    assert!(core.conversation(&first_id).is_err());

    let mut chat = core.send(None, "hello from b", Some(WARM_MODEL.into())).unwrap();
    let text = collect(&mut chat).await;
    assert_eq!(text, answer_for(WARM_MODEL, "hello from b"));
    let second_id = chat.conversation_id.clone();
    assert_eq!(core.conversations(0, 10).unwrap().len(), 1);

    core.switch_pool(&first_pool.id).await.unwrap();
    until(&core, "back on first", |s| {
        s.pool.as_ref().is_some_and(|p| p.id == first_pool.id) && matches!(s.pair, PairState::Linked { .. }) && s.online
    })
    .await;
    let page = core.conversations(0, 10).unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].id, first_id);
    assert!(core.conversation(&second_id).is_err());

    // The choice is remembered.
    drop(core);
    let again = Core::start(a.app_config(&tmp.path().join("app"), "Bragi test")).unwrap();
    until(&again, "reopen first", |s| s.pool.as_ref().is_some_and(|p| p.id == first_pool.id) && matches!(s.pair, PairState::Linked { .. })).await;
    assert_eq!(again.conversations(0, 10).unwrap()[0].id, first_id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rename_remove_and_cancel_a_new_pool() {
    let tmp = tempfile::tempdir().unwrap();
    let a = Arc::new(DemoNode::start(&tmp.path().join("demo-a"), Duration::from_millis(5)).await.unwrap());
    let b = Arc::new(DemoNode::start(&tmp.path().join("demo-b"), Duration::from_millis(5)).await.unwrap());
    let core = Core::start(a.app_config(&tmp.path().join("app"), "Bragi test")).unwrap();
    pair_with(&a, &core).await;
    let first = core.active_pool().unwrap();

    core.rename_pool(&first.id, "Home").unwrap();
    assert_eq!(core.active_pool().unwrap().name, "Home");
    assert_eq!(core.resolve_pool("home").unwrap(), first.id);
    assert_eq!(core.rename_pool("missing", "X").unwrap_err().kind, ErrorKind::NotFound);

    core.add_pool().await.unwrap();
    until(&core, "adding", |s| matches!(s.pair, PairState::Confirm { .. }) || matches!(s.pair, PairState::Offering { .. })).await;
    core.pair_cancel();
    until(&core, "still first", |s| matches!(s.pair, PairState::Linked { .. })).await;
    assert_eq!(core.pools().len(), 1, "a cancelled add does not leave a pool");
    assert_eq!(core.active_pool().unwrap().id, first.id);

    add_pool_with(&b, &core).await;
    let second = core.active_pool().unwrap();
    assert_ne!(second.id, first.id);
    core.remove_pool(&first.id).await.unwrap();
    let left = core.pools();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, second.id);
    assert!(left[0].active);

    core.remove_pool(&second.id).await.unwrap();
    assert!(core.pools().is_empty());
    until(&core, "unpaired after last pool", |s| matches!(s.pair, PairState::Unpaired) && !s.online).await;
}

#[test]
fn the_sidebar_choice_is_on_the_preference() {
    let tmp = tempfile::tempdir().unwrap();
    let core = Core::start(bragi_core::Config {
        dir: tmp.path().join("app"),
        label: "t".into(),
        transport: bragi_core::Transport::Memory,
        peers: Vec::new(),
    })
    .unwrap();
    assert!(!core.appearance().preference().sidebar_collapsed);
    core.appearance().set_sidebar_collapsed(true).unwrap();
    let pref: Preference = Core::start(bragi_core::Config {
        dir: tmp.path().join("app"),
        label: "t".into(),
        transport: bragi_core::Transport::Memory,
        peers: Vec::new(),
    })
    .unwrap()
    .appearance()
    .preference();
    assert!(pref.sidebar_collapsed);
}
