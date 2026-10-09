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
        self.beats
            .retain(|l| !(l.beat == beat && l.target == target));
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
        let Target::Slider(slider) = Target::from_saved(entry.get("target")?.as_str()?)? else {
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
        assert_eq!(offsets.iter().map(|(id, _)| *id).collect::<Vec<_>>(), order);
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
        assert_eq!(
            read.follow[1].depth, span,
            "depth held to ± the range's span"
        );
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
