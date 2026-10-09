//! Sound that moves the picture: follow signals (Level, Bass, Mid, Treble, Pulse) that
//! sliders follow, and beats that fire options and actions the way a MIDI key does.

pub mod analysis;

use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

/// The rate everything is analysed and played at, in samples per second.
pub const RATE: u32 = 48_000;

/// A follow signal: a value from 0 to 1 that sliders can follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Signal {
    /// The whole signal's loudness.
    Level,
    /// Below 200 Hz.
    Bass,
    /// 200 Hz to 2 kHz.
    Mid,
    /// Above 2 kHz.
    Treble,
    /// Jumps to 1 on every beat and falls back over the release time.
    Pulse,
}

impl Signal {
    /// Every signal, in the order the meter shows them.
    pub const ALL: [Signal; 5] = [
        Signal::Level,
        Signal::Bass,
        Signal::Mid,
        Signal::Treble,
        Signal::Pulse,
    ];

    /// Its permanent name, as links are saved.
    pub fn name(self) -> &'static str {
        match self {
            Signal::Level => "level",
            Signal::Bass => "bass",
            Signal::Mid => "mid",
            Signal::Treble => "treble",
            Signal::Pulse => "pulse",
        }
    }

    /// Its name in the panel.
    pub fn label(self) -> &'static str {
        match self {
            Signal::Level => "Level",
            Signal::Bass => "Bass",
            Signal::Mid => "Mid",
            Signal::Treble => "Treble",
            Signal::Pulse => "Pulse",
        }
    }

    /// The signal saved as `name`, if this version knows it.
    pub fn from_name(name: &str) -> Option<Signal> {
        Signal::ALL.into_iter().find(|s| s.name() == name)
    }

    /// Its place in [`Signal::ALL`] and [`Signals::values`].
    pub fn index(self) -> usize {
        self as usize
    }
}

/// A beat: fires like a key press.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Beat {
    /// An onset in the whole signal, or any hand beat.
    Any,
    /// An onset in the bass band (a kick).
    Bass,
    /// An onset in the treble band (hats, snare).
    Treble,
}

impl Beat {
    /// Every beat, in the order the meter shows them.
    pub const ALL: [Beat; 3] = [Beat::Any, Beat::Bass, Beat::Treble];

    /// Its permanent name, as links are saved.
    pub fn name(self) -> &'static str {
        match self {
            Beat::Any => "any",
            Beat::Bass => "bass",
            Beat::Treble => "treble",
        }
    }

    /// Its name in the panel.
    pub fn label(self) -> &'static str {
        match self {
            Beat::Any => "Any",
            Beat::Bass => "Bass",
            Beat::Treble => "Treble",
        }
    }

    /// The beat saved as `name`, if this version knows it.
    pub fn from_name(name: &str) -> Option<Beat> {
        Beat::ALL.into_iter().find(|b| b.name() == name)
    }

    /// Its place in [`Beat::ALL`] and [`Signals::beats`].
    pub fn index(self) -> usize {
        self as usize
    }
}

/// What the sound is doing on one canvas frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Signals {
    /// Each follow signal, 0 to 1, indexed by [`Signal::index`].
    pub values: [f32; 5],
    /// Whether each beat fired this frame, indexed by [`Beat::index`].
    pub beats: [bool; 3],
}

impl Signals {
    /// One follow signal's value.
    pub fn get(&self, signal: Signal) -> f32 {
        self.values[signal.index()]
    }

    /// Whether `beat` fired this frame.
    pub fn beat(&self, beat: Beat) -> bool {
        self.beats[beat.index()]
    }
}

/// The gain setting's range.
pub const GAIN: RangeInclusive<f32> = 0.0..=8.0;
/// The attack time's range, in seconds.
pub const ATTACK: RangeInclusive<f32> = 0.001..=0.2;
/// The release time's range, in seconds.
pub const RELEASE: RangeInclusive<f32> = 0.01..=2.0;
/// The beat sensitivity's range.
pub const SENSITIVITY: RangeInclusive<f32> = 0.0..=1.0;

/// How the signals are shaped: one setting for all of them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shaping {
    /// Multiplies every follow signal before it is held to 1.
    pub gain: f32,
    /// How fast a follow signal rises, in seconds.
    pub attack: f32,
    /// How fast a follow signal (and Pulse) falls, in seconds.
    pub release: f32,
    /// How easily beats fire: higher fires on smaller jumps.
    pub sensitivity: f32,
}

impl Default for Shaping {
    fn default() -> Self {
        Self {
            gain: 1.0,
            attack: 0.01,
            release: 0.15,
            sensitivity: 0.5,
        }
    }
}

impl Shaping {
    /// Every setting held inside its range (a file could hold anything).
    pub fn clamped(self) -> Shaping {
        let hold = |v: f32, range: RangeInclusive<f32>, default: f32| {
            if v.is_finite() {
                v.clamp(*range.start(), *range.end())
            } else {
                default
            }
        };
        let d = Shaping::default();
        Shaping {
            gain: hold(self.gain, GAIN, d.gain),
            attack: hold(self.attack, ATTACK, d.attack),
            release: hold(self.release, RELEASE, d.release),
            sensitivity: hold(self.sensitivity, SENSITIVITY, d.sensitivity),
        }
    }
}

/// The Pulse signal: jumps to 1 on a beat and falls back over the release time. It is
/// stepped once per canvas frame, so it is the same for every source.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pulse {
    value: f32,
}

impl Pulse {
    /// Moves on by `dt` seconds; a `beat` this frame sets it to 1. Returns the new value.
    pub fn step(&mut self, beat: bool, dt: f32, release: f32) -> f32 {
        self.value = if beat {
            1.0
        } else {
            self.value * (-dt / release.max(f32::EPSILON)).exp()
        };
        self.value
    }

    /// Its value now.
    pub fn value(&self) -> f32 {
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_and_beat_names_never_change() {
        let signals: Vec<&str> = Signal::ALL.iter().map(|s| s.name()).collect();
        assert_eq!(signals, ["level", "bass", "mid", "treble", "pulse"]);
        let beats: Vec<&str> = Beat::ALL.iter().map(|b| b.name()).collect();
        assert_eq!(beats, ["any", "bass", "treble"]);
        for s in Signal::ALL {
            assert_eq!(Signal::from_name(s.name()), Some(s));
        }
        for b in Beat::ALL {
            assert_eq!(Beat::from_name(b.name()), Some(b));
        }
        assert_eq!(Signal::from_name("sub-bass"), None);
        assert_eq!(Beat::from_name("snare"), None);
    }

    #[test]
    fn shaping_from_a_file_is_held_in_range() {
        let wild = Shaping {
            gain: 100.0,
            attack: 0.0,
            release: f32::NAN,
            sensitivity: -1.0,
        };
        let held = wild.clamped();
        assert_eq!(held.gain, *GAIN.end());
        assert_eq!(held.attack, *ATTACK.start());
        assert_eq!(held.release, Shaping::default().release);
        assert_eq!(held.sensitivity, 0.0);
    }

    #[test]
    fn pulse_jumps_on_a_beat_and_falls_over_the_release() {
        let mut pulse = Pulse::default();
        assert_eq!(pulse.step(false, 1.0 / 60.0, 0.15), 0.0);
        assert_eq!(pulse.step(true, 1.0 / 60.0, 0.15), 1.0);
        let dt = 0.001;
        for _ in 0..150 {
            pulse.step(false, dt, 0.15);
        }
        assert!(
            (pulse.value() - (-1.0f32).exp()).abs() < 0.01,
            "{}",
            pulse.value()
        );
    }
}
