# Rasterwarp: Handoff

## Goal
Build **Rasterwarp**, animation software that emulates the 1970s Scanimate analog video-synthesis look: high-contrast artwork warped by oscillator-driven deflection, colorized by brightness level, with CRT glow and feedback trails. Performance-oriented: live control at interactive frame rates.

## Decisions so far
- **Name:** Rasterwarp. Avoid using "Scanimate" in branding (trademark/heritage ties to Computer Image Corporation); check trademark and domain availability for "Rasterwarp" before shipping commercially.
- **Graphics stack:** **wgpu** (Rust), targeting Vulkan so AMD and NVIDIA are both supported. Everything in the render pipeline runs on the GPU.
- **Input format:** raster is the internal format.
  - Vector inputs (SVG, text, fonts) are rasterized on load, ideally at 2-4x output resolution so edges stay crisp after heavy warping, and re-rasterized live when the source is edited or animated.
  - Consider a **signed distance field (SDF)** for logos and text: stays sharp under extreme magnification and makes thresholded levels and glowing edges cheap.
  - Accept grayscale / multi-level artwork, not just pure black and white, since the colorizer keys off brightness levels.
  - Video or live camera input is an optional extra on the same raster path.
- **Optional mode:** Rutt/Etra-style rendering (line geometry displaced by luminance), generated from the raster's brightness.

## Rendering pipeline
1. **Source:** artwork rendered to a texture.
2. **Deflection:** per-pixel (or per-scanline) displacement of the sampling position using a vector field built from summed waveforms (sine, triangle, ramp, noise) with controllable amplitude, frequency, and phase, plus envelopes and LFOs. Implemented in a fragment shader (WGSL).
3. **Colorizing:** posterize the grayscale signal into levels, then map each level to a palette color via lookup.
4. **Glow and feedback:** bloom/blur, scanline masking, phosphor persistence (blend with the previous frame), slight chromatic bleed. Feedback loops produce the trailing "echo" streaks.
5. **Control:** the main product-design challenge (see below).

## Design notes
- **Easy:** warping, colorizing, bloom, scanlines. A working prototype should come together quickly.
- **Moderate:** authentic look. The charm comes from imperfection: analog noise, nonlinear oscillator drift, bandwidth-limited edges, interlace artifacts. Plan on tuning by eye against reference footage.
- **Hardest:** the interface. The original was a performance instrument operated live by knob. Aim for:
  - A modular, patch-style node graph (oscillators, mixers, filters, envelopes), analogous to a modular audio synth, driving geometry instead of sound.
  - Real-time performance (target 60 fps).
  - MIDI/OSC input and automation recording.
  - Possibly audio-reactive control.
- Treat the core as a vector-field warp plus feedback rather than a literal raster-scan simulation, unless a purist "true scanline deflection" mode is added later.
- Export targets to plan for: alpha video, image sequences.

## Suggested next steps
1. Create the Rust project with wgpu and a windowing layer (e.g., winit); confirm the Vulkan backend is selected on the dev machine.
2. Render a test texture (text/logo) to the screen through a pass-through shader.
3. Add the deflection shader with a couple of oscillators and live parameter control.
4. Add posterize + palette colorizing.
5. Add the feedback/phosphor and bloom passes.
6. Prototype the control layer (start with a simple parameter UI, then move toward the node-graph patching).
7. Add SVG/font rasterization (and optionally SDF) for inputs, then export.

## Open questions
- UI toolkit for the control surface (e.g., egui vs. a custom node editor).
- SVG/text rasterization library choice.
- Whether to ship a purist scanline-deflection mode alongside the vector-field warp.
- Final trademark/domain check for the name.
