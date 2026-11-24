//! The end-to-end scenario both shells run (pair, list models, chat, cancel, continue), against
//! `bragi-core` with a demo inference node and a fake `InferHost`, all on the `weirdnet-net`
//! memory transport with test identities. Nothing touches the real network.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bragi_core::demo::{COLD_MODEL, DemoNode, WARM_MODEL, answer_for};
use bragi_core::identity::Identity;
use bragi_core::{AppState, ChatEvent, ChatHandle, Core, ErrorKind, MessageState, ModelStatus, PairState};

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

/// Collects a stream, applying deltas the way a shell must: at their offset.
struct Shown {
    text: String,
    events: Vec<ChatEvent>,
}

async fn collect(h: &mut ChatHandle, base: &str, stop_after: Option<(usize, &Core)>) -> Shown {
    let mut shown = Shown { text: base.to_string(), events: Vec::new() };
    let mut deltas = 0;
    while let Some(ev) = tokio::time::timeout(Duration::from_secs(20), h.events.recv()).await.expect("stream stalled") {
        if let ChatEvent::Delta { offset, text } = &ev {
            let keep: String = shown.text.chars().take(*offset as usize).collect();
            assert_eq!(keep.chars().count() as u64, *offset, "a delta past the end of the text");
            shown.text = keep + text;
            deltas += 1;
            if let Some((n, core)) = stop_after
                && deltas == n
            {
                assert!(core.cancel(&h.request_id));
            }
        }
        let end = matches!(ev, ChatEvent::Done { .. } | ChatEvent::Error { .. });
        shown.events.push(ev);
        if end {
            break;
        }
    }
    shown
}

async fn paired(dir: &std::path::Path, delay_ms: u64) -> (Arc<DemoNode>, Core) {
    let demo = Arc::new(DemoNode::start(&dir.join("demo"), Duration::from_millis(delay_ms)).await.unwrap());
    let core = Core::start(demo.app_config(&dir.join("app"), "Bragi test")).unwrap();
    demo.auto_accept(&core);
    core.pair_offer().await.unwrap();
    let s = until(&core, "the code check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    let PairState::Confirm { sas } = s.pair else { unreachable!() };
    assert_eq!(sas.len(), 6);
    core.pair_confirm(true).unwrap();
    until(&core, "linked", |s| matches!(s.pair, PairState::Linked { .. }) && s.online).await;
    (demo, core)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pair_list_chat_cancel_continue() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = Arc::new(DemoNode::start(&tmp.path().join("demo"), Duration::from_millis(15)).await.unwrap());
    let core = Core::start(demo.app_config(&tmp.path().join("app"), "Bragi test")).unwrap();
    assert_eq!(core.state().pair, PairState::Unpaired);
    assert_eq!(core.send(None, "hi", None).err().unwrap().kind, ErrorKind::NotPaired);

    // 1. Pair: this device shows a code; the existing device opens it; both show the same check.
    let link = core.pair_offer().await.unwrap();
    assert!(link.starts_with("weirdnet-bragi://link/"), "{link}");
    let (sas, ()) = tokio::join!(async { demo.accept(&link).await.unwrap() }, async {
        let s = until(&core, "the code check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
        let PairState::Confirm { sas } = s.pair else { unreachable!() };
        core.pair_confirm(true).unwrap();
        let _ = sas;
    });
    let linked = until(&core, "linked", |s| matches!(s.pair, PairState::Linked { .. }) && s.online).await;
    let PairState::Linked { user, .. } = linked.pair else { unreachable!() };
    assert_eq!(user, demo.identity.user_id());
    assert_eq!(sas.len(), 6);

    // 2. List models: the demo node's chat models, warm first.
    let s = until(&core, "the catalog", |s| s.catalog.models.len() == 2).await;
    assert_eq!(s.catalog.nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), [demo.node.peer_id().to_string().as_str()]);
    assert_eq!(s.catalog.models[0].id, WARM_MODEL);
    assert_eq!(s.catalog.models[0].status, ModelStatus::Warm);
    assert_eq!(s.catalog.models[1].id, COLD_MODEL);

    // 3. Chat: streamed, coalesced, saved.
    let mut h = core.send(None, "hello", None).unwrap();
    let conv = h.conversation_id.clone();
    let got = collect(&mut h, "", None).await;
    assert_eq!(got.text, answer_for(WARM_MODEL, "hello"));
    assert!(matches!(got.events[0], ChatEvent::Started { ref model, .. } if model == WARM_MODEL), "{:?}", got.events[0]);
    assert!(matches!(got.events.last(), Some(ChatEvent::Done { .. })));
    let c = core.conversation(&conv).unwrap();
    assert_eq!(c.messages.len(), 2);
    assert_eq!(c.messages[1].state, MessageState::Complete);
    assert_eq!(c.messages[1].node.as_deref(), Some(demo.node.peer_id().to_string().as_str()));

    // 4. Cancel: the stop button ends the stream; what was shown is kept.
    let mut h = core.send(Some(&conv), "tell me more", Some(COLD_MODEL.into())).unwrap();
    let got = collect(&mut h, "", Some((3, &core))).await;
    let Some(ChatEvent::Error { kind, partial, .. }) = got.events.last() else { panic!("{:?}", got.events) };
    assert_eq!(*kind, ErrorKind::Cancelled);
    assert_eq!(*partial, got.text);
    let full = answer_for(COLD_MODEL, "tell me more");
    assert!(full.starts_with(&got.text) && got.text.len() < full.len(), "{}", got.text);
    let c = core.conversation(&conv).unwrap();
    assert_eq!(c.messages[3].state, MessageState::Stopped);
    assert_eq!(c.messages[3].text, got.text);
    assert!(!core.cancel(&h.request_id), "the request is over");

    // 5. Continue: the answer goes on after what was shown, in the same message.
    let mut h = core.continue_answer(&conv, None).unwrap();
    let cont = collect(&mut h, &got.text, None).await;
    assert_eq!(cont.text, full);
    let first = cont.events.iter().find_map(|e| if let ChatEvent::Delta { offset, .. } = e { Some(*offset) } else { None });
    assert_eq!(first, Some(got.text.chars().count() as u64), "continues after the partial text");
    let c = core.conversation(&conv).unwrap();
    assert_eq!(c.messages.len(), 4);
    assert_eq!((c.messages[3].state, c.messages[3].text.as_str()), (MessageState::Complete, full.as_str()));
    assert!(core.continue_answer(&conv, None).is_err(), "nothing left to continue");
    assert_eq!(core.conversations(0, 10).unwrap().len(), 1);

    // A restart comes back online with the stored identity and history.
    let dir = tmp.path().join("app");
    core.shutdown();
    drop(core);
    let again = Core::start(demo.app_config(&dir, "Bragi test")).unwrap();
    until(&again, "back online", |s| s.online && s.catalog.models.len() == 2).await;
    assert_eq!(again.conversation(&conv).unwrap().messages.len(), 4);
    let mut h = again.send(Some(&conv), "and again", None).unwrap();
    assert_eq!(collect(&mut h, "", None).await.text, answer_for(WARM_MODEL, "and again"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codes_that_differ_link_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = Arc::new(DemoNode::start(&tmp.path().join("demo"), Duration::from_millis(5)).await.unwrap());
    let core = Core::start(demo.app_config(&tmp.path().join("app"), "Bragi test")).unwrap();
    let link = core.pair_offer().await.unwrap();
    let accepting = {
        let (d, l) = (demo.clone(), link.clone());
        tokio::spawn(async move { d.accept(&l).await })
    };
    until(&core, "the code check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    core.pair_confirm(false).unwrap();
    until(&core, "failed", |s| matches!(s.pair, PairState::Failed { .. })).await;
    let _ = accepting.await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!Identity::exists(&tmp.path().join("app/identity")), "no identity was stored");
    assert!(!core.state().online);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_stranger_is_refused_and_a_linked_device_links_another() {
    let tmp = tempfile::tempdir().unwrap();
    let (demo, core) = paired(tmp.path(), 5).await;
    until(&core, "the catalog", |s| !s.catalog.models.is_empty()).await;

    // A device with an identity of its own: the demo node won't answer it.
    let sdir = tmp.path().join("stranger");
    Identity::create(&sdir.join("identity"), "stranger").unwrap();
    let stranger = Core::start(demo.app_config(&sdir, "stranger")).unwrap();
    until(&stranger, "online", |s| s.online).await;
    let e = stranger.send(None, "let me in", Some(WARM_MODEL.into())).unwrap();
    let mut h = e;
    let got = collect(&mut h, "", None).await;
    assert!(matches!(got.events.last(), Some(ChatEvent::Error { .. })), "{:?}", got.events);
    assert!(stranger.state().catalog.models.is_empty());

    // From the linked device, link a third one (the phone in the scenario's other shell).
    let third = Core::start(demo.app_config(&tmp.path().join("third"), "Bragi phone")).unwrap();
    let link = third.pair_offer().await.unwrap();
    let check = core.pair_accept(&link).await.unwrap();
    assert_eq!(check.device, "Bragi phone");
    let s = until(&third, "the code check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    assert_eq!(s.pair, PairState::Confirm { sas: check.sas.clone() });
    third.pair_confirm(true).unwrap();
    core.pair_accept_confirm(true).await.unwrap();
    let s = until(&third, "linked", |s| matches!(s.pair, PairState::Linked { .. })).await;
    let PairState::Linked { user, .. } = s.pair else { unreachable!() };
    assert_eq!(user, demo.identity.user_id());
    assert!(third.pair_offer().await.is_err(), "already linked");
}

/// A code shown by Bragi but carried under Ymir's or Morph's scheme (what `ymirctl pool add`
/// and Morph read) links the same way once the opening device normalizes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn codes_under_another_apps_scheme_link_too() {
    for scheme in ["weirdnet-ymir://", "weirdnet-morph://"] {
        let tmp = tempfile::tempdir().unwrap();
        let demo = Arc::new(DemoNode::start(&tmp.path().join("demo"), Duration::from_millis(5)).await.unwrap());
        let core = Core::start(demo.app_config(&tmp.path().join("app"), "Bragi test")).unwrap();
        let link = core.pair_offer().await.unwrap();
        let other = link.replacen("weirdnet-bragi://", scheme, 1);
        assert_ne!(other, link);
        let accepting = {
            let d = demo.clone();
            tokio::spawn(async move { d.accept(&other).await })
        };
        until(&core, "the code check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
        core.pair_confirm(true).unwrap();
        until(&core, "linked", |s| matches!(s.pair, PairState::Linked { .. }) && s.online).await;
        accepting.await.unwrap().unwrap();
    }
}

/// A Ymir server linked cert-only from Bragi (`ymirctl pool link --cert-only` on the server):
/// it gets a certificate and a signed service grant, never the seed, and the grant stays in
/// this device's roster.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ymir_server_is_linked_cert_only_without_the_seed() {
    use bragi_core::ServerRole;
    use weirdnet_device_link::{HostNode, HostStatus, Received};
    use weirdnet_net::{Keypair, PublicKey};
    use weirdnet_sync::{DeviceClass, LinkBundle, SyncState};

    let tmp = tempfile::tempdir().unwrap();
    let (demo, core) = paired(tmp.path(), 5).await;
    until(&core, "the catalog", |s| !s.catalog.models.is_empty()).await;
    // None: the default confirm, which links a server's code cert-only as inference.
    for (name, chosen) in [("model-srv", Some(ServerRole::Model)), ("inf-srv", Some(ServerRole::Inference)), ("default-srv", None)] {
        let role = chosen.unwrap_or(ServerRole::Inference);
        let cfg = demo.app_config(&tmp.path().join(name), name);
        let net = bragi_core::node_config(&cfg, true).unwrap();
        let key = Keypair::generate_ed25519();
        let peer = key.public().to_peer_id().to_string();
        let k = key.clone();
        let host = tokio::task::spawn_blocking(move || HostNode::start("weirdnet-ymir://", k, net, "Ymir on server")).await.unwrap().unwrap();
        let check = core.pair_accept(host.link()).await.unwrap();
        assert_eq!(check.device, "Ymir on server");
        assert!(core.pairing_is_server());
        let deadline = Instant::now() + Duration::from_secs(20);
        while !matches!(host.status(), HostStatus::Paired { .. }) {
            assert!(Instant::now() < deadline, "{:?}", host.status());
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        host.confirm();
        match chosen {
            Some(r) => core.pair_accept_confirm_server(true, r).await.unwrap(),
            None => core.pair_accept_confirm(true).await.unwrap(),
        }
        while !matches!(host.status(), HostStatus::Received { .. }) {
            assert!(Instant::now() < deadline, "{:?}", host.status());
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let Received::Service(c) = host.take_any().unwrap() else { panic!("the seed was sent") };
        assert_eq!(c.user_id, demo.identity.user_id());
        let bundle = LinkBundle::from_bytes(&c.bundle).unwrap();
        assert_eq!(bundle.grant.class, DeviceClass::Service);
        assert_eq!(bundle.grant.roles, role.roles());
        assert_eq!(bundle.grant.roster_only(), role == ServerRole::Model);
        assert_eq!(bundle.keys.is_some(), role == ServerRole::Inference);
        // What the server does with it: a service state, no seed.
        let user_key = PublicKey::try_decode_protobuf(&c.user_key).unwrap();
        let st = SyncState::new_service(user_key, c.node_cert.clone(), &key, vec!["ymir.pool".into()], &bundle).unwrap();
        assert!(st.is_service() && st.seed.is_none());
        // This device keeps its certificate and grant.
        let mine = Identity::open(&tmp.path().join("app/identity")).unwrap().unwrap();
        assert!(mine.roster().active().any(|cert| cert.peer_id == peer));
        assert!(mine.is_service(&peer));
    }
    // The opt-out: the full-identity link sends the seed to a server's code.
    let cfg = demo.app_config(&tmp.path().join("full-srv"), "full-srv");
    let net = bragi_core::node_config(&cfg, true).unwrap();
    let key = Keypair::generate_ed25519();
    let host = tokio::task::spawn_blocking(move || HostNode::start("weirdnet-ymir://", key, net, "Ymir on server")).await.unwrap().unwrap();
    core.pair_accept(host.link()).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !matches!(host.status(), HostStatus::Paired { .. }) {
        assert!(Instant::now() < deadline, "{:?}", host.status());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    host.confirm();
    core.pair_accept_confirm_full(true).await.unwrap();
    while !matches!(host.status(), HostStatus::Received { .. }) {
        assert!(Instant::now() < deadline, "{:?}", host.status());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(matches!(host.take_any().unwrap(), Received::Full(_)), "--full-identity sends the seed");
}
