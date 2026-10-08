//! Paces canvas frames at the program frame rate, independently of the display's
//! refresh rate.

use crate::rate::FrameRate;

/// Most canvas frames drawn on one screen refresh when the app has fallen behind.
pub const MAX_CATCH_UP: u32 = 4;

/// How far, in frame periods, a refresh may sit on the wrong side of a due boundary
/// before it changes the count, once a frame has just been gained or lost. On a display
/// near the frame rate (60 fps on a 59.94 Hz panel) the refresh phase drifts slowly
/// through the half-period boundary, and while it sits there jitter would flip refreshes
/// between drawing 0 and 2 frames for up to a second. Holding the boundary back by this
/// much after it is crossed keeps each crossing to one uneven refresh, and frames still
/// follow wall-clock time to within half a period plus this.
const HYSTERESIS: f64 = 0.25;

/// How canvas frames are paced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pacing {
    /// Follow wall-clock time, catching up on late frames.
    RealTime,
    /// One frame per refresh, whatever the time (offline recording).
    Offline,
}

/// Decides how many canvas frames each screen refresh draws. Frame `anchor_frame` is due
/// at wall-clock time `anchor`, and frame `n` one period per frame after it.
#[derive(Clone, Debug)]
pub struct CanvasClock {
    rate: FrameRate,
    anchor: f64,
    anchor_frame: i64,
    /// Canvas frames drawn so far; also the index of the next one.
    drawn: i64,
    /// Frames that were due but skipped because the app fell too far behind.
    late: u64,
    /// Frames drawn by the last refresh that drew other than one (1 if none since the
    /// last re-anchor): which way the schedule last crossed a due boundary, for
    /// [`HYSTERESIS`].
    last: u32,
}

impl CanvasClock {
    /// A clock whose first frame is due at wall-clock time `now` (in seconds).
    pub fn new(rate: FrameRate, now: f64) -> Self {
        Self {
            rate,
            anchor: now,
            anchor_frame: 0,
            drawn: 0,
            late: 0,
            last: 1,
        }
    }

    pub fn rate(&self) -> FrameRate {
        self.rate
    }

    /// Frames skipped so far because the app fell more than [`MAX_CATCH_UP`] behind.
    pub fn late(&self) -> u64 {
        self.late
    }

    /// Makes the next frame due at `now`, as after a stall that shouldn't count as late
    /// (opening an encoder, a dialog). The new schedule starts mid-period, so no earlier
    /// boundary crossing carries over.
    pub fn reanchor(&mut self, now: f64) {
        self.anchor = now;
        self.anchor_frame = self.drawn;
        self.last = 1;
    }

    /// Switches to `rate`; the next frame is due at `now`.
    pub fn set_rate(&mut self, rate: FrameRate, now: f64) {
        self.rate = rate;
        self.reanchor(now);
    }

    /// How many canvas frames to draw on the refresh at `now`. Real-time pacing draws the
    /// frames that are due (a frame is due once `now` is within half a period of its time,
    /// so a display's jitter doesn't move frames between refreshes), up to
    /// [`MAX_CATCH_UP`]; frames further behind are skipped without ever falling due.
    /// After a refresh that drew 2+ frames, one that would draw none draws one if it is
    /// within [`HYSTERESIS`] of due; after a refresh that drew none, one that would draw 2
    /// draws one unless it is more than [`HYSTERESIS`] past due. So jitter can't undo a
    /// frame just gained or lost.
    /// Offline pacing always draws one frame and keeps real-time pacing in step with it.
    pub fn due(&mut self, now: f64, pacing: Pacing) -> u32 {
        let count = match pacing {
            Pacing::Offline => {
                // The frame drawn now counts as due now.
                self.anchor = now;
                self.anchor_frame = self.drawn;
                1
            }
            Pacing::RealTime => {
                // Frames due and not yet drawn, before rounding down.
                let exact = (now - self.anchor) * self.rate.fps()
                    + 0.5
                    + (self.anchor_frame + 1 - self.drawn) as f64;
                let mut behind = exact.floor() as i64;
                if self.last >= 2 && behind == 0 && exact + HYSTERESIS >= 1.0 {
                    behind = 1;
                }
                if self.last == 0 && behind == 2 && exact - HYSTERESIS < 2.0 {
                    behind = 1;
                }
                let behind = behind.max(0);
                let skipped = (behind - i64::from(MAX_CATCH_UP)).max(0);
                self.late += skipped as u64;
                self.anchor_frame -= skipped;
                (behind - skipped) as u32
            }
        };
        self.drawn += i64::from(count);
        if count != 1 {
            self.last = count;
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Refresh times of a 60 Hz display with ±0.3 ms of jitter.
    fn refresh(i: u32) -> f64 {
        let jitter = [0.0003, -0.0003, 0.0001, -0.0002, 0.0];
        f64::from(i) / 60.0 + jitter[i as usize % 5]
    }

    #[test]
    fn sixty_fps_on_a_60_hz_display_draws_one_frame_per_refresh() {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        let counts: Vec<u32> = (0..600)
            .map(|i| clock.due(refresh(i), Pacing::RealTime))
            .collect();
        assert!(counts.iter().all(|&n| n == 1), "{counts:?}");
        assert_eq!(clock.late(), 0);
    }

    #[test]
    fn twenty_four_fps_on_a_60_hz_display_alternates_two_and_three_refreshes() {
        let mut clock = CanvasClock::new(FrameRate::whole(24), 0.0);
        let counts: Vec<u32> = (0..10)
            .map(|i| clock.due(refresh(i), Pacing::RealTime))
            .collect();
        assert_eq!(counts, [1, 0, 1, 0, 1, 0, 0, 1, 0, 1]);
    }

    #[test]
    fn ntsc_rate_does_not_drift_over_an_hour() {
        let rate = FrameRate::ntsc(24);
        let mut clock = CanvasClock::new(rate, 0.0);
        let refreshes = 60 * 3600;
        let mut drawn = 0u64;
        for i in 0..refreshes {
            let n = clock.due(refresh(i), Pacing::RealTime);
            assert!(n <= 1, "refresh {i} drew {n} frames");
            drawn += u64::from(n);
        }
        let expected = rate.frames_in(refresh(refreshes - 1)) as u64 + 1;
        assert_eq!(drawn, expected);
        assert_eq!(clock.late(), 0);
    }

    /// Paces 60 fps for 60 s on a display at `hz` whose refreshes jitter by up to ±1 ms
    /// (deterministic pseudo-random). Returns the refreshes that drew other than one
    /// frame, the frames drawn, and the frames that the elapsed wall-clock time calls for.
    fn sixty_fps_near_60_hz(hz: f64) -> (usize, i64, f64) {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        let mut seed: u32 = 12345;
        let (mut uneven, mut drawn, mut now) = (0, 0, 0.0);
        for i in 0..(60.0 * hz) as u32 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let jitter = (f64::from(seed >> 8) / f64::from(1u32 << 24) - 0.5) * 0.002;
            now = f64::from(i) / hz + jitter;
            let n = clock.due(now, Pacing::RealTime);
            if n != 1 {
                uneven += 1;
            }
            drawn += i64::from(n);
        }
        assert_eq!(clock.late(), 0);
        (uneven, drawn, now * 60.0)
    }

    #[test]
    fn sixty_fps_near_60_hz_draws_one_uneven_refresh_per_beat() {
        for hz in [59.94, 59.95, 60.05] {
            let (uneven, drawn, wall_clock) = sixty_fps_near_60_hz(hz);
            // The refresh phase crosses a due boundary once per beat (every 1 / |hz − 60|
            // s), and that crossing must gain or lose a frame: one uneven refresh each,
            // plus one for a partial beat. Without hysteresis jitter makes dozens.
            let beats = (60.0 * (hz - 60.0)).abs();
            assert!(
                uneven as f64 <= beats.ceil() + 1.0,
                "{hz} Hz: {uneven} refreshes drew 0 or 2 frames over {beats:.1} beats"
            );
            // Frame 0 is drawn at time 0, so the count is one more than the elapsed frames.
            assert!(
                (drawn as f64 - (wall_clock + 1.0)).abs() <= 1.0,
                "{hz} Hz: drew {drawn} frames for {wall_clock:.2} frames of time"
            );
        }
    }

    #[test]
    fn catches_up_after_a_short_stall() {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        assert_eq!(clock.due(0.0, Pacing::RealTime), 1);
        assert_eq!(clock.due(3.0 / 60.0, Pacing::RealTime), 3);
        assert_eq!(clock.due(4.0 / 60.0, Pacing::RealTime), 1);
        assert_eq!(clock.late(), 0);
    }

    #[test]
    fn a_long_stall_draws_four_frames_and_skips_the_rest() {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        assert_eq!(clock.due(0.0, Pacing::RealTime), 1);
        assert_eq!(clock.due(10.0 / 60.0, Pacing::RealTime), MAX_CATCH_UP);
        assert_eq!(clock.late(), 6);
        assert_eq!(
            clock.due(11.0 / 60.0, Pacing::RealTime),
            1,
            "no longer behind"
        );
    }

    #[test]
    fn offline_draws_one_frame_per_refresh_and_hands_back_without_a_jump() {
        let mut clock = CanvasClock::new(FrameRate::whole(24), 0.0);
        assert_eq!(clock.due(0.0, Pacing::RealTime), 1);
        for i in 1..=5 {
            assert_eq!(clock.due(f64::from(i) * 0.5, Pacing::Offline), 1);
        }
        assert_eq!(clock.due(2.5 + 1.0 / 60.0, Pacing::RealTime), 0);
        assert_eq!(clock.due(2.5 + 1.0 / 24.0, Pacing::RealTime), 1);
        assert_eq!(clock.late(), 0);
    }

    #[test]
    fn reanchoring_forgives_a_stall() {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        assert_eq!(clock.due(0.0, Pacing::RealTime), 1);
        clock.reanchor(5.0);
        assert_eq!(clock.due(5.0, Pacing::RealTime), 1);
        assert_eq!(clock.due(5.0 + 1.0 / 60.0, Pacing::RealTime), 1);
        assert_eq!(clock.late(), 0);
    }

    #[test]
    fn changing_the_rate_paces_from_the_change() {
        let mut clock = CanvasClock::new(FrameRate::whole(60), 0.0);
        assert_eq!(clock.due(0.0, Pacing::RealTime), 1);
        clock.set_rate(FrameRate::whole(25), 1.0);
        assert_eq!(clock.rate(), FrameRate::whole(25));
        assert_eq!(clock.due(1.0, Pacing::RealTime), 1);
        assert_eq!(clock.due(1.01, Pacing::RealTime), 0);
        assert_eq!(clock.due(1.04, Pacing::RealTime), 1);
    }
}
