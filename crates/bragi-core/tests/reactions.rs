//! Reactions through `Core`: toggling, persistence across restarts, the event hook, and a record
//! from before reactions existing.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bragi_core::demo::DemoNode;
use bragi_core::{AppState, ChatEvent, Core, ErrorKind, Feedback, PairState};

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
async fn reactions_toggle_persist_and_emit_events() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = Arc::new(DemoNode::start(&tmp.path().join("demo"), Duration::from_millis(5)).await.unwrap());
    let cfg = demo.app_config(&tmp.path().join("app"), "Bragi test");
    let core = Core::start(cfg.clone()).unwrap();
    demo.auto_accept(&core);
    core.pair_offer().await.unwrap();
    until(&core, "check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    core.pair_confirm(true).unwrap();
    until(&core, "linked", |s| matches!(s.pair, PairState::Linked { .. }) && s.online).await;
    until(&core, "catalog", |s| !s.catalog.models.is_empty()).await;

    let mut h = core.send(None, "hello", None).unwrap();
    let conv = h.conversation_id.clone();
    while let Some(ev) = h.events.recv().await {
        if matches!(ev, ChatEvent::Done { .. } | ChatEvent::Error { .. }) {
            break;
        }
    }
    let c = core.conversation(&conv).unwrap();
    let answer = c.messages[1].id.clone();
    let mine = c.messages[0].id.clone();
    assert!(c.messages[1].reactions.is_empty());

    let mut events = core.reaction_events();
    assert_eq!(core.react(&conv, &answer, "👍").unwrap(), ["👍"]);
    assert_eq!(core.react(&conv, &answer, "🎉").unwrap(), ["👍", "🎉"]);
    assert_eq!(core.react(&conv, &mine, "🤔").unwrap(), ["🤔"]);
    // The same reaction again turns it off.
    assert_eq!(core.react(&conv, &answer, "👍").unwrap(), ["🎉"]);
    assert_eq!(core.conversation(&conv).unwrap().messages[1].reactions, ["🎉"]);

    let e = events.recv().await.unwrap();
    assert!(e.on && e.emoji == "👍" && e.feedback == Some(Feedback::Good) && e.message == answer && e.model.is_some());
    let _ = events.recv().await.unwrap();
    let _ = events.recv().await.unwrap();
    let off = events.recv().await.unwrap();
    assert!(!off.on && off.emoji == "👍");

    assert_eq!(core.react(&conv, "nope", "👍").unwrap_err().kind, ErrorKind::NotFound);
    assert_eq!(core.react(&conv, &answer, "hi").unwrap_err().kind, ErrorKind::Refused);

    // They survive a restart.
    core.shutdown();
    drop(core);
    let again = Core::start(cfg).unwrap();
    until(&again, "online", |s| s.online).await;
    let c = again.conversation(&conv).unwrap();
    assert_eq!(c.messages[1].reactions, ["🎉"]);
    assert_eq!(c.messages[0].reactions, ["🤔"]);

    // A reaction goes with its message when that is deleted, and comes back with an undo.
    let d = again.delete_message(&conv, &answer, false).unwrap();
    assert!(again.conversation(&conv).unwrap().messages.iter().all(|m| m.id != answer));
    again.undo_delete(&d.undo).unwrap();
    let back = again.conversation(&conv).unwrap();
    assert_eq!(back.messages.iter().find(|m| m.id == answer).unwrap().reactions, ["🎉"]);
}
