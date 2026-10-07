//! Bloom pass: bright-pass, a chain of half-size downsamples, then additive upsamples.

use bytemuck::{Pod, Zeroable};

use crate::gpu::{FullscreenPass, INTERNAL_FORMAT, PassDesc, RenderTarget};

/// Number of mip levels in the chain; level 0 is half the internal resolution.
pub const LEVELS: usize = 5;

/// Matches `struct Bloom` in bloom.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct BloomUniforms {
    pub settings: [f32; 4],
}

/// Sizes of the bloom chain for a given internal resolution, halving each level.
pub fn level_sizes(width: u32, height: u32) -> [(u32, u32); LEVELS] {
    std::array::from_fn(|i| ((width >> (i + 1)).max(1), (height >> (i + 1)).max(1)))
}

pub struct BloomPass {
    prefilter: FullscreenPass,
    down: FullscreenPass,
    up: FullscreenPass,
    uniform: wgpu::Buffer,
    levels: Vec<RenderTarget>,
}

impl BloomPass {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let shader = include_str!("../../shaders/bloom.wgsl");
        let desc = |label, fragment_entry, blend| PassDesc {
            label,
            shader,
            fragment_entry,
            uniform_size: size_of::<BloomUniforms>() as u64,
            textures: 1,
            format: INTERNAL_FORMAT,
            blend,
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        let prefilter = FullscreenPass::new(device, &desc("bloom prefilter", "fs_prefilter", None));
        let down = FullscreenPass::new(device, &desc("bloom down", "fs_down", None));
        let up = FullscreenPass::new(device, &desc("bloom up", "fs_up", Some(additive)));
        let uniform = prefilter.create_uniform_buffer(device);
        let levels = level_sizes(width, height)
            .iter()
            .map(|&(w, h)| RenderTarget::new(device, "bloom level", w, h, INTERNAL_FORMAT))
            .collect();
        Self {
            prefilter,
            down,
            up,
            uniform,
            levels,
        }
    }

    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        threshold: f32,
    ) {
        let uniforms = BloomUniforms {
            settings: [threshold, 0.0, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniforms));

        let bind_group = self.prefilter.bind_group(device, &self.uniform, &[input]);
        self.prefilter
            .draw(encoder, &self.levels[0].view, &bind_group, true);
        for i in 1..LEVELS {
            let bind_group =
                self.down
                    .bind_group(device, &self.uniform, &[&self.levels[i - 1].view]);
            self.down
                .draw(encoder, &self.levels[i].view, &bind_group, true);
        }
        for i in (0..LEVELS - 1).rev() {
            let bind_group = self
                .up
                .bind_group(device, &self.uniform, &[&self.levels[i + 1].view]);
            self.up
                .draw(encoder, &self.levels[i].view, &bind_group, false);
        }
    }

    /// The accumulated glow, at half the internal resolution.
    pub fn output(&self) -> &wgpu::TextureView {
        &self.levels[0].view
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
        BloomPass::new(&device, 64, 64);
    }

    #[test]
    fn uniform_layout_matches_wgsl() {
        assert_eq!(size_of::<BloomUniforms>(), 16);
    }

    #[test]
    fn levels_halve_and_never_reach_zero() {
        let sizes = level_sizes(1920, 1080);
        assert_eq!(sizes[0], (960, 540));
        assert_eq!(sizes[4], (60, 33));
        assert_eq!(level_sizes(8, 8)[4], (1, 1));
    }
}
