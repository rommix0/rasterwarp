//! Blends two parameter sets into what the renderer draws this frame
//! (the hardware's INITIAL + ramp × (FINAL − INITIAL)).

use std::f32::consts::PI;
use std::ops::RangeInclusive;

use crate::params::{
    Axis, Between, ColorizeParams, Envelope, FeedbackParams, GlowParams, KeyParams,
    OSCILLATOR_COUNT, OscInput, OscSync, Oscillator, PALETTE_SIZE, Params, PlayMode, RasterParams,
    Role, Slit, THRESHOLD_COUNT, VideoParams, Waveform, ranges, srgb_to_linear,
};

/// At most two slots per oscillator (one per bank) while their settings differ.
pub const MAX_SLOTS: usize = 2 * OSCILLATOR_COUNT;

/// Running phases, in cycles. Stored as unwrapped f64 running totals so the two sides
/// of a blend stay continuous even when one passes a wrap boundary. Values are wrapped
/// only when producing frame values, so f64 precision (which holds for days of playback)
/// is preserved. Frame-synced oscillators hold still (don't advance osc clock).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Clocks {
    pub osc: [f64; OSCILLATOR_COUNT],
    pub lfo: [f64; OSCILLATOR_COUNT],
    /// Palette cycle offset, in levels (unwrapped).
    pub cycle: f64,
    /// Clip time of each input's video, in seconds (unwrapped), indexed by
    /// [`Role::index`]. It runs in every play mode; Scrub just doesn't read it.
    pub video: [f64; 2],
}

impl Clocks {
    /// Advances by the rates in `p` over `dt` seconds. Stores unwrapped running totals
    /// so blends stay continuous across wrap boundaries. Frame-synced oscillators hold
    /// still.
    pub fn advance(&mut self, p: &Params, dt: f32) {
        let dt = f64::from(dt);
        for (i, osc) in p.warp.oscillators.iter().enumerate() {
            self.advance_osc(i, osc.sync, osc.phase_speed, osc.lfo_rate, dt);
        }
        self.cycle += f64::from(p.colorize.cycle_speed) * dt;
        for role in Role::ALL {
            self.video[role.index()] += f64::from(p.video(role).speed) * dt;
        }
    }

    fn advance_osc(&mut self, i: usize, sync: OscSync, phase_speed: f32, lfo_rate: f32, dt: f64) {
        if sync == OscSync::Free {
            self.osc[i] += f64::from(phase_speed) * dt;
        }
        self.lfo[i] += f64::from(lfo_rate) * dt;
    }
}

/// Advances a running ramp's clocks by `dt` seconds, where `t` is the curve-mapped
/// progress (as passed to [`blend`]).
///
/// A clock read by a sweeping oscillator (one slot, lerped between the two sides)
/// advances on both sides at the lerped rate. Both sides start a ramp equal, so they
/// stay equal, and the oscillator's speed glides from `from`'s to `to`'s along the
/// curve. (Lerping two clocks that each ran at their own side's rate would spin the wave
/// faster than either side, more so the longer the ramp.) Clocks that only crossfading
/// oscillators read run at their own side's rates. The palette cycle and the video clocks
/// always glide.
pub fn advance_ramp(
    from_clocks: &mut Clocks,
    to_clocks: &mut Clocks,
    from: &Params,
    to: &Params,
    t: f32,
    dt: f32,
) {
    let mut glides = [false; OSCILLATOR_COUNT];
    let a_osc = from.warp.effective_oscillators();
    let b_osc = to.warp.effective_oscillators();
    for (a, b) in a_osc.iter().zip(&b_osc) {
        if sweeps(a, b) {
            glides[a.1] = true;
        }
    }
    let dt64 = f64::from(dt);
    for (i, glide) in glides.into_iter().enumerate() {
        let (a, b) = (&from.warp.oscillators[i], &to.warp.oscillators[i]);
        if glide {
            // A sweeping oscillator reads this clock with the same sync on both sides.
            let phase_speed = lerp_in(a.phase_speed, b.phase_speed, t, ranges::PHASE_SPEED);
            let lfo_rate = lerp_in(a.lfo_rate, b.lfo_rate, t, ranges::LFO_RATE);
            for clocks in [&mut *from_clocks, &mut *to_clocks] {
                clocks.advance_osc(i, a.sync, phase_speed, lfo_rate, dt64);
            }
        } else {
            from_clocks.advance_osc(i, a.sync, a.phase_speed, a.lfo_rate, dt64);
            to_clocks.advance_osc(i, b.sync, b.phase_speed, b.lfo_rate, dt64);
        }
    }
    let (a, b) = (&from.colorize, &to.colorize);
    let cycle = f64::from(lerp_in(
        a.cycle_speed,
        b.cycle_speed,
        t,
        ranges::CYCLE_SPEED,
    )) * dt64;
    from_clocks.cycle += cycle;
    to_clocks.cycle += cycle;
    for role in Role::ALL {
        let (a, b) = (from.video(role), to.video(role));
        let step = f64::from(lerp_in(a.speed, b.speed, t, ranges::PLAY_SPEED)) * dt64;
        from_clocks.video[role.index()] += step;
        to_clocks.video[role.index()] += step;
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
    /// Canvas pixels.
    pub line_jitter: f32,
    pub axis_wander: f32,
    /// Seeds the line jitter: the number of animation frames so far (see
    /// `Motion::frame`), so the jitter is new every frame and frozen while paused.
    pub seed: u32,
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
    /// Not necessarily ascending: an overshooting curve can push them past each other.
    /// The shader counts the thresholds at or below a brightness, so order doesn't matter.
    pub thresholds: [f32; THRESHOLD_COUNT],
    pub bandwidth: f32,
    pub ringing: f32,
}

/// One input's playback settings as this frame uses them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoFrame {
    pub mode: PlayMode,
    /// The video clock: clip time in seconds, unwrapped.
    pub clock: f64,
    /// Which way the clip is playing (its sign), for slit-scan.
    pub speed: f32,
    pub position: f32,
    pub between: Between,
    pub delay: f32,
    pub slit: Slit,
    pub slit_depth: f32,
    pub slit_flip: bool,
}

/// Everything the renderer needs for one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameParams {
    pub warp: WarpFrame,
    pub colorize: ColorizeFrame,
    pub feedback: FeedbackParams,
    pub glow: GlowParams,
    pub raster: RasterParams,
    pub key: KeyParams,
    /// Indexed by [`Role::index`].
    pub video: [VideoFrame; 2],
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

/// True when two effective oscillators (with the clocks they read) sweep in one slot.
/// Only when both read the same clock: clocks are unwrapped running totals, so lerping
/// between two different clocks would spin through every cycle separating them.
fn sweeps(a: &(Oscillator, usize), b: &(Oscillator, usize)) -> bool {
    same_shape(&a.0, &b.0) && a.1 == b.1
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
        lfo_phase: clocks.lfo[clock].rem_euclid(1.0) as f32,
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
        if sweeps(&a_osc[i], &b_osc[i]) {
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
            line_jitter: lerp_in(a.line_jitter, b.line_jitter, t, ranges::LINE_JITTER),
            axis_wander: lerp_in(a.axis_wander, b.axis_wander, t, ranges::AXIS_WANDER),
            seed: 0,
        },
        colorize,
        feedback: blend_feedback(&from.feedback, &to.feedback, t),
        glow: blend_glow(&from.glow, &to.glow, t),
        raster: blend_raster(&from.raster, &to.raster, t),
        key: if t >= 0.5 { to.key } else { from.key },
        video: Role::ALL.map(|role| {
            let i = role.index();
            let clock = lerp_phase(from_clocks.video[i], to_clocks.video[i], t);
            blend_video(from.video(role), to.video(role), t, clock)
        }),
    }
}

fn blend_video(a: &VideoParams, b: &VideoParams, t: f32, clock: f64) -> VideoFrame {
    let discrete = if t >= 0.5 { b } else { a };
    VideoFrame {
        mode: discrete.mode,
        clock,
        speed: lerp_in(a.speed, b.speed, t, ranges::PLAY_SPEED),
        position: lerp_in(a.position, b.position, t, ranges::POSITION),
        between: discrete.between,
        delay: lerp_in(a.delay, b.delay, t, ranges::DELAY),
        slit: discrete.slit,
        slit_depth: lerp_in(a.slit_depth, b.slit_depth, t, ranges::SLIT_DEPTH),
        slit_flip: discrete.slit_flip,
    }
}

fn blend_raster(a: &RasterParams, b: &RasterParams, t: f32) -> RasterParams {
    let discrete = if t >= 0.5 { b } else { a };
    RasterParams {
        enabled: discrete.enabled,
        lines: discrete.lines,
        beam_width: lerp_in(a.beam_width, b.beam_width, t, ranges::BEAM_WIDTH),
        compensation: lerp_in(a.compensation, b.compensation, t, ranges::COMPENSATION),
        speed_compensation: lerp_in(
            a.speed_compensation,
            b.speed_compensation,
            t,
            ranges::SPEED_COMPENSATION,
        ),
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
        thresholds: std::array::from_fn(|i| {
            lerp_in(a.thresholds[i], b.thresholds[i], t, ranges::THRESHOLD)
        }),
        bandwidth: lerp_in(a.bandwidth, b.bandwidth, t, ranges::BANDWIDTH),
        ringing: lerp_in(a.ringing, b.ringing, t, ranges::RINGING),
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
    fn look_fields_lerp_and_switch_at_midpoint() {
        let a = Params::default();
        let mut b = a;
        b.colorize.thresholds[0] = 0.5;
        b.colorize.bandwidth = 3.0;
        b.colorize.ringing = 0.6;
        b.warp.line_jitter = 4.0;
        b.warp.axis_wander = 0.55;
        b.raster.enabled = true;
        b.raster.lines = 200;
        b.raster.beam_width = 2.2;
        b.key.enabled = true;
        b.key.levels = 0b110;
        let early = blend(&a, &b, 0.25, Some(0.25), &rest(), &rest());
        let expected = a.colorize.thresholds[0] + 0.25 * (0.5 - a.colorize.thresholds[0]);
        assert!((early.colorize.thresholds[0] - expected).abs() < 1e-6);
        assert!((early.colorize.bandwidth - 1.5).abs() < 1e-6);
        assert!((early.colorize.ringing - 0.3).abs() < 1e-6);
        assert!((early.warp.line_jitter - 1.0).abs() < 1e-6);
        assert!((early.warp.axis_wander - 0.25).abs() < 1e-6);
        assert!((early.raster.beam_width - 1.45).abs() < 1e-6);
        assert_eq!((early.raster.enabled, early.raster.lines), (false, 600));
        assert_eq!(early.key, a.key);
        let late = blend(&a, &b, 0.5, Some(0.5), &rest(), &rest());
        assert_eq!((late.raster.enabled, late.raster.lines), (true, 200));
        assert_eq!(late.key, b.key);
        // Overshooting curves stay inside each range.
        let over = blend(&a, &b, 3.0, Some(0.9), &rest(), &rest());
        assert_eq!(over.warp.line_jitter, *ranges::LINE_JITTER.end());
        assert_eq!(over.raster.beam_width, *ranges::BEAM_WIDTH.end());
    }

    #[test]
    fn video_settings_lerp_and_switch_at_midpoint() {
        let a = Params::default();
        let mut b = a;
        let v = b.video_mut(Role::Background);
        v.mode = PlayMode::Scrub;
        v.speed = -3.0;
        v.position = 0.8;
        v.between = Between::Nearest;
        v.delay = 4.0;
        v.slit = Slit::Rows;
        v.slit_depth = 5.0;
        v.slit_flip = true;
        let early = blend(&a, &b, 0.25, Some(0.25), &rest(), &rest()).video[1];
        assert!((early.speed - 0.0).abs() < 1e-6);
        assert!((early.position - 0.2).abs() < 1e-6);
        assert!((early.delay - 1.0).abs() < 1e-6);
        assert!((early.slit_depth - 2.0).abs() < 1e-6);
        assert_eq!(early.mode, PlayMode::Loop);
        assert_eq!(
            (early.between, early.slit, early.slit_flip),
            (Between::Blend, Slit::Off, false)
        );
        let late = blend(&a, &b, 0.5, Some(0.5), &rest(), &rest()).video[1];
        assert_eq!(late.mode, PlayMode::Scrub);
        assert_eq!(
            (late.between, late.slit, late.slit_flip),
            (Between::Nearest, Slit::Rows, true)
        );
        let source = blend(&a, &b, 0.5, Some(0.5), &rest(), &rest()).video[0];
        assert_eq!(source.mode, PlayMode::Loop, "the source is unchanged");
    }

    #[test]
    fn video_clocks_run_at_their_speeds() {
        let mut p = Params::default();
        p.video_mut(Role::Source).speed = -2.0;
        let mut c = Clocks::default();
        c.advance(&p, 0.5);
        assert_eq!(c.video, [-1.0, 0.5]);
        let f = blend(&p, &p, 0.0, None, &c, &c);
        assert_eq!((f.video[0].clock, f.video[1].clock), (-1.0, 0.5));
    }

    #[test]
    fn ramps_glide_video_speed_on_both_sides() {
        let a = Params::default();
        let mut b = a;
        b.video_mut(Role::Source).speed = -3.0;
        let (mut from, mut to) = (rest(), rest());
        advance_ramp(&mut from, &mut to, &a, &b, 0.5, 2.0);
        // Halfway from 1× to −3×: −1× for 2 s.
        assert!((from.video[0] + 2.0).abs() < 1e-9);
        assert_eq!(from, to, "both sides stay on one clock");
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
    fn slave_flag_mismatch_crossfades_instead_of_sweeping_clocks() {
        let mut a = Params::default();
        a.warp.slave_4_to_3 = true;
        let mut b = a;
        b.warp.slave_4_to_3 = false;
        // Make osc 4's own shape equal to the slaved (osc 3) shape on side B.
        let master = b.warp.oscillators[2];
        let own = &mut b.warp.oscillators[3];
        own.waveform = master.waveform;
        own.input = master.input;
        own.sync = master.sync;
        own.envelope = master.envelope;
        own.target = a.warp.oscillators[3].target;
        a.warp.oscillators[3].envelope = master.envelope;
        // Osc 4 defaults to zero amplitude; make it audible on both sides.
        a.warp.oscillators[3].amplitude = 0.1;
        b.warp.oscillators[3].amplitude = 0.1;
        // Unrelated running totals for clocks 2 and 3.
        let mut c = Clocks::default();
        c.osc[2] = 600.0;
        c.osc[3] = 0.0;
        let ea = a.warp.effective_oscillators()[3];
        let eb = b.warp.effective_oscillators()[3];
        assert!(same_shape(&ea.0, &eb.0));
        assert_ne!(ea.1, eb.1);

        let mid = blend(&a, &b, 0.5, Some(0.5), &c, &c);
        assert_eq!(mid.warp.oscillators.len(), OSCILLATOR_COUNT + 1);

        let phase_of = |osc: &Oscillator, clock: usize| {
            (f64::from(osc.phase) + c.osc[clock]).rem_euclid(1.0) as f32
        };
        for (t, osc, clock) in [(0.0, &ea.0, ea.1), (1.0, &eb.0, eb.1)] {
            let f = blend(&a, &b, t, Some(t), &c, &c);
            let visible: Vec<_> = f
                .warp
                .oscillators
                .iter()
                .filter(|s| s.index == 3 && s.amplitude > 0.0)
                .collect();
            assert_eq!(visible.len(), 1);
            assert!((visible[0].phase - phase_of(osc, clock)).abs() < 1e-5);
        }
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
    fn ramp_glides_sweeping_speeds_on_both_sides() {
        let mut a = Params::default();
        a.warp.oscillators[0].phase_speed = 0.5;
        a.warp.oscillators[0].lfo_rate = 1.0;
        a.colorize.cycle_speed = 1.0;
        let mut b = a;
        b.warp.oscillators[0].phase_speed = 2.0;
        b.warp.oscillators[0].lfo_rate = 3.0;
        b.colorize.cycle_speed = 3.0;
        let (mut from, mut to) = (rest(), rest());
        advance_ramp(&mut from, &mut to, &a, &b, 0.25, 2.0);
        // A quarter of the way: 0.875 cycles/s, LFO 1.5 Hz, cycle 1.5 levels/s, for 2 s.
        assert!((from.osc[0] - 1.75).abs() < 1e-6);
        assert!((from.lfo[0] - 3.0).abs() < 1e-6);
        assert!((from.cycle - 3.0).abs() < 1e-6);
        assert_eq!(from, to, "both sides stay on one clock");
    }

    #[test]
    fn ramp_keeps_crossfading_oscillators_at_their_own_speeds() {
        let mut a = Params::default();
        a.warp.oscillators[0].phase_speed = 0.5;
        let mut b = a;
        b.warp.oscillators[0].waveform = Waveform::Square;
        b.warp.oscillators[0].phase_speed = 2.0;
        let (mut from, mut to) = (rest(), rest());
        advance_ramp(&mut from, &mut to, &a, &b, 0.5, 1.0);
        assert!((from.osc[0] - 0.5).abs() < 1e-6);
        assert!((to.osc[0] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn overshooting_curves_push_speeds_past_the_destination_within_range() {
        let mut a = Params::default();
        a.warp.oscillators[0].phase_speed = 0.0;
        let mut b = a;
        b.warp.oscillators[0].phase_speed = 3.0;
        let (mut from, mut to) = (rest(), rest());
        advance_ramp(&mut from, &mut to, &a, &b, 1.2, 1.0);
        assert!((from.osc[0] - 3.6).abs() < 1e-5, "past B's 3.0");
        let (mut from, mut to) = (rest(), rest());
        advance_ramp(&mut from, &mut to, &a, &b, 1.6, 1.0);
        let top = f64::from(*ranges::PHASE_SPEED.end());
        assert!((from.osc[0] - top).abs() < 1e-6, "clamped to the range");
    }

    #[test]
    fn ramp_holds_frame_synced_oscillators() {
        let mut a = Params::default();
        a.warp.oscillators[0].sync = OscSync::Frame;
        a.warp.oscillators[0].phase_speed = 1.0;
        let b = a;
        let (mut from, mut to) = (rest(), rest());
        advance_ramp(&mut from, &mut to, &a, &b, 0.5, 1.0);
        assert_eq!((from.osc[0], to.osc[0]), (0.0, 0.0));
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

    #[test]
    fn phase_is_continuous_when_one_side_passes_a_wrap() {
        let mut a = Params::default();
        a.warp.oscillators[0].phase_speed = 0.0;
        let mut b = a;
        b.warp.oscillators[0].phase_speed = 1.0;
        let mut from = Clocks::default();
        from.osc[0] = 0.98;
        let mut to = from;
        from.advance(&a, 0.04);
        to.advance(&b, 0.04); // 1.02: past the wrap
        let f = blend(&a, &b, 0.5, Some(0.5), &from, &to);
        let phase = f.warp.oscillators[0].phase;
        let off = phase.min(1.0 - phase);
        assert!(
            off < 1e-4,
            "expected ~0.0 (half way from 0.98 to 1.02), got {phase}"
        );
    }
}
