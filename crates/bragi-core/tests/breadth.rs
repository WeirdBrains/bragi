//! B5: attachments and agent mode, against the demo cloud (memory transport, test identities).

use std::sync::Arc;
use std::time::{Duration, Instant};

use bragi_core::demo::{DemoNode, WARM_MODEL};
use bragi_core::{AppState, Attachment, ChatEvent, ChatHandle, Core, ErrorKind, MessageState, PairState};

async fn until(core: &Core, what: &str, f: impl Fn(&AppState) -> bool) -> AppState {
    let deadline = Instant::now() + Duration::from_secs(30);
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

/// An app linked to the first of two inference nodes, both in its catalog.
async fn cloud(dir: &std::path::Path) -> (Arc<DemoNode>, Core) {
    let demo = DemoNode::start_cloud(&dir.join("demo"), Duration::from_millis(8)).await.unwrap();
    let core = Core::start(demo.app_config(&dir.join("app"), "Bragi test")).unwrap();
    demo.auto_accept(&core);
    core.pair_offer().await.unwrap();
    until(&core, "the code check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    core.pair_confirm(true).unwrap();
    until(&core, "both nodes' catalogs", |s| matches!(s.pair, PairState::Linked { .. }) && s.catalog.nodes.len() == 2).await;
    (demo, core)
}

struct Answer {
    text: String,
    events: Vec<ChatEvent>,
}

async fn collect(mut h: ChatHandle) -> Answer {
    let mut a = Answer { text: String::new(), events: Vec::new() };
    while let Some(ev) = tokio::time::timeout(Duration::from_secs(30), h.events.recv()).await.expect("stream stalled") {
        if let ChatEvent::Delta { offset, text } = &ev {
            let keep: String = a.text.chars().take(*offset as usize).collect();
            assert_eq!(keep.chars().count() as u64, *offset, "a delta past the end of the text");
            a.text = keep + text;
        }
        let end = matches!(ev, ChatEvent::Done { .. } | ChatEvent::Error { .. });
        a.events.push(ev);
        if end {
            break;
        }
    }
    a
}

fn done(a: &Answer) {
    assert!(matches!(a.events.last(), Some(ChatEvent::Done { .. })), "{:?}", a.events.last());
}

async fn ask_with(core: &Core, prompt: &str, files: Vec<Attachment>) -> Answer {
    collect(core.send_with(None, prompt, Some(WARM_MODEL.to_string()), files).unwrap()).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_attached_text_file_reaches_the_model_and_stays_with_the_message() {
    let tmp = tempfile::tempdir().unwrap();
    let (_demo, core) = cloud(tmp.path()).await;
    let path = tmp.path().join("notes.md");
    std::fs::write(&path, "# Notes\nbuy milk\nwrite B5\n").unwrap();
    let file = Attachment::from_path(&path).unwrap();
    assert_eq!(file.name, "notes.md");

    let a = ask_with(&core, "What is on my list?", vec![file.clone()]).await;
    done(&a);
    assert!(a.text.contains("I read notes.md (3 lines)"), "{}", a.text);
    assert!(a.text.contains("What is on my list?"), "{}", a.text);

    // The conversation keeps the file with the user's message (and the typed text stays just the prompt).
    let id = core.conversations(0, 10).unwrap()[0].id.clone();
    let c = core.conversation(&id).unwrap();
    assert_eq!(c.messages[0].text, "What is on my list?");
    assert_eq!(c.messages[0].attachments, vec![file.clone()]);
    assert_eq!(c.messages[1].state, MessageState::Complete);

    // A follow-up in the same conversation still has the file in the context the model gets.
    let h = core.send_with(Some(&id), "And the second item?", Some(WARM_MODEL.into()), vec![]).unwrap();
    done(&collect(h).await);
    let c = core.conversation(&id).unwrap();
    assert_eq!(c.messages.len(), 4);
    assert_eq!(c.messages[0].attachments.len(), 1, "the earlier turn kept its file");

    // Refusals are clear and typed.
    let e = core.send_with(None, "x", Some(WARM_MODEL.into()), vec![file.clone(); 6]).err().expect("too many");
    assert_eq!(e.kind, ErrorKind::BadRequest);
    assert!(Attachment::from_bytes("pic.png", &[0x89, 0x50, 0, 0]).unwrap_err().message.contains("not a text file"));
    core.shutdown();
}
