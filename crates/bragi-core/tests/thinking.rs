//! The live thinking view's data: reasoning from reasoning frames and from an inline `<think>`
//! block arrives apart from the answer, `Thought` says how long it took once the answer starts,
//! and the message keeps the reasoning. Demo node, memory transport, test identities.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bragi_core::demo::{DemoNode, THINK_INLINE_PROMPT, THINK_PROMPT, WARM_MODEL, answer_for, reasoning_for};
use bragi_core::{AppState, ChatEvent, Core, MessageState, PairState};

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

async fn paired(dir: &std::path::Path) -> (Arc<DemoNode>, Core) {
    let demo = Arc::new(DemoNode::start(&dir.join("demo"), Duration::from_millis(5)).await.unwrap());
    let core = Core::start(demo.app_config(&dir.join("app"), "Bragi test")).unwrap();
    demo.auto_accept(&core);
    core.pair_offer().await.unwrap();
    until(&core, "the code check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    core.pair_confirm(true).unwrap();
    until(&core, "linked", |s| matches!(s.pair, PairState::Linked { .. }) && s.online).await;
    until(&core, "the models", |s| s.catalog.models.iter().any(|m| m.id == WARM_MODEL)).await;
    (demo, core)
}

/// Plays one answer the way a shell does and checks the order: reasoning, `Thought`, answer.
async fn think(core: &Core, prompt: &str) {
    let mut h = core.send(None, prompt, Some(WARM_MODEL.into())).unwrap();
    let (mut reasoning, mut text, mut thought, mut order) = (String::new(), String::new(), None, Vec::new());
    while let Some(ev) = tokio::time::timeout(Duration::from_secs(20), h.events.recv()).await.expect("stream stalled") {
        match &ev {
            ChatEvent::Reasoning { offset, text: t } => {
                assert_eq!(*offset, reasoning.chars().count() as u64, "reasoning offsets continue");
                assert!(text.is_empty(), "reasoning after the answer started");
                reasoning.push_str(t);
                order.push('r');
            }
            ChatEvent::Thought { elapsed_ms } => {
                assert!(thought.is_none(), "one Thought");
                assert!(text.is_empty(), "Thought comes before the answer");
                thought = Some(*elapsed_ms);
                order.push('t');
            }
            ChatEvent::Delta { offset, text: t } => {
                assert_eq!(*offset, text.chars().count() as u64, "answer offsets count only the answer");
                text.push_str(t);
                order.push('d');
            }
            _ => {}
        }
        if matches!(ev, ChatEvent::Done { .. } | ChatEvent::Error { .. }) {
            assert!(matches!(ev, ChatEvent::Done { .. }), "{ev:?}");
            break;
        }
    }
    let want_reasoning = reasoning_for(prompt).unwrap();
    assert_eq!(reasoning.trim(), want_reasoning, "{prompt}");
    assert_eq!(text, answer_for(WARM_MODEL, prompt), "no tag or reasoning in the answer: {prompt}");
    assert!(thought.is_some(), "{prompt}: no Thought");
    order.dedup();
    assert_eq!(order, ['r', 't', 'd'], "{prompt}");

    // Kept with the message, on this device and in the store.
    let c = core.conversation(&h.conversation_id).unwrap();
    let m = c.messages.last().unwrap();
    assert_eq!(m.state, MessageState::Complete);
    assert_eq!(m.text, text);
    assert_eq!(m.reasoning.trim(), want_reasoning);
    assert_eq!(m.thinking_ms, thought);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reasoning_frames_and_inline_think_blocks() {
    let tmp = tempfile::tempdir().unwrap();
    let (_demo, core) = paired(tmp.path()).await;
    think(&core, &format!("{THINK_PROMPT} which node answers?")).await;
    think(&core, &format!("{THINK_INLINE_PROMPT} which node answers?")).await;

    // A model that doesn't reason: no reasoning, no Thought.
    let mut h = core.send(None, "Plain question", Some(WARM_MODEL.into())).unwrap();
    while let Some(ev) = tokio::time::timeout(Duration::from_secs(20), h.events.recv()).await.unwrap() {
        assert!(!matches!(ev, ChatEvent::Reasoning { .. } | ChatEvent::Thought { .. }), "{ev:?}");
        if matches!(ev, ChatEvent::Done { .. } | ChatEvent::Error { .. }) {
            break;
        }
    }
    let m = core.conversation(&h.conversation_id).unwrap().messages.pop().unwrap();
    assert!(m.reasoning.is_empty() && m.thinking_ms.is_none());
}
