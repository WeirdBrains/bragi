//! Sessions, endpoints and the Weirdnet fallback against a fake bridge on 127.0.0.1
//! (`bragi_core::demo_bridge`) and the in-process demo inference node: Weirdnet first, the bridge
//! when no node answers or one dies mid-answer, endpoint sessions that send only to their
//! endpoint, history per session, folders, and a Ymir bridge told apart from a generic API.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bragi_core::demo::{DemoNode, WARM_MODEL, answer_for};
use bragi_core::demo_bridge::FakeBridge;
use bragi_core::{
    AppState, ChatEvent, ChatHandle, Core, DEFAULT_SESSION, EndpointHealth, EndpointKind, EndpointState, ErrorKind, MessageState, PairState,
    RouteKind, Sessions, Target,
};

const TOKEN: &str = "fake-bridge-token-0123456789";

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

/// An app linked to one demo inference node.
async fn linked(dir: &std::path::Path) -> (Arc<DemoNode>, Core) {
    let demo = Arc::new(DemoNode::start(&dir.join("demo"), Duration::from_millis(5)).await.unwrap());
    let core = Core::start(demo.app_config(&dir.join("app"), "Bragi test")).unwrap();
    demo.auto_accept(&core);
    core.pair_offer().await.unwrap();
    until(&core, "the code check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    core.pair_confirm(true).unwrap();
    until(&core, "the node's catalog", |s| matches!(s.pair, PairState::Linked { .. }) && !s.catalog.nodes.is_empty()).await;
    (demo, core)
}

async fn bridge(ymir: bool) -> Arc<FakeBridge> {
    FakeBridge::start(ymir, TOKEN, &[WARM_MODEL, "bridge-only"], Duration::from_millis(3)).await.unwrap()
}

struct Answer {
    text: String,
    events: Vec<ChatEvent>,
}

impl Answer {
    fn via(&self) -> Option<(String, bool)> {
        self.events.iter().find_map(|e| if let ChatEvent::Via { endpoint, fallback, .. } = e { Some((endpoint.clone(), *fallback)) } else { None })
    }
    fn done(&self) -> bool {
        matches!(self.events.last(), Some(ChatEvent::Done { .. }))
    }
}

async fn collect(mut h: ChatHandle) -> Answer {
    let mut a = Answer { text: String::new(), events: Vec::new() };
    while let Some(ev) = tokio::time::timeout(Duration::from_secs(60), h.events.recv()).await.expect("stream stalled") {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn weirdnet_answers_first_and_the_bridge_only_when_no_node_does() {
    let tmp = tempfile::tempdir().unwrap();
    let (demo, core) = linked(tmp.path()).await;
    let b = bridge(true).await;
    let ep = core.add_endpoint("Home bridge", &b.url(), TOKEN).await.unwrap();
    assert_eq!(ep.kind, EndpointKind::YmirBridge);
    assert_eq!(ep.health, EndpointHealth::Ok);
    assert_eq!(ep.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), [WARM_MODEL, "bridge-only"], "chat models only, warm first");
    core.set_fallback(Some(&ep.id)).unwrap();
    let s = core.state();
    assert_eq!(s.route().kind, RouteKind::Weirdnet, "{:?}", s.route());
    assert!(s.route().detail.contains("Home bridge as fallback"), "{:?}", s.route());

    // Nodes up: the pool answers and the bridge is never asked.
    let prompt = "Where do answers come from?";
    let a = collect(core.send(None, prompt, Some(WARM_MODEL.into())).unwrap()).await;
    assert!(a.done(), "{:?}", a.events.last());
    assert_eq!(a.via(), None);
    assert_eq!(a.text, answer_for(WARM_MODEL, prompt));
    assert_eq!(b.chats(), 0, "Weirdnet first");

    // The node goes away: the same session's next message goes to the bridge.
    demo.stop();
    let prompt2 = "And now?";
    let a = collect(core.send(None, prompt2, Some(WARM_MODEL.into())).unwrap()).await;
    assert!(a.done(), "{:?}", a.events);
    assert_eq!(a.via(), Some(("Home bridge".to_string(), true)));
    assert_eq!(a.text, answer_for(WARM_MODEL, prompt2));
    assert_eq!(b.chats(), 1);
    let conv = core.conversations(0, 10).unwrap().into_iter().find(|c| c.title == prompt2).unwrap();
    let last = core.conversation(&conv.id).unwrap().messages.pop().unwrap();
    assert_eq!((last.state, last.via.as_deref(), last.model.as_deref()), (MessageState::Complete, Some("Home bridge"), Some(WARM_MODEL)));
    let sent = b.last_request().unwrap();
    assert_eq!(sent["messages"].as_array().unwrap().last().unwrap()["content"], prompt2);
    assert!(sent.get("continue_final_message").is_none(), "a fresh answer is not a continuation");

    // Once the catalog has dropped the node, the shells say where messages go.
    let s = until(&core, "the node to drop out", |s| s.catalog.nodes.is_empty()).await;
    assert_eq!(s.route().kind, RouteKind::Fallback);
    assert_eq!(s.route().endpoint.as_deref(), Some("Home bridge"));
    assert_eq!(s.default_model().as_deref(), Some(WARM_MODEL), "the bridge's models stand in for the pool's");
    // The token is nowhere in what the shells see.
    assert!(!serde_json::to_string(&s).unwrap().contains(TOKEN));
    assert!(!format!("{s:?}").contains(TOKEN));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_answer_cut_off_on_weirdnet_continues_on_the_bridge() {
    let tmp = tempfile::tempdir().unwrap();
    let (demo, core) = linked(tmp.path()).await;
    let b = bridge(true).await;
    let ep = core.add_endpoint("Home bridge", &b.url(), TOKEN).await.unwrap();
    core.set_fallback(Some(&ep.id)).unwrap();

    let prompt = "Tell me a long story about a node that went away";
    demo.faults.die_next_answer_after(5);
    let mut h = core.send(None, prompt, Some(WARM_MODEL.into())).unwrap();
    // Once the node started, it refuses everything: no Weirdnet node can finish the answer.
    let mut a = Answer { text: String::new(), events: Vec::new() };
    let first = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let ev = h.events.recv().await.unwrap();
            let started = matches!(ev, ChatEvent::Started { .. });
            if let ChatEvent::Delta { offset, text } = &ev {
                let keep: String = a.text.chars().take(*offset as usize).collect();
                a.text = keep + text;
            }
            a.events.push(ev);
            if started {
                return;
            }
        }
    })
    .await;
    assert!(first.is_ok(), "the node started");
    demo.faults.refuse_next(1000);
    let rest = collect(h).await;
    for ev in rest.events {
        if let ChatEvent::Delta { offset, text } = &ev {
            let keep: String = a.text.chars().take(*offset as usize).collect();
            assert_eq!(keep.chars().count() as u64, *offset, "offsets carry on across the switch");
            a.text = keep + text;
        }
        a.events.push(ev);
    }
    assert!(a.done(), "{:?}", a.events);
    assert_eq!(a.via(), Some(("Home bridge".to_string(), true)));
    let via_at = a.events.iter().position(|e| matches!(e, ChatEvent::Via { .. })).unwrap();
    assert!(a.events[..via_at].iter().any(|e| matches!(e, ChatEvent::Delta { .. })), "Weirdnet had shown text first");
    assert_eq!(a.text, answer_for(WARM_MODEL, prompt), "the bridge carried on from where the node stopped");
    let sent = b.last_request().unwrap();
    assert_eq!(sent["continue_final_message"], true);
    let partial = sent["messages"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(partial["role"], "assistant");
    assert!(!partial["content"].as_str().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_a_ymir_bridge_can_be_the_fallback_and_a_wrong_token_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let (_demo, core) = linked(tmp.path()).await;
    let generic = bridge(false).await;
    let e = core.add_endpoint("Some API", &generic.url(), TOKEN).await.unwrap();
    assert_eq!(e.kind, EndpointKind::OpenAi);
    let err = core.set_fallback(Some(&e.id)).unwrap_err();
    assert!(err.message.contains("not a Ymir bridge"), "{err}");
    let ymir = bridge(true).await;
    let err = core.add_endpoint("Bad token", &ymir.url(), "wrong-token-0000000000").await.unwrap_err();
    assert_eq!(err.kind, ErrorKind::Refused, "{err}");
    assert!(!err.message.contains("wrong-token"));
    assert!(core.add_endpoint("Remote http", "http://ymir.example.ts.net:8484/v1", TOKEN).await.is_err(), "https for remote endpoints");
    assert_eq!(core.state().endpoints.len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_endpoint_session_sends_only_there_and_keeps_its_own_history() {
    let tmp = tempfile::tempdir().unwrap();
    let (_demo, core) = linked(tmp.path()).await;
    // History from before sessions belongs to the Weirdnet session.
    let old = collect(core.send(None, "From before sessions", Some(WARM_MODEL.into())).unwrap()).await;
    assert!(old.done());
    let b = bridge(false).await;
    let ep = core.add_endpoint("Some API", &b.url(), TOKEN).await.unwrap();
    let s = core.create_session("API prompts", Target::Endpoint { id: ep.id.clone() }, Some("bridge-only".into()), None).unwrap();
    core.switch_session(&s.id).unwrap();
    let st = core.state();
    assert_eq!(st.session().id, s.id);
    assert_eq!(st.route().kind, RouteKind::Endpoint);
    assert_eq!(st.default_model().as_deref(), Some("bridge-only"));
    assert!(core.conversations(0, 10).unwrap().is_empty(), "a new session starts empty");

    let prompt = "Only to the API, please";
    let a = collect(core.send(None, prompt, None).unwrap()).await;
    assert!(a.done(), "{:?}", a.events);
    assert_eq!(a.via(), Some(("Some API".to_string(), false)));
    assert_eq!(a.text, answer_for("bridge-only", prompt));
    assert_eq!(b.chats(), 1);
    let list = core.conversations(0, 10).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(core.conversation(&list[0].id).unwrap().session.as_deref(), Some(s.id.as_str()));
    assert!(core.send_agent(None, "agent?", None, None, Vec::new()).is_err(), "agent tasks are Weirdnet-only");

    // The endpoint goes away: the session fails rather than using Weirdnet.
    drop(b);
    let a = collect(core.send(Some(&list[0].id), "Still there?", None).unwrap()).await;
    assert!(matches!(a.events.last(), Some(ChatEvent::Error { kind: ErrorKind::Unavailable, .. })), "{:?}", a.events.last());
    assert!(!a.events.iter().any(|e| matches!(e, ChatEvent::Started { node, .. } if !node.contains("Some API"))));

    // The Weirdnet session still has its own history.
    core.switch_session(DEFAULT_SESSION).unwrap();
    let list = core.conversations(0, 10).unwrap();
    assert_eq!(list.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["From before sessions"]);
    assert_eq!(core.all_conversations(0, 10).unwrap().len(), 2);

    // The endpoint can't go while the session uses it; deleting the session keeps its history.
    assert!(core.remove_endpoint(&ep.id).is_err());
    core.delete_session(&s.id).unwrap();
    assert_eq!(core.conversations(0, 10).unwrap().len(), 2, "its conversations moved to the Weirdnet session");
    core.remove_endpoint(&ep.id).unwrap();
    assert!(core.state().endpoints.is_empty());
    assert!(!tmp.path().join("app/secrets").join(format!("{}.token", ep.id)).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn folders_and_sessions_persist_across_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let (demo, core) = linked(tmp.path()).await;
    let top = core.create_folder("Projects", None).unwrap();
    let sub = core.create_folder("Bragi", Some(top.id.clone())).unwrap();
    let s = core.create_session("Release notes", Target::Weirdnet, None, Some(sub.id.clone())).unwrap();
    core.set_folder_collapsed(&top.id, true).unwrap();
    assert!(core.move_folder(&top.id, Some(sub.id.clone())).is_err(), "not into its own subfolder");
    core.switch_session(&s.id).unwrap();
    core.set_session_model(&s.id, Some(WARM_MODEL.into())).unwrap();
    let st = until(&core, "the tree", |st| st.sessions.folders.len() == 2).await;
    assert_eq!(st.sessions.rows().len(), 2, "Projects collapsed: itself and the Weirdnet session");
    core.shutdown();
    drop(core);

    let core = Core::start(demo.app_config(&tmp.path().join("app"), "Bragi test")).unwrap();
    let st: Sessions = core.sessions();
    assert_eq!(st.active, s.id);
    assert_eq!(st.get(&s.id).unwrap().model.as_deref(), Some(WARM_MODEL));
    assert!(st.folder(&top.id).unwrap().collapsed);
    core.delete_folder(&top.id).unwrap();
    let st = core.sessions();
    assert_eq!(st.folder(&sub.id).unwrap().parent, None, "a deleted folder's contents move up");
    assert_eq!(st.get(&s.id).unwrap().folder.as_deref(), Some(sub.id.as_str()));
    core.rename_folder(&sub.id, "Bragi app").unwrap();
    core.move_session(&s.id, None).unwrap();
    core.rename_session(&s.id, "Notes").unwrap();
    let st = core.state().sessions;
    assert_eq!(st.get(&s.id).unwrap().name, "Notes");
    assert_eq!(st.get(&s.id).unwrap().folder, None);
}

#[test]
fn the_route_says_where_the_next_message_goes() {
    let mut s = AppState::default();
    assert_eq!(s.route().kind, RouteKind::None);
    s.endpoints.push(EndpointState {
        id: "e".into(),
        name: "Home".into(),
        url: "https://ymir.example.ts.net:9443/v1".into(),
        kind: EndpointKind::YmirBridge,
        health: EndpointHealth::Ok,
        message: None,
        models: Vec::new(),
        fallback: true,
    });
    assert_eq!(s.route().kind, RouteKind::Fallback);
    s.endpoints[0].health = EndpointHealth::Failed;
    assert_eq!(s.route().kind, RouteKind::None, "a failing bridge is not a route");
    s.sessions.sessions.push(bragi_core::Session {
        id: "s".into(),
        name: "API".into(),
        target: Target::Endpoint { id: "gone".into() },
        model: None,
        folder: None,
        created_ms: 0,
    });
    s.sessions.active = "s".into();
    assert_eq!(s.route().kind, RouteKind::None);
    assert!(s.route().detail.contains("removed"));
}
