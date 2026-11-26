//! Playing a clip attached to a message (LF-43, V2): what the shells need to show the clip
//! inline or hand it to the system player. **The core decides, the shell draws**: this module
//! decodes (ffmpeg as a child process, MJPEG frames on a pipe), keeps the decoded frames in step
//! with a clock the shell owns (the desktop's audio output, or a wall clock), and decides when
//! inline playback has to give way to the system player: no ffmpeg, the clip is not on this
//! device, ffmpeg can't decode it, or decoding can't keep up (measured while playing, never
//! guessed). (The decoder half is for `weirdnet-media`.)

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::error::{CoreError, ErrorKind, Result};
use crate::video::{Ffmpeg, Sampled, is_video_path};

/// Frames per second the inline player decodes at most (the picture follows the clock; a chat
/// bubble doesn't need more).
pub const MAX_FPS: f32 = 15.0;
/// The widest frame the inline player decodes.
pub const MAX_WIDTH: u32 = 960;
/// How far the decoded picture may fall behind the clock, and for how long, before the player
/// says the computer can't keep up and offers the system player.
pub const LAG_LIMIT_MS: u64 = 1_500;
pub const LAG_GRACE: Duration = Duration::from_secs(3);

/// Why a clip plays in the system player instead of inline. The shells show [`message`](Self::message)
/// next to an "Open in system player" button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fallback {
    /// The clip was attached on another device, or moved or deleted since.
    NotHere,
    /// No ffmpeg to decode with.
    NoDecoder,
    /// ffmpeg couldn't read it (an unknown codec, a broken file), with what it said.
    CantDecode(String),
    /// Decoding fell behind the clock for longer than [`LAG_GRACE`].
    TooSlow,
}

impl Fallback {
    pub fn message(&self) -> String {
        match self {
            Fallback::NotHere => "The clip isn't on this device; the frames the model saw are below.".into(),
            Fallback::NoDecoder => format!("Can't play it here. {}", Ffmpeg::missing_message()),
            Fallback::CantDecode(why) => format!("Can't play it here ({why}); open it in the system player."),
            Fallback::TooSlow => "This computer can't decode the clip fast enough here; open it in the system player.".into(),
        }
    }
}

/// The clip a message's video can be played from, when it is on this device: a regular file with
/// a video extension (a synced conversation may name a path from another device).
pub fn playable_clip(sampled: &Sampled) -> std::result::Result<PathBuf, Fallback> {
    let path = sampled.clip.as_ref().ok_or(Fallback::NotHere)?;
    if is_video_path(path) && path.is_file() { Ok(path.clone()) } else { Err(Fallback::NotHere) }
}

/// The program and arguments that open `path` in the system's player: `xdg-open` on Linux and
/// the BSDs, `open` on macOS, `cmd /c start` on Windows.
pub fn system_open_command(path: &Path) -> (String, Vec<String>) {
    let p = path.display().to_string();
    if cfg!(target_os = "macos") {
        ("open".into(), vec![p])
    } else if cfg!(windows) {
        ("cmd".into(), vec!["/c".into(), "start".into(), String::new(), p])
    } else {
        ("xdg-open".into(), vec![p])
    }
}

/// Opens `path` in the system player (detached; Bragi doesn't wait for it). Only a video file
/// on this device is opened.
pub fn open_externally(path: &Path) -> Result<()> {
    if !is_video_path(path) || !path.is_file() {
        return Err(CoreError::new(ErrorKind::NotFound, Fallback::NotHere.message()));
    }
    let (program, args) = system_open_command(path);
    Command::new(&program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| CoreError::new(ErrorKind::Unavailable, format!("couldn't start {program}: {e}")))
}

/// Splits a stream of concatenated JPEGs (ffmpeg's `image2pipe` MJPEG output) into whole
/// images: each starts at SOI (`FF D8`) and ends at EOI (`FF D9`). A JPEG's entropy-coded data
/// stuffs every `FF` with `00`, so `FF D9` only appears as the end marker.
#[derive(Debug, Default)]
pub struct JpegSplitter {
    buf: Vec<u8>,
}

/// The longest single frame the splitter keeps; past it the buffer is dropped (a broken stream).
const MAX_JPEG: usize = 16 * 1024 * 1024;

impl JpegSplitter {
    /// Adds `bytes` and returns the images completed by them.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            let Some(start) = find(&self.buf, &[0xFF, 0xD8], 0) else {
                // Keep a trailing FF: it may begin the next SOI.
                let keep = usize::from(self.buf.last() == Some(&0xFF));
                self.buf.drain(..self.buf.len() - keep);
                break;
            };
            match find(&self.buf, &[0xFF, 0xD9], start + 2) {
                Some(end) => {
                    out.push(self.buf[start..end + 2].to_vec());
                    self.buf.drain(..end + 2);
                }
                None => {
                    self.buf.drain(..start);
                    if self.buf.len() > MAX_JPEG {
                        self.buf.clear();
                    }
                    break;
                }
            }
        }
        out
    }
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|i| i + from)
}

/// Decoded frames waiting for the clock, and the lag watch. The shell asks [`pick`](Self::pick)
/// on each redraw with the clock's position.
#[derive(Debug, Default)]
pub struct FrameQueue {
    queue: VecDeque<(u64, Vec<u8>)>,
    /// The frame on screen.
    current: Option<(u64, Vec<u8>)>,
    behind_since: Option<Instant>,
    /// The newest decoded time.
    decoded_to: Option<u64>,
}

impl FrameQueue {
    pub fn push(&mut self, at_ms: u64, jpeg: Vec<u8>) {
        self.decoded_to = Some(at_ms);
        self.queue.push_back((at_ms, jpeg));
    }

    /// How many decoded frames wait (the decoder pauses past a bound).
    pub fn waiting(&self) -> usize {
        self.queue.len()
    }

    /// The frame to show at `position_ms`: the newest decoded one at or before it (older ones
    /// are dropped: picture follows the clock, dropping frames).
    pub fn pick(&mut self, position_ms: u64) -> Option<&(u64, Vec<u8>)> {
        while self.queue.front().is_some_and(|(t, _)| *t <= position_ms) {
            self.current = self.queue.pop_front();
        }
        self.current.as_ref()
    }

    /// After a seek: drop everything decoded for the old position.
    pub fn clear(&mut self) {
        self.queue.clear();
        self.current = None;
        self.behind_since = None;
        self.decoded_to = None;
    }

    /// Whether decoding has been more than [`LAG_LIMIT_MS`] behind `position_ms` for longer than
    /// [`LAG_GRACE`] (`now` is passed in so tests don't sleep). `ended`: the decoder has reached
    /// the end of the clip, so it can't fall behind.
    pub fn too_slow(&mut self, position_ms: u64, ended: bool, now: Instant) -> bool {
        let behind = !ended && self.decoded_to.is_none_or(|d| position_ms > d + LAG_LIMIT_MS);
        if !behind {
            self.behind_since = None;
            return false;
        }
        let since = *self.behind_since.get_or_insert(now);
        now.duration_since(since) > LAG_GRACE
    }
}

/// The decode rate and width for a clip: at most [`MAX_FPS`] and [`MAX_WIDTH`], never above the
/// clip's own (even widths for the encoder).
pub fn decode_shape(fps: Option<f32>, width: u32) -> (f32, u32) {
    let fps = fps.filter(|f| f.is_finite() && *f > 0.0).map_or(MAX_FPS, |f| f.min(MAX_FPS));
    let w = width.clamp(2, MAX_WIDTH) & !1;
    (fps, w)
}

/// ffmpeg's arguments to decode `path` from `start_ms` at `fps` frames per second, `width` wide,
/// as MJPEG on stdout (hardened like the sampler's: `file:` input only, no stdin, no network).
pub fn decode_args(path: &Path, start_ms: u64, fps: f32, width: u32) -> Vec<String> {
    vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-protocol_whitelist".into(),
        "file,pipe".into(),
        "-ss".into(),
        format!("{:.3}", start_ms as f64 / 1000.0),
        "-i".into(),
        format!("file:{}", path.display()),
        "-an".into(),
        "-sn".into(),
        "-vf".into(),
        format!("fps={fps},scale={width}:-2"),
        "-f".into(),
        "image2pipe".into(),
        "-c:v".into(),
        "mjpeg".into(),
        "-q:v".into(),
        "5".into(),
        "pipe:1".into(),
    ]
}

/// What the decoder thread sends.
#[derive(Debug)]
pub enum Decoded {
    Frame(u64, Vec<u8>),
    /// The clip ended where it should.
    End,
    /// ffmpeg stopped with an error before the end.
    Failed(String),
}

/// A running decoder: ffmpeg in a child process and a thread that splits its output into frames
/// with their times. Dropping it kills ffmpeg.
pub struct Decoder {
    child: Child,
    pub frames: mpsc::Receiver<Decoded>,
}

/// Decoded frames buffered ahead of the clock (the pipe then blocks ffmpeg).
const AHEAD: usize = 24;

impl Decoder {
    /// Starts decoding `path` from `start_ms`.
    pub fn start(bin: &Ffmpeg, path: &Path, start_ms: u64, fps: f32, width: u32) -> Result<Self> {
        let mut child = Command::new(&bin.ffmpeg)
            .args(decode_args(path, start_ms, fps, width))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| CoreError::new(ErrorKind::Unavailable, format!("couldn't start ffmpeg: {e}")))?;
        let mut out = child.stdout.take().ok_or_else(|| CoreError::new(ErrorKind::Other, "ffmpeg has no output"))?;
        let mut err = child.stderr.take();
        let (tx, rx) = mpsc::sync_channel(AHEAD);
        std::thread::spawn(move || {
            let mut split = JpegSplitter::default();
            let mut buf = vec![0u8; 64 * 1024];
            let mut n = 0u64;
            let step = 1000.0 / f64::from(fps);
            loop {
                match out.read(&mut buf) {
                    Ok(0) => break,
                    Ok(k) => {
                        for jpeg in split.push(&buf[..k]) {
                            let at = start_ms + (n as f64 * step).round() as u64;
                            n += 1;
                            if tx.send(Decoded::Frame(at, jpeg)).is_err() {
                                return;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            let mut said = String::new();
            if let Some(e) = err.as_mut() {
                let _ = e.take(4096).read_to_string(&mut said);
            }
            let said = said.lines().next().unwrap_or("").trim().to_string();
            let _ = tx.send(if n == 0 || !said.is_empty() {
                Decoded::Failed(if said.is_empty() { "no picture".into() } else { said })
            } else {
                Decoded::End
            });
        });
        Ok(Self { child, frames: rx })
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The bubble line for a message's clip: `video  clip.mp4  0:42  6 frames  sound`.
pub fn bubble(name: &str, v: &Sampled) -> String {
    let n = v.frames.len();
    format!(
        "video  {name}  {}  {n} frame{}  {}",
        crate::audio::clock(v.duration_ms),
        if n == 1 { "" } else { "s" },
        if v.audio.is_some() { "sound" } else { "no sound sent" }
    )
}

/// The frames the model saw, as `[00:07.5]` labels in order.
pub fn frame_list(v: &Sampled) -> Vec<String> {
    v.frames.iter().map(|f| crate::video::label(f.at_ms)).collect()
}

/// The newest clip in a conversation's messages: its attachment name and the sampled clip.
pub fn last_video(c: &crate::history::Conversation) -> Option<(&str, &Sampled)> {
    c.messages.iter().rev().flat_map(|m| m.attachments.iter().rev()).find_map(|a| match &a.media {
        Some(crate::attach::Media::Video(v)) => Some((a.name.as_str(), v)),
        _ => None,
    })
}

/// The soundtrack for playback, as WAV (mono 16 kHz, what the sampler extracts), or `None` when
/// the clip has no sound. Blocking.
pub fn soundtrack(bin: &Ffmpeg, path: &Path) -> Result<Option<Vec<u8>>> {
    use crate::video::FrameSource as _;
    let src = crate::video::FfmpegCli::open(bin.clone(), path)?;
    let probe = src.probe()?;
    if !probe.has_audio {
        return Ok(None);
    }
    src.audio(0, probe.duration_ms).map(Some)
}

/// A silent track `duration_ms` long (1 kHz mono), the clock for a clip without sound.
#[cfg(feature = "decode")]
pub fn silence(duration_ms: u64) -> crate::audio::Pcm {
    crate::audio::Pcm { rate: 1_000, channels: 1, samples: vec![0; duration_ms.max(1) as usize] }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(tag: u8) -> Vec<u8> {
        vec![0xFF, 0xD8, 0xFF, 0xE0, tag, 0xFF, 0x00, tag, 0xFF, 0xD9]
    }

    #[test]
    fn the_splitter_finds_whole_jpegs_across_reads() {
        let stream: Vec<u8> = [jpeg(1), jpeg(2), jpeg(3)].concat();
        let mut s = JpegSplitter::default();
        let mut got = Vec::new();
        for chunk in stream.chunks(3) {
            got.extend(s.push(chunk));
        }
        assert_eq!(got, vec![jpeg(1), jpeg(2), jpeg(3)]);
        // Junk before an image is skipped; a stuffed FF 00 doesn't end it.
        let mut s = JpegSplitter::default();
        assert_eq!(s.push(&[0x00, 0x12, 0xFF]), Vec::<Vec<u8>>::new());
        assert_eq!(s.push(&jpeg(9)[..]), vec![jpeg(9)]);
    }

    #[test]
    fn the_queue_follows_the_clock_and_drops_late_frames() {
        let mut q = FrameQueue::default();
        for t in [0, 100, 200, 300] {
            q.push(t, vec![t as u8]);
        }
        assert_eq!(q.pick(0).map(|f| f.0), Some(0));
        assert_eq!(q.pick(250).map(|f| f.0), Some(200), "100 is dropped, 300 waits");
        assert_eq!(q.waiting(), 1);
        assert_eq!(q.pick(260).map(|f| f.0), Some(200), "holds the frame between decodes");
        q.clear();
        assert!(q.pick(1_000).is_none());
    }

    #[test]
    fn too_slow_is_measured_over_the_grace_period_not_guessed() {
        let mut q = FrameQueue::default();
        let t0 = Instant::now();
        q.push(0, vec![]);
        assert!(!q.too_slow(1_000, false, t0), "within the lag limit");
        assert!(!q.too_slow(2_000, false, t0), "behind, but only just now");
        assert!(!q.too_slow(3_000, false, t0 + Duration::from_secs(2)));
        assert!(q.too_slow(4_000, false, t0 + Duration::from_secs(4)), "behind for longer than the grace");
        q.push(3_900, vec![]);
        assert!(!q.too_slow(4_000, false, t0 + Duration::from_secs(5)), "caught up resets it");
        assert!(!q.too_slow(60_000, true, t0 + Duration::from_secs(60)), "a decoder at the end isn't slow");
    }

    #[test]
    fn decoding_shape_and_arguments_are_bounded_and_hardened() {
        assert_eq!(decode_shape(Some(60.0), 1920), (MAX_FPS, MAX_WIDTH));
        assert_eq!(decode_shape(Some(10.0), 641), (10.0, 640));
        assert_eq!(decode_shape(None, 320), (MAX_FPS, 320));
        let args = decode_args(Path::new("/clips/a.mp4"), 12_500, 15.0, 640);
        let at = |k: &str| args.iter().position(|a| a == k).map(|i| args[i + 1].clone());
        assert_eq!(at("-ss").as_deref(), Some("12.500"));
        assert_eq!(at("-i").as_deref(), Some("file:/clips/a.mp4"));
        assert_eq!(at("-protocol_whitelist").as_deref(), Some("file,pipe"));
        assert_eq!(at("-vf").as_deref(), Some("fps=15,scale=640:-2"));
        assert!(args.contains(&"-nostdin".to_string()) && args.contains(&"-an".to_string()));
    }

    #[test]
    fn fallback_when_the_clip_is_not_here_and_how_the_system_player_is_asked() {
        let mut s = Sampled { summary: String::new(), duration_ms: 1_000, frames: Vec::new(), audio: None, clip: None };
        assert_eq!(playable_clip(&s), Err(Fallback::NotHere));
        let dir = tempfile::tempdir().unwrap();
        s.clip = Some(dir.path().join("gone.mp4"));
        assert_eq!(playable_clip(&s), Err(Fallback::NotHere));
        let notes = dir.path().join("notes.txt");
        std::fs::write(&notes, "x").unwrap();
        s.clip = Some(notes.clone());
        assert_eq!(playable_clip(&s), Err(Fallback::NotHere), "only video files");
        assert!(open_externally(&notes).is_err(), "never hands a non-video to the system");
        let clip = dir.path().join("clip.mp4");
        std::fs::write(&clip, "x").unwrap();
        s.clip = Some(clip.clone());
        assert_eq!(playable_clip(&s), Ok(clip.clone()));
        let (program, args) = system_open_command(&clip);
        assert_eq!(args.last().map(String::as_str), Some(clip.display().to_string().as_str()));
        if cfg!(target_os = "linux") {
            assert_eq!(program, "xdg-open");
        }
        s.frames = vec![
            crate::video::VideoFrame { at_ms: 5_250, mime: "image/jpeg".into(), data: String::new() },
            crate::video::VideoFrame { at_ms: 15_750, mime: "image/jpeg".into(), data: String::new() },
        ];
        s.duration_ms = 42_000;
        assert_eq!(bubble("clip.mp4", &s), "video  clip.mp4  0:42  2 frames  no sound sent");
        assert_eq!(frame_list(&s), ["[00:05.2]", "[00:15.7]"]);
        for f in [Fallback::NotHere, Fallback::NoDecoder, Fallback::CantDecode("x".into()), Fallback::TooSlow] {
            assert!(!f.message().is_empty());
        }
    }
}
