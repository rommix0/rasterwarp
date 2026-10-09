# MIDI Control Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a MIDI controller move Rasterwarp's sliders, pick its choices and run its actions, with links learned by right-clicking a control and saved in the app settings.

**Architecture:** A parameter table (`src/params/table.rs`) names every linkable slider and choice with a permanent id and reads and writes it in a `Params`, carrying the rules that tie values together. `src/control.rs` holds MIDI messages, sources, targets, links, learning and applying a message to `Motion`. `src/midi.rs` opens every input port with `midir` and hands messages over a channel. The panel draws linkable sliders and choices through `src/link_ui.rs`, which also adds the right-click link menu, and `src/midi_ui.rs` draws the MIDI section. The app drains MIDI once per screen refresh, before drawing the panel.

**Tech Stack:** Rust 2024, egui 0.36, `midir` 0.11 (WinMM on Windows), serde/serde_json.

**Spec:** `docs/superpowers/specs/2026-10-09-rasterwarp-midi-control-design.md`

## Global Constraints

- Rust edition 2024, `rust-version = "1.88"`; egui/egui-wgpu/egui-winit 0.36; add `midir = "0.11"` (Task 4) and no other new dependency.
- Ids are permanent once released: lowercase, dot-separated, hyphenated words (`warp.osc1.amplitude`, `colorize.level2-from`, `source.slit-depth`, `transition.duration`). A test pins every one.
- Saved link strings: sources `"cc:1:1"`, `"pitch:1"`, `"note:1:60"` (kind, channel 1–16, number 0–127); targets `"slider:<id>"`, `"choice:<id>=<option>"`, `"action:<name>"`. Action names: `transition`, `cut`, `sequence-run`, `sequence-reset`, `source-loop`, `background-loop`, `record`, `save-still`, `clear-trails`, `pause`.
- A link moves its target the way a hand on the panel would: bank settings go into `Motion::editable()`.
- Note sources only link to choices and actions; CC and pitch sources only link to sliders.
- The pitch wheel's centre (8192) maps exactly halfway; CC is `value / 127`; whole-numbered sliders round.
- MIDI never blocks rendering or startup; ports are rescanned every 3 seconds and on Refresh.
- Code style: match the surrounding code — doc comments in plain sentences on every public item, comment density as in `src/inputs.rs`, no `unwrap` outside tests except where an invariant is stated.
- Rust string line continuations: a `\` at the very end of the line, then the next line indented. (Some shell heredocs eat the backslash — edit `.rs` files with the Edit/Write tools, not shell heredocs.)
- Commands: `cargo test --lib`, `cargo test --test smoke`, `cargo test --test capture`, `cargo test --test video`, `cargo clippy --all-targets`, `cargo fmt`. All must pass/clean before each commit.
- Don't re-run a full verification that already passed on the same tree.

## Rulings made while planning

- **The table lives in `src/params/table.rs`** (a submodule of `params`, declared with `pub mod table;` in `src/params.rs`), not inline in the 900-line `params.rs`. Its entries are typed enums (`SliderId`, `ChoiceId`) rather than function-pointer rows, because the oscillator and input indices are data. The transition duration and the mode, which live on `Motion` rather than in a bank, are the targets `Target::Duration` and `Target::Mode` in `control.rs`, with the ids `transition.duration` and `mode` the spec gives.
- **`saved_names!` also implements a new `save::Named` trait** (`fn name(self) -> &'static str`), so the table's option names are the saved names by construction.
- **The learning line shows at the top of the panel** (under the error lines) as well as in the MIDI section: a control is right-clicked far from the MIDI section, which is collapsed by default, so the prompt must be where the user is looking. Esc cancels from anywhere.
- **Combo-box choices** (waveform, input, sync, envelope) link through a right-click menu on the combo box that lists "Link *option* to MIDI" for each option; choices drawn as rows of labels (axis, play mode, between frames, slit-scan, mode) link per option by right-clicking the option.

---

### Task 1: The parameter table

**Files:**
- Modify: `src/save.rs` (the `saved_names!` macro, a `Named` trait)
- Modify: `src/params.rs` (declare `pub mod table;`, add `Axis::ALL`)
- Create: `src/params/table.rs`

**Interfaces:**
- Produces:
  - `pub trait crate::save::Named: Copy { fn name(self) -> &'static str; }`, implemented by every `saved_names!` enum.
  - `pub const Axis::ALL: [Axis; 2]`.
  - `crate::params::table::{OscSlider, VideoSlider, SliderId, ChoiceId}`:
    - `SliderId::all() -> Vec<SliderId>`, `id(self) -> String`, `from_id(&str) -> Option<SliderId>`, `label(self) -> String`, `range(self) -> RangeInclusive<f32>`, `whole(self) -> bool`, `get(self, &Params) -> f32`, `set(self, &mut Params, f32)`.
    - `ChoiceId::all() -> Vec<ChoiceId>`, `id(self) -> String`, `from_id(&str) -> Option<ChoiceId>`, `label(self) -> String`, `options(self) -> Vec<&'static str>`, `get(self, &Params) -> usize`, `set(self, &mut Params, usize)`.

- [ ] **Step 1: Add the `Named` trait to `saved_names!`**

In `src/save.rs`, replace the `saved_names` macro with:

```rust
/// An enum saved by name (see [`saved_names`]).
pub trait Named: Copy {
    /// The name it is saved as.
    fn name(self) -> &'static str;
}

/// Saves an enum as one of the given names. An unknown name (say, from a newer version)
/// reads as the enum's default instead of failing the whole file.
macro_rules! saved_names {
    ($ty:ty { $($variant:ident => $name:literal),+ $(,)? }) => {
        impl $crate::save::Named for $ty {
            fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)+
                }
            }
        }

        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str($crate::save::Named::name(*self))
            }
        }

        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let value = serde_json::Value::deserialize(d)?;
                Ok(match value.as_str() {
                    $(Some($name) => Self::$variant,)+
                    _ => Self::default(),
                })
            }
        }
    };
}
pub(crate) use saved_names;
```

In `src/params.rs`, give `Axis` an `ALL` like its neighbours (just above `saved_names!(Axis …)`):

```rust
impl Axis {
    pub const ALL: [Axis; 2] = [Axis::X, Axis::Y];
}
```

and declare the table module right after the `use` lines at the top:

```rust
pub mod table;
```

- [ ] **Step 2: Write the table's tests (they fail: the module is empty)**

Create `src/params/table.rs` containing only the test module below for now (plus `use super::*;` inside it), then run `cargo test --lib table` and see it fail to compile.

```rust
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
            for name in ["amplitude", "frequency", "phase", "phase-speed", "lfo-rate", "lfo-depth"] {
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
        assert_eq!(SliderId::Osc(0, OscSlider::PhaseSpeed).label(), "Osc 1 phase speed");
        assert_eq!(SliderId::LevelFrom(0).label(), "Level 2 from");
        assert_eq!(
            SliderId::Video(Role::Background, VideoSlider::SlitDepth).label(),
            "Background slit-scan depth"
        );
        assert_eq!(ChoiceId::Waveform(2).label(), "Osc 3 waveform");
        assert_eq!(ChoiceId::Between(Role::Source).label(), "Source between frames");
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
            let expected = if slider.whole() { middle.round() } else { middle };
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
        assert_eq!(p.colorize.thresholds[4], 1.0, "the last used one goes up to 1");
    }

    fn serde_names<T: serde::Serialize>(all: &[T]) -> Vec<String> {
        all.iter()
            .map(|v| serde_json::to_value(v).unwrap().as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn choice_options_are_the_saved_names() {
        let options = |c: ChoiceId| -> Vec<String> {
            c.options().iter().map(|o| o.to_string()).collect()
        };
        assert_eq!(options(ChoiceId::Waveform(0)), serde_names(&Waveform::ALL));
        assert_eq!(options(ChoiceId::Input(1)), serde_names(&OscInput::ALL));
        assert_eq!(options(ChoiceId::Sync(2)), serde_names(&OscSync::ALL));
        assert_eq!(options(ChoiceId::Envelope(3)), serde_names(&Envelope::ALL));
        assert_eq!(options(ChoiceId::Axis(0)), serde_names(&Axis::ALL));
        assert_eq!(options(ChoiceId::PlayMode(Role::Source)), serde_names(&PlayMode::ALL));
        assert_eq!(options(ChoiceId::Between(Role::Source)), serde_names(&Between::ALL));
        assert_eq!(options(ChoiceId::Slit(Role::Background)), serde_names(&Slit::ALL));
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
            assert_eq!(choice.get(&p), choice.options().len() - 1, "{}", choice.id());
        }
        let mut p = Params::default();
        ChoiceId::Slit(Role::Background).set(&mut p, 2);
        assert_eq!(p.background_video.slit, Slit::Columns);
        assert_eq!(p.source_video.slit, Slit::Off);
        ChoiceId::Axis(3).set(&mut p, 1);
        assert_eq!(p.warp.oscillators[3].target, Axis::Y);
    }
}
```

Run: `cargo test --lib table`
Expected: compile errors (`SliderId`, `ChoiceId` not found).

- [ ] **Step 3: Write the table**

Put this above the test module in `src/params/table.rs`:

```rust
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

fn names<T: Named>(all: &[T]) -> Vec<&'static str> {
    all.iter().map(|v| v.name()).collect()
}

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
            ChoiceId::Between(role) => {
                pick(&Between::ALL, option, &mut p.video_mut(role).between)
            }
            ChoiceId::Slit(role) => pick(&Slit::ALL, option, &mut p.video_mut(role).slit),
        }
    }
}
```

Note on the test `choices_pick_each_option_and_ignore_ones_that_dont_exist`: after the loop, `p` holds the last option, so setting option 99 must leave it there.

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib table` — expected: all 11 table tests pass.
Run: `cargo test --lib` — expected: every unit test passes (`saved_names_never_change` in `save.rs` still passes: the saved names didn't change).
Run: `cargo clippy --all-targets` and `cargo fmt` — expected clean.

- [ ] **Step 5: Commit**

```bash
git add src/save.rs src/params.rs src/params/table.rs
git commit -m "feat: a table of the sliders and choices links can drive"
```

---

### Task 2: Messages, sources, targets and links

**Files:**
- Create: `src/control.rs`
- Modify: `src/lib.rs` (add `pub mod control;` in alphabetical order)

**Interfaces:**
- Consumes (Task 1): `SliderId`, `ChoiceId` and their methods; `crate::save::Named`.
- Produces (`crate::control`):
  - `pub enum Message { Cc { channel: u8, number: u8, value: u8 }, Pitch { channel: u8, value: u16 }, NoteOn { channel: u8, number: u8 } }` with `parse(&[u8]) -> Option<Message>`, `source(self) -> Source`, `amount(self) -> Option<f32>`.
  - `pub enum Source { Cc { channel: u8, number: u8 }, Pitch { channel: u8 }, Note { channel: u8, number: u8 } }` with `saved(self) -> String`, `from_saved(&str) -> Option<Source>`, `label(self) -> String`, `continuous(self) -> bool`.
  - `pub enum Action { Transition, Cut, SequenceRun, SequenceReset, SourceLoop, BackgroundLoop, Record, SaveStill, ClearTrails, Pause }` with `ALL`, `name(self) -> &'static str`, `label(self) -> &'static str`.
  - `pub enum Target { Slider(SliderId), Duration, Choice(ChoiceId, usize), Mode(Mode), Action(Action) }` with `saved(self) -> String`, `from_saved(&str) -> Option<Target>`, `label(self) -> String`, `range(self) -> Option<RangeInclusive<f32>>`, `whole(self) -> bool`, `continuous(self) -> bool`.
  - `pub struct Link { pub source: Source, pub target: Target, pub min: f32, pub max: f32 }` with `new(Source, Target) -> Link`, `value(&self, amount: f32) -> f32`.
  - `pub mod saved_links` with `serialize(&[Link], S)` and `deserialize(D) -> Result<Vec<Link>, D::Error>` for `#[serde(with = "crate::control::saved_links")]`.
  - Constants `DURATION_ID: &str = "transition.duration"`, `MODE_ID: &str = "mode"`.

- [ ] **Step 1: Write the failing tests**

Create `src/control.rs` with just this test module (and `pub mod control;` in `src/lib.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::table::{OscSlider, VideoSlider};
    use crate::params::Role;

    #[test]
    fn messages_are_read_from_their_bytes() {
        assert_eq!(
            Message::parse(&[0xB0, 1, 64]),
            Some(Message::Cc { channel: 1, number: 1, value: 64 })
        );
        assert_eq!(
            Message::parse(&[0xE3, 0x00, 0x40]),
            Some(Message::Pitch { channel: 4, value: 8192 })
        );
        assert_eq!(
            Message::parse(&[0xEF, 0x7F, 0x7F]),
            Some(Message::Pitch { channel: 16, value: 16383 })
        );
        assert_eq!(
            Message::parse(&[0x92, 60, 100]),
            Some(Message::NoteOn { channel: 3, number: 60 })
        );
        assert_eq!(Message::parse(&[0x90, 60, 0]), None, "velocity 0 is a note-off");
        assert_eq!(Message::parse(&[0x80, 60, 64]), None, "note-off");
        assert_eq!(Message::parse(&[0xF8]), None, "clock");
        assert_eq!(Message::parse(&[0xB0, 1]), None, "cut short");
        assert_eq!(Message::parse(&[]), None);
    }

    #[test]
    fn wheels_give_amounts_with_the_pitch_centre_exactly_halfway() {
        let pitch = |value| Message::Pitch { channel: 1, value }.amount().unwrap();
        assert_eq!(pitch(0), 0.0);
        assert_eq!(pitch(8192), 0.5);
        assert_eq!(pitch(16383), 1.0);
        let cc = |value| Message::Cc { channel: 1, number: 1, value }.amount().unwrap();
        assert_eq!(cc(0), 0.0);
        assert_eq!(cc(127), 1.0);
        assert_eq!(Message::NoteOn { channel: 1, number: 60 }.amount(), None);
    }

    #[test]
    fn sources_are_saved_and_named() {
        let cc = Source::Cc { channel: 1, number: 1 };
        let pitch = Source::Pitch { channel: 2 };
        let note = Source::Note { channel: 1, number: 60 };
        assert_eq!(cc.saved(), "cc:1:1");
        assert_eq!(pitch.saved(), "pitch:2");
        assert_eq!(note.saved(), "note:1:60");
        for source in [cc, pitch, note] {
            assert_eq!(Source::from_saved(&source.saved()), Some(source));
        }
        assert_eq!(cc.label(), "CC 1 ch 1");
        assert_eq!(pitch.label(), "Pitch ch 2");
        assert_eq!(note.label(), "Note C4 ch 1");
        assert_eq!(Source::Note { channel: 1, number: 1 }.label(), "Note C#-1 ch 1");
        for bad in ["cc:17:1", "cc:0:1", "cc:1:128", "pitch:0", "note:1", "foo:1:1", ""] {
            assert_eq!(Source::from_saved(bad), None, "{bad}");
        }
        assert!(cc.continuous() && pitch.continuous() && !note.continuous());
        assert_eq!(Message::Cc { channel: 2, number: 7, value: 3 }.source(), Source::Cc {
            channel: 2,
            number: 7
        });
    }

    #[test]
    fn targets_are_saved_and_named() {
        let targets = [
            Target::Slider(SliderId::Osc(0, OscSlider::Amplitude)),
            Target::Slider(SliderId::Video(Role::Background, VideoSlider::SlitDepth)),
            Target::Duration,
            Target::Choice(ChoiceId::Waveform(2), 3),
            Target::Mode(Mode::Transition),
            Target::Action(Action::BackgroundLoop),
        ];
        let saved: Vec<String> = targets.iter().map(|t| t.saved()).collect();
        assert_eq!(
            saved,
            [
                "slider:warp.osc1.amplitude",
                "slider:background.slit-depth",
                "slider:transition.duration",
                "choice:warp.osc3.waveform=square",
                "choice:mode=transition",
                "action:background-loop",
            ]
        );
        for target in targets {
            assert_eq!(Target::from_saved(&target.saved()), Some(target));
        }
        for bad in [
            "slider:warp.nothing",
            "choice:warp.osc1.waveform=wobble",
            "choice:warp.osc1.waveform",
            "choice:mode=chaos",
            "action:explode",
            "knob:warp.zoom",
        ] {
            assert_eq!(Target::from_saved(bad), None, "{bad}");
        }
        assert_eq!(targets[0].label(), "Osc 1 amplitude");
        assert_eq!(targets[2].label(), "Transition duration");
        assert_eq!(targets[3].label(), "Osc 3 waveform: square");
        assert_eq!(targets[4].label(), "Mode: transition");
        assert_eq!(targets[5].label(), "Background camera loop");
        assert!(targets[0].continuous() && targets[2].continuous());
        assert!(!targets[3].continuous() && !targets[4].continuous() && !targets[5].continuous());
        assert!(Target::Slider(SliderId::Levels).whole());
        assert!(!targets[0].whole());
    }

    #[test]
    fn action_names_never_change() {
        let names: Vec<&str> = Action::ALL.iter().map(|a| a.name()).collect();
        assert_eq!(
            names,
            [
                "transition",
                "cut",
                "sequence-run",
                "sequence-reset",
                "source-loop",
                "background-loop",
                "record",
                "save-still",
                "clear-trails",
                "pause",
            ]
        );
    }

    #[test]
    fn links_map_amounts_across_their_range() {
        let source = Source::Cc { channel: 1, number: 1 };
        let zoom = Link::new(source, Target::Slider(SliderId::Zoom));
        assert_eq!((zoom.min, zoom.max), (0.1, 4.0), "the slider's whole range");
        assert_eq!(zoom.value(0.0), 0.1);
        assert_eq!(zoom.value(1.0), 4.0);
        let inverted = Link { min: 2.0, max: 1.0, ..zoom };
        assert_eq!(inverted.value(0.0), 2.0);
        assert_eq!(inverted.value(0.25), 1.75);
        let levels = Link::new(source, Target::Slider(SliderId::Levels));
        assert_eq!(levels.value(0.5), 5.0, "2 + 6 × 0.5, rounded");
        assert_eq!(levels.value(0.45), 5.0);
        let duration = Link::new(source, Target::Duration);
        assert_eq!((duration.min, duration.max), (0.1, 30.0));
    }

    #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
    struct Holder {
        #[serde(with = "saved_links")]
        links: Vec<Link>,
    }

    #[test]
    fn links_are_saved_as_strings_and_unknown_ones_dropped() {
        let cc = Source::Cc { channel: 1, number: 1 };
        let key = Source::Note { channel: 1, number: 60 };
        let holder = Holder {
            links: vec![
                Link { min: 0.1, max: 0.2, ..Link::new(cc, Target::Slider(SliderId::Osc(0, OscSlider::Amplitude))) },
                Link::new(key, Target::Action(Action::Transition)),
            ],
        };
        let json = serde_json::to_value(&holder).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "links": [
                { "source": "cc:1:1", "target": "slider:warp.osc1.amplitude", "min": 0.1, "max": 0.2 },
                { "source": "note:1:60", "target": "action:transition" },
            ]})
        );
        let back: Holder = serde_json::from_value(json).unwrap();
        assert_eq!(back, holder);

        let read: Holder = serde_json::from_value(serde_json::json!({ "links": [
            { "source": "cc:1:1", "target": "slider:warp.nothing" },
            { "source": "osc:/x", "target": "slider:warp.zoom" },
            { "source": "note:1:60", "target": "slider:warp.zoom" },
            { "source": "cc:1:2", "target": "action:cut" },
            { "source": "cc:1:3", "target": "slider:warp.zoom", "min": -50.0, "max": "high" },
            { "source": "cc:1:4" },
            7,
            { "source": "pitch:1", "target": "slider:warp.zoom", "min": 0.5 },
        ]}))
        .unwrap();
        assert_eq!(
            read.links,
            [
                Link { min: 0.1, max: 4.0, ..Link::new(Source::Cc { channel: 1, number: 3 }, Target::Slider(SliderId::Zoom)) },
                Link { min: 0.5, max: 4.0, ..Link::new(Source::Pitch { channel: 1 }, Target::Slider(SliderId::Zoom)) },
            ],
            "unknown ids, sources, mismatched kinds and broken entries are dropped; \
             ranges are held to the slider's"
        );
        let empty: Holder = serde_json::from_value(serde_json::json!({ "links": "nope" })).unwrap();
        assert!(empty.links.is_empty());
    }
}
```

Note on `{ "min": -50.0, "max": "high" }`: reading is tolerant, like the rest of the settings. A min or max that is missing or not a number takes the slider's end, and both are held to its range, so this entry survives as 0.1 to 4.0 (the code in Step 2 reads each field from the JSON value with `as_f64`).

Run: `cargo test --lib control` — expected: fails to compile (types missing).

- [ ] **Step 2: Write `src/control.rs`**

Put this above the tests:

```rust
//! Links from a controller to the panel: MIDI messages, where they come from (a
//! source), what they move (a target), and the links between them as they're saved.

use std::ops::RangeInclusive;

use crate::motion::Mode;
use crate::params::table::{ChoiceId, SliderId};
use crate::save::Named;
use crate::transition::DURATION;

/// The transition duration's id; it lives on the transition, not in a bank.
pub const DURATION_ID: &str = "transition.duration";
/// The mode choice's id (Live, Transition, Sequence).
pub const MODE_ID: &str = "mode";

/// The pitch wheel at rest.
const PITCH_CENTRE: u16 = 8192;
const PITCH_MAX: u16 = 16383;

/// A MIDI message a link can use. Channels count from 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Message {
    Cc { channel: u8, number: u8, value: u8 },
    /// 0–16383, 8192 at rest.
    Pitch { channel: u8, value: u16 },
    NoteOn { channel: u8, number: u8 },
}

impl Message {
    /// Reads one MIDI message: a control change, the pitch wheel or a note-on. Anything
    /// else (note-offs, a note-on with velocity 0, clock, sysex) is None.
    pub fn parse(bytes: &[u8]) -> Option<Message> {
        let (&status, data) = bytes.split_first()?;
        let channel = (status & 0x0F) + 1;
        match (status & 0xF0, data) {
            (0xB0, &[number, value, ..]) => Some(Message::Cc {
                channel,
                number: number & 0x7F,
                value: value & 0x7F,
            }),
            (0xE0, &[lsb, msb, ..]) => Some(Message::Pitch {
                channel,
                value: u16::from(lsb & 0x7F) | (u16::from(msb & 0x7F) << 7),
            }),
            (0x90, &[number, velocity, ..]) if velocity > 0 => Some(Message::NoteOn {
                channel,
                number: number & 0x7F,
            }),
            _ => None,
        }
    }

    /// The control it came from.
    pub fn source(self) -> Source {
        match self {
            Message::Cc { channel, number, .. } => Source::Cc { channel, number },
            Message::Pitch { channel, .. } => Source::Pitch { channel },
            Message::NoteOn { channel, number } => Source::Note { channel, number },
        }
    }

    /// Where a wheel or fader is, 0–1, with the pitch wheel's centre exactly 0.5. None
    /// for a key.
    pub fn amount(self) -> Option<f32> {
        match self {
            Message::Cc { value, .. } => Some(f32::from(value) / 127.0),
            Message::Pitch { value, .. } if value >= PITCH_CENTRE => Some(
                0.5 + 0.5 * f32::from(value - PITCH_CENTRE) / f32::from(PITCH_MAX - PITCH_CENTRE),
            ),
            Message::Pitch { value, .. } => {
                Some(0.5 * f32::from(value) / f32::from(PITCH_CENTRE))
            }
            Message::NoteOn { .. } => None,
        }
    }
}

/// A control on a controller. Channels count from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A control change: a knob, fader or the mod wheel (CC 1).
    Cc { channel: u8, number: u8 },
    Pitch { channel: u8 },
    /// A key.
    Note { channel: u8, number: u8 },
}

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

impl Source {
    /// As saved: `cc:1:1`, `pitch:1`, `note:1:60`.
    pub fn saved(self) -> String {
        match self {
            Source::Cc { channel, number } => format!("cc:{channel}:{number}"),
            Source::Pitch { channel } => format!("pitch:{channel}"),
            Source::Note { channel, number } => format!("note:{channel}:{number}"),
        }
    }

    /// The source saved as `saved`, if it's one this version knows.
    pub fn from_saved(saved: &str) -> Option<Source> {
        let mut parts = saved.split(':');
        let kind = parts.next()?;
        let channel: u8 = parts.next()?.parse().ok().filter(|c| (1..=16).contains(c))?;
        let number = parts.next().map(|n| n.parse::<u8>().ok().filter(|n| *n <= 127));
        if parts.next().is_some() {
            return None;
        }
        match (kind, number) {
            ("cc", Some(Some(number))) => Some(Source::Cc { channel, number }),
            ("pitch", None) => Some(Source::Pitch { channel }),
            ("note", Some(Some(number))) => Some(Source::Note { channel, number }),
            _ => None,
        }
    }

    /// As people read it: "CC 1 ch 1", "Pitch ch 1", "Note C4 ch 1" (middle C is C4).
    pub fn label(self) -> String {
        match self {
            Source::Cc { channel, number } => format!("CC {number} ch {channel}"),
            Source::Pitch { channel } => format!("Pitch ch {channel}"),
            Source::Note { channel, number } => {
                let octave = i32::from(number / 12) - 1;
                let name = NOTE_NAMES[usize::from(number % 12)];
                format!("Note {name}{octave} ch {channel}")
            }
        }
    }

    /// Whether it's a wheel, fader or knob (which moves sliders) rather than a key
    /// (which picks options and runs actions).
    pub fn continuous(self) -> bool {
        !matches!(self, Source::Note { .. })
    }
}

/// Something a key can do: what a button or key in the app does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// Transition, or reverse or resume one (the Space key).
    Transition,
    Cut,
    /// Sequence Run, or Stop while it runs.
    SequenceRun,
    SequenceReset,
    /// The source camera's loop on or off (F9).
    SourceLoop,
    /// The background camera's loop on or off (F10).
    BackgroundLoop,
    /// Record, or Stop recording.
    Record,
    SaveStill,
    ClearTrails,
    /// Pause or resume.
    Pause,
}

impl Action {
    pub const ALL: [Action; 10] = [
        Action::Transition,
        Action::Cut,
        Action::SequenceRun,
        Action::SequenceReset,
        Action::SourceLoop,
        Action::BackgroundLoop,
        Action::Record,
        Action::SaveStill,
        Action::ClearTrails,
        Action::Pause,
    ];

    /// Its permanent name, as links are saved.
    pub fn name(self) -> &'static str {
        match self {
            Action::Transition => "transition",
            Action::Cut => "cut",
            Action::SequenceRun => "sequence-run",
            Action::SequenceReset => "sequence-reset",
            Action::SourceLoop => "source-loop",
            Action::BackgroundLoop => "background-loop",
            Action::Record => "record",
            Action::SaveStill => "save-still",
            Action::ClearTrails => "clear-trails",
            Action::Pause => "pause",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::Transition => "Transition",
            Action::Cut => "Cut",
            Action::SequenceRun => "Sequence run/stop",
            Action::SequenceReset => "Sequence reset",
            Action::SourceLoop => "Source camera loop",
            Action::BackgroundLoop => "Background camera loop",
            Action::Record => "Record/stop",
            Action::SaveStill => "Save still",
            Action::ClearTrails => "Clear trails",
            Action::Pause => "Pause",
        }
    }
}

/// What a link moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// A bank slider.
    Slider(SliderId),
    /// The transition's duration.
    Duration,
    /// Option `usize` of a bank choice.
    Choice(ChoiceId, usize),
    /// Live, Transition or Sequence.
    Mode(Mode),
    Action(Action),
}

impl Target {
    /// As saved: `slider:warp.zoom`, `choice:warp.osc1.waveform=square`,
    /// `action:transition`.
    pub fn saved(self) -> String {
        match self {
            Target::Slider(id) => format!("slider:{}", id.id()),
            Target::Duration => format!("slider:{DURATION_ID}"),
            Target::Choice(id, option) => {
                format!("choice:{}={}", id.id(), id.options()[option])
            }
            Target::Mode(mode) => format!("choice:{MODE_ID}={}", mode.name()),
            Target::Action(action) => format!("action:{}", action.name()),
        }
    }

    /// The target saved as `saved`, if it's one this version knows.
    pub fn from_saved(saved: &str) -> Option<Target> {
        let (kind, rest) = saved.split_once(':')?;
        match kind {
            "slider" if rest == DURATION_ID => Some(Target::Duration),
            "slider" => SliderId::from_id(rest).map(Target::Slider),
            "choice" => {
                let (id, option) = rest.split_once('=')?;
                if id == MODE_ID {
                    return Mode::ALL
                        .into_iter()
                        .find(|m| m.name() == option)
                        .map(Target::Mode);
                }
                let choice = ChoiceId::from_id(id)?;
                let index = choice.options().iter().position(|o| *o == option)?;
                Some(Target::Choice(choice, index))
            }
            "action" => Action::ALL
                .into_iter()
                .find(|a| a.name() == rest)
                .map(Target::Action),
            _ => None,
        }
    }

    /// Its name in the MIDI section: "Osc 1 amplitude", "Osc 1 waveform: square".
    pub fn label(self) -> String {
        match self {
            Target::Slider(id) => id.label(),
            Target::Duration => "Transition duration".into(),
            Target::Choice(id, option) => format!("{}: {}", id.label(), id.options()[option]),
            Target::Mode(mode) => format!("Mode: {}", mode.name()),
            Target::Action(action) => action.label().into(),
        }
    }

    /// A slider's range; None for a choice or action.
    pub fn range(self) -> Option<RangeInclusive<f32>> {
        match self {
            Target::Slider(id) => Some(id.range()),
            Target::Duration => Some(DURATION),
            _ => None,
        }
    }

    /// Whether it's a whole-numbered slider.
    pub fn whole(self) -> bool {
        matches!(self, Target::Slider(id) if id.whole())
    }

    /// Whether wheels and faders move it (a slider) rather than keys (a choice or
    /// action).
    pub fn continuous(self) -> bool {
        self.range().is_some()
    }
}

/// A control linked to a target. A slider target moves from `min` (the control at 0) to
/// `max` (the control at its top); `min` above `max` inverts it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    pub source: Source,
    pub target: Target,
    pub min: f32,
    pub max: f32,
}

impl Link {
    /// A link across the target's whole range.
    pub fn new(source: Source, target: Target) -> Link {
        let (min, max) = target
            .range()
            .map_or((0.0, 1.0), |r| (*r.start(), *r.end()));
        Link {
            source,
            target,
            min,
            max,
        }
    }

    /// The slider value for a control `amount` (0–1) of the way up, rounded for a
    /// whole-numbered slider.
    pub fn value(&self, amount: f32) -> f32 {
        let value = self.min + (self.max - self.min) * amount;
        if self.target.whole() {
            value.round()
        } else {
            value
        }
    }
}

/// Links as the settings save them: strings for the source and target, so a link this
/// version doesn't know (from a newer one, or a renamed control) is dropped with a log
/// line instead of failing the file. Use with `#[serde(with = "...")]`.
pub mod saved_links {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::Value;

    use super::{Link, Source, Target};

    #[derive(Serialize)]
    struct Saved {
        source: String,
        target: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        min: Option<f32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max: Option<f32>,
    }

    pub fn serialize<S: Serializer>(links: &[Link], s: S) -> Result<S::Ok, S::Error> {
        let saved: Vec<Saved> = links
            .iter()
            .map(|link| {
                let slider = link.target.continuous();
                Saved {
                    source: link.source.saved(),
                    target: link.target.saved(),
                    min: slider.then_some(link.min),
                    max: slider.then_some(link.max),
                }
            })
            .collect();
        saved.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Link>, D::Error> {
        let value = Value::deserialize(d)?;
        let entries = value.as_array().cloned().unwrap_or_default();
        Ok(entries
            .iter()
            .filter_map(|entry| {
                let link = read(entry);
                if link.is_none() {
                    log::warn!("ignoring a MIDI link this version doesn't know: {entry}");
                }
                link
            })
            .collect())
    }

    /// One saved link, if its source and target are known and fit each other. A missing
    /// or broken min or max takes the slider's end; both are held to its range.
    fn read(entry: &Value) -> Option<Link> {
        let source = Source::from_saved(entry.get("source")?.as_str()?)?;
        let target = Target::from_saved(entry.get("target")?.as_str()?)?;
        if source.continuous() != target.continuous() {
            return None;
        }
        let mut link = Link::new(source, target);
        if let Some(range) = target.range() {
            let read = |key: &str, default: f32| {
                entry
                    .get(key)
                    .and_then(Value::as_f64)
                    .map_or(default, |v| v as f32)
                    .clamp(*range.start(), *range.end())
            };
            link.min = read("min", link.min);
            link.max = read("max", link.max);
        }
        Some(link)
    }
}
```

Note: `Mode` needs `Hash` for `Target` to derive `Hash`. In `src/motion.rs`, add `Hash` to `Mode`'s derive list: `#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]`.

- [ ] **Step 3: Run the tests**

Run: `cargo test --lib control` — expected: all 8 pass.
Run: `cargo test --lib`, `cargo clippy --all-targets`, `cargo fmt` — expected clean.

- [ ] **Step 4: Commit**

```bash
git add src/control.rs src/lib.rs src/motion.rs
git commit -m "feat: MIDI messages, sources, targets and links"
```

---

### Task 3: Learning and applying links

**Files:**
- Modify: `src/control.rs`

**Interfaces:**
- Consumes (Task 2): `Message`, `Source`, `Target`, `Action`, `Link`; (Task 1) `SliderId::set`, `ChoiceId::set`; `crate::motion::Motion` (`editable()`, `set_mode()`, `pub ab` with `pub duration: f32`).
- Produces: `pub struct Links { pub list: Vec<Link>, /* private */ learning: Option<Target> }` with
  - `Links::new(list: Vec<Link>) -> Links` (and `Default`)
  - `learn(&mut self, target: Target)`, `learning(&self) -> Option<Target>`, `cancel(&mut self)`
  - `to(&self, target: Target) -> impl Iterator<Item = &Link>`
  - `unlink(&mut self, source: Source, target: Target)`, `remove(&mut self, index: usize)`
  - `handle(&mut self, message: Message, motion: &mut Motion) -> Vec<Action>`

- [ ] **Step 1: Write the failing tests** (append to `mod tests` in `src/control.rs`)

```rust
    use crate::motion::Motion;
    use crate::params::Params;

    fn cc(number: u8, value: u8) -> Message {
        Message::Cc { channel: 1, number, value }
    }

    fn key(number: u8) -> Message {
        Message::NoteOn { channel: 1, number }
    }

    const AMPLITUDE: Target = Target::Slider(SliderId::Osc(0, OscSlider::Amplitude));

    #[test]
    fn a_wheel_links_a_slider_and_keys_are_ignored_while_learning_it() {
        let mut links = Links::default();
        let mut motion = Motion::new(Params::default());
        links.learn(AMPLITUDE);
        assert_eq!(links.learning(), Some(AMPLITUDE));
        assert!(links.handle(key(60), &mut motion).is_empty());
        assert_eq!(links.learning(), Some(AMPLITUDE), "a key doesn't fit a slider");
        let before = motion.editable().warp.oscillators[0].amplitude;
        links.handle(cc(1, 127), &mut motion);
        assert_eq!(links.learning(), None);
        assert_eq!(links.list, [Link::new(Source::Cc { channel: 1, number: 1 }, AMPLITUDE)]);
        assert_eq!(
            motion.editable().warp.oscillators[0].amplitude,
            before,
            "the message that links does nothing else"
        );
    }

    #[test]
    fn a_key_links_an_action_or_option_and_wheels_are_ignored_while_learning_it() {
        let mut links = Links::default();
        let mut motion = Motion::new(Params::default());
        let cut = Target::Action(Action::Cut);
        links.learn(cut);
        links.handle(cc(1, 5), &mut motion);
        links.handle(Message::Pitch { channel: 1, value: 0 }, &mut motion);
        assert_eq!(links.learning(), Some(cut));
        links.handle(key(61), &mut motion);
        let square = Target::Choice(ChoiceId::Waveform(0), 3);
        links.learn(square);
        links.handle(key(62), &mut motion);
        assert_eq!(
            links.list,
            [
                Link::new(Source::Note { channel: 1, number: 61 }, cut),
                Link::new(Source::Note { channel: 1, number: 62 }, square),
            ]
        );
    }

    #[test]
    fn relearning_a_pair_keeps_one_link_and_cancel_stops_learning() {
        let mut links = Links::default();
        let mut motion = Motion::new(Params::default());
        for _ in 0..2 {
            links.learn(AMPLITUDE);
            links.handle(cc(1, 0), &mut motion);
        }
        assert_eq!(links.list.len(), 1);
        links.learn(Target::Action(Action::Cut));
        links.learn(AMPLITUDE);
        assert_eq!(links.learning(), Some(AMPLITUDE), "learning another replaces it");
        links.cancel();
        assert_eq!(links.learning(), None);
        links.handle(cc(7, 0), &mut motion);
        assert_eq!(links.list.len(), 1, "cancelled: nothing linked");
    }

    #[test]
    fn links_are_found_by_target_and_removed() {
        let a = Source::Cc { channel: 1, number: 1 };
        let b = Source::Pitch { channel: 1 };
        let mut links = Links::new(vec![
            Link::new(a, AMPLITUDE),
            Link::new(b, AMPLITUDE),
            Link::new(a, Target::Slider(SliderId::Zoom)),
        ]);
        let sources: Vec<Source> = links.to(AMPLITUDE).map(|l| l.source).collect();
        assert_eq!(sources, [a, b]);
        links.unlink(a, AMPLITUDE);
        assert_eq!(links.to(AMPLITUDE).count(), 1);
        assert_eq!(links.to(Target::Slider(SliderId::Zoom)).count(), 1);
        links.remove(0);
        links.remove(9); // out of range: nothing happens
        assert_eq!(links.list, [Link::new(a, Target::Slider(SliderId::Zoom))]);
    }

    #[test]
    fn wheels_move_the_bank_the_panel_edits() {
        let mut links = Links::new(vec![Link::new(
            Source::Cc { channel: 1, number: 1 },
            Target::Slider(SliderId::Zoom),
        )]);
        let mut motion = Motion::new(Params::default());
        links.handle(cc(1, 127), &mut motion);
        assert_eq!(motion.ab.banks[motion.ab.on_air].warp.zoom, 4.0, "Live: on air");

        motion.set_mode(Mode::Transition);
        links.handle(cc(1, 0), &mut motion);
        assert_eq!(motion.ab.banks[motion.ab.off_air()].warp.zoom, 0.1, "Transition: off air");
        assert_eq!(motion.ab.banks[motion.ab.on_air].warp.zoom, 4.0);

        motion.set_mode(Mode::Sequence);
        let seq = motion.sequence_mut();
        seq.add_cue();
        seq.selected = 1;
        links.handle(cc(1, 127), &mut motion);
        let seq = motion.sequence_mut();
        assert_eq!(seq.cues()[1].params.warp.zoom, 4.0, "Sequence: the selected cue");
        assert_ne!(seq.cues()[0].params.warp.zoom, 4.0);
    }

    #[test]
    fn keys_pick_options_and_modes_and_report_actions() {
        let note = |n| Source::Note { channel: 1, number: n };
        let mut links = Links::new(vec![
            Link::new(note(60), Target::Choice(ChoiceId::Waveform(1), 3)),
            Link::new(note(61), Target::Mode(Mode::Transition)),
            Link::new(note(62), Target::Action(Action::Cut)),
            Link::new(note(62), Target::Action(Action::Pause)),
            Link::new(Source::Pitch { channel: 1 }, Target::Duration),
        ]);
        let mut motion = Motion::new(Params::default());
        assert!(links.handle(key(60), &mut motion).is_empty());
        assert_eq!(motion.editable().warp.oscillators[1].waveform, crate::params::Waveform::Square);
        links.handle(key(61), &mut motion);
        assert_eq!(motion.mode(), Mode::Transition);
        assert_eq!(links.handle(key(62), &mut motion), [Action::Cut, Action::Pause]);
        links.handle(Message::Pitch { channel: 1, value: 8192 }, &mut motion);
        assert!((motion.ab.duration - 15.05).abs() < 1e-4, "halfway from 0.1 to 30");
        assert!(links.handle(key(70), &mut motion).is_empty(), "an unlinked key");
        assert!(
            links.handle(Message::NoteOn { channel: 2, number: 62 }, &mut motion).is_empty(),
            "another channel"
        );
    }
```

(`Sequence::cues()` returns `&[Cue]` with `pub params: Params`; `seq.selected` is a public field; `seq.add_cue()` appends a copy of the selected cue and selects it, so the new cue is `cues()[1]`.)

Run: `cargo test --lib control` — expected: compile errors (`Links` missing).

- [ ] **Step 2: Write `Links`** (in `src/control.rs`, after `Link`)

```rust
/// The links in use, and the target waiting for a control while one is being learned.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Links {
    pub list: Vec<Link>,
    learning: Option<Target>,
}

impl Links {
    pub fn new(list: Vec<Link>) -> Links {
        Links {
            list,
            learning: None,
        }
    }

    /// Waits for the next fitting control to link to `target`, instead of any target
    /// that was waiting.
    pub fn learn(&mut self, target: Target) {
        self.learning = Some(target);
    }

    /// The target waiting for a control.
    pub fn learning(&self) -> Option<Target> {
        self.learning
    }

    pub fn cancel(&mut self) {
        self.learning = None;
    }

    /// The links that drive `target`.
    pub fn to(&self, target: Target) -> impl Iterator<Item = &Link> {
        self.list.iter().filter(move |l| l.target == target)
    }

    pub fn unlink(&mut self, source: Source, target: Target) {
        self.list
            .retain(|l| !(l.source == source && l.target == target));
    }

    /// Removes link `index` of the list, if there is one.
    pub fn remove(&mut self, index: usize) {
        if index < self.list.len() {
            self.list.remove(index);
        }
    }

    /// Handles one message from a controller. While learning, a fitting message (a wheel
    /// or fader for a slider, a key for a choice or action) links the target waiting and
    /// does nothing else, and one that doesn't fit is ignored. Otherwise each link it
    /// drives moves its slider or picks its option in `motion`, as a hand on the panel
    /// would, and the actions it asks for are returned for the app to run.
    pub fn handle(&mut self, message: Message, motion: &mut Motion) -> Vec<Action> {
        let source = message.source();
        if let Some(target) = self.learning {
            if source.continuous() == target.continuous() {
                if !self
                    .list
                    .iter()
                    .any(|l| l.source == source && l.target == target)
                {
                    self.list.push(Link::new(source, target));
                }
                self.learning = None;
            }
            return Vec::new();
        }
        let mut actions = Vec::new();
        for link in self.list.iter().filter(|l| l.source == source) {
            match (link.target, message.amount()) {
                (Target::Slider(id), Some(amount)) => id.set(motion.editable(), link.value(amount)),
                (Target::Duration, Some(amount)) => {
                    motion.ab.duration = link
                        .value(amount)
                        .clamp(*DURATION.start(), *DURATION.end());
                }
                (Target::Choice(id, option), None) => id.set(motion.editable(), option),
                (Target::Mode(mode), None) => motion.set_mode(mode),
                (Target::Action(action), None) => actions.push(action),
                _ => {}
            }
        }
        actions
    }
}
```

Add `use crate::motion::Motion;` beside `use crate::motion::Mode;` at the top.

- [ ] **Step 3: Run the tests**

Run: `cargo test --lib control` — expected: all 14 pass.
Run: `cargo test --lib`, `cargo clippy --all-targets`, `cargo fmt` — clean.

- [ ] **Step 4: Commit**

```bash
git add src/control.rs
git commit -m "feat: learning links and applying them like a hand on the panel"
```

---

### Task 4: MIDI input ports

**Files:**
- Modify: `Cargo.toml` (add `midir = "0.11"` under `[dependencies]`, alphabetically after `log`)
- Create: `src/midi.rs`
- Modify: `src/lib.rs` (add `pub mod midi;`)

**Interfaces:**
- Consumes (Task 2): `crate::control::Message::parse`.
- Produces (`crate::midi`):
  - `pub const RESCAN: Duration` (3 s)
  - `pub struct Midi` with `Midi::new() -> Midi` (and `Default`), `poll(&mut self, now: Instant) -> Vec<Message>`, `rescan(&mut self, now: Instant)`, `devices(&self) -> Vec<String>`, `failed(&self) -> &[(String, String)]`
  - `pub fn list() -> Result<Vec<(String, String)>, String>` — (port id, name) pairs.

midir 0.11 API (verified): `midir::MidiInput::new(client) -> Result<MidiInput, InitError>`; `input.ports() -> Vec<MidiInputPort>`; `port.id() -> String`; `input.port_name(&port) -> Result<String, PortInfoError>`; `input.find_port_by_id(&id) -> Option<MidiInputPort>`; `input.connect(&port, name, callback: FnMut(u64, &[u8], &mut T) + Send + 'static, data: T) -> Result<MidiInputConnection<T>, ConnectError<MidiInput>>` (it consumes the `MidiInput`, so each port needs its own). Errors implement `Display`. On this machine the ports are "theLooper" (a virtualMIDI loopback port) and "Q25" (the user's keyboard). WinMM input ports are exclusive: **tests must not open ports**, only list them.

- [ ] **Step 1: Write the failing tests** (create `src/midi.rs` with the test module; add `pub mod midi;` to `lib.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::Message;

    #[test]
    fn listing_ports_works_with_or_without_devices() {
        let ports = list().expect("listing MIDI ports");
        for (id, name) in &ports {
            assert!(!id.is_empty(), "{name} has no id");
        }
    }

    #[test]
    fn messages_wait_until_drained_in_order() {
        let mut midi = Midi::new();
        let first = Message::Cc { channel: 1, number: 1, value: 3 };
        let second = Message::NoteOn { channel: 1, number: 60 };
        midi.sender.send(first).unwrap();
        midi.sender.send(second).unwrap();
        assert_eq!(midi.drain(), [first, second]);
        assert!(midi.drain().is_empty());
        assert!(midi.devices().is_empty() && midi.failed().is_empty(), "nothing opened");
    }

    #[test]
    fn a_rescan_is_due_at_first_and_then_every_few_seconds() {
        let mut midi = Midi::new();
        let start = Instant::now();
        assert!(midi.rescan_due(start));
        midi.scanned = Some(start);
        assert!(!midi.rescan_due(start + RESCAN / 2));
        assert!(midi.rescan_due(start + RESCAN));
    }
}
```

Run: `cargo test --lib midi` — expected: compile errors.

- [ ] **Step 2: Write `src/midi.rs`**

```rust
//! MIDI input: every input port is open at once, and each message goes to the app over
//! a channel, so neither rendering nor the panel ever waits on a controller. Ports are
//! looked for again every few seconds, since Windows doesn't say when a device is
//! plugged in.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::control::Message;

/// How often the ports are looked for again.
pub const RESCAN: Duration = Duration::from_secs(3);

/// The client name midir gives the system.
const CLIENT: &str = "rasterwarp";

/// An open input port.
struct Open {
    id: String,
    name: String,
    /// Closes the port when dropped.
    _connection: midir::MidiInputConnection<()>,
}

/// The open MIDI inputs and the messages they've sent.
pub struct Midi {
    open: Vec<Open>,
    /// Ports that wouldn't open, and why; tried again at the next rescan.
    failed: Vec<(String, String)>,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    /// When the ports were last looked for.
    scanned: Option<Instant>,
}

impl Default for Midi {
    fn default() -> Self {
        Midi::new()
    }
}

impl Midi {
    /// No ports yet: the first [`Midi::poll`] looks for them.
    pub fn new() -> Midi {
        let (sender, receiver) = mpsc::channel();
        Midi {
            open: Vec::new(),
            failed: Vec::new(),
            sender,
            receiver,
            scanned: None,
        }
    }

    /// The messages that arrived since the last call, oldest first, after looking for
    /// ports again if [`RESCAN`] has passed.
    pub fn poll(&mut self, now: Instant) -> Vec<Message> {
        if self.rescan_due(now) {
            self.rescan(now);
        }
        self.drain()
    }

    /// Opens ports that appeared, drops ones that went, and tries again ones that
    /// failed. Ports already open stay as they are.
    pub fn rescan(&mut self, now: Instant) {
        self.scanned = Some(now);
        self.failed.clear();
        let ports = match list() {
            Ok(ports) => ports,
            Err(err) => {
                self.open.clear();
                self.failed.push(("MIDI".into(), err));
                return;
            }
        };
        self.open.retain(|o| ports.iter().any(|(id, _)| *id == o.id));
        for (id, name) in ports {
            if self.open.iter().any(|o| o.id == id) {
                continue;
            }
            match connect(&id, self.sender.clone()) {
                Ok(connection) => self.open.push(Open {
                    id,
                    name,
                    _connection: connection,
                }),
                Err(err) => {
                    log::warn!("could not open MIDI input {name}: {err}");
                    self.failed.push((name, err));
                }
            }
        }
    }

    /// The open devices' names.
    pub fn devices(&self) -> Vec<String> {
        self.open.iter().map(|o| o.name.clone()).collect()
    }

    /// Ports that wouldn't open at the last rescan, and why.
    pub fn failed(&self) -> &[(String, String)] {
        &self.failed
    }

    fn rescan_due(&self, now: Instant) -> bool {
        self.scanned
            .is_none_or(|then| now.saturating_duration_since(then) >= RESCAN)
    }

    fn drain(&mut self) -> Vec<Message> {
        self.receiver.try_iter().collect()
    }
}

/// The MIDI input ports there are now, as (id, name).
pub fn list() -> Result<Vec<(String, String)>, String> {
    let input = midir::MidiInput::new(CLIENT).map_err(|err| err.to_string())?;
    Ok(input
        .ports()
        .iter()
        .map(|port| {
            let name = input
                .port_name(port)
                .unwrap_or_else(|_| "MIDI device".into());
            (port.id(), name)
        })
        .collect())
}

/// Opens the port with id `id`, sending each message a link can use to `sender`.
fn connect(id: &str, sender: Sender<Message>) -> Result<midir::MidiInputConnection<()>, String> {
    let input = midir::MidiInput::new(CLIENT).map_err(|err| err.to_string())?;
    let port = input
        .find_port_by_id(id)
        .ok_or_else(|| "it was unplugged".to_string())?;
    input
        .connect(
            &port,
            "rasterwarp-in",
            move |_, bytes, _| {
                if let Some(message) = Message::parse(bytes) {
                    let _ = sender.send(message);
                }
            },
            (),
        )
        .map_err(|err| err.to_string())
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test --lib midi` — expected: 3 pass (the listing test passes with or without devices).
Run: `cargo test --lib`, `cargo clippy --all-targets`, `cargo fmt` — clean.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock src/midi.rs src/lib.rs
git commit -m "feat: open every MIDI input and queue its messages"
```

---

### Task 5: Saving links, and the app's MIDI loop

**Files:**
- Modify: `src/save.rs` (`Settings` gains `midi`, test)
- Create: `src/midi_ui.rs` (the `MidiUi` state only; the section comes in Task 7)
- Modify: `src/lib.rs` (`pub mod midi_ui;`)
- Modify: `src/ui.rs` (`UiState` gains `pub midi: MidiUi`)
- Modify: `src/app.rs` (owns a `Midi`; loads and saves links; drains MIDI each refresh; runs actions)

**Interfaces:**
- Consumes: `Links`, `Link`, `Action`, `saved_links` (Tasks 2–3); `Midi` (Task 4).
- Produces:
  - `Settings.midi: Vec<Link>` (saved as `"midi"`)
  - `pub struct crate::midi_ui::MidiUi { pub links: Links, pub devices: Vec<String>, pub failed: Vec<(String, String)> }` (`Default`)
  - `UiState.midi: MidiUi`
  - In `app.rs` (private): `fn handle_midi(&mut self)`, `fn run_action(&mut self, action: Action)`, `fn toggle_loop(&mut self, role: Role)`.

- [ ] **Step 1: Write the failing settings test**

In `src/save.rs`'s `settings_round_trip_and_fall_back_to_defaults`, before `save_settings(&dir, &settings)`, add:

```rust
        settings.midi = vec![crate::control::Link::new(
            crate::control::Source::Cc { channel: 1, number: 1 },
            crate::control::Target::Slider(crate::params::table::SliderId::Zoom),
        )];
```

and add a test that settings without links (an older file) still load:

```rust
    #[test]
    fn settings_without_midi_links_load_with_none() {
        let dir = temp_dir("settings-no-midi");
        fs::write(
            dir.join(SETTINGS_FILE),
            r#"{ "format": "rasterwarp-settings", "version": 1, "show_preview": false }"#,
        )
        .unwrap();
        let settings = load_settings(&dir);
        assert!(!settings.show_preview);
        assert!(settings.midi.is_empty());
    }
```

Run: `cargo test --lib save` — expected: compile error (no field `midi`).

- [ ] **Step 2: Add the field**

In `src/save.rs`:

```rust
/// App settings that belong to no project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub presets_folder: PathBuf,
    pub capture: RecordSettings,
    pub show_preview: bool,
    /// The MIDI links; they belong to the controller, so every project uses them.
    #[serde(with = "crate::control::saved_links")]
    pub midi: Vec<Link>,
}
```

with `midi: Vec::new()` in `Default` and `use crate::control::Link;` at the top.

Run: `cargo test --lib save` — expected: pass.

- [ ] **Step 3: The panel's MIDI state**

Create `src/midi_ui.rs`:

```rust
//! The panel's MIDI section: the devices, the links, and learning a new one.

use crate::control::Links;

/// What the MIDI section shows and edits.
#[derive(Clone, Debug, Default)]
pub struct MidiUi {
    /// The links in use; the app applies them and saves them with the settings.
    pub links: Links,
    /// The open devices' names.
    pub devices: Vec<String>,
    /// Ports that wouldn't open, and why.
    pub failed: Vec<(String, String)>,
}
```

Add `pub mod midi_ui;` to `src/lib.rs`, and to `UiState` in `src/ui.rs`:

```rust
    /// MIDI devices and links.
    pub midi: MidiUi,
```

(with `use crate::midi_ui::MidiUi;`).

- [ ] **Step 4: The app drains MIDI and runs actions**

In `src/app.rs`:

1. Add a field to `State`: `midi: Midi,` (doc: `/// The MIDI inputs; drained once per screen refresh.`), initialised with `midi: Midi::new(),`. Imports: `use crate::control::{Action, Links};`, `use crate::midi::Midi;`, and `Kind` from `crate::inputs` if not imported.
2. At startup, after `ui.capture.settings = settings.capture.clone();` add `ui.midi.links = Links::new(settings.midi.clone());`.
3. In `fn settings(&self)`, add `midi: self.ui.midi.links.list.clone(),`.
4. Replace the F9/F10 toggle block in `input_actions` with a call to a new method, and add the methods:

```rust
        if let Some(role) = actions.toggle_loop {
            self.toggle_loop(role);
        }
```

```rust
    /// Loops `role`'s camera, or goes back to live if it loops (F9, F10). Nothing
    /// happens when the input isn't a camera.
    fn toggle_loop(&mut self, role: Role) {
        let feed = &mut self.feeds[role.index()];
        if feed.kind() != Kind::Camera {
            return;
        }
        if feed.looping().is_some() {
            feed.release_loop();
        } else {
            self.grab_loop(role);
        }
    }

    /// Applies what controllers sent since the last refresh, and runs the actions they
    /// asked for.
    fn handle_midi(&mut self) {
        let messages = self.midi.poll(Instant::now());
        self.ui.midi.devices = self.midi.devices();
        self.ui.midi.failed = self.midi.failed().to_vec();
        for message in messages {
            for action in self.ui.midi.links.handle(message, &mut self.motion) {
                self.run_action(action);
            }
        }
    }

    /// Does what an action's button or key does, and nothing where its button would be
    /// disabled or missing.
    fn run_action(&mut self, action: Action) {
        let sequence = self.motion.mode() == Mode::Sequence;
        match action {
            Action::Transition => self.motion.trigger(),
            Action::Cut => self.motion.cut(),
            Action::SequenceRun if sequence => {
                let seq = self.motion.sequence_mut();
                if seq.is_running() {
                    seq.stop();
                } else {
                    seq.run();
                }
            }
            Action::SequenceReset if sequence => self.motion.sequence_mut().reset(),
            Action::SequenceRun | Action::SequenceReset => {}
            Action::SourceLoop => self.toggle_loop(Role::Source),
            Action::BackgroundLoop => self.toggle_loop(Role::Background),
            Action::Record if self.recorder.is_some() => self.stop_recording(),
            Action::Record => self.start_recording(),
            Action::SaveStill => self.save_still(),
            Action::ClearTrails => self.clear_feedback(),
            Action::Pause => self.ui.paused = !self.ui.paused,
        }
    }
```

(`motion.trigger()` and `motion.cut()` already do nothing outside Transition mode.)

5. In `redraw`, call `self.handle_midi();` right after `self.show_inputs();` (before the panel is drawn, so a slider shows the new value in the same frame).

`Mode` is `crate::motion::Mode`; import it if `app.rs` doesn't already.

- [ ] **Step 5: Build, test, and check by hand**

Run: `cargo build`, `cargo test --lib`, `cargo test --test smoke`, `cargo clippy --all-targets`, `cargo fmt` — all clean.

By hand (optional at this point, there's no learn UI yet): hand-write a link into the settings file of a scratch `APPDATA` (`<APPDATA>/rasterwarp/settings.json`, `"midi": [{ "source": "cc:1:1", "target": "slider:warp.osc1.amplitude" }]`), start the app with that `APPDATA`, move the keyboard's mod wheel: oscillator 1's amplitude slider moves.

- [ ] **Step 6: Commit**

```bash
git add src/save.rs src/midi_ui.rs src/lib.rs src/ui.rs src/app.rs
git commit -m "feat: MIDI links are saved in the settings and drive the panel"
```

---

### Task 6: Linkable sliders and choices in the panel

**Files:**
- Create: `src/link_ui.rs`
- Modify: `src/lib.rs` (`pub mod link_ui;`)
- Modify: `src/ui.rs` (oscillator, deflection, raster, colorize, feedback and glow sections; `draw` passes the links)
- Modify: `src/inputs_ui.rs` (`input_section`/`playback` take `&mut Params` and `&mut Links`)

**Interfaces:**
- Consumes: `SliderId`, `ChoiceId`, `OscSlider`, `VideoSlider` (Task 1); `Links`, `Target`, `Source` (Tasks 2–3); `UiState.midi.links` (Task 5).
- Produces (`crate::link_ui`):
  - `pub fn slider(ui: &mut Ui, links: &mut Links, p: &mut Params, id: SliderId, text: &str) -> Response`
  - `pub fn slider_shown(ui: &mut Ui, links: &mut Links, p: &mut Params, id: SliderId, text: &str, shown: RangeInclusive<f32>, tweak: impl FnOnce(Slider<'_>) -> Slider<'_>) -> Response` — draws over `shown` (a narrower range, e.g. up to the camera buffer), showing the value held to it; only an edit changes it.
  - `pub fn option(ui: &mut Ui, links: &mut Links, p: &mut Params, id: ChoiceId, option: usize, text: impl Into<egui::WidgetText>) -> Response`
  - `pub fn combo(ui: &mut Ui, links: &mut Links, p: &mut Params, id: ChoiceId, labels: &[String])`
  - `pub fn link_menu(response: &Response, links: &mut Links, target: Target)`
  - `input_section(ui, role, state, cameras, params: &mut Params, links: &mut Links, actions)` in `inputs_ui.rs` (was `video: &mut VideoParams`).

- [ ] **Step 1: Write `src/link_ui.rs`**

```rust
//! Drawing linkable controls: sliders and choices from the parameter table, written
//! through its setters (so the panel and MIDI follow the same rules), and the
//! right-click menu that links any control to MIDI or unlinks it.

use std::ops::RangeInclusive;

use egui::{ComboBox, Response, Slider, Ui, WidgetText};

use crate::control::{Links, Source, Target};
use crate::params::Params;
use crate::params::table::{ChoiceId, SliderId};

/// A bank slider over its whole range.
pub fn slider(ui: &mut Ui, links: &mut Links, p: &mut Params, id: SliderId, text: &str) -> Response {
    slider_shown(ui, links, p, id, text, id.range(), |s| s)
}

/// A bank slider drawn over `shown`, which may be narrower than its range (a camera's
/// delay goes up to its buffer). It shows the value held to `shown`, but only an edit
/// changes the value, so drawing it isn't a change. `tweak` adds formatting.
pub fn slider_shown(
    ui: &mut Ui,
    links: &mut Links,
    p: &mut Params,
    id: SliderId,
    text: &str,
    shown: RangeInclusive<f32>,
    tweak: impl FnOnce(Slider<'_>) -> Slider<'_>,
) -> Response {
    let mut value = id.get(p).clamp(*shown.start(), *shown.end());
    let mut slider = Slider::new(&mut value, shown).text(text);
    if id.whole() {
        slider = slider.integer();
    }
    let response = ui.add(tweak(slider));
    if response.changed() {
        id.set(p, value);
    }
    link_menu(&response, links, Target::Slider(id));
    response
}

/// Option `option` of a choice, as a selectable label: clicking picks it.
pub fn option(
    ui: &mut Ui,
    links: &mut Links,
    p: &mut Params,
    id: ChoiceId,
    option: usize,
    text: impl Into<WidgetText>,
) -> Response {
    let response = ui.selectable_label(id.get(p) == option, text);
    if response.clicked() {
        id.set(p, option);
    }
    link_menu(&response, links, Target::Choice(id, option));
    response
}

/// A choice as a drop-down with `labels` for its options. Right-clicking it offers to
/// link any of them.
pub fn combo(ui: &mut Ui, links: &mut Links, p: &mut Params, id: ChoiceId, labels: &[String]) {
    let current = id.get(p);
    let mut picked = None;
    let response = ComboBox::from_id_salt(id)
        .selected_text(labels[current].as_str())
        .show_ui(ui, |ui| {
            for (option, label) in labels.iter().enumerate() {
                if ui.selectable_label(current == option, label).clicked() {
                    picked = Some(option);
                }
            }
        })
        .response;
    if let Some(option) = picked {
        id.set(p, option);
    }
    response.context_menu(|ui| {
        for (option, label) in labels.iter().enumerate() {
            menu_items(ui, links, Target::Choice(id, option), &format!("Link {label} to MIDI"));
        }
    });
}

/// Adds the right-click menu that links `target` to MIDI or unlinks it, and names its
/// links when hovered.
pub fn link_menu(response: &Response, links: &mut Links, target: Target) {
    let linked = sources(links, target);
    if !linked.is_empty() {
        response.clone().on_hover_text(format!("MIDI: {}", names(&linked)));
    }
    response.context_menu(|ui| menu_items(ui, links, target, "Link to MIDI"));
}

/// "Link to MIDI", then "Unlink …" for each control linked to `target`.
fn menu_items(ui: &mut Ui, links: &mut Links, target: Target, text: &str) {
    if ui.button(text).clicked() {
        links.learn(target);
        ui.close();
    }
    for source in sources(links, target) {
        if ui.button(format!("Unlink {}", source.label())).clicked() {
            links.unlink(source, target);
            ui.close();
        }
    }
}

fn sources(links: &Links, target: Target) -> Vec<Source> {
    links.to(target).map(|l| l.source).collect()
}

fn names(sources: &[Source]) -> String {
    sources
        .iter()
        .map(|s| s.label())
        .collect::<Vec<_>>()
        .join(", ")
}
```

`ComboBox::from_id_salt(id)` needs `ChoiceId: Hash` (it derives it). If `ComboBox::from_id_salt` wants `impl Hash` by value, pass `id` (it's `Copy`).

- [ ] **Step 2: Migrate `src/ui.rs`**

`draw` passes `&mut state.midi.links` down. Every section that edits bank sliders or choices gains a `links: &mut Links` parameter. Replace each `ui.add(Slider::new(&mut <field>, ranges::<X>).text("<text>"))` with `link_ui::slider(ui, links, params, SliderId::<Y>, "<text>")`, keeping the panel's text exactly. Concretely:

- `warp_section(ui, params, links)`:
  ```rust
  fn warp_section(ui: &mut Ui, params: &mut Params, links: &mut Links) {
      CollapsingHeader::new("Deflection")
          .default_open(true)
          .show(ui, |ui| {
              for (id, text) in [
                  (SliderId::Zoom, "zoom"),
                  (SliderId::Rotation, "rotation"),
                  (SliderId::OffsetX, "offset x"),
                  (SliderId::OffsetY, "offset y"),
                  (SliderId::Drift, "analog drift"),
                  (SliderId::AxisWander, "axis wander"),
                  (SliderId::LineJitter, "line jitter (px)"),
              ] {
                  link_ui::slider(ui, links, params, id, text);
              }
              ui.checkbox(
                  &mut params.warp.slave_4_to_3,
                  "Slave osc 4 to osc 3 (sine/cosine pair)",
              );
              let slaved = params.warp.slave_4_to_3;
              for i in 0..OSCILLATOR_COUNT {
                  // A slaved oscillator 4 keeps only its own target, amplitude and envelope.
                  let follows = slaved && i == 3;
                  CollapsingHeader::new(format!("Oscillator {}", i + 1))
                      .default_open(i == 0)
                      .show(ui, |ui| {
                          ui.horizontal(|ui| {
                              link_ui::option(ui, links, params, ChoiceId::Axis(i), 0, "→ X");
                              link_ui::option(ui, links, params, ChoiceId::Axis(i), 1, "→ Y");
                              let envelopes = Envelope::ALL.map(|e| format!("{e:?}"));
                              link_ui::combo(ui, links, params, ChoiceId::Envelope(i), &envelopes);
                          });
                          link_ui::slider(
                              ui,
                              links,
                              params,
                              SliderId::Osc(i, OscSlider::Amplitude),
                              "amplitude",
                          );
                          ui.add_enabled_ui(!follows, |ui| oscillator_shape(ui, i, params, links));
                      });
              }
          });
  }
  ```
- `oscillator_shape(ui, i, params, links)`:
  ```rust
  /// The controls a slaved oscillator 4 takes from oscillator 3.
  fn oscillator_shape(ui: &mut Ui, i: usize, params: &mut Params, links: &mut Links) {
      ui.horizontal(|ui| {
          let waveforms = Waveform::ALL.map(|w| format!("{w:?}"));
          link_ui::combo(ui, links, params, ChoiceId::Waveform(i), &waveforms);
          let inputs = OscInput::ALL.map(|input| format!("by {input:?}"));
          link_ui::combo(ui, links, params, ChoiceId::Input(i), &inputs);
          let syncs = OscSync::ALL.map(|sync| format!("{sync:?} sync"));
          link_ui::combo(ui, links, params, ChoiceId::Sync(i), &syncs);
      });
      for (slider, text) in [
          (OscSlider::Frequency, "frequency"),
          (OscSlider::Phase, "phase"),
          (OscSlider::PhaseSpeed, "phase speed"),
          (OscSlider::LfoRate, "LFO rate"),
          (OscSlider::LfoDepth, "LFO depth"),
      ] {
          link_ui::slider(ui, links, params, SliderId::Osc(i, slider), text);
      }
  }
  ```
- `raster_section`: keep the `True raster (scan lines)` checkbox and its hover text; inside `add_enabled_ui`, draw `RasterLines` "lines", `BeamWidth` "beam width (px)", `Compensation` "area compensation", `SpeedCompensation` "speed compensation" through `link_ui::slider`.
- `colorize_section`: keep the Bypass checkbox and the "Thresholds"/"Even" row. The levels slider becomes `link_ui::slider(ui, links, params, SliderId::Levels, "levels");` — **delete** the old `.changed()` block (the setter now evens the thresholds and calls `keep_levels`). The thresholds loop becomes:
  ```rust
  let used = params.colorize.levels as usize - 1;
  for k in 0..used {
      // Shown to 3 decimals but not rounded: a slider that rounds its value changes the
      // look just by being drawn (and it would always look unsaved).
      link_ui::slider_shown(
          ui,
          links,
          params,
          SliderId::LevelFrom(k),
          &format!("level {} from", k + 2),
          ranges::THRESHOLD,
          |s| s.custom_formatter(|v, _| format!("{v:.3}")),
      );
  }
  ```
  (The setter now keeps a threshold between its neighbours.) Then `Softness` "softness", `Fringe` "edge fringe (px)", `Ringing` "ringing", `CycleSpeed` "cycle speed"; the palette colour buttons stay as they are (on `params.colorize.palette`).
- `feedback_section`: `FeedbackAmount` "amount", `FeedbackZoom` "zoom", `FeedbackRotation` "rotation", `FeedbackOffsetX` "offset x", `FeedbackOffsetY` "offset y"; Clear trails stays.
- `glow_section`: `Bloom` "bloom", `BloomThreshold` "bloom threshold", `Scanlines` "scanlines", `ScanlineCount` "scanline count", `ChromaBleed` "chroma bleed", `Noise` "noise".
- `key_section` is unchanged (checkboxes aren't linkable).
- In `draw`, the calls become `warp_section(ui, params, &mut state.midi.links)` etc., where `params` is `motion.editable()` as now. The input sections get `motion.editable()` and `&mut state.midi.links` (see Step 3).

Remove imports that become unused (`Axis`, `Oscillator`, `even_thresholds`, maybe `Slider`), and add `use crate::control::Links;`, `use crate::link_ui;`, `use crate::params::table::{ChoiceId, OscSlider, SliderId};`, `use crate::params::OSCILLATOR_COUNT;`.

- [ ] **Step 3: Migrate `src/inputs_ui.rs`**

`input_section` takes `params: &mut Params, links: &mut Links` instead of `video: &mut VideoParams`, and passes them to `playback`. In `playback`, replace the play-mode row, the speed/position sliders, the delay slider, the between and slit rows and the depth slider:

```rust
    if state.running == Kind::Video || state.looping.is_some() {
        ui.horizontal(|ui| {
            for (option, mode) in PlayMode::ALL.into_iter().enumerate() {
                link_ui::option(ui, links, params, ChoiceId::PlayMode(role), option, mode.label());
            }
        });
        if params.video(role).mode == PlayMode::Scrub {
            link_ui::slider(ui, links, params, SliderId::Video(role, VideoSlider::Position), "position");
        } else {
            link_ui::slider(ui, links, params, SliderId::Video(role, VideoSlider::Speed), "speed")
                .on_hover_text("× the frame rate it was filmed at; negative plays backwards");
        }
    } else {
        // Shown up to the buffer length; a longer delay from a file shows the oldest
        // frame.
        let buffer = state.buffer_seconds.min(*ranges::DELAY.end());
        link_ui::slider_shown(
            ui,
            links,
            params,
            SliderId::Video(role, VideoSlider::Delay),
            "delay (s)",
            0.0..=buffer,
            |s| s,
        );
        // … the "Loop last N s" button and its note stay as they are …
    }
    ui.horizontal(|ui| {
        ui.label("Between frames");
        for (option, between) in Between::ALL.into_iter().enumerate() {
            link_ui::option(ui, links, params, ChoiceId::Between(role), option, between.label());
        }
    });
    ui.horizontal(|ui| {
        ui.label("Slit-scan");
        for (option, slit) in Slit::ALL.into_iter().enumerate() {
            link_ui::option(ui, links, params, ChoiceId::Slit(role), option, slit.label());
        }
    });
    if params.video(role).slit == Slit::Off {
        return;
    }
    let deepest = state
        .max_depth
        .unwrap_or(*ranges::SLIT_DEPTH.end())
        .min(*ranges::SLIT_DEPTH.end());
    // Shown up to the deepest the ring allows; a deeper value from a file is used at
    // that depth.
    link_ui::slider_shown(
        ui,
        links,
        params,
        SliderId::Video(role, VideoSlider::SlitDepth),
        "depth (s)",
        0.0..=deepest,
        |s| s,
    )
    .on_hover_text(format!("Up to {deepest:.1} s fits in the GPU's frame ring"));
    ui.checkbox(&mut params.video_mut(role).slit_flip, "Flip");
    if params.video(role).slit == Slit::Map {
        // … the map image buttons stay as they are …
    }
```

`ui.rs`'s call becomes `inputs_ui::input_section(ui, role, &mut state.inputs[role.index()], state.cameras.as_deref(), motion.editable(), &mut state.midi.links, &mut actions.inputs)`.

- [ ] **Step 4: Build, test, check by hand**

Run: `cargo build`, `cargo test --lib`, `cargo test --test smoke`, `cargo clippy --all-targets`, `cargo fmt` — clean.

By hand (scratch `APPDATA`): every slider and choice still works as before; changing the level count evens the thresholds; a threshold stops at its neighbours; right-clicking a slider shows "Link to MIDI"; choosing it then moving the mod wheel links it (the slider follows the wheel); right-clicking again shows "Unlink CC 1 ch 1"; right-clicking the waveform combo lists "Link Sine to MIDI" … "Link Noise to MIDI"; hovering a linked slider shows "MIDI: CC 1 ch 1".

- [ ] **Step 5: Commit**

```bash
git add src/link_ui.rs src/lib.rs src/ui.rs src/inputs_ui.rs
git commit -m "feat: right-click a slider or choice to link it to MIDI"
```

---

### Task 7: Action links, the MIDI section and learning

**Files:**
- Modify: `src/midi_ui.rs` (the section)
- Modify: `src/ui.rs` (action buttons, mode labels, the duration slider, the learning line, Esc, the section)
- Modify: `src/inputs_ui.rs` (the loop buttons)
- Modify: `src/app.rs` (Refresh)

**Interfaces:**
- Consumes: `link_ui::link_menu` (Task 6); `Links::{learning, cancel, remove}`, `Target`, `Action` (Tasks 2–3); `Midi::rescan` (Task 4); `MidiUi` (Task 5).
- Produces: `pub fn midi_ui::midi_section(ui: &mut Ui, midi: &mut MidiUi) -> bool` (true when Refresh was pressed), `pub fn midi_ui::learning_line(ui: &mut Ui, links: &mut Links)`; `UiActions.refresh_midi: bool`.

- [ ] **Step 1: The MIDI section and learning line**

Append to `src/midi_ui.rs`:

```rust
use egui::{CollapsingHeader, Color32, DragValue, Ui};

/// "Learning *target*: move a wheel or press a key…" with Cancel, while a link is being
/// learned. Esc cancels too (see `ui::draw`).
pub fn learning_line(ui: &mut Ui, links: &mut Links) {
    let Some(target) = links.learning() else {
        return;
    };
    ui.horizontal_wrapped(|ui| {
        ui.colored_label(
            Color32::YELLOW,
            format!("Learning {}: move a wheel or press a key…", target.label()),
        );
        if ui.button("Cancel").on_hover_text("Esc").clicked() {
            links.cancel();
        }
    });
}

/// The MIDI section: the devices, ports that failed, and every link with its range.
/// Returns whether Refresh was pressed.
pub fn midi_section(ui: &mut Ui, midi: &mut MidiUi) -> bool {
    let mut refresh = false;
    CollapsingHeader::new("MIDI").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            if midi.devices.is_empty() {
                ui.label("No MIDI input devices");
            } else {
                ui.label(format!("Devices: {}", midi.devices.join(", ")));
            }
            if ui.button("Refresh").clicked() {
                refresh = true;
            }
        });
        for (name, why) in &midi.failed {
            ui.colored_label(Color32::LIGHT_RED, format!("{name}: {why}"));
        }
        learning_line(ui, &mut midi.links);
        if midi.links.list.is_empty() {
            ui.small("Right-click a slider, option or button and choose Link to MIDI.");
        }
        let mut remove = None;
        egui::Grid::new("midi links").striped(true).show(ui, |ui| {
            for (i, link) in midi.links.list.iter_mut().enumerate() {
                ui.label(link.source.label());
                ui.label(link.target.label());
                match link.target.range() {
                    Some(range) => {
                        ui.horizontal(|ui| {
                            let speed = (range.end() - range.start()) / 200.0;
                            ui.add(DragValue::new(&mut link.min).range(range.clone()).speed(speed))
                                .on_hover_text("The value with the control at the bottom");
                            ui.label("to");
                            ui.add(DragValue::new(&mut link.max).range(range).speed(speed))
                                .on_hover_text("The value with the control at the top");
                        });
                    }
                    None => {
                        ui.label("");
                    }
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            midi.links.remove(i);
        }
    });
    refresh
}
```

- [ ] **Step 2: Wire it into the panel**

In `src/ui.rs`:

1. `UiActions` gains `/// Look for MIDI devices again.` `pub refresh_midi: bool,`.
2. At the top of `draw`, after the shortcuts: Esc cancels learning —
   ```rust
   if state.midi.links.learning().is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
       state.midi.links.cancel();
   }
   ```
3. In the panel, right after the error/note lines, `midi_ui::learning_line(ui, &mut state.midi.links);`.
4. After `canvas_section(...)`: `actions.refresh_midi = midi_ui::midi_section(ui, &mut state.midi);`.
5. Pause: `let pause = ui.checkbox(&mut state.paused, "Pause"); link_ui::link_menu(&pause, &mut state.midi.links, Target::Action(Action::Pause));`
6. `mode_section(ui, motion, links)`: each mode label's response gets `link_ui::link_menu(&response, links, Target::Mode(mode))` (keep `motion.set_mode(mode)` on click). Pass `links` on to `transition_controls` and `sequence_controls`.
7. `transition_controls(ui, motion, links)`: the Transition button → `Target::Action(Action::Transition)`; Cut → `Action::Cut`; the duration slider:
   ```rust
   let duration = ui.add(
       Slider::new(&mut motion.ab.duration, DURATION)
           .text("duration (s)")
           .logarithmic(true),
   );
   link_ui::link_menu(&duration, links, Target::Duration);
   ```
8. `sequence_controls(ui, motion, links)`: the Run and the Stop button → `Action::SequenceRun`; Reset → `Action::SequenceReset`. (`let curves = motion.curves.clone(); let seq = motion.sequence_mut();` already splits the borrows; `links` is a separate borrow.)
9. `capture_section(ui, capture, links, actions)`: Record and Stop recording → `Action::Record`; Save still → `Action::SaveStill`.
10. `feedback_section`: Clear trails → `Action::ClearTrails`.

Pattern for a button:

```rust
let transition = ui.button(label);
if transition.clicked() {
    motion.trigger();
}
link_ui::link_menu(&transition, links, Target::Action(Action::Transition));
```

In `src/inputs_ui.rs` `playback`: the "Loop last N s" button and the "Back to live" button each get `link_ui::link_menu(&response, links, Target::Action(loop_action))` where

```rust
let loop_action = match role {
    Role::Source => Action::SourceLoop,
    Role::Background => Action::BackgroundLoop,
};
```

(Keep their `.clicked()` handling and hover texts.)

In `src/app.rs`, after `self.input_actions(actions.inputs);`:

```rust
        if actions.refresh_midi {
            self.midi.rescan(Instant::now());
        }
```

- [ ] **Step 3: Build and test**

Run: `cargo build`, `cargo test --lib`, `cargo test --test smoke`, `cargo test --test capture`, `cargo test --test video`, `cargo clippy --all-targets`, `cargo fmt` — clean.

- [ ] **Step 4: Check by hand with the keyboard (Q25)**

With a scratch `APPDATA`:
1. The MIDI section lists "Q25" (and "theLooper"); Refresh works; with the keyboard unplugged it disappears within 3 s and comes back when replugged, links intact.
2. Right-click oscillator 1's amplitude → Link to MIDI: the learning line appears at the top; a key press is ignored; the mod wheel links it; the slider follows the wheel; the MIDI section lists "CC 1 ch 1 · Osc 1 amplitude · 0 to 0.5".
3. Link the pitch wheel to zoom, set the range to 0.8–1.2 in the MIDI section: the wheel bends zoom around 1.0 and springs back to it.
4. Link keys to Transition (in Transition mode), Cut, the camera loop (with a camera running), Pause, and the waveform Square option; each key does what its button does; Cut outside Transition mode does nothing.
5. Esc cancels learning; "Unlink" removes a link; Remove in the MIDI section removes one.
6. Quit and restart: the links are still there.

- [ ] **Step 5: Commit**

```bash
git add src/midi_ui.rs src/ui.rs src/inputs_ui.rs src/app.rs
git commit -m "feat: link actions, a MIDI section, and learning from the panel"
```

---

## Self-review (done while writing)

- **Spec coverage:** linkable sliders (Task 1 table + Task 6), choices (Tasks 1, 6; mode in Task 7), actions (Tasks 2, 5, 7), where values go (Task 3 `handle` → `editable()`; Task 5 handles before drawing, so collapsed/hidden panels still work), table ids/labels/rules (Task 1), links/matching/values/notes (Tasks 2–3), MIDI input/rescan (Task 4), learning and the panel (Tasks 6–7), saving (Tasks 2, 5), errors (Task 4 `failed`, Task 7 shows them; startup never blocks: `Midi::new` opens nothing), code structure (all files), testing (each task; by hand in Task 7). Unsaved marking: the project compares `Params`, so MIDI edits to the edited bank mark it unsaved with no extra code.
- **Types:** `SliderId`, `ChoiceId`, `Target`, `Source`, `Message`, `Link`, `Links`, `Action`, `Midi`, `MidiUi` are named identically in every task.
