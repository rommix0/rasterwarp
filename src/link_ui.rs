//! Drawing linkable controls: sliders and choices from the parameter table, written
//! through its setters (so the panel and MIDI follow the same rules), and the
//! right-click menu that links any control to MIDI or unlinks it.

use std::ops::RangeInclusive;

use egui::{ComboBox, Response, Slider, Ui, WidgetText};

use crate::control::{Links, Source, Target};
use crate::params::Params;
use crate::params::table::{ChoiceId, SliderId};

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
    if response.changed() {
        id.set(p, value);
    }
    link_menu(&response, links, Target::Slider(id));
    response
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
    response.context_menu(|ui| {
        for (option, label) in labels.iter().enumerate() {
            menu_items(
                ui,
                links,
                Target::Choice(id, option),
                &format!("Link {label} to MIDI"),
            );
        }
    });
}

/// Adds the right-click menu that links `target` to MIDI or unlinks it, and names its
/// links when hovered.
pub fn link_menu(response: &Response, links: &mut Links, target: Target) {
    let linked = sources(links, target);
    if !linked.is_empty() {
        response
            .clone()
            .on_hover_text(format!("MIDI: {}", names(&linked)));
    }
    response.context_menu(|ui| menu_items(ui, links, target, "Link to MIDI"));
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
}
