//! wgpu setup and the small helpers shared by every render pass.

use std::num::NonZeroU64;

use anyhow::{Context, Result, anyhow};

/// Format of every internal render target. Float so glow and feedback have headroom above 1.0.
pub const INTERNAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Shared full-screen-triangle vertex shader and helpers, prepended to every pass shader.
const FULLSCREEN_WGSL: &str = include_str!("../shaders/fullscreen.wgsl");

/// Backend from `RASTERWARP_BACKEND`, defaulting to Vulkan.
pub fn backends_from_env() -> Result<wgpu::Backends> {
    match std::env::var("RASTERWARP_BACKEND") {
        Ok(name) => parse_backend(&name),
        Err(_) => Ok(wgpu::Backends::VULKAN),
    }
}

pub fn parse_backend(name: &str) -> Result<wgpu::Backends> {
    match name.trim().to_ascii_lowercase().as_str() {
        "vulkan" => Ok(wgpu::Backends::VULKAN),
        "dx12" => Ok(wgpu::Backends::DX12),
        "gl" => Ok(wgpu::Backends::GL),
        other => Err(anyhow!(
            "unknown RASTERWARP_BACKEND '{other}' (expected vulkan, dx12 or gl)"
        )),
    }
}

pub fn create_instance(backends: wgpu::Backends) -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

/// Picks a high-performance adapter for `backends` and opens a device on it.
pub async fn request_device(
    instance: &wgpu::Instance,
    backends: wgpu::Backends,
    surface: Option<&wgpu::Surface<'_>>,
) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: surface,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        })
        .await
        .with_context(|| {
            format!("no GPU adapter for backend {backends:?}; try RASTERWARP_BACKEND=dx12 or gl")
        })?;
    let info = adapter.get_info();
    log::info!("GPU adapter: {} ({:?})", info.name, info.backend);
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("rasterwarp"),
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        })
        .await
        .context("failed to open GPU device")?;
    Ok((adapter, device, queue))
}

/// A texture that passes render into and later passes sample from.
pub struct RenderTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl RenderTarget {
    pub fn new(
        device: &wgpu::Device,
        label: &str,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view }
    }
}

/// Clears `view` to black.
pub fn clear(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
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
}

/// Describes one full-screen fragment pass.
pub struct PassDesc<'a> {
    pub label: &'a str,
    /// WGSL fragment code; `fullscreen.wgsl` is prepended automatically.
    pub shader: &'a str,
    pub fragment_entry: &'a str,
    /// Size in bytes of the uniform struct bound at `@group(0) @binding(0)`.
    pub uniform_size: u64,
    /// Number of textures bound at `@binding(2)` onward (binding 1 is the sampler).
    pub textures: u32,
    pub format: wgpu::TextureFormat,
    pub blend: Option<wgpu::BlendState>,
}

/// A pipeline that draws one full-screen triangle with a uniform buffer, a linear
/// clamp sampler, and N textures bound in group 0.
pub struct FullscreenPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_size: u64,
}

impl FullscreenPass {
    pub fn new(device: &wgpu::Device, desc: &PassDesc) -> Self {
        let mut entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    // wgpu checks this against the WGSL struct size when building the pipeline.
                    min_binding_size: NonZeroU64::new(desc.uniform_size),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ];
        for i in 0..desc.textures {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 2 + i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(desc.label),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(desc.label),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(desc.label),
            source: wgpu::ShaderSource::Wgsl(format!("{FULLSCREEN_WGSL}\n{}", desc.shader).into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(desc.label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(desc.fragment_entry),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: desc.format,
                    blend: desc.blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(desc.label),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
            uniform_size: desc.uniform_size,
        }
    }

    pub fn create_uniform_buffer(&self, device: &wgpu::Device) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pass uniforms"),
            size: self.uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn bind_group(
        &self,
        device: &wgpu::Device,
        uniform: &wgpu::Buffer,
        textures: &[&wgpu::TextureView],
    ) -> wgpu::BindGroup {
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&self.sampler),
            },
        ];
        for (i, view) in textures.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: 2 + i as u32,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &entries,
        })
    }

    /// Draws into `target`. `clear` clears it to black first; otherwise the existing
    /// contents are kept (for additive blending).
    pub fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        bind_group: &wgpu::BindGroup,
        clear: bool,
    ) {
        let load = if clear {
            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
        } else {
            wgpu::LoadOp::Load
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// A GPU device for tests, or `None` (with a message) on machines without one.
#[cfg(test)]
pub(crate) fn test_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let backends = wgpu::Backends::all();
    let instance = create_instance(backends);
    match pollster::block_on(request_device(&instance, backends, None)) {
        Ok((_adapter, device, queue)) => Some((device, queue)),
        Err(err) => {
            eprintln!("skipping GPU test: {err:#}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SHADER: &str = "
struct U { tint: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var tex: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(tex, samp, in.uv, 0.0) + u.tint;
}
";

    #[test]
    fn fullscreen_pass_builds_and_draws() {
        let Some((device, queue)) = test_device() else {
            return;
        };
        let pass = FullscreenPass::new(
            &device,
            &PassDesc {
                label: "test",
                shader: TEST_SHADER,
                fragment_entry: "fs_main",
                uniform_size: 16,
                textures: 1,
                format: INTERNAL_FORMAT,
                blend: None,
            },
        );
        let input = RenderTarget::new(&device, "test input", 8, 8, INTERNAL_FORMAT);
        let output = RenderTarget::new(&device, "test output", 8, 8, INTERNAL_FORMAT);
        let uniform = pass.create_uniform_buffer(&device);
        let bind_group = pass.bind_group(&device, &uniform, &[&input.view]);
        let mut encoder = device.create_command_encoder(&Default::default());
        clear(&mut encoder, &input.view);
        pass.draw(&mut encoder, &output.view, &bind_group, true);
        queue.submit([encoder.finish()]);
    }

    #[test]
    fn parses_backend_names() {
        assert_eq!(parse_backend("vulkan").unwrap(), wgpu::Backends::VULKAN);
        assert_eq!(parse_backend(" DX12 ").unwrap(), wgpu::Backends::DX12);
        assert_eq!(parse_backend("gl").unwrap(), wgpu::Backends::GL);
        assert!(parse_backend("metal").is_err());
    }
}
