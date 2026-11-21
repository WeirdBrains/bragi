//! A real OpenAI-compatible server, on request only:
//! `BRAGI_LIVE_URL=http://127.0.0.1:8484/v1 BRAGI_LIVE_KEY=... cargo test -p bragi-core --test live -- --ignored`
//! (the Ymir loopback bridge works; so does a local llama-server). The key comes from the
//! environment and is never printed.

mod common;

use bragi_core::{ChatEvent, EndpointInput, EndpointMode, Target};
use common::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a live server"]
async fn live_endpoint_lists_models_and_streams_a_chat() {
    let url = std::env::var("BRAGI_LIVE_URL").expect("BRAGI_LIVE_URL");
    let key = std::env::var("BRAGI_LIVE_KEY").ok().filter(|k| !k.is_empty());
    let tmp = tempfile::tempdir().unwrap();
    let (core, _demo) = linked(tmp.path()).await;

    let r = core.test_endpoint_address(&url, None).await.unwrap();
    println!("without a key: ok={} {}", r.ok, r.message);
    let r = core.test_endpoint_address(&url, key.clone()).await.unwrap();
    println!("with the key: ok={} models={:?} {} ms {}", r.ok, r.models, r.latency_ms, r.message);
    assert!(r.ok, "{r:?}");
    let model = std::env::var("BRAGI_LIVE_MODEL").ok().or_else(|| r.models.first().cloned()).expect("a model");

    let input = EndpointInput { name: "Live".into(), base_url: url, model: Some(model.clone()), mode: EndpointMode::Alongside, enabled: true };
    let v = core.add_endpoint(input, key).unwrap();
    let mut h = core
        .send_to(None, "Reply with exactly the word: pong", None, Vec::new(), Some(Target::Endpoint { id: v.id.clone(), model: model.clone() }))
        .unwrap();
    let conv = h.conversation_id.clone();
    let events = drain(&mut h).await;
    let text = answer_text(&events);
    println!("model={model} events={} answer={:?}", events.len(), text.chars().take(200).collect::<String>());
    assert!(matches!(events.last(), Some(ChatEvent::Done { .. })), "{events:?}");
    assert!(!text.trim().is_empty());
    let s = core.sessions(0, 10).unwrap();
    assert_eq!(s.iter().find(|x| x.id == conv).map(|x| x.target_label.clone()), Some(format!("Live · {model}")));
}
