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
