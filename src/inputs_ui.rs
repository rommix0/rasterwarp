//! The panel's Source and Background sections: what each input is (an image, a video
//! or a camera) and how a video or camera plays. They only report what was asked for;
//! the app opens files and cameras.

use egui::{CollapsingHeader, ComboBox, Slider, Ui};

use crate::control::Links;
use crate::inputs::Kind;
use crate::link_ui;
use crate::params::table::{ChoiceId, SliderId, VideoSlider};
use crate::params::{Between, Params, PlayMode, Role, Slit, ranges};
use crate::video::camera::{BUFFER_SECONDS, DEFAULT_BUFFER_SECONDS};

/// What an input's section shows, kept between frames.
#[derive(Clone, Debug)]
pub struct InputUi {
    /// The kind picked in the panel; it takes over once a file or camera is chosen.
    pub kind: Kind,
    /// The kind actually showing.
    pub running: Kind,
    /// What's showing.
    pub info: String,
    /// While a clip loads: "Loading flip.mp4… 40%".
    pub loading: Option<String>,
    /// A camera's state: "Opening…", "Live" or "No Signal".
    pub status: Option<String>,
    /// Why a camera has no signal, or that its buffer had to be shortened.
    pub note: Option<String>,
    /// The camera picked in the list.
    pub camera: String,
    /// Seconds of frames the camera keeps.
    pub buffer_seconds: f32,
    /// The deepest slit-scan the GPU's frame ring holds, in seconds.
    pub max_depth: Option<f32>,
    /// The slit-scan map image's file name, if one is loaded.
    pub map: Option<String>,
    /// A camera loop's length, while the camera loops.
    pub looping: Option<f32>,
    /// How long a loop taken now would be: the delay on screen.
    pub loop_length: f32,
}

impl Default for InputUi {
    fn default() -> Self {
        Self {
            kind: Kind::Image,
            running: Kind::Image,
            info: String::new(),
            loading: None,
            status: None,
            note: None,
            camera: String::new(),
            buffer_seconds: DEFAULT_BUFFER_SECONDS,
            max_depth: None,
            map: None,
            looping: None,
            loop_length: 0.0,
        }
    }
}

/// One-shot requests from the input sections this frame.
#[derive(Default)]
pub struct InputActions {
    /// Ask for an image or video file.
    pub open: Option<Role>,
    /// Switch to the camera picked in the list.
    pub use_camera: Option<Role>,
    /// Show nothing: the test card, or a black background.
    pub clear: Option<Role>,
    pub choose_map: Option<Role>,
    pub clear_map: Option<Role>,
    /// List the cameras again.
    pub list_cameras: bool,
    /// Loop the camera's last delay seconds, replacing any loop.
    pub grab_loop: Option<Role>,
    /// Go back to the live camera.
    pub release_loop: Option<Role>,
    /// Loop the camera, or go back to live if it loops (F9, F10).
    pub toggle_loop: Option<Role>,
}

/// The key that loops `role`'s camera or lets its loop go.
pub fn loop_key(role: Role) -> egui::Key {
    match role {
        Role::Source => egui::Key::F9,
        Role::Background => egui::Key::F10,
    }
}

/// The Source or Background section. `cameras` is the listed devices (none until
/// listed); `params` is the edited bank, whose playback settings for this input the
/// section edits, and `links` are the MIDI links its controls can take.
pub fn input_section(
    ui: &mut Ui,
    role: Role,
    state: &mut InputUi,
    cameras: Option<&[String]>,
    params: &mut Params,
    links: &mut Links,
    actions: &mut InputActions,
) {
    CollapsingHeader::new(role.label())
        .default_open(role == Role::Source)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for kind in Kind::ALL {
                    ui.selectable_value(&mut state.kind, kind, kind.label());
                }
            });
            match state.kind {
                Kind::Image | Kind::Video => {
                    ui.horizontal(|ui| {
                        if ui.button("Open…").clicked() {
                            actions.open = Some(role);
                        }
                        let nothing = match role {
                            Role::Source => "Test card",
                            Role::Background => "No background",
                        };
                        if ui.button(nothing).clicked() {
                            actions.clear = Some(role);
                        }
                    });
                }
                Kind::Camera => camera_choice(ui, role, state, cameras, actions),
            }
            ui.small(&state.info);
            for line in [&state.loading, &state.status, &state.note]
                .into_iter()
                .flatten()
            {
                ui.small(line);
            }
            if state.running != Kind::Image {
                playback(ui, role, state, params, links, actions);
            }
        });
}

fn camera_choice(
    ui: &mut Ui,
    role: Role,
    state: &mut InputUi,
    cameras: Option<&[String]>,
    actions: &mut InputActions,
) {
    let Some(cameras) = cameras else {
        actions.list_cameras = true;
        ui.small("Looking for cameras…");
        return;
    };
    ui.horizontal(|ui| {
        let shown = if state.camera.is_empty() {
            "Choose a camera"
        } else {
            &state.camera
        };
        ComboBox::from_id_salt(("camera", role.index()))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for name in cameras {
                    ui.selectable_value(&mut state.camera, name.clone(), name);
                }
            });
        if ui.button("Refresh").clicked() {
            actions.list_cameras = true;
        }
    });
    if cameras.is_empty() {
        ui.small("No cameras found.");
    }
    ui.add(Slider::new(&mut state.buffer_seconds, BUFFER_SECONDS).text("buffer (s)"))
        .on_hover_text("Seconds of frames kept for delay and slit-scan");
    let label = if state.running == Kind::Camera {
        "Restart camera"
    } else {
        "Use camera"
    };
    if ui
        .add_enabled(!state.camera.is_empty(), egui::Button::new(label))
        .clicked()
    {
        actions.use_camera = Some(role);
    }
}

/// How a video or camera plays: it edits the same bank or cue as the rest of the panel.
fn playback(
    ui: &mut Ui,
    role: Role,
    state: &InputUi,
    params: &mut Params,
    links: &mut Links,
    actions: &mut InputActions,
) {
    ui.separator();
    let key = loop_key(role).name();
    if let Some(seconds) = state.looping {
        ui.horizontal(|ui| {
            ui.label(format!("Looping {seconds:.1} s"));
            if ui
                .button("Back to live")
                .on_hover_text(format!("Show the live camera again ({key})"))
                .clicked()
            {
                actions.release_loop = Some(role);
            }
            if ui
                .button("Loop again")
                .on_hover_text("Loop the latest frames instead")
                .clicked()
            {
                actions.grab_loop = Some(role);
            }
        });
    }
    if state.running == Kind::Video || state.looping.is_some() {
        ui.horizontal(|ui| {
            for (option, mode) in PlayMode::ALL.into_iter().enumerate() {
                link_ui::option(
                    ui,
                    links,
                    params,
                    ChoiceId::PlayMode(role),
                    option,
                    mode.label(),
                );
            }
        });
        if params.video(role).mode == PlayMode::Scrub {
            let position = SliderId::Video(role, VideoSlider::Position);
            link_ui::slider(ui, links, params, position, "position");
        } else {
            let speed = SliderId::Video(role, VideoSlider::Speed);
            link_ui::slider(ui, links, params, speed, "speed")
                .on_hover_text("× the frame rate it was filmed at; negative plays backwards");
        }
    } else {
        // Shown up to the buffer length; a longer delay from a file shows the oldest
        // frame.
        let buffer = state.buffer_seconds.min(*ranges::DELAY.end());
        link_ui::slider_shown(
            ui,
            links,
            params,
            SliderId::Video(role, VideoSlider::Delay),
            "delay (s)",
            0.0..=buffer,
            |s| s,
        );
        let length = state.loop_length;
        if ui
            .add_enabled(
                length > 0.0,
                egui::Button::new(format!("Loop last {length:.1} s")),
            )
            .on_hover_text(format!(
                "Play the last {length:.1} s over and over, carrying on from what's on \
                 screen ({key})"
            ))
            .clicked()
        {
            actions.grab_loop = Some(role);
        }
        if length <= 0.0 {
            ui.small("Set a delay to choose the loop length.");
        }
    }
    ui.horizontal(|ui| {
        ui.label("Between frames");
        for (option, between) in Between::ALL.into_iter().enumerate() {
            let choice = ChoiceId::Between(role);
            link_ui::option(ui, links, params, choice, option, between.label());
        }
    });
    ui.horizontal(|ui| {
        ui.label("Slit-scan");
        for (option, slit) in Slit::ALL.into_iter().enumerate() {
            link_ui::option(
                ui,
                links,
                params,
                ChoiceId::Slit(role),
                option,
                slit.label(),
            );
        }
    });
    if params.video(role).slit == Slit::Off {
        return;
    }
    let deepest = state
        .max_depth
        .unwrap_or(*ranges::SLIT_DEPTH.end())
        .min(*ranges::SLIT_DEPTH.end());
    // Shown up to the deepest the ring allows; a deeper value from a file is used at
    // that depth.
    link_ui::slider_shown(
        ui,
        links,
        params,
        SliderId::Video(role, VideoSlider::SlitDepth),
        "depth (s)",
        0.0..=deepest,
        |s| s,
    )
    .on_hover_text(format!("Up to {deepest:.1} s fits in the GPU's frame ring"));
    ui.checkbox(&mut params.video_mut(role).slit_flip, "Flip");
    if params.video(role).slit == Slit::Map {
        ui.horizontal(|ui| {
            if ui.button("Map image…").clicked() {
                actions.choose_map = Some(role);
            }
            if state.map.is_some() && ui.button("Clear").clicked() {
                actions.clear_map = Some(role);
            }
        });
        ui.small(
            state
                .map
                .as_deref()
                .unwrap_or("No map image: Map works as Rows."),
        );
    }
}
