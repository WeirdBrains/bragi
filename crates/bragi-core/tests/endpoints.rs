//! OpenAI-compatible endpoints through `Core`, against a mock server: model listing, auth,
//! streaming (content, reasoning, usage), errors, fallback, the key store, and Settings
//! add/edit/delete/reorder/test.

mod common;

use bragi_core::{ChatEvent, Core, EndpointInput, EndpointMode, ErrorKind, KeyChange, MessageState, Target};
use common::*;

fn input(name: &str, url: &str, mode: EndpointMode) -> EndpointInput {
    EndpointInput { name: name.into(), base_url: url.into(), model: Some("mock-1".into()), mode, enabled: true }
}

fn endpoint_target(core: &Core, id: &str) -> Target {
    let _ = core;
    Target::Endpoint { id: id.into(), model: "mock-1".into() }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_and_list_models_with_and_without_a_key() {
    let tmp = tempfile::tempdir().unwrap();
    let mock = mock_openai(Some("sk-good"), vec!["b-model", "a-model"], Chat::Words(vec![])).await;
    let core = Core::start(bragi_core::Config::new(tmp.path().join("app"), "t")).unwrap();

    let r = core.test_endpoint_address(&mock.url, None).await.unwrap();
    assert!(!r.ok && r.message.contains("refused the API key"), "{r:?}");
    let r = core.test_endpoint_address(&mock.url, Some("sk-bad".into())).await.unwrap();
    assert!(!r.ok, "{r:?}");
    let r = core.test_endpoint_address(&mock.url, Some("sk-good".into())).await.unwrap();
    assert!(r.ok && r.models == ["a-model", "b-model"], "{r:?}");

    let v = core.add_endpoint(input("Mock", &mock.url, EndpointMode::Alongside), Some("sk-good".into())).unwrap();
    assert!(v.has_key && !v.insecure);
    assert_eq!(core.endpoint_models(&v.id).await.unwrap(), ["a-model", "b-model"]);
    assert!(core.test_endpoint(&v.id).await.unwrap().ok);
    // Every request used the bearer key.
    assert!(mock.requests().iter().rev().take(2).all(|s| s.auth.as_deref() == Some("Bearer sk-good")));

    // A server that isn't an OpenAI API, and one that isn't there.
    assert!(core.test_endpoint_address("http://127.0.0.1:9/v1", None).await.unwrap().message.contains("connect"));
    assert!(core.test_endpoint_address("nonsense", None).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streams_into_a_session_and_keeps_the_key_out_of_everything() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, _demo) = linked(tmp.path()).await;
    let mock = mock_openai(Some("sk-top-secret"), vec!["mock-1"], Chat::Reasoning(vec!["hmm ", "ok"], vec!["Hel", "lo ", "there"])).await;
    let v = core.add_endpoint(input("Mock", &mock.url, EndpointMode::Alongside), Some("sk-top-secret".into())).unwrap();

    let mut h = core.send_to(None, "hi", None, Vec::new(), Some(endpoint_target(&core, &v.id))).unwrap();
    let conv = h.conversation_id.clone();
    let events = drain(&mut h).await;
    assert!(matches!(events.last(), Some(ChatEvent::Done { finish_reason: Some(f) }) if f == "stop"), "{events:?}");
    assert_eq!(answer_text(&events), "Hello there");
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Started { node, model, .. } if node == "Mock" && model == "mock-1")));
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Reasoning { .. })));
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Usage { prompt_tokens: 7, completion_tokens: 3 })));

    let c = core.conversation(&conv).unwrap();
    assert_eq!((c.messages[1].text.as_str(), c.messages[1].state), ("Hello there", MessageState::Complete));
    assert_eq!(c.messages[1].reasoning, "hmm ok");
    assert_eq!(c.messages[1].node.as_deref(), Some("Mock"));
    assert_eq!(c.messages[1].model.as_deref(), Some("mock-1"));

    // The request: bound model, streamed, our messages, bearer key.
    let sent = mock.requests().into_iter().find(|s| s.path.ends_with("/chat/completions")).unwrap();
    assert_eq!(sent.auth.as_deref(), Some("Bearer sk-top-secret"));
    let body: serde_json::Value = serde_json::from_str(&sent.body).unwrap();
    assert_eq!((body["model"].as_str(), body["stream"].as_bool()), (Some("mock-1"), Some(true)));
    assert_eq!(body["messages"][0]["role"], "user");
    assert!(body["messages"][0]["content"].as_str().unwrap().contains("hi"));

    // A second turn needs no target: the session is bound, and the history goes along.
    let mut h2 = core.send_to(Some(&conv), "again", None, Vec::new(), None).unwrap();
    assert_eq!(drain(&mut h2).await.last().map(|e| matches!(e, ChatEvent::Done { .. })), Some(true));
    let second = mock.requests().into_iter().rev().find(|s| s.path.ends_with("/chat/completions")).unwrap();
    let body: serde_json::Value = serde_json::from_str(&second.body).unwrap();
    assert_eq!(body["messages"].as_array().unwrap().len(), 3);

    // The key is nowhere: not in the app's folder as text, not in the views, not in any error.
    let mut found = Vec::new();
    for e in walk(tmp.path().join("app")) {
        if std::fs::read(&e).map(|b| String::from_utf8_lossy(&b).contains("sk-top-secret")).unwrap_or(false) {
            found.push(e);
        }
    }
    assert!(found.is_empty(), "key in plaintext: {found:?}");
    assert!(!format!("{:?}", core.endpoints()).contains("sk-top-secret"));
    assert!(!serde_json::to_string(&core.sessions(0, 10).unwrap()).unwrap().contains("sk-top-secret"));
}

fn walk(dir: std::path::PathBuf) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() { out.extend(walk(p)) } else { out.push(p) }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn endpoint_errors_are_typed_and_the_answer_can_continue() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, _demo) = linked(tmp.path()).await;
    for (status, body, kind) in [
        (401, r#"{"error":{"message":"bad key"}}"#, ErrorKind::Refused),
        (429, "{}", ErrorKind::Busy),
        (404, "{}", ErrorKind::BadRequest),
        (500, r#"{"error":"boom"}"#, ErrorKind::Unavailable),
    ] {
        let mock = mock_openai(None, vec!["mock-1"], Chat::Status(status, body)).await;
        let v = core.add_endpoint(input("E", &mock.url, EndpointMode::Alongside), None).unwrap();
        let mut h = core.send_to(None, "hi", None, Vec::new(), Some(endpoint_target(&core, &v.id))).unwrap();
        let events = drain(&mut h).await;
        match events.last() {
            Some(ChatEvent::Error { kind: k, message, .. }) => {
                assert_eq!(*k, kind, "{status}: {message}");
                assert!(!message.is_empty());
            }
            other => panic!("{status}: {other:?}"),
        }
        let c = core.conversation(&h.conversation_id).unwrap();
        assert_eq!(c.messages[1].state, MessageState::Failed);
    }
    // A stream that ends with no finish keeps what arrived and fails the message.
    let mock = mock_openai(None, vec!["mock-1"], Chat::CutOff(vec!["half ", "an"])).await;
    let v = core.add_endpoint(input("Cut", &mock.url, EndpointMode::Alongside), None).unwrap();
    let mut h = core.send_to(None, "hi", None, Vec::new(), Some(endpoint_target(&core, &v.id))).unwrap();
    let events = drain(&mut h).await;
    assert!(matches!(events.last(), Some(ChatEvent::Error { partial, .. }) if partial == "half an"), "{events:?}");
    assert!(core.conversation(&h.conversation_id).unwrap().continuable());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fallback_endpoints_answer_when_the_pool_cannot_and_stay_out_of_the_way_otherwise() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, demo) = linked(tmp.path()).await;
    let down = mock_openai(None, vec!["mock-1"], Chat::Status(500, "{}")).await;
    let up = mock_openai(None, vec!["mock-1"], Chat::Words(vec!["from ", "fallback"])).await;
    let off = mock_openai(None, vec!["mock-1"], Chat::Words(vec!["disabled"])).await;
    let a = core.add_endpoint(input("Down", &down.url, EndpointMode::Fallback), None).unwrap();
    core.add_endpoint(input("Up", &up.url, EndpointMode::Fallback), None).unwrap();
    let mut disabled = input("Off", &off.url, EndpointMode::Fallback);
    disabled.enabled = false;
    core.add_endpoint(disabled, None).unwrap();

    // The pool is up: the fallbacks aren't called.
    let mut h = core.send(None, "hello", None).unwrap();
    drain(&mut h).await;
    assert!(down.requests().is_empty() && up.requests().is_empty());

    // The pool is gone: the first fallback fails before any text, the second answers.
    demo.stop();
    until(&core, "no models", |s| s.catalog.models.is_empty()).await;
    let mut h = core.send(None, "hello", None).unwrap();
    let events = drain(&mut h).await;
    assert_eq!(answer_text(&events), "from fallback", "{events:?}");
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Retry { .. })));
    assert!(off.requests().is_empty(), "a disabled endpoint is never used");
    assert_eq!(core.conversation(&h.conversation_id).unwrap().messages[1].node.as_deref(), Some("Up"));
    // Order matters: moving Up first means Down isn't tried again.
    let before = down.requests().len();
    let up_id = core.endpoints()[1].id.clone();
    core.move_endpoint(&up_id, 0).unwrap();
    assert_eq!(core.endpoints()[0].name, "Up");
    let mut h = core.send(None, "hello", None).unwrap();
    drain(&mut h).await;
    assert_eq!(down.requests().len(), before);
    // Fallback-only endpoints aren't offered as targets.
    assert_eq!(core.targets().len(), 1);
    let _ = a;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn settings_add_edit_reorder_delete() {
    let tmp = tempfile::tempdir().unwrap();
    let core = Core::start(bragi_core::Config::new(tmp.path().join("app"), "t")).unwrap();
    let a = core.add_endpoint(input("A", "https://a.example.com/v1/", EndpointMode::Alongside), Some("ka".into())).unwrap();
    let b = core.add_endpoint(input("B", "http://b.example.com/v1", EndpointMode::Fallback), None).unwrap();
    assert!(b.insecure && !a.insecure);
    assert_eq!(core.targets().len(), 2, "pool and the alongside endpoint");

    let e = core.edit_endpoint(&a.id, input("A2", "https://a2.example.com/v1", EndpointMode::Fallback), KeyChange::Keep).unwrap();
    assert!(e.has_key && e.name == "A2" && e.mode == EndpointMode::Fallback);
    assert_eq!(core.targets().len(), 1);
    assert!(!core.edit_endpoint(&a.id, input("A2", "https://a2.example.com/v1", EndpointMode::Fallback), KeyChange::Clear).unwrap().has_key);

    core.move_endpoint(&b.id, 0).unwrap();
    // Settings survive a restart (a new core on the same folder).
    drop(core);
    let core = Core::start(bragi_core::Config::new(tmp.path().join("app"), "t")).unwrap();
    let names: Vec<String> = core.endpoints().into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["B", "A2"]);
    core.delete_endpoint(&b.id, None).unwrap();
    assert_eq!(core.endpoints().len(), 1);
    assert_eq!(core.delete_endpoint(&b.id, None).unwrap_err().kind, ErrorKind::NotFound);
    assert!(core.add_endpoint(input("", "https://x", EndpointMode::Fallback), None).is_err());
    assert!(core.edit_endpoint("ep-nope", input("x", "https://x", EndpointMode::Fallback), KeyChange::Keep).is_err());
}

/// Code requests (`complete_to`) go where a chat would, the pool or an endpoint, and leave no
/// conversation behind.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn code_requests_route_by_target_and_stay_out_of_history() {
    use bragi_core::code::{PromptMessage, PromptRole};
    let tmp = tempfile::tempdir().unwrap();
    let (core, _demo) = linked(tmp.path()).await;
    let mock = mock_openai(None, vec!["mock-1"], Chat::Words(vec!["fn ", "a() {}"])).await;
    let v = core.add_endpoint(input("Mock", &mock.url, EndpointMode::Alongside), None).unwrap();
    let ask = || {
        vec![PromptMessage { role: PromptRole::System, content: "code".into() }, PromptMessage { role: PromptRole::User, content: "rewrite".into() }]
    };

    let mut h = core.complete_to(ask(), None, Some(endpoint_target(&core, &v.id))).unwrap();
    assert!(h.conversation_id.is_empty());
    let events = drain(&mut h).await;
    assert_eq!(answer_text(&events), "fn a() {}");
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Started { node, .. } if node == "Mock")));

    let mut h = core.complete_to(ask(), None, Some(Target::Pool)).unwrap();
    let events = drain(&mut h).await;
    assert!(matches!(events.last(), Some(ChatEvent::Done { .. })), "{events:?}");
    assert!(!answer_text(&events).is_empty());

    assert!(core.complete_to(vec![], None, None).is_err());
    assert!(core.conversations(0, 10).unwrap().is_empty(), "nothing was saved");
}
