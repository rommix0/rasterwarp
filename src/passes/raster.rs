//! Raster pass (true raster mode): draws the source as scan lines mapped forward through
//! the global transform and the deflection, with a Gaussian beam and additive blending.
//! It writes the same grayscale target as the warp pass, which it replaces.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};

use super::warp::WarpUniforms;
use crate::gpu::INTERNAL_FORMAT;
use crate::params::RasterParams;

/// Matches `struct Raster` in raster.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct RasterUniforms {
    pub now: WarpUniforms,
    /// The previous canvas frame's deflection, for the beam speed.
    pub before: WarpUniforms,
    pub beam: [f32; 4],
    pub misc: [f32; 4],
}

/// Shared helpers and deflection functions, followed by the raster shader.
const SHADER: &str = concat!(
    include_str!("../../shaders/fullscreen.wgsl"),
    "\n",
    include_str!("../../shaders/deflection.wgsl"),
    "\n",
    include_str!("../../shaders/raster.wgsl")
);

/// Samples along each line: one per two target pixels, from 256 to 2048.
pub fn samples(width: u32) -> u32 {
    (width / 2).clamp(256, 2048)
}

/// `target` is the target size in pixels and `pixel_scale` the target pixels per canvas
/// pixel, which converts the beam width (canvas pixels) to frame heights.
///
/// A beam narrower than one target pixel is drawn one pixel wide, so it doesn't alias,
/// and dimmed by the true width over the drawn width, so its mean brightness is kept.
pub fn uniforms(
    raster: &RasterParams,
    now: &WarpUniforms,
    before: &WarpUniforms,
    target: (u32, u32),
    pixel_scale: f32,
) -> RasterUniforms {
    let pixel = 1.0 / target.1 as f32;
    let width = raster.beam_width * pixel_scale * pixel;
    let drawn = width.max(pixel);
    RasterUniforms {
        now: *now,
        before: *before,
        beam: [
            raster.lines as f32,
            drawn,
            raster.compensation,
            raster.speed_compensation,
        ],
        misc: [samples(target.0) as f32, width / drawn, 0.0, 0.0],
    }
}

pub struct RasterPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
}

impl RasterPass {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raster"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(size_of::<RasterUniforms>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raster"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raster"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("raster"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_beam"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_beam"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: INTERNAL_FORMAT,
                    // Overlapping lines add up, so packed lines brighten.
                    blend: Some(wgpu::BlendState {
                        color: add,
                        alpha: add,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("raster"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("raster uniforms"),
            size: size_of::<RasterUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            sampler,
            uniform,
        }
    }

    /// Clears `target` and draws the lines into it.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        target: &wgpu::TextureView,
        uniforms: &RasterUniforms,
    ) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniforms));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raster"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(source),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("raster"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        let vertices = 2 * (uniforms.misc[0] as u32 + 1);
        pass.draw(0..vertices, 0..uniforms.beam[0] as u32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend::FrameParams;
    use crate::params::Params;
    use crate::passes::warp;

    #[test]
    fn pipeline_matches_shader() {
        // Building the pipeline validates the WGSL against the bind group layout.
        let Some((device, _queue)) = crate::gpu::test_device() else {
            return;
        };
        RasterPass::new(&device);
    }

    #[test]
    fn uniform_layout_matches_wgsl() {
        // Two Deflection structs (464 bytes each) and two vec4s.
        assert_eq!(size_of::<RasterUniforms>(), 960);
    }

    #[test]
    fn samples_follow_the_canvas_width() {
        assert_eq!(samples(1920), 960);
        assert_eq!(samples(320), 256);
        assert_eq!(samples(7680), 2048);
    }

    #[test]
    fn packs_the_beam_in_frame_heights() {
        let p = Params::default();
        let frame = FrameParams::at_rest(&p);
        let now = warp::uniforms(&frame.warp, 1.0, 16.0 / 9.0, 1.0, 1080, 1.0);
        let before = warp::uniforms(&frame.warp, 0.5, 16.0 / 9.0, 1.0, 1080, 1.0);
        let u = uniforms(&p.raster, &now, &before, (1920, 1080), 1.0);
        assert_eq!(u.beam[0], 600.0);
        assert!((u.beam[1] - 1.2 / 1080.0).abs() < 1e-9);
        assert_eq!((u.beam[2], u.beam[3]), (1.0, 0.3));
        assert_eq!(
            u.misc,
            [960.0, 1.0, 0.0, 0.0],
            "over a pixel wide: full gain"
        );
        assert_eq!((u.now.frame[0], u.before.frame[0]), (1.0, 0.5));
    }

    #[test]
    fn a_quarter_size_renderer_draws_the_beam_a_pixel_wide_and_dimmer() {
        let p = Params::default();
        let frame = FrameParams::at_rest(&p);
        let now = warp::uniforms(&frame.warp, 1.0, 16.0 / 9.0, 1.0, 270, 0.25);
        let u = uniforms(&p.raster, &now, &now, (480, 270), 0.25);
        // 1.2 canvas pixels are 0.3 preview pixels: drawn 1 pixel wide at 0.3 gain.
        assert!((u.beam[1] - 1.0 / 270.0).abs() < 1e-9);
        assert!((u.misc[1] - 0.3).abs() < 1e-6);
        assert_eq!(u.misc[0], 256.0);
    }
}
