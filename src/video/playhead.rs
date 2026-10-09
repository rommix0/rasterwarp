//! Where an input's playhead is: from the playback settings and the video clock to the
//! frames to show and how to mix them. Pure maths, with no GPU or files.
//!
//! Frames are addressed by *virtual index*: consecutive whole numbers along the way the
//! playhead travels. A clip's virtual index keeps counting past its ends (the clip frame
//! is found by wrapping or folding it), so the frames around the playhead are always a
//! contiguous run, even across a loop point. A camera's virtual index is its frame
//! number.
//!
//! Slit-scan on a playing clip shows where the playhead actually was: each depth looks up
//! the clip's [`Trail`], so slowing down, stopping and reversing all stay continuous.
//!
//! A camera loop holds the camera's last frames and plays them as a clip, starting on the
//! frame that was on show (see [`loop_span`]).

use std::collections::VecDeque;

use crate::blend::VideoFrame;
use crate::params::{Between, PlayMode, Slit};

/// The most texture-array layers a frame ring uses (wgpu's default
/// `max_texture_array_layers`).
pub const MAX_LAYERS: u32 = 256;

/// Video memory one frame ring may use.
pub const VRAM_BUDGET: u64 = 1 << 30;

/// A camera's frame rate when it can't be measured yet.
pub const FALLBACK_FPS: f64 = 30.0;

/// Steps in a sample's slit-scan table, from map 0 (now) to map 1 (the full depth).
pub const SLIT_STEPS: usize = 64;

/// What the frames pass draws for one input this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// The playhead, as a fractional virtual index.
    pub base: f64,
    /// How many frames behind `base` each slit-scan map value shows, at map values
    /// `i / (SLIT_STEPS - 1)`; negative where it lies ahead (a clip that was playing
    /// backwards). All 0 without slit-scan.
    pub behind: [f32; SLIT_STEPS],
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

/// A slit-scan table reaching evenly back to `reach` frames behind the playhead.
pub fn straight_behind(reach: f64) -> [f32; SLIT_STEPS] {
    std::array::from_fn(|i| (reach * i as f64 / (SLIT_STEPS - 1) as f64) as f32)
}

/// Where a clip's playhead has been, for slit-scan: its virtual index at moments of
/// animation time, which stands still while paused.
#[derive(Clone, Debug, Default)]
pub struct Trail {
    /// (time, index), oldest first, with times rising.
    points: VecDeque<(f64, f64)>,
}

impl Trail {
    /// The history of a playhead that has moved steadily at `fps` frames a second and
    /// is about to reach `index` around `time`: a live camera's, carried into a loop. It
    /// holds the point `keep` seconds back; recording the playhead's next position
    /// completes the line, wherever between canvas frames the loop began.
    pub fn approaching(time: f64, index: f64, fps: f64, keep: f64) -> Self {
        let mut trail = Self::default();
        trail.record(time - keep, index - keep * fps, keep);
        trail
    }

    /// Notes that the playhead is at `index` at `time`, keeping `keep` seconds of history.
    pub fn record(&mut self, time: f64, index: f64, keep: f64) {
        match self.points.back_mut() {
            // Paused: the moment hasn't moved on.
            Some(last) if time <= last.0 => last.1 = index,
            _ => self.points.push_back((time, index)),
        }
        // Keeps one point at or before the horizon, to look up the moment there.
        while self.points.len() > 2 && self.points[1].0 <= time - keep {
            self.points.pop_front();
        }
    }

    /// The fractional index the playhead was on `ago` seconds before the latest point;
    /// the oldest one before the history starts. None before anything is recorded.
    pub fn at(&self, ago: f64) -> Option<f64> {
        let &(now, newest) = self.points.back()?;
        let time = now - ago.max(0.0);
        let after = self.points.partition_point(|&(t, _)| t <= time);
        if after == self.points.len() {
            return Some(newest);
        }
        if after == 0 {
            return Some(self.points[0].1);
        }
        let ((t0, a), (t1, b)) = (self.points[after - 1], self.points[after]);
        Some(a + (b - a) * (time - t0) / (t1 - t0))
    }
}

/// The table for a playhead at `index` whose history is `trail`, reaching back `depth`
/// seconds, held so its frames span at most `span` frames around the playhead.
fn trail_behind(trail: &Trail, index: f64, depth: f64, span: f64) -> [f32; SLIT_STEPS] {
    let (mut lo, mut hi) = (index, index);
    std::array::from_fn(|i| {
        let ago = depth * i as f64 / (SLIT_STEPS - 1) as f64;
        let then = trail.at(ago).unwrap_or(index).clamp(hi - span, lo + span);
        (lo, hi) = (lo.min(then), hi.max(then));
        (index - then) as f32
    })
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
    /// Playback that carries on from `index`, as it does when leaving Scrub.
    pub fn continuing(index: f64) -> Self {
        Self {
            offset: None,
            scrubbed: Some(index),
        }
    }

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

/// What a clip shows this frame. `index` is the playhead from [`Playback::index`], and
/// `trail` where it has been (see [`Trail::record`]); `layers` is the most layers its
/// ring may have.
pub fn clip_sample(
    v: &VideoFrame,
    index: f64,
    trail: &Trail,
    count: u32,
    fps: f64,
    layers: u32,
) -> Sample {
    let depth = reach_frames(v, fps, layers);
    let (behind, first, last) = if v.mode == PlayMode::Scrub {
        // Behind is towards the first frame, which holds.
        let last_frame = i64::from(count.saturating_sub(1));
        let first = ((index - depth).floor() as i64).max(0);
        let last = (index.floor() as i64 + 1).min(last_frame);
        (straight_behind(depth), first, last.max(first))
    } else {
        // Behind is where the playhead was, whichever way it went.
        let behind = if depth > 0.0 {
            let span = f64::from(layers.saturating_sub(2));
            trail_behind(trail, index, f64::from(v.slit_depth), span)
        } else {
            [0.0; SLIT_STEPS]
        };
        let shown = behind.iter().map(|&b| index - f64::from(b));
        let lo = shown.clone().fold(index, f64::min);
        let hi = shown.fold(index, f64::max);
        (behind, lo.floor() as i64, hi.floor() as i64 + 1)
    };
    Sample {
        base: index,
        behind,
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

/// The frames a camera loop of `seconds` holds, ending at the newest of `arrivals`:
/// the first and last frame numbers, and where in the loop the playhead starts. It starts
/// on the frame a delay of `seconds` is showing, so the picture carries on. None if that
/// is under two frames.
pub fn loop_span(arrivals: &[Arrival], seconds: f64) -> Option<(i64, i64, f64)> {
    let newest = arrivals.last()?;
    let start = index_at(arrivals, newest.time - seconds.max(0.0));
    let first = start.floor() as i64;
    (newest.seq > first).then_some((first, newest.seq, start - first as f64))
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
        behind: straight_behind(depth),
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
        let s = clip_sample(&v, 4.25, &Trail::default(), 10, 24.0, 256);
        assert_eq!((s.base, s.between), (4.25, Between::Nearest));
        assert_eq!(s.behind, [0.0; SLIT_STEPS]);
        assert_eq!(s.window(), 4..=5, "the two frames either side");
        assert_eq!(s.slit, Slit::Off);
        let s = clip_sample(&video(|_| {}), 4.25, &Trail::default(), 10, 24.0, 256);
        assert_eq!(s.between, Between::Blend);
    }

    /// A trail of a playhead sampled 60 times a second from `from` to `to` seconds,
    /// continuing `trail`, at the index `at` gives for each time.
    fn play(trail: &mut Trail, from: f64, to: f64, at: impl Fn(f64) -> f64) -> f64 {
        let mut index = at(from);
        for frame in 0..=((to - from) * 60.0).round() as i64 {
            let time = from + frame as f64 / 60.0;
            index = at(time);
            trail.record(time, index, 30.0);
        }
        index
    }

    fn close(a: f32, b: f64) -> bool {
        (f64::from(a) - b).abs() < 1e-3
    }

    #[test]
    fn the_trail_looks_up_where_the_playhead_was() {
        let mut trail = Trail::default();
        assert_eq!(trail.at(0.0), None);
        play(&mut trail, 0.0, 2.0, |t| 24.0 * t);
        assert!((trail.at(0.0).unwrap() - 48.0).abs() < 1e-9);
        assert!((trail.at(0.5).unwrap() - 36.0).abs() < 1e-9);
        assert!(
            (trail.at(0.51).unwrap() - 35.76).abs() < 1e-9,
            "between samples"
        );
        assert_eq!(trail.at(10.0), Some(0.0), "before the history starts");
        // Paused: the same moment again only moves the latest index.
        trail.record(2.0, 50.0, 30.0);
        assert_eq!(trail.at(0.0), Some(50.0));
        // Only `keep` seconds (and one point before them) are kept: the moment at 1 s
        // is gone, so the oldest point left (2 s) stands in for it.
        trail.record(40.0, 60.0, 1.0);
        assert_eq!(trail.at(39.0), Some(50.0));
        let halfway = trail.at(19.0).unwrap();
        assert!((halfway - 55.0).abs() < 1e-9, "{halfway}");
    }

    #[test]
    fn slit_scan_shows_where_the_playhead_was() {
        let v = video(|v| {
            v.slit = Slit::Rows;
            v.slit_depth = 0.5;
        });
        // Playing forwards at 24 fps: the full depth is 12 frames back.
        let mut trail = Trail::default();
        let index = play(&mut trail, 0.0, 2.0, |t| 24.0 * t);
        let s = clip_sample(&v, index, &trail, 100, 24.0, 256);
        assert_eq!(s.slit, Slit::Rows);
        assert!(close(s.behind[0], 0.0) && close(s.behind[SLIT_STEPS - 1], 12.0));
        assert_eq!(s.window(), 36..=49);
        // Reversed for a quarter of a second: the recent past lies ahead of the
        // playhead, and the depth's far end has come back round to it. No jump.
        let index = play(&mut trail, 2.0, 2.25, |t| 48.0 - 24.0 * (t - 2.0));
        let s = clip_sample(&v, index, &trail, 100, 24.0, 256);
        // The turn falls between two of the table's steps.
        let most_ahead = s.behind.iter().copied().fold(0.0, f32::min);
        assert!((most_ahead + 6.0).abs() < 0.2, "{most_ahead}");
        assert!(close(s.behind[SLIT_STEPS - 1], 0.0));
        assert_eq!(s.window(), 42..=48);
        // Stopped for longer than the depth: every row shows the still frame.
        let index = play(&mut trail, 2.25, 3.0, |_| 42.0);
        let s = clip_sample(&v, index, &trail, 100, 24.0, 256);
        assert!(s.behind.iter().all(|&b| close(b, 0.0)));
        assert_eq!(s.window(), 42..=43);
    }

    #[test]
    fn slit_scan_starts_from_the_playhead_without_a_trail() {
        let v = video(|v| {
            v.slit = Slit::Columns;
            v.slit_depth = 1.0;
        });
        let s = clip_sample(&v, 7.5, &Trail::default(), 100, 24.0, 256);
        assert!(s.behind.iter().all(|&b| b == 0.0));
        assert_eq!(s.window(), 7..=8);
    }

    #[test]
    fn scrub_slit_scan_holds_at_the_first_frame() {
        let v = video(|v| {
            v.mode = PlayMode::Scrub;
            v.slit = Slit::Columns;
            v.slit_depth = 1.0;
        });
        let s = clip_sample(&v, 5.0, &Trail::default(), 10, 24.0, 256);
        assert_eq!(s.window(), 0..=6);
        let s = clip_sample(&v, 9.0, &Trail::default(), 10, 24.0, 256);
        assert_eq!(s.window(), 0..=9, "never past the last frame");
    }

    #[test]
    fn slit_scan_depth_is_capped_by_the_ring() {
        let v = video(|v| {
            v.slit = Slit::Rows;
            v.slit_depth = 30.0;
        });
        // At 4× a 30 fps clip, 30 s back is 3600 frames; the ring holds 64.
        let mut trail = Trail::default();
        let index = play(&mut trail, 0.0, 40.0, |t| 120.0 * t);
        let s = clip_sample(&v, index, &trail, 5000, 30.0, 64);
        assert!(close(s.behind[SLIT_STEPS - 1], 62.0));
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
        assert_eq!(
            clip_sample(&v, 3.0, &Trail::default(), 10, 24.0, 256).slit,
            Slit::Off
        );
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
        assert!(close(deep.behind[SLIT_STEPS - 1], 30.0));
        assert_eq!(deep.first, 100, "clamped to the oldest frame");
        assert!(camera_sample(&video(|_| {}), &[], 256).is_none());
    }

    #[test]
    fn a_camera_loop_holds_the_delay_and_starts_on_the_frame_on_show() {
        let times: Vec<f64> = (0..31).map(|i| i as f64 / 30.0).collect();
        let a = arrivals(&times);
        // Half a second: from the frame a 0.5 s delay shows up to the newest.
        assert_eq!(loop_span(&a, 0.5), Some((115, 130, 0.0)));
        // Between frames, the playhead starts part-way into the first.
        let (first, last, start) = loop_span(&a, 0.51).unwrap();
        assert_eq!((first, last), (114, 130));
        assert!((start - 0.7).abs() < 1e-6);
        assert_eq!(
            loop_span(&a, 5.0),
            Some((100, 130, 0.0)),
            "the whole buffer"
        );
        assert_eq!(loop_span(&a, 0.02).map(|s| (s.0, s.1)), Some((129, 130)));
        assert_eq!(loop_span(&a, 0.0), None, "one frame is no loop");
        assert_eq!(loop_span(&[], 1.0), None);
    }

    #[test]
    fn a_camera_loop_carries_on_from_the_frame_on_show() {
        let mut playback = Playback::continuing(0.7);
        let at = |clock| video(|v| v.clock = clock);
        assert!((playback.index(&at(5.0), 16, 30.0) - 0.7).abs() < 1e-9);
        assert!((playback.index(&at(5.5), 16, 30.0) - 15.7).abs() < 1e-9);
    }

    #[test]
    fn a_camera_loop_carries_on_the_live_slit_scan() {
        // Live at 30 fps reaches loop index 0.7 at time 8, where the loop plays it.
        let mut trail = Trail::approaching(8.0, 0.7, 30.0, 2.0);
        trail.record(8.0, 0.7, 2.0);
        assert!((trail.at(0.0).unwrap() - 0.7).abs() < 1e-9);
        assert!((trail.at(1.0).unwrap() + 29.3).abs() < 1e-9);
        let v = video(|v| {
            v.slit = Slit::Rows;
            v.slit_depth = 1.0;
        });
        let sample = clip_sample(&v, 0.7, &trail, 16, 30.0, 256);
        for (got, live) in sample.behind.iter().zip(straight_behind(30.0)) {
            assert!(close(*got, f64::from(live)), "{got} for {live}");
        }
    }

    #[test]
    fn slit_scan_stays_continuous_across_a_loop_point() {
        // A 16-frame loop playing at 30 fps for 3 s wraps several times.
        let mut trail = Trail::default();
        let index = play(&mut trail, 0.0, 3.0, |t| 30.0 * t);
        let v = video(|v| {
            v.slit = Slit::Rows;
            v.slit_depth = 1.0;
        });
        let sample = clip_sample(&v, index, &trail, 16, 30.0, 256);
        for (i, pair) in sample.behind.windows(2).enumerate() {
            let step = pair[1] - pair[0];
            assert!(
                (step - 30.0 / 63.0).abs() < 1e-3,
                "row {i} jumps by {step} frames"
            );
        }
    }
}
