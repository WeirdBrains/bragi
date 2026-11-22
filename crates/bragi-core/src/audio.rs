//! Playing the sound files in a chat (WAV and MP3, the audio a message takes): what both shells
//! agree on, whichever player makes the sound.
//!
//! - [`Deck`]: one clip plays at a time, and it stops when the chat on screen changes.
//! - [`clock`] and [`seek_ms`]: the elapsed / total times and the seek bar.
//! - [`decode`] (the `decode` feature, for shells that play through their own output, the
//!   desktop): the file's samples. Phones play through the platform's player and audio session
//!   instead.
//!
//! The approach follows Mouser's player (`mouser-media`): the sound card's callback is the
//! clock, and without a sound card a wall clock runs instead, so the player still works (and
//! the tests run) on a machine with no audio device.

/// Which clip is playing: one at a time, in the chat on screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Deck {
    clip: Option<(String, String)>,
}

impl Deck {
    /// The clip playing (or paused): its conversation and file id.
    pub fn clip(&self) -> Option<(&str, &str)> {
        self.clip.as_ref().map(|(c, f)| (c.as_str(), f.as_str()))
    }

    /// Whether file `file` of `conversation` is the current clip.
    pub fn is(&self, conversation: &str, file: &str) -> bool {
        self.clip() == Some((conversation, file))
    }

    /// Makes file `file` of `conversation` the current clip. Returns the clip to stop first (a
    /// different one), if any.
    pub fn start(&mut self, conversation: &str, file: &str) -> Option<(String, String)> {
        if self.is(conversation, file) {
            return None;
        }
        self.clip.replace((conversation.to_string(), file.to_string()))
    }

    /// The chat on screen is now `conversation` (`None`: none). Returns whether the clip must
    /// stop (it belongs to another chat); the deck is then empty.
    pub fn shown(&mut self, conversation: Option<&str>) -> bool {
        let stop = self.clip.as_ref().is_some_and(|(c, _)| Some(c.as_str()) != conversation);
        if stop {
            self.clip = None;
        }
        stop
    }

    /// The clip stopped (ended, or the player was closed).
    pub fn stop(&mut self) {
        self.clip = None;
    }
}

/// `m:ss`, or `h:mm:ss` from an hour (as Mouser's player writes it).
pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// `elapsed / total`, as the player shows it.
pub fn times(position_ms: u64, duration_ms: u64) -> String {
    format!("{} / {}", clock(position_ms.min(duration_ms)), clock(duration_ms))
}

/// Where a click at `fraction` (0 to 1) of the seek bar goes, in milliseconds.
pub fn seek_ms(fraction: f64, duration_ms: u64) -> u64 {
    let f = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.0 };
    (f * duration_ms as f64).round() as u64
}

/// How far along `position_ms` is, 0 to 1 (for the seek bar).
pub fn progress(position_ms: u64, duration_ms: u64) -> f64 {
    if duration_ms == 0 { 0.0 } else { (position_ms as f64 / duration_ms as f64).clamp(0.0, 1.0) }
}

/// How long a WAV or MP3 file plays, read from its headers (no decoding, so every shell can
/// show the total before the clip loads). `None` when it can't tell.
pub fn duration_ms(bytes: &[u8]) -> Option<u64> {
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") { wav_ms(bytes) } else { mp3_ms(bytes) }
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The `data` chunk's length over the `fmt ` chunk's byte rate.
fn wav_ms(b: &[u8]) -> Option<u64> {
    let (mut at, mut byte_rate, mut data) = (12usize, None, None);
    while at + 8 <= b.len() {
        let size = le32(b, at + 4)? as usize;
        match &b[at..at + 4] {
            b"fmt " => byte_rate = le32(b, at + 16),
            // A streamed WAV may say 0 or more than it has: count what is there.
            b"data" => data = Some(size.min(b.len().saturating_sub(at + 8))),
            _ => {}
        }
        at = at.checked_add(8 + size + (size & 1))?;
    }
    let rate = byte_rate.filter(|r| *r > 0)?;
    Some(data? as u64 * 1000 / rate as u64)
}

/// Walks the MPEG audio frames (layer III, after any ID3v2 tag), adding up their samples.
fn mp3_ms(b: &[u8]) -> Option<u64> {
    const V1: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
    const V2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    let mut at = 0usize;
    if b.starts_with(b"ID3") && b.len() >= 10 {
        let size = b[6..10].iter().fold(0usize, |n, x| (n << 7) | (*x as usize & 0x7f));
        at = 10 + size + if b[5] & 0x10 != 0 { 10 } else { 0 };
    }
    let (mut samples, mut rate, mut frames) = (0u64, 0u32, 0u32);
    while at + 4 <= b.len() {
        let h = &b[at..at + 4];
        let version = (h[1] >> 3) & 3; // 0: 2.5, 2: 2, 3: 1
        let layer = (h[1] >> 1) & 3; // 1: III
        let br = (h[2] >> 4) as usize;
        let sr = ((h[2] >> 2) & 3) as usize;
        if h[0] != 0xff || h[1] & 0xe0 != 0xe0 || version == 1 || layer != 1 || br == 0 || br == 15 || sr == 3 {
            at += 1;
            continue;
        }
        let (kbps, hz, spf, k) = match version {
            3 => (V1[br], [44100, 48000, 32000][sr], 1152, 144),
            2 => (V2[br], [22050, 24000, 16000][sr], 576, 72),
            _ => (V2[br], [11025, 12000, 8000][sr], 576, 72),
        };
        let len = (k * kbps * 1000 / hz + ((h[2] >> 1) & 1) as u32) as usize;
        samples += spf;
        rate = hz;
        frames += 1;
        at += len.max(1);
    }
    (frames > 0 && rate > 0).then(|| samples * 1000 / rate as u64)
}

/// A decoded sound: interleaved 16-bit samples.
#[cfg(feature = "decode")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

#[cfg(feature = "decode")]
impl Pcm {
    /// Frames (one sample per channel).
    pub fn frames(&self) -> u64 {
        (self.samples.len() / self.channels.max(1) as usize) as u64
    }

    pub fn duration_ms(&self) -> u64 {
        self.frames() * 1000 / self.rate.max(1) as u64
    }
}

/// Decodes a WAV or MP3 file (`name` gives the type when the bytes don't). At most two
/// channels are kept.
#[cfg(feature = "decode")]
pub fn decode(bytes: Vec<u8>, name: &str) -> crate::Result<Pcm> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    use crate::error::{CoreError, ErrorKind};
    let bad = |e: &dyn std::fmt::Display| CoreError::new(ErrorKind::BadRequest, format!("{name} can't be played: {e}"));
    let mut hint = Hint::new();
    if let Some((_, ext)) = name.rsplit_once('.') {
        hint.with_extension(&ext.to_ascii_lowercase());
    }
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).map_err(|e| bad(&e))?;
    let mut format = probed.format;
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).ok_or_else(|| bad(&"no sound in it"))?;
    let id = track.id;
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).map_err(|e| bad(&e))?;
    let mut pcm = Pcm { rate: track.codec_params.sample_rate.unwrap_or(0), channels: 0, samples: Vec::new() };
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => return Err(bad(&e)),
        };
        if packet.track_id() != id {
            continue;
        }
        let buf = match decoder.decode(&packet) {
            Ok(b) => b,
            // A damaged frame is skipped, as players do.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(bad(&e)),
        };
        let spec = *buf.spec();
        let n = spec.channels.count();
        let mut sb = SampleBuffer::<i16>::new(buf.capacity() as u64, spec);
        sb.copy_interleaved_ref(buf);
        pcm.rate = spec.rate;
        let keep = n.min(2);
        pcm.channels = keep as u16;
        if n == keep {
            pcm.samples.extend_from_slice(sb.samples());
        } else {
            pcm.samples.extend(sb.samples().chunks(n).flat_map(|f| f[..keep].to_vec()));
        }
    }
    if pcm.channels == 0 || pcm.rate == 0 {
        return Err(bad(&"no sound in it"));
    }
    Ok(pcm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_clip_at_a_time_and_it_stops_when_the_chat_changes() {
        let mut d = Deck::default();
        assert_eq!(d.start("c1", "m1#a0"), None);
        assert!(d.is("c1", "m1#a0"));
        assert_eq!(d.start("c1", "m1#a0"), None, "the same clip again stops nothing");
        assert_eq!(d.start("c1", "m2#a0"), Some(("c1".into(), "m1#a0".into())), "the other clip stops");
        assert!(!d.shown(Some("c1")), "the same chat keeps playing");
        assert!(d.shown(Some("c2")));
        assert_eq!(d.clip(), None);
        d.start("c2", "x#a0");
        assert!(d.shown(None), "no chat on screen stops it too");
        d.start("c2", "x#a0");
        d.stop();
        assert!(!d.shown(Some("c3")), "nothing to stop");
    }

    #[test]
    fn times_and_the_seek_bar() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(61_999), "1:01");
        assert_eq!(clock(3_725_000), "1:02:05");
        assert_eq!(times(5_000, 3_000), "0:03 / 0:03");
        assert_eq!(seek_ms(0.5, 10_000), 5_000);
        assert_eq!(seek_ms(-1.0, 10_000), 0);
        assert_eq!(seek_ms(2.0, 10_000), 10_000);
        assert_eq!(seek_ms(f64::NAN, 10_000), 0);
        assert_eq!(progress(2_500, 10_000), 0.25);
        assert_eq!(progress(1, 0), 0.0);
    }

    #[test]
    fn the_length_comes_from_the_headers() {
        assert_eq!(duration_ms(&crate::demo::tone_wav(2500, 440)), Some(2500));
        // Ten MPEG-1 layer III frames, 128 kbit/s at 44.1 kHz, behind an ID3v2 tag.
        let mut mp3 = b"ID3\x04\0\0\0\0\0\x05hello".to_vec();
        for _ in 0..10 {
            let mut frame = vec![0u8; 417];
            frame[..4].copy_from_slice(&[0xff, 0xfb, 0x90, 0x00]);
            mp3.extend(frame);
        }
        assert_eq!(duration_ms(&mp3), Some(11520 * 1000 / 44100));
        assert_eq!(duration_ms(b"plain text"), None);
    }

    #[cfg(feature = "decode")]
    #[test]
    fn a_wav_decodes_to_its_samples_and_length() {
        let wav = crate::demo::tone_wav(1500, 440);
        let pcm = decode(wav, "tone.wav").unwrap();
        assert_eq!((pcm.rate, pcm.channels, pcm.duration_ms()), (8000, 1, 1500));
        assert!(pcm.samples.iter().any(|s| s.unsigned_abs() > 1000), "a tone, not silence");
        assert!(decode(b"not audio".to_vec(), "x.wav").is_err());
    }
}
