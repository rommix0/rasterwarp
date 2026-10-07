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
