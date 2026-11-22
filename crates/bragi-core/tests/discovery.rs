//! Finding the inference nodes when discovery can't (the nodes on another network, mDNS and the
//! DHT off): the device that links this one says where they are, and an address the user adds
//! is dialed at once. Memory transport, test identities.

use std::time::{Duration, Instant};

use bragi_core::demo::DemoNode;
use bragi_core::{AppState, Config, Core, NodeSearch, PairState, Transport};

async fn until(core: &Core, what: &str, limit: Duration, f: impl Fn(&AppState) -> bool) -> Option<AppState> {
    let deadline = Instant::now() + limit;
    let mut w = core.watch();
    loop {
        let s = w.borrow_and_update().clone();
        if f(&s) {
            return Some(s);
        }
        if Instant::now() >= deadline {
            eprintln!("gave up waiting for {what}: {s:?}");
            return None;
        }
        let _ = tokio::time::timeout(Duration::from_millis(200), w.changed()).await;
    }
}

/// An app that knows no node address at all: only the link can tell it where the nodes are.
fn bare(dir: &std::path::Path) -> Config {
    Config { dir: dir.to_path_buf(), label: "Bragi test".into(), transport: Transport::Memory, peers: Vec::new() }
}

async fn link(core: &Core, accept: impl std::future::Future<Output = ()>) {
    let link = core.pair_offer().await.unwrap();
    assert!(link.starts_with("weirdnet-bragi://"));
    let confirm = async {
        until(core, "the code check", Duration::from_secs(30), |s| matches!(s.pair, PairState::Confirm { .. })).await.expect("code check");
        core.pair_confirm(true).unwrap();
    };
    tokio::join!(accept, confirm);
    until(core, "linked", Duration::from_secs(30), |s| matches!(s.pair, PairState::Linked { .. })).await.expect("linked");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_link_says_where_the_nodes_are() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = DemoNode::start_cloud(&tmp.path().join("demo"), Duration::from_millis(8)).await.unwrap();
    let core = Core::start(bare(&tmp.path().join("app"))).unwrap();
    let d = demo.clone();
    let offer = core.watch();
    link(&core, async move {
        let mut w = offer;
        let code = loop {
            if let PairState::Offering { link, .. } = &w.borrow_and_update().pair {
                break link.clone();
            }
            w.changed().await.unwrap();
        };
        d.accept(&code).await.unwrap();
    })
    .await;
    assert_eq!(core.peers().len(), 2, "both nodes' addresses came with the link");
    until(&core, "both nodes' catalogs", Duration::from_secs(30), |s| s.catalog.nodes.len() == 2).await.expect("both nodes found");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_addresses_an_added_one_finds_the_node_without_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = DemoNode::start_cloud(&tmp.path().join("demo"), Duration::from_millis(8)).await.unwrap();
    let core = Core::start(bare(&tmp.path().join("app"))).unwrap();
    let d = demo.clone();
    let offer = core.watch();
    link(&core, async move {
        let mut w = offer;
        let code = loop {
            if let PairState::Offering { link, .. } = &w.borrow_and_update().pair {
                break link.clone();
            }
            w.changed().await.unwrap();
        };
        d.accept_without_addresses(&code).await.unwrap();
    })
    .await;
    // The bug: linked, but no node is ever found, and the message said only "no inference node found yet".
    assert!(until(&core, "a catalog", Duration::from_secs(4), |s| !s.catalog.nodes.is_empty()).await.is_none());
    let e = core.send(None, "hello", None).err().expect("no node to send to");
    assert!(e.message.contains("2 other linked devices") && e.message.contains("ymirctl pool addresses"), "{}", e.message);

    let address = demo.address();
    assert!(bragi_core::is_node_address(&format!("  {address}\n")));
    assert!(!bragi_core::is_node_address("weirdnet-bragi://link/abc"));
    assert_eq!(core.add_node(&address).unwrap(), bragi_core::NODE_ADDED);
    assert!(core.peers().contains(&address));
    until(&core, "the added node's catalog", Duration::from_secs(30), |s| !s.catalog.nodes.is_empty()).await.expect("the added node found");
    // The status line: looking, then "Connected to <its name>", which clears itself (it used to
    // say "Added the node's address; looking for it…" forever).
    let found = until(&core, "the node found", Duration::from_secs(10), |s| matches!(s.node_search, NodeSearch::Found { .. })).await.expect("found");
    assert_eq!(found.node_search.line().as_deref(), Some("Connected to demo inference node."));
    assert!(!found.node_search.is_problem());
    until(&core, "the line cleared", bragi_core::NODE_FOUND_SHOWN + Duration::from_secs(3), |s| s.node_search == NodeSearch::Idle)
        .await
        .expect("cleared after a few seconds");
    assert!(core.add_node("/ip4/192.0.2.1/tcp/4848").is_err(), "an address without a peer id is refused");
}

/// The node search's other endings: a node that isn't linked, one that never answers (the
/// limit), and the user moving on (dismissed).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_node_search_times_out_into_advice_and_clears_when_dismissed() {
    let tmp = tempfile::tempdir().unwrap();
    let demo = DemoNode::start_cloud(&tmp.path().join("demo"), Duration::from_millis(8)).await.unwrap();
    let core = Core::start(demo.app_config(&tmp.path().join("app"), "Bragi test")).unwrap();
    demo.auto_accept(&core);
    core.pair_offer().await.unwrap();
    until(&core, "the code check", Duration::from_secs(30), |s| matches!(s.pair, PairState::Confirm { .. })).await.expect("code check");
    core.pair_confirm(true).unwrap();
    until(&core, "both nodes", Duration::from_secs(30), |s| s.catalog.nodes.len() == 2).await.expect("both nodes");

    // Not one of the user's devices: said at once.
    core.add_node("/ip4/192.0.2.7/tcp/4848/p2p/12D3KooWQH96LGJd33JquZXNyrmrs7HAeaeviiFqdwnzwfdmj1VY").unwrap();
    let s = core.state();
    assert!(s.node_search.is_problem() && s.node_search.line().unwrap().contains("isn't one of your linked devices"), "{:?}", s.node_search);

    // A linked node that went away: looked for, then advice after the limit.
    let second = demo.siblings()[0].clone();
    let address = second.address();
    second.stop();
    until(&core, "the stopped node gone", Duration::from_secs(30), |s| s.catalog.nodes.len() == 1).await.expect("gone");
    core.set_node_search_limit(Duration::from_secs(1));
    core.add_node(&address).unwrap();
    assert_eq!(core.state().node_search, NodeSearch::Looking { node: "demo inference node 2".into() });
    assert_eq!(core.state().node_search.line().as_deref(), Some("Looking for demo inference node 2…"));
    let s = until(&core, "the advice", Duration::from_secs(10), |s| s.node_search.is_problem()).await.expect("advice");
    let line = s.node_search.line().unwrap();
    assert!(line.contains("Couldn't reach demo inference node 2") && line.contains("ymirctl status"), "{line}");

    // Anything else the user does clears it, and a dismissed search never comes back.
    core.set_node_search_limit(Duration::from_millis(500));
    core.add_node(&address).unwrap();
    core.dismiss_node_search();
    assert_eq!(core.state().node_search, NodeSearch::Idle);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(core.state().node_search, NodeSearch::Idle);
}
