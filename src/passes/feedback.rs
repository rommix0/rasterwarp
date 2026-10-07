//! Feedback pass: phosphor-style trails using two textures that swap roles each frame.

use bytemuck::{Pod, Zeroable};

use crate::gpu::{self, FullscreenPass, INTERNAL_FORMAT, PassDesc, RenderTarget};
use crate::params::FeedbackParams;

/// Matches `struct Feedback` in feedback.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct FeedbackUniforms {
    pub settings: [f32; 4],
    pub offset: [f32; 4],
}

pub fn uniforms(p: &FeedbackParams, frame_aspect: f32) -> FeedbackUniforms {
    FeedbackUniforms {
        settings: [p.amount, p.zoom, p.rotation, frame_aspect],
        offset: [p.offset[0], p.offset[1], 0.0, 0.0],
    }
}

pub struct FeedbackPass {
    pass: FullscreenPass,
    uniform: wgpu::Buffer,
    targets: [RenderTarget; 2],
    /// Index of the target holding the latest frame.
    current: usize,
}

impl FeedbackPass {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let pass = FullscreenPass::new(
            device,
            &PassDesc {
                label: "feedback",
                shader: include_str!("../../shaders/feedback.wgsl"),
                fragment_entry: "fs_main",
                uniform_size: size_of::<FeedbackUniforms>() as u64,
                textures: 2,
                format: INTERNAL_FORMAT,
                blend: None,
            },
        );
        let uniform = pass.create_uniform_buffer(device);
        let targets = [
            RenderTarget::new(device, "feedback a", width, height, INTERNAL_FORMAT),
            RenderTarget::new(device, "feedback b", width, height, INTERNAL_FORMAT),
        ];
        Self {
            pass,
            uniform,
            targets,
            current: 0,
        }
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        uniforms: &FeedbackUniforms,
    ) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniforms));
        let next = 1 - self.current;
        let bind_group = self.pass.bind_group(
            device,
            &self.uniform,
            &[input, &self.targets[self.current].view],
        );
        self.pass
            .draw(encoder, &self.targets[next].view, &bind_group, true);
        self.current = next;
    }

    /// The latest frame, with trails.
    pub fn output(&self) -> &wgpu::TextureView {
        &self.targets[self.current].view
    }

    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder) {
        for target in &self.targets {
            gpu::clear(encoder, &target.view);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_matches_shader() {
        // Building the pipeline validates the WGSL against the bind group layout.
        let Some((device, _queue)) = crate::gpu::test_device() else {
            return;
        };
        FeedbackPass::new(&device, 64, 64);
    }

    #[test]
    fn uniform_layout_matches_wgsl() {
        assert_eq!(size_of::<FeedbackUniforms>(), 32);
    }
}
