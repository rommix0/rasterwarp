//! The egui parameter panel.

use std::path::PathBuf;

use egui::{Button, CollapsingHeader, ComboBox, ProgressBar, Slider, Ui};

use crate::audio_ui::{self, AudioActions, AudioUi};
use crate::canvas::{CanvasChoice, PRESETS};
use crate::capture::CaptureMode;
use crate::capture::encode::VideoFormat;
use crate::capture::recorder::{RecordSettings, RecordStatus};
use crate::control::{Action, Links, Target};
use crate::curve::{CurveLibrary, CurveRef};
use crate::curve_editor::curve_editor;
use crate::files_ui::{self, FileActions, FilesUi};
use crate::inputs_ui::{self, InputActions, InputUi};
use crate::link_ui;
use crate::midi_ui::{self, MidiUi};
use crate::motion::{Mode, Motion};
use crate::params::table::{ChoiceId, OscSlider, SliderId};
use crate::params::{
    Envelope, OSCILLATOR_COUNT, OscInput, OscSync, Params, Role, Waveform, even_thresholds, ranges,
};
use crate::rate::FrameRate;
use crate::sequence::{FRAMES_PER_SECOND, MAX_FRAME};
use crate::theme::{self, ThemeChoice};
use crate::transition::AbState;

/// UI-only state that isn't a render parameter.
#[derive(Default)]
pub struct UiState {
    pub paused: bool,
    /// The control panel is hidden, so the canvas fills the window.
    pub panel_hidden: bool,
    /// Show the off-air preview in Transition and Sequence modes.
    pub show_preview: bool,
    /// The panel's light or dark appearance.
    pub theme: ThemeChoice,
    /// Smoothed time between screen refreshes in milliseconds.
    pub frame_ms: f32,
    /// The program frame rate chosen in the panel.
    pub rate: FrameRate,
    /// Canvas frames skipped so far because the app fell behind.
    pub late: u64,
    pub load_error: Option<String>,
    /// A project, preset or autosave problem.
    pub file_error: Option<String>,
    /// News about the last file opened (e.g. that a newer version wrote it).
    pub file_note: Option<String>,
    /// Why this window doesn't autosave. It stays up; file messages come and go.
    pub session_note: Option<String>,
    pub presets_folder: PathBuf,
    /// The Source and Background sections, indexed by [`Role::index`].
    pub inputs: [InputUi; 2],
    /// The cameras DirectShow listed; none until they're first needed.
    pub cameras: Option<Vec<String>>,
    /// The user curve open in the curve editor.
    pub editing_curve: Option<u32>,
    pub canvas: CanvasChoice,
    pub capture: CaptureUi,
    pub files: FilesUi,
    /// MIDI devices and links.
    pub midi: MidiUi,
    /// The sound source's state and settings.
    pub audio: AudioUi,
}

/// The capture controls and what the current or last recording reported.
#[derive(Clone, Debug, Default)]
pub struct CaptureUi {
    pub settings: RecordSettings,
    /// Progress while recording.
    pub status: Option<RecordStatus>,
    /// What the last recording saved.
    pub saved: Option<String>,
    pub error: Option<String>,
}

/// The off-air preview as the panel shows it.
pub struct PreviewOverlay {
    pub texture: egui::TextureId,
    /// Texture size in pixels.
    pub size: (u32, u32),
    pub label: String,
}

/// Display width of the preview overlay, in points.
const PREVIEW_WIDTH: f32 = 320.0;

/// One-shot actions requested by the user this frame.
#[derive(Default)]
pub struct UiActions {
    pub clear_feedback: bool,
    /// Switch the canvas to this size.
    pub apply_canvas: Option<(u32, u32)>,
    pub start_recording: bool,
    pub save_still: bool,
    pub stop_recording: bool,
    pub choose_folder: bool,
    pub files: FileActions,
    pub inputs: InputActions,
    /// Look for MIDI devices again.
    pub refresh_midi: bool,
    /// What the Audio section asked for.
    pub audio: AudioActions,
    /// The part of the window left for the canvas, in points (beside the panel).
    pub canvas_rect: Option<egui::Rect>,
}

/// F12 saves a still of the canvas.
const SAVE_STILL: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::F12);

/// Tab hides and shows the control panel; not while a text field has the keyboard.
const TOGGLE_PANEL: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Tab);

/// The panel's title: the white "rasterwarp" wordmark.
const TITLE_PNG: &[u8] = include_bytes!("../icon/rasterwarp_title.png");

/// How tall the title wordmark is drawn, in points: the size of the heading text it replaced.
const TITLE_HEIGHT: f32 = 18.0;

/// The panel's title, tinted to the theme's heading color; the plain heading if the
/// wordmark can't be decoded.
fn title(ui: &mut Ui) {
    let id = egui::Id::new("rasterwarp title");
    let texture = ui
        .ctx()
        .data_mut(|d| d.get_temp::<Option<egui::TextureHandle>>(id));
    let texture = texture.unwrap_or_else(|| {
        let texture = match image::load_from_memory(TITLE_PNG) {
            Ok(png) => {
                let png = png.to_rgba8();
                let size = [png.width() as usize, png.height() as usize];
                let pixels = egui::ColorImage::from_rgba_unmultiplied(size, png.as_raw());
                Some(
                    ui.ctx()
                        .load_texture("title", pixels, egui::TextureOptions::LINEAR),
                )
            }
            Err(err) => {
                log::warn!("no title image: {err}");
                None
            }
        };
        ui.ctx().data_mut(|d| d.insert_temp(id, texture.clone()));
        texture
    });
    match texture {
        Some(texture) => {
            let size = texture.size_vec2() * (TITLE_HEIGHT / texture.size_vec2().y);
            ui.add(
                egui::Image::new((texture.id(), size))
                    .tint(ui.visuals().strong_text_color())
                    .alt_text("Rasterwarp"),
            );
        }
        None => {
            ui.heading("Rasterwarp");
        }
    }
}

/// The Theme drop-down beside the title.
fn theme_picker(ui: &mut Ui, theme: &mut ThemeChoice) {
    ComboBox::from_id_salt("theme")
        .selected_text(theme.label())
        .width(70.0)
        .show_ui(ui, |ui| {
            for choice in ThemeChoice::ALL {
                ui.selectable_value(theme, choice, choice.label());
            }
        })
        .response
        .on_hover_text("The panel's appearance; System follows Windows' app mode");
}

pub fn draw(
    ui: &mut Ui,
    motion: &mut Motion,
    state: &mut UiState,
    preview: Option<&PreviewOverlay>,
) -> UiActions {
    let mut actions = UiActions::default();
    files_ui::shortcuts(ui.ctx(), &mut actions.files);
    if ui.input_mut(|i| i.consume_shortcut(&SAVE_STILL)) {
        actions.save_still = true;
    }
    for role in Role::ALL {
        let key = egui::KeyboardShortcut::new(egui::Modifiers::NONE, inputs_ui::loop_key(role));
        if state.inputs[role.index()].running == crate::inputs::Kind::Camera
            && ui.input_mut(|i| i.consume_shortcut(&key))
        {
            actions.inputs.toggle_loop = Some(role);
        }
    }
    if state.midi.links.learning().is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.midi.links.cancel();
    }
    if !ui.ctx().egui_wants_keyboard_input() && ui.input_mut(|i| i.consume_shortcut(&TOGGLE_PANEL))
    {
        state.panel_hidden = !state.panel_hidden;
    }
    let was_open = !state.panel_hidden;
    let mut open = was_open;
    egui::Panel::left("controls")
        .resizable(true)
        .default_size(320.0)
        .show_collapsible(ui, &mut open, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.horizontal(|ui| {
                    title(ui);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        theme_picker(ui, &mut state.theme);
                    });
                });
                ui.small("Developed by Anthony C. Bartman (@rommix0)");
                ui.label(format!(
                    "{} · display {:.0} Hz · {} late",
                    state.rate.label(),
                    1000.0 / state.frame_ms.max(0.001),
                    state.late
                ));
                if let Some(err) = &state.load_error {
                    ui.colored_label(theme::error(ui), err);
                }
                if let Some(err) = &state.file_error {
                    ui.colored_label(theme::error(ui), err);
                }
                if let Some(note) = &state.file_note {
                    ui.label(note);
                }
                if let Some(note) = &state.session_note {
                    ui.label(note);
                }
                midi_ui::learning_line(ui, &mut state.midi.links);
                ui.label("Drop an image, video, preset or project onto the window.");
                ui.horizontal(|ui| {
                    let pause = ui.checkbox(&mut state.paused, "Pause");
                    link_ui::link_menu(
                        &pause,
                        &mut state.midi.links,
                        Target::Action(Action::Pause),
                    );
                    ui.checkbox(&mut state.show_preview, "Show preview");
                    if ui.button("Reset all").clicked() {
                        *motion.editable() = Params::default();
                        actions.clear_feedback = true;
                    }
                    if ui
                        .button("Hide panel")
                        .on_hover_text("Show the whole canvas (Tab)")
                        .clicked()
                    {
                        state.panel_hidden = true;
                    }
                });
                let recording = state.capture.status.is_some();
                files_ui::project_row(ui, &state.files, recording, &mut actions.files);
                capture_section(ui, &mut state.capture, &mut state.midi.links, &mut actions);
                rate_section(ui, &mut state.rate, recording);
                canvas_section(ui, &mut state.canvas, recording, &mut actions);
                actions.refresh_midi = midi_ui::midi_section(ui, &mut state.midi);
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
                mode_section(ui, motion, &mut state.midi.links);
                for role in Role::ALL {
                    inputs_ui::input_section(
                        ui,
                        role,
                        &mut state.inputs[role.index()],
                        state.cameras.as_deref(),
                        motion.editable(),
                        &mut state.midi.links,
                        &mut actions.inputs,
                    );
                }
                files_ui::presets_section(
                    ui,
                    &mut state.files,
                    &state.presets_folder,
                    &mut actions.files,
                );
                curves_section(ui, &mut motion.curves, state);
                let params = motion.editable();
                let links = &mut state.midi.links;
                warp_section(ui, params, links);
                raster_section(ui, params, links);
                colorize_section(ui, params, links);
                key_section(ui, params);
                feedback_section(ui, params, links, &mut actions);
                glow_section(ui, params, links);
            });
        });
    // The panel's edge can also be dragged shut or open.
    if open != was_open {
        state.panel_hidden = !open;
    }
    actions.canvas_rect = Some(ui.available_rect_before_wrap());
    if state.panel_hidden {
        show_panel_button(ui.ctx(), state);
    }
    if let Some(preview) = preview {
        preview_overlay(ui.ctx(), preview);
    }
    actions
}

/// While the panel is hidden, a button in the top-left corner brings it back.
fn show_panel_button(ctx: &egui::Context, state: &mut UiState) {
    egui::Area::new(egui::Id::new("show panel"))
        .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0])
        .show(ctx, |ui| {
            if ui
                .button("Show panel")
                .on_hover_text("Show the controls (Tab)")
                .clicked()
            {
                state.panel_hidden = false;
            }
        });
}

/// The preview, framed and labelled, in the bottom-right corner of the window.
fn preview_overlay(ctx: &egui::Context, preview: &PreviewOverlay) {
    let height = PREVIEW_WIDTH * preview.size.1 as f32 / preview.size.0 as f32;
    egui::Area::new(egui::Id::new("preview"))
        .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(&preview.label);
                ui.image((preview.texture, egui::vec2(PREVIEW_WIDTH, height)));
            });
        });
}

fn capture_section(
    ui: &mut Ui,
    capture: &mut CaptureUi,
    links: &mut Links,
    actions: &mut UiActions,
) {
    CollapsingHeader::new("Capture")
        .default_open(true)
        .show(ui, |ui| {
            let settings = &mut capture.settings;
            ui.add_enabled_ui(capture.status.is_none(), |ui| {
                ComboBox::from_id_salt("capture format")
                    .selected_text(settings.format.label())
                    .show_ui(ui, |ui| {
                        for format in VideoFormat::ALL {
                            ui.selectable_value(&mut settings.format, format, format.label());
                        }
                    });
                // A checked box stays enabled under HEVC so it can be turned off again.
                let alpha_ok = settings.format.carries_alpha();
                ui.add_enabled(
                    alpha_ok || settings.alpha,
                    egui::Checkbox::new(&mut settings.alpha, "Transparent background (alpha)"),
                )
                .on_hover_text(
                    "Recordings and stills leave the background out: keyed levels are \
                     see-through, for compositing over other video",
                );
                if !alpha_ok {
                    let note = "HEVC can't carry alpha; choose ProRes 4444 or FFV1";
                    if settings.alpha {
                        ui.colored_label(theme::note(ui), note);
                    } else {
                        ui.small(note);
                    }
                }
                ComboBox::from_id_salt("capture mode")
                    .selected_text(settings.mode.label())
                    .show_ui(ui, |ui| {
                        for mode in CaptureMode::ALL {
                            ui.selectable_value(&mut settings.mode, mode, mode.label());
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("Stop after");
                    ui.add(
                        egui::DragValue::new(&mut settings.stop_after)
                            .range(0.0..=3600.0)
                            .speed(0.1)
                            .suffix(" s"),
                    );
                    ui.small("(0 = stop by hand)");
                });
                ui.horizontal(|ui| {
                    ui.label(format!("Folder: {}", settings.folder.display()));
                    if ui.button("Choose…").clicked() {
                        actions.choose_folder = true;
                    }
                });
            });
            match &capture.status {
                None => {
                    let record = ui.button("Record");
                    if record.clicked() {
                        actions.start_recording = true;
                    }
                    link_ui::link_menu(&record, links, Target::Action(Action::Record));
                }
                Some(status) => {
                    let stop = ui.button("Stop recording");
                    if stop.clicked() {
                        actions.stop_recording = true;
                    }
                    link_ui::link_menu(&stop, links, Target::Action(Action::Record));
                    ui.label(format!(
                        "{:.1} s · {} frames · {} dropped",
                        status.seconds, status.frames, status.dropped
                    ));
                    if let Some(speed) = status.speed {
                        ui.label(format!("offline: {speed:.2}× realtime"));
                    } else if status.dropped > 0 {
                        ui.colored_label(
                            theme::note(ui),
                            "The encoder can't keep up, so frames are being dropped. Offline mode records every frame.",
                        );
                    }
                }
            }
            let still = ui
                .button("Save still")
                .on_hover_text("Save the canvas as a PNG in the captures folder (F12)");
            if still.clicked() {
                actions.save_still = true;
            }
            link_ui::link_menu(&still, links, Target::Action(Action::SaveStill));
            if let Some(saved) = &capture.saved {
                ui.small(saved);
            }
            if let Some(err) = &capture.error {
                ui.colored_label(theme::error(ui), err);
            }
        });
}

/// The program frame rate: canvas frames and recordings both run at it.
fn rate_section(ui: &mut Ui, rate: &mut FrameRate, recording: bool) {
    ui.add_enabled_ui(!recording, |ui| {
        ui.horizontal(|ui| {
            ui.label("Frame rate");
            ComboBox::from_id_salt("frame rate")
                .selected_text(rate.label())
                .show_ui(ui, |ui| {
                    for choice in FrameRate::ALL {
                        ui.selectable_value(rate, choice, choice.label());
                    }
                });
        });
    });
}

fn canvas_section(
    ui: &mut Ui,
    canvas: &mut CanvasChoice,
    recording: bool,
    actions: &mut UiActions,
) {
    CollapsingHeader::new("Canvas").show(ui, |ui| {
        let name = |i: usize| match PRESETS.get(i) {
            Some((name, (w, h))) => format!("{name} ({w}×{h})"),
            None => "Custom".to_owned(),
        };
        ComboBox::from_id_salt("canvas preset")
            .selected_text(name(canvas.choice))
            .show_ui(ui, |ui| {
                for i in 0..=CanvasChoice::CUSTOM {
                    ui.selectable_value(&mut canvas.choice, i, name(i));
                }
            });
        if canvas.choice == CanvasChoice::CUSTOM {
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut canvas.custom.0).range(1..=canvas.max_side));
                ui.label("×");
                ui.add(egui::DragValue::new(&mut canvas.custom.1).range(1..=canvas.max_side));
            });
        }
        let (w, h) = canvas.requested();
        let (cw, ch) = canvas.current;
        ui.label(format!("Rendering at {cw}×{ch}"));
        ui.horizontal(|ui| {
            let changed = (w, h) != canvas.current;
            if ui
                .add_enabled(changed && !recording, Button::new(format!("Apply {w}×{h}")))
                .clicked()
            {
                actions.apply_canvas = Some((w, h));
            }
            ui.small(if recording {
                "stop recording to change"
            } else {
                "clears the trails"
            });
        });
    });
}

fn mode_section(ui: &mut Ui, motion: &mut Motion, links: &mut Links) {
    ui.separator();
    ui.horizontal(|ui| {
        for mode in Mode::ALL {
            let label = ui.selectable_label(motion.mode() == mode, format!("{mode:?}"));
            if label.clicked() {
                motion.set_mode(mode);
            }
            link_ui::link_menu(&label, links, Target::Mode(mode));
        }
    });
    match motion.mode() {
        Mode::Live => {
            ui.label("Editing what's on screen.");
        }
        Mode::Transition => transition_controls(ui, motion, links),
        Mode::Sequence => sequence_controls(ui, motion, links),
    }
    ui.separator();
}

fn transition_controls(ui: &mut Ui, motion: &mut Motion, links: &mut Links) {
    let ab = &motion.ab;
    ui.label(format!(
        "On air: {} · editing: {}",
        AbState::bank_name(ab.on_air),
        AbState::bank_name(ab.off_air())
    ));
    let label = match ab.ramp() {
        Some(r) if r.forward => "Reverse (Space)",
        Some(_) => "Resume (Space)",
        None => "Transition (Space)",
    };
    ui.horizontal(|ui| {
        let transition = ui.button(label);
        if transition.clicked() {
            motion.trigger();
        }
        link_ui::link_menu(&transition, links, Target::Action(Action::Transition));
        let cut = ui.button("Cut");
        if cut.clicked() {
            motion.cut();
        }
        link_ui::link_menu(&cut, links, Target::Action(Action::Cut));
    });
    let progress = motion.ab.ramp().map_or(0.0, |r| r.progress);
    ui.add(ProgressBar::new(progress).show_percentage());
    link_ui::duration(ui, links, &mut motion.ab.duration);
    curve_picker(ui, "ab curve", &motion.curves, &mut motion.ab.curve);
}

fn sequence_controls(ui: &mut Ui, motion: &mut Motion, links: &mut Links) {
    let curves = motion.curves.clone();
    let seq = motion.sequence_mut();
    ui.horizontal(|ui| {
        let run = if seq.is_running() {
            let stop = ui.button("Stop");
            if stop.clicked() {
                seq.stop();
            }
            stop
        } else {
            let run = ui.button("Run");
            if run.clicked() {
                seq.run();
            }
            run
        };
        link_ui::link_menu(&run, links, Target::Action(Action::SequenceRun));
        let reset = ui.button("Reset");
        if reset.clicked() {
            seq.reset();
        }
        link_ui::link_menu(&reset, links, Target::Action(Action::SequenceReset));
        ui.checkbox(&mut seq.looping, "Loop");
    });
    let frame = seq.clock_frames();
    ui.label(format!(
        "Frame {:.0} ({:.2} s at {FRAMES_PER_SECOND} fps)",
        frame,
        frame / FRAMES_PER_SECOND
    ));
    for i in 0..seq.cues().len() {
        let text = format!("Cue {} @ frame {}", i + 1, seq.cues()[i].start_frame);
        if ui.selectable_label(seq.selected == i, text).clicked() {
            seq.selected = i;
        }
    }
    ui.horizontal(|ui| {
        if ui.button("Add cue").clicked() {
            seq.add_cue();
        }
        let can_delete = seq.cues().len() > 1;
        if ui
            .add_enabled(can_delete, Button::new("Delete cue"))
            .clicked()
        {
            seq.delete_cue();
        }
    });
    let i = seq.selected;
    let mut start = seq.cues()[i].start_frame;
    let mut duration = seq.cues()[i].duration_frames;
    // Cue 1 always starts at frame 0 and is never ramped into, so its controls stay disabled.
    ui.add_enabled_ui(i > 0, |ui| {
        ui.horizontal(|ui| {
            if ui
                .add(Slider::new(&mut start, 0..=MAX_FRAME).text("start frame"))
                .changed()
            {
                seq.set_start_frame(i, start);
            }
            ui.label(format!("{:.2} s", start as f32 / FRAMES_PER_SECOND));
        });
        ui.horizontal(|ui| {
            if ui
                .add(Slider::new(&mut duration, 1..=MAX_FRAME).text("ramp frames"))
                .changed()
            {
                seq.set_duration(i, duration);
            }
            ui.label(format!("{:.2} s", duration as f32 / FRAMES_PER_SECOND));
        });
        curve_picker(ui, "cue curve", &curves, &mut seq.selected_cue_mut().curve);
    });
    if seq.is_running() {
        ui.label("Running: the panel still edits the selected cue.");
    }
}

fn curve_picker(ui: &mut Ui, id: &str, curves: &CurveLibrary, curve: &mut CurveRef) {
    ComboBox::from_id_salt(id)
        .selected_text(format!("curve: {}", curves.name(*curve)))
        .show_ui(ui, |ui| {
            for choice in curves.choices() {
                ui.selectable_value(curve, choice, curves.name(choice));
            }
        });
}

fn curves_section(ui: &mut Ui, curves: &mut CurveLibrary, state: &mut UiState) {
    CollapsingHeader::new("Curves").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            for c in &curves.custom {
                if ui
                    .selectable_label(state.editing_curve == Some(c.id), &c.name)
                    .clicked()
                {
                    state.editing_curve = Some(c.id);
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("New curve").clicked() {
                state.editing_curve = curves.add().or(state.editing_curve);
            }
            if let Some(id) = state.editing_curve
                && ui.button("Delete").clicked()
            {
                curves.remove(id);
                state.editing_curve = None;
            }
        });
        if let Some(curve) = state.editing_curve.and_then(|id| curves.get_mut(id)) {
            ui.text_edit_singleline(&mut curve.name);
            curve_editor(ui, curve);
            ui.small("Drag points · double-click to add · right-click to remove");
        }
    });
}

fn warp_section(ui: &mut Ui, params: &mut Params, links: &mut Links) {
    CollapsingHeader::new("Deflection")
        .default_open(true)
        .show(ui, |ui| {
            for (id, text) in [
                (SliderId::Zoom, "zoom"),
                (SliderId::Rotation, "rotation"),
                (SliderId::OffsetX, "offset x"),
                (SliderId::OffsetY, "offset y"),
                (SliderId::Drift, "analog drift"),
                (SliderId::AxisWander, "axis wander"),
                (SliderId::LineJitter, "line jitter (px)"),
            ] {
                link_ui::slider(ui, links, params, id, text);
            }
            ui.checkbox(
                &mut params.warp.slave_4_to_3,
                "Slave osc 4 to osc 3 (sine/cosine pair)",
            );
            let slaved = params.warp.slave_4_to_3;
            for i in 0..OSCILLATOR_COUNT {
                // A slaved oscillator 4 keeps only its own target, amplitude and envelope.
                let follows = slaved && i == 3;
                CollapsingHeader::new(format!("Oscillator {}", i + 1))
                    .default_open(i == 0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            link_ui::option(ui, links, params, ChoiceId::Axis(i), 0, "→ X");
                            link_ui::option(ui, links, params, ChoiceId::Axis(i), 1, "→ Y");
                            let envelopes = Envelope::ALL.map(|e| format!("{e:?}"));
                            link_ui::combo(ui, links, params, ChoiceId::Envelope(i), &envelopes);
                        });
                        link_ui::slider(
                            ui,
                            links,
                            params,
                            SliderId::Osc(i, OscSlider::Amplitude),
                            "amplitude",
                        );
                        ui.add_enabled_ui(!follows, |ui| oscillator_shape(ui, i, params, links));
                    });
            }
        });
}

/// The controls a slaved oscillator 4 takes from oscillator 3.
fn oscillator_shape(ui: &mut Ui, i: usize, params: &mut Params, links: &mut Links) {
    ui.horizontal(|ui| {
        let waveforms = Waveform::ALL.map(|w| format!("{w:?}"));
        link_ui::combo(ui, links, params, ChoiceId::Waveform(i), &waveforms);
        let inputs = OscInput::ALL.map(|input| format!("by {input:?}"));
        link_ui::combo(ui, links, params, ChoiceId::Input(i), &inputs);
        let syncs = OscSync::ALL.map(|sync| format!("{sync:?} sync"));
        link_ui::combo(ui, links, params, ChoiceId::Sync(i), &syncs);
    });
    for (slider, text) in [
        (OscSlider::Frequency, "frequency"),
        (OscSlider::Phase, "phase"),
        (OscSlider::PhaseSpeed, "phase speed"),
        (OscSlider::LfoRate, "LFO rate"),
        (OscSlider::LfoDepth, "LFO depth"),
    ] {
        link_ui::slider(ui, links, params, SliderId::Osc(i, slider), text);
    }
}

fn raster_section(ui: &mut Ui, params: &mut Params, links: &mut Links) {
    CollapsingHeader::new("Raster").show(ui, |ui| {
        ui.checkbox(&mut params.raster.enabled, "True raster (scan lines)")
            .on_hover_text(
                "Draws the source as real scan lines bent by the oscillators. Warp mode \
                 maps each pixel back to the source and raster mode maps each line forward, \
                 so the same settings give mirror-image ripples.",
            );
        ui.add_enabled_ui(params.raster.enabled, |ui| {
            for (id, text) in [
                (SliderId::RasterLines, "lines"),
                (SliderId::BeamWidth, "beam width (px)"),
                (SliderId::Compensation, "area compensation"),
                (SliderId::SpeedCompensation, "speed compensation"),
            ] {
                link_ui::slider(ui, links, params, id, text);
            }
        });
    });
}

fn colorize_section(ui: &mut Ui, params: &mut Params, links: &mut Links) {
    CollapsingHeader::new("Colorize")
        .default_open(true)
        .show(ui, |ui| {
            ui.checkbox(&mut params.colorize.bypass, "Bypass (grayscale)");
            // A new level count starts from evenly spaced thresholds, and levels that are
            // gone stop being see-through: the slider's setter does both.
            link_ui::slider(ui, links, params, SliderId::Levels, "levels");
            ui.horizontal(|ui| {
                ui.label("Thresholds");
                if ui.button("Even").clicked() {
                    params.colorize.thresholds = even_thresholds(params.colorize.levels);
                }
            });
            let used = params.colorize.levels as usize - 1;
            for k in 0..used {
                // Shown to 3 decimals but not rounded: a slider that rounds its value
                // changes the look just by being drawn (and it would always look unsaved).
                link_ui::slider_shown(
                    ui,
                    links,
                    params,
                    SliderId::LevelFrom(k),
                    &format!("level {} from", k + 2),
                    ranges::THRESHOLD,
                    |s| s.custom_formatter(|v, _| format!("{v:.3}")),
                );
            }
            for (id, text) in [
                (SliderId::Softness, "softness"),
                (SliderId::Fringe, "edge fringe (px)"),
                (SliderId::Ringing, "ringing"),
                (SliderId::CycleSpeed, "cycle speed"),
            ] {
                link_ui::slider(ui, links, params, id, text);
            }
            ui.horizontal_wrapped(|ui| {
                for color in &mut params.colorize.palette {
                    // The button edits a copy: its color conversion isn't exact, and only
                    // a real edit may change the look.
                    let mut edited = *color;
                    if ui.color_edit_button_rgb(&mut edited).changed() {
                        *color = edited;
                    }
                }
            });
        });
}

fn key_section(ui: &mut Ui, params: &mut Params) {
    let levels = params.colorize.levels;
    let k = &mut params.key;
    CollapsingHeader::new("Keying").show(ui, |ui| {
        ui.checkbox(&mut k.enabled, "Level keying");
        ui.add_enabled_ui(k.enabled, |ui| {
            ui.label("See-through levels (darkest first):");
            ui.horizontal_wrapped(|ui| {
                for level in 0..levels {
                    let bit = 1u8 << level;
                    let mut on = k.levels & bit != 0;
                    if ui.checkbox(&mut on, format!("{}", level + 1)).changed() {
                        k.levels ^= bit;
                    }
                }
            });
        });
        ui.small("See-through levels show the Background input.");
    });
}

fn feedback_section(ui: &mut Ui, params: &mut Params, links: &mut Links, actions: &mut UiActions) {
    CollapsingHeader::new("Feedback")
        .default_open(true)
        .show(ui, |ui| {
            for (id, text) in [
                (SliderId::FeedbackAmount, "amount"),
                (SliderId::FeedbackZoom, "zoom"),
                (SliderId::FeedbackRotation, "rotation"),
                (SliderId::FeedbackOffsetX, "offset x"),
                (SliderId::FeedbackOffsetY, "offset y"),
            ] {
                link_ui::slider(ui, links, params, id, text);
            }
            let clear = ui.button("Clear trails");
            if clear.clicked() {
                actions.clear_feedback = true;
            }
            link_ui::link_menu(&clear, links, Target::Action(Action::ClearTrails));
        });
}

fn glow_section(ui: &mut Ui, params: &mut Params, links: &mut Links) {
    CollapsingHeader::new("Glow & CRT")
        .default_open(true)
        .show(ui, |ui| {
            for (id, text) in [
                (SliderId::Bloom, "bloom"),
                (SliderId::BloomThreshold, "bloom threshold"),
                (SliderId::Scanlines, "scanlines"),
                (SliderId::ScanlineCount, "scanline count"),
                (SliderId::ChromaBleed, "chroma bleed"),
                (SliderId::Noise, "noise"),
            ] {
                link_ui::slider(ui, links, params, id, text);
            }
        });
}
