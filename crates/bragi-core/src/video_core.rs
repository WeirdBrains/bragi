//! [`Core`]'s video actions (LF-43, V1): the limits a model has for video (what the pool reports,
//! with this device's per-model override) and attaching a clip by sampling it with
//! [`crate::video::sample`]. Shells call these; they never sample on their own.

use std::path::{Path, PathBuf};

use weirdnet_agent::Modalities;

use crate::attach::Attachment;
use crate::core::Core;
use crate::error::Result;
use crate::video::{Ffmpeg, FfmpegCli, LimitOverrides, ModelLimits, VideoPrefs};

/// This device's per-model video limits, beside the other device settings.
const OVERRIDES_FILE: &str = "video-limits.json";

impl Core {
    fn video_overrides_path(&self) -> PathBuf {
        self.inner.cfg.dir.join(OVERRIDES_FILE)
    }

    /// What `model` reports for video, from the catalog. Today the pool reports the context
    /// window; image cost, image count and size, audio length and the request size need fields
    /// in `weirdnet-agent`'s catalog (Weirdnet wish list) and stay absent until then.
    pub fn reported_video_limits(&self, model: &str) -> ModelLimits {
        let state = self.state();
        ModelLimits { context_tokens: state.catalog.context_for(model), ..ModelLimits::default() }
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
        let bin = Ffmpeg::locate().ok_or_else(|| crate::error::CoreError::new(crate::error::ErrorKind::Unavailable, Ffmpeg::missing_message()))?;
        let limits = self.video_limits(model)?;
        let audio_ok = self.state().catalog.model(model).is_some_and(|m| m.takes(Modalities::AUDIO));
        let source = FfmpegCli::open(bin, path)?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let sampled = crate::video::sample(&name, &source, model, &limits, prefs, reserved, audio_ok)?;
        Ok(Attachment::from_video(&name, sampled))
    }
}
