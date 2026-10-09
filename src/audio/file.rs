//! Sound files: decoded once into 48 kHz stereo samples (for the speakers and the
//! soundtrack), and analysed once into tracks that links read at the playhead's time,
//! so the same moment of a song always gives the same signals.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use anyhow::{Context, Result, anyhow, bail};
use ff::format::sample::{Sample, Type};
use ff::software::resampling;
use ff::{ChannelLayout, frame};
use ffmpeg_next as ff;

use super::analysis::Analyzer;
use super::{RATE, Shaping};
use crate::video::{Budget, Reservation, describe_bytes};

/// Track values per second.
pub const TRACK_RATE: u32 = 1000;
/// Samples per track value.
const STEP: usize = (RATE / TRACK_RATE) as usize;
/// What the decoder writes: packed (interleaved) 16-bit.
const S16: Sample = Sample::I16(Type::Packed);

/// A file's analysis: Level, Bass, Mid and Treble (before gain) every millisecond, and
/// when each beat starts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tracks {
    values: Vec<[f32; 4]>,
    /// Onset times in seconds, indexed by `Beat::index`, each sorted.
    beats: [Vec<f64>; 3],
}

impl Tracks {
    /// Analyses interleaved stereo `samples` at [`RATE`].
    pub fn analyse(samples: &[i16], shaping: Shaping) -> Tracks {
        let mut analyzer = Analyzer::new(RATE, shaping);
        let mut tracks = Tracks {
            values: Vec::with_capacity(samples.len() / 2 / STEP + 1),
            beats: Default::default(),
        };
        for (i, &[left, right]) in samples.as_chunks::<2>().0.iter().enumerate() {
            let x = (f32::from(left) + f32::from(right)) / (2.0 * 32768.0);
            for (times, fired) in tracks.beats.iter_mut().zip(analyzer.push(x)) {
                if fired {
                    times.push(i as f64 / f64::from(RATE));
                }
            }
            if i % STEP == STEP - 1 {
                tracks.values.push(analyzer.values());
            }
        }
        tracks
    }

    /// Level, Bass, Mid and Treble at `seconds`, before gain; held at the first and
    /// last values outside the file.
    pub fn at(&self, seconds: f64) -> [f32; 4] {
        let Some(last) = self.values.len().checked_sub(1) else {
            return [0.0; 4];
        };
        let pos = (seconds * f64::from(TRACK_RATE)).clamp(0.0, last as f64);
        let i = pos.floor() as usize;
        let t = (pos - i as f64) as f32;
        let (a, b) = (self.values[i], self.values[(i + 1).min(last)]);
        std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
    }

    /// Which beats start in the span from `from` up to (not including) `to`. A span
    /// with `to < from` wrapped around the end of a looping file.
    pub fn beats_in(&self, from: f64, to: f64) -> [bool; 3] {
        self.beats.each_ref().map(|times| {
            let starts_at_or_after = |t: f64| times.partition_point(|&b| b < t);
            if to >= from {
                starts_at_or_after(from) < starts_at_or_after(to)
            } else {
                starts_at_or_after(from) < times.len() || starts_at_or_after(to) > 0
            }
        })
    }
}

/// A decoded, analysed sound file.
#[derive(Debug)]
pub struct Sound {
    /// The file's name, for the panel.
    pub name: String,
    /// Interleaved stereo samples at [`RATE`].
    pub samples: Arc<[i16]>,
    /// What links read at the playhead.
    pub tracks: Tracks,
    _memory: Reservation,
}

/// Memory a sound of `frames` stereo frames holds: its samples and its tracks.
fn bytes_for(frames: usize) -> u64 {
    let tracks = frames / STEP * std::mem::size_of::<[f32; 4]>();
    (frames * 4 + tracks) as u64
}

impl Sound {
    /// Charges `samples` (interleaved stereo at [`RATE`]) to `budget` and analyses them.
    pub fn new(
        name: String,
        samples: Vec<i16>,
        shaping: Shaping,
        budget: &Budget,
    ) -> Result<Sound> {
        let needed = bytes_for(samples.len() / 2);
        let Some(memory) = budget.take(needed) else {
            bail!(
                "This file needs {} of memory and only {} is free",
                describe_bytes(needed),
                describe_bytes(budget.available())
            );
        };
        let tracks = Tracks::analyse(&samples, shaping);
        Ok(Sound {
            name,
            samples: samples.into(),
            tracks,
            _memory: memory,
        })
    }

    /// Its length in stereo frames.
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    /// Its length in seconds.
    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / f64::from(RATE)
    }

    /// `frames` stereo frames starting at `from` seconds: wrapping to the start when
    /// `looping`, silence past the end when not.
    pub fn slice(&self, from: f64, frames: usize, looping: bool) -> Vec<i16> {
        let total = self.frames();
        let mut out = Vec::with_capacity(frames * 2);
        if total == 0 {
            out.resize(frames * 2, 0);
            return out;
        }
        let mut at = (from * f64::from(RATE)).round().max(0.0) as usize;
        if looping {
            at %= total;
        }
        for _ in 0..frames {
            if at >= total {
                if looping {
                    at = 0;
                } else {
                    out.extend([0, 0]);
                    continue;
                }
            }
            out.extend_from_slice(&self.samples[at * 2..at * 2 + 2]);
            at += 1;
        }
        out
    }
}

/// A fresh output frame with room for `capacity` stereo frames.
fn output_frame(capacity: usize) -> frame::Audio {
    let mut out = frame::Audio::new(S16, capacity, ChannelLayout::STEREO);
    out.set_rate(RATE);
    out
}

/// Resamples one decoded frame to [`RATE`] stereo onto the end of `samples`, building
/// the resampler from the first frame (a file's real format is only known then).
fn take(
    decoded: &mut frame::Audio,
    resampler: &mut Option<resampling::Context>,
    samples: &mut Vec<i16>,
) -> Result<()> {
    // Mono files decode with no layout, which the resampler refuses.
    if decoded.channel_layout().is_empty() {
        decoded.set_channel_layout(ChannelLayout::default(i32::from(decoded.channels())));
    }
    let r = match resampler {
        Some(r) => r,
        None => resampler.insert(resampling::Context::get(
            decoded.format(),
            decoded.channel_layout(),
            decoded.rate(),
            S16,
            ChannelLayout::STEREO,
            RATE,
        )?),
    };
    let capacity = (decoded.samples() as u64 * u64::from(RATE))
        .div_ceil(u64::from(decoded.rate().max(1))) as usize
        + 64;
    let mut out = output_frame(capacity);
    r.run(decoded, &mut out)?;
    samples.extend_from_slice(out.plane::<i16>(0));
    Ok(())
}

/// Decodes the first audio stream of `path` into interleaved stereo at [`RATE`].
/// `progress` goes 0..=1000; `cancel` stops it.
pub fn decode(path: &Path, progress: &AtomicU32, cancel: &AtomicBool) -> Result<Vec<i16>> {
    ff::init().context("could not initialise FFmpeg")?;
    let mut input = ff::format::input(path)?;
    let stream = input
        .streams()
        .best(ff::media::Type::Audio)
        .ok_or_else(|| anyhow!("no sound in it"))?;
    let index = stream.index();
    let mut decoder = ff::codec::context::Context::from_parameters(stream.parameters())?
        .decoder()
        .audio()?;
    let expected =
        input.duration().max(0) as f64 / f64::from(ff::ffi::AV_TIME_BASE) * f64::from(RATE);
    let mut samples: Vec<i16> = Vec::new();
    let mut resampler: Option<resampling::Context> = None;
    let mut decoded = frame::Audio::empty();
    for (stream, packet) in input.packets() {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        if stream.index() == index {
            decoder.send_packet(&packet)?;
            while decoder.receive_frame(&mut decoded).is_ok() {
                take(&mut decoded, &mut resampler, &mut samples)?;
            }
            if expected > 0.0 {
                let done = (samples.len() / 2) as f64 / expected;
                progress.store((done.min(1.0) * 999.0) as u32, Ordering::Relaxed);
            }
        }
    }
    decoder.send_eof()?;
    while decoder.receive_frame(&mut decoded).is_ok() {
        take(&mut decoded, &mut resampler, &mut samples)?;
    }
    if let Some(mut r) = resampler {
        loop {
            let mut out = output_frame(4096);
            r.flush(&mut out)?;
            if out.samples() == 0 {
                break;
            }
            samples.extend_from_slice(out.plane::<i16>(0));
        }
    }
    progress.store(1000, Ordering::Relaxed);
    Ok(samples)
}

/// The file name of `path`, for messages.
fn name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Decodes, charges and analyses `path`.
pub fn load(
    path: &Path,
    shaping: Shaping,
    budget: &Budget,
    progress: &AtomicU32,
    cancel: &AtomicBool,
) -> Result<Sound> {
    let name = name(path);
    let samples =
        decode(path, progress, cancel).with_context(|| format!("Couldn't read {name}"))?;
    Sound::new(name, samples, shaping, budget)
}

/// A file loading on a worker thread. Dropping it cancels the load.
pub struct Loading {
    /// The file being loaded.
    pub path: PathBuf,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    result: Receiver<Result<Sound>>,
}

impl Loading {
    /// Starts loading `path`.
    pub fn start(path: &Path, shaping: Shaping, budget: &Budget) -> Loading {
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (send, result) = mpsc::channel();
        let (owned, p, c, budget) = (
            path.to_path_buf(),
            progress.clone(),
            cancel.clone(),
            budget.clone(),
        );
        let spawned = std::thread::Builder::new()
            .name("sound loader".into())
            .spawn({
                let send = send.clone();
                move || {
                    let _ = send.send(load(&owned, shaping, &budget, &p, &c));
                }
            });
        if let Err(err) = spawned {
            let _ = send.send(Err(anyhow!(
                "could not start loading {}: {err}",
                name(path)
            )));
        }
        Loading {
            path: path.to_path_buf(),
            progress,
            cancel,
            result,
        }
    }

    /// How far it has got, 0 to 1.
    pub fn progress(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// The sound, or why it failed, once the load has finished.
    pub fn poll(&self) -> Option<Result<Sound>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(anyhow!(
                "the sound loader for {} stopped",
                name(&self.path)
            ))),
        }
    }
}

impl Drop for Loading {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// A loaded file's analysis being re-run with new shaping, on a worker thread.
pub struct Reanalysis {
    result: Receiver<Tracks>,
}

impl Reanalysis {
    /// Starts analysing `samples` with `shaping`.
    pub fn start(samples: Arc<[i16]>, shaping: Shaping) -> Reanalysis {
        let (send, result) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("sound analysis".into())
            .spawn(move || {
                let _ = send.send(Tracks::analyse(&samples, shaping));
            });
        if let Err(err) = spawned {
            log::warn!("could not start re-analysing the sound: {err}");
        }
        Reanalysis { result }
    }

    /// The new tracks, once ready.
    pub fn poll(&self) -> Option<Tracks> {
        self.result.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::{Budget, MEMORY_LIMIT};
    use std::f32::consts::TAU;

    /// Interleaved stereo: `count` 20 ms bursts of 1 kHz, `gap` seconds apart.
    fn clicks(count: usize, gap: f64) -> Vec<i16> {
        let frames = (count as f64 * gap * RATE as f64) as usize;
        let burst = (0.02 * RATE as f64) as usize;
        let every = (gap * RATE as f64) as usize;
        let mut out = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let v = if i % every < burst {
                ((TAU * 1000.0 * i as f32 / RATE as f32).sin() * 30000.0) as i16
            } else {
                0
            };
            out.extend([v, v]);
        }
        out
    }

    fn ramp(frames: usize) -> Vec<i16> {
        (0..frames)
            .flat_map(|i| [i as i16, (i as i16).wrapping_neg()])
            .collect()
    }

    #[test]
    fn tracks_hold_a_value_per_millisecond_and_find_the_clicks() {
        let tracks = Tracks::analyse(&clicks(4, 0.5), Shaping::default());
        assert_eq!(tracks.values.len(), 2000);
        let all = tracks.beats_in(0.0, 2.0);
        assert_eq!(all, [true, false, true], "1 kHz clicks are Any and Treble");
        let mut count = 0;
        let mut t = 0.0;
        while t < 2.0 {
            count += usize::from(tracks.beats_in(t, t + 1.0 / 60.0)[0]);
            t += 1.0 / 60.0;
        }
        assert_eq!(count, 4, "each click fires once across frame spans");
    }

    #[test]
    fn a_wrapped_span_covers_the_end_and_the_start() {
        let tracks = Tracks::analyse(&clicks(4, 0.5), Shaping::default());
        assert!(
            tracks.beats_in(1.99, 0.01)[0],
            "the first click, just after the wrap"
        );
        assert!(!tracks.beats_in(1.9, 1.95)[0]);
    }

    #[test]
    fn reading_between_values_interpolates_and_holds_at_the_ends() {
        let tracks = Tracks {
            values: vec![[0.0; 4], [1.0; 4]],
            beats: Default::default(),
        };
        assert_eq!(tracks.at(0.0005), [0.5; 4]);
        assert_eq!(tracks.at(-1.0), [0.0; 4]);
        assert_eq!(tracks.at(10.0), [1.0; 4]);
        assert_eq!(Tracks::default().at(1.0), [0.0; 4]);
    }

    #[test]
    fn slices_wrap_when_looping_and_fall_silent_when_not() {
        let budget = Budget::new(MEMORY_LIMIT);
        let sound = Sound::new("ramp".into(), ramp(100), Shaping::default(), &budget).unwrap();
        assert_eq!(sound.frames(), 100);
        let from = 98.0 / RATE as f64;
        assert_eq!(sound.slice(from, 4, true), [98, -98, 99, -99, 0, 0, 1, -1]);
        assert_eq!(sound.slice(from, 4, false), [98, -98, 99, -99, 0, 0, 0, 0]);
        assert_eq!(sound.slice(0.0, 2, false), [0, 0, 1, -1]);
    }

    #[test]
    fn a_sound_is_charged_to_the_budget() {
        let budget = Budget::new(MEMORY_LIMIT);
        let before = budget.available();
        let sound = Sound::new("ramp".into(), ramp(48_000), Shaping::default(), &budget).unwrap();
        assert!(before - budget.available() >= 48_000 * 4);
        drop(sound);
        assert_eq!(budget.available(), before);
        let tiny = Budget::new(1000);
        let err = Sound::new("big.wav".into(), ramp(48_000), Shaping::default(), &tiny)
            .expect_err("it doesn't fit");
        assert!(format!("{err:#}").starts_with("This file needs"), "{err:#}");
    }
}
