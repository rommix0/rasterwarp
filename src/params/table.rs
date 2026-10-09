//! The sliders and choices links can drive (MIDI now, audio later). Each is named once
//! with a permanent id, and read and written in a [`Params`] exactly as the panel does,
//! including the rules that tie values together.

use std::ops::RangeInclusive;

use super::{
    Axis, Between, Envelope, OSCILLATOR_COUNT, OscInput, OscSync, Params, PlayMode, Role, Slit,
    THRESHOLD_COUNT, Waveform, even_thresholds, ranges,
};
use crate::save::Named;

/// An oscillator's sliders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OscSlider {
    Amplitude,
    Frequency,
    Phase,
    PhaseSpeed,
    LfoRate,
    LfoDepth,
}

impl OscSlider {
    /// All of them, in panel order.
    pub const ALL: [OscSlider; 6] = [
        OscSlider::Amplitude,
        OscSlider::Frequency,
        OscSlider::Phase,
        OscSlider::PhaseSpeed,
        OscSlider::LfoRate,
        OscSlider::LfoDepth,
    ];

    fn name(self) -> &'static str {
        match self {
            OscSlider::Amplitude => "amplitude",
            OscSlider::Frequency => "frequency",
            OscSlider::Phase => "phase",
            OscSlider::PhaseSpeed => "phase-speed",
            OscSlider::LfoRate => "lfo-rate",
            OscSlider::LfoDepth => "lfo-depth",
        }
    }

    fn label(self) -> &'static str {
        match self {
            OscSlider::Amplitude => "amplitude",
            OscSlider::Frequency => "frequency",
            OscSlider::Phase => "phase",
            OscSlider::PhaseSpeed => "phase speed",
            OscSlider::LfoRate => "LFO rate",
            OscSlider::LfoDepth => "LFO depth",
        }
    }

    fn range(self) -> RangeInclusive<f32> {
        match self {
            OscSlider::Amplitude => ranges::AMPLITUDE,
            OscSlider::Frequency => ranges::FREQUENCY,
            OscSlider::Phase => ranges::PHASE,
            OscSlider::PhaseSpeed => ranges::PHASE_SPEED,
            OscSlider::LfoRate => ranges::LFO_RATE,
            OscSlider::LfoDepth => ranges::LFO_DEPTH,
        }
    }
}

/// A video or camera input's sliders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VideoSlider {
    Speed,
    Position,
    Delay,
    SlitDepth,
}

impl VideoSlider {
    /// All of them, in panel order.
    pub const ALL: [VideoSlider; 4] = [
        VideoSlider::Speed,
        VideoSlider::Position,
        VideoSlider::Delay,
        VideoSlider::SlitDepth,
    ];

    fn name(self) -> &'static str {
        match self {
            VideoSlider::Speed => "speed",
            VideoSlider::Position => "position",
            VideoSlider::Delay => "delay",
            VideoSlider::SlitDepth => "slit-depth",
        }
    }

    fn label(self) -> &'static str {
        match self {
            VideoSlider::Speed => "speed",
            VideoSlider::Position => "position",
            VideoSlider::Delay => "delay",
            VideoSlider::SlitDepth => "slit-scan depth",
        }
    }

    fn range(self) -> RangeInclusive<f32> {
        match self {
            VideoSlider::Speed => ranges::PLAY_SPEED,
            VideoSlider::Position => ranges::POSITION,
            VideoSlider::Delay => ranges::DELAY,
            VideoSlider::SlitDepth => ranges::SLIT_DEPTH,
        }
    }
}

/// A slider a link can move: one bank setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SliderId {
    /// A slider of oscillator `0..OSCILLATOR_COUNT`.
    Osc(usize, OscSlider),
    Zoom,
    Rotation,
    OffsetX,
    OffsetY,
    Drift,
    AxisWander,
    LineJitter,
    RasterLines,
    BeamWidth,
    Compensation,
    SpeedCompensation,
    Levels,
    /// Where level `k + 2` starts: threshold `k`.
    LevelFrom(usize),
    Softness,
    Fringe,
    Ringing,
    CycleSpeed,
    FeedbackAmount,
    FeedbackZoom,
    FeedbackRotation,
    FeedbackOffsetX,
    FeedbackOffsetY,
    Bloom,
    BloomThreshold,
    Scanlines,
    ScanlineCount,
    ChromaBleed,
    Noise,
    Video(Role, VideoSlider),
}

/// The `f32` field a plain slider edits, borrowed shared or (with `mut`) mutably. Levels,
/// raster lines and thresholds have rules of their own and aren't here.
macro_rules! float_field {
    ($id:expr, $p:expr, $($m:tt)*) => {
        match $id {
            SliderId::Osc(i, s) => {
                let o = & $($m)* $p.warp.oscillators[i];
                match s {
                    OscSlider::Amplitude => & $($m)* o.amplitude,
                    OscSlider::Frequency => & $($m)* o.frequency,
                    OscSlider::Phase => & $($m)* o.phase,
                    OscSlider::PhaseSpeed => & $($m)* o.phase_speed,
                    OscSlider::LfoRate => & $($m)* o.lfo_rate,
                    OscSlider::LfoDepth => & $($m)* o.lfo_depth,
                }
            }
            SliderId::Zoom => & $($m)* $p.warp.zoom,
            SliderId::Rotation => & $($m)* $p.warp.rotation,
            SliderId::OffsetX => & $($m)* $p.warp.offset[0],
            SliderId::OffsetY => & $($m)* $p.warp.offset[1],
            SliderId::Drift => & $($m)* $p.warp.drift,
            SliderId::AxisWander => & $($m)* $p.warp.axis_wander,
            SliderId::LineJitter => & $($m)* $p.warp.line_jitter,
            SliderId::BeamWidth => & $($m)* $p.raster.beam_width,
            SliderId::Compensation => & $($m)* $p.raster.compensation,
            SliderId::SpeedCompensation => & $($m)* $p.raster.speed_compensation,
            SliderId::Softness => & $($m)* $p.colorize.softness,
            SliderId::Fringe => & $($m)* $p.colorize.bandwidth,
            SliderId::Ringing => & $($m)* $p.colorize.ringing,
            SliderId::CycleSpeed => & $($m)* $p.colorize.cycle_speed,
            SliderId::FeedbackAmount => & $($m)* $p.feedback.amount,
            SliderId::FeedbackZoom => & $($m)* $p.feedback.zoom,
            SliderId::FeedbackRotation => & $($m)* $p.feedback.rotation,
            SliderId::FeedbackOffsetX => & $($m)* $p.feedback.offset[0],
            SliderId::FeedbackOffsetY => & $($m)* $p.feedback.offset[1],
            SliderId::Bloom => & $($m)* $p.glow.bloom_intensity,
            SliderId::BloomThreshold => & $($m)* $p.glow.bloom_threshold,
            SliderId::Scanlines => & $($m)* $p.glow.scanline_strength,
            SliderId::ScanlineCount => & $($m)* $p.glow.scanline_count,
            SliderId::ChromaBleed => & $($m)* $p.glow.chroma,
            SliderId::Noise => & $($m)* $p.glow.noise,
            SliderId::Video(role, s) => {
                let v = match role {
                    Role::Source => & $($m)* $p.source_video,
                    Role::Background => & $($m)* $p.background_video,
                };
                match s {
                    VideoSlider::Speed => & $($m)* v.speed,
                    VideoSlider::Position => & $($m)* v.position,
                    VideoSlider::Delay => & $($m)* v.delay,
                    VideoSlider::SlitDepth => & $($m)* v.slit_depth,
                }
            }
            SliderId::Levels | SliderId::RasterLines | SliderId::LevelFrom(_) => {
                unreachable!("{:?} has a rule of its own", $id)
            }
        }
    };
}

/// How a role is named in ids: `source`, `background`.
fn role_name(role: Role) -> &'static str {
    match role {
        Role::Source => "source",
        Role::Background => "background",
    }
}

fn whole_range(range: RangeInclusive<u32>) -> RangeInclusive<f32> {
    *range.start() as f32..=*range.end() as f32
}

impl SliderId {
    /// Every slider, in panel order.
    pub fn all() -> Vec<SliderId> {
        use SliderId::*;
        let mut all = Vec::new();
        for i in 0..OSCILLATOR_COUNT {
            all.extend(OscSlider::ALL.map(|s| Osc(i, s)));
        }
        all.extend([
            Zoom,
            Rotation,
            OffsetX,
            OffsetY,
            Drift,
            AxisWander,
            LineJitter,
            RasterLines,
            BeamWidth,
            Compensation,
            SpeedCompensation,
            Levels,
        ]);
        all.extend((0..THRESHOLD_COUNT).map(LevelFrom));
        all.extend([
            Softness,
            Fringe,
            Ringing,
            CycleSpeed,
            FeedbackAmount,
            FeedbackZoom,
            FeedbackRotation,
            FeedbackOffsetX,
            FeedbackOffsetY,
            Bloom,
            BloomThreshold,
            Scanlines,
            ScanlineCount,
            ChromaBleed,
            Noise,
        ]);
        for role in Role::ALL {
            all.extend(VideoSlider::ALL.map(|s| Video(role, s)));
        }
        all
    }

    /// Its permanent id, as links are saved: `warp.osc1.amplitude`, `source.speed`.
    pub fn id(self) -> String {
        use SliderId::*;
        match self {
            Osc(i, s) => format!("warp.osc{}.{}", i + 1, s.name()),
            Zoom => "warp.zoom".into(),
            Rotation => "warp.rotation".into(),
            OffsetX => "warp.offset-x".into(),
            OffsetY => "warp.offset-y".into(),
            Drift => "warp.drift".into(),
            AxisWander => "warp.axis-wander".into(),
            LineJitter => "warp.line-jitter".into(),
            RasterLines => "raster.lines".into(),
            BeamWidth => "raster.beam-width".into(),
            Compensation => "raster.compensation".into(),
            SpeedCompensation => "raster.speed-compensation".into(),
            Levels => "colorize.levels".into(),
            LevelFrom(k) => format!("colorize.level{}-from", k + 2),
            Softness => "colorize.softness".into(),
            Fringe => "colorize.edge-fringe".into(),
            Ringing => "colorize.ringing".into(),
            CycleSpeed => "colorize.cycle-speed".into(),
            FeedbackAmount => "feedback.amount".into(),
            FeedbackZoom => "feedback.zoom".into(),
            FeedbackRotation => "feedback.rotation".into(),
            FeedbackOffsetX => "feedback.offset-x".into(),
            FeedbackOffsetY => "feedback.offset-y".into(),
            Bloom => "glow.bloom".into(),
            BloomThreshold => "glow.bloom-threshold".into(),
            Scanlines => "glow.scanlines".into(),
            ScanlineCount => "glow.scanline-count".into(),
            ChromaBleed => "glow.chroma-bleed".into(),
            Noise => "glow.noise".into(),
            Video(role, s) => format!("{}.{}", role_name(role), s.name()),
        }
    }

    /// The slider with this id, if there is one.
    pub fn from_id(id: &str) -> Option<SliderId> {
        SliderId::all().into_iter().find(|s| s.id() == id)
    }

    /// Its name in the MIDI section: "Osc 1 amplitude", "Source speed".
    pub fn label(self) -> String {
        use SliderId::*;
        match self {
            Osc(i, s) => format!("Osc {} {}", i + 1, s.label()),
            Zoom => "Zoom".into(),
            Rotation => "Rotation".into(),
            OffsetX => "Offset x".into(),
            OffsetY => "Offset y".into(),
            Drift => "Analog drift".into(),
            AxisWander => "Axis wander".into(),
            LineJitter => "Line jitter".into(),
            RasterLines => "Raster lines".into(),
            BeamWidth => "Beam width".into(),
            Compensation => "Area compensation".into(),
            SpeedCompensation => "Speed compensation".into(),
            Levels => "Levels".into(),
            LevelFrom(k) => format!("Level {} from", k + 2),
            Softness => "Softness".into(),
            Fringe => "Edge fringe".into(),
            Ringing => "Ringing".into(),
            CycleSpeed => "Cycle speed".into(),
            FeedbackAmount => "Feedback amount".into(),
            FeedbackZoom => "Feedback zoom".into(),
            FeedbackRotation => "Feedback rotation".into(),
            FeedbackOffsetX => "Feedback offset x".into(),
            FeedbackOffsetY => "Feedback offset y".into(),
            Bloom => "Bloom".into(),
            BloomThreshold => "Bloom threshold".into(),
            Scanlines => "Scanlines".into(),
            ScanlineCount => "Scanline count".into(),
            ChromaBleed => "Chroma bleed".into(),
            Noise => "Noise".into(),
            Video(role, s) => format!("{} {}", role.label(), s.label()),
        }
    }

    /// The panel slider's range.
    pub fn range(self) -> RangeInclusive<f32> {
        use SliderId::*;
        match self {
            Osc(_, s) => s.range(),
            Zoom => ranges::ZOOM,
            Rotation => ranges::ROTATION,
            OffsetX | OffsetY => ranges::OFFSET,
            Drift => ranges::DRIFT,
            AxisWander => ranges::AXIS_WANDER,
            LineJitter => ranges::LINE_JITTER,
            RasterLines => whole_range(ranges::RASTER_LINES),
            BeamWidth => ranges::BEAM_WIDTH,
            Compensation => ranges::COMPENSATION,
            SpeedCompensation => ranges::SPEED_COMPENSATION,
            Levels => whole_range(ranges::LEVELS),
            LevelFrom(_) => ranges::THRESHOLD,
            Softness => ranges::SOFTNESS,
            Fringe => ranges::BANDWIDTH,
            Ringing => ranges::RINGING,
            CycleSpeed => ranges::CYCLE_SPEED,
            FeedbackAmount => ranges::FEEDBACK_AMOUNT,
            FeedbackZoom => ranges::FEEDBACK_ZOOM,
            FeedbackRotation => ranges::FEEDBACK_ROTATION,
            FeedbackOffsetX | FeedbackOffsetY => ranges::FEEDBACK_OFFSET,
            Bloom => ranges::BLOOM_INTENSITY,
            BloomThreshold => ranges::BLOOM_THRESHOLD,
            Scanlines => ranges::SCANLINE_STRENGTH,
            ScanlineCount => ranges::SCANLINE_COUNT,
            ChromaBleed => ranges::CHROMA,
            Noise => ranges::NOISE,
            Video(_, s) => s.range(),
        }
    }

    /// Whether it holds whole numbers (levels, raster lines).
    pub fn whole(self) -> bool {
        matches!(self, SliderId::Levels | SliderId::RasterLines)
    }

    /// Its value in `p`.
    pub fn get(self, p: &Params) -> f32 {
        match self {
            SliderId::Levels => p.colorize.levels as f32,
            SliderId::RasterLines => p.raster.lines as f32,
            SliderId::LevelFrom(k) => p.colorize.thresholds[k],
            _ => *float_field!(self, p,),
        }
    }

    /// Sets it to `value`, held to its range (and rounded if it's whole-numbered), with
    /// the rules the panel follows: a new level count evens the thresholds and drops
    /// keyed levels that are gone, and a threshold can't pass its neighbours.
    pub fn set(self, p: &mut Params, value: f32) {
        let range = self.range();
        let value = value.clamp(*range.start(), *range.end());
        match self {
            SliderId::Levels => {
                let levels = value.round() as u32;
                if levels != p.colorize.levels {
                    p.colorize.levels = levels;
                    p.colorize.thresholds = even_thresholds(levels);
                    p.key.keep_levels(levels);
                }
            }
            SliderId::RasterLines => p.raster.lines = value.round() as u32,
            SliderId::LevelFrom(k) => {
                let c = &mut p.colorize;
                let used = c.levels as usize - 1;
                let lo = if k == 0 { 0.0 } else { c.thresholds[k - 1] };
                let hi = if k + 1 < used {
                    c.thresholds[k + 1]
                } else {
                    1.0
                };
                // max then min, not clamp: thresholds from a file may be out of order.
                c.thresholds[k] = value.max(lo).min(hi);
            }
            _ => *float_field!(self, p, mut) = value,
        }
    }
}

/// A choice a link can pick an option of: one bank setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChoiceId {
    Waveform(usize),
    Input(usize),
    Sync(usize),
    Envelope(usize),
    /// Which displacement an oscillator adds to: → X or → Y.
    Axis(usize),
    PlayMode(Role),
    Between(Role),
    Slit(Role),
}

/// The saved names of `all`, in order.
fn names<T: Named>(all: &[T]) -> Vec<&'static str> {
    all.iter().map(|v| v.name()).collect()
}

/// Where `value` is in `all`.
fn position<T: PartialEq>(all: &[T], value: &T) -> usize {
    all.iter().position(|v| v == value).unwrap_or(0)
}

/// Sets `field` to option `option` of `all`, if there is one.
fn pick<T: Copy>(all: &[T], option: usize, field: &mut T) {
    if let Some(&value) = all.get(option) {
        *field = value;
    }
}

impl ChoiceId {
    /// Every choice, in panel order.
    pub fn all() -> Vec<ChoiceId> {
        let mut all = Vec::new();
        for i in 0..OSCILLATOR_COUNT {
            all.extend([
                ChoiceId::Waveform(i),
                ChoiceId::Input(i),
                ChoiceId::Sync(i),
                ChoiceId::Envelope(i),
                ChoiceId::Axis(i),
            ]);
        }
        for role in Role::ALL {
            all.extend([
                ChoiceId::PlayMode(role),
                ChoiceId::Between(role),
                ChoiceId::Slit(role),
            ]);
        }
        all
    }

    /// Its permanent id, as links are saved: `warp.osc1.waveform`, `source.play-mode`.
    pub fn id(self) -> String {
        match self {
            ChoiceId::Waveform(i) => format!("warp.osc{}.waveform", i + 1),
            ChoiceId::Input(i) => format!("warp.osc{}.input", i + 1),
            ChoiceId::Sync(i) => format!("warp.osc{}.sync", i + 1),
            ChoiceId::Envelope(i) => format!("warp.osc{}.envelope", i + 1),
            ChoiceId::Axis(i) => format!("warp.osc{}.axis", i + 1),
            ChoiceId::PlayMode(role) => format!("{}.play-mode", role_name(role)),
            ChoiceId::Between(role) => format!("{}.between", role_name(role)),
            ChoiceId::Slit(role) => format!("{}.slit-scan", role_name(role)),
        }
    }

    /// The choice with this id, if there is one.
    pub fn from_id(id: &str) -> Option<ChoiceId> {
        ChoiceId::all().into_iter().find(|c| c.id() == id)
    }

    /// Its name in the MIDI section: "Osc 1 waveform", "Source play mode".
    pub fn label(self) -> String {
        match self {
            ChoiceId::Waveform(i) => format!("Osc {} waveform", i + 1),
            ChoiceId::Input(i) => format!("Osc {} input", i + 1),
            ChoiceId::Sync(i) => format!("Osc {} sync", i + 1),
            ChoiceId::Envelope(i) => format!("Osc {} envelope", i + 1),
            ChoiceId::Axis(i) => format!("Osc {} axis", i + 1),
            ChoiceId::PlayMode(role) => format!("{} play mode", role.label()),
            ChoiceId::Between(role) => format!("{} between frames", role.label()),
            ChoiceId::Slit(role) => format!("{} slit-scan", role.label()),
        }
    }

    /// Its options' saved names, in panel order.
    pub fn options(self) -> Vec<&'static str> {
        match self {
            ChoiceId::Waveform(_) => names(&Waveform::ALL),
            ChoiceId::Input(_) => names(&OscInput::ALL),
            ChoiceId::Sync(_) => names(&OscSync::ALL),
            ChoiceId::Envelope(_) => names(&Envelope::ALL),
            ChoiceId::Axis(_) => names(&Axis::ALL),
            ChoiceId::PlayMode(_) => names(&PlayMode::ALL),
            ChoiceId::Between(_) => names(&Between::ALL),
            ChoiceId::Slit(_) => names(&Slit::ALL),
        }
    }

    /// The option chosen in `p`.
    pub fn get(self, p: &Params) -> usize {
        let osc = |i: usize| &p.warp.oscillators[i];
        match self {
            ChoiceId::Waveform(i) => position(&Waveform::ALL, &osc(i).waveform),
            ChoiceId::Input(i) => position(&OscInput::ALL, &osc(i).input),
            ChoiceId::Sync(i) => position(&OscSync::ALL, &osc(i).sync),
            ChoiceId::Envelope(i) => position(&Envelope::ALL, &osc(i).envelope),
            ChoiceId::Axis(i) => position(&Axis::ALL, &osc(i).target),
            ChoiceId::PlayMode(role) => position(&PlayMode::ALL, &p.video(role).mode),
            ChoiceId::Between(role) => position(&Between::ALL, &p.video(role).between),
            ChoiceId::Slit(role) => position(&Slit::ALL, &p.video(role).slit),
        }
    }

    /// Chooses option `option`; one that doesn't exist changes nothing.
    pub fn set(self, p: &mut Params, option: usize) {
        match self {
            ChoiceId::Waveform(i) => {
                pick(&Waveform::ALL, option, &mut p.warp.oscillators[i].waveform)
            }
            ChoiceId::Input(i) => pick(&OscInput::ALL, option, &mut p.warp.oscillators[i].input),
            ChoiceId::Sync(i) => pick(&OscSync::ALL, option, &mut p.warp.oscillators[i].sync),
            ChoiceId::Envelope(i) => {
                pick(&Envelope::ALL, option, &mut p.warp.oscillators[i].envelope)
            }
            ChoiceId::Axis(i) => pick(&Axis::ALL, option, &mut p.warp.oscillators[i].target),
            ChoiceId::PlayMode(role) => pick(&PlayMode::ALL, option, &mut p.video_mut(role).mode),
            ChoiceId::Between(role) => pick(&Between::ALL, option, &mut p.video_mut(role).between),
            ChoiceId::Slit(role) => pick(&Slit::ALL, option, &mut p.video_mut(role).slit),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{Axis, Between, Envelope, OscInput, OscSync, PlayMode, Slit, Waveform};
    use std::collections::HashSet;

    #[test]
    fn slider_ids_never_change() {
        let ids: Vec<String> = SliderId::all().iter().map(|s| s.id()).collect();
        let mut pinned = Vec::new();
        for osc in 1..=4 {
            for name in [
                "amplitude",
                "frequency",
                "phase",
                "phase-speed",
                "lfo-rate",
                "lfo-depth",
            ] {
                pinned.push(format!("warp.osc{osc}.{name}"));
            }
        }
        for id in [
            "warp.zoom",
            "warp.rotation",
            "warp.offset-x",
            "warp.offset-y",
            "warp.drift",
            "warp.axis-wander",
            "warp.line-jitter",
            "raster.lines",
            "raster.beam-width",
            "raster.compensation",
            "raster.speed-compensation",
            "colorize.levels",
            "colorize.level2-from",
            "colorize.level3-from",
            "colorize.level4-from",
            "colorize.level5-from",
            "colorize.level6-from",
            "colorize.level7-from",
            "colorize.level8-from",
            "colorize.softness",
            "colorize.edge-fringe",
            "colorize.ringing",
            "colorize.cycle-speed",
            "feedback.amount",
            "feedback.zoom",
            "feedback.rotation",
            "feedback.offset-x",
            "feedback.offset-y",
            "glow.bloom",
            "glow.bloom-threshold",
            "glow.scanlines",
            "glow.scanline-count",
            "glow.chroma-bleed",
            "glow.noise",
        ] {
            pinned.push(id.to_string());
        }
        for role in ["source", "background"] {
            for name in ["speed", "position", "delay", "slit-depth"] {
                pinned.push(format!("{role}.{name}"));
            }
        }
        assert_eq!(ids, pinned);
    }

    #[test]
    fn choice_ids_never_change() {
        let ids: Vec<String> = ChoiceId::all().iter().map(|c| c.id()).collect();
        let mut pinned = Vec::new();
        for osc in 1..=4 {
            for name in ["waveform", "input", "sync", "envelope", "axis"] {
                pinned.push(format!("warp.osc{osc}.{name}"));
            }
        }
        for role in ["source", "background"] {
            for name in ["play-mode", "between", "slit-scan"] {
                pinned.push(format!("{role}.{name}"));
            }
        }
        assert_eq!(ids, pinned);
    }

    #[test]
    fn ids_are_unique_and_found_again() {
        let mut seen = HashSet::new();
        for slider in SliderId::all() {
            assert!(seen.insert(slider.id()), "{} twice", slider.id());
            assert_eq!(SliderId::from_id(&slider.id()), Some(slider));
        }
        for choice in ChoiceId::all() {
            assert!(seen.insert(choice.id()), "{} twice", choice.id());
            assert_eq!(ChoiceId::from_id(&choice.id()), Some(choice));
        }
        assert_eq!(SliderId::from_id("warp.nothing"), None);
        assert_eq!(ChoiceId::from_id("warp.osc9.waveform"), None);
    }

    #[test]
    fn labels_read_naturally() {
        assert_eq!(
            SliderId::Osc(0, OscSlider::PhaseSpeed).label(),
            "Osc 1 phase speed"
        );
        assert_eq!(SliderId::LevelFrom(0).label(), "Level 2 from");
        assert_eq!(
            SliderId::Video(Role::Background, VideoSlider::SlitDepth).label(),
            "Background slit-scan depth"
        );
        assert_eq!(ChoiceId::Waveform(2).label(), "Osc 3 waveform");
        assert_eq!(
            ChoiceId::Between(Role::Source).label(),
            "Source between frames"
        );
    }

    #[test]
    fn defaults_lie_in_their_ranges() {
        let p = Params::default();
        for slider in SliderId::all() {
            let range = slider.range();
            assert!(range.contains(&slider.get(&p)), "{}", slider.id());
        }
    }

    #[test]
    fn plain_sliders_set_what_they_get_and_hold_to_their_range() {
        for slider in SliderId::all() {
            if matches!(slider, SliderId::Levels | SliderId::LevelFrom(_)) {
                continue; // they have rules of their own, tested below
            }
            let mut p = Params::default();
            let range = slider.range();
            let middle = (range.start() + range.end()) / 2.0;
            slider.set(&mut p, middle);
            let expected = if slider.whole() {
                middle.round()
            } else {
                middle
            };
            assert_eq!(slider.get(&p), expected, "{}", slider.id());
            slider.set(&mut p, range.end() + 100.0);
            assert_eq!(slider.get(&p), *range.end(), "{}", slider.id());
            slider.set(&mut p, range.start() - 100.0);
            assert_eq!(slider.get(&p), *range.start(), "{}", slider.id());
        }
    }

    #[test]
    fn sliders_reach_their_own_fields() {
        let mut p = Params::default();
        SliderId::Osc(2, OscSlider::LfoDepth).set(&mut p, 0.75);
        assert_eq!(p.warp.oscillators[2].lfo_depth, 0.75);
        assert_eq!(p.warp.oscillators[1].lfo_depth, 0.0);
        SliderId::FeedbackOffsetY.set(&mut p, 0.01);
        assert_eq!(p.feedback.offset, [0.0, 0.01]);
        SliderId::Video(Role::Background, VideoSlider::Delay).set(&mut p, 2.5);
        assert_eq!(p.background_video.delay, 2.5);
        assert_eq!(p.source_video.delay, 0.0);
        SliderId::RasterLines.set(&mut p, 333.4);
        assert_eq!(p.raster.lines, 333);
    }

    #[test]
    fn a_new_level_count_evens_the_thresholds_and_drops_gone_keyed_levels() {
        let mut p = Params::default();
        p.key.levels = 0b10_0001; // levels 1 and 6 see-through
        p.colorize.thresholds[0] = 0.05;
        SliderId::Levels.set(&mut p, 4.2);
        assert_eq!(p.colorize.levels, 4);
        assert_eq!(p.colorize.thresholds, even_thresholds(4));
        assert_eq!(p.key.levels, 0b1, "level 6 is gone");
        // The same count again (a wheel resting) keeps edited thresholds.
        p.colorize.thresholds[0] = 0.1;
        SliderId::Levels.set(&mut p, 4.0);
        assert_eq!(p.colorize.thresholds[0], 0.1);
    }

    #[test]
    fn a_threshold_cannot_pass_its_neighbours() {
        let mut p = Params::default(); // 6 levels: thresholds 1/6 … 5/6
        SliderId::LevelFrom(1).set(&mut p, 0.9);
        assert_eq!(p.colorize.thresholds[1], p.colorize.thresholds[2]);
        SliderId::LevelFrom(1).set(&mut p, 0.0);
        assert_eq!(p.colorize.thresholds[1], p.colorize.thresholds[0]);
        SliderId::LevelFrom(0).set(&mut p, 0.02);
        assert_eq!(p.colorize.thresholds[0], 0.02);
        SliderId::LevelFrom(4).set(&mut p, 1.0);
        assert_eq!(
            p.colorize.thresholds[4], 1.0,
            "the last used one goes up to 1"
        );
    }

    fn serde_names<T: serde::Serialize>(all: &[T]) -> Vec<String> {
        all.iter()
            .map(|v| {
                serde_json::to_value(v)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn choice_options_are_the_saved_names() {
        let options =
            |c: ChoiceId| -> Vec<String> { c.options().iter().map(|o| o.to_string()).collect() };
        assert_eq!(options(ChoiceId::Waveform(0)), serde_names(&Waveform::ALL));
        assert_eq!(options(ChoiceId::Input(1)), serde_names(&OscInput::ALL));
        assert_eq!(options(ChoiceId::Sync(2)), serde_names(&OscSync::ALL));
        assert_eq!(options(ChoiceId::Envelope(3)), serde_names(&Envelope::ALL));
        assert_eq!(options(ChoiceId::Axis(0)), serde_names(&Axis::ALL));
        assert_eq!(
            options(ChoiceId::PlayMode(Role::Source)),
            serde_names(&PlayMode::ALL)
        );
        assert_eq!(
            options(ChoiceId::Between(Role::Source)),
            serde_names(&Between::ALL)
        );
        assert_eq!(
            options(ChoiceId::Slit(Role::Background)),
            serde_names(&Slit::ALL)
        );
    }

    #[test]
    fn choices_pick_each_option_and_ignore_ones_that_dont_exist() {
        for choice in ChoiceId::all() {
            let mut p = Params::default();
            for option in 0..choice.options().len() {
                choice.set(&mut p, option);
                assert_eq!(choice.get(&p), option, "{}", choice.id());
            }
            choice.set(&mut p, 99);
            assert_eq!(
                choice.get(&p),
                choice.options().len() - 1,
                "{}",
                choice.id()
            );
        }
        let mut p = Params::default();
        ChoiceId::Slit(Role::Background).set(&mut p, 2);
        assert_eq!(p.background_video.slit, Slit::Columns);
        assert_eq!(p.source_video.slit, Slit::Off);
        ChoiceId::Axis(3).set(&mut p, 1);
        assert_eq!(p.warp.oscillators[3].target, Axis::Y);
    }
}
