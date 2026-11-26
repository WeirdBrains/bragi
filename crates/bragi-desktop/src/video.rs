//! The desktop's inline player for a clip attached to a message (LF-43, V2): ffmpeg decodes the
//! picture ([`bragi_core::video_play::Decoder`]), the soundtrack plays through the same output as
//! the audio files ([`crate::audio::Player`], the master clock; a silent wall clock when the clip
//! has no sound or is muted), and the picture follows the clock, dropping frames. When the clip
//! can't play here (not on this device, no ffmpeg, a codec ffmpeg can't read, or decoding that
//! falls behind) [`VideoPlayback::fallback`] says why and the view offers the system player.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

use bragi_core::audio::Pcm;
use bragi_core::video_play::{Decoded, Decoder, Fallback, FrameQueue};
use gpui::{Image, ImageFormat};

use crate::audio::Player;

/// Decoded frames coming in: from ffmpeg (killed when this is dropped), or scripted in tests.
pub enum Feed {
    Ffmpeg(Decoder),
    Scripted(Receiver<Decoded>),
}

impl Feed {
    fn rx(&self) -> &Receiver<Decoded> {
        match self {
            Feed::Ffmpeg(d) => &d.frames,
            Feed::Scripted(r) => r,
        }
    }
}

/// Starts decoding at a position (ms).
pub type StartDecode = Box<dyn FnMut(u64) -> bragi_core::Result<Feed>>;

/// Frames kept decoded ahead of the clock.
const AHEAD: usize = 24;

pub struct VideoPlayback {
    pub conversation: String,
    /// `<message id>#a<n>`, the attachment's file id.
    pub key: String,
    pub clip: PathBuf,
    pub duration_ms: u64,
    /// The clip has sound to play.
    pub has_sound: bool,
    pub muted: bool,
    pub clock: Player,
    pcm: Pcm,
    start: StartDecode,
    feed: Option<Feed>,
    queue: FrameQueue,
    decode_ended: bool,
    /// The picture on screen and its time.
    pub frame: Option<(u64, Arc<Image>)>,
    /// Why it stopped playing inline; the view shows it with "Open in system player".
    pub fallback: Option<Fallback>,
}

impl VideoPlayback {
    /// Starts playing from the top: `pcm` is the soundtrack (or silence when `has_sound` is
    /// false), `start` begins decoding at a position.
    pub fn open(conversation: &str, key: &str, clip: PathBuf, pcm: Pcm, has_sound: bool, device: bool, start: StartDecode) -> Self {
        let clock = Player::open_on(conversation, key, pcm.clone(), device && has_sound);
        let mut p = Self {
            conversation: conversation.to_string(),
            key: key.to_string(),
            clip,
            duration_ms: clock.duration_ms,
            has_sound,
            muted: false,
            clock,
            pcm,
            start,
            feed: None,
            queue: FrameQueue::default(),
            decode_ended: false,
            frame: None,
            fallback: None,
        };
        p.restart(0);
        p
    }

    fn restart(&mut self, at_ms: u64) {
        self.queue.clear();
        self.decode_ended = false;
        self.feed = None;
        match (self.start)(at_ms) {
            Ok(f) => self.feed = Some(f),
            Err(e) => self.fail(Fallback::CantDecode(e.message)),
        }
    }

    fn fail(&mut self, why: Fallback) {
        if self.clock.playing() {
            self.clock.toggle();
        }
        self.feed = None;
        self.fallback = Some(why);
    }

    pub fn playing(&self) -> bool {
        self.fallback.is_none() && self.clock.playing()
    }

    pub fn position_ms(&self) -> u64 {
        self.clock.position_ms()
    }

    /// Takes decoded frames, puts the right one on screen and watches the lag. Returns whether
    /// anything changed (the view redraws).
    pub fn tick(&mut self, now: Instant) -> bool {
        if self.fallback.is_some() {
            return false;
        }
        let mut changed = false;
        let mut failed = None;
        if let Some(feed) = &self.feed {
            while self.queue.waiting() < AHEAD {
                match feed.rx().try_recv() {
                    Ok(Decoded::Frame(at, jpeg)) => self.queue.push(at, jpeg),
                    Ok(Decoded::End) | Err(TryRecvError::Disconnected) => {
                        self.decode_ended = true;
                        break;
                    }
                    Ok(Decoded::Failed(why)) => {
                        failed = Some(why);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }
        }
        if let Some(why) = failed {
            // Mid-clip or at the start: the same fallback.
            self.fail(Fallback::CantDecode(why));
            return true;
        }
        let pos = self.clock.position_ms();
        if let Some((at, jpeg)) = self.queue.pick(pos)
            && self.frame.as_ref().is_none_or(|(t, _)| t != at)
        {
            self.frame = Some((*at, Arc::new(Image::from_bytes(ImageFormat::Jpeg, jpeg.clone()))));
            changed = true;
        }
        if self.clock.playing() && self.queue.too_slow(pos, self.decode_ended, now) {
            self.fail(Fallback::TooSlow);
            return true;
        }
        changed
    }

    /// Play or pause; at the end, from the top.
    pub fn toggle(&mut self) {
        if self.fallback.is_some() {
            return;
        }
        if self.clock.ended() {
            self.seek(0);
        }
        self.clock.toggle();
    }

    pub fn seek(&mut self, ms: u64) {
        if self.fallback.is_some() {
            return;
        }
        let ms = ms.min(self.duration_ms);
        self.clock.seek(ms);
        self.restart(ms);
    }

    /// Sound off or on: the clock moves to a silent wall clock (or back to the output) at the
    /// same place, playing or paused as it was.
    pub fn toggle_mute(&mut self, device: bool) {
        if !self.has_sound {
            return;
        }
        self.muted = !self.muted;
        let (at, playing) = (self.clock.position_ms(), self.clock.playing());
        let mut clock = Player::open_on(&self.conversation, &self.key, self.pcm.clone(), device && !self.muted);
        clock.seek(at);
        if !playing {
            clock.toggle();
        }
        self.clock = clock;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn jpeg(tag: u8) -> Vec<u8> {
        vec![0xFF, 0xD8, tag, 0xFF, 0xD9]
    }

    /// A playback over a scripted decoder: each start hands back a fresh channel, kept here.
    fn scripted(len_ms: u64) -> (VideoPlayback, Arc<std::sync::Mutex<Vec<(u64, mpsc::Sender<Decoded>)>>>) {
        let senders = Arc::new(std::sync::Mutex::new(Vec::new()));
        let keep = senders.clone();
        let start: StartDecode = Box::new(move |at| {
            let (tx, rx) = mpsc::channel();
            keep.lock().unwrap().push((at, tx));
            Ok(Feed::Scripted(rx))
        });
        let p = VideoPlayback::open("c1", "m1#a0", PathBuf::from("/clip.mp4"), bragi_core::video_play::silence(len_ms), false, false, start);
        (p, senders)
    }

    #[test]
    fn the_picture_follows_the_clock_and_seeking_restarts_the_decoder() {
        let (mut p, feeds) = scripted(10_000);
        assert!(p.playing());
        {
            let f = feeds.lock().unwrap();
            assert_eq!(f[0].0, 0);
            for (i, t) in [0u64, 66, 133].iter().enumerate() {
                f[0].1.send(Decoded::Frame(*t, jpeg(i as u8))).unwrap();
            }
        }
        assert!(p.tick(Instant::now()));
        assert_eq!(p.frame.as_ref().map(|f| f.0), Some(0));
        p.toggle();
        assert!(!p.playing(), "paused");
        p.seek(5_000);
        assert_eq!(feeds.lock().unwrap().last().unwrap().0, 5_000, "a seek starts decoding there");
        assert_eq!(p.position_ms(), 5_000);
        feeds.lock().unwrap().last().unwrap().1.send(Decoded::Frame(5_000, jpeg(9))).unwrap();
        p.tick(Instant::now());
        assert_eq!(p.frame.as_ref().map(|f| f.0), Some(5_000));
        assert!(p.fallback.is_none());
    }

    #[test]
    fn a_decoder_failure_falls_back_to_the_system_player() {
        let (mut p, feeds) = scripted(10_000);
        feeds.lock().unwrap()[0].1.send(Decoded::Failed("Invalid data found when processing input".into())).unwrap();
        assert!(p.tick(Instant::now()));
        assert_eq!(p.fallback, Some(Fallback::CantDecode("Invalid data found when processing input".into())));
        assert!(!p.playing() && !p.clock.playing(), "stopped");
        p.toggle();
        assert!(!p.clock.playing(), "no inline play once it fell back");
    }

    #[test]
    fn decoding_that_cant_keep_up_falls_back_after_the_grace() {
        let (mut p, feeds) = scripted(60_000);
        feeds.lock().unwrap()[0].1.send(Decoded::Frame(0, jpeg(0))).unwrap();
        let t0 = Instant::now();
        p.tick(t0);
        // The clock runs on; nothing more is decoded.
        p.clock.seek(5_000);
        p.tick(t0 + Duration::from_millis(100));
        assert!(p.fallback.is_none(), "behind, not yet for long");
        p.clock.seek(9_000);
        p.tick(t0 + Duration::from_secs(4));
        assert_eq!(p.fallback, Some(Fallback::TooSlow));
    }

    #[test]
    fn mute_keeps_the_place_and_a_clip_without_sound_ignores_it() {
        let (mut p, _feeds) = scripted(10_000);
        p.toggle_mute(false);
        assert!(!p.muted, "no sound to mute");
        p.has_sound = true;
        p.seek(3_000);
        p.toggle();
        p.toggle_mute(false);
        assert!(p.muted);
        assert!(!p.clock.playing(), "paused as it was");
        assert!(p.position_ms().abs_diff(3_000) < 50, "{}", p.position_ms());
    }
}
