//! Video inputs: clips written with the capture encoder decode back into the frames they
//! were made from; cameras deliver frames when one is connected.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32};

use rasterwarp::capture::encode::{Encoder, Frame, VideoFormat};
use rasterwarp::rate::FrameRate;
use rasterwarp::video::clip::{self, Loading};
use rasterwarp::video::{Budget, MEMORY_LIMIT, Pixels};

const SIZE: (u32, u32) = (320, 180);
const FRAMES: i64 = 24;

/// A fresh path in a per-test temporary directory.
fn temp_file(test: &str, file: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rasterwarp-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.join(file)
}

/// Frame `n` is flat gray at level `10 n`, with a white top-left pixel.
fn level(n: i64) -> u8 {
    (n * 10) as u8
}

/// A lossless 24 fps clip of [`FRAMES`] flat frames.
fn write_clip(path: &Path) {
    let encoder = Encoder::start(path, VideoFormat::Ffv1, SIZE, FrameRate::whole(24)).unwrap();
    for pts in 0..FRAMES {
        let g = level(pts);
        let mut rgba = [g, g, g, 255].repeat((SIZE.0 * SIZE.1) as usize);
        rgba[..4].copy_from_slice(&[255, 255, 255, 255]);
        encoder.send(Frame { pts, rgba }).unwrap();
    }
    encoder.finish().unwrap();
}

fn load(
    path: &Path,
    pixels: Pixels,
    canvas: (u32, u32),
    budget: &Budget,
) -> anyhow::Result<clip::Clip> {
    clip::load(
        path,
        pixels,
        canvas,
        budget,
        &AtomicU32::new(0),
        &AtomicBool::new(false),
    )
}

#[test]
fn clips_decode_every_frame_scaled_to_the_canvas() {
    let path = temp_file("clip-luma", "clip.mkv");
    write_clip(&path);
    let budget = Budget::new(MEMORY_LIMIT);
    let clip = load(&path, Pixels::Luma, (160, 160), &budget).unwrap();
    assert_eq!(clip.count(), FRAMES as u32);
    assert!((clip.fps - 24.0).abs() < 1e-6, "{} fps", clip.fps);
    assert_eq!(clip.format.size, (160, 90), "fitted inside the canvas");
    assert_eq!(clip.format.small, (160, 90), "never scaled up");
    for (n, frame) in clip.frames.iter().enumerate() {
        assert_eq!(frame.full.len(), 160 * 90);
        let middle = frame.full[45 * 160 + 80];
        let expected = level(n as i64);
        assert!(
            middle.abs_diff(expected) <= 2,
            "frame {n}: {middle} vs {expected}"
        );
    }
    let held = budget.clone();
    assert!(held.available() < MEMORY_LIMIT, "the clip holds its memory");
    drop(clip);
    assert_eq!(budget.available(), MEMORY_LIMIT, "and gives it back");
}

#[test]
fn background_clips_keep_their_color() {
    let path = temp_file("clip-rgba", "clip.mkv");
    write_clip(&path);
    let budget = Budget::new(MEMORY_LIMIT);
    let clip = load(&path, Pixels::Rgba, (1920, 1080), &budget).unwrap();
    assert_eq!(clip.format.size, SIZE);
    assert_eq!(clip.format.small, SIZE);
    let frame = &clip.frames[5];
    assert_eq!(frame.full.len(), (SIZE.0 * SIZE.1 * 4) as usize);
    assert_eq!(&frame.full[..4], &[255, 255, 255, 255]);
    let px = &frame.full[(90 * SIZE.0 as usize + 160) * 4..][..4];
    for c in &px[..3] {
        assert!(c.abs_diff(50) <= 1, "{px:?}");
    }
    assert_eq!(px[3], 255);
}

#[test]
fn clips_over_the_memory_budget_are_refused() {
    let path = temp_file("clip-budget", "clip.mkv");
    write_clip(&path);
    let budget = Budget::new(1 << 20);
    let err = load(&path, Pixels::Rgba, (1920, 1080), &budget).unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("clip.mkv needs 11 MB of memory; shorten it or lower the canvas size"),
        "{message}"
    );
    assert_eq!(budget.available(), 1 << 20);
}

#[test]
fn unreadable_files_name_themselves() {
    let path = temp_file("clip-bad", "broken.mp4");
    std::fs::write(&path, b"not a video").unwrap();
    let err = load(&path, Pixels::Luma, (640, 360), &Budget::new(MEMORY_LIMIT)).unwrap_err();
    assert!(
        format!("{err:#}").starts_with("could not open broken.mp4"),
        "{err:#}"
    );
}

#[test]
fn clips_load_on_a_worker_thread() {
    let path = temp_file("clip-worker", "clip.mkv");
    write_clip(&path);
    let loading = Loading::start(&path, Pixels::Luma, (320, 180), &Budget::new(MEMORY_LIMIT));
    let clip = loading.wait().unwrap();
    assert_eq!(clip.count(), FRAMES as u32);
    assert_eq!(loading.progress(), 1.0);
}
