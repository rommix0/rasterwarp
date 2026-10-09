//! Drawing linkable controls: sliders and choices from the parameter table, written
//! through its setters (so the panel and MIDI follow the same rules), and the
//! right-click menu that links any control to MIDI or audio, or unlinks it.

use std::ops::RangeInclusive;

use egui::{ComboBox, Popup, Response, SetOpenCommand, Slider, Ui, WidgetText};

use crate::audio::{Beat, Signal};
use crate::audio_links::{AudioLinks, fires};
use crate::control::{Links, Source, Target};
use crate::params::Params;
use crate::params::table::{ChoiceId, SliderId};
use crate::transition::DURATION;

/// A bank slider over its whole range.
pub fn slider(
    ui: &mut Ui,
    links: &mut Links,
    p: &mut Params,
    id: SliderId,
    text: &str,
) -> Response {
    slider_shown(ui, links, p, id, text, id.range(), |s| s)
}

/// The transition duration's slider, in seconds.
pub fn duration(ui: &mut Ui, links: &mut Links, seconds: &mut f32) -> Response {
    let mut value = *seconds;
    let response = ui.add(
        Slider::new(&mut value, DURATION)
            .text("duration (s)")
            .logarithmic(true),
    );
    if response.changed() && !by_secondary_button(ui) {
        *seconds = value;
    }
    slider_menu(ui, &response, links, Target::Duration);
    response
}

/// A bank slider drawn over `shown`, which may be narrower than its range (a camera's
/// delay goes up to its buffer). It shows the value held to `shown`, but only an edit
/// changes the value, so drawing it isn't a change. `tweak` adds formatting.
pub fn slider_shown(
    ui: &mut Ui,
    links: &mut Links,
    p: &mut Params,
    id: SliderId,
    text: &str,
    shown: RangeInclusive<f32>,
    tweak: impl FnOnce(Slider<'_>) -> Slider<'_>,
) -> Response {
    let mut value = id.get(p).clamp(*shown.start(), *shown.end());
    let mut slider = Slider::new(&mut value, shown).text(text);
    if id.whole() {
        slider = slider.integer();
    }
    let response = ui.add(tweak(slider));
    if response.changed() && !by_secondary_button(ui) {
        id.set(p, value);
    }
    slider_menu(ui, &response, links, Target::Slider(id));
    response
}

/// Whether the secondary button is down or just came up. The slider's rail takes a
/// press of either button as a drag, so this tells a right-click from an edit.
fn by_secondary_button(ui: &Ui) -> bool {
    ui.input(|i| i.pointer.secondary_down() || i.pointer.secondary_released())
}

/// Option `option` of a choice, as a selectable label: clicking picks it.
pub fn option(
    ui: &mut Ui,
    links: &mut Links,
    p: &mut Params,
    id: ChoiceId,
    option: usize,
    text: impl Into<WidgetText>,
) -> Response {
    let response = ui.selectable_label(id.get(p) == option, text);
    if response.clicked() {
        id.set(p, option);
    }
    link_menu(&response, links, Target::Choice(id, option));
    response
}

/// A choice as a drop-down with `labels` for its options. Right-clicking it offers to
/// link any of them.
pub fn combo(ui: &mut Ui, links: &mut Links, p: &mut Params, id: ChoiceId, labels: &[String]) {
    let current = id.get(p);
    let mut picked = None;
    let response = ComboBox::from_id_salt(id)
        .selected_text(labels[current].as_str())
        .show_ui(ui, |ui| {
            for (option, label) in labels.iter().enumerate() {
                if ui.selectable_label(current == option, label).clicked() {
                    picked = Some(option);
                }
            }
        })
        .response;
    if let Some(option) = picked {
        id.set(p, option);
    }
    let linked: Vec<String> = labels
        .iter()
        .enumerate()
        .filter_map(|(option, label)| {
            let names = link_names(links, Target::Choice(id, option))?;
            Some(format!("{label}: {names}"))
        })
        .collect();
    if !linked.is_empty() {
        response.clone().on_hover_text(linked.join(
            "
",
        ));
    }
    response.context_menu(|ui| {
        for (option, label) in labels.iter().enumerate() {
            let target = Target::Choice(id, option);
            menu_items(ui, links, target, &format!("Link {label} to MIDI"));
            audio_items(ui, &mut links.audio, target, Some(label));
        }
    });
}

/// Adds the right-click menu that links `target` to MIDI or audio, or unlinks it, and
/// names its links when hovered.
pub fn link_menu(response: &Response, links: &mut Links, target: Target) {
    name_links(response, links, target);
    response.context_menu(|ui| {
        menu_items(ui, links, target, "Link to MIDI");
        audio_items(ui, &mut links.audio, target, None);
    });
}

/// [`link_menu`] for a slider. The rail only senses drags, so it never reports a
/// right-click and `context_menu` would open on the value box alone; this opens the
/// same menu when the button comes up anywhere over the slider.
fn slider_menu(ui: &Ui, response: &Response, links: &mut Links, target: Target) {
    name_links(response, links, target);
    let right_clicked = response.contains_pointer() && ui.input(|i| i.pointer.secondary_clicked());
    let open = if right_clicked || response.secondary_clicked() {
        Some(SetOpenCommand::Bool(true))
    } else if response.clicked() {
        Some(SetOpenCommand::Bool(false))
    } else {
        None
    };
    Popup::menu(response)
        .open_memory(open)
        .at_pointer_fixed()
        .show(|ui| {
            menu_items(ui, links, target, "Link to MIDI");
            audio_items(ui, &mut links.audio, target, None);
        });
}

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

/// "Link to MIDI", then "Unlink …" for each control linked to `target`.
fn menu_items(ui: &mut Ui, links: &mut Links, target: Target, text: &str) {
    if ui.button(text).clicked() {
        links.learn(target);
        ui.close();
    }
    for source in sources(links, target) {
        if ui.button(format!("Unlink {}", source.label())).clicked() {
            links.unlink(source, target);
            ui.close();
        }
    }
}

/// The controls linked to `target`.
fn sources(links: &Links, target: Target) -> Vec<Source> {
    links.to(target).map(|l| l.source).collect()
}

/// Their names, joined for a hover text.
fn names(sources: &[Source]) -> String {
    sources
        .iter()
        .map(|s| s.label())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::Role;
    use crate::params::table::VideoSlider;

    /// Draws `draw` once in a headless egui frame.
    fn frame(draw: impl FnOnce(&mut Ui)) {
        let ctx = egui::Context::default();
        let mut draw = Some(draw);
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            if let Some(draw) = draw.take() {
                draw(ui);
            }
        });
        // The font textures it made would otherwise panic when dropped unapplied.
        let mut textures = output.textures_delta;
        textures.clear();
    }

    /// Presses and releases `button` over the rail of the slider `draw` shows, one
    /// headless frame at a time, and gives the context to look at afterwards.
    fn click_rail(
        button: egui::PointerButton,
        mut draw: impl FnMut(&mut Ui) -> Response,
    ) -> egui::Context {
        let ctx = egui::Context::default();
        let mut rail = egui::Pos2::ZERO;
        let mut step = |events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                events,
                ..Default::default()
            };
            let output = ctx.run_ui(raw, |ui| {
                let rect = draw(ui).rect;
                rail = egui::pos2(rect.min.x + 30.0, rect.center().y);
            });
            let mut textures = output.textures_delta;
            textures.clear();
            rail
        };
        let pointer = |pos, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        };
        // The first frame finds the slider; the pointer then has to be seen over it
        // before a press can land on it.
        let at = step(vec![]);
        step(vec![egui::Event::PointerMoved(at)]);
        step(vec![egui::Event::PointerMoved(at)]);
        step(vec![pointer(at, true)]);
        step(vec![pointer(at, false)]);
        step(vec![]);
        step(vec![]);
        ctx
    }

    #[test]
    fn a_primary_click_on_the_rail_moves_the_slider_but_opens_no_menu() {
        let mut p = Params::default();
        let id = SliderId::Video(Role::Source, VideoSlider::Delay);
        id.set(&mut p, 2.0);
        let mut links = Links::default();
        let ctx = click_rail(egui::PointerButton::Primary, |ui| {
            slider_shown(ui, &mut links, &mut p, id, "delay", 0.0..=2.0, |s| s)
        });
        assert!(id.get(&p) < 1.5, "the click lands on the rail");
        assert!(!Popup::is_any_open(&ctx));
    }

    #[test]
    fn right_clicking_a_slider_rail_opens_the_menu_and_leaves_the_value() {
        let mut p = Params::default();
        let id = SliderId::Video(Role::Source, VideoSlider::Delay);
        id.set(&mut p, 2.0);
        let mut links = Links::default();
        let ctx = click_rail(egui::PointerButton::Secondary, |ui| {
            slider_shown(ui, &mut links, &mut p, id, "delay", 0.0..=2.0, |s| s)
        });
        assert_eq!(id.get(&p), 2.0);
        assert!(Popup::is_any_open(&ctx), "the link menu is open");
    }

    #[test]
    fn right_clicking_the_duration_slider_opens_the_menu_and_leaves_the_value() {
        let mut seconds = 30.0;
        let mut links = Links::default();
        let ctx = click_rail(egui::PointerButton::Secondary, |ui| {
            duration(ui, &mut links, &mut seconds)
        });
        assert_eq!(seconds, 30.0);
        assert!(Popup::is_any_open(&ctx), "the link menu is open");
    }

    #[test]
    fn drawing_a_slider_shown_narrower_leaves_its_value() {
        let mut p = Params::default();
        let id = SliderId::Video(Role::Source, VideoSlider::Delay);
        id.set(&mut p, 9.0);
        let mut links = Links::default();
        frame(|ui| {
            slider_shown(ui, &mut links, &mut p, id, "delay", 0.0..=2.0, |s| s);
        });
        assert_eq!(id.get(&p), 9.0);
    }

    #[test]
    fn drawing_choices_leaves_them_alone() {
        let mut p = Params::default();
        let id = ChoiceId::Waveform(1);
        id.set(&mut p, 2);
        let mut links = Links::default();
        let labels = ["a", "b", "c", "d", "e", "f"].map(String::from);
        let labels = &labels[..crate::params::Waveform::ALL.len()];
        frame(|ui| {
            option(ui, &mut links, &mut p, id, 0, "a");
            combo(ui, &mut links, &mut p, id, labels);
        });
        assert_eq!(id.get(&p), 2);
    }

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
            audio_section(
                ui,
                &mut audio,
                &mut links,
                &params,
                &[],
                &mut AudioActions::default(),
            )
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
}
