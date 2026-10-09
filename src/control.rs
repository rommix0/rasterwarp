//! Links from a controller to the panel: MIDI messages, where they come from (a
//! source), what they move (a target), and the links between them as they're saved.

use std::ops::RangeInclusive;

use crate::motion::Mode;
use crate::params::table::{ChoiceId, SliderId};
use crate::save::Named;
use crate::transition::DURATION;

/// The transition duration's id; it lives on the transition, not in a bank.
pub const DURATION_ID: &str = "transition.duration";
/// The mode choice's id (Live, Transition, Sequence).
pub const MODE_ID: &str = "mode";

/// The pitch wheel at rest.
const PITCH_CENTRE: u16 = 8192;
/// The pitch wheel at its top.
const PITCH_MAX: u16 = 16383;

/// A MIDI message a link can use. Channels count from 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Message {
    /// A control change: a knob, fader or the mod wheel (CC 1).
    Cc { channel: u8, number: u8, value: u8 },
    /// 0–16383, 8192 at rest.
    Pitch { channel: u8, value: u16 },
    /// A key going down.
    NoteOn { channel: u8, number: u8 },
}

impl Message {
    /// Reads one MIDI message: a control change, the pitch wheel or a note-on. Anything
    /// else (note-offs, a note-on with velocity 0, clock, sysex) is None.
    pub fn parse(bytes: &[u8]) -> Option<Message> {
        let (&status, data) = bytes.split_first()?;
        let channel = (status & 0x0F) + 1;
        match (status & 0xF0, data) {
            (0xB0, &[number, value, ..]) => Some(Message::Cc {
                channel,
                number: number & 0x7F,
                value: value & 0x7F,
            }),
            (0xE0, &[lsb, msb, ..]) => Some(Message::Pitch {
                channel,
                value: u16::from(lsb & 0x7F) | (u16::from(msb & 0x7F) << 7),
            }),
            (0x90, &[number, velocity, ..]) if velocity > 0 => Some(Message::NoteOn {
                channel,
                number: number & 0x7F,
            }),
            _ => None,
        }
    }

    /// The control it came from.
    pub fn source(self) -> Source {
        match self {
            Message::Cc {
                channel, number, ..
            } => Source::Cc { channel, number },
            Message::Pitch { channel, .. } => Source::Pitch { channel },
            Message::NoteOn { channel, number } => Source::Note { channel, number },
        }
    }

    /// Where a wheel or fader is, 0–1, with the pitch wheel's centre exactly 0.5. None
    /// for a key.
    pub fn amount(self) -> Option<f32> {
        match self {
            Message::Cc { value, .. } => Some(f32::from(value) / 127.0),
            Message::Pitch { value, .. } if value >= PITCH_CENTRE => Some(
                0.5 + 0.5 * f32::from(value - PITCH_CENTRE) / f32::from(PITCH_MAX - PITCH_CENTRE),
            ),
            Message::Pitch { value, .. } => Some(0.5 * f32::from(value) / f32::from(PITCH_CENTRE)),
            Message::NoteOn { .. } => None,
        }
    }
}

/// A control on a controller. Channels count from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A control change: a knob, fader or the mod wheel (CC 1).
    Cc { channel: u8, number: u8 },
    /// The pitch wheel.
    Pitch { channel: u8 },
    /// A key.
    Note { channel: u8, number: u8 },
}

/// Note names within an octave, for labelling keys.
const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

impl Source {
    /// As saved: `cc:1:1`, `pitch:1`, `note:1:60`.
    pub fn saved(self) -> String {
        match self {
            Source::Cc { channel, number } => format!("cc:{channel}:{number}"),
            Source::Pitch { channel } => format!("pitch:{channel}"),
            Source::Note { channel, number } => format!("note:{channel}:{number}"),
        }
    }

    /// The source saved as `saved`, if it's one this version knows.
    pub fn from_saved(saved: &str) -> Option<Source> {
        let mut parts = saved.split(':');
        let kind = parts.next()?;
        let channel: u8 = parts
            .next()?
            .parse()
            .ok()
            .filter(|c| (1..=16).contains(c))?;
        // None: no number given. Some(None): a number that isn't 0-127.
        let number = parts
            .next()
            .map(|n| n.parse::<u8>().ok().filter(|n| *n <= 127));
        if parts.next().is_some() {
            return None;
        }
        match (kind, number) {
            ("cc", Some(Some(number))) => Some(Source::Cc { channel, number }),
            ("pitch", None) => Some(Source::Pitch { channel }),
            ("note", Some(Some(number))) => Some(Source::Note { channel, number }),
            _ => None,
        }
    }

    /// As people read it: "CC 1 ch 1", "Pitch ch 1", "Note C4 ch 1" (middle C is C4).
    pub fn label(self) -> String {
        match self {
            Source::Cc { channel, number } => format!("CC {number} ch {channel}"),
            Source::Pitch { channel } => format!("Pitch ch {channel}"),
            Source::Note { channel, number } => {
                let octave = i32::from(number / 12) - 1;
                let name = NOTE_NAMES[usize::from(number % 12)];
                format!("Note {name}{octave} ch {channel}")
            }
        }
    }

    /// Whether it's a wheel, fader or knob (which moves sliders) rather than a key
    /// (which picks options and runs actions).
    pub fn continuous(self) -> bool {
        !matches!(self, Source::Note { .. })
    }
}

/// Something a key can do: what a button or key in the app does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// Transition, or reverse or resume one (the Space key).
    Transition,
    /// Jump to the other bank at once.
    Cut,
    /// Sequence Run, or Stop while it runs.
    SequenceRun,
    /// Back to the sequence's first cue.
    SequenceReset,
    /// The source camera's loop on or off (F9).
    SourceLoop,
    /// The background camera's loop on or off (F10).
    BackgroundLoop,
    /// Record, or Stop recording.
    Record,
    /// Save the current frame as an image.
    SaveStill,
    /// Wipe the trails buffer.
    ClearTrails,
    /// Pause or resume.
    Pause,
}

impl Action {
    /// Every action, in the order the MIDI section lists them.
    pub const ALL: [Action; 10] = [
        Action::Transition,
        Action::Cut,
        Action::SequenceRun,
        Action::SequenceReset,
        Action::SourceLoop,
        Action::BackgroundLoop,
        Action::Record,
        Action::SaveStill,
        Action::ClearTrails,
        Action::Pause,
    ];

    /// Its permanent name, as links are saved.
    pub fn name(self) -> &'static str {
        match self {
            Action::Transition => "transition",
            Action::Cut => "cut",
            Action::SequenceRun => "sequence-run",
            Action::SequenceReset => "sequence-reset",
            Action::SourceLoop => "source-loop",
            Action::BackgroundLoop => "background-loop",
            Action::Record => "record",
            Action::SaveStill => "save-still",
            Action::ClearTrails => "clear-trails",
            Action::Pause => "pause",
        }
    }

    /// Its name in the MIDI section.
    pub fn label(self) -> &'static str {
        match self {
            Action::Transition => "Transition",
            Action::Cut => "Cut",
            Action::SequenceRun => "Sequence run/stop",
            Action::SequenceReset => "Sequence reset",
            Action::SourceLoop => "Source camera loop",
            Action::BackgroundLoop => "Background camera loop",
            Action::Record => "Record/stop",
            Action::SaveStill => "Save still",
            Action::ClearTrails => "Clear trails",
            Action::Pause => "Pause",
        }
    }
}

/// What a link moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// A bank slider.
    Slider(SliderId),
    /// The transition's duration.
    Duration,
    /// Option `usize` of a bank choice.
    Choice(ChoiceId, usize),
    /// Live, Transition or Sequence.
    Mode(Mode),
    /// A button or key's job.
    Action(Action),
}

impl Target {
    /// As saved: `slider:warp.zoom`, `choice:warp.osc1.waveform=square`,
    /// `action:transition`.
    pub fn saved(self) -> String {
        match self {
            Target::Slider(id) => format!("slider:{}", id.id()),
            Target::Duration => format!("slider:{DURATION_ID}"),
            Target::Choice(id, option) => {
                format!("choice:{}={}", id.id(), id.options()[option])
            }
            Target::Mode(mode) => format!("choice:{MODE_ID}={}", mode.name()),
            Target::Action(action) => format!("action:{}", action.name()),
        }
    }

    /// The target saved as `saved`, if it's one this version knows.
    pub fn from_saved(saved: &str) -> Option<Target> {
        let (kind, rest) = saved.split_once(':')?;
        match kind {
            "slider" if rest == DURATION_ID => Some(Target::Duration),
            "slider" => SliderId::from_id(rest).map(Target::Slider),
            "choice" => {
                let (id, option) = rest.split_once('=')?;
                if id == MODE_ID {
                    return Mode::ALL
                        .into_iter()
                        .find(|m| m.name() == option)
                        .map(Target::Mode);
                }
                let choice = ChoiceId::from_id(id)?;
                let index = choice.options().iter().position(|o| *o == option)?;
                Some(Target::Choice(choice, index))
            }
            "action" => Action::ALL
                .into_iter()
                .find(|a| a.name() == rest)
                .map(Target::Action),
            _ => None,
        }
    }

    /// Its name in the MIDI section: "Osc 1 amplitude", "Osc 1 waveform: square".
    pub fn label(self) -> String {
        match self {
            Target::Slider(id) => id.label(),
            Target::Duration => "Transition duration".into(),
            Target::Choice(id, option) => format!("{}: {}", id.label(), id.options()[option]),
            Target::Mode(mode) => format!("Mode: {}", mode.name()),
            Target::Action(action) => action.label().into(),
        }
    }

    /// A slider's range; None for a choice or action.
    pub fn range(self) -> Option<RangeInclusive<f32>> {
        match self {
            Target::Slider(id) => Some(id.range()),
            Target::Duration => Some(DURATION),
            _ => None,
        }
    }

    /// Whether it's a whole-numbered slider.
    pub fn whole(self) -> bool {
        matches!(self, Target::Slider(id) if id.whole())
    }

    /// Whether wheels and faders move it (a slider) rather than keys (a choice or
    /// action).
    pub fn continuous(self) -> bool {
        self.range().is_some()
    }
}

/// A control linked to a target. A slider target moves from `min` (the control at 0) to
/// `max` (the control at its top); `min` above `max` inverts it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    /// The control that drives it.
    pub source: Source,
    /// What it moves.
    pub target: Target,
    /// The slider value with the control at 0.
    pub min: f32,
    /// The slider value with the control at its top.
    pub max: f32,
}

impl Link {
    /// A link across the target's whole range.
    pub fn new(source: Source, target: Target) -> Link {
        let (min, max) = target
            .range()
            .map_or((0.0, 1.0), |r| (*r.start(), *r.end()));
        Link {
            source,
            target,
            min,
            max,
        }
    }

    /// The slider value for a control `amount` (0–1) of the way up, rounded for a
    /// whole-numbered slider.
    pub fn value(&self, amount: f32) -> f32 {
        let value = self.min + (self.max - self.min) * amount;
        if self.target.whole() {
            value.round()
        } else {
            value
        }
    }
}

/// Links as the settings save them: strings for the source and target, so a link this
/// version doesn't know (from a newer one, or a renamed control) is dropped with a log
/// line instead of failing the file. Use with `#[serde(with = "...")]`.
pub mod saved_links {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::Value;

    use super::{Link, Source, Target};

    /// A link as it sits in the file; min and max only for a slider.
    #[derive(Serialize)]
    struct Saved {
        source: String,
        target: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    }

    /// An f32 as the f64 with the same shortest decimal (0.1, not 0.10000000149), so a
    /// saved range reads the way it was typed.
    fn decimal(value: f32) -> f64 {
        value.to_string().parse().unwrap_or(f64::from(value))
    }

    /// Writes the links as a list of strings-and-ranges entries.
    pub fn serialize<S: Serializer>(links: &[Link], s: S) -> Result<S::Ok, S::Error> {
        let saved: Vec<Saved> = links
            .iter()
            .map(|link| {
                let slider = link.target.continuous();
                Saved {
                    source: link.source.saved(),
                    target: link.target.saved(),
                    min: slider.then(|| decimal(link.min)),
                    max: slider.then(|| decimal(link.max)),
                }
            })
            .collect();
        saved.serialize(s)
    }

    /// Reads the links it knows; anything else (or a list that isn't a list) is left out.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Link>, D::Error> {
        let value = Value::deserialize(d)?;
        let entries = value.as_array().cloned().unwrap_or_default();
        Ok(entries
            .iter()
            .filter_map(|entry| {
                let link = read(entry);
                if link.is_none() {
                    log::warn!("ignoring a MIDI link this version doesn't know: {entry}");
                }
                link
            })
            .collect())
    }

    /// One saved link, if its source and target are known and fit each other. A missing
    /// or broken min or max takes the slider's end; both are held to its range.
    fn read(entry: &Value) -> Option<Link> {
        let source = Source::from_saved(entry.get("source")?.as_str()?)?;
        let target = Target::from_saved(entry.get("target")?.as_str()?)?;
        if source.continuous() != target.continuous() {
            return None;
        }
        let mut link = Link::new(source, target);
        if let Some(range) = target.range() {
            let read = |key: &str, default: f32| {
                entry
                    .get(key)
                    .and_then(Value::as_f64)
                    .map_or(default, |v| v as f32)
                    .clamp(*range.start(), *range.end())
            };
            link.min = read("min", link.min);
            link.max = read("max", link.max);
        }
        Some(link)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::Role;
    use crate::params::table::{OscSlider, VideoSlider};

    #[test]
    fn messages_are_read_from_their_bytes() {
        assert_eq!(
            Message::parse(&[0xB0, 1, 64]),
            Some(Message::Cc {
                channel: 1,
                number: 1,
                value: 64
            })
        );
        assert_eq!(
            Message::parse(&[0xE3, 0x00, 0x40]),
            Some(Message::Pitch {
                channel: 4,
                value: 8192
            })
        );
        assert_eq!(
            Message::parse(&[0xEF, 0x7F, 0x7F]),
            Some(Message::Pitch {
                channel: 16,
                value: 16383
            })
        );
        assert_eq!(
            Message::parse(&[0x92, 60, 100]),
            Some(Message::NoteOn {
                channel: 3,
                number: 60
            })
        );
        assert_eq!(
            Message::parse(&[0x90, 60, 0]),
            None,
            "velocity 0 is a note-off"
        );
        assert_eq!(Message::parse(&[0x80, 60, 64]), None, "note-off");
        assert_eq!(Message::parse(&[0xF8]), None, "clock");
        assert_eq!(Message::parse(&[0xB0, 1]), None, "cut short");
        assert_eq!(Message::parse(&[]), None);
    }

    #[test]
    fn wheels_give_amounts_with_the_pitch_centre_exactly_halfway() {
        let pitch = |value| Message::Pitch { channel: 1, value }.amount().unwrap();
        assert_eq!(pitch(0), 0.0);
        assert_eq!(pitch(8192), 0.5);
        assert_eq!(pitch(16383), 1.0);
        let cc = |value| {
            Message::Cc {
                channel: 1,
                number: 1,
                value,
            }
            .amount()
            .unwrap()
        };
        assert_eq!(cc(0), 0.0);
        assert_eq!(cc(127), 1.0);
        assert_eq!(
            Message::NoteOn {
                channel: 1,
                number: 60
            }
            .amount(),
            None
        );
    }

    #[test]
    fn sources_are_saved_and_named() {
        let cc = Source::Cc {
            channel: 1,
            number: 1,
        };
        let pitch = Source::Pitch { channel: 2 };
        let note = Source::Note {
            channel: 1,
            number: 60,
        };
        assert_eq!(cc.saved(), "cc:1:1");
        assert_eq!(pitch.saved(), "pitch:2");
        assert_eq!(note.saved(), "note:1:60");
        for source in [cc, pitch, note] {
            assert_eq!(Source::from_saved(&source.saved()), Some(source));
        }
        assert_eq!(cc.label(), "CC 1 ch 1");
        assert_eq!(pitch.label(), "Pitch ch 2");
        assert_eq!(note.label(), "Note C4 ch 1");
        assert_eq!(
            Source::Note {
                channel: 1,
                number: 1
            }
            .label(),
            "Note C#-1 ch 1"
        );
        for bad in [
            "cc:17:1", "cc:0:1", "cc:1:128", "pitch:0", "note:1", "foo:1:1", "",
        ] {
            assert_eq!(Source::from_saved(bad), None, "{bad}");
        }
        assert!(cc.continuous() && pitch.continuous() && !note.continuous());
        assert_eq!(
            Message::Cc {
                channel: 2,
                number: 7,
                value: 3
            }
            .source(),
            Source::Cc {
                channel: 2,
                number: 7
            }
        );
    }

    #[test]
    fn targets_are_saved_and_named() {
        let targets = [
            Target::Slider(SliderId::Osc(0, OscSlider::Amplitude)),
            Target::Slider(SliderId::Video(Role::Background, VideoSlider::SlitDepth)),
            Target::Duration,
            Target::Choice(ChoiceId::Waveform(2), 3),
            Target::Mode(Mode::Transition),
            Target::Action(Action::BackgroundLoop),
        ];
        let saved: Vec<String> = targets.iter().map(|t| t.saved()).collect();
        assert_eq!(
            saved,
            [
                "slider:warp.osc1.amplitude",
                "slider:background.slit-depth",
                "slider:transition.duration",
                "choice:warp.osc3.waveform=square",
                "choice:mode=transition",
                "action:background-loop",
            ]
        );
        for target in targets {
            assert_eq!(Target::from_saved(&target.saved()), Some(target));
        }
        for bad in [
            "slider:warp.nothing",
            "choice:warp.osc1.waveform=wobble",
            "choice:warp.osc1.waveform",
            "choice:mode=chaos",
            "action:explode",
            "knob:warp.zoom",
        ] {
            assert_eq!(Target::from_saved(bad), None, "{bad}");
        }
        assert_eq!(targets[0].label(), "Osc 1 amplitude");
        assert_eq!(targets[2].label(), "Transition duration");
        assert_eq!(targets[3].label(), "Osc 3 waveform: square");
        assert_eq!(targets[4].label(), "Mode: transition");
        assert_eq!(targets[5].label(), "Background camera loop");
        assert!(targets[0].continuous() && targets[2].continuous());
        assert!(!targets[3].continuous() && !targets[4].continuous() && !targets[5].continuous());
        assert!(Target::Slider(SliderId::Levels).whole());
        assert!(!targets[0].whole());
    }

    #[test]
    fn action_names_never_change() {
        let names: Vec<&str> = Action::ALL.iter().map(|a| a.name()).collect();
        assert_eq!(
            names,
            [
                "transition",
                "cut",
                "sequence-run",
                "sequence-reset",
                "source-loop",
                "background-loop",
                "record",
                "save-still",
                "clear-trails",
                "pause",
            ]
        );
    }

    #[test]
    fn links_map_amounts_across_their_range() {
        let source = Source::Cc {
            channel: 1,
            number: 1,
        };
        let zoom = Link::new(source, Target::Slider(SliderId::Zoom));
        assert_eq!((zoom.min, zoom.max), (0.1, 4.0), "the slider's whole range");
        assert_eq!(zoom.value(0.0), 0.1);
        assert_eq!(zoom.value(1.0), 4.0);
        let inverted = Link {
            min: 2.0,
            max: 1.0,
            ..zoom
        };
        assert_eq!(inverted.value(0.0), 2.0);
        assert_eq!(inverted.value(0.25), 1.75);
        let levels = Link::new(source, Target::Slider(SliderId::Levels));
        assert_eq!(levels.value(0.5), 5.0, "2 + 6 × 0.5, rounded");
        assert_eq!(levels.value(0.45), 5.0);
        let duration = Link::new(source, Target::Duration);
        assert_eq!((duration.min, duration.max), (0.1, 30.0));
    }

    #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
    struct Holder {
        #[serde(with = "saved_links")]
        links: Vec<Link>,
    }

    #[test]
    fn links_are_saved_as_strings_and_unknown_ones_dropped() {
        let cc = Source::Cc {
            channel: 1,
            number: 1,
        };
        let key = Source::Note {
            channel: 1,
            number: 60,
        };
        let holder = Holder {
            links: vec![
                Link {
                    min: 0.1,
                    max: 0.2,
                    ..Link::new(cc, Target::Slider(SliderId::Osc(0, OscSlider::Amplitude)))
                },
                Link::new(key, Target::Action(Action::Transition)),
            ],
        };
        let json = serde_json::to_value(&holder).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "links": [
                { "source": "cc:1:1", "target": "slider:warp.osc1.amplitude", "min": 0.1, "max": 0.2 },
                { "source": "note:1:60", "target": "action:transition" },
            ]})
        );
        let back: Holder = serde_json::from_value(json).unwrap();
        assert_eq!(back, holder);

        let read: Holder = serde_json::from_value(serde_json::json!({ "links": [
            { "source": "cc:1:1", "target": "slider:warp.nothing" },
            { "source": "osc:/x", "target": "slider:warp.zoom" },
            { "source": "note:1:60", "target": "slider:warp.zoom" },
            { "source": "cc:1:2", "target": "action:cut" },
            { "source": "cc:1:3", "target": "slider:warp.zoom", "min": -50.0, "max": "high" },
            { "source": "cc:1:4" },
            7,
            { "source": "pitch:1", "target": "slider:warp.zoom", "min": 0.5 },
        ]}))
        .unwrap();
        assert_eq!(
            read.links,
            [
                Link {
                    min: 0.1,
                    max: 4.0,
                    ..Link::new(
                        Source::Cc {
                            channel: 1,
                            number: 3
                        },
                        Target::Slider(SliderId::Zoom)
                    )
                },
                Link {
                    min: 0.5,
                    max: 4.0,
                    ..Link::new(Source::Pitch { channel: 1 }, Target::Slider(SliderId::Zoom))
                },
            ],
            "unknown ids, sources, mismatched kinds and broken entries are dropped; \
             ranges are held to the slider's"
        );
        let empty: Holder = serde_json::from_value(serde_json::json!({ "links": "nope" })).unwrap();
        assert!(empty.links.is_empty());
    }
}
