//! Blends two parameter sets into what the renderer draws this frame
//! (the hardware's INITIAL + ramp × (FINAL − INITIAL)).

use std::f32::consts::PI;
use std::ops::RangeInclusive;

use crate::params::{
    Axis, ColorizeParams, Envelope, FeedbackParams, GlowParams, OSCILLATOR_COUNT, OscInput,
    OscSync, Oscillator, PALETTE_SIZE, Params, Waveform, ranges, srgb_to_linear,
};

/// At most two slots per oscillator (one per bank) while their settings differ.
pub const MAX_SLOTS: usize = 2 * OSCILLATOR_COUNT;

/// Running phases, in cycles. They advance by each rate every frame, so changing a
/// rate (even mid-transition) changes the speed without jumping the wave.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Clocks {
    pub osc: [f64; OSCILLATOR_COUNT],
    pub lfo: [f64; OSCILLATOR_COUNT],
    /// Palette cycle offset, in levels.
    pub cycle: f64,
}

impl Clocks {
    /// Advances by the rates in `p` over `dt` seconds. Frame-synced oscillators hold still.
    pub fn advance(&mut self, p: &Params, dt: f32) {
        let dt = f64::from(dt);
        for (i, osc) in p.warp.oscillators.iter().enumerate() {
            if osc.sync == OscSync::Free {
                self.osc[i] = (self.osc[i] + f64::from(osc.phase_speed) * dt).rem_euclid(1.0);
            }
            self.lfo[i] = (self.lfo[i] + f64::from(osc.lfo_rate) * dt).rem_euclid(1.0);
        }
        // 840 is a multiple of every level count 2..=8, so wrapping never shifts colors.
        self.cycle = (self.cycle + f64::from(p.colorize.cycle_speed) * dt).rem_euclid(840.0);
    }
}

/// One oscillator as the warp shader runs it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OscSlot {
    /// Which oscillator (0..4) this came from; seeds its drift noise.
    pub index: usize,
    pub waveform: Waveform,
    pub target: Axis,
    pub input: OscInput,
    pub frequency: f32,
    /// Final amplitude, including bank weight and envelope.
    pub amplitude: f32,
    /// Total phase in cycles, wrapped to [0, 1).
    pub phase: f32,
    /// LFO phase in cycles, wrapped to [0, 1).
    pub lfo_phase: f32,
    pub lfo_depth: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WarpFrame {
    pub zoom: f32,
    pub rotation: f32,
    pub offset: [f32; 2],
    pub drift: f32,
    pub oscillators: Vec<OscSlot>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorizeFrame {
    pub levels: u32,
    pub softness: f32,
    /// Linear-light colors.
    pub palette_linear: [[f32; 3]; PALETTE_SIZE],
    /// Palette cycle offset in levels, wrapped to [0, levels).
    pub cycle: f32,
    pub bypass: bool,
}

/// Everything the renderer needs for one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameParams {
    pub warp: WarpFrame,
    pub colorize: ColorizeFrame,
    pub feedback: FeedbackParams,
    pub glow: GlowParams,
}

impl FrameParams {
    /// A single parameter set at rest, with all clocks at zero.
    pub fn at_rest(p: &Params) -> Self {
        let clocks = Clocks::default();
        blend(p, p, 0.0, None, &clocks, &clocks)
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_in(a: f32, b: f32, t: f32, range: RangeInclusive<f32>) -> f32 {
    lerp(a, b, t).clamp(*range.start(), *range.end())
}

fn lerp_phase(a: f64, b: f64, t: f32) -> f64 {
    a + (b - a) * f64::from(t)
}

/// Swell multiplier: 0 at rest and at both ends of a ramp, 1 in the middle.
fn envelope_gain(envelope: Envelope, ramp_progress: Option<f32>) -> f32 {
    match envelope {
        Envelope::Constant => 1.0,
        Envelope::Swell => ramp_progress.map_or(0.0, |p| (PI * p.clamp(0.0, 1.0)).sin()),
    }
}

/// True when two oscillators can sweep into each other instead of crossfading.
fn same_shape(a: &Oscillator, b: &Oscillator) -> bool {
    a.waveform == b.waveform
        && a.target == b.target
        && a.input == b.input
        && a.sync == b.sync
        && a.envelope == b.envelope
}

fn slot(
    index: usize,
    osc: &Oscillator,
    clock: usize,
    clocks: &Clocks,
    weight: f32,
    ramp_progress: Option<f32>,
) -> OscSlot {
    OscSlot {
        index,
        waveform: osc.waveform,
        target: osc.target,
        input: osc.input,
        frequency: osc.frequency,
        amplitude: osc.amplitude * weight * envelope_gain(osc.envelope, ramp_progress),
        phase: (f64::from(osc.phase) + clocks.osc[clock]).rem_euclid(1.0) as f32,
        lfo_phase: clocks.lfo[clock] as f32,
        lfo_depth: osc.lfo_depth,
    }
}

/// Blends `from` toward `to`.
///
/// * `t` is curve-mapped progress (may overshoot outside [0, 1]).
/// * `ramp_progress` is the raw linear ramp progress while a ramp runs (drives Swell).
/// * `from_clocks` / `to_clocks` are the running phases belonging to each side.
pub fn blend(
    from: &Params,
    to: &Params,
    t: f32,
    ramp_progress: Option<f32>,
    from_clocks: &Clocks,
    to_clocks: &Clocks,
) -> FrameParams {
    let tc = t.clamp(0.0, 1.0);
    let (a, b) = (&from.warp, &to.warp);
    let a_osc = a.effective_oscillators();
    let b_osc = b.effective_oscillators();

    let mut oscillators = Vec::with_capacity(MAX_SLOTS);
    for i in 0..OSCILLATOR_COUNT {
        let ((oa, ca), (ob, cb)) = (a_osc[i], b_osc[i]);
        if same_shape(&oa, &ob) {
            let phase = lerp_phase(
                f64::from(oa.phase) + from_clocks.osc[ca],
                f64::from(ob.phase) + to_clocks.osc[cb],
                t,
            );
            let lfo = lerp_phase(from_clocks.lfo[ca], to_clocks.lfo[cb], t);
            oscillators.push(OscSlot {
                index: i,
                waveform: oa.waveform,
                target: oa.target,
                input: oa.input,
                frequency: lerp_in(oa.frequency, ob.frequency, t, ranges::FREQUENCY),
                amplitude: lerp_in(oa.amplitude, ob.amplitude, t, ranges::AMPLITUDE)
                    * envelope_gain(oa.envelope, ramp_progress),
                phase: phase.rem_euclid(1.0) as f32,
                lfo_phase: lfo.rem_euclid(1.0) as f32,
                lfo_depth: lerp_in(oa.lfo_depth, ob.lfo_depth, t, ranges::LFO_DEPTH),
            });
        } else {
            oscillators.push(slot(i, &oa, ca, from_clocks, 1.0 - tc, ramp_progress));
            oscillators.push(slot(i, &ob, cb, to_clocks, tc, ramp_progress));
        }
    }

    let colorize = blend_colorize(&from.colorize, &to.colorize, t, from_clocks, to_clocks);

    FrameParams {
        warp: WarpFrame {
            zoom: lerp_in(a.zoom, b.zoom, t, ranges::ZOOM),
            rotation: lerp_in(a.rotation, b.rotation, t, ranges::ROTATION),
            offset: [
                lerp_in(a.offset[0], b.offset[0], t, ranges::OFFSET),
                lerp_in(a.offset[1], b.offset[1], t, ranges::OFFSET),
            ],
            drift: lerp_in(a.drift, b.drift, t, ranges::DRIFT),
            oscillators,
        },
        colorize,
        feedback: blend_feedback(&from.feedback, &to.feedback, t),
        glow: blend_glow(&from.glow, &to.glow, t),
    }
}

fn blend_colorize(
    a: &ColorizeParams,
    b: &ColorizeParams,
    t: f32,
    from_clocks: &Clocks,
    to_clocks: &Clocks,
) -> ColorizeFrame {
    let discrete = if t >= 0.5 { b } else { a };
    let levels = discrete.levels;
    let palette_linear = std::array::from_fn(|i| {
        std::array::from_fn(|c| {
            lerp(
                srgb_to_linear(a.palette[i][c]),
                srgb_to_linear(b.palette[i][c]),
                t,
            )
            .clamp(0.0, 1.0)
        })
    });
    let cycle = lerp_phase(from_clocks.cycle, to_clocks.cycle, t).rem_euclid(f64::from(levels));
    ColorizeFrame {
        levels,
        softness: lerp_in(a.softness, b.softness, t, ranges::SOFTNESS),
        palette_linear,
        cycle: cycle as f32,
        bypass: discrete.bypass,
    }
}

fn blend_feedback(a: &FeedbackParams, b: &FeedbackParams, t: f32) -> FeedbackParams {
    FeedbackParams {
        amount: lerp_in(a.amount, b.amount, t, ranges::FEEDBACK_AMOUNT),
        zoom: lerp_in(a.zoom, b.zoom, t, ranges::FEEDBACK_ZOOM),
        rotation: lerp_in(a.rotation, b.rotation, t, ranges::FEEDBACK_ROTATION),
        offset: [
            lerp_in(a.offset[0], b.offset[0], t, ranges::FEEDBACK_OFFSET),
            lerp_in(a.offset[1], b.offset[1], t, ranges::FEEDBACK_OFFSET),
        ],
    }
}

fn blend_glow(a: &GlowParams, b: &GlowParams, t: f32) -> GlowParams {
    GlowParams {
        bloom_intensity: lerp_in(
            a.bloom_intensity,
            b.bloom_intensity,
            t,
            ranges::BLOOM_INTENSITY,
        ),
        bloom_threshold: lerp_in(
            a.bloom_threshold,
            b.bloom_threshold,
            t,
            ranges::BLOOM_THRESHOLD,
        ),
        scanline_strength: lerp_in(
            a.scanline_strength,
            b.scanline_strength,
            t,
            ranges::SCANLINE_STRENGTH,
        ),
        scanline_count: lerp_in(
            a.scanline_count,
            b.scanline_count,
            t,
            ranges::SCANLINE_COUNT,
        ),
        chroma: lerp_in(a.chroma, b.chroma, t, ranges::CHROMA),
        noise: lerp_in(a.noise, b.noise, t, ranges::NOISE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Params, Params) {
        let a = Params::default();
        let mut b = Params::default();
        b.warp.zoom = 3.0;
        b.colorize.levels = 3;
        b.colorize.bypass = true;
        b.colorize.palette[1] = [1.0, 1.0, 1.0];
        (a, b)
    }

    fn rest() -> Clocks {
        Clocks::default()
    }

    #[test]
    fn numeric_fields_lerp_and_clamp() {
        let (a, b) = pair();
        let mid = blend(&a, &b, 0.5, Some(0.5), &rest(), &rest());
        assert!((mid.warp.zoom - 2.0).abs() < 1e-6);
        // An overshooting curve can push past B, but never outside the range.
        let over = blend(&a, &b, 1.6, Some(0.9), &rest(), &rest());
        assert_eq!(over.warp.zoom, *ranges::ZOOM.end());
    }

    #[test]
    fn discrete_fields_switch_at_midpoint() {
        let (a, b) = pair();
        let early = blend(&a, &b, 0.49, Some(0.49), &rest(), &rest());
        let late = blend(&a, &b, 0.5, Some(0.5), &rest(), &rest());
        assert_eq!((early.colorize.levels, early.colorize.bypass), (6, false));
        assert_eq!((late.colorize.levels, late.colorize.bypass), (3, true));
    }

    #[test]
    fn palette_lerps_in_linear_light() {
        let (a, b) = pair();
        let mid = blend(&a, &b, 0.5, Some(0.5), &rest(), &rest());
        let expected = (srgb_to_linear(0.05) + 1.0) / 2.0;
        assert!((mid.colorize.palette_linear[1][0] - expected).abs() < 1e-6);
    }

    #[test]
    fn matching_oscillators_sweep_in_one_slot() {
        let a = Params::default();
        let mut b = a;
        b.warp.oscillators[0].frequency = 13.0;
        let mid = blend(&a, &b, 0.5, Some(0.5), &rest(), &rest());
        assert_eq!(mid.warp.oscillators.len(), OSCILLATOR_COUNT);
        assert!((mid.warp.oscillators[0].frequency - 8.0).abs() < 1e-6);
    }

    #[test]
    fn mismatched_oscillators_crossfade_in_two_slots() {
        let a = Params::default();
        let mut b = a;
        b.warp.oscillators[0].waveform = Waveform::Square;
        let f = blend(&a, &b, 0.25, Some(0.25), &rest(), &rest());
        assert_eq!(f.warp.oscillators.len(), OSCILLATOR_COUNT + 1);
        let pair: Vec<_> = f.warp.oscillators.iter().filter(|s| s.index == 0).collect();
        assert_eq!(pair[0].waveform, Waveform::Sine);
        assert_eq!(pair[1].waveform, Waveform::Square);
        let amp = a.warp.oscillators[0].amplitude;
        assert!((pair[0].amplitude - 0.75 * amp).abs() < 1e-6);
        assert!((pair[1].amplitude - 0.25 * amp).abs() < 1e-6);
    }

    #[test]
    fn never_more_than_max_slots() {
        let a = Params::default();
        let mut b = a;
        for osc in &mut b.warp.oscillators {
            osc.target = Axis::Y;
            osc.waveform = Waveform::Noise;
        }
        let f = blend(&a, &b, 0.5, Some(0.5), &rest(), &rest());
        assert_eq!(f.warp.oscillators.len(), MAX_SLOTS);
    }

    #[test]
    fn swell_is_silent_at_rest_and_peaks_mid_ramp() {
        let mut p = Params::default();
        p.warp.oscillators[0].envelope = Envelope::Swell;
        let amp = p.warp.oscillators[0].amplitude;
        let at_rest = blend(&p, &p, 0.0, None, &rest(), &rest());
        let start = blend(&p, &p, 0.0, Some(0.0), &rest(), &rest());
        let middle = blend(&p, &p, 0.5, Some(0.5), &rest(), &rest());
        assert_eq!(at_rest.warp.oscillators[0].amplitude, 0.0);
        assert!(start.warp.oscillators[0].amplitude.abs() < 1e-6);
        assert!((middle.warp.oscillators[0].amplitude - amp).abs() < 1e-6);
    }

    #[test]
    fn clocks_advance_by_rate_and_frame_sync_holds() {
        let mut p = Params::default();
        p.warp.oscillators[0].phase_speed = 0.5;
        p.warp.oscillators[1].phase_speed = 0.5;
        p.warp.oscillators[1].sync = OscSync::Frame;
        let mut c = Clocks::default();
        c.advance(&p, 0.5);
        assert!((c.osc[0] - 0.25).abs() < 1e-9);
        assert_eq!(c.osc[1], 0.0);
    }

    #[test]
    fn phase_is_continuous_across_a_rate_change() {
        // Same running phase on both sides: blending different rates mid-ramp moves
        // the phase smoothly instead of sweeping through rate × total time.
        let a = Params::default();
        let mut b = a;
        b.warp.oscillators[0].phase_speed = 3.0;
        let mut ca = Clocks::default();
        ca.osc[0] = 0.4;
        let cb = ca;
        let f0 = blend(&a, &b, 0.0, Some(0.0), &ca, &cb);
        let f1 = blend(&a, &b, 1.0, Some(1.0), &ca, &cb);
        assert!((f0.warp.oscillators[0].phase - 0.4).abs() < 1e-6);
        assert!((f1.warp.oscillators[0].phase - 0.4).abs() < 1e-6);
    }

    #[test]
    fn slaved_oscillator_uses_master_clock() {
        let mut p = Params::default();
        p.warp.slave_4_to_3 = true;
        let mut c = Clocks::default();
        c.osc[2] = 0.1;
        c.osc[3] = 0.6;
        let f = blend(&p, &p, 0.0, None, &c, &c);
        let osc4 = f.warp.oscillators.iter().find(|s| s.index == 3).unwrap();
        let expected = p.warp.oscillators[2].phase + 0.25 + 0.1;
        assert!((osc4.phase - expected).abs() < 1e-6);
    }

    #[test]
    fn palette_cycle_wraps_within_levels() {
        let mut p = Params::default();
        p.colorize.levels = 4;
        p.colorize.cycle_speed = -1.0;
        let mut c = Clocks::default();
        c.advance(&p, 5.5);
        let f = blend(&p, &p, 0.0, None, &c, &c);
        assert!((f.colorize.cycle - 2.5).abs() < 1e-4);
    }

    #[test]
    fn at_rest_matches_params() {
        let p = Params::default();
        let f = FrameParams::at_rest(&p);
        assert_eq!(f.warp.zoom, p.warp.zoom);
        assert_eq!(f.feedback, p.feedback);
        assert_eq!(f.glow, p.glow);
        assert_eq!(f.colorize.levels, p.colorize.levels);
        assert_eq!(f.warp.oscillators.len(), OSCILLATOR_COUNT);
    }
}
