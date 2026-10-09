//! Frames pass: draws a video or camera input's picture for this frame from a ring of
//! its frames on the GPU (a texture array), mixing neighbouring frames and, for
//! slit-scan, giving each pixel its own moment. The rest of the pipeline samples its
//! output like an image.

use bytemuck::{Pod, Zeroable};

use crate::gpu::{FullscreenPass, PassDesc, RenderTarget};
use crate::params::{Between, Slit};
use crate::source::GrayImage;
use crate::video::Pixels;
use crate::video::playhead::{Sample, layer};

/// Matches `struct Frames` in frames.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct FramesUniforms {
    pub playhead: [f32; 4],
    pub ring: [u32; 4],
}

/// The texture format frames of `pixels` are stored and drawn in. Color frames are sRGB,
/// so frames blend in linear light.
pub fn texture_format(pixels: Pixels) -> wgpu::TextureFormat {
    match pixels {
        Pixels::Luma => wgpu::TextureFormat::R8Unorm,
        Pixels::Rgba => wgpu::TextureFormat::Rgba8UnormSrgb,
    }
}

/// The uniforms for `sample` drawn from a ring of `layers` layers.
pub fn uniforms(sample: &Sample, layers: u32) -> FramesUniforms {
    let slit = match sample.slit {
        Slit::Off => 0,
        Slit::Rows => 1,
        Slit::Columns => 2,
        Slit::Map => 3,
    };
    let flags = u32::from(sample.flip) | u32::from(sample.between == Between::Blend) << 1;
    FramesUniforms {
        playhead: [
            (sample.base - sample.first as f64) as f32,
            sample.reach as f32,
            (sample.last - sample.first) as f32,
            0.0,
        ],
        ring: [layer(sample.first, layers), layers, slit, flags],
    }
}

pub struct FramesPass {
    pass: FullscreenPass,
    uniform: wgpu::Buffer,
    pixels: Pixels,
    size: (u32, u32),
    /// The most layers the ring may grow to.
    max_layers: u32,
    ring: wgpu::Texture,
    ring_view: wgpu::TextureView,
    /// The virtual index each layer holds.
    held: Vec<Option<i64>>,
    map: wgpu::TextureView,
    bind_group: Option<wgpu::BindGroup>,
    pub target: RenderTarget,
}

fn ring_texture(
    device: &wgpu::Device,
    pixels: Pixels,
    size: (u32, u32),
    layers: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("frame ring"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: texture_format(pixels),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    (texture, view)
}

/// A 1×1 black map, for when there's no map image.
fn blank_map(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let image = GrayImage {
        width: 1,
        height: 1,
        pixels: vec![0],
    };
    crate::source::upload(device, queue, &image).create_view(&Default::default())
}

impl FramesPass {
    /// A pass for frames of `size` pixels whose ring may grow to `max_layers` layers.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pixels: Pixels,
        size: (u32, u32),
        max_layers: u32,
    ) -> Self {
        let format = texture_format(pixels);
        let pass = FullscreenPass::with_dimensions(
            device,
            &PassDesc {
                label: "frames",
                shader: include_str!("../../shaders/frames.wgsl"),
                fragment_entry: "fs_main",
                uniform_size: size_of::<FramesUniforms>() as u64,
                textures: 2,
                format,
                blend: None,
            },
            &[
                wgpu::TextureViewDimension::D2Array,
                wgpu::TextureViewDimension::D2,
            ],
        );
        let uniform = pass.create_uniform_buffer(device);
        let (ring, ring_view) = ring_texture(device, pixels, size, 2);
        Self {
            pass,
            uniform,
            pixels,
            size,
            max_layers: max_layers.max(2),
            ring,
            ring_view,
            held: vec![None; 2],
            map: blank_map(device, queue),
            bind_group: None,
            target: RenderTarget::new(device, "frames", size.0, size.1, format),
        }
    }

    pub fn pixels(&self) -> Pixels {
        self.pixels
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Layers in the ring now.
    pub fn layers(&self) -> u32 {
        self.held.len() as u32
    }

    /// Uses `map` (or none) for slit-scan's Map.
    pub fn set_map(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, map: Option<&GrayImage>) {
        self.map = match map {
            Some(image) => {
                crate::source::upload(device, queue, image).create_view(&Default::default())
            }
            None => blank_map(device, queue),
        };
        self.bind_group = None;
    }

    /// The virtual indices in `sample`'s window that aren't on the GPU yet, oldest first.
    /// Grows the ring first if the window needs more layers (which empties it).
    pub fn missing(&mut self, device: &wgpu::Device, sample: &Sample) -> Vec<i64> {
        let needed = (sample.last - sample.first + 1).max(1) as u32;
        if needed > self.layers() {
            // Doubling, so a deepening slit-scan doesn't rebuild it every frame.
            let layers = needed.max(self.layers() * 2).min(self.max_layers);
            (self.ring, self.ring_view) = ring_texture(device, self.pixels, self.size, layers);
            self.held = vec![None; layers as usize];
            self.bind_group = None;
        }
        let layers = self.layers();
        sample
            .window()
            .filter(|&k| self.held[layer(k, layers) as usize] != Some(k))
            .collect()
    }

    /// Puts frame `k` (tightly packed rows of this pass's size) in its layer.
    pub fn upload(&mut self, queue: &wgpu::Queue, k: i64, frame: &[u8]) {
        let slot = layer(k, self.layers());
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.ring,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: slot,
                },
                aspect: wgpu::TextureAspect::All,
            },
            frame,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.size.0 * self.pixels.bytes()),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: self.size.0,
                height: self.size.1,
                depth_or_array_layers: 1,
            },
        );
        self.held[slot as usize] = Some(k);
    }

    /// Draws `sample`'s picture into [`FramesPass::target`]. Its window must fit in the
    /// ring (see [`FramesPass::missing`]).
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        sample: &Sample,
    ) {
        queue.write_buffer(
            &self.uniform,
            0,
            bytemuck::bytes_of(&uniforms(sample, self.layers())),
        );
        let bind_group = self.bind_group.get_or_insert_with(|| {
            self.pass
                .bind_group(device, &self.uniform, &[&self.ring_view, &self.map])
        });
        self.pass.draw(encoder, &self.target.view, bind_group, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Sample {
        Sample {
            base: 102.25,
            reach: 7.5,
            first: 94,
            last: 103,
            between: Between::Blend,
            slit: Slit::Columns,
            flip: true,
        }
    }

    #[test]
    fn uniforms_count_from_the_window_start() {
        let u = uniforms(&sample(), 16);
        assert_eq!(u.playhead, [8.25, 7.5, 9.0, 0.0]);
        assert_eq!(u.ring, [94 % 16, 16, 2, 0b11]);
        let nearest = Sample {
            between: Between::Nearest,
            flip: false,
            slit: Slit::Off,
            ..sample()
        };
        assert_eq!(uniforms(&nearest, 16).ring[2..], [0, 0]);
    }
}
