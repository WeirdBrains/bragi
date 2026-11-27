//! Video through `bragi serve` against the demo inference nodes, with `FakeSource` for the clip
//! (no ffmpeg): the limits routes and the override, a clip on disk attached to a code chat (the
//! `video` event the extension draws, kept with the message), and the refusals.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use bragi_core::demo::DemoNode;
use bragi_core::video::{FakeSource, FrameSource};
use bragi_core::{AppState, Core, Media, PairState};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const PORT: u16 = 4877;

async fn until(core: &Core, what: &str, f: impl Fn(&AppState) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut w = core.watch();
    loop {
        let s = w.borrow_and_update().clone();
        if f(&s) {
            return;
        }
        assert!(Instant::now() < deadline, "waiting for {what}: {s:?}");
        let _ = tokio::time::timeout(Duration::from_millis(200), w.changed()).await;
    }
}

async fn linked(dir: &Path) -> (Arc<DemoNode>, Core) {
    let demo = DemoNode::start_cloud(&dir.join("demo"), Duration::from_millis(2)).await.unwrap();
    let core = Core::start(demo.app_config(&dir.join("app"), "Bragi serve video test")).unwrap();
    demo.auto_accept(&core);
    core.pair_offer().await.unwrap();
    until(&core, "the code check", |s| matches!(s.pair, PairState::Confirm { .. })).await;
    core.pair_confirm(true).unwrap();
    until(&core, "the catalog", |s| matches!(s.pair, PairState::Linked { .. }) && s.catalog.default_model().is_some()).await;
    (demo, core)
}

async fn call(app: &Router, path: &str, body: Value) -> (StatusCode, String) {
    let r = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", format!("127.0.0.1:{PORT}"))
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = tokio::time::timeout(Duration::from_secs(60), app.clone().oneshot(r)).await.expect("answered").unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn json_of(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|e| panic!("{e}: {s}"))
}

fn events(s: &str) -> Vec<Value> {
    s.split("\n\n").filter_map(|e| e.strip_prefix("data: ")).map(json_of).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_clip_on_disk_goes_with_a_code_chat_sized_by_the_limits_and_override() {
    let d = tempfile::tempdir().unwrap();
    let (_demo, core) = linked(d.path()).await;
    let fake: bragi_serve::video::OpenClip = Arc::new(|_p: &Path| Ok(Box::new(FakeSource::new(42_000, false)) as Box<dyn FrameSource + Send>));
    let app = bragi_serve::router_with_clips(core.clone(), TOKEN.into(), PORT, fake);
    let clip = d.path().join("clip.mp4");
    std::fs::write(&clip, b"not decoded: the fake source answers").unwrap();
    let model = core.state().catalog.default_model().unwrap().to_string();

    // The demo pool reports a context but no image cost: not ready, and the chip says what to set.
    let (s, b) = call(&app, "/v1/video/limits", json!({})).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let v = json_of(&b);
    assert_eq!(v["model"], model.as_str());
    assert_eq!(v["ready"], false);
    assert!(v["chip"].as_str().unwrap().contains("needs image cost"), "{v}");
    assert!(v["reported"].as_str().unwrap().contains("context="), "{v}");
    assert!(v["keys"].as_array().unwrap().iter().any(|k| k["key"] == "image_tokens"));

    // So a clip is refused, saying what is missing, before anything streams.
    let chat = json!({ "prompt": "what happens?", "video": { "path": clip, "max_frames": 4 } });
    let (s, b) = call(&app, "/v1/code/chat", chat.clone()).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b.contains("image cost"), "{b}");

    // The override fills the gap; a looser context than reported is ignored and said so.
    let (s, b) = call(&app, "/v1/video/limits/set", json!({ "model": model, "limits": "image_tokens=256 max_images=6 context=99999999" })).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let v = json_of(&b);
    assert_eq!(v["ready"], true, "{v}");
    assert_eq!(v["ignored"], json!(["context"]), "{v}");
    assert!(v["user"].as_str().unwrap().contains("image_tokens=256"), "{v}");
    let (s, b) = call(&app, "/v1/video/limits/set", json!({ "model": model, "limits": "frames=3" })).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "an unknown key is refused: {b}");

    // Now the chat carries the clip: the `video` event (bubble, frames with times, where the clip
    // is) comes after `conversation`, and the message keeps the frames and the clip's path.
    let (s, b) = call(&app, "/v1/code/chat", chat).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let evs = events(&b);
    assert_eq!(evs[0]["type"], "conversation");
    assert_eq!(evs[1]["type"], "video", "{b}");
    let v = &evs[1];
    assert_eq!(v["name"], "clip.mp4");
    assert_eq!(v["clip"], clip.to_string_lossy().as_ref());
    assert_eq!(v["duration_ms"], 42_000);
    assert_eq!(v["sound"], false);
    assert_eq!(v["frames"].as_array().unwrap().len(), 4, "max_frames narrows the budget: {v}");
    assert_eq!(v["frames"][0]["label"], "[00:05.2]");
    assert_eq!(v["frames"][0]["mime"], "image/jpeg");
    assert!(!v["frames"][0]["data"].as_str().unwrap().is_empty());
    assert_eq!(v["bubble"], "video  clip.mp4  0:42  4 frames  no sound sent");
    assert_eq!(evs.last().unwrap()["type"], "done", "{b}");
    let id = evs[0]["id"].as_str().unwrap();
    let c = core.conversation(id).unwrap();
    match &c.messages[0].attachments[0].media {
        Some(Media::Video(s)) => {
            assert_eq!(s.frames.len(), 4);
            assert_eq!(s.clip.as_deref(), Some(clip.as_path()));
        }
        other => panic!("not a video: {other:?}"),
    }

    // Refusals: a relative path, a missing file, a range that runs backwards.
    for (video, want) in [
        (json!({ "path": "clip.mp4" }), "absolute"),
        (json!({ "path": d.path().join("gone.mp4") }), "isn't there"),
        (json!({ "path": clip, "start_ms": 5000, "end_ms": 1000 }), "after its start"),
    ] {
        let (s, b) = call(&app, "/v1/code/chat", json!({ "prompt": "?", "video": video })).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
        assert!(b.contains(want), "{want}: {b}");
    }

    // Clearing the override puts the chip back.
    let (_, b) = call(&app, "/v1/video/limits/set", json!({ "model": model, "limits": "" })).await;
    assert_eq!(json_of(&b)["ready"], false);
}
