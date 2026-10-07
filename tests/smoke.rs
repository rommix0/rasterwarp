//! Headless GPU smoke tests: build every pass on a real device, render a few frames
//! offscreen, and read results back. wgpu validation errors panic, failing the test.

use rasterwarp::blend::FrameParams;
use rasterwarp::gpu;
use rasterwarp::motion::{Mode, Motion};
use rasterwarp::params::Params;
use rasterwarp::passes::Renderer;
use rasterwarp::preview::PreviewView;
use rasterwarp::source::test_card;

const OUT_W: u32 = 320;
const OUT_H: u32 = 180;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let backends = wgpu::Backends::all();
    let instance = gpu::create_instance(backends);
    match pollster::block_on(gpu::request_device(&instance, backends, None)) {
        Ok((_adapter, device, queue)) => Some((device, queue)),
        Err(_) => {
            eprintln!("skipping smoke test: no GPU adapter available");
            None
        }
    }
}

#[test]
fn renders_frames_offscreen() {
    let Some((device, queue)) = device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(&device, "smoke output", OUT_W, OUT_H, format);
    let mut renderer = Renderer::new(&device, &queue, format, (640, 360), &test_card(400, 300));
    let frame_params = FrameParams::at_rest(&Params::default());

    for frame in 0..3 {
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(
            &device,
            &queue,
            &mut encoder,
            &frame_params,
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

#[test]
fn renders_the_preview_offscreen() {
    let Some((device, queue)) = device() else {
        return;
    };
    let mut egui_renderer = egui_wgpu::Renderer::new(
        &device,
        wgpu::TextureFormat::Bgra8Unorm,
        egui_wgpu::RendererOptions::default(),
    );
    let mut preview = PreviewView::new(
        &device,
        &queue,
        &mut egui_renderer,
        (1920, 1080),
        &test_card(400, 300),
    );
    assert_eq!(preview.size(), (480, 270));
    let mut motion = Motion::new(Params::default());
    motion.set_mode(Mode::Transition);
    let shown = motion.preview().expect("Transition mode has a preview");
    for frame in 0..3 {
        let mut encoder = device.create_command_encoder(&Default::default());
        preview.render(&device, &queue, &mut encoder, &shown, frame as f32 * 0.1);
        queue.submit([encoder.finish()]);
    }
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll device");
}

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
/// that buffer copies require.
fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let row = width * 4;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(padded * height),
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
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    buffer.map_async(wgpu::MapMode::Read, .., |result| {
        result.expect("map readback buffer")
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll device");
    let mapped = buffer.get_mapped_range(..).expect("mapped range");
    mapped
        .chunks(padded as usize)
        .flat_map(|r| &r[..row as usize])
        .copied()
        .collect()
}
