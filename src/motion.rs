//! Ties the modes together: which parameters the panel edits, how time moves
//! ramps and phase clocks forward, and what the renderer draws.

use crate::blend::{Clocks, FrameParams, advance_ramp, blend};
use crate::curve::CurveLibrary;
use crate::params::Params;
use crate::params::table::SliderId;
use crate::rate::TICKS_PER_SECOND;
use crate::save::saved_names;
use crate::sequence::{SeqEvent, SeqView, Sequence};
use crate::transition::{AbEvent, AbState};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    /// The panel edits what's on screen.
    #[default]
    Live,
    /// A/B banks: the panel edits the off-air bank; Transition ramps to it.
    Transition,
    /// Up to 5 cues played on a frame clock.
    Sequence,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Live, Mode::Transition, Mode::Sequence];
}

saved_names!(Mode {
    Live => "live",
    Transition => "transition",
    Sequence => "sequence",
});

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
    /// Animation frames so far (calls to `advance`); seeds the line jitter.
    frames: u32,
    /// The output jumped (a cut or a snapped cue) since the last `take_jump`.
    jumped: bool,
    /// What audio adds to each linked slider this frame, in table order. Applied to
    /// copies of the banks it blends; the banks themselves never change.
    offsets: Vec<(SliderId, f32)>,
}

/// `p` with each offset added through the table's setter (held in range, rounded, tied
/// values kept).
fn shown(offsets: &[(SliderId, f32)], p: &Params) -> Params {
    let mut p = *p;
    for &(id, offset) in offsets {
        let value = id.get(&p) + offset;
        id.set(&mut p, value);
    }
    p
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
            frames: 0,
            jumped: false,
            offsets: Vec::new(),
        }
    }

    /// Motion restored from a project, at rest: no ramp running, the sequence stopped,
    /// and phases starting over.
    pub fn restored(
        mode: Mode,
        ab: AbState,
        sequence: Option<Sequence>,
        curves: CurveLibrary,
    ) -> Self {
        Self {
            mode,
            ab,
            sequence,
            curves,
            // The picture jumps to the project's look.
            jumped: true,
            ..Self::new(Params::default())
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
        self.jumped = true;
    }

    /// Whether the output jumped instead of moving (a Transition cut, a sequence ramp cut
    /// short by the next cue, a loop back to cue 1) since the last call. Ramps are real
    /// motion and don't count.
    pub fn take_jump(&mut self) -> bool {
        std::mem::take(&mut self.jumped)
    }

    /// Sets what audio adds to each linked slider from the next frame on.
    pub fn set_offsets(&mut self, offsets: Vec<(SliderId, f32)>) {
        self.offsets = offsets;
    }

    /// What audio adds to each linked slider now.
    pub fn offsets(&self) -> &[(SliderId, f32)] {
        &self.offsets
    }

    /// Advances ramps and phase clocks by `ticks` of animation time (see
    /// [`TICKS_PER_SECOND`]); the app passes one canvas frame period.
    pub fn advance(&mut self, ticks: i64) {
        let dt = (ticks as f64 / TICKS_PER_SECOND as f64) as f32;
        self.frames = self.frames.wrapping_add(1);
        match self.mode {
            Mode::Live => {
                let on_air = shown(&self.offsets, &self.ab.banks[self.ab.on_air]);
                self.rest.advance(&on_air, dt)
            }
            Mode::Transition => {
                let on_air = shown(&self.offsets, &self.ab.banks[self.ab.on_air]);
                match self.ab.ramp() {
                    Some(ramp) => {
                        let off_air = shown(&self.offsets, &self.ab.banks[self.ab.off_air()]);
                        advance_ramp(
                            &mut self.rest,
                            &mut self.target,
                            &on_air,
                            &off_air,
                            self.curves.eval(self.ab.curve, ramp.progress),
                            dt,
                        )
                    }
                    None => self.rest.advance(&on_air, dt),
                }
                if self.ab.advance(dt) == Some(AbEvent::Finished) {
                    self.rest = self.target;
                }
            }
            Mode::Sequence => {
                let seq = self
                    .sequence
                    .get_or_insert_with(|| Sequence::new(self.ab.banks[self.ab.on_air]));
                let events = seq.advance(ticks);
                // A ramp snapped to its end by the next cue, or a loop back to cue 1.
                let snapped = events
                    .windows(2)
                    .any(|w| w == [SeqEvent::RampFinished, SeqEvent::RampStarted]);
                if snapped || events.contains(&SeqEvent::Restarted) {
                    self.jumped = true;
                }
                for event in events {
                    match event {
                        SeqEvent::RampStarted => self.target = self.rest,
                        SeqEvent::RampFinished => self.rest = self.target,
                        SeqEvent::Restarted => {}
                    }
                }
                match seq.view() {
                    SeqView::Rest(p) => self.rest.advance(&shown(&self.offsets, p), dt),
                    SeqView::Ramp {
                        from,
                        to,
                        progress,
                        curve,
                    } => advance_ramp(
                        &mut self.rest,
                        &mut self.target,
                        &shown(&self.offsets, from),
                        &shown(&self.offsets, to),
                        self.curves.eval(curve, progress),
                        dt,
                    ),
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
            self.preview.advance(&shown(&self.offsets, &p), dt);
        }
    }

    /// What the off-air preview shows, if anything: the off-air bank in Transition mode,
    /// and the selected cue while a sequence runs.
    pub fn preview(&self) -> Option<Preview> {
        let (source, p) = self.preview_params()?;
        let p = shown(&self.offsets, p);
        let mut frame = blend(&p, &p, 0.0, None, &self.preview, &self.preview);
        frame.warp.seed = self.frames;
        Some(Preview { source, frame })
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
        let mut frame = self.blended();
        frame.warp.seed = self.frames;
        frame
    }

    fn blended(&self) -> FrameParams {
        let offsets = &self.offsets;
        let rest = |p: &Params| {
            let p = shown(offsets, p);
            blend(&p, &p, 0.0, None, &self.rest, &self.rest)
        };
        let ramp = |from: &Params, to: &Params, eased: f32, progress: f32| {
            blend(
                &shown(offsets, from),
                &shown(offsets, to),
                eased,
                Some(progress),
                &self.rest,
                &self.target,
            )
        };
        match self.mode {
            Mode::Live => rest(&self.ab.banks[self.ab.on_air]),
            Mode::Transition => match self.ab.ramp() {
                Some(r) => ramp(
                    &self.ab.banks[self.ab.on_air],
                    &self.ab.banks[self.ab.off_air()],
                    self.curves.eval(self.ab.curve, r.progress),
                    r.progress,
                ),
                None => rest(&self.ab.banks[self.ab.on_air]),
            },
            Mode::Sequence => match self.sequence.as_ref().map(Sequence::view) {
                Some(SeqView::Ramp {
                    from,
                    to,
                    progress,
                    curve,
                }) => ramp(from, to, self.curves.eval(curve, progress), progress),
                Some(SeqView::Rest(p)) => rest(p),
                None => rest(&self.ab.banks[self.ab.on_air]),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::CurveRef;

    /// Ticks in `seconds` of animation time.
    fn secs(seconds: f64) -> i64 {
        (seconds * TICKS_PER_SECOND as f64).round() as i64
    }

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
        m.advance(secs(1.0));
        assert_eq!(m.preview().unwrap().source, PreviewSource::Bank(1));
        m.advance(secs(1.5)); // the ramp finishes and B goes on air
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
        m.advance(secs(0.7));
        m.set_mode(Mode::Transition);
        m.advance(secs(0.0)); // the preview appears
        let phase = |f: &FrameParams| f.warp.oscillators[0].phase;
        assert_eq!(phase(&m.preview().unwrap().frame), phase(&m.frame()));
        m.editable().warp.oscillators[0].phase_speed = 0.0;
        m.advance(secs(0.5));
        let still = phase(&m.preview().unwrap().frame);
        let moved = phase(&m.frame());
        // The output moved 0.4 × 0.5 = 0.2 cycles while the preview held still.
        let off = moved - still - 0.2;
        assert!((off - off.round()).abs() < 1e-4, "{moved} vs {still}");
    }

    #[test]
    fn jitter_seed_counts_animation_frames() {
        let mut m = Motion::new(Params::default());
        assert_eq!(m.frame().warp.seed, 0);
        m.advance(secs(1.0 / 60.0));
        m.advance(secs(1.0 / 60.0));
        assert_eq!(m.frame().warp.seed, 2);
        // A paused frame doesn't advance animation, so the seed (and jitter) holds.
        assert_eq!(m.frame().warp.seed, 2);
        m.set_mode(Mode::Transition);
        assert_eq!(m.preview().unwrap().frame.warp.seed, 2);
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
        m.advance(secs(1.0)); // halfway through the default 2 s S-curve
        assert!((m.frame().warp.zoom - 2.0).abs() < 1e-5);
        m.advance(secs(1.0));
        assert_eq!(m.frame().warp.zoom, 3.0);
        assert_eq!(m.editable().warp.zoom, 1.0, "now editing the other bank");
    }

    #[test]
    fn a_cut_jumps_but_a_ramp_does_not() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Transition);
        m.editable().warp.zoom = 3.0;
        m.trigger();
        m.advance(secs(1.0));
        m.advance(secs(1.5)); // the ramp finishes
        assert!(!m.take_jump(), "a ramp is real motion");
        m.cut();
        assert!(m.take_jump());
        assert!(!m.take_jump(), "taking clears it");
    }

    #[test]
    fn a_snapped_cue_and_a_loop_jump() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Sequence);
        let seq = m.sequence_mut();
        seq.add_cue(); // cue 2 at frame 48, ramp 48 frames
        seq.add_cue(); // cue 3 at frame 96
        seq.set_duration(1, 96); // cue 2's ramp is cut short by cue 3
        seq.set_duration(2, 24);
        seq.looping = true;
        seq.run();
        let frame = secs(1.0 / 24.0);
        let mut jumps = Vec::new();
        for f in 1..=120 {
            m.advance(frame);
            if m.take_jump() {
                jumps.push(f);
            }
        }
        // Cue 3 starts at frame 96 and snaps cue 2's ramp; it ends at 120 and loops.
        assert_eq!(jumps, [96, 120]);
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
        m.advance(secs(0.3));
        m.trigger();
        m.advance(secs(1.99));
        let before = m.frame().warp.oscillators[0].phase;
        m.advance(secs(0.02)); // finishes; the destination's clocks take over
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
        m.advance(secs(0.3));
        m.trigger();
        m.advance(secs(1.0)); // halfway through the ramp
        let before = osc0_phase(&m);
        m.cut();
        assert_eq!(m.ab.on_air, 1);
        // The sweep's one running clock stays on screen, so the cut doesn't jump...
        let at_cut = osc0_phase(&m);
        assert!(
            phase_step(before, at_cut).abs() < 1e-5,
            "phase jumped at the cut"
        );
        let (mut last, mut turned) = (at_cut, 0.0);
        for _ in 0..100 {
            m.advance(secs(0.01));
            let now = osc0_phase(&m);
            let step = phase_step(last, now);
            assert!(step.abs() < 0.1, "phase jumped after cut");
            turned += step;
            last = now;
        }
        // ...and then runs at the destination's 2 cycles/s.
        assert!((turned - 2.0).abs() < 1e-3, "turned {turned} cycles in 1 s");
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
            m.advance(secs(0.01));
            let now = osc0_phase(&m);
            assert!(phase_step(last, now).abs() < 0.1, "phase jumped");
            last = now;
        }
        assert!(
            !matches!(m.sequence.as_ref().unwrap().view(), SeqView::Ramp { .. }),
            "ramp finished"
        );
    }

    /// Runs `frames` canvas frames at 60 fps. Returns how far oscillator 1 turned
    /// (cycles, unwrapped) and its fastest speed (cycles per second).
    fn track_osc0(m: &mut Motion, frames: u32) -> (f32, f32) {
        let period = crate::rate::FrameRate::whole(60).period_ticks();
        let mut last = osc0_phase(m);
        let (mut turned, mut fastest) = (0.0, 0.0_f32);
        for _ in 0..frames {
            m.advance(period);
            let now = osc0_phase(m);
            let step = phase_step(last, now);
            turned += step;
            fastest = fastest.max(step.abs() * 60.0);
            last = now;
        }
        (turned, fastest)
    }

    #[test]
    fn transition_glides_phase_speed_from_a_to_b() {
        let mut m = Motion::new(Params::default());
        m.editable().warp.oscillators[0].phase_speed = 0.5;
        m.set_mode(Mode::Transition);
        m.editable().warp.oscillators[0].phase_speed = 2.0;
        m.ab.curve = CurveRef::Linear;
        m.trigger();
        // Over the 2 s ramp the speed rises evenly from 0.5 to 2.0 cycles/s, so the
        // wave turns about 2.5 cycles and never runs faster than B.
        let (turned, fastest) = track_osc0(&mut m, 121);
        assert!(m.ab.ramp().is_none(), "the ramp finished");
        assert!((turned - 2.5).abs() < 0.05, "turned {turned} cycles");
        assert!(fastest <= 2.0 + 1e-3, "peaked at {fastest} cycles/s");
    }

    #[test]
    fn sequence_ramp_glides_phase_speed_from_cue_to_cue() {
        let mut m = Motion::new(Params::default());
        m.editable().warp.oscillators[0].phase_speed = 0.5;
        m.set_mode(Mode::Sequence);
        let seq = m.sequence_mut();
        seq.add_cue();
        let cue = seq.selected_cue_mut();
        cue.params.warp.oscillators[0].phase_speed = 2.0;
        cue.curve = CurveRef::Linear;
        seq.set_start_frame(1, 24);
        seq.set_duration(1, 48);
        seq.run();
        track_osc0(&mut m, 60); // 1 s resting on cue 1; the ramp starts at frame 24
        let (turned, fastest) = track_osc0(&mut m, 120); // the 2 s ramp
        assert!((turned - 2.5).abs() < 0.05, "turned {turned} cycles");
        assert!(fastest <= 2.0 + 1e-3, "peaked at {fastest} cycles/s");
    }

    #[test]
    fn reversed_transition_reverts_without_changing_banks_or_phase() {
        let mut m = Motion::new(Params::default());
        m.set_mode(Mode::Transition);
        m.editable().warp.oscillators[0].phase_speed = 2.0;
        m.advance(secs(0.3));
        m.trigger();
        m.advance(secs(0.5));
        m.trigger(); // reverse
        assert!(!m.ab.ramp().unwrap().forward);
        let mut last = osc0_phase(&m);
        let mut steps = 0;
        while m.ab.ramp().is_some() {
            m.advance(secs(0.01));
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
        m.advance(secs(1.0)); // ramp to cue 2 starts at frame 24
        m.advance(secs(1.0)); // and ends at frame 48
        assert_eq!(m.frame().warp.zoom, 2.0);
    }

    #[test]
    fn offsets_move_the_frame_but_not_the_bank() {
        use crate::params::table::SliderId;
        let mut motion = Motion::new(Params::default());
        let base = motion.editable().warp.zoom;
        motion.set_offsets(vec![(SliderId::Zoom, 0.5)]);
        assert!((motion.frame().warp.zoom - (base + 0.5)).abs() < 1e-6);
        assert_eq!(
            motion.editable().warp.zoom,
            base,
            "the bank keeps the hand's value"
        );
        motion.set_offsets(Vec::new());
        assert!((motion.frame().warp.zoom - base).abs() < 1e-6);
    }

    #[test]
    fn offsets_are_held_in_range_and_rounded() {
        use crate::params::table::SliderId;
        let mut motion = Motion::new(Params::default());
        motion.set_offsets(vec![(SliderId::Zoom, 1.0e6)]);
        assert_eq!(motion.frame().warp.zoom, *SliderId::Zoom.range().end());
        motion.set_offsets(vec![(SliderId::Levels, 1.4)]);
        let levels = motion.editable().colorize.levels;
        let mut shown = *motion.editable();
        SliderId::Levels.set(&mut shown, levels as f32 + 1.4);
        assert_eq!(
            shown.colorize.levels,
            levels + 1,
            "rounded like a hand edit"
        );
    }

    #[test]
    fn a_modulated_speed_really_runs_faster() {
        use crate::params::table::{OscSlider, SliderId};
        let speed = SliderId::Osc(0, OscSlider::PhaseSpeed);
        let amplitude = SliderId::Osc(0, OscSlider::Amplitude);
        let mut still = Motion::new(Params::default());
        let mut pushed = Motion::new(Params::default());
        for motion in [&mut still, &mut pushed] {
            speed.set(motion.editable(), 0.0);
            amplitude.set(motion.editable(), 0.5); // a silent oscillator may be left out
        }
        pushed.set_offsets(vec![(speed, 1.0)]);
        for _ in 0..10 {
            still.advance(TICKS_PER_SECOND / 10);
            pushed.advance(TICKS_PER_SECOND / 10);
        }
        let phase = |m: &Motion| {
            m.frame()
                .warp
                .oscillators
                .iter()
                .find(|o| o.index == 0)
                .expect("oscillator 1 is drawn")
                .phase
        };
        assert_ne!(
            phase(&still),
            phase(&pushed),
            "the phase clock advanced with the modulated speed"
        );
    }

    #[test]
    fn the_preview_is_modulated_too() {
        use crate::params::table::SliderId;
        let mut motion = Motion::new(Params::default());
        motion.set_mode(Mode::Transition);
        let base = motion.editable().warp.zoom; // the off-air bank in Transition
        motion.set_offsets(vec![(SliderId::Zoom, 0.25)]);
        let preview = motion.preview().expect("Transition shows the off-air bank");
        assert!((preview.frame.warp.zoom - (base + 0.25)).abs() < 1e-6);
    }
}
