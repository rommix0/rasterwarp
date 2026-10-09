//! The panel's MIDI section: the devices, the links, and learning a new one.

use egui::{CollapsingHeader, Color32, DragValue, Ui};

use crate::control::Links;

/// What the MIDI section shows and edits.
#[derive(Clone, Debug, Default)]
pub struct MidiUi {
    /// The links in use; the app applies them and saves them with the settings.
    pub links: Links,
    /// The open devices' names.
    pub devices: Vec<String>,
    /// Ports that wouldn't open, and why.
    pub failed: Vec<(String, String)>,
}

/// "Learning *target*: move a wheel or press a key…" with Cancel, while a link is being
/// learned. Esc cancels too (see `ui::draw`).
pub fn learning_line(ui: &mut Ui, links: &mut Links) {
    let Some(target) = links.learning() else {
        return;
    };
    ui.horizontal_wrapped(|ui| {
        ui.colored_label(
            Color32::YELLOW,
            format!("Learning {}: move a wheel or press a key…", target.label()),
        );
        if ui.button("Cancel").on_hover_text("Esc").clicked() {
            links.cancel();
        }
    });
}

/// The MIDI section: the devices, ports that failed, and every link with its range.
/// Returns whether Refresh was pressed.
pub fn midi_section(ui: &mut Ui, midi: &mut MidiUi) -> bool {
    let mut refresh = false;
    CollapsingHeader::new("MIDI").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            if midi.devices.is_empty() {
                ui.label("No MIDI input devices");
            } else {
                ui.label(format!("Devices: {}", midi.devices.join(", ")));
            }
            if ui.button("Refresh").clicked() {
                refresh = true;
            }
        });
        for (name, why) in &midi.failed {
            ui.colored_label(Color32::LIGHT_RED, format!("{name}: {why}"));
        }
        learning_line(ui, &mut midi.links);
        if midi.links.list.is_empty() {
            ui.small("Right-click a slider, option or button and choose Link to MIDI.");
        }
        let mut remove = None;
        egui::Grid::new("midi links").striped(true).show(ui, |ui| {
            for (i, link) in midi.links.list.iter_mut().enumerate() {
                ui.label(link.source.label());
                ui.label(link.target.label());
                match link.target.range() {
                    Some(range) => {
                        ui.horizontal(|ui| {
                            let speed = (range.end() - range.start()) / 200.0;
                            ui.add(
                                DragValue::new(&mut link.min)
                                    .range(range.clone())
                                    .speed(speed),
                            )
                            .on_hover_text("The value with the control at the bottom");
                            ui.label("to");
                            ui.add(DragValue::new(&mut link.max).range(range).speed(speed))
                                .on_hover_text("The value with the control at the top");
                        });
                    }
                    None => {
                        ui.label("");
                    }
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = remove {
            midi.links.remove(i);
        }
    });
    refresh
}
