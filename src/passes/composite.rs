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
    pub background: [f32; 4],
}

/// The part of the output the canvas is fitted into, in pixels. The rest of the output
/// stays black (in the window, the control panel covers it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Area {
    /// The whole output.
    pub fn whole(size: (u32, u32)) -> Self {
        Self {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        }
    }

    /// The pixels inside `[left, top, right, bottom]`, rounded, kept inside an output of
    /// `size` pixels and at least one pixel each way.
    pub fn within(size: (u32, u32), [left, top, right, bottom]: [f32; 4]) -> Self {
        let edge = |v: f32, max: u32| (v.round().max(0.0) as u32).min(max);
        let x = edge(left, size.0.saturating_sub(1));
        let y = edge(top, size.1.saturating_sub(1));
        let right = edge(right, size.0).max(x + 1);
        let bottom = edge(bottom, size.1).max(y + 1);
        Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// `[x, y, width, height]`, as a render pass viewport.
    pub fn viewport(&self) -> [f32; 4] {
        [
            self.x as f32,
            self.y as f32,
            self.width as f32,
            self.height as f32,
        ]
    }
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

/// The background's uv scale for a "cover" fit: it fills the canvas, cropping whichever
/// way it is too long.
pub fn cover(canvas_aspect: f32, image_aspect: f32) -> [f32; 2] {
    if image_aspect > canvas_aspect {
        [canvas_aspect / image_aspect, 1.0]
    } else {
        [1.0, image_aspect / canvas_aspect]
    }
}

pub fn uniforms(
    p: &GlowParams,
    time: f32,
    internal: (u32, u32),
    output: (u32, u32),
    encode_srgb: bool,
    background_aspect: f32,
) -> CompositeUniforms {
    let fit = letterbox(output, internal);
    let canvas_aspect = internal.0 as f32 / internal.1 as f32;
    let back = cover(canvas_aspect, background_aspect);
    CompositeUniforms {
        glow: [
            p.bloom_intensity,
            p.scanline_strength,
            p.scanline_count,
            p.chroma / internal.0 as f32,
        ],
        misc: [p.noise, time, internal.0 as f32, internal.1 as f32],
        fit: [fit[0], fit[1], if encode_srgb { 1.0 } else { 0.0 }, 0.0],
        background: [back[0], back[1], 0.0, 0.0],
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
                textures: 3,
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
        background: &wgpu::TextureView,
        output: &wgpu::TextureView,
        area: Area,
        uniforms: &CompositeUniforms,
    ) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniforms));
        let bind_group = self
            .pass
            .bind_group(device, &self.uniform, &[image, bloom, background]);
        self.pass
            .draw_in(encoder, output, &bind_group, area.viewport());
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
        assert_eq!(size_of::<CompositeUniforms>(), 64);
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
        let u = uniforms(&p, 0.0, (1000, 500), (1000, 500), false, 1.0);
        assert!((u.glow[3] - 0.002).abs() < 1e-7);
    }

    #[test]
    fn srgb_encode_flag_is_packed() {
        let p = crate::params::Params::default().glow;
        let on = uniforms(&p, 0.0, (1000, 500), (1000, 500), true, 1.0);
        let off = uniforms(&p, 0.0, (1000, 500), (1000, 500), false, 1.0);
        assert_eq!(on.fit[2], 1.0);
        assert_eq!(off.fit[2], 0.0);
    }

    #[test]
    fn background_covers_the_canvas() {
        // A square background on a 2:1 canvas: full width, the middle half of its height.
        assert_eq!(cover(2.0, 1.0), [1.0, 0.5]);
        // A 4:1 background on a 2:1 canvas: full height, the middle half of its width.
        assert_eq!(cover(2.0, 4.0), [0.5, 1.0]);
        let p = crate::params::Params::default().glow;
        let u = uniforms(&p, 0.0, (1000, 500), (1000, 500), false, 1.0);
        assert_eq!(u.background, [1.0, 0.5, 0.0, 0.0]);
    }

    #[test]
    fn the_area_is_rounded_to_whole_pixels() {
        // A 320-point panel at 150% scaling leaves the window right of pixel 480.
        let area = Area::within((1920, 1080), [480.4, 0.0, 1920.0, 1080.0]);
        assert_eq!(
            area,
            Area {
                x: 480,
                y: 0,
                width: 1440,
                height: 1080
            }
        );
        assert_eq!(
            Area::whole((800, 600)),
            Area::within((800, 600), [0.0, 0.0, 800.0, 600.0])
        );
    }

    #[test]
    fn the_area_stays_inside_the_window_and_is_never_empty() {
        let area = Area::within((800, 600), [-10.0, -5.0, 900.0, 700.0]);
        assert_eq!(area, Area::whole((800, 600)));
        // A panel as wide as the window still leaves one pixel column.
        let area = Area::within((800, 600), [800.0, 0.0, 800.0, 600.0]);
        assert_eq!((area.x, area.width), (799, 1));
    }
}
