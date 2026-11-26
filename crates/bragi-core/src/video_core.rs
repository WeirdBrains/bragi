//! [`Core`]'s video actions (LF-43, V1): the limits a model has for video (what the pool reports,
//! with this device's per-model override) and attaching a clip by sampling it with
//! [`crate::video::sample`]. Shells call these; they never sample on their own.

use std::path::{Path, PathBuf};

use weirdnet_agent::Modalities;

use crate::attach::Attachment;
use crate::core::Core;
use crate::error::Result;
use crate::sessions::Target;
use crate::video::{Ffmpeg, FfmpegCli, FrameSource, LimitOverrides, LimitsView, ModelLimits, VideoPrefs};

/// This device's per-model video limits, beside the other device settings.
const OVERRIDES_FILE: &str = "video-limits.json";

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
        let target = self.inner.endpoints.target(endpoint)?;
        crate::openai::model_limits(&target, model).await
    }

    /// `model`'s limits for the shells' chip and settings: reported by `target` (an endpoint's
    /// `/v1/models`, else the pool's catalog), the user's override, and the two together.
    pub async fn video_limits_view(&self, target: Option<&Target>, model: &str) -> Result<LimitsView> {
        let reported = match target {
            Some(Target::Endpoint { id, .. }) => self.reported_endpoint_video_limits(id, model).await?,
            _ => self.reported_video_limits(model),
        };
        Ok(LimitsView::new(model, reported, self.video_override(model)?))
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
        let limits = self.video_limits(model)?;
        let audio_ok = self.state().catalog.model(model).is_some_and(|m| m.takes(Modalities::AUDIO));
        self.attach_video_with(path, model, &limits, audio_ok, prefs, reserved)
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
        let mut a = self.attach_video_source(&name, &source, model, &VideoPlan { limits: *limits, audio_ok }, prefs, reserved)?;
        keep_clip(&mut a, prefs, &source.path);
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
    pub async fn video_plan(&self, target: Option<&Target>, model: &str) -> Result<VideoPlan> {
        match target {
            Some(Target::Endpoint { id, .. }) => Ok(VideoPlan { limits: self.endpoint_video_limits(id, model).await?, audio_ok: false }),
            _ => Ok(VideoPlan {
                limits: self.video_limits(model)?,
                audio_ok: self.state().catalog.model(model).is_some_and(|m| m.takes(Modalities::AUDIO)),
            }),
        }
    }
}

/// The limits a clip is planned with and whether its sound may go as audio
/// ([`Core::video_plan`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoPlan {
    pub limits: ModelLimits,
    pub audio_ok: bool,
}

/// Notes where the original clip is, for the player, when the user keeps originals.
pub(crate) fn keep_clip(a: &mut Attachment, prefs: &VideoPrefs, path: &Path) {
    if prefs.keep_original
        && let Some(crate::attach::Media::Video(v)) = &mut a.media
    {
        v.clip = Some(path.to_path_buf());
    }
}
