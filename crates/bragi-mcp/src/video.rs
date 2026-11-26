//! Video over MCP (LF-43, V2): the `video` input on `bragi_chat`, `bragi_video_probe` and
//! `bragi_video_frames`. Sampling is `bragi-core`'s ([`bragi_core::video`]); this only reads the
//! arguments, finds the clip (inside the project root, or a `data:` URL) and shapes the results.
//!
//! weirdnet-mcp 0.6.2's `Content::Image` carries no annotations, so each frame's time goes in the
//! text block just before its image and in `structuredContent` (`frames[].at_ms`, `label`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use bragi_core::video::{self, FrameRequest, FrameSource, Mode, VideoFrame, VideoPrefs};
use bragi_core::{Attachment, CoreError, ErrorKind, Target};
use serde_json::{Value, json};
use weirdnet_mcp::{CallToolResult, Content, Param, ParamType as T, ToolSpec};

use crate::tools::{BragiTools, bad, spec};

/// Opens a clip for reading: ffmpeg in the server, [`video::FakeSource`] in tests.
pub type VideoSourceFn = Arc<dyn Fn(&Path) -> Result<Box<dyn FrameSource + Send>, CoreError> + Send + Sync>;

const FILE: Param = Param::required(
    "file",
    T::String,
    "The clip: a path inside the project root (mp4, m4v, mov, webm, mkv, avi; at most 500 MiB and 10 minutes), or a base64 data: URL.",
);
const START: Param = Param::optional("start_ms", T::Integer, "Read from here (milliseconds). Default: the start.");
const END: Param = Param::optional("end_ms", T::Integer, "Read up to here (milliseconds). Default: the end.");
const MODE: Param = Param::optional(
    "mode",
    T::Enum(&["uniform", "scene", "hybrid"]),
    "How frame times are picked: evenly (uniform, the default), at scene changes (scene), or both (hybrid).",
);
const MAX_EDGE: Param = Param::optional("max_edge", T::Integer, "The longest side of a frame in pixels; may only shrink frames.");

const PROBE_PARAMS: &[Param] = &[FILE];

const FRAMES_PARAMS: &[Param] = &[
    FILE,
    Param::optional(
        "count",
        T::Integer,
        "How many frames (1 to 64). Leave out to size them for `model` from its reported limits instead (there is no default count).",
    ),
    Param::optional("model", T::String, "Size the frames for this model's limits (as bragi_chat would). Default: the pool's pick."),
    Param::optional("endpoint", T::String, "With `model`: the endpoint whose /v1/models limits apply (`pool` or an endpoint id)."),
    MAX_EDGE,
    MODE,
    START,
    END,
];

/// The video tools every server offers (read-only: they read one file the client named).
pub const VIDEO_TOOLS: &[ToolSpec] = &[
    spec(
        "bragi_video_probe",
        "A clip's length, frame size, frame rate and whether it has sound (ffprobe; nothing is decoded or sent anywhere).",
        PROBE_PARAMS,
        true,
        false,
    ),
    spec(
        "bragi_video_frames",
        "Frames from a clip as images, each after a text block with its time ([mm:ss.s]); times and sizes also in structuredContent. Read-only and local.",
        FRAMES_PARAMS,
        true,
        false,
    ),
];

/// A clip on disk for the length of one call: the project file itself, or a `data:` URL written to
/// a private temp file that goes when this is dropped.
pub(crate) struct Clip {
    pub path: PathBuf,
    pub name: String,
    _temp: Option<tempfile::TempDir>,
}

fn int(args: &Value, k: &str) -> Result<Option<u64>, CoreError> {
    match args.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| bad(format!("`{k}` must be a whole number of at least 0"))),
    }
}

fn small(args: &Value, k: &str) -> Result<Option<u32>, CoreError> {
    int(args, k)?.map(|n| u32::try_from(n).map_err(|_| bad(format!("`{k}` is too large")))).transpose()
}

fn mode(args: &Value, k: &str) -> Result<Mode, CoreError> {
    match args.get(k).and_then(Value::as_str) {
        None | Some("") | Some("uniform") => Ok(Mode::Uniform),
        Some("scene") => Ok(Mode::Scene),
        Some("hybrid") => Ok(Mode::Hybrid),
        Some(other) => Err(bad(format!("`{k}` is uniform, scene or hybrid, not `{other}`"))),
    }
}

fn range(args: &Value, start: &str, end: &str) -> Result<Option<(u64, u64)>, CoreError> {
    match (int(args, start)?, int(args, end)?) {
        (None, None) => Ok(None),
        (s, e) => Ok(Some((s.unwrap_or(0), e.unwrap_or(u64::MAX)))),
    }
}

/// The preferences a `bragi_chat` call gives for its clip (with `prefix` `video_`) or a
/// `bragi_video_frames` call (no prefix).
pub(crate) fn prefs(args: &Value, prefix: &str) -> Result<VideoPrefs, CoreError> {
    let k = |s: &str| format!("{prefix}{s}");
    Ok(VideoPrefs {
        mode: mode(args, &k("mode"))?,
        max_frames: small(args, &k("max_frames"))?,
        max_edge: small(args, &k("max_edge"))?,
        range: range(args, &k("start_ms"), &k("end_ms"))?,
        ..VideoPrefs::default()
    })
}

/// The frames as MCP content: a text line, then for each frame its time label and its image.
pub fn frames_result(summary: &str, probe: Option<&video::Probe>, frames: &[VideoFrame]) -> CallToolResult {
    let mut content = vec![Content::text(summary.to_string())];
    for f in frames {
        content.push(Content::text(video::label(f.at_ms)));
        content.push(Content::image(f.data.clone(), f.mime.clone()));
    }
    let mut structured = json!({
        "summary": summary,
        "frames": frames.iter().enumerate().map(|(i, f)| json!({"index": i, "at_ms": f.at_ms, "label": video::label(f.at_ms), "mime_type": f.mime})).collect::<Vec<_>>(),
    });
    if let Some(p) = probe {
        structured["clip"] = probe_json(p);
    }
    CallToolResult::new(content).with_structured(structured)
}

pub fn probe_json(p: &video::Probe) -> Value {
    json!({"duration_ms": p.duration_ms, "width": p.width, "height": p.height, "fps": p.fps, "has_audio": p.has_audio})
}

fn blocking_error(e: tokio::task::JoinError) -> CoreError {
    CoreError::new(ErrorKind::Other, format!("reading the clip stopped: {e}"))
}

impl BragiTools {
    /// The clip a client named: a path inside the root, or a `data:` URL (at most
    /// [`video::MAX_CLIP_BYTES`]) written to a private temp folder.
    pub(crate) fn clip(&self, f: &str) -> Result<Clip, CoreError> {
        if let Some(rest) = f.strip_prefix("data:") {
            let (head, data) = rest.split_once(',').ok_or_else(|| bad("a data: URL needs a comma before its data"))?;
            let mime = head.strip_suffix(";base64").ok_or_else(|| bad("only base64 data: URLs can be attached"))?;
            if data.len() as u64 > video::MAX_CLIP_BYTES / 3 * 4 + 4 {
                return Err(bad(format!("the clip is over {} MiB (trim it)", video::MAX_CLIP_BYTES >> 20)));
            }
            let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|_| bad("the data: URL is not valid base64"))?;
            let ext = match mime {
                "video/webm" => "webm",
                "video/quicktime" => "mov",
                "video/x-matroska" => "mkv",
                "video/x-msvideo" => "avi",
                _ => "mp4",
            };
            let dir = tempfile::Builder::new().prefix("bragi-video-").tempdir().map_err(|e| CoreError::new(ErrorKind::Storage, e.to_string()))?;
            let path = dir.path().join(format!("clip.{ext}"));
            std::fs::write(&path, bytes).map_err(|e| CoreError::new(ErrorKind::Storage, e.to_string()))?;
            return Ok(Clip { name: format!("clip.{ext}"), path, _temp: Some(dir) });
        }
        let path = self.resolve(f)?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(Clip { path, name, _temp: None })
    }

    /// The model a clip is sized for: `model`, the endpoint's, or the pool's pick.
    fn video_model(&self, model: Option<&str>, target: Option<&Target>) -> Result<String, CoreError> {
        if let Some(m) = model.filter(|m| !m.is_empty()) {
            return Ok(m.to_string());
        }
        if let Some(Target::Endpoint { model, .. }) = target
            && !model.is_empty()
        {
            return Ok(model.clone());
        }
        self.core()
            .state()
            .catalog
            .default_model()
            .map(str::to_string)
            .ok_or_else(|| bad("no model to size the frames for: give `model`, or `count` for a fixed number of frames"))
    }

    pub(crate) async fn video_probe(&self, args: &Value) -> Result<CallToolResult, CoreError> {
        let f = weirdnet_mcp::args::string(args, "file").map(str::to_string).map_err(bad)?;
        let clip = self.clip(&f)?;
        let open = self.video_source_fn();
        let path = clip.path.clone();
        let probe = tokio::task::spawn_blocking(move || open(&path)?.probe()).await.map_err(blocking_error)??;
        let secs = probe.duration_ms as f64 / 1000.0;
        let text = format!(
            "{}: {secs:.1} s, {}x{}{}, {}.",
            clip.name,
            probe.width,
            probe.height,
            probe.fps.map(|f| format!(", {f:.2} fps")).unwrap_or_default(),
            if probe.has_audio { "with sound" } else { "no sound" }
        );
        let mut structured = probe_json(&probe);
        structured["name"] = json!(clip.name);
        Ok(CallToolResult::new(vec![Content::text(text)]).with_structured(structured))
    }

    pub(crate) async fn video_frames(&self, args: &Value) -> Result<CallToolResult, CoreError> {
        let f = weirdnet_mcp::args::string(args, "file").map(str::to_string).map_err(bad)?;
        let clip = self.clip(&f)?;
        let prefs = prefs(args, "")?;
        let open = self.video_source_fn();
        let path = clip.path.clone();
        let name = clip.name.clone();
        if let Some(count) = small(args, "count")? {
            let mut req = FrameRequest::new(count);
            req.max_edge = prefs.max_edge;
            req.mode = prefs.mode;
            req.range = prefs.range;
            let (probe, frames) =
                tokio::task::spawn_blocking(move || video::extract(&name, open(&path)?.as_ref(), &req)).await.map_err(blocking_error)??;
            let summary = format!("{} frame{} of {}.", frames.len(), if frames.len() == 1 { "" } else { "s" }, clip.name);
            return Ok(frames_result(&summary, Some(&probe), &frames));
        }
        let model_arg = weirdnet_mcp::args::opt_string(args, "model").map(str::to_string);
        let target = self.target_arg(args, model_arg.as_deref())?;
        let model = self.video_model(model_arg.as_deref(), target.as_ref())?;
        let mut plan = self.core().video_plan(target.as_ref(), &model).await?;
        plan.audio_ok = false;
        let reserved = 0;
        let sampled = tokio::task::spawn_blocking(move || {
            let source = open(&path)?;
            video::sample(&name, source.as_ref(), &model, &plan.limits, &prefs, reserved, plan.audio_ok)
        })
        .await
        .map_err(blocking_error)??;
        Ok(frames_result(&sampled.summary, None, &sampled.frames))
    }

    /// The `video` argument of `bragi_chat` as an attachment sized for where the message goes.
    pub(crate) async fn chat_video(
        &self,
        args: &Value,
        model: Option<&str>,
        target: Option<&Target>,
        prompt: &str,
    ) -> Result<Option<Attachment>, CoreError> {
        let Some(f) = weirdnet_mcp::args::opt_string(args, "video").filter(|v| !v.is_empty()).map(str::to_string) else {
            return Ok(None);
        };
        let clip = self.clip(&f)?;
        let prefs = prefs(args, "video_")?;
        let model = self.video_model(model, target)?;
        let plan = self.core().video_plan(target, &model).await?;
        let mut history: Vec<String> = vec![prompt.to_string()];
        if let Some(s) = weirdnet_mcp::args::opt_string(args, "session")
            && let Ok(c) = self.core().conversation(s)
        {
            history.extend(c.messages.iter().map(|m| m.text.clone()));
        }
        let reserved = video::history_tokens(history.iter().map(String::as_str));
        let open = self.video_source_fn();
        let core = self.core().clone();
        let (path, name) = (clip.path.clone(), clip.name.clone());
        let a = tokio::task::spawn_blocking(move || {
            let source = open(&path)?;
            core.attach_video_source(&name, source.as_ref(), &model, &plan, &prefs, reserved)
        })
        .await
        .map_err(blocking_error)??;
        drop(clip);
        Ok(Some(a))
    }
}
