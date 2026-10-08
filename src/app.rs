//! The windowed application: owns the GPU context, renderer, parameters and UI. Each
//! screen refresh draws the canvas frames that are due at the program frame rate, then
//! shows the newest one with the UI on top.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

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
use crate::gpu;
use crate::motion::Motion;
use crate::params::Params;
use crate::passes::Renderer;
use crate::preview::PreviewView;
use crate::source::{self, ColorImage, GrayImage};
use crate::ui::{self, UiActions, UiState};

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
                event_loop.exit();
            }
            WindowEvent::Resized(size) => state.resize(size),
            WindowEvent::DroppedFile(path) => state.load_source(&path),
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

        let mut ui = UiState {
            frame_ms: 16.7,
            show_preview: true,
            canvas: CanvasChoice::new(canvas::DEFAULT, max_texture_side),
            ..Default::default()
        };
        let image = match initial_image {
            Some(path) => match load_fitting(path, max_texture_side) {
                Ok(image) => {
                    ui.source_info = describe(&path.display().to_string(), &image);
                    image
                }
                Err(err) => {
                    log::warn!("{err:#}");
                    ui.load_error = Some(format!("{err:#}"));
                    test_card(&mut ui)
                }
            },
            None => test_card(&mut ui),
        };
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

        Ok(Self {
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
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = size.width;
        self.config.height = size.height;
        if size.width > 0 && size.height > 0 {
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn load_source(&mut self, path: &Path) {
        match load_fitting(path, self.max_texture_side) {
            Ok(image) => {
                self.renderer.set_source(&self.device, &self.queue, &image);
                self.preview.set_source(&self.device, &self.queue, &image);
                self.ui.source_info = describe(&path.display().to_string(), &image);
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

    /// Asks for a background image for keyed levels and shows it.
    fn choose_background(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Background image")
            .add_filter("Images", &["png", "jpg", "jpeg"])
            .pick_file();
        if let Some(path) = picked {
            match load_background(&path, self.max_texture_side) {
                Ok(image) => {
                    self.set_background(Some(&image));
                    self.ui.background_info = Some(format!(
                        "Background: {} ({}×{})",
                        path.display(),
                        image.width,
                        image.height
                    ));
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

    fn set_background(&mut self, image: Option<&ColorImage>) {
        self.renderer
            .set_background(&self.device, &self.queue, image);
        self.preview
            .set_background(&self.device, &self.queue, image);
        if image.is_none() {
            self.ui.background_info = None;
        }
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
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.renderer.clear_feedback(&mut encoder);
            self.preview.clear_feedback(&mut encoder);
            self.queue.submit([encoder.finish()]);
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
