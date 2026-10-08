//! Every live parameter, edited by the UI and read by the render passes each frame.

use std::f32::consts::PI;
use std::ops::RangeInclusive;

/// Slider ranges. The UI uses these, and tests check that defaults fall inside them.
pub mod ranges {
    use super::*;

    pub const FREQUENCY: RangeInclusive<f32> = 0.0..=40.0;
    pub const AMPLITUDE: RangeInclusive<f32> = 0.0..=0.5;
    pub const PHASE: RangeInclusive<f32> = 0.0..=1.0;
    pub const PHASE_SPEED: RangeInclusive<f32> = -4.0..=4.0;
    pub const LFO_RATE: RangeInclusive<f32> = 0.0..=10.0;
    pub const LFO_DEPTH: RangeInclusive<f32> = 0.0..=1.0;
    pub const ZOOM: RangeInclusive<f32> = 0.1..=4.0;
    pub const ROTATION: RangeInclusive<f32> = -PI..=PI;
    pub const OFFSET: RangeInclusive<f32> = -1.0..=1.0;
    pub const DRIFT: RangeInclusive<f32> = 0.0..=1.0;
    pub const LEVELS: RangeInclusive<u32> = 2..=8;
    pub const SOFTNESS: RangeInclusive<f32> = 0.0..=1.0;
    pub const CYCLE_SPEED: RangeInclusive<f32> = -4.0..=4.0;
    pub const FEEDBACK_AMOUNT: RangeInclusive<f32> = 0.0..=0.99;
    pub const FEEDBACK_ZOOM: RangeInclusive<f32> = 0.9..=1.1;
    pub const FEEDBACK_ROTATION: RangeInclusive<f32> = -0.1..=0.1;
    pub const FEEDBACK_OFFSET: RangeInclusive<f32> = -0.02..=0.02;
    pub const BLOOM_INTENSITY: RangeInclusive<f32> = 0.0..=4.0;
    pub const BLOOM_THRESHOLD: RangeInclusive<f32> = 0.0..=1.0;
    pub const SCANLINE_STRENGTH: RangeInclusive<f32> = 0.0..=1.0;
    pub const SCANLINE_COUNT: RangeInclusive<f32> = 100.0..=1080.0;
    pub const CHROMA: RangeInclusive<f32> = 0.0..=4.0;
    pub const NOISE: RangeInclusive<f32> = 0.0..=0.5;
    pub const THRESHOLD: RangeInclusive<f32> = 0.0..=1.0;
    pub const BANDWIDTH: RangeInclusive<f32> = 0.0..=8.0;
    pub const RINGING: RangeInclusive<f32> = 0.0..=1.0;
    pub const LINE_JITTER: RangeInclusive<f32> = 0.0..=10.0;
    pub const AXIS_WANDER: RangeInclusive<f32> = 0.0..=1.0;
    pub const RASTER_LINES: RangeInclusive<u32> = 100..=1200;
    pub const BEAM_WIDTH: RangeInclusive<f32> = 0.5..=4.0;
    pub const COMPENSATION: RangeInclusive<f32> = 0.0..=1.0;
    pub const SPEED_COMPENSATION: RangeInclusive<f32> = 0.0..=1.0;
}

pub const OSCILLATOR_COUNT: usize = 4;
pub const PALETTE_SIZE: usize = 8;
/// One threshold between each pair of neighbouring levels.
pub const THRESHOLD_COUNT: usize = PALETTE_SIZE - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waveform {
    Sine,
    Triangle,
    Ramp,
    Square,
    Noise,
}

impl Waveform {
    pub const ALL: [Waveform; 5] = [
        Waveform::Sine,
        Waveform::Triangle,
        Waveform::Ramp,
        Waveform::Square,
        Waveform::Noise,
    ];
}

/// Which displacement component an oscillator adds to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

/// What drives an oscillator's phase across the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OscInput {
    /// Horizontal position.
    U,
    /// Vertical position: X displacement driven by V gives the classic per-scanline wiggle.
    V,
    /// Distance from the frame center.
    Radius,
    /// Time only: the whole frame moves together.
    Time,
}

impl OscInput {
    pub const ALL: [OscInput; 4] = [OscInput::U, OscInput::V, OscInput::Radius, OscInput::Time];
}

/// How an oscillator's phase moves over time (the manual's sync modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OscSync {
    /// Phase advances at `phase_speed`: the wave drifts.
    Free,
    /// Phase is locked to the frame: the wave stands still.
    Frame,
}

impl OscSync {
    pub const ALL: [OscSync; 2] = [OscSync::Free, OscSync::Frame];
}

/// How an oscillator's amplitude behaves while a transition or sequence ramp runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Envelope {
    /// Always at its set amplitude.
    Constant,
    /// Silent at rest; swells to its amplitude in the middle of a ramp.
    Swell,
}

impl Envelope {
    pub const ALL: [Envelope; 2] = [Envelope::Constant, Envelope::Swell];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oscillator {
    pub waveform: Waveform,
    pub target: Axis,
    pub input: OscInput,
    /// Cycles per frame height.
    pub frequency: f32,
    /// Displacement in frame heights. 0 turns the oscillator off.
    pub amplitude: f32,
    /// Phase offset in cycles.
    pub phase: f32,
    /// Phase drift in cycles per second.
    pub phase_speed: f32,
    /// Amplitude LFO rate in Hz.
    pub lfo_rate: f32,
    /// 0 = constant amplitude, 1 = amplitude swings fully between 0 and `amplitude`.
    pub lfo_depth: f32,
    pub sync: OscSync,
    pub envelope: Envelope,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WarpParams {
    pub oscillators: [Oscillator; OSCILLATOR_COUNT],
    pub zoom: f32,
    /// Radians.
    pub rotation: f32,
    /// Frame heights.
    pub offset: [f32; 2],
    /// Slow random wobble of oscillator frequency and phase.
    pub drift: f32,
    /// Oscillator 4 follows oscillator 3 a quarter cycle behind (sine/cosine pair),
    /// keeping its own target and amplitude.
    pub slave_4_to_3: bool,
    /// Time-base error: each line shifts sideways by up to this many canvas pixels,
    /// new every animation frame.
    pub line_jitter: f32,
    /// How far the rotation pivot wanders while rotated, like a real deflection yoke.
    pub axis_wander: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorizeParams {
    pub levels: u32,
    pub softness: f32,
    /// sRGB colors, one per brightness level (darkest first).
    pub palette: [[f32; 3]; PALETTE_SIZE],
    /// Levels per second the palette rotates through.
    pub cycle_speed: f32,
    pub bypass: bool,
    /// Brightness where each level starts, ascending; only the first `levels - 1` are
    /// used.
    pub thresholds: [f32; THRESHOLD_COUNT],
    /// How far edges smear to the right of themselves, in canvas pixels.
    pub bandwidth: f32,
    /// How much edges overshoot and ripple after themselves.
    pub ringing: f32,
}

/// True raster mode: draw the source as deflected scan lines instead of warping it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterParams {
    pub enabled: bool,
    pub lines: u32,
    /// Beam width in canvas pixels.
    pub beam_width: f32,
    /// How much packed or spread lines are evened out (the manual's area compensator).
    pub compensation: f32,
    /// How much fast-moving lines are brightened so fast sweeps don't fade.
    pub speed_compensation: f32,
}

/// Level keying: chosen colorizer levels become see-through, showing a background image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyParams {
    pub enabled: bool,
    /// Bit i set: level i is see-through.
    pub levels: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeedbackParams {
    /// How much of the previous canvas frame survives each canvas frame (so trails
    /// decay at the program frame rate). 0 = no trails.
    pub amount: f32,
    /// Per-canvas-frame transform applied to the previous canvas frame.
    pub zoom: f32,
    pub rotation: f32,
    pub offset: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlowParams {
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    pub scanline_strength: f32,
    pub scanline_count: f32,
    /// Chromatic bleed, in internal-resolution pixels.
    pub chroma: f32,
    pub noise: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub warp: WarpParams,
    pub colorize: ColorizeParams,
    pub feedback: FeedbackParams,
    pub glow: GlowParams,
    pub raster: RasterParams,
    pub key: KeyParams,
}

/// Thresholds spaced evenly for `levels` levels (`k / levels`); unused entries are 1.
pub fn even_thresholds(levels: u32) -> [f32; THRESHOLD_COUNT] {
    std::array::from_fn(|i| {
        let k = i as u32 + 1;
        if k < levels {
            k as f32 / levels as f32
        } else {
            1.0
        }
    })
}

impl Default for Oscillator {
    fn default() -> Self {
        Self {
            waveform: Waveform::Sine,
            target: Axis::X,
            input: OscInput::V,
            frequency: 1.0,
            amplitude: 0.0,
            phase: 0.0,
            phase_speed: 0.0,
            lfo_rate: 0.0,
            lfo_depth: 0.0,
            sync: OscSync::Free,
            envelope: Envelope::Constant,
        }
    }
}

impl Default for Params {
    fn default() -> Self {
        Self {
            warp: WarpParams {
                oscillators: [
                    Oscillator {
                        frequency: 3.0,
                        amplitude: 0.05,
                        phase_speed: 0.5,
                        lfo_rate: 0.2,
                        lfo_depth: 0.5,
                        ..Oscillator::default()
                    },
                    Oscillator {
                        waveform: Waveform::Triangle,
                        target: Axis::Y,
                        input: OscInput::U,
                        frequency: 2.0,
                        amplitude: 0.03,
                        phase_speed: -0.3,
                        ..Oscillator::default()
                    },
                    Oscillator {
                        input: OscInput::Radius,
                        frequency: 6.0,
                        phase_speed: 1.0,
                        ..Oscillator::default()
                    },
                    Oscillator {
                        waveform: Waveform::Noise,
                        frequency: 20.0,
                        ..Oscillator::default()
                    },
                ],
                zoom: 1.0,
                rotation: 0.0,
                offset: [0.0, 0.0],
                drift: 0.2,
                slave_4_to_3: false,
                line_jitter: 0.0,
                axis_wander: 0.15,
            },
            colorize: ColorizeParams {
                levels: 6,
                softness: 0.1,
                palette: [
                    [0.0, 0.0, 0.0],
                    [0.05, 0.1, 0.6],
                    [0.7, 0.1, 0.8],
                    [1.0, 0.45, 0.1],
                    [1.0, 0.9, 0.2],
                    [1.0, 1.0, 1.0],
                    [0.2, 0.9, 1.0],
                    [0.2, 1.0, 0.4],
                ],
                cycle_speed: 0.0,
                bypass: false,
                thresholds: even_thresholds(6),
                bandwidth: 1.0,
                ringing: 0.2,
            },
            feedback: FeedbackParams {
                amount: 0.85,
                zoom: 1.01,
                rotation: 0.002,
                offset: [0.0, 0.0],
            },
            glow: GlowParams {
                bloom_intensity: 1.0,
                bloom_threshold: 0.6,
                scanline_strength: 0.3,
                scanline_count: 540.0,
                chroma: 1.0,
                noise: 0.04,
            },
            raster: RasterParams {
                enabled: false,
                lines: 600,
                beam_width: 1.2,
                compensation: 1.0,
                speed_compensation: 0.3,
            },
            key: KeyParams {
                enabled: false,
                levels: 1,
            },
        }
    }
}

impl WarpParams {
    /// The oscillators as they actually run, with oscillator 4 resolved when it is
    /// slaved to oscillator 3. Each entry also names the phase clock it uses.
    pub fn effective_oscillators(&self) -> [(Oscillator, usize); OSCILLATOR_COUNT] {
        let mut out = std::array::from_fn(|i| (self.oscillators[i], i));
        if self.slave_4_to_3 {
            let master = self.oscillators[2];
            let own = self.oscillators[3];
            out[3] = (
                Oscillator {
                    target: own.target,
                    amplitude: own.amplitude,
                    envelope: own.envelope,
                    phase: master.phase + 0.25,
                    ..master
                },
                2,
            );
        }
        out
    }
}

impl KeyParams {
    /// Clears the see-through bits of levels `levels` and up, which a colorizer with
    /// `levels` levels doesn't have, so they don't come back when the count grows.
    pub fn keep_levels(&mut self, levels: u32) {
        self.levels &= ((1u16 << levels.min(8)) - 1) as u8;
    }
}

/// sRGB-encoded channel to linear, for palette colors going to the GPU.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_within_ui_ranges() {
        let p = Params::default();
        for o in &p.warp.oscillators {
            assert!(ranges::FREQUENCY.contains(&o.frequency));
            assert!(ranges::AMPLITUDE.contains(&o.amplitude));
            assert!(ranges::PHASE.contains(&o.phase));
            assert!(ranges::PHASE_SPEED.contains(&o.phase_speed));
            assert!(ranges::LFO_RATE.contains(&o.lfo_rate));
            assert!(ranges::LFO_DEPTH.contains(&o.lfo_depth));
        }
        assert!(ranges::ZOOM.contains(&p.warp.zoom));
        assert!(ranges::ROTATION.contains(&p.warp.rotation));
        assert!(p.warp.offset.iter().all(|v| ranges::OFFSET.contains(v)));
        assert!(ranges::DRIFT.contains(&p.warp.drift));
        assert!(ranges::LEVELS.contains(&p.colorize.levels));
        assert!(ranges::SOFTNESS.contains(&p.colorize.softness));
        assert!(ranges::CYCLE_SPEED.contains(&p.colorize.cycle_speed));
        assert!(
            p.colorize
                .palette
                .iter()
                .flatten()
                .all(|c| (0.0..=1.0).contains(c))
        );
        assert!(ranges::FEEDBACK_AMOUNT.contains(&p.feedback.amount));
        assert!(ranges::FEEDBACK_ZOOM.contains(&p.feedback.zoom));
        assert!(ranges::FEEDBACK_ROTATION.contains(&p.feedback.rotation));
        assert!(
            p.feedback
                .offset
                .iter()
                .all(|v| ranges::FEEDBACK_OFFSET.contains(v))
        );
        assert!(ranges::BLOOM_INTENSITY.contains(&p.glow.bloom_intensity));
        assert!(ranges::BLOOM_THRESHOLD.contains(&p.glow.bloom_threshold));
        assert!(ranges::SCANLINE_STRENGTH.contains(&p.glow.scanline_strength));
        assert!(ranges::SCANLINE_COUNT.contains(&p.glow.scanline_count));
        assert!(ranges::CHROMA.contains(&p.glow.chroma));
        assert!(ranges::NOISE.contains(&p.glow.noise));
        assert!(
            p.colorize
                .thresholds
                .iter()
                .all(|t| ranges::THRESHOLD.contains(t))
        );
        assert!(ranges::BANDWIDTH.contains(&p.colorize.bandwidth));
        assert!(ranges::RINGING.contains(&p.colorize.ringing));
        assert!(ranges::LINE_JITTER.contains(&p.warp.line_jitter));
        assert!(ranges::AXIS_WANDER.contains(&p.warp.axis_wander));
        assert!(ranges::RASTER_LINES.contains(&p.raster.lines));
        assert!(ranges::BEAM_WIDTH.contains(&p.raster.beam_width));
        assert!(ranges::COMPENSATION.contains(&p.raster.compensation));
        assert!(ranges::SPEED_COMPENSATION.contains(&p.raster.speed_compensation));
    }

    #[test]
    fn keeping_levels_clears_the_hidden_see_through_bits() {
        let mut key = KeyParams {
            enabled: true,
            levels: 0b1010_0101,
        };
        key.keep_levels(8);
        assert_eq!(key.levels, 0b1010_0101, "all eight levels exist");
        key.keep_levels(6);
        assert_eq!(key.levels, 0b0010_0101);
        key.keep_levels(2);
        assert_eq!(key.levels, 0b01);
    }

    #[test]
    fn even_thresholds_split_brightness_into_equal_bands() {
        let six = even_thresholds(6);
        for (k, t) in six.iter().take(5).enumerate() {
            assert!((t - (k + 1) as f32 / 6.0).abs() < 1e-6);
        }
        assert_eq!(&six[5..], &[1.0, 1.0], "unused thresholds");
        for levels in ranges::LEVELS {
            let t = even_thresholds(levels);
            assert!(t.windows(2).all(|w| w[0] <= w[1]), "{levels} levels: {t:?}");
        }
        assert_eq!(Params::default().colorize.thresholds, six);
    }

    #[test]
    fn default_has_a_moving_oscillator() {
        let p = Params::default();
        assert!(
            p.warp
                .oscillators
                .iter()
                .any(|o| o.amplitude > 0.0 && o.phase_speed != 0.0)
        );
    }

    #[test]
    fn slaved_oscillator_follows_master_a_quarter_cycle_behind() {
        let mut w = Params::default().warp;
        w.oscillators[2] = Oscillator {
            waveform: Waveform::Triangle,
            frequency: 5.0,
            phase: 0.1,
            phase_speed: 0.7,
            ..Oscillator::default()
        };
        w.oscillators[3] = Oscillator {
            target: Axis::Y,
            amplitude: 0.2,
            ..Oscillator::default()
        };
        w.slave_4_to_3 = true;
        let (osc4, clock) = w.effective_oscillators()[3];
        assert_eq!(clock, 2);
        assert_eq!(osc4.waveform, Waveform::Triangle);
        assert_eq!(osc4.frequency, 5.0);
        assert_eq!(osc4.phase_speed, 0.7);
        assert!((osc4.phase - 0.35).abs() < 1e-6);
        assert_eq!(osc4.target, Axis::Y);
        assert_eq!(osc4.amplitude, 0.2);
    }

    #[test]
    fn unslaved_oscillators_use_their_own_clocks() {
        let w = Params::default().warp;
        let eff = w.effective_oscillators();
        for (i, (osc, clock)) in eff.iter().enumerate() {
            assert_eq!(*clock, i);
            assert_eq!(*osc, w.oscillators[i]);
        }
    }

    #[test]
    fn srgb_to_linear_known_values() {
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-6);
        assert!((srgb_to_linear(0.5) - 0.214).abs() < 1e-3);
    }
}
