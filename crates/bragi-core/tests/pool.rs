//! B2: the pool client against two inference nodes of one user (memory transport, test
//! identities, scripted faults): the merged catalog, routing, retries, failover before and in
//! the middle of an answer, and the stand-in and cache flags.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bragi_core::demo::{BIG_MODEL, COLD_MODEL, DemoNode, WARM_MODEL, answer_for};
use bragi_core::{AppState, ChatEvent, ChatHandle, Core, MessageState, ModelStatus, PairState};

async fn until(core: &Core, what: &str, f: impl Fn(&AppState) -> bool) -> AppState {
    let deadline = Instant::now() + Duration::from_secs(30);
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

/// An app linked to the first of two inference nodes, both in its catalog.
async fn cloud(dir: &std::path::Path) -> (Arc<DemoNode>, Core) {
    let demo = DemoNode::start_cloud(&dir.join("demo"), Duration::from_millis(8)).await.unwrap();
    let core = Core::start(demo.app_config(&dir.join("app"), "Bragi test")).unwrap();
    demo.auto_accept(&core);
    core.pair_offer().await.unwrap();
    until(&core, "the code check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    core.pair_confirm(true).unwrap();
    until(&core, "both nodes' catalogs", |s| matches!(s.pair, PairState::Linked { .. }) && s.catalog.nodes.len() == 2).await;
    (demo, core)
}

struct Answer {
    text: String,
    events: Vec<ChatEvent>,
}

impl Answer {
    fn node(&self) -> String {
        self.events.iter().rev().find_map(|e| if let ChatEvent::Started { node, .. } = e { Some(node.clone()) } else { None }).expect("started")
    }
    fn meta(&self) -> Option<(bool, Option<String>)> {
        self.events.iter().find_map(|e| if let ChatEvent::Meta { cache_hit, standin, .. } = e { Some((*cache_hit, standin.clone())) } else { None })
    }
    fn count(&self, f: impl Fn(&ChatEvent) -> bool) -> usize {
        self.events.iter().filter(|e| f(e)).count()
    }
}

async fn collect(mut h: ChatHandle) -> Answer {
    let mut a = Answer { text: String::new(), events: Vec::new() };
    while let Some(ev) = tokio::time::timeout(Duration::from_secs(30), h.events.recv()).await.expect("stream stalled") {
        if let ChatEvent::Delta { offset, text } = &ev {
            let keep: String = a.text.chars().take(*offset as usize).collect();
            assert_eq!(keep.chars().count() as u64, *offset, "a delta past the end of the text");
            a.text = keep + text;
        }
        let end = matches!(ev, ChatEvent::Done { .. } | ChatEvent::Error { .. });
        a.events.push(ev);
        if end {
            break;
        }
    }
    a
}

async fn ask(core: &Core, model: &str, prompt: &str) -> Answer {
    collect(core.send(None, prompt, Some(model.to_string())).unwrap()).await
}

fn done(a: &Answer) {
    assert!(matches!(a.events.last(), Some(ChatEvent::Done { .. })), "{:?}", a.events.last());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_catalog_merges_both_nodes() {
    let tmp = tempfile::tempdir().unwrap();
    let (demo, core) = cloud(tmp.path()).await;
    let second = demo.siblings().remove(0);
    let (p1, p2) = (demo.node.peer_id().to_string(), second.node.peer_id().to_string());

    let c = core.state().catalog;
    let ids: Vec<&str> = c.models.iter().map(|m| m.id.as_str()).collect();
    // Warm models first; the embedding model is not a chat model.
    assert_eq!(ids.len(), 3, "{ids:?}");
    assert!(!ids.contains(&"nomic-embed"));
    let gemma = c.models.iter().find(|m| m.id == WARM_MODEL).unwrap();
    assert_eq!((gemma.status, gemma.replicas, gemma.nodes.len()), (ModelStatus::Warm, 2, 2), "one row for a model on two nodes");
    let big = c.models.iter().find(|m| m.id == BIG_MODEL).unwrap();
    assert_eq!((big.status, big.nodes.clone()), (ModelStatus::Warm, vec![p2.clone()]));
    let qwen = c.models.iter().find(|m| m.id == COLD_MODEL).unwrap();
    assert_eq!((qwen.status, qwen.nodes.clone()), (ModelStatus::Loadable, vec![p1.clone()]));
    assert_eq!(c.nodes.len(), 2);
    assert!(c.models.iter().take(2).all(|m| m.status == ModelStatus::Warm), "warm first");

    // Each model is asked of a node that serves it.
    let a = ask(&core, BIG_MODEL, "which node?").await;
    done(&a);
    assert_eq!(a.node(), p2);
    assert_eq!(a.text, answer_for(BIG_MODEL, "which node?"));
    let a = ask(&core, COLD_MODEL, "and this one?").await;
    done(&a);
    assert_eq!(a.node(), p1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_node_refusing_fails_over_before_the_first_token() {
    let tmp = tempfile::tempdir().unwrap();
    let (demo, core) = cloud(tmp.path()).await;
    // Whichever node the pool tries first refuses; the other answers, with no retry needed.
    demo.faults.refuse_next(1);
    let a = ask(&core, WARM_MODEL, "hello pool").await;
    done(&a);
    assert_eq!(a.text, answer_for(WARM_MODEL, "hello pool"));
    assert_eq!(a.count(|e| matches!(e, ChatEvent::Retry { .. })), 0);
    assert_eq!(a.count(|e| matches!(e, ChatEvent::Resumed { .. })), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn when_every_node_fails_the_core_retries() {
    let tmp = tempfile::tempdir().unwrap();
    let (demo, core) = cloud(tmp.path()).await;
    // Both nodes refuse the first round; the second round works.
    demo.faults.refuse_next(2);
    let a = ask(&core, WARM_MODEL, "try again").await;
    done(&a);
    assert_eq!(a.text, answer_for(WARM_MODEL, "try again"));
    assert_eq!(a.count(|e| matches!(e, ChatEvent::Retry { .. })), 1, "{:?}", a.events);

    // Nothing works for long enough: the error comes back after the retries, and the history says failed.
    demo.faults.refuse_next(100);
    let h = core.send(None, "still down", Some(WARM_MODEL.into())).unwrap();
    let id = h.conversation_id.clone();
    let a = collect(h).await;
    assert!(matches!(a.events.last(), Some(ChatEvent::Error { .. })), "{:?}", a.events.last());
    assert_eq!(a.count(|e| matches!(e, ChatEvent::Retry { .. })), 2);
    assert_eq!(core.conversation(&id).unwrap().messages.last().unwrap().state, MessageState::Failed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_answer_moves_to_the_other_node_when_its_node_dies() {
    let tmp = tempfile::tempdir().unwrap();
    let (demo, core) = cloud(tmp.path()).await;
    demo.faults.die_next_answer_after(6);
    let h = core.send(None, "survive a crash", Some(WARM_MODEL.into())).unwrap();
    let id = h.conversation_id.clone();
    let a = collect(h).await;
    done(&a);
    // The text is complete and exact: nothing repeated, nothing lost at the seam.
    assert_eq!(a.text, answer_for(WARM_MODEL, "survive a crash"));
    assert_eq!(a.count(|e| matches!(e, ChatEvent::Resumed { .. })), 1, "{:?}", a.events);
    let m = core.conversation(&id).unwrap().messages.pop().unwrap();
    assert_eq!((m.state, m.resumes), (MessageState::Complete, 1));
    assert_eq!(m.text, a.text);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_cache_and_stand_in_flags_reach_the_shells_and_history() {
    let tmp = tempfile::tempdir().unwrap();
    let (_demo, core) = cloud(tmp.path()).await;

    // A model that isn't warm anywhere is answered by a stand-in, and says so.
    let h = core.send(None, "load slowly", Some(COLD_MODEL.into())).unwrap();
    let id = h.conversation_id.clone();
    let a = collect(h).await;
    done(&a);
    assert_eq!(a.meta(), Some((false, Some(WARM_MODEL.to_string()))), "{:?}", a.events);
    let m = core.conversation(&id).unwrap().messages.pop().unwrap();
    assert_eq!((m.standin.as_deref(), m.model.as_deref()), (Some(WARM_MODEL), Some(COLD_MODEL)), "answered by the stand-in, kept as the requested model's");
    assert_eq!(a.text, answer_for(WARM_MODEL, "load slowly"));

    // The same question again is a cache hit.
    let first = ask(&core, WARM_MODEL, "same question").await;
    assert_eq!(first.meta(), None);
    let h = core.send(None, "same question", Some(WARM_MODEL.into())).unwrap();
    let id = h.conversation_id.clone();
    let again = collect(h).await;
    done(&again);
    assert_eq!(again.text, first.text);
    assert_eq!(again.meta().map(|m| m.0), Some(true));
    let m = core.conversation(&id).unwrap().messages.pop().unwrap();
    assert!(m.cache_hit && m.standin.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_node_that_goes_away_leaves_the_catalog() {
    let tmp = tempfile::tempdir().unwrap();
    let (demo, core) = cloud(tmp.path()).await;
    let second = demo.siblings().remove(0);
    second.stop();
    let s = until(&core, "the second node to leave", |s| s.catalog.nodes.len() == 1).await;
    assert!(s.catalog.models.iter().all(|m| m.id != BIG_MODEL), "{:?}", s.catalog);
    let gemma = s.catalog.models.iter().find(|m| m.id == WARM_MODEL).unwrap();
    assert_eq!(gemma.nodes.len(), 1);
    // The rest still answers.
    let a = ask(&core, WARM_MODEL, "alone now").await;
    done(&a);
    assert_eq!(a.node(), demo.node.peer_id().to_string());
}
