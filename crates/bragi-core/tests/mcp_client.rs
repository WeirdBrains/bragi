//! Outside MCP servers through `Core` (LF-37): a mock OpenAI endpoint that asks for a tool, a demo
//! MCP server over HTTP with a bearer token, and the whole loop: approval card, tool call, result
//! fed back, answer. Also: "always allow", a declined card, a server that is down, and secrets.

mod common;

use std::sync::Arc;

use bragi_core::{ChatEvent, Choice, Core, EndpointInput, EndpointMode, McpScope, McpSecrets, McpServerInput, McpTransport, Target};
use common::*;
use serde_json::json;

struct Demo;

const DEMO_TOOLS: &[weirdnet_mcp::ToolSpec] = &[weirdnet_mcp::ToolSpec {
    name: "echo",
    description: "Repeats the text.",
    params: &[weirdnet_mcp::Param::required("text", weirdnet_mcp::ParamType::String, "Text")],
    auth: false,
    read_only: true,
    destructive: false,
    open_world: false,
}];

impl weirdnet_mcp::Tools for Demo {
    fn specs(&self) -> &[weirdnet_mcp::ToolSpec] {
        DEMO_TOOLS
    }
    fn call(&self, _t: &weirdnet_mcp::ToolSpec, a: &serde_json::Value, _token: Option<&str>) -> Result<serde_json::Value, weirdnet_mcp::ToolError> {
        Ok(json!({ "echo": a["text"] }))
    }
}

async fn demo_server(token: &str) -> String {
    use weirdnet_mcp::http::{Auth, Guard};
    let server = Arc::new(weirdnet_mcp::Server::new(
        weirdnet_mcp::ServerInfo { name: "demo".into(), title: "Demo".into(), version: "1".into(), instructions: String::new() },
        Demo,
    ));
    let app = weirdnet_mcp::http::router("/mcp", server, Guard::loopback(Auth::token(token)));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

fn server_input(name: &str, url: &str) -> McpServerInput {
    McpServerInput {
        name: name.into(),
        transport: McpTransport::Http,
        enabled: true,
        command: None,
        args: vec![],
        cwd: None,
        url: Some(url.into()),
        scope: McpScope::All,
        timeout_secs: Some(10),
    }
}

fn bearer(t: &str) -> McpSecrets {
    McpSecrets { bearer: Some(t.into()), ..Default::default() }
}

async fn endpoint(core: &Core, url: &str) -> Target {
    let v = core
        .add_endpoint(
            EndpointInput { name: "Mock".into(), base_url: url.into(), model: Some("mock-1".into()), mode: EndpointMode::Alongside, enabled: true },
            None,
        )
        .unwrap();
    Target::Endpoint { id: v.id, model: "mock-1".into() }
}

/// Sends a chat turn, answering each approval card with `choice`; returns the events.
async fn ask(core: &Core, target: Option<Target>, conv: Option<&str>, choice: Choice) -> Vec<ChatEvent> {
    let mut h = core.send_to(conv, "say it", None, Vec::new(), target).unwrap();
    let mut events = Vec::new();
    while let Some(ev) = tokio::time::timeout(std::time::Duration::from_secs(30), h.events.recv()).await.expect("stream stalled") {
        if let ChatEvent::Approval { ask, .. } = &ev {
            assert!(core.approve(&h.request_id, *ask, choice));
        }
        let end = matches!(ev, ChatEvent::Done { .. } | ChatEvent::Error { .. });
        events.push(ev);
        if end {
            break;
        }
    }
    events
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_chat_on_an_endpoint_uses_an_outside_servers_tool_after_the_card_is_approved() {
    let tmp = tempfile::tempdir().unwrap();
    let core = Core::start(bragi_core::Config::new(tmp.path().join("app"), "t")).unwrap();
    let url = demo_server("tok-123").await;
    let mock = mock_openai(None, vec!["mock-1"], Chat::Tools("demo.echo", r#"{"text":"hi from the server"}"#, vec!["It said: ", "hi from the server"])).await;
    let target = endpoint(&core, &mock.url).await;

    // No server yet: a plain chat, no tools sent.
    let events = ask(&core, Some(target.clone()), None, Choice::Once).await;
    assert!(!events.iter().any(|e| matches!(e, ChatEvent::Tool { .. })));

    let v = core.add_mcp_server(server_input("Demo", &url), bearer("tok-123")).unwrap();
    assert!(v.has_bearer && v.warning.is_none());
    let report = core.test_mcp_server(&v.id).await;
    assert!(report.ok && report.tools == ["demo.echo"], "{report:?}");

    let events = ask(&core, Some(target.clone()), None, Choice::Once).await;
    assert!(matches!(events.last(), Some(ChatEvent::Done { .. })), "{events:?}");
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Approval { tool, .. } if tool == "demo.echo")));
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Tool { tool, state: bragi_core::ToolState::Done, .. } if tool == "demo.echo")));
    assert!(answer_text(&events).contains("hi from the server"));
    // The request offered the tool as an OpenAI function tool, and the result went back as a tool message.
    let bodies: Vec<serde_json::Value> =
        mock.requests().iter().filter(|s| s.path.ends_with("/chat/completions")).map(|s| serde_json::from_str(&s.body).unwrap()).collect();
    let with_tools = bodies.iter().find(|b| b.get("tools").is_some()).expect("tools were sent");
    assert_eq!(with_tools["tools"][0]["function"]["name"], "demo.echo");
    let last = bodies.last().unwrap();
    let msgs = last["messages"].as_array().unwrap();
    assert!(msgs.iter().any(|m| m["role"] == "assistant" && m["tool_calls"][0]["function"]["name"] == "demo.echo"));
    assert!(msgs.iter().any(|m| m["role"] == "tool" && m["tool_call_id"] == "call_1" && m["content"].as_str().unwrap().contains("hi from the server")));

    // Declined: the tool never runs.
    let events = ask(&core, Some(target.clone()), None, Choice::Deny).await;
    assert!(!events.iter().any(|e| matches!(e, ChatEvent::Tool { state: bragi_core::ToolState::Done, .. })));

    // Always allow: the next call runs without a card.
    let events = ask(&core, Some(target.clone()), None, Choice::Always).await;
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Tool { state: bragi_core::ToolState::Done, .. })));
    assert!(core.mcp_servers()[0].always_allow.contains(&"demo.echo".to_string()));
    let events = ask(&core, Some(target.clone()), None, Choice::Deny).await;
    assert!(!events.iter().any(|e| matches!(e, ChatEvent::Approval { .. })), "no card for an always-allowed tool");
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Tool { state: bragi_core::ToolState::Done, .. })));
    core.set_mcp_tool_always_allowed(&v.id, "demo.echo", false).unwrap();

    // Disabled: back to a plain chat.
    core.set_mcp_server_enabled(&v.id, false).unwrap();
    let events = ask(&core, Some(target), None, Choice::Once).await;
    assert!(!events.iter().any(|e| matches!(e, ChatEvent::Approval { .. })));
    core.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_server_that_is_down_or_refuses_the_token_is_left_out_and_secrets_stay_out_of_files() {
    let tmp = tempfile::tempdir().unwrap();
    let core = Core::start(bragi_core::Config::new(tmp.path().join("app"), "t")).unwrap();
    let url = demo_server("right-token").await;
    let mock = mock_openai(None, vec!["mock-1"], Chat::Words(vec!["plain answer"])).await;
    let target = endpoint(&core, &mock.url).await;

    let bad = core.add_mcp_server(server_input("Wrong", &url), bearer("sk-wrong-token-xyz")).unwrap();
    let down = core.add_mcp_server(server_input("Down", "http://127.0.0.1:1/mcp"), bearer("sk-other-secret")).unwrap();
    let r = core.test_mcp_server(&bad.id).await;
    assert!(!r.ok && r.message.contains("refused the token"), "{r:?}");
    assert!(!core.test_mcp_server(&down.id).await.ok);

    // Both left out: the model gets no tools, and the chat still answers (with notices).
    let events = ask(&core, Some(target), None, Choice::Once).await;
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Notice { text } if text.contains("Wrong left out"))), "{events:?}");
    assert!(matches!(events.last(), Some(ChatEvent::Done { .. })));

    for secret in ["sk-wrong-token-xyz", "sk-other-secret"] {
        for f in walk(tmp.path().join("app")) {
            let bytes = std::fs::read(&f).unwrap_or_default();
            assert!(!String::from_utf8_lossy(&bytes).contains(secret), "{secret} in {f:?}");
        }
        assert!(!format!("{:?}", core.mcp_servers()).contains(secret));
    }
    core.shutdown();
}

fn walk(dir: std::path::PathBuf) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() { out.extend(walk(p)) } else { out.push(p) }
    }
    out
}
