//! The panel's Audio section: the sound source, its settings, a meter, hand-beat
//! buttons and the audio links. It only reports what was asked for; the app does it.

use std::time::{Duration, Instant};

use egui::{CollapsingHeader, Color32, ComboBox, DragValue, ProgressBar, Slider, Ui};

use crate::audio::{
    ATTACK, Beat, FileState, GAIN, RELEASE, SENSITIVITY, Shaping, Signal, Signals, SourceChoice,
};
use crate::control::{Action, Links, Target};
use crate::link_ui;
use crate::params::Params;
use crate::params::table::SliderId;

/// How long a beat light stays lit.
const LIT: Duration = Duration::from_millis(150);

/// The kinds of source the section offers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceKind {
    /// No sound.
    #[default]
    None,
    /// An input device, such as a microphone.
    Input,
    /// What an output device is playing.
    Loopback,
    /// A sound file.
    File,
}

impl SourceKind {
    /// Every kind, in the order the section shows them.
    pub const ALL: [SourceKind; 4] = [
        SourceKind::None,
        SourceKind::Input,
        SourceKind::Loopback,
        SourceKind::File,
    ];

    /// Its name in the panel.
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::None => "None",
            SourceKind::Input => "Input",
            SourceKind::Loopback => "Loopback",
            SourceKind::File => "File",
        }
    }

    /// The kind of `choice`.
    pub fn of(choice: &SourceChoice) -> SourceKind {
        match choice {
            SourceChoice::None => SourceKind::None,
            SourceChoice::Input(_) => SourceKind::Input,
            SourceChoice::Loopback(_) => SourceKind::Loopback,
            SourceChoice::File(_) => SourceKind::File,
        }
    }
}

/// What the Audio section shows and edits, kept between frames.
#[derive(Clone, Debug)]
pub struct AudioUi {
    /// Whether Record restarts the sound file from its start.
    pub start_with_recording: bool,
    /// The kind of source the section is set to show.
    pub kind: SourceKind,
    /// The input devices listed; none until they're first needed.
    pub inputs: Option<Vec<String>>,
    /// The output devices listed (for loopback and for the speakers).
    pub outputs: Option<Vec<String>>,
    /// Why the devices couldn't be listed.
    pub listing_error: Option<String>,
    /// The source the engine is using.
    pub chosen: SourceChoice,
    /// The loaded file's state.
    pub file: Option<FileState>,
    /// The file loading and how far it has got.
    pub loading: Option<String>,
    /// Why the source or the speakers aren't working.
    pub error: Option<String>,
    /// How the signals are shaped.
    pub shaping: Shaping,
    /// The speakers' volume, 0 to 1.
    pub volume: f32,
    /// Whether the file starts over when it ends.
    pub looping: bool,
    /// The speakers' output device; empty for the system default.
    pub output: String,
    /// The last canvas frame's signals, for the meter.
    pub meter: Signals,
    /// When each beat last fired, for the beat lights.
    pub lit: [Option<Instant>; 3],
}

impl Default for AudioUi {
    fn default() -> Self {
        Self {
            start_with_recording: true,
            kind: SourceKind::None,
            inputs: None,
            outputs: None,
            listing_error: None,
            chosen: SourceChoice::None,
            file: None,
            loading: None,
            error: None,
            shaping: Shaping::default(),
            volume: 1.0,
            looping: true,
            output: String::new(),
            meter: Signals::default(),
            lit: [None; 3],
        }
    }
}

/// What the Audio section asked for this frame.
#[derive(Default)]
pub struct AudioActions {
    /// Use this source.
    pub use_source: Option<SourceChoice>,
    /// Ask for a sound file to open.
    pub choose_file: bool,
    /// List the input and output devices (a view needs them; nothing is retried).
    pub list_devices: bool,
    /// Refresh: list the devices again and retry the source and the speakers.
    pub refresh: bool,
    /// Play (true) or pause (false) the file.
    pub play: Option<bool>,
    /// Start the file over.
    pub restart: bool,
    /// Move the file's playhead to this many seconds.
    pub seek: Option<f64>,
    /// Fire this beat by hand.
    pub tap: Option<Beat>,
}

/// The Audio section (collapsed by default).
pub fn audio_section(
    ui: &mut Ui,
    audio: &mut AudioUi,
    links: &mut Links,
    base: &Params,
    offsets: &[(SliderId, f32)],
    actions: &mut AudioActions,
) {
    CollapsingHeader::new("Audio").show(ui, |ui| {
        source(ui, audio, actions);
        if let Some(error) = &audio.error {
            ui.colored_label(crate::theme::error(ui), error);
        }
        ui.add(Slider::new(&mut audio.shaping.gain, GAIN).text("gain"));
        ui.add(
            Slider::new(&mut audio.shaping.attack, ATTACK)
                .logarithmic(true)
                .text("attack (s)"),
        );
        ui.add(
            Slider::new(&mut audio.shaping.release, RELEASE)
                .logarithmic(true)
                .text("release (s)"),
        );
        ui.add(Slider::new(&mut audio.shaping.sensitivity, SENSITIVITY).text("beat sensitivity"));
        meter(ui, audio);
        ui.horizontal(|ui| {
            for (beat, action, text) in [
                (Beat::Any, Action::Beat, "Beat"),
                (Beat::Bass, Action::BassBeat, "Bass beat"),
                (Beat::Treble, Action::TrebleBeat, "Treble beat"),
            ] {
                let tap = ui.button(text).on_hover_text("Fire this beat by hand");
                if tap.clicked() {
                    actions.tap = Some(beat);
                }
                link_ui::link_menu(&tap, links, Target::Action(action));
            }
        });
        link_list(ui, links, base, offsets);
    });
}

/// The source: its kind, then what that kind needs (a device, or a file and its
/// transport).
fn source(ui: &mut Ui, audio: &mut AudioUi, actions: &mut AudioActions) {
    ui.horizontal(|ui| {
        for kind in SourceKind::ALL {
            let picked = ui.selectable_value(&mut audio.kind, kind, kind.label());
            if picked.clicked() && kind == SourceKind::None {
                actions.use_source = Some(SourceChoice::None);
            }
        }
    });
    match audio.kind {
        SourceKind::None => {}
        SourceKind::Input | SourceKind::Loopback => device(ui, audio, actions),
        SourceKind::File => file(ui, audio, actions),
    }
}

/// The device drop-down for an input or a loopback, with Refresh.
fn device(ui: &mut Ui, audio: &mut AudioUi, actions: &mut AudioActions) {
    let input = audio.kind == SourceKind::Input;
    let listed = if input { &audio.inputs } else { &audio.outputs };
    let Some(devices) = listed else {
        actions.list_devices = true;
        ui.label("Looking for devices…");
        return;
    };
    let current = match &audio.chosen {
        SourceChoice::Input(name) | SourceChoice::Loopback(name)
            if SourceKind::of(&audio.chosen) == audio.kind =>
        {
            name.as_str()
        }
        _ => "Choose a device",
    };
    let mut picked = None;
    ui.horizontal(|ui| {
        ComboBox::from_id_salt("audio device")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for name in devices {
                    if ui.selectable_label(current == name, name).clicked() {
                        picked = Some(name.clone());
                    }
                }
            });
        if ui.button("Refresh").clicked() {
            actions.refresh = true;
        }
    });
    if devices.is_empty() {
        ui.label("No devices");
    }
    if let Some(error) = &audio.listing_error {
        ui.colored_label(crate::theme::error(ui), error);
    }
    if let Some(name) = picked {
        actions.use_source = Some(if input {
            SourceChoice::Input(name)
        } else {
            SourceChoice::Loopback(name)
        });
    }
}

/// Opening a sound file, and playing it.
fn file(ui: &mut Ui, audio: &mut AudioUi, actions: &mut AudioActions) {
    if audio.outputs.is_none() {
        // The speakers' drop-down needs the output devices.
        actions.list_devices = true;
    }
    if ui.button("Open…").clicked() {
        actions.choose_file = true;
    }
    if let Some(loading) = &audio.loading {
        ui.label(loading);
    }
    if let Some(file) = &audio.file {
        ui.label(&file.name);
        ui.horizontal(|ui| {
            let text = if file.playing { "Pause" } else { "Play" };
            if ui.button(text).clicked() {
                actions.play = Some(!file.playing);
            }
            if ui.button("Restart").clicked() {
                actions.restart = true;
            }
            ui.checkbox(&mut audio.looping, "Loop");
        });
        let mut position = file.position;
        let seek = Slider::new(&mut position, 0.0..=file.length.max(0.001)).text("position (s)");
        if ui.add(seek).changed() {
            actions.seek = Some(position);
        }
    }
    ui.add(Slider::new(&mut audio.volume, 0.0..=1.0).text("volume"));
    let shown = if audio.output.is_empty() {
        "System default".to_string()
    } else {
        audio.output.clone()
    };
    ui.horizontal(|ui| {
        ComboBox::from_label("output")
            .selected_text(shown)
            .show_ui(ui, |ui| {
                let default = ui.selectable_label(audio.output.is_empty(), "System default");
                if default.clicked() {
                    audio.output.clear();
                }
                for name in audio.outputs.iter().flatten() {
                    if ui.selectable_label(audio.output == *name, name).clicked() {
                        audio.output = name.clone();
                    }
                }
            });
        if ui
            .button("Refresh")
            .on_hover_text("List the devices again and reopen the sound output")
            .clicked()
        {
            actions.refresh = true;
        }
    });
    ui.checkbox(
        &mut audio.start_with_recording,
        "Start the sound with the recording",
    );
}

/// A bar for each signal, and a light for each beat.
fn meter(ui: &mut Ui, audio: &AudioUi) {
    for signal in Signal::ALL {
        ui.add(ProgressBar::new(audio.meter.get(signal)).text(signal.label()));
    }
    let now = Instant::now();
    ui.horizontal(|ui| {
        for beat in Beat::ALL {
            let lit = audio.lit[beat.index()].is_some_and(|at| now.duration_since(at) < LIT);
            let colour = if lit {
                crate::theme::note(ui)
            } else {
                Color32::GRAY
            };
            ui.colored_label(colour, format!("● {}", beat.label()));
        }
    });
}

/// The audio links: each follow link with its depth and the value it shows now, each
/// beat link, and a Remove for both.
fn link_list(ui: &mut Ui, links: &mut Links, base: &Params, offsets: &[(SliderId, f32)]) {
    let audio = &mut links.audio;
    if audio.follow.is_empty() && audio.beats.is_empty() {
        ui.small("Right-click a slider, option or button and choose an Audio or Beat item.");
        return;
    }
    let mut remove_follow = None;
    let mut remove_beat = None;
    egui::Grid::new("audio links").striped(true).show(ui, |ui| {
        for (i, link) in audio.follow.iter_mut().enumerate() {
            ui.label(link.signal.label());
            ui.label(Target::Slider(link.slider).label());
            let span = link.span();
            ui.add(
                DragValue::new(&mut link.depth)
                    .range(-span..=span)
                    .speed(span / 200.0),
            )
            .on_hover_text("How far a full signal moves the slider; negative pulls down");
            let offset = offsets
                .iter()
                .find(|(id, _)| *id == link.slider)
                .map_or(0.0, |(_, offset)| *offset);
            let range = link.slider.range();
            let shown = (link.slider.get(base) + offset).clamp(*range.start(), *range.end());
            ui.label(format!("{shown:.2}"));
            if ui.small_button("Remove").clicked() {
                remove_follow = Some(i);
            }
            ui.end_row();
        }
        for (i, link) in audio.beats.iter().enumerate() {
            ui.label(format!("Beat: {}", link.beat.label()));
            ui.label(link.target.label());
            ui.label("");
            ui.label("");
            if ui.small_button("Remove").clicked() {
                remove_beat = Some(i);
            }
            ui.end_row();
        }
    });
    if let Some(i) = remove_follow {
        audio.follow.remove(i);
    }
    if let Some(i) = remove_beat {
        audio.beats.remove(i);
    }
}
