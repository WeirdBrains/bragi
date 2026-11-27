//! The optional on-device model (B4) through `Core`: off by default, nothing downloaded without
//! consent, a consented download from a mock Hub (progress, cancel, resume), the "On this device"
//! target, chats and stops on a fake engine, and the fallback when no node is reachable. No real
//! weights are fetched and no real engine runs.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use bragi_core::demo_models::{MockRepo, mock_hub};
use bragi_core::{Attachment, ChatEvent, Core, ErrorKind, MessageState, Target};
use common::*;
use weirdnet_model::{Engine, GenerateRequest, ModelSpec, Stats, worker::StopCause};

const TINY: &str = "tiny-chat";

/// A tiny model the mock Hub serves (`size` bytes of GGUF header and padding).
fn tiny(size: u64) -> ModelSpec {
    ModelSpec {
        id: TINY.into(),
        label: "Tiny chat (test)".into(),
        repo: "test/tiny-chat-GGUF".into(),
        file: "tiny-chat.gguf".into(),
        size,
        sha256: None,
    }
}

fn hub(size: u64, chunk_delay: Duration) -> String {
    mock_hub(vec![MockRepo::new("test/tiny-chat-GGUF", 1, 1, &[("tiny-chat.gguf", size)]).downloadable()], chunk_delay)
}

/// Says "you said: <the last user message>" a word at a time, `delay` apart.
struct Parrot {
    delay: Duration,
}

impl Engine for Parrot {
    fn generate(&mut self, req: &GenerateRequest, on_text: &mut dyn FnMut(&str) -> bool) -> weirdnet_model::Result<Stats> {
        let last = req.messages.last().map(|m| m.content.clone()).unwrap_or_default();
        let mut n = 0;
        for w in format!("you said: {last}").split_inclusive(' ') {
            std::thread::sleep(self.delay);
            if !on_text(w) {
                return Ok(Stats { prompt_tokens: req.messages.len() as u64, generated_tokens: n, stop: StopCause::Cancelled });
            }
            n += 1;
        }
        Ok(Stats { prompt_tokens: req.messages.len() as u64, generated_tokens: n, stop: StopCause::End })
    }
}

fn fake_engine(core: &Core, delay: Duration) -> Arc<AtomicUsize> {
    let loads = Arc::new(AtomicUsize::new(0));
    let l = loads.clone();
    core.set_on_device_engine(
        "fake engine",
        Arc::new(move |path| {
            assert!(path.exists(), "loads the downloaded file");
            l.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Parrot { delay }) as Box<dyn Engine>)
        }),
    );
    loads
}

async fn downloaded(core: &Core) {
    until(core, "the download", |s| s.on_device.download.is_none() && s.on_device.installed).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn off_by_default_and_nothing_downloads_without_consent() {
    let tmp = tempfile::tempdir().unwrap();
    let core = Core::start(bragi_core::Config::new(tmp.path().join("app"), "t")).unwrap();

    // Off, with the real catalog offered and nothing on disk.
    let st = core.on_device_status();
    assert!(!st.enabled && !st.ready() && st.model.is_none(), "{st:?}");
    assert!(st.line().starts_with("Off."));
    assert!(core.on_device_models().iter().any(|m| m.id == "gemma-4-e2b" && !m.installed && m.size_label == "2.84 GB"));
    assert!(!core.targets().iter().any(|t| matches!(t.target, Target::Device { .. })));

    core.set_on_device_source(&hub(4096, Duration::ZERO), vec![tiny(4096)]);
    let e = core.download_on_device_model(TINY, false).unwrap_err();
    assert_eq!(e.kind, ErrorKind::BadRequest);
    assert!(!tmp.path().join("app/models").join("tiny-chat.gguf").exists());
    assert!(core.download_on_device_model("nope", true).is_err());

    // On picks a model but downloads nothing; the picker still leaves it out.
    let st = core.set_on_device(true).unwrap();
    assert_eq!(st.model.as_deref(), Some(TINY));
    assert!(!st.installed);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(core.state().on_device.download.is_none());
    assert!(!core.targets().iter().any(|t| matches!(t.target, Target::Device { .. })));
    let e = core.send_to(None, "hi", None, Vec::new(), Some(Target::Device { model: TINY.into() })).unwrap_err();
    assert!(e.message.contains("download") || e.message.contains("engine"), "{e:?}");
    // The setting is kept per device.
    assert!(core.on_device_settings().enabled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_consented_download_then_chats_on_this_device() {
    let tmp = tempfile::tempdir().unwrap();
    let core = Core::start(bragi_core::Config::new(tmp.path().join("app"), "t")).unwrap();
    let url = hub(200_000, Duration::ZERO);
    core.set_on_device_source(&url, vec![tiny(200_000)]);
    let loads = fake_engine(&core, Duration::ZERO);

    let consent = core.on_device_consent(TINY).unwrap();
    assert!(consent[0].contains("Tiny chat (test)") && consent[0].contains("200000 bytes"), "{consent:?}");
    assert!(consent.iter().any(|l| l.contains(&url)), "{consent:?}");
    assert!(consent.iter().any(|l| l.contains("models")), "{consent:?}");

    core.download_on_device_model(TINY, true).unwrap();
    downloaded(&core).await;
    // Downloading chose it; it isn't on until the user turns it on.
    assert_eq!(core.on_device_settings().model.as_deref(), Some(TINY));
    assert!(!core.targets().iter().any(|t| matches!(t.target, Target::Device { .. })));
    core.set_on_device(true).unwrap();
    let st = core.state().on_device;
    assert!(st.ready(), "{st:?}");
    assert_eq!(st.line(), "Tiny chat (test), ready (fake engine).");
    let opt = core.targets().into_iter().find(|t| matches!(t.target, Target::Device { .. })).unwrap();
    assert_eq!(opt.label, "On this device · Tiny chat (test)");

    // With no node reachable, a new chat goes to this device and stays bound to it.
    let mut h = core.send_to(None, "hello there", None, Vec::new(), None).unwrap();
    let conv = h.conversation_id.clone();
    let events = drain(&mut h).await;
    assert!(matches!(events.last(), Some(ChatEvent::Done { finish_reason: Some(f) }) if f == "stop"), "{events:?}");
    assert!(events.iter().any(|e| matches!(e, ChatEvent::Started { node, .. } if node == "On this device")), "{events:?}");
    assert_eq!(answer_text(&events), "you said: hello there");
    let c = core.conversation(&conv).unwrap();
    assert_eq!((c.messages[1].text.as_str(), c.messages[1].state), ("you said: hello there", MessageState::Complete));
    assert_eq!(c.messages[1].node.as_deref(), Some("On this device"));
    assert_eq!(c.messages[1].model.as_deref(), Some("Tiny chat (test)"));
    let s = core.sessions(0, 10).unwrap();
    assert_eq!(s[0].target, Some(Target::Device { model: TINY.into() }));
    assert_eq!(s[0].target_label, "On this device · Tiny chat (test)");

    // The next turn reuses the loaded model and sends the history along.
    let mut h = core.send_to(Some(&conv), "again", None, Vec::new(), None).unwrap();
    assert_eq!(answer_text(&drain(&mut h).await), "you said: again");
    assert_eq!(loads.load(Ordering::SeqCst), 1);
    assert_eq!(core.conversation(&conv).unwrap().messages.len(), 4);

    // A one-off completion (code mode) runs here too and saves nothing.
    let mut h = core.complete(vec![bragi_core::code::PromptMessage::user("fn main")], None).unwrap();
    assert_eq!(answer_text(&drain(&mut h).await), "you said: fn main");
    assert!(h.conversation_id.is_empty());

    // Pictures, sound and video stay with nodes.
    let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, b'I', b'H', b'D', b'R', 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89];
    let img = Attachment::from_bytes("shot.png", &png).unwrap();
    let e = core.send_to(Some(&conv), "what is this", None, vec![img], None).unwrap_err();
    assert!(e.message.contains("text only"), "{e:?}");

    // Off: gone from the picker, and the bound chat says why it can't send.
    core.set_on_device(false).unwrap();
    assert!(!core.targets().iter().any(|t| matches!(t.target, Target::Device { .. })));
    let e = core.send_to(Some(&conv), "still there?", None, Vec::new(), None).unwrap_err();
    assert!(e.message.contains("turn it on"), "{e:?}");

    // Removing deletes the file.
    assert!(core.remove_on_device_model(TINY).unwrap());
    assert!(!tmp.path().join("app/models/tiny-chat.gguf").exists());
    assert!(!core.on_device_models()[0].installed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancelled_download_keeps_what_came_and_resumes() {
    let tmp = tempfile::tempdir().unwrap();
    let core = Core::start(bragi_core::Config::new(tmp.path().join("app"), "t")).unwrap();
    let size = 64 * 1024 * 12;
    core.set_on_device_source(&hub(size, Duration::from_millis(60)), vec![tiny(size)]);

    core.download_on_device_model(TINY, true).unwrap();
    assert_eq!(core.download_on_device_model(TINY, true).unwrap_err().kind, ErrorKind::Busy);
    until(&core, "some bytes", |s| s.on_device.download.as_ref().is_some_and(|d| d.done > 0 && d.total == size)).await;
    assert!(core.state().on_device.line().starts_with("Downloading tiny-chat"));
    assert!(core.cancel_on_device_download());
    until(&core, "the stop", |s| s.on_device.download.is_none()).await;
    let m = &core.on_device_models()[0];
    assert!(!m.installed && m.partial > 0 && m.partial < size, "{m:?}");
    assert!(core.state().on_device.error.is_none());
    assert!(!core.cancel_on_device_download());

    core.download_on_device_model(TINY, true).unwrap();
    downloaded(&core).await;
    let m = &core.on_device_models()[0];
    assert!(m.installed && m.partial == 0, "{m:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_download_says_so_and_stop_ends_an_answer() {
    let tmp = tempfile::tempdir().unwrap();
    let core = Core::start(bragi_core::Config::new(tmp.path().join("app"), "t")).unwrap();
    // The Hub doesn't have this one.
    let mut missing = tiny(1000);
    (missing.id, missing.repo) = ("missing".into(), "test/elsewhere".into());
    core.set_on_device_source(&hub(4096, Duration::ZERO), vec![missing, tiny(4096)]);
    core.download_on_device_model("missing", true).unwrap();
    until(&core, "the failure", |s| s.on_device.error.is_some() && s.on_device.download.is_none()).await;
    let st = core.state().on_device;
    assert!(st.line().contains("failed") && st.line().contains("404"), "{st:?}");

    // A slow answer stopped part way keeps what came.
    core.set_on_device_source(&hub(4096, Duration::ZERO), vec![tiny(4096)]);
    core.download_on_device_model(TINY, true).unwrap();
    downloaded(&core).await;
    assert!(core.state().on_device.error.is_none(), "a new try clears the error");
    fake_engine(&core, Duration::from_millis(150));
    core.set_on_device(true).unwrap();
    let mut h = core.send_to(None, "one two three four five six", None, Vec::new(), Some(Target::Device { model: TINY.into() })).unwrap();
    let mut first = None;
    while let Some(ev) = h.events.recv().await {
        if matches!(ev, ChatEvent::Delta { .. }) {
            first = Some(ev);
            break;
        }
    }
    assert!(first.is_some());
    assert!(core.cancel(&h.request_id));
    let events = drain(&mut h).await;
    assert!(matches!(events.last(), Some(ChatEvent::Error { kind: ErrorKind::Cancelled, .. })), "{events:?}");
    let c = core.conversation(&h.conversation_id).unwrap();
    assert_eq!(c.messages[1].state, MessageState::Stopped);
    assert!(c.messages[1].text.starts_with("you said:") && !c.messages[1].text.contains("six"), "{:?}", c.messages[1].text);
}
