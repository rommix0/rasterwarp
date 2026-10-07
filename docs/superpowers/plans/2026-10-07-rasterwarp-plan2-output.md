# Rasterwarp Plan 2 (Output) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the off-air preview overlay, a user-settable canvas resolution, and video capture (HEVC NVENC 4:4:4 or FFV1 through the linked FFmpeg libraries) in real-time or frame-accurate offline mode, on top of Plan 1 (Motion) now on `main`.

**Architecture:**
- **Preview:** `Motion::preview()` says what the preview shows (the off-air bank, or the selected cue while a sequence runs). A second, small `Renderer` with its own trails draws it into a texture that egui shows in the bottom-right corner.
- **Canvas:** `Renderer::resize` rebuilds every canvas-sized target. The panel picks a preset or a custom size.
- **Capture:**
  - Each `Renderer` has a second composite pass that draws the clean frame into a canvas-sized capture texture.
  - A ring of 3 staging buffers reads it back asynchronously.
  - A `Recorder` decides which frames are due and feeds an encoder thread that converts and encodes them with `ffmpeg-next`.
  - Offline mode advances animation time by exactly 1/60 s per drawn frame and never drops a frame.

**Tech Stack:** Rust 1.98 (edition 2024), wgpu 30, winit 0.30, egui 0.36, ffmpeg-next 9 (linked to a shared FFmpeg 9.0.2 build), chrono 0.4, rfd 0.17.

**Spec:** `docs/superpowers/specs/2026-10-07-rasterwarp-motion-output-look-design.md` (Plan 2 section, plus Error handling and Testing)

## Global Constraints

- Platform: Windows 10, NVIDIA RTX 3090, Rust 1.98 stable MSVC, edition 2024.
- New crates, exactly these and only in the task that introduces them:
  - `ffmpeg-next = { version = "9.0.0", default-features = false, features = ["codec", "format", "software-scaling"] }` (Task 4);
  - `chrono = { version = "0.4", default-features = false, features = ["clock"] }` (Task 5);
  - `rfd = "0.17"` (Task 7).

  The other versions stay pinned: wgpu 30, winit 0.30, egui 0.36.
- FFmpeg is the shared build at `C:\Users\abart\Desktop\ffmpeg-9.0.2-full_build-shared`. bindgen uses the libclang at `C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Tools\Llvm\x64\bin`. Both paths live in `.cargo/config.toml` (Task 4).
- `cargo build`, `cargo test` and `cargo clippy --all-targets` must have zero warnings. Run `cargo fmt` before committing.
- Every commit message ends with the trailer line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Don't use the word "Scanimate" in user-facing text.
- Code in this plan was compiled, tested (108 unit tests, 3 capture tests, 5 GPU smoke tests) and run on the dev machine before the plan was written, including real recordings in both formats. Transcribe it exactly. Every task's end state was also built and tested on its own.

## Design decisions made while building (beyond the spec text)

1. **The preview texture is `Rgba8Unorm`, and the composite shader encodes sRGB.** egui-wgpu 0.36 treats texture bytes as already gamma-encoded. With an sRGB texture format the preview was decoded twice and its mid-tones came out too dark (orange showed as red). This was seen and fixed during the spike.
2. **The preview in Transition mode always shows the off-air bank**, the one being edited. During a forward ramp that is the destination. Its phase clocks restart from the output's phases whenever the preview's source changes (it appears, a bank swaps, or a cue changes), so it starts in step with the output.
3. **Animation time advances only for frames that are actually drawn**, after the surface texture is acquired. A skipped frame (for example while minimized) then can't put a gap in an offline recording.
4. **Offline capture skips frames whose animation time hasn't moved**, so Pause records nothing.
5. **"Stop after" counts whole frames.** 10/60 s as `f32` is slightly more than 10 frames and would otherwise record 11; the smoke test caught this.
6. **Capture draws a second composite** (its own `CompositePass` in each `Renderer`) into a canvas-sized `Rgba8UnormSrgb` texture after the main composite, at a fit of 1:1 with no UI. The trails and bloom are shared, so it costs one extra full-screen pass.
7. **File names use local time via `chrono`**, because the standard library has no local time.
8. **The encoder sets each packet's duration to one frame** (otherwise the MP4 muxer warns) and **declares 60 fps on the stream** (otherwise MKV files had no frame rate). It calls `av_frame_make_writable` before reusing its frame, because the encoder may still reference the previous frame's buffer.
9. **Closing the window finishes the recording first**, so the file is complete.
10. **Recording sizes:** HEVC at QP 18 4:4:4 of content with the default film-grain noise ran at about 320 Mbit/s (40 MB/s) at 1080p in the spike, because per-frame noise doesn't compress. FFV1 offline at 1080p ran at 0.52× realtime. Lower the noise slider for much smaller files.

## File Structure

| File | Responsibility |
|---|---|
| `src/motion.rs` (modify) | `PreviewSource`, `Preview`, `Motion::preview()`, preview phase clocks |
| `src/preview.rs` (new) | `PreviewView`: the preview `Renderer`, its texture and egui registration, `size_for` |
| `src/canvas.rs` (new) | Canvas presets, `sanitize`, `CanvasChoice` (the panel's canvas state) |
| `src/passes/mod.rs` (modify) | `Renderer::size`, `Renderer::resize`, `CAPTURE_FORMAT`, `Renderer::composite_capture` |
| `.cargo/config.toml`, `build.rs` (new) | FFmpeg and libclang paths; copying the FFmpeg DLLs next to the executables |
| `src/capture/mod.rs` (new) | `CaptureMode`, `file_name`, `FrameClock`, row padding helpers |
| `src/capture/encode.rs` (new) | `VideoFormat`, `Frame`, `Encoder` thread: conversion, encoding, muxing |
| `src/capture/readback.rs` (new) | `Ring` slot rotation and `Readback`: capture texture plus staging buffers |
| `src/capture/recorder.rs` (new) | `RecordSettings`, `RecordStatus`, `Recorder`: one recording from start to finish |
| `src/ui.rs`, `src/app.rs` (modify) | Preview overlay, Canvas section, Capture section; wiring all of it into the app |
| `tests/smoke.rs`, `tests/capture.rs` | GPU smoke tests (preview, resize, offline recording) and FFmpeg round-trip tests |

---

### Task 1: Off-air preview in Motion

**Files:**
- Modify: `src/motion.rs`
- Test: unit tests in `src/motion.rs`

**Interfaces:**
- Consumes: existing `Motion`, `Mode`, `AbState::bank_name`, `Sequence::{is_running, selected, cues}`, `Clocks`, `blend`, `FrameParams`.
- Produces:
  - `pub enum PreviewSource { Bank(usize), Cue(usize) }` with `label(self) -> String` ("Off-air: B", "Cue 2");
  - `pub struct Preview { pub source: PreviewSource, pub frame: FrameParams }`;
  - `Motion::preview(&self) -> Option<Preview>`: `None` in Live mode and while a sequence is stopped.
  - `Motion::advance` also advances the preview's own phase clocks. They restart from the output's clocks whenever the preview source changes.

- [ ] **Step 1: Write the failing tests.** In `src/motion.rs`, add these tests at the top of `mod tests`, just after `use super::*;`:

```rust
    #[test]
    fn live_mode_has_no_preview() {
        let m = Motion::new(Params::default());
        assert!(m.preview().is_none());
    }

    #[test]
    fn transition_preview_shows_the_off_air_bank() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Transition);
        m.editable().warp.zoom = 3.0;
        let preview = m.preview().expect("preview in Transition mode");
        assert_eq!(preview.source, PreviewSource::Bank(1));
        assert_eq!(preview.source.label(), "Off-air: B");
        assert_eq!(preview.frame.warp.zoom, 3.0);
        assert_eq!(m.frame().warp.zoom, 1.0, "the output is unchanged");
    }

    #[test]
    fn preview_shows_the_destination_then_the_new_off_air_bank() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Transition);
        m.editable().warp.zoom = 3.0;
        m.trigger();
        m.advance(1.0);
        assert_eq!(m.preview().unwrap().source, PreviewSource::Bank(1));
        m.advance(1.5); // the ramp finishes and B goes on air
        let preview = m.preview().unwrap();
        assert_eq!(preview.source, PreviewSource::Bank(0));
        assert_eq!(preview.frame.warp.zoom, 1.0);
    }

    #[test]
    fn sequence_preview_shows_the_selected_cue_only_while_running() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Sequence);
        let seq = m.sequence_mut();
        seq.add_cue();
        seq.selected_cue_mut().params.warp.zoom = 2.0;
        assert!(m.preview().is_none(), "stopped: the output shows the cue");
        m.sequence_mut().run();
        let preview = m.preview().expect("preview while running");
        assert_eq!(preview.source, PreviewSource::Cue(1));
        assert_eq!(preview.source.label(), "Cue 2");
        assert_eq!(preview.frame.warp.zoom, 2.0);
    }

    #[test]
    fn preview_starts_from_the_output_phases_then_runs_on_its_own() {
        let mut m = Motion::new(Params::default());
        m.editable().warp.oscillators[0].phase_speed = 0.4;
        m.advance(0.7);
        m.set_mode(Mode::Transition);
        m.advance(0.0); // the preview appears
        let phase = |f: &FrameParams| f.warp.oscillators[0].phase;
        assert_eq!(phase(&m.preview().unwrap().frame), phase(&m.frame()));
        m.editable().warp.oscillators[0].phase_speed = 0.0;
        m.advance(0.5);
        let still = phase(&m.preview().unwrap().frame);
        let moved = phase(&m.frame());
        // The output moved 0.4 × 0.5 = 0.2 cycles while the preview held still.
        let off = moved - still - 0.2;
        assert!((off - off.round()).abs() < 1e-4, "{moved} vs {still}");
    }
```

- [ ] **Step 2: Run them and confirm they fail.** Run: `cargo test --lib motion::`. Expected: compile errors such as "no method named `preview` found for struct `Motion`".

- [ ] **Step 3: Implement.** Replace `src/motion.rs` with:

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

/// What the off-air preview is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewSource {
    /// An A/B bank (an index into `AbState::banks`).
    Bank(usize),
    /// A sequence cue (an index into the cue list).
    Cue(usize),
}

impl PreviewSource {
    pub fn label(self) -> String {
        match self {
            PreviewSource::Bank(i) => format!("Off-air: {}", AbState::bank_name(i)),
            PreviewSource::Cue(i) => format!("Cue {}", i + 1),
        }
    }
}

/// The off-air preview for this frame.
pub struct Preview {
    pub source: PreviewSource,
    pub frame: FrameParams,
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
    /// Phases of the off-air preview, independent of the output.
    preview: Clocks,
    /// The preview source the `preview` clocks belong to.
    preview_shown: Option<PreviewSource>,
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
            preview: Clocks::default(),
            preview_shown: None,
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
        // A new preview source starts from the output's phases, then runs on its own.
        let preview = self.preview_params().map(|(source, p)| (source, *p));
        let source = preview.map(|(source, _)| source);
        if source != self.preview_shown {
            self.preview = self.rest;
            self.preview_shown = source;
        }
        if let Some((_, p)) = preview {
            self.preview.advance(&p, dt);
        }
    }

    /// What the off-air preview shows, if anything: the off-air bank in Transition mode,
    /// and the selected cue while a sequence runs.
    pub fn preview(&self) -> Option<Preview> {
        let (source, p) = self.preview_params()?;
        Some(Preview {
            source,
            frame: blend(p, p, 0.0, None, &self.preview, &self.preview),
        })
    }

    fn preview_params(&self) -> Option<(PreviewSource, &Params)> {
        match self.mode {
            Mode::Live => None,
            Mode::Transition => {
                let off = self.ab.off_air();
                Some((PreviewSource::Bank(off), &self.ab.banks[off]))
            }
            Mode::Sequence => {
                let seq = self.sequence.as_ref()?;
                if !seq.is_running() {
                    return None; // the output already shows the selected cue
                }
                let i = seq.selected;
                Some((PreviewSource::Cue(i), &seq.cues()[i].params))
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
    fn live_mode_has_no_preview() {
        let m = Motion::new(Params::default());
        assert!(m.preview().is_none());
    }

    #[test]
    fn transition_preview_shows_the_off_air_bank() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Transition);
        m.editable().warp.zoom = 3.0;
        let preview = m.preview().expect("preview in Transition mode");
        assert_eq!(preview.source, PreviewSource::Bank(1));
        assert_eq!(preview.source.label(), "Off-air: B");
        assert_eq!(preview.frame.warp.zoom, 3.0);
        assert_eq!(m.frame().warp.zoom, 1.0, "the output is unchanged");
    }

    #[test]
    fn preview_shows_the_destination_then_the_new_off_air_bank() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Transition);
        m.editable().warp.zoom = 3.0;
        m.trigger();
        m.advance(1.0);
        assert_eq!(m.preview().unwrap().source, PreviewSource::Bank(1));
        m.advance(1.5); // the ramp finishes and B goes on air
        let preview = m.preview().unwrap();
        assert_eq!(preview.source, PreviewSource::Bank(0));
        assert_eq!(preview.frame.warp.zoom, 1.0);
    }

    #[test]
    fn sequence_preview_shows_the_selected_cue_only_while_running() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Sequence);
        let seq = m.sequence_mut();
        seq.add_cue();
        seq.selected_cue_mut().params.warp.zoom = 2.0;
        assert!(m.preview().is_none(), "stopped: the output shows the cue");
        m.sequence_mut().run();
        let preview = m.preview().expect("preview while running");
        assert_eq!(preview.source, PreviewSource::Cue(1));
        assert_eq!(preview.source.label(), "Cue 2");
        assert_eq!(preview.frame.warp.zoom, 2.0);
    }

    #[test]
    fn preview_starts_from_the_output_phases_then_runs_on_its_own() {
        let mut m = Motion::new(Params::default());
        m.editable().warp.oscillators[0].phase_speed = 0.4;
        m.advance(0.7);
        m.set_mode(Mode::Transition);
        m.advance(0.0); // the preview appears
        let phase = |f: &FrameParams| f.warp.oscillators[0].phase;
        assert_eq!(phase(&m.preview().unwrap().frame), phase(&m.frame()));
        m.editable().warp.oscillators[0].phase_speed = 0.0;
        m.advance(0.5);
        let still = phase(&m.preview().unwrap().frame);
        let moved = phase(&m.frame());
        // The output moved 0.4 × 0.5 = 0.2 cycles while the preview held still.
        let off = moved - still - 0.2;
        assert!((off - off.round()).abs() < 1e-4, "{moved} vs {still}");
    }

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

    /// Signed shortest step between two wrapped phases.
    fn phase_step(before: f32, after: f32) -> f32 {
        (after - before + 0.5).rem_euclid(1.0) - 0.5
    }

    fn osc0_phase(m: &Motion) -> f32 {
        m.frame().warp.oscillators[0].phase
    }

    #[test]
    fn forward_cut_mid_ramp_hands_the_destination_clock_to_rest() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Transition);
        m.editable().warp.oscillators[0].phase_speed = 2.0;
        m.advance(0.3);
        m.trigger();
        m.advance(1.0); // halfway through the ramp
        m.cut();
        assert_eq!(m.ab.on_air, 1);
        // The destination's clock (0.15 at the trigger, then 2 cycles/s) is on screen.
        let expected = |secs: f64| (0.15 + 2.0 * secs).rem_euclid(1.0) as f32;
        assert!(phase_step(osc0_phase(&m), expected(1.0)).abs() < 1e-3);
        let mut last = osc0_phase(&m);
        for _ in 0..100 {
            m.advance(0.01);
            let now = osc0_phase(&m);
            assert!(phase_step(last, now).abs() < 0.1, "phase jumped after cut");
            last = now;
        }
        assert!(phase_step(last, expected(2.0)).abs() < 1e-3);
    }

    #[test]
    fn sequence_ramp_finishing_keeps_phase_continuous() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Sequence);
        let seq = m.sequence_mut();
        seq.add_cue();
        seq.selected_cue_mut().params.warp.oscillators[0].phase_speed = 3.0;
        seq.set_start_frame(1, 24);
        seq.set_duration(1, 24);
        seq.run();
        let mut last = osc0_phase(&m);
        for _ in 0..250 {
            // Frames 0..60: crosses RampStarted (24) and RampFinished (48).
            m.advance(0.01);
            let now = osc0_phase(&m);
            assert!(phase_step(last, now).abs() < 0.1, "phase jumped");
            last = now;
        }
        assert!(
            !matches!(m.sequence.as_ref().unwrap().view(), SeqView::Ramp { .. }),
            "ramp finished"
        );
    }

    #[test]
    fn reversed_transition_reverts_without_changing_banks_or_phase() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Transition);
        m.editable().warp.oscillators[0].phase_speed = 2.0;
        m.advance(0.3);
        m.trigger();
        m.advance(0.5);
        m.trigger(); // reverse
        assert!(!m.ab.ramp().unwrap().forward);
        let mut last = osc0_phase(&m);
        let mut steps = 0;
        while m.ab.ramp().is_some() {
            m.advance(0.01);
            let now = osc0_phase(&m);
            assert!(phase_step(last, now).abs() < 0.1, "phase jumped");
            last = now;
            steps += 1;
            assert!(steps < 1000, "never reverted");
        }
        assert_eq!(m.ab.on_air, 0, "on-air bank unchanged");
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

- [ ] **Step 4: Run the tests and confirm they pass.** Run: `cargo test --lib motion::`. Expected: `15 passed`. Then `cargo test`: `91 passed` for the unit tests, `1 passed` in `tests/smoke.rs`.

- [ ] **Step 5: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add src/motion.rs
git commit -m "feat: add the off-air preview source to Motion" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Preview renderer and overlay

**Files:**
- Create: `src/preview.rs`
- Modify: `src/lib.rs`, `src/ui.rs`, `src/app.rs`, `tests/smoke.rs`
- Test: unit tests in `src/preview.rs`; `renders_the_preview_offscreen` in `tests/smoke.rs`; a manual look

**Interfaces:**
- Consumes: `Motion::preview`, `Preview`, `PreviewSource::label` (Task 1); `Renderer::{new, render, set_source, clear_feedback}`; `gpu::RenderTarget`.
- Produces:
  - `preview::WIDTH` (480), `preview::FORMAT` (`Rgba8Unorm`), and `preview::size_for(canvas: (u32, u32)) -> (u32, u32)`.
  - `PreviewView` with these methods:
    - `new(device, queue, &mut egui_wgpu::Renderer, canvas, &GrayImage)`;
    - `texture_id()`, `size()`, `set_source(..)`, `clear_feedback(encoder)`;
    - `hide()`: forget what was shown, so the trails clear when the preview reappears;
    - `render(device, queue, encoder, &Preview, time)`: clears the trails when the source changes.
  - `ui::PreviewOverlay { texture, size, label }` and `ui::draw(ui, &mut Motion, &mut UiState, Option<&PreviewOverlay>) -> UiActions`.
  - `UiState.show_preview: bool` (the app starts it as `true`).

> egui-wgpu 0.36 notes: `register_native_texture(device, &view, wgpu::FilterMode::Linear)` returns the `egui::TextureId`. egui treats texture bytes as already sRGB-encoded, which is why `FORMAT` is the plain `Rgba8Unorm` (design decision 1). The overlay is an `egui::Area` anchored at `Align2::RIGHT_BOTTOM`, shown 320 points wide.

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
pub mod preview;
pub mod sequence;
pub mod source;
pub mod transition;
pub mod ui;
```

- [ ] **Step 2: Write the failing tests.** Create `src/preview.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_keeps_the_canvas_aspect_ratio() {
        assert_eq!(size_for((1920, 1080)), (480, 270));
        assert_eq!(size_for((640, 480)), (480, 360));
        assert_eq!(size_for((854, 480)), (480, 270));
    }

    #[test]
    fn preview_height_is_even_and_at_least_64() {
        assert_eq!(size_for((1000, 333)).1 % 2, 0);
        assert_eq!(size_for((4000, 64)), (480, 64));
    }
}
```

- [ ] **Step 3: Run them and confirm they fail.** Run: `cargo test --lib preview::`. Expected: compile errors such as "cannot find function `size_for`".

- [ ] **Step 4: Implement.** Replace `src/preview.rs` with:

```rust
//! The off-air preview: a second, small renderer with its own trails, drawn into a
//! texture that the panel shows in the bottom-right corner of the window.

use crate::gpu::RenderTarget;
use crate::motion::{Preview, PreviewSource};
use crate::passes::Renderer;
use crate::source::GrayImage;

/// Preview width in pixels; the height follows the canvas aspect ratio.
pub const WIDTH: u32 = 480;
/// The preview texture's format. egui treats texture bytes as already sRGB-encoded, so
/// this is a plain format and the composite shader does the encoding.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The preview size for a canvas: 480 wide, the canvas's aspect ratio, and an even
/// height of at least 64.
pub fn size_for(canvas: (u32, u32)) -> (u32, u32) {
    let height = (WIDTH as f32 * canvas.1 as f32 / canvas.0 as f32 / 2.0).round() as u32 * 2;
    (WIDTH, height.max(64))
}

pub struct PreviewView {
    renderer: Renderer,
    target: RenderTarget,
    texture_id: egui::TextureId,
    /// What the trails currently belong to; a change clears them.
    shown: Option<PreviewSource>,
}

impl PreviewView {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        egui_renderer: &mut egui_wgpu::Renderer,
        canvas: (u32, u32),
        image: &GrayImage,
    ) -> Self {
        let size = size_for(canvas);
        let target = RenderTarget::new(device, "preview", size.0, size.1, FORMAT);
        let texture_id =
            egui_renderer.register_native_texture(device, &target.view, wgpu::FilterMode::Linear);
        Self {
            renderer: Renderer::new(device, queue, FORMAT, size, image),
            target,
            texture_id,
            shown: None,
        }
    }

    pub fn texture_id(&self) -> egui::TextureId {
        self.texture_id
    }

    /// The preview texture's size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.target.texture.width(), self.target.texture.height())
    }

    pub fn set_source(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, image: &GrayImage) {
        self.renderer.set_source(device, queue, image);
    }

    pub fn clear_feedback(&self, encoder: &mut wgpu::CommandEncoder) {
        self.renderer.clear_feedback(encoder);
    }

    /// Forgets what was shown, so the trails are cleared when the preview appears again.
    pub fn hide(&mut self) {
        self.shown = None;
    }

    /// Records the preview frame into `encoder`, clearing the trails first when the
    /// preview has switched to a different bank or cue.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        preview: &Preview,
        time: f32,
    ) {
        if self.shown != Some(preview.source) {
            self.renderer.clear_feedback(encoder);
            self.shown = Some(preview.source);
        }
        let size = self.size();
        self.renderer.render(
            device,
            queue,
            encoder,
            &preview.frame,
            time,
            &self.target.view,
            size,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_keeps_the_canvas_aspect_ratio() {
        assert_eq!(size_for((1920, 1080)), (480, 270));
        assert_eq!(size_for((640, 480)), (480, 360));
        assert_eq!(size_for((854, 480)), (480, 270));
    }

    #[test]
    fn preview_height_is_even_and_at_least_64() {
        assert_eq!(size_for((1000, 333)).1 % 2, 0);
        assert_eq!(size_for((4000, 64)), (480, 64));
    }
}
```

- [ ] **Step 5: Run the tests and confirm they pass.** Run: `cargo test --lib preview::`. Expected: `2 passed`.

- [ ] **Step 6: Add the smoke test.** Replace `tests/smoke.rs` with the following. It factors device creation into `device()`, generalises `read_back` to any texture size, and adds `renders_the_preview_offscreen`.

```rust
//! Headless GPU smoke tests: build every pass on a real device, render a few frames
//! offscreen, and read results back. wgpu validation errors panic, failing the test.

use rasterwarp::blend::FrameParams;
use rasterwarp::gpu;
use rasterwarp::motion::{Mode, Motion};
use rasterwarp::params::Params;
use rasterwarp::passes::Renderer;
use rasterwarp::preview::PreviewView;
use rasterwarp::source::test_card;

const OUT_W: u32 = 320;
const OUT_H: u32 = 180;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let backends = wgpu::Backends::all();
    let instance = gpu::create_instance(backends);
    match pollster::block_on(gpu::request_device(&instance, backends, None)) {
        Ok((_adapter, device, queue)) => Some((device, queue)),
        Err(_) => {
            eprintln!("skipping smoke test: no GPU adapter available");
            None
        }
    }
}

#[test]
fn renders_frames_offscreen() {
    let Some((device, queue)) = device() else {
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

#[test]
fn renders_the_preview_offscreen() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut egui_renderer = egui_wgpu::Renderer::new(
        &device,
        wgpu::TextureFormat::Bgra8Unorm,
        egui_wgpu::RendererOptions::default(),
    );
    let mut preview = PreviewView::new(
        &device,
        &queue,
        &mut egui_renderer,
        (1920, 1080),
        &test_card(400, 300),
    );
    assert_eq!(preview.size(), (480, 270));
    let mut motion = Motion::new(Params::default());
    motion.set_mode(Mode::Transition);
    let shown = motion.preview().expect("Transition mode has a preview");
    for frame in 0..3 {
        let mut encoder = device.create_command_encoder(&Default::default());
        preview.render(&device, &queue, &mut encoder, &shown, frame as f32 * 0.1);
        queue.submit([encoder.finish()]);
    }
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll device");
}

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
/// that buffer copies require.
fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let row = width * 4;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded * height),
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
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    buffer.map_async(wgpu::MapMode::Read, .., |result| {
        result.expect("map readback buffer")
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll device");
    let mapped = buffer.get_mapped_range(..).expect("mapped range");
    mapped
        .chunks(padded as usize)
        .flat_map(|r| &r[..row as usize])
        .copied()
        .collect()
}
```

Run: `cargo test --test smoke`. Expected: `2 passed`.

- [ ] **Step 7: Add the overlay to the panel.** Replace `src/ui.rs` with:

```rust
//! The egui parameter panel.

use egui::{Button, CollapsingHeader, ComboBox, ProgressBar, Slider, Ui};

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
    /// Show the off-air preview in Transition and Sequence modes.
    pub show_preview: bool,
    /// Smoothed frame time in milliseconds.
    pub frame_ms: f32,
    pub source_info: String,
    pub load_error: Option<String>,
    /// The user curve open in the curve editor.
    pub editing_curve: Option<u32>,
}

/// The off-air preview as the panel shows it.
pub struct PreviewOverlay {
    pub texture: egui::TextureId,
    /// Texture size in pixels.
    pub size: (u32, u32),
    pub label: String,
}

/// Display width of the preview overlay, in points.
const PREVIEW_WIDTH: f32 = 320.0;

/// One-shot actions requested by the user this frame.
#[derive(Default)]
pub struct UiActions {
    pub clear_feedback: bool,
}

pub fn draw(
    ui: &mut Ui,
    motion: &mut Motion,
    state: &mut UiState,
    preview: Option<&PreviewOverlay>,
) -> UiActions {
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
                    ui.checkbox(&mut state.show_preview, "Show preview");
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
    if let Some(preview) = preview {
        preview_overlay(ui.ctx(), preview);
    }
    actions
}

/// The preview, framed and labelled, in the bottom-right corner of the window.
fn preview_overlay(ctx: &egui::Context, preview: &PreviewOverlay) {
    let height = PREVIEW_WIDTH * preview.size.1 as f32 / preview.size.0 as f32;
    egui::Area::new(egui::Id::new("preview"))
        .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(&preview.label);
                ui.image((preview.texture, egui::vec2(PREVIEW_WIDTH, height)));
            });
        });
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
        let can_delete = seq.cues().len() > 1;
        if ui
            .add_enabled(can_delete, Button::new("Delete cue"))
            .clicked()
        {
            seq.delete_cue();
        }
    });
    let i = seq.selected;
    let mut start = seq.cues()[i].start_frame;
    let mut duration = seq.cues()[i].duration_frames;
    // Cue 1 always starts at frame 0 and is never ramped into, so its controls stay disabled.
    ui.add_enabled_ui(i > 0, |ui| {
        ui.horizontal(|ui| {
            if ui
                .add(Slider::new(&mut start, 0..=MAX_FRAME).text("start frame"))
                .changed()
            {
                seq.set_start_frame(i, start);
            }
            ui.label(format!("{:.2} s", start as f32 / FRAMES_PER_SECOND));
        });
        ui.horizontal(|ui| {
            if ui
                .add(Slider::new(&mut duration, 1..=MAX_FRAME).text("ramp frames"))
                .changed()
            {
                seq.set_duration(i, duration);
            }
            ui.label(format!("{:.2} s", duration as f32 / FRAMES_PER_SECOND));
        });
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

- [ ] **Step 8: Wire the preview into the app.** Replace `src/app.rs` with:

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
use crate::preview::PreviewView;
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
    preview: PreviewView,
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
            show_preview: true,
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
        let mut egui_renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        let preview = PreviewView::new(&device, &queue, &mut egui_renderer, INTERNAL_SIZE, &image);

        Ok(Self {
            window,
            surface,
            config,
            device,
            queue,
            max_texture_side,
            composite_format,
            renderer,
            preview,
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
                self.preview.set_source(&self.device, &self.queue, &image);
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

        let overlay = self
            .ui
            .show_preview
            .then(|| self.motion.preview())
            .flatten()
            .map(|p| ui::PreviewOverlay {
                texture: self.preview.texture_id(),
                size: self.preview.size(),
                label: p.source.label(),
            });
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let mut actions = UiActions::default();
        let egui_output = self.egui_ctx.run_ui(raw_input, |ui| {
            actions = ui::draw(ui, &mut self.motion, &mut self.ui, overlay.as_ref())
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
            self.preview.clear_feedback(&mut encoder);
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
        match self
            .ui
            .show_preview
            .then(|| self.motion.preview())
            .flatten()
        {
            Some(preview) => self.preview.render(
                &self.device,
                &self.queue,
                &mut encoder,
                &preview,
                self.time as f32,
            ),
            None => self.preview.hide(),
        }
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

- [ ] **Step 9: Run everything.** Run: `cargo test`. Expected: `93 passed` for the unit tests, `2 passed` in `tests/smoke.rs`.

- [ ] **Step 10: Check by eye.** Run: `cargo run --release` and switch to Transition mode. Expected:
  - A framed overlay labelled "Off-air: B" appears in the bottom-right corner.
  - Changing zoom changes only the overlay.
  - Its colours match the output's: orange bands stay orange and aren't darkened to red.
  - Unticking **Show preview** hides it. Live mode has no overlay.

- [ ] **Step 11: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 12: Commit**

```bash
git add src/lib.rs src/preview.rs src/ui.rs src/app.rs tests/smoke.rs
git commit -m "feat: show a live off-air preview in the corner of the window" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Canvas resolution

**Files:**
- Create: `src/canvas.rs`
- Modify: `src/lib.rs`, `src/passes/mod.rs`, `src/preview.rs`, `src/ui.rs`, `src/app.rs`, `tests/smoke.rs`
- Test: unit tests in `src/canvas.rs`; `renders_after_a_canvas_resize` in `tests/smoke.rs`

**Interfaces:**
- Consumes: `PreviewView` (Task 2), `ui::draw` (Task 2).
- Produces:
  - `canvas::DEFAULT` ((1920, 1080)), `canvas::MIN_SIDE` (64), `canvas::PRESETS: [(&str, (u32, u32)); 6]` and `canvas::sanitize(size, max_side) -> (u32, u32)`. `sanitize` rounds each side up to even and clamps it to [64, max_side].
  - `CanvasChoice { choice, custom, current, max_side }` with `CanvasChoice::CUSTOM`, `new(current, max_side)` and `requested() -> (u32, u32)`.
  - `Renderer::size()` and `Renderer::resize(device, size)`. `resize` rebuilds the warp, colorize, feedback and bloom targets, so the trails start cleared.
  - `PreviewView::resize(device, &mut egui_wgpu::Renderer, canvas)`, which keeps the canvas's aspect ratio.
  - `UiState.canvas: CanvasChoice` and `UiActions.apply_canvas: Option<(u32, u32)>`.
  - The app no longer has `INTERNAL_SIZE`; it starts at `canvas::DEFAULT`.

- [ ] **Step 1: Register the module.** Replace `src/lib.rs` with:

```rust
//! Rasterwarp: real-time analog video-synthesis animation.

pub mod app;
pub mod blend;
pub mod canvas;
pub mod curve;
pub mod curve_editor;
pub mod gpu;
pub mod motion;
pub mod params;
pub mod passes;
pub mod preview;
pub mod sequence;
pub mod source;
pub mod transition;
pub mod ui;
```

- [ ] **Step 2: Write the failing tests.** Create `src/canvas.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_rounded_up_to_even() {
        assert_eq!(sanitize((1001, 721), 8192), (1002, 722));
        assert_eq!(sanitize((1280, 720), 8192), (1280, 720));
    }

    #[test]
    fn sizes_are_clamped_to_the_limits() {
        assert_eq!(sanitize((10, 0), 8192), (64, 64));
        assert_eq!(sanitize((9000, 9000), 8192), (8192, 8192));
        assert_eq!(sanitize((9000, 100), 8191), (8190, 100));
    }

    #[test]
    fn current_size_selects_its_preset() {
        assert_eq!(CanvasChoice::new((1280, 720), 8192).choice, 2);
        assert_eq!(
            CanvasChoice::new((1000, 500), 8192).choice,
            CanvasChoice::CUSTOM
        );
    }

    #[test]
    fn requested_size_comes_from_the_preset_or_the_custom_fields() {
        let mut c = CanvasChoice::new(DEFAULT, 4096);
        c.choice = 5;
        assert_eq!(c.requested(), (3840, 2160));
        c.choice = CanvasChoice::CUSTOM;
        c.custom = (5001, 333);
        assert_eq!(c.requested(), (4096, 334));
    }
}
```

- [ ] **Step 3: Run them and confirm they fail.** Run: `cargo test --lib canvas::`. Expected: compile errors such as "cannot find function `sanitize`".

- [ ] **Step 4: Implement.** Replace `src/canvas.rs` with:

```rust
//! The canvas: the internal render resolution, chosen from presets or typed in.

/// The canvas size at startup.
pub const DEFAULT: (u32, u32) = (1920, 1080);
/// The smallest allowed canvas side, in pixels.
pub const MIN_SIDE: u32 = 64;

pub const PRESETS: [(&str, (u32, u32)); 6] = [
    ("480p 4:3", (640, 480)),
    ("480p 16:9", (854, 480)),
    ("720p", (1280, 720)),
    ("1080p", (1920, 1080)),
    ("1440p", (2560, 1440)),
    ("2160p (4K)", (3840, 2160)),
];

/// Rounds each side up to an even number and clamps it to [`MIN_SIDE`, `max_side`].
pub fn sanitize(size: (u32, u32), max_side: u32) -> (u32, u32) {
    let max_even = max_side & !1;
    let side = |v: u32| (v.saturating_add(1) & !1).clamp(MIN_SIDE, max_even);
    (side(size.0), side(size.1))
}

/// The panel's canvas controls.
#[derive(Clone, Debug)]
pub struct CanvasChoice {
    /// An index into [`PRESETS`], or `PRESETS.len()` for a custom size.
    pub choice: usize,
    pub custom: (u32, u32),
    /// The size the renderer is using now.
    pub current: (u32, u32),
    /// The GPU's largest texture side.
    pub max_side: u32,
}

impl CanvasChoice {
    pub const CUSTOM: usize = PRESETS.len();

    pub fn new(current: (u32, u32), max_side: u32) -> Self {
        let choice = PRESETS
            .iter()
            .position(|(_, size)| *size == current)
            .unwrap_or(Self::CUSTOM);
        Self {
            choice,
            custom: current,
            current,
            max_side,
        }
    }

    /// The size that Apply would switch to.
    pub fn requested(&self) -> (u32, u32) {
        let size = PRESETS
            .get(self.choice)
            .map_or(self.custom, |(_, size)| *size);
        sanitize(size, self.max_side)
    }
}

impl Default for CanvasChoice {
    fn default() -> Self {
        Self::new(DEFAULT, 8192)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_rounded_up_to_even() {
        assert_eq!(sanitize((1001, 721), 8192), (1002, 722));
        assert_eq!(sanitize((1280, 720), 8192), (1280, 720));
    }

    #[test]
    fn sizes_are_clamped_to_the_limits() {
        assert_eq!(sanitize((10, 0), 8192), (64, 64));
        assert_eq!(sanitize((9000, 9000), 8192), (8192, 8192));
        assert_eq!(sanitize((9000, 100), 8191), (8190, 100));
    }

    #[test]
    fn current_size_selects_its_preset() {
        assert_eq!(CanvasChoice::new((1280, 720), 8192).choice, 2);
        assert_eq!(
            CanvasChoice::new((1000, 500), 8192).choice,
            CanvasChoice::CUSTOM
        );
    }

    #[test]
    fn requested_size_comes_from_the_preset_or_the_custom_fields() {
        let mut c = CanvasChoice::new(DEFAULT, 4096);
        c.choice = 5;
        assert_eq!(c.requested(), (3840, 2160));
        c.choice = CanvasChoice::CUSTOM;
        c.custom = (5001, 333);
        assert_eq!(c.requested(), (4096, 334));
    }
}
```

- [ ] **Step 5: Run the tests and confirm they pass.** Run: `cargo test --lib canvas::`. Expected: `4 passed`.

- [ ] **Step 6: Make the renderer resizable.** Replace `src/passes/mod.rs` with:

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
    /// `size` is the internal (canvas) resolution; `output_format` is the format of the
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

    /// The internal (canvas) resolution.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Rebuilds every canvas-sized target at `size`. The trails start out cleared.
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let (w, h) = size;
        self.size = size;
        self.warp = warp::WarpPass::new(device, w, h);
        self.colorize = colorize::ColorizePass::new(device, w, h);
        self.feedback = feedback::FeedbackPass::new(device, w, h);
        self.bloom = bloom::BloomPass::new(device, w, h);
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

Replace `src/preview.rs` with (adds `resize`):

```rust
//! The off-air preview: a second, small renderer with its own trails, drawn into a
//! texture that the panel shows in the bottom-right corner of the window.

use crate::gpu::RenderTarget;
use crate::motion::{Preview, PreviewSource};
use crate::passes::Renderer;
use crate::source::GrayImage;

/// Preview width in pixels; the height follows the canvas aspect ratio.
pub const WIDTH: u32 = 480;
/// The preview texture's format. egui treats texture bytes as already sRGB-encoded, so
/// this is a plain format and the composite shader does the encoding.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The preview size for a canvas: 480 wide, the canvas's aspect ratio, and an even
/// height of at least 64.
pub fn size_for(canvas: (u32, u32)) -> (u32, u32) {
    let height = (WIDTH as f32 * canvas.1 as f32 / canvas.0 as f32 / 2.0).round() as u32 * 2;
    (WIDTH, height.max(64))
}

pub struct PreviewView {
    renderer: Renderer,
    target: RenderTarget,
    texture_id: egui::TextureId,
    /// What the trails currently belong to; a change clears them.
    shown: Option<PreviewSource>,
}

impl PreviewView {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        egui_renderer: &mut egui_wgpu::Renderer,
        canvas: (u32, u32),
        image: &GrayImage,
    ) -> Self {
        let size = size_for(canvas);
        let target = RenderTarget::new(device, "preview", size.0, size.1, FORMAT);
        let texture_id =
            egui_renderer.register_native_texture(device, &target.view, wgpu::FilterMode::Linear);
        Self {
            renderer: Renderer::new(device, queue, FORMAT, size, image),
            target,
            texture_id,
            shown: None,
        }
    }

    pub fn texture_id(&self) -> egui::TextureId {
        self.texture_id
    }

    /// The preview texture's size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.target.texture.width(), self.target.texture.height())
    }

    /// Matches a new canvas size, keeping the canvas's aspect ratio.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        egui_renderer: &mut egui_wgpu::Renderer,
        canvas: (u32, u32),
    ) {
        let size = size_for(canvas);
        if size == self.size() {
            return;
        }
        self.renderer.resize(device, size);
        self.target = RenderTarget::new(device, "preview", size.0, size.1, FORMAT);
        egui_renderer.update_egui_texture_from_wgpu_texture(
            device,
            &self.target.view,
            wgpu::FilterMode::Linear,
            self.texture_id,
        );
    }

    pub fn set_source(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, image: &GrayImage) {
        self.renderer.set_source(device, queue, image);
    }

    pub fn clear_feedback(&self, encoder: &mut wgpu::CommandEncoder) {
        self.renderer.clear_feedback(encoder);
    }

    /// Forgets what was shown, so the trails are cleared when the preview appears again.
    pub fn hide(&mut self) {
        self.shown = None;
    }

    /// Records the preview frame into `encoder`, clearing the trails first when the
    /// preview has switched to a different bank or cue.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        preview: &Preview,
        time: f32,
    ) {
        if self.shown != Some(preview.source) {
            self.renderer.clear_feedback(encoder);
            self.shown = Some(preview.source);
        }
        let size = self.size();
        self.renderer.render(
            device,
            queue,
            encoder,
            &preview.frame,
            time,
            &self.target.view,
            size,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_keeps_the_canvas_aspect_ratio() {
        assert_eq!(size_for((1920, 1080)), (480, 270));
        assert_eq!(size_for((640, 480)), (480, 360));
        assert_eq!(size_for((854, 480)), (480, 270));
    }

    #[test]
    fn preview_height_is_even_and_at_least_64() {
        assert_eq!(size_for((1000, 333)).1 % 2, 0);
        assert_eq!(size_for((4000, 64)), (480, 64));
    }
}
```

- [ ] **Step 7: Add the smoke test.** Replace `tests/smoke.rs` with (adds `renders_after_a_canvas_resize`):

```rust
//! Headless GPU smoke tests: build every pass on a real device, render a few frames
//! offscreen, and read results back. wgpu validation errors panic, failing the test.

use rasterwarp::blend::FrameParams;
use rasterwarp::gpu;
use rasterwarp::motion::{Mode, Motion};
use rasterwarp::params::Params;
use rasterwarp::passes::Renderer;
use rasterwarp::preview::PreviewView;
use rasterwarp::source::test_card;

const OUT_W: u32 = 320;
const OUT_H: u32 = 180;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let backends = wgpu::Backends::all();
    let instance = gpu::create_instance(backends);
    match pollster::block_on(gpu::request_device(&instance, backends, None)) {
        Ok((_adapter, device, queue)) => Some((device, queue)),
        Err(_) => {
            eprintln!("skipping smoke test: no GPU adapter available");
            None
        }
    }
}

#[test]
fn renders_frames_offscreen() {
    let Some((device, queue)) = device() else {
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

#[test]
fn renders_after_a_canvas_resize() {
    let Some((device, queue)) = device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(&device, "smoke output", OUT_W, OUT_H, format);
    let mut renderer = Renderer::new(&device, &queue, format, (640, 360), &test_card(400, 300));
    renderer.resize(&device, (256, 192));
    assert_eq!(renderer.size(), (256, 192));
    let frame_params = FrameParams::at_rest(&Params::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(
        &device,
        &queue,
        &mut encoder,
        &frame_params,
        0.0,
        &output.view,
        (OUT_W, OUT_H),
    );
    queue.submit([encoder.finish()]);
    let pixels = read_back(&device, &queue, &output.texture);
    let (rgba, _) = pixels.as_chunks::<4>();
    assert!(
        rgba.iter().any(|px| *px != rgba[0]),
        "output is a single flat color"
    );
}

#[test]
fn renders_the_preview_offscreen() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut egui_renderer = egui_wgpu::Renderer::new(
        &device,
        wgpu::TextureFormat::Bgra8Unorm,
        egui_wgpu::RendererOptions::default(),
    );
    let mut preview = PreviewView::new(
        &device,
        &queue,
        &mut egui_renderer,
        (1920, 1080),
        &test_card(400, 300),
    );
    assert_eq!(preview.size(), (480, 270));
    let mut motion = Motion::new(Params::default());
    motion.set_mode(Mode::Transition);
    let shown = motion.preview().expect("Transition mode has a preview");
    for frame in 0..3 {
        let mut encoder = device.create_command_encoder(&Default::default());
        preview.render(&device, &queue, &mut encoder, &shown, frame as f32 * 0.1);
        queue.submit([encoder.finish()]);
    }
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll device");
}

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
/// that buffer copies require.
fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let row = width * 4;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded * height),
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
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    buffer.map_async(wgpu::MapMode::Read, .., |result| {
        result.expect("map readback buffer")
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll device");
    let mapped = buffer.get_mapped_range(..).expect("mapped range");
    mapped
        .chunks(padded as usize)
        .flat_map(|r| &r[..row as usize])
        .copied()
        .collect()
}
```

Run: `cargo test --test smoke`. Expected: `3 passed`.

- [ ] **Step 8: Add the Canvas section and wire it in.** Replace `src/ui.rs` with:

```rust
//! The egui parameter panel.

use egui::{Button, CollapsingHeader, ComboBox, ProgressBar, Slider, Ui};

use crate::canvas::{CanvasChoice, PRESETS};
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
    /// Show the off-air preview in Transition and Sequence modes.
    pub show_preview: bool,
    /// Smoothed frame time in milliseconds.
    pub frame_ms: f32,
    pub source_info: String,
    pub load_error: Option<String>,
    /// The user curve open in the curve editor.
    pub editing_curve: Option<u32>,
    pub canvas: CanvasChoice,
}

/// The off-air preview as the panel shows it.
pub struct PreviewOverlay {
    pub texture: egui::TextureId,
    /// Texture size in pixels.
    pub size: (u32, u32),
    pub label: String,
}

/// Display width of the preview overlay, in points.
const PREVIEW_WIDTH: f32 = 320.0;

/// One-shot actions requested by the user this frame.
#[derive(Default)]
pub struct UiActions {
    pub clear_feedback: bool,
    /// Switch the canvas to this size.
    pub apply_canvas: Option<(u32, u32)>,
}

pub fn draw(
    ui: &mut Ui,
    motion: &mut Motion,
    state: &mut UiState,
    preview: Option<&PreviewOverlay>,
) -> UiActions {
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
                    ui.checkbox(&mut state.show_preview, "Show preview");
                    if ui.button("Reset all").clicked() {
                        *motion.editable() = Params::default();
                        actions.clear_feedback = true;
                    }
                });
                canvas_section(ui, &mut state.canvas, &mut actions);
                mode_section(ui, motion);
                curves_section(ui, &mut motion.curves, state);
                let params = motion.editable();
                warp_section(ui, params);
                colorize_section(ui, params);
                feedback_section(ui, params, &mut actions);
                glow_section(ui, params);
            });
        });
    if let Some(preview) = preview {
        preview_overlay(ui.ctx(), preview);
    }
    actions
}

/// The preview, framed and labelled, in the bottom-right corner of the window.
fn preview_overlay(ctx: &egui::Context, preview: &PreviewOverlay) {
    let height = PREVIEW_WIDTH * preview.size.1 as f32 / preview.size.0 as f32;
    egui::Area::new(egui::Id::new("preview"))
        .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(&preview.label);
                ui.image((preview.texture, egui::vec2(PREVIEW_WIDTH, height)));
            });
        });
}

fn canvas_section(ui: &mut Ui, canvas: &mut CanvasChoice, actions: &mut UiActions) {
    CollapsingHeader::new("Canvas").show(ui, |ui| {
        let name = |i: usize| match PRESETS.get(i) {
            Some((name, (w, h))) => format!("{name} ({w}×{h})"),
            None => "Custom".to_owned(),
        };
        ComboBox::from_id_salt("canvas preset")
            .selected_text(name(canvas.choice))
            .show_ui(ui, |ui| {
                for i in 0..=CanvasChoice::CUSTOM {
                    ui.selectable_value(&mut canvas.choice, i, name(i));
                }
            });
        if canvas.choice == CanvasChoice::CUSTOM {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut canvas.custom.0).range(1..=canvas.max_side));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut canvas.custom.1).range(1..=canvas.max_side));
            });
        }
        let (w, h) = canvas.requested();
        let (cw, ch) = canvas.current;
        ui.label(format!("Rendering at {cw}×{ch}"));
        ui.horizontal(|ui| {
            let changed = (w, h) != canvas.current;
            if ui
                .add_enabled(changed, Button::new(format!("Apply {w}×{h}")))
                .clicked()
            {
                actions.apply_canvas = Some((w, h));
            }
            ui.small("clears the trails");
        });
    });
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
        let can_delete = seq.cues().len() > 1;
        if ui
            .add_enabled(can_delete, Button::new("Delete cue"))
            .clicked()
        {
            seq.delete_cue();
        }
    });
    let i = seq.selected;
    let mut start = seq.cues()[i].start_frame;
    let mut duration = seq.cues()[i].duration_frames;
    // Cue 1 always starts at frame 0 and is never ramped into, so its controls stay disabled.
    ui.add_enabled_ui(i > 0, |ui| {
        ui.horizontal(|ui| {
            if ui
                .add(Slider::new(&mut start, 0..=MAX_FRAME).text("start frame"))
                .changed()
            {
                seq.set_start_frame(i, start);
            }
            ui.label(format!("{:.2} s", start as f32 / FRAMES_PER_SECOND));
        });
        ui.horizontal(|ui| {
            if ui
                .add(Slider::new(&mut duration, 1..=MAX_FRAME).text("ramp frames"))
                .changed()
            {
                seq.set_duration(i, duration);
            }
            ui.label(format!("{:.2} s", duration as f32 / FRAMES_PER_SECOND));
        });
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

Replace `src/app.rs` with:

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

use crate::canvas::{self, CanvasChoice};
use crate::gpu;
use crate::motion::Motion;
use crate::params::Params;
use crate::passes::Renderer;
use crate::preview::PreviewView;
use crate::source::{self, GrayImage};
use crate::ui::{self, UiActions, UiState};

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
    preview: PreviewView,
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
            show_preview: true,
            canvas: CanvasChoice::new(canvas::DEFAULT, max_texture_side),
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
        let renderer = Renderer::new(&device, &queue, composite_format, canvas::DEFAULT, &image);

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui_ctx.viewport_id(),
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(max_texture_side as usize),
        );
        let mut egui_renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        let preview =
            PreviewView::new(&device, &queue, &mut egui_renderer, canvas::DEFAULT, &image);

        Ok(Self {
            window,
            surface,
            config,
            device,
            queue,
            max_texture_side,
            composite_format,
            renderer,
            preview,
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
                self.preview.set_source(&self.device, &self.queue, &image);
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

        let overlay = self
            .ui
            .show_preview
            .then(|| self.motion.preview())
            .flatten()
            .map(|p| ui::PreviewOverlay {
                texture: self.preview.texture_id(),
                size: self.preview.size(),
                label: p.source.label(),
            });
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let mut actions = UiActions::default();
        let egui_output = self.egui_ctx.run_ui(raw_input, |ui| {
            actions = ui::draw(ui, &mut self.motion, &mut self.ui, overlay.as_ref())
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

        if let Some(size) = actions.apply_canvas {
            self.renderer.resize(&self.device, size);
            self.preview
                .resize(&self.device, &mut self.egui_renderer, size);
            self.ui.canvas.current = size;
        }
        let frame_params = self.motion.frame();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        if actions.clear_feedback {
            self.renderer.clear_feedback(&mut encoder);
            self.preview.clear_feedback(&mut encoder);
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
        match self
            .ui
            .show_preview
            .then(|| self.motion.preview())
            .flatten()
        {
            Some(preview) => self.preview.render(
                &self.device,
                &self.queue,
                &mut encoder,
                &preview,
                self.time as f32,
            ),
            None => self.preview.hide(),
        }
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

- [ ] **Step 9: Run everything.** Run: `cargo test`. Expected: `97 passed` for the unit tests, `3 passed` in `tests/smoke.rs`.

- [ ] **Step 10: Check by eye.** Run: `cargo run --release`. Open **Canvas**, pick "480p 4:3 (640×480)" and press **Apply 640×480**. Expected:
  - The output becomes 4:3, pillarboxed, and the label reads "Rendering at 640×480".
  - In Transition mode the preview overlay is 4:3 too.
  - A custom 1001×721 shows "Apply 1002×722".

- [ ] **Step 11: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 12: Commit**

```bash
git add src/lib.rs src/canvas.rs src/passes/mod.rs src/preview.rs src/ui.rs src/app.rs tests/smoke.rs
git commit -m "feat: make the canvas resolution selectable" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: FFmpeg encoder thread

**Files:**
- Create: `.cargo/config.toml`, `build.rs`, `src/capture/mod.rs`, `src/capture/encode.rs`, `tests/capture.rs`
- Modify: `Cargo.toml`, `Cargo.lock` (generated), `src/lib.rs`
- Test: `tests/capture.rs` (an FFV1 lossless round trip, HEVC within a small error, and an existing file is never overwritten)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces (in `capture::encode`):
  - `FPS` (60) and `QUEUE` (4);
  - `VideoFormat { Hevc, Ffv1 }` with `ALL`, `label()` and `extension()` ("mp4" / "mkv");
  - `Frame { pts: i64, rgba: Vec<u8> }`: tightly packed sRGB RGBA;
  - `Encoder` with these methods:
    - `start(path, VideoFormat, (w, h)) -> Result<Encoder>`. It fails if the file exists or the encoder won't open, and leaves no file behind.
    - `try_send(Frame) -> Result<Option<Frame>>`: returns `Some(frame)` back when the queue is full.
    - `send(Frame) -> Result<()>`: blocking.
    - `recycled_buffer() -> Option<Vec<u8>>`.
    - `finish(self) -> Result<u64>`: the number of frames written, or the error that stopped the thread.

> **Prerequisites:**
> - The shared FFmpeg 9.0.2 build and Visual Studio 2019's LLVM must be at the paths in `.cargo/config.toml`.
> - `ffmpeg-sys-next` runs bindgen on first build, which takes about 45 s.
> - `build.rs` copies the 5 FFmpeg DLLs into `target/<profile>/` and `target/<profile>/deps/`. Without them, every exe and test binary exits immediately with 0xC0000135 and no message.
> - `hevc_nvenc` needs an NVIDIA GPU. Elsewhere the HEVC test prints "skipping" and passes.

- [ ] **Step 1: Add the dependency and the build setup.** Replace `Cargo.toml` with:

```toml
[package]
name = "rasterwarp"
version = "0.1.0"
edition = "2024"
rust-version = "1.88"

[dependencies]
anyhow = "1"
bytemuck = { version = "1", features = ["derive"] }
egui = "0.36"
egui-wgpu = "0.36"
egui-winit = "0.36"
env_logger = "0.11"
ffmpeg-next = { version = "9.0.0", default-features = false, features = ["codec", "format", "software-scaling"] }
image = { version = "0.25", default-features = false, features = ["png", "jpeg"] }
log = "0.4"
pollster = "1"
wgpu = "30"
winit = "0.30"
```

Create `.cargo/config.toml`:

```toml
# Machine-specific paths for linking FFmpeg (video capture). Adjust these when building
# elsewhere: FFMPEG_DIR is a shared FFmpeg 9 build (with bin/, include/ and lib/), and
# LIBCLANG_PATH is any libclang that bindgen can load (the one bundled with Visual Studio works).
[env]
FFMPEG_DIR = 'C:\Users\abart\Desktop\ffmpeg-9.0.2-full_build-shared'
LIBCLANG_PATH = 'C:\Program Files (x86)\Microsoft Visual Studio\2019\Community\VC\Tools\Llvm\x64\bin'
```

Create `build.rs`:

```rust
//! Copies the FFmpeg DLLs that capture links against next to the built executables, so
//! `cargo run`, `cargo test` and the exe in `target/<profile>/` all find them at startup.
//! Without them Windows exits the program with 0xC0000135 and no message.

use std::path::{Path, PathBuf};
use std::{env, fs};

const DLLS: [&str; 5] = [
    "avcodec-63.dll",
    "avformat-63.dll",
    "avutil-61.dll",
    "swscale-10.dll",
    "swresample-7.dll",
];

fn main() {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let Ok(ffmpeg_dir) = env::var("FFMPEG_DIR") else {
        panic!("FFMPEG_DIR is not set; see .cargo/config.toml");
    };
    let bin = Path::new(&ffmpeg_dir).join("bin");
    // OUT_DIR is target/<profile>/build/<crate>-<hash>/out.
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("OUT_DIR is inside target/<profile>")
        .to_path_buf();
    for dll in DLLS {
        let src = bin.join(dll);
        assert!(
            src.is_file(),
            "{} is missing; FFMPEG_DIR must point at a shared FFmpeg 9 build",
            src.display()
        );
        for dir in [profile_dir.clone(), profile_dir.join("deps")] {
            copy_if_changed(&src, &dir.join(dll));
        }
    }
}

fn copy_if_changed(src: &Path, dst: &Path) {
    let same = match (fs::metadata(src), fs::metadata(dst)) {
        (Ok(a), Ok(b)) => a.len() == b.len(),
        _ => false,
    };
    if !same {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).expect("create target directory");
        }
        fs::copy(src, dst).unwrap_or_else(|e| panic!("copy {} failed: {e}", src.display()));
    }
}
```

Run: `cargo build`. Expected: it builds; `target/debug/avcodec-63.dll` and `target/debug/deps/avcodec-63.dll` exist.

- [ ] **Step 2: Write the failing tests.** Create `tests/capture.rs`:

```rust
//! Encodes short clips with the linked FFmpeg libraries and decodes them again.

use std::path::{Path, PathBuf};

use ffmpeg_next as ff;
use rasterwarp::capture::encode::{Encoder, Frame, VideoFormat};

const SIZE: (u32, u32) = (320, 180);
const FRAMES: i64 = 30;

/// A fresh path in a per-test temporary directory.
fn temp_file(test: &str, extension: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rasterwarp-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.join(format!("clip.{extension}"))
}

/// A moving gradient with some fine detail, different in every frame.
fn pattern(index: i64) -> Vec<u8> {
    let (w, h) = (SIZE.0 as usize, SIZE.1 as usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let shift = index as usize * 4;
            rgba.extend_from_slice(&[
                ((x + shift) * 255 / w) as u8,
                (y * 255 / h) as u8,
                ((x / 8 + y / 8) % 2 * 64 + shift) as u8,
                255,
            ]);
        }
    }
    rgba
}

fn encode(path: &Path, format: VideoFormat) -> anyhow::Result<u64> {
    let encoder = Encoder::start(path, format, SIZE)?;
    for pts in 0..FRAMES {
        encoder.send(Frame {
            pts,
            rgba: pattern(pts),
        })?;
    }
    encoder.finish()
}

/// Decodes every frame of `path` to tightly packed RGBA (BT.709 for YUV input).
fn decode(path: &Path) -> Vec<(u32, u32, Vec<u8>)> {
    ff::init().unwrap();
    let mut input = ff::format::input(path).expect("open the recording");
    let stream = input
        .streams()
        .best(ff::media::Type::Video)
        .expect("a video stream");
    let index = stream.index();
    assert_eq!(
        stream.avg_frame_rate(),
        ff::Rational(60, 1),
        "declared frame rate"
    );
    let context = ff::codec::context::Context::from_parameters(stream.parameters()).unwrap();
    let mut decoder = context.decoder().video().unwrap();
    let (w, h) = (decoder.width(), decoder.height());
    let mut scaler = ff::software::scaling::Context::get(
        decoder.format(),
        w,
        h,
        ff::format::Pixel::RGBA,
        w,
        h,
        ff::software::scaling::Flags::BILINEAR,
    )
    .unwrap();
    if decoder.format() == ff::format::Pixel::YUV444P {
        // SAFETY: valid scaler pointer; FFmpeg's coefficient tables are static.
        unsafe {
            let bt709 = ff::ffi::sws_getCoefficients(ff::ffi::SWS_CS_ITU709);
            ff::ffi::sws_setColorspaceDetails(
                scaler.as_mut_ptr(),
                bt709,
                0,
                bt709,
                1,
                0,
                1 << 16,
                1 << 16,
            );
        }
    }
    let mut frames = Vec::new();
    let mut receive = |decoder: &mut ff::decoder::Video, frames: &mut Vec<_>| {
        let mut decoded = ff::frame::Video::empty();
        while decoder.receive_frame(&mut decoded).is_ok() {
            let mut rgba = ff::frame::Video::empty();
            scaler.run(&decoded, &mut rgba).unwrap();
            let row = w as usize * 4;
            let packed: Vec<u8> = rgba
                .data(0)
                .chunks(rgba.stride(0))
                .take(h as usize)
                .flat_map(|r| &r[..row])
                .copied()
                .collect();
            frames.push((w, h, packed));
        }
    };
    for (stream, packet) in input.packets() {
        if stream.index() == index {
            decoder.send_packet(&packet).unwrap();
            receive(&mut decoder, &mut frames);
        }
    }
    decoder.send_eof().unwrap();
    receive(&mut decoder, &mut frames);
    frames
}

#[test]
fn ffv1_recording_is_lossless() {
    let path = temp_file("ffv1", "mkv");
    assert_eq!(encode(&path, VideoFormat::Ffv1).expect("encode"), 30);
    let frames = decode(&path);
    assert_eq!(frames.len(), FRAMES as usize);
    let (w, h, rgba) = &frames[7];
    assert_eq!((*w, *h), SIZE);
    assert!(
        *rgba == pattern(7),
        "frame 7 differs from what was recorded"
    );
}

#[test]
fn hevc_recording_is_close_to_the_source() {
    let path = temp_file("hevc", "mp4");
    let written = match encode(&path, VideoFormat::Hevc) {
        Ok(written) => written,
        Err(err) => {
            eprintln!("skipping HEVC test: {err:#}");
            return;
        }
    };
    assert_eq!(written, 30);
    let frames = decode(&path);
    assert_eq!(frames.len(), FRAMES as usize);
    let (w, h, rgba) = &frames[7];
    assert_eq!((*w, *h), SIZE);
    let source = pattern(7);
    let error: u64 = rgba
        .iter()
        .zip(&source)
        .map(|(a, b)| u64::from(a.abs_diff(*b)))
        .sum();
    let mean = error as f64 / source.len() as f64;
    assert!(mean < 3.0, "mean error {mean:.2} per channel");
}

#[test]
fn an_existing_file_is_not_overwritten() {
    let path = temp_file("exists", "mkv");
    std::fs::write(&path, b"keep me").unwrap();
    let err = Encoder::start(&path, VideoFormat::Ffv1, SIZE)
        .err()
        .expect("start must fail");
    assert!(format!("{err:#}").contains("already exists"));
    assert_eq!(std::fs::read(&path).unwrap(), b"keep me");
}
```

- [ ] **Step 3: Run them and confirm they fail.** Run: `cargo test --test capture`. Expected: compile error "unresolved import `rasterwarp::capture`".

- [ ] **Step 4: Implement.** Replace `src/lib.rs` with:

```rust
//! Rasterwarp: real-time analog video-synthesis animation.

pub mod app;
pub mod blend;
pub mod canvas;
pub mod capture;
pub mod curve;
pub mod curve_editor;
pub mod gpu;
pub mod motion;
pub mod params;
pub mod passes;
pub mod preview;
pub mod sequence;
pub mod source;
pub mod transition;
pub mod ui;
```

Create `src/capture/mod.rs`:

```rust
//! Video capture: recording the clean canvas output to a file.

pub mod encode;
```

Create `src/capture/encode.rs`:

```rust
//! The encoder thread: converts captured RGBA frames and writes them to a video file
//! with the linked FFmpeg libraries.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::thread::JoinHandle;

use anyhow::{Context, Result, anyhow, bail};
use ff::{Dictionary, Packet, Rational, codec, encoder, format, frame, software::scaling};
use ffmpeg_next as ff;

/// Frames per second of every recording.
pub const FPS: i32 = 60;
/// Frames that can wait for the encoder before real-time capture starts dropping them.
pub const QUEUE: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoFormat {
    /// HEVC 4:4:4 on the NVIDIA hardware encoder, near-lossless, in fragmented MP4.
    Hevc,
    /// FFV1 lossless RGB in Matroska, on the CPU.
    Ffv1,
}

impl VideoFormat {
    pub const ALL: [VideoFormat; 2] = [VideoFormat::Hevc, VideoFormat::Ffv1];

    pub fn label(self) -> &'static str {
        match self {
            VideoFormat::Hevc => "HEVC NVENC 4:4:4, near-lossless",
            VideoFormat::Ffv1 => "FFV1 lossless, may drop frames above 720p",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            VideoFormat::Hevc => "mp4",
            VideoFormat::Ffv1 => "mkv",
        }
    }
}

/// One captured frame: tightly packed sRGB RGBA rows, `width × height × 4` bytes.
pub struct Frame {
    /// Presentation time in frames at [`FPS`].
    pub pts: i64,
    pub rgba: Vec<u8>,
}

/// A running encoder thread. Dropping it without [`Encoder::finish`] still closes the file.
pub struct Encoder {
    frames: Option<SyncSender<Frame>>,
    recycled: Receiver<Vec<u8>>,
    thread: Option<JoinHandle<Result<u64>>>,
}

impl Encoder {
    /// Opens `path` for writing and starts the encoder thread. Fails without leaving a
    /// file behind if the file exists or the encoder can't be opened.
    pub fn start(path: &Path, format: VideoFormat, size: (u32, u32)) -> Result<Self> {
        if path.exists() {
            bail!("{} already exists", path.display());
        }
        let (frames, frames_rx) = mpsc::sync_channel(QUEUE);
        let (recycle_tx, recycled) = mpsc::channel();
        let (ready_tx, ready) = mpsc::sync_channel(1);
        let owned = path.to_path_buf();
        let thread = std::thread::Builder::new()
            .name("encoder".into())
            .spawn(move || run(owned, format, size, frames_rx, recycle_tx, ready_tx))
            .context("could not start the encoder thread")?;
        match ready.recv() {
            Ok(Ok(())) => Ok(Self {
                frames: Some(frames),
                recycled,
                thread: Some(thread),
            }),
            Ok(Err(err)) => {
                let _ = thread.join();
                let _ = std::fs::remove_file(path);
                Err(err)
            }
            Err(_) => Err(anyhow!("the encoder thread stopped while opening")),
        }
    }

    /// Queues a frame without waiting. Returns the frame back if the queue is full.
    /// An error means the encoder has stopped; [`Encoder::finish`] reports why.
    pub fn try_send(&self, frame: Frame) -> Result<Option<Frame>> {
        match self.sender()?.try_send(frame) {
            Ok(()) => Ok(None),
            Err(TrySendError::Full(frame)) => Ok(Some(frame)),
            Err(TrySendError::Disconnected(_)) => Err(anyhow!("the encoder stopped")),
        }
    }

    /// Queues a frame, waiting for room. An error means the encoder has stopped.
    pub fn send(&self, frame: Frame) -> Result<()> {
        self.sender()?
            .send(frame)
            .map_err(|_| anyhow!("the encoder stopped"))
    }

    /// A frame buffer the encoder has finished with, for reuse.
    pub fn recycled_buffer(&self) -> Option<Vec<u8>> {
        self.recycled.try_recv().ok()
    }

    /// Flushes the encoder, closes the file and returns the number of frames written,
    /// or the error that stopped the encoder.
    pub fn finish(mut self) -> Result<u64> {
        self.frames = None;
        match self.thread.take().map(JoinHandle::join) {
            Some(Ok(result)) => result,
            Some(Err(_)) => Err(anyhow!("the encoder thread panicked")),
            None => Err(anyhow!("the encoder already finished")),
        }
    }

    fn sender(&self) -> Result<&SyncSender<Frame>> {
        self.frames
            .as_ref()
            .ok_or_else(|| anyhow!("the encoder already finished"))
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        self.frames = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    path: PathBuf,
    format: VideoFormat,
    size: (u32, u32),
    frames: Receiver<Frame>,
    recycled: Sender<Vec<u8>>,
    ready: SyncSender<Result<()>>,
) -> Result<u64> {
    let mut sink = match Sink::open(&path, format, size) {
        Ok(sink) => {
            let _ = ready.send(Ok(()));
            sink
        }
        Err(err) => {
            let _ = ready.send(Err(err));
            return Ok(0);
        }
    };
    let mut written = 0;
    for frame in frames {
        if let Err(err) = sink.write(&frame) {
            // Keep what was written playable where possible.
            let _ = sink.finish();
            return Err(err.context(format!("encoding frame {}", frame.pts)));
        }
        written += 1;
        let _ = recycled.send(frame.rgba);
    }
    sink.finish()?;
    Ok(written)
}

/// How captured RGBA becomes the encoder's pixel format.
enum Convert {
    /// RGBA → YUV 4:4:4 with BT.709 coefficients (HEVC).
    Scale {
        scaler: scaling::Context,
        rgba: frame::Video,
    },
    /// RGBA → BGRA by swapping bytes, which is exact (FFV1).
    Swizzle,
}

struct Sink {
    output: format::context::Output,
    encoder: encoder::Video,
    convert: Convert,
    frame: frame::Video,
    size: (u32, u32),
    stream_time_base: Rational,
}

impl Sink {
    fn open(path: &Path, format: VideoFormat, size: (u32, u32)) -> Result<Self> {
        ff::init().context("could not initialise FFmpeg")?;
        let (width, height) = size;
        let time_base = Rational(1, FPS);
        let (codec_name, pixel) = match format {
            VideoFormat::Hevc => ("hevc_nvenc", format::Pixel::YUV444P),
            VideoFormat::Ffv1 => ("ffv1", format::Pixel::BGRA),
        };
        let codec = encoder::find_by_name(codec_name)
            .ok_or_else(|| anyhow!("this FFmpeg build has no {codec_name} encoder"))?;
        let mut output =
            format::output(path).with_context(|| format!("could not create {}", path.display()))?;
        let global_header = output
            .format()
            .flags()
            .contains(format::Flags::GLOBAL_HEADER);
        let mut stream = output.add_stream(codec)?;
        let mut setup = codec::context::Context::new_with_codec(codec)
            .encoder()
            .video()?;
        setup.set_width(width);
        setup.set_height(height);
        setup.set_format(pixel);
        setup.set_time_base(time_base);
        setup.set_frame_rate(Some(Rational(FPS, 1)));
        setup.set_gop(FPS as u32);
        setup.set_max_b_frames(0);
        let mut options = Dictionary::new();
        match format {
            VideoFormat::Hevc => {
                setup.set_colorspace(ff::util::color::Space::BT709);
                setup.set_color_range(ff::util::color::Range::MPEG);
                setup.set_color_primaries(ff::util::color::Primaries::BT709);
                setup.set_color_transfer_characteristic(
                    ff::util::color::TransferCharacteristic::BT709,
                );
                for (key, value) in [
                    ("preset", "p4"),
                    ("tune", "hq"),
                    ("rc", "constqp"),
                    ("qp", "18"),
                    ("profile", "rext"),
                ] {
                    options.set(key, value);
                }
            }
            VideoFormat::Ffv1 => {
                // Without slice threads FFV1 runs on one core (about 8 fps at 1080p).
                setup.set_threading(codec::threading::Config {
                    kind: codec::threading::Type::Slice,
                    count: 0,
                });
                for (key, value) in [
                    ("level", "3"),
                    ("slices", "16"),
                    ("slicecrc", "1"),
                    ("coder", "1"),
                ] {
                    options.set(key, value);
                }
            }
        }
        if global_header {
            setup.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let encoder = setup.open_with(options).map_err(|err| match format {
            VideoFormat::Hevc => anyhow!(
                "could not open the NVENC HEVC encoder ({err}); it needs an NVIDIA GPU with \
                 NVENC and a free encode session. Try FFV1 instead."
            ),
            VideoFormat::Ffv1 => anyhow!("could not open the FFV1 encoder ({err})"),
        })?;
        stream.set_parameters(&encoder);
        stream.set_time_base(time_base);
        // Declare the constant frame rate so players don't have to infer it.
        stream.set_rate(Rational(FPS, 1));
        stream.set_avg_frame_rate(Rational(FPS, 1));
        let mut header = Dictionary::new();
        if format == VideoFormat::Hevc {
            // Fragmented MP4 stays playable if the recording is interrupted.
            header.set("movflags", "frag_keyframe+empty_moov");
        }
        output
            .write_header_with(header)
            .context("could not write the file header")?;
        let stream_time_base = output.stream(0).context("no video stream")?.time_base();

        let convert = match format {
            VideoFormat::Hevc => {
                let mut scaler = scaling::Context::get(
                    format::Pixel::RGBA,
                    width,
                    height,
                    pixel,
                    width,
                    height,
                    scaling::Flags::BILINEAR,
                )?;
                // swscale defaults to BT.601; match the BT.709 tags (limited range out).
                // SAFETY: the scaler pointer is valid for the life of `scaler`, and the
                // coefficient table returned by FFmpeg is static.
                unsafe {
                    let bt709 = ff::ffi::sws_getCoefficients(ff::ffi::SWS_CS_ITU709);
                    ff::ffi::sws_setColorspaceDetails(
                        scaler.as_mut_ptr(),
                        bt709,
                        1,
                        bt709,
                        0,
                        0,
                        1 << 16,
                        1 << 16,
                    );
                }
                Convert::Scale {
                    scaler,
                    rgba: frame::Video::new(format::Pixel::RGBA, width, height),
                }
            }
            VideoFormat::Ffv1 => Convert::Swizzle,
        };
        Ok(Self {
            output,
            encoder,
            convert,
            frame: frame::Video::new(pixel, width, height),
            size,
            stream_time_base,
        })
    }

    fn write(&mut self, captured: &Frame) -> Result<()> {
        let (width, height) = (self.size.0 as usize, self.size.1 as usize);
        let row = width * 4;
        if captured.rgba.len() != row * height {
            bail!("captured frame has the wrong size");
        }
        // The encoder may still hold a reference to the last frame's buffer.
        // SAFETY: `self.frame` owns a valid AVFrame.
        if unsafe { ff::ffi::av_frame_make_writable(self.frame.as_mut_ptr()) } < 0 {
            bail!("out of memory for the encoder frame");
        }
        match &mut self.convert {
            Convert::Swizzle => {
                let stride = self.frame.stride(0);
                let data = self.frame.data_mut(0);
                for (y, src) in captured.rgba.chunks_exact(row).enumerate() {
                    let dst = &mut data[y * stride..y * stride + row];
                    let (dst, _) = dst.as_chunks_mut::<4>();
                    let (src, _) = src.as_chunks::<4>();
                    for (d, [r, g, b, a]) in dst.iter_mut().zip(src) {
                        *d = [*b, *g, *r, *a];
                    }
                }
            }
            Convert::Scale { scaler, rgba } => {
                let stride = rgba.stride(0);
                let data = rgba.data_mut(0);
                for (y, src) in captured.rgba.chunks_exact(row).enumerate() {
                    data[y * stride..y * stride + row].copy_from_slice(src);
                }
                scaler.run(rgba, &mut self.frame)?;
            }
        }
        self.frame.set_pts(Some(captured.pts));
        self.encoder.send_frame(&self.frame)?;
        self.drain()
    }

    /// Writes every packet the encoder has ready.
    fn drain(&mut self) -> Result<()> {
        let mut packet = Packet::empty();
        loop {
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(0);
                    packet.set_duration(1); // one frame; the MP4 muxer warns without it
                    packet.rescale_ts(Rational(1, FPS), self.stream_time_base);
                    packet.write_interleaved(&mut self.output)?;
                }
                // EAGAIN means the encoder wants more input; Eof follows the final flush.
                Err(ff::Error::Other { errno }) if errno == ff::error::EAGAIN => return Ok(()),
                Err(ff::Error::Eof) => return Ok(()),
                Err(err) => return Err(err.into()),
            }
        }
    }

    fn finish(&mut self) -> Result<()> {
        self.encoder.send_eof()?;
        self.drain()?;
        self.output.write_trailer()?;
        Ok(())
    }
}
```

- [ ] **Step 5: Run the tests and confirm they pass.** Run: `cargo test --test capture`. Expected:
  - `3 passed`.
  - No FFmpeg warnings in the output. Before the packet duration was set, the MP4 muxer printed "Estimating the duration of the last packet".

  Then run `cargo test`: `97 passed` for the unit tests, `3 passed` in `tests/capture.rs`, `3 passed` in `tests/smoke.rs`.

- [ ] **Step 6: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock .cargo/config.toml build.rs src/lib.rs src/capture/mod.rs src/capture/encode.rs tests/capture.rs
git commit -m "feat: add an FFmpeg encoder thread for HEVC and FFV1" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Capture timing and file names

**Files:**
- Modify: `Cargo.toml` (adds `chrono`), `Cargo.lock` (generated), `src/capture/mod.rs`
- Test: unit tests in `src/capture/mod.rs`

**Interfaces:**
- Consumes: `encode::{FPS, VideoFormat}` (Task 4).
- Produces (in `capture`):
  - `OFFLINE_STEP` (1/60 s);
  - `CaptureMode { RealTime, Offline }` with `ALL`, `label()` and `Default` (`RealTime`);
  - `file_name(chrono::NaiveDateTime, VideoFormat) -> String`;
  - `FrameClock` with these methods:
    - `new(mode, start_time)`;
    - `frame_due(time) -> Option<i64>`: in real-time mode, decimates to 60 fps and leaves gaps when rendering is slow. In offline mode, every frame whose time moved.
    - `frames()` and `seconds()`;
    - `reached(stop_after_seconds) -> bool`: counted in whole frames, and 0 means no limit.
  - `padded_bytes_per_row(width)` and `unpad(padded, width, height, &mut Vec<u8>)`.

- [ ] **Step 1: Add `chrono`.** Replace `Cargo.toml` with:

```toml
[package]
name = "rasterwarp"
version = "0.1.0"
edition = "2024"
rust-version = "1.88"

[dependencies]
anyhow = "1"
bytemuck = { version = "1", features = ["derive"] }
chrono = { version = "0.4", default-features = false, features = ["clock"] }
egui = "0.36"
egui-wgpu = "0.36"
egui-winit = "0.36"
env_logger = "0.11"
ffmpeg-next = { version = "9.0.0", default-features = false, features = ["codec", "format", "software-scaling"] }
image = { version = "0.25", default-features = false, features = ["png", "jpeg"] }
log = "0.4"
pollster = "1"
wgpu = "30"
winit = "0.30"
```

- [ ] **Step 2: Write the failing tests.** Append this test module to `src/capture/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_named_after_the_start_time() {
        let t = chrono::NaiveDate::from_ymd_opt(2026, 10, 7)
            .unwrap()
            .and_hms_opt(9, 5, 3)
            .unwrap();
        assert_eq!(
            file_name(t, VideoFormat::Hevc),
            "rasterwarp-20261007-090503.mp4"
        );
        assert_eq!(
            file_name(t, VideoFormat::Ffv1),
            "rasterwarp-20261007-090503.mkv"
        );
    }

    #[test]
    fn real_time_capture_decimates_a_fast_display_to_60_fps() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 10.0);
        let captured: Vec<i64> = (0..143)
            .filter_map(|i| clock.frame_due(10.0 + f64::from(i) / 144.0))
            .collect();
        assert_eq!(captured, (0..60).collect::<Vec<_>>());
        assert!((clock.seconds() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn real_time_capture_keeps_every_frame_of_a_jittery_60_hz_display() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 10.0);
        // ±0.3 ms of timestamp noise around exact 60 Hz frame times.
        let jitter = [0.0003, -0.0003, 0.0001, -0.0002, 0.0];
        let captured: Vec<i64> = (0..600)
            .filter_map(|i| clock.frame_due(10.0 + f64::from(i) / 60.0 + jitter[i as usize % 5]))
            .collect();
        assert_eq!(captured, (0..600).collect::<Vec<_>>());
    }

    #[test]
    fn real_time_capture_leaves_gaps_when_rendering_is_slow() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 0.0);
        let captured: Vec<i64> = (0..4)
            .filter_map(|i| clock.frame_due(f64::from(i) / 30.0))
            .collect();
        assert_eq!(captured, [0, 2, 4, 6]);
    }

    #[test]
    fn paused_time_records_nothing_new() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 0.0);
        assert_eq!(clock.frame_due(0.5), Some(30));
        assert_eq!(clock.frame_due(0.5), None);
    }

    #[test]
    fn offline_capture_records_every_frame() {
        let mut clock = FrameClock::new(CaptureMode::Offline, 3.0);
        let captured: Vec<i64> = (0..5)
            .filter_map(|i| clock.frame_due(3.0 + f64::from(i) * 0.25))
            .collect();
        assert_eq!(captured, [0, 1, 2, 3, 4]);
        assert!((clock.seconds() - 5.0 / 60.0).abs() < 1e-9);
        assert_eq!(clock.frame_due(4.0), None, "paused: time hasn't moved");
    }

    #[test]
    fn stop_after_counts_whole_frames() {
        let mut clock = FrameClock::new(CaptureMode::Offline, 0.0);
        for i in 0..9 {
            clock.frame_due(f64::from(i));
        }
        assert!(!clock.reached(10.0 / 60.0));
        clock.frame_due(9.0);
        assert!(clock.reached(10.0 / 60.0));
        assert!(!clock.reached(0.0), "0 means no limit");
    }

    #[test]
    fn rows_are_padded_to_256_bytes() {
        assert_eq!(padded_bytes_per_row(64), 256);
        assert_eq!(padded_bytes_per_row(65), 512);
        assert_eq!(padded_bytes_per_row(1920), 7680);
    }

    #[test]
    fn unpad_drops_the_row_padding() {
        let (w, h) = (65, 2);
        let stride = padded_bytes_per_row(w) as usize;
        let mut padded = vec![0u8; stride * h as usize];
        padded[0] = 1;
        padded[w as usize * 4 - 1] = 2;
        padded[stride] = 3;
        let mut out = Vec::new();
        unpad(&padded, w, h, &mut out);
        assert_eq!(out.len(), (w * h * 4) as usize);
        assert_eq!(
            (out[0], out[w as usize * 4 - 1], out[w as usize * 4]),
            (1, 2, 3)
        );
    }
}
```

- [ ] **Step 3: Run them and confirm they fail.** Run: `cargo test --lib capture::`. Expected: compile errors such as "cannot find function `file_name`".

- [ ] **Step 4: Implement.** Replace `src/capture/mod.rs` with:

```rust
//! Video capture: recording the clean canvas output to a file.

pub mod encode;

use encode::{FPS, VideoFormat};

/// Animation time per frame while recording offline.
pub const OFFLINE_STEP: f32 = 1.0 / FPS as f32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaptureMode {
    /// Capture at 60 fps from the live output; frames the encoder can't keep up with are
    /// dropped and counted.
    #[default]
    RealTime,
    /// Advance animation time by exactly 1/60 s per rendered frame and capture every
    /// frame, waiting for the encoder when it falls behind.
    Offline,
}

impl CaptureMode {
    pub const ALL: [CaptureMode; 2] = [CaptureMode::RealTime, CaptureMode::Offline];

    pub fn label(self) -> &'static str {
        match self {
            CaptureMode::RealTime => "Real-time",
            CaptureMode::Offline => "Offline (frame-accurate)",
        }
    }
}

/// `rasterwarp-YYYYMMDD-HHMMSS.<ext>` for a recording started at `now` (local time).
pub fn file_name(now: chrono::NaiveDateTime, format: VideoFormat) -> String {
    format!(
        "rasterwarp-{}.{}",
        now.format("%Y%m%d-%H%M%S"),
        format.extension()
    )
}

/// Decides which rendered frames are recorded and their timestamps (in frames at 60 fps).
#[derive(Clone, Debug)]
pub struct FrameClock {
    mode: CaptureMode,
    /// Animation time when the recording started.
    start: f64,
    /// Animation time of the last recorded frame.
    last: f64,
    /// The next timestamp to hand out; also the recording's length in frames.
    next: i64,
}

impl FrameClock {
    pub fn new(mode: CaptureMode, start: f64) -> Self {
        Self {
            mode,
            start,
            last: start,
            next: 0,
        }
    }

    /// The timestamp for a frame rendered at animation time `time`, or `None` to skip it.
    /// Real-time mode records a frame whenever `(time − start) · 60`, rounded to the nearest
    /// frame, moves on (rounding keeps a 60 Hz display's jittery frame times inside their own
    /// frame). Offline mode records every frame whose time has moved on (so pausing records
    /// nothing).
    pub fn frame_due(&mut self, time: f64) -> Option<i64> {
        let pts = match self.mode {
            CaptureMode::Offline => {
                if self.next > 0 && time <= self.last {
                    return None;
                }
                self.next
            }
            CaptureMode::RealTime => {
                let n = ((time - self.start) * f64::from(FPS) + 0.5).floor() as i64;
                if n < self.next {
                    return None;
                }
                n
            }
        };
        self.next = pts + 1;
        self.last = time;
        Some(pts)
    }

    /// The recording's length in frames so far.
    pub fn frames(&self) -> i64 {
        self.next
    }

    /// The recording's length in seconds so far.
    pub fn seconds(&self) -> f64 {
        self.next as f64 / f64::from(FPS)
    }

    /// Whether a recording limited to `seconds` (0 = no limit) is long enough. Counted
    /// in whole frames, so 10/60 s stops after exactly 10 frames.
    pub fn reached(&self, seconds: f32) -> bool {
        seconds > 0.0 && self.next >= (f64::from(seconds) * f64::from(FPS)).round() as i64
    }
}

/// Bytes per row in a texture-to-buffer copy: `width × 4` rounded up to 256.
pub fn padded_bytes_per_row(width: u32) -> u32 {
    (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

/// Copies `height` rows of `width` RGBA pixels out of a padded buffer into `out`.
pub fn unpad(padded: &[u8], width: u32, height: u32, out: &mut Vec<u8>) {
    let row = width as usize * 4;
    let stride = padded_bytes_per_row(width) as usize;
    out.clear();
    for r in padded.chunks(stride).take(height as usize) {
        out.extend_from_slice(&r[..row]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_named_after_the_start_time() {
        let t = chrono::NaiveDate::from_ymd_opt(2026, 10, 7)
            .unwrap()
            .and_hms_opt(9, 5, 3)
            .unwrap();
        assert_eq!(
            file_name(t, VideoFormat::Hevc),
            "rasterwarp-20261007-090503.mp4"
        );
        assert_eq!(
            file_name(t, VideoFormat::Ffv1),
            "rasterwarp-20261007-090503.mkv"
        );
    }

    #[test]
    fn real_time_capture_decimates_a_fast_display_to_60_fps() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 10.0);
        let captured: Vec<i64> = (0..143)
            .filter_map(|i| clock.frame_due(10.0 + f64::from(i) / 144.0))
            .collect();
        assert_eq!(captured, (0..60).collect::<Vec<_>>());
        assert!((clock.seconds() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn real_time_capture_keeps_every_frame_of_a_jittery_60_hz_display() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 10.0);
        // ±0.3 ms of timestamp noise around exact 60 Hz frame times.
        let jitter = [0.0003, -0.0003, 0.0001, -0.0002, 0.0];
        let captured: Vec<i64> = (0..600)
            .filter_map(|i| clock.frame_due(10.0 + f64::from(i) / 60.0 + jitter[i as usize % 5]))
            .collect();
        assert_eq!(captured, (0..600).collect::<Vec<_>>());
    }

    #[test]
    fn real_time_capture_leaves_gaps_when_rendering_is_slow() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 0.0);
        let captured: Vec<i64> = (0..4)
            .filter_map(|i| clock.frame_due(f64::from(i) / 30.0))
            .collect();
        assert_eq!(captured, [0, 2, 4, 6]);
    }

    #[test]
    fn paused_time_records_nothing_new() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 0.0);
        assert_eq!(clock.frame_due(0.5), Some(30));
        assert_eq!(clock.frame_due(0.5), None);
    }

    #[test]
    fn offline_capture_records_every_frame() {
        let mut clock = FrameClock::new(CaptureMode::Offline, 3.0);
        let captured: Vec<i64> = (0..5)
            .filter_map(|i| clock.frame_due(3.0 + f64::from(i) * 0.25))
            .collect();
        assert_eq!(captured, [0, 1, 2, 3, 4]);
        assert!((clock.seconds() - 5.0 / 60.0).abs() < 1e-9);
        assert_eq!(clock.frame_due(4.0), None, "paused: time hasn't moved");
    }

    #[test]
    fn stop_after_counts_whole_frames() {
        let mut clock = FrameClock::new(CaptureMode::Offline, 0.0);
        for i in 0..9 {
            clock.frame_due(f64::from(i));
        }
        assert!(!clock.reached(10.0 / 60.0));
        clock.frame_due(9.0);
        assert!(clock.reached(10.0 / 60.0));
        assert!(!clock.reached(0.0), "0 means no limit");
    }

    #[test]
    fn rows_are_padded_to_256_bytes() {
        assert_eq!(padded_bytes_per_row(64), 256);
        assert_eq!(padded_bytes_per_row(65), 512);
        assert_eq!(padded_bytes_per_row(1920), 7680);
    }

    #[test]
    fn unpad_drops_the_row_padding() {
        let (w, h) = (65, 2);
        let stride = padded_bytes_per_row(w) as usize;
        let mut padded = vec![0u8; stride * h as usize];
        padded[0] = 1;
        padded[w as usize * 4 - 1] = 2;
        padded[stride] = 3;
        let mut out = Vec::new();
        unpad(&padded, w, h, &mut out);
        assert_eq!(out.len(), (w * h * 4) as usize);
        assert_eq!(
            (out[0], out[w as usize * 4 - 1], out[w as usize * 4]),
            (1, 2, 3)
        );
    }
}
```

- [ ] **Step 5: Run the tests and confirm they pass.** Run: `cargo test --lib capture::`. Expected: `8 passed`. Then `cargo test`: `105 passed` for the unit tests, `3 passed` in `tests/capture.rs`, `3 passed` in `tests/smoke.rs`.

- [ ] **Step 6: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/capture/mod.rs
git commit -m "feat: add capture frame timing and file naming" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: GPU readback and the recorder

**Files:**
- Create: `src/capture/readback.rs`, `src/capture/recorder.rs`
- Modify: `src/capture/mod.rs`, `src/passes/mod.rs`, `tests/smoke.rs`
- Test: unit tests in `src/capture/readback.rs`; `records_every_frame_offline` and `real_time_capture_counts_frames_it_has_to_drop` in `tests/smoke.rs`

**Interfaces:**
- Consumes: `Encoder`, `Frame`, `VideoFormat` (Task 4); `CaptureMode`, `FrameClock`, `file_name`, `padded_bytes_per_row`, `unpad` (Task 5); `Renderer` (Task 3).
- Produces:
  - `passes::CAPTURE_FORMAT` (`Rgba8UnormSrgb`) and `Renderer::composite_capture(device, queue, encoder, &FrameParams, time, &view)`. The latter draws the same composite into a canvas-sized capture texture, unletterboxed.
  - In `capture::readback`:
    - `SLOTS` (3) and `SlotState`;
    - `Ring` with `claim(pts) -> Option<usize>`, `start_mapping()`, `oldest_mapping()`, `release(i)` and `is_idle()`;
    - `Readback` with `new(device, size)`, `view()`, `copy(encoder, pts) -> bool`, `after_submit()`, `is_busy()` and `collect(device, wait, buffer_fn) -> Vec<Frame>`.
  - In `capture::recorder`:
    - `RecordSettings { format, mode, folder, stop_after }`;
    - `RecordStatus { path, seconds, frames, dropped, speed }`;
    - `Recorder` with these methods:
      - `start(device, &RecordSettings, canvas, time) -> Result<Recorder>`;
      - `mode()` and `pump(device) -> Result<()>`;
      - `capture(device, encoder, time, draw: FnOnce(&mut CommandEncoder, &TextureView)) -> Result<()>`;
      - `after_submit()`, `finished_recording()` and `status()`;
      - `finish(self, device) -> Result<RecordStatus>`.

> **How a frame flows**, once per drawn frame:
> 1. `pump` hands finished readbacks to the encoder.
> 2. The app renders.
> 3. `capture` asks the `FrameClock` whether a frame is due. If it is, `draw` draws the capture composite and the texture is copied into the next ring slot.
> 4. After `queue.submit`, `after_submit` starts mapping that slot.
>
> **When something falls behind:**
> - In real-time mode, a busy ring or a full encoder queue drops the frame and counts it.
> - In offline mode, `capture` waits for the GPU, and sends block.

- [ ] **Step 1: Register the modules.** Replace `src/capture/mod.rs` with:

```rust
//! Video capture: recording the clean canvas output to a file.

pub mod encode;
pub mod readback;
pub mod recorder;

use encode::{FPS, VideoFormat};

/// Animation time per frame while recording offline.
pub const OFFLINE_STEP: f32 = 1.0 / FPS as f32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaptureMode {
    /// Capture at 60 fps from the live output; frames the encoder can't keep up with are
    /// dropped and counted.
    #[default]
    RealTime,
    /// Advance animation time by exactly 1/60 s per rendered frame and capture every
    /// frame, waiting for the encoder when it falls behind.
    Offline,
}

impl CaptureMode {
    pub const ALL: [CaptureMode; 2] = [CaptureMode::RealTime, CaptureMode::Offline];

    pub fn label(self) -> &'static str {
        match self {
            CaptureMode::RealTime => "Real-time",
            CaptureMode::Offline => "Offline (frame-accurate)",
        }
    }
}

/// `rasterwarp-YYYYMMDD-HHMMSS.<ext>` for a recording started at `now` (local time).
pub fn file_name(now: chrono::NaiveDateTime, format: VideoFormat) -> String {
    format!(
        "rasterwarp-{}.{}",
        now.format("%Y%m%d-%H%M%S"),
        format.extension()
    )
}

/// Decides which rendered frames are recorded and their timestamps (in frames at 60 fps).
#[derive(Clone, Debug)]
pub struct FrameClock {
    mode: CaptureMode,
    /// Animation time when the recording started.
    start: f64,
    /// Animation time of the last recorded frame.
    last: f64,
    /// The next timestamp to hand out; also the recording's length in frames.
    next: i64,
}

impl FrameClock {
    pub fn new(mode: CaptureMode, start: f64) -> Self {
        Self {
            mode,
            start,
            last: start,
            next: 0,
        }
    }

    /// The timestamp for a frame rendered at animation time `time`, or `None` to skip it.
    /// Real-time mode records a frame whenever `(time − start) · 60`, rounded to the nearest
    /// frame, moves on (rounding keeps a 60 Hz display's jittery frame times inside their own
    /// frame). Offline mode records every frame whose time has moved on (so pausing records
    /// nothing).
    pub fn frame_due(&mut self, time: f64) -> Option<i64> {
        let pts = match self.mode {
            CaptureMode::Offline => {
                if self.next > 0 && time <= self.last {
                    return None;
                }
                self.next
            }
            CaptureMode::RealTime => {
                let n = ((time - self.start) * f64::from(FPS) + 0.5).floor() as i64;
                if n < self.next {
                    return None;
                }
                n
            }
        };
        self.next = pts + 1;
        self.last = time;
        Some(pts)
    }

    /// The recording's length in frames so far.
    pub fn frames(&self) -> i64 {
        self.next
    }

    /// The recording's length in seconds so far.
    pub fn seconds(&self) -> f64 {
        self.next as f64 / f64::from(FPS)
    }

    /// Whether a recording limited to `seconds` (0 = no limit) is long enough. Counted
    /// in whole frames, so 10/60 s stops after exactly 10 frames.
    pub fn reached(&self, seconds: f32) -> bool {
        seconds > 0.0 && self.next >= (f64::from(seconds) * f64::from(FPS)).round() as i64
    }
}

/// Bytes per row in a texture-to-buffer copy: `width × 4` rounded up to 256.
pub fn padded_bytes_per_row(width: u32) -> u32 {
    (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

/// Copies `height` rows of `width` RGBA pixels out of a padded buffer into `out`.
pub fn unpad(padded: &[u8], width: u32, height: u32, out: &mut Vec<u8>) {
    let row = width as usize * 4;
    let stride = padded_bytes_per_row(width) as usize;
    out.clear();
    for r in padded.chunks(stride).take(height as usize) {
        out.extend_from_slice(&r[..row]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_named_after_the_start_time() {
        let t = chrono::NaiveDate::from_ymd_opt(2026, 10, 7)
            .unwrap()
            .and_hms_opt(9, 5, 3)
            .unwrap();
        assert_eq!(
            file_name(t, VideoFormat::Hevc),
            "rasterwarp-20261007-090503.mp4"
        );
        assert_eq!(
            file_name(t, VideoFormat::Ffv1),
            "rasterwarp-20261007-090503.mkv"
        );
    }

    #[test]
    fn real_time_capture_decimates_a_fast_display_to_60_fps() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 10.0);
        let captured: Vec<i64> = (0..143)
            .filter_map(|i| clock.frame_due(10.0 + f64::from(i) / 144.0))
            .collect();
        assert_eq!(captured, (0..60).collect::<Vec<_>>());
        assert!((clock.seconds() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn real_time_capture_keeps_every_frame_of_a_jittery_60_hz_display() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 10.0);
        // ±0.3 ms of timestamp noise around exact 60 Hz frame times.
        let jitter = [0.0003, -0.0003, 0.0001, -0.0002, 0.0];
        let captured: Vec<i64> = (0..600)
            .filter_map(|i| clock.frame_due(10.0 + f64::from(i) / 60.0 + jitter[i as usize % 5]))
            .collect();
        assert_eq!(captured, (0..600).collect::<Vec<_>>());
    }

    #[test]
    fn real_time_capture_leaves_gaps_when_rendering_is_slow() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 0.0);
        let captured: Vec<i64> = (0..4)
            .filter_map(|i| clock.frame_due(f64::from(i) / 30.0))
            .collect();
        assert_eq!(captured, [0, 2, 4, 6]);
    }

    #[test]
    fn paused_time_records_nothing_new() {
        let mut clock = FrameClock::new(CaptureMode::RealTime, 0.0);
        assert_eq!(clock.frame_due(0.5), Some(30));
        assert_eq!(clock.frame_due(0.5), None);
    }

    #[test]
    fn offline_capture_records_every_frame() {
        let mut clock = FrameClock::new(CaptureMode::Offline, 3.0);
        let captured: Vec<i64> = (0..5)
            .filter_map(|i| clock.frame_due(3.0 + f64::from(i) * 0.25))
            .collect();
        assert_eq!(captured, [0, 1, 2, 3, 4]);
        assert!((clock.seconds() - 5.0 / 60.0).abs() < 1e-9);
        assert_eq!(clock.frame_due(4.0), None, "paused: time hasn't moved");
    }

    #[test]
    fn stop_after_counts_whole_frames() {
        let mut clock = FrameClock::new(CaptureMode::Offline, 0.0);
        for i in 0..9 {
            clock.frame_due(f64::from(i));
        }
        assert!(!clock.reached(10.0 / 60.0));
        clock.frame_due(9.0);
        assert!(clock.reached(10.0 / 60.0));
        assert!(!clock.reached(0.0), "0 means no limit");
    }

    #[test]
    fn rows_are_padded_to_256_bytes() {
        assert_eq!(padded_bytes_per_row(64), 256);
        assert_eq!(padded_bytes_per_row(65), 512);
        assert_eq!(padded_bytes_per_row(1920), 7680);
    }

    #[test]
    fn unpad_drops_the_row_padding() {
        let (w, h) = (65, 2);
        let stride = padded_bytes_per_row(w) as usize;
        let mut padded = vec![0u8; stride * h as usize];
        padded[0] = 1;
        padded[w as usize * 4 - 1] = 2;
        padded[stride] = 3;
        let mut out = Vec::new();
        unpad(&padded, w, h, &mut out);
        assert_eq!(out.len(), (w * h * 4) as usize);
        assert_eq!(
            (out[0], out[w as usize * 4 - 1], out[w as usize * 4]),
            (1, 2, 3)
        );
    }
}
```

- [ ] **Step 2: Write the failing tests.** Create `src/capture/readback.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_used_in_rotation() {
        let mut ring = Ring::default();
        assert_eq!(ring.claim(0), Some(0));
        assert_eq!(ring.claim(1), Some(1));
        assert_eq!(ring.claim(2), Some(2));
        assert_eq!(ring.start_mapping(), [0, 1, 2]);
        ring.release(1);
        assert_eq!(
            ring.claim(3),
            None,
            "slot 0 is next in rotation and still busy"
        );
        ring.release(0);
        assert_eq!(ring.claim(3), Some(0));
        assert_eq!(ring.claim(4), Some(1));
    }

    #[test]
    fn a_full_ring_refuses_new_frames() {
        let mut ring = Ring::default();
        for pts in 0..SLOTS as i64 {
            assert!(ring.claim(pts).is_some());
        }
        assert_eq!(ring.claim(9), None);
        ring.start_mapping();
        let (slot, pts) = ring.oldest_mapping().unwrap();
        assert_eq!((slot, pts), (0, 0));
        ring.release(slot);
        assert_eq!(ring.claim(9), Some(0));
    }

    #[test]
    fn oldest_frame_comes_back_first() {
        let mut ring = Ring::default();
        ring.claim(5);
        ring.claim(6);
        ring.start_mapping();
        ring.release(0);
        ring.claim(7);
        ring.start_mapping();
        assert_eq!(ring.oldest_mapping(), Some((1, 6)));
        assert!(!ring.is_idle());
    }
}
```

Run: `cargo test --lib capture::readback`. Expected: compile errors such as "cannot find type `Ring`". (Create an empty `src/capture/recorder.rs` for now so the module declaration resolves.)

- [ ] **Step 3: Implement the readback ring.** Replace `src/capture/readback.rs` with:

```rust
//! Copies captured frames from the GPU into memory without stalling rendering: the
//! capture texture is copied into one of a few staging buffers, which are mapped
//! asynchronously and read a frame or two later.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::encode::Frame;
use super::{padded_bytes_per_row, unpad};
use crate::gpu::RenderTarget;
use crate::passes::CAPTURE_FORMAT;

/// Staging buffers in the ring.
pub const SLOTS: usize = 3;

/// Where one staging buffer is in its round trip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotState {
    Free,
    /// A copy of frame `pts` is recorded; mapping starts after the submit.
    Copied(i64),
    /// Mapping of frame `pts` has been requested.
    Mapping(i64),
}

/// The slots, used strictly in rotation so frames come back in order.
#[derive(Clone, Debug)]
pub struct Ring {
    states: [SlotState; SLOTS],
    next: usize,
}

impl Default for Ring {
    fn default() -> Self {
        Self {
            states: [SlotState::Free; SLOTS],
            next: 0,
        }
    }
}

impl Ring {
    /// Claims the next slot in rotation for frame `pts`, or `None` while it's still busy.
    pub fn claim(&mut self, pts: i64) -> Option<usize> {
        let i = self.next;
        if self.states[i] != SlotState::Free {
            return None;
        }
        self.states[i] = SlotState::Copied(pts);
        self.next = (i + 1) % SLOTS;
        Some(i)
    }

    /// Marks every copied slot as mapping and returns them.
    pub fn start_mapping(&mut self) -> Vec<usize> {
        let mut started = Vec::new();
        for (i, state) in self.states.iter_mut().enumerate() {
            if let SlotState::Copied(pts) = *state {
                *state = SlotState::Mapping(pts);
                started.push(i);
            }
        }
        started
    }

    /// The mapping slot holding the oldest frame, with its timestamp.
    pub fn oldest_mapping(&self) -> Option<(usize, i64)> {
        self.states
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s {
                SlotState::Mapping(pts) => Some((i, *pts)),
                _ => None,
            })
            .min_by_key(|(_, pts)| *pts)
    }

    pub fn release(&mut self, i: usize) {
        self.states[i] = SlotState::Free;
    }

    pub fn is_idle(&self) -> bool {
        self.states.iter().all(|s| *s == SlotState::Free)
    }
}

pub struct Readback {
    target: RenderTarget,
    buffers: Vec<wgpu::Buffer>,
    mapped: Vec<Arc<AtomicBool>>,
    ring: Ring,
    size: (u32, u32),
}

impl Readback {
    /// A capture texture of `size` in [`CAPTURE_FORMAT`], plus the staging ring.
    pub fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let bytes = u64::from(padded_bytes_per_row(size.0)) * u64::from(size.1);
        let buffers = (0..SLOTS)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("capture staging"),
                    size: bytes,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })
            })
            .collect();
        Self {
            target: RenderTarget::new(device, "capture", size.0, size.1, CAPTURE_FORMAT),
            buffers,
            mapped: (0..SLOTS)
                .map(|_| Arc::new(AtomicBool::new(false)))
                .collect(),
            ring: Ring::default(),
            size,
        }
    }

    /// The texture the capture composite draws into.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.target.view
    }

    /// Records a copy of the capture texture for frame `pts`. Returns false, recording
    /// nothing, when every staging buffer is still busy.
    pub fn copy(&mut self, encoder: &mut wgpu::CommandEncoder, pts: i64) -> bool {
        let Some(slot) = self.ring.claim(pts) else {
            return false;
        };
        encoder.copy_texture_to_buffer(
            self.target.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffers[slot],
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row(self.size.0)),
                    rows_per_image: Some(self.size.1),
                },
            },
            self.target.texture.size(),
        );
        true
    }

    /// Starts mapping the buffers copied into since the last call. Call after submitting
    /// the commands that contain the copies.
    pub fn after_submit(&mut self) {
        for slot in self.ring.start_mapping() {
            let mapped = self.mapped[slot].clone();
            mapped.store(false, Ordering::Release);
            self.buffers[slot].map_async(wgpu::MapMode::Read, .., move |result| {
                if result.is_ok() {
                    mapped.store(true, Ordering::Release);
                }
            });
        }
    }

    /// Whether any frame is still on its way back from the GPU.
    pub fn is_busy(&self) -> bool {
        !self.ring.is_idle()
    }

    /// Returns the frames that have arrived, oldest first. With `wait`, blocks until
    /// everything submitted so far has arrived. `buffer` supplies reusable memory.
    pub fn collect(
        &mut self,
        device: &wgpu::Device,
        wait: bool,
        mut buffer: impl FnMut() -> Vec<u8>,
    ) -> Vec<Frame> {
        let poll = if wait {
            wgpu::PollType::wait_indefinitely()
        } else {
            wgpu::PollType::Poll
        };
        if let Err(err) = device.poll(poll) {
            log::warn!("capture readback poll failed: {err}");
        }
        let mut frames = Vec::new();
        while let Some((slot, pts)) = self.ring.oldest_mapping() {
            if !self.mapped[slot].load(Ordering::Acquire) {
                break;
            }
            let mut rgba = buffer();
            {
                let data = self.buffers[slot]
                    .get_mapped_range(..)
                    .expect("capture buffer is mapped");
                unpad(&data, self.size.0, self.size.1, &mut rgba);
            }
            self.buffers[slot].unmap();
            self.ring.release(slot);
            frames.push(Frame { pts, rgba });
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_used_in_rotation() {
        let mut ring = Ring::default();
        assert_eq!(ring.claim(0), Some(0));
        assert_eq!(ring.claim(1), Some(1));
        assert_eq!(ring.claim(2), Some(2));
        assert_eq!(ring.start_mapping(), [0, 1, 2]);
        ring.release(1);
        assert_eq!(
            ring.claim(3),
            None,
            "slot 0 is next in rotation and still busy"
        );
        ring.release(0);
        assert_eq!(ring.claim(3), Some(0));
        assert_eq!(ring.claim(4), Some(1));
    }

    #[test]
    fn a_full_ring_refuses_new_frames() {
        let mut ring = Ring::default();
        for pts in 0..SLOTS as i64 {
            assert!(ring.claim(pts).is_some());
        }
        assert_eq!(ring.claim(9), None);
        ring.start_mapping();
        let (slot, pts) = ring.oldest_mapping().unwrap();
        assert_eq!((slot, pts), (0, 0));
        ring.release(slot);
        assert_eq!(ring.claim(9), Some(0));
    }

    #[test]
    fn oldest_frame_comes_back_first() {
        let mut ring = Ring::default();
        ring.claim(5);
        ring.claim(6);
        ring.start_mapping();
        ring.release(0);
        ring.claim(7);
        ring.start_mapping();
        assert_eq!(ring.oldest_mapping(), Some((1, 6)));
        assert!(!ring.is_idle());
    }
}
```

Add the capture composite to the renderer by replacing `src/passes/mod.rs` with:

```rust
//! The render pipeline: source -> warp -> colorize -> feedback -> bloom -> composite.

pub mod bloom;
pub mod colorize;
pub mod composite;
pub mod feedback;
pub mod warp;

use crate::blend::FrameParams;
use crate::source::{self, GrayImage};

/// The format of the capture texture that recordings are read from.
pub const CAPTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

pub struct Renderer {
    size: (u32, u32),
    source: wgpu::TextureView,
    source_aspect: f32,
    warp: warp::WarpPass,
    colorize: colorize::ColorizePass,
    feedback: feedback::FeedbackPass,
    bloom: bloom::BloomPass,
    composite: composite::CompositePass,
    /// Draws the same composite into a canvas-sized capture texture.
    capture: composite::CompositePass,
}

impl Renderer {
    /// `size` is the internal (canvas) resolution; `output_format` is the format of the
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
            capture: composite::CompositePass::new(device, CAPTURE_FORMAT),
        }
    }

    /// The internal (canvas) resolution.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Rebuilds every canvas-sized target at `size`. The trails start out cleared.
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let (w, h) = size;
        self.size = size;
        self.warp = warp::WarpPass::new(device, w, h);
        self.colorize = colorize::ColorizePass::new(device, w, h);
        self.feedback = feedback::FeedbackPass::new(device, w, h);
        self.bloom = bloom::BloomPass::new(device, w, h);
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

    /// Draws this frame's composite again into `output`: a canvas-sized texture in
    /// [`CAPTURE_FORMAT`], with no letterboxing and no UI. Call after `render`, in the
    /// same encoder.
    pub fn composite_capture(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameParams,
        time: f32,
        output: &wgpu::TextureView,
    ) {
        self.capture.render(
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
                self.size,
                self.capture.encode_srgb(),
            ),
        );
    }
}
```

Run: `cargo test --lib capture::readback`. Expected: `3 passed`.

- [ ] **Step 4: Write the failing recorder tests.** Replace `tests/smoke.rs` with (adds `records_every_frame_offline` and `real_time_capture_counts_frames_it_has_to_drop`):

```rust
//! Headless GPU smoke tests: build every pass on a real device, render a few frames
//! offscreen, and read results back. wgpu validation errors panic, failing the test.

use rasterwarp::blend::FrameParams;
use rasterwarp::capture::encode::VideoFormat;
use rasterwarp::capture::recorder::{RecordSettings, Recorder};
use rasterwarp::capture::{CaptureMode, OFFLINE_STEP};
use rasterwarp::gpu;
use rasterwarp::motion::{Mode, Motion};
use rasterwarp::params::Params;
use rasterwarp::passes::Renderer;
use rasterwarp::preview::PreviewView;
use rasterwarp::source::test_card;

const OUT_W: u32 = 320;
const OUT_H: u32 = 180;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let backends = wgpu::Backends::all();
    let instance = gpu::create_instance(backends);
    match pollster::block_on(gpu::request_device(&instance, backends, None)) {
        Ok((_adapter, device, queue)) => Some((device, queue)),
        Err(_) => {
            eprintln!("skipping smoke test: no GPU adapter available");
            None
        }
    }
}

#[test]
fn renders_frames_offscreen() {
    let Some((device, queue)) = device() else {
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

#[test]
fn renders_after_a_canvas_resize() {
    let Some((device, queue)) = device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(&device, "smoke output", OUT_W, OUT_H, format);
    let mut renderer = Renderer::new(&device, &queue, format, (640, 360), &test_card(400, 300));
    renderer.resize(&device, (256, 192));
    assert_eq!(renderer.size(), (256, 192));
    let frame_params = FrameParams::at_rest(&Params::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(
        &device,
        &queue,
        &mut encoder,
        &frame_params,
        0.0,
        &output.view,
        (OUT_W, OUT_H),
    );
    queue.submit([encoder.finish()]);
    let pixels = read_back(&device, &queue, &output.texture);
    let (rgba, _) = pixels.as_chunks::<4>();
    assert!(
        rgba.iter().any(|px| *px != rgba[0]),
        "output is a single flat color"
    );
}

#[test]
fn renders_the_preview_offscreen() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut egui_renderer = egui_wgpu::Renderer::new(
        &device,
        wgpu::TextureFormat::Bgra8Unorm,
        egui_wgpu::RendererOptions::default(),
    );
    let mut preview = PreviewView::new(
        &device,
        &queue,
        &mut egui_renderer,
        (1920, 1080),
        &test_card(400, 300),
    );
    assert_eq!(preview.size(), (480, 270));
    let mut motion = Motion::new(Params::default());
    motion.set_mode(Mode::Transition);
    let shown = motion.preview().expect("Transition mode has a preview");
    for frame in 0..3 {
        let mut encoder = device.create_command_encoder(&Default::default());
        preview.render(&device, &queue, &mut encoder, &shown, frame as f32 * 0.1);
        queue.submit([encoder.finish()]);
    }
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll device");
}

#[test]
fn records_every_frame_offline() {
    let Some((device, queue)) = device() else {
        return;
    };
    let folder = std::env::temp_dir().join(format!("rasterwarp-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let settings = RecordSettings {
        format: VideoFormat::Ffv1,
        mode: CaptureMode::Offline,
        folder: folder.clone(),
        stop_after: 10.0 / 60.0,
    };
    let canvas = (256, 144);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(&device, "smoke output", OUT_W, OUT_H, format);
    let mut renderer = Renderer::new(&device, &queue, format, canvas, &test_card(400, 300));
    let frame_params = FrameParams::at_rest(&Params::default());
    let mut recorder = Recorder::start(&device, &settings, canvas, 0.0).expect("start recording");
    let mut time = 0.0;
    while !recorder.finished_recording() {
        recorder.pump(&device).expect("encoder running");
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(
            &device,
            &queue,
            &mut encoder,
            &frame_params,
            time as f32,
            &output.view,
            (OUT_W, OUT_H),
        );
        recorder
            .capture(&device, &mut encoder, time, |encoder, view| {
                renderer.composite_capture(
                    &device,
                    &queue,
                    encoder,
                    &frame_params,
                    time as f32,
                    view,
                )
            })
            .expect("capture");
        queue.submit([encoder.finish()]);
        recorder.after_submit();
        time += f64::from(OFFLINE_STEP);
    }
    let status = recorder.finish(&device).expect("finish recording");
    assert_eq!(status.frames, 10);
    assert_eq!(status.dropped, 0);
    assert!(status.path.starts_with(&folder));
    assert!(std::fs::metadata(&status.path).expect("file written").len() > 0);
}

#[test]
fn real_time_capture_counts_frames_it_has_to_drop() {
    let Some((device, queue)) = device() else {
        return;
    };
    let folder = std::env::temp_dir().join(format!("rasterwarp-drops-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let settings = RecordSettings {
        format: VideoFormat::Ffv1,
        mode: CaptureMode::RealTime,
        folder,
        stop_after: 0.0,
    };
    let canvas = (256, 144);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(&device, "smoke output", OUT_W, OUT_H, format);
    let mut renderer = Renderer::new(&device, &queue, format, canvas, &test_card(400, 300));
    let frame_params = FrameParams::at_rest(&Params::default());
    let mut recorder = Recorder::start(&device, &settings, canvas, 0.0).expect("start recording");
    // Five frames, but the readbacks aren't started until the end, so the three staging
    // buffers fill up and the last two frames are dropped.
    for frame in 0..5 {
        let time = f64::from(frame) / 60.0;
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(
            &device,
            &queue,
            &mut encoder,
            &frame_params,
            time as f32,
            &output.view,
            (OUT_W, OUT_H),
        );
        recorder
            .capture(&device, &mut encoder, time, |encoder, view| {
                renderer.composite_capture(
                    &device,
                    &queue,
                    encoder,
                    &frame_params,
                    time as f32,
                    view,
                )
            })
            .expect("capture");
        queue.submit([encoder.finish()]);
    }
    recorder.after_submit();
    let status = recorder.finish(&device).expect("finish recording");
    assert_eq!((status.frames, status.dropped), (3, 2));
}

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
/// that buffer copies require.
fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let row = width * 4;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded * height),
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
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    buffer.map_async(wgpu::MapMode::Read, .., |result| {
        result.expect("map readback buffer")
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll device");
    let mapped = buffer.get_mapped_range(..).expect("mapped range");
    mapped
        .chunks(padded as usize)
        .flat_map(|r| &r[..row as usize])
        .copied()
        .collect()
}
```

Run: `cargo test --test smoke`. Expected: compile errors such as "cannot find type `Recorder`".

- [ ] **Step 5: Implement the recorder.** Replace `src/capture/recorder.rs` with:

```rust
//! One recording: decides which frames to capture, reads them back from the GPU and
//! feeds them to the encoder thread.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};

use super::encode::{Encoder, Frame, VideoFormat};
use super::readback::Readback;
use super::{CaptureMode, FrameClock, file_name};

/// What the panel chose before pressing Record.
#[derive(Clone, Debug)]
pub struct RecordSettings {
    pub format: VideoFormat,
    pub mode: CaptureMode,
    pub folder: PathBuf,
    /// Stop automatically after this many seconds of video (0 = stop by hand).
    pub stop_after: f32,
}

/// Progress shown while recording.
#[derive(Clone, Debug)]
pub struct RecordStatus {
    pub path: PathBuf,
    /// Seconds of video recorded so far.
    pub seconds: f64,
    /// Frames handed to the encoder.
    pub frames: u64,
    /// Frames lost because the encoder or the GPU readback fell behind (real-time only).
    pub dropped: u64,
    /// Offline only: seconds of video per second of wall time.
    pub speed: Option<f64>,
}

pub struct Recorder {
    encoder: Encoder,
    readback: Readback,
    clock: FrameClock,
    mode: CaptureMode,
    stop_after: f32,
    path: PathBuf,
    started: Instant,
    frames: u64,
    dropped: u64,
    /// Frame buffers ready for reuse.
    spare: Vec<Vec<u8>>,
}

impl Recorder {
    /// Opens a new file in the settings' folder and starts encoding. `time` is the
    /// current animation time.
    pub fn start(
        device: &wgpu::Device,
        settings: &RecordSettings,
        canvas: (u32, u32),
        time: f64,
    ) -> Result<Self> {
        std::fs::create_dir_all(&settings.folder)
            .with_context(|| format!("could not create {}", settings.folder.display()))?;
        let now = chrono::Local::now().naive_local();
        let path = settings.folder.join(file_name(now, settings.format));
        let encoder = Encoder::start(&path, settings.format, canvas)?;
        Ok(Self {
            encoder,
            readback: Readback::new(device, canvas),
            clock: FrameClock::new(settings.mode, time),
            mode: settings.mode,
            stop_after: settings.stop_after,
            path,
            started: Instant::now(),
            frames: 0,
            dropped: 0,
            spare: Vec::new(),
        })
    }

    pub fn mode(&self) -> CaptureMode {
        self.mode
    }

    /// Call once per frame before rendering: hands frames that have come back from the
    /// GPU to the encoder. An error means the encoder has stopped.
    pub fn pump(&mut self, device: &wgpu::Device) -> Result<()> {
        self.deliver(device, false)
    }

    /// Captures the frame rendered at animation time `time` if one is due: `draw` must
    /// draw the capture composite into the given view. In offline mode this waits for
    /// a free staging buffer; in real-time mode a busy ring drops the frame.
    pub fn capture(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        time: f64,
        draw: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView),
    ) -> Result<()> {
        let Some(pts) = self.clock.frame_due(time) else {
            return Ok(());
        };
        draw(encoder, self.readback.view());
        if self.readback.copy(encoder, pts) {
            return Ok(());
        }
        match self.mode {
            CaptureMode::RealTime => self.dropped += 1,
            CaptureMode::Offline => {
                self.deliver(device, true)?;
                let copied = self.readback.copy(encoder, pts);
                debug_assert!(copied, "a staging buffer is free after waiting");
            }
        }
        Ok(())
    }

    /// Call after submitting the frame's commands.
    pub fn after_submit(&mut self) {
        self.readback.after_submit();
    }

    /// Whether the "stop after" length has been reached.
    pub fn finished_recording(&self) -> bool {
        self.clock.reached(self.stop_after)
    }

    pub fn status(&self) -> RecordStatus {
        let seconds = self.clock.seconds();
        RecordStatus {
            path: self.path.clone(),
            seconds,
            frames: self.frames,
            dropped: self.dropped,
            speed: (self.mode == CaptureMode::Offline)
                .then(|| seconds / self.started.elapsed().as_secs_f64().max(1e-3)),
        }
    }

    /// Waits for the frames still on the GPU, encodes them, closes the file, and returns
    /// the final status or the error that stopped the encoder.
    pub fn finish(mut self, device: &wgpu::Device) -> Result<RecordStatus> {
        while self.readback.is_busy() {
            if let Err(err) = self.deliver(device, true) {
                return Err(self.encoder.finish().err().unwrap_or(err));
            }
        }
        let status = self.status();
        let written = self.encoder.finish()?;
        log::info!("recorded {written} frames to {}", status.path.display());
        Ok(status)
    }

    /// Sends finished readbacks to the encoder. Waits for the GPU when `wait` is set;
    /// then sends block too, so no frame is dropped.
    fn deliver(&mut self, device: &wgpu::Device, wait: bool) -> Result<()> {
        while let Some(buffer) = self.encoder.recycled_buffer() {
            self.spare.push(buffer);
        }
        let spare = &mut self.spare;
        let frames = self
            .readback
            .collect(device, wait, || spare.pop().unwrap_or_default());
        let blocking = wait || self.mode == CaptureMode::Offline;
        for frame in frames {
            self.send(frame, blocking)?;
        }
        Ok(())
    }

    fn send(&mut self, frame: Frame, blocking: bool) -> Result<()> {
        if blocking {
            self.encoder.send(frame)?;
        } else if let Some(frame) = self.encoder.try_send(frame)? {
            self.dropped += 1;
            self.spare.push(frame.rgba);
            return Ok(());
        }
        self.frames += 1;
        Ok(())
    }
}
```

- [ ] **Step 6: Run everything.** Run: `cargo test`. Expected: `108 passed` for the unit tests, `3 passed` in `tests/capture.rs`, `5 passed` in `tests/smoke.rs`. `records_every_frame_offline` records exactly 10 frames (`stop_after = 10/60` s) with none dropped. `real_time_capture_counts_frames_it_has_to_drop` fills the 3 staging buffers and counts the 2 frames that didn't fit.

- [ ] **Step 7: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add src/capture/mod.rs src/capture/readback.rs src/capture/recorder.rs src/passes/mod.rs tests/smoke.rs
git commit -m "feat: read captured frames back from the GPU and record them" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Capture panel and app wiring

**Files:**
- Modify: `Cargo.toml` (adds `rfd`), `Cargo.lock` (generated), `.gitignore`, `src/ui.rs`, `src/app.rs`
- Test: full suite; manual checklist with real recordings

**Interfaces:**
- Consumes: `Recorder`, `RecordSettings`, `RecordStatus` (Task 6); `CaptureMode`, `OFFLINE_STEP` (Task 5); `VideoFormat` (Task 4); `CanvasChoice` (Task 3).
- Produces:
  - `ui::CaptureUi { settings: RecordSettings, status: Option<RecordStatus>, saved: Option<String>, error: Option<String> }`. Its defaults are HEVC, real-time, folder `captures` and stop after 0.
  - `UiState.capture`, plus `UiActions.{start_recording, stop_recording, choose_folder}`.
  - In the app:
    - `State.recorder: Option<Recorder>`.
    - Animation now advances after the surface texture is acquired, by `OFFLINE_STEP` while recording offline (design decisions 3 and 4).
    - Canvas Apply is disabled while recording.
    - Closing the window finishes the recording.

- [ ] **Step 1: Add `rfd` and ignore the default capture folder.** Replace `Cargo.toml` with:

```toml
[package]
name = "rasterwarp"
version = "0.1.0"
edition = "2024"
rust-version = "1.88"

[dependencies]
anyhow = "1"
bytemuck = { version = "1", features = ["derive"] }
chrono = { version = "0.4", default-features = false, features = ["clock"] }
egui = "0.36"
egui-wgpu = "0.36"
egui-winit = "0.36"
env_logger = "0.11"
ffmpeg-next = { version = "9.0.0", default-features = false, features = ["codec", "format", "software-scaling"] }
image = { version = "0.25", default-features = false, features = ["png", "jpeg"] }
log = "0.4"
pollster = "1"
rfd = "0.17"
wgpu = "30"
winit = "0.30"
```

Append this line to `.gitignore`:

```
/captures/
```

- [ ] **Step 2: Add the Capture section.** Replace `src/ui.rs` with:

```rust
//! The egui parameter panel.

use std::path::PathBuf;

use egui::{Button, CollapsingHeader, ComboBox, ProgressBar, Slider, Ui};

use crate::canvas::{CanvasChoice, PRESETS};
use crate::capture::CaptureMode;
use crate::capture::encode::VideoFormat;
use crate::capture::recorder::{RecordSettings, RecordStatus};
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
    /// Show the off-air preview in Transition and Sequence modes.
    pub show_preview: bool,
    /// Smoothed frame time in milliseconds.
    pub frame_ms: f32,
    pub source_info: String,
    pub load_error: Option<String>,
    /// The user curve open in the curve editor.
    pub editing_curve: Option<u32>,
    pub canvas: CanvasChoice,
    pub capture: CaptureUi,
}

/// The capture controls and what the current or last recording reported.
#[derive(Clone, Debug)]
pub struct CaptureUi {
    pub settings: RecordSettings,
    /// Progress while recording.
    pub status: Option<RecordStatus>,
    /// What the last recording saved.
    pub saved: Option<String>,
    pub error: Option<String>,
}

impl Default for CaptureUi {
    fn default() -> Self {
        Self {
            settings: RecordSettings {
                format: VideoFormat::Hevc,
                mode: CaptureMode::RealTime,
                folder: PathBuf::from("captures"),
                stop_after: 0.0,
            },
            status: None,
            saved: None,
            error: None,
        }
    }
}

/// The off-air preview as the panel shows it.
pub struct PreviewOverlay {
    pub texture: egui::TextureId,
    /// Texture size in pixels.
    pub size: (u32, u32),
    pub label: String,
}

/// Display width of the preview overlay, in points.
const PREVIEW_WIDTH: f32 = 320.0;

/// One-shot actions requested by the user this frame.
#[derive(Default)]
pub struct UiActions {
    pub clear_feedback: bool,
    /// Switch the canvas to this size.
    pub apply_canvas: Option<(u32, u32)>,
    pub start_recording: bool,
    pub stop_recording: bool,
    pub choose_folder: bool,
}

pub fn draw(
    ui: &mut Ui,
    motion: &mut Motion,
    state: &mut UiState,
    preview: Option<&PreviewOverlay>,
) -> UiActions {
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
                    ui.checkbox(&mut state.show_preview, "Show preview");
                    if ui.button("Reset all").clicked() {
                        *motion.editable() = Params::default();
                        actions.clear_feedback = true;
                    }
                });
                let recording = state.capture.status.is_some();
                capture_section(ui, &mut state.capture, state.frame_ms, &mut actions);
                canvas_section(ui, &mut state.canvas, recording, &mut actions);
                mode_section(ui, motion);
                curves_section(ui, &mut motion.curves, state);
                let params = motion.editable();
                warp_section(ui, params);
                colorize_section(ui, params);
                feedback_section(ui, params, &mut actions);
                glow_section(ui, params);
            });
        });
    if let Some(preview) = preview {
        preview_overlay(ui.ctx(), preview);
    }
    actions
}

/// The preview, framed and labelled, in the bottom-right corner of the window.
fn preview_overlay(ctx: &egui::Context, preview: &PreviewOverlay) {
    let height = PREVIEW_WIDTH * preview.size.1 as f32 / preview.size.0 as f32;
    egui::Area::new(egui::Id::new("preview"))
        .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(&preview.label);
                ui.image((preview.texture, egui::vec2(PREVIEW_WIDTH, height)));
            });
        });
}

fn capture_section(ui: &mut Ui, capture: &mut CaptureUi, frame_ms: f32, actions: &mut UiActions) {
    CollapsingHeader::new("Capture")
        .default_open(true)
        .show(ui, |ui| {
            let settings = &mut capture.settings;
            ui.add_enabled_ui(capture.status.is_none(), |ui| {
                ComboBox::from_id_salt("capture format")
                    .selected_text(settings.format.label())
                    .show_ui(ui, |ui| {
                        for format in VideoFormat::ALL {
                            ui.selectable_value(&mut settings.format, format, format.label());
                        }
                    });
                ComboBox::from_id_salt("capture mode")
                    .selected_text(settings.mode.label())
                    .show_ui(ui, |ui| {
                        for mode in CaptureMode::ALL {
                            ui.selectable_value(&mut settings.mode, mode, mode.label());
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("Stop after");
                    ui.add(
                        egui::DragValue::new(&mut settings.stop_after)
                            .range(0.0..=3600.0)
                            .speed(0.1)
                            .suffix(" s"),
                    );
                    ui.small("(0 = stop by hand)");
                });
                ui.horizontal(|ui| {
                    ui.label(format!("Folder: {}", settings.folder.display()));
                    if ui.button("Choose…").clicked() {
                        actions.choose_folder = true;
                    }
                });
            });
            match &capture.status {
                None => {
                    if ui.button("Record").clicked() {
                        actions.start_recording = true;
                    }
                }
                Some(status) => {
                    if ui.button("Stop recording").clicked() {
                        actions.stop_recording = true;
                    }
                    ui.label(format!(
                        "{:.1} s · {} frames · {} dropped",
                        status.seconds, status.frames, status.dropped
                    ));
                    if let Some(speed) = status.speed {
                        ui.label(format!("offline: {speed:.2}× realtime"));
                    } else if frame_ms > 1000.0 / 60.0 * 1.05 {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            "Rendering is below 60 fps, so frames will be missing.                              Offline mode records every frame.",
                        );
                    }
                }
            }
            if let Some(saved) = &capture.saved {
                ui.small(saved);
            }
            if let Some(err) = &capture.error {
                ui.colored_label(egui::Color32::LIGHT_RED, err);
            }
        });
}

fn canvas_section(
    ui: &mut Ui,
    canvas: &mut CanvasChoice,
    recording: bool,
    actions: &mut UiActions,
) {
    CollapsingHeader::new("Canvas").show(ui, |ui| {
        let name = |i: usize| match PRESETS.get(i) {
            Some((name, (w, h))) => format!("{name} ({w}×{h})"),
            None => "Custom".to_owned(),
        };
        ComboBox::from_id_salt("canvas preset")
            .selected_text(name(canvas.choice))
            .show_ui(ui, |ui| {
                for i in 0..=CanvasChoice::CUSTOM {
                    ui.selectable_value(&mut canvas.choice, i, name(i));
                }
            });
        if canvas.choice == CanvasChoice::CUSTOM {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut canvas.custom.0).range(1..=canvas.max_side));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut canvas.custom.1).range(1..=canvas.max_side));
            });
        }
        let (w, h) = canvas.requested();
        let (cw, ch) = canvas.current;
        ui.label(format!("Rendering at {cw}×{ch}"));
        ui.horizontal(|ui| {
            let changed = (w, h) != canvas.current;
            if ui
                .add_enabled(changed && !recording, Button::new(format!("Apply {w}×{h}")))
                .clicked()
            {
                actions.apply_canvas = Some((w, h));
            }
            ui.small(if recording {
                "stop recording to change"
            } else {
                "clears the trails"
            });
        });
    });
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
        let can_delete = seq.cues().len() > 1;
        if ui
            .add_enabled(can_delete, Button::new("Delete cue"))
            .clicked()
        {
            seq.delete_cue();
        }
    });
    let i = seq.selected;
    let mut start = seq.cues()[i].start_frame;
    let mut duration = seq.cues()[i].duration_frames;
    // Cue 1 always starts at frame 0 and is never ramped into, so its controls stay disabled.
    ui.add_enabled_ui(i > 0, |ui| {
        ui.horizontal(|ui| {
            if ui
                .add(Slider::new(&mut start, 0..=MAX_FRAME).text("start frame"))
                .changed()
            {
                seq.set_start_frame(i, start);
            }
            ui.label(format!("{:.2} s", start as f32 / FRAMES_PER_SECOND));
        });
        ui.horizontal(|ui| {
            if ui
                .add(Slider::new(&mut duration, 1..=MAX_FRAME).text("ramp frames"))
                .changed()
            {
                seq.set_duration(i, duration);
            }
            ui.label(format!("{:.2} s", duration as f32 / FRAMES_PER_SECOND));
        });
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

- [ ] **Step 3: Wire recording into the app.** Replace `src/app.rs` with:

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

use crate::canvas::{self, CanvasChoice};
use crate::capture::recorder::Recorder;
use crate::capture::{CaptureMode, OFFLINE_STEP};
use crate::gpu;
use crate::motion::Motion;
use crate::params::Params;
use crate::passes::Renderer;
use crate::preview::PreviewView;
use crate::source::{self, GrayImage};
use crate::ui::{self, UiActions, UiState};

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
            WindowEvent::CloseRequested => {
                state.stop_recording(); // finish the file before exiting
                event_loop.exit();
            }
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
    preview: PreviewView,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    motion: Motion,
    recorder: Option<Recorder>,
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
            show_preview: true,
            canvas: CanvasChoice::new(canvas::DEFAULT, max_texture_side),
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
        let renderer = Renderer::new(&device, &queue, composite_format, canvas::DEFAULT, &image);

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui_ctx.viewport_id(),
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(max_texture_side as usize),
        );
        let mut egui_renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        let preview =
            PreviewView::new(&device, &queue, &mut egui_renderer, canvas::DEFAULT, &image);

        Ok(Self {
            window,
            surface,
            config,
            device,
            queue,
            max_texture_side,
            composite_format,
            renderer,
            preview,
            egui_ctx,
            egui_state,
            egui_renderer,
            motion: Motion::new(Params::default()),
            recorder: None,
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
                self.preview.set_source(&self.device, &self.queue, &image);
                self.ui.source_info = describe(&path.display().to_string(), &image);
                self.ui.load_error = None;
            }
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.load_error = Some(format!("{err:#}"));
            }
        }
    }

    /// Advances animation time: by the frame time, or by exactly 1/60 s per frame while
    /// recording offline.
    fn advance_animation(&mut self, dt: f32) {
        if self.ui.paused {
            return;
        }
        let offline = self
            .recorder
            .as_ref()
            .is_some_and(|r| r.mode() == CaptureMode::Offline);
        // Clamp so a stall (e.g. dragging the window) doesn't make animation jump.
        let step = if offline { OFFLINE_STEP } else { dt.min(0.1) };
        self.time += f64::from(step);
        self.motion.advance(step);
    }

    fn start_recording(&mut self) {
        self.ui.capture.saved = None;
        match Recorder::start(
            &self.device,
            &self.ui.capture.settings,
            self.renderer.size(),
            self.time,
        ) {
            Ok(recorder) => {
                self.ui.capture.error = None;
                self.ui.capture.status = Some(recorder.status());
                self.recorder = Some(recorder);
            }
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.capture.error = Some(format!("{err:#}"));
            }
        }
    }

    /// Finishes the current recording, if any, and reports how it went.
    fn stop_recording(&mut self) {
        let Some(recorder) = self.recorder.take() else {
            return;
        };
        self.ui.capture.status = None;
        match recorder.finish(&self.device) {
            Ok(status) => {
                self.ui.capture.saved = Some(format!(
                    "Saved {} ({:.1} s, {} frames, {} dropped)",
                    status.path.display(),
                    status.seconds,
                    status.frames,
                    status.dropped
                ));
            }
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.capture.error = Some(format!("Recording stopped: {err:#}"));
            }
        }
    }

    fn choose_capture_folder(&mut self) {
        let folder = &mut self.ui.capture.settings.folder;
        let mut dialog = rfd::FileDialog::new().set_title("Capture folder");
        if let Ok(start) = std::path::absolute(&*folder)
            && start.is_dir()
        {
            dialog = dialog.set_directory(start);
        }
        if let Some(picked) = dialog.pick_folder() {
            *folder = picked;
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.ui.frame_ms += (dt * 1000.0 - self.ui.frame_ms) * 0.05;

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
        // Animation moves only for frames that are drawn, so offline recordings stay exact.
        self.advance_animation(dt);
        if let Some(recorder) = &mut self.recorder
            && let Err(err) = recorder.pump(&self.device)
        {
            log::warn!("{err:#}");
            self.stop_recording();
        }
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let srgb_view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.composite_format),
            ..Default::default()
        });
        let output_size = (self.config.width, self.config.height);

        let overlay = self
            .ui
            .show_preview
            .then(|| self.motion.preview())
            .flatten()
            .map(|p| ui::PreviewOverlay {
                texture: self.preview.texture_id(),
                size: self.preview.size(),
                label: p.source.label(),
            });
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let mut actions = UiActions::default();
        let egui_output = self.egui_ctx.run_ui(raw_input, |ui| {
            actions = ui::draw(ui, &mut self.motion, &mut self.ui, overlay.as_ref())
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

        if actions.choose_folder {
            self.choose_capture_folder();
        }
        if actions.stop_recording {
            self.stop_recording();
        }
        if let Some(size) = actions.apply_canvas
            && self.recorder.is_none()
        {
            self.renderer.resize(&self.device, size);
            self.preview
                .resize(&self.device, &mut self.egui_renderer, size);
            self.ui.canvas.current = size;
        }
        if actions.start_recording && self.recorder.is_none() {
            self.start_recording();
        }
        let frame_params = self.motion.frame();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        if actions.clear_feedback {
            self.renderer.clear_feedback(&mut encoder);
            self.preview.clear_feedback(&mut encoder);
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
        match self
            .ui
            .show_preview
            .then(|| self.motion.preview())
            .flatten()
        {
            Some(preview) => self.preview.render(
                &self.device,
                &self.queue,
                &mut encoder,
                &preview,
                self.time as f32,
            ),
            None => self.preview.hide(),
        }
        let mut capture_failed = false;
        if let Some(recorder) = &mut self.recorder {
            let (device, queue, renderer, time) =
                (&self.device, &self.queue, &self.renderer, self.time);
            let captured = recorder.capture(device, &mut encoder, time, |encoder, view| {
                renderer.composite_capture(device, queue, encoder, &frame_params, time as f32, view)
            });
            if let Err(err) = captured {
                log::warn!("{err:#}");
                capture_failed = true;
            }
        }
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
        if let Some(recorder) = &mut self.recorder {
            recorder.after_submit();
            self.ui.capture.status = Some(recorder.status());
            if capture_failed || recorder.finished_recording() {
                self.stop_recording();
            }
        }
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

- [ ] **Step 4: Run everything.** Run: `cargo test`. Expected: `108 passed` for the unit tests, `3 passed` in `tests/capture.rs`, `5 passed` in `tests/smoke.rs`.

- [ ] **Step 5: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings.

- [ ] **Step 6: Manual check (release).** Run: `cargo run --release`, then check each item. ffprobe is in `C:\Users\abart\Desktop\ffmpeg-9.0.2-full_build-shared\bin`.
  - [ ] **Record** with HEVC in real time for about 5 s, including a transition (Space), then **Stop recording**.
    - While recording, the panel shows seconds, frames and "0 dropped".
    - Afterwards it shows "Saved captures\rasterwarp-YYYYMMDD-HHMMSS.mp4 (…)".
    - `ffprobe -v error -count_packets -show_entries stream=codec_name,profile,pix_fmt,r_frame_rate,nb_read_packets,color_space -of default=nw=1 <file>` reports hevc, Rext, yuv444p, 60/1 and bt709, with a packet count equal to the panel's frame count.
    - The file plays in a media player, and the preview overlay and panel are not in it.
  - [ ] Choose **FFV1**, **Offline (frame-accurate)** and **Stop after 2.0 s**, then press **Record**.
    - The panel shows "offline: N× realtime" (about 0.5× at 1080p with the default noise).
    - Recording stops by itself, and the saved line reads "2.0 s, 120 frames, 0 dropped".
    - ffprobe reports ffv1, bgra and 120 packets.
  - [ ] While recording, the format, mode, stop-after and folder controls are greyed out, and Canvas **Apply** says "stop recording to change".
  - [ ] **Pause** during an offline recording: the frame count stops growing. Unpause and it resumes, with no jump in the video.
  - [ ] **Choose…** opens a folder dialog, and the next recording goes into the chosen folder.
  - [ ] HEVC on a machine without NVENC (or with all sessions in use): **Record** shows an error suggesting FFV1, and no file is left behind.
  - [ ] Closing the window mid-recording leaves a playable file.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock .gitignore src/ui.rs src/app.rs
git commit -m "feat: add the capture panel and record from the app" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Done criteria (Plan 2)

- The spec's Plan 2 behaviours all work as in the checklists of Tasks 2, 3 and 7: the preview, canvas resolution, real-time and offline capture in both formats, and stop-after.
- `cargo test` passes (108 unit, 3 capture and 5 smoke tests). `cargo clippy --all-targets` is clean and `cargo fmt` makes no changes.
- Recordings contain only the clean canvas output, at a constant 60 fps.
