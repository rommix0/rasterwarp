# Audio Links Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let sound move Rasterwarp's sliders (Level, Bass, Mid, Treble, Pulse pushing a slider from its hand-set value) and let beats (detected, or tapped by hand) fire options, modes and actions; play a sound file in sync with the clock and record it as the video's soundtrack.

**Architecture:** `src/audio/` holds the signal types and engine (`mod.rs`), the analysis shared by live input and files (`analysis.rs`), sound-file decoding into samples plus precomputed tracks (`file.rs`), live input and loopback through `cpal` (`input.rs`), and speaker playback following the playhead (`output.rs`). `src/audio_links.rs` turns signals into per-slider offsets and fired targets. `Motion` applies the offsets to copies of the banks it blends, so the frame drawn and recorded is modulated while the banks stay as the hand left them. The recorder carries the file's samples as a second stream.

**Tech Stack:** Rust 2024, egui 0.36, `cpal` 0.18 (WASAPI), `ffmpeg-next` 9 (decode, swresample, AAC/PCM encode), serde/serde_json.

**Spec:** `docs/superpowers/specs/2026-10-09-rasterwarp-audio-links-design.md`

## Global Constraints

- Rust edition 2024, `rust-version = "1.88"`. New dependency: `cpal = "0.18"` only; enable ffmpeg-next's `software-resampling` feature (Task 4). `swresample-7.dll` is already copied by `build.rs`.
- Analysis runs at 48 kHz mono. Bands: Bass below 200 Hz (low-pass), Mid 200 Hz–2 kHz (band-pass), Treble above 2 kHz (high-pass), Level unfiltered. A full-scale in-band sine reads 1 at gain 1; values clamp at 1.
- Shaping defaults and ranges: gain 1 (0–8), attack 10 ms (1–200 ms), release 150 ms (10 ms–2 s), sensitivity 0.5 (0–1, higher fires more easily). Beat hold-off 100 ms per band. Slow average about 1 s.
- Saved names, pinned by tests: signals `level`, `bass`, `mid`, `treble`, `pulse`; beats `any`, `bass`, `treble`; actions `beat`, `bass-beat`, `treble-beat` (appended after the existing ten).
- Follow links target table sliders only (never the transition duration, never choices). Beat links target what a MIDI note fires: choice options, modes, actions.
- Shown value = base + Σ depth × signal, held in the slider's range, whole sliders rounded, written through `SliderId::set`. Modulation never changes the banks, cues or the panel's values, and never marks the project unsaved.
- File tracks: 1,000 values per second. Speaker sync tolerance 40 ms. Decoded files: 48 kHz stereo 16-bit, charged to the shared `Budget`.
- Soundtrack codecs: HEVC `.mp4` → AAC; ProRes `.mov` and FFV1 `.mkv` → PCM signed 16-bit. Soundtrack only when the source is a sound file; never live input or hand beats.
- Code style: match the surrounding code — doc comments in plain sentences on every public item, comment density as in `src/inputs.rs`, no `unwrap` outside tests except where an invariant is stated.
- Rust string line continuations: a `\` at the very end of the line, then the next line indented. Shell heredocs eat the backslash — edit `.rs` files with the Edit/Write tools only. Keep files LF.
- Commands: `cargo test --lib`, `cargo test --test smoke`, `cargo test --test capture`, `cargo test --test video`, `cargo clippy --all-targets`, `cargo fmt`. All must pass/clean before each commit; GPU suites only need running when the task touches code they exercise (otherwise `cargo test --no-run`).
- Tests must never open a MIDI port or an audio stream; listing devices is fine.
- Don't re-run a full verification that already passed on the same tree.

## Rulings made while planning

- **Modulation goes through `Motion`, not `FrameParams`.** `Motion::frame()` returns a derived `FrameParams` that `SliderId::set` can't edit, so `Motion` gains `set_offsets(Vec<(SliderId, f32)>)` and applies the offsets to a copy of every `Params` it blends or advances (`frame`, `preview`, and `advance`, so a modulated phase speed or video speed really runs faster). Offsets are applied in `SliderId::all()` order, which puts `Levels` before the `LevelFrom` thresholds.
- **Audio links live on `Links`** (the struct the panel already threads through every linkable control) as a new field `pub audio: AudioLinks`. `Links.list` stays the MIDI links (saved in the app settings); `Links.audio` is saved in the project. This keeps every `link_ui` signature unchanged.
- **The beat actions are added in Task 8,** where `App::run_action` (an exhaustive match) is edited, so no task leaves the build broken.
- **The recorder takes sound through a separate channel** keyed by sample position, so a real-time recording that drops a video frame still writes continuous sound. Frame `n` covers samples `round(n·48000·den/num)` to `round((n+1)·48000·den/num)`, so the soundtrack's length always matches the video's.
- **`Encoder::start` and `Recorder::start` keep their signatures** (tests call them); new `start_with_sound` variants take the extra flag.
- **Changing attack, release or sensitivity on a loaded file re-runs its analysis** on a background thread (a few hundred milliseconds for a song); gain is applied when reading, so it needs no re-run.

---

### Task 1: Signals and analysis

**Files:**
- Create: `src/audio/mod.rs` (signal and beat types, `Signals`, `Shaping`, `Pulse`)
- Create: `src/audio/analysis.rs` (filters, followers, onsets)
- Modify: `src/lib.rs` (add `pub mod audio;` after `pub mod app;`)

**Interfaces:**
- Produces (`crate::audio`):
  - `pub const RATE: u32 = 48_000;`
  - `pub enum Signal { Level, Bass, Mid, Treble, Pulse }` with `ALL: [Signal; 5]`, `name(self) -> &'static str`, `label(self) -> &'static str`, `from_name(&str) -> Option<Signal>`, `index(self) -> usize`.
  - `pub enum Beat { Any, Bass, Treble }` with `ALL: [Beat; 3]`, `name`, `label`, `from_name`, `index`.
  - `pub struct Signals { pub values: [f32; 5], pub beats: [bool; 3] }` (`Default`, `Copy`) with `get(&self, Signal) -> f32`, `beat(&self, Beat) -> bool`.
  - `pub struct Shaping { pub gain: f32, pub attack: f32, pub release: f32, pub sensitivity: f32 }` (seconds; `Default`, serde, `clamped(self) -> Shaping`), and range consts `GAIN`, `ATTACK`, `RELEASE`, `SENSITIVITY: RangeInclusive<f32>`.
  - `pub struct Pulse` with `step(&mut self, beat: bool, dt: f32, release: f32) -> f32` and `value(&self) -> f32`.
- Produces (`crate::audio::analysis`):
  - `pub struct Analyzer` with `new(rate: u32, shaping: Shaping) -> Analyzer`, `set_shaping(&mut self, Shaping)`, `push(&mut self, x: f32) -> [bool; 3]` (beats this sample, indexed by `Beat::index`), `values(&self) -> [f32; 4]` (Level, Bass, Mid, Treble before gain).
  - `pub fn gained(values: [f32; 4], gain: f32) -> [f32; 4]` (multiply and clamp to 0..=1).

- [ ] **Step 1: Write `src/audio/mod.rs`**

```rust
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
```

- [ ] **Step 2: Add the module**

In `src/lib.rs`, add `pub mod audio;` directly after `pub mod app;`.

- [ ] **Step 3: Write the failing analysis tests**

Create `src/audio/analysis.rs` with only the test module first (and an empty `use`-free body above it), so the tests fail to compile:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{Beat, RATE, Shaping};
    use std::f32::consts::TAU;

    /// A sine at `freq` Hz with amplitude `amp`, for `seconds`.
    fn sine(freq: f32, amp: f32, seconds: f32) -> Vec<f32> {
        let n = (seconds * RATE as f32) as usize;
        (0..n)
            .map(|i| amp * (TAU * freq * i as f32 / RATE as f32).sin())
            .collect()
    }

    fn silence(seconds: f32) -> Vec<f32> {
        vec![0.0; (seconds * RATE as f32) as usize]
    }

    /// Feeds `samples`; returns the values after the last one and the beats counted.
    fn run(analyzer: &mut Analyzer, samples: &[f32]) -> ([f32; 4], [usize; 3]) {
        let mut beats = [0; 3];
        for &x in samples {
            for (count, fired) in beats.iter_mut().zip(analyzer.push(x)) {
                *count += usize::from(fired);
            }
        }
        (analyzer.values(), beats)
    }

    /// `count` bursts of a `freq` Hz sine, 20 ms long, `gap` seconds apart.
    fn clicks(freq: f32, count: usize, gap: f32) -> Vec<f32> {
        let mut out = Vec::new();
        for _ in 0..count {
            out.extend(sine(freq, 1.0, 0.02));
            out.extend(silence(gap - 0.02));
        }
        out
    }

    const LEVEL: usize = 0;
    const BASS: usize = 1;
    const MID: usize = 2;
    const TREBLE: usize = 3;

    #[test]
    fn the_bands_split_the_spectrum() {
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (low, _) = run(&mut a, &sine(60.0, 1.0, 1.0));
        assert!(low[BASS] > 0.8, "{low:?}");
        assert!(low[TREBLE] < 0.05, "{low:?}");
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (high, _) = run(&mut a, &sine(8000.0, 1.0, 1.0));
        assert!(high[TREBLE] > 0.8, "{high:?}");
        assert!(high[BASS] < 0.05, "{high:?}");
    }

    #[test]
    fn a_full_scale_sine_in_its_band_reads_about_one() {
        for (freq, band) in [(60.0, BASS), (632.0, MID), (8000.0, TREBLE), (440.0, LEVEL)] {
            let mut a = Analyzer::new(RATE, Shaping::default());
            let (values, _) = run(&mut a, &sine(freq, 1.0, 1.0));
            assert!(
                (0.9..=1.1).contains(&values[band]),
                "{freq} Hz band {band}: {values:?}"
            );
        }
    }

    #[test]
    fn gain_multiplies_and_holds_at_one() {
        assert_eq!(gained([0.25, 0.5, 0.0, 1.0], 2.0), [0.5, 1.0, 0.0, 1.0]);
        assert_eq!(gained([0.5; 4], 0.0), [0.0; 4]);
    }

    #[test]
    fn attack_and_release_follow_their_times() {
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (rising, _) = run(&mut a, &sine(1000.0, 1.0, 0.05));
        assert!(rising[LEVEL] > 0.8, "fast attack: {rising:?}");
        run(&mut a, &sine(1000.0, 1.0, 0.5));
        let (released, _) = run(&mut a, &silence(0.15));
        assert!(
            (0.2..=0.55).contains(&released[LEVEL]),
            "one release time: {released:?}"
        );
        let (gone, _) = run(&mut a, &silence(0.3));
        assert!(gone[LEVEL] < 0.1, "three release times: {gone:?}");

        let slow = Shaping {
            attack: 0.2,
            ..Shaping::default()
        };
        let mut a = Analyzer::new(RATE, slow);
        let (rising, _) = run(&mut a, &sine(1000.0, 1.0, 0.05));
        assert!(rising[LEVEL] < 0.4, "slow attack: {rising:?}");
    }

    #[test]
    fn clicks_beat_once_each() {
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (_, beats) = run(&mut a, &clicks(1000.0, 4, 0.5));
        assert_eq!(beats[Beat::Any.index()], 4);
    }

    #[test]
    fn a_band_beats_only_on_its_own_sound() {
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (_, beats) = run(&mut a, &clicks(8000.0, 4, 0.5));
        assert_eq!(beats[Beat::Treble.index()], 4);
        assert_eq!(beats[Beat::Bass.index()], 0);
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (_, beats) = run(&mut a, &clicks(60.0, 4, 0.5));
        assert_eq!(beats[Beat::Bass.index()], 4);
        assert_eq!(beats[Beat::Treble.index()], 0);
    }

    #[test]
    fn clicks_closer_than_the_hold_off_beat_once() {
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (_, beats) = run(&mut a, &clicks(1000.0, 2, 0.05));
        assert_eq!(beats[Beat::Any.index()], 1);
    }

    #[test]
    fn a_steady_tone_beats_only_when_it_starts() {
        let mut a = Analyzer::new(RATE, Shaping::default());
        let (_, beats) = run(&mut a, &sine(1000.0, 1.0, 3.0));
        assert_eq!(beats[Beat::Any.index()], 1);
    }
}
```

- [ ] **Step 4: Run them to see them fail**

Run: `cargo test --lib audio::analysis`
Expected: compile errors (`Analyzer`, `gained` not found).

- [ ] **Step 5: Write the analysis above the test module**

```rust
//! The analysis shared by live input and sound files: band filters, envelope followers
//! with attack and release, and beat (onset) detection, one sample at a time.

use std::f32::consts::TAU;

use super::Shaping;

/// The top of the bass band, in hertz.
pub const BASS_TOP: f32 = 200.0;
/// The bottom of the treble band, in hertz.
pub const TREBLE_BOTTOM: f32 = 2000.0;
/// How long the detector averages the squared signal, in seconds: long enough to smooth
/// a 50 Hz bass note, short enough for drums.
const DETECTOR: f32 = 0.02;
/// How long the slow average that beats are compared against takes, in seconds.
const SLOW: f32 = 1.0;
/// A band fires at most this often, in seconds.
pub const HOLD_OFF: f32 = 0.1;
/// Below this a band never fires (quiet hiss).
const FLOOR: f32 = 0.02;
/// Butterworth Q for the low-pass and high-pass filters.
const BUTTERWORTH: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// The per-sample smoothing coefficient for a one-pole filter with time constant
/// `seconds` at `rate` samples per second.
fn coefficient(seconds: f32, rate: f32) -> f32 {
    1.0 - (-1.0 / (seconds * rate).max(f32::EPSILON)).exp()
}

/// How far above its slow average a band must jump to fire, for a sensitivity of 0
/// (3×) to 1 (1.2×).
fn jump_ratio(sensitivity: f32) -> f32 {
    3.0 - 1.8 * sensitivity.clamp(0.0, 1.0)
}

/// A second-order IIR filter (the RBJ cookbook's biquad).
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
}

impl Biquad {
    fn new(b: [f32; 3], a0: f32, a: [f32; 2]) -> Self {
        Self {
            b: b.map(|v| v / a0),
            a: a.map(|v| v / a0),
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    /// Lets everything through (the Level band).
    fn pass() -> Self {
        Self::new([1.0, 0.0, 0.0], 1.0, [0.0, 0.0])
    }

    fn low_pass(rate: f32, freq: f32) -> Self {
        let (cos, alpha) = Self::angle(rate, freq, BUTTERWORTH);
        Self::new(
            [(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0],
            1.0 + alpha,
            [-2.0 * cos, 1.0 - alpha],
        )
    }

    fn high_pass(rate: f32, freq: f32) -> Self {
        let (cos, alpha) = Self::angle(rate, freq, BUTTERWORTH);
        Self::new(
            [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0],
            1.0 + alpha,
            [-2.0 * cos, 1.0 - alpha],
        )
    }

    /// A band-pass from `low` to `high` hertz with 0 dB at its centre.
    fn band_pass(rate: f32, low: f32, high: f32) -> Self {
        let centre = (low * high).sqrt();
        let q = centre / (high - low);
        let (cos, alpha) = Self::angle(rate, centre, q);
        Self::new([alpha, 0.0, -alpha], 1.0 + alpha, [-2.0 * cos, 1.0 - alpha])
    }

    fn angle(rate: f32, freq: f32, q: f32) -> (f32, f32) {
        let w = TAU * freq / rate;
        (w.cos(), w.sin() / (2.0 * q))
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

/// One band: its filter, its follower and its beat detector.
#[derive(Clone, Copy, Debug)]
struct Band {
    filter: Biquad,
    /// The detector's mean square.
    power: f32,
    /// The follower's output: the band's value before gain.
    value: f32,
    /// The slow average of `value` that jumps are measured against.
    slow: f32,
    /// Whether the band has fallen back below its threshold since it last fired.
    armed: bool,
    /// Seconds since the band last fired.
    since: f32,
}

impl Band {
    fn new(filter: Biquad) -> Self {
        Self {
            filter,
            power: 0.0,
            value: 0.0,
            slow: 0.0,
            armed: true,
            since: HOLD_OFF,
        }
    }
}

/// The bands, in the order [`Analyzer::values`] returns them.
const LEVEL: usize = 0;
const BASS: usize = 1;
const TREBLE: usize = 3;

/// Turns samples into Level, Bass, Mid and Treble values and beats.
#[derive(Clone, Debug)]
pub struct Analyzer {
    bands: [Band; 4],
    /// Seconds per sample.
    dt: f32,
    rate: f32,
    detector: f32,
    slow: f32,
    attack: f32,
    release: f32,
    ratio: f32,
}

impl Analyzer {
    /// An analyzer for mono samples at `rate` per second.
    pub fn new(rate: u32, shaping: Shaping) -> Self {
        let r = rate as f32;
        let mut analyzer = Self {
            bands: [
                Band::new(Biquad::pass()),
                Band::new(Biquad::low_pass(r, BASS_TOP)),
                Band::new(Biquad::band_pass(r, BASS_TOP, TREBLE_BOTTOM)),
                Band::new(Biquad::high_pass(r, TREBLE_BOTTOM)),
            ],
            dt: 1.0 / r,
            rate: r,
            detector: coefficient(DETECTOR, r),
            slow: coefficient(SLOW, r),
            attack: 0.0,
            release: 0.0,
            ratio: 0.0,
        };
        analyzer.set_shaping(shaping);
        analyzer
    }

    /// Uses new attack, release and sensitivity from the next sample on.
    pub fn set_shaping(&mut self, shaping: Shaping) {
        self.attack = coefficient(shaping.attack, self.rate);
        self.release = coefficient(shaping.release, self.rate);
        self.ratio = jump_ratio(shaping.sensitivity);
    }

    /// Takes one sample. Returns which beats it starts, indexed by `Beat::index`
    /// (Any, Bass, Treble).
    pub fn push(&mut self, x: f32) -> [bool; 3] {
        let mut fired = [false; 4];
        for (band, fired) in self.bands.iter_mut().zip(&mut fired) {
            let y = band.filter.run(x);
            band.power += (y * y - band.power) * self.detector;
            let level = (2.0 * band.power).sqrt();
            let k = if level > band.value {
                self.attack
            } else {
                self.release
            };
            band.value += (level - band.value) * k;
            band.slow += (band.value - band.slow) * self.slow;
            band.since += self.dt;
            let threshold = band.slow * self.ratio;
            if band.value > threshold.max(FLOOR) {
                if band.armed && band.since >= HOLD_OFF {
                    *fired = true;
                    band.since = 0.0;
                }
                band.armed = false;
            } else if band.value <= threshold {
                band.armed = true;
            }
        }
        [fired[LEVEL], fired[BASS], fired[TREBLE]]
    }

    /// Level, Bass, Mid and Treble now, before gain.
    pub fn values(&self) -> [f32; 4] {
        self.bands.map(|b| b.value)
    }
}

/// `values` times `gain`, each held to 0..=1.
pub fn gained(values: [f32; 4], gain: f32) -> [f32; 4] {
    values.map(|v| (v * gain).clamp(0.0, 1.0))
}
```

- [ ] **Step 6: Run the tests**

Run: `cargo test --lib audio::`
Expected: all pass (11 tests). If a band's tolerance fails by a small margin, check the filter formulas against the RBJ cookbook before touching the test's numbers; the numbers come from the analysis in the plan's ruling (detector 20 ms, peak follower on `sqrt(2·mean square)`).

- [ ] **Step 7: Check and commit**

Run: `cargo fmt`, `cargo clippy --all-targets`, `cargo test --lib`, `cargo test --no-run`.

```bash
git add src/audio src/lib.rs
git commit -m "feat: audio analysis: band followers and beat detection"
```

---

### Task 2: Audio links

**Files:**
- Create: `src/audio_links.rs`
- Modify: `src/lib.rs` (add `pub mod audio_links;` after `pub mod audio;`)
- Modify: `src/control.rs` (`pub fn fire`, used by `Links::handle`; `Links` gains `pub audio: AudioLinks`)

**Interfaces:**
- Consumes (Task 1): `Signal`, `Beat`, `Signals`. Existing: `Target` (`saved`, `from_saved`, `continuous`, `label`), `SliderId` (`all`, `range`, `id`, `from_id`, `label`), `Links`, `Motion::editable`, `Motion::set_mode`.
- Produces (`crate::audio_links`):
  - `pub struct AudioLink { pub signal: Signal, pub slider: SliderId, pub depth: f32 }` with `new(signal, slider) -> AudioLink` (depth = a quarter of the slider's range).
  - `pub struct BeatLink { pub beat: Beat, pub target: Target }`.
  - `pub struct AudioLinks { pub follow: Vec<AudioLink>, pub beats: Vec<BeatLink> }` (`Clone, Debug, Default, PartialEq`) with `follow_signal(&mut self, Signal, SliderId)`, `fire_on(&mut self, Beat, Target)` (ignores continuous targets), `unfollow(&mut self, Signal, SliderId)`, `unfire(&mut self, Beat, Target)`, `signals_for(&self, SliderId) -> Vec<Signal>`, `beats_for(&self, Target) -> Vec<Beat>`, `offsets(&self, &Signals) -> Vec<(SliderId, f32)>` (in `SliderId::all()` order, summed per slider), `fired(&self, &Signals) -> Vec<Target>`.
  - `pub mod saved_follow` and `pub mod saved_beats` for `#[serde(with = "...")]` (tolerant reading, like `control::saved_links`).
- Produces (`crate::control`): `pub fn fire(target: Target, motion: &mut Motion) -> Option<Action>`; `Links { pub list, learning, pub audio: AudioLinks }`.

- [ ] **Step 1: Write the failing tests**

Create `src/audio_links.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{Beat, Signal, Signals};
    use crate::control::{Action, Target};
    use crate::motion::Mode;
    use crate::params::table::{ChoiceId, OscSlider, SliderId};
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    const ZOOM: SliderId = SliderId::Zoom;
    const AMP: SliderId = SliderId::Osc(0, OscSlider::Amplitude);

    fn signals(values: [f32; 5], beats: [bool; 3]) -> Signals {
        Signals { values, beats }
    }

    #[test]
    fn a_new_link_moves_a_quarter_of_the_range() {
        let link = AudioLink::new(Signal::Bass, ZOOM);
        let range = ZOOM.range();
        assert_eq!(link.depth, (range.end() - range.start()) / 4.0);
    }

    #[test]
    fn linking_twice_keeps_one_link() {
        let mut links = AudioLinks::default();
        links.follow_signal(Signal::Bass, ZOOM);
        links.follow_signal(Signal::Bass, ZOOM);
        links.fire_on(Beat::Bass, Target::Action(Action::Cut));
        links.fire_on(Beat::Bass, Target::Action(Action::Cut));
        assert_eq!(links.follow.len(), 1);
        assert_eq!(links.beats.len(), 1);
    }

    #[test]
    fn beats_never_link_to_sliders() {
        let mut links = AudioLinks::default();
        links.fire_on(Beat::Any, Target::Slider(ZOOM));
        links.fire_on(Beat::Any, Target::Duration);
        assert!(links.beats.is_empty());
    }

    #[test]
    fn offsets_add_up_per_slider_in_table_order() {
        let mut links = AudioLinks::default();
        links.follow_signal(Signal::Bass, ZOOM);
        links.follow_signal(Signal::Pulse, ZOOM);
        links.follow_signal(Signal::Level, AMP);
        links.follow[0].depth = 1.0;
        links.follow[1].depth = -0.5;
        links.follow[2].depth = 0.2;
        let s = signals([0.5, 0.25, 0.0, 0.0, 1.0], [false; 3]);
        let offsets = links.offsets(&s);
        let order: Vec<SliderId> = SliderId::all()
            .into_iter()
            .filter(|id| *id == ZOOM || *id == AMP)
            .collect();
        assert_eq!(
            offsets.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            order
        );
        let zoom = offsets.iter().find(|(id, _)| *id == ZOOM).unwrap().1;
        assert!((zoom - (1.0 * 0.25 - 0.5 * 1.0)).abs() < 1e-6);
        let amp = offsets.iter().find(|(id, _)| *id == AMP).unwrap().1;
        assert!((amp - 0.2 * 0.5).abs() < 1e-6);
    }

    #[test]
    fn only_this_frames_beats_fire() {
        let mut links = AudioLinks::default();
        let cut = Target::Action(Action::Cut);
        let live = Target::Mode(Mode::Live);
        links.fire_on(Beat::Bass, cut);
        links.fire_on(Beat::Treble, live);
        let s = signals([0.0; 5], [true, true, false]);
        assert_eq!(links.fired(&s), vec![cut]);
    }

    #[test]
    fn unlinking_removes_only_that_pair() {
        let mut links = AudioLinks::default();
        links.follow_signal(Signal::Bass, ZOOM);
        links.follow_signal(Signal::Mid, ZOOM);
        links.unfollow(Signal::Bass, ZOOM);
        assert_eq!(links.signals_for(ZOOM), vec![Signal::Mid]);
        let cut = Target::Action(Action::Cut);
        links.fire_on(Beat::Any, cut);
        links.fire_on(Beat::Bass, cut);
        links.unfire(Beat::Any, cut);
        assert_eq!(links.beats_for(cut), vec![Beat::Bass]);
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq, Default)]
    struct Holder {
        #[serde(with = "saved_follow")]
        follow: Vec<AudioLink>,
        #[serde(with = "saved_beats")]
        beats: Vec<BeatLink>,
    }

    #[test]
    fn links_are_saved_by_name_and_unknown_ones_dropped() {
        let waveform = ChoiceId::Waveform(0);
        let holder = Holder {
            follow: vec![AudioLink {
                signal: Signal::Bass,
                slider: ZOOM,
                depth: 0.5,
            }],
            beats: vec![
                BeatLink {
                    beat: Beat::Bass,
                    target: Target::Action(Action::Cut),
                },
                BeatLink {
                    beat: Beat::Treble,
                    target: Target::Choice(waveform, 1),
                },
            ],
        };
        let value = serde_json::to_value(&holder).unwrap();
        assert_eq!(
            value,
            json!({
                "follow": [{ "signal": "bass", "target": "slider:warp.zoom", "depth": 0.5 }],
                "beats": [
                    { "beat": "bass", "target": "action:cut" },
                    { "beat": "treble", "target": Target::Choice(waveform, 1).saved() },
                ],
            })
        );
        let back: Holder = serde_json::from_value(value).unwrap();
        assert_eq!(back, holder);

        let messy = json!({
            "follow": [
                { "signal": "sub-bass", "target": "slider:warp.zoom", "depth": 0.5 },
                { "signal": "bass", "target": "slider:warp.nonsense", "depth": 0.5 },
                { "signal": "bass", "target": "action:cut", "depth": 0.5 },
                { "signal": "bass", "target": "slider:transition.duration", "depth": 0.5 },
                { "signal": "mid", "target": "slider:warp.zoom" },
                { "signal": "treble", "target": "slider:warp.zoom", "depth": 1.0e9 },
                "nonsense",
            ],
            "beats": [
                { "beat": "snare", "target": "action:cut" },
                { "beat": "any", "target": "slider:warp.zoom" },
                { "beat": "any", "target": "action:dance" },
                { "beat": "any", "target": "mode:live" },
                { "beat": "any", "target": "choice:mode=live" },
            ],
        });
        let read: Holder = serde_json::from_value(messy).unwrap();
        assert_eq!(read.follow.len(), 2, "{:?}", read.follow);
        assert_eq!(read.follow[0].signal, Signal::Mid);
        assert_eq!(
            read.follow[0].depth,
            AudioLink::new(Signal::Mid, ZOOM).depth,
            "a missing depth takes the default"
        );
        let span = ZOOM.range().end() - ZOOM.range().start();
        assert_eq!(read.follow[1].depth, span, "depth held to ± the range's span");
        assert_eq!(
            read.beats,
            vec![BeatLink {
                beat: Beat::Any,
                target: Target::Mode(Mode::Live),
            }]
        );
        let not_a_list: Holder =
            serde_json::from_value(json!({ "follow": "nope", "beats": 3 })).unwrap();
        assert_eq!(not_a_list, Holder::default());
    }
}
```

(The choice's expected string comes from `Target::saved` so the test doesn't hard-code option names.)

Add to `src/control.rs`'s test module:

```rust
    #[test]
    fn firing_a_target_acts_like_a_key() {
        let mut motion = Motion::new(Params::default());
        let waveform = crate::params::table::ChoiceId::Waveform(1);
        assert_eq!(fire(Target::Choice(waveform, 2), &mut motion), None);
        assert_eq!(waveform.get(motion.editable()), 2);
        assert_eq!(fire(Target::Mode(Mode::Transition), &mut motion), None);
        assert_eq!(motion.mode(), Mode::Transition);
        assert_eq!(
            fire(Target::Action(Action::Cut), &mut motion),
            Some(Action::Cut)
        );
        let before = *motion.editable();
        assert_eq!(fire(Target::Slider(SliderId::Zoom), &mut motion), None);
        assert_eq!(fire(Target::Duration, &mut motion), None);
        assert_eq!(*motion.editable(), before, "sliders aren't fired");
    }
```

(`Motion::new(Params)` is how the existing tests build one; `Params` is already imported in that test module.)

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib audio_links control::tests::firing`
Expected: compile errors (`AudioLink`, `fire` not found).

- [ ] **Step 3: Factor `fire` out of `Links::handle`**

In `src/control.rs`, add above `impl Links` (after the `Link` impl):

```rust
/// Fires a key-style target the way a MIDI key does: picks an option in the bank the
/// panel edits, switches the mode, or returns the action for the app to run. Sliders
/// and the duration aren't fired; they return None and change nothing.
pub fn fire(target: Target, motion: &mut Motion) -> Option<Action> {
    match target {
        Target::Choice(id, option) => id.set(motion.editable(), option),
        Target::Mode(mode) => motion.set_mode(mode),
        Target::Action(action) => return Some(action),
        Target::Slider(_) | Target::Duration => {}
    }
    None
}
```

and in `Links::handle` replace the three note arms

```rust
                (Target::Choice(id, option), None) => id.set(motion.editable(), option),
                (Target::Mode(mode), None) => motion.set_mode(mode),
                (Target::Action(action), None) => actions.push(action),
```

with

```rust
                (target, None) => actions.extend(fire(target, motion)),
```

(The two slider arms above it stay; the `_ => {}` arm stays for sliders reached by a note.)

Add the audio links to `Links`:

```rust
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Links {
    /// Every MIDI link, in the order they were made (saved with the app settings).
    pub list: Vec<Link>,
    learning: Option<Target>,
    /// The audio links (saved with the project).
    pub audio: AudioLinks,
}
```

with `use crate::audio_links::AudioLinks;` at the top. `Links::new(list)` must set `audio: AudioLinks::default()` (use `..Default::default()` if it builds the struct literally).

- [ ] **Step 4: Write `src/audio_links.rs` above its tests**

```rust
//! Audio links: a follow signal pushing a slider up or down from where the hand left
//! it, and a beat firing an option, mode or action the way a MIDI key does. Saved with
//! the project.

use crate::audio::{Beat, Signal, Signals};
use crate::control::Target;
use crate::params::table::SliderId;

/// A follow signal moving a slider: the slider shows its base plus `depth × signal`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioLink {
    pub signal: Signal,
    pub slider: SliderId,
    /// How far a signal of 1 moves the slider, in its own units; negative pulls down.
    pub depth: f32,
}

impl AudioLink {
    /// A link that moves `slider` by a quarter of its range at full signal.
    pub fn new(signal: Signal, slider: SliderId) -> Self {
        let range = slider.range();
        Self {
            signal,
            slider,
            depth: (range.end() - range.start()) / 4.0,
        }
    }

    /// The most a depth may be either way: the slider's whole span.
    pub fn span(&self) -> f32 {
        let range = self.slider.range();
        range.end() - range.start()
    }
}

/// A beat firing a key-style target (an option, a mode or an action).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeatLink {
    pub beat: Beat,
    pub target: Target,
}

/// Every audio link in the project.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioLinks {
    /// Follow links, in the order they were made.
    pub follow: Vec<AudioLink>,
    /// Beat links, in the order they were made.
    pub beats: Vec<BeatLink>,
}

impl AudioLinks {
    /// Links `signal` to `slider`, unless it already is.
    pub fn follow_signal(&mut self, signal: Signal, slider: SliderId) {
        if !self
            .follow
            .iter()
            .any(|l| l.signal == signal && l.slider == slider)
        {
            self.follow.push(AudioLink::new(signal, slider));
        }
    }

    /// Links `beat` to `target`, unless it already is. Sliders and the duration can't be
    /// fired, so they are ignored.
    pub fn fire_on(&mut self, beat: Beat, target: Target) {
        if target.continuous() {
            return;
        }
        let link = BeatLink { beat, target };
        if !self.beats.contains(&link) {
            self.beats.push(link);
        }
    }

    /// Removes the link from `signal` to `slider`.
    pub fn unfollow(&mut self, signal: Signal, slider: SliderId) {
        self.follow
            .retain(|l| !(l.signal == signal && l.slider == slider));
    }

    /// Removes the link from `beat` to `target`.
    pub fn unfire(&mut self, beat: Beat, target: Target) {
        self.beats.retain(|l| !(l.beat == beat && l.target == target));
    }

    /// The signals `slider` follows.
    pub fn signals_for(&self, slider: SliderId) -> Vec<Signal> {
        self.follow
            .iter()
            .filter(|l| l.slider == slider)
            .map(|l| l.signal)
            .collect()
    }

    /// The beats that fire `target`.
    pub fn beats_for(&self, target: Target) -> Vec<Beat> {
        self.beats
            .iter()
            .filter(|l| l.target == target)
            .map(|l| l.beat)
            .collect()
    }

    /// How far each linked slider moves this frame: the sum of its links' `depth ×
    /// signal`, in [`SliderId::all`] order (so the level count comes before the
    /// thresholds it re-spaces).
    pub fn offsets(&self, signals: &Signals) -> Vec<(SliderId, f32)> {
        if self.follow.is_empty() {
            return Vec::new();
        }
        SliderId::all()
            .into_iter()
            .filter_map(|id| {
                let mut linked = self.follow.iter().filter(|l| l.slider == id).peekable();
                linked.peek()?;
                Some((id, linked.map(|l| l.depth * signals.get(l.signal)).sum()))
            })
            .collect()
    }

    /// The targets this frame's beats fire, in the order the links were made.
    pub fn fired(&self, signals: &Signals) -> Vec<Target> {
        self.beats
            .iter()
            .filter(|l| signals.beat(l.beat))
            .map(|l| l.target)
            .collect()
    }
}

/// An f32 as the f64 with the same shortest decimal (0.1, not 0.10000000149), so a
/// saved depth reads the way it was typed.
fn decimal(value: f32) -> f64 {
    value.to_string().parse().unwrap_or(f64::from(value))
}

/// Saving follow links as `{ "signal": "bass", "target": "slider:warp.zoom", "depth":
/// 0.5 }`, for `#[serde(with = "crate::audio_links::saved_follow")]`.
pub mod saved_follow {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::Value;

    use super::{AudioLink, decimal};
    use crate::audio::Signal;
    use crate::control::Target;

    #[derive(Serialize)]
    struct Saved {
        signal: &'static str,
        target: String,
        depth: f64,
    }

    /// Writes each link by name.
    pub fn serialize<S: Serializer>(links: &[AudioLink], s: S) -> Result<S::Ok, S::Error> {
        links
            .iter()
            .map(|l| Saved {
                signal: l.signal.name(),
                target: Target::Slider(l.slider).saved(),
                depth: decimal(l.depth),
            })
            .collect::<Vec<_>>()
            .serialize(s)
    }

    /// Reads the links it knows; anything else (or a list that isn't a list) is left out.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<AudioLink>, D::Error> {
        let value = Value::deserialize(d)?;
        let entries = value.as_array().cloned().unwrap_or_default();
        Ok(entries
            .iter()
            .filter_map(|entry| {
                let link = read(entry);
                if link.is_none() {
                    log::warn!("ignoring an audio link this version doesn't know: {entry}");
                }
                link
            })
            .collect())
    }

    fn read(entry: &Value) -> Option<AudioLink> {
        let signal = Signal::from_name(entry.get("signal")?.as_str()?)?;
        let Target::Slider(slider) = Target::from_saved(entry.get("target")?.as_str()?)?
        else {
            return None;
        };
        let mut link = AudioLink::new(signal, slider);
        if let Some(depth) = entry.get("depth").and_then(Value::as_f64) {
            let span = link.span();
            link.depth = (depth as f32).clamp(-span, span);
        }
        Some(link)
    }
}

/// Saving beat links as `{ "beat": "bass", "target": "action:cut" }`, for
/// `#[serde(with = "crate::audio_links::saved_beats")]`.
pub mod saved_beats {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::Value;

    use super::BeatLink;
    use crate::audio::Beat;
    use crate::control::Target;

    #[derive(Serialize)]
    struct Saved {
        beat: &'static str,
        target: String,
    }

    /// Writes each link by name.
    pub fn serialize<S: Serializer>(links: &[BeatLink], s: S) -> Result<S::Ok, S::Error> {
        links
            .iter()
            .map(|l| Saved {
                beat: l.beat.name(),
                target: l.target.saved(),
            })
            .collect::<Vec<_>>()
            .serialize(s)
    }

    /// Reads the links it knows; anything else (or a list that isn't a list) is left out.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<BeatLink>, D::Error> {
        let value = Value::deserialize(d)?;
        let entries = value.as_array().cloned().unwrap_or_default();
        Ok(entries
            .iter()
            .filter_map(|entry| {
                let link = read(entry);
                if link.is_none() {
                    log::warn!("ignoring a beat link this version doesn't know: {entry}");
                }
                link
            })
            .collect())
    }

    fn read(entry: &Value) -> Option<BeatLink> {
        let beat = Beat::from_name(entry.get("beat")?.as_str()?)?;
        let target = Target::from_saved(entry.get("target")?.as_str()?)?;
        (!target.continuous()).then_some(BeatLink { beat, target })
    }
}
```

In `src/lib.rs` add `pub mod audio_links;` after `pub mod audio;`.

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib audio_links control`
Expected: all pass, including the existing `keys_pick_options_and_modes_and_report_actions` (the refactor of `handle` must not change it).

- [ ] **Step 6: Check and commit**

Run: `cargo fmt`, `cargo clippy --all-targets`, `cargo test --lib`, `cargo test --no-run`.

```bash
git add src/audio_links.rs src/control.rs src/lib.rs
git commit -m "feat: audio links: follow offsets, beat targets, saved forms"
```

---

### Task 3: Modulation in Motion

**Files:**
- Modify: `src/motion.rs` (`offsets` field, `set_offsets`, `shown` helper; `advance`, `preview`, `blended` use it)

**Interfaces:**
- Consumes: `SliderId::{get, set}`.
- Produces: `Motion::set_offsets(&mut self, offsets: Vec<(SliderId, f32)>)` and `Motion::offsets(&self) -> &[(SliderId, f32)]`. While offsets are set, `frame()`, `preview()` and `advance()` work on copies of the banks with each offset added through `SliderId::set`; `editable()`, the banks and the cues never change.

- [ ] **Step 1: Write the failing tests**

Add to `src/motion.rs`'s test module (`Motion::new(Params::default())` builds one; import `Params` and `Mode` in the test module if they aren't already):

```rust
    #[test]
    fn offsets_move_the_frame_but_not_the_bank() {
        use crate::params::table::SliderId;
        let mut motion = Motion::new(Params::default());
        let base = motion.editable().warp.zoom;
        motion.set_offsets(vec![(SliderId::Zoom, 0.5)]);
        assert!((motion.frame().warp.zoom - (base + 0.5)).abs() < 1e-6);
        assert_eq!(motion.editable().warp.zoom, base, "the bank keeps the hand's value");
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
        assert_eq!(shown.colorize.levels, levels + 1, "rounded like a hand edit");
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
```

The names `OscSlider::PhaseSpeed`, `OscSlot { index, phase }`, `SliderId::Levels` and `colorize.levels` are the ones in `src/params/table.rs`, `src/params.rs` and `src/blend.rs`; if a field is named differently there (for example the blended oscillator's phase field), use the real name — the assertions' meaning must stay: the frame moves, the bank doesn't, and a modulated phase speed changes the accumulated phase.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib motion::tests::offsets motion::tests::a_modulated motion::tests::the_preview`
Expected: compile error (`set_offsets` not found).

- [ ] **Step 3: Add the offsets**

In `src/motion.rs`:

1. Add `use crate::params::table::SliderId;` with the other imports.

2. Add a field to `Motion` (with its other fields; include it in any struct literal or `Default` for `Motion` as `offsets: Vec::new()`):

```rust
    /// What audio adds to each linked slider this frame, in table order. Applied to
    /// copies of the banks it blends; the banks themselves never change.
    offsets: Vec<(SliderId, f32)>,
```

3. Add a free function at module level:

```rust
/// `p` with each offset added through the table's setter (held in range, rounded, tied
/// values kept).
fn shown(offsets: &[(SliderId, f32)], p: &Params) -> Params {
    let mut p = *p;
    for &(id, offset) in offsets {
        id.set(&mut p, id.get(&p) + offset);
    }
    p
}
```

4. Add methods:

```rust
    /// Sets what audio adds to each linked slider from the next frame on.
    pub fn set_offsets(&mut self, offsets: Vec<(SliderId, f32)>) {
        self.offsets = offsets;
    }

    /// What audio adds to each linked slider now.
    pub fn offsets(&self) -> &[(SliderId, f32)] {
        &self.offsets
    }
```

5. In `advance`, feed modulated copies everywhere a bank or cue is read. Because `seq` borrows `self.sequence` mutably, use the field `self.offsets` directly (disjoint borrows), never a `&self` method:

```rust
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
                ...unchanged...
            }
```

and in the Sequence arm:

```rust
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
```

and for the preview at the end of `advance`:

```rust
        if let Some((_, p)) = preview {
            self.preview.advance(&shown(&self.offsets, &p), dt);
        }
```

6. In `preview()`:

```rust
        let (source, p) = self.preview_params()?;
        let p = shown(&self.offsets, p);
        let mut frame = blend(&p, &p, 0.0, None, &self.preview, &self.preview);
```

7. In `blended()`, replace each bank or cue reference passed to `blend` (directly or through the `rest` closure) by its modulated copy:

```rust
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
```

(Keep the exact `blend` argument types the current code uses — if `blend` takes the eased value as `f32` and progress as `Option<f32>`, the closure above matches; adjust only the types, not the meaning.)

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib motion`
Expected: all pass, the existing motion tests included (no offsets set means `shown` returns an identical copy).

- [ ] **Step 5: Check and commit**

Run: `cargo fmt`, `cargo clippy --all-targets`, `cargo test --lib`, `cargo test --test smoke` (motion feeds the renderer).

```bash
git add src/motion.rs
git commit -m "feat: Motion applies audio offsets to copies of the banks it blends"
```

---
### Task 4: Sound files: decoding, tracks and loading

**Files:**
- Modify: `Cargo.toml` (add `"software-resampling"` to ffmpeg-next's features)
- Create: `src/audio/file.rs`
- Modify: `src/audio/mod.rs` (add `pub mod file;`)
- Create: `tests/audio.rs`

**Interfaces:**
- Consumes (Task 1): `Analyzer`, `Shaping`, `RATE`. Existing: `crate::video::{Budget, Reservation, describe_bytes}`.
- Produces (`crate::audio::file`):
  - `pub const TRACK_RATE: u32 = 1000;`
  - `pub struct Tracks` (`Clone, Debug, Default, PartialEq`) with `analyse(samples: &[i16], shaping: Shaping) -> Tracks` (interleaved stereo at `RATE`), `at(&self, seconds: f64) -> [f32; 4]` (Level, Bass, Mid, Treble before gain, interpolated), `beats_in(&self, from: f64, to: f64) -> [bool; 3]` (onsets with `from <= t < to`; when `to < from` the span wrapped: `t >= from || t < to`).
  - `pub struct Sound { pub name: String, pub samples: Arc<[i16]>, pub tracks: Tracks, .. }` with `new(name: String, samples: Vec<i16>, shaping: Shaping, budget: &Budget) -> Result<Sound>`, `frames(&self) -> usize`, `seconds(&self) -> f64`, `slice(&self, from: f64, frames: usize, looping: bool) -> Vec<i16>`.
  - `pub fn decode(path: &Path, progress: &AtomicU32, cancel: &AtomicBool) -> Result<Vec<i16>>` (interleaved stereo at `RATE`; progress 0..=1000).
  - `pub fn load(path: &Path, shaping: Shaping, budget: &Budget, progress: &AtomicU32, cancel: &AtomicBool) -> Result<Sound>` (errors read "Couldn't read <file>: <reason>", or the memory message).
  - `pub struct Loading` with `start(path: &Path, shaping: Shaping, budget: &Budget) -> Loading`, `pub path: PathBuf`, `progress(&self) -> f32` (0..=1), `poll(&self) -> Option<Result<Sound>>`; dropping it cancels.
  - `pub struct Reanalysis` with `start(samples: Arc<[i16]>, shaping: Shaping) -> Reanalysis`, `poll(&self) -> Option<Tracks>`.

FFmpeg facts (verified with ffmpeg-next 9.0.0 and the FFmpeg 9.0.2 build on this machine):
- `ff::software::resampling::Context::get(src_format, src_layout, src_rate, dst_format, dst_layout, dst_rate) -> Result<Context, ff::Error>`; `run(&mut self, &frame::Audio, &mut frame::Audio)`; `flush(&mut self, &mut frame::Audio)`.
- `ff::ChannelLayout` has `MONO`, `STEREO`, `default(channels: i32)`, `is_empty()`. **A mono WAV decodes with an empty layout**: set `ChannelLayout::default(channels)` on each decoded frame before resampling, or swresample fails with "Input changed".
- **Every `run` and `flush` needs a fresh, pre-allocated output frame** (`frame::Audio::new(S16, capacity, ChannelLayout::STEREO)` then `set_rate(48000)`), sized `ceil(in_samples × 48000 / in_rate) + 64`; flushing into `frame::Audio::empty()` fails with "Output changed". Flush in a loop until a frame comes back with 0 samples.
- Build the resampler from the first decoded **frame's** format, layout and rate (mp3 only knows its planar float format once frames arrive).
- Packed s16 output: `out.plane::<i16>(0)` is the interleaved L,R samples, `samples × 2` long.
- `samples/drums.wav` (44.1 kHz mono, 16-bit) decodes to **420,612** stereo frames at 48 kHz (8.7628 s).

- [ ] **Step 1: Enable resampling**

In `Cargo.toml`, change the ffmpeg-next line to:

```toml
ffmpeg-next = { version = "9.0.0", default-features = false, features = ["codec", "device", "format", "software-resampling", "software-scaling"] }
```

(`swresample-7.dll` is already in `build.rs`'s `DLLS`; nothing else to copy.)

- [ ] **Step 2: Write the failing tests**

Create `src/audio/file.rs` with only its test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::{Budget, MEMORY_LIMIT};
    use std::f32::consts::TAU;

    /// Interleaved stereo: `count` 20 ms bursts of 1 kHz, `gap` seconds apart.
    fn clicks(count: usize, gap: f64) -> Vec<i16> {
        let frames = (count as f64 * gap * RATE as f64) as usize;
        let burst = (0.02 * RATE as f64) as usize;
        let every = (gap * RATE as f64) as usize;
        let mut out = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let v = if i % every < burst {
                ((TAU * 1000.0 * i as f32 / RATE as f32).sin() * 30000.0) as i16
            } else {
                0
            };
            out.extend([v, v]);
        }
        out
    }

    fn ramp(frames: usize) -> Vec<i16> {
        (0..frames).flat_map(|i| [i as i16, -(i as i16)]).collect()
    }

    #[test]
    fn tracks_hold_a_value_per_millisecond_and_find_the_clicks() {
        let tracks = Tracks::analyse(&clicks(4, 0.5), Shaping::default());
        assert_eq!(tracks.values.len(), 2000);
        let all = tracks.beats_in(0.0, 2.0);
        assert_eq!(all, [true, false, true], "1 kHz clicks are Any and Treble");
        let mut count = 0;
        let mut t = 0.0;
        while t < 2.0 {
            count += usize::from(tracks.beats_in(t, t + 1.0 / 60.0)[0]);
            t += 1.0 / 60.0;
        }
        assert_eq!(count, 4, "each click fires once across frame spans");
    }

    #[test]
    fn a_wrapped_span_covers_the_end_and_the_start() {
        let tracks = Tracks::analyse(&clicks(4, 0.5), Shaping::default());
        assert!(tracks.beats_in(1.99, 0.01)[0], "the first click, just after the wrap");
        assert!(!tracks.beats_in(1.9, 1.95)[0]);
    }

    #[test]
    fn reading_between_values_interpolates_and_holds_at_the_ends() {
        let tracks = Tracks {
            values: vec![[0.0; 4], [1.0; 4]],
            beats: Default::default(),
        };
        assert_eq!(tracks.at(0.0005), [0.5; 4]);
        assert_eq!(tracks.at(-1.0), [0.0; 4]);
        assert_eq!(tracks.at(10.0), [1.0; 4]);
        assert_eq!(Tracks::default().at(1.0), [0.0; 4]);
    }

    #[test]
    fn slices_wrap_when_looping_and_fall_silent_when_not() {
        let budget = Budget::new(MEMORY_LIMIT);
        let sound = Sound::new("ramp".into(), ramp(100), Shaping::default(), &budget).unwrap();
        assert_eq!(sound.frames(), 100);
        let from = 98.0 / RATE as f64;
        assert_eq!(sound.slice(from, 4, true), [98, -98, 99, -99, 0, 0, 1, -1]);
        assert_eq!(sound.slice(from, 4, false), [98, -98, 99, -99, 0, 0, 0, 0]);
        assert_eq!(sound.slice(0.0, 2, false), [0, 0, 1, -1]);
    }

    #[test]
    fn a_sound_is_charged_to_the_budget() {
        let budget = Budget::new(MEMORY_LIMIT);
        let before = budget.available();
        let sound = Sound::new("ramp".into(), ramp(48_000), Shaping::default(), &budget).unwrap();
        assert!(before - budget.available() >= 48_000 * 4);
        drop(sound);
        assert_eq!(budget.available(), before);
        let tiny = Budget::new(1000);
        let err = Sound::new("big.wav".into(), ramp(48_000), Shaping::default(), &tiny)
            .err()
            .expect("it doesn't fit");
        assert!(
            format!("{err:#}").starts_with("This file needs"),
            "{err:#}"
        );
    }
}
```

Create `tests/audio.rs`:

```rust
//! Decoding real sound files.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32};

use rasterwarp::audio::Shaping;
use rasterwarp::audio::file;
use rasterwarp::video::{Budget, MEMORY_LIMIT};

#[test]
fn drums_decode_to_48_khz_stereo_with_beats() {
    let budget = Budget::new(MEMORY_LIMIT);
    let progress = AtomicU32::new(0);
    let sound = file::load(
        Path::new("samples/drums.wav"),
        Shaping::default(),
        &budget,
        &progress,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(sound.frames(), 420_612);
    assert!((sound.seconds() - 8.7628).abs() < 0.001);
    assert_eq!(sound.name, "drums.wav");
    assert!(sound.tracks.beats_in(0.0, sound.seconds())[0], "drums have beats");
    assert_eq!(progress.load(std::sync::atomic::Ordering::Relaxed), 1000);
}

#[test]
fn a_file_without_sound_says_so() {
    let budget = Budget::new(MEMORY_LIMIT);
    let err = file::load(
        Path::new("samples/displace_test.png"),
        Shaping::default(),
        &budget,
        &AtomicU32::new(0),
        &AtomicBool::new(false),
    )
    .err()
    .expect("a picture has no sound");
    assert!(
        format!("{err:#}").starts_with("Couldn't read displace_test.png: "),
        "{err:#}"
    );
}
```

Add `pub mod file;` to `src/audio/mod.rs` (after `pub mod analysis;`).

- [ ] **Step 3: Run them to see them fail**

Run: `cargo test --lib audio::file` and `cargo test --test audio`
Expected: compile errors (`Tracks`, `Sound`, `load` not found).

- [ ] **Step 4: Write `src/audio/file.rs` above its tests**

```rust
//! Sound files: decoded once into 48 kHz stereo samples (for the speakers and the
//! soundtrack), and analysed once into tracks that links read at the playhead's time,
//! so the same moment of a song always gives the same signals.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use anyhow::{Context, Result, anyhow, bail};
use ff::format::sample::{Sample, Type};
use ff::software::resampling;
use ff::{ChannelLayout, frame};
use ffmpeg_next as ff;

use super::analysis::Analyzer;
use super::{RATE, Shaping};
use crate::video::{Budget, Reservation, describe_bytes};

/// Track values per second.
pub const TRACK_RATE: u32 = 1000;
/// Samples per track value.
const STEP: usize = (RATE / TRACK_RATE) as usize;
/// What the decoder writes: packed (interleaved) 16-bit.
const S16: Sample = Sample::I16(Type::Packed);

/// A file's analysis: Level, Bass, Mid and Treble (before gain) every millisecond, and
/// when each beat starts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tracks {
    values: Vec<[f32; 4]>,
    /// Onset times in seconds, indexed by `Beat::index`, each sorted.
    beats: [Vec<f64>; 3],
}

impl Tracks {
    /// Analyses interleaved stereo `samples` at [`RATE`].
    pub fn analyse(samples: &[i16], shaping: Shaping) -> Tracks {
        let mut analyzer = Analyzer::new(RATE, shaping);
        let mut tracks = Tracks {
            values: Vec::with_capacity(samples.len() / 2 / STEP + 1),
            beats: Default::default(),
        };
        for (i, pair) in samples.chunks_exact(2).enumerate() {
            let x = (f32::from(pair[0]) + f32::from(pair[1])) / (2.0 * 32768.0);
            for (times, fired) in tracks.beats.iter_mut().zip(analyzer.push(x)) {
                if fired {
                    times.push(i as f64 / f64::from(RATE));
                }
            }
            if i % STEP == STEP - 1 {
                tracks.values.push(analyzer.values());
            }
        }
        tracks
    }

    /// Level, Bass, Mid and Treble at `seconds`, before gain; held at the first and
    /// last values outside the file.
    pub fn at(&self, seconds: f64) -> [f32; 4] {
        let Some(last) = self.values.len().checked_sub(1) else {
            return [0.0; 4];
        };
        let pos = (seconds * f64::from(TRACK_RATE)).clamp(0.0, last as f64);
        let i = pos.floor() as usize;
        let t = (pos - i as f64) as f32;
        let (a, b) = (self.values[i], self.values[(i + 1).min(last)]);
        std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
    }

    /// Which beats start in the span from `from` up to (not including) `to`. A span
    /// with `to < from` wrapped around the end of a looping file.
    pub fn beats_in(&self, from: f64, to: f64) -> [bool; 3] {
        self.beats.each_ref().map(|times| {
            let starts_at_or_after = |t: f64| times.partition_point(|&b| b < t);
            if to >= from {
                starts_at_or_after(from) < starts_at_or_after(to)
            } else {
                starts_at_or_after(from) < times.len() || starts_at_or_after(to) > 0
            }
        })
    }
}

/// A decoded, analysed sound file.
#[derive(Debug)]
pub struct Sound {
    /// The file's name, for the panel.
    pub name: String,
    /// Interleaved stereo samples at [`RATE`].
    pub samples: Arc<[i16]>,
    pub tracks: Tracks,
    _memory: Reservation,
}

/// Memory a sound of `frames` stereo frames holds: its samples and its tracks.
fn bytes_for(frames: usize) -> u64 {
    let tracks = frames / STEP * std::mem::size_of::<[f32; 4]>();
    (frames * 4 + tracks) as u64
}

impl Sound {
    /// Charges `samples` (interleaved stereo at [`RATE`]) to `budget` and analyses them.
    pub fn new(name: String, samples: Vec<i16>, shaping: Shaping, budget: &Budget) -> Result<Sound> {
        let needed = bytes_for(samples.len() / 2);
        let Some(memory) = budget.take(needed) else {
            bail!(
                "This file needs {} of memory and only {} is free",
                describe_bytes(needed),
                describe_bytes(budget.available())
            );
        };
        let tracks = Tracks::analyse(&samples, shaping);
        Ok(Sound {
            name,
            samples: samples.into(),
            tracks,
            _memory: memory,
        })
    }

    /// Its length in stereo frames.
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    /// Its length in seconds.
    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / f64::from(RATE)
    }

    /// `frames` stereo frames starting at `from` seconds: wrapping to the start when
    /// `looping`, silence past the end when not.
    pub fn slice(&self, from: f64, frames: usize, looping: bool) -> Vec<i16> {
        let total = self.frames();
        let mut out = Vec::with_capacity(frames * 2);
        if total == 0 {
            out.resize(frames * 2, 0);
            return out;
        }
        let mut at = (from * f64::from(RATE)).round().max(0.0) as usize;
        if looping {
            at %= total;
        }
        for _ in 0..frames {
            if at >= total {
                if looping {
                    at = 0;
                } else {
                    out.extend([0, 0]);
                    continue;
                }
            }
            out.extend_from_slice(&self.samples[at * 2..at * 2 + 2]);
            at += 1;
        }
        out
    }
}

/// A fresh output frame with room for `capacity` stereo frames.
fn output_frame(capacity: usize) -> frame::Audio {
    let mut out = frame::Audio::new(S16, capacity, ChannelLayout::STEREO);
    out.set_rate(RATE);
    out
}

/// Decodes the first audio stream of `path` into interleaved stereo at [`RATE`].
/// `progress` goes 0..=1000; `cancel` stops it.
pub fn decode(path: &Path, progress: &AtomicU32, cancel: &AtomicBool) -> Result<Vec<i16>> {
    ff::init().context("could not initialise FFmpeg")?;
    let mut input = ff::format::input(path)?;
    let stream = input
        .streams()
        .best(ff::media::Type::Audio)
        .ok_or_else(|| anyhow!("no sound in it"))?;
    let index = stream.index();
    let mut decoder = ff::codec::context::Context::from_parameters(stream.parameters())?
        .decoder()
        .audio()?;
    let expected = input.duration().max(0) as f64 / f64::from(ff::ffi::AV_TIME_BASE)
        * f64::from(RATE);
    let mut samples: Vec<i16> = Vec::new();
    let mut resampler: Option<resampling::Context> = None;
    let mut decoded = frame::Audio::empty();
    let mut take = |decoded: &mut frame::Audio,
                    resampler: &mut Option<resampling::Context>,
                    samples: &mut Vec<i16>|
     -> Result<()> {
        if decoded.channel_layout().is_empty() {
            decoded.set_channel_layout(ChannelLayout::default(i32::from(decoded.channels())));
        }
        let r = match resampler {
            Some(r) => r,
            None => resampler.insert(resampling::Context::get(
                decoded.format(),
                decoded.channel_layout(),
                decoded.rate(),
                S16,
                ChannelLayout::STEREO,
                RATE,
            )?),
        };
        let capacity = (decoded.samples() as u64 * u64::from(RATE))
            .div_ceil(u64::from(decoded.rate().max(1))) as usize
            + 64;
        let mut out = output_frame(capacity);
        r.run(decoded, &mut out)?;
        samples.extend_from_slice(out.plane::<i16>(0));
        if expected > 0.0 {
            let done = (samples.len() / 2) as f64 / expected;
            progress.store((done.min(1.0) * 999.0) as u32, Ordering::Relaxed);
        }
        Ok(())
    };
    for (stream, packet) in input.packets() {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        if stream.index() == index {
            decoder.send_packet(&packet)?;
            while decoder.receive_frame(&mut decoded).is_ok() {
                take(&mut decoded, &mut resampler, &mut samples)?;
            }
        }
    }
    decoder.send_eof()?;
    while decoder.receive_frame(&mut decoded).is_ok() {
        take(&mut decoded, &mut resampler, &mut samples)?;
    }
    if let Some(mut r) = resampler {
        loop {
            let mut out = output_frame(4096);
            r.flush(&mut out)?;
            if out.samples() == 0 {
                break;
            }
            samples.extend_from_slice(out.plane::<i16>(0));
        }
    }
    progress.store(1000, Ordering::Relaxed);
    Ok(samples)
}

/// The file name of `path`, for messages.
fn name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// Decodes, charges and analyses `path`.
pub fn load(
    path: &Path,
    shaping: Shaping,
    budget: &Budget,
    progress: &AtomicU32,
    cancel: &AtomicBool,
) -> Result<Sound> {
    let name = name(path);
    let samples =
        decode(path, progress, cancel).with_context(|| format!("Couldn't read {name}"))?;
    Sound::new(name, samples, shaping, budget)
}

/// A file loading on a worker thread. Dropping it cancels the load.
pub struct Loading {
    /// The file being loaded.
    pub path: PathBuf,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    result: Receiver<Result<Sound>>,
}

impl Loading {
    /// Starts loading `path`.
    pub fn start(path: &Path, shaping: Shaping, budget: &Budget) -> Loading {
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (send, result) = mpsc::channel();
        let (owned, p, c, budget) = (
            path.to_path_buf(),
            progress.clone(),
            cancel.clone(),
            budget.clone(),
        );
        let spawned = std::thread::Builder::new()
            .name("sound loader".into())
            .spawn({
                let send = send.clone();
                move || {
                    let _ = send.send(load(&owned, shaping, &budget, &p, &c));
                }
            });
        if let Err(err) = spawned {
            let _ = send.send(Err(anyhow!("could not start loading {}: {err}", name(path))));
        }
        Loading {
            path: path.to_path_buf(),
            progress,
            cancel,
            result,
        }
    }

    /// How far it has got, 0 to 1.
    pub fn progress(&self) -> f32 {
        self.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// The sound, or why it failed, once the load has finished.
    pub fn poll(&self) -> Option<Result<Sound>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(anyhow!(
                "the sound loader for {} stopped",
                name(&self.path)
            ))),
        }
    }
}

impl Drop for Loading {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// A loaded file's analysis being re-run with new shaping, on a worker thread.
pub struct Reanalysis {
    result: Receiver<Tracks>,
}

impl Reanalysis {
    /// Starts analysing `samples` with `shaping`.
    pub fn start(samples: Arc<[i16]>, shaping: Shaping) -> Reanalysis {
        let (send, result) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("sound analysis".into())
            .spawn(move || {
                let _ = send.send(Tracks::analyse(&samples, shaping));
            });
        if let Err(err) = spawned {
            log::warn!("could not start re-analysing the sound: {err}");
        }
        Reanalysis { result }
    }

    /// The new tracks, once ready.
    pub fn poll(&self) -> Option<Tracks> {
        self.result.try_recv().ok()
    }
}
```

The closure `take` captures `progress` and `expected`; if the borrow checker objects to the closure borrowing `progress` while it is also used after the loops, turn `take` into a private `fn take(decoded, resampler, samples, progress, expected) -> Result<()>`. A `Reanalysis` whose thread failed to start never yields; the engine (Task 6) treats that as "keep the old tracks".

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib audio::file` and `cargo test --test audio`
Expected: all pass. drums.wav must give exactly 420,612 frames; if it gives fewer, the flush loop is missing or an output frame was reused.

- [ ] **Step 6: Check and commit**

Run: `cargo fmt`, `cargo clippy --all-targets`, `cargo test --lib`, `cargo test --no-run`.

```bash
git add Cargo.toml Cargo.lock src/audio tests/audio.rs
git commit -m "feat: sound files decode to 48 kHz stereo and analyse into tracks"
```

---

### Task 5: Live input, loopback and speakers

**Files:**
- Modify: `Cargo.toml` (add `cpal = "0.18"`, alphabetically after `chrono`)
- Create: `src/audio/input.rs`
- Create: `src/audio/output.rs`
- Modify: `src/audio/mod.rs` (add `pub mod input;` and `pub mod output;`)

**Interfaces:**
- Consumes: `RATE` (Task 1).
- Produces (`crate::audio::input`):
  - `pub enum LiveKind { Input, Loopback }`
  - `pub fn input_devices() -> Result<Vec<String>, String>`, `pub fn output_devices() -> Result<Vec<String>, String>` (device names; loopback sources are the output devices).
  - `pub struct Live` with `open(kind: LiveKind, name: &str) -> anyhow::Result<Live>`, `rate(&self) -> u32`, `drain(&mut self, now: Instant) -> Vec<f32>` (mono samples since the last call, with zeros filling any silence longer than 50 ms), `failure(&self) -> Option<String>`.
  - `pub fn mono(data: &[f32], channels: usize) -> Vec<f32>`, `pub fn silence_to_fill(last: Instant, now: Instant, rate: u32) -> usize`.
- Produces (`crate::audio::output`):
  - `pub const SYNC: f64 = 0.04;`
  - `pub struct Speakers` with `open(device: &str, samples: Arc<[i16]>) -> anyhow::Result<Speakers>` (`""` = the system default), `follow(&self, seconds: f64, playing: bool, looping: bool, volume: f32)`, `failure(&self) -> Option<String>`.
  - `pub struct Shared` (the state the speaker callback reads) with `new(samples: Arc<[i16]>) -> Shared`, `fill(&self, out: &mut [f32])`, `follow(&self, seconds, playing, looping, volume)`, `position(&self) -> u64` — public so its behaviour is unit-tested without a device.

cpal 0.18.2 facts (verified on this machine):
- `cpal::default_host()`; `host.input_devices()` / `host.output_devices()` return `Result<impl Iterator<Item = cpal::Device>, cpal::Error>`; a device's name is its `Display` (`device.to_string()`); `host.default_output_device()`.
- `device.default_input_config()` / `default_output_config()` → `SupportedStreamConfig` with `sample_format()`, `channels()`, `sample_rate() -> u32`, `config() -> StreamConfig`.
- `device.build_input_stream::<T, _, _>(config, |data: &[T], _: &cpal::InputCallbackInfo| .., |err: cpal::Error| .., None)` and `build_output_stream::<f32, _, _>(config, |data: &mut [f32], _: &cpal::OutputCallbackInfo| .., err_fn, None)`; streams start paused — call `stream.play()`. Dropping the stream stops it. `cpal::Stream` is `Send + Sync`.
- **Loopback** is `build_input_stream` on an *output* device, with `default_output_config().config()` (its `default_input_config()` fails). It delivers **no callbacks while nothing plays** — hence the zero-filling in `Live::drain`.
- Input must use the device's own rate and channels (`default_input_config()`); mix to mono and analyse at that rate (the analyzer is rate-aware). Output may request `StreamConfig { channels: 2, sample_rate: 48000, buffer_size: cpal::BufferSize::Default }`; WASAPI converts.
- Errors are `cpal::Error` with `kind() -> cpal::ErrorKind` (`#[non_exhaustive]`: `DeviceBusy`, `DeviceNotAvailable`, `Xrun`, `UnsupportedConfig`, `StreamInvalidated`, …; always keep a `_` arm). Every capture stream reports one `Xrun` right after `play()`: ignore `Xrun`.
- Sample conversion: `cpal::Sample::to_sample::<f32>()` with bounds `T: cpal::SizedSample + Send + 'static, f32: cpal::FromSample<T>`.

- [ ] **Step 1: Write the failing tests**

`src/audio/input.rs` test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn channels_mix_to_mono() {
        assert_eq!(mono(&[1.0, 0.0, 0.5, 0.5], 2), [0.5, 0.5]);
        assert_eq!(mono(&[0.25, -0.25], 1), [0.25, -0.25]);
        assert_eq!(mono(&[1.0, 1.0, 1.0, 0.0, 0.0, 0.0], 3), [1.0, 0.0]);
    }

    #[test]
    fn short_gaps_are_left_alone_and_long_ones_filled() {
        let t0 = Instant::now();
        assert_eq!(silence_to_fill(t0, t0 + Duration::from_millis(30), 48_000), 0);
        assert_eq!(silence_to_fill(t0, t0 + Duration::from_millis(100), 48_000), 4_800);
        assert_eq!(
            silence_to_fill(t0, t0 + Duration::from_secs(60), 48_000),
            48_000,
            "at most a second is filled at once"
        );
    }

    #[test]
    fn listing_devices_works_with_or_without_any() {
        // Lists only; opening a stream in a test would grab real hardware.
        let _ = input_devices();
        let _ = output_devices();
    }
}
```

`src/audio/output.rs` test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn shared(frames: i16) -> Shared {
        let samples: Vec<i16> = (0..frames).flat_map(|i| [i * 100, -(i * 100)]).collect();
        Shared::new(samples.into())
    }

    #[test]
    fn playing_copies_samples_and_moves_on() {
        let s = shared(4);
        s.follow(0.0, true, false, 1.0);
        let mut out = [9.0; 4];
        s.fill(&mut out);
        assert_eq!(out, [0.0, 0.0, 100.0 / 32768.0, -100.0 / 32768.0]);
        assert_eq!(s.position(), 2);
    }

    #[test]
    fn stopped_volume_and_the_end_are_silent() {
        let s = shared(4);
        let mut out = [9.0; 4];
        s.follow(0.0, false, false, 1.0);
        s.fill(&mut out);
        assert_eq!(out, [0.0; 4], "not playing");
        s.follow(0.0, true, false, 0.0);
        s.fill(&mut out);
        assert_eq!(out, [0.0; 4], "volume 0");
        s.follow(3.0 / 48_000.0, true, false, 1.0);
        let mut out = [9.0; 6];
        s.fill(&mut out);
        assert_eq!(out[2..], [0.0; 4], "silence after the last frame");
    }

    #[test]
    fn looping_wraps_to_the_start() {
        let s = shared(4);
        s.follow(3.0 / 48_000.0, true, true, 1.0);
        let mut out = [9.0; 4];
        s.fill(&mut out);
        assert_eq!(out, [300.0 / 32768.0, -300.0 / 32768.0, 0.0, 0.0]);
    }

    #[test]
    fn small_drift_is_left_and_large_drift_jumps() {
        let s = Shared::new(vec![0i16; 48_000 * 2].into());
        s.follow(0.0, true, false, 1.0);
        let mut out = vec![0.0; 960 * 2];
        s.fill(&mut out); // the speakers are 20 ms in
        s.follow(0.0, true, false, 1.0);
        assert_eq!(s.position(), 960, "20 ms off is within SYNC: left alone");
        s.follow(0.5, true, false, 1.0);
        assert_eq!(s.position(), 24_000, "a scrub jumps");
    }
}
```

Add `pub mod input;` and `pub mod output;` to `src/audio/mod.rs`, and `cpal = "0.18"` to `Cargo.toml`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib audio::input audio::output`
Expected: compile errors.

- [ ] **Step 3: Write `src/audio/input.rs` above its tests**

```rust
//! Live sound through cpal (WASAPI on Windows): an input device (microphone, line-in),
//! or an output device's loopback ("what the PC is playing"). The stream's callback
//! mixes to mono and hands blocks over a channel; the app analyses them.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};

/// Silence longer than this is filled with zeros (loopback sends nothing while the PC
/// plays nothing).
const GAP: Duration = Duration::from_millis(50);
/// At most this much silence is filled at once.
const MAX_FILL: Duration = Duration::from_secs(1);

/// Which kind of live source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveKind {
    /// An input device.
    Input,
    /// An output device's loopback.
    Loopback,
}

fn names(devices: Result<impl Iterator<Item = cpal::Device>, cpal::Error>) -> Result<Vec<String>, String> {
    devices
        .map(|list| list.map(|d| d.to_string()).collect())
        .map_err(|err| err.to_string())
}

/// The input devices' names.
pub fn input_devices() -> Result<Vec<String>, String> {
    names(cpal::default_host().input_devices())
}

/// The output devices' names (also the loopback sources).
pub fn output_devices() -> Result<Vec<String>, String> {
    names(cpal::default_host().output_devices())
}

/// A message for a device error.
pub fn describe(err: &cpal::Error) -> String {
    match err.kind() {
        cpal::ErrorKind::DeviceBusy => "Device in use".into(),
        cpal::ErrorKind::DeviceNotAvailable => "Device gone".into(),
        _ => err.to_string(),
    }
}

/// `data` (interleaved, `channels` per frame) averaged to one channel.
pub fn mono(data: &[f32], channels: usize) -> Vec<f32> {
    let channels = channels.max(1);
    data.chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// How many zeros to feed for the silence since `last`: none for a gap shorter than
/// 50 ms (a late block), else the gap's length, at most a second.
pub fn silence_to_fill(last: Instant, now: Instant, rate: u32) -> usize {
    let gap = now.saturating_duration_since(last);
    if gap < GAP {
        return 0;
    }
    (gap.min(MAX_FILL).as_secs_f64() * f64::from(rate)).round() as usize
}

/// An open live stream.
pub struct Live {
    _stream: cpal::Stream,
    rate: u32,
    blocks: Receiver<Vec<f32>>,
    failure: Arc<Mutex<Option<String>>>,
    /// When samples (or filled silence) last came.
    last: Instant,
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    send: mpsc::Sender<Vec<f32>>,
    failure: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels);
    device.build_input_stream::<T, _, _>(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let floats: Vec<f32> = data.iter().map(|s| s.to_sample::<f32>()).collect();
            let _ = send.send(mono(&floats, channels));
        },
        move |err: cpal::Error| {
            if err.kind() != cpal::ErrorKind::Xrun {
                if let Ok(mut slot) = failure.lock() {
                    *slot = Some(describe(&err));
                }
            }
        },
        None,
    )
}

impl Live {
    /// Opens the device called `name` and starts listening.
    pub fn open(kind: LiveKind, name: &str) -> Result<Live> {
        let host = cpal::default_host();
        let devices = match kind {
            LiveKind::Input => host.input_devices(),
            LiveKind::Loopback => host.output_devices(),
        }
        .map_err(|err| anyhow!(describe(&err)))?;
        let device = devices
            .into_iter()
            .find(|d| d.to_string() == name)
            .ok_or_else(|| anyhow!("{name} isn't connected"))?;
        let supported = match kind {
            LiveKind::Input => device.default_input_config(),
            LiveKind::Loopback => device.default_output_config(),
        }
        .map_err(|err| anyhow!(describe(&err)))?;
        let config = supported.config();
        let rate = config.sample_rate;
        let (send, blocks) = mpsc::channel();
        let failure = Arc::new(Mutex::new(None));
        let f = failure.clone();
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config, send, f),
            SampleFormat::I16 => build::<i16>(&device, config, send, f),
            SampleFormat::I32 => build::<i32>(&device, config, send, f),
            SampleFormat::U8 => build::<u8>(&device, config, send, f),
            other => Err(cpal::Error::with_message(
                cpal::ErrorKind::UnsupportedConfig,
                format!("unsupported sample format {other:?}"),
            )),
        }
        .map_err(|err| match kind {
            LiveKind::Loopback => anyhow!("This device can't loop back what it plays: {}", describe(&err)),
            LiveKind::Input => anyhow!(describe(&err)),
        })?;
        stream.play().map_err(|err| anyhow!(describe(&err)))?;
        Ok(Live {
            _stream: stream,
            rate,
            blocks,
            failure,
            last: Instant::now(),
        })
    }

    /// The device's rate: the rate [`Live::drain`]'s samples are at.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Mono samples since the last call; when nothing came for more than 50 ms, zeros
    /// for the silence.
    pub fn drain(&mut self, now: Instant) -> Vec<f32> {
        let mut out: Vec<f32> = Vec::new();
        for block in self.blocks.try_iter() {
            out.extend(block);
        }
        if !out.is_empty() {
            self.last = now;
            return out;
        }
        let fill = silence_to_fill(self.last, now, self.rate);
        if fill > 0 {
            self.last = now;
        }
        vec![0.0; fill]
    }

    /// Why the stream stopped, if it did (the device was unplugged, say).
    pub fn failure(&self) -> Option<String> {
        self.failure.lock().ok().and_then(|slot| slot.clone())
    }
}
```

- [ ] **Step 4: Write `src/audio/output.rs` above its tests**

```rust
//! Playing a sound file on the speakers. The callback reads from the decoded samples at
//! its own position; each canvas frame the engine tells it where the playhead is, and
//! it jumps there when it has drifted more than [`SYNC`].

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::RATE;
use super::input::describe;

/// How far, in seconds, the speakers may drift from the playhead before they jump.
pub const SYNC: f64 = 0.04;

/// What the speaker callback reads and the engine sets.
pub struct Shared {
    /// Interleaved stereo at [`RATE`].
    samples: Arc<[i16]>,
    /// The next stereo frame to play.
    position: AtomicU64,
    playing: AtomicBool,
    looping: AtomicBool,
    /// The volume's f32 bits.
    volume: AtomicU32,
    failure: Mutex<Option<String>>,
}

impl Shared {
    /// Stopped, at the start, at full volume.
    pub fn new(samples: Arc<[i16]>) -> Shared {
        Shared {
            samples,
            position: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            looping: AtomicBool::new(false),
            volume: AtomicU32::new(1.0f32.to_bits()),
            failure: Mutex::new(None),
        }
    }

    /// The next stereo frame the speakers will play.
    pub fn position(&self) -> u64 {
        self.position.load(Ordering::Relaxed)
    }

    /// Follows the playhead at `seconds`: jumps there if the speakers are more than
    /// [`SYNC`] away, and takes the playing state, looping and volume.
    pub fn follow(&self, seconds: f64, playing: bool, looping: bool, volume: f32) {
        let wanted = (seconds.max(0.0) * f64::from(RATE)).round() as u64;
        let now = self.position();
        if now.abs_diff(wanted) as f64 > SYNC * f64::from(RATE) {
            self.position.store(wanted, Ordering::Relaxed);
        }
        self.playing.store(playing, Ordering::Relaxed);
        self.looping.store(looping, Ordering::Relaxed);
        self.volume.store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Fills `out` (interleaved stereo f32) with what plays next.
    pub fn fill(&self, out: &mut [f32]) {
        out.fill(0.0);
        if !self.playing.load(Ordering::Relaxed) {
            return;
        }
        let volume = f32::from_bits(self.volume.load(Ordering::Relaxed)) / 32768.0;
        let looping = self.looping.load(Ordering::Relaxed);
        let total = (self.samples.len() / 2) as u64;
        let mut at = self.position();
        for frame in out.chunks_exact_mut(2) {
            if at >= total {
                if looping && total > 0 {
                    at = 0;
                } else {
                    break;
                }
            }
            let i = at as usize * 2;
            frame[0] = f32::from(self.samples[i]) * volume;
            frame[1] = f32::from(self.samples[i + 1]) * volume;
            at += 1;
        }
        self.position.store(at, Ordering::Relaxed);
    }
}

/// A sound file playing on an output device.
pub struct Speakers {
    _stream: cpal::Stream,
    shared: Arc<Shared>,
}

impl Speakers {
    /// Opens the output device called `device` (`""` for the system default) to play
    /// `samples`. It starts stopped; [`Speakers::follow`] sets it going.
    pub fn open(device: &str, samples: Arc<[i16]>) -> Result<Speakers> {
        let host = cpal::default_host();
        let found = if device.is_empty() {
            host.default_output_device()
        } else {
            host.output_devices()
                .map_err(|err| anyhow!(describe(&err)))?
                .find(|d| d.to_string() == device)
        };
        let found = found.ok_or_else(|| anyhow!("no output device"))?;
        let shared = Arc::new(Shared::new(samples));
        let (reader, failure) = (shared.clone(), shared.clone());
        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: RATE,
            buffer_size: cpal::BufferSize::Default,
        };
        let stream = found
            .build_output_stream::<f32, _, _>(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| reader.fill(data),
                move |err: cpal::Error| {
                    if err.kind() != cpal::ErrorKind::Xrun {
                        if let Ok(mut slot) = failure.failure.lock() {
                            *slot = Some(describe(&err));
                        }
                    }
                },
                None,
            )
            .map_err(|err| anyhow!(describe(&err)))?;
        stream.play().map_err(|err| anyhow!(describe(&err)))?;
        Ok(Speakers {
            _stream: stream,
            shared,
        })
    }

    /// See [`Shared::follow`].
    pub fn follow(&self, seconds: f64, playing: bool, looping: bool, volume: f32) {
        self.shared.follow(seconds, playing, looping, volume);
    }

    /// Why the speakers stopped, if they did.
    pub fn failure(&self) -> Option<String> {
        self.shared.failure.lock().ok().and_then(|slot| slot.clone())
    }
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib audio::`
Expected: all pass. (No test opens a stream.)

- [ ] **Step 6: Check by hand that streams open (optional, quick)**

Not a test: a scratch `cargo run --example` is not needed — Task 8's app wiring exercises `Live::open` and `Speakers::open` by hand. Skip unless a compile question needs it.

- [ ] **Step 7: Check and commit**

Run: `cargo fmt`, `cargo clippy --all-targets`, `cargo test --lib`, `cargo test --no-run`.

```bash
git add Cargo.toml Cargo.lock src/audio
git commit -m "feat: live audio input and loopback, and speakers that follow a playhead"
```

---

### Task 6: The audio engine

**Files:**
- Create: `src/audio/engine.rs`
- Modify: `src/audio/mod.rs` (`mod engine;` and `pub use engine::{Audio, AudioFrame, FileState, SoundSpan, SourceChoice};`)

**Interfaces:**
- Consumes: `Analyzer`, `gained` (Task 1); `Sound`, `Loading`, `Reanalysis` (Task 4); `Live`, `LiveKind`, `Speakers` (Task 5); `Pulse`, `Shaping`, `Signals`, `Signal`, `Beat`.
- Produces (`crate::audio`):
  - `pub enum SourceChoice { None, Input(String), Loopback(String), File(PathBuf) }` (`Clone, Debug, Default, PartialEq`) with `saved(&self) -> String` (`"none"`, `"input:<name>"`, `"loopback:<name>"`, `"file:<path>"`) and `from_saved(&str) -> Option<SourceChoice>`.
  - `pub struct SoundSpan { pub from: f64, pub playing: bool }` and `pub struct AudioFrame { pub signals: Signals, pub sound: Option<SoundSpan> }`.
  - `pub struct FileState { pub name: String, pub position: f64, pub length: f64, pub playing: bool }`.
  - `pub struct Audio` (`Default`) with:
    - `new() -> Audio` (no speakers until `enable_speakers(true)`);
    - `use_choice(&mut self, choice: SourceChoice, budget: &Budget)`, `choice(&self) -> &SourceChoice`, `use_sound(&mut self, sound: Sound)`, `retry(&mut self, budget: &Budget)` (reopen the chosen device or file after a failure);
    - `poll(&mut self, now: Instant) -> Option<Result<String, String>>` (once per screen refresh: drains live input, finishes loads and re-analyses; `Some(Ok(name))` when a file finished loading, `Some(Err(message))` when a load failed);
    - `frame(&mut self, dt: f64) -> AudioFrame` (once per canvas frame; `dt` is 0 while paused);
    - `tap(&mut self, beat: Beat)`;
    - `set_shaping(&mut self, Shaping)`, `shaping(&self) -> Shaping`;
    - file controls `set_playing(bool)`, `restart()`, `seek(seconds: f64)`, `set_looping(bool)`, `looping(&self) -> bool`, `file(&self) -> Option<FileState>`, `ended(&self) -> bool`, `soundtrack(&self, span: SoundSpan, frames: usize) -> Vec<i16>`;
    - speakers `enable_speakers(&mut self, on: bool)`, `set_volume(f32)`, `volume(&self) -> f32`, `set_output(&mut self, name: &str)`, `output(&self) -> &str`, `set_muted(bool)`;
    - panel `loading(&self) -> Option<(String, f32)>`, `error(&self) -> Option<&str>`, `meter(&self) -> Signals`.

Behaviour (spec "Signals", "The sound file", "Errors"):
- **None:** follow signals read 0; hand beats still fire and kick Pulse.
- **Live:** `poll` drains the stream into an `Analyzer` at the stream's rate; beats found since the last canvas frame fire on the next frame; values are `gained(analyzer.values(), gain)`. Live keeps running while the app is paused (`frame(0.0)` still reads it).
- **File:** `frame(dt)` reads the tracks at the playhead *before* advancing (`SoundSpan::from`), fires beats in `[from, from + dt)` (wrapping when looping), then advances. Without Loop the playhead stops at the end and `playing` turns off. `dt == 0` (paused) neither advances nor fires file beats.
- **Hand beats:** `tap(Bass)` or `tap(Treble)` also taps `Any`. They fire on the next `frame` with any source.
- **Pulse:** stepped once per frame from that frame's `Any` beat, with `dt` and the release time; it is `values[Signal::Pulse]`.
- **Loading** keeps the previous source working until the new file is ready. A failed load sets the error line and keeps the previous source.
- **Shaping:** a change re-analyses a loaded file on a worker (one at a time; a change during a run queues one more run) and applies to live input at once; gain needs no re-run.
- **Errors:** a device that won't open, or fails while open, sets the error line; follow signals read 0. Speakers that fail to open set "No sound output: <reason>"; the file still drives links and recordings.

- [ ] **Step 1: Write the failing tests**

Create `src/audio/engine.rs` with its test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::file::Sound;
    use crate::video::{Budget, MEMORY_LIMIT};
    use std::f32::consts::TAU;

    const FRAME: f64 = 1.0 / 60.0;

    /// Interleaved stereo: a full-scale 1 kHz tone for `tone` seconds, then silence for
    /// `rest`.
    fn tone_then_rest(tone: f64, rest: f64) -> Vec<i16> {
        let on = (tone * RATE as f64) as usize;
        let off = (rest * RATE as f64) as usize;
        (0..on + off)
            .flat_map(|i| {
                let v = if i < on {
                    ((TAU * 1000.0 * i as f32 / RATE as f32).sin() * 32767.0) as i16
                } else {
                    0
                };
                [v, v]
            })
            .collect()
    }

    /// `count` 20 ms clicks, `gap` seconds apart.
    fn clicks(count: usize, gap: f64) -> Vec<i16> {
        let every = (gap * RATE as f64) as usize;
        let burst = (0.02 * RATE as f64) as usize;
        (0..count * every)
            .flat_map(|i| {
                let v = if i % every < burst {
                    ((TAU * 1000.0 * i as f32 / RATE as f32).sin() * 30000.0) as i16
                } else {
                    0
                };
                [v, v]
            })
            .collect()
    }

    fn with_sound(samples: Vec<i16>) -> Audio {
        let budget = Budget::new(MEMORY_LIMIT);
        let sound = Sound::new("test".into(), samples, Shaping::default(), &budget).unwrap();
        let mut audio = Audio::new();
        audio.use_sound(sound);
        audio
    }

    fn run(audio: &mut Audio, seconds: f64) -> (Signals, usize) {
        let mut beats = 0;
        let mut last = Signals::default();
        let mut t = 0.0;
        while t < seconds - 1e-9 {
            last = audio.frame(FRAME).signals;
            beats += usize::from(last.beat(Beat::Any));
            t += FRAME;
        }
        (last, beats)
    }

    #[test]
    fn source_names_round_trip() {
        for choice in [
            SourceChoice::None,
            SourceChoice::Input("Microphone (USB)".into()),
            SourceChoice::Loopback("Speakers: 2".into()),
            SourceChoice::File(PathBuf::from("C:/music/drums.wav")),
        ] {
            assert_eq!(SourceChoice::from_saved(&choice.saved()), Some(choice.clone()));
        }
        assert_eq!(SourceChoice::None.saved(), "none");
        assert_eq!(SourceChoice::from_saved("radio:bbc"), None);
    }

    #[test]
    fn no_source_reads_zero_but_hand_beats_pulse() {
        let mut audio = Audio::new();
        assert_eq!(audio.frame(FRAME).signals, Signals::default());
        audio.tap(Beat::Bass);
        let s = audio.frame(FRAME).signals;
        assert!(s.beat(Beat::Bass) && s.beat(Beat::Any) && !s.beat(Beat::Treble));
        assert_eq!(s.get(Signal::Pulse), 1.0);
        assert_eq!(s.get(Signal::Level), 0.0);
        let next = audio.frame(FRAME).signals;
        assert!(!next.beat(Beat::Any), "a tap fires once");
        assert!(next.get(Signal::Pulse) < 1.0, "and Pulse falls");
    }

    #[test]
    fn a_file_reads_its_tracks_at_the_playhead() {
        let mut audio = with_sound(tone_then_rest(1.0, 1.0));
        let (during, _) = run(&mut audio, 0.5);
        assert!(during.get(Signal::Level) > 0.8, "{during:?}");
        let (after, _) = run(&mut audio, 1.0);
        assert!(after.get(Signal::Level) < 0.1, "{after:?}");
        let state = audio.file().unwrap();
        assert!((state.position - 1.5).abs() < 2.0 * FRAME);
        assert_eq!(state.length, 2.0);
    }

    #[test]
    fn each_beat_fires_once_and_again_on_the_next_loop() {
        let mut audio = with_sound(clicks(4, 0.5));
        audio.set_looping(true);
        let (_, beats) = run(&mut audio, 2.0);
        assert_eq!(beats, 4);
        let (_, beats) = run(&mut audio, 2.0);
        assert_eq!(beats, 4, "the second time round");
    }

    #[test]
    fn without_loop_the_file_stops_at_its_end() {
        let mut audio = with_sound(clicks(2, 0.5));
        audio.set_looping(false);
        run(&mut audio, 1.5);
        let state = audio.file().unwrap();
        assert!(!state.playing);
        assert_eq!(state.position, 1.0);
        assert!(audio.ended());
        audio.restart();
        assert!(!audio.ended());
        assert_eq!(audio.file().unwrap().position, 0.0);
    }

    #[test]
    fn a_paused_frame_neither_moves_nor_fires() {
        let mut audio = with_sound(clicks(4, 0.5));
        let frame = audio.frame(0.0);
        assert_eq!(frame.signals.beats, [false; 3]);
        assert_eq!(audio.file().unwrap().position, 0.0);
        assert_eq!(frame.sound, Some(SoundSpan { from: 0.0, playing: true }));
    }

    #[test]
    fn the_soundtrack_comes_from_the_file_at_the_span() {
        let ramp: Vec<i16> = (0..100).flat_map(|i| [i as i16, i as i16]).collect();
        let mut audio = with_sound(ramp);
        audio.set_looping(false);
        let span = SoundSpan {
            from: 2.0 / RATE as f64,
            playing: true,
        };
        assert_eq!(audio.soundtrack(span, 2), [2, 2, 3, 3]);
        let stopped = SoundSpan { playing: false, ..span };
        assert_eq!(audio.soundtrack(stopped, 2), [0; 4]);
        assert_eq!(Audio::new().soundtrack(span, 2), [0; 4], "no file, silence");
    }

    #[test]
    fn gain_applies_without_reanalysis() {
        let mut audio = with_sound(tone_then_rest(1.0, 0.0));
        let (full, _) = run(&mut audio, 0.5);
        audio.set_shaping(Shaping {
            gain: 0.5,
            ..Shaping::default()
        });
        let half = audio.frame(FRAME).signals;
        assert!((half.get(Signal::Level) - full.get(Signal::Level) / 2.0).abs() < 0.05);
    }

    #[test]
    fn a_new_release_reanalyses_the_file() {
        let mut audio = with_sound(tone_then_rest(0.5, 1.0));
        audio.set_shaping(Shaping {
            release: 1.0,
            ..Shaping::default()
        });
        let start = Instant::now();
        while audio.reanalysis.is_some() && start.elapsed().as_secs() < 10 {
            audio.poll(Instant::now());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        audio.seek(0.8);
        let s = audio.frame(FRAME).signals;
        assert!(
            s.get(Signal::Level) > 0.5,
            "0.3 s after the tone a 1 s release still holds: {s:?}"
        );
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib audio::engine`
Expected: compile errors (`Audio` not found).

- [ ] **Step 3: Write `src/audio/engine.rs` above its tests**

```rust
//! The audio engine: one source at a time (none, a live device, or a sound file), read
//! once per canvas frame into [`Signals`], plus hand beats and Pulse.

use std::path::PathBuf;
use std::time::Instant;

use super::analysis::{Analyzer, gained};
use super::file::{Loading, Reanalysis, Sound};
use super::input::{Live, LiveKind};
use super::output::Speakers;
use super::{Beat, Pulse, RATE, Shaping, Signal, Signals};
use crate::video::Budget;

/// Where the sound comes from, as the project saves it.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum SourceChoice {
    #[default]
    None,
    /// An input device, by name.
    Input(String),
    /// An output device's loopback, by name.
    Loopback(String),
    /// A sound file.
    File(PathBuf),
}

impl SourceChoice {
    /// Its saved form: `"none"`, `"input:<name>"`, `"loopback:<name>"` or `"file:<path>"`.
    pub fn saved(&self) -> String {
        match self {
            SourceChoice::None => "none".into(),
            SourceChoice::Input(name) => format!("input:{name}"),
            SourceChoice::Loopback(name) => format!("loopback:{name}"),
            SourceChoice::File(path) => format!("file:{}", path.display()),
        }
    }

    /// The choice saved as `saved`, if this version knows the kind.
    pub fn from_saved(saved: &str) -> Option<SourceChoice> {
        if saved == "none" {
            return Some(SourceChoice::None);
        }
        let (kind, rest) = saved.split_once(':')?;
        match kind {
            "input" => Some(SourceChoice::Input(rest.into())),
            "loopback" => Some(SourceChoice::Loopback(rest.into())),
            "file" => Some(SourceChoice::File(PathBuf::from(rest))),
            _ => None,
        }
    }
}

/// Where a file's playhead was at the start of a canvas frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundSpan {
    /// Seconds into the file.
    pub from: f64,
    /// Whether it was playing (a stopped file records silence).
    pub playing: bool,
}

/// One canvas frame of sound.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioFrame {
    pub signals: Signals,
    /// Set when the source is a loaded file: what the soundtrack takes for this frame.
    pub sound: Option<SoundSpan>,
}

/// A loaded file's state, for the panel.
#[derive(Clone, Debug, PartialEq)]
pub struct FileState {
    pub name: String,
    pub position: f64,
    pub length: f64,
    pub playing: bool,
}

/// A loaded file and its playhead.
struct FileSource {
    sound: Sound,
    position: f64,
    playing: bool,
}

/// What is playing now.
enum Current {
    None,
    Live {
        live: Live,
        analyzer: Analyzer,
        /// Beats found since the last canvas frame.
        beats: [bool; 3],
    },
    File(FileSource),
}

/// The audio source, its analysis and its playback.
pub struct Audio {
    shaping: Shaping,
    current: Current,
    /// What was asked for (kept while a file loads, or a device is missing).
    choice: SourceChoice,
    loading: Option<Loading>,
    reanalysis: Option<Reanalysis>,
    /// The shaping changed while a re-analysis ran.
    reanalyse_again: bool,
    pulse: Pulse,
    /// Hand beats since the last canvas frame.
    hand: [bool; 3],
    looping: bool,
    volume: f32,
    muted: bool,
    output: String,
    speakers_on: bool,
    speakers: Option<Speakers>,
    error: Option<String>,
    meter: Signals,
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

impl Audio {
    /// No source, no speakers.
    pub fn new() -> Self {
        Self {
            shaping: Shaping::default(),
            current: Current::None,
            choice: SourceChoice::None,
            loading: None,
            reanalysis: None,
            reanalyse_again: false,
            pulse: Pulse::default(),
            hand: [false; 3],
            looping: true,
            volume: 1.0,
            muted: false,
            output: String::new(),
            speakers_on: false,
            speakers: None,
            error: None,
            meter: Signals::default(),
        }
    }

    /// What was asked for.
    pub fn choice(&self) -> &SourceChoice {
        &self.choice
    }

    /// Switches to `choice`. A file loads in the background and the current source keeps
    /// working until it is ready; a device opens at once.
    pub fn use_choice(&mut self, choice: SourceChoice, budget: &Budget) {
        self.choice = choice.clone();
        self.error = None;
        self.loading = None;
        match choice {
            SourceChoice::None => self.stop(),
            SourceChoice::Input(name) => self.open_live(LiveKind::Input, &name),
            SourceChoice::Loopback(name) => self.open_live(LiveKind::Loopback, &name),
            SourceChoice::File(path) => {
                self.loading = Some(Loading::start(&path, self.shaping, budget));
            }
        }
    }

    /// Tries the chosen device or file again (Refresh, or after a failure).
    pub fn retry(&mut self, budget: &Budget) {
        let loaded_file = matches!(self.current, Current::File(_))
            && matches!(self.choice, SourceChoice::File(_));
        if !loaded_file {
            self.use_choice(self.choice.clone(), budget);
        }
    }

    fn stop(&mut self) {
        self.current = Current::None;
        self.speakers = None;
        self.reanalysis = None;
    }

    fn open_live(&mut self, kind: LiveKind, name: &str) {
        self.stop();
        match Live::open(kind, name) {
            Ok(live) => {
                let analyzer = Analyzer::new(live.rate(), self.shaping);
                self.current = Current::Live {
                    live,
                    analyzer,
                    beats: [false; 3],
                };
            }
            Err(err) => self.error = Some(format!("{err:#}")),
        }
    }

    /// Makes `sound` the source, playing from the start.
    pub fn use_sound(&mut self, sound: Sound) {
        self.stop();
        self.current = Current::File(FileSource {
            sound,
            position: 0.0,
            playing: true,
        });
        self.open_speakers();
    }

    /// Once per screen refresh: listens to live input, and finishes loads and
    /// re-analyses. Returns the file's name when a load finished, or why it failed.
    pub fn poll(&mut self, now: Instant) -> Option<Result<String, String>> {
        if let Current::Live {
            live,
            analyzer,
            beats,
        } = &mut self.current
        {
            for x in live.drain(now) {
                for (pending, fired) in beats.iter_mut().zip(analyzer.push(x)) {
                    *pending |= fired;
                }
            }
            if let Some(why) = live.failure() {
                self.error = Some(why);
                self.current = Current::None;
            }
        }
        if let Some(why) = self.speakers.as_ref().and_then(Speakers::failure) {
            self.error = Some(format!("No sound output: {why}"));
            self.speakers = None;
        }
        if let Some(tracks) = self.reanalysis.as_ref().and_then(Reanalysis::poll) {
            self.reanalysis = None;
            if let Current::File(file) = &mut self.current {
                file.sound.tracks = tracks;
                if std::mem::take(&mut self.reanalyse_again) {
                    self.reanalysis = Some(Reanalysis::start(file.sound.samples.clone(), self.shaping));
                }
            }
        }
        let finished = self.loading.as_ref().and_then(Loading::poll)?;
        self.loading = None;
        match finished {
            Ok(sound) => {
                let name = sound.name.clone();
                self.use_sound(sound);
                Some(Ok(name))
            }
            Err(err) => {
                let why = format!("{err:#}");
                self.error = Some(why.clone());
                Some(Err(why))
            }
        }
    }

    /// One canvas frame `dt` seconds long (0 while paused): this frame's signals, and
    /// for a file, where the soundtrack starts.
    pub fn frame(&mut self, dt: f64) -> AudioFrame {
        let mut frame = AudioFrame::default();
        let mut beats = std::mem::take(&mut self.hand);
        let mut values = [0.0; 4];
        let gain = self.shaping.gain;
        match &mut self.current {
            Current::None => {}
            Current::Live {
                analyzer,
                beats: pending,
                ..
            } => {
                values = gained(analyzer.values(), gain);
                for (beat, fired) in beats.iter_mut().zip(std::mem::take(pending)) {
                    *beat |= fired;
                }
            }
            Current::File(file) => {
                let from = file.position;
                frame.sound = Some(SoundSpan {
                    from,
                    playing: file.playing,
                });
                values = gained(file.sound.tracks.at(from), gain);
                if file.playing && dt > 0.0 {
                    let length = file.sound.seconds();
                    let mut to = from + dt;
                    if to >= length {
                        if self.looping && length > 0.0 {
                            to %= length;
                        } else {
                            to = length;
                            file.playing = false;
                        }
                    }
                    let found = if to < from || to < length || self.looping {
                        file.sound.tracks.beats_in(from, to)
                    } else {
                        // Stopped at the end: everything up to and including it.
                        file.sound.tracks.beats_in(from, f64::INFINITY)
                    };
                    for (beat, fired) in beats.iter_mut().zip(found) {
                        *beat |= fired;
                    }
                    file.position = to;
                }
            }
        }
        let pulse = self
            .pulse
            .step(beats[Beat::Any.index()], dt as f32, self.shaping.release);
        frame.signals.values[..4].copy_from_slice(&values);
        frame.signals.values[Signal::Pulse.index()] = pulse;
        frame.signals.beats = beats;
        self.meter = frame.signals;
        self.sync_speakers();
        frame
    }

    /// A hand beat, fired on the next canvas frame. A Bass or Treble tap is also Any.
    pub fn tap(&mut self, beat: Beat) {
        self.hand[beat.index()] = true;
        self.hand[Beat::Any.index()] = true;
    }

    /// The shaping in use.
    pub fn shaping(&self) -> Shaping {
        self.shaping
    }

    /// Uses new shaping: at once for live input; a loaded file is re-analysed unless
    /// only the gain changed.
    pub fn set_shaping(&mut self, shaping: Shaping) {
        let shaping = shaping.clamped();
        if shaping == self.shaping {
            return;
        }
        let only_gain = Shaping {
            gain: shaping.gain,
            ..self.shaping
        } == shaping;
        self.shaping = shaping;
        match &mut self.current {
            Current::Live { analyzer, .. } => analyzer.set_shaping(shaping),
            Current::File(file) if !only_gain => {
                if self.reanalysis.is_some() {
                    self.reanalyse_again = true;
                } else {
                    self.reanalysis = Some(Reanalysis::start(file.sound.samples.clone(), shaping));
                }
            }
            _ => {}
        }
    }

    /// Plays or stops a loaded file.
    pub fn set_playing(&mut self, playing: bool) {
        if let Current::File(file) = &mut self.current {
            file.playing = playing && (self.looping || file.position < file.sound.seconds());
        }
    }

    /// Back to the start of a loaded file, playing.
    pub fn restart(&mut self) {
        if let Current::File(file) = &mut self.current {
            file.position = 0.0;
            file.playing = true;
        }
    }

    /// Moves a loaded file's playhead to `seconds`.
    pub fn seek(&mut self, seconds: f64) {
        if let Current::File(file) = &mut self.current {
            file.position = seconds.clamp(0.0, file.sound.seconds());
        }
    }

    /// Whether a file wraps to the start at its end.
    pub fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    /// Whether a file wraps to the start at its end.
    pub fn looping(&self) -> bool {
        self.looping
    }

    /// The loaded file's state, if a file is the source.
    pub fn file(&self) -> Option<FileState> {
        match &self.current {
            Current::File(file) => Some(FileState {
                name: file.sound.name.clone(),
                position: file.position,
                length: file.sound.seconds(),
                playing: file.playing,
            }),
            _ => None,
        }
    }

    /// Whether a non-looping file has played to its end.
    pub fn ended(&self) -> bool {
        match &self.current {
            Current::File(file) => {
                !self.looping && !file.playing && file.position >= file.sound.seconds()
            }
            _ => false,
        }
    }

    /// The soundtrack for a frame: `frames` stereo frames of the file from `span`, or
    /// silence when it wasn't playing (or no file is loaded).
    pub fn soundtrack(&self, span: SoundSpan, frames: usize) -> Vec<i16> {
        match &self.current {
            Current::File(file) if span.playing => file.sound.slice(span.from, frames, self.looping),
            _ => vec![0; frames * 2],
        }
    }

    /// Lets the engine open speakers (the app does; tests don't).
    pub fn enable_speakers(&mut self, on: bool) {
        self.speakers_on = on;
        if on {
            self.open_speakers();
        } else {
            self.speakers = None;
        }
    }

    fn open_speakers(&mut self) {
        self.speakers = None;
        let Current::File(file) = &self.current else {
            return;
        };
        if !self.speakers_on {
            return;
        }
        match Speakers::open(&self.output, file.sound.samples.clone()) {
            Ok(speakers) => self.speakers = Some(speakers),
            Err(err) => self.error = Some(format!("No sound output: {err:#}")),
        }
        self.sync_speakers();
    }

    fn sync_speakers(&self) {
        if let (Some(speakers), Current::File(file)) = (&self.speakers, &self.current) {
            speakers.follow(
                file.position,
                file.playing && !self.muted,
                self.looping,
                self.volume,
            );
        }
    }

    /// The speakers' volume, 0 to 1.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
    }

    /// The speakers' volume, 0 to 1.
    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Plays on the output device called `name` (`""` for the system default).
    pub fn set_output(&mut self, name: &str) {
        if name != self.output {
            self.output = name.to_string();
            self.open_speakers();
        }
    }

    /// The output device's name (`""` for the system default).
    pub fn output(&self) -> &str {
        &self.output
    }

    /// Silences the speakers (during a frame-by-frame recording) without stopping the
    /// file.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// The file loading and how far it has got, if one is.
    pub fn loading(&self) -> Option<(String, f32)> {
        self.loading.as_ref().map(|l| {
            let name = l
                .path
                .file_name()
                .map_or_else(|| l.path.display().to_string(), |n| n.to_string_lossy().into_owned());
            (name, l.progress())
        })
    }

    /// Why the source or the speakers aren't working, if they aren't.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The last frame's signals, for the meter.
    pub fn meter(&self) -> Signals {
        self.meter
    }
}
```

Notes for the implementer:
- `RATE` is used by the tests; if clippy flags it unused in the non-test code, import it inside the test module instead.
- In `frame`, the `found` logic: a span that ends exactly at the end of a non-looping file must still include an onset right at its last sample; `beats_in(from, INFINITY)` covers that. A looping span that wrapped (`to < from`) uses the wrapped form of `beats_in`.
- The test `a_new_release_reanalyses_the_file` reads the private field `reanalysis`; it is in the same module, so that's allowed.

In `src/audio/mod.rs` add:

```rust
mod engine;

pub use engine::{Audio, AudioFrame, FileState, SoundSpan, SourceChoice};
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib audio::`
Expected: all pass.

- [ ] **Step 5: Check and commit**

Run: `cargo fmt`, `cargo clippy --all-targets`, `cargo test --lib`, `cargo test --no-run`.

```bash
git add src/audio
git commit -m "feat: the audio engine: sources, playhead, hand beats and Pulse"
```

---
### Task 7: Saving audio in the project

**Files:**
- Modify: `src/session.rs` (`AudioProject`; `Project.audio`; `Default`, `checked()`; tests)

**Interfaces:**
- Consumes: `SourceChoice` (Task 6), `Shaping` (Task 1), `AudioLink`, `BeatLink`, `saved_follow`, `saved_beats` (Task 2).
- Produces (`crate::session`):
  - `pub struct AudioProject { pub source: SourceChoice, pub shaping: Shaping, pub volume: f32, pub looping: bool, pub start_with_recording: bool, pub output: String, pub follow: Vec<AudioLink>, pub beats: Vec<BeatLink> }` (`Clone, Debug, PartialEq, Serialize, Deserialize`, manual `Default`: source None, default shaping, volume 1, looping true, start_with_recording true, output `""`, no links) with `checked(&self) -> AudioProject`.
  - `Project.audio: AudioProject`. `Project::capture` leaves it at its default (the app fills it, Task 8); `Project::checked` keeps it (checked).

Saved shape (spec "Saving"): one `"audio"` object with keys `source`, `gain`, `attack`, `release`, `sensitivity`, `volume`, `loop`, `start_with_recording`, `output`, `follow`, `beats`.

- [ ] **Step 1: Write the failing tests**

Add to `src/session.rs`'s test module:

```rust
    fn with_audio() -> Project {
        use crate::audio::{Beat, Signal, SourceChoice};
        use crate::audio_links::{AudioLink, BeatLink};
        use crate::control::{Action, Target};
        use crate::params::table::SliderId;
        let mut p = project();
        p.audio = AudioProject {
            source: SourceChoice::File("C:/music/drums.wav".into()),
            volume: 0.5,
            looping: false,
            follow: vec![AudioLink {
                signal: Signal::Bass,
                slider: SliderId::Zoom,
                depth: 0.5,
            }],
            beats: vec![BeatLink {
                beat: Beat::Bass,
                target: Target::Action(Action::Cut),
            }],
            ..AudioProject::default()
        };
        p
    }

    #[test]
    fn audio_is_saved_in_one_block() {
        let value = serde_json::to_value(&with_audio()).unwrap();
        let audio = &value["audio"];
        assert_eq!(audio["source"], "file:C:/music/drums.wav");
        assert_eq!(audio["gain"], 1.0);
        assert_eq!(audio["loop"], false);
        assert_eq!(audio["start_with_recording"], true);
        assert_eq!(audio["output"], "");
        assert_eq!(audio["follow"][0]["target"], "slider:warp.zoom");
        assert_eq!(audio["beats"][0]["target"], "action:cut");
        let back: Project = serde_json::from_value(value).unwrap();
        assert_eq!(back, with_audio());
    }

    #[test]
    fn a_project_without_audio_has_none() {
        let p = project_with(|v| {
            v.as_object_mut().unwrap().remove("audio");
        });
        assert_eq!(p.audio, AudioProject::default());
    }

    #[test]
    fn unknown_audio_entries_are_dropped_and_the_rest_load() {
        let p = project_with(|v| {
            let audio = serde_json::to_value(&with_audio().audio).unwrap();
            v["audio"] = audio;
            v["audio"]["source"] = "radio:bbc".into();
            v["audio"]["gain"] = 1000.0.into();
            v["audio"]["follow"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({ "signal": "sub-bass", "target": "slider:warp.zoom" }));
        });
        assert_eq!(p.audio.source, crate::audio::SourceChoice::None);
        assert_eq!(p.audio.follow.len(), 1);
        assert_eq!(p.audio.beats.len(), 1);
        assert_eq!(p.audio.checked().shaping.gain, *crate::audio::GAIN.end());
    }

    #[test]
    fn checking_keeps_the_audio() {
        let p = with_audio();
        assert_eq!(p.checked().audio, p.audio.checked());
        assert_eq!(p.checked().checked(), p.checked());
    }
```

(`project()` and `project_with()` are the module's existing helpers; `project_with` edits the saved JSON and reads it back through `read_project`.)

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib session`
Expected: compile errors (`AudioProject`, `Project.audio` not found).

- [ ] **Step 3: Add `AudioProject`**

In `src/session.rs` add imports:

```rust
use crate::audio::{Shaping, SourceChoice};
use crate::audio_links::{AudioLink, BeatLink};
```

and the type (near `CameraChoice`):

```rust
/// The project's audio: the source, its shaping and playback, and the audio links.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioProject {
    #[serde(with = "saved_source")]
    pub source: SourceChoice,
    #[serde(flatten)]
    pub shaping: Shaping,
    /// The speakers' volume, 0 to 1.
    pub volume: f32,
    /// Whether a sound file wraps to the start at its end.
    #[serde(rename = "loop")]
    pub looping: bool,
    /// Whether Record restarts the sound file from its start.
    pub start_with_recording: bool,
    /// The output device's name; empty for the system default.
    pub output: String,
    #[serde(with = "crate::audio_links::saved_follow")]
    pub follow: Vec<AudioLink>,
    #[serde(with = "crate::audio_links::saved_beats")]
    pub beats: Vec<BeatLink>,
}

impl Default for AudioProject {
    fn default() -> Self {
        Self {
            source: SourceChoice::None,
            shaping: Shaping::default(),
            volume: 1.0,
            looping: true,
            start_with_recording: true,
            output: String::new(),
            follow: Vec::new(),
            beats: Vec::new(),
        }
    }
}

impl AudioProject {
    /// Every setting held in range (a file could hold anything).
    pub fn checked(&self) -> AudioProject {
        AudioProject {
            shaping: self.shaping.clamped(),
            volume: if self.volume.is_finite() {
                self.volume.clamp(0.0, 1.0)
            } else {
                1.0
            },
            ..self.clone()
        }
    }
}

/// Saving the audio source as one string (see [`SourceChoice::saved`]); one this version
/// doesn't know reads as no source.
mod saved_source {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::audio::SourceChoice;

    pub fn serialize<S: Serializer>(source: &SourceChoice, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&source.saved())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SourceChoice, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        let source = value.as_str().and_then(SourceChoice::from_saved);
        if source.is_none() {
            log::warn!("ignoring an audio source this version doesn't know: {value}");
        }
        Ok(source.unwrap_or_default())
    }
}
```

Add the field to `Project` (after `inputs`):

```rust
    /// The sound source, its settings and the audio links.
    pub audio: AudioProject,
```

In `impl Default for Project` add `audio: AudioProject::default(),`; in `Project::capture`'s struct literal add `audio: AudioProject::default(),` (the app fills it in; see Task 8); and make `checked` keep it:

```rust
    pub fn checked(&self) -> Self {
        let rate = ...unchanged...;
        Self {
            audio: self.audio.checked(),
            ..Self::capture(
                &self.motion(),
                canvas::sanitize(self.canvas, u32::MAX),
                rate,
                &self.inputs.checked(),
            )
        }
    }
```

Note: `#[serde(flatten)]` on `Project.inputs` already exists; `audio` is an ordinary nested field, so it's saved as the `"audio"` object. If serde rejects `#[serde(flatten)]` of `shaping` combined with the `with` fields, keep the flatten (the spec's JSON is flat) and check that `Shaping` has `#[serde(default)]` (Task 1 gave it that).

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib session`
Expected: all pass, including the existing round-trip, old-project (`missing_fields_take_their_defaults`, `the_version_1_project_still_loads`) and autosave tests.

- [ ] **Step 5: Check and commit**

Run: `cargo fmt`, `cargo clippy --all-targets`, `cargo test --lib`, `cargo test --no-run`.

```bash
git add src/session.rs
git commit -m "feat: projects save their audio source, settings and links"
```

---

### Task 8: The app runs the audio, and hand-beat actions

**Files:**
- Modify: `src/control.rs` (three beat actions; `Action::beat`)
- Modify: `src/audio_links.rs` (beat links never target a beat action)
- Create: `src/audio_ui.rs` (`AudioUi` state; the section comes in Task 10)
- Modify: `src/lib.rs` (`pub mod audio_ui;` after `pub mod audio_links;`)
- Modify: `src/ui.rs` (`UiState.audio: AudioUi`)
- Modify: `src/app.rs` (owns `Audio`; polls it each refresh; per canvas frame reads it, fires beat links, sets offsets; runs beat actions; saves and loads it with the project)

**Interfaces:**
- Consumes: `Audio`, `AudioFrame`, `SourceChoice`, `Beat` (Tasks 1, 6); `AudioLinks::{offsets, fired}`, `control::fire` (Task 2); `Motion::set_offsets` (Task 3); `AudioProject` (Task 7).
- Produces:
  - `Action::{Beat, BassBeat, TrebleBeat}` (names `beat`, `bass-beat`, `treble-beat`; labels "Beat", "Bass beat", "Treble beat"), appended to `Action::ALL` (now 13), and `Action::beat(self) -> Option<crate::audio::Beat>`.
  - `pub struct crate::audio_ui::AudioUi { pub start_with_recording: bool }` (`Clone, Debug`, `Default` with `start_with_recording: true`) — Task 10 adds the rest.
  - `UiState.audio: AudioUi`.
  - In `app.rs` (private): `State.audio: Audio`; `fn poll_audio(&mut self)`; `fn audio_project(&self) -> AudioProject`; `fn apply_audio(&mut self, audio: &AudioProject)`.

- [ ] **Step 1: Write the failing tests**

In `src/control.rs`'s tests, extend `action_names_never_change`'s expected list with `"beat", "bass-beat", "treble-beat",` after `"pause",`, and add:

```rust
    #[test]
    fn beat_actions_name_their_beat() {
        use crate::audio::Beat;
        assert_eq!(Action::Beat.beat(), Some(Beat::Any));
        assert_eq!(Action::BassBeat.beat(), Some(Beat::Bass));
        assert_eq!(Action::TrebleBeat.beat(), Some(Beat::Treble));
        assert_eq!(Action::Cut.beat(), None);
        assert_eq!(
            Target::from_saved("action:bass-beat"),
            Some(Target::Action(Action::BassBeat))
        );
    }
```

In `src/audio_links.rs`'s tests add:

```rust
    #[test]
    fn a_beat_never_fires_a_beat_action() {
        // A beat tapping a beat would fire itself every frame.
        let mut links = AudioLinks::default();
        links.fire_on(Beat::Any, Target::Action(Action::Beat));
        links.fire_on(Beat::Bass, Target::Action(Action::TrebleBeat));
        assert!(links.beats.is_empty());
        let read: Holder = serde_json::from_value(json!({
            "follow": [],
            "beats": [{ "beat": "any", "target": "action:beat" }],
        }))
        .unwrap();
        assert!(read.beats.is_empty());
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib control audio_links`
Expected: compile errors (`Action::Beat` not found).

- [ ] **Step 3: Add the beat actions**

In `src/control.rs`'s `Action` enum, after `Pause`:

```rust
    /// A hand beat, as if the sound had one.
    Beat,
    /// A hand bass beat (also an Any beat).
    BassBeat,
    /// A hand treble beat (also an Any beat).
    TrebleBeat,
```

`ALL` becomes `[Action; 13]` with the three appended in that order. In `name`: `Action::Beat => "beat"`, `Action::BassBeat => "bass-beat"`, `Action::TrebleBeat => "treble-beat"`. In `label`: `"Beat"`, `"Bass beat"`, `"Treble beat"`. Add:

```rust
    /// The beat a hand-beat action taps, if it is one.
    pub fn beat(self) -> Option<crate::audio::Beat> {
        use crate::audio::Beat;
        match self {
            Action::Beat => Some(Beat::Any),
            Action::BassBeat => Some(Beat::Bass),
            Action::TrebleBeat => Some(Beat::Treble),
            _ => None,
        }
    }
```

In `src/audio_links.rs`, make `fire_on` refuse beat actions:

```rust
    pub fn fire_on(&mut self, beat: Beat, target: Target) {
        if !fires(target) {
            return;
        }
        ...
    }
```

with, at module level:

```rust
/// Whether a beat may fire `target`: anything a key fires, except the hand-beat actions
/// (a beat tapping a beat would fire itself every frame).
pub fn fires(target: Target) -> bool {
    !target.continuous() && !matches!(target, Target::Action(a) if a.beat().is_some())
}
```

and in `saved_beats::read` replace `(!target.continuous()).then_some(...)` with `super::fires(target).then_some(BeatLink { beat, target })`. Update `fire_on`'s doc comment to say beat actions are ignored too.

- [ ] **Step 4: Add `AudioUi`**

Create `src/audio_ui.rs`:

```rust
//! The panel's Audio section: the sound source, its settings, a meter, hand-beat
//! buttons and the audio links. It only reports what was asked for; the app does it.

/// What the Audio section shows and edits, kept between frames.
#[derive(Clone, Debug)]
pub struct AudioUi {
    /// Whether Record restarts the sound file from its start.
    pub start_with_recording: bool,
}

impl Default for AudioUi {
    fn default() -> Self {
        Self {
            start_with_recording: true,
        }
    }
}
```

Add `pub mod audio_ui;` to `src/lib.rs`, and to `UiState` in `src/ui.rs`:

```rust
    /// The sound source's state and settings.
    pub audio: AudioUi,
```

with `use crate::audio_ui::AudioUi;`.

- [ ] **Step 5: Wire the app**

In `src/app.rs`:

1. Imports: `use crate::audio::Audio;`, `use crate::audio_links::AudioLinks;`, `use crate::control::{self, Action, Links};` (keep what's there), `use crate::session::AudioProject;` (with the other session imports).

2. `State` gains:

```rust
    /// The sound source, its analysis and its speakers.
    audio: Audio,
```

and `State::new` builds it with speakers on:

```rust
            audio: {
                let mut audio = Audio::new();
                audio.enable_speakers(true);
                audio
            },
```

3. Saving and loading. Change `fn project(&self)` to fill the audio block:

```rust
    fn project(&self) -> Project {
        Project {
            audio: self.audio_project(),
            ..Project::capture(&self.motion, self.ui.canvas.current, self.ui.rate, &self.inputs)
        }
    }

    /// The audio as the project saves it.
    fn audio_project(&self) -> AudioProject {
        let links = &self.ui.midi.links.audio;
        AudioProject {
            source: self.audio.choice().clone(),
            shaping: self.audio.shaping(),
            volume: self.audio.volume(),
            looping: self.audio.looping(),
            start_with_recording: self.ui.audio.start_with_recording,
            output: self.audio.output().to_string(),
            follow: links.follow.clone(),
            beats: links.beats.clone(),
        }
    }

    /// Takes a project's audio: its settings, its links, and its source (a file loads in
    /// the background; a missing file or device shows in the Audio section).
    fn apply_audio(&mut self, audio: &AudioProject) {
        let audio = audio.checked();
        self.audio.set_shaping(audio.shaping);
        self.audio.set_volume(audio.volume);
        self.audio.set_looping(audio.looping);
        self.audio.set_output(&audio.output);
        self.ui.audio.start_with_recording = audio.start_with_recording;
        self.ui.midi.links.audio = AudioLinks {
            follow: audio.follow.clone(),
            beats: audio.beats.clone(),
        };
        if *self.audio.choice() != audio.source {
            self.audio.use_choice(audio.source, &self.budget);
        }
    }
```

(Keep the argument order `Project::capture` really has.) In `apply_project`, call `self.apply_audio(&project.audio);` after the inputs are applied.

4. Each screen refresh: add a method and call it at the top of `redraw`, next to `self.handle_midi();` (before the early returns, so live input is drained while minimised):

```rust
    /// Listens to live input and finishes sound-file loads; mutes the speakers during a
    /// frame-by-frame recording.
    fn poll_audio(&mut self) {
        if let Some(Err(why)) = self.audio.poll(Instant::now()) {
            log::warn!("{why}");
        }
        let offline = self
            .recorder
            .as_ref()
            .is_some_and(|r| r.mode() == CaptureMode::Offline);
        self.audio.set_muted(offline);
    }
```

5. Each canvas frame: at the top of `draw_canvas_frame`, before the existing `if !paused { ... }` block:

```rust
        let paused = self.ui.paused;
        let period = self.clock.rate().period();
        let audio_frame = self.audio.frame(if paused { 0.0 } else { period });
        let links = &self.ui.midi.links.audio;
        let fired = links.fired(&audio_frame.signals);
        self.motion.set_offsets(links.offsets(&audio_frame.signals));
        for target in fired {
            if let Some(action) = control::fire(target, &mut self.motion) {
                self.run_action(action);
            }
        }
```

(`paused` is the variable the function already declares first — keep one declaration.) `audio_frame` is used again by Tasks 9 and 10; until then, if the compiler warns it's unused past this block, that's expected only for its `sound` field — don't prefix the variable with `_`.

6. `run_action` gets the hand beats:

```rust
            Action::Beat | Action::BassBeat | Action::TrebleBeat => {
                if let Some(beat) = action.beat() {
                    self.audio.tap(beat);
                }
            }
```

- [ ] **Step 6: Run the tests**

Run: `cargo test --lib`, `cargo test --test smoke`
Expected: all pass.

- [ ] **Step 7: Check by hand**

Run the app with scratch settings so nothing of the user's is touched (PowerShell):

```powershell
$env:APPDATA = "$env:TEMP\rasterwarp-audio-check"; cargo run --release
```

There's no Audio section yet. Check that the app starts, draws and closes normally, that a project saved now contains an `"audio"` block (Save as…, open the file in an editor), and that the log shows no audio errors. Close the app; leave nothing running.

- [ ] **Step 8: Check and commit**

Run: `cargo fmt`, `cargo clippy --all-targets`.

```bash
git add src/control.rs src/audio_links.rs src/audio_ui.rs src/lib.rs src/ui.rs src/app.rs
git commit -m "feat: the app runs audio links each frame; hand-beat actions"
```

---

### Task 9: The soundtrack in recordings

**Files:**
- Modify: `src/capture/encode.rs` (`Encoder::start_with_sound`, `Encoder::send_sound`; the sound stream in `Sink`)
- Modify: `src/capture/recorder.rs` (`Recorder::start_with_sound`, `has_sound`, `sound_frames`, `add_sound`)
- Modify: `src/app.rs` (start with sound; add each frame's soundtrack; restart the file; stop at its end)
- Modify: `tests/capture.rs` (soundtrack test)

**Interfaces:**
- Consumes: `Audio::{file, restart, ended, soundtrack}`, `AudioFrame::sound` (Task 6); `AudioUi.start_with_recording` (Task 8).
- Produces:
  - `Encoder::start_with_sound(path: &Path, format: VideoFormat, size: (u32, u32), rate: FrameRate, sound: bool) -> Result<Encoder>`; `Encoder::start` calls it with `false`.
  - `Encoder::send_sound(&self, samples: Vec<i16>) -> Result<()>` (interleaved stereo at 48 kHz; never dropped; an error if the encoder was started without sound).
  - `Recorder::start_with_sound(device, settings, canvas, rate, sound: bool) -> Result<Recorder>`; `Recorder::start` calls it with `false`.
  - `Recorder::has_sound(&self) -> bool`, `Recorder::sound_frames(&self) -> usize` (stereo frames the frame just captured covers), `Recorder::add_sound(&mut self, samples: Vec<i16>) -> Result<()>`.

FFmpeg facts (verified on this machine with ffmpeg-next 9 and FFmpeg 9.0.2):
- Encoders: `encoder::find_by_name("aac")` takes only `Sample::F32(Type::Planar)` and exactly `frame_size()` (1024) samples per frame — except the last, which may be shorter. `encoder::find_by_name("pcm_s16le")` takes `Sample::I16(Type::Packed)`, any frame length (`frame_size()` is 0; use 1024).
- Setup: `codec::context::Context::new_with_codec(codec).encoder().audio()?`, then `set_rate(48000)`, `set_channel_layout(ChannelLayout::STEREO)` (no `set_channels` in FFmpeg 7+), `set_format(..)`, `set_time_base(Rational(1, 48000))`, `set_bit_rate(192_000)` for AAC, `set_flags(codec::Flags::GLOBAL_HEADER)` when the container wants it, then `open_as(codec)?`. Then `stream.set_parameters(&encoder)` and `stream.set_time_base(Rational(1, 48000))`.
- `output.add_stream(..)` borrows the output mutably: take the stream's `index()` and let it drop before adding the next. Add the audio stream **after** the video stream is fully set up and **before** `write_header`. Read **both** streams' time bases **after** `write_header` (muxers change them: mkv uses 1/1000).
- Frames: `frame::Audio::new(format, samples, ChannelLayout::STEREO)`, `set_rate(48000)`, `set_pts(Some(samples_sent_so_far))`. Planar: `plane_mut::<f32>(0)` left, `plane_mut::<f32>(1)` right. Packed s16: `plane_mut::<i16>(0)` interleaved.
- Packets: `set_stream(audio_index)`, `rescale_ts(Rational(1, 48000), audio_stream_time_base)`, `write_interleaved`. Finish: `send_eof` and drain **both** encoders before `write_trailer`.
- AAC's first packet has pts −1024 (priming); the mp4/mov muxer writes an edit list, and decoding gives back exactly the samples sent.

- [ ] **Step 1: Write the failing test**

Add to `tests/capture.rs`:

```rust
/// Stereo samples of a 440 Hz tone for frame `pts`: `per_frame` stereo frames.
fn tone(pts: i64, per_frame: usize) -> Vec<i16> {
    (0..per_frame)
        .flat_map(|i| {
            let n = pts as f32 * per_frame as f32 + i as f32;
            let v = ((std::f32::consts::TAU * 440.0 * n / 48_000.0).sin() * 8000.0) as i16;
            [v, v]
        })
        .collect()
}

/// The audio stream's codec, rate, channels, and how many stereo frames decode.
fn decode_sound(path: &Path) -> (ff::codec::Id, u32, u16, usize) {
    ff::init().unwrap();
    let mut input = ff::format::input(path).unwrap();
    let stream = input.streams().best(ff::media::Type::Audio).expect("a sound stream");
    let index = stream.index();
    let mut decoder = ff::codec::context::Context::from_parameters(stream.parameters())
        .unwrap()
        .decoder()
        .audio()
        .unwrap();
    let (id, rate, channels) = (decoder.id(), decoder.rate(), decoder.channels());
    let mut frames = 0;
    let mut decoded = ff::frame::Audio::empty();
    for (stream, packet) in input.packets() {
        if stream.index() == index {
            decoder.send_packet(&packet).unwrap();
            while decoder.receive_frame(&mut decoded).is_ok() {
                frames += decoded.samples();
            }
        }
    }
    decoder.send_eof().unwrap();
    while decoder.receive_frame(&mut decoded).is_ok() {
        frames += decoded.samples();
    }
    (id, rate, channels, frames)
}

#[test]
fn soundtracks_are_written_and_last_as_long_as_the_video() {
    let rate = FrameRate::whole(30);
    let per_frame = 1600; // 48 000 / 30
    for (format, codec) in [
        (VideoFormat::Ffv1, ff::codec::Id::PCM_S16LE),
        (VideoFormat::ProRes4444, ff::codec::Id::PCM_S16LE),
        (VideoFormat::Hevc, ff::codec::Id::AAC),
    ] {
        let path = temp_file(&format!("sound-{format:?}"), format.extension());
        let encoder = match Encoder::start_with_sound(&path, format, SIZE, rate, true) {
            Ok(encoder) => encoder,
            Err(err) if format == VideoFormat::Hevc => {
                eprintln!("skipping HEVC soundtrack: {err:#}");
                continue;
            }
            Err(err) => panic!("{err:#}"),
        };
        for pts in 0..FRAMES {
            encoder.send(Frame { pts, rgba: pattern(pts) }).unwrap();
            encoder.send_sound(tone(pts, per_frame)).unwrap();
        }
        encoder.finish().unwrap();
        let (id, sound_rate, channels, frames) = decode_sound(&path);
        assert_eq!(id, codec, "{format:?}");
        assert_eq!((sound_rate, channels), (48_000, 2), "{format:?}");
        let sent = FRAMES as usize * per_frame;
        let slack = if codec == ff::codec::Id::AAC { 1024 } else { 0 };
        assert!(
            frames.abs_diff(sent) <= slack,
            "{format:?}: decoded {frames} of {sent} sound frames"
        );
    }
}

#[test]
fn a_recording_without_sound_has_no_sound_stream() {
    let path = temp_file("no-sound", "mkv");
    encode(&path, VideoFormat::Ffv1, FrameRate::whole(30)).unwrap();
    ff::init().unwrap();
    let input = ff::format::input(&path).unwrap();
    assert!(input.streams().best(ff::media::Type::Audio).is_none());
}
```

(Use the file's existing imports and alias for `ffmpeg_next`; if it's imported under another name than `ff`, use that. `temp_file`, `pattern`, `encode`, `SIZE`, `FRAMES` are the file's existing helpers.)

Add to `src/capture/recorder.rs`'s tests (or `capture/mod.rs`'s, wherever `FrameClock` is tested) a test of the span arithmetic:

```rust
    #[test]
    fn sound_frames_per_video_frame_add_up_exactly() {
        let rate = FrameRate::ntsc(24); // 24000/1001
        let total: usize = (0..1000).map(|n| sound_frames_for(rate, n)).sum();
        let exact = (1000.0 * 48_000.0 * 1001.0 / 24_000.0_f64).round() as usize;
        assert_eq!(total, exact);
        assert_eq!(sound_frames_for(FrameRate::whole(30), 7), 1600);
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --test capture sound` and `cargo test --lib capture`
Expected: compile errors (`start_with_sound`, `sound_frames_for` not found).

- [ ] **Step 3: The span arithmetic**

In `src/capture/recorder.rs` (module level):

```rust
/// Stereo sound frames (at 48 kHz) that video frame `n` covers at `rate`: frame `n`
/// runs from sample `round(n·48000/fps)` to `round((n+1)·48000/fps)`, so the sound
/// never drifts from the video.
pub fn sound_frames_for(rate: FrameRate, n: i64) -> usize {
    let at = |n: i64| -> i64 {
        let num = i128::from(rate.num);
        let scaled = i128::from(n) * 48_000 * i128::from(rate.den);
        ((scaled * 2 + num) / (num * 2)) as i64
    };
    (at(n + 1) - at(n)) as usize
}
```

(`rate.num` / `rate.den` are the fields `encode.rs` already uses for `Rational(rate.num, rate.den)`; adapt the `i128::from` if they are not integers of a type `i128: From` accepts.)

- [ ] **Step 4: The sound stream in the encoder**

In `src/capture/encode.rs`:

1. Imports: add `ChannelLayout` and `format::sample::{Sample, Type}` from `ff`, and `std::sync::mpsc::{Receiver, Sender, channel}` as needed.

2. `Encoder` gains `sound: Option<Sender<Vec<i16>>>`. `start_with_sound` is today's `start` body with: when `sound` is true, create `let (sound_tx, sound_rx) = channel();`, pass `sound.then_some(sound_rx)` into the thread's `run(...)` and `sound` into `Sink::open`; keep `sound_tx` in the `Encoder`. `start` becomes:

```rust
    pub fn start(path: &Path, format: VideoFormat, size: (u32, u32), rate: FrameRate) -> Result<Self> {
        Self::start_with_sound(path, format, size, rate, false)
    }
```

```rust
    /// Queues interleaved stereo samples (48 kHz) for the soundtrack. Sound is never
    /// dropped, even when video frames are.
    pub fn send_sound(&self, samples: Vec<i16>) -> Result<()> {
        let sound = self
            .sound
            .as_ref()
            .ok_or_else(|| anyhow!("this recording has no soundtrack"))?;
        sound
            .send(samples)
            .map_err(|_| anyhow!("the encoder stopped"))
    }
```

`finish` must drop `self.sound` (set it to `None`) as well as the frame sender before joining the thread.

3. The thread body `run(...)` takes `sound: Option<Receiver<Vec<i16>>>`. Before each `sink.write(&frame)`, write the sound that has arrived:

```rust
        if let Some(sound) = &sound {
            for samples in sound.try_iter() {
                sink.write_sound(&samples)?;
            }
        }
```

and after the frame loop (the frame channel closed), before `sink.finish()?`:

```rust
    if let Some(sound) = sound {
        for samples in sound.iter() {
            sink.write_sound(&samples)?;
        }
    }
```

(`sound.iter()` ends once `Encoder::finish` drops the sender. Wrap errors the way the loop does for frames, e.g. `.context("encoding the soundtrack")`, and make sure an error path still calls `sink.finish()` as the existing frame path does.)

4. `Sink` gains `sound: Option<SoundTrack>`:

```rust
/// The soundtrack's encoder and stream.
struct SoundTrack {
    encoder: encoder::Audio,
    index: usize,
    stream_time_base: Rational,
    /// Interleaved stereo samples not yet sent (the encoder takes whole frames).
    pending: Vec<i16>,
    /// The next sample's position since the start.
    next_pts: i64,
    /// Stereo frames per encoder frame.
    frame_size: usize,
    /// AAC takes planar floats; PCM takes packed 16-bit.
    planar: bool,
}

/// The sample time base: one sample at 48 kHz.
const SOUND_TIME_BASE: Rational = Rational(1, 48_000);
```

In `Sink::open(path, format, size, rate, sound: bool)`: after the video stream is fully set up (after `stream.set_avg_frame_rate(..)`) and **before** `output.write_header_with(..)`:

```rust
    let mut track = None;
    if sound {
        let (name, sample) = match format {
            VideoFormat::Hevc => ("aac", Sample::F32(Type::Planar)),
            VideoFormat::Ffv1 | VideoFormat::ProRes4444 => ("pcm_s16le", Sample::I16(Type::Packed)),
        };
        let codec = encoder::find_by_name(name)
            .ok_or_else(|| anyhow!("this FFmpeg build has no {name} encoder"))?;
        let mut stream = output.add_stream(codec)?;
        let index = stream.index();
        let mut setup = codec::context::Context::new_with_codec(codec).encoder().audio()?;
        setup.set_rate(48_000);
        setup.set_channel_layout(ChannelLayout::STEREO);
        setup.set_format(sample);
        setup.set_time_base(SOUND_TIME_BASE);
        if format == VideoFormat::Hevc {
            setup.set_bit_rate(192_000);
        }
        if global_header {
            setup.set_flags(codec::Flags::GLOBAL_HEADER);
        }
        let encoder = setup
            .open_as(codec)
            .with_context(|| format!("could not open the {name} encoder"))?;
        stream.set_parameters(&encoder);
        stream.set_time_base(SOUND_TIME_BASE);
        let frame_size = match encoder.frame_size() {
            0 => 1024,
            n => n as usize,
        };
        track = Some(SoundTrack {
            encoder,
            index,
            stream_time_base: SOUND_TIME_BASE,
            pending: Vec::new(),
            next_pts: 0,
            frame_size,
            planar: sample == Sample::F32(Type::Planar),
        });
    }
```

(If the video `stream` variable is still borrowed at that point, end its use before this block; `add_stream` needs the output mutably.) After `write_header_with(..)`, alongside the video stream's time base, read the audio stream's:

```rust
    if let Some(track) = &mut track {
        track.stream_time_base = output
            .stream(track.index)
            .context("no sound stream")?
            .time_base();
    }
```

and store `sound: track` in the `Sink`.

5. Writing and finishing:

```rust
impl Sink {
    /// Queues soundtrack samples and encodes every whole frame of them.
    fn write_sound(&mut self, samples: &[i16]) -> Result<()> {
        let Some(track) = &mut self.sound else {
            return Ok(());
        };
        track.pending.extend_from_slice(samples);
        while track.pending.len() >= track.frame_size * 2 {
            let chunk: Vec<i16> = track.pending.drain(..track.frame_size * 2).collect();
            Self::send_sound(track, &mut self.output, &chunk)?;
        }
        Ok(())
    }

    /// Encodes one frame of interleaved stereo samples.
    fn send_sound(track: &mut SoundTrack, output: &mut format::context::Output, chunk: &[i16]) -> Result<()> {
        let samples = chunk.len() / 2;
        let format = if track.planar {
            Sample::F32(Type::Planar)
        } else {
            Sample::I16(Type::Packed)
        };
        let mut frame = frame::Audio::new(format, samples, ChannelLayout::STEREO);
        frame.set_rate(48_000);
        if track.planar {
            for (i, pair) in chunk.chunks_exact(2).enumerate() {
                frame.plane_mut::<f32>(0)[i] = f32::from(pair[0]) / 32768.0;
                frame.plane_mut::<f32>(1)[i] = f32::from(pair[1]) / 32768.0;
            }
        } else {
            frame.plane_mut::<i16>(0).copy_from_slice(chunk);
        }
        frame.set_pts(Some(track.next_pts));
        track.next_pts += samples as i64;
        track.encoder.send_frame(&frame)?;
        Self::drain_sound(track, output)
    }

    /// Writes the soundtrack's finished packets.
    fn drain_sound(track: &mut SoundTrack, output: &mut format::context::Output) -> Result<()> {
        let mut packet = Packet::empty();
        while track.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(track.index);
            packet.rescale_ts(SOUND_TIME_BASE, track.stream_time_base);
            packet.write_interleaved(output)?;
        }
        Ok(())
    }
}
```

(Match the error handling the video `drain` uses for `receive_packet` — if it distinguishes EAGAIN/EOF from real errors, do the same here.) In `Sink::finish`, after the video encoder's `send_eof` and drain and **before** `write_trailer`:

```rust
        if let Some(track) = &mut self.sound {
            if !track.pending.is_empty() {
                let rest = std::mem::take(&mut track.pending);
                Self::send_sound(track, &mut self.output, &rest)?;
            }
            track.encoder.send_eof()?;
            Self::drain_sound(track, &mut self.output)?;
        }
```

- [ ] **Step 5: The recorder**

In `src/capture/recorder.rs`, `Recorder` gains `sound: bool`. `start_with_sound(device, settings, canvas, rate, sound)` is today's `start` with `Encoder::start_with_sound(&path, settings.format, canvas, rate, sound)?` and `sound` stored; `start` calls it with `false`. Add:

```rust
    /// Whether this recording has a soundtrack.
    pub fn has_sound(&self) -> bool {
        self.sound
    }

    /// Stereo sound frames the frame just captured covers.
    pub fn sound_frames(&self) -> usize {
        sound_frames_for(self.rate(), self.clock.frames() - 1)
    }

    /// Adds the soundtrack for the frame just captured.
    pub fn add_sound(&mut self, samples: Vec<i16>) -> Result<()> {
        if self.sound {
            self.encoder.send_sound(samples)?;
        }
        Ok(())
    }
```

(`self.rate()` stands for however the recorder knows its frame rate — its `FrameClock` was built from it; store the `FrameRate` in a field if there's no accessor. `clock.frames()` counts frames captured so far, so the latest has pts `frames() - 1`.)

- [ ] **Step 6: The app**

In `src/app.rs`:

1. `start_recording`: decide the soundtrack and restart the file before starting:

```rust
        let sound = self.audio.file().is_some();
        if sound && self.ui.audio.start_with_recording {
            self.audio.restart();
        }
        let started = Recorder::start_with_sound(
            &self.device,
            &self.ui.capture.settings,
            self.renderer.size(),
            self.clock.rate(),
            sound,
        );
```

2. `draw_canvas_frame`: inside the recorder block, right after a successful `recorder.capture(...)`, using Task 8's `audio_frame`:

```rust
            if recorder.has_sound()
                && let Some(span) = audio_frame.sound
            {
                let frames = recorder.sound_frames();
                if let Err(err) = recorder.add_sound(self.audio.soundtrack(span, frames)) {
                    log::warn!("{err:#}");
                    capture_failed = true;
                }
            }
```

(The block already skips paused frames, so no sound is added for them. If a recording with sound runs while the source changes away from the file, `audio_frame.sound` is `None`: add silence of `recorder.sound_frames()` frames instead — `vec![0; frames * 2]` — so the soundtrack keeps pace with the video.)

3. Stop at the file's end: where the function checks `capture_failed || recorder.finished_recording()`, also stop when the recording has sound and the file ended:

```rust
            if capture_failed
                || recorder.finished_recording()
                || (recorder.has_sound() && self.audio.ended())
            {
                self.stop_recording();
            }
```

- [ ] **Step 7: Run the tests**

Run: `cargo test --test capture`, `cargo test --lib`, `cargo test --test smoke`, `cargo test --test video`
Expected: all pass (HEVC skipped only if NVENC is missing; on this machine it is present).

- [ ] **Step 8: Check and commit**

Run: `cargo fmt`, `cargo clippy --all-targets`.

```bash
git add src/capture src/app.rs tests/capture.rs
git commit -m "feat: recordings carry the sound file as their soundtrack"
```

---

### Task 10: The Audio section and audio items in the link menus

**Files:**
- Modify: `src/link_ui.rs` (Audio and Beat menu items; hover names audio links)
- Modify: `src/audio_ui.rs` (`AudioUi` fields, `AudioActions`, `audio_section`)
- Modify: `src/ui.rs` (`UiActions.audio`; draw the section after the MIDI section)
- Modify: `src/app.rs` (fill `AudioUi` each refresh; apply edits and actions)

**Interfaces:**
- Consumes: everything above; `link_ui::link_menu`; `Links.audio`; `Audio` methods; `input::{input_devices, output_devices}`; `Motion::offsets`.
- Produces:
  - `pub enum crate::audio_ui::SourceKind { None, Input, Loopback, File }` with `ALL` and `label`.
  - `AudioUi` fields (all `pub`): `start_with_recording: bool`, `kind: SourceKind`, `inputs: Option<Vec<String>>`, `outputs: Option<Vec<String>>`, `listing_error: Option<String>`, `chosen: SourceChoice`, `file: Option<FileState>`, `loading: Option<String>`, `error: Option<String>`, `shaping: Shaping`, `volume: f32`, `looping: bool`, `output: String`, `meter: Signals`, `lit: [Option<Instant>; 3]`.
  - `pub struct AudioActions { pub use_source: Option<SourceChoice>, pub choose_file: bool, pub list_devices: bool, pub play: Option<bool>, pub restart: bool, pub seek: Option<f64>, pub tap: Option<Beat> }` (`Default`).
  - `pub fn audio_section(ui: &mut Ui, audio: &mut AudioUi, links: &mut Links, base: &Params, offsets: &[(SliderId, f32)], actions: &mut AudioActions)`.
  - `UiActions.audio: AudioActions`.

- [ ] **Step 1: Write the failing tests**

Add to `src/link_ui.rs`'s tests (they use the module's `frame` helper, which draws once in a headless egui context):

```rust
    #[test]
    fn hover_text_names_midi_and_audio_links() {
        use crate::audio::{Beat, Signal};
        use crate::control::Action;
        let mut links = Links::default();
        links.audio.follow_signal(Signal::Bass, SliderId::Zoom);
        links.audio.follow[0].depth = 0.5;
        assert_eq!(
            link_names(&links, Target::Slider(SliderId::Zoom)),
            Some("Audio: Bass +0.50".to_string())
        );
        links.audio.fire_on(Beat::Any, Target::Action(Action::Cut));
        assert_eq!(
            link_names(&links, Target::Action(Action::Cut)),
            Some("Beat: Any".to_string())
        );
        assert_eq!(link_names(&links, Target::Action(Action::Pause)), None);
    }

    #[test]
    fn the_audio_section_draws_with_and_without_links() {
        use crate::audio::Signal;
        use crate::audio_ui::{AudioActions, AudioUi, audio_section};
        let mut links = Links::default();
        let params = Params::default();
        let mut audio = AudioUi::default();
        frame(|ui| {
            audio_section(ui, &mut audio, &mut links, &params, &[], &mut AudioActions::default())
        });
        links.audio.follow_signal(Signal::Level, SliderId::Zoom);
        frame(|ui| {
            audio_section(
                ui,
                &mut audio,
                &mut links,
                &params,
                &[(SliderId::Zoom, 0.25)],
                &mut AudioActions::default(),
            )
        });
    }
```

(If the module's `frame` helper has another name or shape, use it the way the existing link_ui tests do.)

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --lib link_ui`
Expected: compile errors (`link_names`, `audio_section` not found).

- [ ] **Step 3: Menu items and hover text in `link_ui.rs`**

Imports: `use crate::audio::{Beat, Signal};`, `use crate::audio_links::{AudioLinks, fires};`.

Replace `name_links` with one function that builds the whole hover text, and a helper the tests call:

```rust
/// Everything linked to `target`, for a hover text: "MIDI: CC 1 ch 1 · Audio: Bass
/// +0.50 · Beat: Any". None when nothing is.
fn link_names(links: &Links, target: Target) -> Option<String> {
    let mut parts = Vec::new();
    let midi = sources(links, target);
    if !midi.is_empty() {
        parts.push(format!("MIDI: {}", names(&midi)));
    }
    if let Target::Slider(id) = target {
        for link in links.audio.follow.iter().filter(|l| l.slider == id) {
            parts.push(format!("Audio: {} {:+.2}", link.signal.label(), link.depth));
        }
    }
    for beat in links.audio.beats_for(target) {
        parts.push(format!("Beat: {}", beat.label()));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Names `target`'s links when the control is hovered.
fn name_links(response: &Response, links: &Links, target: Target) {
    if let Some(text) = link_names(links, target) {
        response.clone().on_hover_text(text);
    }
}
```

Add the audio items, called after `menu_items` by `link_menu` and `slider_menu` (each menu closure becomes `|ui| { menu_items(ui, links, target, "Link to MIDI"); audio_items(ui, &mut links.audio, target, None); }`):

```rust
/// "Audio: Level" … for a slider and "Beat: Any" … for anything a beat can fire, or
/// "Unlink …" for links already made. `option` names a drop-down's option.
fn audio_items(ui: &mut Ui, links: &mut AudioLinks, target: Target, option: Option<&str>) {
    let on = option.map(|o| format!(" → {o}")).unwrap_or_default();
    match target {
        Target::Slider(id) => {
            ui.separator();
            let linked = links.signals_for(id);
            for signal in Signal::ALL {
                if linked.contains(&signal) {
                    if ui
                        .button(format!("Unlink Audio: {}", signal.label()))
                        .clicked()
                    {
                        links.unfollow(signal, id);
                        ui.close();
                    }
                } else if ui.button(format!("Audio: {}", signal.label())).clicked() {
                    links.follow_signal(signal, id);
                    ui.close();
                }
            }
        }
        target if fires(target) => {
            ui.separator();
            let linked = links.beats_for(target);
            for beat in Beat::ALL {
                if linked.contains(&beat) {
                    if ui
                        .button(format!("Unlink Beat: {}{on}", beat.label()))
                        .clicked()
                    {
                        links.unfire(beat, target);
                        ui.close();
                    }
                } else if ui.button(format!("Beat: {}{on}", beat.label())).clicked() {
                    links.fire_on(beat, target);
                    ui.close();
                }
            }
        }
        _ => {}
    }
}
```

In `combo`, build the hover from `link_names` per option (prefixing each option's text with its label, e.g. `"{label}: {names}"`), and in its context menu call `audio_items(ui, &mut links.audio, Target::Choice(id, option), Some(label))` after each option's `menu_items`.

- [ ] **Step 4: The section in `audio_ui.rs`**

Extend `src/audio_ui.rs` (keep `start_with_recording` and its default `true`):

```rust
use std::time::{Duration, Instant};

use egui::{CollapsingHeader, Color32, ComboBox, DragValue, ProgressBar, Slider, Ui};

use crate::audio::{
    ATTACK, Beat, FileState, GAIN, RELEASE, SENSITIVITY, Shaping, Signal, Signals, SourceChoice,
};
use crate::control::{Action, Links, Target};
use crate::link_ui;
use crate::params::Params;
use crate::params::table::SliderId;

/// How long a beat light stays lit.
const LIT: Duration = Duration::from_millis(150);

/// The kinds of source the section offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceKind {
    #[default]
    None,
    Input,
    Loopback,
    File,
}

impl SourceKind {
    pub const ALL: [SourceKind; 4] = [
        SourceKind::None,
        SourceKind::Input,
        SourceKind::Loopback,
        SourceKind::File,
    ];

    /// Its name in the panel.
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::None => "None",
            SourceKind::Input => "Input",
            SourceKind::Loopback => "Loopback",
            SourceKind::File => "File",
        }
    }

    /// The kind of `choice`.
    pub fn of(choice: &SourceChoice) -> SourceKind {
        match choice {
            SourceChoice::None => SourceKind::None,
            SourceChoice::Input(_) => SourceKind::Input,
            SourceChoice::Loopback(_) => SourceKind::Loopback,
            SourceChoice::File(_) => SourceKind::File,
        }
    }
}
```

`AudioUi` gets the fields listed in Interfaces, with `Default`: `start_with_recording: true`, `kind: SourceKind::None`, `inputs: None`, `outputs: None`, `listing_error: None`, `chosen: SourceChoice::None`, `file: None`, `loading: None`, `error: None`, `shaping: Shaping::default()`, `volume: 1.0`, `looping: true`, `output: String::new()`, `meter: Signals::default()`, `lit: [None; 3]`. Document each field in one line.

`AudioActions` as in Interfaces, `#[derive(Default)]`, each field documented.

The section:

```rust
/// The Audio section (collapsed by default).
pub fn audio_section(
    ui: &mut Ui,
    audio: &mut AudioUi,
    links: &mut Links,
    base: &Params,
    offsets: &[(SliderId, f32)],
    actions: &mut AudioActions,
) {
    CollapsingHeader::new("Audio").show(ui, |ui| {
        source(ui, audio, actions);
        if let Some(error) = &audio.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        ui.add(Slider::new(&mut audio.shaping.gain, GAIN).text("gain"));
        ui.add(
            Slider::new(&mut audio.shaping.attack, ATTACK)
                .logarithmic(true)
                .text("attack (s)"),
        );
        ui.add(
            Slider::new(&mut audio.shaping.release, RELEASE)
                .logarithmic(true)
                .text("release (s)"),
        );
        ui.add(Slider::new(&mut audio.shaping.sensitivity, SENSITIVITY).text("beat sensitivity"));
        meter(ui, audio);
        ui.horizontal(|ui| {
            for (beat, action, text) in [
                (Beat::Any, Action::Beat, "Beat"),
                (Beat::Bass, Action::BassBeat, "Bass beat"),
                (Beat::Treble, Action::TrebleBeat, "Treble beat"),
            ] {
                let tap = ui.button(text).on_hover_text("Fire this beat by hand");
                if tap.clicked() {
                    actions.tap = Some(beat);
                }
                link_ui::link_menu(&tap, links, Target::Action(action));
            }
        });
        link_list(ui, links, base, offsets);
    });
}
```

`source` draws the kind row (`ui.selectable_value(&mut audio.kind, kind, kind.label())` for each `SourceKind::ALL`; picking None reports `actions.use_source = Some(SourceChoice::None)`); for Input/Loopback a `ComboBox` of the listed devices (`audio.inputs` / `audio.outputs`; when `None`, set `actions.list_devices = true` and show "Looking for devices…") whose pick reports `use_source = Some(SourceChoice::Input(name))` or `Loopback(name)`, a **Refresh** button (`list_devices = true`), "No devices" when the list is empty, and `listing_error` in red; for File, **Open…** (`choose_file = true`), the loading line, and when `audio.file` is set: the name, a Play/Pause button (`play = Some(!playing)`), **Restart** (`restart = true`), a position `Slider` from 0 to the length whose `changed()` reports `seek`, a **Loop** checkbox on `audio.looping`, a volume slider (0–1), an output `ComboBox` ("System default" for `""`, then `audio.outputs`), and the "Start the sound with the recording" checkbox on `audio.start_with_recording`.

`meter` draws one `ProgressBar::new(audio.meter.get(signal)).text(signal.label())` per `Signal::ALL`, and a row of three labels for `Beat::ALL` coloured `Color32::YELLOW` while `audio.lit[beat.index()]` is within `LIT` of now, grey otherwise.

`link_list` draws, when there are no audio links, the hint "Right-click a slider, option or button and choose an Audio or Beat item." Otherwise an `egui::Grid::new("audio links").striped(true)`: for each follow link — the signal's label, `Target::Slider(link.slider).label()`, a `DragValue::new(&mut link.depth).range(-span..=span).speed(span / 200.0)` (span = `link.span()`), the shown value `format!("{:.2}", shown)` where `shown = (link.slider.get(base) + offset).clamp(range)` with `offset` this slider's entry in `offsets` (0 if absent), and a **Remove** small button; for each beat link — "Beat: <label>", the target's label, an empty cell, an empty cell, **Remove**. Removals are applied after the grid (collect the index first, as `midi_ui::midi_section` does).

- [ ] **Step 5: Draw it and wire the app**

In `src/ui.rs`: `UiActions` gains `pub audio: AudioActions` (documented). In `draw`, right after the `midi_section` call:

```rust
        let base = *motion.editable();
        let offsets = motion.offsets().to_vec();
        audio_ui::audio_section(
            ui,
            &mut state.audio,
            &mut state.midi.links,
            &base,
            &offsets,
            &mut actions.audio,
        );
```

In `src/app.rs`:

1. Each refresh, before `ui::draw` (next to `self.show_inputs()`), copy the engine's state into the panel:

```rust
    /// Shows the audio engine's state in the Audio section.
    fn show_audio(&mut self) {
        let a = &mut self.ui.audio;
        a.chosen = self.audio.choice().clone();
        a.file = self.audio.file();
        a.loading = self
            .audio
            .loading()
            .map(|(name, done)| format!("Loading {name}… {:.0}%", done * 100.0));
        a.error = self.audio.error().map(str::to_string);
        a.shaping = self.audio.shaping();
        a.volume = self.audio.volume();
        a.looping = self.audio.looping();
        a.output = self.audio.output().to_string();
        a.meter = self.audio.meter();
    }
```

and in `draw_canvas_frame`, after the audio frame is read (Task 8's block), light the beats:

```rust
        for beat in Beat::ALL {
            if audio_frame.signals.beat(beat) {
                self.ui.audio.lit[beat.index()] = Some(Instant::now());
            }
        }
```

In `apply_audio` (Task 8) also set `self.ui.audio.kind = SourceKind::of(&audio.source);`. On startup the kind starts at None, matching the default project.

2. After `ui::draw`, apply what the panel edited and asked for:

```rust
    /// Applies the Audio section's edits and requests.
    fn audio_actions(&mut self, actions: AudioActions) {
        let a = &self.ui.audio;
        self.audio.set_shaping(a.shaping);
        self.audio.set_volume(a.volume);
        self.audio.set_looping(a.looping);
        let output = a.output.clone();
        self.audio.set_output(&output);
        if let Some(choice) = actions.use_source {
            self.audio.use_choice(choice, &self.budget);
        }
        if actions.choose_file
            && let Some(path) = rfd::FileDialog::new()
                .set_title("Open a sound file")
                .add_filter("Sound", SOUND_EXTENSIONS)
                .pick_file()
        {
            self.audio.use_choice(SourceChoice::File(absolute(&path)), &self.budget);
        }
        if actions.choose_file {
            // The dialog blocks the app; that must not make canvas frames late.
            self.clock.reanchor(self.seconds());
        }
        if actions.list_devices {
            let (inputs, outputs) = (input_devices(), output_devices());
            self.ui.audio.listing_error = inputs.as_ref().err().or(outputs.as_ref().err()).cloned();
            self.ui.audio.inputs = Some(inputs.unwrap_or_default());
            self.ui.audio.outputs = Some(outputs.unwrap_or_default());
            self.audio.retry(&self.budget);
            self.clock.reanchor(self.seconds());
        }
        if let Some(playing) = actions.play {
            self.audio.set_playing(playing);
        }
        if actions.restart {
            self.audio.restart();
        }
        if let Some(seconds) = actions.seek {
            self.audio.seek(seconds);
        }
        if let Some(beat) = actions.tap {
            self.audio.tap(beat);
        }
    }
```

with `const SOUND_EXTENSIONS: &[&str] = &["wav", "mp3", "flac", "ogg", "m4a", "aac", "opus"];` next to `MEDIA_EXTENSIONS`, and the imports `crate::audio::{Beat, SourceChoice}`, `crate::audio::input::{input_devices, output_devices}`, `crate::audio_ui::{AudioActions, SourceKind}`. Call `self.audio_actions(actions.audio)` where the other `UiActions` are handled (move `actions.audio` out before other fields are used, or `std::mem::take` it). Also call `self.show_audio();` beside `self.show_inputs();`.

- [ ] **Step 6: Run the tests**

Run: `cargo test --lib`, `cargo test --test smoke`
Expected: all pass.

- [ ] **Step 7: Check by hand**

Launch with scratch settings (PowerShell): `$env:APPDATA = "$env:TEMP\rasterwarp-audio-check"; cargo run --release`. Launching opens the MIDI ports and plays sound; close the app when done and leave nothing running.

1. Audio → File → Open… `samples/drums.wav`: the loading line, then it plays on the speakers; the meter moves; the beat lights flash on the kicks.
2. Right-click zoom → **Audio: Bass**; zoom pumps with the kick while the panel slider stays put. The Audio section lists it with depth 0.25 of the range and a moving shown value; drag the depth negative and the pump inverts.
3. Right-click Cut → **Beat: Bass** in Transition mode: every kick cuts.
4. Pause (the app's): the sound stops and the picture freezes; resume continues in sync. Scrub the position; the sound jumps there. Untick Loop: it stops at the end.
5. Input / Loopback: Refresh lists devices; pick the speakers' loopback while something plays in another app: the meter follows it; with nothing playing it falls to zero.
6. Tap **Beat** with no source (None): Pulse and beat links fire. Right-click **Bass beat** → Link to MIDI and press a Q25 key: the key taps the beat.
7. Capture → HEVC, Frame-by-frame, Record with drums.wav loaded and "Start the sound with the recording" ticked: the recording stops at the file's end; play the `.mp4` and check sound and picture are in sync. Repeat with ProRes (`.mov`) in real time.
8. Save the project, close, reopen it: the source, settings and links come back.

- [ ] **Step 8: Check and commit**

Run: `cargo fmt`, `cargo clippy --all-targets`, `cargo test --lib`.

```bash
git add src/link_ui.rs src/audio_ui.rs src/ui.rs src/app.rs
git commit -m "feat: the Audio section, and Audio and Beat items in the link menus"
```

---

## Self-review (done while writing)

- **Spec coverage:** sources (Tasks 5, 6, 10), signals and analysis (Task 1), live and file analysis (Tasks 4–6), hand beats and Pulse (Tasks 1, 6, 8), follow and beat links (Task 2), where modulation applies incl. preview and not marking unsaved (Task 3; the project compares `Params`, which modulation never touches), the sound file's playhead, speakers, sync, mute while recording frame-by-frame (Tasks 5, 6, 8), memory and loading (Task 4), recording (Task 9), the panel and menus (Task 10), saving (Task 7, with app glue in Task 8), errors (Tasks 5, 6, 10), testing (each task; by hand in Tasks 8, 10).
- **Deviations from the spec's wording, by ruling:** the engine lives in `src/audio/engine.rs` (re-exported from `audio`); live analysis runs on the app thread from blocks the callback sends (the spec's atomic slot), because loopback sends nothing during silence and the app must fill that gap; `AudioProject` lives in `session.rs`.
- **Types:** `Signal`, `Beat`, `Signals`, `Shaping`, `Pulse`, `Analyzer`, `Tracks`, `Sound`, `Live`, `Speakers`, `Shared`, `Audio`, `AudioFrame`, `SoundSpan`, `FileState`, `SourceChoice`, `AudioLink`, `BeatLink`, `AudioLinks`, `AudioProject`, `AudioUi`, `AudioActions`, `SourceKind` are named the same in every task.
