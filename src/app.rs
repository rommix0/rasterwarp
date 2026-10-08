//! The windowed application: owns the GPU context, renderer, parameters and UI. Each
//! screen refresh draws the canvas frames that are due at the program frame rate, then
//! shows the newest one with the UI on top.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::blend::FrameParams;
use crate::canvas::{self, CanvasChoice};
use crate::capture::CaptureMode;
use crate::capture::recorder::Recorder;
use crate::clock::{CanvasClock, Pacing};
use crate::files_ui::FileActions;
use crate::gpu;
use crate::motion::{Mode, Motion};
use crate::params::Params;
use crate::passes::Renderer;
use crate::presets;
use crate::preview::PreviewView;
use crate::save::{self, PRESET_EXTENSION, PROJECT_EXTENSION, Settings};
use crate::session::{self, Autosave, Project};
use crate::source::{self, ColorImage, GrayImage};
use crate::ui::{self, UiActions, UiState};

/// How often the session is autosaved (when it changed).
const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(60);

/// How the panel starts an autosave failure, so a later success can clear it.
const AUTOSAVE_FAILED: &str = "Autosave failed: ";

pub struct App {
    initial_image: Option<PathBuf>,
    state: Option<State>,
    error: Option<anyhow::Error>,
}

impl App {
    pub fn new(initial_image: Option<PathBuf>) -> Self {
        Self {
            initial_image,
            state: None,
            error: None,
        }
    }

    /// The startup error, if the app exited because it could not start.
    pub fn finish(self) -> Result<()> {
        match self.error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match State::new(event_loop, self.initial_image.as_deref()) {
            Ok(mut state) => {
                // Startup took a while; the first canvas frame is due now.
                state.clock.reanchor(state.seconds());
                self.state = Some(state);
            }
            Err(err) => {
                self.error = Some(err);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.state else { return };
        let response = state.egui_state.on_window_event(&state.window, &event);
        match event {
            WindowEvent::CloseRequested => {
                state.stop_recording(); // finish the file before exiting
                state.autosave();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => state.resize(size),
            WindowEvent::DroppedFile(path) => state.dropped(&path),
            // Space triggers a transition unless egui is using the keyboard (e.g. a text field).
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && !event.repeat
                    && event.logical_key == Key::Named(NamedKey::Space)
                    && !response.consumed
                    && !state.egui_ctx.egui_wants_keyboard_input() =>
            {
                state.motion.trigger();
            }
            WindowEvent::RedrawRequested => {
                state.redraw();
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    max_texture_side: u32,
    composite_format: wgpu::TextureFormat,
    renderer: Renderer,
    preview: PreviewView,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    motion: Motion,
    recorder: Option<Recorder>,
    ui: UiState,
    /// Paces canvas frames at the program frame rate.
    clock: CanvasClock,
    /// Wall-clock origin for the canvas clock.
    epoch: Instant,
    /// Animation time of the newest canvas frame.
    time: f64,
    /// What the newest canvas frame was drawn with; refreshes with no new canvas frame
    /// composite it again.
    frame_params: FrameParams,
    /// When the previous screen refresh started, for the display rate readout.
    last_refresh: Instant,
    /// Where the autosave and settings live.
    data_dir: PathBuf,
    /// The lock that makes this the one window that autosaves and writes the settings.
    /// Held until exit; none when another window holds it.
    session_lock: Option<File>,
    /// No autosave this run: another window owns it, or an unreadable one is in the way.
    autosave_off: bool,
    /// The source and background image files, as a project saves them. A project's
    /// missing image keeps its path here, so saving doesn't drop it.
    source_path: Option<PathBuf>,
    background_path: Option<PathBuf>,
    /// The named project file the session belongs to; none while Untitled.
    project_file: Option<PathBuf>,
    /// The session as last saved or opened; none when it matches no file.
    saved: Option<Project>,
    /// What the autosave file holds.
    autosaved: Option<Autosave>,
    /// The last autosave failed (reported once until one succeeds).
    autosave_failed: bool,
    last_autosave: Instant,
    /// The settings as last written.
    settings_written: Settings,
}

impl State {
    fn new(event_loop: &ActiveEventLoop, initial_image: Option<&Path>) -> Result<Self> {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Rasterwarp")
                        .with_inner_size(LogicalSize::new(1280.0, 720.0)),
                )
                .context("failed to create window")?,
        );
        let backends = gpu::backends_from_env()?;
        let instance = gpu::create_instance(backends);
        let surface = instance
            .create_surface(window.clone())
            .with_context(|| {
                format!(
                    "failed to create window surface for backend {backends:?}; try RASTERWARP_BACKEND=dx12 or gl"
                )
            })?;
        let (adapter, device, queue) =
            pollster::block_on(gpu::request_device(&instance, backends, Some(&surface)))?;
        let max_texture_side = device.limits().max_texture_dimension_2d;

        let caps = surface.get_capabilities(&adapter);
        // The swapchain itself is non-sRGB (what egui expects). Where the adapter allows an
        // sRGB view of it, the composite pass draws through that view; otherwise (e.g. GL)
        // it draws into the plain format and encodes sRGB in the shader.
        let format = caps.formats[0].remove_srgb_suffix();
        let srgb_format = format.add_srgb_suffix();
        let srgb_view_ok = adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::SURFACE_VIEW_FORMATS);
        let composite_format = if srgb_view_ok { srgb_format } else { format };
        // Fifo (vsync) presents every refresh. RASTERWARP_UNCAPPED=1 opts into Mailbox
        // (no vsync wait, no tearing); canvas frames are paced by the frame rate either way.
        let uncapped = std::env::var("RASTERWARP_UNCAPPED").is_ok_and(|v| v == "1");
        let present_mode = if uncapped && caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .context("window surface is not supported by the GPU adapter")?;
        config.format = format;
        config.view_formats = if srgb_view_ok {
            vec![srgb_format]
        } else {
            vec![]
        };
        config.present_mode = present_mode;
        if size.width > 0 && size.height > 0 {
            surface.configure(&device, &config);
        }
        log::info!("surface: {format:?} (composite via {composite_format:?}), {present_mode:?}");

        let data_dir = save::data_dir();
        let session_lock = session::lock_session(&data_dir);
        let settings = save::load_settings(&data_dir);
        let mut ui = UiState {
            frame_ms: 16.7,
            show_preview: settings.show_preview,
            canvas: CanvasChoice::new(canvas::DEFAULT, max_texture_side),
            presets_folder: settings.presets_folder.clone(),
            ..Default::default()
        };
        ui.capture.settings = settings.capture.clone();
        let image = test_card(&mut ui);
        let renderer = Renderer::new(&device, &queue, composite_format, canvas::DEFAULT, &image);

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui_ctx.viewport_id(),
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(max_texture_side as usize),
        );
        let mut egui_renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        let preview =
            PreviewView::new(&device, &queue, &mut egui_renderer, canvas::DEFAULT, &image);
        let motion = Motion::new(Params::default());
        let frame_params = motion.frame();
        let epoch = Instant::now();

        let mut state = Self {
            window,
            surface,
            config,
            device,
            queue,
            max_texture_side,
            composite_format,
            renderer,
            preview,
            egui_ctx,
            egui_state,
            egui_renderer,
            motion,
            recorder: None,
            clock: CanvasClock::new(ui.rate, 0.0),
            ui,
            epoch,
            time: 0.0,
            frame_params,
            last_refresh: epoch,
            data_dir,
            session_lock,
            autosave_off: false,
            source_path: None,
            background_path: None,
            project_file: None,
            saved: None,
            autosaved: None,
            autosave_failed: false,
            last_autosave: epoch,
            settings_written: settings,
        };
        state.start_session(initial_image);
        Ok(state)
    }

    /// Picks up the last session from the autosave, if there is one, then loads the image
    /// named on the command line over it. Another window owns the autosave if it holds the
    /// lock; then this one starts fresh and leaves it alone.
    fn start_session(&mut self, initial_image: Option<&Path>) {
        let restored = if self.session_lock.is_some() {
            session::restore_autosave(&self.data_dir)
        } else {
            self.autosave_off = true;
            self.ui.session_note =
                Some("Another rasterwarp window is open; this one won't autosave.".into());
            Ok(None)
        };
        match restored {
            Ok(Some(loaded)) => {
                let autosave = loaded.value;
                self.apply_project(&autosave.project);
                self.project_file = autosave.file.clone();
                let current = self.project();
                self.saved = (!autosave.unsaved).then_some(current);
                if loaded.newer {
                    self.ui.file_note = Some(save::NEWER_NOTE.into());
                }
                self.autosaved = Some(autosave);
            }
            Ok(None) => self.saved = Some(self.project()),
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.file_error = Some(if err.is::<session::LeftInPlace>() {
                    // The next autosave would overwrite it.
                    self.autosave_off = true;
                    format!(
                        "Couldn't restore the last session; the file was left in place, so \
                         this run won't autosave: {err:#}"
                    )
                } else {
                    format!(
                        "Couldn't restore the last session (kept as {}): {err:#}",
                        session::BAD_AUTOSAVE_FILE
                    )
                });
                self.saved = Some(self.project());
            }
        }
        if let Some(path) = initial_image {
            self.load_source(path);
        }
        self.list_presets();
    }

    /// Makes `project` the running session, at rest: its motion, canvas size, frame rate
    /// and images. A missing image is reported and the test card (or no background) is
    /// shown instead, but its path stays in the session.
    fn apply_project(&mut self, project: &Project) {
        self.motion = project.motion();
        if self.recorder.is_none() {
            let size = canvas::sanitize(project.canvas, self.max_texture_side);
            if size != self.renderer.size() {
                self.renderer.resize(&self.device, size);
                self.preview
                    .resize(&self.device, &mut self.egui_renderer, size);
            }
            self.ui.canvas = CanvasChoice::new(size, self.max_texture_side);
            self.ui.rate = project.frame_rate;
        }
        let mut problems = Vec::new();
        match &project.source {
            Some(path) => {
                if let Err(err) = self.open_source(path) {
                    problems.push(image_problem("Source", path, &err));
                    self.show_test_card();
                }
            }
            None => self.show_test_card(),
        }
        match &project.background {
            Some(path) => {
                if let Err(err) = self.open_background(path) {
                    problems.push(image_problem("Background", path, &err));
                    self.set_background(None);
                }
            }
            None => self.set_background(None),
        }
        self.source_path = project.source.clone();
        self.background_path = project.background.clone();
        self.ui.load_error = (!problems.is_empty()).then(|| problems.join("\n"));
        self.clear_feedback();
        // Loading images stalls; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
    }

    /// A snapshot of the session as a project.
    fn project(&self) -> Project {
        Project::capture(
            &self.motion,
            self.ui.canvas.current,
            self.ui.rate,
            self.source_path.as_deref(),
            self.background_path.as_deref(),
        )
    }

    /// Whether `current` differs from the project file (or, Untitled, from how the
    /// session started).
    fn unsaved(&self, current: &Project) -> bool {
        self.saved.as_ref() != Some(current)
    }

    fn settings(&self) -> Settings {
        Settings {
            presets_folder: self.ui.presets_folder.clone(),
            capture: self.ui.capture.settings.clone(),
            show_preview: self.ui.show_preview,
        }
    }

    /// Writes the session to the autosave file, and the settings to theirs, if they
    /// changed since they were last written.
    fn autosave(&mut self) {
        self.last_autosave = Instant::now();
        let project = self.project();
        let autosave = Autosave {
            file: self.project_file.clone(),
            unsaved: self.unsaved(&project),
            project,
        };
        if !self.autosave_off && self.autosaved.as_ref() != Some(&autosave) {
            match session::save_autosave(&self.data_dir, &autosave) {
                Ok(()) => {
                    self.autosaved = Some(autosave);
                    if self.autosave_failed {
                        self.autosave_failed = false;
                        if let Some(shown) = &self.ui.file_error
                            && shown.starts_with(AUTOSAVE_FAILED)
                        {
                            self.ui.file_error = None;
                        }
                    }
                }
                Err(err) => {
                    log::warn!("{err:#}");
                    if !self.autosave_failed {
                        self.ui.file_error = Some(format!("{AUTOSAVE_FAILED}{err:#}"));
                    }
                    self.autosave_failed = true;
                }
            }
        }
        // Another window owns the settings file too.
        let settings = self.settings();
        if self.session_lock.is_some() && settings != self.settings_written {
            match save::save_settings(&self.data_dir, &settings) {
                Ok(()) => self.settings_written = settings,
                Err(err) => log::warn!("{err:#}"),
            }
        }
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = size.width;
        self.config.height = size.height;
        if size.width > 0 && size.height > 0 {
            self.surface.configure(&self.device, &self.config);
        }
    }

    /// Opens a dropped preset or project, or loads a dropped image as the source.
    fn dropped(&mut self, path: &Path) {
        let is = |ext: &str| {
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(ext))
        };
        if is(PRESET_EXTENSION) {
            self.load_preset(path);
        } else if is(PROJECT_EXTENSION) {
            self.open_project(Some(path));
        } else {
            self.load_source(path);
        }
    }

    /// The project file's name, or "Untitled".
    fn project_name(&self) -> String {
        self.project_file
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned())
    }

    fn file_actions(&mut self, actions: FileActions) {
        if actions.save_project {
            self.save_project(false);
        }
        if actions.save_project_as {
            self.save_project(true);
        }
        if actions.open_project {
            self.open_project(None);
        }
        if actions.new_project {
            self.new_project();
        }
        if let Some(name) = actions.save_preset {
            self.save_preset(&name);
        }
        if let Some(name) = actions.load_preset {
            self.load_preset(&presets::path(&self.ui.presets_folder, &name));
        }
        if let Some((from, to)) = actions.rename_preset {
            let renamed = presets::rename(&self.ui.presets_folder, &from, &to);
            self.report(renamed);
            self.list_presets();
        }
        if let Some(name) = actions.delete_preset
            && self.confirm(&format!("Delete preset {name}?"))
        {
            let deleted = presets::delete(&self.ui.presets_folder, &name);
            self.report(deleted);
            self.list_presets();
        }
        if actions.choose_presets_folder {
            self.choose_presets_folder();
        }
        if actions.list_presets {
            self.list_presets();
        }
    }

    /// Shows a file operation's error, or clears the last one.
    fn report(&mut self, result: Result<()>) {
        self.ui.file_note = None;
        self.ui.file_error = result.err().map(|err| {
            log::warn!("{err:#}");
            format!("{err:#}")
        });
    }

    /// Asks a yes/no question in a system dialog.
    fn confirm(&mut self, question: &str) -> bool {
        let answer = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Rasterwarp")
            .set_description(question)
            .set_buttons(rfd::MessageButtons::YesNo)
            .set_parent(&*self.window)
            .show();
        // The dialog blocks the app; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
        answer == rfd::MessageDialogResult::Yes
    }

    /// Before the session is replaced: if it has unsaved changes, asks whether to save
    /// them (Yes saves, No discards). False means don't go on: the user cancelled, or
    /// saving was cancelled or failed.
    fn may_replace_session(&mut self) -> bool {
        if !self.unsaved(&self.project()) {
            return true;
        }
        let answer = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Unsaved changes")
            .set_description(format!("Save changes to {}?", self.project_name()))
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .set_parent(&*self.window)
            .show();
        self.clock.reanchor(self.seconds());
        match answer {
            rfd::MessageDialogResult::Yes => self.save_project(false),
            rfd::MessageDialogResult::No => true,
            _ => false,
        }
    }

    /// Saves the session to its project file, asking for one when Untitled or when
    /// `ask` (Save as). Returns whether it saved.
    fn save_project(&mut self, ask: bool) -> bool {
        let path = match &self.project_file {
            Some(path) if !ask => path.clone(),
            _ => match self.ask_project_path() {
                Some(path) => path,
                None => return false,
            },
        };
        let project = self.project();
        let saved = session::save_project(&path, &project);
        let ok = saved.is_ok();
        self.report(saved);
        if ok {
            self.project_file = Some(path);
            self.saved = Some(project);
        }
        ok
    }

    fn ask_project_path(&mut self) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save project")
            .add_filter("Rasterwarp project", &[PROJECT_EXTENSION])
            .set_file_name(format!("{}.{PROJECT_EXTENSION}", self.project_name()));
        if let Some(dir) = self.project_file.as_deref().and_then(Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        let picked = dialog.save_file();
        self.clock.reanchor(self.seconds());
        let path = picked?;
        // "show" becomes "show.rwproject"; "show.v2" becomes "show.v2.rwproject".
        let has_extension = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(PROJECT_EXTENSION));
        Some(if has_extension {
            path
        } else {
            let mut named = path.into_os_string();
            named.push(format!(".{PROJECT_EXTENSION}"));
            PathBuf::from(named)
        })
    }

    /// Opens a project (asking which when `path` is none), after the unsaved-changes
    /// check. Not while recording: the canvas can't change then.
    fn open_project(&mut self, path: Option<&Path>) {
        if self.recorder.is_some() || !self.may_replace_session() {
            return;
        }
        let path = match path {
            Some(path) => path.to_path_buf(),
            None => {
                let picked = rfd::FileDialog::new()
                    .set_title("Open project")
                    .add_filter("Rasterwarp project", &[PROJECT_EXTENSION])
                    .pick_file();
                self.clock.reanchor(self.seconds());
                match picked {
                    Some(path) => path,
                    None => return,
                }
            }
        };
        match session::load_project(&path) {
            Ok(loaded) => {
                self.apply_project(&loaded.value);
                self.project_file = Some(absolute(&path));
                self.saved = Some(self.project());
                self.ui.file_error = None;
                self.ui.file_note = loaded.newer.then(|| save::NEWER_NOTE.into());
            }
            Err(err) => self.report(Err(err)),
        }
    }

    /// Starts over with the default session, Untitled.
    fn new_project(&mut self) {
        if self.recorder.is_some() || !self.may_replace_session() {
            return;
        }
        self.apply_project(&Project::default());
        self.project_file = None;
        self.saved = Some(self.project());
        self.report(Ok(()));
    }

    /// Saves the edited look as preset `name`, asking before replacing one.
    fn save_preset(&mut self, name: &str) {
        let folder = self.ui.presets_folder.clone();
        if presets::exists(&folder, name) && !self.confirm(&format!("Replace preset {name}?")) {
            return;
        }
        let saved = presets::save(&folder, name, self.motion.editable());
        self.report(saved);
        self.list_presets();
    }

    /// Loads a preset into the edited look: on screen in Live mode, the off-air bank in
    /// Transition mode, the selected cue in Sequence mode.
    fn load_preset(&mut self, path: &Path) {
        match save::load_preset(path) {
            Ok(loaded) => {
                *self.motion.editable() = loaded.value;
                if self.motion.mode() == Mode::Live {
                    // The picture cuts to the preset.
                    self.renderer.reset_motion();
                }
                self.ui.file_error = None;
                self.ui.file_note = loaded.newer.then(|| save::NEWER_NOTE.into());
            }
            Err(err) => self.report(Err(err)),
        }
    }

    fn choose_presets_folder(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Presets folder");
        if let Ok(start) = std::path::absolute(&self.ui.presets_folder)
            && start.is_dir()
        {
            dialog = dialog.set_directory(start);
        }
        if let Some(picked) = dialog.pick_folder() {
            self.ui.presets_folder = picked;
            self.list_presets();
        }
        // The dialog blocks the app; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
    }

    fn list_presets(&mut self) {
        self.ui.files.presets = presets::list(&self.ui.presets_folder);
    }

    /// Loads a dropped (or command-line) image as the source.
    fn load_source(&mut self, path: &Path) {
        match self.open_source(path) {
            Ok(()) => {
                self.source_path = Some(absolute(path));
                self.ui.load_error = None;
            }
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.load_error = Some(format!("{err:#}"));
            }
        }
        // Decoding the image stalls; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
    }

    /// Shows the image at `path` as the source.
    fn open_source(&mut self, path: &Path) -> Result<()> {
        let image = load_fitting(path, self.max_texture_side)?;
        self.renderer.set_source(&self.device, &self.queue, &image);
        self.preview.set_source(&self.device, &self.queue, &image);
        self.ui.source_info = describe(&path.display().to_string(), &image);
        Ok(())
    }

    fn show_test_card(&mut self) {
        let image = test_card(&mut self.ui);
        self.renderer.set_source(&self.device, &self.queue, &image);
        self.preview.set_source(&self.device, &self.queue, &image);
    }

    /// Asks for a background image for keyed levels and shows it.
    fn choose_background(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Background image")
            .add_filter("Images", &["png", "jpg", "jpeg"])
            .pick_file();
        if let Some(path) = picked {
            match self.open_background(&path) {
                Ok(()) => {
                    self.background_path = Some(absolute(&path));
                    self.ui.load_error = None;
                }
                Err(err) => {
                    log::warn!("{err:#}");
                    self.ui.load_error = Some(format!("{err:#}"));
                }
            }
        }
        // The dialog and decoding block the app; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
    }

    /// Shows the image at `path` behind see-through levels.
    fn open_background(&mut self, path: &Path) -> Result<()> {
        let image = load_background(path, self.max_texture_side)?;
        self.set_background(Some(&image));
        self.ui.background_info = Some(format!(
            "Background: {} ({}×{})",
            path.display(),
            image.width,
            image.height
        ));
        Ok(())
    }

    fn set_background(&mut self, image: Option<&ColorImage>) {
        self.renderer
            .set_background(&self.device, &self.queue, image);
        self.preview
            .set_background(&self.device, &self.queue, image);
        if image.is_none() {
            self.ui.background_info = None;
        }
    }

    fn clear_feedback(&mut self) {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.renderer.clear_feedback(&mut encoder);
        self.preview.clear_feedback(&mut encoder);
        self.queue.submit([encoder.finish()]);
    }

    /// Wall-clock seconds since the app started, for the canvas clock.
    fn seconds(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    fn start_recording(&mut self) {
        self.ui.capture.saved = None;
        let started = Recorder::start(
            &self.device,
            &self.ui.capture.settings,
            self.renderer.size(),
            self.clock.rate(),
        );
        // Opening the encoder stalls, even when it fails; that must not make canvas
        // frames late.
        self.clock.reanchor(self.seconds());
        match started {
            Ok(recorder) => {
                self.ui.capture.error = None;
                self.ui.capture.status = Some(recorder.status());
                self.recorder = Some(recorder);
            }
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.capture.error = Some(format!("{err:#}"));
            }
        }
    }

    /// Finishes the current recording, if any, and reports how it went.
    fn stop_recording(&mut self) {
        let Some(recorder) = self.recorder.take() else {
            return;
        };
        self.ui.capture.status = None;
        let result = recorder.finish(&self.device);
        // Flushing the encoder stalls; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
        match result {
            Ok(status) => {
                self.ui.capture.saved = Some(format!(
                    "Saved {} ({:.1} s, {} frames at {}, {} dropped)",
                    status.path.display(),
                    status.seconds,
                    status.frames,
                    self.clock.rate().label(),
                    status.dropped
                ));
            }
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.capture.error = Some(format!("Recording stopped: {err:#}"));
            }
        }
    }

    fn choose_capture_folder(&mut self) {
        let folder = &mut self.ui.capture.settings.folder;
        let mut dialog = rfd::FileDialog::new().set_title("Capture folder");
        if let Ok(start) = std::path::absolute(&*folder)
            && start.is_dir()
        {
            dialog = dialog.set_directory(start);
        }
        if let Some(picked) = dialog.pick_folder() {
            *folder = picked;
        }
        // The dialog blocks the app; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
    }

    /// Draws one canvas frame: advances animation by one frame period (unless paused),
    /// draws the canvas and the preview, and records the frame if a recording is running.
    fn draw_canvas_frame(&mut self) {
        let paused = self.ui.paused;
        if !paused {
            let rate = self.clock.rate();
            self.time += rate.period();
            self.motion.advance(rate.period_ticks());
        }
        self.frame_params = self.motion.frame();
        if self.motion.take_jump() {
            self.renderer.reset_motion();
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.renderer.render_canvas(
            &self.device,
            &self.queue,
            &mut encoder,
            &self.frame_params,
            self.time as f32,
        );
        match self
            .ui
            .show_preview
            .then(|| self.motion.preview())
            .flatten()
        {
            Some(preview) => self.preview.render(
                &self.device,
                &self.queue,
                &mut encoder,
                &preview,
                self.time as f32,
            ),
            None => self.preview.hide(),
        }
        let mut capture_failed = false;
        // A paused frame repeats the last one, so it isn't recorded.
        if let Some(recorder) = &mut self.recorder
            && !paused
        {
            let (device, queue, renderer) = (&self.device, &self.queue, &self.renderer);
            let (frame_params, time) = (&self.frame_params, self.time as f32);
            let captured = recorder.capture(device, &mut encoder, |encoder, view| {
                renderer.composite_capture(device, queue, encoder, frame_params, time, view)
            });
            if let Err(err) = captured {
                log::warn!("{err:#}");
                capture_failed = true;
            }
        }
        self.queue.submit([encoder.finish()]);
        if let Some(recorder) = &mut self.recorder {
            recorder.after_submit();
            self.ui.capture.status = Some(recorder.status());
            if capture_failed || recorder.finished_recording() {
                self.stop_recording();
            }
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_refresh).as_secs_f32();
        self.last_refresh = now;
        self.ui.frame_ms += (dt * 1000.0 - self.ui.frame_ms) * 0.05;

        // While the window is minimized or hidden no canvas frames are drawn; that time
        // must not count as late frames when it reappears.
        if self.config.width == 0 || self.config.height == 0 {
            self.clock.reanchor(self.seconds());
            return; // minimized
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.clock.reanchor(self.seconds());
                return;
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("validation error acquiring the surface texture");
                return;
            }
        };
        if let Some(recorder) = &mut self.recorder
            && let Err(err) = recorder.pump(&self.device)
        {
            log::warn!("{err:#}");
            self.stop_recording();
        }
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let srgb_view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.composite_format),
            ..Default::default()
        });
        let output_size = (self.config.width, self.config.height);

        let overlay = self
            .ui
            .show_preview
            .then(|| self.motion.preview())
            .flatten()
            .map(|p| ui::PreviewOverlay {
                texture: self.preview.texture_id(),
                size: self.preview.size(),
                label: p.source.label(),
            });
        self.ui.late = self.clock.late();
        self.ui.files.project_name = self.project_name();
        self.ui.files.unsaved = self.unsaved(&self.project());
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let mut actions = UiActions::default();
        let egui_output = self.egui_ctx.run_ui(raw_input, |ui| {
            actions = ui::draw(ui, &mut self.motion, &mut self.ui, overlay.as_ref())
        });
        self.egui_state
            .handle_platform_output(&self.window, egui_output.platform_output);
        let jobs = self
            .egui_ctx
            .tessellate(egui_output.shapes, egui_output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [output_size.0, output_size.1],
            pixels_per_point: egui_output.pixels_per_point,
        };
        for (id, deltas) in &egui_output.textures_delta.set {
            for delta in deltas {
                self.egui_renderer
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
        }

        if actions.choose_folder {
            self.choose_capture_folder();
        }
        if actions.choose_background {
            self.choose_background();
        }
        if actions.clear_background {
            self.set_background(None);
            self.background_path = None;
        }
        if actions.stop_recording {
            self.stop_recording();
        }
        if let Some(size) = actions.apply_canvas
            && self.recorder.is_none()
        {
            self.renderer.resize(&self.device, size);
            self.preview
                .resize(&self.device, &mut self.egui_renderer, size);
            self.ui.canvas.current = size;
        }
        if self.ui.rate != self.clock.rate() && self.recorder.is_none() {
            self.clock.set_rate(self.ui.rate, self.seconds());
        }
        if actions.start_recording && self.recorder.is_none() {
            self.start_recording();
        }
        if actions.clear_feedback {
            self.clear_feedback();
        }
        self.file_actions(actions.files);
        // Not while recording; closing the window still autosaves.
        if self.last_autosave.elapsed() >= AUTOSAVE_INTERVAL && self.recorder.is_none() {
            self.autosave();
        }

        let offline = self
            .recorder
            .as_ref()
            .is_some_and(|r| r.mode() == CaptureMode::Offline);
        let pacing = if offline {
            Pacing::Offline
        } else {
            Pacing::RealTime
        };
        for _ in 0..self.clock.due(self.seconds(), pacing) {
            self.draw_canvas_frame();
        }

        let mut encoder = self.device.create_command_encoder(&Default::default());
        self.renderer.composite(
            &self.device,
            &self.queue,
            &mut encoder,
            &self.frame_params,
            self.time as f32,
            &srgb_view,
            output_size,
        );
        let egui_commands = self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &jobs,
            &screen,
        );
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.egui_renderer.render(&mut pass, &jobs, &screen);
        }
        self.queue
            .submit(egui_commands.into_iter().chain([encoder.finish()]));
        self.window.pre_present_notify();
        self.queue.present(frame);
        for id in &egui_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
    }
}

fn test_card(ui: &mut UiState) -> GrayImage {
    let image = source::test_card(1600, 900);
    ui.source_info = describe("built-in test card", &image);
    image
}

/// What to say about a project's image that didn't load.
fn image_problem(kind: &str, path: &Path, err: &anyhow::Error) -> String {
    if path.exists() {
        format!("{err:#}")
    } else {
        format!("{kind} image not found: {}", path.display())
    }
}

/// `path` made absolute, so a saved project finds it from any working directory.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

fn describe(name: &str, image: &GrayImage) -> String {
    format!("Source: {name} ({}×{})", image.width, image.height)
}

/// Loads a background image and checks it fits in a GPU texture.
fn load_background(path: &Path, max_side: u32) -> anyhow::Result<ColorImage> {
    let image = source::load_color(path)?;
    source::ensure_size_fits(image.width, image.height, max_side)
        .with_context(|| format!("could not load {}", path.display()))?;
    Ok(image)
}

/// Loads an image and checks it fits in a GPU texture.
fn load_fitting(path: &Path, max_side: u32) -> anyhow::Result<GrayImage> {
    let image = source::load_image(path)?;
    source::ensure_fits(&image, max_side)
        .with_context(|| format!("could not load {}", path.display()))?;
    Ok(image)
}
