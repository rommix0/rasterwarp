# Rasterwarp Plan 3 (Look) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the Plan 3 look: true raster (scan line) mode, colorizer thresholds, scan-direction edge fringing with ringing, line timing jitter, rotation axis wander and level keying over a background image.

**Architecture:**
- **Parameters** live in `Params` and blend like everything else (numbers lerp, switches flip at the ramp's midpoint), so every look setting works in Transition and Sequence modes.
- **Deflection** moves into a shared WGSL file used by both the warp pass (inverse map, per pixel) and the new raster pass (forward map, instanced scan lines with additive blending).
- **Colorize** smears edges to their right with a CPU-computed one-sided kernel, then counts thresholds to find each pixel's level, and writes premultiplied color with alpha for keying.
- **Composite** shows a background image through see-through levels.

**Tech Stack:** Rust 1.98 (edition 2024), wgpu 30, winit 0.30, egui 0.36, image 0.25, rfd 0.17.

**Spec:** `docs/superpowers/specs/2026-10-07-rasterwarp-motion-output-look-design.md`, section "Plan 3: Look" (refined alongside this plan; see the design decisions below), plus the Testing section's Plan 3 items.

## Global Constraints

- Platform: Windows 10, NVIDIA RTX 3090, Rust 1.98 stable MSVC, edition 2024.
- No new crates. The versions stay pinned: wgpu 30, winit 0.30, egui 0.36, ffmpeg-next 9.0.0, chrono 0.4, rfd 0.17, image 0.25.
- `cargo build`, `cargo test` and `cargo clippy --all-targets` must have zero warnings. Run `cargo fmt` before committing.
- Every commit message ends with the trailer line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Don't use the word "Scanimate" in user-facing text.
- Ranges and defaults, exactly: thresholds 0–1 (default even, `k/levels`); bandwidth 0–8 px (1.0); ringing 0–1 (0.2); line jitter 0–10 px (0); axis wander 0–1 (0.15); raster lines 100–1200 (600); beam width 0.5–4.0 px (1.2); compensation 0–1 (1.0); speed compensation 0–1 (0.3); keying off with level 1 see-through.
- Code in this plan was compiled, tested and run on the dev machine before the plan was written. Every task's end state was also built and tested on its own. Transcribe it exactly. Each edit's "replace" text appears exactly once in the file when applied in the order given. A few inserted blocks end just before an existing closing brace, which then closes them; the result is the tested file.

## Design decisions made while building (beyond the spec text)

1. **Raster beam speed** comes from the previous canvas frame's deflection: the renderer keeps last frame's `WarpUniforms` and the vertex shader evaluates the beam with both. Oscillator phases are CPU-integrated clocks, so the spec's original `time − 1/60` re-evaluation wasn't possible. Speed is distance moved per second of animation time; while paused it is 0.
2. **Raster strip and beam:** each line's strip reaches one beam width either side, and the beam is a Gaussian with σ = half the beam width, so the strip edges are already dark.
3. **Area compensation** measures the distance to the next line across this line, against the undeflected, unzoomed spacing, so zooming in brightens spread lines too.
4. **Thresholds:** order doesn't matter to the shader (it counts thresholds at or below the brightness), so overshooting curves that cross them are harmless. Softness spans `softness / levels` of brightness centred on each threshold, the same width as before. Changing the level count re-spaces the thresholds evenly; the sliders are 0–1 and a changed threshold is clamped between its neighbours.
5. **Fringing kernel:** below 2/π px the minimum kernel fades toward no smear (smaller cutoffs would ring even at ringing 0); the last 12 taps fade out; taps below 1e-5 are skipped. Bypass (grayscale) shows the smeared brightness too.
6. **Line jitter seed** is `Motion`'s count of `advance` calls, carried in `WarpFrame.seed`, and hashed with PCG in the shader. The jitter hash is tested on the GPU, not as a CPU function.
7. **Keying output is premultiplied** (color × alpha), so trails over the background fade at the feedback rate instead of twice as fast. Keying follows the brightness level, not the cycling palette color.
8. **Verified in the real app on a 60 Hz display:** default look with 1 px fringing, raster mode at 600 lines and keying over a background image all ran at 60 fps with 0 late canvas frames.

## File Structure

| File | Responsibility |
|---|---|
| `src/params.rs` (modify) | New fields, ranges, defaults, `RasterParams`, `KeyParams`, `even_thresholds` |
| `src/blend.rs` (modify) | Blending the new fields; frame fields for the renderer |
| `src/motion.rs` (modify) | The line jitter seed (animation frame count) |
| `src/fringe.rs` (new) | The one-sided smear kernel |
| `shaders/colorize.wgsl`, `src/passes/colorize.rs` (modify) | Smear, thresholds, keying alpha |
| `shaders/deflection.wgsl` (new) | Deflection, transform with axis wander, line jitter, shared by warp and raster |
| `shaders/warp.wgsl`, `src/passes/warp.rs` (modify) | The warp pass on the shared deflection |
| `shaders/raster.wgsl`, `src/passes/raster.rs` (new) | True raster mode |
| `src/passes/mod.rs` (modify) | Choosing warp or raster; previous deflection; background; scanlines off in raster mode |
| `shaders/feedback.wgsl` (modify) | Alpha through the trails |
| `shaders/composite.wgsl`, `src/passes/composite.rs` (modify) | Background through see-through levels |
| `src/source.rs` (modify) | `ColorImage` loading and upload |
| `src/preview.rs`, `src/app.rs`, `src/ui.rs` (modify) | Background plumbing and the new panel controls |
| `tests/smoke.rs` (modify) | GPU readback tests for every feature |

---
### Task 1: Look parameters and their blending

Adds every Plan 3 parameter with its range and default, blends them, and gives the line jitter its seed. Nothing renders them yet.

**Files:**
- Modify: `src/blend.rs`, `src/motion.rs`, `src/params.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `params::ranges::{THRESHOLD, BANDWIDTH, RINGING, LINE_JITTER, AXIS_WANDER, RASTER_LINES (u32), BEAM_WIDTH, COMPENSATION, SPEED_COMPENSATION}`; `params::THRESHOLD_COUNT` (7).
  - `WarpParams.line_jitter: f32` (canvas pixels), `WarpParams.axis_wander: f32`.
  - `ColorizeParams.thresholds: [f32; 7]`, `.bandwidth: f32` (canvas pixels), `.ringing: f32`.
  - `params::RasterParams { enabled: bool, lines: u32, beam_width: f32, compensation: f32, speed_compensation: f32 }` and `params::KeyParams { enabled: bool, levels: u8 }` (bit i = level i see-through), as `Params.raster` and `Params.key`.
  - `params::even_thresholds(levels: u32) -> [f32; 7]` (`k / levels`, unused entries 1).
  - `blend::WarpFrame.{line_jitter, axis_wander, seed: u32}`; `blend::ColorizeFrame.{thresholds, bandwidth, ringing}`; `blend::FrameParams.{raster: RasterParams, key: KeyParams}`.
  - `Motion::frame()` and `Motion::preview()` set `warp.seed` to the number of `advance` calls so far (animation frames); `FrameParams::at_rest` leaves it 0.

- [ ] **Step 1: Write the failing tests.**

In `src/blend.rs`, replace:

```rust
    }

    #[test]
    fn palette_lerps_in_linear_light() {
```

with:

```rust
    }

    #[test]
    fn look_fields_lerp_and_switch_at_midpoint() {
        let a = Params::default();
        let mut b = a;
        b.colorize.thresholds[0] = 0.5;
        b.colorize.bandwidth = 3.0;
        b.colorize.ringing = 0.6;
        b.warp.line_jitter = 4.0;
        b.warp.axis_wander = 0.55;
        b.raster.enabled = true;
        b.raster.lines = 200;
        b.raster.beam_width = 2.2;
        b.key.enabled = true;
        b.key.levels = 0b110;
        let early = blend(&a, &b, 0.25, Some(0.25), &rest(), &rest());
        let expected = a.colorize.thresholds[0] + 0.25 * (0.5 - a.colorize.thresholds[0]);
        assert!((early.colorize.thresholds[0] - expected).abs() < 1e-6);
        assert!((early.colorize.bandwidth - 1.5).abs() < 1e-6);
        assert!((early.colorize.ringing - 0.3).abs() < 1e-6);
        assert!((early.warp.line_jitter - 1.0).abs() < 1e-6);
        assert!((early.warp.axis_wander - 0.25).abs() < 1e-6);
        assert!((early.raster.beam_width - 1.45).abs() < 1e-6);
        assert_eq!((early.raster.enabled, early.raster.lines), (false, 600));
        assert_eq!(early.key, a.key);
        let late = blend(&a, &b, 0.5, Some(0.5), &rest(), &rest());
        assert_eq!((late.raster.enabled, late.raster.lines), (true, 200));
        assert_eq!(late.key, b.key);
        // Overshooting curves stay inside each range.
        let over = blend(&a, &b, 3.0, Some(0.9), &rest(), &rest());
        assert_eq!(over.warp.line_jitter, *ranges::LINE_JITTER.end());
        assert_eq!(over.raster.beam_width, *ranges::BEAM_WIDTH.end());
    }

    #[test]
    fn palette_lerps_in_linear_light() {
```

In `src/motion.rs`, replace:

```rust
    }

    #[test]
    fn live_mode_edits_what_is_shown() {
```

with:

```rust
    }

    #[test]
    fn jitter_seed_counts_animation_frames() {
        let mut m = Motion::new(Params::default());
        assert_eq!(m.frame().warp.seed, 0);
        m.advance(secs(1.0 / 60.0));
        m.advance(secs(1.0 / 60.0));
        assert_eq!(m.frame().warp.seed, 2);
        // A paused frame doesn't advance animation, so the seed (and jitter) holds.
        assert_eq!(m.frame().warp.seed, 2);
        m.set_mode(Mode::Transition);
        assert_eq!(m.preview().unwrap().frame.warp.seed, 2);
    }

    #[test]
    fn live_mode_edits_what_is_shown() {
```

In `src/params.rs`, replace:

```rust
        assert!(ranges::NOISE.contains(&p.glow.noise));
```

with:

```rust
        assert!(ranges::NOISE.contains(&p.glow.noise));
        assert!(
            p.colorize
                .thresholds
                .iter()
                .all(|t| ranges::THRESHOLD.contains(t))
        );
        assert!(ranges::BANDWIDTH.contains(&p.colorize.bandwidth));
        assert!(ranges::RINGING.contains(&p.colorize.ringing));
        assert!(ranges::LINE_JITTER.contains(&p.warp.line_jitter));
        assert!(ranges::AXIS_WANDER.contains(&p.warp.axis_wander));
        assert!(ranges::RASTER_LINES.contains(&p.raster.lines));
        assert!(ranges::BEAM_WIDTH.contains(&p.raster.beam_width));
        assert!(ranges::COMPENSATION.contains(&p.raster.compensation));
        assert!(ranges::SPEED_COMPENSATION.contains(&p.raster.speed_compensation));
    }

    #[test]
    fn even_thresholds_split_brightness_into_equal_bands() {
        let six = even_thresholds(6);
        for (k, t) in six.iter().take(5).enumerate() {
            assert!((t - (k + 1) as f32 / 6.0).abs() < 1e-6);
        }
        assert_eq!(&six[5..], &[1.0, 1.0], "unused thresholds");
        for levels in ranges::LEVELS {
            let t = even_thresholds(levels);
            assert!(t.windows(2).all(|w| w[0] <= w[1]), "{levels} levels: {t:?}");
        }
        assert_eq!(Params::default().colorize.thresholds, six);
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test --lib`. Expected: fails to compile, with errors such as ``cannot find value `LINE_JITTER` in module `ranges` `` for each new range, field and function the tests use.

- [ ] **Step 3: Implement.**

In `src/blend.rs`, replace:

```rust
    Axis, ColorizeParams, Envelope, FeedbackParams, GlowParams, OSCILLATOR_COUNT, OscInput,
    OscSync, Oscillator, PALETTE_SIZE, Params, Waveform, ranges, srgb_to_linear,
```

with:

```rust
    Axis, ColorizeParams, Envelope, FeedbackParams, GlowParams, KeyParams, OSCILLATOR_COUNT,
    OscInput, OscSync, Oscillator, PALETTE_SIZE, Params, RasterParams, THRESHOLD_COUNT, Waveform,
    ranges, srgb_to_linear,
```

In `src/blend.rs`, replace:

```rust
    pub oscillators: Vec<OscSlot>,
```

with:

```rust
    pub oscillators: Vec<OscSlot>,
    /// Canvas pixels.
    pub line_jitter: f32,
    pub axis_wander: f32,
    /// Seeds the line jitter: the number of animation frames so far (see
    /// `Motion::frame`), so the jitter is new every frame and frozen while paused.
    pub seed: u32,
```

In `src/blend.rs`, replace:

```rust
    pub bypass: bool,
```

with:

```rust
    pub bypass: bool,
    /// Not necessarily ascending: an overshooting curve can push them past each other.
    /// The shader counts the thresholds at or below a brightness, so order doesn't matter.
    pub thresholds: [f32; THRESHOLD_COUNT],
    pub bandwidth: f32,
    pub ringing: f32,
```

In `src/blend.rs`, replace:

```rust
    pub glow: GlowParams,
```

with:

```rust
    pub glow: GlowParams,
    pub raster: RasterParams,
    pub key: KeyParams,
```

In `src/blend.rs`, replace:

```rust
            oscillators,
```

with:

```rust
            oscillators,
            line_jitter: lerp_in(a.line_jitter, b.line_jitter, t, ranges::LINE_JITTER),
            axis_wander: lerp_in(a.axis_wander, b.axis_wander, t, ranges::AXIS_WANDER),
            seed: 0,
```

In `src/blend.rs`, replace:

```rust
        glow: blend_glow(&from.glow, &to.glow, t),
```

with:

```rust
        glow: blend_glow(&from.glow, &to.glow, t),
        raster: blend_raster(&from.raster, &to.raster, t),
        key: if t >= 0.5 { to.key } else { from.key },
    }
}

fn blend_raster(a: &RasterParams, b: &RasterParams, t: f32) -> RasterParams {
    let discrete = if t >= 0.5 { b } else { a };
    RasterParams {
        enabled: discrete.enabled,
        lines: discrete.lines,
        beam_width: lerp_in(a.beam_width, b.beam_width, t, ranges::BEAM_WIDTH),
        compensation: lerp_in(a.compensation, b.compensation, t, ranges::COMPENSATION),
        speed_compensation: lerp_in(
            a.speed_compensation,
            b.speed_compensation,
            t,
            ranges::SPEED_COMPENSATION,
        ),
```

In `src/blend.rs`, replace:

```rust
        bypass: discrete.bypass,
```

with:

```rust
        bypass: discrete.bypass,
        thresholds: std::array::from_fn(|i| {
            lerp_in(a.thresholds[i], b.thresholds[i], t, ranges::THRESHOLD)
        }),
        bandwidth: lerp_in(a.bandwidth, b.bandwidth, t, ranges::BANDWIDTH),
        ringing: lerp_in(a.ringing, b.ringing, t, ranges::RINGING),
```

In `src/motion.rs`, replace:

```rust
    preview_shown: Option<PreviewSource>,
```

with:

```rust
    preview_shown: Option<PreviewSource>,
    /// Animation frames so far (calls to `advance`); seeds the line jitter.
    frames: u32,
```

In `src/motion.rs`, replace:

```rust
            preview_shown: None,
```

with:

```rust
            preview_shown: None,
            frames: 0,
```

In `src/motion.rs`, replace:

```rust
        let dt = (ticks as f64 / TICKS_PER_SECOND as f64) as f32;
```

with:

```rust
        let dt = (ticks as f64 / TICKS_PER_SECOND as f64) as f32;
        self.frames = self.frames.wrapping_add(1);
```

In `src/motion.rs`, replace:

```rust
        Some(Preview {
            source,
            frame: blend(p, p, 0.0, None, &self.preview, &self.preview),
        })
```

with:

```rust
        let mut frame = blend(p, p, 0.0, None, &self.preview, &self.preview);
        frame.warp.seed = self.frames;
        Some(Preview { source, frame })
```

In `src/motion.rs`, replace:

```rust
    pub fn frame(&self) -> FrameParams {
```

with:

```rust
    pub fn frame(&self) -> FrameParams {
        let mut frame = self.blended();
        frame.warp.seed = self.frames;
        frame
    }

    fn blended(&self) -> FrameParams {
```

In `src/params.rs`, replace:

```rust
    pub const NOISE: RangeInclusive<f32> = 0.0..=0.5;
```

with:

```rust
    pub const NOISE: RangeInclusive<f32> = 0.0..=0.5;
    pub const THRESHOLD: RangeInclusive<f32> = 0.0..=1.0;
    pub const BANDWIDTH: RangeInclusive<f32> = 0.0..=8.0;
    pub const RINGING: RangeInclusive<f32> = 0.0..=1.0;
    pub const LINE_JITTER: RangeInclusive<f32> = 0.0..=10.0;
    pub const AXIS_WANDER: RangeInclusive<f32> = 0.0..=1.0;
    pub const RASTER_LINES: RangeInclusive<u32> = 100..=1200;
    pub const BEAM_WIDTH: RangeInclusive<f32> = 0.5..=4.0;
    pub const COMPENSATION: RangeInclusive<f32> = 0.0..=1.0;
    pub const SPEED_COMPENSATION: RangeInclusive<f32> = 0.0..=1.0;
```

In `src/params.rs`, replace:

```rust
pub const PALETTE_SIZE: usize = 8;
```

with:

```rust
pub const PALETTE_SIZE: usize = 8;
/// One threshold between each pair of neighbouring levels.
pub const THRESHOLD_COUNT: usize = PALETTE_SIZE - 1;
```

In `src/params.rs`, replace:

```rust
    pub slave_4_to_3: bool,
```

with:

```rust
    pub slave_4_to_3: bool,
    /// Time-base error: each line shifts sideways by up to this many canvas pixels,
    /// new every animation frame.
    pub line_jitter: f32,
    /// How far the rotation pivot wanders while rotated, like a real deflection yoke.
    pub axis_wander: f32,
```

In `src/params.rs`, replace:

```rust
    pub bypass: bool,
```

with:

```rust
    pub bypass: bool,
    /// Brightness where each level starts, ascending; only the first `levels - 1` are
    /// used.
    pub thresholds: [f32; THRESHOLD_COUNT],
    /// How far edges smear to the right of themselves, in canvas pixels.
    pub bandwidth: f32,
    /// How much edges overshoot and ripple after themselves.
    pub ringing: f32,
}

/// True raster mode: draw the source as deflected scan lines instead of warping it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterParams {
    pub enabled: bool,
    pub lines: u32,
    /// Beam width in canvas pixels.
    pub beam_width: f32,
    /// How much packed or spread lines are evened out (the manual's area compensator).
    pub compensation: f32,
    /// How much fast-moving lines are brightened so fast sweeps don't fade.
    pub speed_compensation: f32,
}

/// Level keying: chosen colorizer levels become see-through, showing a background image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyParams {
    pub enabled: bool,
    /// Bit i set: level i is see-through.
    pub levels: u8,
```

In `src/params.rs`, replace:

```rust
    pub glow: GlowParams,
```

with:

```rust
    pub glow: GlowParams,
    pub raster: RasterParams,
    pub key: KeyParams,
}

/// Thresholds spaced evenly for `levels` levels (`k / levels`); unused entries are 1.
pub fn even_thresholds(levels: u32) -> [f32; THRESHOLD_COUNT] {
    std::array::from_fn(|i| {
        let k = i as u32 + 1;
        if k < levels {
            k as f32 / levels as f32
        } else {
            1.0
        }
    })
```

In `src/params.rs`, replace:

```rust
                slave_4_to_3: false,
```

with:

```rust
                slave_4_to_3: false,
                line_jitter: 0.0,
                axis_wander: 0.15,
```

In `src/params.rs`, replace:

```rust
                bypass: false,
```

with:

```rust
                bypass: false,
                thresholds: even_thresholds(6),
                bandwidth: 1.0,
                ringing: 0.2,
```

In `src/params.rs`, replace:

```rust
                noise: 0.04,
```

with:

```rust
                noise: 0.04,
            },
            raster: RasterParams {
                enabled: false,
                lines: 600,
                beam_width: 1.2,
                compensation: 1.0,
                speed_compensation: 0.3,
            },
            key: KeyParams {
                enabled: false,
                levels: 1,
```

- [ ] **Step 4: Run everything.** Run: `cargo test`. Expected: `133 passed` for the unit tests, `4 passed` in `tests/capture.rs` and `7 passed` in `tests/smoke.rs`.

- [ ] **Step 5: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 6: Commit**

```bash
git add src/blend.rs src/motion.rs src/params.rs
git commit -m "feat: add the Plan 3 look parameters and their blending" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Colorizer thresholds

The colorize shader counts how many thresholds lie at or below a pixel's brightness instead of dividing brightness evenly. The panel gets a slider per threshold and an Even button.

**Files:**
- Modify: `shaders/colorize.wgsl`, `src/passes/colorize.rs`, `src/ui.rs`, `tests/smoke.rs`

**Interfaces:**
- Consumes: `ColorizeFrame.thresholds`, `params::even_thresholds`, `ranges::THRESHOLD` (Task 1).
- Produces:
  - `ColorizeUniforms.thresholds: [[f32; 4]; 2]` (after `palette`; unused slots 1.0).
  - `tests/smoke.rs` helpers used by later tasks: `flat_params() -> Params` (no deflection, trails or CRT effects, no fringing), `gradient(w, h) -> GrayImage`, `render_still(device, queue, &Params, &GrayImage, (w, h)) -> Vec<u8>` (RGBA rows of one composited canvas frame), `first_bright(pixels, width, y) -> Option<u32>`.

- [ ] **Step 1: Write the failing tests.**

In `src/passes/colorize.rs`, replace:

```rust
        // vec4 settings + array<vec4, 8> = 16 + 128 bytes.
        assert_eq!(size_of::<ColorizeUniforms>(), 144);
```

with:

```rust
        // vec4 settings + array<vec4, 8> palette + array<vec4, 2> thresholds.
        assert_eq!(size_of::<ColorizeUniforms>(), 176);
```

In `src/passes/colorize.rs`, replace:

```rust
        let u = uniforms(&frame);
        assert_eq!(u.settings, [6.0, frame.softness, 2.5, 1.0]);
        assert_eq!(u.palette[1], [0.25, 0.5, 0.75, 1.0]);
```

with:

```rust
        frame.thresholds = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7];
        let u = uniforms(&frame);
        assert_eq!(u.settings, [6.0, frame.softness, 2.5, 1.0]);
        assert_eq!(u.palette[1], [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(u.thresholds, [[0.1, 0.2, 0.3, 0.4], [0.5, 0.6, 0.7, 1.0]]);
```

In `tests/smoke.rs`, replace:

```rust
use rasterwarp::source::test_card;
```

with:

```rust
use rasterwarp::source::{GrayImage, test_card};
```

In `tests/smoke.rs`, replace:

```rust

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
```

with:

```rust

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
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test --test smoke colorizer_levels_start_at_their_thresholds`. Expected: FAIL with `threshold 0.25: white starts at x = 128, expected about 63.75` (the levels are still split evenly).

- [ ] **Step 3: Implement.**

Replace `shaders/colorize.wgsl` with:

```wgsl
// Colorizing: posterize brightness into levels at adjustable thresholds, map each level
// to a palette color.

struct Colorize {
    settings: vec4<f32>, // levels, softness, cycle offset (levels), bypass (0 or 1)
    palette: array<vec4<f32>, 8>, // linear RGB
    thresholds: array<vec4<f32>, 2>, // 7 thresholds; only the first levels - 1 are used
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

// Fractional level index of brightness g: the number of thresholds at or below g, each
// step softened over `width` centered on its threshold. Order doesn't matter.
fn level_of(g: f32, levels: u32, width: f32) -> f32 {
    var x = 0.0;
    for (var k = 0u; k + 1u < levels; k++) {
        let th = u.thresholds[k / 4u][k % 4u];
        if width > 0.0 {
            x += smoothstep(th - 0.5 * width, th + 0.5 * width, g);
        } else {
            x += select(0.0, 1.0, g >= th);
        }
    }
    return x;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let g = textureSampleLevel(source, samp, in.uv, 0.0).r;
    if u.settings.w > 0.5 {
        let l = pow(g, 2.2);
        return vec4<f32>(l, l, l, 1.0);
    }
    let levels = u.settings.x;
    let n = u32(levels);
    // Softness is in level bands: an even band is 1 / levels of brightness.
    let x = level_of(g, n, u.settings.y / levels);
    let i0 = min(floor(x), levels - 1.0);
    let i1 = min(i0 + 1.0, levels - 1.0);
    let cycle = u.settings.z;
    let color = mix(palette_at(i0 + cycle, n), palette_at(i1 + cycle, n), x - i0);
    return vec4<f32>(color, 1.0);
}
```

In `src/passes/colorize.rs`, replace:

```rust
use crate::params::PALETTE_SIZE;
```

with:

```rust
use crate::params::{PALETTE_SIZE, THRESHOLD_COUNT};
```

In `src/passes/colorize.rs`, replace:

```rust
    pub palette: [[f32; 4]; PALETTE_SIZE],
```

with:

```rust
    pub palette: [[f32; 4]; PALETTE_SIZE],
    pub thresholds: [[f32; 4]; 2],
```

In `src/passes/colorize.rs`, replace:

```rust
        palette: c.palette_linear.map(|[r, g, b]| [r, g, b, 1.0]),
    }
}
```

with:

```rust
        palette: c.palette_linear.map(|[r, g, b]| [r, g, b, 1.0]),
        thresholds: std::array::from_fn(|row| {
            std::array::from_fn(|col| c.thresholds.get(row * 4 + col).copied().unwrap_or(1.0))
        }),
    }
}

const _: () = assert!(THRESHOLD_COUNT <= 8, "thresholds fit in two vec4s");
```

In `src/ui.rs`, replace:

```rust
use crate::params::{Axis, Envelope, OscInput, OscSync, Oscillator, Params, Waveform, ranges};
```

with:

```rust
use crate::params::{
    Axis, Envelope, OscInput, OscSync, Oscillator, Params, Waveform, even_thresholds, ranges,
};
```

In `src/ui.rs`, replace:

```rust
            ui.add(Slider::new(&mut c.levels, ranges::LEVELS).text("levels"));
```

with:

```rust
            // A new level count starts from evenly spaced thresholds.
            if ui
                .add(Slider::new(&mut c.levels, ranges::LEVELS).text("levels"))
                .changed()
            {
                c.thresholds = even_thresholds(c.levels);
            }
            ui.horizontal(|ui| {
                ui.label("Thresholds");
                if ui.button("Even").clicked() {
                    c.thresholds = even_thresholds(c.levels);
                }
            });
            let used = c.levels as usize - 1;
            for k in 0..used {
                let slider = Slider::new(&mut c.thresholds[k], ranges::THRESHOLD)
                    .max_decimals(3)
                    .text(format!("level {} from", k + 2));
                if ui.add(slider).changed() {
                    // A threshold can't pass its neighbours.
                    let lo = if k == 0 { 0.0 } else { c.thresholds[k - 1] };
                    let hi = if k + 1 < used {
                        c.thresholds[k + 1]
                    } else {
                        1.0
                    };
                    c.thresholds[k] = c.thresholds[k].clamp(lo, hi);
                }
            }
```

- [ ] **Step 4: Run everything.** Run: `cargo test`. Expected: `133 passed` for the unit tests, `4 passed` in `tests/capture.rs` and `8 passed` in `tests/smoke.rs`.

- [ ] **Step 5: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 6: Commit**

```bash
git add shaders/colorize.wgsl src/passes/colorize.rs src/ui.rs tests/smoke.rs
git commit -m "feat: colorizer thresholds" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Scan-direction edge fringing

A new `fringe` module computes the one-sided smear kernel; the colorize shader applies it before posterizing (and in bypass), so edges smear and ring to their right.

**Files:**
- Create: `src/fringe.rs`
- Modify: `shaders/colorize.wgsl`, `src/lib.rs`, `src/passes/colorize.rs`, `src/ui.rs`, `tests/smoke.rs`

**Interfaces:**
- Consumes: `ColorizeFrame.{bandwidth, ringing}`, `ranges::{BANDWIDTH, RINGING}` (Task 1); `ColorizeUniforms.thresholds`, smoke helpers (Task 2).
- Produces:
  - `fringe::TAPS` (48) and `fringe::kernel(bandwidth: f32, ringing: f32) -> ([f32; 48], usize)`: weights (`[k]` applies to the pixel k to the left, summing to 1) and how many matter.
  - `ColorizeUniforms.fringe: [[f32; 4]; 12]` and `ColorizeUniforms.extra: [f32; 4]` (x = taps in use; y and z are reserved for keying in Task 6).

- [ ] **Step 1: Write the failing tests.**

In `src/passes/colorize.rs`, replace:

```rust
        // vec4 settings + array<vec4, 8> palette + array<vec4, 2> thresholds.
        assert_eq!(size_of::<ColorizeUniforms>(), 176);
```

with:

```rust
        // vec4 settings, array<vec4, 8> palette, array<vec4, 2> thresholds,
        // array<vec4, 12> smear weights, vec4 extra.
        assert_eq!(size_of::<ColorizeUniforms>(), 384);
```

In `src/passes/colorize.rs`, replace:

```rust
        assert_eq!(u.thresholds, [[0.1, 0.2, 0.3, 0.4], [0.5, 0.6, 0.7, 1.0]]);
```

with:

```rust
        assert_eq!(u.thresholds, [[0.1, 0.2, 0.3, 0.4], [0.5, 0.6, 0.7, 1.0]]);
    }

    #[test]
    fn packs_the_smear_kernel() {
        let mut frame = FrameParams::at_rest(&Params::default()).colorize;
        frame.bandwidth = 0.0;
        let u = uniforms(&frame);
        assert_eq!(u.extra[0], 1.0, "one tap");
        assert_eq!(u.fringe[0], [1.0, 0.0, 0.0, 0.0]);
        frame.bandwidth = 3.0;
        frame.ringing = 0.4;
        let (weights, taps) = fringe::kernel(3.0, 0.4);
        let u = uniforms(&frame);
        assert_eq!(u.extra[0], taps as f32);
        assert_eq!(u.fringe[2][1], weights[9]);
```

In `tests/smoke.rs`, replace:

```rust

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
```

with:

```rust

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

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test --test smoke edges_smear_only_to_their_right`. Expected: FAIL with `the edge smears over a few pixels: []`.

- [ ] **Step 3: Implement.**

In `shaders/colorize.wgsl`, replace:

```wgsl
// Colorizing: posterize brightness into levels at adjustable thresholds, map each level
// to a palette color.
```

with:

```wgsl
// Colorizing: smear edges in the scan direction, posterize brightness into levels at
// adjustable thresholds, and map each level to a palette color.
```

In `shaders/colorize.wgsl`, replace:

```wgsl
    thresholds: array<vec4<f32>, 2>, // 7 thresholds; only the first levels - 1 are used
```

with:

```wgsl
    thresholds: array<vec4<f32>, 2>, // 7 thresholds; only the first levels - 1 are used
    fringe: array<vec4<f32>, 12>, // 48 smear weights: [k] applies to the pixel k to the left
    extra: vec4<f32>, // smear taps in use, unused, unused, unused
```

In `shaders/colorize.wgsl`, replace:

```wgsl
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let g = textureSampleLevel(source, samp, in.uv, 0.0).r;
```

with:

```wgsl
// Brightness after the scan-direction smear: a weighted sum of this pixel and the
// pixels to its left, as a band-limited signal scanned left to right smears after edges.
fn smeared(pos: vec2<i32>) -> f32 {
    let taps = i32(u.extra.x);
    var g = 0.0;
    for (var k = 0; k < taps; k++) {
        let x = max(pos.x - k, 0);
        g += u.fringe[k / 4][k % 4] * textureLoad(source, vec2<i32>(x, pos.y), 0).r;
    }
    return g;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let g = smeared(vec2<i32>(in.pos.xy));
```

Create `src/fringe.rs`:

```rust
//! Edge fringing: the one-sided smear a band-limited video signal, scanned left to right,
//! leaves after every edge. The colorize pass applies it before posterizing, so sharp
//! edges briefly pass through the intermediate levels and show their colors as fringes.

use std::f64::consts::PI;

/// Most pixels the kernel reads: the pixel itself and up to 47 to its left.
pub const TAPS: usize = 48;

/// Below this bandwidth the filter's cutoff would pass a quarter of the pixel rate, where
/// its poles turn negative and the smear would ring even with no ringing set. Smaller
/// bandwidths fade the minimum kernel out toward no smear instead.
const MIN_BANDWIDTH: f64 = 2.0 / PI;

/// Taps from here on fade out, so a long ringing tail doesn't end in a visible step.
const TAPER_FROM: usize = 36;

/// The smear kernel for `bandwidth` (canvas pixels) and `ringing` (0-1).
///
/// `weights[k]` applies to the pixel `k` to the left; they sum to 1, so flat areas are
/// unchanged. Returns the weights and how many of them matter (at least 1).
///
/// The weights are the impulse response of a 2-pole low-pass (biquad) filter with cutoff
/// `1 / (2π · bandwidth)` cycles per pixel and Q `0.5 + 1.5 · ringing`: 0.5 is critically
/// damped (no overshoot), 2.0 rings visibly.
pub fn kernel(bandwidth: f32, ringing: f32) -> ([f32; TAPS], usize) {
    let mut weights = [0.0f32; TAPS];
    weights[0] = 1.0;
    if bandwidth <= 0.0 {
        return (weights, 1);
    }
    let bandwidth = f64::from(bandwidth);
    let response = impulse_response(bandwidth.max(MIN_BANDWIDTH), f64::from(ringing));
    // Fade the minimum kernel toward no smear below the minimum bandwidth.
    let mix = (bandwidth / MIN_BANDWIDTH).min(1.0);
    for (k, w) in weights.iter_mut().enumerate() {
        let identity = if k == 0 { 1.0 } else { 0.0 };
        *w = (identity + (response[k] - identity) * mix) as f32;
    }
    let taps = weights
        .iter()
        .rposition(|w| w.abs() > 1e-5)
        .map_or(1, |last| last + 1);
    (weights, taps)
}

/// The normalized, tapered impulse response of the low-pass filter.
fn impulse_response(bandwidth: f64, ringing: f64) -> [f64; TAPS] {
    // RBJ cookbook low-pass, one sample per pixel.
    let w0 = 1.0 / bandwidth;
    let q = 0.5 + 1.5 * ringing.clamp(0.0, 1.0);
    let alpha = w0.sin() / (2.0 * q);
    let cos = w0.cos();
    let a0 = 1.0 + alpha;
    let (b0, b1, b2) = (
        (1.0 - cos) / 2.0 / a0,
        (1.0 - cos) / a0,
        (1.0 - cos) / 2.0 / a0,
    );
    let (a1, a2) = (-2.0 * cos / a0, (1.0 - alpha) / a0);
    let mut h = [0.0; TAPS];
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    for (n, out) in h.iter_mut().enumerate() {
        let x0 = if n == 0 { 1.0 } else { 0.0 };
        let y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        (x2, x1, y2, y1) = (x1, x0, y1, y0);
        let taper = if n < TAPER_FROM {
            1.0
        } else {
            let t = (n - TAPER_FROM + 1) as f64 / (TAPS - TAPER_FROM + 1) as f64;
            0.5 + 0.5 * (PI * t).cos()
        };
        *out = y0 * taper;
    }
    let sum: f64 = h.iter().sum();
    h.map(|v| v / sum)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BANDWIDTHS: [f32; 7] = [0.1, 0.5, 0.64, 1.0, 2.0, 4.0, 8.0];

    /// The step response: what an edge from 0 to 1 looks like after the smear.
    fn step(weights: &[f32; TAPS]) -> Vec<f32> {
        weights
            .iter()
            .scan(0.0, |sum, w| {
                *sum += w;
                Some(*sum)
            })
            .collect()
    }

    #[test]
    fn weights_sum_to_one() {
        for bandwidth in BANDWIDTHS {
            for ringing in [0.0, 0.2, 0.5, 1.0] {
                let (w, taps) = kernel(bandwidth, ringing);
                let sum: f32 = w[..taps].iter().sum();
                assert!(
                    (sum - 1.0).abs() < 1e-4,
                    "{bandwidth} px, ringing {ringing}: {sum}"
                );
            }
        }
    }

    #[test]
    fn zero_bandwidth_is_no_smear() {
        let (w, taps) = kernel(0.0, 0.7);
        assert_eq!(taps, 1);
        assert_eq!(w[0], 1.0);
        assert!(w[1..].iter().all(|&v| v == 0.0));
    }

    #[test]
    fn no_ringing_never_overshoots() {
        for bandwidth in BANDWIDTHS {
            let (w, _) = kernel(bandwidth, 0.0);
            assert!(w.iter().all(|&v| v >= 0.0), "{bandwidth} px: {w:?}");
            let peak = step(&w).into_iter().fold(0.0, f32::max);
            assert!(peak <= 1.0 + 1e-5, "{bandwidth} px overshoots to {peak}");
        }
    }

    #[test]
    fn ringing_overshoots_after_an_edge() {
        let (w, _) = kernel(2.0, 0.6);
        let peak = step(&w).into_iter().fold(0.0, f32::max);
        assert!(peak > 1.02, "peak {peak}");
    }

    #[test]
    fn wider_bandwidth_smears_further() {
        let reach = |bandwidth| {
            let s = step(&kernel(bandwidth, 0.0).0);
            s.iter().position(|&v| v >= 0.9).unwrap()
        };
        assert!(reach(1.0) < reach(4.0));
        assert!(reach(4.0) < reach(8.0));
    }

    #[test]
    fn weights_change_continuously() {
        let near = |a: (f32, f32), b: (f32, f32)| {
            let (wa, wb) = (kernel(a.0, a.1).0, kernel(b.0, b.1).0);
            wa.iter()
                .zip(&wb)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0, f32::max)
        };
        for bandwidth in [0.0, 0.3, 0.636, 0.64, 2.0, 7.99] {
            let gap = near((bandwidth, 0.3), (bandwidth + 0.01, 0.3));
            assert!(gap < 0.03, "bandwidth {bandwidth}: {gap}");
        }
        for ringing in [0.0, 0.5, 0.99] {
            let gap = near((2.0, ringing), (2.0, ringing + 0.01));
            assert!(gap < 0.03, "ringing {ringing}: {gap}");
        }
    }
}
```

In `src/lib.rs`, replace:

```rust
pub mod curve_editor;
```

with:

```rust
pub mod curve_editor;
pub mod fringe;
```

In `src/passes/colorize.rs`, replace:

```rust
use crate::blend::ColorizeFrame;
```

with:

```rust
use crate::blend::ColorizeFrame;
use crate::fringe;
```

In `src/passes/colorize.rs`, replace:

```rust
}

pub fn uniforms(c: &ColorizeFrame) -> ColorizeUniforms {
```

with:

```rust
    pub fringe: [[f32; 4]; fringe::TAPS / 4],
    pub extra: [f32; 4],
}

pub fn uniforms(c: &ColorizeFrame) -> ColorizeUniforms {
    let (weights, taps) = fringe::kernel(c.bandwidth, c.ringing);
```

In `src/passes/colorize.rs`, replace:

```rust
        }),
```

with:

```rust
        }),
        fringe: std::array::from_fn(|row| std::array::from_fn(|col| weights[row * 4 + col])),
        extra: [taps as f32, 0.0, 0.0, 0.0],
```

In `src/ui.rs`, replace:

```rust
            ui.add(Slider::new(&mut c.softness, ranges::SOFTNESS).text("softness"));
```

with:

```rust
            ui.add(Slider::new(&mut c.softness, ranges::SOFTNESS).text("softness"));
            ui.add(Slider::new(&mut c.bandwidth, ranges::BANDWIDTH).text("edge fringe (px)"));
            ui.add(Slider::new(&mut c.ringing, ranges::RINGING).text("ringing"));
```

- [ ] **Step 4: Run everything.** Run: `cargo test`. Expected: `140 passed` for the unit tests, `4 passed` in `tests/capture.rs` and `9 passed` in `tests/smoke.rs`.

- [ ] **Step 5: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 6: Commit**

```bash
git add shaders/colorize.wgsl src/fringe.rs src/lib.rs src/passes/colorize.rs src/ui.rs tests/smoke.rs
git commit -m "feat: scan-direction edge fringing" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Shared deflection, line timing jitter and axis wander

The deflection code moves into `shaders/deflection.wgsl` so the raster pass (Task 5) can share it. The warp pass gains line timing jitter (a sideways shift per output row) and the wandering rotation axis.

**Files:**
- Create: `shaders/deflection.wgsl`
- Modify: `shaders/warp.wgsl`, `src/passes/mod.rs`, `src/passes/warp.rs`, `src/ui.rs`, `tests/smoke.rs`

**Interfaces:**
- Consumes: `WarpFrame.{line_jitter, axis_wander, seed}`, `ranges::{LINE_JITTER, AXIS_WANDER}` (Task 1); smoke helpers (Task 2).
- Produces:
  - `shaders/deflection.wgsl`: `struct Osc`, `struct Deflection` (frame, transform, source_size, look, seed, osc), `wave`, `displacement(w, p)`, `pivot(w)`, `to_source(w, s)` (inverse transform), `from_source(w, q)` (forward transform, exactly undoes `to_source`), `pcg`, `line_shift(w, line)`.
  - `WarpUniforms` matches `Deflection`: new `look: [f32; 4]` (line jitter in frame heights, axis wander) and `seed: [u32; 4]`; 464 bytes.
  - `warp::uniforms(w, time, frame_aspect, source_aspect, canvas_height: u32)` (new last argument).
  - `tests/smoke.rs`: `render_frame(device, queue, &FrameParams, &GrayImage, (w, h)) -> Vec<u8>`; `render_still` now calls it.

- [ ] **Step 1: Write the failing tests.**

In `src/passes/warp.rs`, replace:

```rust
        // Osc = 3 x vec4 = 48 bytes; Warp = 3 x vec4 + 8 x Osc = 432 bytes.
        assert_eq!(size_of::<OscUniform>(), 48);
        assert_eq!(size_of::<WarpUniforms>(), 432);
```

with:

```rust
        // Osc = 3 x vec4 = 48 bytes; Deflection = 5 x vec4 + 8 x Osc = 464 bytes.
        assert_eq!(size_of::<OscUniform>(), 48);
        assert_eq!(size_of::<WarpUniforms>(), 464);
```

In `src/passes/warp.rs`, replace:

```rust
        let u = uniforms(&frame.warp, 2.0, 16.0 / 9.0, 1.0);
```

with:

```rust
        let u = uniforms(&frame.warp, 2.0, 16.0 / 9.0, 1.0, 1080);
```

In `src/passes/warp.rs`, replace:

```rust
        let u = uniforms(&frame.warp, 0.0, 16.0 / 9.0, 1.0);
        assert_eq!(u.frame[3], MAX_SLOTS as f32, "slot count");
        assert!(u.osc[MAX_SLOTS - 1].wave[1] > 0.0, "last slot is packed");
```

with:

```rust
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
```

In `tests/smoke.rs`, replace:

```rust
    params: &Params,
```

with:

```rust
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
```

In `tests/smoke.rs`, replace:

```rust
    renderer.render(
        device,
        queue,
        &mut encoder,
        &FrameParams::at_rest(params),
        0.0,
        &output.view,
        size,
    );
```

with:

```rust
    renderer.render(device, queue, &mut encoder, frame, 0.0, &output.view, size);
```

In `tests/smoke.rs`, replace:

```rust

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
```

with:

```rust

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

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test --test smoke line_jitter_shifts_each_row_by_at_most_the_amount`. Expected: FAIL with `rows move differently: [8, 8, 8, …]` (no row is shifted yet).

- [ ] **Step 3: Implement.**

Create `shaders/deflection.wgsl`:

```wgsl
// Deflection shared by the warp and raster passes: summed oscillators, the global
// transform with its wandering rotation axis, and line timing jitter. Positions are in
// frame heights, centered: y spans [-0.5, 0.5], x spans [-aspect/2, aspect/2].

struct Osc {
    mode: vec4<u32>, // waveform, target (0 = X, 1 = Y), input (0 = U, 1 = V, 2 = radius, 3 = time), oscillator index
    wave: vec4<f32>, // frequency, amplitude (weighted), phase (cycles), unused
    lfo: vec4<f32>,  // phase (cycles), depth, unused, unused
};

struct Deflection {
    frame: vec4<f32>,       // time, frame aspect, drift, slot count
    transform: vec4<f32>,   // zoom, rotation, offset.x, offset.y
    source_size: vec4<f32>, // source width, height in frame units, unused, unused
    look: vec4<f32>,        // line jitter (frame heights), axis wander, unused, unused
    seed: vec4<u32>,        // line jitter seed (animation frame count), unused, unused, unused
    osc: array<Osc, 8>,
};

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

// The summed oscillator displacement at frame position p.
fn displacement(w: Deflection, p: vec2<f32>) -> vec2<f32> {
    let time = w.frame.x;
    let drift = w.frame.z;
    var d = vec2<f32>(0.0);
    let slots = u32(w.frame.w);
    for (var i = 0u; i < slots; i++) {
        let o = w.osc[i];
        var input = 0.0;
        switch o.mode.z {
            case 0u: { input = p.x; }
            case 1u: { input = p.y; }
            case 2u: { input = length(p); }
            default: { input = 0.0; }
        }
        // Seed drift by oscillator, not slot, so it doesn't jump when slots appear.
        let seed = f32(o.mode.w) * 17.0;
        let freq = o.wave.x * (1.0 + drift * 0.05 * value_noise(time * 0.3 + seed));
        let phase = o.wave.z + drift * 0.1 * value_noise(time * 0.2 + seed + 5.0);
        let lfo = 1.0 - o.lfo.y * (0.5 - 0.5 * cos(TAU * o.lfo.x));
        let v = wave(o.mode.x, input * freq + phase) * o.wave.y * lfo;
        if o.mode.y == 0u {
            d.x += v;
        } else {
            d.y += v;
        }
    }
    return d;
}

// Where the rotation axis has wandered to: up to 2% of the frame height times the axis
// wander, scaled by how far the frame is rotated, drifting slowly with time.
fn pivot(w: Deflection) -> vec2<f32> {
    let time = w.frame.x;
    let amount = w.look.y * 0.02 * abs(sin(w.transform.y));
    return amount * vec2<f32>(value_noise(time * 0.15 + 31.0), value_noise(time * 0.15 + 57.0));
}

// The global transform, inverse direction: deflected frame position to source frame
// position (the warp pass's sampling map).
fn to_source(w: Deflection, s: vec2<f32>) -> vec2<f32> {
    let c = pivot(w);
    return (rotate2(s - c, w.transform.y) + c) / w.transform.x - w.transform.zw;
}

// The global transform, forward direction: source frame position to frame position (the
// raster pass's beam map). Exactly undoes `to_source`.
fn from_source(w: Deflection, q: vec2<f32>) -> vec2<f32> {
    let c = pivot(w);
    return rotate2((q + w.transform.zw) * w.transform.x - c, -w.transform.y) + c;
}

// PCG integer hash.
fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

// Line `line`'s sideways time-base error this animation frame, in frame heights: a
// uniform random amount within ± the line jitter.
fn line_shift(w: Deflection, line: u32) -> f32 {
    let r = f32(pcg(line ^ pcg(w.seed.x))) / 4294967295.0;
    return w.look.x * (2.0 * r - 1.0);
}
```

Replace `shaders/warp.wgsl` with:

```wgsl
// Deflection: displace the sampling position with summed oscillators.
// deflection.wgsl is prepended.

@group(0) @binding(0) var<uniform> u: Deflection;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var source: texture_2d<f32>;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let aspect = u.frame.y;
    var p = (in.uv - 0.5) * vec2<f32>(aspect, 1.0);
    // Each output row is a scan line; its time-base error shifts it sideways.
    p.x += line_shift(u, u32(in.pos.y));
    let q = to_source(u, p + displacement(u, p));
    let suv = q / u.source_size.xy + 0.5;
    if !inside01(suv) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let g = textureSampleLevel(source, samp, suv, 0.0).r;
    return vec4<f32>(g, g, g, 1.0);
}
```

In `src/passes/mod.rs`, replace:

```rust
            &warp::uniforms(&frame.warp, time, aspect, self.source_aspect),
```

with:

```rust
            &warp::uniforms(&frame.warp, time, aspect, self.source_aspect, self.size.1),
```

In `src/passes/warp.rs`, replace:

```rust
/// Matches `struct Warp` in warp.wgsl.
```

with:

```rust
/// Matches `struct Deflection` in deflection.wgsl.
```

In `src/passes/warp.rs`, replace:

```rust
    pub osc: [OscUniform; MAX_SLOTS],
}
```

with:

```rust
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
```

In `src/passes/warp.rs`, replace:

```rust
pub fn uniforms(w: &WarpFrame, time: f32, frame_aspect: f32, source_aspect: f32) -> WarpUniforms {
```

with:

```rust
/// `canvas_height` (pixels) converts the line jitter to frame heights.
pub fn uniforms(
    w: &WarpFrame,
    time: f32,
    frame_aspect: f32,
    source_aspect: f32,
    canvas_height: u32,
) -> WarpUniforms {
```

In `src/passes/warp.rs`, replace:

```rust
        source_size: [fit[0], fit[1], 0.0, 0.0],
```

with:

```rust
        source_size: [fit[0], fit[1], 0.0, 0.0],
        look: [
            w.line_jitter / canvas_height as f32,
            w.axis_wander,
            0.0,
            0.0,
        ],
        seed: [w.seed, 0, 0, 0],
```

In `src/passes/warp.rs`, replace:

```rust
                shader: include_str!("../../shaders/warp.wgsl"),
```

with:

```rust
                shader: SHADER,
```

In `src/ui.rs`, replace:

```rust
            ui.add(Slider::new(&mut warp.drift, ranges::DRIFT).text("analog drift"));
```

with:

```rust
            ui.add(Slider::new(&mut warp.drift, ranges::DRIFT).text("analog drift"));
            ui.add(Slider::new(&mut warp.axis_wander, ranges::AXIS_WANDER).text("axis wander"));
            ui.add(
                Slider::new(&mut warp.line_jitter, ranges::LINE_JITTER).text("line jitter (px)"),
            );
```

- [ ] **Step 4: Run everything.** Run: `cargo test`. Expected: `141 passed` for the unit tests, `4 passed` in `tests/capture.rs` and `10 passed` in `tests/smoke.rs`.

- [ ] **Step 5: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 6: Commit**

```bash
git add shaders/deflection.wgsl shaders/warp.wgsl src/passes/mod.rs src/passes/warp.rs src/ui.rs tests/smoke.rs
git commit -m "feat: shared deflection, line timing jitter and axis wander" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: True raster mode

A new raster pass draws the source as instanced scan lines mapped forward through the deflection, with a Gaussian beam, area and speed compensation, and additive blending. It replaces the warp pass while raster mode is on, and the composite's painted-on scanlines turn off.

**Files:**
- Create: `shaders/raster.wgsl`, `src/passes/raster.rs`
- Modify: `src/passes/mod.rs`, `src/ui.rs`, `tests/smoke.rs`

**Interfaces:**
- Consumes: `FrameParams.raster`, `ranges::{RASTER_LINES, BEAM_WIDTH, COMPENSATION, SPEED_COMPENSATION}` (Task 1); `shaders/deflection.wgsl` and `warp::WarpUniforms` (Task 4); smoke helpers (Tasks 2 and 4).
- Produces:
  - `passes::raster::{RasterPass, RasterUniforms { now, before: WarpUniforms, beam, misc }, samples(canvas_width) -> u32, uniforms(&RasterParams, &now, &before, canvas) -> RasterUniforms}`.
  - `Renderer` keeps the previous canvas frame's `WarpUniforms` for the beam speed, and draws with `RasterPass` into the warp target when `frame.raster.enabled`.
  - `passes::crt_glow(&FrameParams) -> GlowParams` (private): scanline strength 0 in raster mode.

- [ ] **Step 1: Write the failing tests.**

In `tests/smoke.rs`, replace:

```rust

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
```

with:

```rust

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

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test --test smoke raster_mode_draws_separate_scan_lines`. Expected: FAIL with `0 lines: []` (the warp pass still draws a flat image).

- [ ] **Step 3: Implement.**

Create `shaders/raster.wgsl`:

```wgsl
// True raster mode: draws the source as scan lines, each mapped forward through the
// global transform and the deflection, with a Gaussian beam profile and additive
// blending. fullscreen.wgsl and deflection.wgsl are prepended.

struct Raster {
    now: Deflection,
    before: Deflection, // the previous canvas frame's deflection, for the beam speed
    beam: vec4<f32>,    // lines, beam width (frame heights), compensation, speed compensation
    misc: vec4<f32>,    // samples per line, unused, unused, unused
};

@group(0) @binding(0) var<uniform> r: Raster;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var source: texture_2d<f32>;

struct BeamOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) suv: vec2<f32>,  // source uv under the beam centre
    @location(1) across: f32,     // -1 to 1 across the drawn strip
    @location(2) gain: f32,       // area and speed compensation
};

// Where the beam for source frame position q lands, before line jitter.
fn deflected(w: Deflection, q: vec2<f32>) -> vec2<f32> {
    let s = from_source(w, q);
    return s + displacement(w, s);
}

@vertex
fn vs_beam(@builtin(vertex_index) vi: u32, @builtin(instance_index) line: u32) -> BeamOut {
    let lines = r.beam.x;
    let samples = r.misc.x;
    let size = r.now.source_size.xy;
    // Each line is a triangle strip: two vertices (one per side) per sample.
    let u = f32(vi / 2u) / samples;
    let side = select(-1.0, 1.0, (vi & 1u) == 1u);
    let v = (f32(line) + 0.5) / lines;
    let q = (vec2<f32>(u, v) - 0.5) * size;
    let centre = deflected(r.now, q);

    // The line's direction, from its neighbouring samples.
    let du = vec2<f32>(size.x / samples, 0.0);
    let along = deflected(r.now, q + du) - deflected(r.now, q - du);
    let len = length(along);
    let normal = select(vec2<f32>(0.0, 1.0), vec2<f32>(-along.y, along.x) / len, len > 1e-8);

    // Area compensation: lines spread apart brighten, packed lines dim, measured as the
    // distance to the next line across this one against the undeflected spacing.
    let rest = size.y / lines;
    let next = deflected(r.now, q + vec2<f32>(0.0, rest));
    let spacing = abs(dot(next - centre, normal)) / rest;
    let area = mix(1.0, spacing, r.beam.z);

    // Speed compensation: fast-moving beams brighten so fast sweeps don't fade. Speed is
    // in frame heights per second since the previous canvas frame (none while paused).
    let dt = r.now.frame.x - r.before.frame.x;
    let moved = length(centre - deflected(r.before, q));
    let speed = select(0.0, moved / dt, dt > 0.0);
    let boost = 1.0 + r.beam.w * min(speed / 0.5, 4.0);

    // Each line's time-base error shifts it sideways.
    var p = centre;
    p.x += line_shift(r.now, line);
    // The strip reaches one beam width either side of the centre: the beam's Gaussian is
    // half that wide, so the edges are already dark.
    p += normal * side * r.beam.y;

    var out: BeamOut;
    let aspect = r.now.frame.y;
    out.pos = vec4<f32>(p.x / (0.5 * aspect), -2.0 * p.y, 0.0, 1.0);
    out.suv = vec2<f32>(u, v);
    out.across = side;
    out.gain = area * boost;
    return out;
}

@fragment
fn fs_beam(in: BeamOut) -> @location(0) vec4<f32> {
    let g = textureSampleLevel(source, samp, in.suv, 0.0).r;
    // Gaussian with sigma = half the beam width.
    let profile = exp(-2.0 * in.across * in.across);
    let b = g * profile * in.gain;
    return vec4<f32>(b, b, b, 1.0);
}
```

In `src/passes/mod.rs`, replace:

```rust
pub mod warp;

use crate::blend::FrameParams;
```

with:

```rust
pub mod raster;
pub mod warp;

use crate::blend::FrameParams;
use crate::params::GlowParams;
```

In `src/passes/mod.rs`, replace:

```rust
    warp: warp::WarpPass,
```

with:

```rust
    warp: warp::WarpPass,
    raster: raster::RasterPass,
    /// The deflection the previous canvas frame was drawn with, for the raster beam speed.
    previous: Option<warp::WarpUniforms>,
```

In `src/passes/mod.rs`, replace:

```rust
            warp: warp::WarpPass::new(device, w, h),
```

with:

```rust
            warp: warp::WarpPass::new(device, w, h),
            raster: raster::RasterPass::new(device),
            previous: None,
```

In `src/passes/mod.rs`, replace:

```rust
    /// Draws the next canvas frame (warp, colorize, feedback, bloom) without showing it.
    /// Each call advances the feedback trails by one frame.
```

with:

```rust
    /// Draws the next canvas frame (warp or raster, colorize, feedback, bloom) without
    /// showing it. Each call advances the feedback trails by one frame.
```

In `src/passes/mod.rs`, replace:

```rust
        self.warp.render(
            device,
            queue,
            encoder,
            &self.source,
            &warp::uniforms(&frame.warp, time, aspect, self.source_aspect, self.size.1),
        );
```

with:

```rust
        let deflection = warp::uniforms(&frame.warp, time, aspect, self.source_aspect, self.size.1);
        if frame.raster.enabled {
            let before = self.previous.unwrap_or(deflection);
            self.raster.render(
                device,
                queue,
                encoder,
                &self.source,
                &self.warp.target.view,
                &raster::uniforms(&frame.raster, &deflection, &before, self.size),
            );
        } else {
            self.warp
                .render(device, queue, encoder, &self.source, &deflection);
        }
        self.previous = Some(deflection);
```

In `src/passes/mod.rs`, replace:

```rust
            self.bloom.output(),
            output,
            &composite::uniforms(
                &frame.glow,
                time,
                self.size,
                output_size,
```

with:

```rust
            self.bloom.output(),
            output,
            &composite::uniforms(
                &crt_glow(frame),
                time,
                self.size,
                output_size,
```

In `src/passes/mod.rs`, replace:

```rust
            self.bloom.output(),
            output,
            &composite::uniforms(
                &frame.glow,
                time,
                self.size,
                self.size,
```

with:

```rust
            self.bloom.output(),
            output,
            &composite::uniforms(
                &crt_glow(frame),
                time,
                self.size,
                self.size,
```

In `src/passes/mod.rs`, replace:

```rust
            ),
        );
    }
}
```

with:

```rust
            ),
        );
    }
}

/// The CRT effects the composite applies. Raster mode draws real scan lines, so the
/// painted-on scanline overlay is off.
fn crt_glow(frame: &FrameParams) -> GlowParams {
    let mut glow = frame.glow;
    if frame.raster.enabled {
        glow.scanline_strength = 0.0;
    }
    glow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::Params;

    #[test]
    fn raster_mode_turns_off_the_painted_scanlines() {
        let mut frame = FrameParams::at_rest(&Params::default());
        assert_eq!(crt_glow(&frame), frame.glow);
        frame.raster.enabled = true;
        let glow = crt_glow(&frame);
        assert_eq!(glow.scanline_strength, 0.0);
        assert_eq!(glow.bloom_intensity, frame.glow.bloom_intensity);
    }
}
```

Create `src/passes/raster.rs`:

```rust
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

/// Samples along each line: one per two canvas pixels, from 256 to 2048.
pub fn samples(canvas_width: u32) -> u32 {
    (canvas_width / 2).clamp(256, 2048)
}

/// `canvas` is the canvas size in pixels.
pub fn uniforms(
    raster: &RasterParams,
    now: &WarpUniforms,
    before: &WarpUniforms,
    canvas: (u32, u32),
) -> RasterUniforms {
    RasterUniforms {
        now: *now,
        before: *before,
        beam: [
            raster.lines as f32,
            raster.beam_width / canvas.1 as f32,
            raster.compensation,
            raster.speed_compensation,
        ],
        misc: [samples(canvas.0) as f32, 0.0, 0.0, 0.0],
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
        let now = warp::uniforms(&frame.warp, 1.0, 16.0 / 9.0, 1.0, 1080);
        let before = warp::uniforms(&frame.warp, 0.5, 16.0 / 9.0, 1.0, 1080);
        let u = uniforms(&p.raster, &now, &before, (1920, 1080));
        assert_eq!(u.beam[0], 600.0);
        assert!((u.beam[1] - 1.2 / 1080.0).abs() < 1e-9);
        assert_eq!((u.beam[2], u.beam[3]), (1.0, 0.3));
        assert_eq!(u.misc[0], 960.0);
        assert_eq!((u.now.frame[0], u.before.frame[0]), (1.0, 0.5));
    }
}
```

In `src/ui.rs`, replace:

```rust
                warp_section(ui, params);
```

with:

```rust
                warp_section(ui, params);
                raster_section(ui, params);
```

In `src/ui.rs`, replace:

```rust

fn colorize_section(ui: &mut Ui, params: &mut Params) {
```

with:

```rust

fn raster_section(ui: &mut Ui, params: &mut Params) {
    let r = &mut params.raster;
    CollapsingHeader::new("Raster").show(ui, |ui| {
        ui.checkbox(&mut r.enabled, "True raster (scan lines)")
            .on_hover_text(
                "Draws the source as real scan lines bent by the oscillators. Warp mode \
                 maps each pixel back to the source and raster mode maps each line forward, \
                 so the same settings give mirror-image ripples.",
            );
        ui.add_enabled_ui(r.enabled, |ui| {
            ui.add(Slider::new(&mut r.lines, ranges::RASTER_LINES).text("lines"));
            ui.add(Slider::new(&mut r.beam_width, ranges::BEAM_WIDTH).text("beam width (px)"));
            ui.add(
                Slider::new(&mut r.compensation, ranges::COMPENSATION).text("area compensation"),
            );
            ui.add(
                Slider::new(&mut r.speed_compensation, ranges::SPEED_COMPENSATION)
                    .text("speed compensation"),
            );
        });
    });
}

fn colorize_section(ui: &mut Ui, params: &mut Params) {
```

- [ ] **Step 4: Run everything.** Run: `cargo test`. Expected: `146 passed` for the unit tests, `4 passed` in `tests/capture.rs` and `11 passed` in `tests/smoke.rs`.

- [ ] **Step 5: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 6: Commit**

```bash
git add shaders/raster.wgsl src/passes/mod.rs src/passes/raster.rs src/ui.rs tests/smoke.rs
git commit -m "feat: true raster mode" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Level keying over a background image

Chosen levels become see-through. Colorize writes premultiplied color with alpha, feedback keeps alpha with the same max rule, and the composite shows a background image (or black) where the image is see-through. The panel's Keying section picks levels and loads the background.

**Files:**
- Modify: `shaders/colorize.wgsl`, `shaders/composite.wgsl`, `shaders/feedback.wgsl`, `src/app.rs`, `src/passes/colorize.rs`, `src/passes/composite.rs`, `src/passes/mod.rs`, `src/preview.rs`, `src/source.rs`, `src/ui.rs`, `tests/smoke.rs`

**Interfaces:**
- Consumes: `FrameParams.key` (Task 1); `ColorizeUniforms.extra` (Task 3); smoke helpers (Tasks 2 and 4).
- Produces:
  - `source::{ColorImage { width, height, pixels (sRGB RGBA) }, ColorImage::black(), ColorImage::aspect(), load_color(path), upload_color(device, queue, &ColorImage) (Rgba8UnormSrgb), ensure_size_fits(width, height, max_side)}`.
  - `colorize::uniforms(&ColorizeFrame, &KeyParams)` (new argument); `extra.y` = keying on, `extra.z` = see-through mask.
  - `composite::{cover(canvas_aspect, image_aspect) -> [f32; 2]}`; `CompositeUniforms.background: [f32; 4]` (64 bytes); `composite::uniforms(.., encode_srgb, background_aspect: f32)`; `CompositePass::render(.., image, bloom, background, output, uniforms)`.
  - `Renderer::set_background(device, queue, Option<&ColorImage>)` and `PreviewView::set_background(..)`.
  - `UiState.background_info: Option<String>`; `UiActions.{choose_background, clear_background}`.

- [ ] **Step 1: Write the failing tests.**

In `src/passes/colorize.rs`, replace:

```rust
        frame.thresholds = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7];
        let u = uniforms(&frame);
```

with:

```rust
        frame.thresholds = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7];
        let u = uniforms(&frame, &Params::default().key);
```

In `src/passes/colorize.rs`, replace:

```rust
        frame.bandwidth = 0.0;
        let u = uniforms(&frame);
```

with:

```rust
        frame.bandwidth = 0.0;
        let key = Params::default().key;
        let u = uniforms(&frame, &key);
```

In `src/passes/colorize.rs`, replace:

```rust
        let u = uniforms(&frame);
        assert_eq!(u.extra[0], taps as f32);
        assert_eq!(u.fringe[2][1], weights[9]);
```

with:

```rust
        let u = uniforms(&frame, &key);
        assert_eq!(u.extra[0], taps as f32);
        assert_eq!(u.fringe[2][1], weights[9]);
    }

    #[test]
    fn packs_the_keyed_levels() {
        let frame = FrameParams::at_rest(&Params::default()).colorize;
        let key = KeyParams {
            enabled: true,
            levels: 0b101,
        };
        assert_eq!(uniforms(&frame, &key).extra[1..3], [1.0, 5.0]);
        let off = KeyParams {
            enabled: false,
            ..key
        };
        assert_eq!(uniforms(&frame, &off).extra[1], 0.0);
```

In `src/passes/composite.rs`, replace:

```rust
        assert_eq!(size_of::<CompositeUniforms>(), 48);
```

with:

```rust
        assert_eq!(size_of::<CompositeUniforms>(), 64);
```

In `src/passes/composite.rs`, replace:

```rust
        let u = uniforms(&p, 0.0, (1000, 500), (1000, 500), false);
```

with:

```rust
        let u = uniforms(&p, 0.0, (1000, 500), (1000, 500), false, 1.0);
```

In `src/passes/composite.rs`, replace:

```rust
        let on = uniforms(&p, 0.0, (1000, 500), (1000, 500), true);
        let off = uniforms(&p, 0.0, (1000, 500), (1000, 500), false);
        assert_eq!(on.fit[2], 1.0);
        assert_eq!(off.fit[2], 0.0);
```

with:

```rust
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
```

In `src/source.rs`, replace:

```rust
        assert!(err.to_string().contains("does-not-exist.png"));
```

with:

```rust
        assert!(err.to_string().contains("does-not-exist.png"));
        let err = load_color(Path::new("no-background.png")).unwrap_err();
        assert!(err.to_string().contains("no-background.png"));
    }

    #[test]
    fn loads_a_color_image_as_rgba() {
        let path = std::env::temp_dir().join(format!("rasterwarp-bg-{}.png", std::process::id()));
        let mut rgb = image::RgbImage::new(3, 2);
        rgb.put_pixel(2, 1, image::Rgb([200, 100, 50]));
        rgb.save(&path).expect("write test image");
        let loaded = load_color(&path).expect("load test image");
        let _ = std::fs::remove_file(&path);
        assert_eq!((loaded.width, loaded.height), (3, 2));
        assert_eq!(&loaded.pixels[20..24], &[200, 100, 50, 255]);
        assert_eq!(loaded.aspect(), 1.5);
        assert_eq!(ColorImage::black().pixels, vec![0, 0, 0, 255]);
```

In `src/source.rs`, replace:

```rust
        assert!(err.to_string().contains("65×32"));
```

with:

```rust
        assert!(err.to_string().contains("65×32"));
        assert!(ensure_size_fits(64, 65, 64).is_err());
```

In `tests/smoke.rs`, replace:

```rust
use rasterwarp::source::{GrayImage, test_card};
```

with:

```rust
use rasterwarp::source::{ColorImage, GrayImage, test_card};
```

In `tests/smoke.rs`, replace:

```rust

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
```

with:

```rust

#[test]
fn keyed_levels_show_the_background() {
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

/// Copies a 4-byte-per-pixel texture into memory, removing the 256-byte row padding
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test --test smoke keyed_levels_show_the_background`. Expected: fails to compile with ``unresolved import `rasterwarp::source::ColorImage` `` and ``no method named `set_background` found for struct `rasterwarp::passes::Renderer` ``.

- [ ] **Step 3: Implement.**

In `shaders/colorize.wgsl`, replace:

```wgsl
// adjustable thresholds, and map each level to a palette color.
```

with:

```wgsl
// adjustable thresholds, and map each level to a palette color. Keyed (see-through)
// levels get alpha 0; the output is premultiplied (color times alpha).
```

In `shaders/colorize.wgsl`, replace:

```wgsl
    extra: vec4<f32>, // smear taps in use, unused, unused, unused
```

with:

```wgsl
    extra: vec4<f32>, // smear taps in use, keying on (0 or 1), see-through level mask, unused
```

In `shaders/colorize.wgsl`, replace:

```wgsl

@fragment
```

with:

```wgsl

// 0 for a see-through level while keying, 1 otherwise.
fn opacity(level: f32) -> f32 {
    let mask = u32(u.extra.z);
    let see_through = u.extra.y > 0.5 && ((mask >> u32(level)) & 1u) == 1u;
    return select(1.0, 0.0, see_through);
}

@fragment
```

In `shaders/colorize.wgsl`, replace:

```wgsl
    return vec4<f32>(color, 1.0);
```

with:

```wgsl
    // Keying follows the brightness level, not the (cycling) palette color.
    let alpha = mix(opacity(i0), opacity(i1), x - i0);
    return vec4<f32>(color * alpha, alpha);
```

In `shaders/composite.wgsl`, replace:

```wgsl
// Composite: add bloom, then CRT treatment (chromatic bleed, scanlines, noise),
// letterboxed into the output.
```

with:

```wgsl
// Composite: show the background through keyed levels, add bloom, then CRT treatment
// (chromatic bleed, scanlines, noise), letterboxed into the output.
```

In `shaders/composite.wgsl`, replace:

```wgsl
    fit: vec4<f32>,  // image size as a fraction of the output (x, y), encode sRGB (z: 0 or 1), unused
```

with:

```wgsl
    fit: vec4<f32>,  // image size as a fraction of the output (x, y), encode sRGB (z: 0 or 1), unused
    background: vec4<f32>, // background uv scale for a "cover" fit (x, y), unused, unused
```

In `shaders/composite.wgsl`, replace:

```wgsl
@group(0) @binding(3) var bloom: texture_2d<f32>;
```

with:

```wgsl
@group(0) @binding(3) var bloom: texture_2d<f32>;
@group(0) @binding(4) var background: texture_2d<f32>;
```

In `shaders/composite.wgsl`, replace:

```wgsl
    );
    // The upsample chain sums all bloom levels, so scale it back down.
```

with:

```wgsl
    );
    // The image is premultiplied: the background shows where it is see-through.
    let alpha = textureSampleLevel(image, samp, iuv, 0.0).a;
    let buv = (iuv - 0.5) * u.background.xy + 0.5;
    c += textureSampleLevel(background, samp, buv, 0.0).rgb * (1.0 - alpha);
    // The upsample chain sums all bloom levels, so scale it back down.
```

In `shaders/feedback.wgsl`, replace:

```wgsl
// Feedback: keep a decaying, transformed copy of the previous frame under the new one.
```

with:

```wgsl
// Feedback: keep a decaying, transformed copy of the previous frame under the new one.
// Color and alpha (keying) both take the brighter of the new frame and the decayed trail.
```

In `shaders/feedback.wgsl`, replace:

```wgsl
    let cur = textureSampleLevel(current, samp, in.uv, 0.0).rgb;
```

with:

```wgsl
    let cur = textureSampleLevel(current, samp, in.uv, 0.0);
```

In `shaders/feedback.wgsl`, replace:

```wgsl
    var prev = vec3<f32>(0.0);
    if inside01(puv) {
        prev = textureSampleLevel(previous, samp, puv, 0.0).rgb;
    }
    return vec4<f32>(max(cur, prev * u.settings.x), 1.0);
```

with:

```wgsl
    var prev = vec4<f32>(0.0);
    if inside01(puv) {
        prev = textureSampleLevel(previous, samp, puv, 0.0);
    }
    return max(cur, prev * u.settings.x);
```

In `src/app.rs`, replace:

```rust
use crate::source::{self, GrayImage};
```

with:

```rust
use crate::source::{self, ColorImage, GrayImage};
```

In `src/app.rs`, replace:

```rust

    /// Wall-clock seconds since the app started, for the canvas clock.
```

with:

```rust

    /// Asks for a background image for keyed levels and shows it.
    fn choose_background(&mut self) {
        let picked = rfd::FileDialog::new()
            .set_title("Background image")
            .add_filter("Images", &["png", "jpg", "jpeg"])
            .pick_file();
        if let Some(path) = picked {
            match load_background(&path, self.max_texture_side) {
                Ok(image) => {
                    self.set_background(Some(&image));
                    self.ui.background_info = Some(format!(
                        "Background: {} ({}×{})",
                        path.display(),
                        image.width,
                        image.height
                    ));
                    self.ui.load_error = None;
                }
                Err(err) => {
                    log::warn!("{err:#}");
                    self.ui.load_error = Some(format!("{err:#}"));
                }
            }
        }
        // The dialog and decoding block the app; that must not make canvas frames late.
        self.clock.reanchor(self.seconds());
    }

    fn set_background(&mut self, image: Option<&ColorImage>) {
        self.renderer
            .set_background(&self.device, &self.queue, image);
        self.preview
            .set_background(&self.device, &self.queue, image);
        if image.is_none() {
            self.ui.background_info = None;
        }
    }

    /// Wall-clock seconds since the app started, for the canvas clock.
```

In `src/app.rs`, replace:

```rust
            self.choose_capture_folder();
```

with:

```rust
            self.choose_capture_folder();
        }
        if actions.choose_background {
            self.choose_background();
        }
        if actions.clear_background {
            self.set_background(None);
```

In `src/app.rs`, replace:

```rust

/// Loads an image and checks it fits in a GPU texture.
```

with:

```rust

/// Loads a background image and checks it fits in a GPU texture.
fn load_background(path: &Path, max_side: u32) -> anyhow::Result<ColorImage> {
    let image = source::load_color(path)?;
    source::ensure_size_fits(image.width, image.height, max_side)
        .with_context(|| format!("could not load {}", path.display()))?;
    Ok(image)
}

/// Loads an image and checks it fits in a GPU texture.
```

In `src/passes/colorize.rs`, replace:

```rust
use crate::gpu::{FullscreenPass, INTERNAL_FORMAT, PassDesc, RenderTarget};
```

with:

```rust
use crate::gpu::{FullscreenPass, INTERNAL_FORMAT, PassDesc, RenderTarget};
use crate::params::KeyParams;
```

In `src/passes/colorize.rs`, replace:

```rust
pub fn uniforms(c: &ColorizeFrame) -> ColorizeUniforms {
```

with:

```rust
pub fn uniforms(c: &ColorizeFrame, key: &KeyParams) -> ColorizeUniforms {
```

In `src/passes/colorize.rs`, replace:

```rust
        extra: [taps as f32, 0.0, 0.0, 0.0],
```

with:

```rust
        extra: [
            taps as f32,
            if key.enabled { 1.0 } else { 0.0 },
            f32::from(key.levels),
            0.0,
        ],
```

In `src/passes/composite.rs`, replace:

```rust
    pub fit: [f32; 4],
```

with:

```rust
    pub fit: [f32; 4],
    pub background: [f32; 4],
```

In `src/passes/composite.rs`, replace:

```rust

pub fn uniforms(
```

with:

```rust

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
```

In `src/passes/composite.rs`, replace:

```rust
) -> CompositeUniforms {
    let fit = letterbox(output, internal);
```

with:

```rust
    background_aspect: f32,
) -> CompositeUniforms {
    let fit = letterbox(output, internal);
    let canvas_aspect = internal.0 as f32 / internal.1 as f32;
    let back = cover(canvas_aspect, background_aspect);
```

In `src/passes/composite.rs`, replace:

```rust
        fit: [fit[0], fit[1], if encode_srgb { 1.0 } else { 0.0 }, 0.0],
```

with:

```rust
        fit: [fit[0], fit[1], if encode_srgb { 1.0 } else { 0.0 }, 0.0],
        background: [back[0], back[1], 0.0, 0.0],
```

In `src/passes/composite.rs`, replace:

```rust
                textures: 2,
```

with:

```rust
                textures: 3,
```

In `src/passes/composite.rs`, replace:

```rust
        bloom: &wgpu::TextureView,
```

with:

```rust
        bloom: &wgpu::TextureView,
        background: &wgpu::TextureView,
```

In `src/passes/composite.rs`, replace:

```rust
        let bind_group = self.pass.bind_group(device, &self.uniform, &[image, bloom]);
```

with:

```rust
        let bind_group = self
            .pass
            .bind_group(device, &self.uniform, &[image, bloom, background]);
```

In `src/passes/mod.rs`, replace:

```rust
use crate::source::{self, GrayImage};
```

with:

```rust
use crate::source::{self, ColorImage, GrayImage};
```

In `src/passes/mod.rs`, replace:

```rust
    source_aspect: f32,
```

with:

```rust
    source_aspect: f32,
    /// What keyed levels show.
    background: wgpu::TextureView,
    background_aspect: f32,
```

In `src/passes/mod.rs`, replace:

```rust
            source_aspect: image.aspect(),
```

with:

```rust
            source_aspect: image.aspect(),
            background: source::upload_color(device, queue, &ColorImage::black())
                .create_view(&Default::default()),
            background_aspect: 1.0,
```

In `src/passes/mod.rs`, replace:

```rust

    pub fn clear_feedback(&self, encoder: &mut wgpu::CommandEncoder) {
```

with:

```rust

    /// Sets what keyed levels show: an image, or black.
    pub fn set_background(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: Option<&ColorImage>,
    ) {
        let black = ColorImage::black();
        let image = image.unwrap_or(&black);
        self.background =
            source::upload_color(device, queue, image).create_view(&Default::default());
        self.background_aspect = image.aspect();
    }

    pub fn clear_feedback(&self, encoder: &mut wgpu::CommandEncoder) {
```

In `src/passes/mod.rs`, replace:

```rust
            &colorize::uniforms(&frame.colorize),
```

with:

```rust
            &colorize::uniforms(&frame.colorize, &frame.key),
```

In `src/passes/mod.rs`, replace:

```rust
        self.composite.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            self.bloom.output(),
            output,
            &composite::uniforms(
                &crt_glow(frame),
                time,
                self.size,
```

with:

```rust
        self.composite.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            self.bloom.output(),
            &self.background,
            output,
            &composite::uniforms(
                &crt_glow(frame),
                time,
                self.size,
```

In `src/passes/mod.rs`, replace:

```rust
                self.composite.encode_srgb(),
```

with:

```rust
                self.composite.encode_srgb(),
                self.background_aspect,
```

In `src/passes/mod.rs`, replace:

```rust
        self.capture.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            self.bloom.output(),
            output,
            &composite::uniforms(
                &crt_glow(frame),
                time,
                self.size,
```

with:

```rust
        self.capture.render(
            device,
            queue,
            encoder,
            self.feedback.output(),
            self.bloom.output(),
            &self.background,
            output,
            &composite::uniforms(
                &crt_glow(frame),
                time,
                self.size,
```

In `src/passes/mod.rs`, replace:

```rust
                self.capture.encode_srgb(),
```

with:

```rust
                self.capture.encode_srgb(),
                self.background_aspect,
```

In `src/preview.rs`, replace:

```rust
use crate::source::GrayImage;
```

with:

```rust
use crate::source::{ColorImage, GrayImage};
```

In `src/preview.rs`, replace:

```rust

    pub fn clear_feedback(&self, encoder: &mut wgpu::CommandEncoder) {
```

with:

```rust

    pub fn set_background(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: Option<&ColorImage>,
    ) {
        self.renderer.set_background(device, queue, image);
    }

    pub fn clear_feedback(&self, encoder: &mut wgpu::CommandEncoder) {
```

In `src/source.rs`, replace:

```rust
//! Source artwork: image files and the built-in test card, as 8-bit grayscale.
```

with:

```rust
//! Source artwork (image files and the built-in test card, as 8-bit grayscale) and the
//! color background image that keyed levels show.
```

In `src/source.rs`, replace:

```rust
    if image.width > max_side || image.height > max_side {
        anyhow::bail!(
            "image is {}×{}, larger than the GPU texture limit of {max_side}×{max_side}",
            image.width,
            image.height
```

with:

```rust
    ensure_size_fits(image.width, image.height, max_side)
}

/// Errors if a `width`×`height` image is too large to upload as a single GPU texture.
pub fn ensure_size_fits(width: u32, height: u32, max_side: u32) -> Result<()> {
    if width > max_side || height > max_side {
        anyhow::bail!(
            "image is {width}×{height}, larger than the GPU texture limit of {max_side}×{max_side}"
```

In `src/source.rs`, replace:

```rust

/// Procedural test card with many gray levels so every colorizer level gets used:
```

with:

```rust

/// A color image (the keying background), 8-bit sRGB RGBA, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl ColorImage {
    /// A single black pixel: what keyed levels show without a background image.
    pub fn black() -> Self {
        Self {
            width: 1,
            height: 1,
            pixels: vec![0, 0, 0, 255],
        }
    }

    pub fn aspect(&self) -> f32 {
        self.width as f32 / self.height as f32
    }
}

pub fn load_color(path: &Path) -> Result<ColorImage> {
    let image = image::open(path).with_context(|| format!("could not load {}", path.display()))?;
    let rgba = image.to_rgba8();
    Ok(ColorImage {
        width: rgba.width(),
        height: rgba.height(),
        pixels: rgba.into_raw(),
    })
}

/// Uploads a color image as an `Rgba8UnormSrgb` texture, so sampling returns linear light.
pub fn upload_color(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &ColorImage,
) -> wgpu::Texture {
    let size = wgpu::Extent3d {
        width: image.width,
        height: image.height,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("background"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
            bytes_per_row: Some(image.width * 4),
            rows_per_image: None,
        },
        size,
    );
    texture
}

/// Procedural test card with many gray levels so every colorizer level gets used:
```

In `src/ui.rs`, replace:

```rust
    pub load_error: Option<String>,
```

with:

```rust
    pub load_error: Option<String>,
    /// The keying background's description, if one is loaded.
    pub background_info: Option<String>,
```

In `src/ui.rs`, replace:

```rust
    pub choose_folder: bool,
```

with:

```rust
    pub choose_folder: bool,
    pub choose_background: bool,
    pub clear_background: bool,
```

In `src/ui.rs`, replace:

```rust
                colorize_section(ui, params);
```

with:

```rust
                colorize_section(ui, params);
                key_section(ui, params, state, &mut actions);
```

In `src/ui.rs`, replace:

```rust

fn feedback_section(ui: &mut Ui, params: &mut Params, actions: &mut UiActions) {
```

with:

```rust

fn key_section(ui: &mut Ui, params: &mut Params, state: &UiState, actions: &mut UiActions) {
    let levels = params.colorize.levels;
    let k = &mut params.key;
    CollapsingHeader::new("Keying").show(ui, |ui| {
        ui.checkbox(&mut k.enabled, "Level keying");
        ui.add_enabled_ui(k.enabled, |ui| {
            ui.label("See-through levels (darkest first):");
            ui.horizontal_wrapped(|ui| {
                for level in 0..levels {
                    let bit = 1u8 << level;
                    let mut on = k.levels & bit != 0;
                    if ui.checkbox(&mut on, format!("{}", level + 1)).changed() {
                        k.levels ^= bit;
                    }
                }
            });
        });
        ui.horizontal(|ui| {
            if ui.button("Background image…").clicked() {
                actions.choose_background = true;
            }
            if state.background_info.is_some() && ui.button("No background").clicked() {
                actions.clear_background = true;
            }
        });
        ui.small(
            state
                .background_info
                .as_deref()
                .unwrap_or("No background: see-through levels show black."),
        );
    });
}

fn feedback_section(ui: &mut Ui, params: &mut Params, actions: &mut UiActions) {
```

- [ ] **Step 4: Run everything.** Run: `cargo test`. Expected: `149 passed` for the unit tests, `4 passed` in `tests/capture.rs` and `12 passed` in `tests/smoke.rs`.

- [ ] **Step 5: Lint and format.** Run: `cargo fmt` then `cargo clippy --all-targets`. Expected: no warnings, and `cargo fmt` leaves the files as written here.

- [ ] **Step 6: Manual check (release).** Run: `cargo run --release` on the 60 Hz display, then check each item.
  - [ ] The header reads "60 fps · display 60 Hz · 0 late". Thin color fringes trail to the right of edges (edge fringe 1.0 px). Raising **edge fringe** widens them; raising **ringing** adds ripples after edges.
  - [ ] **Thresholds:** each "level N from" slider moves its level boundary and can't pass its neighbours. Changing **levels** re-spaces them evenly; **Even** restores even spacing.
  - [ ] **Line jitter 4 px:** edges turn ragged line by line; Pause freezes the pattern; unpausing it moves again.
  - [ ] **Axis wander:** with rotation set to about 1.0 rad, the picture's pivot drifts slightly over a few seconds; at rotation 0 nothing wanders.
  - [ ] **Raster → True raster:** the picture is drawn as real scan lines bent by the oscillators, with no painted-on scanlines. **Lines**, **beam width** and both compensation sliders change the look. The header stays at 0 late.
  - [ ] In **Transition** mode, with raster off on A and on B, the switch happens halfway through the ramp; line jitter blends smoothly from one bank's amount to the other's.
  - [ ] **Keying:** turn on **Level keying** with level 1 see-through: dark areas turn black. **Background image…** loads a PNG or JPG, which fills the canvas behind the see-through level, and trails fade over it smoothly. **No background** returns to black.
  - [ ] Record 3 s of FFV1 with raster mode and keying on; the file plays and matches the screen.

- [ ] **Step 7: Commit**

```bash
git add shaders/colorize.wgsl shaders/composite.wgsl shaders/feedback.wgsl src/app.rs src/passes/colorize.rs src/passes/composite.rs src/passes/mod.rs src/preview.rs src/source.rs src/ui.rs tests/smoke.rs
git commit -m "feat: level keying over a background image" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Done when

- `cargo test` passes (149 unit, 4 capture and 12 smoke tests). `cargo clippy --all-targets` is clean and `cargo fmt` makes no changes.
- The manual checklist in Task 6 passes on the 60 Hz display.
