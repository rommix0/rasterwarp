# Rasterwarp Save and Load Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Save and load presets (one look) and projects (the whole session), with an autosave that brings the session back on every launch.

**Architecture:**
- **`save`** owns the file formats: JSON with a format name and version, read tolerantly (missing fields default, unknown fields are ignored, unknown options fall back, numbers are clamped), and written through a temporary file.
- **`session`** is the project in memory: a snapshot of the running session (mode, banks, sequence, curves, canvas, frame rate, image paths) that can rebuild the motion at rest. Comparing snapshots tells whether there are unsaved changes.
- **`presets`** manages the presets folder; **`files_ui`** draws the project row and Presets section; **`app`** runs dialogs, the autosave and drop routing.

**Tech Stack:** Rust 1.98 (edition 2024), serde 1 (derive) and serde_json 1 (new), egui 0.36, rfd 0.17, wgpu 30, winit 0.30.

**Spec:** `docs/superpowers/specs/2026-10-07-rasterwarp-save-load-design.md` (refined alongside this plan; see the design decisions below).

## Global Constraints

- Platform: Windows 10, NVIDIA RTX 3090, Rust 1.98 stable MSVC, edition 2024.
- New crates: exactly `serde = { version = "1", features = ["derive"] }` and `serde_json = "1"`. No others. The existing versions stay pinned: wgpu 30, winit 0.30, egui 0.36, ffmpeg-next 9.0.0, chrono 0.4, rfd 0.17, image 0.25.
- `cargo build`, `cargo test` and `cargo clippy --all-targets` must have zero warnings. Run `cargo fmt` before committing.
- Every commit message ends with the trailer line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Don't use the word "Scanimate" in user-facing text.
- Files, exactly: presets `<name>.rwpreset` (`"format": "rasterwarp-preset"`), projects `.rwproject` (`"format": "rasterwarp-project"`), settings `settings.json` (`"format": "rasterwarp-settings"`), all `"version": 1`; autosave `autosave.rwproject`, a bad one kept as `autosave.bad.rwproject`; data folder `%APPDATA%\rasterwarp` (or `rasterwarp-data` in the working directory without `APPDATA`); default presets folder `presets`; autosave every 60 s when changed and on close.
- When testing the app by hand, point `APPDATA` at a scratch folder so the developer's own settings and autosave are untouched.
- Code in this plan was compiled, tested and run on the dev machine before the plan was written. Every task's end state was also built and tested on its own. Transcribe it exactly. Each edit's "replace" text appears exactly once in the file when applied in the order given. A few inserted blocks end just before an existing closing brace, which then closes them; the result is the tested file.

## Design decisions made while building (beyond the spec text)

1. **Enums are saved by name** (`"sine"`, `"transition"`, `"real-time"`, …) through the `saved_names!` macro, which also makes unknown names read as the default. Curves are `"linear"`, `"s-curve"` or `"custom:<id>"`; an unknown curve (or one naming a user curve not in the file) becomes Linear, and a missing curve field takes its default like any other field.
2. **Unsaved changes** are found by comparing project snapshots. Panel widgets therefore must not change values just by being drawn: the palette color buttons edit a copy (their color conversion isn't exact), and the threshold sliders show 3 decimals with a formatter instead of rounding the value (`max_decimals` rounds it every frame). Ramps in progress, the sequence's position and the oscillator phases are not in the snapshot, so a running transition doesn't count as a change.
3. **The canvas is saved as its size** (the preset choice follows from it), sanitized on opening and again against the GPU's limit when applied.
4. **The autosave is a project file** with two more fields (`file`, `unsaved`), so it also opens as a project. Restoring it sets the "last saved" snapshot only when it wasn't unsaved.
5. **Settings are written with the autosave** (every 60 s when they changed, and on close), not on every change, so dragging a value doesn't write a file per frame.
6. **"Reset all" stays** (it resets only the look being edited); **New** resets the whole session and makes it Untitled.
7. **Questions use the system's Yes / No / Cancel buttons** (Yes saves, No discards); custom button labels need a Windows manifest feature the app doesn't have. Open project… and New are disabled while recording, because the canvas can't change then.
8. **Loading a preset in Live mode is a cut**, so the raster beam speed resets for that frame (like a Transition cut).
9. **Fixtures:** `tests/fixtures/v1.rwpreset` is a complete file written by this version. `tests/fixtures/v1.rwproject` is a compact hand-written version-1 project that lists only non-default values (a full one is 670 lines), so it also guards the missing-fields rule. Neither is ever regenerated.
10. **Verified in the real app:** quit and relaunch restores the session; saving a preset, the replace question, Ctrl+S on an Untitled session, Save as, New with the unsaved question, and a clean header after New and after saving in every mode with every section open. Each blocking dialog adds a few late frames (as the background-image dialog already did).

## File Structure

| File | Responsibility |
|---|---|
| `Cargo.toml` (modify) | `serde`, `serde_json` |
| `src/save.rs` (new) | File envelope, tolerant reading, safe writing, presets as files, app settings, data folder |
| `src/session.rs` (new) | Project snapshot, rebuilding motion, project files, autosave |
| `src/presets.rs` (new) | The presets folder |
| `src/files_ui.rs` (new) | Project row, Presets section, shortcuts |
| `src/params.rs` (modify) | serde, defaults for each part, `Params::clamped` |
| `src/curve.rs`, `src/sequence.rs`, `src/motion.rs`, `src/rate.rs` (modify) | serde and repair on load |
| `src/capture/*.rs` (modify) | serde for the capture settings |
| `src/app.rs` (modify) | Image paths, restore, autosave, dialogs, file actions, drop routing |
| `src/ui.rs` (modify) | File messages, the new sections, widgets that don't write back unchanged values |
| `tests/fixtures/v1.rwpreset`, `tests/fixtures/v1.rwproject` (new) | Version-1 files that must always load |

---
### Task 1: Preset files and tolerant reading

Adds `serde` and `serde_json`, makes every parameter readable and writable as JSON, and adds the preset file format. Reading is tolerant: a missing field takes its default, an unknown field is ignored, an unknown option (say, a waveform from a later version) falls back to its default, and numbers are clamped to their slider ranges. Every write goes to a temporary file first.

**Files:**
- Create: `src/save.rs`, `tests/fixtures/v1.rwpreset`
- Modify: `Cargo.toml`, `src/lib.rs`, `src/params.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `save::{VERSION (1), PRESET_FORMAT ("rasterwarp-preset"), PRESET_EXTENSION ("rwpreset"), PROJECT_FORMAT ("rasterwarp-project"), PROJECT_EXTENSION ("rwproject"), SETTINGS_FORMAT ("rasterwarp-settings")}`.
  - `save::Loaded<T> { value: T, newer: bool }` (`newer`: a later version wrote the file).
  - `save::saved_names!` (crate-visible macro): `saved_names!(Type { Variant => "name", … });` saves an enum by name; an unknown name reads as `Type::default()`.
  - `save::to_json<T: Serialize>(format: &str, contents: &T) -> String` and `save::from_json<T: DeserializeOwned>(format: &str, text: &str) -> anyhow::Result<Loaded<T>>`. Errors read "this isn't a JSON file", "this is a project, not a preset", "this isn't a rasterwarp file" or "some settings in the file are damaged".
  - `save::write_atomic(path: &Path, text: &str) -> Result<()>` (via `<name>.tmp`, creating the folder) and `save::read_file(path: &Path) -> Result<String>`.
  - `save::{preset_json(&Params) -> String, read_preset(&str) -> Result<Loaded<Params>>, save_preset(&Path, &Params) -> Result<()>, load_preset(&Path) -> Result<Loaded<Params>>}`.
  - `Params::clamped(self) -> Params`; `Default` for `WarpParams`, `ColorizeParams`, `FeedbackParams`, `GlowParams`, `RasterParams`, `KeyParams` (the default look's part) and for the oscillator enums; `Serialize`/`Deserialize` on all of them.
  - Test helper `save::temp_dir(name: &str) -> PathBuf` (`#[cfg(test)]`): a fresh, empty folder.

- [ ] **Step 1: Add the dependencies.**

In `Cargo.toml`, replace:

```toml
rfd = "0.17"
```

with:

```toml
rfd = "0.17"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

Cargo adds `serde_json` and its dependencies to `Cargo.lock` on the next build; commit `Cargo.lock` with the task.

- [ ] **Step 2: Write the failing tests.**

In `src/lib.rs`, replace:

```rust
pub mod rate;
```

with:

```rust
pub mod rate;
pub mod save;
```

In `src/params.rs`, replace:

```rust
    }

    #[test]
    fn keeping_levels_clears_the_hidden_see_through_bits() {
```

with:

```rust
    }

    #[test]
    fn clamping_moves_values_into_their_ranges() {
        assert_eq!(Params::default().clamped(), Params::default());
        let mut p = Params::default();
        p.warp.zoom = 99.0;
        p.warp.oscillators[2].frequency = -5.0;
        p.colorize.levels = 50;
        p.colorize.palette[3][1] = 2.0;
        p.raster.lines = 5;
        p.key.levels = 0xff;
        let p = p.clamped();
        assert_eq!(p.warp.zoom, 4.0);
        assert_eq!(p.warp.oscillators[2].frequency, 0.0);
        assert_eq!(p.colorize.levels, 8);
        assert_eq!(p.colorize.palette[3][1], 1.0);
        assert_eq!(p.raster.lines, 100);
        assert_eq!(p.key.levels, 0xff, "all eight levels exist");
        let mut few = Params::default();
        few.colorize.levels = 3;
        few.key.levels = 0xff;
        assert_eq!(few.clamped().key.levels, 0b111);
    }

    #[test]
    fn keeping_levels_clears_the_hidden_see_through_bits() {
```

Create `src/save.rs` holding only its tests for now (Step 4 puts the implementation above them):

```rust
#[cfg(test)]
pub(crate) fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rasterwarp-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{Axis, Envelope, OscInput, OscSync, Waveform};

    /// Every parameter changed from its default.
    fn changed() -> Params {
        let mut p = Params::default();
        for o in &mut p.warp.oscillators {
            o.waveform = Waveform::Square;
            o.target = Axis::Y;
            o.input = OscInput::Time;
            o.frequency = 7.5;
            o.amplitude = 0.25;
            o.phase = 0.75;
            o.phase_speed = -1.5;
            o.lfo_rate = 3.0;
            o.lfo_depth = 0.6;
            o.sync = OscSync::Frame;
            o.envelope = Envelope::Swell;
        }
        let w = &mut p.warp;
        (w.zoom, w.rotation, w.offset, w.drift) = (2.5, -1.0, [0.25, -0.5], 0.7);
        (w.slave_4_to_3, w.line_jitter, w.axis_wander) = (true, 3.0, 0.4);
        let c = &mut p.colorize;
        (c.levels, c.softness, c.cycle_speed, c.bypass) = (4, 0.3, 1.5, true);
        for (i, color) in c.palette.iter_mut().enumerate() {
            *color = [i as f32 / 8.0, 0.5, 1.0 - i as f32 / 8.0];
        }
        c.thresholds = [0.2, 0.5, 0.7, 1.0, 1.0, 1.0, 1.0];
        (c.bandwidth, c.ringing) = (2.5, 0.6);
        let f = &mut p.feedback;
        (f.amount, f.zoom, f.rotation, f.offset) = (0.5, 0.95, -0.05, [0.01, -0.01]);
        let g = &mut p.glow;
        (g.bloom_intensity, g.bloom_threshold) = (2.0, 0.3);
        (g.scanline_strength, g.scanline_count, g.chroma, g.noise) = (0.8, 700.0, 2.5, 0.2);
        let r = &mut p.raster;
        (r.enabled, r.lines, r.beam_width) = (true, 800, 2.0);
        (r.compensation, r.speed_compensation) = (0.5, 0.9);
        (p.key.enabled, p.key.levels) = (true, 0b1010);
        p
    }

    fn preset(params: &str) -> String {
        format!(r#"{{"format": "rasterwarp-preset", "version": 1, "params": {params}}}"#)
    }

    #[test]
    fn presets_round_trip() {
        for p in [Params::default(), changed()] {
            let loaded = read_preset(&preset_json(&p)).unwrap();
            assert_eq!(loaded.value, p);
            assert!(!loaded.newer);
        }
        assert_ne!(changed(), Params::default());
    }

    #[test]
    fn missing_fields_take_their_defaults() {
        let p = read_preset(&preset(r#"{"warp": {"zoom": 2.0}}"#))
            .unwrap()
            .value;
        let mut expected = Params::default();
        expected.warp.zoom = 2.0;
        assert_eq!(p, expected);
        let empty = r#"{"format": "rasterwarp-preset", "version": 1}"#;
        assert_eq!(read_preset(empty).unwrap().value, Params::default());
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let text = r#"{"format": "rasterwarp-preset", "version": 1, "sparkle": 3,
            "params": {"warp": {"zoom": 2.0, "wobble": [1, 2]}, "lasers": true}}"#;
        assert_eq!(read_preset(text).unwrap().value.warp.zoom, 2.0);
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let p = read_preset(&preset(
            r#"{"warp": {"zoom": 99.0}, "colorize": {"levels": 50}, "raster": {"lines": 5}}"#,
        ))
        .unwrap()
        .value;
        assert_eq!(p.warp.zoom, 4.0);
        assert_eq!(p.colorize.levels, 8);
        assert_eq!(p.raster.lines, 100);
    }

    #[test]
    fn unknown_options_fall_back_to_their_defaults() {
        let mut json: Value = serde_json::from_str(&preset_json(&changed())).unwrap();
        json["params"]["warp"]["oscillators"][0]["waveform"] = "wobble".into();
        json["params"]["warp"]["oscillators"][1]["input"] = 7.into();
        let p = read_preset(&json.to_string()).unwrap().value;
        assert_eq!(p.warp.oscillators[0].waveform, Waveform::Sine);
        assert_eq!(p.warp.oscillators[1].input, OscInput::V);
        let mut expected = changed();
        expected.warp.oscillators[0].waveform = Waveform::Sine;
        expected.warp.oscillators[1].input = OscInput::V;
        assert_eq!(p, expected, "the rest of the file still loads");
    }

    #[test]
    fn newer_versions_load_and_are_flagged() {
        let text =
            r#"{"format": "rasterwarp-preset", "version": 2, "params": {"warp": {"zoom": 2.0}}}"#;
        let loaded = read_preset(text).unwrap();
        assert!(loaded.newer);
        assert_eq!(loaded.value.warp.zoom, 2.0);
    }

    #[test]
    fn other_files_are_refused() {
        let project = r#"{"format": "rasterwarp-project", "version": 1}"#;
        let err = read_preset(project).unwrap_err();
        assert_eq!(err.to_string(), "this is a project, not a preset");
        let err = read_preset(r#"{"zoom": 2}"#).unwrap_err();
        assert_eq!(err.to_string(), "this isn't a rasterwarp file");
        let err = read_preset("zoom = 2").unwrap_err();
        assert_eq!(err.to_string(), "this isn't a JSON file");
        let damaged = preset(r#"{"warp": {"zoom": "big"}}"#);
        let err = read_preset(&damaged).unwrap_err();
        assert_eq!(err.to_string(), "some settings in the file are damaged");
    }

    #[test]
    fn writing_replaces_the_file_and_leaves_no_temporary_file() {
        let dir = temp_dir("write");
        let path = dir.join("sub").join("look.rwpreset");
        save_preset(&path, &Params::default()).unwrap();
        save_preset(&path, &changed()).unwrap();
        assert_eq!(load_preset(&path).unwrap().value, changed());
        let names: Vec<_> = fs::read_dir(dir.join("sub"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["look.rwpreset"]);
        let err = load_preset(&dir.join("missing.rwpreset")).unwrap_err();
        assert!(err.to_string().starts_with("could not read "), "{err}");
    }

    #[test]
    fn the_version_1_preset_still_loads() {
        let loaded = read_preset(include_str!("../tests/fixtures/v1.rwpreset")).unwrap();
        assert!(!loaded.newer);
        let p = loaded.value;
        assert_eq!(p.warp.zoom, 2.5);
        assert_eq!(p.warp.oscillators[0].waveform, Waveform::Square);
        assert_eq!(p.warp.oscillators[3].envelope, Envelope::Swell);
        assert_eq!(p.colorize.levels, 4);
        assert_eq!(p.colorize.palette[2], [0.25, 0.5, 0.75]);
        assert_eq!(p.raster.lines, 800);
        assert_eq!(p.key.levels, 0b1010);
    }
}
```

Create `tests/fixtures/v1.rwpreset` exactly as shown (it must never be regenerated):

```json
{
  "format": "rasterwarp-preset",
  "version": 1,
  "params": {
    "warp": {
      "oscillators": [
        {
          "waveform": "square",
          "target": "y",
          "input": "time",
          "frequency": 7.5,
          "amplitude": 0.25,
          "phase": 0.75,
          "phase_speed": -1.5,
          "lfo_rate": 3.0,
          "lfo_depth": 0.6,
          "sync": "frame",
          "envelope": "swell"
        },
        {
          "waveform": "square",
          "target": "y",
          "input": "time",
          "frequency": 7.5,
          "amplitude": 0.25,
          "phase": 0.75,
          "phase_speed": -1.5,
          "lfo_rate": 3.0,
          "lfo_depth": 0.6,
          "sync": "frame",
          "envelope": "swell"
        },
        {
          "waveform": "square",
          "target": "y",
          "input": "time",
          "frequency": 7.5,
          "amplitude": 0.25,
          "phase": 0.75,
          "phase_speed": -1.5,
          "lfo_rate": 3.0,
          "lfo_depth": 0.6,
          "sync": "frame",
          "envelope": "swell"
        },
        {
          "waveform": "square",
          "target": "y",
          "input": "time",
          "frequency": 7.5,
          "amplitude": 0.25,
          "phase": 0.75,
          "phase_speed": -1.5,
          "lfo_rate": 3.0,
          "lfo_depth": 0.6,
          "sync": "frame",
          "envelope": "swell"
        }
      ],
      "zoom": 2.5,
      "rotation": -1.0,
      "offset": [
        0.25,
        -0.5
      ],
      "drift": 0.7,
      "slave_4_to_3": true,
      "line_jitter": 3.0,
      "axis_wander": 0.4
    },
    "colorize": {
      "levels": 4,
      "softness": 0.3,
      "palette": [
        [
          0.0,
          0.5,
          1.0
        ],
        [
          0.125,
          0.5,
          0.875
        ],
        [
          0.25,
          0.5,
          0.75
        ],
        [
          0.375,
          0.5,
          0.625
        ],
        [
          0.5,
          0.5,
          0.5
        ],
        [
          0.625,
          0.5,
          0.375
        ],
        [
          0.75,
          0.5,
          0.25
        ],
        [
          0.875,
          0.5,
          0.125
        ]
      ],
      "cycle_speed": 1.5,
      "bypass": true,
      "thresholds": [
        0.2,
        0.5,
        0.7,
        1.0,
        1.0,
        1.0,
        1.0
      ],
      "bandwidth": 2.5,
      "ringing": 0.6
    },
    "feedback": {
      "amount": 0.5,
      "zoom": 0.95,
      "rotation": -0.05,
      "offset": [
        0.01,
        -0.01
      ]
    },
    "glow": {
      "bloom_intensity": 2.0,
      "bloom_threshold": 0.3,
      "scanline_strength": 0.8,
      "scanline_count": 700.0,
      "chroma": 2.5,
      "noise": 0.2
    },
    "raster": {
      "enabled": true,
      "lines": 800,
      "beam_width": 2.0,
      "compensation": 0.5,
      "speed_compensation": 0.9
    },
    "key": {
      "enabled": true,
      "levels": 10
    }
  }
}
```

- [ ] **Step 3: Run them to see them fail.** Run: `cargo test --lib`. Expected: fails to compile, with ``cannot find function `read_preset` in this scope`` (and the same for `preset_json`, `save_preset` and `load_preset`), ``no method named `clamped` found for struct `params::Params` ``, and errors for `Params`, `Value` and `fs`, which `save.rs`'s imports bring in at Step 4.

- [ ] **Step 4: Implement.**

In `src/params.rs`, replace:

```rust

/// Slider ranges. The UI uses these, and tests check that defaults fall inside them.
```

with:

```rust

use serde::{Deserialize, Serialize};

use crate::save::saved_names;

/// Slider ranges. The UI uses these, and tests check that defaults fall inside them.
```

In `src/params.rs`, replace:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waveform {
```

with:

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Waveform {
    #[default]
```

In `src/params.rs`, replace:

```rust
/// Which displacement component an oscillator adds to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
```

with:

```rust
saved_names!(Waveform {
    Sine => "sine",
    Triangle => "triangle",
    Ramp => "ramp",
    Square => "square",
    Noise => "noise",
});

/// Which displacement component an oscillator adds to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Axis {
    #[default]
```

In `src/params.rs`, replace:

```rust
/// What drives an oscillator's phase across the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
```

with:

```rust
saved_names!(Axis { X => "x", Y => "y" });

/// What drives an oscillator's phase across the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
```

In `src/params.rs`, replace:

```rust
    /// Vertical position: X displacement driven by V gives the classic per-scanline wiggle.
```

with:

```rust
    /// Vertical position: X displacement driven by V gives the classic per-scanline wiggle.
    #[default]
```

In `src/params.rs`, replace:

```rust
/// How an oscillator's phase moves over time (the manual's sync modes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OscSync {
    /// Phase advances at `phase_speed`: the wave drifts.
```

with:

```rust
saved_names!(OscInput {
    U => "u",
    V => "v",
    Radius => "radius",
    Time => "time",
});

/// How an oscillator's phase moves over time (the manual's sync modes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OscSync {
    /// Phase advances at `phase_speed`: the wave drifts.
    #[default]
```

In `src/params.rs`, replace:

```rust
/// How an oscillator's amplitude behaves while a transition or sequence ramp runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Envelope {
    /// Always at its set amplitude.
```

with:

```rust
saved_names!(OscSync {
    Free => "free",
    Frame => "frame",
});

/// How an oscillator's amplitude behaves while a transition or sequence ramp runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Envelope {
    /// Always at its set amplitude.
    #[default]
```

In `src/params.rs`, replace:

```rust

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oscillator {
```

with:

```rust

saved_names!(Envelope {
    Constant => "constant",
    Swell => "swell",
});

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Oscillator {
```

In `src/params.rs`, replace:

```rust

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WarpParams {
```

with:

```rust

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WarpParams {
```

In `src/params.rs`, replace:

```rust

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorizeParams {
```

with:

```rust

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorizeParams {
```

In `src/params.rs`, replace:

```rust
/// True raster mode: draw the source as deflected scan lines instead of warping it.
#[derive(Clone, Copy, Debug, PartialEq)]
```

with:

```rust
/// True raster mode: draw the source as deflected scan lines instead of warping it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
```

In `src/params.rs`, replace:

```rust
/// Level keying: chosen colorizer levels become see-through, showing a background image.
#[derive(Clone, Copy, Debug, PartialEq)]
```

with:

```rust
/// Level keying: chosen colorizer levels become see-through, showing a background image.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
```

In `src/params.rs`, replace:

```rust

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeedbackParams {
```

with:

```rust

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeedbackParams {
```

In `src/params.rs`, replace:

```rust

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlowParams {
```

with:

```rust

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GlowParams {
```

In `src/params.rs`, replace:

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
```

with:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
```

In `src/params.rs`, replace:

```rust

impl WarpParams {
```

with:

```rust

// Each part's default is the default look's part, so a file missing a field gets the
// value a fresh session has.
impl Default for WarpParams {
    fn default() -> Self {
        Params::default().warp
    }
}

impl Default for ColorizeParams {
    fn default() -> Self {
        Params::default().colorize
    }
}

impl Default for FeedbackParams {
    fn default() -> Self {
        Params::default().feedback
    }
}

impl Default for GlowParams {
    fn default() -> Self {
        Params::default().glow
    }
}

impl Default for RasterParams {
    fn default() -> Self {
        Params::default().raster
    }
}

impl Default for KeyParams {
    fn default() -> Self {
        Params::default().key
    }
}

fn clamp(value: &mut f32, range: RangeInclusive<f32>) {
    *value = value.clamp(*range.start(), *range.end());
}

impl Params {
    /// A copy with every value inside its slider range, for parameters read from a file.
    pub fn clamped(mut self) -> Self {
        let w = &mut self.warp;
        for o in &mut w.oscillators {
            clamp(&mut o.frequency, ranges::FREQUENCY);
            clamp(&mut o.amplitude, ranges::AMPLITUDE);
            clamp(&mut o.phase, ranges::PHASE);
            clamp(&mut o.phase_speed, ranges::PHASE_SPEED);
            clamp(&mut o.lfo_rate, ranges::LFO_RATE);
            clamp(&mut o.lfo_depth, ranges::LFO_DEPTH);
        }
        clamp(&mut w.zoom, ranges::ZOOM);
        clamp(&mut w.rotation, ranges::ROTATION);
        for v in &mut w.offset {
            clamp(v, ranges::OFFSET);
        }
        clamp(&mut w.drift, ranges::DRIFT);
        clamp(&mut w.line_jitter, ranges::LINE_JITTER);
        clamp(&mut w.axis_wander, ranges::AXIS_WANDER);

        let c = &mut self.colorize;
        c.levels = c
            .levels
            .clamp(*ranges::LEVELS.start(), *ranges::LEVELS.end());
        clamp(&mut c.softness, ranges::SOFTNESS);
        for v in c.palette.iter_mut().flatten() {
            clamp(v, 0.0..=1.0);
        }
        clamp(&mut c.cycle_speed, ranges::CYCLE_SPEED);
        for t in &mut c.thresholds {
            clamp(t, ranges::THRESHOLD);
        }
        clamp(&mut c.bandwidth, ranges::BANDWIDTH);
        clamp(&mut c.ringing, ranges::RINGING);

        let f = &mut self.feedback;
        clamp(&mut f.amount, ranges::FEEDBACK_AMOUNT);
        clamp(&mut f.zoom, ranges::FEEDBACK_ZOOM);
        clamp(&mut f.rotation, ranges::FEEDBACK_ROTATION);
        for v in &mut f.offset {
            clamp(v, ranges::FEEDBACK_OFFSET);
        }

        let g = &mut self.glow;
        clamp(&mut g.bloom_intensity, ranges::BLOOM_INTENSITY);
        clamp(&mut g.bloom_threshold, ranges::BLOOM_THRESHOLD);
        clamp(&mut g.scanline_strength, ranges::SCANLINE_STRENGTH);
        clamp(&mut g.scanline_count, ranges::SCANLINE_COUNT);
        clamp(&mut g.chroma, ranges::CHROMA);
        clamp(&mut g.noise, ranges::NOISE);

        let r = &mut self.raster;
        r.lines = r
            .lines
            .clamp(*ranges::RASTER_LINES.start(), *ranges::RASTER_LINES.end());
        clamp(&mut r.beam_width, ranges::BEAM_WIDTH);
        clamp(&mut r.compensation, ranges::COMPENSATION);
        clamp(&mut r.speed_compensation, ranges::SPEED_COMPENSATION);

        // Levels the colorizer doesn't have can't be see-through.
        self.key.keep_levels(self.colorize.levels);
        self
    }
}

impl WarpParams {
```

In `src/save.rs`, put this above the tests (the file then starts with it):

```rust
//! Saved files: presets, projects and app settings, written as JSON. Reading is tolerant:
//! a missing field takes its default, an unknown field is ignored, an unknown option
//! falls back to its default, and numbers are clamped into their ranges.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::params::Params;

/// The file format version this build writes.
pub const VERSION: u32 = 1;

pub const PRESET_FORMAT: &str = "rasterwarp-preset";
pub const PRESET_EXTENSION: &str = "rwpreset";
pub const PROJECT_FORMAT: &str = "rasterwarp-project";
pub const PROJECT_EXTENSION: &str = "rwproject";
pub const SETTINGS_FORMAT: &str = "rasterwarp-settings";

/// What was read from a file.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded<T> {
    pub value: T,
    /// A newer version of rasterwarp wrote the file, so some settings may not have loaded.
    pub newer: bool,
}

/// Saves an enum as one of the given names. An unknown name (say, from a newer version)
/// reads as the enum's default instead of failing the whole file.
macro_rules! saved_names {
    ($ty:ty { $($variant:ident => $name:literal),+ $(,)? }) => {
        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(match self {
                    $(Self::$variant => $name,)+
                })
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

/// A whole file: its format name and version, then the contents' own fields.
#[derive(Serialize)]
struct Envelope<'a, T> {
    format: &'a str,
    version: u32,
    #[serde(flatten)]
    contents: &'a T,
}

/// `contents` as a pretty-printed file of the given format.
pub fn to_json<T: Serialize>(format: &str, contents: &T) -> String {
    let file = Envelope {
        format,
        version: VERSION,
        contents,
    };
    serde_json::to_string_pretty(&file).expect("saved settings always serialize")
}

/// Reads a file of the given format.
pub fn from_json<T: DeserializeOwned>(format: &str, text: &str) -> Result<Loaded<T>> {
    let value: Value = serde_json::from_str(text).context("this isn't a JSON file")?;
    let found = value.get("format").and_then(Value::as_str).unwrap_or("");
    if found != format {
        match kind(found) {
            Some(other) => bail!("this is {other}, not {}", kind(format).unwrap_or(format)),
            None => bail!("this isn't a rasterwarp file"),
        }
    }
    let version = value.get("version").and_then(Value::as_u64).unwrap_or(1);
    let contents = T::deserialize(value).context("some settings in the file are damaged")?;
    Ok(Loaded {
        value: contents,
        newer: version > u64::from(VERSION),
    })
}

/// "a preset", "a project" or "a settings file", for messages.
fn kind(format: &str) -> Option<&'static str> {
    match format {
        PRESET_FORMAT => Some("a preset"),
        PROJECT_FORMAT => Some("a project"),
        SETTINGS_FORMAT => Some("a settings file"),
        _ => None,
    }
}

/// Writes `text` to `path` without ever leaving a half-written file: it goes to
/// `<name>.tmp` first and is then renamed over the target. Creates the folder if needed.
pub fn write_atomic(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let written = fs::write(&tmp, text).and_then(|()| fs::rename(&tmp, path));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written.with_context(|| format!("could not write {}", path.display()))
}

/// Reads a whole file, naming it in the error.
pub fn read_file(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))
}

/// A preset file's contents: one look.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Preset {
    params: Params,
}

pub fn preset_json(params: &Params) -> String {
    to_json(PRESET_FORMAT, &Preset { params: *params })
}

pub fn read_preset(text: &str) -> Result<Loaded<Params>> {
    let loaded = from_json::<Preset>(PRESET_FORMAT, text)?;
    Ok(Loaded {
        value: loaded.value.params.clamped(),
        newer: loaded.newer,
    })
}

pub fn save_preset(path: &Path, params: &Params) -> Result<()> {
    write_atomic(path, &preset_json(params))
}

pub fn load_preset(path: &Path) -> Result<Loaded<Params>> {
    read_preset(&read_file(path)?).with_context(|| format!("could not open {}", path.display()))
}

/// A fresh, empty folder for a test's files.
```

- [ ] **Step 5: Run everything.** Run: `cargo test`. Expected: `164 passed` for the unit tests, `4 passed` in `tests/capture.rs` and `15 passed` in `tests/smoke.rs`.

- [ ] **Step 6: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml src/lib.rs src/params.rs src/save.rs tests/fixtures/v1.rwpreset Cargo.lock
git commit -m "feat: preset files with tolerant reading" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Project files

A new `session` module holds a project: the mode, both transition banks, the sequence, user curves, canvas size, frame rate and image paths, without anything in motion. It takes a snapshot of the running motion and builds motion back from one, repairing what a file can get wrong (cue timing, missing curves, unknown frame rates, odd canvas sizes).

**Files:**
- Create: `src/session.rs`, `tests/fixtures/v1.rwproject`
- Modify: `src/curve.rs`, `src/lib.rs`, `src/motion.rs`, `src/rate.rs`, `src/sequence.rs`

**Interfaces:**
- Consumes: `save::{to_json, from_json, write_atomic, read_file, Loaded, PROJECT_FORMAT, saved_names!}`, `Params::clamped` (Task 1).
- Produces:
  - `session::Project { mode: Mode, transition: Transition, sequence: Option<Cues>, curves: Vec<CustomCurve>, canvas: (u32, u32), frame_rate: FrameRate, source: Option<PathBuf>, background: Option<PathBuf> }`, `session::Transition { banks: [Params; 2], on_air: usize, duration: f32, curve: CurveRef }`, `session::Cues { cues: Vec<Cue>, selected: usize, looping: bool }`; all `Clone + PartialEq + Default + serde`.
  - `Project::capture(motion: &Motion, canvas: (u32, u32), frame_rate: FrameRate, source: Option<&Path>, background: Option<&Path>) -> Project`; `Project::motion(&self) -> Motion` (at rest); `Project::checked(&self) -> Project` (repaired as opening repairs it).
  - `session::{project_json(&Project) -> String, read_project(&str) -> Result<Loaded<Project>>, save_project(&Path, &Project) -> Result<()>, load_project(&Path) -> Result<Loaded<Project>>}`.
  - `Motion::restored(mode, ab: AbState, sequence: Option<Sequence>, curves: CurveLibrary) -> Motion` (its first `take_jump()` is true); `Mode` saved as `"live"`, `"transition"`, `"sequence"`.
  - `Sequence::from_cues(cues: &[Cue], selected: usize, looping: bool) -> Sequence`; `Cue: Default + serde`.
  - `CustomCurve::checked(&self) -> CustomCurve`, `CurveLibrary::from_curves(&[CustomCurve]) -> CurveLibrary`, `CurveLibrary::resolve(CurveRef) -> CurveRef`; `CurveRef` saved as `"linear"`, `"s-curve"` or `"custom:<id>"`; `FrameRate: serde`.

- [ ] **Step 1: Write the failing tests.**

In `src/lib.rs`, replace:

```rust
pub mod sequence;
```

with:

```rust
pub mod sequence;
pub mod session;
```

Create `src/session.rs` holding only its tests for now (Step 3 puts the implementation above them):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequence::MAX_FRAME;
    use serde_json::{Value, json};

    /// A session using every part of a project.
    fn session() -> Motion {
        let mut m = Motion::new(Params::default());
        let id = m.curves.add().unwrap();
        let curve = m.curves.get_mut(id).unwrap();
        curve.name = "Snap".into();
        curve.insert(0.25, 0.9);
        m.set_mode(Mode::Sequence);
        let seq = m.sequence_mut();
        seq.add_cue();
        seq.selected_cue_mut().params.warp.zoom = 2.0;
        seq.selected_cue_mut().curve = CurveRef::Custom(id);
        seq.looping = true;
        m.set_mode(Mode::Transition);
        m.editable().colorize.levels = 3;
        m.ab.duration = 5.0;
        m.ab.curve = CurveRef::Custom(id);
        m
    }

    fn project() -> Project {
        Project::capture(
            &session(),
            (1280, 720),
            FrameRate::ntsc(24),
            Some(Path::new(r"C:\images\card.png")),
            None,
        )
    }

    fn project_with(edit: impl FnOnce(&mut Value)) -> Project {
        let mut json: Value = serde_json::from_str(&project_json(&project())).unwrap();
        edit(&mut json);
        read_project(&json.to_string()).unwrap().value
    }

    #[test]
    fn projects_round_trip() {
        let p = project();
        assert_eq!(p.mode, Mode::Transition);
        assert_eq!(p.sequence.as_ref().unwrap().cues.len(), 2);
        let loaded = read_project(&project_json(&p)).unwrap();
        assert_eq!(loaded.value, p);
        assert!(!loaded.newer);
    }

    #[test]
    fn a_restored_session_matches_and_is_at_rest() {
        let mut m = session();
        m.trigger();
        m.set_mode(Mode::Sequence);
        m.sequence_mut().run();
        let p = Project::capture(&m, (1280, 720), FrameRate::ntsc(24), None, None);
        let mut restored = p.motion();
        assert_eq!(
            Project::capture(&restored, (1280, 720), FrameRate::ntsc(24), None, None),
            p
        );
        assert!(restored.ab.ramp().is_none());
        assert!(!restored.sequence_mut().is_running());
        assert!(restored.take_jump(), "the picture jumps to the project");
    }

    #[test]
    fn missing_fields_take_their_defaults() {
        let text = r#"{"format": "rasterwarp-project", "version": 1}"#;
        assert_eq!(read_project(text).unwrap().value, Project::default());
        let p = project_with(|j| {
            j.as_object_mut().unwrap().remove("transition");
        });
        assert_eq!(p.transition, Transition::default());
    }

    #[test]
    fn missing_curves_become_linear() {
        let p = project_with(|j| j["curves"] = json!([]));
        assert_eq!(p.transition.curve, CurveRef::Linear);
        assert_eq!(p.sequence.unwrap().cues[1].curve, CurveRef::Linear);
        let p = project_with(|j| j["transition"]["curve"] = json!("wiggly"));
        assert_eq!(p.transition.curve, CurveRef::Linear);
    }

    #[test]
    fn only_eight_curves_are_kept() {
        let p = project_with(|j| {
            j["curves"] = (0..9)
                .map(|id| json!({"id": id, "name": format!("c{id}"), "points": [[0.5, 0.8]]}))
                .collect();
        });
        let ids: Vec<u32> = p.curves.iter().map(|c| c.id).collect();
        assert_eq!(ids, [0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(p.curves[0].points(), &[[0.5, 0.8]]);
    }

    #[test]
    fn bad_values_are_repaired() {
        let p = project_with(|j| {
            j["sequence"]["cues"][0]["start_frame"] = json!(40);
            j["sequence"]["cues"][1]["start_frame"] = json!(5000);
            j["sequence"]["cues"][1]["duration_frames"] = json!(0);
            j["sequence"]["selected"] = json!(9);
            j["transition"]["on_air"] = json!(5);
            j["transition"]["duration"] = json!(100.0);
            j["transition"]["banks"][1]["warp"]["zoom"] = json!(99.0);
            j["frame_rate"] = json!({"num": 7, "den": 1});
            j["canvas"] = json!([1001, 3]);
            j["curves"][0]["points"] = json!([[0.5, 9.0], [0.502, 0.1], [2.0, 0.5]]);
        });
        let seq = p.sequence.unwrap();
        let starts: Vec<u32> = seq.cues.iter().map(|c| c.start_frame).collect();
        assert_eq!(starts, [0, MAX_FRAME]);
        assert_eq!(seq.cues[1].duration_frames, 1);
        assert_eq!(seq.selected, 1);
        assert_eq!(p.transition.on_air, 1);
        assert_eq!(p.transition.duration, 30.0);
        assert_eq!(p.transition.banks[1].warp.zoom, 4.0);
        assert_eq!(p.frame_rate, FrameRate::default());
        assert_eq!(p.canvas, (1002, 64));
        assert_eq!(
            p.curves[0].points(),
            &[[0.5, 1.5]],
            "too close and outside dropped"
        );
    }

    #[test]
    fn editing_the_session_changes_the_snapshot() {
        let mut m = session();
        let before = Project::capture(&m, (1280, 720), FrameRate::default(), None, None);
        assert_eq!(
            Project::capture(&m, (1280, 720), FrameRate::default(), None, None),
            before,
            "nothing changed"
        );
        m.editable().warp.rotation = 0.5;
        let after = Project::capture(&m, (1280, 720), FrameRate::default(), None, None);
        assert_ne!(after, before);
        m.trigger();
        m.advance(1000);
        let running = Project::capture(&m, (1280, 720), FrameRate::default(), None, None);
        assert_eq!(running, after, "a running ramp isn't an edit");
    }

    #[test]
    fn a_preset_is_not_a_project() {
        let preset = save::preset_json(&Params::default());
        let err = read_project(&preset).unwrap_err();
        assert_eq!(err.to_string(), "this is a preset, not a project");
    }

    #[test]
    fn the_version_1_project_still_loads() {
        let loaded = read_project(include_str!("../tests/fixtures/v1.rwproject")).unwrap();
        assert!(!loaded.newer);
        let p = loaded.value;
        assert_eq!(p.mode, Mode::Transition);
        assert_eq!(p.transition.banks[1].colorize.levels, 3);
        assert_eq!(p.transition.duration, 5.0);
        assert_eq!(p.transition.curve, CurveRef::Custom(0));
        let seq = p.sequence.unwrap();
        assert_eq!(seq.cues[1].start_frame, 48);
        assert_eq!(seq.cues[1].params.warp.zoom, 2.0);
        assert!(seq.looping);
        assert_eq!(p.curves[0].name, "Snap");
        assert_eq!(p.curves[0].points(), &[[0.25, 0.9], [0.5, 0.5]]);
        assert_eq!(p.canvas, (1280, 720));
        assert_eq!(p.frame_rate, FrameRate::ntsc(24));
        assert_eq!(p.source.as_deref(), Some(Path::new(r"C:\images\card.png")));
        assert_eq!(p.background, None);
    }
}
```

Create `tests/fixtures/v1.rwproject` exactly as shown (it must never be regenerated):

```json
{
  "format": "rasterwarp-project",
  "version": 1,
  "mode": "transition",
  "transition": {
    "banks": [
      {},
      { "colorize": { "levels": 3 } }
    ],
    "on_air": 0,
    "duration": 5.0,
    "curve": "custom:0"
  },
  "sequence": {
    "cues": [
      { "start_frame": 0, "duration_frames": 48, "curve": "s-curve" },
      {
        "params": { "warp": { "zoom": 2.0 } },
        "start_frame": 48,
        "duration_frames": 48,
        "curve": "custom:0"
      }
    ],
    "selected": 1,
    "looping": true
  },
  "curves": [
    { "id": 0, "name": "Snap", "points": [[0.25, 0.9], [0.5, 0.5]] }
  ],
  "canvas": [1280, 720],
  "frame_rate": { "num": 24000, "den": 1001 },
  "source": "C:\\images\\card.png",
  "background": null
}
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test --lib`. Expected: fails to compile, with ``cannot find type `Project` in this scope``, ``cannot find function `read_project` in this scope``, and the same for `Transition`, `project_json`, `Mode`, `Motion`, `CurveRef`, `FrameRate`, `Params`, `Path` and `save`, which `session.rs`'s imports bring in at Step 3.

- [ ] **Step 3: Implement.**

In `src/curve.rs`, replace:

```rust

/// Allowed y range for user curve points (overshoot and anticipation).
```

with:

```rust

use serde::{Deserialize, Serialize};

/// Allowed y range for user curve points (overshoot and anticipation).
```

In `src/curve.rs`, replace:

```rust
/// A user-drawn curve through (0,0), its interior points, and (1,1).
#[derive(Clone, Debug, PartialEq)]
```

with:

```rust
impl Serialize for CurveRef {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            CurveRef::Linear => s.serialize_str("linear"),
            CurveRef::SCurve => s.serialize_str("s-curve"),
            CurveRef::Custom(id) => s.serialize_str(&format!("custom:{id}")),
        }
    }
}

/// An unknown curve name reads as Linear.
impl<'de> Deserialize<'de> for CurveRef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        let name = value.as_str().unwrap_or("");
        Ok(match name {
            "s-curve" => CurveRef::SCurve,
            _ => name
                .strip_prefix("custom:")
                .and_then(|id| id.parse().ok())
                .map_or(CurveRef::Linear, CurveRef::Custom),
        })
    }
}

/// A user-drawn curve through (0,0), its interior points, and (1,1).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
```

In `src/curve.rs`, replace:

```rust

    pub fn points(&self) -> &[[f32; 2]] {
```

with:

```rust

    /// A copy of a curve read from a file, keeping only the points `insert` would accept.
    pub fn checked(&self) -> Self {
        let mut curve = Self {
            id: self.id,
            name: self.name.clone(),
            points: Vec::new(),
        };
        if curve.name.trim().is_empty() {
            curve.name = format!("Curve {}", self.id + 1);
        }
        for &[x, y] in &self.points {
            curve.insert(x, y);
        }
        curve
    }

    pub fn points(&self) -> &[[f32; 2]] {
```

In `src/curve.rs`, replace:

```rust
impl CurveLibrary {
```

with:

```rust
impl CurveLibrary {
    /// A library holding `curves` (from a file): at most [`MAX_CUSTOM`], each id once,
    /// each curve checked.
    pub fn from_curves(curves: &[CustomCurve]) -> Self {
        let mut library = Self::default();
        for curve in curves {
            if library.custom.len() < MAX_CUSTOM && library.get(curve.id).is_none() {
                library.custom.push(curve.checked());
            }
        }
        library.next_id = library.custom.iter().map(|c| c.id + 1).max().unwrap_or(0);
        library
    }

    /// `curve`, or Linear if it names a user curve that isn't in the library.
    pub fn resolve(&self, curve: CurveRef) -> CurveRef {
        match curve {
            CurveRef::Custom(id) if self.get(id).is_none() => CurveRef::Linear,
            _ => curve,
        }
    }
```

In `src/motion.rs`, replace:

```rust
use crate::sequence::{SeqEvent, SeqView, Sequence};
use crate::transition::{AbEvent, AbState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The panel edits what's on screen.
```

with:

```rust
use crate::save::saved_names;
use crate::sequence::{SeqEvent, SeqView, Sequence};
use crate::transition::{AbEvent, AbState};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// The panel edits what's on screen.
    #[default]
```

In `src/motion.rs`, replace:

```rust

/// What the off-air preview is showing.
```

with:

```rust

saved_names!(Mode {
    Live => "live",
    Transition => "transition",
    Sequence => "sequence",
});

/// What the off-air preview is showing.
```

In `src/motion.rs`, replace:

```rust

    pub fn mode(&self) -> Mode {
```

with:

```rust

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
```

In `src/rate.rs`, replace:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
```

with:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
```

In `src/sequence.rs`, replace:

```rust

use crate::curve::CurveRef;
```

with:

```rust

use serde::{Deserialize, Serialize};

use crate::curve::CurveRef;
```

In `src/sequence.rs`, replace:

```rust

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cue {
```

with:

```rust

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cue {
```

In `src/sequence.rs`, replace:

```rust

/// What happened during one `advance`.
```

with:

```rust

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
```

In `src/sequence.rs`, replace:

```rust
                start_frame: 0,
                duration_frames: 48,
                curve: CurveRef::SCurve,
```

with:

```rust
                ..Cue::default()
```

In `src/sequence.rs`, replace:

```rust

    pub fn cues(&self) -> &[Cue] {
```

with:

```rust

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
```

In `src/session.rs`, put this above the tests (the file then starts with it):

```rust
//! A project: the whole session in one file. It holds the mode, both transition banks,
//! the sequence, user curves, canvas size, frame rate and image paths, but nothing in
//! motion (ramp progress, the sequence's position, phases, trails).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::canvas;
use crate::curve::{CurveLibrary, CurveRef, CustomCurve};
use crate::motion::{Mode, Motion};
use crate::params::Params;
use crate::rate::FrameRate;
use crate::save::{self, Loaded, PROJECT_FORMAT};
use crate::sequence::{Cue, Sequence};
use crate::transition::{AbState, DURATION};

/// Everything a project file holds. Two snapshots are equal exactly when saving would
/// write the same file, which is how unsaved changes are found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub mode: Mode,
    pub transition: Transition,
    pub sequence: Option<Cues>,
    pub curves: Vec<CustomCurve>,
    /// Canvas width and height.
    pub canvas: (u32, u32),
    pub frame_rate: FrameRate,
    /// The source image, or none for the built-in test card.
    pub source: Option<PathBuf>,
    /// The keying background image, if any.
    pub background: Option<PathBuf>,
}

/// The A/B banks without a running ramp.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transition {
    pub banks: [Params; 2],
    pub on_air: usize,
    pub duration: f32,
    pub curve: CurveRef,
}

/// The sequence's cues without its running position.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cues {
    pub cues: Vec<Cue>,
    pub selected: usize,
    pub looping: bool,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            mode: Mode::Live,
            transition: Transition::default(),
            sequence: None,
            curves: Vec::new(),
            canvas: canvas::DEFAULT,
            frame_rate: FrameRate::default(),
            source: None,
            background: None,
        }
    }
}

impl Default for Transition {
    fn default() -> Self {
        let ab = AbState::new(Params::default());
        Self {
            banks: ab.banks,
            on_air: ab.on_air,
            duration: ab.duration,
            curve: ab.curve,
        }
    }
}

impl Default for Cues {
    fn default() -> Self {
        let seq = Sequence::new(Params::default());
        Self {
            cues: seq.cues().to_vec(),
            selected: seq.selected,
            looping: seq.looping,
        }
    }
}

impl Project {
    /// A snapshot of the running session.
    pub fn capture(
        motion: &Motion,
        canvas: (u32, u32),
        frame_rate: FrameRate,
        source: Option<&Path>,
        background: Option<&Path>,
    ) -> Self {
        let ab = &motion.ab;
        Self {
            mode: motion.mode(),
            transition: Transition {
                banks: ab.banks,
                on_air: ab.on_air,
                duration: ab.duration,
                curve: ab.curve,
            },
            sequence: motion.sequence.as_ref().map(|seq| Cues {
                cues: seq.cues().to_vec(),
                selected: seq.selected,
                looping: seq.looping,
            }),
            curves: motion.curves.custom.clone(),
            canvas,
            frame_rate,
            source: source.map(Path::to_path_buf),
            background: background.map(Path::to_path_buf),
        }
    }

    /// The motion this project describes, at rest. The canvas, frame rate and images are
    /// the app's to apply.
    pub fn motion(&self) -> Motion {
        let curves = CurveLibrary::from_curves(&self.curves);
        let t = &self.transition;
        let mut ab = AbState::new(Params::default());
        ab.banks = t.banks.map(Params::clamped);
        ab.on_air = t.on_air.min(1);
        ab.duration = t.duration.clamp(*DURATION.start(), *DURATION.end());
        ab.curve = curves.resolve(t.curve);
        let sequence = self.sequence.as_ref().map(|s| {
            let cues: Vec<Cue> = s
                .cues
                .iter()
                .map(|c| Cue {
                    curve: curves.resolve(c.curve),
                    ..*c
                })
                .collect();
            Sequence::from_cues(&cues, s.selected, s.looping)
        });
        Motion::restored(self.mode, ab, sequence, curves)
    }

    /// A copy repaired the way opening it repairs it, so a file's oddities (an unknown
    /// frame rate, a missing curve, cues out of order) show as they will run.
    pub fn checked(&self) -> Self {
        let rate = if FrameRate::ALL.contains(&self.frame_rate) {
            self.frame_rate
        } else {
            FrameRate::default()
        };
        Self::capture(
            &self.motion(),
            canvas::sanitize(self.canvas, u32::MAX),
            rate,
            self.source.as_deref(),
            self.background.as_deref(),
        )
    }
}

pub fn project_json(project: &Project) -> String {
    save::to_json(PROJECT_FORMAT, project)
}

/// Reads a project, repaired with [`Project::checked`].
pub fn read_project(text: &str) -> Result<Loaded<Project>> {
    let loaded = save::from_json::<Project>(PROJECT_FORMAT, text)?;
    Ok(Loaded {
        value: loaded.value.checked(),
        newer: loaded.newer,
    })
}

pub fn save_project(path: &Path, project: &Project) -> Result<()> {
    save::write_atomic(path, &project_json(project))
}

pub fn load_project(path: &Path) -> Result<Loaded<Project>> {
    read_project(&save::read_file(path)?)
        .with_context(|| format!("could not open {}", path.display()))
}
```

- [ ] **Step 4: Run everything.** Run: `cargo test`. Expected: `173 passed` for the unit tests, `4 passed` in `tests/capture.rs` and `15 passed` in `tests/smoke.rs`.

- [ ] **Step 5: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 6: Commit**

```bash
git add src/curve.rs src/lib.rs src/motion.rs src/rate.rs src/sequence.rs src/session.rs tests/fixtures/v1.rwproject
git commit -m "feat: project files" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: The presets folder

A `presets` module lists, saves, renames and deletes the `.rwpreset` files in a folder, and checks preset names.

**Files:**
- Create: `src/presets.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `save::{save_preset, load_preset, Loaded, PRESET_EXTENSION, temp_dir}` (Task 1).
- Produces: `presets::{check_name(&str) -> Result<&str, &'static str>, path(folder: &Path, name: &str) -> PathBuf, list(folder: &Path) -> Vec<String>, exists(folder, name) -> bool, save(folder, name, &Params) -> Result<()>, load(folder, name) -> Result<Loaded<Params>>, rename(folder, from, to) -> Result<()>, delete(folder, name) -> Result<()>}`.

- [ ] **Step 1: Write the failing tests.**

In `src/lib.rs`, replace:

```rust
pub mod passes;
```

with:

```rust
pub mod passes;
pub mod presets;
```

Create `src/presets.rs` holding only its tests for now (Step 3 puts the implementation above them):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::temp_dir;

    #[test]
    fn names_are_trimmed_and_checked() {
        assert_eq!(check_name("  Night drive "), Ok("Night drive"));
        assert!(check_name("   ").is_err());
        for bad in ["a/b", "a\\b", "c:", "why?", "\"q\"", "<x>", "a|b", "*"] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn presets_are_listed_by_name_ignoring_case() {
        let dir = temp_dir("list");
        assert!(list(&dir.join("missing")).is_empty());
        for name in ["beta", "Alpha", "gamma"] {
            save(&dir, name, &Params::default()).unwrap();
        }
        fs::write(dir.join("notes.txt"), "not a preset").unwrap();
        fs::create_dir(dir.join("folder.rwpreset")).unwrap();
        assert_eq!(list(&dir), ["Alpha", "beta", "gamma"]);
    }

    #[test]
    fn presets_save_load_rename_and_delete() {
        let dir = temp_dir("presets").join("new folder");
        let mut look = Params::default();
        look.warp.zoom = 3.0;
        save(&dir, " Wide ", &look).unwrap();
        assert!(exists(&dir, "Wide"));
        assert_eq!(load(&dir, "Wide").unwrap().value, look);

        save(&dir, "Other", &Params::default()).unwrap();
        let err = rename(&dir, "Wide", "Other").unwrap_err();
        assert_eq!(err.to_string(), "A preset named Other already exists.");
        assert!(rename(&dir, "Wide", "a/b").is_err());
        rename(&dir, "Wide", "Zoomed").unwrap();
        rename(&dir, "Zoomed", "ZOOMED").unwrap();
        assert_eq!(list(&dir), ["Other", "ZOOMED"]);
        assert_eq!(load(&dir, "ZOOMED").unwrap().value, look);

        delete(&dir, "Other").unwrap();
        assert_eq!(list(&dir), ["ZOOMED"]);
        assert!(delete(&dir, "Other").is_err());
    }
}
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test --lib`. Expected: fails to compile, with ``cannot find function `check_name` in this scope`` and the same for `save`, `list`, `exists`, `load`, `rename` and `delete`, plus `Params` and `fs`, which `presets.rs`'s imports bring in at Step 3.

- [ ] **Step 3: Implement.**

In `src/presets.rs`, put this above the tests (the file then starts with it):

```rust
//! The presets folder: one `<name>.rwpreset` file per look, listed by name.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::params::Params;
use crate::save::{self, Loaded, PRESET_EXTENSION};

/// Characters Windows doesn't allow in file names.
const FORBIDDEN: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

/// The name a preset would be saved under (trimmed), or why it can't be used.
pub fn check_name(name: &str) -> Result<&str, &'static str> {
    let name = name.trim();
    if name.is_empty() {
        Err("Type a name for the preset.")
    } else if name.contains(FORBIDDEN) {
        Err("Names can't contain \\ / : * ? \" < > |")
    } else {
        Ok(name)
    }
}

pub fn path(folder: &Path, name: &str) -> PathBuf {
    folder.join(format!("{name}.{PRESET_EXTENSION}"))
}

/// The presets in `folder`, sorted by name ignoring case. A missing folder has none.
pub fn list(folder: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let ext = path.extension()?.to_str()?;
            if !path.is_file() || !ext.eq_ignore_ascii_case(PRESET_EXTENSION) {
                return None;
            }
            Some(path.file_stem()?.to_str()?.to_owned())
        })
        .collect();
    names.sort_by_key(|name| (name.to_lowercase(), name.clone()));
    names
}

pub fn exists(folder: &Path, name: &str) -> bool {
    path(folder, name).is_file()
}

/// Saves `params` as preset `name`, replacing any preset of that name. Creates the
/// folder if needed.
pub fn save(folder: &Path, name: &str, params: &Params) -> Result<()> {
    let name = check_name(name).map_err(anyhow::Error::msg)?;
    save::save_preset(&path(folder, name), params)
}

pub fn load(folder: &Path, name: &str) -> Result<Loaded<Params>> {
    save::load_preset(&path(folder, name))
}

/// Renames preset `from` to `to`. Refuses a name another preset already has.
pub fn rename(folder: &Path, from: &str, to: &str) -> Result<()> {
    let to = check_name(to).map_err(anyhow::Error::msg)?;
    // Changing only the case renames the same file.
    if exists(folder, to) && !to.eq_ignore_ascii_case(from) {
        bail!("A preset named {to} already exists.");
    }
    fs::rename(path(folder, from), path(folder, to))
        .with_context(|| format!("could not rename {from} to {to}"))
}

pub fn delete(folder: &Path, name: &str) -> Result<()> {
    fs::remove_file(path(folder, name)).with_context(|| format!("could not delete {name}"))
}
```

- [ ] **Step 4: Run everything.** Run: `cargo test`. Expected: `176 passed` for the unit tests, `4 passed` in `tests/capture.rs` and `15 passed` in `tests/smoke.rs`.

- [ ] **Step 5: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/presets.rs
git commit -m "feat: presets folder" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: App settings, autosave and restore

App settings (presets folder, capture options, Show preview) and the autosave live in `%APPDATA%\rasterwarp`. The app restores the last session on launch, autosaves every 60 seconds when something changed and when the window closes, and now remembers which image files the session uses.

**Files:**
- Modify: `src/app.rs`, `src/capture/encode.rs`, `src/capture/mod.rs`, `src/capture/recorder.rs`, `src/save.rs`, `src/session.rs`, `src/ui.rs`

**Interfaces:**
- Consumes: `Project`, `Project::{capture, motion}`, `session::{save_project, load_project}` (Task 2); `save::*` (Task 1).
- Produces:
  - `save::{data_dir() -> PathBuf, Settings { presets_folder: PathBuf, capture: RecordSettings, show_preview: bool }, load_settings(dir: &Path) -> Settings, save_settings(dir: &Path, &Settings) -> Result<()>, SETTINGS_FILE ("settings.json"), NEWER_NOTE}`.
  - `session::{Autosave { file: Option<PathBuf>, unsaved: bool, project: Project (flattened) }, AUTOSAVE_FILE ("autosave.rwproject"), BAD_AUTOSAVE_FILE ("autosave.bad.rwproject"), save_autosave(dir: &Path, &Autosave) -> Result<()>, restore_autosave(dir: &Path) -> Result<Option<Loaded<Autosave>>>}`.
  - `RecordSettings: Default + PartialEq + serde`; `VideoFormat` saved as `"hevc"`/`"ffv1"`, `CaptureMode` as `"real-time"`/`"offline"`.
  - `UiState.{file_error: Option<String>, file_note: Option<String>, presets_folder: PathBuf}`, shown under the header.
  - In `app.rs`: `State.{data_dir, source_path, background_path, project_file, saved: Option<Project>, autosaved, autosave_failed, last_autosave, settings_written}` and the methods `start_session`, `apply_project(&Project)`, `project() -> Project`, `unsaved(&Project) -> bool`, `settings() -> Settings`, `autosave()`, `open_source(&Path) -> Result<()>`, `show_test_card()`, `open_background(&Path) -> Result<()>`, `clear_feedback()`.

- [ ] **Step 1: Write the failing tests.**

In `src/save.rs`, replace:

```rust
    }

    #[test]
    fn the_version_1_preset_still_loads() {
```

with:

```rust
    }

    #[test]
    fn settings_round_trip_and_fall_back_to_defaults() {
        let dir = temp_dir("settings");
        assert_eq!(load_settings(&dir), Settings::default());
        let mut settings = Settings {
            presets_folder: PathBuf::from(r"D:\looks"),
            show_preview: false,
            ..Settings::default()
        };
        settings.capture.folder = PathBuf::from(r"D:\video");
        settings.capture.format = crate::capture::encode::VideoFormat::Ffv1;
        settings.capture.mode = crate::capture::CaptureMode::Offline;
        settings.capture.stop_after = 12.5;
        save_settings(&dir, &settings).unwrap();
        assert_eq!(load_settings(&dir), settings);
        fs::write(dir.join(SETTINGS_FILE), "{ broken").unwrap();
        assert_eq!(load_settings(&dir), Settings::default());
    }

    #[test]
    fn data_lives_in_appdata() {
        assert_eq!(
            data_dir_in(Some(OsStr::new(r"C:\Users\me\AppData\Roaming"))),
            Path::new(r"C:\Users\me\AppData\Roaming\rasterwarp")
        );
        assert_eq!(data_dir_in(None), Path::new("rasterwarp-data"));
        assert_eq!(
            data_dir_in(Some(OsStr::new(""))),
            Path::new("rasterwarp-data")
        );
    }

    #[test]
    fn the_version_1_preset_still_loads() {
```

In `src/session.rs`, replace:

```rust
    }

    #[test]
    fn the_version_1_project_still_loads() {
```

with:

```rust
    }

    #[test]
    fn the_autosave_keeps_its_file_and_unsaved_flag() {
        let dir = save::temp_dir("autosave");
        assert!(restore_autosave(&dir).unwrap().is_none());
        let autosave = Autosave {
            file: Some(PathBuf::from(r"C:\shows\night.rwproject")),
            unsaved: true,
            project: project(),
        };
        save_autosave(&dir, &autosave).unwrap();
        let restored = restore_autosave(&dir).unwrap().unwrap();
        assert_eq!(restored.value, autosave);
        let as_project = load_project(&dir.join(AUTOSAVE_FILE)).unwrap();
        assert_eq!(
            as_project.value,
            project(),
            "an autosave opens as a project"
        );
    }

    #[test]
    fn a_bad_autosave_is_kept_aside() {
        let dir = save::temp_dir("bad-autosave");
        std::fs::write(dir.join(AUTOSAVE_FILE), "{ older and broken").unwrap();
        assert!(restore_autosave(&dir).is_err());
        std::fs::write(dir.join(AUTOSAVE_FILE), "{ broken").unwrap();
        let err = restore_autosave(&dir).unwrap_err();
        assert_eq!(err.to_string(), "this isn't a JSON file");
        assert!(!dir.join(AUTOSAVE_FILE).exists());
        let kept = std::fs::read_to_string(dir.join(BAD_AUTOSAVE_FILE)).unwrap();
        assert_eq!(
            kept, "{ broken",
            "the newest bad file replaces the older one"
        );
        assert!(restore_autosave(&dir).unwrap().is_none());
    }

    #[test]
    fn the_version_1_project_still_loads() {
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test --lib`. Expected: fails to compile, with ``cannot find function `restore_autosave` in this scope``, ``cannot find type `Settings` in this scope`` and the same for `load_settings`, `save_settings`, `SETTINGS_FILE`, `data_dir_in`, `OsStr`, `Autosave`, `save_autosave`, `AUTOSAVE_FILE` and `BAD_AUTOSAVE_FILE`.

- [ ] **Step 3: Implement.**

In `src/app.rs`, replace:

```rust
use std::time::Instant;
```

with:

```rust
use std::time::{Duration, Instant};
```

In `src/app.rs`, replace:

```rust
use crate::source::{self, ColorImage, GrayImage};
use crate::ui::{self, UiActions, UiState};
```

with:

```rust
use crate::save::{self, Settings};
use crate::session::{self, Autosave, Project};
use crate::source::{self, ColorImage, GrayImage};
use crate::ui::{self, UiActions, UiState};

/// How often the session is autosaved (when it changed).
const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(60);
```

In `src/app.rs`, replace:

```rust
                state.stop_recording(); // finish the file before exiting
```

with:

```rust
                state.stop_recording(); // finish the file before exiting
                state.autosave();
```

In `src/app.rs`, replace:

```rust
    last_refresh: Instant,
```

with:

```rust
    last_refresh: Instant,
    /// Where the autosave and settings live.
    data_dir: PathBuf,
    /// The source and background image files, as a project saves them. A project's
    /// missing image keeps its path here, so saving doesn't drop it.
    source_path: Option<PathBuf>,
    background_path: Option<PathBuf>,
    /// The named project file the session belongs to; none while Untitled.
    project_file: Option<PathBuf>,
    /// The session as last saved or opened; none when it matches no file.
    saved: Option<Project>,
    /// What the autosave file holds.
    autosaved: Option<Autosave>,
    /// The last autosave failed (reported once until one succeeds).
    autosave_failed: bool,
    last_autosave: Instant,
    /// The settings as last written.
    settings_written: Settings,
```

In `src/app.rs`, replace:

```rust
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
```

with:

```rust
        let data_dir = save::data_dir();
        let settings = save::load_settings(&data_dir);
        let mut ui = UiState {
            frame_ms: 16.7,
            show_preview: settings.show_preview,
            canvas: CanvasChoice::new(canvas::DEFAULT, max_texture_side),
            presets_folder: settings.presets_folder.clone(),
            ..Default::default()
        };
        ui.capture.settings = settings.capture.clone();
        let image = test_card(&mut ui);
```

In `src/app.rs`, replace:

```rust
        Ok(Self {
```

with:

```rust
        let mut state = Self {
```

In `src/app.rs`, replace:

```rust
            last_refresh: epoch,
        })
```

with:

```rust
            last_refresh: epoch,
            data_dir,
            source_path: None,
            background_path: None,
            project_file: None,
            saved: None,
            autosaved: None,
            autosave_failed: false,
            last_autosave: epoch,
            settings_written: settings,
        };
        state.start_session(initial_image);
        Ok(state)
    }

    /// Picks up the last session from the autosave, if there is one, then loads the image
    /// named on the command line over it.
    fn start_session(&mut self, initial_image: Option<&Path>) {
        match session::restore_autosave(&self.data_dir) {
            Ok(Some(loaded)) => {
                let autosave = loaded.value;
                self.apply_project(&autosave.project);
                self.project_file = autosave.file.clone();
                let current = self.project();
                self.saved = (!autosave.unsaved).then_some(current);
                if loaded.newer {
                    self.ui.file_note = Some(save::NEWER_NOTE.into());
                }
                self.autosaved = Some(autosave);
            }
            Ok(None) => self.saved = Some(self.project()),
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.file_error = Some(format!(
                    "Couldn't restore the last session (kept as {}): {err:#}",
                    session::BAD_AUTOSAVE_FILE
                ));
                self.saved = Some(self.project());
            }
        }
        if let Some(path) = initial_image {
            self.load_source(path);
        }
    }

    /// Makes `project` the running session, at rest: its motion, canvas size, frame rate
    /// and images. A missing image is reported and the test card (or no background) is
    /// shown instead, but its path stays in the session.
    fn apply_project(&mut self, project: &Project) {
        self.motion = project.motion();
        if self.recorder.is_none() {
            let size = canvas::sanitize(project.canvas, self.max_texture_side);
            if size != self.renderer.size() {
                self.renderer.resize(&self.device, size);
                self.preview
                    .resize(&self.device, &mut self.egui_renderer, size);
            }
            self.ui.canvas = CanvasChoice::new(size, self.max_texture_side);
            self.ui.rate = project.frame_rate;
        }
        let mut problems = Vec::new();
        match &project.source {
            Some(path) => {
                if let Err(err) = self.open_source(path) {
                    problems.push(image_problem("Source", path, &err));
                    self.show_test_card();
                }
            }
            None => self.show_test_card(),
        }
        match &project.background {
            Some(path) => {
                if let Err(err) = self.open_background(path) {
                    problems.push(image_problem("Background", path, &err));
                    self.set_background(None);
                }
            }
            None => self.set_background(None),
        }
        self.source_path = project.source.clone();
        self.background_path = project.background.clone();
        self.ui.load_error = (!problems.is_empty()).then(|| problems.join("\n"));
        self.clear_feedback();
        // Loading images stalls; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
    }

    /// A snapshot of the session as a project.
    fn project(&self) -> Project {
        Project::capture(
            &self.motion,
            self.ui.canvas.current,
            self.ui.rate,
            self.source_path.as_deref(),
            self.background_path.as_deref(),
        )
    }

    /// Whether `current` differs from the project file (or, Untitled, from how the
    /// session started).
    fn unsaved(&self, current: &Project) -> bool {
        self.saved.as_ref() != Some(current)
    }

    fn settings(&self) -> Settings {
        Settings {
            presets_folder: self.ui.presets_folder.clone(),
            capture: self.ui.capture.settings.clone(),
            show_preview: self.ui.show_preview,
        }
    }

    /// Writes the session to the autosave file, and the settings to theirs, if they
    /// changed since they were last written.
    fn autosave(&mut self) {
        self.last_autosave = Instant::now();
        let project = self.project();
        let autosave = Autosave {
            file: self.project_file.clone(),
            unsaved: self.unsaved(&project),
            project,
        };
        if self.autosaved.as_ref() != Some(&autosave) {
            match session::save_autosave(&self.data_dir, &autosave) {
                Ok(()) => {
                    self.autosaved = Some(autosave);
                    self.autosave_failed = false;
                }
                Err(err) => {
                    log::warn!("{err:#}");
                    if !self.autosave_failed {
                        self.ui.file_error = Some(format!("Autosave failed: {err:#}"));
                    }
                    self.autosave_failed = true;
                }
            }
        }
        let settings = self.settings();
        if settings != self.settings_written {
            match save::save_settings(&self.data_dir, &settings) {
                Ok(()) => self.settings_written = settings,
                Err(err) => log::warn!("{err:#}"),
            }
        }
```

In `src/app.rs`, replace:

```rust
    fn load_source(&mut self, path: &Path) {
        match load_fitting(path, self.max_texture_side) {
            Ok(image) => {
                self.renderer.set_source(&self.device, &self.queue, &image);
                self.preview.set_source(&self.device, &self.queue, &image);
                self.ui.source_info = describe(&path.display().to_string(), &image);
```

with:

```rust
    /// Loads a dropped (or command-line) image as the source.
    fn load_source(&mut self, path: &Path) {
        match self.open_source(path) {
            Ok(()) => {
                self.source_path = Some(absolute(path));
```

In `src/app.rs`, replace:

```rust

    /// Asks for a background image for keyed levels and shows it.
```

with:

```rust

    /// Shows the image at `path` as the source.
    fn open_source(&mut self, path: &Path) -> Result<()> {
        let image = load_fitting(path, self.max_texture_side)?;
        self.renderer.set_source(&self.device, &self.queue, &image);
        self.preview.set_source(&self.device, &self.queue, &image);
        self.ui.source_info = describe(&path.display().to_string(), &image);
        Ok(())
    }

    fn show_test_card(&mut self) {
        let image = test_card(&mut self.ui);
        self.renderer.set_source(&self.device, &self.queue, &image);
        self.preview.set_source(&self.device, &self.queue, &image);
    }

    /// Asks for a background image for keyed levels and shows it.
```

In `src/app.rs`, replace:

```rust
            match load_background(&path, self.max_texture_side) {
                Ok(image) => {
                    self.set_background(Some(&image));
                    self.ui.background_info = Some(format!(
                        "Background: {} ({}×{})",
                        path.display(),
                        image.width,
                        image.height
                    ));
```

with:

```rust
            match self.open_background(&path) {
                Ok(()) => {
                    self.background_path = Some(absolute(&path));
```

In `src/app.rs`, replace:

```rust

    fn set_background(&mut self, image: Option<&ColorImage>) {
```

with:

```rust

    /// Shows the image at `path` behind see-through levels.
    fn open_background(&mut self, path: &Path) -> Result<()> {
        let image = load_background(path, self.max_texture_side)?;
        self.set_background(Some(&image));
        self.ui.background_info = Some(format!(
            "Background: {} ({}×{})",
            path.display(),
            image.width,
            image.height
        ));
        Ok(())
    }

    fn set_background(&mut self, image: Option<&ColorImage>) {
```

In `src/app.rs`, replace:

```rust

    /// Wall-clock seconds since the app started, for the canvas clock.
```

with:

```rust

    fn clear_feedback(&mut self) {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.renderer.clear_feedback(&mut encoder);
        self.preview.clear_feedback(&mut encoder);
        self.queue.submit([encoder.finish()]);
    }

    /// Wall-clock seconds since the app started, for the canvas clock.
```

In `src/app.rs`, replace:

```rust
            self.set_background(None);
        }
```

with:

```rust
            self.set_background(None);
            self.background_path = None;
        }
```

In `src/app.rs`, replace:

```rust
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.renderer.clear_feedback(&mut encoder);
            self.preview.clear_feedback(&mut encoder);
            self.queue.submit([encoder.finish()]);
```

with:

```rust
            self.clear_feedback();
        }
        if self.last_autosave.elapsed() >= AUTOSAVE_INTERVAL {
            self.autosave();
```

In `src/app.rs`, replace:

```rust

fn describe(name: &str, image: &GrayImage) -> String {
```

with:

```rust

/// What to say about a project's image that didn't load.
fn image_problem(kind: &str, path: &Path, err: &anyhow::Error) -> String {
    if path.exists() {
        format!("{err:#}")
    } else {
        format!("{kind} image not found: {}", path.display())
    }
}

/// `path` made absolute, so a saved project finds it from any working directory.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn describe(name: &str, image: &GrayImage) -> String {
```

In `src/capture/encode.rs`, replace:

```rust
use crate::rate::FrameRate;
```

with:

```rust
use crate::rate::FrameRate;
use crate::save::saved_names;
```

In `src/capture/encode.rs`, replace:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoFormat {
    /// HEVC 4:4:4 on the NVIDIA hardware encoder, near-lossless, in fragmented MP4.
```

with:

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoFormat {
    /// HEVC 4:4:4 on the NVIDIA hardware encoder, near-lossless, in fragmented MP4.
    #[default]
```

In `src/capture/encode.rs`, replace:

```rust

impl VideoFormat {
```

with:

```rust

saved_names!(VideoFormat {
    Hevc => "hevc",
    Ffv1 => "ffv1",
});

impl VideoFormat {
```

In `src/capture/mod.rs`, replace:

```rust
use crate::rate::FrameRate;
```

with:

```rust
use crate::rate::FrameRate;
use crate::save::saved_names;
```

In `src/capture/mod.rs`, replace:

```rust

impl CaptureMode {
```

with:

```rust

saved_names!(CaptureMode {
    RealTime => "real-time",
    Offline => "offline",
});

impl CaptureMode {
```

In `src/capture/recorder.rs`, replace:

```rust
use anyhow::{Context, Result, bail};
```

with:

```rust
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
```

In `src/capture/recorder.rs`, replace:

```rust
/// What the panel chose before pressing Record.
#[derive(Clone, Debug)]
```

with:

```rust
/// What the panel chose before pressing Record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
```

In `src/capture/recorder.rs`, replace:

```rust

/// Progress shown while recording.
```

with:

```rust

impl Default for RecordSettings {
    fn default() -> Self {
        Self {
            format: VideoFormat::Hevc,
            mode: CaptureMode::RealTime,
            folder: PathBuf::from("captures"),
            stop_after: 0.0,
        }
    }
}

/// Progress shown while recording.
```

In `src/save.rs`, replace:

```rust

use std::fs;
```

with:

```rust

use std::ffi::OsStr;
use std::fs;
```

In `src/save.rs`, replace:

```rust

use crate::params::Params;
```

with:

```rust

use crate::capture::recorder::RecordSettings;
use crate::params::Params;
```

In `src/save.rs`, replace:

```rust
pub const SETTINGS_FORMAT: &str = "rasterwarp-settings";
```

with:

```rust
pub const SETTINGS_FORMAT: &str = "rasterwarp-settings";
pub const SETTINGS_FILE: &str = "settings.json";

/// Shown when a file from a newer version was opened.
pub const NEWER_NOTE: &str =
    "Saved by a newer version of rasterwarp; some settings may not have loaded.";
```

In `src/save.rs`, replace:

```rust

/// A fresh, empty folder for a test's files.
```

with:

```rust

/// Where the autosave and settings live: `%APPDATA%\rasterwarp`.
pub fn data_dir() -> PathBuf {
    data_dir_in(std::env::var_os("APPDATA").as_deref())
}

/// [`data_dir`] for a given `APPDATA`; without one, `rasterwarp-data` in the working
/// directory.
fn data_dir_in(appdata: Option<&OsStr>) -> PathBuf {
    match appdata {
        Some(dir) if !dir.is_empty() => Path::new(dir).join("rasterwarp"),
        _ => PathBuf::from("rasterwarp-data"),
    }
}

/// App settings that belong to no project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub presets_folder: PathBuf,
    pub capture: RecordSettings,
    pub show_preview: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            presets_folder: PathBuf::from("presets"),
            capture: RecordSettings::default(),
            show_preview: true,
        }
    }
}

/// The settings in `dir`, or the defaults if there are none or they can't be read.
pub fn load_settings(dir: &Path) -> Settings {
    let path = dir.join(SETTINGS_FILE);
    if !path.exists() {
        return Settings::default();
    }
    match read_file(&path).and_then(|text| from_json::<Settings>(SETTINGS_FORMAT, &text)) {
        Ok(loaded) => {
            let mut settings = loaded.value;
            settings.capture.stop_after = settings.capture.stop_after.clamp(0.0, 3600.0);
            settings
        }
        Err(err) => {
            log::warn!("ignoring {}: {err:#}", path.display());
            Settings::default()
        }
    }
}

pub fn save_settings(dir: &Path, settings: &Settings) -> Result<()> {
    write_atomic(
        &dir.join(SETTINGS_FILE),
        &to_json(SETTINGS_FORMAT, settings),
    )
}

/// A fresh, empty folder for a test's files.
```

In `src/session.rs`, replace:

```rust

pub fn project_json(project: &Project) -> String {
```

with:

```rust

pub const AUTOSAVE_FILE: &str = "autosave.rwproject";
/// Where an autosave that couldn't be read is kept.
pub const BAD_AUTOSAVE_FILE: &str = "autosave.bad.rwproject";

/// The session as the app last left it. It's a project file with two more fields, so it
/// can also be opened as a project.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Autosave {
    /// The named project file the session belongs to, if any.
    pub file: Option<PathBuf>,
    /// The session had changes that weren't saved to `file`.
    pub unsaved: bool,
    #[serde(flatten)]
    pub project: Project,
}

pub fn save_autosave(dir: &Path, autosave: &Autosave) -> Result<()> {
    save::write_atomic(
        &dir.join(AUTOSAVE_FILE),
        &save::to_json(PROJECT_FORMAT, autosave),
    )
}

/// The autosave in `dir`, if there is one. One that can't be read is renamed to
/// [`BAD_AUTOSAVE_FILE`] (replacing an older one) so the next autosave doesn't
/// overwrite it, and the error says why.
pub fn restore_autosave(dir: &Path) -> Result<Option<Loaded<Autosave>>> {
    let path = dir.join(AUTOSAVE_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let read =
        save::read_file(&path).and_then(|text| save::from_json::<Autosave>(PROJECT_FORMAT, &text));
    match read {
        Ok(mut loaded) => {
            loaded.value.project = loaded.value.project.checked();
            Ok(Some(loaded))
        }
        Err(err) => {
            let bad = dir.join(BAD_AUTOSAVE_FILE);
            let _ = std::fs::remove_file(&bad);
            std::fs::rename(&path, &bad)
                .with_context(|| format!("could not move {} aside", path.display()))?;
            Err(err)
        }
    }
}

pub fn project_json(project: &Project) -> String {
```

In `src/ui.rs`, replace:

```rust
    pub load_error: Option<String>,
```

with:

```rust
    pub load_error: Option<String>,
    /// A project, preset or autosave problem.
    pub file_error: Option<String>,
    /// News about the last file opened (e.g. that a newer version wrote it).
    pub file_note: Option<String>,
    pub presets_folder: PathBuf,
```

In `src/ui.rs`, replace:

```rust
#[derive(Clone, Debug)]
```

with:

```rust
#[derive(Clone, Debug, Default)]
```

In `src/ui.rs`, replace:

```rust
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
```

with:

```rust

```

In `src/ui.rs`, replace:

```rust
                    ui.colored_label(egui::Color32::LIGHT_RED, err);
```

with:

```rust
                    ui.colored_label(egui::Color32::LIGHT_RED, err);
                }
                if let Some(err) = &state.file_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, err);
                }
                if let Some(note) = &state.file_note {
                    ui.label(note);
```

- [ ] **Step 4: Run everything.** Run: `cargo test`. Expected: `180 passed` for the unit tests, `4 passed` in `tests/capture.rs` and `15 passed` in `tests/smoke.rs`.

- [ ] **Step 5: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 6: Manual check (release).** With `APPDATA` pointing at an empty scratch folder (so your own settings are untouched), run `cargo run --release`, switch to **Transition**, and close the window. `<APPDATA>\rasterwarp\autosave.rwproject` now exists and starts with `"format": "rasterwarp-project"`, `"file": null`, `"unsaved": true`, `"mode": "transition"`. Run it again: it opens in Transition mode, at 0 late.

- [ ] **Step 7: Commit**

```bash
git add src/app.rs src/capture/encode.rs src/capture/mod.rs src/capture/recorder.rs src/save.rs src/session.rs src/ui.rs
git commit -m "feat: autosave the session and restore it on launch" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Project row, Presets section and file shortcuts

The panel gets a project row (name with an unsaved mark, Open project…, Save project, Save as…, New) and a Presets section (save with a name, click to load, right-click to rename or delete, choose the folder). Ctrl+S, Ctrl+Shift+S and Ctrl+O work, dropped presets and projects open, and the app asks before replacing unsaved work or a preset. Two panel widgets stop changing values just by being drawn, so an untouched session never looks unsaved.

**Files:**
- Create: `src/files_ui.rs`
- Modify: `src/app.rs`, `src/lib.rs`, `src/ui.rs`

**Interfaces:**
- Consumes: `presets::*` (Task 3); `session::{save_project, load_project}`, `Project` (Task 2); the Task 4 app methods and `UiState.{file_error, file_note, presets_folder}`.
- Produces:
  - `files_ui::{FilesUi { project_name, unsaved, preset_name, presets, renaming }, FileActions { open_project, save_project, save_project_as, new_project, save_preset, load_preset, rename_preset, delete_preset, choose_presets_folder, list_presets }, project_title(&FilesUi) -> String, project_row(ui, &FilesUi, recording, &mut FileActions), presets_section(ui, &mut FilesUi, folder, &mut FileActions), shortcuts(ctx, &mut FileActions)}`.
  - `UiState.files: FilesUi`; `UiActions.files: FileActions`.

- [ ] **Step 1: Write the failing tests.**

Create `src/files_ui.rs` holding only its tests for now (Step 3 puts the implementation above them):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_marks_unsaved_changes() {
        let mut files = FilesUi {
            project_name: "night-drive".into(),
            ..FilesUi::default()
        };
        assert_eq!(project_title(&files), "Project: night-drive");
        files.unsaved = true;
        assert_eq!(project_title(&files), "Project: night-drive •");
    }
}
```

In `src/lib.rs`, replace:

```rust
pub mod curve_editor;
```

with:

```rust
pub mod curve_editor;
pub mod files_ui;
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test --lib`. Expected: fails to compile, with ``cannot find function `project_title` in this scope`` and ``cannot find type `FilesUi` in this scope``.

- [ ] **Step 3: Implement.**

In `src/app.rs`, replace:

```rust
use crate::gpu;
use crate::motion::Motion;
use crate::params::Params;
use crate::passes::Renderer;
use crate::preview::PreviewView;
use crate::save::{self, Settings};
```

with:

```rust
use crate::files_ui::FileActions;
use crate::gpu;
use crate::motion::{Mode, Motion};
use crate::params::Params;
use crate::passes::Renderer;
use crate::presets;
use crate::preview::PreviewView;
use crate::save::{self, PRESET_EXTENSION, PROJECT_EXTENSION, Settings};
```

In `src/app.rs`, replace:

```rust
            WindowEvent::DroppedFile(path) => state.load_source(&path),
```

with:

```rust
            WindowEvent::DroppedFile(path) => state.dropped(&path),
```

In `src/app.rs`, replace:

```rust
            self.load_source(path);
        }
    }
```

with:

```rust
            self.load_source(path);
        }
        self.list_presets();
    }
```

In `src/app.rs`, replace:

```rust

    /// Loads a dropped (or command-line) image as the source.
```

with:

```rust

    /// Opens a dropped preset or project, or loads a dropped image as the source.
    fn dropped(&mut self, path: &Path) {
        let is = |ext: &str| {
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(ext))
        };
        if is(PRESET_EXTENSION) {
            self.load_preset(path);
        } else if is(PROJECT_EXTENSION) {
            self.open_project(Some(path));
        } else {
            self.load_source(path);
        }
    }

    /// The project file's name, or "Untitled".
    fn project_name(&self) -> String {
        self.project_file
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned())
    }

    fn file_actions(&mut self, actions: FileActions) {
        if actions.save_project {
            self.save_project(false);
        }
        if actions.save_project_as {
            self.save_project(true);
        }
        if actions.open_project {
            self.open_project(None);
        }
        if actions.new_project {
            self.new_project();
        }
        if let Some(name) = actions.save_preset {
            self.save_preset(&name);
        }
        if let Some(name) = actions.load_preset {
            self.load_preset(&presets::path(&self.ui.presets_folder, &name));
        }
        if let Some((from, to)) = actions.rename_preset {
            let renamed = presets::rename(&self.ui.presets_folder, &from, &to);
            self.report(renamed);
            self.list_presets();
        }
        if let Some(name) = actions.delete_preset
            && self.confirm(&format!("Delete preset {name}?"))
        {
            let deleted = presets::delete(&self.ui.presets_folder, &name);
            self.report(deleted);
            self.list_presets();
        }
        if actions.choose_presets_folder {
            self.choose_presets_folder();
        }
        if actions.list_presets {
            self.list_presets();
        }
    }

    /// Shows a file operation's error, or clears the last one.
    fn report(&mut self, result: Result<()>) {
        self.ui.file_note = None;
        self.ui.file_error = result.err().map(|err| {
            log::warn!("{err:#}");
            format!("{err:#}")
        });
    }

    /// Asks a yes/no question in a system dialog.
    fn confirm(&mut self, question: &str) -> bool {
        let answer = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Rasterwarp")
            .set_description(question)
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_parent(&*self.window)
            .show();
        // The dialog blocks the app; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
        answer == rfd::MessageDialogResult::Yes
    }

    /// Before the session is replaced: if it has unsaved changes, asks whether to save
    /// them (Yes saves, No discards). False means don't go on: the user cancelled, or
    /// saving was cancelled or failed.
    fn may_replace_session(&mut self) -> bool {
        if !self.unsaved(&self.project()) {
            return true;
        }
        let answer = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Unsaved changes")
            .set_description(format!("Save changes to {}?", self.project_name()))
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .set_parent(&*self.window)
            .show();
        self.clock.reanchor(self.seconds());
        match answer {
            rfd::MessageDialogResult::Yes => self.save_project(false),
            rfd::MessageDialogResult::No => true,
            _ => false,
        }
    }

    /// Saves the session to its project file, asking for one when Untitled or when
    /// `ask` (Save as). Returns whether it saved.
    fn save_project(&mut self, ask: bool) -> bool {
        let path = match &self.project_file {
            Some(path) if !ask => path.clone(),
            _ => match self.ask_project_path() {
                Some(path) => path,
                None => return false,
            },
        };
        let project = self.project();
        let saved = session::save_project(&path, &project);
        let ok = saved.is_ok();
        self.report(saved);
        if ok {
            self.project_file = Some(path);
            self.saved = Some(project);
        }
        ok
    }

    fn ask_project_path(&mut self) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save project")
            .add_filter("Rasterwarp project", &[PROJECT_EXTENSION])
            .set_file_name(format!("{}.{PROJECT_EXTENSION}", self.project_name()));
        if let Some(dir) = self.project_file.as_deref().and_then(Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        let picked = dialog.save_file();
        self.clock.reanchor(self.seconds());
        let path = picked?;
        // "show" becomes "show.rwproject"; "show.v2" becomes "show.v2.rwproject".
        let has_extension = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(PROJECT_EXTENSION));
        Some(if has_extension {
            path
        } else {
            let mut named = path.into_os_string();
            named.push(format!(".{PROJECT_EXTENSION}"));
            PathBuf::from(named)
        })
    }

    /// Opens a project (asking which when `path` is none), after the unsaved-changes
    /// check. Not while recording: the canvas can't change then.
    fn open_project(&mut self, path: Option<&Path>) {
        if self.recorder.is_some() || !self.may_replace_session() {
            return;
        }
        let path = match path {
            Some(path) => path.to_path_buf(),
            None => {
                let picked = rfd::FileDialog::new()
                    .set_title("Open project")
                    .add_filter("Rasterwarp project", &[PROJECT_EXTENSION])
                    .pick_file();
                self.clock.reanchor(self.seconds());
                match picked {
                    Some(path) => path,
                    None => return,
                }
            }
        };
        match session::load_project(&path) {
            Ok(loaded) => {
                self.apply_project(&loaded.value);
                self.project_file = Some(absolute(&path));
                self.saved = Some(self.project());
                self.ui.file_error = None;
                self.ui.file_note = loaded.newer.then(|| save::NEWER_NOTE.into());
            }
            Err(err) => self.report(Err(err)),
        }
    }

    /// Starts over with the default session, Untitled.
    fn new_project(&mut self) {
        if self.recorder.is_some() || !self.may_replace_session() {
            return;
        }
        self.apply_project(&Project::default());
        self.project_file = None;
        self.saved = Some(self.project());
        self.report(Ok(()));
    }

    /// Saves the edited look as preset `name`, asking before replacing one.
    fn save_preset(&mut self, name: &str) {
        let folder = self.ui.presets_folder.clone();
        if presets::exists(&folder, name) && !self.confirm(&format!("Replace preset {name}?")) {
            return;
        }
        let saved = presets::save(&folder, name, self.motion.editable());
        self.report(saved);
        self.list_presets();
    }

    /// Loads a preset into the edited look: on screen in Live mode, the off-air bank in
    /// Transition mode, the selected cue in Sequence mode.
    fn load_preset(&mut self, path: &Path) {
        match save::load_preset(path) {
            Ok(loaded) => {
                *self.motion.editable() = loaded.value;
                if self.motion.mode() == Mode::Live {
                    // The picture cuts to the preset.
                    self.renderer.reset_motion();
                }
                self.ui.file_error = None;
                self.ui.file_note = loaded.newer.then(|| save::NEWER_NOTE.into());
            }
            Err(err) => self.report(Err(err)),
        }
    }

    fn choose_presets_folder(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Presets folder");
        if let Ok(start) = std::path::absolute(&self.ui.presets_folder)
            && start.is_dir()
        {
            dialog = dialog.set_directory(start);
        }
        if let Some(picked) = dialog.pick_folder() {
            self.ui.presets_folder = picked;
            self.list_presets();
        }
        // The dialog blocks the app; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
    }

    fn list_presets(&mut self) {
        self.ui.files.presets = presets::list(&self.ui.presets_folder);
    }

    /// Loads a dropped (or command-line) image as the source.
```

In `src/app.rs`, replace:

```rust
        self.ui.late = self.clock.late();
```

with:

```rust
        self.ui.late = self.clock.late();
        self.ui.files.project_name = self.project_name();
        self.ui.files.unsaved = self.unsaved(&self.project());
```

In `src/app.rs`, replace:

```rust
        }
        if self.last_autosave.elapsed() >= AUTOSAVE_INTERVAL {
```

with:

```rust
        }
        self.file_actions(actions.files);
        if self.last_autosave.elapsed() >= AUTOSAVE_INTERVAL {
```

In `src/files_ui.rs`, put this above the tests (the file then starts with it):

```rust
//! The panel's project row and Presets section. They only report what was asked for;
//! the app does the file work and asks any questions.

use std::path::Path;

use egui::{Button, CollapsingHeader, Key, KeyboardShortcut, Modifiers, Ui};

use crate::presets;

/// What the project row and Presets section show, kept between frames.
#[derive(Default)]
pub struct FilesUi {
    /// The project's name, or "Untitled".
    pub project_name: String,
    /// The session has changes its project file doesn't.
    pub unsaved: bool,
    /// The name box for saving a preset.
    pub preset_name: String,
    /// The presets in the folder, as last listed.
    pub presets: Vec<String>,
    /// A preset being renamed: its name and the edited name.
    pub renaming: Option<(String, String)>,
}

/// One-shot file requests from the panel this frame.
#[derive(Default)]
pub struct FileActions {
    pub open_project: bool,
    pub save_project: bool,
    pub save_project_as: bool,
    pub new_project: bool,
    /// Save the edited look as this (checked) preset name.
    pub save_preset: Option<String>,
    pub load_preset: Option<String>,
    /// Rename a preset: from, to.
    pub rename_preset: Option<(String, String)>,
    pub delete_preset: Option<String>,
    pub choose_presets_folder: bool,
    /// List the presets folder again.
    pub list_presets: bool,
}

const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SAVE_AS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);

/// Ctrl+S saves the project, Ctrl+Shift+S saves it as, Ctrl+O opens one; not while a text
/// field has the keyboard.
pub fn shortcuts(ctx: &egui::Context, actions: &mut FileActions) {
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    ctx.input_mut(|i| {
        // Ctrl+Shift+S first, so Ctrl+S doesn't take it.
        if i.consume_shortcut(&SAVE_AS) {
            actions.save_project_as = true;
        } else if i.consume_shortcut(&SAVE) {
            actions.save_project = true;
        }
        if i.consume_shortcut(&OPEN) {
            actions.open_project = true;
        }
    });
}

/// `Project: <name>`, with ` •` while there are unsaved changes.
pub fn project_title(files: &FilesUi) -> String {
    let mark = if files.unsaved { " •" } else { "" };
    format!("Project: {}{mark}", files.project_name)
}

/// The project's name and its Open / Save / Save as / New buttons. Opening a project or
/// starting a new one changes the canvas, so they wait while recording.
pub fn project_row(ui: &mut Ui, files: &FilesUi, recording: bool, actions: &mut FileActions) {
    ui.label(project_title(files));
    ui.horizontal_wrapped(|ui| {
        let waits = "stop recording first";
        if ui
            .add_enabled(!recording, Button::new("Open project…"))
            .on_disabled_hover_text(waits)
            .clicked()
        {
            actions.open_project = true;
        }
        if ui.button("Save project").clicked() {
            actions.save_project = true;
        }
        if ui.button("Save as…").clicked() {
            actions.save_project_as = true;
        }
        if ui
            .add_enabled(!recording, Button::new("New"))
            .on_disabled_hover_text(waits)
            .clicked()
        {
            actions.new_project = true;
        }
    });
}

pub fn presets_section(ui: &mut Ui, files: &mut FilesUi, folder: &Path, actions: &mut FileActions) {
    let section = CollapsingHeader::new("Presets").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut files.preset_name).desired_width(140.0));
            let name = presets::check_name(&files.preset_name);
            let save = ui
                .add_enabled(name.is_ok(), Button::new("Save preset"))
                .on_disabled_hover_text(name.err().unwrap_or_default());
            if save.clicked()
                && let Ok(name) = name
            {
                actions.save_preset = Some(name.to_owned());
            }
        });
        // An empty box needs no explanation; a bad character does.
        if !files.preset_name.trim().is_empty()
            && let Err(why) = presets::check_name(&files.preset_name)
        {
            ui.small(why);
        }
        if files.presets.is_empty() {
            ui.small("No presets in this folder yet.");
        }
        for name in &files.presets {
            match &mut files.renaming {
                Some((from, to)) if from == name => {
                    let edit = ui.text_edit_singleline(to);
                    if edit.lost_focus() {
                        // Enter renames; Escape or clicking elsewhere cancels.
                        if ui.input(|i| i.key_pressed(Key::Enter)) {
                            actions.rename_preset = Some((from.clone(), to.clone()));
                        }
                        files.renaming = None;
                    } else if !edit.has_focus() {
                        edit.request_focus();
                    }
                }
                _ => {
                    let item = ui
                        .selectable_label(false, name)
                        .on_hover_text("Click to load · right-click to rename or delete");
                    if item.clicked() {
                        actions.load_preset = Some(name.clone());
                    }
                    item.context_menu(|ui| {
                        if ui.button("Rename").clicked() {
                            files.renaming = Some((name.clone(), name.clone()));
                            ui.close();
                        }
                        if ui.button("Delete").clicked() {
                            actions.delete_preset = Some(name.clone());
                            ui.close();
                        }
                    });
                }
            }
        }
        ui.horizontal(|ui| {
            ui.label(format!("Folder: {}", folder.display()));
            if ui.button("Choose…").clicked() {
                actions.choose_presets_folder = true;
            }
        });
    });
    // Opening the section shows what's in the folder now.
    if section.header_response.clicked() {
        actions.list_presets = true;
    }
}
```

In `src/ui.rs`, replace:

```rust
use crate::curve_editor::curve_editor;
```

with:

```rust
use crate::curve_editor::curve_editor;
use crate::files_ui::{self, FileActions, FilesUi};
```

In `src/ui.rs`, replace:

```rust
    pub capture: CaptureUi,
```

with:

```rust
    pub capture: CaptureUi,
    pub files: FilesUi,
```

In `src/ui.rs`, replace:

```rust
    pub clear_background: bool,
```

with:

```rust
    pub clear_background: bool,
    pub files: FileActions,
```

In `src/ui.rs`, replace:

```rust
    let mut actions = UiActions::default();
```

with:

```rust
    let mut actions = UiActions::default();
    files_ui::shortcuts(ui.ctx(), &mut actions.files);
```

In `src/ui.rs`, replace:

```rust
                ui.label("Drop a PNG/JPG onto the window to load it.");
```

with:

```rust
                ui.label("Drop an image, preset or project onto the window.");
```

In `src/ui.rs`, replace:

```rust
                let recording = state.capture.status.is_some();
```

with:

```rust
                let recording = state.capture.status.is_some();
                files_ui::project_row(ui, &state.files, recording, &mut actions.files);
```

In `src/ui.rs`, replace:

```rust
                mode_section(ui, motion);
```

with:

```rust
                mode_section(ui, motion);
                files_ui::presets_section(
                    ui,
                    &mut state.files,
                    &state.presets_folder,
                    &mut actions.files,
                );
```

In `src/ui.rs`, replace:

```rust
                let slider = Slider::new(&mut c.thresholds[k], ranges::THRESHOLD)
                    .max_decimals(3)
```

with:

```rust
                // Shown to 3 decimals but not rounded: a slider that rounds its value
                // changes the look just by being drawn (and it would always look unsaved).
                let slider = Slider::new(&mut c.thresholds[k], ranges::THRESHOLD)
                    .custom_formatter(|v, _| format!("{v:.3}"))
```

In `src/ui.rs`, replace:

```rust
                    ui.color_edit_button_rgb(color);
```

with:

```rust
                    // The button edits a copy: its color conversion isn't exact, and only
                    // a real edit may change the look.
                    let mut edited = *color;
                    if ui.color_edit_button_rgb(&mut edited).changed() {
                        *color = edited;
                    }
```

- [ ] **Step 4: Run everything.** Run: `cargo test`. Expected: `181 passed` for the unit tests, `4 passed` in `tests/capture.rs` and `15 passed` in `tests/smoke.rs`.

- [ ] **Step 5: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 6: Manual check (release).** Run `cargo run --release` (with a scratch `APPDATA`, as in Task 4), then check each item.
  - [ ] Right after **New**, the header reads `Project: Untitled` with no `•`, and stays that way with every panel section opened, in each mode.
  - [ ] **Presets:** type a name and **Save preset**; it appears in the list and in `presets\<name>.rwpreset`. Saving the same name asks "Replace preset <name>?". A name with `/` disables the button and shows the hint. Right-click → **Rename** (Enter renames, Escape cancels) and **Delete** (asks first).
  - [ ] Clicking a preset loads it: on screen in Live mode, into the off-air bank (the preview) in Transition mode, into the selected cue in Sequence mode.
  - [ ] **Ctrl+S** on an Untitled session opens the save dialog; after saving, the header shows the file's name with no `•`. Changing a slider adds `•`; **Ctrl+S** removes it without a dialog. **Save as…** always asks.
  - [ ] With unsaved changes, **New** and **Open project…** ask "Save changes to <name>?": Yes saves first, No discards, Cancel does nothing.
  - [ ] Dropping a `.rwpreset` onto the window loads it; dropping a `.rwproject` opens it (with the same question when there are unsaved changes).
  - [ ] Open a project whose source image was moved away: it opens with the test card and "Source image not found: <path>".
  - [ ] Close and reopen the app: the project name and the `•` come back as they were.

- [ ] **Step 7: Commit**

```bash
git add src/app.rs src/files_ui.rs src/lib.rs src/ui.rs
git commit -m "feat: open, save and preset controls in the panel" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Done when

- `cargo test` passes (181 unit, 4 capture and 15 smoke tests). `cargo clippy --all-targets` is clean and `cargo fmt` makes no changes.
- The manual checks in Tasks 4 and 5 pass.
