//! Deleting history across linked devices: a message deleted on one device is gone on the other
//! after a sync pass, from what it shows, what it sends to a model, and its log on disk. Two Bragi
//! cores linked to the same demo node, on the memory transport with test identities.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bragi_core::demo::DemoNode;
use bragi_core::identity::{CONTEXT_TYPE, Identity};
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

#[allow(dead_code)]
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

/// Every context value the device's log on disk can still open.
fn on_disk(dir: &std::path::Path) -> Vec<String> {
    let id = Identity::open(&dir.join("identity")).unwrap().unwrap();
    let st = id.state();
    let st = st.lock().unwrap();
    st.log
        .iter()
        .filter(|e| e.typ == CONTEXT_TYPE)
        .filter_map(|e| match st.read(e) {
            Ok(weirdnet_sync::RecordValue::Ready(v)) => Some(String::from_utf8_lossy(&v).into_owned()),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_deleted_message_is_gone_on_the_other_device_too() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = Arc::new(DemoNode::start(&tmp.path().join("demo"), Duration::from_millis(5)).await.unwrap());
    let (da, db) = (tmp.path().join("a"), tmp.path().join("b"));
    let a = link(&demo, &da, "Bragi A").await;
    let b = link(&demo, &db, "Bragi B").await;
    a.add_peer(&b.address().unwrap()).unwrap();
    b.add_peer(&a.address().unwrap()).unwrap();

    let conv = chat(&a, None, "keep this one").await;
    chat(&a, Some(&conv), "forget the zebra password").await;
    chat(&a, Some(&conv), "and keep this too").await;
    // B's log reaches its disk when its pass saves, which can be just after the history shows it.
    sync_until(&demo, &[&a, &b], "the conversation on B and its disk", || {
        b.conversation(&conv).map(|c| c.messages.len() == 6).unwrap_or(false) && on_disk(&db).iter().any(|v| v.contains("zebra"))
    })
    .await;

    // A deletes the question and its answer; B follows, and its own log drops them.
    let c = a.conversation(&conv).unwrap();
    let q = c.messages.iter().find(|m| m.text.contains("zebra")).unwrap().id.clone();
    let d = a.delete_message(&conv, &q, true).unwrap();
    assert_eq!(d.messages, 2);
    assert_eq!(a.conversation(&conv).unwrap().messages.len(), 4);
    sync_until(&demo, &[&a, &b], "the delete on B and both disks", || {
        b.conversation(&conv).map(|c| c.messages.len() == 4).unwrap_or(false)
            && [&da, &db].iter().all(|d| on_disk(d).iter().all(|v| !v.contains("zebra")))
    })
    .await;
    for (who, dir) in [("A", &da), ("B", &db)] {
        let left = on_disk(dir);
        assert!(left.iter().all(|v| !v.contains("zebra")), "{who} still holds it: {left:?}");
    }
    // Undo on A (within the undo window, so before anything slow) brings it back on B as well.
    a.undo_delete(&d.undo).unwrap();
    sync_until(&demo, &[&a, &b], "the undo on B", || {
        b.conversation(&conv).map(|c| c.messages.iter().any(|m| m.text.contains("zebra"))).unwrap_or(false)
    })
    .await;

    // Deleted again, the next turn on B carries on without it.
    let q = a.conversation(&conv).unwrap().messages.iter().find(|m| m.text.contains("zebra")).unwrap().id.clone();
    assert_eq!(a.delete_message(&conv, &q, true).unwrap().messages, 2);
    sync_until(&demo, &[&a, &b], "the second delete on B", || b.conversation(&conv).map(|c| c.messages.len() == 4).unwrap_or(false)).await;
    chat(&b, Some(&conv), "one more").await;
    assert!(b.conversation(&conv).unwrap().messages.iter().all(|m| !m.text.contains("zebra")));

    // Clearing everything on B clears A.
    assert_eq!(b.clear_history().unwrap(), 1);
    sync_until(&demo, &[&a, &b], "the clear on A and both disks", || {
        a.conversations(0, 10).map(|l| l.is_empty()).unwrap_or(false)
            && [&da, &db].iter().all(|d| on_disk(d).iter().all(|v| !v.contains("zebra") && !v.contains("keep this")))
    })
    .await;
}

/// A stored clip's written copy goes when its message does, and the copies of a deleted
/// conversation go with it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_deleted_message_takes_its_clip_copy_with_it() {
    use bragi_core::video::{Sampled, StoredClip};
    let tmp = tempfile::tempdir().unwrap();
    let demo = Arc::new(DemoNode::start(&tmp.path().join("demo"), Duration::from_millis(5)).await.unwrap());
    let a = link(&demo, &tmp.path().join("a"), "Bragi A").await;
    let clip = |name: &str, bytes: &[u8]| {
        let p = tmp.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        let s = Sampled {
            summary: "Video".into(),
            duration_ms: 1_000,
            frames: Vec::new(),
            audio: None,
            clip: None,
            stored: StoredClip::read(&p).unwrap(),
        };
        bragi_core::Attachment::from_video(name, s)
    };
    let send = |conversation: Option<String>, a2: bragi_core::Attachment| {
        let mut h = a.send_with(conversation.as_deref(), "what happens?", None, vec![a2]).unwrap();
        async move {
            while let Some(ev) = tokio::time::timeout(Duration::from_secs(20), h.events.recv()).await.expect("stream stalled") {
                if matches!(ev, ChatEvent::Done { .. } | ChatEvent::Error { .. }) {
                    break;
                }
            }
            h.conversation_id
        }
    };
    let conv = send(None, clip("one.mp4", b"first clip")).await;
    send(Some(conv.clone()), clip("two.webm", b"second clip")).await;
    let other = send(None, clip("three.mkv", b"third clip")).await;
    let played = |conv: &str, n: usize| {
        let c = a.conversation(conv).unwrap();
        let m = c.messages.iter().filter(|m| m.role == bragi_core::Role::User).nth(n).unwrap().clone();
        let Some(bragi_core::Media::Video(v)) = &m.attachments[0].media else { panic!("no clip") };
        (m.id.clone(), a.playable_clip(v).unwrap())
    };
    let (first, p1) = played(&conv, 0);
    let (_, p2) = played(&conv, 1);
    let (_, p3) = played(&other, 0);
    assert!(p1.starts_with(a.clip_cache()) && p1.is_file() && p2.is_file() && p3.is_file());

    a.delete_message(&conv, &first, true).unwrap();
    assert!(!p1.exists(), "the deleted message's copy went");
    assert!(p2.is_file() && p3.is_file(), "the others stay");
    a.delete_conversation(&other).unwrap();
    assert!(!p3.exists(), "a deleted conversation's copies go");
    assert!(p2.is_file());
    a.clear_history().unwrap();
    assert!(!p2.exists(), "clearing history empties the cache");
}
