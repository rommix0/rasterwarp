//! Playing a sound file on the speakers. The callback reads from the decoded samples at
//! its own position; each canvas frame the engine tells it where the playhead is, and
//! it jumps there when it has drifted more than [`SYNC`].

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::RATE;
use super::input::describe;

/// How far, in seconds, the speakers may drift from the playhead before they jump.
pub const SYNC: f64 = 0.04;

/// What the speaker callback reads and the engine sets.
pub struct Shared {
    /// Interleaved stereo at [`RATE`].
    samples: Arc<[i16]>,
    /// The next stereo frame to play.
    position: AtomicU64,
    playing: AtomicBool,
    looping: AtomicBool,
    /// The volume's f32 bits.
    volume: AtomicU32,
    failure: Mutex<Option<String>>,
}

impl Shared {
    /// Stopped, at the start, at full volume.
    pub fn new(samples: Arc<[i16]>) -> Shared {
        Shared {
            samples,
            position: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            looping: AtomicBool::new(false),
            volume: AtomicU32::new(1.0f32.to_bits()),
            failure: Mutex::new(None),
        }
    }

    /// The next stereo frame the speakers will play.
    pub fn position(&self) -> u64 {
        self.position.load(Ordering::Relaxed)
    }

    /// Follows the playhead at `seconds`: jumps there if the speakers were stopped (so a
    /// scrub or a start lands exactly) or are more than [`SYNC`] away, and takes the
    /// playing state, looping and volume.
    pub fn follow(&self, seconds: f64, playing: bool, looping: bool, volume: f32) {
        let wanted = (seconds.max(0.0) * f64::from(RATE)).round() as u64;
        let now = self.position();
        let stopped = !self.playing.load(Ordering::Relaxed);
        if stopped || now.abs_diff(wanted) as f64 > SYNC * f64::from(RATE) {
            self.position.store(wanted, Ordering::Relaxed);
        }
        self.playing.store(playing, Ordering::Relaxed);
        self.looping.store(looping, Ordering::Relaxed);
        self.volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Fills `out` (interleaved stereo f32) with what plays next.
    pub fn fill(&self, out: &mut [f32]) {
        out.fill(0.0);
        if !self.playing.load(Ordering::Relaxed) {
            return;
        }
        let volume = f32::from_bits(self.volume.load(Ordering::Relaxed)) / 32768.0;
        let looping = self.looping.load(Ordering::Relaxed);
        let total = (self.samples.len() / 2) as u64;
        let mut at = self.position();
        for frame in out.as_chunks_mut::<2>().0 {
            if at >= total {
                if looping && total > 0 {
                    at = 0;
                } else {
                    break;
                }
            }
            let i = at as usize * 2;
            frame[0] = f32::from(self.samples[i]) * volume;
            frame[1] = f32::from(self.samples[i + 1]) * volume;
            at += 1;
        }
        self.position.store(at, Ordering::Relaxed);
    }
}

/// A sound file playing on an output device.
pub struct Speakers {
    _stream: cpal::Stream,
    shared: Arc<Shared>,
}

impl Speakers {
    /// Opens the output device called `device` (`""` for the system default) to play
    /// `samples`. It starts stopped; [`Speakers::follow`] sets it going.
    pub fn open(device: &str, samples: Arc<[i16]>) -> Result<Speakers> {
        let host = cpal::default_host();
        let found = if device.is_empty() {
            host.default_output_device()
        } else {
            host.output_devices()
                .map_err(|err| anyhow!(describe(&err)))?
                .find(|d| d.to_string() == device)
        };
        let found = found.ok_or_else(|| anyhow!("no output device"))?;
        let shared = Arc::new(Shared::new(samples));
        let (reader, failure) = (shared.clone(), shared.clone());
        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: RATE,
            buffer_size: cpal::BufferSize::Default,
        };
        let stream = found
            .build_output_stream::<f32, _, _>(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| reader.fill(data),
                move |err: cpal::Error| {
                    if err.kind() != cpal::ErrorKind::Xrun
                        && let Ok(mut slot) = failure.failure.lock()
                    {
                        *slot = Some(describe(&err));
                    }
                },
                None,
            )
            .map_err(|err| anyhow!(describe(&err)))?;
        stream.play().map_err(|err| anyhow!(describe(&err)))?;
        Ok(Speakers {
            _stream: stream,
            shared,
        })
    }

    /// See [`Shared::follow`].
    pub fn follow(&self, seconds: f64, playing: bool, looping: bool, volume: f32) {
        self.shared.follow(seconds, playing, looping, volume);
    }

    /// Why the speakers stopped, if they did.
    pub fn failure(&self) -> Option<String> {
        self.shared
            .failure
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared(frames: i16) -> Shared {
        let samples: Vec<i16> = (0..frames).flat_map(|i| [i * 100, -(i * 100)]).collect();
        Shared::new(samples.into())
    }

    #[test]
    fn playing_copies_samples_and_moves_on() {
        let s = shared(4);
        s.follow(0.0, true, false, 1.0);
        let mut out = [9.0; 4];
        s.fill(&mut out);
        assert_eq!(out, [0.0, 0.0, 100.0 / 32768.0, -100.0 / 32768.0]);
        assert_eq!(s.position(), 2);
    }

    #[test]
    fn stopped_volume_and_the_end_are_silent() {
        let s = shared(4);
        let mut out = [9.0; 4];
        s.follow(0.0, false, false, 1.0);
        s.fill(&mut out);
        assert_eq!(out, [0.0; 4], "not playing");
        s.follow(0.0, true, false, 0.0);
        s.fill(&mut out);
        assert_eq!(out, [0.0; 4], "volume 0");
        s.follow(0.0, false, false, 1.0);
        s.follow(3.0 / 48_000.0, true, false, 1.0);
        let mut out = [9.0; 6];
        s.fill(&mut out);
        assert_eq!(out[2..], [0.0; 4], "silence after the last frame");
    }

    #[test]
    fn looping_wraps_to_the_start() {
        let s = shared(4);
        s.follow(3.0 / 48_000.0, true, true, 1.0);
        let mut out = [9.0; 4];
        s.fill(&mut out);
        assert_eq!(out, [300.0 / 32768.0, -300.0 / 32768.0, 0.0, 0.0]);
    }

    #[test]
    fn small_drift_is_left_and_large_drift_jumps() {
        let s = Shared::new(vec![0i16; 48_000 * 2].into());
        s.follow(0.0, true, false, 1.0);
        let mut out = vec![0.0; 960 * 2];
        s.fill(&mut out); // the speakers are 20 ms in
        s.follow(0.0, true, false, 1.0);
        assert_eq!(s.position(), 960, "20 ms off is within SYNC: left alone");
        s.follow(0.5, true, false, 1.0);
        assert_eq!(s.position(), 24_000, "a scrub jumps");
    }
}
