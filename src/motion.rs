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
