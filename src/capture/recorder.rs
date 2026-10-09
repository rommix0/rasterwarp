//! One recording: reads captured canvas frames back from the GPU and feeds them to the
//! encoder thread.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::encode::{Encoder, Frame, VideoFormat};
use super::readback::{Readback, staging_bytes};
use super::{CaptureMode, FrameClock, file_name, unused_path};
use crate::rate::FrameRate;

/// What the panel chose before pressing Record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordSettings {
    pub format: VideoFormat,
    pub mode: CaptureMode,
    pub folder: PathBuf,
    /// Stop automatically after this many seconds of video (0 = stop by hand).
    pub stop_after: f32,
    /// Record and save stills with a transparent background: keyed levels and the
    /// background behind them become see-through.
    pub alpha: bool,
}

impl Default for RecordSettings {
    fn default() -> Self {
        Self {
            format: VideoFormat::Hevc,
            mode: CaptureMode::RealTime,
            folder: PathBuf::from("captures"),
            stop_after: 0.0,
            alpha: false,
        }
    }
}

impl RecordSettings {
    /// Why these settings can't record, if they can't.
    pub fn check(&self) -> Result<()> {
        if self.alpha && !self.format.carries_alpha() {
            bail!("HEVC can't carry alpha; choose ProRes 4444 or FFV1");
        }
        Ok(())
    }
}

/// Progress shown while recording.
#[derive(Clone, Debug)]
pub struct RecordStatus {
    pub path: PathBuf,
    /// Seconds of video recorded so far.
    pub seconds: f64,
    /// Frames handed to the encoder.
    pub frames: u64,
    /// Frames lost because the encoder fell behind (real-time only).
    pub dropped: u64,
    /// Offline only: seconds of video per second of wall time.
    pub speed: Option<f64>,
}

pub struct Recorder {
    encoder: Encoder,
    readback: Readback,
    clock: FrameClock,
    mode: CaptureMode,
    stop_after: f32,
    alpha: bool,
    rate: FrameRate,
    /// Whether the recording has a soundtrack.
    sound: bool,
    path: PathBuf,
    started: Instant,
    frames: u64,
    dropped: u64,
    /// Frame buffers ready for reuse.
    spare: Vec<Vec<u8>>,
}

impl Recorder {
    /// Opens a new file in the settings' folder and starts encoding at `rate` frames per
    /// second.
    pub fn start(
        device: &wgpu::Device,
        settings: &RecordSettings,
        canvas: (u32, u32),
        rate: FrameRate,
    ) -> Result<Self> {
        Self::start_with_sound(device, settings, canvas, rate, false)
    }

    /// Like [`Recorder::start`], with a soundtrack stream when `sound` is set; add to it
    /// with [`Recorder::add_sound`] after each captured frame.
    pub fn start_with_sound(
        device: &wgpu::Device,
        settings: &RecordSettings,
        canvas: (u32, u32),
        rate: FrameRate,
        sound: bool,
    ) -> Result<Self> {
        settings.check()?;
        if staging_bytes(canvas) > device.limits().max_buffer_size {
            bail!(
                "{}×{} is too large to record on this GPU; choose a smaller canvas",
                canvas.0,
                canvas.1
            );
        }
        std::fs::create_dir_all(&settings.folder)
            .with_context(|| format!("could not create {}", settings.folder.display()))?;
        let now = chrono::Local::now().naive_local();
        let path = unused_path(&settings.folder, &file_name(now, settings.format));
        let encoder = Encoder::start_with_sound(&path, settings.format, canvas, rate, sound)?;
        Ok(Self {
            encoder,
            readback: Readback::new(device, canvas),
            clock: FrameClock::new(rate),
            mode: settings.mode,
            stop_after: settings.stop_after,
            alpha: settings.alpha,
            rate,
            sound,
            path,
            started: Instant::now(),
            frames: 0,
            dropped: 0,
            spare: Vec::new(),
        })
    }

    pub fn mode(&self) -> CaptureMode {
        self.mode
    }

    /// Whether frames are captured with a transparent background.
    pub fn alpha(&self) -> bool {
        self.alpha
    }

    /// Whether this recording has a soundtrack.
    pub fn has_sound(&self) -> bool {
        self.sound
    }

    /// Stereo sound frames the frame just captured covers.
    pub fn sound_frames(&self) -> usize {
        sound_frames_for(self.rate, self.clock.frames() - 1)
    }

    /// Adds the soundtrack for the frame just captured.
    pub fn add_sound(&mut self, samples: Vec<i16>) -> Result<()> {
        if self.sound {
            self.encoder.send_sound(samples)?;
        }
        Ok(())
    }

    /// Call once per screen refresh, before drawing its canvas frames: hands frames that
    /// have come back from the GPU to the encoder. An error means the encoder has stopped.
    pub fn pump(&mut self, device: &wgpu::Device) -> Result<()> {
        self.deliver(device, false, self.mode == CaptureMode::Offline)
    }

    /// Captures the canvas frame just drawn as the next frame of the file: `draw` must
    /// draw the capture composite into the given view. When every staging buffer is
    /// still busy this waits for the GPU. Submit the frame's commands and call
    /// [`Recorder::after_submit`] before capturing the next frame.
    pub fn capture(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        draw: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView),
    ) -> Result<()> {
        let pts = self.clock.next_pts();
        draw(encoder, self.readback.view());
        if self.readback.copy(encoder, pts) {
            return Ok(());
        }
        // Waiting for the GPU is brief; only the encoder's queue may drop frames.
        self.deliver(device, true, self.mode == CaptureMode::Offline)?;
        if !self.readback.copy(encoder, pts) {
            bail!("no capture buffer came free");
        }
        Ok(())
    }

    /// Call after submitting the frame's commands.
    pub fn after_submit(&mut self) {
        self.readback.after_submit();
    }

    /// Whether the "stop after" length has been reached.
    pub fn finished_recording(&self) -> bool {
        self.clock.reached(self.stop_after)
    }

    pub fn status(&self) -> RecordStatus {
        let seconds = self.clock.seconds();
        RecordStatus {
            path: self.path.clone(),
            seconds,
            frames: self.frames,
            dropped: self.dropped,
            speed: (self.mode == CaptureMode::Offline)
                .then(|| seconds / self.started.elapsed().as_secs_f64().max(1e-3)),
        }
    }

    /// Waits for the frames still on the GPU, encodes them, closes the file, and returns
    /// the final status or the error that stopped the encoder.
    pub fn finish(mut self, device: &wgpu::Device) -> Result<RecordStatus> {
        while self.readback.is_busy() {
            if let Err(err) = self.deliver(device, true, true) {
                return Err(self.encoder.finish().err().unwrap_or(err));
            }
        }
        let status = self.status();
        let written = self.encoder.finish()?;
        log::info!("recorded {written} frames to {}", status.path.display());
        Ok(status)
    }

    /// Sends finished readbacks to the encoder. Waits for the GPU when `wait` is set, and
    /// for room in the encoder's queue when `block` is set (otherwise a full queue drops
    /// the frame).
    fn deliver(&mut self, device: &wgpu::Device, wait: bool, block: bool) -> Result<()> {
        while let Some(buffer) = self.encoder.recycled_buffer() {
            self.spare.push(buffer);
        }
        let spare = &mut self.spare;
        let frames = self
            .readback
            .collect(device, wait, || spare.pop().unwrap_or_default())?;
        for frame in frames {
            self.send(frame, block)?;
        }
        Ok(())
    }

    fn send(&mut self, frame: Frame, block: bool) -> Result<()> {
        if block {
            self.encoder.send(frame)?;
            self.frames += 1;
        } else {
            let rejected = self.encoder.try_send(frame)?;
            count_offer(
                rejected,
                &mut self.frames,
                &mut self.dropped,
                &mut self.spare,
            );
        }
        Ok(())
    }
}

/// Stereo sound frames (at 48 kHz) that video frame `n` covers at `rate`: frame `n`
/// runs from sample `round(n·48000/fps)` to `round((n+1)·48000/fps)`, so the sound
/// never drifts from the video.
pub fn sound_frames_for(rate: FrameRate, n: i64) -> usize {
    let at = |n: i64| -> i64 {
        let num = i128::from(rate.num);
        let scaled = i128::from(n) * 48_000 * i128::from(rate.den);
        ((scaled * 2 + num) / (num * 2)) as i64
    };
    (at(n + 1) - at(n)) as usize
}

/// Counts a frame offered to the encoder without waiting. `rejected` is the frame a full
/// queue handed back: it counts as dropped, and its buffer is kept in `spare` for reuse.
fn count_offer(
    rejected: Option<Frame>,
    frames: &mut u64,
    dropped: &mut u64,
    spare: &mut Vec<Vec<u8>>,
) {
    match rejected {
        Some(frame) => {
            *dropped += 1;
            spare.push(frame.rgba);
        }
        None => *frames += 1,
    }
}

#[cfg(test)]
mod tests {
    use super::super::encode::try_queue;
    use super::*;

    #[test]
    fn a_full_encoder_queue_drops_frames_and_keeps_their_buffers() {
        // The encoder never takes a frame, so the queue fills after one.
        let (queue, _encoder) = std::sync::mpsc::sync_channel(1);
        let (mut frames, mut dropped, mut spare) = (0, 0, Vec::new());
        for pts in 0..3 {
            let frame = Frame {
                pts,
                rgba: vec![pts as u8; 4],
            };
            let rejected = try_queue(&queue, frame).unwrap();
            count_offer(rejected, &mut frames, &mut dropped, &mut spare);
        }
        assert_eq!((frames, dropped), (1, 2));
        assert_eq!(spare, [vec![1; 4], vec![2; 4]]);
    }

    #[test]
    fn only_formats_with_an_alpha_channel_record_alpha() {
        let alpha = |format| RecordSettings {
            format,
            alpha: true,
            ..RecordSettings::default()
        };
        let err = alpha(VideoFormat::Hevc).check().unwrap_err();
        assert!(format!("{err:#}").contains("HEVC can't carry alpha"));
        assert!(alpha(VideoFormat::Ffv1).check().is_ok());
        assert!(alpha(VideoFormat::ProRes4444).check().is_ok());
        assert!(
            RecordSettings::default().check().is_ok(),
            "opaque HEVC records"
        );
        assert!(!RecordSettings::default().alpha, "alpha starts off");
    }

    #[test]
    fn sound_frames_per_video_frame_add_up_exactly() {
        let rate = FrameRate::ntsc(24); // 24000/1001
        let total: usize = (0..1000).map(|n| sound_frames_for(rate, n)).sum();
        let exact = (1000.0 * 48_000.0 * 1001.0 / 24_000.0_f64).round() as usize;
        assert_eq!(total, exact);
        assert_eq!(sound_frames_for(FrameRate::whole(30), 7), 1600);
    }
}
