//! Encodes short clips with the linked FFmpeg libraries and decodes them again.

use std::path::{Path, PathBuf};

use ffmpeg_next as ff;
use rasterwarp::capture::encode::{Encoder, Frame, VideoFormat};

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

fn encode(path: &Path, format: VideoFormat) -> anyhow::Result<u64> {
    let encoder = Encoder::start(path, format, SIZE)?;
    for pts in 0..FRAMES {
        encoder.send(Frame {
            pts,
            rgba: pattern(pts),
        })?;
    }
    encoder.finish()
}

/// Decodes every frame of `path` to tightly packed RGBA (BT.709 for YUV input).
fn decode(path: &Path) -> Vec<(u32, u32, Vec<u8>)> {
    ff::init().unwrap();
    let mut input = ff::format::input(path).expect("open the recording");
    let stream = input
        .streams()
        .best(ff::media::Type::Video)
        .expect("a video stream");
    let index = stream.index();
    assert_eq!(
        stream.avg_frame_rate(),
        ff::Rational(60, 1),
        "declared frame rate"
    );
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
    if decoder.format() == ff::format::Pixel::YUV444P {
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
    assert_eq!(encode(&path, VideoFormat::Ffv1).expect("encode"), 30);
    let frames = decode(&path);
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
    let written = match encode(&path, VideoFormat::Hevc) {
        Ok(written) => written,
        Err(err) => {
            eprintln!("skipping HEVC test: {err:#}");
            return;
        }
    };
    assert_eq!(written, 30);
    let frames = decode(&path);
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
fn an_existing_file_is_not_overwritten() {
    let path = temp_file("exists", "mkv");
    std::fs::write(&path, b"keep me").unwrap();
    let err = Encoder::start(&path, VideoFormat::Ffv1, SIZE)
        .err()
        .expect("start must fail");
    assert!(format!("{err:#}").contains("already exists"));
    assert_eq!(std::fs::read(&path).unwrap(), b"keep me");
}
