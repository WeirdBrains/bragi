//! Editing and deleting what the settings screens list: endpoints (name, URL, a replaced token,
//! the default model, the fallback; deleting one rebinds its sessions or leaves them unbound) and
//! Weirdnet links (rename, a new address after a test connection, delete). Against fake bridges
//! on 127.0.0.1 and demo nodes on the memory transport; no real network, no real identity.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bragi_core::demo::{DemoNode, WARM_MODEL};
use bragi_core::demo_bridge::FakeBridge;
use bragi_core::{AppState, ChatEvent, ChatHandle, Core, DEFAULT_SESSION, EndpointKind, PairState, Rebind, RouteKind, Target};

const TOKEN: &str = "fake-bridge-token-0123456789";
const OTHER_TOKEN: &str = "another-bridge-token-9876543210";

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

async fn finish(mut h: ChatHandle) -> bool {
    while let Some(ev) = tokio::time::timeout(Duration::from_secs(60), h.events.recv()).await.expect("stream stalled") {
        match ev {
            ChatEvent::Done { .. } => return true,
            ChatEvent::Error { .. } => return false,
            _ => {}
        }
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_endpoint_is_edited_after_a_test_and_its_token_is_never_shown() {
    let tmp = tempfile::tempdir().unwrap();
    let core = Core::start(bragi_core::Config::new(tmp.path(), "Bragi test")).unwrap();
    let a = FakeBridge::start(true, TOKEN, &[WARM_MODEL, "bridge-big"], Duration::from_millis(2)).await.unwrap();
    let b = FakeBridge::start(true, OTHER_TOKEN, &[WARM_MODEL, "bridge-big"], Duration::from_millis(2)).await.unwrap();
    let generic = FakeBridge::start(false, TOKEN, &["gpt-x"], Duration::from_millis(2)).await.unwrap();

    let ep = core.add_endpoint("Home", &a.url(), TOKEN).await.unwrap();
    for n in ["One", "Two"] {
        core.create_session(n, Target::Endpoint { id: ep.id.clone() }, None, None).unwrap();
    }
    let s = until(&core, "two sessions on it", |s| s.endpoints.first().is_some_and(|e| e.sessions == 2)).await;
    assert_eq!(s.endpoints[0].default_model(), Some(WARM_MODEL));

    // A test connection says what answers, and saves nothing.
    let t = core.test_endpoint(None, &generic.url(), Some(TOKEN)).await.unwrap();
    assert_eq!((t.kind, t.models.as_slice()), (EndpointKind::OpenAi, ["gpt-x".to_string()].as_slice()));
    let t = core.test_endpoint(Some(&ep.id), &a.url(), None).await.unwrap();
    assert_eq!(t.kind, EndpointKind::YmirBridge, "the saved token is used when no new one is typed");
    assert!(core.test_endpoint(Some(&ep.id), &b.url(), None).await.is_err(), "the saved token is wrong for the other bridge");
    assert_eq!(core.state().endpoints.len(), 1);

    // Name, default model and fallback in one save; a model it doesn't list is refused.
    let e = core.edit_endpoint(&ep.id, "Home bridge", &a.url(), None, Some("bridge-big".into()), true).await.unwrap();
    assert_eq!((e.name.as_str(), e.model.as_deref(), e.fallback), ("Home bridge", Some("bridge-big"), true));
    assert_eq!(core.state().default_model().as_deref(), Some("bridge-big"), "with no node, the Weirdnet session uses the fallback's default model");
    core.switch_session(&core.state().sessions.sessions.iter().find(|s| s.name == "One").unwrap().id.clone()).unwrap();
    assert_eq!(core.state().default_model().as_deref(), Some("bridge-big"), "an endpoint session uses the endpoint's default model");
    assert!(core.edit_endpoint(&ep.id, "Home bridge", &a.url(), None, Some("nope".into()), true).await.is_err());

    // A new URL with the old token fails its test and changes nothing; with the new token it saves.
    assert!(core.edit_endpoint(&ep.id, "Home bridge", &b.url(), None, None, true).await.is_err());
    assert_eq!(core.state().endpoints[0].url, a.url());
    let e = core.edit_endpoint(&ep.id, "Home bridge", &b.url(), Some(OTHER_TOKEN.into()), Some("bridge-big".into()), true).await.unwrap();
    assert_eq!(e.url, b.url());
    core.check_endpoint(&ep.id).await.unwrap();
    // A replaced token is in the private file only: not in the state, the settings file, or the old file.
    let st = serde_json::to_string(&core.state()).unwrap();
    let settings = std::fs::read_to_string(tmp.path().join("endpoints.json")).unwrap();
    for t in [TOKEN, OTHER_TOKEN] {
        assert!(!st.contains(t) && !settings.contains(t), "a token leaked");
    }
    let file = std::fs::read_to_string(tmp.path().join("secrets").join(format!("{}.token", ep.id))).unwrap();
    assert_eq!(file.trim(), OTHER_TOKEN);

    // Turning a generic API's fallback on is refused.
    let g = core.add_endpoint("Generic", &generic.url(), TOKEN).await.unwrap();
    assert!(core.edit_endpoint(&g.id, "Generic", &generic.url(), None, None, true).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deleting_an_endpoint_rebinds_or_unbinds_its_sessions_and_keeps_history() {
    let tmp = tempfile::tempdir().unwrap();
    let core = Core::start(bragi_core::Config::new(tmp.path(), "Bragi test")).unwrap();
    let a = FakeBridge::start(true, TOKEN, &[WARM_MODEL], Duration::from_millis(2)).await.unwrap();
    let b = FakeBridge::start(false, TOKEN, &["gpt-x"], Duration::from_millis(2)).await.unwrap();
    let ea = core.add_endpoint("A", &a.url(), TOKEN).await.unwrap();
    let eb = core.add_endpoint("B", &b.url(), TOKEN).await.unwrap();
    let sa = core.create_session("On A", Target::Endpoint { id: ea.id.clone() }, None, None).unwrap();
    let sb = core.create_session("On B", Target::Endpoint { id: eb.id.clone() }, None, None).unwrap();
    for s in [&sa, &sb] {
        core.switch_session(&s.id).unwrap();
        assert!(finish(core.send(None, "Hello there", None).unwrap()).await);
    }
    let before = core.all_conversations(0, 100).unwrap().len();
    assert_eq!(before, 2);

    // Rebind A's session to B, then delete A.
    assert!(core.delete_endpoint(&ea.id, Rebind::To(Target::Endpoint { id: ea.id.clone() })).is_err(), "not onto itself");
    assert_eq!(core.delete_endpoint(&ea.id, Rebind::To(Target::Endpoint { id: eb.id.clone() })).unwrap(), 1);
    let s = core.state();
    assert_eq!(s.sessions.get(&sa.id).unwrap().target, Target::Endpoint { id: eb.id.clone() });
    assert_eq!(s.endpoints.len(), 1);
    assert_eq!(s.endpoints[0].sessions, 2);
    assert!(!tmp.path().join("secrets").join(format!("{}.token", ea.id)).exists());

    // Delete B leaving its sessions unbound: they stay, say so, and send nowhere.
    assert_eq!(core.delete_endpoint(&eb.id, Rebind::Unbound).unwrap(), 2);
    core.switch_session(&sb.id).unwrap();
    let s = core.state();
    assert!(s.endpoints.is_empty());
    assert_eq!(s.route().kind, RouteKind::None);
    assert!(core.send(None, "Anyone?", None).is_err());
    // No history was deleted, and each conversation is still in its session.
    assert_eq!(core.all_conversations(0, 100).unwrap().len(), before);
    assert_eq!(core.conversations_in(&sa.id, 0, 100).unwrap().len(), 1);
    assert_eq!(core.conversations_in(&sb.id, 0, 100).unwrap().len(), 1);
    // An unbound session can be pointed at Weirdnet.
    core.set_session_target(&sb.id, Target::Weirdnet).unwrap();
    assert_eq!(core.sessions().get(&sb.id).unwrap().target, Target::Weirdnet);
    assert!(core.sessions().get(DEFAULT_SESSION).is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn links_are_renamed_edited_after_a_test_connection_and_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    let (demo, core) = linked(tmp.path()).await;
    let other = Arc::new(DemoNode::start(&tmp.path().join("other"), Duration::from_millis(5)).await.unwrap());
    let third = Arc::new(DemoNode::start(&tmp.path().join("third"), Duration::from_millis(5)).await.unwrap());

    // The config's address: renamed, not edited or deleted.
    let s = until(&core, "the links", |s| !s.links.is_empty()).await;
    let fixed = s.links.iter().find(|l| l.address == demo.address()).expect("the demo node's address").clone();
    assert!(fixed.fixed && fixed.name.starts_with("Node …"));
    core.rename_link(&fixed.address, "Demo node").unwrap();
    assert_eq!(core.state().links.iter().find(|l| l.address == fixed.address).unwrap().name, "Demo node");
    assert!(core.edit_link(&fixed.address, &other.address()).await.is_err());
    assert!(core.delete_link(&fixed.address).is_err());

    // A new link is tested first; an address where nothing answers is refused.
    let peer = other.address().rsplit("/p2p/").next().unwrap().to_string();
    let nowhere = format!("/memory/987654321/p2p/{peer}");
    assert!(core.test_link(&nowhere).await.is_err());
    assert!(core.add_link("Nowhere", &nowhere).await.is_err());
    assert!(!core.peers().contains(&nowhere));
    assert!(core.test_link("/ip4/192.0.2.1/tcp/4848").await.is_err(), "an address needs its /p2p/ part");
    let l = core.add_link("Other", &other.address()).await.unwrap();
    assert_eq!((l.name.as_str(), l.peer_id.as_str(), l.fixed), ("Other", peer.as_str(), false));

    // Editing to an address that doesn't answer changes nothing; to one that does, it moves (name too).
    assert!(core.edit_link(&l.address, &nowhere).await.is_err());
    assert!(core.peers().contains(&other.address()));
    let moved = core.edit_link(&l.address, &third.address()).await.unwrap();
    assert_eq!(moved.name, "Other");
    assert!(core.peers().contains(&third.address()) && !core.peers().contains(&other.address()));

    // Deleting forgets it; no session or conversation depends on one link.
    assert!(finish(core.send(None, "Still here?", Some(WARM_MODEL.into())).unwrap()).await);
    let convs = core.all_conversations(0, 100).unwrap().len();
    assert_eq!(core.delete_link(&moved.address).unwrap(), 0);
    assert!(!core.peers().contains(&third.address()));
    assert!(!core.state().links.iter().any(|l| l.address == moved.address));
    assert_eq!(core.all_conversations(0, 100).unwrap().len(), convs);
    let names = std::fs::read_to_string(tmp.path().join("app").join("links.json")).unwrap();
    assert!(names.contains("Demo node") && !names.contains("Other"));
}
