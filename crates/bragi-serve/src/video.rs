//! Video for editor extensions (LF-43, V2): a clip on disk attached to a code chat (sampled by
//! [`bragi_core`] with the same limits and override the app uses, so the extension never needs its
//! own ffmpeg), and each model's video limits with this device's override.
//!
//! - `POST /v1/code/chat` takes `video: {path, max_frames?, max_edge?, mode?, start_ms?, end_ms?}`.
//!   The stream then carries a `{"type": "video", …}` event after `conversation`: the bubble line,
//!   the frames the model saw (times, labels and the JPEGs as base64) and where the clip is.
//! - `POST /v1/video/limits` `{model?, target?}`: reported, yours and effective limits as
//!   `key=value` text, whether video is ready and the chip line.
//! - `POST /v1/video/limits/set` `{model, limits}`: sets this device's override (`key=value …`;
//!   empty clears it); says which values it ignored (an override only tightens a reported limit).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use bragi_core::video::{self, FrameSource, LimitsView, ModelLimits, Sampled, VideoPrefs};
use bragi_core::{Attachment, Core, CoreError, ErrorKind, Media, Target};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{App, core_error};

/// Opens a clip for sampling (ffmpeg by default; a fake source in tests).
pub type OpenClip = Arc<dyn Fn(&Path) -> Result<Box<dyn FrameSource + Send>, CoreError> + Send + Sync>;

/// ffmpeg through the core (refused, with the sentence to show, when it is missing).
pub fn ffmpeg(core: Core) -> OpenClip {
    Arc::new(move |p: &Path| core.video_source(p).map(|s| Box::new(s) as Box<dyn FrameSource + Send>))
}

/// A clip named by an editor: a file on this machine, and how to sample it. None of these sets a
/// frame count on its own: they only narrow what the model's limits allow.
#[derive(Debug, Clone, Deserialize)]
pub struct VideoBody {
    pub path: PathBuf,
    #[serde(default)]
    pub max_frames: Option<u32>,
    #[serde(default)]
    pub max_edge: Option<u32>,
    #[serde(default)]
    pub mode: Option<video::Mode>,
    #[serde(default)]
    pub start_ms: Option<u64>,
    #[serde(default)]
    pub end_ms: Option<u64>,
}

fn bad(m: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::BadRequest, m)
}

impl VideoBody {
    fn prefs(&self) -> Result<VideoPrefs, CoreError> {
        let range = match (self.start_ms, self.end_ms) {
            (None, None) => None,
            (s, e) => {
                let (s, e) = (s.unwrap_or(0), e.unwrap_or(u64::MAX));
                if e <= s {
                    return Err(bad("the clip's end must come after its start"));
                }
                Some((s, e))
            }
        };
        Ok(VideoPrefs { mode: self.mode.unwrap_or_default(), max_frames: self.max_frames, max_edge: self.max_edge, range, ..VideoPrefs::default() })
    }

    /// The clip, if it is an absolute path to a file that is there.
    fn file(&self) -> Result<PathBuf, CoreError> {
        if !self.path.is_absolute() {
            return Err(bad("the clip's path must be absolute"));
        }
        let meta = std::fs::metadata(&self.path).map_err(|_| bad(format!("{} isn't there", self.path.display())))?;
        if !meta.is_file() {
            return Err(bad(format!("{} isn't a file", self.path.display())));
        }
        Ok(self.path.clone())
    }
}

/// The model a clip is sized for: the one asked for, the endpoint target's, or the pool's default.
pub fn model_for(core: &Core, model: Option<&str>, target: Option<&Target>) -> Result<String, CoreError> {
    if let Some(m) = model.filter(|m| !m.is_empty()) {
        return Ok(m.to_string());
    }
    if let Some(Target::Endpoint { model, .. }) = target
        && !model.is_empty()
    {
        return Ok(model.clone());
    }
    core.state().catalog.default_model().map(str::to_string).ok_or_else(|| bad("no model to size the clip's frames for: choose one"))
}

/// Samples the clip for `model` at `target` into one attachment that remembers where the clip is
/// (for the player). `history` is the text already in the request (prompt, context, earlier turns).
pub async fn attach(
    core: &Core,
    open: &OpenClip,
    v: &VideoBody,
    model: &str,
    target: Option<&Target>,
    history: &[&str],
) -> Result<Attachment, CoreError> {
    let path = v.file()?;
    let prefs = v.prefs()?;
    let plan = core.video_plan(target, model).await?;
    let reserved = video::history_tokens(history.iter().copied());
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "clip".into());
    let (core, open, model, p) = (core.clone(), open.clone(), model.to_string(), path.clone());
    let mut a = tokio::task::spawn_blocking(move || {
        let source = open(&p)?;
        core.attach_video_source(&name, source.as_ref(), &model, &plan, &prefs, reserved)
    })
    .await
    .map_err(|e| CoreError::new(ErrorKind::Other, format!("reading the clip stopped: {e}")))??;
    if let Some(Media::Video(s)) = &mut a.media {
        s.clip = Some(path);
    }
    Ok(a)
}

/// The stream's `video` event for an attached clip.
pub fn event(name: &str, s: &Sampled) -> Value {
    json!({
        "type": "video",
        "name": name,
        "bubble": bragi_core::video_play::bubble(name, s),
        "summary": s.summary,
        "duration_ms": s.duration_ms,
        "sound": s.audio.is_some(),
        "clip": s.clip,
        "frames": s.frames.iter().map(|f| json!({"at_ms": f.at_ms, "label": video::label(f.at_ms), "mime": f.mime, "data": f.data})).collect::<Vec<_>>(),
    })
}

fn view_json(v: &LimitsView) -> Value {
    json!({
        "model": v.model,
        "reported": v.reported.to_text(),
        "user": v.user.to_text(),
        "effective": v.effective.to_text(),
        "ready": v.ready(),
        "chip": v.chip(),
        "keys": video::OVERRIDE_KEYS.iter().map(|(k, about)| json!({"key": k, "about": about})).collect::<Vec<_>>(),
    })
}

#[derive(Deserialize)]
pub(crate) struct LimitsBody {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    target: Option<Target>,
}

pub(crate) async fn limits(State(app): State<App>, Json(b): Json<LimitsBody>) -> Response {
    let r = async {
        let model = model_for(&app.core, b.model.as_deref(), b.target.as_ref())?;
        app.core.video_limits_view(b.target.as_ref(), &model).await
    };
    match r.await {
        Ok(v) => Json(view_json(&v)).into_response(),
        Err(e) => core_error(e),
    }
}

#[derive(Deserialize)]
pub(crate) struct SetLimitsBody {
    model: String,
    #[serde(default)]
    limits: String,
    #[serde(default)]
    target: Option<Target>,
}

pub(crate) async fn set_limits(State(app): State<App>, Json(b): Json<SetLimitsBody>) -> Response {
    let r = async {
        if b.model.trim().is_empty() {
            return Err(bad("which model's limits?"));
        }
        let user = ModelLimits::parse(&b.limits)?;
        app.core.set_video_override(&b.model, user)?;
        let v = app.core.video_limits_view(b.target.as_ref(), &b.model).await?;
        let ignored: Vec<&str> = v.ignored(&user);
        let mut out = view_json(&v);
        out["ignored"] = json!(ignored);
        Ok::<_, CoreError>(out)
    };
    match r.await {
        Ok(v) => Json(v).into_response(),
        Err(e) => core_error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clip_must_be_an_absolute_file_and_a_range_must_run_forward() {
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("clip.mp4");
        std::fs::write(&clip, b"x").unwrap();
        let body = |p: &Path, s: Option<u64>, e: Option<u64>| VideoBody {
            path: p.to_path_buf(),
            max_frames: Some(4),
            max_edge: None,
            mode: None,
            start_ms: s,
            end_ms: e,
        };
        assert_eq!(body(&clip, None, None).file().unwrap(), clip);
        assert!(body(Path::new("clip.mp4"), None, None).file().unwrap_err().message.contains("absolute"));
        assert!(body(&dir.path().join("gone.mp4"), None, None).file().unwrap_err().message.contains("isn't there"));
        assert!(body(dir.path(), None, None).file().unwrap_err().message.contains("isn't a file"));
        let p = body(&clip, Some(1000), Some(5000)).prefs().unwrap();
        assert_eq!((p.range, p.max_frames, p.keep_original), (Some((1000, 5000)), Some(4), true));
        assert_eq!(body(&clip, Some(2000), None).prefs().unwrap().range, Some((2000, u64::MAX)));
        assert!(body(&clip, Some(5000), Some(1000)).prefs().is_err());
    }
}
