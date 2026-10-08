//! The egui parameter panel.

use std::path::PathBuf;

use egui::{Button, CollapsingHeader, ComboBox, ProgressBar, Slider, Ui};

use crate::canvas::{CanvasChoice, PRESETS};
use crate::capture::CaptureMode;
use crate::capture::encode::VideoFormat;
use crate::capture::recorder::{RecordSettings, RecordStatus};
use crate::curve::{CurveLibrary, CurveRef};
use crate::curve_editor::curve_editor;
use crate::motion::{Mode, Motion};
use crate::params::{Axis, Envelope, OscInput, OscSync, Oscillator, Params, Waveform, ranges};
use crate::rate::FrameRate;
use crate::sequence::{FRAMES_PER_SECOND, MAX_FRAME};
use crate::transition::{AbState, DURATION};

/// UI-only state that isn't a render parameter.
#[derive(Default)]
pub struct UiState {
    pub paused: bool,
    /// Show the off-air preview in Transition and Sequence modes.
    pub show_preview: bool,
    /// Smoothed time between screen refreshes in milliseconds.
    pub frame_ms: f32,
    /// The program frame rate chosen in the panel.
    pub rate: FrameRate,
    /// Canvas frames skipped so far because the app fell behind.
    pub late: u64,
    pub source_info: String,
    pub load_error: Option<String>,
    /// The user curve open in the curve editor.
    pub editing_curve: Option<u32>,
    pub canvas: CanvasChoice,
    pub capture: CaptureUi,
}

/// The capture controls and what the current or last recording reported.
#[derive(Clone, Debug)]
pub struct CaptureUi {
    pub settings: RecordSettings,
    /// Progress while recording.
    pub status: Option<RecordStatus>,
    /// What the last recording saved.
    pub saved: Option<String>,
    pub error: Option<String>,
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
    pub stop_recording: bool,
    pub choose_folder: bool,
}

pub fn draw(
    ui: &mut Ui,
    motion: &mut Motion,
    state: &mut UiState,
    preview: Option<&PreviewOverlay>,
) -> UiActions {
    let mut actions = UiActions::default();
    egui::Panel::left("controls")
        .resizable(true)
        .default_size(320.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Rasterwarp");
                ui.label(format!(
                    "{} · display {:.0} Hz · {} late",
                    state.rate.label(),
                    1000.0 / state.frame_ms.max(0.001),
                    state.late
                ));
                ui.label(&state.source_info);
                if let Some(err) = &state.load_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, err);
                }
                ui.label("Drop a PNG/JPG onto the window to load it.");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut state.paused, "Pause");
                    ui.checkbox(&mut state.show_preview, "Show preview");
                    if ui.button("Reset all").clicked() {
                        *motion.editable() = Params::default();
                        actions.clear_feedback = true;
                    }
                });
                let recording = state.capture.status.is_some();
                capture_section(ui, &mut state.capture, &mut actions);
                rate_section(ui, &mut state.rate, recording);
                canvas_section(ui, &mut state.canvas, recording, &mut actions);
                mode_section(ui, motion);
                curves_section(ui, &mut motion.curves, state);
                let params = motion.editable();
                warp_section(ui, params);
                colorize_section(ui, params);
                feedback_section(ui, params, &mut actions);
                glow_section(ui, params);
            });
        });
    if let Some(preview) = preview {
        preview_overlay(ui.ctx(), preview);
    }
    actions
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

fn capture_section(ui: &mut Ui, capture: &mut CaptureUi, actions: &mut UiActions) {
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
                    if ui.button("Record").clicked() {
                        actions.start_recording = true;
                    }
                }
                Some(status) => {
                    if ui.button("Stop recording").clicked() {
                        actions.stop_recording = true;
                    }
                    ui.label(format!(
                        "{:.1} s · {} frames · {} dropped",
                        status.seconds, status.frames, status.dropped
                    ));
                    if let Some(speed) = status.speed {
                        ui.label(format!("offline: {speed:.2}× realtime"));
                    } else if status.dropped > 0 {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            "The encoder can't keep up, so frames are being dropped. Offline mode records every frame.",
                        );
                    }
                }
            }
            if let Some(saved) = &capture.saved {
                ui.small(saved);
            }
            if let Some(err) = &capture.error {
                ui.colored_label(egui::Color32::LIGHT_RED, err);
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

fn mode_section(ui: &mut Ui, motion: &mut Motion) {
    ui.separator();
    ui.horizontal(|ui| {
        for mode in Mode::ALL {
            if ui
                .selectable_label(motion.mode() == mode, format!("{mode:?}"))
                .clicked()
            {
                motion.set_mode(mode);
            }
        }
    });
    match motion.mode() {
        Mode::Live => {
            ui.label("Editing what's on screen.");
        }
        Mode::Transition => transition_controls(ui, motion),
        Mode::Sequence => sequence_controls(ui, motion),
    }
    ui.separator();
}

fn transition_controls(ui: &mut Ui, motion: &mut Motion) {
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
        if ui.button(label).clicked() {
            motion.trigger();
        }
        if ui.button("Cut").clicked() {
            motion.cut();
        }
    });
    let progress = motion.ab.ramp().map_or(0.0, |r| r.progress);
    ui.add(ProgressBar::new(progress).show_percentage());
    ui.add(
        Slider::new(&mut motion.ab.duration, DURATION)
            .text("duration (s)")
            .logarithmic(true),
    );
    curve_picker(ui, "ab curve", &motion.curves, &mut motion.ab.curve);
}

fn sequence_controls(ui: &mut Ui, motion: &mut Motion) {
    let curves = motion.curves.clone();
    let seq = motion.sequence_mut();
    ui.horizontal(|ui| {
        if seq.is_running() {
            if ui.button("Stop").clicked() {
                seq.stop();
            }
        } else if ui.button("Run").clicked() {
            seq.run();
        }
        if ui.button("Reset").clicked() {
            seq.reset();
        }
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

fn warp_section(ui: &mut Ui, params: &mut Params) {
    let warp = &mut params.warp;
    CollapsingHeader::new("Deflection")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(Slider::new(&mut warp.zoom, ranges::ZOOM).text("zoom"));
            ui.add(Slider::new(&mut warp.rotation, ranges::ROTATION).text("rotation"));
            ui.add(Slider::new(&mut warp.offset[0], ranges::OFFSET).text("offset x"));
            ui.add(Slider::new(&mut warp.offset[1], ranges::OFFSET).text("offset y"));
            ui.add(Slider::new(&mut warp.drift, ranges::DRIFT).text("analog drift"));
            ui.checkbox(
                &mut warp.slave_4_to_3,
                "Slave osc 4 to osc 3 (sine/cosine pair)",
            );
            let slaved = warp.slave_4_to_3;
            for (i, osc) in warp.oscillators.iter_mut().enumerate() {
                // A slaved oscillator 4 keeps only its own target, amplitude and envelope.
                let follows = slaved && i == 3;
                CollapsingHeader::new(format!("Oscillator {}", i + 1))
                    .default_open(i == 0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut osc.target, Axis::X, "→ X");
                            ui.selectable_value(&mut osc.target, Axis::Y, "→ Y");
                            ComboBox::from_id_salt(("envelope", i))
                                .selected_text(format!("{:?}", osc.envelope))
                                .show_ui(ui, |ui| {
                                    for e in Envelope::ALL {
                                        ui.selectable_value(&mut osc.envelope, e, format!("{e:?}"));
                                    }
                                });
                        });
                        ui.add(
                            Slider::new(&mut osc.amplitude, ranges::AMPLITUDE).text("amplitude"),
                        );
                        ui.add_enabled_ui(!follows, |ui| oscillator_shape(ui, i, osc));
                    });
            }
        });
}

/// The controls a slaved oscillator 4 takes from oscillator 3.
fn oscillator_shape(ui: &mut Ui, i: usize, osc: &mut Oscillator) {
    ui.horizontal(|ui| {
        ComboBox::from_id_salt(("waveform", i))
            .selected_text(format!("{:?}", osc.waveform))
            .show_ui(ui, |ui| {
                for w in Waveform::ALL {
                    ui.selectable_value(&mut osc.waveform, w, format!("{w:?}"));
                }
            });
        ComboBox::from_id_salt(("input", i))
            .selected_text(format!("by {:?}", osc.input))
            .show_ui(ui, |ui| {
                for input in OscInput::ALL {
                    ui.selectable_value(&mut osc.input, input, format!("by {input:?}"));
                }
            });
        ComboBox::from_id_salt(("sync", i))
            .selected_text(format!("{:?} sync", osc.sync))
            .show_ui(ui, |ui| {
                for sync in OscSync::ALL {
                    ui.selectable_value(&mut osc.sync, sync, format!("{sync:?} sync"));
                }
            });
    });
    ui.add(Slider::new(&mut osc.frequency, ranges::FREQUENCY).text("frequency"));
    ui.add(Slider::new(&mut osc.phase, ranges::PHASE).text("phase"));
    ui.add(Slider::new(&mut osc.phase_speed, ranges::PHASE_SPEED).text("phase speed"));
    ui.add(Slider::new(&mut osc.lfo_rate, ranges::LFO_RATE).text("LFO rate"));
    ui.add(Slider::new(&mut osc.lfo_depth, ranges::LFO_DEPTH).text("LFO depth"));
}

fn colorize_section(ui: &mut Ui, params: &mut Params) {
    let c = &mut params.colorize;
    CollapsingHeader::new("Colorize")
        .default_open(true)
        .show(ui, |ui| {
            ui.checkbox(&mut c.bypass, "Bypass (grayscale)");
            ui.add(Slider::new(&mut c.levels, ranges::LEVELS).text("levels"));
            ui.add(Slider::new(&mut c.softness, ranges::SOFTNESS).text("softness"));
            ui.add(Slider::new(&mut c.cycle_speed, ranges::CYCLE_SPEED).text("cycle speed"));
            ui.horizontal_wrapped(|ui| {
                for color in &mut c.palette {
                    ui.color_edit_button_rgb(color);
                }
            });
        });
}

fn feedback_section(ui: &mut Ui, params: &mut Params, actions: &mut UiActions) {
    let f = &mut params.feedback;
    CollapsingHeader::new("Feedback")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(Slider::new(&mut f.amount, ranges::FEEDBACK_AMOUNT).text("amount"));
            ui.add(Slider::new(&mut f.zoom, ranges::FEEDBACK_ZOOM).text("zoom"));
            ui.add(Slider::new(&mut f.rotation, ranges::FEEDBACK_ROTATION).text("rotation"));
            ui.add(Slider::new(&mut f.offset[0], ranges::FEEDBACK_OFFSET).text("offset x"));
            ui.add(Slider::new(&mut f.offset[1], ranges::FEEDBACK_OFFSET).text("offset y"));
            if ui.button("Clear trails").clicked() {
                actions.clear_feedback = true;
            }
        });
}

fn glow_section(ui: &mut Ui, params: &mut Params) {
    let g = &mut params.glow;
    CollapsingHeader::new("Glow & CRT")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(Slider::new(&mut g.bloom_intensity, ranges::BLOOM_INTENSITY).text("bloom"));
            ui.add(
                Slider::new(&mut g.bloom_threshold, ranges::BLOOM_THRESHOLD)
                    .text("bloom threshold"),
            );
            ui.add(
                Slider::new(&mut g.scanline_strength, ranges::SCANLINE_STRENGTH).text("scanlines"),
            );
            ui.add(
                Slider::new(&mut g.scanline_count, ranges::SCANLINE_COUNT).text("scanline count"),
            );
            ui.add(Slider::new(&mut g.chroma, ranges::CHROMA).text("chroma bleed"));
            ui.add(Slider::new(&mut g.noise, ranges::NOISE).text("noise"));
        });
}
