# Rasterwarp Prototype: Design

Date: 2026-10-06
Status: Built (the prototype is on `main`); later specs extend it
Source: `HANDOFF.md` (project goals and prior decisions)

## Scope

The initial prototype covers HANDOFF steps 1–6:

1. Rust project with wgpu + winit, Vulkan backend confirmed on the dev machine.
2. Source artwork rendered to a texture and shown on screen.
3. Deflection (warp) shader driven by oscillators with live parameter control.
4. Posterize + palette colorizing.
5. Feedback/phosphor trails and bloom, plus scanlines, chromatic bleed, and noise.
6. A simple egui parameter panel for live control.

**Out of scope** (deferred to later specs): node-graph patching, MIDI/OSC, automation
recording, audio reactivity, SVG/font/SDF inputs, video/camera input, export,
Rutt/Etra mode, purist scanline-deflection mode, preset save/load, shader hot-reload.
Built since, in later specs: export (video capture) and a true raster (scanline) mode
(`2026-10-07-rasterwarp-motion-output-look-design.md`), preset save/load
(`2026-10-07-rasterwarp-save-load-design.md`), and video/camera input
(`2026-10-08-rasterwarp-video-inputs-design.md`).

Dev machine: Windows 10, NVIDIA RTX 3090, Rust 1.98 stable (MSVC).

## Architecture

Separate render passes. Each pipeline stage is a small Rust module with its own WGSL
shader and uniform buffer, and the stages are chained through textures. All live
parameters sit in one plain `Params` struct that the UI edits. A future node graph can
write into `Params` (or replace it) without touching the shaders.

### Crates

`wgpu`, `winit`, `egui`, `egui-wgpu`, `egui-winit`, `image` (PNG/JPG decode only),
`bytemuck`, `pollster`, `anyhow`, `log`, `env_logger`. Use versions of the egui crates
that match the chosen `wgpu`/`winit` versions.

### Layout (single binary crate)

```
src/main.rs          winit event loop, CLI image arg, drag-and-drop
src/app.rs           owns GPU context, passes, Params, UI; runs one frame
src/gpu.rs           device/surface setup, render-target helpers
src/params.rs        Params struct, defaults, GPU uniform packing
src/source.rs        image file -> grayscale texture; procedural test card
src/ui.rs            egui parameter panel
src/passes/warp.rs
src/passes/colorize.rs
src/passes/feedback.rs
src/passes/bloom.rs
src/passes/composite.rs
shaders/warp.wgsl, colorize.wgsl, feedback.wgsl, bloom.wgsl, composite.wgsl
```

Shaders are embedded with `include_str!`.

### Backend selection

The app uses `wgpu::Backends::VULKAN` by default. The environment variable
`RASTERWARP_BACKEND` (`vulkan` | `dx12` | `gl`) overrides it for debugging. The
selected adapter name and backend are logged at startup.

### Frame flow

```
source (R8, native res, loaded once)
  -> warp       sample source at oscillator-displaced coordinates      -> warped (Rgba16Float)
  -> colorize   posterize into N levels, map level -> palette color     -> colored
  -> feedback   mix(colored, transform(prev_feedback), decay)           -> feedback[ping/pong]
  -> bloom      threshold + downsample/upsample blur chain              -> bloom
  -> composite  feedback + bloom, scanlines, chroma bleed, noise,
                letterboxed to the window                                -> swapchain
  -> egui panel drawn on top
```

- Internal render targets are `Rgba16Float` at a fixed internal resolution
  (default 1920×1080), so the look doesn't depend on the window size.
- Feedback uses two textures that swap roles each frame (ping-pong).
- Bloom uses a mip chain (about 5 levels) of downsample then upsample passes.
- The composite pass letterboxes the internal image into the window's aspect ratio.

## Parameters (`Params`)

All parameters are live and edited from an egui side panel. Each pass gets only its
slice of `Params`, written to a uniform buffer every frame.

### Deflection (warp)

Four oscillators, each with:

- `waveform`: sine | triangle | ramp | square | noise
- `target`: X or Y displacement
- `input`: horizontal position (u) | vertical position (v) | radius from center | time only
- `frequency`, `amplitude`, `phase`, `phase_speed` (phase drift in cycles per second)
- amplitude LFO: sine, with `lfo_rate` and `lfo_depth`

Displacement = sum of the oscillator outputs, applied to the sampling coordinate.

Global warp controls: `zoom`, `rotation`, `offset` (x, y), and `drift` (amount of slow
random wobble on each oscillator's frequency and phase, for analog imperfection).
Areas sampled outside the source are black.

### Colorize

- `levels` (2–8), `softness` (edge smoothing between levels)
- `palette`: 8 editable colors
- `cycle_speed`: shifts colors through the levels over time
- `bypass`: show plain grayscale instead

### Feedback

- `amount` (decay; 0 = no trails)
- per-frame `zoom`, `rotation`, `offset` applied to the previous frame
- `clear` action (UI button that clears both ping-pong textures)

### Glow and CRT (bloom + composite)

- bloom `intensity`, `threshold`
- scanline `strength`, `count`
- chromatic bleed `amount`
- noise `amount`

### Global

- pause time, reset all to defaults
- FPS / frame-time readout
- source info and the last load error, if any

Parameters only last for the session. Preset save/load is out of scope.

## Source artwork

- Command-line argument: `rasterwarp [image.png|jpg]`.
- Drag-and-drop onto the window replaces the current source.
- Images are converted to 8-bit grayscale (luma) and uploaded as an `R8Unorm` texture
  at native resolution with linear filtering.
- With no image, a procedural test card is generated on the CPU: multi-level gray
  shapes (concentric rings, bars, a gradient strip, and a block shape) so all colorizer
  levels are exercised.

## Error handling

- **Startup failures** (no adapter for the selected backend, surface creation failure):
  exit with a clear `anyhow` error naming the backend and suggesting
  `RASTERWARP_BACKEND`.
- **Image load failures** (CLI or drag-drop): log a warning, keep the current source,
  and show the error in the UI panel. The app doesn't crash.
- **Surface `Lost`/`Outdated`:** reconfigure the surface and skip the frame.
  `Timeout`: skip the frame. Out of memory: exit with an error.
- **Zero-size window** (minimized): skip rendering.
- **Shader/pipeline errors:** shaders are compile-time embedded, and wgpu validation
  panics in debug builds, so they surface during development and in the smoke test.

## Testing

- **Unit tests (pure Rust):**
  - The size and alignment of each uniform struct matches WGSL uniform layout rules
    (16-byte alignment, `vec3` padding).
  - Test card: expected dimensions, and contains at least `8` distinct gray levels
    spread across 0–255.
  - Image-to-grayscale conversion of a small in-memory RGB image gives the expected
    luma.
  - `Params::default()` values fall within their UI ranges.
- **Headless GPU smoke test:** create a device with no surface, build all the passes,
  render 3 frames into an offscreen `Rgba8Unorm` texture, read it back, and assert there
  are no validation errors and the output isn't blank (not every pixel the same
  value). If no adapter is available, the test is skipped with a message.
- **Manual check by eye:** `cargo run` shows the test card warping, colorized, with
  trails and glow, at about 60 fps on the dev machine. `cargo run -- some.png` loads an
  image. Drag-and-drop works.

## Success criteria

- `cargo build` and `cargo test` pass with no warnings in the project's own code.
- The startup log shows the Vulkan backend on the RTX 3090.
- At 1920×1080 internal resolution with all effects on, the frame time readout stays
  under 16.7 ms in a release build.
- Every parameter in the panel visibly changes the output live.
