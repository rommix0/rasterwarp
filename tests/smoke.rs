//! Headless GPU smoke tests: build every pass on a real device, render a few frames
//! offscreen, and read results back. wgpu validation errors panic, failing the test.

use rasterwarp::blend::FrameParams;
use rasterwarp::capture::CaptureMode;
use rasterwarp::capture::encode::VideoFormat;
use rasterwarp::capture::recorder::{RecordSettings, Recorder};
use rasterwarp::capture::still;
use rasterwarp::gpu;
use rasterwarp::motion::{Mode, Motion};
use rasterwarp::params::{Between, Params, Slit};
use rasterwarp::passes::Renderer;
use rasterwarp::passes::composite::Area;
use rasterwarp::passes::frames::FramesPass;
use rasterwarp::preview::PreviewView;
use rasterwarp::rate::FrameRate;
use rasterwarp::source::{ColorImage, GrayImage, test_card};
use rasterwarp::video::Pixels;
use rasterwarp::video::playhead::Sample;

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
        Area::whole((OUT_W, OUT_H)),
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
        Area::whole((OUT_W, OUT_H)),
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
fn composites_only_into_the_canvas_area() {
    let Some((device, queue)) = device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(&device, "smoke output", OUT_W, OUT_H, format);
    let mut renderer = Renderer::new(&device, &queue, format, (640, 360), &test_card(400, 300));
    let frame_params = FrameParams::at_rest(&Params::default());
    // The right half of the window, as if a panel covered the left half.
    let area = Area {
        x: OUT_W / 2,
        y: 0,
        width: OUT_W / 2,
        height: OUT_H,
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render_canvas(&device, &queue, &mut encoder, &frame_params, 0.5);
    renderer.composite(
        &device,
        &queue,
        &mut encoder,
        &frame_params,
        0.5,
        &output.view,
        area,
    );
    queue.submit([encoder.finish()]);
    let pixels = read_back(&device, &queue, &output.texture);
    let (rgba, _) = pixels.as_chunks::<4>();
    let black = [0, 0, 0, 255];
    let (left, right): (Vec<_>, Vec<_>) = rgba
        .iter()
        .enumerate()
        .partition(|(i, _)| (*i as u32 % OUT_W) < OUT_W / 2);
    assert!(
        left.iter().all(|(_, px)| **px == black),
        "the picture spilled outside its area"
    );
    assert!(
        right.iter().any(|(_, px)| **px != black),
        "the area shows no picture"
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

#[test]
fn grabs_a_still_of_the_whole_canvas() {
    let Some((device, queue)) = device() else {
        return;
    };
    let canvas = (256, 144);
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut renderer = Renderer::new(&device, &queue, format, canvas, &test_card(400, 300));
    let frame_params = FrameParams::at_rest(&Params::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render_canvas(&device, &queue, &mut encoder, &frame_params, 0.5);
    queue.submit([encoder.finish()]);
    let rgba = still::grab(&device, &queue, renderer.size(), |encoder, view| {
        renderer.composite_capture(&device, &queue, encoder, &frame_params, 0.5, view)
    })
    .expect("grab a still");
    assert_eq!(rgba.len(), (canvas.0 * canvas.1 * 4) as usize);
    let (pixels, _) = rgba.as_chunks::<4>();
    assert!(
        pixels.iter().any(|px| *px != pixels[0]),
        "the still is a single flat color"
    );
    assert!(pixels.iter().all(|px| px[3] == 255), "the canvas is opaque");
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

/// Setup result for keying tests.
struct KeyingSetup {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    output: gpu::RenderTarget,
    params: Params,
    size: (u32, u32),
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
    render_frame(device, queue, &FrameParams::at_rest(params), image, size)
}

/// Like `render_still`, for a frame with its own seed or other frame values.
fn render_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &FrameParams,
    image: &GrayImage,
    size: (u32, u32),
) -> Vec<u8> {
    render_scaled(device, queue, frame, image, size, 1.0)
}

/// Like `render_frame`, for a renderer with `pixel_scale` of its pixels per canvas pixel.
fn render_scaled(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &FrameParams,
    image: &GrayImage,
    size: (u32, u32),
    pixel_scale: f32,
) -> Vec<u8> {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(device, "still", size.0, size.1, format);
    let mut renderer = Renderer::new(device, queue, format, size, image);
    renderer.set_pixel_scale(pixel_scale);
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(device, queue, &mut encoder, frame, 0.0, &output.view, size);
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

#[test]
fn edges_smear_only_to_their_right() {
    let Some((device, queue)) = device() else {
        return;
    };
    let size = (256, 64);
    // Black on the left half, white on the right.
    let image = GrayImage {
        width: size.0,
        height: size.1,
        pixels: (0..size.0 * size.1)
            .map(|i| if i % size.0 < size.0 / 2 { 0 } else { 255 })
            .collect(),
    };
    let mut params = flat_params();
    params.colorize.bypass = true; // show the smeared brightness itself
    let row = |pixels: &[u8]| -> Vec<u8> {
        let y = size.1 / 2;
        (0..size.0)
            .map(|x| pixels[((y * size.0 + x) * 4) as usize])
            .collect()
    };
    let intermediate = |r: &[u8]| -> Vec<u32> {
        (0..size.0)
            .filter(|&x| (1..255).contains(&r[x as usize]))
            .collect()
    };
    let edge = size.0 / 2;
    let sharp = row(&render_still(&device, &queue, &params, &image, size));
    assert!(intermediate(&sharp).is_empty(), "no smear at bandwidth 0");
    params.colorize.bandwidth = 3.0;
    let smeared = row(&render_still(&device, &queue, &params, &image, size));
    assert!(
        smeared[..edge as usize].iter().all(|&v| v == 0),
        "the left side is unchanged"
    );
    let fringe = intermediate(&smeared);
    assert!(
        fringe.len() >= 3,
        "the edge smears over a few pixels: {fringe:?}"
    );
    assert!(
        fringe.iter().all(|&x| x >= edge),
        "smear only to the right: {fringe:?}"
    );
}

#[test]
fn line_jitter_shifts_each_row_by_at_most_the_amount() {
    let Some((device, queue)) = device() else {
        return;
    };
    let size = (256, 64);
    // Vertical stripes 8 pixels wide; the first white stripe starts at x = 8.
    let image = GrayImage {
        width: size.0,
        height: size.1,
        pixels: (0..size.0 * size.1)
            .map(|i| if i % size.0 % 16 < 8 { 0 } else { 255 })
            .collect(),
    };
    let mut params = flat_params();
    params.colorize.bypass = true;
    let starts = |pixels: &[u8]| -> Vec<u32> {
        (0..size.1)
            .map(|y| first_bright(pixels, size.0, y).expect("a white stripe"))
            .collect()
    };
    let still = starts(&render_still(&device, &queue, &params, &image, size));
    assert!(still.iter().all(|&x| x == 8), "no jitter: {still:?}");

    params.warp.line_jitter = 4.0;
    let mut frame = FrameParams::at_rest(&params);
    frame.warp.seed = 5;
    let a = render_frame(&device, &queue, &frame, &image, size);
    let again = render_frame(&device, &queue, &frame, &image, size);
    assert!(a == again, "the same frame jitters the same way");
    let jittered = starts(&a);
    assert!(
        jittered.iter().all(|&x| x.abs_diff(8) <= 5),
        "rows move by at most the jitter (plus a pixel of resampling): {jittered:?}"
    );
    let distinct: std::collections::BTreeSet<_> = jittered.iter().collect();
    assert!(distinct.len() >= 4, "rows move differently: {jittered:?}");
    frame.warp.seed = 6;
    let next = render_frame(&device, &queue, &frame, &image, size);
    assert!(
        starts(&next) != jittered,
        "the next frame jitters differently"
    );
}

/// Rows of column `x` that are brighter than both neighbours: the centres of scan lines.
fn line_centres(pixels: &[u8], size: (u32, u32), x: u32) -> Vec<u32> {
    let at = |y: u32| pixels[((y * size.0 + x) * 4) as usize];
    (1..size.1 - 1)
        .filter(|&y| at(y) > 100 && at(y) > at(y - 1) && at(y) >= at(y + 1))
        .collect()
}

#[test]
fn raster_mode_draws_separate_scan_lines() {
    let Some((device, queue)) = device() else {
        return;
    };
    let size = (256, 400);
    let white = GrayImage {
        width: size.0,
        height: size.1,
        pixels: vec![255; (size.0 * size.1) as usize],
    };
    let mut params = flat_params();
    params.colorize.bypass = true;
    params.raster.enabled = true;
    params.raster.lines = 100;
    let pixels = render_still(&device, &queue, &params, &white, size);
    let centres = line_centres(&pixels, size, size.0 / 2);
    assert!(
        (98..=100).contains(&centres.len()),
        "{} lines: {centres:?}",
        centres.len()
    );
    assert!(
        centres.windows(2).all(|w| w[1] - w[0] == 4),
        "one line every 4 rows: {centres:?}"
    );
    let at = |y: u32| pixels[((y * size.0 + size.0 / 2) * 4) as usize];
    assert!(at(centres[10] + 2) < 40, "dark between lines");

    // Zooming in stretches the raster: the gaps widen.
    params.warp.zoom = 2.0;
    let pixels = render_still(&device, &queue, &params, &white, size);
    let centres = line_centres(&pixels, size, size.0 / 2);
    assert!(
        centres.windows(2).all(|w| w[1] - w[0] == 8),
        "one line every 8 rows: {centres:?}"
    );
}

#[test]
fn small_raster_renderer_matches_the_canvas_brightness() {
    let Some((device, queue)) = device() else {
        return;
    };
    let white = GrayImage {
        width: 160,
        height: 90,
        pixels: vec![255; 160 * 90],
    };
    let mut params = flat_params();
    params.colorize.bypass = true; // compare the raster brightness itself
    params.raster.enabled = true; // default lines and beam width
    let frame = FrameParams::at_rest(&params);
    // Mean red value over the middle half of the picture, 0-1.
    let mean = |pixels: &[u8], size: (u32, u32)| -> f32 {
        let (w, h) = size;
        let mut sum = 0u64;
        let mut count = 0u64;
        for y in h / 4..h * 3 / 4 {
            for x in w / 4..w * 3 / 4 {
                sum += u64::from(pixels[((y * w + x) * 4) as usize]);
                count += 1;
            }
        }
        sum as f32 / count as f32 / 255.0
    };
    let canvas = (1920, 1080);
    let preview = (480, 270);
    let full = mean(
        &render_scaled(&device, &queue, &frame, &white, canvas, 1.0),
        canvas,
    );
    let small = mean(
        &render_scaled(&device, &queue, &frame, &white, preview, 0.25),
        preview,
    );
    assert!(
        (small / full - 1.0).abs() < 0.1,
        "canvas mean {full:.3}, quarter-size preview mean {small:.3}"
    );
}

#[test]
fn sub_pixel_beams_do_not_drop_lines() {
    let Some((device, queue)) = device() else {
        return;
    };
    let white = GrayImage {
        width: 160,
        height: 90,
        pixels: vec![255; 160 * 90],
    };
    let mut params = flat_params();
    params.colorize.bypass = true;
    params.raster.enabled = true;
    params.raster.lines = 100; // 2.7 preview rows apart; the beam is 0.3 preview pixels
    let size = (480, 270);
    let frame = FrameParams::at_rest(&params);
    let pixels = render_scaled(&device, &queue, &frame, &white, size, 0.25);
    let lit: Vec<bool> = (0..size.1)
        .map(|y| pixels[((y * size.0 + size.0 / 2) * 4) as usize] > 20)
        .collect();
    let mut gap = 0;
    for &on in &lit[20..250] {
        gap = if on { 0 } else { gap + 1 };
        assert!(gap <= 2, "a line dropped out: lit rows {lit:?}");
    }
}

/// Sets up a keying test: striped grayscale image, red background, keying params.
fn setup_keying_test() -> Option<KeyingSetup> {
    let (device, queue) = device()?;
    let size = (256, 64);
    // Black on the left half, white on the right.
    let image = GrayImage {
        width: size.0,
        height: size.1,
        pixels: (0..size.0 * size.1)
            .map(|i| if i % size.0 < size.0 / 2 { 0 } else { 255 })
            .collect(),
    };
    let red = ColorImage {
        width: 2,
        height: 2,
        pixels: [255, 0, 0, 255].repeat(4),
    };
    let mut params = flat_params();
    params.colorize.levels = 2;
    params.colorize.palette[0] = [0.0, 0.0, 1.0];
    params.colorize.palette[1] = [1.0, 1.0, 1.0];
    params.key.enabled = true;
    params.key.levels = 0b01; // the dark level is see-through

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let output = gpu::RenderTarget::new(&device, "keyed", size.0, size.1, format);
    let mut renderer = Renderer::new(&device, &queue, format, size, &image);
    renderer.set_background(&device, &queue, Some(&red));

    Some(KeyingSetup {
        device,
        queue,
        renderer,
        output,
        params,
        size,
    })
}

#[test]
fn keyed_levels_show_the_background() {
    let Some(KeyingSetup {
        device,
        queue,
        mut renderer,
        output,
        params,
        size,
    }) = setup_keying_test()
    else {
        return;
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(
        &device,
        &queue,
        &mut encoder,
        &FrameParams::at_rest(&params),
        0.0,
        &output.view,
        size,
    );
    queue.submit([encoder.finish()]);
    let pixels = read_back(&device, &queue, &output.texture);
    let px = |x: u32| {
        let i = ((size.1 / 2 * size.0 + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    };
    assert_eq!(
        px(40),
        [255, 0, 0],
        "the keyed dark level shows the red background"
    );
    assert_eq!(px(200), [255, 255, 255], "the white level stays");
}

#[test]
fn keyed_levels_show_the_background_after_clearing_trails() {
    let Some(KeyingSetup {
        device,
        queue,
        mut renderer,
        output,
        mut params,
        size,
    }) = setup_keying_test()
    else {
        return;
    };
    // Enable feedback so the cleared trails feed into the next frame
    params.feedback.amount = 0.9;

    // Render first frame
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(
        &device,
        &queue,
        &mut encoder,
        &FrameParams::at_rest(&params),
        0.0,
        &output.view,
        size,
    );
    queue.submit([encoder.finish()]);

    // Clear trails
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.clear_feedback(&mut encoder);
    queue.submit([encoder.finish()]);

    // Render second frame after clearing
    let mut encoder = device.create_command_encoder(&Default::default());
    renderer.render(
        &device,
        &queue,
        &mut encoder,
        &FrameParams::at_rest(&params),
        0.1,
        &output.view,
        size,
    );
    queue.submit([encoder.finish()]);

    let pixels = read_back(&device, &queue, &output.texture);
    let px = |x: u32| {
        let i = ((size.1 / 2 * size.0 + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    };
    assert_eq!(
        px(40),
        [255, 0, 0],
        "the keyed dark level still shows the red background after clearing trails"
    );
    assert_eq!(px(200), [255, 255, 255], "the white level stays");
}

/// Frame `k` of a test clip: flat gray at `levels[k]`.
fn flat_frames(levels: &[u8], size: (u32, u32)) -> Vec<Vec<u8>> {
    levels
        .iter()
        .map(|&g| vec![g; (size.0 * size.1) as usize])
        .collect()
}

/// Draws `sample` from `frames` (luma) through a frames pass and reads back the R8 output.
fn draw_frames(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frames: &[Vec<u8>],
    size: (u32, u32),
    sample: &Sample,
    map: Option<&GrayImage>,
) -> Vec<u8> {
    let mut pass = FramesPass::new(device, queue, Pixels::Luma, size, 256);
    pass.set_map(device, queue, map);
    for k in pass.missing(device, sample) {
        if let Some(frame) = usize::try_from(k).ok().and_then(|k| frames.get(k)) {
            pass.upload(queue, k, frame);
        }
    }
    assert!(
        pass.missing(device, sample)
            .iter()
            .all(|&k| k >= frames.len() as i64)
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    pass.draw(device, queue, &mut encoder, sample);
    queue.submit([encoder.finish()]);
    read_back_texels(device, queue, &pass.target.texture, 1)
}

fn still_sample(base: f64, between: Between) -> Sample {
    Sample {
        base,
        reach: 0.0,
        first: base.floor() as i64,
        last: base.floor() as i64 + 1,
        between,
        slit: Slit::Off,
        flip: false,
    }
}

#[test]
fn the_frames_pass_shows_or_blends_neighbouring_frames() {
    let Some((device, queue)) = device() else {
        return;
    };
    let size = (8, 4);
    let frames = flat_frames(&[0, 100, 200, 250], size);
    let middle = |base, between| {
        draw_frames(
            &device,
            &queue,
            &frames,
            size,
            &still_sample(base, between),
            None,
        )[13]
    };
    assert_eq!(middle(1.0, Between::Blend), 100);
    assert!(middle(1.5, Between::Blend).abs_diff(150) <= 1);
    assert!(middle(2.25, Between::Blend).abs_diff(213) <= 1);
    assert_eq!(middle(1.4, Between::Nearest), 100);
    assert_eq!(middle(1.6, Between::Nearest), 200);
}

#[test]
fn slit_scan_rows_show_older_frames_further_down() {
    let Some((device, queue)) = device() else {
        return;
    };
    let size = (4, 16);
    let frames = flat_frames(&[0, 30, 60, 90, 120, 150, 180, 210], size);
    let sample = Sample {
        base: 7.0,
        reach: 7.0,
        first: 0,
        last: 8,
        between: Between::Blend,
        slit: Slit::Rows,
        flip: false,
    };
    let pixels = draw_frames(&device, &queue, &frames, size, &sample, None);
    for y in 0..size.1 {
        let expected = 30.0 * (7.0 - 7.0 * (y as f64 + 0.5) / 16.0);
        let got = pixels[(y * size.0) as usize];
        assert!(
            (f64::from(got) - expected).abs() <= 1.5,
            "row {y}: {got} vs {expected}"
        );
    }
    // Flipped, the bottom row is now and the top is the depth ago.
    let flipped = draw_frames(
        &device,
        &queue,
        &frames,
        size,
        &Sample {
            flip: true,
            ..sample
        },
        None,
    );
    assert!(
        flipped[0] < 15 && flipped[(15 * size.0) as usize] > 195,
        "{flipped:?}"
    );
}

#[test]
fn slit_scan_follows_the_map_image() {
    let Some((device, queue)) = device() else {
        return;
    };
    let size = (16, 4);
    let frames = flat_frames(&[0, 30, 60, 90, 120, 150, 180, 210], size);
    // Black on the left (now), white on the right (the depth ago).
    let map = GrayImage {
        width: 2,
        height: 1,
        pixels: vec![0, 255],
    };
    let sample = Sample {
        base: 6.0,
        reach: 4.0,
        first: 2,
        last: 7,
        between: Between::Nearest,
        slit: Slit::Map,
        flip: false,
    };
    let pixels = draw_frames(&device, &queue, &frames, size, &sample, Some(&map));
    assert_eq!(pixels[0], 180, "now: frame 6");
    assert_eq!(pixels[15], 60, "the depth ago: frame 2");
}

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
/// that buffer copies require.
fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    read_back_texels(device, queue, texture, 4)
}

/// Copies a texture of `bytes` bytes per pixel into memory, removing the 256-byte row
/// padding that buffer copies require.
fn read_back_texels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    bytes: u32,
) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let row = width * bytes;
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
