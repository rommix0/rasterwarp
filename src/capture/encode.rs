//! The encoder thread: converts captured RGBA frames and writes them to a video file
//! with the linked FFmpeg libraries.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::thread::JoinHandle;

use anyhow::{Context, Result, anyhow, bail};
use ff::format::sample::{Sample, Type};
use ff::{
    ChannelLayout, Dictionary, Packet, Rational, codec, encoder, format, frame, software::scaling,
};
use ffmpeg_next as ff;

use crate::rate::FrameRate;
use crate::save::saved_names;

/// Frames that can wait for the encoder before real-time capture starts dropping them.
pub const QUEUE: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoFormat {
    /// HEVC 4:4:4 on the NVIDIA hardware encoder, near-lossless, in fragmented MP4.
    #[default]
    Hevc,
    /// FFV1 lossless RGB in Matroska, on the CPU.
    Ffv1,
    /// ProRes 4444 with an alpha channel in QuickTime, on the CPU: what editors and
    /// compositors read.
    ProRes4444,
}

saved_names!(VideoFormat {
    Hevc => "hevc",
    Ffv1 => "ffv1",
    ProRes4444 => "prores-4444",
});

impl VideoFormat {
    pub const ALL: [VideoFormat; 3] = [
        VideoFormat::Hevc,
        VideoFormat::Ffv1,
        VideoFormat::ProRes4444,
    ];

    pub fn label(self) -> &'static str {
        match self {
            VideoFormat::Hevc => "HEVC NVENC 4:4:4, near-lossless",
            VideoFormat::Ffv1 => "FFV1 lossless, may drop frames above 720p",
            VideoFormat::ProRes4444 => "ProRes 4444 with alpha, may drop frames at 1080p60",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            VideoFormat::Hevc => "mp4",
            VideoFormat::Ffv1 => "mkv",
            VideoFormat::ProRes4444 => "mov",
        }
    }

    /// Whether the file keeps the canvas's alpha channel.
    pub fn carries_alpha(self) -> bool {
        match self {
            VideoFormat::Hevc => false,
            VideoFormat::Ffv1 | VideoFormat::ProRes4444 => true,
        }
    }
}

/// One captured frame: tightly packed sRGB RGBA rows, `width × height × 4` bytes, with
/// straight (not premultiplied) alpha.
pub struct Frame {
    /// Presentation time in frames at the recording's frame rate.
    pub pts: i64,
    pub rgba: Vec<u8>,
}

/// A running encoder thread. Dropping it without [`Encoder::finish`] still closes the file.
pub struct Encoder {
    frames: Option<SyncSender<Frame>>,
    sound: Option<Sender<Vec<i16>>>,
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
        Self::start_with_sound(path, format, size, rate, false)
    }

    /// Like [`Encoder::start`], with a second stream for a soundtrack when `sound` is
    /// set: AAC in HEVC files, 16-bit PCM in the others. Feed it with
    /// [`Encoder::send_sound`].
    pub fn start_with_sound(
        path: &Path,
        format: VideoFormat,
        size: (u32, u32),
        rate: FrameRate,
        sound: bool,
    ) -> Result<Self> {
        if path.exists() {
            bail!("{} already exists", path.display());
        }
        let (frames, frames_rx) = mpsc::sync_channel(QUEUE);
        let (recycle_tx, recycled) = mpsc::channel();
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let (sound_tx, sound_rx) = mpsc::channel();
        let owned = path.to_path_buf();
        let thread = std::thread::Builder::new()
            .name("encoder".into())
            .spawn(move || {
                let job = Job {
                    path: owned,
                    format,
                    size,
                    rate,
                    frames: frames_rx,
                    sound: sound.then_some(sound_rx),
                    recycled: recycle_tx,
                    ready: ready_tx,
                };
                run(job)
            })
            .context("could not start the encoder thread")?;
        match ready.recv() {
            Ok(Ok(())) => Ok(Self {
                frames: Some(frames),
                sound: sound.then_some(sound_tx),
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
        try_queue(self.sender()?, frame)
    }

    /// Queues a frame, waiting for room. An error means the encoder has stopped.
    pub fn send(&self, frame: Frame) -> Result<()> {
        self.sender()?
            .send(frame)
            .map_err(|_| anyhow!("the encoder stopped"))
    }

    /// Queues interleaved stereo samples (48 kHz) for the soundtrack. Sound is never
    /// dropped, even when video frames are.
    pub fn send_sound(&self, samples: Vec<i16>) -> Result<()> {
        let sound = self
            .sound
            .as_ref()
            .ok_or_else(|| anyhow!("this recording has no soundtrack"))?;
        sound
            .send(samples)
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
        self.sound = None;
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

/// Queues `frame` on an encoder's `queue` without waiting. Returns the frame back if the
/// queue is full; an error means the encoder thread has stopped.
pub(super) fn try_queue(queue: &SyncSender<Frame>, frame: Frame) -> Result<Option<Frame>> {
    match queue.try_send(frame) {
        Ok(()) => Ok(None),
        Err(TrySendError::Full(frame)) => Ok(Some(frame)),
        Err(TrySendError::Disconnected(_)) => Err(anyhow!("the encoder stopped")),
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        self.frames = None;
        self.sound = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// What the encoder thread needs to run.
struct Job {
    path: PathBuf,
    format: VideoFormat,
    size: (u32, u32),
    rate: FrameRate,
    frames: Receiver<Frame>,
    /// Present when the recording has a soundtrack.
    sound: Option<Receiver<Vec<i16>>>,
    recycled: Sender<Vec<u8>>,
    ready: SyncSender<Result<()>>,
}

fn run(job: Job) -> Result<u64> {
    let Job {
        path,
        format,
        size,
        rate,
        frames,
        sound,
        recycled,
        ready,
    } = job;
    let mut sink = match Sink::open(&path, format, size, rate, sound.is_some()) {
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
        if let Some(sound) = &sound {
            for samples in sound.try_iter() {
                if let Err(err) = sink.write_sound(&samples) {
                    let _ = sink.finish();
                    return Err(err.context("encoding the soundtrack"));
                }
            }
        }
        if let Err(err) = sink.write(&frame) {
            // Keep what was written playable where possible.
            let _ = sink.finish();
            return Err(err.context(format!("encoding frame {}", frame.pts)));
        }
        written += 1;
        let _ = recycled.send(frame.rgba);
    }
    // The frame channel is closed, and the sound one closes right after it.
    if let Some(sound) = sound {
        for samples in sound.iter() {
            if let Err(err) = sink.write_sound(&samples) {
                let _ = sink.finish();
                return Err(err.context("encoding the soundtrack"));
            }
        }
    }
    sink.finish()?;
    Ok(written)
}

/// How captured RGBA becomes the encoder's pixel format.
enum Convert {
    /// RGBA → YUV 4:4:4 with BT.709 coefficients (HEVC; ProRes keeps the alpha too).
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
    sound: Option<SoundTrack>,
}

/// The soundtrack's encoder and stream.
struct SoundTrack {
    encoder: encoder::Audio,
    index: usize,
    stream_time_base: Rational,
    /// Interleaved stereo samples not yet sent (the encoder takes whole frames).
    pending: Vec<i16>,
    /// The next sample's position since the start.
    next_pts: i64,
    /// Stereo frames per encoder frame.
    frame_size: usize,
    /// AAC takes planar floats; PCM takes packed 16-bit.
    planar: bool,
}

/// The sample time base: one sample at 48 kHz.
const SOUND_TIME_BASE: Rational = Rational(1, 48_000);

impl Sink {
    fn open(
        path: &Path,
        format: VideoFormat,
        size: (u32, u32),
        rate: FrameRate,
        sound: bool,
    ) -> Result<Self> {
        ff::init().context("could not initialise FFmpeg")?;
        let (width, height) = size;
        let frame_rate = Rational(rate.num, rate.den);
        let time_base = frame_rate.invert();
        let (codec_name, pixel) = match format {
            VideoFormat::Hevc => ("hevc_nvenc", format::Pixel::YUV444P),
            VideoFormat::Ffv1 => ("ffv1", format::Pixel::BGRA),
            VideoFormat::ProRes4444 => ("prores_ks", format::Pixel::YUVA444P10LE),
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
        if format != VideoFormat::Ffv1 {
            setup.set_colorspace(ff::util::color::Space::BT709);
            setup.set_color_range(ff::util::color::Range::MPEG);
            setup.set_color_primaries(ff::util::color::Primaries::BT709);
            setup.set_color_transfer_characteristic(ff::util::color::TransferCharacteristic::BT709);
        }
        match format {
            VideoFormat::Hevc => {
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
            VideoFormat::ProRes4444 => {
                // Each frame is coded on its own, so frame threads keep every core busy.
                setup.set_threading(codec::threading::Config {
                    kind: codec::threading::Type::Frame,
                    count: 0,
                });
                for (key, value) in [
                    ("profile", "4444"),
                    ("alpha_bits", "16"),
                    ("vendor", "apl0"),
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
            VideoFormat::ProRes4444 => anyhow!("could not open the ProRes encoder ({err})"),
        })?;
        stream.set_parameters(&encoder);
        stream.set_time_base(time_base);
        // Declare the constant frame rate so players don't have to infer it.
        stream.set_rate(frame_rate);
        stream.set_avg_frame_rate(frame_rate);
        let mut track = None;
        if sound {
            let (name, sample) = match format {
                VideoFormat::Hevc => ("aac", Sample::F32(Type::Planar)),
                VideoFormat::Ffv1 | VideoFormat::ProRes4444 => {
                    ("pcm_s16le", Sample::I16(Type::Packed))
                }
            };
            let codec = encoder::find_by_name(name)
                .ok_or_else(|| anyhow!("this FFmpeg build has no {name} encoder"))?;
            let mut stream = output.add_stream(codec)?;
            let index = stream.index();
            let mut setup = codec::context::Context::new_with_codec(codec)
                .encoder()
                .audio()?;
            setup.set_rate(48_000);
            setup.set_channel_layout(ChannelLayout::STEREO);
            setup.set_format(sample);
            setup.set_time_base(SOUND_TIME_BASE);
            if format == VideoFormat::Hevc {
                setup.set_bit_rate(192_000);
            }
            if global_header {
                setup.set_flags(codec::Flags::GLOBAL_HEADER);
            }
            let encoder = setup
                .open_as(codec)
                .with_context(|| format!("could not open the {name} encoder"))?;
            stream.set_parameters(&encoder);
            stream.set_time_base(SOUND_TIME_BASE);
            let frame_size = match encoder.frame_size() {
                0 => 1024,
                n => n as usize,
            };
            track = Some(SoundTrack {
                encoder,
                index,
                stream_time_base: SOUND_TIME_BASE,
                pending: Vec::new(),
                next_pts: 0,
                frame_size,
                planar: sample == Sample::F32(Type::Planar),
            });
        }
        let mut header = Dictionary::new();
        if format == VideoFormat::Hevc {
            // Fragmented MP4 stays playable if the recording is interrupted.
            header.set("movflags", "frag_keyframe+empty_moov");
            if track.is_some() {
                // AAC starts 1024 samples early (encoder priming). The muxer only writes
                // the edit list that trims it when the header is delayed to the first
                // fragment; without it the sound would lag the picture by 21 ms.
                header.set("movflags", "frag_keyframe+empty_moov+delay_moov");
                header.set("use_editlist", "1");
            }
        }
        output
            .write_header_with(header)
            .context("could not write the file header")?;
        let stream_time_base = output.stream(0).context("no video stream")?.time_base();
        if let Some(track) = &mut track {
            track.stream_time_base = output
                .stream(track.index)
                .context("no sound stream")?
                .time_base();
        }

        let convert = match format {
            VideoFormat::Hevc | VideoFormat::ProRes4444 => {
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
            sound: track,
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

    /// Queues soundtrack samples and encodes every whole frame of them.
    fn write_sound(&mut self, samples: &[i16]) -> Result<()> {
        let Some(track) = &mut self.sound else {
            return Ok(());
        };
        track.pending.extend_from_slice(samples);
        while track.pending.len() >= track.frame_size * 2 {
            let chunk: Vec<i16> = track.pending.drain(..track.frame_size * 2).collect();
            Self::send_sound(track, &mut self.output, &chunk)?;
        }
        Ok(())
    }

    /// Encodes one frame of interleaved stereo samples.
    fn send_sound(
        track: &mut SoundTrack,
        output: &mut format::context::Output,
        chunk: &[i16],
    ) -> Result<()> {
        let samples = chunk.len() / 2;
        let format = if track.planar {
            Sample::F32(Type::Planar)
        } else {
            Sample::I16(Type::Packed)
        };
        let mut frame = frame::Audio::new(format, samples, ChannelLayout::STEREO);
        frame.set_rate(48_000);
        if track.planar {
            for (i, pair) in chunk.as_chunks::<2>().0.iter().enumerate() {
                frame.plane_mut::<f32>(0)[i] = f32::from(pair[0]) / 32768.0;
                frame.plane_mut::<f32>(1)[i] = f32::from(pair[1]) / 32768.0;
            }
        } else {
            frame.plane_mut::<i16>(0).copy_from_slice(chunk);
        }
        frame.set_pts(Some(track.next_pts));
        track.next_pts += samples as i64;
        track.encoder.send_frame(&frame)?;
        Self::drain_sound(track, output)
    }

    /// Writes the soundtrack's finished packets.
    fn drain_sound(track: &mut SoundTrack, output: &mut format::context::Output) -> Result<()> {
        let mut packet = Packet::empty();
        loop {
            match track.encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(track.index);
                    packet.rescale_ts(SOUND_TIME_BASE, track.stream_time_base);
                    packet.write_interleaved(output)?;
                }
                // As for video: EAGAIN wants more input; Eof follows the final flush.
                Err(ff::Error::Other { errno }) if errno == ff::error::EAGAIN => return Ok(()),
                Err(ff::Error::Eof) => return Ok(()),
                Err(err) => return Err(err.into()),
            }
        }
    }

    fn finish(&mut self) -> Result<()> {
        self.encoder.send_eof()?;
        self.drain()?;
        if let Some(track) = &mut self.sound {
            if !track.pending.is_empty() {
                let rest = std::mem::take(&mut track.pending);
                Self::send_sound(track, &mut self.output, &rest)?;
            }
            track.encoder.send_eof()?;
            Self::drain_sound(track, &mut self.output)?;
        }
        self.output.write_trailer()?;
        Ok(())
    }
}
