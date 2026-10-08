//! The program frame rate: every canvas frame advances animation by exactly one period,
//! and recordings run at the same rate.

/// A frame rate as an exact fraction, `num / den` frames per second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameRate {
    pub num: i32,
    pub den: i32,
}

impl FrameRate {
    /// The rates the panel offers, slowest first.
    pub const ALL: [FrameRate; 8] = [
        FrameRate::ntsc(24),
        FrameRate::whole(24),
        FrameRate::whole(25),
        FrameRate::ntsc(30),
        FrameRate::whole(30),
        FrameRate::whole(50),
        FrameRate::ntsc(60),
        FrameRate::whole(60),
    ];

    /// `fps` frames per second exactly.
    pub const fn whole(fps: i32) -> Self {
        Self { num: fps, den: 1 }
    }

    /// The NTSC rate just below `fps`: `fps × 1000/1001` (23.976, 29.97, 59.94).
    pub const fn ntsc(fps: i32) -> Self {
        Self {
            num: fps * 1000,
            den: 1001,
        }
    }

    /// Frames per second.
    pub fn fps(self) -> f64 {
        f64::from(self.num) / f64::from(self.den)
    }

    /// Seconds per frame.
    pub fn period(self) -> f64 {
        f64::from(self.den) / f64::from(self.num)
    }

    /// Seconds of `frames` frames.
    pub fn seconds(self, frames: i64) -> f64 {
        frames as f64 * self.period()
    }

    /// Whole frames in `seconds`, rounded to the nearest frame.
    pub fn frames_in(self, seconds: f64) -> i64 {
        (seconds * self.fps()).round() as i64
    }

    /// Frames between keyframes in a recording: about one second.
    pub fn keyframe_interval(self) -> u32 {
        self.fps().round() as u32
    }

    /// "24 fps", "23.976 fps", "29.97 fps".
    pub fn label(self) -> String {
        let fps = format!("{:.3}", self.fps());
        let fps = fps.trim_end_matches('0').trim_end_matches('.');
        format!("{fps} fps")
    }
}

impl Default for FrameRate {
    fn default() -> Self {
        FrameRate::whole(60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntsc_rates_are_exact_fractions() {
        assert_eq!(
            FrameRate::ntsc(24),
            FrameRate {
                num: 24000,
                den: 1001
            }
        );
        assert!((FrameRate::ntsc(30).fps() - 29.97003).abs() < 1e-5);
        assert!((FrameRate::whole(25).period() - 0.04).abs() < 1e-12);
    }

    #[test]
    fn labels_drop_trailing_zeros() {
        let labels: Vec<String> = FrameRate::ALL.iter().map(|r| r.label()).collect();
        assert_eq!(
            labels,
            [
                "23.976 fps",
                "24 fps",
                "25 fps",
                "29.97 fps",
                "30 fps",
                "50 fps",
                "59.94 fps",
                "60 fps"
            ]
        );
    }

    #[test]
    fn frames_and_seconds_round_trip() {
        let rate = FrameRate::ntsc(24);
        assert_eq!(rate.frames_in(rate.seconds(1000)), 1000);
        assert_eq!(FrameRate::whole(60).frames_in(10.0 / 60.0), 10);
        assert_eq!(rate.keyframe_interval(), 24);
        assert_eq!(FrameRate::default(), FrameRate::whole(60));
    }
}
