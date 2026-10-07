//! Deflection pass: samples the source at oscillator-displaced coordinates.

use bytemuck::{Pod, Zeroable};

use crate::gpu::{FullscreenPass, INTERNAL_FORMAT, PassDesc, RenderTarget};
use crate::params::{Axis, OSCILLATOR_COUNT, OscInput, WarpParams, Waveform};

/// Matches `struct Osc` in warp.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct OscUniform {
    pub mode: [u32; 4],
    pub wave: [f32; 4],
    pub lfo: [f32; 4],
}

/// Matches `struct Warp` in warp.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct WarpUniforms {
    pub frame: [f32; 4],
    pub transform: [f32; 4],
    pub source_size: [f32; 4],
    pub osc: [OscUniform; OSCILLATOR_COUNT],
}

/// Size of the source inside the frame (frame height = 1), fitted so the whole
/// source is visible.
pub fn source_fit(frame_aspect: f32, source_aspect: f32) -> [f32; 2] {
    if source_aspect >= frame_aspect {
        [frame_aspect, frame_aspect / source_aspect]
    } else {
        [source_aspect, 1.0]
    }
}

pub fn uniforms(p: &WarpParams, time: f32, frame_aspect: f32, source_aspect: f32) -> WarpUniforms {
    let fit = source_fit(frame_aspect, source_aspect);
    WarpUniforms {
        frame: [time, frame_aspect, p.drift, 0.0],
        transform: [p.zoom, p.rotation, p.offset[0], p.offset[1]],
        source_size: [fit[0], fit[1], 0.0, 0.0],
        osc: p.oscillators.map(|o| OscUniform {
            mode: [
                match o.waveform {
                    Waveform::Sine => 0,
                    Waveform::Triangle => 1,
                    Waveform::Ramp => 2,
                    Waveform::Square => 3,
                    Waveform::Noise => 4,
                },
                match o.target {
                    Axis::X => 0,
                    Axis::Y => 1,
                },
                match o.input {
                    OscInput::U => 0,
                    OscInput::V => 1,
                    OscInput::Radius => 2,
                    OscInput::Time => 3,
                },
                0,
            ],
            wave: [o.frequency, o.amplitude, o.phase, o.phase_speed],
            lfo: [o.lfo_rate, o.lfo_depth, 0.0, 0.0],
        }),
    }
}

pub struct WarpPass {
    pass: FullscreenPass,
    uniform: wgpu::Buffer,
    pub target: RenderTarget,
}

impl WarpPass {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let pass = FullscreenPass::new(
            device,
            &PassDesc {
                label: "warp",
                shader: include_str!("../../shaders/warp.wgsl"),
                fragment_entry: "fs_main",
                uniform_size: size_of::<WarpUniforms>() as u64,
                textures: 1,
                format: INTERNAL_FORMAT,
                blend: None,
            },
        );
        let uniform = pass.create_uniform_buffer(device);
        let target = RenderTarget::new(device, "warp", width, height, INTERNAL_FORMAT);
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
        source: &wgpu::TextureView,
        uniforms: &WarpUniforms,
    ) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniforms));
        let bind_group = self.pass.bind_group(device, &self.uniform, &[source]);
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
        WarpPass::new(&device, 64, 64);
    }

    #[test]
    fn uniform_layout_matches_wgsl() {
        // Osc = 3 x vec4 = 48 bytes; Warp = 3 x vec4 + 4 x Osc = 240 bytes.
        assert_eq!(size_of::<OscUniform>(), 48);
        assert_eq!(size_of::<WarpUniforms>(), 240);
    }

    #[test]
    fn wide_source_fits_frame_width() {
        assert_eq!(source_fit(1.5, 3.0), [1.5, 0.5]);
    }

    #[test]
    fn tall_source_fits_frame_height() {
        assert_eq!(source_fit(1.5, 0.5), [0.5, 1.0]);
    }

    #[test]
    fn packs_oscillator_modes() {
        let p = Params::default().warp;
        let u = uniforms(&p, 2.0, 16.0 / 9.0, 1.0);
        assert_eq!(u.frame[0], 2.0);
        // Default oscillator 1: triangle, Y target, U input.
        assert_eq!(u.osc[1].mode, [1, 1, 0, 0]);
        assert_eq!(u.osc[0].wave[1], p.oscillators[0].amplitude);
    }
}
