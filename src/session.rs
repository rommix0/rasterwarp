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
        let mut sequence = self.sequence.as_ref().map(|s| {
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
        if self.mode == Mode::Sequence && sequence.is_none() {
            // As when switching into Sequence mode: one cue from the on-air bank.
            sequence = Some(Sequence::new(ab.banks[ab.on_air]));
        }
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
        &save::to_json(PROJECT_FORMAT, autosave)?,
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

pub fn project_json(project: &Project) -> Result<String> {
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
    save::write_atomic(path, &project_json(project)?)
}

pub fn load_project(path: &Path) -> Result<Loaded<Project>> {
    read_project(&save::read_file(path)?)
        .with_context(|| format!("could not open {}", path.display()))
}

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
        let mut json: Value = serde_json::from_str(&project_json(&project()).unwrap()).unwrap();
        edit(&mut json);
        read_project(&json.to_string()).unwrap().value
    }

    #[test]
    fn projects_round_trip() {
        let p = Project {
            background: Some(PathBuf::from(r"C:\images\key.png")),
            ..project()
        };
        assert_eq!(p.mode, Mode::Transition);
        assert_eq!(p.sequence.as_ref().unwrap().cues.len(), 2);
        let loaded = read_project(&project_json(&p).unwrap()).unwrap();
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
    fn sequence_mode_without_a_sequence_gets_one() {
        let hand_edited = Project {
            mode: Mode::Sequence,
            sequence: None,
            ..Project::default()
        };
        let checked = hand_edited.checked();
        let cues = checked.sequence.as_ref().expect("a sequence is built");
        assert_eq!(cues.cues.len(), 1);
        assert_eq!(checked.checked(), checked, "so it doesn't read as edited");
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
        let preset = save::preset_json(&Params::default()).unwrap();
        let err = read_project(&preset).unwrap_err();
        assert_eq!(err.to_string(), "this is a preset, not a project");
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
