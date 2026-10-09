//! Encodes short clips with the linked FFmpeg libraries and decodes them again.

use std::path::{Path, PathBuf};

use ffmpeg_next as ff;
use rasterwarp::capture::encode::{Encoder, Frame, VideoFormat};
use rasterwarp::rate::FrameRate;

const SIZE: (u32, u32) = (320, 180);
const FRAMES: i64 = 30;

/// A fresh path in a per-test temporary directory.
fn temp_file(test: &str, extension: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rasterwarp-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.join(format!("clip.{extension}"))
}

/// A moving gradient with some fine detail, different in every frame.
fn pattern(index: i64) -> Vec<u8> {
    let (w, h) = (SIZE.0 as usize, SIZE.1 as usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let shift = index as usize * 4;
            rgba.extend_from_slice(&[
                ((x + shift) * 255 / w) as u8,
                (y * 255 / h) as u8,
                ((x / 8 + y / 8) % 2 * 64 + shift) as u8,
                255,
            ]);
        }
    }
    rgba
}

/// `pattern` with see-through parts: the left quarter is clear (and black, as straight
/// alpha leaves it), the next quarter half-transparent.
fn pattern_with_alpha(index: i64) -> Vec<u8> {
    let mut rgba = pattern(index);
    let (pixels, _) = rgba.as_chunks_mut::<4>();
    for (i, px) in pixels.iter_mut().enumerate() {
        let x = i % SIZE.0 as usize;
        if x < SIZE.0 as usize / 4 {
            *px = [0, 0, 0, 0];
        } else if x < SIZE.0 as usize / 2 {
            px[3] = 128;
        }
    }
    rgba
}

fn encode(path: &Path, format: VideoFormat, rate: FrameRate) -> anyhow::Result<u64> {
    encode_frames(path, format, rate, pattern)
}

fn encode_frames(
    path: &Path,
    format: VideoFormat,
    rate: FrameRate,
    frame: fn(i64) -> Vec<u8>,
) -> anyhow::Result<u64> {
    let encoder = Encoder::start(path, format, SIZE, rate)?;
    for pts in 0..FRAMES {
        encoder.send(Frame {
            pts,
            rgba: frame(pts),
        })?;
    }
    encoder.finish()
}

/// The pixel format `path`'s video stream is coded in.
fn coded_format(path: &Path) -> ff::format::Pixel {
    ff::init().unwrap();
    let input = ff::format::input(path).expect("open the recording");
    let stream = input
        .streams()
        .best(ff::media::Type::Video)
        .expect("a video stream");
    let context = ff::codec::context::Context::from_parameters(stream.parameters()).unwrap();
    context.decoder().video().unwrap().format()
}

/// Decodes every frame of `path` to tightly packed RGBA (BT.709 for YUV input), checking
/// that the file declares `rate`.
fn decode(path: &Path, rate: FrameRate) -> Vec<(u32, u32, Vec<u8>)> {
    ff::init().unwrap();
    let mut input = ff::format::input(path).expect("open the recording");
    let stream = input
        .streams()
        .best(ff::media::Type::Video)
        .expect("a video stream");
    let index = stream.index();
    let declared = ff::Rational(rate.num, rate.den);
    assert_eq!(stream.avg_frame_rate(), declared, "declared frame rate");
    assert_eq!(stream.rate(), declared, "stream frame rate");
    let context = ff::codec::context::Context::from_parameters(stream.parameters()).unwrap();
    let mut decoder = context.decoder().video().unwrap();
    let (w, h) = (decoder.width(), decoder.height());
    let mut scaler = ff::software::scaling::Context::get(
        decoder.format(),
        w,
        h,
        ff::format::Pixel::RGBA,
        w,
        h,
        ff::software::scaling::Flags::BILINEAR,
    )
    .unwrap();
    if matches!(
        decoder.format(),
        ff::format::Pixel::YUV444P | ff::format::Pixel::YUVA444P12LE
    ) {
        // SAFETY: valid scaler pointer; FFmpeg's coefficient tables are static.
        unsafe {
            let bt709 = ff::ffi::sws_getCoefficients(ff::ffi::SWS_CS_ITU709);
            ff::ffi::sws_setColorspaceDetails(
                scaler.as_mut_ptr(),
                bt709,
                0,
                bt709,
                1,
                0,
                1 << 16,
                1 << 16,
            );
        }
    }
    let mut frames = Vec::new();
    let mut receive = |decoder: &mut ff::decoder::Video, frames: &mut Vec<_>| {
        let mut decoded = ff::frame::Video::empty();
        while decoder.receive_frame(&mut decoded).is_ok() {
            let mut rgba = ff::frame::Video::empty();
            scaler.run(&decoded, &mut rgba).unwrap();
            let row = w as usize * 4;
            let packed: Vec<u8> = rgba
                .data(0)
                .chunks(rgba.stride(0))
                .take(h as usize)
                .flat_map(|r| &r[..row])
                .copied()
                .collect();
            frames.push((w, h, packed));
        }
    };
    for (stream, packet) in input.packets() {
        if stream.index() == index {
            decoder.send_packet(&packet).unwrap();
            receive(&mut decoder, &mut frames);
        }
    }
    decoder.send_eof().unwrap();
    receive(&mut decoder, &mut frames);
    frames
}

#[test]
fn ffv1_recording_is_lossless() {
    let path = temp_file("ffv1", "mkv");
    let rate = FrameRate::default();
    assert_eq!(encode(&path, VideoFormat::Ffv1, rate).expect("encode"), 30);
    let frames = decode(&path, rate);
    assert_eq!(frames.len(), FRAMES as usize);
    let (w, h, rgba) = &frames[7];
    assert_eq!((*w, *h), SIZE);
    assert!(
        *rgba == pattern(7),
        "frame 7 differs from what was recorded"
    );
}

#[test]
fn hevc_recording_is_close_to_the_source() {
    let path = temp_file("hevc", "mp4");
    let rate = FrameRate::default();
    let written = match encode(&path, VideoFormat::Hevc, rate) {
        Ok(written) => written,
        Err(err) => {
            eprintln!("skipping HEVC test: {err:#}");
            return;
        }
    };
    assert_eq!(written, 30);
    let frames = decode(&path, rate);
    assert_eq!(frames.len(), FRAMES as usize);
    let (w, h, rgba) = &frames[7];
    assert_eq!((*w, *h), SIZE);
    let source = pattern(7);
    let error: u64 = rgba
        .iter()
        .zip(&source)
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    let mean = error as f64 / source.len() as f64;
    assert!(mean < 3.0, "mean error {mean:.2} per channel");
}

#[test]
fn recordings_declare_an_ntsc_rate_exactly() {
    let path = temp_file("ntsc", "mkv");
    let rate = FrameRate::ntsc(24);
    assert_eq!(encode(&path, VideoFormat::Ffv1, rate).expect("encode"), 30);
    assert_eq!(decode(&path, rate).len(), FRAMES as usize);
}

#[test]
fn an_existing_file_is_not_overwritten() {
    let path = temp_file("exists", "mkv");
    std::fs::write(&path, b"keep me").unwrap();
    let err = Encoder::start(&path, VideoFormat::Ffv1, SIZE, FrameRate::default())
        .err()
        .expect("start must fail");
    assert!(format!("{err:#}").contains("already exists"));
    assert_eq!(std::fs::read(&path).unwrap(), b"keep me");
}

#[test]
fn ffv1_keeps_alpha_exactly() {
    let path = temp_file("ffv1-alpha", "mkv");
    let rate = FrameRate::default();
    let written = encode_frames(&path, VideoFormat::Ffv1, rate, pattern_with_alpha);
    assert_eq!(written.expect("encode"), 30);
    let frames = decode(&path, rate);
    assert!(
        frames[7].2 == pattern_with_alpha(7),
        "frame 7 differs from what was recorded"
    );
}

#[test]
fn prores_4444_keeps_alpha() {
    let path = temp_file("prores", "mov");
    let rate = FrameRate::default();
    let written = encode_frames(&path, VideoFormat::ProRes4444, rate, pattern_with_alpha);
    assert_eq!(written.expect("encode"), 30);
    // The decoder hands ProRes 4444 out at 12 bits; what matters is the alpha plane.
    assert_eq!(coded_format(&path), ff::format::Pixel::YUVA444P12LE);
    let frames = decode(&path, rate);
    assert_eq!(frames.len(), FRAMES as usize);
    let (w, h, rgba) = &frames[7];
    assert_eq!((*w, *h), SIZE);
    let source = pattern_with_alpha(7);
    let (got, _) = rgba.as_chunks::<4>();
    let (want, _) = source.as_chunks::<4>();
    let mut error = 0u64;
    for (g, s) in got.iter().zip(want) {
        assert!(g[3].abs_diff(s[3]) <= 1, "alpha {} for {}", g[3], s[3]);
        if s[3] == 255 {
            error += (0..3).map(|c| u64::from(g[c].abs_diff(s[c]))).sum::<u64>();
        }
    }
    let opaque = want.iter().filter(|s| s[3] == 255).count() as f64 * 3.0;
    let mean = error as f64 / opaque;
    assert!(mean < 3.0, "mean error {mean:.2} per channel");
}
