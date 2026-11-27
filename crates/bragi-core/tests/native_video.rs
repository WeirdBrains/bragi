//! Native video (V4) through `Core`: a clip goes whole (`video_url`) to a model that reads video
//! when it fits the reported limits, through the pool (infer/2, the demo node) and an
//! OpenAI-compatible endpoint (the mock), and falls back to the frames when a limit is missing
//! or exceeded, the clip is gone, or the server turns it down (the frames hint, a 413).

mod common;

use std::path::{Path, PathBuf};

use bragi_core::demo::{COLD_MODEL, WARM_MODEL};
use bragi_core::video::{FakeSource, ModelLimits, VideoPrefs};
use bragi_core::{Attachment, ChatEvent, Core, EndpointInput, EndpointMode, Media, Target};
use common::*;

/// A clip file of `bytes` bytes (the content isn't decoded: FakeSource stands in for ffmpeg).
fn clip(dir: &Path, name: &str, bytes: usize) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, vec![7u8; bytes]).unwrap();
    p
}

/// Samples a 10 s clip at `path` for `model` at `target` and decides how it goes, as the shells do.
async fn attach(core: &Core, target: Option<&Target>, model: &str, path: &Path, prefs: &VideoPrefs) -> Attachment {
    let plan = core.video_plan(target, model).await.unwrap();
    let mut a = core.attach_video_source("clip.mp4", &FakeSource::new(10_000, false), model, &plan, prefs, 0).unwrap();
    core.finish_video_attach(&mut a, &plan, prefs, path).unwrap();
    a
}

fn frames(a: &Attachment) -> usize {
    match &a.media {
        Some(Media::Video(v)) => v.frames.len(),
        _ => 0,
    }
}

fn whole(a: &Attachment) -> bool {
    matches!(&a.media, Some(Media::Video(v)) if v.sends_whole())
}

fn retries(events: &[ChatEvent]) -> Vec<String> {
    events.iter().filter_map(|e| if let ChatEvent::Retry { message, .. } = e { Some(message.clone()) } else { None }).collect()
}

/// The newest user message's clip in `conversation` still says it went whole.
fn saved_whole(core: &Core, conversation: &str) -> bool {
    let c = core.conversation(conversation).unwrap();
    c.messages.iter().rev().find(|m| !m.attachments.is_empty()).is_some_and(|m| m.attachments.iter().any(whole))
}

fn pool_limits(extra: &str) -> ModelLimits {
    ModelLimits::parse(&format!("image_tokens=256 max_images=8 {extra}")).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn through_the_pool_a_clip_goes_whole_when_it_fits_and_as_frames_when_a_limit_is_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, _demo) = linked(tmp.path()).await;
    let prefs = VideoPrefs::default();
    let path = clip(tmp.path(), "clip.mp4", 3000);
    assert!(core.pool_model_reads_video(WARM_MODEL) && !core.pool_model_reads_video(COLD_MODEL));

    // The model reads video, but nobody reports its video limits: frames, and the chip says why.
    core.set_video_override(WARM_MODEL, pool_limits("")).unwrap();
    let a = attach(&core, None, WARM_MODEL, &path, &prefs).await;
    assert!(!whole(&a) && a.label().contains("frames"), "{}", a.label());
    let chip = core.video_limits_view(None, WARM_MODEL).await.unwrap().chip();
    assert!(chip.contains("video_seconds and video_bytes aren't reported"), "{chip}");

    // With the user's video limits (filling what the pool doesn't say): whole.
    core.set_video_override(WARM_MODEL, pool_limits("video_seconds=60 video_bytes=100000")).unwrap();
    let a = attach(&core, None, WARM_MODEL, &path, &prefs).await;
    assert!(whole(&a), "{}", a.label());
    let n = frames(&a);
    assert!(n > 1, "the frames are still sampled for the player and the fallback");
    assert_eq!(a.label(), "clip.mp4 (video, whole clip (3 KiB))");
    assert!(core.video_limits_view(None, WARM_MODEL).await.unwrap().chip().contains("whole clips up to 60 s"));
    let mut h = core.send_to(None, "what happens?", Some(WARM_MODEL.into()), vec![a], Some(Target::Pool)).unwrap();
    let conv = h.conversation_id.clone();
    let events = drain(&mut h).await;
    let text = answer_text(&events);
    assert!(text.starts_with("I can see the whole clip (video/mp4, 3000 bytes)."), "{text} {events:?}");
    assert!(!text.contains("image(s)"), "no frames went with it: {text}");
    assert!(retries(&events).is_empty(), "{events:?}");
    assert!(saved_whole(&core, &conv));

    // The next turn carries the earlier clip as its frames, never the clip again.
    let mut h = core.send_to(Some(&conv), "and then?", Some(WARM_MODEL.into()), Vec::new(), None).unwrap();
    let text = answer_text(&drain(&mut h).await);
    assert!(text.starts_with(&format!("I can see {n} image(s)")), "{text}");
    assert!(!text.contains("whole clip"), "{text}");

    // Too long, too big, a trimmed range, or a model that doesn't read video: frames.
    core.set_video_override(WARM_MODEL, pool_limits("video_seconds=5 video_bytes=100000")).unwrap();
    assert!(!whole(&attach(&core, None, WARM_MODEL, &path, &prefs).await), "10 s over 5 s");
    core.set_video_override(WARM_MODEL, pool_limits("video_seconds=60 video_bytes=1000")).unwrap();
    assert!(!whole(&attach(&core, None, WARM_MODEL, &path, &prefs).await), "3000 bytes over 1000");
    core.set_video_override(WARM_MODEL, pool_limits("video_seconds=60 video_bytes=100000")).unwrap();
    let trimmed = VideoPrefs { range: Some((1_000, 4_000)), ..VideoPrefs::default() };
    assert!(!whole(&attach(&core, None, WARM_MODEL, &path, &trimmed).await), "only part of it");
    let not_kept = VideoPrefs { keep_original: false, ..VideoPrefs::default() };
    assert!(!whole(&attach(&core, None, WARM_MODEL, &path, &not_kept).await), "not on this device at send time");
    core.set_video_override(COLD_MODEL, pool_limits("context=32768 video_seconds=60 video_bytes=100000")).unwrap();
    assert!(!whole(&attach(&core, None, COLD_MODEL, &path, &prefs).await), "no video in its catalog entry");
    // A request limit under the clip as base64 (3000 bytes become 4000 and the data: prefix).
    core.set_video_override(WARM_MODEL, pool_limits("video_seconds=60 video_bytes=100000 max_request_bytes=4010")).unwrap();
    assert!(!whole(&attach(&core, None, WARM_MODEL, &path, &prefs).await), "over the request limit");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn through_the_pool_a_refused_or_missing_clip_goes_as_frames_and_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, demo) = linked(tmp.path()).await;
    let prefs = VideoPrefs::default();
    core.set_video_override(WARM_MODEL, pool_limits("video_seconds=60 video_bytes=100000")).unwrap();

    // The node turns the clip down with the frames hint: the frames go once, the user is told,
    // and the saved message says frames.
    let path = clip(tmp.path(), "clip.mp4", 3000);
    let a = attach(&core, None, WARM_MODEL, &path, &prefs).await;
    assert!(whole(&a));
    let n = frames(&a);
    demo.faults.refuse_video_next(1);
    let mut h = core.send_to(None, "what happens?", Some(WARM_MODEL.into()), vec![a], Some(Target::Pool)).unwrap();
    let conv = h.conversation_id.clone();
    let events = drain(&mut h).await;
    let said = retries(&events);
    assert!(said.iter().any(|m| m.contains("didn't take the whole clip") && m.contains(&format!("sending {n} frames instead"))), "{events:?}");
    let text = answer_text(&events);
    assert!(text.starts_with(&format!("I can see {n} image(s)")), "{text}");
    assert!(matches!(events.last(), Some(ChatEvent::Done { .. })), "{events:?}");
    assert!(!saved_whole(&core, &conv), "the bubble says frames now");

    // The clip is gone by the time it is sent: frames, with a note first.
    let gone = clip(tmp.path(), "gone.mp4", 3000);
    let a = attach(&core, None, WARM_MODEL, &gone, &prefs).await;
    assert!(whole(&a));
    std::fs::remove_file(&gone).unwrap();
    let mut h = core.send_to(None, "what happens?", Some(WARM_MODEL.into()), vec![a], Some(Target::Pool)).unwrap();
    let conv = h.conversation_id.clone();
    let events = drain(&mut h).await;
    assert!(retries(&events).iter().any(|m| m.contains("isn't on this device") && m.contains("frames")), "{events:?}");
    assert!(answer_text(&events).starts_with(&format!("I can see {n} image(s)")), "{events:?}");
    assert!(!saved_whole(&core, &conv));

    // The clip changed since it was attached: frames too.
    let changed = clip(tmp.path(), "changed.mp4", 3000);
    let a = attach(&core, None, WARM_MODEL, &changed, &prefs).await;
    std::fs::write(&changed, vec![1u8; 2000]).unwrap();
    let mut h = core.send_to(None, "what happens?", Some(WARM_MODEL.into()), vec![a], Some(Target::Pool)).unwrap();
    let events = drain(&mut h).await;
    assert!(answer_text(&events).starts_with(&format!("I can see {n} image(s)")), "{events:?}");
}

async fn endpoint(core: &Core, url: &str, model: &str) -> Target {
    let v = core
        .add_endpoint(
            EndpointInput { name: "Bridge".into(), base_url: url.into(), model: Some(model.into()), mode: EndpointMode::Alongside, enabled: true },
            None,
        )
        .unwrap();
    Target::Endpoint { id: v.id, model: model.into() }
}

fn chat_bodies(mock: &Mock) -> Vec<String> {
    mock.requests().into_iter().filter(|s| s.path.ends_with("/chat/completions")).map(|s| s.body).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_endpoint_that_reads_video_gets_the_whole_clip() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, _demo) = linked(tmp.path()).await;
    let mock = mock_openai(None, vec!["video-1", "plain-1"], Chat::Words(vec!["A ", "dog."])).await;
    let t = endpoint(&core, &mock.url, "video-1").await;
    let path = clip(tmp.path(), "clip.mp4", 3000);

    let plan = core.video_plan(Some(&t), "video-1").await.unwrap();
    assert!(plan.video_ok && plan.max_request_bytes == Some(4_194_304), "{plan:?}");
    assert_eq!(
        (plan.limits.media.video_max_seconds, plan.limits.media.video_max_bytes, plan.limits.media.video_frames_per_minute),
        (Some(60), Some(1_000_000), Some(120))
    );
    let a = attach(&core, Some(&t), "video-1", &path, &VideoPrefs::default()).await;
    assert!(whole(&a));
    let mut h = core.send_to(None, "what is it?", None, vec![a], Some(t.clone())).unwrap();
    let events = drain(&mut h).await;
    assert_eq!(answer_text(&events), "A dog.");
    let bodies = chat_bodies(&mock);
    assert_eq!(bodies.len(), 1);
    assert!(
        bodies[0].contains(r#""type":"video_url""#) && bodies[0].contains("data:video/mp4;base64,"),
        "{}",
        &bodies[0][..300.min(bodies[0].len())]
    );
    assert!(!bodies[0].contains("image_url"), "no frames with the whole clip");
    assert!(bodies[0].contains("sent whole."));

    // A model on the same server that doesn't say it reads video: frames.
    let plain = Target::Endpoint {
        id: match &t {
            Target::Endpoint { id, .. } => id.clone(),
            _ => unreachable!(),
        },
        model: "plain-1".into(),
    };
    core.set_video_override(
        "plain-1",
        ModelLimits::parse("context=32768 image_tokens=256 max_images=8 video_seconds=60 video_bytes=100000 max_request_bytes=4000000").unwrap(),
    )
    .unwrap();
    assert!(!whole(&attach(&core, Some(&plain), "plain-1", &path, &VideoPrefs::default()).await));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_endpoint_that_turns_the_clip_down_gets_the_frames() {
    let tmp = tempfile::tempdir().unwrap();
    let (core, _demo) = linked(tmp.path()).await;
    let path = clip(tmp.path(), "clip.mp4", 3000);
    let hint = format!(r#"{{"error":{{"message":"`video-1` does not take video, or {}"}}}}"#, weirdnet_agent::remote::catalog::VIDEO_FALLBACK);
    let hint: &'static str = Box::leak(hint.into_boxed_str());
    for (code, body, why) in [(400, hint, "frames hint"), (413, r#"{"error":{"message":"request body over 4194304 bytes"}}"#, "too large")] {
        let mock = mock_openai(None, vec!["video-1"], Chat::RefuseVideo(code, body, vec!["Some ", "frames."])).await;
        let t = endpoint(&core, &mock.url, "video-1").await;
        let a = attach(&core, Some(&t), "video-1", &path, &VideoPrefs::default()).await;
        assert!(whole(&a), "{why}");
        let mut h = core.send_to(None, "what is it?", None, vec![a], Some(t)).unwrap();
        let conv = h.conversation_id.clone();
        let events = drain(&mut h).await;
        assert_eq!(answer_text(&events), "Some frames.", "{why}: {events:?}");
        assert!(retries(&events).iter().any(|m| m.contains("didn't take the whole clip") && m.contains("frames instead")), "{why}: {events:?}");
        let bodies = chat_bodies(&mock);
        assert_eq!(bodies.len(), 2, "{why}: one whole, then the frames");
        assert!(bodies[0].contains("video_url") && !bodies[1].contains("video_url") && bodies[1].contains("image_url"), "{why}");
        assert!(!saved_whole(&core, &conv), "{why}");
    }
}
