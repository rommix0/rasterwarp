//! Cameras: a DirectShow device read on a worker thread into a rolling buffer of its
//! most recent frames. The render loop only ever looks at the buffer, so it never waits
//! for the camera.

use std::collections::VecDeque;
use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use ffmpeg_next as ff;

use super::playhead::Arrival;
use super::{Budget, Converter, Format, Frame, Pixels, Reservation};

/// How many seconds of frames a camera keeps, for delay and slit-scan.
pub const BUFFER_SECONDS: RangeInclusive<f32> = 1.0..=30.0;
pub const DEFAULT_BUFFER_SECONDS: f32 = 10.0;

/// How long to wait before trying a camera that stopped again.
pub const RETRY: Duration = Duration::from_secs(3);

/// A camera's frame rate when the device doesn't report one.
const FALLBACK_FPS: f64 = 30.0;

/// DirectShow, FFmpeg's Windows capture input.
fn dshow() -> Result<ff::Format> {
    ff::init().context("could not initialise FFmpeg")?;
    ff::device::register_all();
    ff::device::input::video()
        .find(|f| f.name() == "dshow")
        .ok_or_else(|| anyhow!("this FFmpeg build has no DirectShow input"))
}

/// The names of the video capture devices DirectShow lists.
pub fn list() -> Result<Vec<String>> {
    let ff::Format::Input(format) = dshow()? else {
        bail!("DirectShow is not an input format");
    };
    let mut names = Vec::new();
    // SAFETY: the list is filled in and freed by FFmpeg; every pointer read here lives in
    // it until `avdevice_free_list_devices`.
    unsafe {
        use ff::ffi::*;
        let mut list: *mut AVDeviceInfoList = std::ptr::null_mut();
        let found = avdevice_list_input_sources(
            format.as_ptr() as *const _,
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut list,
        );
        if found < 0 {
            avdevice_free_list_devices(&mut list);
            return Err(ff::Error::from(found)).context("could not list the cameras");
        }
        for i in 0..(*list).nb_devices.max(0) as usize {
            let device = *(*list).devices.add(i);
            let types = std::slice::from_raw_parts(
                (*device).media_types,
                (*device).nb_media_types.max(0) as usize,
            );
            if types.contains(&AVMediaType::AVMEDIA_TYPE_VIDEO) {
                let name = std::ffi::CStr::from_ptr((*device).device_description);
                names.push(name.to_string_lossy().into_owned());
            }
        }
        avdevice_free_list_devices(&mut list);
    }
    Ok(names)
}

/// How the camera is doing.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Opening,
    Live,
    /// It stopped or couldn't be opened; it's tried again every [`RETRY`]. The text says
    /// why.
    NoSignal(String),
}

/// A buffered frame and when it arrived.
pub struct Buffered {
    pub arrival: Arrival,
    pub frame: Arc<Frame>,
}

/// What the capture thread shares with the app.
struct Shared {
    status: Status,
    format: Option<Format>,
    frames: VecDeque<Buffered>,
    /// Said when the buffer had to be shorter than asked.
    note: Option<String>,
    /// The buffer's memory; kept while there's no signal, so the frames stay.
    memory: Option<Reservation>,
}

/// An open camera. Dropping it stops the capture thread.
pub struct Camera {
    pub name: String,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
}

impl Camera {
    /// Starts capturing from the device called `name`, keeping `buffer_seconds` of
    /// frames for a `canvas`-sized canvas.
    pub fn open(
        name: &str,
        pixels: Pixels,
        canvas: (u32, u32),
        buffer_seconds: f32,
        budget: &Budget,
    ) -> Self {
        let shared = Arc::new(Mutex::new(Shared {
            status: Status::Opening,
            format: None,
            frames: VecDeque::new(),
            note: None,
            memory: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let capture = Capture {
            name: name.to_string(),
            pixels,
            canvas,
            buffer_seconds: buffer_seconds.clamp(*BUFFER_SECONDS.start(), *BUFFER_SECONDS.end()),
            budget: budget.clone(),
            shared: shared.clone(),
            stop: stop.clone(),
            epoch: Instant::now(),
            next_seq: 0,
        };
        let spawned = std::thread::Builder::new()
            .name("camera".into())
            .spawn(move || capture.run());
        if let Err(err) = spawned {
            lock(&shared).status = Status::NoSignal(format!("could not start the camera: {err}"));
        }
        Self {
            name: name.to_string(),
            shared,
            stop,
        }
    }

    pub fn status(&self) -> Status {
        lock(&self.shared).status.clone()
    }

    /// The buffered frames' format, once the camera has opened.
    pub fn format(&self) -> Option<Format> {
        lock(&self.shared).format
    }

    /// A note about the buffer, if it had to be shorter than asked.
    pub fn note(&self) -> Option<String> {
        lock(&self.shared).note.clone()
    }

    /// When each buffered frame arrived, oldest first.
    pub fn arrivals(&self) -> Vec<Arrival> {
        lock(&self.shared)
            .frames
            .iter()
            .map(|b| b.arrival)
            .collect()
    }

    /// Frame number `seq`, if it's still buffered.
    pub fn frame(&self, seq: i64) -> Option<Arc<Frame>> {
        let shared = lock(&self.shared);
        let first = shared.frames.front()?.arrival.seq;
        let i = usize::try_from(seq - first).ok()?;
        shared
            .frames
            .get(i)
            .filter(|b| b.arrival.seq == seq)
            .map(|b| b.frame.clone())
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        // The thread notices at its next frame or retry and exits on its own.
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The capture thread's state.
struct Capture {
    name: String,
    pixels: Pixels,
    canvas: (u32, u32),
    buffer_seconds: f32,
    budget: Budget,
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    epoch: Instant,
    /// Frame numbers keep counting across reconnections.
    next_seq: i64,
}

impl Capture {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Captures until stopped, retrying after each failure.
    fn run(mut self) {
        while !self.stopped() {
            let err = match self.capture() {
                Ok(()) => return,
                Err(err) => err,
            };
            log::warn!("camera {}: {err:#}", self.name);
            lock(&self.shared).status = Status::NoSignal(format!("{err:#}"));
            let retry = Instant::now() + RETRY;
            while Instant::now() < retry {
                if self.stopped() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            lock(&self.shared).status = Status::Opening;
        }
    }

    /// Opens the camera and buffers its frames. Ok when stopped; an error when it fails
    /// to open or stops sending pictures.
    fn capture(&mut self) -> Result<()> {
        let device = format!("video={}", self.name);
        let mut input = ff::format::open_with(&device, &dshow()?, ff::Dictionary::new())
            .with_context(|| format!("could not open {}", self.name))?
            .input();
        let stream = input
            .streams()
            .best(ff::media::Type::Video)
            .ok_or_else(|| anyhow!("{} sends no video", self.name))?;
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
        let format = Format::new(
            self.pixels,
            (decoder.width(), decoder.height()),
            self.canvas,
        );
        let capacity = self.reserve(format, fps)?;
        let mut converter = Converter::new(format);
        let mut decoded = ff::frame::Video::empty();
        for (stream, packet) in input.packets() {
            if self.stopped() {
                return Ok(());
            }
            if stream.index() != index {
                continue;
            }
            decoder.send_packet(&packet)?;
            while decoder.receive_frame(&mut decoded).is_ok() {
                let frame = Arc::new(converter.convert(&decoded)?);
                let arrival = Arrival {
                    seq: self.next_seq,
                    time: self.epoch.elapsed().as_secs_f64(),
                };
                self.next_seq += 1;
                let mut shared = lock(&self.shared);
                shared.status = Status::Live;
                shared.frames.push_back(Buffered { arrival, frame });
                while shared.frames.len() > capacity {
                    shared.frames.pop_front();
                }
            }
        }
        if self.stopped() {
            return Ok(());
        }
        Err(anyhow!("{} stopped sending pictures", self.name))
    }

    /// Makes room for the buffer in the budget: the asked-for length, or what fits.
    /// Returns how many frames it holds. A reopened camera starts a new buffer.
    fn reserve(&self, format: Format, fps: f64) -> Result<usize> {
        let mut shared = lock(&self.shared);
        shared.frames.clear();
        shared.memory = None;
        let wanted = (f64::from(self.buffer_seconds) * fps).ceil().max(2.0) as u64;
        let fits = self.budget.available() / format.frame_bytes().max(1);
        let frames = wanted.min(fits);
        if frames < 2 {
            bail!("not enough memory left for the {} buffer", self.name);
        }
        shared.memory = self.budget.take(frames * format.frame_bytes());
        if shared.memory.is_none() {
            bail!("not enough memory left for the {} buffer", self.name);
        }
        shared.note = (frames < wanted).then(|| {
            format!(
                "Buffer shortened to {:.1} s to fit in memory",
                frames as f64 / fps
            )
        });
        shared.format = Some(format);
        Ok(frames as usize)
    }
}
