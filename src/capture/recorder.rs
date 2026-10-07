//! One recording: decides which frames to capture, reads them back from the GPU and
//! feeds them to the encoder thread.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};

use super::encode::{Encoder, Frame, VideoFormat};
use super::readback::{Readback, staging_bytes};
use super::{CaptureMode, FrameClock, file_name, unused_path};

/// What the panel chose before pressing Record.
#[derive(Clone, Debug)]
pub struct RecordSettings {
    pub format: VideoFormat,
    pub mode: CaptureMode,
    pub folder: PathBuf,
    /// Stop automatically after this many seconds of video (0 = stop by hand).
    pub stop_after: f32,
}

/// Progress shown while recording.
#[derive(Clone, Debug)]
pub struct RecordStatus {
    pub path: PathBuf,
    /// Seconds of video recorded so far.
    pub seconds: f64,
    /// Frames handed to the encoder.
    pub frames: u64,
    /// Frames lost because the encoder or the GPU readback fell behind (real-time only).
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
    path: PathBuf,
    started: Instant,
    frames: u64,
    dropped: u64,
    /// Frame buffers ready for reuse.
    spare: Vec<Vec<u8>>,
}

impl Recorder {
    /// Opens a new file in the settings' folder and starts encoding. `time` is the
    /// current animation time.
    pub fn start(
        device: &wgpu::Device,
        settings: &RecordSettings,
        canvas: (u32, u32),
        time: f64,
    ) -> Result<Self> {
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
        let encoder = Encoder::start(&path, settings.format, canvas)?;
        Ok(Self {
            encoder,
            readback: Readback::new(device, canvas),
            clock: FrameClock::new(settings.mode, time),
            mode: settings.mode,
            stop_after: settings.stop_after,
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

    /// Call once per frame before rendering: hands frames that have come back from the
    /// GPU to the encoder. An error means the encoder has stopped.
    pub fn pump(&mut self, device: &wgpu::Device) -> Result<()> {
        self.deliver(device, false)
    }

    /// Captures the frame rendered at animation time `time` if one is due: `draw` must
    /// draw the capture composite into the given view. In offline mode this waits for
    /// a free staging buffer; in real-time mode a busy ring drops the frame.
    pub fn capture(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        time: f64,
        draw: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView),
    ) -> Result<()> {
        let Some(pts) = self.clock.frame_due(time) else {
            return Ok(());
        };
        draw(encoder, self.readback.view());
        if self.readback.copy(encoder, pts) {
            return Ok(());
        }
        match self.mode {
            CaptureMode::RealTime => self.dropped += 1,
            CaptureMode::Offline => {
                self.deliver(device, true)?;
                if !self.readback.copy(encoder, pts) {
                    bail!("no capture buffer came free");
                }
            }
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
            if let Err(err) = self.deliver(device, true) {
                return Err(self.encoder.finish().err().unwrap_or(err));
            }
        }
        let status = self.status();
        let written = self.encoder.finish()?;
        log::info!("recorded {written} frames to {}", status.path.display());
        Ok(status)
    }

    /// Sends finished readbacks to the encoder. Waits for the GPU when `wait` is set;
    /// then sends block too, so no frame is dropped.
    fn deliver(&mut self, device: &wgpu::Device, wait: bool) -> Result<()> {
        while let Some(buffer) = self.encoder.recycled_buffer() {
            self.spare.push(buffer);
        }
        let spare = &mut self.spare;
        let frames = self
            .readback
            .collect(device, wait, || spare.pop().unwrap_or_default())?;
        let blocking = wait || self.mode == CaptureMode::Offline;
        for frame in frames {
            self.send(frame, blocking)?;
        }
        Ok(())
    }

    fn send(&mut self, frame: Frame, blocking: bool) -> Result<()> {
        if blocking {
            self.encoder.send(frame)?;
        } else if let Some(frame) = self.encoder.try_send(frame)? {
            self.dropped += 1;
            self.spare.push(frame.rgba);
            return Ok(());
        }
        self.frames += 1;
        Ok(())
    }
}
