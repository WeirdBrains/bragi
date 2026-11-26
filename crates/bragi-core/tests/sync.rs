//! B4: a conversation follows the user from one linked device to another. Two Bragi cores are
//! linked to the same demo node (the user's other device), on the memory transport with test
//! identities; one chats, the other reads it after a sync pass.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bragi_core::demo::DemoNode;
use bragi_core::{AppState, ChatEvent, Core, PairState};

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

async fn link(demo: &Arc<DemoNode>, dir: &std::path::Path, label: &str) -> Core {
    let core = Core::start(demo.app_config(dir, label)).unwrap();
    demo.auto_accept(&core);
    core.pair_offer().await.unwrap();
    until(&core, "the code check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    core.pair_confirm(true).unwrap();
    until(&core, "linked", |s| matches!(s.pair, PairState::Linked { .. }) && s.online && s.catalog.models.len() == 2).await;
    core
}

async fn chat(core: &Core, conversation: Option<&str>, prompt: &str) -> String {
    let mut h = core.send(conversation, prompt, None).unwrap();
    while let Some(ev) = tokio::time::timeout(Duration::from_secs(20), h.events.recv()).await.expect("stream stalled") {
        if matches!(ev, ChatEvent::Done { .. } | ChatEvent::Error { .. }) {
            break;
        }
    }
    h.conversation_id
}

/// Passes until `f` holds (the first ones bring the key chain, the later ones the records).
async fn sync_until(demo: &DemoNode, cores: &[&Core], what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !f() {
        assert!(Instant::now() < deadline, "waiting for {what}");
        for c in cores {
            c.sync_now().await.unwrap();
        }
        let addrs: Vec<String> = cores.iter().filter_map(|c| c.address()).collect();
        demo.sync_with(&addrs).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_conversation_follows_the_user_to_another_device() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = Arc::new(DemoNode::start(&tmp.path().join("demo"), Duration::from_millis(5)).await.unwrap());
    let a = link(&demo, &tmp.path().join("a"), "Bragi A").await;
    let b = link(&demo, &tmp.path().join("b"), "Bragi B").await;
    // On the memory transport devices only find each other by address.
    a.add_peer(&b.address().unwrap()).unwrap();
    b.add_peer(&a.address().unwrap()).unwrap();

    let conv = chat(&a, None, "hello from the first device").await;
    assert_eq!(a.conversation(&conv).unwrap().messages.len(), 2);
    // b may already have it: the devices sync in the background as well (seen on Windows CI),
    // so only the end state is checked. A background pass can also bring the reply while it is
    // still streaming (empty, seen on macOS CI), so wait for the whole reply text, not a count.
    let reply = a.conversation(&conv).unwrap().messages[1].text.clone();
    sync_until(&demo, &[&a, &b], "the whole conversation on the second device", || {
        b.conversation(&conv).map(|c| c.messages.len() == 2 && c.messages[1].text == reply).unwrap_or(false)
    })
    .await;
    let seen = b.conversation(&conv).unwrap();
    assert_eq!(seen.messages[0].text, "hello from the first device");
    assert_eq!(seen.messages[1].text, a.conversation(&conv).unwrap().messages[1].text);
    assert_eq!(b.conversations(0, 10).unwrap()[0].id, conv);

    // Carry on from the second device; the first sees it.
    chat(&b, Some(&conv), "and a follow-up from the second").await;
    let follow_up = b.conversation(&conv).unwrap().messages[3].text.clone();
    sync_until(&demo, &[&a, &b], "the whole follow-up on the first device", || {
        a.conversation(&conv).map(|c| c.messages.len() == 4 && c.messages[3].text == follow_up).unwrap_or(false)
    })
    .await;

    // A delete travels too.
    b.delete_conversation(&conv).unwrap();
    sync_until(&demo, &[&a, &b], "the delete on the first device", || a.conversations(0, 10).map(|l| l.is_empty()).unwrap_or(false)).await;
}
