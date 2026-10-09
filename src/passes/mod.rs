//! The render pipeline: source -> warp or raster -> colorize -> feedback -> bloom ->
//! composite.

pub mod bloom;
pub mod colorize;
pub mod composite;
pub mod feedback;
pub mod frames;
pub mod raster;
pub mod warp;

use crate::blend::FrameParams;
use crate::params::{GlowParams, Role};
use crate::passes::composite::Area;
use crate::passes::frames::FramesPass;
use crate::source::{self, ColorImage, GrayImage};
use crate::video::Pixels;
use crate::video::playhead::Sample;

/// The format of the capture texture that recordings are read from.
pub const CAPTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Where an input's picture comes from on the GPU.
enum Picture {
    Image(wgpu::TextureView),
    /// A video or camera, drawn each frame by its frames pass.
    Frames(Box<FramesPass>),
}

impl Picture {
    fn view(&self) -> &wgpu::TextureView {
        match self {
            Picture::Image(view) => view,
            Picture::Frames(pass) => &pass.target.view,
        }
    }
}

pub struct Renderer {
    size: (u32, u32),
    /// This renderer's pixels per program canvas pixel (see [`Renderer::set_pixel_scale`]).
    pixel_scale: f32,
    source: Picture,
    source_aspect: f32,
    /// What keyed levels show.
    background: Picture,
    background_aspect: f32,
    warp: warp::WarpPass,
    raster: raster::RasterPass,
    /// The deflection the previous canvas frame was drawn with, for the raster beam speed.
    previous: Option<warp::WarpUniforms>,
    colorize: colorize::ColorizePass,
    feedback: feedback::FeedbackPass,
    bloom: bloom::BloomPass,
    composite: composite::CompositePass,
    /// Draws the same composite into a canvas-sized capture texture.
    capture: composite::CompositePass,
}

impl Renderer {
    /// `size` is the internal (canvas) resolution; `output_format` is the format of the
    /// texture `render` draws into.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        output_format: wgpu::TextureFormat,
        size: (u32, u32),
        image: &GrayImage,
    ) -> Self {
        let (w, h) = size;
        Self {
            size,
            pixel_scale: 1.0,
            source: Picture::Image(
                source::upload(device, queue, image).create_view(&Default::default()),
            ),
            source_aspect: image.aspect(),
            background: Picture::Image(
                source::upload_color(device, queue, &ColorImage::black())
                    .create_view(&Default::default()),
            ),
            background_aspect: 1.0,
            warp: warp::WarpPass::new(device, w, h),
            raster: raster::RasterPass::new(device),
            previous: None,
            colorize: colorize::ColorizePass::new(device, w, h),
            feedback: feedback::FeedbackPass::new(device, w, h),
            bloom: bloom::BloomPass::new(device, w, h),
            composite: composite::CompositePass::new(device, output_format),
            capture: composite::CompositePass::new(device, CAPTURE_FORMAT),
        }
    }

    /// The internal (canvas) resolution.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Sets how many of this renderer's pixels make one program canvas pixel: 1.0 (the
    /// default) when it renders at the canvas size, less for a smaller off-air preview.
    /// Parameters measured in canvas pixels (beam width, line jitter, edge fringe) are
    /// scaled by it, so a smaller renderer shows the same picture.
    pub fn set_pixel_scale(&mut self, scale: f32) {
        self.pixel_scale = scale;
    }

    /// Forgets the previous frame's deflection, so the next raster frame has no beam
    /// speed. Call it when the picture jumps (a cut, a snapped cue): otherwise the jump
    /// reads as a very fast beam and flashes the speed boost for one frame.
    pub fn reset_motion(&mut self) {
        self.previous = None;
    }

    /// Rebuilds every canvas-sized target at `size`. The trails start out cleared.
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let (w, h) = size;
        self.size = size;
        self.reset_motion(); // the aspect ratio may have changed
        self.warp = warp::WarpPass::new(device, w, h);
        self.colorize = colorize::ColorizePass::new(device, w, h);
        self.feedback = feedback::FeedbackPass::new(device, w, h);
        self.bloom = bloom::BloomPass::new(device, w, h);
    }

    /// Shows a new source image. Its fit in the frame may differ, so this resets the beam
    /// motion (see [`Renderer::reset_motion`]).
    pub fn set_source(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, image: &GrayImage) {
        self.source =
            Picture::Image(source::upload(device, queue, image).create_view(&Default::default()));
        self.source_aspect = image.aspect();
        self.reset_motion();
    }

    /// Sets what keyed levels show: an image, or black.
    pub fn set_background(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: Option<&ColorImage>,
    ) {
        let black = ColorImage::black();
        let image = image.unwrap_or(&black);
        self.background = Picture::Image(
            source::upload_color(device, queue, image).create_view(&Default::default()),
        );
        self.background_aspect = image.aspect();
    }

    /// Makes `role`'s input a video or camera with frames of `size` pixels, drawn by a
    /// frames pass whose ring may grow to `max_layers` layers. A new source resets the
    /// beam motion, like a new image.
    pub fn set_frames(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        role: Role,
        size: (u32, u32),
        max_layers: u32,
    ) {
        let pass = FramesPass::new(device, queue, Pixels::for_role(role), size, max_layers);
        let picture = Picture::Frames(Box::new(pass));
        let aspect = size.0 as f32 / size.1 as f32;
        match role {
            Role::Source => {
                self.source = picture;
                self.source_aspect = aspect;
                self.reset_motion();
            }
            Role::Background => {
                self.background = picture;
                self.background_aspect = aspect;
            }
        }
    }

    /// `role`'s frames pass, when its input is a video or camera.
    pub fn frames_mut(&mut self, role: Role) -> Option<&mut FramesPass> {
        let picture = match role {
            Role::Source => &mut self.source,
            Role::Background => &mut self.background,
        };
        match picture {
            Picture::Frames(pass) => Some(pass),
            Picture::Image(_) => None,
        }
    }

    /// Draws `role`'s picture for this frame from its frames pass (whose window the
    /// caller has filled; see [`FramesPass::missing`]). Nothing for an image.
    pub fn draw_frames(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        role: Role,
        sample: &Sample,
    ) {
        if let Some(pass) = self.frames_mut(role) {
            pass.draw(device, queue, encoder, sample);
        }
    }

    pub fn clear_feedback(&self, encoder: &mut wgpu::CommandEncoder) {
        self.feedback.clear(encoder);
    }

    /// Records one frame into `encoder`: draws a canvas frame, then composites it into
    /// the whole of `output` (`output_size` pixels, letterboxed).
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameParams,
        time: f32,
        output: &wgpu::TextureView,
        output_size: (u32, u32),
    ) {
        self.render_canvas(device, queue, encoder, frame, time);
        let area = Area::whole(output_size);
        self.composite(device, queue, encoder, frame, time, output, area);
    }

    /// Draws the next canvas frame (warp or raster, colorize, feedback, bloom) without
    /// showing it. Each call advances the feedback trails by one frame.
    pub fn render_canvas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameParams,
        time: f32,
    ) {
        let aspect = self.size.0 as f32 / self.size.1 as f32;
        let deflection = warp::uniforms(
            &frame.warp,
            time,
            aspect,
            self.source_aspect,
            self.size.1,
            self.pixel_scale,
        );
        if frame.raster.enabled {
            let before = self.previous.unwrap_or(deflection);
            self.raster.render(
                device,
                queue,
                encoder,
                self.source.view(),
                &self.warp.target.view,
                &raster::uniforms(
                    &frame.raster,
                    &deflection,
                    &before,
                    self.size,
                    self.pixel_scale,
                ),
            );
        } else {
            self.warp
                .render(device, queue, encoder, self.source.view(), &deflection);
        }
        self.previous = Some(deflection);
        self.colorize.render(
            device,
            queue,
            encoder,
            &self.warp.target.view,
            &colorize::uniforms(&frame.colorize, &frame.key, self.pixel_scale),
        );
        self.feedback.render(
            device,
            queue,
            encoder,
            &self.colorize.target.view,
            &feedback::uniforms(&frame.feedback, aspect),
        );
        self.bloom.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            frame.glow.bloom_threshold,
        );
    }

    /// Composites the current canvas frame into `area` of `output`, letterboxed, and
    /// clears the rest of `output` to black. Doesn't touch the canvas, so it can run any
    /// number of times per canvas frame.
    #[allow(clippy::too_many_arguments)]
    pub fn composite(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameParams,
        time: f32,
        output: &wgpu::TextureView,
        area: Area,
    ) {
        self.composite.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            self.bloom.output(),
            self.background.view(),
            output,
            area,
            &composite::uniforms(
                &crt_glow(frame),
                time,
                self.size,
                area.size(),
                self.composite.encode_srgb(),
                self.background_aspect,
                false,
            ),
        );
    }

    /// Draws the current canvas frame's composite into `output`: a canvas-sized texture
    /// in [`CAPTURE_FORMAT`], with no letterboxing and no UI. With `alpha`, the background
    /// is left out and keyed levels are see-through, in straight (not premultiplied) alpha.
    #[allow(clippy::too_many_arguments)]
    pub fn composite_capture(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameParams,
        time: f32,
        alpha: bool,
        output: &wgpu::TextureView,
    ) {
        self.capture.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            self.bloom.output(),
            self.background.view(),
            output,
            Area::whole(self.size),
            &composite::uniforms(
                &crt_glow(frame),
                time,
                self.size,
                self.size,
                self.capture.encode_srgb(),
                self.background_aspect,
                alpha,
            ),
        );
    }
}

/// The CRT effects the composite applies. Raster mode draws real scan lines, so the
/// painted-on scanline overlay is off.
fn crt_glow(frame: &FrameParams) -> GlowParams {
    let mut glow = frame.glow;
    if frame.raster.enabled {
        glow.scanline_strength = 0.0;
    }
    glow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::Params;

    #[test]
    fn raster_mode_turns_off_the_painted_scanlines() {
        let mut frame = FrameParams::at_rest(&Params::default());
        assert_eq!(crt_glow(&frame), frame.glow);
        frame.raster.enabled = true;
        let glow = crt_glow(&frame);
        assert_eq!(glow.scanline_strength, 0.0);
        assert_eq!(glow.bloom_intensity, frame.glow.bloom_intensity);
    }
}
