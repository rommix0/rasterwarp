//! Live sound through cpal (WASAPI on Windows): an input device (microphone, line-in),
//! or an output device's loopback ("what the PC is playing"). The stream's callback
//! mixes to mono and hands blocks over a channel; the app analyses them.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

/// Silence longer than this is filled with zeros (loopback sends nothing while the PC
/// plays nothing).
const GAP: Duration = Duration::from_millis(50);
/// At most this much silence is filled at once.
const MAX_FILL: Duration = Duration::from_secs(1);

/// Which kind of live source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveKind {
    /// An input device.
    Input,
    /// An output device's loopback.
    Loopback,
}

fn names(
    devices: Result<impl Iterator<Item = cpal::Device>, cpal::Error>,
) -> Result<Vec<String>, String> {
    devices
        .map(|list| list.map(|d| d.to_string()).collect())
        .map_err(|err| err.to_string())
}

/// The input devices' names.
pub fn input_devices() -> Result<Vec<String>, String> {
    names(cpal::default_host().input_devices())
}

/// The output devices' names (also the loopback sources).
pub fn output_devices() -> Result<Vec<String>, String> {
    names(cpal::default_host().output_devices())
}

/// A message for a device error.
pub fn describe(err: &cpal::Error) -> String {
    match err.kind() {
        cpal::ErrorKind::DeviceBusy => "Device in use".into(),
        cpal::ErrorKind::DeviceNotAvailable => "Device gone".into(),
        _ => err.to_string(),
    }
}

/// `data` (interleaved, `channels` per frame) averaged to one channel.
pub fn mono(data: &[f32], channels: usize) -> Vec<f32> {
    let channels = channels.max(1);
    data.chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// How many zeros to feed for the silence since `last`: none for a gap shorter than
/// 50 ms (a late block), else the gap's length, at most a second.
pub fn silence_to_fill(last: Instant, now: Instant, rate: u32) -> usize {
    let gap = now.saturating_duration_since(last);
    if gap < GAP {
        return 0;
    }
    (gap.min(MAX_FILL).as_secs_f64() * f64::from(rate)).round() as usize
}

/// An open live stream.
pub struct Live {
    _stream: cpal::Stream,
    rate: u32,
    blocks: Receiver<Vec<f32>>,
    failure: Arc<Mutex<Option<String>>>,
    /// When samples (or filled silence) last came.
    last: Instant,
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    send: mpsc::Sender<Vec<f32>>,
    failure: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels);
    device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let floats: Vec<f32> = data.iter().map(|s| s.to_sample::<f32>()).collect();
            let _ = send.send(mono(&floats, channels));
        },
        move |err: cpal::Error| {
            if err.kind() != cpal::ErrorKind::Xrun
                && let Ok(mut slot) = failure.lock()
            {
                *slot = Some(describe(&err));
            }
        },
        None,
    )
}

impl Live {
    /// Opens the device called `name` and starts listening.
    pub fn open(kind: LiveKind, name: &str) -> Result<Live> {
        let host = cpal::default_host();
        let devices = match kind {
            LiveKind::Input => host.input_devices(),
            LiveKind::Loopback => host.output_devices(),
        }
        .map_err(|err| anyhow!(describe(&err)))?;
        let device = devices
            .into_iter()
            .find(|d| d.to_string() == name)
            .ok_or_else(|| anyhow!("{name} isn't connected"))?;
        let supported = match kind {
            LiveKind::Input => device.default_input_config(),
            LiveKind::Loopback => device.default_output_config(),
        }
        .map_err(|err| anyhow!(describe(&err)))?;
        let config = supported.config();
        let rate = config.sample_rate;
        let (send, blocks) = mpsc::channel();
        let failure = Arc::new(Mutex::new(None));
        let f = failure.clone();
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config, send, f),
            SampleFormat::I16 => build::<i16>(&device, config, send, f),
            SampleFormat::I32 => build::<i32>(&device, config, send, f),
            SampleFormat::U8 => build::<u8>(&device, config, send, f),
            other => Err(cpal::Error::with_message(
                cpal::ErrorKind::UnsupportedConfig,
                format!("unsupported sample format {other:?}"),
            )),
        }
        .map_err(|err| match kind {
            LiveKind::Loopback => {
                anyhow!(
                    "This device can't loop back what it plays: {}",
                    describe(&err)
                )
            }
            LiveKind::Input => anyhow!(describe(&err)),
        })?;
        stream.play().map_err(|err| anyhow!(describe(&err)))?;
        Ok(Live {
            _stream: stream,
            rate,
            blocks,
            failure,
            last: Instant::now(),
        })
    }

    /// The device's rate: the rate [`Live::drain`]'s samples are at.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Mono samples since the last call; when nothing came for more than 50 ms, zeros
    /// for the silence.
    pub fn drain(&mut self, now: Instant) -> Vec<f32> {
        let mut out: Vec<f32> = Vec::new();
        for block in self.blocks.try_iter() {
            out.extend(block);
        }
        if !out.is_empty() {
            self.last = now;
            return out;
        }
        let fill = silence_to_fill(self.last, now, self.rate);
        if fill > 0 {
            self.last = now;
        }
        vec![0.0; fill]
    }

    /// Why the stream stopped, if it did (the device was unplugged, say).
    pub fn failure(&self) -> Option<String> {
        self.failure.lock().ok().and_then(|slot| slot.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn channels_mix_to_mono() {
        assert_eq!(mono(&[1.0, 0.0, 0.5, 0.5], 2), [0.5, 0.5]);
        assert_eq!(mono(&[0.25, -0.25], 1), [0.25, -0.25]);
        assert_eq!(mono(&[1.0, 1.0, 1.0, 0.0, 0.0, 0.0], 3), [1.0, 0.0]);
    }

    #[test]
    fn short_gaps_are_left_alone_and_long_ones_filled() {
        let t0 = Instant::now();
        assert_eq!(
            silence_to_fill(t0, t0 + Duration::from_millis(30), 48_000),
            0
        );
        assert_eq!(
            silence_to_fill(t0, t0 + Duration::from_millis(100), 48_000),
            4_800
        );
        assert_eq!(
            silence_to_fill(t0, t0 + Duration::from_secs(60), 48_000),
            48_000,
            "at most a second is filled at once"
        );
    }

    #[test]
    fn listing_devices_works_with_or_without_any() {
        // Lists only; opening a stream in a test would grab real hardware.
        let _ = input_devices();
        let _ = output_devices();
    }
}
