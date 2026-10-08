//! Deflection pass: samples the source at oscillator-displaced coordinates.

use bytemuck::{Pod, Zeroable};

use crate::blend::{MAX_SLOTS, OscSlot, WarpFrame};
use crate::gpu::{FullscreenPass, INTERNAL_FORMAT, PassDesc, RenderTarget};
use crate::params::{Axis, OscInput, Waveform};

/// Matches `struct Osc` in warp.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct OscUniform {
    pub mode: [u32; 4],
    pub wave: [f32; 4],
    pub lfo: [f32; 4],
}

/// Matches `struct Deflection` in deflection.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct WarpUniforms {
    pub frame: [f32; 4],
    pub transform: [f32; 4],
    pub source_size: [f32; 4],
    pub look: [f32; 4],
    pub seed: [u32; 4],
    pub osc: [OscUniform; MAX_SLOTS],
}

/// The deflection functions shared with the raster pass, followed by the warp shader.
const SHADER: &str = concat!(
    include_str!("../../shaders/deflection.wgsl"),
    "\n",
    include_str!("../../shaders/warp.wgsl")
);

/// Size of the source inside the frame (frame height = 1), fitted so the whole
/// source is visible.
pub fn source_fit(frame_aspect: f32, source_aspect: f32) -> [f32; 2] {
    if source_aspect >= frame_aspect {
        [frame_aspect, frame_aspect / source_aspect]
    } else {
        [source_aspect, 1.0]
    }
}

/// `canvas_height` (pixels) converts the line jitter to frame heights.
pub fn uniforms(
    w: &WarpFrame,
    time: f32,
    frame_aspect: f32,
    source_aspect: f32,
    canvas_height: u32,
) -> WarpUniforms {
    let fit = source_fit(frame_aspect, source_aspect);
    let count = w.oscillators.len().min(MAX_SLOTS);
    let mut osc = [OscUniform::zeroed(); MAX_SLOTS];
    for (dst, slot) in osc.iter_mut().zip(&w.oscillators) {
        *dst = osc_uniform(slot);
    }
    WarpUniforms {
        frame: [time, frame_aspect, w.drift, count as f32],
        transform: [w.zoom, w.rotation, w.offset[0], w.offset[1]],
        source_size: [fit[0], fit[1], 0.0, 0.0],
        look: [
            w.line_jitter / canvas_height as f32,
            w.axis_wander,
            0.0,
            0.0,
        ],
        seed: [w.seed, 0, 0, 0],
        osc,
    }
}

fn osc_uniform(o: &OscSlot) -> OscUniform {
    OscUniform {
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
            o.index as u32,
        ],
        wave: [o.frequency, o.amplitude, o.phase, 0.0],
        lfo: [o.lfo_phase, o.lfo_depth, 0.0, 0.0],
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
                shader: SHADER,
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
    use crate::blend::FrameParams;
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
        // Osc = 3 x vec4 = 48 bytes; Deflection = 5 x vec4 + 8 x Osc = 464 bytes.
        assert_eq!(size_of::<OscUniform>(), 48);
        assert_eq!(size_of::<WarpUniforms>(), 464);
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
    fn packs_oscillator_slots() {
        let p = Params::default();
        let frame = FrameParams::at_rest(&p);
        let u = uniforms(&frame.warp, 2.0, 16.0 / 9.0, 1.0, 1080);
        assert_eq!(u.frame[0], 2.0);
        assert_eq!(u.frame[3], 4.0, "slot count");
        // Default oscillator 2: triangle, Y target, U input, oscillator index 1.
        assert_eq!(u.osc[1].mode, [1, 1, 0, 1]);
        assert_eq!(u.osc[0].wave[1], p.warp.oscillators[0].amplitude);
        assert_eq!(u.osc[4].wave, [0.0; 4], "unused slots are zero");
    }

    #[test]
    fn packs_all_eight_slots_when_every_oscillator_crossfades() {
        let mut a = Params::default();
        let mut b = Params::default();
        for (oa, ob) in a.warp.oscillators.iter_mut().zip(&mut b.warp.oscillators) {
            oa.amplitude = 0.05;
            oa.waveform = Waveform::Sine;
            ob.amplitude = 0.05;
            ob.waveform = Waveform::Square;
        }
        let clocks = crate::blend::Clocks::default();
        let frame = crate::blend::blend(&a, &b, 0.5, Some(0.5), &clocks, &clocks);
        assert_eq!(frame.warp.oscillators.len(), MAX_SLOTS);
        let u = uniforms(&frame.warp, 0.0, 16.0 / 9.0, 1.0, 1080);
        assert_eq!(u.frame[3], MAX_SLOTS as f32, "slot count");
        assert!(u.osc[MAX_SLOTS - 1].wave[1] > 0.0, "last slot is packed");
    }

    #[test]
    fn packs_line_jitter_in_frame_heights_and_the_seed() {
        let mut frame = FrameParams::at_rest(&Params::default());
        frame.warp.line_jitter = 4.0;
        frame.warp.axis_wander = 0.6;
        frame.warp.seed = 77;
        let u = uniforms(&frame.warp, 0.0, 16.0 / 9.0, 1.0, 800);
        assert_eq!(u.look, [0.005, 0.6, 0.0, 0.0]);
        assert_eq!(u.seed, [77, 0, 0, 0]);
    }
}
