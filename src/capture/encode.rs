//! The encoder thread: converts captured RGBA frames and writes them to a video file
//! with the linked FFmpeg libraries.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::thread::JoinHandle;

use anyhow::{Context, Result, anyhow, bail};
use ff::{Dictionary, Packet, Rational, codec, encoder, format, frame, software::scaling};
use ffmpeg_next as ff;

use crate::rate::FrameRate;

/// Frames that can wait for the encoder before real-time capture starts dropping them.
pub const QUEUE: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoFormat {
    /// HEVC 4:4:4 on the NVIDIA hardware encoder, near-lossless, in fragmented MP4.
    Hevc,
    /// FFV1 lossless RGB in Matroska, on the CPU.
    Ffv1,
}

impl VideoFormat {
    pub const ALL: [VideoFormat; 2] = [VideoFormat::Hevc, VideoFormat::Ffv1];

    pub fn label(self) -> &'static str {
        match self {
            VideoFormat::Hevc => "HEVC NVENC 4:4:4, near-lossless",
            VideoFormat::Ffv1 => "FFV1 lossless, may drop frames above 720p",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            VideoFormat::Hevc => "mp4",
            VideoFormat::Ffv1 => "mkv",
        }
    }
}

/// One captured frame: tightly packed sRGB RGBA rows, `width × height × 4` bytes.
pub struct Frame {
    /// Presentation time in frames at the recording's frame rate.
    pub pts: i64,
    pub rgba: Vec<u8>,
}

/// A running encoder thread. Dropping it without [`Encoder::finish`] still closes the file.
pub struct Encoder {
    frames: Option<SyncSender<Frame>>,
    recycled: Receiver<Vec<u8>>,
    thread: Option<JoinHandle<Result<u64>>>,
}

impl Encoder {
    /// Opens `path` for writing at `rate` frames per second and starts the encoder
    /// thread. Fails without leaving a file behind if the file exists or the encoder
    /// can't be opened.
    pub fn start(
        path: &Path,
        format: VideoFormat,
        size: (u32, u32),
        rate: FrameRate,
    ) -> Result<Self> {
        if path.exists() {
            bail!("{} already exists", path.display());
        }
        let (frames, frames_rx) = mpsc::sync_channel(QUEUE);
        let (recycle_tx, recycled) = mpsc::channel();
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let owned = path.to_path_buf();
        let thread = std::thread::Builder::new()
            .name("encoder".into())
            .spawn(move || run(owned, format, size, rate, frames_rx, recycle_tx, ready_tx))
            .context("could not start the encoder thread")?;
        match ready.recv() {
            Ok(Ok(())) => Ok(Self {
                frames: Some(frames),
                recycled,
                thread: Some(thread),
            }),
            Ok(Err(err)) => {
                let _ = thread.join();
                let _ = std::fs::remove_file(path);
                Err(err)
            }
            Err(_) => Err(anyhow!("the encoder thread stopped while opening")),
        }
    }

    /// Queues a frame without waiting. Returns the frame back if the queue is full.
    /// An error means the encoder has stopped; [`Encoder::finish`] reports why.
    pub fn try_send(&self, frame: Frame) -> Result<Option<Frame>> {
        match self.sender()?.try_send(frame) {
            Ok(()) => Ok(None),
            Err(TrySendError::Full(frame)) => Ok(Some(frame)),
            Err(TrySendError::Disconnected(_)) => Err(anyhow!("the encoder stopped")),
        }
    }

    /// Queues a frame, waiting for room. An error means the encoder has stopped.
    pub fn send(&self, frame: Frame) -> Result<()> {
        self.sender()?
            .send(frame)
            .map_err(|_| anyhow!("the encoder stopped"))
    }

    /// A frame buffer the encoder has finished with, for reuse.
    pub fn recycled_buffer(&self) -> Option<Vec<u8>> {
        self.recycled.try_recv().ok()
    }

    /// Flushes the encoder, closes the file and returns the number of frames written,
    /// or the error that stopped the encoder.
    pub fn finish(mut self) -> Result<u64> {
        self.frames = None;
        match self.thread.take().map(JoinHandle::join) {
            Some(Ok(result)) => result,
            Some(Err(_)) => Err(anyhow!("the encoder thread panicked")),
            None => Err(anyhow!("the encoder already finished")),
        }
    }

    fn sender(&self) -> Result<&SyncSender<Frame>> {
        self.frames
            .as_ref()
            .ok_or_else(|| anyhow!("the encoder already finished"))
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        self.frames = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    path: PathBuf,
    format: VideoFormat,
    size: (u32, u32),
    rate: FrameRate,
    frames: Receiver<Frame>,
    recycled: Sender<Vec<u8>>,
    ready: SyncSender<Result<()>>,
) -> Result<u64> {
    let mut sink = match Sink::open(&path, format, size, rate) {
        Ok(sink) => {
            let _ = ready.send(Ok(()));
            sink
        }
        Err(err) => {
            let _ = ready.send(Err(err));
            return Ok(0);
        }
    };
    let mut written = 0;
    for frame in frames {
        if let Err(err) = sink.write(&frame) {
            // Keep what was written playable where possible.
            let _ = sink.finish();
            return Err(err.context(format!("encoding frame {}", frame.pts)));
        }
        written += 1;
        let _ = recycled.send(frame.rgba);
    }
    sink.finish()?;
    Ok(written)
}

/// How captured RGBA becomes the encoder's pixel format.
enum Convert {
    /// RGBA → YUV 4:4:4 with BT.709 coefficients (HEVC).
    Scale {
        scaler: scaling::Context,
        rgba: frame::Video,
    },
    /// RGBA → BGRA by swapping bytes, which is exact (FFV1).
    Swizzle,
}

struct Sink {
    output: format::context::Output,
    encoder: encoder::Video,
    convert: Convert,
    frame: frame::Video,
    size: (u32, u32),
    /// One frame: the time base the frames' timestamps count in.
    time_base: Rational,
    stream_time_base: Rational,
}

impl Sink {
    fn open(path: &Path, format: VideoFormat, size: (u32, u32), rate: FrameRate) -> Result<Self> {
        ff::init().context("could not initialise FFmpeg")?;
        let (width, height) = size;
        let frame_rate = Rational(rate.num, rate.den);
        let time_base = frame_rate.invert();
        let (codec_name, pixel) = match format {
            VideoFormat::Hevc => ("hevc_nvenc", format::Pixel::YUV444P),
            VideoFormat::Ffv1 => ("ffv1", format::Pixel::BGRA),
        };
        let codec = encoder::find_by_name(codec_name)
            .ok_or_else(|| anyhow!("this FFmpeg build has no {codec_name} encoder"))?;
        let mut output =
            format::output(path).with_context(|| format!("could not create {}", path.display()))?;
        let global_header = output
            .format()
            .flags()
            .contains(format::Flags::GLOBAL_HEADER);
        let mut stream = output.add_stream(codec)?;
        let mut setup = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()?;
        setup.set_width(width);
        setup.set_height(height);
        setup.set_format(pixel);
        setup.set_time_base(time_base);
        setup.set_frame_rate(Some(frame_rate));
        setup.set_gop(rate.keyframe_interval());
        setup.set_max_b_frames(0);
        let mut options = Dictionary::new();
        match format {
            VideoFormat::Hevc => {
                setup.set_colorspace(ff::util::color::Space::BT709);
                setup.set_color_range(ff::util::color::Range::MPEG);
                setup.set_color_primaries(ff::util::color::Primaries::BT709);
                setup.set_color_transfer_characteristic(
                    ff::util::color::TransferCharacteristic::BT709,
                );
                for (key, value) in [
                    ("preset", "p4"),
                    ("tune", "hq"),
                    ("rc", "constqp"),
                    ("qp", "18"),
                    ("profile", "rext"),
                ] {
                    options.set(key, value);
                }
            }
            VideoFormat::Ffv1 => {
                // Without slice threads FFV1 runs on one core (about 8 fps at 1080p).
                setup.set_threading(codec::threading::Config {
                    kind: codec::threading::Type::Slice,
                    count: 0,
                });
                for (key, value) in [
                    ("level", "3"),
                    ("slices", "16"),
                    ("slicecrc", "1"),
                    ("coder", "1"),
                ] {
                    options.set(key, value);
                }
            }
        }
        if global_header {
            setup.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let encoder = setup.open_with(options).map_err(|err| match format {
            VideoFormat::Hevc => anyhow!(
                "could not open the NVENC HEVC encoder ({err}); it needs an NVIDIA GPU with \
                 NVENC and a free encode session. Try FFV1 instead."
            ),
            VideoFormat::Ffv1 => anyhow!("could not open the FFV1 encoder ({err})"),
        })?;
        stream.set_parameters(&encoder);
        stream.set_time_base(time_base);
        // Declare the constant frame rate so players don't have to infer it.
        stream.set_rate(frame_rate);
        stream.set_avg_frame_rate(frame_rate);
        let mut header = Dictionary::new();
        if format == VideoFormat::Hevc {
            // Fragmented MP4 stays playable if the recording is interrupted.
            header.set("movflags", "frag_keyframe+empty_moov");
        }
        output
            .write_header_with(header)
            .context("could not write the file header")?;
        let stream_time_base = output.stream(0).context("no video stream")?.time_base();

        let convert = match format {
            VideoFormat::Hevc => {
                let mut scaler = scaling::Context::get(
                    format::Pixel::RGBA,
                    width,
                    height,
                    pixel,
                    width,
                    height,
                    scaling::Flags::BILINEAR,
                )?;
                // swscale defaults to BT.601; match the BT.709 tags (limited range out).
                // SAFETY: the scaler pointer is valid for the life of `scaler`, and the
                // coefficient table returned by FFmpeg is static.
                unsafe {
                    let bt709 = ff::ffi::sws_getCoefficients(ff::ffi::SWS_CS_ITU709);
                    ff::ffi::sws_setColorspaceDetails(
                        scaler.as_mut_ptr(),
                        bt709,
                        1,
                        bt709,
                        0,
                        0,
                        1 << 16,
                        1 << 16,
                    );
                }
                Convert::Scale {
                    scaler,
                    rgba: frame::Video::new(format::Pixel::RGBA, width, height),
                }
            }
            VideoFormat::Ffv1 => Convert::Swizzle,
        };
        Ok(Self {
            output,
            encoder,
            convert,
            frame: frame::Video::new(pixel, width, height),
            size,
            time_base,
            stream_time_base,
        })
    }

    fn write(&mut self, captured: &Frame) -> Result<()> {
        let (width, height) = (self.size.0 as usize, self.size.1 as usize);
        let row = width * 4;
        if captured.rgba.len() != row * height {
            bail!("captured frame has the wrong size");
        }
        // The encoder may still hold a reference to the last frame's buffer.
        // SAFETY: `self.frame` owns a valid AVFrame.
        if unsafe { ff::ffi::av_frame_make_writable(self.frame.as_mut_ptr()) } < 0 {
            bail!("out of memory for the encoder frame");
        }
        match &mut self.convert {
            Convert::Swizzle => {
                let stride = self.frame.stride(0);
                let data = self.frame.data_mut(0);
                for (y, src) in captured.rgba.chunks_exact(row).enumerate() {
                    let dst = &mut data[y * stride..y * stride + row];
                    let (dst, _) = dst.as_chunks_mut::<4>();
                    let (src, _) = src.as_chunks::<4>();
                    for (d, [r, g, b, a]) in dst.iter_mut().zip(src) {
                        *d = [*b, *g, *r, *a];
                    }
                }
            }
            Convert::Scale { scaler, rgba } => {
                let stride = rgba.stride(0);
                let data = rgba.data_mut(0);
                for (y, src) in captured.rgba.chunks_exact(row).enumerate() {
                    data[y * stride..y * stride + row].copy_from_slice(src);
                }
                scaler.run(rgba, &mut self.frame)?;
            }
        }
        self.frame.set_pts(Some(captured.pts));
        self.encoder.send_frame(&self.frame)?;
        self.drain()
    }

    /// Writes every packet the encoder has ready.
    fn drain(&mut self) -> Result<()> {
        let mut packet = Packet::empty();
        loop {
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(0);
                    packet.set_duration(1); // one frame; the MP4 muxer warns without it
                    packet.rescale_ts(self.time_base, self.stream_time_base);
                    packet.write_interleaved(&mut self.output)?;
                }
                // EAGAIN means the encoder wants more input; Eof follows the final flush.
                Err(ff::Error::Other { errno }) if errno == ff::error::EAGAIN => return Ok(()),
                Err(ff::Error::Eof) => return Ok(()),
                Err(err) => return Err(err.into()),
            }
        }
    }

    fn finish(&mut self) -> Result<()> {
        self.encoder.send_eof()?;
        self.drain()?;
        self.output.write_trailer()?;
        Ok(())
    }
}
