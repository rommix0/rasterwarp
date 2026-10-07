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
