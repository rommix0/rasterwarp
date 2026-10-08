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
