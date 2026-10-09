//! Where an input's playhead is: from the playback settings and the video clock to the
//! frames to show and how to mix them. Pure maths, with no GPU or files.
//!
//! Frames are addressed by *virtual index*: consecutive whole numbers along the way the
//! playhead travels. A clip's virtual index keeps counting past its ends (the clip frame
//! is found by wrapping or folding it), so the frames around the playhead are always a
//! contiguous run, even across a loop point. A camera's virtual index is its frame
//! number.

use crate::blend::VideoFrame;
use crate::params::{Between, PlayMode, Slit};

/// The most texture-array layers a frame ring uses (wgpu's default
/// `max_texture_array_layers`).
pub const MAX_LAYERS: u32 = 256;

/// Video memory one frame ring may use.
pub const VRAM_BUDGET: u64 = 1 << 30;

/// A camera's frame rate when it can't be measured yet.
pub const FALLBACK_FPS: f64 = 30.0;

/// What the frames pass draws for one input this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// The playhead, as a fractional virtual index.
    pub base: f64,
    /// How many frames behind `base` the far end of the slit-scan map lies; negative
    /// when a clip plays backwards (behind is then ahead). 0 without slit-scan.
    pub reach: f64,
    /// The virtual indices any pixel may show, which must be on the GPU.
    pub first: i64,
    pub last: i64,
    pub between: Between,
    /// Off when there's no slit-scan this frame.
    pub slit: Slit,
    pub flip: bool,
}

impl Sample {
    /// The virtual indices to have on the GPU, oldest first.
    pub fn window(&self) -> std::ops::RangeInclusive<i64> {
        self.first..=self.last
    }
}

/// A clip's playback state between frames: where its virtual index starts, and the frame
/// Scrub showed last.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Playback {
    /// Added to clock × fps. None until the first frame, which then starts the clip at
    /// its first frame.
    offset: Option<f64>,
    /// The index Scrub showed on the previous frame, if it was in Scrub.
    scrubbed: Option<f64>,
}

impl Playback {
    /// The playhead's fractional virtual index for a clip of `count` frames at `fps`.
    /// Leaving Scrub carries on from the frame Scrub showed.
    pub fn index(&mut self, v: &VideoFrame, count: u32, fps: f64) -> f64 {
        if v.mode == PlayMode::Scrub {
            let index = scrub_index(v.position, count);
            self.scrubbed = Some(index);
            return index;
        }
        let playing = v.clock * fps;
        if let Some(shown) = self.scrubbed.take() {
            self.offset = Some(shown - playing);
        }
        playing + *self.offset.get_or_insert(-playing)
    }
}

/// Scrub's index for `position` (0–1) in a clip of `count` frames.
pub fn scrub_index(position: f32, count: u32) -> f64 {
    f64::from(position.clamp(0.0, 1.0)) * f64::from(count.saturating_sub(1))
}

/// The clip frame (0..count) that virtual index `k` shows.
pub fn clip_frame(mode: PlayMode, k: i64, count: u32) -> u32 {
    let count = i64::from(count.max(1));
    let frame = match mode {
        PlayMode::Loop => k.rem_euclid(count),
        PlayMode::PingPong if count == 1 => 0,
        PlayMode::PingPong => {
            // Forwards then backwards, without showing the end frames twice.
            let period = 2 * (count - 1);
            let m = k.rem_euclid(period);
            if m < count { m } else { period - m }
        }
        PlayMode::Scrub => k.clamp(0, count - 1),
    };
    frame as u32
}

/// The ring layer that holds virtual index `k`.
pub fn layer(k: i64, layers: u32) -> u32 {
    k.rem_euclid(i64::from(layers.max(1))) as u32
}

/// How many layers a frame ring may have, for frames of `layer_bytes` each on a device
/// allowing `device_layers`. At least 2, so the playhead can always blend.
pub fn max_layers(device_layers: u32, layer_bytes: u64) -> u32 {
    let budget = (VRAM_BUDGET / layer_bytes.max(1)).min(u64::from(u32::MAX)) as u32;
    MAX_LAYERS.min(device_layers).min(budget).max(2)
}

/// The deepest slit-scan, in seconds, a ring of `layers` layers holds at `fps`.
pub fn max_depth(layers: u32, fps: f64) -> f32 {
    (f64::from(layers.saturating_sub(2)) / fps.max(1e-6)) as f32
}

/// Slit-scan's reach in frames, held so the window fits in `layers` layers. 0 without
/// slit-scan.
fn reach_frames(v: &VideoFrame, fps: f64, layers: u32) -> f64 {
    if v.slit == Slit::Off {
        return 0.0;
    }
    (f64::from(v.slit_depth.max(0.0)) * fps).min(f64::from(layers.saturating_sub(2)))
}

/// What a clip shows this frame. `index` is the playhead from [`Playback::index`];
/// `layers` is the most layers its ring may have.
pub fn clip_sample(v: &VideoFrame, index: f64, count: u32, fps: f64, layers: u32) -> Sample {
    let depth = reach_frames(v, fps, layers);
    let (reach, first, last) = if v.mode == PlayMode::Scrub {
        // Behind is towards the first frame, which holds.
        let last_frame = i64::from(count.saturating_sub(1));
        let first = ((index - depth).floor() as i64).max(0);
        let last = (index.floor() as i64 + 1).min(last_frame);
        (depth, first, last.max(first))
    } else {
        // Behind is the way the playhead came.
        let reach = if v.speed < 0.0 { -depth } else { depth };
        let (lo, hi) = (index.min(index - reach), index.max(index - reach));
        (reach, lo.floor() as i64, hi.floor() as i64 + 1)
    };
    Sample {
        base: index,
        reach,
        first,
        last,
        between: v.between,
        slit: if depth > 0.0 { v.slit } else { Slit::Off },
        flip: v.slit_flip,
    }
}

/// When a camera frame arrived.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arrival {
    /// The camera's frame number (its virtual index).
    pub seq: i64,
    /// Seconds, on any clock that only moves forwards.
    pub time: f64,
}

/// The fractional frame number on show at `time`, between the frames that arrived
/// either side of it. Clamped to the frames there are. `arrivals` is oldest first and
/// not empty.
pub fn index_at(arrivals: &[Arrival], time: f64) -> f64 {
    let after = arrivals.partition_point(|a| a.time <= time);
    if after == 0 {
        return arrivals[0].seq as f64;
    }
    if after == arrivals.len() {
        return arrivals[after - 1].seq as f64;
    }
    let (a, b) = (arrivals[after - 1], arrivals[after]);
    let span = b.time - a.time;
    let fraction = if span > 0.0 {
        (time - a.time) / span
    } else {
        0.0
    };
    a.seq as f64 + fraction * (b.seq - a.seq) as f64
}

/// The camera's measured frame rate over the buffer.
pub fn camera_fps(arrivals: &[Arrival]) -> f64 {
    match (arrivals.first(), arrivals.last()) {
        (Some(a), Some(b)) if b.time > a.time => (b.seq - a.seq) as f64 / (b.time - a.time),
        _ => FALLBACK_FPS,
    }
}

/// What a camera shows this frame: live minus the delay, with slit-scan reaching further
/// back. Nothing before its first frame arrives.
pub fn camera_sample(v: &VideoFrame, arrivals: &[Arrival], layers: u32) -> Option<Sample> {
    let (oldest, newest) = (arrivals.first()?, arrivals.last()?);
    let base = index_at(arrivals, newest.time - f64::from(v.delay.max(0.0)));
    let depth = reach_frames(v, camera_fps(arrivals), layers);
    let first = ((base - depth).floor() as i64).max(oldest.seq);
    let last = (base.floor() as i64 + 1).min(newest.seq).max(first);
    Some(Sample {
        base,
        reach: depth,
        first,
        last,
        between: v.between,
        slit: if depth > 0.0 { v.slit } else { Slit::Off },
        flip: v.slit_flip,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend::FrameParams;
    use crate::params::Params;

    fn video(edit: impl FnOnce(&mut VideoFrame)) -> VideoFrame {
        let mut v = FrameParams::at_rest(&Params::default()).video[0];
        edit(&mut v);
        v
    }

    #[test]
    fn loop_wraps_both_ways() {
        assert_eq!(clip_frame(PlayMode::Loop, 9, 10), 9);
        assert_eq!(clip_frame(PlayMode::Loop, 10, 10), 0);
        assert_eq!(clip_frame(PlayMode::Loop, 23, 10), 3);
        assert_eq!(clip_frame(PlayMode::Loop, -1, 10), 9);
        assert_eq!(clip_frame(PlayMode::Loop, -11, 10), 9);
    }

    #[test]
    fn ping_pong_folds_without_doubling_the_ends() {
        let frames: Vec<u32> = (0..10)
            .map(|k| clip_frame(PlayMode::PingPong, k, 4))
            .collect();
        assert_eq!(frames, [0, 1, 2, 3, 2, 1, 0, 1, 2, 3]);
        assert_eq!(clip_frame(PlayMode::PingPong, -1, 4), 1);
        assert_eq!(clip_frame(PlayMode::PingPong, 7, 1), 0, "a one-frame clip");
    }

    #[test]
    fn scrub_maps_position_onto_the_clip() {
        assert_eq!(scrub_index(0.0, 61), 0.0);
        assert_eq!(scrub_index(0.5, 61), 30.0);
        assert_eq!(scrub_index(1.0, 61), 60.0);
        assert_eq!(clip_frame(PlayMode::Scrub, 99, 61), 60);
        assert_eq!(clip_frame(PlayMode::Scrub, -3, 61), 0);
    }

    #[test]
    fn a_new_clip_starts_at_its_first_frame_and_follows_the_clock() {
        let mut playback = Playback::default();
        let at = |clock| video(|v| v.clock = clock);
        assert_eq!(playback.index(&at(7.5), 48, 24.0), 0.0);
        assert_eq!(playback.index(&at(8.0), 48, 24.0), 12.0);
        // A clock running backwards (negative speed) counts down.
        assert_eq!(playback.index(&at(7.0), 48, 24.0), -12.0);
    }

    #[test]
    fn leaving_scrub_carries_on_from_the_frame_on_screen() {
        let mut playback = Playback::default();
        let scrub = video(|v| {
            v.mode = PlayMode::Scrub;
            v.position = 0.5;
            v.clock = 3.0;
        });
        assert_eq!(playback.index(&scrub, 21, 10.0), 10.0);
        let looping = |clock| video(|v| v.clock = clock);
        assert_eq!(playback.index(&looping(3.1), 21, 10.0), 10.0);
        assert!((playback.index(&looping(3.5), 21, 10.0) - 14.0).abs() < 1e-9);
    }

    #[test]
    fn nearest_and_blend_come_through() {
        let v = video(|v| v.between = Between::Nearest);
        let s = clip_sample(&v, 4.25, 10, 24.0, 256);
        assert_eq!((s.base, s.reach, s.between), (4.25, 0.0, Between::Nearest));
        assert_eq!(s.window(), 4..=5, "the two frames either side");
        assert_eq!(s.slit, Slit::Off);
        let s = clip_sample(&video(|_| {}), 4.25, 10, 24.0, 256);
        assert_eq!(s.between, Between::Blend);
    }

    #[test]
    fn slit_scan_reaches_back_the_way_the_clip_came() {
        let forward = video(|v| {
            v.slit = Slit::Rows;
            v.slit_depth = 0.5;
        });
        let s = clip_sample(&forward, 30.25, 100, 24.0, 256);
        assert_eq!((s.reach, s.slit), (12.0, Slit::Rows));
        assert_eq!(s.window(), 18..=31);
        let backward = video(|v| {
            v.slit = Slit::Rows;
            v.slit_depth = 0.5;
            v.speed = -1.0;
        });
        let s = clip_sample(&backward, 30.25, 100, 24.0, 256);
        assert_eq!(s.reach, -12.0);
        assert_eq!(s.window(), 30..=43);
    }

    #[test]
    fn scrub_slit_scan_holds_at_the_first_frame() {
        let v = video(|v| {
            v.mode = PlayMode::Scrub;
            v.slit = Slit::Columns;
            v.slit_depth = 1.0;
        });
        let s = clip_sample(&v, 5.0, 10, 24.0, 256);
        assert_eq!(s.window(), 0..=6);
        let s = clip_sample(&v, 9.0, 10, 24.0, 256);
        assert_eq!(s.window(), 0..=9, "never past the last frame");
    }

    #[test]
    fn slit_scan_depth_is_capped_by_the_ring() {
        let v = video(|v| {
            v.slit = Slit::Rows;
            v.slit_depth = 30.0;
        });
        let s = clip_sample(&v, 1000.5, 5000, 30.0, 64);
        assert_eq!(s.reach, 62.0);
        let size = s.last - s.first + 1;
        assert!(size <= 64, "{size} frames");
        assert_eq!(max_depth(64, 30.0), 62.0 / 30.0);
    }

    #[test]
    fn zero_depth_is_no_slit_scan() {
        let v = video(|v| {
            v.slit = Slit::Map;
            v.slit_depth = 0.0;
        });
        assert_eq!(clip_sample(&v, 3.0, 10, 24.0, 256).slit, Slit::Off);
    }

    #[test]
    fn ring_layers_wrap_and_respect_the_budgets() {
        assert_eq!(layer(5, 4), 1);
        assert_eq!(layer(-1, 4), 3);
        assert_eq!(max_layers(2048, 1280 * 720), 256);
        assert_eq!(max_layers(100, 1280 * 720), 100);
        // 1080p RGBA: about 8.3 MB a layer, so 1 GB holds 129.
        assert_eq!(max_layers(2048, 1920 * 1080 * 4), 129);
        assert_eq!(max_layers(2048, u64::MAX), 2);
    }

    fn arrivals(times: &[f64]) -> Vec<Arrival> {
        times
            .iter()
            .enumerate()
            .map(|(i, &time)| Arrival {
                seq: 100 + i as i64,
                time,
            })
            .collect()
    }

    #[test]
    fn camera_frames_are_found_by_arrival_time() {
        let a = arrivals(&[10.0, 10.1, 10.2, 10.4]);
        assert_eq!(index_at(&a, 10.2), 102.0);
        assert!((index_at(&a, 10.3) - 102.5).abs() < 1e-9);
        assert_eq!(index_at(&a, 9.0), 100.0, "older than the buffer");
        assert_eq!(index_at(&a, 11.0), 103.0);
        assert!((camera_fps(&a) - 7.5).abs() < 1e-9);
        assert_eq!(camera_fps(&a[..1]), FALLBACK_FPS);
    }

    #[test]
    fn camera_delay_and_slit_scan_stay_in_the_buffer() {
        let times: Vec<f64> = (0..31).map(|i| i as f64 / 30.0).collect();
        let a = arrivals(&times);
        let live = camera_sample(&video(|_| {}), &a, 256).unwrap();
        assert_eq!(live.base, 130.0);
        assert_eq!(live.window(), 130..=130);
        let delayed = camera_sample(&video(|v| v.delay = 0.5), &a, 256).unwrap();
        assert!((delayed.base - 115.0).abs() < 1e-6);
        let deep = camera_sample(
            &video(|v| {
                v.delay = 0.9;
                v.slit = Slit::Rows;
                v.slit_depth = 1.0;
            }),
            &a,
            256,
        )
        .unwrap();
        assert!((deep.reach - 30.0).abs() < 1e-6);
        assert_eq!(deep.first, 100, "clamped to the oldest frame");
        assert!(camera_sample(&video(|_| {}), &[], 256).is_none());
    }
}
