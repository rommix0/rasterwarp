//! Every live parameter, edited by the UI and read by the render passes each frame.

use std::f32::consts::PI;
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

use crate::save::saved_names;

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
    /// Clip playback speed, × the clip's own frame rate; negative plays backwards.
    pub const PLAY_SPEED: RangeInclusive<f32> = -4.0..=4.0;
    pub const POSITION: RangeInclusive<f32> = 0.0..=1.0;
    /// Seconds. A camera's delay is also held to its buffer length when used.
    pub const DELAY: RangeInclusive<f32> = 0.0..=30.0;
    /// Seconds. The GPU's frame ring may allow less, which is applied when used.
    pub const SLIT_DEPTH: RangeInclusive<f32> = 0.0..=30.0;
}

pub const OSCILLATOR_COUNT: usize = 4;
pub const PALETTE_SIZE: usize = 8;
/// One threshold between each pair of neighbouring levels.
pub const THRESHOLD_COUNT: usize = PALETTE_SIZE - 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Waveform {
    #[default]
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

saved_names!(Waveform {
    Sine => "sine",
    Triangle => "triangle",
    Ramp => "ramp",
    Square => "square",
    Noise => "noise",
});

/// Which displacement component an oscillator adds to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Axis {
    #[default]
    X,
    Y,
}

saved_names!(Axis { X => "x", Y => "y" });

/// What drives an oscillator's phase across the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OscInput {
    /// Horizontal position.
    U,
    /// Vertical position: X displacement driven by V gives the classic per-scanline wiggle.
    #[default]
    V,
    /// Distance from the frame center.
    Radius,
    /// Time only: the whole frame moves together.
    Time,
}

impl OscInput {
    pub const ALL: [OscInput; 4] = [OscInput::U, OscInput::V, OscInput::Radius, OscInput::Time];
}

saved_names!(OscInput {
    U => "u",
    V => "v",
    Radius => "radius",
    Time => "time",
});

/// How an oscillator's phase moves over time (the manual's sync modes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OscSync {
    /// Phase advances at `phase_speed`: the wave drifts.
    #[default]
    Free,
    /// Phase is locked to the frame: the wave stands still.
    Frame,
}

impl OscSync {
    pub const ALL: [OscSync; 2] = [OscSync::Free, OscSync::Frame];
}

saved_names!(OscSync {
    Free => "free",
    Frame => "frame",
});

/// How an oscillator's amplitude behaves while a transition or sequence ramp runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Envelope {
    /// Always at its set amplitude.
    #[default]
    Constant,
    /// Silent at rest; swells to its amplitude in the middle of a ramp.
    Swell,
}

impl Envelope {
    pub const ALL: [Envelope; 2] = [Envelope::Constant, Envelope::Swell];
}

saved_names!(Envelope {
    Constant => "constant",
    Swell => "swell",
});

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyParams {
    pub enabled: bool,
    /// Bit i set: level i is see-through.
    pub levels: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeedbackParams {
    /// How much of the previous canvas frame survives each canvas frame (so trails
    /// decay at the program frame rate). 0 = no trails.
    pub amount: f32,
    /// Per-canvas-frame transform applied to the previous canvas frame.
    pub zoom: f32,
    pub rotation: f32,
    pub offset: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GlowParams {
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    pub scanline_strength: f32,
    pub scanline_count: f32,
    /// Chromatic bleed, in internal-resolution pixels.
    pub chroma: f32,
    pub noise: f32,
}

/// How a clip plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayMode {
    /// Plays at `speed` and wraps around at the ends.
    #[default]
    Loop,
    /// Plays at `speed` and bounces back at the ends.
    PingPong,
    /// Shows the frame at `position`; transitions and cues ramp through the clip.
    Scrub,
}

impl PlayMode {
    pub const ALL: [PlayMode; 3] = [PlayMode::Loop, PlayMode::PingPong, PlayMode::Scrub];

    pub fn label(self) -> &'static str {
        match self {
            PlayMode::Loop => "Loop",
            PlayMode::PingPong => "Ping-pong",
            PlayMode::Scrub => "Scrub",
        }
    }
}

saved_names!(PlayMode {
    Loop => "loop",
    PingPong => "ping-pong",
    Scrub => "scrub",
});

/// What shows when the playhead sits between two frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Between {
    Nearest,
    #[default]
    Blend,
}

impl Between {
    pub const ALL: [Between; 2] = [Between::Nearest, Between::Blend];

    pub fn label(self) -> &'static str {
        match self {
            Between::Nearest => "Nearest",
            Between::Blend => "Blend",
        }
    }
}

saved_names!(Between {
    Nearest => "nearest",
    Blend => "blend",
});

/// Slit-scan: which pixels show older moments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Slit {
    #[default]
    Off,
    /// The top row is now, the bottom row is the depth ago.
    Rows,
    /// The left column is now, the right column is the depth ago.
    Columns,
    /// A black-and-white map image: black is now, white is the depth ago.
    Map,
}

impl Slit {
    pub const ALL: [Slit; 4] = [Slit::Off, Slit::Rows, Slit::Columns, Slit::Map];

    pub fn label(self) -> &'static str {
        match self {
            Slit::Off => "Off",
            Slit::Rows => "Rows",
            Slit::Columns => "Columns",
            Slit::Map => "Map",
        }
    }
}

saved_names!(Slit {
    Off => "off",
    Rows => "rows",
    Columns => "columns",
    Map => "map",
});

/// How a video or camera input plays. Images ignore it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VideoParams {
    /// Clips only.
    pub mode: PlayMode,
    /// Loop and Ping-pong: × the clip's own frame rate; negative plays backwards.
    pub speed: f32,
    /// Scrub: 0 = the first frame, 1 = the last.
    pub position: f32,
    pub between: Between,
    /// Cameras only: how many seconds behind live.
    pub delay: f32,
    pub slit: Slit,
    /// Seconds behind the playhead at the far end of the slit-scan map.
    pub slit_depth: f32,
    /// Reverses the slit-scan map, so its far end is now.
    pub slit_flip: bool,
}

impl Default for VideoParams {
    fn default() -> Self {
        Self {
            mode: PlayMode::Loop,
            speed: 1.0,
            position: 0.0,
            between: Between::Blend,
            delay: 0.0,
            slit: Slit::Off,
            slit_depth: 1.0,
            slit_flip: false,
        }
    }
}

/// The two inputs: the source the passes manipulate, and the background keyed levels
/// show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Source,
    Background,
}

impl Role {
    pub const ALL: [Role; 2] = [Role::Source, Role::Background];

    pub fn index(self) -> usize {
        match self {
            Role::Source => 0,
            Role::Background => 1,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Role::Source => "Source",
            Role::Background => "Background",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    pub warp: WarpParams,
    pub colorize: ColorizeParams,
    pub feedback: FeedbackParams,
    pub glow: GlowParams,
    pub raster: RasterParams,
    pub key: KeyParams,
    pub source_video: VideoParams,
    pub background_video: VideoParams,
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
            source_video: VideoParams::default(),
            background_video: VideoParams::default(),
        }
    }
}

// Each part's default is the default look's part, so a file missing a field gets the
// value a fresh session has.
impl Default for WarpParams {
    fn default() -> Self {
        Params::default().warp
    }
}

impl Default for ColorizeParams {
    fn default() -> Self {
        Params::default().colorize
    }
}

impl Default for FeedbackParams {
    fn default() -> Self {
        Params::default().feedback
    }
}

impl Default for GlowParams {
    fn default() -> Self {
        Params::default().glow
    }
}

impl Default for RasterParams {
    fn default() -> Self {
        Params::default().raster
    }
}

impl Default for KeyParams {
    fn default() -> Self {
        Params::default().key
    }
}

fn clamp(value: &mut f32, range: RangeInclusive<f32>) {
    *value = value.clamp(*range.start(), *range.end());
}

impl Params {
    /// A copy with every value inside its slider range, for parameters read from a file.
    pub fn clamped(mut self) -> Self {
        let w = &mut self.warp;
        for o in &mut w.oscillators {
            clamp(&mut o.frequency, ranges::FREQUENCY);
            clamp(&mut o.amplitude, ranges::AMPLITUDE);
            clamp(&mut o.phase, ranges::PHASE);
            clamp(&mut o.phase_speed, ranges::PHASE_SPEED);
            clamp(&mut o.lfo_rate, ranges::LFO_RATE);
            clamp(&mut o.lfo_depth, ranges::LFO_DEPTH);
        }
        clamp(&mut w.zoom, ranges::ZOOM);
        clamp(&mut w.rotation, ranges::ROTATION);
        for v in &mut w.offset {
            clamp(v, ranges::OFFSET);
        }
        clamp(&mut w.drift, ranges::DRIFT);
        clamp(&mut w.line_jitter, ranges::LINE_JITTER);
        clamp(&mut w.axis_wander, ranges::AXIS_WANDER);

        let c = &mut self.colorize;
        c.levels = c
            .levels
            .clamp(*ranges::LEVELS.start(), *ranges::LEVELS.end());
        clamp(&mut c.softness, ranges::SOFTNESS);
        for v in c.palette.iter_mut().flatten() {
            clamp(v, 0.0..=1.0);
        }
        clamp(&mut c.cycle_speed, ranges::CYCLE_SPEED);
        for t in &mut c.thresholds {
            clamp(t, ranges::THRESHOLD);
        }
        clamp(&mut c.bandwidth, ranges::BANDWIDTH);
        clamp(&mut c.ringing, ranges::RINGING);

        let f = &mut self.feedback;
        clamp(&mut f.amount, ranges::FEEDBACK_AMOUNT);
        clamp(&mut f.zoom, ranges::FEEDBACK_ZOOM);
        clamp(&mut f.rotation, ranges::FEEDBACK_ROTATION);
        for v in &mut f.offset {
            clamp(v, ranges::FEEDBACK_OFFSET);
        }

        let g = &mut self.glow;
        clamp(&mut g.bloom_intensity, ranges::BLOOM_INTENSITY);
        clamp(&mut g.bloom_threshold, ranges::BLOOM_THRESHOLD);
        clamp(&mut g.scanline_strength, ranges::SCANLINE_STRENGTH);
        clamp(&mut g.scanline_count, ranges::SCANLINE_COUNT);
        clamp(&mut g.chroma, ranges::CHROMA);
        clamp(&mut g.noise, ranges::NOISE);

        let r = &mut self.raster;
        r.lines = r
            .lines
            .clamp(*ranges::RASTER_LINES.start(), *ranges::RASTER_LINES.end());
        clamp(&mut r.beam_width, ranges::BEAM_WIDTH);
        clamp(&mut r.compensation, ranges::COMPENSATION);
        clamp(&mut r.speed_compensation, ranges::SPEED_COMPENSATION);

        // Levels the colorizer doesn't have can't be see-through.
        self.key.keep_levels(self.colorize.levels);

        for v in [&mut self.source_video, &mut self.background_video] {
            clamp(&mut v.speed, ranges::PLAY_SPEED);
            clamp(&mut v.position, ranges::POSITION);
            clamp(&mut v.delay, ranges::DELAY);
            clamp(&mut v.slit_depth, ranges::SLIT_DEPTH);
        }
        self
    }

    pub fn video(&self, role: Role) -> &VideoParams {
        match role {
            Role::Source => &self.source_video,
            Role::Background => &self.background_video,
        }
    }

    pub fn video_mut(&mut self, role: Role) -> &mut VideoParams {
        match role {
            Role::Source => &mut self.source_video,
            Role::Background => &mut self.background_video,
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
        for v in [p.source_video, p.background_video] {
            assert!(ranges::PLAY_SPEED.contains(&v.speed));
            assert!(ranges::POSITION.contains(&v.position));
            assert!(ranges::DELAY.contains(&v.delay));
            assert!(ranges::SLIT_DEPTH.contains(&v.slit_depth));
        }
    }

    #[test]
    fn video_settings_default_to_looping_at_normal_speed() {
        let v = VideoParams::default();
        assert_eq!((v.mode, v.speed, v.position), (PlayMode::Loop, 1.0, 0.0));
        assert_eq!((v.between, v.delay), (Between::Blend, 0.0));
        assert_eq!((v.slit, v.slit_depth, v.slit_flip), (Slit::Off, 1.0, false));
        let p = Params::default();
        assert_eq!(*p.video(Role::Source), v);
        assert_eq!(*p.video(Role::Background), v);
    }

    #[test]
    fn video_settings_are_clamped() {
        let mut p = Params::default();
        let v = p.video_mut(Role::Background);
        v.speed = -9.0;
        v.position = 1.5;
        v.delay = 99.0;
        v.slit_depth = -1.0;
        let v = p.clamped().background_video;
        assert_eq!((v.speed, v.position), (-4.0, 1.0));
        assert_eq!((v.delay, v.slit_depth), (30.0, 0.0));
    }

    #[test]
    fn clamping_moves_values_into_their_ranges() {
        assert_eq!(Params::default().clamped(), Params::default());
        let mut p = Params::default();
        p.warp.zoom = 99.0;
        p.warp.oscillators[2].frequency = -5.0;
        p.colorize.levels = 50;
        p.colorize.palette[3][1] = 2.0;
        p.raster.lines = 5;
        p.key.levels = 0xff;
        let p = p.clamped();
        assert_eq!(p.warp.zoom, 4.0);
        assert_eq!(p.warp.oscillators[2].frequency, 0.0);
        assert_eq!(p.colorize.levels, 8);
        assert_eq!(p.colorize.palette[3][1], 1.0);
        assert_eq!(p.raster.lines, 100);
        assert_eq!(p.key.levels, 0xff, "all eight levels exist");
        let mut few = Params::default();
        few.colorize.levels = 3;
        few.key.levels = 0xff;
        assert_eq!(few.clamped().key.levels, 0b111);
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
