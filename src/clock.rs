//! Paces canvas frames at the program frame rate, independently of the display's
//! refresh rate.

use crate::rate::FrameRate;

/// Most canvas frames drawn on one screen refresh when the app has fallen behind.
pub const MAX_CATCH_UP: u32 = 4;

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
    /// (opening an encoder, a dialog).
    pub fn reanchor(&mut self, now: f64) {
        self.anchor = now;
        self.anchor_frame = self.drawn;
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
                let newest = self.anchor_frame
                    + ((now - self.anchor) * self.rate.fps() + 0.5).floor() as i64;
                let behind = (newest + 1 - self.drawn).max(0);
                let skipped = (behind - i64::from(MAX_CATCH_UP)).max(0);
                self.late += skipped as u64;
                self.anchor_frame -= skipped;
                (behind - skipped) as u32
            }
        };
        self.drawn += i64::from(count);
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
