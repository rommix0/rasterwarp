//! Headless GPU smoke tests: build every pass on a real device, render a few frames
//! offscreen, and read results back. wgpu validation errors panic, failing the test.

use rasterwarp::blend::FrameParams;
use rasterwarp::capture::CaptureMode;
use rasterwarp::capture::encode::VideoFormat;
use rasterwarp::capture::recorder::{RecordSettings, Recorder};
use rasterwarp::gpu;
use rasterwarp::motion::{Mode, Motion};
use rasterwarp::params::Params;
use rasterwarp::passes::Renderer;
use rasterwarp::preview::PreviewView;
use rasterwarp::rate::FrameRate;
use rasterwarp::source::{GrayImage, test_card};

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
fn renders_after_a_canvas_resize() {
    let Some((device, queue)) = device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(&device, "smoke output", OUT_W, OUT_H, format);
    let mut renderer = Renderer::new(&device, &queue, format, (640, 360), &test_card(400, 300));
    renderer.resize(&device, (256, 192));
    assert_eq!(renderer.size(), (256, 192));
    let frame_params = FrameParams::at_rest(&Params::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(
        &device,
        &queue,
        &mut encoder,
        &frame_params,
        0.0,
        &output.view,
        (OUT_W, OUT_H),
    );
    queue.submit([encoder.finish()]);
    let pixels = read_back(&device, &queue, &output.texture);
    let (rgba, _) = pixels.as_chunks::<4>();
    assert!(
        rgba.iter().any(|px| *px != rgba[0]),
        "output is a single flat color"
    );
}

#[test]
fn compositing_again_shows_the_same_canvas_frame() {
    let Some((device, queue)) = device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let first = gpu::RenderTarget::new(&device, "first", OUT_W, OUT_H, format);
    let second = gpu::RenderTarget::new(&device, "second", OUT_W, OUT_H, format);
    let mut renderer = Renderer::new(&device, &queue, format, (640, 360), &test_card(400, 300));
    let frame_params = FrameParams::at_rest(&Params::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render_canvas(&device, &queue, &mut encoder, &frame_params, 0.5);
    renderer.composite(
        &device,
        &queue,
        &mut encoder,
        &frame_params,
        0.5,
        &first.view,
        (OUT_W, OUT_H),
    );
    queue.submit([encoder.finish()]);
    // A later screen refresh with no new canvas frame due composites again.
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.composite(
        &device,
        &queue,
        &mut encoder,
        &frame_params,
        0.5,
        &second.view,
        (OUT_W, OUT_H),
    );
    queue.submit([encoder.finish()]);
    let a = read_back(&device, &queue, &first.texture);
    let b = read_back(&device, &queue, &second.texture);
    let (rgba, _) = a.as_chunks::<4>();
    assert!(
        rgba.iter().any(|px| *px != rgba[0]),
        "output is a single flat color"
    );
    assert!(a == b, "compositing again changed the picture");
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

#[test]
fn records_every_frame_offline() {
    let Some((device, queue)) = device() else {
        return;
    };
    let folder = std::env::temp_dir().join(format!("rasterwarp-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let settings = RecordSettings {
        format: VideoFormat::Ffv1,
        mode: CaptureMode::Offline,
        folder: folder.clone(),
        stop_after: 0.5,
    };
    let canvas = (256, 144);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(&device, "smoke output", OUT_W, OUT_H, format);
    let mut renderer = Renderer::new(&device, &queue, format, canvas, &test_card(400, 300));
    let frame_params = FrameParams::at_rest(&Params::default());
    let rate = FrameRate::ntsc(24);
    let mut recorder = Recorder::start(&device, &settings, canvas, rate).expect("start recording");
    let mut time = 0.0;
    while !recorder.finished_recording() {
        recorder.pump(&device).expect("encoder running");
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(
            &device,
            &queue,
            &mut encoder,
            &frame_params,
            time as f32,
            &output.view,
            (OUT_W, OUT_H),
        );
        recorder
            .capture(&device, &mut encoder, |encoder, view| {
                renderer.composite_capture(
                    &device,
                    &queue,
                    encoder,
                    &frame_params,
                    time as f32,
                    view,
                )
            })
            .expect("capture");
        queue.submit([encoder.finish()]);
        recorder.after_submit();
        time += rate.period();
    }
    let status = recorder.finish(&device).expect("finish recording");
    assert_eq!(status.frames, 12, "0.5 s at 23.976 fps");
    assert_eq!(status.dropped, 0);
    assert!(status.path.starts_with(&folder));
    assert!(std::fs::metadata(&status.path).expect("file written").len() > 0);
}

#[test]
fn offline_capture_waits_for_the_gpu_instead_of_dropping() {
    let Some((device, queue)) = device() else {
        return;
    };
    let folder = std::env::temp_dir().join(format!("rasterwarp-wait-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let settings = RecordSettings {
        format: VideoFormat::Ffv1,
        mode: CaptureMode::Offline,
        folder: folder.clone(),
        stop_after: 0.0,
    };
    let canvas = (256, 144);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(&device, "smoke output", OUT_W, OUT_H, format);
    let mut renderer = Renderer::new(&device, &queue, format, canvas, &test_card(400, 300));
    let frame_params = FrameParams::at_rest(&Params::default());
    let rate = FrameRate::default();
    let mut recorder = Recorder::start(&device, &settings, canvas, rate).expect("start recording");
    // No pump: the ring fills after 3 frames, so capture must wait for the GPU.
    for frame in 0..8 {
        let time = rate.seconds(frame);
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(
            &device,
            &queue,
            &mut encoder,
            &frame_params,
            time as f32,
            &output.view,
            (OUT_W, OUT_H),
        );
        recorder
            .capture(&device, &mut encoder, |encoder, view| {
                renderer.composite_capture(
                    &device,
                    &queue,
                    encoder,
                    &frame_params,
                    time as f32,
                    view,
                )
            })
            .expect("capture");
        queue.submit([encoder.finish()]);
        recorder.after_submit();
    }
    let status = recorder.finish(&device).expect("finish recording");
    assert_eq!(status.frames, 8);
    assert_eq!(status.dropped, 0);
    assert!(status.path.starts_with(&folder));
    assert!(std::fs::metadata(&status.path).expect("file written").len() > 0);
}

#[test]
fn real_time_capture_waits_for_the_gpu_when_the_ring_is_full() {
    let Some((device, queue)) = device() else {
        return;
    };
    let folder = std::env::temp_dir().join(format!("rasterwarp-ring-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let settings = RecordSettings {
        format: VideoFormat::Ffv1,
        mode: CaptureMode::RealTime,
        folder,
        stop_after: 0.0,
    };
    let canvas = (256, 144);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(&device, "smoke output", OUT_W, OUT_H, format);
    let mut renderer = Renderer::new(&device, &queue, format, canvas, &test_card(400, 300));
    let frame_params = FrameParams::at_rest(&Params::default());
    let rate = FrameRate::default();
    let mut recorder = Recorder::start(&device, &settings, canvas, rate).expect("start recording");
    // Five frames back to back with no pump, as when the app catches up: the fourth finds
    // the three staging buffers busy and waits for the GPU instead of dropping a frame.
    // The three frames that come back fit in the encoder's queue of four.
    for frame in 0..5 {
        let time = rate.seconds(frame);
        let mut encoder = device.create_command_encoder(&Default::default());
        renderer.render(
            &device,
            &queue,
            &mut encoder,
            &frame_params,
            time as f32,
            &output.view,
            (OUT_W, OUT_H),
        );
        recorder
            .capture(&device, &mut encoder, |encoder, view| {
                renderer.composite_capture(
                    &device,
                    &queue,
                    encoder,
                    &frame_params,
                    time as f32,
                    view,
                )
            })
            .expect("capture");
        queue.submit([encoder.finish()]);
        recorder.after_submit();
    }
    let status = recorder.finish(&device).expect("finish recording");
    assert_eq!((status.frames, status.dropped), (5, 0));
}

/// Parameters that show the source as it is: no deflection, trails or CRT effects.
fn flat_params() -> Params {
    let mut p = Params::default();
    for osc in &mut p.warp.oscillators {
        osc.amplitude = 0.0;
    }
    p.warp.drift = 0.0;
    p.colorize.bandwidth = 0.0;
    p.feedback.amount = 0.0;
    p.glow.bloom_intensity = 0.0;
    p.glow.scanline_strength = 0.0;
    p.glow.chroma = 0.0;
    p.glow.noise = 0.0;
    p
}

/// A left-to-right gradient from black to white.
fn gradient(width: u32, height: u32) -> GrayImage {
    let row: Vec<u8> = (0..width)
        .map(|x| (x as f32 / (width - 1) as f32 * 255.0).round() as u8)
        .collect();
    GrayImage {
        width,
        height,
        pixels: row.repeat(height as usize),
    }
}

/// Renders one canvas frame of `image` with `params` at `size`, and reads back the
/// composite (RGBA, no letterboxing).
fn render_still(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    params: &Params,
    image: &GrayImage,
    size: (u32, u32),
) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(device, "still", size.0, size.1, format);
    let mut renderer = Renderer::new(device, queue, format, size, image);
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(
        device,
        queue,
        &mut encoder,
        &FrameParams::at_rest(params),
        0.0,
        &output.view,
        size,
    );
    queue.submit([encoder.finish()]);
    read_back(device, queue, &output.texture)
}

/// The first x in row `y` whose red channel is above half brightness.
fn first_bright(pixels: &[u8], width: u32, y: u32) -> Option<u32> {
    (0..width).find(|&x| pixels[((y * width + x) * 4) as usize] > 128)
}

#[test]
fn colorizer_levels_start_at_their_thresholds() {
    let Some((device, queue)) = device() else {
        return;
    };
    let size = (256, 64);
    let mut params = flat_params();
    params.colorize.levels = 2;
    params.colorize.softness = 0.0;
    params.colorize.palette[0] = [0.0, 0.0, 0.0];
    params.colorize.palette[1] = [1.0, 1.0, 1.0];
    let image = gradient(size.0, size.1);
    for threshold in [0.25, 0.75] {
        params.colorize.thresholds[0] = threshold;
        let pixels = render_still(&device, &queue, &params, &image, size);
        let edge = first_bright(&pixels, size.0, size.1 / 2).expect("a white level") as f32;
        let expected = threshold * (size.0 - 1) as f32;
        assert!(
            (edge - expected).abs() <= 2.0,
            "threshold {threshold}: white starts at x = {edge}, expected about {expected}"
        );
    }
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
