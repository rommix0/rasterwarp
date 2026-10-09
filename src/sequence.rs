//! Sequence mode: up to 5 cues, each ramping in at its own start frame
//! (the Animation Aid's sequence ramps with frame-count thumbwheels).

use serde::{Deserialize, Serialize};

use crate::curve::CurveRef;
use crate::params::Params;
use crate::rate::TICKS_PER_SECOND;

pub const MAX_CUES: usize = 5;
/// The manual's thumbwheels count film frames.
pub const FRAMES_PER_SECOND: f32 = 24.0;
/// Highest start frame (3-digit thumbwheel) and longest ramp.
pub const MAX_FRAME: u32 = 999;
/// Animation-time ticks per sequence frame: exact, so cues start on the right canvas
/// frame at every program frame rate.
const TICKS_PER_FRAME: i64 = TICKS_PER_SECOND / FRAMES_PER_SECOND as i64;

/// Ticks from Run to sequence frame `frame`.
fn frame_ticks(frame: u32) -> i64 {
    i64::from(frame) * TICKS_PER_FRAME
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cue {
    pub params: Params,
    pub start_frame: u32,
    pub duration_frames: u32,
    pub curve: CurveRef,
}

impl Default for Cue {
    fn default() -> Self {
        Self {
            params: Params::default(),
            start_frame: 0,
            duration_frames: 48,
            curve: CurveRef::SCurve,
        }
    }
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
    /// Ticks of animation time since Run (see [`TICKS_PER_SECOND`]).
    clock: i64,
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
                ..Cue::default()
            }],
            selected: 0,
            looping: false,
            running: false,
            clock: 0,
            base: 0,
            target: None,
        }
    }

    /// A stopped sequence of `cues` (from a file), repaired to the rules the panel keeps:
    /// 1 to [`MAX_CUES`] cues, cue 1 at frame 0, start frames rising and at most
    /// [`MAX_FRAME`], ramps 1 to [`MAX_FRAME`] frames, parameters in range.
    pub fn from_cues(cues: &[Cue], selected: usize, looping: bool) -> Self {
        let mut kept: Vec<Cue> = Vec::new();
        for cue in cues.iter().take(MAX_CUES) {
            let start = match kept.last() {
                None => 0,
                Some(prev) if prev.start_frame >= MAX_FRAME => break,
                Some(prev) => cue.start_frame.clamp(prev.start_frame + 1, MAX_FRAME),
            };
            kept.push(Cue {
                params: cue.params.clamped(),
                start_frame: start,
                duration_frames: cue.duration_frames.clamp(1, MAX_FRAME),
                curve: cue.curve,
            });
        }
        if kept.is_empty() {
            kept.push(Cue::default());
        }
        Self {
            selected: selected.min(kept.len() - 1),
            looping,
            cues: kept,
            ..Self::new(Params::default())
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

    /// Sequence frames since Run.
    pub fn clock_frames(&self) -> f32 {
        (self.clock as f64 / TICKS_PER_FRAME as f64) as f32
    }

    /// Appends a copy of the selected cue two seconds after the last one.
    /// Returns false when full or out of thumbwheel range.
    pub fn add_cue(&mut self) -> bool {
        self.push_cue(self.cues[self.selected])
    }

    /// Stops on a cue holding `params`, so entering Sequence mode keeps the frame on
    /// screen: selects a cue that already holds them, or appends one (as [`add_cue`]
    /// would). Returns false, leaving the cues and selection alone, when it can't add one.
    ///
    /// [`add_cue`]: Self::add_cue
    pub fn hold(&mut self, params: Params) -> bool {
        if let Some(i) = self.cues.iter().position(|cue| cue.params == params) {
            self.selected = i;
            self.stop();
            return true;
        }
        self.push_cue(Cue {
            params,
            ..self.cues[self.selected]
        })
    }

    /// Appends `cue` two seconds after the last one and selects it.
    fn push_cue(&mut self, mut cue: Cue) -> bool {
        let start = self.cues[self.cues.len() - 1].start_frame + 48;
        if self.cues.len() >= MAX_CUES || start > MAX_FRAME {
            return false;
        }
        cue.start_frame = start;
        self.cues.push(cue);
        self.selected = self.cues.len() - 1;
        self.stop();
        true
    }

    /// Deletes the selected cue; at least one cue always remains. Deleting the
    /// first cue promotes the next one, which then starts at frame 0.
    pub fn delete_cue(&mut self) -> bool {
        if self.cues.len() <= 1 {
            return false;
        }
        self.cues.remove(self.selected);
        if self.selected == 0 {
            self.cues[0].start_frame = 0;
        } else {
            self.selected -= 1;
        }
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
        self.clock = 0;
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
        let elapsed = self.clock - frame_ticks(cue.start_frame);
        (elapsed as f64 / frame_ticks(cue.duration_frames) as f64) as f32
    }

    /// Advances by `ticks` of animation time (see [`TICKS_PER_SECOND`]).
    pub fn advance(&mut self, ticks: i64) -> Vec<SeqEvent> {
        let mut events = Vec::new();
        if !self.running {
            return events;
        }
        self.clock += ticks;
        loop {
            let next = self.target.map_or(self.base + 1, |t| t + 1);
            if next >= self.cues.len() || self.clock < frame_ticks(self.cues[next].start_frame) {
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
        let end = frame_ticks(last.start_frame + last.duration_frames);
        if self.looping
            && self.target.is_none()
            && self.base == self.cues.len() - 1
            && self.clock >= end
        {
            // Carry the overshoot so loops don't drift by up to a frame per cycle.
            let overshoot = self.clock - end;
            self.reset();
            self.clock = overshoot;
            events.push(SeqEvent::Restarted);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ticks in `seconds` of animation time.
    fn secs(seconds: f64) -> i64 {
        (seconds * TICKS_PER_SECOND as f64).round() as i64
    }

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
    fn cues_start_on_the_exact_canvas_frame_at_every_rate() {
        use crate::rate::FrameRate;
        for rate in FrameRate::ALL {
            for start in [1, 24, 25, 48, 100, 999] {
                let mut s = Sequence::new(Params::default());
                s.add_cue();
                s.set_start_frame(1, start);
                s.run();
                // The first canvas frame at or after the cue's start time.
                let period = rate.period_ticks();
                let due = (frame_ticks(start) + period - 1) / period;
                let mut frame = 0;
                loop {
                    frame += 1;
                    if s.advance(period).contains(&SeqEvent::RampStarted) {
                        break;
                    }
                    assert!(frame < due, "{} cue at {start}: late", rate.label());
                }
                assert_eq!(frame, due, "{} cue at {start}: early", rate.label());
            }
        }
    }

    #[test]
    fn changing_the_frame_rate_mid_run_keeps_time() {
        use crate::rate::FrameRate;
        let mut s = three_cues();
        s.run();
        // Half a second at 24 fps, then just short of another half second at 30 fps.
        for _ in 0..12 {
            assert!(s.advance(FrameRate::whole(24).period_ticks()).is_empty());
        }
        for _ in 0..14 {
            assert!(s.advance(FrameRate::whole(30).period_ticks()).is_empty());
        }
        // Frame 24 (exactly 1 s) is where cue 2 starts.
        let events = s.advance(FrameRate::whole(30).period_ticks());
        assert_eq!(events, vec![SeqEvent::RampStarted]);
        assert_eq!(s.clock_frames(), 24.0);
    }

    #[test]
    fn stopped_sequence_shows_selected_cue() {
        let mut s = three_cues();
        s.selected = 2;
        assert_eq!(zoom_from(s.view()), (3.0, None));
        assert!(s.advance(secs(1.0)).is_empty());
    }

    #[test]
    fn cues_ramp_in_at_their_start_frames() {
        let mut s = three_cues();
        s.run();
        assert_eq!(s.advance(secs(0.5)), vec![]); // frame 12: resting on cue 1
        assert_eq!(zoom_from(s.view()), (1.0, None));
        assert_eq!(s.advance(secs(0.5)), vec![SeqEvent::RampStarted]); // frame 24
        assert_eq!(s.advance(secs(0.5)), vec![]); // frame 36: halfway to cue 2
        assert_eq!(zoom_from(s.view()), (1.0, Some(0.5)));
        assert_eq!(s.advance(secs(0.5)), vec![SeqEvent::RampFinished]); // frame 48
        assert_eq!(zoom_from(s.view()), (2.0, None));
    }

    #[test]
    fn overlapping_cue_snaps_previous_ramp() {
        let mut s = three_cues();
        s.cues[1].duration_frames = 100; // still ramping when cue 3 starts at 72
        s.run();
        s.advance(secs(1.0)); // frame 24: ramp to cue 2 starts
        let events = s.advance(secs(2.0)); // frame 72
        assert_eq!(events, vec![SeqEvent::RampFinished, SeqEvent::RampStarted]);
        assert_eq!(zoom_from(s.view()), (2.0, Some(0.0)));
    }

    #[test]
    fn reset_returns_to_cue_one() {
        let mut s = three_cues();
        s.run();
        s.advance(secs(3.0));
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
            events.extend(s.advance(secs(1.0))); // frames 24..120; last cue ends at 96
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
    fn loop_carries_the_overshoot_into_the_next_cycle() {
        let mut s = three_cues();
        s.looping = true;
        s.run();
        // Last cue ends at frame 96; 4.125 s is frame 99, three frames past it.
        let events = s.advance(frame_ticks(99));
        assert!(events.contains(&SeqEvent::Restarted));
        assert_eq!(s.clock_frames(), 3.0);
        assert_eq!(zoom_from(s.view()), (1.0, None));
    }

    #[test]
    fn cue_count_is_capped_and_the_last_cue_is_permanent() {
        let mut s = Sequence::new(Params::default());
        assert!(!s.delete_cue(), "a single cue always remains");
        for _ in 0..4 {
            assert!(s.add_cue());
        }
        assert!(!s.add_cue());
        assert_eq!(s.cues().len(), MAX_CUES);
        s.selected = 3;
        assert!(s.delete_cue());
        assert_eq!(s.selected, 2);
        while s.cues().len() > 1 {
            assert!(s.delete_cue());
        }
        assert!(!s.delete_cue());
        assert_eq!(s.cues().len(), 1);
    }

    #[test]
    fn deleting_the_first_cue_promotes_the_next_to_frame_zero() {
        let mut s = three_cues();
        s.selected = 0;
        assert!(s.delete_cue());
        assert_eq!(s.selected, 0);
        let frames: Vec<u32> = s.cues().iter().map(|c| c.start_frame).collect();
        let zooms: Vec<f32> = s.cues().iter().map(|c| c.params.warp.zoom).collect();
        assert_eq!(frames, vec![0, 72]);
        assert_eq!(zooms, vec![2.0, 3.0]);
    }

    /// Default parameters with `zoom`.
    fn zoomed(zoom: f32) -> Params {
        let mut p = Params::default();
        p.warp.zoom = zoom;
        p
    }

    #[test]
    fn holding_new_params_appends_a_cue_that_shows_them() {
        let mut s = three_cues();
        s.selected = 0;
        s.run();
        assert!(s.hold(zoomed(1.5)));
        assert!(!s.is_running());
        assert_eq!(s.selected, 3);
        assert_eq!(s.cues()[3].start_frame, 120);
        assert_eq!(zoom_from(s.view()), (1.5, None));
        let zooms: Vec<f32> = s.cues().iter().map(|c| c.params.warp.zoom).collect();
        assert_eq!(
            zooms,
            vec![1.0, 2.0, 3.0, 1.5],
            "the other cues are untouched"
        );
    }

    #[test]
    fn holding_params_a_cue_has_selects_it() {
        let mut s = three_cues();
        assert!(s.hold(zoomed(2.0)));
        assert_eq!(s.selected, 1);
        assert_eq!(s.cues().len(), 3);
    }

    #[test]
    fn holding_with_no_room_leaves_the_cues() {
        let mut s = three_cues();
        s.add_cue();
        s.add_cue();
        s.selected = 1;
        assert!(!s.hold(zoomed(1.5)));
        assert_eq!(s.cues().len(), MAX_CUES);
        assert_eq!(s.selected, 1);
    }
}
