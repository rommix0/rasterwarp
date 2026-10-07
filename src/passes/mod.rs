//! The render pipeline: source -> warp -> colorize -> feedback -> bloom -> composite.

pub mod bloom;
pub mod colorize;
pub mod composite;
pub mod feedback;
pub mod warp;

use crate::params::Params;
use crate::source::{self, GrayImage};

pub struct Renderer {
    size: (u32, u32),
    source: wgpu::TextureView,
    source_aspect: f32,
    warp: warp::WarpPass,
    colorize: colorize::ColorizePass,
    feedback: feedback::FeedbackPass,
    bloom: bloom::BloomPass,
    composite: composite::CompositePass,
}

impl Renderer {
    /// `size` is the fixed internal resolution; `output_format` is the format of the
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
        }
    }

    pub fn set_source(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, image: &GrayImage) {
        self.source = source::upload(device, queue, image).create_view(&Default::default());
        self.source_aspect = image.aspect();
    }

    pub fn clear_feedback(&self, encoder: &mut wgpu::CommandEncoder) {
        self.feedback.clear(encoder);
    }

    /// Records one frame into `encoder`, ending with a draw into `output`
    /// (`output_size` pixels, letterboxed).
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        params: &Params,
        time: f32,
        output: &wgpu::TextureView,
        output_size: (u32, u32),
    ) {
        let aspect = self.size.0 as f32 / self.size.1 as f32;
        self.warp.render(
            device,
            queue,
            encoder,
            &self.source,
            &warp::uniforms(&params.warp, time, aspect, self.source_aspect),
        );
        self.colorize.render(
            device,
            queue,
            encoder,
            &self.warp.target.view,
            &colorize::uniforms(&params.colorize, time),
        );
        self.feedback.render(
            device,
            queue,
            encoder,
            &self.colorize.target.view,
            &feedback::uniforms(&params.feedback, aspect),
        );
        self.bloom.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            params.glow.bloom_threshold,
        );
        self.composite.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            self.bloom.output(),
            output,
            &composite::uniforms(
                &params.glow,
                time,
                self.size,
                output_size,
                self.composite.encode_srgb(),
            ),
        );
    }
}
