//! The analysis shared by live input and sound files: band filters, envelope followers
//! with attack and release, and beat (onset) detection, one sample at a time.

use std::f32::consts::TAU;

use super::Shaping;

/// The top of the bass band, in hertz.
pub const BASS_TOP: f32 = 200.0;
/// The bottom of the treble band, in hertz.
pub const TREBLE_BOTTOM: f32 = 2000.0;
/// How long the detector averages the squared signal, in seconds: long enough to smooth
/// a 50 Hz bass note, short enough for drums.
const DETECTOR: f32 = 0.02;
/// How long the slow average that beats are compared against takes, in seconds.
const SLOW: f32 = 1.0;
/// A band fires at most this often, in seconds.
pub const HOLD_OFF: f32 = 0.1;
/// Below this a band never fires (quiet hiss, and the faint broadband snap where a hard-cut
/// burst starts or stops, which reaches about 0.03 in the treble band).
const FLOOR: f32 = 0.05;
/// Butterworth Q for the low-pass and high-pass filters.
const BUTTERWORTH: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// The per-sample smoothing coefficient for a one-pole filter with time constant
/// `seconds` at `rate` samples per second.
fn coefficient(seconds: f32, rate: f32) -> f32 {
    1.0 - (-1.0 / (seconds * rate).max(f32::EPSILON)).exp()
}

/// How far above its slow average a band must jump to fire, for a sensitivity of 0
/// (3×) to 1 (1.2×).
fn jump_ratio(sensitivity: f32) -> f32 {
    3.0 - 1.8 * sensitivity.clamp(0.0, 1.0)
}

/// A second-order IIR filter (the RBJ cookbook's biquad).
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
}

impl Biquad {
    fn new(b: [f32; 3], a0: f32, a: [f32; 2]) -> Self {
        Self {
            b: b.map(|v| v / a0),
            a: a.map(|v| v / a0),
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    /// Lets everything through (the Level band).
    fn pass() -> Self {
        Self::new([1.0, 0.0, 0.0], 1.0, [0.0, 0.0])
    }

    fn low_pass(rate: f32, freq: f32) -> Self {
        let (cos, alpha) = Self::angle(rate, freq, BUTTERWORTH);
        Self::new(
            [(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0],
            1.0 + alpha,
            [-2.0 * cos, 1.0 - alpha],
        )
    }

    fn high_pass(rate: f32, freq: f32) -> Self {
        let (cos, alpha) = Self::angle(rate, freq, BUTTERWORTH);
        Self::new(
            [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0],
            1.0 + alpha,
            [-2.0 * cos, 1.0 - alpha],
        )
    }

    /// A band-pass from `low` to `high` hertz with 0 dB at its centre.
    fn band_pass(rate: f32, low: f32, high: f32) -> Self {
        let centre = (low * high).sqrt();
        let q = centre / (high - low);
        let (cos, alpha) = Self::angle(rate, centre, q);
        Self::new([alpha, 0.0, -alpha], 1.0 + alpha, [-2.0 * cos, 1.0 - alpha])
    }

    fn angle(rate: f32, freq: f32, q: f32) -> (f32, f32) {
        let w = TAU * freq / rate;
        (w.cos(), w.sin() / (2.0 * q))
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

/// One band: its filter, its follower and its beat detector.
#[derive(Clone, Copy, Debug)]
struct Band {
    filter: Biquad,
    /// The detector's mean square.
    power: f32,
    /// The follower's output: the band's value before gain.
    value: f32,
    /// The slow average of `value` that jumps are measured against.
    slow: f32,
    /// Whether the band has fallen back below its threshold since it last fired.
    armed: bool,
    /// Seconds since the band last fired.
    since: f32,
}

impl Band {
    fn new(filter: Biquad) -> Self {
        Self {
            filter,
            power: 0.0,
            value: 0.0,
            slow: 0.0,
            armed: true,
            since: HOLD_OFF,
        }
    }
}

/// The bands, in the order [`Analyzer::values`] returns them.
const LEVEL: usize = 0;
const BASS: usize = 1;
const TREBLE: usize = 3;

/// Turns samples into Level, Bass, Mid and Treble values and beats.
#[derive(Clone, Debug)]
pub struct Analyzer {
    bands: [Band; 4],
    /// Seconds per sample.
    dt: f32,
    rate: f32,
    detector: f32,
    slow: f32,
    attack: f32,
    release: f32,
    ratio: f32,
}

impl Analyzer {
    /// An analyzer for mono samples at `rate` per second.
    pub fn new(rate: u32, shaping: Shaping) -> Self {
        let r = rate as f32;
        let mut analyzer = Self {
            bands: [
                Band::new(Biquad::pass()),
                Band::new(Biquad::low_pass(r, BASS_TOP)),
                Band::new(Biquad::band_pass(r, BASS_TOP, TREBLE_BOTTOM)),
                Band::new(Biquad::high_pass(r, TREBLE_BOTTOM)),
            ],
            dt: 1.0 / r,
            rate: r,
            detector: coefficient(DETECTOR, r),
            slow: coefficient(SLOW, r),
            attack: 0.0,
            release: 0.0,
            ratio: 0.0,
        };
        analyzer.set_shaping(shaping);
        analyzer
    }

    /// Uses new attack, release and sensitivity from the next sample on.
    pub fn set_shaping(&mut self, shaping: Shaping) {
        self.attack = coefficient(shaping.attack, self.rate);
        self.release = coefficient(shaping.release, self.rate);
        self.ratio = jump_ratio(shaping.sensitivity);
    }

    /// Takes one sample. Returns which beats it starts, indexed by `Beat::index`
    /// (Any, Bass, Treble).
    pub fn push(&mut self, x: f32) -> [bool; 3] {
        let mut fired = [false; 4];
        for (band, fired) in self.bands.iter_mut().zip(&mut fired) {
            let y = band.filter.run(x);
            band.power += (y * y - band.power) * self.detector;
            let level = (2.0 * band.power).sqrt();
            let k = if level > band.value {
                self.attack
            } else {
                self.release
            };
            band.value += (level - band.value) * k;
            band.slow += (band.value - band.slow) * self.slow;
            band.since += self.dt;
            let threshold = band.slow * self.ratio;
            if band.value > threshold.max(FLOOR) {
                if band.armed && band.since >= HOLD_OFF {
                    *fired = true;
                    band.since = 0.0;
                }
                band.armed = false;
            } else if band.value <= threshold {
                band.armed = true;
            }
        }
        [fired[LEVEL], fired[BASS], fired[TREBLE]]
    }

    /// Level, Bass, Mid and Treble now, before gain.
    pub fn values(&self) -> [f32; 4] {
        self.bands.map(|b| b.value)
    }
}

/// `values` times `gain`, each held to 0..=1.
pub fn gained(values: [f32; 4], gain: f32) -> [f32; 4] {
    values.map(|v| (v * gain).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{Beat, RATE, Shaping};
    use std::f32::consts::TAU;

    /// A sine at `freq` Hz with amplitude `amp`, for `seconds`.
    fn sine(freq: f32, amp: f32, seconds: f32) -> Vec<f32> {
        let n = (seconds * RATE as f32) as usize;
        (0..n)
            .map(|i| amp * (TAU * freq * i as f32 / RATE as f32).sin())
            .collect()
    }

    fn silence(seconds: f32) -> Vec<f32> {
        vec![0.0; (seconds * RATE as f32) as usize]
    }

    /// Feeds `samples`; returns the values after the last one and the beats counted.
    fn run(analyzer: &mut Analyzer, samples: &[f32]) -> ([f32; 4], [usize; 3]) {
        let mut beats = [0; 3];
        for &x in samples {
            for (count, fired) in beats.iter_mut().zip(analyzer.push(x)) {
                *count += usize::from(fired);
            }
        }
        (analyzer.values(), beats)
    }

    /// `count` bursts of a `freq` Hz sine, 20 ms long, `gap` seconds apart.
    fn clicks(freq: f32, count: usize, gap: f32) -> Vec<f32> {
        let mut out = Vec::new();
        for _ in 0..count {
            out.extend(sine(freq, 1.0, 0.02));
            out.extend(silence(gap - 0.02));
        }
        out
    }

    const LEVEL: usize = 0;
    const BASS: usize = 1;
    const MID: usize = 2;
    const TREBLE: usize = 3;

    #[test]
    fn the_bands_split_the_spectrum() {
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (low, _) = run(&mut a, &sine(60.0, 1.0, 1.0));
        assert!(low[BASS] > 0.8, "{low:?}");
        assert!(low[TREBLE] < 0.05, "{low:?}");
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (high, _) = run(&mut a, &sine(8000.0, 1.0, 1.0));
        assert!(high[TREBLE] > 0.8, "{high:?}");
        assert!(high[BASS] < 0.05, "{high:?}");
    }

    #[test]
    fn a_full_scale_sine_in_its_band_reads_about_one() {
        for (freq, band) in [(60.0, BASS), (632.0, MID), (8000.0, TREBLE), (440.0, LEVEL)] {
            let mut a = Analyzer::new(RATE, Shaping::default());
            let (values, _) = run(&mut a, &sine(freq, 1.0, 1.0));
            assert!(
                (0.9..=1.1).contains(&values[band]),
                "{freq} Hz band {band}: {values:?}"
            );
        }
    }

    #[test]
    fn gain_multiplies_and_holds_at_one() {
        assert_eq!(gained([0.25, 0.5, 0.0, 1.0], 2.0), [0.5, 1.0, 0.0, 1.0]);
        assert_eq!(gained([0.5; 4], 0.0), [0.0; 4]);
    }

    #[test]
    fn attack_and_release_follow_their_times() {
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (rising, _) = run(&mut a, &sine(1000.0, 1.0, 0.05));
        assert!(rising[LEVEL] > 0.8, "fast attack: {rising:?}");
        run(&mut a, &sine(1000.0, 1.0, 0.5));
        let (released, _) = run(&mut a, &silence(0.15));
        assert!(
            (0.2..=0.55).contains(&released[LEVEL]),
            "one release time: {released:?}"
        );
        let (gone, _) = run(&mut a, &silence(0.3));
        assert!(gone[LEVEL] < 0.1, "three release times: {gone:?}");

        let slow = Shaping {
            attack: 0.2,
            ..Shaping::default()
        };
        let mut a = Analyzer::new(RATE, slow);
        let (rising, _) = run(&mut a, &sine(1000.0, 1.0, 0.05));
        assert!(rising[LEVEL] < 0.4, "slow attack: {rising:?}");
    }

    #[test]
    fn clicks_beat_once_each() {
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (_, beats) = run(&mut a, &clicks(1000.0, 4, 0.5));
        assert_eq!(beats[Beat::Any.index()], 4);
    }

    #[test]
    fn a_band_beats_only_on_its_own_sound() {
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (_, beats) = run(&mut a, &clicks(8000.0, 4, 0.5));
        assert_eq!(beats[Beat::Treble.index()], 4);
        assert_eq!(beats[Beat::Bass.index()], 0);
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (_, beats) = run(&mut a, &clicks(60.0, 4, 0.5));
        assert_eq!(beats[Beat::Bass.index()], 4);
        assert_eq!(beats[Beat::Treble.index()], 0);
    }

    #[test]
    fn clicks_closer_than_the_hold_off_beat_once() {
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (_, beats) = run(&mut a, &clicks(1000.0, 2, 0.05));
        assert_eq!(beats[Beat::Any.index()], 1);
    }

    #[test]
    fn a_steady_tone_beats_only_when_it_starts() {
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (_, beats) = run(&mut a, &sine(1000.0, 1.0, 3.0));
        assert_eq!(beats[Beat::Any.index()], 1);
    }
}
