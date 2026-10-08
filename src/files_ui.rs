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
