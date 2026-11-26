//! The LF-11 part of the desktop scenario: Settings (add, edit, test, reorder, delete with the
//! rebind choice), a chat bound to an endpoint, links, folders and sessions. The endpoint is a
//! tiny OpenAI-compatible server on a loopback port; nothing leaves the machine.

use std::io::{Read, Write};
use std::net::TcpListener;

use bragi_core::{EndpointMode, Target};

use super::*;

/// A loopback server answering `/v1/models` and a streamed `/v1/chat/completions`.
fn mock_openai() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut s = stream;
            // Read the whole request (head and body) so closing never resets the client.
            let mut raw = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = s.read(&mut buf).unwrap_or(0);
                raw.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&raw).to_string();
                let Some(end) = text.find("\r\n\r\n") else {
                    if n == 0 {
                        break;
                    }
                    continue;
                };
                let want = text[..end]
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
                    .unwrap_or(0);
                if raw.len() >= end + 4 + want || n == 0 {
                    break;
                }
            }
            let head = String::from_utf8_lossy(&raw).to_string();
            let path = head.split_whitespace().nth(1).unwrap_or_default().to_string();
            let reply = if path.ends_with("/models") {
                let b = r#"{"object":"list","data":[{"id":"mock-1","object":"model"},{"id":"mock-2","object":"model"}]}"#;
                format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{b}", b.len())
            } else {
                let chunk = |t: &str| format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{t}\"}},\"finish_reason\":null}}]}}\n\n");
                let stop = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
                let body = format!("{}{}{}{stop}data: [DONE]\n\n", chunk("Hello "), chunk("from the "), chunk("mock."));
                format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
            };
            let _ = s.write_all(reply.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}/v1")
}

fn e<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

pub(super) async fn run(this: &WeakEntity<BragiView>, cx: &mut gpui::AsyncApp, pause: Duration) -> Result<(), String> {
    let shot = async |cx: &mut gpui::AsyncApp, name: &str| {
        println!("bragi-scenario: screenshot {name}");
        cx.background_executor().timer(pause).await;
    };
    let set = |cx: &mut gpui::AsyncApp, name: &str, url: &str, key: &str, model: &str| {
        let (name, url, key, model) = (name.to_string(), url.to_string(), key.to_string(), model.to_string());
        this.update(cx, move |v, cx| {
            let f = &v.fields;
            f.name.update(cx, |i, cx| i.set_text(&name, cx));
            f.url.update(cx, |i, cx| i.set_text(&url, cx));
            f.key.update(cx, |i, cx| i.set_text(&key, cx));
            f.model.update(cx, |i, cx| i.set_text(&model, cx));
        })
        .map_err(|e| e.to_string())
    };
    let base = mock_openai();

    // Add an endpoint with a key, test it, save it.
    this.update(cx, |v, cx| {
        v.toggle_settings(cx);
        v.endpoint_form(None, cx);
        v.form_mode(EndpointMode::Alongside, cx);
    })
    .map_err(e)?;
    set(cx, "Local mock", &base, "sk-demo-key", "mock-1")?;
    this.update(cx, |v, cx| v.endpoint_test(None, cx)).map_err(e)?;
    wait_for(this, cx, "the test result", |v| v.prefs.tests.get("form").is_some_and(|t| t.1 != "Testing…")).await?;
    if !this.read_with(cx, |v, _| v.prefs.tests["form"].0).map_err(e)? {
        return Err("the endpoint test failed".into());
    }
    shot(cx, "21-settings-add-endpoint").await;
    this.update(cx, |v, cx| v.form_save(cx)).map_err(e)?;
    // A second, fallback-only one; a bad address is refused with words.
    this.update(cx, |v, cx| v.endpoint_form(None, cx)).map_err(e)?;
    set(cx, "Backup", "nope", "", "")?;
    this.update(cx, |v, cx| v.form_save(cx)).map_err(e)?;
    if this.read_with(cx, |v, _| v.prefs.form_error.is_none()).map_err(e)? {
        return Err("a bad address was accepted".into());
    }
    set(cx, "Backup", &base, "", "")?;
    this.update(cx, |v, cx| v.form_save(cx)).map_err(e)?;
    let names = |v: &BragiView| v.prefs.endpoints.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
    if this.read_with(cx, |v, _| names(v)).map_err(e)? != ["Local mock", "Backup"] {
        return Err("endpoints missing after add".into());
    }
    // Edit keeps the key when the field is blank; reorder moves Backup first.
    let first = this.read_with(cx, |v, _| v.prefs.endpoints[0].id.clone()).map_err(e)?;
    let second = this.read_with(cx, |v, _| v.prefs.endpoints[1].id.clone()).map_err(e)?;
    this.update(cx, |v, cx| v.endpoint_form(Some(&first), cx)).map_err(e)?;
    set(cx, "Local mock (edited)", &base, "", "mock-1")?;
    this.update(cx, |v, cx| v.form_save(cx)).map_err(e)?;
    if !this.read_with(cx, |v, _| v.prefs.endpoints[0].has_key && v.prefs.endpoints[0].name == "Local mock (edited)").map_err(e)? {
        return Err("edit lost the key or the name".into());
    }
    this.update(cx, |v, cx| v.endpoint_move(&second, -1, cx)).map_err(e)?;
    if this.read_with(cx, |v, _| v.prefs.endpoints[0].id.clone()).map_err(e)? != second {
        return Err("reorder did not move the endpoint".into());
    }
    this.update(cx, |v, cx| v.endpoint_test(Some(first.clone()), cx)).map_err(e)?;
    wait_for(this, cx, "the row test", |v| v.prefs.tests.get(&first).is_some_and(|t| t.1 != "Testing…")).await?;
    shot(cx, "22-settings-endpoints").await;

    // Links: add, edit, delete (a made-up address; it is never reachable).
    let a1 = "/ip4/192.0.2.7/tcp/4848/p2p/12D3KooWQH96LGJd33JquZXNyrmrs7HAeaeviiFqdwnzwfdmj1VY";
    let a2 = "/ip4/192.0.2.8/tcp/4848/p2p/12D3KooWQH96LGJd33JquZXNyrmrs7HAeaeviiFqdwnzwfdmj1VY";
    let before = this.read_with(cx, |v, _| v.prefs.links.len()).map_err(e)?;
    this.update(cx, |v, cx| v.link_form(None, cx)).map_err(e)?;
    set(cx, "", a1, "", "")?;
    this.update(cx, |v, cx| v.form_save(cx)).map_err(e)?;
    this.update(cx, |v, cx| v.link_form(Some(a1), cx)).map_err(e)?;
    set(cx, "", a2, "", "")?;
    this.update(cx, |v, cx| v.form_save(cx)).map_err(e)?;
    if !this.read_with(cx, |v, _| v.prefs.links.iter().any(|l| l.address == a2) && !v.prefs.links.iter().any(|l| l.address == a1)).map_err(e)? {
        return Err("link edit did not replace the address".into());
    }
    shot(cx, "23-settings-links").await;
    this.update(cx, |v, cx| v.ask_delete_link(a2, cx)).map_err(e)?;
    shot(cx, "24-delete-link-confirm").await;
    this.update(cx, |v, cx| v.confirm_delete_link(cx)).map_err(e)?;
    if this.read_with(cx, |v, _| v.prefs.links.len()).map_err(e)? != before {
        return Err("link delete left it in the list".into());
    }

    // A chat bound to the endpoint answers from it and keeps the label.
    this.update(cx, |v, cx| {
        v.toggle_settings(cx);
        if v.agent {
            v.toggle_agent(cx);
        }
        v.new_chat(cx);
    })
    .map_err(e)?;
    let target = this
        .read_with(cx, |v, _| v.prefs.targets.iter().find(|o| matches!(&o.target, Target::Endpoint { .. })).map(|o| o.target.clone()))
        .map_err(e)?
        .ok_or("the endpoint is not offered as a target")?;
    this.update(cx, |v, cx| {
        v.pick_target(target.clone(), cx);
        v.set_prompt("hello endpoint", cx)
    })
    .map_err(e)?;
    press_send(this, cx)?;
    wait_for(this, cx, "the endpoint's answer", |v| v.streaming.is_none() && last_assistant(v).is_some_and(|m| m.state == MessageState::Complete))
        .await?;
    if this.read_with(cx, |v, _| last_assistant(v).map(|m| m.text)).map_err(e)?.as_deref() != Some("Hello from the mock.") {
        return Err("the endpoint's answer is wrong".into());
    }
    let conv = this.read_with(cx, |v, _| v.current.as_ref().map(|c| c.id.clone())).map_err(e)?.ok_or("no chat")?;
    shot(cx, "25-chat-on-endpoint").await;
    this.update(cx, |v, cx| {
        v.refresh_prefs();
        v.toggle_target_menu(cx)
    })
    .map_err(e)?;
    shot(cx, "26-target-picker").await;
    this.update(cx, |v, cx| v.toggle_target_menu(cx)).map_err(e)?;

    // Folders: nest one inside another, file the chat in the inner one.
    this.update(cx, |v, cx| v.folder_form(None, None, cx)).map_err(e)?;
    set(cx, "Work", "", "", "")?;
    this.update(cx, |v, cx| v.form_save(cx)).map_err(e)?;
    let work = this.read_with(cx, |v, _| v.prefs.folders[0].id.clone()).map_err(e)?;
    this.update(cx, |v, cx| v.folder_form(None, Some(&work), cx)).map_err(e)?;
    set(cx, "Reports", "", "", "")?;
    this.update(cx, |v, cx| v.form_save(cx)).map_err(e)?;
    let reports = this
        .read_with(cx, |v, _| v.prefs.folders.iter().find(|f| f.name == "Reports").map(|f| (f.id.clone(), f.parent.clone())))
        .map_err(e)?
        .ok_or("no subfolder")?;
    if reports.1.as_deref() != Some(work.as_str()) {
        return Err("the subfolder is not nested".into());
    }
    this.update(cx, |v, cx| {
        v.ask_move(&conv, cx);
    })
    .map_err(e)?;
    shot(cx, "27-move-to-folder").await;
    this.update(cx, |v, cx| v.move_to(Some(reports.0.clone()), cx)).map_err(e)?;
    if this.read_with(cx, |v, _| v.prefs.sessions.iter().find(|s| s.id == conv).and_then(|s| s.folder.clone())).map_err(e)?.as_deref()
        != Some(reports.0.as_str())
    {
        return Err("the chat is not in the folder".into());
    }
    shot(cx, "28-sessions-folders").await;

    // Delete the endpoint: the confirm asks where its chat goes; leaving it unbound keeps history.
    this.update(cx, |v, cx| {
        v.toggle_settings(cx);
        v.ask_delete_endpoint(&first, cx)
    })
    .map_err(e)?;
    if this.read_with(cx, |v, _| v.prefs.delete_endpoint.as_ref().map(|d| d.chats)).map_err(e)? != Some(1) {
        return Err("the confirm does not count the bound chat".into());
    }
    shot(cx, "29-delete-endpoint-confirm").await;
    this.update(cx, |v, cx| v.pick_rebind(None, cx)).map_err(e)?;
    this.update(cx, |v, cx| v.confirm_delete_endpoint(cx)).map_err(e)?;
    if !this.read_with(cx, |v, _| v.prefs.sessions.iter().any(|s| s.id == conv && s.unbound) && v.prefs.endpoints.len() == 1).map_err(e)? {
        return Err("the chat was not marked unbound".into());
    }
    this.update(cx, |v, cx| {
        v.toggle_settings(cx);
        v.open(&conv, cx)
    })
    .map_err(e)?;
    if this.read_with(cx, |v, _| v.current.as_ref().map(|c| c.messages.len())).map_err(e)? != Some(2) {
        return Err("history was not kept".into());
    }
    shot(cx, "30-unbound-chat").await;

    // Wake-ups (B4): push on and relay wake off by default; relay wake asks first with its
    // battery cost, Cancel keeps it off, and push off turns it off with it.
    let wake =
        |cx: &mut gpui::AsyncApp| this.read_with(cx, |v, _| (v.state.wake.settings.push, v.state.wake.settings.relay_wake, v.prefs.wake_confirm));
    if wake(cx).map_err(e)? != (true, false, false) {
        return Err("wake-ups are not push on, relay wake off by default".into());
    }
    this.update(cx, |v, cx| {
        v.toggle_settings(cx);
        v.toggle_relay_wake(cx)
    })
    .map_err(e)?;
    if wake(cx).map_err(e)? != (true, false, true) {
        return Err("relay wake did not ask first".into());
    }
    shot(cx, "30a-relay-wake-confirm").await;
    this.update(cx, |v, cx| {
        v.prefs.wake_confirm = false;
        cx.notify()
    })
    .map_err(e)?;
    if wake(cx).map_err(e)? != (true, false, false) {
        return Err("Cancel turned relay wake on".into());
    }
    this.update(cx, |v, cx| {
        v.toggle_relay_wake(cx);
        v.confirm_relay_wake(cx)
    })
    .map_err(e)?;
    if wake(cx).map_err(e)? != (true, true, false) {
        return Err("relay wake did not turn on".into());
    }
    this.update(cx, |v, cx| v.toggle_wake_push(cx)).map_err(e)?;
    if this.read_with(cx, |v, _| v.core.wake_status().settings.push).map_err(e)? {
        return Err("push did not turn off".into());
    }
    this.update(cx, |v, cx| {
        v.set_wake(bragi_core::wake::WakeSettings::default(), cx);
        v.toggle_settings(cx)
    })
    .map_err(e)?;
    println!("bragi-scenario: endpoints, links, folders and wake-ups");
    Ok(())
}
