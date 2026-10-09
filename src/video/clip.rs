//! Clips: a video file decoded whole into memory on a worker thread, so every frame is
//! there at once for playing backwards, scrubbing and slit-scan.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use anyhow::{Context, Result, anyhow, bail};
use ffmpeg_next as ff;

use super::{Budget, Converter, Format, Frame, Pixels, Reservation, describe_bytes};

/// A clip's frame rate when the file doesn't give one.
pub const FALLBACK_FPS: f64 = 30.0;

/// A decoded clip.
#[derive(Debug)]
pub struct Clip {
    pub format: Format,
    /// The stream's average frame rate.
    pub fps: f64,
    pub frames: Vec<Frame>,
    /// The memory the frames take from the budget, given back when the clip goes.
    _memory: Reservation,
}

impl Clip {
    pub fn count(&self) -> u32 {
        self.frames.len() as u32
    }
}

/// The file name, for messages.
fn name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Decodes the first video stream of `path` into frames for a `canvas`-sized canvas.
/// `progress` counts up to 1000 as it goes; setting `cancel` stops it.
pub fn load(
    path: &Path,
    pixels: Pixels,
    canvas: (u32, u32),
    budget: &Budget,
    progress: &AtomicU32,
    cancel: &AtomicBool,
) -> Result<Clip> {
    let name = name(path);
    decode(path, &name, pixels, canvas, budget, progress, cancel)
        .with_context(|| format!("could not open {name}"))
}

#[allow(clippy::too_many_arguments)]
fn decode(
    path: &Path,
    name: &str,
    pixels: Pixels,
    canvas: (u32, u32),
    budget: &Budget,
    progress: &AtomicU32,
    cancel: &AtomicBool,
) -> Result<Clip> {
    ff::init().context("could not initialise FFmpeg")?;
    let mut input = ff::format::input(path)?;
    let stream = input
        .streams()
        .best(ff::media::Type::Video)
        .ok_or_else(|| anyhow!("no video stream"))?;
    let index = stream.index();
    let rate = stream.avg_frame_rate();
    let fps = if rate.numerator() > 0 && rate.denominator() > 0 {
        f64::from(rate)
    } else {
        FALLBACK_FPS
    };
    let mut decoder = ff::codec::context::Context::from_parameters(stream.parameters())?
        .decoder()
        .video()?;
    let format = Format::new(pixels, (decoder.width(), decoder.height()), canvas);
    // The frame count the file claims, or one from its length, for progress and the
    // memory check.
    let expected = match stream.frames() {
        n if n > 0 => n as u64,
        _ => {
            let seconds = input.duration().max(0) as f64 / f64::from(ff::ffi::AV_TIME_BASE);
            (seconds * fps).ceil() as u64
        }
    };
    let too_big = |frames: u64| {
        anyhow!(
            "{name} needs {} of memory; shorten it or lower the canvas size",
            describe_bytes(frames * format.frame_bytes())
        )
    };
    let mut memory = budget
        .take(expected * format.frame_bytes())
        .ok_or_else(|| too_big(expected))?;
    let mut converter = Converter::new(format);
    let mut frames = Vec::with_capacity(expected as usize);
    let mut decoded = ff::frame::Video::empty();
    let mut receive = |decoder: &mut ff::decoder::Video, frames: &mut Vec<Frame>| -> Result<()> {
        while decoder.receive_frame(&mut decoded).is_ok() {
            if frames.len() as u64 >= expected && !memory.grow(format.frame_bytes()) {
                return Err(too_big(frames.len() as u64 + 1));
            }
            frames.push(converter.convert(&decoded)?);
            let done = frames.len() as u64 * 1000 / expected.max(1);
            progress.store(done.min(1000) as u32, Ordering::Relaxed);
        }
        Ok(())
    };
    for (stream, packet) in input.packets() {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        if stream.index() == index {
            decoder.send_packet(&packet)?;
            receive(&mut decoder, &mut frames)?;
        }
    }
    decoder.send_eof()?;
    receive(&mut decoder, &mut frames)?;
    if frames.is_empty() {
        bail!("no frames");
    }
    // Give back what the file over-claimed.
    let used = frames.len() as u64 * format.frame_bytes();
    let memory = if memory.bytes() > used {
        drop(memory);
        budget
            .take(used)
            .ok_or_else(|| too_big(frames.len() as u64))?
    } else {
        memory
    };
    progress.store(1000, Ordering::Relaxed);
    Ok(Clip {
        format,
        fps,
        frames,
        _memory: memory,
    })
}

/// A clip being decoded on a worker thread. Dropping it cancels the decode.
pub struct Loading {
    pub path: PathBuf,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    result: Receiver<Result<Clip>>,
}

impl Loading {
    pub fn start(path: &Path, pixels: Pixels, canvas: (u32, u32), budget: &Budget) -> Self {
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (send, result) = mpsc::channel();
        let (owned, budget) = (path.to_path_buf(), budget.clone());
        let (p, c) = (progress.clone(), cancel.clone());
        let spawned = std::thread::Builder::new()
            .name("clip loader".into())
            .spawn(move || {
                let _ = send.send(load(&owned, pixels, canvas, &budget, &p, &c));
            });
        if let Err(err) = spawned {
            let (send, failed) = mpsc::channel();
            let _ = send.send(Err(anyhow!(
                "could not start loading {}: {err}",
                name(path)
            )));
            return Self {
                path: path.to_path_buf(),
                progress,
                cancel,
                result: failed,
            };
        }
        Self {
            path: path.to_path_buf(),
            progress,
            cancel,
            result,
        }
    }

    /// How far it got, 0–1.
    pub fn progress(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// The clip, or why it failed, once the worker is done.
    pub fn poll(&self) -> Option<Result<Clip>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(anyhow!(
                "the clip loader for {} stopped",
                name(&self.path)
            ))),
        }
    }

    /// Waits for the worker to finish.
    pub fn wait(&self) -> Result<Clip> {
        self.result
            .recv()
            .map_err(|_| anyhow!("the clip loader for {} stopped", name(&self.path)))?
    }
}

impl Drop for Loading {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
