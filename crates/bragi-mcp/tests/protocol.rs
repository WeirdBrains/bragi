//! `bragi mcp` against the in-process demo node: raw MCP JSON-RPC through `weirdnet-mcp`'s
//! protocol layer (no transport), then loopback HTTP with the token. Nothing here touches the
//! network, a real model or a real Bragi home.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bragi_core::{Core, EndpointInput, EndpointMode};
use bragi_mcp::{BragiTools, Options};
use serde_json::{Value, json};
use weirdnet_mcp::Server;

type Rpc = Server<BragiTools>;

/// A 1×1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

fn rpc(core: &Core, root: &Path, allow: bool) -> Arc<Rpc> {
    let journal = root.parent().unwrap().join("undo");
    let opts = Options { allow_deletes: allow, allow_writes: allow, root: root.to_path_buf(), timeout: Duration::from_secs(60), journal };
    Arc::new(bragi_mcp::server(BragiTools::new(core.clone(), opts).unwrap()))
}

async fn call(rpc: &Rpc, method: &str, params: Value) -> Value {
    rpc.handle_async(None, json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params})).await.expect("a reply")
}

/// A tool's result; fails the test on a tool error.
async fn ok(rpc: &Rpc, name: &str, args: Value) -> Value {
    let r = call(rpc, "tools/call", json!({"name": name, "arguments": args})).await;
    assert_eq!(r["result"]["isError"], false, "{name} failed: {r}");
    r["result"].clone()
}

/// A tool's error text (a tool error or a protocol error); fails the test when the tool succeeds.
async fn fails(rpc: &Rpc, name: &str, args: Value) -> String {
    let r = call(rpc, "tools/call", json!({"name": name, "arguments": args})).await;
    if let Some(e) = r.get("error") {
        return e["message"].as_str().unwrap_or_default().to_string();
    }
    assert_eq!(r["result"]["isError"], true, "{name} should have failed: {r}");
    r["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

fn names(r: &Value) -> Vec<String> {
    r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tools_sessions_files_and_resources() {
    let tmp = tempfile::tempdir().unwrap();
    let (_node, core) = bragi_mcp::cli::demo(&tmp.path().join("bragi")).await.unwrap();
    let root = tmp.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("notes.md"), "remember the milk\n").unwrap();
    std::fs::write(root.join("dot.png"), PNG).unwrap();
    std::fs::write(tmp.path().join("secret.txt"), "outside\n").unwrap();
    let rpc = rpc(&core, &root, false);

    // The handshake and the tool table: no delete tools without the flag.
    let r = call(&rpc, "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}})).await;
    assert_eq!(r["result"]["serverInfo"]["name"], "bragi");
    assert!(r["result"]["capabilities"]["resources"].is_object(), "{r}");
    let tools = names(&call(&rpc, "tools/list", json!({})).await);
    for t in
        ["bragi_status", "bragi_chat", "bragi_sessions_list", "bragi_session_move", "bragi_folder_create", "bragi_models_list", "bragi_file_fetch"]
    {
        assert!(tools.contains(&t.to_string()), "{t} missing from {tools:?}");
    }
    assert!(tools.contains(&"bragi_code_rewrite".to_string()) && tools.contains(&"bragi_code_suggest".to_string()));
    assert!(!tools.iter().any(|t| t.ends_with("_delete") || t.ends_with("_save") || t.ends_with("_apply") || t.ends_with("_undo")), "{tools:?}");
    assert!(fails(&rpc, "bragi_session_delete", json!({"session": "x"})).await.contains("bragi_session_delete"));
    assert!(fails(&rpc, "bragi_file_save", json!({"session": "x", "file": "y"})).await.contains("bragi_file_save"));

    // Status and models from the demo node.
    let s = ok(&rpc, "bragi_status", json!({})).await["structuredContent"].clone();
    assert_eq!(s["linked"], true, "{s}");
    assert_eq!(s["online"], true);
    let models = ok(&rpc, "bragi_models_list", json!({})).await["structuredContent"]["models"].clone();
    assert!(!models.as_array().unwrap().is_empty());

    // A chat with a text file and an image makes a session.
    let r = ok(&rpc, "bragi_chat", json!({"prompt": "hello there", "files": ["notes.md", "dot.png"]})).await;
    let answer = r["content"][0]["text"].as_str().unwrap().to_string();
    assert!(!answer.is_empty());
    let session = r["structuredContent"]["session"].as_str().unwrap().to_string();
    assert_eq!(r["structuredContent"]["text"], answer.as_str());
    // ...continued in the same session.
    let r = ok(&rpc, "bragi_chat", json!({"prompt": "and again", "session": session})).await;
    assert_eq!(r["structuredContent"]["session"], session.as_str());
    let got = ok(&rpc, "bragi_session_get", json!({"session": session})).await["structuredContent"].clone();
    assert_eq!(got["messages"].as_array().unwrap().len(), 4, "{got}");
    assert_eq!(got["messages"][0]["text"], "hello there");
    let last = ok(&rpc, "bragi_session_get", json!({"session": session, "last": 1})).await["structuredContent"].clone();
    assert_eq!(last["messages"].as_array().unwrap().len(), 1);

    // Files outside the project root are refused.
    assert!(fails(&rpc, "bragi_chat", json!({"prompt": "x", "files": ["../secret.txt"]})).await.contains("outside the project root"));

    // The attached files: text as text, the image as MCP image content.
    let files = ok(&rpc, "bragi_files_list", json!({"session": session})).await["structuredContent"]["files"].clone();
    let file_id = |name: &str| {
        files.as_array().unwrap().iter().find(|f| f["name"] == name).unwrap_or_else(|| panic!("{name} in {files}"))["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let (notes, dot) = (file_id("notes.md"), file_id("dot.png"));
    let r = ok(&rpc, "bragi_file_fetch", json!({"session": session, "file": notes})).await;
    assert_eq!(r["content"][0]["text"], "remember the milk\n");
    let r = ok(&rpc, "bragi_file_fetch", json!({"session": session, "file": dot})).await;
    assert_eq!(r["content"][0]["type"], "image");
    assert_eq!(r["content"][0]["mimeType"], "image/png");
    assert!(!fails(&rpc, "bragi_file_fetch", json!({"session": session, "file": "nope#a9"})).await.is_empty());

    // Folders: make two, nest one, file the session, list by folder.
    let a = ok(&rpc, "bragi_folder_create", json!({"name": "Work"})).await["structuredContent"]["folder"]["id"].as_str().unwrap().to_string();
    let b = ok(&rpc, "bragi_folder_create", json!({"name": "Later"})).await["structuredContent"]["folder"]["id"].as_str().unwrap().to_string();
    ok(&rpc, "bragi_folder_move", json!({"folder": b, "parent": a})).await;
    ok(&rpc, "bragi_session_move", json!({"session": session, "folder": b})).await;
    let folders = ok(&rpc, "bragi_folders_list", json!({})).await["structuredContent"]["folders"].clone();
    assert!(folders.as_array().unwrap().iter().any(|f| f["id"] == b.as_str() && f["parent"] == a.as_str()), "{folders}");
    let in_b = ok(&rpc, "bragi_sessions_list", json!({"folder": b})).await["structuredContent"]["sessions"].clone();
    assert_eq!(in_b.as_array().unwrap().len(), 1);
    assert_eq!(in_b[0]["id"], session.as_str());
    ok(&rpc, "bragi_session_move", json!({"session": session, "folder": null})).await;
    let in_b = ok(&rpc, "bragi_sessions_list", json!({"folder": b})).await["structuredContent"]["sessions"].clone();
    assert!(in_b.as_array().unwrap().is_empty());
    // A new chat can be filed as it is made.
    let r = ok(&rpc, "bragi_chat", json!({"prompt": "filed", "folder": a})).await;
    let filed = r["structuredContent"]["session"].as_str().unwrap().to_string();
    let in_a = ok(&rpc, "bragi_sessions_list", json!({"folder": a})).await["structuredContent"]["sessions"].clone();
    assert_eq!(in_a[0]["id"], filed.as_str());

    // Endpoints: the pool, and one from Settings (never its key).
    core.add_endpoint(
        EndpointInput {
            name: "Local".into(),
            base_url: "http://127.0.0.1:9/v1".into(),
            model: Some("m".into()),
            mode: EndpointMode::Alongside,
            enabled: true,
        },
        Some("sk-not-shown".into()),
    )
    .unwrap();
    let r = ok(&rpc, "bragi_endpoints_list", json!({})).await;
    let text = r.to_string();
    assert!(!text.contains("sk-not-shown"));
    assert_eq!(r["structuredContent"]["targets"][0]["target"]["kind"], "pool");
    assert_eq!(r["structuredContent"]["endpoints"][0]["name"], "Local");
    assert_eq!(r["structuredContent"]["endpoints"][0]["has_key"], true);
    assert!(fails(&rpc, "bragi_chat", json!({"prompt": "x", "endpoint": "nope"})).await.contains("no endpoint"));

    // Resources: status, the session list, each session as Markdown, its files.
    let r = call(&rpc, "resources/list", json!({})).await;
    let uris: Vec<&str> = r["result"]["resources"].as_array().unwrap().iter().map(|x| x["uri"].as_str().unwrap()).collect();
    assert!(uris.contains(&"bragi://status") && uris.contains(&"bragi://sessions"), "{uris:?}");
    let su = format!("bragi://sessions/{session}");
    assert!(uris.contains(&su.as_str()));
    let r = call(&rpc, "resources/templates/list", json!({})).await;
    assert_eq!(r["result"]["resourceTemplates"].as_array().unwrap().len(), 2, "{r}");
    let r = call(&rpc, "resources/read", json!({"uri": su})).await;
    let md = r["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(md.contains("hello there") && md.contains("(attached: dot.png (image))"), "{md}");
    let dot_uri = files.as_array().unwrap().iter().find(|f| f["name"] == "dot.png").unwrap()["uri"].as_str().unwrap().to_string();
    let r = call(&rpc, "resources/read", json!({"uri": dot_uri})).await;
    assert_eq!(r["result"]["contents"][0]["mimeType"], "image/png");
    assert!(r["result"]["contents"][0]["blob"].is_string());
    let r = call(&rpc, "resources/read", json!({"uri": "bragi://sessions/nope"})).await;
    assert!(r["error"].is_object(), "{r}");
    let r = call(&rpc, "resources/read", json!({"uri": "bragi://status"})).await;
    let status: Value = serde_json::from_str(r["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(status["linked"], true);

    // With --allow-writes: save a file into the project, never outside it, never over a file.
    let rpc = self::rpc(&core, &root, true);
    std::fs::create_dir_all(root.join("out")).unwrap();
    let saved = ok(&rpc, "bragi_file_save", json!({"session": session, "file": notes, "path": "out"})).await["structuredContent"]["saved"].clone();
    assert!(saved.as_str().unwrap().ends_with("notes.md"), "{saved}");
    assert_eq!(std::fs::read_to_string(root.join("out/notes.md")).unwrap(), "remember the milk\n");
    let again = ok(&rpc, "bragi_file_save", json!({"session": session, "file": notes, "path": "out"})).await["structuredContent"]["saved"].clone();
    assert!(again.as_str().unwrap().ends_with("notes (2).md"), "{again}");
    assert!(fails(&rpc, "bragi_file_save", json!({"session": session, "file": notes, "path": "out/notes.md"})).await.contains("overwrite"));
    ok(&rpc, "bragi_file_save", json!({"session": session, "file": notes, "path": "out/notes.md", "overwrite": true})).await;
    assert!(
        fails(&rpc, "bragi_file_save", json!({"session": session, "file": notes, "path": "../escaped.md"}))
            .await
            .contains("outside the project root")
    );
    assert!(!tmp.path().join("escaped.md").exists());

    // With --allow-deletes: delete a folder (its contents move up) and a session.
    let tools = names(&call(&rpc, "tools/list", json!({})).await);
    assert!(tools.contains(&"bragi_session_delete".to_string()) && tools.contains(&"bragi_folder_delete".to_string()));
    let r = call(&rpc, "tools/list", json!({})).await;
    let del = r["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "bragi_session_delete").unwrap().clone();
    assert_eq!(del["annotations"]["destructiveHint"], true, "{del}");
    ok(&rpc, "bragi_folder_delete", json!({"folder": a})).await;
    let folders = ok(&rpc, "bragi_folders_list", json!({})).await["structuredContent"]["folders"].clone();
    assert_eq!(folders.as_array().unwrap().len(), 1, "{folders}");
    ok(&rpc, "bragi_session_delete", json!({"session": filed})).await;
    assert!(!fails(&rpc, "bragi_session_get", json!({"session": filed})).await.is_empty());
    core.shutdown();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn loopback_http_needs_the_token() {
    let tmp = tempfile::tempdir().unwrap();
    let (_node, core) = bragi_mcp::cli::demo(&tmp.path().join("bragi")).await.unwrap();
    let token = bragi_core::local_api::load_or_create_token(&tmp.path().join("home")).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(bragi_mcp::serve_http(listener, rpc(&core, tmp.path(), false), token.clone(), async {
        let _ = stop_rx.await;
    }));
    let url = format!("http://127.0.0.1:{port}/mcp");
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}});
    let post = |tok: Option<&str>, body: &Value| {
        let mut r = http.post(&url).header("accept", "application/json, text/event-stream").json(body);
        if let Some(t) = tok {
            r = r.bearer_auth(t);
        }
        r.send()
    };
    assert_eq!(post(None, &init).await.unwrap().status(), 401);
    assert_eq!(post(Some("wrong"), &init).await.unwrap().status(), 401);
    let r = post(Some(&token), &init).await.unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["result"]["serverInfo"]["name"], "bragi");
    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "bragi_status", "arguments": {}}});
    let v: Value = post(Some(&token), &call).await.unwrap().json().await.unwrap();
    assert_eq!(v["result"]["structuredContent"]["linked"], true, "{v}");
    // A foreign Host or Origin is refused even with the token.
    let r = http.post(&url).header("host", "evil.example").bearer_auth(&token).json(&init).send().await.unwrap();
    assert_eq!(r.status(), 403);
    let r = http.post(&url).header("origin", "https://evil.example").bearer_auth(&token).json(&init).send().await.unwrap();
    assert_eq!(r.status(), 403);
    let _ = stop_tx.send(());
    task.await.unwrap().unwrap();
    core.shutdown();
}

const LIB: &str = "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn code_suggest_rewrite_apply_and_undo() {
    let tmp = tempfile::tempdir().unwrap();
    let (_node, core) = bragi_mcp::cli::demo(&tmp.path().join("bragi")).await.unwrap();
    let root = tmp.path().join("project");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), LIB).unwrap();
    std::fs::write(root.join(".env"), "TOKEN=x\n").unwrap();
    let ro = rpc(&core, &root, false);

    // A rewrite of a line range: a unified diff and a proposal, nothing written.
    let r = ok(&ro, "bragi_code_rewrite", json!({"target": "src/lib.rs:1-3", "instruction": "document it"})).await;
    let s = r["structuredContent"].clone();
    let diff = s["diff"].as_str().unwrap();
    assert!(diff.starts_with("--- a/src/lib.rs\n+++ b/src/lib.rs\n"), "{s}");
    assert!(diff.contains("+// ") && diff.contains("document it"), "{diff}");
    assert!(s["proposal"].is_string());
    assert!(r["content"][0]["text"].as_str().unwrap().contains("```diff"));
    assert_eq!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap(), LIB);
    // Code requests stay out of chat history.
    let sessions = ok(&ro, "bragi_sessions_list", json!({})).await["structuredContent"]["sessions"].clone();
    assert!(sessions.as_array().unwrap().is_empty(), "{sessions}");
    // Suggestions over a folder; secrets are never read.
    let s = ok(&ro, "bragi_code_suggest", json!({"paths": ["src"], "instruction": "tidy"})).await["structuredContent"].clone();
    assert_eq!(s["files"][0]["path"], "src/lib.rs", "{s}");
    let s = ok(&ro, "bragi_code_suggest", json!({})).await["structuredContent"].clone();
    assert!(!s["diff"].as_str().unwrap().contains(".env"), "{s}");
    // Outside the root, or a file that isn't there, is refused.
    assert!(fails(&ro, "bragi_code_rewrite", json!({"target": "../x.rs", "instruction": "x"})).await.len() > 3);
    assert!(fails(&ro, "bragi_code_suggest", json!({"paths": ["/etc"]})).await.contains("outside the project root"));
    // Applying needs --allow-writes.
    let proposal = ok(&ro, "bragi_code_rewrite", json!({"target": "src/lib.rs", "instruction": "x"})).await["structuredContent"]["proposal"].clone();
    assert!(fails(&ro, "bragi_code_apply", json!({"proposal": proposal})).await.contains("bragi_code_apply"));

    // With --allow-writes: apply, undo, and a stale proposal is refused.
    let rw = rpc(&core, &root, true);
    let p = ok(&rw, "bragi_code_rewrite", json!({"target": "src/lib.rs:2", "instruction": "explain"})).await["structuredContent"]["proposal"].clone();
    assert!(fails(&rw, "bragi_code_apply", json!({"proposal": p, "hunks": {"src/lib.rs": [9]}})).await.contains("hunks 1 to"));
    assert!(fails(&rw, "bragi_code_apply", json!({"proposal": p, "hunks": {"other.rs": [1]}})).await.contains("not in proposal"));
    let a = ok(&rw, "bragi_code_apply", json!({"proposal": p})).await["structuredContent"].clone();
    assert_eq!(a["files"][0], "src/lib.rs", "{a}");
    let now = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
    assert!(now.contains("explain") && now.contains("a + b"), "{now}");
    // The journal lives outside the project.
    assert!(!root.join("code-undo").exists() && tmp.path().join("undo").exists());
    let u = ok(&rw, "bragi_code_undo", json!({})).await["structuredContent"].clone();
    assert_eq!(u["undone"], a["applied"]);
    assert_eq!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap(), LIB);
    let p = ok(&rw, "bragi_code_rewrite", json!({"target": "src/lib.rs", "instruction": "y"})).await["structuredContent"]["proposal"].clone();
    std::fs::write(root.join("src/lib.rs"), "changed meanwhile\n").unwrap();
    assert!(fails(&rw, "bragi_code_apply", json!({"proposal": p})).await.contains("changed since"));
    assert!(fails(&rw, "bragi_code_apply", json!({"proposal": "p-nope"})).await.contains("no proposal"));
    core.shutdown();
}
