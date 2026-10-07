# Rasterwarp Plan 1 (Motion) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Live / Transition (A/B) / Sequence modes with drawn transition curves, plus the oscillator extras (Frame/Free sync, sine/cosine slave, Swell envelope), on top of the prototype now on `main`.

**Architecture:**
- New pure-logic modules, all GPU-free and unit-tested: `curve` (built-in and drawn curves), `transition` (A/B banks and ramp), `sequence` (5 cues on a 24 fps frame clock), `blend` (`FrameParams` + phase `Clocks`), and `motion` (ties the modes together).
- The renderer switches from `&Params` to `&FrameParams`. The warp shader runs up to 8 weighted oscillator slots whose phases are computed on the CPU.
- The egui panel gains a mode selector, transition controls, sequence controls and a curve editor. Space triggers a transition.

**Tech Stack:** Rust 1.98 (edition 2024), wgpu 30, winit 0.30, egui 0.36. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-07-rasterwarp-motion-output-look-design.md` (Plan 1 section, plus Error handling and Testing)

## Global Constraints

- Platform: Windows 10, NVIDIA RTX 3090, Rust 1.98 stable MSVC, edition 2024.
- No new crates. Versions stay pinned: wgpu 30, winit 0.30, egui 0.36 (see `Cargo.toml`).
- `cargo build`, `cargo test` and `cargo clippy --all-targets` must have zero warnings. Run `cargo fmt` before committing.
- Every commit message ends with the trailer line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Don't use the word "Scanimate" in user-facing text.
- Code in this plan was compiled, tested (77 unit tests + GPU smoke test) and run on the dev machine before the plan was written. Transcribe it exactly. Every task's end state was also built and tested on its own.

## Design decisions made while building (beyond the spec text)

1. **Running phase clocks (`blend::Clocks`).** Oscillator phase, LFO phase and palette cycling advance by *rate × dt each frame* instead of being computed as *rate × total time*. Blending two rates over a transition then changes speed smoothly, like a real voltage-controlled oscillator, instead of sweeping through rate × elapsed-time cycles. The shader receives final phases and no longer multiplies by time (drift noise still uses time).
2. **Two clock sets in `Motion`.** `rest` belongs to what's shown at rest, or a ramp's origin. `target` belongs to a ramp's destination.
   - When a ramp starts, `target` is copied from `rest`.
   - When it finishes, `rest` is copied from `target`.
   - So phases are continuous at both ends (tested).
3. **Oscillators are matched by shape**: waveform, target, input, sync and envelope all equal.
   - Matched oscillators sweep their numbers in one slot.
   - Unmatched ones crossfade in two slots, weighted (1−t) and t, with the weight folded into the amplitude.
   - Drift noise is seeded by oscillator index, so it doesn't jump when slots appear.
4. **Reversing a transition** runs the same ramp backwards (`forward = false`), so any curve, even an asymmetric drawn one, reverses without a jump.
5. **Sequence:**
   - It is created on the *first* entry into Sequence mode and kept afterwards, so switching modes doesn't wipe cues.
   - **Stop** halts playback and the output returns to the selected cue. **Run** always starts from cue 1 at frame 0.
6. **Leaving Transition mode** cancels a running ramp, keeping the on-air bank. **Leaving Sequence mode** puts the shown cue on air.

## File Structure

| File | Responsibility |
|---|---|
| `src/params.rs` (modify) | Adds `OscSync`, `Envelope`, `Oscillator.sync/envelope`, `WarpParams.slave_4_to_3`, `WarpParams::effective_oscillators()` |
| `src/curve.rs` (new) | `CurveRef`, `CustomCurve`, `CurveLibrary`, `monotone_cubic` |
| `src/transition.rs` (new) | `AbState` A/B banks, `Ramp`, `AbEvent`, `DURATION` |
| `src/sequence.rs` (new) | `Sequence`, `Cue`, `SeqEvent`, `SeqView`, frame clock |
| `src/blend.rs` (new) | `Clocks`, `FrameParams` (`WarpFrame`, `OscSlot`, `ColorizeFrame`), `blend()` |
| `src/motion.rs` (new) | `Mode`, `Motion`: editable params per mode, trigger/cut, advance, frame |
| `src/curve_editor.rs` (new) | egui curve-editor widget + screen mapping helpers |
| `shaders/warp.wgsl`, `src/passes/warp.rs` (modify) | 8 weighted oscillator slots; CPU-computed phases |
| `src/passes/colorize.rs`, `src/passes/mod.rs`, `tests/smoke.rs` (modify) | Renderer consumes `FrameParams` |
| `src/ui.rs`, `src/app.rs` (modify) | Mode/transition/sequence/curve UI; `Motion` in the app; Space key |

---

### Task 1: Oscillator extras in Params

**Files:**
- Modify: `src/params.rs`
- Test: unit tests in `src/params.rs`

**Interfaces:**
- Consumes: existing `Params`, `Oscillator`, `WarpParams`.
- Produces:
  - `pub enum OscSync { Free, Frame }` with `ALL`;
  - `pub enum Envelope { Constant, Swell }` with `ALL`;
  - new fields `Oscillator.sync: OscSync`, `Oscillator.envelope: Envelope`, `WarpParams.slave_4_to_3: bool`, defaulting to Free / Constant / false;
  - `WarpParams::effective_oscillators(&self) -> [(Oscillator, usize); OSCILLATOR_COUNT]`. The `usize` is the phase-clock index: 2 for a slaved oscillator 4, otherwise its own index.

- [ ] **Step 1: Write the failing tests.** Add these two tests inside `mod tests` in `src/params.rs`, just before `srgb_to_linear_known_values`:

```rust
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
```

- [ ] **Step 2: Run them and confirm they fail.** Run: `cargo test --lib params::`. Expected: compile errors such as "no field `slave_4_to_3` on type `WarpParams`" and "no method named `effective_oscillators`".

- [ ] **Step 3: Implement.** Replace `src/params.rs` with:

```rust
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
}

pub const OSCILLATOR_COUNT: usize = 4;
pub const PALETTE_SIZE: usize = 8;

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
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeedbackParams {
    /// How much of the previous frame survives each frame. 0 = no trails.
    pub amount: f32,
    /// Per-frame transform applied to the previous frame.
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
```

- [ ] **Step 4: Run the tests and confirm they pass.** Run: `cargo test --lib params::`. Expected: `5 passed`. Then `cargo build` (the app still builds; the new fields aren't used yet).

- [ ] **Step 5: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/params.rs
git commit -m "feat: add oscillator sync, envelope and sine/cosine slave params" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Transition curves

**Files:**
- Create: `src/curve.rs`
- Modify: `src/lib.rs`
- Test: unit tests in `src/curve.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `CurveRef {Linear, SCurve, Custom(u32)}`; `CustomCurve` (`new(id, name)`, `points()`, `all_points()`, `insert(x, y) -> Option<usize>`, `remove(i)`, `move_point(i, x, y)`, `eval(x)`, pub `name`, `id`); `CurveLibrary` (`add() -> Option<u32>`, `remove(id)`, `get(id)`, `get_mut(id)`, `eval(CurveRef, x) -> f32`, `name(CurveRef) -> String`, `choices() -> Vec<CurveRef>`, pub `custom: Vec<CustomCurve>`); consts `Y_RANGE` (-0.5..=1.5), `MIN_GAP` (0.01), `MAX_CUSTOM` (8); `monotone_cubic(&[[f32;2]], x) -> f32`.

> Note: curves are evaluated with Fritsch–Carlson monotone cubic interpolation. It never overshoots between neighbouring points, so overshoot happens only where a point is placed outside [0, 1].

- [ ] **Step 1: Register the module.** Replace `src/lib.rs` with:

```rust
//! Rasterwarp: real-time analog video-synthesis animation.

pub mod app;
pub mod curve;
pub mod gpu;
pub mod params;
pub mod passes;
pub mod source;
pub mod ui;
```

- [ ] **Step 2: Write the failing tests.** Create `src/curve.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_curves() {
        let lib = CurveLibrary::default();
        assert_eq!(lib.eval(CurveRef::Linear, 0.25), 0.25);
        assert!((lib.eval(CurveRef::SCurve, 0.5) - 0.5).abs() < 1e-6);
        assert!(lib.eval(CurveRef::SCurve, 0.1) < 0.1);
        assert!(lib.eval(CurveRef::SCurve, 0.9) > 0.9);
    }

    #[test]
    fn endpoints_are_exact() {
        let mut lib = CurveLibrary::default();
        let id = lib.add().unwrap();
        lib.get_mut(id).unwrap().move_point(0, 0.3, 0.9);
        for curve in lib.choices() {
            assert_eq!(lib.eval(curve, 0.0), 0.0, "{curve:?}");
            assert!((lib.eval(curve, 1.0) - 1.0).abs() < 1e-6, "{curve:?}");
        }
    }

    #[test]
    fn progress_is_clamped() {
        let lib = CurveLibrary::default();
        assert_eq!(lib.eval(CurveRef::Linear, -1.0), 0.0);
        assert_eq!(lib.eval(CurveRef::Linear, 2.0), 1.0);
    }

    #[test]
    fn monotone_points_never_overshoot() {
        let pts = [[0.0, 0.0], [0.2, 0.8], [0.5, 0.9], [1.0, 1.0]];
        let mut prev = 0.0;
        for i in 0..=100 {
            let y = monotone_cubic(&pts, i as f32 / 100.0);
            assert!((0.0..=1.0 + 1e-6).contains(&y), "y = {y}");
            assert!(y >= prev - 1e-6, "not monotone at {i}");
            prev = y;
        }
    }

    #[test]
    fn points_outside_unit_range_overshoot() {
        let mut c = CustomCurve::new(0, "bounce".into());
        c.move_point(0, 0.7, 1.3);
        assert!((c.eval(0.7) - 1.3).abs() < 1e-5);
        assert!(c.eval(0.8) > 1.0);
    }

    #[test]
    fn points_stay_sorted_and_clamped() {
        let mut c = CustomCurve::new(0, "c".into());
        assert_eq!(c.insert(0.2, 0.1), Some(0));
        assert_eq!(c.insert(0.8, 3.0), Some(2));
        assert_eq!(c.points()[2], [0.8, 1.5]);
        assert_eq!(c.insert(0.505, 0.0), None, "too close to 0.5");
        assert_eq!(c.insert(0.0, 0.0), None, "endpoint");
        // Dragging a point past its neighbour stops short of it.
        c.move_point(1, 0.95, 0.5);
        assert!((c.points()[1][0] - (0.8 - MIN_GAP)).abs() < 1e-6);
        c.remove(0);
        assert_eq!(c.points().len(), 2);
    }

    #[test]
    fn deleted_curve_falls_back_to_linear() {
        let mut lib = CurveLibrary::default();
        let id = lib.add().unwrap();
        lib.remove(id);
        assert_eq!(lib.eval(CurveRef::Custom(id), 0.3), 0.3);
        assert!(lib.choices().iter().all(|c| *c != CurveRef::Custom(id)));
    }

    #[test]
    fn library_is_capped() {
        let mut lib = CurveLibrary::default();
        for _ in 0..MAX_CUSTOM {
            assert!(lib.add().is_some());
        }
        assert!(lib.add().is_none());
    }
}
```

- [ ] **Step 3: Run them and confirm they fail.** Run: `cargo test --lib curve::`. Expected: compile errors such as "cannot find type `CurveLibrary`".

- [ ] **Step 4: Implement.** Replace `src/curve.rs` with:

```rust
//! Transition curves: built-in Linear and S-curve, plus user-drawn curves.

use std::ops::RangeInclusive;

/// Allowed y range for user curve points (overshoot and anticipation).
pub const Y_RANGE: RangeInclusive<f32> = -0.5..=1.5;
/// Minimum x gap kept between neighbouring points.
pub const MIN_GAP: f32 = 0.01;
/// Maximum number of user curves in the library.
pub const MAX_CUSTOM: usize = 8;

/// Which curve a transition or cue uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveRef {
    Linear,
    SCurve,
    /// A user curve, by its stable id in the library.
    Custom(u32),
}

/// A user-drawn curve through (0,0), its interior points, and (1,1).
#[derive(Clone, Debug, PartialEq)]
pub struct CustomCurve {
    pub id: u32,
    pub name: String,
    /// Interior points, sorted by x, each x strictly inside (0, 1).
    points: Vec<[f32; 2]>,
}

impl CustomCurve {
    pub fn new(id: u32, name: String) -> Self {
        Self {
            id,
            name,
            points: vec![[0.5, 0.5]],
        }
    }

    pub fn points(&self) -> &[[f32; 2]] {
        &self.points
    }

    /// All points including the fixed endpoints.
    pub fn all_points(&self) -> Vec<[f32; 2]> {
        let mut all = Vec::with_capacity(self.points.len() + 2);
        all.push([0.0, 0.0]);
        all.extend_from_slice(&self.points);
        all.push([1.0, 1.0]);
        all
    }

    /// Inserts a point, keeping x order. Returns its index, or `None` if it would
    /// sit too close to an existing point.
    pub fn insert(&mut self, x: f32, y: f32) -> Option<usize> {
        if x < MIN_GAP || x > 1.0 - MIN_GAP {
            return None;
        }
        if self.points.iter().any(|p| (p[0] - x).abs() < MIN_GAP) {
            return None;
        }
        let i = self.points.partition_point(|p| p[0] < x);
        self.points
            .insert(i, [x, y.clamp(*Y_RANGE.start(), *Y_RANGE.end())]);
        Some(i)
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.points.len() {
            self.points.remove(i);
        }
    }

    /// Moves point `i`, keeping it between its neighbours and inside the y range.
    pub fn move_point(&mut self, i: usize, x: f32, y: f32) {
        let lo = if i == 0 { 0.0 } else { self.points[i - 1][0] } + MIN_GAP;
        let hi = if i + 1 == self.points.len() {
            1.0
        } else {
            self.points[i + 1][0]
        } - MIN_GAP;
        self.points[i] = [
            x.clamp(lo, hi.max(lo)),
            y.clamp(*Y_RANGE.start(), *Y_RANGE.end()),
        ];
    }

    pub fn eval(&self, x: f32) -> f32 {
        monotone_cubic(&self.all_points(), x)
    }
}

/// Built-in curves plus the user's curves.
#[derive(Clone, Debug, Default)]
pub struct CurveLibrary {
    pub custom: Vec<CustomCurve>,
    next_id: u32,
}

impl CurveLibrary {
    /// Adds a new user curve. Returns its id, or `None` when the library is full.
    pub fn add(&mut self) -> Option<u32> {
        if self.custom.len() >= MAX_CUSTOM {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.custom
            .push(CustomCurve::new(id, format!("Curve {}", id + 1)));
        Some(id)
    }

    pub fn remove(&mut self, id: u32) {
        self.custom.retain(|c| c.id != id);
    }

    pub fn get(&self, id: u32) -> Option<&CustomCurve> {
        self.custom.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut CustomCurve> {
        self.custom.iter_mut().find(|c| c.id == id)
    }

    /// Curve value at progress `x` in [0, 1]. A deleted user curve falls back to Linear.
    pub fn eval(&self, curve: CurveRef, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        match curve {
            CurveRef::Linear => x,
            CurveRef::SCurve => 0.5 - 0.5 * (std::f32::consts::PI * x).cos(),
            CurveRef::Custom(id) => self.get(id).map_or(x, |c| c.eval(x)),
        }
    }

    pub fn name(&self, curve: CurveRef) -> String {
        match curve {
            CurveRef::Linear => "Linear".into(),
            CurveRef::SCurve => "S-curve".into(),
            CurveRef::Custom(id) => self
                .get(id)
                .map_or_else(|| "Linear (deleted curve)".into(), |c| c.name.clone()),
        }
    }

    /// Every selectable curve, built-ins first.
    pub fn choices(&self) -> Vec<CurveRef> {
        let mut all = vec![CurveRef::Linear, CurveRef::SCurve];
        all.extend(self.custom.iter().map(|c| CurveRef::Custom(c.id)));
        all
    }
}

/// Monotone cubic Hermite interpolation (Fritsch–Carlson) through `pts`, sorted by x.
/// It never overshoots between neighbouring points.
pub fn monotone_cubic(pts: &[[f32; 2]], x: f32) -> f32 {
    let n = pts.len();
    if n < 2 {
        return x;
    }
    let d: Vec<f32> = pts
        .windows(2)
        .map(|w| (w[1][1] - w[0][1]) / (w[1][0] - w[0][0]))
        .collect();
    let mut m = vec![0.0f32; n];
    m[0] = d[0];
    m[n - 1] = d[n - 2];
    for k in 1..n - 1 {
        m[k] = if d[k - 1] * d[k] <= 0.0 {
            0.0
        } else {
            (d[k - 1] + d[k]) / 2.0
        };
    }
    for k in 0..n - 1 {
        if d[k] == 0.0 {
            m[k] = 0.0;
            m[k + 1] = 0.0;
        } else {
            let a = m[k] / d[k];
            let b = m[k + 1] / d[k];
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                m[k] = t * a * d[k];
                m[k + 1] = t * b * d[k];
            }
        }
    }
    let x = x.clamp(pts[0][0], pts[n - 1][0]);
    let k = pts.partition_point(|p| p[0] <= x).clamp(1, n - 1) - 1;
    let (x0, y0) = (pts[k][0], pts[k][1]);
    let (x1, y1) = (pts[k + 1][0], pts[k + 1][1]);
    let h = x1 - x0;
    let t = (x - x0) / h;
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * y0
        + (t3 - 2.0 * t2 + t) * h * m[k]
        + (-2.0 * t3 + 3.0 * t2) * y1
        + (t3 - t2) * h * m[k + 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_curves() {
        let lib = CurveLibrary::default();
        assert_eq!(lib.eval(CurveRef::Linear, 0.25), 0.25);
        assert!((lib.eval(CurveRef::SCurve, 0.5) - 0.5).abs() < 1e-6);
        assert!(lib.eval(CurveRef::SCurve, 0.1) < 0.1);
        assert!(lib.eval(CurveRef::SCurve, 0.9) > 0.9);
    }

    #[test]
    fn endpoints_are_exact() {
        let mut lib = CurveLibrary::default();
        let id = lib.add().unwrap();
        lib.get_mut(id).unwrap().move_point(0, 0.3, 0.9);
        for curve in lib.choices() {
            assert_eq!(lib.eval(curve, 0.0), 0.0, "{curve:?}");
            assert!((lib.eval(curve, 1.0) - 1.0).abs() < 1e-6, "{curve:?}");
        }
    }

    #[test]
    fn progress_is_clamped() {
        let lib = CurveLibrary::default();
        assert_eq!(lib.eval(CurveRef::Linear, -1.0), 0.0);
        assert_eq!(lib.eval(CurveRef::Linear, 2.0), 1.0);
    }

    #[test]
    fn monotone_points_never_overshoot() {
        let pts = [[0.0, 0.0], [0.2, 0.8], [0.5, 0.9], [1.0, 1.0]];
        let mut prev = 0.0;
        for i in 0..=100 {
            let y = monotone_cubic(&pts, i as f32 / 100.0);
            assert!((0.0..=1.0 + 1e-6).contains(&y), "y = {y}");
            assert!(y >= prev - 1e-6, "not monotone at {i}");
            prev = y;
        }
    }

    #[test]
    fn points_outside_unit_range_overshoot() {
        let mut c = CustomCurve::new(0, "bounce".into());
        c.move_point(0, 0.7, 1.3);
        assert!((c.eval(0.7) - 1.3).abs() < 1e-5);
        assert!(c.eval(0.8) > 1.0);
    }

    #[test]
    fn points_stay_sorted_and_clamped() {
        let mut c = CustomCurve::new(0, "c".into());
        assert_eq!(c.insert(0.2, 0.1), Some(0));
        assert_eq!(c.insert(0.8, 3.0), Some(2));
        assert_eq!(c.points()[2], [0.8, 1.5]);
        assert_eq!(c.insert(0.505, 0.0), None, "too close to 0.5");
        assert_eq!(c.insert(0.0, 0.0), None, "endpoint");
        // Dragging a point past its neighbour stops short of it.
        c.move_point(1, 0.95, 0.5);
        assert!((c.points()[1][0] - (0.8 - MIN_GAP)).abs() < 1e-6);
        c.remove(0);
        assert_eq!(c.points().len(), 2);
    }

    #[test]
    fn deleted_curve_falls_back_to_linear() {
        let mut lib = CurveLibrary::default();
        let id = lib.add().unwrap();
        lib.remove(id);
        assert_eq!(lib.eval(CurveRef::Custom(id), 0.3), 0.3);
        assert!(lib.choices().iter().all(|c| *c != CurveRef::Custom(id)));
    }

    #[test]
    fn library_is_capped() {
        let mut lib = CurveLibrary::default();
        for _ in 0..MAX_CUSTOM {
            assert!(lib.add().is_some());
        }
        assert!(lib.add().is_none());
    }
}
```

- [ ] **Step 5: Run the tests and confirm they pass.** Run: `cargo test --lib curve::`. Expected: `8 passed`.

- [ ] **Step 6: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/curve.rs src/lib.rs
git commit -m "feat: add built-in and user-drawn transition curves" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: A/B transition state

**Files:**
- Create: `src/transition.rs`
- Modify: `src/lib.rs`
- Test: unit tests in `src/transition.rs`

**Interfaces:**
- Consumes: `Params` (Task 1), `CurveRef` (Task 2).
- Produces: `DURATION` (0.1..=30.0 s); `Ramp {progress, forward}`; `AbEvent {Finished, Reverted}`; `AbState` (pub `banks: [Params; 2]`, pub `on_air`, pub `duration`, pub `curve`; `new(Params)`, `off_air()`, `bank_name(i)`, `sync_off_air()`, `ramp() -> Option<Ramp>`, `trigger() -> bool` (true = a new ramp started), `cut()`, `cancel()`, `advance(dt) -> Option<AbEvent>`).

- [ ] **Step 1: Register the module.** Replace `src/lib.rs` with:

```rust
//! Rasterwarp: real-time analog video-synthesis animation.

pub mod app;
pub mod curve;
pub mod gpu;
pub mod params;
pub mod passes;
pub mod source;
pub mod transition;
pub mod ui;
```

- [ ] **Step 2: Write the failing tests.** Create `src/transition.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AbState {
        let mut s = AbState::new(Params::default());
        s.banks[1].warp.zoom = 2.0;
        s.duration = 2.0;
        s
    }

    #[test]
    fn sync_copies_on_air_into_off_air() {
        let mut s = state();
        s.sync_off_air();
        assert_eq!(s.banks[0], s.banks[1]);
    }

    #[test]
    fn ramp_finishes_and_swaps_banks() {
        let mut s = state();
        assert!(s.trigger());
        assert_eq!(s.advance(1.0), None);
        assert_eq!(s.ramp().unwrap().progress, 0.5);
        assert_eq!(s.advance(1.0), Some(AbEvent::Finished));
        assert_eq!(s.on_air, 1);
        assert_eq!(s.off_air(), 0);
        assert!(s.ramp().is_none());
    }

    #[test]
    fn trigger_mid_ramp_reverses() {
        let mut s = state();
        s.trigger();
        s.advance(1.5);
        assert!(!s.trigger(), "reversing does not start a new ramp");
        s.advance(1.0);
        assert!((s.ramp().unwrap().progress - 0.25).abs() < 1e-6);
        assert_eq!(s.advance(1.0), Some(AbEvent::Reverted));
        assert_eq!(s.on_air, 0, "back where it started");
    }

    #[test]
    fn cut_without_ramp_swaps() {
        let mut s = state();
        s.cut();
        assert_eq!(s.on_air, 1);
    }

    #[test]
    fn cut_during_forward_ramp_lands_on_destination() {
        let mut s = state();
        s.trigger();
        s.advance(0.5);
        s.cut();
        assert_eq!(s.on_air, 1);
        assert!(s.ramp().is_none());
    }

    #[test]
    fn cut_during_reversed_ramp_returns_to_origin() {
        let mut s = state();
        s.trigger();
        s.advance(0.5);
        s.trigger();
        s.cut();
        assert_eq!(s.on_air, 0);
        assert!(s.ramp().is_none());
    }

    #[test]
    fn cancel_keeps_on_air_bank() {
        let mut s = state();
        s.trigger();
        s.advance(1.0);
        s.cancel();
        assert_eq!(s.on_air, 0);
        assert!(s.ramp().is_none());
    }

    #[test]
    fn zero_dt_does_not_advance() {
        let mut s = state();
        s.trigger();
        assert_eq!(s.advance(0.0), None);
        assert_eq!(s.ramp().unwrap().progress, 0.0);
    }
}
```

- [ ] **Step 3: Run them and confirm they fail.** Run: `cargo test --lib transition::`. Expected: compile errors such as "cannot find type `AbState`".

- [ ] **Step 4: Implement.** Replace `src/transition.rs` with:

```rust
//! A/B transition mode: two parameter banks, one on air, and a ramp between them.

use std::ops::RangeInclusive;

use crate::curve::CurveRef;
use crate::params::Params;

/// Transition duration range in seconds.
pub const DURATION: RangeInclusive<f32> = 0.1..=30.0;

/// A running ramp from the on-air bank toward the off-air bank.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ramp {
    /// Linear progress, 0 = on-air bank, 1 = off-air bank.
    pub progress: f32,
    /// False after the ramp has been reversed (it is heading back to the on-air bank).
    pub forward: bool,
}

/// What a call to `advance` finished, if anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbEvent {
    /// Reached the off-air bank, which is now on air.
    Finished,
    /// Reversed all the way back to the on-air bank.
    Reverted,
}

#[derive(Clone, Debug)]
pub struct AbState {
    pub banks: [Params; 2],
    pub on_air: usize,
    /// Seconds for a full ramp.
    pub duration: f32,
    pub curve: CurveRef,
    ramp: Option<Ramp>,
}

impl AbState {
    pub fn new(params: Params) -> Self {
        Self {
            banks: [params, params],
            on_air: 0,
            duration: 2.0,
            curve: CurveRef::SCurve,
            ramp: None,
        }
    }

    pub fn off_air(&self) -> usize {
        1 - self.on_air
    }

    /// Bank label for the UI.
    pub fn bank_name(index: usize) -> &'static str {
        ["A", "B"][index]
    }

    /// Makes the off-air bank a copy of the on-air bank (on entering Transition mode).
    pub fn sync_off_air(&mut self) {
        self.banks[self.off_air()] = self.banks[self.on_air];
    }

    pub fn ramp(&self) -> Option<Ramp> {
        self.ramp
    }

    /// Starts a ramp, or reverses the running one. Returns true when a new ramp started.
    pub fn trigger(&mut self) -> bool {
        match &mut self.ramp {
            Some(ramp) => {
                ramp.forward = !ramp.forward;
                false
            }
            None => {
                self.ramp = Some(Ramp {
                    progress: 0.0,
                    forward: true,
                });
                true
            }
        }
    }

    /// Jumps straight to the destination: the bank the ramp is heading to, or the
    /// off-air bank when no ramp is running.
    pub fn cut(&mut self) {
        match self.ramp.take() {
            Some(ramp) if !ramp.forward => {}
            _ => self.on_air = self.off_air(),
        }
    }

    /// Abandons a running ramp; the on-air bank stays on air.
    pub fn cancel(&mut self) {
        self.ramp = None;
    }

    pub fn advance(&mut self, dt: f32) -> Option<AbEvent> {
        let ramp = self.ramp.as_mut()?;
        let step = dt / self.duration.clamp(*DURATION.start(), *DURATION.end());
        if ramp.forward {
            ramp.progress += step;
            if ramp.progress >= 1.0 {
                self.ramp = None;
                self.on_air = self.off_air();
                return Some(AbEvent::Finished);
            }
        } else {
            ramp.progress -= step;
            if ramp.progress <= 0.0 {
                self.ramp = None;
                return Some(AbEvent::Reverted);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AbState {
        let mut s = AbState::new(Params::default());
        s.banks[1].warp.zoom = 2.0;
        s.duration = 2.0;
        s
    }

    #[test]
    fn sync_copies_on_air_into_off_air() {
        let mut s = state();
        s.sync_off_air();
        assert_eq!(s.banks[0], s.banks[1]);
    }

    #[test]
    fn ramp_finishes_and_swaps_banks() {
        let mut s = state();
        assert!(s.trigger());
        assert_eq!(s.advance(1.0), None);
        assert_eq!(s.ramp().unwrap().progress, 0.5);
        assert_eq!(s.advance(1.0), Some(AbEvent::Finished));
        assert_eq!(s.on_air, 1);
        assert_eq!(s.off_air(), 0);
        assert!(s.ramp().is_none());
    }

    #[test]
    fn trigger_mid_ramp_reverses() {
        let mut s = state();
        s.trigger();
        s.advance(1.5);
        assert!(!s.trigger(), "reversing does not start a new ramp");
        s.advance(1.0);
        assert!((s.ramp().unwrap().progress - 0.25).abs() < 1e-6);
        assert_eq!(s.advance(1.0), Some(AbEvent::Reverted));
        assert_eq!(s.on_air, 0, "back where it started");
    }

    #[test]
    fn cut_without_ramp_swaps() {
        let mut s = state();
        s.cut();
        assert_eq!(s.on_air, 1);
    }

    #[test]
    fn cut_during_forward_ramp_lands_on_destination() {
        let mut s = state();
        s.trigger();
        s.advance(0.5);
        s.cut();
        assert_eq!(s.on_air, 1);
        assert!(s.ramp().is_none());
    }

    #[test]
    fn cut_during_reversed_ramp_returns_to_origin() {
        let mut s = state();
        s.trigger();
        s.advance(0.5);
        s.trigger();
        s.cut();
        assert_eq!(s.on_air, 0);
        assert!(s.ramp().is_none());
    }

    #[test]
    fn cancel_keeps_on_air_bank() {
        let mut s = state();
        s.trigger();
        s.advance(1.0);
        s.cancel();
        assert_eq!(s.on_air, 0);
        assert!(s.ramp().is_none());
    }

    #[test]
    fn zero_dt_does_not_advance() {
        let mut s = state();
        s.trigger();
        assert_eq!(s.advance(0.0), None);
        assert_eq!(s.ramp().unwrap().progress, 0.0);
    }
}
```

- [ ] **Step 5: Run the tests and confirm they pass.** Run: `cargo test --lib transition::`. Expected: `8 passed`.

- [ ] **Step 6: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/transition.rs src/lib.rs
git commit -m "feat: add A/B transition state machine" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Sequence cues

**Files:**
- Create: `src/sequence.rs`
- Modify: `src/lib.rs`
- Test: unit tests in `src/sequence.rs`

**Interfaces:**
- Consumes: `Params` (Task 1), `CurveRef` (Task 2).
- Produces: `MAX_CUES` (5), `FRAMES_PER_SECOND` (24.0), `MAX_FRAME` (999); `Cue {params, start_frame, duration_frames, curve}`; `SeqEvent {RampStarted, RampFinished, Restarted}`; `SeqView::{Rest(&Params), Ramp{from, to, progress, curve}}`; `Sequence` (`new(Params)`, `cues()`, `selected_cue_mut()`, pub `selected`, pub `looping`, `is_running()`, `clock_frames()`, `add_cue() -> bool`, `delete_cue() -> bool`, `set_start_frame(i, f)`, `set_duration(i, f)`, `run()`, `stop()`, `reset()`, `displayed_cue()`, `view()`, `advance(dt) -> Vec<SeqEvent>`).

- [ ] **Step 1: Register the module.** Replace `src/lib.rs` with:

```rust
//! Rasterwarp: real-time analog video-synthesis animation.

pub mod app;
pub mod curve;
pub mod gpu;
pub mod params;
pub mod passes;
pub mod sequence;
pub mod source;
pub mod transition;
pub mod ui;
```

- [ ] **Step 2: Write the failing tests.** Create `src/sequence.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Three cues at frames 0, 24 and 72, each ramp 24 frames, zoom 1, 2, 3.
    fn three_cues() -> Sequence {
        let mut s = Sequence::new(Params::default());
        s.add_cue();
        s.add_cue();
        for (i, start) in [0, 24, 72].into_iter().enumerate() {
            s.cues[i].params.warp.zoom = (i + 1) as f32;
            s.cues[i].start_frame = start;
            s.cues[i].duration_frames = 24;
        }
        s
    }

    fn zoom_from(view: SeqView<'_>) -> (f32, Option<f32>) {
        match view {
            SeqView::Rest(p) => (p.warp.zoom, None),
            SeqView::Ramp { from, progress, .. } => (from.warp.zoom, Some(progress)),
        }
    }

    #[test]
    fn stopped_sequence_shows_selected_cue() {
        let mut s = three_cues();
        s.selected = 2;
        assert_eq!(zoom_from(s.view()), (3.0, None));
        assert!(s.advance(1.0).is_empty());
    }

    #[test]
    fn cues_ramp_in_at_their_start_frames() {
        let mut s = three_cues();
        s.run();
        assert_eq!(s.advance(0.5), vec![]); // frame 12: resting on cue 1
        assert_eq!(zoom_from(s.view()), (1.0, None));
        assert_eq!(s.advance(0.5), vec![SeqEvent::RampStarted]); // frame 24
        assert_eq!(s.advance(0.5), vec![]); // frame 36: halfway to cue 2
        assert_eq!(zoom_from(s.view()), (1.0, Some(0.5)));
        assert_eq!(s.advance(0.5), vec![SeqEvent::RampFinished]); // frame 48
        assert_eq!(zoom_from(s.view()), (2.0, None));
    }

    #[test]
    fn overlapping_cue_snaps_previous_ramp() {
        let mut s = three_cues();
        s.cues[1].duration_frames = 100; // still ramping when cue 3 starts at 72
        s.run();
        s.advance(1.0); // frame 24: ramp to cue 2 starts
        let events = s.advance(2.0); // frame 72
        assert_eq!(events, vec![SeqEvent::RampFinished, SeqEvent::RampStarted]);
        assert_eq!(zoom_from(s.view()), (2.0, Some(0.0)));
    }

    #[test]
    fn reset_returns_to_cue_one() {
        let mut s = three_cues();
        s.run();
        s.advance(3.0);
        s.reset();
        assert_eq!(s.clock_frames(), 0.0);
        assert_eq!(zoom_from(s.view()), (1.0, None));
        assert!(s.is_running());
    }

    #[test]
    fn loop_restarts_after_last_ramp() {
        let mut s = three_cues();
        s.looping = true;
        s.run();
        let mut events = Vec::new();
        for _ in 0..5 {
            events.extend(s.advance(1.0)); // frames 24..120; last cue ends at 96
        }
        assert!(events.contains(&SeqEvent::Restarted));
    }

    #[test]
    fn start_frames_stay_ordered() {
        let mut s = three_cues();
        s.set_start_frame(1, 500);
        assert_eq!(s.cues()[1].start_frame, 71);
        s.set_start_frame(1, 0);
        assert_eq!(s.cues()[1].start_frame, 1);
        s.set_start_frame(0, 10);
        assert_eq!(s.cues()[0].start_frame, 0);
    }

    #[test]
    fn cue_count_is_capped_and_first_cue_is_permanent() {
        let mut s = Sequence::new(Params::default());
        for _ in 0..4 {
            assert!(s.add_cue());
        }
        assert!(!s.add_cue());
        assert_eq!(s.cues().len(), MAX_CUES);
        s.selected = 0;
        assert!(!s.delete_cue());
        s.selected = 3;
        assert!(s.delete_cue());
        assert_eq!(s.selected, 2);
    }
}
```

- [ ] **Step 3: Run them and confirm they fail.** Run: `cargo test --lib sequence::`. Expected: compile errors such as "cannot find type `Sequence`".

- [ ] **Step 4: Implement.** Replace `src/sequence.rs` with:

```rust
//! Sequence mode: up to 5 cues, each ramping in at its own start frame
//! (the Animation Aid's sequence ramps with frame-count thumbwheels).

use crate::curve::CurveRef;
use crate::params::Params;

pub const MAX_CUES: usize = 5;
/// The manual's thumbwheels count film frames.
pub const FRAMES_PER_SECOND: f32 = 24.0;
/// Highest start frame (3-digit thumbwheel) and longest ramp.
pub const MAX_FRAME: u32 = 999;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cue {
    pub params: Params,
    pub start_frame: u32,
    pub duration_frames: u32,
    pub curve: CurveRef,
}

/// What happened during one `advance`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeqEvent {
    /// A ramp toward the next cue began.
    RampStarted,
    /// The current ramp reached its cue (normally, or snapped by the next cue starting).
    RampFinished,
    /// Looped back to cue 1.
    Restarted,
}

/// What the sequence is showing right now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SeqView<'a> {
    Rest(&'a Params),
    Ramp {
        from: &'a Params,
        to: &'a Params,
        /// Linear progress 0..1.
        progress: f32,
        curve: CurveRef,
    },
}

#[derive(Clone, Debug)]
pub struct Sequence {
    cues: Vec<Cue>,
    pub selected: usize,
    pub looping: bool,
    running: bool,
    /// Frames since Run.
    clock: f32,
    /// The last cue fully reached.
    base: usize,
    /// The cue currently being ramped toward.
    target: Option<usize>,
}

impl Sequence {
    pub fn new(params: Params) -> Self {
        Self {
            cues: vec![Cue {
                params,
                start_frame: 0,
                duration_frames: 48,
                curve: CurveRef::SCurve,
            }],
            selected: 0,
            looping: false,
            running: false,
            clock: 0.0,
            base: 0,
            target: None,
        }
    }

    pub fn cues(&self) -> &[Cue] {
        &self.cues
    }

    pub fn selected_cue_mut(&mut self) -> &mut Cue {
        &mut self.cues[self.selected]
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn clock_frames(&self) -> f32 {
        self.clock
    }

    /// Appends a copy of the selected cue two seconds after the last one.
    /// Returns false when full or out of thumbwheel range.
    pub fn add_cue(&mut self) -> bool {
        let last = self.cues[self.cues.len() - 1];
        let start = last.start_frame + 48;
        if self.cues.len() >= MAX_CUES || start > MAX_FRAME {
            return false;
        }
        let mut cue = self.cues[self.selected];
        cue.start_frame = start;
        self.cues.push(cue);
        self.selected = self.cues.len() - 1;
        self.stop();
        true
    }

    /// Deletes the selected cue; the first cue can't be deleted.
    pub fn delete_cue(&mut self) -> bool {
        if self.selected == 0 {
            return false;
        }
        self.cues.remove(self.selected);
        self.selected -= 1;
        self.stop();
        true
    }

    /// Sets a cue's start frame, kept strictly between its neighbours. Cue 1 is fixed at 0.
    pub fn set_start_frame(&mut self, index: usize, frame: u32) {
        if index == 0 {
            return;
        }
        let lo = self.cues[index - 1].start_frame + 1;
        let hi = self
            .cues
            .get(index + 1)
            .map_or(MAX_FRAME, |c| c.start_frame - 1);
        self.cues[index].start_frame = frame.clamp(lo, hi.max(lo));
    }

    pub fn set_duration(&mut self, index: usize, frames: u32) {
        self.cues[index].duration_frames = frames.clamp(1, MAX_FRAME);
    }

    /// Starts from cue 1 at frame 0.
    pub fn run(&mut self) {
        self.running = true;
        self.reset();
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    /// Instantly back to cue 1, frame 0 (keeps running if it was).
    pub fn reset(&mut self) {
        self.clock = 0.0;
        self.base = 0;
        self.target = None;
    }

    /// The parameters shown when leaving Sequence mode.
    pub fn displayed_cue(&self) -> &Params {
        if self.running {
            &self.cues[self.base].params
        } else {
            &self.cues[self.selected].params
        }
    }

    pub fn view(&self) -> SeqView<'_> {
        if !self.running {
            return SeqView::Rest(&self.cues[self.selected].params);
        }
        match self.target {
            Some(t) => SeqView::Ramp {
                from: &self.cues[self.base].params,
                to: &self.cues[t].params,
                progress: self.progress(t).clamp(0.0, 1.0),
                curve: self.cues[t].curve,
            },
            None => SeqView::Rest(&self.cues[self.base].params),
        }
    }

    fn progress(&self, t: usize) -> f32 {
        let cue = &self.cues[t];
        (self.clock - cue.start_frame as f32) / cue.duration_frames as f32
    }

    /// Advances by `dt` seconds of animation time.
    pub fn advance(&mut self, dt: f32) -> Vec<SeqEvent> {
        let mut events = Vec::new();
        if !self.running {
            return events;
        }
        self.clock += dt * FRAMES_PER_SECOND;
        loop {
            let next = self.target.map_or(self.base + 1, |t| t + 1);
            if next >= self.cues.len() || self.clock < self.cues[next].start_frame as f32 {
                break;
            }
            if let Some(t) = self.target {
                // The next cue starts before this ramp finished: snap to its end.
                self.base = t;
                events.push(SeqEvent::RampFinished);
            }
            self.target = Some(next);
            events.push(SeqEvent::RampStarted);
        }
        if let Some(t) = self.target
            && self.progress(t) >= 1.0
        {
            self.base = t;
            self.target = None;
            events.push(SeqEvent::RampFinished);
        }
        let last = &self.cues[self.cues.len() - 1];
        let end = (last.start_frame + last.duration_frames) as f32;
        if self.looping
            && self.target.is_none()
            && self.base == self.cues.len() - 1
            && self.clock >= end
        {
            self.reset();
            events.push(SeqEvent::Restarted);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three cues at frames 0, 24 and 72, each ramp 24 frames, zoom 1, 2, 3.
    fn three_cues() -> Sequence {
        let mut s = Sequence::new(Params::default());
        s.add_cue();
        s.add_cue();
        for (i, start) in [0, 24, 72].into_iter().enumerate() {
            s.cues[i].params.warp.zoom = (i + 1) as f32;
            s.cues[i].start_frame = start;
            s.cues[i].duration_frames = 24;
        }
        s
    }

    fn zoom_from(view: SeqView<'_>) -> (f32, Option<f32>) {
        match view {
            SeqView::Rest(p) => (p.warp.zoom, None),
            SeqView::Ramp { from, progress, .. } => (from.warp.zoom, Some(progress)),
        }
    }

    #[test]
    fn stopped_sequence_shows_selected_cue() {
        let mut s = three_cues();
        s.selected = 2;
        assert_eq!(zoom_from(s.view()), (3.0, None));
        assert!(s.advance(1.0).is_empty());
    }

    #[test]
    fn cues_ramp_in_at_their_start_frames() {
        let mut s = three_cues();
        s.run();
        assert_eq!(s.advance(0.5), vec![]); // frame 12: resting on cue 1
        assert_eq!(zoom_from(s.view()), (1.0, None));
        assert_eq!(s.advance(0.5), vec![SeqEvent::RampStarted]); // frame 24
        assert_eq!(s.advance(0.5), vec![]); // frame 36: halfway to cue 2
        assert_eq!(zoom_from(s.view()), (1.0, Some(0.5)));
        assert_eq!(s.advance(0.5), vec![SeqEvent::RampFinished]); // frame 48
        assert_eq!(zoom_from(s.view()), (2.0, None));
    }

    #[test]
    fn overlapping_cue_snaps_previous_ramp() {
        let mut s = three_cues();
        s.cues[1].duration_frames = 100; // still ramping when cue 3 starts at 72
        s.run();
        s.advance(1.0); // frame 24: ramp to cue 2 starts
        let events = s.advance(2.0); // frame 72
        assert_eq!(events, vec![SeqEvent::RampFinished, SeqEvent::RampStarted]);
        assert_eq!(zoom_from(s.view()), (2.0, Some(0.0)));
    }

    #[test]
    fn reset_returns_to_cue_one() {
        let mut s = three_cues();
        s.run();
        s.advance(3.0);
        s.reset();
        assert_eq!(s.clock_frames(), 0.0);
        assert_eq!(zoom_from(s.view()), (1.0, None));
        assert!(s.is_running());
    }

    #[test]
    fn loop_restarts_after_last_ramp() {
        let mut s = three_cues();
        s.looping = true;
        s.run();
        let mut events = Vec::new();
        for _ in 0..5 {
            events.extend(s.advance(1.0)); // frames 24..120; last cue ends at 96
        }
        assert!(events.contains(&SeqEvent::Restarted));
    }

    #[test]
    fn start_frames_stay_ordered() {
        let mut s = three_cues();
        s.set_start_frame(1, 500);
        assert_eq!(s.cues()[1].start_frame, 71);
        s.set_start_frame(1, 0);
        assert_eq!(s.cues()[1].start_frame, 1);
        s.set_start_frame(0, 10);
        assert_eq!(s.cues()[0].start_frame, 0);
    }

    #[test]
    fn cue_count_is_capped_and_first_cue_is_permanent() {
        let mut s = Sequence::new(Params::default());
        for _ in 0..4 {
            assert!(s.add_cue());
        }
        assert!(!s.add_cue());
        assert_eq!(s.cues().len(), MAX_CUES);
        s.selected = 0;
        assert!(!s.delete_cue());
        s.selected = 3;
        assert!(s.delete_cue());
        assert_eq!(s.selected, 2);
    }
}
```

- [ ] **Step 5: Run the tests and confirm they pass.** Run: `cargo test --lib sequence::`. Expected: `7 passed`.

- [ ] **Step 6: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/sequence.rs src/lib.rs
git commit -m "feat: add sequence cues on a 24 fps frame clock" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Blend and phase clocks

**Files:**
- Create: `src/blend.rs`
- Modify: `src/lib.rs`
- Test: unit tests in `src/blend.rs`

**Interfaces:**
- Consumes: `Params`, `OscSync`, `Envelope`, `effective_oscillators`, `ranges`, `srgb_to_linear` (Task 1).
- Produces: `MAX_SLOTS` (8); `Clocks {osc: [f64;4], lfo: [f64;4], cycle: f64}` with `advance(&Params, dt)`; `OscSlot {index, waveform, target, input, frequency, amplitude, phase, lfo_phase, lfo_depth}`; `WarpFrame {zoom, rotation, offset, drift, oscillators: Vec<OscSlot>}`; `ColorizeFrame {levels, softness, palette_linear, cycle, bypass}`; `FrameParams {warp, colorize, feedback, glow}` with `FrameParams::at_rest(&Params)`; `blend(from, to, t, ramp_progress: Option<f32>, from_clocks, to_clocks) -> FrameParams`.

> Note: see design decisions 1–3 at the top of this plan. Swell is driven by the *raw linear* ramp progress (`ramp_progress`), not the curve-mapped `t`.

- [ ] **Step 1: Register the module.** Replace `src/lib.rs` with:

```rust
//! Rasterwarp: real-time analog video-synthesis animation.

pub mod app;
pub mod blend;
pub mod curve;
pub mod gpu;
pub mod params;
pub mod passes;
pub mod sequence;
pub mod source;
pub mod transition;
pub mod ui;
```

- [ ] **Step 2: Write the failing tests.** Create `src/blend.rs` containing only:

```rust
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
```

- [ ] **Step 3: Run them and confirm they fail.** Run: `cargo test --lib blend::`. Expected: compile errors such as "cannot find function `blend`".

- [ ] **Step 4: Implement.** Replace `src/blend.rs` with:

```rust
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
```

- [ ] **Step 5: Run the tests and confirm they pass.** Run: `cargo test --lib blend::`. Expected: `12 passed`.

- [ ] **Step 6: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/blend.rs src/lib.rs
git commit -m "feat: blend parameter sets with continuous phase clocks" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Renderer consumes FrameParams

**Files:**
- Modify: `shaders/warp.wgsl`, `src/passes/warp.rs`, `src/passes/colorize.rs`, `src/passes/mod.rs`, `tests/smoke.rs`, `src/app.rs`
- Test: unit tests in `warp.rs` and `colorize.rs`; `tests/smoke.rs`

**Interfaces:**
- Consumes: `FrameParams`, `WarpFrame`, `OscSlot`, `ColorizeFrame`, `MAX_SLOTS`, `Clocks`, `blend` (Task 5).
- Produces:
  - `warp::uniforms(&WarpFrame, time, frame_aspect, source_aspect) -> WarpUniforms`. `WarpUniforms` is now 432 bytes with 8 slots, and `frame.w` holds the slot count.
  - `colorize::uniforms(&ColorizeFrame) -> ColorizeUniforms`.
  - `Renderer::render(.., frame: &FrameParams, time: f32, ..)`.
- **Intermediate app:** `src/app.rs` keeps `params: Params`, adds `clocks: Clocks` (advanced each frame), and renders `blend(&params, &params, 0.0, None, &clocks, &clocks)`. Task 8 replaces this with `Motion`.

- [ ] **Step 1: Write the failing tests.** Replace the `mod tests` block of `src/passes/warp.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend::FrameParams;
    use crate::params::Params;

    #[test]
    fn pipeline_matches_shader() {
        // Building the pipeline validates the WGSL against the bind group layout.
        let Some((device, _queue)) = crate::gpu::test_device() else {
            return;
        };
        WarpPass::new(&device, 64, 64);
    }

    #[test]
    fn uniform_layout_matches_wgsl() {
        // Osc = 3 x vec4 = 48 bytes; Warp = 3 x vec4 + 8 x Osc = 432 bytes.
        assert_eq!(size_of::<OscUniform>(), 48);
        assert_eq!(size_of::<WarpUniforms>(), 432);
    }

    #[test]
    fn wide_source_fits_frame_width() {
        assert_eq!(source_fit(1.5, 3.0), [1.5, 0.5]);
    }

    #[test]
    fn tall_source_fits_frame_height() {
        assert_eq!(source_fit(1.5, 0.5), [0.5, 1.0]);
    }

    #[test]
    fn packs_oscillator_slots() {
        let p = Params::default();
        let frame = FrameParams::at_rest(&p);
        let u = uniforms(&frame.warp, 2.0, 16.0 / 9.0, 1.0);
        assert_eq!(u.frame[0], 2.0);
        assert_eq!(u.frame[3], 4.0, "slot count");
        // Default oscillator 2: triangle, Y target, U input, oscillator index 1.
        assert_eq!(u.osc[1].mode, [1, 1, 0, 1]);
        assert_eq!(u.osc[0].wave[1], p.warp.oscillators[0].amplitude);
        assert_eq!(u.osc[4].wave, [0.0; 4], "unused slots are zero");
    }
}
```

and the `mod tests` block of `src/passes/colorize.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend::FrameParams;
    use crate::params::Params;

    #[test]
    fn pipeline_matches_shader() {
        // Building the pipeline validates the WGSL against the bind group layout.
        let Some((device, _queue)) = crate::gpu::test_device() else {
            return;
        };
        ColorizePass::new(&device, 64, 64);
    }

    #[test]
    fn uniform_layout_matches_wgsl() {
        // vec4 settings + array<vec4, 8> = 16 + 128 bytes.
        assert_eq!(size_of::<ColorizeUniforms>(), 144);
    }

    #[test]
    fn packs_frame_values() {
        let mut frame = FrameParams::at_rest(&Params::default()).colorize;
        frame.cycle = 2.5;
        frame.bypass = true;
        frame.palette_linear[1] = [0.25, 0.5, 0.75];
        let u = uniforms(&frame);
        assert_eq!(u.settings, [6.0, frame.softness, 2.5, 1.0]);
        assert_eq!(u.palette[1], [0.25, 0.5, 0.75, 1.0]);
    }
}
```

- [ ] **Step 2: Run them and confirm they fail.** Run: `cargo test --lib passes::`. Expected: compile errors, because `uniforms` still takes `&WarpParams` / `&ColorizeParams`.

- [ ] **Step 3: Update the shader.** Replace `shaders/warp.wgsl` with:

```wgsl
// Deflection: displace the sampling position with summed oscillators.

struct Osc {
    mode: vec4<u32>, // waveform, target (0 = X, 1 = Y), input (0 = U, 1 = V, 2 = radius, 3 = time), oscillator index
    wave: vec4<f32>, // frequency, amplitude (weighted), phase (cycles), unused
    lfo: vec4<f32>,  // phase (cycles), depth, unused, unused
};

struct Warp {
    frame: vec4<f32>,       // time, frame aspect, drift, slot count
    transform: vec4<f32>,   // zoom, rotation, offset.x, offset.y
    source_size: vec4<f32>, // source width, height in frame units, unused, unused
    osc: array<Osc, 8>,
};

@group(0) @binding(0) var<uniform> u: Warp;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var source: texture_2d<f32>;

// One cycle per unit of x, output in [-1, 1].
fn wave(shape: u32, x: f32) -> f32 {
    let f = fract(x);
    switch shape {
        case 0u: { return sin(x * TAU); }
        case 1u: { return 1.0 - 4.0 * abs(f - 0.5); }
        case 2u: { return 2.0 * f - 1.0; }
        case 3u: { return select(1.0, -1.0, f >= 0.5); }
        default: { return value_noise(x); }
    }
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let time = u.frame.x;
    let aspect = u.frame.y;
    let drift = u.frame.z;
    // Centered frame coordinates: y spans [-0.5, 0.5], x spans [-aspect/2, aspect/2].
    let p = (in.uv - 0.5) * vec2<f32>(aspect, 1.0);

    var d = vec2<f32>(0.0);
    let slots = u32(u.frame.w);
    for (var i = 0u; i < slots; i++) {
        let o = u.osc[i];
        var input = 0.0;
        switch o.mode.z {
            case 0u: { input = p.x; }
            case 1u: { input = p.y; }
            case 2u: { input = length(p); }
            default: { input = 0.0; }
        }
        // Seed drift by oscillator, not slot, so it doesn't jump when slots appear.
        let seed = f32(o.mode.w) * 17.0;
        let freq = o.wave.x * (1.0 + drift * 0.05 * value_noise(time * 0.3 + seed));
        let phase = o.wave.z + drift * 0.1 * value_noise(time * 0.2 + seed + 5.0);
        let lfo = 1.0 - o.lfo.y * (0.5 - 0.5 * cos(TAU * o.lfo.x));
        let v = wave(o.mode.x, input * freq + phase) * o.wave.y * lfo;
        if o.mode.y == 0u {
            d.x += v;
        } else {
            d.y += v;
        }
    }

    let q = rotate2(p + d, u.transform.y) / u.transform.x - u.transform.zw;
    let suv = q / u.source_size.xy + 0.5;
    if !inside01(suv) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let g = textureSampleLevel(source, samp, suv, 0.0).r;
    return vec4<f32>(g, g, g, 1.0);
}
```

- [ ] **Step 4: Update the passes.** Replace `src/passes/warp.rs` with:

```rust
//! Deflection pass: samples the source at oscillator-displaced coordinates.

use bytemuck::{Pod, Zeroable};

use crate::blend::{MAX_SLOTS, OscSlot, WarpFrame};
use crate::gpu::{FullscreenPass, INTERNAL_FORMAT, PassDesc, RenderTarget};
use crate::params::{Axis, OscInput, Waveform};

/// Matches `struct Osc` in warp.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct OscUniform {
    pub mode: [u32; 4],
    pub wave: [f32; 4],
    pub lfo: [f32; 4],
}

/// Matches `struct Warp` in warp.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct WarpUniforms {
    pub frame: [f32; 4],
    pub transform: [f32; 4],
    pub source_size: [f32; 4],
    pub osc: [OscUniform; MAX_SLOTS],
}

/// Size of the source inside the frame (frame height = 1), fitted so the whole
/// source is visible.
pub fn source_fit(frame_aspect: f32, source_aspect: f32) -> [f32; 2] {
    if source_aspect >= frame_aspect {
        [frame_aspect, frame_aspect / source_aspect]
    } else {
        [source_aspect, 1.0]
    }
}

pub fn uniforms(w: &WarpFrame, time: f32, frame_aspect: f32, source_aspect: f32) -> WarpUniforms {
    let fit = source_fit(frame_aspect, source_aspect);
    let count = w.oscillators.len().min(MAX_SLOTS);
    let mut osc = [OscUniform::zeroed(); MAX_SLOTS];
    for (dst, slot) in osc.iter_mut().zip(&w.oscillators) {
        *dst = osc_uniform(slot);
    }
    WarpUniforms {
        frame: [time, frame_aspect, w.drift, count as f32],
        transform: [w.zoom, w.rotation, w.offset[0], w.offset[1]],
        source_size: [fit[0], fit[1], 0.0, 0.0],
        osc,
    }
}

fn osc_uniform(o: &OscSlot) -> OscUniform {
    OscUniform {
        mode: [
            match o.waveform {
                Waveform::Sine => 0,
                Waveform::Triangle => 1,
                Waveform::Ramp => 2,
                Waveform::Square => 3,
                Waveform::Noise => 4,
            },
            match o.target {
                Axis::X => 0,
                Axis::Y => 1,
            },
            match o.input {
                OscInput::U => 0,
                OscInput::V => 1,
                OscInput::Radius => 2,
                OscInput::Time => 3,
            },
            o.index as u32,
        ],
        wave: [o.frequency, o.amplitude, o.phase, 0.0],
        lfo: [o.lfo_phase, o.lfo_depth, 0.0, 0.0],
    }
}

pub struct WarpPass {
    pass: FullscreenPass,
    uniform: wgpu::Buffer,
    pub target: RenderTarget,
}

impl WarpPass {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let pass = FullscreenPass::new(
            device,
            &PassDesc {
                label: "warp",
                shader: include_str!("../../shaders/warp.wgsl"),
                fragment_entry: "fs_main",
                uniform_size: size_of::<WarpUniforms>() as u64,
                textures: 1,
                format: INTERNAL_FORMAT,
                blend: None,
            },
        );
        let uniform = pass.create_uniform_buffer(device);
        let target = RenderTarget::new(device, "warp", width, height, INTERNAL_FORMAT);
        Self {
            pass,
            uniform,
            target,
        }
    }

    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        uniforms: &WarpUniforms,
    ) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniforms));
        let bind_group = self.pass.bind_group(device, &self.uniform, &[source]);
        self.pass
            .draw(encoder, &self.target.view, &bind_group, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend::FrameParams;
    use crate::params::Params;

    #[test]
    fn pipeline_matches_shader() {
        // Building the pipeline validates the WGSL against the bind group layout.
        let Some((device, _queue)) = crate::gpu::test_device() else {
            return;
        };
        WarpPass::new(&device, 64, 64);
    }

    #[test]
    fn uniform_layout_matches_wgsl() {
        // Osc = 3 x vec4 = 48 bytes; Warp = 3 x vec4 + 8 x Osc = 432 bytes.
        assert_eq!(size_of::<OscUniform>(), 48);
        assert_eq!(size_of::<WarpUniforms>(), 432);
    }

    #[test]
    fn wide_source_fits_frame_width() {
        assert_eq!(source_fit(1.5, 3.0), [1.5, 0.5]);
    }

    #[test]
    fn tall_source_fits_frame_height() {
        assert_eq!(source_fit(1.5, 0.5), [0.5, 1.0]);
    }

    #[test]
    fn packs_oscillator_slots() {
        let p = Params::default();
        let frame = FrameParams::at_rest(&p);
        let u = uniforms(&frame.warp, 2.0, 16.0 / 9.0, 1.0);
        assert_eq!(u.frame[0], 2.0);
        assert_eq!(u.frame[3], 4.0, "slot count");
        // Default oscillator 2: triangle, Y target, U input, oscillator index 1.
        assert_eq!(u.osc[1].mode, [1, 1, 0, 1]);
        assert_eq!(u.osc[0].wave[1], p.warp.oscillators[0].amplitude);
        assert_eq!(u.osc[4].wave, [0.0; 4], "unused slots are zero");
    }
}
```

Replace `src/passes/colorize.rs` with:

```rust
//! Colorize pass: posterizes the warped grayscale and maps levels to palette colors.

use bytemuck::{Pod, Zeroable};

use crate::blend::ColorizeFrame;
use crate::gpu::{FullscreenPass, INTERNAL_FORMAT, PassDesc, RenderTarget};
use crate::params::PALETTE_SIZE;

/// Matches `struct Colorize` in colorize.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ColorizeUniforms {
    pub settings: [f32; 4],
    pub palette: [[f32; 4]; PALETTE_SIZE],
}

pub fn uniforms(c: &ColorizeFrame) -> ColorizeUniforms {
    ColorizeUniforms {
        settings: [
            c.levels as f32,
            c.softness,
            c.cycle,
            if c.bypass { 1.0 } else { 0.0 },
        ],
        palette: c.palette_linear.map(|[r, g, b]| [r, g, b, 1.0]),
    }
}

pub struct ColorizePass {
    pass: FullscreenPass,
    uniform: wgpu::Buffer,
    pub target: RenderTarget,
}

impl ColorizePass {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let pass = FullscreenPass::new(
            device,
            &PassDesc {
                label: "colorize",
                shader: include_str!("../../shaders/colorize.wgsl"),
                fragment_entry: "fs_main",
                uniform_size: size_of::<ColorizeUniforms>() as u64,
                textures: 1,
                format: INTERNAL_FORMAT,
                blend: None,
            },
        );
        let uniform = pass.create_uniform_buffer(device);
        let target = RenderTarget::new(device, "colorize", width, height, INTERNAL_FORMAT);
        Self {
            pass,
            uniform,
            target,
        }
    }

    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        uniforms: &ColorizeUniforms,
    ) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniforms));
        let bind_group = self.pass.bind_group(device, &self.uniform, &[input]);
        self.pass
            .draw(encoder, &self.target.view, &bind_group, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend::FrameParams;
    use crate::params::Params;

    #[test]
    fn pipeline_matches_shader() {
        // Building the pipeline validates the WGSL against the bind group layout.
        let Some((device, _queue)) = crate::gpu::test_device() else {
            return;
        };
        ColorizePass::new(&device, 64, 64);
    }

    #[test]
    fn uniform_layout_matches_wgsl() {
        // vec4 settings + array<vec4, 8> = 16 + 128 bytes.
        assert_eq!(size_of::<ColorizeUniforms>(), 144);
    }

    #[test]
    fn packs_frame_values() {
        let mut frame = FrameParams::at_rest(&Params::default()).colorize;
        frame.cycle = 2.5;
        frame.bypass = true;
        frame.palette_linear[1] = [0.25, 0.5, 0.75];
        let u = uniforms(&frame);
        assert_eq!(u.settings, [6.0, frame.softness, 2.5, 1.0]);
        assert_eq!(u.palette[1], [0.25, 0.5, 0.75, 1.0]);
    }
}
```

Replace `src/passes/mod.rs` with:

```rust
//! The render pipeline: source -> warp -> colorize -> feedback -> bloom -> composite.

pub mod bloom;
pub mod colorize;
pub mod composite;
pub mod feedback;
pub mod warp;

use crate::blend::FrameParams;
use crate::source::{self, GrayImage};

pub struct Renderer {
    size: (u32, u32),
    source: wgpu::TextureView,
    source_aspect: f32,
    warp: warp::WarpPass,
    colorize: colorize::ColorizePass,
    feedback: feedback::FeedbackPass,
    bloom: bloom::BloomPass,
    composite: composite::CompositePass,
}

impl Renderer {
    /// `size` is the fixed internal resolution; `output_format` is the format of the
    /// texture `render` draws into.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        output_format: wgpu::TextureFormat,
        size: (u32, u32),
        image: &GrayImage,
    ) -> Self {
        let (w, h) = size;
        Self {
            size,
            source: source::upload(device, queue, image).create_view(&Default::default()),
            source_aspect: image.aspect(),
            warp: warp::WarpPass::new(device, w, h),
            colorize: colorize::ColorizePass::new(device, w, h),
            feedback: feedback::FeedbackPass::new(device, w, h),
            bloom: bloom::BloomPass::new(device, w, h),
            composite: composite::CompositePass::new(device, output_format),
        }
    }

    pub fn set_source(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, image: &GrayImage) {
        self.source = source::upload(device, queue, image).create_view(&Default::default());
        self.source_aspect = image.aspect();
    }

    pub fn clear_feedback(&self, encoder: &mut wgpu::CommandEncoder) {
        self.feedback.clear(encoder);
    }

    /// Records one frame into `encoder`, ending with a draw into `output`
    /// (`output_size` pixels, letterboxed).
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameParams,
        time: f32,
        output: &wgpu::TextureView,
        output_size: (u32, u32),
    ) {
        let aspect = self.size.0 as f32 / self.size.1 as f32;
        self.warp.render(
            device,
            queue,
            encoder,
            &self.source,
            &warp::uniforms(&frame.warp, time, aspect, self.source_aspect),
        );
        self.colorize.render(
            device,
            queue,
            encoder,
            &self.warp.target.view,
            &colorize::uniforms(&frame.colorize),
        );
        self.feedback.render(
            device,
            queue,
            encoder,
            &self.colorize.target.view,
            &feedback::uniforms(&frame.feedback, aspect),
        );
        self.bloom.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            frame.glow.bloom_threshold,
        );
        self.composite.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            self.bloom.output(),
            output,
            &composite::uniforms(
                &frame.glow,
                time,
                self.size,
                output_size,
                self.composite.encode_srgb(),
            ),
        );
    }
}
```

- [ ] **Step 5: Update the smoke test and the app.** Replace `tests/smoke.rs` with:

```rust
//! Headless GPU smoke test: builds every pass on a real device, renders a few frames
//! offscreen, and reads the result back. wgpu validation errors panic, failing the test.

use rasterwarp::blend::FrameParams;
use rasterwarp::gpu;
use rasterwarp::params::Params;
use rasterwarp::passes::Renderer;
use rasterwarp::source::test_card;

const OUT_W: u32 = 320; // 320 * 4 bytes = 1280, a multiple of 256 as buffer copies require
const OUT_H: u32 = 180;

#[test]
fn renders_frames_offscreen() {
    let backends = wgpu::Backends::all();
    let instance = gpu::create_instance(backends);
    let Ok((_adapter, device, queue)) =
        pollster::block_on(gpu::request_device(&instance, backends, None))
    else {
        eprintln!("skipping smoke test: no GPU adapter available");
        return;
    };

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(&device, "smoke output", OUT_W, OUT_H, format);
    let mut renderer = Renderer::new(&device, &queue, format, (640, 360), &test_card(400, 300));
    let frame_params = FrameParams::at_rest(&Params::default());

    for frame in 0..3 {
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(
            &device,
            &queue,
            &mut encoder,
            &frame_params,
            frame as f32 * 0.1,
            &output.view,
            (OUT_W, OUT_H),
        );
        queue.submit([encoder.finish()]);
    }

    let pixels = read_back(&device, &queue, &output.texture);
    let (rgba, _) = pixels.as_chunks::<4>();
    assert!(
        rgba.iter().any(|px| *px != rgba[0]),
        "output is a single flat color"
    );
}

fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let size = (OUT_W * OUT_H * 4) as u64;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(OUT_W * 4),
                rows_per_image: Some(OUT_H),
            },
        },
        wgpu::Extent3d {
            width: OUT_W,
            height: OUT_H,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    buffer.map_async(wgpu::MapMode::Read, .., |result| {
        result.expect("map readback buffer")
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll device");
    buffer.get_mapped_range(..).expect("mapped range").to_vec()
}
```

Replace `src/app.rs` with this intermediate version (Task 8 replaces it):

```rust
//! The windowed application: owns the GPU context, renderer, parameters and UI,
//! and runs one frame per redraw.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::blend::{Clocks, blend};
use crate::gpu;
use crate::params::Params;
use crate::passes::Renderer;
use crate::source::{self, GrayImage};
use crate::ui::{self, UiActions, UiState};

/// Fixed internal render resolution, independent of the window size.
pub const INTERNAL_SIZE: (u32, u32) = (1920, 1080);

pub struct App {
    initial_image: Option<PathBuf>,
    state: Option<State>,
    error: Option<anyhow::Error>,
}

impl App {
    pub fn new(initial_image: Option<PathBuf>) -> Self {
        Self {
            initial_image,
            state: None,
            error: None,
        }
    }

    /// The startup error, if the app exited because it could not start.
    pub fn finish(self) -> Result<()> {
        match self.error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match State::new(event_loop, self.initial_image.as_deref()) {
            Ok(state) => self.state = Some(state),
            Err(err) => {
                self.error = Some(err);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.state else { return };
        let _ = state.egui_state.on_window_event(&state.window, &event);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.resize(size),
            WindowEvent::DroppedFile(path) => state.load_source(&path),
            WindowEvent::RedrawRequested => {
                state.redraw();
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    max_texture_side: u32,
    composite_format: wgpu::TextureFormat,
    renderer: Renderer,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    params: Params,
    /// Running oscillator, LFO and palette phases.
    clocks: Clocks,
    ui: UiState,
    time: f64,
    last_frame: Instant,
}

impl State {
    fn new(event_loop: &ActiveEventLoop, initial_image: Option<&Path>) -> Result<Self> {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Rasterwarp")
                        .with_inner_size(LogicalSize::new(1280.0, 720.0)),
                )
                .context("failed to create window")?,
        );
        let backends = gpu::backends_from_env()?;
        let instance = gpu::create_instance(backends);
        let surface = instance
            .create_surface(window.clone())
            .with_context(|| {
                format!(
                    "failed to create window surface for backend {backends:?}; try RASTERWARP_BACKEND=dx12 or gl"
                )
            })?;
        let (adapter, device, queue) =
            pollster::block_on(gpu::request_device(&instance, backends, Some(&surface)))?;
        let max_texture_side = device.limits().max_texture_dimension_2d;

        let caps = surface.get_capabilities(&adapter);
        // The swapchain itself is non-sRGB (what egui expects). Where the adapter allows an
        // sRGB view of it, the composite pass draws through that view; otherwise (e.g. GL)
        // it draws into the plain format and encodes sRGB in the shader.
        let format = caps.formats[0].remove_srgb_suffix();
        let srgb_format = format.add_srgb_suffix();
        let srgb_view_ok = adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::SURFACE_VIEW_FORMATS);
        let composite_format = if srgb_view_ok { srgb_format } else { format };
        // Fifo (vsync) keeps the per-frame feedback semantics at 60 fps. RASTERWARP_UNCAPPED=1
        // opts into Mailbox (uncapped, no tearing) to measure the real render cost.
        let uncapped = std::env::var("RASTERWARP_UNCAPPED").is_ok_and(|v| v == "1");
        let present_mode = if uncapped && caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .context("window surface is not supported by the GPU adapter")?;
        config.format = format;
        config.view_formats = if srgb_view_ok {
            vec![srgb_format]
        } else {
            vec![]
        };
        config.present_mode = present_mode;
        if size.width > 0 && size.height > 0 {
            surface.configure(&device, &config);
        }
        log::info!("surface: {format:?} (composite via {composite_format:?}), {present_mode:?}");

        let mut ui = UiState {
            frame_ms: 16.7,
            ..Default::default()
        };
        let image = match initial_image {
            Some(path) => match load_fitting(path, max_texture_side) {
                Ok(image) => {
                    ui.source_info = describe(&path.display().to_string(), &image);
                    image
                }
                Err(err) => {
                    log::warn!("{err:#}");
                    ui.load_error = Some(format!("{err:#}"));
                    test_card(&mut ui)
                }
            },
            None => test_card(&mut ui),
        };
        let renderer = Renderer::new(&device, &queue, composite_format, INTERNAL_SIZE, &image);

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui_ctx.viewport_id(),
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(max_texture_side as usize),
        );
        let egui_renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());

        Ok(Self {
            window,
            surface,
            config,
            device,
            queue,
            max_texture_side,
            composite_format,
            renderer,
            egui_ctx,
            egui_state,
            egui_renderer,
            params: Params::default(),
            clocks: Clocks::default(),
            ui,
            time: 0.0,
            last_frame: Instant::now(),
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = size.width;
        self.config.height = size.height;
        if size.width > 0 && size.height > 0 {
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn load_source(&mut self, path: &Path) {
        match load_fitting(path, self.max_texture_side) {
            Ok(image) => {
                self.renderer.set_source(&self.device, &self.queue, &image);
                self.ui.source_info = describe(&path.display().to_string(), &image);
                self.ui.load_error = None;
            }
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.load_error = Some(format!("{err:#}"));
            }
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.ui.frame_ms += (dt * 1000.0 - self.ui.frame_ms) * 0.05;
        if !self.ui.paused {
            // Clamp so a stall (e.g. dragging the window) doesn't make animation jump.
            let step = dt.min(0.1);
            self.time += f64::from(step);
            self.clocks.advance(&self.params, step);
        }

        if self.config.width == 0 || self.config.height == 0 {
            return; // minimized
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("validation error acquiring the surface texture");
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let srgb_view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.composite_format),
            ..Default::default()
        });
        let output_size = (self.config.width, self.config.height);

        let raw_input = self.egui_state.take_egui_input(&self.window);
        let mut actions = UiActions::default();
        let egui_output = self.egui_ctx.run_ui(raw_input, |ui| {
            actions = ui::draw(ui, &mut self.params, &mut self.ui)
        });
        self.egui_state
            .handle_platform_output(&self.window, egui_output.platform_output);
        let jobs = self
            .egui_ctx
            .tessellate(egui_output.shapes, egui_output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [output_size.0, output_size.1],
            pixels_per_point: egui_output.pixels_per_point,
        };
        for (id, deltas) in &egui_output.textures_delta.set {
            for delta in deltas {
                self.egui_renderer
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
        }

        let frame_params = blend(
            &self.params,
            &self.params,
            0.0,
            None,
            &self.clocks,
            &self.clocks,
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        if actions.clear_feedback {
            self.renderer.clear_feedback(&mut encoder);
        }
        self.renderer.render(
            &self.device,
            &self.queue,
            &mut encoder,
            &frame_params,
            self.time as f32,
            &srgb_view,
            output_size,
        );
        let egui_commands = self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &jobs,
            &screen,
        );
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.egui_renderer.render(&mut pass, &jobs, &screen);
        }
        self.queue
            .submit(egui_commands.into_iter().chain([encoder.finish()]));
        self.window.pre_present_notify();
        self.queue.present(frame);
        for id in &egui_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
    }
}

fn test_card(ui: &mut UiState) -> GrayImage {
    let image = source::test_card(1600, 900);
    ui.source_info = describe("built-in test card", &image);
    image
}

fn describe(name: &str, image: &GrayImage) -> String {
    format!("Source: {name} ({}×{})", image.width, image.height)
}

/// Loads an image and checks it fits in a GPU texture.
fn load_fitting(path: &Path, max_side: u32) -> anyhow::Result<GrayImage> {
    let image = source::load_image(path)?;
    source::ensure_fits(&image, max_side)
        .with_context(|| format!("could not load {}", path.display()))?;
    Ok(image)
}
```

- [ ] **Step 6: Run everything.** Run: `cargo test`. Expected: all unit tests and `renders_frames_offscreen` pass. `pipeline_matches_shader` in `warp.rs` validates the new 8-slot WGSL layout on the GPU.

- [ ] **Step 7: Check by eye.** Run: `cargo run --release`. The default look animates exactly as before: the ripple drifts, the LFO breathes, and there are trails and glow.

- [ ] **Step 8: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 9: Commit**

```bash
git add shaders/warp.wgsl src/passes/warp.rs src/passes/colorize.rs src/passes/mod.rs tests/smoke.rs src/app.rs
git commit -m "refactor: render from blended FrameParams with CPU phase clocks" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Motion controller

**Files:**
- Create: `src/motion.rs`
- Modify: `src/lib.rs`
- Test: unit tests in `src/motion.rs`

**Interfaces:**
- Consumes: `AbState`, `AbEvent` (Task 3); `Sequence`, `SeqEvent`, `SeqView` (Task 4); `Clocks`, `FrameParams`, `blend` (Task 5); `CurveLibrary` (Task 2).
- Produces: `Mode {Live, Transition, Sequence}` with `ALL`; `Motion` (pub `ab: AbState`, pub `sequence: Option<Sequence>`, pub `curves: CurveLibrary`; `new(Params)`, `mode()`, `set_mode(Mode)`, `editable() -> &mut Params`, `sequence_mut() -> &mut Sequence`, `trigger()`, `cut()`, `advance(dt)`, `frame() -> FrameParams`).

> Note: see design decisions 2, 5 and 6 at the top of this plan. `trigger` and `cut` do nothing outside Transition mode.

- [ ] **Step 1: Register the module.** Replace `src/lib.rs` with:

```rust
//! Rasterwarp: real-time analog video-synthesis animation.

pub mod app;
pub mod blend;
pub mod curve;
pub mod gpu;
pub mod motion;
pub mod params;
pub mod passes;
pub mod sequence;
pub mod source;
pub mod transition;
pub mod ui;
```

- [ ] **Step 2: Write the failing tests.** Create `src/motion.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_mode_edits_what_is_shown() {
        let mut m = Motion::new(Params::default());
        m.editable().warp.zoom = 2.5;
        assert_eq!(m.frame().warp.zoom, 2.5);
    }

    #[test]
    fn transition_mode_edits_off_air_until_triggered() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Transition);
        m.editable().warp.zoom = 3.0;
        assert_eq!(m.frame().warp.zoom, 1.0, "edits stay off air");
        m.trigger();
        m.advance(1.0); // halfway through the default 2 s S-curve
        assert!((m.frame().warp.zoom - 2.0).abs() < 1e-5);
        m.advance(1.0);
        assert_eq!(m.frame().warp.zoom, 3.0);
        assert_eq!(m.editable().warp.zoom, 1.0, "now editing the other bank");
    }

    #[test]
    fn trigger_and_cut_do_nothing_outside_transition_mode() {
        let mut m = Motion::new(Params::default());
        m.trigger();
        m.cut();
        assert!(m.ab.ramp().is_none());
        assert_eq!(m.ab.on_air, 0);
    }

    #[test]
    fn oscillator_phase_is_continuous_when_a_transition_finishes() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Transition);
        m.editable().warp.oscillators[0].phase_speed = 2.0;
        m.advance(0.3);
        m.trigger();
        m.advance(1.99);
        let before = m.frame().warp.oscillators[0].phase;
        m.advance(0.02); // finishes; the destination's clocks take over
        let after = m.frame().warp.oscillators[0].phase;
        let step = (after - before).rem_euclid(1.0);
        assert!(step < 0.1, "phase jumped by {step}");
    }

    #[test]
    fn leaving_sequence_puts_shown_cue_on_air() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Sequence);
        m.editable().warp.zoom = 1.7;
        m.set_mode(Mode::Live);
        assert_eq!(m.frame().warp.zoom, 1.7);
    }

    #[test]
    fn cues_survive_switching_modes() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Sequence);
        m.sequence_mut().add_cue();
        m.set_mode(Mode::Live);
        m.set_mode(Mode::Sequence);
        assert_eq!(m.sequence_mut().cues().len(), 2);
    }

    #[test]
    fn sequence_plays_its_cues() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Sequence);
        let seq = m.sequence_mut();
        seq.add_cue();
        seq.selected_cue_mut().params.warp.zoom = 2.0;
        seq.set_start_frame(1, 24);
        seq.set_duration(1, 24);
        seq.run();
        m.advance(1.0); // ramp to cue 2 starts at frame 24
        m.advance(1.0); // and ends at frame 48
        assert_eq!(m.frame().warp.zoom, 2.0);
    }
}
```

- [ ] **Step 3: Run them and confirm they fail.** Run: `cargo test --lib motion::`. Expected: compile errors such as "cannot find type `Motion`".

- [ ] **Step 4: Implement.** Replace `src/motion.rs` with:

```rust
//! Ties the modes together: which parameters the panel edits, how time moves
//! ramps and phase clocks forward, and what the renderer draws.

use crate::blend::{Clocks, FrameParams, blend};
use crate::curve::CurveLibrary;
use crate::params::Params;
use crate::sequence::{SeqEvent, SeqView, Sequence};
use crate::transition::{AbEvent, AbState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The panel edits what's on screen.
    Live,
    /// A/B banks: the panel edits the off-air bank; Transition ramps to it.
    Transition,
    /// Up to 5 cues played on a frame clock.
    Sequence,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Live, Mode::Transition, Mode::Sequence];
}

pub struct Motion {
    mode: Mode,
    pub ab: AbState,
    pub sequence: Option<Sequence>,
    pub curves: CurveLibrary,
    /// Phases of what's shown at rest (a running ramp's origin).
    rest: Clocks,
    /// Phases of a running ramp's destination.
    target: Clocks,
}

impl Motion {
    pub fn new(params: Params) -> Self {
        Self {
            mode: Mode::Live,
            ab: AbState::new(params),
            sequence: None,
            curves: CurveLibrary::default(),
            rest: Clocks::default(),
            target: Clocks::default(),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if mode == self.mode {
            return;
        }
        // Whatever is on screen when leaving becomes the on-air bank.
        match self.mode {
            Mode::Transition => self.ab.cancel(),
            Mode::Sequence => {
                if let Some(seq) = &mut self.sequence {
                    let shown = *seq.displayed_cue();
                    seq.stop();
                    self.ab.banks[self.ab.on_air] = shown;
                }
            }
            Mode::Live => {}
        }
        match mode {
            Mode::Transition => self.ab.sync_off_air(),
            Mode::Sequence => {
                let on_air = self.ab.banks[self.ab.on_air];
                self.sequence.get_or_insert_with(|| Sequence::new(on_air));
            }
            Mode::Live => {}
        }
        self.mode = mode;
    }

    /// The parameters the panel edits in the current mode.
    pub fn editable(&mut self) -> &mut Params {
        match self.mode {
            Mode::Live => &mut self.ab.banks[self.ab.on_air],
            Mode::Transition => {
                let off = self.ab.off_air();
                &mut self.ab.banks[off]
            }
            Mode::Sequence => &mut self.sequence_mut().selected_cue_mut().params,
        }
    }

    pub fn sequence_mut(&mut self) -> &mut Sequence {
        let on_air = self.ab.banks[self.ab.on_air];
        self.sequence.get_or_insert_with(|| Sequence::new(on_air))
    }

    /// Transition button / Space: start or reverse the A/B ramp (Transition mode only).
    pub fn trigger(&mut self) {
        if self.mode == Mode::Transition && self.ab.trigger() {
            self.target = self.rest;
        }
    }

    /// Cut button: jump to the destination bank (Transition mode only).
    pub fn cut(&mut self) {
        if self.mode != Mode::Transition {
            return;
        }
        let lands_on_target = self.ab.ramp().is_some_and(|r| r.forward);
        self.ab.cut();
        if lands_on_target {
            self.rest = self.target;
        }
    }

    /// Advances ramps and phase clocks by `dt` seconds of animation time.
    pub fn advance(&mut self, dt: f32) {
        match self.mode {
            Mode::Live => self.rest.advance(&self.ab.banks[self.ab.on_air], dt),
            Mode::Transition => {
                self.rest.advance(&self.ab.banks[self.ab.on_air], dt);
                if self.ab.ramp().is_some() {
                    self.target.advance(&self.ab.banks[self.ab.off_air()], dt);
                }
                if self.ab.advance(dt) == Some(AbEvent::Finished) {
                    self.rest = self.target;
                }
            }
            Mode::Sequence => {
                let seq = self
                    .sequence
                    .get_or_insert_with(|| Sequence::new(self.ab.banks[self.ab.on_air]));
                for event in seq.advance(dt) {
                    match event {
                        SeqEvent::RampStarted => self.target = self.rest,
                        SeqEvent::RampFinished => self.rest = self.target,
                        SeqEvent::Restarted => {}
                    }
                }
                match seq.view() {
                    SeqView::Rest(p) => self.rest.advance(p, dt),
                    SeqView::Ramp { from, to, .. } => {
                        self.rest.advance(from, dt);
                        self.target.advance(to, dt);
                    }
                }
            }
        }
    }

    /// What the renderer draws this frame.
    pub fn frame(&self) -> FrameParams {
        let rest = |p: &Params| blend(p, p, 0.0, None, &self.rest, &self.rest);
        match self.mode {
            Mode::Live => rest(&self.ab.banks[self.ab.on_air]),
            Mode::Transition => match self.ab.ramp() {
                Some(ramp) => blend(
                    &self.ab.banks[self.ab.on_air],
                    &self.ab.banks[self.ab.off_air()],
                    self.curves.eval(self.ab.curve, ramp.progress),
                    Some(ramp.progress),
                    &self.rest,
                    &self.target,
                ),
                None => rest(&self.ab.banks[self.ab.on_air]),
            },
            Mode::Sequence => match self.sequence.as_ref().map(Sequence::view) {
                Some(SeqView::Ramp {
                    from,
                    to,
                    progress,
                    curve,
                }) => blend(
                    from,
                    to,
                    self.curves.eval(curve, progress),
                    Some(progress),
                    &self.rest,
                    &self.target,
                ),
                Some(SeqView::Rest(p)) => rest(p),
                None => rest(&self.ab.banks[self.ab.on_air]),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_mode_edits_what_is_shown() {
        let mut m = Motion::new(Params::default());
        m.editable().warp.zoom = 2.5;
        assert_eq!(m.frame().warp.zoom, 2.5);
    }

    #[test]
    fn transition_mode_edits_off_air_until_triggered() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Transition);
        m.editable().warp.zoom = 3.0;
        assert_eq!(m.frame().warp.zoom, 1.0, "edits stay off air");
        m.trigger();
        m.advance(1.0); // halfway through the default 2 s S-curve
        assert!((m.frame().warp.zoom - 2.0).abs() < 1e-5);
        m.advance(1.0);
        assert_eq!(m.frame().warp.zoom, 3.0);
        assert_eq!(m.editable().warp.zoom, 1.0, "now editing the other bank");
    }

    #[test]
    fn trigger_and_cut_do_nothing_outside_transition_mode() {
        let mut m = Motion::new(Params::default());
        m.trigger();
        m.cut();
        assert!(m.ab.ramp().is_none());
        assert_eq!(m.ab.on_air, 0);
    }

    #[test]
    fn oscillator_phase_is_continuous_when_a_transition_finishes() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Transition);
        m.editable().warp.oscillators[0].phase_speed = 2.0;
        m.advance(0.3);
        m.trigger();
        m.advance(1.99);
        let before = m.frame().warp.oscillators[0].phase;
        m.advance(0.02); // finishes; the destination's clocks take over
        let after = m.frame().warp.oscillators[0].phase;
        let step = (after - before).rem_euclid(1.0);
        assert!(step < 0.1, "phase jumped by {step}");
    }

    #[test]
    fn leaving_sequence_puts_shown_cue_on_air() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Sequence);
        m.editable().warp.zoom = 1.7;
        m.set_mode(Mode::Live);
        assert_eq!(m.frame().warp.zoom, 1.7);
    }

    #[test]
    fn cues_survive_switching_modes() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Sequence);
        m.sequence_mut().add_cue();
        m.set_mode(Mode::Live);
        m.set_mode(Mode::Sequence);
        assert_eq!(m.sequence_mut().cues().len(), 2);
    }

    #[test]
    fn sequence_plays_its_cues() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Sequence);
        let seq = m.sequence_mut();
        seq.add_cue();
        seq.selected_cue_mut().params.warp.zoom = 2.0;
        seq.set_start_frame(1, 24);
        seq.set_duration(1, 24);
        seq.run();
        m.advance(1.0); // ramp to cue 2 starts at frame 24
        m.advance(1.0); // and ends at frame 48
        assert_eq!(m.frame().warp.zoom, 2.0);
    }
}
```

- [ ] **Step 5: Run the tests and confirm they pass.** Run: `cargo test --lib motion::`. Expected: `7 passed`.

- [ ] **Step 6: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/motion.rs src/lib.rs
git commit -m "feat: add Motion controller tying modes, ramps and clocks together" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Panel, curve editor, and app wiring

**Files:**
- Create: `src/curve_editor.rs`
- Modify: `src/lib.rs`, `src/ui.rs`, `src/app.rs`
- Test: unit tests in `src/curve_editor.rs`; manual checklist

**Interfaces:**
- Consumes: `Motion`, `Mode` (Task 7); `CurveLibrary`, `CurveRef`, `CustomCurve`, `Y_RANGE` (Task 2); `AbState::bank_name`, `DURATION` (Task 3); `FRAMES_PER_SECOND`, `MAX_FRAME` (Task 4); `OscSync`, `Envelope`, `Oscillator` (Task 1).
- Produces:
  - `curve_editor(ui, &mut CustomCurve) -> egui::Response`, plus the testable helpers `to_screen`, `from_screen` and `nearest_point`;
  - `ui::draw(ui, &mut Motion, &mut UiState) -> UiActions`. `UiState` gains `editing_curve: Option<u32>`.
  - The app holds `motion: Motion` in place of `params` and `clocks`. Space triggers a transition when egui isn't using the keyboard.

> **egui 0.36 API notes:** the curve editor uses `ui.allocate_painter(size, Sense::click_and_drag())`, `painter.rect_stroke(rect, radius, stroke, StrokeKind::Inside)` (the stroke kind argument is required), and `ui.memory_mut(|m| m.data.insert_temp(id, value))` to remember which point is being dragged. The keyboard check is `egui_ctx.egui_wants_keyboard_input()` (renamed from `wants_keyboard_input`).

- [ ] **Step 1: Register the module.** Replace `src/lib.rs` with:

```rust
//! Rasterwarp: real-time analog video-synthesis animation.

pub mod app;
pub mod blend;
pub mod curve;
pub mod curve_editor;
pub mod gpu;
pub mod motion;
pub mod params;
pub mod passes;
pub mod sequence;
pub mod source;
pub mod transition;
pub mod ui;
```

- [ ] **Step 2: Write the failing tests.** Create `src/curve_editor.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), SIZE)
    }

    #[test]
    fn screen_mapping_round_trips() {
        for p in [[0.0, 0.0], [1.0, 1.0], [0.3, -0.4], [0.9, 1.4]] {
            let back = from_screen(rect(), to_screen(rect(), p));
            assert!((back[0] - p[0]).abs() < 1e-4 && (back[1] - p[1]).abs() < 1e-4);
        }
    }

    #[test]
    fn y_increases_upward() {
        let low = to_screen(rect(), [0.5, 0.0]);
        let high = to_screen(rect(), [0.5, 1.0]);
        assert!(high.y < low.y);
    }

    #[test]
    fn grabs_only_nearby_points() {
        let points = [[0.25, 0.5], [0.75, 0.5]];
        let near = to_screen(rect(), points[1]) + Vec2::new(3.0, 3.0);
        let far = to_screen(rect(), [0.5, 1.4]);
        assert_eq!(nearest_point(rect(), &points, near), Some(1));
        assert_eq!(nearest_point(rect(), &points, far), None);
    }
}
```

- [ ] **Step 3: Run them and confirm they fail.** Run: `cargo test --lib curve_editor::`. Expected: compile errors such as "cannot find function `to_screen`".

- [ ] **Step 4: Implement the curve editor.** Replace `src/curve_editor.rs` with:

```rust
//! Interactive editor for a user curve: drag points, double-click to add,
//! right-click to remove.

use egui::{Color32, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2};

use crate::curve::{CustomCurve, Y_RANGE};

const SIZE: Vec2 = Vec2::new(260.0, 160.0);
/// How close (in points) the pointer must be to grab a curve point.
const GRAB_RADIUS: f32 = 8.0;

/// Curve space (x in 0..1, y in the curve's y range, y up) to screen space.
pub fn to_screen(rect: Rect, p: [f32; 2]) -> Pos2 {
    let (y0, y1) = (*Y_RANGE.start(), *Y_RANGE.end());
    pos2(
        rect.left() + p[0] * rect.width(),
        rect.bottom() - (p[1] - y0) / (y1 - y0) * rect.height(),
    )
}

/// Screen space back to curve space.
pub fn from_screen(rect: Rect, pos: Pos2) -> [f32; 2] {
    let (y0, y1) = (*Y_RANGE.start(), *Y_RANGE.end());
    [
        (pos.x - rect.left()) / rect.width(),
        y0 + (rect.bottom() - pos.y) / rect.height() * (y1 - y0),
    ]
}

/// Index of the interior point nearest `pos`, if within the grab radius.
pub fn nearest_point(rect: Rect, points: &[[f32; 2]], pos: Pos2) -> Option<usize> {
    points
        .iter()
        .enumerate()
        .map(|(i, p)| (i, to_screen(rect, *p).distance(pos)))
        .filter(|(_, d)| *d <= GRAB_RADIUS)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

pub fn curve_editor(ui: &mut Ui, curve: &mut CustomCurve) -> Response {
    let (response, painter) = ui.allocate_painter(SIZE, Sense::click_and_drag());
    let rect = response.rect;
    let drag_id = response.id.with("dragging");

    if response.drag_started()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let grabbed = nearest_point(rect, curve.points(), pos);
        ui.memory_mut(|m| m.data.insert_temp(drag_id, grabbed));
    }
    let dragging: Option<usize> = ui.memory(|m| m.data.get_temp(drag_id)).flatten();
    if let (true, Some(i), Some(pos)) = (
        response.dragged(),
        dragging,
        response.interact_pointer_pos(),
    ) {
        let [x, y] = from_screen(rect, pos);
        curve.move_point(i, x, y);
    }
    if response.drag_stopped() {
        ui.memory_mut(|m| m.data.remove_temp::<Option<usize>>(drag_id));
    }
    if response.double_clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let [x, y] = from_screen(rect, pos);
        curve.insert(x, y);
    }
    if response.secondary_clicked()
        && let Some(pos) = response.interact_pointer_pos()
        && let Some(i) = nearest_point(rect, curve.points(), pos)
    {
        curve.remove(i);
    }

    let visuals = ui.visuals();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    painter.rect_stroke(
        rect,
        2.0,
        visuals.widgets.noninteractive.bg_stroke,
        StrokeKind::Inside,
    );
    let guide = Stroke::new(1.0, visuals.weak_text_color());
    for y in [0.0, 1.0] {
        let a = to_screen(rect, [0.0, y]);
        let b = to_screen(rect, [1.0, y]);
        painter.line_segment([a, b], guide);
    }
    let line: Vec<Pos2> = (0..=128)
        .map(|i| {
            let x = i as f32 / 128.0;
            to_screen(rect, [x, curve.eval(x)])
        })
        .collect();
    painter.line(line, Stroke::new(2.0, Color32::from_rgb(255, 170, 60)));
    for p in curve.all_points() {
        painter.circle_filled(to_screen(rect, p), 4.0, visuals.strong_text_color());
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), SIZE)
    }

    #[test]
    fn screen_mapping_round_trips() {
        for p in [[0.0, 0.0], [1.0, 1.0], [0.3, -0.4], [0.9, 1.4]] {
            let back = from_screen(rect(), to_screen(rect(), p));
            assert!((back[0] - p[0]).abs() < 1e-4 && (back[1] - p[1]).abs() < 1e-4);
        }
    }

    #[test]
    fn y_increases_upward() {
        let low = to_screen(rect(), [0.5, 0.0]);
        let high = to_screen(rect(), [0.5, 1.0]);
        assert!(high.y < low.y);
    }

    #[test]
    fn grabs_only_nearby_points() {
        let points = [[0.25, 0.5], [0.75, 0.5]];
        let near = to_screen(rect(), points[1]) + Vec2::new(3.0, 3.0);
        let far = to_screen(rect(), [0.5, 1.4]);
        assert_eq!(nearest_point(rect(), &points, near), Some(1));
        assert_eq!(nearest_point(rect(), &points, far), None);
    }
}
```

- [ ] **Step 5: Run the tests and confirm they pass.** Run: `cargo test --lib curve_editor::`. Expected: `3 passed`.

- [ ] **Step 6: Replace the panel.** Replace `src/ui.rs` with:

```rust
//! The egui parameter panel.

use egui::{CollapsingHeader, ComboBox, ProgressBar, Slider, Ui};

use crate::curve::{CurveLibrary, CurveRef};
use crate::curve_editor::curve_editor;
use crate::motion::{Mode, Motion};
use crate::params::{Axis, Envelope, OscInput, OscSync, Oscillator, Params, Waveform, ranges};
use crate::sequence::{FRAMES_PER_SECOND, MAX_FRAME};
use crate::transition::{AbState, DURATION};

/// UI-only state that isn't a render parameter.
#[derive(Default)]
pub struct UiState {
    pub paused: bool,
    /// Smoothed frame time in milliseconds.
    pub frame_ms: f32,
    pub source_info: String,
    pub load_error: Option<String>,
    /// The user curve open in the curve editor.
    pub editing_curve: Option<u32>,
}

/// One-shot actions requested by the user this frame.
#[derive(Default)]
pub struct UiActions {
    pub clear_feedback: bool,
}

pub fn draw(ui: &mut Ui, motion: &mut Motion, state: &mut UiState) -> UiActions {
    let mut actions = UiActions::default();
    egui::Panel::left("controls")
        .resizable(true)
        .default_size(320.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Rasterwarp");
                ui.label(format!(
                    "{:.2} ms ({:.0} fps)",
                    state.frame_ms,
                    1000.0 / state.frame_ms.max(0.001)
                ));
                ui.label(&state.source_info);
                if let Some(err) = &state.load_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, err);
                }
                ui.label("Drop a PNG/JPG onto the window to load it.");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut state.paused, "Pause");
                    if ui.button("Reset all").clicked() {
                        *motion.editable() = Params::default();
                        actions.clear_feedback = true;
                    }
                });
                mode_section(ui, motion);
                curves_section(ui, &mut motion.curves, state);
                let params = motion.editable();
                warp_section(ui, params);
                colorize_section(ui, params);
                feedback_section(ui, params, &mut actions);
                glow_section(ui, params);
            });
        });
    actions
}

fn mode_section(ui: &mut Ui, motion: &mut Motion) {
    ui.separator();
    ui.horizontal(|ui| {
        for mode in Mode::ALL {
            if ui
                .selectable_label(motion.mode() == mode, format!("{mode:?}"))
                .clicked()
            {
                motion.set_mode(mode);
            }
        }
    });
    match motion.mode() {
        Mode::Live => {
            ui.label("Editing what's on screen.");
        }
        Mode::Transition => transition_controls(ui, motion),
        Mode::Sequence => sequence_controls(ui, motion),
    }
    ui.separator();
}

fn transition_controls(ui: &mut Ui, motion: &mut Motion) {
    let ab = &motion.ab;
    ui.label(format!(
        "On air: {} · editing: {}",
        AbState::bank_name(ab.on_air),
        AbState::bank_name(ab.off_air())
    ));
    let label = match ab.ramp() {
        Some(r) if r.forward => "Reverse (Space)",
        Some(_) => "Resume (Space)",
        None => "Transition (Space)",
    };
    ui.horizontal(|ui| {
        if ui.button(label).clicked() {
            motion.trigger();
        }
        if ui.button("Cut").clicked() {
            motion.cut();
        }
    });
    let progress = motion.ab.ramp().map_or(0.0, |r| r.progress);
    ui.add(ProgressBar::new(progress).show_percentage());
    ui.add(
        Slider::new(&mut motion.ab.duration, DURATION)
            .text("duration (s)")
            .logarithmic(true),
    );
    curve_picker(ui, "ab curve", &motion.curves, &mut motion.ab.curve);
}

fn sequence_controls(ui: &mut Ui, motion: &mut Motion) {
    let curves = motion.curves.clone();
    let seq = motion.sequence_mut();
    ui.horizontal(|ui| {
        if seq.is_running() {
            if ui.button("Stop").clicked() {
                seq.stop();
            }
        } else if ui.button("Run").clicked() {
            seq.run();
        }
        if ui.button("Reset").clicked() {
            seq.reset();
        }
        ui.checkbox(&mut seq.looping, "Loop");
    });
    let frame = seq.clock_frames();
    ui.label(format!(
        "Frame {:.0} ({:.2} s at {FRAMES_PER_SECOND} fps)",
        frame,
        frame / FRAMES_PER_SECOND
    ));
    for i in 0..seq.cues().len() {
        let text = format!("Cue {} @ frame {}", i + 1, seq.cues()[i].start_frame);
        if ui.selectable_label(seq.selected == i, text).clicked() {
            seq.selected = i;
        }
    }
    ui.horizontal(|ui| {
        if ui.button("Add cue").clicked() {
            seq.add_cue();
        }
        if ui.button("Delete cue").clicked() {
            seq.delete_cue();
        }
    });
    let i = seq.selected;
    let mut start = seq.cues()[i].start_frame;
    let mut duration = seq.cues()[i].duration_frames;
    ui.add_enabled_ui(i > 0, |ui| {
        if ui
            .add(Slider::new(&mut start, 0..=MAX_FRAME).text("start frame"))
            .changed()
        {
            seq.set_start_frame(i, start);
        }
        if ui
            .add(Slider::new(&mut duration, 1..=MAX_FRAME).text("ramp frames"))
            .changed()
        {
            seq.set_duration(i, duration);
        }
        curve_picker(ui, "cue curve", &curves, &mut seq.selected_cue_mut().curve);
    });
    if seq.is_running() {
        ui.label("Running: the panel still edits the selected cue.");
    }
}

fn curve_picker(ui: &mut Ui, id: &str, curves: &CurveLibrary, curve: &mut CurveRef) {
    ComboBox::from_id_salt(id)
        .selected_text(format!("curve: {}", curves.name(*curve)))
        .show_ui(ui, |ui| {
            for choice in curves.choices() {
                ui.selectable_value(curve, choice, curves.name(choice));
            }
        });
}

fn curves_section(ui: &mut Ui, curves: &mut CurveLibrary, state: &mut UiState) {
    CollapsingHeader::new("Curves").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for c in &curves.custom {
                if ui
                    .selectable_label(state.editing_curve == Some(c.id), &c.name)
                    .clicked()
                {
                    state.editing_curve = Some(c.id);
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("New curve").clicked() {
                state.editing_curve = curves.add().or(state.editing_curve);
            }
            if let Some(id) = state.editing_curve
                && ui.button("Delete").clicked()
            {
                curves.remove(id);
                state.editing_curve = None;
            }
        });
        if let Some(curve) = state.editing_curve.and_then(|id| curves.get_mut(id)) {
            ui.text_edit_singleline(&mut curve.name);
            curve_editor(ui, curve);
            ui.small("Drag points · double-click to add · right-click to remove");
        }
    });
}

fn warp_section(ui: &mut Ui, params: &mut Params) {
    let warp = &mut params.warp;
    CollapsingHeader::new("Deflection")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(Slider::new(&mut warp.zoom, ranges::ZOOM).text("zoom"));
            ui.add(Slider::new(&mut warp.rotation, ranges::ROTATION).text("rotation"));
            ui.add(Slider::new(&mut warp.offset[0], ranges::OFFSET).text("offset x"));
            ui.add(Slider::new(&mut warp.offset[1], ranges::OFFSET).text("offset y"));
            ui.add(Slider::new(&mut warp.drift, ranges::DRIFT).text("analog drift"));
            ui.checkbox(
                &mut warp.slave_4_to_3,
                "Slave osc 4 to osc 3 (sine/cosine pair)",
            );
            let slaved = warp.slave_4_to_3;
            for (i, osc) in warp.oscillators.iter_mut().enumerate() {
                // A slaved oscillator 4 keeps only its own target, amplitude and envelope.
                let follows = slaved && i == 3;
                CollapsingHeader::new(format!("Oscillator {}", i + 1))
                    .default_open(i == 0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut osc.target, Axis::X, "→ X");
                            ui.selectable_value(&mut osc.target, Axis::Y, "→ Y");
                            ComboBox::from_id_salt(("envelope", i))
                                .selected_text(format!("{:?}", osc.envelope))
                                .show_ui(ui, |ui| {
                                    for e in Envelope::ALL {
                                        ui.selectable_value(&mut osc.envelope, e, format!("{e:?}"));
                                    }
                                });
                        });
                        ui.add(
                            Slider::new(&mut osc.amplitude, ranges::AMPLITUDE).text("amplitude"),
                        );
                        ui.add_enabled_ui(!follows, |ui| oscillator_shape(ui, i, osc));
                    });
            }
        });
}

/// The controls a slaved oscillator 4 takes from oscillator 3.
fn oscillator_shape(ui: &mut Ui, i: usize, osc: &mut Oscillator) {
    ui.horizontal(|ui| {
        ComboBox::from_id_salt(("waveform", i))
            .selected_text(format!("{:?}", osc.waveform))
            .show_ui(ui, |ui| {
                for w in Waveform::ALL {
                    ui.selectable_value(&mut osc.waveform, w, format!("{w:?}"));
                }
            });
        ComboBox::from_id_salt(("input", i))
            .selected_text(format!("by {:?}", osc.input))
            .show_ui(ui, |ui| {
                for input in OscInput::ALL {
                    ui.selectable_value(&mut osc.input, input, format!("by {input:?}"));
                }
            });
        ComboBox::from_id_salt(("sync", i))
            .selected_text(format!("{:?} sync", osc.sync))
            .show_ui(ui, |ui| {
                for sync in OscSync::ALL {
                    ui.selectable_value(&mut osc.sync, sync, format!("{sync:?} sync"));
                }
            });
    });
    ui.add(Slider::new(&mut osc.frequency, ranges::FREQUENCY).text("frequency"));
    ui.add(Slider::new(&mut osc.phase, ranges::PHASE).text("phase"));
    ui.add(Slider::new(&mut osc.phase_speed, ranges::PHASE_SPEED).text("phase speed"));
    ui.add(Slider::new(&mut osc.lfo_rate, ranges::LFO_RATE).text("LFO rate"));
    ui.add(Slider::new(&mut osc.lfo_depth, ranges::LFO_DEPTH).text("LFO depth"));
}

fn colorize_section(ui: &mut Ui, params: &mut Params) {
    let c = &mut params.colorize;
    CollapsingHeader::new("Colorize")
        .default_open(true)
        .show(ui, |ui| {
            ui.checkbox(&mut c.bypass, "Bypass (grayscale)");
            ui.add(Slider::new(&mut c.levels, ranges::LEVELS).text("levels"));
            ui.add(Slider::new(&mut c.softness, ranges::SOFTNESS).text("softness"));
            ui.add(Slider::new(&mut c.cycle_speed, ranges::CYCLE_SPEED).text("cycle speed"));
            ui.horizontal_wrapped(|ui| {
                for color in &mut c.palette {
                    ui.color_edit_button_rgb(color);
                }
            });
        });
}

fn feedback_section(ui: &mut Ui, params: &mut Params, actions: &mut UiActions) {
    let f = &mut params.feedback;
    CollapsingHeader::new("Feedback")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(Slider::new(&mut f.amount, ranges::FEEDBACK_AMOUNT).text("amount"));
            ui.add(Slider::new(&mut f.zoom, ranges::FEEDBACK_ZOOM).text("zoom"));
            ui.add(Slider::new(&mut f.rotation, ranges::FEEDBACK_ROTATION).text("rotation"));
            ui.add(Slider::new(&mut f.offset[0], ranges::FEEDBACK_OFFSET).text("offset x"));
            ui.add(Slider::new(&mut f.offset[1], ranges::FEEDBACK_OFFSET).text("offset y"));
            if ui.button("Clear trails").clicked() {
                actions.clear_feedback = true;
            }
        });
}

fn glow_section(ui: &mut Ui, params: &mut Params) {
    let g = &mut params.glow;
    CollapsingHeader::new("Glow & CRT")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(Slider::new(&mut g.bloom_intensity, ranges::BLOOM_INTENSITY).text("bloom"));
            ui.add(
                Slider::new(&mut g.bloom_threshold, ranges::BLOOM_THRESHOLD)
                    .text("bloom threshold"),
            );
            ui.add(
                Slider::new(&mut g.scanline_strength, ranges::SCANLINE_STRENGTH).text("scanlines"),
            );
            ui.add(
                Slider::new(&mut g.scanline_count, ranges::SCANLINE_COUNT).text("scanline count"),
            );
            ui.add(Slider::new(&mut g.chroma, ranges::CHROMA).text("chroma bleed"));
            ui.add(Slider::new(&mut g.noise, ranges::NOISE).text("noise"));
        });
}
```

- [ ] **Step 7: Wire Motion into the app.** Replace `src/app.rs` with:

```rust
//! The windowed application: owns the GPU context, renderer, parameters and UI,
//! and runs one frame per redraw.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::gpu;
use crate::motion::Motion;
use crate::params::Params;
use crate::passes::Renderer;
use crate::source::{self, GrayImage};
use crate::ui::{self, UiActions, UiState};

/// Fixed internal render resolution, independent of the window size.
pub const INTERNAL_SIZE: (u32, u32) = (1920, 1080);

pub struct App {
    initial_image: Option<PathBuf>,
    state: Option<State>,
    error: Option<anyhow::Error>,
}

impl App {
    pub fn new(initial_image: Option<PathBuf>) -> Self {
        Self {
            initial_image,
            state: None,
            error: None,
        }
    }

    /// The startup error, if the app exited because it could not start.
    pub fn finish(self) -> Result<()> {
        match self.error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match State::new(event_loop, self.initial_image.as_deref()) {
            Ok(state) => self.state = Some(state),
            Err(err) => {
                self.error = Some(err);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.state else { return };
        let response = state.egui_state.on_window_event(&state.window, &event);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.resize(size),
            WindowEvent::DroppedFile(path) => state.load_source(&path),
            // Space triggers a transition unless egui is using the keyboard (e.g. a text field).
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && !event.repeat
                    && event.logical_key == Key::Named(NamedKey::Space)
                    && !response.consumed
                    && !state.egui_ctx.egui_wants_keyboard_input() =>
            {
                state.motion.trigger();
            }
            WindowEvent::RedrawRequested => {
                state.redraw();
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    max_texture_side: u32,
    composite_format: wgpu::TextureFormat,
    renderer: Renderer,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    motion: Motion,
    ui: UiState,
    time: f64,
    last_frame: Instant,
}

impl State {
    fn new(event_loop: &ActiveEventLoop, initial_image: Option<&Path>) -> Result<Self> {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Rasterwarp")
                        .with_inner_size(LogicalSize::new(1280.0, 720.0)),
                )
                .context("failed to create window")?,
        );
        let backends = gpu::backends_from_env()?;
        let instance = gpu::create_instance(backends);
        let surface = instance
            .create_surface(window.clone())
            .with_context(|| {
                format!(
                    "failed to create window surface for backend {backends:?}; try RASTERWARP_BACKEND=dx12 or gl"
                )
            })?;
        let (adapter, device, queue) =
            pollster::block_on(gpu::request_device(&instance, backends, Some(&surface)))?;
        let max_texture_side = device.limits().max_texture_dimension_2d;

        let caps = surface.get_capabilities(&adapter);
        // The swapchain itself is non-sRGB (what egui expects). Where the adapter allows an
        // sRGB view of it, the composite pass draws through that view; otherwise (e.g. GL)
        // it draws into the plain format and encodes sRGB in the shader.
        let format = caps.formats[0].remove_srgb_suffix();
        let srgb_format = format.add_srgb_suffix();
        let srgb_view_ok = adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::SURFACE_VIEW_FORMATS);
        let composite_format = if srgb_view_ok { srgb_format } else { format };
        // Fifo (vsync) keeps the per-frame feedback semantics at 60 fps. RASTERWARP_UNCAPPED=1
        // opts into Mailbox (uncapped, no tearing) to measure the real render cost.
        let uncapped = std::env::var("RASTERWARP_UNCAPPED").is_ok_and(|v| v == "1");
        let present_mode = if uncapped && caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .context("window surface is not supported by the GPU adapter")?;
        config.format = format;
        config.view_formats = if srgb_view_ok {
            vec![srgb_format]
        } else {
            vec![]
        };
        config.present_mode = present_mode;
        if size.width > 0 && size.height > 0 {
            surface.configure(&device, &config);
        }
        log::info!("surface: {format:?} (composite via {composite_format:?}), {present_mode:?}");

        let mut ui = UiState {
            frame_ms: 16.7,
            ..Default::default()
        };
        let image = match initial_image {
            Some(path) => match load_fitting(path, max_texture_side) {
                Ok(image) => {
                    ui.source_info = describe(&path.display().to_string(), &image);
                    image
                }
                Err(err) => {
                    log::warn!("{err:#}");
                    ui.load_error = Some(format!("{err:#}"));
                    test_card(&mut ui)
                }
            },
            None => test_card(&mut ui),
        };
        let renderer = Renderer::new(&device, &queue, composite_format, INTERNAL_SIZE, &image);

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui_ctx.viewport_id(),
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(max_texture_side as usize),
        );
        let egui_renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());

        Ok(Self {
            window,
            surface,
            config,
            device,
            queue,
            max_texture_side,
            composite_format,
            renderer,
            egui_ctx,
            egui_state,
            egui_renderer,
            motion: Motion::new(Params::default()),
            ui,
            time: 0.0,
            last_frame: Instant::now(),
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = size.width;
        self.config.height = size.height;
        if size.width > 0 && size.height > 0 {
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn load_source(&mut self, path: &Path) {
        match load_fitting(path, self.max_texture_side) {
            Ok(image) => {
                self.renderer.set_source(&self.device, &self.queue, &image);
                self.ui.source_info = describe(&path.display().to_string(), &image);
                self.ui.load_error = None;
            }
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.load_error = Some(format!("{err:#}"));
            }
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.ui.frame_ms += (dt * 1000.0 - self.ui.frame_ms) * 0.05;
        if !self.ui.paused {
            // Clamp so a stall (e.g. dragging the window) doesn't make animation jump.
            let step = dt.min(0.1);
            self.time += f64::from(step);
            self.motion.advance(step);
        }

        if self.config.width == 0 || self.config.height == 0 {
            return; // minimized
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("validation error acquiring the surface texture");
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let srgb_view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.composite_format),
            ..Default::default()
        });
        let output_size = (self.config.width, self.config.height);

        let raw_input = self.egui_state.take_egui_input(&self.window);
        let mut actions = UiActions::default();
        let egui_output = self.egui_ctx.run_ui(raw_input, |ui| {
            actions = ui::draw(ui, &mut self.motion, &mut self.ui)
        });
        self.egui_state
            .handle_platform_output(&self.window, egui_output.platform_output);
        let jobs = self
            .egui_ctx
            .tessellate(egui_output.shapes, egui_output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [output_size.0, output_size.1],
            pixels_per_point: egui_output.pixels_per_point,
        };
        for (id, deltas) in &egui_output.textures_delta.set {
            for delta in deltas {
                self.egui_renderer
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
        }

        let frame_params = self.motion.frame();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        if actions.clear_feedback {
            self.renderer.clear_feedback(&mut encoder);
        }
        self.renderer.render(
            &self.device,
            &self.queue,
            &mut encoder,
            &frame_params,
            self.time as f32,
            &srgb_view,
            output_size,
        );
        let egui_commands = self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &jobs,
            &screen,
        );
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.egui_renderer.render(&mut pass, &jobs, &screen);
        }
        self.queue
            .submit(egui_commands.into_iter().chain([encoder.finish()]));
        self.window.pre_present_notify();
        self.queue.present(frame);
        for id in &egui_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
    }
}

fn test_card(ui: &mut UiState) -> GrayImage {
    let image = source::test_card(1600, 900);
    ui.source_info = describe("built-in test card", &image);
    image
}

fn describe(name: &str, image: &GrayImage) -> String {
    format!("Source: {name} ({}×{})", image.width, image.height)
}

/// Loads an image and checks it fits in a GPU texture.
fn load_fitting(path: &Path, max_side: u32) -> anyhow::Result<GrayImage> {
    let image = source::load_image(path)?;
    source::ensure_fits(&image, max_side)
        .with_context(|| format!("could not load {}", path.display()))?;
    Ok(image)
}
```

- [ ] **Step 8: Run everything.** Run: `cargo test`. Expected: all unit tests (77) and the smoke test pass.

- [ ] **Step 9: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 10: Manual check (release).** Run: `cargo run --release`, then check each item:
  - [ ] Live mode: sliders change the output immediately, as before.
  - [ ] Transition mode:
    - The label reads "On air: A · editing: B". Changing zoom doesn't change the output.
    - **Transition** (or Space) ramps to the new zoom over the set duration along the chosen curve. The progress bar fills. Afterwards the label swaps.
    - Pressing Space mid-ramp reverses it smoothly. **Cut** jumps.
  - [ ] Change oscillator 1's waveform in the off-air bank and transition: the two waveforms crossfade, with no snap.
  - [ ] Change phase speed in the off-air bank and transition: the ripple speeds up smoothly, with no flicker or sweep.
  - [ ] Curves: **New curve** opens the editor. Drag the point, double-click to add one, right-click to remove one. Select the curve in the transition's curve picker, and the transition follows it. A point dragged above 1 overshoots.
  - [ ] Sequence mode:
    - Add 2 cues with different zooms at frames 24 and 72. **Run** plays them in order.
    - **Loop** repeats. **Reset** jumps to cue 1. **Stop** returns to editing the selected cue.
  - [ ] Oscillator extras:
    - *Frame* sync freezes that oscillator's ripple.
    - *Slave osc 4 to osc 3* greys out oscillator 4's shape controls. Set osc 3 by Time → X and osc 4 → Y with equal amplitude to get circular motion.
    - A *Swell* oscillator is still at rest and wobbles only during a transition.
  - [ ] Typing in the curve-name text field doesn't trigger a transition with Space.

- [ ] **Step 11: Commit**

```bash
git add src/lib.rs src/curve_editor.rs src/ui.rs src/app.rs
git commit -m "feat: add mode, transition, sequence and curve-editor panel" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Done criteria (Plan 1)

- All spec Plan 1 behaviors work as in the Task 8 checklist.
- `cargo test` passes (77 unit tests + smoke), `cargo clippy --all-targets` is clean, and `cargo fmt` makes no changes.
- Phases stay continuous across transitions and rate changes (unit-tested in `blend` and `motion`).
