//! Colorize pass: posterizes the warped grayscale and maps levels to palette colors.

use bytemuck::{Pod, Zeroable};

use crate::gpu::{FullscreenPass, INTERNAL_FORMAT, PassDesc, RenderTarget};
use crate::params::{ColorizeParams, PALETTE_SIZE, srgb_to_linear};

/// Matches `struct Colorize` in colorize.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ColorizeUniforms {
    pub settings: [f32; 4],
    pub palette: [[f32; 4]; PALETTE_SIZE],
}

pub fn uniforms(p: &ColorizeParams, time: f32) -> ColorizeUniforms {
    let levels = p.levels as f32;
    // Wrap on the CPU so the shader never sees a huge, imprecise offset.
    let cycle = (time * p.cycle_speed).rem_euclid(levels);
    ColorizeUniforms {
        settings: [levels, p.softness, cycle, if p.bypass { 1.0 } else { 0.0 }],
        palette: p
            .palette
            .map(|[r, g, b]| [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), 1.0]),
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
    fn palette_is_converted_to_linear() {
        let mut p = Params::default().colorize;
        p.palette[1] = [0.5, 1.0, 0.0];
        let u = uniforms(&p, 0.0);
        assert!((u.palette[1][0] - 0.214).abs() < 1e-3);
        assert_eq!(&u.palette[1][1..], &[1.0, 0.0, 1.0]);
    }

    #[test]
    fn cycle_offset_wraps_within_levels() {
        let mut p = Params::default().colorize;
        p.levels = 4;
        p.cycle_speed = -1.0;
        let u = uniforms(&p, 5.5);
        assert!((u.settings[2] - 2.5).abs() < 1e-6);
    }
}
