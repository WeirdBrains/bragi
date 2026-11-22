//! The `bragi-mcp` binary over stdio against its demo node: initialize, list tools, chat. Runs
//! with an empty environment and a throwaway HOME, so it can't read or touch a real profile.

use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stdio_smoke() {
    let home = tempfile::tempdir().unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_bragi-mcp"))
        .args(["--demo", "--root"])
        .arg(home.path())
        .env_clear()
        .env("HOME", home.path())
        .env("TMPDIR", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    let msgs = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "smoke", "version": "0"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "bragi_chat", "arguments": {"prompt": "hi"}}}),
    ];
    for m in &msgs {
        stdin.write_all(format!("{m}\n").as_bytes()).await.unwrap();
    }
    stdin.flush().await.unwrap();
    let mut replies = std::collections::HashMap::new();
    while replies.len() < 3 {
        let line = tokio::time::timeout(Duration::from_secs(60), out.next_line()).await.expect("a reply in time").unwrap().expect("a line");
        let v: Value = serde_json::from_str(&line).unwrap();
        replies.insert(v["id"].as_i64().unwrap(), v);
    }
    assert_eq!(replies[&1]["result"]["serverInfo"]["name"], "bragi");
    assert!(replies[&2]["result"]["tools"].as_array().unwrap().iter().any(|t| t["name"] == "bragi_chat"));
    assert_eq!(replies[&3]["result"]["isError"], false, "{}", replies[&3]);
    assert!(!replies[&3]["result"]["content"][0]["text"].as_str().unwrap().is_empty());
    // End of input: the server answers what is left and exits.
    drop(stdin);
    let status = tokio::time::timeout(Duration::from_secs(30), child.wait()).await.expect("exits").unwrap();
    assert!(status.success());
}
