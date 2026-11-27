//! Video for the Flutter app (LF-43, V3). On Android and iOS the platform decodes: the app reads
//! the clip's facts with the platform (`video_frame_plan` takes them), the core plans which frames
//! and how big from the model's limits, the platform decodes those frames, and
//! `attach_video_frames` turns them into the attachment. Elsewhere (a Flutter desktop build) ffmpeg
//! reads the clip through the core (`attach_video_file`). The player's facts come from
//! `video_attachment`; limits and the override from `video_limits` / `set_video_limits`.

use flutter_rust_bridge::frb;
use std::path::{Path, PathBuf};

use bragi_core::video::{self, Frame, ModelLimits, Probe, Sampled};
use bragi_core::{Core, Media, Target};

use super::bragi::{AttachmentView, TargetView, attachment_view, core, err, target_of};

/// A clip's facts as the platform reads them.
#[derive(Debug, Clone, Copy)]
pub struct VideoProbeView {
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub fps: Option<f32>,
    pub has_audio: bool,
}

/// The frames to decode: times in milliseconds and the size to scale them to.
#[derive(Debug, Clone)]
pub struct VideoFramePlanView {
    pub times_ms: Vec<u64>,
    pub width: u32,
    pub height: u32,
    pub frame_tokens: u32,
}

/// A frame the platform decoded (JPEG, usually).
#[derive(Debug, Clone)]
pub struct VideoFrameIn {
    pub at_ms: u64,
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// A frame the model saw, base64, with its `[00:07.5]` label.
#[derive(Debug, Clone)]
pub struct VideoFrameView {
    pub at_ms: u64,
    pub label: String,
    pub mime: String,
    pub data: String,
}

/// An attached clip, for its card and player.
#[derive(Debug, Clone)]
pub struct VideoInfoView {
    pub name: String,
    /// `video  clip.mp4  0:42  6 frames  no sound sent`.
    pub bubble: String,
    pub summary: String,
    pub duration_ms: u64,
    pub sound: bool,
    /// Where the clip is on the device that attached it.
    pub clip: Option<String>,
    /// The clip is on this device, so it can play.
    pub clip_here: bool,
    pub frames: Vec<VideoFrameView>,
}

/// A model's video limits (each as `key=value` text), the chip line, and after a set, which
/// values were ignored (they would have raised a reported limit).
#[derive(Debug, Clone)]
pub struct VideoLimitsView {
    pub model: String,
    pub reported: String,
    pub user: String,
    pub effective: String,
    pub ready: bool,
    pub chip: String,
    pub keys: Vec<String>,
    pub ignored: Vec<String>,
}

fn probe_of(p: VideoProbeView) -> Probe {
    Probe { duration_ms: p.duration_ms, width: p.width, height: p.height, fps: p.fps, has_audio: p.has_audio }
}

fn model_for(core: &Core, model: Option<String>, target: Option<&Target>) -> Result<String, String> {
    if let Some(m) = model.filter(|m| !m.is_empty()) {
        return Ok(m);
    }
    if let Some(Target::Endpoint { model, .. }) = target
        && !model.is_empty()
    {
        return Ok(model.clone());
    }
    core.state().catalog.default_model().map(str::to_string).ok_or_else(|| "no model to size the clip's frames for: choose one".to_string())
}

fn parts(model: Option<String>, target: Option<TargetView>) -> Result<(Core, String, Option<Target>), String> {
    let core = core()?;
    let target = target.as_ref().map(target_of).transpose()?;
    let model = model_for(&core, model, target.as_ref())?;
    Ok((core, model, target))
}

fn reserved(history: &[String]) -> u32 {
    video::history_tokens(history.iter().map(String::as_str))
}

/// Whether ffmpeg is here for [`attach_video_file`]: `None` when it is, else what to tell the user.
#[frb(sync)]
pub fn video_file_unavailable() -> Option<String> {
    core().ok().and_then(|c| c.video_unavailable()).map(str::to_string)
}

/// Whether clips attached here are copied into their message so linked devices can play them
/// (this device's `video.store_clip`, off by default).
#[frb(sync)]
pub fn video_store_clip() -> bool {
    core().is_ok_and(|c| c.video_prefs().store_clip)
}

/// Turns copying clips into their messages on or off for this device; returns the new value.
#[frb(sync)]
pub fn set_video_store_clip(enabled: bool) -> Result<bool, String> {
    let c = core()?;
    let mut prefs = c.video_prefs();
    prefs.store_clip = enabled;
    c.set_video_prefs(&prefs).map_err(err)?;
    Ok(c.video_prefs().store_clip)
}

/// The largest clip copied into a message, in MiB, for the settings' text.
#[frb(sync)]
pub fn video_stored_clip_max_mib() -> u32 {
    (video::MAX_STORED_CLIP_BYTES >> 20) as u32
}

/// Which frames of the clip `name` to decode, and how big, for `model` at `target` (the default
/// model and target when absent). `history` is the text already going with the message.
pub async fn video_frame_plan(
    name: String,
    probe: VideoProbeView,
    model: Option<String>,
    target: Option<TargetView>,
    history: Vec<String>,
) -> Result<VideoFramePlanView, String> {
    let (core, model, target) = parts(model, target)?;
    let c = core.clone();
    let p = core
        .handle()
        .spawn(async move { c.video_frame_plan(target.as_ref(), &model, &name, &probe_of(probe), &c.video_prefs(), reserved(&history)).await })
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)?;
    Ok(VideoFramePlanView { times_ms: p.times_ms, width: p.width, height: p.height, frame_tokens: p.frame_tokens })
}

/// The clip as one attachment from the frames the platform decoded for [`video_frame_plan`]
/// (call it with the same arguments). `path` is where the clip is, for the player.
#[allow(clippy::too_many_arguments)]
pub async fn attach_video_frames(
    name: String,
    path: Option<String>,
    probe: VideoProbeView,
    frames: Vec<VideoFrameIn>,
    model: Option<String>,
    target: Option<TargetView>,
    history: Vec<String>,
) -> Result<AttachmentView, String> {
    let (core, model, target) = parts(model, target)?;
    let frames: Vec<(u64, Frame)> = frames.into_iter().map(|f| (f.at_ms, Frame { mime: f.mime, bytes: f.bytes })).collect();
    let c = core.clone();
    let a = core
        .handle()
        .spawn(async move {
            let path = path.map(PathBuf::from);
            c.attach_video_frames(target.as_ref(), &model, &name, path.as_deref(), probe_of(probe), frames, &c.video_prefs(), reserved(&history))
                .await
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)?;
    Ok(attachment_view(&a, true))
}

/// The clip at `path` read by ffmpeg through the core (a Flutter build on a computer).
pub async fn attach_video_file(
    path: String,
    model: Option<String>,
    target: Option<TargetView>,
    history: Vec<String>,
) -> Result<AttachmentView, String> {
    let (core, model, target) = parts(model, target)?;
    let c = core.clone();
    let a = core
        .handle()
        .spawn(async move {
            let plan = c.video_plan(target.as_ref(), &model).await?;
            let c2 = c.clone();
            tokio::task::spawn_blocking(move || {
                c2.attach_video_with(Path::new(&path), &model, &plan.limits, plan.audio_ok, &c2.video_prefs(), reserved(&history))
            })
            .await
            .map_err(|e| bragi_core::CoreError::new(bragi_core::ErrorKind::Other, format!("reading the clip stopped: {e}")))?
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)?;
    Ok(attachment_view(&a, true))
}

fn info(name: &str, s: &Sampled) -> VideoInfoView {
    // The original on this device, else the copy stored with the message, written out to play.
    let here =
        bragi_core::video_play::playable_clip(s).ok().or_else(|| s.stored.as_ref().and_then(|_| core().ok()).and_then(|c| c.playable_clip(s).ok()));
    VideoInfoView {
        name: name.to_string(),
        bubble: bragi_core::video_play::bubble(name, s),
        summary: s.summary.clone(),
        duration_ms: s.duration_ms,
        sound: s.audio.is_some(),
        clip: here.as_ref().or(s.clip.as_ref()).map(|p| p.to_string_lossy().into_owned()),
        clip_here: here.is_some(),
        frames: s
            .frames
            .iter()
            .map(|f| VideoFrameView { at_ms: f.at_ms, label: video::label(f.at_ms), mime: f.mime.clone(), data: f.data.clone() })
            .collect(),
    }
}

/// An attachment's clip (the JSON in a `video` [`AttachmentView`]'s `data`), for the chip before sending.
#[frb(sync)]
pub fn video_info(name: String, json: String) -> Result<VideoInfoView, String> {
    let s: Sampled = sampled_of(&json)?;
    Ok(info(&name, &s))
}

fn sampled_of(json: &str) -> Result<Sampled, String> {
    bragi_core::Attachment::from_media("clip", "video", "", json).map_err(err).and_then(|a| match a.media {
        Some(Media::Video(s)) => Ok(s),
        _ => Err("not a video".to_string()),
    })
}

/// The clip of file `file` (`<message id>#a<n>`) in `conversation`, when it is one.
#[frb(sync)]
pub fn video_attachment(conversation: String, file: String) -> Result<Option<VideoInfoView>, String> {
    let c = core()?.conversation(&conversation).map_err(err)?;
    let Some((msg, n)) = file.rsplit_once("#a") else { return Ok(None) };
    let Ok(n) = n.parse::<usize>() else { return Ok(None) };
    Ok(c.messages.iter().find(|m| m.id == msg).and_then(|m| m.attachments.get(n)).and_then(|a| match &a.media {
        Some(Media::Video(s)) => Some(info(&a.name, s)),
        _ => None,
    }))
}

/// `[00:07.5]` for a time in milliseconds.
#[frb(sync)]
pub fn video_label(ms: u64) -> String {
    video::label(ms)
}

fn limits_view(v: &bragi_core::video::LimitsView, ignored: Vec<&str>) -> VideoLimitsView {
    VideoLimitsView {
        model: v.model.clone(),
        reported: v.reported.to_text(),
        user: v.user.to_text(),
        effective: v.effective.to_text(),
        ready: v.ready(),
        chip: v.chip(),
        keys: video::OVERRIDE_KEYS.iter().map(|(k, _)| k.to_string()).collect(),
        ignored: ignored.into_iter().map(str::to_string).collect(),
    }
}

/// `model`'s video limits at `target` (the defaults when absent): reported, yours, effective.
pub async fn video_limits(model: Option<String>, target: Option<TargetView>) -> Result<VideoLimitsView, String> {
    let (core, model, target) = parts(model, target)?;
    let c = core.clone();
    let v = core.handle().spawn(async move { c.video_limits_view(target.as_ref(), &model).await }).await.map_err(|e| e.to_string())?.map_err(err)?;
    Ok(limits_view(&v, Vec::new()))
}

/// Sets this device's video limits for `model` (`key=value …`; empty clears them).
pub async fn set_video_limits(model: Option<String>, limits: String, target: Option<TargetView>) -> Result<VideoLimitsView, String> {
    let (core, model, target) = parts(model, target)?;
    let user = ModelLimits::parse(&limits).map_err(err)?;
    core.set_video_override(&model, user).map_err(err)?;
    let c = core.clone();
    let v = core.handle().spawn(async move { c.video_limits_view(target.as_ref(), &model).await }).await.map_err(|e| e.to_string())?.map_err(err)?;
    let ignored = v.ignored(&user);
    Ok(limits_view(&v, ignored))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_reads_back_for_its_card() {
        let s = Sampled {
            summary: "Video \"clip.mp4\", 00:42, 1 frame sampled, no sound.".into(),
            duration_ms: 42_000,
            frames: vec![bragi_core::video::VideoFrame { at_ms: 7_500, mime: "image/jpeg".into(), data: "AAAA".into() }],
            audio: None,
            clip: Some(PathBuf::from("/nowhere/clip.mp4")),
            stored: None,
        };
        let v = video_info("clip.mp4".into(), s.to_json()).unwrap();
        assert_eq!(v.bubble, "video  clip.mp4  0:42  1 frame  no sound sent");
        assert_eq!(v.frames[0].label, "[00:07.5]");
        assert_eq!(v.clip.as_deref(), Some("/nowhere/clip.mp4"));
        assert!(!v.clip_here, "not on this device");
        assert!(video_info("x".into(), "{".into()).is_err());
        assert_eq!(video_label(61_000), "[01:01.0]");
    }
}
