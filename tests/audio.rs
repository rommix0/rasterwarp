//! Decoding real sound files.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32};

use rasterwarp::audio::Shaping;
use rasterwarp::audio::file;
use rasterwarp::video::{Budget, MEMORY_LIMIT};

#[test]
fn drums_decode_to_48_khz_stereo_with_beats() {
    let budget = Budget::new(MEMORY_LIMIT);
    let progress = AtomicU32::new(0);
    let sound = file::load(
        Path::new("samples/drums.wav"),
        Shaping::default(),
        &budget,
        &progress,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(sound.frames(), 420_612);
    assert!((sound.seconds() - 8.7628).abs() < 0.001);
    assert_eq!(sound.name, "drums.wav");
    assert!(
        sound.tracks.beats_in(0.0, sound.seconds())[0],
        "drums have beats"
    );
    assert_eq!(progress.load(std::sync::atomic::Ordering::Relaxed), 1000);
}

#[test]
fn a_file_without_sound_says_so() {
    let budget = Budget::new(MEMORY_LIMIT);
    let err = file::load(
        Path::new("samples/displace_test.png"),
        Shaping::default(),
        &budget,
        &AtomicU32::new(0),
        &AtomicBool::new(false),
    )
    .expect_err("a picture has no sound");
    assert!(
        format!("{err:#}").starts_with("Couldn't read displace_test.png: "),
        "{err:#}"
    );
}
