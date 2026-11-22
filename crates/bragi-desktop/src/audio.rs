//! The desktop's sound output for the audio files in a chat, after Mouser's player
//! (`mouser-media`): `cpal` plays the decoded samples and its callback is the clock (the
//! position is what the sound card has taken). With no sound card, or `BRAGI_AUDIO=null` (the
//! headless scenario), a wall clock runs instead and nothing is heard, so the player still
//! works. Which clip plays, and when it stops, is `bragi_core::audio::Deck`'s call.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use bragi_core::audio::Pcm;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const NO_SEEK: u64 = u64::MAX;

/// State shared with the audio callback.
struct Shared {
    pcm: Arc<Pcm>,
    /// The play position in source frames (f64 bits), written by the callback.
    at: AtomicU64,
    /// A seek for the callback to take (source frames), or [`NO_SEEK`].
    seek: AtomicU64,
    paused: AtomicBool,
    ended: AtomicBool,
}

/// Runs while playing, frozen while paused (Mouser's `WallClock`), for output without a device.
#[derive(Debug, Clone, Copy)]
struct Wall {
    base_ms: f64,
    since: Option<Instant>,
}

impl Wall {
    fn at(&self) -> f64 {
        self.base_ms + self.since.map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0)
    }
}

/// One clip, playing or paused.
pub struct Player {
    pub conversation: String,
    pub file: String,
    pub duration_ms: u64,
    /// What makes the sound: the audio host's name, or "no audio device" (silent).
    pub output: String,
    shared: Arc<Shared>,
    stream: Option<cpal::Stream>,
    wall: Wall,
}

impl Player {
    /// Opens the output and starts playing `pcm` from the start (silently on a wall clock when
    /// `BRAGI_AUDIO=null`).
    pub fn open(conversation: &str, file: &str, pcm: Pcm) -> Self {
        Self::open_on(conversation, file, pcm, !std::env::var("BRAGI_AUDIO").is_ok_and(|v| v == "null"))
    }

    /// [`open`](Self::open), on the sound card when `device`, else on a wall clock.
    pub fn open_on(conversation: &str, file: &str, pcm: Pcm, device: bool) -> Self {
        let pcm = Arc::new(pcm);
        let shared = Arc::new(Shared {
            pcm: pcm.clone(),
            at: AtomicU64::new(0f64.to_bits()),
            seek: AtomicU64::new(NO_SEEK),
            paused: AtomicBool::new(false),
            ended: AtomicBool::new(false),
        });
        let (stream, output) = if !device {
            (None, "no audio device".to_string())
        } else {
            match start(shared.clone()) {
                Ok((s, host)) => (Some(s), host.to_string()),
                Err(e) => {
                    eprintln!("bragi: audio output: {e}; playing silently");
                    (None, "no audio device".to_string())
                }
            }
        };
        Self {
            conversation: conversation.to_string(),
            file: file.to_string(),
            duration_ms: pcm.duration_ms(),
            output,
            wall: Wall { base_ms: 0.0, since: stream.is_none().then(Instant::now) },
            shared,
            stream,
        }
    }

    pub fn playing(&self) -> bool {
        !self.shared.paused.load(Ordering::Relaxed) && !self.ended()
    }

    /// It played to the end.
    pub fn ended(&self) -> bool {
        match &self.stream {
            Some(_) => self.shared.ended.load(Ordering::Relaxed),
            None => self.wall.at() >= self.duration_ms as f64,
        }
    }

    pub fn position_ms(&self) -> u64 {
        let ms = match &self.stream {
            Some(_) => {
                let pending = self.shared.seek.load(Ordering::Relaxed);
                let frames = if pending != NO_SEEK { pending as f64 } else { f64::from_bits(self.shared.at.load(Ordering::Relaxed)) };
                frames * 1000.0 / self.shared.pcm.rate.max(1) as f64
            }
            None => self.wall.at(),
        };
        (ms.max(0.0) as u64).min(self.duration_ms)
    }

    /// Play or pause; at the end, Play starts again from the top.
    pub fn toggle(&mut self) {
        if self.ended() {
            self.seek(0);
            self.set_paused(false);
        } else {
            self.set_paused(self.playing());
        }
    }

    fn set_paused(&mut self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
        if self.stream.is_none() {
            if paused {
                self.wall = Wall { base_ms: self.wall.at().min(self.duration_ms as f64), since: None };
            } else if self.wall.since.is_none() {
                self.wall.since = Some(Instant::now());
            }
        }
    }

    pub fn seek(&mut self, ms: u64) {
        let ms = ms.min(self.duration_ms);
        self.shared.ended.store(false, Ordering::Relaxed);
        self.shared.seek.store(ms * self.shared.pcm.rate as u64 / 1000, Ordering::Relaxed);
        if self.stream.is_none() {
            self.wall = Wall { base_ms: ms as f64, since: self.wall.since.map(|_| Instant::now()) };
        }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if let Some(s) = &self.stream {
            let _ = s.pause();
        }
    }
}

/// The default output device's stream, playing `shared` (resampled to the device's rate,
/// linear interpolation, mono to every channel).
fn start(shared: Arc<Shared>) -> Result<(cpal::Stream, &'static str), String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no audio output device")?;
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    let stream = match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config, shared),
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, shared),
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, shared),
        cpal::SampleFormat::I32 => build::<i32>(&device, &config, shared),
        other => return Err(format!("unsupported output format {other:?}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, host.id().name()))
}

fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    s: Arc<Shared>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels.max(1) as usize;
    let step = s.pcm.rate as f64 / config.sample_rate.0.max(1) as f64;
    let src = s.pcm.channels.max(1) as usize;
    let frames = s.pcm.frames() as usize;
    let mut pos = 0f64;
    let sample = move |pcm: &Pcm, i: usize, c: usize| pcm.samples.get(i * src + c.min(src - 1)).map_or(0.0, |v| *v as f32 / 32768.0);
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let seek = s.seek.swap(NO_SEEK, Ordering::Relaxed);
                if seek != NO_SEEK {
                    pos = seek as f64;
                }
                let silent = s.paused.load(Ordering::Relaxed) || s.ended.load(Ordering::Relaxed);
                for frame in data.chunks_mut(channels) {
                    let i = pos as usize;
                    if silent || i >= frames {
                        frame.iter_mut().for_each(|x| *x = T::from_sample(0.0));
                        continue;
                    }
                    let f = (pos - i as f64) as f32;
                    for (c, out) in frame.iter_mut().enumerate() {
                        let (a, b) = (sample(&s.pcm, i, c), sample(&s.pcm, (i + 1).min(frames - 1), c));
                        *out = T::from_sample(a + (b - a) * f);
                    }
                    pos += step;
                }
                if !silent && pos as usize >= frames {
                    pos = frames as f64;
                    s.ended.store(true, Ordering::Relaxed);
                }
                s.at.store(pos.to_bits(), Ordering::Relaxed);
            },
            |e| eprintln!("bragi: audio output: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone() -> Pcm {
        bragi_core::audio::decode(bragi_core::demo::tone_wav(1000, 440), "t.wav").unwrap()
    }

    #[test]
    fn without_a_device_a_wall_clock_plays_pauses_and_seeks() {
        let mut p = Player::open_on("c", "m#a0", tone(), false);
        assert_eq!((p.duration_ms, p.output.as_str()), (1000, "no audio device"));
        assert!(p.playing());
        std::thread::sleep(std::time::Duration::from_millis(120));
        p.toggle();
        let at = p.position_ms();
        assert!(!p.playing() && (100..400).contains(&at), "{at}");
        std::thread::sleep(std::time::Duration::from_millis(80));
        assert_eq!(p.position_ms(), at, "paused holds");
        p.seek(900);
        assert_eq!(p.position_ms(), 900);
        p.toggle();
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(p.ended() && !p.playing() && p.position_ms() == 1000);
        p.toggle();
        assert!(p.playing() && p.position_ms() < 100, "Play at the end starts again");
    }
}
