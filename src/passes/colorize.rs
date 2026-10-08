//! Colorize pass: posterizes the warped grayscale and maps levels to palette colors.

use bytemuck::{Pod, Zeroable};

use crate::blend::ColorizeFrame;
use crate::fringe;
use crate::gpu::{FullscreenPass, INTERNAL_FORMAT, PassDesc, RenderTarget};
use crate::params::KeyParams;
use crate::params::{PALETTE_SIZE, THRESHOLD_COUNT};

/// Matches `struct Colorize` in colorize.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ColorizeUniforms {
    pub settings: [f32; 4],
    pub palette: [[f32; 4]; PALETTE_SIZE],
    pub thresholds: [[f32; 4]; 2],
    pub fringe: [[f32; 4]; fringe::TAPS / 4],
    pub extra: [f32; 4],
}

pub fn uniforms(c: &ColorizeFrame, key: &KeyParams) -> ColorizeUniforms {
    let (weights, taps) = fringe::kernel(c.bandwidth, c.ringing);
    ColorizeUniforms {
        settings: [
            c.levels as f32,
            c.softness,
            c.cycle,
            if c.bypass { 1.0 } else { 0.0 },
        ],
        palette: c.palette_linear.map(|[r, g, b]| [r, g, b, 1.0]),
        thresholds: std::array::from_fn(|row| {
            std::array::from_fn(|col| c.thresholds.get(row * 4 + col).copied().unwrap_or(1.0))
        }),
        fringe: std::array::from_fn(|row| std::array::from_fn(|col| weights[row * 4 + col])),
        extra: [
            taps as f32,
            if key.enabled { 1.0 } else { 0.0 },
            f32::from(key.levels),
            0.0,
        ],
    }
}

const _: () = assert!(THRESHOLD_COUNT <= 8, "thresholds fit in two vec4s");

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
    use crate::params::{KeyParams, Params};

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
        // vec4 settings, array<vec4, 8> palette, array<vec4, 2> thresholds,
        // array<vec4, 12> smear weights, vec4 extra.
        assert_eq!(size_of::<ColorizeUniforms>(), 384);
    }

    #[test]
    fn packs_frame_values() {
        let mut frame = FrameParams::at_rest(&Params::default()).colorize;
        frame.cycle = 2.5;
        frame.bypass = true;
        frame.palette_linear[1] = [0.25, 0.5, 0.75];
        frame.thresholds = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7];
        let u = uniforms(&frame, &Params::default().key);
        assert_eq!(u.settings, [6.0, frame.softness, 2.5, 1.0]);
        assert_eq!(u.palette[1], [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(u.thresholds, [[0.1, 0.2, 0.3, 0.4], [0.5, 0.6, 0.7, 1.0]]);
    }

    #[test]
    fn packs_the_smear_kernel() {
        let mut frame = FrameParams::at_rest(&Params::default()).colorize;
        frame.bandwidth = 0.0;
        let key = Params::default().key;
        let u = uniforms(&frame, &key);
        assert_eq!(u.extra[0], 1.0, "one tap");
        assert_eq!(u.fringe[0], [1.0, 0.0, 0.0, 0.0]);
        frame.bandwidth = 3.0;
        frame.ringing = 0.4;
        let (weights, taps) = fringe::kernel(3.0, 0.4);
        let u = uniforms(&frame, &key);
        assert_eq!(u.extra[0], taps as f32);
        assert_eq!(u.fringe[2][1], weights[9]);
    }

    #[test]
    fn packs_the_keyed_levels() {
        let frame = FrameParams::at_rest(&Params::default()).colorize;
        let key = KeyParams {
            enabled: true,
            levels: 0b101,
        };
        assert_eq!(uniforms(&frame, &key).extra[1..3], [1.0, 5.0]);
        let off = KeyParams {
            enabled: false,
            ..key
        };
        assert_eq!(uniforms(&frame, &off).extra[1], 0.0);
    }
}
