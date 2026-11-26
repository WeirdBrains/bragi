//! Video attachments, phase 1 of `docs/design/video.md` (LF-43, V1): a clip becomes a short list of
//! timestamped frames plus, when the model can take it, its soundtrack. The frames and sound travel as
//! the image and audio parts that already exist, so storage, sync and routing don't change;
//! [`crate::Media::Video`] only groups them so a shell can draw one video instead of N pictures.
//!
//! **There is no default frame count, frame size or soundtrack length.** The plan comes from the
//! limits the model reports ([`ModelLimits`], from Ymir's catalog) and the user's own per-model
//! override for limits nobody reported. When the limits needed to plan are missing, sampling is
//! refused with a message that names them ([`Refusal`]); single images keep working.
//!
//! Decoding is behind [`FrameSource`]: [`FfmpegCli`] runs a bundled or installed `ffmpeg`/`ffprobe`
//! in a child process (`-protocol_whitelist file,pipe`, a time limit, killed on timeout), and the
//! tests use [`FakeSource`]. The generic half of this module (probe, `FrameSource`, the planner and
//! the hardened ffmpeg runner) is meant for a `weirdnet-media` crate; it lives here, marked as such,
//! until the Weirdnet lead publishes one.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, ErrorKind, Result};

/// Largest clip read from disk. A limit on what is *read*; what travels is bounded by the plan.
pub const MAX_CLIP_BYTES: u64 = 500 * 1024 * 1024;
/// Longest clip, in milliseconds (10 minutes).
pub const MAX_CLIP_MS: u64 = 10 * 60 * 1000;
/// Scene changes closer than this to a frame already chosen are the same moment (hybrid mode).
const DEDUP_MS: u64 = 250;
/// JPEG quality for frames, as ffmpeg's `-q:v` (2 best .. 31 worst); about quality 80.
const JPEG_Q: &str = "4";

fn bad(msg: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::BadRequest, msg)
}

fn unavailable(msg: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::Unavailable, msg)
}

/// What a model can take, as reported (by Ymir's catalog) or set by the user for that model. Every
/// field is optional and **absent when unknown**; nothing here is ever filled with a guess.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelLimits {
    /// The context window a request must fit, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u32>,
    /// What one image costs, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_tokens: Option<u32>,
    /// Most images in one request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_images: Option<u32>,
    /// The model's preferred or largest image edge, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_edge: Option<u32>,
    /// Longest soundtrack the model takes as audio, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_max_secs: Option<u32>,
    /// What one second of audio costs, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_tokens_per_sec: Option<u32>,
    /// Largest request the pool's bridge accepts, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_request_bytes: Option<u64>,
}

fn lower(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

impl ModelLimits {
    /// The reported limits with the user's override: the override fills what wasn't reported and may
    /// lower what was, never raise it.
    pub fn with_override(self, user: &ModelLimits) -> ModelLimits {
        ModelLimits {
            context_tokens: lower(self.context_tokens, user.context_tokens),
            image_tokens: match (self.image_tokens, user.image_tokens) {
                // A cost the model reports is what it costs; the user can't make images cheaper.
                (Some(r), Some(u)) => Some(r.max(u)),
                (r, u) => r.or(u),
            },
            max_images: lower(self.max_images, user.max_images),
            max_edge: lower(self.max_edge, user.max_edge),
            audio_max_secs: lower(self.audio_max_secs, user.audio_max_secs),
            audio_tokens_per_sec: match (self.audio_tokens_per_sec, user.audio_tokens_per_sec) {
                (Some(r), Some(u)) => Some(r.max(u)),
                (r, u) => r.or(u),
            },
            max_request_bytes: match (self.max_request_bytes, user.max_request_bytes) {
                (Some(r), Some(u)) => Some(r.min(u)),
                (r, u) => r.or(u),
            },
        }
    }

    /// What planning frames needs and is not known, in words for the user.
    pub fn missing(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.image_tokens.is_none() {
            out.push("image cost");
        }
        if self.context_tokens.is_none() {
            out.push("context size");
        }
        out
    }
}

/// Why a clip can't be sampled for this model, in words a shell shows as is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The limits needed to plan frames aren't reported and the user hasn't set them.
    MissingLimits { model: String, missing: Vec<&'static str> },
    /// The model has room for fewer than two frames of a clip longer than a couple of seconds.
    NoRoom { model: String, frames: u32 },
}

impl Refusal {
    pub fn message(&self) -> String {
        match self {
            Refusal::MissingLimits { model, missing } => format!(
                "{model}'s {} {} reported, so Bragi can't size the frames for it. Images alone still work. \
                 Set limits for this model (context tokens, tokens per image, max images) to attach video.",
                join_and(missing),
                if missing.len() == 1 { "isn't" } else { "aren't" }
            ),
            Refusal::NoRoom { model, frames } => format!(
                "{model} has room for only {frames} frame{} of this clip. Pick a model with a longer context, a shorter clip, or a trimmed range.",
                if *frames == 1 { "" } else { "s" }
            ),
        }
    }

    pub fn into_error(self) -> CoreError {
        bad(self.message())
    }
}

fn join_and(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [a] => (*a).to_string(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// The user's explicit per-model limits for models that don't report them, kept on this device
/// (`video.limits.<model>`). Shown in the attach chip as a user setting, never used as a default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitOverrides(pub BTreeMap<String, ModelLimits>);

impl LimitOverrides {
    /// The overrides in `path`; none when the file isn't there.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(b) => serde_json::from_slice(&b).map_err(|e| CoreError::new(ErrorKind::Storage, format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(CoreError::new(ErrorKind::Storage, format!("{}: {e}", path.display()))),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?;
        crate::files::write_atomic(path, &bytes)
    }

    pub fn get(&self, model: &str) -> ModelLimits {
        self.0.get(model).copied().unwrap_or_default()
    }

    /// Sets (or with all fields empty, clears) the override for `model`.
    pub fn set(&mut self, model: &str, limits: ModelLimits) {
        if limits == ModelLimits::default() {
            self.0.remove(model);
        } else {
            self.0.insert(model.to_string(), limits);
        }
    }
}

/// How the frame times are chosen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// As many evenly spaced frames as the budget allows.
    #[default]
    Uniform,
    /// Frames at scene changes, topped up uniformly.
    Scene,
    /// A uniform backbone with half the budget, scene changes with the rest.
    Hybrid,
}

/// What happens to the soundtrack.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioPref {
    /// As audio when the model takes it, else a transcript when the pool can make one.
    #[default]
    Auto,
    Transcript,
    Off,
}

/// The user's video preferences. None of them sets a frame count or size; they only narrow what the
/// model's limits allow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoPrefs {
    #[serde(default)]
    pub mode: Mode,
    /// A ceiling in frames per second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f32>,
    /// May only lower the model's limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_frames: Option<u32>,
    /// The scene detector's threshold (a detector setting, not a count).
    #[serde(default = "scene_threshold")]
    pub scene_threshold: f32,
    /// May only lower the model's size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_edge: Option<u32>,
    #[serde(default)]
    pub audio: AudioPref,
    /// Start and end in milliseconds; the whole clip when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<(u64, u64)>,
    /// Keep the clip itself with the message for playback.
    #[serde(default = "yes")]
    pub keep_original: bool,
}

fn scene_threshold() -> f32 {
    0.3
}

fn yes() -> bool {
    true
}

impl Default for VideoPrefs {
    fn default() -> Self {
        Self {
            mode: Mode::Uniform,
            fps: None,
            max_frames: None,
            scene_threshold: scene_threshold(),
            max_edge: None,
            audio: AudioPref::Auto,
            range: None,
            keep_original: true,
        }
    }
}

/// What a probe of the clip found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Probe {
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    /// Frames per second, when the container says.
    pub fps: Option<f32>,
    pub has_audio: bool,
}

/// One decoded frame, encoded for the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// A decoder. `FfmpegCli` on desktops; platform decoders on phones; [`FakeSource`] in tests.
/// (To move to `weirdnet-media`.)
pub trait FrameSource {
    fn probe(&self) -> Result<Probe>;
    /// The frame at `at_ms`, scaled to fit `width` x `height`.
    fn frame_at(&self, at_ms: u64, width: u32, height: u32) -> Result<Frame>;
    /// Scene changes above `threshold` as `(ms, score)`, in time order.
    fn scenes(&self, threshold: f32) -> Result<Vec<(u64, f32)>>;
    /// The soundtrack from `start_ms` for `len_ms`, as mono 16 kHz WAV.
    fn audio(&self, start_ms: u64, len_ms: u64) -> Result<Vec<u8>>;
}

/// The frame budget for one clip: how many frames, how large, and how much sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub frames: u32,
    /// Largest edge of a frame, in pixels; `None` keeps the clip's size.
    pub max_edge: Option<u32>,
    /// Milliseconds of soundtrack to send as audio; `None` sends none.
    pub audio_ms: Option<u64>,
}

/// Rough token count of `text`, for the budget.
fn text_tokens(text: &str) -> u64 {
    (text.len() as u64).div_ceil(4)
}

/// A frame's timestamp label, `[00:07.5]`.
pub fn label(ms: u64) -> String {
    let s = ms / 1000;
    format!("[{:02}:{:02}.{}]", s / 60, s % 60, (ms % 1000) / 100)
}

impl Budget {
    /// The budget for a clip of `len_ms` (the chosen range) for `model`, given its limits, the user's
    /// preferences, and `reserved` tokens kept for the prompt, history and the reply. `audio_ok` says
    /// the model takes audio parts. Refuses when the limits that planning needs are missing.
    pub fn from_limits(
        model: &str,
        limits: &ModelLimits,
        prefs: &VideoPrefs,
        len_ms: u64,
        reserved: u32,
        audio_ok: bool,
        has_audio: bool,
    ) -> std::result::Result<Budget, Refusal> {
        let missing = limits.missing();
        let (Some(context), Some(image_tokens)) = (limits.context_tokens, limits.image_tokens) else {
            return Err(Refusal::MissingLimits { model: model.to_string(), missing });
        };
        let image_tokens = u64::from(image_tokens.max(1));
        let mut room = u64::from(context).saturating_sub(u64::from(reserved));
        // The leading line about the clip.
        room = room.saturating_sub(text_tokens("Video \"a-long-file-name.mp4\", 00:00, 000 frames sampled, with sound."));
        // Sound as audio only when the model takes it and says how much (and what it costs).
        let audio_ms = match (prefs.audio, audio_ok && has_audio, limits.audio_max_secs, limits.audio_tokens_per_sec) {
            (AudioPref::Auto, true, Some(max_s), Some(per_s)) => {
                let ms = len_ms.min(u64::from(max_s) * 1000);
                // Leave at least half the room for frames: shorten the sound first.
                let ms = ms.min(room / 2 * 1000 / u64::from(per_s.max(1)));
                (ms > 0).then_some(ms)
            }
            _ => None,
        };
        if let (Some(ms), Some(per_s)) = (audio_ms, limits.audio_tokens_per_sec) {
            room = room.saturating_sub((ms * u64::from(per_s)).div_ceil(1000));
        }
        let per_frame = image_tokens + text_tokens(&label(0));
        let mut frames = room / per_frame;
        if let Some(m) = limits.max_images {
            frames = frames.min(u64::from(m));
        }
        if let Some(m) = prefs.max_frames {
            frames = frames.min(u64::from(m));
        }
        if let Some(fps) = prefs.fps.filter(|f| *f > 0.0) {
            frames = frames.min(((len_ms as f64 / 1000.0) * f64::from(fps)).floor().max(1.0) as u64);
        }
        let frames = frames.min(u64::from(u32::MAX)) as u32;
        if frames == 0 || (frames < 2 && len_ms > 2_000) {
            return Err(Refusal::NoRoom { model: model.to_string(), frames });
        }
        Ok(Budget { frames, max_edge: lower(limits.max_edge, prefs.max_edge), audio_ms })
    }
}

/// Evenly spread times inside `[start, end)`: the middle of each of `n` equal slices.
fn uniform(start: u64, end: u64, n: u32) -> Vec<u64> {
    let len = end.saturating_sub(start);
    if n == 0 || len == 0 {
        return if n == 0 { Vec::new() } else { vec![start] };
    }
    let n = u64::from(n).min(len);
    (0..n).map(|i| start + (2 * i + 1) * len / (2 * n)).collect()
}

/// Sorted, unique, inside `[start, end)`.
fn tidy(mut times: Vec<u64>, start: u64, end: u64) -> Vec<u64> {
    times.retain(|t| *t >= start && *t < end.max(start + 1));
    times.sort_unstable();
    times.dedup();
    times
}

/// Adds uniform times to `times` until it has `n`, skipping ones within [`DEDUP_MS`] of a chosen one.
fn top_up(mut times: Vec<u64>, start: u64, end: u64, n: u32) -> Vec<u64> {
    let mut k = n;
    while (times.len() as u32) < n && k <= n.saturating_mul(4).max(4) {
        for t in uniform(start, end, k) {
            if times.len() as u32 >= n {
                break;
            }
            if !times.iter().any(|c| c.abs_diff(t) < DEDUP_MS) {
                times.push(t);
            }
        }
        k += 1;
    }
    tidy(times, start, end)
}

/// The frame times for `range` with `n` frames. `scenes` are the detector's `(ms, score)` hits (only
/// used by `Scene` and `Hybrid`). Always sorted, unique and inside the range; never more than `n`.
pub fn plan_frames(mode: Mode, range: (u64, u64), n: u32, scenes: &[(u64, f32)]) -> Vec<u64> {
    let (start, end) = range;
    if n == 0 {
        return Vec::new();
    }
    let strongest = |k: usize, near: &[u64]| -> Vec<u64> {
        let mut hits: Vec<(u64, f32)> = scenes.iter().copied().filter(|(t, _)| *t > start && *t < end).collect();
        hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut out: Vec<u64> = Vec::new();
        for (t, _) in hits {
            if out.len() >= k {
                break;
            }
            if !out.iter().chain(near).any(|c| c.abs_diff(t) < DEDUP_MS) {
                out.push(t);
            }
        }
        out
    };
    let times = match mode {
        Mode::Uniform => uniform(start, end, n),
        Mode::Scene => {
            let mut t = vec![start];
            t.extend(strongest(n as usize - 1, &[start]));
            top_up(t, start, end, n)
        }
        Mode::Hybrid => {
            let backbone = uniform(start, end, n.div_ceil(2));
            let mut t = backbone.clone();
            t.extend(strongest((n as usize).saturating_sub(backbone.len()), &backbone));
            top_up(t, start, end, n)
        }
    };
    let mut times = tidy(times, start, end);
    times.truncate(n as usize);
    times
}

/// `w` x `h` scaled to fit `edge` on its longer side (even sizes, as encoders want); unchanged when
/// it fits already or there is no edge.
pub fn fit(w: u32, h: u32, edge: Option<u32>) -> (u32, u32) {
    let even = |x: u32| (x.max(2) / 2) * 2;
    match edge {
        Some(e) if w.max(h) > e && e > 0 => {
            let (w, h) = (u64::from(w), u64::from(h));
            let e = u64::from(e);
            if w >= h { (even(e as u32), even((h * e / w).max(2) as u32)) } else { (even((w * e / h).max(2) as u32), even(e as u32)) }
        }
        _ => (even(w), even(h)),
    }
}

/// One sampled frame, kept with the message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoFrame {
    pub at_ms: u64,
    pub mime: String,
    /// Base64.
    pub data: String,
}

/// What a clip became: what the model sees, grouped for the chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sampled {
    /// The leading line: `Video "name.mp4", 00:42, 6 frames sampled, with sound.`
    pub summary: String,
    pub duration_ms: u64,
    pub frames: Vec<VideoFrame>,
    /// The soundtrack as WAV (base64), when it goes as audio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
}

impl Sampled {
    /// As JSON, the way a shell hands a sampled clip back ([`crate::Attachment::from_media`] with kind `video`).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// Samples the clip `name` read by `source` for `model`: probe, budget, plan, extract, label.
/// `reserved` is the tokens kept for the prompt, history and reply; `audio_ok` says the model takes
/// audio. Refusals come back as [`ErrorKind::BadRequest`] errors with the message to show.
pub fn sample(
    name: &str,
    source: &dyn FrameSource,
    model: &str,
    limits: &ModelLimits,
    prefs: &VideoPrefs,
    reserved: u32,
    audio_ok: bool,
) -> Result<Sampled> {
    let probe = source.probe()?;
    if probe.duration_ms == 0 {
        return Err(bad(format!("{name} has no length; it can't be read as a video")));
    }
    if probe.duration_ms > MAX_CLIP_MS {
        return Err(bad(format!("{name} is {}; the longest clip is {} (trim it)", clock(probe.duration_ms), clock(MAX_CLIP_MS))));
    }
    let (start, end) = match prefs.range {
        Some((s, e)) if s < e => (s.min(probe.duration_ms), e.min(probe.duration_ms)),
        Some(_) => return Err(bad("the clip's range ends before it starts")),
        None => (0, probe.duration_ms),
    };
    if end <= start {
        return Err(bad("the clip's range is empty"));
    }
    let budget = Budget::from_limits(model, limits, prefs, end - start, reserved, audio_ok, probe.has_audio).map_err(Refusal::into_error)?;
    let scenes = match prefs.mode {
        Mode::Uniform => Vec::new(),
        Mode::Scene | Mode::Hybrid => source.scenes(prefs.scene_threshold)?,
    };
    let times = plan_frames(prefs.mode, (start, end), budget.frames, &scenes);
    let (w, h) = fit(probe.width, probe.height, budget.max_edge);
    let mut frames = Vec::with_capacity(times.len());
    for t in times {
        let f = source.frame_at(t, w, h)?;
        if f.bytes.len() > crate::attach::MAX_IMAGE_BYTES {
            return Err(bad(format!("a frame of {name} is over {} KiB; set a smaller frame size", crate::attach::MAX_IMAGE_BYTES / 1024)));
        }
        frames.push(VideoFrame { at_ms: t, mime: f.mime, data: STANDARD.encode(&f.bytes) });
    }
    let mut audio = match budget.audio_ms {
        Some(ms) => {
            let wav = source.audio(start, ms)?;
            (!wav.is_empty() && wav.len() <= crate::attach::MAX_AUDIO_BYTES).then(|| STANDARD.encode(&wav))
        }
        None => None,
    };
    // The request limit: drop frames evenly, then the sound, until it fits.
    if let Some(max) = limits.max_request_bytes {
        let size =
            |f: &[VideoFrame], a: &Option<String>| f.iter().map(|x| x.data.len() as u64).sum::<u64>() + a.as_ref().map_or(0, |a| a.len() as u64);
        while size(&frames, &audio) > max && frames.len() > 1 {
            let keep = frames.len() - 1;
            let picks = uniform(0, frames.len() as u64, keep as u32);
            frames = picks.into_iter().filter_map(|i| frames.get(i as usize).cloned()).collect();
        }
        if size(&frames, &audio) > max {
            audio = None;
        }
        if size(&frames, &audio) > max {
            return Err(bad(format!("even one frame of {name} is bigger than this pool takes in a request ({} KiB)", max / 1024)));
        }
    }
    let sound = if !probe.has_audio {
        "no sound"
    } else if audio.is_some() {
        "with sound"
    } else if prefs.audio == AudioPref::Off {
        "sound off"
    } else {
        "sound not included"
    };
    let n = frames.len();
    let summary = format!(
        "Video \"{}\", {}, {n} frame{} sampled, {sound}.",
        crate::attach::clean_name(name),
        clock(end - start),
        if n == 1 { "" } else { "s" }
    );
    Ok(Sampled { summary, duration_ms: probe.duration_ms, frames, audio })
}

/// A source with generated frames, for tests: no ffmpeg.
#[derive(Debug, Clone)]
pub struct FakeSource {
    pub probe: Probe,
    pub scenes: Vec<(u64, f32)>,
}

impl FakeSource {
    pub fn new(duration_ms: u64, has_audio: bool) -> Self {
        Self { probe: Probe { duration_ms, width: 1920, height: 1080, fps: Some(30.0), has_audio }, scenes: Vec::new() }
    }
}

impl FrameSource for FakeSource {
    fn probe(&self) -> Result<Probe> {
        Ok(self.probe)
    }

    fn frame_at(&self, at_ms: u64, width: u32, height: u32) -> Result<Frame> {
        let mut bytes = vec![0xFF, 0xD8, 0xFF];
        bytes.extend(format!("{at_ms}@{width}x{height}").into_bytes());
        Ok(Frame { mime: "image/jpeg".into(), bytes })
    }

    fn scenes(&self, threshold: f32) -> Result<Vec<(u64, f32)>> {
        Ok(self.scenes.iter().copied().filter(|(_, s)| *s > threshold).collect())
    }

    fn audio(&self, _start_ms: u64, len_ms: u64) -> Result<Vec<u8>> {
        let mut wav = b"RIFF\0\0\0\0WAVE".to_vec();
        wav.extend(std::iter::repeat_n(0u8, (len_ms / 100) as usize));
        Ok(wav)
    }
}

/// `ffmpeg` and `ffprobe`, found in this order: `BRAGI_FFMPEG` (the ffmpeg binary; ffprobe beside
/// it), the copies beside Bragi's executable, then `PATH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ffmpeg {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

impl Ffmpeg {
    pub fn locate() -> Option<Ffmpeg> {
        let pair = |dir: &Path| {
            let (f, p) = (dir.join(exe("ffmpeg")), dir.join(exe("ffprobe")));
            (f.is_file() && p.is_file()).then_some(Ffmpeg { ffmpeg: f, ffprobe: p })
        };
        if let Some(f) = std::env::var_os("BRAGI_FFMPEG").map(PathBuf::from).filter(|p| p.is_file()) {
            let probe = f.parent().map(|d| d.join(exe("ffprobe"))).filter(|p| p.is_file())?;
            return Some(Ffmpeg { ffmpeg: f, ffprobe: probe });
        }
        if let Some(found) = std::env::current_exe().ok().and_then(|e| e.parent().and_then(pair)) {
            return Some(found);
        }
        std::env::split_paths(&std::env::var_os("PATH")?).find_map(|d| pair(&d))
    }

    /// The sentence a shell shows where video attach is unavailable.
    pub fn missing_message() -> &'static str {
        "Video needs ffmpeg: install it from your package manager, or set BRAGI_FFMPEG to an ffmpeg binary with ffprobe beside it."
    }
}

/// Containers that can point at other files or the network; refused even though the protocol
/// whitelist would stop them reading anything.
const REFUSED_FORMATS: &[&str] = &["concat", "hls", "dash", "tty", "lavfi", "ffmetadata", "sdp", "rtp", "rtsp", "image2", "txt"];

/// `ffmpeg` and `ffprobe` in child processes, one file, hardened: input only as `file:` through
/// `-protocol_whitelist file,pipe`, no stdin, a time limit, killed on timeout.
/// (To move to `weirdnet-media`.)
#[derive(Debug, Clone)]
pub struct FfmpegCli {
    pub bin: Ffmpeg,
    pub path: PathBuf,
    /// The time limit for each child.
    pub timeout: Duration,
}

impl FfmpegCli {
    /// For the clip at `path`, refusing files over [`MAX_CLIP_BYTES`].
    pub fn open(bin: Ffmpeg, path: &Path) -> Result<Self> {
        let meta = std::fs::metadata(path).map_err(|e| CoreError::new(ErrorKind::Storage, format!("{}: {e}", path.display())))?;
        if !meta.is_file() {
            return Err(bad(format!("{} is not a file", path.display())));
        }
        if meta.len() > MAX_CLIP_BYTES {
            return Err(bad(format!(
                "{} is {} MiB; the largest clip read is {} MiB (trim it)",
                path.display(),
                meta.len() >> 20,
                MAX_CLIP_BYTES >> 20
            )));
        }
        let path = std::fs::canonicalize(path).map_err(|e| CoreError::new(ErrorKind::Storage, format!("{}: {e}", path.display())))?;
        // 60 s plus up to the longest clip's length.
        Ok(Self { bin, path, timeout: Duration::from_secs(60 + MAX_CLIP_MS / 1000) })
    }

    fn input(&self) -> String {
        format!("file:{}", self.path.display())
    }

    fn ffmpeg(&self) -> Command {
        let mut c = Command::new(&self.bin.ffmpeg);
        c.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-protocol_whitelist", "file,pipe"]);
        c
    }
}

/// Runs `cmd` with no stdin, collecting stdout, and kills it after `timeout`.
pub fn run_limited(mut cmd: Command, timeout: Duration) -> Result<Vec<u8>> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| unavailable(format!("couldn't start ffmpeg: {e}")))?;
    let mut out = child.stdout.take().ok_or_else(|| unavailable("ffmpeg gave no output"))?;
    let mut err = child.stderr.take().ok_or_else(|| unavailable("ffmpeg gave no output"))?;
    let reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = out.read_to_end(&mut b);
        b
    });
    let err_reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = (&mut err).take(64 * 1024).read_to_end(&mut b);
        b
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(bad("reading the clip took too long and was stopped"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(unavailable(format!("ffmpeg: {e}"))),
        }
    };
    let stdout = reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    if !status.success() {
        let why = String::from_utf8_lossy(&stderr);
        let why = why.lines().next().unwrap_or("").trim();
        return Err(bad(format!("the clip can't be read as a video{}", if why.is_empty() { String::new() } else { format!(" ({why})") })));
    }
    Ok(stdout)
}

/// A probe from `ffprobe -of json -show_format -show_streams` output.
pub fn parse_probe(json: &[u8]) -> Result<Probe> {
    let unreadable = || bad("the file can't be read as a video");
    let v: serde_json::Value = serde_json::from_slice(json).map_err(|_| unreadable())?;
    let format = v.get("format").ok_or_else(unreadable)?;
    let names = format.get("format_name").and_then(|n| n.as_str()).unwrap_or("");
    if names.split(',').any(|n| REFUSED_FORMATS.contains(&n)) || names.ends_with("_pipe") {
        return Err(bad("this file is not a video container"));
    }
    let streams = v.get("streams").and_then(|s| s.as_array()).ok_or_else(unreadable)?;
    let video = streams
        .iter()
        .find(|s| {
            s.get("codec_type").and_then(|t| t.as_str()) == Some("video")
                && s.get("disposition").and_then(|d| d.get("attached_pic")).and_then(|a| a.as_i64()) != Some(1)
        })
        .ok_or_else(|| bad("the file has no video track"))?;
    let num = |x: Option<&serde_json::Value>| -> Option<f64> {
        let x = x?;
        x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse().ok()))
    };
    let secs = num(format.get("duration")).or_else(|| num(video.get("duration"))).unwrap_or(0.0);
    let width = video.get("width").and_then(|w| w.as_u64()).unwrap_or(0) as u32;
    let height = video.get("height").and_then(|h| h.as_u64()).unwrap_or(0) as u32;
    if width == 0 || height == 0 {
        return Err(bad("the video track has no picture size"));
    }
    let fps = video.get("avg_frame_rate").and_then(|r| r.as_str()).and_then(|r| {
        let (a, b) = r.split_once('/')?;
        let (a, b): (f32, f32) = (a.parse().ok()?, b.parse().ok()?);
        (b > 0.0 && a > 0.0).then_some(a / b)
    });
    let has_audio = streams.iter().any(|s| s.get("codec_type").and_then(|t| t.as_str()) == Some("audio"));
    Ok(Probe { duration_ms: (secs * 1000.0).max(0.0) as u64, width, height, fps, has_audio })
}

/// Scene hits from ffmpeg's `metadata=print:file=-` output (`pts_time:` then `lavfi.scene_score=`).
pub fn parse_scenes(text: &str) -> Vec<(u64, f32)> {
    let mut out = Vec::new();
    let mut at: Option<u64> = None;
    for line in text.lines() {
        if let Some(t) = line.split_whitespace().find_map(|w| w.strip_prefix("pts_time:")).and_then(|t| t.parse::<f64>().ok()) {
            at = Some((t * 1000.0).max(0.0) as u64);
        } else if let (Some(t), Some(score)) = (at, line.trim().strip_prefix("lavfi.scene_score=").and_then(|s| s.parse::<f32>().ok())) {
            out.push((t, score));
            at = None;
        }
    }
    out
}

impl FrameSource for FfmpegCli {
    fn probe(&self) -> Result<Probe> {
        let mut c = Command::new(&self.bin.ffprobe);
        c.args(["-hide_banner", "-loglevel", "error", "-protocol_whitelist", "file,pipe", "-of", "json", "-show_format", "-show_streams"]);
        c.arg(self.input());
        parse_probe(&run_limited(c, Duration::from_secs(30))?)
    }

    fn frame_at(&self, at_ms: u64, width: u32, height: u32) -> Result<Frame> {
        let mut c = self.ffmpeg();
        c.args(["-ss", &format!("{}.{:03}", at_ms / 1000, at_ms % 1000), "-i"]);
        c.arg(self.input());
        c.args(["-an", "-sn", "-dn", "-map_metadata", "-1", "-frames:v", "1", "-vf", &format!("scale={width}:{height}")]);
        c.args(["-c:v", "mjpeg", "-q:v", JPEG_Q, "-f", "image2pipe", "pipe:1"]);
        let bytes = run_limited(c, self.timeout)?;
        if !bytes.starts_with(&[0xFF, 0xD8]) {
            return Err(bad(format!("no frame at {}", label(at_ms))));
        }
        Ok(Frame { mime: "image/jpeg".into(), bytes })
    }

    fn scenes(&self, threshold: f32) -> Result<Vec<(u64, f32)>> {
        let mut c = self.ffmpeg();
        c.arg("-i").arg(self.input());
        c.args(["-an", "-sn", "-dn", "-vf", &format!("select='gt(scene,{threshold})',metadata=print:file=-"), "-f", "null", "-"]);
        Ok(parse_scenes(&String::from_utf8_lossy(&run_limited(c, self.timeout)?)))
    }

    fn audio(&self, start_ms: u64, len_ms: u64) -> Result<Vec<u8>> {
        let secs = |ms: u64| format!("{}.{:03}", ms / 1000, ms % 1000);
        let mut c = self.ffmpeg();
        c.args(["-ss", &secs(start_ms), "-t", &secs(len_ms), "-i"]);
        c.arg(self.input());
        c.args(["-vn", "-sn", "-dn", "-map_metadata", "-1", "-ac", "1", "-ar", "16000", "-f", "wav", "pipe:1"]);
        run_limited(c, self.timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(context: u32, image: u32) -> ModelLimits {
        ModelLimits { context_tokens: Some(context), image_tokens: Some(image), ..ModelLimits::default() }
    }

    #[test]
    fn missing_limits_refuse_and_name_what_is_missing() {
        let e = Budget::from_limits("m", &ModelLimits::default(), &VideoPrefs::default(), 42_000, 0, false, false).unwrap_err();
        assert_eq!(e, Refusal::MissingLimits { model: "m".into(), missing: vec!["image cost", "context size"] });
        let msg = e.message();
        assert!(msg.contains("image cost and context size aren't reported") && msg.contains("Images alone still work"), "{msg}");
        let only_ctx = ModelLimits { image_tokens: Some(256), ..ModelLimits::default() };
        let e = Budget::from_limits("m", &only_ctx, &VideoPrefs::default(), 42_000, 0, false, false).unwrap_err();
        assert!(e.message().contains("context size isn't reported"), "{}", e.message());
    }

    #[test]
    fn the_override_fills_gaps_and_only_lowers() {
        let reported = ModelLimits { context_tokens: Some(8192), max_images: Some(16), ..ModelLimits::default() };
        let user = ModelLimits { context_tokens: Some(32_768), image_tokens: Some(256), max_images: Some(4), ..ModelLimits::default() };
        let m = reported.with_override(&user);
        assert_eq!((m.context_tokens, m.image_tokens, m.max_images), (Some(8192), Some(256), Some(4)));
    }

    #[test]
    fn the_budget_comes_from_the_limits() {
        let b = Budget::from_limits("m", &limits(8192, 256), &VideoPrefs::default(), 60_000, 2048, false, false).unwrap();
        assert!(b.frames > 2 && u64::from(b.frames) * 256 <= 8192 - 2048, "{b:?}");
        let capped = ModelLimits { max_images: Some(5), ..limits(8192, 256) };
        assert_eq!(Budget::from_limits("m", &capped, &VideoPrefs::default(), 60_000, 0, false, false).unwrap().frames, 5);
        let prefs = VideoPrefs { max_frames: Some(3), ..VideoPrefs::default() };
        assert_eq!(Budget::from_limits("m", &limits(8192, 256), &prefs, 60_000, 0, false, false).unwrap().frames, 3);
        let prefs = VideoPrefs { fps: Some(0.1), ..VideoPrefs::default() };
        assert_eq!(Budget::from_limits("m", &limits(100_000, 256), &prefs, 60_000, 0, false, false).unwrap().frames, 6);
    }

    #[test]
    fn too_little_room_refuses_instead_of_sending_one_poor_frame() {
        let e = Budget::from_limits("tiny", &limits(400, 256), &VideoPrefs::default(), 30_000, 0, false, false).unwrap_err();
        assert!(matches!(e, Refusal::NoRoom { frames: 1, .. }), "{e:?}");
        assert!(e.message().contains("room for only 1 frame"), "{}", e.message());
        // A clip of two seconds or less may go as one frame.
        assert_eq!(Budget::from_limits("tiny", &limits(400, 256), &VideoPrefs::default(), 1_500, 0, false, false).unwrap().frames, 1);
    }

    #[test]
    fn sound_goes_as_audio_only_with_reported_audio_limits() {
        let mut l = limits(32_768, 256);
        let b = Budget::from_limits("m", &l, &VideoPrefs::default(), 60_000, 0, true, true).unwrap();
        assert_eq!(b.audio_ms, None, "no audio limits reported");
        l.audio_max_secs = Some(30);
        l.audio_tokens_per_sec = Some(25);
        let b = Budget::from_limits("m", &l, &VideoPrefs::default(), 60_000, 0, true, true).unwrap();
        assert_eq!(b.audio_ms, Some(30_000));
        let off = VideoPrefs { audio: AudioPref::Off, ..VideoPrefs::default() };
        assert_eq!(Budget::from_limits("m", &l, &off, 60_000, 0, true, true).unwrap().audio_ms, None);
        assert_eq!(Budget::from_limits("m", &l, &VideoPrefs::default(), 60_000, 0, false, true).unwrap().audio_ms, None, "model takes no audio");
    }

    fn check_times(t: &[u64], range: (u64, u64), n: u32) {
        assert!(t.len() as u32 <= n, "{t:?}");
        assert!(t.windows(2).all(|w| w[0] < w[1]), "sorted and unique: {t:?}");
        assert!(t.iter().all(|x| *x >= range.0 && *x < range.1), "inside: {t:?}");
    }

    #[test]
    fn uniform_spreads_across_the_whole_clip() {
        let t = plan_frames(Mode::Uniform, (0, 60_000), 6, &[]);
        assert_eq!(t, vec![5_000, 15_000, 25_000, 35_000, 45_000, 55_000]);
        let t = plan_frames(Mode::Uniform, (10_000, 20_000), 2, &[]);
        assert_eq!(t, vec![12_500, 17_500]);
        assert_eq!(plan_frames(Mode::Uniform, (0, 5), 10, &[]).len(), 5, "never more frames than milliseconds");
        assert!(plan_frames(Mode::Uniform, (0, 1000), 0, &[]).is_empty());
    }

    #[test]
    fn scene_keeps_the_first_frame_and_the_strongest_changes() {
        let scenes = [(3_000, 0.9), (3_100, 0.8), (20_000, 0.4), (40_000, 0.7), (59_000, 0.35)];
        let t = plan_frames(Mode::Scene, (0, 60_000), 3, &scenes);
        assert_eq!(t, vec![0, 3_000, 40_000]);
        // Fewer changes than the budget: topped up uniformly.
        let t = plan_frames(Mode::Scene, (0, 60_000), 8, &scenes[..1]);
        assert_eq!(t.len(), 8);
        assert!(t.contains(&0) && t.contains(&3_000));
        check_times(&t, (0, 60_000), 8);
    }

    #[test]
    fn hybrid_mixes_a_backbone_with_changes_without_near_duplicates() {
        let scenes = [(10_100, 0.9), (33_000, 0.8)];
        let t = plan_frames(Mode::Hybrid, (0, 60_000), 6, &scenes);
        check_times(&t, (0, 60_000), 6);
        assert_eq!(t.len(), 6);
        assert!(t.contains(&33_000) && !t.contains(&10_100), "10.1 s is within 250 ms of the backbone's 10 s: {t:?}");
    }

    #[test]
    fn times_are_always_sorted_unique_and_inside() {
        let scenes: Vec<(u64, f32)> = (0..50).map(|i| (i * 997 % 30_000, (i % 7) as f32 / 7.0)).collect();
        for mode in [Mode::Uniform, Mode::Scene, Mode::Hybrid] {
            for n in [1, 2, 3, 7, 16, 64] {
                for range in [(0, 30_000), (1_000, 1_200), (29_000, 30_000), (0, 3)] {
                    check_times(&plan_frames(mode, range, n, &scenes), range, n);
                }
            }
        }
    }

    #[test]
    fn labels_and_fit() {
        assert_eq!(label(7_500), "[00:07.5]");
        assert_eq!(label(125_040), "[02:05.0]");
        assert_eq!(fit(1920, 1080, Some(768)), (768, 432));
        assert_eq!(fit(1080, 1920, Some(768)), (432, 768));
        assert_eq!(fit(640, 480, Some(768)), (640, 480));
        assert_eq!(fit(641, 481, None), (640, 480));
    }

    #[test]
    fn sampling_labels_frames_and_counts_the_sound() {
        let src = FakeSource::new(42_000, true);
        let l = ModelLimits { max_images: Some(6), max_edge: Some(768), ..limits(32_768, 256) };
        let s = sample("dir/clip.mp4", &src, "m", &l, &VideoPrefs::default(), 1024, false).unwrap();
        assert_eq!(s.summary, "Video \"clip.mp4\", 00:42, 6 frames sampled, sound not included.");
        assert_eq!(s.frames.len(), 6);
        assert_eq!(s.frames[0].at_ms, 3_500);
        let first = STANDARD.decode(&s.frames[0].data).unwrap();
        assert!(first.ends_with(b"3500@768x432"));
        assert!(s.audio.is_none());

        let with_audio = ModelLimits { audio_max_secs: Some(30), audio_tokens_per_sec: Some(25), ..l };
        let s = sample("clip.mp4", &src, "m", &with_audio, &VideoPrefs::default(), 1024, true).unwrap();
        assert!(s.summary.ends_with("with sound.") && s.audio.is_some(), "{}", s.summary);

        let quiet = FakeSource::new(42_000, false);
        assert!(sample("q.mp4", &quiet, "m", &l, &VideoPrefs::default(), 0, true).unwrap().summary.ends_with("no sound."));
    }

    #[test]
    fn sampling_refuses_long_empty_and_unknown() {
        let l = limits(32_768, 256);
        let e = sample("long.mp4", &FakeSource::new(MAX_CLIP_MS + 1, false), "m", &l, &VideoPrefs::default(), 0, false).unwrap_err();
        assert!(e.message.contains("trim it"), "{}", e.message);
        assert!(sample("z.mp4", &FakeSource::new(0, false), "m", &l, &VideoPrefs::default(), 0, false).is_err());
        let e = sample("c.mp4", &FakeSource::new(10_000, false), "m", &ModelLimits::default(), &VideoPrefs::default(), 0, false).unwrap_err();
        assert_eq!(e.kind, ErrorKind::BadRequest);
        assert!(e.message.contains("aren't reported"), "{}", e.message);
    }

    #[test]
    fn the_request_limit_drops_frames_evenly() {
        let src = FakeSource::new(60_000, false);
        let l = ModelLimits { max_images: Some(10), max_request_bytes: Some(100), ..limits(32_768, 256) };
        let s = sample("c.mp4", &src, "m", &l, &VideoPrefs::default(), 0, false).unwrap();
        let size: usize = s.frames.iter().map(|f| f.data.len()).sum();
        assert!(size <= 100 && !s.frames.is_empty() && s.frames.len() < 10, "{} frames, {size} bytes", s.frames.len());
    }

    #[test]
    fn a_range_spends_the_budget_on_that_part() {
        let src = FakeSource::new(60_000, false);
        let prefs = VideoPrefs { range: Some((10_000, 20_000)), ..VideoPrefs::default() };
        let l = ModelLimits { max_images: Some(4), ..limits(32_768, 256) };
        let s = sample("c.mp4", &src, "m", &l, &prefs, 0, false).unwrap();
        assert!(s.frames.iter().all(|f| (10_000..20_000).contains(&f.at_ms)));
        assert!(s.summary.contains("00:10,"), "{}", s.summary);
    }

    #[test]
    fn overrides_round_trip_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("video-limits.json");
        let mut o = LimitOverrides::load(&path).unwrap();
        assert_eq!(o.get("m"), ModelLimits::default());
        o.set("m", limits(8192, 256));
        o.save(&path).unwrap();
        assert_eq!(LimitOverrides::load(&path).unwrap().get("m"), limits(8192, 256));
        o.set("m", ModelLimits::default());
        assert!(o.0.is_empty());
    }

    #[test]
    fn probe_output_is_parsed_and_non_videos_are_refused() {
        let ok = br#"{"streams":[{"codec_type":"video","width":1280,"height":720,"avg_frame_rate":"30000/1001"},{"codec_type":"audio"}],
            "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"42.5"}}"#;
        let p = parse_probe(ok).unwrap();
        assert_eq!((p.duration_ms, p.width, p.height, p.has_audio), (42_500, 1280, 720, true));
        assert!((p.fps.unwrap() - 29.97).abs() < 0.01);
        let concat = br#"{"streams":[{"codec_type":"video","width":2,"height":2}],"format":{"format_name":"concat","duration":"1"}}"#;
        assert!(parse_probe(concat).unwrap_err().message.contains("not a video container"));
        let audio_only = br#"{"streams":[{"codec_type":"audio"}],"format":{"format_name":"wav","duration":"3"}}"#;
        assert!(parse_probe(audio_only).unwrap_err().message.contains("no video track"));
        let cover = br#"{"streams":[{"codec_type":"video","width":500,"height":500,"disposition":{"attached_pic":1}}],"format":{"format_name":"mp3","duration":"3"}}"#;
        assert!(parse_probe(cover).is_err(), "an mp3's cover art is not a video");
        assert!(parse_probe(b"not json").is_err());
    }

    #[test]
    fn scene_output_is_parsed() {
        let text = "frame:0    pts:1001  pts_time:1.001\nlavfi.scene_score=0.512\nframe:1 pts:9009 pts_time:9.009\nlavfi.scene_score=0.33\n";
        assert_eq!(parse_scenes(text), vec![(1_001, 0.512), (9_009, 0.33)]);
    }

    /// Real ffmpeg against a clip made at test time (no fixture checked in); run with `--ignored`
    /// where ffmpeg is installed.
    #[test]
    #[ignore]
    fn ffmpeg_reads_a_generated_clip_and_refuses_hostile_files() {
        let Some(bin) = Ffmpeg::locate() else { return };
        let dir = tempfile::tempdir().unwrap();
        let clip = dir.path().join("clip.mp4");
        let status = Command::new(&bin.ffmpeg)
            .args(["-nostdin", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=duration=3:size=320x240:rate=10"])
            .args(["-f", "lavfi", "-i", "sine=duration=3", "-shortest", "-pix_fmt", "yuv420p"])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(status.success());
        let src = FfmpegCli::open(bin.clone(), &clip).unwrap();
        let p = src.probe().unwrap();
        assert!(p.has_audio && (2_900..=3_100).contains(&p.duration_ms), "{p:?}");
        let f = src.frame_at(1_000, 160, 120).unwrap();
        assert!(f.bytes.starts_with(&[0xFF, 0xD8]));
        assert!(src.audio(0, 1_000).unwrap().starts_with(b"RIFF"));

        let text = dir.path().join("notes.mp4");
        std::fs::write(&text, "just text").unwrap();
        assert!(FfmpegCli::open(bin.clone(), &text).unwrap().probe().is_err());
        let truncated = dir.path().join("cut.mp4");
        std::fs::write(&truncated, &std::fs::read(&clip).unwrap()[..200]).unwrap();
        assert!(FfmpegCli::open(bin.clone(), &truncated).unwrap().probe().is_err());
        let playlist = dir.path().join("list.mp4");
        std::fs::write(&playlist, format!("ffconcat version 1.0\nfile '{}'\n", clip.display())).unwrap();
        assert!(FfmpegCli::open(bin, &playlist).unwrap().probe().is_err());
    }
}
