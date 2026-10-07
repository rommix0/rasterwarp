# Rasterwarp Prototype Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a real-time Rust/wgpu prototype that warps grayscale artwork with oscillators, colorizes it by brightness level, and adds feedback trails, bloom and CRT effects, with a live egui parameter panel.

**Architecture:** One package (library + binary). Each pipeline stage (warp, colorize, feedback, bloom, composite) is its own module with its own WGSL shader, built on a shared `FullscreenPass` helper (uniform buffer + sampler + N textures, one full-screen triangle). A `Renderer` chains the passes through `Rgba16Float` textures at a fixed 1920×1080 internal resolution. A plain `Params` struct holds every live parameter; the egui panel edits it, and each pass packs its slice into a uniform struct every frame.

**Tech Stack:** Rust 1.98 (edition 2024), wgpu 30 (Vulkan), winit 0.30, egui / egui-wgpu / egui-winit 0.36, image 0.25, bytemuck, pollster, anyhow, log + env_logger.

**Spec:** `docs/superpowers/specs/2026-10-06-rasterwarp-prototype-design.md`

## Global Constraints

- Platform: Windows 10, NVIDIA RTX 3090, Rust 1.98 stable MSVC toolchain, edition 2024.
- Dependency versions (exact major/minor): `wgpu = "30"`, `winit = "0.30"`, `egui = "0.36"`, `egui-wgpu = "0.36"`, `egui-winit = "0.36"`, `image = "0.25"` (png + jpeg only), `bytemuck = "1"` (derive), `pollster = "1"`, `anyhow = "1"`, `log = "0.4"`, `env_logger = "0.11"`. egui 0.36 is built against wgpu 30 / winit 0.30. Do not bump one without the others.
- Backend: Vulkan by default; `RASTERWARP_BACKEND` = `vulkan` | `dx12` | `gl` overrides it.
- Internal render targets: `Rgba16Float`, fixed internal resolution 1920×1080.
- Shaders are embedded with `include_str!`; every pass shader gets `shaders/fullscreen.wgsl` prepended (it defines `VsOut`, `vs_main`, `TAU`, `rotate2`, `inside01`, `hash11`, `hash12`, `value_noise`).
- `cargo build`, `cargo test` and `cargo clippy --all-targets` must finish with zero warnings.
- Don't use the word "Scanimate" anywhere in user-facing text or branding.
- Commit after every task. Every commit message ends with the line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Where this plan intentionally differs from the spec

All of these were found while compiling and running a throwaway prototype of this exact code against the pinned crate versions:

1. **Library + binary in one package** (`src/lib.rs` + `src/main.rs`), so the headless GPU smoke test in `tests/` can use the renderer.
2. **Uniform packing lives in each pass module**, next to the WGSL struct it must match, rather than in `params.rs`. `params.rs` holds only the parameters and their ranges.
3. **Surface status handling follows wgpu 30's `CurrentSurfaceTexture`**: `Timeout`/`Occluded` skip the frame, `Outdated`/`Lost` reconfigure and skip, `Validation` logs and skips. wgpu 30 has no out-of-memory surface status.
4. **The swapchain is non-sRGB** (egui-wgpu requires that). The composite pass draws through an sRGB *view* of the same texture (`view_formats`), so its linear output is still encoded correctly.
5. **Present mode is Mailbox when available** (Fifo otherwise), so the frame-time readout shows the real render cost instead of being locked to vsync.
6. **Each pass also has a GPU test** that builds its pipeline (wgpu validates the WGSL against the bind group layout), in addition to the end-to-end smoke test.

## File Structure

| File | Responsibility |
|---|---|
| `Cargo.toml` | Package and pinned dependencies |
| `src/lib.rs` | Module declarations |
| `src/main.rs` | Logging, CLI image argument, winit event loop |
| `src/gpu.rs` | Backend selection, instance/device setup, `RenderTarget`, `clear`, `FullscreenPass`, test device helper |
| `src/params.rs` | `Params` and sub-structs, defaults, slider ranges, `srgb_to_linear` |
| `src/source.rs` | `GrayImage`, image loading, grayscale conversion, procedural test card, `R8Unorm` upload |
| `src/passes/mod.rs` | `Renderer`: owns all passes and the source texture, records one frame |
| `src/passes/warp.rs` | Oscillator deflection pass + `WarpUniforms` packing |
| `src/passes/colorize.rs` | Posterize/palette pass + `ColorizeUniforms` packing |
| `src/passes/feedback.rs` | Ping-pong feedback trails pass |
| `src/passes/bloom.rs` | Bright-pass + 5-level down/up bloom chain |
| `src/passes/composite.rs` | Bloom add, chroma bleed, scanlines, noise, letterboxing to the output |
| `src/ui.rs` | egui side panel; `UiState`, `UiActions` |
| `src/app.rs` | winit `ApplicationHandler`: window, surface, egui integration, per-frame loop, drag-and-drop |
| `shaders/*.wgsl` | `fullscreen` (shared), `warp`, `colorize`, `feedback`, `bloom`, `composite` |
| `tests/smoke.rs` | Headless end-to-end render + readback |

---

### Task 1: Project scaffold and GPU helpers

**Files:**
- Create: `Cargo.toml`, `src/lib.rs`, `shaders/fullscreen.wgsl`, `src/gpu.rs`
- Test: unit tests inside `src/gpu.rs`

**Interfaces:**
- Consumes: nothing (the repo has `HANDOFF.md`, `docs/`, and `.gitignore` containing `/target`).
- Produces (used by every later task):
  - `pub const INTERNAL_FORMAT: wgpu::TextureFormat` (= `Rgba16Float`)
  - `pub fn backends_from_env() -> anyhow::Result<wgpu::Backends>`
  - `pub fn parse_backend(name: &str) -> anyhow::Result<wgpu::Backends>`
  - `pub fn create_instance(backends: wgpu::Backends) -> wgpu::Instance`
  - `pub async fn request_device(instance: &wgpu::Instance, backends: wgpu::Backends, surface: Option<&wgpu::Surface<'_>>) -> anyhow::Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue)>`
  - `pub struct RenderTarget { pub texture: wgpu::Texture, pub view: wgpu::TextureView }` with `RenderTarget::new(device, label: &str, width: u32, height: u32, format) -> Self`
  - `pub fn clear(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView)`
  - `pub struct PassDesc<'a> { label, shader, fragment_entry, uniform_size: u64, textures: u32, format, blend: Option<wgpu::BlendState> }`
  - `pub struct FullscreenPass` with `new(device, &PassDesc) -> Self`, `create_uniform_buffer(&self, device) -> wgpu::Buffer`, `bind_group(&self, device, uniform: &wgpu::Buffer, textures: &[&wgpu::TextureView]) -> wgpu::BindGroup`, `draw(&self, encoder, target: &wgpu::TextureView, bind_group, clear: bool)`
  - `#[cfg(test)] pub(crate) fn test_device() -> Option<(wgpu::Device, wgpu::Queue)>`: returns `None` and prints a skip message when no GPU is available.
  - Bind group convention for every pass shader: `@group(0) @binding(0)` uniform struct, `@binding(1)` sampler (linear, clamp-to-edge), `@binding(2..)` `texture_2d<f32>` inputs.

- [ ] **Step 1: Create `Cargo.toml`**

```toml
[package]
name = "rasterwarp"
version = "0.1.0"
edition = "2024"

[dependencies]
anyhow = "1"
bytemuck = { version = "1", features = ["derive"] }
egui = "0.36"
egui-wgpu = "0.36"
egui-winit = "0.36"
env_logger = "0.11"
image = { version = "0.25", default-features = false, features = ["png", "jpeg"] }
log = "0.4"
pollster = "1"
wgpu = "30"
winit = "0.30"
```

- [ ] **Step 2: Create `src/lib.rs`**

```rust
//! Rasterwarp: real-time analog video-synthesis animation.

pub mod gpu;
```

- [ ] **Step 3: Write the failing tests**

Create `src/gpu.rs` containing only this test code:

```rust
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
```

- [ ] **Step 4: Run the tests and confirm they fail**

Run: `cargo test --lib gpu::`
Expected: compile errors such as "cannot find function `create_instance`" and "cannot find struct `FullscreenPass`".

- [ ] **Step 5: Create the shared shader `shaders/fullscreen.wgsl`**

```wgsl
// Shared full-screen triangle and helpers. Prepended to every pass shader by
// gpu::FullscreenPass. uv is (0,0) at the top-left and (1,1) at the bottom-right,
// matching texture coordinates.

const TAU: f32 = 6.28318530718;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

fn rotate2(p: vec2<f32>, angle: f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
}

fn inside01(uv: vec2<f32>) -> bool {
    return all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
}

fn hash11(x: f32) -> f32 {
    return fract(sin(x * 127.1) * 43758.5453);
}

fn hash12(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

// Smooth 1D value noise in [-1, 1].
fn value_noise(x: f32) -> f32 {
    let i = floor(x);
    let f = fract(x);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(hash11(i), hash11(i + 1.0), u) * 2.0 - 1.0;
}
```

- [ ] **Step 6: Write the implementation**

Replace `src/gpu.rs` with the full file:

```rust
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
```

- [ ] **Step 7: Run the tests and confirm they pass**

Run: `cargo test --lib gpu::`
Expected: `2 passed`. On the RTX 3090 `fullscreen_pass_builds_and_draws` really runs. On a machine with no GPU it prints `skipping GPU test` and passes.

- [ ] **Step 8: Lint**

Run: `cargo clippy --all-targets`
Expected: no warnings.

- [ ] **Step 9: Commit** (including `Cargo.lock`, since this is an application)

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/gpu.rs shaders/fullscreen.wgsl
git commit -m "feat: scaffold crate with wgpu setup and full-screen pass helper" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Parameters

**Files:**
- Create: `src/params.rs`
- Modify: `src/lib.rs`
- Test: unit tests inside `src/params.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub mod ranges` with `RangeInclusive<f32>` consts `FREQUENCY, AMPLITUDE, PHASE, PHASE_SPEED, LFO_RATE, LFO_DEPTH, ZOOM, ROTATION, OFFSET, DRIFT, SOFTNESS, CYCLE_SPEED, FEEDBACK_AMOUNT, FEEDBACK_ZOOM, FEEDBACK_ROTATION, FEEDBACK_OFFSET, BLOOM_INTENSITY, BLOOM_THRESHOLD, SCANLINE_STRENGTH, SCANLINE_COUNT, CHROMA, NOISE` and `LEVELS: RangeInclusive<u32>`
  - `pub const OSCILLATOR_COUNT: usize = 4; pub const PALETTE_SIZE: usize = 8;`
  - `enum Waveform { Sine, Triangle, Ramp, Square, Noise }` (+ `Waveform::ALL`), `enum Axis { X, Y }`, `enum OscInput { U, V, Radius, Time }` (+ `OscInput::ALL`)
  - `struct Oscillator { waveform, target: Axis, input: OscInput, frequency, amplitude, phase, phase_speed, lfo_rate, lfo_depth }` (all `f32` except the enums)
  - `struct WarpParams { oscillators: [Oscillator; 4], zoom, rotation, offset: [f32; 2], drift }`
  - `struct ColorizeParams { levels: u32, softness, palette: [[f32; 3]; 8] /* sRGB */, cycle_speed, bypass: bool }`
  - `struct FeedbackParams { amount, zoom, rotation, offset: [f32; 2] }`
  - `struct GlowParams { bloom_intensity, bloom_threshold, scanline_strength, scanline_count, chroma, noise }`
  - `struct Params { warp, colorize, feedback, glow }` with `impl Default`
  - `pub fn srgb_to_linear(c: f32) -> f32`

- [ ] **Step 1: Register the module**

Replace `src/lib.rs` with:

```rust
//! Rasterwarp: real-time analog video-synthesis animation.

pub mod gpu;
pub mod params;
```

- [ ] **Step 2: Write the failing tests**

Create `src/params.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_within_ui_ranges() {
        let p = Params::default();
        for o in &p.warp.oscillators {
            assert!(ranges::FREQUENCY.contains(&o.frequency));
            assert!(ranges::AMPLITUDE.contains(&o.amplitude));
            assert!(ranges::PHASE.contains(&o.phase));
            assert!(ranges::PHASE_SPEED.contains(&o.phase_speed));
            assert!(ranges::LFO_RATE.contains(&o.lfo_rate));
            assert!(ranges::LFO_DEPTH.contains(&o.lfo_depth));
        }
        assert!(ranges::ZOOM.contains(&p.warp.zoom));
        assert!(ranges::ROTATION.contains(&p.warp.rotation));
        assert!(p.warp.offset.iter().all(|v| ranges::OFFSET.contains(v)));
        assert!(ranges::DRIFT.contains(&p.warp.drift));
        assert!(ranges::LEVELS.contains(&p.colorize.levels));
        assert!(ranges::SOFTNESS.contains(&p.colorize.softness));
        assert!(ranges::CYCLE_SPEED.contains(&p.colorize.cycle_speed));
        assert!(
            p.colorize
                .palette
                .iter()
                .flatten()
                .all(|c| (0.0..=1.0).contains(c))
        );
        assert!(ranges::FEEDBACK_AMOUNT.contains(&p.feedback.amount));
        assert!(ranges::FEEDBACK_ZOOM.contains(&p.feedback.zoom));
        assert!(ranges::FEEDBACK_ROTATION.contains(&p.feedback.rotation));
        assert!(
            p.feedback
                .offset
                .iter()
                .all(|v| ranges::FEEDBACK_OFFSET.contains(v))
        );
        assert!(ranges::BLOOM_INTENSITY.contains(&p.glow.bloom_intensity));
        assert!(ranges::BLOOM_THRESHOLD.contains(&p.glow.bloom_threshold));
        assert!(ranges::SCANLINE_STRENGTH.contains(&p.glow.scanline_strength));
        assert!(ranges::SCANLINE_COUNT.contains(&p.glow.scanline_count));
        assert!(ranges::CHROMA.contains(&p.glow.chroma));
        assert!(ranges::NOISE.contains(&p.glow.noise));
    }

    #[test]
    fn default_has_a_moving_oscillator() {
        let p = Params::default();
        assert!(
            p.warp
                .oscillators
                .iter()
                .any(|o| o.amplitude > 0.0 && o.phase_speed != 0.0)
        );
    }

    #[test]
    fn srgb_to_linear_known_values() {
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-6);
        assert!((srgb_to_linear(0.5) - 0.214).abs() < 1e-3);
    }
}
```

- [ ] **Step 3: Run the tests and confirm they fail**

Run: `cargo test --lib params::`
Expected: compile errors such as "cannot find type `Params`".

- [ ] **Step 4: Write the implementation**

Replace `src/params.rs` with:

```rust
//! Every live parameter, edited by the UI and read by the render passes each frame.

use std::f32::consts::PI;
use std::ops::RangeInclusive;

/// Slider ranges. The UI uses these, and tests check that defaults fall inside them.
pub mod ranges {
    use super::*;

    pub const FREQUENCY: RangeInclusive<f32> = 0.0..=40.0;
    pub const AMPLITUDE: RangeInclusive<f32> = 0.0..=0.5;
    pub const PHASE: RangeInclusive<f32> = 0.0..=1.0;
    pub const PHASE_SPEED: RangeInclusive<f32> = -4.0..=4.0;
    pub const LFO_RATE: RangeInclusive<f32> = 0.0..=10.0;
    pub const LFO_DEPTH: RangeInclusive<f32> = 0.0..=1.0;
    pub const ZOOM: RangeInclusive<f32> = 0.1..=4.0;
    pub const ROTATION: RangeInclusive<f32> = -PI..=PI;
    pub const OFFSET: RangeInclusive<f32> = -1.0..=1.0;
    pub const DRIFT: RangeInclusive<f32> = 0.0..=1.0;
    pub const LEVELS: RangeInclusive<u32> = 2..=8;
    pub const SOFTNESS: RangeInclusive<f32> = 0.0..=1.0;
    pub const CYCLE_SPEED: RangeInclusive<f32> = -4.0..=4.0;
    pub const FEEDBACK_AMOUNT: RangeInclusive<f32> = 0.0..=0.99;
    pub const FEEDBACK_ZOOM: RangeInclusive<f32> = 0.9..=1.1;
    pub const FEEDBACK_ROTATION: RangeInclusive<f32> = -0.1..=0.1;
    pub const FEEDBACK_OFFSET: RangeInclusive<f32> = -0.02..=0.02;
    pub const BLOOM_INTENSITY: RangeInclusive<f32> = 0.0..=4.0;
    pub const BLOOM_THRESHOLD: RangeInclusive<f32> = 0.0..=1.0;
    pub const SCANLINE_STRENGTH: RangeInclusive<f32> = 0.0..=1.0;
    pub const SCANLINE_COUNT: RangeInclusive<f32> = 100.0..=1080.0;
    pub const CHROMA: RangeInclusive<f32> = 0.0..=4.0;
    pub const NOISE: RangeInclusive<f32> = 0.0..=0.5;
}

pub const OSCILLATOR_COUNT: usize = 4;
pub const PALETTE_SIZE: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waveform {
    Sine,
    Triangle,
    Ramp,
    Square,
    Noise,
}

impl Waveform {
    pub const ALL: [Waveform; 5] = [
        Waveform::Sine,
        Waveform::Triangle,
        Waveform::Ramp,
        Waveform::Square,
        Waveform::Noise,
    ];
}

/// Which displacement component an oscillator adds to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

/// What drives an oscillator's phase across the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OscInput {
    /// Horizontal position.
    U,
    /// Vertical position: X displacement driven by V gives the classic per-scanline wiggle.
    V,
    /// Distance from the frame center.
    Radius,
    /// Time only: the whole frame moves together.
    Time,
}

impl OscInput {
    pub const ALL: [OscInput; 4] = [OscInput::U, OscInput::V, OscInput::Radius, OscInput::Time];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oscillator {
    pub waveform: Waveform,
    pub target: Axis,
    pub input: OscInput,
    /// Cycles per frame height.
    pub frequency: f32,
    /// Displacement in frame heights. 0 turns the oscillator off.
    pub amplitude: f32,
    /// Phase offset in cycles.
    pub phase: f32,
    /// Phase drift in cycles per second.
    pub phase_speed: f32,
    /// Amplitude LFO rate in Hz.
    pub lfo_rate: f32,
    /// 0 = constant amplitude, 1 = amplitude swings fully between 0 and `amplitude`.
    pub lfo_depth: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WarpParams {
    pub oscillators: [Oscillator; OSCILLATOR_COUNT],
    pub zoom: f32,
    /// Radians.
    pub rotation: f32,
    /// Frame heights.
    pub offset: [f32; 2],
    /// Slow random wobble of oscillator frequency and phase.
    pub drift: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorizeParams {
    pub levels: u32,
    pub softness: f32,
    /// sRGB colors, one per brightness level (darkest first).
    pub palette: [[f32; 3]; PALETTE_SIZE],
    /// Levels per second the palette rotates through.
    pub cycle_speed: f32,
    pub bypass: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeedbackParams {
    /// How much of the previous frame survives each frame. 0 = no trails.
    pub amount: f32,
    /// Per-frame transform applied to the previous frame.
    pub zoom: f32,
    pub rotation: f32,
    pub offset: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlowParams {
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    pub scanline_strength: f32,
    pub scanline_count: f32,
    /// Chromatic bleed, in internal-resolution pixels.
    pub chroma: f32,
    pub noise: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub warp: WarpParams,
    pub colorize: ColorizeParams,
    pub feedback: FeedbackParams,
    pub glow: GlowParams,
}

impl Default for Oscillator {
    fn default() -> Self {
        Self {
            waveform: Waveform::Sine,
            target: Axis::X,
            input: OscInput::V,
            frequency: 1.0,
            amplitude: 0.0,
            phase: 0.0,
            phase_speed: 0.0,
            lfo_rate: 0.0,
            lfo_depth: 0.0,
        }
    }
}

impl Default for Params {
    fn default() -> Self {
        Self {
            warp: WarpParams {
                oscillators: [
                    Oscillator {
                        frequency: 3.0,
                        amplitude: 0.05,
                        phase_speed: 0.5,
                        lfo_rate: 0.2,
                        lfo_depth: 0.5,
                        ..Oscillator::default()
                    },
                    Oscillator {
                        waveform: Waveform::Triangle,
                        target: Axis::Y,
                        input: OscInput::U,
                        frequency: 2.0,
                        amplitude: 0.03,
                        phase_speed: -0.3,
                        ..Oscillator::default()
                    },
                    Oscillator {
                        input: OscInput::Radius,
                        frequency: 6.0,
                        phase_speed: 1.0,
                        ..Oscillator::default()
                    },
                    Oscillator {
                        waveform: Waveform::Noise,
                        frequency: 20.0,
                        ..Oscillator::default()
                    },
                ],
                zoom: 1.0,
                rotation: 0.0,
                offset: [0.0, 0.0],
                drift: 0.2,
            },
            colorize: ColorizeParams {
                levels: 6,
                softness: 0.1,
                palette: [
                    [0.0, 0.0, 0.0],
                    [0.05, 0.1, 0.6],
                    [0.7, 0.1, 0.8],
                    [1.0, 0.45, 0.1],
                    [1.0, 0.9, 0.2],
                    [1.0, 1.0, 1.0],
                    [0.2, 0.9, 1.0],
                    [0.2, 1.0, 0.4],
                ],
                cycle_speed: 0.0,
                bypass: false,
            },
            feedback: FeedbackParams {
                amount: 0.85,
                zoom: 1.01,
                rotation: 0.002,
                offset: [0.0, 0.0],
            },
            glow: GlowParams {
                bloom_intensity: 1.0,
                bloom_threshold: 0.6,
                scanline_strength: 0.3,
                scanline_count: 540.0,
                chroma: 1.0,
                noise: 0.04,
            },
        }
    }
}

/// sRGB-encoded channel to linear, for palette colors going to the GPU.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_within_ui_ranges() {
        let p = Params::default();
        for o in &p.warp.oscillators {
            assert!(ranges::FREQUENCY.contains(&o.frequency));
            assert!(ranges::AMPLITUDE.contains(&o.amplitude));
            assert!(ranges::PHASE.contains(&o.phase));
            assert!(ranges::PHASE_SPEED.contains(&o.phase_speed));
            assert!(ranges::LFO_RATE.contains(&o.lfo_rate));
            assert!(ranges::LFO_DEPTH.contains(&o.lfo_depth));
        }
        assert!(ranges::ZOOM.contains(&p.warp.zoom));
        assert!(ranges::ROTATION.contains(&p.warp.rotation));
        assert!(p.warp.offset.iter().all(|v| ranges::OFFSET.contains(v)));
        assert!(ranges::DRIFT.contains(&p.warp.drift));
        assert!(ranges::LEVELS.contains(&p.colorize.levels));
        assert!(ranges::SOFTNESS.contains(&p.colorize.softness));
        assert!(ranges::CYCLE_SPEED.contains(&p.colorize.cycle_speed));
        assert!(
            p.colorize
                .palette
                .iter()
                .flatten()
                .all(|c| (0.0..=1.0).contains(c))
        );
        assert!(ranges::FEEDBACK_AMOUNT.contains(&p.feedback.amount));
        assert!(ranges::FEEDBACK_ZOOM.contains(&p.feedback.zoom));
        assert!(ranges::FEEDBACK_ROTATION.contains(&p.feedback.rotation));
        assert!(
            p.feedback
                .offset
                .iter()
                .all(|v| ranges::FEEDBACK_OFFSET.contains(v))
        );
        assert!(ranges::BLOOM_INTENSITY.contains(&p.glow.bloom_intensity));
        assert!(ranges::BLOOM_THRESHOLD.contains(&p.glow.bloom_threshold));
        assert!(ranges::SCANLINE_STRENGTH.contains(&p.glow.scanline_strength));
        assert!(ranges::SCANLINE_COUNT.contains(&p.glow.scanline_count));
        assert!(ranges::CHROMA.contains(&p.glow.chroma));
        assert!(ranges::NOISE.contains(&p.glow.noise));
    }

    #[test]
    fn default_has_a_moving_oscillator() {
        let p = Params::default();
        assert!(
            p.warp
                .oscillators
                .iter()
                .any(|o| o.amplitude > 0.0 && o.phase_speed != 0.0)
        );
    }

    #[test]
    fn srgb_to_linear_known_values() {
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-6);
        assert!((srgb_to_linear(0.5) - 0.214).abs() < 1e-3);
    }
}
```

- [ ] **Step 5: Run the tests and confirm they pass**

Run: `cargo test --lib params::`
Expected: `3 passed`.

- [ ] **Step 6: Lint**

Run: `cargo clippy --all-targets`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs src/params.rs
git commit -m "feat: add live parameter model with defaults and ranges" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Source artwork

**Files:**
- Create: `src/source.rs`
- Modify: `src/lib.rs`
- Test: unit tests inside `src/source.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `pub struct GrayImage { pub width: u32, pub height: u32, pub pixels: Vec<u8> }` with `fn aspect(&self) -> f32`
  - `pub fn from_dynamic(image: image::DynamicImage) -> GrayImage` (Rec. 709 luma via `to_luma8`)
  - `pub fn load_image(path: &Path) -> anyhow::Result<GrayImage>`: the error message includes the path.
  - `pub fn test_card(width: u32, height: u32) -> GrayImage`
  - `pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, image: &GrayImage) -> wgpu::Texture` (`R8Unorm`, `TEXTURE_BINDING | COPY_DST`)

- [ ] **Step 1: Register the module**

Replace `src/lib.rs` with:

```rust
//! Rasterwarp: real-time analog video-synthesis animation.

pub mod gpu;
pub mod params;
pub mod source;
```

- [ ] **Step 2: Write the failing tests**

Create `src/source.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn test_card_has_requested_size() {
        let card = test_card(320, 180);
        assert_eq!((card.width, card.height), (320, 180));
        assert_eq!(card.pixels.len(), 320 * 180);
    }

    #[test]
    fn test_card_spans_many_gray_levels() {
        let card = test_card(320, 180);
        let levels: BTreeSet<u8> = card.pixels.iter().copied().collect();
        assert!(levels.len() >= 8, "only {} distinct levels", levels.len());
        assert_eq!(levels.first(), Some(&0));
        assert_eq!(levels.last(), Some(&255));
    }

    #[test]
    fn converts_rgb_to_luma() {
        let mut rgb = image::RgbImage::new(2, 1);
        rgb.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        rgb.put_pixel(1, 0, image::Rgb([255, 255, 255]));
        let gray = from_dynamic(image::DynamicImage::ImageRgb8(rgb));
        assert_eq!((gray.width, gray.height), (2, 1));
        // Rec. 709 luma: pure red is about 21% brightness.
        assert_eq!(gray.pixels, vec![54, 255]);
    }

    #[test]
    fn missing_file_is_an_error() {
        let err = load_image(Path::new("does-not-exist.png")).unwrap_err();
        assert!(err.to_string().contains("does-not-exist.png"));
    }
}
```

- [ ] **Step 3: Run the tests and confirm they fail**

Run: `cargo test --lib source::`
Expected: compile errors such as "cannot find function `test_card`".

- [ ] **Step 4: Write the implementation**

Replace `src/source.rs` with:

```rust
//! Source artwork: image files and the built-in test card, as 8-bit grayscale.

use std::path::Path;

use anyhow::{Context, Result};

/// 8-bit grayscale artwork, row-major, one byte per pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct GrayImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl GrayImage {
    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height as f32
    }
}

/// Converts any decoded image to grayscale (luma).
pub fn from_dynamic(image: image::DynamicImage) -> GrayImage {
    let luma = image.to_luma8();
    GrayImage {
        width: luma.width(),
        height: luma.height(),
        pixels: luma.into_raw(),
    }
}

pub fn load_image(path: &Path) -> Result<GrayImage> {
    let image = image::open(path).with_context(|| format!("could not load {}", path.display()))?;
    Ok(from_dynamic(image))
}

/// Procedural test card with many gray levels so every colorizer level gets used:
/// concentric rings, stepped bars, a hollow block, and a gradient strip.
pub fn test_card(width: u32, height: u32) -> GrayImage {
    let (w, h) = (width as f32, height as f32);
    let ring_levels = [255u8, 0, 200, 0, 145, 0, 90];
    let mut pixels = vec![0u8; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let (u, v) = (fx / w, fy / h);
            let ring_radius = 0.42 * h;
            let d = ((fx - 0.3 * w).powi(2) + (fy - 0.5 * h).powi(2)).sqrt() / ring_radius;
            let level = if d < 1.0 {
                ring_levels[(d * ring_levels.len() as f32) as usize]
            } else if (0.6..0.95).contains(&u) && (0.08..0.45).contains(&v) {
                let bar = ((u - 0.6) / 0.35 * 8.0) as u8;
                bar.min(7) * 32 + 31
            } else if (0.6..0.72).contains(&u) && (0.5..0.68).contains(&v) {
                let hole = (0.63..0.69).contains(&u) && (0.545..0.635).contains(&v);
                if hole { 0 } else { 220 }
            } else if (0.6..0.95).contains(&u) && (0.75..0.9).contains(&v) {
                ((u - 0.6) / 0.35 * 255.0).round() as u8
            } else {
                0
            };
            pixels[(y * width + x) as usize] = level;
        }
    }
    GrayImage {
        width,
        height,
        pixels,
    }
}

/// Uploads grayscale artwork as an `R8Unorm` texture.
pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, image: &GrayImage) -> wgpu::Texture {
    let size = wgpu::Extent3d {
        width: image.width,
        height: image.height,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("source"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &image.pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(image.width),
            rows_per_image: None,
        },
        size,
    );
    texture
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn test_card_has_requested_size() {
        let card = test_card(320, 180);
        assert_eq!((card.width, card.height), (320, 180));
        assert_eq!(card.pixels.len(), 320 * 180);
    }

    #[test]
    fn test_card_spans_many_gray_levels() {
        let card = test_card(320, 180);
        let levels: BTreeSet<u8> = card.pixels.iter().copied().collect();
        assert!(levels.len() >= 8, "only {} distinct levels", levels.len());
        assert_eq!(levels.first(), Some(&0));
        assert_eq!(levels.last(), Some(&255));
    }

    #[test]
    fn converts_rgb_to_luma() {
        let mut rgb = image::RgbImage::new(2, 1);
        rgb.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        rgb.put_pixel(1, 0, image::Rgb([255, 255, 255]));
        let gray = from_dynamic(image::DynamicImage::ImageRgb8(rgb));
        assert_eq!((gray.width, gray.height), (2, 1));
        // Rec. 709 luma: pure red is about 21% brightness.
        assert_eq!(gray.pixels, vec![54, 255]);
    }

    #[test]
    fn missing_file_is_an_error() {
        let err = load_image(Path::new("does-not-exist.png")).unwrap_err();
        assert!(err.to_string().contains("does-not-exist.png"));
    }
}
```

- [ ] **Step 5: Run the tests and confirm they pass**

Run: `cargo test --lib source::`
Expected: `4 passed`.

- [ ] **Step 6: Lint**

Run: `cargo clippy --all-targets`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs src/source.rs
git commit -m "feat: load artwork as grayscale and add procedural test card" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Warp (deflection) pass

**Files:**
- Create: `shaders/warp.wgsl`, `src/passes/warp.rs`
- Modify: `src/passes/mod.rs` (create), `src/lib.rs`
- Test: unit + GPU pipeline tests inside `src/passes/warp.rs`

**Interfaces:**
- Consumes: `gpu::{FullscreenPass, PassDesc, RenderTarget, INTERNAL_FORMAT}`, `gpu::test_device` (Task 1); `params::{WarpParams, Waveform, Axis, OscInput, OSCILLATOR_COUNT, Params}` (Task 2).
- Produces:
  - `#[repr(C)] struct OscUniform { mode: [u32; 4], wave: [f32; 4], lfo: [f32; 4] }` (48 bytes) and `#[repr(C)] struct WarpUniforms { frame, transform, source_size: [f32; 4], osc: [OscUniform; 4] }` (240 bytes)
  - `pub fn source_fit(frame_aspect: f32, source_aspect: f32) -> [f32; 2]`
  - `pub fn uniforms(p: &WarpParams, time: f32, frame_aspect: f32, source_aspect: f32) -> WarpUniforms`
  - `pub struct WarpPass { pub target: RenderTarget, .. }` with `new(device, width, height)` and `render(&self, device, queue, encoder, source: &wgpu::TextureView, uniforms: &WarpUniforms)`

> **Note:** The shader samples with `textureSampleLevel` (not `textureSample`) because it samples inside non-uniform control flow, which `textureSample` doesn't allow.

- [ ] **Step 1: Register the module**

Replace `src/lib.rs` with:

```rust
//! Rasterwarp: real-time analog video-synthesis animation.

pub mod gpu;
pub mod params;
pub mod passes;
pub mod source;
```

Create `src/passes/mod.rs`:

```rust
//! The render pipeline: source -> warp -> colorize -> feedback -> bloom -> composite.

pub mod warp;
```

- [ ] **Step 2: Write the failing tests**

Create `src/passes/warp.rs` containing only:

```rust
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
```

- [ ] **Step 3: Run the tests and confirm they fail**

Run: `cargo test --lib passes::warp::`
Expected: compile errors such as "cannot find type `WarpPass`".

- [ ] **Step 4: Write the shader `shaders/warp.wgsl`**

```wgsl
// Deflection: displace the sampling position with summed oscillators.

struct Osc {
    mode: vec4<u32>, // waveform, target (0 = X, 1 = Y), input (0 = U, 1 = V, 2 = radius, 3 = time), unused
    wave: vec4<f32>, // frequency, amplitude, phase, phase_speed
    lfo: vec4<f32>,  // rate, depth, unused, unused
};

struct Warp {
    frame: vec4<f32>,       // time, frame aspect, drift, unused
    transform: vec4<f32>,   // zoom, rotation, offset.x, offset.y
    source_size: vec4<f32>, // source width, height in frame units, unused, unused
    osc: array<Osc, 4>,
};

@group(0) @binding(0) var<uniform> u: Warp;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var source: texture_2d<f32>;

// One cycle per unit of x, output in [-1, 1].
fn wave(shape: u32, x: f32) -> f32 {
    let f = fract(x);
    switch shape {
        case 0u: { return sin(x * TAU); }
        case 1u: { return 1.0 - 4.0 * abs(f - 0.5); }
        case 2u: { return 2.0 * f - 1.0; }
        case 3u: { return select(1.0, -1.0, f >= 0.5); }
        default: { return value_noise(x); }
    }
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let time = u.frame.x;
    let aspect = u.frame.y;
    let drift = u.frame.z;
    // Centered frame coordinates: y spans [-0.5, 0.5], x spans [-aspect/2, aspect/2].
    let p = (in.uv - 0.5) * vec2<f32>(aspect, 1.0);

    var d = vec2<f32>(0.0);
    for (var i = 0u; i < 4u; i++) {
        let o = u.osc[i];
        var input = 0.0;
        switch o.mode.z {
            case 0u: { input = p.x; }
            case 1u: { input = p.y; }
            case 2u: { input = length(p); }
            default: { input = 0.0; }
        }
        let seed = f32(i) * 17.0;
        let freq = o.wave.x * (1.0 + drift * 0.05 * value_noise(time * 0.3 + seed));
        let phase = o.wave.z + o.wave.w * time + drift * 0.1 * value_noise(time * 0.2 + seed + 5.0);
        let lfo = 1.0 - o.lfo.y * (0.5 - 0.5 * cos(TAU * o.lfo.x * time));
        let v = wave(o.mode.x, input * freq + phase) * o.wave.y * lfo;
        if o.mode.y == 0u {
            d.x += v;
        } else {
            d.y += v;
        }
    }

    let q = rotate2(p + d, u.transform.y) / u.transform.x - u.transform.zw;
    let suv = q / u.source_size.xy + 0.5;
    if !inside01(suv) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let g = textureSampleLevel(source, samp, suv, 0.0).r;
    return vec4<f32>(g, g, g, 1.0);
}
```

- [ ] **Step 5: Write the implementation**

Replace `src/passes/warp.rs` with:

```rust
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
```

- [ ] **Step 6: Run the tests and confirm they pass**

Run: `cargo test --lib passes::warp::`
Expected: `5 passed`. `pipeline_matches_shader` really builds the pipeline on the GPU; a WGSL error or a uniform-size mismatch makes wgpu panic with `Validation Error`.

- [ ] **Step 7: Lint**

Run: `cargo clippy --all-targets`
Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add src/lib.rs src/passes/warp.rs shaders/warp.wgsl src/passes/mod.rs
git commit -m "feat: add oscillator-driven warp pass" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Colorize pass

**Files:**
- Create: `shaders/colorize.wgsl`, `src/passes/colorize.rs`
- Modify: `src/passes/mod.rs`
- Test: unit + GPU pipeline tests inside `src/passes/colorize.rs`

**Interfaces:**
- Consumes: `gpu` helpers (Task 1); `params::{ColorizeParams, PALETTE_SIZE, srgb_to_linear, Params}` (Task 2).
- Produces:
  - `#[repr(C)] struct ColorizeUniforms { settings: [f32; 4], palette: [[f32; 4]; 8] }` (144 bytes); `settings` = levels, softness, cycle offset, bypass
  - `pub fn uniforms(p: &ColorizeParams, time: f32) -> ColorizeUniforms` (palette converted to linear, cycle offset wrapped into `[0, levels)`)
  - `pub struct ColorizePass { pub target: RenderTarget, .. }` with `new(device, width, height)` and `render(&self, device, queue, encoder, input: &wgpu::TextureView, uniforms: &ColorizeUniforms)`

- [ ] **Step 1: Register the module**

Replace `src/passes/mod.rs` with:

```rust
//! The render pipeline: source -> warp -> colorize -> feedback -> bloom -> composite.

pub mod colorize;
pub mod warp;
```

- [ ] **Step 2: Write the failing tests**

Create `src/passes/colorize.rs` containing only:

```rust
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
```

- [ ] **Step 3: Run the tests and confirm they fail**

Run: `cargo test --lib passes::colorize::`
Expected: compile errors such as "cannot find type `ColorizeUniforms`".

- [ ] **Step 4: Write the shader `shaders/colorize.wgsl`**

```wgsl
// Colorizing: posterize brightness into levels, map each level to a palette color.

struct Colorize {
    settings: vec4<f32>, // levels, softness, cycle offset (levels), bypass (0 or 1)
    palette: array<vec4<f32>, 8>, // linear RGB
};

@group(0) @binding(0) var<uniform> u: Colorize;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var source: texture_2d<f32>;

// Palette color for a (possibly fractional) level index, wrapping and blending
// between neighbors so palette cycling is smooth.
fn palette_at(c: f32, levels: u32) -> vec3<f32> {
    let i0 = floor(c);
    let a = u32(i0) % levels;
    let b = (a + 1u) % levels;
    return mix(u.palette[a].rgb, u.palette[b].rgb, c - i0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let g = textureSampleLevel(source, samp, in.uv, 0.0).r;
    if u.settings.w > 0.5 {
        return vec4<f32>(g, g, g, 1.0);
    }
    let levels = u.settings.x;
    let softness = u.settings.y;
    let cycle = u.settings.z;
    let x = g * levels;
    let i0 = min(floor(x), levels - 1.0);
    let i1 = min(i0 + 1.0, levels - 1.0);
    let t = select(0.0, smoothstep(1.0 - softness, 1.0, fract(x)), softness > 0.0);
    let n = u32(levels);
    let color = mix(palette_at(i0 + cycle, n), palette_at(i1 + cycle, n), t);
    return vec4<f32>(color, 1.0);
}
```

- [ ] **Step 5: Write the implementation**

Replace `src/passes/colorize.rs` with:

```rust
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
```

- [ ] **Step 6: Run the tests and confirm they pass**

Run: `cargo test --lib passes::colorize::`
Expected: `4 passed`. `pipeline_matches_shader` really builds the pipeline on the GPU; a WGSL error or a uniform-size mismatch makes wgpu panic with `Validation Error`.

- [ ] **Step 7: Lint**

Run: `cargo clippy --all-targets`
Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add src/passes/colorize.rs shaders/colorize.wgsl src/passes/mod.rs
git commit -m "feat: add posterize and palette colorize pass" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Feedback pass

**Files:**
- Create: `shaders/feedback.wgsl`, `src/passes/feedback.rs`
- Modify: `src/passes/mod.rs`
- Test: unit + GPU pipeline tests inside `src/passes/feedback.rs`

**Interfaces:**
- Consumes: `gpu::{self, FullscreenPass, PassDesc, RenderTarget, INTERNAL_FORMAT}` incl. `gpu::clear` (Task 1); `params::FeedbackParams` (Task 2).
- Produces:
  - `#[repr(C)] struct FeedbackUniforms { settings: [f32; 4], offset: [f32; 4] }` (32 bytes); `settings` = amount, zoom, rotation, frame aspect
  - `pub fn uniforms(p: &FeedbackParams, frame_aspect: f32) -> FeedbackUniforms`
  - `pub struct FeedbackPass` with `new(device, width, height)`, `render(&mut self, device, queue, encoder, input: &wgpu::TextureView, uniforms: &FeedbackUniforms)` (swaps ping-pong targets), `output(&self) -> &wgpu::TextureView`, `clear(&self, encoder)`

> **Note:** Trails use `max(current, previous * amount)`, so new content always sits on top and trails fade without washing out to white.

- [ ] **Step 1: Register the module**

Replace `src/passes/mod.rs` with:

```rust
//! The render pipeline: source -> warp -> colorize -> feedback -> bloom -> composite.

pub mod colorize;
pub mod feedback;
pub mod warp;
```

- [ ] **Step 2: Write the failing tests**

Create `src/passes/feedback.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_matches_shader() {
        // Building the pipeline validates the WGSL against the bind group layout.
        let Some((device, _queue)) = crate::gpu::test_device() else {
            return;
        };
        FeedbackPass::new(&device, 64, 64);
    }

    #[test]
    fn uniform_layout_matches_wgsl() {
        assert_eq!(size_of::<FeedbackUniforms>(), 32);
    }
}
```

- [ ] **Step 3: Run the tests and confirm they fail**

Run: `cargo test --lib passes::feedback::`
Expected: compile errors such as "cannot find type `FeedbackUniforms`".

- [ ] **Step 4: Write the shader `shaders/feedback.wgsl`**

```wgsl
// Feedback: keep a decaying, transformed copy of the previous frame under the new one.

struct Feedback {
    settings: vec4<f32>, // amount, zoom, rotation, frame aspect
    offset: vec4<f32>,   // offset.x, offset.y, unused, unused
};

@group(0) @binding(0) var<uniform> u: Feedback;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var current: texture_2d<f32>;
@group(0) @binding(3) var previous: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let cur = textureSampleLevel(current, samp, in.uv, 0.0).rgb;
    let aspect = u.settings.w;
    let p = (in.uv - 0.5) * vec2<f32>(aspect, 1.0);
    let q = rotate2(p, -u.settings.z) / u.settings.y - u.offset.xy;
    let puv = q / vec2<f32>(aspect, 1.0) + 0.5;
    var prev = vec3<f32>(0.0);
    if inside01(puv) {
        prev = textureSampleLevel(previous, samp, puv, 0.0).rgb;
    }
    return vec4<f32>(max(cur, prev * u.settings.x), 1.0);
}
```

- [ ] **Step 5: Write the implementation**

Replace `src/passes/feedback.rs` with:

```rust
//! Feedback pass: phosphor-style trails using two textures that swap roles each frame.

use bytemuck::{Pod, Zeroable};

use crate::gpu::{self, FullscreenPass, INTERNAL_FORMAT, PassDesc, RenderTarget};
use crate::params::FeedbackParams;

/// Matches `struct Feedback` in feedback.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct FeedbackUniforms {
    pub settings: [f32; 4],
    pub offset: [f32; 4],
}

pub fn uniforms(p: &FeedbackParams, frame_aspect: f32) -> FeedbackUniforms {
    FeedbackUniforms {
        settings: [p.amount, p.zoom, p.rotation, frame_aspect],
        offset: [p.offset[0], p.offset[1], 0.0, 0.0],
    }
}

pub struct FeedbackPass {
    pass: FullscreenPass,
    uniform: wgpu::Buffer,
    targets: [RenderTarget; 2],
    /// Index of the target holding the latest frame.
    current: usize,
}

impl FeedbackPass {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let pass = FullscreenPass::new(
            device,
            &PassDesc {
                label: "feedback",
                shader: include_str!("../../shaders/feedback.wgsl"),
                fragment_entry: "fs_main",
                uniform_size: size_of::<FeedbackUniforms>() as u64,
                textures: 2,
                format: INTERNAL_FORMAT,
                blend: None,
            },
        );
        let uniform = pass.create_uniform_buffer(device);
        let targets = [
            RenderTarget::new(device, "feedback a", width, height, INTERNAL_FORMAT),
            RenderTarget::new(device, "feedback b", width, height, INTERNAL_FORMAT),
        ];
        Self {
            pass,
            uniform,
            targets,
            current: 0,
        }
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::TextureView,
        uniforms: &FeedbackUniforms,
    ) {
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniforms));
        let next = 1 - self.current;
        let bind_group = self.pass.bind_group(
            device,
            &self.uniform,
            &[input, &self.targets[self.current].view],
        );
        self.pass
            .draw(encoder, &self.targets[next].view, &bind_group, true);
        self.current = next;
    }

    /// The latest frame, with trails.
    pub fn output(&self) -> &wgpu::TextureView {
        &self.targets[self.current].view
    }

    pub fn clear(&self, encoder: &mut wgpu::CommandEncoder) {
        for target in &self.targets {
            gpu::clear(encoder, &target.view);
        }
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
        FeedbackPass::new(&device, 64, 64);
    }

    #[test]
    fn uniform_layout_matches_wgsl() {
        assert_eq!(size_of::<FeedbackUniforms>(), 32);
    }
}
```

- [ ] **Step 6: Run the tests and confirm they pass**

Run: `cargo test --lib passes::feedback::`
Expected: `2 passed`. `pipeline_matches_shader` really builds the pipeline on the GPU; a WGSL error or a uniform-size mismatch makes wgpu panic with `Validation Error`.

- [ ] **Step 7: Lint**

Run: `cargo clippy --all-targets`
Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add src/passes/feedback.rs shaders/feedback.wgsl src/passes/mod.rs
git commit -m "feat: add ping-pong feedback trails pass" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Bloom pass

**Files:**
- Create: `shaders/bloom.wgsl`, `src/passes/bloom.rs`
- Modify: `src/passes/mod.rs`
- Test: unit + GPU pipeline tests inside `src/passes/bloom.rs`

**Interfaces:**
- Consumes: `gpu` helpers (Task 1).
- Produces:
  - `pub const LEVELS: usize = 5;`
  - `#[repr(C)] struct BloomUniforms { settings: [f32; 4] }` (16 bytes; `settings.x` = threshold)
  - `pub fn level_sizes(width: u32, height: u32) -> [(u32, u32); LEVELS]`
  - `pub struct BloomPass` with `new(device, width, height)`, `render(&self, device, queue, encoder, input: &wgpu::TextureView, threshold: f32)`, `output(&self) -> &wgpu::TextureView` (half resolution)

> **Note:** One WGSL file with three entry points (`fs_prefilter`, `fs_down`, `fs_up`) gives three `FullscreenPass` pipelines. The up pipeline uses additive blending and draws with `clear = false`, so each level adds onto the one above it.

- [ ] **Step 1: Register the module**

Replace `src/passes/mod.rs` with:

```rust
//! The render pipeline: source -> warp -> colorize -> feedback -> bloom -> composite.

pub mod bloom;
pub mod colorize;
pub mod feedback;
pub mod warp;
```

- [ ] **Step 2: Write the failing tests**

Create `src/passes/bloom.rs` containing only:

```rust
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
```

- [ ] **Step 3: Run the tests and confirm they fail**

Run: `cargo test --lib passes::bloom::`
Expected: compile errors such as "cannot find type `BloomPass`".

- [ ] **Step 4: Write the shader `shaders/bloom.wgsl`**

```wgsl
// Bloom: threshold + downsample chain, then upsample back up adding each level.

struct Bloom {
    settings: vec4<f32>, // threshold, unused, unused, unused
};

@group(0) @binding(0) var<uniform> u: Bloom;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var src: texture_2d<f32>;

fn tap(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(src, samp, uv, 0.0).rgb;
}

// 4 bilinear taps one source texel off each diagonal: a smooth 2x downsample.
fn box4(uv: vec2<f32>) -> vec3<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(src));
    return 0.25 * (tap(uv + t * vec2<f32>(-1.0, -1.0)) + tap(uv + t * vec2<f32>(1.0, -1.0))
        + tap(uv + t * vec2<f32>(-1.0, 1.0)) + tap(uv + t * vec2<f32>(1.0, 1.0)));
}

@fragment
fn fs_prefilter(in: VsOut) -> @location(0) vec4<f32> {
    let c = box4(in.uv);
    let brightness = max(c.r, max(c.g, c.b));
    let keep = max(brightness - u.settings.x, 0.0) / max(brightness, 1e-4);
    return vec4<f32>(c * keep, 1.0);
}

@fragment
fn fs_down(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(box4(in.uv), 1.0);
}

// 3x3 tent filter over the smaller level; additively blended onto the larger one.
@fragment
fn fs_up(in: VsOut) -> @location(0) vec4<f32> {
    let t = 1.0 / vec2<f32>(textureDimensions(src));
    var c = tap(in.uv) * 4.0;
    c += (tap(in.uv + vec2<f32>(t.x, 0.0)) + tap(in.uv - vec2<f32>(t.x, 0.0))
        + tap(in.uv + vec2<f32>(0.0, t.y)) + tap(in.uv - vec2<f32>(0.0, t.y))) * 2.0;
    c += tap(in.uv + t) + tap(in.uv - t) + tap(in.uv + vec2<f32>(t.x, -t.y))
        + tap(in.uv + vec2<f32>(-t.x, t.y));
    return vec4<f32>(c / 16.0, 1.0);
}
```

- [ ] **Step 5: Write the implementation**

Replace `src/passes/bloom.rs` with:

```rust
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
```

- [ ] **Step 6: Run the tests and confirm they pass**

Run: `cargo test --lib passes::bloom::`
Expected: `3 passed`. `pipeline_matches_shader` really builds the pipeline on the GPU; a WGSL error or a uniform-size mismatch makes wgpu panic with `Validation Error`.

- [ ] **Step 7: Lint**

Run: `cargo clippy --all-targets`
Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add src/passes/bloom.rs shaders/bloom.wgsl src/passes/mod.rs
git commit -m "feat: add downsample/upsample bloom pass" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Composite pass

**Files:**
- Create: `shaders/composite.wgsl`, `src/passes/composite.rs`
- Modify: `src/passes/mod.rs`
- Test: unit + GPU pipeline tests inside `src/passes/composite.rs`

**Interfaces:**
- Consumes: `gpu` helpers (Task 1); `params::{GlowParams, Params}` (Task 2).
- Produces:
  - `#[repr(C)] struct CompositeUniforms { glow, misc, fit: [f32; 4] }` (48 bytes)
  - `pub fn letterbox(output: (u32, u32), internal: (u32, u32)) -> [f32; 2]`
  - `pub fn uniforms(p: &GlowParams, time: f32, internal: (u32, u32), output: (u32, u32)) -> CompositeUniforms` (chroma converted from pixels to uv)
  - `pub struct CompositePass` with `new(device, output_format: wgpu::TextureFormat)` and `render(&self, device, queue, encoder, image: &wgpu::TextureView, bloom: &wgpu::TextureView, output: &wgpu::TextureView, uniforms: &CompositeUniforms)`

> **Note:** This pass writes to the *output* format (the swapchain's sRGB view, or `Rgba8Unorm` in tests), not `INTERNAL_FORMAT`.

- [ ] **Step 1: Register the module**

Replace `src/passes/mod.rs` with:

```rust
//! The render pipeline: source -> warp -> colorize -> feedback -> bloom -> composite.

pub mod bloom;
pub mod colorize;
pub mod composite;
pub mod feedback;
pub mod warp;
```

- [ ] **Step 2: Write the failing tests**

Create `src/passes/composite.rs` containing only:

```rust
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
        let u = uniforms(&p, 0.0, (1000, 500), (1000, 500));
        assert!((u.glow[3] - 0.002).abs() < 1e-7);
    }
}
```

- [ ] **Step 3: Run the tests and confirm they fail**

Run: `cargo test --lib passes::composite::`
Expected: compile errors such as "cannot find function `letterbox`".

- [ ] **Step 4: Write the shader `shaders/composite.wgsl`**

```wgsl
// Composite: add bloom, then CRT treatment (chromatic bleed, scanlines, noise),
// letterboxed into the output.

struct Composite {
    glow: vec4<f32>, // bloom intensity, scanline strength, scanline count, chroma offset (uv)
    misc: vec4<f32>, // noise, time, internal width, internal height
    fit: vec4<f32>,  // image size as a fraction of the output (x, y), unused, unused
};

@group(0) @binding(0) var<uniform> u: Composite;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var image: texture_2d<f32>;
@group(0) @binding(3) var bloom: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let iuv = (in.uv - 0.5) / u.fit.xy + 0.5;
    if !inside01(iuv) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let off = vec2<f32>(u.glow.w, 0.0);
    var c = vec3<f32>(
        textureSampleLevel(image, samp, iuv + off, 0.0).r,
        textureSampleLevel(image, samp, iuv, 0.0).g,
        textureSampleLevel(image, samp, iuv - off, 0.0).b,
    );
    // The upsample chain sums all bloom levels, so scale it back down.
    c += textureSampleLevel(bloom, samp, iuv, 0.0).rgb * u.glow.x * 0.2;
    let scan = 0.5 + 0.5 * cos(iuv.y * u.glow.z * TAU);
    c *= mix(1.0, scan, u.glow.y);
    let grain = hash12(floor(iuv * u.misc.zw) + fract(u.misc.y) * 113.0) - 0.5;
    c += grain * u.misc.x;
    return vec4<f32>(max(c, vec3<f32>(0.0)), 1.0);
}
```

- [ ] **Step 5: Write the implementation**

Replace `src/passes/composite.rs` with:

```rust
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
        fit: [fit[0], fit[1], 0.0, 0.0],
    }
}

pub struct CompositePass {
    pass: FullscreenPass,
    uniform: wgpu::Buffer,
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
        Self { pass, uniform }
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
        let u = uniforms(&p, 0.0, (1000, 500), (1000, 500));
        assert!((u.glow[3] - 0.002).abs() < 1e-7);
    }
}
```

- [ ] **Step 6: Run the tests and confirm they pass**

Run: `cargo test --lib passes::composite::`
Expected: `5 passed`. `pipeline_matches_shader` really builds the pipeline on the GPU; a WGSL error or a uniform-size mismatch makes wgpu panic with `Validation Error`.

- [ ] **Step 7: Lint**

Run: `cargo clippy --all-targets`
Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add src/passes/composite.rs shaders/composite.wgsl src/passes/mod.rs
git commit -m "feat: add composite pass with bloom, scanlines, chroma bleed and noise" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 9: Renderer and headless smoke test

**Files:**
- Modify: `src/passes/mod.rs` (add `Renderer`)
- Create: `tests/smoke.rs`

**Interfaces:**
- Consumes: every pass from Tasks 4–8; `source::{upload, GrayImage, test_card}` (Task 3); `params::Params` (Task 2); `gpu::{create_instance, request_device, RenderTarget}` (Task 1).
- Produces (used by Task 10):
  - `pub struct Renderer` with
    - `new(device, queue, output_format: wgpu::TextureFormat, size: (u32, u32), image: &GrayImage) -> Self`
    - `set_source(&mut self, device, queue, image: &GrayImage)`
    - `clear_feedback(&self, encoder: &mut wgpu::CommandEncoder)`
    - `render(&mut self, device, queue, encoder, params: &Params, time: f32, output: &wgpu::TextureView, output_size: (u32, u32))`

- [ ] **Step 1: Write the failing smoke test**

Create `tests/smoke.rs`:

```rust
//! Headless GPU smoke test: builds every pass on a real device, renders a few frames
//! offscreen, and reads the result back. wgpu validation errors panic, failing the test.

use rasterwarp::gpu;
use rasterwarp::params::Params;
use rasterwarp::passes::Renderer;
use rasterwarp::source::test_card;

const OUT_W: u32 = 320; // 320 * 4 bytes = 1280, a multiple of 256 as buffer copies require
const OUT_H: u32 = 180;

#[test]
fn renders_frames_offscreen() {
    let backends = wgpu::Backends::all();
    let instance = gpu::create_instance(backends);
    let Ok((_adapter, device, queue)) =
        pollster::block_on(gpu::request_device(&instance, backends, None))
    else {
        eprintln!("skipping smoke test: no GPU adapter available");
        return;
    };

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(&device, "smoke output", OUT_W, OUT_H, format);
    let mut renderer = Renderer::new(&device, &queue, format, (640, 360), &test_card(400, 300));
    let params = Params::default();

    for frame in 0..3 {
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(
            &device,
            &queue,
            &mut encoder,
            &params,
            frame as f32 * 0.1,
            &output.view,
            (OUT_W, OUT_H),
        );
        queue.submit([encoder.finish()]);
    }

    let pixels = read_back(&device, &queue, &output.texture);
    let (rgba, _) = pixels.as_chunks::<4>();
    assert!(
        rgba.iter().any(|px| *px != rgba[0]),
        "output is a single flat color"
    );
}

fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let size = (OUT_W * OUT_H * 4) as u64;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(OUT_W * 4),
                rows_per_image: Some(OUT_H),
            },
        },
        wgpu::Extent3d {
            width: OUT_W,
            height: OUT_H,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    buffer.map_async(wgpu::MapMode::Read, .., |result| {
        result.expect("map readback buffer")
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll device");
    buffer.get_mapped_range(..).expect("mapped range").to_vec()
}
```

- [ ] **Step 2: Run it and confirm it fails**

Run: `cargo test --test smoke`
Expected: compile error "unresolved import `rasterwarp::passes::Renderer`".

- [ ] **Step 3: Write the implementation**

Replace `src/passes/mod.rs` with:

```rust
//! The render pipeline: source -> warp -> colorize -> feedback -> bloom -> composite.

pub mod bloom;
pub mod colorize;
pub mod composite;
pub mod feedback;
pub mod warp;

use crate::params::Params;
use crate::source::{self, GrayImage};

pub struct Renderer {
    size: (u32, u32),
    source: wgpu::TextureView,
    source_aspect: f32,
    warp: warp::WarpPass,
    colorize: colorize::ColorizePass,
    feedback: feedback::FeedbackPass,
    bloom: bloom::BloomPass,
    composite: composite::CompositePass,
}

impl Renderer {
    /// `size` is the fixed internal resolution; `output_format` is the format of the
    /// texture `render` draws into.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        output_format: wgpu::TextureFormat,
        size: (u32, u32),
        image: &GrayImage,
    ) -> Self {
        let (w, h) = size;
        Self {
            size,
            source: source::upload(device, queue, image).create_view(&Default::default()),
            source_aspect: image.aspect(),
            warp: warp::WarpPass::new(device, w, h),
            colorize: colorize::ColorizePass::new(device, w, h),
            feedback: feedback::FeedbackPass::new(device, w, h),
            bloom: bloom::BloomPass::new(device, w, h),
            composite: composite::CompositePass::new(device, output_format),
        }
    }

    pub fn set_source(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, image: &GrayImage) {
        self.source = source::upload(device, queue, image).create_view(&Default::default());
        self.source_aspect = image.aspect();
    }

    pub fn clear_feedback(&self, encoder: &mut wgpu::CommandEncoder) {
        self.feedback.clear(encoder);
    }

    /// Records one frame into `encoder`, ending with a draw into `output`
    /// (`output_size` pixels, letterboxed).
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        params: &Params,
        time: f32,
        output: &wgpu::TextureView,
        output_size: (u32, u32),
    ) {
        let aspect = self.size.0 as f32 / self.size.1 as f32;
        self.warp.render(
            device,
            queue,
            encoder,
            &self.source,
            &warp::uniforms(&params.warp, time, aspect, self.source_aspect),
        );
        self.colorize.render(
            device,
            queue,
            encoder,
            &self.warp.target.view,
            &colorize::uniforms(&params.colorize, time),
        );
        self.feedback.render(
            device,
            queue,
            encoder,
            &self.colorize.target.view,
            &feedback::uniforms(&params.feedback, aspect),
        );
        self.bloom.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            params.glow.bloom_threshold,
        );
        self.composite.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            self.bloom.output(),
            output,
            &composite::uniforms(&params.glow, time, self.size, output_size),
        );
    }
}
```

- [ ] **Step 4: Run the smoke test and confirm it passes**

Run: `cargo test --test smoke`
Expected: `1 passed` (`renders_frames_offscreen`), taking about 1 second.

- [ ] **Step 5: Sanity-check that the smoke test catches layout bugs**

Temporarily change `uniform_size: size_of::<WarpUniforms>() as u64,` in `src/passes/warp.rs` to `uniform_size: 16,` and run `cargo test --test smoke`.
Expected: FAIL with `wgpu error: Validation Error`. **Revert the change** and confirm `git diff src/passes/warp.rs` shows nothing.

- [ ] **Step 6: Run everything**

Run: `cargo test` then `cargo clippy --all-targets`
Expected: all tests pass and there are no warnings.

- [ ] **Step 7: Commit**

```bash
git add src/passes/mod.rs tests/smoke.rs
git commit -m "feat: chain passes in Renderer and add headless GPU smoke test" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Parameter panel, window, and app loop

**Files:**
- Create: `src/ui.rs`, `src/app.rs`, `src/main.rs`
- Modify: `src/lib.rs`
- Test: build, full test suite, and the manual checklist below. The UI and window code has no unit tests; its logic is thin glue around the tested pieces.

**Interfaces:**
- Consumes: `Renderer` (Task 9); `Params`, `ranges`, enums (Task 2); `source::{load_image, test_card, GrayImage}` (Task 3); `gpu::{backends_from_env, create_instance, request_device}` (Task 1).
- Produces:
  - `ui::UiState { paused, frame_ms, source_info, load_error }`, `ui::UiActions { clear_feedback }`, `ui::draw(ui: &mut egui::Ui, params: &mut Params, state: &mut UiState) -> UiActions`
  - `app::App::new(initial_image: Option<PathBuf>)`, `App::finish(self) -> anyhow::Result<()>` (returns the startup error, if any), `impl winit::application::ApplicationHandler for App`
  - `app::INTERNAL_SIZE: (u32, u32) = (1920, 1080)`

> **API notes for egui 0.36 / wgpu 30** (they differ from older tutorials):
> - `egui::Context::run_ui(raw_input, |ui| ...)` gives a root `&mut Ui`; panels are `egui::Panel::left(id).show(ui, ...)`. The old `SidePanel` and `ctx.run` are gone.
> - `textures_delta.set` maps each texture id to a *list* of deltas.
> - `surface.get_current_texture()` returns the `wgpu::CurrentSurfaceTexture` enum, and frames are presented with `queue.present(frame)`.
> - `surface.get_default_config(..)` fills fields that are new in wgpu 30 (e.g. `color_space`).

- [ ] **Step 1: Write the UI panel**

Create `src/ui.rs`:

```rust
//! The egui parameter panel.

use egui::{CollapsingHeader, ComboBox, Slider, Ui};

use crate::params::{Axis, OscInput, Params, Waveform, ranges};

/// UI-only state that isn't a render parameter.
#[derive(Default)]
pub struct UiState {
    pub paused: bool,
    /// Smoothed frame time in milliseconds.
    pub frame_ms: f32,
    pub source_info: String,
    pub load_error: Option<String>,
}

/// One-shot actions requested by the user this frame.
#[derive(Default)]
pub struct UiActions {
    pub clear_feedback: bool,
}

pub fn draw(ui: &mut Ui, params: &mut Params, state: &mut UiState) -> UiActions {
    let mut actions = UiActions::default();
    egui::Panel::left("controls")
        .resizable(true)
        .default_size(320.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Rasterwarp");
                ui.label(format!(
                    "{:.2} ms ({:.0} fps)",
                    state.frame_ms,
                    1000.0 / state.frame_ms.max(0.001)
                ));
                ui.label(&state.source_info);
                if let Some(err) = &state.load_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, err);
                }
                ui.label("Drop a PNG/JPG onto the window to load it.");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut state.paused, "Pause");
                    if ui.button("Reset all").clicked() {
                        *params = Params::default();
                        actions.clear_feedback = true;
                    }
                });
                warp_section(ui, params);
                colorize_section(ui, params);
                feedback_section(ui, params, &mut actions);
                glow_section(ui, params);
            });
        });
    actions
}

fn warp_section(ui: &mut Ui, params: &mut Params) {
    let warp = &mut params.warp;
    CollapsingHeader::new("Deflection")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(Slider::new(&mut warp.zoom, ranges::ZOOM).text("zoom"));
            ui.add(Slider::new(&mut warp.rotation, ranges::ROTATION).text("rotation"));
            ui.add(Slider::new(&mut warp.offset[0], ranges::OFFSET).text("offset x"));
            ui.add(Slider::new(&mut warp.offset[1], ranges::OFFSET).text("offset y"));
            ui.add(Slider::new(&mut warp.drift, ranges::DRIFT).text("analog drift"));
            for (i, osc) in warp.oscillators.iter_mut().enumerate() {
                CollapsingHeader::new(format!("Oscillator {}", i + 1))
                    .default_open(i == 0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ComboBox::from_id_salt(("waveform", i))
                                .selected_text(format!("{:?}", osc.waveform))
                                .show_ui(ui, |ui| {
                                    for w in Waveform::ALL {
                                        ui.selectable_value(&mut osc.waveform, w, format!("{w:?}"));
                                    }
                                });
                            ComboBox::from_id_salt(("input", i))
                                .selected_text(format!("by {:?}", osc.input))
                                .show_ui(ui, |ui| {
                                    for input in OscInput::ALL {
                                        ui.selectable_value(
                                            &mut osc.input,
                                            input,
                                            format!("by {input:?}"),
                                        );
                                    }
                                });
                            ui.selectable_value(&mut osc.target, Axis::X, "→ X");
                            ui.selectable_value(&mut osc.target, Axis::Y, "→ Y");
                        });
                        ui.add(
                            Slider::new(&mut osc.amplitude, ranges::AMPLITUDE).text("amplitude"),
                        );
                        ui.add(
                            Slider::new(&mut osc.frequency, ranges::FREQUENCY).text("frequency"),
                        );
                        ui.add(Slider::new(&mut osc.phase, ranges::PHASE).text("phase"));
                        ui.add(
                            Slider::new(&mut osc.phase_speed, ranges::PHASE_SPEED)
                                .text("phase speed"),
                        );
                        ui.add(Slider::new(&mut osc.lfo_rate, ranges::LFO_RATE).text("LFO rate"));
                        ui.add(
                            Slider::new(&mut osc.lfo_depth, ranges::LFO_DEPTH).text("LFO depth"),
                        );
                    });
            }
        });
}

fn colorize_section(ui: &mut Ui, params: &mut Params) {
    let c = &mut params.colorize;
    CollapsingHeader::new("Colorize")
        .default_open(true)
        .show(ui, |ui| {
            ui.checkbox(&mut c.bypass, "Bypass (grayscale)");
            ui.add(Slider::new(&mut c.levels, ranges::LEVELS).text("levels"));
            ui.add(Slider::new(&mut c.softness, ranges::SOFTNESS).text("softness"));
            ui.add(Slider::new(&mut c.cycle_speed, ranges::CYCLE_SPEED).text("cycle speed"));
            ui.horizontal_wrapped(|ui| {
                for color in &mut c.palette {
                    ui.color_edit_button_rgb(color);
                }
            });
        });
}

fn feedback_section(ui: &mut Ui, params: &mut Params, actions: &mut UiActions) {
    let f = &mut params.feedback;
    CollapsingHeader::new("Feedback")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(Slider::new(&mut f.amount, ranges::FEEDBACK_AMOUNT).text("amount"));
            ui.add(Slider::new(&mut f.zoom, ranges::FEEDBACK_ZOOM).text("zoom"));
            ui.add(Slider::new(&mut f.rotation, ranges::FEEDBACK_ROTATION).text("rotation"));
            ui.add(Slider::new(&mut f.offset[0], ranges::FEEDBACK_OFFSET).text("offset x"));
            ui.add(Slider::new(&mut f.offset[1], ranges::FEEDBACK_OFFSET).text("offset y"));
            if ui.button("Clear trails").clicked() {
                actions.clear_feedback = true;
            }
        });
}

fn glow_section(ui: &mut Ui, params: &mut Params) {
    let g = &mut params.glow;
    CollapsingHeader::new("Glow & CRT")
        .default_open(true)
        .show(ui, |ui| {
            ui.add(Slider::new(&mut g.bloom_intensity, ranges::BLOOM_INTENSITY).text("bloom"));
            ui.add(
                Slider::new(&mut g.bloom_threshold, ranges::BLOOM_THRESHOLD)
                    .text("bloom threshold"),
            );
            ui.add(
                Slider::new(&mut g.scanline_strength, ranges::SCANLINE_STRENGTH).text("scanlines"),
            );
            ui.add(
                Slider::new(&mut g.scanline_count, ranges::SCANLINE_COUNT).text("scanline count"),
            );
            ui.add(Slider::new(&mut g.chroma, ranges::CHROMA).text("chroma bleed"));
            ui.add(Slider::new(&mut g.noise, ranges::NOISE).text("noise"));
        });
}
```

- [ ] **Step 2: Write the app**

Create `src/app.rs`:

```rust
//! The windowed application: owns the GPU context, renderer, parameters and UI,
//! and runs one frame per redraw.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::gpu;
use crate::params::Params;
use crate::passes::Renderer;
use crate::source::{self, GrayImage};
use crate::ui::{self, UiActions, UiState};

/// Fixed internal render resolution, independent of the window size.
pub const INTERNAL_SIZE: (u32, u32) = (1920, 1080);

pub struct App {
    initial_image: Option<PathBuf>,
    state: Option<State>,
    error: Option<anyhow::Error>,
}

impl App {
    pub fn new(initial_image: Option<PathBuf>) -> Self {
        Self {
            initial_image,
            state: None,
            error: None,
        }
    }

    /// The startup error, if the app exited because it could not start.
    pub fn finish(self) -> Result<()> {
        match self.error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        match State::new(event_loop, self.initial_image.as_deref()) {
            Ok(state) => self.state = Some(state),
            Err(err) => {
                self.error = Some(err);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.state else { return };
        let _ = state.egui_state.on_window_event(&state.window, &event);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.resize(size),
            WindowEvent::DroppedFile(path) => state.load_source(&path),
            WindowEvent::RedrawRequested => {
                state.redraw();
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    params: Params,
    ui: UiState,
    time: f32,
    last_frame: Instant,
}

impl State {
    fn new(event_loop: &ActiveEventLoop, initial_image: Option<&Path>) -> Result<Self> {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Rasterwarp")
                        .with_inner_size(LogicalSize::new(1280.0, 720.0)),
                )
                .context("failed to create window")?,
        );
        let backends = gpu::backends_from_env()?;
        let instance = gpu::create_instance(backends);
        let surface = instance
            .create_surface(window.clone())
            .context("failed to create window surface")?;
        let (adapter, device, queue) =
            pollster::block_on(gpu::request_device(&instance, backends, Some(&surface)))?;

        let caps = surface.get_capabilities(&adapter);
        // The swapchain itself is non-sRGB (what egui expects); the composite pass draws
        // through an sRGB view of it so its linear output is encoded correctly.
        let format = caps.formats[0].remove_srgb_suffix();
        let srgb_format = format.add_srgb_suffix();
        // Mailbox shows the true frame rate without tearing; Fifo (vsync) is always available.
        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width, size.height)
            .context("window surface is not supported by the GPU adapter")?;
        config.format = format;
        config.view_formats = vec![srgb_format];
        config.present_mode = present_mode;
        if size.width > 0 && size.height > 0 {
            surface.configure(&device, &config);
        }
        log::info!("surface: {format:?} (composite via {srgb_format:?}), {present_mode:?}");

        let mut ui = UiState {
            frame_ms: 16.7,
            ..Default::default()
        };
        let image = match initial_image {
            Some(path) => match source::load_image(path) {
                Ok(image) => {
                    ui.source_info = describe(&path.display().to_string(), &image);
                    image
                }
                Err(err) => {
                    log::warn!("{err:#}");
                    ui.load_error = Some(format!("{err:#}"));
                    test_card(&mut ui)
                }
            },
            None => test_card(&mut ui),
        };
        let renderer = Renderer::new(&device, &queue, srgb_format, INTERNAL_SIZE, &image);

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui_ctx.viewport_id(),
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());

        Ok(Self {
            window,
            surface,
            config,
            device,
            queue,
            renderer,
            egui_ctx,
            egui_state,
            egui_renderer,
            params: Params::default(),
            ui,
            time: 0.0,
            last_frame: Instant::now(),
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = size.width;
        self.config.height = size.height;
        if size.width > 0 && size.height > 0 {
            self.surface.configure(&self.device, &self.config);
        }
    }

    fn load_source(&mut self, path: &Path) {
        match source::load_image(path) {
            Ok(image) => {
                self.renderer.set_source(&self.device, &self.queue, &image);
                self.ui.source_info = describe(&path.display().to_string(), &image);
                self.ui.load_error = None;
            }
            Err(err) => {
                log::warn!("{err:#}");
                self.ui.load_error = Some(format!("{err:#}"));
            }
        }
    }

    fn redraw(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.ui.frame_ms += (dt * 1000.0 - self.ui.frame_ms) * 0.05;
        if !self.ui.paused {
            self.time += dt;
        }

        if self.config.width == 0 || self.config.height == 0 {
            return; // minimized
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("validation error acquiring the surface texture");
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let srgb_view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.config.format.add_srgb_suffix()),
            ..Default::default()
        });
        let output_size = (self.config.width, self.config.height);

        let raw_input = self.egui_state.take_egui_input(&self.window);
        let mut actions = UiActions::default();
        let egui_output = self.egui_ctx.run_ui(raw_input, |ui| {
            actions = ui::draw(ui, &mut self.params, &mut self.ui)
        });
        self.egui_state
            .handle_platform_output(&self.window, egui_output.platform_output);
        let jobs = self
            .egui_ctx
            .tessellate(egui_output.shapes, egui_output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [output_size.0, output_size.1],
            pixels_per_point: egui_output.pixels_per_point,
        };
        for (id, deltas) in &egui_output.textures_delta.set {
            for delta in deltas {
                self.egui_renderer
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
        }

        let mut encoder = self.device.create_command_encoder(&Default::default());
        if actions.clear_feedback {
            self.renderer.clear_feedback(&mut encoder);
        }
        self.renderer.render(
            &self.device,
            &self.queue,
            &mut encoder,
            &self.params,
            self.time,
            &srgb_view,
            output_size,
        );
        let egui_commands = self.egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &jobs,
            &screen,
        );
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.egui_renderer.render(&mut pass, &jobs, &screen);
        }
        self.queue
            .submit(egui_commands.into_iter().chain([encoder.finish()]));
        self.window.pre_present_notify();
        self.queue.present(frame);
        for id in &egui_output.textures_delta.free {
            self.egui_renderer.free_texture(id);
        }
    }
}

fn test_card(ui: &mut UiState) -> GrayImage {
    let image = source::test_card(1600, 900);
    ui.source_info = describe("built-in test card", &image);
    image
}

fn describe(name: &str, image: &GrayImage) -> String {
    format!("Source: {name} ({}×{})", image.width, image.height)
}
```

- [ ] **Step 3: Write the entry point**

Create `src/main.rs`:

```rust
use std::path::PathBuf;

use anyhow::Result;
use winit::event_loop::{ControlFlow, EventLoop};

use rasterwarp::app::App;

fn main() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn"),
    )
    .init();
    let image = std::env::args_os().nth(1).map(PathBuf::from);
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(image);
    event_loop.run_app(&mut app)?;
    app.finish()
}
```

- [ ] **Step 4: Register the modules**

Replace `src/lib.rs` with:

```rust
//! Rasterwarp: real-time analog video-synthesis animation.

pub mod app;
pub mod gpu;
pub mod params;
pub mod passes;
pub mod source;
pub mod ui;
```

- [ ] **Step 5: Build, test, lint**

Run: `cargo build` then `cargo test` then `cargo clippy --all-targets`
Expected: builds with no warnings, all tests pass, and clippy reports nothing.

- [ ] **Step 6: Manual check (release build)**

Run: `cargo run --release`
Check each item:
- [ ] The log shows `GPU adapter: NVIDIA GeForce RTX 3090 (Vulkan)` and `surface: Bgra8Unorm (composite via Bgra8UnormSrgb), Mailbox`, and has no egui sRGB warning.
- [ ] The window shows the test card warping in a colored palette, with trails, glow, scanlines and grain. The panel is on the left.
- [ ] The frame-time readout stays well under 16.7 ms (the prototype measured about 0.7–0.9 ms on the RTX 3090).
- [ ] Every slider, combo box, checkbox and color button visibly changes the output. Check at least: oscillator amplitude/waveform/input/target, zoom/rotation/offset, drift, levels, softness, cycle speed, bypass, palette color, feedback amount/zoom/rotation/offset, Clear trails, bloom, threshold, scanlines, scanline count, chroma, noise, Pause, Reset all.
- [ ] Dragging a PNG or JPG onto the window replaces the source and updates the "Source:" line.
- [ ] Dragging a non-image file (e.g. `Cargo.toml`) shows a red error in the panel and keeps the current source.
- [ ] `cargo run --release -- missing.png` starts with the test card and shows the load error.
- [ ] Resizing letterboxes the image without stretching. Minimizing and restoring keeps running.
- [ ] Setting `$env:RASTERWARP_BACKEND="dx12"` and running starts on DX12. `$env:RASTERWARP_BACKEND="bogus"` exits with "unknown RASTERWARP_BACKEND 'bogus' ...". Clear the variable afterwards with `Remove-Item Env:RASTERWARP_BACKEND`.

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs src/ui.rs src/app.rs src/main.rs
git commit -m "feat: add egui parameter panel and windowed app loop" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Done criteria (from the spec)

- `cargo build` and `cargo test` pass with no warnings in the project's own code; `cargo clippy --all-targets` is clean.
- The startup log shows the Vulkan backend on the RTX 3090.
- At 1920×1080 internal resolution with all effects on, frame time stays under 16.7 ms in a release build.
- Every parameter in the panel visibly changes the output live.
