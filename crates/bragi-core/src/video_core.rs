//! [`Core`]'s video actions (LF-43, V1): the limits a model has for video (what the pool reports,
//! with this device's per-model override) and attaching a clip by sampling it with
//! [`crate::video::sample`]. Shells call these; they never sample on their own.
//!
//! Native video (V4): when the model reads video itself (catalog/1 `inputs.video`, weirdnet-agent
//! 0.5.10, or an endpoint's `/v1/models` `weirdnet.video`) and the clip fits the limits it reports,
//! the attachment is marked to go whole ([`crate::video::whole_clip`]); the frames are still
//! sampled, for the player, later turns and the fallback. When the message is sent the clip is
//! read and goes as one `video_url` part ([`Core::whole_video`]); when it is gone, the request
//! would be too large, or the server turns it down, the frames go instead and the user is told.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use weirdnet_agent::{Message, Modalities, Role};

use crate::attach::{Attachment, Media};
use crate::core::Core;
use crate::error::Result;
use crate::sessions::Target;
use crate::video::{
    Ffmpeg, FfmpegCli, Frame, FramePlan, FrameSource, LimitOverrides, LimitsView, ModelLimits, Probe, ProvidedSource, VideoPrefs, phone_prefs,
};

/// This device's per-model video limits, beside the other device settings.
const OVERRIDES_FILE: &str = "video-limits.json";
/// This device's video preferences ([`VideoPrefs`] without the per-attach range).
const PREFS_FILE: &str = "video-prefs.json";

impl Core {
    fn video_overrides_path(&self) -> PathBuf {
        self.inner.cfg.dir.join(OVERRIDES_FILE)
    }

    /// What `model` reports for video, from the pool's catalog: the context window and the media
    /// and request limits (catalog/1 `context` and `limits`, the strictest across its nodes).
    /// Whatever no node reports stays absent.
    pub fn reported_video_limits(&self, model: &str) -> ModelLimits {
        let state = self.state();
        ModelLimits::new(state.catalog.context_for(model), state.catalog.limits_for(model))
    }

    /// What `model` on OpenAI-compatible endpoint `endpoint` reports (`/v1/models`
    /// `weirdnet.context`, `weirdnet.limits`, `weirdnet.max_request_bytes`: a Ymir bridge says;
    /// other servers say nothing), with this device's override for that model.
    pub async fn endpoint_video_limits(&self, endpoint: &str, model: &str) -> Result<ModelLimits> {
        Ok(self.reported_endpoint_video_limits(endpoint, model).await?.with_override(&self.video_override(model)?))
    }

    /// What `model` on `endpoint` reports, without the override.
    pub async fn reported_endpoint_video_limits(&self, endpoint: &str, model: &str) -> Result<ModelLimits> {
        Ok(self.reported_endpoint_media(endpoint, model).await?.0)
    }

    /// What `model` on `endpoint` reports, without the override, and whether it reads video
    /// itself (`/v1/models` `weirdnet.video`).
    pub async fn reported_endpoint_media(&self, endpoint: &str, model: &str) -> Result<(ModelLimits, bool)> {
        let target = self.inner.endpoints.target(endpoint)?;
        crate::openai::model_media(&target, model).await
    }

    /// Whether `model` in the pool reads video itself (some node's catalog/1 entry says
    /// `inputs.video`; older nodes never do).
    pub fn pool_model_reads_video(&self, model: &str) -> bool {
        self.state().catalog.model(model).is_some_and(|m| m.video)
    }

    /// `model`'s limits for the shells' chip and settings: reported by `target` (an endpoint's
    /// `/v1/models`, else the pool's catalog), the user's override, and the two together.
    pub async fn video_limits_view(&self, target: Option<&Target>, model: &str) -> Result<LimitsView> {
        let (reported, video) = match target {
            Some(Target::Endpoint { id, .. }) => self.reported_endpoint_media(id, model).await?,
            _ => (self.reported_video_limits(model), self.pool_model_reads_video(model)),
        };
        Ok(LimitsView::new(model, reported, self.video_override(model)?).with_video(video))
    }

    /// The user's own limits for `model` (the "set limits for this model" override); empty when
    /// none were set.
    pub fn video_override(&self, model: &str) -> Result<ModelLimits> {
        Ok(LimitOverrides::load(&self.video_overrides_path())?.get(model))
    }

    /// Sets the user's limits for `model`; all fields empty clears them.
    pub fn set_video_override(&self, model: &str, limits: ModelLimits) -> Result<()> {
        let path = self.video_overrides_path();
        let mut all = LimitOverrides::load(&path)?;
        all.set(model, limits);
        all.save(&path)
    }

    /// The limits a clip for `model` is planned with: reported, then the override for what wasn't
    /// (it may lower a reported number, never raise one).
    pub fn video_limits(&self, model: &str) -> Result<ModelLimits> {
        Ok(self.reported_video_limits(model).with_override(&self.video_override(model)?))
    }

    /// This device's video preferences (the defaults until the user changes one). Shells start
    /// every attach from these and add the range.
    pub fn video_prefs(&self) -> VideoPrefs {
        std::fs::read(self.inner.cfg.dir.join(PREFS_FILE))
            .ok()
            .and_then(|b| serde_json::from_slice::<VideoPrefs>(&b).ok())
            .map(|p| VideoPrefs { range: None, ..p })
            .unwrap_or_default()
    }

    /// Saves this device's video preferences (not the range, which is per attach).
    pub fn set_video_prefs(&self, prefs: &VideoPrefs) -> Result<()> {
        let json = serde_json::to_vec_pretty(&VideoPrefs { range: None, ..prefs.clone() })
            .map_err(|e| crate::error::CoreError::new(crate::error::ErrorKind::Storage, e.to_string()))?;
        crate::files::write_atomic(&self.inner.cfg.dir.join(PREFS_FILE), &json)
    }

    /// Whether video can be attached here: `None` when it can, else the sentence to show.
    pub fn video_unavailable(&self) -> Option<&'static str> {
        Ffmpeg::locate().is_none().then(Ffmpeg::missing_message)
    }

    /// Samples the clip at `path` for `model` into one attachment. `reserved` is the tokens kept
    /// for the prompt, history and the reply. Refuses, with the message to show, when ffmpeg is
    /// missing, the clip can't be read, or the model's limits aren't known (then the user can set
    /// them with [`set_video_override`](Self::set_video_override)). Blocking: shells call it off
    /// their UI thread.
    pub fn attach_video(&self, path: &Path, model: &str, prefs: &VideoPrefs, reserved: u32) -> Result<Attachment> {
        let plan = self.pool_video_plan(model)?;
        self.attach_video_file(path, model, &plan, prefs, reserved)
    }

    /// Samples the clip at `path` (read by ffmpeg) for `model` with `plan` ([`video_plan`](Self::video_plan))
    /// into one attachment, keeps the clip for the player when the user keeps originals, and marks
    /// it to go whole when the model reads video and it fits ([`finish_video_attach`](Self::finish_video_attach)).
    /// Blocking: shells call it off their UI thread.
    pub fn attach_video_file(&self, path: &Path, model: &str, plan: &VideoPlan, prefs: &VideoPrefs, reserved: u32) -> Result<Attachment> {
        let source = self.video_source(path)?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut a = self.attach_video_source(&name, &source, model, plan, prefs, reserved)?;
        self.finish_video_attach(&mut a, plan, prefs, &source.path)?;
        Ok(a)
    }

    /// After sampling the clip at `path` into `a`: remembers the clip for the player (and copies it
    /// into the message with `store_clip`) when the user keeps originals, then decides whether it
    /// goes whole ([`decide_video_whole`](Self::decide_video_whole)).
    pub fn finish_video_attach(&self, a: &mut Attachment, plan: &VideoPlan, prefs: &VideoPrefs, path: &Path) -> Result<()> {
        keep_clip(a, prefs, path)?;
        self.decide_video_whole(a, plan, prefs, path);
        Ok(())
    }

    /// Marks the clip in `a` (at `path` on this device) to go whole when [`crate::video::whole_clip`]
    /// says it can: the model reads video, the clip is kept here and not trimmed, and its length, size
    /// and request size fit what is reported. Otherwise it goes as its frames.
    pub fn decide_video_whole(&self, a: &mut Attachment, plan: &VideoPlan, prefs: &VideoPrefs, path: &Path) {
        let Some(Media::Video(v)) = &mut a.media else { return };
        let size = std::fs::metadata(path).ok().filter(|m| m.is_file()).map(|m| m.len());
        let trimmed = prefs.range.is_some_and(|(start, end)| start > 0 || end < v.duration_ms);
        let check = crate::video::WholeCheck {
            takes_video: plan.video_ok,
            limits: &plan.limits,
            max_request_bytes: plan.max_request_bytes,
            duration_ms: v.duration_ms,
            bytes: size.unwrap_or(0),
            mime: crate::video::video_mime(path),
            trimmed,
            kept: size.is_some() && (v.clip.is_some() || v.stored.is_some()),
        };
        v.whole = match crate::video::whole_clip(&check) {
            Ok(w) => Some(w),
            Err(why) => {
                if plan.video_ok {
                    tracing::debug!("the clip goes as frames: {why}");
                }
                None
            }
        };
    }

    /// The plan for `model` in the pool: its limits with this device's override, whether it takes
    /// audio and reads video, and the largest request (the link's message limit, or less when the
    /// pool reports less).
    pub fn pool_video_plan(&self, model: &str) -> Result<VideoPlan> {
        let limits = self.video_limits(model)?;
        let link = weirdnet_agent::remote::codec::MAX_FRAME as u64;
        Ok(VideoPlan {
            limits,
            audio_ok: self.state().catalog.model(model).is_some_and(|m| m.takes(Modalities::AUDIO)),
            video_ok: self.pool_model_reads_video(model),
            max_request_bytes: Some(limits.media.max_request_bytes.map_or(link, |m| m.min(link))),
        })
    }

    /// [`attach_video`](Self::attach_video) with limits the caller already has (an endpoint's,
    /// from [`endpoint_video_limits`](Self::endpoint_video_limits)). `audio_ok`: the model takes audio.
    pub fn attach_video_with(
        &self,
        path: &Path,
        model: &str,
        limits: &ModelLimits,
        audio_ok: bool,
        prefs: &VideoPrefs,
        reserved: u32,
    ) -> Result<Attachment> {
        let source = self.video_source(path)?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let plan = VideoPlan { limits: *limits, audio_ok, video_ok: false, max_request_bytes: limits.media.max_request_bytes };
        let mut a = self.attach_video_source(&name, &source, model, &plan, prefs, reserved)?;
        keep_clip(&mut a, prefs, &source.path)?;
        Ok(a)
    }

    /// [`attach_video_with`](Self::attach_video_with) from any [`FrameSource`] (a platform
    /// decoder, or `FakeSource` in tests).
    pub fn attach_video_source(
        &self,
        name: &str,
        source: &dyn FrameSource,
        model: &str,
        plan: &VideoPlan,
        prefs: &VideoPrefs,
        reserved: u32,
    ) -> Result<Attachment> {
        let sampled = crate::video::sample(name, source, model, &plan.limits, prefs, reserved, plan.audio_ok)?;
        Ok(Attachment::from_video(name, sampled))
    }

    /// The clip at `path` read by ffmpeg (refused, with the sentence to show, when ffmpeg is
    /// missing or the file is too big).
    pub fn video_source(&self, path: &Path) -> Result<FfmpegCli> {
        let bin = Ffmpeg::locate().ok_or_else(|| crate::error::CoreError::new(crate::error::ErrorKind::Unavailable, Ffmpeg::missing_message()))?;
        FfmpegCli::open(bin, path)
    }

    /// What a clip for `model` at `target` is planned with: an endpoint's `/v1/models` limits
    /// (frames only: an endpoint doesn't say it takes audio), else the pool's, each with this
    /// device's override.
    /// Native video (V4): `video_ok` says the model reads video itself, `max_request_bytes` how
    /// large a request may be (an endpoint's own number only; through the pool, the link's message
    /// limit or less).
    pub async fn video_plan(&self, target: Option<&Target>, model: &str) -> Result<VideoPlan> {
        match target {
            Some(Target::Endpoint { id, .. }) => {
                let (reported, video_ok) = self.reported_endpoint_media(id, model).await?;
                let limits = reported.with_override(&self.video_override(model)?);
                Ok(VideoPlan { limits, audio_ok: false, video_ok, max_request_bytes: limits.media.max_request_bytes })
            }
            _ => self.pool_video_plan(model),
        }
    }
}

impl Core {
    /// What a phone's decoder should read for the clip `name` (its `probe` from the platform) sent
    /// to `model` at `target`: the frame times and size, from the same limits and override as
    /// everywhere else (LF-43, V3). Refuses like [`attach_video`](Self::attach_video).
    pub async fn video_frame_plan(
        &self,
        target: Option<&Target>,
        model: &str,
        name: &str,
        probe: &Probe,
        prefs: &VideoPrefs,
        reserved: u32,
    ) -> Result<FramePlan> {
        let plan = self.video_plan(target, model).await?;
        crate::video::plan(name, probe, model, &plan.limits, prefs, reserved)
    }

    /// The clip `name` as one attachment from the frames a phone's decoder made for
    /// [`video_frame_plan`](Self::video_frame_plan) (no soundtrack on phones). `path` is where the
    /// clip is on this device, kept for the player when `prefs.keep_original` is on.
    #[allow(clippy::too_many_arguments)]
    pub async fn attach_video_frames(
        &self,
        target: Option<&Target>,
        model: &str,
        name: &str,
        path: Option<&Path>,
        probe: Probe,
        frames: Vec<(u64, Frame)>,
        prefs: &VideoPrefs,
        reserved: u32,
    ) -> Result<Attachment> {
        let plan = self.video_plan(target, model).await?;
        let source = ProvidedSource { probe, frames };
        let phone = phone_prefs(prefs);
        let plan = VideoPlan { audio_ok: false, ..plan };
        let mut a = self.attach_video_source(name, &source, model, &plan, &phone, reserved)?;
        if let Some(p) = path {
            self.finish_video_attach(&mut a, &plan, prefs, p)?;
        }
        Ok(a)
    }
}

/// The limits a clip is planned with and whether its sound may go as audio
/// ([`Core::video_plan`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoPlan {
    pub limits: ModelLimits,
    pub audio_ok: bool,
    /// The model reads video itself, so a clip that fits may go whole (V4).
    pub video_ok: bool,
    /// The largest request that can carry a whole clip, in bytes; `None` when nobody says (then
    /// clips go as frames).
    pub max_request_bytes: Option<u64>,
}

/// Notes where the original clip is, for the player, when the user keeps originals; with
/// `store_clip` also copies it into the message (up to [`crate::video::MAX_STORED_CLIP_BYTES`]) for
/// linked devices.
pub(crate) fn keep_clip(a: &mut Attachment, prefs: &VideoPrefs, path: &Path) -> Result<()> {
    if prefs.keep_original
        && let Some(crate::attach::Media::Video(v)) = &mut a.media
    {
        v.clip = Some(path.to_path_buf());
        if prefs.store_clip {
            v.stored = crate::video::StoredClip::read(path)?;
        }
    }
    Ok(())
}

/// How the newest message's clip goes ([`Core::whole_video`]).
#[derive(Debug, Clone, Default)]
pub(crate) struct VideoSend {
    /// The same messages with the frames instead of the whole clip, to send when the server
    /// turns the clip down. `None` when nothing went whole.
    pub frames: Option<Vec<Message>>,
    /// How many frames those are, for what the user is told.
    pub frame_count: usize,
    /// Said before sending: the clip was to go whole but goes as frames.
    pub note: Option<String>,
}

impl VideoSend {
    /// What the user is told when the server turned the whole clip down.
    pub fn retry_message(&self, e: &crate::error::CoreError) -> String {
        format!(
            "The model didn't take the whole clip ({}); sending {} frame{} instead",
            e.message,
            self.frame_count,
            if self.frame_count == 1 { "" } else { "s" }
        )
    }
}

/// Whether one of `attachments` is a clip marked to go whole.
pub(crate) fn sends_whole(attachments: &[Attachment]) -> bool {
    attachments.iter().any(|a| matches!(&a.media, Some(Media::Video(v)) if v.sends_whole()))
}

impl Core {
    /// The messages to send for a turn whose newest message (`prompt` and `attachments`, saved in
    /// `conversation`) carries a clip marked to go whole, from `messages` (with the frames, as
    /// [`begin_turn`](Self::begin_turn) made them): the newest user message gets the clip as one
    /// `video_url` part, read now from the original (or the copy stored with the message), and the
    /// frames version is kept for a retry. When the clip is gone or changed, or the request would
    /// be over the limit it was checked against, the frames go and [`VideoSend::note`] says why
    /// (and the saved message says frames). Earlier turns always carry their frames.
    pub(crate) fn whole_video(
        &self,
        conversation: &str,
        prompt: &str,
        attachments: &[Attachment],
        messages: Vec<Message>,
    ) -> (Vec<Message>, VideoSend) {
        let Some((name, v)) = attachments.iter().find_map(|a| match &a.media {
            Some(Media::Video(v)) if v.sends_whole() => Some((a.name.as_str(), v)),
            _ => None,
        }) else {
            return (messages, VideoSend::default());
        };
        let Some(whole) = v.whole.clone() else { return (messages, VideoSend::default()) };
        let frame_count = v.frames.len();
        let frames_note = |why: String| {
            let note = format!("{why}; sending {frame_count} frame{} of {name} instead", if frame_count == 1 { "" } else { "s" });
            VideoSend { frames: None, frame_count, note: Some(note) }
        };
        let read = self.playable_clip(v).ok().and_then(|p| std::fs::read(p).ok()).filter(|b| b.len() as u64 == whole.bytes);
        let Some(bytes) = read else {
            self.sent_as_frames(conversation);
            return (messages, frames_note("The clip isn't on this device as it was attached".into()));
        };
        let url = format!("data:{};base64,{}", whole.mime, base64::engine::general_purpose::STANDARD.encode(&bytes));
        drop(bytes);
        let mut native = messages.clone();
        if let Some(m) = native.iter_mut().rev().find(|m| m.role == Role::User) {
            m.content = crate::attach::content_whole(prompt.trim(), attachments, &url);
        }
        let size = serde_json::to_vec(&native).map_or(u64::MAX, |b| b.len() as u64);
        if size > whole.max_request_bytes {
            self.sent_as_frames(conversation);
            return (
                messages,
                frames_note(format!(
                    "The whole clip and this chat come to {}, over the {} a request takes",
                    crate::video::size_label(size),
                    crate::video::size_label(whole.max_request_bytes)
                )),
            );
        }
        (native, VideoSend { frames: Some(messages), frame_count, note: None })
    }

    /// The newest user message in `conversation` says its clip went as frames after all (the
    /// clip was gone or too large, or the server turned it down), so its bubble says so.
    pub(crate) fn sent_as_frames(&self, conversation: &str) {
        if conversation.is_empty() {
            return;
        }
        let Ok(mut c) = self.inner.history.load(conversation) else { return };
        let Some(m) = c.messages.iter_mut().rev().find(|m| m.role == crate::history::Role::User) else { return };
        let mut changed = false;
        for a in &mut m.attachments {
            if let Some(Media::Video(v)) = &mut a.media
                && v.whole.take().is_some()
            {
                changed = true;
            }
        }
        if changed && let Err(e) = self.inner.history.save(&c) {
            tracing::warn!("saving {conversation}: {e}");
        }
    }
}

impl Core {
    /// Where the copies of clips stored with messages are written to play (`<dir>/clips`).
    pub fn clip_cache(&self) -> PathBuf {
        self.inner.cfg.dir.join("clips")
    }

    /// The file a message's clip plays from on this device: the original, else the copy stored
    /// with the message, written into [`clip_cache`](Self::clip_cache) once.
    pub fn playable_clip(&self, sampled: &crate::video::Sampled) -> std::result::Result<PathBuf, crate::video_play::Fallback> {
        crate::video_play::playable_clip_in(sampled, &self.clip_cache())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::{Sampled, StoredClip};

    fn video() -> Attachment {
        Attachment::from_video(
            "c.mp4",
            Sampled { summary: String::new(), duration_ms: 1, frames: Vec::new(), audio: None, clip: None, stored: None, whole: None },
        )
    }

    fn sampled(a: &Attachment) -> &Sampled {
        match &a.media {
            Some(crate::attach::Media::Video(v)) => v,
            _ => unreachable!(),
        }
    }

    #[test]
    fn keep_clip_remembers_the_path_and_copies_the_clip_only_when_asked() {
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("c.mp4");
        std::fs::write(&clip, b"clip").unwrap();
        let mut a = video();
        keep_clip(&mut a, &VideoPrefs::default(), &clip).unwrap();
        assert_eq!((sampled(&a).clip.as_deref(), &sampled(&a).stored), (Some(clip.as_path()), &None), "off by default: the path only");
        let mut a = video();
        keep_clip(&mut a, &VideoPrefs { store_clip: true, ..VideoPrefs::default() }, &clip).unwrap();
        assert_eq!(sampled(&a).stored, StoredClip::read(&clip).unwrap());
        let mut a = video();
        keep_clip(&mut a, &VideoPrefs { store_clip: true, keep_original: false, ..VideoPrefs::default() }, &clip).unwrap();
        assert_eq!((&sampled(&a).clip, &sampled(&a).stored), (&None, &None), "nothing kept without keep_original");
    }

    #[test]
    fn video_prefs_persist_on_this_device_without_the_range() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = crate::core::Config { transport: crate::core::Transport::Memory, ..crate::core::Config::new(dir.path().join("app"), "t") };
        let core = Core::start(cfg).unwrap();
        assert_eq!(core.video_prefs(), VideoPrefs::default());
        assert!(!core.video_prefs().store_clip, "opt-in");
        core.set_video_prefs(&VideoPrefs { store_clip: true, range: Some((1, 2)), ..VideoPrefs::default() }).unwrap();
        let p = core.video_prefs();
        assert!(p.store_clip && p.keep_original);
        assert_eq!(p.range, None, "the range is per attach");
        assert_eq!(core.clip_cache(), dir.path().join("app").join("clips"));
        std::fs::write(dir.path().join("app").join(PREFS_FILE), b"{oops").unwrap();
        assert_eq!(core.video_prefs(), VideoPrefs::default(), "a damaged file reads as the defaults");
    }
}
