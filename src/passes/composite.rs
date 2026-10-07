//! Composite pass: combines feedback and bloom, applies CRT effects, and draws
//! the letterboxed image into the output (the window, or an offscreen texture).

use bytemuck::{Pod, Zeroable};

use crate::gpu::{FullscreenPass, PassDesc};
use crate::params::GlowParams;

/// Matches `struct Composite` in composite.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CompositeUniforms {
    pub glow: [f32; 4],
    pub misc: [f32; 4],
    pub fit: [f32; 4],
}

/// Fraction of the output covered by the internal image, keeping its aspect ratio.
pub fn letterbox(output: (u32, u32), internal: (u32, u32)) -> [f32; 2] {
    let output_aspect = output.0 as f32 / output.1 as f32;
    let internal_aspect = internal.0 as f32 / internal.1 as f32;
    if output_aspect > internal_aspect {
        [internal_aspect / output_aspect, 1.0]
    } else {
        [1.0, output_aspect / internal_aspect]
    }
}

pub fn uniforms(
    p: &GlowParams,
    time: f32,
    internal: (u32, u32),
    output: (u32, u32),
    encode_srgb: bool,
) -> CompositeUniforms {
    let fit = letterbox(output, internal);
    CompositeUniforms {
        glow: [
            p.bloom_intensity,
            p.scanline_strength,
            p.scanline_count,
            p.chroma / internal.0 as f32,
        ],
        misc: [p.noise, time, internal.0 as f32, internal.1 as f32],
        fit: [fit[0], fit[1], if encode_srgb { 1.0 } else { 0.0 }, 0.0],
    }
}

pub struct CompositePass {
    pass: FullscreenPass,
    uniform: wgpu::Buffer,
    encode_srgb: bool,
}

impl CompositePass {
    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Self {
        let pass = FullscreenPass::new(
            device,
            &PassDesc {
                label: "composite",
                shader: include_str!("../../shaders/composite.wgsl"),
                fragment_entry: "fs_main",
                uniform_size: size_of::<CompositeUniforms>() as u64,
                textures: 2,
                format: output_format,
                blend: None,
            },
        );
        let uniform = pass.create_uniform_buffer(device);
        Self {
            pass,
            uniform,
            encode_srgb: !output_format.is_srgb(),
        }
    }

    /// Whether the shader must encode sRGB itself because the output format is not sRGB.
    pub fn encode_srgb(&self) -> bool {
        self.encode_srgb
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        image: &wgpu::TextureView,
        bloom: &wgpu::TextureView,
        output: &wgpu::TextureView,
        uniforms: &CompositeUniforms,
    ) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniforms));
        let bind_group = self.pass.bind_group(device, &self.uniform, &[image, bloom]);
        self.pass.draw(encoder, output, &bind_group, true);
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
        CompositePass::new(&device, wgpu::TextureFormat::Bgra8UnormSrgb);
    }

    #[test]
    fn uniform_layout_matches_wgsl() {
        assert_eq!(size_of::<CompositeUniforms>(), 48);
    }

    #[test]
    fn wide_window_gets_pillarboxed() {
        assert_eq!(letterbox((3200, 900), (1600, 900)), [0.5, 1.0]);
    }

    #[test]
    fn tall_window_gets_letterboxed() {
        assert_eq!(letterbox((1600, 1800), (1600, 900)), [1.0, 0.5]);
    }

    #[test]
    fn chroma_is_converted_to_uv() {
        let mut p = crate::params::Params::default().glow;
        p.chroma = 2.0;
        let u = uniforms(&p, 0.0, (1000, 500), (1000, 500), false);
        assert!((u.glow[3] - 0.002).abs() < 1e-7);
    }

    #[test]
    fn srgb_encode_flag_is_packed() {
        let p = crate::params::Params::default().glow;
        let on = uniforms(&p, 0.0, (1000, 500), (1000, 500), true);
        let off = uniforms(&p, 0.0, (1000, 500), (1000, 500), false);
        assert_eq!(on.fit[2], 1.0);
        assert_eq!(off.fit[2], 0.0);
    }
}
