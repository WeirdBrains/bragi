//! Manual check against a real Ymir bridge, ignored by default. It runs in a throwaway profile
//! (a temp dir, no identity, so no Weirdnet node): the bridge is added and must be told apart as a
//! Ymir bridge, made the fallback, and must answer a short prompt. The token is read from a file
//! and never printed.
//!
//!   BRAGI_LIVE_BRIDGE_URL=https://ymir.example.ts.net:9443/v1 BRAGI_LIVE_BRIDGE_TOKEN_FILE=path \
//!     cargo test -p bragi-core --test live_bridge -- --ignored --nocapture

use std::time::Duration;

use bragi_core::{ChatEvent, Config, Core, EndpointKind, RouteKind};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a real Ymir bridge (BRAGI_LIVE_BRIDGE_URL, BRAGI_LIVE_BRIDGE_TOKEN_FILE)"]
async fn a_real_ymir_bridge_is_the_fallback_in_a_throwaway_profile() {
    let url = std::env::var("BRAGI_LIVE_BRIDGE_URL").expect("BRAGI_LIVE_BRIDGE_URL");
    let token =
        std::fs::read_to_string(std::env::var("BRAGI_LIVE_BRIDGE_TOKEN_FILE").expect("BRAGI_LIVE_BRIDGE_TOKEN_FILE")).expect("the token file");
    let tmp = tempfile::tempdir().unwrap();
    let core = Core::start(Config::new(tmp.path(), "Bragi live check")).unwrap();

    let t = core.test_endpoint(None, &url, Some(token.trim())).await.expect("the test connection");
    assert_eq!(t.kind, EndpointKind::YmirBridge, "not a Ymir bridge");
    println!("test connection: Ymir bridge, {} chat models", t.models.len());
    let wrong = core.test_endpoint(None, &url, Some("not-the-token-0000000000")).await;
    assert!(wrong.is_err(), "a wrong token must be refused");

    let ep = core.add_endpoint("Live bridge", &url, token.trim()).await.expect("added");
    core.set_fallback(Some(&ep.id)).unwrap();
    assert_eq!(core.state().route().kind, RouteKind::Fallback);

    let mut h = core.send(None, "Reply with one short sentence: what are you?", None).expect("sent");
    let (mut text, mut via, mut done) = (String::new(), None, false);
    while let Some(ev) = tokio::time::timeout(Duration::from_secs(300), h.events.recv()).await.expect("the answer stalled") {
        match ev {
            ChatEvent::Via { endpoint, fallback, .. } => via = Some((endpoint, fallback)),
            ChatEvent::Delta { offset, text: d } => {
                text = text.chars().take(offset as usize).collect::<String>() + &d;
            }
            ChatEvent::Done { .. } => {
                done = true;
                break;
            }
            ChatEvent::Error { message, .. } => panic!("the bridge failed: {message}"),
            _ => {}
        }
    }
    assert!(done && !text.trim().is_empty(), "no answer");
    assert_eq!(via, Some(("Live bridge".to_string(), true)));
    let model = core.conversation(&h.conversation_id).unwrap().messages.last().and_then(|m| m.model.clone());
    println!("answered via the fallback ({} chars, model {model:?})", text.chars().count());
    core.delete_endpoint(&ep.id, bragi_core::Rebind::Unbound).unwrap();
    assert!(!tmp.path().join("secrets").join(format!("{}.token", ep.id)).exists());
}
