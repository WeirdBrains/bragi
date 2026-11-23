//! Chat and code are modes of one conversation: code turns recorded with `Core::record` are
//! saved in it (marked code) and go to the model with the next chat turn,
//! with turns from the same side joined so the roles still alternate.

mod common;

use bragi_core::{ChatMessage, EndpointInput, EndpointMode, Mode, Role, Target};
use common::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_plan_and_code_turns_are_one_conversation() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, _demo) = linked(tmp.path()).await;
    let mock = mock_openai(None, vec!["mock-1"], Chat::Words(vec!["ok"])).await;
    let ep = core
        .add_endpoint(
            EndpointInput {
                name: "M".into(),
                base_url: mock.url.clone(),
                model: Some("mock-1".into()),
                mode: EndpointMode::Alongside,
                enabled: true,
            },
            None,
        )
        .unwrap();
    let target = Target::Endpoint { id: ep.id.clone(), model: "mock-1".into() };

    // Plan in chat.
    let mut h = core.send_to(None, "plan a hello world", None, Vec::new(), Some(target.clone())).unwrap();
    drain(&mut h).await;
    let id = h.conversation_id.clone();

    // Code: the ask and its answer, then a shell run, recorded in the same conversation.
    let mut answer = ChatMessage::new(Role::Assistant, "Made hello.py.", Mode::Code);
    answer.reasoning = "one file".into();
    answer.model = Some("mock-1".into());
    let got = core
        .record(
            Some(&id),
            vec![ChatMessage::new(Role::User, "make it", Mode::Code), answer, ChatMessage::new(Role::User, "$ python hello.py\nhello", Mode::Code)],
        )
        .unwrap();
    assert_eq!(got, id);
    let c = core.conversation(&id).unwrap();
    let modes: Vec<Mode> = c.messages.iter().map(|m| m.mode).collect();
    assert_eq!(modes, [Mode::Chat, Mode::Chat, Mode::Code, Mode::Code, Mode::Code]);
    assert_eq!(c.messages[3].reasoning, "one file");

    // Back in chat: the model gets the plan and the code turns, roles alternating.
    let mut h = core.send_to(Some(&id), "what next?", None, Vec::new(), Some(target)).unwrap();
    drain(&mut h).await;
    let body = mock.seen.lock().unwrap().iter().rev().find(|s| s.path.ends_with("/chat/completions")).unwrap().body.clone();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let msgs = v["messages"].as_array().unwrap();
    let roles: Vec<&str> = msgs.iter().map(|m| m["role"].as_str().unwrap()).filter(|r| *r != "system").collect();
    assert_eq!(roles, ["user", "assistant", "user", "assistant", "user"], "{body}");
    let last = msgs.last().unwrap()["content"].to_string();
    assert!(last.contains("python hello.py") && last.contains("what next?"), "{last}");

    // A record with no conversation starts one, titled after its first message.
    let new = core.record(None, vec![ChatMessage::new(Role::User, "scaffold a crate", Mode::Code)]).unwrap();
    assert_eq!(core.conversation(&new).unwrap().title, "scaffold a crate");
}
