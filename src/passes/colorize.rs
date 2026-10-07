//! Colorize pass: posterizes the warped grayscale and maps levels to palette colors.

use bytemuck::{Pod, Zeroable};

use crate::blend::ColorizeFrame;
use crate::gpu::{FullscreenPass, INTERNAL_FORMAT, PassDesc, RenderTarget};
use crate::params::PALETTE_SIZE;

/// Matches `struct Colorize` in colorize.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ColorizeUniforms {
    pub settings: [f32; 4],
    pub palette: [[f32; 4]; PALETTE_SIZE],
}

pub fn uniforms(c: &ColorizeFrame) -> ColorizeUniforms {
    ColorizeUniforms {
        settings: [
            c.levels as f32,
            c.softness,
            c.cycle,
            if c.bypass { 1.0 } else { 0.0 },
        ],
        palette: c.palette_linear.map(|[r, g, b]| [r, g, b, 1.0]),
    }
}

pub struct ColorizePass {
    pass: FullscreenPass,
    uniform: wgpu::Buffer,
    pub target: RenderTarget,
}

impl ColorizePass {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let pass = FullscreenPass::new(
            device,
            &PassDesc {
                label: "colorize",
                shader: include_str!("../../shaders/colorize.wgsl"),
                fragment_entry: "fs_main",
                uniform_size: size_of::<ColorizeUniforms>() as u64,
                textures: 1,
                format: INTERNAL_FORMAT,
                blend: None,
            },
        );
        let uniform = pass.create_uniform_buffer(device);
        let target = RenderTarget::new(device, "colorize", width, height, INTERNAL_FORMAT);
        Self {
            pass,
            uniform,
            target,
        }
    }

    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        uniforms: &ColorizeUniforms,
    ) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniforms));
        let bind_group = self.pass.bind_group(device, &self.uniform, &[input]);
        self.pass
            .draw(encoder, &self.target.view, &bind_group, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend::FrameParams;
    use crate::params::Params;

    #[test]
    fn pipeline_matches_shader() {
        // Building the pipeline validates the WGSL against the bind group layout.
        let Some((device, _queue)) = crate::gpu::test_device() else {
            return;
        };
        ColorizePass::new(&device, 64, 64);
    }

    #[test]
    fn uniform_layout_matches_wgsl() {
        // vec4 settings + array<vec4, 8> = 16 + 128 bytes.
        assert_eq!(size_of::<ColorizeUniforms>(), 144);
    }

    #[test]
    fn packs_frame_values() {
        let mut frame = FrameParams::at_rest(&Params::default()).colorize;
        frame.cycle = 2.5;
        frame.bypass = true;
        frame.palette_linear[1] = [0.25, 0.5, 0.75];
        let u = uniforms(&frame);
        assert_eq!(u.settings, [6.0, frame.softness, 2.5, 1.0]);
        assert_eq!(u.palette[1], [0.25, 0.5, 0.75, 1.0]);
    }
}
