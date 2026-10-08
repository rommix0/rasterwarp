//! The render pipeline: source -> warp -> colorize -> feedback -> bloom -> composite.

pub mod bloom;
pub mod colorize;
pub mod composite;
pub mod feedback;
pub mod warp;

use crate::blend::FrameParams;
use crate::source::{self, GrayImage};

/// The format of the capture texture that recordings are read from.
pub const CAPTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

pub struct Renderer {
    size: (u32, u32),
    source: wgpu::TextureView,
    source_aspect: f32,
    warp: warp::WarpPass,
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
            source: source::upload(device, queue, image).create_view(&Default::default()),
            source_aspect: image.aspect(),
            warp: warp::WarpPass::new(device, w, h),
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

    /// Rebuilds every canvas-sized target at `size`. The trails start out cleared.
    pub fn resize(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        let (w, h) = size;
        self.size = size;
        self.warp = warp::WarpPass::new(device, w, h);
        self.colorize = colorize::ColorizePass::new(device, w, h);
        self.feedback = feedback::FeedbackPass::new(device, w, h);
        self.bloom = bloom::BloomPass::new(device, w, h);
    }

    pub fn set_source(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, image: &GrayImage) {
        self.source = source::upload(device, queue, image).create_view(&Default::default());
        self.source_aspect = image.aspect();
    }

    pub fn clear_feedback(&self, encoder: &mut wgpu::CommandEncoder) {
        self.feedback.clear(encoder);
    }

    /// Records one frame into `encoder`: draws a canvas frame, then composites it into
    /// `output` (`output_size` pixels, letterboxed).
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
        self.composite(device, queue, encoder, frame, time, output, output_size);
    }

    /// Draws the next canvas frame (warp, colorize, feedback, bloom) without showing it.
    /// Each call advances the feedback trails by one frame.
    pub fn render_canvas(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameParams,
        time: f32,
    ) {
        let aspect = self.size.0 as f32 / self.size.1 as f32;
        self.warp.render(
            device,
            queue,
            encoder,
            &self.source,
            &warp::uniforms(&frame.warp, time, aspect, self.source_aspect),
        );
        self.colorize.render(
            device,
            queue,
            encoder,
            &self.warp.target.view,
            &colorize::uniforms(&frame.colorize),
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

    /// Composites the current canvas frame into `output` (`output_size` pixels,
    /// letterboxed). Doesn't touch the canvas, so it can run any number of times per
    /// canvas frame.
    #[allow(clippy::too_many_arguments)]
    pub fn composite(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameParams,
        time: f32,
        output: &wgpu::TextureView,
        output_size: (u32, u32),
    ) {
        self.composite.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            self.bloom.output(),
            output,
            &composite::uniforms(
                &frame.glow,
                time,
                self.size,
                output_size,
                self.composite.encode_srgb(),
            ),
        );
    }

    /// Draws the current canvas frame's composite into `output`: a canvas-sized texture
    /// in [`CAPTURE_FORMAT`], with no letterboxing and no UI.
    pub fn composite_capture(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &FrameParams,
        time: f32,
        output: &wgpu::TextureView,
    ) {
        self.capture.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            self.bloom.output(),
            output,
            &composite::uniforms(
                &frame.glow,
                time,
                self.size,
                self.size,
                self.capture.encode_srgb(),
            ),
        );
    }
}
